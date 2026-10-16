//! B03: deterministic ACP loopback smoke — the default-gate twin of the
//! opt-in external smoke (`smoke.rs`).
//!
//! Runs the exact same journey — initialize handshake, capability
//! negotiation, session/new, prompt with delivery tracking, agent-request
//! handling through the lifecycle machine, trace normalization, kill and
//! reap — against a REAL child process: the in-repo deterministic stub
//! binary (`mock_acp_agent`) speaking ACP v1 JSON-RPC over stdin/stdout.
//!
//! This is not a mock impersonating acceptance: the client-side stack
//! under test is the production wire path (real OS subprocess, real
//! stdio framing, real JSON-RPC decode). What it deliberately does NOT
//! claim is real third-party agent behavior or real model output; that
//! remains the opt-in `ALTIOR_ACP_SMOKE_AGENTS` gate.
//!
//! Deterministic: the stub scripts its responses, so no sleeps, no
//! network, no machine-load sensitivity on the pass path.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp copy of the stub, run with the given scenario.
fn stub_spec(tag: &str, scenario: &str) -> (common::ChildSpec, TempStubGuard) {
    let guard = TempStubGuard::new();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let scratch = std::env::temp_dir().join(format!(
        "altior-acp-loopback-{tag}-{}-{counter}",
        std::process::id()
    ));
    let spec = common::ChildSpec {
        program: guard.path_str(),
        args: vec!["--scenario".to_owned(), scenario.to_owned()],
        scratch,
    };
    (spec, guard)
}

/// Copies the stub binary to a unique temp path and removes it on drop.
struct TempStubGuard {
    path: PathBuf,
}

impl TempStubGuard {
    fn new() -> Self {
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path =
            std::env::temp_dir().join(format!("altior_loopback_agent_a_{pid}_{counter}.exe"));
        let src = env!("CARGO_BIN_EXE_mock_acp_agent");
        std::fs::copy(src, &path).expect("copy stub binary to temp path");
        Self { path }
    }

    fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempStubGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn loopback_agent_completes_a_confirmed_smoke_turn() {
    let (spec, _guard) = stub_spec("plain", "agent_a_full");
    let summary =
        common::run_smoke_turn(&spec).expect("the deterministic stub must complete a smoke turn");

    // agentCapabilities.loadSession = true for this scenario.
    assert!(summary.contains("resume=true"), "summary: {summary}");
    assert!(
        !summary.contains("deltas=0"),
        "the echoed prompt must produce at least one delta; summary: {summary}"
    );
    assert!(summary.contains("delivery=Confirmed"), "summary: {summary}");
}

#[test]
fn loopback_agent_cancel_journey_settles_through_the_lifecycle_machine() {
    // The minimal stub emits one delta, then blocks until it observes a
    // `session/cancel` notification, then answers with stopReason
    // `cancelled` — the same observable shape a real agent produces. The
    // harness must route the user cancel through
    // AgentLifecycle::on_cancel_requested (cancel notification write),
    // and the prompt response must settle delivery as Confirmed.
    let (spec, _guard) = stub_spec("cancel", "agent_b_minimal");
    let summary = common::run_smoke_turn_with_prompt(
        &spec,
        "please [TRIGGER_CANCEL] this long task",
        common::TurnPlan::CancelAfterFirstStreamActivity,
    )
    .expect("the deterministic stub must settle the cancelled turn");

    // loadSession = false for the minimal scenario.
    assert!(summary.contains("resume=false"), "summary: {summary}");
    assert!(summary.contains("delivery=Confirmed"), "summary: {summary}");
    assert!(
        !summary.contains("deltas=0"),
        "the pre-cancel delta must be observed; summary: {summary}"
    );
}
