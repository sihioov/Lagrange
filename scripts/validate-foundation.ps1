#!/usr/bin/env pwsh
# validate-foundation.ps1 — assert every documented workspace boundary of the
# Lagrange Station monorepo (design §20 + Todo 1 list) and its pin files exist.
# Exit 0 when the full tree is present; exit 1 listing every missing path.
# Twin: scripts/validate-foundation.sh (CI / clean containers).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$missing = @()

$dirs = @(
    'apps/web', 'apps/api-server',
    'crates/domain', 'crates/auth', 'crates/market-data', 'crates/factor-engine',
    'crates/selector', 'crates/portfolio-model', 'crates/job-queue', 'crates/result-model',
    'crates/risk-gateway', 'crates/kis-client',
    'nt/strategies', 'nt/custom-data', 'nt/backtest-worker', 'nt/paper-runner', 'nt/live-node',
    'data-pipelines/collectors', 'data-pipelines/validators', 'data-pipelines/normalizers',
    'data-pipelines/nt-catalog-builder',
    'migrations', 'configs',
    'tests/fixtures', 'tests/golden', 'tests/integration', 'tests/e2e', 'tests/failure',
    'deploy/compose', 'deploy/nginx', 'deploy/backup',
    'scripts', 'scripts/qa'
)
$files = @(
    'rust-toolchain.toml', '.python-version', 'Cargo.toml', 'Cargo.lock',
    'package.json', 'package-lock.json', 'nt/pyproject.toml', '.gitignore'
)

foreach ($d in $dirs) { if (-not (Test-Path (Join-Path $root $d))) { $missing += "dir: $d" } }
foreach ($f in $files) { if (-not (Test-Path (Join-Path $root $f))) { $missing += "file: $f" } }

if ($missing.Count -gt 0) {
    Write-Host 'FOUNDATION VALIDATION FAILED - missing:' -ForegroundColor Red
    $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
    exit 1
}
Write-Host 'FOUNDATION OK: documented workspace topology and pin files present' -ForegroundColor Green
exit 0
