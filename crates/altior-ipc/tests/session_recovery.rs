//! Deterministic session-level evidence for P0.2
//! (`docs/IMPLEMENTATION_PLAN.md`): reload must not disturb an active
//! stream, restarts must surface recovery instead of duplicates, and
//! authentication plus version checks must fail explicitly.

use std::str::FromStr;

use altior_domain::{CoreInstanceId, EventId, OperationId, UnixMillis};
use altior_ipc::{
    CatchUpDelivery, ClientSession, CommandLedger, EventDelivery, GreetingOutcome, IpcError,
    LaunchCredentials, RecordOutcome, ServerSession, mint_launch_token,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandEnvelope as Command, CommandKind, CoreGreeting,
    DesktopHello, EventBody, KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange,
    Sequence,
};

const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: CoreInstanceId::from_str("cor_fixture000000009").unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn server(credentials: &LaunchCredentials, retained: usize) -> ServerSession {
    ServerSession::new(
        credentials.clone(),
        ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
        ProductVersion::new(0, 1, 0),
        CapabilitySet::new(),
        retained,
    )
}

fn hello(launch_token: &str, min: u32, max: u32) -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(min).unwrap(),
            ProtocolVersion::try_new(max).unwrap(),
        )
        .unwrap(),
        desktop_version: ProductVersion::new(0, 2, 0),
        capabilities: CapabilitySet::new(),
        launch_token: launch_token.parse().unwrap(),
    }
}

fn ping(operation: &str, issued_at: u64) -> CommandEnvelope {
    Command {
        protocol_version: ProtocolVersion::V1,
        operation_id: OperationId::from_str(operation).unwrap(),
        kind: CommandKind::Ping,
        payload: None,
        issued_at: UnixMillis::from_millis(issued_at),
    }
}

fn subscribe(operation: &str, since: Option<u64>, issued_at: u64) -> CommandEnvelope {
    CommandEnvelope::subscribe(
        since.map(|value| Sequence::try_new(value).unwrap()),
        OperationId::from_str(operation).unwrap(),
        UnixMillis::from_millis(issued_at),
        &altior_protocol::EnvelopeLimits::default(),
    )
    .unwrap()
}

fn turn_started(event_number: u64, number: u64) -> altior_ipc::NewEvent {
    altior_ipc::NewEvent {
        event_id: EventId::from_str(&format!("evt_fixture{number:011}")).unwrap(),
        occurred_at: UnixMillis::from_millis(1_700_000_000_000 + event_number),
        body: EventBody::Known(KnownEvent::TurnStarted),
        operation_id: Some(OperationId::from_str("op_fixture000000005").unwrap()),
        thread_id: None,
        turn_id: None,
    }
}

#[test]
fn authentication_precedes_and_guards_version_negotiation() {
    let credentials = credentials();
    let mut core = server(&credentials, 8);

    // A wrong token is rejected before any version information is revealed.
    let bad_token = hello("11111111111111111111111111111111", 1, 1);
    assert!(matches!(
        core.accept_hello(&bad_token),
        Err(IpcError::AuthenticationRejected)
    ));

    // The right token with a disjoint version range still fails explicitly.
    let disjoint = hello(mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(), 5, 9);
    assert!(matches!(
        core.accept_hello(&disjoint),
        Err(IpcError::Protocol { .. })
    ));

    // The correct token and an overlapping range establish the session.
    let good = hello(mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(), 1, 1);
    let established = core.accept_hello(&good).unwrap();
    assert_eq!(established.negotiated.selected_version.as_u32(), 1);
    assert_eq!(
        established.greeting.instance_id.as_str(),
        "cor_fixture000000009"
    );
    assert!(established.greeting.retained.is_none());

    // A second hello on one connection is a protocol-order violation.
    assert!(matches!(
        core.accept_hello(&good),
        Err(IpcError::SessionOrder { .. })
    ));
}

#[test]
fn catch_up_replays_the_retained_window_then_goes_live() {
    let credentials = credentials();
    // One Core launch, one event log: connections come and go over it,
    // which is exactly what a UI reload is (ADR 0006).
    let log = std::sync::Arc::new(std::sync::Mutex::new(altior_ipc::EventLog::new(8).unwrap()));
    let attach = || {
        ServerSession::with_log(
            credentials.clone(),
            ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
            ProductVersion::new(0, 1, 0),
            CapabilitySet::new(),
            std::sync::Arc::clone(&log),
        )
    };

    // Connection 1: handshake, then three events flow to the client.
    let mut conn1 = attach();
    let established = conn1
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();
    let mut client = ClientSession::new();
    assert_eq!(
        client
            .accept_greeting(&established.greeting, &established.negotiated)
            .unwrap(),
        GreetingOutcome::Restarted
    );
    for number in 1..=3u64 {
        let envelope = conn1.publish(turn_started(number, number)).unwrap();
        assert_eq!(
            client.accept_event(&envelope).unwrap(),
            EventDelivery::Applied {
                sequence: Sequence::try_new(number).unwrap()
            }
        );
    }

    // Reload: a brand-new connection greets with the retained window and
    // subscribes from the client's last seen sequence — nothing to replay.
    let mut conn2 = attach();
    let second = conn2
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();
    assert_eq!(
        second
            .greeting
            .retained
            .map(|window| window.through.as_u64()),
        Some(3)
    );
    assert_eq!(
        client
            .accept_greeting(&second.greeting, &second.negotiated)
            .unwrap(),
        GreetingOutcome::Resumed
    );
    let decision = conn2
        .accept_subscribe(
            &subscribe(
                "op_fixture000000012",
                client.subscribe_since().map(Sequence::as_u64),
                5,
            ),
            EventId::from_str("evt_fixture000000012").unwrap(),
            UnixMillis::from_millis(1_700_000_000_005),
        )
        .unwrap();
    assert!(matches!(decision, CatchUpDelivery::UpToDate));

    // The client disconnects, two more events flow, and its next attach is
    // behind: the missing window replays with a boundary sequenced right
    // after it.
    let fourth = conn1.publish(turn_started(4, 4)).unwrap();
    let fifth = conn1.publish(turn_started(5, 5)).unwrap();
    let mut conn3 = attach();
    conn3
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();
    let decision = conn3
        .accept_subscribe(
            &subscribe("op_fixture000000013", Some(3), 6),
            EventId::from_str("evt_fixture000000013").unwrap(),
            UnixMillis::from_millis(1_700_000_000_006),
        )
        .unwrap();
    let CatchUpDelivery::Replay { events, boundary } = decision else {
        panic!("a behind subscriber expects a replay");
    };
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], fourth);
    assert_eq!(events[1], fifth);
    // Replayed envelopes keep their original sequences and ids.
    assert_eq!(events[0].sequence.as_u64(), 4);
    assert!(matches!(
        boundary.body,
        EventBody::Known(KnownEvent::StreamReplayed { ref from, ref through })
            if from.as_u64() == 4 && through.as_u64() == 5
    ));
    assert_eq!(boundary.sequence.as_u64(), 6);
}

#[test]
fn evicted_windows_report_gaps_instead_of_lying() {
    let credentials = credentials();
    let mut core = server(&credentials, 2); // retains only the last two
    core.accept_hello(&hello(
        mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
        1,
        1,
    ))
    .unwrap();
    for number in 1..=4u64 {
        core.publish(turn_started(number, number)).unwrap();
    }

    // Sequences 1 and 2 were evicted; a client last at 1 is missing 2.
    let decision = core
        .accept_subscribe(
            &subscribe("op_fixture000000012", Some(1), 5),
            EventId::from_str("evt_fixture000000012").unwrap(),
            UnixMillis::from_millis(1_700_000_000_005),
        )
        .unwrap();
    let CatchUpDelivery::Gap { boundary } = decision else {
        panic!("an evicted catch-up range expects a gap");
    };
    assert!(matches!(
        boundary.body,
        EventBody::Known(KnownEvent::StreamGap { ref from }) if from.as_u64() == 2
    ));
    // The gap boundary itself is the next stream position.
    assert_eq!(boundary.sequence.as_u64(), 5);
}

#[test]
fn core_restarts_change_the_epoch_and_require_a_snapshot() {
    let first_credentials = credentials();
    let mut first_core = server(&first_credentials, 8);
    let established = first_core
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();

    let mut client = ClientSession::new();
    client
        .accept_greeting(&established.greeting, &established.negotiated)
        .unwrap();
    let envelope = first_core.publish(turn_started(1, 1)).unwrap();
    client.accept_event(&envelope).unwrap();
    assert_eq!(client.subscribe_since().map(Sequence::as_u64), Some(1));

    // Core restarts: a new launch with a new instance id and token.
    let second_credentials = LaunchCredentials {
        instance_id: CoreInstanceId::from_str("cor_fixture000000020").unwrap(),
        launch_token: mint_launch_token(&[0x99; 16]).unwrap(),
    };
    let mut second_core = server(&second_credentials, 8);
    let second = second_core
        .accept_hello(&hello(second_credentials.launch_token.as_str(), 1, 1))
        .unwrap();

    // The client classifies the greeting as a restart and drops every
    // accumulated sequence expectation.
    assert_eq!(
        client
            .accept_greeting(&second.greeting, &second.negotiated)
            .unwrap(),
        GreetingOutcome::Restarted
    );
    assert_eq!(client.subscribe_since(), None);

    // The re-delivered first event is new state, not a duplicate: the
    // duplicate filter was cleared with the epoch.
    let replayed = second_core.publish(turn_started(2, 1)).unwrap();
    assert_eq!(
        client.accept_event(&replayed).unwrap(),
        EventDelivery::Applied {
            sequence: Sequence::FIRST
        }
    );
}

#[test]
fn duplicate_delivery_is_idempotent_within_one_instance() {
    let credentials = credentials();
    let mut core = server(&credentials, 8);
    let established = core
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();
    let mut client = ClientSession::new();
    client
        .accept_greeting(&established.greeting, &established.negotiated)
        .unwrap();

    let envelope = core.publish(turn_started(1, 1)).unwrap();
    assert_eq!(
        client.accept_event(&envelope).unwrap(),
        EventDelivery::Applied {
            sequence: Sequence::FIRST
        }
    );
    // The same envelope delivered again (replay overlap) is dropped.
    assert_eq!(
        client.accept_event(&envelope).unwrap(),
        EventDelivery::Duplicate
    );
    // And the stream position did not regress.
    assert_eq!(client.subscribe_since().map(Sequence::as_u64), Some(1));
}

#[test]
fn the_command_ledger_prevents_duplicate_commands_across_reconnects() {
    let mut ledger = CommandLedger::new(16).unwrap();
    let first = ping("op_fixture000000005", 1);
    assert_eq!(ledger.record(&first).unwrap(), RecordOutcome::Recorded);
    // A retry after a reconnect (same operation, new issued_at) is refused.
    let retry = ping("op_fixture000000005", 2);
    assert_eq!(ledger.record(&retry).unwrap(), RecordOutcome::AlreadyIssued);
    // A different operation proceeds.
    let second = ping("op_fixture000000006", 3);
    assert_eq!(ledger.record(&second).unwrap(), RecordOutcome::Recorded);
    // The ledger is bounded; a full ledger fails explicitly rather than
    // growing without limit.
    let mut tiny = CommandLedger::new(1).unwrap();
    tiny.record(&first).unwrap();
    assert!(matches!(
        tiny.record(&second),
        Err(IpcError::SessionOrder { .. })
    ));
    tiny.clear();
    tiny.record(&second).unwrap();
}

#[test]
fn greetings_must_match_the_negotiated_version() {
    let credentials = credentials();
    let mut core = server(&credentials, 8);
    let established = core
        .accept_hello(&hello(
            mint_launch_token(&TOKEN_ENTROPY).unwrap().as_str(),
            1,
            1,
        ))
        .unwrap();

    let mut mismatched = established.greeting.clone();
    mismatched.protocol_version = ProtocolVersion::try_new(99).unwrap();
    let mut client = ClientSession::new();
    assert!(matches!(
        client.accept_greeting(&mismatched, &established.negotiated),
        Err(IpcError::Protocol { .. })
    ));

    // A valid greeting still establishes the epoch afterwards.
    let valid: CoreGreeting = established.greeting.clone();
    assert_eq!(
        client
            .accept_greeting(&valid, &established.negotiated)
            .unwrap(),
        GreetingOutcome::Restarted
    );
}
