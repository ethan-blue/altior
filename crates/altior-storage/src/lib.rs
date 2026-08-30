//! SQLite storage: append-only event journal with rebuildable
//! projections (ADR 0009, ADR 0013).
//!
//! The durable-ownership rule from `docs/ARCHITECTURE.md` is the
//! contract here: the journal is authoritative for syncable knowledge
//! lifecycle; the projection tables are caches with a recorded
//! high-water marker, and any detected staleness heals by replay.
//! Everything is deterministic — SQLite runs in memory or in a
//! `tempfile` path, timestamps come from the envelopes, and no test
//! sleeps or touches the network.
//!
//! P1.1 adds domain journal records (independent from IPC `EventEnvelope`),
//! enriched thread/turn/permission projections, bounded thread list,
//! history, and search queries.

pub mod error;
mod migrations;

use rusqlite::{Connection, params};

use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileCursor, AgentProfileId, AgentProfileListLimit,
    BoundedLabel, BoundedPath, DisplayName, DomainEvent, DomainEventKind, EntityError, EventId,
    EventPayload, HarnessBindingCursor, HarnessBindingId, HarnessBindingListLimit, HarnessKind,
    HistoryLimit, MemoryMode, Permission, PermissionCursor, PermissionDecision,
    PermissionDescription, PermissionKind, PermissionListLimit, ProjectId, ProjectRef,
    ProjectRefCursor, ProjectRefListLimit, SearchQuery, ThreadId, ThreadListLimit, ThreadState,
    TurnCursor, TurnId, TurnListLimit, UnixMillis,
};
use altior_protocol::EventEnvelope;
pub use error::StorageError;

/// Defense-in-depth cap on a single journal payload (ADR 0009 §3),
/// mirroring the ACP line cap.
pub const JOURNAL_PAYLOAD_MAX: usize = 1024 * 1024;

/// Current fold semantics for derived projections. This is separate
/// from SQLite `user_version`, which versions physical schema only.
pub const PROJECTION_VERSION: i64 = 1;

/// Current fold semantics for domain projections. Separate from the
/// protocol projection version.
pub const DOMAIN_PROJECTION_VERSION: i64 = 1;

/// Maximum rows returned by one journal catch-up read.
pub const JOURNAL_LIMIT_MAX: u32 = 10_000;

/// Validated unsigned journal page size; SQLite never receives a
/// negative or unbounded `LIMIT` through the public API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalLimit(u32);

impl JournalLimit {
    /// Validates an unsigned row limit (zero is a legal empty page).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::JournalLimitOutOfRange`] above
    /// [`JOURNAL_LIMIT_MAX`].
    pub fn try_new(value: u32) -> Result<Self, StorageError> {
        if value > JOURNAL_LIMIT_MAX {
            return Err(StorageError::JournalLimitOutOfRange {
                value,
                max: JOURNAL_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated row count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The outcome of an append: appended, or recognized as a duplicate of
/// an event already journaled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendOutcome {
    /// The event was appended at this journal sequence.
    Appended {
        /// The journal sequence assigned to the new row.
        seq: i64,
    },
    /// The event id was already journaled; nothing changed.
    Duplicate {
        /// The journal sequence of the existing row.
        seq: i64,
    },
}

/// A per-thread aggregate derived from the journal (P0.5 protocol projection).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadSummary {
    /// The thread this projection row describes.
    pub thread_id: String,
    /// How many journaled events belong to the thread.
    pub event_count: i64,
    /// The journal sequence of the thread's first event.
    pub first_seq: i64,
    /// The journal sequence of the thread's most recent event.
    pub last_seq: i64,
    /// The event id of the thread's most recent event.
    pub last_event_id: String,
    /// The kind name of the thread's most recent event.
    pub last_kind: String,
    /// `occurred_at` of the thread's most recent event.
    pub updated_at: i64,
}

/// A journal row as stored, with the payload still encoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRow {
    /// The journal sequence (append order, 1-based).
    pub seq: i64,
    /// The event id (`evt_…`).
    pub event_id: String,
    /// The thread id, when the event is thread-scoped.
    pub thread_id: Option<String>,
    /// The turn id, when the event is turn-scoped.
    pub turn_id: Option<String>,
    /// The envelope's per-stream sequence.
    pub stream_sequence: i64,
    /// The event kind name (known or `acp.*` provider kind).
    pub kind: String,
    /// The JSON-encoded `EventEnvelope`.
    pub payload: Vec<u8>,
    /// `occurred_at` in Unix milliseconds.
    pub occurred_at: i64,
}

impl JournalRow {
    /// Decodes the stored payload back into an envelope.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::PayloadNotUtf8`] or
    /// [`StorageError::DecodeFailed`] when the payload cannot become
    /// an envelope again.
    pub fn decode(&self) -> Result<EventEnvelope, StorageError> {
        let text = std::str::from_utf8(&self.payload)
            .map_err(|_| StorageError::PayloadNotUtf8 { seq: self.seq })?;
        EventEnvelope::from_json(text).map_err(|source| StorageError::DecodeFailed {
            seq: self.seq,
            source,
        })
    }
}

/// A domain journal row as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainJournalRow {
    /// The domain journal sequence (append order, 1-based).
    pub seq: i64,
    /// The event id.
    pub event_id: String,
    /// The thread id, when thread-scoped.
    pub thread_id: Option<String>,
    /// The turn id, when turn-scoped.
    pub turn_id: Option<String>,
    /// Optional operation id.
    pub operation_id: Option<String>,
    /// The domain event kind name.
    pub kind: String,
    /// The domain event payload bytes.
    pub payload: Vec<u8>,
    /// `occurred_at` in Unix milliseconds.
    pub occurred_at: i64,
}

/// A thread row as projected from the domain journal (P1.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadRow {
    /// The thread id.
    pub thread_id: String,
    /// The agent profile id.
    pub agent_profile_id: String,
    /// The thread title (may be empty).
    pub title: String,
    /// Lifecycle state as a string.
    pub state: String,
    /// Optional project id.
    pub project_id: Option<String>,
    /// How many domain events belong to the thread.
    pub event_count: i64,
    /// Created-at timestamp.
    pub created_at: i64,
    /// Updated-at timestamp.
    pub updated_at: i64,
}

/// A turn row as projected from the domain journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnRow {
    /// The turn id.
    pub turn_id: String,
    /// The thread id.
    pub thread_id: String,
    /// Optional operation id.
    pub operation_id: Option<String>,
    /// Lifecycle state.
    pub state: String,
    /// Delivery classification.
    pub delivery: String,
    /// Domain event count for this turn.
    pub event_count: i64,
    /// When the turn started.
    pub started_at: i64,
    /// When the turn ended, if terminal.
    pub ended_at: Option<i64>,
}

pub use altior_domain::ThreadCursor;

/// Permission projection row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRow {
    pub event_id: String,
    pub turn_id: String,
    pub thread_id: String,
    pub kind: String,
    pub description: String,
    pub decision: String,
    pub requested_at: i64,
    pub decided_at: Option<i64>,
}

/// An open journal store: a migrated, self-healing SQLite database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating or migrating) a database file, then heals any
    /// stale projection before returning.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::SchemaTooNew`] when the file was written
    /// by a newer build, or the migration/rebuild failures otherwise.
    pub fn open(path: &std::path::Path) -> Result<Self, StorageError> {
        let conn =
            Connection::open(path).map_err(|error| StorageError::from_sqlite("open", error))?;
        Self::open_inner(conn)
    }

    /// Opens a private in-memory database (deterministic tests, no fs).
    ///
    /// # Errors
    ///
    /// Returns the migration/rebuild failures.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()
            .map_err(|error| StorageError::from_sqlite("open in-memory", error))?;
        Self::open_inner(conn)
    }

    fn open_inner(mut conn: Connection) -> Result<Self, StorageError> {
        Self::apply_pragmas(&conn)?;
        migrations::migrate(&mut conn)?;
        let mut store = Self { conn };
        store.ensure_projections_current()?;
        store.ensure_domain_projections_current()?;
        Ok(store)
    }

    fn apply_pragmas(conn: &Connection) -> Result<(), StorageError> {
        // WAL keeps readers unblocked by the writer; in-memory
        // databases report `memory` instead, which is fine (ADR 0009
        // context). `synchronous=NORMAL` is the standard WAL pairing.
        let _mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|error| StorageError::from_sqlite("set journal_mode", error))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| StorageError::from_sqlite("set foreign_keys", error))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|error| StorageError::from_sqlite("set synchronous", error))?;
        Ok(())
    }

    /// The schema version stamped in this database.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] when the pragma cannot be read.
    pub fn schema_version(&self) -> Result<i64, StorageError> {
        migrations::schema_version(&self.conn)
    }

    // ── Protocol journal (v1, preserved) ───────────────────────────

    /// Appends an event, idempotently by event id (ADR 0009 §3).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::PayloadTooLarge`] over the journal cap,
    /// [`StorageError::EncodeFailed`] when the envelope cannot be
    /// encoded, or the SQLite failures.
    pub fn append_event(
        &mut self,
        envelope: &EventEnvelope,
    ) -> Result<AppendOutcome, StorageError> {
        let payload = envelope
            .to_json()
            .map_err(|source| StorageError::EncodeFailed { source })?;
        let bytes = payload.as_bytes();
        if bytes.len() > JOURNAL_PAYLOAD_MAX {
            return Err(StorageError::PayloadTooLarge {
                size_bytes: bytes.len(),
                limit_bytes: JOURNAL_PAYLOAD_MAX,
            });
        }

        let thread = envelope
            .thread_id
            .as_ref()
            .map(altior_domain::ThreadId::as_str);
        let turn = envelope.turn_id.as_ref().map(altior_domain::TurnId::as_str);
        let kind = envelope.body.kind_name();
        let occurred_at = i64::try_from(envelope.occurred_at.as_millis()).map_err(|_| {
            StorageError::RebuildInvariant {
                detail: format!(
                    "occurred_at {} exceeds the i64 column",
                    envelope.occurred_at.as_millis()
                ),
            }
        })?;
        let stream_sequence = i64::try_from(envelope.sequence.as_u64()).map_err(|_| {
            StorageError::RebuildInvariant {
                detail: format!("sequence {} exceeds the i64 column", envelope.sequence),
            }
        })?;

        let tx = self
            .conn
            .transaction()
            .map_err(|error| StorageError::from_sqlite("append_event", error))?;
        if let Some((seq, existing_payload)) = existing_event(&tx, envelope.event_id.as_str())? {
            if existing_payload == bytes {
                return Ok(AppendOutcome::Duplicate { seq });
            }
            return Err(StorageError::EventIdCollision {
                event_id: envelope.event_id.to_string(),
                existing_seq: seq,
            });
        }
        tx.execute(
            "INSERT INTO journal
                 (event_id, thread_id, turn_id, stream_sequence, kind, payload, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                envelope.event_id.as_str(),
                thread,
                turn,
                stream_sequence,
                kind,
                bytes,
                occurred_at,
            ],
        )
        .map_err(|error| StorageError::from_sqlite("append_event", error))?;
        let seq = tx.last_insert_rowid();

        if let Some(thread) = thread {
            // Incremental fold: the projection stays current on the
            // happy path so reads never wait for a rebuild.
            tx.execute(
                "INSERT INTO thread_projection
                     (thread_id, event_count, first_seq, last_seq, last_event_id, last_kind, updated_at)
                 VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5)
                 ON CONFLICT(thread_id) DO UPDATE SET
                     event_count = event_count + 1,
                     last_seq = excluded.last_seq,
                     last_event_id = excluded.last_event_id,
                     last_kind = excluded.last_kind,
                     updated_at = excluded.updated_at",
                params![thread, seq, envelope.event_id.as_str(), kind, occurred_at],
            )
            .map_err(|error| StorageError::from_sqlite("append_event", error))?;
        }
        tx.execute(
            "INSERT INTO projection_state (id, journal_max_seq, projection_version, rebuilt_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
                 journal_max_seq = excluded.journal_max_seq,
                 projection_version = excluded.projection_version,
                 rebuilt_at = excluded.rebuilt_at",
            params![seq, PROJECTION_VERSION, occurred_at,],
        )
        .map_err(|error| StorageError::from_sqlite("append_event", error))?;
        tx.commit()
            .map_err(|error| StorageError::from_sqlite("append_event", error))?;
        Ok(AppendOutcome::Appended { seq })
    }

    /// Reads journal rows after `after_seq` in append order.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn journal_records(
        &self,
        after_seq: i64,
        limit: JournalLimit,
    ) -> Result<Vec<JournalRow>, StorageError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT seq, event_id, thread_id, turn_id, stream_sequence, kind, payload, occurred_at
                 FROM journal WHERE seq > ?1 ORDER BY seq LIMIT ?2",
            )
            .map_err(|error| StorageError::from_sqlite("journal_records", error))?;
        let rows = statement
            .query_map(
                params![after_seq, i64::from(limit.get())],
                row_to_journal_row,
            )
            .map_err(|error| StorageError::from_sqlite("journal_records", error))?;
        collect(rows, "journal_records")
    }

    /// Returns every thread projection row, ordered by thread id.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn thread_summaries(&self) -> Result<Vec<ThreadSummary>, StorageError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT thread_id, event_count, first_seq, last_seq, last_event_id, last_kind, updated_at
                 FROM thread_projection ORDER BY thread_id",
            )
            .map_err(|error| StorageError::from_sqlite("thread_summaries", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(ThreadSummary {
                    thread_id: row.get(0)?,
                    event_count: row.get(1)?,
                    first_seq: row.get(2)?,
                    last_seq: row.get(3)?,
                    last_event_id: row.get(4)?,
                    last_kind: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|error| StorageError::from_sqlite("thread_summaries", error))?;
        collect(rows, "thread_summaries")
    }

    /// The number of journaled events (protocol journal).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn journal_len(&self) -> Result<i64, StorageError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM journal", [], |row| row.get(0))
            .map_err(|error| StorageError::from_sqlite("journal_len", error))
    }

    /// The highest journal sequence, or 0 when empty.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn journal_max_seq(&self) -> Result<i64, StorageError> {
        self.conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM journal", [], |row| {
                row.get(0)
            })
            .map_err(|error| StorageError::from_sqlite("journal_max_seq", error))
    }

    /// Rebuilds every projection from the journal in one transaction
    /// and refreshes the high-water marker (ADR 0009 §4).
    ///
    /// Returns the number of journaled events replayed.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure; on error the
    /// transaction rolls back and the previous projection survives.
    pub fn rebuild_projections(&mut self) -> Result<i64, StorageError> {
        let tx = self
            .conn
            .transaction()
            .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        tx.execute("DELETE FROM thread_projection", [])
            .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        // Self-join on the per-thread maximum keeps the "last event"
        // columns deterministic without relying on SQLite's bare-column
        // behavior for mixed min/max aggregates.
        tx.execute(
            "INSERT INTO thread_projection
                 (thread_id, event_count, first_seq, last_seq, last_event_id, last_kind, updated_at)
             SELECT j.thread_id, agg.event_count, agg.first_seq, j.seq, j.event_id, j.kind, j.occurred_at
             FROM (
                 SELECT thread_id, COUNT(*) AS event_count, MIN(seq) AS first_seq, MAX(seq) AS last_seq
                 FROM journal WHERE thread_id IS NOT NULL GROUP BY thread_id
             ) agg
             JOIN journal j ON j.thread_id = agg.thread_id AND j.seq = agg.last_seq",
            [],
        )
        .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        tx.execute(
            "INSERT INTO projection_state (id, journal_max_seq, projection_version, rebuilt_at)
             SELECT 1, COALESCE(MAX(seq), 0), ?1, COALESCE(MAX(occurred_at), 0) FROM journal
             WHERE true
             ON CONFLICT(id) DO UPDATE SET
                 journal_max_seq = excluded.journal_max_seq,
                 projection_version = excluded.projection_version,
                 rebuilt_at = excluded.rebuilt_at",
            params![PROJECTION_VERSION],
        )
        .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        let replayed = tx
            .query_row("SELECT COUNT(*) FROM journal", [], |row| row.get(0))
            .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        tx.commit()
            .map_err(|error| StorageError::from_sqlite("rebuild", error))?;
        Ok(replayed)
    }

    /// Heals the projections when the high-water marker disagrees with
    /// the journal (ADR 0009 §4). A marker ahead of the journal means
    /// history shrank, which the append-only triggers should have made
    /// impossible — surfaced instead of silently repaired.
    fn ensure_projections_current(&mut self) -> Result<(), StorageError> {
        let marker: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT journal_max_seq, projection_version FROM projection_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(StorageError::from_sqlite("ensure_current", other)),
            })?;
        let journal_max = self.journal_max_seq()?;
        match marker {
            None if journal_max == 0 => Ok(()),
            None => self.rebuild_projections().map(|_| ()),
            Some((_, version)) if version != PROJECTION_VERSION => {
                self.rebuild_projections().map(|_| ())
            }
            Some((marker, _)) if marker < journal_max => self.rebuild_projections().map(|_| ()),
            Some((marker, _)) if marker > journal_max => Err(StorageError::RebuildInvariant {
                detail: format!(
                    "projection marker {marker} is ahead of journal max {journal_max}; \
                     journal history cannot shrink"
                ),
            }),
            Some(_) => Ok(()),
        }
    }

    // ── Domain journal (P1.1) ──────────────────────────────────────

    /// Appends a domain event, idempotently by event id.
    ///
    /// Same idempotency contract as `append_event`: same id+payload →
    /// Duplicate; same id + different payload → collision error.
    ///
    /// # Errors
    ///
    /// Returns typed errors for payload too large, collision, or SQLite
    /// failures.
    pub fn append_domain_event(
        &mut self,
        event: &DomainEvent,
    ) -> Result<AppendOutcome, StorageError> {
        let payload_bytes = event.payload.as_bytes();
        if payload_bytes.len() > JOURNAL_PAYLOAD_MAX {
            return Err(StorageError::PayloadTooLarge {
                size_bytes: payload_bytes.len(),
                limit_bytes: JOURNAL_PAYLOAD_MAX,
            });
        }

        let thread = event.thread_id.as_ref().map(ThreadId::as_str);
        let turn = event.turn_id.as_ref().map(TurnId::as_str);
        let operation = event
            .operation_id
            .as_ref()
            .map(altior_domain::OperationId::as_str);
        let kind = event.kind.as_str();
        let occurred_at = i64::try_from(event.occurred_at.as_millis()).map_err(|_| {
            StorageError::RebuildInvariant {
                detail: format!(
                    "occurred_at {} exceeds the i64 column",
                    event.occurred_at.as_millis()
                ),
            }
        })?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("append_domain_event", error))?;

        if let Some(existing) = existing_domain_event_tuple(&tx, event.event_id.as_str())? {
            let candidate = durable_tuple(event, occurred_at);
            if existing.1 == candidate.1
                && existing.2 == candidate.2
                && existing.3 == candidate.3
                && existing.4 == candidate.4
                && existing.5 == candidate.5
                && existing.6 == candidate.6
            {
                return Ok(AppendOutcome::Duplicate { seq: existing.0 });
            }
            return Err(StorageError::EventIdCollision {
                event_id: event.event_id.to_string(),
                existing_seq: existing.0,
            });
        }
        validate_domain_event_in_tx(&tx, event)?;
        tx.execute(
            "INSERT INTO domain_journal
                 (event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.event_id.as_str(),
                thread,
                turn,
                operation,
                kind,
                payload_bytes,
                occurred_at,
            ],
        )
        .map_err(|error| StorageError::from_sqlite("append_domain_event", error))?;
        let seq = tx.last_insert_rowid();

        // Incremental fold into domain projections.
        fold_domain_event_in_tx(&tx, event, seq, occurred_at)?;

        // Marker includes a deterministic projection digest, so equal journal
        // high-water marks cannot mask deleted/tampered cache rows.
        let digest = domain_projection_digest(&tx)?;
        tx.execute(
            "INSERT INTO domain_projection_state (id, journal_max_seq, projection_version, rebuilt_at, projection_digest)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 journal_max_seq = excluded.journal_max_seq,
                 projection_version = excluded.projection_version,
                 rebuilt_at = excluded.rebuilt_at, projection_digest = excluded.projection_digest",
            params![seq, DOMAIN_PROJECTION_VERSION, occurred_at, digest],
        )
        .map_err(|error| StorageError::from_sqlite("append_domain_event", error))?;

        tx.commit()
            .map_err(|error| StorageError::from_sqlite("append_domain_event", error))?;
        Ok(AppendOutcome::Appended { seq })
    }
}

/// Incrementally folds one domain event into projections inside
/// an existing transaction.
#[allow(clippy::too_many_lines)]
fn fold_domain_event_in_tx(
    tx: &rusqlite::Transaction<'_>,
    event: &DomainEvent,
    seq: i64,
    occurred_at: i64,
) -> Result<(), StorageError> {
    let Some(thread_id) = event.thread_id.as_ref() else {
        return Ok(());
    };

    match &event.kind {
        DomainEventKind::ThreadCreated => {
            // Extract agent_profile_id from payload if available,
            // otherwise use a placeholder that will be fixed on rebuild.
            let agent_profile_id =
                extract_json_string(event.payload.as_bytes(), "agent_profile_id")
                    .unwrap_or_default();
            let title = extract_json_string(event.payload.as_bytes(), "title").unwrap_or_default();
            let project_id = extract_json_string(event.payload.as_bytes(), "project_id");

            tx.execute(
                "INSERT INTO thread
                         (thread_id, agent_profile_id, title, state, project_id,
                          event_count, first_event_seq, last_event_seq, last_event_id,
                          last_event_kind, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'open', ?4, 1, ?5, ?5, ?6, ?7, ?8, ?8)
                     ON CONFLICT(thread_id) DO UPDATE SET
                         event_count = event_count + 1,
                         last_event_seq = excluded.last_event_seq,
                         last_event_id = excluded.last_event_id,
                         last_event_kind = excluded.last_event_kind,
                         updated_at = excluded.updated_at",
                params![
                    thread_id.as_str(),
                    agent_profile_id,
                    title,
                    project_id,
                    seq,
                    event.event_id.as_str(),
                    event.kind.as_str(),
                    occurred_at,
                ],
            )
            .map_err(|error| StorageError::from_sqlite("fold_thread_created", error))?;
        }
        DomainEventKind::ThreadTitleChanged => {
            let title = extract_json_string(event.payload.as_bytes(), "title").unwrap_or_default();
            tx.execute(
                "UPDATE thread SET title = ?1, event_count = event_count + 1,
                         last_event_seq = ?2, last_event_id = ?3,
                         last_event_kind = ?4, updated_at = ?5
                     WHERE thread_id = ?6",
                params![
                    title,
                    seq,
                    event.event_id.as_str(),
                    event.kind.as_str(),
                    occurred_at,
                    thread_id.as_str(),
                ],
            )
            .map_err(|error| StorageError::from_sqlite("fold_title_changed", error))?;
        }
        DomainEventKind::ThreadStateChanged => {
            let state = extract_json_string(event.payload.as_bytes(), "state")
                .unwrap_or_else(|| "open".to_owned());
            tx.execute(
                "UPDATE thread SET state = ?1, event_count = event_count + 1,
                         last_event_seq = ?2, last_event_id = ?3,
                         last_event_kind = ?4, updated_at = ?5
                     WHERE thread_id = ?6",
                params![
                    state,
                    seq,
                    event.event_id.as_str(),
                    event.kind.as_str(),
                    occurred_at,
                    thread_id.as_str(),
                ],
            )
            .map_err(|error| StorageError::from_sqlite("fold_state_changed", error))?;
        }
        DomainEventKind::TurnStarted => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                let operation = event
                    .operation_id
                    .as_ref()
                    .map(altior_domain::OperationId::as_str);
                tx.execute(
                    "INSERT INTO turn
                             (turn_id, thread_id, operation_id, state, delivery,
                              event_count, started_at, ended_at)
                         VALUES (?1, ?2, ?3, 'active', 'absent', 1, ?4, NULL)
                         ON CONFLICT(turn_id) DO UPDATE SET
                             event_count = event_count + 1",
                    params![turn_id.as_str(), thread_id.as_str(), operation, occurred_at,],
                )
                .map_err(|error| StorageError::from_sqlite("fold_turn_started", error))?;
            }
            // Update thread activity.
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::TurnCompleted => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                tx.execute(
                    "UPDATE turn SET state = 'completed', delivery = 'confirmed',
                             event_count = event_count + 1, ended_at = ?1
                         WHERE turn_id = ?2",
                    params![occurred_at, turn_id.as_str()],
                )
                .map_err(|error| StorageError::from_sqlite("fold_turn_completed", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::TurnCancelled => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                tx.execute(
                    "UPDATE turn SET state = 'cancelled',
                             event_count = event_count + 1, ended_at = ?1
                         WHERE turn_id = ?2",
                    params![occurred_at, turn_id.as_str()],
                )
                .map_err(|error| StorageError::from_sqlite("fold_turn_cancelled", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::TurnFailed => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                tx.execute(
                    "UPDATE turn SET state = 'failed',
                             event_count = event_count + 1, ended_at = ?1
                         WHERE turn_id = ?2",
                    params![occurred_at, turn_id.as_str()],
                )
                .map_err(|error| StorageError::from_sqlite("fold_turn_failed", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::PermissionRequested => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                let kind = extract_json_string(event.payload.as_bytes(), "permission_kind")
                    .unwrap_or_else(|| "execute".to_owned());
                let description = extract_json_string(event.payload.as_bytes(), "description")
                    .unwrap_or_default();
                tx.execute(
                    "INSERT INTO permission
                             (event_id, turn_id, thread_id, kind, description,
                              decision, requested_at, decided_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, NULL)
                         ON CONFLICT(event_id) DO NOTHING",
                    params![
                        event.event_id.as_str(),
                        turn_id.as_str(),
                        thread_id.as_str(),
                        kind,
                        description,
                        occurred_at,
                    ],
                )
                .map_err(|error| StorageError::from_sqlite("fold_permission_requested", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::PermissionDecided => {
            let perm_event_id =
                extract_json_string(event.payload.as_bytes(), "permission_event_id");
            let decision = extract_json_string(event.payload.as_bytes(), "decision")
                .unwrap_or_else(|| "denied".to_owned());
            if let Some(perm_event_id) = perm_event_id {
                tx.execute(
                    "UPDATE permission SET decision = ?1, decided_at = ?2
                         WHERE event_id = ?3",
                    params![decision, occurred_at, perm_event_id],
                )
                .map_err(|error| StorageError::from_sqlite("fold_permission_decided", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        DomainEventKind::Other(_) => {
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
        // Message deltas update turn and thread activity only.
        DomainEventKind::MessageDelta => {
            if let Some(turn_id) = event.turn_id.as_ref() {
                tx.execute(
                    "UPDATE turn SET event_count = event_count + 1
                         WHERE turn_id = ?1",
                    params![turn_id.as_str()],
                )
                .map_err(|error| StorageError::from_sqlite("fold_turn_activity", error))?;
            }
            update_thread_activity(
                tx,
                thread_id.as_str(),
                seq,
                event.event_id.as_str(),
                event.kind.as_str(),
                occurred_at,
            )?;
        }
    }
    Ok(())
}

impl Store {
    /// The number of domain journal events.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn domain_journal_len(&self) -> Result<i64, StorageError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM domain_journal", [], |row| row.get(0))
            .map_err(|error| StorageError::from_sqlite("domain_journal_len", error))
    }

    /// The highest domain journal sequence, or 0 when empty.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn domain_journal_max_seq(&self) -> Result<i64, StorageError> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM domain_journal",
                [],
                |row| row.get(0),
            )
            .map_err(|error| StorageError::from_sqlite("domain_journal_max_seq", error))
    }

    /// Reads domain journal rows after `after_seq`, bounded.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn domain_journal_records(
        &self,
        after_seq: i64,
        limit: JournalLimit,
    ) -> Result<Vec<DomainJournalRow>, StorageError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT seq, event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at
                 FROM domain_journal WHERE seq > ?1 ORDER BY seq LIMIT ?2",
            )
            .map_err(|error| StorageError::from_sqlite("domain_journal_records", error))?;
        let rows = statement
            .query_map(params![after_seq, i64::from(limit.get())], |row| {
                Ok(DomainJournalRow {
                    seq: row.get(0)?,
                    event_id: row.get(1)?,
                    thread_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    operation_id: row.get(4)?,
                    kind: row.get(5)?,
                    payload: row.get(6)?,
                    occurred_at: row.get(7)?,
                })
            })
            .map_err(|error| StorageError::from_sqlite("domain_journal_records", error))?;
        collect(rows, "domain_journal_records")
    }

    // ── Domain projections: threads ────────────────────────────────

    /// Returns a bounded page of threads, ordered by `updated_at` descending
    /// (most recent first), optionally filtered by state.
    ///
    /// Pagination is cursor-based: pass the `updated_at` of the last row
    /// in the previous page as `before_updated_at` to get the next page.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn thread_list(
        &self,
        state_filter: Option<ThreadState>,
        before: Option<&ThreadCursor>,
        limit: ThreadListLimit,
    ) -> Result<Vec<ThreadRow>, StorageError> {
        let state_str = state_filter.map(thread_state_to_str);
        let (before_updated_at, before_thread_id) = before.map_or((i64::MAX, ""), |cursor| {
            (
                i64::try_from(cursor.updated_at.as_millis()).unwrap_or(i64::MAX),
                cursor.thread_id.as_str(),
            )
        });

        let mut statement = self
            .conn
            .prepare(
                "SELECT thread_id, agent_profile_id, title, state, project_id,
                        event_count, created_at, updated_at
                 FROM thread
                 WHERE (?1 IS NULL OR state = ?1)
                   AND (updated_at < ?2 OR (updated_at = ?2 AND thread_id > ?3))
                 ORDER BY updated_at DESC, thread_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("thread_list", error))?;
        let rows = statement
            .query_map(
                params![
                    state_str,
                    before_updated_at,
                    before_thread_id,
                    i64::from(limit.get())
                ],
                row_to_thread_row,
            )
            .map_err(|error| StorageError::from_sqlite("thread_list", error))?;
        collect(rows, "thread_list")
    }

    /// Returns the thread row for a specific thread id.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn thread_by_id(&self, thread_id: &ThreadId) -> Result<Option<ThreadRow>, StorageError> {
        self.conn
            .query_row(
                "SELECT thread_id, agent_profile_id, title, state, project_id,
                        event_count, created_at, updated_at
                 FROM thread WHERE thread_id = ?1",
                params![thread_id.as_str()],
                row_to_thread_row,
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(StorageError::from_sqlite("thread_by_id", other)),
            })
    }

    /// Returns the event history for a thread, ordered by domain journal
    /// sequence (oldest first), with cursor-based pagination.
    ///
    /// Pass `after_seq` from the last row's sequence to page forward.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn thread_history(
        &self,
        thread_id: &ThreadId,
        after_seq: i64,
        limit: HistoryLimit,
    ) -> Result<Vec<DomainJournalRow>, StorageError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT seq, event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at
                 FROM domain_journal
                 WHERE thread_id = ?1 AND seq > ?2
                 ORDER BY seq
                 LIMIT ?3",
            )
            .map_err(|error| StorageError::from_sqlite("thread_history", error))?;
        let rows = statement
            .query_map(
                params![thread_id.as_str(), after_seq, i64::from(limit.get())],
                |row| {
                    Ok(DomainJournalRow {
                        seq: row.get(0)?,
                        event_id: row.get(1)?,
                        thread_id: row.get(2)?,
                        turn_id: row.get(3)?,
                        operation_id: row.get(4)?,
                        kind: row.get(5)?,
                        payload: row.get(6)?,
                        occurred_at: row.get(7)?,
                    })
                },
            )
            .map_err(|error| StorageError::from_sqlite("thread_history", error))?;
        collect(rows, "thread_history")
    }

    /// Searches threads by title using FTS5, returning matches bounded
    /// by `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn search_threads(
        &self,
        query: &SearchQuery,
        before: Option<&ThreadCursor>,
        limit: ThreadListLimit,
    ) -> Result<Vec<ThreadRow>, StorageError> {
        let (before_updated_at, before_thread_id) = before.map_or((i64::MAX, ""), |cursor| {
            (
                i64::try_from(cursor.updated_at.as_millis()).unwrap_or(i64::MAX),
                cursor.thread_id.as_str(),
            )
        });
        let fts_query = fts5_quoted_literal(query.as_str());
        let mut statement = self
            .conn
            .prepare(
                "SELECT t.thread_id, t.agent_profile_id, t.title, t.state, t.project_id,
                        t.event_count, t.created_at, t.updated_at
                 FROM thread_search s
                 JOIN thread t ON t.thread_id = s.thread_id
                 WHERE thread_search MATCH ?1
                   AND (t.updated_at < ?2 OR (t.updated_at = ?2 AND t.thread_id > ?3))
                 ORDER BY t.updated_at DESC, t.thread_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("search_threads", error))?;
        let rows = statement
            .query_map(
                params![
                    fts_query,
                    before_updated_at,
                    before_thread_id,
                    i64::from(limit.get())
                ],
                row_to_thread_row,
            )
            .map_err(|error| StorageError::from_sqlite("search_threads", error))?;
        collect(rows, "search_threads")
    }

    /// Returns turns for a thread, ordered by `started_at` ascending, with `turn_id` ascending as tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn turns_for_thread(
        &self,
        thread_id: &ThreadId,
        after: Option<&TurnCursor>,
        limit: TurnListLimit,
    ) -> Result<Vec<TurnRow>, StorageError> {
        let (after_started_at, after_turn_id) = after.map_or((-1, ""), |cursor| {
            (
                i64::try_from(cursor.started_at.as_millis()).unwrap_or(-1),
                cursor.turn_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT turn_id, thread_id, operation_id, state, delivery,
                        event_count, started_at, ended_at
                 FROM turn
                 WHERE thread_id = ?1
                   AND (started_at > ?2 OR (started_at = ?2 AND turn_id > ?3))
                 ORDER BY started_at ASC, turn_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("turns_for_thread", error))?;
        let rows = statement
            .query_map(
                params![
                    thread_id.as_str(),
                    after_started_at,
                    after_turn_id,
                    i64::from(limit.get()),
                ],
                |row| {
                    Ok(TurnRow {
                        turn_id: row.get(0)?,
                        thread_id: row.get(1)?,
                        operation_id: row.get(2)?,
                        state: row.get(3)?,
                        delivery: row.get(4)?,
                        event_count: row.get(5)?,
                        started_at: row.get(6)?,
                        ended_at: row.get(7)?,
                    })
                },
            )
            .map_err(|error| StorageError::from_sqlite("turns_for_thread", error))?;
        collect(rows, "turns_for_thread")
    }

    // ── Domain projections & metadata: AgentProfile CRUD ───────────

    /// Creates an agent profile.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::AgentProfileAlreadyExists`] if the ID already exists with conflicting data,
    /// [`StorageError::InvalidEntityData`] if `updated_at` is earlier than `created_at`,
    /// or SQLite failures. Same-ID same-content is idempotent.
    pub fn create_agent_profile(&mut self, profile: &AgentProfile) -> Result<(), StorageError> {
        if profile.updated_at < profile.created_at {
            return Err(StorageError::InvalidEntityData {
                detail: "updated_at cannot be earlier than created_at".to_owned(),
            });
        }
        let created_at = i64::try_from(profile.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", profile.created_at.as_millis()),
            }
        })?;
        let updated_at = i64::try_from(profile.updated_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("updated_at {} exceeds i64", profile.updated_at.as_millis()),
            }
        })?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("create_agent_profile", error))?;

        let res = tx.execute(
            "INSERT INTO agent_profile
                 (agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                profile.id.as_str(),
                profile.display_name.as_str(),
                profile.preferred_harness.as_str(),
                profile.memory_mode.as_str(),
                created_at,
                updated_at,
            ],
        );

        match res {
            Ok(_) => {
                tx.commit()
                    .map_err(|error| StorageError::from_sqlite("create_agent_profile", error))?;
                Ok(())
            }
            Err(ref err) if is_unique_constraint_violation(err) => {
                let mut stmt = tx
                    .prepare(
                        "SELECT agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at
                         FROM agent_profile WHERE agent_profile_id = ?1",
                    )
                    .map_err(|e| StorageError::from_sqlite("create_agent_profile", e))?;
                let mut rows = stmt
                    .query(params![profile.id.as_str()])
                    .map_err(|e| StorageError::from_sqlite("create_agent_profile", e))?;
                let existing = if let Some(row) = rows
                    .next()
                    .map_err(|e| StorageError::from_sqlite("create_agent_profile", e))?
                {
                    row_to_agent_profile(row)?
                } else {
                    return Err(StorageError::AgentProfileAlreadyExists {
                        agent_profile_id: profile.id.to_string(),
                    });
                };
                if existing == *profile {
                    Ok(())
                } else {
                    Err(StorageError::AgentProfileAlreadyExists {
                        agent_profile_id: profile.id.to_string(),
                    })
                }
            }
            Err(other) => Err(StorageError::from_sqlite("create_agent_profile", other)),
        }
    }

    /// Updates an existing agent profile.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::AgentProfileNotFound`] if the profile does not exist,
    /// [`StorageError::InvalidEntityData`] if `created_at` is modified or `updated_at` is earlier than `created_at`,
    /// or SQLite failures.
    pub fn update_agent_profile(&mut self, profile: &AgentProfile) -> Result<(), StorageError> {
        let existing = self.agent_profile_by_id(&profile.id)?.ok_or_else(|| {
            StorageError::AgentProfileNotFound {
                agent_profile_id: profile.id.to_string(),
            }
        })?;

        if existing.created_at != profile.created_at {
            return Err(StorageError::InvalidEntityData {
                detail: format!(
                    "cannot modify immutable created_at for agent profile {}",
                    profile.id
                ),
            });
        }
        if profile.updated_at < existing.created_at {
            return Err(StorageError::InvalidEntityData {
                detail: "updated_at cannot be earlier than created_at".to_owned(),
            });
        }
        if existing == *profile {
            return Ok(());
        }

        let updated_at = i64::try_from(profile.updated_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("updated_at {} exceeds i64", profile.updated_at.as_millis()),
            }
        })?;
        let affected = self
            .conn
            .execute(
                "UPDATE agent_profile
                 SET display_name = ?1, preferred_harness = ?2, memory_mode = ?3, updated_at = ?4
                 WHERE agent_profile_id = ?5",
                params![
                    profile.display_name.as_str(),
                    profile.preferred_harness.as_str(),
                    profile.memory_mode.as_str(),
                    updated_at,
                    profile.id.as_str(),
                ],
            )
            .map_err(|error| StorageError::from_sqlite("update_agent_profile", error))?;
        if affected == 0 {
            return Err(StorageError::AgentProfileNotFound {
                agent_profile_id: profile.id.to_string(),
            });
        }
        Ok(())
    }

    /// Upserts an agent profile (inserts or updates on conflict).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidEntityData`] if `created_at` is modified or `updated_at` is earlier than `created_at`,
    /// or SQLite failures.
    pub fn upsert_agent_profile(&mut self, profile: &AgentProfile) -> Result<(), StorageError> {
        if profile.updated_at < profile.created_at {
            return Err(StorageError::InvalidEntityData {
                detail: "updated_at cannot be earlier than created_at".to_owned(),
            });
        }

        let created_at = i64::try_from(profile.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", profile.created_at.as_millis()),
            }
        })?;
        let updated_at = i64::try_from(profile.updated_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("updated_at {} exceeds i64", profile.updated_at.as_millis()),
            }
        })?;
        let affected = self
            .conn
            .execute(
                "INSERT INTO agent_profile
                     (agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(agent_profile_id) DO UPDATE SET
                     display_name = excluded.display_name,
                     preferred_harness = excluded.preferred_harness,
                     memory_mode = excluded.memory_mode,
                     updated_at = excluded.updated_at
                 WHERE agent_profile.created_at = excluded.created_at",
                params![
                    profile.id.as_str(),
                    profile.display_name.as_str(),
                    profile.preferred_harness.as_str(),
                    profile.memory_mode.as_str(),
                    created_at,
                    updated_at,
                ],
            )
            .map_err(|error| StorageError::from_sqlite("upsert_agent_profile", error))?;

        if affected == 0 {
            return Err(StorageError::InvalidEntityData {
                detail: format!(
                    "cannot modify immutable created_at for agent profile {}",
                    profile.id
                ),
            });
        }

        Ok(())
    }

    /// Returns an agent profile by its ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn agent_profile_by_id(
        &self,
        id: &AgentProfileId,
    ) -> Result<Option<AgentProfile>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at
                 FROM agent_profile WHERE agent_profile_id = ?1",
            )
            .map_err(|error| StorageError::from_sqlite("agent_profile_by_id", error))?;
        let mut rows = stmt
            .query(params![id.as_str()])
            .map_err(|error| StorageError::from_sqlite("agent_profile_by_id", error))?;
        if let Some(row) = rows
            .next()
            .map_err(|error| StorageError::from_sqlite("agent_profile_by_id", error))?
        {
            row_to_agent_profile(row).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns a bounded page of agent profiles, ordered by `updated_at` descending,
    /// with `agent_profile_id` ascending as unique tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn agent_profiles(
        &self,
        before: Option<&AgentProfileCursor>,
        limit: AgentProfileListLimit,
    ) -> Result<Vec<AgentProfile>, StorageError> {
        let (before_updated_at, before_id) = before.map_or((i64::MAX, ""), |cursor| {
            (
                i64::try_from(cursor.updated_at.as_millis()).unwrap_or(i64::MAX),
                cursor.agent_profile_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at
                 FROM agent_profile
                 WHERE (updated_at < ?1 OR (updated_at = ?1 AND agent_profile_id > ?2))
                 ORDER BY updated_at DESC, agent_profile_id ASC
                 LIMIT ?3",
            )
            .map_err(|error| StorageError::from_sqlite("agent_profiles", error))?;
        let rows = statement
            .query(params![
                before_updated_at,
                before_id,
                i64::from(limit.get())
            ])
            .map_err(|error| StorageError::from_sqlite("agent_profiles", error))?;
        collect_rows(rows, row_to_agent_profile, "agent_profiles")
    }

    // ── AcpHarnessBinding CRUD ─────────────────────────────────────

    /// Creates an ACP harness binding.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::AgentProfileNotFound`] if the referenced agent profile does not exist,
    /// [`StorageError::HarnessBindingAlreadyExists`] if the binding ID already exists with conflicting data,
    /// or SQLite failures. Same-ID same-content is idempotent.
    pub fn create_harness_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<(), StorageError> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("create_harness_binding", error))?;

        let agent_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM agent_profile WHERE agent_profile_id = ?1",
                params![binding.agent_profile_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| {
                StorageError::from_sqlite("create_harness_binding check agent", error)
            })?;
        if agent_exists == 0 {
            return Err(StorageError::AgentProfileNotFound {
                agent_profile_id: binding.agent_profile_id.to_string(),
            });
        }

        let created_at = i64::try_from(binding.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", binding.created_at.as_millis()),
            }
        })?;

        let res = tx.execute(
            "INSERT INTO harness_binding
                 (harness_binding_id, agent_profile_id, label, command, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                binding.id.as_str(),
                binding.agent_profile_id.as_str(),
                binding.label.as_str(),
                binding.command.as_str(),
                created_at,
            ],
        );

        match res {
            Ok(_) => {
                tx.commit()
                    .map_err(|error| StorageError::from_sqlite("create_harness_binding", error))?;
                Ok(())
            }
            Err(ref err) if is_unique_constraint_violation(err) => {
                let mut stmt = tx
                    .prepare(
                        "SELECT harness_binding_id, agent_profile_id, label, command, created_at
                         FROM harness_binding WHERE harness_binding_id = ?1",
                    )
                    .map_err(|e| StorageError::from_sqlite("create_harness_binding", e))?;
                let mut rows = stmt
                    .query(params![binding.id.as_str()])
                    .map_err(|e| StorageError::from_sqlite("create_harness_binding", e))?;
                let existing = if let Some(row) = rows
                    .next()
                    .map_err(|e| StorageError::from_sqlite("create_harness_binding", e))?
                {
                    row_to_harness_binding(row)?
                } else {
                    return Err(StorageError::HarnessBindingAlreadyExists {
                        harness_binding_id: binding.id.to_string(),
                    });
                };
                if existing == *binding {
                    Ok(())
                } else {
                    Err(StorageError::HarnessBindingAlreadyExists {
                        harness_binding_id: binding.id.to_string(),
                    })
                }
            }
            Err(other) => Err(StorageError::from_sqlite("create_harness_binding", other)),
        }
    }

    /// Upserts an ACP harness binding (validates agent profile exists and immutable fields).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::AgentProfileNotFound`] if the referenced agent profile does not exist,
    /// [`StorageError::InvalidEntityData`] if immutable fields (`agent_profile_id`, `created_at`) are modified,
    /// or SQLite failures.
    pub fn upsert_harness_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<(), StorageError> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("upsert_harness_binding", error))?;

        let agent_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM agent_profile WHERE agent_profile_id = ?1",
                params![binding.agent_profile_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| {
                StorageError::from_sqlite("upsert_harness_binding check agent", error)
            })?;
        if agent_exists == 0 {
            return Err(StorageError::AgentProfileNotFound {
                agent_profile_id: binding.agent_profile_id.to_string(),
            });
        }

        let created_at = i64::try_from(binding.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", binding.created_at.as_millis()),
            }
        })?;
        let affected = tx
            .execute(
                "INSERT INTO harness_binding
                     (harness_binding_id, agent_profile_id, label, command, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(harness_binding_id) DO UPDATE SET
                     label = excluded.label,
                     command = excluded.command
                 WHERE harness_binding.agent_profile_id = excluded.agent_profile_id
                   AND harness_binding.created_at = excluded.created_at",
                params![
                    binding.id.as_str(),
                    binding.agent_profile_id.as_str(),
                    binding.label.as_str(),
                    binding.command.as_str(),
                    created_at,
                ],
            )
            .map_err(|error| StorageError::from_sqlite("upsert_harness_binding", error))?;
        if affected == 0 {
            return Err(StorageError::InvalidEntityData {
                detail: format!(
                    "cannot modify immutable agent_profile_id or created_at for harness binding {}",
                    binding.id
                ),
            });
        }
        tx.commit()
            .map_err(|error| StorageError::from_sqlite("upsert_harness_binding", error))?;
        Ok(())
    }

    /// Returns a harness binding by its ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn harness_binding_by_id(
        &self,
        id: &HarnessBindingId,
    ) -> Result<Option<AcpHarnessBinding>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT harness_binding_id, agent_profile_id, label, command, created_at
                 FROM harness_binding WHERE harness_binding_id = ?1",
            )
            .map_err(|error| StorageError::from_sqlite("harness_binding_by_id", error))?;
        let mut rows = stmt
            .query(params![id.as_str()])
            .map_err(|error| StorageError::from_sqlite("harness_binding_by_id", error))?;
        if let Some(row) = rows
            .next()
            .map_err(|error| StorageError::from_sqlite("harness_binding_by_id", error))?
        {
            row_to_harness_binding(row).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns a bounded page of harness bindings for an agent profile,
    /// ordered by `created_at` ascending, with `harness_binding_id` ascending as tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn harness_bindings_for_agent(
        &self,
        agent_id: &AgentProfileId,
        after: Option<&HarnessBindingCursor>,
        limit: HarnessBindingListLimit,
    ) -> Result<Vec<AcpHarnessBinding>, StorageError> {
        let (after_created_at, after_id) = after.map_or((-1, ""), |cursor| {
            (
                i64::try_from(cursor.created_at.as_millis()).unwrap_or(-1),
                cursor.harness_binding_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT harness_binding_id, agent_profile_id, label, command, created_at
                 FROM harness_binding
                 WHERE agent_profile_id = ?1
                   AND (created_at > ?2 OR (created_at = ?2 AND harness_binding_id > ?3))
                 ORDER BY created_at ASC, harness_binding_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("harness_bindings_for_agent", error))?;
        let rows = statement
            .query(params![
                agent_id.as_str(),
                after_created_at,
                after_id,
                i64::from(limit.get())
            ])
            .map_err(|error| StorageError::from_sqlite("harness_bindings_for_agent", error))?;
        collect_rows(rows, row_to_harness_binding, "harness_bindings_for_agent")
    }

    /// Deletes a harness binding by ID. Returns `true` if deleted, `false` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn delete_harness_binding(&mut self, id: &HarnessBindingId) -> Result<bool, StorageError> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM harness_binding WHERE harness_binding_id = ?1",
                params![id.as_str()],
            )
            .map_err(|error| StorageError::from_sqlite("delete_harness_binding", error))?;
        Ok(affected > 0)
    }

    // ── ProjectRef CRUD ────────────────────────────────���───────────

    /// Creates a project reference.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ProjectRefAlreadyExists`] if the ID already exists with conflicting data,
    /// or SQLite failures. Same-ID same-content is idempotent.
    pub fn create_project_ref(&mut self, project: &ProjectRef) -> Result<(), StorageError> {
        let created_at = i64::try_from(project.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", project.created_at.as_millis()),
            }
        })?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("create_project_ref", error))?;

        let res = tx.execute(
            "INSERT INTO project_ref (project_id, label, path, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                project.id.as_str(),
                project.label.as_str(),
                project.path.as_str(),
                created_at,
            ],
        );

        match res {
            Ok(_) => {
                tx.commit()
                    .map_err(|error| StorageError::from_sqlite("create_project_ref", error))?;
                Ok(())
            }
            Err(ref err) if is_unique_constraint_violation(err) => {
                let mut stmt = tx
                    .prepare(
                        "SELECT project_id, label, path, created_at
                         FROM project_ref WHERE project_id = ?1",
                    )
                    .map_err(|e| StorageError::from_sqlite("create_project_ref", e))?;
                let mut rows = stmt
                    .query(params![project.id.as_str()])
                    .map_err(|e| StorageError::from_sqlite("create_project_ref", e))?;
                let existing = if let Some(row) = rows
                    .next()
                    .map_err(|e| StorageError::from_sqlite("create_project_ref", e))?
                {
                    row_to_project_ref(row)?
                } else {
                    return Err(StorageError::ProjectRefAlreadyExists {
                        project_id: project.id.to_string(),
                    });
                };
                if existing == *project {
                    Ok(())
                } else {
                    Err(StorageError::ProjectRefAlreadyExists {
                        project_id: project.id.to_string(),
                    })
                }
            }
            Err(other) => Err(StorageError::from_sqlite("create_project_ref", other)),
        }
    }

    /// Upserts a project reference (validates immutable `created_at`).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidEntityData`] if immutable `created_at` is modified,
    /// or SQLite failures.
    pub fn upsert_project_ref(&mut self, project: &ProjectRef) -> Result<(), StorageError> {
        let created_at = i64::try_from(project.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", project.created_at.as_millis()),
            }
        })?;
        let affected = self
            .conn
            .execute(
                "INSERT INTO project_ref (project_id, label, path, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project_id) DO UPDATE SET
                     label = excluded.label,
                     path = excluded.path
                 WHERE project_ref.created_at = excluded.created_at",
                params![
                    project.id.as_str(),
                    project.label.as_str(),
                    project.path.as_str(),
                    created_at,
                ],
            )
            .map_err(|error| StorageError::from_sqlite("upsert_project_ref", error))?;
        if affected == 0 {
            return Err(StorageError::InvalidEntityData {
                detail: format!(
                    "cannot modify immutable created_at for project ref {}",
                    project.id
                ),
            });
        }
        Ok(())
    }

    /// Returns a project reference by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn project_ref_by_id(&self, id: &ProjectId) -> Result<Option<ProjectRef>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT project_id, label, path, created_at
                 FROM project_ref WHERE project_id = ?1",
            )
            .map_err(|error| StorageError::from_sqlite("project_ref_by_id", error))?;
        let mut rows = stmt
            .query(params![id.as_str()])
            .map_err(|error| StorageError::from_sqlite("project_ref_by_id", error))?;
        if let Some(row) = rows
            .next()
            .map_err(|error| StorageError::from_sqlite("project_ref_by_id", error))?
        {
            row_to_project_ref(row).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns a bounded page of project references, ordered by `created_at` ascending,
    /// with `project_id` ascending as tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn project_refs(
        &self,
        after: Option<&ProjectRefCursor>,
        limit: ProjectRefListLimit,
    ) -> Result<Vec<ProjectRef>, StorageError> {
        let (after_created_at, after_id) = after.map_or((-1, ""), |cursor| {
            (
                i64::try_from(cursor.created_at.as_millis()).unwrap_or(-1),
                cursor.project_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT project_id, label, path, created_at
                 FROM project_ref
                 WHERE (created_at > ?1 OR (created_at = ?1 AND project_id > ?2))
                 ORDER BY created_at ASC, project_id ASC
                 LIMIT ?3",
            )
            .map_err(|error| StorageError::from_sqlite("project_refs", error))?;
        let rows = statement
            .query(params![after_created_at, after_id, i64::from(limit.get())])
            .map_err(|error| StorageError::from_sqlite("project_refs", error))?;
        collect_rows(rows, row_to_project_ref, "project_refs")
    }

    /// Deletes a project reference safely. If any thread in the thread projection
    /// references this project ID, deletion is refused with [`StorageError::ProjectReferencedByThreads`].
    /// Returns `true` if deleted, `false` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ProjectReferencedByThreads`] if referenced, or SQLite failures.
    pub fn delete_project_ref(&mut self, id: &ProjectId) -> Result<bool, StorageError> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| StorageError::from_sqlite("delete_project_ref", error))?;
        let thread_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM thread WHERE project_id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| {
                StorageError::from_sqlite("delete_project_ref check threads", error)
            })?;
        if thread_count > 0 {
            return Err(StorageError::ProjectReferencedByThreads {
                project_id: id.to_string(),
                thread_count: usize::try_from(thread_count).unwrap_or(usize::MAX),
            });
        }
        let affected = tx
            .execute(
                "DELETE FROM project_ref WHERE project_id = ?1",
                params![id.as_str()],
            )
            .map_err(|error| StorageError::from_sqlite("delete_project_ref", error))?;
        tx.commit()
            .map_err(|error| StorageError::from_sqlite("delete_project_ref", error))?;
        Ok(affected > 0)
    }

    // ── Permission queries ─────────────────────────────────────────

    /// Returns a permission request by its event ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn permission_by_event_id(
        &self,
        event_id: &EventId,
    ) -> Result<Option<Permission>, StorageError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT event_id, turn_id, thread_id, kind, description, decision, requested_at, decided_at
                 FROM permission WHERE event_id = ?1",
            )
            .map_err(|error| StorageError::from_sqlite("permission_by_event_id", error))?;
        let mut rows = statement
            .query(params![event_id.as_str()])
            .map_err(|error| StorageError::from_sqlite("permission_by_event_id", error))?;
        if let Some(row) = rows
            .next()
            .map_err(|error| StorageError::from_sqlite("permission_by_event_id", error))?
        {
            row_to_permission(row).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns a bounded page of permissions for a turn, ordered by `requested_at` ascending,
    /// with `event_id` ascending as unique tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn permissions_for_turn(
        &self,
        turn_id: &TurnId,
        after: Option<&PermissionCursor>,
        limit: PermissionListLimit,
    ) -> Result<Vec<Permission>, StorageError> {
        let (after_requested_at, after_id) = after.map_or((-1, ""), |cursor| {
            (
                i64::try_from(cursor.requested_at.as_millis()).unwrap_or(-1),
                cursor.event_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT event_id, turn_id, thread_id, kind, description, decision, requested_at, decided_at
                 FROM permission
                 WHERE turn_id = ?1
                   AND (requested_at > ?2 OR (requested_at = ?2 AND event_id > ?3))
                 ORDER BY requested_at ASC, event_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("permissions_for_turn", error))?;
        let rows = statement
            .query(params![
                turn_id.as_str(),
                after_requested_at,
                after_id,
                i64::from(limit.get())
            ])
            .map_err(|error| StorageError::from_sqlite("permissions_for_turn", error))?;
        collect_rows(rows, row_to_permission, "permissions_for_turn")
    }

    /// Returns a bounded page of permissions for a thread, ordered by `requested_at` ascending,
    /// with `event_id` ascending as unique tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn permissions_for_thread(
        &self,
        thread_id: &ThreadId,
        after: Option<&PermissionCursor>,
        limit: PermissionListLimit,
    ) -> Result<Vec<Permission>, StorageError> {
        let (after_requested_at, after_id) = after.map_or((-1, ""), |cursor| {
            (
                i64::try_from(cursor.requested_at.as_millis()).unwrap_or(-1),
                cursor.event_id.as_str(),
            )
        });
        let mut statement = self
            .conn
            .prepare(
                "SELECT event_id, turn_id, thread_id, kind, description, decision, requested_at, decided_at
                 FROM permission
                 WHERE thread_id = ?1
                   AND (requested_at > ?2 OR (requested_at = ?2 AND event_id > ?3))
                 ORDER BY requested_at ASC, event_id ASC
                 LIMIT ?4",
            )
            .map_err(|error| StorageError::from_sqlite("permissions_for_thread", error))?;
        let rows = statement
            .query(params![
                thread_id.as_str(),
                after_requested_at,
                after_id,
                i64::from(limit.get())
            ])
            .map_err(|error| StorageError::from_sqlite("permissions_for_thread", error))?;
        collect_rows(rows, row_to_permission, "permissions_for_thread")
    }

    /// Rebuilds domain projections from the domain journal in one
    /// transaction (ADR 0013).
    ///
    /// Returns the number of domain events replayed.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure; on error the
    /// transaction rolls back and the previous projection survives.
    #[allow(clippy::too_many_lines)]
    pub fn rebuild_domain_projections(&mut self) -> Result<i64, StorageError> {
        let tx = self
            .conn
            .transaction()
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;

        // Repair the external-content index against the current `thread`
        // projection before deleting rows. Otherwise a drifted index may make
        // the FTS delete trigger fail while projections are being cleared.
        // This uses SQLite's public FTS5 maintenance command, never private
        // shadow tables.
        tx.execute(
            "INSERT INTO thread_search(thread_search) VALUES('rebuild')",
            [],
        )
        .map_err(|error| StorageError::from_sqlite("domain_rebuild preflight fts", error))?;

        // Wipe all derived projections. FTS is rebuilt again from the
        // journal-replayed `thread` projection below.
        tx.execute("DELETE FROM permission", [])
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;
        tx.execute("DELETE FROM turn", [])
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;
        tx.execute("DELETE FROM thread", [])
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;

        // Replay every domain journal event in order.
        let mut stmt = tx
            .prepare(
                "SELECT seq, event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at
                 FROM domain_journal ORDER BY seq",
            )
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;

        let journal_rows: Vec<DomainJournalRow> = stmt
            .query_map([], |row| {
                Ok(DomainJournalRow {
                    seq: row.get(0)?,
                    event_id: row.get(1)?,
                    thread_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    operation_id: row.get(4)?,
                    kind: row.get(5)?,
                    payload: row.get(6)?,
                    occurred_at: row.get(7)?,
                })
            })
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;

        drop(stmt);

        let count = i64::try_from(journal_rows.len()).unwrap_or(i64::MAX);
        for row in &journal_rows {
            let domain_event = DomainEvent {
                event_id: row
                    .event_id
                    .parse()
                    .map_err(|_| StorageError::RebuildInvariant {
                        detail: format!("invalid event_id in domain journal: {}", row.event_id),
                    })?,
                thread_id: row
                    .thread_id
                    .as_ref()
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| StorageError::RebuildInvariant {
                        detail: format!("invalid thread_id in domain journal: {:?}", row.thread_id),
                    })?,
                turn_id: row
                    .turn_id
                    .as_ref()
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| StorageError::RebuildInvariant {
                        detail: format!("invalid turn_id in domain journal: {:?}", row.turn_id),
                    })?,
                operation_id: row
                    .operation_id
                    .as_ref()
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| StorageError::RebuildInvariant {
                        detail: format!(
                            "invalid operation_id in domain journal: {:?}",
                            row.operation_id
                        ),
                    })?,
                kind: DomainEventKind::try_from_str(&row.kind).map_err(|error| {
                    StorageError::RebuildInvariant {
                        detail: format!("invalid kind in domain journal: {error}"),
                    }
                })?,
                payload: EventPayload::try_from(row.payload.as_slice()).map_err(
                    |e: EntityError| StorageError::RebuildInvariant {
                        detail: format!("invalid payload in domain journal: {e}"),
                    },
                )?,
                occurred_at: UnixMillis::from_millis(u64::try_from(row.occurred_at).map_err(
                    |_| StorageError::RebuildInvariant {
                        detail: format!(
                            "negative occurred_at in domain journal: {}",
                            row.occurred_at
                        ),
                    },
                )?),
            };
            validate_domain_event_in_tx(&tx, &domain_event)?;
            fold_domain_event_in_tx(&tx, &domain_event, row.seq, row.occurred_at)?;
        }

        // Rebuild FTS5 virtual table to be 100% clean and consistent with projected threads.
        tx.execute(
            "INSERT INTO thread_search(thread_search) VALUES('rebuild')",
            [],
        )
        .map_err(|error| StorageError::from_sqlite("domain_rebuild fts", error))?;

        // Store the checksum after every replay; normal opens compare this
        // projection-sized value rather than replaying the full journal.
        let digest = domain_projection_digest(&tx)?;
        tx.execute(
            "INSERT INTO domain_projection_state (id, journal_max_seq, projection_version, rebuilt_at, projection_digest)
             SELECT 1, COALESCE(MAX(seq), 0), ?1, COALESCE(MAX(occurred_at), 0), ?2 FROM domain_journal
             WHERE true
             ON CONFLICT(id) DO UPDATE SET
                 journal_max_seq = excluded.journal_max_seq,
                 projection_version = excluded.projection_version,
                 rebuilt_at = excluded.rebuilt_at,
                 projection_digest = excluded.projection_digest",
            params![DOMAIN_PROJECTION_VERSION, digest],
        )
        .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;

        tx.commit()
            .map_err(|error| StorageError::from_sqlite("domain_rebuild", error))?;
        Ok(count)
    }

    /// Heals domain projections on open.
    fn ensure_domain_projections_current(&mut self) -> Result<(), StorageError> {
        let marker: Option<(i64, i64, String)> = self
            .conn
            .query_row(
                "SELECT journal_max_seq, projection_version, projection_digest FROM domain_projection_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(StorageError::from_sqlite("ensure_domain_current", other)),
            })?;
        let journal_max = self.domain_journal_max_seq()?;
        let needs_rebuild = match &marker {
            None => true,
            Some((_, version, _)) if *version != DOMAIN_PROJECTION_VERSION => true,
            Some((marker_seq, _, _)) if *marker_seq < journal_max => true,
            Some((marker_seq, _, _)) if *marker_seq > journal_max => {
                return Err(StorageError::RebuildInvariant {
                    detail: format!(
                        "domain projection marker {marker_seq} is ahead of domain journal max {journal_max}; \
                         journal history cannot shrink"
                    ),
                });
            }
            Some((_, _, stored_digest)) => {
                if stored_digest.is_empty() {
                    true
                } else {
                    let live_digest_match = match domain_projection_digest(&self.conn) {
                        Ok(live_digest) => live_digest == *stored_digest,
                        Err(_) => false,
                    };
                    let fts_ok = check_fts_consistency(&self.conn);
                    !live_digest_match || !fts_ok
                }
            }
        };

        if needs_rebuild {
            self.rebuild_domain_projections()?;
            // Verify new marker and digest after rebuild.
            let post_marker: (i64, i64, String) = self
                .conn
                .query_row(
                    "SELECT journal_max_seq, projection_version, projection_digest FROM domain_projection_state WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(|error| StorageError::from_sqlite("ensure_domain_current_verify", error))?;
            let post_digest = domain_projection_digest(&self.conn)?;
            if post_marker.0 != journal_max {
                return Err(StorageError::RebuildInvariant {
                    detail: format!(
                        "domain projection marker seq mismatch after rebuild: stored={}, journal_max={}",
                        post_marker.0, journal_max
                    ),
                });
            }
            if post_marker.1 != DOMAIN_PROJECTION_VERSION {
                return Err(StorageError::RebuildInvariant {
                    detail: format!(
                        "domain projection marker version mismatch after rebuild: stored={}, expected={}",
                        post_marker.1, DOMAIN_PROJECTION_VERSION
                    ),
                });
            }
            if post_marker.2 != post_digest || post_marker.2.is_empty() {
                return Err(StorageError::RebuildInvariant {
                    detail: format!(
                        "domain projection digest mismatch after rebuild: stored={}, live={}",
                        post_marker.2, post_digest
                    ),
                });
            }
            if !check_fts_consistency(&self.conn) {
                return Err(StorageError::RebuildInvariant {
                    detail: "FTS5 external-content index remains inconsistent after rebuild"
                        .to_owned(),
                });
            }
        }
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn is_unique_constraint_violation(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(ffi_err, _) => {
            ffi_err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                || ffi_err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || ffi_err.code == rusqlite::ErrorCode::ConstraintViolation
        }
        _ => false,
    }
}

fn row_to_journal_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalRow> {
    Ok(JournalRow {
        seq: row.get(0)?,
        event_id: row.get(1)?,
        thread_id: row.get(2)?,
        turn_id: row.get(3)?,
        stream_sequence: row.get(4)?,
        kind: row.get(5)?,
        payload: row.get(6)?,
        occurred_at: row.get(7)?,
    })
}

/// Maps one thread row.
fn row_to_thread_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRow> {
    Ok(ThreadRow {
        thread_id: row.get(0)?,
        agent_profile_id: row.get(1)?,
        title: row.get(2)?,
        state: row.get(3)?,
        project_id: row.get(4)?,
        event_count: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// Maps one agent profile row.
fn row_to_agent_profile(row: &rusqlite::Row<'_>) -> Result<AgentProfile, StorageError> {
    let id_raw: String = row
        .get(0)
        .map_err(|e| StorageError::from_sqlite("read agent_profile_id", e))?;
    let name_raw: String = row
        .get(1)
        .map_err(|e| StorageError::from_sqlite("read display_name", e))?;
    let harness_raw: String = row
        .get(2)
        .map_err(|e| StorageError::from_sqlite("read preferred_harness", e))?;
    let memory_raw: String = row
        .get(3)
        .map_err(|e| StorageError::from_sqlite("read memory_mode", e))?;
    let created_at_raw: i64 = row
        .get(4)
        .map_err(|e| StorageError::from_sqlite("read created_at", e))?;
    let updated_at_raw: i64 = row
        .get(5)
        .map_err(|e| StorageError::from_sqlite("read updated_at", e))?;

    let id = id_raw
        .parse::<AgentProfileId>()
        .map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid agent_profile_id {id_raw}: {e}"),
        })?;
    let display_name =
        DisplayName::try_from(name_raw.as_str()).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid display_name {name_raw}: {e}"),
        })?;
    let preferred_harness =
        HarnessKind::try_from_str(&harness_raw).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid preferred_harness {harness_raw}: {e}"),
        })?;
    let memory_mode =
        MemoryMode::try_from_str(&memory_raw).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid memory_mode {memory_raw}: {e}"),
        })?;
    let created_at = UnixMillis::from_millis(u64::try_from(created_at_raw).map_err(|_| {
        StorageError::InvalidEntityData {
            detail: format!("negative created_at {created_at_raw}"),
        }
    })?);
    let updated_at = UnixMillis::from_millis(u64::try_from(updated_at_raw).map_err(|_| {
        StorageError::InvalidEntityData {
            detail: format!("negative updated_at {updated_at_raw}"),
        }
    })?);

    Ok(AgentProfile {
        id,
        display_name,
        preferred_harness,
        memory_mode,
        created_at,
        updated_at,
    })
}

/// Maps one harness binding row.
fn row_to_harness_binding(row: &rusqlite::Row<'_>) -> Result<AcpHarnessBinding, StorageError> {
    let id_raw: String = row
        .get(0)
        .map_err(|e| StorageError::from_sqlite("read harness_binding_id", e))?;
    let agent_id_raw: String = row
        .get(1)
        .map_err(|e| StorageError::from_sqlite("read agent_profile_id", e))?;
    let label_raw: String = row
        .get(2)
        .map_err(|e| StorageError::from_sqlite("read label", e))?;
    let command_raw: String = row
        .get(3)
        .map_err(|e| StorageError::from_sqlite("read command", e))?;
    let created_at_raw: i64 = row
        .get(4)
        .map_err(|e| StorageError::from_sqlite("read created_at", e))?;

    let id = id_raw
        .parse::<HarnessBindingId>()
        .map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid harness_binding_id {id_raw}: {e}"),
        })?;
    let agent_profile_id =
        agent_id_raw
            .parse::<AgentProfileId>()
            .map_err(|e| StorageError::InvalidEntityData {
                detail: format!("invalid agent_profile_id {agent_id_raw}: {e}"),
            })?;
    let label =
        DisplayName::try_from(label_raw.as_str()).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid label {label_raw}: {e}"),
        })?;
    let command = BoundedPath::try_from(command_raw.as_str()).map_err(|e| {
        StorageError::InvalidEntityData {
            detail: format!("invalid command {command_raw}: {e}"),
        }
    })?;
    let created_at = UnixMillis::from_millis(u64::try_from(created_at_raw).map_err(|_| {
        StorageError::InvalidEntityData {
            detail: format!("negative created_at {created_at_raw}"),
        }
    })?);

    Ok(AcpHarnessBinding {
        id,
        agent_profile_id,
        label,
        command,
        created_at,
    })
}

/// Maps one project reference row.
fn row_to_project_ref(row: &rusqlite::Row<'_>) -> Result<ProjectRef, StorageError> {
    let id_raw: String = row
        .get(0)
        .map_err(|e| StorageError::from_sqlite("read project_id", e))?;
    let label_raw: String = row
        .get(1)
        .map_err(|e| StorageError::from_sqlite("read label", e))?;
    let path_raw: String = row
        .get(2)
        .map_err(|e| StorageError::from_sqlite("read path", e))?;
    let created_at_raw: i64 = row
        .get(3)
        .map_err(|e| StorageError::from_sqlite("read created_at", e))?;

    let id = id_raw
        .parse::<ProjectId>()
        .map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid project_id {id_raw}: {e}"),
        })?;
    let label = BoundedLabel::try_from(label_raw.as_str()).map_err(|e| {
        StorageError::InvalidEntityData {
            detail: format!("invalid label {label_raw}: {e}"),
        }
    })?;
    let path =
        BoundedPath::try_from(path_raw.as_str()).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid path {path_raw}: {e}"),
        })?;
    let created_at = UnixMillis::from_millis(u64::try_from(created_at_raw).map_err(|_| {
        StorageError::InvalidEntityData {
            detail: format!("negative created_at {created_at_raw}"),
        }
    })?);

    Ok(ProjectRef {
        id,
        label,
        path,
        created_at,
    })
}

/// Maps one permission row.
fn row_to_permission(row: &rusqlite::Row<'_>) -> Result<Permission, StorageError> {
    let event_id_raw: String = row
        .get(0)
        .map_err(|e| StorageError::from_sqlite("read event_id", e))?;
    let turn_id_raw: String = row
        .get(1)
        .map_err(|e| StorageError::from_sqlite("read turn_id", e))?;
    let thread_id_raw: String = row
        .get(2)
        .map_err(|e| StorageError::from_sqlite("read thread_id", e))?;
    let kind_raw: String = row
        .get(3)
        .map_err(|e| StorageError::from_sqlite("read kind", e))?;
    let desc_raw: String = row
        .get(4)
        .map_err(|e| StorageError::from_sqlite("read description", e))?;
    let decision_raw: String = row
        .get(5)
        .map_err(|e| StorageError::from_sqlite("read decision", e))?;
    let req_at_raw: i64 = row
        .get(6)
        .map_err(|e| StorageError::from_sqlite("read requested_at", e))?;
    let dec_at_raw: Option<i64> = row
        .get(7)
        .map_err(|e| StorageError::from_sqlite("read decided_at", e))?;

    let event_id =
        event_id_raw
            .parse::<EventId>()
            .map_err(|e| StorageError::InvalidEntityData {
                detail: format!("invalid event_id {event_id_raw}: {e}"),
            })?;
    let turn_id = turn_id_raw
        .parse::<TurnId>()
        .map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid turn_id {turn_id_raw}: {e}"),
        })?;
    let thread_id =
        thread_id_raw
            .parse::<ThreadId>()
            .map_err(|e| StorageError::InvalidEntityData {
                detail: format!("invalid thread_id {thread_id_raw}: {e}"),
            })?;
    let kind =
        PermissionKind::try_from_str(&kind_raw).map_err(|e| StorageError::InvalidEntityData {
            detail: format!("invalid permission kind {kind_raw}: {e}"),
        })?;
    let description = PermissionDescription::try_from(desc_raw.as_str()).map_err(|e| {
        StorageError::InvalidEntityData {
            detail: format!("invalid description {desc_raw}: {e}"),
        }
    })?;
    let decision = PermissionDecision::try_from_str(&decision_raw).map_err(|e| {
        StorageError::InvalidEntityData {
            detail: format!("invalid decision {decision_raw}: {e}"),
        }
    })?;
    let requested_at = UnixMillis::from_millis(u64::try_from(req_at_raw).map_err(|_| {
        StorageError::InvalidEntityData {
            detail: format!("negative requested_at {req_at_raw}"),
        }
    })?);
    let decided_at = dec_at_raw
        .map(|t| {
            u64::try_from(t).map(UnixMillis::from_millis).map_err(|_| {
                StorageError::InvalidEntityData {
                    detail: format!("negative decided_at {t}"),
                }
            })
        })
        .transpose()?;

    Ok(Permission {
        event_id,
        turn_id,
        thread_id,
        kind,
        description,
        decision,
        requested_at,
        decided_at,
    })
}

/// Collects rows mapped with custom fallible mapper.
fn collect_rows<T>(
    mut rows: rusqlite::Rows<'_>,
    mut map: impl FnMut(&rusqlite::Row<'_>) -> Result<T, StorageError>,
    context: &'static str,
) -> Result<Vec<T>, StorageError> {
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| StorageError::from_sqlite(context, e))?
    {
        out.push(map(row)?);
    }
    Ok(out)
}

/// Collects a mapped query into a `Vec`, mapping failures.
fn collect<T, F>(
    rows: rusqlite::MappedRows<'_, F>,
    context: &'static str,
) -> Result<Vec<T>, StorageError>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| StorageError::from_sqlite(context, error))?);
    }
    Ok(out)
}

/// Looks up the journal sequence of an already-stored event id.
fn existing_event(
    conn: &Connection,
    event_id: &str,
) -> Result<Option<(i64, Vec<u8>)>, StorageError> {
    conn.query_row(
        "SELECT seq, payload FROM journal WHERE event_id = ?1",
        params![event_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(StorageError::from_sqlite("lookup event", other)),
    })
}

#[allow(clippy::type_complexity)]
fn existing_domain_event_tuple(
    conn: &Connection,
    id: &str,
) -> Result<
    Option<(
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Vec<u8>,
        i64,
    )>,
    StorageError,
> {
    conn.query_row("SELECT seq,thread_id,turn_id,operation_id,kind,payload,occurred_at FROM domain_journal WHERE event_id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map(Some).or_else(|e|match e {rusqlite::Error::QueryReturnedNoRows=>Ok(None),e=>Err(StorageError::from_sqlite("domain lookup",e))})
}
#[allow(clippy::type_complexity)]
fn durable_tuple(
    e: &DomainEvent,
    at: i64,
) -> (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Vec<u8>,
    i64,
) {
    (
        0,
        e.thread_id.as_ref().map(ToString::to_string),
        e.turn_id.as_ref().map(ToString::to_string),
        e.operation_id.as_ref().map(ToString::to_string),
        e.kind.as_str().into(),
        e.payload.as_bytes().into(),
        at,
    )
}
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
fn validate_domain_event_in_tx(
    tx: &rusqlite::Transaction<'_>,
    e: &DomainEvent,
) -> Result<(), StorageError> {
    let fail = |d| StorageError::InvalidDomainEvent { detail: d };
    DomainEventKind::try_from_str(e.kind.as_str()).map_err(|x| fail(x.to_string()))?;
    let p: serde_json::Value = serde_json::from_slice(e.payload.as_bytes())
        .map_err(|_| fail("payload is not JSON object".into()))?;
    let o = p
        .as_object()
        .ok_or_else(|| fail("payload is not JSON object".into()))?;
    if matches!(e.kind, DomainEventKind::Other(_)) {
        if e.turn_id.is_some() {
            return Err(fail("custom event cannot have turn scope".into()));
        }
        if e.thread_id.is_none() {
            return Ok(());
        }
    }
    let t = e
        .thread_id
        .as_ref()
        .ok_or_else(|| fail("thread scope required".into()))?;
    let req = |k| {
        o.get(k)
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| fail(format!("missing/wrong {k}")))
    };
    match e.kind {
        DomainEventKind::ThreadCreated => {
            let agp_str = req("agent_profile_id")?;
            agp_str
                .parse::<AgentProfileId>()
                .map_err(|x| fail(format!("invalid agent_profile_id: {x}")))?;
            if let Some(prj_str) = o
                .get("project_id")
                .and_then(serde_json::Value::as_str)
                .filter(|v| !v.is_empty())
            {
                prj_str
                    .parse::<altior_domain::ProjectId>()
                    .map_err(|x| fail(format!("invalid project_id: {x}")))?;
            }
            if let Some(title_val) = o.get("title") {
                let title_str = title_val
                    .as_str()
                    .ok_or_else(|| fail("title must be a string".into()))?;
                if title_str.len() > altior_domain::ThreadTitle::capacity() {
                    return Err(fail("title exceeds capacity".into()));
                }
            }
            if e.turn_id.is_some() {
                return Err(fail("create has turn scope".into()));
            }
        }
        DomainEventKind::ThreadTitleChanged => {
            let title_val = o
                .get("title")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| fail("missing/wrong title".into()))?;
            if title_val.len() > altior_domain::ThreadTitle::capacity() {
                return Err(fail("title exceeds capacity".into()));
            }
        }
        DomainEventKind::ThreadStateChanged => {
            if !matches!(req("state")?, "open" | "pinned" | "archived") {
                return Err(fail("invalid state".into()));
            }
        }
        DomainEventKind::TurnStarted => {
            let turn = e
                .turn_id
                .as_ref()
                .ok_or_else(|| fail("start missing turn".into()))?;
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id = ?1",
                    params![turn.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate turn exists", x))?;
            if exists > 0 {
                return Err(fail("turn already exists".into()));
            }
        }
        DomainEventKind::MessageDelta | DomainEventKind::PermissionRequested => {
            let turn = e
                .turn_id
                .as_ref()
                .ok_or_else(|| fail("turn scope required".into()))?;
            let n: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND thread_id=?2",
                    params![turn.as_str(), t.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate turn", x))?;
            if n != 1 {
                return Err(fail("turn missing/cross-thread".into()));
            }
            let active: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND state='active'",
                    params![turn.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate active turn", x))?;
            if active != 1 {
                return Err(fail("turn is terminal; requires active turn".into()));
            }
            if matches!(e.kind, DomainEventKind::PermissionRequested) {
                req("description")?;
                if !matches!(
                    req("permission_kind")?,
                    "execute" | "read" | "write" | "network"
                ) {
                    return Err(fail("invalid permission kind".into()));
                }
            }
        }
        DomainEventKind::TurnCompleted
        | DomainEventKind::TurnCancelled
        | DomainEventKind::TurnFailed => {
            let turn = e
                .turn_id
                .as_ref()
                .ok_or_else(|| fail("turn scope required".into()))?;
            let n: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND thread_id=?2",
                    params![turn.as_str(), t.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate turn", x))?;
            if n != 1 {
                return Err(fail("turn missing/cross-thread".into()));
            }
            let active: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND state='active'",
                    params![turn.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate terminal", x))?;
            if active != 1 {
                return Err(fail("terminal requires active turn".into()));
            }
        }
        DomainEventKind::PermissionDecided => {
            let turn = e
                .turn_id
                .as_ref()
                .ok_or_else(|| fail("turn scope required".into()))?;
            let n: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND thread_id=?2",
                    params![turn.as_str(), t.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate turn", x))?;
            if n != 1 {
                return Err(fail("turn missing/cross-thread".into()));
            }
            let active: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turn WHERE turn_id=?1 AND state='active'",
                    params![turn.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate permission active turn", x))?;
            if active != 1 {
                return Err(fail(
                    "turn is terminal; permission cannot be decided".into(),
                ));
            }
            let id = req("permission_event_id")?;
            if !matches!(req("decision")?, "approved" | "denied") {
                return Err(fail("invalid decision".into()));
            }
            let n: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM permission WHERE event_id=?1 AND turn_id=?2 AND thread_id=?3 AND decision='pending'",
                    params![id, turn.as_str(), t.as_str()],
                    |r| r.get(0),
                )
                .map_err(|x| StorageError::from_sqlite("validate permission", x))?;
            if n != 1 {
                return Err(fail("decision requires pending permission".into()));
            }
        }
        DomainEventKind::Other(_) => {}
    }
    if !matches!(e.kind, DomainEventKind::ThreadCreated) {
        let n: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM thread WHERE thread_id=?1",
                params![t.as_str()],
                |r| r.get(0),
            )
            .map_err(|x| StorageError::from_sqlite("validate thread", x))?;
        if n != 1 {
            return Err(fail("thread missing".into()));
        }
    }
    Ok(())
}
fn update_thread_activity(
    tx: &rusqlite::Transaction<'_>,
    thread: &str,
    seq: i64,
    id: &str,
    kind: &str,
    at: i64,
) -> Result<(), StorageError> {
    let n=tx.execute("UPDATE thread SET event_count=event_count+1,last_event_seq=?1,last_event_id=?2,last_event_kind=?3,updated_at=?4 WHERE thread_id=?5",params![seq,id,kind,at,thread]).map_err(|e|StorageError::from_sqlite("thread activity",e))?;
    if n != 1 {
        return Err(StorageError::InvalidDomainEvent {
            detail: "thread missing during fold".into(),
        });
    }
    Ok(())
}
fn extract_json_string(p: &[u8], k: &str) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(p)
        .ok()?
        .get(k)?
        .as_str()
        .map(str::to_owned)
}
fn thread_state_to_str(s: ThreadState) -> &'static str {
    match s {
        ThreadState::Open => "open",
        ThreadState::Pinned => "pinned",
        ThreadState::Archived => "archived",
    }
}

/// Formats a search query string as a literal FTS5 phrase query by wrapping it in
/// double quotes and escaping any inner double quotes as `""`.
fn fts5_quoted_literal(input: &str) -> String {
    format!("\"{}\"", input.replace('"', "\"\""))
}

/// Checks both FTS5's internal index and its external-content parity.
///
/// Passing `rank = 1` is required for an external-content FTS5 table:
/// without it SQLite checks only the index structure, not whether it still
/// represents the `thread` projection.
fn check_fts_consistency(conn: &Connection) -> bool {
    conn.execute(
        "INSERT INTO thread_search(thread_search, rank) VALUES('integrity-check', 1)",
        [],
    )
    .is_ok()
}

/// Streaming FNV-1a 64-bit hasher with structured byte framing.
struct Fnv1aDigestHasher {
    state: u64,
}

impl Fnv1aDigestHasher {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x100_0000_01b3;

    fn new() -> Self {
        Self {
            state: Self::FNV_OFFSET_BASIS,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::FNV_PRIME);
        }
    }

    fn write_tag(&mut self, tag: u8) {
        self.write_bytes(&[tag]);
    }

    fn write_u64(&mut self, val: u64) {
        self.write_bytes(&val.to_be_bytes());
    }

    fn write_i64(&mut self, val: i64) {
        self.write_bytes(&val.to_be_bytes());
    }

    fn write_str(&mut self, s: &str) {
        let len = u64::try_from(s.len()).unwrap_or(u64::MAX);
        self.write_u64(len);
        self.write_bytes(s.as_bytes());
    }

    fn write_opt_str(&mut self, s: Option<&str>) {
        match s {
            Some(val) => {
                self.write_tag(1);
                self.write_str(val);
            }
            None => {
                self.write_tag(0);
            }
        }
    }

    fn write_opt_i64(&mut self, val: Option<i64>) {
        match val {
            Some(v) => {
                self.write_tag(1);
                self.write_i64(v);
            }
            None => {
                self.write_tag(0);
            }
        }
    }

    fn finish_hex(&self) -> String {
        format!("{:016x}", self.state)
    }
}

/// Computes a deterministic checksum over the rebuildable business projections
/// (`thread`, `turn`, `permission`).
///
/// Uses explicit length prefixes and type tags (no delimiter collision possible),
/// fixed ascending ordering on primary keys, and avoids reading any FTS shadow tables.
///
/// # Errors
///
/// Returns [`StorageError::Sqlite`] on query failure.
#[allow(clippy::too_many_lines)]
pub fn domain_projection_digest(conn: &Connection) -> Result<String, StorageError> {
    let mut hasher = Fnv1aDigestHasher::new();

    // 1. Thread projection rows ordered by thread_id
    let mut thread_stmt = conn
        .prepare(
            "SELECT thread_id, agent_profile_id, title, state, project_id,
                    event_count, first_event_seq, last_event_seq, last_event_id,
                    last_event_kind, created_at, updated_at
             FROM thread ORDER BY thread_id ASC",
        )
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest thread", e))?;

    let mut thread_rows = thread_stmt
        .query([])
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest thread query", e))?;

    while let Some(row) = thread_rows
        .next()
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest thread row", e))?
    {
        hasher.write_tag(b'T');
        let thread_id: String = row
            .get(0)
            .map_err(|e| StorageError::from_sqlite("digest thread.thread_id", e))?;
        let agent_profile_id: String = row
            .get(1)
            .map_err(|e| StorageError::from_sqlite("digest thread.agent_profile_id", e))?;
        let title: String = row
            .get(2)
            .map_err(|e| StorageError::from_sqlite("digest thread.title", e))?;
        let state: String = row
            .get(3)
            .map_err(|e| StorageError::from_sqlite("digest thread.state", e))?;
        let project_id: Option<String> = row
            .get(4)
            .map_err(|e| StorageError::from_sqlite("digest thread.project_id", e))?;
        let event_count: i64 = row
            .get(5)
            .map_err(|e| StorageError::from_sqlite("digest thread.event_count", e))?;
        let first_event_seq: Option<i64> = row
            .get(6)
            .map_err(|e| StorageError::from_sqlite("digest thread.first_event_seq", e))?;
        let last_event_seq: Option<i64> = row
            .get(7)
            .map_err(|e| StorageError::from_sqlite("digest thread.last_event_seq", e))?;
        let last_event_id: Option<String> = row
            .get(8)
            .map_err(|e| StorageError::from_sqlite("digest thread.last_event_id", e))?;
        let last_event_kind: Option<String> = row
            .get(9)
            .map_err(|e| StorageError::from_sqlite("digest thread.last_event_kind", e))?;
        let created_at: i64 = row
            .get(10)
            .map_err(|e| StorageError::from_sqlite("digest thread.created_at", e))?;
        let updated_at: i64 = row
            .get(11)
            .map_err(|e| StorageError::from_sqlite("digest thread.updated_at", e))?;

        hasher.write_str(&thread_id);
        hasher.write_str(&agent_profile_id);
        hasher.write_str(&title);
        hasher.write_str(&state);
        hasher.write_opt_str(project_id.as_deref());
        hasher.write_i64(event_count);
        hasher.write_opt_i64(first_event_seq);
        hasher.write_opt_i64(last_event_seq);
        hasher.write_opt_str(last_event_id.as_deref());
        hasher.write_opt_str(last_event_kind.as_deref());
        hasher.write_i64(created_at);
        hasher.write_i64(updated_at);
    }
    drop(thread_rows);
    drop(thread_stmt);

    // 2. Turn projection rows ordered by turn_id
    let mut turn_stmt = conn
        .prepare(
            "SELECT turn_id, thread_id, operation_id, state, delivery,
                    event_count, started_at, ended_at
             FROM turn ORDER BY turn_id ASC",
        )
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest turn", e))?;

    let mut turn_rows = turn_stmt
        .query([])
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest turn query", e))?;

    while let Some(row) = turn_rows
        .next()
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest turn row", e))?
    {
        hasher.write_tag(b'U');
        let turn_id: String = row
            .get(0)
            .map_err(|e| StorageError::from_sqlite("digest turn.turn_id", e))?;
        let thread_id: String = row
            .get(1)
            .map_err(|e| StorageError::from_sqlite("digest turn.thread_id", e))?;
        let operation_id: Option<String> = row
            .get(2)
            .map_err(|e| StorageError::from_sqlite("digest turn.operation_id", e))?;
        let state: String = row
            .get(3)
            .map_err(|e| StorageError::from_sqlite("digest turn.state", e))?;
        let delivery: String = row
            .get(4)
            .map_err(|e| StorageError::from_sqlite("digest turn.delivery", e))?;
        let event_count: i64 = row
            .get(5)
            .map_err(|e| StorageError::from_sqlite("digest turn.event_count", e))?;
        let started_at: i64 = row
            .get(6)
            .map_err(|e| StorageError::from_sqlite("digest turn.started_at", e))?;
        let ended_at: Option<i64> = row
            .get(7)
            .map_err(|e| StorageError::from_sqlite("digest turn.ended_at", e))?;

        hasher.write_str(&turn_id);
        hasher.write_str(&thread_id);
        hasher.write_opt_str(operation_id.as_deref());
        hasher.write_str(&state);
        hasher.write_str(&delivery);
        hasher.write_i64(event_count);
        hasher.write_i64(started_at);
        hasher.write_opt_i64(ended_at);
    }
    drop(turn_rows);
    drop(turn_stmt);

    // 3. Permission projection rows ordered by event_id
    let mut perm_stmt = conn
        .prepare(
            "SELECT event_id, turn_id, thread_id, kind, description, decision,
                    requested_at, decided_at
             FROM permission ORDER BY event_id ASC",
        )
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest perm", e))?;

    let mut perm_rows = perm_stmt
        .query([])
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest perm query", e))?;

    while let Some(row) = perm_rows
        .next()
        .map_err(|e| StorageError::from_sqlite("domain_projection_digest perm row", e))?
    {
        hasher.write_tag(b'P');
        let event_id: String = row
            .get(0)
            .map_err(|e| StorageError::from_sqlite("digest permission.event_id", e))?;
        let turn_id: String = row
            .get(1)
            .map_err(|e| StorageError::from_sqlite("digest permission.turn_id", e))?;
        let thread_id: String = row
            .get(2)
            .map_err(|e| StorageError::from_sqlite("digest permission.thread_id", e))?;
        let kind: String = row
            .get(3)
            .map_err(|e| StorageError::from_sqlite("digest permission.kind", e))?;
        let description: String = row
            .get(4)
            .map_err(|e| StorageError::from_sqlite("digest permission.description", e))?;
        let decision: String = row
            .get(5)
            .map_err(|e| StorageError::from_sqlite("digest permission.decision", e))?;
        let requested_at: i64 = row
            .get(6)
            .map_err(|e| StorageError::from_sqlite("digest permission.requested_at", e))?;
        let decided_at: Option<i64> = row
            .get(7)
            .map_err(|e| StorageError::from_sqlite("digest permission.decided_at", e))?;

        hasher.write_str(&event_id);
        hasher.write_str(&turn_id);
        hasher.write_str(&thread_id);
        hasher.write_str(&kind);
        hasher.write_str(&description);
        hasher.write_str(&decision);
        hasher.write_i64(requested_at);
        hasher.write_opt_i64(decided_at);
    }
    drop(perm_rows);
    drop(perm_stmt);

    Ok(hasher.finish_hex())
}
