//! Platform-neutral time values used by domain records and protocol
//! envelopes.
//!
//! Both values are plain data supplied by the caller: infrastructure reads
//! a clock port and fixtures use constants, so no domain or protocol code
//! depends on the system clock and tests stay deterministic. `UnixMillis`
//! answers "when did this happen"; `LogicalTick` answers "what causal
//! order did the emitter see" and is the frozen convention the P0.2
//! subscription/catch-up work consumes.

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch, stored as plain data.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixMillis(u64);

impl UnixMillis {
    /// Constructs an instant from a raw millisecond count.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Returns the raw millisecond count.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

/// A Lamport-style logical tick carried by an emitter's records.
///
/// Ticks start at zero and only ever increase within one emission stream.
/// They never wrap: advancing past the representable maximum is a typed
/// failure, not a restart.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LogicalTick(u64);

impl LogicalTick {
    /// The initial tick of a new emission stream.
    pub const ORIGIN: Self = Self(0);

    /// Constructs a tick from a raw value.
    #[must_use]
    pub const fn from_raw(tick: u64) -> Self {
        Self(tick)
    }

    /// Returns the raw tick value.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns the next tick of this stream without wraparound.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::TickOverflow`] at the representable maximum.
    pub const fn next(self) -> Result<Self, TimeError> {
        if self.0 == u64::MAX {
            Err(TimeError::TickOverflow)
        } else {
            Ok(Self(self.0 + 1))
        }
    }
}

/// Typed failure for time-contract violations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TimeError {
    /// A logical tick advanced past the representable maximum.
    TickOverflow,
}

impl std::fmt::Display for TimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TickOverflow => write!(f, "logical tick overflow"),
        }
    }
}

impl std::error::Error for TimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_serde_as_a_plain_number() {
        let at = UnixMillis::from_millis(1_777_777_777_777);
        let encoded = serde_json::to_value(at).unwrap();
        assert_eq!(encoded, serde_json::json!(1_777_777_777_777i64));
        assert_eq!(serde_json::from_value::<UnixMillis>(encoded).unwrap(), at);
    }

    #[test]
    fn logical_ticks_advance_without_wraparound() {
        assert_eq!(LogicalTick::ORIGIN.as_u64(), 0);
        assert_eq!(LogicalTick::from_raw(41).next().unwrap().as_u64(), 42);
        assert_eq!(
            LogicalTick::from_raw(u64::MAX).next(),
            Err(TimeError::TickOverflow)
        );
    }
}
