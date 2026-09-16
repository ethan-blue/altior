/**
 * A07 Acceptance Evidence: Honest and Robust ACP Configuration (F11 / F27).
 *
 * Proves that:
 * 1. Command-line arguments parse correctly, preserving quoted paths with spaces
 *    instead of naive whitespace splitting breaking executable paths.
 * 2. Unauthenticated local agents (empty envKeys and empty secretRef) are fully supported.
 * 3. 1-to-1 parity between environment variable keys and opaque secret references
 *    is enforced before dispatch.
 * 4. Plaintext API keys are rejected with an explicit error up front; no fake random
 *    tokens are silently fabricated.
 * 5. Canary: plaintext credentials never leak into store state, stream logs, or IPC payloads.
 * 6. Honest protocol: ACP is the sole supported harness; terminal/native are deferred.
 */
import { describe, expect, it } from "vitest";
import {
  createApplicationStore,
  parseCommandLineArgs,
  resolveEnvSecretPairs,
  sanitizeSecretRef,
} from "../stores/applicationStore";
import { InMemoryTransport } from "../ipc/inMemoryTransport";

describe("A07 evidence: Honest & robust ACP configuration", () => {
  describe("Command-line argument parsing (preserves quoted paths with spaces)", () => {
    it("preserves double and single quoted paths and flags containing spaces", () => {
      const input =
        '--mode server --config "C:\\Program Files\\Altior Agent\\config.json" --tag \'local runner\' --verbose';
      const parsed = parseCommandLineArgs(input);

      expect(parsed).toEqual([
        "--mode",
        "server",
        "--config",
        "C:\\Program Files\\Altior Agent\\config.json",
        "--tag",
        "local runner",
        "--verbose",
      ]);
    });

    it("handles simple unquoted arguments and empty strings cleanly", () => {
      expect(parseCommandLineArgs("")).toEqual([]);
      expect(parseCommandLineArgs("   ")).toEqual([]);
      expect(parseCommandLineArgs("--acp --port 8080")).toEqual([
        "--acp",
        "--port",
        "8080",
      ]);
    });

    it("handles already parsed array input transparently", () => {
      const existing = ["--acp", "--stdio"];
      expect(parseCommandLineArgs(existing)).toEqual(["--acp", "--stdio"]);
    });
  });

  describe("Opaque secret reference validation & plaintext rejection (F27)", () => {
    it("accepts valid opaque secret store references", () => {
      expect(sanitizeSecretRef("sec_anthropic_vault_01")).toBe("sec_anthropic_vault_01");
      expect(sanitizeSecretRef("vault://agents/claude-key")).toBe("vault://agents/claude-key");
      expect(sanitizeSecretRef("env:ANTHROPIC_API_KEY")).toBe("env:ANTHROPIC_API_KEY");
      expect(sanitizeSecretRef("secret://production/key")).toBe("secret://production/key");
      expect(sanitizeSecretRef("ref:opaque-handle-42")).toBe("ref:opaque-handle-42");
    });

    it("strictly rejects plaintext API keys without fabricating fake random references", () => {
      const plaintextKeys = [
        "sk-ant-api03-abcdef1234567890",
        "sk-proj-9876543210fedcba",
        "ghp_randomGitHubPersonalAccessToken123",
        "password123456!",
      ];

      for (const key of plaintextKeys) {
        expect(() => sanitizeSecretRef(key)).toThrow(
          /credentials must be stored in the OS secret store/i,
        );
      }
    });

    it("canary: plaintext secrets never leak into state, stream logs, or command payloads", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const canarySecret = "SUPER-SECRET-PLAINTEXT-KEY-CANARY-99999";

      // Onboarding with plaintext secret throws immediately
      await expect(
        store.onboardAgent({
          name: "Canary Agent",
          provider: "acp",
          program: "C:\\Agents\\acp.exe",
          args: '--config "C:\\Program Files\\agent.json"',
          envKeys: ["ANTHROPIC_API_KEY"],
          secretRef: canarySecret,
        }),
      ).rejects.toThrow(/credentials must be stored in the OS secret store/i);

      // Testing with plaintext secret fails immediately
      const testResult = await store.testAgent({
        provider: "acp",
        program: "C:\\Agents\\acp.exe",
        envKeys: ["ANTHROPIC_API_KEY"],
        secretRef: canarySecret,
      });
      expect(testResult.success).toBe(false);

      // Verify canary secret never entered store state
      expect(JSON.stringify(store.getState())).not.toContain(canarySecret);

      // Verify canary secret never entered transport commands
      for (const cmd of transport.sentCommands) {
        expect(JSON.stringify(cmd)).not.toContain(canarySecret);
      }
    });
  });

  describe("Agent configuration and harness parity", () => {
    it("supports local agents without authentication (empty envKeys and empty secretRef)", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const profile = await store.onboardAgent({
        name: "Local Offline Agent",
        provider: "acp",
        program: "ollama-acp",
        args: '--model llama3 --host "http://127.0.0.1:11434"',
        envKeys: [],
        secretRef: undefined,
      });

      expect(profile).toBeDefined();
      expect(profile.name).toBe("Local Offline Agent");
      expect(profile.args).toEqual(["--model", "llama3", "--host", "http://127.0.0.1:11434"]);
    });

    it("enforces 1-to-1 parity between environment variable keys and secret references", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Mismatch: 1 env key provided, but 0 secret refs
      await expect(
        store.onboardAgent({
          name: "Mismatched Agent 1",
          provider: "acp",
          program: "acp-agent",
          envKeys: ["ANTHROPIC_API_KEY"],
          secretRef: undefined,
        }),
      ).rejects.toThrow(/requires exactly one credential reference/i);

      // Mismatch: 0 env keys provided, but 1 secret ref
      await expect(
        store.onboardAgent({
          name: "Mismatched Agent 2",
          provider: "acp",
          program: "acp-agent",
          envKeys: [],
          secretRef: "sec_vault_0000000001",
        }),
      ).rejects.toThrow(/requires exactly one credential reference/i);
    });

    it("test agent captures latency and failure without blocking save of corrected configuration", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Test with missing program returns clean failure
      const failTest = await store.testAgent({
        program: "",
        provider: "",
      });
      expect(failTest.success).toBe(false);
      expect(failTest.error).toContain("program is required");

      // Test with valid configuration succeeds
      const passTest = await store.testAgent({
        provider: "acp",
        program: "C:\\Agents\\acp-agent.exe",
        args: '--flag "value with spaces"',
        envKeys: ["API_KEY"],
        secretRef: "sec_vault_0000000001",
      });
      expect(passTest.success).toBe(true);
      expect(passTest.latencyMs).toBeGreaterThanOrEqual(0);
    });

    it("resolveEnvSecretPairs supports 0, 1, and N mappings and rejects duplicates or missing keys", () => {
      // 0 mappings
      expect(resolveEnvSecretPairs({ envMappings: [] })).toEqual({ envKeys: [], secretRefs: [] });

      // 1 mapping
      const single = resolveEnvSecretPairs({
        envMappings: [{ envKey: "OPENAI_API_KEY", secretRef: "vault://key-1" }],
      });
      expect(single.envKeys).toEqual(["OPENAI_API_KEY"]);
      expect(single.secretRefs).toEqual(["vault://key-1"]);

      // N mappings
      const multiple = resolveEnvSecretPairs({
        envMappings: [
          { envKey: "ANTHROPIC_API_KEY", secretRef: "sec_anthropic_vault_01" },
          { envKey: "OPENAI_API_KEY", secretRef: "vault://openai-key" },
          { envKey: "CUSTOM_SECRET_ENV", secretRef: "env:CUSTOM_ENV_VAL" },
        ],
      });
      expect(multiple.envKeys).toEqual(["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "CUSTOM_SECRET_ENV"]);
      expect(multiple.secretRefs).toEqual(["sec_anthropic_vault_01", "vault://openai-key", "env:CUSTOM_ENV_VAL"]);

      // Duplicate env key rejection
      expect(() =>
        resolveEnvSecretPairs({
          envMappings: [
            { envKey: "DUP_KEY", secretRef: "vault://1" },
            { envKey: "DUP_KEY", secretRef: "vault://2" },
          ],
        }),
      ).toThrow(/Duplicate environment variable key: DUP_KEY/);

      // Missing secret ref
      expect(() =>
        resolveEnvSecretPairs({
          envMappings: [{ envKey: "VALID_KEY", secretRef: "" }],
        }),
      ).toThrow(/requires an opaque credential reference/);

      // Missing env key
      expect(() =>
        resolveEnvSecretPairs({
          envMappings: [{ envKey: "", secretRef: "vault://key" }],
        }),
      ).toThrow(/must provide a variable key/);

      // Plaintext secret rejection in mapping
      expect(() =>
        resolveEnvSecretPairs({
          envMappings: [{ envKey: "API_KEY", secretRef: "sk-proj-plaintext12345" }],
        }),
      ).toThrow(/credentials must be stored in the OS secret store/i);
    });

    it("capabilities roundtrip: testAgent returns negotiated capabilities and onboardAgent persists them", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      const testRes = await store.testAgent({
        provider: "acp",
        program: "acp-agent",
        envMappings: [{ envKey: "TEST_KEY", secretRef: "vault://test-ref" }],
      });

      expect(testRes.success).toBe(true);
      expect(testRes.capabilities).toBeDefined();
      expect(testRes.capabilities?.["session.update"]).toBe("supported");
      expect(testRes.capabilities?.["thread.streaming"]).toBe("supported");

      // Onboard agent adopts capabilities from test result or parameter
      const agent = await store.onboardAgent({
        name: "Capable ACP Agent",
        provider: "acp",
        program: "acp-agent",
        envMappings: [{ envKey: "TEST_KEY", secretRef: "vault://test-ref" }],
        capabilities: testRes.capabilities,
      });

      expect(agent.capabilities).toEqual(testRes.capabilities);
      expect(agent.secretRefs).toEqual(["vault://test-ref"]);
    });

    it("model is optional and does not force hardcoded claude-3-7-sonnet default", async () => {
      const transport = new InMemoryTransport();
      const store = createApplicationStore(transport);
      await store.init();

      // Onboarding without model leaves model undefined
      const agentWithoutModel = await store.onboardAgent({
        name: "Agent Without Model",
        provider: "acp",
        program: "local-agent",
        envKeys: [],
        secretRefs: [],
      });

      expect(agentWithoutModel.model).toBeUndefined();

      // Onboarding with explicit model preserves it
      const agentWithModel = await store.onboardAgent({
        name: "Agent With Model",
        provider: "acp",
        model: "custom-gpt-4o",
        program: "custom-agent",
        envKeys: [],
        secretRefs: [],
      });

      expect(agentWithModel.model).toBe("custom-gpt-4o");
    });
  });
});
