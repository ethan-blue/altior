//! IPC transport adapter and connector traits (ADR 0006).
//!
//! Abstracts the physical OS transport (Windows named pipes / Unix domain sockets)
//! so the Tauri client logic can be tested deterministically in memory, while production
//! runs over real OS IPC via [`altior_ipc::LocalStream`].

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use altior_ipc::endpoint::Endpoint;
use altior_ipc::LocalStream;

use crate::error::TransportError;

/// A bidirectional framed IPC channel between Tauri Desktop backend and Core.
pub trait CoreChannel: Send + Sync {
    /// Sends an encoded length-prefixed frame to Core.
    fn send_frame(&self, frame: &[u8]) -> Result<(), TransportError>;

    /// Reads the next complete frame from Core, with optional timeout.
    fn read_frame(&self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError>;

    /// Closes the channel.
    fn close(&self) -> Result<(), TransportError>;

    /// Checks if the channel is currently open.
    fn is_connected(&self) -> bool;

    /// Clones this channel handle if supported.
    fn try_clone_channel(&self) -> Result<Box<dyn CoreChannel>, TransportError>;
}

/// Factory for connecting to a Core endpoint.
pub trait CoreConnector: Send + Sync {
    /// Establishes a new channel connection to the specified Core endpoint.
    fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn CoreChannel>, TransportError>;
}

// ── In-Memory Mock Adapter ──────────────────────────────────────────

/// Thread-safe in-memory message queue pair for testing without OS sockets.
#[derive(Clone, Debug, Default)]
pub struct MockChannelState {
    client_to_server: Arc<Mutex<VecDeque<Vec<u8>>>>,
    server_to_client: Arc<Mutex<VecDeque<Vec<u8>>>>,
    cv: Arc<Condvar>,
    connected: Arc<Mutex<bool>>,
}

impl MockChannelState {
    /// Creates a connected mock channel pair.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client_to_server: Arc::new(Mutex::new(VecDeque::new())),
            server_to_client: Arc::new(Mutex::new(VecDeque::new())),
            cv: Arc::new(Condvar::new()),
            connected: Arc::new(Mutex::new(true)),
        }
    }

    /// Server pushes a raw JSON frame into client's incoming queue.
    pub fn push_server_frame(&self, frame: Vec<u8>) {
        let mut q = self
            .server_to_client
            .lock()
            .expect("server_to_client mutex");
        q.push_back(frame);
        self.cv.notify_all();
    }

    /// Server reads next message sent from client (payload with prefix stripped if present).
    pub fn pop_client_frame(&self) -> Option<Vec<u8>> {
        let mut q = self
            .client_to_server
            .lock()
            .expect("client_to_server mutex");
        let raw = q.pop_front()?;
        if raw.len() >= 4 {
            let len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
            if raw.len() == 4 + len {
                return Some(raw[4..].to_vec());
            }
        }
        Some(raw)
    }

    /// Checks if the channel is currently connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.connected.lock().expect("connected mutex")
    }

    /// Disconnects the channel.
    pub fn disconnect(&self) {
        let mut connected = self.connected.lock().expect("connected mutex");
        *connected = false;
        self.cv.notify_all();
    }

    /// Reopens the deterministic mock transport for a fresh physical
    /// connection to the same fake Core endpoint.
    pub fn reconnect(&self) {
        let mut connected = self.connected.lock().expect("connected mutex");
        *connected = true;
        self.cv.notify_all();
    }
}

/// Client-side mock channel implementing `CoreChannel`.
#[derive(Clone, Debug)]
pub struct MockCoreChannel {
    state: MockChannelState,
}

impl MockCoreChannel {
    /// Creates a mock channel wrapping the given state.
    #[must_use]
    pub fn new(state: MockChannelState) -> Self {
        Self { state }
    }
}

impl CoreChannel for MockCoreChannel {
    fn send_frame(&self, frame: &[u8]) -> Result<(), TransportError> {
        if !self.state.is_connected() {
            return Err(TransportError::Disconnected);
        }
        let mut q = self
            .state
            .client_to_server
            .lock()
            .expect("client_to_server mutex");
        q.push_back(frame.to_vec());
        self.state.cv.notify_all();
        Ok(())
    }

    fn read_frame(&self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        let mut q = self
            .state
            .server_to_client
            .lock()
            .expect("server_to_client mutex");

        let start = Instant::now();
        loop {
            if !self.state.is_connected() && q.is_empty() {
                return Err(TransportError::Disconnected);
            }
            if let Some(raw) = q.pop_front() {
                if raw.len() >= 4 {
                    let len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
                    if raw.len() == 4 + len {
                        return Ok(raw[4..].to_vec());
                    }
                }
                return Ok(raw);
            }

            if let Some(limit) = timeout {
                let elapsed = start.elapsed();
                if elapsed >= limit {
                    return Err(TransportError::Io("read timeout".to_string()));
                }
                let remaining = limit - elapsed;
                let (guard, result) = self
                    .state
                    .cv
                    .wait_timeout(q, remaining)
                    .expect("condvar wait");
                q = guard;
                if result.timed_out() && q.is_empty() {
                    return Err(TransportError::Io("read timeout".to_string()));
                }
            } else {
                q = self.state.cv.wait(q).expect("condvar wait");
            }
        }
    }

    fn close(&self) -> Result<(), TransportError> {
        self.state.disconnect();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.state.is_connected()
    }

    fn try_clone_channel(&self) -> Result<Box<dyn CoreChannel>, TransportError> {
        Ok(Box::new(self.clone()))
    }
}

/// In-memory mock connector for unit testing.
#[derive(Clone, Debug, Default)]
pub struct MockCoreConnector {
    channel_state: Option<MockChannelState>,
    should_fail: Arc<Mutex<bool>>,
}

impl MockCoreConnector {
    /// Creates a mock connector yielding channels connected to `channel_state`.
    #[must_use]
    pub fn new(channel_state: MockChannelState) -> Self {
        Self {
            channel_state: Some(channel_state),
            should_fail: Arc::new(Mutex::new(false)),
        }
    }

    /// Sets whether connection attempts should fail.
    pub fn set_should_fail(&self, fail: bool) {
        *self.should_fail.lock().expect("should_fail mutex") = fail;
    }
}

impl CoreConnector for MockCoreConnector {
    fn connect(&self, _endpoint: &Endpoint) -> Result<Box<dyn CoreChannel>, TransportError> {
        if *self.should_fail.lock().expect("should_fail mutex") {
            return Err(TransportError::Connect(
                "Mock connection deliberately failed".to_string(),
            ));
        }
        let state = self.channel_state.clone().unwrap_or_default();
        state.reconnect();
        Ok(Box::new(MockCoreChannel::new(state)))
    }
}

// ── Real OS Pipe / Socket Adapter ───────────────────────────────────

/// Production OS connector connecting to Core via [`altior_ipc::LocalStream`].
#[derive(Debug, Default)]
pub struct OsPipeConnector;

impl OsPipeConnector {
    /// Creates a new production OS connector.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl CoreConnector for OsPipeConnector {
    fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn CoreChannel>, TransportError> {
        let stream = LocalStream::connect(endpoint, Some(Duration::from_secs(5)))
            .map_err(|e| TransportError::Connect(e.to_string()))?;
        Ok(Box::new(OsPipeChannel::new(stream)))
    }
}

/// Bidirectional IPC channel backed by a real [`LocalStream`].
pub struct OsPipeChannel {
    stream: Arc<Mutex<LocalStream>>,
    connected: Arc<Mutex<bool>>,
}

impl OsPipeChannel {
    /// Wraps a connected [`LocalStream`] in an IPC channel.
    #[must_use]
    pub fn new(stream: LocalStream) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            connected: Arc::new(Mutex::new(true)),
        }
    }
}

impl CoreChannel for OsPipeChannel {
    fn send_frame(&self, frame: &[u8]) -> Result<(), TransportError> {
        if !self.is_connected() {
            return Err(TransportError::Disconnected);
        }
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| TransportError::Disconnected)?;
        stream.send_raw_frame(frame).map_err(|e| {
            if let Ok(mut conn) = self.connected.lock() {
                *conn = false;
            }
            TransportError::Io(e.to_string())
        })
    }

    fn read_frame(&self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        if !self.is_connected() {
            return Err(TransportError::Disconnected);
        }

        let start = Instant::now();
        let timeout_limit = timeout.unwrap_or(Duration::from_secs(30));

        loop {
            let mut stream = self
                .stream
                .lock()
                .map_err(|_| TransportError::Disconnected)?;

            match stream.try_recv_frame() {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {
                    drop(stream);
                    if start.elapsed() >= timeout_limit {
                        return Err(TransportError::Io("read timeout".to_string()));
                    }
                    std::thread::yield_now();
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(err) => {
                    if let Ok(mut conn) = self.connected.lock() {
                        *conn = false;
                    }
                    return match err {
                        altior_ipc::IpcError::EndpointUnavailable { .. } => {
                            Err(TransportError::Disconnected)
                        }
                        altior_ipc::IpcError::FrameTooLarge {
                            size_bytes,
                            limit_bytes,
                        } => Err(TransportError::Frame(format!(
                            "frame size {size_bytes} exceeds limit {limit_bytes}"
                        ))),
                        other => Err(TransportError::Io(other.to_string())),
                    };
                }
            }
        }
    }

    fn close(&self) -> Result<(), TransportError> {
        if let Ok(mut conn) = self.connected.lock() {
            *conn = false;
        }
        if let Ok(mut stream) = self.stream.lock() {
            let _ = stream.close();
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        if let Ok(conn) = self.connected.lock() {
            if !*conn {
                return false;
            }
        }
        if let Ok(stream) = self.stream.lock() {
            !stream.is_closed()
        } else {
            false
        }
    }

    fn try_clone_channel(&self) -> Result<Box<dyn CoreChannel>, TransportError> {
        let stream = self
            .stream
            .lock()
            .map_err(|_| TransportError::Disconnected)?;
        let cloned = stream
            .try_clone()
            .map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(Box::new(OsPipeChannel::new(cloned)))
    }
}
