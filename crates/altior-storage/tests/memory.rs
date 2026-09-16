//! P2.1 memory persistence, lifecycle transitions, and search retrieval tests (ADR 0017).
//!
//! Deterministic throughout: fixture timestamps, in-memory SQLite or tempfiles.
//! No sleeps, no network, no wall clock.

use altior_domain::{
    MemoryContent, MemoryDraft, MemoryExcerpt, MemoryKind, MemoryListLimit, MemoryProvenance,
    MemoryScope, MemorySearchLimit, MemorySensitivity, MemorySource, MemoryState, SearchQuery,
    UnixMillis,
};
use altior_storage::{StorageError, Store, domain_projection_digest};

/// Fixture base epoch (2024-01-01T00:00:00Z).
const BASE_MILLIS: u64 = 1_704_067_200_000;

fn t(offset_ms: u64) -> UnixMillis {
    UnixMillis::from_millis(BASE_MILLIS + offset_ms)
}

fn fixture_draft(content: &str) -> MemoryDraft {
    MemoryDraft {
        id: None,
        content: MemoryContent::try_from(content).expect("valid memory content"),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: Some(MemoryExcerpt::try_from("user said preference").expect("valid excerpt")),
        },
        expires_at: None,
    }
}

// ── (a) Propose -> Confirm happy path & invalid transitions ────────────────

#[test]
fn memory_lifecycle_propose_confirm_and_invalid_transitions() {
    let mut store = Store::open_in_memory().expect("open store");

    // 1. Propose memory -> Candidate state
    let draft = fixture_draft("User prefers dark mode");
    let proposed = store.propose_memory(draft, t(100)).expect("propose memory");
    assert_eq!(proposed.state, MemoryState::Candidate);
    assert_eq!(proposed.source, MemorySource::Inferred);

    // Candidates must not appear in search results
    let q = SearchQuery::try_from("dark mode").expect("search query");
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(200))
        .expect("search");
    assert!(hits.is_empty(), "candidates must not appear in search");

    // 2. Confirm memory -> Confirmed state
    let confirmed = store
        .confirm_memory(&proposed.memory_id, t(300))
        .expect("confirm memory");
    assert_eq!(confirmed.state, MemoryState::Confirmed);

    // Now confirmed memory appears in search
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(400))
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.memory_id, confirmed.memory_id);

    // 3. Invalid transition: Confirming an already confirmed memory
    let err = store
        .confirm_memory(&proposed.memory_id, t(500))
        .expect_err("cannot confirm already confirmed memory");
    assert!(
        matches!(err, StorageError::InvalidDomainEvent { .. }),
        "expected InvalidDomainEvent, got {err:?}"
    );

    // 4. Invalid transition: Rejecting an already confirmed memory
    let err = store
        .reject_memory(&proposed.memory_id, Some("not true"), t(600))
        .expect_err("cannot reject confirmed memory");
    assert!(
        matches!(err, StorageError::InvalidDomainEvent { .. }),
        "expected InvalidDomainEvent, got {err:?}"
    );

    // 5. Test rejection on candidate memory
    let draft2 = fixture_draft("User dislikes TypeScript");
    let proposed2 = store
        .propose_memory(draft2, t(700))
        .expect("propose memory 2");
    assert_eq!(proposed2.state, MemoryState::Candidate);

    let rejected = store
        .reject_memory(&proposed2.memory_id, Some("user actually loves TS"), t(800))
        .expect("reject candidate");
    assert_eq!(rejected.state, MemoryState::Rejected);

    // Rejection again must fail
    let err = store
        .reject_memory(&proposed2.memory_id, None, t(900))
        .expect_err("cannot reject already rejected memory");
    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));

    // Confirming rejected must fail
    let err = store
        .confirm_memory(&proposed2.memory_id, t(1000))
        .expect_err("cannot confirm rejected memory");
    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));
}

// ── (b) Correction supersedes ───────────────────────────────────────────────

#[test]
fn memory_correction_supersedes_original_and_updates_search() {
    let mut store = Store::open_in_memory().expect("open store");

    // Create confirmed memory
    let draft = fixture_draft("User primary email is alpha_old_address@example.com");
    let original = store
        .create_memory(&draft, t(100))
        .expect("create original memory");
    assert_eq!(original.state, MemoryState::Confirmed);

    // Verify search finds original
    let q_old = SearchQuery::try_from("alpha_old_address").expect("search query");
    let hits_old = store
        .search_memories(&q_old, None, MemorySearchLimit::default_limit(), t(200))
        .expect("search");
    assert_eq!(hits_old.len(), 1);
    assert_eq!(hits_old[0].record.memory_id, original.memory_id);

    // Correct the memory
    let new_draft = fixture_draft("User primary email is beta_new_address@example.com");
    let replacement = store
        .correct_memory(&original.memory_id, &new_draft, t(300))
        .expect("correct memory");
    assert_eq!(replacement.state, MemoryState::Confirmed);
    assert_ne!(replacement.memory_id, original.memory_id);

    // Original memory is kept in database with superseded_by set
    let original_reloaded = store
        .get_memory(&original.memory_id)
        .expect("get original")
        .expect("found");
    assert_eq!(
        original_reloaded.superseded_by,
        Some(replacement.memory_id.clone())
    );

    // Original must NO LONGER appear in search
    let hits_old_after = store
        .search_memories(&q_old, None, MemorySearchLimit::default_limit(), t(400))
        .expect("search old");
    assert!(
        hits_old_after.is_empty(),
        "superseded memory must not appear in search"
    );

    // Replacement is retrievable in search
    let q_new = SearchQuery::try_from("beta_new_address").expect("search query");
    let hits_new = store
        .search_memories(&q_new, None, MemorySearchLimit::default_limit(), t(500))
        .expect("search new");
    assert_eq!(hits_new.len(), 1);
    assert_eq!(hits_new[0].record.memory_id, replacement.memory_id);

    // Journal contains events for both creation and superseding
    let domain_journal_len = store.domain_journal_len().expect("journal count");
    assert!(
        domain_journal_len >= 3,
        "journal should record create, new create, and supersede"
    );

    // Correcting an already superseded memory must fail
    let another_draft = fixture_draft("User primary email is gamma_address@example.com");
    let err = store
        .correct_memory(&original.memory_id, &another_draft, t(600))
        .expect_err("cannot correct already superseded memory");
    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));
}

// ── (c) Forget ─────────────────────────────────────────────────────────────

#[test]
fn memory_forget_tombstone_retained_in_db_and_excluded_from_search() {
    let mut store = Store::open_in_memory().expect("open store");

    let draft = fixture_draft("User lives in Amsterdam");
    let memory = store.create_memory(&draft, t(100)).expect("create memory");
    assert_eq!(memory.state, MemoryState::Confirmed);

    let q = SearchQuery::try_from("Amsterdam").expect("query");
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(200))
        .expect("search");
    assert_eq!(hits.len(), 1);

    let journal_before = store.domain_journal_len().expect("journal count");

    // Forget memory
    let forgotten = store
        .forget_memory(&memory.memory_id, t(300))
        .expect("forget memory");
    assert_eq!(forgotten.state, MemoryState::Forgotten);

    // Row retained in database
    let reloaded = store
        .get_memory(&memory.memory_id)
        .expect("get")
        .expect("found");
    assert_eq!(reloaded.state, MemoryState::Forgotten);

    // Excluded from search
    let hits_after = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(400))
        .expect("search after forget");
    assert!(
        hits_after.is_empty(),
        "forgotten memory excluded from search"
    );

    // Tombstone event appended in journal
    let journal_after = store.domain_journal_len().expect("journal count");
    assert_eq!(journal_after, journal_before + 1);

    // Forgetting already forgotten memory fails
    let err = store
        .forget_memory(&memory.memory_id, t(500))
        .expect_err("cannot forget already forgotten memory");
    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));
}

// ── (d) Lazy expiry and sweep_expired_memories ──────────────────────────────

#[test]
fn memory_lazy_expiry_at_search_and_eager_sweep() {
    let mut store = Store::open_in_memory().expect("open store");

    let mut draft = fixture_draft("Temporary access token valid until tomorrow");
    let expires_at = t(1000);
    draft.expires_at = Some(expires_at);

    let memory = store.create_memory(&draft, t(100)).expect("create memory");
    assert_eq!(memory.state, MemoryState::Confirmed);

    let q = SearchQuery::try_from("temporary access").expect("search query");

    // Before expiry: search finds it
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(500))
        .expect("search before expiry");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.memory_id, memory.memory_id);

    // After expiry: search excludes it even before sweep (lazy expiry)
    let hits_expired = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), t(1500))
        .expect("search after expiry");
    assert!(
        hits_expired.is_empty(),
        "lazy expiry must exclude expired record at search time"
    );

    // Still Confirmed in database prior to sweep
    let reloaded = store
        .get_memory(&memory.memory_id)
        .expect("get")
        .expect("found");
    assert_eq!(reloaded.state, MemoryState::Confirmed);

    // Run sweep_expired_memories
    let swept = store
        .sweep_expired_memories(t(1500))
        .expect("sweep expired memories");
    assert_eq!(swept, vec![memory.memory_id.clone()]);

    // Now state in DB is Expired
    let swept_reloaded = store
        .get_memory(&memory.memory_id)
        .expect("get")
        .expect("found");
    assert_eq!(swept_reloaded.state, MemoryState::Expired);
}

// ── (e) Secret-shaped rejection ─────────────────────────────────────────────

#[test]
fn memory_secret_shaped_content_and_excerpt_rejection_leaves_journal_empty() {
    let mut store = Store::open_in_memory().expect("open store");

    let journal_before = store.domain_journal_len().expect("journal count");
    assert_eq!(journal_before, 0);

    let secret_contents = [
        "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC...\n-----END PRIVATE KEY-----",
        "Here is the token: sk-ant-api03-12345678901234567890123456789012345678901234567890",
        "AWS credentials: AKIAIOSFODNN7EXAMPLE and secret",
        "ghp_123456789012345678901234567890123456",
        "github_pat_11AAAAAAA01234567890123456789012345678901234567890123456789012345678901234",
        "password = hunter2secretpasswordvalue123",
    ];

    for secret in secret_contents {
        // 1. Content violation
        let draft = fixture_draft(secret);
        let err = store
            .create_memory(&draft, t(100))
            .expect_err("must reject secret content");
        assert!(
            matches!(err, StorageError::SecretShapedContent),
            "expected SecretShapedContent, got {err:?}"
        );

        // 2. Excerpt violation
        let mut draft_excerpt = fixture_draft("Safe summary of preferences");
        draft_excerpt.provenance.excerpt =
            Some(MemoryExcerpt::try_from(secret).expect("excerpt parsed"));
        let err = store
            .create_memory(&draft_excerpt, t(100))
            .expect_err("must reject secret excerpt");
        assert!(
            matches!(err, StorageError::SecretShapedContent),
            "expected SecretShapedContent on excerpt, got {err:?}"
        );

        // 3. Propose with secret must also be rejected
        let draft_prop = fixture_draft(secret);
        let err = store
            .propose_memory(draft_prop, t(100))
            .expect_err("must reject secret in propose");
        assert!(matches!(err, StorageError::SecretShapedContent));
    }

    // Journal remains completely untouched
    let journal_after = store.domain_journal_len().expect("journal count");
    assert_eq!(
        journal_after, 0,
        "journal must remain empty after secret rejections"
    );

    // Memory table remains completely empty
    let memories = store
        .list_memories(
            None,
            None,
            MemoryListLimit::try_new(10).expect("valid limit"),
            None,
        )
        .expect("list memories");
    assert!(memories.is_empty(), "memory table must be empty");
}

// ── (f) FTS literal escaping ────────────────────────────────────────────────

#[test]
fn memory_fts_literal_escaping_and_special_characters() {
    let mut store = Store::open_in_memory().expect("open store");

    let draft1 = fixture_draft("Expert in C++ and systems programming with g++ toolchain");
    store
        .create_memory(&draft1, t(100))
        .expect("create memory 1");

    let draft2 = fixture_draft("Familiar with Python 3.11 and Rust 2024 edition");
    store
        .create_memory(&draft2, t(200))
        .expect("create memory 2");

    // Search "C++" — should find the C++ memory
    let q_cpp = SearchQuery::try_from("C++").expect("query");
    let hits_cpp = store
        .search_memories(&q_cpp, None, MemorySearchLimit::default_limit(), t(300))
        .expect("search C++");
    assert_eq!(hits_cpp.len(), 1);
    assert!(hits_cpp[0].record.content.as_str().contains("C++"));

    // Search with quotes, parentheses, colons, stars, OR/AND keywords
    let test_queries = [
        "\"systems programming\"",
        "C++ OR Python",
        "Python (3.11)",
        "Rust*",
        "NOT something",
        "syntax:error? [brackets]",
    ];

    for raw_q in test_queries {
        if let Ok(q) = SearchQuery::try_from(raw_q) {
            let res = store.search_memories(&q, None, MemorySearchLimit::default_limit(), t(400));
            assert!(
                res.is_ok(),
                "search for query '{raw_q}' must not error on FTS syntax: {res:?}"
            );
        }
    }
}

// ── (g) Ranking, limits, and explanation ───────────────────────────────────

#[test]
fn memory_ranking_deterministic_scoring_explanations_and_limits() {
    let mut store = Store::open_in_memory().expect("open store");

    // 1. Create a set of memories with known differing properties
    // Memory A: Explicit user, recent (t=900), Project scope
    let project_scope =
        MemoryScope::Project(altior_domain::BoundedLabel::try_from("project-alpha").unwrap());
    let mut draft_a = fixture_draft("Database indexing optimization strategy for postgres");
    draft_a.scope = project_scope.clone();
    draft_a.source = MemorySource::Explicit;
    draft_a.confidence = 100;
    let rec_a = store.create_memory(&draft_a, t(900)).expect("create a");

    // Memory B: Inferred, older (t=100), Global scope
    let mut draft_b = fixture_draft("Database indexing with btree and gin indexes");
    draft_b.scope = MemoryScope::Global;
    draft_b.source = MemorySource::Inferred;
    draft_b.confidence = 70;
    let rec_b = store.create_memory(&draft_b, t(100)).expect("create b");

    // Search querying "database indexing" with project scope filter
    let q = SearchQuery::try_from("database indexing").expect("query");
    let now = t(1000);
    let hits = store
        .search_memories(
            &q,
            Some(&project_scope),
            MemorySearchLimit::default_limit(),
            now,
        )
        .expect("search");

    assert_eq!(hits.len(), 2);
    // A should rank higher due to scope affinity match (1.5), confidence (1.0 vs 0.7), recency, and explicit bonus (+0.2)
    assert_eq!(hits[0].record.memory_id, rec_a.memory_id);
    assert_eq!(hits[1].record.memory_id, rec_b.memory_id);

    // Verify explanation fields
    let exp_a = &hits[0].explanation;
    assert!((exp_a.scope_weight - 1.5).abs() < 1e-6);
    assert!((exp_a.confidence_score - 1.0).abs() < 1e-6);
    assert!((exp_a.explicitness_bonus - 0.2).abs() < 1e-6);
    assert!(exp_a.recency_score > 0.9);
    assert!(!exp_a.matched_terms.is_empty());
    assert!(!exp_a.why_selected.is_empty());

    let exp_b = &hits[1].explanation;
    assert!((exp_b.scope_weight - 1.0).abs() < 1e-6); // Global matches project filter with 1.0
    assert!((exp_b.confidence_score - 0.7).abs() < 1e-6);
    assert!((exp_b.explicitness_bonus - 0.0).abs() < 1e-6);

    // 2. Limit validation
    assert!(matches!(
        MemorySearchLimit::try_new(0),
        Err(altior_domain::EntityError::MemorySearchLimitOutOfRange { .. })
    ));
    assert!(matches!(
        MemorySearchLimit::try_new(65),
        Err(altior_domain::EntityError::MemorySearchLimitOutOfRange { .. })
    ));
    let valid_limit = MemorySearchLimit::try_new(1).expect("valid limit");
    let hits_limited = store
        .search_memories(&q, Some(&project_scope), valid_limit, now)
        .expect("search limited");
    assert_eq!(hits_limited.len(), 1);
}

// ── (h) Projection Rebuild ─────────────────────────────────────────────────

#[test]
fn memory_projection_rebuild_restores_identical_state_and_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("memories.db");

    let (id1, id2) = {
        let mut store = Store::open(&db_path).expect("open store");

        let draft1 = fixture_draft("User speaks German and English fluently");
        let rec1 = store.create_memory(&draft1, t(100)).expect("create 1");

        let draft2 = fixture_draft("User is an expert in Kubernetes deployment");
        let rec2 = store.create_memory(&draft2, t(200)).expect("create 2");

        (rec1.memory_id, rec2.memory_id)
    };

    // Calculate baseline digest before wiping projection
    let baseline_digest = {
        let raw = rusqlite::Connection::open(&db_path).expect("raw open");
        domain_projection_digest(&raw).expect("baseline digest")
    };

    // Wipe memory projection and FTS table using raw SQLite connection
    {
        let raw = rusqlite::Connection::open(&db_path).expect("raw open");
        raw.execute("DELETE FROM memory", []).expect("wipe memory");
        raw.execute("DELETE FROM memory_fts", [])
            .expect("wipe memory_fts");
        // Reset domain projection state marker
        raw.execute("DELETE FROM domain_projection_state", [])
            .expect("wipe marker");
    }

    // Reopen and rebuild domain projections
    {
        let mut store = Store::open(&db_path).expect("reopen store");
        store
            .rebuild_domain_projections()
            .expect("rebuild domain projections");

        // Verify digest is identical to baseline
        let rebuilt_digest = {
            let raw = rusqlite::Connection::open(&db_path).expect("raw open");
            domain_projection_digest(&raw).expect("rebuilt digest")
        };
        assert_eq!(rebuilt_digest, baseline_digest);

        // Verify memory records are restored
        let rec1 = store.get_memory(&id1).expect("get").expect("found 1");
        assert_eq!(
            rec1.content.as_str(),
            "User speaks German and English fluently"
        );

        let rec2 = store.get_memory(&id2).expect("get").expect("found 2");
        assert_eq!(
            rec2.content.as_str(),
            "User is an expert in Kubernetes deployment"
        );

        // Verify FTS search works after rebuild
        let q = SearchQuery::try_from("Kubernetes").expect("query");
        let hits = store
            .search_memories(&q, None, MemorySearchLimit::default_limit(), t(300))
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.memory_id, id2);
    }
}

// ── (i) Migration v5 -> v6 ─────────────────────────────────────────────────

/// Hand-written snapshot of schema v5 (all migrations through v5 applied),
/// mirroring the pattern of the v3/v4 migration tests in `domain.rs`.
const V5_SCHEMA_SQL: &str = r"
CREATE TABLE journal (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL,
    operation_id TEXT,
    thread_id TEXT,
    turn_id TEXT,
    sequence INTEGER NOT NULL,
    occurred_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    body BLOB NOT NULL
);
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
    event_id TEXT NOT NULL,
    thread_id TEXT,
    turn_id TEXT,
    operation_id TEXT,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    occurred_at INTEGER NOT NULL
);
CREATE TABLE agent_profile (
    agent_profile_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    preferred_harness TEXT NOT NULL,
    memory_mode TEXT NOT NULL,
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
CREATE TABLE project_ref (
    project_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE thread (
    thread_id TEXT PRIMARY KEY,
    agent_profile_id TEXT NOT NULL,
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    project_id TEXT,
    event_count INTEGER NOT NULL,
    first_event_seq INTEGER,
    last_event_seq INTEGER,
    last_event_id TEXT,
    last_event_kind TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE turn (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    operation_id TEXT,
    state TEXT NOT NULL,
    delivery TEXT NOT NULL,
    event_count INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER
);
CREATE TABLE permission (
    event_id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    description TEXT NOT NULL,
    decision TEXT NOT NULL,
    requested_at INTEGER NOT NULL,
    decided_at INTEGER
);
CREATE TABLE domain_projection_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    journal_max_seq INTEGER NOT NULL,
    projection_version INTEGER NOT NULL,
    rebuilt_at INTEGER NOT NULL,
    projection_digest TEXT NOT NULL DEFAULT ''
);
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
CREATE TABLE thread_session_binding (
    thread_id TEXT PRIMARY KEY,
    harness_binding_id TEXT NOT NULL,
    opaque_session_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL
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
PRAGMA user_version = 5;
";

#[test]
fn migration_v5_to_v6_creates_memory_tables_and_preserves_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("migration_test.db");

    // Initialize database up to schema v5 manually: create the v5 schema
    // snapshot, seed one journal row, then let Store::open migrate to v6.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("wal");
        conn.execute_batch(V5_SCHEMA_SQL).expect("apply v5 schema");
        conn.execute(
            "INSERT INTO journal (event_id, sequence, occurred_at, kind, body)
             VALUES ('evt_v5seed0000000000000000000001', 1, 1000, 'ThreadStarted', x'00')",
            (),
        )
        .expect("seed journal row");
        assert_eq!(
            conn.query_row("SELECT user_version FROM pragma_user_version", [], |r| {
                r.get::<_, i64>(0)
            })
            .expect("user_version"),
            5
        );
    }

    // Now open with Store::open, which should auto-migrate from v5 to latest (v8)
    {
        let mut store = Store::open(&db_path).expect("open and migrate to v7");
        assert_eq!(store.schema_version().expect("schema_version"), 8);

        // Verify we can write and search memories on the migrated database
        let draft = fixture_draft("Memory created after v5 to v6 migration");
        let memory = store.create_memory(&draft, t(100)).expect("create memory");
        assert_eq!(memory.state, MemoryState::Confirmed);

        let q = SearchQuery::try_from("migration").expect("query");
        let hits = store
            .search_memories(&q, None, MemorySearchLimit::default_limit(), t(200))
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.memory_id, memory.memory_id);
    }
}

/// Complete snapshot of Schema v7 (P2.2, ADR 0018) before v8 trigram upgrade.
const V7_SCHEMA_SQL: &str = r"
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

CREATE TABLE identity_document (
    identity_document_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX identity_document_kind_id ON identity_document(kind, identity_document_id);
CREATE TABLE context_snapshot (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    memory_mode TEXT NOT NULL,
    passthrough INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
) STRICT;
CREATE INDEX context_snapshot_thread_created
    ON context_snapshot(thread_id, created_at, turn_id);
PRAGMA user_version = 7;

";

#[test]
fn migration_v7_to_v8_upgrades_memory_fts_to_trigram_and_preserves_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("v7_to_v8.db");

    // 1. Manually initialize raw SQLite with genuine schema v7
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("wal");
        conn.execute_batch(V7_SCHEMA_SQL).expect("apply v7 schema");

        // Seed domain agent_profile
        conn.execute(
            "INSERT INTO agent_profile (agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at)
             VALUES ('agp_v7test0000000001', 'Agent v7', 'acp', 'long_term', 1000, 1000)",
            [],
        ).expect("seed agent profile");

        // Seed confirmed memory in v7
        conn.execute(
            "INSERT INTO memory (
                memory_id, content, scope_kind, scope_target, kind, state,
                confidence, sensitivity, source, explicit,
                provenance_thread_id, provenance_turn_id, excerpt,
                created_at, updated_at, expires_at, superseded_by
            ) VALUES (
                'mem_v7seed0000000001', '用户偏好喝乌龙茶，尤其是冻顶乌龙茶，不加糖。', 'global', NULL,
                'preference', 'confirmed', 95, 'normal', 'explicit', 1,
                NULL, NULL, NULL, 1000, 1000, NULL, NULL
            )",
            [],
        ).expect("seed memory");

        // Also seed domain journal event for projection consistency
        conn.execute(
            "INSERT INTO domain_journal (event_id, kind, payload, occurred_at)
             VALUES (
               'evt_v7seed000000000000000001',
               'memory.confirmed',
               CAST(?1 AS BLOB),
               1000
             )",
            [r#"{"memory_id":"mem_v7seed0000000001","content":"用户偏好喝乌龙茶，尤其是冻顶乌龙茶，不加糖。","scope_kind":"global","scope_target":null,"kind":"preference","state":"confirmed","confidence":95,"sensitivity":"normal","source":"explicit","explicit":true,"provenance_thread_id":null,"provenance_turn_id":null,"excerpt":null,"expires_at":null}"#],
        ).expect("seed domain journal");

        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read version");
        assert_eq!(v, 7);
    }

    // 2. Open with Store::open: must migrate from v7 to v8
    {
        let store = Store::open(&db_path).expect("open and migrate to v8");
        assert_eq!(store.schema_version().expect("schema_version"), 8);

        // Verify that Chinese substring search (which previously failed on v6/v7) now succeeds!
        let q = SearchQuery::try_from("乌龙茶").expect("query");
        let hits = store
            .search_memories(&q, None, MemorySearchLimit::default_limit(), t(2000))
            .expect("search");
        assert_eq!(
            hits.len(),
            1,
            "Migrated memory must be searchable with Chinese query in v8 trigram"
        );
        assert_eq!(hits[0].record.memory_id.as_str(), "mem_v7seed0000000001");

        // Verify 2-char short Chinese query also works via hybrid search!
        let q2 = SearchQuery::try_from("乌龙").expect("query 2");
        let hits2 = store
            .search_memories(&q2, None, MemorySearchLimit::default_limit(), t(2000))
            .expect("search short");
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].record.memory_id.as_str(), "mem_v7seed0000000001");
    }
}
