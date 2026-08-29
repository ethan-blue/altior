//! Stable, infrastructure-independent Altior domain contracts.

pub mod id;
pub mod time;

pub use id::{
    AgentProfileId, CoreInstanceId, EventId, HarnessBindingId, IdParseError, OperationId, ThreadId,
    TurnId,
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

/// Long-term memory behavior selected for a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMode {
    Off,
    Session,
    LongTerm,
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
