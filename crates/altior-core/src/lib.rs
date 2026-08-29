//! Altior Core runtime contracts: supervision, turn ownership, and the
//! operation registry (ADR 0006).
//!
//! P0.2 core semantics, all deterministic and transport-free:
//!
//! - [`supervision`] — the spawn-or-attach supervisor as a pure state
//!   machine: probe the endpoint, decide, health-check with `ping`, and
//!   never hide a timer inside the contract;
//! - [`ownership`] — who may stop a turn: Desktop detach and reload never
//!   do; only an explicit cancel or Core's own shutdown policy;
//! - [`operations`] — Core's dedup registry, the mirror of Desktop's
//!   command ledger: a re-delivered `OperationId` is acknowledged, never
//!   executed twice.
//!
//! The bin target remains the composition root; the real OS transport and
//! process spawn arrive in the ADR 0006 OS-facing slice.

pub mod operations;
pub mod ownership;
pub mod supervision;
