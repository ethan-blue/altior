//! Opt-in smoke test against real ACP agents (ADR 0007).
//!
//! Default gates never touch the network or external binaries. To run
//! against two real agents, set `ALTIOR_ACP_SMOKE_AGENTS` to two
//! `;;`-separated command lines, for example:
//!
//! ```text
//! ALTIOR_ACP_SMOKE_AGENTS="npx -y @zed-industries/claude-code-acp;;npx -y @google/gemini-cli --experimental-acp"
//! cargo test -p altior-acp --test smoke -- --nocapture
//! ```
//!
//! Each entry must be a real agent CLI that speaks ACP v1 over
//! stdin/stdout and holds its own credentials (API keys) in the
//! environment; this harness grants no filesystem, no terminal. The run
//! performs the initialize handshake, creates a session in a scratch
//! directory, prompts `Reply with the single word: pong`, normalizes the
//! observed stream with the same fixture-pinned normalizer, asserts the
//! prompt delivery was confirmed, then kills and reaps the child.
//!
//! The deterministic in-repo equivalent of this exact journey (real
//! subprocess, no external agent) lives in `loopback_smoke.rs` and runs
//! in every default gate.

mod common;

#[test]
fn two_real_agents_complete_a_smoke_turn() {
    let Ok(spec) = std::env::var("ALTIOR_ACP_SMOKE_AGENTS") else {
        eprintln!(
            "skipping: ALTIOR_ACP_SMOKE_AGENTS is not set; set it to \
             '<agent-a>;;<agent-b>' (two ACP v1 command lines) to run"
        );
        return;
    };
    let agents: Vec<&str> = spec
        .split(";;")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        agents.len() >= 2,
        "the plan needs two real agents; set ALTIOR_ACP_SMOKE_AGENTS='a;;b'"
    );
    for (index, command_line) in agents.iter().enumerate() {
        smoke_one_agent(index, command_line);
    }
}

fn smoke_one_agent(index: usize, command_line: &str) {
    let mut parts = command_line.split_whitespace();
    let program = parts.next().unwrap_or_default();
    assert!(!program.is_empty(), "agent {index} command line is empty");

    let spec = common::ChildSpec {
        program: program.to_owned(),
        args: parts.map(str::to_owned).collect(),
        scratch: scratch_dir(index),
    };

    let summary = common::run_smoke_turn(&spec)
        .unwrap_or_else(|error| panic!("agent {index} ({program}): {error}"));
    println!("agent {index} ({program}): {summary}");
}

/// The scratch working directory for one agent's session.
fn scratch_dir(index: usize) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("altior-acp-smoke-{}-{index}", std::process::id()))
}
