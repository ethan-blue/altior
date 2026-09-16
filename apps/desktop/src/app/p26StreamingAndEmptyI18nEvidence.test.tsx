/**
 * Evidence: streaming markdown chrome and workbench empty/disconnected
 * copy are locale-aware (zh-CN / en).
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CodeBlock, ToolBlock } from "../components/SafeMarkdown";
import { I18nProvider } from "../i18n";
import { getDictionary } from "../i18n";

afterEach(() => {
  cleanup();
});

describe("p26 streaming markdown + workbench empty i18n", () => {
  it("code copy affordances render Chinese under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <CodeBlock code="fn main() {}" language="rust" />
      </I18nProvider>,
    );
    const btn = screen.getByTestId("copy-code-btn");
    expect(btn).toHaveTextContent("复制");
    expect(btn).toHaveAttribute("aria-label", "复制代码");
  });

  it("tool expand/collapse chrome is Chinese under zh-CN", () => {
    const longOutput = Array.from({ length: 20 }, (_, i) => `line ${i}`).join("\n");
    render(
      <I18nProvider localeSource="zh-CN">
        <ToolBlock text={longOutput} status="completed" />
      </I18nProvider>,
    );
    expect(screen.getByText("工具执行")).toBeInTheDocument();
    const toggle = screen.getByTestId("tool-expand-toggle");
    expect(toggle.textContent).toMatch(/展开完整输出/);
    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("收起输出");
  });

  it("workbench empty dictionary covers connecting / disconnected / empty vault", () => {
    const zh = getDictionary("zh-CN").workbenchEmpty;
    const en = getDictionary("en").workbenchEmpty;
    expect(zh.connecting).toContain("连接");
    expect(zh.coreDisconnected).toContain("断开");
    expect(zh.noConversations).toContain("暂无会话");
    expect(zh.selectConversation).toContain("选择");
    expect(en.coreDisconnected).toBe("Core disconnected");
    expect(en.noConversations).toContain("No conversations yet");
  });
});
