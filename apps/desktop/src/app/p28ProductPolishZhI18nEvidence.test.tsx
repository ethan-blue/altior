/**
 * Evidence: continuous product polish — StatusBar fallbacks, ContextPanel chrome,
 * diagnostics/tool badges, onboarding capability dump under zh-CN.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextPanel } from "../components/ContextPanel";
import { ToolBlock } from "../components/SafeMarkdown";
import {
  AgentOnboardingModal,
  RuntimeDiagnosticsView,
  StatusBar,
  ThreadsPane,
} from "../components/shell";
import { I18nProvider, getDictionary } from "../i18n";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";

afterEach(() => {
  cleanup();
});

function fixtureSnapshot(): ContextSnapshotDto {
  return {
    turn_id: "trn_p28panel00000000000000001",
    thread_id: "thr_p28panel00000000000000001",
    memory_mode: "long_term",
    created_at: 2000,
    passthrough: false,
    budget: {
      identity_limit_tokens: 512,
      memory_limit_tokens: 1024,
      prompt_tokens: 24,
      identity_tokens: 40,
      memory_tokens: 88,
      total_tokens: 152,
    },
    identity: [{ document_id: "idt_p28panel00000000000000001", kind: "preference", tokens: 40 }],
    memories: [
      {
        memory_id: "mem_p28panel00000000000000001",
        kind: "preference",
        scope_kind: "global",
        scope_target: null,
        confidence: 100,
        explicit: true,
        tokens: 88,
        score: 0.92,
        why_selected: "matched terms: rust",
        provenance_thread_id: "thr_p28origin0000000000000001",
        provenance_turn_id: null,
      },
    ],
    dropped: [
      {
        memory_id: "mem_p28panel00000000000000002",
        tokens: 6000,
        rank: 2,
        reason: "budget_exhausted",
      },
    ],
    degraded: null,
    rendered_prompt: null,
  } as ContextSnapshotDto;
}

describe("p28 continuous product polish zh-CN i18n", () => {
  it("StatusBar unknown thread/stream fallbacks are Chinese", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <StatusBar coreState="weird-core" threadStatus="weird-thread" streamState="weird-stream" />
      </I18nProvider>,
    );
    const bar = screen.getByTestId("status-bar");
    expect(bar.textContent).toContain("核心 · weird-core");
    expect(bar.textContent).toContain("会话 · weird-thread");
    expect(bar.textContent).toContain("传输 · weird-stream");
  });

  it("thread list status labels are Chinese under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ThreadsPane
          threads={[{ id: "thr_p28", title: "demo", agent: "agent", status: "running", pinned: false }]}
          selectedThreadId=""
          onSelect={vi.fn()}
          filter=""
          onFilterChange={vi.fn()}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("thread-thr_p28").textContent).toContain("运行中");
  });

  it("ContextPanel tokens/score/provenance/dropped meta and kind badge are Chinese", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ContextPanel snapshot={fixtureSnapshot()} status="loaded" />
      </I18nProvider>,
    );
    expect(screen.getByText("词元 / 得分")).toBeInTheDocument();
    expect(screen.getByText(/88 词元 · 总分/)).toBeInTheDocument();
    expect(screen.getByText(/排名 #2 · 6000 词元/)).toBeInTheDocument();
    expect(screen.getByText(/40 \/ 512 词元/)).toBeInTheDocument();
    const provenance = screen.getByTestId("memory-provenance");
    expect(provenance.textContent).toContain("会话 ID");
    expect(provenance.textContent).toContain("轮次 ID");
    expect(provenance.textContent).toContain("未知");
    expect(provenance.textContent).toContain("摘录");
    expect(screen.getAllByText("偏好").length).toBeGreaterThan(0);
  });

  it("diagnostics refresh + runtime status badge are Chinese", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <RuntimeDiagnosticsView
          diagnostics={{ instance_id: "core_p28", status: "ready", active_threads: 1, active_turns: 0, summary: null }}
          status="loaded"
          onRefresh={vi.fn()}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("diag-status").textContent).toContain("就绪");
    expect(screen.getByTestId("diag-refresh-btn")).toHaveTextContent("刷新");
  });

  it("tool status badge is Chinese under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ToolBlock text={"ok\n".repeat(2)} status="completed" />
      </I18nProvider>,
    );
    expect(screen.getByTestId("tool-status-badge")).toHaveTextContent("已完成");
  });

  it("onboarding capability dump prefix is Chinese", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <AgentOnboardingModal
          isOpen
          onClose={vi.fn()}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={{ success: true, latencyMs: 12, capabilities: { "session.update": "supported" } }}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("tested-capabilities").textContent).toContain("能力：");
    expect(screen.getByTestId("tested-capabilities").textContent).toContain("session.update");
  });

  it("dictionary keys for continuous polish exist in both locales", () => {
    const zh = getDictionary("zh-CN");
    const en = getDictionary("en");
    expect(zh.inspector.runtimeReady).toBe("就绪");
    expect(zh.contextPanel.tokensFraction(1, 2)).toContain("词元");
    expect(zh.workbenchEmpty.coreDisconnected).toContain("本地服务");
    expect(zh.statusBar.threadLine("x")).toBe("会话 · x");
    expect(en.statusBar.streamLine("x")).toBe("Stream · x");
    expect(zh.markdown.toolStatusCompleted).toBe("已完成");
    expect(zh.onboarding.capabilitiesSummary("a: b")).toBe("能力：a: b");
    expect(zh.threadStatus.running).toBe("运行中");
  });
});
