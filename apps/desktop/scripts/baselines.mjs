/**
 * Visual baseline capture (ADR 0008 §7).
 *
 * Drives the synthetic fixture shell in a real browser and captures
 * the five pinned screenshots: light, dark, narrow, error, approval.
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

const server = await createServer({
  root: resolve(here, ".."),
  server: { port: 4173, strictPort: true },
});
await server.listen();
const address = server.resolvedUrls?.local?.[0] ?? "http://localhost:4173";
console.log(`serving ${address} for visual baselines`);

let browser;
try {
  browser = await chromium.launch();
  await mkdir(outDir, { recursive: true });

  async function capture(name, { viewport, drive }) {
    const context = await browser.newContext({ viewport });
    const page = await context.newPage();

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

  console.log(`baselines written to ${outDir}`);
} finally {
  if (browser) await browser.close();
  await server.close();
}
