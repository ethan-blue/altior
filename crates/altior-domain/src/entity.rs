//! Domain entities for P1.1: `AgentProfile`, `Thread`, `Turn`, `Event`, `Permission`,
//! `ProjectRef`, and `HarnessBinding` (ADR 0013).
//!
//! These are pure domain records: no rusqlite, no protocol envelopes, no ACP.
//! Invariants are enforced at construction with typed validation errors.
//! All boundary inputs (names, labels, queries) are bounded.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::{
    AgentProfileId, EventId, HarnessBindingId, OperationId, ProjectId, ThreadId, TurnId,
};
use crate::time::UnixMillis;
use crate::{DeliveryState, HarnessKind, MemoryMode};

// ── Bounded strings ────────────────────────────────────────────────

/// Maximum display name length in bytes.
const MAX_DISPLAY_NAME_BYTES: usize = 256;

/// Maximum thread title length in bytes.
const MAX_THREAD_TITLE_BYTES: usize = 512;

/// Maximum search query length in bytes.
const MAX_SEARCH_QUERY_BYTES: usize = 256;

/// Maximum bounded label length in bytes.
const MAX_LABEL_BYTES: usize = 256;

/// Maximum path length in bytes.
const MAX_PATH_BYTES: usize = 4096;

/// A bounded display name validated at construction.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DisplayName(String);

impl DisplayName {
    /// The byte-cap of a display name.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_DISPLAY_NAME_BYTES
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for DisplayName {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(EntityError::EmptyDisplayName);
        }
        if trimmed.len() > MAX_DISPLAY_NAME_BYTES {
            return Err(EntityError::DisplayNameTooLong {
                length: trimmed.len(),
                max: MAX_DISPLAY_NAME_BYTES,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl TryFrom<String> for DisplayName {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<DisplayName> for String {
    fn from(value: DisplayName) -> Self {
        value.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A bounded thread title validated at construction. May be empty
/// (untitled threads).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThreadTitle(String);

impl ThreadTitle {
    /// An empty (untitled) thread title.
    pub const UNTITLED: Self = Self(String::new());

    /// The byte-cap of a thread title.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_THREAD_TITLE_BYTES
    }

    /// Returns the title as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this thread has no title.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<&str> for ThreadTitle {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() > MAX_THREAD_TITLE_BYTES {
            return Err(EntityError::ThreadTitleTooLong {
                length: value.len(),
                max: MAX_THREAD_TITLE_BYTES,
            });
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for ThreadTitle {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > MAX_THREAD_TITLE_BYTES {
            return Err(EntityError::ThreadTitleTooLong {
                length: value.len(),
                max: MAX_THREAD_TITLE_BYTES,
            });
        }
        Ok(Self(value))
    }
}

impl From<ThreadTitle> for String {
    fn from(value: ThreadTitle) -> Self {
        value.0
    }
}

impl fmt::Display for ThreadTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A bounded search query validated at construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery(String);

impl SearchQuery {
    /// The byte-cap of a search query.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_SEARCH_QUERY_BYTES
    }

    /// Returns the query as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for SearchQuery {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(EntityError::EmptySearchQuery);
        }
        if trimmed.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(EntityError::SearchQueryTooLong {
                length: trimmed.len(),
                max: MAX_SEARCH_QUERY_BYTES,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }
}

// ── Agent profile ──────────────────────────────────────────────────

/// An Altior-owned agent profile (ADR 0002, ARCHITECTURE.md).
///
/// The profile selects identity, memory, and preferred harness bindings.
/// It is not a provider process — ACP, terminal, and native execution
/// are adapters below this stable profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProfile {
    /// The stable identity of this profile.
    pub id: AgentProfileId,
    /// Human-readable display name.
    pub display_name: DisplayName,
    /// Preferred harness kind for new threads.
    pub preferred_harness: HarnessKind,
    /// Long-term memory behavior.
    pub memory_mode: MemoryMode,
    /// When the profile was created.
    pub created_at: UnixMillis,
    /// When the profile was last updated.
    pub updated_at: UnixMillis,
}

// ── ACP harness binding ────────────────────────────────────────────

/// A bounded label for an executable path or command fragment.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BoundedPath(String);

impl BoundedPath {
    /// The byte-cap.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_PATH_BYTES
    }

    /// Returns the path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for BoundedPath {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(EntityError::EmptyPath);
        }
        if trimmed.len() > MAX_PATH_BYTES {
            return Err(EntityError::PathTooLong {
                length: trimmed.len(),
                max: MAX_PATH_BYTES,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl TryFrom<String> for BoundedPath {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<BoundedPath> for String {
    fn from(value: BoundedPath) -> Self {
        value.0
    }
}

impl fmt::Display for BoundedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A device-local ACP harness binding: the executable path and arguments
/// needed to launch an ACP agent process.
///
/// This is the replaceable binding on an `AgentProfile`, not the profile
/// itself (ARCHITECTURE.md: "A harness session ID is an optional
/// replaceable binding on a Thread").
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpHarnessBinding {
    /// The stable identity of this binding.
    pub id: HarnessBindingId,
    /// The agent profile this binding belongs to.
    pub agent_profile_id: AgentProfileId,
    /// Human-readable label.
    pub label: DisplayName,
    /// The executable path.
    pub command: BoundedPath,
    /// When the binding was created.
    pub created_at: UnixMillis,
}

// ── Thread lifecycle ───────────────────────────────────────────────

/// Lifecycle state of a conversation thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThreadState {
    /// The thread is open and accepting turns.
    Open,
    /// The thread is pinned (sorted above unpinned threads).
    Pinned,
    /// The thread is archived (hidden from the default list).
    Archived,
}

/// An Altior-owned conversation thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thread {
    /// The stable identity of this thread.
    pub id: ThreadId,
    /// The agent profile this thread is bound to.
    pub agent_profile_id: AgentProfileId,
    /// Optional human-readable title.
    pub title: ThreadTitle,
    /// Lifecycle state.
    pub state: ThreadState,
    /// Optional project association.
    pub project_id: Option<ProjectId>,
    /// When the thread was created.
    pub created_at: UnixMillis,
    /// When the thread was last active (any event).
    pub updated_at: UnixMillis,
}

// ── Turn lifecycle ─────────────────────────────────────────────────

/// Lifecycle state of a turn within a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TurnState {
    /// The turn has been started but not yet completed.
    Active,
    /// The turn completed successfully.
    Completed,
    /// The turn was cancelled.
    Cancelled,
    /// The turn ended with an error.
    Failed,
}

/// A delivery-safe unit of work inside a thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Turn {
    /// The stable identity of this turn.
    pub id: TurnId,
    /// The thread this turn belongs to.
    pub thread_id: ThreadId,
    /// Optional parent operation (for multi-agent delegation).
    pub operation_id: Option<OperationId>,
    /// Lifecycle state.
    pub state: TurnState,
    /// Delivery classification.
    pub delivery: DeliveryState,
    /// When the turn started.
    pub started_at: UnixMillis,
    /// When the turn reached a terminal state, if it has.
    pub ended_at: Option<UnixMillis>,
}

// ── Permission ─────────────────────────────────────────────────────

/// The kind of permission requested by an agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PermissionKind {
    /// Execute a command or tool invocation.
    Execute,
    /// Read a file or resource.
    Read,
    /// Write a file or resource.
    Write,
    /// Network access.
    Network,
}

impl PermissionKind {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Read => "read",
            Self::Write => "write",
            Self::Network => "network",
        }
    }

    /// Parses a permission kind from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidPermissionKind`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, EntityError> {
        match s {
            "execute" => Ok(Self::Execute),
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "network" => Ok(Self::Network),
            _ => Err(EntityError::InvalidPermissionKind),
        }
    }
}

/// The decision on a permission request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PermissionDecision {
    /// Waiting for user decision.
    Pending,
    /// Approved by the user.
    Approved,
    /// Denied by the user.
    Denied,
}

impl PermissionDecision {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }

    /// Parses a permission decision from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidPermissionDecision`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, EntityError> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            _ => Err(EntityError::InvalidPermissionDecision),
        }
    }
}

/// Maximum length of a permission description in bytes.
const MAX_PERMISSION_DESCRIPTION_BYTES: usize = 4096;

/// A bounded permission description.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PermissionDescription(String);

impl PermissionDescription {
    /// The byte-cap.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_PERMISSION_DESCRIPTION_BYTES
    }

    /// Returns the description as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for PermissionDescription {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() > MAX_PERMISSION_DESCRIPTION_BYTES {
            return Err(EntityError::PermissionDescriptionTooLong {
                length: value.len(),
                max: MAX_PERMISSION_DESCRIPTION_BYTES,
            });
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for PermissionDescription {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > MAX_PERMISSION_DESCRIPTION_BYTES {
            return Err(EntityError::PermissionDescriptionTooLong {
                length: value.len(),
                max: MAX_PERMISSION_DESCRIPTION_BYTES,
            });
        }
        Ok(Self(value))
    }
}

impl From<PermissionDescription> for String {
    fn from(value: PermissionDescription) -> Self {
        value.0
    }
}

impl fmt::Display for PermissionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A permission request within a turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Permission {
    /// The event id that recorded this permission request.
    pub event_id: EventId,
    /// The turn that requested this permission.
    pub turn_id: TurnId,
    /// The thread this permission belongs to.
    pub thread_id: ThreadId,
    /// What kind of access is requested.
    pub kind: PermissionKind,
    /// Human-readable description of what is requested.
    pub description: PermissionDescription,
    /// Current decision.
    pub decision: PermissionDecision,
    /// When the request was made.
    pub requested_at: UnixMillis,
    /// When the decision was recorded, if decided.
    pub decided_at: Option<UnixMillis>,
}

// ── Project reference ──────────────────────────────────────────────

/// A bounded label for a project.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BoundedLabel(String);

impl BoundedLabel {
    /// The byte-cap.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_LABEL_BYTES
    }

    /// Returns the label as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for BoundedLabel {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(EntityError::EmptyLabel);
        }
        if trimmed.len() > MAX_LABEL_BYTES {
            return Err(EntityError::LabelTooLong {
                length: trimmed.len(),
                max: MAX_LABEL_BYTES,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl TryFrom<String> for BoundedLabel {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<BoundedLabel> for String {
    fn from(value: BoundedLabel) -> Self {
        value.0
    }
}

impl fmt::Display for BoundedLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A device-local project reference: a label plus an approved local path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRef {
    /// The stable identity of this project reference.
    pub id: ProjectId,
    /// Human-readable label.
    pub label: BoundedLabel,
    /// The approved local path.
    pub path: BoundedPath,
    /// When the reference was created.
    pub created_at: UnixMillis,
}

// ── Domain journal record ──────────────────────────────────────────

/// Maximum length of a domain event payload in bytes.
const MAX_EVENT_PAYLOAD_BYTES: usize = 1024 * 1024;

/// A bounded event payload: the domain-layer JSON content of one event.
///
/// This is distinct from the IPC `EventEnvelope`: the domain record is
/// the durable authority; the IPC envelope is a transport DTO. Protocol
/// DTOs and domain persistence are decoupled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventPayload(Vec<u8>);

impl EventPayload {
    /// The byte-cap of an event payload.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_EVENT_PAYLOAD_BYTES
    }

    /// Returns the payload bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the payload as a UTF-8 string, if valid.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Payload length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<Vec<u8>> for EventPayload {
    type Error = EntityError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        if value.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(EntityError::EventPayloadTooLarge {
                size_bytes: value.len(),
                limit_bytes: MAX_EVENT_PAYLOAD_BYTES,
            });
        }
        let text = std::str::from_utf8(&value).map_err(|_| EntityError::EventPayloadNotUtf8)?;
        if !matches!(
            serde_json::from_str::<serde_json::Value>(text),
            Ok(serde_json::Value::Object(_))
        ) {
            return Err(EntityError::EventPayloadNotJsonObject);
        }
        Ok(Self(value))
    }
}

impl TryFrom<&[u8]> for EventPayload {
    type Error = EntityError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from(value.to_vec())
    }
}

impl From<EventPayload> for Vec<u8> {
    fn from(value: EventPayload) -> Self {
        value.0
    }
}

/// Classification of a domain event's kind for routing and projection.
///
/// This is the domain vocabulary — not the IPC `KnownEvent` which is a
/// transport concern. Domain events record what happened to domain
/// entities; protocol events are how the transport describes them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DomainEventKind {
    /// A thread was created.
    ThreadCreated,
    /// A thread title was changed.
    ThreadTitleChanged,
    /// A thread state was changed (pin, archive, reopen).
    ThreadStateChanged,
    /// A turn started.
    TurnStarted,
    /// A streaming message delta.
    MessageDelta,
    /// A turn completed.
    TurnCompleted,
    /// A turn was cancelled.
    TurnCancelled,
    /// A turn failed.
    TurnFailed,
    /// A permission was requested.
    PermissionRequested,
    /// A permission decision was recorded.
    PermissionDecided,
    /// An event that does not match any known domain kind; the string
    /// is preserved for forward compatibility.
    Other(String),
}

impl DomainEventKind {
    /// Maximum length in bytes for a custom domain event kind.
    pub const MAX_OTHER_KIND_BYTES: usize = 128;

    /// Returns the canonical string name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::ThreadCreated => "thread.created",
            Self::ThreadTitleChanged => "thread.title_changed",
            Self::ThreadStateChanged => "thread.state_changed",
            Self::TurnStarted => "turn.started",
            Self::MessageDelta => "message.delta",
            Self::TurnCompleted => "turn.completed",
            Self::TurnCancelled => "turn.cancelled",
            Self::TurnFailed => "turn.failed",
            Self::PermissionRequested => "permission.requested",
            Self::PermissionDecided => "permission.decided",
            Self::Other(s) => s,
        }
    }

    /// Parses a kind name string with bounded forward-compatible custom kinds.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidDomainEventKind`] for empty, oversized,
    /// or non-canonical custom kinds.
    pub fn try_from_str(s: &str) -> Result<Self, EntityError> {
        if s.is_empty()
            || s.len() > Self::MAX_OTHER_KIND_BYTES
            || !s.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(EntityError::InvalidDomainEventKind);
        }
        Ok(match s {
            "thread.created" => Self::ThreadCreated,
            "thread.title_changed" => Self::ThreadTitleChanged,
            "thread.state_changed" => Self::ThreadStateChanged,
            "turn.started" => Self::TurnStarted,
            "message.delta" => Self::MessageDelta,
            "turn.completed" => Self::TurnCompleted,
            "turn.cancelled" => Self::TurnCancelled,
            "turn.failed" => Self::TurnFailed,
            "permission.requested" => Self::PermissionRequested,
            "permission.decided" => Self::PermissionDecided,
            other => Self::Other(other.to_owned()),
        })
    }
}

impl TryFrom<&str> for DomainEventKind {
    type Error = EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(value)
    }
}

impl TryFrom<String> for DomainEventKind {
    type Error = EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_str(&value)
    }
}

impl std::str::FromStr for DomainEventKind {
    type Err = EntityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl From<DomainEventKind> for String {
    fn from(kind: DomainEventKind) -> Self {
        kind.as_str().to_owned()
    }
}

impl fmt::Display for DomainEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A domain event record: the durable authority for one thing that
/// happened inside Altior.
///
/// This is decoupled from the IPC `EventEnvelope`. The journal stores
/// domain records; the IPC layer maps to/from protocol DTOs at the
/// adapter boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEvent {
    /// The stable identity of this event.
    pub event_id: EventId,
    /// The thread this event belongs to, when thread-scoped.
    pub thread_id: Option<ThreadId>,
    /// The turn this event belongs to, when turn-scoped.
    pub turn_id: Option<TurnId>,
    /// Optional parent operation id.
    pub operation_id: Option<OperationId>,
    /// The domain event kind.
    pub kind: DomainEventKind,
    /// The event payload (JSON bytes).
    pub payload: EventPayload,
    /// When the event occurred.
    pub occurred_at: UnixMillis,
}

// ── Query types ────────────────────────────────────────────────────

/// Maximum number of threads in a bounded list page.
pub const THREAD_LIST_LIMIT_MAX: u32 = 200;

/// Maximum number of events in a bounded history page.
pub const HISTORY_LIMIT_MAX: u32 = 500;

/// Maximum number of agent profiles in a bounded list page.
pub const AGENT_PROFILE_LIST_LIMIT_MAX: u32 = 200;

/// Maximum number of harness bindings in a bounded list page.
pub const HARNESS_BINDING_LIST_LIMIT_MAX: u32 = 200;

/// Maximum number of project references in a bounded list page.
pub const PROJECT_REF_LIST_LIMIT_MAX: u32 = 200;

/// Maximum number of permissions in a bounded list page.
pub const PERMISSION_LIST_LIMIT_MAX: u32 = 500;

/// Maximum number of turns in a bounded list page.
pub const TURN_LIST_LIMIT_MAX: u32 = 500;

/// Validated unsigned page size for thread list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadListLimit(u32);

impl ThreadListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::ThreadListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::ThreadListLimitOutOfRange {
                value,
                max: THREAD_LIST_LIMIT_MAX,
            });
        }
        if value > THREAD_LIST_LIMIT_MAX {
            return Err(EntityError::ThreadListLimitOutOfRange {
                value,
                max: THREAD_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for history queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryLimit(u32);

impl HistoryLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::HistoryLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::HistoryLimitOutOfRange {
                value,
                max: HISTORY_LIMIT_MAX,
            });
        }
        if value > HISTORY_LIMIT_MAX {
            return Err(EntityError::HistoryLimitOutOfRange {
                value,
                max: HISTORY_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for agent profile list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentProfileListLimit(u32);

impl AgentProfileListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::AgentProfileListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::AgentProfileListLimitOutOfRange {
                value,
                max: AGENT_PROFILE_LIST_LIMIT_MAX,
            });
        }
        if value > AGENT_PROFILE_LIST_LIMIT_MAX {
            return Err(EntityError::AgentProfileListLimitOutOfRange {
                value,
                max: AGENT_PROFILE_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for harness binding list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HarnessBindingListLimit(u32);

impl HarnessBindingListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::HarnessBindingListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::HarnessBindingListLimitOutOfRange {
                value,
                max: HARNESS_BINDING_LIST_LIMIT_MAX,
            });
        }
        if value > HARNESS_BINDING_LIST_LIMIT_MAX {
            return Err(EntityError::HarnessBindingListLimitOutOfRange {
                value,
                max: HARNESS_BINDING_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for project reference list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectRefListLimit(u32);

impl ProjectRefListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::ProjectRefListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::ProjectRefListLimitOutOfRange {
                value,
                max: PROJECT_REF_LIST_LIMIT_MAX,
            });
        }
        if value > PROJECT_REF_LIST_LIMIT_MAX {
            return Err(EntityError::ProjectRefListLimitOutOfRange {
                value,
                max: PROJECT_REF_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for permission list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionListLimit(u32);

impl PermissionListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::PermissionListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::PermissionListLimitOutOfRange {
                value,
                max: PERMISSION_LIST_LIMIT_MAX,
            });
        }
        if value > PERMISSION_LIST_LIMIT_MAX {
            return Err(EntityError::PermissionListLimitOutOfRange {
                value,
                max: PERMISSION_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Validated unsigned page size for turn list queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TurnListLimit(u32);

impl TurnListLimit {
    /// Validates an unsigned page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::TurnListLimitOutOfRange`] above the cap.
    pub fn try_new(value: u32) -> Result<Self, EntityError> {
        if value == 0 {
            return Err(EntityError::TurnListLimitOutOfRange {
                value,
                max: TURN_LIST_LIMIT_MAX,
            });
        }
        if value > TURN_LIST_LIMIT_MAX {
            return Err(EntityError::TurnListLimitOutOfRange {
                value,
                max: TURN_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Unique cursor for stable newest-first thread pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadCursor {
    /// Timestamp of the last row.
    pub updated_at: UnixMillis,
    /// Identifier tie-breaker of the last row.
    pub thread_id: ThreadId,
}

/// Unique cursor for stable agent profile pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProfileCursor {
    /// Timestamp of the last row.
    pub updated_at: UnixMillis,
    /// Identifier tie-breaker of the last row.
    pub agent_profile_id: AgentProfileId,
}

/// Unique cursor for stable harness binding pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessBindingCursor {
    /// Timestamp of the last row.
    pub created_at: UnixMillis,
    /// Identifier tie-breaker of the last row.
    pub harness_binding_id: HarnessBindingId,
}

/// Unique cursor for stable project reference pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRefCursor {
    /// Timestamp of the last row.
    pub created_at: UnixMillis,
    /// Identifier tie-breaker of the last row.
    pub project_id: ProjectId,
}

/// Unique cursor for stable permission query pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionCursor {
    /// Request timestamp of the last row.
    pub requested_at: UnixMillis,
    /// Event identifier tie-breaker of the last row.
    pub event_id: EventId,
}

/// Unique cursor for stable turn pagination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnCursor {
    /// Start timestamp of the last row.
    pub started_at: UnixMillis,
    /// Turn identifier tie-breaker of the last row.
    pub turn_id: TurnId,
}

// ── Typed validation errors ────────────────────────────────────────

/// Typed validation error for domain entity construction.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EntityError {
    /// A display name is empty after trimming.
    EmptyDisplayName,
    /// A display name exceeds the byte cap.
    DisplayNameTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A thread title exceeds the byte cap.
    ThreadTitleTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A search query is empty after trimming.
    EmptySearchQuery,
    /// A search query exceeds the byte cap.
    SearchQueryTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A label is empty after trimming.
    EmptyLabel,
    /// A label exceeds the byte cap.
    LabelTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A path is empty after trimming.
    EmptyPath,
    /// A path exceeds the byte cap.
    PathTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// An event payload is not valid UTF-8.
    EventPayloadNotUtf8,
    /// An event payload is not a JSON object.
    EventPayloadNotJsonObject,
    /// A custom domain event kind is invalid or exceeds its cap.
    InvalidDomainEventKind,
    /// An event payload exceeds the byte cap.
    EventPayloadTooLarge {
        /// The actual size.
        size_bytes: usize,
        /// The maximum allowed size.
        limit_bytes: usize,
    },
    /// A permission description exceeds the byte cap.
    PermissionDescriptionTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A thread list page size is outside the valid range.
    ThreadListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// A history page size is outside the valid range.
    HistoryLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// An agent profile list page size is outside the valid range.
    AgentProfileListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// A harness binding list page size is outside the valid range.
    HarnessBindingListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// A project reference list page size is outside the valid range.
    ProjectRefListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// A permission list page size is outside the valid range.
    PermissionListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// A turn list page size is outside the valid range.
    TurnListLimitOutOfRange {
        /// The rejected value.
        value: u32,
        /// The maximum allowed value.
        max: u32,
    },
    /// The string does not name a valid harness kind.
    InvalidHarnessKind,
    /// The string does not name a valid memory mode.
    InvalidMemoryMode,
    /// The string does not name a valid permission kind.
    InvalidPermissionKind,
    /// The string does not name a valid permission decision.
    InvalidPermissionDecision,
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDisplayName => write!(f, "display name is empty"),
            Self::DisplayNameTooLong { length, max } => {
                write!(f, "display name is {length} bytes, limit is {max}")
            }
            Self::ThreadTitleTooLong { length, max } => {
                write!(f, "thread title is {length} bytes, limit is {max}")
            }
            Self::EmptySearchQuery => write!(f, "search query is empty"),
            Self::SearchQueryTooLong { length, max } => {
                write!(f, "search query is {length} bytes, limit is {max}")
            }
            Self::EmptyLabel => write!(f, "label is empty"),
            Self::LabelTooLong { length, max } => {
                write!(f, "label is {length} bytes, limit is {max}")
            }
            Self::EmptyPath => write!(f, "path is empty"),
            Self::PathTooLong { length, max } => {
                write!(f, "path is {length} bytes, limit is {max}")
            }
            Self::EventPayloadNotUtf8 => write!(f, "event payload is not UTF-8"),
            Self::EventPayloadNotJsonObject => write!(f, "event payload must be a JSON object"),
            Self::InvalidDomainEventKind => {
                write!(f, "domain event kind is invalid or exceeds its cap")
            }
            Self::EventPayloadTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "event payload is {size_bytes} bytes, limit is {limit_bytes}"
            ),
            Self::PermissionDescriptionTooLong { length, max } => {
                write!(
                    f,
                    "permission description is {length} bytes, limit is {max}"
                )
            }
            Self::ThreadListLimitOutOfRange { value, max } => {
                write!(f, "thread list limit {value} is outside 1..={max}")
            }
            Self::HistoryLimitOutOfRange { value, max } => {
                write!(f, "history limit {value} is outside 1..={max}")
            }
            Self::AgentProfileListLimitOutOfRange { value, max } => {
                write!(f, "agent profile list limit {value} is outside 1..={max}")
            }
            Self::HarnessBindingListLimitOutOfRange { value, max } => {
                write!(f, "harness binding list limit {value} is outside 1..={max}")
            }
            Self::ProjectRefListLimitOutOfRange { value, max } => {
                write!(f, "project ref list limit {value} is outside 1..={max}")
            }
            Self::PermissionListLimitOutOfRange { value, max } => {
                write!(f, "permission list limit {value} is outside 1..={max}")
            }
            Self::TurnListLimitOutOfRange { value, max } => {
                write!(f, "turn list limit {value} is outside 1..={max}")
            }
            Self::InvalidHarnessKind => write!(f, "invalid harness kind"),
            Self::InvalidMemoryMode => write!(f, "invalid memory mode"),
            Self::InvalidPermissionKind => write!(f, "invalid permission kind"),
            Self::InvalidPermissionDecision => write!(f, "invalid permission decision"),
        }
    }
}

impl std::error::Error for EntityError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn display_name_trims_and_validates() {
        assert!(DisplayName::try_from("  Alice  ").is_ok());
        assert_eq!(
            DisplayName::try_from("  Alice  ").unwrap().as_str(),
            "Alice"
        );
        assert_eq!(
            DisplayName::try_from(""),
            Err(EntityError::EmptyDisplayName)
        );
        assert_eq!(
            DisplayName::try_from("   "),
            Err(EntityError::EmptyDisplayName)
        );
        let long = "x".repeat(MAX_DISPLAY_NAME_BYTES + 1);
        assert!(matches!(
            DisplayName::try_from(long.as_str()),
            Err(EntityError::DisplayNameTooLong { .. })
        ));
        let max = "x".repeat(MAX_DISPLAY_NAME_BYTES);
        assert!(DisplayName::try_from(max.as_str()).is_ok());
    }

    #[test]
    fn thread_title_allows_empty_and_bounds() {
        assert!(ThreadTitle::try_from("").is_ok());
        assert!(ThreadTitle::UNTITLED.is_empty());
        let long = "x".repeat(MAX_THREAD_TITLE_BYTES + 1);
        assert!(matches!(
            ThreadTitle::try_from(long.as_str()),
            Err(EntityError::ThreadTitleTooLong { .. })
        ));
    }

    #[test]
    fn search_query_trims_and_bounds() {
        assert!(SearchQuery::try_from("hello").is_ok());
        assert_eq!(
            SearchQuery::try_from("  "),
            Err(EntityError::EmptySearchQuery)
        );
        let long = "x".repeat(MAX_SEARCH_QUERY_BYTES + 1);
        assert!(matches!(
            SearchQuery::try_from(long.as_str()),
            Err(EntityError::SearchQueryTooLong { .. })
        ));
    }

    #[test]
    fn bounded_path_trims_and_bounds() {
        assert!(BoundedPath::try_from("/usr/bin/claude").is_ok());
        assert_eq!(BoundedPath::try_from(""), Err(EntityError::EmptyPath));
        let long = "x".repeat(MAX_PATH_BYTES + 1);
        assert!(matches!(
            BoundedPath::try_from(long.as_str()),
            Err(EntityError::PathTooLong { .. })
        ));
    }

    #[test]
    fn event_payload_bounds() {
        let small = br#"{"text":"small"}"#.to_vec();
        assert!(EventPayload::try_from(small).is_ok());
        assert_eq!(
            EventPayload::try_from(vec![0xff]),
            Err(EntityError::EventPayloadNotUtf8)
        );
        assert_eq!(
            EventPayload::try_from(br"[]".to_vec()),
            Err(EntityError::EventPayloadNotJsonObject)
        );
        let too_big = vec![0u8; MAX_EVENT_PAYLOAD_BYTES + 1];
        assert!(matches!(
            EventPayload::try_from(too_big),
            Err(EntityError::EventPayloadTooLarge { .. })
        ));
    }

    #[test]
    fn thread_list_limit_validates() {
        assert!(ThreadListLimit::try_new(1).is_ok());
        assert!(ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            ThreadListLimit::try_new(0),
            Err(EntityError::ThreadListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX + 1),
            Err(EntityError::ThreadListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn history_limit_validates() {
        assert!(HistoryLimit::try_new(1).is_ok());
        assert!(HistoryLimit::try_new(HISTORY_LIMIT_MAX).is_ok());
        assert!(matches!(
            HistoryLimit::try_new(0),
            Err(EntityError::HistoryLimitOutOfRange { .. })
        ));
        assert!(matches!(
            HistoryLimit::try_new(HISTORY_LIMIT_MAX + 1),
            Err(EntityError::HistoryLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn domain_event_kind_roundtrips() {
        for kind in [
            DomainEventKind::ThreadCreated,
            DomainEventKind::ThreadTitleChanged,
            DomainEventKind::ThreadStateChanged,
            DomainEventKind::TurnStarted,
            DomainEventKind::MessageDelta,
            DomainEventKind::TurnCompleted,
            DomainEventKind::TurnCancelled,
            DomainEventKind::TurnFailed,
            DomainEventKind::PermissionRequested,
            DomainEventKind::PermissionDecided,
            DomainEventKind::Other("custom.kind-1_0".to_owned()),
        ] {
            let name = kind.as_str().to_owned();
            assert_eq!(
                DomainEventKind::try_from_str(&name).expect("parse valid kind"),
                kind
            );
            assert_eq!(kind.to_string(), name);
        }
    }

    #[test]
    fn domain_event_kind_rejects_invalid() {
        // Empty
        assert_eq!(
            DomainEventKind::try_from_str(""),
            Err(EntityError::InvalidDomainEventKind)
        );

        // Oversized (> 128 bytes)
        let exact_max = "a".repeat(DomainEventKind::MAX_OTHER_KIND_BYTES);
        assert!(DomainEventKind::try_from_str(&exact_max).is_ok());
        let too_long = "a".repeat(DomainEventKind::MAX_OTHER_KIND_BYTES + 1);
        assert_eq!(
            DomainEventKind::try_from_str(&too_long),
            Err(EntityError::InvalidDomainEventKind)
        );

        // Illegal characters
        for invalid in [
            "Uppercase.Kind",
            "has space",
            "custom/kind",
            "custom:kind",
            "custom@kind",
            "custom$kind",
            "custom#kind",
            "custom!kind",
            "custom.kind\n",
            "custom.kind\0",
            "custom.kind.你好",
        ] {
            assert_eq!(
                DomainEventKind::try_from_str(invalid),
                Err(EntityError::InvalidDomainEventKind),
                "expected rejection for: {invalid}"
            );
        }
    }

    #[test]
    fn domain_event_kind_serde_validates() {
        let kind = DomainEventKind::Other("custom.event-v1_0".to_owned());
        let json = serde_json::to_string(&kind).expect("serialize");
        assert_eq!(json, "\"custom.event-v1_0\"");
        let deserialized: DomainEventKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, kind);

        // Serde rejects invalid kinds
        let bad_json = "\"Bad.Uppercase\"";
        assert!(serde_json::from_str::<DomainEventKind>(bad_json).is_err());
        let oversized = format!(
            "\"{}\"",
            "a".repeat(DomainEventKind::MAX_OTHER_KIND_BYTES + 1)
        );
        assert!(serde_json::from_str::<DomainEventKind>(&oversized).is_err());
    }

    #[test]
    fn turn_list_limit_validates() {
        assert!(TurnListLimit::try_new(1).is_ok());
        assert!(TurnListLimit::try_new(TURN_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            TurnListLimit::try_new(0),
            Err(EntityError::TurnListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            TurnListLimit::try_new(TURN_LIST_LIMIT_MAX + 1),
            Err(EntityError::TurnListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn permission_description_bounds() {
        let ok = "x".repeat(MAX_PERMISSION_DESCRIPTION_BYTES);
        assert!(PermissionDescription::try_from(ok.as_str()).is_ok());
        let too_long = "x".repeat(MAX_PERMISSION_DESCRIPTION_BYTES + 1);
        assert!(matches!(
            PermissionDescription::try_from(too_long.as_str()),
            Err(EntityError::PermissionDescriptionTooLong { .. })
        ));
    }

    #[test]
    fn agent_profile_constructs() {
        let _profile = AgentProfile {
            id: AgentProfileId::from_str("agp_fixture000000001").unwrap(),
            display_name: DisplayName::try_from("Claude").unwrap(),
            preferred_harness: HarnessKind::Acp,
            memory_mode: MemoryMode::LongTerm,
            created_at: UnixMillis::from_millis(1_700_000_000_000),
            updated_at: UnixMillis::from_millis(1_700_000_000_000),
        };
    }

    #[test]
    fn agent_profile_list_limit_validates() {
        assert!(AgentProfileListLimit::try_new(1).is_ok());
        assert!(AgentProfileListLimit::try_new(AGENT_PROFILE_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            AgentProfileListLimit::try_new(0),
            Err(EntityError::AgentProfileListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            AgentProfileListLimit::try_new(AGENT_PROFILE_LIST_LIMIT_MAX + 1),
            Err(EntityError::AgentProfileListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn harness_binding_list_limit_validates() {
        assert!(HarnessBindingListLimit::try_new(1).is_ok());
        assert!(HarnessBindingListLimit::try_new(HARNESS_BINDING_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            HarnessBindingListLimit::try_new(0),
            Err(EntityError::HarnessBindingListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            HarnessBindingListLimit::try_new(HARNESS_BINDING_LIST_LIMIT_MAX + 1),
            Err(EntityError::HarnessBindingListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn project_ref_list_limit_validates() {
        assert!(ProjectRefListLimit::try_new(1).is_ok());
        assert!(ProjectRefListLimit::try_new(PROJECT_REF_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            ProjectRefListLimit::try_new(0),
            Err(EntityError::ProjectRefListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            ProjectRefListLimit::try_new(PROJECT_REF_LIST_LIMIT_MAX + 1),
            Err(EntityError::ProjectRefListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn permission_list_limit_validates() {
        assert!(PermissionListLimit::try_new(1).is_ok());
        assert!(PermissionListLimit::try_new(PERMISSION_LIST_LIMIT_MAX).is_ok());
        assert!(matches!(
            PermissionListLimit::try_new(0),
            Err(EntityError::PermissionListLimitOutOfRange { .. })
        ));
        assert!(matches!(
            PermissionListLimit::try_new(PERMISSION_LIST_LIMIT_MAX + 1),
            Err(EntityError::PermissionListLimitOutOfRange { .. })
        ));
    }

    #[test]
    fn domain_enum_roundtrips() {
        assert_eq!(HarnessKind::try_from_str("acp"), Ok(HarnessKind::Acp));
        assert_eq!(
            HarnessKind::try_from_str("terminal"),
            Ok(HarnessKind::Terminal)
        );
        assert_eq!(HarnessKind::try_from_str("native"), Ok(HarnessKind::Native));
        assert_eq!(
            HarnessKind::try_from_str("other"),
            Err(EntityError::InvalidHarnessKind)
        );

        assert_eq!(MemoryMode::try_from_str("off"), Ok(MemoryMode::Off));
        assert_eq!(MemoryMode::try_from_str("session"), Ok(MemoryMode::Session));
        assert_eq!(
            MemoryMode::try_from_str("long_term"),
            Ok(MemoryMode::LongTerm)
        );
        assert_eq!(
            MemoryMode::try_from_str("other"),
            Err(EntityError::InvalidMemoryMode)
        );

        assert_eq!(
            PermissionKind::try_from_str("execute"),
            Ok(PermissionKind::Execute)
        );
        assert_eq!(
            PermissionKind::try_from_str("read"),
            Ok(PermissionKind::Read)
        );
        assert_eq!(
            PermissionKind::try_from_str("write"),
            Ok(PermissionKind::Write)
        );
        assert_eq!(
            PermissionKind::try_from_str("network"),
            Ok(PermissionKind::Network)
        );
        assert_eq!(
            PermissionKind::try_from_str("other"),
            Err(EntityError::InvalidPermissionKind)
        );

        assert_eq!(
            PermissionDecision::try_from_str("pending"),
            Ok(PermissionDecision::Pending)
        );
        assert_eq!(
            PermissionDecision::try_from_str("approved"),
            Ok(PermissionDecision::Approved)
        );
        assert_eq!(
            PermissionDecision::try_from_str("denied"),
            Ok(PermissionDecision::Denied)
        );
        assert_eq!(
            PermissionDecision::try_from_str("other"),
            Err(EntityError::InvalidPermissionDecision)
        );
    }

    #[test]
    fn display_name_serde_roundtrip() {
        let name = DisplayName::try_from("Alice").unwrap();
        let json = serde_json::to_value(&name).unwrap();
        assert_eq!(json, serde_json::json!("Alice"));
        let decoded: DisplayName = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, name);

        // Reject empty on deserialize
        let bad = serde_json::from_value::<DisplayName>(serde_json::json!("  "));
        assert!(bad.is_err());
    }

    #[test]
    fn thread_cursor_constructs_and_compares() {
        let cursor1 = ThreadCursor {
            updated_at: UnixMillis::from_millis(1_700_000_000_000),
            thread_id: ThreadId::from_str("thr_fixture000000001").unwrap(),
        };
        let cursor2 = cursor1.clone();
        assert_eq!(cursor1, cursor2);
    }
}
