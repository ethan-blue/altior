//! Device-local identity documents and `ContextSnapshot` contracts (P2.2, ADR 0018).
//!
//! Identity documents are user-authored "about me" statements stored
//! device-locally; they are **not** journaled domain knowledge and never
//! participate in the domain projection digest or rebuild. A
//! [`ContextSnapshot`] is the deterministic, serializable audit record of
//! everything injected into one turn's wire prompt: the token budget, the
//! selected identity documents, the selected memories with their
//! explainability metadata, and any entries dropped by budget exhaustion.
//!
//! All contracts here are pure data: no clocks, no I/O, no wall time.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::IdentityDocumentId;
use crate::time::UnixMillis;
use crate::{EntityError, MemoryId, MemoryKind, MemoryScope, ThreadId, TurnId};

/// Maximum length of one identity document in bytes (4 KiB).
pub const IDENTITY_CONTENT_MAX_BYTES: usize = 4096;

/// Maximum number of identity documents retained per device (P2.2 bound).
pub const IDENTITY_DOCUMENT_COUNT_MAX: usize = 32;

/// Maximum serialized size of one `ContextSnapshot` payload in bytes (64 KiB).
pub const CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

/// Maximum number of context snapshots returned per thread listing.
pub const CONTEXT_SNAPSHOT_LIST_LIMIT_MAX: u32 = 50;

/// A bounded identity document text validated at construction.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IdentityContent(String);

impl IdentityContent {
    /// The byte-cap of identity document content.
    #[must_use]
    pub const fn capacity() -> usize {
        IDENTITY_CONTENT_MAX_BYTES
    }

    /// Returns the content as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for IdentityContent {
    type Error = crate::EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(EntityError::EmptyIdentityContent);
        }
        if trimmed.len() > IDENTITY_CONTENT_MAX_BYTES {
            return Err(EntityError::IdentityContentTooLong {
                length: trimmed.len(),
                max: IDENTITY_CONTENT_MAX_BYTES,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl TryFrom<String> for IdentityContent {
    type Error = crate::EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<IdentityContent> for String {
    fn from(value: IdentityContent) -> Self {
        value.0
    }
}

impl fmt::Display for IdentityContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Semantic classification of an identity document.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum IdentityDocumentKind {
    /// The user's preferred name or handle.
    Name,
    /// Free-form biographical context about the user.
    About,
    /// A durable user preference.
    Preference,
    /// A standing instruction the agent must respect.
    Instruction,
}

impl IdentityDocumentKind {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::About => "about",
            Self::Preference => "preference",
            Self::Instruction => "instruction",
        }
    }

    /// Rendering priority: lower sorts first in assembled context
    /// (name, then about, then instructions, then preferences).
    #[must_use]
    pub const fn render_priority(self) -> u8 {
        match self {
            Self::Name => 0,
            Self::About => 1,
            Self::Instruction => 2,
            Self::Preference => 3,
        }
    }

    /// Parses an identity document kind from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidIdentityDocumentKind`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, crate::EntityError> {
        match s {
            "name" => Ok(Self::Name),
            "about" => Ok(Self::About),
            "preference" => Ok(Self::Preference),
            "instruction" => Ok(Self::Instruction),
            _ => Err(EntityError::InvalidIdentityDocumentKind),
        }
    }
}

impl TryFrom<&str> for IdentityDocumentKind {
    type Error = crate::EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(value)
    }
}

impl TryFrom<String> for IdentityDocumentKind {
    type Error = crate::EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_str(&value)
    }
}

impl std::str::FromStr for IdentityDocumentKind {
    type Err = crate::EntityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl From<IdentityDocumentKind> for String {
    fn from(kind: IdentityDocumentKind) -> Self {
        kind.as_str().to_owned()
    }
}

impl fmt::Display for IdentityDocumentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A device-local identity document (ADR 0018).
///
/// Identity documents are explicit, user-authored statements injected into
/// the wire prompt of every turn. They live only in the device-local SQLite
/// store; they are never journaled and never synchronized (P3 may add a
/// separate opt-in sync family).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentityDocument {
    /// Stable identity of this document.
    pub id: IdentityDocumentId,
    /// Semantic classification.
    pub kind: IdentityDocumentKind,
    /// Bounded content.
    pub content: IdentityContent,
    /// When the document was created.
    pub created_at: UnixMillis,
    /// When the document was last updated.
    pub updated_at: UnixMillis,
}

impl IdentityDocument {
    /// Validates the document invariants (bounded content, updated >= created).
    ///
    /// # Errors
    ///
    /// Returns [`crate::EntityError`] when timestamps move backwards.
    pub fn validate(&self) -> Result<(), crate::EntityError> {
        if self.updated_at < self.created_at {
            return Err(EntityError::InvalidIdentityDocument {
                detail: "updated_at earlier than created_at".to_owned(),
            });
        }
        Ok(())
    }
}

/// Validated page size for identity document listings (1..=32).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityDocumentListLimit(u32);

impl IdentityDocumentListLimit {
    /// The inclusive upper bound.
    pub const MAX: u32 = 32;

    /// The default page size.
    pub const DEFAULT: u32 = 32;

    /// Validates a page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::IdentityDocumentListLimitOutOfRange`] outside 1..=32.
    pub fn try_new(value: u32) -> Result<Self, crate::EntityError> {
        if value == 0 || value > Self::MAX {
            return Err(EntityError::IdentityDocumentListLimitOutOfRange {
                value,
                max: Self::MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for IdentityDocumentListLimit {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

/// Validated page size for context snapshot listings (1..=50).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextSnapshotListLimit(u32);

impl ContextSnapshotListLimit {
    /// Validates a page size.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::ContextSnapshotListLimitOutOfRange`] outside 1..=50.
    pub fn try_new(value: u32) -> Result<Self, crate::EntityError> {
        if value == 0 || value > CONTEXT_SNAPSHOT_LIST_LIMIT_MAX {
            return Err(EntityError::ContextSnapshotListLimitOutOfRange {
                value,
                max: CONTEXT_SNAPSHOT_LIST_LIMIT_MAX,
            });
        }
        Ok(Self(value))
    }

    /// The validated count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

// ── ContextSnapshot contracts ──────────────────────────────────────

/// Why one retrieved memory did not make it into the wire prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ContextDropReason {
    /// The deterministic token budget was exhausted before this entry.
    BudgetExhausted,
}

impl ContextDropReason {
    /// Returns the canonical string identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budget_exhausted",
        }
    }

    /// Parses a drop reason from a string slice.
    ///
    /// # Errors
    ///
    /// Returns [`EntityError::InvalidContextDropReason`] if `s` is not recognized.
    pub fn try_from_str(s: &str) -> Result<Self, crate::EntityError> {
        match s {
            "budget_exhausted" => Ok(Self::BudgetExhausted),
            _ => Err(EntityError::InvalidContextDropReason),
        }
    }
}

impl TryFrom<&str> for ContextDropReason {
    type Error = crate::EntityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(value)
    }
}

impl TryFrom<String> for ContextDropReason {
    type Error = crate::EntityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_str(&value)
    }
}

impl std::str::FromStr for ContextDropReason {
    type Err = crate::EntityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl From<ContextDropReason> for String {
    fn from(reason: ContextDropReason) -> Self {
        reason.as_str().to_owned()
    }
}

impl fmt::Display for ContextDropReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A recorded, observable degradation of context assembly (ADR 0018).
///
/// Degradation is never silent: whenever assembly had to proceed with less
/// than complete information in a way that still yields a safe prompt, the
/// reason is recorded here. Hard failures never degrade; they fail the turn.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextDegradation {
    /// Canonical machine-readable code (e.g. `memory_query_truncated`).
    pub code: String,
    /// Bounded human-readable explanation.
    pub detail: String,
}

/// Token accounting for one assembled turn context.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextTokenBudget {
    /// Maximum identity tokens the policy allows.
    pub identity_limit_tokens: u32,
    /// Maximum memory tokens the policy allows.
    pub memory_limit_tokens: u32,
    /// Estimated tokens of the user's original prompt.
    pub prompt_tokens: u32,
    /// Tokens consumed by the injected identity block (including framing).
    pub identity_tokens: u32,
    /// Tokens consumed by the injected memory block (including framing).
    pub memory_tokens: u32,
    /// Total tokens of the assembled wire prompt.
    pub total_tokens: u32,
}

/// One identity document selected into the wire prompt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextIdentityEntry {
    /// The identity document id.
    pub document_id: IdentityDocumentId,
    /// The document kind.
    pub kind: IdentityDocumentKind,
    /// Estimated tokens of the rendered entry.
    pub tokens: u32,
}

/// One confirmed memory selected into the wire prompt, with explainability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextMemoryEntry {
    /// The memory record id.
    pub memory_id: MemoryId,
    /// Memory kind.
    pub kind: MemoryKind,
    /// Memory scope.
    pub scope: MemoryScope,
    /// Confidence percentage (0..=100).
    pub confidence: u8,
    /// Whether the memory source was explicit.
    pub explicit: bool,
    /// Estimated tokens of the rendered entry.
    pub tokens: u32,
    /// Composite retrieval score.
    pub score: f64,
    /// Human-readable selection rationale from the ranking engine.
    pub why_selected: String,
    /// Provenance thread id, when known.
    pub provenance_thread_id: Option<ThreadId>,
    /// Provenance turn id, when known.
    pub provenance_turn_id: Option<TurnId>,
}

/// One retrieved memory that was dropped from the wire prompt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextDroppedEntry {
    /// The memory record id.
    pub memory_id: MemoryId,
    /// Estimated tokens the entry would have consumed.
    pub tokens: u32,
    /// Rank of the entry at drop time (1 = highest score).
    pub rank: u32,
    /// Why the entry was dropped.
    pub reason: ContextDropReason,
}

/// The deterministic audit record of one turn's context assembly (ADR 0018).
///
/// Device-local only: stored in the `context_snapshot` table for
/// explainability; never journaled, never part of the domain digest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// The turn this snapshot assembled context for.
    pub turn_id: TurnId,
    /// The thread the turn belongs to.
    pub thread_id: ThreadId,
    /// The agent profile's memory mode at assembly time (`off` | `session` | `long_term`).
    pub memory_mode: String,
    /// When the snapshot was assembled (caller-passed clock).
    pub created_at: UnixMillis,
    /// Whether the wire prompt was byte-identical to the user prompt
    /// (no identity documents and no memories were selected).
    pub passthrough: bool,
    /// Token accounting.
    pub budget: ContextTokenBudget,
    /// Identity documents injected, in render order.
    pub identity: Vec<ContextIdentityEntry>,
    /// Memories injected, in rank order.
    pub memories: Vec<ContextMemoryEntry>,
    /// Memories retrieved but dropped, in rank order.
    pub dropped: Vec<ContextDroppedEntry>,
    /// Recorded degradation, if assembly proceeded with reduced information.
    pub degraded: Option<ContextDegradation>,
    /// Optional rendered wire prompt or bounded representation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_prompt: Option<String>,
}

impl ContextSnapshot {
    /// Returns `true` when nothing was injected into the wire prompt.
    #[must_use]
    pub fn is_passthrough(&self) -> bool {
        self.identity.is_empty() && self.memories.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_content_bounds_and_trimming() {
        assert!(IdentityContent::try_from("").is_err());
        assert!(IdentityContent::try_from("   ").is_err());
        assert_eq!(
            IdentityContent::try_from("  Ada Lovelace  ")
                .unwrap()
                .as_str(),
            "Ada Lovelace"
        );
        let long = "x".repeat(IDENTITY_CONTENT_MAX_BYTES + 1);
        assert!(IdentityContent::try_from(long.as_str()).is_err());
    }

    #[test]
    fn identity_document_kind_roundtrip() {
        for kind in [
            IdentityDocumentKind::Name,
            IdentityDocumentKind::About,
            IdentityDocumentKind::Preference,
            IdentityDocumentKind::Instruction,
        ] {
            assert_eq!(
                IdentityDocumentKind::try_from_str(kind.as_str()).unwrap(),
                kind
            );
        }
        assert!(IdentityDocumentKind::try_from_str("bogus").is_err());
        assert!(
            IdentityDocumentKind::Name.render_priority()
                < IdentityDocumentKind::Preference.render_priority()
        );
    }

    #[test]
    fn list_limits_validate_range() {
        assert!(IdentityDocumentListLimit::try_new(0).is_err());
        assert!(IdentityDocumentListLimit::try_new(33).is_err());
        assert_eq!(IdentityDocumentListLimit::default().get(), 32);
        assert!(ContextSnapshotListLimit::try_new(0).is_err());
        assert!(ContextSnapshotListLimit::try_new(51).is_err());
        assert_eq!(ContextSnapshotListLimit::try_new(50).unwrap().get(), 50);
    }

    #[test]
    fn drop_reason_roundtrip() {
        assert_eq!(
            ContextDropReason::try_from_str(ContextDropReason::BudgetExhausted.as_str()).unwrap(),
            ContextDropReason::BudgetExhausted
        );
        assert!(ContextDropReason::try_from_str("nope").is_err());
    }

    #[test]
    fn snapshot_serializes_with_typed_ids() {
        let snapshot = ContextSnapshot {
            turn_id: "trn_fixture000000001".parse().unwrap(),
            thread_id: "thr_fixture000000001".parse().unwrap(),
            memory_mode: "long_term".to_owned(),
            created_at: UnixMillis::from_millis(42),
            passthrough: false,
            budget: ContextTokenBudget {
                identity_limit_tokens: 512,
                memory_limit_tokens: 1024,
                prompt_tokens: 7,
                identity_tokens: 5,
                memory_tokens: 9,
                total_tokens: 21,
            },
            identity: vec![ContextIdentityEntry {
                document_id: "idd_fixture000000001".parse().unwrap(),
                kind: IdentityDocumentKind::Name,
                tokens: 5,
            }],
            memories: vec![ContextMemoryEntry {
                memory_id: "mem_fixture000000001".parse().unwrap(),
                kind: MemoryKind::Fact,
                scope: MemoryScope::Global,
                confidence: 100,
                explicit: true,
                tokens: 9,
                score: 1.25,
                why_selected: "matched terms: [\"zephyr\"]".to_owned(),
                provenance_thread_id: None,
                provenance_turn_id: None,
            }],
            dropped: vec![],
            degraded: None,
            rendered_prompt: None,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"trn_fixture000000001\""));
        assert!(json.contains("\"budget_exhausted") || json.contains("\"mem_fixture000000001\""));
        let back: ContextSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
        assert!(!back.is_passthrough());

        let mut with_prompt = snapshot.clone();
        with_prompt.rendered_prompt = Some("Hello Zephyr".to_owned());
        let prompt_json = serde_json::to_string(&with_prompt).unwrap();
        assert!(prompt_json.contains("\"rendered_prompt\":\"Hello Zephyr\""));
        let back_with_prompt: ContextSnapshot = serde_json::from_str(&prompt_json).unwrap();
        assert_eq!(back_with_prompt, with_prompt);
    }
}
