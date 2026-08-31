//! Integration tests for the ACP subprocess runtime adapter (P1.2, ADR 0007).
//!
//! Uses the in-repo mock child binary (`mock_acp_agent`) to exercise all
//! adapter capabilities deterministically without sleeps, external binaries,
//! or network dependencies.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use altior_acp::messages::{ContentBlock, StopReason};
use altior_acp::{
    AcpChild, AcpError, AcpRuntime, AgentEvent, DeliveryState, LaunchConfig,
    MAX_STDERR_CAPTURE_BYTES, NormalizedEvent, ProcessTransport, ResolvedLaunchConfig, SecretRef,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempExeGuard {
    path: PathBuf,
}

impl TempExeGuard {
    fn new() -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_path =
            std::env::temp_dir().join(format!("altior_mock_agent_{pid}_{counter}_{timestamp}.exe"));
        let src = env!("CARGO_BIN_EXE_mock_acp_agent");
        std::fs::copy(src, &temp_path).expect("failed to copy mock agent binary to temp path");
        Self { path: temp_path }
    }

    fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempExeGuard {
    fn drop(&mut self) {
        if std::fs::remove_file(&self.path).is_err() {
            for _ in 0..10 {
                std::thread::yield_now();
                if std::fs::remove_file(&self.path).is_ok() {
                    break;
                }
            }
        }
    }
}

struct TestRuntimeGuard {
    runtime: Option<AcpRuntime<AcpChild>>,
    _exe_guard: TempExeGuard,
}

impl TestRuntimeGuard {
    fn spawn(scenario: &str) -> Self {
        let exe_guard = TempExeGuard::new();
        let mut env = BTreeMap::new();
        env.insert("ALTIOR_ACP_MOCK_SCENARIO".to_owned(), scenario.to_owned());

        let config = ResolvedLaunchConfig::new(exe_guard.path_str(), Vec::new(), None, env)
            .expect("valid resolved launch config");

        let child = AcpChild::spawn(&config).expect("mock child spawned");
        let runtime = AcpRuntime::new(child);
        Self {
            runtime: Some(runtime),
            _exe_guard: exe_guard,
        }
    }

    fn close(&mut self) -> Result<Option<std::process::ExitStatus>, AcpError> {
        if let Some(mut rt) = self.runtime.take() {
            rt.close()
        } else {
            Ok(None)
        }
    }

    fn terminate(&mut self) -> Result<(), AcpError> {
        if let Some(mut rt) = self.runtime.take() {
            rt.terminate()
        } else {
            Ok(())
        }
    }
}

impl std::ops::Deref for TestRuntimeGuard {
    type Target = AcpRuntime<AcpChild>;
    fn deref(&self) -> &Self::Target {
        self.runtime.as_ref().expect("runtime active")
    }
}

impl std::ops::DerefMut for TestRuntimeGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.runtime.as_mut().expect("runtime active")
    }
}

impl Drop for TestRuntimeGuard {
    fn drop(&mut self) {
        let _ = self.runtime.take();
    }
}

fn spawn_runtime(scenario: &str) -> TestRuntimeGuard {
    TestRuntimeGuard::spawn(scenario)
}

#[test]
fn prompt_streaming_delivers_deltas_and_confirms_delivery() {
    let mut runtime = spawn_runtime("prompt_streaming");

    // 1. Initialize
    let capabilities = runtime.initialize("0.1.0").expect("initialize succeeds");
    assert!(capabilities.may_resume());
    assert!(!capabilities.supports_steer());

    // 2. Create session
    let session_id = runtime
        .new_session("/scratch/test", Vec::new())
        .expect("session created");
    assert_eq!(session_id, "mock-session-1");

    // 3. Prompt and stream deltas
    let mut streamed_text = String::new();
    let mut delta_count = 0;

    let stop_reason = runtime
        .prompt(
            vec![ContentBlock::Text {
                text: "Say hello".to_owned(),
            }],
            |event: NormalizedEvent| {
                if let AgentEvent::Delta { text } = event.event {
                    streamed_text.push_str(&text);
                    delta_count += 1;
                }
                Ok(())
            },
        )
        .expect("prompt turn succeeds");

    assert_eq!(stop_reason, StopReason::EndTurn);
    assert_eq!(streamed_text, "Hello World!");
    assert_eq!(delta_count, 3);
    assert_eq!(runtime.delivery().state(), DeliveryState::Confirmed);

    // Clean shutdown
    let _ = runtime.close();
}

#[test]
fn permission_flow_receives_request_and_settles() {
    let mut runtime = spawn_runtime("permission_flow");

    runtime.initialize("0.1.0").unwrap();
    runtime.new_session("/scratch", Vec::new()).unwrap();

    let mut permission_observed = false;
    let stop_reason = runtime
        .prompt_with_handlers(
            vec![ContentBlock::Text {
                text: "Run tool".to_owned(),
            }],
            |event: NormalizedEvent| {
                if let AgentEvent::PermissionRequested { ref request_id, .. } = event.event {
                    assert_eq!(request_id, "77");
                    permission_observed = true;
                }
                Ok(())
            },
            |_id, _tool_call| Ok(Some(serde_json::json!({ "outcome": "approved" }))),
            None,
        )
        .expect("permission flow turn succeeds");

    assert!(permission_observed, "permission request must be observed");
    assert_eq!(stop_reason, StopReason::EndTurn);
    assert_eq!(runtime.delivery().state(), DeliveryState::Confirmed);

    let _ = runtime.close();
}

#[test]
fn cancel_flow_cancels_turn_and_settles() {
    let mut runtime = spawn_runtime("cancel_flow");

    runtime.initialize("0.1.0").unwrap();
    runtime.new_session("/scratch", Vec::new()).unwrap();

    // Trigger cancel mid-stream via on_event returning Err(AcpError::Cancelled)
    let stop_reason = runtime
        .prompt(
            vec![ContentBlock::Text {
                text: "Long running task".to_owned(),
            }],
            |_| Err(AcpError::Cancelled),
        )
        .expect("cancelled turn completes");

    assert_eq!(stop_reason, StopReason::Cancelled);
    assert_eq!(runtime.delivery().state(), DeliveryState::Confirmed);

    let _ = runtime.close();
}

#[test]
fn malformed_frame_results_in_typed_error() {
    let mut runtime = spawn_runtime("malformed_frame");

    let err = runtime.initialize("0.1.0").unwrap_err();
    assert!(
        matches!(err, AcpError::MalformedMessage { .. }),
        "expected MalformedMessage, got {err:?}"
    );

    let _ = runtime.terminate();
}

#[test]
fn oversized_line_results_in_line_too_large_error() {
    let mut runtime = spawn_runtime("oversized_line");

    let err = runtime.initialize("0.1.0").unwrap_err();
    assert!(
        matches!(err, AcpError::LineTooLarge { .. }),
        "expected LineTooLarge, got {err:?}"
    );

    let _ = runtime.terminate();
}

#[test]
fn unexpected_exit_classifies_delivery_as_indeterminate() {
    let mut runtime = spawn_runtime("unexpected_exit");

    runtime.initialize("0.1.0").unwrap();
    runtime.new_session("/scratch", Vec::new()).unwrap();

    let err = runtime
        .prompt(
            vec![ContentBlock::Text {
                text: "Trigger crash".to_owned(),
            }],
            |_| Ok(()),
        )
        .unwrap_err();

    assert!(
        matches!(
            err,
            AcpError::UnexpectedEof { .. } | AcpError::ProcessExited { .. }
        ),
        "expected unexpected termination error, got {err:?}"
    );
    assert_eq!(runtime.delivery().state(), DeliveryState::Indeterminate);
    assert!(!runtime.delivery().may_resend());

    let _ = runtime.terminate();
}

#[test]
fn stderr_capture_is_bounded_and_does_not_overflow() {
    let mut runtime = spawn_runtime("stderr_capture_capped");

    runtime.initialize("0.1.0").unwrap();
    runtime.new_session("/scratch", Vec::new()).unwrap();

    runtime
        .prompt(
            vec![ContentBlock::Text {
                text: "Prompt".to_owned(),
            }],
            |_| Ok(()),
        )
        .unwrap();

    let captured = runtime.transport().captured_stderr();
    assert!(captured.starts_with("EEEE"));
    assert!(captured.contains("...[stderr truncated]"));
    assert!(
        captured.len() <= MAX_STDERR_CAPTURE_BYTES + 256,
        "captured stderr length {} exceeds buffer budget",
        captured.len()
    );

    let _ = runtime.close();
}

#[test]
fn graceful_close_terminates_cleanly() {
    let mut runtime = spawn_runtime("graceful_close");

    runtime.initialize("0.1.0").unwrap();
    let status = runtime.close().expect("close succeeds");
    assert!(status.is_some());
    assert!(status.unwrap().success());
}

#[test]
fn unknown_notifications_and_updates_preserve_without_panic() {
    let mut runtime = spawn_runtime("unknown_notification_preservation");

    runtime.initialize("0.1.0").unwrap();
    runtime.new_session("/scratch", Vec::new()).unwrap();

    let mut preserved_kinds = Vec::new();
    let stop_reason = runtime
        .prompt(
            vec![ContentBlock::Text {
                text: "Prompt with future features".to_owned(),
            }],
            |event: NormalizedEvent| {
                if let AgentEvent::Preserved {
                    ref provider_kind, ..
                } = event.event
                {
                    preserved_kinds.push(provider_kind.clone());
                }
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(stop_reason, StopReason::EndTurn);
    assert!(
        preserved_kinds
            .iter()
            .any(|k| k == "acp.notification.unmapped"),
        "top-level unknown notification must be preserved"
    );
    assert!(
        preserved_kinds
            .iter()
            .any(|k| k == "acp.update.neural_memory_checkpoint"),
        "unknown session update kind must be preserved"
    );

    let _ = runtime.close();
}

#[test]
fn launch_config_and_secret_resolver_integration_flow() {
    let exe_guard = TempExeGuard::new();
    let config = LaunchConfig::new(exe_guard.path_str())
        .unwrap()
        .with_literal_env("MODE", "test")
        .unwrap()
        .with_secret_env("API_KEY", SecretRef::new("vault-secret-key-1").unwrap())
        .unwrap();

    let resolver = |sref: &SecretRef| -> Result<String, AcpError> {
        if sref.as_str() == "vault-secret-key-1" {
            Ok("unmasked-secret-for-process-env".to_owned())
        } else {
            Err(AcpError::SecretResolutionFailed {
                secret_ref: sref.to_string(),
                diagnostic: "missing".to_owned(),
            })
        }
    };

    let resolved_cfg = config.resolve(&resolver).unwrap();

    // Verify debug representation never leaks the secret
    let debug_output = format!("{resolved_cfg:?}");
    assert!(!debug_output.contains("unmasked-secret-for-process-env"));
    assert!(debug_output.contains("[REDACTED]"));

    let child = AcpChild::spawn(&resolved_cfg).expect("child process spawns with resolved env");
    let mut runtime = AcpRuntime::new(child);
    let caps = runtime.initialize("0.1.0").expect("initialize succeeds");
    assert!(caps.accepts_text_prompts());
    let _ = runtime.close();
    drop(runtime);
    drop(exe_guard);
}
