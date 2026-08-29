//! The agent process lifecycle decision machine (ADR 0007).
//!
//! Pure, in the style of the P0.2 supervisor: inputs are explicit events
//! (cancel requested, permission arrived, prompt answered, process
//! exited, stream ended, idle budget elapsed); outputs are [`HostAction`]s
//! the host executes against the real child process. No timers — the
//! idle budget is host policy reported as an event — so tests drive the
//! machine directly and every path is deterministic.
//!
//! The cleanup contract (ADR 0007): cancelling answers every pending
//! `session/request_permission` with `{"outcome":"cancelled"}`, waits for
//! the prompt response, and only then kills the child. A crash or an
//! idle timeout skips the handshake and goes straight to kill-and-reap,
//! with the outstanding delivery classified by the caller's
//! [`PromptDelivery`](crate::PromptDelivery).

use crate::error::AcpError;
use crate::wire::RpcId;

/// One phase of the agent process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentPhase {
    /// Spawned; no turn running.
    Ready,
    /// A prompt is outstanding; updates and permission requests may
    /// arrive.
    Prompting {
        /// Permission requests awaiting an answer, in arrival order.
        pending_permissions: Vec<RpcId>,
    },
    /// `session/cancel` was sent; waiting for the prompt response while
    /// answering late permissions as cancelled.
    Cancelling {
        /// Permissions that arrived before or during cancellation; each
        /// is answered `cancelled`.
        pending_permissions: Vec<RpcId>,
    },
    /// The child must die; the host kills and reaps it.
    Killing,
    /// The child is gone and reaped.
    Dead,
}

impl AgentPhase {
    fn pending_permissions(&self) -> &[RpcId] {
        match self {
            Self::Prompting {
                pending_permissions,
            }
            | Self::Cancelling {
                pending_permissions,
            } => pending_permissions,
            Self::Ready | Self::Killing | Self::Dead => &[],
        }
    }
}

/// One instruction for the host, in the order the producing event
/// requires.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostAction {
    /// Nothing to do.
    Continue,
    /// Send the `session/cancel` notification for the current session.
    SendCancelNotification,
    /// Answer a pending permission request with
    /// `{"outcome":"cancelled"}` (the v1 cancellation contract).
    AnswerPermissionCancelled {
        /// The agent's request id to answer.
        id: RpcId,
    },
    /// The turn response arrived; the agent stays alive and reusable.
    TurnSettled,
    /// Kill the child, wait for exit, and release its resources.
    KillAndReap,
}

/// The lifecycle state machine for one agent child process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLifecycle {
    phase: AgentPhase,
}

impl AgentLifecycle {
    /// A freshly spawned agent with no turn running.
    #[must_use]
    pub const fn spawned() -> Self {
        Self {
            phase: AgentPhase::Ready,
        }
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> &AgentPhase {
        &self.phase
    }

    /// Reports that a prompt request was written; the turn is now
    /// outstanding.
    pub fn on_prompt_written(&mut self) {
        if matches!(self.phase, AgentPhase::Ready) {
            self.phase = AgentPhase::Prompting {
                pending_permissions: Vec::new(),
            };
        }
    }

    /// Reports that the agent asked for a permission decision.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no turn is running.
    pub fn on_permission_requested(&mut self, id: RpcId) -> Result<Vec<HostAction>, AcpError> {
        match &mut self.phase {
            AgentPhase::Prompting {
                pending_permissions,
            } => {
                pending_permissions.push(id);
                Ok(vec![HostAction::Continue])
            }
            // A permission racing a cancel is answered cancelled
            // immediately per the v1 contract.
            AgentPhase::Cancelling { .. } => Ok(vec![HostAction::AnswerPermissionCancelled { id }]),
            AgentPhase::Ready | AgentPhase::Killing | AgentPhase::Dead => {
                Err(AcpError::OutOfOrder {
                    attempted: "receive a permission request with no turn running",
                })
            }
        }
    }

    /// The user cancelled the running turn. Sends the cancel
    /// notification, answers every already-pending permission as
    /// cancelled, and waits for the prompt response before any kill.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no turn is running.
    pub fn on_cancel_requested(&mut self) -> Result<Vec<HostAction>, AcpError> {
        let AgentPhase::Prompting {
            pending_permissions,
        } = &self.phase
        else {
            return Err(AcpError::OutOfOrder {
                attempted: "cancel a turn with no turn running",
            });
        };
        let pending = pending_permissions.clone();
        let mut actions = vec![HostAction::SendCancelNotification];
        actions.extend(
            pending
                .iter()
                .map(|id| HostAction::AnswerPermissionCancelled { id: id.clone() }),
        );
        self.phase = AgentPhase::Cancelling {
            pending_permissions: Vec::new(),
        };
        Ok(actions)
    }

    /// Reports that the prompt request was answered (response or error).
    /// A cancelled turn settles back to `Ready`; the agent survives.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no turn is outstanding.
    pub fn on_prompt_settled(&mut self) -> Result<Vec<HostAction>, AcpError> {
        match self.phase {
            AgentPhase::Prompting { .. } | AgentPhase::Cancelling { .. } => {
                self.phase = AgentPhase::Ready;
                Ok(vec![HostAction::TurnSettled])
            }
            AgentPhase::Ready | AgentPhase::Killing | AgentPhase::Dead => {
                Err(AcpError::OutOfOrder {
                    attempted: "settle a prompt with no turn outstanding",
                })
            }
        }
    }

    /// Reports that the child exited or its stream ended. Every plan
    /// collapses to reap; outstanding deliveries are classified by the
    /// caller.
    pub fn on_process_lost(&mut self) -> Vec<HostAction> {
        self.phase = AgentPhase::Dead;
        vec![HostAction::KillAndReap]
    }

    /// Reports that the idle budget elapsed with no activity. The child
    /// is untrusted: kill and reap without waiting for a response.
    pub fn on_idle_elapsed(&mut self) -> Vec<HostAction> {
        self.phase = AgentPhase::Killing;
        vec![HostAction::KillAndReap]
    }

    /// The host finished killing and reaping.
    pub fn on_reaped(&mut self) {
        if matches!(self.phase, AgentPhase::Killing) {
            self.phase = AgentPhase::Dead;
        }
    }

    /// Whether every pending permission has been answered (used by tests
    /// and the smoke host's cleanup assertion).
    #[must_use]
    pub fn permissions_unanswered(&self) -> usize {
        self.phase.pending_permissions().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_answers_permissions_then_waits_then_settles() {
        let mut lifecycle = AgentLifecycle::spawned();
        lifecycle.on_prompt_written();
        assert_eq!(
            lifecycle.on_permission_requested(RpcId::Number(7)).unwrap(),
            vec![HostAction::Continue]
        );
        assert_eq!(lifecycle.permissions_unanswered(), 1);

        let actions = lifecycle.on_cancel_requested().unwrap();
        assert_eq!(
            actions,
            vec![
                HostAction::SendCancelNotification,
                HostAction::AnswerPermissionCancelled {
                    id: RpcId::Number(7)
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
                .on_permission_requested(RpcId::Text("late".to_owned()))
                .unwrap(),
            vec![HostAction::AnswerPermissionCancelled {
                id: RpcId::Text("late".to_owned())
            }]
        );
        // The cancelled prompt response arrives; the agent survives.
        assert_eq!(
            lifecycle.on_prompt_settled().unwrap(),
            vec![HostAction::TurnSettled]
        );
        assert_eq!(lifecycle.phase(), &AgentPhase::Ready);
    }

    #[test]
    fn crashes_collapse_to_reap_regardless_of_phase() {
        for setup in [false, true] {
            let mut lifecycle = AgentLifecycle::spawned();
            if setup {
                lifecycle.on_prompt_written();
                lifecycle.on_permission_requested(RpcId::Number(1)).unwrap();
            }
            assert_eq!(lifecycle.on_process_lost(), vec![HostAction::KillAndReap]);
            assert_eq!(lifecycle.phase(), &AgentPhase::Dead);
        }
    }

    #[test]
    fn idle_timeouts_kill_without_waiting_for_a_response() {
        let mut lifecycle = AgentLifecycle::spawned();
        lifecycle.on_prompt_written();
        assert_eq!(lifecycle.on_idle_elapsed(), vec![HostAction::KillAndReap]);
        assert_eq!(lifecycle.phase(), &AgentPhase::Killing);
        lifecycle.on_reaped();
        assert_eq!(lifecycle.phase(), &AgentPhase::Dead);
    }

    #[test]
    fn lifecycle_reports_are_order_sensitive() {
        let mut lifecycle = AgentLifecycle::spawned();
        assert!(matches!(
            lifecycle.on_cancel_requested(),
            Err(AcpError::OutOfOrder { .. })
        ));
        assert!(matches!(
            lifecycle.on_prompt_settled(),
            Err(AcpError::OutOfOrder { .. })
        ));
        lifecycle.on_prompt_written();
        lifecycle.on_prompt_settled().unwrap();
        assert!(matches!(
            lifecycle.on_prompt_settled(),
            Err(AcpError::OutOfOrder { .. })
        ));
    }
}
