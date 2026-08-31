//! Application command definitions, envelope framing, and command dispatcher (P1.3).
//!
//! Exposes a typed internal command API with an `operation_id` dedup ledger
//! and adapter seams for mapping external IPC protocol envelopes.

use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, EventId, OperationId, PermissionDecision,
    ProjectId, SearchQuery, ThreadCursor, ThreadId, ThreadListLimit, ThreadState, ThreadTitle,
    TurnId, UnixMillis,
};
use altior_storage::ThreadRow;

use crate::application::CoreApplication;
use crate::application::error::CoreAppError;
use crate::application::mod_types::{CoreDiagnosticsReport, CoreStatusReport, ThreadOpenResult};
use crate::operations::Admission;
use crate::runtime::adapters::storage::StoreCheckpointAdapter;
use crate::runtime::ports::HarnessRuntimePort;
use crate::runtime::state::{BindingProbeOutcome, CancelOutcome, TurnAdmission};

/// Internal command discriminators and payloads supported by Core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreCommand {
    /// Transport liveness health check.
    Ping,
    /// Configure or update an agent profile with optional harness binding.
    ConfigureAgent {
        profile: AgentProfile,
        binding: Option<AcpHarnessBinding>,
    },
    /// Probes and tests an agent harness binding without starting a full session.
    TestAgentBinding { binding: AcpHarnessBinding },
    /// Creates a new thread and registers its domain event.
    CreateThread {
        thread_id: ThreadId,
        agent_profile_id: AgentProfileId,
        title: Option<ThreadTitle>,
        project_id: Option<ProjectId>,
    },
    /// Returns a bounded page of threads.
    ListThreads {
        state_filter: Option<ThreadState>,
        before: Option<ThreadCursor>,
        limit: ThreadListLimit,
    },
    /// Searches threads by title using FTS5.
    SearchThreads {
        query: SearchQuery,
        before: Option<ThreadCursor>,
        limit: ThreadListLimit,
    },
    /// Opens a thread and binds or checks runtime session.
    OpenThread {
        thread_id: ThreadId,
        binding: Option<AcpHarnessBinding>,
    },
    /// Initiates a prompt turn on a thread.
    StartPrompt {
        thread_id: ThreadId,
        turn_id: TurnId,
        content: String,
    },
    /// Cancels or steers an active turn.
    Cancel { thread_id: ThreadId },
    /// Submits a user decision for a pending permission request.
    PermissionDecision {
        thread_id: ThreadId,
        permission_id: EventId,
        decision: PermissionDecision,
    },
    /// Returns application status summary.
    Status,
    /// Returns diagnostics report.
    Diagnostics { thread_id: Option<ThreadId> },
}

/// A versioned, identified command envelope carrying a [`CoreCommand`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreCommandEnvelope {
    /// Unique operation ID for idempotency deduplication.
    pub operation_id: OperationId,
    /// Timestamp when sender issued this command.
    pub issued_at: UnixMillis,
    /// The specific command and payload.
    pub command: CoreCommand,
}

impl CoreCommandEnvelope {
    /// Creates a new command envelope.
    #[must_use]
    pub fn new(operation_id: OperationId, issued_at: UnixMillis, command: CoreCommand) -> Self {
        Self {
            operation_id,
            issued_at,
            command,
        }
    }
}

/// Typed responses returned by command execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreCommandResponse {
    /// Ping reply.
    Pong,
    /// Agent profile and binding configured.
    AgentConfigured,
    /// Outcome of agent binding probe.
    AgentBindingTested(BindingProbeOutcome),
    /// Thread created.
    ThreadCreated(ThreadRow),
    /// Page of threads.
    ThreadList(Vec<ThreadRow>),
    /// Search results.
    SearchResults(Vec<ThreadRow>),
    /// Opened thread metadata and turn history.
    ThreadOpened(ThreadOpenResult),
    /// Turn prompt admission.
    PromptStarted(TurnAdmission),
    /// Cancellation outcome.
    Cancelled(CancelOutcome),
    /// Permission decision applied.
    PermissionDecided,
    /// Application status report.
    Status(CoreStatusReport),
    /// Application diagnostics report.
    Diagnostics(CoreDiagnosticsReport),
    /// Duplicate command acknowledged without second execution.
    DuplicateAcknowledged { operation_id: OperationId },
}

/// Command dispatcher coordinating operation idempotency and application handlers.
#[derive(Debug, Default)]
pub struct CommandDispatcher;

impl CommandDispatcher {
    /// Creates a new command dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Dispatches a command envelope through `CoreApplication`, enforcing `operation_id`
    /// idempotency deduplication and returning typed responses or errors.
    ///
    /// # Errors
    ///
    /// Returns [`CoreAppError`] on validation, execution, or persistence failure.
    #[allow(clippy::too_many_lines)]
    pub fn dispatch<H>(
        &self,
        app: &mut CoreApplication<H, StoreCheckpointAdapter>,
        envelope: CoreCommandEnvelope,
    ) -> Result<CoreCommandResponse, CoreAppError>
    where
        H: HarnessRuntimePort,
    {
        let op_id = envelope.operation_id.clone();

        // 1. Idempotency ledger check
        match app.operation_registry_mut().admit_operation(&op_id) {
            Ok(Admission::Duplicate) => {
                return Ok(CoreCommandResponse::DuplicateAcknowledged {
                    operation_id: op_id,
                });
            }
            Ok(Admission::Execute) => {}
            Err(err) => return Err(CoreAppError::Admission(err)),
        }

        // 2. Dispatch command
        let res = match envelope.command {
            CoreCommand::Ping => Ok(CoreCommandResponse::Pong),
            CoreCommand::ConfigureAgent { profile, binding } => {
                app.configure_agent(&profile, binding.as_ref())?;
                Ok(CoreCommandResponse::AgentConfigured)
            }
            CoreCommand::TestAgentBinding { binding } => {
                let outcome = app.test_agent_binding(&binding)?;
                Ok(CoreCommandResponse::AgentBindingTested(outcome))
            }
            CoreCommand::CreateThread {
                thread_id,
                agent_profile_id,
                title,
                project_id,
            } => {
                let row = app.create_thread(
                    thread_id,
                    &agent_profile_id,
                    title.as_ref(),
                    project_id.as_ref(),
                    envelope.issued_at,
                )?;
                Ok(CoreCommandResponse::ThreadCreated(row))
            }
            CoreCommand::ListThreads {
                state_filter,
                before,
                limit,
            } => {
                let rows = app.list_threads(state_filter, before.as_ref(), limit)?;
                Ok(CoreCommandResponse::ThreadList(rows))
            }
            CoreCommand::SearchThreads {
                query,
                before,
                limit,
            } => {
                let rows = app.search_threads(&query, before.as_ref(), limit)?;
                Ok(CoreCommandResponse::SearchResults(rows))
            }
            CoreCommand::OpenThread { thread_id, binding } => {
                let result = app.open_thread(&thread_id, binding.as_ref())?;
                Ok(CoreCommandResponse::ThreadOpened(result))
            }
            CoreCommand::StartPrompt {
                thread_id,
                turn_id,
                content,
            } => {
                let admission = app.start_prompt(
                    op_id.clone(),
                    thread_id,
                    turn_id,
                    &content,
                    envelope.issued_at,
                )?;
                Ok(CoreCommandResponse::PromptStarted(admission))
            }
            CoreCommand::Cancel { thread_id } => {
                let outcome = app.cancel_turn(&op_id, &thread_id, envelope.issued_at)?;
                Ok(CoreCommandResponse::Cancelled(outcome))
            }
            CoreCommand::PermissionDecision {
                thread_id,
                permission_id,
                decision,
            } => {
                app.decide_permission(
                    &op_id,
                    &thread_id,
                    &permission_id,
                    decision,
                    envelope.issued_at,
                )?;
                Ok(CoreCommandResponse::PermissionDecided)
            }
            CoreCommand::Status => {
                let status = app.get_status()?;
                Ok(CoreCommandResponse::Status(status))
            }
            CoreCommand::Diagnostics { thread_id } => {
                let diag = app.get_diagnostics(thread_id.as_ref())?;
                Ok(CoreCommandResponse::Diagnostics(diag))
            }
        };

        // 3. Mark operation finished in ledger if completed (or leave remembered)
        if res.is_ok() {
            app.operation_registry_mut().retire(&op_id);
        }

        res
    }
}
