//! Versioned command envelopes sent from Desktop to Core.
//!
//! Commands are active, state-changing requests directed at Core. Every command
//! carries an `operation_id` for idempotency and is versioned (ADR 0004, ADR 0006).
//!
//! Forward-compatibility rule: Command kinds form a closed enum. Unknown command
//! kinds fail explicitly with [`ProtocolError::UnsupportedCommandKind`].
//! Commands are requests, not forward-compatible observations (ADR 0004).
//!
//! Plaintext secret isolation (ADR 0006, ADR 0014): No secret plaintext ever enters
//! command payloads; configuration commands only carry opaque secret references
//! (`secret_refs`) or command metadata.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;

use altior_domain::{
    AgentProfileId, EventId, HarnessBindingId, OperationId, ProjectId, ThreadId, TurnId, UnixMillis,
};

use crate::bounded::{BoundedPayload, EnvelopeLimits, MessageText};
use crate::dto::{ThreadCursorDto, TurnCursorDto};
use crate::error::ProtocolError;
use crate::version::{ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// The command kinds defined by protocol version 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        rename_all = "snake_case"
    )
)]
pub enum CommandKind {
    /// Transport liveness check used for Core health monitoring.
    Ping,
    /// Request an initial bounded snapshot for the visible surface.
    RequestSnapshot,
    /// Cooperatively cancel the operation named in the payload.
    Cancel,
    /// Subscribe to the event stream, optionally catching up from a prior
    /// sequence (ADR 0006).
    Subscribe,
    /// Create a new conversation thread.
    CreateThread,
    /// List conversation threads with bounded pagination.
    ListThreads,
    /// Search conversation threads by title query.
    SearchThreads,
    /// Open a thread and retrieve its recent snapshot / history.
    OpenThread,
    /// Retrieve paginated turn history for a thread.
    GetHistory,
    /// Create or update an agent profile configuration.
    ConfigureAgent,
    /// Test or probe an agent harness binding without running a full turn.
    TestHarnessBinding,
    /// Dispatch a new turn prompt to an active or new harness session.
    StartTurn,
    /// Cooperatively cancel an in-flight turn within a thread.
    CancelTurn,
    /// Submit a user decision for a pending permission request.
    RespondPermission,
    /// Query Core runtime operational status.
    RuntimeStatus,
    /// Query Core runtime diagnostic summaries.
    Diagnostics,
}

impl CommandKind {
    /// Returns the canonical wire name of this kind.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::RequestSnapshot => "request_snapshot",
            Self::Cancel => "cancel",
            Self::Subscribe => "subscribe",
            Self::CreateThread => "create_thread",
            Self::ListThreads => "list_threads",
            Self::SearchThreads => "search_threads",
            Self::OpenThread => "open_thread",
            Self::GetHistory => "get_history",
            Self::ConfigureAgent => "configure_agent",
            Self::TestHarnessBinding => "test_harness_binding",
            Self::StartTurn => "start_turn",
            Self::CancelTurn => "cancel_turn",
            Self::RespondPermission => "respond_permission",
            Self::RuntimeStatus => "runtime_status",
            Self::Diagnostics => "diagnostics",
        }
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_name())
    }
}

impl FromStr for CommandKind {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ping" => Ok(Self::Ping),
            "request_snapshot" => Ok(Self::RequestSnapshot),
            "cancel" => Ok(Self::Cancel),
            "subscribe" => Ok(Self::Subscribe),
            "create_thread" => Ok(Self::CreateThread),
            "list_threads" => Ok(Self::ListThreads),
            "search_threads" => Ok(Self::SearchThreads),
            "open_thread" => Ok(Self::OpenThread),
            "get_history" => Ok(Self::GetHistory),
            "configure_agent" => Ok(Self::ConfigureAgent),
            "test_harness_binding" => Ok(Self::TestHarnessBinding),
            "start_turn" => Ok(Self::StartTurn),
            "cancel_turn" => Ok(Self::CancelTurn),
            "respond_permission" => Ok(Self::RespondPermission),
            "runtime_status" => Ok(Self::RuntimeStatus),
            "diagnostics" => Ok(Self::Diagnostics),
            other => Err(ProtocolError::UnsupportedCommandKind {
                kind: other.to_owned(),
            }),
        }
    }
}

// ── Typed command payloads ──────────────────────────────────────────

/// Payload for creating a thread (`create_thread`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CreateThreadCommand {
    /// Agent profile to associate with this thread.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub agent_profile_id: AgentProfileId,
    /// Optional thread title (up to 512 bytes).
    pub title: Option<String>,
    /// Optional project association.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub project_id: Option<ProjectId>,
}

/// Payload for listing threads with bounded pagination (`list_threads`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ListThreadsCommand {
    /// Starting cursor for pagination.
    pub cursor: Option<ThreadCursorDto>,
    /// Maximum items to return (bounded to 200).
    pub limit: Option<u32>,
}

/// Payload for searching threads (`search_threads`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct SearchThreadsCommand {
    /// Query string (bounded to 256 bytes).
    pub query: String,
    /// Maximum items to return (bounded to 200).
    pub limit: Option<u32>,
}

/// Payload for opening a thread (`open_thread`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct OpenThreadCommand {
    /// Thread to open.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Maximum turn history items to return in initial snapshot (bounded to 500).
    pub history_limit: Option<u32>,
}

/// Payload for paginating turn history (`get_history`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct GetHistoryCommand {
    /// Owning thread identity.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Cursor for turn pagination.
    pub cursor: Option<TurnCursorDto>,
    /// Maximum turns to return (bounded to 500).
    pub limit: Option<u32>,
}

/// Payload for creating or updating an agent profile (`configure_agent`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ConfigureAgentCommand {
    /// Profile ID to update, or `None` to create a new profile.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub agent_profile_id: Option<AgentProfileId>,
    /// Display name (1..=256 bytes).
    pub display_name: String,
    /// Preferred harness: `"acp"`, `"terminal"`, or `"native"`.
    pub preferred_harness: String,
    /// Memory mode: `"off"`, `"session"`, or `"long_term"`.
    pub memory_mode: String,
}

/// Payload for probing/testing a harness binding (`test_harness_binding`).
///
/// NOTE: Plaintext secrets are strictly prohibited (ADR 0006, ADR 0014).
/// `secret_refs` holds only opaque reference keys.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct TestHarnessBindingCommand {
    /// Existing harness binding ID, if probing an already saved binding.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub harness_binding_id: Option<HarnessBindingId>,
    /// Program executable path (bounded to 4096 bytes).
    pub program: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variable keys to pass.
    pub env_keys: Vec<String>,
    /// Opaque secret references (NEVER plaintext secrets).
    pub secret_refs: Vec<String>,
    /// Optional label for the binding.
    pub label: Option<String>,
}

/// Payload for starting a turn / sending a prompt (`start_turn`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct StartTurnCommand {
    /// Target conversation thread.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Client-specified turn ID, or `None` for Core to generate.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub turn_id: Option<TurnId>,
    /// Prompt text (bounded to 64 KiB).
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub prompt: MessageText,
}

/// Payload for cancelling an in-flight turn (`cancel_turn`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CancelTurnCommand {
    /// Target thread.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub thread_id: ThreadId,
    /// Specific turn to cancel, if scoped.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub turn_id: Option<TurnId>,
    /// Specific operation ID to cancel, if known.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub target_operation_id: Option<OperationId>,
}

/// Payload for submitting a user permission response (`respond_permission`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct RespondPermissionCommand {
    /// Identity of the permission request event being answered.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub event_id: EventId,
    /// Decision: `"approved"` or `"denied"`.
    pub decision: String,
}

/// Payload for querying runtime status (`runtime_status`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct RuntimeStatusCommand {
    /// Whether to include redacted diagnostics summary text.
    pub include_diagnostics: bool,
}

/// Payload for querying runtime diagnostics (`diagnostics`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct DiagnosticsCommand {
    /// Optional thread filter.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub thread_id: Option<ThreadId>,
    /// Maximum diagnostic entries to retrieve.
    pub limit: Option<u32>,
}

// ── Command Envelope ────────────────────────────────────────────────

/// A versioned command envelope. No transport is attached; encoding is a
/// plain JSON contract so any local IPC transport can carry it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CommandEnvelope {
    /// The protocol version negotiated for this connection.
    pub protocol_version: ProtocolVersion,
    /// The Altior operation this command belongs to (for idempotency).
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub operation_id: OperationId,
    /// The command discriminator.
    pub kind: CommandKind,
    /// Optional bounded JSON payload.
    #[cfg_attr(feature = "dto-export", ts(type = "unknown"))]
    pub payload: Option<BoundedPayload>,
    /// When the sender issued the command. Supplied by the sender's clock;
    /// fixtures use constants.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub issued_at: UnixMillis,
}

impl CommandEnvelope {
    /// Decodes a command envelope from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the input is not
    /// a valid envelope, including unknown command kinds.
    pub fn from_json(input: &str) -> Result<Self, ProtocolError> {
        Ok(serde_json::from_str(input)?)
    }

    /// Encodes the envelope as deterministic JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the envelope
    /// cannot be encoded.
    pub fn to_json(&self) -> Result<String, ProtocolError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Helper to construct a typed command envelope.
    fn new_typed<T: Serialize>(
        kind: CommandKind,
        value: &T,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let json_value = serde_json::to_value(value)?;
        let payload = BoundedPayload::new(json_value, limits.payload_bytes)?;
        Ok(Self {
            protocol_version: ProtocolVersion::V1,
            operation_id,
            kind,
            payload: Some(payload),
            issued_at,
        })
    }

    /// Parses the bounded payload into a typed command struct.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if the payload is missing
    /// or cannot be decoded as `T`.
    pub fn parse_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, ProtocolError> {
        let malformed = |msg: &'static str| ProtocolError::MalformedEnvelope {
            source: serde_json::Error::custom(msg),
        };
        let payload = self
            .payload
            .as_ref()
            .ok_or_else(|| malformed("command envelope missing payload"))?;
        serde_json::from_value(payload.value().clone())
            .map_err(|source| ProtocolError::MalformedEnvelope { source })
    }

    /// Builds a `ping` command.
    #[must_use]
    pub fn ping(operation_id: OperationId, issued_at: UnixMillis) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            operation_id,
            kind: CommandKind::Ping,
            payload: None,
            issued_at,
        }
    }

    /// Builds a cooperative cancellation command for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the payload exceeds
    /// `limits.payload_bytes`.
    pub fn cancel(
        target: &OperationId,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let payload = BoundedPayload::new(
            json!({ "operation_id": target.as_str() }),
            limits.payload_bytes,
        )?;
        Ok(Self {
            protocol_version: ProtocolVersion::V1,
            operation_id,
            kind: CommandKind::Cancel,
            payload: Some(payload),
            issued_at,
        })
    }

    /// Returns the target operation of a `cancel` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the command is a
    /// `cancel` whose payload is missing or does not carry a well-formed
    /// `operation_id`. Non-cancel commands return `Ok(None)`.
    pub fn cancel_target(&self) -> Result<Option<OperationId>, ProtocolError> {
        if self.kind != CommandKind::Cancel {
            return Ok(None);
        }
        let malformed = |message: &'static str| ProtocolError::MalformedEnvelope {
            source: serde_json::Error::custom(message),
        };
        let payload = self
            .payload
            .as_ref()
            .ok_or_else(|| malformed("cancel command without payload"))?;
        let target = payload
            .value()
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| malformed("cancel payload without operation_id"))?;
        let parsed =
            OperationId::try_from(target).map_err(|_| malformed("invalid operation id"))?;
        Ok(Some(parsed))
    }

    /// Builds an event-stream subscription command (ADR 0006).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the payload exceeds
    /// `limits.payload_bytes`.
    pub fn subscribe(
        since: Option<crate::event::Sequence>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let payload = BoundedPayload::new(
            json!({ "since": since.map(crate::event::Sequence::as_u64) }),
            limits.payload_bytes,
        )?;
        Ok(Self {
            protocol_version: ProtocolVersion::V1,
            operation_id,
            kind: CommandKind::Subscribe,
            payload: Some(payload),
            issued_at,
        })
    }

    /// Returns the subscription catch-up point of a `subscribe` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the command is a
    /// `subscribe` whose payload is missing, carries a non-numeric or
    /// zero `since`, or is otherwise malformed. Non-subscribe commands
    /// return `Ok(None)`; a `{"since": null}` payload returns
    /// `Ok(Some(None))` meaning "from now".
    pub fn subscribe_since(&self) -> Result<Option<Option<crate::event::Sequence>>, ProtocolError> {
        if self.kind != CommandKind::Subscribe {
            return Ok(None);
        }
        let malformed = |message: &'static str| ProtocolError::MalformedEnvelope {
            source: serde_json::Error::custom(message),
        };
        let payload = self
            .payload
            .as_ref()
            .ok_or_else(|| malformed("subscribe command without payload"))?;
        let Some(since) = payload.value().get("since") else {
            return Err(malformed("subscribe payload without since"));
        };
        if since.is_null() {
            return Ok(Some(None));
        }
        let raw = since
            .as_u64()
            .ok_or_else(|| malformed("subscribe since is not a sequence number"))?;
        let sequence = crate::event::Sequence::try_new(raw)
            .map_err(|_| malformed("subscribe since is not a sequence number"))?;
        Ok(Some(Some(sequence)))
    }

    /// Builds a `create_thread` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization or bounds check fails.
    pub fn create_thread(
        agent_profile_id: AgentProfileId,
        title: Option<String>,
        project_id: Option<ProjectId>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = CreateThreadCommand {
            agent_profile_id,
            title,
            project_id,
        };
        Self::new_typed(
            CommandKind::CreateThread,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `create_thread` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn create_thread_payload(&self) -> Result<CreateThreadCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `list_threads` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization or bounds check fails.
    pub fn list_threads(
        cursor: Option<ThreadCursorDto>,
        limit: Option<u32>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = ListThreadsCommand { cursor, limit };
        Self::new_typed(
            CommandKind::ListThreads,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `list_threads` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn list_threads_payload(&self) -> Result<ListThreadsCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `search_threads` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if query exceeds 256 bytes or bounds check fails.
    pub fn search_threads(
        query: String,
        limit: Option<u32>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        if query.len() > 256 {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: query.len(),
                limit_bytes: 256,
            });
        }
        let cmd = SearchThreadsCommand { query, limit };
        Self::new_typed(
            CommandKind::SearchThreads,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `search_threads` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn search_threads_payload(&self) -> Result<SearchThreadsCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds an `open_thread` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn open_thread(
        thread_id: ThreadId,
        history_limit: Option<u32>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = OpenThreadCommand {
            thread_id,
            history_limit,
        };
        Self::new_typed(
            CommandKind::OpenThread,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of an `open_thread` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn open_thread_payload(&self) -> Result<OpenThreadCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `get_history` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn get_history(
        thread_id: ThreadId,
        cursor: Option<TurnCursorDto>,
        limit: Option<u32>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = GetHistoryCommand {
            thread_id,
            cursor,
            limit,
        };
        Self::new_typed(
            CommandKind::GetHistory,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `get_history` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn get_history_payload(&self) -> Result<GetHistoryCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `configure_agent` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if display name exceeds 256 bytes or bounds check fails.
    pub fn configure_agent(
        agent_profile_id: Option<AgentProfileId>,
        display_name: String,
        preferred_harness: String,
        memory_mode: String,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        if display_name.len() > 256 {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: display_name.len(),
                limit_bytes: 256,
            });
        }
        let cmd = ConfigureAgentCommand {
            agent_profile_id,
            display_name,
            preferred_harness,
            memory_mode,
        };
        Self::new_typed(
            CommandKind::ConfigureAgent,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `configure_agent` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn configure_agent_payload(&self) -> Result<ConfigureAgentCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `test_harness_binding` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if program path exceeds 4096 bytes or bounds check fails.
    #[allow(clippy::too_many_arguments)]
    pub fn test_harness_binding(
        harness_binding_id: Option<HarnessBindingId>,
        program: String,
        args: Vec<String>,
        env_keys: Vec<String>,
        secret_refs: Vec<String>,
        label: Option<String>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        if program.len() > 4096 {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: program.len(),
                limit_bytes: 4096,
            });
        }
        let cmd = TestHarnessBindingCommand {
            harness_binding_id,
            program,
            args,
            env_keys,
            secret_refs,
            label,
        };
        Self::new_typed(
            CommandKind::TestHarnessBinding,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `test_harness_binding` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn test_harness_binding_payload(&self) -> Result<TestHarnessBindingCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `start_turn` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if prompt exceeds bounds or payload is too large.
    pub fn start_turn(
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        prompt: MessageText,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = StartTurnCommand {
            thread_id,
            turn_id,
            prompt,
        };
        Self::new_typed(
            CommandKind::StartTurn,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `start_turn` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn start_turn_payload(&self) -> Result<StartTurnCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `cancel_turn` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn cancel_turn(
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        target_operation_id: Option<OperationId>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = CancelTurnCommand {
            thread_id,
            turn_id,
            target_operation_id,
        };
        Self::new_typed(
            CommandKind::CancelTurn,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `cancel_turn` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn cancel_turn_payload(&self) -> Result<CancelTurnCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `respond_permission` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn respond_permission(
        event_id: EventId,
        decision: String,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = RespondPermissionCommand { event_id, decision };
        Self::new_typed(
            CommandKind::RespondPermission,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `respond_permission` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn respond_permission_payload(&self) -> Result<RespondPermissionCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `runtime_status` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn runtime_status(
        include_diagnostics: bool,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = RuntimeStatusCommand {
            include_diagnostics,
        };
        Self::new_typed(
            CommandKind::RuntimeStatus,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `runtime_status` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn runtime_status_payload(&self) -> Result<RuntimeStatusCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Builds a `diagnostics` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if payload serialization fails.
    pub fn diagnostics(
        thread_id: Option<ThreadId>,
        limit: Option<u32>,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let cmd = DiagnosticsCommand { thread_id, limit };
        Self::new_typed(
            CommandKind::Diagnostics,
            &cmd,
            operation_id,
            issued_at,
            limits,
        )
    }

    /// Extracts the payload of a `diagnostics` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] if decoding fails.
    pub fn diagnostics_payload(&self) -> Result<DiagnosticsCommand, ProtocolError> {
        self.parse_payload()
    }

    /// Validates the envelope against `limits` and the locally supported
    /// protocol versions.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnsupportedProtocolVersion`] when the
    /// envelope's version is outside [`SUPPORTED_PROTOCOL_VERSIONS`] and
    /// [`ProtocolError::PayloadTooLarge`] when the encoded payload exceeds
    /// the limit.
    pub fn validate(&self, limits: &EnvelopeLimits) -> Result<(), ProtocolError> {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(self.protocol_version) {
            return Err(ProtocolError::UnsupportedProtocolVersion {
                requested: self.protocol_version.as_u32(),
                supported: SUPPORTED_PROTOCOL_VERSIONS,
            });
        }
        if let Some(payload) = &self.payload {
            payload.ensure_within(limits.payload_bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_commands_carry_their_target_in_the_payload() {
        let target: OperationId = "op_fixture000000005".parse().unwrap();
        let envelope = CommandEnvelope::cancel(
            &target,
            "op_fixture000000011".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_000),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        assert_eq!(envelope.kind, CommandKind::Cancel);
        assert_eq!(
            envelope
                .cancel_target()
                .unwrap()
                .map(|id| id.as_str().to_owned()),
            Some("op_fixture000000005".to_owned())
        );

        let json = envelope.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn create_thread_command_roundtrips() {
        let envelope = CommandEnvelope::create_thread(
            "agp_fixture000000003".parse().unwrap(),
            Some("New Thread".to_string()),
            Some("prj_fixture000000010".parse().unwrap()),
            "op_fixture000000020".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_001),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        assert_eq!(envelope.kind, CommandKind::CreateThread);
        let payload = envelope.create_thread_payload().unwrap();
        assert_eq!(payload.agent_profile_id.as_str(), "agp_fixture000000003");
        assert_eq!(payload.title.as_deref(), Some("New Thread"));
        assert_eq!(
            payload
                .project_id
                .as_ref()
                .map(altior_domain::ProjectId::as_str),
            Some("prj_fixture000000010")
        );

        let json = envelope.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn start_turn_command_roundtrips() {
        let envelope = CommandEnvelope::start_turn(
            "thr_fixture000000001".parse().unwrap(),
            Some("trn_fixture000000002".parse().unwrap()),
            MessageText::try_from("Write a function").unwrap(),
            "op_fixture000000021".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_002),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        assert_eq!(envelope.kind, CommandKind::StartTurn);
        let payload = envelope.start_turn_payload().unwrap();
        assert_eq!(payload.thread_id.as_str(), "thr_fixture000000001");
        assert_eq!(payload.prompt.as_str(), "Write a function");

        let json = envelope.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn test_harness_binding_command_carries_only_opaque_secret_refs() {
        let envelope = CommandEnvelope::test_harness_binding(
            Some("hsb_fixture000000004".parse().unwrap()),
            "agent.exe".to_string(),
            vec!["--stdio".to_string()],
            vec!["KEY".to_string()],
            vec!["sec_ref_key1".to_string()],
            Some("Test Probe".to_string()),
            "op_fixture000000022".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_003),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        let payload = envelope.test_harness_binding_payload().unwrap();
        assert_eq!(payload.secret_refs, vec!["sec_ref_key1".to_string()]);
        assert_eq!(payload.program, "agent.exe");

        let json = envelope.to_json().unwrap();
        assert!(!json.contains("sk-"));
        assert!(json.contains("sec_ref_key1"));
    }

    #[test]
    fn respond_permission_command_roundtrips() {
        let envelope = CommandEnvelope::respond_permission(
            "evt_fixture000000006".parse().unwrap(),
            "approved".to_string(),
            "op_fixture000000023".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_004),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        let payload = envelope.respond_permission_payload().unwrap();
        assert_eq!(payload.event_id.as_str(), "evt_fixture000000006");
        assert_eq!(payload.decision, "approved");
    }

    #[test]
    fn subscribe_commands_carry_their_catch_up_mode() {
        let from_now = CommandEnvelope::subscribe(
            None,
            "op_fixture000000012".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_005),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        from_now.validate(&EnvelopeLimits::default()).unwrap();
        assert_eq!(from_now.subscribe_since().unwrap(), Some(None));
        assert!(from_now.to_json().unwrap().contains(r#""since":null"#));

        let catch_up = CommandEnvelope::subscribe(
            Some(crate::event::Sequence::try_new(5).unwrap()),
            "op_fixture000000012".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_005),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        assert_eq!(
            catch_up.subscribe_since().unwrap(),
            Some(Some(crate::event::Sequence::try_new(5).unwrap()))
        );
        assert!(catch_up.to_json().unwrap().contains(r#""since":5"#));

        let json = catch_up.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, catch_up);
    }

    #[test]
    fn malformed_subscribe_payloads_fail_explicitly() {
        let no_payload = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: "op_fixture000000012".parse().unwrap(),
            kind: CommandKind::Subscribe,
            payload: None,
            issued_at: UnixMillis::from_millis(0),
        };
        assert!(matches!(
            no_payload.subscribe_since(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));

        let zero_since = CommandEnvelope {
            payload: Some(
                BoundedPayload::new(
                    serde_json::json!({"since": 0}),
                    EnvelopeLimits::default().payload_bytes,
                )
                .unwrap(),
            ),
            ..no_payload.clone()
        };
        assert!(matches!(
            zero_since.subscribe_since(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));

        let ping = CommandEnvelope {
            kind: CommandKind::Ping,
            payload: None,
            ..no_payload
        };
        assert_eq!(ping.subscribe_since().unwrap(), None);
    }

    #[test]
    fn malformed_cancel_payloads_fail_explicitly() {
        let no_payload = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: "op_fixture000000011".parse().unwrap(),
            kind: CommandKind::Cancel,
            payload: None,
            issued_at: UnixMillis::from_millis(0),
        };
        assert!(matches!(
            no_payload.cancel_target(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));

        let bad_target = CommandEnvelope {
            payload: Some(
                BoundedPayload::new(
                    serde_json::json!({"operation_id": "not-an-id"}),
                    EnvelopeLimits::default().payload_bytes,
                )
                .unwrap(),
            ),
            ..no_payload
        };
        assert!(matches!(
            bad_target.cancel_target(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));
    }
}
