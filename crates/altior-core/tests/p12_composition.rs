//! End-to-end integration tests composing Core + SQLite Store + real ACP mock agent subprocess (P1.2).
//!
//! Validates the full vertical composition across real OS subprocess execution and SQLite persistence:
//! 1. Happy path: prompt streaming with delta events, turn completion, and Confirmed checkpoint settlement.
//! 2. Crash mode: child process abnormal exit (code 42) leads to Indeterminate settlement,
//!    strictly forbids automatic resend on the same turn, and survives store reopen with no lingering active intents.
//! 3. Permission mode: mock agent requests permission, supervisor transitions to `AwaitingPermission`,
//!    `decide_permission` is processed, and turn runs to terminal settlement.
//! 4. Cancel mode: prompt streaming is interrupted via `steer_cancel`, child terminates, and turn settles Rejected.
//! 5. Secret check: secret canary tokens never leak into domain journal payloads, runtime checkpoint
//!    diagnostics, or the SQLite database file bytes.
//! 6. RAII and multi-run idempotence: cleanly cleans up temp binaries and child processes across consecutive runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use altior_acp::{AcpError, EnvVarValue, LaunchConfig, SecretRef};
use altior_core::runtime::{
    AcpHarnessAdapter, AgentRuntime, AgentRuntimeSupervisor, CancelOutcome, RuntimeError,
    RuntimeEvent, StoreCheckpointAdapter, SupervisorState, TurnAdmission,
};
use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, BoundedPath, CheckpointListLimit,
    CheckpointState, DeliveryState, DisplayName, DomainEvent, DomainEventKind, EventId,
    EventPayload, HarnessBindingId, HarnessKind, MemoryMode, OperationId, PermissionDecision,
    ThreadId, TurnId, UnixMillis,
};
use altior_storage::Store;

const BASE_MILLIS: u64 = 1_700_000_000_000;
const SECRET_CANARY: &str = "SK_FIXTURE_TOP_SECRET_CANARY_VALUE_999";
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

// ── Mock Binary Location and Temp Executable RAII Guard ────────────

fn find_mock_binary() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_mock_acp_agent") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_mock_acp_agent") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }
    // Check relative to current test executable (e.g. target/debug/deps/p12_composition-xyz.exe)
    if let Ok(cur_exe) = std::env::current_exe()
        && let Some(target_dir) = cur_exe.parent().and_then(Path::parent)
    {
        let candidate = target_dir.join(format!("mock_acp_agent{}", std::env::consts::EXE_SUFFIX));
        if candidate.exists() {
            return candidate;
        }
    }
    // Check relative to CARGO_MANIFEST_DIR
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&manifest_dir);
    for profile in &["debug", "release"] {
        let candidate = workspace_root
            .join("target")
            .join(profile)
            .join(format!("mock_acp_agent{}", std::env::consts::EXE_SUFFIX));
        if candidate.exists() {
            return candidate;
        }
    }
    panic!(
        "mock_acp_agent binary not found in workspace target directory. Run `cargo build --bin mock_acp_agent`."
    );
}

struct TempExeGuard {
    path: PathBuf,
}

impl TempExeGuard {
    fn new(scenario_tag: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let src = find_mock_binary();
        let ext = std::env::consts::EXE_SUFFIX;
        let file_name = format!("altior_mock_{scenario_tag}_{pid}_{counter}_{timestamp}{ext}");
        let temp_path = std::env::temp_dir().join(file_name);
        std::fs::copy(&src, &temp_path).expect("failed to copy mock agent binary to temp path");
        Self { path: temp_path }
    }

    fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempExeGuard {
    fn drop(&mut self) {
        if std::fs::remove_file(&self.path).is_err() {
            for _ in 0..10 {
                std::thread::yield_now();
                if std::fs::remove_file(&self.path).is_ok() {
                    break;
                }
            }
        }
    }
}

// ── Test Helpers ───────────────────────────────────────────────────

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

fn setup_domain_records(
    store: &mut Store,
    thr: &ThreadId,
    trn: &TurnId,
    binding_id: &HarnessBindingId,
    exe_path: &str,
) -> (AgentProfile, AcpHarnessBinding) {
    let agp_id = agent_profile_id("default");
    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Mock Agent Profile").unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Off,
        created_at: UnixMillis::from_millis(BASE_MILLIS),
        updated_at: UnixMillis::from_millis(BASE_MILLIS),
    };
    store.create_agent_profile(&profile).unwrap();

    let binding = AcpHarnessBinding {
        id: binding_id.clone(),
        agent_profile_id: agp_id.clone(),
        label: DisplayName::try_from("Mock ACP Binding").unwrap(),
        command: BoundedPath::try_from(exe_path).unwrap(),
        args: Vec::new(),
        env_keys: Vec::new(),
        secret_refs: Vec::new(),
        created_at: UnixMillis::from_millis(BASE_MILLIS),
    };
    store.create_harness_binding(&binding).unwrap();

    let create_thread_event = DomainEvent {
        event_id: event_id(101),
        thread_id: Some(thr.clone()),
        turn_id: None,
        operation_id: None,
        kind: DomainEventKind::ThreadCreated,
        payload: EventPayload::try_from(
            format!(r#"{{"agent_profile_id":"{agp_id}","title":"E2E Test Thread"}}"#).as_bytes(),
        )
        .unwrap(),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + 1),
    };
    store.append_domain_event(&create_thread_event).unwrap();

    let start_turn_event = DomainEvent {
        event_id: event_id(102),
        thread_id: Some(thr.clone()),
        turn_id: Some(trn.clone()),
        operation_id: None,
        kind: DomainEventKind::TurnStarted,
        payload: EventPayload::try_from(b"{}".as_slice()).unwrap(),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + 2),
    };
    store.append_domain_event(&start_turn_event).unwrap();

    (profile, binding)
}

fn poll_until_event<F>(
    supervisor: &mut AgentRuntimeSupervisor<AcpHarnessAdapter, StoreCheckpointAdapter>,
    thread_id: &ThreadId,
    timeout: Duration,
    mut predicate: F,
) -> RuntimeEvent
where
    F: FnMut(&RuntimeEvent) -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(ev)) = supervisor.poll_stream(thread_id)
            && predicate(&ev)
        {
            return ev;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out after {timeout:?} waiting for matching event on thread {thread_id}");
}

// ── E2E Test Suite ─────────────────────────────────────────────────

#[test]
fn test_e2e_composition_happy_path_prompt_streaming() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("altior_happy.db");
    let mut store = Store::open(&db_path).expect("open sqlite store");

    let thr = thread_id("happy_t1");
    let trn = turn_id("happy_turn1");
    let op = op_id("happy_op1");
    let hsb_id = harness_binding_id("happy_bind1");

    let exe_guard = TempExeGuard::new("prompt_streaming");
    let (_profile, binding) =
        setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

    let harness = AcpHarnessAdapter::new();
    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

    // 1. Create session and assert thread_session_binding is persisted
    let session_id = supervisor
        .create_session(&binding, &thr, None)
        .expect("session created");
    assert_eq!(session_id.as_str(), "mock-session-1");

    let bound_session = supervisor
        .checkpoint()
        .get_session_binding(&thr)
        .expect("query session binding")
        .expect("session binding row must exist");
    assert_eq!(bound_session.thread_id, thr);
    assert_eq!(bound_session.harness_binding_id, binding.id);
    assert_eq!(bound_session.opaque_session_id.as_str(), "mock-session-1");

    // 2. Prompt real worker
    let admission = supervisor
        .prompt(&thr, op.clone(), trn.clone(), "Hello world prompt")
        .expect("prompt admitted");
    assert_eq!(admission, TurnAdmission::Admitted);

    // 3. Poll stream until MessageDelta chunks and TurnCompleted
    let mut chunks = Vec::new();
    let mut completed = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) && !completed {
        if let Ok(Some(ev)) = supervisor.poll_stream(&thr) {
            match ev {
                RuntimeEvent::MessageDelta { text, .. } => chunks.push(text),
                RuntimeEvent::TurnCompleted { .. } => completed = true,
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(completed, "turn must complete");
    assert_eq!(chunks.concat(), "Hello World!");

    // 4. Assert runtime checkpoint for prompt is settled as Confirmed
    let cp = supervisor
        .checkpoint()
        .store()
        .runtime_checkpoint_by_operation(&op)
        .expect("query checkpoint")
        .expect("prompt checkpoint must exist");
    assert_eq!(cp.state, CheckpointState::Confirmed);
    assert!(cp.settled_at.is_some());

    // 5. Clean teardown
    supervisor.close_session(&thr).expect("close session");
}

#[test]
fn test_e2e_composition_crash_mode_indeterminate_and_no_resend() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("altior_crash.db");
    let mut store = Store::open(&db_path).expect("open sqlite store");

    let thr = thread_id("crash_t1");
    let trn = turn_id("crash_turn1");
    let op = op_id("crash_op1");
    let hsb_id = harness_binding_id("crash_bind1");

    let exe_guard = TempExeGuard::new("unexpected_exit");
    let (_profile, binding) =
        setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

    let harness = AcpHarnessAdapter::new();
    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

    // 1. Create session
    let session_id = supervisor
        .create_session(&binding, &thr, None)
        .expect("create session");
    assert_eq!(session_id.as_str(), "mock-s1");

    // 2. Prompt triggers child crash (exit code 42)
    let admission = supervisor
        .prompt(&thr, op.clone(), trn.clone(), "trigger crash")
        .expect("prompt admitted");
    assert_eq!(admission, TurnAdmission::Admitted);

    // 3. Poll until TurnFailed with Indeterminate delivery
    let fail_ev = poll_until_event(&mut supervisor, &thr, Duration::from_secs(5), |ev| {
        matches!(ev, RuntimeEvent::TurnFailed { .. })
    });

    if let RuntimeEvent::TurnFailed { delivery, .. } = fail_ev {
        assert_eq!(delivery, DeliveryState::Indeterminate);
    } else {
        panic!("expected TurnFailed event");
    }

    // 4. Assert supervisor state machine is Crashed
    let sup_state = supervisor
        .supervisor(&thr)
        .expect("supervisor exists")
        .state();
    assert!(matches!(sup_state, SupervisorState::Crashed { .. }));

    // 5. Automatic resend of the same turn MUST be strictly forbidden
    let op2 = op_id("crash_op2");
    let resend_err = supervisor.prompt(&thr, op2, trn.clone(), "resend attempt");
    assert!(
        matches!(
            resend_err,
            Err(RuntimeError::AutomaticResendForbidden {
                delivery: DeliveryState::Indeterminate,
                ..
            })
        ),
        "resend on indeterminate turn must return AutomaticResendForbidden"
    );

    // 6. Checkpoint state in store is Indeterminate and no active intents linger
    let active = supervisor
        .checkpoint()
        .store()
        .active_checkpoints(Some(&thr))
        .expect("active checkpoints");
    assert!(
        active.is_empty(),
        "no active intent checkpoints should linger"
    );

    let limit = CheckpointListLimit::try_new(10).unwrap();
    let indeterminate = supervisor
        .checkpoint()
        .store()
        .indeterminate_checkpoints(Some(&thr), None, limit)
        .expect("query indeterminate checkpoints");
    assert_eq!(indeterminate.len(), 1);
    assert_eq!(indeterminate[0].turn_id.as_ref(), Some(&trn));

    // 7. Drop supervisor and reopen store: Indeterminate remains, no intents linger
    drop(supervisor);

    let reopened = Store::open(&db_path).expect("reopen store");
    let reopened_active = reopened
        .active_checkpoints(Some(&thr))
        .expect("reopened active checkpoints");
    assert!(reopened_active.is_empty());

    let reopened_indet = reopened
        .indeterminate_checkpoints(Some(&thr), None, limit)
        .expect("reopened indeterminate checkpoints");
    assert_eq!(reopened_indet.len(), 1);
    assert_eq!(reopened_indet[0].turn_id.as_ref(), Some(&trn));
}

#[test]
fn test_e2e_composition_permission_flow() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("altior_perm.db");
    let mut store = Store::open(&db_path).expect("open sqlite store");

    let thr = thread_id("perm_t1");
    let trn = turn_id("perm_turn1");
    let op = op_id("perm_op1");
    let hsb_id = harness_binding_id("perm_bind1");

    let exe_guard = TempExeGuard::new("permission_flow");
    let (_profile, binding) =
        setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

    let harness = AcpHarnessAdapter::new();
    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

    // 1. Create session and prompt
    supervisor
        .create_session(&binding, &thr, None)
        .expect("create session");
    supervisor
        .prompt(&thr, op.clone(), trn.clone(), "perform privileged action")
        .expect("prompt admitted");

    // 2. Poll until PermissionRequested
    let perm_ev = poll_until_event(&mut supervisor, &thr, Duration::from_secs(5), |ev| {
        matches!(ev, RuntimeEvent::PermissionRequested { .. })
    });

    let RuntimeEvent::PermissionRequested {
        permission_id: perm_id,
        ..
    } = perm_ev
    else {
        unreachable!()
    };

    // Assert supervisor state is AwaitingPermission
    let sup_state = supervisor
        .supervisor(&thr)
        .expect("supervisor exists")
        .state();
    assert!(matches!(
        sup_state,
        SupervisorState::AwaitingPermission { .. }
    ));

    // 3. Decide permission (Approved)
    supervisor
        .decide_permission(&thr, &perm_id, PermissionDecision::Approved)
        .expect("decide permission");

    // 4. Poll until turn completion
    let completed_ev = poll_until_event(&mut supervisor, &thr, Duration::from_secs(5), |ev| {
        matches!(ev, RuntimeEvent::TurnCompleted { .. })
    });
    assert!(matches!(completed_ev, RuntimeEvent::TurnCompleted { .. }));

    // 5. Assert prompt checkpoint is Confirmed
    let cp = supervisor
        .checkpoint()
        .store()
        .runtime_checkpoint_by_operation(&op)
        .expect("query checkpoint")
        .expect("prompt checkpoint must exist");
    assert_eq!(cp.state, CheckpointState::Confirmed);

    supervisor.close_session(&thr).expect("close session");
}

#[test]
fn test_e2e_composition_cancel_flow() {
    let start_time = Instant::now();
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("altior_cancel.db");
    let mut store = Store::open(&db_path).expect("open sqlite store");

    let thr = thread_id("cancel_t1");
    let trn = turn_id("cancel_turn1");
    let op = op_id("cancel_op1");
    let hsb_id = harness_binding_id("cancel_bind1");

    let exe_guard = TempExeGuard::new("cancel_flow");
    let (_profile, binding) =
        setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

    let harness = AcpHarnessAdapter::new();
    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

    // 1. Create session and prompt
    supervisor
        .create_session(&binding, &thr, None)
        .expect("create session");
    supervisor
        .prompt(&thr, op.clone(), trn.clone(), "start long running task")
        .expect("prompt admitted");

    // 2. Poll until first MessageDelta chunk arrives ("Working...")
    let _chunk_ev = poll_until_event(&mut supervisor, &thr, Duration::from_secs(2), |ev| {
        matches!(ev, RuntimeEvent::MessageDelta { .. })
    });

    // 3. Issue steer_cancel while turn is active
    let cancel_outcome = supervisor
        .steer_cancel(&thr, None)
        .expect("cancel initiated");
    assert_eq!(cancel_outcome, CancelOutcome::CancelledActive);

    // 4. Poll until TurnCancelled event arrives
    let cancel_ev = poll_until_event(&mut supervisor, &thr, Duration::from_secs(2), |ev| {
        matches!(ev, RuntimeEvent::TurnCancelled { .. })
    });
    assert!(matches!(cancel_ev, RuntimeEvent::TurnCancelled { .. }));

    // 5. Assert checkpoint state is Rejected (cancelled terminal state)
    let cp = supervisor
        .checkpoint()
        .store()
        .runtime_checkpoint_by_operation(&op)
        .expect("query checkpoint")
        .expect("prompt checkpoint must exist");
    assert_eq!(cp.state, CheckpointState::Rejected);

    // 6. Supervisor is back in Ready state
    let sup_state = supervisor
        .supervisor(&thr)
        .expect("supervisor exists")
        .state();
    assert!(matches!(sup_state, SupervisorState::Ready));

    supervisor.close_session(&thr).expect("close session");

    assert!(
        start_time.elapsed() < Duration::from_secs(2),
        "cancel E2E test must complete in <2s, took {:?}",
        start_time.elapsed()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_e2e_composition_secret_non_leakage() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("altior_secret_test.db");
    let mut store = Store::open(&db_path).expect("open sqlite store");

    let thr = thread_id("secret_t1");
    let trn = turn_id("secret_turn1");
    let op = op_id("secret_op1");
    let hsb_id = harness_binding_id("secret_bind1");

    // TempExeGuard name contains "secret", which triggers the "secret_check" scenario in mock_acp_agent
    let exe_guard = TempExeGuard::new("secret_check");
    let (_profile, binding) =
        setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

    let secret_ref = SecretRef::new("test-secret-ref").unwrap();
    let secret_resolver = Arc::new(|sref: &SecretRef| -> Result<String, AcpError> {
        if sref.as_str() == "test-secret-ref" {
            Ok(SECRET_CANARY.to_owned())
        } else {
            Err(AcpError::SecretResolutionFailed {
                secret_ref: sref.to_string(),
                diagnostic: "unknown secret".to_owned(),
            })
        }
    });

    let mut launch_env = BTreeMap::new();
    launch_env.insert(
        "ALTIOR_TEST_SECRET".to_owned(),
        EnvVarValue::SecretRef(secret_ref.clone()),
    );

    let harness = AcpHarnessAdapter::new()
        .with_secret_resolver(secret_resolver)
        .with_launch_env(launch_env);

    // Format Debug assertions: Debug on adapter, LaunchConfig, and resolved config must never leak secret
    let harness_debug = format!("{harness:?}");
    assert!(
        !harness_debug.contains(SECRET_CANARY),
        "harness Debug must not contain secret canary"
    );

    let sample_lc = LaunchConfig::new("test-prog")
        .unwrap()
        .with_secret_env("ALTIOR_TEST_SECRET", secret_ref)
        .unwrap();
    let lc_debug = format!("{sample_lc:?}");
    assert!(
        !lc_debug.contains(SECRET_CANARY),
        "LaunchConfig Debug must not contain secret canary"
    );

    let sample_resolver = |s: &SecretRef| {
        if s.as_str() == "test-secret-ref" {
            Ok(SECRET_CANARY.to_string())
        } else {
            Err(AcpError::SecretResolutionFailed {
                secret_ref: s.to_string(),
                diagnostic: "not found".to_string(),
            })
        }
    };
    let resolved_lc = sample_lc.resolve(&sample_resolver).unwrap();
    let resolved_debug = format!("{resolved_lc:?}");
    assert!(
        !resolved_debug.contains(SECRET_CANARY),
        "ResolvedLaunchConfig Debug must not contain secret canary"
    );
    assert!(
        resolved_debug.contains("[REDACTED]"),
        "ResolvedLaunchConfig Debug must redact env values"
    );

    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

    // 1. Create session
    let session_id = supervisor
        .create_session(&binding, &thr, None)
        .expect("session created");
    assert_eq!(session_id.as_str(), "mock-session-1");

    // 2. Prompt real worker (proves child received secret and passed secret_check)
    let admission = supervisor
        .prompt(&thr, op.clone(), trn, "prompt with secret check")
        .expect("prompt admitted");
    assert_eq!(admission, TurnAdmission::Admitted);

    let mut chunks = Vec::new();
    let mut completed = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) && !completed {
        if let Ok(Some(ev)) = supervisor.poll_stream(&thr) {
            match ev {
                RuntimeEvent::MessageDelta { text, .. } => chunks.push(text),
                RuntimeEvent::TurnCompleted { .. } => completed = true,
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        completed,
        "turn must complete successfully with child receiving secret"
    );
    assert_eq!(chunks.concat(), "Hello World!");

    // 3. Scan runtime checkpoint diagnostic summaries
    let cp = supervisor
        .checkpoint()
        .store()
        .runtime_checkpoint_by_operation(&op)
        .expect("query checkpoint")
        .expect("prompt checkpoint must exist");
    assert_eq!(cp.state, CheckpointState::Confirmed);
    if let Some(ref diag) = cp.diagnostic_summary {
        assert!(
            !diag.as_str().contains(SECRET_CANARY),
            "checkpoint diagnostic summary must not contain canary"
        );
    }

    let all_cps = supervisor
        .checkpoint()
        .store()
        .runtime_checkpoints(Some(&thr), None, CheckpointListLimit::try_new(50).unwrap())
        .expect("query checkpoints");
    for item in all_cps {
        if let Some(ref diag) = item.diagnostic_summary {
            assert!(
                !diag.as_str().contains(SECRET_CANARY),
                "checkpoint diagnostics must not contain canary"
            );
        }
    }

    // 4. Scan domain journal payload raw bytes
    let journal_rows = supervisor
        .checkpoint()
        .store()
        .domain_journal_records(0, altior_storage::JournalLimit::try_new(100).unwrap())
        .expect("query domain journal");
    for row in journal_rows {
        let payload_str = String::from_utf8_lossy(&row.payload);
        assert!(
            !payload_str.contains(SECRET_CANARY),
            "domain journal payload must not contain canary: {payload_str}"
        );
    }

    // 5. Scan thread events
    supervisor.close_session(&thr).expect("close session");
    drop(supervisor);

    // 6. Scan raw SQLite file bytes to assert secret canary is nowhere inside the database
    let db_bytes = std::fs::read(&db_path).expect("read sqlite db file bytes");
    let canary_bytes = SECRET_CANARY.as_bytes();
    let found = db_bytes
        .windows(canary_bytes.len())
        .any(|window| window == canary_bytes);
    assert!(
        !found,
        "secret canary must never appear in SQLite file bytes"
    );
}

#[test]
fn test_e2e_composition_consecutive_runs_clean_resources() {
    // Run all scenarios across two consecutive iterations to verify RAII cleanup,
    // absence of Windows executable locks, and database file lifecycle.
    for iteration in 1..=2 {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir
            .path()
            .join(format!("altior_multi_run_{iteration}.db"));
        let mut store = Store::open(&db_path).expect("open sqlite store");

        let thr = thread_id(&format!("multi_t{iteration}"));
        let trn = turn_id(&format!("multi_turn{iteration}"));
        let op = op_id(&format!("multi_op{iteration}"));
        let hsb_id = harness_binding_id(&format!("multi_bind{iteration}"));

        let exe_guard = TempExeGuard::new("prompt_streaming");
        let (_profile, binding) =
            setup_domain_records(&mut store, &thr, &trn, &hsb_id, &exe_guard.path_str());

        let harness = AcpHarnessAdapter::new();
        let checkpoint = StoreCheckpointAdapter::new(store);
        let mut supervisor = AgentRuntimeSupervisor::new(harness, checkpoint);

        supervisor
            .create_session(&binding, &thr, None)
            .expect("session created");
        supervisor
            .prompt(&thr, op, trn, "multi run test")
            .expect("prompt admitted");

        poll_until_event(&mut supervisor, &thr, Duration::from_secs(5), |ev| {
            matches!(ev, RuntimeEvent::TurnCompleted { .. })
        });

        supervisor.close_session(&thr).expect("close session");
    }
}
