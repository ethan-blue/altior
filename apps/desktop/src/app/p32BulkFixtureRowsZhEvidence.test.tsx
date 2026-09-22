/**
 * Evidence: bulk synthetic fixture rows + streamingReplyChunks zh-CN chrome.
 * Tool/permission CLI strings (`rg`, `cargo tree`) stay as intentional protocol
 * code spans rendered via ToolBlock / mono permission boxes.
 */
import { describe, expect, it } from "vitest";
import {
  approvalThread,
  hundredThousandRowThread,
  olderHistory,
  standardThread,
  streamingReplyChunks,
} from "../fixtures/timeline";

describe("p32 bulk synthetic fixture rows zh-CN", () => {
  it("hundredThousandRowThread user/assistant/tool bodies are Chinese", () => {
    const huge = hundredThousandRowThread();
    expect(huge.rows[0]?.text).toBe("确定性提问 0");
    expect(huge.rows[1]?.text).toBe("关于契约的确定性回答 1。");
    const toolRow = huge.rows.find((r) => r.kind === "tool");
    expect(toolRow?.text).toMatch(/^扫描批次 \d+$/);
    expect(huge.rows.some((r) => /deterministic question|scan batch/.test(r.text))).toBe(
      false,
    );
  });

  it("olderHistory prepend rows are Chinese", () => {
    const rows = olderHistory(4);
    expect(rows[0]?.text).toMatch(/^更早的提问 \d+$/);
    expect(rows[1]?.text).toMatch(/^更早的回答 \d+。$/);
    expect(rows.every((r) => !/older (question|answer)/.test(r.text))).toBe(true);
  });

  it("streamingReplyChunks join to zh product chrome; App.test couples to fixture", () => {
    const joined = streamingReplyChunks.join("");
    expect(joined).toContain("帧为长度前缀");
    expect(joined).toContain("重载不会停止轮次");
    expect(joined).not.toMatch(/Frames are|length-prefixed/);
  });

  it("demo tool/permission CLI strings remain intentional protocol spans", () => {
    const tool = standardThread.rows.find((r) => r.kind === "tool");
    expect(tool?.text).toBe("rg --files crates/altior-ipc");
    const aprTool = approvalThread.rows.find((r) => r.kind === "tool");
    expect(aprTool?.text).toBe("cargo tree --workspace");
    const perm = approvalThread.rows.find((r) => r.kind === "permission");
    expect(perm?.permission?.requestedAction).toBe(
      "cargo tree --workspace --edges all",
    );
  });
});
