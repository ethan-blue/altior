//! Agent runtime use-case coordinator (P1.2).
//!
//! Composes injectable [`HarnessRuntimePort`] and [`RuntimeCheckpointPort`]
//! into the [`AgentRuntime`] use-case layer.

use std::collections::BTreeMap;
use std::str::FromStr;

use altior_domain::{
    AcpHarnessBinding, EventId, OperationId, PermissionDecision, ProjectRef, ThreadId, TurnId,
    UnixMillis,
};
use altior_protocol::{CapabilityId, CapabilitySupport};

use super::ports::{AgentRuntime, HarnessRuntimePort, RuntimeCheckpointPort};
use super::state::{
    BindingProbeOutcome, CancelOutcome, HarnessSessionId, RuntimeError, RuntimeEvent,
    SupervisorState, TurnAdmission,
};
use super::supervisor::ThreadRuntimeSupervisor;

/// Coordinator managing thread supervisors and external harness/checkpoint ports.
#[derive(Debug)]
pub struct AgentRuntimeSupervisor<H, C> {
    harness: H,
    checkpoint: C,
    threads: BTreeMap<ThreadId, ThreadRuntimeSupervisor>,
}

impl<H, C> AgentRuntimeSupervisor<H, C>
where
    H: HarnessRuntimePort,
    C: RuntimeCheckpointPort,
{
    /// Creates a new coordinator over injectable harness and checkpoint ports.
    pub fn new(harness: H, checkpoint: C) -> Self {
        Self {
            harness,
            checkpoint,
            threads: BTreeMap::new(),
        }
    }

    /// Accesses the underlying harness port reference.
    pub fn harness(&self) -> &H {
        &self.harness
    }

    /// Accesses the underlying harness port mutably.
    pub fn harness_mut(&mut self) -> &mut H {
        &mut self.harness
    }

    /// Accesses the underlying checkpoint port reference.
    pub fn checkpoint(&self) -> &C {
        &self.checkpoint
    }

    /// Accesses the underlying checkpoint port mutably.
    pub fn checkpoint_mut(&mut self) -> &mut C {
        &mut self.checkpoint
    }

    /// Returns a thread supervisor if registered.
    #[must_use]
    pub fn supervisor(&self, thread_id: &ThreadId) -> Option<&ThreadRuntimeSupervisor> {
        self.threads.get(thread_id)
    }

    /// Returns a mutable thread supervisor if registered.
    pub fn supervisor_mut(&mut self, thread_id: &ThreadId) -> Option<&mut ThreadRuntimeSupervisor> {
        self.threads.get_mut(thread_id)
    }

    /// Number of tracked thread supervisors.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// Returns a snapshot of all thread supervisor states.
    #[must_use]
    pub fn thread_states(&self) -> BTreeMap<ThreadId, SupervisorState> {
        self.threads
            .iter()
            .map(|(id, s)| (id.clone(), s.state().clone()))
            .collect()
    }

    /// Returns all registered thread IDs.
    #[must_use]
    pub fn thread_ids(&self) -> Vec<ThreadId> {
        self.threads.keys().cloned().collect()
    }

    /// Preflights prompt readiness for a thread without mutating state.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if thread is unknown or session is not ready.
    pub fn preflight_prompt(
        &self,
        thread_id: &ThreadId,
        operation_id: &OperationId,
        turn_id: &TurnId,
    ) -> Result<TurnAdmission, RuntimeError> {
        let supervisor = self
            .threads
            .get(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;
        supervisor.preflight_prompt(operation_id, turn_id)
    }

    /// Returns active turn ID for a thread if currently executing.
    #[must_use]
    pub fn active_turn_id(&self, thread_id: &ThreadId) -> Option<&TurnId> {
        self.supervisor(thread_id)
            .and_then(ThreadRuntimeSupervisor::active_turn_id)
    }

    /// Returns active turn ID for a thread if currently awaiting the given permission.
    #[must_use]
    pub fn active_permission_turn_id(
        &self,
        thread_id: &ThreadId,
        permission_id: &EventId,
    ) -> Option<&TurnId> {
        self.supervisor(thread_id)
            .and_then(|s| s.active_permission_turn_id(permission_id))
    }

    /// Drains all active work and closes all sessions for application shutdown.
    pub fn drain_for_shutdown(&mut self, now: UnixMillis) {
        for supervisor in self.threads.values_mut() {
            let _ = supervisor.close_session(now, &mut self.harness, &mut self.checkpoint);
        }
    }
}

impl<H, C> AgentRuntime for AgentRuntimeSupervisor<H, C>
where
    H: HarnessRuntimePort,
    C: RuntimeCheckpointPort,
{
    fn configure_and_test_binding(
        &mut self,
        binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, RuntimeError> {
        self.harness
            .probe_binding(binding)
            .map_err(RuntimeError::from)
    }

    fn create_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        project: Option<&ProjectRef>,
    ) -> Result<HarnessSessionId, RuntimeError> {
        let supervisor = self
            .threads
            .entry(thread_id.clone())
            .or_insert_with(|| ThreadRuntimeSupervisor::new(thread_id.clone()));

        if supervisor.session_id().is_some() {
            return Err(RuntimeError::SessionAlreadyActive(thread_id.clone()));
        }

        supervisor.mark_starting();
        let session_info = match self.harness.create_session(binding, thread_id, project) {
            Ok(info) => info,
            Err(err) => {
                supervisor.reset_idle();
                return Err(RuntimeError::from(err));
            }
        };

        let session_id = session_info.session_id.clone();
        let now = UnixMillis::from_millis(0);
        let _ = self
            .checkpoint
            .bind_session(thread_id, &binding.id, &session_id, now);
        supervisor.on_session_established(session_info.session_id, session_info.capabilities);

        Ok(session_id)
    }

    fn resume_session(
        &mut self,
        binding: &AcpHarnessBinding,
        thread_id: &ThreadId,
        session_id: &HarnessSessionId,
    ) -> Result<(), RuntimeError> {
        let supervisor = self
            .threads
            .entry(thread_id.clone())
            .or_insert_with(|| ThreadRuntimeSupervisor::new(thread_id.clone()));

        if supervisor.session_id().is_some() {
            return Err(RuntimeError::SessionAlreadyActive(thread_id.clone()));
        }

        supervisor.mark_starting();
        let session_info = match self.harness.resume_session(binding, session_id, thread_id) {
            Ok(info) => info,
            Err(err) => {
                supervisor.reset_idle();
                return Err(RuntimeError::from(err));
            }
        };

        // Capability gate check before establishing session on supervisor
        if let Ok(cap_id) = CapabilityId::from_str("session.resume")
            && session_info.capabilities.get(&cap_id) == Some(CapabilitySupport::Unsupported)
        {
            supervisor.reset_idle();
            return Err(RuntimeError::UnsupportedCapability(cap_id));
        }

        let now = UnixMillis::from_millis(0);
        let _ = self
            .checkpoint
            .bind_session(thread_id, &binding.id, session_id, now);
        supervisor.on_session_established(session_info.session_id, session_info.capabilities);

        Ok(())
    }

    fn prompt(
        &mut self,
        thread_id: &ThreadId,
        operation_id: OperationId,
        turn_id: TurnId,
        content: &str,
    ) -> Result<TurnAdmission, RuntimeError> {
        let supervisor = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;

        let now = UnixMillis::from_millis(0);
        supervisor.prompt(
            operation_id,
            turn_id,
            content,
            now,
            &mut self.harness,
            &mut self.checkpoint,
        )
    }

    fn poll_stream(&mut self, thread_id: &ThreadId) -> Result<Option<RuntimeEvent>, RuntimeError> {
        let supervisor = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;

        let now = UnixMillis::from_millis(0);
        supervisor.poll_stream(now, &mut self.harness, &mut self.checkpoint)
    }

    fn decide_permission(
        &mut self,
        thread_id: &ThreadId,
        permission_id: &EventId,
        decision: PermissionDecision,
    ) -> Result<(), RuntimeError> {
        let supervisor = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;

        let now = UnixMillis::from_millis(0);
        supervisor.decide_permission(
            permission_id,
            decision,
            now,
            &mut self.harness,
            &mut self.checkpoint,
        )
    }

    fn steer_cancel(
        &mut self,
        thread_id: &ThreadId,
        operation_id: Option<&OperationId>,
    ) -> Result<CancelOutcome, RuntimeError> {
        let supervisor = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;

        let now = UnixMillis::from_millis(0);
        supervisor.steer_cancel(operation_id, now, &mut self.harness, &mut self.checkpoint)
    }

    fn close_session(&mut self, thread_id: &ThreadId) -> Result<(), RuntimeError> {
        let supervisor = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| RuntimeError::UnknownThread(thread_id.clone()))?;

        let now = UnixMillis::from_millis(0);
        supervisor.close_session(now, &mut self.harness, &mut self.checkpoint)
    }
}
