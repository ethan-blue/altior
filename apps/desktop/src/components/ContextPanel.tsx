import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import shell from "./shell.module.css";

export interface ContextPanelProps {
  readonly snapshot?: ContextSnapshotDto | null;
}

/**
 * Context Inspector Panel (P2.2 / ADR 0018).
 *
 * Visualizes the deterministic audit record of context assembly for a turn:
 * - Token budget accounting (limit / identity / memory / total)
 * - Selected memories with explainability (why_selected rationale, provenance, score)
 * - Dropped memory candidates with reasons
 * - Degraded assembly badge when retrieval was truncated or impaired
 * - Empty states when no record exists or memory is disabled
 */
export function ContextPanel({ snapshot }: ContextPanelProps) {
  if (!snapshot) {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p className={shell.inspectorEmpty} data-testid="context-empty">
          未记录（暂无上下文快照）
        </p>
      </div>
    );
  }

  if (snapshot.memory_mode === "off") {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p className={shell.inspectorEmpty} data-testid="context-memory-off">
          记忆关闭（未启用记忆检索）
        </p>
      </div>
    );
  }

  return (
    <div className={shell.contextPanel} data-testid="context-panel">
      {/* Degraded Badge */}
      {snapshot.degraded ? (
        <div className={shell.degradedBadge} data-testid="context-degraded-badge">
          <span>⚠️ 降级: {snapshot.degraded.code}</span>
          {snapshot.degraded.detail ? (
            <span className={shell.degradedDetail}>({snapshot.degraded.detail})</span>
          ) : null}
        </div>
      ) : null}

      {/* Token Budget Accounting */}
      <section className={shell.contextSection} aria-label="Token Budget" data-testid="context-budget">
        <h3 className={shell.sectionSubtitle}>Token Budget</h3>
        <dl className={shell.inspectorFields}>
          <dt>Identity Limit</dt>
          <dd className={shell.mono}>{snapshot.budget.identity_limit_tokens}</dd>
          <dt>Memory Limit</dt>
          <dd className={shell.mono}>{snapshot.budget.memory_limit_tokens}</dd>
          <dt>Identity</dt>
          <dd className={shell.mono}>
            {snapshot.budget.identity_tokens} / {snapshot.budget.identity_limit_tokens} tokens
          </dd>
          <dt>Memory</dt>
          <dd className={shell.mono}>
            {snapshot.budget.memory_tokens} / {snapshot.budget.memory_limit_tokens} tokens
          </dd>
          <dt>Prompt</dt>
          <dd className={shell.mono}>{snapshot.budget.prompt_tokens} tokens</dd>
          <dt>Total</dt>
          <dd className={shell.mono}>{snapshot.budget.total_tokens} tokens</dd>
        </dl>
      </section>

      {/* Selected Memories */}
      <section className={shell.contextSection} aria-label="Selected Memories" data-testid="context-memories">
        <h3 className={shell.sectionSubtitle}>
          Selected Memories ({snapshot.memories.length})
        </h3>
        {snapshot.memories.length === 0 ? (
          <p className={shell.inspectorEmpty}>无选中的记忆条目</p>
        ) : (
          <div className={shell.memoryList}>
            {snapshot.memories.map((mem) => {
              const customSummary =
                (mem as unknown as { excerpt?: string }).excerpt ||
                (mem as unknown as { content?: string }).content ||
                mem.memory_id;
              return (
                <div
                  key={mem.memory_id}
                  className={shell.memoryCard}
                  data-testid={`context-memory-${mem.memory_id}`}
                >
                  <div className={shell.memoryCardHeader}>
                    <span className={shell.mono}>{mem.memory_id}</span>
                    <span className={shell.memoryKindBadge}>{mem.kind}</span>
                    <span className={shell.memoryConfidence}>{mem.confidence}%</span>
                  </div>
                  <dl className={shell.inspectorFields} style={{ marginTop: "var(--spacing-6)" }}>
                    <dt>内容摘要</dt>
                    <dd data-testid="memory-summary">{customSummary}</dd>

                    <dt>Kind</dt>
                    <dd>{mem.kind}</dd>

                    <dt>Confidence</dt>
                    <dd>{mem.confidence}%</dd>

                    <dt>Tokens / Score</dt>
                    <dd className={shell.mono}>
                      {mem.tokens} tokens · total_score: {mem.score}
                    </dd>

                    <dt>Why selected</dt>
                    <dd>
                      <div className={shell.mono} data-testid="memory-why-selected">
                        {mem.why_selected}
                      </div>
                    </dd>

                    <dt>来源 Provenance</dt>
                    <dd data-testid="memory-provenance">
                      <div>thread_id: <span className={shell.mono}>{mem.provenance_thread_id ?? "unknown"}</span></div>
                      <div>turn_id: <span className={shell.mono}>{mem.provenance_turn_id ?? "unknown"}</span></div>
                      <div>excerpt: <span>{(mem as unknown as { excerpt?: string }).excerpt ?? "(none)"}</span></div>
                    </dd>
                  </dl>
                </div>
              );
            })}
          </div>
        )}
      </section>

      {/* Dropped Entries */}
      {snapshot.dropped && snapshot.dropped.length > 0 ? (
        <section className={shell.contextSection} aria-label="Dropped Entries" data-testid="context-dropped">
          <h3 className={shell.sectionSubtitle}>
            Dropped Entries ({snapshot.dropped.length})
          </h3>
          <ul className={shell.droppedList} data-testid="context-dropped-list">
            {snapshot.dropped.map((item, idx) => (
              <li
                key={`${item.memory_id}-${idx}`}
                className={shell.droppedItem}
                data-testid={`dropped-item-${item.memory_id}`}
              >
                <span className={shell.mono}>{item.memory_id}</span>
                <span className={shell.droppedReason}>({item.reason})</span>
                <span className={shell.droppedMeta}>
                  rank #{item.rank} · {item.tokens} tokens
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {/* Identity Documents */}
      {snapshot.identity && snapshot.identity.length > 0 ? (
        <section className={shell.contextSection} aria-label="Identity Documents" data-testid="context-identity">
          <h3 className={shell.sectionSubtitle}>
            Identity Documents ({snapshot.identity.length})
          </h3>
          <ul className={shell.droppedList}>
            {snapshot.identity.map((idoc) => (
              <li key={idoc.document_id} className={shell.droppedItem}>
                <span className={shell.mono}>{idoc.document_id}</span>
                <span className={shell.memoryKindBadge}>{idoc.kind}</span>
                <span className={shell.mono}>{idoc.tokens} tokens</span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
