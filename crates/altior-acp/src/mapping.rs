//! The ADR 0007 mapping table, executable: ACP wire messages become
//! adapter events, adapter events become protocol event bodies, and raw
//! traces normalize deterministically.
//!
//! Turn lifecycle maps onto the existing known events (`turn.started`,
//! `message.delta`, `turn.completed`); everything else — failures, tool
//! calls, permission requests, unmapped updates — survives as the bounded
//! preserved form with an `acp.` provider kind. Nothing is dropped.

use serde_json::Value;

use altior_protocol::{DiagnosticText, EventBody, KnownEvent, MessageText};

use crate::error::AcpError;
use crate::messages::{ContentBlock, NewSessionResult, PromptResult, SessionUpdate, StopReason};
use crate::wire::RpcMessage;

/// One normalized adapter event: what the wire said, in Altior's
/// vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    /// A `session/new` result bound a session id.
    SessionCreated {
        /// The agent-assigned opaque session id.
        session_id: String,
    },
    /// A `session/load` result resumed a session.
    SessionResumed {
        /// The resumed session id.
        session_id: String,
    },
    /// A prompt request was written; the turn implicitly started.
    TurnStarted,
    /// A streamed agent text delta.
    Delta {
        /// The delta text.
        text: String,
    },
    /// A tool call started or changed status.
    ToolObserved {
        /// The tool call id.
        tool_call_id: String,
        /// The status, when the update carried one.
        status: Option<String>,
    },
    /// The agent asked the user to approve a tool call.
    PermissionRequested {
        /// The agent's request id, echoed back in the answer.
        request_id: String,
        /// The tool call awaiting approval, when known.
        tool_call_id: Option<String>,
    },
    /// The prompt response arrived; the turn completed for this reason.
    TurnCompleted {
        /// Why the turn stopped.
        stop_reason: StopReason,
    },
    /// The turn failed: a refusal, budget exhaustion, or an RPC error.
    TurnFailed {
        /// A bounded diagnostic naming the failure.
        diagnostic: String,
    },
    /// Anything mapped by preservation instead of a known kind.
    Preserved {
        /// The `acp.` provider kind, e.g. `acp.tool`.
        provider_kind: String,
        /// The raw wire fragment, bounded on conversion.
        raw: Value,
    },
}

impl AgentEvent {
    /// The event as a protocol [`EventBody`]: known kinds where the
    /// vocabulary has them, the bounded preserved form otherwise.
    /// Oversized content truncates at a character boundary — nothing is
    /// ever dropped or rejected for size.
    #[must_use]
    pub fn to_event_body(&self) -> EventBody {
        match self {
            Self::SessionCreated { .. } | Self::SessionResumed { .. } => {
                // Thread/session bindings are P1 domain state; the wire
                // fact is preserved until then.
                EventBody::Unknown {
                    provider_kind: self.provider_kind(),
                    diagnostic: bounded_diagnostic(&self.raw_json()),
                }
            }
            Self::TurnStarted => EventBody::Known(KnownEvent::TurnStarted),
            Self::Delta { text } => EventBody::Known(KnownEvent::MessageDelta {
                text: truncate_to_message(text),
            }),
            Self::ToolObserved {
                tool_call_id,
                status,
            } => EventBody::Unknown {
                provider_kind: "acp.tool".to_owned(),
                diagnostic: bounded_diagnostic(&serde_json::json!({
                    "toolCallId": tool_call_id,
                    "status": status,
                })),
            },
            Self::PermissionRequested {
                request_id,
                tool_call_id,
            } => EventBody::Unknown {
                provider_kind: "acp.permission.requested".to_owned(),
                diagnostic: bounded_diagnostic(&serde_json::json!({
                    "requestId": request_id,
                    "toolCallId": tool_call_id,
                })),
            },
            Self::TurnCompleted { stop_reason } => {
                // `cancelled` completes the turn exactly like `end_turn`;
                // the cancellation cause lives in the adapter and delivery
                // state (ADR 0007, ADR 0006 ownership model).
                let _ = stop_reason;
                EventBody::Known(KnownEvent::TurnCompleted)
            }
            Self::TurnFailed { diagnostic } => EventBody::Unknown {
                provider_kind: "acp.turn.failed".to_owned(),
                diagnostic: bounded_diagnostic(&serde_json::json!({
                    "diagnostic": diagnostic,
                })),
            },
            Self::Preserved { provider_kind, raw } => EventBody::Unknown {
                provider_kind: provider_kind.clone(),
                diagnostic: bounded_diagnostic(raw),
            },
        }
    }

    /// The `acp.` provider kind this event preserves as, when it is not a
    /// known event.
    #[must_use]
    pub fn provider_kind(&self) -> String {
        match self {
            Self::SessionCreated { .. } => "acp.session.created".to_owned(),
            Self::SessionResumed { .. } => "acp.session.resumed".to_owned(),
            Self::TurnStarted | Self::Delta { .. } | Self::TurnCompleted { .. } => {
                "acp.turn".to_owned()
            }
            Self::ToolObserved { .. } => "acp.tool".to_owned(),
            Self::PermissionRequested { .. } => "acp.permission.requested".to_owned(),
            Self::TurnFailed { .. } => "acp.turn.failed".to_owned(),
            Self::Preserved { provider_kind, .. } => provider_kind.clone(),
        }
    }

    /// The raw wire fragment this event carries or was derived from, for
    /// diagnostics and preserved bodies.
    #[must_use]
    pub fn raw_json(&self) -> Value {
        match self {
            Self::SessionCreated { session_id } | Self::SessionResumed { session_id } => {
                serde_json::json!({ "sessionId": session_id })
            }
            Self::TurnStarted => Value::Null,
            Self::Delta { text } => serde_json::json!({ "text": text }),
            Self::ToolObserved {
                tool_call_id,
                status,
            } => serde_json::json!({ "toolCallId": tool_call_id, "status": status }),
            Self::PermissionRequested {
                request_id,
                tool_call_id,
            } => serde_json::json!({ "requestId": request_id, "toolCallId": tool_call_id }),
            Self::TurnCompleted { stop_reason } => {
                serde_json::json!({ "stopReason": stop_reason.wire_name() })
            }
            Self::TurnFailed { diagnostic } => {
                serde_json::json!({ "diagnostic": diagnostic })
            }
            Self::Preserved { raw, .. } => raw.clone(),
        }
    }
}

/// One entry of a normalized trace: the wire kind plus the event it
/// became.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedEvent {
    /// The wire fact, e.g. `session/update:agent_message_chunk`.
    pub wire_kind: String,
    /// The normalized adapter event.
    pub event: AgentEvent,
}

impl NormalizedEvent {
    /// The fixture/smoke-report form: `{"wire_kind": …, "body": …}` where
    /// `body` is the protocol event body JSON.
    ///
    /// # Panics
    ///
    /// Panics when the protocol body cannot serialize — impossible for
    /// the derived serialization of [`EventBody`].
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "wire_kind": self.wire_kind,
            "body": serde_json::to_value(self.event.to_event_body())
                .expect("EventBody serialization is derived and infallible"),
        })
    }
}

/// Normalizes a whole trace straight to the fixture/report JSON form.
///
/// # Errors
///
/// Propagates [`AcpError`] from line decoding or shape mapping.
pub fn normalize_trace_to_json<I>(lines: I) -> Result<Value, AcpError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    Ok(Value::Array(
        normalize_trace(lines)?
            .iter()
            .map(NormalizedEvent::to_json)
            .collect(),
    ))
}

/// Maps one decoded wire message to zero or more normalized events.
///
/// # Errors
///
/// Returns [`AcpError::MalformedMessage`] when a modeled method's params
/// do not match the v1 shapes.
pub fn normalize_message(message: &RpcMessage) -> Result<Vec<NormalizedEvent>, AcpError> {
    match message {
        RpcMessage::Response { result, .. } => Ok(vec![NormalizedEvent {
            wire_kind: "response".to_owned(),
            event: map_result(result),
        }]),
        RpcMessage::ErrorResponse { id, error } => Ok(vec![NormalizedEvent {
            wire_kind: "error_response".to_owned(),
            event: AgentEvent::Preserved {
                provider_kind: "acp.failure".to_owned(),
                raw: serde_json::json!({
                    "id": id_json(id),
                    "code": error.code,
                    "message": error.message,
                }),
            },
        }]),
        RpcMessage::Request { id, method, params } => match method.as_str() {
            "session/request_permission" => {
                let permission: crate::messages::RequestPermissionParams =
                    serde_json::from_value(params.clone())?;
                Ok(vec![NormalizedEvent {
                    wire_kind: "session/request_permission".to_owned(),
                    event: AgentEvent::PermissionRequested {
                        request_id: id_json(id),
                        tool_call_id: Some(permission.tool_call.tool_call_id),
                    },
                }])
            }
            "fs/read_text_file" | "fs/write_text_file" => Ok(vec![NormalizedEvent {
                wire_kind: method.clone(),
                // The host answers with a typed refusal (ADR 0007); the
                // trace records that the agent tried.
                event: AgentEvent::Preserved {
                    provider_kind: "acp.fs.denied".to_owned(),
                    raw: serde_json::json!({ "method": method }),
                },
            }]),
            _ => Ok(vec![NormalizedEvent {
                wire_kind: format!("request:{method}"),
                event: AgentEvent::Preserved {
                    provider_kind: "acp.request.unmapped".to_owned(),
                    raw: serde_json::json!({ "method": method, "id": id_json(id) }),
                },
            }]),
        },
        RpcMessage::Notification { method, params } => match method.as_str() {
            "session/update" => {
                // The v1 shape is { sessionId, update }; the update object
                // carries the `sessionUpdate` discriminator.
                let update = params
                    .get("update")
                    .ok_or_else(|| AcpError::MalformedMessage {
                        diagnostic: "session/update params carry no update object".to_owned(),
                    })?;
                let update = SessionUpdate::from_value(update)?;
                Ok(vec![NormalizedEvent {
                    wire_kind: format!("session/update:{}", update.kind_name()),
                    event: map_update(&update),
                }])
            }
            _ => Ok(vec![NormalizedEvent {
                wire_kind: format!("notification:{method}"),
                event: AgentEvent::Preserved {
                    provider_kind: "acp.notification.unmapped".to_owned(),
                    raw: serde_json::json!({ "method": method }),
                },
            }]),
        },
    }
}

/// Normalizes a whole raw trace (one stream line per entry).
///
/// # Errors
///
/// Propagates [`AcpError`] from line decoding or shape mapping; a trace
/// that cannot normalize is broken evidence, not a partial success.
pub fn normalize_trace<I>(lines: I) -> Result<Vec<NormalizedEvent>, AcpError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut events = Vec::new();
    for line in lines {
        let message = RpcMessage::decode(line.as_ref())?;
        events.extend(normalize_message(&message)?);
    }
    Ok(events)
}

fn map_result(result: &Value) -> AgentEvent {
    // Result objects are ambiguous without the request context; trace
    // normalization sniffs the v1 shapes: `sessionId` means session
    // creation, `stopReason` means a prompt response (session/load
    // results carry neither and preserve). Real wiring in P1 routes
    // responses by pending request id instead of sniffing.
    if let Ok(new_session) = serde_json::from_value::<NewSessionResult>(result.clone()) {
        return AgentEvent::SessionCreated {
            session_id: new_session.session_id,
        };
    }
    if let Ok(prompt) = serde_json::from_value::<PromptResult>(result.clone()) {
        return map_stop_reason(prompt.stop_reason);
    }
    AgentEvent::Preserved {
        provider_kind: "acp.result.unmapped".to_owned(),
        raw: result.clone(),
    }
}

/// Maps a prompt stop reason per the ADR table.
#[must_use]
pub fn map_stop_reason(stop_reason: StopReason) -> AgentEvent {
    if stop_reason.is_success() {
        AgentEvent::TurnCompleted { stop_reason }
    } else {
        AgentEvent::TurnFailed {
            diagnostic: format!("turn stopped: {}", stop_reason.wire_name()),
        }
    }
}

fn map_update(update: &SessionUpdate) -> AgentEvent {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) | SessionUpdate::UserMessageChunk(chunk) => {
            match &chunk.content {
                ContentBlock::Text { text } => {
                    if matches!(update, SessionUpdate::AgentMessageChunk(_)) {
                        AgentEvent::Delta { text: text.clone() }
                    } else {
                        // The user's own words echo back; the Desktop
                        // already has them. Preserved, never dropped.
                        AgentEvent::Preserved {
                            provider_kind: "acp.user_message".to_owned(),
                            raw: serde_json::json!({ "text": text }),
                        }
                    }
                }
                other => AgentEvent::Preserved {
                    provider_kind: "acp.content.unmapped".to_owned(),
                    raw: serde_json::json!({ "content": other }),
                },
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => AgentEvent::Preserved {
            provider_kind: "acp.agent_thought".to_owned(),
            raw: serde_json::json!({ "content": chunk.content }),
        },
        SessionUpdate::ToolCall(tool_call) | SessionUpdate::ToolCallUpdate(tool_call) => {
            AgentEvent::ToolObserved {
                tool_call_id: tool_call.tool_call_id.clone(),
                status: tool_call.status.map(|status| status.wire_name().to_owned()),
            }
        }
        SessionUpdate::Preserved { kind, raw } => AgentEvent::Preserved {
            provider_kind: format!("acp.update.{kind}"),
            raw: raw.clone(),
        },
    }
}

fn id_json(id: &crate::wire::RpcId) -> String {
    match id {
        crate::wire::RpcId::Number(number) => number.to_string(),
        crate::wire::RpcId::Text(text) => text.clone(),
    }
}

/// Caps a compact JSON fragment to the diagnostic limit at a UTF-8
/// character boundary, marking truncation. Preserved bodies must never
/// fail or drop.
///
/// # Panics
///
/// Never in practice: the string is truncated to
/// [`DiagnosticText::capacity`] above, so the final construction cannot
/// fail.
#[must_use]
pub fn bounded_diagnostic(value: &Value) -> DiagnosticText {
    let mut json = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    let capacity = DiagnosticText::capacity();
    if json.len() > capacity {
        let marker = "…[truncated]";
        let cut = capacity - marker.len();
        let boundary = json
            .char_indices()
            .map(|(index, _)| index)
            .rfind(|index| *index <= cut)
            .unwrap_or(0);
        json.truncate(boundary);
        json.push_str(marker);
    }
    // Pre-truncated to the capacity above, so construction cannot fail.
    DiagnosticText::try_from(json.as_str()).expect("pre-truncated to the diagnostic cap")
}

/// Caps delta text to the message limit at a UTF-8 character boundary.
///
/// # Panics
///
/// Never in practice: every branch constructs from text at or below
/// [`MessageText::capacity`].
#[must_use]
pub fn truncate_to_message(text: &str) -> MessageText {
    let capacity = MessageText::capacity();
    if text.len() <= capacity {
        // At or below the cap by the branch condition.
        return MessageText::try_from(text).expect("within the message cap");
    }
    let boundary = text
        .char_indices()
        .map(|(index, _)| index)
        .rfind(|index| *index <= capacity)
        .unwrap_or(0);
    MessageText::try_from(&text[..boundary]).expect("truncated at or below the message cap")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_lifecycle_maps_onto_known_events() {
        assert_eq!(
            AgentEvent::TurnStarted.to_event_body(),
            EventBody::Known(KnownEvent::TurnStarted)
        );
        assert_eq!(
            AgentEvent::Delta {
                text: "Hel".to_owned()
            }
            .to_event_body(),
            EventBody::Known(KnownEvent::MessageDelta {
                text: MessageText::try_from("Hel").unwrap()
            })
        );
        assert_eq!(
            map_stop_reason(StopReason::EndTurn),
            AgentEvent::TurnCompleted {
                stop_reason: StopReason::EndTurn
            }
        );
        assert_eq!(
            map_stop_reason(StopReason::Cancelled),
            AgentEvent::TurnCompleted {
                stop_reason: StopReason::Cancelled
            }
        );
        let failed = map_stop_reason(StopReason::Refusal);
        let EventBody::Unknown { provider_kind, .. } = failed.to_event_body() else {
            panic!("refusal must preserve as a failure, not a known kind");
        };
        assert_eq!(provider_kind, "acp.turn.failed");
    }

    #[test]
    fn tools_and_permissions_preserve_with_acp_kinds() {
        let tool = AgentEvent::ToolObserved {
            tool_call_id: "tc_1".to_owned(),
            status: Some("completed".to_owned()),
        };
        let EventBody::Unknown { provider_kind, .. } = tool.to_event_body() else {
            panic!("tool events preserve until P1");
        };
        assert_eq!(provider_kind, "acp.tool");

        let permission = AgentEvent::PermissionRequested {
            request_id: "7".to_owned(),
            tool_call_id: Some("tc_2".to_owned()),
        };
        let EventBody::Unknown { provider_kind, .. } = permission.to_event_body() else {
            panic!("permission events preserve until P1");
        };
        assert_eq!(provider_kind, "acp.permission.requested");
    }

    #[test]
    fn oversized_payloads_truncate_never_drop() {
        let huge = "x".repeat(MessageText::capacity() * 2);
        let delta = AgentEvent::Delta { text: huge };
        let EventBody::Known(KnownEvent::MessageDelta { text }) = delta.to_event_body() else {
            panic!("delta stays a delta");
        };
        assert!(text.len() <= MessageText::capacity());

        let preserved = AgentEvent::Preserved {
            provider_kind: "acp.update.usage_update".to_owned(),
            raw: serde_json::json!({ "padding": "y".repeat(DiagnosticText::capacity() * 2) }),
        };
        let EventBody::Unknown { diagnostic, .. } = preserved.to_event_body() else {
            panic!("preserved stays preserved");
        };
        assert!(diagnostic.len() <= DiagnosticText::capacity());
        assert!(diagnostic.as_str().ends_with("…[truncated]"));
    }

    #[test]
    fn unknown_updates_survive_with_their_kind() {
        let events = normalize_trace([
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":42,"size":2000}}}"#,
            r#"{"jsonrpc":"2.0","method":"session/whatever","params":{}}"#,
        ])
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].wire_kind, "session/update:usage_update");
        let AgentEvent::Preserved { provider_kind, .. } = &events[0].event else {
            panic!("usage_update preserves");
        };
        assert_eq!(provider_kind, "acp.update.usage_update");
        assert_eq!(events[1].wire_kind, "notification:session/whatever");
    }
}
