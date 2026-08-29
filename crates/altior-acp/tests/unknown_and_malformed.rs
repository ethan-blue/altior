//! P0.3 evidence: unknown events survive and malformed frames fail
//! explicitly (ADR 0007).
//!
//! Unknown *kinds* — session updates, methods, notifications — preserve
//! verbatim and never break normalization. Malformed *frames* — bad JSON,
//! wrong RPC envelope, missing discriminator, oversized or non-UTF-8
//! lines — fail with typed errors instead of being skipped or resynced.

use altior_acp::mapping::{self, AgentEvent};
use altior_acp::{AcpError, LineDecoder, MAX_LINE_BYTES, RpcMessage};

#[test]
fn unknown_kinds_preserve_and_never_break_normalization() {
    let events = mapping::normalize_trace([
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"future_update","payload":{"a":[1,2]},"_meta":{"x":1}}}}"#,
        r#"{"jsonrpc":"2.0","id":41,"method":"elicitation/create","params":{"schema":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"terminal/output","params":{"terminalId":"t1"}}"#,
    ])
    .unwrap();
    assert_eq!(events.len(), 3);
    for event in &events {
        let AgentEvent::Preserved { provider_kind, .. } = &event.event else {
            panic!("every unknown kind must preserve");
        };
        assert!(provider_kind.starts_with("acp."), "{provider_kind}");
    }
    assert_eq!(events[0].wire_kind, "session/update:future_update");
    assert!(matches!(
        &events[0].event,
        AgentEvent::Preserved { provider_kind, .. } if provider_kind == "acp.update.future_update"
    ));
    assert_eq!(events[1].wire_kind, "request:elicitation/create");
    assert_eq!(events[2].wire_kind, "notification:terminal/output");
}

#[test]
fn malformed_frames_fail_with_typed_errors() {
    let cases = [
        "not json at all",
        r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"content":{}}}}"#,
        r#"{"id":1,"result":{}}"#,
    ];
    for line in cases {
        let error = mapping::normalize_trace([line]).unwrap_err();
        assert!(
            matches!(error, AcpError::MalformedMessage { .. }),
            "{line} must fail as MalformedMessage, got {error}"
        );
    }
}

#[test]
fn oversized_and_non_utf8_lines_are_typed_stream_failures() {
    let oversized = "x".repeat(MAX_LINE_BYTES + 1);
    assert!(matches!(
        altior_acp::encode_line(&oversized),
        Err(AcpError::LineTooLarge { .. })
    ));
    let mut decoder = LineDecoder::new();
    assert!(matches!(
        decoder.feed(oversized.as_bytes()),
        Err(AcpError::LineTooLarge { .. })
    ));

    let mut utf8 = LineDecoder::new();
    assert!(matches!(
        utf8.feed(&[0xff, 0xfe, b'\n']),
        Err(AcpError::LineNotUtf8)
    ));
}

#[test]
fn unknown_content_block_kinds_inside_modeled_chunks_fail_explicitly() {
    // The five v1 content-block kinds are modeled; an agent sending a
    // sixth inside a modeled chunk violates the v1 shape and the whole
    // trace fails as evidence rather than silently dropping content.
    let error = mapping::normalize_trace([
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"hologram"}}}}"#,
    ])
    .unwrap_err();
    assert!(matches!(error, AcpError::MalformedMessage { .. }));
}

#[test]
fn responses_without_a_known_shape_preserve_instead_of_guessing() {
    let events =
        mapping::normalize_trace([r#"{"jsonrpc":"2.0","id":9,"result":{"anything":"else"}}"#])
            .unwrap();
    assert!(matches!(
        &events[0].event,
        AgentEvent::Preserved { provider_kind, .. } if provider_kind == "acp.result.unmapped"
    ));
    // Sanity for the wire decode path used above.
    assert!(matches!(
        RpcMessage::decode(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#),
        Ok(RpcMessage::Response { .. })
    ));
}
