//! Local IPC contract layer between Altior Desktop and Altior Core
//! (ADR 0006).
//!
//! This crate owns everything transport-adjacent that must stay provable
//! without I/O: length-prefixed frame encoding, user-scoped endpoint
//! naming, per-launch capability tokens, discovery file publication,
//! real local OS transport (Windows named pipes and Unix domain sockets),
//! and the session state machines that drive authentication, version
//! negotiation, greeting, subscription, and catch-up.

pub mod auth;
pub mod discovery;
pub mod endpoint;
pub mod error;
pub mod frame;
pub mod session;
pub mod transport;

pub use auth::{
    LaunchCredentials, authenticate, decode_token_file, encode_token_file, generate_instance_id,
    generate_launch_token, mint_launch_token,
};
pub use discovery::{
    EndpointDiscovery, cleanup_stale_discovery, decode_discovery, encode_discovery,
    read_discovery_file, write_discovery_file,
};
pub use endpoint::{Endpoint, EndpointEnv, LocalEndpoint};
pub use error::IpcError;
pub use frame::{FrameDecoder, MAX_FRAME_BYTES, encode_frame};
pub use session::{
    CatchUpDelivery, ClientSession, CommandLedger, DEFAULT_RETAINED_CAPACITY, EventDelivery,
    EventLog, GreetingOutcome, NewEvent, RecordOutcome, ReplayPlan, ServerSession,
    SessionEstablished,
};
pub use transport::{LocalListener, LocalStream};
