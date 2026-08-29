# Acceptance targets

Targets are budgets, not claims. P0 records baselines and any justified revision.

## Runtime

- Desktop reaches usable local history within 2 seconds on the reference Windows host.
- Idle Desktop plus Core remains below 150 MiB working set on that host.
- Closing/reloading Desktop does not stop an active Core turn.
- Core restart recovers durable thread state without duplicating a prompt.

## Desktop UI

- The renderer has no direct database, filesystem, shell, agent-process, or secret
  access; those actions cross versioned Core or narrowly scoped Tauri contracts.
- Keyboard navigation reaches every first-release command and restores visible
  focus after dialogs, menus, virtualized rows, and pane changes.
- Streaming one event does not rerender the complete visible timeline.
- Prepending history, appending output, and changing streaming row height preserve
  the documented scroll anchor behavior.
- Shell, thread, permission, settings, empty, disconnected, and failure states
  have reviewed deterministic visual baselines in light and dark themes.
- Narrow-window layout keeps the active thread and permission decisions usable
  without horizontal page scrolling.

## Data

- Search remains interactive with 100,000 messages and 100,000 memory records.
- Every released migration is tested from the previous supported release.
- Projection deletion followed by rebuild produces equivalent query-visible state.
- Abrupt termination during a write leaves either the prior or committed state,
  never a partially valid record.

## Sync

- Three devices converge after concurrent offline creation, correction, and forgetting.
- Duplicate and out-of-order envelopes do not change the converged result.
- A device offline for a simulated 30 days catches up without full-vault redownload
  unless compaction policy explicitly requires a snapshot.
- A revoked device cannot publish accepted new events.
- Relay storage inspection reveals no plaintext knowledge or stable secret material.

## Harnesses

- At least two ACP agents support create, prompt, stream, cancel, and resume/fallback.
- Unsupported capabilities are visible and never silently emulated.
- Indeterminate prompt delivery never causes automatic duplicate execution.
- Agent and terminal subprocesses are cleaned after close, crash, and Core shutdown.
- The first release exposes only negotiated ACP controls and ships no placeholder
  Terminal, Codex app-server, Native Harness, or multi-agent execution UI.

## Memory

- Confirmed memories are retrievable across paired devices.
- Retrieval exposes source and ranking explanation.
- Correction supersedes prior content without erasing audit history.
- Forgetting propagates and survives stale-device return and compaction.
- Credential fixtures are rejected before persistence and synchronization.
