/**
 * Format and whitespace checker for Altior Desktop.
 *
 * Enforces:
 * 1. No trailing whitespace at end of lines.
 * 2. File ends with a newline.
 * 3. Indentation uses spaces, not tabs.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const srcDir = path.resolve(here, "..", "src");

const violations = [];

function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full);
    } else if (entry.isFile() && /\.(ts|tsx|css)$/.test(entry.name)) {
      checkFile(full);
    }
  }
}

function checkFile(filePath) {
  const rel = path.relative(srcDir, filePath).replace(/\\/g, "/");
  const raw = fs.readFileSync(filePath, "utf8");

  if (!raw.endsWith("\n")) {
    violations.push({
      file: rel,
      rule: "trailing-newline-missing",
      message: "File does not end with a newline character",
    });
  }

  const lines = raw.split("\n");
  // If ends with \n, last element is empty string
  const checkLines = raw.endsWith("\n") ? lines.slice(0, -1) : lines;

  checkLines.forEach((line, idx) => {
    if (/[ \t]+$/.test(line)) {
      violations.push({
        file: rel,
        line: idx + 1,
        rule: "no-trailing-whitespace",
        message: "Trailing whitespace detected",
      });
    }
    if (/^\t+/.test(line)) {
      violations.push({
        file: rel,
        line: idx + 1,
        rule: "no-tabs",
        message: "Tab indentation detected instead of spaces",
      });
    }
  });
}

walk(srcDir);

if (violations.length > 0) {
  console.error(`=== Format Check Failed: ${violations.length} violations found ===`);
  for (const v of violations.slice(0, 20)) {
    console.error(`[${v.file}${v.line ? ":" + v.line : ""}] (${v.rule}): ${v.message}`);
  }
  if (violations.length > 20) {
    console.error(`... and ${violations.length - 20} more violations`);
  }
  process.exit(1);
} else {
  console.log("=== Format Check Passed: All source files clean ===");
}
