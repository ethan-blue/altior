//! DTO bindings export and deterministic post-processing.
//!
//! When the `dto-export` feature is active, this test exports all TypeScript
//! DTO definitions to `apps/desktop/src/ipc/dto/`, strips trailing whitespace
//! from every line, unifies newlines to LF (`\n`), and ensures a single
//! newline at EOF.
//!
//! Regeneration: `cargo test -p altior-protocol --features dto-export`

#![cfg(feature = "dto-export")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use ts_rs::TS;

use altior_protocol::{
    AgentProfileDto, BoundedPayload, CancelTurnCommand, CapabilityId, CapabilitySet,
    CapabilitySupport, CommandEnvelope, CommandKind, ConfigureAgentCommand, CoreGreeting,
    CoreHello, CreateThreadCommand, DesktopHello, DiagnosticsCommand, EventBody, EventEnvelope,
    GetHistoryCommand, HarnessBindingConfigDto, HarnessBindingDto, KnownEvent, LaunchToken,
    ListThreadsCommand, NegotiatedHandshake, OpenThreadCommand, PermissionDto, ProductVersion,
    ProtocolVersion, ProtocolVersionRange, RespondPermissionCommand, RetainedWindow,
    RuntimeDiagnosticsDto, RuntimeStatusCommand, SearchThreadsCommand, Sequence, SnapshotEnvelope,
    StartTurnCommand, TestHarnessBindingCommand, ThreadCursorDto, ThreadDto,
    ThreadHistoryResponseDto, ThreadListResponseDto, ThreadSnapshotDto, ThreadSummaryDto,
    TurnCursorDto, TurnDto,
};

fn dto_export_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/src/ipc/dto")
        .canonicalize()
        .expect("canonicalize dto export dir")
}

fn export_all_types() {
    let cfg = ts_rs::Config::default();
    LaunchToken::export_all(&cfg).expect("export LaunchToken");
    BoundedPayload::export_all(&cfg).expect("export BoundedPayload");
    CapabilityId::export_all(&cfg).expect("export CapabilityId");
    CapabilitySupport::export_all(&cfg).expect("export CapabilitySupport");
    CapabilitySet::export_all(&cfg).expect("export CapabilitySet");
    CommandKind::export_all(&cfg).expect("export CommandKind");
    CreateThreadCommand::export_all(&cfg).expect("export CreateThreadCommand");
    ListThreadsCommand::export_all(&cfg).expect("export ListThreadsCommand");
    SearchThreadsCommand::export_all(&cfg).expect("export SearchThreadsCommand");
    OpenThreadCommand::export_all(&cfg).expect("export OpenThreadCommand");
    GetHistoryCommand::export_all(&cfg).expect("export GetHistoryCommand");
    ConfigureAgentCommand::export_all(&cfg).expect("export ConfigureAgentCommand");
    TestHarnessBindingCommand::export_all(&cfg).expect("export TestHarnessBindingCommand");
    StartTurnCommand::export_all(&cfg).expect("export StartTurnCommand");
    CancelTurnCommand::export_all(&cfg).expect("export CancelTurnCommand");
    RespondPermissionCommand::export_all(&cfg).expect("export RespondPermissionCommand");
    RuntimeStatusCommand::export_all(&cfg).expect("export RuntimeStatusCommand");
    DiagnosticsCommand::export_all(&cfg).expect("export DiagnosticsCommand");
    CommandEnvelope::export_all(&cfg).expect("export CommandEnvelope");
    ThreadDto::export_all(&cfg).expect("export ThreadDto");
    TurnDto::export_all(&cfg).expect("export TurnDto");
    PermissionDto::export_all(&cfg).expect("export PermissionDto");
    AgentProfileDto::export_all(&cfg).expect("export AgentProfileDto");
    HarnessBindingConfigDto::export_all(&cfg).expect("export HarnessBindingConfigDto");
    HarnessBindingDto::export_all(&cfg).expect("export HarnessBindingDto");
    ThreadCursorDto::export_all(&cfg).expect("export ThreadCursorDto");
    TurnCursorDto::export_all(&cfg).expect("export TurnCursorDto");
    ThreadSummaryDto::export_all(&cfg).expect("export ThreadSummaryDto");
    ThreadSnapshotDto::export_all(&cfg).expect("export ThreadSnapshotDto");
    ThreadListResponseDto::export_all(&cfg).expect("export ThreadListResponseDto");
    ThreadHistoryResponseDto::export_all(&cfg).expect("export ThreadHistoryResponseDto");
    RuntimeDiagnosticsDto::export_all(&cfg).expect("export RuntimeDiagnosticsDto");
    KnownEvent::export_all(&cfg).expect("export KnownEvent");
    EventBody::export_all(&cfg).expect("export EventBody");
    Sequence::export_all(&cfg).expect("export Sequence");
    EventEnvelope::export_all(&cfg).expect("export EventEnvelope");
    RetainedWindow::export_all(&cfg).expect("export RetainedWindow");
    CoreGreeting::export_all(&cfg).expect("export CoreGreeting");
    DesktopHello::export_all(&cfg).expect("export DesktopHello");
    CoreHello::export_all(&cfg).expect("export CoreHello");
    NegotiatedHandshake::export_all(&cfg).expect("export NegotiatedHandshake");
    SnapshotEnvelope::export_all(&cfg).expect("export SnapshotEnvelope");
    ProtocolVersion::export_all(&cfg).expect("export ProtocolVersion");
    ProtocolVersionRange::export_all(&cfg).expect("export ProtocolVersionRange");
    ProductVersion::export_all(&cfg).expect("export ProductVersion");
}

fn post_process_dto_dir(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let entries = fs::read_dir(dir).expect("read dto dir");
    for entry in entries {
        let entry = entry.expect("valid dir entry");
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("ts") {
            let raw = fs::read_to_string(&path).expect("read ts file");
            let mut lines: Vec<&str> = Vec::new();
            for line in raw.split('\n') {
                let trimmed = line.trim_end_matches(['\r', ' ', '\t']);
                lines.push(trimmed);
            }
            let mut formatted = lines.join("\n");
            // Normalize trailing whitespace/newlines to exactly one '\n' at EOF.
            formatted.truncate(formatted.trim_end_matches(['\r', '\n', ' ', '\t']).len());
            formatted.push('\n');

            let bytes = formatted.into_bytes();
            fs::write(&path, &bytes).expect("write normalized ts file");
            files.insert(path, bytes);
        }
    }
    files
}

static EXPORT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn export_dto_bindings_and_post_process() {
    let _guard = EXPORT_LOCK.lock().unwrap();
    export_all_types();
    let dto_dir = dto_export_dir();
    let files = post_process_dto_dir(&dto_dir);
    assert!(!files.is_empty(), "expected exported .ts files in dto dir");

    for (path, bytes) in &files {
        let text = String::from_utf8(bytes.clone()).expect("valid utf-8");

        // Assert LF newlines and no CRLF.
        assert!(
            !text.contains('\r'),
            "file {path:?} contains carriage returns (CR)"
        );

        // Assert single newline at EOF.
        assert!(
            text.ends_with('\n'),
            "file {path:?} does not end with newline"
        );
        assert!(
            !text.ends_with("\n\n"),
            "file {path:?} ends with multiple blank lines"
        );

        // Assert no line has trailing whitespace.
        for (i, line) in text.lines().enumerate() {
            let line_num = i + 1;
            assert!(
                !line.ends_with(' ') && !line.ends_with('\t'),
                "file {path:?} line {line_num} has trailing whitespace: {line:?}"
            );
        }
    }
}

#[test]
fn dto_export_is_deterministic_and_idempotent() {
    let _guard = EXPORT_LOCK.lock().unwrap();
    let dto_dir = dto_export_dir();

    // First pass
    export_all_types();
    let first_pass = post_process_dto_dir(&dto_dir);

    // Second pass
    export_all_types();
    let second_pass = post_process_dto_dir(&dto_dir);

    assert_eq!(
        first_pass.len(),
        second_pass.len(),
        "number of exported files differed"
    );

    for (path, first_bytes) in &first_pass {
        let second_bytes = second_pass.get(path).expect("file present in second pass");
        assert_eq!(
            first_bytes, second_bytes,
            "file {path:?} differed between export passes"
        );
    }
}

fn has_bool_type_token(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|token| token == "bool")
}

#[test]
fn command_result_and_event_body_types_are_boolean_not_bool() {
    let _guard = EXPORT_LOCK.lock().unwrap();
    let dto_dir = dto_export_dir();
    export_all_types();
    post_process_dto_dir(&dto_dir);

    let event_body_path = dto_dir.join("EventBody.ts");
    let event_body = fs::read_to_string(&event_body_path).expect("read EventBody.ts");

    let known_event_path = dto_dir.join("KnownEvent.ts");
    let known_event = fs::read_to_string(&known_event_path).expect("read KnownEvent.ts");

    // EventBody assertions
    assert!(
        event_body.contains("success: boolean"),
        "EventBody.ts must contain `success: boolean`, got:\n{event_body}"
    );
    assert!(
        !has_bool_type_token(&event_body),
        "EventBody.ts must not contain `bool` type token, got:\n{event_body}"
    );

    // KnownEvent assertions
    assert!(
        known_event.contains("success: boolean"),
        "KnownEvent.ts must contain `success: boolean`, got:\n{known_event}"
    );
    assert!(
        !has_bool_type_token(&known_event),
        "KnownEvent.ts must not contain `bool` type token, got:\n{known_event}"
    );

    // Global check: no generated .ts file contains `bool` token
    let files = fs::read_dir(&dto_dir).expect("read dto dir");
    for entry in files {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("ts") {
            let content = fs::read_to_string(&path).expect("read file");
            assert!(
                !has_bool_type_token(&content),
                "file {path:?} contains `bool` token instead of `boolean`:\n{content}"
            );
        }
    }
}
