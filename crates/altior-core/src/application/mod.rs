//! Core application service, command dispatcher, and event pump (P1.3).
//!
//! Composes:
//! - [`StoreCheckpointAdapter`] (SQLite domain persistence and boundary checkpoints)
//! - [`AcpHarnessAdapter`] (external agent subprocess execution)
//! - [`AgentRuntimeSupervisor`] (pure state machine supervision, capability gates, turn ownership)
//! - Bounded event pump and IPC replay log
//! - Local IPC server port and connection sessions
//! - `operation_id` deduplication ledger and typed application errors

pub mod connection;
pub mod daemon;
pub mod dispatch;
pub mod error;
pub mod event_pump;
pub mod mod_types;
pub mod session;

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileCursor, AgentProfileId, AgentProfileListLimit,
    CHECKPOINT_LIST_LIMIT_MAX, CheckpointListLimit, CheckpointState, CoreInstanceId, DeliveryState,
    DomainEvent, DomainEventKind, EventId, HarnessBindingCursor, HarnessBindingId,
    HarnessBindingListLimit, HistoryLimit, OperationId, Permission, PermissionCursor,
    PermissionDecision, PermissionListLimit, ProjectId, SearchQuery, THREAD_LIST_LIMIT_MAX,
    TURN_LIST_LIMIT_MAX, ThreadCursor, ThreadId, ThreadListLimit, ThreadState, ThreadTitle,
    TurnCursor, TurnId, TurnListLimit, UnixMillis,
};
use altior_ipc::{DEFAULT_RETAINED_CAPACITY, EventLog, LaunchCredentials};
use altior_protocol::{
    CapabilitySet, EventEnvelope, ProductVersion, ProtocolVersion, ProtocolVersionRange,
    SUPPORTED_PROTOCOL_VERSIONS,
};
use altior_storage::{DomainJournalRow, Store, ThreadRow, TurnRow};

pub use connection::{
    InMemoryClient, InMemoryDuplexConnection, InMemoryListener, IpcConnection, IpcListener,
};
pub use daemon::{
    CoreDaemon, CoreDaemonConfig, DEFAULT_HANDSHAKE_TIMEOUT, DEFAULT_HANDSHAKE_TIMEOUT_MS,
    DEFAULT_MAX_CLIENT_SESSIONS, DaemonClientSession, DaemonSessionState, DaemonStepReport,
};
pub use dispatch::{CommandDispatcher, CoreCommand, CoreCommandEnvelope, CoreCommandResponse};
pub use error::CoreAppError;
pub use event_pump::{EventIdGenerator, EventPump};
pub use mod_types::{
    CoreDiagnosticsReport, CoreStatusReport, StartupRecoveryReport, ThreadOpenResult,
};
pub use session::{CoreServerPort, FakeConnection};

use crate::operations::OperationRegistry;
use crate::runtime::adapters::acp::AcpHarnessAdapter;
use crate::runtime::adapters::storage::StoreCheckpointAdapter;

/// Returns the statically-valid fallback harness binding id used in error
/// payloads when a derived id cannot be parsed.
///
/// # Panics
///
/// Panics if the fixed constant form fails to parse, which cannot occur.
fn hsb_fallback() -> HarnessBindingId {
    HarnessBindingId::from_str("hsb_0000000000000000")
        .expect("static fallback harness binding id is valid")
}
use crate::runtime::coordinator::AgentRuntimeSupervisor;
use crate::runtime::ports::{AgentRuntime, HarnessRuntimePort, RuntimeCheckpointPort};
use crate::runtime::state::{
    BindingProbeOutcome, CancelOutcome, HarnessSessionId, RuntimeError, RuntimeEvent,
    SupervisorState, TurnAdmission,
};

/// Default capacity for the operation dedup ledger.
const DEFAULT_OPERATION_CAPACITY: usize = 256;

/// Core application root composing storage, harness adapters, supervisor, and event pump.
#[derive(Debug)]
pub struct CoreApplication<H = AcpHarnessAdapter, C = StoreCheckpointAdapter> {
    instance_id: CoreInstanceId,
    credentials: LaunchCredentials,
    supported_versions: ProtocolVersionRange,
    core_version: ProductVersion,
    capabilities: CapabilitySet,
    supervisor: AgentRuntimeSupervisor<H, C>,
    event_pump: EventPump,
    operations: OperationRegistry,
}

impl<H, C> CoreApplication<H, C>
where
    H: HarnessRuntimePort,
    C: RuntimeCheckpointPort,
{
    /// Creates a new `CoreApplication` over injectable harness and checkpoint ports.
    ///
    /// # Panics
    ///
    /// Panics if static package version or instance ID cannot be parsed.
    pub fn new(harness: H, checkpoint: C, credentials: LaunchCredentials) -> Self {
        let core_version = ProductVersion::from_str(env!("CARGO_PKG_VERSION"))
            .expect("valid workspace semver version");
        let supported_versions = SUPPORTED_PROTOCOL_VERSIONS;
        let capabilities = CapabilitySet::new();

        let event_log = Arc::new(Mutex::new(
            EventLog::new(DEFAULT_RETAINED_CAPACITY).expect("non-zero retained capacity"),
        ));
        let event_pump = EventPump::new(Arc::clone(&event_log), ProtocolVersion::V1);
        let operations = OperationRegistry::new(DEFAULT_OPERATION_CAPACITY)
            .expect("non-zero operation capacity");

        Self {
            instance_id: credentials.instance_id.clone(),
            credentials,
            supported_versions,
            core_version,
            capabilities,
            supervisor: AgentRuntimeSupervisor::new(harness, checkpoint),
            event_pump,
            operations,
        }
    }

    /// The Core launch instance ID.
    #[must_use]
    pub fn instance_id(&self) -> &CoreInstanceId {
        &self.instance_id
    }

    /// The launch credentials for local IPC authentication.
    #[must_use]
    pub fn credentials(&self) -> &LaunchCredentials {
        &self.credentials
    }

    /// Accesses the underlying supervisor coordinator reference.
    #[must_use]
    pub fn supervisor(&self) -> &AgentRuntimeSupervisor<H, C> {
        &self.supervisor
    }

    /// Accesses the underlying supervisor coordinator mutably.
    pub fn supervisor_mut(&mut self) -> &mut AgentRuntimeSupervisor<H, C> {
        &mut self.supervisor
    }

    /// Accesses the event pump.
    #[must_use]
    pub fn event_pump(&self) -> &EventPump {
        &self.event_pump
    }

    /// Accesses the shared IPC event log.
    #[must_use]
    pub fn event_log(&self) -> &Arc<Mutex<EventLog>> {
        self.event_pump.log()
    }

    /// Accesses the operation dedup registry.
    #[must_use]
    pub fn operation_registry(&self) -> &OperationRegistry {
        &self.operations
    }

    /// Mutably accesses the operation dedup registry.
    pub fn operation_registry_mut(&mut self) -> &mut OperationRegistry {
        &mut self.operations
    }

    /// Creates an IPC server port for accepting client connections.
    #[must_use]
    pub fn server_port(&self) -> CoreServerPort {
        CoreServerPort::new(
            self.credentials.clone(),
            self.supported_versions,
            self.core_version,
            self.capabilities.clone(),
            Arc::clone(self.event_pump.log()),
        )
    }

    /// Probes and tests an agent harness binding without starting a full session.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if probe fails.
    pub fn test_agent_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, CoreAppError> {
        self.supervisor
            .configure_and_test_binding(binding)
            .map_err(CoreAppError::from)
    }
}

impl<H> CoreApplication<H, StoreCheckpointAdapter>
where
    H: HarnessRuntimePort,
{
    /// Creates a `CoreApplication` with a concrete SQLite [`Store`].
    pub fn with_store(harness: H, store: Store, credentials: LaunchCredentials) -> Self {
        let adapter = StoreCheckpointAdapter::new(store);
        Self::new(harness, adapter, credentials)
    }

    /// Creates a `CoreApplication` with an in-memory SQLite store for tests.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if SQLite memory store fails to open.
    pub fn open_in_memory(
        harness: H,
        credentials: LaunchCredentials,
    ) -> Result<Self, CoreAppError> {
        let store = Store::open_in_memory()?;
        Ok(Self::with_store(harness, store, credentials))
    }

    /// Recovers any unsettled intent checkpoints on restart, scans indeterminate
    /// checkpoints and active turns, and enforces boundary non-retransmission invariants.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn on_startup(&mut self) -> Result<StartupRecoveryReport, CoreAppError> {
        // 1. Recover unsettled checkpoints: SQLite atomically transitions 'intent' -> 'indeterminate'
        let recovered = self
            .supervisor
            .checkpoint_mut()
            .store_mut()
            .recover_unsettled_checkpoints()?;

        // 2. Scan all Indeterminate checkpoints across store
        let chk_limit = CheckpointListLimit::try_new(CHECKPOINT_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let all_checkpoints = self
            .supervisor
            .checkpoint()
            .store()
            .runtime_checkpoints(None, None, chk_limit)?;
        let indeterminate_count = all_checkpoints
            .iter()
            .filter(|cp| cp.state == CheckpointState::Indeterminate)
            .count();

        // 3. Scan interrupted active turns
        let thread_limit = ThreadListLimit::try_new(THREAD_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let all_threads =
            self.supervisor
                .checkpoint()
                .store()
                .thread_list(None, None, thread_limit)?;
        let turn_limit = TurnListLimit::try_new(TURN_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let mut active_turns_count = 0;
        for t in &all_threads {
            if let Ok(tid) = ThreadId::from_str(&t.thread_id) {
                let turns = self
                    .supervisor
                    .checkpoint()
                    .store()
                    .turns_for_thread(&tid, None, turn_limit)?;
                active_turns_count += turns.iter().filter(|turn| turn.state == "active").count();
            }
        }

        Ok(StartupRecoveryReport {
            recovered_unsettled_intents: recovered,
            indeterminate_checkpoints_count: indeterminate_count,
            active_turns_count,
            resend_prohibited: true,
        })
    }

    // ── Agent Profile CRUD ─────────────────────────────────────────

    /// Creates an agent profile.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on validation or collision.
    pub fn create_agent_profile(&mut self, profile: &AgentProfile) -> Result<(), CoreAppError> {
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .create_agent_profile(profile)
            .map_err(CoreAppError::from)
    }

    /// Updates an existing agent profile.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn update_agent_profile(&mut self, profile: &AgentProfile) -> Result<(), CoreAppError> {
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .update_agent_profile(profile)
            .map_err(CoreAppError::from)
    }

    /// Fetches an agent profile by ID.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_agent_profile(
        &self,
        id: &AgentProfileId,
    ) -> Result<Option<AgentProfile>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .agent_profile_by_id(id)
            .map_err(CoreAppError::from)
    }

    /// Lists agent profiles with cursor-based pagination.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn list_agent_profiles(
        &self,
        before: Option<&AgentProfileCursor>,
        limit: AgentProfileListLimit,
    ) -> Result<Vec<AgentProfile>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .agent_profiles(before, limit)
            .map_err(CoreAppError::from)
    }

    /// Configures or updates an agent profile and its harness binding.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn configure_agent(
        &mut self,
        profile: &AgentProfile,
        binding: Option<&AcpHarnessBinding>,
    ) -> Result<(), CoreAppError> {
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .upsert_agent_profile(profile)?;
        if let Some(binding) = binding {
            self.supervisor
                .checkpoint_mut()
                .store_mut()
                .upsert_harness_binding(binding)?;
        }
        Ok(())
    }

    // ── Harness Binding CRUD ───────────────────────────────────────

    /// Creates an ACP harness binding.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn create_harness_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<(), CoreAppError> {
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .create_harness_binding(binding)
            .map_err(CoreAppError::from)
    }

    /// Fetches a harness binding by ID.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn get_harness_binding(
        &self,
        id: &HarnessBindingId,
    ) -> Result<Option<AcpHarnessBinding>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .harness_binding_by_id(id)
            .map_err(CoreAppError::from)
    }

    /// Lists harness bindings for an agent with cursor pagination.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn list_harness_bindings_for_agent(
        &self,
        agent_id: &AgentProfileId,
        before: Option<&HarnessBindingCursor>,
        limit: HarnessBindingListLimit,
    ) -> Result<Vec<AcpHarnessBinding>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .harness_bindings_for_agent(agent_id, before, limit)
            .map_err(CoreAppError::from)
    }

    // ── Thread CRUD & Projections ──────────────────────────────────

    /// Creates a new thread by appending a `ThreadCreated` event to the domain journal.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage or projection failure.
    pub fn create_thread(
        &mut self,
        thread_id: ThreadId,
        agent_profile_id: &AgentProfileId,
        title: Option<&ThreadTitle>,
        project_id: Option<&ProjectId>,
        now: UnixMillis,
    ) -> Result<ThreadRow, CoreAppError> {
        let payload = serde_json::json!({
            "agent_profile_id": agent_profile_id.as_str(),
            "title": title.map(ThreadTitle::as_str).unwrap_or_default(),
            "project_id": project_id.map(ProjectId::as_str),
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| CoreAppError::Other(e.to_string()))?
            .try_into()?;

        let event_id = self.event_pump.next_event_id(now)?;
        let domain_event = DomainEvent {
            event_id,
            thread_id: Some(thread_id.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::ThreadCreated,
            payload: payload_bytes,
            occurred_at: now,
        };

        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&domain_event)?;

        self.supervisor
            .checkpoint()
            .store()
            .thread_by_id(&thread_id)?
            .ok_or(CoreAppError::ThreadNotFound(thread_id))
    }

    /// Lists threads with optional state filter and cursor pagination.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn list_threads(
        &self,
        state_filter: Option<ThreadState>,
        before: Option<&ThreadCursor>,
        limit: ThreadListLimit,
    ) -> Result<Vec<ThreadRow>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .thread_list(state_filter, before, limit)
            .map_err(CoreAppError::from)
    }

    /// Searches threads by title via FTS5.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn search_threads(
        &self,
        query: &SearchQuery,
        before: Option<&ThreadCursor>,
        limit: ThreadListLimit,
    ) -> Result<Vec<ThreadRow>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .search_threads(query, before, limit)
            .map_err(CoreAppError::from)
    }

    /// Returns a single thread row by ID.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_thread(&self, thread_id: &ThreadId) -> Result<Option<ThreadRow>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .thread_by_id(thread_id)
            .map_err(CoreAppError::from)
    }

    /// Opens a thread: fetches projections, turn history, and initializes or resumes runtime session.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if thread is not found or session establishment fails.
    pub fn open_thread(
        &mut self,
        thread_id: &ThreadId,
        binding: Option<&AcpHarnessBinding>,
    ) -> Result<ThreadOpenResult, CoreAppError> {
        let thread = self
            .supervisor
            .checkpoint()
            .store()
            .thread_by_id(thread_id)?
            .ok_or_else(|| CoreAppError::ThreadNotFound(thread_id.clone()))?;

        let turn_limit = TurnListLimit::try_new(TURN_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let turns = self
            .supervisor
            .checkpoint()
            .store()
            .turns_for_thread(thread_id, None, turn_limit)?;

        let session_binding = self
            .supervisor
            .checkpoint()
            .store()
            .get_session_binding(thread_id)?;

        let effective_binding = if let Some(b) = binding {
            Some(b.clone())
        } else if let Some(ref sb) = session_binding {
            self.supervisor
                .checkpoint()
                .store()
                .harness_binding_by_id(&sb.harness_binding_id)?
        } else {
            let limit1 = HarnessBindingListLimit::try_new(1)
                .map_err(|e| CoreAppError::Other(e.to_string()))?;
            let agent_profile_id =
                AgentProfileId::from_str(&thread.agent_profile_id).map_err(CoreAppError::Id)?;
            self.supervisor
                .checkpoint()
                .store()
                .harness_bindings_for_agent(&agent_profile_id, None, limit1)?
                .into_iter()
                .next()
        };

        let effective_binding = effective_binding.ok_or_else(|| {
            let agent_body = thread
                .agent_profile_id
                .as_str()
                .strip_prefix("agp_")
                .unwrap_or(thread.agent_profile_id.as_str());
            let fallback_hsb =
                HarnessBindingId::from_str(&format!("hsb_{agent_body}")).unwrap_or(hsb_fallback());
            CoreAppError::HarnessBindingNotFound(fallback_hsb)
        })?;

        let current_state = self
            .supervisor
            .supervisor(thread_id)
            .map_or(SupervisorState::Idle, |s| s.state().clone());

        if matches!(
            current_state,
            SupervisorState::Idle | SupervisorState::Closed
        ) {
            let resume_res = if let Some(ref sb) = session_binding {
                if let Ok(sess_id) = HarnessSessionId::new(sb.opaque_session_id.as_str()) {
                    self.supervisor
                        .resume_session(&effective_binding, thread_id, &sess_id)
                } else {
                    Err(RuntimeError::InvalidSessionId("opaque session id".into()))
                }
            } else {
                Err(RuntimeError::SessionNotReady {
                    state: "no binding".into(),
                })
            };

            if resume_res.is_err() {
                let _ = self
                    .supervisor
                    .create_session(&effective_binding, thread_id, None);
            }
        }

        let (supervisor_state, session_id) = self
            .supervisor
            .supervisor(thread_id)
            .map_or((SupervisorState::Idle, None), |s| {
                (s.state().clone(), s.session_id().cloned())
            });

        Ok(ThreadOpenResult {
            thread,
            turns,
            supervisor_state,
            session_id,
        })
    }

    /// Returns event history for a thread from the domain journal.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_thread_history(
        &self,
        thread_id: &ThreadId,
        after_seq: i64,
        limit: HistoryLimit,
    ) -> Result<Vec<DomainJournalRow>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .thread_history(thread_id, after_seq, limit)
            .map_err(CoreAppError::from)
    }

    /// Returns turns for a thread.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_thread_turns(
        &self,
        thread_id: &ThreadId,
        after: Option<&TurnCursor>,
        limit: TurnListLimit,
    ) -> Result<Vec<TurnRow>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .turns_for_thread(thread_id, after, limit)
            .map_err(CoreAppError::from)
    }

    /// Returns permissions requested on a thread.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_thread_permissions(
        &self,
        thread_id: &ThreadId,
        after: Option<&PermissionCursor>,
        limit: PermissionListLimit,
    ) -> Result<Vec<Permission>, CoreAppError> {
        self.supervisor
            .checkpoint()
            .store()
            .permissions_for_thread(thread_id, after, limit)
            .map_err(CoreAppError::from)
    }

    /// Updates thread title in the domain journal.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn set_thread_title(
        &mut self,
        thread_id: &ThreadId,
        title: &ThreadTitle,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let payload = serde_json::json!({
            "title": title.as_str(),
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| CoreAppError::Other(e.to_string()))?
            .try_into()?;
        let event_id = self.event_pump.next_event_id(now)?;
        let domain_event = DomainEvent {
            event_id,
            thread_id: Some(thread_id.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::ThreadTitleChanged,
            payload: payload_bytes,
            occurred_at: now,
        };
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&domain_event)?;
        Ok(())
    }

    /// Updates thread lifecycle state (e.g. Pin, Archive, Open).
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn set_thread_state(
        &mut self,
        thread_id: &ThreadId,
        state: ThreadState,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let state_str = match state {
            ThreadState::Open => "open",
            ThreadState::Pinned => "pinned",
            ThreadState::Archived => "archived",
        };
        let payload = serde_json::json!({
            "state": state_str,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| CoreAppError::Other(e.to_string()))?
            .try_into()?;
        let event_id = self.event_pump.next_event_id(now)?;
        let domain_event = DomainEvent {
            event_id,
            thread_id: Some(thread_id.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::ThreadStateChanged,
            payload: payload_bytes,
            occurred_at: now,
        };
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&domain_event)?;
        Ok(())
    }

    // ── Turn & Execution Orchestration ─────────────────────────────

    /// Initiates a prompt turn on a thread.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on state mismatch, resend violation, or harness error.
    pub fn start_prompt(
        &mut self,
        operation_id: OperationId,
        thread_id: ThreadId,
        turn_id: TurnId,
        content: &str,
        now: UnixMillis,
    ) -> Result<TurnAdmission, CoreAppError> {
        let (admission, _) =
            self.start_prompt_envelope(operation_id, thread_id, turn_id, content, now)?;
        Ok(admission)
    }

    fn persist_turn_started(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        operation_id: &OperationId,
        content: &str,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let payload = serde_json::json!({
            "thread_id": thread_id.as_str(),
            "turn_id": turn_id.as_str(),
            "content": content,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| CoreAppError::Other(e.to_string()))?
            .try_into()?;

        let event_id = self.event_pump.next_event_id(now)?;
        let domain_event = DomainEvent {
            event_id,
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id.clone()),
            operation_id: Some(operation_id.clone()),
            kind: DomainEventKind::TurnStarted,
            payload: payload_bytes,
            occurred_at: now,
        };
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&domain_event)?;
        Ok(())
    }

    fn persist_turn_failed(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        operation_id: &OperationId,
        prompt_err: &RuntimeError,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        let delivery = match prompt_err {
            RuntimeError::Harness(_) => DeliveryState::Indeterminate,
            RuntimeError::UnsupportedCapability(_)
            | RuntimeError::AutomaticResendForbidden { .. } => DeliveryState::Rejected,
            _ => DeliveryState::Absent,
        };
        let delivery_str = match delivery {
            DeliveryState::Absent => "absent",
            DeliveryState::Confirmed => "confirmed",
            DeliveryState::Rejected => "rejected",
            DeliveryState::Indeterminate => "indeterminate",
        };
        let fail_payload = serde_json::json!({
            "thread_id": thread_id.as_str(),
            "turn_id": turn_id.as_str(),
            "reason": prompt_err.to_string(),
            "delivery": delivery_str,
        });
        let fail_payload_bytes = serde_json::to_vec(&fail_payload)
            .map_err(|error| CoreAppError::Other(error.to_string()))?
            .try_into()?;
        let fail_domain_event = DomainEvent {
            event_id: self.event_pump.next_event_id(now)?,
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id.clone()),
            operation_id: Some(operation_id.clone()),
            kind: DomainEventKind::TurnFailed,
            payload: fail_payload_bytes,
            occurred_at: now,
        };
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&fail_domain_event)?;
        Ok(())
    }

    /// Initiates a prompt turn on a thread and returns the emitted [`EventEnvelope`] if admitted.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on state mismatch, resend violation, or harness error.
    #[allow(clippy::too_many_lines)]
    pub fn start_prompt_envelope(
        &mut self,
        operation_id: OperationId,
        thread_id: ThreadId,
        turn_id: TurnId,
        content: &str,
        now: UnixMillis,
    ) -> Result<(TurnAdmission, Option<EventEnvelope>), CoreAppError> {
        let _ = self
            .supervisor
            .checkpoint()
            .store()
            .thread_by_id(&thread_id)?
            .ok_or_else(|| CoreAppError::ThreadNotFound(thread_id.clone()))?;

        // Query persistent turns across restarts
        let turn_limit = TurnListLimit::try_new(TURN_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let existing_turns = self
            .supervisor
            .checkpoint()
            .store()
            .turns_for_thread(&thread_id, None, turn_limit)?;

        for turn_row in existing_turns {
            if turn_row.turn_id == turn_id.as_str() {
                let delivery = match turn_row.delivery.as_str() {
                    "confirmed" => DeliveryState::Confirmed,
                    "indeterminate" => DeliveryState::Indeterminate,
                    "rejected" => DeliveryState::Rejected,
                    _ => DeliveryState::Absent,
                };
                let is_terminal = matches!(
                    turn_row.state.as_str(),
                    "completed" | "cancelled" | "failed"
                );
                if is_terminal
                    || delivery == DeliveryState::Confirmed
                    || delivery == DeliveryState::Indeterminate
                {
                    return Err(CoreAppError::AutomaticResendForbidden {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        delivery,
                    });
                }
            }
        }

        // Query persistent checkpoints across restarts
        let chk_limit = CheckpointListLimit::try_new(CHECKPOINT_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let existing_checkpoints = self.supervisor.checkpoint().store().runtime_checkpoints(
            Some(&thread_id),
            None,
            chk_limit,
        )?;

        for cp in existing_checkpoints {
            if cp.turn_id.as_ref() == Some(&turn_id)
                && (cp.state == CheckpointState::Confirmed
                    || cp.state == CheckpointState::Indeterminate
                    || cp.state.is_terminal())
            {
                let delivery = match cp.state {
                    CheckpointState::Confirmed => DeliveryState::Confirmed,
                    CheckpointState::Indeterminate => DeliveryState::Indeterminate,
                    CheckpointState::Rejected => DeliveryState::Rejected,
                    CheckpointState::Intent => DeliveryState::Absent,
                };
                return Err(CoreAppError::AutomaticResendForbidden {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    delivery,
                });
            }
        }

        let admission = self
            .supervisor
            .preflight_prompt(&thread_id, &operation_id, &turn_id)?;

        if admission == TurnAdmission::Duplicate {
            return Ok((TurnAdmission::Duplicate, None));
        }

        // Persist TurnStarted domain event BEFORE calling supervisor.prompt (external boundary)
        // so that the turn exists in domain storage for intent checkpoint FK constraints.
        self.persist_turn_started(&thread_id, &turn_id, &operation_id, content, now)?;

        let prompt_result =
            self.supervisor
                .prompt(&thread_id, operation_id.clone(), turn_id.clone(), content);

        match prompt_result {
            Ok(admission) => {
                let opt_envelope = if admission == TurnAdmission::Admitted {
                    let raw_event = RuntimeEvent::TurnStarted {
                        thread_id,
                        turn_id: turn_id.clone(),
                    };
                    let env = self.event_pump.publish_runtime_event(
                        &raw_event,
                        Some(operation_id),
                        Some(turn_id),
                        now,
                        None,
                    )?;
                    Some(env)
                } else {
                    None
                };
                Ok((admission, opt_envelope))
            }
            Err(prompt_err) => {
                // If prompt failed before or during external call, append TurnFailed
                // so that the turn does not remain perpetually Active in domain storage.
                match self.persist_turn_failed(
                    &thread_id,
                    &turn_id,
                    &operation_id,
                    &prompt_err,
                    now,
                ) {
                    Ok(()) => Err(CoreAppError::from(prompt_err)),
                    Err(persist_error) => Err(CoreAppError::Other(format!(
                        "prompt failed: {prompt_err}; failed to persist terminal turn state: {persist_error}"
                    ))),
                }
            }
        }
    }

    /// Cancels an active turn on `thread_id`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on capability or harness error.
    pub fn cancel_turn(
        &mut self,
        operation_id: &OperationId,
        thread_id: &ThreadId,
        now: UnixMillis,
    ) -> Result<CancelOutcome, CoreAppError> {
        let active_turn = self.supervisor.active_turn_id(thread_id).cloned();

        let outcome = self
            .supervisor
            .steer_cancel(thread_id, Some(operation_id))?;

        if outcome == CancelOutcome::CancelledActive {
            let turn_id = active_turn.ok_or_else(|| {
                CoreAppError::Other("missing active turn during cancel".to_string())
            })?;
            let event_id = self.event_pump.next_event_id(now)?;
            let payload = serde_json::json!({
                "thread_id": thread_id.as_str(),
                "turn_id": turn_id.as_str(),
                "operation_id": operation_id.as_str(),
            });
            let payload_bytes = serde_json::to_vec(&payload)
                .map_err(|e| CoreAppError::Other(e.to_string()))?
                .try_into()?;
            let domain_event = DomainEvent {
                event_id,
                thread_id: Some(thread_id.clone()),
                turn_id: Some(turn_id),
                operation_id: Some(operation_id.clone()),
                kind: DomainEventKind::TurnCancelled,
                payload: payload_bytes,
                occurred_at: now,
            };
            self.supervisor
                .checkpoint_mut()
                .store_mut()
                .append_domain_event(&domain_event)?;
        }

        Ok(outcome)
    }

    /// Submits a permission decision on a thread awaiting approval.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on permission mismatch or harness failure.
    pub fn decide_permission(
        &mut self,
        operation_id: &OperationId,
        thread_id: &ThreadId,
        permission_id: &EventId,
        decision: PermissionDecision,
        now: UnixMillis,
    ) -> Result<(), CoreAppError> {
        if matches!(decision, PermissionDecision::Pending) {
            return Err(CoreAppError::InvalidInput(
                "pending is not a valid permission decision".to_string(),
            ));
        }

        let turn_id = self
            .supervisor
            .active_permission_turn_id(thread_id, permission_id)
            .cloned()
            .ok_or_else(|| CoreAppError::PermissionNotFound {
                thread_id: thread_id.clone(),
                permission_id: permission_id.clone(),
            })?;

        self.supervisor
            .decide_permission(thread_id, permission_id, decision)?;

        let decision_str = match decision {
            PermissionDecision::Approved => "approved",
            PermissionDecision::Denied => "denied",
            PermissionDecision::Pending => unreachable!(),
        };

        let payload = serde_json::json!({
            "thread_id": thread_id.as_str(),
            "turn_id": turn_id.as_str(),
            "permission_event_id": permission_id.as_str(),
            "decision": decision_str,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| CoreAppError::Other(e.to_string()))?
            .try_into()?;
        let event_id = self.event_pump.next_event_id(now)?;
        let domain_event = DomainEvent {
            event_id,
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id),
            operation_id: Some(operation_id.clone()),
            kind: DomainEventKind::PermissionDecided,
            payload: payload_bytes,
            occurred_at: now,
        };
        self.supervisor
            .checkpoint_mut()
            .store_mut()
            .append_domain_event(&domain_event)?;

        Ok(())
    }

    /// Polls stream events for a single thread, pumping them into domain storage and the IPC event log.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on harness or storage failure.
    pub fn poll_thread_events(
        &mut self,
        thread_id: &ThreadId,
        now: UnixMillis,
    ) -> Result<Option<EventEnvelope>, CoreAppError> {
        let opt_event = self.supervisor.poll_stream(thread_id)?;
        if let Some(event) = opt_event {
            let envelope = self.event_pump.publish_runtime_event(
                &event,
                None,
                None,
                now,
                Some(self.supervisor.checkpoint_mut().store_mut()),
            )?;
            Ok(Some(envelope))
        } else {
            Ok(None)
        }
    }

    /// Polls and pumps all registered thread supervisors.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on failure.
    pub fn pump_all_threads(
        &mut self,
        now: UnixMillis,
    ) -> Result<Vec<EventEnvelope>, CoreAppError> {
        let thread_ids = self.supervisor.thread_ids();
        let mut envelopes = Vec::new();
        for thread_id in thread_ids {
            while let Some(env) = self.poll_thread_events(&thread_id, now)? {
                envelopes.push(env);
            }
        }
        Ok(envelopes)
    }

    // ── Status & Diagnostics ───────────────────────────────────────

    /// Returns a summary report of the application, active threads, and storage status.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage or lock acquisition failure.
    pub fn get_status(&self) -> Result<CoreStatusReport, CoreAppError> {
        let chk_limit = CheckpointListLimit::try_new(CHECKPOINT_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let indeterminate_count = self
            .supervisor
            .checkpoint()
            .store()
            .runtime_checkpoints(None, None, chk_limit)
            .map_or(0, |list| {
                list.into_iter()
                    .filter(|cp| cp.state == CheckpointState::Indeterminate)
                    .count()
            });

        let retained = self
            .event_pump
            .log()
            .lock()
            .map_err(|_| CoreAppError::LockPoisoned("event_log"))?
            .retained();

        Ok(CoreStatusReport {
            instance_id: self.instance_id.clone(),
            core_version: self.core_version,
            protocol_versions: self.supported_versions,
            active_thread_count: self.supervisor.thread_count(),
            registered_operations: self.operations.len(),
            retained_event_window: retained,
            indeterminate_checkpoints: indeterminate_count,
        })
    }

    /// Returns a diagnostics report of thread supervisor machines and checkpoints.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage failure.
    pub fn get_diagnostics(
        &self,
        thread_id: Option<&ThreadId>,
    ) -> Result<CoreDiagnosticsReport, CoreAppError> {
        let thread_states = self.supervisor.thread_states();
        let chk_limit = CheckpointListLimit::try_new(CHECKPOINT_LIST_LIMIT_MAX)
            .map_err(|e| CoreAppError::Other(e.to_string()))?;
        let all_checkpoints = self
            .supervisor
            .checkpoint()
            .store()
            .runtime_checkpoints(thread_id, None, chk_limit)?;

        let indeterminate: Vec<altior_domain::RuntimeCheckpoint> = all_checkpoints
            .iter()
            .filter(|cp| cp.state == CheckpointState::Indeterminate)
            .cloned()
            .collect();

        let active_count = all_checkpoints
            .iter()
            .filter(|cp| !cp.state.is_terminal())
            .count();

        Ok(CoreDiagnosticsReport {
            thread_states,
            active_checkpoints: active_count,
            indeterminate_checkpoints: indeterminate,
        })
    }
}
