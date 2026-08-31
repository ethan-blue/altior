import { describe, expect, it } from "vitest";
import { approvalThread, standardThread } from "../fixtures/timeline";
import type { ConfigureAgentCommand } from "../ipc/dto/ConfigureAgentCommand";
import type { CreateThreadCommand } from "../ipc/dto/CreateThreadCommand";
import type { EventEnvelope } from "../ipc/dto/EventEnvelope";
import type { OpenThreadCommand } from "../ipc/dto/OpenThreadCommand";
import type { RespondPermissionCommand } from "../ipc/dto/RespondPermissionCommand";
import type { StartTurnCommand } from "../ipc/dto/StartTurnCommand";
import type { TestHarnessBindingCommand } from "../ipc/dto/TestHarnessBindingCommand";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
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
    it("sanitizes plaintext secrets into opaque references without persisting plaintext", () => {
      expect(sanitizeSecretRef("vault://key-123")).toBe("vault://key-123");
      expect(sanitizeSecretRef("env:ANTHROPIC_API_KEY")).toBe("env:ANTHROPIC_API_KEY");
      expect(sanitizeSecretRef("ref:secret-456")).toBe("ref:secret-456");

      // Plaintext API key is converted to opaque reference
      const sanitized = sanitizeSecretRef("sk-proj-1234567890abcdef");
      expect(sanitized).toMatch(/^ref:opaque-sec-/);
      expect(sanitized).not.toContain("sk-proj");
    });

    it("canary: plaintext secrets never enter stream log, state, or command payloads", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const canarySecret = "SUPER-SECRET-PLAINTEXT-KEY-CANARY-12345";

      await store.onboardAgent({
        name: "Canary Agent",
        provider: "acp",
        model: "claude-3-7-sonnet",
        secretRef: canarySecret,
      });

      await store.testAgent({
        provider: "acp",
        model: "claude-3-7-sonnet",
        secretRef: canarySecret,
      });

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

    it("onboards a new agent using configure_agent command and sets it active", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const newAgent = await store.onboardAgent({
        name: "Custom Agent",
        provider: "acp",
        model: "claude-3-7-sonnet",
        secretRef: "vault://custom-key",
      });

      expect(newAgent.id).toContain("custom-agent");
      expect(newAgent.secretRef).toBe("vault://custom-key");

      const state = store.getState();
      expect(state.agents.some((a) => a.id === newAgent.id)).toBe(true);
      expect(state.selectedAgentId).toBe(newAgent.id);

      // Verify configure_agent command envelope
      const configCmd = transport.sentCommands.find((c) => c.kind === "configure_agent");
      expect(configCmd).toBeDefined();
      const payload = configCmd?.payload as ConfigureAgentCommand;
      expect(payload.display_name).toBe("Custom Agent");
      expect(payload.preferred_harness).toBe("acp");
    });

    it("tests agent connection via test_harness_binding command with secret_refs", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const result = await store.testAgent({
        provider: "/path/to/acp-binary",
        model: "claude-3-7-sonnet",
        secretRef: "vault://test-key",
      });

      expect(result.success).toBe(true);
      expect(result.latencyMs).toBeGreaterThan(0);
      expect(store.getState().onboardingStatus.testResult?.success).toBe(true);

      const testCmd = transport.sentCommands.find((c) => c.kind === "test_harness_binding");
      expect(testCmd).toBeDefined();
      const payload = testCmd?.payload as TestHarnessBindingCommand;
      expect(payload.program).toBe("/path/to/acp-binary");
      expect(payload.secret_refs).toEqual(["vault://test-key"]);
    });
  });

  describe("Thread List, Creation, Search & Navigation", () => {
    it("creates a new thread using create_thread command and switches to it", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const thread = await store.createThread("Spike Investigation", "agent-beta");
      expect(thread.title).toBe("Spike Investigation");

      const state = store.getState();
      expect(state.threads.some((t) => t.id === thread.id)).toBe(true);
      expect(state.selectedThreadId).toBe(thread.id);

      const createCmd = transport.sentCommands.find((c) => c.kind === "create_thread");
      expect(createCmd).toBeDefined();
      const payload = createCmd?.payload as CreateThreadCommand;
      expect(payload.title).toBe("Spike Investigation");
      expect(payload.agent_profile_id).toBe("agent-beta");

      // Verify open_thread was sent for the new thread
      const openCmds = transport.sentCommands.filter((c) => c.kind === "open_thread");
      expect(openCmds.some((c) => (c.payload as OpenThreadCommand).thread_id === thread.id)).toBe(true);
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
    it("dispatches prompt via start_turn and streams incoming deltas to the assistant reply row", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();

      await store.sendPrompt("Explain length-prefixed framing");

      const state = store.getState();
      expect(state.activeTurn).not.toBeNull();
      expect(state.activeTurn?.isStreaming).toBe(true);

      const startCmd = transport.sentCommands.find((c) => c.kind === "start_turn");
      expect(startCmd).toBeDefined();
      const startPayload = startCmd?.payload as StartTurnCommand;
      expect(startPayload.prompt).toBe("Explain length-prefixed framing");

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
        event_id: "evt_delta_1",
        operation_id: "op_start_turn_1",
        thread_id: state.selectedThreadId,
        turn_id: "turn-1",
        sequence: 10 as any,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "Frames have 4-byte headers." },
      };
      store._handleEvent(deltaEvent);

      expect(threadStore.getRow("send-1-reply")?.text).toBe("Frames have 4-byte headers.");

      // Second delta
      const deltaEvent2: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_delta_2",
        operation_id: "op_start_turn_1",
        thread_id: state.selectedThreadId,
        turn_id: "turn-1",
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
        event_id: "evt_comp_1",
        operation_id: "op_start_turn_1",
        thread_id: state.selectedThreadId,
        turn_id: "turn-1",
        sequence: 12 as any,
        occurred_at: Date.now(),
        body: { kind: "turn.completed" },
      };
      store._handleEvent(completeEvent);

      expect(threadStore.getRow("send-1-reply")?.streaming).toBe(false);
      expect(store.getState().activeTurn).toBeNull();
    });

    it("cancels an active streaming turn using cancel_turn command", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();

      await store.sendPrompt("Long running task");
      expect(store.getState().activeTurn?.isStreaming).toBe(true);

      await store.cancelActiveTurn();
      expect(store.getState().activeTurn).toBeNull();

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("cancel_turn");
    });
  });

  describe("Permission approve / deny actions", () => {
    it("approves permission and sends respond_permission command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await store.decidePermission("apr-3", "approved");

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("apr-3")?.permission?.decision).toBe("approved");

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("respond_permission");
      const payload = lastCommand?.payload as RespondPermissionCommand;
      expect(payload.event_id).toBe("apr-3");
      expect(payload.decision).toBe("approved");
    });

    it("denies permission and sends respond_permission command", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await store.decidePermission("apr-3", "denied");

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("apr-3")?.permission?.decision).toBe("denied");

      const lastCommand = transport.sentCommands.at(-1);
      expect(lastCommand?.kind).toBe("respond_permission");
      const payload = lastCommand?.payload as RespondPermissionCommand;
      expect(payload.event_id).toBe("apr-3");
      expect(payload.decision).toBe("denied");
    });

    it("rolls back permission decision and sets error on command failure", async () => {
      const transport = new InMemoryTransport();
      transport.setCommandHandler((cmd) => {
        if (cmd.kind === "respond_permission") {
          throw new Error("Core rejected permission response");
        }
        return { ok: true };
      });

      const store = createApplicationStore(transport);
      await store.init();
      await store.selectThread(approvalThread.id);

      await expect(store.decidePermission("apr-3", "approved")).rejects.toThrow(
        "Core rejected permission response",
      );

      const timelineStore = store.getTimelineStore(approvalThread.id);
      expect(timelineStore.getRow("apr-3")?.permission?.decision).toBeNull();
      expect(store.getState().error).toContain("Core rejected permission response");
    });
  });

  describe("Event Deduplication, Reconnect Replay & Command Errors", () => {
    it("deduplicates events by event_id and sequence idempotently", async () => {
      const transport = new InMemoryTransport({ autoStreamReplies: false });
      const store = createApplicationStore(transport);
      await store.init();
      await store.sendPrompt("Test deduplication");

      const deltaEvent: EventEnvelope = {
        protocol_version: 1,
        event_id: "evt_dup_1",
        operation_id: "op_1",
        thread_id: store.getState().selectedThreadId,
        turn_id: "turn-1",
        sequence: 20 as any,
        occurred_at: Date.now(),
        body: { kind: "message.delta", text: "Unique chunk" },
      };

      // Deliver once
      store._handleEvent(deltaEvent);
      // Deliver duplicate with same event_id
      store._handleEvent(deltaEvent);
      // Deliver duplicate with same sequence
      store._handleEvent({ ...deltaEvent, event_id: "evt_dup_2" });

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
        event_id: "evt_replay_ctrl",
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
        event_id: "evt_ready_ctrl",
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
        event_id: "evt_err_1",
        operation_id: "op_err_1",
        thread_id: null,
        turn_id: null,
        sequence: 40 as any,
        occurred_at: Date.now(),
        body: {
          kind: "command.error",
          operation_id: "op_err_1",
          code: "AGENT_NOT_FOUND",
          message: "The requested agent does not exist",
        },
      });

      expect(store.getState().error).toBe("[AGENT_NOT_FOUND] The requested agent does not exist");
    });
  });
});
