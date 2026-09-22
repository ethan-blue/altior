//! ADR 0006: `request_snapshot` returns a real snapshot envelope.
//!
//! The pre-fix handler lied to clients by wrapping `RuntimeDiagnosticsDto`.
//! These tests prove the command returns the same `SnapshotEnvelope` shapes
//! already used by `list_threads` / `open_thread`, never diagnostics, and
//! that a missing payload stays valid (Desktop currently treats the command
//! as payload-free). Deterministic in-memory duplex only; no sleeps.

use std::str::FromStr;

use altior_core::application::{CoreDaemon, InMemoryClient, InMemoryListener};
use altior_core::runtime::{AcpHarnessAdapter, StoreCheckpointAdapter};
use altior_domain::{AgentProfileId, OperationId, ThreadId, UnixMillis};
use altior_ipc::{LaunchCredentials, mint_launch_token};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, DesktopHello, EnvelopeLimits, EventBody, EventEnvelope,
    KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange, RuntimeDiagnosticsDto,
    SnapshotEnvelope, ThreadDto, ThreadListResponseDto, ThreadSnapshotDto,
};

type TestDaemon = CoreDaemon<AcpHarnessAdapter, StoreCheckpointAdapter, InMemoryListener>;

const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn fixture_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_fixture000000001".parse().unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn desktop_hello(credentials: &LaunchCredentials) -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .unwrap(),
        desktop_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
        launch_token: credentials.launch_token.clone(),
    }
}

fn now() -> UnixMillis {
    UnixMillis::from_millis(1_700_000_000_000)
}

fn limits() -> EnvelopeLimits {
    EnvelopeLimits::default()
}

fn handshake_client(
    daemon: &mut TestDaemon,
    listener: &InMemoryListener,
    credentials: &LaunchCredentials,
) -> InMemoryClient {
    let mut client = listener.create_client();
    client.send_json(&desktop_hello(credentials)).unwrap();
    daemon.step(now()).unwrap();
    let _: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();
    client
}

fn create_thread(
    daemon: &mut TestDaemon,
    client: &mut InMemoryClient,
    title: &str,
    op: &str,
) -> ThreadId {
    let envelope = CommandEnvelope::create_thread(
        AgentProfileId::from_str("agp_fixture000000001").unwrap(),
        Some(title.to_owned()),
        None,
        OperationId::from_str(op).unwrap(),
        now(),
        &limits(),
    )
    .unwrap();
    client.send_json(&envelope).unwrap();
    daemon.step(now()).unwrap();
    let event: EventEnvelope = client.recv_json().unwrap().unwrap();
    match event.body {
        EventBody::Known(KnownEvent::CommandResult { success, data, .. }) => {
            assert!(success, "create_thread must succeed");
            let dto: ThreadDto = serde_json::from_value(data.unwrap().value().clone()).unwrap();
            dto.id
        }
        other => panic!("expected CommandResult, got {other:?}"),
    }
}

fn recv_snapshot(
    daemon: &mut TestDaemon,
    client: &mut InMemoryClient,
    envelope: &CommandEnvelope,
) -> SnapshotEnvelope {
    client.send_json(envelope).unwrap();
    let report = daemon.step(now()).unwrap();
    assert_eq!(report.normal_commands_dispatched, 1);
    let snap: SnapshotEnvelope = client.recv_json().unwrap().unwrap();
    assert_eq!(snap.operation_id, envelope.operation_id);
    snap
}

fn assert_not_diagnostics(snap: &SnapshotEnvelope) {
    let as_diag: Result<RuntimeDiagnosticsDto, _> = snap.parse_data();
    assert!(
        as_diag.is_err(),
        "request_snapshot must not return RuntimeDiagnosticsDto"
    );
}

#[test]
fn request_snapshot_unscoped_returns_thread_list_envelope() {
    let credentials = fixture_credentials();
    let (mut daemon, listener) =
        CoreDaemon::in_memory(AcpHarnessAdapter::new(), credentials.clone()).unwrap();
    let mut client = handshake_client(&mut daemon, &listener, &credentials);
    let thread_id = create_thread(
        &mut daemon,
        &mut client,
        "Snapshot List Thread",
        "op_snapcreate000000001",
    );

    let envelope = CommandEnvelope::request_snapshot(
        None,
        None,
        OperationId::from_str("op_snaplist0000000001").unwrap(),
        now(),
        &limits(),
    )
    .unwrap();
    assert!(
        envelope.payload.is_none(),
        "unscoped request_snapshot stays payload-free"
    );

    let snap = recv_snapshot(&mut daemon, &mut client, &envelope);
    assert!(
        snap.thread_id.is_none(),
        "list snapshot is not thread-scoped"
    );
    assert_not_diagnostics(&snap);

    let list: ThreadListResponseDto = snap.parse_data().expect("payload is ThreadListResponseDto");
    assert!(
        list.threads.iter().any(|summary| {
            summary.thread.id == thread_id && summary.thread.title == "Snapshot List Thread"
        }),
        "thread list snapshot must include the created thread: {list:?}"
    );
}

#[test]
fn request_snapshot_scoped_returns_thread_snapshot_envelope() {
    let credentials = fixture_credentials();
    let (mut daemon, listener) =
        CoreDaemon::in_memory(AcpHarnessAdapter::new(), credentials.clone()).unwrap();
    let mut client = handshake_client(&mut daemon, &listener, &credentials);
    let thread_id = create_thread(
        &mut daemon,
        &mut client,
        "Snapshot Detail Thread",
        "op_snapcreate000000002",
    );

    let envelope = CommandEnvelope::request_snapshot(
        Some(thread_id.clone()),
        Some(50),
        OperationId::from_str("op_snapthread000000001").unwrap(),
        now(),
        &limits(),
    )
    .unwrap();

    let snap = recv_snapshot(&mut daemon, &mut client, &envelope);
    assert_eq!(snap.thread_id.as_ref(), Some(&thread_id));
    assert_not_diagnostics(&snap);

    let detail: ThreadSnapshotDto = snap
        .parse_data()
        .expect("payload is ThreadSnapshotDto, the same envelope open_thread uses");
    assert_eq!(detail.thread.id, thread_id);
    assert_eq!(detail.thread.title, "Snapshot Detail Thread");
    assert!(detail.pending_permissions.is_empty());
}

#[test]
fn request_snapshot_unknown_thread_returns_typed_error() {
    let credentials = fixture_credentials();
    let (mut daemon, listener) =
        CoreDaemon::in_memory(AcpHarnessAdapter::new(), credentials.clone()).unwrap();
    let mut client = handshake_client(&mut daemon, &listener, &credentials);

    let missing: ThreadId = "thr_missing000000001".parse().unwrap();
    let envelope = CommandEnvelope::request_snapshot(
        Some(missing),
        None,
        OperationId::from_str("op_snapmissing00000001").unwrap(),
        now(),
        &limits(),
    )
    .unwrap();
    client.send_json(&envelope).unwrap();
    daemon.step(now()).unwrap();

    let event: EventEnvelope = client.recv_json().unwrap().unwrap();
    match event.body {
        EventBody::Known(KnownEvent::CommandError { code, message, .. }) => {
            assert_eq!(code, "REQUEST_SNAPSHOT_FAILED");
            assert!(
                message.as_str().contains("thread not found"),
                "error must name the missing thread: {message}"
            );
        }
        other => panic!("expected CommandError, got {other:?}"),
    }
}
