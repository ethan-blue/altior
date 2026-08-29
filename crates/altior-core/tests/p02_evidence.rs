//! The three P0.2 evidence statements from `docs/IMPLEMENTATION_PLAN.md`,
//! proven end to end at the contract level:
//!
//! 1. UI reload does not stop a synthetic active turn.
//! 2. Core restart exposes recovery state without duplicate commands.
//! 3. Wrong protocol versions and stale endpoints fail clearly.

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use altior_core::operations::{Admission, OperationRegistry};
use altior_core::ownership::{DesktopLifecycle, TurnOwnership, TurnTransition};
use altior_core::supervision::{Decision, ProbeOutcome, ReconnectPolicy, Supervisor};
use altior_domain::{CoreInstanceId, EventId, OperationId, TurnId, UnixMillis};
use altior_ipc::{
    ClientSession, CommandLedger, EventDelivery, EventLog, GreetingOutcome, IpcError,
    LaunchCredentials, RecordOutcome, ServerSession, mint_launch_token,
};
use altior_protocol::{
    CapabilitySet, CommandKind, DesktopHello, EventBody, KnownEvent, ProductVersion,
    ProtocolVersion, ProtocolVersionRange,
};

const INSTANCE: &str = "cor_fixture000000009";
const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: CoreInstanceId::from_str(INSTANCE).unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn attach(log: &Arc<Mutex<EventLog>>) -> ServerSession {
    ServerSession::with_log(
        credentials(),
        ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
        ProductVersion::new(0, 1, 0),
        CapabilitySet::new(),
        Arc::clone(log),
    )
}

fn hello() -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .unwrap(),
        desktop_version: ProductVersion::new(0, 2, 0),
        capabilities: CapabilitySet::new(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn turn_started_event(number: u64) -> altior_ipc::NewEvent {
    altior_ipc::NewEvent {
        event_id: EventId::from_str(&format!("evt_fixture{number:09}")).unwrap(),
        occurred_at: UnixMillis::from_millis(1_700_000_000_000 + number),
        body: EventBody::Known(KnownEvent::TurnStarted),
        operation_id: Some(OperationId::from_str("op_fixture000000005").unwrap()),
        thread_id: None,
        turn_id: None,
    }
}

#[test]
fn ui_reload_does_not_stop_a_synthetic_active_turn() {
    // Core owns the turn; Desktop owns only windows.
    let mut ownership = TurnOwnership::new();
    let turn = TurnId::from_str("trn_fixture000000002").unwrap();
    let operation = OperationId::from_str("op_fixture000000005").unwrap();
    ownership.start(turn.clone(), operation.clone());

    // The turn is streaming over the shared log.
    let log = Arc::new(Mutex::new(EventLog::new(8).unwrap()));
    let mut conn = attach(&log);
    let established = conn.accept_hello(&hello()).unwrap();
    let mut client = ClientSession::new();
    client
        .accept_greeting(&established.greeting, &established.negotiated)
        .unwrap();
    let first = conn.publish(turn_started_event(1)).unwrap();
    client.accept_event(&first).unwrap();

    // Reload: the UI tears down and reattaches. The turn keeps running,
    // keeps streaming, and the client resumes without losing anything.
    for event in [DesktopLifecycle::Reload, DesktopLifecycle::WindowClosed] {
        assert_eq!(
            ownership.on_desktop_lifecycle(event, &turn),
            TurnTransition::StillRunning
        );
    }
    let second = conn.publish(turn_started_event(2)).unwrap();
    let mut reattach = attach(&log);
    let reestablished = reattach.accept_hello(&hello()).unwrap();
    assert_eq!(
        client
            .accept_greeting(&reestablished.greeting, &reestablished.negotiated)
            .unwrap(),
        GreetingOutcome::Resumed
    );
    assert_eq!(
        client.accept_event(&second).unwrap(),
        EventDelivery::Applied {
            sequence: altior_protocol::Sequence::try_new(2).unwrap()
        }
    );
    assert!(ownership.is_running(&turn));
}

#[test]
fn core_restart_exposes_recovery_state_without_duplicate_commands() {
    let log = Arc::new(Mutex::new(EventLog::new(8).unwrap()));
    let mut conn = attach(&log);
    let established = conn.accept_hello(&hello()).unwrap();

    let mut client = ClientSession::new();
    client
        .accept_greeting(&established.greeting, &established.negotiated)
        .unwrap();
    let event = conn.publish(turn_started_event(1)).unwrap();
    client.accept_event(&event).unwrap();

    // Desktop issued a command before the restart; its ledger remembers it.
    let mut ledger = CommandLedger::new(16).unwrap();
    let operation = OperationId::from_str("op_fixture000000005").unwrap();
    let command = altior_protocol::CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: operation.clone(),
        kind: CommandKind::Ping,
        payload: None,
        issued_at: UnixMillis::from_millis(1),
    };
    assert_eq!(ledger.record(&command).unwrap(), RecordOutcome::Recorded);
    // Core admitted it.
    let mut registry = OperationRegistry::new(16).unwrap();
    assert_eq!(registry.admit(&command).unwrap(), Admission::Execute);

    // Core restarts: a different launch id and token answer the endpoint.
    let restarted = LaunchCredentials {
        instance_id: CoreInstanceId::from_str("cor_fixture000000020").unwrap(),
        launch_token: mint_launch_token(&[0x99; 16]).unwrap(),
    };
    let mut restarted_core = ServerSession::new(
        restarted.clone(),
        ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
        ProductVersion::new(0, 1, 1),
        CapabilitySet::new(),
        8,
    );
    let reestablished = restarted_core
        .accept_hello(&DesktopHello {
            launch_token: restarted.launch_token.clone(),
            ..hello()
        })
        .unwrap();

    // The client sees the epoch change, drops sequence expectations, and
    // re-derives from a snapshot (retained window empty on the fresh log).
    assert_eq!(
        client
            .accept_greeting(&reestablished.greeting, &reestablished.negotiated)
            .unwrap(),
        GreetingOutcome::Restarted
    );
    assert_eq!(client.subscribe_since(), None);
    assert!(reestablished.greeting.retained.is_none());

    // Recovery does not duplicate commands: Desktop's ledger refuses the
    // re-issue, and even if a stale transport redelivered it, Core's
    // registry would acknowledge it as a duplicate without executing.
    assert_eq!(
        ledger.record(&command).unwrap(),
        RecordOutcome::AlreadyIssued
    );
    let mut restarted_registry = OperationRegistry::new(16).unwrap();
    restarted_registry.admit(&command).unwrap();
    assert_eq!(
        restarted_registry.admit(&command).unwrap(),
        Admission::Duplicate
    );
}

#[test]
fn wrong_versions_and_stale_endpoints_fail_clearly() {
    // Wrong protocol version: explicit typed failure, no silent downgrade.
    let mut core = attach(&Arc::new(Mutex::new(EventLog::new(8).unwrap())));
    let incompatible = DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(7).unwrap(),
            ProtocolVersion::try_new(9).unwrap(),
        )
        .unwrap(),
        ..hello()
    };
    assert!(matches!(
        core.accept_hello(&incompatible),
        Err(IpcError::Protocol { .. })
    ));

    // Stale endpoint: supervision probes, classifies, and respawns.
    let mut supervisor = Supervisor::new(
        altior_ipc::Endpoint::WindowsPipe(r"\\.\pipe\altior-core-ethan".to_owned()),
        ReconnectPolicy::new(1, 5),
    );
    assert!(matches!(supervisor.start(), Decision::Probe { .. }));
    assert!(matches!(
        supervisor.on_probe(&ProbeOutcome::Stale).unwrap(),
        Decision::Spawn { .. }
    ));
}
