//! SQLite storage adapter for runtime intent and settlement checkpoints (P1.2, ADR 0002, ADR 0013).
//!
//! Maps core runtime [`CheckpointIntent`], [`CheckpointSettled`], and [`DomainEvent`]
//! directly to [`altior_storage::Store`] methods without bypassing storage invariants.

use std::str::FromStr;

use altior_domain::{
    BoundaryKind, CheckpointState, DeliveryState, DomainEvent, EventId, HarnessBindingId,
    OpaqueSessionId, OperationId, PermissionDecision, RuntimeCheckpoint, RuntimeCheckpointId,
    SessionBinding, ThreadId, TurnState, UnixMillis,
};
use altior_storage::{AppendOutcome, StorageError, Store};

use crate::runtime::ports::RuntimeCheckpointPort;
use crate::runtime::state::{
    CheckpointError, CheckpointIntent, CheckpointSettled, HarnessSessionId,
};

/// Adapter wrapping [`altior_storage::Store`] and implementing [`RuntimeCheckpointPort`].
#[derive(Debug)]
pub struct StoreCheckpointAdapter {
    store: Store,
}

impl StoreCheckpointAdapter {
    /// Creates a new adapter over an existing store.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Accesses the underlying [`Store`].
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Mutably accesses the underlying [`Store`].
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Unwraps the underlying [`Store`].
    #[must_use]
    pub fn into_inner(self) -> Store {
        self.store
    }

    /// Durably records or updates a thread-to-harness session binding.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] on persistence failure or invalid session ID.
    pub fn bind_session(
        &mut self,
        thread_id: &ThreadId,
        harness_binding_id: &HarnessBindingId,
        session_id: &HarnessSessionId,
        now: UnixMillis,
    ) -> Result<(), CheckpointError> {
        let opaque = OpaqueSessionId::try_from(session_id.as_str())
            .map_err(|e| CheckpointError::Other(format!("invalid opaque session id: {e}")))?;
        let binding = SessionBinding {
            thread_id: thread_id.clone(),
            harness_binding_id: harness_binding_id.clone(),
            opaque_session_id: opaque,
            updated_at: now,
        };
        self.store
            .replace_session_binding(&binding)
            .map_err(|e| CheckpointError::Persistence(e.to_string()))
    }

    /// Fetches the current session binding for a thread, if any.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] on persistence failure.
    pub fn get_session_binding(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Option<SessionBinding>, CheckpointError> {
        self.store
            .get_session_binding(thread_id)
            .map_err(|e| CheckpointError::Persistence(e.to_string()))
    }

    /// Removes a session binding for a thread.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] on persistence failure.
    pub fn remove_session_binding(
        &mut self,
        thread_id: &ThreadId,
    ) -> Result<bool, CheckpointError> {
        self.store
            .remove_session_binding(thread_id)
            .map_err(|e| CheckpointError::Persistence(e.to_string()))
    }
}

impl RuntimeCheckpointPort for StoreCheckpointAdapter {
    fn checkpoint_intent(&mut self, intent: &CheckpointIntent) -> Result<(), CheckpointError> {
        let checkpoint = map_intent_to_checkpoint(intent);
        self.store
            .record_runtime_intent(&checkpoint)
            .map_err(|err| match err {
                StorageError::CheckpointCollision { detail, .. } => {
                    CheckpointError::Conflict(detail)
                }
                other => CheckpointError::Persistence(other.to_string()),
            })
    }

    #[allow(clippy::too_many_lines)]
    fn settle_checkpoint(&mut self, settled: &CheckpointSettled) -> Result<(), CheckpointError> {
        match settled {
            CheckpointSettled::PromptDelivered {
                thread_id,
                turn_id: _,
                delivery,
                timestamp: _,
            } => {
                // If in-flight indeterminate delivery, keep intent pending in SQLite
                // so that crash/recovery will turn it into indeterminate.
                if *delivery == DeliveryState::Confirmed {
                    let active = self
                        .store
                        .active_checkpoints(Some(thread_id))
                        .map_err(|e| CheckpointError::Persistence(e.to_string()))?;
                    for cp in active {
                        let _ = self.store.settle_runtime_checkpoint(
                            &cp.id,
                            CheckpointState::Confirmed,
                            None,
                            None,
                            settled_timestamp(settled),
                        );
                    }
                }
                Ok(())
            }
            CheckpointSettled::TurnTerminal {
                thread_id,
                turn_id,
                state,
                delivery,
                timestamp,
            } => {
                let target_state = match (*state, *delivery) {
                    (TurnState::Completed, DeliveryState::Confirmed) => CheckpointState::Confirmed,
                    (TurnState::Cancelled, DeliveryState::Confirmed) => CheckpointState::Rejected,
                    _ => CheckpointState::Indeterminate,
                };
                let active = self
                    .store
                    .active_checkpoints(Some(thread_id))
                    .map_err(|e| CheckpointError::Persistence(e.to_string()))?;
                for cp in active {
                    if cp.turn_id.as_ref() == Some(turn_id) || cp.turn_id.is_none() {
                        let _ = self.store.settle_runtime_checkpoint(
                            &cp.id,
                            target_state,
                            None,
                            None,
                            *timestamp,
                        );
                    }
                }
                Ok(())
            }
            CheckpointSettled::PermissionSettled {
                thread_id,
                turn_id: _,
                permission_id: _,
                decision,
                timestamp,
            } => {
                let target_state = match decision {
                    PermissionDecision::Approved => CheckpointState::Confirmed,
                    PermissionDecision::Denied => CheckpointState::Rejected,
                    PermissionDecision::Pending => {
                        return Err(CheckpointError::Conflict(
                            "a pending permission is not a settlement".to_owned(),
                        ));
                    }
                };
                let active = self
                    .store
                    .active_checkpoints(Some(thread_id))
                    .map_err(|e| CheckpointError::Persistence(e.to_string()))?;
                for cp in active {
                    if cp.boundary_kind == BoundaryKind::PermissionDecision {
                        let _ = self.store.settle_runtime_checkpoint(
                            &cp.id,
                            target_state,
                            None,
                            None,
                            *timestamp,
                        );
                    }
                }
                Ok(())
            }
            CheckpointSettled::SessionClosed {
                thread_id,
                timestamp,
            } => {
                let active = self
                    .store
                    .active_checkpoints(Some(thread_id))
                    .map_err(|e| CheckpointError::Persistence(e.to_string()))?;
                for cp in active {
                    let _ = self.store.settle_runtime_checkpoint(
                        &cp.id,
                        CheckpointState::Confirmed,
                        None,
                        None,
                        *timestamp,
                    );
                }
                Ok(())
            }
        }
    }

    fn record_event(&mut self, event: &DomainEvent) -> Result<(), CheckpointError> {
        match self.store.append_domain_event(event) {
            Ok(AppendOutcome::Appended { .. } | AppendOutcome::Duplicate { .. }) => Ok(()),
            Err(err) => Err(CheckpointError::Persistence(err.to_string())),
        }
    }

    fn bind_session(
        &mut self,
        thread_id: &ThreadId,
        harness_binding_id: &HarnessBindingId,
        session_id: &HarnessSessionId,
        now: UnixMillis,
    ) -> Result<(), CheckpointError> {
        StoreCheckpointAdapter::bind_session(self, thread_id, harness_binding_id, session_id, now)
    }
}

fn settled_timestamp(settled: &CheckpointSettled) -> UnixMillis {
    match settled {
        CheckpointSettled::PromptDelivered { timestamp, .. }
        | CheckpointSettled::TurnTerminal { timestamp, .. }
        | CheckpointSettled::PermissionSettled { timestamp, .. }
        | CheckpointSettled::SessionClosed { timestamp, .. } => *timestamp,
    }
}

fn map_intent_to_checkpoint(intent: &CheckpointIntent) -> RuntimeCheckpoint {
    match intent {
        CheckpointIntent::Prompt {
            thread_id,
            turn_id,
            operation_id,
            timestamp,
        } => {
            let id = checkpoint_id_from_op(operation_id, thread_id, "prompt", *timestamp);
            RuntimeCheckpoint {
                id,
                thread_id: thread_id.clone(),
                turn_id: Some(turn_id.clone()),
                operation_id: operation_id.clone(),
                boundary_kind: BoundaryKind::Prompt,
                state: CheckpointState::Intent,
                remote_request_id: None,
                diagnostic_summary: None,
                created_at: *timestamp,
                settled_at: None,
            }
        }
        CheckpointIntent::PermissionDecision {
            thread_id,
            turn_id,
            permission_id,
            decision: _,
            timestamp,
        } => {
            let id = checkpoint_id_from_event(permission_id, thread_id, "perm", *timestamp);
            let op = synthetic_op("perm", thread_id, *timestamp);
            RuntimeCheckpoint {
                id,
                thread_id: thread_id.clone(),
                turn_id: Some(turn_id.clone()),
                operation_id: op,
                boundary_kind: BoundaryKind::PermissionDecision,
                state: CheckpointState::Intent,
                remote_request_id: None,
                diagnostic_summary: None,
                created_at: *timestamp,
                settled_at: None,
            }
        }
        CheckpointIntent::Cancel {
            thread_id,
            turn_id,
            operation_id,
            timestamp,
        } => {
            let op = operation_id
                .clone()
                .unwrap_or_else(|| synthetic_op("cancel", thread_id, *timestamp));
            let id = checkpoint_id_from_op(&op, thread_id, "cancel", *timestamp);
            RuntimeCheckpoint {
                id,
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                operation_id: op,
                boundary_kind: BoundaryKind::Cancel,
                state: CheckpointState::Intent,
                remote_request_id: None,
                diagnostic_summary: None,
                created_at: *timestamp,
                settled_at: None,
            }
        }
        CheckpointIntent::Close {
            thread_id,
            timestamp,
        } => {
            let op = synthetic_op("close", thread_id, *timestamp);
            let id = checkpoint_id_from_op(&op, thread_id, "close", *timestamp);
            RuntimeCheckpoint {
                id,
                thread_id: thread_id.clone(),
                turn_id: None,
                operation_id: op,
                boundary_kind: BoundaryKind::Close,
                state: CheckpointState::Intent,
                remote_request_id: None,
                diagnostic_summary: None,
                created_at: *timestamp,
                settled_at: None,
            }
        }
    }
}

fn checkpoint_id_from_op(
    op: &OperationId,
    thread_id: &ThreadId,
    tag: &str,
    ts: UnixMillis,
) -> RuntimeCheckpointId {
    deterministic_checkpoint_id(tag, thread_id, op.as_str(), ts)
}

fn checkpoint_id_from_event(
    event_id: &EventId,
    thread_id: &ThreadId,
    tag: &str,
    ts: UnixMillis,
) -> RuntimeCheckpointId {
    deterministic_checkpoint_id(tag, thread_id, event_id.as_str(), ts)
}

fn synthetic_op(tag: &str, thread_id: &ThreadId, ts: UnixMillis) -> OperationId {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut hasher);
    thread_id.as_str().hash(&mut hasher);
    ts.as_millis().hash(&mut hasher);
    let h = hasher.finish();
    OperationId::from_str(&format!("op_{tag}{h:016x}"))
        .or_else(|_| OperationId::from_str(&format!("op_{h:016x}")))
        .expect("valid synthetic operation id")
}

fn deterministic_checkpoint_id(
    kind_tag: &str,
    thread_id: &ThreadId,
    scope: &str,
    ts: UnixMillis,
) -> RuntimeCheckpointId {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind_tag.hash(&mut hasher);
    thread_id.as_str().hash(&mut hasher);
    scope.hash(&mut hasher);
    ts.as_millis().hash(&mut hasher);
    let h1 = hasher.finish();
    hasher.write_u64(0x9e37_79b9_7f4a_7c15);
    let h2 = hasher.finish();
    let hex = format!("{h1:016x}{h2:016x}");
    RuntimeCheckpointId::from_str(&format!("chk_{hex}")).expect("valid deterministic checkpoint id")
}
