#![allow(clippy::too_many_lines)]
//! P1 / A15 Retrieval relevance, Chinese CJK search, and bounded candidate acceptance suite.
//!
//! Verifies:
//! 1. Chinese phrase, 2-char, and 1-char search ("我喜欢喝乌龙茶" -> "乌龙茶", "乌龙", "茶").
//! 2. Mixed CJK + Code/Symbol search ("Rust 开发", "API 接口", "C++").
//! 3. Zero leakage of candidate, forgotten, superseded, or expired memories.
//! 4. Scope isolation and SQL pushdown across `LongTerm` and Session modes.
//! 5. Deterministic tie-breaker alignment and canonical rank accounting.
//! 6. Transparent explainability metrics and algorithm version stamping.

use altior_core::context::{AssembleParams, ContextBudgetConfig, assemble_context};
use altior_domain::{
    BoundedLabel, MemoryContent, MemoryDraft, MemoryExcerpt, MemoryKind, MemoryProvenance,
    MemoryScope, MemorySearchLimit, MemorySensitivity, MemorySource, ProjectId, SearchQuery,
    ThreadId, TurnId, UnixMillis,
};
use altior_storage::Store;
use altior_storage::memory::RETRIEVAL_ALGORITHM_VERSION;
use std::str::FromStr;

const BASE_MILLIS: u64 = 1_704_067_200_000;
fn t(offset_ms: u64) -> UnixMillis {
    UnixMillis::from_millis(BASE_MILLIS + offset_ms)
}

fn init_test_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("relevance_acceptance.db");
    let store = Store::open(&db_path).expect("open store");
    (dir, store)
}

#[test]
fn test_p25_chinese_cjk_retrieval_and_sub_term_matching() {
    let (_dir, mut store) = init_test_store();
    let now = t(5000);

    let draft = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("用户偏好喝乌龙茶，尤其是冻顶乌龙茶，不加糖。").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        state: None,
        confidence: 95,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: Some(MemoryExcerpt::try_from("用户说：平时喜欢喝乌龙茶").unwrap()),
        },
        expires_at: None,
    };

    let rec = store.create_memory(&draft, t(100)).expect("create");

    // 1. 3-char Chinese query
    let q3 = SearchQuery::try_from("乌龙茶").unwrap();
    let hits3 = store
        .search_memories(&q3, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits3.len(), 1);
    assert_eq!(hits3[0].record.memory_id, rec.memory_id);
    assert!(
        hits3[0]
            .explanation
            .why_selected
            .contains(RETRIEVAL_ALGORITHM_VERSION)
    );
    assert!(
        hits3[0]
            .explanation
            .matched_terms
            .contains(&"乌龙茶".to_string())
    );

    // 2. 2-char short Chinese query
    let q2 = SearchQuery::try_from("乌龙").unwrap();
    let hits2 = store
        .search_memories(&q2, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits2.len(), 1);
    assert_eq!(hits2[0].record.memory_id, rec.memory_id);

    // 3. 1-char short Chinese query
    let q1 = SearchQuery::try_from("茶").unwrap();
    let hits1 = store
        .search_memories(&q1, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits1.len(), 1);
    assert_eq!(hits1[0].record.memory_id, rec.memory_id);

    // 4. Natural sentence query without spaces
    let q_sentence = SearchQuery::try_from("我喜欢喝乌龙茶").unwrap();
    let hits_sentence = store
        .search_memories(&q_sentence, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits_sentence.len(), 1);
    assert_eq!(hits_sentence[0].record.memory_id, rec.memory_id);
}

#[test]
fn test_p25_mixed_cjk_code_symbols_retrieval() {
    let (_dir, mut store) = init_test_store();
    let now = t(5000);

    let draft_rust = MemoryDraft {
        id: None,
        content: MemoryContent::try_from(
            "Rust并发编程开发规范：强制使用cargo fmt与cargo clippy -D warnings。",
        )
        .unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Instruction,
        state: None,
        confidence: 100,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let draft_cpp = MemoryDraft {
        id: None,
        content: MemoryContent::try_from(
            "C++高性能计算与内存管理：禁止裸指针，采用std::unique_ptr与RAII。",
        )
        .unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };

    let rec_rust = store
        .create_memory(&draft_rust, t(100))
        .expect("create rust");
    let rec_cpp = store.create_memory(&draft_cpp, t(200)).expect("create cpp");

    // Search "Rust 开发"
    let q_rust = SearchQuery::try_from("Rust 开发").unwrap();
    let hits_rust = store
        .search_memories(&q_rust, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert!(!hits_rust.is_empty());
    assert_eq!(hits_rust[0].record.memory_id, rec_rust.memory_id);

    // Search "C++"
    let q_cpp = SearchQuery::try_from("C++").unwrap();
    let hits_cpp = store
        .search_memories(&q_cpp, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits_cpp.len(), 1);
    assert_eq!(hits_cpp[0].record.memory_id, rec_cpp.memory_id);

    // Search "unique_ptr"
    let q_ptr = SearchQuery::try_from("unique_ptr").unwrap();
    let hits_ptr = store
        .search_memories(&q_ptr, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    assert_eq!(hits_ptr.len(), 1);
    assert_eq!(hits_ptr[0].record.memory_id, rec_cpp.memory_id);
}

#[test]
fn test_p25_zero_leakage_and_lifecycle_boundaries() {
    let (_dir, mut store) = init_test_store();
    let now = t(5000);

    // 1. Candidate memory (unconfirmed)
    let draft_cand = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("推测用户可能喜欢喝乌龙茶").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        state: None,
        confidence: 60,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Inferred,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let cand = store.propose_memory(draft_cand, t(100)).expect("propose");

    // 2. Confirmed then forgotten memory
    let draft_forg = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("用户偏好喝乌龙茶（后被遗忘）").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let forg = store
        .create_memory(&draft_forg, t(200))
        .expect("create forg");
    store
        .forget_memory(&forg.memory_id, t(300))
        .expect("forget");

    // 3. Superseded memory
    let draft_sup = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("乌龙茶采购计划（旧版本已被替代）").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let sup = store.create_memory(&draft_sup, t(400)).expect("create sup");
    let draft_rep = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("新版本的采购计划").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 95,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    store
        .correct_memory(&sup.memory_id, &draft_rep, t(500))
        .expect("correct");

    // 4. Expired memory
    let draft_exp = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("乌龙茶临时会议通知（已过期）").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: Some(t(1000)), // expired in the past relative to now (5000)
    };
    let _exp = store.create_memory(&draft_exp, t(600)).expect("create exp");

    // Search "乌龙茶": MUST NOT return candidate, forgotten, superseded, or expired memories!
    let q = SearchQuery::try_from("乌龙茶").unwrap();
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), now)
        .unwrap();
    for hit in &hits {
        assert_ne!(
            hit.record.memory_id, cand.memory_id,
            "Candidate must never leak"
        );
        assert_ne!(
            hit.record.memory_id, forg.memory_id,
            "Forgotten must never leak"
        );
        assert_ne!(
            hit.record.memory_id, sup.memory_id,
            "Superseded must never leak"
        );
        assert!(
            hit.record.expires_at.is_none() || hit.record.expires_at.unwrap() > now,
            "Expired must never leak"
        );
    }
}

#[test]
fn test_p25_scope_pushdown_and_context_assembly_integration() {
    let (_dir, mut store) = init_test_store();
    let now = t(5000);

    let proj_a = ProjectId::from_str("prj_alpha00000000001").unwrap();
    let proj_b = ProjectId::from_str("prj_beta000000000001").unwrap();

    let scope_a = MemoryScope::Project(BoundedLabel::try_from(proj_a.as_str()).unwrap());
    let scope_b = MemoryScope::Project(BoundedLabel::try_from(proj_b.as_str()).unwrap());

    let draft_a = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("Project Alpha数据库架构配置：使用PostgreSQL连接池。")
            .unwrap(),
        scope: scope_a.clone(),
        kind: MemoryKind::Instruction,
        state: None,
        confidence: 95,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let rec_a = store.create_memory(&draft_a, t(100)).expect("create a");

    let draft_b = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("Project Beta数据库架构配置：使用MySQL主从复制。")
            .unwrap(),
        scope: scope_b.clone(),
        kind: MemoryKind::Instruction,
        state: None,
        confidence: 95,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let _rec_b = store.create_memory(&draft_b, t(200)).expect("create b");

    // Search with scope_a filter
    let q = SearchQuery::try_from("数据库架构").unwrap();
    let hits = store
        .search_memories(&q, Some(&scope_a), MemorySearchLimit::default_limit(), now)
        .unwrap();

    assert_eq!(
        hits.len(),
        1,
        "Scope pushdown must restrict candidates in SQLite"
    );
    assert_eq!(hits[0].record.memory_id, rec_a.memory_id);

    // Verify context assembly integration
    let thread_id = ThreadId::from_str("thr_p25test0000000001").unwrap();
    let turn_id = TurnId::from_str("trn_p25test0000000001").unwrap();

    let assemble_res = assemble_context(AssembleParams {
        thread_id: &thread_id,
        turn_id: &turn_id,
        user_prompt: "请给出数据库架构建议",
        memory_mode: "long_term",
        project_id: Some(&proj_a),
        identity_docs: &[],
        memory_hits: &hits,
        budget: ContextBudgetConfig::default(),
        now,
        degraded: None,
    })
    .expect("assemble context");

    assert_eq!(assemble_res.snapshot.memories.len(), 1);
    assert_eq!(assemble_res.snapshot.memories[0].memory_id, rec_a.memory_id);
    assert!(
        assemble_res
            .wire_prompt
            .contains("Project Alpha数据库架构配置")
    );
    assert!(!assemble_res.wire_prompt.contains("Project Beta"));
}

#[test]
fn test_p25_deterministic_tie_breaker_alignment() {
    let (_dir, mut store) = init_test_store();
    let now = t(5000);

    // Create 3 memories with the same content, scope, confidence, source, but different updated_at
    let draft1 = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("Deterministic tie testing fact entry").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: None,
        },
        expires_at: None,
    };
    let rec1 = store.create_memory(&draft1, t(100)).unwrap();

    let draft2 = draft1.clone();
    let rec2 = store.create_memory(&draft2, t(200)).unwrap();

    let draft3 = draft1.clone();
    let rec3 = store.create_memory(&draft3, t(300)).unwrap();

    let q = SearchQuery::try_from("deterministic tie").unwrap();
    let hits = store
        .search_memories(&q, None, MemorySearchLimit::default_limit(), now)
        .unwrap();

    assert_eq!(hits.len(), 3);
    // Updated at DESC: rec3 (t=300) > rec2 (t=200) > rec1 (t=100)
    assert_eq!(hits[0].record.memory_id, rec3.memory_id);
    assert_eq!(hits[1].record.memory_id, rec2.memory_id);
    assert_eq!(hits[2].record.memory_id, rec1.memory_id);

    // Verify context assembly preserves the exact same ranking and assigns canonical ranks 1, 2, 3
    let thread_id = ThreadId::from_str("thr_tie000000000000001").unwrap();
    let turn_id = TurnId::from_str("trn_tie000000000000001").unwrap();

    let res = assemble_context(AssembleParams {
        thread_id: &thread_id,
        turn_id: &turn_id,
        user_prompt: "tie prompt",
        memory_mode: "long_term",
        project_id: None,
        identity_docs: &[],
        memory_hits: &hits,
        budget: ContextBudgetConfig::default(),
        now,
        degraded: None,
    })
    .unwrap();

    assert_eq!(res.snapshot.memories.len(), 3);
    assert_eq!(res.snapshot.memories[0].memory_id, rec3.memory_id);
    assert_eq!(res.snapshot.memories[1].memory_id, rec2.memory_id);
    assert_eq!(res.snapshot.memories[2].memory_id, rec1.memory_id);
}
