//! Real OS transport and discovery integration tests (ADR 0006).
//!
//! Tests real bind/connect/framing roundtrips, bounded frames, authentication
//! handshakes over transport, client disconnect and reconnect against a persistent
//! listener, concurrent clients, typed errors, and discovery lifecycle without
//! any thread sleep.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use altior_domain::{CoreInstanceId, OperationId, UnixMillis};
use altior_ipc::{
    ClientSession, Endpoint, EndpointDiscovery, GreetingOutcome, IpcError, LaunchCredentials,
    LocalListener, LocalStream, MAX_FRAME_BYTES, ServerSession, cleanup_stale_discovery,
    mint_launch_token, read_discovery_file, write_discovery_file,
};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandKind, DesktopHello, ProductVersion, ProtocolVersion,
    ProtocolVersionRange,
};

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_endpoint() -> Endpoint {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("test-pipe-{}-{}", std::process::id(), id);
    if cfg!(windows) {
        Endpoint::windows_pipe(&name).unwrap()
    } else {
        Endpoint::unix_socket(&format!("/tmp/altior-test-{name}.sock")).unwrap()
    }
}

fn test_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_fixture000000009".parse().unwrap(),
        launch_token: mint_launch_token(&[0x42; 16]).unwrap(),
    }
}

#[test]
fn real_bind_connect_and_frame_roundtrip() {
    let endpoint = unique_endpoint();
    let (ready_tx, ready_rx) = channel();

    let server_endpoint = endpoint.clone();
    let server_handle = thread::spawn(move || {
        let listener = LocalListener::bind(&server_endpoint).unwrap();
        ready_tx.send(()).unwrap();

        let mut stream = listener.accept().unwrap();
        let frame = stream.recv_frame_string().unwrap();
        assert_eq!(frame, r#"{"ping":"hello"}"#);

        stream.send_frame(r#"{"pong":"world"}"#).unwrap();
    });

    ready_rx.recv().unwrap();

    let mut client = LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();
    client.send_frame(r#"{"ping":"hello"}"#).unwrap();
    let reply = client.recv_frame_string().unwrap();
    assert_eq!(reply, r#"{"pong":"world"}"#);

    server_handle.join().unwrap();
}

#[test]
fn oversize_frame_rejection_at_client_and_stream() {
    let endpoint = unique_endpoint();
    let (ready_tx, ready_rx) = channel();

    let server_endpoint = endpoint.clone();
    let server_handle = thread::spawn(move || {
        let listener = LocalListener::bind(&server_endpoint).unwrap();
        ready_tx.send(()).unwrap();

        let mut stream = listener.accept().unwrap();
        // Server receives oversized header without reading the payload body
        let err = stream.recv_frame();
        assert!(matches!(
            err,
            Err(IpcError::FrameTooLarge {
                size_bytes,
                limit_bytes: MAX_FRAME_BYTES
            }) if size_bytes == MAX_FRAME_BYTES + 10
        ));
    });

    ready_rx.recv().unwrap();

    let mut client = LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();

    // 1. Client encode-time rejection for string exceeding cap
    let big_payload = "x".repeat(MAX_FRAME_BYTES + 1);
    let send_err = client.send_frame(&big_payload);
    assert!(matches!(
        send_err,
        Err(IpcError::FrameTooLarge {
            size_bytes,
            limit_bytes: MAX_FRAME_BYTES
        }) if size_bytes == MAX_FRAME_BYTES + 1
    ));

    // 2. Client sends raw header with declared length > MAX_FRAME_BYTES
    let mut raw_oversized_header = u32::try_from(MAX_FRAME_BYTES + 10)
        .unwrap()
        .to_be_bytes()
        .to_vec();
    raw_oversized_header.extend_from_slice(b"header-only-data");
    client.send_raw_frame(&raw_oversized_header).unwrap();

    server_handle.join().unwrap();
}

#[test]
fn handshake_and_bad_token_rejection_over_real_stream() {
    let endpoint = unique_endpoint();
    let creds = test_credentials();
    let (ready_tx, ready_rx) = channel();

    let server_endpoint = endpoint.clone();
    let server_creds = creds.clone();
    let server_handle = thread::spawn(move || {
        let listener = LocalListener::bind(&server_endpoint).unwrap();
        ready_tx.send(()).unwrap();

        // 1. First client connection: sends bad token -> rejected
        {
            let mut stream = listener.accept().unwrap();
            let mut session = ServerSession::new(
                server_creds.clone(),
                ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
                ProductVersion::new(0, 1, 0),
                CapabilitySet::new(),
                8,
            );
            let hello: DesktopHello = stream.recv_json().unwrap();
            let auth_res = session.accept_hello(&hello);
            assert!(matches!(auth_res, Err(IpcError::AuthenticationRejected)));
            // Server responds with auth rejected error and closes stream
            stream.send_frame(r#"{"error":"rejected"}"#).unwrap();
        }

        // 2. Second client connection: sends correct token -> established
        {
            let mut stream = listener.accept().unwrap();
            let mut session = ServerSession::new(
                server_creds,
                ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap(),
                ProductVersion::new(0, 1, 0),
                CapabilitySet::new(),
                8,
            );
            let hello: DesktopHello = stream.recv_json().unwrap();
            let established = session.accept_hello(&hello).unwrap();
            stream
                .send_json(&(established.negotiated.clone(), established.greeting.clone()))
                .unwrap();

            // Client sends a command
            let cmd: CommandEnvelope = stream.recv_json().unwrap();
            assert_eq!(cmd.kind, CommandKind::Ping);
        }
    });

    ready_rx.recv().unwrap();

    // 1. Bad token client
    {
        let mut client_stream =
            LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();
        let bad_hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .unwrap(),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
            launch_token: mint_launch_token(&[0x99; 16]).unwrap(),
        };
        client_stream.send_json(&bad_hello).unwrap();
        let reply = client_stream.recv_frame_string().unwrap();
        assert_eq!(reply, r#"{"error":"rejected"}"#);
    }

    // 2. Valid token client
    {
        let mut client_stream =
            LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();
        let good_hello = DesktopHello {
            supported_versions: ProtocolVersionRange::try_new(
                ProtocolVersion::V1,
                ProtocolVersion::V1,
            )
            .unwrap(),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
            launch_token: creds.launch_token.clone(),
        };
        client_stream.send_json(&good_hello).unwrap();

        let mut client_session = ClientSession::new();
        let (negotiated, greeting): (
            altior_protocol::NegotiatedHandshake,
            altior_protocol::CoreGreeting,
        ) = client_stream.recv_json().unwrap();
        let outcome = client_session
            .accept_greeting(&greeting, &negotiated)
            .unwrap();
        assert_eq!(outcome, GreetingOutcome::Restarted);

        let ping = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: OperationId::from_str("op_fixture000000001").unwrap(),
            kind: CommandKind::Ping,
            payload: None,
            issued_at: UnixMillis::from_millis(100),
        };
        client_stream.send_json(&ping).unwrap();
    }

    server_handle.join().unwrap();
}

#[test]
fn listener_survives_disconnect_and_handles_sequential_and_concurrent_clients() {
    let endpoint = unique_endpoint();
    let (ready_tx, ready_rx) = channel();

    let server_endpoint = endpoint.clone();
    let server_handle = thread::spawn(move || {
        let listener = LocalListener::bind(&server_endpoint).unwrap();
        ready_tx.send(()).unwrap();

        // 1. Accept client 1, client 1 sends message then disconnects
        {
            let mut stream1 = listener.accept().unwrap();
            let msg1 = stream1.recv_frame_string().unwrap();
            assert_eq!(msg1, "client-1-msg");
            stream1.send_frame("ack-1").unwrap();
            // stream1 is dropped here (client 1 disconnected)
        }

        // 2. Accept client 2 on same listener
        {
            let mut stream2 = listener.accept().unwrap();
            let msg2 = stream2.recv_frame_string().unwrap();
            assert_eq!(msg2, "client-2-msg");
            stream2.send_frame("ack-2").unwrap();
        }

        // 3. Accept 2 concurrent clients
        let mut stream_c1 = listener.accept().unwrap();
        let mut stream_c2 = listener.accept().unwrap();

        let msg_c1 = stream_c1.recv_frame_string().unwrap();
        let msg_c2 = stream_c2.recv_frame_string().unwrap();

        stream_c1.send_frame(&format!("reply-{msg_c1}")).unwrap();
        stream_c2.send_frame(&format!("reply-{msg_c2}")).unwrap();
    });

    ready_rx.recv().unwrap();

    // Client 1 connects and disconnects
    {
        let mut client1 = LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();
        client1.send_frame("client-1-msg").unwrap();
        assert_eq!(client1.recv_frame_string().unwrap(), "ack-1");
    }

    // Client 2 connects to surviving listener
    {
        let mut client2 = LocalStream::connect(&endpoint, Some(Duration::from_secs(5))).unwrap();
        client2.send_frame("client-2-msg").unwrap();
        assert_eq!(client2.recv_frame_string().unwrap(), "ack-2");
    }

    // Concurrent clients using Barrier
    let barrier = Arc::new(Barrier::new(2));
    let ep = endpoint.clone();
    let b1 = Arc::clone(&barrier);
    let h1 = thread::spawn(move || {
        b1.wait();
        let mut c = LocalStream::connect(&ep, Some(Duration::from_secs(5))).unwrap();
        c.send_frame("alpha").unwrap();
        let reply = c.recv_frame_string().unwrap();
        assert_eq!(reply, "reply-alpha");
    });

    let ep2 = endpoint.clone();
    let b2 = Arc::clone(&barrier);
    let h2 = thread::spawn(move || {
        b2.wait();
        let mut c = LocalStream::connect(&ep2, Some(Duration::from_secs(5))).unwrap();
        c.send_frame("beta").unwrap();
        let reply = c.recv_frame_string().unwrap();
        assert_eq!(reply, "reply-beta");
    });

    h1.join().unwrap();
    h2.join().unwrap();
    server_handle.join().unwrap();
}

#[test]
fn typed_connect_failures() {
    // Non-existent endpoint returns NotFound
    let missing_endpoint = if cfg!(windows) {
        Endpoint::windows_pipe(r"\\.\pipe\altior-nonexistent-pipe-999999").unwrap()
    } else {
        Endpoint::unix_socket("/tmp/altior-nonexistent-sock-999999.sock").unwrap()
    };

    let res = LocalStream::connect(&missing_endpoint, Some(Duration::from_millis(50)));
    assert!(
        matches!(res, Err(IpcError::NotFound { .. })),
        "expected NotFound, got {res:?}"
    );
}

#[test]
fn discovery_file_lifecycle_and_stale_cleanup() {
    let temp_dir = tempfile::tempdir().unwrap();
    let discovery_path = temp_dir.path().join("altior-core-discovery.json");

    let instance_id: CoreInstanceId = "cor_fixture000000042".parse().unwrap();
    let endpoint = unique_endpoint();
    let launch_token = mint_launch_token(&[0x77; 16]).unwrap();

    let discovery = EndpointDiscovery {
        instance_id: instance_id.clone(),
        endpoint: endpoint.clone(),
        launch_token: launch_token.clone(),
    };

    // 1. Initial write
    write_discovery_file(&discovery_path, &discovery).unwrap();
    let read = read_discovery_file(&discovery_path).unwrap();
    assert_eq!(read, discovery);

    // 2. Debug redaction check
    let debug_repr = format!("{read:?}");
    assert!(!debug_repr.contains(launch_token.as_str()));
    assert!(debug_repr.contains("[REDACTED]"));

    // 3. Stale cleanup
    cleanup_stale_discovery(&discovery_path).unwrap();
    assert!(!discovery_path.exists());

    // 4. Cleaned up file returns NotFound
    let not_found = read_discovery_file(&discovery_path);
    assert!(matches!(not_found, Err(IpcError::NotFound { .. })));

    // 5. Subsequent rewrite succeeds atomically
    write_discovery_file(&discovery_path, &discovery).unwrap();
    assert!(discovery_path.exists());
    let reread = read_discovery_file(&discovery_path).unwrap();
    assert_eq!(reread.instance_id, instance_id);
}

#[test]
fn second_listener_on_same_endpoint_is_rejected() {
    let endpoint = unique_endpoint();
    let _first_listener = LocalListener::bind(&endpoint).unwrap();

    // Binding a second listener on the exact same endpoint must fail
    let second_res = LocalListener::bind(&endpoint);
    assert!(
        second_res.is_err(),
        "second listener bind on {endpoint:?} must be rejected to prevent squatting"
    );
}

#[test]
fn unix_path_length_limit_104_and_105_bytes() {
    let valid_path_104 = format!("/tmp/{}", "x".repeat(104 - 5));
    assert_eq!(valid_path_104.len(), 104);
    let ep_104 = Endpoint::unix_socket(&valid_path_104).unwrap();
    assert_eq!(ep_104.address(), valid_path_104);

    let invalid_path_105 = format!("/tmp/{}", "x".repeat(105 - 5));
    assert_eq!(invalid_path_105.len(), 105);
    let err_105 = Endpoint::unix_socket(&invalid_path_105).unwrap_err();
    assert!(matches!(
        err_105,
        IpcError::InvalidEndpoint { reason } if reason.contains("104 bytes")
    ));

    // Multi-byte UTF-8 test: 34 * 3 bytes + 2 bytes = 104 bytes
    let valid_mb_104 = format!("{}ab", "测".repeat(34));
    assert_eq!(valid_mb_104.len(), 104);
    assert!(Endpoint::unix_socket(&valid_mb_104).is_ok());

    // Multi-byte UTF-8 test: 35 * 3 bytes = 105 bytes
    let invalid_mb_105 = "测".repeat(35);
    assert_eq!(invalid_mb_105.len(), 105);
    assert!(matches!(
        Endpoint::unix_socket(&invalid_mb_105),
        Err(IpcError::InvalidEndpoint { reason }) if reason.contains("104 bytes")
    ));
}

#[test]
fn all_structs_debug_redact_tokens_zero_plaintext() {
    let secret = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";
    let token: altior_protocol::LaunchToken = secret.parse().unwrap();
    let instance_id: CoreInstanceId = "cor_fixture000000042".parse().unwrap();

    // 1. LaunchToken Debug
    let token_dbg = format!("{token:?}");
    let token_dbg_alt = format!("{token:#?}");
    assert!(!token_dbg.contains(secret));
    assert!(!token_dbg_alt.contains(secret));
    assert_eq!(token_dbg, "LaunchToken(\"[REDACTED]\")");

    // 2. DesktopHello Debug
    let hello = DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(1).unwrap(),
            ProtocolVersion::try_new(1).unwrap(),
        )
        .unwrap(),
        desktop_version: ProductVersion::new(1, 0, 0),
        capabilities: CapabilitySet::new(),
        launch_token: token.clone(),
    };
    let hello_dbg = format!("{hello:?}");
    let hello_dbg_alt = format!("{hello:#?}");
    assert!(!hello_dbg.contains(secret));
    assert!(!hello_dbg_alt.contains(secret));
    assert!(hello_dbg.contains("[REDACTED]"));
    assert!(hello_dbg.contains("desktop_version"));
    assert!(hello_dbg.contains("supported_versions"));

    // 3. LaunchCredentials Debug
    let creds = LaunchCredentials {
        instance_id: instance_id.clone(),
        launch_token: token.clone(),
    };
    let creds_dbg = format!("{creds:?}");
    let creds_dbg_alt = format!("{creds:#?}");
    assert!(!creds_dbg.contains(secret));
    assert!(!creds_dbg_alt.contains(secret));
    assert!(creds_dbg.contains("[REDACTED]"));

    // 4. EndpointDiscovery Debug
    let discovery = EndpointDiscovery {
        instance_id,
        endpoint: unique_endpoint(),
        launch_token: token,
    };
    let disc_dbg = format!("{discovery:?}");
    let disc_dbg_alt = format!("{discovery:#?}");
    assert!(!disc_dbg.contains(secret));
    assert!(!disc_dbg_alt.contains(secret));
    assert!(disc_dbg.contains("[REDACTED]"));
}

#[cfg(unix)]
#[test]
fn unix_socket_bind_permissions_0600_and_umask_restoration() {
    use std::os::unix::fs::PermissionsExt;

    fn get_umask() -> libc::mode_t {
        unsafe {
            let m = libc::umask(0);
            libc::umask(m);
            m
        }
    }

    // Explicitly set a known umask
    let initial_mask = 0o022;
    unsafe {
        libc::umask(initial_mask);
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let sock_path = temp_dir.path().join("test_perm.sock");
    let sock_path_str = sock_path.to_str().unwrap();
    let endpoint = Endpoint::unix_socket(sock_path_str).unwrap();

    // Successful bind
    let listener = LocalListener::bind(&endpoint).unwrap();

    // Check socket file permissions: must be 0600
    let metadata = std::fs::metadata(&sock_path).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "socket permissions should be strictly 0600");

    // Check umask after successful bind: must be restored to initial_mask
    assert_eq!(
        get_umask(),
        initial_mask,
        "umask must be restored after successful bind"
    );

    // Second bind on existing in-use socket should fail
    let second_bind = LocalListener::bind(&endpoint);
    assert!(second_bind.is_err());

    // Check umask after failed bind: must STILL be restored to initial_mask
    assert_eq!(
        get_umask(),
        initial_mask,
        "umask must be restored after failed bind"
    );

    drop(listener);
}
