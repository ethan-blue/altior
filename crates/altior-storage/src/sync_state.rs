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

use rusqlite::Connection;

use crate::error::StorageError;

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
                if matches!(
                    error,
                    rusqlite::Error::QueryReturnedNoRows
                ) {
                    Ok(1)
                } else {
                    Err(StorageError::from_sqlite("read counter lease", error))
                }
            })
    }
}

impl altior_crypto::CounterLease for SqliteCounterLease<'_> {
    fn reserve(
        &mut self,
        block_size: u64,
    ) -> std::io::Result<altior_crypto::CounterBlock> {
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
            rusqlite::params![self.lease_key, i64::try_from(end).map_err(|error| {
                std::io::Error::other(error.to_string())
            })?],
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        tx.commit()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(altior_crypto::CounterBlock { start, end })
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
        altior_crypto::DeviceKeys::from_seed(
            key_seed,
            DeviceId::new(label).unwrap(),
        )
        .unwrap()
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
            let mut session = SessionV2::outbound(
                &alice,
                &bob.public_identity(),
                epoch.clone(),
                1,
                &mut lease,
            )
            .unwrap();
            for n in 0..10 {
                let envelope = session
                    .seal(format!("kill-and-restart {n}").as_bytes())
                    .unwrap();
                let counter =
                    u64::from_be_bytes(envelope[29..37].try_into().unwrap());
                assert!(seen.insert(counter), "counter {counter} reused after restart");
            }
        }
        assert_eq!(seen.len(), 500);
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
