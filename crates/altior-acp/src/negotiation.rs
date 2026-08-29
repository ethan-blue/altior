//! Capability negotiation for the `initialize` handshake (ADR 0007).
//!
//! The plan is explicit: negotiate capabilities, never inspect version
//! strings. The agent's `protocolVersion` is recorded as data; every
//! feature gate reads a capability boolean. An agent advertising nothing
//! negotiates to all-false — a usable-but-minimal partner, not an error.

use crate::messages::{
    AgentCapabilities, ClientCapabilities, FileSystemCapabilities, Implementation,
    InitializeParams, InitializeResult, PromptCapabilities, SessionCapabilities,
};

/// The protocol version Altior speaks. Recorded in both directions; never
/// used as a feature gate (ADR 0007).
pub const CLIENT_PROTOCOL_VERSION: u16 = 1;

/// What the handshake established, as capability data only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedCapabilities {
    /// `agentCapabilities.loadSession`: `session/load` may be sent.
    pub load_session: bool,
    /// `agentCapabilities.sessionCapabilities.resume`: the newer resume
    /// path exists; the spike still uses `session/load`.
    pub resume: bool,
    /// Which prompt content kinds the agent accepts.
    pub prompt: PromptCapabilities,
    /// The agent's `protocolVersion`, recorded verbatim.
    pub agent_protocol_version: u16,
}

impl NegotiatedCapabilities {
    /// Whether a suspended thread may be resumed with this agent.
    #[must_use]
    pub fn may_resume(self) -> bool {
        self.load_session || self.resume
    }

    /// Whether plain-text prompts work (every v1 agent must accept them).
    #[must_use]
    pub fn accepts_text_prompts(self) -> bool {
        true
    }
}

/// Derives the negotiated view from the agent's `initialize` result.
#[must_use]
pub fn negotiate(result: &InitializeResult) -> NegotiatedCapabilities {
    NegotiatedCapabilities {
        load_session: result.agent_capabilities.load_session,
        resume: result.agent_capabilities.session_capabilities.resume,
        prompt: result.agent_capabilities.prompt_capabilities,
        agent_protocol_version: result.protocol_version,
    }
}

/// Builds Altior's `initialize` request parameters. The spike grants the
/// agent no filesystem and no terminal: those are P4 workbench concerns
/// behind permission profiles, and a capability never advertised is a
/// request the agent must not send.
#[must_use]
pub fn initialize_request(client_version: &str) -> InitializeParams {
    InitializeParams {
        protocol_version: CLIENT_PROTOCOL_VERSION,
        client_capabilities: ClientCapabilities {
            fs: FileSystemCapabilities {
                read_text_file: false,
                write_text_file: false,
            },
        },
        client_info: Some(Implementation {
            name: "altior".to_owned(),
            version: Some(client_version.to_owned()),
        }),
    }
}

/// Re-exported for negotiation tests: the default capability object an
/// agent that advertises nothing parses into.
#[must_use]
pub fn no_capabilities() -> AgentCapabilities {
    AgentCapabilities {
        load_session: false,
        prompt_capabilities: PromptCapabilities::default(),
        session_capabilities: SessionCapabilities::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_result(json: &str) -> InitializeResult {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn capability_booleans_gate_features_not_versions() {
        let capable =
            initialize_result(r#"{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}"#);
        assert!(negotiate(&capable).may_resume());

        // A different version number with the same capabilities negotiates
        // identical gates: versions are data, capabilities are the contract.
        let other_version =
            initialize_result(r#"{"protocolVersion":7,"agentCapabilities":{"loadSession":true}}"#);
        let a = negotiate(&capable);
        let b = negotiate(&other_version);
        assert_eq!(
            (a.load_session, a.resume, a.prompt),
            (b.load_session, b.resume, b.prompt)
        );
        assert_ne!(a.agent_protocol_version, b.agent_protocol_version);

        let silent = initialize_result(r#"{"protocolVersion":1}"#);
        let negotiated = negotiate(&silent);
        assert!(!negotiated.may_resume());
        assert!(negotiated.accepts_text_prompts());
    }

    #[test]
    fn the_client_advertises_nothing_it_does_not_serve() {
        let params = initialize_request("0.3.0");
        let encoded = serde_json::to_value(&params).unwrap();
        assert_eq!(
            encoded["clientCapabilities"]["fs"],
            serde_json::json!({"readTextFile": false, "writeTextFile": false})
        );
        assert_eq!(
            encoded["protocolVersion"],
            serde_json::json!(CLIENT_PROTOCOL_VERSION)
        );
        assert_eq!(encoded["clientInfo"]["name"], serde_json::json!("altior"));
    }
}
