//! SQLite storage spike: append-only event journal with rebuildable
//! projections (ADR 0009).
//!
//! The durable-ownership rule from `docs/ARCHITECTURE.md` is the
//! contract here: the journal is authoritative for syncable knowledge
//! lifecycle; the projection tables are caches with a recorded
//! high-water marker, and any detected staleness heals by replay.
//! Everything is deterministic — SQLite runs in memory or in a
//! `tempfile` path, timestamps come from the envelopes, and no test
//! sleeps or touches the network.

pub mod error;
mod migrations;

use rusqlite::{Connection, params};

use altior_domain::{ThreadId, TurnId};
use altior_protocol::EventEnvelope;
pub use error::StorageError;

/// Defense-in-depth cap on a single journal payload (ADR 0009 §3),
/// mirroring the ACP line cap.
pub const JOURNAL_PAYLOAD_MAX: usize = 1024 * 1024;

/// Current fold semantics for derived projections. This is separate
/// from SQLite `user_version`, which versions physical schema only.
pub const PROJECTION_VERSION: i64 = 1;

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

/// A per-thread aggregate derived from the journal.
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

        let thread = envelope.thread_id.as_ref().map(ThreadId::as_str);
        let turn = envelope.turn_id.as_ref().map(TurnId::as_str);
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

    /// The number of journaled events.
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
}

/// Maps one journal row.
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
