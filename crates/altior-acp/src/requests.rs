//! JSON-RPC 2.0 request builders and request-ID tracking for ACP (ADR 0007).
//!
//! Provides typed builders for `initialize`, `session/new`, `session/load`,
//! `session/prompt`, `session/cancel`, and capability-gated `session/steer`,
//! while tracking pending request IDs for response routing.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::AcpError;
use crate::messages::{
    CancelParams, CancelledOutcome, CancelledPermissionOutcome, ContentBlock, IdSource,
    LoadSessionParams, NewSessionParams, PromptParams,
};
use crate::negotiation::{NegotiatedCapabilities, initialize_request};
use crate::wire::{RpcError, RpcId, RpcMessage};

/// The kind of an outstanding request sent by the client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingKind {
    /// Handshake initialization request.
    Initialize,
    /// New session creation request.
    NewSession,
    /// Resuming an existing session via `session/load`.
    LoadSession,
    /// Active prompt request.
    Prompt,
    /// Mid-turn prompt steer request.
    Steer,
    /// Other custom or forward-compatible request.
    Other(String),
}

/// Tracks outstanding requests and correlates incoming responses with their origin.
#[derive(Debug, Default)]
pub struct PendingRequests {
    pending: BTreeMap<String, (RpcId, PendingKind)>,
}

impl PendingRequests {
    /// Creates an empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an outstanding request ID and its expected kind.
    pub fn insert(&mut self, id: RpcId, kind: PendingKind) {
        let key = id_key(&id);
        self.pending.insert(key, (id, kind));
    }

    /// Removes and returns the request kind for an answered request ID.
    pub fn remove(&mut self, id: &RpcId) -> Option<PendingKind> {
        let key = id_key(id);
        self.pending.remove(&key).map(|(_, kind)| kind)
    }

    /// Looks up the pending request kind for a given ID without removing it.
    #[must_use]
    pub fn get(&self, id: &RpcId) -> Option<&PendingKind> {
        let key = id_key(id);
        self.pending.get(&key).map(|(_, kind)| kind)
    }

    /// Number of outstanding requests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether there are no outstanding requests.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

fn id_key(id: &RpcId) -> String {
    match id {
        RpcId::Number(n) => format!("num:{n}"),
        RpcId::Text(t) => format!("txt:{t}"),
    }
}

/// Factory for building outgoing JSON-RPC requests with allocated IDs.
#[derive(Debug, Default)]
pub struct RequestBuilder {
    id_source: IdSource,
}

impl RequestBuilder {
    /// Creates a new request builder with a fresh ID source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an `initialize` handshake request.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    pub fn build_initialize(&mut self, client_version: &str) -> (RpcId, RpcMessage) {
        let id = self.id_source.allocate();
        let params = serde_json::to_value(initialize_request(client_version))
            .expect("InitializeParams is statically serializable");
        let msg = RpcMessage::Request {
            id: id.clone(),
            method: "initialize".to_owned(),
            params,
        };
        (id, msg)
    }

    /// Builds a `session/new` request.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    pub fn build_new_session(&mut self, cwd: &str, mcp_servers: Vec<Value>) -> (RpcId, RpcMessage) {
        let id = self.id_source.allocate();
        let params = serde_json::to_value(NewSessionParams {
            cwd: cwd.to_owned(),
            mcp_servers,
        })
        .expect("NewSessionParams is statically serializable");
        let msg = RpcMessage::Request {
            id: id.clone(),
            method: "session/new".to_owned(),
            params,
        };
        (id, msg)
    }

    /// Builds a `session/load` request to resume an existing session.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    pub fn build_load_session(
        &mut self,
        cwd: &str,
        session_id: &str,
        mcp_servers: Vec<Value>,
    ) -> (RpcId, RpcMessage) {
        let id = self.id_source.allocate();
        let params = serde_json::to_value(LoadSessionParams {
            cwd: cwd.to_owned(),
            session_id: session_id.to_owned(),
            mcp_servers,
        })
        .expect("LoadSessionParams is statically serializable");
        let msg = RpcMessage::Request {
            id: id.clone(),
            method: "session/load".to_owned(),
            params,
        };
        (id, msg)
    }

    /// Builds a `session/prompt` request.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    pub fn build_prompt(
        &mut self,
        session_id: &str,
        prompt: Vec<ContentBlock>,
    ) -> (RpcId, RpcMessage) {
        let id = self.id_source.allocate();
        let params = serde_json::to_value(PromptParams {
            session_id: session_id.to_owned(),
            prompt,
        })
        .expect("PromptParams is statically serializable");
        let msg = RpcMessage::Request {
            id: id.clone(),
            method: "session/prompt".to_owned(),
            params,
        };
        (id, msg)
    }

    /// Builds a capability-gated `session/steer` request.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::Unsupported`] if the agent capabilities do not
    /// advertise `steer`.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    pub fn build_steer(
        &mut self,
        session_id: &str,
        prompt: Vec<ContentBlock>,
        capabilities: &NegotiatedCapabilities,
    ) -> Result<(RpcId, RpcMessage), AcpError> {
        if !capabilities.supports_steer() {
            return Err(AcpError::Unsupported {
                feature: "session/steer",
            });
        }
        let id = self.id_source.allocate();
        let params = serde_json::to_value(PromptParams {
            session_id: session_id.to_owned(),
            prompt,
        })
        .expect("PromptParams is statically serializable");
        let msg = RpcMessage::Request {
            id: id.clone(),
            method: "session/steer".to_owned(),
            params,
        };
        Ok((id, msg))
    }

    /// Builds a `session/cancel` one-way notification.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    #[must_use]
    pub fn build_cancel(session_id: &str) -> RpcMessage {
        let params = serde_json::to_value(CancelParams {
            session_id: session_id.to_owned(),
        })
        .expect("CancelParams is statically serializable");
        RpcMessage::Notification {
            method: "session/cancel".to_owned(),
            params,
        }
    }

    /// Builds a response to an agent `session/request_permission` request.
    #[must_use]
    pub fn build_permission_response(id: RpcId, outcome: Value) -> RpcMessage {
        RpcMessage::Response {
            id,
            result: outcome,
        }
    }

    /// Builds a `{"outcome":"cancelled"}` response to a permission request.
    ///
    /// # Panics
    ///
    /// Panics if static serialization fails.
    #[must_use]
    pub fn build_permission_cancelled(id: RpcId) -> RpcMessage {
        let result = serde_json::to_value(CancelledPermissionOutcome {
            outcome: CancelledOutcome::Cancelled,
        })
        .expect("CancelledPermissionOutcome is statically serializable");
        RpcMessage::Response { id, result }
    }

    /// Builds a standard `-32601` method-not-found refusal for unserved agent requests (e.g. `fs/*`).
    #[must_use]
    pub fn build_refusal(id: RpcId, method: &str) -> RpcMessage {
        RpcMessage::ErrorResponse {
            id,
            error: RpcError {
                code: -32601,
                message: format!("altior grants no {method}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_allocates_sequential_ids_and_encodes_valid_messages() {
        let mut builder = RequestBuilder::new();
        let (id1, msg1) = builder.build_initialize("0.1.0");
        let (id2, msg2) = builder.build_new_session("/tmp", Vec::new());
        let (id3, msg3) =
            builder.build_prompt("s1", vec![ContentBlock::Text { text: "hi".into() }]);

        assert_eq!(id1, RpcId::Number(1));
        assert_eq!(id2, RpcId::Number(2));
        assert_eq!(id3, RpcId::Number(3));

        assert!(matches!(msg1, RpcMessage::Request { ref method, .. } if method == "initialize"));
        assert!(matches!(msg2, RpcMessage::Request { ref method, .. } if method == "session/new"));
        assert!(
            matches!(msg3, RpcMessage::Request { ref method, .. } if method == "session/prompt")
        );
    }

    #[test]
    fn pending_requests_tracks_and_correlates_ids() {
        let mut tracker = PendingRequests::new();
        tracker.insert(RpcId::Number(1), PendingKind::Initialize);
        tracker.insert(
            RpcId::Text("agent-custom".into()),
            PendingKind::Other("custom".into()),
        );

        assert_eq!(tracker.len(), 2);
        assert_eq!(
            tracker.get(&RpcId::Number(1)),
            Some(&PendingKind::Initialize)
        );
        assert_eq!(
            tracker.remove(&RpcId::Number(1)),
            Some(PendingKind::Initialize)
        );
        assert_eq!(tracker.remove(&RpcId::Number(1)), None);
        assert_eq!(tracker.len(), 1);
        assert!(!tracker.is_empty());
    }

    #[test]
    fn steer_is_capability_gated() {
        use crate::messages::PromptCapabilities;
        let mut builder = RequestBuilder::new();
        let unsupportive = NegotiatedCapabilities {
            load_session: false,
            resume: false,
            steer: false,
            prompt: PromptCapabilities::default(),
            agent_protocol_version: 1,
        };
        let err = builder
            .build_steer("s1", vec![], &unsupportive)
            .unwrap_err();
        assert!(matches!(
            err,
            AcpError::Unsupported {
                feature: "session/steer"
            }
        ));

        let supportive = NegotiatedCapabilities {
            load_session: false,
            resume: false,
            steer: true,
            prompt: PromptCapabilities::default(),
            agent_protocol_version: 1,
        };
        let res = builder.build_steer("s1", vec![], &supportive);
        assert!(res.is_ok());
    }
}
