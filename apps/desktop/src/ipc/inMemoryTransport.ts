import type { AgentProfileDto } from "./dto/AgentProfileDto";
import type { CancelTurnCommand } from "./dto/CancelTurnCommand";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "./dto/ConfigureAgentCommand";
import type { CreateThreadCommand } from "./dto/CreateThreadCommand";
import type { DiagnosticsCommand } from "./dto/DiagnosticsCommand";
import type { EventBody } from "./dto/EventBody";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { GetHistoryCommand } from "./dto/GetHistoryCommand";
import type { HarnessBindingConfigDto } from "./dto/HarnessBindingConfigDto";
import type { HarnessBindingDto } from "./dto/HarnessBindingDto";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";
import type { OpenThreadCommand } from "./dto/OpenThreadCommand";
import type { PermissionDto } from "./dto/PermissionDto";
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
import { ConnectionClosedError } from "./errors";
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
}

const DEFAULT_AGENTS_DTO: AgentProfileDto[] = [
  {
    id: "agent-alpha",
    display_name: "alpha (ACP)",
    preferred_harness: "acp",
    memory_mode: "session",
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "agent-beta",
    display_name: "beta (ACP)",
    preferred_harness: "acp",
    memory_mode: "session",
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
];

const DEFAULT_BINDINGS_DTO: HarnessBindingDto[] = [
  {
    id: "bin_alpha_01",
    agent_profile_id: "agent-alpha",
    program: "/usr/local/bin/acp-alpha",
    args: ["--mode", "server"],
    env_keys: ["ANTHROPIC_API_KEY"],
    secret_refs: ["vault://acp-alpha"],
    label: "Alpha ACP",
    created_at: 1700000000000,
  },
  {
    id: "bin_beta_01",
    agent_profile_id: "agent-beta",
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
export class InMemoryTransport implements CoreTransport {
  readonly id = "in-memory";
  readonly #negotiated: NegotiatedHandshake;
  #events: EventEnvelope[];
  readonly #listeners = new Set<(event: EventEnvelope) => void>();
  readonly #sent: CommandEnvelope[] = [];
  #status: TransportStatus;
  #commandHandler?: (command: CommandEnvelope) => Promise<unknown> | unknown;
  #nextSeq: number;
  #threadSeq = 0;
  #bindingSeq = 0;
  readonly #autoStreamReplies: boolean;

  #threadFixtures: ThreadFixture[];
  #agents: AgentProfileDto[];
  #bindings = new Map<string, HarnessBindingDto>();

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
    const cloned = structuredClone(envelope);
    this.#sent.push(cloned);

    if (this.#commandHandler) {
      const result = await this.#commandHandler(cloned);
      return result as T;
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
        event_id: `evt_replay_${Date.now()}_${from}`,
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
      event_id: `evt_ready_${Date.now()}`,
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
      event_id: overrides?.event_id ?? `evt_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
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
        agent_profile_id: matchingAgent?.id ?? "agent-alpha",
        title: fixture.title,
        state: fixture.pinned ? "pinned" : "open",
        project_id: null,
        created_at: 1700000000000,
        updated_at: 1700000000000,
      };

      const lastTurn: TurnDto | null =
        fixture.rows.length > 0
          ? {
              id: `trn_${fixture.id}_last`,
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
              id: `trn_${fixture.id}_active`,
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
        const summaries = this.#getThreadSummaries();
        const response: ThreadListResponseDto = {
          threads: summaries,
          next_cursor: summaries.length > 0
            ? {
                updated_at: summaries[summaries.length - 1]!.thread.updated_at,
                thread_id: summaries[summaries.length - 1]!.thread.id,
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
        const threadId = payload?.thread_id ?? this.#threadFixtures[0]?.id ?? "thread-1";
        const fixture =
          this.#threadFixtures.find((t) => t.id === threadId) ??
          allThreads(true).find((t) => t.id === threadId) ??
          this.#threadFixtures[0];

        const matchingAgent =
          this.#agents.find((a) => a.display_name === fixture?.agent || a.id === fixture?.agent) ??
          this.#agents[0];

        const threadDto: ThreadDto = {
          id: fixture?.id ?? threadId,
          agent_profile_id: matchingAgent?.id ?? "agent-alpha",
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

        const permissions: PermissionDto[] = (fixture?.rows ?? [])
          .filter((r) => r.kind === "permission")
          .map((r) => ({
            event_id: r.id,
            turn_id: `trn_${threadDto.id}`,
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
        const turns: TurnDto[] = (fixture?.rows ?? [])
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

        const response: ThreadHistoryResponseDto = {
          thread_id: threadId,
          turns,
          next_cursor: null,
          has_more: false,
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
        const newId = `thread-${++this.#threadSeq}_${Date.now()}`;
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
          agent_profile_id: payload?.agent_profile_id ?? this.#agents[0]?.id ?? "agent-alpha",
          title: newFixture.title,
          state: "open",
          project_id: payload?.project_id ?? null,
          created_at: Date.now(),
          updated_at: Date.now(),
        };
        return newThreadDto;
      }

      case "configure_agent": {
        const payload = command.payload as ConfigureAgentCommand | null;
        const agentId =
          payload?.agent_profile_id ??
          `agent-${(payload?.display_name ?? "agent").toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${Date.now().toString(36)}`;

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

        let warning: string | null = null;
        let bindingDto: HarnessBindingDto | null = null;
        if (payload?.binding) {
          const bindingConfig: HarnessBindingConfigDto = payload.binding;
          const bindingId =
            bindingConfig.harness_binding_id ?? `bin_${++this.#bindingSeq}_${Date.now().toString(36)}`;
          bindingDto = {
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
        } else {
          warning = "Legacy configuration without harness binding";
        }

        return { ok: true, profile, binding: bindingDto, warning };
      }

      case "test_harness_binding": {
        const payload = command.payload as TestHarnessBindingCommand | null;
        if (!payload?.program && !payload?.harness_binding_id) {
          throw new Error("Missing required harness binding program executable");
        }
        return {
          ok: true,
          diagnostics: null,
          probed_binding_id: payload?.harness_binding_id ?? null,
        };
      }

      case "start_turn": {
        const payload = command.payload as StartTurnCommand | null;
        const threadId = payload?.thread_id ?? "";
        const turnId = payload?.turn_id ?? `turn-${Date.now()}`;

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

        return { admission: "admitted" };
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
          instance_id: "core-mock-instance",
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
          instance_id: "core-mock-instance",
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

      default:
        return { ok: true };
    }
  }
}
