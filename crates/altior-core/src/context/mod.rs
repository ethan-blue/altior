//! Deterministic context assembly and token budgeting engine (CORE P2.2, ADR 0018).
//!
//! # Architecture & Determinism
//!
//! A [`ContextSnapshot`] is the deterministic, serializable audit record of
//! everything injected into one turn's wire prompt:
//!
//! 1. **Pure Clock Injection**: Caller explicitly passes [`UnixMillis`]; no wall-clock I/O.
//! 2. **Algorithmic Token Estimation**:
//!    - An empty string yields 0 tokens.
//!    - For any non-empty UTF-8 string, `estimate_tokens(s) = ceil(s.len() / 4)`,
//!      computed via `(s.len() + 3) / 4`.
//!    - Pure, model-agnostic, zero floating-point drift, zero tokenizer dependencies.
//! 3. **Fixed Section Ordering**:
//!    `[Identity Section]` -> `[Memory Section]` -> `[User Prompt]`.
//! 4. **Stable Tie-Breaking**:
//!    - Identity documents: `kind.render_priority()` ASC (Name < About < Instruction < Preference),
//!      then `created_at` ASC, then `identity_document_id` ASC.
//!    - Memory hits: `total_score` DESC, then `created_at` ASC, then `memory_id` ASC.
//! 5. **Byte-for-Byte Passthrough**:
//!    When no identity documents and no memories are selected, the wire prompt is
//!    guaranteed to be **byte-for-byte identical** to the user prompt.
//! 6. **Fail-Closed Secret Protection**:
//!    Any secret-shaped patterns in user prompt, identity documents, or candidate memories
//!    trigger an immediate fail-closed error before rendering, never polluting journals or logs.

use std::fmt;

use altior_domain::{
    CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES, ContextDegradation, ContextDropReason, ContextDroppedEntry,
    ContextIdentityEntry, ContextMemoryEntry, ContextSnapshot, ContextTokenBudget,
    IdentityDocument, MemoryHit, ThreadId, TurnId, UnixMillis, is_secret_shaped,
};

/// Default token budget allocated to injected identity documents.
pub const DEFAULT_IDENTITY_LIMIT_TOKENS: u32 = 1024;

/// Default token budget allocated to injected retrievable memories.
pub const DEFAULT_MEMORY_LIMIT_TOKENS: u32 = 2048;

const IDENTITY_HEADER: &str = "# Identity\n";
const MEMORY_HEADER: &str = "# Relevant Memories\n";

/// Algorithmic token estimation:
///
/// An exact tokenizer dependency introduces model coupling, binary bloat,
/// and non-deterministic version drift. Altior uses a pure, deterministic,
/// model-agnostic byte-heuristic:
///
/// - An empty string yields 0 tokens.
/// - For any non-empty UTF-8 string, `estimate_tokens(s) = ceil(s.len() / 4)`,
///   computed via `(s.len() + 3) / 4`.
///
/// This aligns with the standard cross-model rule-of-thumb that 1 token is
/// approximately 4 UTF-8 bytes.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        0
    } else {
        u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
    }
}

/// Token budget configuration for turn context assembly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextBudgetConfig {
    /// Maximum tokens allocated to the identity section (including framing).
    pub identity_limit_tokens: u32,
    /// Maximum tokens allocated to the memory section (including framing).
    pub memory_limit_tokens: u32,
}

impl Default for ContextBudgetConfig {
    fn default() -> Self {
        Self {
            identity_limit_tokens: DEFAULT_IDENTITY_LIMIT_TOKENS,
            memory_limit_tokens: DEFAULT_MEMORY_LIMIT_TOKENS,
        }
    }
}

/// Error produced during context assembly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblerError {
    /// A secret-shaped pattern was detected in input data.
    SecretShapedInput(&'static str),
    /// The serialized `ContextSnapshot` exceeded maximum allowed payload size.
    PayloadTooLarge { size: usize, limit: usize },
    /// JSON serialization failed.
    Serialization(String),
    /// General context assembly failure.
    Other(String),
}

impl fmt::Display for AssemblerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecretShapedInput(kind) => {
                write!(
                    f,
                    "secret-shaped pattern detected in {kind}; refused to assemble context"
                )
            }
            Self::PayloadTooLarge { size, limit } => {
                write!(
                    f,
                    "context snapshot payload size {size} exceeds limit {limit}"
                )
            }
            Self::Serialization(detail) => {
                write!(f, "failed to serialize context snapshot: {detail}")
            }
            Self::Other(msg) => write!(f, "context assembler error: {msg}"),
        }
    }
}

impl std::error::Error for AssemblerError {}

/// Parameters for deterministic context assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct AssembleParams<'a> {
    /// The turn identifier.
    pub turn_id: &'a TurnId,
    /// The thread identifier.
    pub thread_id: &'a ThreadId,
    /// Memory mode at assembly time (e.g. "off", "session", "`long_term`").
    pub memory_mode: &'a str,
    /// Raw user prompt text.
    pub user_prompt: &'a str,
    /// Candidate device-local identity documents.
    pub identity_docs: &'a [IdentityDocument],
    /// Candidate retrievable memory hits.
    pub memory_hits: &'a [MemoryHit],
    /// Token budget configuration.
    pub budget: ContextBudgetConfig,
    /// Explicit timestamp passed by caller.
    pub now: UnixMillis,
    /// Optional pre-existing degradation (e.g. search query truncated).
    pub degraded: Option<ContextDegradation>,
}

/// Result of deterministic context assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct AssembledContext {
    /// The assembled prompt text to send to the harness boundary.
    pub wire_prompt: String,
    /// The persistent audit snapshot recording selection and token accounting.
    pub snapshot: ContextSnapshot,
}

/// Assembles context deterministically according to ADR 0018.
///
/// # Invariants
///
/// 1. When `identity_docs` and `memory_hits` contribute zero entries,
///    `assembled.wire_prompt == user_prompt` (exact byte equality).
/// 2. Secret shapes in user prompt, identity docs, or memories are rejected fail-closed.
/// 3. Ordering is completely deterministic with stable tie-breaking.
///
/// # Errors
///
/// Returns [`AssemblerError::SecretShapedInput`] if any secret is detected,
/// or [`AssemblerError::PayloadTooLarge`] if snapshot size exceeds 64 KiB.
#[allow(clippy::too_many_lines)]
pub fn assemble_context(params: AssembleParams<'_>) -> Result<AssembledContext, AssemblerError> {
    // ── 1. Fail-closed secret scan on raw inputs ───────────────────
    if is_secret_shaped(params.user_prompt) {
        return Err(AssemblerError::SecretShapedInput("user_prompt"));
    }
    for doc in params.identity_docs {
        if is_secret_shaped(doc.content.as_str()) {
            return Err(AssemblerError::SecretShapedInput("identity_document"));
        }
    }
    for hit in params.memory_hits {
        if is_secret_shaped(hit.record.content.as_str()) {
            return Err(AssemblerError::SecretShapedInput("memory_content"));
        }
        if is_secret_shaped(&hit.explanation.why_selected) {
            return Err(AssemblerError::SecretShapedInput("memory_explanation"));
        }
    }

    let mut degraded = params.degraded;

    // ── 2. Sort Identity Documents (stable priority + timestamp + id)
    let mut sorted_identity_docs: Vec<&IdentityDocument> = params.identity_docs.iter().collect();
    sorted_identity_docs.sort_by(|a, b| {
        a.kind
            .render_priority()
            .cmp(&b.kind.render_priority())
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });

    // ── 3. Identity Budget Selection ───────────────────────────────
    let identity_header_tokens = estimate_tokens(IDENTITY_HEADER);
    let mut selected_identities = Vec::new();
    let mut identity_lines = Vec::new();
    let mut used_identity_tokens = 0u32;
    let mut identity_budget_exhausted = false;

    if params.budget.identity_limit_tokens >= identity_header_tokens {
        for doc in &sorted_identity_docs {
            let line = format!("- [{}]: {}\n", doc.kind.as_str(), doc.content.as_str());
            let line_tokens = estimate_tokens(&line);
            let needed = if selected_identities.is_empty() {
                identity_header_tokens.saturating_add(line_tokens)
            } else {
                used_identity_tokens.saturating_add(line_tokens)
            };

            if needed <= params.budget.identity_limit_tokens {
                if selected_identities.is_empty() {
                    used_identity_tokens = identity_header_tokens.saturating_add(line_tokens);
                } else {
                    used_identity_tokens = needed;
                }
                selected_identities.push(ContextIdentityEntry {
                    document_id: doc.id.clone(),
                    kind: doc.kind,
                    tokens: line_tokens,
                });
                identity_lines.push(line);
            } else {
                identity_budget_exhausted = true;
                break;
            }
        }
    }

    if identity_budget_exhausted && degraded.is_none() {
        degraded = Some(ContextDegradation {
            code: "identity_budget_exceeded".to_string(),
            detail: "some identity documents dropped due to identity token budget limit"
                .to_string(),
        });
    }

    let identity_tokens = if selected_identities.is_empty() {
        0
    } else {
        used_identity_tokens
    };

    // ── 4. Sort Memory Hits (stable score DESC + created_at ASC + id ASC)
    let mut sorted_hits: Vec<(usize, &MemoryHit)> = params.memory_hits.iter().enumerate().collect();
    sorted_hits.sort_by(|(_, a), (_, b)| {
        b.explanation
            .total_score
            .partial_cmp(&a.explanation.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.record.created_at.cmp(&b.record.created_at))
            .then_with(|| a.record.memory_id.as_str().cmp(b.record.memory_id.as_str()))
    });

    // ── 5. Memory Budget Selection and Dropped Entries ───────────────
    let memory_header_tokens = estimate_tokens(MEMORY_HEADER);
    let mut selected_memories = Vec::new();
    let mut memory_lines = Vec::new();
    let mut dropped = Vec::new();
    let mut used_memory_tokens = 0u32;
    let mut memory_budget_cutoff = false;

    for (orig_idx, hit) in sorted_hits {
        let rank = u32::try_from(orig_idx.saturating_add(1)).unwrap_or(u32::MAX);
        let line = format!(
            "- [{}]: {}\n",
            hit.record.kind.as_str(),
            hit.record.content.as_str()
        );
        let line_tokens = estimate_tokens(&line);

        if memory_budget_cutoff {
            dropped.push(ContextDroppedEntry {
                memory_id: hit.record.memory_id.clone(),
                tokens: line_tokens,
                rank,
                reason: ContextDropReason::BudgetExhausted,
            });
            continue;
        }

        let needed = if selected_memories.is_empty() {
            memory_header_tokens.saturating_add(line_tokens)
        } else {
            used_memory_tokens.saturating_add(line_tokens)
        };

        if needed <= params.budget.memory_limit_tokens {
            if selected_memories.is_empty() {
                used_memory_tokens = memory_header_tokens.saturating_add(line_tokens);
            } else {
                used_memory_tokens = needed;
            }
            selected_memories.push(ContextMemoryEntry {
                memory_id: hit.record.memory_id.clone(),
                kind: hit.record.kind,
                scope: hit.record.scope.clone(),
                confidence: hit.record.confidence,
                explicit: hit.record.source.is_explicit(),
                tokens: line_tokens,
                score: hit.explanation.total_score,
                why_selected: hit.explanation.why_selected.clone(),
                provenance_thread_id: hit.record.provenance.thread_id.clone(),
                provenance_turn_id: hit.record.provenance.turn_id.clone(),
            });
            memory_lines.push(line);
        } else {
            memory_budget_cutoff = true;
            dropped.push(ContextDroppedEntry {
                memory_id: hit.record.memory_id.clone(),
                tokens: line_tokens,
                rank,
                reason: ContextDropReason::BudgetExhausted,
            });
        }
    }

    let memory_tokens = if selected_memories.is_empty() {
        0
    } else {
        used_memory_tokens
    };

    // ── 6. Assemble Wire Prompt ────────────────────────────────────
    let passthrough = selected_identities.is_empty() && selected_memories.is_empty();
    let wire_prompt = if passthrough {
        // Strict invariant: byte-for-byte passthrough if nothing injected
        params.user_prompt.to_string()
    } else {
        let mut rendered = String::new();
        if !selected_identities.is_empty() {
            rendered.push_str(IDENTITY_HEADER);
            for line in &identity_lines {
                rendered.push_str(line);
            }
            rendered.push('\n');
        }
        if !selected_memories.is_empty() {
            rendered.push_str(MEMORY_HEADER);
            for line in &memory_lines {
                rendered.push_str(line);
            }
            rendered.push('\n');
        }
        rendered.push_str(params.user_prompt);
        rendered
    };

    // Defense-in-depth: fail-closed secret scan on fully assembled wire prompt
    if is_secret_shaped(&wire_prompt) {
        return Err(AssemblerError::SecretShapedInput("rendered_wire_prompt"));
    }

    // ── 7. Token Budget Accounting ─────────────────────────────────
    let prompt_tokens = estimate_tokens(params.user_prompt);
    let total_tokens = estimate_tokens(&wire_prompt);

    let budget = ContextTokenBudget {
        identity_limit_tokens: params.budget.identity_limit_tokens,
        memory_limit_tokens: params.budget.memory_limit_tokens,
        prompt_tokens,
        identity_tokens,
        memory_tokens,
        total_tokens,
    };

    let mut snapshot = ContextSnapshot {
        turn_id: params.turn_id.clone(),
        thread_id: params.thread_id.clone(),
        memory_mode: params.memory_mode.to_string(),
        created_at: params.now,
        passthrough,
        budget,
        identity: selected_identities,
        memories: selected_memories,
        dropped,
        degraded,
        rendered_prompt: if passthrough {
            None
        } else {
            Some(wire_prompt.clone())
        },
    };

    // ── 8. Snapshot Serialization & Size Bounding ──────────────────
    let mut payload = serde_json::to_string(&snapshot)
        .map_err(|e| AssemblerError::Serialization(e.to_string()))?;

    if payload.len() > CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES {
        // Omit rendered_prompt if payload exceeds maximum
        if snapshot.rendered_prompt.is_some() {
            snapshot.rendered_prompt = None;
            if snapshot.degraded.is_none() {
                snapshot.degraded = Some(ContextDegradation {
                    code: "rendered_prompt_omitted".to_string(),
                    detail: "rendered prompt omitted from audit snapshot due to payload size limit"
                        .to_string(),
                });
            }
            payload = serde_json::to_string(&snapshot)
                .map_err(|e| AssemblerError::Serialization(e.to_string()))?;
        }

        if payload.len() > CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES {
            return Err(AssemblerError::PayloadTooLarge {
                size: payload.len(),
                limit: CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES,
            });
        }
    }

    Ok(AssembledContext {
        wire_prompt,
        snapshot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use altior_domain::{
        IdentityContent, IdentityDocumentId, IdentityDocumentKind, MemoryContent, MemoryId,
        MemoryKind, MemoryMatchExplanation, MemoryProvenance, MemoryRecord, MemoryScope,
        MemorySensitivity, MemorySource, MemoryState,
    };
    use std::str::FromStr;

    fn make_identity_doc(
        id: &str,
        kind: IdentityDocumentKind,
        content: &str,
        created: u64,
    ) -> IdentityDocument {
        IdentityDocument {
            id: IdentityDocumentId::from_str(id).unwrap(),
            kind,
            content: IdentityContent::try_from(content).unwrap(),
            created_at: UnixMillis::from_millis(created),
            updated_at: UnixMillis::from_millis(created),
        }
    }

    fn make_memory_hit(
        id: &str,
        kind: MemoryKind,
        content: &str,
        score: f64,
        created: u64,
    ) -> MemoryHit {
        let mem_id = MemoryId::from_str(id).unwrap();
        let record = MemoryRecord {
            memory_id: mem_id,
            content: MemoryContent::try_from(content).unwrap(),
            scope: MemoryScope::Global,
            kind,
            state: MemoryState::Confirmed,
            confidence: 90,
            sensitivity: MemorySensitivity::Normal,
            source: MemorySource::Explicit,
            provenance: MemoryProvenance {
                thread_id: Some(ThreadId::from_str("thr_prov00000000000000001").unwrap()),
                turn_id: Some(TurnId::from_str("trn_prov00000000000000001").unwrap()),
                excerpt: None,
            },
            created_at: UnixMillis::from_millis(created),
            updated_at: UnixMillis::from_millis(created),
            expires_at: None,
            superseded_by: None,
        };
        let explanation = MemoryMatchExplanation {
            matched_terms: vec![],
            fts_rank: score,
            scope_weight: 1.0,
            confidence_score: 0.9,
            recency_score: 1.0,
            explicitness_bonus: 0.1,
            total_score: score,
            why_selected: format!("matched query with score {score}"),
        };
        MemoryHit {
            record,
            explanation,
        }
    }

    #[test]
    fn test_token_estimator_algorithm() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        assert_eq!(estimate_tokens("1234567890"), 3);
    }

    #[test]
    fn test_passthrough_exact_byte_equality() {
        let turn_id = TurnId::from_str("trn_test00000000000000001").unwrap();
        let thread_id = ThreadId::from_str("thr_test00000000000000001").unwrap();
        let raw_prompt = "Hello, world! Exactly byte-for-byte preserved.\nNo changes.";

        let res = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: raw_prompt,
            identity_docs: &[],
            memory_hits: &[],
            budget: ContextBudgetConfig::default(),
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap();

        assert_eq!(res.wire_prompt, raw_prompt);
        assert!(res.snapshot.passthrough);
        assert!(res.snapshot.is_passthrough());
        assert!(res.snapshot.identity.is_empty());
        assert!(res.snapshot.memories.is_empty());
        assert!(res.snapshot.dropped.is_empty());
        assert_eq!(res.snapshot.rendered_prompt, None);
        assert_eq!(
            res.snapshot.budget.prompt_tokens,
            estimate_tokens(raw_prompt)
        );
        assert_eq!(
            res.snapshot.budget.total_tokens,
            estimate_tokens(raw_prompt)
        );
    }

    #[test]
    fn test_deterministic_section_order_and_tie_breaking() {
        let turn_id = TurnId::from_str("trn_test00000000000000002").unwrap();
        let thread_id = ThreadId::from_str("thr_test00000000000000002").unwrap();

        // Documents out of order
        let doc_pref = make_identity_doc(
            "idd_pref00000000000000001",
            IdentityDocumentKind::Preference,
            "Prefers dark mode",
            100,
        );
        let doc_name = make_identity_doc(
            "idd_name00000000000000001",
            IdentityDocumentKind::Name,
            "Ada Lovelace",
            200,
        );
        let doc_about = make_identity_doc(
            "idd_about0000000000000001",
            IdentityDocumentKind::About,
            "Mathematician and writer",
            150,
        );

        // Memory hits out of score order
        let hit_low = make_memory_hit(
            "mem_low000000000000000001",
            MemoryKind::Fact,
            "Born in London",
            2.5,
            300,
        );
        let hit_high = make_memory_hit(
            "mem_high00000000000000001",
            MemoryKind::Fact,
            "Worked on the Analytical Engine",
            8.0,
            400,
        );

        let res = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: "Who am I?",
            identity_docs: &[doc_pref, doc_name, doc_about],
            memory_hits: &[hit_low, hit_high],
            budget: ContextBudgetConfig::default(),
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap();

        // Assert Identity ordering: Name < About < Preference
        assert_eq!(res.snapshot.identity.len(), 3);
        assert_eq!(res.snapshot.identity[0].kind, IdentityDocumentKind::Name);
        assert_eq!(res.snapshot.identity[1].kind, IdentityDocumentKind::About);
        assert_eq!(
            res.snapshot.identity[2].kind,
            IdentityDocumentKind::Preference
        );

        // Assert Memory ordering: High score first
        assert_eq!(res.snapshot.memories.len(), 2);
        assert_eq!(
            res.snapshot.memories[0].memory_id.as_str(),
            "mem_high00000000000000001"
        );
        assert_eq!(
            res.snapshot.memories[1].memory_id.as_str(),
            "mem_low000000000000000001"
        );

        // Assert Wire Prompt section order: Identity header -> Memory header -> User Prompt
        let id_pos = res.wire_prompt.find("# Identity").unwrap();
        let mem_pos = res.wire_prompt.find("# Relevant Memories").unwrap();
        let prompt_pos = res.wire_prompt.find("Who am I?").unwrap();
        assert!(id_pos < mem_pos);
        assert!(mem_pos < prompt_pos);
    }

    #[test]
    fn test_memory_budget_drop() {
        let turn_id = TurnId::from_str("trn_test00000000000000003").unwrap();
        let thread_id = ThreadId::from_str("thr_test00000000000000003").unwrap();

        let hit1 = make_memory_hit(
            "mem_hit000000000000000001",
            MemoryKind::Fact,
            "This is a relatively long memory sentence for budgeting.",
            10.0,
            100,
        );
        let hit2 = make_memory_hit(
            "mem_hit000000000000000002",
            MemoryKind::Fact,
            "Second memory entry which should exceed a tight budget.",
            5.0,
            200,
        );

        // Budget tight enough for header + 1 hit only
        let line1 = format!("- [fact]: {}\n", hit1.record.content.as_str());
        let exact_budget = estimate_tokens(MEMORY_HEADER) + estimate_tokens(&line1);

        let res = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: "Query",
            identity_docs: &[],
            memory_hits: &[hit1, hit2],
            budget: ContextBudgetConfig {
                identity_limit_tokens: 1024,
                memory_limit_tokens: exact_budget,
            },
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap();

        assert_eq!(res.snapshot.memories.len(), 1);
        assert_eq!(
            res.snapshot.memories[0].memory_id.as_str(),
            "mem_hit000000000000000001"
        );
        assert_eq!(res.snapshot.dropped.len(), 1);
        assert_eq!(
            res.snapshot.dropped[0].memory_id.as_str(),
            "mem_hit000000000000000002"
        );
        assert_eq!(
            res.snapshot.dropped[0].reason,
            ContextDropReason::BudgetExhausted
        );
        assert_eq!(res.snapshot.dropped[0].rank, 2);
    }

    #[test]
    fn test_secret_shape_fail_closed() {
        let turn_id = TurnId::from_str("trn_test00000000000000004").unwrap();
        let thread_id = ThreadId::from_str("thr_test00000000000000004").unwrap();
        let secret_prompt = "My secret key is sk-ant-api03-12345678901234567890";

        let err = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: secret_prompt,
            identity_docs: &[],
            memory_hits: &[],
            budget: ContextBudgetConfig::default(),
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap_err();

        assert_eq!(err, AssemblerError::SecretShapedInput("user_prompt"));
        // Ensure error display does not leak candidate text
        let msg = err.to_string();
        assert!(!msg.contains("sk-ant-api03"));
    }

    #[test]
    fn test_determinism_across_multiple_runs() {
        let turn_id = TurnId::from_str("trn_test00000000000000005").unwrap();
        let thread_id = ThreadId::from_str("thr_test00000000000000005").unwrap();

        let doc = make_identity_doc(
            "idd_doc000000000000000001",
            IdentityDocumentKind::Instruction,
            "Always follow PEP8",
            100,
        );
        let hit = make_memory_hit(
            "mem_hit000000000000000005",
            MemoryKind::Preference,
            "Prefers 4 spaces",
            3.2,
            120,
        );

        let run1 = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: "Indent code",
            identity_docs: std::slice::from_ref(&doc),
            memory_hits: std::slice::from_ref(&hit),
            budget: ContextBudgetConfig::default(),
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap();

        let run2 = assemble_context(AssembleParams {
            turn_id: &turn_id,
            thread_id: &thread_id,
            memory_mode: "long_term",
            user_prompt: "Indent code",
            identity_docs: &[doc],
            memory_hits: &[hit],
            budget: ContextBudgetConfig::default(),
            now: UnixMillis::from_millis(1_700_000_000_000),
            degraded: None,
        })
        .unwrap();

        assert_eq!(run1.wire_prompt, run2.wire_prompt);
        assert_eq!(run1.snapshot, run2.snapshot);
    }
}
