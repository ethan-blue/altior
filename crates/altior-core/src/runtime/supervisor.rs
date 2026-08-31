//! Thread-level runtime supervisor state machine (P1.2).
//!
//! Enforces:
//! - Pure state transitions: `Idle` -> `Starting` -> `Ready` -> `Prompting` -> `AwaitingPermission` -> `Cancelling` -> `Closed` / `Crashed`
//! - Bounded per-thread active operations (at most 1 active turn at a time)
//! - Correlation of `OperationId`, `TurnId`, `ThreadId`, `EventId`
//! - Capability gate checks with typed errors
//! - Pre-call intent checkpoint and post-call settlement
//! - Indeterminate delivery classification on transport / process crash, strictly forbidding auto-resend
//! - Desktop UI detachment resilience

use std::collections::BTreeMap;
use std::str::FromStr;

use altior_domain::{
    DeliveryState, EventId, OperationId, PermissionDecision, ThreadId, TurnId, TurnState,
    UnixMillis,
};
use altior_protocol::{CapabilityId, CapabilitySet, CapabilitySupport};

use super::diagnostics::BoundedDiagnosticsSummary;
use super::ports::{HarnessRuntimePort, RuntimeCheckpointPort};
use super::state::{
    CancelOutcome, CheckpointIntent, CheckpointSettled, HarnessEvent, HarnessPromptRequest,
    HarnessSessionId, RuntimeError, RuntimeEvent, SupervisorState, TurnAdmission,
};
use crate::operations::OperationRegistry;
use crate::ownership::{DesktopLifecycle, TurnOwnership, TurnTransition};

/// Capacity of the per-thread admitted and remembered finished operations.
const OPERATION_REGISTRY_CAPACITY: usize = 32;

/// Last completed or terminal turn metadata for boundary resend safety.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnExecutionRecord {
    pub turn_id: TurnId,
    pub operation_id: OperationId,
    pub delivery: DeliveryState,
    pub state: TurnState,
}

/// The runtime supervisor state machine for a single thread.
#[derive(Debug)]
pub struct ThreadRuntimeSupervisor {
    thread_id: ThreadId,
    session_id: Option<HarnessSessionId>,
    capabilities: CapabilitySet,
    state: SupervisorState,
    ownership: TurnOwnership,
    operations: OperationRegistry,
    turn_history: BTreeMap<TurnId, TurnExecutionRecord>,
}

impl ThreadRuntimeSupervisor {
    /// Creates a new supervisor for `thread_id` in `Idle` state.
    ///
    /// # Panics
    ///
    /// Panics if static configuration capacity is zero.
    #[must_use]
    pub fn new(thread_id: ThreadId) -> Self {
        Self {
            thread_id,
            session_id: None,
            capabilities: CapabilitySet::new(),
            state: SupervisorState::Idle,
            ownership: TurnOwnership::new(),
            operations: OperationRegistry::new(OPERATION_REGISTRY_CAPACITY)
                .expect("non-zero capacity"),
            turn_history: BTreeMap::new(),
        }
    }

    /// The thread identifier under supervision.
    #[must_use]
    pub fn thread_id(&self) -> &ThreadId {
        &self.thread_id
    }

    /// The current supervisor state.
    #[must_use]
    pub fn state(&self) -> &SupervisorState {
        &self.state
    }

    /// The active session ID, if bound.
    #[must_use]
    pub fn session_id(&self) -> Option<&HarnessSessionId> {
        self.session_id.as_ref()
    }

    /// The negotiated capabilities of this session.
    #[must_use]
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Marks the supervisor as starting a session.
    pub fn mark_starting(&mut self) {
        self.state = SupervisorState::Starting;
    }

    /// Resets the supervisor to `Idle` state when session establishment fails.
    pub fn reset_idle(&mut self) {
        self.state = SupervisorState::Idle;
        self.session_id = None;
    }

    /// Marks the session established and ready.
    pub fn on_session_established(
        &mut self,
        session_id: HarnessSessionId,
        capabilities: CapabilitySet,
    ) {
        self.session_id = Some(session_id);
        self.capabilities = capabilities;
        self.state = SupervisorState::Ready;
    }

    /// Validates capability support against negotiated capability declarations.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::UnsupportedCapability`] if explicitly declared `Unsupported`.
    pub fn check_capability(&self, capability_name: &str) -> Result<(), RuntimeError> {
        if let Ok(cap_id) = CapabilityId::from_str(capability_name)
            && self.capabilities.get(&cap_id) == Some(CapabilitySupport::Unsupported)
        {
            return Err(RuntimeError::UnsupportedCapability(cap_id));
        }
        Ok(())
    }

    /// Checks whether a prompt turn can be admitted on this thread without mutating state.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on state mismatch, forbidden resend, or session error.
    pub fn preflight_prompt(
        &self,
        operation_id: &OperationId,
        turn_id: &TurnId,
    ) -> Result<TurnAdmission, RuntimeError> {
        if let Some(record) = self.turn_history.get(turn_id)
            && (record.delivery == DeliveryState::Indeterminate
                || record.delivery == DeliveryState::Confirmed)
        {
            return Err(RuntimeError::AutomaticResendForbidden {
                thread_id: self.thread_id.clone(),
                turn_id: turn_id.clone(),
                delivery: record.delivery,
            });
        }

        match &self.state {
            SupervisorState::Ready => {}
            SupervisorState::Prompting {
                turn_id: active, ..
            }
            | SupervisorState::AwaitingPermission {
                turn_id: active, ..
            }
            | SupervisorState::Cancelling {
                turn_id: Some(active),
                ..
            } => {
                return Err(RuntimeError::ActiveOperationInProgress {
                    thread_id: self.thread_id.clone(),
                    turn_id: active.clone(),
                });
            }
            other => {
                return Err(RuntimeError::SessionNotReady {
                    state: format!("{other:?}"),
                });
            }
        }

        if self.session_id.is_none() {
            return Err(RuntimeError::SessionNotReady {
                state: "no bound session".to_string(),
            });
        }

        if self.operations.knows(operation_id) {
            return Ok(TurnAdmission::Duplicate);
        }

        Ok(TurnAdmission::Admitted)
    }

    /// Returns the currently active turn ID, if any.
    #[must_use]
    pub fn active_turn_id(&self) -> Option<&TurnId> {
        match &self.state {
            SupervisorState::Prompting { turn_id, .. }
            | SupervisorState::AwaitingPermission { turn_id, .. }
            | SupervisorState::Cancelling {
                turn_id: Some(turn_id),
                ..
            } => Some(turn_id),
            _ => None,
        }
    }

    /// Returns the active turn ID if this supervisor is awaiting the given permission decision.
    #[must_use]
    pub fn active_permission_turn_id(&self, permission_id: &EventId) -> Option<&TurnId> {
        match &self.state {
            SupervisorState::AwaitingPermission {
                turn_id,
                pending_permission_id,
                ..
            } if pending_permission_id == permission_id => Some(turn_id),
            _ => None,
        }
    }

    /// Initiates a prompt turn on this thread.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on state mismatch, active operation conflict, forbidden resend,
    /// or transport/checkpoint failure.
    #[allow(clippy::too_many_lines)]
    pub fn prompt<H, C>(
        &mut self,
        operation_id: OperationId,
        turn_id: TurnId,
        content: &str,
        now: UnixMillis,
        harness: &mut H,
        checkpoint: &mut C,
    ) -> Result<TurnAdmission, RuntimeError>
    where
        H: HarnessRuntimePort,
        C: RuntimeCheckpointPort,
    {
        // 1. Enforce boundary discipline FIRST: automatic prompt resend of indeterminate or confirmed turn is forbidden
        if let Some(record) = self.turn_history.get(&turn_id)
            && (record.delivery == DeliveryState::Indeterminate
                || record.delivery == DeliveryState::Confirmed)
        {
            return Err(RuntimeError::AutomaticResendForbidden {
                thread_id: self.thread_id.clone(),
                turn_id,
                delivery: record.delivery,
            });
        }

        // 2. Enforce state machine preconditions
        match &self.state {
            SupervisorState::Ready => {}
            SupervisorState::Prompting {
                turn_id: active, ..
            }
            | SupervisorState::AwaitingPermission {
                turn_id: active, ..
            }
            | SupervisorState::Cancelling {
                turn_id: Some(active),
                ..
            } => {
                return Err(RuntimeError::ActiveOperationInProgress {
                    thread_id: self.thread_id.clone(),
                    turn_id: active.clone(),
                });
            }
            other => {
                return Err(RuntimeError::SessionNotReady {
                    state: format!("{other:?}"),
                });
            }
        }

        let Some(session_id) = self.session_id.clone() else {
            return Err(RuntimeError::SessionNotReady {
                state: "no bound session".to_string(),
            });
        };

        // 3. Operation dedup check
        match self.operations.admit_operation(&operation_id) {
            Ok(crate::operations::Admission::Duplicate) => {
                return Ok(TurnAdmission::Duplicate);
            }
            Ok(crate::operations::Admission::Execute) => {}
            Err(err) => return Err(RuntimeError::OperationAdmitFailed(err)),
        }

        // 4. Intent Checkpoint BEFORE calling harness adapter
        let intent = CheckpointIntent::Prompt {
            thread_id: self.thread_id.clone(),
            turn_id: turn_id.clone(),
            operation_id: operation_id.clone(),
            timestamp: now,
        };
        checkpoint.checkpoint_intent(&intent)?;

        // 5. Call external harness adapter
        let prompt_req = HarnessPromptRequest {
            turn_id: turn_id.clone(),
            operation_id: operation_id.clone(),
            prompt: content.to_string(),
        };

        if let Err(err) = harness.send_prompt(&session_id, prompt_req) {
            // Transport or write error: mark Indeterminate, settle, and move to Crashed
            let delivery = DeliveryState::Indeterminate;
            let settle_deliv = CheckpointSettled::PromptDelivered {
                thread_id: self.thread_id.clone(),
                turn_id: turn_id.clone(),
                delivery,
                timestamp: now,
            };
            let _ = checkpoint.settle_checkpoint(&settle_deliv);

            let settle_term = CheckpointSettled::TurnTerminal {
                thread_id: self.thread_id.clone(),
                turn_id: turn_id.clone(),
                state: TurnState::Failed,
                delivery,
                timestamp: now,
            };
            let _ = checkpoint.settle_checkpoint(&settle_term);

            self.turn_history.insert(
                turn_id.clone(),
                TurnExecutionRecord {
                    turn_id: turn_id.clone(),
                    operation_id: operation_id.clone(),
                    delivery,
                    state: TurnState::Failed,
                },
            );

            self.state = SupervisorState::Crashed {
                reason: err.to_string(),
                delivery,
            };
            return Err(RuntimeError::Harness(err));
        }

        // 6. Settle prompt delivery as Indeterminate (in-flight) and transition to Prompting
        let settle = CheckpointSettled::PromptDelivered {
            thread_id: self.thread_id.clone(),
            turn_id: turn_id.clone(),
            delivery: DeliveryState::Indeterminate,
            timestamp: now,
        };
        checkpoint.settle_checkpoint(&settle)?;

        self.ownership.start(turn_id.clone(), operation_id.clone());
        self.state = SupervisorState::Prompting {
            turn_id,
            operation_id,
            delivery: DeliveryState::Indeterminate,
        };

        Ok(TurnAdmission::Admitted)
    }

    /// Polls events from the harness and applies pure state transitions.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on harness or checkpoint failure.
    #[allow(clippy::too_many_lines)]
    pub fn poll_stream<H, C>(
        &mut self,
        now: UnixMillis,
        harness: &mut H,
        checkpoint: &mut C,
    ) -> Result<Option<RuntimeEvent>, RuntimeError>
    where
        H: HarnessRuntimePort,
        C: RuntimeCheckpointPort,
    {
        let Some(session_id) = self.session_id.clone() else {
            return Ok(None);
        };

        let raw_event = harness.poll_event(&session_id)?;
        let Some(event) = raw_event else {
            return Ok(None);
        };

        match event {
            HarnessEvent::Started { turn_id } => Ok(Some(RuntimeEvent::TurnStarted {
                thread_id: self.thread_id.clone(),
                turn_id,
            })),
            HarnessEvent::MessageDelta { text } => {
                let current_turn = match &self.state {
                    SupervisorState::Prompting { turn_id, .. }
                    | SupervisorState::AwaitingPermission { turn_id, .. }
                    | SupervisorState::Cancelling {
                        turn_id: Some(turn_id),
                        ..
                    } => turn_id.clone(),
                    _ => {
                        let summary = BoundedDiagnosticsSummary::from_raw(&text);
                        return Ok(Some(RuntimeEvent::Unknown {
                            thread_id: self.thread_id.clone(),
                            name: "untracked.message_delta".to_string(),
                            summary,
                        }));
                    }
                };
                Ok(Some(RuntimeEvent::MessageDelta {
                    thread_id: self.thread_id.clone(),
                    turn_id: current_turn,
                    text,
                }))
            }
            HarnessEvent::PermissionRequest {
                event_id,
                kind,
                description,
            } => {
                if let SupervisorState::Prompting {
                    turn_id,
                    operation_id,
                    ..
                } = &self.state
                {
                    let active_turn = turn_id.clone();
                    let active_op = operation_id.clone();
                    self.state = SupervisorState::AwaitingPermission {
                        turn_id: active_turn.clone(),
                        operation_id: active_op,
                        pending_permission_id: event_id.clone(),
                    };
                    Ok(Some(RuntimeEvent::PermissionRequested {
                        thread_id: self.thread_id.clone(),
                        turn_id: active_turn,
                        permission_id: event_id,
                        kind,
                        description,
                    }))
                } else {
                    Ok(None)
                }
            }
            HarnessEvent::Completed { payload } => {
                let (active_turn, active_op) = match &self.state {
                    SupervisorState::Prompting {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::AwaitingPermission {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::Cancelling {
                        turn_id: Some(turn_id),
                        operation_id: Some(operation_id),
                    } => (turn_id.clone(), operation_id.clone()),
                    _ => {
                        let raw_text =
                            payload
                                .as_ref()
                                .map_or("completed without active turn", |p| {
                                    std::str::from_utf8(p.as_bytes())
                                        .unwrap_or("completed without active turn")
                                });
                        let summary = BoundedDiagnosticsSummary::from_raw(raw_text);
                        return Ok(Some(RuntimeEvent::Unknown {
                            thread_id: self.thread_id.clone(),
                            name: "untracked.completed".to_string(),
                            summary,
                        }));
                    }
                };

                let settle = CheckpointSettled::TurnTerminal {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn.clone(),
                    state: TurnState::Completed,
                    delivery: DeliveryState::Confirmed,
                    timestamp: now,
                };
                checkpoint.settle_checkpoint(&settle)?;

                self.operations.retire(&active_op);
                self.turn_history.insert(
                    active_turn.clone(),
                    TurnExecutionRecord {
                        turn_id: active_turn.clone(),
                        operation_id: active_op,
                        delivery: DeliveryState::Confirmed,
                        state: TurnState::Completed,
                    },
                );

                self.state = SupervisorState::Ready;
                Ok(Some(RuntimeEvent::TurnCompleted {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn,
                    payload,
                }))
            }
            HarnessEvent::Cancelled => {
                let (active_turn, active_op) = match &self.state {
                    SupervisorState::Prompting {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::AwaitingPermission {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::Cancelling {
                        turn_id: Some(turn_id),
                        operation_id: Some(operation_id),
                    } => (turn_id.clone(), operation_id.clone()),
                    _ => {
                        let summary =
                            BoundedDiagnosticsSummary::from_raw("cancelled without active turn");
                        return Ok(Some(RuntimeEvent::Unknown {
                            thread_id: self.thread_id.clone(),
                            name: "untracked.cancelled".to_string(),
                            summary,
                        }));
                    }
                };

                let settle = CheckpointSettled::TurnTerminal {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn.clone(),
                    state: TurnState::Cancelled,
                    delivery: DeliveryState::Confirmed,
                    timestamp: now,
                };
                checkpoint.settle_checkpoint(&settle)?;

                self.operations.retire(&active_op);
                self.turn_history.insert(
                    active_turn.clone(),
                    TurnExecutionRecord {
                        turn_id: active_turn.clone(),
                        operation_id: active_op,
                        delivery: DeliveryState::Confirmed,
                        state: TurnState::Cancelled,
                    },
                );

                self.state = SupervisorState::Ready;
                Ok(Some(RuntimeEvent::TurnCancelled {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn,
                }))
            }
            HarnessEvent::Failed { error, delivery } => {
                let (active_turn, active_op) = match &self.state {
                    SupervisorState::Prompting {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::AwaitingPermission {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::Cancelling {
                        turn_id: Some(turn_id),
                        operation_id: Some(operation_id),
                    } => (turn_id.clone(), operation_id.clone()),
                    _ => {
                        let summary = BoundedDiagnosticsSummary::from_raw(&error);
                        return Ok(Some(RuntimeEvent::Unknown {
                            thread_id: self.thread_id.clone(),
                            name: "untracked.failed".to_string(),
                            summary,
                        }));
                    }
                };

                let settle = CheckpointSettled::TurnTerminal {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn.clone(),
                    state: TurnState::Failed,
                    delivery,
                    timestamp: now,
                };
                checkpoint.settle_checkpoint(&settle)?;

                self.operations.retire(&active_op);
                self.turn_history.insert(
                    active_turn.clone(),
                    TurnExecutionRecord {
                        turn_id: active_turn.clone(),
                        operation_id: active_op,
                        delivery,
                        state: TurnState::Failed,
                    },
                );

                if delivery == DeliveryState::Indeterminate {
                    self.state = SupervisorState::Crashed {
                        reason: error.clone(),
                        delivery,
                    };
                } else {
                    self.state = SupervisorState::Ready;
                }
                Ok(Some(RuntimeEvent::TurnFailed {
                    thread_id: self.thread_id.clone(),
                    turn_id: active_turn,
                    reason: error,
                    delivery,
                }))
            }
            HarnessEvent::ProcessExited { exit_code } => {
                let current_state = self.state.clone();
                match current_state {
                    SupervisorState::Prompting {
                        turn_id,
                        operation_id,
                        ..
                    }
                    | SupervisorState::AwaitingPermission {
                        turn_id,
                        operation_id,
                        ..
                    } => {
                        let delivery = DeliveryState::Indeterminate;
                        let settle = CheckpointSettled::TurnTerminal {
                            thread_id: self.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            state: TurnState::Failed,
                            delivery,
                            timestamp: now,
                        };
                        let _ = checkpoint.settle_checkpoint(&settle);

                        self.operations.retire(&operation_id);
                        self.turn_history.insert(
                            turn_id.clone(),
                            TurnExecutionRecord {
                                turn_id: turn_id.clone(),
                                operation_id,
                                delivery,
                                state: TurnState::Failed,
                            },
                        );

                        self.state = SupervisorState::Crashed {
                            reason: format!("unexpected subprocess exit with code {exit_code:?}"),
                            delivery,
                        };
                        Ok(Some(RuntimeEvent::ProcessExited {
                            thread_id: self.thread_id.clone(),
                            exit_code,
                        }))
                    }
                    SupervisorState::Cancelling { .. }
                    | SupervisorState::Ready
                    | SupervisorState::Idle => {
                        self.state = SupervisorState::Closed;
                        Ok(Some(RuntimeEvent::ProcessExited {
                            thread_id: self.thread_id.clone(),
                            exit_code,
                        }))
                    }
                    SupervisorState::Closed
                    | SupervisorState::Crashed { .. }
                    | SupervisorState::Starting => Ok(Some(RuntimeEvent::ProcessExited {
                        thread_id: self.thread_id.clone(),
                        exit_code,
                    })),
                }
            }
            HarnessEvent::RawUnknown { name, data } => {
                let summary = BoundedDiagnosticsSummary::from_raw(&data);
                Ok(Some(RuntimeEvent::Unknown {
                    thread_id: self.thread_id.clone(),
                    name,
                    summary,
                }))
            }
        }
    }

    /// Submits a permission decision on an active turn awaiting approval.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on state mismatch, permission mismatch, or harness failure.
    pub fn decide_permission<H, C>(
        &mut self,
        permission_id: &EventId,
        decision: PermissionDecision,
        now: UnixMillis,
        harness: &mut H,
        checkpoint: &mut C,
    ) -> Result<(), RuntimeError>
    where
        H: HarnessRuntimePort,
        C: RuntimeCheckpointPort,
    {
        let (turn_id, operation_id) = match &self.state {
            SupervisorState::AwaitingPermission {
                turn_id,
                operation_id,
                pending_permission_id,
            } => {
                if pending_permission_id != permission_id {
                    return Err(RuntimeError::PermissionNotFound {
                        thread_id: self.thread_id.clone(),
                        permission_id: permission_id.clone(),
                    });
                }
                (turn_id.clone(), operation_id.clone())
            }
            _ => {
                return Err(RuntimeError::PermissionNotFound {
                    thread_id: self.thread_id.clone(),
                    permission_id: permission_id.clone(),
                });
            }
        };

        let Some(session_id) = self.session_id.clone() else {
            return Err(RuntimeError::SessionNotReady {
                state: "no bound session".to_string(),
            });
        };

        // 1. Checkpoint intent
        let intent = CheckpointIntent::PermissionDecision {
            thread_id: self.thread_id.clone(),
            turn_id: turn_id.clone(),
            permission_id: permission_id.clone(),
            decision,
            timestamp: now,
        };
        checkpoint.checkpoint_intent(&intent)?;

        // 2. Send decision to harness
        harness.decide_permission(&session_id, permission_id, decision)?;

        // 3. Settle checkpoint
        let settle = CheckpointSettled::PermissionSettled {
            thread_id: self.thread_id.clone(),
            turn_id: turn_id.clone(),
            permission_id: permission_id.clone(),
            decision,
            timestamp: now,
        };
        checkpoint.settle_checkpoint(&settle)?;

        // 4. Move back to Prompting
        self.state = SupervisorState::Prompting {
            turn_id,
            operation_id,
            delivery: DeliveryState::Indeterminate,
        };

        Ok(())
    }

    /// Initiates cancellation of an active turn on this thread.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on capability check failure or harness communication error.
    pub fn steer_cancel<H, C>(
        &mut self,
        _operation_id: Option<&OperationId>,
        now: UnixMillis,
        harness: &mut H,
        checkpoint: &mut C,
    ) -> Result<CancelOutcome, RuntimeError>
    where
        H: HarnessRuntimePort,
        C: RuntimeCheckpointPort,
    {
        // 1. Check capability gate for cancellation
        self.check_capability("session.cancel")?;

        match &self.state {
            SupervisorState::Cancelling { .. } => Ok(CancelOutcome::AlreadyCancelling),
            SupervisorState::Prompting {
                turn_id,
                operation_id,
                ..
            }
            | SupervisorState::AwaitingPermission {
                turn_id,
                operation_id,
                ..
            } => {
                let active_turn = turn_id.clone();
                let active_op = operation_id.clone();

                let Some(session_id) = self.session_id.clone() else {
                    return Err(RuntimeError::SessionNotReady {
                        state: "no bound session".to_string(),
                    });
                };

                // Checkpoint cancel intent
                let intent = CheckpointIntent::Cancel {
                    thread_id: self.thread_id.clone(),
                    turn_id: Some(active_turn.clone()),
                    operation_id: Some(active_op.clone()),
                    timestamp: now,
                };
                checkpoint.checkpoint_intent(&intent)?;

                // Call harness adapter to notify cancellation
                harness.cancel_turn(&session_id)?;

                self.state = SupervisorState::Cancelling {
                    turn_id: Some(active_turn),
                    operation_id: Some(active_op),
                };
                Ok(CancelOutcome::CancelledActive)
            }
            _ => Ok(CancelOutcome::NoActiveTurn),
        }
    }

    /// Closes this supervisor session cleanly.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on harness or checkpoint failure.
    pub fn close_session<H, C>(
        &mut self,
        now: UnixMillis,
        harness: &mut H,
        checkpoint: &mut C,
    ) -> Result<(), RuntimeError>
    where
        H: HarnessRuntimePort,
        C: RuntimeCheckpointPort,
    {
        if let Some(session_id) = self.session_id.take() {
            let intent = CheckpointIntent::Close {
                thread_id: self.thread_id.clone(),
                timestamp: now,
            };
            let _ = checkpoint.checkpoint_intent(&intent);

            let _ = harness.close_session(&session_id);

            let settle = CheckpointSettled::SessionClosed {
                thread_id: self.thread_id.clone(),
                timestamp: now,
            };
            let _ = checkpoint.settle_checkpoint(&settle);
        }

        self.state = SupervisorState::Closed;
        Ok(())
    }

    /// Applies Desktop UI lifecycle events (reload, window close, desktop exit).
    ///
    /// Proves that desktop UI events NEVER interrupt running turns or close runtime sessions.
    pub fn on_desktop_lifecycle(&mut self, event: DesktopLifecycle) -> TurnTransition {
        if let SupervisorState::Prompting { turn_id, .. }
        | SupervisorState::AwaitingPermission { turn_id, .. }
        | SupervisorState::Cancelling {
            turn_id: Some(turn_id),
            ..
        } = &self.state
        {
            self.ownership.on_desktop_lifecycle(event, turn_id)
        } else {
            TurnTransition::StillRunning
        }
    }
}
