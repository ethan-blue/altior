import { describe, expect, it } from "vitest";
import { approvalThread, failureThread, standardThread } from "../fixtures/timeline";
import { validateCommandEnvelope } from "../ipc/commandContract";
import type { CommandEnvelope } from "../ipc/dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "../ipc/dto/ConfigureAgentCommand";
import type { CreateThreadCommand } from "../ipc/dto/CreateThreadCommand";
import type { EventEnvelope } from "../ipc/dto/EventEnvelope";
import type { OpenThreadCommand } from "../ipc/dto/OpenThreadCommand";
import type { RespondPermissionCommand } from "../ipc/dto/RespondPermissionCommand";
import type { Sequence } from "../ipc/dto/Sequence";
import type { StartTurnCommand } from "../ipc/dto/StartTurnCommand";
import type { TestHarnessBindingCommand } from "../ipc/dto/TestHarnessBindingCommand";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { ConnectionClosedError, InvalidCommandError } from "../ipc/errors";
import { createOperationId, isValidOperationId } from "../ipc/operationId";
import {
  createApplicationStore,
  sanitizeSecretRef,
} from "./applicationStore";

describe("ApplicationStore", () => {
  it("initializes transport connection and loads threads and runtime_status", async () => {
    const transport = new InMemoryTransport();
    const store = createApplicationStore(transport);

    await store.init();

    const state = store.getState();
    expect(state.connectionStatus).toBe("connected");
    expect(state.negotiated?.selected_version).toBeGreaterThanOrEqual(1);
    expect(state.threads.length).toBeGreaterThanOrEqual(3);
    expect(state.selectedThreadId).toBe(standardThread.id);

    // Verify list_threads and runtime_status commands were sent
    const sentKinds = transport.sentCommands.map((c) => c.kind);
    expect(sentKinds).toContain("list_threads");
    expect(sentKinds).toContain("runtime_status");
  });

  describe("Agent Onboarding & Testing with Opaque Secret References", () => {
    it("accepts valid opaque secret references and rejects plaintext secrets", () => {
      expect(sanitizeSecretRef("sec_anthropic_vault_01")).toBe("sec_anthropic_vault_01");
      expect(sanitizeSecretRef("vault://key-123")).toBe("vault://key-123");
      expect(sanitizeSecretRef("env:ANTHROPIC_API_KEY")).toBe("env:ANTHROPIC_API_KEY");
      expect(sanitizeSecretRef("ref:secret-456")).toBe("ref:secret-456");

      // Plaintext API key is rejected with an explicit error (F27 / user rules)
      expect(() => sanitizeSecretRef("sk-proj-1234567890abcdef")).toThrow(
        /credentials must be stored in the OS secret store/i,
      );
    });

    it("canary: plaintext secrets are rejected up front and never enter stream log, state, or command payloads", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const canarySecret = "SUPER-SECRET-PLAINTEXT-KEY-CANARY-12345";

      await expect(
        store.onboardAgent({
          name: "Canary Agent",
          provider: "acp",
          model: "claude-3-7-sonnet",
          envKeys: ["ANTHROPIC_API_KEY"],
          secretRef: canarySecret,
        }),
      ).rejects.toThrow(/credentials must be stored in the OS secret store/i);

      const testResult = await store.testAgent({
        provider: "acp",
        model: "claude-3-7-sonnet",
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: canarySecret,
      });
      expect(testResult.success).toBe(false);

      // 1. Inspect state
      const stateStr = JSON.stringify(store.getState());
      expect(stateStr).not.toContain(canarySecret);

      // 2. Inspect streamLog
      const streamLogStr = JSON.stringify(store.getState().streamLog);
      expect(streamLogStr).not.toContain(canarySecret);

      // 3. Inspect sentCommands
      const sentCmdsStr = JSON.stringify(transport.sentCommands);
      expect(sentCmdsStr).not.toContain(canarySecret);
    });

    it("onboards a new agent: Core mints the identity and the store adopts it", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const newAgent = await store.onboardAgent({
        name: "Custom Agent",
        provider: "acp",
        model: "claude-3-7-sonnet",
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://custom-key",
      });

      // The agent identity is Core-minted (ADR 0019), not a client slug.
      expect(newAgent.id).toMatch(/^agp_[0-9a-z]{16,64}$/);

      const state = store.getState();
      expect(state.agents.some((a) => a.id === newAgent.id)).toBe(true);
      expect(state.selectedAgentId).toBe(newAgent.id);
      expect(newAgent.bindingId).toMatch(/^hsb_[0-9a-z]{16,64}$/);
      expect(transport.agents.some((a) => a.id === newAgent.id)).toBe(true);

      // Verify configure_agent command envelope: null identities ask Core
      // to allocate, and the command itself is contract-valid.
      const configCmd = transport.sentCommands.find((c) => c.kind === "configure_agent");
      expect(configCmd).toBeDefined();
      expect(validateCommandEnvelope(configCmd!)).toBeNull();
      const payload = configCmd?.payload as ConfigureAgentCommand;
      expect(payload.display_name).toBe("Custom Agent");
      expect(payload.preferred_harness).toBe("acp");
      expect(payload.agent_profile_id).toBeNull();
      expect(payload.binding?.harness_binding_id).toBeNull();
      expect(payload.binding?.agent_profile_id).toBeNull();
    });

    it("tests agent connection via test_harness_binding command with secret_refs", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const result = await store.testAgent({
        provider: "/path/to/acp-binary",
        model: "claude-3-7-sonnet",
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://test-key",
      });

      expect(result.success).toBe(true);
      expect(result.latencyMs).toBeGreaterThan(0);
      expect(store.getState().onboardingStatus.testResult?.success).toBe(true);

      const testCmd = transport.sentCommands.find((c) => c.kind === "test_harness_binding");
      expect(testCmd).toBeDefined();
      expect(validateCommandEnvelope(testCmd!)).toBeNull();
      const payload = testCmd?.payload as TestHarnessBindingCommand;
      expect(payload.program).toBe("/path/to/acp-binary");
      expect(payload.secret_refs).toEqual(["vault://test-key"]);
      expect(payload.harness_binding_id).toBeNull();

      // Core returns the binding identity it probed with (ADR 0019).
      expect(store.getState().onboardingStatus.bindingId).toMatch(/^hsb_[0-9a-z]{16,64}$/);
    });

    it("configures dual agents sequentially and retains the tested binding identity", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // 1. Probe Agent A with test_harness_binding (one env key per ref —
      // the protocol requires env_keys.length === secret_refs.length; the
      // multi-key mapping UX is task A07).
      const testResultA = await store.testAgent({
        provider: "acp",
        model: "claude-3-7-sonnet",
        program: "/opt/bin/agent-alpha",
        args: ["--mode", "server", "--port", "8001"],
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://sec-alpha-01",
        label: "Alpha Primary Binding",
      });
      expect(testResultA.success).toBe(true);

      const testCmdA = transport.sentCommands.find((c) => c.kind === "test_harness_binding");
      expect(testCmdA).toBeDefined();
      expect(validateCommandEnvelope(testCmdA!)).toBeNull();
      const testPayloadA = testCmdA?.payload as TestHarnessBindingCommand;
      expect(testPayloadA.program).toBe("/opt/bin/agent-alpha");
      expect(testPayloadA.args).toEqual(["--mode", "server", "--port", "8001"]);
      expect(testPayloadA.env_keys).toEqual(["ANTHROPIC_API_KEY"]);
      expect(testPayloadA.secret_refs).toEqual(["vault://sec-alpha-01"]);
      expect(testPayloadA.label).toBe("Alpha Primary Binding");
      const probedBindingIdA = store.getState().onboardingStatus.bindingId;
      expect(probedBindingIdA).toMatch(/^hsb_[0-9a-z]{16,64}$/);

      // Save Agent A -> should use the exact same Core-probed binding ID
      const agentA = await store.onboardAgent({
        name: "Agent Alpha Custom",
        provider: "acp",
        model: "claude-3-7-sonnet",
        program: "/opt/bin/agent-alpha",
        args: ["--mode", "server", "--port", "8001"],
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://sec-alpha-01",
        label: "Alpha Primary Binding",
      });

      const configCmdA = transport.sentCommands.find((c) => c.kind === "configure_agent");
      expect(configCmdA).toBeDefined();
      const configPayloadA = configCmdA?.payload as ConfigureAgentCommand;
      expect(configPayloadA.display_name).toBe("Agent Alpha Custom");
      expect(configPayloadA.preferred_harness).toBe("acp");
      expect(configPayloadA.binding).toBeDefined();
      expect(configPayloadA.binding?.harness_binding_id).toBe(probedBindingIdA);
      expect(configPayloadA.binding?.program).toBe("/opt/bin/agent-alpha");
      expect(configPayloadA.binding?.args).toEqual(["--mode", "server", "--port", "8001"]);
      expect(configPayloadA.binding?.env_keys).toEqual(["ANTHROPIC_API_KEY"]);
      expect(configPayloadA.binding?.secret_refs).toEqual(["vault://sec-alpha-01"]);
      expect(configPayloadA.binding?.label).toBe("Alpha Primary Binding");

      // 2. Configure Agent B (terminal harness)
      const agentB = await store.onboardAgent({
        name: "Agent Beta Custom",
        provider: "terminal",
        model: "gpt-4o",
        program: "/opt/bin/agent-beta",
        args: ["--interactive"],
        envKeys: ["OPENAI_API_KEY"],
        secretRef: "env:OPENAI_API_KEY",
        label: "Beta Terminal Binding",
      });

      const configCmds = transport.sentCommands.filter((c) => c.kind === "configure_agent");
      expect(configCmds).toHaveLength(2);
      const configPayloadB = configCmds[1]?.payload as ConfigureAgentCommand;
      expect(configPayloadB.display_name).toBe("Agent Beta Custom");
      expect(configPayloadB.preferred_harness).toBe("terminal");
      expect(configPayloadB.binding?.program).toBe("/opt/bin/agent-beta");
      expect(configPayloadB.binding?.args).toEqual(["--interactive"]);
      expect(configPayloadB.binding?.secret_refs).toEqual(["env:OPENAI_API_KEY"]);

      // Verify both agents exist in application store state with Core-minted ids
      const state = store.getState();
      expect(state.agents.some((a) => a.id === agentA.id)).toBe(true);
      expect(state.agents.some((a) => a.id === agentB.id)).toBe(true);
      expect(state.selectedAgentId).toBe(agentB.id);

      // Verify transport in-memory bindings map has both bindings
      expect(transport.bindings.has(agentA.bindingId!)).toBe(true);
      expect(transport.bindings.has(agentB.bindingId!)).toBe(true);
    });

    it("configures an agent without binding and mirrors the real Core result shape", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const configureEnvelope: CommandEnvelope = {
        protocol_version: 1,
        operation_id: createOperationId(),
        kind: "configure_agent",
        payload: {
          agent_profile_id: null,
          display_name: "Legacy Agent",
          preferred_harness: "acp",
          memory_mode: "session",
          binding: null,
        } as ConfigureAgentCommand,
        issued_at: Date.now(),
      };

      // The fixture transport answers exactly like real Core (ADR 0019):
      // result data carries the configured profile id and no binding id.
      const res = await transport.command<{ agent_profile_id: string; harness_binding_id: string | null }>(
        configureEnvelope,
      );
      expect(res.agent_profile_id).toMatch(/^agp_[0-9a-z]{16,64}$/);
      expect(res.harness_binding_id).toBeNull();
      expect(transport.agents.some((a) => a.id === res.agent_profile_id)).toBe(true);
    });

    it("displays error notice when testing without required binding program", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const result = await store.testAgent({
        provider: "",
        program: "",
        model: "claude-3-7-sonnet",
      });

      expect(result.success).toBe(false);
      expect(result.error).toContain("Harness binding program is required");
      expect(store.getState().error).toContain("[INVALID_BINDING]");
      expect(store.getState().onboardingStatus.testResult?.success).toBe(false);
    });

    it("shows onboarding modal when initialized on a clean profile with 0 agents", async () => {
      const transport = new InMemoryTransport({ initialAgents: [], initialThreads: [] });
      const store = createApplicationStore(transport);

      await store.init();

      expect(store.getState().agents).toHaveLength(0);
      expect(store.getState().isOnboardingOpen).toBe(true);
    });
  });

  describe("Thread List, Creation, Search & Navigation", () => {
    it("creates a new thread using create_thread command and switches to it", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const thread = await store.createThread("Spike Investigation", "agp_fixture000000002");
      expect(thread.title).toBe("Spike Investigation");
      // The thread identity is Core-minted (ADR 0019).
      expect(thread.id).toMatch(/^thr_[0-9a-z]{16,64}$/);

      const state = store.getState();
      expect(state.threads.some((t) => t.id === thread.id)).toBe(true);
      expect(state.selectedThreadId).toBe(thread.id);

      const createCmd = transport.sentCommands.find((c) => c.kind === "create_thread");
      expect(createCmd).toBeDefined();
      expect(validateCommandEnvelope(createCmd!)).toBeNull();
      const payload = createCmd?.payload as CreateThreadCommand;
      expect(payload.title).toBe("Spike Investigation");
      expect(payload.agent_profile_id).toBe("agp_fixture000000002");

      // Verify open_thread was sent for the new thread
      const openCmds = transport.sentCommands.filter((c) => c.kind === "open_thread");
      expect(openCmds.some((c) => (c.payload as OpenThreadCommand).thread_id === thread.id)).toBe(true);
    });

    it("adds no fabricated thread when create_thread is rejected", async () => {
      const transport = new InMemoryTransport();
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "create_thread") {
          throw new Error("Core rejected thread creation");
        }
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();

      const before = store.getState().threads.length;
      const selectedBefore = store.getState().selectedThreadId;

      await expect(store.createThread("Ghost thread", "agp_fixture000000001")).rejects.toThrow(
        "Core rejected thread creation",
      );

      const state = store.getState();
      expect(state.threads).toHaveLength(before);
      expect(state.threads.some((t) => t.title === "Ghost thread")).toBe(false);
      expect(state.selectedThreadId).toBe(selectedBefore);
    });

    it("selects thread and opens it via open_thread command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      await store.selectThread(approvalThread.id);
      expect(store.getState().selectedThreadId).toBe(approvalThread.id);

      const openCmd = transport.sentCommands.find(
        (c) => c.kind === "open_thread" && (c.payload as OpenThreadCommand).thread_id === approvalThread.id,
      );
      expect(openCmd).toBeDefined();
    });

    it("filters threads via search_threads command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      await store.setThreadFilter("audit");
      expect(store.getState().threadFilter).toBe("audit");

      const searchCmd = transport.sentCommands.find((c) => c.kind === "search_threads");
      expect(searchCmd).toBeDefined();
    });

    it("fetches history via get_history command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      await store.getHistory(standardThread.id, 20);

      const histCmd = transport.sentCommands.find((c) => c.kind === "get_history");
      expect(histCmd).toBeDefined();
    });

    it("queries runtime diagnostics via diagnostics command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const diag = await store.getDiagnostics();
      expect(diag).not.toBeNull();
      expect(diag?.status).toBe("ready");

      const diagCmd = transport.sentCommands.find((c) => c.kind === "diagnostics");
      expect(diagCmd).toBeDefined();
    });
  });

  describe("Prompt Streaming & Turn Cancellation", () => {
    it("dispatches prompt with a Core-allocated turn id and streams deltas to the reply row", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();

      await store.sendPrompt("Explain length-prefixed framing");

      const state = store.getState();
      expect(state.activeTurns).toHaveLength(1);
      expect(state.activeTurns[0]?.isStreaming).toBe(true);
      // The response-resolved turn identity is Core-minted (ADR 0019).
      expect(state.activeTurns[0]?.turnId).toMatch(/^trn_[0-9a-z]{16,64}$/);

      const startCmd = transport.sentCommands.find((c) => c.kind === "start_turn");
      expect(startCmd).toBeDefined();
      expect(validateCommandEnvelope(startCmd!)).toBeNull();
      const startPayload = startCmd?.payload as StartTurnCommand;
      expect(startPayload.prompt).toBe("Explain length-prefixed framing");
      expect(startPayload.turn_id).toBeNull();

      const threadStore = store.getTimelineStore(state.selectedThreadId);
      const rows = threadStore.getSnapshot().rows;
      const userRow = rows.find((r) => r.id === "send-1");
      const replyRow = rows.find((r) => r.id === "send-1-reply");

      expect(userRow?.text).toBe("Explain length-prefixed framing");
      expect(replyRow?.streaming).toBe(true);
      expect(replyRow?.text).toBe("");

      // Dispatch delta event from transport
      const deltaEvent: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_fixture000000201",
        operation_id: "op_fixture000000210",
        thread_id: state.selectedThreadId,
        turn_id: state.activeTurns[0]?.turnId ?? null,
        sequence: 10 as any,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "Frames have 4-byte headers." },
      };
      store._handleEvent(deltaEvent);

      expect(threadStore.getRow("send-1-reply")?.text).toBe("Frames have 4-byte headers.");

      // Second delta
      const deltaEvent2: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_fixture000000202",
        operation_id: "op_fixture000000210",
        thread_id: state.selectedThreadId,
        turn_id: state.activeTurns[0]?.turnId ?? null,
        sequence: 11 as any,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: " Max payload is 256 KiB." },
      };
      store._handleEvent(deltaEvent2);

      expect(threadStore.getRow("send-1-reply")?.text).toBe(
        "Frames have 4-byte headers. Max payload is 256 KiB.",
      );

      // Complete turn
      const completeEvent: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_fixture000000203",
        operation_id: "op_fixture000000210",
        thread_id: state.selectedThreadId,
        turn_id: state.activeTurns[0]?.turnId ?? null,
        sequence: 12 as any,
        occurred_at: Date.now(),
        body: { kind: "turn.completed" },
      };
      store._handleEvent(completeEvent);

      expect(threadStore.getRow("send-1-reply")?.streaming).toBe(false);
      expect(store.getState().activeTurns).toHaveLength(0);
    });

    it("cancels an active streaming turn using cancel_turn command", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();

      await store.sendPrompt("Long running task");
      expect(store.getState().activeTurns[0]?.isStreaming).toBe(true);

      await store.cancelActiveTurn();
      expect(store.getState().activeTurns).toHaveLength(0);

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("cancel_turn");
      expect(validateCommandEnvelope(lastCommand!)).toBeNull();
    });
  });

  describe("Permission approve / deny actions", () => {
    it("approves permission and sends respond_permission command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await store.decidePermission("evt_fixture000000111", "approved");

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("evt_fixture000000111")?.permission?.decision).toBe("approved");

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("respond_permission");
      expect(validateCommandEnvelope(lastCommand!)).toBeNull();
      const payload = lastCommand?.payload as RespondPermissionCommand;
      expect(payload.event_id).toBe("evt_fixture000000111");
      expect(payload.decision).toBe("approved");
    });

    it("denies permission and sends respond_permission command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await store.decidePermission("evt_fixture000000111", "denied");

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("evt_fixture000000111")?.permission?.decision).toBe("denied");

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("respond_permission");
      const payload = lastCommand?.payload as RespondPermissionCommand;
      expect(payload.event_id).toBe("evt_fixture000000111");
      expect(payload.decision).toBe("denied");
    });

    it("rolls back permission decision and sets error on command failure", async () => {
      const transport = new InMemoryTransport();
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "respond_permission") {
          throw new Error("Core rejected permission response");
        }
        return undefined; // built-in responses keep the snapshot flow real
      });

      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await expect(store.decidePermission("evt_fixture000000111", "approved")).rejects.toThrow(
        "Core rejected permission response",
      );

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("evt_fixture000000111")?.permission?.decision).toBeNull();
      expect(store.getState().error).toContain("Core rejected permission response");
    });
  });

  describe("Event Deduplication, Reconnect Replay & Command Errors", () => {
    it("deduplicates events by event_id and sequence idempotently", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();
      await store.sendPrompt("Test deduplication");
      const liveTurnId = store.getState().activeTurns[0]?.turnId ?? null;

      const deltaEvent: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_fixture000000301",
        operation_id: "op_fixture000000310",
        thread_id: store.getState().selectedThreadId,
        turn_id: liveTurnId,
        sequence: 20 as any,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "Unique chunk" },
      };

      // Deliver once
      store._handleEvent(deltaEvent);
      // Deliver duplicate with same event_id
      store._handleEvent(deltaEvent);
      // Deliver duplicate with same sequence
      store._handleEvent({ ...deltaEvent, event_id: "evt_fixture000000302" });

      const threadStore = store.getTimelineStore(store.getState().selectedThreadId);
      expect(threadStore.getRow("send-1-reply")?.text).toBe("Unique chunk");
    });

    it("handles stream.replayed and stream.ready control events during reconnect", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Emit stream.replayed
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000311",
        operation_id: null,
        thread_id: null,
        turn_id: null,
        sequence: 30 as any,
        occurred_at: Date.now(),
        body: { kind: "stream.replayed", from: 25, through: 29 },
      });

      expect(store.getState().streamState).toBe("replaying");

      // Emit stream.ready
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000312",
        operation_id: null,
        thread_id: null,
        turn_id: null,
        sequence: 31 as any,
        occurred_at: Date.now(),
        body: { kind: "stream.ready", diagnostic: "ready" },
      });

      expect(store.getState().streamState).toBe("ready");
    });

    it("handles command.error events updating UI error state", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000321",
        operation_id: "op_fixture000000321",
        thread_id: null,
        turn_id: null,
        sequence: 40 as Sequence,
        occurred_at: Date.now(),
        body: {
          kind: "command.error",
          operation_id: "op_fixture000000321",
          code: "AGENT_NOT_FOUND",
          message: "The requested agent does not exist",
        },
      });

      expect(store.getState().error).toBe("[AGENT_NOT_FOUND] The requested agent does not exist");
    });

    it("automatically triggers request_snapshot recovery when stream.gap is received", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Clear previous commands count
      const initialCount = transport.sentCommands.length;

      // Deliver stream.gap event
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000331",
        operation_id: null,
        thread_id: null,
        turn_id: null,
        sequence: 50 as Sequence,
        occurred_at: Date.now(),
        body: {
          kind: "stream.gap",
          from: 45,
        },
      });

      await new Promise((resolve) => setTimeout(resolve, 10));

      const newCommands = transport.sentCommands.slice(initialCount);
      const newKinds = newCommands.map((c) => c.kind);
      expect(newKinds.filter((k) => k === "request_snapshot").length).toBeGreaterThanOrEqual(1);
    });

    it("refreshes threads and snapshot when core restarted greeting event arrives", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const initialCount = transport.sentCommands.length;

      // Deliver core.greeting event
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000341",
        operation_id: null,
        thread_id: null,
        turn_id: null,
        sequence: 60 as Sequence,
        occurred_at: Date.now(),
        body: {
          kind: "core.greeting",
          diagnostic: "Core daemon restarted",
        },
      });

      await new Promise((resolve) => setTimeout(resolve, 10));

      const newCommands = transport.sentCommands.slice(initialCount);
      const newKinds = newCommands.map((c) => c.kind);
      expect(newKinds.filter((k) => k === "request_snapshot").length).toBeGreaterThanOrEqual(1);
    });
  });

  describe("Thread and Agent Association", () => {
    it("binds created thread to selected agent and updates displayed agent on thread switch", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // 1. Select Agent Alpha and create Thread 1
      store.selectAgent("agp_fixture000000001");
      const thread1 = await store.createThread("Alpha Task", "agp_fixture000000001");
      expect(thread1.agent).toBe("alpha (ACP)");

      // 2. Select Agent Beta and create Thread 2
      store.selectAgent("agp_fixture000000002");
      const thread2 = await store.createThread("Beta Task", "agp_fixture000000002");
      expect(thread2.agent).toBe("beta (ACP)");

      // 3. Switch between threads
      await store.selectThread(thread1.id);
      expect(store.getState().selectedThreadId).toBe(thread1.id);
      const activeThread1 = store.getState().threads.find((t) => t.id === thread1.id);
      expect(activeThread1?.agent).toBe("alpha (ACP)");

      await store.selectThread(thread2.id);
      expect(store.getState().selectedThreadId).toBe(thread2.id);
      const activeThread2 = store.getState().threads.find((t) => t.id === thread2.id);
      expect(activeThread2?.agent).toBe("beta (ACP)");
    });
  });

  describe("A01 command identity contract", () => {
    it("every command the store emits satisfies the desktop contract validator", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      await store.createThread("Contract sweep", "agp_fixture000000001");
      await store.selectThread(approvalThread.id);
      await store.setThreadFilter("audit");
      await store.setThreadFilter("");
      await store.getHistory(approvalThread.id, 20);
      await store.testAgent({
        provider: "acp",
        program: "/opt/bin/agent-alpha",
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://sec-alpha-01",
        label: "Alpha Binding",
      });
      await store.onboardAgent({
        name: "中文代理",
        provider: "acp",
        program: "C:\\Agents\\My Agent\\acp-agent.exe",
        args: ["--mode", "server"],
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: "vault://acp-alpha",
        label: "Alpha Binding",
      });
      await store.sendPrompt("请帮我总结这份文档的三个要点");
      await store.cancelActiveTurn();
      await store.decidePermission("evt_fixture000000111", "approved");
      await store.getDiagnostics();
      await store.getContextSnapshot(approvalThread.id);
      await store.putIdentityDocument({ kind: "about", content: "用户偏好深色主题" });
      await store.listIdentityDocuments();
      await store.deleteIdentityDocument("idd_fixture000000013");
      await store.reconnect();

      expect(transport.sentCommands.length).toBeGreaterThan(10);
      for (const cmd of transport.sentCommands) {
        const rejection = validateCommandEnvelope(cmd);
        expect(rejection, `command ${cmd.kind} violated the contract`).toBeNull();
        expect(isValidOperationId(cmd.operation_id), `bad operation id ${cmd.operation_id}`).toBe(true);
      }
    });

    it("the fixture transport rejects the historical invalid command shapes", async () => {
      const transport = new InMemoryTransport();
      const validOp = createOperationId();

      const invalidCommands: CommandEnvelope[] = [
        {
          protocol_version: 1,
          operation_id: "op_start_turn_1",
          kind: "start_turn",
          payload: { thread_id: standardThread.id, turn_id: null, prompt: "x" },
          issued_at: 1700000000000,
        },
        {
          protocol_version: 1,
          operation_id: validOp,
          kind: "create_thread",
          payload: { agent_profile_id: "agent-alpha", title: "demo", project_id: null },
          issued_at: 1700000000000,
        },
        {
          protocol_version: 1,
          operation_id: validOp,
          kind: "start_turn",
          payload: { thread_id: standardThread.id, turn_id: "trn_1700000000000_1", prompt: "x" },
          issued_at: 1700000000000,
        },
        {
          protocol_version: 1,
          operation_id: "op_list_threads_1_1700000000000",
          kind: "list_threads",
          payload: { cursor: null, limit: 50 },
          issued_at: 1700000000000,
        },
        {
          protocol_version: 2,
          operation_id: validOp,
          kind: "list_threads",
          payload: { cursor: null, limit: 50 },
          issued_at: 1700000000000,
        },
      ];

      for (const cmd of invalidCommands) {
        await expect(transport.command(cmd)).rejects.toBeInstanceOf(InvalidCommandError);
        expect(transport.sentCommands).not.toContain(cmd);
      }
    });

    it("operation identities stay unique across store instances (renderer restart)", async () => {
      const seen = new Set<string>();

      for (let i = 0; i < 3; i += 1) {
        const transport = new InMemoryTransport();
        const store = createApplicationStore(transport);
        await store.init();
        for (const cmd of transport.sentCommands) {
          expect(seen.has(cmd.operation_id)).toBe(false);
          seen.add(cmd.operation_id);
        }
        store.disconnect();
      }
      expect(seen.size).toBeGreaterThan(0);
    });
  });

  describe("A03 authority, empty state & search", () => {
    it("a clean vault starts empty: no demo agents, no demo threads, no selection", async () => {
      const transport = new InMemoryTransport({ initialThreads: [], initialAgents: [] });
      const store = createApplicationStore(transport);
      await store.init();

      const state = store.getState();
      expect(state.threads).toHaveLength(0);
      expect(state.agents).toHaveLength(0);
      expect(state.selectedThreadId).toBe("");
      expect(state.selectedThread).toBeNull();
      expect(state.isOnboardingOpen).toBe(true);
      // No list extras may survive the authoritative empty response.
      expect(state.threads.some((t) => t.title.includes("Contract"))).toBe(false);
    });

    it("a late search response never overwrites a newer one (A/B out of order)", async () => {
      const transport = new InMemoryTransport();
      const timer: ReturnType<typeof setTimeout>[] = [];
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "search_threads") {
          const query = (cmd.payload as { query: string }).query;
          if (query === "slow-query") {
            // Resolve late to `undefined`, deferring to the built-in
            // handler — after request B has already won.
            return new Promise((resolve) => {
              timer.push(setTimeout(() => resolve(undefined), 20));
            });
          }
        }
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();

      // Request A is slow, request B resolves first and must win.
      const slow = store.setThreadFilter("slow-query");
      const fast = store.setThreadFilter("审计");
      await fast;
      expect(store.getState().searchActive).toBe(true);
      expect(store.getState().threads.map((t) => t.id)).toEqual([approvalThread.id]);

      await slow;
      // The stale A response arrives and must not replace B's results.
      expect(store.getState().threads.map((t) => t.id)).toEqual([approvalThread.id]);
      timer.forEach(clearTimeout);
    });

    it("clearing the query restores the authoritative list and selection stays stable", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);
      expect(store.getState().selectedThread?.id).toBe(approvalThread.id);

      await store.setThreadFilter("审计");
      expect(store.getState().threads.map((t) => t.id)).toEqual([approvalThread.id]);
      expect(store.getState().selectedThread?.id).toBe(approvalThread.id);

      await store.setThreadFilter("   ");
      const state = store.getState();
      expect(state.searchActive).toBe(false);
      expect(state.threads.map((t) => t.id)).toEqual([
        standardThread.id,
        approvalThread.id,
        failureThread.id,
      ]);
      expect(state.selectedThread?.id).toBe(approvalThread.id);
    });

    it("a search failure is surfaced in place instead of faking client filter success", async () => {
      const transport = new InMemoryTransport();
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "search_threads") {
          throw new Error("search index unavailable");
        }
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await store.setThreadFilter("anything");

      expect(store.getState().error).toContain("search index unavailable");
      // The list itself is untouched: no pretend results.
      expect(store.getState().searchActive).toBe(false);
      expect(store.getState().threads.map((t) => t.id)).toEqual([
        standardThread.id,
        approvalThread.id,
        failureThread.id,
      ]);
    });

    it("paginates 50+ threads with has_more and the stored cursor", async () => {
      const threads = Array.from({ length: 55 }, (_, i) => ({
        id: `thr_bulk${String(1000 + i).padStart(12, "0")}`,
        title: `Bulk conversation ${i}`,
        agent: "alpha (ACP)",
        status: "completed" as const,
        pinned: false,
        rows: [],
      }));
      const transport = new InMemoryTransport({ initialThreads: threads });
      const store = createApplicationStore(transport);
      await store.init();

      // The fake defaults to limit 20 when only cursor is absent... the
      // store requests 50 per page, so page one holds 50 rows.
      let state = store.getState();
      expect(state.threads).toHaveLength(50);
      expect(state.hasMoreThreads).toBe(true);

      await store.loadMoreThreads();
      state = store.getState();
      expect(state.threads).toHaveLength(55);
      expect(state.hasMoreThreads).toBe(false);

      // No duplicates after paging to the end.
      const ids = state.threads.map((t) => t.id);
      expect(new Set(ids).size).toBe(ids.length);
    });

    it("selecting a thread while searching keeps the search results visible", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      await store.setThreadFilter("审计");
      expect(store.getState().threads.map((t) => t.id)).toEqual([approvalThread.id]);

      await store.selectThread(approvalThread.id);
      const state = store.getState();
      // The nav list still shows the search results…
      expect(state.searchActive).toBe(true);
      expect(state.threads.map((t) => t.id)).toEqual([approvalThread.id]);
      // …while the read conversation is the selected one.
      expect(state.selectedThread?.id).toBe(approvalThread.id);
    });
  });

  describe("A04 delivery, cancellation & permissions", () => {
    it("tracks two background turns independently and routes deltas by thread/turn", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();

      const threadA = standardThread.id;
      const threadB = approvalThread.id;
      await store.selectThread(threadA);
      await store.sendPrompt("Prompt for A", threadA);
      await store.sendPrompt("Prompt for B", threadB);

      const turns = store.getState().activeTurns;
      expect(turns).toHaveLength(2);
      const turnA = turns.find((t) => t.threadId === threadA)!;
      const turnB = turns.find((t) => t.threadId === threadB)!;
      expect(turnA.turnId).toMatch(/^trn_[0-9a-z]{16,64}$/);
      expect(turnB.turnId).toMatch(/^trn_[0-9a-z]{16,64}$/);
      expect(turnA.turnId).not.toBe(turnB.turnId);

      // A's delta lands only in A's reply row…
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000401",
        operation_id: "op_fixture000000410",
        thread_id: threadA,
        turn_id: turnA.turnId,
        sequence: 100 as Sequence,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "chunk-for-A" },
      });
      // …and B's delta only in B's reply row.
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000402",
        operation_id: "op_fixture000000410",
        thread_id: threadB,
        turn_id: turnB.turnId,
        sequence: 101 as Sequence,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "chunk-for-B" },
      });

      expect(store.getTimelineStore(threadA).getRow(turnA.replyRowId)?.text).toBe("chunk-for-A");
      expect(store.getTimelineStore(threadB).getRow(turnB.replyRowId)?.text).toBe("chunk-for-B");

      // A completes: only A's turn clears.
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000403",
        operation_id: "op_fixture000000410",
        thread_id: threadA,
        turn_id: turnA.turnId,
        sequence: 102 as Sequence,
        occurred_at: Date.now(),
        body: { kind: "turn.completed" },
      });
      expect(store.getState().activeTurns.map((t) => t.threadId)).toEqual([threadB]);
    });

    it("rejects a double send while a turn is unsettled, without new rows", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();
      const threadId = standardThread.id;

      const first = await store.sendPrompt("First dispatch", threadId);
      expect(first.status).toBe("admitted");
      const rowsBefore = store.getTimelineStore(threadId).rowCount();

      const second = await store.sendPrompt("Second dispatch", threadId);
      if (second.status !== "rejected") throw new Error("expected rejected dispatch");
      expect(second.reason).toContain("already running");
      expect(store.getTimelineStore(threadId).rowCount()).toBe(rowsBefore);
      expect(store.getState().activeTurns).toHaveLength(1);
    });

    it("distinguishes not-delivered from indeterminate dispatch failures", async () => {
      // Hard rejection before Core executes: prompt was not delivered.
      const transportA = new InMemoryTransport({ autoStreamReplies: false });
      transportA.setCommandHandler((cmd) => {
        if (cmd.kind === "start_turn") throw new Error("agent refused the turn");
        return undefined;
      });
      const storeA = createApplicationStore(transportA);
      await storeA.init();
      const rejected = await storeA.sendPrompt("hello", standardThread.id);
      if (rejected.status !== "rejected") throw new Error("expected rejected dispatch");
      expect(rejected.reason).toContain("agent refused");
      expect(
        storeA.getTimelineStore(standardThread.id).getSnapshot().rows.some(
          (r) => r.kind === "error" && r.text.startsWith("Prompt not delivered"),
        ),
      ).toBe(true);
      expect(storeA.getState().activeTurns).toHaveLength(0);

      // Connection dropped mid-dispatch: delivery is indeterminate, never
      // auto-retried.
      const transportB = new InMemoryTransport({ autoStreamReplies: false });
      transportB.setCommandHandler((cmd) => {
        if (cmd.kind === "start_turn") throw new ConnectionClosedError();
        return undefined;
      });
      const storeB = createApplicationStore(transportB);
      await storeB.init();
      const indeterminate = await storeB.sendPrompt("hello", standardThread.id);
      if (indeterminate.status !== "indeterminate") throw new Error("expected indeterminate dispatch");
      expect(
        storeB.getTimelineStore(standardThread.id).getSnapshot().rows.some(
          (r) => r.kind === "error" && r.text.includes("Delivery indeterminate"),
        ),
      ).toBe(true);
      expect(storeB.getState().activeTurns).toHaveLength(0);
    });

    it("keeps the turn running when the cancel request fails, then settles on the authoritative event", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "cancel_turn") throw new Error("cancel channel lost");
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();
      const threadId = standardThread.id;
      await store.sendPrompt("Long task", threadId);

      await store.cancelActiveTurn(threadId);
      // The turn is NOT locally finished: Core may still be running it.
      expect(store.getState().activeTurns).toHaveLength(1);
      expect(store.getState().activeTurns[0]?.cancelState).toBe("failed");
      expect(store.getState().activeTurns[0]?.notice).toBe("cancel_failed:cancel channel lost");

      // The authoritative settlement clears the turn.
      const turnId = store.getState().activeTurns[0]?.turnId ?? null;
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_fixture000000411",
        operation_id: "op_fixture000000411",
        thread_id: threadId,
        turn_id: turnId,
        sequence: 120 as Sequence,
        occurred_at: Date.now(),
        body: { kind: "turn.cancelled", reason: "cancelled after reconnect" },
      });
      expect(store.getState().activeTurns).toHaveLength(0);
    });

    it("marks cancel as requested and waits for Core before clearing", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();
      const threadId = standardThread.id;
      await store.sendPrompt("Task", threadId);

      // The cancel command stays in flight: the turn must already read as
      // "requested", and it must remain active until Core settles it.
      let releaseCancel: (() => void) | null = null;
      const gate = new Promise<void>((resolve) => {
        releaseCancel = resolve;
      });
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "cancel_turn") {
          return gate.then(() => undefined);
        }
        return undefined;
      });

      const cancelling = store.cancelActiveTurn(threadId);
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(store.getState().activeTurns[0]?.cancelState).toBe("requested");
      // Still running until Core settles it.
      expect(store.getState().activeTurns).toHaveLength(1);

      const cancelCmd = transport.sentCommands.find((c) => c.kind === "cancel_turn");
      expect(cancelCmd).toBeDefined();
      expect(validateCommandEnvelope(cancelCmd!)).toBeNull();

      releaseCancel!();
      await cancelling;
      // The built-in handler settles the cancellation by emitting the
      // authoritative turn.cancelled event — that, not the RPC result, is
      // what finally clears the turn.
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(store.getState().activeTurns).toHaveLength(0);
    });

    it("ignores duplicate permission submissions while one is in flight", async () => {
      const transport = new InMemoryTransport();
      let release!: () => void;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "respond_permission") {
          return gate.then(() => ({ ok: true }));
        }
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);
      const permRowId = "evt_fixture000000111";

      const first = store.decidePermission(permRowId, "approved");
      // Second click while the first is in flight is a no-op.
      await store.decidePermission(permRowId, "denied");
      release();
      await first;

      const permissionCommands = transport.sentCommands.filter(
        (c) => c.kind === "respond_permission",
      );
      expect(permissionCommands).toHaveLength(1);
      expect(
        store.getTimelineStore(approvalThread.id).getRow(permRowId)?.permission?.decision,
      ).toBe("approved");
      expect(
        store.getTimelineStore(approvalThread.id).getRow(permRowId)?.permission?.submission,
      ).toBeNull();
    });

    it("shows a failed permission decision as still unanswered and checkable", async () => {
      const transport = new InMemoryTransport();
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "respond_permission") {
          throw new Error("permission window closed");
        }
        return undefined;
      });
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);
      const permRowId = "evt_fixture000000111";

      await expect(store.decidePermission(permRowId, "approved")).rejects.toThrow(
        "permission window closed",
      );

      const row = store.getTimelineStore(approvalThread.id).getRow(permRowId);
      expect(row?.permission?.decision).toBeNull();
      expect(row?.permission?.submission).toBe("failed");
      expect(store.getState().error).toContain("permission window closed");
    });
  });
});
