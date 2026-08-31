//! P0.5 storage-spike acceptance evidence (ADR 0009).
//!
//! Deterministic throughout: fixture timestamps, in-memory SQLite or
//! `tempfile` paths, raw second connections only to prove what the
//! database itself refuses. No sleeps, no network, no wall clock.

use altior_domain::{EventId, ThreadId, TurnId, UnixMillis};
use altior_protocol::{EventBody, EventEnvelope, KnownEvent, ProtocolVersion, Sequence};
use altior_storage::{
    AppendOutcome, JOURNAL_LIMIT_MAX, JOURNAL_PAYLOAD_MAX, JournalLimit, StorageError, Store,
    ThreadSummary,
};

/// Fixture epoch for `occurred_at` values (2023-11-14T22:13:20Z).
const BASE_MILLIS: u64 = 1_700_000_000_000;

/// Pads an identifier body to the domain minimum of 16 characters.
fn id(prefix: &str, body: &str) -> String {
    let padded = format!("{body:0<16}");
    format!("{prefix}{padded}")
}

fn event_id(n: u64) -> EventId {
    EventId::try_from(id("evt_", &n.to_string())).expect("valid event id")
}

fn thread_id(name: &str) -> ThreadId {
    ThreadId::try_from(id("thr_", name)).expect("valid thread id")
}

fn turn_id(name: &str) -> TurnId {
    TurnId::try_from(id("trn_", name)).expect("valid turn id")
}

/// A deterministic thread-scoped envelope.
fn envelope(n: u64, thread: Option<&str>, body: EventBody) -> EventEnvelope {
    EventEnvelope {
        protocol_version: ProtocolVersion::try_new(1).expect("version 1 supported"),
        event_id: event_id(n),
        operation_id: None,
        thread_id: thread.map(thread_id),
        turn_id: thread.map(|name| turn_id(&format!("{name}turn"))),
        sequence: Sequence::try_new(n).expect("nonzero sequence"),
        occurred_at: UnixMillis::from_millis(BASE_MILLIS + n),
        body,
    }
}

fn delta(text: &str) -> EventBody {
    EventBody::Known(KnownEvent::MessageDelta {
        text: altior_protocol::MessageText::try_from(text).expect("bounded text"),
    })
}

fn unknown(provider: &str) -> EventBody {
    EventBody::Unknown {
        provider_kind: provider.to_owned(),
        diagnostic: altior_protocol::DiagnosticText::try_from("{\"rows\":12}")
            .expect("bounded diagnostic"),
    }
}

#[test]
fn fresh_database_migrates_to_latest_and_reopens_unchanged() {
    let store = Store::open_in_memory().expect("fresh in-memory store");
    assert_eq!(store.schema_version().expect("schema version"), 4);
    assert_eq!(store.journal_len().expect("count"), 0);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let file_store = Store::open(&path).expect("open fresh file");
        assert_eq!(file_store.schema_version().expect("read version"), 4);
    }
    // Reopening an already-migrated file applies nothing and succeeds.
    let reopened = Store::open(&path).expect("reopen migrated file");
    assert_eq!(reopened.schema_version().expect("read version"), 4);
    assert_eq!(reopened.journal_len().expect("count"), 0);
}

#[test]
fn newer_schema_is_refused_not_downgraded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    drop(Store::open(&path).expect("open fresh file"));

    // Simulate a file written by a newer build.
    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.pragma_update(None, "user_version", 99)
        .expect("stamp version");
    drop(raw);

    let error = Store::open(&path).expect_err("must refuse newer schema");
    let StorageError::SchemaTooNew { found, supported } = error else {
        panic!("expected SchemaTooNew, got {error:?}");
    };
    assert_eq!((found, supported), (99, 4));

    // The file is untouched: still stamped 99.
    let raw = rusqlite::Connection::open(&path).expect("raw open");
    let version: i64 = raw
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read version");
    assert_eq!(version, 99);
}

#[test]
fn append_folds_thread_projections_and_skips_thread_less_events() {
    let mut store = Store::open_in_memory().expect("store");

    let script = [
        (1u64, Some("alpha"), delta("hello")),
        (2, Some("alpha"), EventBody::Known(KnownEvent::TurnStarted)),
        (3, None, unknown("usage.stats.snapshot")),
        (4, Some("beta"), delta("hi")),
        (5, Some("beta"), unknown("acp.update.plan")),
        (
            6,
            Some("alpha"),
            EventBody::Known(KnownEvent::TurnCompleted),
        ),
    ];
    for (n, thread, body) in script {
        let outcome = store
            .append_event(&envelope(n, thread, body))
            .expect("append");
        assert_eq!(
            outcome,
            AppendOutcome::Appended {
                seq: i64::try_from(n).expect("seq fits")
            }
        );
    }

    assert_eq!(store.journal_len().expect("count"), 6);

    let summaries = store.thread_summaries().expect("summaries");
    assert_eq!(summaries.len(), 2, "thread-less event does not project");
    assert_eq!(
        summaries,
        vec![
            ThreadSummary {
                thread_id: id("thr_", "alpha"),
                event_count: 3,
                first_seq: 1,
                last_seq: 6,
                last_event_id: id("evt_", "6"),
                last_kind: "turn.completed".to_owned(),
                updated_at: i64::try_from(BASE_MILLIS + 6).expect("fits"),
            },
            ThreadSummary {
                thread_id: id("thr_", "beta"),
                event_count: 2,
                first_seq: 4,
                last_seq: 5,
                last_event_id: id("evt_", "5"),
                last_kind: "acp.update.plan".to_owned(),
                updated_at: i64::try_from(BASE_MILLIS + 5).expect("fits"),
            },
        ]
    );
}

#[test]
fn duplicate_append_is_idempotent() {
    let mut store = Store::open_in_memory().expect("store");
    let first = envelope(1, Some("alpha"), delta("one"));
    let second = envelope(2, Some("alpha"), delta("two"));

    assert_eq!(
        store.append_event(&first).expect("append"),
        AppendOutcome::Appended { seq: 1 }
    );
    assert_eq!(
        store.append_event(&first).expect("duplicate append"),
        AppendOutcome::Duplicate { seq: 1 }
    );
    assert_eq!(
        store.append_event(&second).expect("append"),
        AppendOutcome::Appended { seq: 2 }
    );

    assert_eq!(store.journal_len().expect("count"), 2);
    let summaries = store.thread_summaries().expect("summaries");
    assert_eq!(summaries[0].event_count, 2, "duplicate did not inflate");
    assert_eq!(summaries[0].last_event_id, id("evt_", "2"));
}

#[test]
fn duplicate_event_id_with_different_payload_is_a_collision() {
    let mut store = Store::open_in_memory().expect("store");
    let original = envelope(1, Some("alpha"), delta("one"));
    let collision = envelope(1, Some("alpha"), delta("different"));
    store.append_event(&original).expect("append");
    let error = store.append_event(&collision).unwrap_err();
    assert!(matches!(
        error,
        StorageError::EventIdCollision {
            existing_seq: 1,
            ..
        }
    ));
    assert_eq!(store.journal_len().expect("count"), 1);
    assert_eq!(
        store
            .journal_records(0, JournalLimit::try_new(1).expect("bounded"))
            .expect("row")[0]
            .decode()
            .expect("decode"),
        original
    );
}

#[test]
fn journal_history_is_append_only_at_the_database_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let mut store = Store::open(&path).expect("store");
        store
            .append_event(&envelope(1, Some("alpha"), delta("hello")))
            .expect("append");
    }

    let raw = rusqlite::Connection::open(&path).expect("raw open");
    let update = raw.execute("UPDATE journal SET kind = 'tampered'", []);
    assert!(update.is_err(), "UPDATE must be refused");
    let update_detail = update.unwrap_err().to_string();
    assert!(
        update_detail.contains("journal is append-only"),
        "trigger message present: {update_detail}"
    );

    let delete = raw.execute("DELETE FROM journal", []);
    assert!(delete.is_err(), "DELETE must be refused");
    assert!(
        delete
            .unwrap_err()
            .to_string()
            .contains("journal is append-only"),
        "trigger message present"
    );

    // Projections are derived state and stay mutable by design.
    raw.execute("DELETE FROM thread_projection", [])
        .expect("projections are rebuildable");
}

#[test]
fn rebuild_reproduces_identical_projections_and_heals_on_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let mut store = Store::open(&path).expect("store");
        for (n, thread, body) in [
            (1u64, Some("alpha"), delta("one")),
            (2, Some("alpha"), delta("two")),
            (3, Some("beta"), EventBody::Known(KnownEvent::TurnStarted)),
            (4, Some("alpha"), unknown("acp.update.plan")),
            (5, Some("beta"), delta("three")),
            (6, None, unknown("usage.stats.snapshot")),
            (7, Some("beta"), EventBody::Known(KnownEvent::TurnCompleted)),
        ] {
            store
                .append_event(&envelope(n, thread, body))
                .expect("append");
        }
        assert_eq!(store.journal_len().expect("count"), 7);
    }

    // Simulate a lost/damaged projection: both derived tables wiped.
    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.execute("DELETE FROM thread_projection", [])
        .expect("wipe");
    raw.execute("DELETE FROM projection_state", [])
        .expect("wipe marker");
    drop(raw);

    // Reopening heals: non-empty journal + missing marker forces replay.
    let healed = Store::open(&path).expect("heal on reopen");
    assert_eq!(healed.journal_len().expect("count"), 7);
    assert_eq!(
        healed.journal_max_seq().expect("max seq"),
        healed.journal_max_seq().expect("max seq again")
    );

    // The healed summaries equal the incremental fold exactly.
    let mut expected = Store::open_in_memory().expect("reference store");
    for (n, thread, body) in [
        (1u64, Some("alpha"), delta("one")),
        (2, Some("alpha"), delta("two")),
        (3, Some("beta"), EventBody::Known(KnownEvent::TurnStarted)),
        (4, Some("alpha"), unknown("acp.update.plan")),
        (5, Some("beta"), delta("three")),
        (6, None, unknown("usage.stats.snapshot")),
        (7, Some("beta"), EventBody::Known(KnownEvent::TurnCompleted)),
    ] {
        expected
            .append_event(&envelope(n, thread, body))
            .expect("append");
    }
    assert_eq!(
        healed.thread_summaries().expect("healed summaries"),
        expected.thread_summaries().expect("reference summaries")
    );
}

#[test]
fn explicit_rebuild_replays_the_whole_journal_and_is_stable() {
    let mut store = Store::open_in_memory().expect("store");
    for (n, thread, body) in [
        (1u64, Some("alpha"), delta("one")),
        (2, Some("beta"), delta("two")),
        (3, Some("alpha"), delta("three")),
    ] {
        store
            .append_event(&envelope(n, thread, body))
            .expect("append");
    }
    let before = store.thread_summaries().expect("summaries");

    let replayed = store.rebuild_projections().expect("rebuild");
    assert_eq!(replayed, 3, "replayed the whole journal");
    assert_eq!(
        store.thread_summaries().expect("summaries after"),
        before,
        "rebuild reproduces the incremental fold"
    );

    let again = store.rebuild_projections().expect("rebuild again");
    assert_eq!(again, 3, "rebuild is repeatable");
    assert_eq!(
        store.thread_summaries().expect("summaries stable"),
        before,
        "second rebuild is stable"
    );
}

#[test]
fn stale_marker_on_reopen_triggers_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    let reference_summaries = {
        let mut store = Store::open(&path).expect("store");
        for (n, thread, body) in [
            (1u64, Some("alpha"), delta("one")),
            (2, Some("alpha"), delta("two")),
            (3, Some("alpha"), delta("three")),
        ] {
            store
                .append_event(&envelope(n, thread, body))
                .expect("append");
        }
        store.thread_summaries().expect("summaries")
    };

    // Simulate a crash after the journal write but before the marker
    // caught up (marker behind the journal).
    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.execute("UPDATE projection_state SET journal_max_seq = 1", [])
        .expect("age the marker");
    drop(raw);

    let reopened = Store::open(&path).expect("reopen with stale marker");
    assert_eq!(
        reopened.thread_summaries().expect("summaries"),
        reference_summaries,
        "stale marker healed by replay"
    );
}

#[test]
fn stale_projection_fold_version_rebuilds_even_when_journal_marker_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let mut store = Store::open(&path).expect("store");
        store
            .append_event(&envelope(1, Some("alpha"), delta("one")))
            .expect("append");
    }

    let raw = rusqlite::Connection::open(&path).expect("raw open");
    raw.execute("DELETE FROM thread_projection", [])
        .expect("damage projection only");
    raw.execute("UPDATE projection_state SET projection_version = 0", [])
        .expect("age fold version while leaving journal_max_seq unchanged");
    drop(raw);

    let reopened = Store::open(&path).expect("rebuild stale fold");
    let summaries = reopened.thread_summaries().expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].event_count, 1);
}

#[test]
fn journal_records_roundtrip_envelopes_in_append_order() {
    let mut store = Store::open_in_memory().expect("store");
    let originals = [
        envelope(1, Some("alpha"), delta("hello")),
        envelope(2, Some("alpha"), EventBody::Known(KnownEvent::TurnStarted)),
        envelope(3, None, unknown("usage.stats.snapshot")),
        envelope(4, Some("beta"), unknown("acp.update.plan")),
        envelope(5, Some("beta"), EventBody::Known(KnownEvent::TurnCompleted)),
    ];
    for event in &originals {
        store.append_event(event).expect("append");
    }

    let rows = store
        .journal_records(0, JournalLimit::try_new(100).expect("bounded"))
        .expect("records");
    assert_eq!(rows.len(), 5);
    for (row, original) in rows.iter().zip(&originals) {
        assert_eq!(row.event_id, original.event_id.as_str());
        assert_eq!(row.kind, original.body.kind_name());
        assert_eq!(row.decode().expect("decode"), *original);
    }
    assert!(
        rows.windows(2).all(|pair| pair[0].seq < pair[1].seq),
        "append order preserved"
    );

    let tail = store
        .journal_records(3, JournalLimit::try_new(2).expect("bounded"))
        .expect("tail records");
    let tail_ids: Vec<&str> = tail.iter().map(|row| row.event_id.as_str()).collect();
    assert_eq!(
        tail_ids,
        vec![id("evt_", "4").as_str(), id("evt_", "5").as_str()],
        "after_seq and limit are honored"
    );
}

#[test]
fn journal_limit_is_unsigned_and_bounded_before_sqlite() {
    assert_eq!(JournalLimit::try_new(0).expect("zero page").get(), 0);
    assert_eq!(
        JournalLimit::try_new(JOURNAL_LIMIT_MAX)
            .expect("max page")
            .get(),
        JOURNAL_LIMIT_MAX
    );
    assert!(matches!(
        JournalLimit::try_new(JOURNAL_LIMIT_MAX + 1),
        Err(StorageError::JournalLimitOutOfRange { .. })
    ));
}

#[test]
fn defense_in_depth_cap_exceeds_the_largest_legal_envelope() {
    // The protocol's own bounds (64 KiB message text, 4 KiB diagnostic)
    // keep every legal envelope far under the journal cap, so the cap is
    // pure defense-in-depth against future envelope growth (ADR 0009 §3).
    let biggest = envelope(
        1,
        Some("alpha"),
        delta(&"x".repeat(altior_protocol::MessageText::capacity())),
    );
    let encoded = biggest.to_json().expect("encode");
    assert!(
        encoded.len() < JOURNAL_PAYLOAD_MAX,
        "largest legal envelope is {} bytes, cap is {JOURNAL_PAYLOAD_MAX}",
        encoded.len()
    );

    let mut store = Store::open_in_memory().expect("store");
    store
        .append_event(&biggest)
        .expect("append biggest legal event");
    assert_eq!(store.journal_len().expect("count"), 1);
}
