//! Desktop IPC session and sequence management (ADR 0006).

use std::sync::Mutex;

use altior_domain::{OperationId, UnixMillis};
use altior_ipc::{
    ClientSession, CommandLedger, EventDelivery, GreetingOutcome, RecordOutcome,
    DEFAULT_RETAINED_CAPACITY,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CoreGreeting, DesktopHello, EnvelopeLimits, EventEnvelope,
    NegotiatedHandshake, ProductVersion, ProtocolVersion, ProtocolVersionRange, Sequence,
};

use crate::error::BridgeError;

/// Client session state maintaining epoch tracking, duplicate filtering,
/// sequence history, and command idempotency ledger.
#[derive(Debug)]
pub struct BridgeSession {
    inner: Mutex<ClientSession>,
    ledger: Mutex<CommandLedger>,
    limits: EnvelopeLimits,
    desktop_version: ProductVersion,
    supported_versions: ProtocolVersionRange,
    capabilities: CapabilitySet,
}

impl Default for BridgeSession {
    fn default() -> Self {
        let min_v = ProtocolVersion::V1;
        let max_v = ProtocolVersion::V1;
        let range = ProtocolVersionRange::try_new(min_v, max_v).expect("valid protocol range");
        let desktop_version = ProductVersion::new(0, 0, 1);

        Self {
            inner: Mutex::new(ClientSession::new()),
            ledger: Mutex::new(
                CommandLedger::new(DEFAULT_RETAINED_CAPACITY).expect("valid ledger capacity"),
            ),
            limits: EnvelopeLimits::default(),
            desktop_version,
            supported_versions: range,
            capabilities: CapabilitySet::new(),
        }
    }
}

impl BridgeSession {
    /// Creates a new bridge session with customized configuration.
    #[must_use]
    pub fn new(desktop_version: ProductVersion, range: ProtocolVersionRange) -> Self {
        Self {
            inner: Mutex::new(ClientSession::new()),
            ledger: Mutex::new(
                CommandLedger::new(DEFAULT_RETAINED_CAPACITY).expect("valid ledger capacity"),
            ),
            limits: EnvelopeLimits::default(),
            desktop_version,
            supported_versions: range,
            capabilities: CapabilitySet::new(),
        }
    }

    /// Creates a `DesktopHello` envelope for initiating handshake with Core.
    pub fn create_hello(&self, token: &altior_protocol::LaunchToken) -> DesktopHello {
        DesktopHello {
            supported_versions: self.supported_versions,
            desktop_version: self.desktop_version,
            capabilities: self.capabilities.clone(),
            launch_token: token.clone(),
        }
    }

    /// Processes a `CoreGreeting` from Core, classifying whether Core was resumed or restarted.
    pub fn accept_greeting(
        &self,
        greeting: &CoreGreeting,
        negotiated: &NegotiatedHandshake,
    ) -> Result<GreetingOutcome, BridgeError> {
        let mut session = self.inner.lock().expect("session mutex");
        let outcome = session
            .accept_greeting(greeting, negotiated)
            .map_err(BridgeError::from)?;

        if outcome == GreetingOutcome::Restarted {
            // Restarted Core invalidates the old command ledger
            self.ledger.lock().expect("ledger mutex").clear();
        }

        Ok(outcome)
    }

    /// Ingests and deduplicates an incoming event envelope.
    pub fn accept_event(&self, event: &EventEnvelope) -> Result<EventDelivery, BridgeError> {
        let mut session = self.inner.lock().expect("session mutex");
        session.accept_event(event).map_err(BridgeError::from)
    }

    /// Returns the sequence number to subscribe from (or `None` for from-now).
    pub fn subscribe_since(&self) -> Option<Sequence> {
        self.inner.lock().expect("session mutex").subscribe_since()
    }

    /// Registers a command in the idempotency ledger.
    pub fn record_command(&self, command: &CommandEnvelope) -> Result<RecordOutcome, BridgeError> {
        let mut ledger = self.ledger.lock().expect("ledger mutex");
        ledger.record(command).map_err(BridgeError::from)
    }

    /// Builds a subscribe command envelope with the appropriate catchup cursor.
    pub fn create_subscribe_command(
        &self,
        since: Option<Sequence>,
        operation_id: OperationId,
        now: UnixMillis,
    ) -> Result<CommandEnvelope, BridgeError> {
        CommandEnvelope::subscribe(since, operation_id, now, &self.limits)
            .map_err(BridgeError::from)
    }
}
