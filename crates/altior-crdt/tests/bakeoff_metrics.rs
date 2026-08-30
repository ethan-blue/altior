//! Deterministic bake-off metrics (ADR 0010).
//!
//! One fixed 1000-op script, run identically on both engines. No
//! concurrency, so the resulting views must be byte-identical across
//! engines; the encoded document sizes are the recorded bake-off
//! numbers (printed here, cited in the ADR). The assertions are
//! intra-library determinism and cross-engine agreement — never
//! timing, which AGENTS.md keeps out of default gates.

use altior_crdt::{AnyEngine, FieldView, Lcg, SyncDocumentEngine};

/// Runs the fixed script on a fresh engine, returning
/// `(state_size, view)`.
fn run_script(kind: &str) -> (usize, Vec<(String, FieldView)>) {
    let fields = ["body", "outline", "scratch", "notes"];
    let scalars = [
        "tag0", "tag1", "tag2", "tag3", "tag4", "tag5", "tag6", "tag7",
    ];
    let words = [
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
    ];
    let mut engine = AnyEngine::with_schema(kind, &fields, &scalars).expect("valid schema");
    let mut lcg = Lcg::seeded(0x000b_0ca1);
    for step in 0..1000u64 {
        let field = fields[lcg.next_index(fields.len())];
        match lcg.next(5) {
            0..=2 => {
                let word = words[lcg.next_index(words.len())];
                let pos = lcg.next_index(128);
                engine.insert_text(field, pos, word).expect("insert");
            }
            3 => {
                let pos = lcg.next_index(128);
                let len = lcg.next_index(16);
                engine.delete_text(field, pos, len).expect("delete");
            }
            _ => {
                engine
                    .set_field(
                        format!("tag{}", step % 8).as_str(),
                        format!("value{step}").as_str(),
                    )
                    .expect("set");
            }
        }
    }
    (engine.state_size(), engine.view())
}

#[test]
fn fixed_script_is_deterministic_and_prints_the_bakeoff_table() {
    let (loro_size, loro_view) = run_script("loro");
    let (automerge_size, automerge_view) = run_script("automerge");

    // Intra-library determinism: the same script twice, same bytes.
    let (loro_size_again, _) = run_script("loro");
    let (automerge_size_again, _) = run_script("automerge");
    assert_eq!(loro_size, loro_size_again, "loro encoding is deterministic");
    assert_eq!(
        automerge_size, automerge_size_again,
        "automerge encoding is deterministic"
    );

    // Cross-engine agreement: a sequential script has one correct view.
    assert_eq!(
        loro_view, automerge_view,
        "both engines fold the sequential script identically"
    );

    println!("bake-off (1000 deterministic ops, 4 text fields + 8 scalars):");
    println!("  loro       state bytes: {loro_size}");
    println!("  automerge  state bytes: {automerge_size}");
    let total_chars: usize = loro_view
        .iter()
        .map(|(_, value)| match value {
            FieldView::Text(text) => text.chars().count(),
            FieldView::Scalar(value) => value.chars().count(),
        })
        .sum();
    println!("  view chars both engines: {total_chars}");
    assert!(loro_size > 0);
    assert!(automerge_size > 0);
    assert!(total_chars > 0);
}
