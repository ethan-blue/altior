//! Versioned Data Transfer Objects (DTOs) for Desktop/Core IPC.
//!
//! These DTOs define serializable data shapes exchanged between Desktop
//! and Core for threads, turns, permissions, agent profiles, harness bindings,
//! cursors, and pagination responses.
//!
//! Secret isolation rule (ADR 0006, ADR 0014): plaintext secrets never enter
//! DTOs. Configuration payloads only hold opaque secret references
//! (`secret_refs`) or command metadata.

use serde::{Deserialize, Serialize};

use altior_domain::{
    AcpHarnessBinding, AgentProfile, DeliveryState, EventId, HarnessBindingId, OperationId,
    Permission, ProjectId, Thread, ThreadCursor, ThreadId, ThreadState, Turn, TurnCursor, TurnId,
    TurnState, UnixMillis,
};

fn thread_state_to_str(s: ThreadState) -> &'static str {
    match s {
        ThreadState::Open => "open",
        ThreadState::Pinned => "pinned",
        ThreadState::Archived => "archived",
    }
}

fn turn_state_to_str(s: TurnState) -> &'static str {
    match s {
        TurnState::Active => "active",
        TurnState::Completed => "completed",
        TurnState::Cancelled => "cancelled",
        TurnState::Failed => "failed",
    }
}

fn delivery_state_to_str(state: DeliveryState) -> &'static str {
    match state {
        DeliveryState::Absent => "absent",
        DeliveryState::Confirmed => "confirmed",
        DeliveryState::Rejected => "rejected",
        DeliveryState::Indeterminate => "indeterminate",
    }
}

/// Serializable representation of a conversation thread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadDto {
    /// Identity of the thread.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub id: ThreadId,
    /// Associated agent profile.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub agent_profile_id: altior_domain::AgentProfileId,
    /// Thread title (bounded string).
    pub title: String,
    /// Lifecycle state: `"open"`, `"pinned"`, or `"archived"`.
    pub state: String,
    /// Optional associated project reference.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub project_id: Option<ProjectId>,
    /// Thread creation timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub created_at: UnixMillis,
    /// Thread last updated timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub updated_at: UnixMillis,
}

impl From<&Thread> for ThreadDto {
    fn from(t: &Thread) -> Self {
        Self {
            id: t.id.clone(),
            agent_profile_id: t.agent_profile_id.clone(),
            title: t.title.as_str().to_owned(),
            state: thread_state_to_str(t.state).to_owned(),
            project_id: t.project_id.clone(),
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

impl From<Thread> for ThreadDto {
    fn from(t: Thread) -> Self {
        Self::from(&t)
    }
}

/// Serializable representation of an execution turn inside a thread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct TurnDto {
    /// Identity of the turn.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub id: TurnId,
    /// Owning thread identity.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Turn state: `"active"`, `"completed"`, `"cancelled"`, or `"failed"`.
    pub state: String,
    /// Prompt delivery classification: `"absent"`, `"confirmed"`, `"rejected"`, or `"indeterminate"`.
    pub delivery_state: String,
    /// Associated operation identity, when present.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub operation_id: Option<OperationId>,
    /// Turn start timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub started_at: UnixMillis,
    /// Turn finish timestamp, if terminal.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<u64>"))]
    pub ended_at: Option<UnixMillis>,
}

impl From<&Turn> for TurnDto {
    fn from(t: &Turn) -> Self {
        Self {
            id: t.id.clone(),
            thread_id: t.thread_id.clone(),
            state: turn_state_to_str(t.state).to_owned(),
            delivery_state: delivery_state_to_str(t.delivery).to_owned(),
            operation_id: t.operation_id.clone(),
            started_at: t.started_at,
            ended_at: t.ended_at,
        }
    }
}

impl From<Turn> for TurnDto {
    fn from(t: Turn) -> Self {
        Self::from(&t)
    }
}

/// Serializable representation of a user permission request/decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct PermissionDto {
    /// Identity of the permission request event.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub event_id: EventId,
    /// Owning turn identity.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub turn_id: TurnId,
    /// Owning thread identity.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Kind of permission: `"execute"`, `"read"`, `"write"`, or `"network"`.
    pub kind: String,
    /// Bounded description of the requested action.
    pub description: String,
    /// Decision: `"approved"`, `"denied"`, or `"pending"`.
    pub decision: String,
    /// When permission was requested.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub requested_at: UnixMillis,
    /// When permission was decided, if resolved.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<u64>"))]
    pub decided_at: Option<UnixMillis>,
}

impl From<&Permission> for PermissionDto {
    fn from(p: &Permission) -> Self {
        Self {
            event_id: p.event_id.clone(),
            turn_id: p.turn_id.clone(),
            thread_id: p.thread_id.clone(),
            kind: p.kind.as_str().to_owned(),
            description: p.description.as_str().to_owned(),
            decision: p.decision.as_str().to_owned(),
            requested_at: p.requested_at,
            decided_at: p.decided_at,
        }
    }
}

impl From<Permission> for PermissionDto {
    fn from(p: Permission) -> Self {
        Self::from(&p)
    }
}

/// Serializable representation of an agent profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct AgentProfileDto {
    /// Identity of the agent profile.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub id: altior_domain::AgentProfileId,
    /// Display name.
    pub display_name: String,
    /// Preferred harness: `"acp"`, `"terminal"`, or `"native"`.
    pub preferred_harness: String,
    /// Memory mode: `"off"`, `"session"`, or `"long_term"`.
    pub memory_mode: String,
    /// Creation timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub created_at: UnixMillis,
    /// Last update timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub updated_at: UnixMillis,
}

impl From<&AgentProfile> for AgentProfileDto {
    fn from(p: &AgentProfile) -> Self {
        Self {
            id: p.id.clone(),
            display_name: p.display_name.as_str().to_owned(),
            preferred_harness: p.preferred_harness.as_str().to_owned(),
            memory_mode: p.memory_mode.as_str().to_owned(),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

impl From<AgentProfile> for AgentProfileDto {
    fn from(p: AgentProfile) -> Self {
        Self::from(&p)
    }
}

/// Serializable representation of a harness launch binding.
///
/// NOTE: In accordance with ADR 0006 / ADR 0014, plaintext secrets are NEVER
/// stored or transferred. `secret_refs` holds only opaque reference keys.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct HarnessBindingDto {
    /// Identity of this harness binding.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub id: HarnessBindingId,
    /// Associated agent profile.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub agent_profile_id: altior_domain::AgentProfileId,
    /// Executable program path.
    pub program: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Environment variable keys.
    pub env_keys: Vec<String>,
    /// Opaque secret references (NEVER plaintext values).
    pub secret_refs: Vec<String>,
    /// Human-readable label.
    pub label: String,
    /// Creation timestamp.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub created_at: UnixMillis,
}

impl From<&AcpHarnessBinding> for HarnessBindingDto {
    fn from(b: &AcpHarnessBinding) -> Self {
        Self {
            id: b.id.clone(),
            agent_profile_id: b.agent_profile_id.clone(),
            program: b.command.as_str().to_owned(),
            args: Vec::new(),
            env_keys: Vec::new(),
            secret_refs: Vec::new(),
            label: b.label.as_str().to_owned(),
            created_at: b.created_at,
        }
    }
}

impl From<AcpHarnessBinding> for HarnessBindingDto {
    fn from(b: AcpHarnessBinding) -> Self {
        Self::from(&b)
    }
}

/// Cursor for paginating threads (newest-first).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadCursorDto {
    /// Updated timestamp of the last seen thread.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub updated_at: UnixMillis,
    /// Thread ID for tie-breaking.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
}

impl From<&ThreadCursor> for ThreadCursorDto {
    fn from(c: &ThreadCursor) -> Self {
        Self {
            updated_at: c.updated_at,
            thread_id: c.thread_id.clone(),
        }
    }
}

impl From<ThreadCursorDto> for ThreadCursor {
    fn from(dto: ThreadCursorDto) -> Self {
        Self {
            updated_at: dto.updated_at,
            thread_id: dto.thread_id,
        }
    }
}

/// Cursor for paginating turns chronologically.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct TurnCursorDto {
    /// Started timestamp of the last seen turn.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub started_at: UnixMillis,
    /// Turn ID for tie-breaking.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub turn_id: TurnId,
}

impl From<&TurnCursor> for TurnCursorDto {
    fn from(c: &TurnCursor) -> Self {
        Self {
            started_at: c.started_at,
            turn_id: c.turn_id.clone(),
        }
    }
}

impl From<TurnCursorDto> for TurnCursor {
    fn from(dto: TurnCursorDto) -> Self {
        Self {
            started_at: dto.started_at,
            turn_id: dto.turn_id,
        }
    }
}

/// Summary item in a thread list view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadSummaryDto {
    /// The thread record.
    pub thread: ThreadDto,
    /// The most recent turn in this thread, if any.
    pub last_turn: Option<TurnDto>,
    /// Currently active turn, if any.
    pub active_turn: Option<TurnDto>,
}

/// Bounded snapshot of a full thread including recent turns and pending permissions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadSnapshotDto {
    /// The thread record.
    pub thread: ThreadDto,
    /// Associated agent profile, if resolved.
    pub agent_profile: Option<AgentProfileDto>,
    /// Recent turns in this thread (bounded).
    pub turns: Vec<TurnDto>,
    /// Currently pending permission requests.
    pub pending_permissions: Vec<PermissionDto>,
}

/// Paginated response for thread listing and search.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadListResponseDto {
    /// Thread summaries in this page.
    pub threads: Vec<ThreadSummaryDto>,
    /// Cursor for fetching the next page, if more exist.
    pub next_cursor: Option<ThreadCursorDto>,
    /// Whether additional items exist beyond this page.
    pub has_more: bool,
}

/// Paginated response for thread turn history.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ThreadHistoryResponseDto {
    /// Owning thread identity.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Turns in this history page.
    pub turns: Vec<TurnDto>,
    /// Cursor for fetching next history page.
    pub next_cursor: Option<TurnCursorDto>,
    /// Whether additional turns exist.
    pub has_more: bool,
}

/// Diagnostics and status summary for the Core runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct RuntimeDiagnosticsDto {
    /// Core process instance ID.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub instance_id: altior_domain::CoreInstanceId,
    /// Health status: `"ready"`, `"busy"`, `"degraded"`, or `"shutting_down"`.
    pub status: String,
    /// Number of active threads in memory.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub active_threads: u32,
    /// Number of turns currently executing.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub active_turns: u32,
    /// Redacted diagnostics summary text, if available.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use altior_domain::{
        BoundedPath, DisplayName, PermissionDescription, PermissionKind, ThreadTitle,
    };

    #[test]
    fn thread_dto_roundtrips() {
        let thread = Thread {
            id: "thr_fixture000000001".parse().unwrap(),
            agent_profile_id: "agp_fixture000000003".parse().unwrap(),
            title: ThreadTitle::try_from("Fixture Thread").unwrap(),
            state: ThreadState::Open,
            project_id: Some("prj_fixture000000010".parse().unwrap()),
            created_at: UnixMillis::from_millis(1_700_000_000_000),
            updated_at: UnixMillis::from_millis(1_700_000_000_005),
        };
        let dto = ThreadDto::from(&thread);
        let json = serde_json::to_string(&dto).unwrap();
        let decoded: ThreadDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, dto);
        assert_eq!(decoded.title, "Fixture Thread");
        assert_eq!(decoded.state, "open");
    }

    #[test]
    fn permission_dto_roundtrips() {
        let perm = Permission {
            event_id: "evt_fixture000000006".parse().unwrap(),
            turn_id: "trn_fixture000000002".parse().unwrap(),
            thread_id: "thr_fixture000000001".parse().unwrap(),
            kind: PermissionKind::Execute,
            description: PermissionDescription::try_from("Run command").unwrap(),
            decision: altior_domain::PermissionDecision::Approved,
            requested_at: UnixMillis::from_millis(1_700_000_000_000),
            decided_at: Some(UnixMillis::from_millis(1_700_000_000_001)),
        };
        let dto = PermissionDto::from(&perm);
        let json = serde_json::to_string(&dto).unwrap();
        let decoded: PermissionDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, dto);
        assert_eq!(decoded.decision, "approved");
    }

    #[test]
    fn harness_binding_dto_never_contains_secret_plaintext() {
        let binding = AcpHarnessBinding {
            id: "hsb_fixture000000004".parse().unwrap(),
            agent_profile_id: "agp_fixture000000003".parse().unwrap(),
            command: BoundedPath::try_from("C:\\bin\\agent.exe").unwrap(),
            label: DisplayName::try_from("Local ACP").unwrap(),
            created_at: UnixMillis::from_millis(1_700_000_000_000),
        };
        let mut dto = HarnessBindingDto::from(&binding);
        dto.secret_refs = vec!["sec_ref_openai_01".to_string()];
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("sk-"));
        assert!(json.contains("sec_ref_openai_01"));
        let decoded: HarnessBindingDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, dto);
    }
}
