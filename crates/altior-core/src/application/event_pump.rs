//! Runtime event pump: maps runtime events to storage domain events and bounded IPC replay log (P1.3).
//!
//! Bridges [`RuntimeEvent`] emitted by thread supervisors to:
//! 1. Durable SQLite domain journal records (`DomainEvent`).
//! 2. Bounded in-memory IPC event envelopes in [`EventLog`] for live and catch-up subscriptions.
//!
//! UI client detachment resilience: if no UI client is attached or the UI disconnects,
//! the pump continues to sequence and buffer events in the replay window without
//! affecting supervisor execution.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use altior_domain::{
    DeliveryState, DomainEvent, DomainEventKind, EventId, EventPayload, OperationId,
    PermissionDescription, PermissionKind, TURN_LIST_LIMIT_MAX, ThreadId, TurnId, TurnListLimit,
    UnixMillis,
};
use altior_ipc::EventLog;
use altior_protocol::{
    DiagnosticText, EventBody, EventEnvelope, KnownEvent, MessageText, ProtocolVersion, Sequence,
};
use altior_storage::Store;

use crate::application::error::CoreAppError;
use crate::runtime::RuntimeEvent;
use crate::runtime::diagnostics::BoundedDiagnosticsSummary;

/// Generator for deterministic, unique event IDs.
#[derive(Debug, Default)]
pub struct EventIdGenerator {
    counter: AtomicU64,
}

impl EventIdGenerator {
    /// Creates a new generator starting at counter 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    /// Generates the next validated `EventId`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if event ID construction or validation fails.
    pub fn next_id(&self, now: UnixMillis) -> Result<EventId, CoreAppError> {
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        let millis = now.as_millis();
        let hex = format!("{millis:016x}{count:016x}");
        EventId::from_str(&format!("evt_{hex}")).map_err(CoreAppError::from)
    }
}

/// Pump sequencing runtime events into domain storage and the IPC replay buffer.
#[derive(Debug)]
pub struct EventPump {
    log: Arc<Mutex<EventLog>>,
    id_gen: Arc<EventIdGenerator>,
    protocol_version: ProtocolVersion,
}

impl EventPump {
    /// Creates an event pump over a shared event log.
    #[must_use]
    pub fn new(log: Arc<Mutex<EventLog>>, protocol_version: ProtocolVersion) -> Self {
        Self {
            log,
            id_gen: Arc::new(EventIdGenerator::new()),
            protocol_version,
        }
    }

    /// Accesses the underlying event log reference.
    #[must_use]
    pub fn log(&self) -> &Arc<Mutex<EventLog>> {
        &self.log
    }

    /// Generates a fresh unique event ID for caller boundaries.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] if event ID generation fails.
    pub fn next_event_id(&self, now: UnixMillis) -> Result<EventId, CoreAppError> {
        self.id_gen.next_id(now)
    }

    /// Processes one runtime event: appends to SQLite domain journal (if store provided)
    /// and sequences into the in-memory IPC event log.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on storage, lock, or sequence allocation failure.
    pub fn publish_runtime_event(
        &self,
        event: &RuntimeEvent,
        operation_id: Option<OperationId>,
        turn_id: Option<TurnId>,
        now: UnixMillis,
        store: Option<&mut Store>,
    ) -> Result<EventEnvelope, CoreAppError> {
        let event_id = match event {
            RuntimeEvent::PermissionRequested { permission_id, .. } => permission_id.clone(),
            _ => self.id_gen.next_id(now)?,
        };
        let thread_id = extract_thread_id(event);
        let actual_turn_id = turn_id.or_else(|| extract_turn_id(event));

        // 1. Map to DomainEvent and append to SQLite domain journal if storage is active
        if let Some(store) = store {
            let should_append = if let Some(ref trnid) = actual_turn_id {
                if matches!(
                    event,
                    RuntimeEvent::TurnCompleted { .. }
                        | RuntimeEvent::TurnCancelled { .. }
                        | RuntimeEvent::TurnFailed { .. }
                ) {
                    let turn_limit = TurnListLimit::try_new(TURN_LIST_LIMIT_MAX)
                        .map_err(|e| CoreAppError::Other(e.to_string()))?;
                    let turns = store.turns_for_thread(&thread_id, None, turn_limit)?;
                    turns
                        .iter()
                        .any(|t| t.turn_id == trnid.as_str() && t.state == "active")
                } else {
                    true
                }
            } else {
                true
            };

            if should_append
                && let Some(domain_event) = map_runtime_to_domain_event(
                    &event_id,
                    event,
                    Some(thread_id.clone()),
                    actual_turn_id.clone(),
                    operation_id.clone(),
                    now,
                )?
            {
                store.append_domain_event(&domain_event)?;
            }
        }

        // 2. Map to EventBody for IPC envelope
        let body = map_runtime_to_event_body(event)?;

        // 3. Create raw envelope and sequence through shared EventLog
        let raw_envelope = EventEnvelope {
            protocol_version: self.protocol_version,
            event_id,
            operation_id,
            thread_id: Some(thread_id),
            turn_id: actual_turn_id,
            sequence: Sequence::FIRST, // replaced by EventLog::append
            occurred_at: now,
            body,
        };

        let sequenced = self
            .log
            .lock()
            .map_err(|_| CoreAppError::LockPoisoned("event_log"))?
            .append(raw_envelope)?;
        Ok(sequenced)
    }
}

fn extract_thread_id(event: &RuntimeEvent) -> ThreadId {
    match event {
        RuntimeEvent::TurnStarted { thread_id, .. }
        | RuntimeEvent::MessageDelta { thread_id, .. }
        | RuntimeEvent::PermissionRequested { thread_id, .. }
        | RuntimeEvent::TurnCompleted { thread_id, .. }
        | RuntimeEvent::TurnFailed { thread_id, .. }
        | RuntimeEvent::TurnCancelled { thread_id, .. }
        | RuntimeEvent::ProcessExited { thread_id, .. }
        | RuntimeEvent::Unknown { thread_id, .. } => thread_id.clone(),
    }
}

fn extract_turn_id(event: &RuntimeEvent) -> Option<TurnId> {
    match event {
        RuntimeEvent::TurnStarted { turn_id, .. }
        | RuntimeEvent::MessageDelta { turn_id, .. }
        | RuntimeEvent::PermissionRequested { turn_id, .. }
        | RuntimeEvent::TurnCompleted { turn_id, .. }
        | RuntimeEvent::TurnFailed { turn_id, .. }
        | RuntimeEvent::TurnCancelled { turn_id, .. } => Some(turn_id.clone()),
        RuntimeEvent::ProcessExited { .. } | RuntimeEvent::Unknown { .. } => None,
    }
}

struct EventContext<'a> {
    event_id: &'a EventId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    operation_id: Option<OperationId>,
    occurred_at: UnixMillis,
}

impl EventContext<'_> {
    fn into_domain_event(self, kind: DomainEventKind, payload: EventPayload) -> DomainEvent {
        DomainEvent {
            event_id: self.event_id.clone(),
            thread_id: self.thread_id,
            turn_id: self.turn_id,
            operation_id: self.operation_id,
            kind,
            payload,
            occurred_at: self.occurred_at,
        }
    }
}

fn map_message_delta(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    trnid: &TurnId,
    text: &str,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "turn_id": trnid.as_str(),
        "text": text,
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(DomainEventKind::MessageDelta, payload_bytes))
}

fn map_permission_requested(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    trnid: &TurnId,
    permission_id: &EventId,
    kind: PermissionKind,
    description: &PermissionDescription,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "turn_id": trnid.as_str(),
        "permission_id": permission_id.as_str(),
        "permission_kind": kind.as_str(),
        "description": description.as_str(),
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(DomainEventKind::PermissionRequested, payload_bytes))
}

fn map_turn_completed(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    trnid: &TurnId,
    comp_payload: Option<&EventPayload>,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "turn_id": trnid.as_str(),
        "content": comp_payload.and_then(EventPayload::as_str).unwrap_or_default(),
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(DomainEventKind::TurnCompleted, payload_bytes))
}

fn map_turn_cancelled(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    trnid: &TurnId,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "turn_id": trnid.as_str(),
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(DomainEventKind::TurnCancelled, payload_bytes))
}

fn map_turn_failed(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    trnid: &TurnId,
    reason: &str,
    delivery: DeliveryState,
) -> Result<DomainEvent, CoreAppError> {
    let delivery_str = match delivery {
        DeliveryState::Absent => "absent",
        DeliveryState::Confirmed => "confirmed",
        DeliveryState::Rejected => "rejected",
        DeliveryState::Indeterminate => "indeterminate",
    };
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "turn_id": trnid.as_str(),
        "reason": reason,
        "delivery": delivery_str,
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(DomainEventKind::TurnFailed, payload_bytes))
}

fn map_process_exited(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    exit_code: Option<i32>,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "exit_code": exit_code,
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    Ok(ctx.into_domain_event(
        DomainEventKind::Other("process.exited".to_string()),
        payload_bytes,
    ))
}

fn map_unknown_event(
    ctx: EventContext<'_>,
    tid: &ThreadId,
    name: &str,
    summary: &BoundedDiagnosticsSummary,
) -> Result<DomainEvent, CoreAppError> {
    let payload = serde_json::json!({
        "thread_id": tid.as_str(),
        "name": name,
        "summary": summary.as_str(),
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| CoreAppError::Other(e.to_string()))?
        .try_into()?;
    let kind_name = if name.is_empty() {
        "unknown.event".to_string()
    } else {
        format!("unknown.{}", name.replace([':', '/', ' '], "."))
    };
    Ok(ctx.into_domain_event(DomainEventKind::Other(kind_name), payload_bytes))
}

fn map_runtime_to_domain_event(
    event_id: &EventId,
    event: &RuntimeEvent,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    operation_id: Option<OperationId>,
    now: UnixMillis,
) -> Result<Option<DomainEvent>, CoreAppError> {
    let ctx = EventContext {
        event_id,
        thread_id,
        turn_id,
        operation_id,
        occurred_at: now,
    };
    let domain_event = match event {
        // TurnStarted domain persistence is owned by start_prompt authority.
        // Event pump treats Harness Started as protocol-only to avoid duplicate domain folds.
        RuntimeEvent::TurnStarted { .. } => None,
        RuntimeEvent::MessageDelta {
            thread_id: tid,
            turn_id: trnid,
            text,
        } => Some(map_message_delta(ctx, tid, trnid, text)?),
        RuntimeEvent::PermissionRequested {
            thread_id: tid,
            turn_id: trnid,
            permission_id,
            kind,
            description,
        } => Some(map_permission_requested(
            ctx,
            tid,
            trnid,
            permission_id,
            *kind,
            description,
        )?),
        RuntimeEvent::TurnCompleted {
            thread_id: tid,
            turn_id: trnid,
            payload: comp_payload,
        } => Some(map_turn_completed(ctx, tid, trnid, comp_payload.as_ref())?),
        RuntimeEvent::TurnCancelled {
            thread_id: tid,
            turn_id: trnid,
        } => Some(map_turn_cancelled(ctx, tid, trnid)?),
        RuntimeEvent::TurnFailed {
            thread_id: tid,
            turn_id: trnid,
            reason,
            delivery,
        } => Some(map_turn_failed(ctx, tid, trnid, reason, *delivery)?),
        RuntimeEvent::ProcessExited {
            thread_id: tid,
            exit_code,
        } => Some(map_process_exited(ctx, tid, *exit_code)?),
        RuntimeEvent::Unknown {
            thread_id: tid,
            name,
            summary,
        } => Some(map_unknown_event(ctx, tid, name, summary)?),
    };
    Ok(domain_event)
}

fn map_runtime_to_event_body(event: &RuntimeEvent) -> Result<EventBody, CoreAppError> {
    match event {
        RuntimeEvent::TurnStarted { .. } => Ok(EventBody::Known(KnownEvent::TurnStarted)),
        RuntimeEvent::MessageDelta { text, .. } => {
            let message_text = MessageText::try_from(text.as_str())?;
            Ok(EventBody::Known(KnownEvent::MessageDelta {
                text: message_text,
            }))
        }
        RuntimeEvent::TurnCompleted { .. } => Ok(EventBody::Known(KnownEvent::TurnCompleted)),
        RuntimeEvent::PermissionRequested {
            permission_id,
            kind,
            description,
            ..
        } => {
            let kind_str = kind.as_str();
            let desc_str = description.as_str();
            let diagnostic_str = format!(
                r#"{{"permission_id":"{permission_id}","kind":"{kind_str}","description":"{desc_str}"}}"#
            );
            let diagnostic = DiagnosticText::try_from(diagnostic_str.as_str())?;
            Ok(EventBody::Unknown {
                provider_kind: "permission.requested".to_string(),
                diagnostic,
            })
        }
        RuntimeEvent::TurnCancelled { .. } => {
            let diagnostic = DiagnosticText::try_from(r#"{"status":"cancelled"}"#)?;
            Ok(EventBody::Unknown {
                provider_kind: "turn.cancelled".to_string(),
                diagnostic,
            })
        }
        RuntimeEvent::TurnFailed {
            reason, delivery, ..
        } => {
            let diagnostic_str = format!(r#"{{"reason":"{reason}","delivery":"{delivery:?}"}}"#);
            let diagnostic = DiagnosticText::try_from(diagnostic_str.as_str())?;
            Ok(EventBody::Unknown {
                provider_kind: "turn.failed".to_string(),
                diagnostic,
            })
        }
        RuntimeEvent::ProcessExited { exit_code, .. } => {
            let diagnostic_str = format!(r#"{{"exit_code":{exit_code:?}}}"#);
            let diagnostic = DiagnosticText::try_from(diagnostic_str.as_str())?;
            Ok(EventBody::Unknown {
                provider_kind: "process.exited".to_string(),
                diagnostic,
            })
        }
        RuntimeEvent::Unknown { name, summary, .. } => {
            let diagnostic = DiagnosticText::try_from(summary.as_str())?;
            Ok(EventBody::Unknown {
                provider_kind: name.clone(),
                diagnostic,
            })
        }
    }
}
