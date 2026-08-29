//! Turn ownership: who may stop a running turn (ADR 0006).
//!
//! `docs/ARCHITECTURE.md` requires that closing or reloading the UI must
//! not terminate active work, because Core — not Desktop — owns every
//! running turn. This module makes that rule executable: turns live in a
//! registry that reacts only to explicitly modeled lifecycle causes.
//! Desktop lifecycle events (`detach`, `reload`, `window_closed`) are
//! observable here and provably inert for turn state.

use std::collections::BTreeMap;

use altior_domain::{OperationId, TurnId};

/// Why a turn stopped, when it did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCause {
    /// An explicit cancel command named the turn's operation.
    Cancelled,
    /// Core is shutting down and drained its own work.
    CoreShutdown,
}

/// How a turn's lifecycle event was treated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnTransition {
    /// The turn is running; nothing changed.
    StillRunning,
    /// The turn stopped for this cause.
    Stopped(StopCause),
    /// No such turn exists (unknown or already stopped).
    UnknownTurn,
}

/// Lifecycle events Desktop can emit at its own processes. None of them
/// stop a turn; the enum exists so tests can prove it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopLifecycle {
    /// A window reloaded its UI.
    Reload,
    /// A window closed; Core keeps serving other windows.
    WindowClosed,
    /// The last Desktop process exited. Core keeps running detached
    /// (ADR 0006: Desktop does not own Core's lifetime).
    DesktopExited,
}

/// The registry of running turns, owned by Core.
#[derive(Debug, Default)]
pub struct TurnOwnership {
    running: BTreeMap<TurnId, OperationId>,
}

impl TurnOwnership {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a running turn under its operation.
    pub fn start(&mut self, turn: TurnId, operation: OperationId) {
        self.running.insert(turn, operation);
    }

    /// Number of running turns.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    /// Whether a specific turn is still running.
    #[must_use]
    pub fn is_running(&self, turn: &TurnId) -> bool {
        self.running.contains_key(turn)
    }

    /// Cancels the turn whose operation is `operation`, if it is running.
    #[must_use]
    pub fn cancel_by_operation(&mut self, operation: &OperationId) -> TurnTransition {
        let Some(turn) = self
            .running
            .iter()
            .find(|(_, op)| op == &operation)
            .map(|(turn, _)| turn.clone())
        else {
            return TurnTransition::UnknownTurn;
        };
        self.running.remove(&turn);
        TurnTransition::Stopped(StopCause::Cancelled)
    }

    /// Stops every running turn because Core itself is shutting down.
    pub fn drain_for_shutdown(&mut self) {
        self.running.clear();
    }

    /// Applies a Desktop lifecycle event. Desktop does not own turn
    /// lifetimes, so every outcome is [`TurnTransition::StillRunning`] (or
    /// `UnknownTurn` for a turn that is not running) — the signature and
    /// the test suite make the rule impossible to regress silently.
    #[must_use]
    pub fn on_desktop_lifecycle(
        &mut self,
        event: DesktopLifecycle,
        _turn: &TurnId,
    ) -> TurnTransition {
        // Deliberately inert for turn state: reload and close events never
        // reach turn ownership (ADR 0006). The parameter stays so callers
        // cannot "forget" which turn they expected to survive.
        match event {
            DesktopLifecycle::Reload
            | DesktopLifecycle::WindowClosed
            | DesktopLifecycle::DesktopExited => TurnTransition::StillRunning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn turn(number: u32) -> TurnId {
        TurnId::from_str(&format!("trn_fixture{number:09}")).unwrap()
    }

    fn operation(number: u32) -> OperationId {
        OperationId::from_str(&format!("op_fixture{number:09}")).unwrap()
    }

    #[test]
    fn ui_reload_and_window_close_never_stop_a_running_turn() {
        let mut ownership = TurnOwnership::new();
        let active = turn(1);
        ownership.start(active.clone(), operation(5));

        for event in [
            DesktopLifecycle::Reload,
            DesktopLifecycle::WindowClosed,
            DesktopLifecycle::DesktopExited,
        ] {
            assert_eq!(
                ownership.on_desktop_lifecycle(event, &active),
                TurnTransition::StillRunning
            );
            assert!(ownership.is_running(&active));
        }
        assert_eq!(ownership.running_count(), 1);
    }

    #[test]
    fn explicit_cancellation_by_operation_stops_exactly_one_turn() {
        let mut ownership = TurnOwnership::new();
        let first = turn(1);
        let second = turn(2);
        ownership.start(first.clone(), operation(5));
        ownership.start(second.clone(), operation(6));

        assert_eq!(
            ownership.cancel_by_operation(&operation(5)),
            TurnTransition::Stopped(StopCause::Cancelled)
        );
        assert!(!ownership.is_running(&first));
        assert!(ownership.is_running(&second));

        // Cancelling the same operation again finds nothing: idempotent.
        assert_eq!(
            ownership.cancel_by_operation(&operation(5)),
            TurnTransition::UnknownTurn
        );
    }

    #[test]
    fn core_shutdown_drains_but_desktop_exit_does_not() {
        let mut ownership = TurnOwnership::new();
        ownership.start(turn(1), operation(5));
        assert_eq!(
            ownership.on_desktop_lifecycle(DesktopLifecycle::DesktopExited, &turn(1)),
            TurnTransition::StillRunning
        );
        assert_eq!(ownership.running_count(), 1);

        ownership.drain_for_shutdown();
        assert_eq!(ownership.running_count(), 0);
    }
}
