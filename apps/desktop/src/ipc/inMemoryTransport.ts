import type { AgentProfileDto } from "./dto/AgentProfileDto";
import type { CancelTurnCommand } from "./dto/CancelTurnCommand";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "./dto/ConfigureAgentCommand";
import type { ContextSnapshotDto } from "./dto/ContextSnapshotDto";
import type { CreateThreadCommand } from "./dto/CreateThreadCommand";
import type { DeleteIdentityDocumentCommand } from "./dto/DeleteIdentityDocumentCommand";
import type { DiagnosticsCommand } from "./dto/DiagnosticsCommand";
import type { EventBody } from "./dto/EventBody";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { GetContextSnapshotCommand } from "./dto/GetContextSnapshotCommand";
import type { GetHistoryCommand } from "./dto/GetHistoryCommand";
import type { HarnessBindingConfigDto } from "./dto/HarnessBindingConfigDto";
import type { HarnessBindingDto } from "./dto/HarnessBindingDto";
import type { HistoryCursorDto } from "./dto/HistoryCursorDto";
import type { HistoryEntryDto } from "./dto/HistoryEntryDto";
import type { IdentityDocumentDto } from "./dto/IdentityDocumentDto";
import type { ListIdentityDocumentsCommand } from "./dto/ListIdentityDocumentsCommand";
import type { ListThreadsCommand } from "./dto/ListThreadsCommand";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";
import type { OpenThreadCommand } from "./dto/OpenThreadCommand";
import type { PermissionDto } from "./dto/PermissionDto";
import type { PutIdentityDocumentCommand } from "./dto/PutIdentityDocumentCommand";
import type { RuntimeDiagnosticsDto } from "./dto/RuntimeDiagnosticsDto";
import type { SearchThreadsCommand } from "./dto/SearchThreadsCommand";
import type { Sequence } from "./dto/Sequence";
import type { SnapshotEnvelope } from "./dto/SnapshotEnvelope";
import type { StartTurnCommand } from "./dto/StartTurnCommand";
import type { TestHarnessBindingCommand } from "./dto/TestHarnessBindingCommand";
import type { ThreadDto } from "./dto/ThreadDto";
import type { ThreadHistoryResponseDto } from "./dto/ThreadHistoryResponseDto";
import type { ThreadListResponseDto } from "./dto/ThreadListResponseDto";
import type { ThreadSnapshotDto } from "./dto/ThreadSnapshotDto";
import type { ThreadSummaryDto } from "./dto/ThreadSummaryDto";
import type { TurnDto } from "./dto/TurnDto";
import type { ListMemoriesCommand } from "./dto/ListMemoriesCommand";
import type { ProposeMemoryCommand } from "./dto/ProposeMemoryCommand";
import type { ConfirmMemoryCommand } from "./dto/ConfirmMemoryCommand";
import type { RejectMemoryCommand } from "./dto/RejectMemoryCommand";
import type { CorrectMemoryCommand } from "./dto/CorrectMemoryCommand";
import type { ForgetMemoryCommand } from "./dto/ForgetMemoryCommand";
import type { MemoryRecordDto } from "./dto/MemoryRecordDto";
import type { MemoryListResponseDto } from "./dto/MemoryListResponseDto";
import { validateCommandEnvelope } from "./commandContract";
import { ConnectionClosedError, InvalidCommandError } from "./errors";
import { eventFixtures, negotiatedFixture } from "./fixtures";
import {
  allThreads,
  streamingReplyChunks,
  type ThreadFixture,
} from "../fixtures/timeline";
import type { CoreTransport, ReconnectCursor, TransportStatus } from "./transport";

export interface InMemoryTransportOptions {
  /** Handshake result to return; defaults to the negotiated fixture. */
  readonly negotiated?: NegotiatedHandshake;
  /** Events to replay; defaults to the fixture stream. */
  readonly events?: readonly EventEnvelope[];
  /** Custom command execution handler. */
  readonly commandHandler?: (command: CommandEnvelope) => Promise<unknown> | unknown;
  /** Initial connection status. Defaults to "connected". */
  readonly initialStatus?: TransportStatus;
  /** Initial thread fixtures. */
  readonly initialThreads?: readonly ThreadFixture[];
  /** Initial agent profile fixtures. */
  readonly initialAgents?: readonly AgentProfileDto[];
  /** Initial harness bindings. */
  readonly initialBindings?: readonly HarnessBindingDto[];
  /** Include 100k row huge thread in default fixtures. */
  readonly includeHugeThread?: boolean;
  /** Automatically stream reply deltas on start_turn. Defaults to true. */
  readonly autoStreamReplies?: boolean;
  /** Initial identity document fixtures. */
  readonly initialIdentityDocuments?: readonly IdentityDocumentDto[];
  /** Initial context snapshots fixtures. */
  readonly initialContextSnapshots?: readonly ContextSnapshotDto[];
  /** Initial memory fixtures. */
  readonly initialMemories?: readonly MemoryRecordDto[];
}

const DEFAULT_AGENTS_DTO: AgentProfileDto[] = [
  {
    id: "agp_fixture000000001",
    display_name: "alpha (ACP)",
    preferred_harness: "acp",
    memory_mode: "session",
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "agp_fixture000000002",
    display_name: "beta (ACP)",
    preferred_harness: "acp",
    memory_mode: "session",
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
];

const DEFAULT_BINDINGS_DTO: HarnessBindingDto[] = [
  {
    id: "hsb_fixture000000001",
    agent_profile_id: "agp_fixture000000001",
    program: "/usr/local/bin/acp-alpha",
    args: ["--mode", "server"],
    env_keys: ["ANTHROPIC_API_KEY"],
    secret_refs: ["vault://acp-alpha"],
    label: "Alpha ACP",
    created_at: 1700000000000,
  },
  {
    id: "hsb_fixture000000002",
    agent_profile_id: "agp_fixture000000002",
    program: "/usr/local/bin/acp-beta",
    args: ["--mode", "server"],
    env_keys: ["ANTHROPIC_API_KEY"],
    secret_refs: ["vault://acp-beta"],
    label: "Beta ACP",
    created_at: 1700000000000,
  },
];

/**
 * In-memory `CoreTransport` for tests and the fixture shell.
 *
 * Deterministic by construction: fixture events replay synchronously,
 * commands execute the real protocol command contracts (no pseudo-actions),
 * and snapshots / streaming events are emitted accurately.
 */
function isSecretShaped(text: string): boolean {
  if (/sk-[a-zA-Z0-9_\-]{16,}/.test(text)) return true;
  if (/AKIA[0-9A-Z]{16}/.test(text)) return true;
  if (/-----BEGIN [A-Z ]*PRIVATE KEY-----/.test(text)) return true;
  if (/gh[pousr]_[a-zA-Z0-9]{20,}/.test(text)) return true;
  if (/(?:password|token|secret|api_key|apikey)\s*[:=]\s*["']?[^\s"';&]{8,}/i.test(text)) return true;
  return false;
}

export class InMemoryTransport implements CoreTransport {
  readonly id = "in-memory";
  readonly #negotiated: NegotiatedHandshake;
  #events: EventEnvelope[];
  readonly #listeners = new Set<(event: EventEnvelope) => void>();
  readonly #sent: CommandEnvelope[] = [];
  #status: TransportStatus;
  #commandHandler?: (command: CommandEnvelope) => Promise<unknown> | unknown;
  #nextSeq: number;
  readonly #autoStreamReplies: boolean;
  #mintSeq = 100;

  #threadFixtures: ThreadFixture[];
  #agents: AgentProfileDto[];
  #bindings = new Map<string, HarnessBindingDto>();
  #identityDocs: IdentityDocumentDto[] = [];
  #contextSnapshots = new Map<string, ContextSnapshotDto>();
  #memories: MemoryRecordDto[] = [];

  constructor(options: InMemoryTransportOptions = {}) {
    this.#negotiated = options.negotiated ?? negotiatedFixture;
    this.#events = [...(options.events ?? eventFixtures)].sort(
      (a, b) => a.sequence - b.sequence,
    );
    this.#status = options.initialStatus ?? "connected";
    this.#commandHandler = options.commandHandler;
    this.#nextSeq = this.#events.reduce((max, e) => Math.max(max, e.sequence), 0) + 1;
    this.#autoStreamReplies = options.autoStreamReplies ?? true;

    this.#threadFixtures = options.initialThreads
      ? [...options.initialThreads]
      : [...allThreads(options.includeHugeThread ?? false)];
    this.#agents = options.initialAgents
      ? [...options.initialAgents]
      : [...DEFAULT_AGENTS_DTO];

    const bindingsToSeed = options.initialBindings ?? DEFAULT_BINDINGS_DTO;
    for (const binding of bindingsToSeed) {
      this.#bindings.set(binding.id, structuredClone(binding));
    }

    if (options.initialIdentityDocuments) {
      this.#identityDocs = options.initialIdentityDocuments.map((d) => structuredClone(d));
    }
    if (options.initialContextSnapshots) {
      for (const snap of options.initialContextSnapshots) {
        if (snap.turn_id) {
          this.#contextSnapshots.set(snap.turn_id, structuredClone(snap));
        }
      }
    } else {
      const defSnap1: ContextSnapshotDto = {
        turn_id: "trn_p22store0000000000001",
        thread_id: "thr_p22store0000000000000001",
        memory_mode: "long_term",
        created_at: Date.now(),
        passthrough: false,
        budget: {
          identity_limit_tokens: 1024,
          memory_limit_tokens: 2048,
          prompt_tokens: 120,
          identity_tokens: 250,
          memory_tokens: 680,
          total_tokens: 1050,
        },
        identity: [
          {
            document_id: "idd_fixture000000001",
            kind: "about",
            tokens: 250,
          },
        ],
        memories: [
          {
            memory_id: "mem_fixture000000001",
            kind: "preference",
            scope_kind: "project",
            scope_target: null,
            confidence: 95,
            explicit: true,
            tokens: 680,
            score: 0.95,
            why_selected: "matched terms: rust, context, budget",
            provenance_thread_id: "thr_p22store0000000000000001",
            provenance_turn_id: "trn_p22store0000000000001",
          },
        ],
        dropped: [],
        degraded: null,
        rendered_prompt: null,
      };
      this.#contextSnapshots.set(defSnap1.turn_id, defSnap1);
    }
    if (options.initialMemories) {
      this.#memories = options.initialMemories.map((m) => structuredClone(m));
    }
  }

  /** Sets a context snapshot fixture in memory. */
  setContextSnapshot(snapshot: ContextSnapshotDto): void {
    this.#contextSnapshots.set(snapshot.turn_id, structuredClone(snapshot));
  }

  /** Active identity documents in memory. */
  get identityDocuments(): readonly IdentityDocumentDto[] {
    return this.#identityDocs;
  }

  /** Active persistent memories in memory. */
  get memories(): readonly MemoryRecordDto[] {
    return this.#memories;
  }

  /** Sets memory fixtures in memory. */
  setMemories(memories: readonly MemoryRecordDto[]): void {
    this.#memories = memories.map((m) => structuredClone(m));
  }

  /** Commands sent through `send` or `command`, in dispatch order. */
  get sentCommands(): readonly CommandEnvelope[] {
    return this.#sent;
  }

  /** The current event history in memory. */
  get eventHistory(): readonly EventEnvelope[] {
    return this.#events;
  }

  /** Active agent profiles in memory. */
  get agents(): readonly AgentProfileDto[] {
    return this.#agents;
  }

  /** Active harness bindings in memory. */
  get bindings(): ReadonlyMap<string, HarnessBindingDto> {
    return this.#bindings;
  }

  /** Active threads in memory. */
  get threadFixtures(): readonly ThreadFixture[] {
    return this.#threadFixtures;
  }

  /**
   * Mints a deterministic, contract-valid entity identifier (ADR 0019).
   * Bodies derive from a per-instance counter inside the `[0-9a-z]` body
   * rules; the range starts above the checked-in fixture ids.
   */
  #mintId(prefix: "thr" | "trn" | "agp" | "hsb" | "evt" | "idd" | "mem"): string {
    this.#mintSeq += 1;
    return `${prefix}_minted${String(this.#mintSeq).padStart(10, "0")}`;
  }

  status(): TransportStatus {
    return this.#status;
  }

  setCommandHandler(handler: (command: CommandEnvelope) => Promise<unknown> | unknown): void {
    this.#commandHandler = handler;
  }

  simulateDisconnect(): void {
    this.#status = "disconnected";
  }

  connect(): Promise<NegotiatedHandshake> {
    if (this.#status === "closed") {
      return Promise.reject(new ConnectionClosedError());
    }
    this.#status = "connected";
    return Promise.resolve(structuredClone(this.#negotiated));
  }

  handshake(): Promise<NegotiatedHandshake> {
    return this.connect();
  }

  async command<T = unknown>(envelope: CommandEnvelope): Promise<T> {
    if (this.#status === "closed") {
      throw new ConnectionClosedError();
    }
    // A01: the fixture transport enforces the same envelope contract as
    // real Core serde — a command production would reject never executes.
    const rejection = validateCommandEnvelope(envelope);
    if (rejection !== null) {
      throw new InvalidCommandError(`Command rejected by contract validation: ${rejection}`, envelope);
    }
    const cloned = structuredClone(envelope);
    this.#sent.push(cloned);

    if (this.#commandHandler) {
      const result = await this.#commandHandler(cloned);
      // A handler that answers `undefined` defers to the built-in protocol
      // handling, so tests can override one command and keep the rest.
      if (result !== undefined) {
        return result as T;
      }
    }

    // Default built-in handling for core protocol commands
    return this.#defaultCommandHandler(cloned) as T;
  }

  send(command: CommandEnvelope): Promise<void> {
    return this.command(command).then(() => undefined);
  }

  subscribe(onEvent: (event: EventEnvelope) => void): () => void {
    this.#listeners.add(onEvent);
    for (const event of this.#events) {
      onEvent(structuredClone(event));
    }
    return () => {
      this.#listeners.delete(onEvent);
    };
  }

  async reconnect(cursor?: ReconnectCursor): Promise<NegotiatedHandshake> {
    if (this.#status === "closed") {
      throw new ConnectionClosedError();
    }
    this.#status = "connected";
    const handshake = structuredClone(this.#negotiated);

    const lastSeq = cursor?.last_sequence ?? 0;
    const missedEvents = this.#events.filter((e) => e.sequence > lastSeq);

    if (missedEvents.length > 0) {
      const from = missedEvents[0]!.sequence;
      const through = missedEvents[missedEvents.length - 1]!.sequence;

      const replayedHeader: EventEnvelope = {
        protocol_version: handshake.selected_version,
        event_id: this.#mintId("evt"),
        operation_id: null,
        thread_id: null,
        turn_id: null,
        sequence: this.#nextSeq++ as Sequence,
        occurred_at: Date.now(),
        body: { kind: "stream.replayed", from, through },
      };

      for (const listener of this.#listeners) {
        listener(structuredClone(replayedHeader));
      }

      for (const missed of missedEvents) {
        for (const listener of this.#listeners) {
          listener(structuredClone(missed));
        }
      }
    }

    const readyEvent: EventEnvelope = {
      protocol_version: handshake.selected_version,
      event_id: this.#mintId("evt"),
      operation_id: null,
      thread_id: null,
      turn_id: null,
      sequence: this.#nextSeq++ as Sequence,
      occurred_at: Date.now(),
      body: { kind: "stream.ready", diagnostic: "stream caught up to live" },
    };

    for (const listener of this.#listeners) {
      listener(structuredClone(readyEvent));
    }

    return handshake;
  }

  close(): Promise<void> {
    this.#status = "closed";
    this.#listeners.clear();
    return Promise.resolve();
  }

  /**
   * Pushes a new event directly to all active subscribers and appends to log.
   */
  pushEvent(event: EventEnvelope): void {
    const cloned = structuredClone(event);
    this.#events.push(cloned);
    for (const listener of this.#listeners) {
      listener(structuredClone(cloned));
    }
  }

  /**
   * Helper to construct and emit an event envelope with auto-sequencing.
   */
  emit(body: EventBody, overrides?: Partial<EventEnvelope>): EventEnvelope {
    const event: EventEnvelope = {
      protocol_version: overrides?.protocol_version ?? this.#negotiated.selected_version,
      event_id: overrides?.event_id ?? this.#mintId("evt"),
      operation_id: overrides?.operation_id ?? null,
      thread_id: overrides?.thread_id ?? null,
      turn_id: overrides?.turn_id ?? null,
      sequence: overrides?.sequence ?? (this.#nextSeq++ as Sequence),
      occurred_at: overrides?.occurred_at ?? Date.now(),
      body,
    };
    this.pushEvent(event);
    return event;
  }

  #getThreadSummaries(): ThreadSummaryDto[] {
    return this.#threadFixtures.map((fixture) => {
      const matchingAgent =
        this.#agents.find((a) => a.display_name === fixture.agent || a.id === fixture.agent) ??
        this.#agents[0];
      const thread: ThreadDto = {
        id: fixture.id,
        agent_profile_id: matchingAgent?.id ?? "agp_fixture000000001",
        title: fixture.title,
        state: fixture.pinned ? "pinned" : "open",
        project_id: null,
        created_at: 1700000000000,
        updated_at: 1700000000000,
      };

      // Deterministic turn identities derived from the owning thread body,
      // staying inside the trn_ body rules (ADR 0019).
      const threadBody = fixture.id.slice("thr_".length);
      const lastTurn: TurnDto | null =
        fixture.rows.length > 0
          ? {
              id: `trn_${threadBody}ls`,
              thread_id: fixture.id,
              state: fixture.status === "running" ? "active" : "completed",
              delivery_state: "confirmed",
              operation_id: null,
              started_at: 1700000000000,
              ended_at: fixture.status === "running" ? null : BigInt(1700000001000),
            }
          : null;

      const activeTurn: TurnDto | null =
        fixture.status === "running"
          ? {
              id: `trn_${threadBody}ac`,
              thread_id: fixture.id,
              state: "active",
              delivery_state: "confirmed",
              operation_id: null,
              started_at: Date.now() - 5000,
              ended_at: null,
            }
          : null;

      return {
        thread,
        last_turn: lastTurn,
        active_turn: activeTurn,
      };
    });
  }

  #defaultCommandHandler(command: CommandEnvelope): unknown {
    const version = this.#negotiated.selected_version;

    switch (command.kind) {
      case "ping":
        return { status: "ok", timestamp: Date.now() };

      case "list_threads": {
        // Bounded pagination over the fixture summaries, mirroring the real
        // Core handler: `limit` rows after `cursor.thread_id`.
        const payload = command.payload as ListThreadsCommand | null;
        const limit = payload?.limit ?? 20;
        const summaries = this.#getThreadSummaries();
        let start = 0;
        const cursor = payload?.cursor;
        if (cursor?.thread_id) {
          const anchor = summaries.findIndex((s) => s.thread.id === cursor.thread_id);
          if (anchor >= 0) {
            start = anchor + 1;
          }
        }
        const page = summaries.slice(start, start + limit);
        const response: ThreadListResponseDto = {
          threads: page,
          next_cursor:
            page.length > 0
              ? {
                  updated_at: page[page.length - 1]!.thread.updated_at,
                  thread_id: page[page.length - 1]!.thread.id,
                }
              : null,
          has_more: start + limit < summaries.length,
        };
        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: null,
          as_of: Date.now(),
          data: response,
        };
        return snapshot;
      }

      case "search_threads": {
        const payload = command.payload as SearchThreadsCommand | null;
        const query = (payload?.query ?? "").toLowerCase().trim();
        const filtered = this.#getThreadSummaries().filter(
          (s) =>
            s.thread.title.toLowerCase().includes(query) ||
            s.thread.id.toLowerCase().includes(query),
        );
        const response: ThreadListResponseDto = {
          threads: filtered,
          next_cursor: filtered.length > 0
            ? {
                updated_at: filtered[filtered.length - 1]!.thread.updated_at,
                thread_id: filtered[filtered.length - 1]!.thread.id,
              }
            : null,
          has_more: false,
        };
        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: null,
          as_of: Date.now(),
          data: response,
        };
        return snapshot;
      }

      case "open_thread": {
        const payload = command.payload as OpenThreadCommand | null;
        const threadId = payload?.thread_id ?? this.#threadFixtures[0]?.id;
        const fixture =
          (threadId ? this.#threadFixtures.find((t) => t.id === threadId) : undefined) ??
          (threadId ? allThreads(true).find((t) => t.id === threadId) : undefined) ??
          this.#threadFixtures[0];

        const matchingAgent =
          this.#agents.find((a) => a.display_name === fixture?.agent || a.id === fixture?.agent) ??
          this.#agents[0];

        const threadDto: ThreadDto = {
          id: fixture?.id ?? threadId!,
          agent_profile_id: matchingAgent?.id ?? "agp_fixture000000001",
          title: fixture?.title ?? "Conversation",
          state: fixture?.pinned ? "pinned" : "open",
          project_id: null,
          created_at: 1700000000000,
          updated_at: 1700000000000,
        };

        const agentProfile = matchingAgent ?? null;

        const turns: TurnDto[] = (fixture?.rows ?? [])
          .filter((r) => r.kind === "user-message" || r.kind === "assistant-message")
          .map((r, idx) => ({
            id: r.id,
            thread_id: threadDto.id,
            state: "completed",
            delivery_state: "confirmed",
            operation_id: null,
            started_at: 1700000000000 + idx * 1000,
            ended_at: BigInt(1700000000000 + idx * 1000 + 500),
          }));

        const threadBody = threadDto.id.slice("thr_".length);
        const permissions: PermissionDto[] = (fixture?.rows ?? [])
          .filter((r) => r.kind === "permission")
          .map((r) => ({
            event_id: r.id,
            turn_id: `trn_${threadBody}pe`,
            thread_id: threadDto.id,
            kind: "execute",
            description: r.text,
            decision: r.permission?.decision ?? "pending",
            requested_at: 1700000000000,
            decided_at: r.permission?.decision ? BigInt(1700000001000) : null,
          }));

        const snapshotData: ThreadSnapshotDto = {
          thread: threadDto,
          agent_profile: agentProfile,
          turns,
          pending_permissions: permissions,
        };

        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: threadDto.id,
          as_of: Date.now(),
          data: snapshotData,
        };
        return snapshot;
      }

      case "get_history": {
        const payload = command.payload as GetHistoryCommand | null;
        const threadId = payload?.thread_id ?? "";
        const fixture =
          this.#threadFixtures.find((t) => t.id === threadId) ??
          allThreads(true).find((t) => t.id === threadId);
        const rows = fixture?.rows ?? [];

        const turns: TurnDto[] = rows
          .filter((r) => r.kind === "user-message" || r.kind === "assistant-message")
          .map((r, idx) => ({
            id: r.id,
            thread_id: threadId,
            state: "completed",
            delivery_state: "confirmed",
            operation_id: null,
            started_at: 1700000000000 + idx * 1000,
            ended_at: BigInt(1700000000000 + idx * 1000 + 500),
          }));

        // Project journal entries (ADR 0020, review A05). The fixture row id
        // IS that row's journal identity in the fixture world, so projected
        // entries keep it: user/unknown rows carry it as event_id, assistant
        // deltas carry it as turn_id (the reducer keys delta rows by turn).
        const allEntries: HistoryEntryDto[] = rows.map((r, idx) => {
          const seq = idx + 1;
          const occurred_at = 1700000000000 + idx * 1000;
          const turn_id = r.id.startsWith("trn_")
            ? r.id
            : `trn_hist${String(seq).padStart(12, "0")}`;

          if (r.kind === "user-message") {
            return {
              entry_kind: "user_message",
              event_id: r.id,
              turn_id,
              seq,
              text: r.text,
              occurred_at,
            };
          }
          if (r.kind === "permission") {
            return {
              entry_kind: "permission",
              event_id: r.id.startsWith("evt_") ? r.id : `evt_perm${String(seq).padStart(12, "0")}`,
              turn_id,
              seq,
              permission_kind: r.permission?.scope ?? "execute",
              description: r.text,
              decision: r.permission?.decision ?? "pending",
              occurred_at,
            };
          }
          if (r.kind === "unknown" || r.kind === "error") {
            return {
              entry_kind: "unknown",
              event_id: r.id,
              seq,
              kind: r.text.split(":")[0]?.trim() || "unknown.fact",
              diagnostic: r.text,
              occurred_at,
            };
          }
          return {
            entry_kind: "assistant_delta",
            event_id: `evt_asst${String(seq).padStart(12, "0")}`,
            turn_id: r.id,
            seq,
            text: r.text,
            occurred_at,
          };
        });

        const limit = Math.max(1, Math.min(payload?.limit ?? 50, 100));
        const beforeSeq = payload?.before_seq ?? null;

        const candidateEntries = beforeSeq != null
          ? allEntries.filter((e) => e.seq < beforeSeq)
          : allEntries;

        const pagedEntries = candidateEntries.slice(-limit);
        const hasOlder = candidateEntries.length > pagedEntries.length;
        const nextSeqCursor: HistoryCursorDto | null = hasOlder && pagedEntries.length > 0
          ? { seq: pagedEntries[0]!.seq }
          : null;
        const highWaterSeq = allEntries.length > 0 ? allEntries[allEntries.length - 1]!.seq : null;

        const response: ThreadHistoryResponseDto = {
          thread_id: threadId,
          turns,
          next_cursor: null,
          has_more: hasOlder,
          entries: pagedEntries,
          next_seq_cursor: nextSeqCursor,
          high_water_seq: highWaterSeq,
        };

        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: threadId,
          as_of: Date.now(),
          data: response,
        };
        return snapshot;
      }

      case "create_thread": {
        const payload = command.payload as CreateThreadCommand | null;
        const newId = this.#mintId("thr");
        const agentName =
          this.#agents.find((a) => a.id === payload?.agent_profile_id)?.display_name ??
          payload?.agent_profile_id ??
          "alpha (ACP)";

        const newFixture: ThreadFixture = {
          id: newId,
          title: payload?.title || "New conversation",
          agent: agentName,
          status: "running",
          pinned: false,
          rows: [],
        };
        this.#threadFixtures.unshift(newFixture);

        const newThreadDto: ThreadDto = {
          id: newId,
          agent_profile_id: payload?.agent_profile_id ?? this.#agents[0]?.id ?? "agp_fixture000000001",
          title: newFixture.title,
          state: "open",
          project_id: payload?.project_id ?? null,
          created_at: Date.now(),
          updated_at: Date.now(),
        };
        return newThreadDto;
      }

      case "configure_agent": {
        // Mirrors real Core (ADR 0019): a null agent_profile_id asks Core
        // to mint the identity, and the response carries the configured
        // profile and binding ids back to the client.
        const payload = command.payload as ConfigureAgentCommand | null;
        const agentId = payload?.agent_profile_id ?? this.#mintId("agp");

        const profile: AgentProfileDto = {
          id: agentId,
          display_name: payload?.display_name ?? "Custom Agent",
          preferred_harness: payload?.preferred_harness ?? "acp",
          memory_mode: payload?.memory_mode ?? "session",
          created_at: Date.now(),
          updated_at: Date.now(),
        };

        const existingIdx = this.#agents.findIndex((a) => a.id === agentId);
        if (existingIdx >= 0) {
          this.#agents[existingIdx] = profile;
        } else {
          this.#agents.push(profile);
        }

        let bindingId: string | null = null;
        if (payload?.binding) {
          const bindingConfig: HarnessBindingConfigDto = payload.binding;
          bindingId = bindingConfig.harness_binding_id ?? this.#mintId("hsb");
          const bindingDto: HarnessBindingDto = {
            id: bindingId,
            agent_profile_id: bindingConfig.agent_profile_id ?? agentId,
            program: bindingConfig.program,
            args: [...bindingConfig.args],
            env_keys: [...bindingConfig.env_keys],
            secret_refs: [...bindingConfig.secret_refs],
            label: bindingConfig.label ?? profile.display_name,
            created_at: Date.now(),
          };
          this.#bindings.set(bindingId, bindingDto);
        }

        return {
          agent_profile_id: agentId,
          harness_binding_id: bindingId,
        };
      }

      case "test_harness_binding": {
        const payload = command.payload as TestHarnessBindingCommand | null;
        if (!payload?.program && !payload?.harness_binding_id) {
          throw new Error("Missing required harness binding program executable");
        }
        const probedBindingId = payload?.harness_binding_id ?? this.#mintId("hsb");
        return {
          ok: true,
          capabilities: {
            "session.update": "supported",
            "thread.streaming": "supported",
          },
          diagnostics: null,
          probed_binding_id: probedBindingId,
        };
      }

      case "start_turn": {
        const payload = command.payload as StartTurnCommand | null;
        const threadId = payload?.thread_id ?? "";
        const turnId = payload?.turn_id ?? this.#mintId("trn");

        if (this.#autoStreamReplies) {
          this.emit(
            { kind: "turn.started" },
            { thread_id: threadId, turn_id: turnId, operation_id: command.operation_id },
          );

          // Async streaming reply delivery
          queueMicrotask(() => {
            for (const chunk of streamingReplyChunks) {
              this.emit(
                { kind: "message.delta", text: chunk },
                { thread_id: threadId, turn_id: turnId, operation_id: command.operation_id },
              );
            }
            this.emit(
              { kind: "turn.completed" },
              { thread_id: threadId, turn_id: turnId, operation_id: command.operation_id },
            );
          });
        }

        return { admission: "admitted", turn_id: turnId };
      }

      case "respond_permission": {
        return { ok: true };
      }

      case "cancel_turn":
      case "cancel": {
        const payload = command.payload as CancelTurnCommand | null;
        this.emit(
          { kind: "turn.cancelled", reason: "turn cancelled by user" },
          {
            thread_id: payload?.thread_id ?? null,
            turn_id: payload?.turn_id ?? null,
            operation_id: command.operation_id,
          },
        );
        return { cancelled: true };
      }

      case "runtime_status": {
        const statusEvent = this.emit(
          {
            kind: "runtime.status",
            status: "ready",
            active_threads: this.#threadFixtures.length,
            diagnostics: null,
          },
          { operation_id: command.operation_id },
        );
        return statusEvent.body;
      }

      case "diagnostics": {
        const payload = command.payload as DiagnosticsCommand | null;
        const diagDto: RuntimeDiagnosticsDto = {
          instance_id: "cor_fixture000000001",
          status: "ready",
          active_threads: this.#threadFixtures.length,
          active_turns: 0,
          summary: null,
        };
        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: payload?.thread_id ?? null,
          as_of: Date.now(),
          data: diagDto,
        };
        return snapshot;
      }

      case "request_snapshot": {
        const diagDto: RuntimeDiagnosticsDto = {
          instance_id: "cor_fixture000000001",
          status: "ready",
          active_threads: this.#threadFixtures.length,
          active_turns: 0,
          summary: null,
        };
        const snapshot: SnapshotEnvelope = {
          protocol_version: version,
          operation_id: command.operation_id,
          thread_id: null,
          as_of: Date.now(),
          data: diagDto,
        };
        return snapshot;
      }

      case "get_context_snapshot": {
        const payload = command.payload as GetContextSnapshotCommand | null;
        const turnId = payload?.turn_id;
        const threadId = payload?.thread_id;
        if (turnId && this.#contextSnapshots.has(turnId)) {
          return structuredClone(this.#contextSnapshots.get(turnId)!);
        }
        for (const snap of this.#contextSnapshots.values()) {
          if ((threadId && snap.thread_id === threadId) || (turnId && snap.turn_id === turnId)) {
            return structuredClone(snap);
          }
        }
        const fallbackSnap: ContextSnapshotDto = {
          turn_id: turnId ?? "trn_fixture000000002",
          thread_id: threadId ?? "thr_fixture000000001",
          memory_mode: "long_term",
          created_at: Date.now(),
          passthrough: false,
          budget: {
            identity_limit_tokens: 1024,
            memory_limit_tokens: 2048,
            prompt_tokens: 120,
            identity_tokens: 250,
            memory_tokens: 680,
            total_tokens: 1050,
          },
          identity: [
            {
              document_id: "idd_fixture000000001",
              kind: "about",
              tokens: 250,
            },
          ],
          memories: [
            {
              memory_id: "mem_fixture000000001",
              kind: "preference",
              scope_kind: "project",
              scope_target: null,
              confidence: 95,
              explicit: true,
              tokens: 180,
              score: 0.92,
              why_selected: 'matched terms: ["test", "vitest"], bm25: 2.140, total: 0.9200',
              provenance_thread_id: "thr_fixture000000003",
              provenance_turn_id: "trn_fixture000000004",
            },
          ],
          dropped: [
            {
              memory_id: "mem_low_rank",
              tokens: 300,
              rank: 5,
              reason: "budget_exhausted",
            },
          ],
          degraded: null,
          rendered_prompt: "Rendered prompt content with injected memories...",
        };
        return fallbackSnap;
      }

      case "list_identity_documents": {
        const payload = command.payload as ListIdentityDocumentsCommand | null;
        const limit = payload?.limit ?? 32;
        return this.#identityDocs.slice(0, limit).map((d) => structuredClone(d));
      }

      case "put_identity_document": {
        const payload = command.payload as PutIdentityDocumentCommand | null;
        const docId = payload?.document_id ?? this.#mintId("idd");
        const now = Date.now();
        const doc: IdentityDocumentDto = {
          document_id: docId,
          kind: payload?.kind ?? "about",
          content: payload?.content ?? "",
          created_at: now,
          updated_at: now,
        };
        const existingIdx = this.#identityDocs.findIndex((d) => d.document_id === docId);
        if (existingIdx >= 0) {
          doc.created_at = this.#identityDocs[existingIdx]!.created_at;
          this.#identityDocs[existingIdx] = doc;
        } else {
          this.#identityDocs.push(doc);
        }
        return structuredClone(doc);
      }

              case "delete_identity_document": {
        const payload = command.payload as DeleteIdentityDocumentCommand | null;
        if (payload?.document_id) {
          this.#identityDocs = this.#identityDocs.filter(
            (d) => d.document_id !== payload.document_id,
          );
        }
        return { ok: true };
      }

      case "list_memories": {
        const payload = command.payload as ListMemoriesCommand | null;
        const limit = payload?.limit ?? 50;
        let filtered = [...this.#memories];
        if (payload?.scope_kind) {
          filtered = filtered.filter((m) => m.scope_kind === payload.scope_kind);
          if (payload.scope_kind !== "global" && payload.scope_target) {
            filtered = filtered.filter((m) => m.scope_target === payload.scope_target);
          }
        }
        if (payload?.state) {
          filtered = filtered.filter((m) => m.state === payload.state);
        }
        filtered.sort((a, b) => b.updated_at - a.updated_at || b.memory_id.localeCompare(a.memory_id));
        let start = 0;
        if (payload?.cursor) {
          const cur = payload.cursor;
          const anchor = filtered.findIndex(
            (m) => m.updated_at === cur.updated_at && m.memory_id === cur.memory_id,
          );
          if (anchor >= 0) {
            start = anchor + 1;
          }
        }
        const page = filtered.slice(start, start + limit);
        const has_more = start + limit < filtered.length;
        const next_cursor =
          has_more && page.length > 0
            ? {
                updated_at: page[page.length - 1]!.updated_at,
                memory_id: page[page.length - 1]!.memory_id,
              }
            : null;
        const response: MemoryListResponseDto = {
          memories: page.map((m) => structuredClone(m)),
          next_cursor,
          has_more,
        };
        return response;
      }

      case "propose_memory": {
        const payload = command.payload as ProposeMemoryCommand | null;
        if (!payload) throw new Error("Missing propose_memory payload");
        if (isSecretShaped(payload.content) || (payload.excerpt && isSecretShaped(payload.excerpt))) {
          throw new Error("propose_memory failed: secret-shaped content rejected");
        }
        const now = Date.now();
        const isExplicit = payload.source === "explicit";
        const memoryId = this.#mintId("mem");
        const record: MemoryRecordDto = {
          memory_id: memoryId,
          content: payload.content,
          scope_kind: payload.scope_kind,
          scope_target: payload.scope_target ?? null,
          kind: payload.kind,
          state: isExplicit ? "confirmed" : "candidate",
          confidence: payload.confidence ?? (isExplicit ? 100 : 80),
          sensitivity: payload.sensitivity ?? "normal",
          source: payload.source ?? (isExplicit ? "explicit" : "inferred"),
          explicit: isExplicit,
          provenance_thread_id: payload.thread_id ?? null,
          provenance_turn_id: payload.turn_id ?? null,
          excerpt: payload.excerpt ?? null,
          created_at: now,
          updated_at: now,
          expires_at: null,
          superseded_by: null,
        };
        this.#memories.push(record);
        return structuredClone(record);
      }

      case "confirm_memory": {
        const payload = command.payload as ConfirmMemoryCommand | null;
        if (!payload) throw new Error("Missing confirm_memory payload");
        const existing = this.#memories.find((m) => m.memory_id === payload.memory_id);
        if (!existing) {
          throw new Error("Memory not found: " + payload.memory_id);
        }
        existing.state = "confirmed";
        existing.updated_at = Date.now();
        return structuredClone(existing);
      }

      case "reject_memory": {
        const payload = command.payload as RejectMemoryCommand | null;
        if (!payload) throw new Error("Missing reject_memory payload");
        const existing = this.#memories.find((m) => m.memory_id === payload.memory_id);
        if (!existing) {
          throw new Error("Memory not found: " + payload.memory_id);
        }
        existing.state = "rejected";
        existing.updated_at = Date.now();
        return structuredClone(existing);
      }

      case "correct_memory": {
        const payload = command.payload as CorrectMemoryCommand | null;
        if (!payload) throw new Error("Missing correct_memory payload");
        if (isSecretShaped(payload.content)) {
          throw new Error("correct_memory failed: secret-shaped content rejected");
        }
        const existing = this.#memories.find((m) => m.memory_id === payload.memory_id);
        if (!existing) {
          throw new Error("Memory not found: " + payload.memory_id);
        }
        const now = Date.now();
        const newId = this.#mintId("mem");
        existing.state = "superseded";
        existing.superseded_by = newId;
        existing.updated_at = now;

        const corrected: MemoryRecordDto = {
          memory_id: newId,
          content: payload.content,
          scope_kind: payload.scope_kind ?? existing.scope_kind,
          scope_target: payload.scope_target !== undefined ? payload.scope_target : existing.scope_target,
          kind: payload.kind ?? existing.kind,
          state: "confirmed",
          confidence: 100,
          sensitivity: payload.sensitivity ?? existing.sensitivity,
          source: "explicit",
          explicit: true,
          provenance_thread_id: existing.provenance_thread_id,
          provenance_turn_id: existing.provenance_turn_id,
          excerpt: existing.excerpt,
          created_at: now,
          updated_at: now,
          expires_at: existing.expires_at,
          superseded_by: null,
        };
        this.#memories.push(corrected);
        return structuredClone(corrected);
      }

      case "forget_memory": {
        const payload = command.payload as ForgetMemoryCommand | null;
        if (!payload) throw new Error("Missing forget_memory payload");
        const existing = this.#memories.find((m) => m.memory_id === payload.memory_id);
        if (!existing) {
          throw new Error("Memory not found: " + payload.memory_id);
        }
        existing.state = "forgotten";
        existing.updated_at = Date.now();
        return structuredClone(existing);
      }

      default:
        return { ok: true };
    }
  }
}
