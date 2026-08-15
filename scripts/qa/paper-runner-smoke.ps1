<#
.SYNOPSIS
  QA smoke test for the Paper runner deployment unit.

.DESCRIPTION
  Starts the disposable QA PostgreSQL, verifies the systemd unit and its
  credential template, runs the runner/valuation integration tests, and checks
  that the production binary exposes its CLI without requiring live secrets.
#>
[CmdletBinding()]
param([switch]$KeepDb)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$unit = Join-Path $root 'deploy/systemd/paper-runner.service'
$envExample = Join-Path $root 'deploy/systemd/paper-runner.env.example'
$qaCompose = Join-Path $root 'deploy/qa/qa-db.compose.yml'
$qaPort = if ($env:LAGRANGE_QA_DB_PORT) { $env:LAGRANGE_QA_DB_PORT } else { '55432' }

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { throw 'docker not found on PATH' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'cargo not found on PATH' }
if (-not (Test-Path -LiteralPath $unit)) { throw "missing deployment unit: $unit" }
if (-not (Test-Path -LiteralPath $envExample)) { throw "missing env template: $envExample" }

$unitText = Get-Content -Raw -LiteralPath $unit
foreach ($required in @(
    'EnvironmentFile=/etc/lagrange/paper-runner.env',
    'ExecStart=/opt/lagrange/bin/paper-runner',
    'Restart=on-failure',
    'ProtectSystem=strict',
    'ReadOnlyPaths=/var/lib/lagrange/data/phase0'
)) {
    if ($unitText -notmatch [regex]::Escape($required)) { throw "unit missing: $required" }
}

$envText = Get-Content -Raw -LiteralPath $envExample
foreach ($required in @(
    'PAPER_APP_DB_PASSWORD_FILE=', 'PAPER_WORKER_DB_PASSWORD_FILE=',
    'PAPER_ADMIN_DB_PASSWORD_FILE=', 'PAPER_AUDIT_DB_PASSWORD_FILE=',
    'LAGRANGE_DATASET_ROOT=', 'PAPER_HEALTH_STATE_PATH='
)) {
    if ($envText -notmatch [regex]::Escape($required)) { throw "env template missing: $required" }
}

function Invoke-QaCompose { & docker compose -p lagrange-qa -f $qaCompose @args }
$env:DATABASE_URL = "postgres://postgres:lagrange@127.0.0.1:$qaPort/postgres"
$transcript = Join-Path ([System.IO.Path]::GetTempPath()) "lagrange-paper-runner-smoke-$([guid]::NewGuid()).log"

try {
    Invoke-QaCompose up -d --wait qa-db | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'QA database did not become healthy' }

    Push-Location $root
    try {
        & cargo test -p api-server --test paper_runner --test paper_valuation -- --nocapture 2>&1 | Tee-Object -FilePath $transcript
        if ($LASTEXITCODE -ne 0) { throw 'Paper runner QA tests failed' }

        & cargo run -p api-server --bin paper-runner -- --help 2>&1 | Tee-Object -FilePath $transcript -Append
        if ($LASTEXITCODE -ne 0) { throw 'paper-runner --help failed' }
    } finally { Pop-Location }
    Write-Host 'PAPER_RUNNER_SMOKE: PASS'
}
finally {
    Remove-Item -LiteralPath $transcript -Force -ErrorAction SilentlyContinue
    if (-not $KeepDb) { Invoke-QaCompose down -v --remove-orphans | Out-Null }
}
