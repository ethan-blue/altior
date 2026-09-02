//! Versioned contracts shared by Altior Desktop and Altior Core.
//!
//! This crate owns protocol version negotiation, capability declarations,
//! the versioned command/snapshot/event envelopes, and serialized DTOs.
//! It defines DTOs and their validation only: no transport, process supervision,
//! or product behavior. Provider/harness DTOs (ACP, terminal, native) never
//! enter this crate; they stay inside their adapter crates (ADR 0002, ADR 0004).
//!
//! With the `dto-export` feature the same DTO definitions export
//! TypeScript bindings for the Desktop client (ADR 0005), keeping Rust the
//! single schema source.

pub mod auth;
pub mod bounded;
pub mod capability;
pub mod command;
pub mod dto;
pub mod error;
pub mod event;
pub mod greeting;
pub mod handshake;
pub mod snapshot;
pub mod version;

pub use auth::LaunchToken;
pub use bounded::{BoundedPayload, BoundedText, DiagnosticText, EnvelopeLimits, MessageText};
pub use capability::{CapabilityId, CapabilitySet, CapabilitySupport};
pub use command::{
    CancelTurnCommand, CommandEnvelope, CommandKind, ConfigureAgentCommand, CreateThreadCommand,
    DiagnosticsCommand, GetHistoryCommand, ListThreadsCommand, OpenThreadCommand,
    RespondPermissionCommand, RuntimeStatusCommand, SearchThreadsCommand, StartTurnCommand,
    TestHarnessBindingCommand,
};
pub use dto::{
    AgentProfileDto, HarnessBindingConfigDto, HarnessBindingDto, PermissionDto,
    RuntimeDiagnosticsDto, ThreadCursorDto, ThreadDto, ThreadHistoryResponseDto,
    ThreadListResponseDto, ThreadSnapshotDto, ThreadSummaryDto, TurnCursorDto, TurnDto,
};
pub use error::ProtocolError;
pub use event::{EventBody, EventEnvelope, KnownEvent, Sequence};
pub use greeting::{CoreGreeting, RetainedWindow};
pub use handshake::{CoreHello, DesktopHello, NegotiatedHandshake, negotiate};
pub use snapshot::SnapshotEnvelope;
pub use version::{
    ProductVersion, ProtocolVersion, ProtocolVersionRange, SUPPORTED_PROTOCOL_VERSIONS,
};
