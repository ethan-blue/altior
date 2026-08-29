# AI development discipline

Altior is expected to be implemented primarily by coding agents. That changes
the amount of executable context the repository must provide; it does not lower
the engineering bar. Prompts are disposable. Repository contracts, tests,
fixtures, and decisions are durable.

## Sources of truth

Read in this order for every change:

1. every applicable `AGENTS.md`
2. accepted ADRs under `docs/decisions/`
3. product, architecture, security, sync, memory, harness, and UI contracts
4. the narrow work item and its acceptance criteria
5. existing code and tests at the change boundary

If these sources disagree, stop and resolve the durable documentation first.
Never hide a load-bearing rule only in an agent prompt.

## Work-item contract

Every implementation slice must state:

- outcome visible to the user or another component
- in-scope and explicitly out-of-scope behavior
- domain/protocol contracts touched
- failure and cancellation behavior
- security and synchronization impact
- deterministic acceptance evidence
- allowed migration and compatibility behavior

Prefer slices that one reviewer can understand without reconstructing a broad
refactor. A slice may span layers when required for one vertical outcome, but it
must not mix unrelated cleanup.

## Execution loop

1. **Reconnaissance**: read applicable instructions and trace the existing
   boundary. Record assumptions that affect architecture or product behavior.
2. **Contract**: add or update the DTO, port, ADR, acceptance criterion, or
   fixture before implementation when behavior is new.
3. **Failure model**: list invalid input, cancellation, crash, restart, duplicate,
   timeout, offline, permission-denied, and downgrade cases that apply.
4. **Implementation**: make the smallest inward-pointing change that satisfies
   the contract. Preserve unrelated work.
5. **Verification**: run focused tests while iterating, then the repository gates
   required by `AGENTS.md`.
6. **Review**: inspect the final diff for boundary leaks, secret exposure,
   nondeterminism, silent fallback, and undocumented durable changes.
7. **Evidence**: report commands run, fixtures/screens reviewed, and known limits.

## AI-specific guardrails

- Do not introduce a framework, protocol, database, crypto primitive, or durable
  format without the required ADR.
- Do not infer capability support from a version string.
- Do not retry a possibly delivered prompt.
- Do not persist credentials, private keys, or secret-shaped fixtures.
- Do not add real sleeps, public-network tests, machine-load assumptions, random
  identities, or unbounded channels.
- Do not create generic `utils`, `manager`, or global-state dumping grounds when
  a narrow domain or infrastructure owner exists.
- Do not generate large UI components that mix IPC, state reduction, workflow,
  and presentation.
- Do not update visual baselines without reviewing the rendered difference.
- Do not copy reference-project source without license verification, provenance,
  and required notices.
- Do not mark a placeholder, mocked happy path, or compile-only seam as a finished
  feature.

## Contract-first fixtures

Protocol and runtime work starts with synthetic fixtures for:

- normal create/prompt/stream/complete
- partial deltas and missing optional events
- unknown forward-compatible events
- approval request/allow/deny/timeout
- cancel before delivery, during stream, and after completion
- disconnect before confirmed delivery and after possible delivery
- crash and restart with durable recovery
- duplicate, delayed, and out-of-order input where applicable
- bounded oversized payload rejection

Fixtures contain no real user transcript or credentials. Where multiple adapters
eventually exist, the same normalized expected trace runs against all of them.

## UI development with agents

Before implementing a screen, add fixtures for its meaningful states. Build the
screen against the in-memory IPC transport before wiring the real command path.

Required review evidence for a shared component or screen:

- default, empty, loading, error, disconnected, and permission states as relevant
- keyboard navigation and visible focus
- light and dark rendering
- narrow-width behavior
- deterministic screenshot comparison for load-bearing layout
- no raw secrets or unbounded diagnostics in rendered output

AI-authored UI should reuse semantic tokens and shared primitives. Review rejects
new arbitrary colors, spacing, radii, shadows, or decorative status pills unless
the design contract is deliberately updated.

## Review lanes

Use independent review passes even when one model performs them:

- **Domain**: invariants, identities, state transitions, typed errors
- **Runtime**: delivery safety, cancellation, concurrency, resource bounds
- **Security**: trust boundaries, path containment, secret handling, IPC input
- **Data**: migrations, crash safety, projection rebuild, downgrade behavior
- **UI**: state ownership, accessibility, density, visual consistency
- **Test**: determinism, observable assertions, adversarial coverage
- **Provenance**: licenses, third-party source reuse, notices

The implementation agent's own summary is not review evidence.

## Definition of done

A work item is done only when:

- acceptance behavior works through the intended boundary
- applicable failure paths have deterministic tests
- format/protocol changes are versioned and documented
- migrations and downgrade behavior are covered when durable state changes
- UI states are rendered and visually reviewed when applicable
- `cargo fmt`, Clippy with warnings denied, and workspace tests pass
- frontend format, lint, typecheck, unit, browser, and visual checks pass once the
  Desktop workspace exists
- no unrelated change or hidden compatibility fallback remains
- user-facing and contributor documentation reflects the actual behavior

## Suggested change brief

```text
Outcome:
Acceptance:
In scope:
Out of scope:
Contracts touched:
Failure/security considerations:
Fixtures/tests:
Migration/downgrade:
Reference behavior (not source):
```

This brief should live in an issue or checked-in work record when work spans
multiple changes. The final durable behavior still belongs in docs and tests.
