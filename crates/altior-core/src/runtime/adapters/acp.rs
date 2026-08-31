//! ACP harness adapter connecting core runtime to ACP agent subprocesses (P1.2, ADR 0007).
//!
//! Spawns, monitors, streams prompts, routes permissions, and cancels turns
//! on ACP agent subprocesses via [`altior_acp::AcpRuntime`] and [`altior_acp::AcpChild`].
//!
//! Since ACP prompt streaming is blocking on line I/O, this adapter manages
//! a per-session worker thread communicating via bounded channels.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use altior_acp::messages::{ContentBlock, StopReason};
use altior_acp::{
    AcpChild, AcpError, AcpRuntime, AgentEvent, EnvVarValue, LaunchConfig, NegotiatedCapabilities,
    NoSecretsResolver, NormalizedEvent, RpcId, SecretResolver,
};
use altior_domain::{
    AcpHarnessBinding, BoundedPath, DeliveryState, EventId, PermissionDecision,
    PermissionDescription, PermissionKind, ProjectRef, ThreadId,
};
use altior_protocol::{CapabilitySet, CapabilitySupport};

use crate::runtime::ports::HarnessRuntimePort;
use crate::runtime::state::{
    BindingProbeOutcome, HarnessError, HarnessEvent, HarnessPromptRequest, HarnessSessionId,
    HarnessSessionInfo,
};

/// Bounded queue capacity for session command and event channels.
const CHANNEL_BOUND: usize = 1024;

static FALLBACK_EVENT_ID: std::sync::LazyLock<EventId> = std::sync::LazyLock::new(|| {
    EventId::from_str("evt_perm000000000000").expect("static fallback event id is valid")
});

static FALLBACK_PERMISSION_DESC: std::sync::LazyLock<PermissionDescription> =
    std::sync::LazyLock::new(|| {
        PermissionDescription::try_from("permission required")
            .expect("static fallback permission description is valid")
    });

/// Internal command dispatched to a per-session background worker.
enum WorkerCommand {
    Prompt(HarnessPromptRequest),
    Close,
}

struct SessionWorkerHandle {
    command_tx: std::sync::mpsc::SyncSender<WorkerCommand>,
    event_rx: std::sync::mpsc::Receiver<HarnessEvent>,
    ack_tx: std::sync::mpsc::SyncSender<()>,
    event_awaiting_ack: bool,
    cancel_signal: Arc<AtomicBool>,
    permission_txs: Arc<Mutex<BTreeMap<EventId, std::sync::mpsc::SyncSender<PermissionDecision>>>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl SessionWorkerHandle {
    fn ack_if_awaiting(&mut self) {
        if self.event_awaiting_ack {
            self.event_awaiting_ack = false;
            let _ = self.ack_tx.try_send(());
        }
    }
}

const fn requires_ack(event: &HarnessEvent) -> bool {
    matches!(
        event,
        HarnessEvent::Started { .. }
            | HarnessEvent::MessageDelta { .. }
            | HarnessEvent::RawUnknown { .. }
    )
}

/// Harness adapter managing ACP subprocess lifecycles and prompt execution.
pub struct AcpHarnessAdapter {
    sessions: BTreeMap<HarnessSessionId, SessionWorkerHandle>,
    client_version: String,
    secret_resolver: Arc<dyn SecretResolver + Send + Sync>,
    launch_env: BTreeMap<String, EnvVarValue>,
}

impl Default for AcpHarnessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AcpHarnessAdapter {
    /// Creates a new ACP harness adapter with default no-secrets resolver and empty launch env.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            client_version: format!("altior-core/{}", env!("CARGO_PKG_VERSION")),
            secret_resolver: Arc::new(NoSecretsResolver),
            launch_env: BTreeMap::new(),
        }
    }

    /// Creates a new adapter with a custom client version identifier.
    #[must_use]
    pub fn with_client_version(version: impl Into<String>) -> Self {
        Self {
            sessions: BTreeMap::new(),
            client_version: version.into(),
            secret_resolver: Arc::new(NoSecretsResolver),
            launch_env: BTreeMap::new(),
        }
    }

    /// Configures the secret resolver trait object seam.
    #[must_use]
    pub fn with_secret_resolver(mut self, resolver: Arc<dyn SecretResolver + Send + Sync>) -> Self {
        self.secret_resolver = resolver;
        self
    }

    /// Configures the launch environment variables template.
    #[must_use]
    pub fn with_launch_env(mut self, env: impl IntoIterator<Item = (String, EnvVarValue)>) -> Self {
        self.launch_env = env.into_iter().collect();
        self
    }

    /// Helper to resolve a launch config and spawn a child.
    fn spawn_child(
        &self,
        binding: &AcpHarnessBinding,
        project: Option<&ProjectRef>,
    ) -> Result<AcpChild, HarnessError> {
        let mut launch_config = parse_launch_config(&binding.command, project)?;
        for (k, v) in &self.launch_env {
            match v {
                EnvVarValue::Literal(lit) => {
                    launch_config = launch_config
                        .with_literal_env(k, lit)
                        .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
                }
                EnvVarValue::SecretRef(sref) => {
                    launch_config = launch_config
                        .with_secret_env(k, sref.clone())
                        .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
                }
            }
        }
        let resolved = launch_config
            .resolve(&*self.secret_resolver)
            .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
        AcpChild::spawn(&resolved).map_err(|e| HarnessError::SpawnFailed(e.to_string()))
    }
}

impl fmt::Debug for AcpHarnessAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcpHarnessAdapter")
            .field("active_sessions", &self.sessions.keys())
            .field("client_version", &self.client_version)
            .field("launch_env", &self.launch_env)
            .finish_non_exhaustive()
    }
}

impl Drop for AcpHarnessAdapter {
    fn drop(&mut self) {
        for (_id, mut session) in std::mem::take(&mut self.sessions) {
            session.cancel_signal.store(true, Ordering::SeqCst);
            session.ack_if_awaiting();
            let _ = session.command_tx.send(WorkerCommand::Close);
            if let Some(handle) = session.worker_handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl HarnessRuntimePort for AcpHarnessAdapter {
    fn probe_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, HarnessError> {
        let child = match self.spawn_child(binding, None) {
            Ok(child) => child,
            Err(_err) => {
                return Ok(BindingProbeOutcome {
                    ok: false,
                    capabilities: CapabilitySet::new(),
                    diagnostics: None,
                });
            }
        };

        let mut runtime = AcpRuntime::new(child);
        let init_result = runtime.initialize(&self.client_version);
        let _ = runtime.close();

        match init_result {
            Ok(negotiated) => {
                let capabilities = map_negotiated_capabilities(negotiated);
                Ok(BindingProbeOutcome {
                    ok: true,
                    capabilities,
                    diagnostics: None,
                })
            }
            Err(_err) => Ok(BindingProbeOutcome {
                ok: false,
                capabilities: CapabilitySet::new(),
                diagnostics: None,
            }),
        }
    }

    fn create_session(
        &mut self,
        binding: &AcpHarnessBinding,
        _thread_id: &ThreadId,
        project: Option<&ProjectRef>,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        let child = self.spawn_child(binding, project)?;
        let mut runtime = AcpRuntime::new(child);

        let negotiated = runtime
            .initialize(&self.client_version)
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;

        let cwd = project.map_or("", |p| p.path.as_str());
        let raw_session_id = runtime
            .new_session(cwd, Vec::new())
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;

        let session_id = HarnessSessionId::new(&raw_session_id)
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;
        let capabilities = map_negotiated_capabilities(negotiated);

        let (command_tx, command_rx) = std::sync::mpsc::sync_channel(CHANNEL_BOUND);
        let (event_tx, event_rx) = std::sync::mpsc::sync_channel(CHANNEL_BOUND);
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        let cancel_signal = Arc::new(AtomicBool::new(false));
        let permission_txs = Arc::new(Mutex::new(BTreeMap::new()));

        let worker_handle = {
            let cancel_signal = Arc::clone(&cancel_signal);
            let permission_txs = Arc::clone(&permission_txs);
            std::thread::Builder::new()
                .name(format!("acp-worker-{}", session_id.as_str()))
                .spawn(move || {
                    run_session_worker(
                        runtime,
                        command_rx,
                        event_tx,
                        ack_rx,
                        cancel_signal,
                        permission_txs,
                    );
                })
                .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?
        };

        self.sessions.insert(
            session_id.clone(),
            SessionWorkerHandle {
                command_tx,
                event_rx,
                ack_tx,
                event_awaiting_ack: false,
                cancel_signal,
                permission_txs,
                worker_handle: Some(worker_handle),
            },
        );

        Ok(HarnessSessionInfo {
            session_id,
            capabilities,
        })
    }

    fn resume_session(
        &mut self,
        binding: &AcpHarnessBinding,
        session_id: &HarnessSessionId,
        _thread_id: &ThreadId,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        let child = self.spawn_child(binding, None)?;
        let mut runtime = AcpRuntime::new(child);

        let negotiated = runtime
            .initialize(&self.client_version)
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;

        if !negotiated.may_resume() {
            let _ = runtime.close();
            return Err(HarnessError::Protocol(
                "agent does not support session resume".to_string(),
            ));
        }

        let raw_session_id = runtime
            .load_session("", session_id.as_str(), Vec::new())
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;

        let session_id = HarnessSessionId::new(&raw_session_id)
            .map_err(|e| HarnessError::Protocol(e.to_string()))?;
        let capabilities = map_negotiated_capabilities(negotiated);

        let (command_tx, command_rx) = std::sync::mpsc::sync_channel(CHANNEL_BOUND);
        let (event_tx, event_rx) = std::sync::mpsc::sync_channel(CHANNEL_BOUND);
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        let cancel_signal = Arc::new(AtomicBool::new(false));
        let permission_txs = Arc::new(Mutex::new(BTreeMap::new()));

        let worker_handle = {
            let cancel_signal = Arc::clone(&cancel_signal);
            let permission_txs = Arc::clone(&permission_txs);
            std::thread::Builder::new()
                .name(format!("acp-worker-{}", session_id.as_str()))
                .spawn(move || {
                    run_session_worker(
                        runtime,
                        command_rx,
                        event_tx,
                        ack_rx,
                        cancel_signal,
                        permission_txs,
                    );
                })
                .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?
        };

        self.sessions.insert(
            session_id.clone(),
            SessionWorkerHandle {
                command_tx,
                event_rx,
                ack_tx,
                event_awaiting_ack: false,
                cancel_signal,
                permission_txs,
                worker_handle: Some(worker_handle),
            },
        );

        Ok(HarnessSessionInfo {
            session_id,
            capabilities,
        })
    }

    fn send_prompt(
        &mut self,
        session_id: &HarnessSessionId,
        prompt: HarnessPromptRequest,
    ) -> Result<(), HarnessError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.clone()))?;
        session.ack_if_awaiting();
        session
            .command_tx
            .send(WorkerCommand::Prompt(prompt))
            .map_err(|e| HarnessError::Transport(e.to_string()))
    }

    fn cancel_turn(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.clone()))?;
        session.cancel_signal.store(true, Ordering::SeqCst);
        session.ack_if_awaiting();
        Ok(())
    }

    fn decide_permission(
        &mut self,
        session_id: &HarnessSessionId,
        event_id: &EventId,
        decision: PermissionDecision,
    ) -> Result<(), HarnessError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.clone()))?;
        let mut map = session
            .permission_txs
            .lock()
            .map_err(|_| HarnessError::Other("permission mutex poisoned".to_string()))?;
        if let Some(tx) = map.remove(event_id) {
            let _ = tx.send(decision);
            Ok(())
        } else if map.len() == 1 {
            if let Some((_, tx)) = map.pop_first() {
                let _ = tx.send(decision);
            }
            Ok(())
        } else {
            Err(HarnessError::Other(format!(
                "pending permission {event_id} not found"
            )))
        }
    }

    fn poll_event(
        &mut self,
        session_id: &HarnessSessionId,
    ) -> Result<Option<HarnessEvent>, HarnessError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.clone()))?;
        session.ack_if_awaiting();
        match session.event_rx.try_recv() {
            Ok(event) => {
                if requires_ack(&event) {
                    session.event_awaiting_ack = true;
                }
                Ok(Some(event))
            }
            Err(
                std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected,
            ) => Ok(None),
        }
    }

    fn close_session(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        if let Some(mut session) = self.sessions.remove(session_id) {
            session.cancel_signal.store(true, Ordering::SeqCst);
            session.ack_if_awaiting();
            let _ = session.command_tx.send(WorkerCommand::Close);
            if let Some(handle) = session.worker_handle.take() {
                let _ = handle.join();
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
fn run_session_worker(
    mut runtime: AcpRuntime<AcpChild>,
    command_rx: std::sync::mpsc::Receiver<WorkerCommand>,
    event_tx: std::sync::mpsc::SyncSender<HarnessEvent>,
    ack_rx: std::sync::mpsc::Receiver<()>,
    cancel_signal: Arc<AtomicBool>,
    permission_txs: Arc<Mutex<BTreeMap<EventId, std::sync::mpsc::SyncSender<PermissionDecision>>>>,
) {
    while let Ok(cmd) = command_rx.recv() {
        match cmd {
            WorkerCommand::Prompt(req) => {
                cancel_signal.store(false, Ordering::SeqCst);
                let prompt_blocks = vec![ContentBlock::Text { text: req.prompt }];
                let event_tx_clone = event_tx.clone();
                let turn_id = req.turn_id.clone();
                let ack_rx_ref = &ack_rx;

                let on_event = {
                    let event_tx = event_tx_clone.clone();
                    let turn_id = turn_id.clone();
                    move |norm: NormalizedEvent| -> Result<(), AcpError> {
                        let ev = match norm.event {
                            AgentEvent::TurnStarted => HarnessEvent::Started {
                                turn_id: turn_id.clone(),
                            },
                            AgentEvent::Delta { text } => HarnessEvent::MessageDelta { text },
                            AgentEvent::TurnCompleted { .. }
                            | AgentEvent::TurnFailed { .. }
                            | AgentEvent::PermissionRequested { .. }
                            | AgentEvent::SessionCreated { .. }
                            | AgentEvent::SessionResumed { .. } => return Ok(()),
                            AgentEvent::ToolObserved {
                                tool_call_id,
                                status,
                            } => HarnessEvent::RawUnknown {
                                name: "acp.tool".to_string(),
                                data: serde_json::json!({
                                    "toolCallId": tool_call_id,
                                    "status": status,
                                })
                                .to_string(),
                            },
                            AgentEvent::Preserved { provider_kind, raw } => {
                                HarnessEvent::RawUnknown {
                                    name: provider_kind,
                                    data: raw.to_string(),
                                }
                            }
                        };
                        if event_tx.send(ev).is_ok() {
                            let _ = ack_rx_ref.recv();
                        }
                        Ok(())
                    }
                };

                let on_permission = {
                    let event_tx = event_tx_clone;
                    let permission_txs = Arc::clone(&permission_txs);
                    move |rpc_id: &RpcId,
                          tool_call_id: &Option<String>|
                          -> Result<Option<serde_json::Value>, AcpError> {
                        let rpc_str = match rpc_id {
                            RpcId::Number(n) => format!("req{n}"),
                            RpcId::Text(s) => s.clone(),
                        };
                        let perm_event_id = EventId::from_str(&format!("evt_{rpc_str:0<16}"))
                            .unwrap_or_else(|_| FALLBACK_EVENT_ID.clone());

                        let (perm_tx, perm_rx) = std::sync::mpsc::sync_channel(1);
                        {
                            let mut map = permission_txs.lock().map_err(|_| AcpError::IoError {
                                diagnostic: "permission mutex poisoned".to_string(),
                            })?;
                            map.insert(perm_event_id.clone(), perm_tx);
                        }

                        let desc_str = tool_call_id.as_deref().unwrap_or("permission requested");
                        let description = PermissionDescription::try_from(desc_str)
                            .unwrap_or_else(|_| FALLBACK_PERMISSION_DESC.clone());

                        let req_event = HarnessEvent::PermissionRequest {
                            event_id: perm_event_id,
                            kind: PermissionKind::Execute,
                            description,
                        };
                        let _ = event_tx.send(req_event);

                        match perm_rx.recv() {
                            Ok(PermissionDecision::Approved) => {
                                Ok(Some(serde_json::json!({ "optionId": "allow" })))
                            }
                            Ok(PermissionDecision::Denied | PermissionDecision::Pending) => {
                                Ok(None)
                            }
                            Err(_) => Ok(None),
                        }
                    }
                };

                let prompt_res = runtime.prompt_with_handlers(
                    prompt_blocks,
                    on_event,
                    on_permission,
                    Some(&cancel_signal),
                );

                match prompt_res {
                    Ok(StopReason::Cancelled) | Err(AcpError::Cancelled) => {
                        let _ = event_tx.send(HarnessEvent::Cancelled);
                    }
                    Ok(_) => {
                        let _ = event_tx.send(HarnessEvent::Completed { payload: None });
                    }
                    Err(err) => {
                        let _ = event_tx.send(HarnessEvent::Failed {
                            error: err.to_string(),
                            delivery: DeliveryState::Indeterminate,
                        });
                        let _ = event_tx.send(HarnessEvent::ProcessExited { exit_code: None });
                    }
                }
            }
            WorkerCommand::Close => {
                let _ = runtime.close();
                break;
            }
        }
    }
}

fn parse_launch_config(
    command: &BoundedPath,
    project: Option<&ProjectRef>,
) -> Result<LaunchConfig, HarnessError> {
    let raw = command.as_str();
    let mut config = if Path::new(raw).exists() {
        LaunchConfig::new(raw).map_err(|e| HarnessError::SpawnFailed(e.to_string()))?
    } else {
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if let Some((first, rest)) = parts.split_first() {
            let mut cfg =
                LaunchConfig::new(*first).map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
            for arg in rest {
                cfg = cfg
                    .with_arg(*arg)
                    .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
            }
            cfg
        } else {
            LaunchConfig::new(raw).map_err(|e| HarnessError::SpawnFailed(e.to_string()))?
        }
    };

    if let Some(proj) = project {
        config = config
            .with_working_dir(proj.path.as_str())
            .map_err(|e| HarnessError::SpawnFailed(e.to_string()))?;
    }

    Ok(config)
}

fn map_negotiated_capabilities(neg: NegotiatedCapabilities) -> CapabilitySet {
    let mut set = CapabilitySet::new();
    let _ = set.declare(
        "session.resume",
        if neg.may_resume() {
            CapabilitySupport::Supported
        } else {
            CapabilitySupport::Unsupported
        },
    );
    let _ = set.declare(
        "turn.steer",
        if neg.supports_steer() {
            CapabilitySupport::Supported
        } else {
            CapabilitySupport::Unsupported
        },
    );
    let _ = set.declare("prompt.text", CapabilitySupport::Supported);
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use altior_acp::SecretRef;

    #[test]
    fn static_fallbacks_are_valid() {
        assert_eq!(FALLBACK_EVENT_ID.as_str(), "evt_perm000000000000");
        assert_eq!(FALLBACK_PERMISSION_DESC.as_str(), "permission required");
    }

    #[test]
    fn acp_harness_adapter_debug_does_not_leak_secrets() {
        let secret_ref = SecretRef::new("secret-key-1").unwrap();
        let resolver = Arc::new(|_: &SecretRef| -> Result<String, AcpError> {
            Ok("SK_CANARY_VALUE_SUPER_SECRET".to_string())
        });
        let mut launch_env = BTreeMap::new();
        launch_env.insert("API_KEY".to_string(), EnvVarValue::SecretRef(secret_ref));
        let adapter = AcpHarnessAdapter::new()
            .with_secret_resolver(resolver)
            .with_launch_env(launch_env);

        let debug_str = format!("{adapter:?}");
        assert!(!debug_str.contains("SK_CANARY_VALUE_SUPER_SECRET"));
        assert!(debug_str.contains("secret-key-1"));
    }
}
