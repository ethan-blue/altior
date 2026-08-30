//! Typed failures for the SQLite storage layer (ADR 0009).

use std::fmt;

/// Typed failure for a storage operation.
///
/// `#[non_exhaustive]` so new failure kinds can be added without a breaking
/// release.
#[derive(Debug)]
#[non_exhaustive]
pub enum StorageError {
    /// The database schema version exceeds the highest version this build
    /// knows; refuse rather than guess (forward-only migrations).
    SchemaTooNew {
        /// The schema version found in the file.
        found: i64,
        /// The highest schema version this build supports.
        supported: i64,
    },
    /// A migration step failed; the file is left at its previous version.
    MigrationFailed {
        /// The version the failed step was migrating to.
        to_version: i64,
        /// The underlying SQLite failure.
        source: rusqlite::Error,
    },
    /// An envelope payload that exceeds the journal's defense-in-depth cap.
    PayloadTooLarge {
        /// The encoded payload size in bytes.
        size_bytes: usize,
        /// The enforced cap in bytes.
        limit_bytes: usize,
    },
    /// A journal page limit exceeds the public bounded type's cap.
    JournalLimitOutOfRange {
        /// The rejected unsigned value.
        value: u32,
        /// The maximum accepted row count.
        max: u32,
    },
    /// A domain durable row or append input violates the typed event contract.
    InvalidDomainEvent {
        /// The rejected invariant.
        detail: String,
    },
    /// An event id already exists with different durable tuple content.
    EventIdCollision {
        /// The conflicting event id.
        event_id: String,
        /// The sequence of the retained original.
        existing_seq: i64,
    },
    /// The envelope could not be encoded for the journal.
    EncodeFailed {
        /// The underlying protocol failure.
        source: altior_protocol::ProtocolError,
    },
    /// A stored payload that is not valid UTF-8.
    PayloadNotUtf8 {
        /// The journal sequence of the offending row.
        seq: i64,
    },
    /// A stored payload could not be decoded back into an envelope.
    DecodeFailed {
        /// The journal sequence of the offending row.
        seq: i64,
        /// The underlying protocol failure.
        source: altior_protocol::ProtocolError,
    },
    /// An attempt to mutate journal history; the append-only triggers
    /// refused it.
    JournalImmutable {
        /// The SQLite message naming the trigger.
        detail: String,
    },
    /// An invariant check failed during rebuild or reopen.
    RebuildInvariant {
        /// What the projection rebuild observed.
        detail: String,
    },
    /// An agent profile already exists with the given ID.
    AgentProfileAlreadyExists {
        /// The conflicting agent profile ID.
        agent_profile_id: String,
    },
    /// The specified agent profile was not found.
    AgentProfileNotFound {
        /// The missing agent profile ID.
        agent_profile_id: String,
    },
    /// A harness binding already exists with the given ID.
    HarnessBindingAlreadyExists {
        /// The conflicting harness binding ID.
        harness_binding_id: String,
    },
    /// The specified harness binding was not found.
    HarnessBindingNotFound {
        /// The missing harness binding ID.
        harness_binding_id: String,
    },
    /// A project reference already exists with the given ID.
    ProjectRefAlreadyExists {
        /// The conflicting project ID.
        project_id: String,
    },
    /// The specified project reference was not found.
    ProjectRefNotFound {
        /// The missing project ID.
        project_id: String,
    },
    /// A project reference cannot be deleted because it is still referenced by threads.
    ProjectReferencedByThreads {
        /// The project ID in use.
        project_id: String,
        /// How many threads reference this project.
        thread_count: usize,
    },
    /// An entity row contains invalid data that violates domain invariants.
    InvalidEntityData {
        /// The rejected detail.
        detail: String,
    },
    /// A SQLite failure not classified above.
    Sqlite {
        /// The operation context, e.g. `"append_event"`.
        context: &'static str,
        /// The underlying SQLite failure.
        source: rusqlite::Error,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaTooNew { found, supported } => write!(
                f,
                "database schema version {found} is newer than this build supports ({supported}); upgrade Altior instead of opening it"
            ),
            Self::MigrationFailed { to_version, .. } => {
                write!(f, "migration to schema version {to_version} failed")
            }
            Self::PayloadTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "journal payload of {size_bytes} bytes exceeds the {limit_bytes}-byte cap"
            ),
            Self::JournalLimitOutOfRange { value, max } => {
                write!(f, "journal page limit {value} exceeds the maximum {max}")
            }
            Self::InvalidDomainEvent { detail } => write!(f, "invalid domain event: {detail}"),
            Self::EventIdCollision {
                event_id,
                existing_seq,
            } => write!(
                f,
                "event id {event_id} already names different payload at sequence {existing_seq}"
            ),
            Self::EncodeFailed { .. } => f.write_str("envelope encoding failed"),
            Self::PayloadNotUtf8 { seq } => {
                write!(f, "journal row at seq {seq} holds non-UTF-8 payload bytes")
            }
            Self::DecodeFailed { seq, .. } => {
                write!(f, "journal row at seq {seq} holds an undecodable payload")
            }
            Self::JournalImmutable { detail } => {
                write!(f, "the journal is append-only: {detail}")
            }
            Self::RebuildInvariant { detail } => {
                write!(f, "projection rebuild invariant violated: {detail}")
            }
            Self::AgentProfileAlreadyExists { agent_profile_id } => {
                write!(f, "agent profile {agent_profile_id} already exists")
            }
            Self::AgentProfileNotFound { agent_profile_id } => {
                write!(f, "agent profile {agent_profile_id} not found")
            }
            Self::HarnessBindingAlreadyExists { harness_binding_id } => {
                write!(f, "harness binding {harness_binding_id} already exists")
            }
            Self::HarnessBindingNotFound { harness_binding_id } => {
                write!(f, "harness binding {harness_binding_id} not found")
            }
            Self::ProjectRefAlreadyExists { project_id } => {
                write!(f, "project reference {project_id} already exists")
            }
            Self::ProjectRefNotFound { project_id } => {
                write!(f, "project reference {project_id} not found")
            }
            Self::ProjectReferencedByThreads {
                project_id,
                thread_count,
            } => {
                write!(
                    f,
                    "cannot delete project reference {project_id}: referenced by {thread_count} thread(s)"
                )
            }
            Self::InvalidEntityData { detail } => {
                write!(f, "invalid entity data in database: {detail}")
            }
            Self::Sqlite { context, .. } => write!(f, "SQLite failure during {context}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MigrationFailed { source, .. } | Self::Sqlite { source, .. } => Some(source),
            Self::EncodeFailed { source, .. } | Self::DecodeFailed { source, .. } => Some(source),
            Self::PayloadTooLarge { .. }
            | Self::JournalLimitOutOfRange { .. }
            | Self::InvalidDomainEvent { .. }
            | Self::EventIdCollision { .. }
            | Self::SchemaTooNew { .. }
            | Self::PayloadNotUtf8 { .. }
            | Self::JournalImmutable { .. }
            | Self::RebuildInvariant { .. }
            | Self::AgentProfileAlreadyExists { .. }
            | Self::AgentProfileNotFound { .. }
            | Self::HarnessBindingAlreadyExists { .. }
            | Self::HarnessBindingNotFound { .. }
            | Self::ProjectRefAlreadyExists { .. }
            | Self::ProjectRefNotFound { .. }
            | Self::ProjectReferencedByThreads { .. }
            | Self::InvalidEntityData { .. } => None,
        }
    }
}

impl StorageError {
    /// Maps a raw SQLite error, preserving the append-only trigger message
    /// as a typed variant.
    pub(crate) fn from_sqlite(context: &'static str, error: rusqlite::Error) -> Self {
        let detail = error.to_string();
        if detail.contains("journal is append-only")
            || detail.contains("domain journal is append-only")
        {
            Self::JournalImmutable { detail }
        } else {
            Self::Sqlite {
                context,
                source: error,
            }
        }
    }
}
