#![allow(clippy::all, clippy::pedantic)]

use altior_domain::{
    AgentProfile, AgentProfileId, DisplayName, DomainEvent, DomainEventKind, EventId, EventPayload,
    HarnessKind, MemoryMode, ThreadId, TurnId, UnixMillis,
};
use altior_storage::{Store, domain_projection_digest};
use std::time::Instant;

fn id(prefix: &str, num: usize) -> String {
    format!("{prefix}{:016}", num)
}

#[test]
fn test_measure_digest_scaling_and_append_costs() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let mut store = Store::open(&db_path).expect("open store");

    println!("\n=== A16 Performance Evaluation: Digest & Append Costs ===");
    println!("Host: Windows x86_64, Rust 1.98.0");

    let agp_id = AgentProfileId::try_from(id("agp_", 1)).unwrap();
    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Bench Agent").unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::LongTerm,
        created_at: UnixMillis::from_millis(1_700_000_000_000),
        updated_at: UnixMillis::from_millis(1_700_000_000_000),
    };
    store.create_agent_profile(&profile).unwrap();

    // 1. Measure thread creation at scale
    for &scale in &[100, 500, 1000] {
        let start = Instant::now();
        for i in 0..scale {
            let thread_id = ThreadId::try_from(format!("thr_{:02}{:014}", scale % 100, i)).unwrap();
            let event = DomainEvent {
                event_id: EventId::try_from(format!("evt_{:02}{:014}", scale % 100, i)).unwrap(),
                thread_id: Some(thread_id),
                turn_id: None,
                operation_id: None,
                occurred_at: UnixMillis::from_millis(1_700_000_000_000 + i as u64),
                kind: DomainEventKind::ThreadCreated,
                payload: EventPayload::try_from(
                    format!(
                        r#"{{"agent_profile_id":"{}","title":"Bench"}}"#,
                        agp_id.as_str()
                    )
                    .as_bytes(),
                )
                .unwrap(),
            };
            store.append_domain_event(&event).unwrap();
        }
        let elapsed = start.elapsed();
        println!(
            "Appended {} thread events: {:?} total, average {:.3} ms/event",
            scale,
            elapsed,
            elapsed.as_secs_f64() * 1000.0 / scale as f64
        );
    }

    // 2. Measure in-turn MessageDelta streaming events
    let stream_tid = ThreadId::try_from(id("thr_", 999_999)).unwrap();
    let stream_turn = TurnId::try_from(id("trn_", 999_999)).unwrap();

    let thr_ev = DomainEvent {
        event_id: EventId::try_from(id("evt_", 999_998)).unwrap(),
        thread_id: Some(stream_tid.clone()),
        turn_id: None,
        operation_id: None,
        occurred_at: UnixMillis::from_millis(1_700_001_000_000),
        kind: DomainEventKind::ThreadCreated,
        payload: EventPayload::try_from(
            format!(
                r#"{{"agent_profile_id":"{}","title":"Stream Thread"}}"#,
                agp_id.as_str()
            )
            .as_bytes(),
        )
        .unwrap(),
    };
    store.append_domain_event(&thr_ev).unwrap();

    let turn_ev = DomainEvent {
        event_id: EventId::try_from(id("evt_", 999_999)).unwrap(),
        thread_id: Some(stream_tid.clone()),
        turn_id: Some(stream_turn.clone()),
        operation_id: None,
        occurred_at: UnixMillis::from_millis(1_700_001_000_100),
        kind: DomainEventKind::TurnStarted,
        payload: EventPayload::try_from(br#"{"prompt":"Streaming prompt"}"#.as_slice()).unwrap(),
    };
    store.append_domain_event(&turn_ev).unwrap();

    // Measure raw standalone domain_projection_digest at current table size (1602 threads)
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let start_dig = Instant::now();
    let d = domain_projection_digest(&raw).unwrap();
    let dig_elapsed = start_dig.elapsed();
    println!(
        "Standalone domain_projection_digest over ~1600 rows: {:?} (digest: {})",
        dig_elapsed,
        &d[..8]
    );

    let delta_count = 200;
    let start_deltas = Instant::now();
    for i in 0..delta_count {
        let delta_ev = DomainEvent {
            event_id: EventId::try_from(format!("evt_d{:015}", i)).unwrap(),
            thread_id: Some(stream_tid.clone()),
            turn_id: Some(stream_turn.clone()),
            operation_id: None,
            occurred_at: UnixMillis::from_millis(1_700_001_000_200 + i as u64),
            kind: DomainEventKind::MessageDelta,
            payload: EventPayload::try_from(br#"{"text":" token"}"#.as_slice()).unwrap(),
        };
        store.append_domain_event(&delta_ev).unwrap();
    }
    let elapsed_deltas = start_deltas.elapsed();
    println!(
        "Appended {} streaming message deltas: {:?} total, average {:.3} ms/delta",
        delta_count,
        elapsed_deltas,
        elapsed_deltas.as_secs_f64() * 1000.0 / delta_count as f64
    );

    // 3. Settle the turn with TurnCompleted and verify authoritative digest checkpoint
    let comp_ev = DomainEvent {
        event_id: EventId::try_from(id("evt_", 999997)).unwrap(),
        thread_id: Some(stream_tid.clone()),
        turn_id: Some(stream_turn.clone()),
        operation_id: None,
        occurred_at: UnixMillis::from_millis(1_700_001_000_900),
        kind: DomainEventKind::TurnCompleted,
        payload: EventPayload::try_from(br#"{}"#.as_slice()).unwrap(),
    };
    let start_comp = Instant::now();
    store.append_domain_event(&comp_ev).unwrap();
    let comp_elapsed = start_comp.elapsed();
    println!(
        "Settled turn with TurnCompleted (digest checkpointed): {:?}",
        comp_elapsed
    );

    // Verify stored digest strictly matches live digest after settlement
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let stored: String = raw
        .query_row(
            "SELECT projection_digest FROM domain_projection_state WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let live = domain_projection_digest(&raw).unwrap();
    assert_eq!(
        stored, live,
        "Authoritative settlement digest must exactly match live tables"
    );
    println!(
        "Verified: stored digest matches live digest perfectly ({})",
        &live[..8]
    );
}
#[test]
fn test_mid_turn_crash_detected_and_rebuilt_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.db");

    let agp_id = AgentProfileId::try_from(id("agp_", 2)).unwrap();
    let profile = AgentProfile {
        id: agp_id.clone(),
        display_name: DisplayName::try_from("Crash Test Agent").unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode: MemoryMode::LongTerm,
        created_at: UnixMillis::from_millis(1_700_000_000_000),
        updated_at: UnixMillis::from_millis(1_700_000_000_000),
    };

    let tid = ThreadId::try_from(id("thr_", 555_001)).unwrap();
    let trnid = TurnId::try_from(id("trn_", 555_001)).unwrap();

    {
        let mut store = Store::open(&db_path).unwrap();
        store.create_agent_profile(&profile).unwrap();

        store
            .append_domain_event(&DomainEvent {
                event_id: EventId::try_from(id("evt_", 555_001)).unwrap(),
                thread_id: Some(tid.clone()),
                turn_id: None,
                operation_id: None,
                occurred_at: UnixMillis::from_millis(1_700_000_000_100),
                kind: DomainEventKind::ThreadCreated,
                payload: EventPayload::try_from(
                    format!(r#"{{"agent_profile_id":"{}"}}"#, agp_id.as_str()).as_bytes(),
                )
                .unwrap(),
            })
            .unwrap();

        store
            .append_domain_event(&DomainEvent {
                event_id: EventId::try_from(id("evt_", 555_002)).unwrap(),
                thread_id: Some(tid.clone()),
                turn_id: Some(trnid.clone()),
                operation_id: None,
                occurred_at: UnixMillis::from_millis(1_700_000_000_200),
                kind: DomainEventKind::TurnStarted,
                payload: EventPayload::try_from(br#"{"prompt":"Hello"}"#.as_slice()).unwrap(),
            })
            .unwrap();

        // Append 5 deltas
        for i in 0..5 {
            store
                .append_domain_event(&DomainEvent {
                    event_id: EventId::try_from(format!("evt_{:02}{:014}", 55, i)).unwrap(),
                    thread_id: Some(tid.clone()),
                    turn_id: Some(trnid.clone()),
                    operation_id: None,
                    occurred_at: UnixMillis::from_millis(1_700_000_000_300 + i),
                    kind: DomainEventKind::MessageDelta,
                    payload: EventPayload::try_from(
                        format!(r#"{{"text":" chunk_{i}"}}"#).as_bytes(),
                    )
                    .unwrap(),
                })
                .unwrap();
        }

        // Drop store WITHOUT calling TurnCompleted (simulating sudden crash/kill)
        drop(store);
    }

    // Now reopen the database. Store::open MUST detect unfinalized digest, trigger rebuild, and heal!
    let reopened = Store::open(&db_path).expect("reopen should succeed and heal via rebuild");
    let limit = altior_domain::TurnListLimit::try_new(10).unwrap();
    let turns = reopened.turns_for_thread(&tid, None, limit).unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, trnid.as_str());
    assert_eq!(turns[0].event_count, 6);

    let hist_limit = altior_domain::HistoryLimit::try_new(20).unwrap();
    let history = reopened.thread_history(&tid, 0, hist_limit).unwrap();
    assert_eq!(history.len(), 7); // 1 thread + 1 turn start + 5 deltas

    // Live digest and stored digest must now match after the automatic heal
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let stored: String = raw
        .query_row(
            "SELECT projection_digest FROM domain_projection_state WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let live = domain_projection_digest(&raw).unwrap();
    assert_eq!(stored, live);
    println!("Crash recovery verified: turn content fully preserved and digest healed!");
}
