/**
 * Transport-driven Application Store.
 *
 * Coordinates CoreTransport with Desktop state:
 * - All business commands strictly aligned to protocol DTOs.
 * - Agent onboarding, listing, and testing (with opaque secret references).
 * - Thread list, create, search, open, and history driven by Core snapshots.
 * - Prompt dispatch & per-row delta streaming driven by Core events.
 * - Permission approve / deny actions with event settlement and rollback on error.
 * - Turn cancellation via cancel_turn.
 * - Reconnection with sequence cursor and stream.replayed/stream.ready controls.
 * - Idempotent event deduplication by event_id / sequence.
 */
import {
  allThreads,
  standardThread,
  type ThreadFixture,
  type ThreadStatus,
} from "../fixtures/timeline";
import {
  createTimelineStore,
  type PermissionDecision,
  type TimelineRow,
  type TimelineStore,
} from "../features/timeline/timelineStore";
import type { CancelTurnCommand } from "../ipc/dto/CancelTurnCommand";
import type { CommandEnvelope } from "../ipc/dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "../ipc/dto/ConfigureAgentCommand";
import type { CreateThreadCommand } from "../ipc/dto/CreateThreadCommand";
import type { DiagnosticsCommand } from "../ipc/dto/DiagnosticsCommand";
import type { EventEnvelope } from "../ipc/dto/EventEnvelope";
import type { GetHistoryCommand } from "../ipc/dto/GetHistoryCommand";
import type { HarnessBindingConfigDto } from "../ipc/dto/HarnessBindingConfigDto";
import type { ListThreadsCommand } from "../ipc/dto/ListThreadsCommand";
import type { NegotiatedHandshake } from "../ipc/dto/NegotiatedHandshake";
import type { OpenThreadCommand } from "../ipc/dto/OpenThreadCommand";
import type { RespondPermissionCommand } from "../ipc/dto/RespondPermissionCommand";
import type { RuntimeDiagnosticsDto } from "../ipc/dto/RuntimeDiagnosticsDto";
import type { RuntimeStatusCommand } from "../ipc/dto/RuntimeStatusCommand";
import type { SearchThreadsCommand } from "../ipc/dto/SearchThreadsCommand";
import type { Sequence } from "../ipc/dto/Sequence";
import type { SnapshotEnvelope } from "../ipc/dto/SnapshotEnvelope";
import type { StartTurnCommand } from "../ipc/dto/StartTurnCommand";
import type { TestHarnessBindingCommand } from "../ipc/dto/TestHarnessBindingCommand";
import type { ThreadDto } from "../ipc/dto/ThreadDto";
import type { ThreadHistoryResponseDto } from "../ipc/dto/ThreadHistoryResponseDto";
import type { ThreadListResponseDto } from "../ipc/dto/ThreadListResponseDto";
import type { ThreadSnapshotDto } from "../ipc/dto/ThreadSnapshotDto";
import type { ThreadSummaryDto } from "../ipc/dto/ThreadSummaryDto";
import type { CoreTransport, TransportStatus } from "../ipc/transport";

export interface AgentProfile {
  readonly id: string;
  readonly name: string;
  readonly provider: string;
  readonly model: string;
  /** OPAQUE REFERENCE ONLY (e.g. vault://key-1, env:OPENAI_API_KEY). Plaintext is never stored. */
  readonly secretRef?: string;
  readonly program?: string;
  readonly args?: readonly string[];
  readonly envKeys?: readonly string[];
  readonly label?: string;
  readonly bindingId?: string;
  readonly status: "ready" | "testing" | "error";
  readonly latencyMs?: number;
}

export interface OnboardAgentParams {
  readonly name: string;
  readonly provider?: string;
  readonly model?: string;
  readonly program?: string;
  readonly args?: readonly string[] | string;
  readonly envKeys?: readonly string[] | string;
  readonly secretRef?: string;
  readonly label?: string;
  readonly bindingId?: string;
}

export interface TestAgentParams {
  readonly provider?: string;
  readonly model?: string;
  readonly program?: string;
  readonly args?: readonly string[] | string;
  readonly envKeys?: readonly string[] | string;
  readonly secretRef?: string;
  readonly label?: string;
  readonly bindingId?: string;
}

export type StreamStatus = "idle" | "live" | "replaying" | "ready";

export interface ActiveTurnState {
  readonly turnId: string;
  readonly threadId: string;
  readonly userRowId: string;
  readonly replyRowId: string;
  readonly isStreaming: boolean;
}

export interface ApplicationState {
  readonly connectionStatus: TransportStatus;
  readonly negotiated: NegotiatedHandshake | null;
  readonly streamState: StreamStatus;
  readonly lastSequence: number;
  readonly error: string | null;

  readonly agents: readonly AgentProfile[];
  readonly selectedAgentId: string;
  readonly isOnboardingOpen: boolean;
  readonly onboardingStatus: {
    readonly isTesting: boolean;
    readonly testResult: { success: boolean; latencyMs?: number; error?: string } | null;
    readonly bindingId?: string;
  };

  readonly threads: readonly ThreadFixture[];
  readonly selectedThreadId: string;
  readonly threadFilter: string;

  readonly activeTurn: ActiveTurnState | null;
  readonly runtimeDiagnostics: RuntimeDiagnosticsDto | null;
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

  // Agent Operations
  selectAgent(agentId: string): void;
  openOnboarding(open: boolean): void;
  onboardAgent(params: OnboardAgentParams): Promise<AgentProfile>;
  testAgent(params: TestAgentParams): Promise<{ success: boolean; latencyMs?: number; error?: string }>;

  // Thread Operations
  selectThread(threadId: string): Promise<void>;
  setThreadFilter(filter: string): Promise<void>;
  createThread(title: string, agentProfileId?: string): Promise<ThreadFixture>;
  openThread(threadId: string): Promise<void>;
  getHistory(threadId: string, limit?: number): Promise<void>;
  getDiagnostics(threadId?: string | null): Promise<RuntimeDiagnosticsDto | null>;

  // Prompt & Turn Execution
  sendPrompt(text: string): Promise<void>;
  cancelActiveTurn(): Promise<void>;

  // Permissions
  decidePermission(rowId: string, decision: PermissionDecision): Promise<void>;

  // Test helpers
  _handleEvent(event: EventEnvelope): void;
}

const DEFAULT_AGENTS: AgentProfile[] = [
  {
    id: "agent-alpha",
    name: "alpha (ACP)",
    provider: "acp",
    model: "claude-3-7-sonnet",
    program: "/usr/local/bin/acp-alpha",
    args: ["--mode", "server"],
    envKeys: ["ANTHROPIC_API_KEY"],
    label: "Alpha ACP",
    bindingId: "bin_alpha_01",
    secretRef: "vault://acp-alpha",
    status: "ready",
    latencyMs: 18,
  },
  {
    id: "agent-beta",
    name: "beta (ACP)",
    provider: "acp",
    model: "claude-3-5-sonnet",
    program: "/usr/local/bin/acp-beta",
    args: ["--mode", "server"],
    envKeys: ["ANTHROPIC_API_KEY"],
    label: "Beta ACP",
    bindingId: "bin_beta_01",
    secretRef: "vault://acp-beta",
    status: "ready",
    latencyMs: 24,
  },
];

const MAX_STREAM_LOG = 50;

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
 * Validates and converts raw secret input to an opaque reference token if necessary.
 * Never stores or leaks plaintext.
 */
export function sanitizeSecretRef(input?: string): string | undefined {
  if (!input) return undefined;
  const trimmed = input.trim();
  if (!trimmed) return undefined;

  // If already an opaque reference scheme, preserve it
  if (
    trimmed.startsWith("vault://") ||
    trimmed.startsWith("env:") ||
    trimmed.startsWith("ref:") ||
    trimmed.startsWith("secret://")
  ) {
    return trimmed;
  }

  // If user pasted arbitrary token, convert immediately to opaque identifier reference
  const hash = Math.random().toString(36).slice(2, 10);
  return `ref:opaque-sec-${hash}`;
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

function threadSummaryToFixture(
  summary: ThreadSummaryDto,
  agents: readonly AgentProfile[],
  existingRows: readonly TimelineRow[] = [],
): ThreadFixture {
  const status: ThreadStatus =
    summary.active_turn?.state === "active" ? "running" : "completed";
  const agentProfile = agents.find(
    (a) => a.id === summary.thread.agent_profile_id || a.name === summary.thread.agent_profile_id,
  );
  return {
    id: summary.thread.id,
    title: summary.thread.title,
    agent: agentProfile?.name ?? summary.thread.agent_profile_id,
    status,
    pinned: summary.thread.state === "pinned",
    rows: existingRows,
  };
}

export function createApplicationStore(
  transport: CoreTransport,
  options: {
    includeHugeThread?: boolean;
    initialThreads?: readonly ThreadFixture[];
    initialAgents?: readonly AgentProfile[];
  } = {},
): ApplicationStore {
  const listeners = new Set<() => void>();
  const seenEventIds = new Set<string>();
  const processedSequences = new Set<number>();
  const timelineStores = new Map<string, TimelineStore>();

  const baseThreads =
    options.initialThreads ?? allThreads(options.includeHugeThread ?? false);
  const baseAgents = options.initialAgents ?? DEFAULT_AGENTS;

  // Initialize per-thread timeline stores
  for (const thread of baseThreads) {
    timelineStores.set(thread.id, createTimelineStore(thread.rows));
  }

  let state: ApplicationState = {
    connectionStatus: transport.status(),
    negotiated: null,
    streamState: "idle",
    lastSequence: 0,
    error: null,

    agents: [...baseAgents],
    selectedAgentId: baseAgents[0]?.id ?? "",
    isOnboardingOpen: false,
    onboardingStatus: {
      isTesting: false,
      testResult: null,
    },

    threads: [...baseThreads],
    selectedThreadId: baseThreads[0]?.id ?? standardThread.id,
    threadFilter: "",

    activeTurn: null,
    runtimeDiagnostics: null,
    streamLog: [],
  };

  let opCounter = 0;
  let promptCounter = 0;
  let threadCounter = 0;
  let unsubscribeTransport: (() => void) | null = null;
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
    let store = timelineStores.get(threadId);
    if (!store) {
      const thread = state.threads.find((t) => t.id === threadId);
      store = createTimelineStore(thread ? thread.rows : []);
      timelineStores.set(threadId, store);
    }
    return store;
  };

  const applyThreadSnapshot = (
    snapshot: ThreadSnapshotDto,
    threadId: string,
  ): void => {
    const threadStore = getTimelineStore(threadId);

    // If turns are in the snapshot and threadStore is currently empty, populate it
    if (threadStore.rowCount() === 0 && snapshot.turns && snapshot.turns.length > 0) {
      for (const turn of snapshot.turns) {
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
          selectedAgentId: newAgent.id,
        }));
      }
    }
  };

  const handleIncomingEvent = (event: EventEnvelope): void => {
    // 1. Idempotency Check: deduplicate by event_id and sequence
    if (seenEventIds.has(event.event_id) || processedSequences.has(event.sequence)) {
      return;
    }
    seenEventIds.add(event.event_id);
    processedSequences.add(event.sequence);

    const body = event.body;
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

    // 3. Route specific event kinds to thread timeline
    const targetThreadId = event.thread_id ?? state.selectedThreadId;
    const threadStore = getTimelineStore(targetThreadId);

    if (body.kind === "turn.started") {
      updateState((prev) => {
        if (!prev.activeTurn) return prev;
        return {
          ...prev,
          activeTurn: {
            ...prev.activeTurn,
            isStreaming: true,
          },
        };
      });
    } else if (body.kind === "message.delta") {
      const deltaText = "text" in body ? body.text : "";
      if (deltaText) {
        const activeTurn = state.activeTurn;
        if (activeTurn && activeTurn.threadId === targetThreadId) {
          threadStore.appendDelta(activeTurn.replyRowId, deltaText);
        }
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
      const activeTurn = state.activeTurn;
      if (activeTurn && activeTurn.threadId === targetThreadId) {
        threadStore.finishStreaming(activeTurn.replyRowId);
        updateState((prev) => ({
          ...prev,
          activeTurn: null,
        }));
      }
    } else if (body.kind === "turn.failed") {
      const activeTurn = state.activeTurn;
      const failBody = body as { kind: "turn.failed"; reason: string };
      if (activeTurn && activeTurn.threadId === targetThreadId) {
        threadStore.finishStreaming(activeTurn.replyRowId);
        threadStore.appendRow({
          id: `fail-${event.sequence}`,
          kind: "error",
          text: failBody.reason || "Turn failed",
          status: null,
          permission: null,
          streaming: false,
        });
        updateState((prev) => ({
          ...prev,
          activeTurn: null,
        }));
      }
    } else if (body.kind === "turn.cancelled") {
      const activeTurn = state.activeTurn;
      const cancelBody = body as { kind: "turn.cancelled"; reason?: string };
      if (activeTurn && activeTurn.threadId === targetThreadId) {
        threadStore.finishStreaming(activeTurn.replyRowId);
        threadStore.appendRow({
          id: `cancel-${event.sequence}`,
          kind: "error",
          text: cancelBody.reason ?? diagnostic ?? "turn cancelled by user",
          status: null,
          permission: null,
          streaming: false,
        });
        updateState((prev) => ({
          ...prev,
          activeTurn: null,
        }));
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

  const listThreadsFromCore = async (): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_list_threads_${++opCounter}_${Date.now()}`,
      kind: "list_threads",
      payload: { cursor: null, limit: 50 } as ListThreadsCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const listData = res.data as ThreadListResponseDto;
        if (Array.isArray(listData.threads)) {
          const updatedThreads = listData.threads.map((s) => {
            const existing = state.threads.find((t) => t.id === s.thread.id);
            return threadSummaryToFixture(s, state.agents, existing?.rows ?? []);
          });
          const existingExtras = state.threads.filter(
            (t) => !updatedThreads.some((u) => u.id === t.id),
          );
          const combined = [...updatedThreads, ...existingExtras];
          updateState((prev) => ({
            ...prev,
            threads: combined.length > 0 ? combined : prev.threads,
            selectedThreadId:
              combined.find((t) => t.id === prev.selectedThreadId)?.id ??
              combined[0]?.id ??
              prev.selectedThreadId,
          }));
        }
      }
    } catch {
      // Retain fallback threads if Core command is unavailable
    }
  };

  const getRuntimeStatusFromCore = async (): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_runtime_status_${++opCounter}_${Date.now()}`,
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
      operation_id: `op_open_thread_${++opCounter}_${Date.now()}`,
      kind: "open_thread",
      payload: { thread_id: threadId, history_limit: 100 } as OpenThreadCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const snap = res.data as ThreadSnapshotDto;
        applyThreadSnapshot(snap, threadId);
      }
    } catch {
      // Retain existing timeline store
    }
  };

  const getHistory = async (
    threadId: string,
    limit: number = 50,
  ): Promise<void> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_get_history_${++opCounter}_${Date.now()}`,
      kind: "get_history",
      payload: {
        thread_id: threadId,
        cursor: null,
        limit,
      } as GetHistoryCommand,
      issued_at: Date.now(),
    };

    try {
      const res = await transport.command<SnapshotEnvelope>(envelope);
      if (isSnapshotEnvelope(res) && res.data) {
        const hist = res.data as ThreadHistoryResponseDto;
        const threadStore = getTimelineStore(threadId);
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
    } catch {
      // Retain existing timeline store
    }
  };

  const getDiagnostics = async (
    threadId?: string | null,
  ): Promise<RuntimeDiagnosticsDto | null> => {
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_diagnostics_${++opCounter}_${Date.now()}`,
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

  const init = async (): Promise<void> => {
    updateState((prev) => ({
      ...prev,
      connectionStatus: "connecting",
      error: null,
    }));

    if (unsubscribeTransport) {
      unsubscribeTransport();
    }
    unsubscribeTransport = transport.subscribe(handleIncomingEvent);

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

      // Show onboarding if no agents configured
      if (state.agents.length === 0) {
        updateState((prev) => ({ ...prev, isOnboardingOpen: true }));
      } else if (state.selectedThreadId) {
        await openThread(state.selectedThreadId);
      }
    } catch (err) {
      updateState((prev) => ({
        ...prev,
        connectionStatus: transport.status(),
        error: err instanceof Error ? err.message : String(err),
      }));
    }
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

  const openOnboarding = (open: boolean): void => {
    updateState((prev) => ({
      ...prev,
      isOnboardingOpen: open,
      onboardingStatus: { isTesting: false, testResult: null, bindingId: undefined },
    }));
  };

  const onboardAgent = async (params: OnboardAgentParams): Promise<AgentProfile> => {
    const sanitizedRef = sanitizeSecretRef(params.secretRef);
    const agentId = `agent-${params.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${Date.now().toString(36)}`;
    const provider = params.provider ?? "acp";
    const model = params.model ?? "claude-3-7-sonnet";
    const program = (params.program ?? provider).trim();
    const args = parseStringList(params.args);
    const envKeys = parseStringList(params.envKeys);
    const label = (params.label ?? params.name).trim();

    const preferredHarness = provider.toLowerCase().includes("terminal")
      ? "terminal"
      : provider.toLowerCase().includes("native")
        ? "native"
        : "acp";

    // Reuse tested binding ID if available, otherwise generate new one
    const bindingId =
      params.bindingId ??
      state.onboardingStatus.bindingId ??
      `bin_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;

    const bindingConfig: HarnessBindingConfigDto = {
      harness_binding_id: bindingId,
      agent_profile_id: agentId,
      program,
      args,
      env_keys: envKeys,
      secret_refs: sanitizedRef ? [sanitizedRef] : [],
      label: label || params.name,
    };

    const configureEnvelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_configure_agent_${++opCounter}_${Date.now()}`,
      kind: "configure_agent",
      payload: {
        agent_profile_id: agentId,
        display_name: params.name,
        preferred_harness: preferredHarness,
        memory_mode: "session",
        binding: bindingConfig,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    await transport.command(configureEnvelope);

    const newAgent: AgentProfile = {
      id: agentId,
      name: params.name,
      provider,
      model,
      program,
      args,
      envKeys,
      label,
      bindingId,
      secretRef: sanitizedRef,
      status: "ready",
    };

    updateState((prev) => ({
      ...prev,
      agents: [...prev.agents, newAgent],
      selectedAgentId: newAgent.id,
      isOnboardingOpen: false,
      onboardingStatus: { isTesting: false, testResult: null, bindingId: undefined },
    }));

    return newAgent;
  };

  const testAgent = async (
    params: TestAgentParams,
  ): Promise<{ success: boolean; latencyMs?: number; error?: string }> => {
    updateState((prev) => ({
      ...prev,
      onboardingStatus: { ...prev.onboardingStatus, isTesting: true, testResult: null },
    }));

    const program = (params.program ?? params.provider ?? "").trim();
    if (!program) {
      const errorMsg = "Harness binding program is required";
      const result = { success: false, error: errorMsg };
      updateState((prev) => ({
        ...prev,
        error: `[INVALID_BINDING] ${errorMsg}`,
        onboardingStatus: { ...prev.onboardingStatus, isTesting: false, testResult: result },
      }));
      return result;
    }

    const sanitizedRef = sanitizeSecretRef(params.secretRef);
    const args = parseStringList(params.args);
    const envKeys = parseStringList(params.envKeys);
    const label = (params.label ?? params.model ?? params.provider ?? "").trim() || null;

    const bindingId =
      params.bindingId ??
      state.onboardingStatus.bindingId ??
      `bin_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_test_harness_${++opCounter}_${Date.now()}`,
      kind: "test_harness_binding",
      payload: {
        harness_binding_id: bindingId,
        program,
        args,
        env_keys: envKeys,
        secret_refs: sanitizedRef ? [sanitizedRef] : [],
        label,
      } as TestHarnessBindingCommand,
      issued_at: Date.now(),
    };

    const startTime = Date.now();
    try {
      await transport.command(envelope);
      const latencyMs = Math.max(1, Date.now() - startTime);
      const result = { success: true, latencyMs };
      updateState((prev) => ({
        ...prev,
        onboardingStatus: { isTesting: false, testResult: result, bindingId },
      }));
      return result;
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      const result = { success: false, error: errorMsg };
      updateState((prev) => ({
        ...prev,
        onboardingStatus: { isTesting: false, testResult: result, bindingId },
      }));
      return result;
    }
  };

  const selectThread = async (threadId: string): Promise<void> => {
    updateState((prev) => ({ ...prev, selectedThreadId: threadId }));
    await openThread(threadId);
  };

  const setThreadFilter = async (filter: string): Promise<void> => {
    updateState((prev) => ({ ...prev, threadFilter: filter }));

    const query = filter.trim();
    if (query) {
      const envelope: CommandEnvelope = {
        protocol_version: state.negotiated?.selected_version ?? 1,
        operation_id: `op_search_threads_${++opCounter}_${Date.now()}`,
        kind: "search_threads",
        payload: { query, limit: 50 } as SearchThreadsCommand,
        issued_at: Date.now(),
      };

      try {
        const res = await transport.command<SnapshotEnvelope>(envelope);
        if (isSnapshotEnvelope(res) && res.data) {
          const listData = res.data as ThreadListResponseDto;
          const filtered = listData.threads.map((s) => {
            const existing = state.threads.find((t) => t.id === s.thread.id);
            return threadSummaryToFixture(s, state.agents, existing?.rows ?? []);
          });
          updateState((prev) => ({ ...prev, threads: filtered }));
        }
      } catch {
        // Retain client-side fallback filtering
      }
    } else {
      await listThreadsFromCore();
    }
  };

  const createThread = async (
    title: string,
    agentProfileId?: string,
  ): Promise<ThreadFixture> => {
    const targetAgentId =
      agentProfileId ?? state.selectedAgentId ?? state.agents[0]?.id ?? "agent-alpha";
    const selectedAgent =
      state.agents.find((a) => a.id === targetAgentId)?.name ?? "alpha (ACP)";

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_create_thread_${++opCounter}_${Date.now()}`,
      kind: "create_thread",
      payload: {
        agent_profile_id: targetAgentId,
        title: title.trim() || null,
        project_id: null,
      } as CreateThreadCommand,
      issued_at: Date.now(),
    };

    let threadId = `thread-${++threadCounter}_${Date.now()}`;
    let finalTitle = title.trim() || "New conversation";

    try {
      const res = await transport.command<ThreadDto | { thread: ThreadDto }>(envelope);
      if (res && "id" in res) {
        threadId = res.id;
        finalTitle = res.title;
      } else if (res && "thread" in res && res.thread) {
        threadId = res.thread.id;
        finalTitle = res.thread.title;
      }
    } catch {
      // Fallback in-memory creation
    }

    const newThread: ThreadFixture = {
      id: threadId,
      title: finalTitle,
      agent: selectedAgent,
      status: "running" as ThreadStatus,
      pinned: false,
      rows: [],
    };

    timelineStores.set(threadId, createTimelineStore([]));

    updateState((prev) => ({
      ...prev,
      threads: [newThread, ...prev.threads],
      selectedThreadId: newThread.id,
    }));

    // Immediately open the newly created thread
    await openThread(threadId);

    return newThread;
  };

  const sendPrompt = async (text: string): Promise<void> => {
    const trimmed = text.trim();
    if (!trimmed) return;

    promptCounter += 1;
    const turnIndex = promptCounter;
    const currentThreadId = state.selectedThreadId;
    const threadStore = getTimelineStore(currentThreadId);

    const userRowId = `send-${turnIndex}`;
    const replyRowId = `send-${turnIndex}-reply`;
    const turnId = `trn_${Date.now()}_${turnIndex}`;

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

    updateState((prev) => ({
      ...prev,
      activeTurn: {
        turnId,
        threadId: currentThreadId,
        userRowId,
        replyRowId,
        isStreaming: true,
      },
    }));

    // 3. Dispatch start_turn command
    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_start_turn_${turnIndex}`,
      kind: "start_turn",
      payload: {
        thread_id: currentThreadId,
        turn_id: turnId,
        prompt: trimmed,
      } as StartTurnCommand,
      issued_at: Date.now(),
    };

    try {
      await transport.command(envelope);
    } catch (err) {
      threadStore.finishStreaming(replyRowId);
      threadStore.appendRow({
        id: `fail-${Date.now()}`,
        kind: "error",
        text: `Prompt dispatch failed: ${err instanceof Error ? err.message : String(err)}`,
        status: null,
        permission: null,
        streaming: false,
      });
      updateState((prev) => ({
        ...prev,
        activeTurn: null,
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const cancelActiveTurn = async (): Promise<void> => {
    const active = state.activeTurn;
    if (!active) return;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_cancel_turn_${++opCounter}_${Date.now()}`,
      kind: "cancel_turn",
      payload: {
        thread_id: active.threadId,
        turn_id: active.turnId,
        target_operation_id: null,
      } as CancelTurnCommand,
      issued_at: Date.now(),
    };

    try {
      await transport.command(envelope);
    } catch {
      // Local completion fallback
    }

    const threadStore = getTimelineStore(active.threadId);
    threadStore.finishStreaming(active.replyRowId);
    updateState((prev) => ({
      ...prev,
      activeTurn: null,
    }));
  };

  const decidePermission = async (
    rowId: string,
    decision: PermissionDecision,
  ): Promise<void> => {
    const threadStore = getTimelineStore(state.selectedThreadId);
    const existingRow = threadStore.getRow(rowId);
    const prevDecision = existingRow?.permission?.decision ?? null;

    const envelope: CommandEnvelope = {
      protocol_version: state.negotiated?.selected_version ?? 1,
      operation_id: `op_respond_permission_${++opCounter}_${Date.now()}`,
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
      if (existingRow?.permission && prevDecision) {
        threadStore.setPermissionDecision(rowId, prevDecision);
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
    selectAgent,
    openOnboarding,
    onboardAgent,
    testAgent,
    selectThread,
    setThreadFilter,
    createThread,
    openThread,
    getHistory,
    getDiagnostics,
    sendPrompt,
    cancelActiveTurn,
    decidePermission,
    _handleEvent: handleIncomingEvent,
  };
}
