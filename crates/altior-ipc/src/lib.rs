//! Local IPC contract layer between Altior Desktop and Altior Core
//! (ADR 0006).
//!
//! This crate owns everything transport-adjacent that must stay provable
//! without I/O: length-prefixed frame encoding, user-scoped endpoint
//! naming, per-launch capability tokens, and the session state machines
//! that drive authentication, version negotiation, greeting, subscription,
//! and catch-up. The later async transport (tokio, named pipes, Unix domain
//! sockets — selected in ADR 0006) composes these types without changing
//! any contract.
//!
//! Everything here is deterministic: entropy and time are injected, there
//! are no timers, threads, or hidden retries.

pub mod auth;
pub mod endpoint;
pub mod error;
pub mod frame;
pub mod session;

pub use auth::{
    LaunchCredentials, authenticate, decode_token_file, encode_token_file, mint_launch_token,
};
pub use endpoint::{Endpoint, EndpointEnv};
pub use error::IpcError;
pub use frame::{FrameDecoder, MAX_FRAME_BYTES, encode_frame};
pub use session::{
    CatchUpDelivery, ClientSession, CommandLedger, DEFAULT_RETAINED_CAPACITY, EventDelivery,
    EventLog, GreetingOutcome, NewEvent, RecordOutcome, ReplayPlan, ServerSession,
    SessionEstablished,
};
