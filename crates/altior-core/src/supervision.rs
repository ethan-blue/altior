//! Desktop's supervision state machine for Core (ADR 0006).
//!
//! Desktop does not own Core's lifetime; it spawns-or-attaches. The machine
//! is pure: every input is an explicit event, every output an explicit
//! decision for the host to execute. There are no timers here — backoff and
//! retry pacing are host policy, recorded as data
//! ([`ReconnectPolicy`]), so tests drive the machine directly and nothing
//! depends on scheduler luck.

use altior_ipc::{Endpoint, IpcError};

/// What supervision should do next; the host executes it and feeds the
/// outcome back as the next event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    /// Probe the derived endpoint: is a live Core listening?
    Probe { endpoint: Endpoint },
    /// No live Core: launch one (detached) against this endpoint.
    Spawn { endpoint: Endpoint },
    /// Attach: connect, authenticate, negotiate, greet.
    Attach { endpoint: Endpoint },
    /// The connection dropped; wait per policy, then probe again.
    BackoffThenProbe { endpoint: Endpoint, attempt: u32 },
    /// Core is healthy and attached; nothing to do.
    Idle,
}

/// How a probe or connection attempt ended. Supervision needs only the
/// classification; the transport's typed error detail is logged by the
/// host, not carried through the state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    /// Something answered and the token file names a live launch.
    Live,
    /// Nothing answered, or the token file belongs to a dead launch.
    Unavailable,
    /// The endpoint holds a different Core launch than expected.
    Stale,
}

/// Bounded reconnect policy as data (ADR 0006: no hidden timers).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectPolicy {
    /// Backoff starts at this attempt number.
    pub first_attempt: u32,
    /// Attempts at or beyond this number stop escalating; the supervisor
    /// keeps waiting between probes instead of growing without bound.
    pub max_attempts: u32,
}

impl ReconnectPolicy {
    /// A policy with the given bounds.
    #[must_use]
    pub const fn new(first_attempt: u32, max_attempts: u32) -> Self {
        Self {
            first_attempt,
            max_attempts,
        }
    }

    /// The attempt number to use for the next probe after a drop, given the
    /// previous attempt. Escalating attempts are the host's pacing input;
    /// the policy never blocks by itself.
    #[must_use]
    pub const fn next_attempt(&self, previous: u32) -> u32 {
        if previous >= self.max_attempts {
            self.max_attempts
        } else {
            previous + 1
        }
    }
}

/// Supervision state for one Core endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisionState {
    /// No probe has run yet.
    Initial,
    /// A probe is outstanding.
    Probing,
    /// A spawn was issued; waiting for the endpoint to come up.
    Starting { attempt: u32 },
    /// Attached and healthy.
    Attached,
    /// Connection lost; reconnecting with bounded escalation.
    Reconnecting { attempt: u32 },
    /// The user stopped the whole application with no active work.
    Stopped,
}

/// The supervisor state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Supervisor {
    endpoint: Endpoint,
    policy: ReconnectPolicy,
    state: SupervisionState,
    /// The last reconnect attempt number issued in this supervisor's
    /// lifetime. Escalation deliberately survives a short-lived re-attach:
    /// a Core that flaps should not restart its backoff on every brief
    /// success. The machine has no clock, so there is no "healthy for long
    /// enough" decay — hosts that want one reconstruct the supervisor.
    reconnect_attempt: u32,
}

impl Supervisor {
    /// Creates a supervisor for `endpoint`.
    #[must_use]
    pub fn new(endpoint: Endpoint, policy: ReconnectPolicy) -> Self {
        Self {
            endpoint,
            policy,
            state: SupervisionState::Initial,
            reconnect_attempt: 0,
        }
    }

    /// The endpoint under supervision.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> &SupervisionState {
        &self.state
    }

    /// The first decision after construction.
    #[must_use]
    pub fn start(&mut self) -> Decision {
        self.state = SupervisionState::Probing;
        Decision::Probe {
            endpoint: self.endpoint.clone(),
        }
    }

    /// Feeds a probe outcome; returns the next decision.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] when no probe is outstanding.
    pub fn on_probe(&mut self, outcome: &ProbeOutcome) -> Result<Decision, IpcError> {
        if self.state != SupervisionState::Probing {
            return Err(IpcError::SessionOrder {
                attempted: "report a probe outcome without an outstanding probe",
            });
        }
        match outcome {
            ProbeOutcome::Live => {
                self.state = SupervisionState::Attached;
                Ok(Decision::Attach {
                    endpoint: self.endpoint.clone(),
                })
            }
            ProbeOutcome::Unavailable => {
                self.state = SupervisionState::Starting {
                    attempt: self.current_attempt(),
                };
                Ok(Decision::Spawn {
                    endpoint: self.endpoint.clone(),
                })
            }
            ProbeOutcome::Stale => {
                // A stale endpoint holds a dead launch: spawn a fresh Core
                // exactly like an empty endpoint would.
                self.state = SupervisionState::Starting {
                    attempt: self.current_attempt(),
                };
                Ok(Decision::Spawn {
                    endpoint: self.endpoint.clone(),
                })
            }
        }
    }

    /// Reports that the spawned Core is now answering probes.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] when no spawn is outstanding.
    pub fn on_spawned(&mut self) -> Result<Decision, IpcError> {
        let SupervisionState::Starting { .. } = self.state else {
            return Err(IpcError::SessionOrder {
                attempted: "report a spawned Core without an outstanding spawn",
            });
        };
        self.state = SupervisionState::Attached;
        Ok(Decision::Attach {
            endpoint: self.endpoint.clone(),
        })
    }

    /// The attempt number of the reconnect cycle in flight: the last one
    /// issued (floored at the policy's first attempt, so a spawn on a fresh
    /// endpoint also carries a well-defined attempt).
    fn current_attempt(&self) -> u32 {
        self.reconnect_attempt.max(self.policy.first_attempt)
    }

    /// The attempt number for the next reconnect, escalating per policy
    /// from the last one issued.
    fn next_attempt(&self) -> u32 {
        self.policy
            .next_attempt(self.reconnect_attempt)
            .max(self.policy.first_attempt)
    }

    /// Reports that the connection dropped (or a health check failed).
    /// Attaching again always starts with a probe, because the dead
    /// endpoint must not be trusted blindly.
    #[must_use]
    pub fn on_disconnected(&mut self) -> Decision {
        let attempt = self.next_attempt();
        self.reconnect_attempt = attempt;
        self.state = SupervisionState::Reconnecting { attempt };
        Decision::BackoffThenProbe {
            endpoint: self.endpoint.clone(),
            attempt,
        }
    }

    /// Issues the probe that follows a backoff wait. The host decides how
    /// long to wait; this only advances the machine.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] outside a reconnecting state.
    pub fn retry_probe(&mut self) -> Result<Decision, IpcError> {
        if !matches!(self.state, SupervisionState::Reconnecting { .. }) {
            return Err(IpcError::SessionOrder {
                attempted: "retry a probe outside a reconnecting state",
            });
        }
        self.state = SupervisionState::Probing;
        Ok(Decision::Probe {
            endpoint: self.endpoint.clone(),
        })
    }

    /// The user stopped the whole application; supervision stands down.
    /// Core itself keeps running unless an explicit stop was negotiated
    /// (ADR 0006: Desktop does not own Core's lifetime).
    pub fn stop(&mut self) {
        self.state = SupervisionState::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_supervisor() -> Supervisor {
        Supervisor::new(
            Endpoint::WindowsPipe(r"\\.\pipe\altior-core-ethan".to_owned()),
            ReconnectPolicy::new(1, 5),
        )
    }

    #[test]
    fn probes_first_then_attaches_to_a_live_core() {
        let mut supervisor = fresh_supervisor();
        assert!(matches!(supervisor.start(), Decision::Probe { .. }));
        let decision = supervisor.on_probe(&ProbeOutcome::Live).unwrap();
        assert!(matches!(decision, Decision::Attach { .. }));
        assert_eq!(supervisor.state(), &SupervisionState::Attached);
    }

    #[test]
    fn unavailable_and_stale_endpoints_spawn_a_fresh_core() {
        let mut supervisor = fresh_supervisor();
        assert!(matches!(supervisor.start(), Decision::Probe { .. }));
        assert!(matches!(
            supervisor.on_probe(&ProbeOutcome::Unavailable).unwrap(),
            Decision::Spawn { .. }
        ));
        assert!(matches!(
            supervisor.on_spawned().unwrap(),
            Decision::Attach { .. }
        ));

        let mut stale_case = fresh_supervisor();
        assert!(matches!(stale_case.start(), Decision::Probe { .. }));
        assert!(matches!(
            stale_case.on_probe(&ProbeOutcome::Stale).unwrap(),
            Decision::Spawn { .. }
        ));
    }

    #[test]
    fn reconnects_with_bounded_escalation_and_always_reprobes() {
        let mut supervisor = Supervisor::new(
            Endpoint::WindowsPipe(r"\\.\pipe\altior-core-ethan".to_owned()),
            ReconnectPolicy::new(1, 3),
        );
        assert!(matches!(supervisor.start(), Decision::Probe { .. }));
        supervisor.on_probe(&ProbeOutcome::Live).unwrap();

        // Drop 1: back off at attempt 1, reprobe, find Core gone, spawn,
        // attach — every recovery path starts with a probe.
        let Decision::BackoffThenProbe { attempt: 1, .. } = supervisor.on_disconnected() else {
            panic!("first reconnect starts at attempt 1");
        };
        assert!(matches!(
            supervisor.retry_probe().unwrap(),
            Decision::Probe { .. }
        ));
        assert!(matches!(
            supervisor.on_probe(&ProbeOutcome::Unavailable).unwrap(),
            Decision::Spawn { .. }
        ));
        assert!(matches!(
            supervisor.on_spawned().unwrap(),
            Decision::Attach { .. }
        ));

        // Drop 2 escalates to attempt 2.
        let Decision::BackoffThenProbe { attempt: 2, .. } = supervisor.on_disconnected() else {
            panic!("second reconnect escalates to attempt 2");
        };
        supervisor.retry_probe().unwrap();
        supervisor.on_probe(&ProbeOutcome::Live).unwrap();

        // Attempts cap at the policy bound.
        let Decision::BackoffThenProbe { attempt: 3, .. } = supervisor.on_disconnected() else {
            panic!("third reconnect reaches the bound");
        };
        supervisor.retry_probe().unwrap();
        supervisor.on_probe(&ProbeOutcome::Live).unwrap();
        let Decision::BackoffThenProbe { attempt: 3, .. } = supervisor.on_disconnected() else {
            panic!("attempts never exceed the bound");
        };
    }

    #[test]
    fn probe_reports_are_order_sensitive() {
        let mut supervisor = fresh_supervisor();
        // No probe outstanding yet.
        assert!(matches!(
            supervisor.on_probe(&ProbeOutcome::Live),
            Err(IpcError::SessionOrder { .. })
        ));
        // Spawning without a spawn decision is equally invalid.
        assert!(matches!(supervisor.start(), Decision::Probe { .. }));
        assert!(matches!(
            supervisor.on_spawned(),
            Err(IpcError::SessionOrder { .. })
        ));
    }

    #[test]
    fn stopping_stands_down_without_killing_core() {
        let mut supervisor = fresh_supervisor();
        assert!(matches!(supervisor.start(), Decision::Probe { .. }));
        supervisor.on_probe(&ProbeOutcome::Live).unwrap();
        supervisor.stop();
        assert_eq!(supervisor.state(), &SupervisionState::Stopped);
    }

    #[test]
    fn reconnect_attempts_escalate_to_a_bound() {
        let policy = ReconnectPolicy::new(1, 3);
        assert_eq!(policy.next_attempt(1), 2);
        assert_eq!(policy.next_attempt(2), 3);
        // At the bound, attempts stop growing.
        assert_eq!(policy.next_attempt(3), 3);
        assert_eq!(policy.next_attempt(9), 3);
    }
}
