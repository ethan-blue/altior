//! A05 acceptance: bounded timeline history with real content (ADR 0020).
//!
//! Reproduces review finding F07 first — the pre-A05 `TurnDto`-only
//! history cannot render content — and then proves the journal-backed
//! entry page over a real daemon round-trip: prompts, assistant deltas,
//! permissions and decisions, turn states, pagination toward the past,
//! high-water catch-up, and bounded degradation for unprojectable rows.
//! All content is synthetic; Chinese text exercises the byte-boundary
//! paths. Deterministic: no sleeps, in-memory duplex transport only.

use std::str::FromStr;

use altior_core::{
    CoreApplication, CoreDaemon, InMemoryListener,
    runtime::{AcpHarnessAdapter, StoreCheckpointAdapter},
};
use altior_domain::{
    DomainEvent, DomainEventKind, EventId, EventPayload, OperationId, ThreadId, TurnId, UnixMillis,
};
use altior_ipc::LaunchCredentials;
use altior_protocol::{
    BoundedPayload, CapabilitySet, CommandEnvelope, CommandKind, DesktopHello, GetHistoryCommand,
    HistoryEntryDto, LaunchToken, ProductVersion, ProtocolVersion, ProtocolVersionRange,
};
use altior_storage::Store;

const BASE_MILLIS: u64 = 1_700_000_000_000;

fn thread_id(body: &str) -> ThreadId {
    assert!(
        body.len() >= 16,
        "thread id body must satisfy the domain minimum"
    );
    ThreadId::from_str(&format!("thr_{body}")).expect("valid thread id")
}

fn turn_id(body: &str) -> TurnId {
    TurnId::from_str(&format!("trn_{body}")).expect("valid turn id")
}

fn event_id(n: u64) -> EventId {
    EventId::from_str(&format!("evt_history{n:0>16}")).expect("valid event id")
}

fn journal(store: &mut Store, event: &DomainEvent) {
    store.append_domain_event(event).expect("append event");
}

fn entry_seq(entry: &HistoryEntryDto) -> u64 {
    match entry {
        HistoryEntryDto::UserMessage { seq, .. }
        | HistoryEntryDto::AssistantDelta { seq, .. }
        | HistoryEntryDto::Permission { seq, .. }
        | HistoryEntryDto::PermissionDecision { seq, .. }
        | HistoryEntryDto::TurnState { seq, .. }
        | HistoryEntryDto::Unknown { seq, .. } => *seq,
    }
}

/// Writes a synthetic turn: prompt → deltas → permission → decision →
/// completed, plus one forward-compatible row, all through the journal.
#[allow(clippy::too_many_lines)]
fn seed_rich_turn(store: &mut Store, thr: &ThreadId, trn: &TurnId) {
    let now = |offset: u64| UnixMillis::from_millis(BASE_MILLIS + offset);

    journal(
        store,
        &DomainEvent {
            event_id: event_id(1),
            thread_id: Some(thr.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::ThreadCreated,
            payload: EventPayload::try_from(
                r#"{"agent_profile_id":"agp_history000000001","title":"中文历史会话"}"#.as_bytes(),
            )
            .unwrap(),
            occurred_at: now(0),
        },
    );
    journal(
        store,
        &DomainEvent {
            event_id: event_id(2),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::TurnStarted,
            payload: EventPayload::try_from(
                format!(r#"{{"thread_id":"{thr}","turn_id":"{trn}","content":"请帮我总结这份文档的三个要点"}}"#)
                    .as_bytes(),
            )
            .unwrap(),
            occurred_at: now(1),
        },
    );
    journal(
        store,
        &DomainEvent {
            event_id: event_id(3),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::MessageDelta,
            payload: EventPayload::try_from(
                format!(r#"{{"thread_id":"{thr}","turn_id":"{trn}","text":"这份文档的核心是"}}"#)
                    .as_bytes(),
            )
            .unwrap(),
            occurred_at: now(2),
        },
    );
    journal(
        store,
        &DomainEvent {
            event_id: event_id(4),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::MessageDelta,
            payload: EventPayload::try_from(
                format!(r#"{{"thread_id":"{thr}","turn_id":"{trn}","text":"三层本地优先架构。"}}"#)
                    .as_bytes(),
            )
            .unwrap(),
            occurred_at: now(3),
        },
    );
    journal(
        store,
        &DomainEvent {
            event_id: event_id(6),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionRequested,
            payload: EventPayload::try_from(
                format!(
                    r#"{{"thread_id":"{thr}","turn_id":"{trn}","permission_id":"evt_history0000000000000006","permission_kind":"execute","description":"cargo test --workspace"}}"#
                )
                .as_bytes(),
            )
            .unwrap(),
            occurred_at: now(4),
        },
    );
    // The decision journal row reuses the permission's event id per the
    // domain validator (one decision per permission event).
    journal(
        store,
        &DomainEvent {
            event_id: event_id(7),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::PermissionDecided,
            payload: EventPayload::try_from(
                format!(
                    r#"{{"thread_id":"{thr}","turn_id":"{trn}","permission_event_id":"evt_history0000000000000006","decision":"approved"}}"#
                )
                .as_bytes(),
            )
            .unwrap(),
            occurred_at: now(5),
        },
    );
    journal(
        store,
        &DomainEvent {
            event_id: event_id(8),
            thread_id: Some(thr.clone()),
            turn_id: Some(trn.clone()),
            operation_id: None,
            kind: DomainEventKind::TurnCompleted,
            payload: EventPayload::try_from(
                format!(r#"{{"thread_id":"{thr}","turn_id":"{trn}","content":""}}"#).as_bytes(),
            )
            .unwrap(),
            occurred_at: now(6),
        },
    );
    // A forward-compatible row no current version projects: must arrive as
    // a bounded Unknown, not break the page.
    journal(
        store,
        &DomainEvent {
            event_id: event_id(9),
            thread_id: Some(thr.clone()),
            turn_id: None,
            operation_id: None,
            kind: DomainEventKind::Other("usage.stats.snapshot".to_owned()),
            payload: EventPayload::try_from(br#"{"thread_id":"x","input_tokens":1024}"#.as_slice())
                .unwrap(),
            occurred_at: now(7),
        },
    );
}

/// A deterministic daemon over the in-memory duplex transport with a
/// seeded on-disk store.
struct HistoryDaemon {
    daemon: CoreDaemon<AcpHarnessAdapter, StoreCheckpointAdapter, InMemoryListener>,
    client: altior_core::InMemoryClient,
}

impl HistoryDaemon {
    fn new(store: Store) -> Self {
        let credentials = LaunchCredentials {
            instance_id: "cor_history0000000001".parse().unwrap(),
            launch_token: LaunchToken::from_str("0f1e2d3c4b5a69788796a5b4c3d2e1f0").unwrap(),
        };
        let app = CoreApplication::new(
            AcpHarnessAdapter::new(),
            StoreCheckpointAdapter::new(store),
            credentials,
        );
        let listener = InMemoryListener::new("history-test");
        let client = listener.create_client();
        let daemon = CoreDaemon::new(app, listener);
        Self { daemon, client }
    }

    fn step(&mut self) {
        let report = self
            .daemon
            .step(UnixMillis::from_millis(BASE_MILLIS))
            .expect("daemon step");
        if report.accepted_connections > 0
            || report.normal_commands_dispatched > 0
            || report.closed_connections > 0
        {
            println!(
                "step: accepted={} closed={} normal={} control={} events={}",
                report.accepted_connections,
                report.closed_connections,
                report.normal_commands_dispatched,
                report.control_commands_dispatched,
                report.events_published
            );
        }
    }

    fn handshake(&mut self) {
        let hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .unwrap(),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
            launch_token: LaunchToken::from_str("0f1e2d3c4b5a69788796a5b4c3d2e1f0").unwrap(),
        };
        self.client.send_json(&hello).expect("send hello");
        // Accept + handshake may span steps; drain until the hello frame
        // arrives, then read the greeting.
        for _ in 0..10 {
            self.step();
            if let Ok(Some(_core_hello)) = self.client.recv_json::<altior_protocol::CoreHello>() {
                break;
            }
        }
        for _ in 0..10 {
            match self.client.recv_json::<altior_protocol::CoreGreeting>() {
                Ok(Some(greeting)) => {
                    let _ = greeting;
                    break;
                }
                Ok(None) => self.step(),
                Err(err) => panic!("greeting read failed: {err}"),
            }
        }
    }

    fn command_snapshot(
        &mut self,
        envelope: &CommandEnvelope,
    ) -> altior_protocol::SnapshotEnvelope {
        self.client.send_json(envelope).expect("send command");
        // get_history answers with a SnapshotEnvelope frame (unlike result
        // commands, which answer with an event): drain raw frames and
        // decode leniently.
        for _ in 0..32 {
            self.step();
            if let Ok(Some(bytes)) = self.client.recv_frame() {
                let snap: altior_protocol::SnapshotEnvelope =
                    serde_json::from_slice(&bytes).expect("decode snapshot envelope");
                assert_eq!(
                    snap.operation_id, envelope.operation_id,
                    "snapshot must answer the request operation"
                );
                return snap;
            }
        }
        panic!("no snapshot answer within 32 daemon steps");
    }

    fn get_history(
        &mut self,
        thr: &ThreadId,
        before_seq: Option<u64>,
        limit: u32,
    ) -> altior_protocol::ThreadHistoryResponseDto {
        let envelope = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: OperationId::from_str("op_history0000000000001").unwrap(),
            kind: CommandKind::GetHistory,
            payload: Some(
                BoundedPayload::new(
                    serde_json::to_value(GetHistoryCommand {
                        thread_id: thr.clone(),
                        cursor: None,
                        limit: Some(limit),
                        before_seq,
                    })
                    .expect("serialize payload"),
                    64 * 1024,
                )
                .expect("bounded payload"),
            ),
            issued_at: UnixMillis::from_millis(BASE_MILLIS),
        };
        let snap = self.command_snapshot(&envelope);
        serde_json::from_value(snap.data.value().clone()).expect("decode history response")
    }
}

fn seeded_daemon(thr: &ThreadId, trn: &TurnId) -> HistoryDaemon {
    let store = Store::open_in_memory().expect("open in-memory store");
    let mut store = store;
    seed_rich_turn(&mut store, thr, trn);
    HistoryDaemon::new(store)
}

#[test]
fn history_page_returns_journal_content_not_turn_placeholders() {
    let thr = thread_id("history0000000001");
    let trn = turn_id("history0000000001");
    let mut daemon = seeded_daemon(&thr, &trn);
    daemon.handshake();

    let response = daemon.get_history(&thr, None, 50);

    // The turn summary still works…
    assert_eq!(response.turns.len(), 1);
    assert_eq!(response.turns[0].id, trn);
    // …but the entries carry the real content (F07 fixed).
    assert!(
        response.entries.len() >= 7,
        "expected the full fact set, got {} entries",
        response.entries.len()
    );

    let prompt = response.entries.iter().find_map(|e| match e {
        HistoryEntryDto::UserMessage { text, turn_id, .. } => Some((text, turn_id)),
        _ => None,
    });
    let (prompt_text, prompt_turn) = prompt.expect("user message entry present");
    assert_eq!(prompt_text, "请帮我总结这份文档的三个要点");
    assert_eq!(prompt_turn, &trn);

    let deltas: Vec<&str> = response
        .entries
        .iter()
        .filter_map(|e| match e {
            HistoryEntryDto::AssistantDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec!["这份文档的核心是", "三层本地优先架构。"],
        "deltas must preserve order and exact text"
    );

    let permission = response.entries.iter().find_map(|e| match e {
        HistoryEntryDto::Permission {
            permission_kind,
            description,
            ..
        } => Some((permission_kind.as_str(), description.as_str())),
        _ => None,
    });
    assert_eq!(
        permission,
        Some(("execute", "cargo test --workspace")),
        "permission row must survive history"
    );

    let decision = response.entries.iter().find_map(|e| match e {
        HistoryEntryDto::PermissionDecision {
            permission_event_id,
            decision,
            ..
        } => Some((permission_event_id.as_str(), decision.as_str())),
        _ => None,
    });
    assert_eq!(
        decision,
        Some(("evt_history0000000000000006", "approved")),
        "decision must reference the original permission event"
    );

    let terminal = response.entries.iter().find_map(|e| match e {
        HistoryEntryDto::TurnState { state, .. } => Some(state.as_str()),
        _ => None,
    });
    assert_eq!(terminal, Some("completed"));

    // Forward-compatible row degrades to a bounded Unknown.
    let unknown = response.entries.iter().find_map(|e| match e {
        HistoryEntryDto::Unknown {
            kind, diagnostic, ..
        } if kind == "usage.stats.snapshot" => Some((kind.as_str(), diagnostic.as_str())),
        _ => None,
    });
    let (unknown_kind, unknown_diag) = unknown.expect("unknown entry preserves the row");
    assert_eq!(unknown_kind, "usage.stats.snapshot");
    assert!(unknown_diag.contains("input_tokens"));

    // Journal order is preserved across the whole page.
    let seqs: Vec<u64> = response.entries.iter().map(entry_seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "entries must be oldest-first by journal seq");

    // Live catch-up point is present and sane (8 events total in journal).
    assert_eq!(response.high_water_seq, Some(8));
}

#[test]
fn history_pages_walk_toward_the_past_without_overlap() {
    let thr = thread_id("history0000000002");
    let trn = turn_id("history0000000002");
    let mut daemon = seeded_daemon(&thr, &trn);
    daemon.handshake();

    // Page 1: only the last 3 journal facts.
    let page1 = daemon.get_history(&thr, None, 3);
    assert_eq!(page1.entries.len(), 3);
    assert!(page1.has_more, "older entries must be available");
    let cursor1 = page1.next_seq_cursor.expect("page 1 carries a cursor");

    // Page 2: everything strictly older than the cursor.
    let page2 = daemon.get_history(&thr, Some(cursor1.seq), 50);
    assert!(!page2.entries.is_empty());
    assert!(
        !page2.has_more,
        "no entries older than the journal head remain"
    );

    // No overlap: page 2's seqs are all strictly below the cursor, and the
    // union is contiguous with page 1.
    for seq in page2.entries.iter().map(entry_seq) {
        assert!(seq < cursor1.seq, "page 2 overlaps page 1 at seq {seq}");
    }
    let mut all_seqs: Vec<u64> = page1
        .entries
        .iter()
        .chain(page2.entries.iter())
        .map(entry_seq)
        .collect();
    all_seqs.sort_unstable();
    assert_eq!(
        all_seqs,
        (1..=8).collect::<Vec<u64>>(),
        "union of pages must cover every journal row exactly once"
    );

    // Paging past the head yields an empty page.
    let empty = daemon.get_history(&thr, Some(1), 50);
    assert!(empty.entries.is_empty());
    assert!(!empty.has_more);
}
