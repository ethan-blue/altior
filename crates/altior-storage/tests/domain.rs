//! P1.1 domain persistence acceptance evidence (ADR 0013).
//!
//! Deterministic throughout: fixture timestamps, in-memory SQLite.
//! No sleeps, no network, no wall clock.

use altior_domain::{
    AGENT_PROFILE_LIST_LIMIT_MAX, AcpHarnessBinding, AgentProfile, AgentProfileCursor,
    AgentProfileId, AgentProfileListLimit, BoundedLabel, BoundedPath, DisplayName, DomainEvent,
    DomainEventKind, EntityError, EventId, EventPayload, HARNESS_BINDING_LIST_LIMIT_MAX,
    HISTORY_LIMIT_MAX, HarnessArg, HarnessBindingCursor, HarnessBindingId, HarnessBindingListLimit,
    HarnessEnvKey, HarnessKind, HarnessSecretRef, HistoryLimit, MemoryMode, OperationId,
    PERMISSION_LIST_LIMIT_MAX, PROJECT_REF_LIST_LIMIT_MAX, PermissionCursor, PermissionDecision,
    PermissionKind, PermissionListLimit, ProjectId, ProjectRef, ProjectRefCursor,
    ProjectRefListLimit, SearchQuery, THREAD_LIST_LIMIT_MAX, TURN_LIST_LIMIT_MAX, ThreadCursor,
    ThreadId, ThreadListLimit, ThreadState, TurnCursor, TurnId, TurnListLimit, UnixMillis,
};
use altior_storage::{AppendOutcome, JournalLimit, StorageError, Store, domain_projection_digest};

/// Fixture epoch for `occurred_at` values (2023-11-14T22:13:20Z).
const BASE_MILLIS: u64 = 1_700_000_000_000;

/// Pads an identifier body to the domain minimum of 16 characters.
fn id(prefix: &str, body: &str) -> String {
    let padded = format!("{body:0>16}");
    format!("{prefix}{padded}")
}

fn event_id(n: u64) -> EventId {
    EventId::try_from(id("evt_", &n.to_string())).expect("valid event id")
}

fn thread_id(name: &str) -> ThreadId {
    ThreadId::try_from(id("thr_", name)).expect("valid thread id")
}

fn turn_id(name: &str) -> TurnId {
    TurnId::try_from(id("trn_", name)).expect("valid turn id")
}

fn agent_id(name: &str) -> AgentProfileId {
    AgentProfileId::try_from(id("agp_", name)).expect("valid agent id")
}

fn binding_id(name: &str) -> HarnessBindingId {
    HarnessBindingId::try_from(id("hsb_", name)).expect("valid binding id")
}

fn project_id(name: &str) -> ProjectId {
    ProjectId::try_from(id("prj_", name)).expect("valid project id")
}

fn operation_id(name: &str) -> OperationId {
    OperationId::try_from(id("op_", name)).expect("valid operation id")
}

/// Creates a JSON payload from key-value pairs.
fn json_payload(pairs: &[(&str, &str)]) -> EventPayload {
    let mut obj = serde_json::Map::new();
    for (k, v) in pairs {
        obj.insert((*k).to_owned(), serde_json::Value::String((*v).to_owned()));
    }
    let bytes = serde_json::to_vec(&serde_json::Value::Object(obj)).expect("json");
    EventPayload::try_from(bytes).expect("bounded payload")
}

fn empty_payload() -> EventPayload {
    EventPayload::try_from(b"{}".to_vec()).expect("bounded payload")
}

/// Creates a domain event.
fn domain_event(
    n: u64,
    thread: Option<&str>,
    turn: Option<&str>,
    kind: DomainEventKind,
    payload: EventPayload,
) -> DomainEvent {
    DomainEvent {
        event_id: event_id(n),
        thread_id: thread.map(thread_id),
        turn_id: turn.map(turn_id),
        operation_id: None,
        kind,
        payload,
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + n),
    }
}

/// Creates an `AgentProfile` fixture.
fn fixture_agent_profile(name: &str) -> AgentProfile {
    let aid = if name.starts_with("agp_") {
        AgentProfileId::try_from(name.to_owned()).expect("valid agent id")
    } else {
        agent_id(name)
    };
    AgentProfile {
        id: aid,
        display_name: DisplayName::try_from(format!("Agent {name}")).expect("valid display name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::LongTerm,
        created_at: UnixMillis::from_millis(BASE_MILLIS),
        updated_at: UnixMillis::from_millis(BASE_MILLIS),
    }
}

/// Ensures an `AgentProfile` exists in the store, creating it if absent.
fn ensure_agent_profile(store: &mut Store, name: &str) -> AgentProfileId {
    let profile = fixture_agent_profile(name);
    let id = profile.id.clone();
    if store
        .agent_profile_by_id(&id)
        .expect("query agent profile")
        .is_none()
    {
        store
            .create_agent_profile(&profile)
            .expect("ensure agent profile");
    }
    id
}

/// Creates a `ProjectRef` fixture.
fn fixture_project_ref(name: &str) -> ProjectRef {
    let pid = if name.starts_with("prj_") {
        ProjectId::try_from(name.to_owned()).expect("valid project id")
    } else {
        project_id(name)
    };
    ProjectRef {
        id: pid,
        label: BoundedLabel::try_from(format!("Project {name}")).expect("valid label"),
        path: BoundedPath::try_from(format!("/workspace/{name}")).expect("valid path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS),
    }
}

/// Ensures a `ProjectRef` exists in the store, creating it if absent.
fn ensure_project_ref(store: &mut Store, name: &str) -> ProjectId {
    let project = fixture_project_ref(name);
    let id = project.id.clone();
    if store
        .project_ref_by_id(&id)
        .expect("query project ref")
        .is_none()
    {
        store
            .create_project_ref(&project)
            .expect("ensure project ref");
    }
    id
}

// ── Schema and migration ───────────────────────────────────────────

#[test]
fn v3_migration_adds_domain_tables() {
    let store = Store::open_in_memory().expect("store");
    assert_eq!(store.schema_version().expect("version"), 5);
    assert_eq!(store.domain_journal_len().expect("count"), 0);
}

#[test]
fn v3_migration_preserves_v1_journal() {
    use altior_protocol::{EventBody, EventEnvelope, KnownEvent, ProtocolVersion, Sequence};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");

    // Write via the old protocol journal path.
    {
        let mut store = Store::open(&path).expect("open");
        let env = EventEnvelope {
            protocol_version: ProtocolVersion::try_new(1).expect("v1"),
            event_id: event_id(1),
            operation_id: None,
            thread_id: Some(thread_id("alpha")),
            turn_id: Some(turn_id("alpha0turn000")),
            sequence: Sequence::try_new(1).expect("seq"),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 1),
            body: EventBody::Known(KnownEvent::TurnStarted),
        };
        store.append_event(&env).expect("append");
    }

    // Reopen — v5 migration applied, v1 data intact.
    let store = Store::open(&path).expect("reopen");
    assert_eq!(store.schema_version().expect("version"), 5);
    assert_eq!(store.journal_len().expect("v1 count"), 1);
    assert_eq!(store.domain_journal_len().expect("domain count"), 0);
}

#[test]
fn newer_schema_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    drop(Store::open(&path).expect("create"));

    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.pragma_update(None, "user_version", 99).expect("stamp");
    drop(raw);

    let error = Store::open(&path).expect_err("must refuse");
    assert!(matches!(
        error,
        StorageError::SchemaTooNew {
            found: 99,
            supported: 5
        }
    ));
}

// ── Domain event append and idempotency ────────────────────────────

#[test]
fn domain_event_append_and_idempotency() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = ensure_agent_profile(&mut store, "claude");

    let event = domain_event(
        1,
        Some("alpha"),
        None,
        DomainEventKind::ThreadCreated,
        json_payload(&[
            ("agent_profile_id", agp_id.as_str()),
            ("title", "Test Thread"),
        ]),
    );

    // First append.
    let result = store.append_domain_event(&event).expect("append");
    assert_eq!(result, AppendOutcome::Appended { seq: 1 });

    // Idempotent duplicate.
    let result = store.append_domain_event(&event).expect("dup");
    assert_eq!(result, AppendOutcome::Duplicate { seq: 1 });

    assert_eq!(store.domain_journal_len().expect("count"), 1);
}

#[test]
fn domain_event_collision_fails_closed() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = ensure_agent_profile(&mut store, "fixture");

    let event1 = domain_event(
        1,
        Some("alpha"),
        None,
        DomainEventKind::ThreadCreated,
        json_payload(&[("agent_profile_id", agp_id.as_str()), ("title", "A")]),
    );
    store.append_domain_event(&event1).expect("append");

    // Same event_id, different payload.
    let event2 = domain_event(
        1,
        Some("alpha"),
        None,
        DomainEventKind::ThreadCreated,
        json_payload(&[("agent_profile_id", agp_id.as_str()), ("title", "B")]),
    );
    let error = store.append_domain_event(&event2).unwrap_err();
    assert!(matches!(error, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("count"), 1);
}

#[test]
fn domain_journal_is_append_only_at_db_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    {
        let mut store = Store::open(&path).expect("store");
        let agp_id = ensure_agent_profile(&mut store, "fixture");
        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp_id.as_str()), ("title", "hello")]),
            ))
            .expect("append");
    }

    let raw = rusqlite::Connection::open(&path).expect("raw open");
    let update = raw.execute("UPDATE domain_journal SET kind = 'tampered'", []);
    assert!(update.is_err(), "UPDATE must be refused");
    assert!(update.unwrap_err().to_string().contains("append-only"));

    let delete = raw.execute("DELETE FROM domain_journal", []);
    assert!(delete.is_err(), "DELETE must be refused");
    assert!(delete.unwrap_err().to_string().contains("append-only"));
}

// ── Thread projection ──────────────────────────────────────────────

#[test]
fn thread_created_projects_correctly() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "My Thread")]),
        ))
        .expect("append");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.thread_id, tid.as_str());
    assert_eq!(thread.agent_profile_id, agp.as_str());
    assert_eq!(thread.title, "My Thread");
    assert_eq!(thread.state, "open");
    assert_eq!(thread.event_count, 1);
}

#[test]
fn thread_title_change_updates_projection() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Original")]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            None,
            DomainEventKind::ThreadTitleChanged,
            json_payload(&[("title", "Renamed")]),
        ))
        .expect("rename");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.title, "Renamed");
    assert_eq!(thread.event_count, 2);
}

#[test]
fn thread_state_change_projects() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            None,
            DomainEventKind::ThreadStateChanged,
            json_payload(&[("state", "pinned")]),
        ))
        .expect("pin");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.state, "pinned");

    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            None,
            DomainEventKind::ThreadStateChanged,
            json_payload(&[("state", "archived")]),
        ))
        .expect("archive");

    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.state, "archived");
}

// ── Turn lifecycle ─────────────────────────────────────────────────

#[test]
fn turn_lifecycle_projects() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    // Create thread.
    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread");

    // Start turn.
    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start turn");

    let tid = thread_id("alpha");
    let turn_limit = TurnListLimit::try_new(10).expect("limit");
    let turns = store
        .turns_for_thread(&tid, None, turn_limit)
        .expect("query");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, "active");
    assert!(turns[0].ended_at.is_none());

    // Message delta.
    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::MessageDelta,
            json_payload(&[("text", "hello world")]),
        ))
        .expect("delta");

    // Complete turn.
    store
        .append_domain_event(&domain_event(
            4,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnCompleted,
            empty_payload(),
        ))
        .expect("complete");

    let turns = store
        .turns_for_thread(&tid, None, turn_limit)
        .expect("query");
    assert_eq!(turns[0].state, "completed");
    assert_eq!(turns[0].delivery, "confirmed");
    assert!(turns[0].ended_at.is_some());
    assert_eq!(turns[0].event_count, 3);
}

#[test]
fn turn_cancelled_and_failed_states() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create");

    // Cancelled turn.
    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("cancel0turn0000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start");
    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("cancel0turn0000"),
            DomainEventKind::TurnCancelled,
            empty_payload(),
        ))
        .expect("cancel");

    // Failed turn.
    store
        .append_domain_event(&domain_event(
            4,
            Some("alpha"),
            Some("failed0turn0000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start");
    store
        .append_domain_event(&domain_event(
            5,
            Some("alpha"),
            Some("failed0turn0000"),
            DomainEventKind::TurnFailed,
            empty_payload(),
        ))
        .expect("fail");

    let tid = thread_id("alpha");
    let turns = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("query");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].state, "cancelled");
    assert_eq!(turns[1].state, "failed");
}

// ── Permission projection ──────────────────────────────────────────

#[test]
fn permission_request_and_decision() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start");

    // Request permission.
    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::PermissionRequested,
            json_payload(&[
                ("permission_kind", "execute"),
                ("description", "Run npm test"),
            ]),
        ))
        .expect("request");

    // Decide permission.
    store
        .append_domain_event(&domain_event(
            4,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::PermissionDecided,
            json_payload(&[
                ("permission_event_id", &id("evt_", "3")),
                ("decision", "approved"),
            ]),
        ))
        .expect("decide");

    // Verify via domain journal.
    assert_eq!(store.domain_journal_len().expect("count"), 4);
}

// ── Bounded thread list ────────────────────────────────────────────

#[test]

fn thread_list_bounded_and_sorted() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    // Create 5 threads with different update times.

    for i in 1..=5u64 {
        let name = format!("thread{i:010}00000");

        store
            .append_domain_event(&domain_event(
                i,
                Some(&name),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[
                    ("agent_profile_id", agp.as_str()),
                    ("title", &format!("Thread {i}")),
                ]),
            ))
            .expect("create");
    }

    // Bounded list with limit 3.

    let limit = ThreadListLimit::try_new(3).expect("limit");

    let page1 = store.thread_list(None, None, limit).expect("list");

    assert_eq!(page1.len(), 3);

    // Most recent first.

    assert!(page1[0].updated_at >= page1[1].updated_at);

    assert!(page1[1].updated_at >= page1[2].updated_at);

    // Cursor-based pagination.

    let last_updated = page1[2].updated_at;

    let page2 = store
        .thread_list(
            None,
            Some(&ThreadCursor {
                updated_at: UnixMillis::from_millis(
                    u64::try_from(last_updated).expect("non-negative"),
                ),

                thread_id: ThreadId::try_from(page1[2].thread_id.clone()).expect("valid thread id"),
            }),
            limit,
        )
        .expect("page2");

    assert_eq!(page2.len(), 2);

    // No overlap.

    let page1_ids: Vec<&str> = page1.iter().map(|t| t.thread_id.as_str()).collect();

    let page2_ids: Vec<&str> = page2.iter().map(|t| t.thread_id.as_str()).collect();

    for id in &page2_ids {
        assert!(!page1_ids.contains(id), "overlap: {id}");
    }
}

#[test]

fn thread_list_filters_by_state() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("open0thread0000"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Open Thread")]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("archived0thread"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Will Archive"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            3,
            Some("archived0thread"),
            None,
            DomainEventKind::ThreadStateChanged,
            json_payload(&[("state", "archived")]),
        ))
        .expect("archive");

    let limit = ThreadListLimit::try_new(10).expect("limit");

    let open = store
        .thread_list(Some(ThreadState::Open), None, limit)
        .expect("open");

    assert_eq!(open.len(), 1);

    assert_eq!(open[0].state, "open");

    let archived = store
        .thread_list(Some(ThreadState::Archived), None, limit)
        .expect("archived");

    assert_eq!(archived.len(), 1);

    assert_eq!(archived[0].state, "archived");
}

// ── Thread history ─────────────────────────────────────────────────

#[test]

fn thread_history_paginated_and_stable() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start");

    // Add 10 events.

    for i in 3..=12u64 {
        store
            .append_domain_event(&domain_event(
                i,
                Some("alpha"),
                Some("alpha0turn000"),
                DomainEventKind::MessageDelta,
                json_payload(&[("text", &format!("message {i}"))]),
            ))
            .expect("append");
    }

    let tid = thread_id("alpha");

    let limit = HistoryLimit::try_new(5).expect("limit");

    let page1 = store.thread_history(&tid, 0, limit).expect("page1");

    assert_eq!(page1.len(), 5);

    // Stable ordering.

    assert!(page1.windows(2).all(|pair| pair[0].seq < pair[1].seq));

    let page2 = store
        .thread_history(&tid, page1[4].seq, limit)
        .expect("page2");

    assert_eq!(page2.len(), 5);

    assert!(page2[0].seq > page1[4].seq);

    let page3 = store
        .thread_history(&tid, page2[4].seq, limit)
        .expect("page3");

    assert_eq!(page3.len(), 2);
}

// ── Search ─────────────────────────────────────────────────────────

#[test]

fn search_threads_by_title() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("rust0thread0000"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Rust async debugging"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("python0thread00"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Python data pipeline"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            3,
            Some("rust0async0thre"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Async Rust tokio tips"),
            ]),
        ))
        .expect("create");

    let limit = ThreadListLimit::try_new(10).expect("limit");

    let results = store
        .search_threads(&SearchQuery::try_from("Rust").expect("query"), None, limit)
        .expect("search");

    assert_eq!(results.len(), 2, "Two threads match 'Rust'");

    let results = store
        .search_threads(
            &SearchQuery::try_from("Python").expect("query"),
            None,
            limit,
        )
        .expect("search");

    assert_eq!(results.len(), 1);

    let results = store
        .search_threads(
            &SearchQuery::try_from("nonexistent").expect("query"),
            None,
            limit,
        )
        .expect("search");

    assert_eq!(results.len(), 0);
}

// ── Rebuild ────────────────────────────────────────────────────────

#[test]

fn domain_rebuild_reproduces_projections() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    // Build a realistic event sequence.

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Alpha Thread"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start");

    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::MessageDelta,
            json_payload(&[("text", "hello")]),
        ))
        .expect("delta");

    store
        .append_domain_event(&domain_event(
            4,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnCompleted,
            empty_payload(),
        ))
        .expect("complete");

    store
        .append_domain_event(&domain_event(
            5,
            Some("beta"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Beta Thread")]),
        ))
        .expect("create beta");

    // Capture incremental state.

    let tid_alpha = thread_id("alpha");

    let tid_beta = thread_id("beta");

    let turn_limit = TurnListLimit::try_new(10).expect("limit");

    let before_alpha = store.thread_by_id(&tid_alpha).expect("q").expect("exists");

    let before_beta = store.thread_by_id(&tid_beta).expect("q").expect("exists");

    let before_turns = store
        .turns_for_thread(&tid_alpha, None, turn_limit)
        .expect("turns");

    // Force rebuild.

    let replayed = store.rebuild_domain_projections().expect("rebuild");

    assert_eq!(replayed, 5);

    // Verify identical projections.

    let after_alpha = store.thread_by_id(&tid_alpha).expect("q").expect("exists");

    let after_beta = store.thread_by_id(&tid_beta).expect("q").expect("exists");

    let after_turns = store
        .turns_for_thread(&tid_alpha, None, turn_limit)
        .expect("turns");

    assert_eq!(before_alpha, after_alpha);

    assert_eq!(before_beta, after_beta);

    assert_eq!(before_turns, after_turns);
}

#[test]

fn domain_rebuild_is_stable_across_multiple_calls() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Test")]),
        ))
        .expect("create");

    let tid = thread_id("alpha");

    let snapshot1 = store.thread_by_id(&tid).expect("q").expect("exists");

    store.rebuild_domain_projections().expect("rebuild 1");

    let snapshot2 = store.thread_by_id(&tid).expect("q").expect("exists");

    store.rebuild_domain_projections().expect("rebuild 2");

    let snapshot3 = store.thread_by_id(&tid).expect("q").expect("exists");

    assert_eq!(snapshot1, snapshot2);

    assert_eq!(snapshot2, snapshot3);
}

// ── Reopen and recovery ───────────────────────────────────────────

#[test]

fn reopen_heals_missing_domain_projections() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Thread A")]),
            ))
            .expect("create");

        store
            .append_domain_event(&domain_event(
                2,
                Some("alpha"),
                Some("alpha0turn000"),
                DomainEventKind::TurnStarted,
                empty_payload(),
            ))
            .expect("start");
    }

    // Damage: wipe projections.

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute("DELETE FROM thread", []).expect("wipe");

    raw.execute("DELETE FROM turn", []).expect("wipe");

    raw.execute("DELETE FROM domain_projection_state", [])
        .expect("wipe");

    drop(raw);

    // Reopen heals.

    let store = Store::open(&path).expect("reopen");

    let tid = thread_id("alpha");

    let thread = store.thread_by_id(&tid).expect("q").expect("healed");

    assert_eq!(thread.title, "Thread A");

    assert_eq!(thread.event_count, 2);

    let turns = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("turns");

    assert_eq!(turns.len(), 1);
}

#[test]

fn reopen_heals_stale_domain_marker() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        for i in 1..=3u64 {
            let name = format!("thread{i:010}00000");

            store
                .append_domain_event(&domain_event(
                    i,
                    Some(&name),
                    None,
                    DomainEventKind::ThreadCreated,
                    json_payload(&[
                        ("agent_profile_id", agp.as_str()),
                        ("title", &format!("T{i}")),
                    ]),
                ))
                .expect("create");
        }
    }

    // Simulate stale marker.

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute("UPDATE domain_projection_state SET journal_max_seq = 1", [])
        .expect("age");

    drop(raw);

    let store = Store::open(&path).expect("reopen");

    let limit = ThreadListLimit::try_new(10).expect("limit");

    let threads = store.thread_list(None, None, limit).expect("list");

    assert_eq!(threads.len(), 3, "stale marker healed by rebuild");
}

#[test]

fn stale_domain_fold_version_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
            ))
            .expect("create");
    }

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute("DELETE FROM thread", []).expect("damage");

    raw.execute(
        "UPDATE domain_projection_state SET projection_version = 0",
        [],
    )
    .expect("age fold version");

    drop(raw);

    let store = Store::open(&path).expect("reopen");

    let tid = thread_id("alpha");

    let thread = store.thread_by_id(&tid).expect("q").expect("rebuilt");

    assert_eq!(thread.title, "T");
}

#[test]

fn marker_unchanged_delete_thread_projection_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[
                    ("agent_profile_id", agp.as_str()),
                    ("title", "Thread Alpha"),
                ]),
            ))
            .expect("create");
    }

    // Capture marker state before tampering

    let raw = rusqlite::Connection::open(&path).expect("raw");

    let (marker_seq, marker_ver, marker_digest): (i64, i64, String) = raw

        .query_row(

            "SELECT journal_max_seq, projection_version, projection_digest FROM domain_projection_state WHERE id = 1",

            [],

            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),

        )

        .expect("marker");

    // Damage: delete thread projection only, leave marker untouched

    raw.execute("DELETE FROM thread", [])
        .expect("delete thread");

    drop(raw);

    // Reopen must detect drift via digest and rebuild

    let store = Store::open(&path).expect("reopen");

    let tid = thread_id("alpha");

    let thread = store.thread_by_id(&tid).expect("q").expect("rebuilt");

    assert_eq!(thread.title, "Thread Alpha");

    assert_eq!(thread.event_count, 1);

    // Verify marker is current and valid

    let raw = rusqlite::Connection::open(&path).expect("raw");

    let (new_seq, new_ver, new_digest): (i64, i64, String) = raw

        .query_row(

            "SELECT journal_max_seq, projection_version, projection_digest FROM domain_projection_state WHERE id = 1",

            [],

            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),

        )

        .expect("marker");

    assert_eq!(new_seq, marker_seq);

    assert_eq!(new_ver, marker_ver);

    assert_eq!(new_digest, marker_digest);
}

#[test]

fn marker_unchanged_tamper_turn_projection_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Thread A")]),
            ))
            .expect("create");

        store
            .append_domain_event(&domain_event(
                2,
                Some("alpha"),
                Some("turn001"),
                DomainEventKind::TurnStarted,
                empty_payload(),
            ))
            .expect("turn");
    }

    // Damage: tamper turn projection only (set state to 'failed' and delivery to 'rejected')

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute(

        "UPDATE turn SET state = 'failed', delivery = 'rejected', event_count = 99 WHERE turn_id = 'trn_turn00100000000'",

        [],

    )

    .expect("tamper turn");

    drop(raw);

    // Reopen must detect drift and rebuild to active / absent / event_count 1

    let store = Store::open(&path).expect("reopen");

    let tid = thread_id("alpha");

    let turns = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("turns");

    assert_eq!(turns.len(), 1);

    assert_eq!(turns[0].state, "active");

    assert_eq!(turns[0].delivery, "absent");

    assert_eq!(turns[0].event_count, 1);
}

#[test]

fn marker_unchanged_tamper_permission_projection_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    let perm_id = event_id(3);

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Thread A")]),
            ))
            .expect("create");

        store
            .append_domain_event(&domain_event(
                2,
                Some("alpha"),
                Some("turn001"),
                DomainEventKind::TurnStarted,
                empty_payload(),
            ))
            .expect("turn");

        store
            .append_domain_event(&DomainEvent {
                event_id: perm_id.clone(),

                thread_id: Some(thread_id("alpha")),

                turn_id: Some(turn_id("turn001")),

                operation_id: None,

                kind: DomainEventKind::PermissionRequested,

                payload: json_payload(&[
                    ("permission_kind", "execute"),
                    ("description", "Run command"),
                ]),

                occurred_at: UnixMillis::from_millis(BASE_MILLIS + 3),
            })
            .expect("perm");
    }

    // Damage: tamper permission projection only (falsely mark as approved)

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute(
        "UPDATE permission SET decision = 'approved' WHERE event_id = ?1",
        [perm_id.as_str()],
    )
    .expect("tamper perm");

    drop(raw);

    // Reopen must detect drift and rebuild to pending

    let store = Store::open(&path).expect("reopen");

    let perm = store
        .permission_by_event_id(&perm_id)
        .expect("get")
        .expect("found");

    assert_eq!(perm.decision, PermissionDecision::Pending);
}

#[test]
fn marker_unchanged_tamper_thread_attributes_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");
        let agp = agent_id("claude");
        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[
                    ("agent_profile_id", agp.as_str()),
                    ("title", "SearchableUniqueTitle"),
                ]),
            ))
            .expect("create");
    }

    // Damage: tamper thread title directly in SQLite without touching domain_projection_state
    let raw = rusqlite::Connection::open(&path).expect("raw");
    raw.execute(
        "UPDATE thread SET title = 'CorruptedTitle' WHERE thread_id = 'thr_alpha0000000000'",
        [],
    )
    .expect("tamper title");
    drop(raw);

    // Reopen must detect projection digest drift and rebuild back to original journal state
    let store = Store::open(&path).expect("reopen");
    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("found");
    assert_eq!(thread.title, "SearchableUniqueTitle");
}

#[test]
fn marker_unchanged_fts_index_drift_triggers_official_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    let tid = thread_id("ftsdrift");

    {
        let mut store = Store::open(&path).expect("store");
        store
            .append_domain_event(&DomainEvent {
                event_id: event_id(1),
                thread_id: Some(tid.clone()),
                turn_id: None,
                operation_id: None,
                kind: DomainEventKind::ThreadCreated,
                payload: json_payload(&[
                    ("agent_profile_id", agent_id("claude").as_str()),
                    ("title", "UniqueFtsRecoveryToken"),
                ]),
                occurred_at: UnixMillis::from_millis(BASE_MILLIS + 1),
            })
            .expect("create thread");
        let hits = store
            .search_threads(
                &SearchQuery::try_from("UniqueFtsRecoveryToken").expect("query"),
                None,
                ThreadListLimit::try_new(10).expect("limit"),
            )
            .expect("search before drift");
        assert_eq!(hits.len(), 1);
    }

    let raw = rusqlite::Connection::open(&path).expect("raw");
    raw.execute(
        "INSERT INTO thread_search(thread_search, rowid, thread_id, title)
         SELECT 'delete', rowid, thread_id, title FROM thread WHERE thread_id = ?1",
        [tid.as_str()],
    )
    .expect("remove only the FTS index entry");
    drop(raw);

    let store = Store::open(&path).expect("reopen and repair FTS");
    let hits = store
        .search_threads(
            &SearchQuery::try_from("UniqueFtsRecoveryToken").expect("query"),
            None,
            ThreadListLimit::try_new(10).expect("limit"),
        )
        .expect("search after repair");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id, tid.to_string());
}

#[test]
fn matching_reopen_does_not_unconditionally_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Thread A")]),
            ))
            .expect("create");
    }

    // Set sentinel rebuilt_at value in marker while preserving seq, version, and digest

    let sentinel_rebuilt_at: i64 = 777_888_999;

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute(
        "UPDATE domain_projection_state SET rebuilt_at = ?1 WHERE id = 1",
        [sentinel_rebuilt_at],
    )
    .expect("set sentinel");

    drop(raw);

    // Normal matching reopen: digest matches marker, so NO rebuild should occur

    let _store = Store::open(&path).expect("normal reopen");

    let raw = rusqlite::Connection::open(&path).expect("raw");

    let observed_rebuilt_at: i64 = raw
        .query_row(
            "SELECT rebuilt_at FROM domain_projection_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("get rebuilt_at");

    drop(raw);

    assert_eq!(
        observed_rebuilt_at, sentinel_rebuilt_at,
        "matching reopen must not unconditionally rebuild projections"
    );

    // Now introduce drift (delete thread row)

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute("DELETE FROM thread", []).expect("damage");

    drop(raw);

    // Reopen with drift: must rebuild and update rebuilt_at to the event's occurred_at

    let _store2 = Store::open(&path).expect("drift reopen");

    let raw = rusqlite::Connection::open(&path).expect("raw");

    let rebuilt_rebuilt_at: i64 = raw
        .query_row(
            "SELECT rebuilt_at FROM domain_projection_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("get rebuilt_at");

    drop(raw);

    assert_ne!(
        rebuilt_rebuilt_at, sentinel_rebuilt_at,
        "drift reopen must rebuild and refresh rebuilt_at"
    );

    assert_eq!(rebuilt_rebuilt_at, (BASE_MILLIS + 1).cast_signed());
}

#[test]

fn malformed_journal_durable_row_fails_closed_on_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");

    let path = dir.path().join("test.db");

    {
        let mut store = Store::open(&path).expect("store");

        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&domain_event(
                1,
                Some("alpha"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
            ))
            .expect("create");
    }

    // Corrupt journal: insert malformed payload directly bypassing API

    let raw = rusqlite::Connection::open(&path).expect("raw");

    raw.execute(

        "INSERT INTO domain_journal (event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at)

         VALUES ('evt_corrupt000000002', 'thr_alpha0000000000', NULL, NULL, 'thread.created', X'DEADBEEF', 1700000000002)",

        [],

    )

    .expect("insert corrupt");

    // Also trigger rebuild requirement by invalidating digest

    raw.execute(
        "UPDATE domain_projection_state SET projection_digest = 'stale_digest'",
        [],
    )
    .expect("invalidate digest");

    drop(raw);

    // Reopen must fail closed with typed RebuildInvariant error

    let err = Store::open(&path).unwrap_err();

    assert!(
        matches!(err, StorageError::RebuildInvariant { .. }),
        "corrupt durable row must typed fail closed: {err:?}"
    );
}

// ── Limit boundary tests ──────────────────────────────────────────

#[test]

fn thread_list_limit_boundary() {
    assert!(ThreadListLimit::try_new(1).is_ok());

    assert!(ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX).is_ok());

    assert!(matches!(
        ThreadListLimit::try_new(0),
        Err(EntityError::ThreadListLimitOutOfRange { .. })
    ));

    assert!(matches!(
        ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX + 1),
        Err(EntityError::ThreadListLimitOutOfRange { .. })
    ));
}

#[test]

fn history_limit_boundary() {
    assert!(HistoryLimit::try_new(1).is_ok());

    assert!(HistoryLimit::try_new(HISTORY_LIMIT_MAX).is_ok());

    assert!(matches!(
        HistoryLimit::try_new(0),
        Err(EntityError::HistoryLimitOutOfRange { .. })
    ));

    assert!(matches!(
        HistoryLimit::try_new(HISTORY_LIMIT_MAX + 1),
        Err(EntityError::HistoryLimitOutOfRange { .. })
    ));
}

#[test]

fn search_query_boundary() {
    assert!(SearchQuery::try_from("hello").is_ok());

    assert!(SearchQuery::try_from("  ").is_err());

    let long = "x".repeat(257);

    assert!(SearchQuery::try_from(long.as_str()).is_err());
}

// ── Domain journal records roundtrip ───────────────────────────────

#[test]

fn domain_journal_records_roundtrip_in_order() {
    let mut store = Store::open_in_memory().expect("store");

    for i in 1..=5u64 {
        store
            .append_domain_event(&domain_event(
                i,
                None,
                None,
                DomainEventKind::Other("audit.event".to_owned()),
                json_payload(&[("text", &format!("message {i}"))]),
            ))
            .expect("append");
    }

    let rows = store
        .domain_journal_records(0, JournalLimit::try_new(10).expect("limit"))
        .expect("read");

    assert_eq!(rows.len(), 5);

    assert!(rows.windows(2).all(|pair| pair[0].seq < pair[1].seq));

    // After-seq cursor.

    let tail = store
        .domain_journal_records(3, JournalLimit::try_new(10).expect("limit"))
        .expect("tail");

    assert_eq!(tail.len(), 2);

    assert_eq!(tail[0].seq, 4);
}

// ── Search after title change ──────────────────────────────────────

#[test]

fn search_finds_renamed_thread() {
    let mut store = Store::open_in_memory().expect("store");

    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Original Title"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            None,
            DomainEventKind::ThreadTitleChanged,
            json_payload(&[("title", "Completely Different Name")]),
        ))
        .expect("rename");

    let limit = ThreadListLimit::try_new(10).expect("limit");

    // Should NOT find by old title.
    let old = store
        .search_threads(&SearchQuery::try_from("Original").expect("q"), None, limit)
        .expect("search");
    assert_eq!(old.len(), 0);

    // Should find by new title.
    let new = store
        .search_threads(
            &SearchQuery::try_from("Completely Different").expect("q"),
            None,
            limit,
        )
        .expect("search");
    assert_eq!(new.len(), 1);
}

// ── Thread-less domain events ──────────────────────────────────────

#[test]
fn threadless_domain_event_journals_but_does_not_project() {
    let mut store = Store::open_in_memory().expect("store");

    store
        .append_domain_event(&domain_event(
            1,
            None,
            None,
            DomainEventKind::Other("system.startup".to_owned()),
            empty_payload(),
        ))
        .expect("append");

    assert_eq!(store.domain_journal_len().expect("count"), 1);
    let limit = ThreadListLimit::try_new(10).expect("limit");
    let threads = store.thread_list(None, None, limit).expect("list");
    assert!(threads.is_empty());
}

// ── AgentProfile CRUD & Pagination ─────────────────────────────────

#[test]
fn agent_profile_create_get_and_duplicate_fails_closed() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("claude");

    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Claude 3.5 Sonnet").expect("name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::LongTerm,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    store.create_agent_profile(&profile).expect("create");

    let fetched = store
        .agent_profile_by_id(&agp_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, profile);

    // Same-ID same-content create is idempotent
    store
        .create_agent_profile(&profile)
        .expect("same-id same-content idempotent");

    // Same-ID conflicting content duplicate create fails closed with typed error
    let mut conflicting = profile.clone();
    conflicting.display_name = DisplayName::try_from("Different Name").expect("name");
    let err = store.create_agent_profile(&conflicting).unwrap_err();
    assert!(matches!(
        err,
        StorageError::AgentProfileAlreadyExists { agent_profile_id } if agent_profile_id == agp_id.as_str()
    ));

    // Illegal timestamp order (updated_at < created_at) fails closed
    let mut invalid_ts = profile.clone();
    invalid_ts.id = agent_id("claude02");
    invalid_ts.created_at = UnixMillis::from_millis(BASE_MILLIS + 200);
    invalid_ts.updated_at = UnixMillis::from_millis(BASE_MILLIS + 100);
    let err_ts = store.create_agent_profile(&invalid_ts).unwrap_err();
    assert!(matches!(err_ts, StorageError::InvalidEntityData { .. }));
}

#[test]
fn agent_profile_update_and_upsert() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("claude");

    let mut profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Claude Initial").expect("name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Session,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    // Update non-existent fails
    let err = store.update_agent_profile(&profile).unwrap_err();
    assert!(matches!(
        err,
        StorageError::AgentProfileNotFound { agent_profile_id } if agent_profile_id == agp_id.as_str()
    ));

    // Upsert creates when non-existent
    store.upsert_agent_profile(&profile).expect("upsert create");
    let fetched = store.agent_profile_by_id(&agp_id).expect("get").unwrap();
    assert_eq!(fetched.display_name.as_str(), "Claude Initial");
    assert_eq!(fetched.memory_mode, MemoryMode::Session);

    // Update with conflicting immutable created_at fails closed
    let mut bad_update = profile.clone();
    bad_update.created_at = UnixMillis::from_millis(BASE_MILLIS + 50);
    let err_created_at = store.update_agent_profile(&bad_update).unwrap_err();
    assert!(matches!(
        err_created_at,
        StorageError::InvalidEntityData { .. }
    ));

    // Update with updated_at < created_at fails closed
    let mut bad_ts_update = profile.clone();
    bad_ts_update.updated_at = UnixMillis::from_millis(BASE_MILLIS + 50);
    let err_ts = store.update_agent_profile(&bad_ts_update).unwrap_err();
    assert!(matches!(err_ts, StorageError::InvalidEntityData { .. }));

    // Update existing modifies fields
    profile.display_name = DisplayName::try_from("Claude Updated").expect("name");
    profile.memory_mode = MemoryMode::LongTerm;
    profile.preferred_harness = HarnessKind::Native;
    profile.updated_at = UnixMillis::from_millis(BASE_MILLIS + 200);

    store.update_agent_profile(&profile).expect("update");
    let updated = store.agent_profile_by_id(&agp_id).expect("get").unwrap();
    assert_eq!(updated.display_name.as_str(), "Claude Updated");
    assert_eq!(updated.memory_mode, MemoryMode::LongTerm);
    assert_eq!(updated.preferred_harness, HarnessKind::Native);
    assert_eq!(
        updated.updated_at,
        UnixMillis::from_millis(BASE_MILLIS + 200)
    );

    // Upsert with conflicting immutable created_at fails closed
    let mut bad_upsert = profile.clone();
    bad_upsert.created_at = UnixMillis::from_millis(BASE_MILLIS + 50);
    let err_upsert_created = store.upsert_agent_profile(&bad_upsert).unwrap_err();
    assert!(matches!(
        err_upsert_created,
        StorageError::InvalidEntityData { .. }
    ));

    // Upsert updates when existing
    profile.display_name = DisplayName::try_from("Claude Upserted").expect("name");
    profile.updated_at = UnixMillis::from_millis(BASE_MILLIS + 300);
    store.upsert_agent_profile(&profile).expect("upsert update");
    let upserted = store.agent_profile_by_id(&agp_id).expect("get").unwrap();
    assert_eq!(upserted.display_name.as_str(), "Claude Upserted");
}

#[test]
fn agent_profile_pagination_with_timestamp_ties() {
    let mut store = Store::open_in_memory().expect("store");

    // Insert 5 profiles with identical updated_at to stress-test tie-breaking
    let mut ids = Vec::new();
    for i in 1..=5 {
        let id = agent_id(&format!("agent{i:02}"));
        ids.push(id.clone());
        let profile = AgentProfile {
            id,
            display_name: DisplayName::try_from(format!("Agent {i}").as_str()).expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 1000),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 1000),
        };
        store.create_agent_profile(&profile).expect("create");
    }

    let limit = AgentProfileListLimit::try_new(2).expect("limit");

    // Page 1
    let page1 = store.agent_profiles(None, limit).expect("page 1");
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].id, ids[0]);
    assert_eq!(page1[1].id, ids[1]);

    // Page 2
    let cursor1 = AgentProfileCursor {
        updated_at: page1[1].updated_at,
        agent_profile_id: page1[1].id.clone(),
    };
    let page2 = store.agent_profiles(Some(&cursor1), limit).expect("page 2");
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].id, ids[2]);
    assert_eq!(page2[1].id, ids[3]);

    // Page 3
    let cursor2 = AgentProfileCursor {
        updated_at: page2[1].updated_at,
        agent_profile_id: page2[1].id.clone(),
    };
    let page3 = store.agent_profiles(Some(&cursor2), limit).expect("page 3");
    assert_eq!(page3.len(), 1);
    assert_eq!(page3[0].id, ids[4]);

    // Page 4: empty
    let cursor3 = AgentProfileCursor {
        updated_at: page3[0].updated_at,
        agent_profile_id: page3[0].id.clone(),
    };
    let page4 = store.agent_profiles(Some(&cursor3), limit).expect("page 4");
    assert!(page4.is_empty());
}

#[test]
fn agent_profile_reopen_equivalence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("agent_test.db");

    let agp_id = agent_id("claude");
    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Claude 3.5").expect("name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::LongTerm,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    {
        let mut store = Store::open(&path).expect("open");
        store.create_agent_profile(&profile).expect("create");
    }

    let store = Store::open(&path).expect("reopen");
    let fetched = store
        .agent_profile_by_id(&agp_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, profile);
}

// ── AcpHarnessBinding CRUD & Validation ────────────────────────────

#[test]
fn harness_binding_requires_agent_profile_fk() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("nonexistent");
    let hnb_id = binding_id("binding01");

    let binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Local Claude").expect("label"),
        command: BoundedPath::try_from("/usr/local/bin/claude").expect("path"),
        args: vec![],
        env_keys: vec![],
        secret_refs: vec![],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    // Create fails if agent_profile does not exist
    let err = store.create_harness_binding(&binding).unwrap_err();
    assert!(matches!(
        err,
        StorageError::AgentProfileNotFound { agent_profile_id } if agent_profile_id == agp_id.as_str()
    ));

    // Upsert fails if agent_profile does not exist
    let err_upsert = store.upsert_harness_binding(&binding).unwrap_err();
    assert!(matches!(
        err_upsert,
        StorageError::AgentProfileNotFound { agent_profile_id } if agent_profile_id == agp_id.as_str()
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn harness_binding_create_duplicate_get_delete_and_list() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("claude");

    // First create agent profile
    store
        .create_agent_profile(&AgentProfile {
            id: agp_id.clone(),
            display_name: DisplayName::try_from("Claude").expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
        })
        .expect("create agent");

    let hnb_id1 = binding_id("binding01");
    let binding1 = AcpHarnessBinding {
        id: hnb_id1.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Claude Stdio").expect("label"),
        command: BoundedPath::try_from("/usr/bin/claude-stdio").expect("cmd"),
        args: vec![
            HarnessArg::try_from("--verbose").expect("arg"),
            HarnessArg::try_from("--mode=fast").expect("arg"),
        ],
        env_keys: vec![
            HarnessEnvKey::try_from("ANTHROPIC_API_KEY").expect("env"),
            HarnessEnvKey::try_from("RUST_LOG").expect("env"),
        ],
        secret_refs: vec![HarnessSecretRef::try_from("vault:secret:claude_key").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    store
        .create_harness_binding(&binding1)
        .expect("create binding");

    // Same-ID same-content create is idempotent
    store
        .create_harness_binding(&binding1)
        .expect("same-id same-content idempotent");

    // Same-ID conflicting content duplicate create fails closed (different label)
    let mut conflicting_binding = binding1.clone();
    conflicting_binding.label = DisplayName::try_from("Different Label").expect("label");
    let err_dup = store
        .create_harness_binding(&conflicting_binding)
        .unwrap_err();
    assert!(matches!(
        err_dup,
        StorageError::HarnessBindingAlreadyExists { harness_binding_id } if harness_binding_id == hnb_id1.as_str()
    ));

    // Same-ID conflicting content duplicate create fails closed (different args)
    let mut conflicting_args = binding1.clone();
    conflicting_args.args = vec![HarnessArg::try_from("--different").expect("arg")];
    let err_dup_args = store.create_harness_binding(&conflicting_args).unwrap_err();
    assert!(matches!(
        err_dup_args,
        StorageError::HarnessBindingAlreadyExists { harness_binding_id } if harness_binding_id == hnb_id1.as_str()
    ));

    // Same-ID conflicting content duplicate create fails closed (different env_keys)
    let mut conflicting_env = binding1.clone();
    conflicting_env.env_keys = vec![HarnessEnvKey::try_from("OTHER_KEY").expect("env")];
    let err_dup_env = store.create_harness_binding(&conflicting_env).unwrap_err();
    assert!(matches!(
        err_dup_env,
        StorageError::HarnessBindingAlreadyExists { harness_binding_id } if harness_binding_id == hnb_id1.as_str()
    ));

    // Same-ID conflicting content duplicate create fails closed (different secret_refs)
    let mut conflicting_sec = binding1.clone();
    conflicting_sec.secret_refs =
        vec![HarnessSecretRef::try_from("vault:other_secret").expect("sec")];
    let err_dup_sec = store.create_harness_binding(&conflicting_sec).unwrap_err();
    assert!(matches!(
        err_dup_sec,
        StorageError::HarnessBindingAlreadyExists { harness_binding_id } if harness_binding_id == hnb_id1.as_str()
    ));

    // Upsert with conflicting immutable created_at fails closed
    let mut bad_upsert_hnb = binding1.clone();
    bad_upsert_hnb.created_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err_hnb_created = store.upsert_harness_binding(&bad_upsert_hnb).unwrap_err();
    assert!(matches!(
        err_hnb_created,
        StorageError::InvalidEntityData { .. }
    ));

    // Upsert with conflicting immutable agent_profile_id fails closed
    let agp_id2 = agent_id("claude2");
    store
        .create_agent_profile(&AgentProfile {
            id: agp_id2.clone(),
            display_name: DisplayName::try_from("Claude 2").expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
        })
        .expect("create agent 2");
    let mut bad_upsert_agent = binding1.clone();
    bad_upsert_agent.agent_profile_id = agp_id2;
    let err_hnb_agent = store.upsert_harness_binding(&bad_upsert_agent).unwrap_err();
    assert!(matches!(
        err_hnb_agent,
        StorageError::InvalidEntityData { .. }
    ));

    // Get by ID
    let fetched = store
        .harness_binding_by_id(&hnb_id1)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, binding1);

    // Create second binding with same timestamp (tie-break test)
    let hnb_id2 = binding_id("binding02");
    let binding2 = AcpHarnessBinding {
        id: hnb_id2.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Claude Pipe").expect("label"),
        command: BoundedPath::try_from("/usr/bin/claude-pipe").expect("cmd"),
        args: vec![],
        env_keys: vec![],
        secret_refs: vec![],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store
        .create_harness_binding(&binding2)
        .expect("create binding 2");

    // List with pagination and tie-breaker
    let limit = HarnessBindingListLimit::try_new(1).expect("limit");
    let page1 = store
        .harness_bindings_for_agent(&agp_id, None, limit)
        .expect("page 1");
    assert_eq!(page1.len(), 1);
    assert_eq!(page1[0].id, hnb_id1);

    let cursor = HarnessBindingCursor {
        created_at: page1[0].created_at,
        harness_binding_id: page1[0].id.clone(),
    };
    let page2 = store
        .harness_bindings_for_agent(&agp_id, Some(&cursor), limit)
        .expect("page 2");
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].id, hnb_id2);

    // Delete
    assert!(store.delete_harness_binding(&hnb_id1).expect("delete"));
    assert!(
        !store
            .delete_harness_binding(&hnb_id1)
            .expect("delete again returns false")
    );
    assert!(
        store
            .harness_binding_by_id(&hnb_id1)
            .expect("get")
            .is_none()
    );
}

#[test]
fn harness_binding_reopen_equivalence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hnb_test.db");

    let agp_id = agent_id("claude");
    let hnb_id = binding_id("binding01");
    let binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Claude Stdio").expect("label"),
        command: BoundedPath::try_from("/usr/bin/claude-stdio").expect("cmd"),
        args: vec![HarnessArg::try_from("--flag").expect("arg")],
        env_keys: vec![HarnessEnvKey::try_from("FOO_ENV").expect("env")],
        secret_refs: vec![HarnessSecretRef::try_from("vault:sec1").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    {
        let mut store = Store::open(&path).expect("open");
        store
            .create_agent_profile(&AgentProfile {
                id: agp_id.clone(),
                display_name: DisplayName::try_from("Claude").expect("name"),
                preferred_harness: HarnessKind::Acp,
                memory_mode: MemoryMode::Off,
                created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
                updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
            })
            .expect("create agent");
        store
            .create_harness_binding(&binding)
            .expect("create binding");
    }

    let store = Store::open(&path).expect("reopen");
    let fetched = store
        .harness_binding_by_id(&hnb_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, binding);
}

// ── ProjectRef CRUD & Safe Delete ──────────────────────────────────

#[test]
fn project_ref_create_duplicate_get_and_upsert() {
    let mut store = Store::open_in_memory().expect("store");
    let prj_id = project_id("altiorrepo");

    let project = ProjectRef {
        id: prj_id.clone(),
        label: BoundedLabel::try_from("Altior Main").expect("label"),
        path: BoundedPath::try_from("D:/Projects/Altior").expect("path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    store.create_project_ref(&project).expect("create");

    // Same-ID same-content create is idempotent
    store
        .create_project_ref(&project)
        .expect("same-id same-content idempotent");

    // Same-ID conflicting content duplicate create fails closed
    let mut conflicting_prj = project.clone();
    conflicting_prj.label = BoundedLabel::try_from("Different Project").expect("label");
    let err_dup = store.create_project_ref(&conflicting_prj).unwrap_err();
    assert!(matches!(
        err_dup,
        StorageError::ProjectRefAlreadyExists { project_id } if project_id == prj_id.as_str()
    ));

    // Upsert with conflicting immutable created_at fails closed
    let mut bad_upsert_prj = project.clone();
    bad_upsert_prj.created_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err_prj_created = store.upsert_project_ref(&bad_upsert_prj).unwrap_err();
    assert!(matches!(
        err_prj_created,
        StorageError::InvalidEntityData { .. }
    ));

    // Get by ID
    let fetched = store
        .project_ref_by_id(&prj_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, project);

    // Upsert updates
    let updated = ProjectRef {
        id: prj_id.clone(),
        label: BoundedLabel::try_from("Altior Fork").expect("label"),
        path: BoundedPath::try_from("D:/Projects/AltiorFork").expect("path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store.upsert_project_ref(&updated).expect("upsert");
    let fetched_updated = store.project_ref_by_id(&prj_id).expect("get").unwrap();
    assert_eq!(fetched_updated.label.as_str(), "Altior Fork");
    assert_eq!(fetched_updated.path.as_str(), "D:/Projects/AltiorFork");
}

#[test]
fn project_ref_delete_rejected_when_referenced_by_thread() {
    let mut store = Store::open_in_memory().expect("store");
    let prj_id = project_id("altiorrepo");
    let agp = ensure_agent_profile(&mut store, "claude");

    // Create project
    store
        .create_project_ref(&ProjectRef {
            id: prj_id.clone(),
            label: BoundedLabel::try_from("Altior Project").expect("label"),
            path: BoundedPath::try_from("D:/Projects/Altior").expect("path"),
            created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
        })
        .expect("create project");

    // Create thread referencing this project
    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Project Thread"),
                ("project_id", prj_id.as_str()),
            ]),
        ))
        .expect("append thread");

    // Attempting to delete project should fail closed with typed error
    let delete_err = store.delete_project_ref(&prj_id).unwrap_err();
    assert!(matches!(
        delete_err,
        StorageError::ProjectReferencedByThreads {
            project_id,
            thread_count: 1
        } if project_id == prj_id.as_str()
    ));

    // Project is still intact
    assert!(store.project_ref_by_id(&prj_id).expect("get").is_some());

    // Another unreferenced project can be deleted safely
    let unref_id = project_id("unrefrepo");
    store
        .create_project_ref(&ProjectRef {
            id: unref_id.clone(),
            label: BoundedLabel::try_from("Unreferenced").expect("label"),
            path: BoundedPath::try_from("/tmp/unref").expect("path"),
            created_at: UnixMillis::from_millis(BASE_MILLIS + 60),
        })
        .expect("create unreferenced");
    assert!(store.delete_project_ref(&unref_id).expect("delete unref"));
    assert!(store.project_ref_by_id(&unref_id).expect("get").is_none());
}

#[test]
fn project_ref_pagination_and_reopen_equivalence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("prj_test.db");

    let prj1 = ProjectRef {
        id: project_id("prj01"),
        label: BoundedLabel::try_from("Project 1").expect("label"),
        path: BoundedPath::try_from("/path/1").expect("path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    let prj2 = ProjectRef {
        id: project_id("prj02"),
        label: BoundedLabel::try_from("Project 2").expect("label"),
        path: BoundedPath::try_from("/path/2").expect("path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    {
        let mut store = Store::open(&path).expect("open");
        store.create_project_ref(&prj1).expect("create 1");
        store.create_project_ref(&prj2).expect("create 2");
    }

    let store = Store::open(&path).expect("reopen");

    // Pagination test on reopened DB
    let limit = ProjectRefListLimit::try_new(1).expect("limit");
    let page1 = store.project_refs(None, limit).expect("page 1");
    assert_eq!(page1.len(), 1);
    assert_eq!(page1[0], prj1);

    let cursor = ProjectRefCursor {
        created_at: page1[0].created_at,
        project_id: page1[0].id.clone(),
    };
    let page2 = store.project_refs(Some(&cursor), limit).expect("page 2");
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0], prj2);
}

// ── Permission Query & Projection Lifecycle ────────────────────────

#[test]
fn permission_query_by_event_id_and_lifecycle_transitions() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
        ))
        .expect("thread");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("turn001"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("turn");

    let perm_evt_id = event_id(3);
    store
        .append_domain_event(&DomainEvent {
            event_id: perm_evt_id.clone(),
            thread_id: Some(thread_id("alpha")),
            turn_id: Some(turn_id("turn001")),
            operation_id: None,
            kind: DomainEventKind::PermissionRequested,
            payload: json_payload(&[
                ("permission_kind", "execute"),
                ("description", "Run bash command `cargo build`"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 300),
        })
        .expect("permission requested");

    // Query by event_id -> Pending
    let perm = store
        .permission_by_event_id(&perm_evt_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(perm.event_id, perm_evt_id);
    assert_eq!(perm.turn_id, turn_id("turn001"));
    assert_eq!(perm.thread_id, thread_id("alpha"));
    assert_eq!(perm.kind, PermissionKind::Execute);
    assert_eq!(perm.description.as_str(), "Run bash command `cargo build`");
    assert_eq!(perm.decision, PermissionDecision::Pending);
    assert_eq!(
        perm.requested_at,
        UnixMillis::from_millis(BASE_MILLIS + 300)
    );
    assert_eq!(perm.decided_at, None);

    // Decision: Approved
    let dec_evt_id = event_id(4);
    store
        .append_domain_event(&DomainEvent {
            event_id: dec_evt_id,
            thread_id: Some(thread_id("alpha")),
            turn_id: Some(turn_id("turn001")),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "approved"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 400),
        })
        .expect("permission decided approved");

    let approved = store
        .permission_by_event_id(&perm_evt_id)
        .expect("get")
        .unwrap();
    assert_eq!(approved.decision, PermissionDecision::Approved);
    assert_eq!(
        approved.decided_at,
        Some(UnixMillis::from_millis(BASE_MILLIS + 400))
    );
}

#[test]
fn permission_lifecycle_denied_and_rebuild_projection() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
        ))
        .expect("thread");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("turn001"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("turn");

    let perm_evt_id = event_id(3);
    store
        .append_domain_event(&DomainEvent {
            event_id: perm_evt_id.clone(),
            thread_id: Some(thread_id("alpha")),
            turn_id: Some(turn_id("turn001")),
            operation_id: None,
            kind: DomainEventKind::PermissionRequested,
            payload: json_payload(&[
                ("permission_kind", "write"),
                ("description", "Write to /etc/hosts"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 300),
        })
        .expect("request");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(4),
            thread_id: Some(thread_id("alpha")),
            turn_id: Some(turn_id("turn001")),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "denied"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 400),
        })
        .expect("denied");

    let denied = store
        .permission_by_event_id(&perm_evt_id)
        .expect("get")
        .unwrap();
    assert_eq!(denied.decision, PermissionDecision::Denied);
    assert_eq!(denied.kind, PermissionKind::Write);

    // Rebuild projection and assert exact reproduction
    store.rebuild_domain_projections().expect("rebuild");
    let after_rebuild = store
        .permission_by_event_id(&perm_evt_id)
        .expect("get")
        .unwrap();
    assert_eq!(after_rebuild, denied);
}

#[test]
fn permission_queries_by_turn_and_thread_with_stable_tie_pagination() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let tid = thread_id("alpha");
    let trn = turn_id("turn001");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "T")]),
        ))
        .expect("thread");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("turn001"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("turn");

    // Insert 4 permission requests with identical requested_at timestamp
    let mut perm_ids = Vec::new();
    for i in 10..=13u64 {
        let eid = event_id(i);
        perm_ids.push(eid.clone());
        store
            .append_domain_event(&DomainEvent {
                event_id: eid,
                thread_id: Some(tid.clone()),
                turn_id: Some(trn.clone()),
                operation_id: None,
                kind: DomainEventKind::PermissionRequested,
                payload: json_payload(&[
                    ("permission_kind", "read"),
                    ("description", &format!("Read file {i}")),
                ]),
                occurred_at: UnixMillis::from_millis(BASE_MILLIS + 1000),
            })
            .expect("perm req");
    }

    let limit = PermissionListLimit::try_new(2).expect("limit");

    // Test permissions_for_turn pagination
    let turn_page1 = store
        .permissions_for_turn(&trn, None, limit)
        .expect("turn page 1");
    assert_eq!(turn_page1.len(), 2);
    assert_eq!(turn_page1[0].event_id, perm_ids[0]);
    assert_eq!(turn_page1[1].event_id, perm_ids[1]);

    let cursor1 = PermissionCursor {
        requested_at: turn_page1[1].requested_at,
        event_id: turn_page1[1].event_id.clone(),
    };
    let turn_page2 = store
        .permissions_for_turn(&trn, Some(&cursor1), limit)
        .expect("turn page 2");
    assert_eq!(turn_page2.len(), 2);
    assert_eq!(turn_page2[0].event_id, perm_ids[2]);
    assert_eq!(turn_page2[1].event_id, perm_ids[3]);

    let cursor2 = PermissionCursor {
        requested_at: turn_page2[1].requested_at,
        event_id: turn_page2[1].event_id.clone(),
    };
    let turn_page3 = store
        .permissions_for_turn(&trn, Some(&cursor2), limit)
        .expect("turn page 3");
    assert!(turn_page3.is_empty());

    // Test permissions_for_thread pagination
    let thr_page1 = store
        .permissions_for_thread(&tid, None, limit)
        .expect("thread page 1");
    assert_eq!(thr_page1.len(), 2);
    assert_eq!(thr_page1[0].event_id, perm_ids[0]);
    assert_eq!(thr_page1[1].event_id, perm_ids[1]);

    let thr_cursor1 = PermissionCursor {
        requested_at: thr_page1[1].requested_at,
        event_id: thr_page1[1].event_id.clone(),
    };
    let thr_page2 = store
        .permissions_for_thread(&tid, Some(&thr_cursor1), limit)
        .expect("thread page 2");
    assert_eq!(thr_page2.len(), 2);
    assert_eq!(thr_page2[0].event_id, perm_ids[2]);
    assert_eq!(thr_page2[1].event_id, perm_ids[3]);
}

// ── Bounded Limits Validation ──────────────────────────────────────

#[test]
fn new_list_limits_enforce_boundaries() {
    assert!(AgentProfileListLimit::try_new(1).is_ok());
    assert!(AgentProfileListLimit::try_new(AGENT_PROFILE_LIST_LIMIT_MAX).is_ok());
    assert!(matches!(
        AgentProfileListLimit::try_new(0),
        Err(EntityError::AgentProfileListLimitOutOfRange { .. })
    ));
    assert!(matches!(
        AgentProfileListLimit::try_new(AGENT_PROFILE_LIST_LIMIT_MAX + 1),
        Err(EntityError::AgentProfileListLimitOutOfRange { .. })
    ));

    assert!(HarnessBindingListLimit::try_new(1).is_ok());
    assert!(HarnessBindingListLimit::try_new(HARNESS_BINDING_LIST_LIMIT_MAX).is_ok());
    assert!(matches!(
        HarnessBindingListLimit::try_new(0),
        Err(EntityError::HarnessBindingListLimitOutOfRange { .. })
    ));
    assert!(matches!(
        HarnessBindingListLimit::try_new(HARNESS_BINDING_LIST_LIMIT_MAX + 1),
        Err(EntityError::HarnessBindingListLimitOutOfRange { .. })
    ));

    assert!(ProjectRefListLimit::try_new(1).is_ok());
    assert!(ProjectRefListLimit::try_new(PROJECT_REF_LIST_LIMIT_MAX).is_ok());
    assert!(matches!(
        ProjectRefListLimit::try_new(0),
        Err(EntityError::ProjectRefListLimitOutOfRange { .. })
    ));
    assert!(matches!(
        ProjectRefListLimit::try_new(PROJECT_REF_LIST_LIMIT_MAX + 1),
        Err(EntityError::ProjectRefListLimitOutOfRange { .. })
    ));

    assert!(PermissionListLimit::try_new(1).is_ok());
    assert!(PermissionListLimit::try_new(PERMISSION_LIST_LIMIT_MAX).is_ok());
    assert!(matches!(
        PermissionListLimit::try_new(0),
        Err(EntityError::PermissionListLimitOutOfRange { .. })
    ));
    assert!(matches!(
        PermissionListLimit::try_new(PERMISSION_LIST_LIMIT_MAX + 1),
        Err(EntityError::PermissionListLimitOutOfRange { .. })
    ));

    assert!(TurnListLimit::try_new(1).is_ok());
    assert!(TurnListLimit::try_new(TURN_LIST_LIMIT_MAX).is_ok());
    assert!(matches!(
        TurnListLimit::try_new(0),
        Err(EntityError::TurnListLimitOutOfRange { .. })
    ));
    assert!(matches!(
        TurnListLimit::try_new(TURN_LIST_LIMIT_MAX + 1),
        Err(EntityError::TurnListLimitOutOfRange { .. })
    ));
}

// ── Additional Fail-Closed and Projection Invariant Tests ───────────

#[test]
fn thread_created_with_invalid_agent_profile_id_fails_closed() {
    let mut store = Store::open_in_memory().expect("store");
    let invalid_agp = "invalid-agent-id-not-base32";

    let err = store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", invalid_agp),
                ("title", "Fail Closed Agent"),
            ]),
        ))
        .unwrap_err();

    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));
}

#[test]
fn thread_created_with_invalid_project_id_fails_closed() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let invalid_prj = "invalid-project-id-not-base32";

    let err = store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Fail Closed Project"),
                ("project_id", invalid_prj),
            ]),
        ))
        .unwrap_err();

    assert!(matches!(err, StorageError::InvalidDomainEvent { .. }));
}

#[test]
fn thread_created_with_valid_project_ref_projects() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let prj = ensure_project_ref(&mut store, "myproject");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Project Thread"),
                ("project_id", prj.as_str()),
            ]),
        ))
        .expect("create thread with project");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.project_id.as_deref(), Some(prj.as_str()));
}

#[test]
fn thread_title_change_to_empty_updates_projection() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Initial Title"),
            ]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            None,
            DomainEventKind::ThreadTitleChanged,
            json_payload(&[("title", "")]),
        ))
        .expect("empty title");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("query").expect("exists");
    assert_eq!(thread.title, "");
    assert_eq!(thread.event_count, 2);
}

#[test]
fn terminal_turn_rejects_message_delta_and_permission_requested() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("start turn");

    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnCompleted,
            empty_payload(),
        ))
        .expect("complete turn");

    // MessageDelta on completed turn must fail closed
    let delta_err = store
        .append_domain_event(&domain_event(
            4,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::MessageDelta,
            json_payload(&[("text", "late text")]),
        ))
        .unwrap_err();
    assert!(
        matches!(delta_err, StorageError::InvalidDomainEvent { .. }),
        "MessageDelta on terminal turn must fail: {delta_err:?}"
    );

    // PermissionRequested on completed turn must fail closed
    let perm_err = store
        .append_domain_event(&domain_event(
            5,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::PermissionRequested,
            json_payload(&[
                ("permission_kind", "execute"),
                ("description", "Late command"),
            ]),
        ))
        .unwrap_err();
    assert!(
        matches!(perm_err, StorageError::InvalidDomainEvent { .. }),
        "PermissionRequested on terminal turn must fail: {perm_err:?}"
    );
}

#[test]
fn empty_agent_profile_table_allows_thread_created_and_rebuild_and_subsequent_profile_creation() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("claude");

    // Empty agent_profile table allows ThreadCreated
    assert_eq!(
        store
            .agent_profiles(None, AgentProfileListLimit::try_new(10).unwrap())
            .unwrap()
            .len(),
        0
    );

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp_id.as_str()),
                ("title", "Initial Title"),
            ]),
        ))
        .expect("append thread created on empty agent_profile");

    let tid = thread_id("alpha");
    let thread = store.thread_by_id(&tid).expect("get").expect("exists");
    assert_eq!(thread.agent_profile_id, agp_id.as_str());

    // Rebuild projections on empty agent_profile table succeeds
    let count = store.rebuild_domain_projections().expect("rebuild");
    assert_eq!(count, 1);

    let thread_rebuilt = store.thread_by_id(&tid).expect("get").expect("exists");
    assert_eq!(thread_rebuilt.agent_profile_id, agp_id.as_str());
    assert_eq!(thread_rebuilt.title, "Initial Title");

    // Subsequent creation of AgentProfile succeeds and does not corrupt thread projection
    let profile = fixture_agent_profile("claude");
    store
        .create_agent_profile(&profile)
        .expect("create agent profile after thread");

    let thread_after = store.thread_by_id(&tid).expect("get").expect("exists");
    assert_eq!(thread_after.agent_profile_id, agp_id.as_str());
    assert_eq!(thread_after.title, "Initial Title");
}

#[test]
fn empty_database_orphan_projections_are_cleaned_up_on_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");

    let orphan_tid = thread_id("orphan0000000000");
    let orphan_turn_id = turn_id("orphan0000000000");
    let orphan_permission_event_id = event_id(9999);
    let orphan_agp = agent_id("orphan0000000000");

    // Initialize DB schema using Store::open and create a device-local AgentProfile row
    {
        let mut store = Store::open(&path).expect("open to create schema");
        let profile = fixture_agent_profile("localdevagent01");
        store
            .create_agent_profile(&profile)
            .expect("create profile");
    }

    let raw = rusqlite::Connection::open(&path).expect("raw connection");

    // Insert orphan rows into thread, turn, permission directly
    raw.execute(
        "INSERT INTO thread (thread_id, agent_profile_id, title, state, created_at, updated_at)
         VALUES (?1, ?2, 'Orphan Thread', 'open', 1000, 1000)",
        rusqlite::params![orphan_tid.as_str(), orphan_agp.as_str()],
    )
    .expect("insert orphan thread");

    raw.execute(
        "INSERT INTO turn (turn_id, thread_id, state, delivery, event_count, started_at)
         VALUES (?1, ?2, 'active', 'absent', 1, 1000)",
        rusqlite::params![orphan_turn_id.as_str(), orphan_tid.as_str()],
    )
    .expect("insert orphan turn");

    raw.execute(
        "INSERT INTO permission (event_id, turn_id, thread_id, kind, description, decision, requested_at)
         VALUES (?1, ?2, ?3, 'execute', 'desc', 'pending', 1000)",
        rusqlite::params![
            orphan_permission_event_id.as_str(),
            orphan_turn_id.as_str(),
            orphan_tid.as_str()
        ],
    )
    .expect("insert orphan permission");

    // Wipe marker so marker is absent
    raw.execute("DELETE FROM domain_projection_state", [])
        .expect("delete marker");

    drop(raw);

    // Reopen store: empty journal with missing marker must rebuild safely,
    // wiping orphan thread/turn/permission rows while preserving device-local agent_profile
    let store = Store::open(&path).expect("reopen store with missing marker");

    assert!(
        store
            .thread_by_id(&orphan_tid)
            .expect("query thread")
            .is_none()
    );

    assert_eq!(
        store
            .turns_for_thread(
                &orphan_tid,
                None,
                TurnListLimit::try_new(10).expect("limit")
            )
            .expect("query turns")
            .len(),
        0
    );

    assert!(
        store
            .permission_by_event_id(&orphan_permission_event_id)
            .expect("query permission")
            .is_none()
    );

    // Device-local CRUD table agent_profile must NOT be cleaned up
    let local_agp_id = agent_id("localdevagent01");
    let agp_row = store
        .agent_profile_by_id(&local_agp_id)
        .expect("query agent")
        .expect("preserved");
    assert_eq!(agp_row.id, local_agp_id);

    // Marker must now be initialized
    let raw = rusqlite::Connection::open(&path).expect("raw connection");
    let (marker_seq, marker_ver, marker_digest): (i64, i64, String) = raw
        .query_row(
            "SELECT journal_max_seq, projection_version, projection_digest FROM domain_projection_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query marker");
    assert_eq!(marker_seq, 0);
    assert_eq!(marker_ver, altior_storage::DOMAIN_PROJECTION_VERSION);
    assert!(!marker_digest.is_empty());
}

#[test]
fn digest_pre_and_post_commit_are_consistent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");

    let mut store = Store::open(&path).expect("open");
    let agp = agent_id("claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("alpha"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str()), ("title", "Thread A")]),
        ))
        .expect("create thread");

    store
        .append_domain_event(&domain_event(
            2,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::TurnStarted,
            empty_payload(),
        ))
        .expect("turn start");

    store
        .append_domain_event(&domain_event(
            3,
            Some("alpha"),
            Some("alpha0turn000"),
            DomainEventKind::PermissionRequested,
            json_payload(&[("permission_kind", "execute"), ("description", "Run build")]),
        ))
        .expect("permission request");

    // Pre-commit digest stored in marker
    let raw = rusqlite::Connection::open(&path).expect("raw");
    let marker_digest: String = raw
        .query_row(
            "SELECT projection_digest FROM domain_projection_state WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("get stored digest");

    // Post-commit live digest computed over committed projections
    let live_digest = domain_projection_digest(&raw).expect("compute live digest");
    assert_eq!(
        marker_digest, live_digest,
        "pre-commit marker digest must match post-commit digest"
    );

    // After rebuild, stored digest must also match post-commit live digest
    drop(raw);
    store.rebuild_domain_projections().expect("rebuild");

    let raw = rusqlite::Connection::open(&path).expect("raw");
    let rebuild_marker_digest: String = raw
        .query_row(
            "SELECT projection_digest FROM domain_projection_state WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("get rebuild stored digest");
    let rebuild_live_digest = domain_projection_digest(&raw).expect("compute rebuild live digest");
    assert_eq!(rebuild_marker_digest, rebuild_live_digest);
    assert_eq!(marker_digest, rebuild_marker_digest);
}

#[test]
fn domain_journal_replays_without_local_agent_profile_or_project_ref() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");

    let foreign_agent_id = agent_id("remotepeerdev001");
    let foreign_project_id = project_id("remoteproject001");

    {
        let mut store = Store::open(&path).expect("open");
        // No local agent_profile or project_ref rows inserted!
        assert_eq!(
            store
                .agent_profiles(None, AgentProfileListLimit::try_new(10).unwrap())
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            store
                .project_refs(None, ProjectRefListLimit::try_new(10).unwrap())
                .unwrap()
                .len(),
            0
        );

        store
            .append_domain_event(&domain_event(
                1,
                Some("remotethread0001"),
                None,
                DomainEventKind::ThreadCreated,
                json_payload(&[
                    ("agent_profile_id", foreign_agent_id.as_str()),
                    ("title", "Remote Sync Thread"),
                    ("project_id", foreign_project_id.as_str()),
                ]),
            ))
            .expect("append thread with remote agent id and project id");

        store
            .append_domain_event(&domain_event(
                2,
                Some("remotethread0001"),
                Some("remoteturn000001"),
                DomainEventKind::TurnStarted,
                empty_payload(),
            ))
            .expect("turn start");

        store
            .append_domain_event(&domain_event(
                3,
                Some("remotethread0001"),
                Some("remoteturn000001"),
                DomainEventKind::TurnCompleted,
                empty_payload(),
            ))
            .expect("turn complete");
    }

    // Damage: wipe projection tables
    let raw = rusqlite::Connection::open(&path).expect("raw");
    raw.execute("DELETE FROM thread", [])
        .expect("delete thread");
    raw.execute("DELETE FROM turn", []).expect("delete turn");
    raw.execute("DELETE FROM domain_projection_state", [])
        .expect("delete marker");
    drop(raw);

    // Reopen must replay the entire domain journal without requiring local agent profile or project ref rows
    let store = Store::open(&path).expect("reopen and rebuild");
    let tid = thread_id("remotethread0001");
    let thread = store
        .thread_by_id(&tid)
        .expect("query thread")
        .expect("rebuilt");
    assert_eq!(thread.agent_profile_id, foreign_agent_id.as_str());
    assert_eq!(
        thread.project_id.as_deref(),
        Some(foreign_project_id.as_str())
    );
    assert_eq!(thread.title, "Remote Sync Thread");
    assert_eq!(thread.event_count, 3);

    let turns = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("query turns");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, "completed");
}

#[test]
fn search_threads_with_special_characters_does_not_error() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    // Create a thread with syntax-like and special characters in the title
    store
        .append_domain_event(&domain_event(
            1,
            Some("special0thread01"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                (
                    "title",
                    "Fix: title with OR NOT AND * (Tokio) and \"quotes\"",
                ),
            ]),
        ))
        .expect("create thread 1");

    store
        .append_domain_event(&domain_event(
            2,
            Some("special0thread02"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Simple thread title"),
            ]),
        ))
        .expect("create thread 2");

    let limit = ThreadListLimit::try_new(10).expect("limit");

    // Various special queries: operators, wildcards, colons, unbalanced quotes, syntax punctuation
    let special_queries = [
        "OR",
        "NOT",
        "AND",
        "*",
        ":",
        "\"",
        "\"\"\"",
        "unclosed \" quote",
        "Fix:",
        "title:",
        "(Tokio)",
        "*Tokio*",
        "OR NOT AND",
        "\"quotes\"",
        "NEAR/2",
        "^ - + = { } [ ] < > ~ ?",
    ];

    for q_str in &special_queries {
        if let Ok(query) = SearchQuery::try_from(*q_str) {
            let result = store.search_threads(&query, None, limit);
            assert!(
                result.is_ok(),
                "search_threads failed on query {:?}: {:?}",
                q_str,
                result.err()
            );
        }
    }

    // Verify literal matching behavior (not boolean syntax)
    let or_query = SearchQuery::try_from("OR").expect("query");
    let or_hits = store
        .search_threads(&or_query, None, limit)
        .expect("search 'OR'");
    assert_eq!(or_hits.len(), 1);
    assert_eq!(or_hits[0].thread_id, id("thr_", "special0thread01"));

    let quotes_query = SearchQuery::try_from("quotes").expect("query");
    let quotes_hits = store
        .search_threads(&quotes_query, None, limit)
        .expect("search 'quotes'");
    assert_eq!(quotes_hits.len(), 1);
    assert_eq!(quotes_hits[0].thread_id, id("thr_", "special0thread01"));
}

#[test]
fn turns_for_thread_timestamp_tie_pagination() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&domain_event(
            1,
            Some("threadtietest001"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread");

    let tid = thread_id("threadtietest001");
    let count = 7;
    let mut created_turn_ids = Vec::new();
    let tied_timestamp = UnixMillis::from_millis(BASE_MILLIS + 500);

    for i in 1..=count {
        let tname = format!("turn{i:012}");
        let trn_id = turn_id(&tname);
        created_turn_ids.push(trn_id.to_string());
        store
            .append_domain_event(&DomainEvent {
                event_id: event_id(10 + i as u64),
                thread_id: Some(tid.clone()),
                turn_id: Some(trn_id),
                operation_id: None,
                kind: DomainEventKind::TurnStarted,
                payload: empty_payload(),
                occurred_at: tied_timestamp,
            })
            .expect("append turn");
    }

    // Expected order is sorted by (started_at ASC, turn_id ASC)
    created_turn_ids.sort();

    let limit = TurnListLimit::try_new(2).expect("limit");
    let mut collected = Vec::new();
    let mut cursor: Option<TurnCursor> = None;
    let mut page_count = 0;

    loop {
        let page = store
            .turns_for_thread(&tid, cursor.as_ref(), limit)
            .expect("page query");
        if page.is_empty() {
            break;
        }
        page_count += 1;
        for turn in &page {
            collected.push(turn.turn_id.clone());
        }
        let last = page.last().unwrap();
        cursor = Some(TurnCursor {
            started_at: UnixMillis::from_millis(
                u64::try_from(last.started_at).expect("positive started_at"),
            ),
            turn_id: TurnId::try_from(last.turn_id.as_str()).expect("turn_id"),
        });
    }

    assert!(
        page_count >= 3,
        "expected at least 3 pages, got {page_count}"
    );
    assert_eq!(collected.len(), count);
    assert_eq!(collected, created_turn_ids);

    // Verify strictly no duplicates
    let mut unique = collected.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), collected.len());
}

#[test]
fn other_domain_events_global_thread_scoped_and_rebuild_consistency() {
    let mut store = Store::open_in_memory().expect("store");

    // 1. Global Other (thread_id: None, turn_id: None)
    store
        .append_domain_event(&domain_event(
            1,
            None,
            None,
            DomainEventKind::Other("system.metric_heartbeat".to_owned()),
            empty_payload(),
        ))
        .expect("append global other");

    assert_eq!(store.domain_journal_len().expect("count"), 1);
    let limit = ThreadListLimit::try_new(10).expect("limit");
    let threads = store.thread_list(None, None, limit).expect("list");
    assert!(
        threads.is_empty(),
        "global other should not project threads"
    );

    // 2. Thread-scoped Other on non-existent thread fails closed
    let non_existent_tid = thread_id("nonexistentthr001");
    let err_missing = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(2),
            thread_id: Some(non_existent_tid),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::Other("custom.thread_ping".to_owned()),
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .unwrap_err();
    assert!(matches!(
        err_missing,
        StorageError::InvalidDomainEvent { .. }
    ));

    // 3. Other with turn_id fails closed (both global and thread-scoped)
    let err_turn_global = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(3),
            thread_id: None,
            turn_id: Some(turn_id("someturn00000001")),
            operation_id: None,
            kind: DomainEventKind::Other("custom.turn_metric".to_owned()),
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .unwrap_err();
    assert!(matches!(
        err_turn_global,
        StorageError::InvalidDomainEvent { .. }
    ));

    // 4. Thread-scoped Other on existing thread folds update_thread_activity
    let agp = ensure_agent_profile(&mut store, "claude");
    let tid = thread_id("customtestthread1");
    store
        .append_domain_event(&domain_event(
            4,
            Some("customtestthread1"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("title", "Custom Thread"),
            ]),
        ))
        .expect("thread created");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(5),
            thread_id: Some(tid.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::Other("custom.thread_ping".to_owned()),
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
        })
        .expect("thread scoped other");

    let thread = store
        .thread_by_id(&tid)
        .expect("query thread")
        .expect("thread exists");
    assert_eq!(thread.event_count, 2);
    assert_eq!(thread.updated_at, (BASE_MILLIS + 200).cast_signed());

    // 5. Rebuild reproduces identical projection state
    let count = store.rebuild_domain_projections().expect("rebuild");
    assert_eq!(count, 3); // events 1, 4, 5
    let thread_post = store
        .thread_by_id(&tid)
        .expect("query thread")
        .expect("thread exists");
    assert_eq!(thread_post.event_count, 2);
    assert_eq!(thread_post.updated_at, (BASE_MILLIS + 200).cast_signed());
}

#[test]
fn durable_collision_all_six_fields_fail_closed_and_journal_count_remains_one() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let tid1 = thread_id("coltestthread001");
    let tid2 = thread_id("coltestthread002");
    let trn1 = turn_id("coltestturn00001");
    let trn2 = turn_id("coltestturn00002");
    let op1 = operation_id("coloperation0001");
    let op2 = operation_id("coloperation0002");

    // Setup thread 1 and thread 2
    store
        .append_domain_event(&domain_event(
            1,
            Some("coltestthread001"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread 1");

    store
        .append_domain_event(&domain_event(
            2,
            Some("coltestthread002"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread 2");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(3),
            thread_id: Some(tid1.clone()),
            turn_id: Some(trn1.clone()),
            operation_id: Some(op1.clone()),
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn 1 on thread 1");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(4),
            thread_id: Some(tid2.clone()),
            turn_id: Some(trn2.clone()),
            operation_id: Some(op2.clone()),
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn 2 on thread 2");

    // Initial base event with all 6 durable fields populated
    let base_event = DomainEvent {
        event_id: event_id(100),
        thread_id: Some(tid1.clone()),
        turn_id: Some(trn1.clone()),
        operation_id: Some(op1.clone()),
        kind: DomainEventKind::MessageDelta,
        payload: json_payload(&[("content", "hello world")]),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
    };
    store.append_domain_event(&base_event).expect("append base");

    let count_before = store.domain_journal_len().expect("journal len");
    assert_eq!(count_before, 5);

    // Exact duplicate append returns Duplicate outcome and does not add row
    let dup_res = store
        .append_domain_event(&base_event)
        .expect("duplicate ok");
    assert!(matches!(dup_res, AppendOutcome::Duplicate { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 1: thread_id
    let mut mut_thread = base_event.clone();
    mut_thread.thread_id = Some(tid2.clone());
    let err = store.append_domain_event(&mut_thread).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 2: turn_id
    let mut mut_turn = base_event.clone();
    mut_turn.turn_id = Some(trn2.clone());
    let err = store.append_domain_event(&mut_turn).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 3: operation_id
    let mut mut_op = base_event.clone();
    mut_op.operation_id = Some(op2.clone());
    let err = store.append_domain_event(&mut_op).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 4: kind
    let mut mut_kind = base_event.clone();
    mut_kind.kind = DomainEventKind::TurnCompleted;
    let err = store.append_domain_event(&mut_kind).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 5: payload
    let mut mut_payload = base_event.clone();
    mut_payload.payload = json_payload(&[("content", "different payload")]);
    let err = store.append_domain_event(&mut_payload).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);

    // Mutation 6: occurred_at
    let mut mut_time = base_event.clone();
    mut_time.occurred_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err = store.append_domain_event(&mut_time).unwrap_err();
    assert!(matches!(err, StorageError::EventIdCollision { .. }));
    assert_eq!(store.domain_journal_len().expect("len"), count_before);
}

#[test]
fn operation_id_turn_started_journal_projection_and_rebuild_consistency() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let tid = thread_id("optestthread0001");
    let trn = turn_id("optestturn000001");
    let op = operation_id("parentoperation1");

    store
        .append_domain_event(&domain_event(
            1,
            Some("optestthread0001"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(2),
            thread_id: Some(tid.clone()),
            turn_id: Some(trn.clone()),
            operation_id: Some(op.clone()),
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn with operation id");

    // Verify in journal records
    let journal = store
        .domain_journal_records(0, JournalLimit::try_new(10).expect("limit"))
        .expect("journal");
    let turn_event = journal
        .iter()
        .find(|r| r.event_id == event_id(2).as_str())
        .unwrap();
    assert_eq!(turn_event.operation_id.as_deref(), Some(op.as_str()));

    // Verify in turn projection
    let turns = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("turns");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].operation_id.as_deref(), Some(op.as_str()));

    // Verify after rebuild
    store.rebuild_domain_projections().expect("rebuild");
    let turns_post = store
        .turns_for_thread(&tid, None, TurnListLimit::try_new(10).expect("limit"))
        .expect("turns post rebuild");
    assert_eq!(turns_post.len(), 1);
    assert_eq!(turns_post[0].operation_id.as_deref(), Some(op.as_str()));
}

#[test]
fn thread_title_clear_and_search_consistency_across_rebuild_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("title_clear_test.db");

    let tid = thread_id("titleclearthread");
    let limit = ThreadListLimit::try_new(10).expect("limit");
    let query = SearchQuery::try_from("Architecture").expect("query");

    {
        let mut store = Store::open(&db_path).expect("open store");
        let agp = ensure_agent_profile(&mut store, "claude");

        store
            .append_domain_event(&DomainEvent {
                event_id: event_id(1),
                thread_id: Some(tid.clone()),
                turn_id: None,
                operation_id: None,
                kind: DomainEventKind::ThreadCreated,
                payload: json_payload(&[
                    ("agent_profile_id", agp.as_str()),
                    ("title", "Architecture Deep Dive"),
                ]),
                occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
            })
            .expect("create");

        // Search finds initial title
        let results = store.search_threads(&query, None, limit).expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].thread_id, tid.as_str());

        // Clear title via ThreadTitleChanged with empty string
        store
            .append_domain_event(&DomainEvent {
                event_id: event_id(2),
                thread_id: Some(tid.clone()),
                turn_id: None,
                operation_id: None,
                kind: DomainEventKind::ThreadTitleChanged,
                payload: json_payload(&[("title", "")]),
                occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
            })
            .expect("clear title");

        // Search no longer finds cleared title
        let results_cleared = store.search_threads(&query, None, limit).expect("search");
        assert_eq!(results_cleared.len(), 0);

        // Rebuild projections maintains cleared search
        store.rebuild_domain_projections().expect("rebuild");
        let results_rebuilt = store.search_threads(&query, None, limit).expect("search");
        assert_eq!(results_rebuilt.len(), 0);
    }

    // Reopen database from disk and verify search is still empty
    {
        let store = Store::open(&db_path).expect("reopen store");
        let results_reopened = store.search_threads(&query, None, limit).expect("search");
        assert_eq!(results_reopened.len(), 0);
    }
}

#[test]
#[allow(clippy::similar_names, clippy::too_many_lines)]
fn permission_decision_second_decision_and_cross_context_rejection() {
    let mut store = Store::open_in_memory().expect("store");
    let agp = ensure_agent_profile(&mut store, "claude");
    let tid_a = thread_id("permthreadalpha1");
    let tid_b = thread_id("permthreadbravo2");
    let trn_a1 = turn_id("permturnalpha001");
    let trn_a2 = turn_id("permturnalpha002");
    let trn_b1 = turn_id("permturnbravo001");

    store
        .append_domain_event(&domain_event(
            1,
            Some("permthreadalpha1"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread a");

    store
        .append_domain_event(&domain_event(
            2,
            Some("permthreadbravo2"),
            None,
            DomainEventKind::ThreadCreated,
            json_payload(&[("agent_profile_id", agp.as_str())]),
        ))
        .expect("create thread b");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(3),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a1.clone()),
            operation_id: None,
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn a1");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(4),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a2.clone()),
            operation_id: None,
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn a2");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(5),
            thread_id: Some(tid_b.clone()),
            turn_id: Some(trn_b1.clone()),
            operation_id: None,
            kind: DomainEventKind::TurnStarted,
            payload: empty_payload(),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        })
        .expect("start turn b1");

    let perm_evt_id = event_id(10);
    // Request permission in thread A, turn A1
    store
        .append_domain_event(&DomainEvent {
            event_id: perm_evt_id.clone(),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a1.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionRequested,
            payload: json_payload(&[
                ("permission_kind", "execute"),
                ("description", "Cargo build"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 150),
        })
        .expect("request permission");

    // Cross-thread decision attempt rejected
    let err_cross_thread = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(11),
            thread_id: Some(tid_b.clone()),
            turn_id: Some(trn_b1.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "approved"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
        })
        .unwrap_err();
    assert!(matches!(
        err_cross_thread,
        StorageError::InvalidDomainEvent { .. }
    ));

    // Cross-turn decision attempt rejected
    let err_cross_turn = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(12),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a2.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "approved"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
        })
        .unwrap_err();
    assert!(matches!(
        err_cross_turn,
        StorageError::InvalidDomainEvent { .. }
    ));

    // First decision: approved -> succeeds
    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(13),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a1.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "approved"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
        })
        .expect("decide approved");

    // Second same decision (approved again) -> rejected
    let err_same = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(14),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a1.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "approved"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 250),
        })
        .unwrap_err();
    assert!(matches!(err_same, StorageError::InvalidDomainEvent { .. }));

    // Second opposite decision (denied after approved) -> rejected
    let err_opposite = store
        .append_domain_event(&DomainEvent {
            event_id: event_id(15),
            thread_id: Some(tid_a.clone()),
            turn_id: Some(trn_a1.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: json_payload(&[
                ("permission_event_id", perm_evt_id.as_str()),
                ("decision", "denied"),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 250),
        })
        .unwrap_err();
    assert!(matches!(
        err_opposite,
        StorageError::InvalidDomainEvent { .. }
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn harness_binding_upsert_atomic_and_rejects_immutable_modifications() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id1 = agent_id("agentone00000000");
    let agp_id2 = agent_id("agenttwo00000000");

    store
        .create_agent_profile(&AgentProfile {
            id: agp_id1.clone(),
            display_name: DisplayName::try_from("Agent One").expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        })
        .expect("create agent 1");

    store
        .create_agent_profile(&AgentProfile {
            id: agp_id2.clone(),
            display_name: DisplayName::try_from("Agent Two").expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 20),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 20),
        })
        .expect("create agent 2");

    let hnb_id = binding_id("harnessbnd000001");
    let non_existent_agent = agent_id("agentnonexistent");

    // Upsert with non-existent agent profile fails with AgentProfileNotFound
    let bad_agent_binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: non_existent_agent,
        label: DisplayName::try_from("Binding 1").expect("label"),
        command: BoundedPath::try_from("/bin/cmd1").expect("cmd"),
        args: vec![],
        env_keys: vec![],
        secret_refs: vec![],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    let err_agent = store
        .upsert_harness_binding(&bad_agent_binding)
        .unwrap_err();
    assert!(matches!(
        err_agent,
        StorageError::AgentProfileNotFound { .. }
    ));

    // Valid initial upsert creates row
    let binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id1.clone(),
        label: DisplayName::try_from("Binding Initial").expect("label"),
        command: BoundedPath::try_from("/bin/initial").expect("cmd"),
        args: vec![HarnessArg::try_from("--init-flag").expect("arg")],
        env_keys: vec![HarnessEnvKey::try_from("INIT_ENV").expect("env")],
        secret_refs: vec![HarnessSecretRef::try_from("vault:sec_init").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store
        .upsert_harness_binding(&binding)
        .expect("upsert create");

    let fetched = store
        .harness_binding_by_id(&hnb_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched.label.as_str(), "Binding Initial");
    assert_eq!(fetched.command.as_str(), "/bin/initial");
    assert_eq!(fetched.args, binding.args);
    assert_eq!(fetched.env_keys, binding.env_keys);
    assert_eq!(fetched.secret_refs, binding.secret_refs);

    // Valid upsert updates label, command, args, env_keys, secret_refs
    let updated_binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id1.clone(),
        label: DisplayName::try_from("Binding Updated").expect("label"),
        command: BoundedPath::try_from("/bin/updated").expect("cmd"),
        args: vec![
            HarnessArg::try_from("--updated-flag").expect("arg"),
            HarnessArg::try_from("--another-flag").expect("arg"),
        ],
        env_keys: vec![HarnessEnvKey::try_from("UPDATED_ENV").expect("env")],
        secret_refs: vec![HarnessSecretRef::try_from("vault:sec_updated").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store
        .upsert_harness_binding(&updated_binding)
        .expect("upsert update");

    let fetched_up = store
        .harness_binding_by_id(&hnb_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched_up.label.as_str(), "Binding Updated");
    assert_eq!(fetched_up.command.as_str(), "/bin/updated");
    assert_eq!(fetched_up.args, updated_binding.args);
    assert_eq!(fetched_up.env_keys, updated_binding.env_keys);
    assert_eq!(fetched_up.secret_refs, updated_binding.secret_refs);

    // Upsert attempting to change agent_profile_id fails with InvalidEntityData
    let mut bad_agent_mod = updated_binding.clone();
    bad_agent_mod.agent_profile_id = agp_id2;
    let err_mod_agp = store.upsert_harness_binding(&bad_agent_mod).unwrap_err();
    assert!(matches!(
        err_mod_agp,
        StorageError::InvalidEntityData { .. }
    ));

    // Upsert attempting to change created_at fails with InvalidEntityData
    let mut bad_created_mod = updated_binding.clone();
    bad_created_mod.created_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err_mod_created = store.upsert_harness_binding(&bad_created_mod).unwrap_err();
    assert!(matches!(
        err_mod_created,
        StorageError::InvalidEntityData { .. }
    ));
}

#[test]
fn agent_profile_upsert_atomic_and_rejects_immutable_created_at_and_invalid_timestamps() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("agentprofileatom");

    // updated_at < created_at fails with InvalidEntityData
    let invalid_ts = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Agent").expect("name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Off,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
    };
    let err_ts = store.upsert_agent_profile(&invalid_ts).unwrap_err();
    assert!(matches!(err_ts, StorageError::InvalidEntityData { .. }));

    // Initial upsert creates profile
    let mut profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Agent Initial").expect("name"),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Off,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store.upsert_agent_profile(&profile).expect("upsert create");

    let fetched = store
        .agent_profile_by_id(&agp_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched.display_name.as_str(), "Agent Initial");

    // Upsert updates display_name, preferred_harness, memory_mode, updated_at
    profile.display_name = DisplayName::try_from("Agent Modified").expect("name");
    profile.preferred_harness = HarnessKind::Native;
    profile.memory_mode = MemoryMode::LongTerm;
    profile.updated_at = UnixMillis::from_millis(BASE_MILLIS + 200);
    store.upsert_agent_profile(&profile).expect("upsert update");

    let fetched_up = store
        .agent_profile_by_id(&agp_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched_up.display_name.as_str(), "Agent Modified");
    assert_eq!(fetched_up.preferred_harness, HarnessKind::Native);
    assert_eq!(fetched_up.memory_mode, MemoryMode::LongTerm);
    assert_eq!(
        fetched_up.updated_at,
        UnixMillis::from_millis(BASE_MILLIS + 200)
    );

    // Upsert attempting to change created_at fails with InvalidEntityData
    let mut bad_created = profile.clone();
    bad_created.created_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err_created = store.upsert_agent_profile(&bad_created).unwrap_err();
    assert!(matches!(
        err_created,
        StorageError::InvalidEntityData { .. }
    ));
}

#[test]
fn project_ref_upsert_and_delete_atomic() {
    let mut store = Store::open_in_memory().expect("store");
    let prj_id = project_id("projectatomic001");

    // Initial upsert creates project ref
    let mut project = ProjectRef {
        id: prj_id.clone(),
        label: BoundedLabel::try_from("Project Init").expect("label"),
        path: BoundedPath::try_from("/path/init").expect("path"),
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store.upsert_project_ref(&project).expect("upsert create");

    let fetched = store
        .project_ref_by_id(&prj_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched.label.as_str(), "Project Init");
    assert_eq!(fetched.path.as_str(), "/path/init");

    // Upsert updates label and path
    project.label = BoundedLabel::try_from("Project Mod").expect("label");
    project.path = BoundedPath::try_from("/path/mod").expect("path");
    store.upsert_project_ref(&project).expect("upsert update");

    let fetched_up = store
        .project_ref_by_id(&prj_id)
        .expect("get")
        .expect("found");
    assert_eq!(fetched_up.label.as_str(), "Project Mod");
    assert_eq!(fetched_up.path.as_str(), "/path/mod");

    // Upsert attempting to change created_at fails with InvalidEntityData
    let mut bad_created = project.clone();
    bad_created.created_at = UnixMillis::from_millis(BASE_MILLIS + 999);
    let err_created = store.upsert_project_ref(&bad_created).unwrap_err();
    assert!(matches!(
        err_created,
        StorageError::InvalidEntityData { .. }
    ));

    // Delete unreferenced project succeeds and returns true
    assert!(store.delete_project_ref(&prj_id).expect("delete"));

    // Delete non-existent project returns false
    assert!(
        !store
            .delete_project_ref(&prj_id)
            .expect("delete non-existent")
    );

    // Recreate project, then create thread referencing it
    store.create_project_ref(&project).expect("recreate");
    let agp = ensure_agent_profile(&mut store, "claude");

    store
        .append_domain_event(&DomainEvent {
            event_id: event_id(1),
            thread_id: Some(thread_id("prjrefthread0001")),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::ThreadCreated,
            payload: json_payload(&[
                ("agent_profile_id", agp.as_str()),
                ("project_id", prj_id.as_str()),
            ]),
            occurred_at: UnixMillis::from_millis(BASE_MILLIS + 200),
        })
        .expect("create thread with project ref");

    // Delete referenced project fails with ProjectReferencedByThreads
    let err_ref = store.delete_project_ref(&prj_id).unwrap_err();
    assert!(matches!(
        err_ref,
        StorageError::ProjectReferencedByThreads { .. }
    ));
}

#[test]
fn harness_binding_launch_params_roundtrip_and_upsert_update() {
    let mut store = Store::open_in_memory().expect("store");
    let agp_id = agent_id("agentroundtrip01");

    store
        .create_agent_profile(&AgentProfile {
            id: agp_id.clone(),
            display_name: DisplayName::try_from("Roundtrip Agent").expect("name"),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::Off,
            created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
            updated_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        })
        .expect("create agent");

    let hnb_id = binding_id("bindingroundtrip");
    let initial_binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Initial Harness").expect("label"),
        command: BoundedPath::try_from("/usr/local/bin/agent-harness").expect("cmd"),
        args: vec![
            HarnessArg::try_from("--port=8080").expect("arg"),
            HarnessArg::try_from("--log-level=debug").expect("arg"),
        ],
        env_keys: vec![
            HarnessEnvKey::try_from("API_SECRET_KEY").expect("env"),
            HarnessEnvKey::try_from("HOST_IP").expect("env"),
        ],
        secret_refs: vec![
            HarnessSecretRef::try_from("vault:credentials:key1").expect("sec"),
            HarnessSecretRef::try_from("vault:credentials:key2").expect("sec"),
        ],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    // Create binding
    store
        .create_harness_binding(&initial_binding)
        .expect("create harness binding");

    // Fetch by ID
    let fetched = store
        .harness_binding_by_id(&hnb_id)
        .expect("query")
        .expect("must exist");
    assert_eq!(fetched, initial_binding);
    assert_eq!(fetched.args.len(), 2);
    assert_eq!(fetched.args[0].as_str(), "--port=8080");
    assert_eq!(fetched.args[1].as_str(), "--log-level=debug");
    assert_eq!(fetched.env_keys.len(), 2);
    assert_eq!(fetched.env_keys[0].as_str(), "API_SECRET_KEY");
    assert_eq!(fetched.env_keys[1].as_str(), "HOST_IP");
    assert_eq!(fetched.secret_refs.len(), 2);
    assert_eq!(fetched.secret_refs[0].as_str(), "vault:credentials:key1");
    assert_eq!(fetched.secret_refs[1].as_str(), "vault:credentials:key2");

    // Fetch via list
    let list = store
        .harness_bindings_for_agent(
            &agp_id,
            None,
            HarnessBindingListLimit::try_new(10).expect("limit"),
        )
        .expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0], initial_binding);

    // Idempotent create with exact same content succeeds
    store
        .create_harness_binding(&initial_binding)
        .expect("idempotent create");

    // Conflicting create fails
    let mut conflict = initial_binding.clone();
    conflict.args = vec![HarnessArg::try_from("--different").expect("arg")];
    let err_conflict = store.create_harness_binding(&conflict).unwrap_err();
    assert!(matches!(
        err_conflict,
        StorageError::HarnessBindingAlreadyExists { .. }
    ));

    // Upsert update changes args, env_keys, secret_refs, label, command
    let updated_binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Updated Harness").expect("label"),
        command: BoundedPath::try_from("/usr/local/bin/agent-harness-v2").expect("cmd"),
        args: vec![HarnessArg::try_from("--production").expect("arg")],
        env_keys: vec![HarnessEnvKey::try_from("PROD_ENV").expect("env")],
        secret_refs: vec![HarnessSecretRef::try_from("vault:prod:sec").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store
        .upsert_harness_binding(&updated_binding)
        .expect("upsert update");

    let fetched_updated = store
        .harness_binding_by_id(&hnb_id)
        .expect("query")
        .expect("must exist");
    assert_eq!(fetched_updated, updated_binding);
}

#[test]
fn harness_binding_launch_params_reopen_equivalence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hnb_reopen.db");

    let agp_id = agent_id("agentlaunchreopen");
    let hnb_id = binding_id("bindingreopen01");
    let binding = AcpHarnessBinding {
        id: hnb_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Persistent Harness").expect("label"),
        command: BoundedPath::try_from("/opt/agent/bin").expect("cmd"),
        args: vec![
            HarnessArg::try_from("--arg1").expect("arg"),
            HarnessArg::try_from("--arg2").expect("arg"),
        ],
        env_keys: vec![
            HarnessEnvKey::try_from("ENV_ONE").expect("env"),
            HarnessEnvKey::try_from("ENV_TWO").expect("env"),
        ],
        secret_refs: vec![
            HarnessSecretRef::try_from("vault:ref1").expect("sec"),
            HarnessSecretRef::try_from("vault:ref2").expect("sec"),
        ],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };

    {
        let mut store = Store::open(&path).expect("open");
        store
            .create_agent_profile(&AgentProfile {
                id: agp_id.clone(),
                display_name: DisplayName::try_from("Reopen Agent").expect("name"),
                preferred_harness: HarnessKind::Acp,
                memory_mode: MemoryMode::Off,
                created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
                updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
            })
            .expect("create agent");
        store
            .create_harness_binding(&binding)
            .expect("create binding");
    }

    let store = Store::open(&path).expect("reopen");
    let fetched = store
        .harness_binding_by_id(&hnb_id)
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched, binding);

    let list = store
        .harness_bindings_for_agent(
            &agp_id,
            None,
            HarnessBindingListLimit::try_new(5).expect("limit"),
        )
        .expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0], binding);
}

#[test]
#[allow(clippy::too_many_lines)]
fn harness_binding_v4_schema_migration_defaults_empty_vectors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("v4_migration_test.db");

    let legacy_agp_id = agent_id("v4legacyagent1");
    let legacy_hnb_id = binding_id("v4legacybinding1");

    // Manually create a schema v4 database using rusqlite
    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        // Apply V1
        raw.execute_batch(
            r"
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
        ",
        )
        .expect("v1");

        // Apply V2
        raw.execute_batch(
            r"
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
                created_at INTEGER NOT NULL
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
                rebuilt_at INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE thread_search USING fts5(
                thread_id UNINDEXED,
                title,
                content='thread',
                content_rowid='rowid'
            );
        ",
        )
        .expect("v2");

        // Apply V3
        raw.execute_batch(
            r"
            ALTER TABLE domain_projection_state ADD COLUMN projection_digest TEXT NOT NULL DEFAULT '';
        ",
        )
        .expect("v3");

        // Apply V4
        raw.execute_batch(
            r"
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
        ",
        )
        .expect("v4");

        raw.pragma_update(None, "user_version", 4)
            .expect("stamp v4");

        // Insert legacy data into v4 tables
        raw.execute(
            "INSERT INTO agent_profile (agent_profile_id, display_name, preferred_harness, memory_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                legacy_agp_id.as_str(),
                "Legacy Agent",
                "acp",
                "long_term",
                1_700_000_000_050_i64,
                1_700_000_000_050_i64,
            ],
        )
        .expect("insert agent");

        raw.execute(
            "INSERT INTO harness_binding (harness_binding_id, agent_profile_id, label, command, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                legacy_hnb_id.as_str(),
                legacy_agp_id.as_str(),
                "Legacy Binding",
                "/usr/bin/legacy-agent",
                1_700_000_000_100_i64,
            ],
        )
        .expect("insert harness binding");
    }

    // Now open with Store (which applies v5 migration)
    let mut store = Store::open(&path).expect("open and migrate to v5");
    assert_eq!(store.schema_version().expect("version"), 5);

    // Read back the legacy binding
    let legacy_binding = store
        .harness_binding_by_id(&legacy_hnb_id)
        .expect("get legacy binding")
        .expect("must exist");

    assert_eq!(legacy_binding.label.as_str(), "Legacy Binding");
    assert_eq!(legacy_binding.command.as_str(), "/usr/bin/legacy-agent");
    assert!(legacy_binding.args.is_empty());
    assert!(legacy_binding.env_keys.is_empty());
    assert!(legacy_binding.secret_refs.is_empty());

    // Create a new binding with non-empty parameters in migrated DB
    let new_hnb_id = binding_id("v5newbinding01");
    let new_binding = AcpHarnessBinding {
        id: new_hnb_id.clone(),
        agent_profile_id: legacy_agp_id.clone(),
        label: DisplayName::try_from("New Binding").expect("label"),
        command: BoundedPath::try_from("/usr/bin/new-agent").expect("cmd"),
        args: vec![HarnessArg::try_from("--v5").expect("arg")],
        env_keys: vec![HarnessEnvKey::try_from("V5_KEY").expect("env")],
        secret_refs: vec![HarnessSecretRef::try_from("vault:v5_secret").expect("sec")],
        created_at: UnixMillis::from_millis(BASE_MILLIS + 200),
    };
    store
        .create_harness_binding(&new_binding)
        .expect("create new");

    // List should return both legacy and new bindings
    let all = store
        .harness_bindings_for_agent(
            &legacy_agp_id,
            None,
            HarnessBindingListLimit::try_new(10).expect("limit"),
        )
        .expect("list");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, legacy_hnb_id);
    assert_eq!(all[1].id, new_hnb_id);
}

#[test]
fn harness_binding_corrupt_json_rejects_with_invalid_entity_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("corrupt_json_test.db");

    let agp_id = agent_id("corruptagent1");
    let hnb_id = binding_id("corruptbinding1");

    {
        let mut store = Store::open(&path).expect("open");
        store
            .create_agent_profile(&AgentProfile {
                id: agp_id.clone(),
                display_name: DisplayName::try_from("Corrupt Test Agent").expect("name"),
                preferred_harness: HarnessKind::Acp,
                memory_mode: MemoryMode::Off,
                created_at: UnixMillis::from_millis(BASE_MILLIS + 50),
                updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
            })
            .expect("create agent");

        let valid_binding = AcpHarnessBinding {
            id: hnb_id.clone(),
            agent_profile_id: agp_id.clone(),
            label: DisplayName::try_from("Valid Label").expect("label"),
            command: BoundedPath::try_from("/usr/bin/valid").expect("cmd"),
            args: vec![],
            env_keys: vec![],
            secret_refs: vec![],
            created_at: UnixMillis::from_millis(BASE_MILLIS + 100),
        };
        store
            .create_harness_binding(&valid_binding)
            .expect("create");
    }

    // Corrupt args_json with malformed JSON
    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        raw.execute(
            "UPDATE harness_binding SET args_json = '{not_valid_json' WHERE harness_binding_id = ?1",
            rusqlite::params![hnb_id.as_str()],
        )
        .expect("corrupt args_json");
    }

    let store = Store::open(&path).expect("reopen");
    let err = store.harness_binding_by_id(&hnb_id).unwrap_err();
    assert!(matches!(err, StorageError::InvalidEntityData { .. }));

    let err_list = store
        .harness_bindings_for_agent(
            &agp_id,
            None,
            HarnessBindingListLimit::try_new(10).expect("limit"),
        )
        .unwrap_err();
    assert!(matches!(err_list, StorageError::InvalidEntityData { .. }));

    // Corrupt env_keys_json with non-array JSON (e.g. integer or object)
    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        raw.execute(
            "UPDATE harness_binding SET args_json = '[]', env_keys_json = '{\"not\":\"an_array\"}' WHERE harness_binding_id = ?1",
            rusqlite::params![hnb_id.as_str()],
        )
        .expect("corrupt env_keys_json non-array");
    }

    let store = Store::open(&path).expect("reopen");
    let err = store.harness_binding_by_id(&hnb_id).unwrap_err();
    assert!(matches!(err, StorageError::InvalidEntityData { .. }));

    // Corrupt env_keys_json with invalid identifier
    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        raw.execute(
            "UPDATE harness_binding SET env_keys_json = '[\"123_INVALID_IDENTIFIER\"]' WHERE harness_binding_id = ?1",
            rusqlite::params![hnb_id.as_str()],
        )
        .expect("corrupt env_keys_json invalid identifier");
    }

    let store = Store::open(&path).expect("reopen");
    let err = store.harness_binding_by_id(&hnb_id).unwrap_err();
    assert!(matches!(err, StorageError::InvalidEntityData { .. }));

    // Corrupt secret_refs_json with control character
    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        raw.execute(
            "UPDATE harness_binding SET env_keys_json = '[]', secret_refs_json = '[\"invalid\\u0000secret\"]' WHERE harness_binding_id = ?1",
            rusqlite::params![hnb_id.as_str()],
        )
        .expect("corrupt secret_refs_json control char");
    }

    let store = Store::open(&path).expect("reopen");
    let err = store.harness_binding_by_id(&hnb_id).unwrap_err();
    assert!(matches!(err, StorageError::InvalidEntityData { .. }));
}
