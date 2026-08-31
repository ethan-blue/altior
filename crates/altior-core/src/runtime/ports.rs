//! Core-owned runtime ports and traits (P1.2).
//!
//! Infrastructure adapters (ACP adapter in Lane A, Storage journal/projections in Lane B)
//! will implement [`HarnessRuntimePort`] and [`RuntimeCheckpointPort`].
//! Use-case callers drive the runtime via [`AgentRuntime`].

use altior_domain::{
    AcpHarnessBinding, DomainEvent, EventId, PermissionDecision, ProjectRef, ThreadId, TurnId,
};

use super::state::{
    BindingProbeOutcome, CancelOutcome, CheckpointIntent, CheckpointSettled, HarnessError,
    HarnessEvent, HarnessPromptRequest, HarnessSessionId, HarnessSessionInfo, RuntimeError,
    RuntimeEvent, TurnAdmission,
};

/// Core-owned port for external agent execution harnesses (e.g. ACP subprocess, Terminal).
pub trait HarnessRuntimePort {
    /// Tests/probes a harness binding without launching a full long-lived session.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if probe fails.
    fn probe_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, HarnessError>;

    /// Spawns or connects a fresh harness session for `thread_id`.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if session spawn or initialization fails.
    fn create_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        project: Option<&ProjectRef>,
    ) -> Result<HarnessSessionInfo, HarnessError>;

    /// Resumes an existing harness session.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if resume fails.
    fn resume_session(
        &mut self,
        binding: &AcpHarnessBinding,
        session_id: &HarnessSessionId,
        thread_id: &ThreadId,
    ) -> Result<HarnessSessionInfo, HarnessError>;

    /// Sends a prompt to the harness for execution.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] on transport or write failure.
    fn send_prompt(
        &mut self,
        session_id: &HarnessSessionId,
        prompt: HarnessPromptRequest,
    ) -> Result<(), HarnessError>;

    /// Requests cancellation of the currently running turn in `session_id`.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if cancellation notification fails.
    fn cancel_turn(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError>;

    /// Submits a user permission decision to the harness.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if permission response fails.
    fn decide_permission(
        &mut self,
        session_id: &HarnessSessionId,
        event_id: &EventId,
        decision: PermissionDecision,
    ) -> Result<(), HarnessError>;

    /// Polls the next event produced by the harness session.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] on communication or decoding errors.
    fn poll_event(
        &mut self,
        session_id: &HarnessSessionId,
    ) -> Result<Option<HarnessEvent>, HarnessError>;

    /// Closes the harness session and cleans up child processes/resources.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessError`] if teardown encounters an error.
    fn close_session(&mut self, session_id: &HarnessSessionId) -> Result<(), HarnessError>;
}

/// Core-owned port for durable checkpointing of intents, settlements, and domain events.
pub trait RuntimeCheckpointPort {
    /// Durably persists an intent BEFORE dispatching an external adapter call.
    ///
    /// # Errors
    ///
    /// Returns [`crate::runtime::CheckpointError`] on persistence failure.
    fn checkpoint_intent(
        &mut self,
        intent: &CheckpointIntent,
    ) -> Result<(), crate::runtime::CheckpointError>;

    /// Durably settles the outcome AFTER an external adapter call returns.
    ///
    /// # Errors
    ///
    /// Returns [`crate::runtime::CheckpointError`] on persistence failure.
    fn settle_checkpoint(
        &mut self,
        settled: &CheckpointSettled,
    ) -> Result<(), crate::runtime::CheckpointError>;

    /// Persists a domain event to the domain journal.
    ///
    /// # Errors
    ///
    /// Returns [`crate::runtime::CheckpointError`] on persistence failure.
    fn record_event(&mut self, event: &DomainEvent) -> Result<(), crate::runtime::CheckpointError>;

    /// Durably records or updates a thread-to-harness session binding.
    ///
    /// Default implementation is a no-op for mock implementations.
    ///
    /// # Errors
    ///
    /// Returns [`crate::runtime::CheckpointError`] on persistence failure.
    fn bind_session(
        &mut self,
        _thread_id: &ThreadId,
        _harness_binding_id: &altior_domain::HarnessBindingId,
        _session_id: &HarnessSessionId,
        _now: altior_domain::UnixMillis,
    ) -> Result<(), crate::runtime::CheckpointError> {
        Ok(())
    }
}

/// Primary use-case port for driving agent execution on threads.
pub trait AgentRuntime {
    /// Tests/probes a harness binding.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on failure.
    fn configure_and_test_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, RuntimeError>;

    /// Creates and binds a new harness session for a thread.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if session creation fails or session already exists.
    fn create_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        project: Option<&ProjectRef>,
    ) -> Result<HarnessSessionId, RuntimeError>;

    /// Resumes an existing harness session for a thread.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if resume fails or capability is unsupported.
    fn resume_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        session_id: &HarnessSessionId,
    ) -> Result<(), RuntimeError>;

    /// Initiates a new prompt turn on `thread_id`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if thread is busy, duplicate operation, or resend is forbidden.
    fn prompt(
        &mut self,
        thread_id: &ThreadId,
        operation_id: altior_domain::OperationId,
        turn_id: TurnId,
        content: &str,
    ) -> Result<TurnAdmission, RuntimeError>;

    /// Polls stream events for `thread_id` and drives state machine transitions.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] on harness or checkpoint errors.
    fn poll_stream(&mut self, thread_id: &ThreadId) -> Result<Option<RuntimeEvent>, RuntimeError>;

    /// Submits a permission decision on a turn awaiting approval.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if permission is not found or thread not in awaiting state.
    fn decide_permission(
        &mut self,
        thread_id: &ThreadId,
        permission_id: &EventId,
        decision: PermissionDecision,
    ) -> Result<(), RuntimeError>;

    /// Requests cancellation or steering of an active turn on `thread_id`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if capability unsupported or transport fails.
    fn steer_cancel(
        &mut self,
        thread_id: &ThreadId,
        operation_id: Option<&altior_domain::OperationId>,
    ) -> Result<CancelOutcome, RuntimeError>;

    /// Closes the session on `thread_id`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if close fails.
    fn close_session(&mut self, thread_id: &ThreadId) -> Result<(), RuntimeError>;
}
