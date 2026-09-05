//! Integration tests for device-local identity documents and `ContextSnapshot` storage (P2.2, ADR 0018).

use std::str::FromStr;

use altior_domain::{
    AgentProfile, AgentProfileId, ContextDegradation, ContextDropReason, ContextDroppedEntry,
    ContextIdentityEntry, ContextMemoryEntry, ContextSnapshot, ContextSnapshotListLimit,
    ContextTokenBudget, DisplayName, IdentityContent, IdentityDocument, IdentityDocumentId,
    IdentityDocumentKind, IdentityDocumentListLimit, MemoryDraft, MemoryId, MemoryKind,
    MemoryScope, MemorySearchLimit, MemorySensitivity, MemorySource, MemoryState, SearchQuery,
    ThreadId, TurnId, UnixMillis,
};
use altior_storage::{StorageError, Store, domain_projection_digest};

fn t(ms: u64) -> UnixMillis {
    UnixMillis::from_millis(ms)
}

fn doc_id(n: u32) -> IdentityDocumentId {
    IdentityDocumentId::from_str(&format!("idd_{n:016x}")).expect("valid doc id")
}

fn thread_id(n: u32) -> ThreadId {
    ThreadId::from_str(&format!("thr_{n:016x}")).expect("valid thread id")
}

fn turn_id(n: u32) -> TurnId {
    TurnId::from_str(&format!("trn_{n:016x}")).expect("valid turn id")
}

fn memory_id(n: u32) -> MemoryId {
    MemoryId::from_str(&format!("mem_{n:016x}")).expect("valid memory id")
}

fn fixture_doc(n: u32, kind: IdentityDocumentKind, content: &str) -> IdentityDocument {
    IdentityDocument {
        id: doc_id(n),
        kind,
        content: IdentityContent::try_from(content).expect("valid content"),
        created_at: t(1000 + u64::from(n)),
        updated_at: t(1000 + u64::from(n)),
    }
}

fn fixture_snapshot(turn_num: u32, thread_num: u32) -> ContextSnapshot {
    ContextSnapshot {
        turn_id: turn_id(turn_num),
        thread_id: thread_id(thread_num),
        memory_mode: "long_term".to_owned(),
        created_at: t(5000 + u64::from(turn_num)),
        passthrough: false,
        budget: ContextTokenBudget {
            identity_limit_tokens: 512,
            memory_limit_tokens: 1024,
            prompt_tokens: 12,
            identity_tokens: 18,
            memory_tokens: 34,
            total_tokens: 64,
        },
        identity: vec![ContextIdentityEntry {
            document_id: doc_id(1),
            kind: IdentityDocumentKind::Name,
            tokens: 6,
        }],
        memories: vec![ContextMemoryEntry {
            memory_id: memory_id(1),
            kind: MemoryKind::Fact,
            scope: MemoryScope::Global,
            confidence: 95,
            explicit: true,
            tokens: 20,
            score: 0.88,
            why_selected: "matched user preference on telemetry".to_owned(),
            provenance_thread_id: Some(thread_id(thread_num)),
            provenance_turn_id: None,
        }],
        dropped: vec![ContextDroppedEntry {
            memory_id: memory_id(2),
            tokens: 45,
            rank: 2,
            reason: ContextDropReason::BudgetExhausted,
        }],
        degraded: None,
        rendered_prompt: Some("[User Identity]\n- Name: Ada\n\n[Relevant Context]\n- [Fact] Likes telemetry off\n\nPrompt".to_owned()),
    }
}

// ── Identity Document CRUD & Idempotency ───────────────────────────

#[test]
fn identity_document_crud_lifecycle() {
    let mut store = Store::open_in_memory().expect("open store");

    let doc = fixture_doc(1, IdentityDocumentKind::Name, "Ada Lovelace");

    // Create / Put
    store.put_identity_document(&doc).expect("put document");
    assert_eq!(store.count_identity_documents().expect("count"), 1);

    // Get / Lookup
    let fetched = store
        .get_identity_document(&doc.id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, doc);

    let fetched_alias = store
        .identity_document_by_id(&doc.id)
        .expect("get alias")
        .expect("must exist");
    assert_eq!(fetched_alias, doc);

    // Update
    let mut updated = doc.clone();
    updated.content = IdentityContent::try_from("Augusta Ada King").expect("valid content");
    updated.updated_at = t(2000);
    store
        .update_identity_document(&updated)
        .expect("update document");

    let fetched_updated = store
        .get_identity_document(&doc.id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched_updated.content.as_str(), "Augusta Ada King");
    assert_eq!(fetched_updated.updated_at, t(2000));

    // Delete
    store
        .delete_identity_document(&doc.id)
        .expect("delete document");
    assert_eq!(store.count_identity_documents().expect("count"), 0);
    assert_eq!(
        store
            .get_identity_document(&doc.id)
            .expect("get after delete"),
        None
    );
}

#[test]
fn identity_document_put_idempotency_and_conflict() {
    let mut store = Store::open_in_memory().expect("open store");

    let doc = fixture_doc(1, IdentityDocumentKind::Preference, "Prefers concise prose");
    store.put_identity_document(&doc).expect("first put");

    // Idempotent put: identical kind and content succeeds with Ok(())
    store
        .put_identity_document(&doc)
        .expect("identical put must succeed idempotently");

    // Conflict: same ID, but different content
    let mut conflicting_content = doc.clone();
    conflicting_content.content =
        IdentityContent::try_from("Prefers verbose prose").expect("valid content");

    let err = store
        .put_identity_document(&conflicting_content)
        .expect_err("conflicting content must fail");
    assert!(
        matches!(err, StorageError::IdentityDocumentConflict { ref identity_document_id } if identity_document_id == doc.id.as_str()),
        "expected IdentityDocumentConflict, got {err:?}"
    );

    // Conflict: same ID, but different kind
    let mut conflicting_kind = doc.clone();
    conflicting_kind.kind = IdentityDocumentKind::Instruction;

    let err_kind = store
        .put_identity_document(&conflicting_kind)
        .expect_err("conflicting kind must fail");
    assert!(
        matches!(err_kind, StorageError::IdentityDocumentConflict { .. }),
        "expected IdentityDocumentConflict, got {err_kind:?}"
    );

    // Content in database remains untouched
    let fetched = store
        .get_identity_document(&doc.id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched.content.as_str(), "Prefers concise prose");
}

#[test]
fn identity_document_typed_not_found() {
    let mut store = Store::open_in_memory().expect("open store");

    let non_existent_id = doc_id(999);

    // Delete non-existent
    let err_del = store
        .delete_identity_document(&non_existent_id)
        .expect_err("delete non-existent must fail");
    assert!(
        matches!(err_del, StorageError::IdentityDocumentNotFound { ref identity_document_id } if identity_document_id == non_existent_id.as_str())
    );

    // Update non-existent
    let doc = fixture_doc(999, IdentityDocumentKind::About, "Non-existent bio");
    let err_upd = store
        .update_identity_document(&doc)
        .expect_err("update non-existent must fail");
    assert!(
        matches!(err_upd, StorageError::IdentityDocumentNotFound { ref identity_document_id } if identity_document_id == non_existent_id.as_str())
    );

    // Get non-existent
    let opt = store
        .get_identity_document(&non_existent_id)
        .expect("get non-existent returns Ok(None)");
    assert_eq!(opt, None);
}

#[test]
fn identity_document_secret_shape_fail_closed_zero_pollution() {
    let mut store = Store::open_in_memory().expect("open store");

    // AWS access key secret shape
    let secret_content = "My key is AKIAIOSFODNN7EXAMPLE for AWS";
    let doc = IdentityDocument {
        id: doc_id(1),
        kind: IdentityDocumentKind::Instruction,
        content: IdentityContent::try_from(secret_content).expect("valid content length"),
        created_at: t(100),
        updated_at: t(100),
    };

    let err = store
        .put_identity_document(&doc)
        .expect_err("must reject secret content");
    assert!(matches!(err, StorageError::SecretShapedContent));

    // Verify zero database rows written
    assert_eq!(store.count_identity_documents().expect("count"), 0);

    // Also verify update refuses secret shape and leaves original intact
    let benign = fixture_doc(2, IdentityDocumentKind::About, "Benign biography");
    store.put_identity_document(&benign).expect("put benign");

    let mut polluted_update = benign.clone();
    polluted_update.content =
        IdentityContent::try_from("ghp_123456789012345678901234567890123456").expect("content");
    polluted_update.updated_at = t(2000);

    let err_upd = store
        .update_identity_document(&polluted_update)
        .expect_err("must reject secret update");
    assert!(matches!(err_upd, StorageError::SecretShapedContent));

    let fetched = store
        .get_identity_document(&benign.id)
        .expect("get")
        .expect("intact");
    assert_eq!(fetched.content.as_str(), "Benign biography");
}

#[test]
fn identity_document_device_cap_exceeded() {
    let mut store = Store::open_in_memory().expect("open store");

    // Fill up to the maximum capacity of 32
    for i in 1..=32 {
        let doc = fixture_doc(i, IdentityDocumentKind::Preference, &format!("Pref #{i}"));
        store
            .put_identity_document(&doc)
            .expect("insert within cap");
    }
    assert_eq!(store.count_identity_documents().expect("count"), 32);

    // 33rd must fail with IdentityDocumentCountExceeded
    let overflow = fixture_doc(33, IdentityDocumentKind::Preference, "Pref #33");
    let err = store
        .put_identity_document(&overflow)
        .expect_err("33rd doc must exceed limit");
    assert!(
        matches!(
            err,
            StorageError::IdentityDocumentCountExceeded { count: 32, max: 32 }
        ),
        "expected IdentityDocumentCountExceeded, got {err:?}"
    );

    // Idempotent re-put of existing document 1 still succeeds even at cap
    let existing = fixture_doc(1, IdentityDocumentKind::Preference, "Pref #1");
    store
        .put_identity_document(&existing)
        .expect("re-put existing at cap is idempotent");
}

#[test]
fn identity_document_list_ordering_by_render_priority() {
    let mut store = Store::open_in_memory().expect("open store");

    // Insert out of priority order: Preference (3), Instruction (2), About (1), Name (0)
    let p = fixture_doc(1, IdentityDocumentKind::Preference, "Preference doc");
    let i = fixture_doc(2, IdentityDocumentKind::Instruction, "Instruction doc");
    let a = fixture_doc(3, IdentityDocumentKind::About, "About doc");
    let n = fixture_doc(4, IdentityDocumentKind::Name, "Name doc");

    store.put_identity_document(&p).expect("put");
    store.put_identity_document(&i).expect("put");
    store.put_identity_document(&a).expect("put");
    store.put_identity_document(&n).expect("put");

    let list = store
        .list_identity_documents(IdentityDocumentListLimit::default())
        .expect("list");
    assert_eq!(list.len(), 4);
    assert_eq!(list[0].kind, IdentityDocumentKind::Name);
    assert_eq!(list[1].kind, IdentityDocumentKind::About);
    assert_eq!(list[2].kind, IdentityDocumentKind::Instruction);
    assert_eq!(list[3].kind, IdentityDocumentKind::Preference);

    // Test bounded limit
    let limited = store
        .list_identity_documents(IdentityDocumentListLimit::try_new(2).expect("limit 2"))
        .expect("limited list");
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[0].kind, IdentityDocumentKind::Name);
    assert_eq!(limited[1].kind, IdentityDocumentKind::About);
}

// ── ContextSnapshot Persist, Lookup, Idempotency & Conflict ─────────

#[test]
fn context_snapshot_roundtrip_with_explainability_and_rendered_prompt() {
    let mut store = Store::open_in_memory().expect("open store");

    let snapshot = fixture_snapshot(1, 1);
    store
        .record_context_snapshot(&snapshot)
        .expect("record snapshot");

    let fetched = store
        .get_context_snapshot(&snapshot.turn_id)
        .expect("get snapshot")
        .expect("must exist");

    assert_eq!(fetched, snapshot);
    assert_eq!(
        fetched.rendered_prompt,
        Some("[User Identity]\n- Name: Ada\n\n[Relevant Context]\n- [Fact] Likes telemetry off\n\nPrompt".to_owned())
    );
    let memory_score = fetched.memories[0].score;
    assert!((memory_score - 0.88).abs() < f64::EPSILON);
    assert_eq!(
        fetched.memories[0].why_selected,
        "matched user preference on telemetry"
    );
    assert_eq!(
        fetched.dropped[0].reason,
        ContextDropReason::BudgetExhausted
    );
}

#[test]
fn context_snapshot_idempotency_and_conflict() {
    let mut store = Store::open_in_memory().expect("open store");

    let snapshot = fixture_snapshot(1, 1);
    store
        .record_context_snapshot(&snapshot)
        .expect("first record");

    // Idempotent re-record succeeds
    store
        .record_context_snapshot(&snapshot)
        .expect("identical record must succeed idempotently");

    // Conflicting snapshot with different budget
    let mut conflicting = snapshot.clone();
    conflicting.budget.total_tokens = 999;

    let err = store
        .record_context_snapshot(&conflicting)
        .expect_err("conflicting snapshot must fail");
    assert!(
        matches!(err, StorageError::ContextSnapshotConflict { ref turn_id } if turn_id == snapshot.turn_id.as_str()),
        "expected ContextSnapshotConflict, got {err:?}"
    );

    // Conflicting snapshot with different memory list
    let mut conflicting_mem = snapshot.clone();
    conflicting_mem.memories.clear();

    let err_mem = store
        .record_context_snapshot(&conflicting_mem)
        .expect_err("conflicting memories must fail");
    assert!(matches!(
        err_mem,
        StorageError::ContextSnapshotConflict { .. }
    ));
}

#[test]
fn context_snapshot_secret_shape_fail_closed_zero_pollution() {
    let mut store = Store::open_in_memory().expect("open store");

    // 1. Secret in why_selected
    let mut secret_why = fixture_snapshot(1, 1);
    secret_why.memories[0].why_selected =
        "matched token password=hunter2_not_real_credential_value".to_owned();

    let err1 = store
        .record_context_snapshot(&secret_why)
        .expect_err("must reject secret in why_selected");
    assert!(matches!(err1, StorageError::SecretShapedContent));
    assert_eq!(
        store
            .get_context_snapshot(&secret_why.turn_id)
            .expect("lookup"),
        None
    );

    // 2. Secret in rendered_prompt
    let mut secret_prompt = fixture_snapshot(2, 1);
    secret_prompt.rendered_prompt = Some("Here is your key AKIAIOSFODNN7EXAMPLE".to_owned());

    let err2 = store
        .record_context_snapshot(&secret_prompt)
        .expect_err("must reject secret in rendered_prompt");
    assert!(matches!(err2, StorageError::SecretShapedContent));
    assert_eq!(
        store
            .get_context_snapshot(&secret_prompt.turn_id)
            .expect("lookup"),
        None
    );

    // 3. Secret in degraded detail
    let mut secret_deg = fixture_snapshot(3, 1);
    secret_deg.degraded = Some(ContextDegradation {
        code: "secret_leak".to_owned(),
        detail: "bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.doNotLeakThisSignatureHerePlease12".to_owned(),
    });

    let err3 = store
        .record_context_snapshot(&secret_deg)
        .expect_err("must reject secret in degraded");
    assert!(matches!(err3, StorageError::SecretShapedContent));
    assert_eq!(
        store
            .get_context_snapshot(&secret_deg.turn_id)
            .expect("lookup"),
        None
    );
}

#[test]
fn context_snapshot_corrupt_payload_handling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("corrupt_test.db");

    let mut store = Store::open(&path).expect("open");
    let snapshot = fixture_snapshot(1, 1);
    store.record_context_snapshot(&snapshot).expect("record");
    drop(store);

    // Tamper with payload_json directly via raw rusqlite
    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.execute(
        "UPDATE context_snapshot SET payload_json = '{ corrupted json...'",
        [],
    )
    .expect("corrupt payload");
    drop(raw);

    let store = Store::open(&path).expect("reopen");
    let err = store
        .get_context_snapshot(&snapshot.turn_id)
        .expect_err("corrupt json must return typed error");
    assert!(
        matches!(err, StorageError::ContextSnapshotCorrupt { ref turn_id, .. } if turn_id == snapshot.turn_id.as_str())
    );
}

#[test]
fn context_snapshot_list_for_thread() {
    let mut store = Store::open_in_memory().expect("open");

    let s1 = fixture_snapshot(1, 1);
    let s2 = fixture_snapshot(2, 1);
    let s3 = fixture_snapshot(3, 2); // Different thread

    store.record_context_snapshot(&s1).expect("record s1");
    store.record_context_snapshot(&s2).expect("record s2");
    store.record_context_snapshot(&s3).expect("record s3");

    let thread1_snaps = store
        .list_context_snapshots_for_thread(
            &thread_id(1),
            ContextSnapshotListLimit::try_new(10).expect("limit"),
        )
        .expect("list");

    assert_eq!(thread1_snaps.len(), 2);
    // Ordered by created_at DESC (s2 has higher timestamp than s1)
    assert_eq!(thread1_snaps[0].turn_id, turn_id(2));
    assert_eq!(thread1_snaps[1].turn_id, turn_id(1));
}

// ── Isolation from Projection Digest and Rebuild ───────────────────

#[test]
fn context_snapshot_and_identity_isolated_from_projection_digest_and_rebuild() {
    let mut store = Store::open_in_memory().expect("open store");

    // 1. Seed domain data
    let agp = AgentProfile {
        id: AgentProfileId::from_str("agp_testprofile00001").expect("agp id"),
        display_name: DisplayName::try_from("Test Agent").expect("name"),
        preferred_harness: altior_domain::HarnessKind::Acp,
        memory_mode: altior_domain::MemoryMode::LongTerm,
        created_at: t(100),
        updated_at: t(100),
    };
    store.create_agent_profile(&agp).expect("create profile");

    let mem_draft = MemoryDraft {
        id: None,
        content: altior_domain::MemoryContent::try_from("Domain fact").expect("content"),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: altior_domain::MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    store.create_memory(&mem_draft, t(200)).expect("memory");

    // Baseline digest before adding identity document or context snapshot
    let raw = rusqlite::Connection::open_in_memory().expect("raw");
    // Backup current DB to compute digest on raw connection
    let backup_dir = tempfile::tempdir().expect("tempdir");
    let db_path = backup_dir.path().join("isolation.db");
    drop(raw);

    let mut disk_store = Store::open(&db_path).expect("open disk");
    disk_store.create_agent_profile(&agp).expect("agp");
    disk_store.create_memory(&mem_draft, t(200)).expect("mem");

    let raw_conn = rusqlite::Connection::open(&db_path).expect("raw open");
    let baseline_digest = domain_projection_digest(&raw_conn).expect("baseline digest");
    drop(raw_conn);

    // 2. Now add identity documents and context snapshots
    let doc = fixture_doc(1, IdentityDocumentKind::Instruction, "Local instruction");
    disk_store.put_identity_document(&doc).expect("put doc");

    let snap = fixture_snapshot(1, 1);
    disk_store
        .record_context_snapshot(&snap)
        .expect("record snapshot");

    // 3. Digest MUST be identical! Neither table enters the projection digest
    let raw_conn2 = rusqlite::Connection::open(&db_path).expect("raw open");
    let digest_after_v7_writes = domain_projection_digest(&raw_conn2).expect("digest after writes");
    drop(raw_conn2);
    assert_eq!(
        digest_after_v7_writes, baseline_digest,
        "identity documents and context snapshots must not alter domain projection digest"
    );

    // 4. Rebuild domain projections
    disk_store
        .rebuild_domain_projections()
        .expect("rebuild projections");

    // 5. Verify identity documents and context snapshots are NOT wiped or disturbed
    let doc_after_rebuild = disk_store
        .get_identity_document(&doc.id)
        .expect("get doc")
        .expect("identity document must survive projection rebuild");
    assert_eq!(doc_after_rebuild, doc);

    let snap_after_rebuild = disk_store
        .get_context_snapshot(&snap.turn_id)
        .expect("get snapshot")
        .expect("context snapshot must survive projection rebuild");
    assert_eq!(snap_after_rebuild, snap);
}

// ── Schema v6 -> v7 Migration & Preservation ───────────────────────

const V6_SCHEMA_SQL: &str = r"
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
CREATE TABLE thread_projection (
    thread_id TEXT PRIMARY KEY,
    event_count INTEGER NOT NULL,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_event_id TEXT NOT NULL,
    last_kind TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE projection_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    journal_max_seq INTEGER NOT NULL,
    projection_version INTEGER NOT NULL,
    rebuilt_at INTEGER NOT NULL
);
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
CREATE TABLE agent_profile (
    agent_profile_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    preferred_harness TEXT NOT NULL DEFAULT 'acp',
    memory_mode TEXT NOT NULL DEFAULT 'long_term',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE harness_binding (
    harness_binding_id TEXT PRIMARY KEY,
    agent_profile_id TEXT NOT NULL,
    label TEXT NOT NULL,
    command TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    args_json TEXT NOT NULL DEFAULT '[]',
    env_keys_json TEXT NOT NULL DEFAULT '[]',
    secret_refs_json TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX harness_binding_agent ON harness_binding(agent_profile_id);
CREATE TABLE project_ref (
    project_id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE domain_projection_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    journal_max_seq INTEGER NOT NULL,
    projection_version INTEGER NOT NULL,
    rebuilt_at INTEGER NOT NULL,
    projection_digest TEXT NOT NULL DEFAULT ''
);
CREATE VIRTUAL TABLE thread_search USING fts5(
    thread_id UNINDEXED,
    title,
    content='thread',
    content_rowid='rowid'
);
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
CREATE TABLE memory (
    memory_id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_target TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    confidence INTEGER NOT NULL,
    sensitivity TEXT NOT NULL,
    source TEXT NOT NULL,
    explicit INTEGER NOT NULL,
    provenance_thread_id TEXT,
    provenance_turn_id TEXT,
    excerpt TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    expires_at INTEGER,
    superseded_by TEXT
) STRICT;
CREATE INDEX memory_updated ON memory(updated_at);
CREATE INDEX memory_state_updated ON memory(state, updated_at);
CREATE VIRTUAL TABLE memory_fts USING fts5(
    memory_id UNINDEXED,
    content,
    tokenize='porter unicode61'
);
CREATE TRIGGER memory_fts_insert AFTER INSERT ON memory
WHEN new.state = 'confirmed' AND new.superseded_by IS NULL
BEGIN
    INSERT INTO memory_fts (memory_id, content) VALUES (new.memory_id, new.content);
END;
CREATE TRIGGER memory_fts_update AFTER UPDATE ON memory
BEGIN
    DELETE FROM memory_fts WHERE memory_id = old.memory_id;
    INSERT INTO memory_fts (memory_id, content)
    SELECT new.memory_id, new.content
    WHERE new.state = 'confirmed' AND new.superseded_by IS NULL;
END;
CREATE TRIGGER memory_fts_delete AFTER DELETE ON memory
BEGIN
    DELETE FROM memory_fts WHERE memory_id = old.memory_id;
END;
PRAGMA user_version = 6;
";

#[test]
fn migration_v6_to_v7_preserves_journal_and_memory_and_enables_v7() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("v6_migration_test.db");

    // 1. Manually initialize raw SQLite with genuine schema v6
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("wal");
        conn.execute_batch(V6_SCHEMA_SQL).expect("apply v6 schema");

        // Seed legacy v1 journal row
        conn.execute(
            "INSERT INTO journal (event_id, stream_sequence, occurred_at, kind, payload)
             VALUES ('evt_v6seed0000000000000000000001', 1, 1000, 'ThreadStarted', x'00')",
            [],
        )
        .expect("seed v1 journal");

        // Seed domain agent_profile
        conn.execute(
            "INSERT INTO agent_profile (agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at)
             VALUES ('agp_v6test0000000001', 'Legacy Agent', 'acp', 'long_term', 1000, 1000)",
            [],
        )
        .expect("seed agent profile");

        // Seed memory row (confirmed)
        conn.execute(
            "INSERT INTO memory (
                memory_id, content, scope_kind, scope_target, kind, state,
                confidence, sensitivity, source, explicit, provenance_thread_id,
                provenance_turn_id, excerpt, created_at, updated_at, expires_at, superseded_by
             ) VALUES (
                'mem_v6seed0000000001', 'User prefers Rust over Python', 'global', NULL,
                'preference', 'confirmed', 100, 'normal', 'explicit', 1,
                NULL, NULL, NULL, 2000, 2000, NULL, NULL
             )",
            [],
        )
        .expect("seed memory");

        // A genuine v6 projection is backed by the append-only domain journal.
        // Store::open may deterministically rebuild projections, so seed the
        // corresponding event rather than relying on an orphan projection row.
        conn.execute(
            "INSERT INTO domain_journal (event_id, kind, payload, occurred_at)
             VALUES (
               'evt_v6memory000000000000000001',
               'memory.confirmed',
               CAST(?1 AS BLOB),
               2000
             )",
            [r#"{"memory_id":"mem_v6seed0000000001","content":"User prefers Rust over Python","scope_kind":"global","scope_target":null,"kind":"preference","state":"confirmed","confidence":100,"sensitivity":"normal","source":"explicit","explicit":true,"provenance_thread_id":null,"provenance_turn_id":null,"excerpt":null,"expires_at":null}"#],
        )
        .expect("seed memory domain event");

        // Verify that raw database is stamped at version 6
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read version");
        assert_eq!(v, 6);
    }

    // 2. Open with Store::open: must migrate from v6 to v7
    {
        let mut store = Store::open(&db_path).expect("open and migrate to v7");
        assert_eq!(store.schema_version().expect("schema_version"), 7);

        // Verify v1 journal row is intact
        assert_eq!(store.journal_len().expect("v1 journal len"), 1);

        // Verify seeded memory is preserved and searchable via FTS5
        let mem = store
            .get_memory(&MemoryId::from_str("mem_v6seed0000000001").unwrap())
            .expect("memory lookup")
            .expect("must exist");
        assert_eq!(mem.content.as_str(), "User prefers Rust over Python");
        assert_eq!(mem.state, MemoryState::Confirmed);

        let q = SearchQuery::try_from("Rust").expect("search query");
        let hits = store
            .search_memories(&q, None, MemorySearchLimit::default_limit(), t(3000))
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.memory_id, mem.memory_id);

        // Verify new v7 operations succeed on migrated DB
        let doc = fixture_doc(1, IdentityDocumentKind::Name, "Migrated User");
        store.put_identity_document(&doc).expect("put v7 doc");
        assert_eq!(store.count_identity_documents().expect("count"), 1);

        let snap = fixture_snapshot(1, 1);
        store
            .record_context_snapshot(&snap)
            .expect("record v7 snapshot");
        let fetched_snap = store
            .get_context_snapshot(&snap.turn_id)
            .expect("get snapshot")
            .expect("exists");
        assert_eq!(fetched_snap, snap);
    }

    // 3. Reopen: verify everything survives restart
    {
        let store = Store::open(&db_path).expect("reopen store");
        assert_eq!(store.schema_version().expect("schema_version"), 7);
        assert_eq!(store.journal_len().expect("v1 journal len"), 1);
        assert_eq!(store.count_identity_documents().expect("doc count"), 1);
        assert!(
            store
                .get_context_snapshot(&turn_id(1))
                .expect("get snap")
                .is_some()
        );
    }
}
