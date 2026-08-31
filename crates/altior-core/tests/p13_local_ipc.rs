//! P1.3 Real local IPC transport end-to-end integration tests.
//!
//! Validates `CoreApplication` and `CoreDaemon` over real OS transport
//! (Windows Named Pipes on Windows, Unix Domain Sockets on Unix):
//! 1. Daemon thread with real `LocalListener`.
//! 2. Client-first bad token rejection (connection closed without `CoreHello` / `CoreGreeting` leak).
//! 3. Valid client authentication, `CoreHello` + `CoreGreeting` receipt, and handshake negotiation.
//! 4. Protocol Ping `CommandEnvelope` execution returning `KnownEvent::CommandResult` success.
//! 5. Client disconnection resilience: listener stays alive for subsequent clients and pings.
//! 6. Clean daemon shutdown via `stop_handle`, join thread, and verify no orphaned endpoint.
//! 7. Zero `thread::sleep` in test logic: uses channel barrier and yield step loop.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use altior_core::application::CoreApplication;
use altior_core::application::daemon::CoreDaemon;
use altior_core::runtime::adapters::acp::AcpHarnessAdapter;
use altior_domain::{OperationId, UnixMillis};
use altior_ipc::{
    ClientSession, GreetingOutcome, IpcError, LaunchCredentials, LocalEndpoint, LocalListener,
    LocalStream, mint_launch_token,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandKind, DesktopHello, EventBody, EventEnvelope,
    KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange,
};

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_endpoint() -> LocalEndpoint {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("altior-ipc-test-{}-{}", std::process::id(), id);
    if cfg!(windows) {
        LocalEndpoint::windows_pipe(&name).expect("valid windows pipe endpoint")
    } else {
        LocalEndpoint::unix_socket(&format!("/tmp/altior-test-{name}.sock"))
            .expect("valid unix socket endpoint")
    }
}

const TOKEN_ENTROPY: [u8; 16] = [
    0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8, 0x09,
];

fn fixture_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_fixture000000042".parse().expect("valid instance id"),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).expect("valid launch token"),
    }
}

fn recv_frame_timeout(stream: &mut LocalStream, timeout: Duration) -> Result<Vec<u8>, IpcError> {
    let start = std::time::Instant::now();
    loop {
        match stream.try_recv_frame() {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return Err(IpcError::Timeout {
                        endpoint: "recv_frame timed out".to_string(),
                    });
                }
                thread::yield_now();
            }
            Err(err) => return Err(err),
        }
    }
}

fn recv_json_timeout<T: serde::de::DeserializeOwned>(
    stream: &mut LocalStream,
    timeout: Duration,
) -> Result<T, IpcError> {
    let bytes = recv_frame_timeout(stream, timeout)?;
    let text = String::from_utf8(bytes).map_err(|_| IpcError::FrameNotUtf8)?;
    serde_json::from_str(&text).map_err(|err| IpcError::Protocol {
        source: altior_protocol::ProtocolError::MalformedEnvelope { source: err },
    })
}

#[test]
#[allow(clippy::too_many_lines)]
fn real_transport_daemon_e2e_lifecycle() {
    let endpoint = unique_endpoint();
    let credentials = fixture_credentials();
    let (ready_tx, ready_rx) = channel();

    // 1. Spawn daemon on a background thread with real LocalListener
    let daemon_endpoint = endpoint.clone();
    let daemon_credentials = credentials.clone();

    let daemon_handle = thread::spawn(move || {
        let harness = AcpHarnessAdapter::new();
        let app = CoreApplication::open_in_memory(harness, daemon_credentials)
            .expect("open in-memory application");
        let listener = LocalListener::bind(&daemon_endpoint).expect("bind local listener");
        let mut daemon = CoreDaemon::new(app, listener);
        let stop_handle = daemon.stop_handle();

        // Signal barrier: daemon listener is bound and ready for client connections
        ready_tx
            .send(stop_handle.clone())
            .expect("send ready signal");

        // Non-blocking step loop with thread yield (no thread::sleep)
        while stop_handle.load(Ordering::SeqCst) {
            let now_millis = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(u64::MAX);
            let now = UnixMillis::from_millis(now_millis);
            let _ = daemon.step(now);
            thread::yield_now();
        }

        let _ = daemon.shutdown();
    });

    let stop_handle: Arc<AtomicBool> = ready_rx.recv().expect("daemon startup ready barrier");

    // 2. Bad token client-first rejection
    {
        let mut bad_client = LocalStream::connect(&endpoint, Some(Duration::from_secs(5)))
            .expect("connect bad client");

        // Client-first check: newly connected client receives nothing before sending DesktopHello
        assert!(bad_client.try_recv_frame().expect("try recv").is_none());

        let bad_token = mint_launch_token(&[0xee; 16]).expect("mint bad token");
        let bad_hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .expect("valid protocol versions"),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
            launch_token: bad_token,
        };
        bad_client.send_json(&bad_hello).expect("send bad hello");

        // Core closes connection on authentication failure without sending CoreHello or CoreGreeting
        let bad_res = recv_json_timeout::<altior_protocol::CoreHello>(
            &mut bad_client,
            Duration::from_secs(3),
        );
        assert!(
            bad_res.is_err(),
            "expected error or closed connection on bad token, got {bad_res:?}"
        );
    }

    let good_hello = DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .expect("valid protocol versions"),
        desktop_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
        launch_token: credentials.launch_token.clone(),
    };

    // 3. Valid client connection, handshake and negotiation
    {
        let mut client1 = LocalStream::connect(&endpoint, Some(Duration::from_secs(5)))
            .expect("connect valid client 1");
        client1.send_json(&good_hello).expect("send good hello 1");

        let core_hello1: altior_protocol::CoreHello =
            recv_json_timeout(&mut client1, Duration::from_secs(3)).expect("recv core hello 1");
        let greeting1: altior_protocol::CoreGreeting =
            recv_json_timeout(&mut client1, Duration::from_secs(3)).expect("recv core greeting 1");
        assert_eq!(greeting1.instance_id, credentials.instance_id);

        let mut client_session1 = ClientSession::new();
        let negotiated1 =
            altior_protocol::negotiate(&good_hello, &core_hello1).expect("negotiate handshake 1");
        let outcome1 = client_session1
            .accept_greeting(&greeting1, &negotiated1)
            .expect("client1 accept greeting");
        assert_eq!(outcome1, GreetingOutcome::Restarted);

        // 4. Send Ping CommandEnvelope and verify KnownEvent::CommandResult success
        let now = UnixMillis::from_millis(1_700_000_000_000);
        let op_ping1 = OperationId::from_str("op_fixture000000001").expect("valid op id");
        let ping1 = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: op_ping1.clone(),
            kind: CommandKind::Ping,
            payload: None,
            issued_at: now,
        };
        client1.send_json(&ping1).expect("send ping 1");

        let event1: EventEnvelope =
            recv_json_timeout(&mut client1, Duration::from_secs(3)).expect("recv ping result 1");
        match event1.body {
            EventBody::Known(KnownEvent::CommandResult {
                operation_id,
                success,
                ..
            }) => {
                assert_eq!(operation_id, op_ping1);
                assert!(success);
            }
            other => panic!("expected CommandResult for ping 1, got {other:?}"),
        }

        // 5. Drop client 1
    }

    // 5 (cont). Second client connects and proves listener survived client 1 drop
    {
        let mut client2 = LocalStream::connect(&endpoint, Some(Duration::from_secs(5)))
            .expect("connect valid client 2");
        client2.send_json(&good_hello).expect("send good hello 2");

        let core_hello2: altior_protocol::CoreHello =
            recv_json_timeout(&mut client2, Duration::from_secs(3)).expect("recv core hello 2");
        let greeting2: altior_protocol::CoreGreeting =
            recv_json_timeout(&mut client2, Duration::from_secs(3)).expect("recv core greeting 2");
        assert_eq!(greeting2.instance_id, credentials.instance_id);

        let mut client_session2 = ClientSession::new();
        let negotiated2 =
            altior_protocol::negotiate(&good_hello, &core_hello2).expect("negotiate handshake 2");
        let outcome2 = client_session2
            .accept_greeting(&greeting2, &negotiated2)
            .expect("client2 accept greeting");
        assert_eq!(outcome2, GreetingOutcome::Restarted);

        let now = UnixMillis::from_millis(1_700_000_000_100);
        let op_ping2 = OperationId::from_str("op_fixture000000002").expect("valid op id");
        let ping2 = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: op_ping2.clone(),
            kind: CommandKind::Ping,
            payload: None,
            issued_at: now,
        };
        client2.send_json(&ping2).expect("send ping 2");

        let event2: EventEnvelope =
            recv_json_timeout(&mut client2, Duration::from_secs(3)).expect("recv ping result 2");
        match event2.body {
            EventBody::Known(KnownEvent::CommandResult {
                operation_id,
                success,
                ..
            }) => {
                assert_eq!(operation_id, op_ping2);
                assert!(success);
            }
            other => panic!("expected CommandResult for ping 2, got {other:?}"),
        }
    }

    // 6. Stop daemon, wake if needed, join thread, verify clean teardown
    stop_handle.store(false, Ordering::SeqCst);
    let _wake = LocalStream::connect(&endpoint, Some(Duration::from_millis(200)));
    daemon_handle.join().expect("daemon thread join");

    #[cfg(unix)]
    {
        if let LocalEndpoint::UnixSocket(ref path) = endpoint {
            assert!(
                !std::path::Path::new(path).exists(),
                "unix socket file must not remain on disk after daemon drop: {path}"
            );
        }
    }

    let connect_after_shutdown = LocalStream::connect(&endpoint, Some(Duration::from_millis(100)));
    assert!(
        connect_after_shutdown.is_err(),
        "expected connection failure on shutdown endpoint, got {connect_after_shutdown:?}"
    );
}
