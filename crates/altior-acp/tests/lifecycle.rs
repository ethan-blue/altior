//! P0.3 evidence: process crash, idle timeout, cancellation, and cleanup
//! (ADR 0007).
//!
//! The lifecycle machine is pure, so these scenarios replay fake agent
//! streams deterministically: the alpha fixture's stream truncated mid-
//! turn stands in for a crash, and the machine's decisions are asserted
//! together with the delivery classification they imply.

use altior_acp::mapping;
use altior_acp::messages::StopReason;
use altior_acp::{
    AgentEvent, AgentLifecycle, AgentPhase, DeliveryCause, HostAction, PromptDelivery, RpcId,
};
use altior_domain::DeliveryState;

const ALPHA_TRACE: &str = include_str!("../fixtures/acp-agent-alpha-trace-v1.json");

fn alpha_lines() -> Vec<String> {
    serde_json::from_str(ALPHA_TRACE).expect("alpha trace fixture parses")
}

#[test]
fn a_process_crash_mid_turn_is_indeterminate_and_collapses_to_reap() {
    let lines = alpha_lines();
    // The stream dies right after the permission request: everything
    // after line 5 (the prompt response included) never arrives.
    let received = mapping::normalize_trace(lines.iter().take(5)).unwrap();

    let mut delivery = PromptDelivery::not_sent();
    delivery.mark_written().unwrap();
    let mut lifecycle = AgentLifecycle::spawned();
    lifecycle.on_prompt_written();
    // The permission request arrived before the crash.
    let actions = lifecycle
        .on_permission_requested(RpcId::Text("alpha-perm-1".to_owned()))
        .unwrap();
    assert_eq!(actions, vec![HostAction::Continue]);
    assert!(
        received
            .iter()
            .any(|event| matches!(&event.event, AgentEvent::PermissionRequested { .. }))
    );

    // The stream ends: no prompt response, so delivery can never be
    // Absent (bytes may have been consumed) — only Indeterminate.
    assert_eq!(
        delivery
            .on_connection_lost(DeliveryCause::ProcessExited)
            .unwrap(),
        DeliveryState::Indeterminate
    );
    assert!(!delivery.may_resend());
    assert_eq!(
        lifecycle.on_process_lost(),
        vec![HostAction::KillAndReap],
        "a crash skips the cancel handshake and goes straight to reap"
    );
    assert_eq!(lifecycle.phase(), &AgentPhase::Dead);
}

#[test]
fn an_idle_timeout_kills_without_waiting_for_a_response() {
    let mut delivery = PromptDelivery::not_sent();
    delivery.mark_written().unwrap();
    let mut lifecycle = AgentLifecycle::spawned();
    lifecycle.on_prompt_written();

    assert_eq!(
        lifecycle.on_idle_elapsed(),
        vec![HostAction::KillAndReap],
        "a silent agent is untrusted; the idle budget ends it"
    );
    assert_eq!(lifecycle.phase(), &AgentPhase::Killing);
    assert_eq!(
        delivery
            .on_connection_lost(DeliveryCause::IdleTimeout)
            .unwrap(),
        DeliveryState::Indeterminate
    );
    lifecycle.on_reaped();
    assert_eq!(lifecycle.phase(), &AgentPhase::Dead);
}

#[test]
fn cancellation_answers_permissions_waits_for_the_response_then_settles() {
    let lines = alpha_lines();
    let received = mapping::normalize_trace(&lines).unwrap();

    let mut lifecycle = AgentLifecycle::spawned();
    lifecycle.on_prompt_written();
    lifecycle
        .on_permission_requested(RpcId::Text("alpha-perm-1".to_owned()))
        .unwrap();

    // User cancels: notification first, then every pending permission is
    // answered cancelled, and the agent is trusted to answer the prompt.
    assert_eq!(
        lifecycle.on_cancel_requested().unwrap(),
        vec![
            HostAction::SendCancelNotification,
            HostAction::AnswerPermissionCancelled {
                id: RpcId::Text("alpha-perm-1".to_owned())
            },
        ]
    );
    assert_eq!(
        lifecycle.phase(),
        &AgentPhase::Cancelling {
            pending_permissions: Vec::new()
        }
    );

    // A permission racing the cancel is answered cancelled on sight.
    assert_eq!(
        lifecycle
            .on_permission_requested(RpcId::Number(99))
            .unwrap(),
        vec![HostAction::AnswerPermissionCancelled {
            id: RpcId::Number(99)
        }]
    );

    // The cancelled turn's response arrives (the fixture's end_turn stands
    // in for the cancelled stop reason); any stop reason confirms receipt.
    let stop = received
        .iter()
        .find_map(|event| match &event.event {
            AgentEvent::TurnCompleted { stop_reason } => Some(*stop_reason),
            _ => None,
        })
        .expect("alpha's trace completes its turn");
    assert_eq!(stop, StopReason::EndTurn);
    assert_eq!(
        lifecycle.on_prompt_settled().unwrap(),
        vec![HostAction::TurnSettled]
    );
    assert_eq!(lifecycle.phase(), &AgentPhase::Ready);
    assert_eq!(lifecycle.permissions_unanswered(), 0);
}

#[test]
fn cleanup_after_a_settled_turn_kills_and_reaps_a_live_child() {
    let mut lifecycle = AgentLifecycle::spawned();
    lifecycle.on_prompt_written();
    lifecycle.on_prompt_settled().unwrap();
    // The agent is reusable, but the smoke host is done: explicit
    // shutdown kills and reaps without the cancel handshake.
    assert_eq!(lifecycle.on_process_lost(), vec![HostAction::KillAndReap]);
    lifecycle.on_reaped();
    assert_eq!(lifecycle.phase(), &AgentPhase::Dead);
    assert!(matches!(
        lifecycle.on_permission_requested(RpcId::Number(1)),
        Err(altior_acp::AcpError::OutOfOrder { .. })
    ));
}
