/**
 * Visual baseline capture (ADR 0008 §7).
 *
 * Drives the same synthetic fixture shell in a real browser and captures
 * the five pinned screenshots: light, dark, narrow, error, approval.
 * The images are operator-reviewed evidence, not image-diff gates (the
 * visual regression audit is P5).
 *
 * Usage: npm run baselines   (in apps/desktop)
 */
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { preview } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(here, "..", "baselines");

const server = await preview({
  root: resolve(here, ".."),
  preview: { port: 4173, strictPort: true },
});
const address =
  server.urls?.local ?? server.url ?? "http://localhost:4173";
console.log(`serving ${address}`);

const browser = await chromium.launch();
await mkdir(outDir, { recursive: true });

async function capture(name, { viewport, drive }) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  await page.goto(address, { waitUntil: "networkidle" });
  await page.waitForSelector("[data-row-id]", { timeout: 10_000 });
  if (drive) await drive(page);
  await page.waitForTimeout(250); // let streaming coalescing settle
  await page.screenshot({ path: resolve(outDir, `${name}.png`) });
  await context.close();
  console.log(`captured ${name}.png`);
}

await capture("light", { viewport: { width: 1280, height: 800 } });
await capture("dark", {
  viewport: { width: 1280, height: 800 },
  drive: (page) => page.click("[data-testid='theme-toggle']"),
});
await capture("narrow", { viewport: { width: 760, height: 800 } });
await capture("error", {
  viewport: { width: 1280, height: 800 },
  drive: (page) => page.click("[data-testid='thread-fixture/failure']"),
});
await capture("approval", {
  viewport: { width: 1280, height: 800 },
  drive: (page) => page.click("[data-testid='thread-fixture/approval']"),
});

await browser.close();
await server.close();
console.log(`baselines written to ${outDir}`);
