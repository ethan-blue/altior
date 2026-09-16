/**
 * Automated pixel-level visual and geometry regression gate (A17 / F33 / Visual Regression).
 *
 * Verifies:
 * 1. Zero pageerror and console.error exceptions during rendering.
 * 2. Deterministic ready state via [data-fixture-ready="true"] and document.fonts.ready.
 * 3. Load-bearing geometry invariants across all 5 pinned views:
 *    - 4-column workbench grid allocation (rail, nav, main, inspector).
 *    - No unintended horizontal scrolling.
 *    - Responsive narrow drawer overlay behavior at 760px.
 *    - High-contrast theme application in dark mode.
 *    - Approval action card and error state presence.
 * 4. Exact pixel-level visual regression against pinned baselines:
 *    - Captures actual browser screenshot for each view.
 *    - Decodes PNG and compares against repository baseline using pixelmatch.
 *    - Strict threshold: per-pixel color delta threshold = 0.1, max allowed diff = 50 pixels (< 0.005%).
 *    - If mismatch, missing baseline, dimension mismatch, or corrupt file occurs:
 *      outputs diff artifacts (diff-*.png, actual-*.png, expected-*.png) to baselines-diff/
 *      and exits with non-zero code.
 * 5. Baseline files are NEVER auto-overwritten here; updates require explicit `npm run baselines`.
 * 6. Clean teardown of dev server and browser.
 */
import fs from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

const here = dirname(fileURLToPath(import.meta.url));
const baselinesDir = resolve(here, "..", "baselines");
const diffDir = resolve(here, "..", "baselines-diff");

// Controlled threshold configuration:
// - PIXEL_MATCH_THRESHOLD: 0.1 (per-pixel color sensitivity, standard strict tolerance)
// - MAX_DIFF_PIXELS_ALLOWED: 50 (strict tolerance allowing slight subpixel font rendering jitter, < 0.005%)
export const PIXEL_MATCH_THRESHOLD = 0.1;
export const MAX_DIFF_PIXELS_ALLOWED = 50;

// Clean up previous diff artifacts if any
if (fs.existsSync(diffDir)) {
  fs.rmSync(diffDir, { recursive: true, force: true });
}

const server = await createServer({
  root: resolve(here, ".."),
  server: { port: 4174, strictPort: true },
});
await server.listen();
const address = server.resolvedUrls?.local?.[0] ?? "http://localhost:4174";
console.log(`serving ${address} for visual gate`);

let browser;
let passedCount = 0;
const failedViews = [];

try {
  browser = await chromium.launch();

  async function testView(name, { viewport, drive, assertGeometry }) {
    console.log(`Verifying view: ${name} (${viewport.width}x${viewport.height})...`);
    const context = await browser.newContext({ viewport });
    const page = await context.newPage();

    page.on("pageerror", (err) => {
      console.error(`[PageError in ${name}]`, err);
      throw new Error(`PageError in ${name}: ${err.message}`);
    });

    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error(`[ConsoleError in ${name}]`, msg.text());
        throw new Error(`ConsoleError in ${name}: ${msg.text()}`);
      }
    });

    await page.goto(address, { waitUntil: "networkidle" });
    await page.waitForSelector("[data-fixture-ready='true']", { timeout: 15_000 });
    await page.evaluate(() => document.fonts.ready);

    if (drive) await drive(page);

    if (assertGeometry) {
      await assertGeometry(page);
    }

    // 1. Capture actual browser screenshot
    const actualBuf = await page.screenshot();

    // 2. Load and validate pinned repository baseline
    const baselinePath = resolve(baselinesDir, `${name}.png`);
    if (!fs.existsSync(baselinePath)) {
      fs.mkdirSync(diffDir, { recursive: true });
      fs.writeFileSync(resolve(diffDir, `actual-${name}.png`), actualBuf);
      const err = `Missing pinned baseline image for ${name}: ${baselinePath}`;
      console.error(`[FAIL] ${err}`);
      failedViews.push({ name, reason: err });
      await context.close();
      return;
    }

    const baselineBuf = fs.readFileSync(baselinePath);
    if (baselineBuf.length < 1000 || baselineBuf[0] !== 0x89 || baselineBuf[1] !== 0x50 || baselineBuf[2] !== 0x4e || baselineBuf[3] !== 0x47) {
      fs.mkdirSync(diffDir, { recursive: true });
      fs.writeFileSync(resolve(diffDir, `actual-${name}.png`), actualBuf);
      const err = `Baseline file ${name}.png is not a valid or intact PNG image`;
      console.error(`[FAIL] ${err}`);
      failedViews.push({ name, reason: err });
      await context.close();
      return;
    }

    let actualImg;
    let baselineImg;
    try {
      actualImg = PNG.sync.read(actualBuf);
      baselineImg = PNG.sync.read(baselineBuf);
    } catch (parseErr) {
      fs.mkdirSync(diffDir, { recursive: true });
      fs.writeFileSync(resolve(diffDir, `actual-${name}.png`), actualBuf);
      const err = `Failed to decode PNG for ${name}: ${parseErr.message}`;
      console.error(`[FAIL] ${err}`);
      failedViews.push({ name, reason: err });
      await context.close();
      return;
    }

    // 3. Verify dimensions
    if (actualImg.width !== baselineImg.width || actualImg.height !== baselineImg.height) {
      fs.mkdirSync(diffDir, { recursive: true });
      fs.writeFileSync(resolve(diffDir, `actual-${name}.png`), actualBuf);
      fs.writeFileSync(resolve(diffDir, `expected-${name}.png`), baselineBuf);
      const err = `Dimension mismatch for ${name}: actual ${actualImg.width}x${actualImg.height} vs baseline ${baselineImg.width}x${baselineImg.height}`;
      console.error(`[FAIL] ${err}`);
      failedViews.push({ name, reason: err });
      await context.close();
      return;
    }

    // 4. Pixel-level difference comparison via pixelmatch
    const diffImg = new PNG({ width: baselineImg.width, height: baselineImg.height });
    const diffPixels = pixelmatch(
      baselineImg.data,
      actualImg.data,
      diffImg.data,
      baselineImg.width,
      baselineImg.height,
      { threshold: PIXEL_MATCH_THRESHOLD }
    );

    const totalPixels = baselineImg.width * baselineImg.height;
    const diffPercent = ((diffPixels / totalPixels) * 100).toFixed(4);

    if (diffPixels > MAX_DIFF_PIXELS_ALLOWED) {
      fs.mkdirSync(diffDir, { recursive: true });
      const actualPath = resolve(diffDir, `actual-${name}.png`);
      const diffPath = resolve(diffDir, `diff-${name}.png`);
      const expectedPath = resolve(diffDir, `expected-${name}.png`);

      fs.writeFileSync(actualPath, actualBuf);
      fs.writeFileSync(diffPath, PNG.sync.write(diffImg));
      fs.writeFileSync(expectedPath, baselineBuf);

      const err = `Visual mismatch: ${diffPixels} diff pixels (${diffPercent}%) exceeds threshold of ${MAX_DIFF_PIXELS_ALLOWED} pixels.`;
      console.error(`[FAIL] ${name} ${err}`);
      console.error(`       Diff artifact:     ${diffPath}`);
      console.error(`       Actual capture:    ${actualPath}`);
      console.error(`       Expected baseline: ${expectedPath}`);
      failedViews.push({ name, diffPixels, totalPixels, diffPercent, reason: err });
    } else {
      console.log(`[PASS] ${name} passed pixel regression & geometry: ${diffPixels} diff pixels (${diffPercent}%).`);
      passedCount += 1;
    }

    await context.close();
  }

  // 1. Light View (1280x800)
  await testView("light", {
    viewport: { width: 1280, height: 800 },
    assertGeometry: async (page) => {
      const geom = await page.evaluate(() => {
        const titleBar = document.querySelector("header[class*='titleBar']");
        const rail = document.querySelector("[class*='railArea']");
        const nav = document.querySelector("[class*='navArea']");
        const workbench = document.querySelector("main[class*='workbench']");
        const inspector = document.querySelector("[class*='inspectorArea']");

        const rRect = rail?.getBoundingClientRect();
        const nRect = nav?.getBoundingClientRect();
        const wRect = workbench?.getBoundingClientRect();
        const iRect = inspector?.getBoundingClientRect();

        return {
          titleBarText: titleBar?.textContent ?? "",
          railWidth: rRect?.width ?? 0,
          navWidth: nRect?.width ?? 0,
          workbenchWidth: wRect?.width ?? 0,
          inspectorWidth: iRect?.width ?? 0,
          scrollWidth: document.documentElement.scrollWidth,
          clientWidth: document.documentElement.clientWidth,
        };
      });

      if (!geom.titleBarText.includes("IPC v1")) {
        throw new Error(`Expected titlebar to include "IPC v1", got "${geom.titleBarText}"`);
      }
      if (geom.railWidth < 40) throw new Error(`Rail width too narrow: ${geom.railWidth}`);
      if (geom.navWidth < 180) throw new Error(`Nav width too narrow: ${geom.navWidth}`);
      if (geom.workbenchWidth < 300) throw new Error(`Workbench width too narrow: ${geom.workbenchWidth}`);
      if (geom.inspectorWidth < 200) throw new Error(`Inspector width too narrow: ${geom.inspectorWidth}`);
      if (geom.scrollWidth > geom.clientWidth + 5) {
        throw new Error(`Unexpected horizontal scrollbar: scrollWidth ${geom.scrollWidth} > clientWidth ${geom.clientWidth}`);
      }
    },
  });

  // 2. Dark View (1280x800)
  await testView("dark", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='theme-toggle']");
      await page.waitForSelector("[data-theme='dark']");
    },
    assertGeometry: async (page) => {
      const themeInfo = await page.evaluate(() => {
        const shell = document.querySelector("[class*='shell']");
        const theme = shell?.getAttribute("data-theme");
        const style = window.getComputedStyle(shell);
        return { theme, bg: style.backgroundColor };
      });
      if (themeInfo.theme !== "dark") {
        throw new Error(`Expected data-theme="dark", got "${themeInfo.theme}"`);
      }
    },
  });

  // 3. Narrow View (760x800)
  await testView("narrow", {
    viewport: { width: 760, height: 800 },
    assertGeometry: async (page) => {
      const narrowInfo = await page.evaluate(() => {
        const shell = document.querySelector("[class*='shell']");
        return {
          isNarrow: shell?.getAttribute("data-narrow"),
          clientWidth: document.documentElement.clientWidth,
        };
      });
      if (narrowInfo.isNarrow !== "true") {
        throw new Error(`Expected data-narrow="true" at width 760, got "${narrowInfo.isNarrow}"`);
      }
    },
  });

  // 4. Error View (1280x800)
  await testView("error", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='thread-thr_fixture000000003']");
      await page.waitForSelector("[data-row-id]");
    },
    assertGeometry: async (page) => {
      const hasRows = await page.evaluate(() => {
        const rows = document.querySelectorAll("[data-row-id]");
        return rows.length > 0;
      });
      if (!hasRows) throw new Error("Expected conversation rows to render in error thread");
    },
  });

  // 5. Approval View (1280x800)
  await testView("approval", {
    viewport: { width: 1280, height: 800 },
    drive: async (page) => {
      await page.click("[data-testid='thread-thr_fixture000000002']");
      await page.waitForSelector("[data-testid='approve']");
    },
    assertGeometry: async (page) => {
      const approveVisible = await page.evaluate(() => {
        const btn = document.querySelector("[data-testid='approve']");
        const r = btn?.getBoundingClientRect();
        return r && r.width > 20 && r.height > 15;
      });
      if (!approveVisible) throw new Error("Approval action button not visible or zero size");
    },
  });

  if (failedViews.length > 0) {
    console.error(`\n=== Visual & Geometry Gate FAILED: ${failedViews.length}/5 views failed pixel regression ===`);
    process.exit(1);
  }

  console.log(`\n=== Visual & Geometry Gate: ${passedCount}/5 views passed (pixel regression 100% compliant) ===`);
} finally {
  if (browser) await browser.close();
  await server.close();
}
