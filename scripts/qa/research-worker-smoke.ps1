<#
.SYNOPSIS
  Static and functional smoke test for the research-worker Compose service.
#>
[CmdletBinding()]
param(
    [switch]$StaticOnly,
    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$composeFile = Join-Path $root 'deploy/compose/compose.yml'
$dockerfile = Join-Path $root 'data-pipelines/collectors/Dockerfile'
$secretExample = Join-Path $root 'deploy/secrets/db_research_password.example'

function Assert-Contains([string]$Text, [string]$Value, [string]$Context) {
    if ($Text.IndexOf($Value, [StringComparison]::Ordinal) -lt 0) {
        throw "$Context missing required value: $Value"
    }
}

function Invoke-ValidatorSelfTests {
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) "lagrange-research-validator-$([guid]::NewGuid().ToString('N'))"
    try {
        foreach ($directory in @(
            'scripts/qa', 'deploy/compose', 'deploy/secrets', 'data-pipelines/collectors'
        )) { New-Item -ItemType Directory -Path (Join-Path $testRoot $directory) -Force | Out-Null }
        Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $testRoot 'scripts/qa/research-worker-smoke.ps1')
        Copy-Item -LiteralPath $composeFile -Destination (Join-Path $testRoot 'deploy/compose/compose.yml')
        Copy-Item -LiteralPath $dockerfile -Destination (Join-Path $testRoot 'data-pipelines/collectors/Dockerfile')
        foreach ($name in @('.gitignore', 'README.md', 'db_research_password.example')) {
            Copy-Item -LiteralPath (Join-Path $root "deploy/secrets/$name") -Destination (Join-Path $testRoot "deploy/secrets/$name")
        }
        & git -C $testRoot init -q
        if ($LASTEXITCODE -ne 0) { throw 'self-test git init failed' }
        & git -C $testRoot add -f -- deploy/secrets *> $null
        if ($LASTEXITCODE -ne 0) { throw 'self-test git add failed' }

        $testScript = Join-Path $testRoot 'scripts/qa/research-worker-smoke.ps1'
        $testCompose = Join-Path $testRoot 'deploy/compose/compose.yml'
        $testDockerfile = Join-Path $testRoot 'data-pipelines/collectors/Dockerfile'
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -ne 0) { throw 'self-test baseline fixture must pass' }

        $baselineCompose = Get-Content -Raw -LiteralPath $testCompose
        [IO.File]::WriteAllText(
            $testCompose,
            $baselineCompose.Replace(
                '${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw',
                '${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw:ro'
            )
        )
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a read-only Raw mount' }
        [IO.File]::WriteAllText($testCompose, $baselineCompose)

        $baselineDockerfile = Get-Content -Raw -LiteralPath $testDockerfile
        $lowercasePinned = [regex]::Replace($baselineDockerfile, '(?m)^FROM ', 'from ', 1)
        [IO.File]::WriteAllText($testDockerfile, $lowercasePinned)
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -ne 0) { throw 'validator rejected a lowercase digest-pinned FROM' }

        $lowercaseUnpinned = [regex]::Replace(
            $lowercasePinned,
            '(?im)^from\s+rust:1\.97\.1-alpine@sha256:[0-9a-f]{64}',
            'from rust:1.97.1-alpine',
            1
        )
        [IO.File]::WriteAllText($testDockerfile, $lowercaseUnpinned)
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a lowercase unpinned FROM' }
        Write-Host 'RESEARCH_WORKER_SMOKE: validator self-test PASS'
    }
    finally {
        Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-StaticChecks {
    if (-not (Test-Path -LiteralPath $composeFile)) { throw "missing Compose file: $composeFile" }
    $compose = Get-Content -Raw -LiteralPath $composeFile
    $match = [regex]::Match(
        $compose,
        '(?ms)^  research-worker:\r?\n(?<body>.*?)(?=^  [A-Za-z0-9][A-Za-z0-9-]*:\s*$|^secrets:\s*$)'
    )
    if (-not $match.Success) { throw 'research-worker service is missing from Compose' }
    $worker = $match.Groups['body'].Value

    foreach ($required in @(
        'build:',
        'context: ../..',
        'dockerfile: data-pipelines/collectors/Dockerfile',
        'entrypoint: ["/usr/local/bin/research-worker"]',
        'APP_ENV: ${APP_ENV:-development}',
        'RESEARCH_FETCH_MODE: ${RESEARCH_FETCH_MODE:-synthetic}',
        'RESEARCH_RUN_AT_KST: ${RESEARCH_RUN_AT_KST:-16:30}',
        'RESEARCH_MAX_PUBLICATION_AGE_SECS: ${RESEARCH_MAX_PUBLICATION_AGE_SECS:-345600}',
        'RESEARCH_RAW_ROOT: /data/raw',
        'DB_HOST: postgres',
        'DB_PORT: "5432"',
        'DB_NAME: ${POSTGRES_DB:-lagrange}',
        'DB_USER: research_writer',
        'DB_PASSWORD_FILE: /run/secrets/db_research_password',
        '- db_research_password',
        'test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]'
    )) { Assert-Contains $worker $required 'research-worker service' }

    if ($worker -match '(?i)time\.sleep|\bsleep\b|python\s+-c') {
        throw 'research-worker service still contains a sleep/Python placeholder'
    }
    $rawMounts = @($worker -split "`r?`n" | Where-Object {
        $_ -match '^\s*-\s+[^#]+:/data/raw(?::[^#\s]+)?\s*(?:#.*)?$'
    })
    if ($rawMounts.Count -ne 1) {
        throw "research-worker must have exactly one volume targeting /data/raw; found $($rawMounts.Count)"
    }
    $rawMount = $rawMounts[0].Trim()
    if ($rawMount -notmatch '^-\s+[^#]+:/data/raw(?::rw)?\s*(?:#.*)?$') {
        throw "research-worker Raw mount must be read/write with mode absent or rw: $rawMount"
    }
    foreach ($line in ($worker -split "`r?`n")) {
        if ($line -match ':/data/(curated|nautilus_catalog|artifacts)(?:/[^\s:]*)?(?:\s|$)' -and $line -notmatch ':ro(?:\s|$)') {
            throw "research-worker non-Raw data mount is not read-only: $($line.Trim())"
        }
    }

    Assert-Contains $compose "  db_research_password:`n" 'Compose secrets'
    Assert-Contains $compose 'file: ${LAGRANGE_DB_RESEARCH_PASSWORD_SECRET_SOURCE:-../secrets/db_research_password}' 'research DB secret'
    if ($compose -match '\blagrange_app\b' -or $compose -match '\blagrange_worker\b') {
        throw 'legacy Compose DB role spelling remains (lagrange_app or lagrange_worker)'
    }
    foreach ($identity in @('db_app_password:', 'db_worker_password:', 'db_audit_password:')) {
        Assert-Contains $compose $identity 'existing Compose secret identities'
    }

    if (-not (Test-Path -LiteralPath $dockerfile)) { throw "missing worker Dockerfile: $dockerfile" }
    if (-not (Test-Path -LiteralPath $secretExample)) { throw "missing research DB secret example: $secretExample" }
    $dockerText = Get-Content -Raw -LiteralPath $dockerfile
    if ($dockerText -notmatch '(?im)^FROM\s+rust:1\.97\.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900\s+AS\s+builder\s*$') {
        throw 'Dockerfile missing the approved digest-pinned Rust builder'
    }
    if ($dockerText -notmatch '(?im)^FROM\s+alpine:3\.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d\s*$') {
        throw 'Dockerfile missing the approved digest-pinned Alpine runtime'
    }
    Assert-Contains $dockerText 'cargo build --locked --release --package collectors --bin research-worker' 'Dockerfile'
    Assert-Contains $dockerText 'ENTRYPOINT ["/usr/local/bin/research-worker"]' 'Dockerfile'
    $fromLines = @($dockerText -split "`r?`n" | Where-Object { $_ -match '(?i)^FROM\s+' })
    if ($fromLines.Count -eq 0) { throw 'Dockerfile has no FROM instructions' }
    foreach ($line in $fromLines) {
        if ($line -notmatch '(?i)^FROM\s+[^\s]+@sha256:[0-9a-f]{64}(?:\s+AS\s+[A-Za-z0-9._-]+)?$') {
            throw "Dockerfile FROM is not immutable: $line"
        }
    }

    $trackedSecrets = @(& git -C $root ls-files -- deploy/secrets)
    if ($LASTEXITCODE -ne 0) { throw 'git ls-files failed while checking secrets' }
    foreach ($path in $trackedSecrets) {
        $name = [IO.Path]::GetFileName($path)
        if ($name -ne 'README.md' -and $name -ne '.gitignore' -and -not $name.EndsWith('.example', [StringComparison]::Ordinal)) {
            throw "real secret-like file is tracked: $path"
        }
    }
    Write-Host 'RESEARCH_WORKER_SMOKE: static PASS'
}

function New-RandomSecret {
    $bytes = [byte[]]::new(32)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    [Convert]::ToBase64String($bytes)
}

if ($SelfTest) {
    Invoke-ValidatorSelfTests
    exit 0
}

Invoke-StaticChecks
if ($StaticOnly -or $env:LAGRANGE_RESEARCH_SMOKE_STATIC_ONLY -eq '1') {
    Write-Host 'RESEARCH_WORKER_SMOKE: functional SKIPPED (explicit static-only request)'
    exit 0
}

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw 'Docker is required for the functional phase; use -StaticOnly only for an explicit static check'
}
& docker info *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'Docker daemon is unavailable; use -StaticOnly only for an explicit static check'
}
& docker compose version *> $null
if ($LASTEXITCODE -ne 0) { throw 'Docker Compose is unavailable' }

$project = "lagrange-research-smoke-$PID-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) $project
$rawRoot = Join-Path $tempRoot 'data'
$postgresSecret = Join-Path $tempRoot 'postgres_password'
$researchSecret = Join-Path $tempRoot 'db_research_password'
$krxSecret = Join-Path $tempRoot 'krx_api_key'
$created = $false

function Invoke-ResearchCompose {
    & docker compose -p $project -f $composeFile @args
}

function Invoke-Psql([string]$Sql) {
    $Sql | & docker compose -p $project -f $composeFile exec -T postgres psql -X -v ON_ERROR_STOP=1 -U lagrange -d lagrange
    if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL command failed' }
}

function Get-PublicationCounts {
    $sql = @'
SELECT concat_ws('|',
  (SELECT count(DISTINCT source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31' AND source_batch_id IS NOT NULL),
  (SELECT count(*) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX' AND source_batch_id IN (SELECT source_batch_id FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31')),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX' AND source_batch_id IN (SELECT source_batch_id FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'))
);
'@
    $value = $sql | & docker compose -p $project -f $composeFile exec -T postgres psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange
    if ($LASTEXITCODE -ne 0) { throw 'publication count query failed' }
    $result = "$value".Trim()
    if ($result -notmatch '^\d+\|\d+\|\d+\|\d+$') { throw "unexpected publication count result: $result" }
    $parts = @($result.Split('|') | ForEach-Object { [int64]$_ })
    if ($parts | Where-Object { $_ -le 0 }) { throw "publication evidence is incomplete: $result" }
    $result
}

try {
    New-Item -ItemType Directory -Path (Join-Path $rawRoot 'raw') -Force | Out-Null
    [IO.File]::WriteAllText($postgresSecret, (New-RandomSecret))
    [IO.File]::WriteAllText($researchSecret, (New-RandomSecret))
    [IO.File]::WriteAllText($krxSecret, 'unused-in-synthetic-smoke')
    $env:LAGRANGE_POSTGRES_PASSWORD_SECRET_SOURCE = $postgresSecret
    $env:LAGRANGE_DB_RESEARCH_PASSWORD_SECRET_SOURCE = $researchSecret
    $env:LAGRANGE_KRX_API_KEY_SECRET_SOURCE = $krxSecret
    $env:LAGRANGE_DATA_DIR = $rawRoot
    $env:LAGRANGE_PGDATA_VOLUME = "$project-pgdata"
    $env:POSTGRES_USER = 'lagrange'
    $env:POSTGRES_DB = 'lagrange'
    $env:APP_ENV = 'qa'
    $env:RESEARCH_FETCH_MODE = 'synthetic'
    $env:RESEARCH_MAX_PUBLICATION_AGE_SECS = '315576000'
    $env:RESEARCH_RUN_AT_KST = ([DateTimeOffset]::UtcNow.ToOffset([TimeSpan]::FromHours(9)).AddHours(12)).ToString('HH:mm')

    $created = $true
    Invoke-ResearchCompose up -d --wait postgres | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL did not become healthy' }

    $researchPassword = [IO.File]::ReadAllText($researchSecret).Replace("'", "''")
    $roleSql = @"
DO `$roles`$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$researchPassword'; END IF;
END
`$roles`$;
"@
    Invoke-Psql $roleSql | Out-Null
    $researchPassword = $null

    foreach ($migration in (Get-ChildItem -LiteralPath (Join-Path $root 'migrations') -Filter '*.up.sql' | Sort-Object Name)) {
        Get-Content -Raw -LiteralPath $migration.FullName | & docker compose -p $project -f $composeFile exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange
        if ($LASTEXITCODE -ne 0) { throw "migration failed: $($migration.Name)" }
    }

    Invoke-ResearchCompose build research-worker
    if ($LASTEXITCODE -ne 0) { throw 'research-worker image build failed' }
    Invoke-ResearchCompose run --rm --no-deps --entrypoint /bin/sh --user 10001:10001 research-worker -c 'probe="$RESEARCH_RAW_ROOT/.qa-write-probe"; : > "$probe"; rm -f "$probe"'
    if ($LASTEXITCODE -ne 0) { throw 'research-worker UID 10001 cannot write the Raw bind mount' }
    Invoke-ResearchCompose up -d --no-deps research-worker | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'research-worker service failed to start' }

    Invoke-ResearchCompose run --rm --no-deps research-worker --once --date 2020-01-31
    if ($LASTEXITCODE -ne 0) { throw 'first research-worker one-shot failed' }

    $healthy = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        & docker compose -p $project -f $composeFile exec -T research-worker /usr/local/bin/research-worker healthcheck *> $null
        if ($LASTEXITCODE -eq 0) { $healthy = $true; break }
        Start-Sleep -Seconds 1
    }
    if (-not $healthy) { throw 'research-worker did not become functionally healthy' }

    $before = Get-PublicationCounts
    Invoke-ResearchCompose run --rm --no-deps research-worker --once --date 2020-01-31
    if ($LASTEXITCODE -ne 0) { throw 'second research-worker one-shot failed' }
    $after = Get-PublicationCounts
    if ($before -ne $after) { throw "idempotency failed: counts changed from $before to $after" }
    Write-Host "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
}
finally {
    if ($created) {
        & docker compose -p $project -f $composeFile down -v --remove-orphans --rmi local *> $null
    }
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
