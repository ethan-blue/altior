//! IPC session state machines: establish, greet, subscribe, catch up.
//!
//! Both sides are pure, event-driven machines — no timers, no threads, no
//! I/O. The later async transport feeds bytes in and ships envelopes out;
//! every decision here is a function of explicit inputs, which is what makes
//! the P0.2 evidence (`docs/IMPLEMENTATION_PLAN.md`) provable in-process:
//!
//! - reconnect to the **same** Core instance replays the missing window
//!   (`stream.replayed`) and duplicate deliveries are dropped by `event_id`;
//! - a **restarted** Core greets with a different `CoreInstanceId`, so the
//!   client discards its sequence expectations, clears its duplicate filter,
//!   and re-derives state from a snapshot;
//! - the client's command ledger refuses to re-issue an `OperationId`
//!   across a reconnect, so recovery never duplicates commands (ADR 0006).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use altior_domain::{CoreInstanceId, EventId, OperationId, ThreadId, TurnId, UnixMillis};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandKind, CoreGreeting, CoreHello, DesktopHello, EventBody,
    EventEnvelope, KnownEvent, NegotiatedHandshake, ProtocolVersion, ProtocolVersionRange,
    RetainedWindow, Sequence, negotiate,
};

use crate::auth::{LaunchCredentials, authenticate};
use crate::error::IpcError;

/// Default in-memory retention: how many past envelopes Core keeps for
/// catch-up replay before older events must come from a snapshot.
pub const DEFAULT_RETAINED_CAPACITY: usize = 1024;

/// A fully identified new event awaiting a sequence assignment.
#[derive(Clone, Debug)]
pub struct NewEvent {
    /// Caller-assigned event identity (Core infra generates these; the
    /// domain never does, ADR 0004).
    pub event_id: EventId,
    /// Emission time supplied by the caller.
    pub occurred_at: UnixMillis,
    /// The event body.
    pub body: EventBody,
    /// Optional scoping to an operation.
    pub operation_id: Option<OperationId>,
    /// Optional scoping to a thread.
    pub thread_id: Option<ThreadId>,
    /// Optional scoping to a turn.
    pub turn_id: Option<TurnId>,
}

/// Core's bounded in-memory event buffer with monotonic sequence
/// assignment. Retention is a ring: once `capacity` envelopes are held, the
/// oldest are evicted and catch-up for those sequences becomes a gap.
#[derive(Debug)]
pub struct EventLog {
    events: VecDeque<EventEnvelope>,
    capacity: usize,
    next_sequence: Sequence,
}

impl EventLog {
    /// Creates an empty log with the given retention capacity.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, IpcError> {
        if capacity == 0 {
            return Err(IpcError::SessionOrder {
                attempted: "create an event log with zero capacity",
            });
        }
        Ok(Self {
            events: VecDeque::new(),
            capacity,
            next_sequence: Sequence::FIRST,
        })
    }

    /// Assigns the next sequence to `event`, stores it, and returns the
    /// sealed envelope. Evicts the oldest retained envelope at capacity.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::Protocol`] on sequence overflow.
    ///
    /// # Panics
    ///
    /// Panics when the just-pushed envelope cannot be read back —
    /// impossible by construction.
    pub fn append(&mut self, mut event: EventEnvelope) -> Result<EventEnvelope, IpcError> {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next().map_err(IpcError::from)?;
        event.sequence = sequence;
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
        Ok(self
            .events
            .back()
            .cloned()
            .expect("just pushed an envelope"))
    }

    /// The retained window, or `None` when nothing has been emitted yet.
    ///
    /// # Panics
    ///
    /// Panics when the buffer holds a front but no back envelope —
    /// impossible by construction.
    #[must_use]
    pub fn retained(&self) -> Option<RetainedWindow> {
        let from = self.events.front()?.sequence;
        let through = self
            .events
            .back()
            .expect("front exists implies back exists")
            .sequence;
        Some(RetainedWindow { from, through })
    }

    /// Plans the catch-up for a subscriber whose last seen sequence is
    /// `since` (`None` means "from now").
    ///
    /// - [`ReplayPlan::Replay`] — every retained event after `since`, in
    ///   order; the caller follows with a `stream.replayed` boundary.
    /// - [`ReplayPlan::Gap`] — the range after `since` was evicted; the
    ///   caller emits `stream.gap` and the client must request a snapshot.
    /// - [`ReplayPlan::UpToDate`] — the subscriber misses nothing; delivery
    ///   continues live with no boundary event.
    ///
    /// # Panics
    ///
    /// Panics when `since` cannot advance by one — impossible because it is
    /// strictly below the retained `through` at that point.
    #[must_use]
    pub fn replay_after(&self, since: Option<Sequence>) -> ReplayPlan {
        let Some(since) = since else {
            return ReplayPlan::UpToDate;
        };
        let Some(retained) = self.retained() else {
            // This instance has emitted nothing, so a client holding one of
            // its sequences is impossible; go live.
            return ReplayPlan::UpToDate;
        };
        if since.as_u64() >= retained.through.as_u64() {
            return ReplayPlan::UpToDate;
        }
        let first_missing = since.next().expect("retained through exceeds since");
        if first_missing.as_u64() < retained.from.as_u64() {
            return ReplayPlan::Gap { first_missing };
        }
        let events = self
            .events
            .iter()
            .filter(|event| event.sequence.as_u64() > since.as_u64())
            .cloned()
            .collect();
        ReplayPlan::Replay { events }
    }
}

/// The catch-up decision for one subscription.
#[derive(Debug)]
pub enum ReplayPlan {
    /// Replay these envelopes in order, then emit a `stream.replayed`
    /// boundary.
    Replay {
        /// The retained events after the subscriber's `since`.
        events: Vec<EventEnvelope>,
    },
    /// The requested range was evicted; emit `stream.gap` and let the client
    /// snapshot.
    Gap {
        /// The first sequence the subscriber is missing.
        first_missing: Sequence,
    },
    /// The subscriber misses nothing; continue live without a boundary.
    UpToDate,
}

/// What `accept_subscribe` tells the transport to deliver, in order.
#[derive(Debug)]
pub enum CatchUpDelivery {
    /// Replay `events`, then the `stream.replayed` boundary.
    Replay {
        /// Retained events after the subscriber's catch-up point.
        events: Vec<EventEnvelope>,
        /// The `stream.replayed` boundary envelope (already sequenced).
        boundary: EventEnvelope,
    },
    /// Deliver this `stream.gap` envelope; the client must snapshot.
    Gap {
        /// The `stream.gap` boundary envelope (already sequenced).
        boundary: EventEnvelope,
    },
    /// Nothing to deliver; the next published event is live.
    UpToDate,
}

/// The successful result of Core accepting a Desktop hello.
#[derive(Clone, Debug)]
pub struct SessionEstablished {
    /// The `CoreHello` reply to send.
    pub core_hello: CoreHello,
    /// The negotiated handshake result.
    pub negotiated: NegotiatedHandshake,
    /// The greeting to send next (instance id + retained window).
    pub greeting: CoreGreeting,
}

/// Core's per-connection session: authenticate, negotiate, greet, serve
/// subscriptions, and sequence everything through a shared [`EventLog`].
///
/// One Core launch owns exactly one log; every connection (initial attach,
/// reload, second window) gets its own session over that same log, which is
/// what makes reload transparent: the log keeps sequencing while
/// connections come and go.
#[derive(Debug)]
pub struct ServerSession {
    credentials: LaunchCredentials,
    supported: ProtocolVersionRange,
    core_version: altior_protocol::ProductVersion,
    capabilities: CapabilitySet,
    selected_version: Option<ProtocolVersion>,
    subscribed: bool,
    log: Arc<Mutex<EventLog>>,
}

impl ServerSession {
    /// Creates the server side of one connection with a private log.
    ///
    /// # Panics
    ///
    /// Panics when a log of at least one slot cannot be built — impossible
    /// because the capacity is floored at one.
    #[must_use]
    pub fn new(
        credentials: LaunchCredentials,
        supported: ProtocolVersionRange,
        core_version: altior_protocol::ProductVersion,
        capabilities: CapabilitySet,
        retained_capacity: usize,
    ) -> Self {
        let log = EventLog::new(retained_capacity.max(1)).expect("capacity is at least 1");
        Self::with_log(
            credentials,
            supported,
            core_version,
            capabilities,
            Arc::new(Mutex::new(log)),
        )
    }

    /// Creates the server side of one connection over a log shared with
    /// other connections of the same Core launch.
    #[must_use]
    pub fn with_log(
        credentials: LaunchCredentials,
        supported: ProtocolVersionRange,
        core_version: altior_protocol::ProductVersion,
        capabilities: CapabilitySet,
        log: Arc<Mutex<EventLog>>,
    ) -> Self {
        Self {
            credentials,
            supported,
            core_version,
            capabilities,
            selected_version: None,
            subscribed: false,
            log,
        }
    }

    /// Authenticates the hello, negotiates versions, and produces the reply
    /// plus greeting. Rejects wrong tokens before revealing any version
    /// information.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::AuthenticationRejected`] on a token mismatch,
    /// [`IpcError::Protocol`] when the version ranges have no intersection,
    /// and [`IpcError::SessionOrder`] when called twice.
    ///
    /// # Panics
    ///
    /// Panics when the shared event-log mutex is poisoned (another
    /// connection panicked while holding the log).
    pub fn accept_hello(&mut self, hello: &DesktopHello) -> Result<SessionEstablished, IpcError> {
        if self.selected_version.is_some() {
            return Err(IpcError::SessionOrder {
                attempted: "accept a second hello on one connection",
            });
        }
        authenticate(&hello.launch_token, &self.credentials.launch_token)?;
        let core_hello = CoreHello {
            supported_versions: self.supported,
            core_version: self.core_version,
            capabilities: self.capabilities.clone(),
        };
        let negotiated = negotiate(hello, &core_hello).map_err(IpcError::from)?;
        self.selected_version = Some(negotiated.selected_version);
        let retained = self.log.lock().expect("event log mutex").retained();
        let greeting = CoreGreeting {
            protocol_version: negotiated.selected_version,
            instance_id: self.credentials.instance_id.clone(),
            core_version: self.core_version,
            retained,
        };
        Ok(SessionEstablished {
            core_hello,
            negotiated,
            greeting,
        })
    }

    /// Handles a `subscribe` command and returns exactly what to deliver.
    /// The boundary event is sequenced through the same log, so it occupies
    /// the position right after any replayed events.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] before a handshake or after a
    /// second subscribe, and [`IpcError::Protocol`] for a malformed
    /// subscribe payload.
    ///
    /// # Panics
    ///
    /// Panics when the shared event-log mutex is poisoned or a replay plan
    /// carries no events — neither can occur in a healthy process.
    pub fn accept_subscribe(
        &mut self,
        command: &CommandEnvelope,
        boundary_event_id: EventId,
        occurred_at: UnixMillis,
    ) -> Result<CatchUpDelivery, IpcError> {
        if self.selected_version.is_none() {
            return Err(IpcError::SessionOrder {
                attempted: "subscribe before the handshake completed",
            });
        }
        if self.subscribed {
            return Err(IpcError::SessionOrder {
                attempted: "subscribe twice on one connection",
            });
        }
        let Some(since) = command.subscribe_since().map_err(IpcError::from)? else {
            return Err(IpcError::Protocol {
                source: altior_protocol::ProtocolError::UnsupportedCommandKind {
                    kind: command.kind.wire_name().to_owned(),
                },
            });
        };
        self.subscribed = true;
        let plan = self
            .log
            .lock()
            .expect("event log mutex")
            .replay_after(since);
        match plan {
            ReplayPlan::Replay { events } => {
                let from = events
                    .first()
                    .expect("replay plan carries at least one event")
                    .sequence;
                let through = events
                    .last()
                    .expect("replay plan carries at least one event")
                    .sequence;
                let boundary = self.publish(NewEvent {
                    event_id: boundary_event_id,
                    occurred_at,
                    body: EventBody::Known(KnownEvent::StreamReplayed { from, through }),
                    operation_id: Some(command.operation_id.clone()),
                    thread_id: None,
                    turn_id: None,
                })?;
                Ok(CatchUpDelivery::Replay { events, boundary })
            }
            ReplayPlan::Gap { first_missing } => {
                let boundary = self.publish(NewEvent {
                    event_id: boundary_event_id,
                    occurred_at,
                    body: EventBody::Known(KnownEvent::StreamGap {
                        from: first_missing,
                    }),
                    operation_id: Some(command.operation_id.clone()),
                    thread_id: None,
                    turn_id: None,
                })?;
                Ok(CatchUpDelivery::Gap { boundary })
            }
            ReplayPlan::UpToDate => Ok(CatchUpDelivery::UpToDate),
        }
    }

    /// Sequences one new event through the log and returns the sealed
    /// envelope for live delivery.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] before a handshake and
    /// [`IpcError::Protocol`] on sequence overflow.
    ///
    /// # Panics
    ///
    /// Panics when the shared event-log mutex is poisoned.
    pub fn publish(&mut self, event: NewEvent) -> Result<EventEnvelope, IpcError> {
        let Some(protocol_version) = self.selected_version else {
            return Err(IpcError::SessionOrder {
                attempted: "publish before the handshake completed",
            });
        };
        let envelope = EventEnvelope {
            protocol_version,
            event_id: event.event_id,
            operation_id: event.operation_id,
            thread_id: event.thread_id,
            turn_id: event.turn_id,
            sequence: Sequence::FIRST, // replaced by the log's assignment
            occurred_at: event.occurred_at,
            body: event.body,
        };
        self.log.lock().expect("event log mutex").append(envelope)
    }
}

/// What a greeting means for the client's accumulated stream state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GreetingOutcome {
    /// Same Core launch: sequences and the duplicate filter stay valid.
    Resumed,
    /// Core restarted: discard sequence expectations, clear the duplicate
    /// filter, and re-derive state from a snapshot.
    Restarted,
}

/// How one delivered event was treated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventDelivery {
    /// A new event; the stream advanced to its sequence.
    Applied {
        /// The event's sequence.
        sequence: Sequence,
    },
    /// An event already applied (same `event_id`); dropped idempotently.
    Duplicate,
}

/// Desktop's per-connection client state: epoch tracking, duplicate
/// filtering, and the catch-up point for resubscribes.
#[derive(Debug, Default)]
pub struct ClientSession {
    instance: Option<CoreInstanceId>,
    last_sequence: Option<Sequence>,
    seen_event_ids: BTreeSet<EventId>,
}

impl ClientSession {
    /// Creates a fresh, disconnected client session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes a validated greeting and classifies the epoch change.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::Protocol`] when the greeting fails validation
    /// and [`IpcError::SessionOrder`] before any greeting arrived twice.
    pub fn accept_greeting(
        &mut self,
        greeting: &CoreGreeting,
        negotiated: &NegotiatedHandshake,
    ) -> Result<GreetingOutcome, IpcError> {
        greeting.validate().map_err(IpcError::from)?;
        if greeting.protocol_version != negotiated.selected_version {
            return Err(IpcError::Protocol {
                source: altior_protocol::ProtocolError::UnsupportedProtocolVersion {
                    requested: greeting.protocol_version.as_u32(),
                    supported: altior_protocol::SUPPORTED_PROTOCOL_VERSIONS,
                },
            });
        }
        let outcome = if self.instance.as_ref() == Some(&greeting.instance_id) {
            GreetingOutcome::Resumed
        } else {
            self.last_sequence = None;
            self.seen_event_ids.clear();
            GreetingOutcome::Restarted
        };
        self.instance = Some(greeting.instance_id.clone());
        Ok(outcome)
    }

    /// The catch-up point for the next subscribe: the last seen sequence on
    /// a resumed session, or "from now" after a restart.
    #[must_use]
    pub fn subscribe_since(&self) -> Option<Sequence> {
        self.last_sequence
    }

    /// Consumes one delivered event, deduplicating by `event_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] before any greeting was accepted.
    pub fn accept_event(&mut self, event: &EventEnvelope) -> Result<EventDelivery, IpcError> {
        if self.instance.is_none() {
            return Err(IpcError::SessionOrder {
                attempted: "accept an event before the greeting arrived",
            });
        }
        if !self.seen_event_ids.insert(event.event_id.clone()) {
            return Ok(EventDelivery::Duplicate);
        }
        let sequence = event.sequence;
        if self
            .last_sequence
            .is_none_or(|last| sequence.as_u64() > last.as_u64())
        {
            self.last_sequence = Some(sequence);
        }
        Ok(EventDelivery::Applied { sequence })
    }
}

/// Whether a command was newly recorded or is a duplicate issue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    /// First issue of this `OperationId`.
    Recorded,
    /// This `OperationId` was already issued; do not send it again.
    AlreadyIssued,
}

/// Desktop's ledger of issued commands, preventing duplicate sends across
/// reconnects and restarts (ADR 0006: recovery without duplicate commands).
#[derive(Debug)]
pub struct CommandLedger {
    recorded: BTreeMap<OperationId, CommandKind>,
    capacity: usize,
}

impl CommandLedger {
    /// Creates a ledger holding at most `capacity` operation ids.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, IpcError> {
        if capacity == 0 {
            return Err(IpcError::SessionOrder {
                attempted: "create a command ledger with zero capacity",
            });
        }
        Ok(Self {
            recorded: BTreeMap::new(),
            capacity,
        })
    }

    /// Records a command unless its operation was already issued.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::SessionOrder`] when the ledger is full; clear it
    /// when the owning session is discarded.
    pub fn record(&mut self, command: &CommandEnvelope) -> Result<RecordOutcome, IpcError> {
        if self.recorded.contains_key(&command.operation_id) {
            return Ok(RecordOutcome::AlreadyIssued);
        }
        if self.recorded.len() == self.capacity {
            return Err(IpcError::SessionOrder {
                attempted: "record a command into a full ledger",
            });
        }
        self.recorded
            .insert(command.operation_id.clone(), command.kind);
        Ok(RecordOutcome::Recorded)
    }

    /// Drops all recorded operations (the session they belonged to is gone).
    pub fn clear(&mut self) {
        self.recorded.clear();
    }
}
