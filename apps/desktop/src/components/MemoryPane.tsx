import { useState } from "react";
import { useI18n } from "../i18n";
import {
  localizeMemoryKind,
  localizeMemoryScope,
  localizeMemoryState,
} from "../i18n/localizeEnums";
import type { MemoryRecordDto } from "../ipc/dto/MemoryRecordDto";
import type { AgentProfile } from "../stores/applicationStore";
import styles from "./memoryPane.module.css";


function localizeMemorySource(
  source: string,
  t: ReturnType<typeof useI18n>["t"],
): string {
  if (source === "explicit") return t.memoryPane.sourceExplicit;
  if (source === "inferred") return t.memoryPane.sourceInferred;
  return source;
}



export interface MemoryPaneProps {
  readonly memories: readonly MemoryRecordDto[];
  readonly isLoading?: boolean;
  readonly activeAgent?: AgentProfile | null;
  readonly onSetAgentMemoryMode?: (agentId: string, mode: "off" | "session" | "long_term") => void;
  readonly onProposeMemory?: (params: {
    content: string;
    scope_kind: string;
    scope_target?: string | null;
    kind: string;
    source?: string;
  }) => Promise<unknown>;
  readonly onConfirmMemory?: (memoryId: string) => Promise<unknown>;
  readonly onRejectMemory?: (memoryId: string) => Promise<unknown>;
  readonly onCorrectMemory?: (params: { memory_id: string; content: string }) => Promise<unknown>;
  readonly onForgetMemory?: (memoryId: string) => Promise<unknown>;
}

export function MemoryPane({
  memories,
  isLoading = false,
  activeAgent,
  onSetAgentMemoryMode,
  onProposeMemory,
  onConfirmMemory,
  onRejectMemory,
  onCorrectMemory,
  onForgetMemory,
}: MemoryPaneProps) {
  const { t } = useI18n();
  const [stateFilter, setStateFilter] = useState<string>("all");
  const [scopeFilter, setScopeFilter] = useState<string>("all");
  const [showAddForm, setShowAddForm] = useState(false);

  // New memory draft state
  const [newContent, setNewContent] = useState("");
  const [newScopeKind, setNewScopeKind] = useState("global");
  const [newScopeTarget, setNewScopeTarget] = useState("");
  const [newKind, setNewKind] = useState("fact");
  const [newIsExplicit, setNewIsExplicit] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Correction state: memoryId -> draft content
  const [editingMemoryId, setEditingMemoryId] = useState<string | null>(null);
  const [editContent, setEditContent] = useState("");

  const filteredMemories = memories.filter((m) => {
    if (stateFilter !== "all" && m.state !== stateFilter) return false;
    if (scopeFilter !== "all" && m.scope_kind !== scopeFilter) return false;
    return true;
  });

  const handlePropose = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newContent.trim() || !onProposeMemory) return;
    setIsSubmitting(true);
    try {
      await onProposeMemory({
        content: newContent.trim(),
        scope_kind: newScopeKind,
        scope_target: newScopeKind === "global" ? null : (newScopeTarget.trim() || null),
        kind: newKind,
        source: newIsExplicit ? "explicit" : "inferred",
      });
      setNewContent("");
      setNewScopeTarget("");
      setShowAddForm(false);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleStartCorrect = (mem: MemoryRecordDto) => {
    setEditingMemoryId(mem.memory_id);
    setEditContent(mem.content);
  };

  const handleSaveCorrect = async (memoryId: string) => {
    if (!editContent.trim() || !onCorrectMemory) return;
    try {
      await onCorrectMemory({
        memory_id: memoryId,
        content: editContent.trim(),
      });
      setEditingMemoryId(null);
      setEditContent("");
    } catch {
      // Error handled by caller
    }
  };

  return (
    <section className={styles.memoryPane} aria-label={t.memoryPane.title} data-testid="memory-pane">
      {/* Header with Title & Action Controls */}
      <header className={styles.header}>
        <div className={styles.titleGroup}>
          <h2 className={styles.title}>{t.memoryPane.title}</h2>
          <span className={styles.stats}>
            {isLoading ? t.contextPanel.loading : t.memoryPane.recordsCount(filteredMemories.length, memories.length)}
          </span>
        </div>

        <div className={styles.controls}>
          {/* State Filter */}
          <select
            className={styles.select}
            value={stateFilter}
            onChange={(e) => setStateFilter(e.target.value)}
            aria-label={t.memoryPane.filterStateAria}
            data-testid="memory-state-filter"
          >
            <option value="all">{t.memoryPane.allStates}</option>
            <option value="confirmed">{t.memoryPane.stateConfirmed}</option>
            <option value="candidate">{t.memoryPane.stateCandidate}</option>
            <option value="superseded">{t.memoryPane.stateSuperseded}</option>
            <option value="forgotten">{t.memoryPane.stateForgotten}</option>
            <option value="rejected">{t.memoryPane.stateRejected}</option>
          </select>

          {/* Scope Filter */}
          <select
            className={styles.select}
            value={scopeFilter}
            onChange={(e) => setScopeFilter(e.target.value)}
            aria-label={t.memoryPane.filterScopeAria}
            data-testid="memory-scope-filter"
          >
            <option value="all">{t.memoryPane.allScopes}</option>
            <option value="global">{t.memoryPane.scopeGlobal}</option>
            <option value="project">{t.memoryPane.scopeProject}</option>
            <option value="person">{t.memoryPane.scopePerson}</option>
            <option value="thread">{t.memoryPane.scopeThread}</option>
          </select>

          {/* Toggle Add Form */}
          <button
            type="button"
            className={styles.primaryButton}
            onClick={() => setShowAddForm((prev) => !prev)}
            data-testid="memory-add-button"
          >
            {showAddForm ? t.memoryPane.cancel : t.memoryPane.addMemory}
          </button>
        </div>
      </header>

      {/* Agent Memory Mode Setting */}
      {activeAgent ? (
        <div className={styles.agentModeBar} data-testid="agent-memory-mode-bar">
          <span className={styles.agentModeLabel}>{t.memoryPane.agentMemoryMode(activeAgent.name)}</span>
          <select
            className={styles.select}
            value={activeAgent.memory_mode ?? "session"}
            onChange={(e) =>
              onSetAgentMemoryMode?.(
                activeAgent.id,
                e.target.value as "off" | "session" | "long_term",
              )
            }
            aria-label={t.memoryPane.agentModeAria}
            data-testid="agent-memory-mode-select"
          >
            <option value="off">{t.memoryPane.modeOff}</option>
            <option value="session">{t.memoryPane.modeSession}</option>
            <option value="long_term">{t.memoryPane.modeLongTerm}</option>
          </select>
        </div>
      ) : null}

      {/* Add Memory Form */}
      {showAddForm ? (
        <form className={styles.formCard} onSubmit={handlePropose} data-testid="memory-propose-form">
          <textarea
            className={styles.textarea}
            placeholder={t.memoryPane.placeholder}
            value={newContent}
            onChange={(e) => setNewContent(e.target.value)}
            required
            data-testid="memory-content-input"
          />

          <div className={styles.formGrid}>
            <div>
              <label htmlFor="memory-scope-kind" className={styles.formFieldLabel}>
                {t.memoryPane.scopeLabel}
              </label>
              <select
                id="memory-scope-kind"
                className={styles.select}
                value={newScopeKind}
                onChange={(e) => setNewScopeKind(e.target.value)}
                data-testid="memory-scope-kind-select"
              >
                <option value="global">{t.memoryPane.scopeGlobal}</option>
                <option value="project">{t.memoryPane.scopeProject}</option>
                <option value="person">{t.memoryPane.scopePerson}</option>
                <option value="thread">{t.memoryPane.scopeThread}</option>
              </select>
            </div>

            {newScopeKind !== "global" ? (
              <div>
                <label htmlFor="memory-scope-target" className={styles.formFieldLabel}>
                  {t.memoryPane.targetLabel}
                </label>
                <input
                  id="memory-scope-target"
                  type="text"
                  className={styles.input}
                  placeholder={t.memoryPane.targetPlaceholder}
                  value={newScopeTarget}
                  onChange={(e) => setNewScopeTarget(e.target.value)}
                  data-testid="memory-scope-target-input"
                />
              </div>
            ) : null}

            <div>
              <label htmlFor="memory-kind" className={styles.formFieldLabel}>
                {t.memoryPane.kindLabel}
              </label>
              <select
                id="memory-kind"
                className={styles.select}
                value={newKind}
                onChange={(e) => setNewKind(e.target.value)}
                data-testid="memory-kind-select"
              >
                <option value="fact">{t.memoryPane.kindFact}</option>
                <option value="preference">{t.memoryPane.kindPreference}</option>
                <option value="instruction">{t.memoryPane.kindInstruction}</option>
                <option value="summary">{t.memoryPane.kindSummary}</option>
              </select>
            </div>

            <div className={styles.explicitCheckRow}>
              <input
                id="memory-explicit"
                type="checkbox"
                checked={newIsExplicit}
                onChange={(e) => setNewIsExplicit(e.target.checked)}
                data-testid="memory-explicit-check"
              />
              <label htmlFor="memory-explicit" className={styles.explicitCheckLabel}>
                {t.memoryPane.confirmDirectly}
              </label>
            </div>
          </div>

          <div className={styles.formActions}>
            <button
              type="button"
              className={styles.btnAction}
              onClick={() => setShowAddForm(false)}
            >
              {t.memoryPane.cancel}
            </button>
            <button
              type="submit"
              className={styles.primaryButton}
              disabled={isSubmitting || !newContent.trim()}
              data-testid="memory-submit-btn"
            >
              {isSubmitting ? t.memoryPane.submitting : t.memoryPane.saveMemory}
            </button>
          </div>
        </form>
      ) : null}

      {/* Memory Cards List */}
      <div className={styles.cardList} data-testid="memory-card-list">
        {filteredMemories.length === 0 ? (
          <p className={styles.emptyMessage} data-testid="memory-empty">
            {memories.length === 0
              ? t.memoryPane.empty
              : t.memoryPane.emptyFiltered}
          </p>
        ) : (
          filteredMemories.map((mem) => {
            const isEditing = editingMemoryId === mem.memory_id;
            return (
              <article
                key={mem.memory_id}
                className={styles.card}
                data-testid={`memory-card-${mem.memory_id}`}
              >
                <div className={styles.cardHeader}>
                  <div className={styles.idAndBadges}>
                    <span className={styles.mono}>{mem.memory_id}</span>
                    <span
                      className={`${styles.badge} ${
                        mem.state === "confirmed"
                          ? styles.badgeConfirmed
                          : mem.state === "candidate"
                            ? styles.badgeCandidate
                            : mem.state === "rejected"
                              ? styles.badgeRejected
                              : mem.state === "superseded"
                                ? styles.badgeSuperseded
                                : styles.badgeForgotten
                      }`}
                      data-testid="memory-state-badge"
                    >
                      {localizeMemoryState(mem.state, t)}
                    </span>
                    <span className={styles.badge}>{localizeMemoryKind(mem.kind, t)}</span>
                    <span className={styles.stats}>{mem.confidence}% {t.contextPanel.confidence}</span>
                  </div>
                  <span className={styles.stats}>
                    {new Date(mem.updated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </span>
                </div>

                {isEditing ? (
                  <div className={styles.editColumn}>
                    <textarea
                      className={styles.textarea}
                      value={editContent}
                      onChange={(e) => setEditContent(e.target.value)}
                      data-testid="memory-edit-input"
                    />
                    <div className={styles.editActions}>
                      <button
                        type="button"
                        className={styles.btnAction}
                        onClick={() => setEditingMemoryId(null)}
                      >
                        {t.memoryPane.cancel}
                      </button>
                      <button
                        type="button"
                        className={styles.primaryButton}
                        onClick={() => handleSaveCorrect(mem.memory_id)}
                        data-testid="memory-save-correct-btn"
                      >
                        {t.memoryPane.save}
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className={styles.content} data-testid="memory-card-content">
                    {mem.content}
                  </div>
                )}

                <div className={styles.metaRow}>
                  <span>{t.memoryPane.scopeLabel}: <strong>{localizeMemoryScope(mem.scope_kind, t)}</strong>{mem.scope_target ? ` (${mem.scope_target})` : ""}</span>
                  <span data-testid="memory-source-label">{t.memoryPane.sourceLabel}: <strong>{localizeMemorySource(mem.source, t)}</strong></span>
                  {mem.provenance_thread_id ? (
                    <span>{t.memoryPane.originLabel}: <span className={styles.mono}>{mem.provenance_thread_id}</span></span>
                  ) : null}
                  {mem.superseded_by ? (
                    <span>{t.memoryPane.supersededByLabel}: <span className={styles.mono}>{mem.superseded_by}</span></span>
                  ) : null}
                </div>

                {/* Actions per state */}
                <div className={styles.cardActions}>
                  {mem.state === "candidate" ? (
                    <>
                      <button
                        type="button"
                        className={`${styles.btnAction} ${styles.btnConfirm}`}
                        onClick={() => onConfirmMemory?.(mem.memory_id)}
                        data-testid="memory-confirm-btn"
                      >
                        ✓ {t.memoryPane.confirm}
                      </button>
                      <button
                        type="button"
                        className={`${styles.btnAction} ${styles.btnReject}`}
                        onClick={() => onRejectMemory?.(mem.memory_id)}
                        data-testid="memory-reject-btn"
                      >
                        × {t.memoryPane.reject}
                      </button>
                    </>
                  ) : null}

                  {mem.state === "confirmed" && !isEditing ? (
                    <>
                      <button
                        type="button"
                        className={styles.btnAction}
                        onClick={() => handleStartCorrect(mem)}
                        data-testid="memory-correct-btn"
                      >
                        ✎ {t.memoryPane.correct}
                      </button>
                      <button
                        type="button"
                        className={styles.btnAction}
                        onClick={() => onForgetMemory?.(mem.memory_id)}
                        data-testid="memory-forget-btn"
                      >
                        🗑 {t.memoryPane.forget}
                      </button>
                    </>
                  ) : null}
                </div>
              </article>
            );
          })
        )}
      </div>
    </section>
  );
}
