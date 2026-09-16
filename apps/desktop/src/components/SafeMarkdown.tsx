/**
 * Safe Markdown & Rich Content Presentation (A12, DESIGN_I18N §2, F20, F25).
 *
 * Requirements:
 * 1. Safe Markdown subset: Headings, bold, italic, lists, tables, code blocks, inline code.
 * 2. Strict HTML safety: Zero dangerouslySetInnerHTML; raw HTML tags are never executed.
 * 3. Safe URLs: Links only permit `http:` and `https:`; `javascript:`, `data:`, `file:` are neutralized.
 * 4. Code blocks: Language tag, horizontal scroll without layout breaking, dedicated copy button.
 * 5. Tool outputs: Status badge, foldable/collapsible long outputs, copy output affordance.
 * 6. Remote images: Policy-compliant privacy notice placeholder rather than automatic tracking fetches.
 */
import { useState, type ReactNode } from "react";
import styles from "./safeMarkdown.module.css";

export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (
      typeof navigator !== "undefined" &&
      navigator.clipboard &&
      typeof navigator.clipboard.writeText === "function"
    ) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Clipboard permission denied or unavailable
  }
  return false;
}

export interface CodeBlockProps {
  readonly code: string;
  readonly language?: string;
}

export function CodeBlock({ code, language = "text" }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    const ok = await copyToClipboard(code);
    if (ok) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };

  return (
    <div className={styles.codeBlock} data-testid="code-block">
      <div className={styles.codeHeader}>
        <span className={styles.codeLanguage}>{language || "text"}</span>
        <button
          type="button"
          className={styles.copyButton}
          onClick={handleCopy}
          aria-label={copied ? "Copied" : "Copy code"}
          data-testid="copy-code-btn"
        >
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
      <pre className={styles.codeContent} tabIndex={0}>
        <code>{code}</code>
      </pre>
    </div>
  );
}

export interface ToolBlockProps {
  readonly text: string;
  readonly status?: string | null;
  readonly onToggle?: () => void;
}

const TOOL_COLLAPSE_LINE_THRESHOLD = 8;
const TOOL_COLLAPSE_CHAR_THRESHOLD = 400;

export function ToolBlock({ text, status, onToggle }: ToolBlockProps) {
  const [copied, setCopied] = useState(false);
  const [expanded, setExpanded] = useState(false);

  const lines = text.split("\n");
  const isLong =
    lines.length > TOOL_COLLAPSE_LINE_THRESHOLD ||
    text.length > TOOL_COLLAPSE_CHAR_THRESHOLD;

  const displayedText =
    isLong && !expanded
      ? lines.slice(0, TOOL_COLLAPSE_LINE_THRESHOLD).join("\n")
      : text;

  const handleCopy = async () => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };

  const handleToggle = () => {
    setExpanded((prev) => !prev);
    onToggle?.();
  };

  const statusClass =
    status === "completed"
      ? styles.statusCompleted
      : status === "failed"
        ? styles.statusFailed
        : styles.statusRunning;

  return (
    <div className={styles.toolOutputContainer} data-testid="tool-output-block">
      <div className={styles.toolHeader}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--spacing-8)" }}>
          <span>Tool execution</span>
          {status ? (
            <span className={`${styles.toolStatusBadge} ${statusClass}`}>
              {status}
            </span>
          ) : null}
        </div>
        <button
          type="button"
          className={styles.copyButton}
          onClick={handleCopy}
          aria-label={copied ? "Copied output" : "Copy output"}
          data-testid="copy-tool-output-btn"
        >
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
      <pre className={styles.toolText}>
        <code>{displayedText}</code>
      </pre>
      {isLong ? (
        <button
          type="button"
          className={styles.expandToggleBtn}
          onClick={handleToggle}
          data-testid="tool-expand-toggle"
        >
          {expanded
            ? "Collapse output"
            : `Expand full output (+${lines.length - TOOL_COLLAPSE_LINE_THRESHOLD} lines)`}
        </button>
      ) : null}
    </div>
  );
}

function parseInline(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let index = 0;

  // Pattern matches images, links, inline code, bold, italic
  const tokenRegex =
    /(!\[([^\]]*)\]\(([^)]+)\))|(\[([^\]]+)\]\(([^)]+)\))|(`([^`]+)`)|(\*\*([^*]+)\*\*)|(\*([^*]+)\*)/g;

  let match: RegExpExecArray | null;
  while ((match = tokenRegex.exec(text)) !== null) {
    if (match.index > index) {
      nodes.push(text.slice(index, match.index));
    }

    if (match[1] != null) {
      // Remote Image ![alt](url) -> Privacy block notice
      const alt = match[2] || "image";
      const url = match[3] || "";
      const isSafe = url.startsWith("https://") || url.startsWith("http://");
      nodes.push(
        <span key={match.index} className={styles.remoteImageNotice}>
          <span>🖼️ [{alt}]</span>
          {isSafe ? (
            <a
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              className={styles.link}
            >
              (Open image)
            </a>
          ) : null}
        </span>,
      );
    } else if (match[4] != null) {
      // Link [text](url) -> Only allow http/https
      const linkText = match[5] || "";
      const url = match[6] || "";
      const isSafe = url.startsWith("https://") || url.startsWith("http://");
      if (isSafe) {
        nodes.push(
          <a
            key={match.index}
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className={styles.link}
          >
            {linkText}
          </a>,
        );
      } else {
        // Unsafe scheme neutralized into safe plain text
        nodes.push(`${linkText} (${url})`);
      }
    } else if (match[7] != null) {
      // Inline code `code`
      nodes.push(
        <code key={match.index} className={styles.inlineCode}>
          {match[8]}
        </code>,
      );
    } else if (match[9] != null) {
      // Bold **text**
      nodes.push(<strong key={match.index}>{match[10]}</strong>);
    } else if (match[11] != null) {
      // Italic *text*
      nodes.push(<em key={match.index}>{match[12]}</em>);
    }

    index = tokenRegex.lastIndex;
  }

  if (index < text.length) {
    nodes.push(text.slice(index));
  }

  return nodes;
}

export interface SafeMarkdownProps {
  readonly text: string;
  readonly onHeightChange?: () => void;
}

export function SafeMarkdown({ text }: SafeMarkdownProps) {
  // If plain single line without any markdown tokens, render direct text
  if (!text.includes("\n") && !/[#*`|!\[]/.test(text)) {
    return <span className={styles.paragraph}>{text}</span>;
  }

  const elements: ReactNode[] = [];
  const lines = text.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i]!;

    // Fenced code block
    if (line.trim().startsWith("```")) {
      const language = line.trim().slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trim().startsWith("```")) {
        codeLines.push(lines[i]!);
        i++;
      }
      i++; // Skip closing ```
      elements.push(
        <CodeBlock
          key={`code-${i}`}
          code={codeLines.join("\n")}
          language={language}
        />,
      );
      continue;
    }

    // Markdown Table
    if (
      line.trim().startsWith("|") &&
      line.trim().endsWith("|") &&
      i + 1 < lines.length &&
      lines[i + 1]!.includes("---")
    ) {
      const headerCells = line
        .split("|")
        .slice(1, -1)
        .map((c) => c.trim());
      i += 2; // Skip header and separator
      const rows: string[][] = [];
      while (i < lines.length && lines[i]!.trim().startsWith("|") && lines[i]!.trim().endsWith("|")) {
        rows.push(
          lines[i]!
            .split("|")
            .slice(1, -1)
            .map((c) => c.trim()),
        );
        i++;
      }
      elements.push(
        <div key={`table-${i}`} className={styles.tableWrapper}>
          <table className={styles.table}>
            <thead>
              <tr>
                {headerCells.map((cell, cIdx) => (
                  <th key={cIdx}>{parseInline(cell)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rIdx) => (
                <tr key={rIdx}>
                  {row.map((cell, cIdx) => (
                    <td key={cIdx}>{parseInline(cell)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    // Heading 1
    if (line.startsWith("# ")) {
      elements.push(
        <h1 key={`h1-${i}`} className={styles.heading1}>
          {parseInline(line.slice(2))}
        </h1>,
      );
      i++;
      continue;
    }

    // Heading 2
    if (line.startsWith("## ")) {
      elements.push(
        <h2 key={`h2-${i}`} className={styles.heading2}>
          {parseInline(line.slice(3))}
        </h2>,
      );
      i++;
      continue;
    }

    // Heading 3
    if (line.startsWith("### ")) {
      elements.push(
        <h3 key={`h3-${i}`} className={styles.heading3}>
          {parseInline(line.slice(4))}
        </h3>,
      );
      i++;
      continue;
    }

    // Unordered list
    if (/^[-*]\s+/.test(line)) {
      const listItems: string[] = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i]!)) {
        listItems.push(lines[i]!.replace(/^[-*]\s+/, ""));
        i++;
      }
      elements.push(
        <ul key={`ul-${i}`} className={styles.list}>
          {listItems.map((item, idx) => (
            <li key={idx}>{parseInline(item)}</li>
          ))}
        </ul>,
      );
      continue;
    }

    // Ordered list
    if (/^\d+\.\s+/.test(line)) {
      const listItems: string[] = [];
      while (i < lines.length && /^\d+\.\s+/.test(lines[i]!)) {
        listItems.push(lines[i]!.replace(/^\d+\.\s+/, ""));
        i++;
      }
      elements.push(
        <ol key={`ol-${i}`} className={styles.list}>
          {listItems.map((item, idx) => (
            <li key={idx}>{parseInline(item)}</li>
          ))}
        </ol>,
      );
      continue;
    }

    // Blank line
    if (!line.trim()) {
      i++;
      continue;
    }

    // Regular paragraph line
    elements.push(
      <p key={`p-${i}`} className={styles.paragraph}>
        {parseInline(line)}
      </p>,
    );
    i++;
  }

  return (
    <div className={styles.markdownContainer} data-testid="safe-markdown-content">
      {elements}
    </div>
  );
}

