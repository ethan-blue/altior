/**
 * Evidence: capability support tokens, degraded badge codes, zh demo fixtures.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextPanel } from "../components/ContextPanel";
import { AgentOnboardingModal } from "../components/shell";
import {
  approvalThread,
  failureThread,
  standardThread,
} from "../fixtures/timeline";
import { I18nProvider, getDictionary } from "../i18n";
import {
  localizeCapabilitySupport,
  localizeDegradedCode,
} from "../i18n/localizeEnums";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";

afterEach(() => {
  cleanup();
});

const zh = getDictionary("zh-CN");
const en = getDictionary("en");

function fixtureSnapshot(
  overrides: Partial<ContextSnapshotDto> = {},
): ContextSnapshotDto {
  return {
    turn_id: "trn_p30degrade000000000000001",
    thread_id: "thr_p30degrade000000000000001",
    memory_mode: "long_term",
    created_at: 1,
    passthrough: false,
    budget: {
      identity_limit_tokens: 100,
      memory_limit_tokens: 200,
      prompt_tokens: 1,
      identity_tokens: 1,
      memory_tokens: 1,
      total_tokens: 3,
    },
    identity: [],
    memories: [],
    dropped: [],
    degraded: null,
    rendered_prompt: null,
    ...overrides,
  };
}

describe("p30 capability tokens + degraded codes + zh fixtures", () => {
  it("maps supported/unsupported capability tokens", () => {
    expect(localizeCapabilitySupport("supported", zh)).toBe(zh.onboarding.capabilitySupported);
    expect(localizeCapabilitySupport("unsupported", zh)).toBe(zh.onboarding.capabilityUnsupported);
    expect(localizeCapabilitySupport("supported", en)).toBe("supported");
    expect(localizeCapabilitySupport("unsupported", en)).toBe("unsupported");
    expect(localizeCapabilitySupport("maybe", zh)).toBe("maybe");
  });

  it("maps known degraded codes and falls back for unknown", () => {
    expect(localizeDegradedCode("retrieval_error", zh)).toBe(
      zh.contextPanel.degradedCodeRetrievalError,
    );
    expect(localizeDegradedCode("memory_query_truncated", zh)).toBe(
      zh.contextPanel.degradedCodeMemoryQueryTruncated,
    );
    expect(localizeDegradedCode("identity_budget_exceeded", zh)).toBe(
      zh.contextPanel.degradedCodeIdentityBudgetExceeded,
    );
    expect(localizeDegradedCode("rendered_prompt_omitted", zh)).toBe(
      zh.contextPanel.degradedCodeRenderedPromptOmitted,
    );
    expect(localizeDegradedCode("weird_code", zh)).toBe("weird_code");
  });

  it("onboarding dump shows localized support tokens under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <AgentOnboardingModal
          isOpen
          onClose={vi.fn()}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={{
            success: true,
            latencyMs: 9,
            capabilities: {
              "session.update": "supported",
              "thread.streaming": "unsupported",
            },
          }}
        />
      </I18nProvider>,
    );
    const dump = screen.getByTestId("tested-capabilities").textContent ?? "";
    expect(dump).toContain(zh.onboarding.capabilitySupported);
    expect(dump).toContain(zh.onboarding.capabilityUnsupported);
    expect(dump).not.toMatch(/: supported\b/);
    expect(dump).not.toMatch(/: unsupported\b/);
  });

  it("ContextPanel degraded badge shows zh label for retrieval_error", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ContextPanel
          snapshot={fixtureSnapshot({
            degraded: { code: "retrieval_error", detail: "fts unavailable" },
          })}
        />
      </I18nProvider>,
    );
    const badge = screen.getByTestId("context-degraded-badge");
    expect(badge.textContent).toContain(zh.contextPanel.degradedCodeRetrievalError);
    expect(badge.textContent).not.toContain("retrieval_error");
  });

  it("demo fixture titles and user prompts are Chinese", () => {
    expect(standardThread.title).toBe("契约夹具演练");
    expect(approvalThread.title).toBe("依赖审计（需审批）");
    expect(failureThread.title).toBe("中断的尖峰试跑");
    expect(standardThread.rows[0]?.text).toBe("用三条要点概括 P0.2 IPC 契约。");
    expect(approvalThread.rows[0]?.text).toBe("审计工作区依赖并标出风险项。");
    expect(failureThread.rows[0]?.text).toBe("起草中继尖峰试跑大纲。");
  });
});
