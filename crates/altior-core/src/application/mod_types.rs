//! Application reporting and composite query DTOs (P1.3).

use std::collections::BTreeMap;

use altior_domain::{CoreInstanceId, RuntimeCheckpoint, ThreadId};
use altior_protocol::{ProductVersion, ProtocolVersionRange, RetainedWindow};
use altior_storage::{ThreadRow, TurnRow};

use crate::runtime::state::{HarnessSessionId, SupervisorState};

/// Result of opening a thread, combining projected storage state and live supervisor machine state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadOpenResult {
    /// Projected thread metadata from SQLite storage.
    pub thread: ThreadRow,
    /// Ordered turn history for this thread.
    pub turns: Vec<TurnRow>,
    /// Live supervisor machine state for this thread.
    pub supervisor_state: SupervisorState,
    /// Active harness session ID if currently bound.
    pub session_id: Option<HarnessSessionId>,
}

/// Overall status summary of the Core application and runtime subsystems.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreStatusReport {
    /// Epoch identifier of this Core launch.
    pub instance_id: CoreInstanceId,
    /// Built product version.
    pub core_version: ProductVersion,
    /// Negotiated protocol version range.
    pub protocol_versions: ProtocolVersionRange,
    /// Number of active thread supervisors currently tracked.
    pub active_thread_count: usize,
    /// Number of admitted operations currently active in dedup ledger.
    pub registered_operations: usize,
    /// Retained IPC sequence window available for subscription catch-up.
    pub retained_event_window: Option<RetainedWindow>,
    /// Number of indeterminate runtime checkpoints in storage.
    pub indeterminate_checkpoints: usize,
}

/// Diagnostics snapshot of active thread state machines and storage checkpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreDiagnosticsReport {
    /// Map of thread IDs to their current supervisor states.
    pub thread_states: BTreeMap<ThreadId, SupervisorState>,
    /// Total number of active (unsettled) checkpoints in storage.
    pub active_checkpoints: usize,
    /// List of indeterminate checkpoints requiring review (auto-resend forbidden).
    pub indeterminate_checkpoints: Vec<RuntimeCheckpoint>,
}

/// Report produced upon application restart recovery scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupRecoveryReport {
    /// Number of unsettled intent checkpoints transitioned to Indeterminate.
    pub recovered_unsettled_intents: usize,
    /// Total count of Indeterminate checkpoints found across store.
    pub indeterminate_checkpoints_count: usize,
    /// Total count of active (interrupted) turns found across store.
    pub active_turns_count: usize,
    /// Invariant assertion: automatic prompt resend is strictly forbidden.
    pub resend_prohibited: bool,
}
