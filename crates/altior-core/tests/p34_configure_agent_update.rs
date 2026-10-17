//! A18 residual acceptance: `configure_agent` must update an existing
//! agent profile over the wire (A18 残留 bug 修复验收).
//!
//! Regression for the daemon handler fabricating `created_at = now` on
//! every configure: the storage upsert treats `created_at` as immutable,
//! so the second configure for the same profile id failed with
//! "cannot modify immutable `created_at`". The wire-level upsert now
//! preserves the stored creation stamp for both the profile and its
//! harness binding. Deterministic: in-memory duplex transport only,
//! no sleeps, fixed clock.

use std::str::FromStr;

use altior_core::{
    CoreApplication, CoreDaemon, InMemoryClient, InMemoryListener,
    runtime::{AcpHarnessAdapter, StoreCheckpointAdapter},
};
use altior_domain::{
    AgentProfileId, AgentProfileListLimit, HarnessBindingId, OperationId, UnixMillis,
};
use altior_protocol::{
    BoundedPayload, CapabilitySet, CommandEnvelope, CommandKind, ConfigureAgentCommand,
    CreateThreadCommand, DesktopHello, EventBody, EventEnvelope, HarnessBindingConfigDto,
    KnownEvent, LaunchToken, OpenThreadCommand, ProductVersion, ProtocolVersion,
    ProtocolVersionRange, SnapshotEnvelope, TestHarnessBindingCommand, ThreadSnapshotDto,
};
use altior_storage::Store;

const BASE_MILLIS: u64 = 1_700_000_000_000;

struct ConfigureDaemon {
    daemon: CoreDaemon<AcpHarnessAdapter, StoreCheckpointAdapter, InMemoryListener>,
    client: InMemoryClient,
    clock_millis: u64,
}

impl ConfigureDaemon {
    fn new() -> Self {
        let store = Store::open_in_memory().expect("open in-memory store");
        let credentials = altior_ipc::LaunchCredentials {
            instance_id: "cor_configure0000001".parse().unwrap(),
            launch_token: LaunchToken::from_str("0f1e2d3c4b5a69788796a5b4c3d2e1f0").unwrap(),
        };
        let app = CoreApplication::new(
            AcpHarnessAdapter::new(),
            StoreCheckpointAdapter::new(store),
            credentials,
        );
        let listener = InMemoryListener::new("configure-test");
        let client = listener.create_client();
        let daemon = CoreDaemon::new(app, listener);
        Self {
            daemon,
            client,
            clock_millis: BASE_MILLIS,
        }
    }

    fn step(&mut self) {
        self.daemon
            .step(UnixMillis::from_millis(self.clock_millis))
            .expect("daemon step");
    }

    fn handshake(&mut self) {
        let hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .unwrap(),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
            launch_token: LaunchToken::from_str("0f1e2d3c4b5a69788796a5b4c3d2e1f0").unwrap(),
        };
        self.client.send_json(&hello).expect("send hello");
        for _ in 0..10 {
            self.step();
            if let Ok(Some(_core_hello)) = self.client.recv_json::<altior_protocol::CoreHello>() {
                break;
            }
        }
        for _ in 0..10 {
            match self.client.recv_json::<altior_protocol::CoreGreeting>() {
                Ok(Some(_greeting)) => break,
                Ok(None) => self.step(),
                Err(err) => panic!("greeting read failed: {err}"),
            }
        }
    }

    fn configure(&mut self, op: &str, cmd: &ConfigureAgentCommand, at: u64) -> EventEnvelope {
        self.send(op, CommandKind::ConfigureAgent, cmd, at);
        for _ in 0..32 {
            self.step();
            if let Ok(Some(event)) = self.client.recv_json::<EventEnvelope>() {
                return event;
            }
        }
        panic!("no configure_agent answer within 32 daemon steps");
    }

    fn send<T: serde::Serialize>(&mut self, op: &str, kind: CommandKind, payload: &T, at: u64) {
        let envelope = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: OperationId::from_str(op).unwrap(),
            kind,
            payload: Some(
                BoundedPayload::new(
                    serde_json::to_value(payload).expect("serialize payload"),
                    64 * 1024,
                )
                .expect("bounded payload"),
            ),
            issued_at: UnixMillis::from_millis(at),
        };
        self.client.send_json(&envelope).expect("send command");
        self.clock_millis = at;
    }

    fn open_thread_snapshot(
        &mut self,
        thread_id: &altior_domain::ThreadId,
        at: u64,
    ) -> ThreadSnapshotDto {
        let op = OperationId::from_str("op_p34openthread0000001").unwrap();
        self.send(
            "op_p34openthread0000001",
            CommandKind::OpenThread,
            &OpenThreadCommand {
                thread_id: thread_id.clone(),
                history_limit: None,
            },
            at,
        );
        // Events (e.g. session state changes) may precede the snapshot on
        // the stream; drain frames until the snapshot answering this op.
        for step in 0..64 {
            self.step();
            if let Ok(Some(bytes)) = self.client.recv_frame() {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let snap: SnapshotEnvelope = match serde_json::from_slice(&bytes) {
                    Ok(snap) => snap,
                    Err(err) => {
                        println!("step {step}: non-snapshot frame: {err}: {text}");
                        continue;
                    }
                };
                if snap.operation_id == op {
                    return serde_json::from_value(snap.data.value().clone())
                        .expect("decode thread snapshot");
                }
                println!("step {step}: snapshot for other op {}", snap.operation_id);
            }
        }
        panic!("no open_thread snapshot within 64 daemon steps");
    }

    fn test_binding(&mut self, program: &str, at: u64) -> serde_json::Value {
        self.send(
            "op_p34testbinding0000001",
            CommandKind::TestHarnessBinding,
            &TestHarnessBindingCommand {
                harness_binding_id: None,
                program: program.to_string(),
                args: Vec::new(),
                env_keys: Vec::new(),
                secret_refs: Vec::new(),
                label: Some("Probe Fixture".to_string()),
            },
            at,
        );
        for _ in 0..32 {
            self.step();
            if let Ok(Some(event)) = self.client.recv_json::<EventEnvelope>() {
                match event.body {
                    EventBody::Known(KnownEvent::CommandResult {
                        operation_id,
                        success,
                        data,
                    }) => {
                        assert_eq!(
                            operation_id,
                            OperationId::from_str("op_p34testbinding0000001").unwrap()
                        );
                        assert!(success, "probe command itself must succeed");
                        let payload = data.expect("probe result data");
                        return serde_json::from_value(payload.value().clone())
                            .expect("decode probe result json");
                    }
                    other => panic!("expected CommandResult, got {other:?}"),
                }
            }
        }
        panic!("no test_harness_binding answer within 32 daemon steps");
    }
}

fn expect_success(event: EventEnvelope, op: &OperationId) {
    match event.body {
        EventBody::Known(KnownEvent::CommandResult {
            operation_id,
            success,
            ..
        }) => {
            assert_eq!(&operation_id, op, "result must answer the configure op");
            assert!(success, "configure_agent must succeed");
        }
        other => panic!("expected CommandResult, got {other:?}"),
    }
}

fn binding_config(agent: &AgentProfileId, binding: &HarnessBindingId) -> HarnessBindingConfigDto {
    HarnessBindingConfigDto {
        harness_binding_id: Some(binding.clone()),
        agent_profile_id: Some(agent.clone()),
        program: "C:\\fixture\\agent.exe".to_string(),
        args: vec!["--serve".to_string()],
        env_keys: Vec::new(),
        secret_refs: Vec::new(),
        label: Some("Fixture Binding".to_string()),
    }
}

#[test]
fn configure_agent_updates_existing_profile_and_binding() {
    let agent = AgentProfileId::from_str("agp_p34updatea0000000001").expect("valid agent id");
    let binding = HarnessBindingId::from_str("hsb_p34updatea0000000001").expect("valid binding id");

    let mut h = ConfigureDaemon::new();
    h.handshake();

    // 1. First configure creates the profile and binding.
    let create = ConfigureAgentCommand {
        agent_profile_id: Some(agent.clone()),
        display_name: "Fixture Agent".to_string(),
        preferred_harness: "acp".to_string(),
        memory_mode: "off".to_string(),
        binding: Some(binding_config(&agent, &binding)),
    };
    let op_create = OperationId::from_str("op_p34configurecreate01").unwrap();
    let created = h.configure("op_p34configurecreate01", &create, BASE_MILLIS);
    expect_success(created, &op_create);

    // 2. Second configure with the same ids updates display name, memory
    //    mode, and binding. Pre-fix this failed with "cannot modify
    //    immutable created_at" because the handler stamped
    //    created_at = now onto the existing rows.
    let update = ConfigureAgentCommand {
        agent_profile_id: Some(agent.clone()),
        display_name: "Renamed Fixture Agent".to_string(),
        preferred_harness: "acp".to_string(),
        memory_mode: "long_term".to_string(),
        binding: Some(HarnessBindingConfigDto {
            program: "C:\\fixture\\agent2.exe".to_string(),
            args: vec!["--serve".to_string(), "--verbose".to_string()],
            label: Some("Renamed Fixture Binding".to_string()),
            ..binding_config(&agent, &binding)
        }),
    };
    let op_update = OperationId::from_str("op_p34configureupdate01").unwrap();
    let updated = h.configure("op_p34configureupdate01", &update, BASE_MILLIS + 1_000);
    expect_success(updated, &op_update);

    // 3. Stored state shows the update applied and created_at preserved.
    let stored_profile = h
        .daemon
        .app()
        .get_agent_profile(&agent)
        .expect("read profile")
        .expect("profile exists");
    assert_eq!(
        stored_profile.display_name.as_str(),
        "Renamed Fixture Agent"
    );
    assert_eq!(stored_profile.memory_mode.as_str(), "long_term");
    assert_eq!(
        stored_profile.created_at.as_millis(),
        BASE_MILLIS,
        "created_at must be preserved across update"
    );
    assert_eq!(stored_profile.updated_at.as_millis(), BASE_MILLIS + 1_000);

    let stored_binding = h
        .daemon
        .app()
        .get_harness_binding(&binding)
        .expect("read binding")
        .expect("binding exists");
    assert_eq!(stored_binding.command.as_str(), "C:\\fixture\\agent2.exe");
    assert_eq!(stored_binding.args.len(), 2);
    assert_eq!(stored_binding.label.as_str(), "Renamed Fixture Binding");
    assert_eq!(
        stored_binding.created_at.as_millis(),
        BASE_MILLIS,
        "binding created_at must be preserved across update"
    );
}

#[test]
fn configure_agent_create_without_id_then_update_by_profile_id() {
    // The desktop create path sends agent_profile_id: None; the update
    // path then targets an existing id. Both must round-trip over the
    // same command kind.
    let mut h = ConfigureDaemon::new();
    h.handshake();

    let create = ConfigureAgentCommand {
        agent_profile_id: None,
        display_name: "Created Without Id".to_string(),
        preferred_harness: "terminal".to_string(),
        memory_mode: "session".to_string(),
        binding: None,
    };
    let op_create = OperationId::from_str("op_p34configurecreate02").unwrap();
    let created = h.configure("op_p34configurecreate02", &create, BASE_MILLIS);
    expect_success(created, &op_create);

    // Core owns identity allocation (ADR 0019); read the created profile
    // back through the app layer.
    let profiles = h
        .daemon
        .app()
        .list_agent_profiles(None, AgentProfileListLimit::try_new(10).unwrap())
        .expect("list profiles");
    assert_eq!(
        profiles.len(),
        1,
        "create without id must persist exactly one profile"
    );
    let agent = profiles[0].id.clone();

    let update = ConfigureAgentCommand {
        agent_profile_id: Some(agent.clone()),
        display_name: "Updated After Create".to_string(),
        preferred_harness: "terminal".to_string(),
        memory_mode: "off".to_string(),
        binding: None,
    };
    let op_update = OperationId::from_str("op_p34configureupdate02").unwrap();
    let updated = h.configure("op_p34configureupdate02", &update, BASE_MILLIS + 500);
    expect_success(updated, &op_update);

    let stored = h
        .daemon
        .app()
        .get_agent_profile(&agent)
        .expect("read profile")
        .expect("profile exists");
    assert_eq!(stored.display_name.as_str(), "Updated After Create");
    assert_eq!(stored.memory_mode.as_str(), "off");
    assert_eq!(
        stored.created_at.as_millis(),
        BASE_MILLIS,
        "created_at must survive the update"
    );
}

#[test]
fn open_thread_reports_session_failure_instead_of_swallowing_it() {
    // A18 残留 bug：open_thread 曾用 `let _ =` 丢弃 create_session 错误。
    // 离线可读不变量要求 open 本身仍成功（历史可读），但失败原因必须随
    // snapshot 传播，而不是无声消失。
    let agent = AgentProfileId::from_str("agp_p34brokensess0000001").expect("valid agent id");
    let binding = HarnessBindingId::from_str("hsb_p34brokensess0000001").expect("valid binding id");

    let mut h = ConfigureDaemon::new();
    h.handshake();

    // Binding program does not exist, so session creation fails
    // deterministically (spawn error, no network, no sleeps).
    let create = ConfigureAgentCommand {
        agent_profile_id: Some(agent.clone()),
        display_name: "Broken Session Agent".to_string(),
        preferred_harness: "acp".to_string(),
        memory_mode: "off".to_string(),
        binding: Some(HarnessBindingConfigDto {
            harness_binding_id: Some(binding.clone()),
            agent_profile_id: Some(agent.clone()),
            program: "C:\\fixture\\definitely-missing-agent.exe".to_string(),
            args: Vec::new(),
            env_keys: Vec::new(),
            secret_refs: Vec::new(),
            label: Some("Broken Binding".to_string()),
        }),
    };
    let op_create = OperationId::from_str("op_p34configurecreate03").unwrap();
    let created = h.configure("op_p34configurecreate03", &create, BASE_MILLIS);
    expect_success(created, &op_create);

    h.send(
        "op_p34createthread000001",
        CommandKind::CreateThread,
        &CreateThreadCommand {
            agent_profile_id: agent.clone(),
            title: Some("Broken Session Thread".to_string()),
            project_id: None,
        },
        BASE_MILLIS + 100,
    );
    let mut create_answered = false;
    for step in 0..32 {
        h.step();
        if let Ok(Some(bytes)) = h.client.recv_frame() {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            println!("create step {step}: {text}");
            let event: EventEnvelope = serde_json::from_slice(&bytes).expect("decode event");
            expect_success(
                event,
                &OperationId::from_str("op_p34createthread000001").unwrap(),
            );
            create_answered = true;
            break;
        }
    }
    assert!(create_answered, "create_thread must answer");

    // Core owns thread identity allocation (ADR 0019); read the created
    // thread back through the app layer.
    let threads = h
        .daemon
        .app()
        .list_threads(
            None,
            None,
            altior_domain::ThreadListLimit::try_new(10).unwrap(),
        )
        .expect("list threads");
    assert_eq!(threads.len(), 1, "create_thread must persist one thread");
    let thread = altior_domain::ThreadId::from_str(&threads[0].thread_id).expect("valid thread id");

    let snapshot = h.open_thread_snapshot(&thread, BASE_MILLIS + 200);
    assert_eq!(
        snapshot.thread.id, thread,
        "snapshot answers the opened thread"
    );
    let reason = snapshot
        .session_error
        .expect("session failure reason must travel with the snapshot");
    assert!(
        !reason.trim().is_empty(),
        "session_error must carry a non-empty diagnostic"
    );

    // The supervisor must not pretend a session exists.
    let state = h
        .daemon
        .app()
        .supervisor()
        .supervisor(&thread)
        .map(|s| s.state().clone());
    assert_ne!(
        state,
        Some(altior_core::runtime::state::SupervisorState::Ready),
        "no session was established, state must not claim readiness"
    );
}

#[test]
fn probe_binding_failure_carries_actionable_diagnostics() {
    // A18 残留 bug：probe_binding 曾把 spawn / 握手失败吞成 diagnostics: None，
    // 用户只能看到 ok=false 而没有任何原因。
    let mut h = ConfigureDaemon::new();
    h.handshake();

    let outcome = h.test_binding("C:\\fixture\\definitely-missing-agent.exe", BASE_MILLIS);
    assert_eq!(
        outcome["ok"], false,
        "probe of a missing program must report failure"
    );
    let diagnostics = outcome["diagnostics"]
        .as_str()
        .expect("probe failure must carry a diagnostics reason");
    assert!(
        diagnostics.contains("definitely-missing-agent.exe"),
        "diagnostics must name the probed program, got: {diagnostics}"
    );
    assert!(
        !diagnostics.trim().is_empty(),
        "diagnostics must be non-empty"
    );

    // The result also hands back the Core-minted binding identity (ADR 0019).
    assert!(
        outcome["probed_binding_id"].is_string(),
        "probe must return the binding id used"
    );
}
