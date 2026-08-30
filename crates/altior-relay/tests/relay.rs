//! Relay queue-machine tests (ADR 0012): cursors, idempotent push,
//! quotas, logical-tick retention, compaction equivalence. No
//! network, no timers, no wall clock.

use altior_relay::{BucketId, BucketStats, FetchPage, PushOutcome, Relay, RelayError, RelayPolicy};

fn bucket() -> BucketId {
    BucketId::new("bob-inbox")
}

fn push(relay: &mut Relay, id: &str, payload: &[u8]) -> u64 {
    match relay.push(&bucket(), id, payload).expect("push") {
        PushOutcome::Pushed { seq } | PushOutcome::Duplicate { seq } => seq,
    }
}

fn entries(page: FetchPage) -> Vec<(u64, String, Vec<u8>)> {
    match page {
        FetchPage::Entries { items, .. } => items
            .into_iter()
            .map(|item| (item.seq, item.id, item.payload))
            .collect(),
        FetchPage::Compacted { compacted_up_to } => {
            panic!("unexpected Compacted({compacted_up_to})")
        }
        FetchPage::InvalidCursor {
            requested,
            frontier,
        } => panic!("unexpected InvalidCursor({requested}, frontier {frontier})"),
    }
}

#[test]
fn push_assigns_sequences_and_fetch_pages_in_order() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    for i in 0..5 {
        let seq = push(
            &mut relay,
            &format!("id{i}"),
            format!("payload-{i}").as_bytes(),
        );
        assert_eq!(
            seq,
            u64::try_from(i).expect("small") + 1,
            "seqs from 1, no gaps"
        );
    }

    let first = relay.fetch(&bucket(), 0, 2);
    let FetchPage::Entries {
        items,
        next_cursor,
        has_more,
    } = first
    else {
        panic!("expected entries");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(next_cursor, 2);
    assert!(has_more);

    let rest = entries(relay.fetch(&bucket(), next_cursor, 10));
    assert_eq!(rest.len(), 3, "the remaining page");
    assert_eq!(rest[0].2, b"payload-2");
    assert_eq!(rest[2].0, 5);
    // Fetching past the end is an empty, has_more=false page.
    let FetchPage::Entries {
        items,
        next_cursor,
        has_more,
    } = relay.fetch(&bucket(), 5, 10)
    else {
        panic!("expected entries");
    };
    assert!(items.is_empty());
    assert_eq!(next_cursor, 5);
    assert!(!has_more);
}

#[test]
fn fetch_is_repeatable_delivery_is_at_least_once() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    push(&mut relay, "a", b"alpha");
    push(&mut relay, "b", b"beta");
    let once = entries(relay.fetch(&bucket(), 0, 10));
    let twice = entries(relay.fetch(&bucket(), 0, 10));
    assert_eq!(once, twice, "fetch never consumes; receivers dedupe");
}

#[test]
fn push_is_idempotent_within_the_retained_window() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    assert_eq!(
        relay.push(&bucket(), "evt-1", b"payload").expect("push"),
        PushOutcome::Pushed { seq: 1 }
    );
    assert_eq!(
        relay.push(&bucket(), "evt-1", b"payload").expect("push"),
        PushOutcome::Duplicate { seq: 1 }
    );
    assert_eq!(relay.stats(&bucket()).pending, 1, "no second copy queued");
    let items = entries(relay.fetch(&bucket(), 0, 10));
    assert_eq!(items.len(), 1);
}

#[test]
fn duplicate_push_id_with_different_payload_is_a_collision() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    push(&mut relay, "evt-1", b"original");
    assert_eq!(
        relay.push(&bucket(), "evt-1", b"different").unwrap_err(),
        RelayError::PushIdCollision {
            id: "evt-1".to_owned(),
            existing_seq: 1,
        }
    );
    assert_eq!(entries(relay.fetch(&bucket(), 0, 10))[0].2, b"original");
}

#[test]
fn oversized_push_is_refused_whole() {
    let mut relay = Relay::new(RelayPolicy {
        max_payload_bytes: 8,
        ..RelayPolicy::permissive()
    });
    assert_eq!(
        relay.push(&bucket(), "big", &[0u8; 9]).unwrap_err(),
        RelayError::PushTooLarge {
            size_bytes: 9,
            limit_bytes: 8
        }
    );
    assert_eq!(relay.stats(&bucket()).pending, 0, "bucket untouched");
}

#[test]
fn depth_quota_pushes_back_until_compaction() {
    let mut relay = Relay::new(RelayPolicy {
        max_bucket_depth: 3,
        ..RelayPolicy::permissive()
    });
    push(&mut relay, "1", b"one");
    push(&mut relay, "2", b"two");
    push(&mut relay, "3", b"three");
    let err = relay.push(&bucket(), "4", b"four").unwrap_err();
    assert!(matches!(err, RelayError::BucketFull { depth: 3, .. }));
    // A retry of an already-queued id is never blocked by the quota.
    assert_eq!(
        relay.push(&bucket(), "2", b"two").expect("retry"),
        PushOutcome::Duplicate { seq: 2 }
    );

    // Fetching alone frees no depth (delivery is at-least-once, the
    // entries stay for re-delivery) ...
    relay.fetch(&bucket(), 0, 10);
    assert!(matches!(
        relay.push(&bucket(), "5", b"five"),
        Err(RelayError::BucketFull { .. })
    ));
    // ... compaction does.
    let summary = relay.compact(&bucket());
    assert_eq!(summary.payloads, 3);
    push(&mut relay, "6", b"six");
    assert_eq!(relay.stats(&bucket()).pending, 1);
}

#[test]
fn compaction_preserves_fetch_equivalence_for_live_cursors() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    for i in 0..10 {
        push(
            &mut relay,
            &format!("id{i}"),
            &[u8::try_from(i).expect("i < 10")],
        );
    }
    // The receiver pages through everything.
    relay.fetch(&bucket(), 0, 10);

    // Snapshot the live cursor — exactly at the coming checkpoint
    // boundary (10). (Probing beyond it would itself assert
    // possession of mail that was never delivered.)
    let before = relay.fetch(&bucket(), 10, 3);
    let summary = relay.compact(&bucket());
    assert_eq!(summary.compacted_up_to, 10);
    assert_eq!(summary.payloads, 10);
    // After compaction the same cursor sees an identical page.
    assert_eq!(relay.fetch(&bucket(), 10, 3), before, "live cursor stable");
    // Only a cursor that fell behind the checkpoint is told to resync.
    assert_eq!(
        relay.fetch(&bucket(), 4, 3),
        FetchPage::Compacted {
            compacted_up_to: 10
        }
    );
}

#[test]
fn retention_runs_on_logical_ticks_and_reports_reclamation() {
    let mut relay = Relay::new(RelayPolicy {
        max_age_ticks: 5,
        ..RelayPolicy::permissive()
    });
    push(&mut relay, "old", b"stale-by-policy");
    // Age the entry by six logical ticks; no wall clock involved.
    for _ in 0..6 {
        relay.tick();
    }
    push(&mut relay, "fresh", b"still-within-retention");

    let swept = relay.sweep_expired();
    assert_eq!(swept.len(), 1, "one bucket reported reclamation");
    let (bucket_id, summary) = &swept[0];
    assert_eq!(bucket_id.as_str(), "bob-inbox");
    assert_eq!(summary.payloads, 1, "only the aged entry");

    // The aged entry is gone; a consumer fetching from the start is
    // told to resync rather than silently missing mail.
    assert_eq!(
        relay.fetch(bucket_id, 0, 10),
        FetchPage::Compacted { compacted_up_to: 1 }
    );
    let fresh = entries(relay.fetch(bucket_id, 1, 10));
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].2, b"still-within-retention");
}

#[test]
fn retention_boundary_is_strict_and_zero_age_is_well_defined() {
    for (age, should_expire) in [(4u64, false), (5, false), (6, true)] {
        let mut relay = Relay::new(RelayPolicy {
            max_age_ticks: 5,
            ..RelayPolicy::permissive()
        });
        push(&mut relay, "entry", b"payload");
        for _ in 0..age {
            relay.tick();
        }
        assert_eq!(
            !relay.sweep_expired().is_empty(),
            should_expire,
            "age {age} around limit 5"
        );
    }

    let mut zero = Relay::new(RelayPolicy {
        max_age_ticks: 0,
        ..RelayPolicy::permissive()
    });
    push(&mut zero, "entry", b"payload");
    assert!(zero.sweep_expired().is_empty(), "age zero is retained");
    zero.tick();
    assert_eq!(zero.sweep_expired()[0].1.payloads, 1, "age one expires");
}

#[test]
fn future_cursor_never_advances_possession_or_hides_future_push() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    push(&mut relay, "first", b"one");
    assert_eq!(
        relay.fetch(&bucket(), u64::MAX, 10),
        FetchPage::InvalidCursor {
            requested: u64::MAX,
            frontier: 1,
        }
    );
    assert_eq!(
        relay.compact(&bucket()).payloads,
        0,
        "no forged ack recorded"
    );
    push(&mut relay, "second", b"two");
    let page = entries(relay.fetch(&bucket(), 0, 10));
    assert_eq!(
        page.len(),
        2,
        "future request hid neither retained nor later mail"
    );
}

#[test]
fn unknown_bucket_fetches_are_empty() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    let ghost = BucketId::new("ghost");
    assert_eq!(
        relay.fetch(&ghost, 0, 10),
        FetchPage::Entries {
            items: Vec::new(),
            next_cursor: 0,
            has_more: false
        }
    );
    assert_eq!(relay.stats(&ghost), BucketStats::default());
    assert_eq!(relay.compact(&ghost).payloads, 0);
}

#[test]
fn sequence_numbers_span_buckets_without_reuse() {
    let mut relay = Relay::new(RelayPolicy::permissive());
    let a = BucketId::new("a");
    let b = BucketId::new("b");
    let PushOutcome::Pushed { seq: seq_a } = relay.push(&a, "1", b"x").expect("push") else {
        panic!("pushed");
    };
    let PushOutcome::Pushed { seq: seq_b } = relay.push(&b, "1", b"x").expect("push") else {
        panic!("pushed");
    };
    assert_ne!(seq_a, seq_b, "one counter space, no reuse anywhere");
}
