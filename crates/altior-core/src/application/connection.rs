//! Abstract IPC connection and listener port contracts with in-memory duplex implementations (P1.3).
//!
//! Provides the transport-neutral abstractions [`IpcConnection`] and [`IpcListener`]
//! that allow Core to run over memory channels in deterministic tests, or over real
//! OS named pipes and Unix domain sockets via adapter implementations.

use std::collections::VecDeque;
use std::io::Write as _;
use std::sync::{Arc, Mutex};

use altior_domain::UnixMillis;
use altior_ipc::{IpcError, MAX_FRAME_BYTES};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CoreGreeting, CoreHello, DesktopHello, EnvelopeLimits,
    ProductVersion, ProtocolVersion, ProtocolVersionRange,
};

/// Abstract duplex framed IPC connection port.
pub trait IpcConnection: std::fmt::Debug + Send {
    /// Reads the next incoming JSON frame from the connection if available.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on framing, size limit, or decoding failure.
    fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, IpcError>;

    /// Sends a serialized JSON frame over the connection.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on size violation or closed connection.
    fn send_frame(&mut self, frame: &[u8]) -> Result<(), IpcError>;

    /// Explicitly closes the connection.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on failure.
    fn close(&mut self) -> Result<(), IpcError>;

    /// Returns `true` if this connection has been closed by either side.
    fn is_closed(&self) -> bool;
}

/// Abstract listener port accepting incoming client connections.
pub trait IpcListener: std::fmt::Debug + Send {
    /// The concrete connection type produced by this listener.
    type Connection: IpcConnection;

    /// Accepts an incoming connection without blocking indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on accept failure.
    fn accept(&mut self) -> Result<Option<Self::Connection>, IpcError>;

    /// The local endpoint address or identifier (e.g. pipe name, UDS path, "in-memory").
    fn endpoint(&self) -> Option<String>;
}

// ── In-Memory Duplex Channel ────────────────────────────────────────

#[derive(Debug)]
struct SharedDuplexState {
    client_to_server: VecDeque<Vec<u8>>,
    server_to_client: VecDeque<Vec<u8>>,
    client_closed: bool,
    server_closed: bool,
    max_frame_bytes: usize,
}

/// The server side of an in-memory duplex connection.
#[derive(Clone, Debug)]
pub struct InMemoryDuplexConnection {
    state: Arc<Mutex<SharedDuplexState>>,
}

impl InMemoryDuplexConnection {
    fn new(state: Arc<Mutex<SharedDuplexState>>) -> Self {
        Self { state }
    }
}

impl IpcConnection for InMemoryDuplexConnection {
    fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        if let Some(frame) = guard.client_to_server.pop_front() {
            if frame.len() > guard.max_frame_bytes {
                return Err(IpcError::FrameTooLarge {
                    size_bytes: frame.len(),
                    limit_bytes: guard.max_frame_bytes,
                });
            }
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        if guard.server_closed || guard.client_closed {
            return Err(IpcError::SessionOrder {
                attempted: "send frame over closed connection",
            });
        }
        if frame.len() > guard.max_frame_bytes {
            return Err(IpcError::FrameTooLarge {
                size_bytes: frame.len(),
                limit_bytes: guard.max_frame_bytes,
            });
        }
        guard.server_to_client.push_back(frame.to_vec());
        Ok(())
    }

    fn close(&mut self) -> Result<(), IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        guard.server_closed = true;
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |g| g.server_closed || g.client_closed)
    }
}

/// The client side of an in-memory duplex connection used by tests.
#[derive(Clone, Debug)]
pub struct InMemoryClient {
    state: Arc<Mutex<SharedDuplexState>>,
}

impl InMemoryClient {
    /// Sends a JSON frame over the client connection.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on size limit violation or closed channel.
    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        if guard.client_closed || guard.server_closed {
            return Err(IpcError::SessionOrder {
                attempted: "send frame from closed client",
            });
        }
        if frame.len() > guard.max_frame_bytes {
            return Err(IpcError::FrameTooLarge {
                size_bytes: frame.len(),
                limit_bytes: guard.max_frame_bytes,
            });
        }
        guard.client_to_server.push_back(frame.to_vec());
        Ok(())
    }

    /// Sends a raw byte slice directly without pre-flight size checks (used to test daemon size enforcement).
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on lock failure.
    pub fn send_raw_bytes(&mut self, raw: &[u8]) -> Result<(), IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        guard.client_to_server.push_back(raw.to_vec());
        Ok(())
    }

    /// Reads the next incoming JSON frame from the server.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on lock failure.
    pub fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, IpcError> {
        let mut guard = self.state.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock duplex state",
        })?;
        Ok(guard.server_to_client.pop_front())
    }

    /// Convenience method to send a serializable payload as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on serialization or send failure.
    pub fn send_json<T: serde::Serialize>(&mut self, value: &T) -> Result<(), IpcError> {
        let json = serde_json::to_string(value).map_err(|e| IpcError::Protocol {
            source: altior_protocol::ProtocolError::MalformedEnvelope { source: e },
        })?;
        self.send_frame(json.as_bytes())
    }

    /// Convenience method to receive and parse a JSON envelope of type `T`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on decoding failure.
    pub fn recv_json<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>, IpcError> {
        let opt_bytes = self.recv_frame()?;
        if let Some(bytes) = opt_bytes {
            let value = serde_json::from_slice(&bytes).map_err(|e| IpcError::Protocol {
                source: altior_protocol::ProtocolError::MalformedEnvelope { source: e },
            })?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Performs the P0.2 `DesktopHello` handshake against Core and receives `(CoreHello, CoreGreeting)`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] if handshake fails or is rejected.
    pub fn handshake(
        &mut self,
        launch_token: &str,
        desktop_version: ProductVersion,
    ) -> Result<(CoreHello, CoreGreeting), IpcError> {
        let hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .map_err(IpcError::from)?,
            desktop_version,
            capabilities: CapabilitySet::new(),
            launch_token: launch_token.parse().map_err(IpcError::from)?,
        };
        self.send_json(&hello)?;

        let core_hello: CoreHello = self.recv_json()?.ok_or(IpcError::SessionOrder {
            attempted: "receive CoreHello reply",
        })?;
        let greeting: CoreGreeting = self.recv_json()?.ok_or(IpcError::SessionOrder {
            attempted: "receive CoreGreeting reply",
        })?;

        Ok((core_hello, greeting))
    }

    /// Subscribes to the event stream with an optional catch-up sequence.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] on send failure or sequence parse error.
    pub fn subscribe(
        &mut self,
        operation_id: &str,
        since: Option<u64>,
        issued_at: UnixMillis,
    ) -> Result<(), IpcError> {
        let op = std::str::FromStr::from_str(operation_id).map_err(|_| IpcError::SessionOrder {
            attempted: "parse operation id",
        })?;
        let seq = match since {
            Some(s) => Some(altior_protocol::Sequence::try_new(s).map_err(IpcError::from)?),
            None => None,
        };
        let limits = EnvelopeLimits::default();
        let cmd = CommandEnvelope::subscribe(seq, op, issued_at, &limits)?;
        self.send_json(&cmd)
    }

    /// Explicitly closes the client connection.
    pub fn disconnect(self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.client_closed = true;
        }
    }

    /// Returns `true` if closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |g| g.client_closed || g.server_closed)
    }
}

/// An in-memory listener producing paired duplex connections.
#[derive(Clone, Debug)]
pub struct InMemoryListener {
    pending: Arc<Mutex<VecDeque<InMemoryDuplexConnection>>>,
    endpoint: String,
    max_frame_bytes: usize,
}

impl Default for InMemoryListener {
    fn default() -> Self {
        Self::new("in-memory-local")
    }
}

impl InMemoryListener {
    /// Creates a new in-memory listener.
    #[must_use]
    pub fn new(endpoint: &str) -> Self {
        Self {
            pending: Arc::new(Mutex::new(VecDeque::new())),
            endpoint: endpoint.to_owned(),
            max_frame_bytes: MAX_FRAME_BYTES,
        }
    }

    /// Sets a custom frame limit for tests.
    #[must_use]
    pub fn with_max_frame_bytes(mut self, max_bytes: usize) -> Self {
        self.max_frame_bytes = max_bytes;
        self
    }

    /// Creates and connects a new in-memory client, queuing the server side into this listener.
    #[must_use]
    pub fn create_client(&self) -> InMemoryClient {
        let shared = Arc::new(Mutex::new(SharedDuplexState {
            client_to_server: VecDeque::new(),
            server_to_client: VecDeque::new(),
            client_closed: false,
            server_closed: false,
            max_frame_bytes: self.max_frame_bytes,
        }));

        let server_conn = InMemoryDuplexConnection::new(Arc::clone(&shared));
        let client_conn = InMemoryClient { state: shared };

        if let Ok(mut guard) = self.pending.lock() {
            guard.push_back(server_conn);
        }

        client_conn
    }
}

impl IpcListener for InMemoryListener {
    type Connection = InMemoryDuplexConnection;

    fn accept(&mut self) -> Result<Option<Self::Connection>, IpcError> {
        let mut guard = self.pending.lock().map_err(|_| IpcError::SessionOrder {
            attempted: "lock pending listener connections",
        })?;
        Ok(guard.pop_front())
    }

    fn endpoint(&self) -> Option<String> {
        Some(self.endpoint.clone())
    }
}

// ── OS IPC Transport Implementations ─────────────────────────────────

impl IpcConnection for altior_ipc::LocalStream {
    fn recv_frame(&mut self) -> Result<Option<Vec<u8>>, IpcError> {
        self.try_recv_frame()
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), IpcError> {
        if self.is_closed() {
            return Err(IpcError::SessionOrder {
                attempted: "send frame over closed connection",
            });
        }
        if frame.len() > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge {
                size_bytes: frame.len(),
                limit_bytes: MAX_FRAME_BYTES,
            });
        }
        let length = u32::try_from(frame.len()).map_err(|_| IpcError::FrameTooLarge {
            size_bytes: frame.len(),
            limit_bytes: MAX_FRAME_BYTES,
        })?;
        self.write_all(&length.to_be_bytes())?;
        self.write_all(frame)?;
        self.flush()?;
        Ok(())
    }

    fn close(&mut self) -> Result<(), IpcError> {
        altior_ipc::LocalStream::close(self)
    }

    fn is_closed(&self) -> bool {
        altior_ipc::LocalStream::is_closed(self)
    }
}

impl IpcListener for altior_ipc::LocalListener {
    type Connection = altior_ipc::LocalStream;

    fn accept(&mut self) -> Result<Option<Self::Connection>, IpcError> {
        self.try_accept()
    }

    fn endpoint(&self) -> Option<String> {
        Some(self.endpoint().address().to_string())
    }
}
