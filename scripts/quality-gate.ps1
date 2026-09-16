# Altior Unified Quality Gate (PowerShell)
# Enforces Rust workspace, Tauri shell, Desktop frontend, and real ACP opt-in gates.
$ErrorActionPreference = "Stop"

function Assert-StepSuccess {
    param([string]$StepName)
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "==================================================" -ForegroundColor Red
        Write-Host "  QUALITY GATE FAILED: $StepName (exit code: $LASTEXITCODE)" -ForegroundColor Red
        Write-Host "==================================================" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Altior Quality Gate: 1. Rust Workspace" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

Write-Host "Checking cargo fmt..."
cargo fmt --all -- --check
Assert-StepSuccess "cargo fmt"

Write-Host "Running cargo clippy..."
cargo clippy --all-targets --all-features -- -D warnings
Assert-StepSuccess "cargo clippy"

Write-Host "Running cargo test --workspace..."
cargo test --workspace
Assert-StepSuccess "cargo test workspace"

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Altior Quality Gate: 2. Tauri Shell Crate" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

Write-Host "Checking Tauri crate clippy..."
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml -- -D warnings
Assert-StepSuccess "Tauri crate clippy"

Write-Host "Running Tauri crate tests..."
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
Assert-StepSuccess "Tauri crate tests"

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Altior Quality Gate: 3. Desktop Frontend" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

cmd.exe /c "npm --prefix apps/desktop run gate"
Assert-StepSuccess "Desktop Frontend (npm run gate)"

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Altior Quality Gate: 4. Real ACP Opt-in Check" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

if ($env:ALTIOR_ACP_SMOKE_AGENTS) {
    Write-Host "Running opt-in real ACP smoke tests against $env:ALTIOR_ACP_SMOKE_AGENTS..."
    cargo test -p altior-acp --test smoke -- --nocapture
    Assert-StepSuccess "Real ACP smoke tests"
} else {
    Write-Host "[SKIPPED] Real ACP external agent smoke test (ALTIOR_ACP_SMOKE_AGENTS is not set; opt-in required for live model testing)" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "==================================================" -ForegroundColor Green
Write-Host "  ALL ALTIOR QUALITY GATES PASSED CLEANLY" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
