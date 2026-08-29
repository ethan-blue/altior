//! Typed failures for the local IPC layer (ADR 0006).

use std::fmt;

use altior_protocol::ProtocolError;

/// Typed failure for an IPC contract operation.
///
/// `#[non_exhaustive]` so new failure kinds can be added without a breaking
/// release.
#[derive(Debug)]
#[non_exhaustive]
pub enum IpcError {
    /// A frame whose declared or encoded size exceeds the cap.
    FrameTooLarge {
        /// The offending size in bytes.
        size_bytes: usize,
        /// The enforced cap in bytes.
        limit_bytes: usize,
    },
    /// A frame payload that is not valid UTF-8 JSON text.
    FrameNotUtf8,
    /// The presented launch token does not match this Core launch.
    AuthenticationRejected,
    /// The endpoint holds no live Core (nothing listening, or the token
    /// file belongs to a dead launch).
    EndpointUnavailable {
        /// The endpoint that was probed.
        endpoint: String,
    },
    /// The endpoint answered, but with a different Core launch than the one
    /// this client sessioned with — Core restarted (ADR 0006).
    StaleEndpoint {
        /// The instance id the client expected.
        expected: String,
        /// The instance id that answered.
        found: String,
    },
    /// A session was driven in an order its state machine forbids.
    SessionOrder {
        /// What the caller attempted.
        attempted: &'static str,
    },
    /// A protocol contract failed underneath the IPC layer.
    Protocol {
        /// The underlying typed protocol failure.
        source: ProtocolError,
    },
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "frame is {size_bytes} bytes, limit is {limit_bytes} bytes"
            ),
            Self::FrameNotUtf8 => write!(f, "frame payload is not valid UTF-8"),
            Self::AuthenticationRejected => {
                write!(f, "launch token rejected; connection refused")
            }
            Self::EndpointUnavailable { endpoint } => {
                write!(f, "no live Core at endpoint {endpoint}")
            }
            Self::StaleEndpoint { expected, found } => write!(
                f,
                "endpoint holds Core instance {found}, session expected {expected}"
            ),
            Self::SessionOrder { attempted } => {
                write!(f, "session state forbids {attempted}")
            }
            Self::Protocol { source } => write!(f, "protocol failure: {source}"),
        }
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol { source } => Some(source),
            _ => None,
        }
    }
}

impl From<ProtocolError> for IpcError {
    fn from(source: ProtocolError) -> Self {
        Self::Protocol { source }
    }
}
