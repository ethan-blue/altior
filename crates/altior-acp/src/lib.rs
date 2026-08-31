//! Narrow ACP v1 adapter (ADR 0007).
//!
//! ACP (Agent Client Protocol) is JSON-RPC 2.0 spoken between a client and
//! an agent; the reference agents use newline-delimited JSON over a
//! subprocess's stdin/stdout. This crate adapts that wire protocol to
//! Altior's contracts without leaking ACP types into the domain or
//! protocol crates:
//!
//! - [`config`] typed bounded launch configuration and secret-reference integration seams;
//! - [`transport`] child process transport port, stdin/stdout line streaming, and bounded stderr capture;
//! - [`requests`] JSON-RPC request builders and request ID tracking;
//! - [`runtime`] high-level subprocess session runtime adapter and reader loop;
//! - [`wire`] models the JSON-RPC envelope and the bounded line codec;
//! - [`messages`] models the ACP v1 subset this adapter maps, preserving
//!   everything else verbatim (the ADR 0004 unknown-preservation rule);
//! - [`negotiation`] negotiates on capabilities, never on version strings;
//! - [`mapping`] turns wire messages into adapter events and protocol
//!   event bodies, and normalizes whole traces;
//! - [`delivery`] classifies every prompt attempt onto the frozen
//!   [`altior_domain::DeliveryState`] vocabulary;
//! - [`lifecycle`] is the pure process decision machine for cancel,
//!   crash, idle timeout, and cleanup.

pub mod config;
pub mod delivery;
pub mod error;
pub mod lifecycle;
pub mod mapping;
pub mod messages;
pub mod negotiation;
pub mod requests;
pub mod runtime;
pub mod transport;
pub mod wire;

pub use altior_domain::DeliveryState;
pub use config::{
    EnvVarValue, LaunchConfig, MAX_ARG_BYTES, MAX_ARGS_COUNT, MAX_ENV_COUNT, MAX_ENV_KEY_BYTES,
    MAX_ENV_VALUE_BYTES, MAX_PROGRAM_BYTES, MAX_SECRET_REF_BYTES, MAX_STDERR_CAPTURE_BYTES,
    NoSecretsResolver, ResolvedLaunchConfig, SecretRef, SecretResolver,
};
pub use delivery::{DeliveryCause, PromptDelivery};
pub use error::AcpError;
pub use lifecycle::{AgentLifecycle, AgentPhase, HostAction};
pub use mapping::{AgentEvent, NormalizedEvent};
pub use negotiation::{NegotiatedCapabilities, initialize_request};
pub use requests::{PendingKind, PendingRequests, RequestBuilder};
pub use runtime::AcpRuntime;
pub use transport::{AcpChild, ProcessTransport};
pub use wire::{LineDecoder, MAX_LINE_BYTES, RpcError, RpcId, RpcMessage, encode_line};
