# ADR 0023: SQLite FTS5 Trigram Hybrid Retrieval, Bounded Heap Candidate Ranking, and Tie-Breaker Alignment

Date: 2026-09-06 · Status: accepted · Scope: P1 Memory Retrieval Relevance, Chinese CJK Search, and Performance Bounding (F21, F22, F23).

## Context

Review findings identified three architectural deficiencies in memory search retrieval:
1. **F21 — Unbounded Search Workload**: `crates/altior-storage/src/memory.rs:742-853` retrieved all FTS matches from SQLite without scope pushdown, materialized full `MemoryRecord` entities (string contents, excerpts, provenances) and formatted explanation strings for all $M$ hits, sorted the entire $M$-element collection in Rust, and only then truncated to limit $K$. For common search terms, this incurred $O(M)$ memory allocations and $O(M \log M)$ CPU cost.
2. **F22 — BM25 Score Clamp and Tie-Breaker Conflict**: Transforming raw BM25 via `(-raw_bm25).max(0.1)` forcibly clamped all scores below 0.1 to a flat constant, erasing relevance distinctions among weaker matches. Furthermore, storage sorted ties by `updated_at DESC, memory_id DESC`, while context assembly (`crates/altior-core/src/context/mod.rs:298`) re-sorted ties by `created_at ASC, id ASC` and assigned explanation ranks based on pre-sort indices, causing ranking drift and explainability desynchronization.
3. **F23 — Chinese Full-Text Search Failure**: `SCHEMA_V6` configured `memory_fts` with `tokenize='porter unicode61'`. Because natural Chinese sentences do not use whitespace between words, `unicode61` indexed continuous CJK text (e.g. "我喜欢喝乌龙茶") as a single monolithic token. Empirical measurement across a 100-query benchmark revealed a baseline Chinese Recall@8 of only **7.5%** (nDCG@8: 0.0750), with queries like "乌龙茶", "架构", "并发", "索引" completely failing to match.

## Decisions

### 1. Schema Migration v8: FTS5 Trigram Tokenizer (F23)

We introduce `SCHEMA_V8` in `crates/altior-storage/src/migrations.rs`:
- Drops the legacy `memory_fts` virtual table and triggers.
- Creates `memory_fts` using SQLite FTS5 `tokenize='trigram'`.
- Recreates the insert/update/delete triggers on `memory` to keep `memory_fts` strictly synchronized with confirmed, non-superseded records.
- Repopulates `memory_fts` from existing confirmed `memory` rows.
- Preserves 100% of the append-only `domain_journal` and `memory` table data.

**Why Trigram over Unicode61 or Custom Tokenizers**:
- SQLite FTS5 `trigram` is compiled natively into the bundled SQLite C engine; it requires zero external functions or dynamically registered C/Rust callbacks in SQLite triggers.
- Any external SQLite connection (CLI, repair tools, replication) can inspect or rebuild the database without missing custom tokenizer dependencies.
- Trigram enables substring matching anywhere in the text for sequences of $\ge 3$ characters (e.g. "乌龙茶" matches "我喜欢喝乌龙茶，尤其是冻顶乌龙茶").
- Measured storage overhead on 1,000 records: Trigram is 960 KiB vs 480 KiB for Unicode61 (2.0x index ratio), well within desktop vault limits (< 50 MiB at 50,000 memories).

### 2. Hybrid Query Strategy for Short Substrings and English Inflections (F23)

Trigram indexes 3-character slices and returns empty results for queries $< 3$ characters. In Chinese and code identifiers, 1-character and 2-character terms are ubiquitous ("架构", "并发", "乌龙", "缓存", "Go", "C").

`Store::search_memories` adopts a hybrid query architecture:
1. **Token Partitioning**:
   - Query tokens are partitioned into `long_tokens` ($\ge 3$ Unicode characters) and `short_tokens` ($< 3$ characters).
2. **Long Tokens (Trigram FTS5)**:
   - Evaluated using `memory_fts MATCH <fts_expr>`.
   - English tokens are expanded with standard stem variants (e.g. `-ments`, `-ing`, `-ed`, `-s`, `-tion`) to preserve parity with Porter stemmer recall across inflections (achieving 93.8% English recall).
3. **Short Tokens (Exact Substring instr)**:
   - Evaluated using SQLite `instr(lower(m.content), lower(?)) > 0` directly on retrievable records.
   - For short-term matches, an equivalent normalized text rank of `-0.5` is assigned, seamlessly integrating into the composite ranking formula.

### 3. Scope Condition Pushdown to SQLite (F21)

Scope filters are pushed directly into the SQL WHERE clause:
- `MemoryScope::Global`: `AND m.scope_kind = 'global'`
- `MemoryScope::Project(t)`: `AND (m.scope_kind = 'global' OR (m.scope_kind = 'project' AND m.scope_target = ?))`
- `MemoryScope::Person(t)`: `AND (m.scope_kind = 'global' OR (m.scope_kind = 'person' AND m.scope_target = ?))`
- `MemoryScope::Thread(t)`: `AND (m.scope_kind = 'global' OR (m.scope_kind = 'thread' AND m.scope_target = ?))`
- `None`: No scope filter.

This eliminates cross-scope row fetching, deserialization, and filtering in Rust, guaranteeing **zero scope leakage** at the engine boundary.

### 4. Bounded Candidate Evaluation and Lazy Materialization (F21)

To eliminate $O(M)$ memory allocations:
1. **Lightweight Candidate Scan**:
   During cursor traversal, only lightweight fields required for scoring (`memory_id`, `confidence`, `explicit`, `updated_at`, `raw_bm25`, `scope_kind`, `scope_target`) and `content` are fetched.
2. **Bounded Top-K Min-Heap**:
   A Min-Heap of bounded capacity $K = \text{limit}$ ($1 \le K \le 64$, default 8) retains only the top-$K$ candidate tuples. Any candidate scoring lower than the heap minimum is discarded in $O(1)$ without allocating record structures.
3. **Lazy Materialization**:
   Full entity deserialization (`row_to_memory_record`) and match explanation formatting (`MemoryMatchExplanation`, `why_selected`, `matched_terms`) occur **only** for the final top-$K$ items. Materialized records are bounded by $K \le 64$ regardless of total matching rows $M$.

### 5. Deterministic Monotonic BM25 Scoring and Aligned Tie-Breakers (F22)

1. **Eliminate Constant Floor Clamping**:
   `text_score = (-raw_bm25).max(0.0)`. All valid BM25 relevance gradations are strictly preserved without collapsing into 0.1.
2. **Unified Tie-Breaker Order**:
   Both `Store::search_memories` and `context::assemble_context` strictly enforce:
   1. `total_score DESC`
   2. `updated_at DESC`
   3. `memory_id DESC`
3. **Accurate Explanation Ranks**:
   Context assembly assigns candidate ranks from the final sorted rank order ($1..=K$), ensuring audit snapshot ranks match prompt injection order bit-for-bit.

### 6. Algorithm Versioning

The retrieval pipeline records:
`pub const RETRIEVAL_ALGORITHM_VERSION: &str = "v2_bounded_scope_pushdown_trigram_hybrid";`

## Benchmark and Verification Evidence

Measured across 100 benchmark queries (40 Chinese, 40 English, 20 Mixed/Symbol/Code):
- **Overall Recall@8**: Improved from **46.5%** to **91.2%** (+44.7%).
- **Overall nDCG@8**: Improved from **0.4642** to **0.8916** (+42.7%).
- **Chinese Category**: Recall@8 jumped from **7.5%** to **84.2%** (+76.7%); nDCG@8 from **0.0750** to **0.8121**.
- **English Category**: Maintained full parity at **93.8%** Recall@8 (0.9262 nDCG@8).
- **Mixed / Code Category**: Recall@8 reached **100.0%** (nDCG@8: 0.9815).
- **Scope Leakage**: **0** (strictly 0).
- **Disallowed Records Leakage**: **0** (zero candidate, expired, forgotten, or superseded records admitted).
