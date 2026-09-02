//! Compatibility harness over the checked-in synthetic protocol fixtures.
//!
//! Fixtures are durable contract evidence (`docs/AI_DEVELOPMENT.md`):
//! they contain only synthetic identifiers and values, no real user
//! transcripts, credentials, or machine data. Every fixture must decode
//! with the current contract and re-encode to the same canonical JSON, so
//! accidental wire-format drift fails this suite.

use altior_protocol::{
    CapabilitySet, CommandEnvelope, CoreGreeting, CoreHello, DesktopHello, EnvelopeLimits,
    EventBody, EventEnvelope, KnownEvent, ProductVersion, ProtocolVersion, ProtocolVersionRange,
    Sequence, SnapshotEnvelope, negotiate,
};

const DESKTOP_HELLO: &str = include_str!("../fixtures/handshake-desktop-hello-v1.json");
const CORE_HELLO: &str = include_str!("../fixtures/handshake-core-hello-v1.json");
const NEGOTIATED: &str = include_str!("../fixtures/handshake-negotiated-v1.json");
const COMPAT_DESKTOP_NEWER: &str =
    include_str!("../fixtures/handshake-compat-desktop-newer-v1.json");
const COMPAT_CORE_NEWER: &str = include_str!("../fixtures/handshake-compat-core-newer-v1.json");
const COMMAND_PING: &str = include_str!("../fixtures/command-ping-v1.json");
const COMMAND_CANCEL: &str = include_str!("../fixtures/command-cancel-v1.json");
const COMMAND_CONFIGURE_AGENT: &str = include_str!("../fixtures/command-configure-agent-v1.json");
const COMMAND_CONFIGURE_AGENT_WITH_BINDING: &str =
    include_str!("../fixtures/command-configure-agent-with-binding-v1.json");
const COMMAND_TEST_HARNESS: &str = include_str!("../fixtures/command-test-harness-v1.json");
const COMMAND_SUBSCRIBE: &str = include_str!("../fixtures/command-subscribe-v1.json");
const SNAPSHOT_THREAD: &str = include_str!("../fixtures/snapshot-thread-v1.json");
const GREETING_CORE: &str = include_str!("../fixtures/greeting-core-v1.json");
const EVENT_TURN_STARTED: &str = include_str!("../fixtures/event-turn-started-v1.json");
const EVENT_STREAM_GAP: &str = include_str!("../fixtures/event-stream-gap-v1.json");
const EVENT_STREAM_REPLAYED: &str = include_str!("../fixtures/event-stream-replayed-v1.json");
const EVENT_UNKNOWN_FUTURE: &str = include_str!("../fixtures/event-unknown-future-v1.json");
const EVENT_UNKNOWN_PRESERVED: &str = include_str!("../fixtures/event-unknown-preserved-v1.json");

#[test]
fn fixture_handshake_negotiates_to_the_checked_in_result() {
    let desktop: DesktopHello = serde_json::from_str(DESKTOP_HELLO).unwrap();
    let core: CoreHello = serde_json::from_str(CORE_HELLO).unwrap();
    let negotiated = negotiate(&desktop, &core).unwrap();
    assert_eq!(
        serde_json::to_string(&negotiated).unwrap(),
        NEGOTIATED.trim()
    );
    // Re-encoding the hello fixtures must reproduce them byte for byte:
    // canonical field order and sorted map keys.
    assert_eq!(
        serde_json::to_string(&desktop).unwrap(),
        DESKTOP_HELLO.trim()
    );
    assert_eq!(serde_json::to_string(&core).unwrap(), CORE_HELLO.trim());
}

#[test]
fn fixture_command_envelope_roundtrips_and_validates() {
    let envelope = CommandEnvelope::from_json(COMMAND_PING).unwrap();
    envelope
        .validate(&altior_protocol::EnvelopeLimits::default())
        .unwrap();
    assert_eq!(envelope.to_json().unwrap(), COMMAND_PING.trim());
    // Deterministic encoding: a second pass produces the same bytes.
    assert_eq!(envelope.to_json().unwrap(), envelope.to_json().unwrap());
}

#[test]
fn fixture_known_event_envelope_roundtrips_and_validates() {
    let envelope = EventEnvelope::from_json(EVENT_TURN_STARTED).unwrap();
    envelope
        .validate(&altior_protocol::EnvelopeLimits::default())
        .unwrap();
    assert_eq!(envelope.to_json().unwrap(), EVENT_TURN_STARTED.trim());
}

#[test]
fn fixture_stream_control_events_roundtrip_and_validate() {
    let gap = EventEnvelope::from_json(EVENT_STREAM_GAP).unwrap();
    gap.validate(&EnvelopeLimits::default()).unwrap();
    assert!(matches!(
        gap.body,
        EventBody::Known(KnownEvent::StreamGap { ref from }) if from.as_u64() == 3
    ));
    assert_eq!(gap.to_json().unwrap(), EVENT_STREAM_GAP.trim());

    let replayed = EventEnvelope::from_json(EVENT_STREAM_REPLAYED).unwrap();
    replayed.validate(&EnvelopeLimits::default()).unwrap();
    assert!(matches!(
        replayed.body,
        EventBody::Known(KnownEvent::StreamReplayed {
            ref from,
            ref through
        }) if from.as_u64() == 6 && through.as_u64() == 9
    ));
    assert_eq!(replayed.to_json().unwrap(), EVENT_STREAM_REPLAYED.trim());
}

#[test]
fn fixture_cancel_command_roundtrips_and_targets_its_operation() {
    let envelope = CommandEnvelope::from_json(COMMAND_CANCEL).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    assert_eq!(
        envelope.cancel_target().unwrap().unwrap().as_str(),
        "op_fixture000000005"
    );
    assert_eq!(envelope.to_json().unwrap(), COMMAND_CANCEL.trim());
}

#[test]
fn fixture_configure_agent_without_binding_roundtrips_and_validates() {
    let envelope = CommandEnvelope::from_json(COMMAND_CONFIGURE_AGENT).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    let payload = envelope.configure_agent_payload().unwrap();
    payload.validate().unwrap();
    assert_eq!(payload.display_name, "Coding Assistant");
    assert!(payload.binding.is_none());
    assert_eq!(envelope.to_json().unwrap(), COMMAND_CONFIGURE_AGENT.trim());
}

#[test]
fn fixture_configure_agent_with_binding_roundtrips_and_validates() {
    let envelope = CommandEnvelope::from_json(COMMAND_CONFIGURE_AGENT_WITH_BINDING).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    let payload = envelope.configure_agent_payload().unwrap();
    payload.validate().unwrap();
    assert_eq!(payload.display_name, "Coding Assistant");
    let binding = payload.binding.expect("expected binding payload");
    assert_eq!(binding.program, "C:\\Agents\\acp-agent.exe");
    assert_eq!(binding.args, vec!["--acp"]);
    assert_eq!(binding.env_keys, vec!["ANTHROPIC_API_KEY"]);
    assert_eq!(binding.secret_refs, vec!["sec_ref_openai_01"]);
    assert_eq!(binding.label.as_deref(), Some("Local ACP Agent"));
    assert_eq!(
        envelope.to_json().unwrap(),
        COMMAND_CONFIGURE_AGENT_WITH_BINDING.trim()
    );
}

#[test]
fn fixture_test_harness_command_roundtrips_and_validates() {
    let envelope = CommandEnvelope::from_json(COMMAND_TEST_HARNESS).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    let payload = envelope.test_harness_binding_payload().unwrap();
    payload.validate().unwrap();
    assert_eq!(payload.program, "C:\\Agents\\acp-agent.exe");
    assert_eq!(payload.args, vec!["--acp"]);
    assert_eq!(payload.env_keys, vec!["PATH"]);
    assert_eq!(payload.secret_refs, vec!["sec_ref_openai_01"]);
    assert_eq!(payload.label.as_deref(), Some("Local ACP Agent"));
    assert_eq!(envelope.to_json().unwrap(), COMMAND_TEST_HARNESS.trim());
}

#[test]
fn fixture_subscribe_command_roundtrips_and_carries_its_catch_up_point() {
    let envelope = CommandEnvelope::from_json(COMMAND_SUBSCRIBE).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    assert_eq!(
        envelope.subscribe_since().unwrap(),
        Some(Some(Sequence::try_new(5).unwrap()))
    );
    assert_eq!(envelope.to_json().unwrap(), COMMAND_SUBSCRIBE.trim());
}

#[test]
fn fixture_greeting_roundtrips_and_validates() {
    let greeting = CoreGreeting::from_json(GREETING_CORE).unwrap();
    greeting.validate().unwrap();
    assert_eq!(greeting.instance_id.as_str(), "cor_fixture000000009");
    let retained = greeting.retained.expect("fixture retains a window");
    assert_eq!(retained.from.as_u64(), 4);
    assert_eq!(retained.through.as_u64(), 9);
    assert_eq!(greeting.to_json().unwrap(), GREETING_CORE.trim());
}

#[test]
fn fixture_snapshot_envelope_roundtrips_and_validates() {
    let envelope = SnapshotEnvelope::from_json(SNAPSHOT_THREAD).unwrap();
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    let thread_id = envelope.thread_id.as_ref().map(|id| id.as_str().to_owned());
    assert_eq!(thread_id, Some("thr_fixture000000001".to_owned()));
    assert_eq!(envelope.to_json().unwrap(), SNAPSHOT_THREAD.trim());
}

#[test]
fn old_and_new_handshake_versions_interoperate_at_the_common_version() {
    // Compatibility vectors for the negotiation algorithm (ADR 0004
    // migration rule): a newer endpoint advertising a range that still
    // includes 1 must connect to a v1-only peer at 1 — neither failing
    // nor silently speaking above what the older side implements.
    let v1_only_range =
        ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1).unwrap();

    // Newer Desktop meets older Core.
    let newer_desktop: DesktopHello = serde_json::from_str(COMPAT_DESKTOP_NEWER).unwrap();
    let older_core = CoreHello {
        supported_versions: v1_only_range,
        core_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
    };
    assert_eq!(
        negotiate(&newer_desktop, &older_core)
            .unwrap()
            .selected_version
            .as_u32(),
        1
    );

    // Older Desktop meets newer Core.
    let older_desktop = DesktopHello {
        supported_versions: v1_only_range,
        desktop_version: ProductVersion::new(0, 1, 0),
        capabilities: CapabilitySet::new(),
        launch_token: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".parse().unwrap(),
    };
    let newer_core: CoreHello = serde_json::from_str(COMPAT_CORE_NEWER).unwrap();
    assert_eq!(
        negotiate(&older_desktop, &newer_core)
            .unwrap()
            .selected_version
            .as_u32(),
        1
    );

    // The fixtures themselves stay canonical.
    assert_eq!(
        serde_json::to_string(&newer_desktop).unwrap(),
        COMPAT_DESKTOP_NEWER.trim()
    );
    assert_eq!(
        serde_json::to_string(&newer_core).unwrap(),
        COMPAT_CORE_NEWER.trim()
    );
}

#[test]
fn fixture_unknown_future_event_is_preserved_not_rejected() {
    // The fixture keeps the raw provider shape; the contract preserves it
    // as a bounded diagnostic instead of failing or discarding it.
    let envelope = EventEnvelope::from_json(EVENT_UNKNOWN_FUTURE).unwrap();
    envelope
        .validate(&altior_protocol::EnvelopeLimits::default())
        .unwrap();
    let EventBody::Unknown {
        provider_kind,
        diagnostic,
    } = &envelope.body
    else {
        panic!("expected an unknown event body");
    };
    assert_eq!(provider_kind, "usage.updated");
    assert!(diagnostic.as_str().contains("input_tokens"));

    // Preservation is stable: once encoded in the preserved shape,
    // further encode/decode passes are fixed points.
    let once = envelope.to_json().unwrap();
    let decoded = EventEnvelope::from_json(&once).unwrap();
    let twice = decoded.to_json().unwrap();
    assert_eq!(once, twice);
    assert_eq!(decoded, envelope);
}

#[test]
fn fixture_unknown_preserved_event_roundtrips_byte_for_byte() {
    // A provider event already encoded in the preserved shape decodes and
    // re-encodes to identical bytes: the preserved shape is a fixed point.
    // This is the exact wire form Core re-emits after wrapping a raw
    // unknown event, and therefore the form the Desktop fixture shell
    // replays (`apps/desktop/src/ipc/fixtures.ts`).
    let envelope = EventEnvelope::from_json(EVENT_UNKNOWN_PRESERVED).unwrap();
    envelope
        .validate(&altior_protocol::EnvelopeLimits::default())
        .unwrap();
    let EventBody::Unknown {
        provider_kind,
        diagnostic,
    } = &envelope.body
    else {
        panic!("expected an unknown event body");
    };
    assert_eq!(provider_kind, "usage.stats.snapshot");
    assert!(diagnostic.as_str().contains("input_tokens"));
    assert_eq!(envelope.to_json().unwrap(), EVENT_UNKNOWN_PRESERVED.trim());
}
