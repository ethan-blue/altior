import { useI18n } from "../i18n";
import { localizeMemoryKind } from "../i18n/localizeEnums";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import shell from "./shell.module.css";

export interface ContextPanelProps {
  readonly snapshot?: ContextSnapshotDto | null;
  readonly status?: "idle" | "loading" | "loaded" | "not_found" | "error";
  readonly error?: string | null;
}

/**
 * Context Inspector Panel (P2.2 / ADR 0018).
 *
 * Visualizes the deterministic audit record of context assembly for a turn:
 * - Token budget accounting (limit / identity / memory / total)
 * - Selected memories with explainability (why_selected rationale, provenance, score)
 * - Dropped memory candidates with reasons
 * - Degraded assembly badge when retrieval was truncated or impaired
 * - Distinct states: loading, error, not_found / empty, memory disabled (off), and loaded
 */
export function ContextPanel({ snapshot, status, error }: ContextPanelProps) {
  const { t } = useI18n();

  if (status === "loading") {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p className={shell.inspectorEmpty} data-testid="context-loading">
          {t.contextPanel.loading}
        </p>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p
          className={shell.inspectorEmpty}
          data-testid="context-error"
          style={{ color: "var(--color-danger, #b3362b)" }}
        >
          {error || t.contextPanel.loadFailed}
        </p>
      </div>
    );
  }

  if (!snapshot || status === "not_found") {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p className={shell.inspectorEmpty} data-testid="context-empty">
          {t.contextPanel.empty}
        </p>
      </div>
    );
  }

  if (snapshot.memory_mode === "off") {
    return (
      <div className={shell.contextPanel} data-testid="context-panel">
        <p className={shell.inspectorEmpty} data-testid="context-memory-off">
          {t.contextPanel.memoryOff}
        </p>
      </div>
    );
  }

  return (
    <div className={shell.contextPanel} data-testid="context-panel">
      {/* Degraded Badge */}
      {snapshot.degraded ? (
        <div className={shell.degradedBadge} data-testid="context-degraded-badge">
          <span>{t.contextPanel.degraded(snapshot.degraded.code)}</span>
          {snapshot.degraded.detail ? (
            <span className={shell.degradedDetail}>({snapshot.degraded.detail})</span>
          ) : null}
        </div>
      ) : null}

      {/* Token Budget Accounting */}
      <section className={shell.contextSection} aria-label={t.contextPanel.tokenBudget} data-testid="context-budget">
        <h3 className={shell.sectionSubtitle}>{t.contextPanel.tokenBudget}</h3>
        <dl className={shell.inspectorFields}>
          <dt>{t.contextPanel.identityLimit}</dt>
          <dd className={shell.mono}>{snapshot.budget.identity_limit_tokens}</dd>
          <dt>{t.contextPanel.memoryLimit}</dt>
          <dd className={shell.mono}>{snapshot.budget.memory_limit_tokens}</dd>
          <dt>{t.contextPanel.identityTokens}</dt>
          <dd className={shell.mono}>
            {t.contextPanel.tokensFraction(
              snapshot.budget.identity_tokens,
              snapshot.budget.identity_limit_tokens,
            )}
          </dd>
          <dt>{t.contextPanel.memoryTokens}</dt>
          <dd className={shell.mono}>
            {t.contextPanel.tokensFraction(
              snapshot.budget.memory_tokens,
              snapshot.budget.memory_limit_tokens,
            )}
          </dd>
          <dt>{t.contextPanel.promptTokens}</dt>
          <dd className={shell.mono}>{t.contextPanel.tokensCount(snapshot.budget.prompt_tokens)}</dd>
          <dt>{t.contextPanel.totalTokens}</dt>
          <dd className={shell.mono}>{t.contextPanel.tokensCount(snapshot.budget.total_tokens)}</dd>
        </dl>
      </section>

      {/* Selected Memories */}
      <section className={shell.contextSection} aria-label={t.contextPanel.selectedMemories(snapshot.memories.length)} data-testid="context-memories">
        <h3 className={shell.sectionSubtitle}>
          {t.contextPanel.selectedMemories(snapshot.memories.length)}
        </h3>
        {snapshot.memories.length === 0 ? (
          <p className={shell.inspectorEmpty}>{t.contextPanel.zeroSelected}</p>
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
                    <span className={shell.memoryKindBadge}>{localizeMemoryKind(mem.kind, t)}</span>
                    <span className={shell.memoryConfidence}>{mem.confidence}%</span>
                  </div>
                  <dl className={shell.inspectorFields} style={{ marginTop: "var(--spacing-6)" }}>
                    <dt>{t.contextPanel.summary}</dt>
                    <dd data-testid="memory-summary">{customSummary}</dd>

                    <dt>{t.inspector.kind}</dt>
                    <dd>{localizeMemoryKind(mem.kind, t)}</dd>

                    <dt>{t.contextPanel.confidence}</dt>
                    <dd>{mem.confidence}%</dd>

                    <dt>{t.contextPanel.tokensScore}</dt>
                    <dd className={shell.mono}>
                      {t.contextPanel.tokensScoreValue(mem.tokens, mem.score)}
                    </dd>

                    <dt>{t.contextPanel.whySelected}</dt>
                    <dd>
                      <div className={shell.mono} data-testid="memory-why-selected">
                        {mem.why_selected}
                      </div>
                    </dd>

                    <dt>{t.contextPanel.provenance}</dt>
                    <dd data-testid="memory-provenance">
                      <div>
                        {t.contextPanel.provenanceThread}:{" "}
                        <span className={shell.mono}>
                          {mem.provenance_thread_id ?? t.contextPanel.unknownValue}
                        </span>
                      </div>
                      <div>
                        {t.contextPanel.provenanceTurn}:{" "}
                        <span className={shell.mono}>
                          {mem.provenance_turn_id ?? t.contextPanel.unknownValue}
                        </span>
                      </div>
                      <div>
                        {t.contextPanel.provenanceExcerpt}:{" "}
                        <span>
                          {(mem as unknown as { excerpt?: string }).excerpt ??
                            t.contextPanel.noneValue}
                        </span>
                      </div>
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
        <section className={shell.contextSection} aria-label={t.contextPanel.droppedMemories(snapshot.dropped.length)} data-testid="context-dropped">
          <h3 className={shell.sectionSubtitle}>
            {t.contextPanel.droppedMemories(snapshot.dropped.length)}
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
                  {t.contextPanel.droppedMeta(item.rank, item.tokens)}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {/* Identity Documents */}
      {snapshot.identity && snapshot.identity.length > 0 ? (
        <section
          className={shell.contextSection}
          aria-label={t.contextPanel.identityDocuments(snapshot.identity.length)}
          data-testid="context-identity"
        >
          <h3 className={shell.sectionSubtitle}>
            {t.contextPanel.identityDocuments(snapshot.identity.length)}
          </h3>
          <ul className={shell.droppedList}>
            {snapshot.identity.map((idoc) => (
              <li key={idoc.document_id} className={shell.droppedItem}>
                <span className={shell.mono}>{idoc.document_id}</span>
                <span className={shell.memoryKindBadge}>{localizeMemoryKind(idoc.kind, t)}</span>
                <span className={shell.mono}>
                  {t.contextPanel.tokensCount(idoc.tokens)}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
