/**
 * WCAG 2.2 Contrast Ratio Verification (DESIGN_I18N §3, A10).
 *
 * Verifies that text/surface and control-border/surface combinations in
 * both light and dark themes meet WCAG AA requirements (4.5:1 for text,
 * 3:1 for essential controls and boundaries).
 */

function luminance(hex: string): number {
  const rgb = hex
    .replace("#", "")
    .match(/.{2}/g)!
    .map((x) => parseInt(x, 16) / 255);
  const a = rgb.map((v) =>
    v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4),
  );
  return a[0]! * 0.2126 + a[1]! * 0.7152 + a[2]! * 0.0722;
}

export function contrastRatio(hex1: string, hex2: string): number {
  const l1 = luminance(hex1);
  const l2 = luminance(hex2);
  const lighter = Math.max(l1, l2);
  const darker = Math.min(l1, l2);
  return (lighter + 0.05) / (darker + 0.05);
}

export const lightTokens = {
  canvas: "#f6f7f9",
  surface: "#ffffff",
  elevated: "#ffffff",
  border: "#d4d7dc",
  controlBorder: "#858c97",
  text: "#1c1f24",
  muted: "#5b616b",
  accent: "#2f5fd7",
  accentForeground: "#ffffff",
  danger: "#b3362b",
  warning: "#946200",
  success: "#2e7d43",
  selection: "#dbe4fb",
  focus: "#2f5fd7",
  warningSurface: "#fff8eb",
  dangerSurface: "#fdf2f1",
  successSurface: "#f0f8f3",
  disabledText: "#8c929d",
  disabledSurface: "#eaecf0",
};

export const darkTokens = {
  canvas: "#14161a",
  surface: "#1b1e24",
  elevated: "#22262d",
  border: "#333841",
  controlBorder: "#707b8c",
  text: "#e4e7ec",
  muted: "#9aa1ac",
  accent: "#7ba0f0",
  accentForeground: "#14161a",
  danger: "#e2766c",
  warning: "#d8a53f",
  success: "#6fbf8a",
  selection: "#26314a",
  focus: "#7ba0f0",
  warningSurface: "#2a2315",
  dangerSurface: "#2e1a19",
  successSurface: "#17261c",
  disabledText: "#636b76",
  disabledSurface: "#262a32",
};

export interface AuditResult {
  theme: "light" | "dark";
  name: string;
  ratio: string;
  required: string;
  status: "PASS" | "FAIL";
}

export interface AuditReport {
  passed: number;
  failed: number;
  total: number;
  results: AuditResult[];
}

export function runAudit(): AuditReport {
  const checks: Array<{
    theme: "light" | "dark";
    name: string;
    fg: string;
    bg: string;
    min: number;
  }> = [
    // Light theme
    { theme: "light", name: "text on surface", fg: lightTokens.text, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "text on canvas", fg: lightTokens.text, bg: lightTokens.canvas, min: 4.5 },
    { theme: "light", name: "muted on surface", fg: lightTokens.muted, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "accent on surface", fg: lightTokens.accent, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "danger on surface", fg: lightTokens.danger, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "warning on surface", fg: lightTokens.warning, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "success on surface", fg: lightTokens.success, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "control-border on surface", fg: lightTokens.controlBorder, bg: lightTokens.surface, min: 3.0 },
    { theme: "light", name: "control-border on canvas", fg: lightTokens.controlBorder, bg: lightTokens.canvas, min: 3.0 },
    { theme: "light", name: "approve button text", fg: lightTokens.accentForeground, bg: lightTokens.success, min: 4.5 },
    { theme: "light", name: "deny button text", fg: lightTokens.danger, bg: lightTokens.surface, min: 4.5 },
    { theme: "light", name: "primary button text", fg: lightTokens.accentForeground, bg: lightTokens.accent, min: 4.5 },

    // Dark theme
    { theme: "dark", name: "text on surface", fg: darkTokens.text, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "text on canvas", fg: darkTokens.text, bg: darkTokens.canvas, min: 4.5 },
    { theme: "dark", name: "muted on surface", fg: darkTokens.muted, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "accent on surface", fg: darkTokens.accent, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "danger on surface", fg: darkTokens.danger, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "warning on surface", fg: darkTokens.warning, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "success on surface", fg: darkTokens.success, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "control-border on surface", fg: darkTokens.controlBorder, bg: darkTokens.surface, min: 3.0 },
    { theme: "dark", name: "control-border on canvas", fg: darkTokens.controlBorder, bg: darkTokens.canvas, min: 3.0 },
    { theme: "dark", name: "approve button text", fg: darkTokens.accentForeground, bg: darkTokens.success, min: 4.5 },
    { theme: "dark", name: "deny button text", fg: darkTokens.danger, bg: darkTokens.surface, min: 4.5 },
    { theme: "dark", name: "primary button text", fg: darkTokens.accentForeground, bg: darkTokens.accent, min: 4.5 },
  ];

  let passed = 0;
  let failed = 0;
  const results: AuditResult[] = [];

  for (const check of checks) {
    const ratio = contrastRatio(check.fg, check.bg);
    const ok = ratio >= check.min;
    if (ok) passed++;
    else failed++;
    results.push({
      theme: check.theme,
      name: check.name,
      ratio: ratio.toFixed(2) + ":1",
      required: check.min + ":1",
      status: ok ? "PASS" : "FAIL",
    });
  }

  return { passed, failed, total: checks.length, results };
}
