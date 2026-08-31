//! AppIpcState: Background Core connection manager, command dispatcher, and event distributor (P1.3).
//!
//! Non-blocking rule: IPC I/O, process spawning, and event streaming run on background
//! worker threads and async Tokio tasks, never blocking Tauri's main UI thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use altior_domain::OperationId;
use altior_ipc::encode_frame;
use altior_protocol::{
    CommandEnvelope, CommandKind, EnvelopeLimits, EventBody, EventEnvelope, KnownEvent,
    NegotiatedHandshake, SnapshotEnvelope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{CoreChannel, OsPipeConnector};
use crate::discovery::FsCoreDiscovery;
use crate::error::BridgeError;
use crate::manager::SpawnOrAttachManager;
use crate::session::BridgeSession;
use crate::spawner::DetachedCoreSpawner;

/// Reconnect cursor specification matching frontend `ReconnectCursor`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconnectCursor {
    /// The last processed sequence number received by the client.
    pub last_sequence: Option<u64>,
    /// Optional cursor identifier or snapshot token.
    pub cursor_id: Option<String>,
}

/// Transport lifecycle status matching frontend `TransportStatus`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Unavailable,
    Closed,
}

impl TransportStatus {
    /// String representation of status matching frontend contracts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Unavailable => "unavailable",
            Self::Closed => "closed",
        }
    }
}

/// Commands passed from Tauri commands to the background IPC worker.
enum WorkerMsg {
    Handshake {
        _client: Option<String>,
        reply: SyncSender<Result<NegotiatedHandshake, BridgeError>>,
    },
    Command {
        envelope: CommandEnvelope,
        reply: SyncSender<Result<Value, BridgeError>>,
    },
    Reconnect {
        cursor: Option<ReconnectCursor>,
        reply: SyncSender<Result<NegotiatedHandshake, BridgeError>>,
    },
    Close {
        reply: SyncSender<Result<(), BridgeError>>,
    },
}

type PendingMap = Arc<Mutex<HashMap<OperationId, SyncSender<Result<Value, BridgeError>>>>>;
type EventCallback = Arc<dyn Fn(EventEnvelope) + Send + Sync>;

/// Tauri application state managing the Core IPC lifecycle.
pub struct AppIpcState {
    status: Arc<RwLock<TransportStatus>>,
    handshake: Arc<RwLock<Option<NegotiatedHandshake>>>,
    last_sequence: Arc<AtomicU64>,
    manager: Arc<SpawnOrAttachManager>,
    event_subscribers: Arc<Mutex<Vec<EventCallback>>>,
    worker_tx: Sender<WorkerMsg>,
    limits: EnvelopeLimits,
}

impl AppIpcState {
    /// Creates a production `AppIpcState` with standard OS discovery, spawner, and connector.
    #[must_use]
    pub fn new_default() -> Self {
        let session = Arc::new(BridgeSession::default());
        let discovery = Arc::new(FsCoreDiscovery::from_env());
        let spawner = Arc::new(DetachedCoreSpawner::new());
        let connector = Arc::new(OsPipeConnector::new());
        let manager = Arc::new(SpawnOrAttachManager::new(
            discovery, spawner, connector, session,
        ));
        Self::with_manager(manager)
    }

    /// Creates an `AppIpcState` with a custom manager (used for injection & test fakes).
    #[must_use]
    pub fn with_manager(manager: Arc<SpawnOrAttachManager>) -> Self {
        let (worker_tx, worker_rx) = channel::<WorkerMsg>();
        let status = Arc::new(RwLock::new(TransportStatus::Disconnected));
        let handshake = Arc::new(RwLock::new(None));
        let last_sequence = Arc::new(AtomicU64::new(0));
        let event_subscribers: Arc<Mutex<Vec<EventCallback>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_commands: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let limits = EnvelopeLimits::default();

        let state = Self {
            status: Arc::clone(&status),
            handshake: Arc::clone(&handshake),
            last_sequence: Arc::clone(&last_sequence),
            manager: Arc::clone(&manager),
            event_subscribers: Arc::clone(&event_subscribers),
            worker_tx,
            limits,
        };

        // Spawn background worker thread
        let worker_status = Arc::clone(&status);
        let worker_handshake = Arc::clone(&handshake);
        let worker_last_seq = Arc::clone(&last_sequence);
        let worker_subscribers = Arc::clone(&event_subscribers);
        let worker_pending = Arc::clone(&pending_commands);
        let worker_manager = Arc::clone(&manager);

        thread::Builder::new()
            .name("altior-ipc-worker".to_string())
            .spawn(move || {
                run_worker_loop(
                    worker_rx,
                    worker_status,
                    worker_handshake,
                    worker_last_seq,
                    worker_subscribers,
                    worker_pending,
                    worker_manager,
                );
            })
            .expect("spawn altior-ipc-worker thread");

        state
    }

    /// Returns current connection status.
    #[must_use]
    pub fn status(&self) -> TransportStatus {
        *self.status.read().expect("status rwlock")
    }

    /// Returns string representation of current connection status.
    #[must_use]
    pub fn status_string(&self) -> String {
        self.status().as_str().to_string()
    }

    /// Returns the last seen sequence number.
    #[must_use]
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence.load(Ordering::SeqCst)
    }

    /// Subscribes to events emitted from Core.
    pub fn subscribe_events<F>(&self, callback: F)
    where
        F: Fn(EventEnvelope) + Send + Sync + 'static,
    {
        let mut subs = self.event_subscribers.lock().expect("subscribers mutex");
        subs.push(Arc::new(callback));
    }

    /// Connects and performs handshake with Core (async / non-blocking).
    pub async fn handshake(
        &self,
        client: Option<String>,
    ) -> Result<NegotiatedHandshake, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Handshake {
                _client: client,
                reply: tx,
            })
            .map_err(|_| BridgeError::Internal("Worker thread dead".to_string()))?;

        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_millis(5000))
                .map_err(|_| BridgeError::Timeout("Handshake timed out".to_string()))?
        })
        .await
        .map_err(|_| BridgeError::Internal("Task join error".to_string()))?
    }

    /// Dispatches a command envelope to Core (async / non-blocking).
    pub async fn command(&self, envelope: CommandEnvelope) -> Result<Value, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Command {
                envelope,
                reply: tx,
            })
            .map_err(|_| BridgeError::Internal("Worker thread dead".to_string()))?;

        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_millis(10000))
                .map_err(|_| BridgeError::Timeout("Command execution timed out".to_string()))?
        })
        .await
        .map_err(|_| BridgeError::Internal("Task join error".to_string()))?
    }

    /// Reconnects with an optional sequence cursor (async / non-blocking).
    pub async fn reconnect(
        &self,
        cursor: Option<ReconnectCursor>,
    ) -> Result<NegotiatedHandshake, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Reconnect { cursor, reply: tx })
            .map_err(|_| BridgeError::Internal("Worker thread dead".to_string()))?;

        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_millis(6000))
                .map_err(|_| BridgeError::Timeout("Reconnect timed out".to_string()))?
        })
        .await
        .map_err(|_| BridgeError::Internal("Task join error".to_string()))?
    }

    /// Closes the transport connection (async / non-blocking).
    pub async fn close(&self) -> Result<(), BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Close { reply: tx })
            .map_err(|_| BridgeError::Internal("Worker thread dead".to_string()))?;

        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_millis(3000))
                .map_err(|_| BridgeError::Timeout("Close timed out".to_string()))?
        })
        .await
        .map_err(|_| BridgeError::Internal("Task join error".to_string()))?
    }

    /// Direct sync handshake method for tests.
    pub fn handshake_sync(
        &self,
        client: Option<String>,
    ) -> Result<NegotiatedHandshake, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Handshake {
                _client: client,
                reply: tx,
            })
            .map_err(|_| BridgeError::Internal("Worker dead".to_string()))?;
        rx.recv()
            .map_err(|_| BridgeError::Internal("Recv error".to_string()))?
    }

    /// Returns currently negotiated handshake details, if connected.
    #[must_use]
    pub fn handshake_info(&self) -> Option<NegotiatedHandshake> {
        self.handshake.read().ok()?.clone()
    }

    /// Returns manager reference.
    #[must_use]
    pub fn manager(&self) -> &Arc<SpawnOrAttachManager> {
        &self.manager
    }

    /// Returns envelope limits.
    #[must_use]
    pub fn limits(&self) -> &EnvelopeLimits {
        &self.limits
    }

    /// Direct sync command method for tests.
    pub fn command_sync(&self, envelope: CommandEnvelope) -> Result<Value, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Command {
                envelope,
                reply: tx,
            })
            .map_err(|_| BridgeError::Internal("Worker dead".to_string()))?;
        rx.recv()
            .map_err(|_| BridgeError::Internal("Recv error".to_string()))?
    }

    /// Direct sync reconnect method for tests.
    pub fn reconnect_sync(
        &self,
        cursor: Option<ReconnectCursor>,
    ) -> Result<NegotiatedHandshake, BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Reconnect { cursor, reply: tx })
            .map_err(|_| BridgeError::Internal("Worker dead".to_string()))?;
        rx.recv()
            .map_err(|_| BridgeError::Internal("Recv error".to_string()))?
    }

    /// Direct sync close method for tests.
    pub fn close_sync(&self) -> Result<(), BridgeError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(WorkerMsg::Close { reply: tx })
            .map_err(|_| BridgeError::Internal("Worker dead".to_string()))?;
        rx.recv()
            .map_err(|_| BridgeError::Internal("Recv error".to_string()))?
    }
}

/// The core background worker event loop.
#[allow(clippy::too_many_arguments)]
fn run_worker_loop(
    worker_rx: Receiver<WorkerMsg>,
    status: Arc<RwLock<TransportStatus>>,
    handshake: Arc<RwLock<Option<NegotiatedHandshake>>>,
    last_sequence: Arc<AtomicU64>,
    subscribers: Arc<Mutex<Vec<EventCallback>>>,
    pending_commands: PendingMap,
    manager: Arc<SpawnOrAttachManager>,
) {
    let mut channel: Option<Box<dyn CoreChannel>> = None;
    let stop_reader = Arc::new(AtomicBool::new(false));
    let mut reader_handle: Option<JoinHandle<()>> = None;

    while let Ok(msg) = worker_rx.recv() {
        match msg {
            WorkerMsg::Handshake { reply, .. } => {
                // If already connected and channel valid, return cached handshake
                if *status.read().expect("status lock") == TransportStatus::Connected {
                    if let Some(h) = handshake.read().expect("handshake lock").clone() {
                        let _ = reply.send(Ok(h));
                        continue;
                    }
                }

                *status.write().expect("status lock") = TransportStatus::Connecting;

                match manager.attach_or_spawn() {
                    Ok((ch, negotiated, _greeting)) => {
                        *handshake.write().expect("handshake lock") = Some(negotiated.clone());
                        *status.write().expect("status lock") = TransportStatus::Connected;

                        // Stop old reader if running
                        stop_reader.store(true, Ordering::SeqCst);
                        if let Some(h) = reader_handle.take() {
                            let _ = h.join();
                        }
                        stop_reader.store(false, Ordering::SeqCst);

                        // Start reader thread for incoming events
                        let reader_channel = match ch.try_clone_channel() {
                            Ok(c) => c,
                            Err(_) => {
                                channel = Some(ch);
                                let _ = reply.send(Ok(negotiated));
                                continue;
                            }
                        };

                        let r_subscribers = Arc::clone(&subscribers);
                        let r_pending = Arc::clone(&pending_commands);
                        let r_last_seq = Arc::clone(&last_sequence);
                        let r_status = Arc::clone(&status);
                        let r_stop = Arc::clone(&stop_reader);
                        let r_session = Arc::clone(manager.session());

                        reader_handle = Some(
                            thread::Builder::new()
                                .name("altior-ipc-reader".to_string())
                                .spawn(move || {
                                    run_reader_loop(
                                        reader_channel,
                                        r_subscribers,
                                        r_pending,
                                        r_last_seq,
                                        r_status,
                                        r_stop,
                                        r_session,
                                    );
                                })
                                .expect("spawn altior-ipc-reader"),
                        );

                        channel = Some(ch);
                        let _ = reply.send(Ok(negotiated));
                    }
                    Err(err) => {
                        *status.write().expect("status lock") = TransportStatus::Unavailable;
                        let _ = reply.send(Err(err));
                    }
                }
            }

            WorkerMsg::Command { envelope, reply } => {
                let current_status = *status.read().expect("status lock");
                if current_status == TransportStatus::Closed {
                    let _ = reply.send(Err(BridgeError::ConnectionClosed));
                    continue;
                }
                if current_status != TransportStatus::Connected {
                    let _ = reply.send(Err(BridgeError::TransportUnavailable(
                        "Not connected to Core".to_string(),
                    )));
                    continue;
                }

                // Check idempotency ledger
                match manager.session().record_command(&envelope) {
                    Ok(altior_ipc::RecordOutcome::AlreadyIssued) => {
                        let _ = reply.send(Ok(serde_json::json!({
                            "acknowledged": true,
                            "duplicate": true,
                            "operation_id": envelope.operation_id.as_str()
                        })));
                        continue;
                    }
                    Err(err) => {
                        let _ = reply.send(Err(err));
                        continue;
                    }
                    Ok(altior_ipc::RecordOutcome::Recorded) => {}
                }

                let op_id = envelope.operation_id.clone();
                let is_ping = envelope.kind == CommandKind::Ping;

                // Send frame over channel
                if let Some(ref ch) = channel {
                    let json_str = match serde_json::to_string(&envelope) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = reply.send(Err(BridgeError::Serialization(e.to_string())));
                            continue;
                        }
                    };
                    let frame = match encode_frame(&json_str) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = reply.send(Err(BridgeError::from(e)));
                            continue;
                        }
                    };

                    if let Err(e) = ch.send_frame(&frame) {
                        let _ = reply.send(Err(BridgeError::from(e)));
                        continue;
                    }

                    if is_ping {
                        let _ = reply.send(Ok(serde_json::json!({ "ok": true, "echo": "ping" })));
                        continue;
                    }

                    // Register in pending commands map
                    pending_commands
                        .lock()
                        .expect("pending lock")
                        .insert(op_id, reply);
                } else {
                    let _ = reply.send(Err(BridgeError::TransportUnavailable(
                        "Channel is None".to_string(),
                    )));
                }
            }

            WorkerMsg::Reconnect { cursor, reply } => {
                *status.write().expect("status lock") = TransportStatus::Reconnecting;

                // Stop old reader thread and close old channel
                stop_reader.store(true, Ordering::SeqCst);
                if let Some(h) = reader_handle.take() {
                    let _ = h.join();
                }
                stop_reader.store(false, Ordering::SeqCst);
                if let Some(ref ch) = channel {
                    let _ = ch.close();
                }
                channel = None;

                if let Some(ref cur) = cursor {
                    if let Some(seq_num) = cur.last_sequence {
                        last_sequence.store(seq_num, Ordering::SeqCst);
                    }
                }

                match manager.attach_or_spawn() {
                    Ok((ch, negotiated, _greeting)) => {
                        *handshake.write().expect("handshake lock") = Some(negotiated.clone());
                        *status.write().expect("status lock") = TransportStatus::Connected;

                        // Start new reader thread
                        let reader_channel = match ch.try_clone_channel() {
                            Ok(c) => c,
                            Err(_) => {
                                channel = Some(ch);
                                let _ = reply.send(Ok(negotiated));
                                continue;
                            }
                        };

                        let r_subscribers = Arc::clone(&subscribers);
                        let r_pending = Arc::clone(&pending_commands);
                        let r_last_seq = Arc::clone(&last_sequence);
                        let r_status = Arc::clone(&status);
                        let r_stop = Arc::clone(&stop_reader);
                        let r_session = Arc::clone(manager.session());

                        reader_handle = Some(
                            thread::Builder::new()
                                .name("altior-ipc-reader".to_string())
                                .spawn(move || {
                                    run_reader_loop(
                                        reader_channel,
                                        r_subscribers,
                                        r_pending,
                                        r_last_seq,
                                        r_status,
                                        r_stop,
                                        r_session,
                                    );
                                })
                                .expect("spawn altior-ipc-reader"),
                        );

                        channel = Some(ch);
                        let _ = reply.send(Ok(negotiated));
                    }
                    Err(err) => {
                        *status.write().expect("status lock") = TransportStatus::Disconnected;
                        let _ = reply.send(Err(err));
                    }
                }
            }

            WorkerMsg::Close { reply } => {
                *status.write().expect("status lock") = TransportStatus::Closed;
                stop_reader.store(true, Ordering::SeqCst);
                if let Some(ref ch) = channel {
                    let _ = ch.close();
                }
                channel = None;
                *handshake.write().expect("handshake lock") = None;

                // Drain pending commands with ConnectionClosed
                let mut pending = pending_commands.lock().expect("pending lock");
                for (_, tx) in pending.drain() {
                    let _ = tx.send(Err(BridgeError::ConnectionClosed));
                }

                let _ = reply.send(Ok(()));
            }
        }
    }
}

/// The background reader loop consuming frames and dispatching events/command completions.
fn run_reader_loop(
    channel: Box<dyn CoreChannel>,
    subscribers: Arc<Mutex<Vec<EventCallback>>>,
    pending_commands: PendingMap,
    last_sequence: Arc<AtomicU64>,
    status: Arc<RwLock<TransportStatus>>,
    stop: Arc<AtomicBool>,
    session: Arc<BridgeSession>,
) {
    while !stop.load(Ordering::SeqCst) {
        let frame = match channel.read_frame(Some(Duration::from_millis(500))) {
            Ok(f) => f,
            Err(crate::error::TransportError::Disconnected) => {
                if !stop.load(Ordering::SeqCst) {
                    *status.write().expect("status lock") = TransportStatus::Disconnected;
                }
                break;
            }
            Err(_) => {
                continue;
            }
        };

        let frame_str = String::from_utf8_lossy(&frame);
        // Try decoding as EventEnvelope
        if let Ok(event) = EventEnvelope::from_json(&frame_str) {
            // Check if this satisfies a pending command
            match &event.body {
                EventBody::Known(KnownEvent::CommandResult {
                    operation_id,
                    success,
                    data,
                }) => {
                    let mut pending = pending_commands.lock().expect("pending lock");
                    if let Some(tx) = pending.remove(operation_id) {
                        let res = if *success {
                            Ok(data.as_ref().map_or(Value::Null, |b| b.value().clone()))
                        } else {
                            Err(BridgeError::CommandFailed {
                                code: "COMMAND_FAILED".to_string(),
                                message: "Operation returned unsuccessful".to_string(),
                            })
                        };
                        let _ = tx.send(res);
                    }
                }
                EventBody::Known(KnownEvent::CommandError {
                    operation_id,
                    code,
                    message,
                }) => {
                    let mut pending = pending_commands.lock().expect("pending lock");
                    if let Some(tx) = pending.remove(operation_id) {
                        let _ = tx.send(Err(BridgeError::CommandFailed {
                            code: code.clone(),
                            message: message.as_str().to_string(),
                        }));
                    }
                }
                EventBody::Known(KnownEvent::RuntimeStatus { .. }) => {
                    if let Some(ref op_id) = event.operation_id {
                        let mut pending = pending_commands.lock().expect("pending lock");
                        if let Some(tx) = pending.remove(op_id) {
                            let res = serde_json::to_value(&event.body).unwrap_or(Value::Null);
                            let _ = tx.send(Ok(res));
                        }
                    }
                }
                _ => {}
            }

            // Deduplicate event through session
            if let Ok(altior_ipc::EventDelivery::Applied { sequence }) =
                session.accept_event(&event)
            {
                last_sequence.store(sequence.as_u64(), Ordering::SeqCst);
            }

            // Dispatch to all subscribers
            let subs = subscribers.lock().expect("subscribers lock").clone();
            for sub in subs {
                sub(event.clone());
            }
        } else if let Ok(snap) = SnapshotEnvelope::from_json(&frame_str) {
            let mut pending = pending_commands.lock().expect("pending lock");
            if let Some(tx) = pending.remove(&snap.operation_id) {
                let res = serde_json::to_value(&snap).unwrap_or(Value::Null);
                let _ = tx.send(Ok(res));
            }
        }
    }
}
