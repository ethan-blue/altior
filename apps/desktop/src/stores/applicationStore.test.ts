import { describe, expect, it } from "vitest";
import { approvalThread, standardThread } from "../fixtures/timeline";
import type { AgentProfileDto } from "../ipc/dto/AgentProfileDto";
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

    it("configures dual agents sequentially with full binding DTOs and retains tested binding ID", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // 1. Probe Agent A with test_harness_binding
      const testResultA = await store.testAgent({
        provider: "acp",
        model: "claude-3-7-sonnet",
        program: "/opt/bin/agent-alpha",
        args: ["--mode", "server", "--port", "8001"],
        envKeys: ["ANTHROPIC_API_KEY", "DEBUG"],
        secretRef: "vault://sec-alpha-01",
        label: "Alpha Primary Binding",
      });
      expect(testResultA.success).toBe(true);

      const testCmdA = transport.sentCommands.find((c) => c.kind === "test_harness_binding");
      expect(testCmdA).toBeDefined();
      const testPayloadA = testCmdA?.payload as TestHarnessBindingCommand;
      expect(testPayloadA.program).toBe("/opt/bin/agent-alpha");
      expect(testPayloadA.args).toEqual(["--mode", "server", "--port", "8001"]);
      expect(testPayloadA.env_keys).toEqual(["ANTHROPIC_API_KEY", "DEBUG"]);
      expect(testPayloadA.secret_refs).toEqual(["vault://sec-alpha-01"]);
      expect(testPayloadA.label).toBe("Alpha Primary Binding");
      const probedBindingIdA = testPayloadA.harness_binding_id;
      expect(probedBindingIdA).toBeTruthy();

      // Save Agent A -> should use the exact same binding ID
      const agentA = await store.onboardAgent({
        name: "Agent Alpha Custom",
        provider: "acp",
        model: "claude-3-7-sonnet",
        program: "/opt/bin/agent-alpha",
        args: ["--mode", "server", "--port", "8001"],
        envKeys: ["ANTHROPIC_API_KEY", "DEBUG"],
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
      expect(configPayloadA.binding?.env_keys).toEqual(["ANTHROPIC_API_KEY", "DEBUG"]);
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

      // Verify both agents exist in application store state
      const state = store.getState();
      expect(state.agents.some((a) => a.id === agentA.id)).toBe(true);
      expect(state.agents.some((a) => a.id === agentB.id)).toBe(true);
      expect(state.selectedAgentId).toBe(agentB.id);

      // Verify transport in-memory bindings map has both bindings
      expect(transport.bindings.has(configPayloadA.binding!.harness_binding_id!)).toBe(true);
      expect(transport.bindings.has(configPayloadB.binding!.harness_binding_id!)).toBe(true);
    });

    it("supports configure_agent without binding for backward compatibility", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const configureEnvelope: CommandEnvelope = {
        protocol_version: 1,
        operation_id: "op_legacy_config",
        kind: "configure_agent",
        payload: {
          agent_profile_id: "agent-legacy",
          display_name: "Legacy Agent",
          preferred_harness: "acp",
          memory_mode: "session",
          binding: null,
        } as ConfigureAgentCommand,
        issued_at: Date.now(),
      };

      const res = await transport.command<{ ok: boolean; profile: AgentProfileDto; warning: string | null }>(
        configureEnvelope,
      );
      expect(res.ok).toBe(true);
      expect(res.profile.id).toBe("agent-legacy");
      expect(res.warning).toContain("Legacy configuration without harness binding");
      expect(transport.agents.some((a) => a.id === "agent-legacy")).toBe(true);
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
      const transport = new InMemoryTransport({ initialAgents: [] });
      const store = createApplicationStore(transport, { initialAgents: [] });

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
        sequence: 40 as Sequence,
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

    it("automatically triggers list_threads and open_thread snapshot recovery when stream.gap is received", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Clear previous commands count
      const initialCount = transport.sentCommands.length;

      // Deliver stream.gap event
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_gap_1",
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
      expect(newKinds).toContain("list_threads");
      expect(newKinds).toContain("open_thread");
    });

    it("refreshes threads and snapshot when core restarted greeting event arrives", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const initialCount = transport.sentCommands.length;

      // Deliver core.greeting event
      store._handleEvent({
        protocol_version: 1,
        event_id: "evt_greeting_restart",
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
      expect(newKinds).toContain("list_threads");
      expect(newKinds).toContain("open_thread");
    });
  });

  describe("Thread and Agent Association", () => {
    it("binds created thread to selected agent and updates displayed agent on thread switch", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // 1. Select Agent Alpha and create Thread 1
      store.selectAgent("agent-alpha");
      const thread1 = await store.createThread("Alpha Task", "agent-alpha");
      expect(thread1.agent).toBe("alpha (ACP)");

      // 2. Select Agent Beta and create Thread 2
      store.selectAgent("agent-beta");
      const thread2 = await store.createThread("Beta Task", "agent-beta");
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
});
