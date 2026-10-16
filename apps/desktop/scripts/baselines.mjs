/**
 * Visual baseline capture (ADR 0008 §7).
 *
 * Drives the synthetic fixture shell in a real browser and captures
 * the six pinned screenshots: light, dark, narrow, error, approval, memory.
 *
 * Quality gates enforced:
 * - Waits for [data-fixture-ready="true"]
 * - Waits for document.fonts.ready
 * - Bypasses arbitrary sleep in favor of deterministic DOM states
 * - Traps unhandled pageerror and console.error
 * - Guaranteed server and browser cleanup in try/finally
 *
 * Usage: npm run baselines   (in apps/desktop)
 */
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(here, "..", "baselines");

// Must match check-visual.mjs VISUAL_CONTEXT_DEFAULTS: baselines are captured
// under one pinned locale/timezone so cross-machine re-runs do not diff.
const VISUAL_CONTEXT_DEFAULTS = { locale: "zh-CN", timezoneId: "UTC" };

// ALTIOR_VISUAL_PORT lets parallel checkouts (scripts/task-pack) avoid port clashes.
const port = Number(process.env.ALTIOR_VISUAL_PORT) || 4173;
const server = await createServer({
  root: resolve(here, ".."),
  server: { port, strictPort: true },
});
await server.listen();
const address = server.resolvedUrls?.local?.[0] ?? `http://localhost:${port}`;
console.log(`serving ${address} for visual baselines`);

let browser;
try {
  browser = await chromium.launch();
  await mkdir(outDir, { recursive: true });

  async function capture(name, { viewport, init, drive }) {
    const context = await browser.newContext({ viewport, ...VISUAL_CONTEXT_DEFAULTS });
    const page = await context.newPage();

    if (init) await init(page);

    page.on("pageerror", (err) => {
      console.error(`[PageError in ${name}]`, err);
      throw err;
    });

    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error(`[ConsoleError in ${name}]`, msg.text());
        throw new Error(`Console error in ${name}: ${msg.text()}`);
      }
    });

    await page.goto(address, { waitUntil: "networkidle" });
    await page.waitForSelector("[data-fixture-ready='true']", { timeout: 15_000 });
    await page.evaluate(() => document.fonts.ready);

    if (drive) await drive(page);

    await page.screenshot({ path: resolve(outDir, `${name}.png`) });
    await context.close();
    console.log(`captured ${name}.png`);
  }

  await capture("light", { viewport: { width: 1280, height: 800 } });
  await capture("dark", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='theme-toggle']");
      await page.waitForSelector("[data-theme='dark']");
    },
  });
  await capture("narrow", { viewport: { width: 760, height: 800 } });
  await capture("error", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='thread-thr_fixture000000003']");
      await page.waitForSelector("[data-row-id]");
    },
  });
  await capture("approval", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='thread-thr_fixture000000002']");
      await page.waitForSelector("[data-testid='approve']");
    },
  });
  await capture("memory", {
    viewport: { width: 1280, height: 800 },
    init: async (page) => {
      await page.addInitScript(() => {
        const fixedNow = 1_700_000_000_000;
        Date.now = () => fixedNow;
      });
    },
    drive: async (page) => {
      await page.click("[data-testid='rail-memory']");
      await page.click("[data-testid='memory-add-button']");
      await page.fill("[data-testid='memory-content-input']", "User prefers a compact local-first workbench");
      await page.click("[data-testid='memory-submit-btn']");
      await page.waitForSelector("[data-testid^='memory-card-'][data-state]");
      await page.click("[data-testid^='memory-card-'][data-state]");
      await page.waitForSelector("[data-testid='inspector-memory-details']");
    },
  });

  console.log(`baselines written to ${outDir}`);
} finally {
  if (browser) await browser.close();
  await server.close();
}
