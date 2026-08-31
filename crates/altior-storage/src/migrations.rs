//! Forward-only schema migrations keyed by `PRAGMA user_version`
//! (ADR 0009 §2, ADR 0013 §2).

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
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: SCHEMA_V1,
    },
    Migration {
        version: 2,
        sql: SCHEMA_V2,
    },
    Migration {
        version: 3,
        sql: SCHEMA_V3,
    },
    Migration {
        version: 4,
        sql: SCHEMA_V4,
    },
];

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

/// The version-2 schema: P1.1 domain tables for threads, turns,
/// permissions, agent profiles, harness bindings, and project refs
/// (ADR 0013). These are projection tables derived from the domain
/// journal — always rebuildable.
const SCHEMA_V2: &str = r"
-- Domain event journal: independent from the IPC EventEnvelope journal.
-- The domain journal is the durable authority; the IPC journal (v1)
-- remains for protocol-level replay. Domain records decouple from
-- protocol DTOs.
CREATE TABLE domain_journal (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    thread_id TEXT,
    turn_id TEXT,
    operation_id TEXT,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    occurred_at INTEGER NOT NULL
);

CREATE INDEX domain_journal_thread_seq ON domain_journal(thread_id, seq);
CREATE INDEX domain_journal_turn_seq ON domain_journal(turn_id, seq);

-- Domain journal is append-only.
CREATE TRIGGER domain_journal_no_update
BEFORE UPDATE ON domain_journal
BEGIN
    SELECT RAISE(ABORT, 'domain journal is append-only');
END;

CREATE TRIGGER domain_journal_no_delete
BEFORE DELETE ON domain_journal
BEGIN
    SELECT RAISE(ABORT, 'domain journal is append-only');
END;

-- Thread projection: enriched from domain journal events.
CREATE TABLE thread (
    thread_id TEXT PRIMARY KEY,
    agent_profile_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'open',
    project_id TEXT,
    event_count INTEGER NOT NULL DEFAULT 0,
    first_event_seq INTEGER,
    last_event_seq INTEGER,
    last_event_id TEXT,
    last_event_kind TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX thread_updated ON thread(updated_at);
CREATE INDEX thread_state ON thread(state);

-- Turn projection: tracks turn lifecycle.
CREATE TABLE turn (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    operation_id TEXT,
    state TEXT NOT NULL DEFAULT 'active',
    delivery TEXT NOT NULL DEFAULT 'absent',
    event_count INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    ended_at INTEGER
);

CREATE INDEX turn_thread_started ON turn(thread_id, started_at, turn_id);

-- Permission projection: tracks permission requests and decisions.
CREATE TABLE permission (
    event_id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    decision TEXT NOT NULL DEFAULT 'pending',
    requested_at INTEGER NOT NULL,
    decided_at INTEGER
);

CREATE INDEX permission_thread_req ON permission(thread_id, requested_at, event_id);
CREATE INDEX permission_turn_req ON permission(turn_id, requested_at, event_id);

-- Agent profile: device-local metadata.
CREATE TABLE agent_profile (
    agent_profile_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    preferred_harness TEXT NOT NULL DEFAULT 'acp',
    memory_mode TEXT NOT NULL DEFAULT 'long_term',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- ACP harness binding: device-local launch configuration.
CREATE TABLE harness_binding (
    harness_binding_id TEXT PRIMARY KEY,
    agent_profile_id TEXT NOT NULL,
    label TEXT NOT NULL,
    command TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX harness_binding_agent ON harness_binding(agent_profile_id);

-- Project reference: device-local path association.
CREATE TABLE project_ref (
    project_id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Domain projection recovery marker (separate from the v1 protocol
-- projection marker so fold versions are independent).
CREATE TABLE domain_projection_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    journal_max_seq INTEGER NOT NULL,
    projection_version INTEGER NOT NULL,
    rebuilt_at INTEGER NOT NULL
);

-- FTS5 virtual table for thread search by title.
CREATE VIRTUAL TABLE thread_search USING fts5(
    thread_id UNINDEXED,
    title,
    content='thread',
    content_rowid='rowid'
);

-- Triggers to keep FTS in sync with the thread table.
CREATE TRIGGER thread_search_insert AFTER INSERT ON thread BEGIN
    INSERT INTO thread_search(rowid, thread_id, title) VALUES (new.rowid, new.thread_id, new.title);
END;

CREATE TRIGGER thread_search_delete AFTER DELETE ON thread BEGIN
    INSERT INTO thread_search(thread_search, rowid, thread_id, title) VALUES ('delete', old.rowid, old.thread_id, old.title);
END;

CREATE TRIGGER thread_search_update AFTER UPDATE OF title ON thread BEGIN
    INSERT INTO thread_search(thread_search, rowid, thread_id, title) VALUES ('delete', old.rowid, old.thread_id, old.title);
    INSERT INTO thread_search(rowid, thread_id, title) VALUES (new.rowid, new.thread_id, new.title);
END;
";

/// P1.1 hardening marker. A projection digest detects deleted or tampered
/// rows even when the journal high-water sequence happens to match.
const SCHEMA_V3: &str = r"
ALTER TABLE domain_projection_state ADD COLUMN projection_digest TEXT NOT NULL DEFAULT '';
";

/// P1.2 runtime boundary checkpointing and thread session bindings (ADR 0002, ADR 0013).
/// Device-local tables, excluded from domain projection rebuilds.
const SCHEMA_V4: &str = r"
CREATE TABLE runtime_checkpoint (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    turn_id TEXT,
    operation_id TEXT NOT NULL,
    boundary_kind TEXT NOT NULL,
    state TEXT NOT NULL,
    remote_request_id TEXT,
    diagnostic_summary TEXT,
    created_at INTEGER NOT NULL,
    settled_at INTEGER
);

CREATE INDEX runtime_checkpoint_thread_created ON runtime_checkpoint(thread_id, created_at, id);
CREATE INDEX runtime_checkpoint_state ON runtime_checkpoint(state, created_at, id);
CREATE INDEX runtime_checkpoint_op ON runtime_checkpoint(operation_id);

CREATE TABLE thread_session_binding (
    thread_id TEXT PRIMARY KEY,
    harness_binding_id TEXT NOT NULL,
    opaque_session_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
";
