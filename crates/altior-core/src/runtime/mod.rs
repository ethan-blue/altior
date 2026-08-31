//! Agent execution runtime, supervision, and use-case ports (P1.2).
//!
//! Subprocess management is delegated to [`HarnessRuntimePort`], and durable
//! intent/settlement persistence to [`RuntimeCheckpointPort`].
//! This module provides pure supervision, bounded concurrency, correlation,
//! boundary discipline, secret-redacted diagnostics, and resilience to desktop detachment.

pub mod adapters;
pub mod coordinator;
pub mod diagnostics;
pub mod ports;
pub mod state;
pub mod supervisor;

pub use adapters::{AcpHarnessAdapter, StoreCheckpointAdapter};
pub use coordinator::AgentRuntimeSupervisor;
pub use diagnostics::{BoundedDiagnosticsSummary, MAX_DIAGNOSTICS_BYTES};
pub use ports::{AgentRuntime, HarnessRuntimePort, RuntimeCheckpointPort};
pub use state::{
    BindingProbeOutcome, CancelOutcome, CheckpointError, CheckpointIntent, CheckpointSettled,
    HarnessError, HarnessEvent, HarnessPromptRequest, HarnessSessionId, HarnessSessionInfo,
    MAX_SESSION_ID_LEN, RuntimeError, RuntimeEvent, SupervisorState, TurnAdmission,
};
pub use supervisor::{ThreadRuntimeSupervisor, TurnExecutionRecord};
