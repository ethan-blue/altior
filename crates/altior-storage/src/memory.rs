//! Durable memory repository and retrieval index (P2.1, ADR 0017).
//!
//! Memories are projected into the `memory` SQLite table and indexed
//! in `memory_fts` (a dedicated FTS5 table containing only retrievable memories).
//! All lifecycle mutations append authoritative `DomainEvent` records to the
//! `domain_journal` within atomic `IMMEDIATE` transactions.

use std::str::FromStr;

use rusqlite::params;

use altior_domain::{
    DomainEvent, DomainEventKind, EventId, EventPayload, MemoryContent, MemoryCursor, MemoryDraft,
    MemoryExcerpt, MemoryHit, MemoryId, MemoryKind, MemoryListLimit, MemoryMatchExplanation,
    MemoryProvenance, MemoryRecord, MemoryScope, MemorySearchLimit, MemorySensitivity,
    MemorySource, MemoryState, SearchQuery, ThreadId, TurnId, UnixMillis, is_secret_shaped,
};

use crate::error::StorageError;
use crate::{Store, collect_rows};

/// Generates a synthetic deterministic event ID based on timestamp and sequence disambiguator.
fn generate_event_id(occurred_at: UnixMillis, disambiguator: u32) -> EventId {
    let raw = format!("evt_{:016x}{:08x}", occurred_at.as_millis(), disambiguator);
    EventId::from_str(&raw).expect("valid synthetic event id")
}

/// Generates a synthetic deterministic memory ID based on timestamp and sequence disambiguator.
fn generate_memory_id(occurred_at: UnixMillis, disambiguator: u32) -> MemoryId {
    let raw = format!("mem_{:016x}{:08x}", occurred_at.as_millis(), disambiguator);
    MemoryId::from_str(&raw).expect("valid synthetic memory id")
}

/// Reads one SQLite column by index with a contextual error message.
fn col<T: rusqlite::types::FromSql>(
    row: &rusqlite::Row<'_>,
    index: usize,
    name: &'static str,
) -> Result<T, StorageError> {
    row.get(index)
        .map_err(|e| StorageError::from_sqlite(name, e))
}

/// Maps one SQLite `memory` row to a pure [`MemoryRecord`].
pub(crate) fn row_to_memory_record(row: &rusqlite::Row<'_>) -> Result<MemoryRecord, StorageError> {
    let invalid = |detail: String| StorageError::InvalidEntityData { detail };
    let id_raw: String = col(row, 0, "memory_id")?;
    let content_raw: String = col(row, 1, "content")?;
    let scope_kind: String = col(row, 2, "scope_kind")?;
    let scope_target: Option<String> = col(row, 3, "scope_target")?;
    let kind_raw: String = col(row, 4, "kind")?;
    let state_raw: String = col(row, 5, "state")?;
    let confidence_raw: u8 = col(row, 6, "confidence")?;
    let sensitivity_raw: String = col(row, 7, "sensitivity")?;
    let source_raw: String = col(row, 8, "source")?;
    let _explicit_raw: i64 = col(row, 9, "explicit")?;
    let prov_thread_raw: Option<String> = col(row, 10, "provenance_thread_id")?;
    let prov_turn_raw: Option<String> = col(row, 11, "provenance_turn_id")?;
    let excerpt_raw: Option<String> = col(row, 12, "excerpt")?;
    let created_at_raw: i64 = col(row, 13, "created_at")?;
    let updated_at_raw: i64 = col(row, 14, "updated_at")?;
    let expires_at_raw: Option<i64> = col(row, 15, "expires_at")?;
    let superseded_by_raw: Option<String> = col(row, 16, "superseded_by")?;

    let memory_id = id_raw
        .parse::<MemoryId>()
        .map_err(|e| invalid(format!("invalid memory_id {id_raw}: {e}")))?;
    let content = MemoryContent::try_from(content_raw.as_str())
        .map_err(|e| invalid(format!("invalid memory content: {e}")))?;
    let scope = MemoryScope::try_from_parts(&scope_kind, scope_target.as_deref())
        .map_err(|e| invalid(format!("invalid memory scope: {e}")))?;
    let kind = MemoryKind::try_from_str(&kind_raw)
        .map_err(|e| invalid(format!("invalid memory kind {kind_raw}: {e}")))?;
    let state = MemoryState::try_from_str(&state_raw)
        .map_err(|e| invalid(format!("invalid memory state {state_raw}: {e}")))?;
    let sensitivity = MemorySensitivity::try_from_str(&sensitivity_raw)
        .map_err(|e| invalid(format!("invalid memory sensitivity {sensitivity_raw}: {e}")))?;
    let source = MemorySource::try_from_str(&source_raw)
        .map_err(|e| invalid(format!("invalid memory source {source_raw}: {e}")))?;
    let thread_id = prov_thread_raw
        .map(|s| s.parse::<ThreadId>())
        .transpose()
        .map_err(|e| invalid(format!("invalid provenance thread_id: {e}")))?;
    let turn_id = prov_turn_raw
        .map(|s| s.parse::<TurnId>())
        .transpose()
        .map_err(|e| invalid(format!("invalid provenance turn_id: {e}")))?;
    let excerpt = excerpt_raw
        .map(|s| MemoryExcerpt::try_from(s.as_str()))
        .transpose()
        .map_err(|e| invalid(format!("invalid provenance excerpt: {e}")))?;
    let millis = |raw: i64, field: &str| {
        u64::try_from(raw)
            .map(UnixMillis::from_millis)
            .map_err(|_| invalid(format!("negative {field} {raw}")))
    };
    let created_at = millis(created_at_raw, "created_at")?;
    let updated_at = millis(updated_at_raw, "updated_at")?;
    let expires_at = expires_at_raw
        .map(|raw| millis(raw, "expires_at"))
        .transpose()?;
    let superseded_by = superseded_by_raw
        .map(|s| s.parse::<MemoryId>())
        .transpose()
        .map_err(|e| invalid(format!("invalid superseded_by memory_id: {e}")))?;

    Ok(MemoryRecord {
        memory_id,
        content,
        scope,
        kind,
        state,
        confidence: confidence_raw,
        sensitivity,
        source,
        provenance: MemoryProvenance {
            thread_id,
            turn_id,
            excerpt,
        },
        created_at,
        updated_at,
        expires_at,
        superseded_by,
    })
}

impl Store {
    /// Validates a candidate memory draft against the secret shape detector
    /// and bounds before durable append.
    fn validate_draft_before_append(draft: &MemoryDraft) -> Result<(), StorageError> {
        if is_secret_shaped(draft.content.as_str()) {
            return Err(StorageError::SecretShapedContent);
        }
        if let Some(excerpt) = &draft.provenance.excerpt
            && is_secret_shaped(excerpt.as_str())
        {
            return Err(StorageError::SecretShapedContent);
        }
        if draft.confidence > 100 {
            return Err(StorageError::InvalidDomainEvent {
                detail: format!("confidence {} exceeds 100", draft.confidence),
            });
        }
        Ok(())
    }

    /// Proposes a new candidate memory record (inferred from conversation or model).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::SecretShapedContent`] if content matches secrets,
    /// or SQLite/validation errors.
    pub fn propose_memory(
        &mut self,
        mut draft: MemoryDraft,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        draft.state = Some(MemoryState::Candidate);
        draft.source = MemorySource::Inferred;
        self.create_memory_with_kind(&draft, DomainEventKind::MemoryProposed, occurred_at)
    }

    /// Directly creates and confirms a memory record (e.g., explicit user statement).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::SecretShapedContent`] if content matches secrets,
    /// or SQLite/validation errors.
    pub fn create_memory(
        &mut self,
        draft: &MemoryDraft,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        let kind = match draft.state {
            Some(MemoryState::Candidate) => DomainEventKind::MemoryProposed,
            _ => DomainEventKind::MemoryConfirmed,
        };
        self.create_memory_with_kind(draft, kind, occurred_at)
    }

    fn create_memory_with_kind(
        &mut self,
        draft: &MemoryDraft,
        kind: DomainEventKind,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        Self::validate_draft_before_append(draft)?;

        let mem_id = draft
            .id
            .clone()
            .unwrap_or_else(|| generate_memory_id(occurred_at, 1));
        let event_id = generate_event_id(occurred_at, 1);

        let state_str = draft
            .state
            .unwrap_or(if matches!(kind, DomainEventKind::MemoryProposed) {
                MemoryState::Candidate
            } else {
                MemoryState::Confirmed
            })
            .as_str();

        let payload_json = serde_json::json!({
            "memory_id": mem_id.as_str(),
            "content": draft.content.as_str(),
            "scope_kind": draft.scope.kind_str(),
            "scope_target": draft.scope.target_str(),
            "kind": draft.kind.as_str(),
            "state": state_str,
            "confidence": draft.confidence,
            "sensitivity": draft.sensitivity.as_str(),
            "source": draft.source.as_str(),
            "explicit": draft.source.is_explicit(),
            "provenance_thread_id": draft.provenance.thread_id.as_ref().map(altior_domain::ThreadId::as_str),
            "provenance_turn_id": draft.provenance.turn_id.as_ref().map(altior_domain::TurnId::as_str),
            "excerpt": draft.provenance.excerpt.as_ref().map(altior_domain::MemoryExcerpt::as_str),
            "expires_at": draft.expires_at.map(altior_domain::UnixMillis::as_millis),
        });

        let payload_bytes =
            serde_json::to_vec(&payload_json).map_err(|e| StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            })?;
        let payload = EventPayload::try_from(payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let domain_event = DomainEvent {
            event_id,
            thread_id: draft.provenance.thread_id.clone(),
            turn_id: draft.provenance.turn_id.clone(),
            operation_id: None,
            kind,
            payload,
            occurred_at,
        };

        self.append_domain_event(&domain_event)?;

        self.get_memory(&mem_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: mem_id.to_string(),
            })
    }

    /// Confirms a candidate memory, moving its state to `Confirmed` and making
    /// it retrievable in search.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::MemoryNotFound`] if not found, or
    /// [`StorageError::InvalidDomainEvent`] if not in `Candidate` state.
    pub fn confirm_memory(
        &mut self,
        memory_id: &MemoryId,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        let existing = self
            .get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })?;

        if existing.state != MemoryState::Candidate {
            return Err(StorageError::InvalidDomainEvent {
                detail: format!(
                    "cannot confirm memory in '{}' state; candidate required",
                    existing.state.as_str()
                ),
            });
        }

        let event_id = generate_event_id(occurred_at, 2);
        let payload_json = serde_json::json!({
            "memory_id": memory_id.as_str(),
        });
        let payload_bytes =
            serde_json::to_vec(&payload_json).map_err(|e| StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            })?;
        let payload = EventPayload::try_from(payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let domain_event = DomainEvent {
            event_id,
            thread_id: existing.provenance.thread_id.clone(),
            turn_id: existing.provenance.turn_id.clone(),
            operation_id: None,
            kind: DomainEventKind::MemoryConfirmed,
            payload,
            occurred_at,
        };

        self.append_domain_event(&domain_event)?;

        self.get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })
    }

    /// Rejects a candidate memory, moving its state to `Rejected`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::MemoryNotFound`] if not found, or
    /// [`StorageError::InvalidDomainEvent`] if not in `Candidate` state.
    pub fn reject_memory(
        &mut self,
        memory_id: &MemoryId,
        reason: Option<&str>,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        let existing = self
            .get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })?;

        if existing.state != MemoryState::Candidate {
            return Err(StorageError::InvalidDomainEvent {
                detail: format!(
                    "cannot reject memory in '{}' state; candidate required",
                    existing.state.as_str()
                ),
            });
        }

        let event_id = generate_event_id(occurred_at, 3);
        let payload_json = serde_json::json!({
            "memory_id": memory_id.as_str(),
            "reason": reason,
        });
        let payload_bytes =
            serde_json::to_vec(&payload_json).map_err(|e| StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            })?;
        let payload = EventPayload::try_from(payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let domain_event = DomainEvent {
            event_id,
            thread_id: existing.provenance.thread_id.clone(),
            turn_id: existing.provenance.turn_id.clone(),
            operation_id: None,
            kind: DomainEventKind::MemoryRejected,
            payload,
            occurred_at,
        };

        self.append_domain_event(&domain_event)?;

        self.get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })
    }

    /// Corrects an existing confirmed memory by superseding it with a new memory record.
    ///
    /// Atomically records a `MemoryConfirmed` event for the replacement and a
    /// `MemorySuperseded` event for the original. The original memory's history
    /// is preserved intact while pointing to the new successor.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::MemoryNotFound`], [`StorageError::SecretShapedContent`],
    /// or [`StorageError::InvalidDomainEvent`] if original is not active confirmed.
    pub fn correct_memory(
        &mut self,
        memory_id: &MemoryId,
        draft: &MemoryDraft,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        Self::validate_draft_before_append(draft)?;

        let existing = self
            .get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })?;

        if existing.state != MemoryState::Confirmed || existing.superseded_by.is_some() {
            return Err(StorageError::InvalidDomainEvent {
                detail: "cannot correct non-confirmed or already-superseded memory".into(),
            });
        }

        let new_mem_id = draft
            .id
            .clone()
            .unwrap_or_else(|| generate_memory_id(occurred_at, 4));

        // 1. Create the new memory record as confirmed
        let new_event_id = generate_event_id(occurred_at, 5);
        let new_payload_json = serde_json::json!({
            "memory_id": new_mem_id.as_str(),
            "content": draft.content.as_str(),
            "scope_kind": draft.scope.kind_str(),
            "scope_target": draft.scope.target_str(),
            "kind": draft.kind.as_str(),
            "state": "confirmed",
            "confidence": draft.confidence,
            "sensitivity": draft.sensitivity.as_str(),
            "source": draft.source.as_str(),
            "explicit": draft.source.is_explicit(),
            "provenance_thread_id": draft.provenance.thread_id.as_ref().map(altior_domain::ThreadId::as_str),
            "provenance_turn_id": draft.provenance.turn_id.as_ref().map(altior_domain::TurnId::as_str),
            "excerpt": draft.provenance.excerpt.as_ref().map(altior_domain::MemoryExcerpt::as_str),
            "expires_at": draft.expires_at.map(altior_domain::UnixMillis::as_millis),
        });
        let new_payload_bytes = serde_json::to_vec(&new_payload_json).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;
        let new_payload = EventPayload::try_from(new_payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let create_event = DomainEvent {
            event_id: new_event_id,
            thread_id: draft.provenance.thread_id.clone(),
            turn_id: draft.provenance.turn_id.clone(),
            operation_id: None,
            kind: DomainEventKind::MemoryConfirmed,
            payload: new_payload,
            occurred_at,
        };
        self.append_domain_event(&create_event)?;

        // 2. Mark original memory as superseded by the new memory
        let super_event_id = generate_event_id(occurred_at, 6);
        let super_payload_json = serde_json::json!({
            "memory_id": memory_id.as_str(),
            "superseded_by": new_mem_id.as_str(),
        });
        let super_payload_bytes = serde_json::to_vec(&super_payload_json).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;
        let super_payload = EventPayload::try_from(super_payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let supersede_event = DomainEvent {
            event_id: super_event_id,
            thread_id: existing.provenance.thread_id.clone(),
            turn_id: existing.provenance.turn_id.clone(),
            operation_id: None,
            kind: DomainEventKind::MemorySuperseded,
            payload: super_payload,
            occurred_at,
        };
        self.append_domain_event(&supersede_event)?;

        self.get_memory(&new_mem_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: new_mem_id.to_string(),
            })
    }

    /// Forgets a confirmed memory, marking it with a durable tombstone (`state = Forgotten`).
    ///
    /// The record remains in the database for sync / audit history but is immediately
    /// removed from the `memory_fts` search index.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::MemoryNotFound`] if not found, or
    /// [`StorageError::InvalidDomainEvent`] if not in `Confirmed` state.
    pub fn forget_memory(
        &mut self,
        memory_id: &MemoryId,
        occurred_at: UnixMillis,
    ) -> Result<MemoryRecord, StorageError> {
        let existing = self
            .get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })?;

        if existing.state != MemoryState::Confirmed {
            return Err(StorageError::InvalidDomainEvent {
                detail: format!(
                    "cannot forget memory in '{}' state; confirmed required",
                    existing.state.as_str()
                ),
            });
        }

        let event_id = generate_event_id(occurred_at, 7);
        let payload_json = serde_json::json!({
            "memory_id": memory_id.as_str(),
        });
        let payload_bytes =
            serde_json::to_vec(&payload_json).map_err(|e| StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            })?;
        let payload = EventPayload::try_from(payload_bytes).map_err(|e| {
            StorageError::InvalidDomainEvent {
                detail: e.to_string(),
            }
        })?;

        let domain_event = DomainEvent {
            event_id,
            thread_id: existing.provenance.thread_id.clone(),
            turn_id: existing.provenance.turn_id.clone(),
            operation_id: None,
            kind: DomainEventKind::MemoryForgotten,
            payload,
            occurred_at,
        };

        self.append_domain_event(&domain_event)?;

        self.get_memory(memory_id)?
            .ok_or_else(|| StorageError::MemoryNotFound {
                memory_id: memory_id.to_string(),
            })
    }

    /// Sweeps past-due memories, appending `MemoryExpired` events and updating their state.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on query failure.
    pub fn sweep_expired_memories(
        &mut self,
        now: UnixMillis,
    ) -> Result<Vec<MemoryId>, StorageError> {
        let now_i64 = i64::try_from(now.as_millis()).unwrap_or(i64::MAX);

        let mut stmt = self
            .conn
            .prepare(
                "SELECT memory_id FROM memory
                 WHERE state = 'confirmed'
                   AND expires_at IS NOT NULL
                   AND expires_at <= ?1",
            )
            .map_err(|e| StorageError::from_sqlite("sweep_expired_memories prepare", e))?;

        let expired_ids: Vec<MemoryId> = stmt
            .query_map(params![now_i64], |row| {
                let id_raw: String = row.get(0)?;
                Ok(id_raw)
            })
            .map_err(|e| StorageError::from_sqlite("sweep_expired_memories query", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StorageError::from_sqlite("sweep_expired_memories collect", e))?
            .into_iter()
            .filter_map(|raw| raw.parse::<MemoryId>().ok())
            .collect();

        drop(stmt);

        let mut disambiguator = 100u32;
        for mem_id in &expired_ids {
            let event_id = generate_event_id(now, disambiguator);
            disambiguator = disambiguator.saturating_add(1);

            let payload_json = serde_json::json!({
                "memory_id": mem_id.as_str(),
            });
            let payload_bytes = serde_json::to_vec(&payload_json).map_err(|e| {
                StorageError::InvalidDomainEvent {
                    detail: e.to_string(),
                }
            })?;
            let payload = EventPayload::try_from(payload_bytes).map_err(|e| {
                StorageError::InvalidDomainEvent {
                    detail: e.to_string(),
                }
            })?;

            let domain_event = DomainEvent {
                event_id,
                thread_id: None,
                turn_id: None,
                operation_id: None,
                kind: DomainEventKind::MemoryExpired,
                payload,
                occurred_at: now,
            };

            self.append_domain_event(&domain_event)?;
        }

        Ok(expired_ids)
    }

    /// Fetches a memory record by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on query failure.
    pub fn get_memory(&self, memory_id: &MemoryId) -> Result<Option<MemoryRecord>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT memory_id, content, scope_kind, scope_target, kind, state,
                        confidence, sensitivity, source, explicit,
                        provenance_thread_id, provenance_turn_id, excerpt,
                        created_at, updated_at, expires_at, superseded_by
                 FROM memory WHERE memory_id = ?1",
            )
            .map_err(|e| StorageError::from_sqlite("get_memory prepare", e))?;

        let mut rows = stmt
            .query(params![memory_id.as_str()])
            .map_err(|e| StorageError::from_sqlite("get_memory query", e))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| StorageError::from_sqlite("get_memory row", e))?
        {
            Ok(Some(row_to_memory_record(row)?))
        } else {
            Ok(None)
        }
    }

    /// Lists memory records with stable cursor-based pagination and optional filters.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on query failure.
    pub fn list_memories(
        &self,
        scope_filter: Option<&MemoryScope>,
        state_filter: Option<MemoryState>,
        limit: MemoryListLimit,
        cursor: Option<&MemoryCursor>,
    ) -> Result<Vec<MemoryRecord>, StorageError> {
        let scope_kind = scope_filter.map(altior_domain::MemoryScope::kind_str);
        let scope_target = scope_filter.and_then(altior_domain::MemoryScope::target_str);
        let state_str = state_filter.map(altior_domain::MemoryState::as_str);

        let (before_updated_at, before_memory_id) = cursor.map_or((i64::MAX, ""), |c| {
            (
                i64::try_from(c.updated_at.as_millis()).unwrap_or(i64::MAX),
                c.memory_id.as_str(),
            )
        });

        let mut statement = self
            .conn
            .prepare(
                "SELECT memory_id, content, scope_kind, scope_target, kind, state,
                        confidence, sensitivity, source, explicit,
                        provenance_thread_id, provenance_turn_id, excerpt,
                        created_at, updated_at, expires_at, superseded_by
                 FROM memory
                 WHERE (?1 IS NULL OR scope_kind = ?1)
                   AND (?2 IS NULL OR scope_target = ?2)
                   AND (?3 IS NULL OR state = ?3)
                   AND (updated_at < ?4 OR (updated_at = ?4 AND memory_id < ?5))
                 ORDER BY updated_at DESC, memory_id DESC
                 LIMIT ?6",
            )
            .map_err(|e| StorageError::from_sqlite("list_memories prepare", e))?;

        let rows = statement
            .query(params![
                scope_kind,
                scope_target,
                state_str,
                before_updated_at,
                before_memory_id,
                i64::from(limit.get()),
            ])
            .map_err(|e| StorageError::from_sqlite("list_memories query", e))?;

        collect_rows(rows, row_to_memory_record, "list_memories")
    }

    /// Full-text search over retrievable memories with deterministic composite ranking.
    ///
    /// Escapes query terms for FTS5, filters lazy-expired records against `now`,
    /// evaluates scope affinity, recency decay, confidence, and explicitness bonus,
    /// and populates explainable [`MemoryMatchExplanation`] structures.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on query failure.
    #[allow(clippy::too_many_lines)]
    pub fn search_memories(
        &self,
        query: &SearchQuery,
        scope_filter: Option<&MemoryScope>,
        limit: MemorySearchLimit,
        now: UnixMillis,
    ) -> Result<Vec<MemoryHit>, StorageError> {
        let query_str = query.as_str().trim();
        if query_str.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize query into non-empty tokens and build literal FTS5 query
        let tokens: Vec<String> = query_str
            .split_whitespace()
            .map(|tok| {
                tok.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|tok| !tok.is_empty())
            .collect();

        if tokens.is_empty() {
            return Ok(vec![]);
        }

        // FTS5 literal escaping: enclose each token in double quotes and escape internal quotes
        let fts5_match_expr = tokens
            .iter()
            .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");

        let now_i64 = i64::try_from(now.as_millis()).unwrap_or(i64::MAX);

        let mut statement = self
            .conn
            .prepare(
                "SELECT m.memory_id, m.content, m.scope_kind, m.scope_target, m.kind, m.state,
                        m.confidence, m.sensitivity, m.source, m.explicit,
                        m.provenance_thread_id, m.provenance_turn_id, m.excerpt,
                        m.created_at, m.updated_at, m.expires_at, m.superseded_by,
                        bm25(memory_fts) as rank
                 FROM memory_fts
                 JOIN memory m ON m.memory_id = memory_fts.memory_id
                 WHERE memory_fts MATCH ?1
                   AND m.state = 'confirmed'
                   AND m.superseded_by IS NULL
                   AND (m.expires_at IS NULL OR m.expires_at > ?2)",
            )
            .map_err(|e| StorageError::from_sqlite("search_memories prepare", e))?;

        let mut rows = statement
            .query(params![fts5_match_expr, now_i64])
            .map_err(|e| StorageError::from_sqlite("search_memories query", e))?;

        let mut hits = Vec::new();

        while let Some(row) = rows
            .next()
            .map_err(|e| StorageError::from_sqlite("search_memories row", e))?
        {
            let record = row_to_memory_record(row)?;
            let raw_bm25: f64 = row
                .get(17)
                .map_err(|e| StorageError::from_sqlite("search_memories bm25 rank", e))?;

            // Scope filter matching
            if let Some(filter) = scope_filter
                && !record.scope.matches_scope(filter)
            {
                continue;
            }

            // Scope affinity weight
            let scope_weight = match scope_filter {
                Some(filter) if record.scope == *filter => 1.5,
                Some(_) if matches!(record.scope, MemoryScope::Global) => 1.0,
                Some(_) => 0.8,
                None => 1.0,
            };

            // Confidence factor (0.0..=1.0)
            let confidence_score = f64::from(record.confidence) / 100.0;

            // Recency decay: half-life based on days elapsed
            let age_millis = now
                .as_millis()
                .saturating_sub(record.updated_at.as_millis());
            #[allow(clippy::cast_precision_loss)]
            let age_days = (age_millis as f64) / 86_400_000.0;
            let recency_score = 1.0 / (1.0 + age_days * 0.05);

            // Explicitness bonus
            let explicitness_bonus = if record.source.is_explicit() {
                0.2
            } else {
                0.0
            };

            // Text relevance: SQLite bm25 produces negative numbers (lower is better)
            let text_score = (-raw_bm25).max(0.1);

            // Deterministic composite ranking
            let total_score = (text_score * 0.40)
                + (scope_weight * 0.25)
                + (confidence_score * 0.20)
                + (recency_score * 0.10)
                + explicitness_bonus;

            // Matched query tokens in content
            let lower_content = record.content.as_str().to_lowercase();
            let matched_terms: Vec<String> = tokens
                .iter()
                .filter(|tok| lower_content.contains(tok.as_str()))
                .cloned()
                .collect();

            let why_selected = format!(
                "matched terms: {matched_terms:?}, bm25: {raw_bm25:.3}, scope_weight: {scope_weight:.2}, \
                 confidence: {confidence_score:.2}, recency: {recency_score:.2}, explicit_bonus: {explicitness_bonus:.2}, \
                 total: {total_score:.4}"
            );

            hits.push(MemoryHit {
                record,
                explanation: MemoryMatchExplanation {
                    matched_terms,
                    fts_rank: raw_bm25,
                    scope_weight,
                    confidence_score,
                    recency_score,
                    explicitness_bonus,
                    total_score,
                    why_selected,
                },
            });
        }

        // Sort by total_score descending, with deterministic tie-breakers
        hits.sort_by(|a, b| {
            b.explanation
                .total_score
                .partial_cmp(&a.explanation.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.record.updated_at.cmp(&a.record.updated_at))
                .then_with(|| b.record.memory_id.cmp(&a.record.memory_id))
        });

        hits.truncate(limit.get() as usize);

        Ok(hits)
    }
}
