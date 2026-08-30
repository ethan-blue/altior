/**
 * The only Tauri-facing layer (docs/UI_ARCHITECTURE.md frontend
 * architecture; ADR 0008 §6). Feature code never imports Tauri APIs;
 * `withGlobalTauri: false` keeps them out of the page entirely, and the
 * capability allowlist is pinned here so capability creep fails CI.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const srcTauri = join(__dirname, "..", "..", "..", "src-tauri");
const repoRoot = join(srcTauri, "..", "..", "..");

const config = JSON.parse(
  readFileSync(join(srcTauri, "tauri.conf.json"), "utf8"),
) as Record<string, any>;
const capabilities = JSON.parse(
  readFileSync(join(srcTauri, "capabilities", "default.json"), "utf8"),
) as Record<string, any>;
const workspaceManifest = readFileSync(join(repoRoot, "Cargo.toml"), "utf8");

describe("Tauri shell capability minimums (ADR 0008 §6)", () => {
  it("declares exactly one main window", () => {
    const windows = config.app?.windows ?? [];
    expect(windows.map((window: any) => window.label)).toEqual(["main"]);
  });

  it("exposes no global Tauri API", () => {
    expect(config.app?.withGlobalTauri).toBe(false);
  });

  it("ships a strict CSP without unsafe-eval or wildcard script sources", () => {
    const csp: string = config.app?.security?.csp ?? "";
    expect(csp).toContain("default-src 'self'");
    expect(csp).not.toContain("unsafe-eval");
    expect(csp).not.toContain("script-src *");
  });

  it("grants only the declared minimum permission set", () => {
    expect(capabilities.identifier).toBe("default");
    expect(capabilities.windows).toEqual(["main"]);
    expect(capabilities.permissions).toEqual(["core:default"]);
  });

  it("keeps the shell out of the Rust workspace so repo gates stay hermetic", () => {
    expect(workspaceManifest).not.toContain("src-tauri");
    // And the shell crate declares its own (empty) workspace root.
    const shellManifest = readFileSync(join(srcTauri, "Cargo.toml"), "utf8");
    expect(shellManifest).toContain("[workspace]");
  });
});
