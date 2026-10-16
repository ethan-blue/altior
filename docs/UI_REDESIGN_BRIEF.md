# Altior Desktop UI Redesign Brief

Bilingual summary of the redesign intent for this shift. Canonical product posture remains `docs/UI_ARCHITECTURE.md` and `AGENTS.md`.

## Goals / 目标

**EN** — Altior is a local-first personal Agent + knowledge workbench, **not** a chat webpage. Visual language should match Linear / Raycast / VS Code / Obsidian / GitHub Desktop: clean, restrained, high information density, keyboard-first, auditable.

**ZH** — Altior 是本地优先的个人 Agent + 知识工作台，**不是**聊天网页。视觉语言对齐 Linear / Raycast / VS Code / Obsidian / GitHub Desktop：克制、高信息密度、键盘优先、可审计。

## Forbidden patterns / 禁止样式

- Purple / pink / neon gradients; glow; glassmorphism
- Robot / sparkle “AI” icons
- Chat bubbles; floating pill composer
- Fake dashboards; card waterfalls as primary density
- Large radii (16–24px) on product surfaces
- Decorative hero panels or uniform marketing card grids in the workbench

## Shell regions (existing)

CSS grid already matches architecture:

| Region | Role | Width / notes |
|--------|------|----------------|
| Title | tabs, palette entry, window tools | full width |
| Rail | module switch | 48px |
| Nav | searchable lists | 208–420px (default ~256) |
| Workbench | thread / management surface | flexible |
| Inspector | context, provenance, details | 280–640px, closable |
| Status | Core / ACP / sync / activity | full width |

Primary implementation: `apps/desktop/src/components/shell.tsx` + `shell.module.css`.
Tokens: `apps/desktop/src/styles/tokens.css` (accent cobalt `#2f5fd7` / dark `#7ba0f0`).
Timeline stream: `features/timeline`.

## Phased delivery / 分期交付

1. **IA** — persist brief + IA map (`docs/UI_REDESIGN_BRIEF.md`, `docs/UI_IA_MAP.md`).
2. **Tokens / shell** — clamp radii, flatten docked composer, keep elevation for menus/dialogs only.
3. **Timeline / composer / permission** — compact tool rows; amber permission strip; no chat bubbles.
4. **Memory table** — replace card waterfall with dense table/list; selection → inspector.
5. **Agents** — config / profiles page polish (next slice).

## Mapping to existing files

| Concern | Path |
|---------|------|
| Shell chrome | `apps/desktop/src/components/shell.tsx` |
| Shell styles | `apps/desktop/src/components/shell.module.css` |
| Memory pane | `apps/desktop/src/components/MemoryPane.tsx` |
| Memory styles | `apps/desktop/src/components/memoryPane.module.css` |
| Tokens | `apps/desktop/src/styles/tokens.css` |
| Timeline | `apps/desktop/src/features/timeline/` |
| i18n | `apps/desktop/src/i18n/zh-CN.ts`, `en.ts` |
| Architecture | `docs/UI_ARCHITECTURE.md` |

## This-shift acceptance

- Brief + IA map in `docs/`
- `--radius-lg` ≤ 8px for cards/inputs; no product use of `--radius-xl: 16px`; `--radius-modal` ≤ 12px
- Composer docked, full workbench width, 1px top divider, flat (no composer card shadow)
- Memory pane dense table with filters; row selection drives inspector
- Typecheck + unit tests green (or intentional contract updates only)

## Out of scope this shift

- Agents config page depth
- Command palette
- Dark-theme contrast pass
- Committing unless explicitly requested
