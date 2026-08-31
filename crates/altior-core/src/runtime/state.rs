//! Core-owned runtime state models, intents, settlements, events, and errors (P1.2).
//!
//! Domain entities remain pure; these types capture runtime execution,
//! intent checkpoints, boundary settlement, and supervisor state machines.

use std::fmt;

use altior_domain::{
    DeliveryState, EventId, EventPayload, OperationId, PermissionDecision, PermissionDescription,
    PermissionKind, ThreadId, TurnId, TurnState, UnixMillis,
};
use altior_protocol::{CapabilityId, CapabilitySet};

use super::diagnostics::BoundedDiagnosticsSummary;
use crate::operations::AdmissionError;

/// Maximum length of a harness session identifier in bytes.
pub const MAX_SESSION_ID_LEN: usize = 256;

/// A validated, bounded harness session identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HarnessSessionId(String);

impl HarnessSessionId {
    /// Creates a session ID after validating bounds and characters.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::InvalidSessionId`] if empty, too long, or
    /// contains ASCII control characters.
    pub fn new(id: &str) -> Result<Self, RuntimeError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(RuntimeError::InvalidSessionId(
                "session id is empty".to_string(),
            ));
        }
        if trimmed.len() > MAX_SESSION_ID_LEN {
            return Err(RuntimeError::InvalidSessionId(format!(
                "session id exceeds {MAX_SESSION_ID_LEN} bytes"
            )));
        }
        if trimmed.chars().any(|c| c.is_ascii_control()) {
            return Err(RuntimeError::InvalidSessionId(
                "session id contains control characters".to_string(),
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Returns the session ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HarnessSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Outcome of probing or testing a harness binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingProbeOutcome {
    /// Whether the binding is alive and functional.
    pub ok: bool,
    /// Capabilities declared/negotiated with the agent harness.
    pub capabilities: CapabilitySet,
    /// Optional redacted diagnostics summary if probe failed or produced notices.
    pub diagnostics: Option<BoundedDiagnosticsSummary>,
}

/// Metadata returned upon creating or resuming a session with a harness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessSessionInfo {
    /// The session identifier issued by or bound in the harness.
    pub session_id: HarnessSessionId,
    /// Capabilities supported for this session.
    pub capabilities: CapabilitySet,
}

/// Request to initiate a prompt turn on a harness session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessPromptRequest {
    /// The turn identifier.
    pub turn_id: TurnId,
    /// The operation identifier for dedup correlation.
    pub operation_id: OperationId,
    /// The prompt text.
    pub prompt: String,
}

/// Events produced by an external harness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessEvent {
    /// The turn began processing on the agent harness.
    Started { turn_id: TurnId },
    /// Streaming message text delta.
    MessageDelta { text: String },
    /// Agent requested a permission approval.
    PermissionRequest {
        event_id: EventId,
        kind: PermissionKind,
        description: PermissionDescription,
    },
    /// The turn completed successfully.
    Completed { payload: Option<EventPayload> },
    /// The turn failed with an error and delivery classification.
    Failed {
        error: String,
        delivery: DeliveryState,
    },
    /// The turn was cancelled.
    Cancelled,
    /// The harness process exited.
    ProcessExited { exit_code: Option<i32> },
    /// An unmapped or unknown raw event from the harness (does not panic).
    RawUnknown { name: String, data: String },
}

/// Durable checkpoint intent recorded BEFORE calling an external harness adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointIntent {
    /// Intending to send a prompt to the harness.
    Prompt {
        thread_id: ThreadId,
        turn_id: TurnId,
        operation_id: OperationId,
        timestamp: UnixMillis,
    },
    /// Intending to submit a permission decision.
    PermissionDecision {
        thread_id: ThreadId,
        turn_id: TurnId,
        permission_id: EventId,
        decision: PermissionDecision,
        timestamp: UnixMillis,
    },
    /// Intending to cancel a turn or session.
    Cancel {
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        operation_id: Option<OperationId>,
        timestamp: UnixMillis,
    },
    /// Intending to close a session.
    Close {
        thread_id: ThreadId,
        timestamp: UnixMillis,
    },
}

/// Durable checkpoint settled record written AFTER an external adapter call returns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointSettled {
    /// Prompt delivery classification settled.
    PromptDelivered {
        thread_id: ThreadId,
        turn_id: TurnId,
        delivery: DeliveryState,
        timestamp: UnixMillis,
    },
    /// Permission decision submission settled.
    PermissionSettled {
        thread_id: ThreadId,
        turn_id: TurnId,
        permission_id: EventId,
        decision: PermissionDecision,
        timestamp: UnixMillis,
    },
    /// Turn reached a terminal lifecycle state.
    TurnTerminal {
        thread_id: ThreadId,
        turn_id: TurnId,
        state: TurnState,
        delivery: DeliveryState,
        timestamp: UnixMillis,
    },
    /// Session closed settled.
    SessionClosed {
        thread_id: ThreadId,
        timestamp: UnixMillis,
    },
}

/// State of a thread's runtime supervisor machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorState {
    /// No session initialized.
    Idle,
    /// Session is starting / attaching.
    Starting,
    /// Session is ready to accept turns.
    Ready,
    /// A prompt turn is actively in progress.
    Prompting {
        turn_id: TurnId,
        operation_id: OperationId,
        delivery: DeliveryState,
    },
    /// Turn is paused awaiting a user permission decision.
    AwaitingPermission {
        turn_id: TurnId,
        operation_id: OperationId,
        pending_permission_id: EventId,
    },
    /// Cancellation was requested and is in flight.
    Cancelling {
        turn_id: Option<TurnId>,
        operation_id: Option<OperationId>,
    },
    /// Session has been cleanly closed.
    Closed,
    /// Session crashed or experienced unexpected process exit.
    Crashed {
        reason: String,
        delivery: DeliveryState,
    },
}

/// How an incoming turn admission request was treated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnAdmission {
    /// New turn admitted and scheduled.
    Admitted,
    /// Operation already admitted or finished; acknowledged without re-execution.
    Duplicate,
}

/// Outcome of a cancel request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    /// Successfully initiated cancellation on active turn.
    CancelledActive,
    /// Cancellation is already in flight.
    AlreadyCancelling,
    /// No active turn was running to cancel.
    NoActiveTurn,
}

/// Core runtime event emitted to listeners / UI layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    /// Turn started.
    TurnStarted {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    /// Streaming text chunk.
    MessageDelta {
        thread_id: ThreadId,
        turn_id: TurnId,
        text: String,
    },
    /// Permission requested by agent.
    PermissionRequested {
        thread_id: ThreadId,
        turn_id: TurnId,
        permission_id: EventId,
        kind: PermissionKind,
        description: PermissionDescription,
    },
    /// Turn completed.
    TurnCompleted {
        thread_id: ThreadId,
        turn_id: TurnId,
        payload: Option<EventPayload>,
    },
    /// Turn failed.
    TurnFailed {
        thread_id: ThreadId,
        turn_id: TurnId,
        reason: String,
        delivery: DeliveryState,
    },
    /// Turn was cancelled.
    TurnCancelled {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    /// Process exited.
    ProcessExited {
        thread_id: ThreadId,
        exit_code: Option<i32>,
    },
    /// Unknown event (bounded and redacted; does not panic).
    Unknown {
        thread_id: ThreadId,
        name: String,
        summary: BoundedDiagnosticsSummary,
    },
}

/// Typed errors from external harness operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessError {
    /// Failed to spawn child process or connect transport.
    SpawnFailed(String),
    /// Process died unexpectedly with exit code and diagnostics.
    ProcessDied {
        exit_code: Option<i32>,
        diagnostics: Option<BoundedDiagnosticsSummary>,
    },
    /// Transport / I/O level error.
    Transport(String),
    /// Session was not found in harness.
    SessionNotFound(HarnessSessionId),
    /// Protocol negotiation or RPC error.
    Protocol(String),
    /// Other harness error.
    Other(String),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed(msg) => write!(f, "harness spawn failed: {msg}"),
            Self::ProcessDied {
                exit_code,
                diagnostics,
            } => {
                write!(
                    f,
                    "harness process died (exit code: {exit_code:?}, diag: {diagnostics:?})"
                )
            }
            Self::Transport(msg) => write!(f, "harness transport error: {msg}"),
            Self::SessionNotFound(id) => write!(f, "harness session not found: {id}"),
            Self::Protocol(msg) => write!(f, "harness protocol error: {msg}"),
            Self::Other(msg) => write!(f, "harness error: {msg}"),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Typed errors from checkpoint persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointError {
    /// Persistence layer write error.
    Persistence(String),
    /// Concurrency or version conflict.
    Conflict(String),
    /// Other checkpoint error.
    Other(String),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persistence(msg) => write!(f, "checkpoint persistence error: {msg}"),
            Self::Conflict(msg) => write!(f, "checkpoint conflict: {msg}"),
            Self::Other(msg) => write!(f, "checkpoint error: {msg}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

/// Typed errors returned by the runtime use-case and supervisor layers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    /// Operation admission rejected by registry (e.g. capacity full).
    OperationAdmitFailed(AdmissionError),
    /// Duplicate operation rejected.
    DuplicateOperation(OperationId),
    /// Supervisor is in an invalid state for requested operation.
    SessionNotReady { state: String },
    /// A turn is already actively running on this thread (bounded active operations).
    ActiveOperationInProgress {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    /// No active turn is running on this thread.
    NoActiveTurn { thread_id: ThreadId },
    /// Automatic prompt resend is forbidden after indeterminate/confirmed delivery.
    AutomaticResendForbidden {
        thread_id: ThreadId,
        turn_id: TurnId,
        delivery: DeliveryState,
    },
    /// The requested operation requires an unsupported capability.
    UnsupportedCapability(CapabilityId),
    /// Permission request was not found or mismatched.
    PermissionNotFound {
        thread_id: ThreadId,
        permission_id: EventId,
    },
    /// Underlying harness failure.
    Harness(HarnessError),
    /// Checkpoint persistence failure.
    Checkpoint(CheckpointError),
    /// Thread runtime supervisor was not found.
    UnknownThread(ThreadId),
    /// Session is already active for this thread.
    SessionAlreadyActive(ThreadId),
    /// Invalid session id format.
    InvalidSessionId(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperationAdmitFailed(err) => write!(f, "operation admission failed: {err:?}"),
            Self::DuplicateOperation(op) => write!(f, "duplicate operation: {op}"),
            Self::SessionNotReady { state } => write!(f, "session is not ready: {state}"),
            Self::ActiveOperationInProgress { thread_id, turn_id } => {
                write!(
                    f,
                    "active turn {turn_id} already in progress for thread {thread_id}"
                )
            }
            Self::NoActiveTurn { thread_id } => write!(f, "no active turn for thread {thread_id}"),
            Self::AutomaticResendForbidden {
                thread_id,
                turn_id,
                delivery,
            } => {
                write!(
                    f,
                    "automatic resend of turn {turn_id} on thread {thread_id} is forbidden with delivery state {delivery:?}"
                )
            }
            Self::UnsupportedCapability(cap) => write!(f, "unsupported capability: {cap}"),
            Self::PermissionNotFound {
                thread_id,
                permission_id,
            } => {
                write!(
                    f,
                    "permission {permission_id} not found on thread {thread_id}"
                )
            }
            Self::Harness(err) => write!(f, "harness runtime error: {err}"),
            Self::Checkpoint(err) => write!(f, "checkpoint error: {err}"),
            Self::UnknownThread(id) => write!(f, "unknown thread: {id}"),
            Self::SessionAlreadyActive(id) => write!(f, "session already active for thread: {id}"),
            Self::InvalidSessionId(msg) => write!(f, "invalid session id: {msg}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<HarnessError> for RuntimeError {
    fn from(err: HarnessError) -> Self {
        Self::Harness(err)
    }
}

impl From<CheckpointError> for RuntimeError {
    fn from(err: CheckpointError) -> Self {
        Self::Checkpoint(err)
    }
}
