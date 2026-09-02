//! P1.3 Core application service and command dispatcher integration tests (Lane B).
//!
//! Validates:
//! 1. UI disconnection resilience: UI disconnect does not interrupt supervisor turns; events buffer in replay log.
//! 2. Reconnect replay (补流): UI client reconnects with `subscribe(since)` and receives missing events + `stream.replayed`.
//! 3. Two concurrent / distinct threads: isolated lifecycle, separate sessions, turns, and event streams.
//! 4. Permission pause/decision and cancellation flow: approved permissions continue; cancel marks turn Cancelled.
//! 5. Operation ID idempotency: duplicate commands are acknowledged without duplicate execution.
//! 6. Status and diagnostics reporting: includes indeterminate checkpoints scan on restart with resend prohibition.

use std::collections::{BTreeMap, VecDeque};
use std::str::FromStr;

use altior_core::application::{
    CommandDispatcher, CoreApplication, CoreCommand, CoreCommandEnvelope, CoreCommandResponse,
    CoreDaemon, CoreDaemonConfig, FakeConnection,
};
use altior_core::runtime::{
    BindingProbeOutcome, CancelOutcome, HarnessError, HarnessEvent, HarnessPromptRequest,
    HarnessRuntimePort, HarnessSessionId, HarnessSessionInfo, SupervisorState, TurnAdmission,
};
use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, DisplayName, DomainEventKind, EventId,
    HISTORY_LIMIT_MAX, HarnessBindingId, HarnessKind, HistoryLimit, MemoryMode, OperationId,
    PERMISSION_LIST_LIMIT_MAX, PermissionDecision, PermissionDescription, PermissionKind,
    PermissionListLimit, ProjectRef, SearchQuery, THREAD_LIST_LIMIT_MAX, TURN_LIST_LIMIT_MAX,
    ThreadId, ThreadListLimit, ThreadTitle, TurnId, TurnListLimit, UnixMillis,
};
use altior_ipc::{CatchUpDelivery, LaunchCredentials, mint_launch_token};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, DesktopHello, EnvelopeLimits, EventBody, EventEnvelope,
    KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange,
};

// ── Test Double: Mock Agent Harness ─────────────────────────────────

#[derive(Debug, Default)]
struct MockHarness {
    capabilities: CapabilitySet,
    fail_prompt: bool,
    created_sessions: Vec<(String, ThreadId)>,
    sent_prompts: Vec<(HarnessSessionId, HarnessPromptRequest)>,
    cancelled_sessions: Vec<HarnessSessionId>,
    decided_permissions: Vec<(HarnessSessionId, EventId, PermissionDecision)>,
    events: BTreeMap<HarnessSessionId, VecDeque<HarnessEvent>>,
}

impl MockHarness {
    fn new() -> Self {
        Self::default()
    }

    fn queue_event(&mut self, session_id: &HarnessSessionId, event: HarnessEvent) {
        self.events
            .entry(session_id.clone())
            .or_default()
            .push_back(event);
    }
}

impl HarnessRuntimePort for MockHarness {
    fn probe_binding(
        &mut self,
        _binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, HarnessError> {
        Ok(BindingProbeOutcome {
            ok: true,
            capabilities: self.capabilities.clone(),
            diagnostics: None,
        })
    }

    fn create_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        _project: Option<&ProjectRef>,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        self.created_sessions
            .push((binding.id.to_string(), thread_id.clone()));
        let session_id = HarnessSessionId::new(&format!("sess_{thread_id}")).unwrap();
        Ok(HarnessSessionInfo {
            session_id,
            capabilities: self.capabilities.clone(),
        })
    }

    fn resume_session(
        &mut self,
        _binding: &AcpHarnessBinding,
        session_id: &HarnessSessionId,
        _thread_id: &ThreadId,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        Ok(HarnessSessionInfo {
            session_id: session_id.clone(),
            capabilities: self.capabilities.clone(),
        })
    }

    fn send_prompt(
        &mut self,
        session_id: &HarnessSessionId,
        prompt: HarnessPromptRequest,
    ) -> Result<(), HarnessError> {
        if self.fail_prompt {
            return Err(HarnessError::Transport(
                "mock harness prompt failure".to_string(),
            ));
        }
        self.sent_prompts.push((session_id.clone(), prompt));
        Ok(())
    }

    fn cancel_turn(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        self.cancelled_sessions.push(session_id.clone());
        Ok(())
    }

    fn decide_permission(
        &mut self,
        session_id: &HarnessSessionId,
        event_id: &EventId,
        decision: PermissionDecision,
    ) -> Result<(), HarnessError> {
        self.decided_permissions
            .push((session_id.clone(), event_id.clone(), decision));
        Ok(())
    }

    fn poll_event(
        &mut self,
        session_id: &HarnessSessionId,
    ) -> Result<Option<HarnessEvent>, HarnessError> {
        if let Some(queue) = self.events.get_mut(session_id) {
            Ok(queue.pop_front())
        } else {
            Ok(None)
        }
    }

    fn close_session(&mut self, _session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        Ok(())
    }
}

// ── Test Helpers ───────────────────────────────────────────────────

const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn fixture_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_fixture000000001".parse().unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn fixture_profile(id: &str, name: &str) -> AgentProfile {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    AgentProfile {
        id: AgentProfileId::from_str(id).unwrap(),
        display_name: DisplayName::try_from(name).unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::Off,
        created_at: now,
        updated_at: now,
    }
}

fn fixture_binding(id: &str, profile_id: &str) -> AcpHarnessBinding {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    AcpHarnessBinding {
        id: HarnessBindingId::from_str(id).unwrap(),
        agent_profile_id: AgentProfileId::from_str(profile_id).unwrap(),
        label: DisplayName::try_from("test-agent").unwrap(),
        command: altior_domain::BoundedPath::try_from("mock_agent").unwrap(),
        args: Vec::new(),
        env_keys: Vec::new(),
        secret_refs: Vec::new(),
        created_at: now,
    }
}

fn desktop_hello(credentials: &LaunchCredentials) -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .unwrap(),
        desktop_version: "0.1.0".parse().unwrap(),
        capabilities: CapabilitySet::new(),
        launch_token: credentials.launch_token.clone(),
    }
}

// ── Test 1: UI Disconnect Resilience ───────────────────────────────

#[test]
fn ui_disconnect_does_not_interrupt_turn_and_buffers_events() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials.clone()).unwrap();
    let server_port = app.server_port();

    // 1. Setup agent and thread
    let profile = fixture_profile("agp_fixture000000001", "Claude");
    let binding = fixture_binding("hsb_fixture000000001", "agp_fixture000000001");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000001").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("UI Disconnect Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();

    // 2. Open thread session
    let open_res = app.open_thread(&thread_id, Some(&binding)).unwrap();
    let session_id = open_res.session_id.unwrap();

    // 3. UI client attaches and completes handshake
    let mut conn = FakeConnection::new(&server_port);
    let established = conn.handshake(&desktop_hello(&credentials)).unwrap();
    assert_eq!(established.greeting.instance_id, credentials.instance_id);

    // 4. Start prompt turn
    let op_id = OperationId::from_str("op_fixture000000001").unwrap();
    let turn_id = TurnId::from_str("trn_fixture000000001").unwrap();
    let admission = app
        .start_prompt(
            op_id,
            thread_id.clone(),
            turn_id.clone(),
            "Hello agent",
            now,
        )
        .unwrap();
    assert_eq!(admission, TurnAdmission::Admitted);

    // 5. Simulate UI disconnecting during execution
    let _client_state = conn.disconnect();

    // 6. Agent harness continues emitting stream deltas and completion in background
    app.supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "Hello from agent while UI is disconnected!".to_string(),
        },
    );
    app.supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::Completed {
            payload: Some(b"{\"status\":\"done\"}".to_vec().try_into().unwrap()),
        },
    );

    // 7. Event pump drains supervisor in background
    let env1 = app.poll_thread_events(&thread_id, now).unwrap().unwrap();
    let env2 = app.poll_thread_events(&thread_id, now).unwrap().unwrap();

    assert!(matches!(
        env1.body,
        EventBody::Known(KnownEvent::MessageDelta { .. })
    ));
    assert!(matches!(
        env2.body,
        EventBody::Known(KnownEvent::TurnCompleted)
    ));

    // 8. Verify supervisor transitioned cleanly to Ready despite UI being disconnected
    let sup_state = app
        .supervisor()
        .supervisor(&thread_id)
        .unwrap()
        .state()
        .clone();
    assert_eq!(sup_state, SupervisorState::Ready);

    // 9. Verify events remain buffered in EventLog for reconnection catch-up
    let retained = app.event_log().lock().unwrap().retained().unwrap();
    assert_eq!(retained.from.as_u64(), 1);
    assert_eq!(retained.through.as_u64(), 3); // 1 = TurnStarted, 2 = MessageDelta, 3 = TurnCompleted
}

// ── Test 2: Reconnect Replay (补流) ─────────────────────────────────

#[test]
fn reconnect_replays_missing_stream_events_with_boundary() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials.clone()).unwrap();
    let server_port = app.server_port();

    let profile = fixture_profile("agp_fixture000000002", "Claude");
    let binding = fixture_binding("hsb_fixture000000002", "agp_fixture000000002");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000002").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("Reconnect Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();

    let open_res = app.open_thread(&thread_id, Some(&binding)).unwrap();
    let session_id = open_res.session_id.unwrap();

    // First UI connection: subscribes from beginning (since: None)
    let mut conn1 = FakeConnection::new(&server_port);
    conn1.handshake(&desktop_hello(&credentials)).unwrap();

    let sub1 = CommandEnvelope::subscribe(
        None,
        "op_fixture000000010".parse().unwrap(),
        now,
        &EnvelopeLimits::default(),
    )
    .unwrap();
    let delivery1 = conn1
        .subscribe(&sub1, "evt_fixture000000010".parse().unwrap(), now)
        .unwrap();
    assert!(matches!(delivery1, CatchUpDelivery::UpToDate));

    // Emit prompt turn (sequence 1 = TurnStarted)
    let op_id = OperationId::from_str("op_fixture000000002").unwrap();
    let turn_id = TurnId::from_str("trn_fixture000000002").unwrap();
    app.start_prompt(
        op_id.clone(),
        thread_id.clone(),
        turn_id.clone(),
        "Stream me",
        now,
    )
    .unwrap();

    // Client 1 receives sequence 1
    let env_start = app.event_log().lock().unwrap().retained().unwrap();
    assert_eq!(env_start.through.as_u64(), 1);
    let event1 = altior_protocol::EventEnvelope {
        protocol_version: ProtocolVersion::V1,
        event_id: EventId::from_str("evt_fixture000000001").unwrap(),
        operation_id: Some(op_id),
        thread_id: Some(thread_id.clone()),
        turn_id: Some(turn_id),
        sequence: altior_protocol::Sequence::FIRST,
        occurred_at: now,
        body: EventBody::Known(KnownEvent::TurnStarted),
    };
    conn1.accept_event(&event1).unwrap();

    // Client 1 disconnects after sequence 1
    let client_state = conn1.disconnect();
    assert_eq!(client_state.subscribe_since().unwrap().as_u64(), 1);

    // Core generates 2 more events while client is disconnected (sequences 2 and 3)
    app.supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "delta chunk 1".to_string(),
        },
    );
    app.supervisor_mut()
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });
    app.poll_thread_events(&thread_id, now).unwrap().unwrap(); // seq 2
    app.poll_thread_events(&thread_id, now).unwrap().unwrap(); // seq 3

    // Client reconnects with client_state (last seen sequence = 1)
    let mut conn2 = FakeConnection::reconnect(&server_port, client_state);
    conn2.handshake(&desktop_hello(&credentials)).unwrap();

    let sub2 = CommandEnvelope::subscribe(
        conn2.client_session().subscribe_since(),
        "op_fixture000000011".parse().unwrap(),
        now,
        &EnvelopeLimits::default(),
    )
    .unwrap();

    let delivery2 = conn2
        .subscribe(&sub2, "evt_fixture000000012".parse().unwrap(), now)
        .unwrap();

    // Replay delivers missed events (seq 2, 3) followed by boundary stream.replayed (seq 4)
    match delivery2 {
        CatchUpDelivery::Replay { events, boundary } => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].sequence.as_u64(), 2);
            assert_eq!(events[1].sequence.as_u64(), 3);
            assert_eq!(boundary.sequence.as_u64(), 4);
            assert!(matches!(
                boundary.body,
                EventBody::Known(KnownEvent::StreamReplayed { from, through })
                if from.as_u64() == 2 && through.as_u64() == 3
            ));
        }
        other => panic!("expected Replay delivery, got {other:?}"),
    }

    // Client session advanced to sequence 4
    assert_eq!(
        conn2.client_session().subscribe_since().unwrap().as_u64(),
        4
    );
}

// ── Test 3: Two Concurrent / Distinct Threads ──────────────────────

#[test]
fn two_threads_operate_with_isolated_state_and_history() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials).unwrap();

    let profile = fixture_profile("agp_fixture000000003", "Claude");
    let binding = fixture_binding("hsb_fixture000000003", "agp_fixture000000003");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let t1 = ThreadId::from_str("thr_fixture00000000a").unwrap();
    let t2 = ThreadId::from_str("thr_fixture00000000b").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);

    let title_a = ThreadTitle::try_from("Thread A").unwrap();
    app.create_thread(
        t1.clone(),
        &binding.agent_profile_id,
        Some(&title_a),
        None,
        now,
    )
    .unwrap();

    let title_b = ThreadTitle::try_from("Thread B").unwrap();
    app.create_thread(
        t2.clone(),
        &binding.agent_profile_id,
        Some(&title_b),
        None,
        now,
    )
    .unwrap();

    let open1 = app.open_thread(&t1, Some(&binding)).unwrap();
    let open2 = app.open_thread(&t2, Some(&binding)).unwrap();
    let sess1 = open1.session_id.unwrap();
    let sess2 = open2.session_id.unwrap();
    assert_ne!(sess1, sess2);

    // Start turn on Thread A
    let op_a = OperationId::from_str("op_fixture00000000a").unwrap();
    let trn_a = TurnId::from_str("trn_fixture00000000a").unwrap();
    app.start_prompt(op_a, t1.clone(), trn_a, "Prompt A", now)
        .unwrap();

    // Start turn on Thread B
    let op_b = OperationId::from_str("op_fixture00000000b").unwrap();
    let trn_b = TurnId::from_str("trn_fixture00000000b").unwrap();
    app.start_prompt(op_b, t2.clone(), trn_b, "Prompt B", now)
        .unwrap();

    // Queue completion on Thread A only
    app.supervisor_mut()
        .harness_mut()
        .queue_event(&sess1, HarnessEvent::Completed { payload: None });
    app.poll_thread_events(&t1, now).unwrap().unwrap();

    // Verify Thread A is Ready, Thread B is still Prompting
    let diag = app.get_diagnostics(None).unwrap();
    assert_eq!(diag.thread_states.get(&t1), Some(&SupervisorState::Ready));
    assert!(matches!(
        diag.thread_states.get(&t2),
        Some(SupervisorState::Prompting { .. })
    ));

    // Finish Thread B
    app.supervisor_mut()
        .harness_mut()
        .queue_event(&sess2, HarnessEvent::Completed { payload: None });
    app.poll_thread_events(&t2, now).unwrap().unwrap();

    let diag2 = app.get_diagnostics(None).unwrap();
    assert_eq!(diag2.thread_states.get(&t2), Some(&SupervisorState::Ready));

    // Check thread list shows both threads
    let threads = app
        .list_threads(
            None,
            None,
            ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    assert_eq!(threads.len(), 2);
}

// ── Test 4: Permission & Cancel Flow ───────────────────────────────

#[test]
#[allow(clippy::too_many_lines)]
fn permission_approval_and_turn_cancel_via_command_dispatcher() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials).unwrap();
    let dispatcher = CommandDispatcher::new();

    let profile = fixture_profile("agp_fixture000000004", "Claude");
    let binding = fixture_binding("hsb_fixture000000004", "agp_fixture000000004");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000004").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);

    // 1. Create thread via dispatcher
    let create_cmd = CoreCommandEnvelope::new(
        "op_fixture000000020".parse().unwrap(),
        now,
        CoreCommand::CreateThread {
            thread_id: thread_id.clone(),
            agent_profile_id: binding.agent_profile_id.clone(),
            title: Some(ThreadTitle::try_from("Permission & Cancel Test").unwrap()),
            project_id: None,
        },
    );
    let res = dispatcher.dispatch(&mut app, create_cmd).unwrap();
    assert!(matches!(res, CoreCommandResponse::ThreadCreated(_)));

    let open_res = app.open_thread(&thread_id, Some(&binding)).unwrap();
    let session_id = open_res.session_id.unwrap();

    // 2. Start prompt turn via dispatcher
    let turn_id = TurnId::from_str("trn_fixture000000004").unwrap();
    let prompt_cmd = CoreCommandEnvelope::new(
        "op_fixture000000021".parse().unwrap(),
        now,
        CoreCommand::StartPrompt {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            content: "Run bash command".to_string(),
        },
    );
    let res = dispatcher.dispatch(&mut app, prompt_cmd).unwrap();
    assert_eq!(
        res,
        CoreCommandResponse::PromptStarted(TurnAdmission::Admitted)
    );

    // 3. Harness emits permission request
    let perm_id = EventId::from_str("evt_fixture000000099").unwrap();
    app.supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::PermissionRequest {
            event_id: perm_id.clone(),
            kind: PermissionKind::Execute,
            description: PermissionDescription::try_from("Execute ls -la").unwrap(),
        },
    );
    app.poll_thread_events(&thread_id, now).unwrap().unwrap();

    // Verify supervisor is in AwaitingPermission state
    assert!(matches!(
        app.supervisor().supervisor(&thread_id).unwrap().state(),
        SupervisorState::AwaitingPermission { .. }
    ));

    // 4. Submit permission approval via dispatcher
    let perm_cmd = CoreCommandEnvelope::new(
        "op_fixture000000022".parse().unwrap(),
        now,
        CoreCommand::PermissionDecision {
            thread_id: thread_id.clone(),
            permission_id: perm_id.clone(),
            decision: PermissionDecision::Approved,
        },
    );
    let res = dispatcher.dispatch(&mut app, perm_cmd).unwrap();
    assert_eq!(res, CoreCommandResponse::PermissionDecided);

    // State returned to Prompting
    assert!(matches!(
        app.supervisor().supervisor(&thread_id).unwrap().state(),
        SupervisorState::Prompting { .. }
    ));

    // 5. Cancel the turn via dispatcher
    let cancel_cmd = CoreCommandEnvelope::new(
        "op_fixture000000023".parse().unwrap(),
        now,
        CoreCommand::Cancel {
            thread_id: thread_id.clone(),
        },
    );
    let res = dispatcher.dispatch(&mut app, cancel_cmd).unwrap();
    assert_eq!(
        res,
        CoreCommandResponse::Cancelled(CancelOutcome::CancelledActive)
    );

    // 6. Harness emits Cancelled confirmation
    app.supervisor_mut()
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Cancelled);
    app.poll_thread_events(&thread_id, now).unwrap().unwrap();

    // Supervisor returns to Ready
    assert_eq!(
        app.supervisor().supervisor(&thread_id).unwrap().state(),
        &SupervisorState::Ready
    );

    // 7. Verify storage projections: permission was approved, turn is cancelled, journal history is preserved
    let permissions = app
        .get_thread_permissions(
            &thread_id,
            None,
            PermissionListLimit::try_new(PERMISSION_LIST_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    assert_eq!(permissions.len(), 1);
    assert_eq!(permissions[0].event_id, perm_id);
    assert_eq!(permissions[0].decision, PermissionDecision::Approved);
    assert!(permissions[0].decided_at.is_some());

    let turns = app
        .get_thread_turns(
            &thread_id,
            None,
            TurnListLimit::try_new(TURN_LIST_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, turn_id.as_str());
    assert_eq!(turns[0].state, "cancelled");

    let history = app
        .get_thread_history(
            &thread_id,
            0,
            HistoryLimit::try_new(HISTORY_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    let kinds: Vec<String> = history.into_iter().map(|row| row.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::ThreadCreated.as_str())
    );
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::TurnStarted.as_str())
    );
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::PermissionRequested.as_str())
    );
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::PermissionDecided.as_str())
    );
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::TurnCancelled.as_str())
    );
}

// ── Test 5: Duplicate Command (Operation ID Idempotency) ───────────

#[test]
fn duplicate_operation_id_is_acknowledged_without_reexecution() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials).unwrap();
    let dispatcher = CommandDispatcher::new();

    let profile = fixture_profile("agp_fixture000000005", "Claude");
    let binding = fixture_binding("hsb_fixture000000005", "agp_fixture000000005");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000005").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("Dedup Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();

    let open_cmd = CoreCommandEnvelope::new(
        "op_fixture000000030".parse().unwrap(),
        now,
        CoreCommand::OpenThread {
            thread_id,
            binding: Some(binding),
        },
    );

    // First issue: executes
    let res1 = dispatcher.dispatch(&mut app, open_cmd.clone()).unwrap();
    assert!(matches!(res1, CoreCommandResponse::ThreadOpened(_)));

    // Second issue with exact same operation ID: acknowledged duplicate
    let res2 = dispatcher.dispatch(&mut app, open_cmd).unwrap();
    assert_eq!(
        res2,
        CoreCommandResponse::DuplicateAcknowledged {
            operation_id: "op_fixture000000030".parse().unwrap(),
        }
    );
}

// ── Test 6: Status, Diagnostics & Restart Recovery Scan ─────────────

#[test]
fn status_diagnostics_and_restart_recovery_scan() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app = CoreApplication::open_in_memory(harness, credentials.clone()).unwrap();

    let profile = fixture_profile("agp_fixture000000006", "Claude");
    let binding = fixture_binding("hsb_fixture000000006", "agp_fixture000000006");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000006").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("Status & Diag Thread").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();

    app.open_thread(&thread_id, Some(&binding)).unwrap();

    // 1. Search threads by title
    let results = app
        .search_threads(
            &SearchQuery::try_from("Status").unwrap(),
            None,
            ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].thread_id, thread_id.as_str());

    // 2. Query Status
    let status = app.get_status().unwrap();
    assert_eq!(status.instance_id, credentials.instance_id);
    assert_eq!(status.active_thread_count, 1);
    assert_eq!(status.indeterminate_checkpoints, 0);

    // 3. Query Diagnostics
    let diag = app.get_diagnostics(Some(&thread_id)).unwrap();
    assert_eq!(
        diag.thread_states.get(&thread_id),
        Some(&SupervisorState::Ready)
    );

    // 4. Test restart recovery scan
    let report = app.on_startup().unwrap();
    assert_eq!(report.recovered_unsettled_intents, 0);
    assert_eq!(report.indeterminate_checkpoints_count, 0);
    assert!(report.resend_prohibited);
}

// ── Test 7: Prompt Harness Failure Marks Turn Failed ───────────────

#[test]
fn prompt_harness_failure_marks_turn_failed_and_not_active() {
    let credentials = fixture_credentials();
    let mut harness = MockHarness::new();
    harness.fail_prompt = true;
    let mut app = CoreApplication::open_in_memory(harness, credentials).unwrap();

    let profile = fixture_profile("agp_fixture000000007", "Claude");
    let binding = fixture_binding("hsb_fixture000000007", "agp_fixture000000007");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000007").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("Prompt Failure Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();

    let open_res = app.open_thread(&thread_id, Some(&binding)).unwrap();
    assert!(open_res.session_id.is_some());

    let op_id = OperationId::from_str("op_fixture000000077").unwrap();
    let turn_id = TurnId::from_str("trn_fixture000000077").unwrap();

    // Start prompt fails at harness send_prompt boundary
    let prompt_res = app.start_prompt(
        op_id,
        thread_id.clone(),
        turn_id.clone(),
        "Failing prompt",
        now,
    );
    assert!(prompt_res.is_err());

    // 1. Turn state in storage MUST NOT remain Active - it must be Failed
    let turns = app
        .get_thread_turns(
            &thread_id,
            None,
            TurnListLimit::try_new(TURN_LIST_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, turn_id.as_str());
    assert_eq!(turns[0].state, "failed");
    assert_ne!(turns[0].state, "active");

    // 2. Supervisor state is Crashed, not Prompting/Active
    let sup_state = app
        .supervisor()
        .supervisor(&thread_id)
        .unwrap()
        .state()
        .clone();
    assert!(matches!(sup_state, SupervisorState::Crashed { .. }));

    // 3. Domain history preserved TurnStarted and TurnFailed
    let history = app
        .get_thread_history(
            &thread_id,
            0,
            HistoryLimit::try_new(HISTORY_LIMIT_MAX).unwrap(),
        )
        .unwrap();
    let kinds: Vec<String> = history.into_iter().map(|row| row.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::TurnStarted.as_str())
    );
    assert!(
        kinds
            .iter()
            .any(|value| value == DomainEventKind::TurnFailed.as_str())
    );
}

// ── Test 8: Daemon Two Clients Sequential & Reconnect ──────────────

#[test]
fn test_daemon_two_clients_sequential_and_reconnect() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);

    // 1. Client 1 connects and handshakes
    let mut client1 = listener.create_client();
    let hello = desktop_hello(&credentials);
    client1.send_json(&hello).unwrap();

    let report1 = daemon.step(now).unwrap();
    assert_eq!(report1.accepted_connections, 1);

    let core_hello: altior_protocol::CoreHello = client1.recv_json().unwrap().unwrap();
    let greeting: altior_protocol::CoreGreeting = client1.recv_json().unwrap().unwrap();
    assert_eq!(core_hello.core_version, ProductVersion::new(0, 0, 1));
    assert_eq!(greeting.instance_id, credentials.instance_id);

    // 2. Client 1 subscribes
    client1.subscribe("op_fixture000000081", None, now).unwrap();
    daemon.step(now).unwrap();

    // 3. Client 1 sends CreateThread command
    let agent_id = AgentProfileId::from_str("agp_fixture000000001").unwrap();
    let op_create = OperationId::from_str("op_fixture000000082").unwrap();
    let limits = EnvelopeLimits::default();
    let create_envelope = CommandEnvelope::create_thread(
        agent_id,
        Some("Daemon Thread 1".to_string()),
        None,
        op_create.clone(),
        now,
        &limits,
    )
    .unwrap();
    client1.send_json(&create_envelope).unwrap();

    let report2 = daemon.step(now).unwrap();
    assert_eq!(report2.normal_commands_dispatched, 1);

    // Receive CommandResult for create_thread
    let res_envelope: EventEnvelope = client1.recv_json().unwrap().unwrap();
    match res_envelope.body {
        EventBody::Known(KnownEvent::CommandResult {
            operation_id,
            success,
            ..
        }) => {
            assert_eq!(operation_id, op_create);
            assert!(success);
        }
        other => panic!("expected CommandResult, got: {other:?}"),
    }

    // 4. Client 1 disconnects
    client1.disconnect();
    let report3 = daemon.step(now).unwrap();
    assert_eq!(report3.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);

    // 5. Client 2 reconnects and subscribes
    let mut client2 = listener.create_client();
    client2.send_json(&hello).unwrap();
    daemon.step(now).unwrap();

    let _core_hello: altior_protocol::CoreHello = client2.recv_json().unwrap().unwrap();
    let greeting2: altior_protocol::CoreGreeting = client2.recv_json().unwrap().unwrap();
    assert_eq!(greeting2.instance_id, credentials.instance_id);

    // Subscribe with since = None
    client2.subscribe("op_fixture000000083", None, now).unwrap();
    daemon.step(now).unwrap();
    assert_eq!(daemon.active_session_count(), 1);
    assert_eq!(daemon.subscribed_session_count(), 1);
}

// ── Test 9: Client Disconnect During Prompt & Second Client Catch-up ──

fn setup_disconnect_catchup_daemon(
    now: UnixMillis,
) -> (
    CoreDaemon<MockHarness>,
    altior_core::application::InMemoryListener,
    LaunchCredentials,
    ThreadId,
) {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let profile = fixture_profile("agp_fixture000000009", "Claude");
    let binding = fixture_binding("hsb_fixture000000009", "agp_fixture000000009");
    daemon
        .app_mut()
        .configure_agent(&profile, Some(&binding))
        .unwrap();

    let thread_id = ThreadId::from_str("thr_fixture000000009").unwrap();
    let title = ThreadTitle::try_from("Prompt Disconnect Test").unwrap();
    daemon
        .app_mut()
        .create_thread(
            thread_id.clone(),
            &binding.agent_profile_id,
            Some(&title),
            None,
            now,
        )
        .unwrap();

    let _ = daemon
        .app_mut()
        .open_thread(&thread_id, Some(&binding))
        .unwrap();

    (daemon, listener, credentials, thread_id)
}

#[test]
fn test_daemon_client_disconnect_during_prompt_harness_continues_second_client_catchup() {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let (mut daemon, listener, credentials, thread_id) = setup_disconnect_catchup_daemon(now);

    // 1. Client 1 connects and subscribes
    let mut client1 = listener.create_client();
    let hello = desktop_hello(&credentials);
    client1.send_json(&hello).unwrap();
    daemon.step(now).unwrap();
    let _: altior_protocol::CoreHello = client1.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client1.recv_json().unwrap().unwrap();

    client1.subscribe("op_fixture000000091", None, now).unwrap();
    daemon.step(now).unwrap();

    // 2. Client 1 starts a turn
    let op_turn = OperationId::from_str("op_fixture000000092").unwrap();
    let turn_id = TurnId::from_str("trn_fixture000000092").unwrap();
    let prompt = altior_protocol::MessageText::try_from("Hello daemon").unwrap();
    let limits = EnvelopeLimits::default();
    let turn_envelope = CommandEnvelope::start_turn(
        thread_id.clone(),
        Some(turn_id),
        prompt,
        op_turn.clone(),
        now,
        &limits,
    )
    .unwrap();
    client1.send_json(&turn_envelope).unwrap();

    // Step daemon: admits prompt and emits TurnStarted (sequence 1)
    daemon.step(now).unwrap();

    let cmd_res: EventEnvelope = client1.recv_json().unwrap().unwrap();
    assert!(matches!(
        cmd_res.body,
        EventBody::Known(KnownEvent::CommandResult { .. })
    ));

    let turn_started_ev: EventEnvelope = client1.recv_json().unwrap().unwrap();
    assert!(matches!(
        turn_started_ev.body,
        EventBody::Known(KnownEvent::TurnStarted)
    ));
    let seq_turn_started = turn_started_ev.sequence.as_u64();
    assert_eq!(seq_turn_started, 1);

    // 3. Client 1 disconnects abruptly!
    client1.disconnect();
    daemon.step(now).unwrap();
    assert_eq!(daemon.active_session_count(), 0);

    // 4. Mock harness produces streaming events and completion while NO client is connected
    let session_id = HarnessSessionId::new(&format!("sess_{thread_id}")).unwrap();
    daemon.app_mut().supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "Chunk A".to_string(),
        },
    );
    daemon.app_mut().supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "Chunk B".to_string(),
        },
    );
    daemon.app_mut().supervisor_mut().harness_mut().queue_event(
        &session_id,
        HarnessEvent::Completed {
            payload: Some(b"{\"status\":\"done\"}".to_vec().try_into().unwrap()),
        },
    );

    // Step daemon: pumps runtime events to SQLite storage and EventLog
    let pump_report = daemon.step(now).unwrap();
    assert_eq!(pump_report.events_published, 3); // 2 deltas + 1 completed

    // 5. Client 2 connects and subscribes with since = seq_turn_started (1)
    let mut client2 = listener.create_client();
    client2.send_json(&hello).unwrap();
    daemon.step(now).unwrap();
    let _: altior_protocol::CoreHello = client2.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client2.recv_json().unwrap().unwrap();

    client2
        .subscribe("op_fixture000000093", Some(seq_turn_started), now)
        .unwrap();
    daemon.step(now).unwrap();

    // Client 2 receives replayed events: delta 1, delta 2, turn completed, then stream.replayed!
    let ev1: EventEnvelope = client2.recv_json().unwrap().unwrap();
    let ev2: EventEnvelope = client2.recv_json().unwrap().unwrap();
    let ev3: EventEnvelope = client2.recv_json().unwrap().unwrap();
    let boundary: EventEnvelope = client2.recv_json().unwrap().unwrap();

    assert!(matches!(
        ev1.body,
        EventBody::Known(KnownEvent::MessageDelta { .. })
    ));
    assert!(matches!(
        ev2.body,
        EventBody::Known(KnownEvent::MessageDelta { .. })
    ));
    assert!(matches!(
        ev3.body,
        EventBody::Known(KnownEvent::TurnCompleted)
    ));
    assert!(
        matches!(boundary.body, EventBody::Known(KnownEvent::StreamReplayed { from, through }) if from.as_u64() == 2 && through.as_u64() == 4)
    );
}

// ── Test 10: Duplicate Operation ID ─────────────────────────────────

#[test]
fn test_daemon_duplicate_operation() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);
    let mut client = listener.create_client();
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();
    daemon.step(now).unwrap();
    let _: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();

    client.subscribe("op_fixture000000101", None, now).unwrap();
    daemon.step(now).unwrap();

    let agent_id = AgentProfileId::from_str("agp_fixture000000001").unwrap();
    let op_dup = OperationId::from_str("op_fixture000000102").unwrap();
    let limits = EnvelopeLimits::default();
    let env1 = CommandEnvelope::create_thread(
        agent_id.clone(),
        Some("Dup Thread".to_string()),
        None,
        op_dup.clone(),
        now,
        &limits,
    )
    .unwrap();

    // First delivery
    client.send_json(&env1).unwrap();
    daemon.step(now).unwrap();
    let res1: EventEnvelope = client.recv_json().unwrap().unwrap();
    assert!(matches!(
        res1.body,
        EventBody::Known(KnownEvent::CommandResult { .. })
    ));

    // Second delivery with identical operation_id
    let env2 = CommandEnvelope::create_thread(
        agent_id,
        Some("Dup Thread".to_string()),
        None,
        op_dup,
        now,
        &limits,
    )
    .unwrap();
    client.send_json(&env2).unwrap();
    daemon.step(now).unwrap();
    let res2: EventEnvelope = client.recv_json().unwrap().unwrap();
    assert!(matches!(
        res2.body,
        EventBody::Known(KnownEvent::CommandResult { .. })
    ));
}

// ── Test 11: Bad Auth Rejected ──────────────────────────────────────

#[test]
fn test_daemon_bad_auth_rejected() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);

    // 1. Client-first verification: newly connected client receives nothing before sending DesktopHello
    let mut client = listener.create_client();
    let init_report = daemon.step(now).unwrap();
    assert_eq!(init_report.accepted_connections, 1);
    assert_eq!(daemon.active_session_count(), 1);
    assert!(client.recv_frame().unwrap().is_none());

    // 2. Client sends bad authentication token
    let mut bad_token_entropy = [0u8; 16];
    bad_token_entropy[0] = 0xff;
    let bad_token = mint_launch_token(&bad_token_entropy).unwrap();

    let bad_hello = DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .unwrap(),
        desktop_version: "0.1.0".parse().unwrap(),
        capabilities: CapabilitySet::new(),
        launch_token: bad_token,
    };

    client.send_json(&bad_hello).unwrap();

    // Step daemon: authentication fails, connection is closed and evicted
    let report = daemon.step(now).unwrap();
    assert_eq!(report.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);

    // Verify client receives nothing (no information leak, no CoreHello, no CoreGreeting)
    assert!(client.recv_frame().unwrap().is_none());
    let reply: Option<altior_protocol::CoreHello> = client.recv_json().unwrap();
    assert!(reply.is_none());
}

// ── Test 12: Malformed and Oversized Frames Rejected ────────────────

#[test]
fn test_daemon_malformed_and_oversized_rejected() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);

    // 1. Malformed JSON bytes
    let mut client1 = listener.create_client();
    client1.send_raw_bytes(b"NOT_VALID_JSON_GARBAGE").unwrap();

    let report1 = daemon.step(now).unwrap();
    assert_eq!(report1.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);

    // 2. Oversized payload (> 256 KiB)
    let mut client2 = listener.create_client();
    let oversized = vec![0x41; 300 * 1024]; // 300 KiB
    client2.send_raw_bytes(&oversized).unwrap();

    let report2 = daemon.step(now).unwrap();
    assert_eq!(report2.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);
}

// ── Test 13: Priority Control Commands ──────────────────────────────

#[test]
fn test_daemon_priority_control_commands() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);
    let mut client = listener.create_client();
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();
    daemon.step(now).unwrap();
    let _: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();

    // Send Ping command
    let op_ping = OperationId::from_str("op_fixture000000131").unwrap();
    let ping_cmd = CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: op_ping.clone(),
        kind: altior_protocol::CommandKind::Ping,
        payload: None,
        issued_at: now,
    };
    client.send_json(&ping_cmd).unwrap();

    let report = daemon.step(now).unwrap();
    assert_eq!(report.control_commands_dispatched, 1);

    let res: EventEnvelope = client.recv_json().unwrap().unwrap();
    match res.body {
        EventBody::Known(KnownEvent::CommandResult {
            operation_id,
            success,
            ..
        }) => {
            assert_eq!(operation_id, op_ping);
            assert!(success);
        }
        other => panic!("expected CommandResult for ping, got {other:?}"),
    }
}

// ── Test 14: Daemon Shutdown ────────────────────────────────────────

#[test]
fn test_daemon_shutdown() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let now = UnixMillis::from_millis(1_700_000_000_000);
    let mut client = listener.create_client();
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();
    daemon.step(now).unwrap();

    assert_eq!(daemon.active_session_count(), 1);

    // Shutdown daemon
    daemon.shutdown().unwrap();
    assert_eq!(daemon.active_session_count(), 0);
    assert!(
        !daemon
            .stop_handle()
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

// ── Test 15: Daemon CLI Args Parsing ────────────────────────────────

#[test]
fn test_daemon_config_cli_parsing() {
    let args = vec![
        "altior-core".to_string(),
        "--daemon".to_string(),
        "--data-dir=/tmp/altior-data".to_string(),
        "--endpoint=altior_pipe_test".to_string(),
        "--discovery=/tmp/discovery.json".to_string(),
    ];
    let config = CoreDaemonConfig::parse_args(&args).unwrap();
    assert!(config.is_daemon);
    assert_eq!(
        config.data_dir,
        Some(std::path::PathBuf::from("/tmp/altior-data"))
    );
    assert_eq!(config.endpoint, Some("altior_pipe_test".to_string()));
    assert_eq!(
        config.discovery_path,
        Some(std::path::PathBuf::from("/tmp/discovery.json"))
    );

    let banner_args = vec!["altior-core".to_string()];
    let banner_config = CoreDaemonConfig::parse_args(&banner_args).unwrap();
    assert!(!banner_config.is_daemon);
}

// ── Deterministic Test Clock ────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct FakeClock {
    current: u64,
}

impl FakeClock {
    fn new(start_millis: u64) -> Self {
        Self {
            current: start_millis,
        }
    }

    fn now(self) -> UnixMillis {
        UnixMillis::from_millis(self.current)
    }

    fn advance(&mut self, millis: u64) -> UnixMillis {
        self.current += millis;
        UnixMillis::from_millis(self.current)
    }
}

// ── Test 16: Unauthenticated Client No Data Times Out & Evicted ─────

#[test]
fn test_daemon_unauthenticated_no_data_times_out_and_evicted() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials).unwrap();

    let mut clock = FakeClock::new(1_700_000_000_000);

    // 1. Client connects at t=0
    let mut client = listener.create_client();
    let report1 = daemon.step(clock.now()).unwrap();
    assert_eq!(report1.accepted_connections, 1);
    assert_eq!(daemon.active_session_count(), 1);

    // Client sends NO data
    // 2. Advance to 4999 ms (just before 5000ms deadline)
    clock.advance(4999);
    let report2 = daemon.step(clock.now()).unwrap();
    assert_eq!(report2.closed_connections, 0);
    assert_eq!(daemon.active_session_count(), 1);

    // 3. Advance to 5000 ms (exact deadline)
    clock.advance(1);
    let report3 = daemon.step(clock.now()).unwrap();
    assert_eq!(report3.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);

    // 4. Verify client received zero auth data or information leak
    assert!(client.recv_frame().unwrap().is_none());
    assert!(client.is_closed());
}

// ── Test 17: Unauthenticated Partial & Malformed Hello Rejected ──────

#[test]
fn test_daemon_unauthenticated_partial_and_malformed_hello() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials).unwrap();

    let clock = FakeClock::new(1_700_000_000_000);

    // Case 1: Client sends truncated / partial JSON
    let mut client_partial = listener.create_client();
    let report1 = daemon.step(clock.now()).unwrap();
    assert_eq!(report1.accepted_connections, 1);
    assert_eq!(daemon.active_session_count(), 1);

    client_partial
        .send_raw_bytes(b"{\"desktop_version\":\"0.1.0\",\"launch_token\":")
        .unwrap();
    let report2 = daemon.step(clock.now()).unwrap();
    assert_eq!(report2.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);
    assert!(client_partial.recv_frame().unwrap().is_none());

    // Case 2: Client sends invalid hello payload (wrong schema)
    let mut client_schema_err = listener.create_client();
    daemon.step(clock.now()).unwrap();
    assert_eq!(daemon.active_session_count(), 1);

    client_schema_err
        .send_raw_bytes(b"{\"unknown_field\": 12345}")
        .unwrap();
    let report3 = daemon.step(clock.now()).unwrap();
    assert_eq!(report3.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);
    assert!(client_schema_err.recv_frame().unwrap().is_none());
}

// ── Test 18: Valid Hello Just Before Deadline Succeeds ──────────────

#[test]
fn test_daemon_valid_hello_just_before_deadline_succeeds() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let mut clock = FakeClock::new(1_700_000_000_000);

    // 1. Client connects at t=0
    let mut client = listener.create_client();
    daemon.step(clock.now()).unwrap();
    assert_eq!(daemon.active_session_count(), 1);

    // 2. Advance to 4999 ms (1ms before 5000ms deadline)
    clock.advance(4999);
    // Send valid DesktopHello right before deadline
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();

    let report = daemon.step(clock.now()).unwrap();
    assert_eq!(report.closed_connections, 0);
    assert_eq!(daemon.active_session_count(), 1);

    // Client receives CoreHello + CoreGreeting
    let core_hello: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let greeting: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();
    assert_eq!(greeting.instance_id, credentials.instance_id);
    assert_eq!(core_hello.core_version, ProductVersion::new(0, 0, 1));

    // 3. Advance past deadline (e.g. +5000 ms more, total 9999 ms)
    clock.advance(5000);
    let report_later = daemon.step(clock.now()).unwrap();
    assert_eq!(report_later.closed_connections, 0);
    assert_eq!(daemon.active_session_count(), 1);

    // 4. Authenticated client can subscribe and operate normally
    client
        .subscribe("op_fixture000000181", None, clock.now())
        .unwrap();
    daemon.step(clock.now()).unwrap();
    assert_eq!(daemon.subscribed_session_count(), 1);
}

// ── Test 19: Multiple Clients Slow Pruned Authenticated Unaffected ───

#[test]
fn test_daemon_multiple_clients_slow_pruned_authenticated_unaffected() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();

    let mut clock = FakeClock::new(1_700_000_000_000);

    // 1. Client 1 (fast/legit) and Client 2 (slow/unauthenticated) connect at t=0
    let mut client1 = listener.create_client();
    let mut client2 = listener.create_client();

    let hello = desktop_hello(&credentials);
    client1.send_json(&hello).unwrap();
    // client2 sends nothing!

    let report1 = daemon.step(clock.now()).unwrap();
    assert_eq!(report1.accepted_connections, 2);
    assert_eq!(daemon.active_session_count(), 2);

    // Client 1 receives greeting and subscribes
    let _: altior_protocol::CoreHello = client1.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client1.recv_json().unwrap().unwrap();
    client1
        .subscribe("op_fixture000000191", None, clock.now())
        .unwrap();
    daemon.step(clock.now()).unwrap();
    assert_eq!(daemon.subscribed_session_count(), 1);

    // 2. Advance clock to 5000 ms (deadline for Client 2)
    clock.advance(5000);
    let report2 = daemon.step(clock.now()).unwrap();
    // Client 2 is pruned!
    assert_eq!(report2.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 1);
    assert_eq!(daemon.subscribed_session_count(), 1);
    assert!(client2.recv_frame().unwrap().is_none());
    assert!(client2.is_closed());

    // 3. Client 1 remains fully functional and sends Ping command
    let op_ping = OperationId::from_str("op_fixture000000192").unwrap();
    let ping_cmd = CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: op_ping.clone(),
        kind: altior_protocol::CommandKind::Ping,
        payload: None,
        issued_at: clock.now(),
    };
    client1.send_json(&ping_cmd).unwrap();

    let report3 = daemon.step(clock.now()).unwrap();
    assert_eq!(report3.control_commands_dispatched, 1);

    let res: EventEnvelope = client1.recv_json().unwrap().unwrap();
    match res.body {
        EventBody::Known(KnownEvent::CommandResult {
            operation_id,
            success,
            ..
        }) => {
            assert_eq!(operation_id, op_ping);
            assert!(success);
        }
        other => panic!("expected CommandResult, got {other:?}"),
    }
}

// ── Test 20: Max Client Sessions Backpressure ───────────────────────

#[test]
fn test_daemon_max_client_sessions_backpressure() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (daemon_raw, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();
    // Cap to 2 max sessions
    let mut daemon = daemon_raw.with_max_client_sessions(2);

    let now = UnixMillis::from_millis(1_700_000_000_000);

    // Create 3 client connections
    let mut client1 = listener.create_client();
    let mut client2 = listener.create_client();
    let mut client3 = listener.create_client();

    let hello = desktop_hello(&credentials);

    // Step daemon: only 2 clients are accepted, 3rd is held in listener pending queue
    let report1 = daemon.step(now).unwrap();
    assert_eq!(report1.accepted_connections, 2);
    assert_eq!(daemon.active_session_count(), 2);

    // Another step: still 2 sessions, client 3 still backpressured
    let report2 = daemon.step(now).unwrap();
    assert_eq!(report2.accepted_connections, 0);
    assert_eq!(daemon.active_session_count(), 2);

    // Client 1 handshakes and disconnects
    client1.send_json(&hello).unwrap();
    daemon.step(now).unwrap();
    let _: altior_protocol::CoreHello = client1.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client1.recv_json().unwrap().unwrap();
    client1.disconnect();

    // Step daemon: Client 1 is closed/evicted, freeing a slot; Client 3 is accepted!
    let report3 = daemon.step(now).unwrap();
    assert_eq!(report3.closed_connections, 1);
    assert_eq!(report3.accepted_connections, 1);
    assert_eq!(daemon.active_session_count(), 2);

    // Client 2 and Client 3 can handshake
    client2.send_json(&hello).unwrap();
    client3.send_json(&hello).unwrap();
    daemon.step(now).unwrap();

    let _: altior_protocol::CoreHello = client2.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client2.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreHello = client3.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client3.recv_json().unwrap().unwrap();
}

// ── Test 21: Authenticated Idle Timeout If Configured ───────────────

#[test]
fn test_daemon_authenticated_idle_timeout_if_configured() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (daemon_raw, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();
    // Configure 10-second idle timeout
    let mut daemon = daemon_raw.with_idle_timeout(Some(std::time::Duration::from_millis(10_000)));

    let mut clock = FakeClock::new(1_700_000_000_000);

    // 1. Client connects and authenticates at t=0
    let mut client = listener.create_client();
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();
    daemon.step(clock.now()).unwrap();
    let _: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();

    assert_eq!(daemon.active_session_count(), 1);

    // 2. Activity at t=4000 ms: client sends Ping
    clock.advance(4000);
    let op_ping = OperationId::from_str("op_fixture000000211").unwrap();
    let ping_cmd = CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: op_ping,
        kind: altior_protocol::CommandKind::Ping,
        payload: None,
        issued_at: clock.now(),
    };
    client.send_json(&ping_cmd).unwrap();
    daemon.step(clock.now()).unwrap();
    let _: EventEnvelope = client.recv_json().unwrap().unwrap();

    // 3. Advance to t=13_999 ms (9999 ms since activity at 4000)
    clock.advance(9999);
    let report_active = daemon.step(clock.now()).unwrap();
    assert_eq!(report_active.closed_connections, 0);
    assert_eq!(daemon.active_session_count(), 1);

    // 4. Advance to t=14_000 ms (10_000 ms since activity at 4000)
    clock.advance(1);
    let report_idle = daemon.step(clock.now()).unwrap();
    assert_eq!(report_idle.closed_connections, 1);
    assert_eq!(daemon.active_session_count(), 0);
    assert!(client.is_closed());
}
