//! Core-owned entity identifier allocation (ADR 0019).
//!
//! `altior-domain` validates identifier strings but never generates them
//! (ADR 0004); generation is Core infrastructure. Bodies are 16 lowercase
//! hex characters: 12 hex digits of unix-millis followed by 4 hex digits of
//! per-process noise (a wrapping allocation counter mixed with the process
//! id), so creations in the same millisecond — or by a Core restarted
//! within the same millisecond — never collide. No wall-clock ordering is
//! implied by identifier sort order.

use std::sync::atomic::{AtomicU64, Ordering};

use altior_domain::{
    AgentProfileId, EventId, HarnessBindingId, IdParseError, ThreadId, TurnId, UnixMillis,
};

/// Per-process noise mixed into every allocation so a restarted process
/// minting in the same millisecond still produces distinct identifiers.
fn process_noise() -> u64 {
    let pid = u64::from(std::process::id());
    pid.rotate_left(17) ^ pid.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Allocates Core-owned entity identifiers within the ADR 0004 string
/// format. The typed parse on allocation is a belt-and-braces check: a
/// formatting bug surfaces as a typed error instead of a malformed stored
/// identity.
#[derive(Debug)]
pub struct EntityIdAllocator {
    counter: AtomicU64,
    noise: u64,
}

impl Default for EntityIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityIdAllocator {
    /// Creates an allocator seeded from the process identity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            noise: process_noise(),
        }
    }

    /// Allocates a body for `prefix` at time `now` and parses it into a
    /// typed identifier.
    fn allocate<T: TryFrom<String, Error = IdParseError>>(
        &self,
        prefix: &'static str,
        now: UnixMillis,
    ) -> Result<T, IdParseError> {
        let millis = now.as_millis() & 0xFFFF_FFFF_FFFF;
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        let noise = counter.wrapping_add(self.noise) & 0xFFFF;
        T::try_from(format!("{prefix}{millis:012x}{noise:04x}"))
    }

    /// Mints a thread identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError`] if the formatted body fails domain
    /// validation (a formatting bug surfaced as a typed error).
    pub fn new_thread_id(&self, now: UnixMillis) -> Result<ThreadId, IdParseError> {
        self.allocate("thr_", now)
    }

    /// Mints a turn identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError`] if the formatted body fails domain
    /// validation.
    pub fn new_turn_id(&self, now: UnixMillis) -> Result<TurnId, IdParseError> {
        self.allocate("trn_", now)
    }

    /// Mints an agent profile identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError`] if the formatted body fails domain
    /// validation.
    pub fn new_agent_profile_id(&self, now: UnixMillis) -> Result<AgentProfileId, IdParseError> {
        self.allocate("agp_", now)
    }

    /// Mints a harness binding identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError`] if the formatted body fails domain
    /// validation.
    pub fn new_harness_binding_id(
        &self,
        now: UnixMillis,
    ) -> Result<HarnessBindingId, IdParseError> {
        self.allocate("hsb_", now)
    }

    /// Mints an event identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError`] if the formatted body fails domain
    /// validation.
    pub fn new_event_id(&self, now: UnixMillis) -> Result<EventId, IdParseError> {
        self.allocate("evt_", now)
    }

    /// Mints a domain-valid [`EventId`] deterministically from opaque
    /// correlation bytes (for example an ACP RPC id). The correlation
    /// material is association metadata only: it is hashed into a valid
    /// body and never pasted into the identifier string, so UUID /
    /// uppercase / hyphen forms cannot collapse onto a shared fallback
    /// (ADR 0019).
    #[must_use]
    pub fn event_id_from_correlation(namespace: &str, correlation: &str) -> EventId {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        namespace.hash(&mut hasher);
        correlation.hash(&mut hasher);
        let h1 = hasher.finish();
        hasher.write_u64(0x9e37_79b9_7f4a_7c15);
        let h2 = hasher.finish();
        EventId::try_from(format!("evt_{h1:016x}{h2:016x}"))
            .expect("correlation event id must be domain-valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const NOW: UnixMillis = UnixMillis::from_millis(1_700_000_000_123);

    #[test]
    fn allocates_domain_valid_identifiers_per_kind() {
        let allocator = EntityIdAllocator::new();
        let thread = allocator.new_thread_id(NOW).unwrap();
        let turn = allocator.new_turn_id(NOW).unwrap();
        let profile = allocator.new_agent_profile_id(NOW).unwrap();
        let binding = allocator.new_harness_binding_id(NOW).unwrap();
        let event = allocator.new_event_id(NOW).unwrap();

        assert!(thread.as_str().starts_with("thr_"));
        assert!(turn.as_str().starts_with("trn_"));
        assert!(profile.as_str().starts_with("agp_"));
        assert!(binding.as_str().starts_with("hsb_"));
        assert!(event.as_str().starts_with("evt_"));
        for id in [
            thread.as_str(),
            turn.as_str(),
            profile.as_str(),
            binding.as_str(),
            event.as_str(),
        ] {
            let body = id.split_once('_').unwrap().1;
            assert_eq!(body.len(), 16, "body {id} must be 16 chars");
            assert!(
                body.chars().all(|c| c.is_ascii_hexdigit()),
                "body {id} must be lowercase hex"
            );
        }
    }

    #[test]
    fn same_millisecond_allocations_never_collide() {
        let allocator = EntityIdAllocator::new();
        let mut seen = BTreeSet::new();
        for _ in 0..10_000 {
            assert!(seen.insert(allocator.new_thread_id(NOW).unwrap()));
        }
        // Burst allocations of another kind at the same timestamp stay
        // distinct from each other as typed identifiers.
        let mut turns = BTreeSet::new();
        for _ in 0..1_000 {
            assert!(turns.insert(allocator.new_turn_id(NOW).unwrap()));
        }
    }

    #[test]
    fn allocations_across_millseconds_stay_distinct() {
        let allocator = EntityIdAllocator::new();
        let earlier = allocator
            .new_agent_profile_id(UnixMillis::from_millis(NOW.as_millis() - 1))
            .unwrap();
        let later = allocator.new_agent_profile_id(NOW).unwrap();
        assert_ne!(earlier.as_str(), later.as_str());
    }

    #[test]
    fn event_id_from_correlation_keeps_rpc_shapes_distinct() {
        let a = EntityIdAllocator::event_id_from_correlation(
            "acp.permission",
            "550E8400-E29B-41D4-A716-446655440000",
        );
        let b = EntityIdAllocator::event_id_from_correlation(
            "acp.permission",
            "550E8400-E29B-41D4-A716-446655440001",
        );
        let c = EntityIdAllocator::event_id_from_correlation("acp.permission", "PERM-REQ-ALPHA");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a.as_str(), "evt_perm000000000000");
        assert_eq!(
            a,
            EntityIdAllocator::event_id_from_correlation(
                "acp.permission",
                "550E8400-E29B-41D4-A716-446655440000",
            )
        );
    }
}
