//! Shared ACP smoke-turn harness.
//!
//! Drives one full ACP v1 journey against a REAL child process over real
//! stdin/stdout: initialize handshake → capability negotiation →
//! session/new → prompt with delivery tracking → agent-request handling
//! through the lifecycle machine → trace normalization. The same journey
//! runs against opt-in external agents (`smoke.rs`) and against the
//! in-repo deterministic stub (`loopback_smoke.rs`); only the spawned
//! program differs.
//!
//! No network access, no real sleeps on the pass path: the watchdog
//! thread below is a hang guard that kills a stuck child at the
//! deadline; a healthy run never waits on it.

use std::io::{BufRead, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use altior_acp::mapping::AgentEvent;
use altior_acp::messages::{
    CancelParams, ContentBlock, InitializeResult, NewSessionParams, PromptParams,
};
use altior_acp::{AgentLifecycle, HostAction, PromptDelivery, RpcError, RpcId, RpcMessage};

/// Kill a hung child at this budget; a hung agent must fail the run
/// with real evidence rather than hang the gate.
pub const WATCHDOG_DEADLINE: Duration = Duration::from_secs(60);

/// The prompt every smoke agent must answer.
pub const SMOKE_PROMPT: &str = "Reply with the single word: pong";

/// How to spawn the agent under test.
pub struct ChildSpec {
    pub program: String,
    pub args: Vec<String>,
    pub scratch: PathBuf,
}

/// How the harness should steer the turn beyond sending the prompt.
// Compiled into every test binary that includes this module; not every
// binary exercises every plan.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum TurnPlan {
    /// Send the prompt and await its response.
    Plain,
    /// After the first stream activity, request cancellation through the
    /// lifecycle machine (cancel notification + pending-permission
    /// answers), then await the prompt response.
    CancelAfterFirstStreamActivity,
}

/// Spawns the agent and runs the smoke turn with the standard smoke prompt.
pub fn run_smoke_turn(spec: &ChildSpec) -> Result<String, String> {
    run_smoke_turn_with_prompt(spec, SMOKE_PROMPT, TurnPlan::Plain)
}

/// Spawns the agent and runs the smoke turn with a caller-supplied prompt
/// (used to exercise agent-initiated flows such as cancellation).
pub fn run_smoke_turn_with_prompt(
    spec: &ChildSpec,
    prompt: &str,
    plan: TurnPlan,
) -> Result<String, String> {
    std::fs::create_dir_all(&spec.scratch)
        .map_err(|e| format!("scratch cwd {} not creatable: {e}", spec.scratch.display()))?;

    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(&spec.scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{} failed to spawn: {error}", spec.program))?;
    let mut stdin = child.stdin.take().expect("agent stdin is piped");
    let stdout = child.stdout.take().expect("agent stdout is piped");
    let mut reader = BufReader::new(stdout);

    // Watchdog: kill a hung child at the deadline; the read loops then
    // see EOF and fail with real evidence instead of hanging the run.
    // The guard exits immediately on the pass path (channel signal), so
    // a healthy run never waits on the deadline.
    let child = Arc::new(Mutex::new(child));
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let child = Arc::clone(&child);
        std::thread::spawn(move || {
            if done_rx.recv_timeout(WATCHDOG_DEADLINE).is_err() {
                let _ = child.lock().expect("child lock").kill();
            }
        })
    };

    let result = drive_smoke_turn(&mut stdin, &mut reader, prompt, plan);
    let _ = done_tx.send(());
    kill_and_reap(&child);
    watchdog.join().expect("watchdog joins");
    let _ = std::fs::remove_dir_all(&spec.scratch);

    result
}

/// Runs the smoke turn; returns a one-line summary on success.
fn drive_smoke_turn(
    stdin: &mut std::process::ChildStdin,
    reader: &mut BufReader<std::process::ChildStdout>,
    prompt: &str,
    plan: TurnPlan,
) -> Result<String, String> {
    // 1. initialize: capabilities, not version strings (ADR 0007).
    let initialize = send_request(
        stdin,
        reader,
        "initialize",
        1,
        &serde_json::to_value(altior_acp::initialize_request(env!("CARGO_PKG_VERSION")))
            .map_err(|e| e.to_string())?,
    )?;
    let negotiated = altior_acp::negotiation::negotiate(
        &serde_json::from_value::<InitializeResult>(initialize)
            .map_err(|e| format!("initialize result is not v1: {e}"))?,
    );

    // 2. session/new in the scratch directory.
    let new_session = send_request(
        stdin,
        reader,
        "session/new",
        2,
        &serde_json::to_value(NewSessionParams {
            cwd: std::env::current_dir()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
            mcp_servers: Vec::new(),
        })
        .map_err(|e| e.to_string())?,
    )?;
    let session_id = new_session
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .ok_or("session/new returned no sessionId")?
        .to_owned();

    // 3. prompt; collect the whole observed stream until its response.
    let mut delivery = PromptDelivery::not_sent();
    let mut lifecycle = AgentLifecycle::spawned();
    let mut trace: Vec<String> = Vec::new();

    send_prompt(stdin, &session_id, prompt)?;
    delivery.mark_written().map_err(|e| e.to_string())?;
    lifecycle.on_prompt_written();
    await_prompt_response(
        stdin,
        reader,
        &session_id,
        &mut delivery,
        &mut lifecycle,
        &mut trace,
        plan,
    )?;
    assert_eq!(
        delivery.state(),
        altior_acp::DeliveryState::Confirmed,
        "a smoke turn must be confirmed, not {:?}",
        delivery.state()
    );

    // 4. the observed stream normalizes with the fixture-pinned mapper.
    let normalized = altior_acp::mapping::normalize_trace(&trace).map_err(|e| e.to_string())?;
    assert!(
        !normalized.is_empty(),
        "the observed stream produced events"
    );
    let deltas = normalized
        .iter()
        .filter(|event| matches!(event.event, AgentEvent::Delta { .. }))
        .count();
    Ok(format!(
        "resume={} deltas={deltas} events={} delivery={:?}",
        negotiated.may_resume(),
        normalized.len(),
        delivery.state()
    ))
}

/// Writes the prompt (request id 3). Delivery classification is the
/// caller's: a failed write leaves the prompt `Absent`, not sent.
fn send_prompt(
    stdin: &mut std::process::ChildStdin,
    session_id: &str,
    prompt: &str,
) -> Result<(), String> {
    let prompt = serde_json::to_value(PromptParams {
        session_id: session_id.to_owned(),
        prompt: vec![ContentBlock::Text {
            text: prompt.to_owned(),
        }],
    })
    .map_err(|e| e.to_string())?;
    write_line(stdin, &request_line(3, "session/prompt", &prompt))
        .map_err(|e| format!("prompt write failed before any byte: {e}"))
}

/// Reads the observed stream until the prompt response settles the turn.
/// Agent requests are answered: permissions through the lifecycle machine,
/// everything else with a typed method-not-found refusal.
#[allow(clippy::too_many_arguments)]
fn await_prompt_response(
    stdin: &mut std::process::ChildStdin,
    reader: &mut BufReader<std::process::ChildStdout>,
    session_id: &str,
    delivery: &mut PromptDelivery,
    lifecycle: &mut AgentLifecycle,
    trace: &mut Vec<String>,
    plan: TurnPlan,
) -> Result<(), String> {
    let mut settled = false;
    let mut cancel_sent = false;
    let mut stream_seen = false;
    for line in reader.lines() {
        let line = line.map_err(|e| format!("stream read failed: {e}"))?;
        trace.push(line.clone());
        let message = RpcMessage::decode(&line).map_err(|e| e.to_string())?;
        match message {
            RpcMessage::Response {
                id: RpcId::Number(3),
                ..
            } => {
                delivery.on_prompt_response().map_err(|e| e.to_string())?;
                lifecycle.on_prompt_settled().map_err(|e| e.to_string())?;
                settled = true;
                break;
            }
            RpcMessage::ErrorResponse {
                id: RpcId::Number(3),
                ..
            } => {
                delivery.on_error_response().map_err(|e| e.to_string())?;
                lifecycle.on_prompt_settled().map_err(|e| e.to_string())?;
                settled = true;
                break;
            }
            RpcMessage::Request { id, method, .. } if method == "session/request_permission" => {
                for action in lifecycle
                    .on_permission_requested(id.clone())
                    .map_err(|e| e.to_string())?
                {
                    execute_host_action(stdin, &action, session_id, &id, &method)?;
                }
            }
            RpcMessage::Request { id, method, .. } => {
                // The smoke client advertises no filesystem and no
                // terminal; any other agent request gets a typed
                // method-not-found error so the agent can proceed.
                let refusal = RpcMessage::ErrorResponse {
                    id,
                    error: RpcError {
                        code: -32601,
                        message: format!("altior smoke grants no {method}"),
                    },
                };
                write_line(stdin, &refusal.encode().map_err(|e| e.to_string())?)
                    .map_err(|e| format!("refusing {method} failed: {e}"))?;
            }
            RpcMessage::Notification { .. } => {
                stream_seen = true;
            }
            // Responses for other requests do not concern this turn.
            RpcMessage::Response { .. } | RpcMessage::ErrorResponse { .. } => {}
        }
        if matches!(plan, TurnPlan::CancelAfterFirstStreamActivity) && stream_seen && !cancel_sent {
            cancel_sent = true;
            for action in lifecycle.on_cancel_requested().map_err(|e| e.to_string())? {
                execute_host_action(
                    stdin,
                    &action,
                    session_id,
                    &RpcId::Number(0),
                    "session/cancel",
                )?;
            }
        }
    }
    if !settled {
        let _ = delivery.on_connection_lost(altior_acp::DeliveryCause::ProcessExited);
        return Err(format!(
            "stream ended before the prompt response; delivery is {:?}",
            delivery.state()
        ));
    }
    Ok(())
}

/// Writes one newline-terminated line and flushes it.
fn write_line(stdin: &mut std::process::ChildStdin, line: &str) -> Result<(), String> {
    stdin
        .write_all(
            altior_acp::encode_line(line)
                .map_err(|e| e.to_string())?
                .as_slice(),
        )
        .and_then(|()| stdin.flush())
        .map_err(|e| e.to_string())
}

/// Sends a request and blocks until its response or error response.
fn send_request(
    stdin: &mut std::process::ChildStdin,
    reader: &mut BufReader<std::process::ChildStdout>,
    method: &str,
    id: u64,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    write_line(stdin, &request_line(id, method, params))
        .map_err(|e| format!("{method} write failed: {e}"))?;
    loop {
        let line = reader
            .lines()
            .next()
            .ok_or_else(|| format!("stream ended before the {method} response"))?
            .map_err(|e| format!("{method} read failed: {e}"))?;
        match RpcMessage::decode(&line).map_err(|e| e.to_string())? {
            RpcMessage::Response {
                id: RpcId::Number(seen),
                result,
            } if seen == id => {
                return Ok(result);
            }
            RpcMessage::ErrorResponse {
                id: RpcId::Number(seen),
                error,
            } if seen == id => {
                return Err(format!("{method} failed: {} {}", error.code, error.message));
            }
            _ => {}
        }
    }
}

fn request_line(id: u64, method: &str, params: &serde_json::Value) -> String {
    RpcMessage::Request {
        id: RpcId::Number(id),
        method: method.to_owned(),
        params: params.clone(),
    }
    .encode()
    .expect("requests always encode")
}

/// Executes one lifecycle decision against the real child. The smoke
/// prompt is benign: permissions are answered cancelled, unknown agent
/// requests with a method-not-found error.
fn execute_host_action(
    stdin: &mut std::process::ChildStdin,
    action: &HostAction,
    session_id: &str,
    request_id: &RpcId,
    method: &str,
) -> Result<(), String> {
    let reply = match action {
        HostAction::Continue | HostAction::TurnSettled => return Ok(()),
        HostAction::SendCancelNotification => RpcMessage::Notification {
            method: "session/cancel".to_owned(),
            params: serde_json::to_value(CancelParams {
                session_id: session_id.to_owned(),
            })
            .map_err(|e| e.to_string())?,
        },
        HostAction::AnswerPermissionCancelled { id } => RpcMessage::Response {
            id: id.clone(),
            result: serde_json::json!({ "outcome": "cancelled" }),
        },
        HostAction::KillAndReap => {
            return Err(format!(
                "lifecycle ordered a kill while answering {method} for {request_id:?}"
            ));
        }
    };
    write_line(stdin, &reply.encode().map_err(|e| e.to_string())?)
        .map_err(|e| format!("answering {method} failed: {e}"))
}

fn kill_and_reap(child: &Arc<Mutex<Child>>) {
    let mut child = child.lock().expect("child lock");
    let _ = child.kill();
    let _ = child.wait();
}
