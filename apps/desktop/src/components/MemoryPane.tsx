import { useRef, useState } from "react";
import { useI18n } from "../i18n";
import {
  localizeMemoryKind,
  localizeMemoryScope,
  localizeMemorySource,
  localizeMemoryState,
} from "../i18n/localizeEnums";
import type { MemoryRecordDto } from "../ipc/dto/MemoryRecordDto";
import type { AgentProfile } from "../stores/applicationStore";
import styles from "./memoryPane.module.css";

export interface MemoryPaneProps {
  readonly memories: readonly MemoryRecordDto[];
  readonly isLoading?: boolean;
  readonly activeAgent?: AgentProfile | null;
  readonly onSetAgentMemoryMode?: (agentId: string, mode: "off" | "session" | "long_term") => void;
  readonly selectedMemoryId?: string | null;
  readonly onSelectMemory?: (memory: MemoryRecordDto) => void;
  readonly onClearMemorySelection?: () => void;
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
  selectedMemoryId = null,
  onSelectMemory,
  onClearMemorySelection,
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
  const [focusedMemoryId, setFocusedMemoryId] = useState<string | null>(null);
  const [operationError, setOperationError] = useState(false);
  const rowRefs = useRef(new Map<string, HTMLTableRowElement>());

  const filteredMemories = memories.filter((m) => {
    if (stateFilter !== "all" && m.state !== stateFilter) return false;
    if (scopeFilter !== "all" && m.scope_kind !== scopeFilter) return false;
    return true;
  });
  const rovingMemoryId =
    filteredMemories.find((memory) => memory.memory_id === focusedMemoryId)?.memory_id ??
    filteredMemories.find((memory) => memory.memory_id === selectedMemoryId)?.memory_id ??
    filteredMemories[0]?.memory_id;

  const runMemoryOperation = async (operation: () => Promise<unknown>) => {
    setOperationError(false);
    try {
      await operation();
      return true;
    } catch {
      setOperationError(true);
      return false;
    }
  };

  const handlePropose = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newContent.trim() || !onProposeMemory) return;
    setIsSubmitting(true);
    try {
      const succeeded = await runMemoryOperation(() =>
        onProposeMemory({
          content: newContent.trim(),
          scope_kind: newScopeKind,
          scope_target: newScopeKind === "global" ? null : (newScopeTarget.trim() || null),
          kind: newKind,
          source: newIsExplicit ? "explicit" : "inferred",
        }),
      );
      if (succeeded) {
        setNewContent("");
        setNewScopeTarget("");
        setShowAddForm(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  };

  const focusMemoryRow = (index: number) => {
    const target = filteredMemories[index];
    if (!target) return;
    setFocusedMemoryId(target.memory_id);
    rowRefs.current.get(target.memory_id)?.focus();
  };

  const handleStartCorrect = (mem: MemoryRecordDto) => {
    setEditingMemoryId(mem.memory_id);
    setEditContent(mem.content);
  };

  const handleSaveCorrect = async (memoryId: string) => {
    if (!editContent.trim() || !onCorrectMemory) return;
    const succeeded = await runMemoryOperation(() =>
      onCorrectMemory({
        memory_id: memoryId,
        content: editContent.trim(),
      }),
    );
    if (succeeded) {
      setEditingMemoryId(null);
      setEditContent("");
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
            onChange={(e) => {
              const nextFilter = e.target.value;
              setStateFilter(nextFilter);
              if (
                selectedMemoryId &&
                !memories.some(
                  (memory) =>
                    memory.memory_id === selectedMemoryId &&
                    (nextFilter === "all" || memory.state === nextFilter) &&
                    (scopeFilter === "all" || memory.scope_kind === scopeFilter),
                )
              ) {
                onClearMemorySelection?.();
              }
            }}
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
            onChange={(e) => {
              const nextFilter = e.target.value;
              setScopeFilter(nextFilter);
              if (
                selectedMemoryId &&
                !memories.some(
                  (memory) =>
                    memory.memory_id === selectedMemoryId &&
                    (stateFilter === "all" || memory.state === stateFilter) &&
                    (nextFilter === "all" || memory.scope_kind === nextFilter),
                )
              ) {
                onClearMemorySelection?.();
              }
            }}
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

      {operationError ? (
        <p className={styles.operationError} role="alert" data-testid="memory-operation-error">
          {t.memoryPane.operationFailed}
        </p>
      ) : null}

      {/* Agent Memory Mode Setting */}
      {activeAgent ? (
        <div className={styles.agentModeBar} data-testid="agent-memory-mode-bar">
          <span className={styles.agentModeLabel}>{t.memoryPane.agentMemoryMode(activeAgent.name)}</span>
          <select
            className={styles.select}
            value={activeAgent.memory_mode ?? "session"}
            onChange={(e) => {
              const nextMode = e.target.value as "off" | "session" | "long_term";
              // Surface transport/validation failures in the shared
              // operation-error banner; the select stays on the old value
              // because the mode only changes in the store on success.
              void runMemoryOperation(async () => {
                await onSetAgentMemoryMode?.(activeAgent.id, nextMode);
              });
            }}
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
      {/* Dense memory table (redesign — replaces card waterfall) */}
      <div className={styles.tableWrap} data-testid="memory-card-list">
        {filteredMemories.length === 0 ? (
          <p className={styles.emptyMessage} data-testid="memory-empty">
            {memories.length === 0
              ? t.memoryPane.empty
              : t.memoryPane.emptyFiltered}
          </p>
        ) : (
          <table className={styles.table} role="grid" aria-label={t.memoryPane.title}>
            <thead>
              <tr>
                <th scope="col">{t.memoryPane.colContent}</th>
                <th scope="col">{t.memoryPane.colKind}</th>
                <th scope="col">{t.memoryPane.colScope}</th>
                <th scope="col">{t.memoryPane.sourceLabel}</th>
                <th scope="col">{t.memoryPane.colConfidence}</th>
                <th scope="col">{t.memoryPane.colState}</th>
                <th scope="col">{t.memoryPane.colUpdated}</th>
                <th scope="col">{t.memoryPane.colActions}</th>
              </tr>
            </thead>
            <tbody>
              {filteredMemories.map((mem, index) => {
                const isEditing = editingMemoryId === mem.memory_id;
                return (
                  <tr
                    key={mem.memory_id}
                    ref={(node) => {
                      if (node) rowRefs.current.set(mem.memory_id, node);
                      else rowRefs.current.delete(mem.memory_id);
                    }}
                    className={`${styles.row}${selectedMemoryId === mem.memory_id ? ` ${styles.rowSelected}` : ""}`}
                    data-testid={`memory-card-${mem.memory_id}`}
                    data-state={mem.state}
                    aria-selected={selectedMemoryId === mem.memory_id}
                    tabIndex={rovingMemoryId === mem.memory_id ? 0 : -1}
                    onFocus={(event) => {
                      if (event.target === event.currentTarget) setFocusedMemoryId(mem.memory_id);
                    }}
                    onClick={() => {
                      setFocusedMemoryId(mem.memory_id);
                      onSelectMemory?.(mem);
                    }}
                    onKeyDown={(event) => {
                      if (event.target !== event.currentTarget) return;
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        onSelectMemory?.(mem);
                      } else if (event.key === "ArrowDown") {
                        event.preventDefault();
                        focusMemoryRow(Math.min(index + 1, filteredMemories.length - 1));
                      } else if (event.key === "ArrowUp") {
                        event.preventDefault();
                        focusMemoryRow(Math.max(index - 1, 0));
                      } else if (event.key === "Home") {
                        event.preventDefault();
                        focusMemoryRow(0);
                      } else if (event.key === "End") {
                        event.preventDefault();
                        focusMemoryRow(filteredMemories.length - 1);
                      }
                    }}
                  >
                    <td className={styles.cellContent}>
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
                        <div
                          className={styles.content}
                          data-testid="memory-card-content"
                        >
                          {mem.content}
                        </div>
                      )}
                    </td>
                    <td>
                        <span className={styles.kindText}>
                          {localizeMemoryKind(mem.kind, t)}
                        </span>
                    </td>
                    <td>
                      <span>
                        {localizeMemoryScope(mem.scope_kind, t)}
                        {mem.scope_target ? (
                          <span className={styles.mono}>
                            {" "}
                            ({mem.scope_target})
                          </span>
                        ) : null}
                      </span>
                    </td>
                    <td data-testid="memory-source-label">{localizeMemorySource(mem.source, t)}</td>
                    <td className={styles.cellNumeric}>
                      {mem.confidence}%
                    </td>
                    <td>
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
                    </td>
                    <td className={styles.cellMuted}>
                      {new Date(mem.updated_at).toLocaleString([], {
                        month: "2-digit",
                        day: "2-digit",
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </td>
                    <td className={styles.cellActions}>
                      {mem.state === "candidate" ? (
                        <>
                          <button
                            type="button"
                            className={`${styles.btnAction} ${styles.btnConfirm}`}
                            onClick={(event) => {
                              event.stopPropagation();
                              if (onConfirmMemory) {
                                void runMemoryOperation(() => onConfirmMemory(mem.memory_id));
                              }
                            }}
                            data-testid="memory-confirm-btn"
                          >
                            {t.memoryPane.confirm}
                          </button>
                          <button
                            type="button"
                            className={`${styles.btnAction} ${styles.btnReject}`}
                            onClick={(event) => {
                              event.stopPropagation();
                              if (onRejectMemory) {
                                void runMemoryOperation(() => onRejectMemory(mem.memory_id));
                              }
                            }}
                            data-testid="memory-reject-btn"
                          >
                            {t.memoryPane.reject}
                          </button>
                        </>
                      ) : null}
                      {mem.state === "confirmed" && !isEditing ? (
                        <>
                          <button
                            type="button"
                            className={styles.btnAction}
                            onClick={(event) => {
                              event.stopPropagation();
                              handleStartCorrect(mem);
                            }}
                            data-testid="memory-correct-btn"
                          >
                            {t.memoryPane.correct}
                          </button>
                          <button
                            type="button"
                            className={styles.btnAction}
                            onClick={(event) => {
                              event.stopPropagation();
                              if (onForgetMemory) {
                                void runMemoryOperation(() => onForgetMemory(mem.memory_id));
                              }
                            }}
                            data-testid="memory-forget-btn"
                          >
                            {t.memoryPane.forget}
                          </button>
                        </>
                      ) : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}
