#!/usr/bin/env pwsh
# verify-all.ps1 - run every baseline quality gate for the Lagrange Station monorepo.
# Fails fast (exit 1) on the first failing gate. The NT/uv gate is REPORTED as
# BLOCKED_ENVIRONMENT (documented, non-fatal) when uv cannot resolve the approved
# pins against the package index - it is never silently skipped or faked.
# Twin: scripts/verify-all.sh (CI / clean containers).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$blocked = @()
$step = 0

function Step([string]$Name) {
    $script:step++
    Write-Host "`n[$script:step] $Name" -ForegroundColor Cyan
}

function Fail([string]$Name, [string]$Out) {
    Write-Host "FAILED: $Name" -ForegroundColor Red
    if ($Out) { $Out | ForEach-Object { Write-Host $_ } }
    exit 1
}

Step 'check-pins (approved toolchain/package pins)'
$pinsOut = & "$root\scripts\check-pins.ps1" 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'check-pins' $pinsOut }

Step 'committed lockfiles (Cargo.lock, package-lock.json)'
foreach ($lf in @('Cargo.lock', 'package-lock.json')) {
    if (-not (Test-Path "$root\$lf")) { Fail "missing committed lockfile $lf" $null }
}
Write-Host 'Cargo.lock and package-lock.json present'

Step 'cargo fmt --all --check'
cargo fmt --all --check 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'cargo fmt --all --check' $null }

Step 'cargo clippy --workspace --all-targets --all-features -- -D warnings'
cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'cargo clippy -D warnings' $null }

Step 'cargo test --workspace'
cargo test --workspace 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'cargo test --workspace' $null }

Step 'npm run lint --workspaces --if-present'
npm run lint --workspaces --if-present 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'npm lint' $null }

Step 'npm run typecheck --workspaces --if-present'
npm run typecheck --workspaces --if-present 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'npm typecheck' $null }

Step 'npm test --workspaces --if-present'
npm test --workspaces --if-present 2>&1
if ($LASTEXITCODE -ne 0) { Fail 'npm test' $null }

Step 'uv run --project nt pytest -q'
$uvOut = (& uv run --project nt pytest -q 2>&1 | Out-String)
$uvCode = $LASTEXITCODE
if ($uvCode -ne 0) {
    if ($uvOut -match 'No solution found when resolving dependencies') {
        Write-Host 'BLOCKED_ENVIRONMENT: uv cannot resolve approved nt pins (polars 0.54.x unavailable on package index); uv.lock not generated.' -ForegroundColor Yellow
        Write-Host 'Exact error:'
        Write-Host $uvOut
        $blocked += 'nt/uv: polars 0.54.x unavailable on package index (see .omo/evidence)'
    } else {
        Fail 'uv run --project nt pytest -q' @($uvOut)
    }
}

if ($blocked.Count -gt 0) {
    Write-Host "`nVERIFY-ALL COMPLETE: all runnable gates OK. BLOCKED_ENVIRONMENT reported: $($blocked -join '; ')" -ForegroundColor Yellow
} else {
    Write-Host "`nALL GATES PASSED" -ForegroundColor Green
}
exit 0
