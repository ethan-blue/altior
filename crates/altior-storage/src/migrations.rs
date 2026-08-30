//! Forward-only schema migrations keyed by `PRAGMA user_version`
//! (ADR 0009 §2).

use rusqlite::Connection;

use crate::error::StorageError;

/// One migration step: the schema version it migrates the database to.
#[derive(Debug)]
pub(crate) struct Migration {
    /// The `user_version` stamped after this step succeeds.
    pub(crate) version: i64,
    /// The SQL applied inside one transaction.
    pub(crate) sql: &'static str,
}

/// The ordered, append-only list of schema steps.
///
/// History is part of the contract: entries are never edited or
/// reordered once released, only appended.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: SCHEMA_V1,
}];

/// The highest schema version this build understands.
#[must_use]
pub fn latest_schema_version() -> i64 {
    MIGRATIONS.last().map_or(0, |step| step.version)
}

/// Reads the schema version currently stamped in the file.
///
/// # Errors
///
/// Returns [`StorageError::Sqlite`] when the pragma cannot be read.
pub fn schema_version(conn: &Connection) -> Result<i64, StorageError> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| StorageError::from_sqlite("read user_version", error))
}

/// Applies pending migrations in order, refusing schemas from the
/// future instead of guessing a downgrade (ADR 0009 §2).
///
/// # Errors
///
/// Returns [`StorageError::SchemaTooNew`] when the file's version
/// exceeds [`latest_schema_version`], or
/// [`StorageError::MigrationFailed`] when a step fails (the file stays
/// at its previous version).
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StorageError> {
    let current = schema_version(conn)?;
    let latest = latest_schema_version();
    if current > latest {
        return Err(StorageError::SchemaTooNew {
            found: current,
            supported: latest,
        });
    }
    for step in MIGRATIONS {
        if step.version <= current {
            continue;
        }
        let tx = conn
            .transaction()
            .map_err(|error| StorageError::from_sqlite("begin migration", error))?;
        tx.execute_batch(step.sql)
            .map_err(|error| StorageError::MigrationFailed {
                to_version: step.version,
                source: error,
            })?;
        tx.pragma_update(None, "user_version", step.version)
            .map_err(|error| StorageError::MigrationFailed {
                to_version: step.version,
                source: error,
            })?;
        tx.commit().map_err(|error| StorageError::MigrationFailed {
            to_version: step.version,
            source: error,
        })?;
    }
    Ok(())
}

/// The version-1 schema: the append-only journal, the derived thread
/// projection, and the rebuild high-water marker (ADR 0009 §3–4).
const SCHEMA_V1: &str = r"
CREATE TABLE journal (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    thread_id TEXT,
    turn_id TEXT,
    stream_sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    occurred_at INTEGER NOT NULL
);

CREATE INDEX journal_thread_seq ON journal(thread_id, seq);

-- The journal is append-only by contract; the triggers make the
-- database itself refuse accidental history rewrites (ADR 0009 §3).
CREATE TRIGGER journal_no_update
BEFORE UPDATE ON journal
BEGIN
    SELECT RAISE(ABORT, 'journal is append-only');
END;

CREATE TRIGGER journal_no_delete
BEFORE DELETE ON journal
BEGIN
    SELECT RAISE(ABORT, 'journal is append-only');
END;

-- Derived state: always rebuildable from the journal.
CREATE TABLE thread_projection (
    thread_id TEXT PRIMARY KEY,
    event_count INTEGER NOT NULL,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_event_id TEXT NOT NULL,
    last_kind TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The recovery marker: which journal position the projections have
-- folded (ADR 0009 §4).
CREATE TABLE projection_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    journal_max_seq INTEGER NOT NULL,
    projection_version INTEGER NOT NULL,
    rebuilt_at INTEGER NOT NULL
);
";
