//! Local IPC server port and connection session abstractions (P1.3).
//!
//! Provides the transport-independent port for Desktop connections,
//! wrapping [`ServerSession`] over the shared Core [`EventLog`].
//! Includes [`FakeConnection`] for deterministic testing of handshake,
//! subscribe, disconnect, and reconnect replay flows.

use std::sync::{Arc, Mutex};

use altior_domain::{EventId, UnixMillis};
use altior_ipc::{
    CatchUpDelivery, ClientSession, EventDelivery, EventLog, IpcError, LaunchCredentials,
    ServerSession, SessionEstablished,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, DesktopHello, EventEnvelope, ProductVersion,
    ProtocolVersionRange,
};

/// The local IPC server port through which Desktop clients connect and authenticate.
#[derive(Clone, Debug)]
pub struct CoreServerPort {
    credentials: LaunchCredentials,
    supported_versions: ProtocolVersionRange,
    core_version: ProductVersion,
    capabilities: CapabilitySet,
    log: Arc<Mutex<EventLog>>,
}

impl CoreServerPort {
    /// Creates a new server port bound to the given credentials and shared event log.
    #[must_use]
    pub fn new(
        credentials: LaunchCredentials,
        supported_versions: ProtocolVersionRange,
        core_version: ProductVersion,
        capabilities: CapabilitySet,
        log: Arc<Mutex<EventLog>>,
    ) -> Self {
        Self {
            credentials,
            supported_versions,
            core_version,
            capabilities,
            log,
        }
    }

    /// Accesses the server's launch credentials.
    #[must_use]
    pub fn credentials(&self) -> &LaunchCredentials {
        &self.credentials
    }

    /// Accesses supported protocol versions.
    #[must_use]
    pub fn supported_versions(&self) -> ProtocolVersionRange {
        self.supported_versions
    }

    /// Accesses product version.
    #[must_use]
    pub fn core_version(&self) -> ProductVersion {
        self.core_version
    }

    /// Accesses negotiated capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Accesses shared event log reference.
    #[must_use]
    pub fn log(&self) -> &Arc<Mutex<EventLog>> {
        &self.log
    }

    /// Creates a new per-connection [`ServerSession`] over the shared event log.
    #[must_use]
    pub fn create_session(&self) -> ServerSession {
        ServerSession::with_log(
            self.credentials.clone(),
            self.supported_versions,
            self.core_version,
            self.capabilities.clone(),
            Arc::clone(&self.log),
        )
    }
}

/// A deterministic test double simulating a Desktop client connection to Core.
#[derive(Debug)]
pub struct FakeConnection {
    server_session: ServerSession,
    client_session: ClientSession,
}

impl FakeConnection {
    /// Creates a new connected session over the given server port.
    #[must_use]
    pub fn new(port: &CoreServerPort) -> Self {
        Self {
            server_session: port.create_session(),
            client_session: ClientSession::new(),
        }
    }

    /// Executes the hello/greeting handshake.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] if handshake or authentication fails.
    pub fn handshake(&mut self, hello: &DesktopHello) -> Result<SessionEstablished, IpcError> {
        let established = self.server_session.accept_hello(hello)?;
        self.client_session
            .accept_greeting(&established.greeting, &established.negotiated)?;
        Ok(established)
    }

    /// Sends a `subscribe` command and delivers catch-up events if any.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] if subscribe is invalid or out of order.
    pub fn subscribe(
        &mut self,
        command: &CommandEnvelope,
        boundary_event_id: EventId,
        now: UnixMillis,
    ) -> Result<CatchUpDelivery, IpcError> {
        let delivery = self
            .server_session
            .accept_subscribe(command, boundary_event_id, now)?;
        match &delivery {
            CatchUpDelivery::Replay { events, boundary } => {
                for event in events {
                    let _ = self.client_session.accept_event(event)?;
                }
                let _ = self.client_session.accept_event(boundary)?;
            }
            CatchUpDelivery::Gap { boundary } => {
                let _ = self.client_session.accept_event(boundary)?;
            }
            CatchUpDelivery::UpToDate => {}
        }
        Ok(delivery)
    }

    /// Delivers one live event envelope to the client session.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] if event delivery fails.
    pub fn accept_event(&mut self, envelope: &EventEnvelope) -> Result<EventDelivery, IpcError> {
        self.client_session.accept_event(envelope)
    }

    /// Accesses the underlying client session.
    #[must_use]
    pub fn client_session(&self) -> &ClientSession {
        &self.client_session
    }

    /// Mutably accesses the underlying client session.
    pub fn client_session_mut(&mut self) -> &mut ClientSession {
        &mut self.client_session
    }

    /// Accesses the underlying server session.
    #[must_use]
    pub fn server_session(&self) -> &ServerSession {
        &self.server_session
    }

    /// Mutably accesses the underlying server session.
    pub fn server_session_mut(&mut self) -> &mut ServerSession {
        &mut self.server_session
    }

    /// Simulates client disconnection, dropping the server session and preserving client state.
    #[must_use]
    pub fn disconnect(self) -> ClientSession {
        self.client_session
    }

    /// Simulates client reconnecting with preserved client session state.
    #[must_use]
    pub fn reconnect(port: &CoreServerPort, client_session: ClientSession) -> Self {
        Self {
            server_session: port.create_session(),
            client_session,
        }
    }
}
