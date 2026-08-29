//! Core's operation registry: dedup by `OperationId` (ADR 0006).
//!
//! The P0.2 evidence requires Core restarts to expose recovery state
//! *without duplicate commands*. Desktop refuses re-issues through its
//! command ledger; Core enforces the same rule from the receiving side —
//! a command envelope whose operation was already accepted is acknowledged
//! as a duplicate, never executed twice, whatever the transport redelivered.

use std::collections::{BTreeMap, VecDeque};

use altior_domain::OperationId;
use altior_protocol::{CommandEnvelope, CommandKind};

/// How an incoming command was treated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    /// First time this operation arrived; execute it.
    Execute,
    /// The operation was already admitted; acknowledge without executing.
    Duplicate,
}

/// Typed admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    /// The registry reached its capacity; retire finished operations
    /// before admitting more.
    Full,
}

/// Bounded registry of admitted operations plus a bounded memory of
/// finished ones. Finished ids are never re-executed while remembered.
#[derive(Debug)]
pub struct OperationRegistry {
    admitted: BTreeMap<OperationId, CommandKind>,
    finished: VecDeque<OperationId>,
    capacity: usize,
}

impl OperationRegistry {
    /// Creates a registry admitting at most `capacity` concurrent
    /// operations and remembering the same number of finished ids.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionError::Full`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, AdmissionError> {
        if capacity == 0 {
            return Err(AdmissionError::Full);
        }
        Ok(Self {
            admitted: BTreeMap::new(),
            finished: VecDeque::new(),
            capacity,
        })
    }

    /// Admits a command unless its operation was already admitted or has
    /// finished. Finished operations are never re-executed.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionError::Full`] at capacity.
    pub fn admit(&mut self, command: &CommandEnvelope) -> Result<Admission, AdmissionError> {
        if self.knows(&command.operation_id) {
            return Ok(Admission::Duplicate);
        }
        if self.admitted.len() == self.capacity {
            return Err(AdmissionError::Full);
        }
        self.admitted
            .insert(command.operation_id.clone(), command.kind);
        Ok(Admission::Execute)
    }

    /// Marks an operation finished: it stops occupying capacity but stays
    /// remembered (bounded), so a late redelivery is still a duplicate.
    pub fn retire(&mut self, operation: &OperationId) {
        self.admitted.remove(operation);
        if self.finished.len() == self.capacity {
            self.finished.pop_front();
        }
        self.finished.push_back(operation.clone());
    }

    /// Whether an operation id was admitted or has finished.
    #[must_use]
    pub fn knows(&self, operation: &OperationId) -> bool {
        self.admitted.contains_key(operation) || self.finished.contains(operation)
    }

    /// Number of concurrently admitted operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.admitted.len()
    }

    /// Whether nothing is admitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.admitted.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use altior_domain::UnixMillis;
    use altior_protocol::ProtocolVersion;
    use std::str::FromStr;

    fn command(operation: &str, kind: CommandKind) -> CommandEnvelope {
        CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: OperationId::from_str(operation).unwrap(),
            kind,
            payload: None,
            issued_at: UnixMillis::from_millis(0),
        }
    }

    #[test]
    fn admits_each_operation_exactly_once() {
        let mut registry = OperationRegistry::new(8).unwrap();
        let first = command("op_fixture000000005", CommandKind::Ping);
        assert_eq!(registry.admit(&first).unwrap(), Admission::Execute);
        // A transport-level redelivery with the same operation id is a
        // duplicate, not a second execution.
        let redelivered = command("op_fixture000000005", CommandKind::Ping);
        assert_eq!(registry.admit(&redelivered).unwrap(), Admission::Duplicate);
        // A different operation proceeds.
        let second = command("op_fixture000000006", CommandKind::Ping);
        assert_eq!(registry.admit(&second).unwrap(), Admission::Execute);
    }

    #[test]
    fn retirement_frees_capacity_but_never_reexecutes() {
        let mut registry = OperationRegistry::new(1).unwrap();
        let first = command("op_fixture000000005", CommandKind::Ping);
        registry.admit(&first).unwrap();
        let second = command("op_fixture000000006", CommandKind::Ping);
        assert_eq!(registry.admit(&second), Err(AdmissionError::Full));

        registry.retire(&first.operation_id);
        assert!(registry.admit(&second).is_ok());
        // Even after retirement, the finished operation is never re-run.
        assert!(registry.knows(&first.operation_id));
        assert_eq!(registry.admit(&first).unwrap(), Admission::Duplicate);
    }

    #[test]
    fn finished_memory_is_bounded() {
        let mut registry = OperationRegistry::new(2).unwrap();
        for number in 5..=8u32 {
            let operation = OperationId::from_str(&format!("op_fixture{number:09}")).unwrap();
            registry
                .admit(&command(operation.as_str(), CommandKind::Ping))
                .unwrap();
            registry.retire(&operation);
        }
        // Only the last two finished ids are remembered; the oldest
        // fell out of the bounded memory.
        let oldest = OperationId::from_str("op_fixture000000005").unwrap();
        assert!(!registry.knows(&oldest));
        let newest = OperationId::from_str("op_fixture000000008").unwrap();
        assert!(registry.knows(&newest));
    }
}
