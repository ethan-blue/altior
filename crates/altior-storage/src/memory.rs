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

/// Algorithm version stamped on retrieval explanations and documented in ADR 0023.
pub const RETRIEVAL_ALGORITHM_VERSION: &str = "v2_bounded_scope_pushdown_trigram_hybrid";

/// Returns standard English root variants for trigram substring robustness (ADR 0023).
fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        ',' | '.'
            | '?'
            | '!'
            | ';'
            | ':'
            | '"'
            | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '，'
            | '。'
            | '？'
            | '！'
            | '；'
            | '：'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '（'
            | '）'
            | '《'
            | '》'
            | '、'
    )
}

fn english_stem_variants(word: &str) -> Vec<String> {
    let mut variants = vec![word.to_string()];
    let w = word.to_lowercase();
    if w.ends_with("ments") && w.len() > 7 {
        variants.push(w[..w.len() - 5].to_string());
    } else if w.ends_with("ment") && w.len() > 6 {
        variants.push(w[..w.len() - 4].to_string());
    } else if w.ends_with("ing") && w.len() > 5 {
        variants.push(w[..w.len() - 3].to_string());
    } else if w.ends_with("ed") && w.len() > 4 {
        variants.push(w[..w.len() - 2].to_string());
        if w.ends_with("red") && w.len() > 5 {
            variants.push(w[..w.len() - 3].to_string());
        }
    } else if w.ends_with("es") && w.len() > 4 {
        variants.push(w[..w.len() - 2].to_string());
    } else if w.ends_with('s') && w.len() > 3 {
        variants.push(w[..w.len() - 1].to_string());
    } else if w.ends_with("tion") && w.len() > 6 {
        variants.push(w[..w.len() - 4].to_string());
    }
    variants
}

#[derive(Clone, PartialEq)]
struct CandidateOrderKey {
    total_score: f64,
    updated_at: UnixMillis,
    memory_id: MemoryId,
}

impl Eq for CandidateOrderKey {}

impl PartialOrd for CandidateOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CandidateOrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.total_score
            .partial_cmp(&other.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| self.updated_at.cmp(&other.updated_at))
            .then_with(|| self.memory_id.cmp(&other.memory_id))
    }
}

struct ScoredCandidate {
    order_key: CandidateOrderKey,
    record: MemoryRecord,
    raw_bm25: f64,
    scope_weight: f64,
    confidence_score: f64,
    recency_score: f64,
    explicitness_bonus: f64,
}

impl PartialEq for ScoredCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.order_key == other.order_key
    }
}

impl Eq for ScoredCandidate {}

impl PartialOrd for ScoredCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order_key.cmp(&other.order_key)
    }
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

        let mut raw_tokens = Vec::new();
        for word in query_str.split_whitespace() {
            let lower = word.to_lowercase();
            let trimmed = lower.trim_matches(is_punctuation).to_string();
            let tok = if trimmed.is_empty() { lower } else { trimmed };
            if !tok.is_empty() && !raw_tokens.contains(&tok) {
                raw_tokens.push(tok);
            }
        }

        if raw_tokens.is_empty() {
            return Ok(vec![]);
        }

        let mut long_tokens = Vec::new();
        let mut short_tokens = Vec::new();
        let mut matched_candidates = raw_tokens.clone();

        for tok in &raw_tokens {
            let char_count = tok.chars().count();
            if char_count >= 3 {
                for variant in english_stem_variants(tok) {
                    if variant.chars().count() >= 3 && !long_tokens.contains(&variant) {
                        long_tokens.push(variant);
                    }
                }
                let chars: Vec<char> = tok.chars().collect();
                if chars
                    .iter()
                    .any(|&c| ('\u{4e00}'..='\u{9fff}').contains(&c))
                {
                    for window in chars.windows(3) {
                        let trigram: String = window.iter().collect();
                        if !long_tokens.contains(&trigram) {
                            long_tokens.push(trigram.clone());
                        }
                        if !matched_candidates.contains(&trigram) {
                            matched_candidates.push(trigram);
                        }
                    }
                }
            } else if !short_tokens.contains(tok) {
                short_tokens.push(tok.clone());
            }
        }

        let now_i64 = i64::try_from(now.as_millis()).unwrap_or(i64::MAX);

        let (scope_sql_fts, scope_sql_instr, scope_target_param) = match scope_filter {
            None => ("", "", None),
            Some(MemoryScope::Global) => (
                "AND m.scope_kind = 'global'",
                "AND m.scope_kind = 'global'",
                None,
            ),
            Some(MemoryScope::Project(lbl)) => (
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'project' AND m.scope_target = ?3))",
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'project' AND m.scope_target = ?2))",
                Some(lbl.as_str().to_string()),
            ),
            Some(MemoryScope::Person(lbl)) => (
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'person' AND m.scope_target = ?3))",
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'person' AND m.scope_target = ?2))",
                Some(lbl.as_str().to_string()),
            ),
            Some(MemoryScope::Thread(lbl)) => (
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'thread' AND m.scope_target = ?3))",
                "AND (m.scope_kind = 'global' OR (m.scope_kind = 'thread' AND m.scope_target = ?2))",
                Some(lbl.as_str().to_string()),
            ),
        };

        // Candidate collector deduplicating by memory_id
        let mut candidate_map: std::collections::HashMap<MemoryId, (MemoryRecord, f64)> =
            std::collections::HashMap::new();

        // 1. Long tokens: FTS5 Trigram
        if !long_tokens.is_empty() {
            let fts5_match_expr = long_tokens
                .iter()
                .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ");

            let sql = format!(
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
                   AND (m.expires_at IS NULL OR m.expires_at > ?2)
                   {scope_sql_fts}"
            );

            let mut statement = self
                .conn
                .prepare(&sql)
                .map_err(|e| StorageError::from_sqlite("search_memories fts prepare", e))?;

            let mut params_vec: Vec<rusqlite::types::Value> = vec![
                rusqlite::types::Value::Text(fts5_match_expr),
                rusqlite::types::Value::Integer(now_i64),
            ];
            if let Some(target) = scope_target_param.as_ref() {
                params_vec.push(rusqlite::types::Value::Text(target.clone()));
            }

            let mut rows = statement
                .query(rusqlite::params_from_iter(params_vec))
                .map_err(|e| StorageError::from_sqlite("search_memories fts query", e))?;

            while let Some(row) = rows
                .next()
                .map_err(|e| StorageError::from_sqlite("search_memories fts row", e))?
            {
                let record = row_to_memory_record(row)?;
                let raw_bm25: f64 = row
                    .get(17)
                    .map_err(|e| StorageError::from_sqlite("search_memories fts bm25 rank", e))?;

                candidate_map
                    .entry(record.memory_id.clone())
                    .and_modify(|(_, existing_rank)| {
                        if raw_bm25 < *existing_rank {
                            *existing_rank = raw_bm25;
                        }
                    })
                    .or_insert((record, raw_bm25));
            }
        }

        // 2. Short tokens: Substring instr
        if !short_tokens.is_empty() {
            let mut instr_conditions = Vec::new();
            for tok in &short_tokens {
                let safe_tok = tok.replace('\'', "''");
                instr_conditions.push(format!("instr(lower(m.content), '{safe_tok}') > 0"));
            }
            let instr_where = instr_conditions.join(" OR ");

            let sql = format!(
                "SELECT m.memory_id, m.content, m.scope_kind, m.scope_target, m.kind, m.state,
                        m.confidence, m.sensitivity, m.source, m.explicit,
                        m.provenance_thread_id, m.provenance_turn_id, m.excerpt,
                        m.created_at, m.updated_at, m.expires_at, m.superseded_by,
                        -0.5 as rank
                 FROM memory m
                 WHERE m.state = 'confirmed'
                   AND m.superseded_by IS NULL
                   AND (m.expires_at IS NULL OR m.expires_at > ?1)
                   AND ({instr_where})
                   {scope_sql_instr}"
            );

            let mut statement = self
                .conn
                .prepare(&sql)
                .map_err(|e| StorageError::from_sqlite("search_memories instr prepare", e))?;

            let mut params_vec: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Integer(now_i64)];
            if let Some(target) = scope_target_param.as_ref() {
                params_vec.push(rusqlite::types::Value::Text(target.clone()));
            }

            let mut rows = statement
                .query(rusqlite::params_from_iter(params_vec))
                .map_err(|e| StorageError::from_sqlite("search_memories instr query", e))?;

            while let Some(row) = rows
                .next()
                .map_err(|e| StorageError::from_sqlite("search_memories instr row", e))?
            {
                let record = row_to_memory_record(row)?;
                let raw_bm25: f64 = -0.5;

                candidate_map
                    .entry(record.memory_id.clone())
                    .or_insert((record, raw_bm25));
            }
        }

        if candidate_map.is_empty() {
            return Ok(vec![]);
        }

        // 3. Bounded Top-K Min-Heap selection
        let k_cap = limit.get() as usize;
        let mut min_heap: std::collections::BinaryHeap<std::cmp::Reverse<ScoredCandidate>> =
            std::collections::BinaryHeap::with_capacity(k_cap.saturating_add(1));

        for (record, raw_bm25) in candidate_map.into_values() {
            let scope_weight = match scope_filter {
                Some(filter) if record.scope == *filter => 1.5,
                Some(_) if matches!(record.scope, MemoryScope::Global) => 1.0,
                Some(_) => 0.8,
                None => 1.0,
            };

            let confidence_score = f64::from(record.confidence) / 100.0;
            let age_millis = now
                .as_millis()
                .saturating_sub(record.updated_at.as_millis());
            #[allow(clippy::cast_precision_loss)]
            let age_days = (age_millis as f64) / 86_400_000.0;
            let recency_score = 1.0 / (1.0 + age_days * 0.05);

            let explicitness_bonus = if record.source.is_explicit() {
                0.2
            } else {
                0.0
            };
            let text_score = (-raw_bm25).max(0.0);

            let total_score = (text_score * 0.40)
                + (scope_weight * 0.25)
                + (confidence_score * 0.20)
                + (recency_score * 0.10)
                + explicitness_bonus;

            let order_key = CandidateOrderKey {
                total_score,
                updated_at: record.updated_at,
                memory_id: record.memory_id.clone(),
            };

            let candidate = ScoredCandidate {
                order_key,
                record,
                raw_bm25,
                scope_weight,
                confidence_score,
                recency_score,
                explicitness_bonus,
            };

            if min_heap.len() < k_cap {
                min_heap.push(std::cmp::Reverse(candidate));
            } else if min_heap
                .peek()
                .is_some_and(|smallest| candidate.order_key > smallest.0.order_key)
            {
                min_heap.pop();
                min_heap.push(std::cmp::Reverse(candidate));
            }
        }

        // 4. Extract top-K candidates and sort in descending order
        let mut top_candidates: Vec<ScoredCandidate> =
            min_heap.into_iter().map(|rev| rev.0).collect();
        top_candidates.sort_by(|a, b| b.order_key.cmp(&a.order_key));

        // 5. Construct explanations strictly for top-K winners
        let mut hits = Vec::with_capacity(top_candidates.len());
        for c in top_candidates {
            let lower_content = c.record.content.as_str().to_lowercase();
            let matched_terms: Vec<String> = matched_candidates
                .iter()
                .filter(|tok| lower_content.contains(tok.as_str()))
                .cloned()
                .collect();

            let why_selected = format!(
                "algorithm: {RETRIEVAL_ALGORITHM_VERSION}, matched terms: {matched_terms:?}, fts_rank: {:.3}, scope_weight: {:.2},                  confidence: {:.2}, recency: {:.2}, explicit_bonus: {:.2}, total: {:.4}",
                c.raw_bm25,
                c.scope_weight,
                c.confidence_score,
                c.recency_score,
                c.explicitness_bonus,
                c.order_key.total_score
            );

            hits.push(MemoryHit {
                record: c.record,
                explanation: MemoryMatchExplanation {
                    matched_terms,
                    fts_rank: c.raw_bm25,
                    scope_weight: c.scope_weight,
                    confidence_score: c.confidence_score,
                    recency_score: c.recency_score,
                    explicitness_bonus: c.explicitness_bonus,
                    total_score: c.order_key.total_score,
                    why_selected,
                },
            });
        }

        Ok(hits)
    }
}
