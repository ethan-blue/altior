//! JSON-RPC 2.0 over newline-delimited UTF-8 lines (ADR 0007).
//!
//! The envelope codec is shape-driven: a message is classified by which
//! JSON-RPC fields it carries (`id`+`method` = request, `method` only =
//! notification, `id`+`result`/`error` = response). Anything else is a
//! typed [`AcpError::MalformedMessage`]; the adapter never guesses.
//!
//! [`LineDecoder`] mirrors the IPC frame decoder of ADR 0006: feed
//! arbitrary byte chunks, receive whole lines. Lines are bounded at
//! [`MAX_LINE_BYTES`]; an oversized line poisons the decoder and the
//! connection must close.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::AcpError;

/// Hard cap on one stream line: 1 MiB (ADR 0007). ACP tool outputs can be
/// large; the cap exists so a corrupt peer cannot pin memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// A JSON-RPC request id: number or string. Altior assigns `Number`
/// ids from a per-connection counter; agent-issued ids are echoed back
/// verbatim, whatever their shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    /// A numeric id.
    Number(u64),
    /// A string id.
    Text(String),
}

/// A JSON-RPC 2.0 error object.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    /// The error code.
    pub code: i64,
    /// The error message.
    pub message: String,
}

/// One message on the ACP stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcMessage {
    /// A request expecting a response.
    Request {
        /// The caller-assigned id.
        id: RpcId,
        /// The method name, e.g. `session/prompt`.
        method: String,
        /// The method parameters.
        params: Value,
    },
    /// A one-way notification.
    Notification {
        /// The method name, e.g. `session/update`.
        method: String,
        /// The notification parameters.
        params: Value,
    },
    /// A successful response to a request.
    Response {
        /// The id of the answered request.
        id: RpcId,
        /// The result payload.
        result: Value,
    },
    /// An error response to a request.
    ErrorResponse {
        /// The id of the answered request.
        id: RpcId,
        /// The error object.
        error: RpcError,
    },
}

impl RpcMessage {
    /// Decodes one stream line into a classified message.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::MalformedMessage`] for anything that is not a
    /// recognizable JSON-RPC 2.0 message.
    pub fn decode(line: &str) -> Result<Self, AcpError> {
        let value: Value = serde_json::from_str(line)?;
        let object = value
            .as_object()
            .ok_or_else(|| AcpError::MalformedMessage {
                diagnostic: "stream line is not a JSON object".to_owned(),
            })?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(AcpError::MalformedMessage {
                diagnostic: "message is not jsonrpc 2.0".to_owned(),
            });
        }
        let id = object.get("id").map(decode_id).transpose()?;
        match (object.get("method"), id) {
            (Some(method), Some(id)) => Ok(Self::Request {
                id,
                method: method.as_str().ok_or_else(malformed)?.to_owned(),
                params: object.get("params").cloned().unwrap_or(Value::Null),
            }),
            (Some(method), None) => Ok(Self::Notification {
                method: method.as_str().ok_or_else(malformed)?.to_owned(),
                params: object.get("params").cloned().unwrap_or(Value::Null),
            }),
            (None, Some(id)) => {
                if let Some(error) = object.get("error") {
                    Ok(Self::ErrorResponse {
                        id,
                        error: serde_json::from_value(error.clone())?,
                    })
                } else {
                    Ok(Self::Response {
                        id,
                        result: object.get("result").cloned().unwrap_or(Value::Null),
                    })
                }
            }
            (None, None) => Err(malformed()),
        }
    }

    /// Encodes the message as one stream line (no trailing newline).
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::MalformedMessage`] when the message cannot be
    /// serialized.
    pub fn encode(&self) -> Result<String, AcpError> {
        let value = match self {
            Self::Request { id, method, params } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }),
            Self::Notification { method, params } => json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            }),
            Self::Response { id, result } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }),
            Self::ErrorResponse { id, error } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": error,
            }),
        };
        Ok(serde_json::to_string(&value)?)
    }

    /// The method this message carries or answers, for diagnostics.
    #[must_use]
    pub fn describes(&self) -> &str {
        match self {
            Self::Request { method, .. } | Self::Notification { method, .. } => method,
            Self::Response { .. } | Self::ErrorResponse { .. } => "response",
        }
    }
}

fn malformed() -> AcpError {
    AcpError::MalformedMessage {
        diagnostic: "message is neither request, notification, nor response".to_owned(),
    }
}

fn decode_id(value: &Value) -> Result<RpcId, AcpError> {
    match value {
        Value::Number(number) => {
            let id = number.as_u64().ok_or_else(|| AcpError::MalformedMessage {
                diagnostic: "request id is not an unsigned number".to_owned(),
            })?;
            Ok(RpcId::Number(id))
        }
        Value::String(text) => Ok(RpcId::Text(text.clone())),
        _ => Err(AcpError::MalformedMessage {
            diagnostic: "request id is neither number nor string".to_owned(),
        }),
    }
}

/// Serializes one line and appends the newline delimiter, checking the
/// [`MAX_LINE_BYTES`] cap.
///
/// # Errors
///
/// Returns [`AcpError::LineTooLarge`] when the encoded line (including
/// the newline) exceeds the cap.
pub fn encode_line(line: &str) -> Result<Vec<u8>, AcpError> {
    if line.len() + 1 > MAX_LINE_BYTES {
        return Err(AcpError::LineTooLarge {
            size_bytes: line.len() + 1,
            limit_bytes: MAX_LINE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    Ok(bytes)
}

/// Incremental line decoder: feed bytes, receive whole lines. A line over
/// [`MAX_LINE_BYTES`] or invalid UTF-8 poisons the decoder — the stream
/// is untrustworthy and must close.
#[derive(Debug, Default)]
pub struct LineDecoder {
    buffer: Vec<u8>,
}

impl LineDecoder {
    /// Creates an empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds `chunk` and returns every complete line now available, in
    /// order, without the newline delimiters.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::LineTooLarge`] or [`AcpError::LineNotUtf8`];
    /// the decoder is then poisoned.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<String>, AcpError> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        loop {
            let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                if self.buffer.len() > MAX_LINE_BYTES {
                    return Err(AcpError::LineTooLarge {
                        size_bytes: self.buffer.len(),
                        limit_bytes: MAX_LINE_BYTES,
                    });
                }
                break;
            };
            if newline + 1 > MAX_LINE_BYTES {
                return Err(AcpError::LineTooLarge {
                    size_bytes: newline + 1,
                    limit_bytes: MAX_LINE_BYTES,
                });
            }
            let mut line_bytes: Vec<u8> = self.buffer.drain(..=newline).collect();
            line_bytes.pop();
            let line = String::from_utf8(line_bytes).map_err(|_| AcpError::LineNotUtf8)?;
            lines.push(line);
        }
        Ok(lines)
    }

    /// Number of buffered bytes waiting for a newline.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_notifications_and_responses_roundtrip() {
        let request = RpcMessage::decode(
            r#"{"jsonrpc":"2.0","id":1,"method":"session/prompt","params":{"sessionId":"s1"}}"#,
        )
        .unwrap();
        assert!(matches!(request, RpcMessage::Request { .. }));
        let encoded = request.encode().unwrap();
        assert_eq!(RpcMessage::decode(&encoded).unwrap(), request);

        let notification =
            RpcMessage::decode(r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#)
                .unwrap();
        assert!(matches!(notification, RpcMessage::Notification { .. }));

        let error = RpcMessage::decode(
            r#"{"jsonrpc":"2.0","id":"agent-7","error":{"code":-32601,"message":"nope"}}"#,
        )
        .unwrap();
        assert!(matches!(error, RpcMessage::ErrorResponse { .. }));
    }

    #[test]
    fn non_rpc_lines_fail_explicitly() {
        assert!(matches!(
            RpcMessage::decode(r#"{"id":1,"method":"x"}"#),
            Err(AcpError::MalformedMessage { .. })
        ));
        assert!(matches!(
            RpcMessage::decode(r#"["batch",1]"#),
            Err(AcpError::MalformedMessage { .. })
        ));
    }

    #[test]
    fn lines_reassemble_across_chunk_boundaries() {
        let mut decoder = LineDecoder::new();
        assert!(decoder.feed(b"{\"a\":").unwrap().is_empty());
        let lines = decoder.feed(b"1}\n{\"b\":2}\n").unwrap();
        assert_eq!(lines, ["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(decoder.pending_bytes(), 0);
    }

    #[test]
    fn oversized_lines_poison_the_decoder() {
        let mut decoder = LineDecoder::new();
        let big = "x".repeat(MAX_LINE_BYTES + 1);
        assert!(matches!(
            decoder.feed(big.as_bytes()),
            Err(AcpError::LineTooLarge { size_bytes, .. }) if size_bytes == MAX_LINE_BYTES + 1
        ));
        assert!(matches!(
            encode_line(&"y".repeat(MAX_LINE_BYTES)),
            Err(AcpError::LineTooLarge { .. })
        ));
    }
}
