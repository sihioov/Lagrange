<#
.SYNOPSIS
  Static deployment contract and real recommendation-runner integration smoke.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$compose = Join-Path $root 'deploy/compose/compose.yml'
$unit = Join-Path $root 'deploy/systemd/lagrange-recommendation-runner.service'
$dockerfile = Join-Path $root 'crates/job-queue/Dockerfile'
$dockerignore = Join-Path $root '.dockerignore'
$qaCompose = Join-Path $root 'deploy/qa/qa-db.compose.yml'
$qaPort = if ($env:LAGRANGE_QA_DB_PORT) { $env:LAGRANGE_QA_DB_PORT } else { '55432' }

foreach ($command in @('cargo', 'docker', 'python', 'uv')) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) { throw "$command not found on PATH" }
}
foreach ($path in @($compose, $unit, $dockerfile)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "missing required deployment file: $path" }
}
$dockerignoreText = Get-Content -Raw -LiteralPath $dockerignore
if ($dockerignoreText -notmatch [regex]::Escape('**/.venv')) {
    throw '.dockerignore must exclude host Python virtual environments'
}

$composeText = Get-Content -Raw -LiteralPath $compose
foreach ($required in @(
    'recommendation-runner:', 'crates/job-queue/Dockerfile',
    'DB_PASSWORD_FILE: /run/secrets/db_worker_password',
    'RECOMMENDATION_HEALTH_STATE_PATH: /run/recommendation-health/health.json',
    '/data/curated:ro', '/opt/lagrange/configs/universes/kr-etf-core-v1.yaml:ro',
    '"/usr/local/bin/recommendation-runner", "healthcheck"'
)) {
    if ($composeText -notmatch [regex]::Escape($required)) { throw "Compose missing: $required" }
}
$unitText = Get-Content -Raw -LiteralPath $unit
foreach ($required in @(
    'RuntimeDirectory=lagrange-recommendation-runner',
    'RuntimeDirectory=lagrange-recommendation-runner/tmp',
    'RECOMMENDATION_HEALTH_STATE_PATH=/run/lagrange-recommendation-runner/health.json',
    'recommendation-runner --repo-root /opt/lagrange',
    'ReadOnlyPaths=/var/lib/lagrange/data/curated /etc/lagrange/universes'
)) {
    if ($unitText -notmatch [regex]::Escape($required)) { throw "systemd unit missing: $required" }
}
if ($unitText -match '(?m)^ExecStartPost=') {
    throw 'systemd unit must not race startup health-state creation with ExecStartPost'
}

$env:DATABASE_URL = if ($env:DATABASE_URL) { $env:DATABASE_URL } else { "postgres://postgres:lagrange@127.0.0.1:$qaPort/postgres" }
function Invoke-QaCompose { & docker compose -p lagrange-recommendation-qa -f $qaCompose @args }
try {
    Invoke-QaCompose up -d --wait qa-db | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'QA database did not become healthy' }
    Push-Location $root
    try {
        # This fixture creates the labeled synthetic 11-ETF QA data, migrates
        # disposable PostgreSQL, seeds pinned universe/dataset/entitlement/
        # config records, and invokes the real runner implementation.
        & cargo test -p job-queue --test recommendation_runner real_worker_and_uv_publish_all_five_shipped_strategies -- --nocapture
        if ($LASTEXITCODE -ne 0) { throw 'real recommendation runner smoke failed' }
        $env:APP_ENV = 'qa'
        & cargo run -p job-queue --bin recommendation-runner -- --help
        if ($LASTEXITCODE -ne 0) { throw 'recommendation-runner CLI smoke failed' }
    } finally { Pop-Location }
    Write-Host 'RECOMMENDATION_RUNNER_SMOKE: PASS'
}
finally {
    Invoke-QaCompose down -v --remove-orphans | Out-Null
}
