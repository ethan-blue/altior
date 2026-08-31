//! Core daemon server layer and main composition seam (P1.3).
//!
//! Owns [`CoreApplication`], a persistent listener loop, per-connection handshake/auth,
//! subscribe cursor tracking, command dispatching, control-priority routing, and live
//! event broadcasting. Disconnection of any client removes only its session while Core
//! and background turns continue running uninterrupted.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, DisplayName, EventId, HarnessBindingId,
    HarnessKind, MemoryMode, OperationId, PermissionDecision, PermissionListLimit, ProjectId,
    SearchQuery, ThreadCursor, ThreadId, ThreadListLimit, ThreadTitle, TurnCursor, TurnId,
    TurnListLimit, UnixMillis,
};
use altior_ipc::{CatchUpDelivery, IpcError, LaunchCredentials, ServerSession};
use altior_protocol::{
    AgentProfileDto, CancelTurnCommand, CommandEnvelope, CommandKind, ConfigureAgentCommand,
    CreateThreadCommand, DesktopHello, DiagnosticText, DiagnosticsCommand, EnvelopeLimits,
    EventBody, EventEnvelope, GetHistoryCommand, KnownEvent, ListThreadsCommand, OpenThreadCommand,
    PermissionDto, ProtocolVersion, RespondPermissionCommand, RuntimeDiagnosticsDto,
    SearchThreadsCommand, Sequence, SnapshotEnvelope, StartTurnCommand, TestHarnessBindingCommand,
    ThreadCursorDto, ThreadDto, ThreadHistoryResponseDto, ThreadListResponseDto, ThreadSnapshotDto,
    ThreadSummaryDto, TurnCursorDto, TurnDto,
};
use altior_storage::{ThreadRow, TurnRow};

use crate::application::CoreApplication;
use crate::application::connection::{InMemoryListener, IpcConnection, IpcListener};
use crate::application::error::CoreAppError;
use crate::runtime::adapters::storage::StoreCheckpointAdapter;
use crate::runtime::diagnostics::BoundedDiagnosticsSummary;
use crate::runtime::ports::HarnessRuntimePort;
use crate::runtime::state::{CancelOutcome, TurnAdmission};

/// Default timeout for completing the [`DesktopHello`] handshake (5 seconds).
pub const DEFAULT_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Default handshake timeout in milliseconds.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 5_000;

/// Default maximum number of concurrent client sessions before backpressure is applied.
pub const DEFAULT_MAX_CLIENT_SESSIONS: usize = 32;

/// Maximum number of incoming frames to process per connection in a single daemon step.
pub const MAX_FRAMES_PER_STEP_PER_SESSION: usize = 64;

/// Configuration options for launching the Core daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreDaemonConfig {
    /// Whether to run in daemon mode.
    pub is_daemon: bool,
    /// Path to data directory for SQLite database storage.
    pub data_dir: Option<PathBuf>,
    /// Target IPC endpoint name or path.
    pub endpoint: Option<String>,
    /// Path to discovery metadata output file.
    pub discovery_path: Option<PathBuf>,
    /// Handshake timeout in milliseconds. Default: 5000 ms.
    pub handshake_timeout_ms: u64,
    /// Maximum concurrent client sessions. Default: 32.
    pub max_client_sessions: usize,
}

impl Default for CoreDaemonConfig {
    fn default() -> Self {
        Self {
            is_daemon: false,
            data_dir: None,
            endpoint: None,
            discovery_path: None,
            handshake_timeout_ms: DEFAULT_HANDSHAKE_TIMEOUT_MS,
            max_client_sessions: DEFAULT_MAX_CLIENT_SESSIONS,
        }
    }
}

impl CoreDaemonConfig {
    /// Parses CLI arguments into [`CoreDaemonConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error message string if required argument values are missing.
    pub fn parse_args<I, T>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let mut config = Self::default();
        let mut iter = args.into_iter();
        let mut first = true;

        while let Some(arg) = iter.next() {
            let s = arg.as_ref();
            if first && (s.ends_with("altior-core") || s.ends_with("altior-core.exe")) {
                first = false;
                continue;
            }
            first = false;

            if s == "--daemon" {
                config.is_daemon = true;
            } else if s == "--data-dir" {
                let val = iter
                    .next()
                    .ok_or_else(|| "--data-dir requires a path argument".to_string())?;
                config.data_dir = Some(PathBuf::from(val.as_ref()));
            } else if let Some(val) = s.strip_prefix("--data-dir=") {
                config.data_dir = Some(PathBuf::from(val));
            } else if s == "--endpoint" {
                let val = iter
                    .next()
                    .ok_or_else(|| "--endpoint requires an address argument".to_string())?;
                config.endpoint = Some(val.as_ref().to_string());
            } else if let Some(val) = s.strip_prefix("--endpoint=") {
                config.endpoint = Some(val.to_string());
            } else if s == "--discovery" {
                let val = iter
                    .next()
                    .ok_or_else(|| "--discovery requires a path argument".to_string())?;
                config.discovery_path = Some(PathBuf::from(val.as_ref()));
            } else if let Some(val) = s.strip_prefix("--discovery=") {
                config.discovery_path = Some(PathBuf::from(val));
            } else if s == "--handshake-timeout-ms" {
                let val = iter.next().ok_or_else(|| {
                    "--handshake-timeout-ms requires an integer value".to_string()
                })?;
                config.handshake_timeout_ms = val
                    .as_ref()
                    .parse()
                    .map_err(|e| format!("Invalid handshake timeout: {e}"))?;
            } else if let Some(val) = s.strip_prefix("--handshake-timeout-ms=") {
                config.handshake_timeout_ms = val
                    .parse()
                    .map_err(|e| format!("Invalid handshake timeout: {e}"))?;
            } else if s == "--max-sessions" {
                let val = iter
                    .next()
                    .ok_or_else(|| "--max-sessions requires an integer value".to_string())?;
                config.max_client_sessions = val
                    .as_ref()
                    .parse()
                    .map_err(|e| format!("Invalid max sessions: {e}"))?;
            } else if let Some(val) = s.strip_prefix("--max-sessions=") {
                config.max_client_sessions = val
                    .parse()
                    .map_err(|e| format!("Invalid max sessions: {e}"))?;
            }
        }

        Ok(config)
    }
}

/// Lifecycle state of a daemon client connection session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonSessionState {
    /// Awaiting `DesktopHello` handshake.
    Handshaking,
    /// Handshake completed; awaiting `subscribe` or commands.
    Authenticated,
    /// Subscribed to live and replayed event broadcasts.
    Subscribed,
}

/// A connected client session managed by [`CoreDaemon`].
#[derive(Debug)]
pub struct DaemonClientSession<C: IpcConnection> {
    connection: C,
    server_session: ServerSession,
    state: DaemonSessionState,
    pending_control_commands: VecDeque<CommandEnvelope>,
    pending_normal_commands: VecDeque<CommandEnvelope>,
    connected_at: UnixMillis,
    last_activity_at: UnixMillis,
}

impl<C: IpcConnection> DaemonClientSession<C> {
    /// Creates a new client session with initial `Handshaking` state.
    #[must_use]
    pub fn new(connection: C, server_session: ServerSession, connected_at: UnixMillis) -> Self {
        Self {
            connection,
            server_session,
            state: DaemonSessionState::Handshaking,
            pending_control_commands: VecDeque::new(),
            pending_normal_commands: VecDeque::new(),
            connected_at,
            last_activity_at: connected_at,
        }
    }

    /// Accesses the underlying connection.
    #[must_use]
    pub fn connection(&self) -> &C {
        &self.connection
    }

    /// Mutably accesses the underlying connection.
    pub fn connection_mut(&mut self) -> &mut C {
        &mut self.connection
    }

    /// Accesses the connection state.
    #[must_use]
    pub fn state(&self) -> DaemonSessionState {
        self.state
    }

    /// Accesses the timestamp when the client connection was established.
    #[must_use]
    pub fn connected_at(&self) -> UnixMillis {
        self.connected_at
    }

    /// Accesses the timestamp of the last complete frame or activity.
    #[must_use]
    pub fn last_activity_at(&self) -> UnixMillis {
        self.last_activity_at
    }

    /// Updates the last activity timestamp.
    pub fn update_activity(&mut self, now: UnixMillis) {
        self.last_activity_at = now;
    }

    /// Sends a JSON-serializable envelope to the client.
    fn send_json<T: serde::Serialize>(&mut self, val: &T) -> Result<(), IpcError> {
        let json = serde_json::to_string(val).map_err(|e| IpcError::Protocol {
            source: altior_protocol::ProtocolError::MalformedEnvelope { source: e },
        })?;
        self.connection.send_frame(json.as_bytes())
    }
}

/// Statistics reported after executing a single daemon tick / step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DaemonStepReport {
    /// Number of new client connections accepted this step.
    pub accepted_connections: usize,
    /// Number of disconnected or closed sessions removed this step.
    pub closed_connections: usize,
    /// Number of control-priority commands executed this step.
    pub control_commands_dispatched: usize,
    /// Number of general commands executed this step.
    pub normal_commands_dispatched: usize,
    /// Number of runtime events published to domain storage and event log.
    pub events_published: usize,
}

/// The Core daemon server managing application state, client sessions, and listener.
#[derive(Debug)]
pub struct CoreDaemon<
    H = crate::runtime::adapters::acp::AcpHarnessAdapter,
    C = StoreCheckpointAdapter,
    L = InMemoryListener,
> where
    L: IpcListener,
{
    app: CoreApplication<H, C>,
    listener: L,
    sessions: Vec<DaemonClientSession<<L as IpcListener>::Connection>>,
    limits: EnvelopeLimits,
    running: Arc<AtomicBool>,
    handshake_timeout: std::time::Duration,
    idle_timeout: Option<std::time::Duration>,
    max_client_sessions: usize,
}

impl<H, L> CoreDaemon<H, StoreCheckpointAdapter, L>
where
    H: HarnessRuntimePort,
    L: IpcListener,
{
    /// Creates a new `CoreDaemon` composing the application, listener, and session manager.
    pub fn new(app: CoreApplication<H, StoreCheckpointAdapter>, listener: L) -> Self {
        Self {
            app,
            listener,
            sessions: Vec::new(),
            limits: EnvelopeLimits::default(),
            running: Arc::new(AtomicBool::new(true)),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            idle_timeout: None,
            max_client_sessions: DEFAULT_MAX_CLIENT_SESSIONS,
        }
    }

    /// Sets the handshake timeout for unauthenticated clients.
    #[must_use]
    pub fn with_handshake_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Sets the idle timeout for authenticated clients.
    #[must_use]
    pub fn with_idle_timeout(mut self, timeout: Option<std::time::Duration>) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Sets the maximum concurrent client sessions.
    #[must_use]
    pub fn with_max_client_sessions(mut self, max: usize) -> Self {
        self.max_client_sessions = max;
        self
    }

    /// Accesses the configured handshake timeout.
    #[must_use]
    pub fn handshake_timeout(&self) -> std::time::Duration {
        self.handshake_timeout
    }

    /// Accesses the configured idle timeout.
    #[must_use]
    pub fn idle_timeout(&self) -> Option<std::time::Duration> {
        self.idle_timeout
    }

    /// Accesses the maximum client sessions capacity.
    #[must_use]
    pub fn max_client_sessions(&self) -> usize {
        self.max_client_sessions
    }

    /// Accesses the underlying application.
    #[must_use]
    pub fn app(&self) -> &CoreApplication<H, StoreCheckpointAdapter> {
        &self.app
    }

    /// Mutably accesses the underlying application.
    pub fn app_mut(&mut self) -> &mut CoreApplication<H, StoreCheckpointAdapter> {
        &mut self.app
    }

    /// Number of currently active connected client sessions.
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Number of subscribed client sessions receiving live event broadcasts.
    #[must_use]
    pub fn subscribed_session_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.state == DaemonSessionState::Subscribed)
            .count()
    }

    /// Accesses the daemon's stop handle.
    #[must_use]
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Signals the daemon to shut down.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if an error occurs while shutting down.
    pub fn shutdown(&mut self) -> Result<(), CoreAppError> {
        self.running.store(false, Ordering::SeqCst);
        for session in &mut self.sessions {
            let _ = session.connection.close();
        }
        self.sessions.clear();
        Ok(())
    }

    /// Runs a single deterministic execution step:
    /// 1. Accepts pending connections from listener up to capacity.
    /// 2. Reads and categorizes incoming command frames from all connections and checks deadlines.
    /// 3. Executes control-priority commands immediately (cancel, permission, ping).
    /// 4. Executes normal commands and returns typed snapshot or result frames.
    /// 5. Pumps runtime harness events, writes to domain storage & event log, and broadcasts to subscribed clients.
    /// 6. Evicts disconnected or timed out client sessions.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage or unrecoverable error.
    #[allow(clippy::too_many_lines)]
    pub fn step(&mut self, now: UnixMillis) -> Result<DaemonStepReport, CoreAppError> {
        let mut report = DaemonStepReport::default();

        // 1. Accept new connections up to max capacity (backpressure prevents slot exhaustion)
        while self.sessions.len() < self.max_client_sessions {
            match self.listener.accept().map_err(CoreAppError::from)? {
                Some(conn) => {
                    let server_session = self.app.server_port().create_session();
                    let session = DaemonClientSession::new(conn, server_session, now);
                    self.sessions.push(session);
                    report.accepted_connections += 1;
                }
                None => break,
            }
        }

        // 2. Read incoming frames from all sessions and check deadlines
        let mut disconnected_indices = Vec::new();
        for (idx, session) in self.sessions.iter_mut().enumerate() {
            if session.connection.is_closed() {
                disconnected_indices.push(idx);
                continue;
            }

            let mut frames_read = 0;
            while frames_read < MAX_FRAMES_PER_STEP_PER_SESSION {
                let recv_res = session.connection.recv_frame();
                match recv_res {
                    Ok(Some(frame)) => {
                        session.last_activity_at = now;
                        frames_read += 1;
                        let handle_res = Self::process_incoming_frame(session, &frame);
                        if handle_res.is_err() {
                            let _ = session.connection.close();
                            disconnected_indices.push(idx);
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let _ = session.connection.close();
                        disconnected_indices.push(idx);
                        break;
                    }
                }
            }

            if session.connection.is_closed() {
                continue;
            }

            // Check handshake deadline for unauthenticated sessions
            if session.state == DaemonSessionState::Handshaking {
                let elapsed_ms = now
                    .as_millis()
                    .saturating_sub(session.connected_at.as_millis());
                let handshake_timeout_ms =
                    u64::try_from(self.handshake_timeout.as_millis()).unwrap_or(u64::MAX);
                if elapsed_ms >= handshake_timeout_ms {
                    let _ = session.connection.close();
                    disconnected_indices.push(idx);
                    continue;
                }
            }

            // Check idle deadline for authenticated sessions if configured
            if let Some(idle_timeout) = self.idle_timeout
                && session.state != DaemonSessionState::Handshaking
            {
                let idle_ms = now
                    .as_millis()
                    .saturating_sub(session.last_activity_at.as_millis());
                let idle_timeout_ms = u64::try_from(idle_timeout.as_millis()).unwrap_or(u64::MAX);
                if idle_ms >= idle_timeout_ms {
                    let _ = session.connection.close();
                    disconnected_indices.push(idx);
                }
            }
        }

        // 3. Priority Control Path: execute cancel, permission decision, and ping first
        for (idx, session) in self.sessions.iter_mut().enumerate() {
            if session.connection.is_closed() {
                continue;
            }
            while let Some(cmd) = session.pending_control_commands.pop_front() {
                let res =
                    Self::execute_control_command(&mut self.app, &self.limits, session, &cmd, now);
                if res.is_err() {
                    let _ = session.connection.close();
                    disconnected_indices.push(idx);
                    break;
                }
                report.control_commands_dispatched += 1;
            }
        }

        // 4. Normal command path
        let mut broadcast_events = Vec::new();
        for (idx, session) in self.sessions.iter_mut().enumerate() {
            if session.connection.is_closed() {
                continue;
            }
            while let Some(cmd) = session.pending_normal_commands.pop_front() {
                let res = Self::execute_normal_command(
                    &mut self.app,
                    &self.limits,
                    session,
                    &cmd,
                    &mut broadcast_events,
                    now,
                );
                if res.is_err() {
                    let _ = session.connection.close();
                    disconnected_indices.push(idx);
                    break;
                }
                report.normal_commands_dispatched += 1;
            }
        }

        // 5. Poll and pump runtime harness events and broadcast to subscribed clients
        let new_events = self.app.pump_all_threads(now)?;
        broadcast_events.extend(new_events);
        report.events_published = broadcast_events.len();

        for event in &broadcast_events {
            let json = event.to_json().map_err(CoreAppError::from)?;
            let bytes = json.as_bytes();

            for (idx, session) in self.sessions.iter_mut().enumerate() {
                if !session.connection.is_closed()
                    && session.state == DaemonSessionState::Subscribed
                    && session.connection.send_frame(bytes).is_err()
                {
                    let _ = session.connection.close();
                    disconnected_indices.push(idx);
                }
            }
        }

        // 6. Evict closed/disconnected sessions
        disconnected_indices.sort_unstable();
        disconnected_indices.dedup();
        for &idx in disconnected_indices.iter().rev() {
            if idx < self.sessions.len() {
                self.sessions.swap_remove(idx);
                report.closed_connections += 1;
            }
        }

        // 7. Accept pending connections if capacity was freed up
        while self.sessions.len() < self.max_client_sessions {
            match self.listener.accept().map_err(CoreAppError::from)? {
                Some(conn) => {
                    let server_session = self.app.server_port().create_session();
                    let session = DaemonClientSession::new(conn, server_session, now);
                    self.sessions.push(session);
                    report.accepted_connections += 1;
                }
                None => break,
            }
        }

        Ok(report)
    }

    /// Runs the daemon loop continuously until stopped by `shutdown()` or stop handle.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on fatal error.
    pub fn run_loop(&mut self, tick_interval: std::time::Duration) -> Result<(), CoreAppError> {
        while self.running.load(Ordering::SeqCst) {
            let now_millis = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(u64::MAX);
            let now = UnixMillis::from_millis(now_millis);
            let report = self.step(now)?;
            if report.accepted_connections == 0
                && report.control_commands_dispatched == 0
                && report.normal_commands_dispatched == 0
                && report.events_published == 0
            {
                std::thread::sleep(tick_interval);
            } else {
                std::thread::yield_now();
            }
        }
        self.shutdown()
    }

    fn process_incoming_frame(
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        frame: &[u8],
    ) -> Result<(), CoreAppError> {
        match session.state {
            DaemonSessionState::Handshaking => {
                let hello: DesktopHello =
                    serde_json::from_slice(frame).map_err(|e| CoreAppError::Protocol(e.into()))?;
                let established = session
                    .server_session
                    .accept_hello(&hello)
                    .map_err(CoreAppError::from)?;

                // Send CoreHello and CoreGreeting to client
                session
                    .send_json(&established.core_hello)
                    .map_err(CoreAppError::from)?;
                session
                    .send_json(&established.greeting)
                    .map_err(CoreAppError::from)?;

                session.state = DaemonSessionState::Authenticated;
                Ok(())
            }
            DaemonSessionState::Authenticated | DaemonSessionState::Subscribed => {
                let cmd: CommandEnvelope =
                    serde_json::from_slice(frame).map_err(|e| CoreAppError::Protocol(e.into()))?;

                if is_control_command(cmd.kind) {
                    session.pending_control_commands.push_back(cmd);
                } else {
                    session.pending_normal_commands.push_back(cmd);
                }
                Ok(())
            }
        }
    }

    fn execute_control_command(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        match cmd.kind {
            CommandKind::Ping => {
                let event = Self::make_command_result_event(app, limits, op_id, true, None, now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            CommandKind::Cancel => {
                let target_op = cmd.payload.as_ref().and_then(|p| {
                    p.value()
                        .get("target_operation_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| OperationId::from_str(s).ok())
                });
                let mut cancelled = false;
                for tid in app.supervisor().thread_ids() {
                    if let Ok(outcome) = app.cancel_turn(&op_id, &tid, now)
                        && outcome == CancelOutcome::CancelledActive
                    {
                        cancelled = true;
                        break;
                    }
                }
                let data = serde_json::json!({
                    "cancelled": cancelled,
                    "target_operation_id": target_op.map(|o| o.to_string()),
                });
                let event =
                    Self::make_command_result_event(app, limits, op_id, true, Some(data), now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            CommandKind::CancelTurn => {
                let payload: Result<CancelTurnCommand, _> = cmd.parse_payload();
                if let Ok(c) = payload {
                    let outcome = app.cancel_turn(&op_id, &c.thread_id, now)?;
                    let data = serde_json::json!({
                        "outcome": format!("{outcome:?}"),
                        "thread_id": c.thread_id.as_str(),
                    });
                    let event =
                        Self::make_command_result_event(app, limits, op_id, true, Some(data), now)?;
                    session.send_json(&event).map_err(CoreAppError::from)?;
                } else {
                    let err_event = Self::make_command_error_event(
                        app,
                        op_id,
                        "INVALID_PAYLOAD",
                        "malformed cancel_turn payload",
                        now,
                    )?;
                    session.send_json(&err_event).map_err(CoreAppError::from)?;
                }
            }
            CommandKind::RespondPermission => {
                let payload: Result<RespondPermissionCommand, _> = cmd.parse_payload();
                if let Ok(p) = payload {
                    let decision = match p.decision.to_lowercase().as_str() {
                        "approved" => PermissionDecision::Approved,
                        "denied" => PermissionDecision::Denied,
                        _ => {
                            let err_event = Self::make_command_error_event(
                                app,
                                op_id,
                                "INVALID_DECISION",
                                "decision must be 'approved' or 'denied'",
                                now,
                            )?;
                            session.send_json(&err_event).map_err(CoreAppError::from)?;
                            return Ok(());
                        }
                    };

                    let thread_id = Self::find_thread_for_permission(app, &p.event_id);
                    if let Some(tid) = thread_id {
                        app.decide_permission(&op_id, &tid, &p.event_id, decision, now)?;
                        let event =
                            Self::make_command_result_event(app, limits, op_id, true, None, now)?;
                        session.send_json(&event).map_err(CoreAppError::from)?;
                    } else {
                        let err_event = Self::make_command_error_event(
                            app,
                            op_id,
                            "PERMISSION_NOT_FOUND",
                            &format!("permission {} not found on any active thread", p.event_id),
                            now,
                        )?;
                        session.send_json(&err_event).map_err(CoreAppError::from)?;
                    }
                } else {
                    let err_event = Self::make_command_error_event(
                        app,
                        op_id,
                        "INVALID_PAYLOAD",
                        "malformed respond_permission payload",
                        now,
                    )?;
                    session.send_json(&err_event).map_err(CoreAppError::from)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn execute_normal_command(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        broadcast_events: &mut Vec<EventEnvelope>,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        match cmd.kind {
            CommandKind::Subscribe => Self::handle_subscribe(app, session, cmd, now),
            CommandKind::CreateThread => Self::handle_create_thread(app, limits, session, cmd, now),
            CommandKind::ListThreads => Self::handle_list_threads(app, limits, session, cmd, now),
            CommandKind::SearchThreads => {
                Self::handle_search_threads(app, limits, session, cmd, now)
            }
            CommandKind::OpenThread => Self::handle_open_thread(app, limits, session, cmd, now),
            CommandKind::GetHistory => Self::handle_get_history(app, limits, session, cmd, now),
            CommandKind::ConfigureAgent => {
                Self::handle_configure_agent(app, limits, session, cmd, now)
            }
            CommandKind::TestHarnessBinding => {
                Self::handle_test_harness_binding(app, limits, session, cmd, now)
            }
            CommandKind::StartTurn => {
                Self::handle_start_turn(app, limits, session, cmd, broadcast_events, now)
            }
            CommandKind::RuntimeStatus => Self::handle_runtime_status(app, session, cmd, now),
            CommandKind::Diagnostics => Self::handle_diagnostics(app, limits, session, cmd, now),
            CommandKind::RequestSnapshot => {
                Self::handle_request_snapshot(app, limits, session, cmd, now)
            }
            _ => Ok(()),
        }
    }

    fn handle_subscribe(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let boundary_event_id = app.event_pump().next_event_id(now)?;
        let delivery = session
            .server_session
            .accept_subscribe(cmd, boundary_event_id, now)
            .map_err(CoreAppError::from)?;

        match delivery {
            CatchUpDelivery::Replay { events, boundary } => {
                for ev in &events {
                    session.send_json(ev).map_err(CoreAppError::from)?;
                }
                session.send_json(&boundary).map_err(CoreAppError::from)?;
            }
            CatchUpDelivery::Gap { boundary } => {
                session.send_json(&boundary).map_err(CoreAppError::from)?;
            }
            CatchUpDelivery::UpToDate => {}
        }

        session.state = DaemonSessionState::Subscribed;
        Ok(())
    }

    fn handle_create_thread(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<ThreadDto, CoreAppError> {
            let payload: CreateThreadCommand =
                cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let thread_id = ThreadId::from_str(&format!("thr_{:016x}", now.as_millis()))?;
            let title = payload
                .title
                .as_deref()
                .and_then(|t| ThreadTitle::try_from(t).ok());
            let row = app.create_thread(
                thread_id,
                &payload.agent_profile_id,
                title.as_ref(),
                payload.project_id.as_ref(),
                now,
            )?;
            thread_row_to_dto(&row)
        })();

        match res {
            Ok(dto) => {
                let data =
                    serde_json::to_value(&dto).map_err(|e| CoreAppError::Other(e.to_string()))?;
                let event =
                    Self::make_command_result_event(app, limits, op_id, true, Some(data), now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "CREATE_THREAD_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_list_threads(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<ThreadListResponseDto, CoreAppError> {
            let payload: Option<ListThreadsCommand> = cmd.parse_payload().ok();
            let limit_val = payload.as_ref().and_then(|p| p.limit).unwrap_or(20);
            let limit = ThreadListLimit::try_new(limit_val)
                .map_err(|e| CoreAppError::Other(e.to_string()))?;
            let before_cursor = payload
                .as_ref()
                .and_then(|p| p.cursor.clone())
                .map(ThreadCursor::from);

            let rows = app.list_threads(None, before_cursor.as_ref(), limit)?;
            let mut summaries = Vec::new();
            for r in &rows {
                let thread_dto = thread_row_to_dto(r)?;
                let last_turn = if let Ok(tid) = ThreadId::from_str(&r.thread_id) {
                    let turn_lim = TurnListLimit::try_new(1)
                        .map_err(|e| CoreAppError::Other(e.to_string()))?;
                    app.get_thread_turns(&tid, None, turn_lim)
                        .ok()
                        .and_then(|t| t.into_iter().next())
                        .and_then(|tr| turn_row_to_dto(&tr).ok())
                } else {
                    None
                };
                summaries.push(ThreadSummaryDto {
                    thread: thread_dto,
                    last_turn,
                    active_turn: None,
                });
            }
            let has_more = summaries.len() == limit_val as usize;
            let next_cursor = summaries.last().map(|s| ThreadCursorDto {
                updated_at: s.thread.updated_at,
                thread_id: s.thread.id.clone(),
            });
            Ok(ThreadListResponseDto {
                threads: summaries,
                next_cursor,
                has_more,
            })
        })();

        match res {
            Ok(dto) => {
                let snap = SnapshotEnvelope::thread_list(&dto, op_id, now, limits)
                    .map_err(CoreAppError::from)?;
                session.send_json(&snap).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "LIST_THREADS_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_search_threads(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<ThreadListResponseDto, CoreAppError> {
            let payload: SearchThreadsCommand =
                cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let limit_val = payload.limit.unwrap_or(20);
            let limit = ThreadListLimit::try_new(limit_val)
                .map_err(|e| CoreAppError::Other(e.to_string()))?;
            let query =
                SearchQuery::try_from(payload.query.as_str()).map_err(CoreAppError::Entity)?;

            let rows = app.search_threads(&query, None, limit)?;
            let mut summaries = Vec::new();
            for r in &rows {
                let thread_dto = thread_row_to_dto(r)?;
                summaries.push(ThreadSummaryDto {
                    thread: thread_dto,
                    last_turn: None,
                    active_turn: None,
                });
            }
            let has_more = summaries.len() == limit_val as usize;
            let next_cursor = summaries.last().map(|s| ThreadCursorDto {
                updated_at: s.thread.updated_at,
                thread_id: s.thread.id.clone(),
            });
            Ok(ThreadListResponseDto {
                threads: summaries,
                next_cursor,
                has_more,
            })
        })();

        match res {
            Ok(dto) => {
                let snap = SnapshotEnvelope::thread_list(&dto, op_id, now, limits)
                    .map_err(CoreAppError::from)?;
                session.send_json(&snap).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "SEARCH_THREADS_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_open_thread(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<ThreadSnapshotDto, CoreAppError> {
            let payload: OpenThreadCommand = cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let open_res = app.open_thread(&payload.thread_id, None)?;
            let thread_dto = thread_row_to_dto(&open_res.thread)?;
            let profile_dto =
                if let Ok(pid) = AgentProfileId::from_str(&open_res.thread.agent_profile_id) {
                    app.get_agent_profile(&pid)?.map(AgentProfileDto::from)
                } else {
                    None
                };
            let turn_dtos: Vec<TurnDto> = open_res
                .turns
                .iter()
                .map(turn_row_to_dto)
                .collect::<Result<Vec<_>, _>>()?;

            let perm_limit =
                PermissionListLimit::try_new(50).map_err(|e| CoreAppError::Other(e.to_string()))?;
            let perms = app.get_thread_permissions(&payload.thread_id, None, perm_limit)?;
            let perm_dtos: Vec<PermissionDto> =
                perms.into_iter().map(PermissionDto::from).collect();

            Ok(ThreadSnapshotDto {
                thread: thread_dto,
                agent_profile: profile_dto,
                turns: turn_dtos,
                pending_permissions: perm_dtos,
            })
        })();

        match res {
            Ok(dto) => {
                let snap = SnapshotEnvelope::thread_snapshot(&dto, op_id, now, limits)
                    .map_err(CoreAppError::from)?;
                session.send_json(&snap).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "OPEN_THREAD_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_get_history(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<ThreadHistoryResponseDto, CoreAppError> {
            let payload: GetHistoryCommand = cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let limit_val = payload.limit.unwrap_or(50);
            let limit = TurnListLimit::try_new(limit_val)
                .map_err(|e| CoreAppError::Other(e.to_string()))?;
            let after_cursor = payload.cursor.map(TurnCursor::from);

            let turn_rows =
                app.get_thread_turns(&payload.thread_id, after_cursor.as_ref(), limit)?;
            let turn_dtos: Vec<TurnDto> = turn_rows
                .iter()
                .map(turn_row_to_dto)
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = turn_dtos.len() == limit_val as usize;
            let next_cursor = turn_dtos.last().map(|t| TurnCursorDto {
                started_at: t.started_at,
                turn_id: t.id.clone(),
            });

            Ok(ThreadHistoryResponseDto {
                thread_id: payload.thread_id,
                turns: turn_dtos,
                next_cursor,
                has_more,
            })
        })();

        match res {
            Ok(dto) => {
                let snap = SnapshotEnvelope::thread_history(&dto, op_id, now, limits)
                    .map_err(CoreAppError::from)?;
                session.send_json(&snap).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "GET_HISTORY_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_configure_agent(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<(), CoreAppError> {
            let payload: ConfigureAgentCommand =
                cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let profile_id = match payload.agent_profile_id {
                Some(p) => p,
                None => AgentProfileId::from_str(&format!("prf_{:016x}", now.as_millis()))
                    .map_err(CoreAppError::Id)?,
            };
            let display_name = DisplayName::try_from(payload.display_name.as_str())
                .map_err(CoreAppError::Entity)?;
            let preferred_harness = match payload.preferred_harness.as_str() {
                "terminal" => HarnessKind::Terminal,
                "native" => HarnessKind::Native,
                _ => HarnessKind::Acp,
            };
            let memory_mode = match payload.memory_mode.as_str() {
                "session" => MemoryMode::Session,
                "long_term" => MemoryMode::LongTerm,
                _ => MemoryMode::Off,
            };
            let profile = AgentProfile {
                id: profile_id,
                display_name,
                preferred_harness,
                memory_mode,
                created_at: now,
                updated_at: now,
            };
            app.configure_agent(&profile, None)
        })();

        match res {
            Ok(()) => {
                let event = Self::make_command_result_event(app, limits, op_id, true, None, now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "CONFIGURE_AGENT_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_test_harness_binding(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<serde_json::Value, CoreAppError> {
            let payload: TestHarnessBindingCommand =
                cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let binding_id = match payload.harness_binding_id {
                Some(b) => b,
                None => HarnessBindingId::from_str(&format!("hnb_{:016x}", now.as_millis()))
                    .map_err(CoreAppError::Id)?,
            };
            let agent_id = AgentProfileId::from_str("prf_default").map_err(CoreAppError::Id)?;
            let binding = AcpHarnessBinding {
                id: binding_id,
                agent_profile_id: agent_id,
                command: altior_domain::BoundedPath::try_from(payload.program.as_str())
                    .map_err(CoreAppError::Entity)?,
                label: DisplayName::try_from(payload.label.as_deref().unwrap_or("test"))
                    .map_err(CoreAppError::Entity)?,
                created_at: now,
            };
            let outcome = app.test_agent_binding(&binding)?;
            Ok(serde_json::json!({
                "ok": outcome.ok,
                "diagnostics": outcome.diagnostics.as_ref().map(BoundedDiagnosticsSummary::as_str),
            }))
        })();

        match res {
            Ok(val) => {
                let event =
                    Self::make_command_result_event(app, limits, op_id, true, Some(val), now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "TEST_HARNESS_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_start_turn(
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        broadcast_events: &mut Vec<EventEnvelope>,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<(TurnAdmission, Option<EventEnvelope>), CoreAppError> {
            let payload: StartTurnCommand = cmd.parse_payload().map_err(CoreAppError::Protocol)?;
            let turn_id = match payload.turn_id {
                Some(t) => t,
                None => TurnId::from_str(&format!("trn_{:016x}", now.as_millis()))
                    .map_err(CoreAppError::Id)?,
            };
            app.start_prompt_envelope(
                op_id.clone(),
                payload.thread_id,
                turn_id,
                payload.prompt.as_str(),
                now,
            )
        })();

        match res {
            Ok((admission, opt_envelope)) => {
                let data = serde_json::json!({
                    "admission": match admission {
                        TurnAdmission::Admitted => "admitted",
                        TurnAdmission::Duplicate => "duplicate",
                    }
                });
                let event =
                    Self::make_command_result_event(app, limits, op_id, true, Some(data), now)?;
                session.send_json(&event).map_err(CoreAppError::from)?;
                if let Some(envelope) = opt_envelope {
                    broadcast_events.push(envelope);
                }
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "START_TURN_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_runtime_status(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = app.get_status();
        match res {
            Ok(status) => {
                let active_threads = u32::try_from(status.active_thread_count).unwrap_or(u32::MAX);
                let body = EventBody::Known(KnownEvent::RuntimeStatus {
                    status: "ready".to_string(),
                    active_threads,
                    diagnostics: None,
                });
                let event = EventEnvelope {
                    protocol_version: ProtocolVersion::V1,
                    event_id: app.event_pump().next_event_id(now)?,
                    operation_id: Some(op_id),
                    thread_id: None,
                    turn_id: None,
                    sequence: Sequence::FIRST,
                    occurred_at: now,
                    body,
                };
                session.send_json(&event).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "RUNTIME_STATUS_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_diagnostics(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let res = (|| -> Result<RuntimeDiagnosticsDto, CoreAppError> {
            let payload: Option<DiagnosticsCommand> = cmd.parse_payload().ok();
            let diag = app.get_diagnostics(payload.as_ref().and_then(|p| p.thread_id.as_ref()))?;
            let active_threads = u32::try_from(diag.thread_states.len()).unwrap_or(u32::MAX);
            Ok(RuntimeDiagnosticsDto {
                instance_id: app.instance_id().clone(),
                status: "ready".to_string(),
                active_threads,
                active_turns: 0,
                summary: Some(format!("active_checkpoints: {}", diag.active_checkpoints)),
            })
        })();

        match res {
            Ok(dto) => {
                let snap = SnapshotEnvelope::runtime_diagnostics(&dto, op_id, now, limits)
                    .map_err(CoreAppError::from)?;
                session.send_json(&snap).map_err(CoreAppError::from)?;
            }
            Err(err) => {
                let err_event = Self::make_command_error_event(
                    app,
                    op_id,
                    "DIAGNOSTICS_FAILED",
                    &err.to_string(),
                    now,
                )?;
                session.send_json(&err_event).map_err(CoreAppError::from)?;
            }
        }
        Ok(())
    }

    fn handle_request_snapshot(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        session: &mut DaemonClientSession<<L as IpcListener>::Connection>,
        cmd: &CommandEnvelope,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let op_id = cmd.operation_id.clone();
        let diag = app.get_diagnostics(None)?;
        let active_threads = u32::try_from(diag.thread_states.len()).unwrap_or(u32::MAX);
        let dto = RuntimeDiagnosticsDto {
            instance_id: app.instance_id().clone(),
            status: "ready".to_string(),
            active_threads,
            active_turns: 0,
            summary: None,
        };
        let snap = SnapshotEnvelope::runtime_diagnostics(&dto, op_id, now, limits)
            .map_err(CoreAppError::from)?;
        session.send_json(&snap).map_err(CoreAppError::from)?;
        Ok(())
    }

    fn find_thread_for_permission(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        permission_id: &EventId,
    ) -> Option<ThreadId> {
        app.supervisor().thread_ids().into_iter().find(|tid| {
            app.supervisor()
                .active_permission_turn_id(tid, permission_id)
                .is_some()
        })
    }

    fn make_command_result_event(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        limits: &EnvelopeLimits,
        operation_id: OperationId,
        success: bool,
        data: Option<serde_json::Value>,
        now: UnixMillis,
    ) -> Result<EventEnvelope, CoreAppError> {
        let bounded_data = if let Some(d) = data {
            Some(altior_protocol::BoundedPayload::new(
                d,
                limits.payload_bytes,
            )?)
        } else {
            None
        };
        let body = EventBody::Known(KnownEvent::CommandResult {
            operation_id: operation_id.clone(),
            success,
            data: bounded_data,
        });
        Ok(EventEnvelope {
            protocol_version: ProtocolVersion::V1,
            event_id: app.event_pump().next_event_id(now)?,
            operation_id: Some(operation_id),
            thread_id: None,
            turn_id: None,
            sequence: Sequence::FIRST,
            occurred_at: now,
            body,
        })
    }

    fn make_command_error_event(
        app: &CoreApplication<H, StoreCheckpointAdapter>,
        operation_id: OperationId,
        code: &str,
        message: &str,
        now: UnixMillis,
    ) -> Result<EventEnvelope, CoreAppError> {
        let msg_diag = DiagnosticText::try_from(message)
            .or_else(|_| DiagnosticText::try_from("error"))
            .map_err(CoreAppError::Protocol)?;
        let body = EventBody::Known(KnownEvent::CommandError {
            operation_id: operation_id.clone(),
            code: code.to_string(),
            message: msg_diag,
        });
        Ok(EventEnvelope {
            protocol_version: ProtocolVersion::V1,
            event_id: app.event_pump().next_event_id(now)?,
            operation_id: Some(operation_id),
            thread_id: None,
            turn_id: None,
            sequence: Sequence::FIRST,
            occurred_at: now,
            body,
        })
    }
}

impl<H> CoreDaemon<H, StoreCheckpointAdapter, InMemoryListener>
where
    H: HarnessRuntimePort,
{
    /// Creates an in-memory `CoreDaemon` with an SQLite memory store and in-memory listener for tests.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage initialization failure.
    pub fn in_memory(
        harness: H,
        credentials: LaunchCredentials,
    ) -> Result<(Self, InMemoryListener), CoreAppError> {
        let app = CoreApplication::open_in_memory(harness, credentials)?;
        let listener = InMemoryListener::default();
        let daemon = Self::new(app, listener.clone());
        Ok((daemon, listener))
    }
}

fn is_control_command(kind: CommandKind) -> bool {
    matches!(
        kind,
        CommandKind::Ping
            | CommandKind::Cancel
            | CommandKind::CancelTurn
            | CommandKind::RespondPermission
    )
}

fn thread_row_to_dto(row: &ThreadRow) -> Result<ThreadDto, CoreAppError> {
    let id = ThreadId::from_str(&row.thread_id).map_err(CoreAppError::Id)?;
    let agent_profile_id =
        AgentProfileId::from_str(&row.agent_profile_id).map_err(CoreAppError::Id)?;
    let project_id = match &row.project_id {
        Some(p) => Some(ProjectId::from_str(p).map_err(CoreAppError::Id)?),
        None => None,
    };
    if row.created_at < 0 || row.updated_at < 0 {
        return Err(CoreAppError::InvalidInput(
            "Negative timestamp in thread row".to_string(),
        ));
    }
    let created_at = u64::try_from(row.created_at).map_err(|_| {
        CoreAppError::InvalidInput("Invalid created_at timestamp in thread row".to_string())
    })?;
    let updated_at = u64::try_from(row.updated_at).map_err(|_| {
        CoreAppError::InvalidInput("Invalid updated_at timestamp in thread row".to_string())
    })?;
    Ok(ThreadDto {
        id,
        agent_profile_id,
        title: row.title.clone(),
        state: row.state.clone(),
        project_id,
        created_at: UnixMillis::from_millis(created_at),
        updated_at: UnixMillis::from_millis(updated_at),
    })
}

fn turn_row_to_dto(row: &TurnRow) -> Result<TurnDto, CoreAppError> {
    let id = TurnId::from_str(&row.turn_id).map_err(CoreAppError::Id)?;
    let thread_id = ThreadId::from_str(&row.thread_id).map_err(CoreAppError::Id)?;
    let operation_id = match &row.operation_id {
        Some(op) => Some(OperationId::from_str(op).map_err(CoreAppError::Id)?),
        None => None,
    };
    if row.started_at < 0 || row.ended_at.is_some_and(|t| t < 0) {
        return Err(CoreAppError::InvalidInput(
            "Negative timestamp in turn row".to_string(),
        ));
    }
    let started_at = u64::try_from(row.started_at).map_err(|_| {
        CoreAppError::InvalidInput("Invalid started_at timestamp in turn row".to_string())
    })?;
    let ended_at = match row.ended_at {
        Some(t) => Some(UnixMillis::from_millis(u64::try_from(t).map_err(|_| {
            CoreAppError::InvalidInput("Invalid ended_at timestamp in turn row".to_string())
        })?)),
        None => None,
    };
    Ok(TurnDto {
        id,
        thread_id,
        state: row.state.clone(),
        delivery_state: row.delivery.clone(),
        operation_id,
        started_at: UnixMillis::from_millis(started_at),
        ended_at,
    })
}
