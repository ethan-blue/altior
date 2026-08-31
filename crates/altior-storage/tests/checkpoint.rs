//! Integration tests for runtime checkpointing and harness session bindings (P1.2, ADR 0002, ADR 0013).

use std::str::FromStr;

use altior_domain::{
    AgentProfile, AgentProfileId, BoundaryKind, CheckpointListLimit, CheckpointState,
    DiagnosticSummary, DisplayName, DomainEvent, DomainEventKind, EventId, EventPayload,
    HarnessBindingId, HarnessKind, MemoryMode, OpaqueSessionId, OperationId, RemoteRequestId,
    RuntimeCheckpoint, RuntimeCheckpointId, SessionBinding, ThreadId, TurnId, UnixMillis,
};
use altior_storage::{StorageError, Store};

const BASE_MILLIS: u64 = 1_700_000_000_000;

fn sanitize(name: &str) -> String {
    let clean: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
    let lower = clean.to_ascii_lowercase();
    format!("{lower:0<16}")
}

fn thread_id(name: &str) -> ThreadId {
    let body = sanitize(name);
    ThreadId::from_str(&format!("thr_{body}")).expect("valid thread id")
}

fn turn_id(name: &str) -> TurnId {
    let body = sanitize(name);
    TurnId::from_str(&format!("trn_{body}")).expect("valid turn id")
}

fn op_id(name: &str) -> OperationId {
    let body = sanitize(name);
    OperationId::from_str(&format!("op_{body}")).expect("valid op id")
}

fn checkpoint_id(name: &str) -> RuntimeCheckpointId {
    let body = sanitize(name);
    RuntimeCheckpointId::from_str(&format!("chk_{body}")).expect("valid checkpoint id")
}

fn harness_binding_id(name: &str) -> HarnessBindingId {
    let body = sanitize(name);
    HarnessBindingId::from_str(&format!("hsb_{body}")).expect("valid harness binding id")
}

fn agent_profile_id(name: &str) -> AgentProfileId {
    let body = sanitize(name);
    AgentProfileId::from_str(&format!("agp_{body}")).expect("valid agent profile id")
}

fn event_id(n: u64) -> EventId {
    let padded = format!("{n:0<16}");
    EventId::from_str(&format!("evt_{padded}")).expect("valid event id")
}

fn ensure_thread_and_turn(store: &mut Store, thr: &ThreadId, trn: &TurnId) {
    let agp_id = agent_profile_id("default");
    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Default").unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Off,
        created_at: UnixMillis::from_millis(BASE_MILLIS),
        updated_at: UnixMillis::from_millis(BASE_MILLIS),
    };
    let _ = store.create_agent_profile(&profile);

    let create_event = DomainEvent {
        event_id: event_id(100),
        thread_id: Some(thr.clone()),
        turn_id: None,
        operation_id: None,
        kind: DomainEventKind::ThreadCreated,
        payload: EventPayload::try_from(
            format!(r#"{{"agent_profile_id":"{agp_id}","title":"Test"}}"#).as_bytes(),
        )
        .unwrap(),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + 1),
    };
    store.append_domain_event(&create_event).unwrap();

    let start_event = DomainEvent {
        event_id: event_id(101),
        thread_id: Some(thr.clone()),
        turn_id: Some(trn.clone()),
        operation_id: None,
        kind: DomainEventKind::TurnStarted,
        payload: EventPayload::try_from(b"{}".as_slice()).unwrap(),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + 2),
    };
    store.append_domain_event(&start_event).unwrap();
}

#[test]
fn record_intent_and_query_roundtrip() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr = thread_id("alpha");
    let op = op_id("alpha_op_1");
    let chk = checkpoint_id("alpha_chk_1");

    let checkpoint = RuntimeCheckpoint {
        id: chk.clone(),
        thread_id: thr.clone(),
        turn_id: None,
        operation_id: op.clone(),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        settled_at: None,
    };

    store
        .record_runtime_intent(&checkpoint)
        .expect("record intent");

    // Query by id
    let fetched = store
        .runtime_checkpoint_by_id(&chk)
        .expect("query by id")
        .expect("must exist");
    assert_eq!(fetched, checkpoint);

    // Query by operation
    let fetched_by_op = store
        .runtime_checkpoint_by_operation(&op)
        .expect("query by op")
        .expect("must exist");
    assert_eq!(fetched_by_op, checkpoint);

    // Query non-existent returns None
    let missing_chk = checkpoint_id("missing");
    assert!(
        store
            .runtime_checkpoint_by_id(&missing_chk)
            .unwrap()
            .is_none()
    );
    let missing_op = op_id("missing");
    assert!(
        store
            .runtime_checkpoint_by_operation(&missing_op)
            .unwrap()
            .is_none()
    );

    // Idempotent record with same parameters
    assert!(store.record_runtime_intent(&checkpoint).is_ok());

    // Conflict on different parameters with same ID
    let conflicting = RuntimeCheckpoint {
        operation_id: op_id("different_op"),
        ..checkpoint.clone()
    };
    let err = store.record_runtime_intent(&conflicting).unwrap_err();
    assert!(matches!(err, StorageError::CheckpointCollision { .. }));
}

#[test]
fn record_intent_with_turn_validates_turn_and_thread() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr = thread_id("turn_thr");
    let trn = turn_id("turn_trn");
    let op = op_id("turn_op");
    let chk = checkpoint_id("turn_chk");

    let checkpoint = RuntimeCheckpoint {
        id: chk.clone(),
        thread_id: thr.clone(),
        turn_id: Some(trn.clone()),
        operation_id: op,
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        settled_at: None,
    };

    // Turn does not exist yet -> TurnNotFound
    let err = store.record_runtime_intent(&checkpoint).unwrap_err();
    assert!(matches!(err, StorageError::TurnNotFound { .. }));

    // Create thread and turn
    ensure_thread_and_turn(&mut store, &thr, &trn);

    // Now recording intent succeeds
    store
        .record_runtime_intent(&checkpoint)
        .expect("record intent");

    // Thread mismatch with existing turn
    let other_thr = thread_id("other_thr");
    let mismatch_chk = checkpoint_id("mismatch_chk");
    let mismatch_checkpoint = RuntimeCheckpoint {
        id: mismatch_chk,
        thread_id: other_thr,
        turn_id: Some(trn),
        operation_id: op_id("other_op"),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 20),
        settled_at: None,
    };
    let err = store
        .record_runtime_intent(&mismatch_checkpoint)
        .unwrap_err();
    assert!(matches!(err, StorageError::TurnThreadMismatch { .. }));
}

#[test]
fn settle_runtime_checkpoint_lifecycle() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr = thread_id("settle_thr");
    let op = op_id("settle_op");
    let chk = checkpoint_id("settle_chk");

    let checkpoint = RuntimeCheckpoint {
        id: chk.clone(),
        thread_id: thr,
        turn_id: None,
        operation_id: op,
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        settled_at: None,
    };
    store.record_runtime_intent(&checkpoint).unwrap();

    // Settle to non-terminal state is refused
    let invalid_err = store
        .settle_runtime_checkpoint(
            &chk,
            CheckpointState::Intent,
            None,
            None,
            UnixMillis::from_millis(BASE_MILLIS + 20),
        )
        .unwrap_err();
    assert!(matches!(
        invalid_err,
        StorageError::InvalidCheckpointTransition { .. }
    ));

    // Settle to Confirmed
    let req_id = RemoteRequestId::try_from("req_12345").unwrap();
    let diag = DiagnosticSummary::try_from("success").unwrap();
    let settled_at = UnixMillis::from_millis(BASE_MILLIS + 20);

    store
        .settle_runtime_checkpoint(
            &chk,
            CheckpointState::Confirmed,
            Some(&req_id),
            Some(&diag),
            settled_at,
        )
        .expect("settle confirmed");

    let settled = store.runtime_checkpoint_by_id(&chk).unwrap().unwrap();
    assert_eq!(settled.state, CheckpointState::Confirmed);
    assert_eq!(settled.remote_request_id.as_ref(), Some(&req_id));
    assert_eq!(settled.diagnostic_summary.as_ref(), Some(&diag));
    assert_eq!(settled.settled_at, Some(settled_at));

    // Idempotent re-settle to same terminal state
    assert!(
        store
            .settle_runtime_checkpoint(
                &chk,
                CheckpointState::Confirmed,
                Some(&req_id),
                Some(&diag),
                settled_at,
            )
            .is_ok()
    );

    // Conflict error on settling to different terminal state
    let conflict_err = store
        .settle_runtime_checkpoint(&chk, CheckpointState::Rejected, None, None, settled_at)
        .unwrap_err();
    assert!(matches!(
        conflict_err,
        StorageError::CheckpointSettlementConflict { .. }
    ));

    // Settle on non-existent checkpoint returns CheckpointNotFound
    let missing_chk = checkpoint_id("not_found");
    let not_found_err = store
        .settle_runtime_checkpoint(
            &missing_chk,
            CheckpointState::Confirmed,
            None,
            None,
            settled_at,
        )
        .unwrap_err();
    assert!(matches!(
        not_found_err,
        StorageError::CheckpointNotFound { .. }
    ));
}

#[test]
fn recovery_of_unsettled_checkpoints() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr = thread_id("rec_thr");
    let chk1 = checkpoint_id("rec_chk_1");
    let chk2 = checkpoint_id("rec_chk_2");

    let cp1 = RuntimeCheckpoint {
        id: chk1.clone(),
        thread_id: thr.clone(),
        turn_id: None,
        operation_id: op_id("rec_op_1"),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        settled_at: None,
    };
    let cp2 = RuntimeCheckpoint {
        id: chk2.clone(),
        thread_id: thr,
        turn_id: None,
        operation_id: op_id("rec_op_2"),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 20),
        settled_at: None,
    };

    store.record_runtime_intent(&cp1).unwrap();
    store.record_runtime_intent(&cp2).unwrap();

    // Settle cp1
    store
        .settle_runtime_checkpoint(
            &chk1,
            CheckpointState::Confirmed,
            None,
            None,
            UnixMillis::from_millis(BASE_MILLIS + 30),
        )
        .unwrap();

    // Recover unsettled checkpoints
    let recovered_count = store.recover_unsettled_checkpoints().expect("recover");
    assert_eq!(recovered_count, 1);

    let chk1_after = store.runtime_checkpoint_by_id(&chk1).unwrap().unwrap();
    assert_eq!(chk1_after.state, CheckpointState::Confirmed);

    let chk2_after = store.runtime_checkpoint_by_id(&chk2).unwrap().unwrap();
    assert_eq!(chk2_after.state, CheckpointState::Indeterminate);
    assert_eq!(chk2_after.settled_at, Some(cp2.created_at));
}

#[test]
fn list_and_filter_checkpoints() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr1 = thread_id("list_thr_1");
    let thr2 = thread_id("list_thr_2");

    let cp1 = RuntimeCheckpoint {
        id: checkpoint_id("list_chk_1"),
        thread_id: thr1.clone(),
        turn_id: None,
        operation_id: op_id("list_op_1"),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 10),
        settled_at: None,
    };
    let cp2 = RuntimeCheckpoint {
        id: checkpoint_id("list_chk_2"),
        thread_id: thr2,
        turn_id: None,
        operation_id: op_id("list_op_2"),
        boundary_kind: BoundaryKind::Prompt,
        state: CheckpointState::Intent,
        remote_request_id: None,
        diagnostic_summary: None,
        created_at: UnixMillis::from_millis(BASE_MILLIS + 20),
        settled_at: None,
    };

    store.record_runtime_intent(&cp1).unwrap();
    store.record_runtime_intent(&cp2).unwrap();

    let limit = CheckpointListLimit::try_new(10).unwrap();
    let all = store.runtime_checkpoints(None, None, limit).unwrap();
    assert_eq!(all.len(), 2);
    // Ordered by created_at DESC
    assert_eq!(all[0].id, cp2.id);
    assert_eq!(all[1].id, cp1.id);

    let filtered_thr1 = store.runtime_checkpoints(Some(&thr1), None, limit).unwrap();
    assert_eq!(filtered_thr1.len(), 1);
    assert_eq!(filtered_thr1[0].id, cp1.id);

    let active = store.active_checkpoints(None).unwrap();
    assert_eq!(active.len(), 2);
}

#[test]
fn session_binding_crud() {
    let mut store = Store::open_in_memory().expect("open store");
    let thr = thread_id("sb_thr");
    let hsb = harness_binding_id("sb_hsb");
    let sid = OpaqueSessionId::try_from("sess_abc123").unwrap();

    // Query non-existent returns None
    assert!(store.get_session_binding(&thr).unwrap().is_none());

    let binding = SessionBinding {
        thread_id: thr.clone(),
        harness_binding_id: hsb.clone(),
        opaque_session_id: sid.clone(),
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 50),
    };

    store
        .replace_session_binding(&binding)
        .expect("replace binding");

    let fetched = store
        .get_session_binding(&thr)
        .unwrap()
        .expect("must exist");
    assert_eq!(fetched, binding);

    // Update existing binding
    let updated_sid = OpaqueSessionId::try_from("sess_updated").unwrap();
    let updated_binding = SessionBinding {
        thread_id: thr.clone(),
        harness_binding_id: hsb,
        opaque_session_id: updated_sid,
        updated_at: UnixMillis::from_millis(BASE_MILLIS + 100),
    };
    store.replace_session_binding(&updated_binding).unwrap();
    let fetched_updated = store.get_session_binding(&thr).unwrap().unwrap();
    assert_eq!(fetched_updated, updated_binding);

    // Remove binding
    let removed = store.remove_session_binding(&thr).unwrap();
    assert!(removed);
    assert!(store.get_session_binding(&thr).unwrap().is_none());

    // Removing non-existent returns false
    let removed_again = store.remove_session_binding(&thr).unwrap();
    assert!(!removed_again);
}
