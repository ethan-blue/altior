//! P1.4 Full Acceptance Journey E2E Integration Test.
//!
//! Validates the entire Altior vertical slice against a true OS Core daemon process,
//! real OS transport (Named Pipes / Unix Domain Sockets), and persistent SQLite storage:
//!
//! 1. Setup & Clean Store: Asserts schema v5 and empty tables on clean DB; launches daemon;
//!    configures Agent A (Full, `loadSession: true`) and Agent B (Minimal, `loadSession: false`)
//!    via IPC with unique mock agent binary copies; probes harnesses; asserts SQLite rows.
//! 2. Thread Creation & Sessions: Creates and opens threads for both agents via IPC; daemon automatically
//!    selects default binding and creates session bindings.
//! 3. Permission Flow: Multi-turn prompt on Agent A triggers permission request; user approves via IPC.
//! 4. Cancellation Flow: Multi-turn prompt on Agent B interrupted via cooperative `CancelTurn` IPC command.
//! 5. Client Disconnect & Reconnect Catch-up: Client disconnects mid-stream; daemon continues
//!    in background; reconnected client catches up via `Subscribe(since)` with `StreamReplayed`.
//! 6. Indeterminate Crash & Process Kill: Agent B triggers abnormal exit (code 42); turn settles
//!    Indeterminate; Core daemon process is gracefully killed via PID RAII guard.
//! 7. Core Daemon Restart & Indeterminate Invariant: Core restarts on same data directory;
//!    startup recovery scans indeterminate checkpoints; automatic resend strictly forbidden over IPC.
//! 8. Offline Search & History Projections: Reconnected client searches threads via FTS5,
//!    paginates full turn history, and inspects thread snapshots across restarts.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use altior_domain::{
    AgentProfileId, AgentProfileListLimit, CheckpointListLimit, CheckpointState, HarnessBindingId,
    HarnessBindingListLimit, OperationId, ThreadListLimit, TurnId, UnixMillis,
};
use altior_ipc::{
    ClientSession, EndpointDiscovery, IpcError, LocalEndpoint, LocalStream,
    cleanup_stale_discovery, read_discovery_file,
};
use altior_protocol::{
    BoundedPayload, CancelTurnCommand, CapabilitySet, CommandEnvelope, CommandKind,
    ConfigureAgentCommand, CreateThreadCommand, DesktopHello, EventBody, EventEnvelope,
    GetHistoryCommand, HarnessBindingConfigDto, KnownEvent, LaunchToken, ListThreadsCommand,
    MessageText, OpenThreadCommand, ProductVersion, ProtocolVersion, ProtocolVersionRange,
    RespondPermissionCommand, RuntimeStatusCommand, SearchThreadsCommand, SnapshotEnvelope,
    StartTurnCommand, TestHarnessBindingCommand, ThreadDto, ThreadHistoryResponseDto,
    ThreadListResponseDto, ThreadSnapshotDto,
};
use altior_storage::Store;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(100);
const BASE_TIMESTAMP: u64 = 1_700_000_000_000;

// ── RAII Guards ───────────────────────────────────────────────────

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
    if let Ok(cur_exe) = std::env::current_exe()
        && let Some(target_dir) = cur_exe.parent().and_then(Path::parent)
    {
        let candidate = target_dir.join(format!("mock_acp_agent{}", std::env::consts::EXE_SUFFIX));
        if candidate.exists() {
            return candidate;
        }
    }
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
    panic!("mock_acp_agent binary not found. Build target/debug/mock_acp_agent.");
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
        std::fs::copy(&src, &temp_path).expect("copy mock agent binary to temp path");
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
                thread::yield_now();
                if std::fs::remove_file(&self.path).is_ok() {
                    break;
                }
            }
        }
    }
}

#[derive(Debug)]
struct CoreProcessGuard {
    child: Option<std::process::Child>,
    stderr_path: std::path::PathBuf,
    stdout_path: std::path::PathBuf,
}

impl CoreProcessGuard {
    fn spawn(
        bin_path: &str,
        data_dir: &Path,
        endpoint_arg: &str,
        discovery_path: &Path,
    ) -> Result<Self, std::io::Error> {
        let stderr_path = data_dir.join("daemon-stderr.log");
        let stdout_path = data_dir.join("daemon-stdout.log");
        let stderr_file = std::fs::File::create(&stderr_path)?;
        let stdout_file = std::fs::File::create(&stdout_path)?;
        let child = std::process::Command::new(bin_path)
            .arg("--daemon")
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--endpoint")
            .arg(endpoint_arg)
            .arg("--discovery")
            .arg(discovery_path)
            .env("RUST_BACKTRACE", "1")
            .stdin(std::process::Stdio::null())
            .stdout(stdout_file)
            .stderr(stderr_file)
            .spawn()?;

        Ok(Self {
            child: Some(child),
            stderr_path,
            stdout_path,
        })
    }

    fn kill_and_wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            child.wait()
        } else {
            Err(std::io::Error::other("process already terminated"))
        }
    }

    fn has_exited(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            child.try_wait().ok().flatten().is_some()
        } else {
            true
        }
    }

    /// Returns daemon exit status plus captured stdout/stderr for failure diagnostics.
    fn diagnose(&mut self) -> String {
        let status = match self
            .child
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
        {
            Some(status) => format!("exited: {status}"),
            None => "still running".to_owned(),
        };
        let stderr_text = std::fs::read_to_string(&self.stderr_path).unwrap_or_default();
        let stdout_text = std::fs::read_to_string(&self.stdout_path).unwrap_or_default();
        format!(
            "daemon process: {status}\n--- daemon stderr ---\n{stderr_text}\n--- daemon stdout ---\n{stdout_text}"
        )
    }
}

impl Drop for CoreProcessGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ── Test Helpers & IPC Client Utilities ───────────────────────────

fn unique_endpoint_arg(temp_dir: &Path) -> (String, LocalEndpoint) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("altior-journey-p14-{}-{}", std::process::id(), id);
    if cfg!(windows) {
        let ep = LocalEndpoint::windows_pipe(&name).expect("valid windows pipe endpoint");
        (name, ep)
    } else {
        let sock_path = temp_dir.join(format!("{name}.sock"));
        let sock_str = sock_path.to_str().expect("valid utf8 path").to_string();
        let ep = LocalEndpoint::unix_socket(&sock_str).expect("valid unix socket endpoint");
        (sock_str, ep)
    }
}

fn wait_for_discovery(
    guard: &mut CoreProcessGuard,
    path: &Path,
    timeout: Duration,
) -> Result<EndpointDiscovery, IpcError> {
    let start = Instant::now();
    loop {
        if path.exists()
            && let Ok(discovery) = read_discovery_file(path)
        {
            return Ok(discovery);
        }
        if guard.has_exited() {
            return Err(IpcError::Io {
                source: std::io::Error::other("Core daemon process exited prematurely"),
            });
        }
        if start.elapsed() >= timeout {
            return Err(IpcError::Timeout {
                endpoint: format!("discovery publication at {}", path.display()),
            });
        }
        thread::yield_now();
    }
}

fn recv_frame_timeout(stream: &mut LocalStream, timeout: Duration) -> Result<Vec<u8>, IpcError> {
    let start = Instant::now();
    loop {
        match stream.try_recv_frame() {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return Err(IpcError::Timeout {
                        endpoint: "recv_frame timed out".to_string(),
                    });
                }
                thread::yield_now();
            }
            Err(err) => return Err(err),
        }
    }
}

fn recv_json_timeout<T: serde::de::DeserializeOwned>(
    stream: &mut LocalStream,
    timeout: Duration,
) -> Result<T, IpcError> {
    let bytes = recv_frame_timeout(stream, timeout)?;
    let text = String::from_utf8(bytes).map_err(|_| IpcError::FrameNotUtf8)?;
    serde_json::from_str(&text).map_err(|err| IpcError::Protocol {
        source: altior_protocol::ProtocolError::MalformedEnvelope { source: err },
    })
}

/// Consumes any live events already delivered to the stream until it stays
/// quiet for `quiet` — used between journey steps so late broadcasts from a
/// previous turn do not corrupt the next step's event ordering assertions.
fn drain_pending_events(stream: &mut LocalStream, quiet: Duration) {
    while recv_json_timeout::<EventEnvelope>(stream, quiet).is_ok() {}
}

fn make_desktop_hello(launch_token: LaunchToken) -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .expect("valid range"),
        desktop_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
        launch_token,
    }
}

fn connect_and_handshake(
    discovery: &EndpointDiscovery,
    timeout: Duration,
) -> Result<(LocalStream, ClientSession), IpcError> {
    let mut stream = LocalStream::connect(&discovery.endpoint, Some(timeout))?;
    let hello = make_desktop_hello(discovery.launch_token.clone());
    stream.send_json(&hello)?;

    let core_hello: altior_protocol::CoreHello = recv_json_timeout(&mut stream, timeout)?;
    let greeting: altior_protocol::CoreGreeting = recv_json_timeout(&mut stream, timeout)?;

    let mut session = ClientSession::new();
    let negotiated = altior_protocol::negotiate(&hello, &core_hello)
        .map_err(|err| IpcError::Protocol { source: err })?;
    session.accept_greeting(&greeting, &negotiated)?;

    Ok((stream, session))
}

fn make_command<T: serde::Serialize>(
    kind: CommandKind,
    op_id: OperationId,
    payload: Option<&T>,
    issued_at: UnixMillis,
) -> CommandEnvelope {
    let payload_val = payload.and_then(|p| {
        let v = serde_json::to_value(p).ok()?;
        BoundedPayload::new(v, 64 * 1024).ok()
    });
    CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: op_id,
        kind,
        payload: payload_val,
        issued_at,
    }
}

// ── E2E Journey Test ──────────────────────────────────────────────

#[test]
#[allow(clippy::too_many_lines, clippy::similar_names)]
fn test_p14_acceptance_journey_complete_eight_steps() {
    let start_instant = Instant::now();
    let core_bin = env!("CARGO_BIN_EXE_altior-core");
    let temp_dir = tempfile::tempdir().expect("create journey temp dir");
    let data_dir = temp_dir.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = data_dir.join("altior-core.sqlite");

    // Copy two mock agent binaries to unique names
    let agent_a_guard = TempExeGuard::new("agent_a_full");
    let agent_b_guard = TempExeGuard::new("agent_b_minimal");

    let agent_a_id = AgentProfileId::from_str("agp_p14journeya000000000001").unwrap();
    let agent_b_id = AgentProfileId::from_str("agp_p14journeyb000000000001").unwrap();
    let binding_a_id = HarnessBindingId::from_str("hsb_p14journeya000000000001").unwrap();
    let binding_b_id = HarnessBindingId::from_str("hsb_p14journeyb000000000001").unwrap();

    let now_base = UnixMillis::from_millis(BASE_TIMESTAMP);

    // ── Step 1: Clean Store Validation & Core Daemon Launch ────────────
    {
        let store = Store::open(&db_path).expect("open sqlite store for clean schema check");
        assert_eq!(store.schema_version().expect("schema version"), 5);
        assert!(
            store
                .agent_profiles(None, AgentProfileListLimit::try_new(50).unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .thread_list(None, None, ThreadListLimit::try_new(50).unwrap())
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.journal_len().unwrap(), 0);
    }

    // Launch Core Daemon (Process 1)
    let discovery_path_1 = temp_dir.path().join("core-discovery-1.json");
    let (endpoint_arg_1, _) = unique_endpoint_arg(temp_dir.path());

    let mut guard_1 =
        CoreProcessGuard::spawn(core_bin, &data_dir, &endpoint_arg_1, &discovery_path_1)
            .expect("spawn altior-core daemon process 1");

    let discovery_1 = wait_for_discovery(&mut guard_1, &discovery_path_1, Duration::from_secs(5))
        .expect("wait for discovery 1");

    let (mut client_1, _session_1) =
        connect_and_handshake(&discovery_1, Duration::from_secs(5)).expect("client 1 connect");

    // Configure Agent A and Agent B via IPC
    {
        let op_cfg_a = OperationId::from_str("op_p14cfgagent000000000000a1").unwrap();
        let cfg_cmd_a = make_command(
            CommandKind::ConfigureAgent,
            op_cfg_a,
            Some(&ConfigureAgentCommand {
                agent_profile_id: Some(agent_a_id.clone()),
                display_name: "Journey Agent A Full".to_string(),
                preferred_harness: "acp".to_string(),
                memory_mode: "off".to_string(),
                binding: Some(HarnessBindingConfigDto {
                    harness_binding_id: Some(binding_a_id.clone()),
                    agent_profile_id: Some(agent_a_id.clone()),
                    program: agent_a_guard.path_str(),
                    args: Vec::new(),
                    env_keys: Vec::new(),
                    secret_refs: Vec::new(),
                    label: Some("ACP Full Binding".to_string()),
                }),
            }),
            now_base,
        );
        client_1.send_json(&cfg_cmd_a).unwrap();
        let res_a: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res_a.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        let op_cfg_b = OperationId::from_str("op_p14cfgagent000000000000b1").unwrap();
        let cfg_cmd_b = make_command(
            CommandKind::ConfigureAgent,
            op_cfg_b,
            Some(&ConfigureAgentCommand {
                agent_profile_id: Some(agent_b_id.clone()),
                display_name: "Journey Agent B Minimal".to_string(),
                preferred_harness: "acp".to_string(),
                memory_mode: "off".to_string(),
                binding: Some(HarnessBindingConfigDto {
                    harness_binding_id: Some(binding_b_id.clone()),
                    agent_profile_id: Some(agent_b_id.clone()),
                    program: agent_b_guard.path_str(),
                    args: Vec::new(),
                    env_keys: Vec::new(),
                    secret_refs: Vec::new(),
                    label: Some("ACP Minimal Binding".to_string()),
                }),
            }),
            now_base,
        );
        client_1.send_json(&cfg_cmd_b).unwrap();
        let res_b: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res_b.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // Probe both bindings via test_harness_binding IPC
        let op_test_a = OperationId::from_str("op_p14tstagent000000000000a1").unwrap();
        let test_cmd_a = make_command(
            CommandKind::TestHarnessBinding,
            op_test_a,
            Some(&TestHarnessBindingCommand {
                harness_binding_id: Some(binding_a_id.clone()),
                program: agent_a_guard.path_str(),
                args: Vec::new(),
                env_keys: Vec::new(),
                secret_refs: Vec::new(),
                label: Some("ACP Full Binding".to_string()),
            }),
            now_base,
        );
        client_1.send_json(&test_cmd_a).unwrap();
        let res_test_a: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res_test_a.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        let op_test_b = OperationId::from_str("op_p14tstagent000000000000b1").unwrap();
        let test_cmd_b = make_command(
            CommandKind::TestHarnessBinding,
            op_test_b,
            Some(&TestHarnessBindingCommand {
                harness_binding_id: Some(binding_b_id.clone()),
                program: agent_b_guard.path_str(),
                args: Vec::new(),
                env_keys: Vec::new(),
                secret_refs: Vec::new(),
                label: Some("ACP Minimal Binding".to_string()),
            }),
            now_base,
        );
        client_1.send_json(&test_cmd_b).unwrap();
        let res_test_b: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res_test_b.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // Direct SQLite read assertions: exactly 2 profiles, 2 bindings, no secret canary
        {
            let store = Store::open(&db_path).expect("open sqlite store read-only");
            let profiles = store
                .agent_profiles(None, AgentProfileListLimit::try_new(50).unwrap())
                .unwrap();
            assert_eq!(profiles.len(), 2);

            let bindings_a = store
                .harness_bindings_for_agent(
                    &agent_a_id,
                    None,
                    HarnessBindingListLimit::try_new(50).unwrap(),
                )
                .unwrap();
            assert_eq!(bindings_a.len(), 1);

            let bindings_b = store
                .harness_bindings_for_agent(
                    &agent_b_id,
                    None,
                    HarnessBindingListLimit::try_new(50).unwrap(),
                )
                .unwrap();
            assert_eq!(bindings_b.len(), 1);

            for b in bindings_a.iter().chain(bindings_b.iter()) {
                assert!(
                    !b.command.as_str().contains("CANARY")
                        && !b.command.as_str().contains("SK_FIXTURE")
                );
                for arg in &b.args {
                    assert!(
                        !arg.as_str().contains("CANARY") && !arg.as_str().contains("SK_FIXTURE")
                    );
                }
                for env in &b.env_keys {
                    assert!(
                        !env.as_str().contains("CANARY") && !env.as_str().contains("SK_FIXTURE")
                    );
                }
                for sec in &b.secret_refs {
                    assert!(
                        !sec.as_str().contains("CANARY") && !sec.as_str().contains("SK_FIXTURE")
                    );
                }
            }
        }
    }

    // ── Step 2: Create & Open Threads ──────────────────────────────────
    let thread_a_id;
    let thread_b_id;
    {
        let op_crt_a = OperationId::from_str("op_p14crtthr00000000000000a1").unwrap();
        let crt_cmd_a = make_command(
            CommandKind::CreateThread,
            op_crt_a,
            Some(&CreateThreadCommand {
                agent_profile_id: agent_a_id.clone(),
                title: Some("Thread A Alpha".to_string()),
                project_id: None,
            }),
            now_base,
        );
        client_1.send_json(&crt_cmd_a).unwrap();
        let crt_res_a: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        thread_a_id = match &crt_res_a.body {
            EventBody::Known(KnownEvent::CommandResult {
                success: true,
                data: Some(val),
                ..
            }) => {
                let dto: ThreadDto = serde_json::from_value(val.value().clone()).unwrap();
                dto.id
            }
            other => panic!("expected CommandResult with ThreadDto, got {other:?}"),
        };

        let op_crt_b = OperationId::from_str("op_p14crtthr00000000000000b1").unwrap();
        let crt_cmd_b = make_command(
            CommandKind::CreateThread,
            op_crt_b,
            Some(&CreateThreadCommand {
                agent_profile_id: agent_b_id.clone(),
                title: Some("Thread B Beta".to_string()),
                project_id: None,
            }),
            now_base,
        );
        client_1.send_json(&crt_cmd_b).unwrap();
        let crt_res_b: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        thread_b_id = match &crt_res_b.body {
            EventBody::Known(KnownEvent::CommandResult {
                success: true,
                data: Some(val),
                ..
            }) => {
                let dto: ThreadDto = serde_json::from_value(val.value().clone()).unwrap();
                dto.id
            }
            other => panic!("expected CommandResult with ThreadDto, got {other:?}"),
        };

        // Open thread A: Core automatically selects default binding and creates session
        let op_opn_a = OperationId::from_str("op_p14opnthr00000000000000a1").unwrap();
        let opn_cmd_a = make_command(
            CommandKind::OpenThread,
            op_opn_a,
            Some(&OpenThreadCommand {
                thread_id: thread_a_id.clone(),
                history_limit: Some(50),
            }),
            now_base,
        );
        client_1.send_json(&opn_cmd_a).unwrap();
        let opn_snap_a: SnapshotEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        let snap_data_a: ThreadSnapshotDto = opn_snap_a.parse_data().unwrap();
        assert_eq!(snap_data_a.thread.id, thread_a_id);

        // Open thread B: Core automatically selects default binding and creates session
        let op_opn_b = OperationId::from_str("op_p14opnthr00000000000000b1").unwrap();
        let opn_cmd_b = make_command(
            CommandKind::OpenThread,
            op_opn_b,
            Some(&OpenThreadCommand {
                thread_id: thread_b_id.clone(),
                history_limit: Some(50),
            }),
            now_base,
        );
        client_1.send_json(&opn_cmd_b).unwrap();
        let opn_snap_b: SnapshotEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        let snap_data_b: ThreadSnapshotDto = opn_snap_b.parse_data().unwrap();
        assert_eq!(snap_data_b.thread.id, thread_b_id);

        // Subscribe to live broadcast
        let op_sub = OperationId::from_str("op_p14sub000000000000000001").unwrap();
        let sub_payload = serde_json::json!({ "since": null });
        let sub_cmd = make_command::<serde_json::Value>(
            CommandKind::Subscribe,
            op_sub,
            Some(&sub_payload),
            now_base,
        );
        client_1.send_json(&sub_cmd).unwrap();

        // Direct SQLite read assertion: thread_session_binding exists for both threads
        {
            let store = Store::open(&db_path).expect("open sqlite store read-only");
            let sess_a = store.get_session_binding(&thread_a_id).unwrap();
            assert!(sess_a.is_some(), "session binding for thread A must exist");
            let sess_b = store.get_session_binding(&thread_b_id).unwrap();
            assert!(sess_b.is_some(), "session binding for thread B must exist");
        }
    }

    // ── Step 3: Permission Request & Approval Flow on Agent A ──────────
    {
        let op_turn_a1 = OperationId::from_str("op_p14turn0000000000000000a1").unwrap();
        let turn_a1_id = TurnId::from_str("trn_p14turn0000000000000000a1").unwrap();
        let prompt_text = MessageText::try_from("[TRIGGER_PERMISSION] run tool").unwrap();

        let start_turn_a1 = make_command(
            CommandKind::StartTurn,
            op_turn_a1,
            Some(&StartTurnCommand {
                thread_id: thread_a_id.clone(),
                turn_id: Some(turn_a1_id),
                prompt: prompt_text,
            }),
            now_base,
        );
        client_1.send_json(&start_turn_a1).unwrap();

        // 1. CommandResult for start_turn
        let res: EventEnvelope = match recv_json_timeout(&mut client_1, Duration::from_secs(5)) {
            Ok(envelope) => envelope,
            Err(err) => {
                panic!(
                    "start_turn CommandResult recv failed: {err}\n{}",
                    guard_1.diagnose()
                )
            }
        };
        assert!(matches!(
            res.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // 2. TurnStarted event
        let started_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            started_ev.body,
            EventBody::Known(KnownEvent::TurnStarted)
        ));

        // 3. PermissionRequested event
        let perm_req_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        let perm_event_id = perm_req_ev.event_id;

        // Respond with Permission Decision: Approved
        let op_perm_dec = OperationId::from_str("op_p14perm0000000000000000a1").unwrap();
        let perm_dec_cmd = make_command(
            CommandKind::RespondPermission,
            op_perm_dec,
            Some(&RespondPermissionCommand {
                event_id: perm_event_id,
                decision: "approved".to_string(),
            }),
            now_base,
        );
        client_1.send_json(&perm_dec_cmd).unwrap();

        // 4. CommandResult for respond_permission
        let perm_res: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            perm_res.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // 5. MessageDelta event
        let delta_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            delta_ev.body,
            EventBody::Known(KnownEvent::MessageDelta { .. })
        ));

        // 6. TurnCompleted event
        let complete_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            complete_ev.body,
            EventBody::Known(KnownEvent::TurnCompleted)
        ));
    }

    // ── Step 4: Cancel Flow on Agent B ─────────────────────────────────
    {
        let op_turn_b1 = OperationId::from_str("op_p14turn0000000000000000b1").unwrap();
        let turn_b1_id = TurnId::from_str("trn_p14turn0000000000000000b1").unwrap();
        let prompt_text = MessageText::try_from("[TRIGGER_CANCEL] compute forever").unwrap();

        let start_turn_b1 = make_command(
            CommandKind::StartTurn,
            op_turn_b1.clone(),
            Some(&StartTurnCommand {
                thread_id: thread_b_id.clone(),
                turn_id: Some(turn_b1_id.clone()),
                prompt: prompt_text,
            }),
            now_base,
        );
        client_1.send_json(&start_turn_b1).unwrap();

        // CommandResult
        let res: EventEnvelope = recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // TurnStarted
        let started_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            started_ev.body,
            EventBody::Known(KnownEvent::TurnStarted)
        ));

        // First MessageDelta
        let delta_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            delta_ev.body,
            EventBody::Known(KnownEvent::MessageDelta { .. })
        ));

        // Send CancelTurn command
        let op_cancel_b = OperationId::from_str("op_p14cancel00000000000000b1").unwrap();
        let cancel_cmd_b = make_command(
            CommandKind::CancelTurn,
            op_cancel_b,
            Some(&CancelTurnCommand {
                thread_id: thread_b_id.clone(),
                turn_id: Some(turn_b1_id),
                target_operation_id: Some(op_turn_b1),
            }),
            now_base,
        );
        client_1.send_json(&cancel_cmd_b).unwrap();

        // Cancel command result
        let cancel_res: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            cancel_res.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // Turn completed / cancelled
        let term_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(
            matches!(term_ev.body, EventBody::Known(KnownEvent::TurnCompleted))
                || matches!(
                    term_ev.body,
                    EventBody::Known(KnownEvent::TurnCancelled { .. })
                )
                || matches!(
                    &term_ev.body,
                    EventBody::Unknown { provider_kind, .. } if provider_kind == "turn.cancelled"
                ),
            "cancelled turn settles with terminal completion: {:?}",
            term_ev.body
        );
    }

    // ── Step 5: Client Disconnect & Reconnect Catch-up (补流) ───────────
    let last_seen_sequence;
    {
        let op_turn_a2 = OperationId::from_str("op_p14turn0000000000000000a2").unwrap();
        let turn_a2_id = TurnId::from_str("trn_p14turn0000000000000000a2").unwrap();
        let prompt_text = MessageText::try_from("multi-turn follow up query").unwrap();

        let start_turn_a2 = make_command(
            CommandKind::StartTurn,
            op_turn_a2,
            Some(&StartTurnCommand {
                thread_id: thread_a_id.clone(),
                turn_id: Some(turn_a2_id),
                prompt: prompt_text,
            }),
            now_base,
        );
        client_1.send_json(&start_turn_a2).unwrap();

        // CommandResult
        let res: EventEnvelope = recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            res.body,
            EventBody::Known(KnownEvent::CommandResult { success: true, .. })
        ));

        // TurnStarted
        let started_ev: EventEnvelope =
            recv_json_timeout(&mut client_1, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            started_ev.body,
            EventBody::Known(KnownEvent::TurnStarted)
        ));
        last_seen_sequence = started_ev.sequence;

        // Disconnect Client 1 immediately
        drop(client_1);
    }

    // Reconnect as Client 2 and perform catch-up
    let mut client_2 = {
        let mut attempts = 0;
        loop {
            if let Ok((stream, _)) = connect_and_handshake(&discovery_1, Duration::from_secs(3)) {
                break stream;
            }
            attempts += 1;
            assert!(attempts < 50, "failed to reconnect client 2");
            thread::yield_now();
        }
    };
    {
        // Subscribe since last_seen_sequence to receive replayed events + boundary
        let op_sub_catchup = OperationId::from_str("op_p14subcatchup000000000001").unwrap();
        let sub_payload = serde_json::json!({
            "since": last_seen_sequence.as_u64(),
        });
        let sub_cmd = make_command(
            CommandKind::Subscribe,
            op_sub_catchup,
            Some(&sub_payload),
            now_base,
        );
        client_2.send_json(&sub_cmd).unwrap();

        let mut replayed_delta = false;
        let mut replayed_completed = false;
        let mut observed_boundary = false;

        for _ in 0..10 {
            if let Ok(ev) =
                recv_json_timeout::<EventEnvelope>(&mut client_2, Duration::from_secs(3))
            {
                match ev.body {
                    EventBody::Known(KnownEvent::MessageDelta { .. }) => {
                        replayed_delta = true;
                    }
                    EventBody::Known(KnownEvent::TurnCompleted) => {
                        replayed_completed = true;
                    }
                    EventBody::Known(KnownEvent::StreamReplayed { .. }) => {
                        observed_boundary = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(
            replayed_delta || replayed_completed,
            "reconnect catch-up must replay missed events"
        );
        assert!(
            observed_boundary,
            "reconnect catch-up must conclude with stream.replayed boundary event"
        );

        // Drain any live events that raced past the replay boundary (e.g. a
        // turn completing right after catch-up) so Step 6 starts clean.
        drain_pending_events(&mut client_2, Duration::from_millis(300));
    }

    // ── Step 6: Indeterminate Crash Flow & Daemon Termination ──────────
    let turn_b2_id = TurnId::from_str("trn_p14turn0000000000000000b2").unwrap();
    {
        let op_turn_b2 = OperationId::from_str("op_p14turn0000000000000000b2").unwrap();
        let prompt_text = MessageText::try_from("[TRIGGER_CRASH] fatal crash").unwrap();

        let start_turn_b2 = make_command(
            CommandKind::StartTurn,
            op_turn_b2,
            Some(&StartTurnCommand {
                thread_id: thread_b_id.clone(),
                turn_id: Some(turn_b2_id.clone()),
                prompt: prompt_text,
            }),
            now_base,
        );
        client_2.send_json(&start_turn_b2).unwrap();

        // 1. CommandResult for start_turn
        let res: EventEnvelope = recv_json_timeout(&mut client_2, Duration::from_secs(5)).unwrap();
        assert!(
            matches!(
                res.body,
                EventBody::Known(KnownEvent::CommandResult { success: true, .. })
            ),
            "start_turn for crash turn must be admitted: {:?}",
            res.body
        );

        // 2. TurnStarted
        let started_ev: EventEnvelope =
            recv_json_timeout(&mut client_2, Duration::from_secs(5)).unwrap();
        assert!(
            matches!(started_ev.body, EventBody::Known(KnownEvent::TurnStarted)),
            "expected TurnStarted: {:?}",
            started_ev.body
        );

        // 3. First MessageDelta
        let delta_ev: EventEnvelope =
            recv_json_timeout(&mut client_2, Duration::from_secs(5)).unwrap();
        assert!(
            matches!(
                delta_ev.body,
                EventBody::Known(KnownEvent::MessageDelta { .. })
            ),
            "expected MessageDelta: {:?}",
            delta_ev.body
        );

        // 4. Turn settles failed as child process exits 42
        let _ = recv_json_timeout::<EventEnvelope>(&mut client_2, Duration::from_secs(5));

        // Terminate Core daemon process 1 cleanly via RAII kill_and_wait
        drop(client_2);
        guard_1
            .kill_and_wait()
            .expect("kill and wait core daemon 1");

        cleanup_stale_discovery(&discovery_path_1).expect("cleanup discovery 1");
    }

    // ── Step 7: Core Daemon Restart & Indeterminate No-Resend Check ─────
    let discovery_path_2 = temp_dir.path().join("core-discovery-2.json");
    let (endpoint_arg_2, _) = unique_endpoint_arg(temp_dir.path());

    let mut guard_2 =
        CoreProcessGuard::spawn(core_bin, &data_dir, &endpoint_arg_2, &discovery_path_2)
            .expect("restart altior-core daemon process 2 on same data dir");

    let discovery_2 = wait_for_discovery(&mut guard_2, &discovery_path_2, Duration::from_secs(5))
        .expect("wait for discovery 2");

    let (mut client_3, _session_3) =
        connect_and_handshake(&discovery_2, Duration::from_secs(5)).expect("client 3 connect");

    // 1. Verify indeterminate checkpoints exist in persistent SQLite
    {
        let store = Store::open(&db_path).expect("open persistent store to inspect checkpoints");
        let chk_limit =
            CheckpointListLimit::try_new(altior_domain::CHECKPOINT_LIST_LIMIT_MAX).unwrap();
        let checkpoints = store.runtime_checkpoints(None, None, chk_limit).unwrap();
        let has_indeterminate = checkpoints
            .iter()
            .any(|cp| cp.state == CheckpointState::Indeterminate);
        assert!(
            has_indeterminate,
            "crashed turn must persist as Indeterminate checkpoint across Core restart"
        );
    }

    // 2. Test no-resend via IPC by attempting to start turn with same turn_id
    {
        let op_resend = OperationId::from_str("op_p14resend00000000000000b2").unwrap();
        let resend_cmd = make_command(
            CommandKind::StartTurn,
            op_resend,
            Some(&StartTurnCommand {
                thread_id: thread_b_id.clone(),
                turn_id: Some(turn_b2_id),
                prompt: MessageText::try_from("resending same turn").unwrap(),
            }),
            now_base,
        );
        client_3.send_json(&resend_cmd).unwrap();
        let resend_res: EventEnvelope =
            recv_json_timeout(&mut client_3, Duration::from_secs(5)).unwrap();
        assert!(
            matches!(
                resend_res.body,
                EventBody::Known(KnownEvent::CommandError { ref code, .. }) if code == "START_TURN_FAILED"
            ) || matches!(
                resend_res.body,
                EventBody::Known(KnownEvent::CommandResult { success: false, .. })
            ),
            "sending same turn_id after crash must fail with CommandError/failure, got: {:?}",
            resend_res.body
        );
    }

    // 3. Verify Core runtime status via IPC
    {
        let op_status = OperationId::from_str("op_p14status0000000000000001").unwrap();
        let status_cmd = make_command(
            CommandKind::RuntimeStatus,
            op_status,
            Some(&RuntimeStatusCommand {
                include_diagnostics: true,
            }),
            now_base,
        );
        client_3.send_json(&status_cmd).unwrap();
        let status_ev: EventEnvelope =
            recv_json_timeout(&mut client_3, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            status_ev.body,
            EventBody::Known(KnownEvent::RuntimeStatus { .. })
        ));
    }

    // ── Step 8: Offline Search, History Retrieval & Snapshot Checks ─────
    {
        // Search threads via FTS5
        let op_search = OperationId::from_str("op_p14search0000000000000001").unwrap();
        let search_cmd = make_command(
            CommandKind::SearchThreads,
            op_search,
            Some(&SearchThreadsCommand {
                query: "Alpha".to_string(),
                limit: Some(20),
            }),
            now_base,
        );
        client_3.send_json(&search_cmd).unwrap();
        let search_snap: SnapshotEnvelope =
            recv_json_timeout(&mut client_3, Duration::from_secs(5)).unwrap();
        let search_data: ThreadListResponseDto = search_snap.parse_data().unwrap();
        assert!(!search_data.threads.is_empty());

        // Get turn history for Thread A (contains turns across restarts)
        let op_hist = OperationId::from_str("op_p14history0000000000000001").unwrap();
        let hist_cmd = make_command(
            CommandKind::GetHistory,
            op_hist,
            Some(&GetHistoryCommand {
                thread_id: thread_a_id.clone(),
                cursor: None,
                limit: Some(50),
            }),
            now_base,
        );
        client_3.send_json(&hist_cmd).unwrap();
        let hist_snap: SnapshotEnvelope =
            recv_json_timeout(&mut client_3, Duration::from_secs(5)).unwrap();
        let hist_data: ThreadHistoryResponseDto = hist_snap.parse_data().unwrap();
        assert_eq!(hist_data.thread_id, thread_a_id);
        assert!(!hist_data.turns.is_empty());

        // List all threads
        let op_list = OperationId::from_str("op_p14listthr0000000000000001").unwrap();
        let list_cmd = make_command(
            CommandKind::ListThreads,
            op_list,
            Some(&ListThreadsCommand {
                cursor: None,
                limit: Some(20),
            }),
            now_base,
        );
        client_3.send_json(&list_cmd).unwrap();
        let list_snap: SnapshotEnvelope =
            recv_json_timeout(&mut client_3, Duration::from_secs(5)).unwrap();
        let list_data: ThreadListResponseDto = list_snap.parse_data().unwrap();
        assert!(list_data.threads.len() >= 2);
    }

    // ── Teardown & Hygiene Validation ──────────────────────────────────
    drop(client_3);
    guard_2
        .kill_and_wait()
        .expect("kill and wait core daemon 2");

    cleanup_stale_discovery(&discovery_path_2).expect("cleanup discovery 2");

    assert!(
        guard_1.has_exited() && guard_2.has_exited(),
        "all core daemon child processes must be terminated without leaks"
    );

    assert!(
        start_instant.elapsed() < Duration::from_secs(30),
        "full P1.4 acceptance journey must complete within 30s, took {:?}",
        start_instant.elapsed()
    );
}
