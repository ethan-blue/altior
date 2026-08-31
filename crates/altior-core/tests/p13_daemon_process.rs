//! P1.3 Real Core daemon process end-to-end integration tests.
//!
//! Validates `altior-core` spawned as a true operating system process over
//! real local OS transport (Windows Named Pipes on Windows, Unix Domain Sockets on Unix):
//! 1. Spawns `CARGO_BIN_EXE_altior-core` with isolated `--data-dir`, `--endpoint`, and `--discovery`.
//! 2. Bounded non-blocking wait for atomic discovery publication with zero fixed `thread::sleep`.
//! 3. Discovery file verification and assertion of debug token redaction.
//! 4. Client-first bad token connection rejection with zero information leak.
//! 5. Client A connection, negotiation handshake, and Ping `CommandEnvelope` execution.
//! 6. Client A disconnection and Client B connection with same token verifying daemon survival.
//! 7. Daemon process termination via RAII guard, verification of endpoint teardown and stale discovery cleanup.
//! 8. Repeatable execution under 10 seconds with clean process hygiene.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use altior_domain::{OperationId, UnixMillis};
use altior_ipc::{
    ClientSession, EndpointDiscovery, GreetingOutcome, IpcError, LocalEndpoint, LocalStream,
    cleanup_stale_discovery, mint_launch_token, read_discovery_file,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandKind, CoreGreeting, CoreHello, DesktopHello, EventBody,
    EventEnvelope, KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange,
};

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_endpoint_arg(temp_dir: &Path) -> (String, LocalEndpoint) {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("altior-daemon-p13-{}-{}", std::process::id(), id);
    if cfg!(windows) {
        let ep = LocalEndpoint::windows_pipe(&name).expect("valid windows pipe endpoint");
        (name, ep)
    } else {
        let sock_path = temp_dir.join(format!("{name}.sock"));
        let sock_str = sock_path.to_str().expect("valid utf8 path").to_string();
        let ep = LocalEndpoint::unix_socket(&sock_str).expect("valid unix socket endpoint");
        (sock_str, ep)
    }
}

#[derive(Debug)]
struct CoreProcessGuard {
    child: Option<std::process::Child>,
}

impl CoreProcessGuard {
    fn spawn(
        bin_path: &str,
        data_dir: &Path,
        endpoint_arg: &str,
        discovery_path: &Path,
    ) -> Result<Self, std::io::Error> {
        let child = std::process::Command::new(bin_path)
            .arg("--daemon")
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--endpoint")
            .arg(endpoint_arg)
            .arg("--discovery")
            .arg(discovery_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        Ok(Self { child: Some(child) })
    }

    fn kill_and_wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            child.wait()
        } else {
            Err(std::io::Error::other("process already terminated"))
        }
    }
}

impl Drop for CoreProcessGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for_discovery(
    guard: &mut CoreProcessGuard,
    path: &Path,
    timeout: Duration,
) -> Result<EndpointDiscovery, IpcError> {
    let start = Instant::now();
    loop {
        if path.exists()
            && let Ok(discovery) = read_discovery_file(path)
        {
            return Ok(discovery);
        }
        if let Some(ref mut child) = guard.child
            && let Ok(Some(status)) = child.try_wait()
        {
            return Err(IpcError::Io {
                source: std::io::Error::other(format!(
                    "Core daemon process exited prematurely with {status}"
                )),
            });
        }
        if start.elapsed() >= timeout {
            return Err(IpcError::Timeout {
                endpoint: format!("discovery file publication at {}", path.display()),
            });
        }
        thread::yield_now();
    }
}

fn recv_frame_timeout(stream: &mut LocalStream, timeout: Duration) -> Result<Vec<u8>, IpcError> {
    let start = Instant::now();
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

#[allow(clippy::too_many_lines)]
fn execute_daemon_process_e2e_cycle(iteration: usize) {
    let bin_path = env!("CARGO_BIN_EXE_altior-core");
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let data_dir = temp_dir.path().join("data");
    let discovery_path = temp_dir.path().join("core-discovery.json");
    let (endpoint_arg, expected_endpoint) = unique_endpoint_arg(temp_dir.path());

    // 1. Spawn altior-core daemon process with RAII guard
    let mut guard = CoreProcessGuard::spawn(bin_path, &data_dir, &endpoint_arg, &discovery_path)
        .expect("spawn altior-core daemon process");

    // 2. Wait for discovery file publication (bounded yield loop, no fixed sleep)
    let discovery = wait_for_discovery(&mut guard, &discovery_path, Duration::from_secs(5))
        .expect("wait for discovery file");

    // 3. Verify discovery metadata and Debug redaction
    assert_eq!(discovery.endpoint, expected_endpoint);
    assert!(!discovery.instance_id.as_str().is_empty());
    assert_eq!(discovery.launch_token.as_str().len(), 32);

    let debug_str = format!("{discovery:?}");
    assert!(
        debug_str.contains("[REDACTED]"),
        "debug representation must contain [REDACTED]: {debug_str}"
    );
    assert!(
        !debug_str.contains(discovery.launch_token.as_str()),
        "debug representation must not leak launch token: {debug_str}"
    );
    assert!(
        debug_str.contains(discovery.instance_id.as_str()),
        "debug representation must contain instance ID: {debug_str}"
    );

    // 4. Bad token connection: client-first no data & rejection without CoreHello
    {
        let mut bad_client =
            LocalStream::connect(&discovery.endpoint, Some(Duration::from_secs(5)))
                .expect("connect bad client");

        // Client-first check: newly connected stream receives zero unsolicited data
        assert!(
            bad_client
                .try_recv_frame()
                .expect("try recv bad client")
                .is_none(),
            "client-first: stream must receive no unsolicited data"
        );

        let bad_token = mint_launch_token(&[0xaa; 16]).expect("mint bad token");
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

        let bad_res = recv_json_timeout::<CoreHello>(&mut bad_client, Duration::from_secs(3));
        assert!(
            bad_res.is_err(),
            "bad token must be rejected without sending CoreHello, got {bad_res:?}"
        );
    }

    let good_hello = DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .expect("valid protocol versions"),
        desktop_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
        launch_token: discovery.launch_token.clone(),
    };

    // 5. Client A connection, handshake, and Ping command execution
    {
        let mut client_a = LocalStream::connect(&discovery.endpoint, Some(Duration::from_secs(5)))
            .expect("connect client A");

        assert!(
            client_a
                .try_recv_frame()
                .expect("try recv client A")
                .is_none(),
            "client-first: stream must receive no unsolicited data"
        );

        client_a
            .send_json(&good_hello)
            .expect("send hello client A");

        let core_hello_a: CoreHello = recv_json_timeout(&mut client_a, Duration::from_secs(3))
            .expect("recv core hello client A");
        let greeting_a: CoreGreeting = recv_json_timeout(&mut client_a, Duration::from_secs(3))
            .expect("recv greeting client A");
        assert_eq!(greeting_a.instance_id, discovery.instance_id);

        let mut session_a = ClientSession::new();
        let negotiated_a = altior_protocol::negotiate(&good_hello, &core_hello_a)
            .expect("negotiate handshake client A");
        let outcome_a = session_a
            .accept_greeting(&greeting_a, &negotiated_a)
            .expect("client A accept greeting");
        assert_eq!(outcome_a, GreetingOutcome::Restarted);

        let now_millis = 1_700_000_000_000_u64.saturating_add((iteration as u64) * 1000);
        let now_a = UnixMillis::from_millis(now_millis);
        let op_ping_a = OperationId::from_str(&format!("op_fixturepinga{iteration:04}00000"))
            .expect("valid op id");
        let ping_cmd_a = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: op_ping_a.clone(),
            kind: CommandKind::Ping,
            payload: None,
            issued_at: now_a,
        };
        client_a.send_json(&ping_cmd_a).expect("send ping client A");

        let event_a: EventEnvelope = recv_json_timeout(&mut client_a, Duration::from_secs(3))
            .expect("recv ping result client A");
        match event_a.body {
            EventBody::Known(KnownEvent::CommandResult {
                operation_id,
                success,
                ..
            }) => {
                assert_eq!(operation_id, op_ping_a);
                assert!(success);
            }
            other => panic!("expected CommandResult for ping A, got {other:?}"),
        }

        // Client A drops here
    }

    // 6. Client B connection with same token: proves daemon survives client drop
    {
        let mut client_b = LocalStream::connect(&discovery.endpoint, Some(Duration::from_secs(5)))
            .expect("connect client B");

        assert!(
            client_b
                .try_recv_frame()
                .expect("try recv client B")
                .is_none(),
            "client-first: stream must receive no unsolicited data"
        );

        client_b
            .send_json(&good_hello)
            .expect("send hello client B");

        let core_hello_b: CoreHello = recv_json_timeout(&mut client_b, Duration::from_secs(3))
            .expect("recv core hello client B");
        let greeting_b: CoreGreeting = recv_json_timeout(&mut client_b, Duration::from_secs(3))
            .expect("recv greeting client B");
        assert_eq!(greeting_b.instance_id, discovery.instance_id);

        let mut session_b = ClientSession::new();
        let negotiated_b = altior_protocol::negotiate(&good_hello, &core_hello_b)
            .expect("negotiate handshake client B");
        let outcome_b = session_b
            .accept_greeting(&greeting_b, &negotiated_b)
            .expect("client B accept greeting");
        assert_eq!(outcome_b, GreetingOutcome::Restarted);

        let now_millis = 1_700_000_000_500_u64.saturating_add((iteration as u64) * 1000);
        let now_b = UnixMillis::from_millis(now_millis);
        let op_ping_b = OperationId::from_str(&format!("op_fixturepingb{iteration:04}00000"))
            .expect("valid op id");
        let ping_cmd_b = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: op_ping_b.clone(),
            kind: CommandKind::Ping,
            payload: None,
            issued_at: now_b,
        };
        client_b.send_json(&ping_cmd_b).expect("send ping client B");

        let event_b: EventEnvelope = recv_json_timeout(&mut client_b, Duration::from_secs(3))
            .expect("recv ping result client B");
        match event_b.body {
            EventBody::Known(KnownEvent::CommandResult {
                operation_id,
                success,
                ..
            }) => {
                assert_eq!(operation_id, op_ping_b);
                assert!(success);
            }
            other => panic!("expected CommandResult for ping B, got {other:?}"),
        }
    }

    // 7. Kill daemon process, verify endpoint closure and stale discovery cleanup
    guard
        .kill_and_wait()
        .expect("kill and wait core daemon process");

    let connect_after = LocalStream::connect(&discovery.endpoint, Some(Duration::from_millis(100)));
    assert!(
        connect_after.is_err(),
        "endpoint must not accept connections after daemon process kill, got {connect_after:?}"
    );

    cleanup_stale_discovery(&discovery_path).expect("cleanup stale discovery file");
    assert!(
        !discovery_path.exists(),
        "discovery file must not exist after cleanup"
    );

    cleanup_stale_discovery(&discovery_path)
        .expect("stale discovery cleanup on missing file is idempotent");
}

#[test]
fn real_daemon_process_e2e_sequential_runs() {
    let start = Instant::now();

    for iteration in 1..=2 {
        execute_daemon_process_e2e_cycle(iteration);
    }

    assert!(
        start.elapsed() < Duration::from_secs(10),
        "two consecutive daemon process E2E runs must finish within 10s, took {:?}",
        start.elapsed()
    );
}
