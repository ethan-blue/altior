/**
 * Evidence: identity document kinds outside memory kinds + zh demo fixture
 * assistant/error chrome; streamingReplyChunks are zh and App.test couples to the fixture.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ContextPanel } from "../components/ContextPanel";
import {
  approvalThread,
  failureThread,
  standardThread,
  streamingReplyChunks,
} from "../fixtures/timeline";
import { I18nProvider, getDictionary } from "../i18n";
import { localizeIdentityKind, localizeMemoryKind } from "../i18n/localizeEnums";
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
    turn_id: "trn_p31ident00000000000000001",
    thread_id: "thr_p31ident00000000000000001",
    memory_mode: "long_term",
    created_at: 1,
    passthrough: false,
    budget: {
      identity_limit_tokens: 100,
      memory_limit_tokens: 200,
      prompt_tokens: 1,
      identity_tokens: 40,
      memory_tokens: 1,
      total_tokens: 42,
    },
    identity: [
      { document_id: "idt_p31name00000000000000001", kind: "name", tokens: 10 },
      { document_id: "idt_p31about0000000000000001", kind: "about", tokens: 12 },
      { document_id: "idt_p31pref00000000000000001", kind: "preference", tokens: 8 },
      { document_id: "idt_p31inst00000000000000001", kind: "instruction", tokens: 10 },
    ],
    memories: [],
    dropped: [],
    degraded: null,
    rendered_prompt: null,
    ...overrides,
  };
}

describe("p31 identity kinds + fixture assistant/error chrome", () => {
  it("maps identity kinds; name/about are outside memory kinds", () => {
    expect(localizeIdentityKind("name", zh)).toBe(zh.contextPanel.identityKindName);
    expect(localizeIdentityKind("about", zh)).toBe(zh.contextPanel.identityKindAbout);
    expect(localizeIdentityKind("preference", zh)).toBe(zh.memoryPane.kindPreference);
    expect(localizeIdentityKind("instruction", zh)).toBe(zh.memoryPane.kindInstruction);
    expect(localizeIdentityKind("name", en)).toBe("Name");
    expect(localizeIdentityKind("about", en)).toBe("About");
    expect(localizeIdentityKind("weird", zh)).toBe("weird");

    // Memory mapper still leaves identity-only kinds raw.
    expect(localizeMemoryKind("name", zh)).toBe("name");
    expect(localizeMemoryKind("about", zh)).toBe("about");
  });

  it("ContextPanel identity badges show zh labels not raw name/about", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ContextPanel snapshot={fixtureSnapshot()} />
      </I18nProvider>,
    );
    const badges = screen.getAllByTestId("identity-kind-badge").map((el) => el.textContent);
    expect(badges).toContain(zh.contextPanel.identityKindName);
    expect(badges).toContain(zh.contextPanel.identityKindAbout);
    expect(badges).toContain(zh.memoryPane.kindPreference);
    expect(badges).toContain(zh.memoryPane.kindInstruction);
    expect(badges.join("|")).not.toMatch(/\bname\b/);
    expect(badges.join("|")).not.toContain("about");
  });

  it("demo fixture assistant/error/streaming chrome is Chinese", () => {
    const stdAssistant = standardThread.rows.find((r) => r.kind === "assistant-message");
    const aprAssistant = approvalThread.rows.find((r) => r.kind === "assistant-message");
    const faiAssistant = failureThread.rows.find((r) => r.kind === "assistant-message");
    const errors = failureThread.rows.filter((r) => r.kind === "error");
    const approved = standardThread.rows.find((r) => r.id === "trn_fixture000000103");
    const unknown = standardThread.rows.find((r) => r.kind === "unknown");

    expect(stdAssistant?.text).toContain("帧为");
    expect(aprAssistant?.text).toContain("等待");
    expect(faiAssistant?.text).toContain("中继");
    expect(errors[0]?.text).toContain("轮次已停止");
    expect(errors[1]?.text).toContain("投递");
    expect(approved?.text).toBe("[已批准]");
    expect(unknown?.text).toContain("协议 v1");

    expect(streamingReplyChunks.join("")).toContain("帧为长度前缀");
  });
});
