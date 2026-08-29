//! Versioned contracts shared by Altior Desktop and Altior Core.
//!
//! This crate owns protocol version negotiation, capability declarations,
//! and the versioned command/snapshot/event envelopes. It defines DTOs and
//! their validation only: no transport, process supervision, or product
//! behavior. Provider/harness DTOs (ACP, terminal, native) never enter
//! this crate; they stay inside their adapter crates (ADR 0002, ADR 0004).
//!
//! With the `dto-export` feature the same DTO definitions export
//! TypeScript bindings for the Desktop client (ADR 0005), keeping Rust the
//! single schema source.

pub mod bounded;
pub mod capability;
pub mod command;
pub mod error;
pub mod event;
pub mod handshake;
pub mod snapshot;
pub mod version;

pub use bounded::{BoundedPayload, BoundedText, DiagnosticText, EnvelopeLimits, MessageText};
pub use capability::{CapabilityId, CapabilitySet, CapabilitySupport};
pub use command::{CommandEnvelope, CommandKind};
pub use error::ProtocolError;
pub use event::{EventBody, EventEnvelope, KnownEvent, Sequence};
pub use handshake::{CoreHello, DesktopHello, NegotiatedHandshake, negotiate};
pub use snapshot::SnapshotEnvelope;
pub use version::{
    ProductVersion, ProtocolVersion, ProtocolVersionRange, SUPPORTED_PROTOCOL_VERSIONS,
};
