/**
 * Evidence: inspector timeline row.kind + permission decision + ContextPanel
 * drop-reason / why_selected Desktop mappers under zh-CN.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextPanel } from "../components/ContextPanel";
import { TimelineRowView } from "../features/timeline/TimelineRowView";
import { createTimelineStore } from "../features/timeline/timelineStore";
import { I18nProvider, getDictionary } from "../i18n";
import {
  localizeDropReason,
  localizePermissionDecision,
  localizeTimelineRowKind,
  localizeWhySelected,
} from "../i18n/localizeEnums";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";

afterEach(() => {
  cleanup();
});

const zh = getDictionary("zh-CN");

describe("p29 inspector kinds + decisions + drop/why mappers", () => {
  it("maps timeline row kinds to zh-CN labels", () => {
    expect(localizeTimelineRowKind("user-message", zh)).toBe(zh.timeline.you);
    expect(localizeTimelineRowKind("assistant-message", zh)).toBe(zh.timeline.assistant);
    expect(localizeTimelineRowKind("tool", zh)).toBe(zh.timeline.tool);
    expect(localizeTimelineRowKind("permission", zh)).toBe(zh.timeline.approval);
    expect(localizeTimelineRowKind("error", zh)).toBe(zh.timeline.failed);
    expect(localizeTimelineRowKind("unknown", zh)).toBe(zh.timeline.unknown);
    expect(localizeTimelineRowKind("user", zh)).toBe(zh.timeline.you);
    expect(localizeTimelineRowKind("approval", zh)).toBe(zh.timeline.approval);
    expect(localizeTimelineRowKind("failed", zh)).toBe(zh.timeline.failed);
  });

  it("maps permission decisions including allow/deny aliases", () => {
    expect(localizePermissionDecision("approved", zh)).toBe(zh.inspector.decisionApproved);
    expect(localizePermissionDecision("denied", zh)).toBe(zh.inspector.decisionDenied);
    expect(localizePermissionDecision("allow", zh)).toBe(zh.inspector.decisionAllow);
    expect(localizePermissionDecision("deny", zh)).toBe(zh.inspector.decisionDeny);
    expect(localizePermissionDecision("pending", zh)).toBe(zh.inspector.decisionPending);
  });

  it("maps drop reasons and rewrites why_selected protocol fragments", () => {
    expect(localizeDropReason("budget_exhausted", zh)).toBe(
      zh.contextPanel.dropReasonBudgetExhausted,
    );
    expect(localizeDropReason("scope_disallowed", zh)).toBe(
      zh.contextPanel.dropReasonScopeDisallowed,
    );
    const why = localizeWhySelected(
      "matched terms: rust; bm25 0.42; explicit +0.2",
      zh,
    );
    expect(why).toContain(zh.contextPanel.whyFragMatchedTerms);
    expect(why).toContain("rust");
    expect(why).not.toContain("matched terms:");
  });

  it("permission decision chip shows localized label under zh-CN", () => {
    const store = createTimelineStore([
      {
        id: "evt_p29perm000000001",
        kind: "permission",
        text: "cargo tree",
        status: null,
        permission: {
          requestedAction: "cargo tree",
          scope: "project:altior",
          decision: "approved",
        },
        streaming: false,
      },
    ]);
    render(
      <I18nProvider localeSource="zh-CN" onLocaleSourceChange={() => undefined}>
        <TimelineRowView
          store={store}
          rowId="evt_p29perm000000001"
          index={0}
          focused={false}
          onFocus={vi.fn()}
          onPermissionDecision={vi.fn()}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("permission-decision-chip")).toHaveTextContent(
      zh.inspector.decisionApproved,
    );
  });

  it("ContextPanel shows localized drop reason and why_selected under zh-CN", () => {
    const snapshot = {
      turn_id: "trn_p29ctx00000000000000001",
      thread_id: "thr_p29ctx00000000000000001",
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
      memories: [
        {
          memory_id: "mem_p29ctx00000000000000001",
          kind: "fact",
          scope_kind: "global",
          scope_target: null,
          confidence: 90,
          explicit: true,
          tokens: 10,
          score: 0.5,
          why_selected: "matched terms: alpha; bm25 1.0; explicit +0.1",
          provenance_thread_id: null,
          provenance_turn_id: null,
        },
      ],
      dropped: [
        {
          memory_id: "mem_p29ctx00000000000000002",
          tokens: 99,
          rank: 2,
          reason: "budget_exhausted",
        },
      ],
      degraded: null,
      rendered_prompt: null,
    } as ContextSnapshotDto;

    render(
      <I18nProvider localeSource="zh-CN" onLocaleSourceChange={() => undefined}>
        <ContextPanel snapshot={snapshot} status="loaded" />
      </I18nProvider>,
    );
    expect(screen.getByTestId("dropped-reason")).toHaveTextContent(
      zh.contextPanel.dropReasonBudgetExhausted,
    );
    expect(screen.getByTestId("memory-why-selected")).toHaveTextContent(
      zh.contextPanel.whyFragMatchedTerms,
    );
  });
});
