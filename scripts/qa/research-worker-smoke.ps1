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
$dockerignore = Join-Path $root '.dockerignore'
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
        if (Test-Path -LiteralPath $dockerignore) {
            Copy-Item -LiteralPath $dockerignore -Destination (Join-Path $testRoot '.dockerignore')
        }
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

        foreach ($mutation in @(
            @('DB_USER', '      DB_USER: research_writer', '      # DB_USER: research_writer'),
            @('entrypoint', '    entrypoint: ["/usr/local/bin/research-worker"]', '    # entrypoint: ["/usr/local/bin/research-worker"]'),
            @('healthcheck', '      test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]', '      # test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]')
        )) {
            [IO.File]::WriteAllText($testCompose, $baselineCompose.Replace($mutation[1], $mutation[2]))
            & pwsh -NoProfile -File $testScript -StaticOnly *> $null
            if ($LASTEXITCODE -eq 0) { throw "validator accepted commented-out $($mutation[0])" }
        }
        $writableCurated = $baselineCompose.Replace(
            '      - ${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw',
            "      - `${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw`n      - type: bind`n        source: `${LAGRANGE_DATA_DIR:-../data}/curated`n        target: /data/curated`n        read_only: false"
        )
        [IO.File]::WriteAllText($testCompose, $writableCurated)
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a writable long-syntax curated mount' }
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
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw 'Docker Compose CLI is required for semantic static validation'
    }
    $resolvedLines = @(& docker compose -f $composeFile config --format json)
    if ($LASTEXITCODE -ne 0) { throw 'docker compose config failed during static validation' }
    $resolvedJson = $resolvedLines -join "`n"
    try { $model = $resolvedJson | ConvertFrom-Json } catch { throw 'docker compose config returned invalid JSON' }

    foreach ($serviceName in @('research-worker', 'research-raw-init', 'research-schema-check')) {
        if (-not $model.services.PSObject.Properties[$serviceName]) {
            throw "Compose service is missing: $serviceName"
        }
    }
    $worker = $model.services.'research-worker'
    if (-not $worker.build -or [IO.Path]::GetFullPath($worker.build.context).TrimEnd('\', '/') -ne [IO.Path]::GetFullPath($root).TrimEnd('\', '/')) {
        throw 'research-worker build context does not resolve to the repository root'
    }
    if ($worker.build.dockerfile -ne 'data-pipelines/collectors/Dockerfile') { throw 'research-worker Dockerfile is incorrect' }
    if ((ConvertTo-Json -Compress -InputObject @($worker.entrypoint)) -ne '["/usr/local/bin/research-worker"]') {
        throw 'research-worker entrypoint is incorrect'
    }
    $requiredEnvironment = [ordered]@{
        APP_ENV = 'development'; RESEARCH_FETCH_MODE = 'synthetic'; RESEARCH_RUN_AT_KST = '16:30'
        RESEARCH_MAX_PUBLICATION_AGE_SECS = '345600'; RESEARCH_RAW_ROOT = '/data/raw'
        DB_HOST = 'postgres'; DB_PORT = '5432'; DB_NAME = 'lagrange'; DB_USER = 'research_writer'
        DB_PASSWORD_FILE = '/run/secrets/db_research_password'
    }
    foreach ($item in $requiredEnvironment.GetEnumerator()) {
        if ($worker.environment.PSObject.Properties[$item.Key].Value -ne $item.Value) {
            throw "research-worker environment is incorrect: $($item.Key)"
        }
    }
    $workerSecrets = @($worker.secrets | ForEach-Object source)
    foreach ($name in @('db_research_password', 'krx_api_key')) {
        if ($workerSecrets -notcontains $name) { throw "research-worker secret is missing: $name" }
    }
    if ((ConvertTo-Json -Compress -InputObject @($worker.healthcheck.test)) -ne '["CMD","/usr/local/bin/research-worker","healthcheck"]') {
        throw 'research-worker healthcheck is incorrect'
    }
    foreach ($dependency in @(
        @('postgres', 'service_healthy'),
        @('research-raw-init', 'service_completed_successfully'),
        @('research-schema-check', 'service_completed_successfully')
    )) {
        if ($worker.depends_on.PSObject.Properties[$dependency[0]].Value.condition -ne $dependency[1]) {
            throw "research-worker dependency is incorrect: $($dependency[0])"
        }
    }
    $rawVolumes = @($worker.volumes | Where-Object target -eq '/data/raw')
    if ($rawVolumes.Count -ne 1 -or $rawVolumes[0].type -ne 'bind' -or $rawVolumes[0].read_only -eq $true) {
        throw 'research-worker must have exactly one read/write bind targeting /data/raw'
    }
    foreach ($volume in @($worker.volumes)) {
        if ($volume.target -like '/data/*' -and $volume.target -ne '/data/raw' -and $volume.read_only -ne $true) {
            throw "research-worker data mount is writable: $($volume.target)"
        }
    }

    $rawInit = $model.services.'research-raw-init'
    $alpineImage = 'alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d'
    $initRaw = @($rawInit.volumes | Where-Object target -eq '/data/raw')
    if ($rawInit.image -ne $alpineImage -or $rawInit.user -ne '0:0' -or $rawInit.read_only -ne $true -or
        $rawInit.network_mode -ne 'none' -or $rawInit.restart -ne 'no' -or $initRaw.Count -ne 1 -or
        $initRaw[0].read_only -eq $true -or $initRaw[0].source -ne $rawVolumes[0].source -or
        ($rawInit.PSObject.Properties.Name -contains 'secrets') -or ($rawInit.PSObject.Properties.Name -contains 'networks')) {
        throw 'research-raw-init isolation or Raw ownership contract is incorrect'
    }
    $initCommand = @($rawInit.command) -join ' '
    if ($initCommand -notmatch 'chown 10001:10001 /data/raw' -or $initCommand -notmatch 'chmod 0750 /data/raw') {
        throw 'research-raw-init command is incorrect'
    }

    $schema = $model.services.'research-schema-check'
    $postgresImage = 'postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a'
    $schemaSecrets = @($schema.secrets | ForEach-Object source)
    $schemaCommand = @($schema.command) -join "`n"
    if ($schema.image -ne $postgresImage -or $schema.read_only -ne $true -or $schema.restart -ne 'no' -or
        $schema.depends_on.postgres.condition -ne 'service_healthy' -or $schemaSecrets -notcontains 'postgres_password') {
        throw 'research-schema-check runtime contract is incorrect'
    }
    foreach ($required in @('data_batches', 'trading_calendar_versions', 'trading_calendars', 'data_batches_source_file_uq', 'trading_calendar_versions_source_lookup_idx', 'research_writer')) {
        Assert-Contains $schemaCommand $required 'research-schema-check command'
    }

    foreach ($identity in @('db_app_password', 'db_worker_password', 'db_audit_password', 'db_research_password')) {
        if (-not $model.secrets.PSObject.Properties[$identity]) { throw "Compose secret identity is missing: $identity" }
    }
    if ($resolvedJson -match '\blagrange_app\b' -or $resolvedJson -match '\blagrange_worker\b') {
        throw 'legacy Compose DB role spelling remains (lagrange_app or lagrange_worker)'
    }

    if (-not (Test-Path -LiteralPath $dockerfile)) { throw "missing worker Dockerfile: $dockerfile" }
    if (-not (Test-Path -LiteralPath $dockerignore)) { throw "missing Docker build-context policy: $dockerignore" }
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
    $ignoreLines = @(Get-Content -LiteralPath $dockerignore | ForEach-Object Trim)
    foreach ($pattern in @('**', '!Cargo.toml', '!Cargo.lock', '!rust-toolchain.toml', '!crates/**', '!data-pipelines/collectors/**', '!apps/api-server/auth/**', '!tests/integration/migration-contract/**', '!tests/fixtures/kr-etf/contract/**', '**/target/**', '**/.git/**', '**/.worktrees/**', '**/.env.*', '**/credentials/**', '**/secrets/**', '**/raw/**')) {
        if ($ignoreLines -notcontains $pattern) { throw "Docker build-context policy is missing: $pattern" }
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
$contextAuditTag = "$project-context-audit"
$created = $false

function Invoke-ResearchCompose {
    & docker compose -p $project -f $composeFile @args
}

function Invoke-Psql([string]$Sql) {
    $Sql | & docker compose -p $project -f $composeFile exec -T postgres psql -X -v ON_ERROR_STOP=1 -U lagrange -d lagrange
    if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL command failed' }
}

function Get-PublicationEvidence {
    $sql = @'
WITH source AS (
  SELECT source_batch_id AS id
  FROM data_batches
  WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'
  LIMIT 1
)
SELECT concat_ws('|',
  (SELECT count(DISTINCT source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31' AND source_batch_id IS NOT NULL),
  (SELECT count(*) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT string_agg(DISTINCT kind, ',' ORDER BY kind) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT bool_and(v.source_batch_id = source.id) FROM trading_calendar_versions v CROSS JOIN source WHERE v.exchange = 'KRX'),
  (SELECT bool_and(c.source_batch_id = source.id) FROM trading_calendars c CROSS JOIN source WHERE c.exchange = 'KRX')
) FROM source;
'@
    $value = $sql | & docker compose -p $project -f $composeFile exec -T postgres psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange
    if ($LASTEXITCODE -ne 0) { throw 'publication evidence query failed' }
    $result = "$value".Trim()
    $expected = '1|4|2|2|CALENDAR,CORPORATE_ACTIONS,EOD,REFERENCE|2020-01-30:TRADING,2020-01-31:TRADING|2020-01-30:TRADING,2020-01-31:TRADING|t|t'
    if ($result -ne $expected) { throw "publication evidence mismatch: $result" }
    $result
}

function Invoke-BuildContextAudit {
    $auditRoot = Join-Path $tempRoot 'context-audit'
    $auditDockerfile = Join-Path $tempRoot 'context-audit.Dockerfile'
    foreach ($directory in @('target', 'credentials', 'secrets', 'data/raw')) {
        New-Item -ItemType Directory -Path (Join-Path $auditRoot $directory) -Force | Out-Null
    }
    Copy-Item -LiteralPath $dockerignore -Destination (Join-Path $auditRoot '.dockerignore')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'Cargo.toml'), '[workspace]')
    [IO.File]::WriteAllText((Join-Path $auditRoot '.env'), 'sentinel-not-a-secret')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'target/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'credentials/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'secrets/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'data/raw/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText($auditDockerfile, @'
FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d
COPY . /context
RUN test -f /context/Cargo.toml \
 && test ! -e /context/.env \
 && test ! -e /context/target/sentinel \
 && test ! -e /context/credentials/sentinel \
 && test ! -e /context/secrets/sentinel \
 && test ! -e /context/data/raw/sentinel
'@)
    & docker build --no-cache -q -t $contextAuditTag -f $auditDockerfile $auditRoot | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Docker build-context sentinel audit failed' }
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
    Invoke-BuildContextAudit
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
    Invoke-ResearchCompose run --rm --no-deps research-raw-init
    if ($LASTEXITCODE -ne 0) { throw 'research-raw-init failed' }
    Invoke-ResearchCompose run --rm --no-deps --entrypoint /bin/sh --user 10001:10001 research-worker -c 'probe="$RESEARCH_RAW_ROOT/.qa-write-probe"; : > "$probe"; rm -f "$probe"'
    if ($LASTEXITCODE -ne 0) { throw 'research-worker UID 10001 cannot write the Raw bind mount' }
    Invoke-ResearchCompose run --rm --no-deps research-schema-check
    if ($LASTEXITCODE -ne 0) { throw 'research-schema-check rejected the migrated database' }
    Invoke-ResearchCompose up -d research-worker | Out-Null
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

    $before = Get-PublicationEvidence
    Invoke-ResearchCompose run --rm --no-deps research-worker --once --date 2020-01-31
    if ($LASTEXITCODE -ne 0) { throw 'second research-worker one-shot failed' }
    $after = Get-PublicationEvidence
    if ($before -ne $after) { throw "idempotency failed: counts changed from $before to $after" }
    Write-Host "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
}
finally {
    if ($created) {
        & docker compose -p $project -f $composeFile down -v --remove-orphans --rmi local *> $null
    }
    & docker image rm -f $contextAuditTag *> $null
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
