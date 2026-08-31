//! Typed application and command dispatcher errors (P1.3).
//!
//! Captures use-case, storage, protocol, and runtime failure modes
//! into deterministic, typed errors without collapsing causal detail.

use std::fmt;

use altior_domain::{
    AgentProfileId, DeliveryState, EntityError, EventId, HarnessBindingId, IdParseError,
    OperationId, ThreadId, TurnId,
};
use altior_ipc::IpcError;
use altior_protocol::{CapabilityId, ProtocolError};
use altior_storage::StorageError;

use crate::operations::AdmissionError;
use crate::runtime::{CheckpointError, HarnessError, RuntimeError};

/// Typed error returned by `CoreApplication` and the command dispatcher.
#[derive(Debug)]
pub enum CoreAppError {
    /// The specified thread was not found in storage or runtime.
    ThreadNotFound(ThreadId),
    /// The specified agent profile was not found.
    AgentProfileNotFound(AgentProfileId),
    /// The specified harness binding was not found.
    HarnessBindingNotFound(HarnessBindingId),
    /// The operation was already admitted or finished.
    DuplicateOperation(OperationId),
    /// Operation registry admission error (e.g. capacity full).
    Admission(AdmissionError),
    /// Runtime layer failure.
    Runtime(RuntimeError),
    /// Storage layer failure.
    Storage(StorageError),
    /// Domain validation failure.
    Entity(EntityError),
    /// IPC session or framing failure.
    Ipc(IpcError),
    /// Protocol encoding or negotiation failure.
    Protocol(ProtocolError),
    /// External agent harness error.
    Harness(HarnessError),
    /// Runtime checkpoint persistence error.
    Checkpoint(CheckpointError),
    /// An active turn is already in progress on this thread.
    ActiveTurnInProgress {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    /// No active turn is running to cancel or steer.
    NoActiveTurn(ThreadId),
    /// Automatic prompt resend is forbidden after indeterminate or confirmed delivery.
    AutomaticResendForbidden {
        thread_id: ThreadId,
        turn_id: TurnId,
        delivery: DeliveryState,
    },
    /// Permission decision request not found on this thread.
    PermissionNotFound {
        thread_id: ThreadId,
        permission_id: EventId,
    },
    /// Operation requested an unsupported capability.
    UnsupportedCapability(CapabilityId),
    /// Thread session is not in a ready state.
    SessionNotReady(String),
    /// Session is already active for this thread.
    SessionAlreadyActive(ThreadId),
    /// Authentication token mismatch or rejection.
    AuthenticationRejected,
    /// Synchronization lock poisoned or unavailable.
    LockPoisoned(&'static str),
    /// Domain identifier parsing or validation failure.
    Id(IdParseError),
    /// Invalid request parameters or malformed input.
    InvalidInput(String),
    /// General application error.
    Other(String),
}

impl fmt::Display for CoreAppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ThreadNotFound(id) => write!(f, "thread not found: {id}"),
            Self::AgentProfileNotFound(id) => write!(f, "agent profile not found: {id}"),
            Self::HarnessBindingNotFound(id) => write!(f, "harness binding not found: {id}"),
            Self::DuplicateOperation(op) => write!(f, "duplicate operation: {op}"),
            Self::Admission(err) => write!(f, "operation admission error: {err:?}"),
            Self::Runtime(err) => write!(f, "runtime error: {err}"),
            Self::Storage(err) => write!(f, "storage error: {err}"),
            Self::Entity(err) => write!(f, "domain entity error: {err}"),
            Self::Ipc(err) => write!(f, "IPC error: {err}"),
            Self::Protocol(err) => write!(f, "protocol error: {err}"),
            Self::Harness(err) => write!(f, "harness error: {err}"),
            Self::Checkpoint(err) => write!(f, "checkpoint error: {err}"),
            Self::ActiveTurnInProgress { thread_id, turn_id } => {
                write!(
                    f,
                    "active turn {turn_id} already in progress for thread {thread_id}"
                )
            }
            Self::NoActiveTurn(thread_id) => write!(f, "no active turn for thread {thread_id}"),
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
            Self::PermissionNotFound {
                thread_id,
                permission_id,
            } => {
                write!(
                    f,
                    "permission {permission_id} not found on thread {thread_id}"
                )
            }
            Self::UnsupportedCapability(cap) => write!(f, "unsupported capability: {cap}"),
            Self::SessionNotReady(state) => write!(f, "session is not ready: {state}"),
            Self::SessionAlreadyActive(id) => write!(f, "session already active for thread: {id}"),
            Self::AuthenticationRejected => write!(f, "IPC authentication rejected"),
            Self::LockPoisoned(resource) => write!(f, "lock poisoned: {resource}"),
            Self::Id(err) => write!(f, "identifier error: {err}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::Other(msg) => write!(f, "application error: {msg}"),
        }
    }
}

impl std::error::Error for CoreAppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(err) => Some(err),
            Self::Storage(err) => Some(err),
            Self::Entity(err) => Some(err),
            Self::Ipc(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::Harness(err) => Some(err),
            Self::Checkpoint(err) => Some(err),
            Self::Id(err) => Some(err),
            _ => None,
        }
    }
}

impl From<RuntimeError> for CoreAppError {
    fn from(err: RuntimeError) -> Self {
        match err {
            RuntimeError::OperationAdmitFailed(adm) => Self::Admission(adm),
            RuntimeError::DuplicateOperation(op) => Self::DuplicateOperation(op),
            RuntimeError::SessionNotReady { state } => Self::SessionNotReady(state),
            RuntimeError::ActiveOperationInProgress { thread_id, turn_id } => {
                Self::ActiveTurnInProgress { thread_id, turn_id }
            }
            RuntimeError::NoActiveTurn { thread_id } => Self::NoActiveTurn(thread_id),
            RuntimeError::AutomaticResendForbidden {
                thread_id,
                turn_id,
                delivery,
            } => Self::AutomaticResendForbidden {
                thread_id,
                turn_id,
                delivery,
            },
            RuntimeError::UnsupportedCapability(cap) => Self::UnsupportedCapability(cap),
            RuntimeError::PermissionNotFound {
                thread_id,
                permission_id,
            } => Self::PermissionNotFound {
                thread_id,
                permission_id,
            },
            RuntimeError::Harness(h) => Self::Harness(h),
            RuntimeError::Checkpoint(c) => Self::Checkpoint(c),
            RuntimeError::UnknownThread(t) => Self::ThreadNotFound(t),
            RuntimeError::SessionAlreadyActive(t) => Self::SessionAlreadyActive(t),
            RuntimeError::InvalidSessionId(s) => Self::InvalidInput(s),
        }
    }
}

impl From<StorageError> for CoreAppError {
    fn from(err: StorageError) -> Self {
        Self::Storage(err)
    }
}

impl From<EntityError> for CoreAppError {
    fn from(err: EntityError) -> Self {
        Self::Entity(err)
    }
}

impl From<IpcError> for CoreAppError {
    fn from(err: IpcError) -> Self {
        Self::Ipc(err)
    }
}

impl From<ProtocolError> for CoreAppError {
    fn from(err: ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

impl From<HarnessError> for CoreAppError {
    fn from(err: HarnessError) -> Self {
        Self::Harness(err)
    }
}

impl From<CheckpointError> for CoreAppError {
    fn from(err: CheckpointError) -> Self {
        Self::Checkpoint(err)
    }
}

impl From<AdmissionError> for CoreAppError {
    fn from(err: AdmissionError) -> Self {
        Self::Admission(err)
    }
}

impl From<IdParseError> for CoreAppError {
    fn from(err: IdParseError) -> Self {
        Self::Id(err)
    }
}
