#!/usr/bin/env bash
# Altior Unified Quality Gate (Bash)
# Enforces Rust workspace, Tauri shell, Desktop frontend, and real ACP opt-in gates.
set -euo pipefail

echo "=================================================="
echo "  Altior Quality Gate: 1. Rust Workspace"
echo "=================================================="

echo "Checking cargo fmt..."
cargo fmt --all -- --check

echo "Running cargo clippy..."
cargo clippy --all-targets --all-features -- -D warnings

echo "Running cargo test --workspace..."
cargo test --workspace

echo "=================================================="
echo "  Altior Quality Gate: 2. Tauri Shell Crate"
echo "=================================================="

echo "Checking Tauri crate clippy..."
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml -- -D warnings

echo "Running Tauri crate tests..."
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml

echo "=================================================="
echo "  Altior Quality Gate: 3. Desktop Frontend"
echo "=================================================="

npm --prefix apps/desktop run gate

echo "=================================================="
echo "  Altior Quality Gate: 4. Real ACP Opt-in Check"
echo "=================================================="

if [ -n "${ALTIOR_ACP_SMOKE_AGENTS:-}" ]; then
    echo "Running opt-in real ACP smoke tests against ${ALTIOR_ACP_SMOKE_AGENTS}..."
    cargo test -p altior-acp --test smoke -- --nocapture
else
    echo "[SKIPPED] Real ACP external agent smoke test (ALTIOR_ACP_SMOKE_AGENTS is not set; opt-in required for live model testing)"
fi

echo ""
echo "=================================================="
echo "  ALL ALTIOR QUALITY GATES PASSED CLEANLY"
echo "=================================================="
