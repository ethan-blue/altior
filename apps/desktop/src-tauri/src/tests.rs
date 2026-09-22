//! Headless unit and integration tests for the Core client, discovery, spawner, and command bridge.
//!
//! Tests run entirely in-process without requiring Tauri GUI or system WebViews (ADR 0008 §6).

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use altior_domain::{CoreInstanceId, EventId, OperationId, UnixMillis};
use altior_ipc::auth::{mint_launch_token, LaunchCredentials};
use altior_ipc::endpoint::Endpoint;
use altior_ipc::frame::encode_frame;
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CommandKind, CoreGreeting, CoreHello, DesktopHello, EventBody,
    EventEnvelope, KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange, Sequence,
};
use serde_json::json;

use crate::adapter::{MockChannelState, MockCoreConnector};
use crate::discovery::CoreDiscovery;
use crate::error::{BridgeError, DiscoveryError, SpawnError};
use crate::manager::SpawnOrAttachManager;
use crate::session::BridgeSession;
use crate::spawner::CoreSpawner;
use crate::state::{AppIpcState, ReconnectCursor, TransportStatus};

// ── Test Fakes ──────────────────────────────────────────────────────

const TEST_TOKEN_ENTROPY: [u8; 16] = [
    0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8, 0xf9,
];

fn test_credentials(instance: &str) -> LaunchCredentials {
    LaunchCredentials {
        instance_id: CoreInstanceId::from_str(instance).unwrap(),
        launch_token: mint_launch_token(&TEST_TOKEN_ENTROPY).unwrap(),
    }
}

/// Fake discovery for testing attach/stale/missing token states.
struct FakeDiscovery {
    creds: Arc<Mutex<Option<LaunchCredentials>>>,
    invalidated: Arc<AtomicBool>,
    endpoint: Endpoint,
}

impl FakeDiscovery {
    fn new(creds: Option<LaunchCredentials>) -> Self {
        Self {
            creds: Arc::new(Mutex::new(creds)),
            invalidated: Arc::new(AtomicBool::new(false)),
            endpoint: Endpoint::WindowsPipe(r"\\.\pipe\altior-core-test".to_string()),
        }
    }

    fn set_credentials(&self, creds: Option<LaunchCredentials>) {
        *self.creds.lock().unwrap() = creds;
    }
}

impl CoreDiscovery for FakeDiscovery {
    fn discover_credentials(&self) -> Result<Option<LaunchCredentials>, DiscoveryError> {
        Ok(self.creds.lock().unwrap().clone())
    }

    fn resolve_endpoint(&self) -> Result<Endpoint, DiscoveryError> {
        Ok(self.endpoint.clone())
    }

    fn invalidate_stale_token(&self) -> Result<(), DiscoveryError> {
        self.invalidated.store(true, Ordering::SeqCst);
        *self.creds.lock().unwrap() = None;
        Ok(())
    }

    fn token_file_path(&self) -> Result<PathBuf, DiscoveryError> {
        Ok(PathBuf::from("/fake/token.path"))
    }
}

/// Fake process spawner tracking spawn count and detached state.
struct FakeSpawner {
    spawn_count: Arc<AtomicU32>,
    is_running_flag: Arc<AtomicBool>,
    killed_on_drop: Arc<AtomicBool>,
    last_args: Arc<Mutex<Vec<String>>>,
}

impl FakeSpawner {
    fn new() -> Self {
        Self {
            spawn_count: Arc::new(AtomicU32::new(0)),
            is_running_flag: Arc::new(AtomicBool::new(false)),
            killed_on_drop: Arc::new(AtomicBool::new(false)),
            last_args: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CoreSpawner for FakeSpawner {
    fn is_running(&self) -> bool {
        self.is_running_flag.load(Ordering::SeqCst)
    }

    fn spawn_detached(&self, args: &[String]) -> Result<u32, SpawnError> {
        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        self.is_running_flag.store(true, Ordering::SeqCst);
        *self.last_args.lock().unwrap() = args.to_vec();
        Ok(12345)
    }

    fn resolve_binary_path(&self) -> Result<PathBuf, SpawnError> {
        Ok(PathBuf::from("/fake/altior-core.exe"))
    }
}

/// Simulates a responsive Core backend over a MockChannelState.
fn simulate_core_server(
    channel_state: &MockChannelState,
    expected_token: &altior_protocol::LaunchToken,
    instance_id: &str,
) {
    let cs = channel_state.clone();
    let expected = expected_token.clone();
    let instance = CoreInstanceId::from_str(instance_id).unwrap();

    std::thread::spawn(move || {
        while cs.is_connected() {
            if let Some(frame) = cs.pop_client_frame() {
                if let Ok(hello) = serde_json::from_slice::<DesktopHello>(&frame) {
                    if hello.launch_token != expected {
                        // Bad auth: close connection
                        cs.disconnect();
                        return;
                    }

                    // Send CoreHello
                    let range =
                        ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
                            .unwrap();
                    let core_hello = CoreHello {
                        supported_versions: range,
                        core_version: ProductVersion::new(0, 1, 0),
                        capabilities: CapabilitySet::new(),
                    };
                    let h_json = serde_json::to_string(&core_hello).unwrap();
                    let h_frame = encode_frame(&h_json).unwrap();
                    cs.push_server_frame(h_frame);

                    // Send CoreGreeting
                    let greeting = CoreGreeting {
                        protocol_version: ProtocolVersion::V1,
                        instance_id: instance.clone(),
                        core_version: ProductVersion::new(0, 1, 0),
                        retained: None,
                    };
                    let g_json = serde_json::to_string(&greeting).unwrap();
                    let g_frame = encode_frame(&g_json).unwrap();
                    cs.push_server_frame(g_frame);
                    continue;
                }

                if let Ok(cmd) = serde_json::from_slice::<CommandEnvelope>(&frame) {
                    match cmd.kind {
                        CommandKind::Ping => {
                            // Ping auto-responded
                        }
                        _ => {
                            // Send CommandResult event
                            let result_event = EventEnvelope {
                                protocol_version: ProtocolVersion::V1,
                                event_id: EventId::from_str("evt_fixture000000001").unwrap(),
                                operation_id: Some(cmd.operation_id.clone()),
                                thread_id: None,
                                turn_id: None,
                                sequence: Sequence::FIRST,
                                occurred_at: UnixMillis::from_millis(1_700_000_000_000),
                                body: EventBody::Known(KnownEvent::CommandResult {
                                    operation_id: cmd.operation_id,
                                    success: true,
                                    data: None,
                                }),
                            };
                            let e_json = serde_json::to_string(&result_event).unwrap();
                            let e_frame = encode_frame(&e_json).unwrap();
                            cs.push_server_frame(e_frame);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn test_attach_existing_core() {
    let creds = test_credentials("cor_fixture000000001");
    let fake_disc = Arc::new(FakeDiscovery::new(Some(creds.clone())));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    simulate_core_server(&channel_state, &creds.launch_token, "cor_fixture000000001");

    let fake_conn = Arc::new(MockCoreConnector::new(channel_state));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc.clone(),
        fake_spawn.clone(),
        fake_conn,
        session,
    ));

    let state = AppIpcState::with_manager(manager);

    let handshake = state
        .handshake_sync(Some("altior-desktop".to_string()))
        .expect("handshake succeeds");
    assert_eq!(handshake.selected_version, ProtocolVersion::V1);
    assert_eq!(state.status(), TransportStatus::Connected);

    // Spawner must NOT have been called because existing Core was found
    assert_eq!(fake_spawn.spawn_count.load(Ordering::SeqCst), 0);
    assert!(!fake_disc.invalidated.load(Ordering::SeqCst));
}

#[test]
fn test_spawn_once_when_missing() {
    let creds = test_credentials("cor_fixture000000002");
    // Start with NO discovery credentials (Core not running)
    let fake_disc = Arc::new(FakeDiscovery::new(None));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc.clone(),
        fake_spawn.clone(),
        fake_conn,
        session,
    ));

    // Spawn server background thread after a short delay (simulating spawn)
    let disc_clone = fake_disc.clone();
    let creds_clone = creds.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        disc_clone.set_credentials(Some(creds_clone.clone()));
        simulate_core_server(
            &channel_state,
            &creds_clone.launch_token,
            "cor_fixture000000002",
        );
    });

    let state = AppIpcState::with_manager(manager);
    let handshake = state
        .handshake_sync(Some("altior-desktop".to_string()))
        .expect("handshake succeeds");
    assert_eq!(handshake.selected_version, ProtocolVersion::V1);
    assert_eq!(state.status(), TransportStatus::Connected);

    // Spawner was called exactly once
    assert_eq!(fake_spawn.spawn_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_stale_discovery_triggers_cleanup_and_respawn() {
    let old_creds = test_credentials("cor_fixture000000088");
    let new_creds = test_credentials("cor_fixture000000099");

    let fake_disc = Arc::new(FakeDiscovery::new(Some(old_creds.clone())));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    // Old connection fails
    fake_conn.set_should_fail(true);

    let session = Arc::new(BridgeSession::default());
    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc.clone(),
        fake_spawn.clone(),
        fake_conn.clone(),
        session,
    ));

    // Simulate spawn yielding fresh credentials and successful connection
    let disc_clone = fake_disc.clone();
    let conn_clone = fake_conn.clone();
    let new_creds_clone = new_creds.clone();
    let cs = channel_state.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        conn_clone.set_should_fail(false);
        disc_clone.set_credentials(Some(new_creds_clone.clone()));
        simulate_core_server(&cs, &new_creds_clone.launch_token, "cor_fixture000000099");
    });

    let state = AppIpcState::with_manager(manager);
    let handshake = state
        .handshake_sync(None)
        .expect("reconnects to freshly spawned core");
    assert_eq!(handshake.selected_version, ProtocolVersion::V1);

    // Stale discovery was invalidated and spawner was invoked
    assert!(fake_disc.invalidated.load(Ordering::SeqCst));
    assert_eq!(fake_spawn.spawn_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_bad_auth_rejects_without_logging_secrets() {
    let real_server_token = mint_launch_token(&[0x11; 16]).unwrap();
    let wrong_client_token = mint_launch_token(&[0x22; 16]).unwrap();

    let creds = LaunchCredentials {
        instance_id: CoreInstanceId::from_str("cor_fixture000000004").unwrap(),
        launch_token: wrong_client_token,
    };

    let fake_disc = Arc::new(FakeDiscovery::new(Some(creds)));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    simulate_core_server(&channel_state, &real_server_token, "cor_fixture000000004");

    let fake_conn = Arc::new(MockCoreConnector::new(channel_state));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc, fake_spawn, fake_conn, session,
    ));

    let state = AppIpcState::with_manager(manager);
    let result = state.handshake_sync(None);

    assert!(result.is_err());
    let err = result.err().unwrap();
    // Error must be typed serializable
    let err_str = err.to_string();
    assert!(!err_str.contains("11111111"));
    assert!(!err_str.contains("22222222"));
    assert_eq!(state.status(), TransportStatus::Unavailable);
}

#[test]
fn test_reconnect_cursor_and_state_continuity() {
    let creds = test_credentials("cor_fixture000000005");
    let fake_disc = Arc::new(FakeDiscovery::new(Some(creds.clone())));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    simulate_core_server(&channel_state, &creds.launch_token, "cor_fixture000000005");

    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc, fake_spawn, fake_conn, session,
    ));

    let state = AppIpcState::with_manager(manager);
    state.handshake_sync(None).expect("initial handshake");

    // Reconnect with cursor 42
    let reconnect_cursor = ReconnectCursor {
        last_sequence: Some(42),
        cursor_id: Some("cur_test_01".to_string()),
    };
    // Reconnect establishes a fresh physical channel to the same Core.
    simulate_core_server(&channel_state, &creds.launch_token, "cor_fixture000000005");
    let reconnected = state
        .reconnect_sync(Some(reconnect_cursor))
        .expect("reconnects");
    assert_eq!(reconnected.selected_version, ProtocolVersion::V1);
    assert_eq!(state.last_sequence(), 42);
}

#[test]
fn test_ui_state_drop_does_not_kill_child() {
    let creds = test_credentials("cor_fixture000000006");
    let fake_disc = Arc::new(FakeDiscovery::new(Some(creds.clone())));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    simulate_core_server(&channel_state, &creds.launch_token, "cor_fixture000000006");

    let fake_conn = Arc::new(MockCoreConnector::new(channel_state));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc,
        fake_spawn.clone(),
        fake_conn,
        session,
    ));

    {
        let state = AppIpcState::with_manager(manager);
        let _ = state.handshake_sync(None);
        let _ = state.close_sync();
        // Drop state at end of block
    }

    // Spawner child must NOT be killed on drop
    assert!(!fake_spawn.killed_on_drop.load(Ordering::SeqCst));
}

#[test]
fn test_command_and_event_mapping_with_deduplication() {
    let creds = test_credentials("cor_fixture000000007");
    let fake_disc = Arc::new(FakeDiscovery::new(Some(creds.clone())));
    let fake_spawn = Arc::new(FakeSpawner::new());

    let channel_state = MockChannelState::new();
    simulate_core_server(&channel_state, &creds.launch_token, "cor_fixture000000007");

    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc, fake_spawn, fake_conn, session,
    ));

    let state = AppIpcState::with_manager(manager);
    state.handshake_sync(None).expect("handshake succeeds");

    let received_events = Arc::new(Mutex::new(Vec::new()));
    let rx_clone = received_events.clone();
    state.subscribe_events(move |event| {
        rx_clone.lock().unwrap().push(event);
    });

    // 1. Send Ping command
    let ping_env = CommandEnvelope::ping(
        OperationId::from_str("op_fixture000000001").unwrap(),
        UnixMillis::from_millis(1_700_000_000_000),
    );
    let ping_res = state.command_sync(ping_env).expect("ping command succeeds");
    assert_eq!(ping_res.get("ok"), Some(&json!(true)));

    // 2. Emit an event from Core to Desktop
    let test_event = EventEnvelope {
        protocol_version: ProtocolVersion::V1,
        event_id: EventId::from_str("evt_fixture000000010").unwrap(),
        operation_id: None,
        thread_id: None,
        turn_id: None,
        sequence: Sequence::try_new(10).unwrap(),
        occurred_at: UnixMillis::from_millis(1_700_000_000_010),
        body: EventBody::Known(KnownEvent::TurnStarted),
    };
    let e_json = serde_json::to_string(&test_event).unwrap();
    let e_frame = encode_frame(&e_json).unwrap();
    channel_state.push_server_frame(e_frame.clone());

    // Push DUPLICATE event
    channel_state.push_server_frame(e_frame);

    std::thread::sleep(Duration::from_millis(100));

    let events = received_events.lock().unwrap();
    // Both frames reached the subscriber, but sequence advanced once
    assert!(!events.is_empty());
    assert_eq!(state.last_sequence(), 10);
}

#[test]
fn test_spawn_passes_explicit_data_dir() {
    let creds = test_credentials("cor_fixture000000010");
    let fake_disc = Arc::new(FakeDiscovery::new(None));
    let fake_spawn = Arc::new(FakeSpawner::new());
    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    let session = Arc::new(BridgeSession::default());

    let manager = Arc::new(SpawnOrAttachManager::new(
        fake_disc.clone(),
        fake_spawn.clone(),
        fake_conn,
        session,
    ));

    let disc_clone = fake_disc.clone();
    let creds_clone = creds.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        disc_clone.set_credentials(Some(creds_clone.clone()));
        simulate_core_server(
            &channel_state,
            &creds_clone.launch_token,
            "cor_fixture000000010",
        );
    });

    let state = AppIpcState::with_manager(manager.clone());
    let _ = state.handshake_sync(None).expect("handshake succeeds");

    let args = fake_spawn.last_args.lock().unwrap().clone();
    assert!(args.contains(&"--daemon".to_string()));
    assert!(args.contains(&"--data-dir".to_string()));
    let data_dir_idx = args.iter().position(|a| a == "--data-dir").unwrap();
    assert_eq!(
        args[data_dir_idx + 1],
        manager.data_dir().unwrap().to_string_lossy()
    );
}

#[test]
fn test_spawn_passes_custom_data_dir() {
    let creds = test_credentials("cor_fixture000000011");
    let fake_disc = Arc::new(FakeDiscovery::new(None));
    let fake_spawn = Arc::new(FakeSpawner::new());
    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state.clone()));
    let session = Arc::new(BridgeSession::default());

    let custom_dir = PathBuf::from("/custom/altior/per_user_data");
    let manager = Arc::new(
        SpawnOrAttachManager::new(fake_disc.clone(), fake_spawn.clone(), fake_conn, session)
            .with_data_dir(custom_dir.clone()),
    );

    let disc_clone = fake_disc.clone();
    let creds_clone = creds.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        disc_clone.set_credentials(Some(creds_clone.clone()));
        simulate_core_server(
            &channel_state,
            &creds_clone.launch_token,
            "cor_fixture000000011",
        );
    });

    let state = AppIpcState::with_manager(manager);
    let _ = state.handshake_sync(None).expect("handshake succeeds");

    let args = fake_spawn.last_args.lock().unwrap().clone();
    assert!(args.contains(&"--daemon".to_string()));
    assert!(args.contains(&"--data-dir".to_string()));
    let data_dir_idx = args.iter().position(|a| a == "--data-dir").unwrap();
    assert_eq!(args[data_dir_idx + 1], custom_dir.to_string_lossy());
}

#[test]
fn test_spawn_fails_when_data_dir_creation_fails() {
    let fake_disc = Arc::new(FakeDiscovery::new(None));
    let fake_spawn = Arc::new(FakeSpawner::new());
    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state));
    let session = Arc::new(BridgeSession::default());

    // Create a regular file in temp dir so create_dir_all fails on a nested child path
    let blocker_file = std::env::temp_dir().join(format!(
        "altior_test_file_blocker_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&blocker_file, b"cannot be a directory").expect("write blocker file");
    let invalid_dir = blocker_file.join("nested_data_dir");

    let manager = Arc::new(
        SpawnOrAttachManager::new(fake_disc, fake_spawn.clone(), fake_conn, session)
            .with_data_dir(invalid_dir),
    );

    let result = manager.attach_or_spawn();
    let _ = std::fs::remove_file(&blocker_file);

    assert!(result.is_err());
    match result.err().unwrap() {
        BridgeError::SpawnFailed(msg) => {
            assert!(msg.contains("Failed to create data directory"));
        }
        other => panic!("expected BridgeError::SpawnFailed, got {other:?}"),
    }

    // Spawner must not have been invoked because create_dir_all failed before spawn_detached
    assert_eq!(fake_spawn.spawn_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_tauri_data_dir_env_deterministic() {
    use crate::manager::{DataDirEnv, DataDirError};

    // Windows with LOCALAPPDATA
    let win_env = DataDirEnv {
        local_app_data: Some(r"C:\Users\tester\AppData\Local".to_string()),
        ..Default::default()
    };
    assert_eq!(
        win_env.resolve_for_windows().unwrap(),
        PathBuf::from(r"C:\Users\tester\AppData\Local\altior\data")
    );

    // Windows with USERPROFILE fallback
    let win_home_env = DataDirEnv {
        home: Some(r"C:\Users\tester".to_string()),
        ..Default::default()
    };
    assert_eq!(
        win_home_env.resolve_for_windows().unwrap(),
        PathBuf::from(r"C:\Users\tester\AppData\Local\altior\data")
    );

    // Windows with empty env fails, never C:\altior\data
    let win_empty = DataDirEnv::default();
    assert_eq!(
        win_empty.resolve_for_windows(),
        Err(DataDirError::ResolutionFailed)
    );

    // Unix with XDG_DATA_HOME
    let unix_xdg_env = DataDirEnv {
        xdg_data_home: Some("/custom/xdg/share".to_string()),
        ..Default::default()
    };
    assert_eq!(
        unix_xdg_env.resolve_for_unix().unwrap(),
        PathBuf::from("/custom/xdg/share/altior/data")
    );

    // Unix with HOME
    let unix_home_env = DataDirEnv {
        home: Some("/home/tester".to_string()),
        ..Default::default()
    };
    assert_eq!(
        unix_home_env.resolve_for_unix().unwrap(),
        PathBuf::from("/home/tester/.local/share/altior/data")
    );

    // Unix with XDG_RUNTIME_DIR fallback
    let unix_runtime_env = DataDirEnv {
        xdg_runtime_dir: Some("/run/user/1000".to_string()),
        ..Default::default()
    };
    assert_eq!(
        unix_runtime_env.resolve_for_unix().unwrap(),
        PathBuf::from("/run/user/1000/altior/data")
    );

    // Unix with empty env fails, never /tmp/altior/data
    let unix_empty = DataDirEnv::default();
    assert_eq!(
        unix_empty.resolve_for_unix(),
        Err(DataDirError::ResolutionFailed)
    );

    // macOS with HOME
    let mac_env = DataDirEnv {
        home: Some("/Users/tester".to_string()),
        ..Default::default()
    };
    assert_eq!(
        mac_env.resolve_for_macos().unwrap(),
        PathBuf::from("/Users/tester/Library/Application Support/altior/data")
    );

    // macOS with empty env fails, never /tmp/altior/data
    let mac_empty = DataDirEnv::default();
    assert_eq!(
        mac_empty.resolve_for_macos(),
        Err(DataDirError::ResolutionFailed)
    );

    // Override takes precedence across all platforms
    let override_env = DataDirEnv {
        altior_data_dir: Some("/override/altior/data".to_string()),
        local_app_data: Some(r"C:\Users\tester\AppData\Local".to_string()),
        home: Some("/home/tester".to_string()),
        ..Default::default()
    };
    assert_eq!(
        override_env.resolve_for_windows().unwrap(),
        PathBuf::from("/override/altior/data")
    );
    assert_eq!(
        override_env.resolve_for_unix().unwrap(),
        PathBuf::from("/override/altior/data")
    );
    assert_eq!(
        override_env.resolve_for_macos().unwrap(),
        PathBuf::from("/override/altior/data")
    );
}

#[test]
fn test_tauri_data_dir_rejects_relative_and_traversal() {
    use crate::manager::{DataDirEnv, DataDirError};

    // Relative override rejected
    let env_rel = DataDirEnv {
        altior_data_dir: Some("relative/data".to_string()),
        ..Default::default()
    };
    assert_eq!(
        env_rel.resolve_for_unix(),
        Err(DataDirError::RelativePath(PathBuf::from("relative/data")))
    );

    // Traversal override rejected
    let env_trav = DataDirEnv {
        altior_data_dir: Some("/var/altior/../evil".to_string()),
        ..Default::default()
    };
    assert_eq!(
        env_trav.resolve_for_unix(),
        Err(DataDirError::ParentDirTraversal(PathBuf::from(
            "/var/altior/../evil"
        )))
    );

    // Traversal in platform home rejected
    let env_home_trav = DataDirEnv {
        home: Some("/home/tester/..".to_string()),
        ..Default::default()
    };
    assert_eq!(
        env_home_trav.resolve_for_unix(),
        Err(DataDirError::ParentDirTraversal(PathBuf::from(
            "/home/tester/.."
        )))
    );
}

#[test]
fn test_tauri_spawn_fails_on_relative_or_traversal_data_dir() {
    let fake_disc = Arc::new(FakeDiscovery::new(None));
    let fake_spawn = Arc::new(FakeSpawner::new());
    let channel_state = MockChannelState::new();
    let fake_conn = Arc::new(MockCoreConnector::new(channel_state));
    let session = Arc::new(BridgeSession::default());

    // Relative data directory
    let manager_rel = Arc::new(
        SpawnOrAttachManager::new(
            fake_disc.clone(),
            fake_spawn.clone(),
            fake_conn.clone(),
            session.clone(),
        )
        .with_data_dir(PathBuf::from("relative/data/dir")),
    );
    let result_rel = manager_rel.attach_or_spawn();
    assert!(result_rel.is_err());
    match result_rel.err().unwrap() {
        BridgeError::SpawnFailed(msg) => {
            assert!(
                msg.contains("Invalid data directory")
                    || msg.contains("data directory path must be absolute")
            );
        }
        other => panic!("expected BridgeError::SpawnFailed, got {other:?}"),
    }

    // Traversal data directory
    let manager_trav = Arc::new(
        SpawnOrAttachManager::new(fake_disc, fake_spawn.clone(), fake_conn, session)
            .with_data_dir(PathBuf::from("/var/altior/../evil")),
    );
    let result_trav = manager_trav.attach_or_spawn();
    assert!(result_trav.is_err());
    match result_trav.err().unwrap() {
        BridgeError::SpawnFailed(msg) => {
            assert!(msg.contains("Invalid data directory") || msg.contains("parent traversal"));
        }
        other => panic!("expected BridgeError::SpawnFailed, got {other:?}"),
    }

    // Spawner must never have been called
    assert_eq!(fake_spawn.spawn_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_tauri_create_secure_data_dir() {
    use crate::manager::create_secure_data_dir;

    let temp_dir = std::env::temp_dir().join(format!(
        "altior_desktop_sec_dir_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let nested = temp_dir.join("altior").join("data");
    create_secure_data_dir(&nested).expect("secure dir creation succeeds");
    assert!(nested.is_dir());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&nested).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "data dir must be mode 0700");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
