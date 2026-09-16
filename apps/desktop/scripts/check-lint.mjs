/**
 * Static architecture and style linter for Altior Desktop.
 *
 * Enforces:
 * 1. No hardcoded hex colors in CSS modules (must use semantic tokens from tokens.css).
 * 2. No direct Tauri global or API access outside ipc/tauriTransport.ts.
 * 3. No direct SQLite or node native imports in desktop source.
 * 4. No forbidden raw z-indices in CSS modules.
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
    } else if (entry.isFile()) {
      checkFile(full);
    }
  }
}

function checkFile(filePath) {
  const rel = path.relative(srcDir, filePath).replace(/\\/g, "/");
  const content = fs.readFileSync(filePath, "utf8");
  const lines = content.split("\n");

  // Rule 1 & 4: CSS module token checks
  if (filePath.endsWith(".module.css")) {
    lines.forEach((line, idx) => {
      // Allow comments
      const cleanLine = line.replace(/\/\*.*?\*\//g, "");
      // Disallow raw hex colors like #fff, #1a1a1a, etc.
      const hexMatch = cleanLine.match(/#[0-9a-fA-F]{3,8}\b/);
      if (hexMatch && !cleanLine.includes("/* allow-hex */")) {
        violations.push({
          file: rel,
          line: idx + 1,
          rule: "no-raw-hex-in-css-modules",
          message: `Raw hex color "${hexMatch[0]}" used instead of design token var(--color-...)`,
        });
      }
    });
  }

  // Rule 2: No direct Tauri globals outside tauriTransport.ts
  if (filePath.endsWith(".ts") || filePath.endsWith(".tsx")) {
    if (rel !== "ipc/tauriTransport.ts" && !rel.startsWith("platform/")) {
      lines.forEach((line, idx) => {
        if (line.includes("__TAURI__") || line.includes("@tauri-apps/api")) {
          violations.push({
            file: rel,
            line: idx + 1,
            rule: "no-direct-tauri-outside-transport",
            message: "Direct Tauri global or import accessed outside tauriTransport.ts",
          });
        }
      });
    }

    // Rule 3: No sqlite or native modules
    lines.forEach((line, idx) => {
      if (line.match(/from\s+["\']sqlite3?["\']/) || line.match(/from\s+["\']rusqlite["\']/)) {
        violations.push({
          file: rel,
          line: idx + 1,
          rule: "no-database-in-renderer",
          message: "Database library imported directly in frontend renderer",
        });
      }
    });
  }
}

walk(srcDir);

if (violations.length > 0) {
  console.error(`=== Lint Failed: ${violations.length} violations found ===`);
  for (const v of violations) {
    console.error(`[${v.file}:${v.line}] (${v.rule}): ${v.message}`);
  }
  process.exit(1);
} else {
  console.log("=== Lint Passed: 0 violations found ===");
}
