//! Adversarial convergence suite for the `SyncDocumentEngine`
//! bake-off (ADR 0010).
//!
//! Every schedule is deterministic (fixed seeds, deterministic peer
//! ids, no timing, no network). The invariants asserted per library:
//! all replicas of a run converge to identical views, merges are
//! idempotent, and no adversarial interleaving panics or loses
//! writes. Cross-library equality is asserted only for *sequential*
//! scripts, where both engines must produce exactly the same text.

use altior_crdt::{
    AnyEngine, CrdtError, DocumentSchema, FieldView, Lcg, MAX_STATE_BYTES, SyncDocumentEngine,
};

fn factories(text_fields: &[&str], scalar_fields: &[&str]) -> Vec<(&'static str, AnyEngine)> {
    vec![
        (
            "loro",
            AnyEngine::with_schema("loro", text_fields, scalar_fields).expect("valid schema"),
        ),
        (
            "automerge",
            AnyEngine::with_schema("automerge", text_fields, scalar_fields).expect("valid schema"),
        ),
    ]
}

/// Merges everything `src` holds into `dst` through the opaque state
/// exchange, exactly as a sync transport would.
fn exchange(dst: &mut AnyEngine, src: &AnyEngine) {
    let bytes = src.as_dyn().export_state();
    dst.as_dyn_mut().import_state(&bytes).expect("valid state");
}

fn char_count(text: &str, needle: char) -> usize {
    text.chars().filter(|c| *c == needle).count()
}

#[test]
fn sequential_scripts_agree_across_engines() {
    let mut views = Vec::new();
    for (_, mut engine) in factories(&["body"], &["title", "mood"]) {
        engine.set_field("title", "session notes").expect("set");
        engine
            .insert_text("body", 0, "hello world")
            .expect("insert");
        engine.insert_text("body", 5, " brave").expect("insert");
        engine.delete_text("body", 0, 6).expect("delete");
        engine.insert_text("body", 999, "!").expect("insert");
        engine.delete_text("body", 3, 999).expect("delete");
        engine.set_field("mood", "focused").expect("set");
        views.push((engine.engine_name().to_owned(), engine.view()));
    }
    let (loro_name, loro_view) = &views[0];
    let (automerge_name, automerge_view) = &views[1];
    assert_eq!(
        loro_view, automerge_view,
        "{loro_name} view {loro_view:#?} vs {automerge_name} view {automerge_view:#?}"
    );
    // The fold of the script: "hello brave world" -> delete 6 -> "brave
    // world" -> append "!" -> delete from 3 -> "bra".
    assert_eq!(
        loro_view[0],
        ("body".to_owned(), FieldView::Text("bra".to_owned()))
    );
}

#[test]
fn concurrent_same_offset_inserts_converge() {
    for (name, mut a) in factories(&["body"], &[]) {
        let mut b = a.fork_with_peer(2);
        a.insert_text("body", 0, "aaa").expect("insert");
        b.insert_text("body", 0, "bbb").expect("insert");
        exchange(&mut a, &b);
        exchange(&mut b, &a);
        assert_eq!(
            a.view_digest(),
            b.view_digest(),
            "{name}: replicas converge after same-offset inserts"
        );
        let text = a.text_of("body").expect("text");
        assert_eq!(char_count(&text, 'a'), 3, "{name}: no aaa write lost");
        assert_eq!(char_count(&text, 'b'), 3, "{name}: no bbb write lost");
    }
}

#[test]
fn merges_are_idempotent_and_order_independent() {
    for (name, mut a) in factories(&["body"], &["k"]) {
        let mut b = a.fork_with_peer(2);
        let mut c = a.fork_with_peer(3);
        a.insert_text("body", 0, "alpha").expect("insert");
        b.insert_text("body", 0, "beta").expect("insert");
        c.set_field("k", "gamma").expect("set");
        exchange(&mut b, &c);
        let first = {
            let mut x = a.fork_with_peer(4);
            exchange(&mut x, &a);
            exchange(&mut x, &b);
            x.view_digest()
        };
        let second = {
            let mut x = a.fork_with_peer(5);
            exchange(&mut x, &b);
            exchange(&mut x, &a);
            x.view_digest()
        };
        assert_eq!(first, second, "{name}: merge order does not matter");

        let mut repeated = a.fork_with_peer(6);
        exchange(&mut repeated, &a);
        let once = repeated.view_digest();
        exchange(&mut repeated, &a);
        exchange(&mut repeated, &a);
        assert_eq!(
            once,
            repeated.view_digest(),
            "{name}: re-merging is a no-op"
        );
    }
}

#[test]
fn delete_racing_insert_converges_without_loss() {
    for (name, mut a) in factories(&["body"], &[]) {
        let mut b = a.fork_with_peer(2);
        a.insert_text("body", 0, "abcdef").expect("insert");
        exchange(&mut b, &a);

        a.delete_text("body", 0, 3).expect("delete");
        b.insert_text("body", 1, "XY").expect("insert");
        exchange(&mut a, &b);
        exchange(&mut b, &a);

        assert_eq!(
            a.view_digest(),
            b.view_digest(),
            "{name}: replicas converge after delete/insert race"
        );
        let len = a.text_of("body").expect("text").chars().count();
        assert_eq!(len, 5, "{name}: 6 - 3 deleted + 2 inserted = 5 chars");
    }
}

#[test]
fn scalar_lww_settles_identically_on_all_replicas() {
    for (name, mut a) in factories(&[], &["status"]) {
        let mut b = a.fork_with_peer(2);
        let mut c = a.fork_with_peer(3);
        a.set_field("status", "draft").expect("set");
        b.set_field("status", "archived").expect("set");
        exchange(&mut c, &a);
        exchange(&mut c, &b);
        exchange(&mut a, &b);
        exchange(&mut b, &c);
        exchange(&mut a, &c);
        let settled = a.field_of("status").expect("field");
        assert_eq!(
            a.field_of("status").expect("field"),
            settled,
            "{name}: a settled"
        );
        assert_eq!(
            b.field_of("status").expect("field"),
            settled,
            "{name}: b settled"
        );
        assert_eq!(
            c.field_of("status").expect("field"),
            settled,
            "{name}: c settled"
        );
        assert!(
            settled
                .as_deref()
                .is_some_and(|v| v == "draft" || v == "archived"),
            "{name}: settled value is one of the written values"
        );
    }
}

#[test]
fn star_topology_converges_under_lcg_schedule() {
    let fields = ["body", "outline", "scratch"];
    let scalar_names: Vec<String> = (0..120).map(|step| format!("tag{step}")).collect();
    let scalar_refs: Vec<&str> = scalar_names.iter().map(String::as_str).collect();
    for (name, mut hub) in factories(&fields, &scalar_refs) {
        let words = ["lorem", "ipsum", "dolor", "sit", "amet", "consectetur"];
        let mut replicas: Vec<AnyEngine> =
            (1..=4u64).map(|peer| hub.fork_with_peer(peer)).collect();
        let mut lcg = Lcg::seeded(0x5eed_1234);

        for step in 0..120u64 {
            let replica = lcg.next_index(4);
            let field = fields[lcg.next_index(fields.len())];
            match lcg.next(4) {
                0 | 1 => {
                    let word = words[lcg.next_index(words.len())];
                    let pos = lcg.next_index(64);
                    replicas[replica]
                        .insert_text(field, pos, word)
                        .expect("insert");
                }
                2 => {
                    let pos = lcg.next_index(64);
                    let len = lcg.next_index(8);
                    replicas[replica]
                        .delete_text(field, pos, len)
                        .expect("delete");
                }
                _ => {
                    replicas[replica]
                        .set_field(format!("tag{step}").as_str(), format!("v{step}").as_str())
                        .expect("set");
                }
            }
            // Relay: every few steps one replica publishes to the hub
            // and another pulls whatever the hub holds.
            if step % 5 == 0 {
                let publisher = lcg.next_index(4);
                exchange(&mut hub, &replicas[publisher]);
                let receiver = lcg.next_index(4);
                exchange(&mut replicas[receiver], &hub);
            }
        }

        // Final gossip: each source is swapped out, merged into every
        // other replica, then returned. Two rounds make the merge
        // graph fully connected.
        for _round in 0..2 {
            for j in 0..replicas.len() {
                let source = replicas.swap_remove(j);
                let bytes = source.as_dyn().export_state();
                for target in &mut replicas {
                    target
                        .as_dyn_mut()
                        .import_state(&bytes)
                        .expect("valid state");
                }
                replicas.push(source);
            }
        }

        let digests: Vec<u64> = replicas
            .iter()
            .map(SyncDocumentEngine::view_digest)
            .collect();
        let first = digests[0];
        assert!(
            digests.iter().all(|digest| *digest == first),
            "{name}: star topology converged for all replicas ({digests:?})"
        );
    }
}

#[test]
fn stale_fork_catches_up_without_losing_divergent_work() {
    for (name, mut trunk) in factories(&["body"], &[]) {
        let mut fork = trunk.fork_with_peer(2);
        trunk.insert_text("body", 0, "shared base").expect("insert");
        exchange(&mut fork, &trunk);

        // The trunk moves on; the fork edits from its stale snapshot.
        trunk.insert_text("body", 999, " (trunk)").expect("insert");
        fork.insert_text("body", 0, "fork: ").expect("insert");
        exchange(&mut trunk, &fork);
        exchange(&mut fork, &trunk);

        assert_eq!(
            trunk.view_digest(),
            fork.view_digest(),
            "{name}: stale fork converges after catching up"
        );
        let text = trunk.text_of("body").expect("text");
        assert!(
            text.starts_with("fork: "),
            "{name}: fork write kept ({text})"
        );
        assert!(text.contains("shared base"), "{name}: base kept ({text})");
        assert!(
            text.contains("(trunk)"),
            "{name}: trunk write kept ({text})"
        );
    }
}

#[test]
fn logical_names_never_collide_with_engine_namespaces() {
    let text_fields = ["fields", "s:title", "__altior_crdt_v1_scalars__"];
    let scalar_fields = ["s:status", "__altior_crdt_v1_text_6669656c6473"];
    let mut views = Vec::new();
    for (name, mut engine) in factories(&text_fields, &scalar_fields) {
        engine
            .insert_text("fields", 0, "text-fields")
            .expect("insert");
        engine
            .insert_text("s:title", 0, "literal-prefix")
            .expect("insert");
        engine
            .insert_text("__altior_crdt_v1_scalars__", 0, "reserved-looking")
            .expect("insert");
        engine.set_field("s:status", "scalar-prefix").expect("set");
        engine
            .set_field("__altior_crdt_v1_text_6669656c6473", "encoded-looking")
            .expect("set");
        views.push((name, engine.view()));
    }
    assert_eq!(
        views[0].1, views[1].1,
        "both engines preserve logical names"
    );
    assert_eq!(views[0].1.len(), 5);
}

#[test]
fn schema_rejects_empty_duplicate_conflicting_and_undeclared_fields() {
    assert_eq!(
        DocumentSchema::new(&[""], &[]),
        Err(CrdtError::EmptyFieldName)
    );
    assert_eq!(
        DocumentSchema::new(&["body"], &["body"]),
        Err(CrdtError::FieldTypeConflict {
            field: "body".to_owned()
        })
    );
    let mut engine =
        AnyEngine::with_schema("automerge", &["body"], &["title"]).expect("valid schema");
    assert!(matches!(
        engine.insert_text("missing", 0, "x"),
        Err(CrdtError::UndeclaredField { .. })
    ));
    assert!(matches!(
        engine.set_field("body", "wrong kind"),
        Err(CrdtError::WrongFieldType { .. })
    ));
}

#[test]
fn malformed_oversized_cross_engine_and_cross_schema_state_are_rejected() {
    let mut loro = AnyEngine::with_fields("loro", &["body"]).expect("schema");
    let mut automerge = AnyEngine::with_fields("automerge", &["body"]).expect("schema");
    assert!(matches!(
        loro.import_state(b"not a state"),
        Err(CrdtError::MalformedState { .. })
    ));
    assert!(matches!(
        automerge.import_state(&vec![0; MAX_STATE_BYTES + 1]),
        Err(CrdtError::StateTooLarge { .. })
    ));

    let loro_state = loro.export_state();
    assert!(matches!(
        automerge.import_state(&loro_state),
        Err(CrdtError::EngineMismatch { .. })
    ));
    let mut other_schema = AnyEngine::with_fields("loro", &["other"]).expect("schema");
    assert_eq!(
        other_schema.import_state(&loro_state),
        Err(CrdtError::SchemaMismatch)
    );

    let mut bad_version = loro_state;
    bad_version[4] = 99;
    assert_eq!(
        loro.import_state(&bad_version),
        Err(CrdtError::UnsupportedStateVersion { found: 99 })
    );
}
