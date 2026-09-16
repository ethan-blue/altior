//! Desktop→Core command contract fixtures (review task A01).
//!
//! The `command-desktop-*.json` fixtures capture the exact envelope shapes
//! the Desktop application store emits after A01, decoded against the real
//! serde contracts so the client and protocol cannot drift apart silently.
//! The `command-invalid-*.json` fixtures preserve the historical invalid
//! shapes the store used to emit (review finding F01) as durable rejection
//! evidence: they must keep failing to decode, so nobody "fixes" the domain
//! validator to accept broken clients again.
//!
//! Fixture content is synthetic only: fixed identifiers, Chinese sample
//! text, and no real transcripts or credentials.

use altior_protocol::{CommandEnvelope, EnvelopeLimits, ProtocolError};

fn limits() -> EnvelopeLimits {
    EnvelopeLimits::default()
}

macro_rules! desktop_fixture {
    ($name:literal) => {
        include_str!(concat!("../fixtures/", $name))
    };
}

/// Every store-emitted command kind with its fixture file, in store order.
const DESKTOP_FIXTURES: &[(&str, &str)] = &[
    (
        "list_threads",
        desktop_fixture!("command-desktop-list-threads-v1.json"),
    ),
    (
        "runtime_status",
        desktop_fixture!("command-desktop-runtime-status-v1.json"),
    ),
    (
        "diagnostics",
        desktop_fixture!("command-desktop-diagnostics-v1.json"),
    ),
    (
        "open_thread",
        desktop_fixture!("command-desktop-open-thread-v1.json"),
    ),
    (
        "get_history",
        desktop_fixture!("command-desktop-get-history-v1.json"),
    ),
    (
        "search_threads",
        desktop_fixture!("command-desktop-search-threads-v1.json"),
    ),
    (
        "create_thread",
        desktop_fixture!("command-desktop-create-thread-v1.json"),
    ),
    (
        "configure_agent",
        desktop_fixture!("command-desktop-configure-agent-v1.json"),
    ),
    (
        "test_harness_binding",
        desktop_fixture!("command-desktop-test-harness-binding-v1.json"),
    ),
    (
        "start_turn",
        desktop_fixture!("command-desktop-start-turn-v1.json"),
    ),
    (
        "cancel_turn",
        desktop_fixture!("command-desktop-cancel-turn-v1.json"),
    ),
    (
        "respond_permission",
        desktop_fixture!("command-desktop-respond-permission-v1.json"),
    ),
    (
        "get_context_snapshot",
        desktop_fixture!("command-desktop-get-context-snapshot-v1.json"),
    ),
    (
        "list_identity_documents",
        desktop_fixture!("command-desktop-list-identity-documents-v1.json"),
    ),
    (
        "put_identity_document",
        desktop_fixture!("command-desktop-put-identity-document-v1.json"),
    ),
    (
        "delete_identity_document",
        desktop_fixture!("command-desktop-delete-identity-document-v1.json"),
    ),
];

/// Historical invalid shapes (review finding F01): each fixture must fail
/// to decode at the envelope or typed-payload boundary.
const INVALID_FIXTURES: &[&str] = &[
    desktop_fixture!("command-invalid-legacy-operation-id-v1.json"),
    desktop_fixture!("command-invalid-client-agent-id-v1.json"),
    desktop_fixture!("command-invalid-client-turn-id-v1.json"),
    desktop_fixture!("command-invalid-client-binding-id-v1.json"),
    desktop_fixture!("command-invalid-client-thread-id-v1.json"),
    desktop_fixture!("command-invalid-underscores-in-operation-id-v1.json"),
];

#[test]
fn every_desktop_emission_fixture_decodes_and_validates() {
    for (kind, json) in DESKTOP_FIXTURES {
        let envelope = CommandEnvelope::from_json(json)
            .unwrap_or_else(|e| panic!("fixture {kind} must decode at the envelope boundary: {e}"));
        envelope.validate(&limits()).unwrap_or_else(|e| {
            panic!("fixture {kind} must validate against envelope limits: {e}")
        });

        // The typed payload must decode per kind; malformed payloads are a
        // fixture bug, not a contract tolerance.
        match envelope.kind.wire_name() {
            "list_threads" => {
                envelope.list_threads_payload().unwrap();
            }
            "runtime_status" => {
                envelope.runtime_status_payload().unwrap();
            }
            "diagnostics" => {
                envelope.diagnostics_payload().unwrap();
            }
            "open_thread" => {
                envelope.open_thread_payload().unwrap();
            }
            "get_history" => {
                envelope.get_history_payload().unwrap();
            }
            "search_threads" => {
                envelope.search_threads_payload().unwrap();
            }
            "create_thread" => {
                envelope.create_thread_payload().unwrap();
            }
            "configure_agent" => {
                let payload = envelope.configure_agent_payload().unwrap();
                payload.validate().unwrap();
            }
            "test_harness_binding" => {
                let payload = envelope.test_harness_binding_payload().unwrap();
                payload.validate().unwrap();
            }
            "start_turn" => {
                envelope.start_turn_payload().unwrap();
            }
            "cancel_turn" => {
                envelope.cancel_turn_payload().unwrap();
            }
            "respond_permission" => {
                envelope.respond_permission_payload().unwrap();
            }
            "get_context_snapshot" => {
                let payload = envelope.get_context_snapshot_payload().unwrap();
                payload.validate().unwrap();
            }
            "list_identity_documents" => {
                let payload = envelope.list_identity_documents_payload().unwrap();
                payload.validate().unwrap();
            }
            "put_identity_document" => {
                let payload = envelope.put_identity_document_payload().unwrap();
                payload.validate().unwrap();
            }
            "delete_identity_document" => {
                let payload = envelope.delete_identity_document_payload().unwrap();
                payload.validate().unwrap();
            }
            other => panic!("unexpected fixture kind {other}"),
        }

        // Re-encoding must stay canonical: the output re-decodes to the
        // same envelope.
        let canonical = envelope.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&canonical).unwrap();
        assert_eq!(decoded, envelope, "fixture {kind} must roundtrip");
    }
}

#[test]
fn desktop_start_turn_fixture_mints_the_turn_on_core() {
    // A01: the client passes turn_id: null and Core allocates the real
    // identity; the client must never guess one.
    let envelope = CommandEnvelope::from_json(DESKTOP_FIXTURES[9].1).unwrap();
    assert_eq!(envelope.kind.wire_name(), "start_turn");
    let payload = envelope.start_turn_payload().unwrap();
    assert!(payload.turn_id.is_none());
    assert!(payload.prompt.as_str().contains("文档"));
}

#[test]
fn desktop_configure_agent_fixture_asks_core_for_identity() {
    let envelope = CommandEnvelope::from_json(DESKTOP_FIXTURES[7].1).unwrap();
    let payload = envelope.configure_agent_payload().unwrap();
    assert!(payload.agent_profile_id.is_none());
    let binding = payload.binding.expect("binding present");
    assert!(binding.harness_binding_id.is_none());
    assert!(binding.agent_profile_id.is_none());
    // Spaces in program paths survive verbatim (no shell re-tokenization).
    assert!(binding.program.contains(' '));
    assert_eq!(binding.env_keys.len(), binding.secret_refs.len());
}

#[test]
fn desktop_create_thread_fixture_carries_chinese_title() {
    let envelope = CommandEnvelope::from_json(DESKTOP_FIXTURES[6].1).unwrap();
    let payload = envelope.create_thread_payload().unwrap();
    assert_eq!(payload.title.as_deref(), Some("中文会话标题"));
    assert_eq!(payload.agent_profile_id.as_str(), "agp_fixture000000001");
    assert!(payload.project_id.is_none());
}

#[test]
fn historical_invalid_desktop_shapes_stay_rejected() {
    for json in INVALID_FIXTURES {
        // The whole envelope may fail (bad operation ids in the typed
        // envelope field)...
        let failure = match CommandEnvelope::from_json(json) {
            Err(_) => true,
            Ok(envelope) => {
                // ...or the envelope decodes but the typed payload must
                // not.
                let payload_err: Result<(), ProtocolError> = match envelope.kind.wire_name() {
                    "start_turn" => envelope.start_turn_payload().map(|_| ()),
                    "create_thread" => envelope.create_thread_payload().map(|_| ()),
                    "open_thread" => envelope.open_thread_payload().map(|_| ()),
                    "list_threads" => envelope.list_threads_payload().map(|_| ()),
                    "configure_agent" => envelope.configure_agent_payload().map(|_| ()),
                    other => panic!("unexpected invalid fixture kind {other}"),
                };
                payload_err.is_err()
            }
        };
        assert!(
            failure,
            "historical invalid shape must be rejected, but decoded cleanly: {json}"
        );
    }
}

#[test]
fn invalid_binding_id_fails_at_the_typed_payload_boundary() {
    // The bin_alpha_01 shape is JSON-shaped but must fail HarnessBindingId
    // validation during typed payload decode — the exact failure that made
    // the old onboarding flow a fake success against real Core.
    let json = INVALID_FIXTURES[3];
    let envelope = CommandEnvelope::from_json(json).expect("envelope itself is decodable");
    assert_eq!(envelope.kind.wire_name(), "configure_agent");
    assert!(
        envelope.configure_agent_payload().is_err(),
        "binding id must fail typed payload decoding"
    );
}
