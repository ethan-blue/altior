//! ADR 0025 threat 7 (relay exhaustion) enforcement: metadata, payload,
//! depth, and byte-quota bounds all refuse whole, leaving the bucket
//! untouched.

use altior_relay::{BucketId, PushOutcome, Relay, RelayError, RelayPolicy};

#[test]
fn test_relay_enforces_adr_0025_resource_bounds() {
    // The production preset carries the ADR 0025 constants.
    let production = RelayPolicy::production();
    assert_eq!(production.max_payload_bytes, 1024 * 1024);
    assert_eq!(production.max_push_id_bytes, 4 * 1024);
    assert_eq!(production.max_bucket_bytes, 50 * 1024 * 1024);
    assert_eq!(production.max_age_ticks, 86_400 * 14);

    // A tight policy to exercise every bound without allocating megabytes.
    let mut relay = Relay::new(RelayPolicy {
        max_payload_bytes: 64,
        max_bucket_depth: 4,
        max_age_ticks: 100,
        max_push_id_bytes: 8,
        max_bucket_bytes: 128,
    });
    let bucket = BucketId::new("bounded");

    // Payload bound.
    let big = [0u8; 65];
    assert!(matches!(
        relay.push(&bucket, "ok-id", &big),
        Err(RelayError::PushTooLarge {
            size_bytes: 65,
            limit_bytes: 64
        })
    ));

    // Metadata bound.
    assert!(matches!(
        relay.push(&bucket, "nine-byte", &[0u8; 8]),
        Err(RelayError::PushIdTooLarge {
            size_bytes: 9,
            limit_bytes: 8
        })
    ));

    // Byte quota: 64-byte payloads; the third push would exceed 128.
    assert_eq!(
        relay.push(&bucket, "one", &[1u8; 64]).unwrap(),
        PushOutcome::Pushed { seq: 1 }
    );
    assert_eq!(
        relay.push(&bucket, "two", &[2u8; 64]).unwrap(),
        PushOutcome::Pushed { seq: 2 }
    );
    assert!(matches!(
        relay.push(&bucket, "three", &[3u8; 64]),
        Err(RelayError::BucketQuotaExceeded {
            bucket, incoming_bytes: 64, limit_bytes: 128,
        }) if bucket == "bounded"
    ));

    // Depth bound, on a fresh bucket: 4 entries max.
    let deep = BucketId::new("deep");
    for n in 0..4 {
        let id = format!("entry-{n}");
        relay
            .push(&deep, &id, &[n])
            .unwrap_or_else(|err| panic!("push {id} refused: {err}"));
    }
    assert!(matches!(
        relay.push(&deep, "entry-4", &[4]),
        Err(RelayError::BucketFull { depth: 4, .. })
    ));

    // Refused pushes leave the buckets untouched: stats still report the
    // retained state, and valid pushes after a refusal keep their ids.
    let stats = relay.stats(&deep);
    assert_eq!(stats.pending + stats.fetched, 4);
    assert_eq!(
        relay.push(&bucket, "one", &[1u8; 64]).unwrap(),
        PushOutcome::Duplicate { seq: 1 }
    );
}
