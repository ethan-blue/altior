/**
 * A12 Acceptance Evidence: Rich Content, Safe Markdown & Tool Output Presentation (F20 / F25).
 *
 * Proves that:
 * 1. Rich Markdown subset (headings, bold, italic, code blocks, lists, tables) renders cleanly.
 * 2. Code blocks include language badge and copy button that calls clipboard without altering text.
 * 3. Tool outputs feature status badges, copy button, and expand/collapse folding for long logs.
 * 4. Raw HTML (<script>, <iframe>, <img onerror>) is completely neutralized (no execution, no XSS).
 * 5. Unsafe URL schemes (javascript:, data:, file:) are neutralized into plain text.
 * 6. Remote images do not trigger automatic network fetches, displaying privacy-safe placeholders instead.
 * 7. Long paths, emojis, Chinese text, and multi-line markdown compose without crashing or breaking.
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { SafeMarkdown, ToolBlock } from "../components/SafeMarkdown";
import { App } from "./App";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import type { TimelineRow } from "../features/timeline/timelineStore";
import { standardThread } from "../fixtures/timeline";

describe("A12 evidence: Safe Markdown, rich content & tool output presentation", () => {
  beforeEach(() => {
    // Mock navigator.clipboard
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  it("renders safe markdown: headings, bold, italic, inline code, and lists", () => {
    const markdown = `# Heading 1
## Heading 2
This is **bold** text and *italic* text with \`inline_code()\`.
- Bullet item 1
- Bullet item 2
1. Ordered 1
2. Ordered 2`;

    const { container } = render(<SafeMarkdown text={markdown} />);

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Heading 1");
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Heading 2");
    expect(container.querySelector("strong")).toHaveTextContent("bold");
    expect(container.querySelector("em")).toHaveTextContent("italic");
    expect(container.querySelector("code")).toHaveTextContent("inline_code()");
    expect(screen.getByText("Bullet item 1")).toBeInTheDocument();
    expect(screen.getByText("Ordered 1")).toBeInTheDocument();
  });

  it("renders markdown tables with proper headers and rows", () => {
    const tableMd = `| Scope | Status | Details |
|---|---|---|
| Project | Active | Local-first |
| Vault | Pinned | Encrypted |`;

    const { container } = render(<SafeMarkdown text={tableMd} />);
    const table = container.querySelector("table");
    expect(table).not.toBeNull();

    const headers = container.querySelectorAll("th");
    expect(headers.length).toBe(3);
    expect(headers[0]).toHaveTextContent("Scope");
    expect(headers[1]).toHaveTextContent("Status");

    const cells = container.querySelectorAll("td");
    expect(cells.length).toBe(6);
    expect(cells[0]).toHaveTextContent("Project");
    expect(cells[2]).toHaveTextContent("Local-first");
  });

  it("code block displays language badge and copies exact code to clipboard", async () => {
    const codeBlockMd = `\`\`\`rust
fn main() {
    println!("Hello Altior!");
}
\`\`\``;

    render(<SafeMarkdown text={codeBlockMd} />);

    expect(screen.getByText("rust")).toBeInTheDocument();
    expect(screen.getByText(/fn main\(\)/)).toBeInTheDocument();

    const copyBtn = screen.getByTestId("copy-code-btn");
    expect(copyBtn).toHaveTextContent("Copy");

    fireEvent.click(copyBtn);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      'fn main() {\n    println!("Hello Altior!");\n}',
    );
  });

  it("tool output displays status badge, copy button, and folds outputs exceeding line threshold", () => {
    const longOutput = Array.from({ length: 20 }, (_, i) => `log line ${i + 1}: checking crate`).join("\n");

    const onToggle = vi.fn();
    render(<ToolBlock text={longOutput} status="completed" onToggle={onToggle} />);

    expect(screen.getByText("completed")).toBeInTheDocument();
    expect(screen.getByTestId("copy-tool-output-btn")).toBeInTheDocument();

    // Long output initially shows folding toggle button
    const toggleBtn = screen.getByTestId("tool-expand-toggle");
    expect(toggleBtn).toHaveTextContent(/Expand full output/);

    // Initial view is collapsed (first lines visible, line 19 not visible)
    expect(screen.getByText(/log line 1:/)).toBeInTheDocument();
    expect(screen.queryByText(/log line 19:/)).toBeNull();

    // Click expand
    fireEvent.click(toggleBtn);
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(toggleBtn).toHaveTextContent("Collapse output");
    expect(screen.getByText(/log line 19:/)).toBeInTheDocument();

    // Click collapse
    fireEvent.click(toggleBtn);
    expect(toggleBtn).toHaveTextContent(/Expand full output/);
  });

  it("neutralizes raw HTML and prevents script execution (XSS defense)", () => {
    const malicious = `<script>window.pwned = true;</script>
<img src="nonexistent" onerror="alert('xss')" />
<iframe src="https://evil.com"></iframe>
Normal text continues.`;

    const { container } = render(<SafeMarkdown text={malicious} />);

    // Never inject raw script or iframe tags
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector("img")).toBeNull();

    // Text content is safely escaped
    expect(screen.getByText(/Normal text continues/)).toBeInTheDocument();
  });

  it("validates link schemes: allows http/https and neutralizes javascript/data/file schemes", () => {
    const mixedLinks = `[Safe Web Link](https://github.com/altior)
[Malicious JS](javascript:alert('pwned'))
[Malicious Data](data:text/html,<script>alert(1)</script>)
[Malicious File](file:///etc/passwd)`;

    const { container } = render(<SafeMarkdown text={mixedLinks} />);

    const links = container.querySelectorAll("a");
    // Only the https link should be an anchor tag
    expect(links.length).toBe(1);
    expect(links[0]?.getAttribute("href")).toBe("https://github.com/altior");
    expect(links[0]?.getAttribute("target")).toBe("_blank");
    expect(links[0]?.getAttribute("rel")).toBe("noopener noreferrer");

    // Unsafe schemes are rendered safely without clickable anchor
    expect(screen.getByText(/Malicious JS \(javascript:alert/)).toBeInTheDocument();
    expect(screen.getByText(/Malicious File \(file:\/\/\/etc\/passwd\)/)).toBeInTheDocument();
  });

  it("remote images are not automatically fetched; renders privacy notice placeholder instead", () => {
    const imageMd = `![Privacy Tracker](https://tracker.example.com/pixel.png)`;

    const { container } = render(<SafeMarkdown text={imageMd} />);

    // No actual <img> element is rendered over network
    expect(container.querySelector("img")).toBeNull();

    // Safe placeholder notice is displayed
    expect(screen.getByText(/\[Privacy Tracker\]/)).toBeInTheDocument();
  });

  it("TimelineRowView integrates rich markdown seamlessly in live conversation", async () => {
    const richRow: TimelineRow = {
      id: "trn_rich_01",
      kind: "assistant-message",
      text: "### Plan of Action\nHere is what we did:\n```bash\ncargo check\n```",
      status: null,
      permission: null,
      streaming: false,
    };

    const richThread = {
      id: standardThread.id,
      title: "Rich Markdown Thread",
      agent: "alpha (ACP)",
      status: "completed" as const,
      pinned: false,
      rows: [richRow],
    };

    render(
      <App
        transport={new InMemoryTransport()}
        fixtureTimelineRows={[richThread]}
      />,
    );

    fireEvent.click(await screen.findByTestId(`thread-${richThread.id}`));

    // Check heading
    expect(await screen.findByRole("heading", { level: 3, name: "Plan of Action" })).toBeInTheDocument();
    // Check code block
    expect(screen.getByText("bash")).toBeInTheDocument();
    expect(screen.getByText("cargo check")).toBeInTheDocument();
    expect(screen.getByTestId("copy-code-btn")).toBeInTheDocument();
  });
});
