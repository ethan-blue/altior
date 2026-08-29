//! P0.3 evidence: normalized trace fixtures for two synthetic agents
//! (ADR 0007).
//!
//! The raw fixtures are synthetic ACP v1 stream lines for two agent
//! personalities — `agent-alpha` (rich: sessions, tool calls, a
//! permission request, a clean `end_turn`) and `agent-beta` (minimal:
//! echoes, thoughts, an unknown `usage_update`, a `refusal`). The
//! normalized fixtures pin what the one shared normalizer produces for
//! both; regenerate them with `cargo test -p altior-acp --test traces --
//! --ignored regenerate` after changing the raw traces or the mapping.

use altior_acp::mapping::{self, AgentEvent};
use altior_acp::negotiation;
use altior_acp::{AcpError, RpcMessage};

const INITIALIZE_ALPHA: &str = include_str!("../fixtures/acp-initialize-alpha-v1.json");
const INITIALIZE_BETA: &str = include_str!("../fixtures/acp-initialize-beta-v1.json");
const ALPHA_TRACE: &str = include_str!("../fixtures/acp-agent-alpha-trace-v1.json");
const BETA_TRACE: &str = include_str!("../fixtures/acp-agent-beta-trace-v1.json");
const ALPHA_NORMALIZED: &str = include_str!("../fixtures/acp-agent-alpha-normalized-v1.json");
const BETA_NORMALIZED: &str = include_str!("../fixtures/acp-agent-beta-normalized-v1.json");

/// The raw fixture lines.
fn lines(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).expect("trace fixtures are JSON arrays of lines")
}

#[test]
fn agent_alpha_normalizes_to_the_pinned_fixture() {
    let actual = mapping::normalize_trace_to_json(lines(ALPHA_TRACE)).unwrap();
    let pinned: serde_json::Value =
        serde_json::from_str(ALPHA_NORMALIZED).expect("normalized fixture parses");
    assert_eq!(actual, pinned);
}

#[test]
fn agent_beta_normalizes_to_the_pinned_fixture() {
    let actual = mapping::normalize_trace_to_json(lines(BETA_TRACE)).unwrap();
    let pinned: serde_json::Value =
        serde_json::from_str(BETA_NORMALIZED).expect("normalized fixture parses");
    assert_eq!(actual, pinned);
}

#[test]
fn two_agents_share_one_normalizer_and_split_on_outcome() {
    let alpha = mapping::normalize_trace(lines(ALPHA_TRACE)).unwrap();
    let beta = mapping::normalize_trace(lines(BETA_TRACE)).unwrap();

    // Alpha: deltas stream, a tool runs, a permission is requested, and
    // the turn completes cleanly.
    let deltas: Vec<&str> = alpha
        .iter()
        .filter_map(|event| match &event.event {
            AgentEvent::Delta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, ["Hel", "lo", " done"]);
    assert!(alpha.iter().any(|event| matches!(
        &event.event,
        AgentEvent::ToolObserved { tool_call_id, status }
            if tool_call_id == "tc_alpha_1" && status.as_deref() == Some("pending")
    )));
    assert!(alpha.iter().any(|event| matches!(
        &event.event,
        AgentEvent::PermissionRequested { request_id, tool_call_id }
            if request_id == "alpha-perm-1" && tool_call_id.as_deref() == Some("tc_alpha_1")
    )));
    assert_eq!(
        alpha.last().unwrap().event,
        AgentEvent::TurnCompleted {
            stop_reason: altior_acp::messages::StopReason::EndTurn
        }
    );

    // Beta: the user echo and thought preserve, the unknown update keeps
    // its kind, and the refusal fails the turn instead of completing it.
    assert!(beta.iter().any(|event| matches!(
        &event.event,
        AgentEvent::Preserved { provider_kind, .. } if provider_kind == "acp.update.usage_update"
    )));
    assert!(beta.iter().any(|event| matches!(
        &event.event,
        AgentEvent::Preserved { provider_kind, .. } if provider_kind == "acp.agent_thought"
    )));
    assert!(beta.iter().any(|event| matches!(
        &event.event,
        AgentEvent::Preserved { provider_kind, .. } if provider_kind == "acp.user_message"
    )));
    assert_eq!(
        beta.last().unwrap().event,
        AgentEvent::Preserved {
            provider_kind: "acp.notification.unmapped".to_owned(),
            raw: serde_json::json!({"method": "_custom/extension"}),
        }
    );
    // The refusal itself sits mid-trace as the prompt response.
    assert!(beta.iter().any(|event| matches!(
        &event.event,
        AgentEvent::TurnFailed { diagnostic } if diagnostic.contains("refusal")
    )));
}

#[test]
fn the_two_agents_negotiate_different_capability_views() {
    let alpha = negotiation::negotiate(&initialize_result(INITIALIZE_ALPHA));
    let beta = negotiation::negotiate(&initialize_result(INITIALIZE_BETA));
    assert!(alpha.may_resume(), "alpha advertises loadSession");
    assert!(!beta.may_resume(), "beta advertises nothing");
    assert_eq!(alpha.agent_protocol_version, beta.agent_protocol_version);
}

fn initialize_result(raw: &str) -> altior_acp::messages::InitializeResult {
    let message = RpcMessage::decode(raw.trim()).expect("initialize fixture decodes");
    let RpcMessage::Response { result, .. } = message else {
        panic!("initialize fixtures are responses");
    };
    serde_json::from_value(result).expect("initialize result matches the v1 shape")
}

/// Rewrites the normalized fixtures from the raw traces. Run explicitly
/// with `cargo test -p altior-acp --test traces -- --ignored
/// regenerate` after changing a raw trace or the mapping table; the
/// comparison tests above then pin the result.
#[test]
#[ignore = "fixture regeneration tool; opt in explicitly"]
fn regenerate() {
    for (raw_path, normalized_path) in [
        (
            "fixtures/acp-agent-alpha-trace-v1.json",
            "fixtures/acp-agent-alpha-normalized-v1.json",
        ),
        (
            "fixtures/acp-agent-beta-trace-v1.json",
            "fixtures/acp-agent-beta-normalized-v1.json",
        ),
    ] {
        let raw: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(raw_path).unwrap()).unwrap();
        let normalized = mapping::normalize_trace_to_json(&raw)
            .map_err(|error: AcpError| error.to_string())
            .unwrap();
        let pretty = serde_json::to_string_pretty(&normalized).unwrap();
        std::fs::write(normalized_path, pretty + "\n").unwrap();
    }
}
