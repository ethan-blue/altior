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

Write-Host "Checking TS DTO export drift (protocol dto-export regeneration)..."
function Get-DtoContentMap {
    param([string]$Dir)
    $map = @{}
    Get-ChildItem $Dir -Filter *.ts | ForEach-Object {
        # -Encoding UTF8: Windows PowerShell 5.1 otherwise decodes BOM-less
        # files in the system ANSI codepage, where a multi-byte char can
        # swallow an adjacent CR and break the CRLF normalization below,
        # flagging every file as drifted.
        $map[$_.Name] = (Get-Content $_.FullName -Raw -Encoding UTF8) -replace "`r`n", "`n"
    }
    return $map
}
$dtoDir = "apps/desktop/src/ipc/dto"
$beforeDto = Get-DtoContentMap $dtoDir
cargo test -p altior-protocol --features dto-export
Assert-StepSuccess "protocol dto-export"
$afterDto = Get-DtoContentMap $dtoDir
$driftedFiles = @()
foreach ($name in $beforeDto.Keys) {
    if (-not $afterDto.ContainsKey($name) -or $afterDto[$name] -ne $beforeDto[$name]) {
        $driftedFiles += $name
    }
}
foreach ($name in $afterDto.Keys) {
    if (-not $beforeDto.ContainsKey($name)) {
        $driftedFiles += $name
    }
}
if ($driftedFiles.Count -gt 0) {
    Write-Host ""
    Write-Host "TS DTO drift detected: $($driftedFiles -join ', ')" -ForegroundColor Red
    Write-Host "Rust DTO definitions changed without matching TS exports." -ForegroundColor Red
    Write-Host "Run 'cargo test -p altior-protocol --features dto-export', review the diff, and commit the regenerated files." -ForegroundColor Red
    exit 1
}
Write-Host "TS DTO exports are in sync with Rust definitions."

# The regeneration rewrote every export with its own line endings; restore
# files whose only delta is CR churn so the gate leaves the working tree
# clean. Files with real content changes are preserved.
$churnedFiles = git diff --name-only -- $dtoDir
foreach ($rel in $churnedFiles) {
    git diff --ignore-cr-at-eol --quiet -- $rel
    if ($LASTEXITCODE -eq 0) {
        git checkout -- $rel
    }
}

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
Write-Host "  Altior Quality Gate: 4. ACP Loopback + Real Opt-in" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

Write-Host "Running deterministic ACP loopback smoke (real subprocess, in-repo stub)..."
cargo test -p altior-acp --test loopback_smoke -- --nocapture
Assert-StepSuccess "ACP loopback smoke tests"

if ($env:ALTIOR_ACP_SMOKE_AGENTS) {
    Write-Host "Running opt-in real ACP smoke tests against $env:ALTIOR_ACP_SMOKE_AGENTS..."
    cargo test -p altior-acp --test smoke -- --nocapture
    Assert-StepSuccess "Real ACP smoke tests"
} else {
    Write-Host "[OPT-IN SKIPPED] Real external ACP agent smoke (ALTIOR_ACP_SMOKE_AGENTS is not set; live model testing stays opt-in). The deterministic loopback above covers the same journey against a real subprocess." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "==================================================" -ForegroundColor Green
Write-Host "  ALL ALTIOR QUALITY GATES PASSED CLEANLY" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
