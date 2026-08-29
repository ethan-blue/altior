# Contributing

Read [AGENTS.md](AGENTS.md), [AI Development Discipline](docs/AI_DEVELOPMENT.md),
and the relevant architecture documents before making changes. Work should
reference a work package, acceptance criterion, or ADR.

Before submitting changes:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

For changes touching `apps/desktop` (ADR 0005), also run inside that
directory:

```text
npm run typecheck
npm test
```

Regenerate the TypeScript DTOs after changing protocol DTOs
(`cargo test -p altior-protocol --features dto-export`) and commit the
result; the generated files under `apps/desktop/src/ipc/dto/` are the
Desktop contract.

Do not include real user conversations, credentials, local paths, or production
Vault material in tests. Use deterministic synthetic fixtures.
