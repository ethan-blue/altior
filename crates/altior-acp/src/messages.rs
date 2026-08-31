//! The ACP v1 message subset this adapter maps (ADR 0007).
//!
//! Shapes follow the published v1 JSON Schema. Every boolean capability
//! defaults to `false` and every unknown field is ignored, so agents that
//! omit or extend the schema parse without surprises. Anything outside
//! the modeled kinds survives as [`SessionUpdate::Preserved`] with the
//! raw JSON attached — dropped data is a contract violation, not an
//! acceptable degradation.
//!
//! ACP session ids are opaque foreign strings; they are deliberately not
//! domain ids (ADR 0004 ids are Altior-owned).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AcpError;
use crate::wire::RpcId;

/// `initialize` request parameters sent by Altior.
#[derive(Debug, Serialize)]
pub struct InitializeParams {
    /// The protocol version Altior speaks; recorded, never branched on.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u16,
    /// What Altior (the ACP client) supports.
    #[serde(rename = "clientCapabilities")]
    pub client_capabilities: ClientCapabilities,
    /// Altior's own implementation identity.
    #[serde(rename = "clientInfo")]
    pub client_info: Option<Implementation>,
}

/// Client capabilities Altior advertises. The spike grants the agent
/// nothing: no filesystem, no terminal, no elicitation — the P4
/// workbench owns file access behind permission profiles.
#[derive(Debug, Serialize)]
pub struct ClientCapabilities {
    /// Filesystem capabilities (all denied).
    pub fs: FileSystemCapabilities,
}

/// Which `fs/*` methods the client answers.
#[derive(Debug, Serialize)]
pub struct FileSystemCapabilities {
    /// Whether `fs/read_text_file` is served.
    #[serde(rename = "readTextFile")]
    pub read_text_file: bool,
    /// Whether `fs/write_text_file` is served.
    #[serde(rename = "writeTextFile")]
    pub write_text_file: bool,
}

/// A `name`/`version` implementation identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Implementation {
    /// Product name, e.g. `altior-desktop`.
    pub name: String,
    /// Product version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The `initialize` result: capabilities, not version strings, are the
/// negotiation surface (ADR 0007).
#[derive(Debug, Deserialize)]
pub struct InitializeResult {
    /// The agent's chosen protocol version; recorded, never branched on.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u16,
    /// What the agent supports.
    #[serde(rename = "agentCapabilities", default)]
    pub agent_capabilities: AgentCapabilities,
    /// The agent's identity, when it declares one.
    #[serde(rename = "agentInfo", default)]
    pub agent_info: Option<Implementation>,
}

/// The agent-capability subset this adapter negotiates on.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentCapabilities {
    /// Whether `session/load` may be used (resume).
    #[serde(rename = "loadSession", default)]
    pub load_session: bool,
    /// Which prompt content kinds the agent accepts.
    #[serde(rename = "promptCapabilities", default)]
    pub prompt_capabilities: PromptCapabilities,
    /// Session capability flags; only `resume` is modeled.
    #[serde(rename = "sessionCapabilities", default)]
    pub session_capabilities: SessionCapabilities,
    /// Whether `session/steer` is supported.
    #[serde(default)]
    pub steer: bool,
}

/// Which prompt content kinds the agent accepts beyond plain text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct PromptCapabilities {
    /// Image blocks in prompts.
    #[serde(default)]
    pub image: bool,
    /// Audio blocks in prompts.
    #[serde(default)]
    pub audio: bool,
    /// Embedded resource blocks in prompts.
    #[serde(rename = "embeddedContext", default)]
    pub embedded_context: bool,
}

/// The session-capability subset that matters here.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct SessionCapabilities {
    /// Whether `session/resume` (the newer resume path) is available.
    #[serde(default)]
    pub resume: bool,
    /// Whether prompt steering is supported via session/steer.
    #[serde(default)]
    pub steer: bool,
}

/// `session/new` request parameters.
#[derive(Debug, Serialize)]
pub struct NewSessionParams {
    /// The absolute working directory for the session.
    pub cwd: String,
    /// MCP servers; the spike always sends none.
    #[serde(rename = "mcpServers")]
    pub mcp_servers: Vec<Value>,
}

/// `session/new` result.
#[derive(Debug, Deserialize)]
pub struct NewSessionResult {
    /// The agent-assigned opaque session id.
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// `session/load` request parameters (gated on `loadSession`).
#[derive(Debug, Serialize)]
pub struct LoadSessionParams {
    /// The working directory for the resumed session.
    pub cwd: String,
    /// MCP servers; the spike always sends none.
    #[serde(rename = "mcpServers")]
    pub mcp_servers: Vec<Value>,
    /// The previously issued session id.
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// One prompt content block. The adapter builds text blocks; agents may
/// send any kind in chunks, so all five v1 kinds decode.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text {
        /// The text.
        text: String,
    },
    /// A link to an out-of-band resource.
    ResourceLink {
        /// The resource URI.
        uri: String,
        /// The human-readable name.
        name: String,
    },
    /// Inline image data.
    Image {
        /// Base64 payload.
        data: String,
        /// MIME type.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// Inline audio data.
    Audio {
        /// Base64 payload.
        data: String,
        /// MIME type.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// An embedded resource, preserved verbatim.
    Resource {
        /// The embedded resource object.
        resource: Value,
    },
}

/// `session/prompt` request parameters.
#[derive(Debug, Serialize)]
pub struct PromptParams {
    /// The session to prompt.
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// The prompt content.
    pub prompt: Vec<ContentBlock>,
}

/// Why a turn stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StopReason {
    /// The turn ended successfully.
    #[serde(rename = "end_turn")]
    EndTurn,
    /// The token budget was exhausted.
    #[serde(rename = "max_tokens")]
    MaxTokens,
    /// The agent-request budget was exhausted.
    #[serde(rename = "max_turn_requests")]
    MaxTurnRequests,
    /// The agent refused to continue.
    #[serde(rename = "refusal")]
    Refusal,
    /// The client cancelled via `session/cancel`.
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl StopReason {
    /// Whether the stop reason is a successful completion of the turn.
    #[must_use]
    pub fn is_success(self) -> bool {
        matches!(self, Self::EndTurn | Self::Cancelled)
    }

    /// The wire name.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
        }
    }
}

/// `session/prompt` result.
#[derive(Debug, Deserialize)]
pub struct PromptResult {
    /// Why the turn stopped.
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
}

/// `session/cancel` notification parameters.
#[derive(Debug, Serialize)]
pub struct CancelParams {
    /// The session whose current turn is cancelled.
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Execution status of a tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Not started (input streaming or approval pending).
    Pending,
    /// Currently running.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
}

impl ToolCallStatus {
    /// The wire name.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// The tool-call snapshot the adapter maps: identity plus status.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ToolCallUpdate {
    /// The tool call id.
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    /// The execution status, when the update carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolCallStatus>,
}

/// A streamed content chunk (`user_message_chunk`, `agent_message_chunk`,
/// `agent_thought_chunk`).
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ContentChunk {
    /// The chunk's content.
    pub content: ContentBlock,
    /// Chunks of one message share an id; a change starts a new message.
    #[serde(rename = "messageId", default)]
    pub message_id: Option<String>,
}

/// One `session/update` update, with unknown kinds preserved verbatim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionUpdate {
    /// A chunk of the user's message echoed back.
    UserMessageChunk(ContentChunk),
    /// A chunk of the agent's reply.
    AgentMessageChunk(ContentChunk),
    /// A chunk of the agent's internal reasoning.
    AgentThoughtChunk(ContentChunk),
    /// A new tool call started.
    ToolCall(ToolCallUpdate),
    /// A tool call's status changed.
    ToolCallUpdate(ToolCallUpdate),
    /// Anything else, kept verbatim (plans, commands, modes, usage, …).
    Preserved {
        /// The `sessionUpdate` kind name.
        kind: String,
        /// The raw update object.
        raw: Value,
    },
}

impl SessionUpdate {
    /// Decodes one update object, dispatching on `sessionUpdate` and
    /// preserving unknown kinds.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::MalformedMessage`] when a modeled kind does
    /// not match its v1 shape.
    pub fn from_value(update: &Value) -> Result<Self, AcpError> {
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .ok_or_else(|| AcpError::MalformedMessage {
                diagnostic: "session/update carries no sessionUpdate kind".to_owned(),
            })?
            .to_owned();
        match kind.as_str() {
            "user_message_chunk" => Ok(Self::UserMessageChunk(serde_json::from_value(
                update.clone(),
            )?)),
            "agent_message_chunk" => Ok(Self::AgentMessageChunk(serde_json::from_value(
                update.clone(),
            )?)),
            "agent_thought_chunk" => Ok(Self::AgentThoughtChunk(serde_json::from_value(
                update.clone(),
            )?)),
            "tool_call" | "tool_call_update" => {
                let tool_call = serde_json::from_value::<ToolCallUpdate>(update.clone())?;
                if kind == "tool_call" {
                    Ok(Self::ToolCall(tool_call))
                } else {
                    Ok(Self::ToolCallUpdate(tool_call))
                }
            }
            _ => Ok(Self::Preserved {
                kind,
                raw: update.clone(),
            }),
        }
    }

    /// The `sessionUpdate` kind name.
    #[must_use]
    pub fn kind_name(&self) -> &str {
        match self {
            Self::UserMessageChunk(_) => "user_message_chunk",
            Self::AgentMessageChunk(_) => "agent_message_chunk",
            Self::AgentThoughtChunk(_) => "agent_thought_chunk",
            Self::ToolCall(_) => "tool_call",
            Self::ToolCallUpdate(_) => "tool_call_update",
            Self::Preserved { kind, .. } => kind,
        }
    }
}

/// `session/request_permission` parameters (agent → client).
#[derive(Debug, Deserialize)]
pub struct RequestPermissionParams {
    /// The choices offered to the user.
    pub options: Vec<PermissionOption>,
    /// The session the tool call belongs to.
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// The tool call awaiting approval.
    #[serde(rename = "toolCall")]
    pub tool_call: ToolCallUpdate,
}

/// One permission choice.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct PermissionOption {
    /// The machine id to echo back when selected.
    #[serde(rename = "optionId")]
    pub option_id: String,
    /// The human-readable label.
    pub name: String,
}

/// The `session/request_permission` answer Altior sends on cancellation:
/// `{"outcome":"cancelled"}` per the v1 cancellation contract.
#[derive(Debug, Serialize)]
pub struct CancelledPermissionOutcome {
    /// Always `cancelled`.
    pub outcome: CancelledOutcome,
}

/// The `cancelled` outcome marker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelledOutcome {
    /// The turn was cancelled before the user answered.
    Cancelled,
}

/// The typed refusal Altior answers to `fs/*` requests in this spike:
/// the method exists but Desktop grants no filesystem yet (P4).
#[derive(Debug, Serialize)]
pub struct FsRefusal {
    /// The JSON-RPC error object.
    pub error: FsRefusalError,
}

/// The error payload of an fs refusal.
#[derive(Debug, Serialize)]
pub struct FsRefusalError {
    /// `-32601`: we deliberately present filesystem access as absent.
    pub code: i64,
    /// The human-readable reason.
    pub message: String,
}

/// The methods this adapter sends or serves, with exact wire names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    /// `initialize`
    Initialize,
    /// `session/new`
    NewSession,
    /// `session/load`
    LoadSession,
    /// `session/prompt`
    Prompt,
    /// `session/steer`
    Steer,
    /// `session/cancel`
    Cancel,
    /// `session/update` (notification, agent → client)
    SessionUpdate,
    /// `session/request_permission` (request, agent → client)
    RequestPermission,
    /// `fs/read_text_file` (request, agent → client)
    FsReadTextFile,
    /// `fs/write_text_file` (request, agent → client)
    FsWriteTextFile,
}

impl Method {
    /// The wire name.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::NewSession => "session/new",
            Self::LoadSession => "session/load",
            Self::Prompt => "session/prompt",
            Self::Steer => "session/steer",
            Self::Cancel => "session/cancel",
            Self::SessionUpdate => "session/update",
            Self::RequestPermission => "session/request_permission",
            Self::FsReadTextFile => "fs/read_text_file",
            Self::FsWriteTextFile => "fs/write_text_file",
        }
    }

    /// Parses a wire method name into the modeled set.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "initialize" => Self::Initialize,
            "session/new" => Self::NewSession,
            "session/load" => Self::LoadSession,
            "session/prompt" => Self::Prompt,
            "session/steer" => Self::Steer,
            "session/cancel" => Self::Cancel,
            "session/update" => Self::SessionUpdate,
            "session/request_permission" => Self::RequestPermission,
            "fs/read_text_file" => Self::FsReadTextFile,
            "fs/write_text_file" => Self::FsWriteTextFile,
            _ => return None,
        })
    }
}

/// The next request id for a message Altior sends.
#[derive(Debug, Default)]
pub struct IdSource {
    next: u64,
}

impl IdSource {
    /// Creates a fresh id source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates the next numeric id.
    pub fn allocate(&mut self) -> RpcId {
        self.next += 1;
        RpcId::Number(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_results_decode_with_defaults_and_renames() {
        let minimal: InitializeResult = serde_json::from_str(r#"{"protocolVersion":1}"#).unwrap();
        assert_eq!(minimal.protocol_version, 1);
        assert!(!minimal.agent_capabilities.load_session);

        let capable: InitializeResult = serde_json::from_str(
            r#"{"protocolVersion":1,"agentCapabilities":{"loadSession":true,
               "promptCapabilities":{"image":true,"audio":false,"embeddedContext":true},
               "sessionCapabilities":{"resume":true}},"agentInfo":{"name":"agent-alpha","version":"9.9"}}"#,
        )
        .unwrap();
        assert!(capable.agent_capabilities.load_session);
        assert!(capable.agent_capabilities.session_capabilities.resume);
        assert!(
            capable
                .agent_capabilities
                .prompt_capabilities
                .embedded_context
        );
        assert_eq!(
            capable.agent_info.as_ref().map(|info| info.name.as_str()),
            Some("agent-alpha")
        );
    }

    #[test]
    fn session_updates_dispatch_and_preserve_unknown_kinds() {
        let update =
            SessionUpdate::from_value(&serde_json::json!({"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"Hel"},"messageId":"m1"}))
            .unwrap();
        assert!(matches!(update, SessionUpdate::AgentMessageChunk(_)));
        assert_eq!(update.kind_name(), "agent_message_chunk");

        let tool =
            SessionUpdate::from_value(&serde_json::json!({"sessionUpdate":"tool_call_update",
                "toolCallId":"tc_1","status":"in_progress"}))
            .unwrap();
        assert_eq!(
            tool,
            SessionUpdate::ToolCallUpdate(ToolCallUpdate {
                tool_call_id: "tc_1".to_owned(),
                status: Some(ToolCallStatus::InProgress),
            })
        );

        let unknown = SessionUpdate::from_value(
            &serde_json::json!({"sessionUpdate":"usage_update","used":100,"size":2000}),
        )
        .unwrap();
        let SessionUpdate::Preserved { kind, raw } = unknown else {
            panic!("usage_update must be preserved, not dropped");
        };
        assert_eq!(kind, "usage_update");
        assert_eq!(raw.get("used"), Some(&serde_json::json!(100)));
    }

    #[test]
    fn permission_params_decode_the_v1_shape() {
        let params: RequestPermissionParams = serde_json::from_value(serde_json::json!({
            "options": [
                {"optionId":"allow_once","name":"Allow","kind":"allow_once"},
                {"optionId":"reject_once","name":"Deny","kind":"reject_once"}
            ],
            "sessionId": "s1",
            "toolCall": {"toolCallId": "tc_9", "status": "pending"}
        }))
        .unwrap();
        assert_eq!(params.options.len(), 2);
        assert_eq!(params.options[0].option_id, "allow_once");
        assert_eq!(params.tool_call.tool_call_id, "tc_9");
    }
}
