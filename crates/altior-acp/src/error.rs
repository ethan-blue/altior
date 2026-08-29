//! Typed adapter errors (AGENTS.md: no vague string errors).

/// Every failure mode of the ACP adapter's pure layers.
#[derive(Debug)]
#[non_exhaustive]
pub enum AcpError {
    /// A line or message did not match the ACP v1 shapes this adapter
    /// models. The diagnostic names the offending field or message.
    MalformedMessage {
        /// What failed, in one bounded sentence.
        diagnostic: String,
    },
    /// A stream line exceeded [`crate::MAX_LINE_BYTES`]; the stream is
    /// untrustworthy and the connection must close.
    LineTooLarge {
        /// The offending line length in bytes.
        size_bytes: usize,
        /// The cap that was exceeded.
        limit_bytes: usize,
    },
    /// A stream line was not valid UTF-8.
    LineNotUtf8,
    /// The agent answered a request with a JSON-RPC error object.
    RpcError {
        /// The JSON-RPC error code (e.g. `-32601` method not found).
        code: i64,
        /// The agent's error message.
        message: String,
    },
    /// The agent process exited or its stream ended.
    ProcessExited {
        /// How it ended, e.g. `exit code 0` or `stream ended`.
        status: String,
    },
    /// A state machine was driven out of order (e.g. reporting a prompt
    /// response with no prompt outstanding).
    OutOfOrder {
        /// What was attempted.
        attempted: &'static str,
    },
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedMessage { diagnostic } => {
                write!(f, "malformed ACP message: {diagnostic}")
            }
            Self::LineTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(f, "ACP line of {size_bytes} bytes exceeds {limit_bytes}"),
            Self::LineNotUtf8 => write!(f, "ACP line is not valid UTF-8"),
            Self::RpcError { code, message } => {
                write!(f, "agent RPC error {code}: {message}")
            }
            Self::ProcessExited { status } => write!(f, "agent process exited ({status})"),
            Self::OutOfOrder { attempted } => write!(f, "out-of-order adapter use: {attempted}"),
        }
    }
}

impl std::error::Error for AcpError {}

impl From<serde_json::Error> for AcpError {
    fn from(source: serde_json::Error) -> Self {
        Self::MalformedMessage {
            diagnostic: source.to_string(),
        }
    }
}
