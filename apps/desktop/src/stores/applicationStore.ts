/**
 * Transport-driven Application Store.
 *
 * Coordinates CoreTransport with Desktop state:
 * - All business commands strictly aligned to protocol DTOs.
 * - Agent onboarding, listing, and testing (with opaque secret references).
 * - Thread list, create, search, open, and history driven by Core snapshots.
 *   The Core list/search responses are the authority: the visible thread
 *   list, the entity cache, the search results, and the selection are kept
 *   separately so an empty or in-flight result never unmounts the
 *   conversation being read (review A03, findings F03/F06).
 * - Prompt dispatch & per-row delta streaming driven by Core events.
 * - Permission approve / deny actions with event settlement and rollback on error.
 * - Turn cancellation via cancel_turn.
 * - Reconnection with sequence cursor and stream.replayed/stream.ready controls.
 * - Idempotent event deduplication by event_id / sequence.
 */
import {
  createTimelineStore,
  type PermissionDecision,
  type TimelineRow,
  type TimelineStore,
} from "../features/timeline/timelineStore";
import { reduceHistoryEntries } from "../features/timeline/historyReducer";
import {
  BoundedIdSet,
  BoundedTimelineCache,
  EpochSequenceTracker,
} from "./streamDeduplicator";
import type { CancelTurnCommand } from "../ipc/dto/CancelTurnCommand";
import type { CommandEnvelope } from "../ipc/dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "../ipc/dto/ConfigureAgentCommand";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import type { CreateThreadCommand } from "../ipc/dto/CreateThreadCommand";
import type { DeleteIdentityDocumentCommand } from "../ipc/dto/DeleteIdentityDocumentCommand";
import type { DiagnosticsCommand } from "../ipc/dto/DiagnosticsCommand";
import type { EventEnvelope } from "../ipc/dto/EventEnvelope";
import type { GetContextSnapshotCommand } from "../ipc/dto/GetContextSnapshotCommand";
import type { GetHistoryCommand } from "../ipc/dto/GetHistoryCommand";
import type { HarnessBindingConfigDto } from "../ipc/dto/HarnessBindingConfigDto";
import type { HistoryCursorDto } from "../ipc/dto/HistoryCursorDto";
import type { IdentityDocumentDto } from "../ipc/dto/IdentityDocumentDto";
import type { ListIdentityDocumentsCommand } from "../ipc/dto/ListIdentityDocumentsCommand";
import type { ListThreadsCommand } from "../ipc/dto/ListThreadsCommand";
import type { NegotiatedHandshake } from "../ipc/dto/NegotiatedHandshake";
import type { ListMemoriesCommand } from "../ipc/dto/ListMemoriesCommand";
import type { ProposeMemoryCommand } from "../ipc/dto/ProposeMemoryCommand";
import type { ConfirmMemoryCommand } from "../ipc/dto/ConfirmMemoryCommand";
import type { RejectMemoryCommand } from "../ipc/dto/RejectMemoryCommand";
import type { CorrectMemoryCommand } from "../ipc/dto/CorrectMemoryCommand";
import type { ForgetMemoryCommand } from "../ipc/dto/ForgetMemoryCommand";
import type { MemoryRecordDto } from "../ipc/dto/MemoryRecordDto";
import type { MemoryListResponseDto } from "../ipc/dto/MemoryListResponseDto";
import type { OpenThreadCommand } from "../ipc/dto/OpenThreadCommand";
import type { PutIdentityDocumentCommand } from "../ipc/dto/PutIdentityDocumentCommand";
import type { RespondPermissionCommand } from "../ipc/dto/RespondPermissionCommand";
import type { RuntimeDiagnosticsDto } from "../ipc/dto/RuntimeDiagnosticsDto";
import type { RuntimeStatusCommand } from "../ipc/dto/RuntimeStatusCommand";
import type { SearchThreadsCommand } from "../ipc/dto/SearchThreadsCommand";
import type { Sequence } from "../ipc/dto/Sequence";
import type { SnapshotEnvelope } from "../ipc/dto/SnapshotEnvelope";
import type { StartTurnCommand } from "../ipc/dto/StartTurnCommand";
import type { TestHarnessBindingCommand } from "../ipc/dto/TestHarnessBindingCommand";
import type { TestHarnessResponseDto } from "../ipc/dto/TestHarnessResponseDto";
import type { CapabilitySet } from "../ipc/dto/CapabilitySet";
import type { ThreadCursorDto } from "../ipc/dto/ThreadCursorDto";
import type { ThreadDto } from "../ipc/dto/ThreadDto";
import type { ThreadHistoryResponseDto } from "../ipc/dto/ThreadHistoryResponseDto";
import type { ThreadListResponseDto } from "../ipc/dto/ThreadListResponseDto";
import type { ThreadSnapshotDto } from "../ipc/dto/ThreadSnapshotDto";
import type { ThreadSummaryDto } from "../ipc/dto/ThreadSummaryDto";
import { createOperationId } from "../ipc/operationId";
import { ConnectionClosedError, CoreTransportError } from "../ipc/errors";
import type { CoreTransport, TransportStatus } from "../ipc/transport";

export interface AgentProfile {
  readonly id: string;
  readonly name: string;
  readonly provider: string;
  readonly model?: string;
  /** OPAQUE REFERENCE ONLY (e.g. vault://key-1, env:OPENAI_API_KEY). Plaintext is never stored. */
  readonly secretRef?: string;
  readonly secretRefs?: readonly string[];
  readonly program?: string;
  readonly args?: readonly string[];
  readonly envKeys?: readonly string[];
  readonly label?: string;
  readonly bindingId?: string;
  readonly status: "ready" | "testing" | "error";
  readonly latencyMs?: number;
  readonly memory_mode?: "off" | "session" | "long_term";
  readonly capabilities?: CapabilitySet;
}

export interface EnvSecretMapping {
  readonly envKey: string;
  readonly secretRef: string;
}

export interface OnboardAgentParams {
  readonly name: string;
  readonly provider?: string;
  readonly model?: string;
  readonly program?: string;
  readonly args?: readonly string[] | string;
  readonly envKeys?: readonly string[] | string;
  readonly secretRef?: string;
  readonly secretRefs?: readonly string[] | string;
  readonly envMappings?: readonly EnvSecretMapping[];
  readonly label?: string;
  readonly bindingId?: string;
  readonly memory_mode?: "off" | "session" | "long_term";
  readonly capabilities?: CapabilitySet;
}

export interface TestAgentParams {
  readonly provider?: string;
  readonly model?: string;
  readonly program?: string;
  readonly args?: readonly string[] | string;
  readonly envKeys?: readonly string[] | string;
  readonly secretRef?: string;
  readonly secretRefs?: readonly string[] | string;
  readonly envMappings?: readonly EnvSecretMapping[];
  readonly label?: string;
  readonly bindingId?: string;
}

export interface TestAgentResult {
  readonly success: boolean;
  readonly latencyMs?: number;
  readonly error?: string;
  readonly capabilities?: CapabilitySet;
  readonly bindingId?: string;
}

export type StreamStatus = "idle" | "live" | "replaying" | "ready";

export type ThreadStatus = "running" | "waiting-for-permission" | "failed" | "completed";

/**
 * Production view model for one conversation, projected from Core DTOs.
 * This is what the shell renders; the fixture `ThreadFixture` shape never
 * enters application state (review A03).
 */
export interface ThreadSummaryView {
  readonly id: string;
  readonly title: string;
  readonly agent: string;
  readonly status: ThreadStatus;
  readonly pinned: boolean;
}

/** Delivery verdict for one dispatched prompt (review A04). */
export type PromptDispatchResult =
  | { status: "admitted"; threadId: string; turnId: string | null }
  | { status: "rejected"; threadId: string; reason: string }
  | { status: "indeterminate"; threadId: string; reason: string };

/** Per-turn cancellation flight status (A04): Core settles cancellations. */
export type CancelState = "requested" | "failed";

export interface ActiveTurnState {
  /** Core-minted turn identity; null until the start_turn response resolves it. */
  readonly turnId: string | null;
  readonly threadId: string;
  readonly userRowId: string;
  readonly replyRowId: string;
  readonly isStreaming: boolean;
  /** Set when a cancel_turn is in flight or has failed; cleared on the authoritative turn.cancelled. */
  readonly cancelState?: CancelState | null;
  /** Human-readable notice tied to this turn (e.g. indeterminate delivery). */
  readonly notice?: string | null;
}

export interface ApplicationState {
  readonly connectionStatus: TransportStatus;
  readonly negotiated: NegotiatedHandshake | null;
  readonly streamState: StreamStatus;
  /** Active Core launch/daemon instance identifier (ADR 0021, A06). */
  readonly coreInstanceId: string | null;
  readonly lastSequence: number;
  readonly error: string | null;

  readonly agents: readonly AgentProfile[];
  readonly selectedAgentId: string;
  readonly isOnboardingOpen: boolean;
  readonly onboardingNotice: string | null;
  readonly onboardingStatus: {
    readonly isTesting: boolean;
    readonly testResult: TestAgentResult | null;
    readonly bindingId?: string;
  };

  /** Visible thread list: search results while searching, otherwise the authoritative list page. */
  readonly threads: readonly ThreadSummaryView[];
  /** True when the authoritative list has more pages behind `loadMoreThreads`. */
  readonly hasMoreThreads: boolean;
  /** True while a search query is active (`threads` holds search results). */
  readonly searchActive: boolean;
  readonly selectedThreadId: string;
  /** The conversation being read, independent of the visible list. */
  readonly selectedThread: ThreadSummaryView | null;
  readonly threadFilter: string;

  /**
   * Active turns keyed by conversation (A04): each entry carries its own
   * thread/turn identity so background conversations never cross wires.
   */
  readonly activeTurns: readonly ActiveTurnState[];
  readonly runtimeDiagnostics: RuntimeDiagnosticsDto | null;
  readonly contextSnapshot: ContextSnapshotDto | null;
  readonly identityDocuments: readonly IdentityDocumentDto[];
  readonly contextSnapshotStatus: "idle" | "loading" | "loaded" | "not_found" | "error";
  readonly contextSnapshotError: string | null;
  readonly memories: readonly MemoryRecordDto[];
  readonly isLoadingMemories: boolean;
  readonly memoryFilter: {
    readonly scope_kind?: string | null;
    readonly scope_target?: string | null;
    readonly state?: string | null;
  };
  readonly streamLog: readonly {
    readonly sequence: number;
    readonly label: string;
    readonly diagnostic: string | null;
    readonly event_id: string;
  }[];
}

export interface ApplicationStore {
  subscribe(listener: () => void): () => void;
  getState(): ApplicationState;
  getTimelineStore(threadId: string): TimelineStore;

  // Lifecycle & Connection
  init(): Promise<void>;
  reconnect(): Promise<void>;
  disconnect(): void;
  /** Effect cleanup: drop the transport listener; Core stays untouched. */
  release(): void;

  // Agent Operations
  selectAgent(agentId: string): void;
  openOnboarding(open: boolean, notice?: string | null): void;
  onboardAgent(params: OnboardAgentParams): Promise<AgentProfile>;
  testAgent(params: TestAgentParams): Promise<TestAgentResult>;

  // Thread Operations
  selectThread(threadId: string): Promise<void>;
  setThreadFilter(filter: string): Promise<void>;
  createThread(title: string, agentProfileId?: string): Promise<ThreadSummaryView>;
  /** Fetches the next authoritative list page using the stored cursor. */
  loadMoreThreads(): Promise<void>;
  openThread(threadId: string): Promise<void>;
  getHistory(threadId: string, limit?: number, beforeSeq?: number | null): Promise<void>;
  /** Fetches older history entries using the thread's nextSeqCursor and prepends to timeline. */
  loadOlderHistory(threadId?: string, limit?: number): Promise<void>;
  getHistoryCursor(threadId: string): { nextSeqCursor: HistoryCursorDto | null; hasMore: boolean; highWaterSeq: number | null } | undefined;
  getDiagnostics(threadId?: string | null): Promise<RuntimeDiagnosticsDto | null>;

  // Context & Identity Operations (P2.2)
  getContextSnapshot(threadId?: string | null, turnId?: string | null): Promise<ContextSnapshotDto | null>;
  listIdentityDocuments(limit?: number): Promise<readonly IdentityDocumentDto[]>;
  putIdentityDocument(params: { document_id?: string | null; kind: string; content: string }): Promise<IdentityDocumentDto>;
  deleteIdentityDocument(documentId: string): Promise<void>;

  // Memory Operations (P2.1 / P2.2)
  listMemories(filter?: {
    scope_kind?: string | null;
    scope_target?: string | null;
    state?: string | null;
    limit?: number | null;
  }): Promise<readonly MemoryRecordDto[]>;
  proposeMemory(params: {
    content: string;
    scope_kind: string;
    scope_target?: string | null;
    kind: string;
    sensitivity?: string | null;
    confidence?: number | null;
    source?: string | null;
    thread_id?: string | null;
    turn_id?: string | null;
    excerpt?: string | null;
  }): Promise<MemoryRecordDto>;
  confirmMemory(memoryId: string): Promise<MemoryRecordDto>;
  rejectMemory(memoryId: string, reason?: string | null): Promise<MemoryRecordDto>;
  correctMemory(params: {
    memory_id: string;
    content: string;
    scope_kind?: string | null;
    scope_target?: string | null;
    kind?: string | null;
    sensitivity?: string | null;
  }): Promise<MemoryRecordDto>;
  forgetMemory(memoryId: string): Promise<MemoryRecordDto>;
  setAgentMemoryMode(agentId: string, memoryMode: "off" | "session" | "long_term"): Promise<void>;

  // Prompt & Turn Execution
  sendPrompt(text: string, threadId?: string): Promise<PromptDispatchResult>;
  cancelActiveTurn(threadId?: string): Promise<void>;

  // Permissions
  decidePermission(rowId: string, decision: PermissionDecision): Promise<void>;

  // A06 Diagnostics & Test helpers
  getEpoch(): string | null;
  getDeduplicationStats(): { seenIds: number; windowSeqs: number; highWater: number };
  _handleEvent(event: EventEnvelope): void;
}

const MAX_STREAM_LOG = 50;
const THREAD_PAGE_LIMIT = 50;

export function parseStringList(input?: readonly string[] | string): string[] {
  if (!input) return [];
  if (Array.isArray(input)) return [...input];
  if (typeof input === "string") {
    return input
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }
  return [];
}

/**
 * Parses a command-line arguments string while preserving quoted tokens (A07, review F11).
 * Supports both single and double quotes around paths and flags with spaces.
 */
export function parseCommandLineArgs(input?: readonly string[] | string): string[] {
  if (!input) return [];
  if (Array.isArray(input)) return [...input];
  if (typeof input === "string") {
    const trimmed = input.trim();
    if (!trimmed) return [];
    const args: string[] = [];
    const regex = /[^\s"']+|"([^"]*)"|'([^']*)'/g;
    let match: RegExpExecArray | null;
    while ((match = regex.exec(trimmed)) !== null) {
      if (match[1] !== undefined) {
        args.push(match[1]);
      } else if (match[2] !== undefined) {
        args.push(match[2]);
      } else {
        args.push(match[0]);
      }
    }
    return args;
  }
  return [];
}

/**
 * Resolves and validates environment variable keys and opaque secret references.
 *
 * Enforces:
 * 1. 1-to-1 parity between environment keys and credential references.
 * 2. All credential references must be validated by `sanitizeSecretRef` (rejects plaintext secrets).
 * 3. Disallows duplicate environment variable keys.
 * 4. Ensures no half-configured mappings (key without ref or ref without key).
 */
export function resolveEnvSecretPairs(params: {
  readonly envMappings?: readonly EnvSecretMapping[];
  readonly envKeys?: readonly string[] | string;
  readonly secretRef?: string;
  readonly secretRefs?: readonly string[] | string;
}): { envKeys: string[]; secretRefs: string[] } {
  const resultEnvKeys: string[] = [];
  const resultSecretRefs: string[] = [];
  const seenKeys = new Set<string>();

  if (params.envMappings && params.envMappings.length > 0) {
    for (const mapping of params.envMappings) {
      const key = mapping.envKey.trim();
      const rawRef = mapping.secretRef.trim();
      if (!key && !rawRef) continue;
      if (!key) {
        throw new Error("Each environment variable mapping must provide a variable key");
      }
      if (!rawRef) {
        throw new Error(`Environment variable '${key}' requires an opaque credential reference`);
      }
      if (seenKeys.has(key)) {
        throw new Error(`Duplicate environment variable key: ${key}`);
      }
      const sanitized = sanitizeSecretRef(rawRef);
      if (!sanitized) {
        throw new Error(`Environment variable '${key}' requires a non-empty credential reference`);
      }
      seenKeys.add(key);
      resultEnvKeys.push(key);
      resultSecretRefs.push(sanitized);
    }
    return { envKeys: resultEnvKeys, secretRefs: resultSecretRefs };
  }

  // Legacy / string-list input
  const parsedKeys = parseStringList(params.envKeys);
  let parsedRefs: string[] = [];
  if (params.secretRefs) {
    parsedRefs = parseStringList(params.secretRefs);
  } else if (params.secretRef) {
    const trimmed = params.secretRef.trim();
    if (trimmed) {
      parsedRefs = [trimmed];
    }
  }

  if (parsedKeys.length !== parsedRefs.length) {
    throw new Error("Each environment variable key requires exactly one credential reference");
  }

  for (let i = 0; i < parsedKeys.length; i++) {
    const key = parsedKeys[i]!.trim();
    if (seenKeys.has(key)) {
      throw new Error(`Duplicate environment variable key: ${key}`);
    }
    const rawRef = parsedRefs[i]!;
    const sanitized = sanitizeSecretRef(rawRef);
    if (!sanitized) {
      throw new Error(`Environment variable '${key}' requires a non-empty credential reference`);
    }
    seenKeys.add(key);
    resultEnvKeys.push(key);
    resultSecretRefs.push(sanitized);
  }

  return { envKeys: resultEnvKeys, secretRefs: resultSecretRefs };
}

/**
 * Validates opaque secret reference tokens (ADR 0006, review A07, F27).
 *
 * Product invariants:
 * - Credentials and keys live in the OS secret store. The UI holds ONLY an opaque ref.
 * - Valid formats: `sec_<id>`, `vault://<id>`, `env:<VAR>`, `ref:<id>`, `secret://<id>`.
 * - Plaintext API keys or arbitrary random strings must be rejected with an explicit
 *   error — never silently invent or randomly generate a non-existent ref.
 */
export function sanitizeSecretRef(input?: string): string | undefined {
  if (!input) return undefined;
  const trimmed = input.trim();
  if (!trimmed) return undefined;

  if (
    trimmed.startsWith("sec_") ||
    trimmed.startsWith("vault://") ||
    trimmed.startsWith("env:") ||
    trimmed.startsWith("ref:") ||
    trimmed.startsWith("secret://")
  ) {
    return trimmed;
  }

  throw new Error(
    `Invalid credential reference: credentials must be stored in the OS secret store and referenced by an opaque key (e.g. sec_..., vault://..., or env:...). Plaintext credentials are never accepted.`,
  );
}

function isSnapshotEnvelope(data: unknown): data is SnapshotEnvelope {
  return (
    typeof data === "object" &&
    data !== null &&
    "protocol_version" in data &&
    "operation_id" in data &&
    "data" in data
  );
}

export function createApplicationStore(
  transport: CoreTransport,
  options: {
    /**
     * Fixture-world only: seeds per-thread timeline rows so the synthetic
     * shell keeps its curated P0.4 content until A05 delivers the real
     * history projection. Never enters application state; the production
     * entry never passes it (review A03).
     */
    readonly fixtureTimelineRows?: readonly {
      readonly id: string;
      readonly rows: readonly TimelineRow[];
    }[];
  } = {},
): ApplicationStore {
  const listeners = new Set<() => void>();
  // Bounded deduplication & timeline cache (ADR 0021, A06)
  const seenEventIds = new BoundedIdSet(4096);
  const sequenceTracker = new EpochSequenceTracker(2048);
  const timelineStores = new BoundedTimelineCache(32);

  const isThreadPinned = (threadId: string): boolean => {
    if (threadId === state.selectedThreadId) return true;
    if (state.activeTurns.some((t) => t.threadId === threadId)) return true;
    return false;
  };

  for (const seed of options.fixtureTimelineRows ?? []) {
    timelineStores.set(seed.id, createTimelineStore(seed.rows), () => true);
  }

  // ── Thread state, separated per review A03 ──────────────────────────
  // `threadViews` is the entity cache (entityById); `listThreadIds` is the
  // authoritative list order from the last list page; `searchResultIds` is
  // the active search result set; `selectedThreadId` is the conversation
  // being read. An empty search result touches only the visible list.
  const threadViews = new Map<string, ThreadSummaryView>();
  const listThreadIds: string[] = [];
  let searchResultIds: readonly string[] | null = null;
  let threadsCursor: ThreadCursorDto | null = null;
  let hasMoreThreads = false;
  let searchGeneration = 0;

  const agentNameFor = (agentProfileId: string): string =>
    // Agent profiles enter state only from Core responses; until one
    // arrives the raw profile id is shown instead of an invented name.
    state.agents.find((a) => a.id === agentProfileId)?.name ?? agentProfileId;

  const summaryToView = (summary: ThreadSummaryDto): ThreadSummaryView => {
    const status: ThreadStatus =
      summary.active_turn?.state === "active" ? "running" : "completed";
    return {
      id: summary.thread.id,
      title: summary.thread.title,
      agent: agentNameFor(summary.thread.agent_profile_id),
      status,
      pinned: summary.thread.state === "pinned",
    };
  };

  const upsertSummary = (summary: ThreadSummaryDto): ThreadSummaryView => {
    const view = summaryToView(summary);
    threadViews.set(view.id, view);
    return view;
  };

  const viewsForIds = (ids: readonly string[]): ThreadSummaryView[] =>
    ids
      .map((id) => threadViews.get(id))
      .filter((v): v is ThreadSummaryView => v !== undefined);

  let state: ApplicationState = {
    connectionStatus: transport.status(),
    negotiated: null,
    streamState: "idle",
    coreInstanceId: null,
    lastSequence: 0,
    error: null,

    agents: [],
    selectedAgentId: "",
    isOnboardingOpen: false,
    onboardingNotice: null,
    onboardingStatus: {
      isTesting: false,
      testResult: null,
    },

    threads: [],
    hasMoreThreads: false,
    searchActive: false,
    selectedThreadId: "",
    selectedThread: null,
    threadFilter: "",

    activeTurns: [],
    runtimeDiagnostics: null,
    contextSnapshot: null,
    identityDocuments: [],
    contextSnapshotStatus: "idle",
    contextSnapshotError: null,
    memories: [],
    isLoadingMemories: false,
    memoryFilter: {},
    streamLog: [],
  };

  let rowCounter = 0;
  let unsubscribeTransport: (() => void) | null = null;
  let bootstrapPromise: Promise<void> | null = null;
  let isRecovering = false;
  let recoveryQueued = false;

  const notify = (): void => {
    for (const listener of listeners) {
      listener();
    }
  };

  const updateState = (
    updater: (prev: ApplicationState) => ApplicationState,
  ): void => {
    state = updater(state);
    notify();
  };

  const getTimelineStore = (threadId: string): TimelineStore => {
    return timelineStores.getOrCreate(
      threadId,
      () => createTimelineStore([]),
      isThreadPinned,
    );
  };

  const applyThreadSnapshot = (
    snapshot: ThreadSnapshotDto,
    threadId: string,
  ): void => {
    const threadStore = getTimelineStore(threadId);

    // Snapshot `turns` carry turn-level metadata only — rendering them as
    // message rows is exactly review F07 ("Turn trn_…" placeholders). Real
    // content arrives via get_history journal entries (ADR 0020), which
    // openThread always requests after this call.

    if (snapshot.pending_permissions && snapshot.pending_permissions.length > 0) {
      for (const perm of snapshot.pending_permissions) {
        if (!threadStore.getRow(perm.event_id)) {
          threadStore.appendRow({
            id: perm.event_id,
            kind: "permission",
            text: perm.description,
            status: null,
            permission: {
              requestedAction: perm.description,
              scope: perm.kind,
              decision:
                perm.decision === "approved" || perm.decision === "denied"
                  ? perm.decision
                  : null,
            },
            streaming: false,
          });
        }
      }
    }

    if (snapshot.agent_profile) {
      const p = snapshot.agent_profile;
      const existing = state.agents.find((a) => a.id === p.id);
      if (!existing) {
        const newAgent: AgentProfile = {
          id: p.id,
          name: p.display_name,
          provider: p.preferred_harness,
          model: p.preferred_harness,
          status: "ready",
        };
        updateState((prev) => ({
          ...prev,
          agents: [...prev.agents, newAgent],
          selectedAgentId: prev.selectedAgentId || newAgent.id,
        }));
      }
    }

    // Upsert the conversation view from the snapshot so the selection and
    // the visible list reflect the authoritative thread record (title,
    // agent display name once the profile is known).
    const snapshotThread = snapshot.thread;
    const refreshed: ThreadSummaryView = {
      id: snapshotThread.id,
      title: snapshotThread.title,
      agent: agentNameFor(snapshotThread.agent_profile_id),
      status: threadViews.get(snapshotThread.id)?.status ?? "completed",
      pinned: snapshotThread.state === "pinned",
    };
    threadViews.set(refreshed.id, refreshed);
    updateState((prev) => ({
      ...prev,
      selectedThread:
        prev.selectedThreadId === refreshed.id ? refreshed : prev.selectedThread,
      // Known views may now resolve agent display names; refresh whatever
      // list is currently visible without changing its membership.
      threads: (prev.searchActive && searchResultIds ? searchResultIds : listThreadIds)
        .map((id) => threadViews.get(id))
        .filter((v): v is ThreadSummaryView => v !== undefined),
    }));
  };

  const handleIncomingEvent = (event: EventEnvelope): void => {
    const body = event.body;

    // Check if event signals Core restart / greeting (ADR 0021, A06)
    let incomingEpoch: string | null = null;
    if (body.kind === "core.greeting" || body.kind === "core.restarted") {
      const gBody = body as { instance_id?: string };
      incomingEpoch = gBody.instance_id ?? null;
      if (incomingEpoch && incomingEpoch !== sequenceTracker.epoch) {
        sequenceTracker.transitionEpoch(incomingEpoch);
        updateState((prev) => ({
          ...prev,
          coreInstanceId: incomingEpoch,
          lastSequence: 0,
        }));
        void triggerSnapshotRecovery();
      }
    }

    // 1. Idempotency Check: deduplicate by event_id and sequence within epoch
    if (seenEventIds.has(event.event_id) || sequenceTracker.isDuplicate(event.sequence, incomingEpoch)) {
      return;
    }
    seenEventIds.add(event.event_id);
    sequenceTracker.record(event.sequence, incomingEpoch);

    const diagnostic = "diagnostic" in body ? body.diagnostic : null;

    // 2. Update stream log and sequence tracking (strictly sanitizing / redacting)
    updateState((prev) => {
      const nextLog = [
        ...prev.streamLog,
        {
          sequence: event.sequence,
          label: body.kind,
          diagnostic: typeof diagnostic === "string" ? diagnostic : null,
          event_id: event.event_id,
        },
      ];
      const boundedLog =
        nextLog.length > MAX_STREAM_LOG ? nextLog.slice(-MAX_STREAM_LOG) : nextLog;

      let nextStreamState = prev.streamState;
      if (body.kind === "stream.replayed") {
        nextStreamState = "replaying";
      } else if (body.kind === "stream.ready") {
        nextStreamState = "ready";
      } else if (nextStreamState === "replaying" || nextStreamState === "ready") {
        nextStreamState = "live";
      }

      return {
        ...prev,
        lastSequence: Math.max(prev.lastSequence, event.sequence),
        streamState: nextStreamState,
        streamLog: boundedLog,
      };
    });

    // 3. Route specific event kinds to thread timelines.
    // Envelope thread/turn identities are authoritative (A04); selected
    // thread only as a fallback for envelope events without a thread.
    const targetThreadId = event.thread_id ?? state.selectedThreadId;
    const threadStore = getTimelineStore(targetThreadId);
    const turnForEvent = (turnId: string | null | undefined): ActiveTurnState | undefined =>
      state.activeTurns.find(
        (t) =>
          t.threadId === targetThreadId &&
          (t.turnId == null || turnId == null || t.turnId === turnId),
      );
    const clearTurn = (turn: ActiveTurnState) => {
      updateState((prev) => ({
        ...prev,
        activeTurns: prev.activeTurns.filter((t) => t !== turn && t.replyRowId !== turn.replyRowId),
      }));
    };

    if (body.kind === "turn.started") {
      const turn = turnForEvent(event.turn_id);
      updateState((prev) => ({
        ...prev,
        activeTurns: prev.activeTurns.map((t) =>
          t === turn ? { ...t, isStreaming: true } : t,
        ),
      }));
    } else if (body.kind === "message.delta") {
      const deltaText = "text" in body ? body.text : "";
      const turn = turnForEvent(event.turn_id);
      if (deltaText && turn) {
        threadStore.appendDelta(turn.replyRowId, deltaText);
      }
    } else if (body.kind === "permission.requested") {
      const permBody = body as {
        kind: "permission.requested";
        permission_kind: string;
        description: string;
      };
      threadStore.appendRow({
        id: event.event_id,
        kind: "permission",
        text: permBody.description,
        status: null,
        permission: {
          requestedAction: permBody.description,
          scope: permBody.permission_kind,
          decision: null,
        },
        streaming: false,
      });
    } else if (body.kind === "permission.decided") {
      const decBody = body as { kind: "permission.decided"; decision: string };
      const candidateId = event.event_id;
      if (candidateId && threadStore.getRow(candidateId)?.permission) {
        threadStore.setPermissionDecision(
          candidateId,
          decBody.decision as PermissionDecision,
        );
      }
    } else if (body.kind === "turn.completed") {
      const turn = turnForEvent(event.turn_id);
      if (turn) {
        threadStore.finishStreaming(turn.replyRowId);
        clearTurn(turn);
      }
    } else if (body.kind === "turn.failed") {
      const failBody = body as { kind: "turn.failed"; reason: string };
      const turn = turnForEvent(event.turn_id);
      if (turn) {
        threadStore.finishStreaming(turn.replyRowId);
        threadStore.appendRow({
          id: `fail-${event.sequence}`,
          kind: "error",
          text: failBody.reason || "Turn failed",
          status: null,
          permission: null,
          streaming: false,
        });
        clearTurn(turn);
      }
    } else if (body.kind === "turn.cancelled") {
      const cancelBody = body as { kind: "turn.cancelled"; reason?: string };
      const turn = turnForEvent(event.turn_id);
      if (turn) {
        // The authoritative settlement: only here does the turn stop.
        threadStore.finishStreaming(turn.replyRowId);
        threadStore.appendRow({
          id: `cancel-${event.sequence}`,
          kind: "error",
          text: cancelBody.reason ?? diagnostic ?? "turn cancelled by user",
          status: null,
          permission: null,
          streaming: false,
        });
        clearTurn(turn);
      }
    } else if (body.kind === "stream.gap" || body.kind === "core.greeting" || body.kind === "core.restarted") {
      // Automatic full snapshot recovery on stream gap or Core restart (ADR 0008, P1.3)
      void triggerSnapshotRecovery();
    } else if (body.kind === "command.error") {
      const errBody = body as {
        kind: "command.error";
        operation_id: string;
        code: string;
        message: string;
      };
      updateState((prev) => ({
        ...prev,
        error: `[${errBody.code}] ${errBody.message}`,
      }));
    }
  };

  const triggerSnapshotRecovery = async (): Promise<void> => {
    if (isRecovering) {
      recoveryQueued = true;
      return;
    }
    isRecovering = true;
    try {
      await listThreadsFromCore();
      const currentSelected = state.selectedThreadId;
      if (currentSelected) {
        await openThread(currentSelected);
        await getHistory(currentSelected);
      }
    } catch {
      // Retain fallback state on recovery error, avoid retry loop
    } finally {
      isRecovering = false;
      if (recoveryQueued) {
        recoveryQueued = false;
        void triggerSnapshotRecovery();
      }
    }
  };

  /**
   * Replaces the authoritative list page with the Core response (A03):
   * the visible list never retains entries the authority no longer
   * reports, while the entity cache keeps opened threads resolvable.
   */
  const applyListPage = (
    summaries: readonly ThreadSummaryDto[],
    options: { append: boolean; hasMore: boolean; cursor: ThreadCursorDto | null },
  ): ThreadSummaryView[] => {
    for (const summary of summaries) {
      upsertSummary(summary);
    }
    const ids = summaries.map((s) => s.thread.id);
    if (options.append) {
      const known = new Set(listThreadIds);
      for (const id of ids) {
        if (!known.has(id)) {
          listThreadIds.push(id);
        }
      }
    } else {
      listThreadIds.length = 0;
      listThreadIds.push(...ids);
    }
    threadsCursor = options.cursor;
    hasMoreThreads = options.hasMore;
    return viewsForIds(listThreadIds);
  };

  const listThreadsFromCore = async (): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "list_threads",
      payload: { cursor: null, limit: THREAD_PAGE_LIMIT } as ListThreadsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const listData = res.data as ThreadListResponseDto;
        if (Array.isArray(listData.threads)) {
          const listViews = applyListPage(listData.threads, {
            append: false,
            hasMore: listData.has_more ?? false,
            cursor: listData.next_cursor ?? null,
          });
          updateState((prev) => {
            // Auto-select the first conversation only when nothing is
            // selected; an established selection stays stable.
            const selectedThreadId =
              prev.selectedThreadId ||
              listThreadIds[0] ||
              "";
            return {
              ...prev,
              threads: prev.searchActive && searchResultIds
                ? viewsForIds(searchResultIds)
                : listViews,
              hasMoreThreads,
              selectedThreadId,
              selectedThread: selectedThreadId
                ? threadViews.get(selectedThreadId) ?? prev.selectedThread
                : null,
            };
          });
        }
      }
    } catch (err) {
      // A failed list leaves the pane empty rather than decorative; the
      // reason is surfaced instead of silently keeping stale rows.
      updateState((prev) => ({
        ...prev,
        error: `Failed to load conversations: ${err instanceof Error ? err.message : String(err)}`,
      }));
    }
  };

  const loadMoreThreads = async (): Promise<void> => {
    if (!threadsCursor) return;
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "list_threads",
      payload: { cursor: threadsCursor, limit: THREAD_PAGE_LIMIT } as ListThreadsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const listData = res.data as ThreadListResponseDto;
        if (Array.isArray(listData.threads)) {
          const listViews = applyListPage(listData.threads, {
            append: true,
            hasMore: listData.has_more ?? false,
            cursor: listData.next_cursor ?? null,
          });
          updateState((prev) => ({
            ...prev,
            threads: prev.searchActive ? prev.threads : listViews,
            hasMoreThreads,
          }));
        }
      }
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        error: `Failed to load more conversations: ${err instanceof Error ? err.message : String(err)}`,
      }));
    }
  };

  const getRuntimeStatusFromCore = async (): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "runtime_status",
      payload: { include_diagnostics: true } as RuntimeStatusCommand,
      issued_at: Date.now(),
    };

    try {
      await transport.command(envelope);
    } catch {
      // Handled gracefully
    }
  };

  const openThread = async (threadId: string): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "open_thread",
      payload: { thread_id: threadId, history_limit: 100 } as OpenThreadCommand,
      issued_at: Date.now(),
    };

    try {
      // Journal order is authoritative (ADR 0020): load history entries
      // first, then layer the snapshot on top — its pending-permission rows
      // no-op when history already projected the same event, so rows keep
      // journal order instead of permissions jumping to the top.
      await getHistory(threadId);
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const snap = res.data as ThreadSnapshotDto;
        applyThreadSnapshot(snap, threadId);
      }
    } catch {
      // Retain existing timeline store
    }
  };

  const threadHistoryCursors = new Map<
    string,
    {
      nextSeqCursor: HistoryCursorDto | null;
      hasMore: boolean;
      highWaterSeq: number | null;
    }
  >();

  const getHistoryCursor = (
    threadId: string,
  ): {
    nextSeqCursor: HistoryCursorDto | null;
    hasMore: boolean;
    highWaterSeq: number | null;
  } | undefined => threadHistoryCursors.get(threadId);

  const getHistory = async (
    threadId: string,
    limit: number = 50,
    beforeSeq?: number | null,
  ): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "get_history",
      payload: {
        thread_id: threadId,
        cursor: null,
        limit,
        before_seq: beforeSeq ?? null,
      } as GetHistoryCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const hist = res.data as ThreadHistoryResponseDto;
        const threadStore = getTimelineStore(threadId);

        threadHistoryCursors.set(threadId, {
          nextSeqCursor: hist.next_seq_cursor ?? null,
          hasMore: hist.has_more ?? false,
          highWaterSeq: hist.high_water_seq ?? null,
        });

        if (hist.entries && hist.entries.length > 0) {
          const rows = reduceHistoryEntries(hist.entries);
          if (beforeSeq != null) {
            // Older history pagination (A05): prepend to keep past order stable
            threadStore.prependRows(rows);
          } else {
            for (const row of rows) {
              if (!threadStore.getRow(row.id)) {
                threadStore.appendRow(row);
              }
            }
          }
        } else {
          // Backward compatibility fallback for turn summaries
          for (const turn of hist.turns) {
            if (!threadStore.getRow(turn.id)) {
              threadStore.appendRow({
                id: turn.id,
                kind: "assistant-message",
                text: `Turn ${turn.id}`,
                status: null,
                permission: null,
                streaming: turn.state === "active",
              });
            }
          }
        }
      }
    } catch {
      // Retain existing timeline store
    }
  };

  const loadOlderHistory = async (
    threadId?: string,
    limit: number = 50,
  ): Promise<void> => {
    const targetThreadId = threadId ?? state.selectedThreadId;
    if (!targetThreadId) return;
    const cursorInfo = threadHistoryCursors.get(targetThreadId);
    if (!cursorInfo || !cursorInfo.hasMore || !cursorInfo.nextSeqCursor) {
      return;
    }
    await getHistory(targetThreadId, limit, cursorInfo.nextSeqCursor.seq);
  };

  const getDiagnostics = async (
    threadId?: string | null,
  ): Promise<RuntimeDiagnosticsDto | null> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "diagnostics",
      payload: {
        thread_id: threadId ?? null,
        limit: 50,
      } as DiagnosticsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const diag = res.data as RuntimeDiagnosticsDto;
        updateState((prev) => ({ ...prev, runtimeDiagnostics: diag }));
        return diag;
      }
    } catch {
      // Diagnostics query failure
    }
    return null;
  };

  let contextSnapshotRequestId = 0;

  const getContextSnapshot = async (
    threadId?: string | null,
    turnId?: string | null,
  ): Promise<ContextSnapshotDto | null> => {
    const requestId = ++contextSnapshotRequestId;
    const targetTurn = turnId || null;
    const targetThread = (!targetTurn && (threadId || state.selectedThreadId)) || null;

    updateState((prev) => ({
      ...prev,
      contextSnapshotStatus: "loading",
      contextSnapshotError: null,
    }));

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "get_context_snapshot",
      payload: {
        turn_id: targetTurn,
        thread_id: targetTurn ? null : targetThread,
      } as GetContextSnapshotCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope | ContextSnapshotDto | ContextSnapshotDto[] | null>(envelope);
      // Eliminate race conditions: ignore stale responses from earlier turn selections
      if (requestId !== contextSnapshotRequestId) {
        return null;
      }
      let snapshot: ContextSnapshotDto | null = null;
      if (isSnapshotEnvelope(res)) {
        if (Array.isArray(res.data)) {
          snapshot = (res.data[0] as ContextSnapshotDto) ?? null;
        } else {
          snapshot = (res.data as ContextSnapshotDto) ?? null;
        }
      } else if (Array.isArray(res)) {
        snapshot = (res[0] as ContextSnapshotDto) ?? null;
      } else if (res && typeof res === "object" && ("budget" in res || "turn_id" in res)) {
        snapshot = res as ContextSnapshotDto;
      }

      updateState((prev) => ({
        ...prev,
        contextSnapshot: snapshot,
        contextSnapshotStatus: snapshot ? "loaded" : "not_found",
        contextSnapshotError: null,
      }));
      return snapshot;
    } catch (err) {
      if (requestId !== contextSnapshotRequestId) {
        return null;
      }
      const errMsg = err instanceof Error ? err.message : String(err);
      updateState((prev) => ({
        ...prev,
        contextSnapshot: null,
        contextSnapshotStatus: "error",
        contextSnapshotError: errMsg,
        error: `Failed to get context snapshot: ${errMsg}`,
      }));
      return null;
    }
  };

  const listMemories = async (
    filter?: {
      scope_kind?: string | null;
      scope_target?: string | null;
      state?: string | null;
      limit?: number | null;
    },
  ): Promise<readonly MemoryRecordDto[]> => {
    updateState((prev) => ({
      ...prev,
      isLoadingMemories: true,
      memoryFilter: {
        scope_kind: filter?.scope_kind ?? null,
        scope_target: filter?.scope_target ?? null,
        state: filter?.state ?? null,
      },
    }));

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "list_memories",
      payload: {
        scope_kind: filter?.scope_kind ?? null,
        scope_target: filter?.scope_target ?? null,
        state: filter?.state ?? null,
        limit: filter?.limit ?? null,
      } as ListMemoriesCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope | MemoryListResponseDto | MemoryRecordDto[]>(envelope);
      let records: MemoryRecordDto[] = [];
      if (isSnapshotEnvelope(res)) {
        if (Array.isArray(res.data)) {
          records = res.data as MemoryRecordDto[];
        } else if (res.data && typeof res.data === "object" && "memories" in res.data) {
          records = (res.data as MemoryListResponseDto).memories;
        }
      } else if (Array.isArray(res)) {
        records = res;
      } else if (res && typeof res === "object" && "memories" in res) {
        records = (res as MemoryListResponseDto).memories;
      }
      updateState((prev) => ({
        ...prev,
        memories: records,
        isLoadingMemories: false,
      }));
      return records;
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        isLoadingMemories: false,
        error: `Failed to list memories: ${err instanceof Error ? err.message : String(err)}`,
      }));
      return [];
    }
  };

  const proposeMemory = async (
    params: {
      content: string;
      scope_kind: string;
      scope_target?: string | null;
      kind: string;
      sensitivity?: string | null;
      confidence?: number | null;
      source?: string | null;
      thread_id?: string | null;
      turn_id?: string | null;
      excerpt?: string | null;
    },
  ): Promise<MemoryRecordDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "propose_memory",
      payload: {
        content: params.content,
        scope_kind: params.scope_kind,
        scope_target: params.scope_target ?? null,
        kind: params.kind,
        sensitivity: params.sensitivity ?? null,
        confidence: params.confidence ?? null,
        source: params.source ?? null,
        thread_id: params.thread_id ?? null,
        turn_id: params.turn_id ?? null,
        excerpt: params.excerpt ?? null,
      } as ProposeMemoryCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | MemoryRecordDto>(envelope);
    const record = (isSnapshotEnvelope(res) ? res.data : res) as MemoryRecordDto;
    updateState((prev) => {
      const existingIdx = prev.memories.findIndex((m) => m.memory_id === record.memory_id);
      const updated = [...prev.memories];
      if (existingIdx >= 0) {
        updated[existingIdx] = record;
      } else {
        updated.unshift(record);
      }
      return {
        ...prev,
        memories: updated,
      };
    });
    return record;
  };

  const confirmMemory = async (memoryId: string): Promise<MemoryRecordDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "confirm_memory",
      payload: {
        memory_id: memoryId,
      } as ConfirmMemoryCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | MemoryRecordDto>(envelope);
    const record = (isSnapshotEnvelope(res) ? res.data : res) as MemoryRecordDto;
    updateState((prev) => ({
      ...prev,
      memories: prev.memories.map((m) => (m.memory_id === record.memory_id ? record : m)),
    }));
    return record;
  };

  const rejectMemory = async (memoryId: string, reason?: string | null): Promise<MemoryRecordDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "reject_memory",
      payload: {
        memory_id: memoryId,
        reason: reason ?? null,
      } as RejectMemoryCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | MemoryRecordDto>(envelope);
    const record = (isSnapshotEnvelope(res) ? res.data : res) as MemoryRecordDto;
    updateState((prev) => ({
      ...prev,
      memories: prev.memories.map((m) => (m.memory_id === record.memory_id ? record : m)),
    }));
    return record;
  };

  const correctMemory = async (
    params: {
      memory_id: string;
      content: string;
      scope_kind?: string | null;
      scope_target?: string | null;
      kind?: string | null;
      sensitivity?: string | null;
    },
  ): Promise<MemoryRecordDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "correct_memory",
      payload: {
        memory_id: params.memory_id,
        content: params.content,
        scope_kind: params.scope_kind ?? null,
        scope_target: params.scope_target ?? null,
        kind: params.kind ?? null,
        sensitivity: params.sensitivity ?? null,
      } as CorrectMemoryCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | MemoryRecordDto>(envelope);
    const record = (isSnapshotEnvelope(res) ? res.data : res) as MemoryRecordDto;
    updateState((prev) => {
      const updated = prev.memories.map((m) =>
        m.memory_id === params.memory_id
          ? { ...m, state: "superseded", superseded_by: record.memory_id }
          : m,
      );
      return {
        ...prev,
        memories: [record, ...updated],
      };
    });
    return record;
  };

  const forgetMemory = async (memoryId: string): Promise<MemoryRecordDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "forget_memory",
      payload: {
        memory_id: memoryId,
      } as ForgetMemoryCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | MemoryRecordDto>(envelope);
    const record = (isSnapshotEnvelope(res) ? res.data : res) as MemoryRecordDto;
    updateState((prev) => ({
      ...prev,
      memories: prev.memories.map((m) => (m.memory_id === record.memory_id ? record : m)),
    }));
    return record;
  };

  const setAgentMemoryMode = async (
    agentId: string,
    memoryMode: "off" | "session" | "long_term",
  ): Promise<void> => {
    const agent = state.agents.find((a) => a.id === agentId);
    if (!agent) throw new Error(`Agent not found: ${agentId}`);

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "configure_agent",
      payload: {
        agent_profile_id: agentId,
        display_name: agent.name,
        preferred_harness: agent.provider.toLowerCase().includes("terminal")
          ? "terminal"
          : agent.provider.toLowerCase().includes("native")
            ? "native"
            : "acp",
        memory_mode: memoryMode,
        binding: agent.program
          ? {
              harness_binding_id: agent.bindingId ?? null,
              agent_profile_id: agentId,
              program: agent.program,
              args: agent.args ? [...agent.args] : [],
              env_keys: agent.envKeys ? [...agent.envKeys] : [],
              secret_refs: agent.secretRef ? [agent.secretRef] : [],
              label: agent.label ?? null,
            }
          : null,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    await transport.command(envelope);
    updateState((prev) => ({
      ...prev,
      agents: prev.agents.map((a) =>
        a.id === agentId ? { ...a, memory_mode: memoryMode } : a,
      ),
    }));
  };

  const listIdentityDocuments = async (
    limit?: number,
  ): Promise<readonly IdentityDocumentDto[]> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "list_identity_documents",
      payload: { limit: limit ?? null } as ListIdentityDocumentsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope | IdentityDocumentDto[]>(envelope);
      const docs = isSnapshotEnvelope(res)
        ? ((res.data as IdentityDocumentDto[]) ?? [])
        : Array.isArray(res)
          ? res
          : [];
      updateState((prev) => ({
        ...prev,
        identityDocuments: docs,
      }));
      return docs;
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        error: `Failed to list identity documents: ${err instanceof Error ? err.message : String(err)}`,
      }));
      return [];
    }
  };

  const putIdentityDocument = async (
    params: { document_id?: string | null; kind: string; content: string },
  ): Promise<IdentityDocumentDto> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "put_identity_document",
      payload: {
        document_id: params.document_id ?? null,
        kind: params.kind,
        content: params.content,
      } as PutIdentityDocumentCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<SnapshotEnvelope | IdentityDocumentDto>(envelope);
    const doc = (isSnapshotEnvelope(res) ? res.data : res) as IdentityDocumentDto;
    updateState((prev) => {
      const existingIdx = prev.identityDocuments.findIndex(
        (d) => d.document_id === doc.document_id,
      );
      const updated = [...prev.identityDocuments];
      if (existingIdx >= 0) {
        updated[existingIdx] = doc;
      } else {
        updated.push(doc);
      }
      return {
        ...prev,
        identityDocuments: updated,
      };
    });
    return doc;
  };

  const deleteIdentityDocument = async (documentId: string): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "delete_identity_document",
      payload: {
        document_id: documentId,
      } as DeleteIdentityDocumentCommand,
      issued_at: Date.now(),
    };

    await transport.command(envelope);
    updateState((prev) => ({
      ...prev,
      identityDocuments: prev.identityDocuments.filter(
        (d) => d.document_id !== documentId,
      ),
    }));
  };

  /**
   * Attaches the store's event listener to the transport. Synchronous
   * registration failures (e.g. a Tauri bridge without invoke/listen) are
   * surfaced as connection state, never as an unhandled rejection (A02).
   */
  const attachTransportListener = (): void => {
    if (unsubscribeTransport) return;
    try {
      unsubscribeTransport = transport.subscribe(handleIncomingEvent);
    } catch (err) {
      unsubscribeTransport = null;
      updateState((prev) => ({
        ...prev,
        connectionStatus: transport.status(),
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  /**
   * Drops the store's transport listener without touching the Core
   * process or the negotiated connection: React effect cleanup uses this,
   * and a remount re-attaches through `init` without re-sending commands.
   */
  const release = (): void => {
    if (unsubscribeTransport) {
      unsubscribeTransport();
      unsubscribeTransport = null;
    }
  };

  const bootstrap = async (): Promise<void> => {
    updateState((prev) => ({
      ...prev,
      connectionStatus: "connecting",
      error: null,
    }));

    try {
      const handshake = await transport.connect();
      updateState((prev) => ({
        ...prev,
        connectionStatus: "connected",
        negotiated: handshake,
        streamState: "live",
      }));

      // Post-init Core data synchronization (ADR 0008, P1.3)
      await listThreadsFromCore();
      await getRuntimeStatusFromCore();

      if (state.selectedThreadId) {
        await openThread(state.selectedThreadId);
      }

      // A genuinely empty vault (no agents and no conversations) opens
      // onboarding; fixture worlds with conversations surface their agent
      // profiles through the thread snapshots instead.
      if (state.agents.length === 0 && state.threads.length === 0) {
        updateState((prev) => ({ ...prev, isOnboardingOpen: true }));
      }
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        connectionStatus: transport.status(),
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  /**
   * Idempotent lifecycle entry (A02): the listener is (re-)attached on
   * every call, but the connect + bootstrap command round runs at most
   * once per store instance, so React StrictMode's double effect never
   * duplicates commands and never drops the first events.
   */
  const init = (): Promise<void> => {
    if (!bootstrapPromise) {
      bootstrapPromise = bootstrap();
    }
    // The listener must be attached before the first await in bootstrap so
    // early command results and events cannot be missed.
    attachTransportListener();
    return bootstrapPromise;
  };

  const reconnect = async (): Promise<void> => {
    updateState((prev) => ({
      ...prev,
      connectionStatus: "reconnecting",
      error: null,
    }));
    try {
      const handshake = await transport.reconnect({
        last_sequence: state.lastSequence as Sequence,
      });
      updateState((prev) => ({
        ...prev,
        connectionStatus: "connected",
        negotiated: handshake,
        streamState: "live",
      }));
      await listThreadsFromCore();
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        connectionStatus: "disconnected",
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const disconnect = (): void => {
    updateState((prev) => ({
      ...prev,
      connectionStatus: "disconnected",
      streamState: "idle",
    }));
  };

  const selectAgent = (agentId: string): void => {
    updateState((prev) => ({ ...prev, selectedAgentId: agentId }));
  };

  const openOnboarding = (open: boolean, notice?: string | null): void => {
    updateState((prev) => ({
      ...prev,
      isOnboardingOpen: open,
      onboardingNotice: open ? (notice ?? null) : null,
      onboardingStatus: { isTesting: false, testResult: null, bindingId: undefined },
    }));
  };

  const onboardAgent = async (params: OnboardAgentParams): Promise<AgentProfile> => {
    const envPairs = resolveEnvSecretPairs(params);
    const provider = params.provider ?? "acp";
    const model = params.model ? params.model.trim() : undefined;
    const program = (params.program ?? provider).trim();
    const args = parseCommandLineArgs(params.args);
    const label = (params.label ?? params.name).trim();

    const preferredHarness = provider.toLowerCase().includes("terminal")
      ? "terminal"
      : provider.toLowerCase().includes("native")
        ? "native"
        : "acp";

    // Reuse the tested binding identity when one exists; otherwise ask Core
    // to mint profile and binding identities (null, ADR 0019).
    const testedBindingId = params.bindingId ?? state.onboardingStatus.bindingId ?? null;

    const bindingConfig: HarnessBindingConfigDto = {
      harness_binding_id: testedBindingId,
      agent_profile_id: null,
      program,
      args,
      env_keys: envPairs.envKeys,
      secret_refs: envPairs.secretRefs,
      label: label || params.name,
    };

    const configureEnvelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "configure_agent",
      payload: {
        agent_profile_id: null,
        display_name: params.name,
        preferred_harness: preferredHarness,
        memory_mode: params.memory_mode ?? "session",
        binding: bindingConfig,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    // Only the Core-confirmed identities are adopted; a rejected command
    // propagates and adds no locally fabricated agent.
    const res = await transport.command<{ agent_profile_id?: string; harness_binding_id?: string | null }>(
      configureEnvelope,
    );
    if (!res?.agent_profile_id) {
      throw new Error("configure_agent response did not carry the agent profile identity");
    }
    const agentId = res.agent_profile_id;
    const bindingId = res.harness_binding_id ?? undefined;
    const capabilities =
      params.capabilities ?? state.onboardingStatus.testResult?.capabilities;

    const newAgent: AgentProfile = {
      id: agentId,
      name: params.name,
      provider,
      model,
      program,
      args,
      envKeys: envPairs.envKeys,
      secretRefs: envPairs.secretRefs,
      secretRef: envPairs.secretRefs[0] ?? undefined,
      label,
      bindingId,
      status: "ready",
      capabilities,
    };

    updateState((prev) => ({
      ...prev,
      agents: [...prev.agents, newAgent],
      selectedAgentId: newAgent.id,
      isOnboardingOpen: false,
      onboardingNotice: null,
      onboardingStatus: { isTesting: false, testResult: null, bindingId: undefined },
    }));

    return newAgent;
  };

  const testAgent = async (
    params: TestAgentParams,
  ): Promise<TestAgentResult> => {
    updateState((prev) => ({
      ...prev,
      onboardingStatus: { ...prev.onboardingStatus, isTesting: true, testResult: null },
    }));

    const program = (params.program ?? params.provider ?? "").trim();
    if (!program) {
      const errorMsg = "Harness binding program is required";
      const result: TestAgentResult = { success: false, error: errorMsg };
      updateState((prev) => ({
        ...prev,
        error: `[INVALID_BINDING] ${errorMsg}`,
        onboardingStatus: { ...prev.onboardingStatus, isTesting: false, testResult: result },
      }));
      return result;
    }

    let envPairs: { envKeys: string[]; secretRefs: string[] };
    try {
      envPairs = resolveEnvSecretPairs(params);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      const result: TestAgentResult = { success: false, error: errorMsg };
      updateState((prev) => ({
        ...prev,
        error: `[INVALID_SECRET] ${errorMsg}`,
        onboardingStatus: { ...prev.onboardingStatus, isTesting: false, testResult: result },
      }));
      return result;
    }

    const args = parseCommandLineArgs(params.args);
    const label = (params.label ?? params.model ?? params.provider ?? "").trim() || null;

    // Probe without a binding id unless an existing one is being re-tested;
    // Core returns the (possibly minted) identity in the response (ADR 0019).
    const knownBindingId = params.bindingId ?? state.onboardingStatus.bindingId ?? null;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "test_harness_binding",
      payload: {
        harness_binding_id: knownBindingId,
        program,
        args,
        env_keys: envPairs.envKeys,
        secret_refs: envPairs.secretRefs,
        label,
      } as TestHarnessBindingCommand,
      issued_at: Date.now(),
    };

    const startTime = Date.now();
    try {
      const res = await transport.command<TestHarnessResponseDto>(envelope);
      if (res?.ok === false) {
        const result: TestAgentResult = {
          success: false,
          error: (res?.diagnostics as string | undefined) || "Harness binding probe returned failure",
          capabilities: res?.capabilities,
        };
        updateState((prev) => ({
          ...prev,
          onboardingStatus: { isTesting: false, testResult: result, bindingId: undefined },
        }));
        return result;
      }
      const latencyMs = Math.max(1, Date.now() - startTime);
      const probedBindingId = res?.probed_binding_id ?? undefined;
      const capabilities = res?.capabilities;
      const result: TestAgentResult = {
        success: true,
        latencyMs,
        capabilities,
        bindingId: probedBindingId,
      };
      updateState((prev) => ({
        ...prev,
        onboardingStatus: { isTesting: false, testResult: result, bindingId: probedBindingId },
      }));
      return result;
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      const result: TestAgentResult = { success: false, error: errorMsg };
      updateState((prev) => ({
        ...prev,
        onboardingStatus: { isTesting: false, testResult: result, bindingId: undefined },
      }));
      return result;
    }
  };

  const selectThread = async (threadId: string): Promise<void> => {
    updateState((prev) => ({
      ...prev,
      selectedThreadId: threadId,
      // Selection is independent of the visible list: picking a search
      // result keeps the search results in the pane while switching the
      // conversation being read.
      selectedThread: threadViews.get(threadId) ?? null,
    }));
    await openThread(threadId);
  };

  const setThreadFilter = async (filter: string): Promise<void> => {
    updateState((prev) => ({ ...prev, threadFilter: filter }));

    const query = filter.trim();
    if (!query) {
      // Clearing the query invalidates any in-flight search and restores
      // the authoritative list without dropping cached entities.
      searchGeneration += 1;
      searchResultIds = null;
      const listViews = viewsForIds(listThreadIds);
      updateState((prev) => ({
        ...prev,
        searchActive: false,
        threads: listViews,
        hasMoreThreads,
      }));
      await listThreadsFromCore();
      return;
    }

    const generation = ++searchGeneration;
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "search_threads",
      payload: { query, limit: THREAD_PAGE_LIMIT } as SearchThreadsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      // A late response from request A must never overwrite newer request B.
      if (generation !== searchGeneration) return;
      if (isSnapshotEnvelope(res) && res.data) {
        const listData = res.data as ThreadListResponseDto;
        if (Array.isArray(listData.threads)) {
          for (const summary of listData.threads) {
            upsertSummary(summary);
          }
          searchResultIds = listData.threads.map((s) => s.thread.id);
          updateState((prev) => ({
            ...prev,
            searchActive: true,
            threads: viewsForIds(searchResultIds!),
            // The conversation being read is untouched by search results.
            selectedThread: threadViews.get(prev.selectedThreadId) ?? prev.selectedThread,
          }));
        }
      }
    } catch (err) {
      if (generation !== searchGeneration) return;
      // Surfaced in place; search failures never silently fall back to a
      // client-side title filter pretending to be results.
      updateState((prev) => ({
        ...prev,
        error: `Thread search failed: ${err instanceof Error ? err.message : String(err)}`,
      }));
    }
  };

  const createThread = async (
    title: string,
    agentProfileId?: string,
  ): Promise<ThreadSummaryView> => {
    const targetAgentId = agentProfileId ?? state.selectedAgentId ?? state.agents[0]?.id;
    if (!targetAgentId) {
      // The create_thread contract requires an agent profile; refuse before
      // dispatching instead of inventing one (A01).
      updateState((prev) => ({
        ...prev,
        isOnboardingOpen: true,
        onboardingNotice: "agent_required",
      }));
      throw new Error("Cannot create a thread without an agent profile");
    }
    const selectedAgent =
      state.agents.find((a) => a.id === targetAgentId)?.name ?? targetAgentId;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "create_thread",
      payload: {
        agent_profile_id: targetAgentId,
        title: title.trim() || null,
        project_id: null,
      } as CreateThreadCommand,
      issued_at: Date.now(),
    };

    // The thread identity is Core-owned (ADR 0019): no local fallback id is
    // fabricated when the command fails — the failure propagates, the
    // reason is surfaced, and no unsaved thread is presented as created.
    let res: ThreadDto | { thread: ThreadDto };
    try {
      res = await transport.command<ThreadDto | { thread: ThreadDto }>(envelope);
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        error: `Thread creation failed: ${err instanceof Error ? err.message : String(err)}`,
      }));
      throw err;
    }
    let threadId: string;
    let finalTitle: string;
    if (res && "id" in res) {
      threadId = res.id;
      finalTitle = res.title;
    } else if (res && "thread" in res && res.thread) {
      threadId = res.thread.id;
      finalTitle = res.thread.title;
    } else {
      const message = "create_thread response did not carry the created thread identity";
      updateState((prev) => ({ ...prev, error: message }));
      throw new Error(message);
    }

    const newThread: ThreadSummaryView = {
      id: threadId,
      title: finalTitle,
      agent: selectedAgent,
      status: "running" as ThreadStatus,
      pinned: false,
    };

    threadViews.set(threadId, newThread);
    listThreadIds.unshift(threadId);
    updateState((prev) => {
      const visible = prev.searchActive && searchResultIds
        ? prev.threads
        : viewsForIds(listThreadIds);
      return {
        ...prev,
        threads: visible,
        selectedThreadId: newThread.id,
        selectedThread: newThread,
      };
    });

    // Immediately open the newly created thread; the snapshot may resolve
    // the agent display name, so hand back the refreshed view.
    await openThread(threadId);

    return threadViews.get(threadId) ?? newThread;
  };

  const sendPrompt = async (text: string, threadId?: string): Promise<PromptDispatchResult> => {
    const trimmed = text.trim();
    if (!trimmed) {
      return { status: "rejected", threadId: threadId ?? state.selectedThreadId, reason: "empty prompt" };
    }

    const currentThreadId = threadId ?? state.selectedThreadId;
    if (!currentThreadId || !(currentThreadId === state.selectedThreadId ? state.selectedThread : threadViews.get(currentThreadId))) {
      // No conversation is open: refuse the dispatch instead of sending a
      // command with an empty thread id.
      const message = "Open a conversation before sending a prompt";
      updateState((prev) => ({ ...prev, error: message }));
      return { status: "rejected", threadId: currentThreadId, reason: message };
    }
    // Double-submit guard (A04): one in-flight/pending turn per thread;
    // a repeated send while it is unsettled is ignored, not queued.
    if (state.activeTurns.some((t) => t.threadId === currentThreadId)) {
      return {
        status: "rejected",
        threadId: currentThreadId,
        reason: "A turn is already running in this conversation",
      };
    }
    const threadStore = getTimelineStore(currentThreadId);

    rowCounter += 1;
    const userRowId = `send-${rowCounter}`;
    const replyRowId = `send-${rowCounter}-reply`;

    // 1. Append user prompt row
    threadStore.appendRow({
      id: userRowId,
      kind: "user-message",
      text: trimmed,
      status: null,
      permission: null,
      streaming: false,
    });

    // 2. Append assistant response placeholder row
    threadStore.appendRow({
      id: replyRowId,
      kind: "assistant-message",
      text: "",
      status: null,
      permission: null,
      streaming: true,
    });

    // The turn identity is Core-owned (ADR 0019): the placeholder is
    // resolved from the start_turn response once Core mints it.
    const turn: ActiveTurnState = {
      turnId: null,
      threadId: currentThreadId,
      userRowId,
      replyRowId,
      isStreaming: true,
      cancelState: null,
      notice: null,
    };
    updateState((prev) => ({ ...prev, activeTurns: [...prev.activeTurns, turn] }));

    // 3. Dispatch start_turn command
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "start_turn",
      payload: {
        thread_id: currentThreadId,
        turn_id: null,
        prompt: trimmed,
      } as StartTurnCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<{ admission?: string; turn_id?: string | null }>(envelope);
      const admittedTurnId = res?.turn_id ?? null;
      updateState((prev) => ({
        ...prev,
        activeTurns: prev.activeTurns.map((t) =>
          t.replyRowId === replyRowId && t.threadId === currentThreadId
            ? { ...t, turnId: admittedTurnId }
            : t,
        ),
      }));
      return { status: "admitted", threadId: currentThreadId, turnId: admittedTurnId };
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      // Indeterminate vs rejected (A04): a connection-level failure after
      // dispatch means the prompt may have been delivered — it is never
      // auto-retried, the user is told to verify instead.
      const indeterminate =
        err instanceof ConnectionClosedError ||
        (err instanceof CoreTransportError && err.code === "CONNECTION_CLOSED");
      threadStore.finishStreaming(replyRowId);
      threadStore.appendRow({
        id: `fail-${Date.now()}`,
        kind: "error",
        text: indeterminate
          ? `Delivery indeterminate: the connection dropped during dispatch — the prompt may still be running. Check history before resending. (${reason})`
          : `Prompt not delivered: ${reason}`,
        status: null,
        permission: null,
        streaming: false,
      });
      updateState((prev) => ({
        ...prev,
        activeTurns: prev.activeTurns.filter((t) => t.replyRowId !== replyRowId),
      }));
      return indeterminate
        ? { status: "indeterminate", threadId: currentThreadId, reason }
        : { status: "rejected", threadId: currentThreadId, reason };
    }
  };

  const cancelActiveTurn = async (threadId?: string): Promise<void> => {
    const targetThreadId = threadId ?? state.selectedThreadId;
    const active = state.activeTurns.find((t) => t.threadId === targetThreadId);
    if (!active) return;
    // One cancel in flight per turn.
    if (active.cancelState === "requested") return;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "cancel_turn",
      payload: {
        thread_id: active.threadId,
        turn_id: active.turnId,
        target_operation_id: null,
      } as CancelTurnCommand,
      issued_at: Date.now(),
    };

    // The turn stays active and streaming; only Core's authoritative
    // turn.cancelled event settles it (A04, F05).
    updateState((prev) => ({
      ...prev,
      activeTurns: prev.activeTurns.map((t) =>
        t.replyRowId === active.replyRowId ? { ...t, cancelState: "requested" } : t,
      ),
    }));

    try {
      await transport.command(envelope);
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      updateState((prev) => ({
        ...prev,
        activeTurns: prev.activeTurns.map((t) =>
          t.replyRowId === active.replyRowId
            ? { ...t, cancelState: "failed", notice: `Cancel request failed — the turn may still be running. (${reason})` }
            : t,
        ),
        error: `Cancel request failed: ${reason}`,
      }));
    }
  };

  const decidePermission = async (
    rowId: string,
    decision: PermissionDecision,
  ): Promise<void> => {
    const threadStore = getTimelineStore(state.selectedThreadId);
    const existingRow = threadStore.getRow(rowId);
    const prevDecision = existingRow?.permission?.decision ?? null;
    // Duplicate-submit guard (A04): a decision already in flight — or one
    // already settled — is not sent again.
    if (existingRow?.permission?.submission === "submitting") return;
    if (prevDecision !== null) return;

    if (existingRow?.permission) {
      threadStore.setPermissionSubmission(rowId, "submitting");
    }

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: createOperationId(),
      kind: "respond_permission",
      payload: {
        event_id: rowId,
        decision: (decision || "approved").toLowerCase(),
      } as RespondPermissionCommand,
      issued_at: Date.now(),
    };

    try {
      await transport.command(envelope);
      if (existingRow?.permission) {
        threadStore.setPermissionDecision(rowId, decision);
      }
    } catch (err) {
      if (existingRow?.permission) {
        threadStore.setPermissionSubmission(rowId, "failed");
      }
      updateState((prev) => ({
        ...prev,
        error: `Permission response failed: ${err instanceof Error ? err.message : String(err)}`,
      }));
      throw err;
    }
  };

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getState() {
      return state;
    },
    getTimelineStore,
    init,
    reconnect,
    disconnect,
    release,
    selectAgent,
    openOnboarding,
    onboardAgent,
    testAgent,
    selectThread,
    setThreadFilter,
    createThread,
    loadMoreThreads,
    openThread,
    getHistory,
    loadOlderHistory,
    getHistoryCursor,
    getDiagnostics,
    getContextSnapshot,
    listMemories,
    proposeMemory,
    confirmMemory,
    rejectMemory,
    correctMemory,
    forgetMemory,
    setAgentMemoryMode,
    listIdentityDocuments,
    putIdentityDocument,
    deleteIdentityDocument,
    sendPrompt,
    cancelActiveTurn,
    decidePermission,
    getEpoch: () => sequenceTracker.epoch,
    getDeduplicationStats: () => ({
      seenIds: seenEventIds.size,
      windowSeqs: sequenceTracker.size,
      highWater: sequenceTracker.highWaterSequence,
    }),
    _handleEvent: handleIncomingEvent,
  };
}
