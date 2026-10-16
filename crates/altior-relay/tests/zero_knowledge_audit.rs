//! ADR 0025 suite 3 — `test_relay_zero_knowledge_audit`, on the ADR 0027
//! v2 session layer.
//!
//! Runs a multi-turn sync exchange shaped like real traffic (turn text,
//! memory canary content, secret-shaped refs) through the relay, then
//! captures every byte the relay holds via `retained_frames` and asserts
//! none of the plaintext canaries appear anywhere — not in payloads, not
//! in push ids, not in the concatenation of all frames. A hostile relay
//! operator with total access to this state learns routing metadata and
//! opaque ciphertext, nothing else.

use altior_crypto::{
    CounterBlock, CounterLease, DeviceId, DeviceIdentity, DeviceKeys, SessionEpoch, SessionV2,
};
use altior_relay::{BucketId, Relay, RelayPolicy, RetainedFrame};

const ALICE_SEED: [u8; 64] = [1; 64];
const BOB_SEED: [u8; 64] = [2; 64];
const EPOCH_BYTES: [u8; 16] = [11; 16];
const TURN_SEED: [u8; 32] = [33; 32];

/// Canary plaintext shaped like real vault traffic. If any of these byte
/// sequences appear in relay-held state, zero knowledge is broken.
const CANARIES: [&str; 6] = [
    "my recovery phrase is never written down",
    "记住：深色主题与中文界面是我的偏好",
    "vault://recovery-key-0001",
    "thread title: quarterly reflection notes",
    "sk-canary-never-a-real-key-000000",
    "agent output: the summary of tonight's journal",
];

#[derive(Default)]
struct AuditLease {
    next: u64,
}

impl AuditLease {
    fn new() -> Self {
        Self { next: 1 }
    }
}

impl CounterLease for AuditLease {
    fn reserve(&mut self, block_size: u64) -> std::io::Result<CounterBlock> {
        let block = CounterBlock {
            start: self.next,
            end: self.next + block_size,
        };
        self.next = block.end;
        Ok(block)
    }
}

fn alice() -> DeviceKeys {
    DeviceKeys::from_seed(ALICE_SEED, DeviceId::new("alice-audit").unwrap()).unwrap()
}

fn bob_identity() -> DeviceIdentity {
    DeviceKeys::from_seed(BOB_SEED, DeviceId::new("bob-audit").unwrap())
        .unwrap()
        .public_identity()
}

#[test]
fn test_relay_zero_knowledge_audit() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    let alice_bucket = BucketId::new("bob-inbox");
    let bob_bucket = BucketId::new("alice-inbox");

    let mut out = SessionV2::outbound(
        &alice(),
        &bob_identity(),
        SessionEpoch::from_bytes(EPOCH_BYTES).unwrap(),
        1,
        &mut AuditLease::new(),
    )
    .unwrap();
    let mut inb = SessionV2::inbound(
        &bob_identity_keys(),
        &alice().public_identity(),
        SessionEpoch::from_bytes(EPOCH_BYTES).unwrap(),
        1,
    )
    .unwrap();

    // Multi-turn exchange shaped like sync traffic: every turn carries a
    // different canary, as real turns would carry real memories and
    // prompts.
    let batches: [&[&str]; 3] = [
        &["my recovery phrase is never written down", "turn text: today I wrote about the garden"],
        &["记住：深色主题与中文界面是我的偏好", "vault://recovery-key-0001"],
        &[
            "thread title: quarterly reflection notes",
            "sk-canary-never-a-real-key-000000",
            "agent output: the summary of tonight's journal",
        ],
    ];

    let mut delivered = 0;
    for (turn_no, batch) in batches.iter().enumerate() {
        if turn_no > 0 {
            out.open_turn(TURN_SEED).unwrap();
        }
        for line in *batch {
            let envelope = out.seal(line.as_bytes()).unwrap();
            let id = format!("env-{delivered:04}");
            delivered += 1;
            relay.push(&alice_bucket, &id, &envelope).expect("relay accepts the envelope");
        }
    }

    // The receiver fetches and opens every envelope; delivery works, so
    // any audit failure is not masking a broken channel.
    let page = relay.fetch(&alice_bucket, 0, 100);
    let altior_relay::FetchPage::Entries { items, .. } = page else {
        panic!("expected entries page");
    };
    assert_eq!(items.len(), delivered);
    for item in &items {
        let plaintext = inb.open(&item.payload).expect("receiver opens the envelope");
        assert_ne!(plaintext, Vec::<u8>::new());
    }

    // A second bucket also holding frames, so the audit covers all buckets.
    let envelope = out.seal(b"ack marker payload").unwrap();
    relay.push(&bob_bucket, "ack-0001", &envelope).unwrap();

    // ── The audit ────────────────────────────────────────────────────
    let frames = relay.retained_frames();
    assert_ne!(frames, Vec::<RetainedFrame>::new());
    let mut all_bytes: Vec<u8> = Vec::new();
    for frame in &frames {
        all_bytes.extend_from_slice(frame.payload.as_slice());
        all_bytes.extend_from_slice(frame.id.as_bytes());
        all_bytes.extend_from_slice(frame.bucket.as_str().as_bytes());
        for canary in CANARIES {
            assert!(
                !find_subslice(&frame.payload, canary.as_bytes()),
                "plaintext canary {canary:?} found in a relay payload"
            );
            assert!(
                !frame.id.contains(canary),
                "plaintext canary {canary:?} found in a relay push id"
            );
        }
        // Every payload is a v2 envelope: version byte checked, and the
        // body is ciphertext, not the plaintext that went in.
        assert_eq!(frame.payload[0], 2, "only sealed v2 envelopes transit");
        for canary in CANARIES {
            assert_ne!(
                &frame.payload[81..], canary.as_bytes(),
                "payload is the plaintext, not ciphertext"
            );
        }
    }
    for canary in CANARIES {
        assert!(
            !find_subslice(&all_bytes, canary.as_bytes()),
            "plaintext canary {canary:?} found anywhere in relay-held bytes"
        );
    }
}

fn bob_identity_keys() -> DeviceKeys {
    DeviceKeys::from_seed(BOB_SEED, DeviceId::new("bob-audit").unwrap()).unwrap()
}

/// Naive substring search over bytes (canaries are short; frames are
/// bounded); deterministic and allocation-light.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
