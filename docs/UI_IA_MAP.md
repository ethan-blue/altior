# Altior Desktop Information Architecture Map

First-release IA aligned with `docs/UI_ARCHITECTURE.md` and the redesign brief.

```text
+--------------------------------------------------------------------------+
| title bar | workspace tabs                                   window tools |
+----+----------------------+--------------------------------+--------------+
|rail| Nav pane             | Workbench                      | Inspector    |
|    | (module-specific)    | (surface for active module)    | (kind)       |
|    |                      |                                |              |
|    |                      +--------------------------------+              |
|    |                      | composer / approval / status   |              |
+----+----------------------+--------------------------------+--------------+
| status: Core | ACP | sync/local | background activity                     |
+--------------------------------------------------------------------------+
```

## Rail modules → Nav → Workbench → Inspector

| Rail module | Nav pane contents | Workbench surfaces | Inspector kinds |
|-------------|-------------------|--------------------|-----------------|
| **Threads** (shipped) | Search/filter; pinned; recent; create thread | Active thread header; event timeline; docked composer; permission strip | Context; turn details; files/diff; terminal; provenance; diagnostics |
| **Memory** (shipped, redesign → table) | Scope + lifecycle filters | Dense memory table | Selected memory provenance / detail |
| **Agents** (shipped shell) | Agent profile list | Agent config / test surface | Profile detail; capability metadata |
| **Settings** (shipped) | Settings sections | Theme / locale / runtime forms | Section help / diagnostics |
| **Projects** (future / P4) | Project list | Project overview; linked threads | Metadata; sync policy |
| **Devices** (future / P3) | Paired devices | Device detail; pairing; revoke | Keys status (no secrets); sync health |

## Surface rules

- One inspector shared across modules; content switches by selection.
- Composer belongs only to thread workbench bottom (docked), never as a floating pill.
- Memory is a table/workbench, not a chat or card waterfall.
- Unshipped rail items stay visible but disabled with phase affordance.

## Density targets

- Rail: 48px
- Primary controls: ~32px on 4px grid
- Collapsed tool rows: ~28–32px
- Card/input radius: ≤ 8px; modal: ≤ 12px via `--radius-modal`
