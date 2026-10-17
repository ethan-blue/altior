//! Durable sync-state ports and implementations (ADR 0027 §6).
//!
//! The engine keeps its working state in memory; this module owns what
//! survives a restart. [`SyncStatePort`] is the seam, [`MemorySyncStatePort`]
//! the in-process default, and [`StoreSyncStatePort`] the SQLite
//! implementation over the storage v9 tables — the applied journal is
//! the load-bearing one: a restarted node replays it through the normal
//! apply path and rebuilds every projection, so relay retention or
//! compaction can never strand it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{EventBody, JournalEvent, SyncError, TombstoneMark};

/// The durable state seam behind a [`crate::sync::SyncNode`].
///
/// Implementations must be crash-safe per call: a `record_*` that
/// returned `Ok` is committed. Errors are [`SyncError::State`].
pub trait SyncStatePort {
    /// Records an applied event durably. Returns `false` when the event
    /// id already exists (the durable dedup basis across restarts).
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on persistence failure.
    fn record_applied(&mut self, event: &JournalEvent) -> Result<bool, SyncError>;

    /// Loads the applied journal in application order.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on load failure.
    fn load_applied(&mut self) -> Result<Vec<JournalEvent>, SyncError>;

    /// Persists the receiver's relay fetch position for one peer bucket.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on persistence failure.
    fn record_fetch_cursor(&mut self, peer: &str, cursor: u64) -> Result<(), SyncError>;

    /// Loads fetch cursors per peer.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on load failure.
    fn load_fetch_cursors(&mut self) -> Result<BTreeMap<String, u64>, SyncError>;

    /// Persists a tombstone mark durably.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on persistence failure.
    fn record_tombstone(
        &mut self,
        memory_id: &str,
        mark: &TombstoneMark,
        forgotten_at_millis: u64,
    ) -> Result<(), SyncError>;

    /// Loads tombstone marks.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] on load failure.
    fn load_tombstones(&mut self) -> Result<BTreeMap<String, TombstoneMark>, SyncError>;
}

/// The in-process default: a node built with it behaves exactly like the
/// pre-port engine (no durability across drops).
#[derive(Debug, Default)]
pub struct MemorySyncStatePort {
    applied: Vec<JournalEvent>,
    applied_ids: BTreeMap<String, usize>,
    cursors: BTreeMap<String, u64>,
    tombstones: BTreeMap<String, (TombstoneMark, u64)>,
}

impl MemorySyncStatePort {
    /// An empty port.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SyncStatePort for MemorySyncStatePort {
    fn record_applied(&mut self, event: &JournalEvent) -> Result<bool, SyncError> {
        if self.applied_ids.contains_key(&event.event_id) {
            return Ok(false);
        }
        self.applied.push(event.clone());
        self.applied_ids
            .insert(event.event_id.clone(), self.applied.len());
        Ok(true)
    }

    fn load_applied(&mut self) -> Result<Vec<JournalEvent>, SyncError> {
        Ok(self.applied.clone())
    }

    fn record_fetch_cursor(&mut self, peer: &str, cursor: u64) -> Result<(), SyncError> {
        self.cursors.insert(peer.to_owned(), cursor);
        Ok(())
    }

    fn load_fetch_cursors(&mut self) -> Result<BTreeMap<String, u64>, SyncError> {
        Ok(self.cursors.clone())
    }

    fn record_tombstone(
        &mut self,
        memory_id: &str,
        mark: &TombstoneMark,
        forgotten_at_millis: u64,
    ) -> Result<(), SyncError> {
        self.tombstones
            .insert(memory_id.to_owned(), (mark.clone(), forgotten_at_millis));
        Ok(())
    }

    fn load_tombstones(&mut self) -> Result<BTreeMap<String, TombstoneMark>, SyncError> {
        Ok(self
            .tombstones
            .iter()
            .map(|(id, (mark, _))| (id.clone(), mark.clone()))
            .collect())
    }
}

/// The wire form of one applied journal event (storage serialization).
#[derive(Serialize, Deserialize)]
struct StoredEvent {
    event_id: String,
    origin: String,
    origin_seq: u64,
    signature: Vec<u8>,
    body: EventBody,
}

/// The SQLite implementation over the storage v9 sync tables
/// (`sync_journal`, `sync_fetch_cursor`, `memory_tombstone`).
pub struct StoreSyncStatePort {
    store: altior_storage::Store,
}

impl std::fmt::Debug for StoreSyncStatePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreSyncStatePort").finish_non_exhaustive()
    }
}

impl StoreSyncStatePort {
    /// A port owning one node's durable store.
    #[must_use]
    pub fn new(store: altior_storage::Store) -> Self {
        Self { store }
    }
}

impl SyncStatePort for StoreSyncStatePort {
    fn record_applied(&mut self, event: &JournalEvent) -> Result<bool, SyncError> {
        let stored = StoredEvent {
            event_id: event.event_id.clone(),
            origin: event.origin.clone(),
            origin_seq: event.origin_seq,
            signature: event.signature.to_vec(),
            body: event.body.clone(),
        };
        let body_json = serde_json::to_string(&stored).map_err(SyncError::Serialization)?;
        self.store
            .append_sync_journal_event(
                &event.event_id,
                &event.origin,
                event.origin_seq,
                &event.signature,
                &body_json,
            )
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))
    }

    fn load_applied(&mut self) -> Result<Vec<JournalEvent>, SyncError> {
        let rows = self
            .store
            .list_sync_journal_events()
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let stored: StoredEvent =
                serde_json::from_str(&row.body_json).map_err(SyncError::Serialization)?;
            events.push(JournalEvent {
                event_id: row.event_id,
                origin: row.origin,
                origin_seq: row.origin_seq,
                signature: row
                    .signature
                    .try_into()
                    .map_err(|_| SyncError::Invariant("stored signature width".to_owned()))?,
                body: stored.body,
            });
        }
        Ok(events)
    }

    fn record_fetch_cursor(&mut self, peer: &str, cursor: u64) -> Result<(), SyncError> {
        self.store
            .record_sync_fetch_cursor(peer, cursor)
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))
    }

    fn load_fetch_cursors(&mut self) -> Result<BTreeMap<String, u64>, SyncError> {
        let rows = self
            .store
            .list_sync_fetch_cursors()
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))?;
        Ok(rows.into_iter().collect())
    }

    fn record_tombstone(
        &mut self,
        memory_id: &str,
        mark: &TombstoneMark,
        forgotten_at_millis: u64,
    ) -> Result<(), SyncError> {
        let forgotten_at = i64::try_from(forgotten_at_millis)
            .map_err(|error| SyncError::Invariant(format!("forgotten_at: {error}")))?;
        self.store
            .insert_memory_tombstone(
                memory_id,
                &mark.origin,
                mark.origin_seq,
                forgotten_at,
                &mark.digest,
            )
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))
    }

    fn load_tombstones(&mut self) -> Result<BTreeMap<String, TombstoneMark>, SyncError> {
        let rows = self
            .store
            .list_memory_tombstones()
            .map_err(|error| SyncError::State(std::io::Error::other(error.to_string())))?;
        let mut out = BTreeMap::new();
        for row in rows {
            out.insert(
                row.memory_id,
                TombstoneMark {
                    digest: row
                        .digest
                        .try_into()
                        .map_err(|_| SyncError::Invariant("stored digest width".to_owned()))?,
                    origin: row.origin,
                    origin_seq: row.origin_seq,
                },
            );
        }
        Ok(out)
    }
}
