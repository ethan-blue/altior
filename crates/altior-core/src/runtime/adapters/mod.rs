//! Infrastructure runtime adapters (P1.2).
//!
//! Provides the SQLite storage checkpoint adapter ([`StoreCheckpointAdapter`])
//! and ACP subprocess harness adapter ([`AcpHarnessAdapter`]).

pub mod acp;
pub mod storage;

pub use acp::AcpHarnessAdapter;
pub use storage::StoreCheckpointAdapter;
