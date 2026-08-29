//! Deterministic negotiation and envelope-boundary scenarios required by
//! the P0.1 acceptance evidence. All values are synthetic constants.

use altior_protocol::{
    CapabilitySet, CapabilitySupport, CommandEnvelope, CommandKind, CoreHello, DesktopHello,
    EnvelopeLimits, ProtocolError, ProtocolVersion, ProtocolVersionRange, negotiate,
};
use std::str::FromStr;

fn range(min: u32, max: u32) -> ProtocolVersionRange {
    ProtocolVersionRange::try_new(
        ProtocolVersion::try_new(min).unwrap(),
        ProtocolVersion::try_new(max).unwrap(),
    )
    .unwrap()
}

fn hello(
    supported: ProtocolVersionRange,
    capabilities: CapabilitySet,
) -> (DesktopHello, CoreHello) {
    (
        DesktopHello {
            supported_versions: supported,
            desktop_version: "0.2.0".parse().unwrap(),
            capabilities: capabilities.clone(),
            launch_token: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".parse().unwrap(),
        },
        CoreHello {
            supported_versions: supported,
            core_version: "0.1.0".parse().unwrap(),
            capabilities,
        },
    )
}

#[test]
fn versions_negotiate_by_intersection_not_equality() {
    // Not a single equal number: overlapping ranges pick the highest
    // common version.
    let (mut desktop, mut core) = hello(range(1, 1), CapabilitySet::new());
    desktop.supported_versions = range(1, 4);
    core.supported_versions = range(3, 7);
    let negotiated = negotiate(&desktop, &core).unwrap();
    assert_eq!(negotiated.selected_version.as_u32(), 4);

    desktop.supported_versions = range(2, 6);
    core.supported_versions = range(1, 3);
    let negotiated = negotiate(&desktop, &core).unwrap();
    assert_eq!(negotiated.selected_version.as_u32(), 3);
}

#[test]
fn disjoint_ranges_fail_explicitly() {
    let (mut desktop, mut core) = hello(range(1, 1), CapabilitySet::new());
    desktop.supported_versions = range(1, 2);
    core.supported_versions = range(5, 9);
    assert!(matches!(
        negotiate(&desktop, &core),
        Err(ProtocolError::NoCommonProtocolVersion { .. })
    ));
}

#[test]
fn capabilities_are_negotiated_explicitly_and_never_inferred() {
    let mut desktop_caps = CapabilitySet::new();
    desktop_caps
        .declare("event.streaming", CapabilitySupport::Supported)
        .unwrap();
    desktop_caps
        .declare("thread.steering", CapabilitySupport::Supported)
        .unwrap();
    desktop_caps
        .declare("usage.reporting", CapabilitySupport::Unsupported)
        .unwrap();
    let mut core_caps = CapabilitySet::new();
    core_caps
        .declare("event.streaming", CapabilitySupport::Supported)
        .unwrap();
    // Core knows `thread.steering` but does not implement it.
    core_caps
        .declare("thread.steering", CapabilitySupport::Unsupported)
        .unwrap();
    // Core supports a capability Desktop has never heard of.
    core_caps
        .declare("tool.permissions", CapabilitySupport::Supported)
        .unwrap();

    let (desktop, core) = hello(range(1, 1), CapabilitySet::new());
    let desktop = DesktopHello {
        capabilities: desktop_caps,
        ..desktop
    };
    let core = CoreHello {
        capabilities: core_caps,
        ..core
    };
    let negotiated = negotiate(&desktop, &core).unwrap();

    // Only the both-supported capability is negotiated.
    assert!(
        negotiated
            .negotiated_capabilities
            .get(&altior_protocol::CapabilityId::from_str("event.streaming").unwrap())
            .is_some()
    );
    assert_eq!(negotiated.negotiated_capabilities.len(), 1);
    // One-sided claims are surfaced for diagnostics.
    let desktop_only: Vec<String> = negotiated
        .desktop_only
        .iter()
        .map(ToString::to_string)
        .collect();
    let core_only: Vec<String> = negotiated
        .core_only
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(desktop_only, ["thread.steering"]);
    assert_eq!(core_only, ["tool.permissions"]);
}

#[test]
fn oversized_payloads_are_rejected_at_the_boundary() {
    let payload =
        altior_protocol::BoundedPayload::new(serde_json::json!({"blob": "x".repeat(2048)}), 4096)
            .unwrap();
    let envelope = CommandEnvelope {
        protocol_version: ProtocolVersion::V1,
        operation_id: "op_fixture000000005".parse().unwrap(),
        kind: CommandKind::Ping,
        payload: Some(payload),
        issued_at: altior_domain::UnixMillis::from_millis(1_700_000_000_000),
    };
    // Within the default limit the envelope validates.
    envelope.validate(&EnvelopeLimits::default()).unwrap();
    // A tightened limit rejects it through the validation entry point.
    let strict = EnvelopeLimits {
        payload_bytes: 1024,
        diagnostic_bytes: 4096,
    };
    assert!(matches!(
        envelope.validate(&strict),
        Err(ProtocolError::PayloadTooLarge {
            size_bytes,
            limit_bytes: 1024
        }) if size_bytes > 1024
    ));
    // And construction itself refuses payloads over their immediate cap.
    let error =
        altior_protocol::BoundedPayload::new(serde_json::json!({"blob": "x".repeat(2048)}), 1024)
            .unwrap_err();
    assert!(matches!(
        error,
        ProtocolError::PayloadTooLarge {
            limit_bytes: 1024,
            ..
        }
    ));
}

#[test]
fn unknown_command_kinds_fail_explicitly() {
    assert!(matches!(
        CommandKind::from_str("prompt.thread"),
        Err(ProtocolError::UnsupportedCommandKind { ref kind }) if kind == "prompt.thread"
    ));
    let json = r#"{"protocol_version":1,"operation_id":"op_fixture000000005","kind":"prompt.thread","payload":null,"issued_at":0}"#;
    assert!(CommandEnvelope::from_json(json).is_err());
}
