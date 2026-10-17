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
    CapabilitySupport, CommandEnvelope, CommandKind, ConfigureAgentCommand, ConfirmMemoryCommand,
    CoreGreeting, CoreHello, CorrectMemoryCommand, CreateThreadCommand, DesktopHello,
    DiagnosticsCommand, EventBody, EventEnvelope, ForgetMemoryCommand, GetHistoryCommand,
    HarnessBindingConfigDto, HarnessBindingDto, HistoryCursorDto, HistoryEntryDto, KnownEvent,
    LaunchToken, ListMemoriesCommand, ListThreadsCommand, MemoryCursorDto, MemoryListResponseDto,
    MemoryRecordDto, NegotiatedHandshake, OpenThreadCommand, PermissionDto, ProductVersion,
    ProposeMemoryCommand, ProtocolVersion, ProtocolVersionRange, RejectMemoryCommand,
    RespondPermissionCommand, RetainedWindow, RuntimeDiagnosticsDto, RuntimeStatusCommand,
    SearchThreadsCommand, Sequence, SnapshotEnvelope, StartTurnCommand, TestHarnessBindingCommand,
    TestHarnessResponseDto, ThreadCursorDto, ThreadDto, ThreadHistoryResponseDto,
    ThreadListResponseDto, ThreadSnapshotDto, ThreadSummaryDto, TurnCursorDto, TurnDto,
};

fn dto_export_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/src/ipc/dto")
        .canonicalize()
        .expect("canonicalize dto export dir")
}

/// Windows ERROR 1224 ("user-mapped section open") is transient
/// interference from file indexers or scanners briefly holding a memory
/// section over the generated .ts files. A bounded retry rides it out;
/// any other error, or a mapper that will not release, fails honestly.
fn export_retrying<T: TS + 'static>(cfg: &ts_rs::Config) {
    const MAX_ATTEMPTS: u32 = 25;
    for attempt in 0..=MAX_ATTEMPTS {
        match T::export_all(cfg) {
            Ok(()) => return,
            Err(ts_rs::ExportError::Io(io))
                if io.raw_os_error() == Some(1224) && attempt < MAX_ATTEMPTS =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(other) => panic!("export {}: {other}", std::any::type_name::<T>()),
        }
    }
}

fn export_all_types() {
    let cfg = ts_rs::Config::default();
    export_retrying::<ListMemoriesCommand>(&cfg);
    export_retrying::<ProposeMemoryCommand>(&cfg);
    export_retrying::<ConfirmMemoryCommand>(&cfg);
    export_retrying::<RejectMemoryCommand>(&cfg);
    export_retrying::<CorrectMemoryCommand>(&cfg);
    export_retrying::<ForgetMemoryCommand>(&cfg);
    export_retrying::<MemoryCursorDto>(&cfg);
    export_retrying::<MemoryRecordDto>(&cfg);
    export_retrying::<MemoryListResponseDto>(&cfg);
    export_retrying::<LaunchToken>(&cfg);
    export_retrying::<BoundedPayload>(&cfg);
    export_retrying::<CapabilityId>(&cfg);
    export_retrying::<CapabilitySupport>(&cfg);
    export_retrying::<CapabilitySet>(&cfg);
    export_retrying::<CommandKind>(&cfg);
    export_retrying::<CreateThreadCommand>(&cfg);
    export_retrying::<ListThreadsCommand>(&cfg);
    export_retrying::<SearchThreadsCommand>(&cfg);
    export_retrying::<OpenThreadCommand>(&cfg);
    export_retrying::<GetHistoryCommand>(&cfg);
    export_retrying::<ConfigureAgentCommand>(&cfg);
    export_retrying::<TestHarnessBindingCommand>(&cfg);
    export_retrying::<StartTurnCommand>(&cfg);
    export_retrying::<CancelTurnCommand>(&cfg);
    export_retrying::<RespondPermissionCommand>(&cfg);
    export_retrying::<RuntimeStatusCommand>(&cfg);
    export_retrying::<DiagnosticsCommand>(&cfg);
    export_retrying::<CommandEnvelope>(&cfg);
    export_retrying::<ThreadDto>(&cfg);
    export_retrying::<TurnDto>(&cfg);
    export_retrying::<PermissionDto>(&cfg);
    export_retrying::<AgentProfileDto>(&cfg);
    export_retrying::<HarnessBindingConfigDto>(&cfg);
    export_retrying::<HarnessBindingDto>(&cfg);
    export_retrying::<TestHarnessResponseDto>(&cfg);
    export_retrying::<ThreadCursorDto>(&cfg);
    export_retrying::<TurnCursorDto>(&cfg);
    export_retrying::<ThreadSummaryDto>(&cfg);
    export_retrying::<ThreadSnapshotDto>(&cfg);
    export_retrying::<ThreadListResponseDto>(&cfg);
    export_retrying::<ThreadHistoryResponseDto>(&cfg);
    export_retrying::<HistoryEntryDto>(&cfg);
    export_retrying::<HistoryCursorDto>(&cfg);
    export_retrying::<RuntimeDiagnosticsDto>(&cfg);
    export_retrying::<KnownEvent>(&cfg);
    export_retrying::<EventBody>(&cfg);
    export_retrying::<Sequence>(&cfg);
    export_retrying::<EventEnvelope>(&cfg);
    export_retrying::<RetainedWindow>(&cfg);
    export_retrying::<CoreGreeting>(&cfg);
    export_retrying::<DesktopHello>(&cfg);
    export_retrying::<CoreHello>(&cfg);
    export_retrying::<NegotiatedHandshake>(&cfg);
    export_retrying::<SnapshotEnvelope>(&cfg);
    export_retrying::<ProtocolVersion>(&cfg);
    export_retrying::<ProtocolVersionRange>(&cfg);
    export_retrying::<ProductVersion>(&cfg);
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
