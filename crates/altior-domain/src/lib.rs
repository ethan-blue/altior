//! Stable, infrastructure-independent Altior domain contracts.

pub mod entity;
pub mod id;
pub mod time;

pub use entity::{
    AGENT_PROFILE_LIST_LIMIT_MAX, AcpHarnessBinding, AgentProfile, AgentProfileCursor,
    AgentProfileListLimit, BoundaryKind, BoundedLabel, BoundedPath, CHECKPOINT_LIST_LIMIT_MAX,
    CheckpointCursor, CheckpointListLimit, CheckpointState, DiagnosticSummary, DisplayName,
    DomainEvent, DomainEventKind, EntityError, EventPayload, HARNESS_BINDING_LIST_LIMIT_MAX,
    HISTORY_LIMIT_MAX, HarnessBindingCursor, HarnessBindingListLimit, HistoryLimit,
    OpaqueSessionId, PERMISSION_LIST_LIMIT_MAX, PROJECT_REF_LIST_LIMIT_MAX, Permission,
    PermissionCursor, PermissionDecision, PermissionDescription, PermissionKind,
    PermissionListLimit, ProjectRef, ProjectRefCursor, ProjectRefListLimit, RemoteRequestId,
    RuntimeCheckpoint, SearchQuery, SessionBinding, THREAD_LIST_LIMIT_MAX, TURN_LIST_LIMIT_MAX,
    Thread, ThreadCursor, ThreadListLimit, ThreadState, ThreadTitle, Turn, TurnCursor,
    TurnListLimit, TurnState,
};

pub use id::{
    AgentProfileId, CoreInstanceId, EventId, HarnessBindingId, IdParseError, OperationId,
    ProjectId, RuntimeCheckpointId, ThreadId, TurnId,
};
pub use time::{LogicalTick, TimeError, UnixMillis};

/// Synchronization policy for a durable data family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncPolicy {
    /// The value never leaves its authoring device.
    DeviceLocal,
    /// The value is synchronized through the encrypted Personal Vault.
    PersonalVault,
    /// Synchronization is explicitly selected by the vault owner.
    OptIn,
}

/// Agent execution path selected for a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HarnessKind {
    Acp,
    Terminal,
    Native,
}

impl HarnessKind {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Terminal => "terminal",
            Self::Native => "native",
        }
    }

    /// Parses a harness kind from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidHarnessKind`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, EntityError> {
        match s {
            "acp" => Ok(Self::Acp),
            "terminal" => Ok(Self::Terminal),
            "native" => Ok(Self::Native),
            _ => Err(EntityError::InvalidHarnessKind),
        }
    }
}

/// Long-term memory behavior selected for a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMode {
    Off,
    Session,
    LongTerm,
}

impl MemoryMode {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Session => "session",
            Self::LongTerm => "long_term",
        }
    }

    /// Parses a memory mode from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidMemoryMode`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, EntityError> {
        match s {
            "off" => Ok(Self::Off),
            "session" => Ok(Self::Session),
            "long_term" => Ok(Self::LongTerm),
            _ => Err(EntityError::InvalidMemoryMode),
        }
    }
}

/// Classification of one prompt delivery against its harness boundary.
///
/// This is the frozen vocabulary for the P0.3 ACP spike: a turn may be
/// re-sent only when delivery is provably [`Absent`](Self::Absent) or was
/// explicitly [`Rejected`](Self::Rejected). [`Indeterminate`](Self::Indeterminate)
/// delivery must never trigger an automatic resend (ADR 0002).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryState {
    /// The prompt provably never reached the harness; a resend is safe.
    Absent,
    /// The harness acknowledged receipt; never resend.
    Confirmed,
    /// The harness rejected the prompt before execution; a corrected
    /// resend is allowed.
    Rejected,
    /// Delivery could not be determined; never resend automatically.
    Indeterminate,
}
