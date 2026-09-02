//! ACP subprocess runtime adapter (P1.2, ADR 0007).
//!
//! Orchestrates the end-to-end lifecycle of an ACP agent subprocess:
//! handshake negotiation, session management, turn prompt streaming,
//! permission routing, mid-turn cancellation, and clean shutdown.
//!
//! Unknown notifications are preserved as bounded diagnostics; all failure
//! modes (EOF, malformed JSON-RPC frames, oversized lines, abnormal child exits)
//! are surfaced as typed [`AcpError`]s with deterministic delivery tracking.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::delivery::{DeliveryCause, PromptDelivery};
use crate::error::AcpError;
use crate::lifecycle::{AgentLifecycle, HostAction};
use crate::mapping::{NormalizedEvent, normalize_message};
use crate::messages::{ContentBlock, InitializeResult, NewSessionResult, PromptResult, StopReason};
use crate::negotiation::{NegotiatedCapabilities, negotiate};
use crate::requests::{PendingKind, PendingRequests, RequestBuilder};
use crate::transport::ProcessTransport;
use crate::wire::{RpcId, RpcMessage};

/// High-level runtime adapter managing one ACP agent subprocess session.
pub struct AcpRuntime<T: ProcessTransport> {
    transport: T,
    builder: RequestBuilder,
    pending: PendingRequests,
    capabilities: Option<NegotiatedCapabilities>,
    lifecycle: AgentLifecycle,
    delivery: PromptDelivery,
    session_id: Option<String>,
}

/// How often the prompt loop re-checks the cancellation signal while the
/// child produces no output.
const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

impl<T: ProcessTransport> AcpRuntime<T> {
    /// Creates a new runtime adapter around the provided transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            builder: RequestBuilder::new(),
            pending: PendingRequests::new(),
            capabilities: None,
            lifecycle: AgentLifecycle::spawned(),
            delivery: PromptDelivery::not_sent(),
            session_id: None,
        }
    }

    /// Accesses the underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutably accesses the underlying transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Returns the negotiated capabilities if the handshake has completed.
    #[must_use]
    pub fn capabilities(&self) -> Option<&NegotiatedCapabilities> {
        self.capabilities.as_ref()
    }

    /// Returns the active ACP session ID, if bound.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Returns the delivery classification of the latest prompt attempt.
    #[must_use]
    pub fn delivery(&self) -> &PromptDelivery {
        &self.delivery
    }

    /// Returns the current lifecycle state machine.
    #[must_use]
    pub fn lifecycle(&self) -> &AgentLifecycle {
        &self.lifecycle
    }

    /// Performs the `initialize` handshake, establishing negotiated capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on transport failures, invalid responses, or RPC errors.
    pub fn initialize(&mut self, client_version: &str) -> Result<NegotiatedCapabilities, AcpError> {
        let (id, msg) = self.builder.build_initialize(client_version);
        self.pending.insert(id.clone(), PendingKind::Initialize);
        self.transport.write_line(&msg.encode()?)?;

        let result = self.wait_for_response(&id)?;
        let init_result: InitializeResult = serde_json::from_value(result)?;
        let negotiated = negotiate(&init_result);
        self.capabilities = Some(negotiated);
        Ok(negotiated)
    }

    /// Creates a new ACP session via `session/new`.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on RPC or transport errors.
    pub fn new_session(&mut self, cwd: &str, mcp_servers: Vec<Value>) -> Result<String, AcpError> {
        let (id, msg) = self.builder.build_new_session(cwd, mcp_servers);
        self.pending.insert(id.clone(), PendingKind::NewSession);
        self.transport.write_line(&msg.encode()?)?;

        let result = self.wait_for_response(&id)?;
        let session_res: NewSessionResult = serde_json::from_value(result)?;
        self.session_id = Some(session_res.session_id.clone());
        Ok(session_res.session_id)
    }

    /// Resumes an existing ACP session via `session/load`.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::Unsupported`] if the agent does not support session resume,
    /// or [`AcpError`] on failure.
    pub fn load_session(
        &mut self,
        cwd: &str,
        session_id: &str,
        mcp_servers: Vec<Value>,
    ) -> Result<String, AcpError> {
        let caps = self.capabilities.ok_or(AcpError::OutOfOrder {
            attempted: "load_session before initialize handshake",
        })?;
        if !caps.may_resume() {
            return Err(AcpError::Unsupported {
                feature: "session/load",
            });
        }

        let (id, msg) = self
            .builder
            .build_load_session(cwd, session_id, mcp_servers);
        self.pending.insert(id.clone(), PendingKind::LoadSession);
        self.transport.write_line(&msg.encode()?)?;

        let _ = self.wait_for_response(&id)?;
        self.session_id = Some(session_id.to_owned());
        Ok(session_id.to_owned())
    }

    /// Sends a prompt and streams normalized events until turn completion.
    ///
    /// # Errors
    ///
    /// Returns typed [`AcpError`] on stream interruption, oversized lines,
    /// malformed frames, or RPC errors.
    pub fn prompt<F>(
        &mut self,
        prompt: Vec<ContentBlock>,
        on_event: F,
    ) -> Result<StopReason, AcpError>
    where
        F: FnMut(NormalizedEvent) -> Result<(), AcpError>,
    {
        self.prompt_with_handlers(prompt, on_event, |_id, _tool| Ok(None), None)
    }

    /// Sends a prompt with custom permission and cancellation handlers.
    ///
    /// # Errors
    ///
    /// Returns typed [`AcpError`] on failure.
    #[allow(clippy::too_many_lines)]
    pub fn prompt_with_handlers<F, P>(
        &mut self,
        prompt: Vec<ContentBlock>,
        mut on_event: F,
        mut on_permission: P,
        cancel_signal: Option<&AtomicBool>,
    ) -> Result<StopReason, AcpError>
    where
        F: FnMut(NormalizedEvent) -> Result<(), AcpError>,
        P: FnMut(&RpcId, &Option<String>) -> Result<Option<Value>, AcpError>,
    {
        let session_id = self.session_id.clone().ok_or(AcpError::OutOfOrder {
            attempted: "prompt without an active session",
        })?;

        let (prompt_id, msg) = self.builder.build_prompt(&session_id, prompt);
        self.pending.insert(prompt_id.clone(), PendingKind::Prompt);

        self.delivery = PromptDelivery::not_sent();
        self.delivery.mark_written()?;
        self.lifecycle.on_prompt_written();

        self.transport.write_line(&msg.encode()?)?;

        let mut cancel_sent = false;

        // Read stream until prompt completes or error occurs. Reads are
        // interruptible: while the child stays silent we re-check the
        // cancellation signal instead of blocking indefinitely.
        loop {
            // Check cancellation signal
            if !cancel_sent
                && let Some(signal) = cancel_signal
                && signal.load(Ordering::Relaxed)
            {
                self.trigger_cancel(&session_id)?;
                cancel_sent = true;
            }

            let line_opt = self
                .transport
                .read_line_timeout(CANCEL_POLL_INTERVAL)
                .inspect_err(|_| {
                    let _ = self
                        .delivery
                        .on_connection_lost(DeliveryCause::ProcessExited);
                    let _ = self.lifecycle.on_process_lost();
                })?;

            let Some(line) = line_opt else {
                // Timed out waiting for output: loop to re-check cancellation.
                continue;
            };

            let Some(line) = line else {
                let _ = self
                    .delivery
                    .on_connection_lost(DeliveryCause::ProcessExited);
                let _ = self.lifecycle.on_process_lost();
                return Err(AcpError::UnexpectedEof {
                    diagnostic: "stream reached EOF while awaiting prompt completion".to_owned(),
                });
            };

            let message = RpcMessage::decode(&line).inspect_err(|_| {
                let _ = self
                    .delivery
                    .on_connection_lost(DeliveryCause::ProcessExited);
                let _ = self.lifecycle.on_process_lost();
            })?;

            // Normalize message events and deliver to consumer.
            let events = normalize_message(&message)?;
            for event in events {
                if let Err(AcpError::Cancelled) = on_event(event)
                    && !cancel_sent
                {
                    self.trigger_cancel(&session_id)?;
                    cancel_sent = true;
                }
            }

            match message {
                RpcMessage::Response { id, result } if id == prompt_id => {
                    self.pending.remove(&id);
                    self.delivery.on_prompt_response()?;
                    self.lifecycle.on_prompt_settled()?;
                    let prompt_res: PromptResult = serde_json::from_value(result)?;
                    return Ok(prompt_res.stop_reason);
                }
                RpcMessage::ErrorResponse { id, error } if id == prompt_id => {
                    self.pending.remove(&id);
                    self.delivery.on_error_response()?;
                    self.lifecycle.on_prompt_settled()?;
                    return Err(AcpError::RpcError {
                        code: error.code,
                        message: error.message,
                    });
                }
                RpcMessage::Request { id, method, params }
                    if method == "session/request_permission" =>
                {
                    let actions = self.lifecycle.on_permission_requested(id.clone())?;
                    let mut answered = false;
                    for action in actions {
                        if let HostAction::AnswerPermissionCancelled { ref id } = action {
                            let msg = RequestBuilder::build_permission_cancelled(id.clone());
                            self.transport.write_line(&msg.encode()?)?;
                            answered = true;
                        }
                    }

                    if !answered {
                        let tool_call_id = params
                            .get("toolCall")
                            .and_then(|tc| tc.get("toolCallId"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);

                        let outcome_opt = on_permission(&id, &tool_call_id)?;
                        let reply = match outcome_opt {
                            Some(outcome) => RequestBuilder::build_permission_response(id, outcome),
                            None => RequestBuilder::build_permission_cancelled(id),
                        };
                        self.transport.write_line(&reply.encode()?)?;
                    }
                }
                RpcMessage::Request { id, method, .. } => {
                    // Refuse unadvertised agent requests (e.g. fs/*).
                    let refusal = RequestBuilder::build_refusal(id, &method);
                    self.transport.write_line(&refusal.encode()?)?;
                }
                _ => {}
            }
        }
    }

    /// Sends mid-turn steering prompt if supported.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::Unsupported`] if steer capability is false.
    pub fn steer(&mut self, prompt: Vec<ContentBlock>) -> Result<RpcId, AcpError> {
        let caps = self.capabilities.ok_or(AcpError::OutOfOrder {
            attempted: "steer before initialize handshake",
        })?;
        let session_id = self.session_id.as_deref().ok_or(AcpError::OutOfOrder {
            attempted: "steer without an active session",
        })?;

        let (id, msg) = self.builder.build_steer(session_id, prompt, &caps)?;
        self.pending.insert(id.clone(), PendingKind::Steer);
        self.transport.write_line(&msg.encode()?)?;
        Ok(id)
    }

    /// Cancels the active turn.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on write failure or out-of-order state.
    pub fn cancel(&mut self) -> Result<(), AcpError> {
        let session_id = self.session_id.clone().ok_or(AcpError::OutOfOrder {
            attempted: "cancel without an active session",
        })?;
        self.trigger_cancel(&session_id)
    }

    fn trigger_cancel(&mut self, session_id: &str) -> Result<(), AcpError> {
        let actions = self.lifecycle.on_cancel_requested()?;
        for action in actions {
            self.execute_host_action(&action, session_id, &RpcId::Number(0))?;
        }
        Ok(())
    }

    /// Responds to an agent permission request.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on write failure.
    pub fn answer_permission(&mut self, id: RpcId, outcome: Value) -> Result<(), AcpError> {
        let reply = RequestBuilder::build_permission_response(id, outcome);
        self.transport.write_line(&reply.encode()?)
    }

    /// Gracefully closes the subprocess transport.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on close failure.
    pub fn close(&mut self) -> Result<Option<std::process::ExitStatus>, AcpError> {
        self.transport.close()
    }

    /// Forcefully terminates the subprocess transport.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError`] on termination failure.
    pub fn terminate(&mut self) -> Result<(), AcpError> {
        self.transport.terminate()
    }

    fn wait_for_response(&mut self, expected_id: &RpcId) -> Result<Value, AcpError> {
        loop {
            let line = self
                .transport
                .read_line()?
                .ok_or_else(|| AcpError::UnexpectedEof {
                    diagnostic: format!("EOF while awaiting response for {expected_id:?}"),
                })?;

            let message = RpcMessage::decode(&line)?;
            match message {
                RpcMessage::Response { id, result } if &id == expected_id => {
                    self.pending.remove(&id);
                    return Ok(result);
                }
                RpcMessage::ErrorResponse { id, error } if &id == expected_id => {
                    self.pending.remove(&id);
                    return Err(AcpError::RpcError {
                        code: error.code,
                        message: error.message,
                    });
                }
                RpcMessage::Request { id, method, .. } => {
                    let refusal = RequestBuilder::build_refusal(id, &method);
                    self.transport.write_line(&refusal.encode()?)?;
                }
                _ => {}
            }
        }
    }

    fn execute_host_action(
        &mut self,
        action: &HostAction,
        session_id: &str,
        _request_id: &RpcId,
    ) -> Result<(), AcpError> {
        match action {
            HostAction::Continue | HostAction::TurnSettled => Ok(()),
            HostAction::SendCancelNotification => {
                let msg = RequestBuilder::build_cancel(session_id);
                self.transport.write_line(&msg.encode()?)
            }
            HostAction::AnswerPermissionCancelled { id } => {
                let msg = RequestBuilder::build_permission_cancelled(id.clone());
                self.transport.write_line(&msg.encode()?)
            }
            HostAction::KillAndReap => self.transport.terminate(),
        }
    }
}

impl<T: ProcessTransport + fmt::Debug> fmt::Debug for AcpRuntime<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcpRuntime")
            .field("transport", &self.transport)
            .field("capabilities", &self.capabilities)
            .field("session_id", &self.session_id)
            .field("delivery", &self.delivery)
            .finish_non_exhaustive()
    }
}
