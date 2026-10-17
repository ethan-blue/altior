//! Durable Personal Vault sync state (ADR 0027 §5–6, ADR 0025 gate).
//!
//! The load-bearing piece is [`SqliteCounterLease`]: the F31 fix. A
//! restarted process must never rewind its send counters into already
//! used values — the reservation is committed to SQLite *before* any
//! ciphertext with those counters exists, so a crash or kill between
//! reservation and use only wastes counters, never reuses them.
//!
//! The remaining tables (`vault_device`, `sync_ack_cursor`,
//! `memory_tombstone`) are written by the sync engine as it applies
//! events; their accessor helpers live with the engine integration.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::Store;
use crate::error::StorageError;
use altior_domain::{
    DeviceLabel, EpochSeq, RecoverySignature, RevocationCertificate, UnixMillis, VaultDeviceRecord,
    VaultDeviceStatus,
};
/// A durable [`altior_crypto::CounterLease`] over one direction of one
/// device pair, keyed by the caller-chosen lease key (convention:
/// `"{sender}→{receiver}"`).
///
/// Blocks are handed out from a monotonically growing ledger row inside
/// a single transaction — two leases on the same key (two threads, two
/// processes) serialize on SQLite's write lock and never overlap.
pub struct SqliteCounterLease<'a> {
    conn: &'a mut Connection,
    lease_key: String,
}

impl std::fmt::Debug for SqliteCounterLease<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteCounterLease")
            .field("lease_key", &self.lease_key)
            .finish_non_exhaustive()
    }
}

impl<'a> SqliteCounterLease<'a> {
    /// A lease over `lease_key` against the store's connection.
    #[must_use]
    pub fn new(conn: &'a mut Connection, lease_key: impl Into<String>) -> Self {
        Self {
            conn,
            lease_key: lease_key.into(),
        }
    }

    /// The next counter the lease would hand out (1 before the first
    /// reservation), for diagnostics and restart assertions.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] when the row cannot be read.
    pub fn next_counter(&mut self) -> Result<u64, StorageError> {
        self.conn
            .query_row(
                "SELECT next_counter FROM sync_counter_lease WHERE lease_key = ?1",
                [&self.lease_key],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| u64::try_from(value).unwrap_or(1))
            .or_else(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(1)
                } else {
                    Err(StorageError::from_sqlite("read counter lease", error))
                }
            })
    }
}

impl altior_crypto::CounterLease for SqliteCounterLease<'_> {
    fn reserve(&mut self, block_size: u64) -> std::io::Result<altior_crypto::CounterBlock> {
        let tx = self
            .conn
            .transaction()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let start: u64 = tx
            .query_row(
                "SELECT next_counter FROM sync_counter_lease WHERE lease_key = ?1",
                [&self.lease_key],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| u64::try_from(value).unwrap_or(1))
            .or_else(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(1)
                } else {
                    Err(std::io::Error::other(error.to_string()))
                }
            })?;
        let end = start
            .checked_add(block_size)
            .ok_or_else(|| std::io::Error::other("counter lease exhausted the u64 space"))?;
        tx.execute(
            "INSERT INTO sync_counter_lease (lease_key, next_counter) VALUES (?1, ?2)
             ON CONFLICT(lease_key) DO UPDATE SET next_counter = ?2",
            rusqlite::params![
                self.lease_key,
                i64::try_from(end).map_err(|error| { std::io::Error::other(error.to_string()) })?
            ],
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        tx.commit()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(altior_crypto::CounterBlock { start, end })
    }
}

/// A persisted tombstone row: the forget event's identity plus the
/// per-origin sequence the compaction gate compares against ack floors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedTombstone {
    /// The forgotten memory's id.
    pub memory_id: String,
    /// The origin (device label) of the forget event.
    pub origin: String,
    /// The forget event's per-origin sequence.
    pub origin_seq: u64,
    /// When the forget was issued (Unix millis).
    pub forgotten_at: i64,
    /// The tombstone digest bytes.
    pub digest: Vec<u8>,
}

/// A stored journal event row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSyncEvent {
    /// The event id.
    pub event_id: String,
    /// The origin (device label).
    pub origin: String,
    /// The per-origin sequence.
    pub origin_seq: u64,
    /// The Ed25519 signature bytes.
    pub signature: Vec<u8>,
    /// The serialized event body.
    pub body_json: String,
}

impl Store {
    /// Inserts or updates a paired-device record from the domain model.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidDomainEvent`] when the record violates the
    /// typed contract; [`StorageError::Sqlite`] on write failure.
    pub fn upsert_vault_device(&self, record: &VaultDeviceRecord) -> Result<(), StorageError> {
        let (status, revoked_at, cert_signature) = match record.status() {
            VaultDeviceStatus::Active => ("active", None, None),
            VaultDeviceStatus::Revoked {
                revoked_at,
                certificate,
            } => (
                "revoked",
                Some(i64::try_from(revoked_at.as_millis()).map_err(|error| {
                    StorageError::InvalidDomainEvent {
                        detail: format!("revoked_at: {error}"),
                    }
                })?),
                Some(certificate.signature().as_bytes().to_vec()),
            ),
        };
        self.conn()
            .execute(
                "INSERT INTO vault_device (device_id, added_at, added_by, epoch_seq, status, revoked_at, cert_signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(device_id) DO UPDATE SET
                    added_at = ?2, added_by = ?3, epoch_seq = ?4, status = ?5,
                    revoked_at = ?6, cert_signature = ?7",
                rusqlite::params![
                    record.device().as_str(),
                    i64::try_from(record.added_at().as_millis()).map_err(|error| {
                        StorageError::InvalidDomainEvent { detail: format!("added_at: {error}") }
                    })?,
                    record.added_by().as_str(),
                    record.epoch_seq().get(),
                    status,
                    revoked_at,
                    cert_signature,
                ],
            )
            .map_err(|error| StorageError::from_sqlite("upsert vault device", error))?;
        Ok(())
    }

    /// Every paired-device record, ordered by device id.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn list_vault_devices(&self) -> Result<Vec<VaultDeviceRecord>, StorageError> {
        let mut statement = self
            .conn()
            .prepare(
                "SELECT device_id, added_at, added_by, epoch_seq, status, revoked_at, cert_signature
                 FROM vault_device ORDER BY device_id",
            )
            .map_err(|error| StorageError::from_sqlite("list vault devices", error))?;
        let rows = statement
            .query_map([], |row| {
                let device_id: String = row.get(0)?;
                let added_at: i64 = row.get(1)?;
                let added_by: String = row.get(2)?;
                let epoch_seq: u32 = row.get(3)?;
                let status: String = row.get(4)?;
                let revoked_at: Option<i64> = row.get(5)?;
                let cert_signature: Option<Vec<u8>> = row.get(6)?;
                Ok((
                    device_id,
                    added_at,
                    added_by,
                    epoch_seq,
                    status,
                    revoked_at,
                    cert_signature,
                ))
            })
            .map_err(|error| StorageError::from_sqlite("query vault devices", error))?;
        let mut records = Vec::new();
        for row in rows {
            let (device_id, added_at, added_by, epoch_seq, status, revoked_at, cert_signature) =
                row.map_err(|error| StorageError::from_sqlite("vault device row", error))?;
            let device =
                DeviceLabel::new(device_id).map_err(|error| StorageError::InvalidDomainEvent {
                    detail: error.to_string(),
                })?;
            let added_by_label =
                DeviceLabel::new(added_by).map_err(|error| StorageError::InvalidDomainEvent {
                    detail: error.to_string(),
                })?;
            let status = match (status.as_str(), revoked_at, cert_signature) {
                ("active", _, _) => VaultDeviceStatus::Active,
                ("revoked", Some(revoked_at), Some(signature)) => {
                    let revoked_at =
                        UnixMillis::from_millis(u64::try_from(revoked_at).map_err(|error| {
                            StorageError::InvalidDomainEvent {
                                detail: format!("revoked_at: {error}"),
                            }
                        })?);
                    let certificate = RevocationCertificate::new(
                        device.clone(),
                        revoked_at,
                        RecoverySignature::new(signature).map_err(|error| {
                            StorageError::InvalidDomainEvent {
                                detail: error.to_string(),
                            }
                        })?,
                    )
                    .map_err(|error| StorageError::InvalidDomainEvent {
                        detail: error.to_string(),
                    })?;
                    VaultDeviceStatus::Revoked {
                        revoked_at,
                        certificate,
                    }
                }
                _ => {
                    return Err(StorageError::InvalidDomainEvent {
                        detail: format!("corrupt vault_device status row: {status}"),
                    });
                }
            };
            records.push(
                VaultDeviceRecord::restore(
                    device,
                    added_by_label,
                    UnixMillis::from_millis(u64::try_from(added_at).map_err(|error| {
                        StorageError::InvalidDomainEvent {
                            detail: format!("added_at: {error}"),
                        }
                    })?),
                    EpochSeq::from_raw(epoch_seq),
                    status,
                )
                .map_err(|error| StorageError::InvalidDomainEvent {
                    detail: error.to_string(),
                })?,
            );
        }
        Ok(records)
    }

    /// Records a forget permanently. Re-inserting the same memory id is
    /// an idempotent overwrite (a re-forget must win).
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn insert_memory_tombstone(
        &self,
        memory_id: &str,
        origin: &str,
        origin_seq: u64,
        forgotten_at: i64,
        digest: &[u8],
    ) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT INTO memory_tombstone (memory_id, origin, origin_seq, forgotten_at, digest)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(memory_id) DO UPDATE SET
                    origin = ?2, origin_seq = ?3, forgotten_at = ?4, digest = ?5",
                rusqlite::params![memory_id, origin, origin_seq, forgotten_at, digest],
            )
            .map_err(|error| StorageError::from_sqlite("insert tombstone", error))?;
        Ok(())
    }

    /// Appends one applied sync event to the durable journal.
    /// Returns `false` when the event id already exists (the durable
    /// dedup basis across restarts). `applied_order` is assigned
    /// internally as MAX+1, so load order equals insertion order.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn append_sync_journal_event(
        &self,
        event_id: &str,
        origin: &str,
        origin_seq: u64,
        signature: &[u8],
        body_json: &str,
    ) -> Result<bool, StorageError> {
        let origin_seq =
            i64::try_from(origin_seq).map_err(|error| StorageError::InvalidDomainEvent {
                detail: format!("origin_seq: {error}"),
            })?;
        let inserted = self
            .conn()
            .execute(
                "INSERT OR IGNORE INTO sync_journal
                    (event_id, origin, origin_seq, applied_order, signature, body_json)
                 SELECT ?1, ?2, ?3, COALESCE(MAX(applied_order), 0) + 1, ?4, ?5 FROM sync_journal",
                rusqlite::params![event_id, origin, origin_seq, signature, body_json],
            )
            .map_err(|error| StorageError::from_sqlite("append sync journal", error))?;
        Ok(inserted == 1)
    }

    /// Loads the applied journal in application order.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn list_sync_journal_events(&self) -> Result<Vec<StoredSyncEvent>, StorageError> {
        let mut statement = self
            .conn()
            .prepare(
                "SELECT event_id, origin, origin_seq, signature, body_json
                 FROM sync_journal ORDER BY applied_order",
            )
            .map_err(|error| StorageError::from_sqlite("list sync journal", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(StoredSyncEvent {
                    event_id: row.get(0)?,
                    origin: row.get(1)?,
                    origin_seq: row.get::<_, i64>(2).unwrap_or(0).cast_unsigned(),
                    signature: row.get(3)?,
                    body_json: row.get(4)?,
                })
            })
            .map_err(|error| StorageError::from_sqlite("query sync journal", error))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| StorageError::from_sqlite("sync journal row", error))?);
        }
        Ok(out)
    }

    /// Stores a blob chunk by its ciphertext identity (hex-encoded
    /// SHA-256). Idempotent: an existing id is left untouched.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn put_blob_chunk(
        &self,
        blob_id_hex: &str,
        content_hash_hex: &str,
        data: &[u8],
    ) -> Result<bool, StorageError> {
        let inserted = self
            .conn()
            .execute(
                "INSERT OR IGNORE INTO blob_chunks (blob_id, content_hash, size, data)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![blob_id_hex, content_hash_hex, data.len(), data],
            )
            .map_err(|error| StorageError::from_sqlite("put blob chunk", error))?;
        Ok(inserted == 1)
    }

    /// Fetches a blob chunk's ciphertext by id, if present.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn get_blob_chunk(&self, blob_id_hex: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.conn()
            .query_row(
                "SELECT data FROM blob_chunks WHERE blob_id = ?1",
                [blob_id_hex],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(StorageError::from_sqlite("get blob chunk", error))
                }
            })
    }

    /// Whether a chunk is present locally.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn has_blob_chunk(&self, blob_id_hex: &str) -> Result<bool, StorageError> {
        Ok(self.get_blob_chunk(blob_id_hex)?.is_some())
    }

    /// Every persisted tombstone.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn list_memory_tombstones(&self) -> Result<Vec<PersistedTombstone>, StorageError> {
        let mut statement = self
            .conn()
            .prepare(
                "SELECT memory_id, origin, origin_seq, forgotten_at, digest
                 FROM memory_tombstone ORDER BY origin, origin_seq",
            )
            .map_err(|error| StorageError::from_sqlite("list tombstones", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(PersistedTombstone {
                    memory_id: row.get(0)?,
                    origin: row.get(1)?,
                    origin_seq: row.get::<_, i64>(2).unwrap_or(0).cast_unsigned(),
                    forgotten_at: row.get(3)?,
                    digest: row.get(4)?,
                })
            })
            .map_err(|error| StorageError::from_sqlite("query tombstones", error))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| StorageError::from_sqlite("tombstone row", error))?);
        }
        Ok(out)
    }

    /// The threat-10 compaction gate: deletes tombstones whose forget
    /// event every device has acknowledged — `origin_seq` strictly below
    /// that origin's ack floor — and keeps everything at or above it,
    /// and everything whose origin has no floor yet. Returns the number
    /// reclaimed.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn compact_tombstones(
        &self,
        ack_floors: &BTreeMap<String, u64>,
    ) -> Result<u64, StorageError> {
        let mut reclaimed = 0u64;
        for (origin, floor) in ack_floors {
            let floor =
                i64::try_from(*floor).map_err(|error| StorageError::InvalidDomainEvent {
                    detail: format!("ack floor: {error}"),
                })?;
            let deleted = self
                .conn()
                .execute(
                    "DELETE FROM memory_tombstone WHERE origin = ?1 AND origin_seq < ?2",
                    rusqlite::params![origin, floor],
                )
                .map_err(|error| StorageError::from_sqlite("compact tombstones", error))?;
            reclaimed += u64::try_from(deleted).unwrap_or(0);
        }
        Ok(reclaimed)
    }

    /// Records this device's durable ack: everything of `peer`'s
    /// journal through `cursor` is applied. Forward-only; a lower value
    /// is ignored (a rollback claim is not an acknowledgement).
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn observe_ack_cursor(
        &self,
        device: &str,
        peer: &str,
        cursor: u64,
    ) -> Result<(), StorageError> {
        let cursor = i64::try_from(cursor).map_err(|error| StorageError::InvalidDomainEvent {
            detail: format!("ack cursor: {error}"),
        })?;
        self.conn()
            .execute(
                "INSERT INTO sync_ack_cursor (device_id, peer_id, cursor) VALUES (?1, ?2, ?3)
                 ON CONFLICT(device_id, peer_id) DO UPDATE SET
                    cursor = MAX(cursor, ?3)",
                rusqlite::params![device, peer, cursor],
            )
            .map_err(|error| StorageError::from_sqlite("observe ack cursor", error))?;
        Ok(())
    }

    /// Persists the receiver's relay fetch position for one peer bucket.
    /// Forward-only: a lower value is ignored.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on write failure.
    pub fn record_sync_fetch_cursor(&self, peer: &str, cursor: u64) -> Result<(), StorageError> {
        let cursor = i64::try_from(cursor).map_err(|error| StorageError::InvalidDomainEvent {
            detail: format!("fetch cursor: {error}"),
        })?;
        self.conn()
            .execute(
                "INSERT INTO sync_fetch_cursor (peer_id, cursor) VALUES (?1, ?2)
                 ON CONFLICT(peer_id) DO UPDATE SET cursor = MAX(cursor, ?2)",
                rusqlite::params![peer, cursor],
            )
            .map_err(|error| StorageError::from_sqlite("record fetch cursor", error))?;
        Ok(())
    }

    /// Loads the receiver's relay fetch positions per peer bucket.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn list_sync_fetch_cursors(&self) -> Result<Vec<(String, u64)>, StorageError> {
        let mut statement = self
            .conn()
            .prepare("SELECT peer_id, cursor FROM sync_fetch_cursor ORDER BY peer_id")
            .map_err(|error| StorageError::from_sqlite("list fetch cursors", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.cast_unsigned(),
                ))
            })
            .map_err(|error| StorageError::from_sqlite("query fetch cursors", error))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| StorageError::from_sqlite("fetch cursor row", error))?);
        }
        Ok(out)
    }

    /// The acknowledged floor across all of this device's peers — the
    /// conservative minimum; `None` pins compaction entirely.
    ///
    /// # Errors
    ///
    /// [`StorageError::Sqlite`] on read failure.
    pub fn ack_floor(&self, device: &str) -> Result<Option<u64>, StorageError> {
        self.conn()
            .query_row(
                "SELECT MIN(cursor) FROM sync_ack_cursor WHERE device_id = ?1",
                [device],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map(|floor| floor.and_then(|value| u64::try_from(value).ok()))
            .map_err(|error| StorageError::from_sqlite("read ack floor", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use altior_crypto::{CounterLease, DeviceId, SessionEpoch, SessionV2};
    use altior_domain::DeviceLabel;

    fn device_keys(seed: u8, label: &str) -> altior_crypto::DeviceKeys {
        let mut key_seed = [seed; 64];
        key_seed[0] = seed;
        altior_crypto::DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap()
    }

    #[test]
    fn leases_survive_reopen_without_reuse() {
        let path = tempfile::tempdir().unwrap().keep().join("sync-lease.db");
        {
            let mut store = Store::open(&path).unwrap();
            let conn = store.conn_mut();
            let mut lease = SqliteCounterLease::new(conn, "alice→bob");
            let first = CounterLease::reserve(&mut lease, 100).unwrap();
            assert_eq!(first.start, 1);
            assert_eq!(first.end, 101);
        }
        // Simulated process kill: reopen, the ledger continues.
        {
            let mut store = Store::open(&path).unwrap();
            let conn = store.conn_mut();
            let mut lease = SqliteCounterLease::new(conn, "alice→bob");
            assert_eq!(lease.next_counter().unwrap(), 101);
            let second = CounterLease::reserve(&mut lease, 100).unwrap();
            assert_eq!(second.start, 101);
            assert_eq!(second.end, 201);
        }
        // The reverse direction is an independent ledger.
        let mut store = Store::open(&path).unwrap();
        let conn = store.conn_mut();
        let mut reverse = SqliteCounterLease::new(conn, "bob→alice");
        assert_eq!(reverse.next_counter().unwrap(), 1);
    }

    #[test]
    fn leased_counters_never_repeat_across_session_reconstruction() {
        // The F31 end-to-end shape: static keys, same epoch, fresh
        // sessions drawing from one durable lease — counters never repeat.
        let mut store = Store::open_in_memory().unwrap();
        let alice = device_keys(1, "alice-f31");
        let bob = device_keys(2, "bob-f31");
        let epoch = SessionEpoch::from_bytes([9; 16]).unwrap();

        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let conn = store.conn_mut();
            let mut lease = SqliteCounterLease::new(conn, "alice→bob");
            let mut session =
                SessionV2::outbound(&alice, &bob.public_identity(), epoch.clone(), 1, &mut lease)
                    .unwrap();
            for n in 0..10 {
                let envelope = session
                    .seal(format!("kill-and-restart {n}").as_bytes())
                    .unwrap();
                let counter = u64::from_be_bytes(envelope[29..37].try_into().unwrap());
                assert!(
                    seen.insert(counter),
                    "counter {counter} reused after restart"
                );
            }
        }
        assert_eq!(seen.len(), 500);
    }

    #[test]
    fn vault_device_records_roundtrip_with_revocation() {
        let store = Store::open_in_memory().unwrap();
        let device = DeviceLabel::new("phone-lifecycle").unwrap();
        let mut record = VaultDeviceRecord::pair(
            device.clone(),
            DeviceLabel::new("laptop-lifecycle").unwrap(),
            UnixMillis::from_millis(1_700_000_000_000),
            EpochSeq::INITIAL,
        )
        .unwrap();
        store.upsert_vault_device(&record).unwrap();

        let loaded = store.list_vault_devices().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].is_active());
        assert_eq!(loaded[0].device().as_str(), "phone-lifecycle");
        assert_eq!(loaded[0].epoch_seq(), EpochSeq::INITIAL);

        record
            .revoke(
                RevocationCertificate::new(
                    device,
                    UnixMillis::from_millis(1_700_000_050_000),
                    RecoverySignature::new(vec![0xAB; 64]).unwrap(),
                )
                .unwrap(),
                UnixMillis::from_millis(1_700_000_060_000),
            )
            .unwrap();
        store.upsert_vault_device(&record).unwrap();

        let reloaded = store.list_vault_devices().unwrap();
        assert_eq!(reloaded.len(), 1);
        assert!(!reloaded[0].is_active());
        match reloaded[0].status() {
            VaultDeviceStatus::Revoked { certificate, .. } => {
                assert_eq!(certificate.signature().as_bytes(), &[0xAB; 64]);
            }
            other @ VaultDeviceStatus::Active => {
                panic!("expected revoked after reload, got {other:?}")
            }
        }
    }

    #[test]
    fn tombstones_persist_and_compact_only_below_the_ack_floor() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_memory_tombstone(
                "mem_compact0000001",
                "device-a",
                4,
                1_700_000_000_000,
                &[1; 32],
            )
            .unwrap();
        store
            .insert_memory_tombstone(
                "mem_compact0000002",
                "device-a",
                9,
                1_700_000_000_001,
                &[2; 32],
            )
            .unwrap();
        store
            .insert_memory_tombstone(
                "mem_compact0000003",
                "device-b",
                2,
                1_700_000_000_002,
                &[3; 32],
            )
            .unwrap();

        // Re-forgetting the same memory is an idempotent overwrite that
        // wins with the newer tuple.
        store
            .insert_memory_tombstone(
                "mem_compact0000001",
                "device-a",
                4,
                1_700_000_000_003,
                &[9; 32],
            )
            .unwrap();
        let all = store.list_memory_tombstones().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].digest, vec![9; 32]);

        // No floors recorded: compaction must not touch anything.
        let empty = std::collections::BTreeMap::new();
        assert_eq!(store.compact_tombstones(&empty).unwrap(), 0);
        assert_eq!(store.list_memory_tombstones().unwrap().len(), 3);

        // A floor below a forget's sequence keeps the tombstone.
        let floors = std::collections::BTreeMap::from([
            ("device-a".to_owned(), 4),
            ("device-b".to_owned(), 1),
        ]);
        assert_eq!(store.compact_tombstones(&floors).unwrap(), 0);

        // Covering floors: device-a's seq-4 tombstone is acknowledged by
        // everyone (floor >= 4 keeps nothing strictly below 4 from a...
        // seq 4 is kept: strictly below only), seq 9 stays.
        let floors = std::collections::BTreeMap::from([
            ("device-a".to_owned(), 5),
            ("device-b".to_owned(), 3),
        ]);
        assert_eq!(store.compact_tombstones(&floors).unwrap(), 2);
        let remaining = store.list_memory_tombstones().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].memory_id, "mem_compact0000002");
    }

    #[test]
    fn ack_cursors_are_forward_only_and_floor_is_the_minimum() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.ack_floor("device-a").unwrap(), None);

        store
            .observe_ack_cursor("device-a", "device-b", 10)
            .unwrap();
        store.observe_ack_cursor("device-a", "device-c", 4).unwrap();
        assert_eq!(store.ack_floor("device-a").unwrap(), Some(4));

        // A rollback claim from device-c does not lower the floor.
        store.observe_ack_cursor("device-a", "device-c", 2).unwrap();
        assert_eq!(store.ack_floor("device-a").unwrap(), Some(4));

        // Progress raises the floor only when the slowest catches up.
        store.observe_ack_cursor("device-c", "device-a", 1).unwrap();
        assert_eq!(store.ack_floor("device-c").unwrap(), Some(1));
        store
            .observe_ack_cursor("device-a", "device-c", 12)
            .unwrap();
        assert_eq!(store.ack_floor("device-a").unwrap(), Some(10));
    }

    #[test]
    fn vault_device_rows_roundtrip_through_the_migration() {
        let mut store = Store::open_in_memory().unwrap();
        let conn = store.conn_mut();
        conn.execute(
            "INSERT INTO vault_device (device_id, added_at, added_by, epoch_seq, status)
             VALUES (?1, ?2, ?3, ?4, 'active')",
            rusqlite::params!["phone", 1_700_000_000_000_i64, "laptop", 1],
        )
        .unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM vault_device WHERE device_id = 'phone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "active");
        let label = DeviceLabel::new("phone").unwrap();
        assert_eq!(label.as_str(), "phone");
    }
}
