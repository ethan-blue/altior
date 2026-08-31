//! P1.2 ACP runtime acceptance and unit tests for Lane C.
//!
//! Validates:
//! 1. `create` and `resume` sessions with capability inspection
//! 2. stream event handling and state transitions
//! 3. permission pause and decision flow
//! 4. cancel race and retry idempotence
//! 5. unexpected subprocess exit transitions active turn to `Failed` + `Indeterminate`
//! 6. indeterminate prompt delivery strictly forbids automatic resend
//! 7. multiple threads bound with isolated state machines and bounded active operations
//! 8. capability gate checks return typed errors
//! 9. closing UI / desktop lifecycle events never interrupt running turns (proven by test double)
//! 10. secret redaction in bounded diagnostics and unknown event handling without panic

use std::collections::{BTreeMap, VecDeque};
use std::str::FromStr;

use altior_core::ownership::DesktopLifecycle;
use altior_core::runtime::{
    AgentRuntime, AgentRuntimeSupervisor, BindingProbeOutcome, CancelOutcome, CheckpointError,
    CheckpointIntent, CheckpointSettled, HarnessError, HarnessEvent, HarnessPromptRequest,
    HarnessRuntimePort, HarnessSessionId, HarnessSessionInfo, RuntimeCheckpointPort, RuntimeError,
    RuntimeEvent, SupervisorState, TurnAdmission,
};
use altior_domain::{
    AcpHarnessBinding, BoundedPath, DeliveryState, DisplayName, DomainEvent, EventId, OperationId,
    PermissionDecision, PermissionDescription, PermissionKind, ProjectRef, ThreadId, TurnId,
    TurnState, UnixMillis,
};
use altior_protocol::{CapabilityId, CapabilitySet, CapabilitySupport};

// ── Test Doubles: Fake Harness and Fake Checkpoint Ports ───────────

#[derive(Debug, Default)]
struct FakeHarness {
    probe_outcome: Option<Result<BindingProbeOutcome, HarnessError>>,
    capabilities: CapabilitySet,
    created_sessions: Vec<(HarnessBindingIdStr, ThreadId)>,
    resumed_sessions: Vec<(HarnessSessionId, ThreadId)>,
    sent_prompts: Vec<(HarnessSessionId, HarnessPromptRequest)>,
    cancelled_sessions: Vec<HarnessSessionId>,
    decided_permissions: Vec<(HarnessSessionId, EventId, PermissionDecision)>,
    closed_sessions: Vec<HarnessSessionId>,
    events: BTreeMap<HarnessSessionId, VecDeque<HarnessEvent>>,
    prompt_error: Option<HarnessError>,
}

type HarnessBindingIdStr = String;

impl FakeHarness {
    fn new() -> Self {
        Self::default()
    }

    fn with_capabilities(mut self, caps: CapabilitySet) -> Self {
        self.capabilities = caps;
        self
    }

    fn queue_event(&mut self, session_id: &HarnessSessionId, event: HarnessEvent) {
        self.events
            .entry(session_id.clone())
            .or_default()
            .push_back(event);
    }
}

impl HarnessRuntimePort for FakeHarness {
    fn probe_binding(
        &mut self,
        _binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, HarnessError> {
        if let Some(outcome) = &self.probe_outcome {
            outcome.clone()
        } else {
            Ok(BindingProbeOutcome {
                ok: true,
                capabilities: self.capabilities.clone(),
                diagnostics: None,
            })
        }
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
        thread_id: &ThreadId,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        self.resumed_sessions
            .push((session_id.clone(), thread_id.clone()));
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
        if let Some(err) = self.prompt_error.take() {
            return Err(err);
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

    fn close_session(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        self.closed_sessions.push(session_id.clone());
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeCheckpoint {
    intents: Vec<CheckpointIntent>,
    settled: Vec<CheckpointSettled>,
    events: Vec<DomainEvent>,
}

impl RuntimeCheckpointPort for FakeCheckpoint {
    fn checkpoint_intent(&mut self, intent: &CheckpointIntent) -> Result<(), CheckpointError> {
        self.intents.push(intent.clone());
        Ok(())
    }

    fn settle_checkpoint(&mut self, settled: &CheckpointSettled) -> Result<(), CheckpointError> {
        self.settled.push(settled.clone());
        Ok(())
    }

    fn record_event(&mut self, event: &DomainEvent) -> Result<(), CheckpointError> {
        self.events.push(event.clone());
        Ok(())
    }
}

// ── Fixture Helpers ────────────────────────────────────────────────

fn sample_binding(id_num: u32) -> AcpHarnessBinding {
    AcpHarnessBinding {
        id: altior_domain::HarnessBindingId::from_str(&format!("hsb_fixture{id_num:09}")).unwrap(),
        agent_profile_id: altior_domain::AgentProfileId::from_str("agp_fixture000000001").unwrap(),
        label: DisplayName::try_from("Test Agent").unwrap(),
        command: BoundedPath::try_from("npx -y @modelcontextprotocol/server-everything").unwrap(),
        created_at: UnixMillis::from_millis(0),
    }
}

fn sample_thread(id_num: u32) -> ThreadId {
    ThreadId::from_str(&format!("thr_fixture{id_num:09}")).unwrap()
}

fn sample_turn(id_num: u32) -> TurnId {
    TurnId::from_str(&format!("trn_fixture{id_num:09}")).unwrap()
}

fn sample_operation(id_num: u32) -> OperationId {
    OperationId::from_str(&format!("op_fixture{id_num:09}")).unwrap()
}

fn sample_event(id_num: u32) -> EventId {
    EventId::from_str(&format!("evt_fixture{id_num:09}")).unwrap()
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn test_create_and_resume_session() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread_1 = sample_thread(1);

    // 1. Configure and test binding probe
    let probe = runtime.configure_and_test_binding(&binding).unwrap();
    assert!(probe.ok);

    // 2. Create session
    let session_id = runtime.create_session(&binding, &thread_1, None).unwrap();
    assert_eq!(session_id.as_str(), "sess_thr_fixture000000001");
    assert_eq!(
        runtime.supervisor(&thread_1).unwrap().state(),
        &SupervisorState::Ready
    );

    // Creating again on same thread fails
    assert!(matches!(
        runtime.create_session(&binding, &thread_1, None),
        Err(RuntimeError::SessionAlreadyActive(_))
    ));

    // 3. Resume session on another thread
    let thread_2 = sample_thread(2);
    let session_2 = HarnessSessionId::new("sess_thr_fixture000000002").unwrap();
    runtime
        .resume_session(&binding, &thread_2, &session_2)
        .unwrap();
    assert_eq!(
        runtime.supervisor(&thread_2).unwrap().state(),
        &SupervisorState::Ready
    );

    // 4. Close session cleanly
    runtime.close_session(&thread_1).unwrap();
    assert_eq!(
        runtime.supervisor(&thread_1).unwrap().state(),
        &SupervisorState::Closed
    );
    assert_eq!(runtime.harness().closed_sessions.len(), 1);
}

#[test]
fn test_streaming_event_flow() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);

    // Queue streaming events in fake harness
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::Started {
            turn_id: turn_id.clone(),
        },
    );
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "Hello ".to_string(),
        },
    );
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "world!".to_string(),
        },
    );
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });

    // Send prompt
    let admission = runtime
        .prompt(&thread, op_id.clone(), turn_id.clone(), "say hello")
        .unwrap();
    assert_eq!(admission, TurnAdmission::Admitted);

    // Verify boundary checkpoint intent was recorded BEFORE execution
    assert!(runtime.checkpoint().intents.iter().any(|i| matches!(
        i,
        CheckpointIntent::Prompt { turn_id: t, operation_id: o, .. } if t == &turn_id && o == &op_id
    )));

    // Poll stream events
    let ev1 = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev1, RuntimeEvent::TurnStarted { .. }));

    let ev2 = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev2, RuntimeEvent::MessageDelta { text, .. } if text == "Hello "));

    let ev3 = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev3, RuntimeEvent::MessageDelta { text, .. } if text == "world!"));

    let ev4 = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev4, RuntimeEvent::TurnCompleted { .. }));

    // Verify checkpoint settlement was recorded
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::TurnTerminal {
            state: TurnState::Completed,
            delivery: DeliveryState::Confirmed,
            ..
        }
    )));

    // State machine is back to Ready
    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );
}

#[test]
fn test_permission_pause_and_decision() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);
    let perm_id = sample_event(10);

    // Queue permission request followed by completion
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::PermissionRequest {
            event_id: perm_id.clone(),
            kind: PermissionKind::Execute,
            description: PermissionDescription::try_from("execute cargo build").unwrap(),
        },
    );
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });

    runtime
        .prompt(&thread, op_id.clone(), turn_id.clone(), "build project")
        .unwrap();

    // Poll stream -> permission requested
    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(
        ev,
        RuntimeEvent::PermissionRequested {
            permission_id,
            kind: PermissionKind::Execute,
            ..
        } if permission_id == perm_id
    ));

    // Supervisor state is AwaitingPermission
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::AwaitingPermission { pending_permission_id, .. } if pending_permission_id == &perm_id
    ));

    // Trying to prompt another turn while awaiting permission is blocked (bounded operations)
    assert!(matches!(
        runtime.prompt(&thread, sample_operation(2), sample_turn(2), "another"),
        Err(RuntimeError::ActiveOperationInProgress { .. })
    ));

    // Submit permission decision: Approved
    runtime
        .decide_permission(&thread, &perm_id, PermissionDecision::Approved)
        .unwrap();

    // Verify intent & settlement for permission
    assert!(runtime.checkpoint().intents.iter().any(|i| matches!(
        i,
        CheckpointIntent::PermissionDecision { permission_id, decision: PermissionDecision::Approved, .. }
            if permission_id == &perm_id
    )));
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::PermissionSettled { permission_id, decision: PermissionDecision::Approved, .. }
            if permission_id == &perm_id
    )));

    // Poll stream -> Completed
    let comp = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(comp, RuntimeEvent::TurnCompleted { .. }));
    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );
}

#[test]
fn test_cancel_race_and_idempotence() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);

    runtime
        .prompt(&thread, op_id.clone(), turn_id.clone(), "long task")
        .unwrap();
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::Prompting { .. }
    ));

    // 1. First cancel request
    let outcome1 = runtime.steer_cancel(&thread, Some(&op_id)).unwrap();
    assert_eq!(outcome1, CancelOutcome::CancelledActive);
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::Cancelling { .. }
    ));

    // 2. Duplicate cancel request in race is idempotent
    let outcome2 = runtime.steer_cancel(&thread, Some(&op_id)).unwrap();
    assert_eq!(outcome2, CancelOutcome::AlreadyCancelling);

    // 3. Harness delivers Cancelled event
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Cancelled);
    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev, RuntimeEvent::TurnCancelled { .. }));

    // Settle terminal cancelled
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::TurnTerminal {
            state: TurnState::Cancelled,
            delivery: DeliveryState::Confirmed,
            ..
        }
    )));

    // Supervisor back to Ready
    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );

    // Cancel when ready returns NoActiveTurn
    let outcome3 = runtime.steer_cancel(&thread, None).unwrap();
    assert_eq!(outcome3, CancelOutcome::NoActiveTurn);
}

#[test]
fn test_unexpected_exit_marks_indeterminate_and_forbids_resend() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);

    runtime
        .prompt(&thread, op_id.clone(), turn_id.clone(), "do work")
        .unwrap();

    // Simulate unexpected subprocess exit with code 137 (OOM / SIGKILL)
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::ProcessExited {
            exit_code: Some(137),
        },
    );

    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(
        ev,
        RuntimeEvent::ProcessExited {
            exit_code: Some(137),
            ..
        }
    ));

    // Supervisor state is Crashed with Indeterminate delivery
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::Crashed {
            delivery: DeliveryState::Indeterminate,
            ..
        }
    ));

    // Settlement recorded Failed + Indeterminate
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::TurnTerminal {
            turn_id: t,
            state: TurnState::Failed,
            delivery: DeliveryState::Indeterminate,
            ..
        } if t == &turn_id
    )));

    // Boundary discipline: Automatic prompt resend of indeterminate turn is STRICTLY FORBIDDEN
    // 1. Resend with same operation_id and same indeterminate turn
    let resend_attempt_same_op = runtime.prompt(&thread, op_id.clone(), turn_id.clone(), "do work");
    assert!(matches!(
        resend_attempt_same_op,
        Err(RuntimeError::AutomaticResendForbidden {
            turn_id: t,
            delivery: DeliveryState::Indeterminate,
            ..
        }) if t == turn_id
    ));

    // 2. Resend with fresh operation_id for same indeterminate turn (auto-resend attempt)
    let resend_attempt = runtime.prompt(&thread, sample_operation(2), turn_id.clone(), "do work");
    assert!(matches!(
        resend_attempt,
        Err(RuntimeError::AutomaticResendForbidden {
            turn_id: t,
            delivery: DeliveryState::Indeterminate,
            ..
        }) if t == turn_id
    ));

    // 3. Clearly distinguished from a fresh turn: new turn on crashed session is rejected as SessionNotReady
    let new_turn_id = sample_turn(2);
    let new_turn_attempt = runtime.prompt(&thread, sample_operation(3), new_turn_id, "fresh work");
    assert!(matches!(
        new_turn_attempt,
        Err(RuntimeError::SessionNotReady { .. })
    ));
}

#[test]
fn test_multiple_threads_bounded_isolation() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread1 = sample_thread(1);
    let thread2 = sample_thread(2);

    let _sess1 = runtime.create_session(&binding, &thread1, None).unwrap();
    let _sess2 = runtime.create_session(&binding, &thread2, None).unwrap();

    // Start turn on Thread 1
    runtime
        .prompt(&thread1, sample_operation(1), sample_turn(1), "t1 prompt")
        .unwrap();

    // Thread 2 can run independently
    runtime
        .prompt(&thread2, sample_operation(2), sample_turn(2), "t2 prompt")
        .unwrap();

    assert!(matches!(
        runtime.supervisor(&thread1).unwrap().state(),
        SupervisorState::Prompting { .. }
    ));
    assert!(matches!(
        runtime.supervisor(&thread2).unwrap().state(),
        SupervisorState::Prompting { .. }
    ));

    // Per-thread active operation bounds: thread 1 rejects second concurrent prompt
    assert!(matches!(
        runtime.prompt(
            &thread1,
            sample_operation(3),
            sample_turn(3),
            "t1 concurrent"
        ),
        Err(RuntimeError::ActiveOperationInProgress { .. })
    ));
}

#[test]
fn test_capability_gates_typed_error() {
    let mut caps = CapabilitySet::new();
    caps.declare("session.cancel", CapabilitySupport::Unsupported)
        .unwrap();
    caps.declare("session.resume", CapabilitySupport::Unsupported)
        .unwrap();

    let harness = FakeHarness::new().with_capabilities(caps);
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);

    // 1. Resume rejected when capability unsupported
    let resume_err =
        runtime.resume_session(&binding, &thread, &HarnessSessionId::new("sess_1").unwrap());
    assert!(matches!(
        resume_err,
        Err(RuntimeError::UnsupportedCapability(cap)) if cap == CapabilityId::from_str("session.resume").unwrap()
    ));

    // 2. Create session and start turn
    let _ = runtime.create_session(&binding, &thread, None).unwrap();
    runtime
        .prompt(&thread, sample_operation(1), sample_turn(1), "task")
        .unwrap();

    // 3. Cancel rejected when session.cancel unsupported
    let cancel_err = runtime.steer_cancel(&thread, None);
    assert!(matches!(
        cancel_err,
        Err(RuntimeError::UnsupportedCapability(cap)) if cap == CapabilityId::from_str("session.cancel").unwrap()
    ));
}

#[test]
fn test_closing_ui_does_not_affect_runtime() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    runtime
        .prompt(&thread, sample_operation(1), turn_id.clone(), "work")
        .unwrap();

    let supervisor = runtime.supervisor_mut(&thread).unwrap();

    // Desktop UI reload, close window, or exit
    for lifecycle_event in [
        DesktopLifecycle::Reload,
        DesktopLifecycle::WindowClosed,
        DesktopLifecycle::DesktopExited,
    ] {
        let transition = supervisor.on_desktop_lifecycle(lifecycle_event);
        assert_eq!(
            transition,
            altior_core::ownership::TurnTransition::StillRunning
        );
        assert!(matches!(
            supervisor.state(),
            SupervisorState::Prompting { .. }
        ));
    }

    // Now finish the turn normally
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });
    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(ev, RuntimeEvent::TurnCompleted { .. }));
    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );
}

#[test]
fn test_diagnostics_redaction_and_unknown_events() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    // Unknown event with sensitive Bearer and API key data
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::RawUnknown {
            name: "agent/custom_telemetry".to_string(),
            data: "Authorization: Bearer my-secret-jwt-token-12345 with sk-proj123456789 and password=topsecret"
                .to_string(),
        },
    );

    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    match ev {
        RuntimeEvent::Unknown { name, summary, .. } => {
            assert_eq!(name, "agent/custom_telemetry");
            let text = summary.as_str();
            assert!(!text.contains("my-secret-jwt-token-12345"));
            assert!(!text.contains("proj123456789"));
            assert!(!text.contains("topsecret"));
            assert!(text.contains("[REDACTED]"));
        }
        other => panic!("expected RuntimeEvent::Unknown, got {other:?}"),
    }
}

#[test]
fn test_indeterminate_failed_event_marks_crashed_and_forbids_resend() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);

    runtime
        .prompt(&thread, op_id.clone(), turn_id.clone(), "process work")
        .unwrap();

    // Harness produces failed event with Indeterminate delivery
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::Failed {
            error: "connection reset by peer".to_string(),
            delivery: DeliveryState::Indeterminate,
        },
    );

    let ev = runtime.poll_stream(&thread).unwrap().unwrap();
    assert!(matches!(
        ev,
        RuntimeEvent::TurnFailed {
            delivery: DeliveryState::Indeterminate,
            ..
        }
    ));

    // State is Crashed
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::Crashed {
            delivery: DeliveryState::Indeterminate,
            ..
        }
    ));

    // Checkpoint records Failed + Indeterminate
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::TurnTerminal {
            turn_id: t,
            state: TurnState::Failed,
            delivery: DeliveryState::Indeterminate,
            ..
        } if t == &turn_id
    )));

    // Auto-resend of indeterminate turn forbidden
    let resend_err = runtime.prompt(&thread, sample_operation(2), turn_id, "process work");
    assert!(matches!(
        resend_err,
        Err(RuntimeError::AutomaticResendForbidden {
            delivery: DeliveryState::Indeterminate,
            ..
        })
    ));

    // Fresh turn on crashed session is rejected as SessionNotReady
    let new_turn_err = runtime.prompt(&thread, sample_operation(3), sample_turn(2), "new turn");
    assert!(matches!(
        new_turn_err,
        Err(RuntimeError::SessionNotReady { .. })
    ));
}

#[test]
fn test_transport_failure_on_prompt_marks_crashed_and_forbids_resend() {
    let mut harness = FakeHarness::new();
    harness.prompt_error = Some(HarnessError::Transport("broken pipe".to_string()));
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let _session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_id = sample_turn(1);
    let op_id = sample_operation(1);

    let prompt_res = runtime.prompt(&thread, op_id.clone(), turn_id.clone(), "send task");
    assert!(matches!(
        prompt_res,
        Err(RuntimeError::Harness(HarnessError::Transport(_)))
    ));

    // Supervisor state is Crashed
    assert!(matches!(
        runtime.supervisor(&thread).unwrap().state(),
        SupervisorState::Crashed {
            delivery: DeliveryState::Indeterminate,
            ..
        }
    ));

    // Checkpoint recorded TurnTerminal Failed + Indeterminate
    assert!(runtime.checkpoint().settled.iter().any(|s| matches!(
        s,
        CheckpointSettled::TurnTerminal {
            turn_id: t,
            state: TurnState::Failed,
            delivery: DeliveryState::Indeterminate,
            ..
        } if t == &turn_id
    )));

    // Resend attempt for the failed prompt is forbidden
    let resend_res = runtime.prompt(&thread, sample_operation(2), turn_id, "send task");
    assert!(matches!(
        resend_res,
        Err(RuntimeError::AutomaticResendForbidden {
            delivery: DeliveryState::Indeterminate,
            ..
        })
    ));
}

#[test]
fn test_completed_and_cancelled_turns_forbid_duplicate_resend_but_allow_new_turn() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);

    let binding = sample_binding(1);
    let thread = sample_thread(1);
    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    let turn_1 = sample_turn(1);
    let op_1 = sample_operation(1);

    // 1. Complete Turn 1
    runtime
        .prompt(&thread, op_1.clone(), turn_1.clone(), "first")
        .unwrap();
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });
    let _ = runtime.poll_stream(&thread).unwrap();

    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );

    // Automatic resend of completed turn is forbidden
    let resend_1 = runtime.prompt(&thread, sample_operation(2), turn_1.clone(), "first");
    assert!(matches!(
        resend_1,
        Err(RuntimeError::AutomaticResendForbidden {
            delivery: DeliveryState::Confirmed,
            ..
        })
    ));

    // 2. New turn on Ready supervisor succeeds
    let turn_2 = sample_turn(2);
    let op_2 = sample_operation(3);
    let adm = runtime
        .prompt(&thread, op_2.clone(), turn_2.clone(), "second")
        .unwrap();
    assert_eq!(adm, TurnAdmission::Admitted);

    // 3. Cancel Turn 2
    let _ = runtime.steer_cancel(&thread, Some(&op_2)).unwrap();
    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Cancelled);
    let _ = runtime.poll_stream(&thread).unwrap();

    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );

    // Automatic resend of cancelled turn is forbidden
    let resend_2 = runtime.prompt(&thread, sample_operation(4), turn_2.clone(), "second");
    assert!(matches!(
        resend_2,
        Err(RuntimeError::AutomaticResendForbidden {
            delivery: DeliveryState::Confirmed,
            ..
        })
    ));

    // 4. Turn 3 succeeds after cancelled turn
    let turn_3 = sample_turn(3);
    let op_3 = sample_operation(5);
    let adm3 = runtime.prompt(&thread, op_3, turn_3, "third").unwrap();
    assert_eq!(adm3, TurnAdmission::Admitted);
}

#[test]
fn test_untracked_events_produce_unknown_without_panic() {
    let harness = FakeHarness::new();
    let checkpoint = FakeCheckpoint::default();
    let mut runtime = AgentRuntimeSupervisor::new(harness, checkpoint);
    let thread = sample_thread(1);
    let binding = sample_binding(1);

    let session_id = runtime.create_session(&binding, &thread, None).unwrap();

    // While supervisor is in Ready state (no active turn), feed untracked events from harness
    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::MessageDelta {
            text: "stray delta".to_string(),
        },
    );
    let ev1 = runtime.poll_stream(&thread).unwrap();
    assert!(matches!(
        ev1,
        Some(RuntimeEvent::Unknown {
            name,
            ..
        }) if name == "untracked.message_delta"
    ));

    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Completed { payload: None });
    let ev2 = runtime.poll_stream(&thread).unwrap();
    assert!(matches!(
        ev2,
        Some(RuntimeEvent::Unknown {
            name,
            ..
        }) if name == "untracked.completed"
    ));

    runtime
        .harness_mut()
        .queue_event(&session_id, HarnessEvent::Cancelled);
    let ev3 = runtime.poll_stream(&thread).unwrap();
    assert!(matches!(
        ev3,
        Some(RuntimeEvent::Unknown {
            name,
            ..
        }) if name == "untracked.cancelled"
    ));

    runtime.harness_mut().queue_event(
        &session_id,
        HarnessEvent::Failed {
            error: "stray failure".to_string(),
            delivery: DeliveryState::Indeterminate,
        },
    );
    let ev4 = runtime.poll_stream(&thread).unwrap();
    assert!(matches!(
        ev4,
        Some(RuntimeEvent::Unknown {
            name,
            ..
        }) if name == "untracked.failed"
    ));

    // Verify supervisor remains intact and in Ready state
    assert_eq!(
        runtime.supervisor(&thread).unwrap().state(),
        &SupervisorState::Ready
    );
}
