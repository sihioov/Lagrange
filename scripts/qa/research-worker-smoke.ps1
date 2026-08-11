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
$gitattributes = Join-Path $root '.gitattributes'
$schemaSqlFile = Join-Path $root 'deploy/compose/research-schema-check.sql'
$secretExample = Join-Path $root 'deploy/secrets/db_research_password.example'
$readOnlyFsyncProbe = Join-Path $root 'scripts/qa/read-only-fsync.rs'

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
        Copy-Item -LiteralPath (Join-Path $root 'scripts/qa/research-worker-smoke.sh') -Destination (Join-Path $testRoot 'scripts/qa/research-worker-smoke.sh')
        Copy-Item -LiteralPath $readOnlyFsyncProbe -Destination (Join-Path $testRoot 'scripts/qa/read-only-fsync.rs')
        Copy-Item -LiteralPath $composeFile -Destination (Join-Path $testRoot 'deploy/compose/compose.yml')
        Copy-Item -LiteralPath $dockerfile -Destination (Join-Path $testRoot 'data-pipelines/collectors/Dockerfile')
        Copy-Item -LiteralPath $schemaSqlFile -Destination (Join-Path $testRoot 'deploy/compose/research-schema-check.sql')
        if (Test-Path -LiteralPath $dockerignore) {
            Copy-Item -LiteralPath $dockerignore -Destination (Join-Path $testRoot '.dockerignore')
        }
        if (Test-Path -LiteralPath $gitattributes) {
            Copy-Item -LiteralPath $gitattributes -Destination (Join-Path $testRoot '.gitattributes')
        }
        foreach ($name in @('.gitignore', 'README.md', 'db_research_password.example')) {
            Copy-Item -LiteralPath (Join-Path $root "deploy/secrets/$name") -Destination (Join-Path $testRoot "deploy/secrets/$name")
        }
        & git -C $testRoot init -q
        if ($LASTEXITCODE -ne 0) { throw 'self-test git init failed' }
        & git -C $testRoot add -f -- deploy/secrets *> $null
        if ($LASTEXITCODE -ne 0) { throw 'self-test git add failed' }

        & git -C $testRoot config core.autocrlf true
        $testSh = Join-Path $testRoot 'scripts/qa/research-worker-smoke.sh'
        $crlf = (Get-Content -Raw -LiteralPath $testSh).Replace("`r`n", "`n").Replace("`n", "`r`n")
        [IO.File]::WriteAllText($testSh, $crlf)
        & git -C $testRoot add -- .gitattributes scripts/qa/research-worker-smoke.sh
        & git -C $testRoot -c user.name=validator -c user.email=validator@example.invalid commit -q -m 'checkout fixture'
        if ($LASTEXITCODE -ne 0) { throw 'self-test checkout fixture commit failed' }
        Remove-Item -LiteralPath $testSh
        & git -C $testRoot checkout -q -- scripts/qa/research-worker-smoke.sh
        if ($LASTEXITCODE -ne 0 -or [IO.File]::ReadAllBytes($testSh) -contains 13) {
            throw '.gitattributes did not preserve LF under a simulated autocrlf checkout'
        }

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

        [IO.File]::WriteAllText(
            (Join-Path $testRoot 'scripts/qa/read-only-fsync.rs'),
            (Get-Content -Raw -LiteralPath $readOnlyFsyncProbe).Replace('File::open(&path)', 'OpenOptions::new().write(true).open(&path)')
        )
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a write-opening Raw fsync probe' }
        Copy-Item -LiteralPath $readOnlyFsyncProbe -Destination (Join-Path $testRoot 'scripts/qa/read-only-fsync.rs') -Force

        Add-Content -LiteralPath (Join-Path $testRoot '.dockerignore') -Value '!scripts/**'
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted the QA fsync probe in the worker build context' }
        Copy-Item -LiteralPath $dockerignore -Destination (Join-Path $testRoot '.dockerignore') -Force

        [IO.File]::WriteAllText(
            $testCompose,
            $baselineCompose.Replace('find /data/raw -xdev -type d', 'find -L /data/raw -type l')
        )
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a symlink-following Raw init' }
        [IO.File]::WriteAllText($testCompose, $baselineCompose)

        $testSchemaSql = Join-Path $testRoot 'deploy/compose/research-schema-check.sql'
        $baselineSchemaSql = Get-Content -Raw -LiteralPath $testSchemaSql
        [IO.File]::WriteAllText($testSchemaSql, $baselineSchemaSql.Replace('has_sequence_privilege', 'has_sequence_permission'))
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted a weakened schema gate' }
        [IO.File]::WriteAllText($testSchemaSql, $baselineSchemaSql)

        $testAttributes = Join-Path $testRoot '.gitattributes'
        [IO.File]::WriteAllText($testAttributes, 'scripts/qa/*.sh text eol=crlf')
        & pwsh -NoProfile -File $testScript -StaticOnly *> $null
        if ($LASTEXITCODE -eq 0) { throw 'validator accepted CRLF shell checkout semantics' }
        Copy-Item -LiteralPath $gitattributes -Destination $testAttributes -Force

        foreach ($mutation in @(
            @('DB_USER', '      DB_USER: research_writer', '      # DB_USER: research_writer'),
            @('entrypoint', '    entrypoint: ["/usr/local/bin/research-worker"]', '    # entrypoint: ["/usr/local/bin/research-worker"]'),
            @('healthcheck', '      test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]', '      # test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]'),
            @('raw-init-caps', '      - DAC_OVERRIDE', '      - SETUID'),
            @('schema-user', '    user: "999:999"', '    user: "0:0"'),
            @('schema-caps', "    user: `"999:999`"`n    cap_drop:`n      - ALL", "    user: `"999:999`"`n    cap_drop:`n      - CHOWN")
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
        RESEARCH_MAX_PUBLICATION_AGE_SECS = '345600'; RESEARCH_RAW_ROOT = '/data'
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
    $rawInitCapabilities = @($rawInit.cap_add | Sort-Object)
    if ($rawInit.image -ne $alpineImage -or $rawInit.user -ne '0:0' -or $rawInit.read_only -ne $true -or
        $rawInit.network_mode -ne 'none' -or $rawInit.restart -ne 'no' -or $initRaw.Count -ne 1 -or
        $initRaw[0].read_only -eq $true -or $initRaw[0].source -ne $rawVolumes[0].source -or
        @($rawInit.cap_drop).Count -ne 1 -or @($rawInit.cap_drop)[0] -ne 'ALL' -or
        ($rawInitCapabilities -join ',') -ne 'CHOWN,DAC_OVERRIDE,FOWNER' -or
        @($rawInit.security_opt) -notcontains 'no-new-privileges:true' -or
        ($rawInit.PSObject.Properties.Name -contains 'secrets') -or ($rawInit.PSObject.Properties.Name -contains 'networks')) {
        throw 'research-raw-init isolation or Raw ownership contract is incorrect'
    }
    $initCommand = @($rawInit.command) -join ' '
    if ($initCommand -notmatch 'find /data/raw -xdev -type d' -or
        $initCommand -notmatch 'find /data/raw -xdev -type f' -or
        $initCommand -notmatch 'manifest.jsonl' -or $initCommand -notmatch 'commit.lock' -or
        $initCommand -notmatch 'chown 10001:10001' -or
        $initCommand -notmatch 'chmod 0750' -or $initCommand -notmatch 'chmod 0640' -or
        $initCommand -notmatch 'chmod 0440' -or
        $initCommand -match '(?:^|\s)-L(?:\s|$)' -or $initCommand -match '-type l') {
        throw 'research-raw-init command is incorrect'
    }

    $schema = $model.services.'research-schema-check'
    $postgresImage = 'postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a'
    $schemaSecrets = @($schema.secrets | ForEach-Object source)
    $schemaCommand = @($schema.command) -join "`n"
    $schemaVolumes = @($schema.volumes | Where-Object target -eq '/opt/lagrange/research-schema-check.sql')
    if ($schema.image -ne $postgresImage -or $schema.read_only -ne $true -or $schema.restart -ne 'no' -or
        $schema.user -ne '999:999' -or @($schema.cap_drop) -notcontains 'ALL' -or
        @($schema.security_opt) -notcontains 'no-new-privileges:true' -or
        $schema.depends_on.postgres.condition -ne 'service_healthy' -or
        $schemaSecrets.Count -ne 1 -or $schemaSecrets -notcontains 'postgres_password' -or
        $schemaVolumes.Count -ne 1 -or $schemaVolumes[0].read_only -ne $true) {
        throw 'research-schema-check runtime contract is incorrect'
    }
    Assert-Contains $schemaCommand '/opt/lagrange/research-schema-check.sql' 'research-schema-check command'
    if (-not (Test-Path -LiteralPath $schemaSqlFile)) { throw "missing tracked schema gate: $schemaSqlFile" }
    $schemaSql = Get-Content -Raw -LiteralPath $schemaSqlFile
    foreach ($required in @(
        '_sqlx_migrations', 'version IN (22, 23, 24, 25, 33)', 'convalidated',
        'pg_get_constraintdef', 'format_type', 'attnotnull', 'attidentity',
        'pg_get_expr', 'storage_path', 'EXCEPT',
        'data_batches_source_file_uq', 'trading_calendar_versions_source_lookup_idx',
        'indisunique', 'indisvalid', 'indisready', 'indislive', 'relrowsecurity',
        'research_writer', 'rolcanlogin', 'rolsuper', 'rolbypassrls', 'rolcreatedb',
        'rolcreaterole', 'pg_auth_members', 'pg_policy', 'polcmd', 'polpermissive',
        'trading_calendar_versions_append_only', 'tgenabled', 'tgtype', 'prosecdef',
        'pg_get_functiondef', 'regexp_replace', 'actual_function', 'expected_function',
        'role_table_grants', 'has_schema_privilege', 'has_table_privilege',
        'has_sequence_privilege', 'MAINTAIN'
    )) {
        Assert-Contains $schemaSql $required 'research-schema-check SQL'
    }

    foreach ($identity in @('db_app_password', 'db_worker_password', 'db_audit_password', 'db_research_password')) {
        if (-not $model.secrets.PSObject.Properties[$identity]) { throw "Compose secret identity is missing: $identity" }
    }
    if ($resolvedJson -match '\blagrange_app\b' -or $resolvedJson -match '\blagrange_worker\b') {
        throw 'legacy Compose DB role spelling remains (lagrange_app or lagrange_worker)'
    }

    if (-not (Test-Path -LiteralPath $dockerfile)) { throw "missing worker Dockerfile: $dockerfile" }
    if (-not (Test-Path -LiteralPath $dockerignore)) { throw "missing Docker build-context policy: $dockerignore" }
    if (-not (Test-Path -LiteralPath $readOnlyFsyncProbe)) { throw "missing read-only fsync probe: $readOnlyFsyncProbe" }
    if (-not (Test-Path -LiteralPath $secretExample)) { throw "missing research DB secret example: $secretExample" }
    $probeText = Get-Content -Raw -LiteralPath $readOnlyFsyncProbe
    foreach ($required in @('File::open(&path)', 'file.sync_all()')) {
        Assert-Contains $probeText $required 'read-only fsync probe'
    }
    if ($probeText -match 'OpenOptions' -or $probeText -match '\.write\s*\(') {
        throw 'read-only fsync probe must not request write access'
    }
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
    foreach ($pattern in @('**', '!Cargo.toml', '!Cargo.lock', '!rust-toolchain.toml', '!crates/**', '!data-pipelines/collectors/**', '!apps/api-server/auth/**', '!tests/integration/migration-contract/**', '!tests/fixtures/kr-etf/contract/**', '**/target/**', '**/.git/**', '**/.worktrees/**', '**/.env.*', '**/credentials/**', '**/secrets/**', '**/raw/**', '**/*.pem', '**/*.key', '**/*.p12', '**/*.pfx')) {
        if ($ignoreLines -notcontains $pattern) { throw "Docker build-context policy is missing: $pattern" }
    }
    if (@($ignoreLines | Where-Object { $_ -match '^!scripts(?:/|$)' }).Count -ne 0) {
        throw 'QA fsync probe must remain outside the worker build context'
    }
    if (-not (Test-Path -LiteralPath $gitattributes) -or
        (Get-Content -Raw -LiteralPath $gitattributes) -notmatch '(?m)^scripts/qa/\*\.sh\s+text\s+eol=lf\s*$') {
        throw 'scripts/qa shell scripts must be forced to LF by .gitattributes'
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

function Assert-SchemaGateFails([string]$Mutation) {
    Invoke-ResearchCompose run --rm --no-deps research-schema-check *> $null
    if ($LASTEXITCODE -eq 0) { throw "research-schema-check accepted $Mutation" }
}

function Assert-SchemaGatePasses([string]$Context) {
    Invoke-ResearchCompose run --rm --no-deps research-schema-check
    if ($LASTEXITCODE -ne 0) { throw "research-schema-check rejected $Context" }
}

function Invoke-SchemaGateMutationTests {
    Invoke-Psql 'ALTER TABLE data_batches DROP CONSTRAINT data_batches_fetch_mode_check; ALTER TABLE data_batches ADD CONSTRAINT data_batches_fetch_mode_check CHECK (true);' | Out-Null
    Assert-SchemaGateFails 'a same-name weakened publication CHECK'
    Invoke-Psql "ALTER TABLE data_batches DROP CONSTRAINT data_batches_fetch_mode_check; ALTER TABLE data_batches ADD CONSTRAINT data_batches_fetch_mode_check CHECK (fetch_mode IS NULL OR fetch_mode IN ('synthetic', 'credentialed'));" | Out-Null
    Assert-SchemaGatePasses 'the restored publication CHECK'

    Invoke-Psql 'ALTER TABLE data_batches DROP COLUMN storage_path;' | Out-Null
    Assert-SchemaGateFails 'a dropped publication storage_path column'
    Invoke-Psql 'ALTER TABLE data_batches ADD COLUMN storage_path text NOT NULL;' | Out-Null
    Assert-SchemaGatePasses 'the restored publication storage_path column'

    Invoke-Psql 'DROP INDEX CONCURRENTLY data_batches_source_file_uq; CREATE INDEX data_batches_source_file_uq ON data_batches (provider);' | Out-Null
    Assert-SchemaGateFails 'a drifted same-name index'
    Invoke-Psql 'DROP INDEX CONCURRENTLY data_batches_source_file_uq; CREATE UNIQUE INDEX CONCURRENTLY data_batches_source_file_uq ON data_batches (provider, market, source_batch_id, source_file_name) WHERE source_batch_id IS NOT NULL;' | Out-Null
    Assert-SchemaGatePasses 'the restored source index'

    Invoke-Psql 'DROP POLICY data_batches_insert_research_writer ON data_batches;' | Out-Null
    Assert-SchemaGateFails 'a missing research_writer policy'
    Invoke-Psql 'CREATE POLICY data_batches_insert_research_writer ON data_batches FOR INSERT TO research_writer WITH CHECK (true);' | Out-Null
    Assert-SchemaGatePasses 'the restored research_writer policy'

    Invoke-Psql 'ALTER TABLE trading_calendar_versions DISABLE TRIGGER trading_calendar_versions_append_only;' | Out-Null
    Assert-SchemaGateFails 'a disabled append-only trigger'
    Invoke-Psql 'ALTER TABLE trading_calendar_versions ENABLE TRIGGER trading_calendar_versions_append_only;' | Out-Null
    Assert-SchemaGatePasses 'the restored append-only trigger'

    Invoke-Psql @'
CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
    IF false THEN
        RAISE EXCEPTION
            'trading_calendar_versions is append-only: % is refused', TG_OP
            USING ERRCODE = '55000';
    END IF;
    RETURN NULL;
END
$fn$;
'@ | Out-Null
    Assert-SchemaGateFails 'a same-name message-preserving no-op append-only function'
    Invoke-Psql @'
CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
    RAISE EXCEPTION
        'trading_calendar_versions is append-only: % is refused', TG_OP
        USING ERRCODE = '55000';
END
$fn$;
'@ | Out-Null
    Assert-SchemaGatePasses 'the restored exact append-only function'

    Invoke-Psql 'GRANT DELETE ON orders TO research_writer;' | Out-Null
    Assert-SchemaGateFails 'a forbidden order-table grant'
    Invoke-Psql 'REVOKE DELETE ON orders FROM research_writer;' | Out-Null
    Assert-SchemaGatePasses 'the restored least-privilege role'
}

function Invoke-RawInitOwnershipTest {
    $alpineImage = 'alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d'
    $rustImage = 'rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900'
    $probeId = "lagrange-raw-init-$PID-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
    $rawVolume = "$probeId-raw"
    $outsideVolume = "$probeId-outside"
    $binaryVolume = "$probeId-binary"
    try {
        & docker volume create $rawVolume *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Raw init probe volume creation failed' }
        & docker volume create $outsideVolume *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Raw init outside volume creation failed' }
        & docker volume create $binaryVolume *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Raw init fsync-probe volume creation failed' }
        & docker run --rm --network none --user 0:0 -v "${rawVolume}:/data/raw" -v "${outsideVolume}:/outside" $alpineImage /bin/sh -ec @'
mkdir -p /data/raw/manifests/provider=krx/market=kr /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture
printf '{}\n' > /data/raw/manifests/provider=krx/market=kr/manifest.jsonl
: > /data/raw/manifests/provider=krx/market=kr/commit.lock
printf evidence > /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
chown -R 12345:12345 /data/raw
find /data/raw -type d -exec chmod 0700 {} +
chmod 0600 /data/raw/manifests/provider=krx/market=kr/manifest.jsonl /data/raw/manifests/provider=krx/market=kr/commit.lock /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
printf outside > /outside/sentinel
chown 12345:12345 /outside/sentinel
chmod 0600 /outside/sentinel
ln -s /outside /data/raw/outside-link
'@ *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Raw init ownership fixture setup failed' }

        $resolved = Invoke-ResearchCompose config --format json | ConvertFrom-Json
        $initCommand = @($resolved.services.'research-raw-init'.command) -join "`n"
        & docker run --rm --network none --user 0:0 --cap-drop ALL --cap-add CHOWN --cap-add FOWNER --cap-add DAC_OVERRIDE --security-opt no-new-privileges:true -v "${rawVolume}:/data/raw" -v "${outsideVolume}:/outside" $alpineImage /bin/sh -ec $initCommand
        if ($LASTEXITCODE -ne 0) { throw 'recursive Raw init probe failed' }

        & docker run --rm --network none --user 0:0 --cap-drop ALL --security-opt no-new-privileges:true -v "${root}:/source:ro" -v "${binaryVolume}:/probe" $rustImage rustc -O -o /probe/read-only-fsync /source/scripts/qa/read-only-fsync.rs
        if ($LASTEXITCODE -ne 0) { throw 'read-only fsync probe compilation failed' }

        & docker run --rm --network none --user 10001:10001 --cap-drop ALL --security-opt no-new-privileges:true -v "${rawVolume}:/data/raw" -v "${outsideVolume}:/outside" -v "${binaryVolume}:/probe:ro" $alpineImage /bin/sh -ec @'
evidence=/data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
manifest=/data/raw/manifests/provider=krx/market=kr/manifest.jsonl
lock=/data/raw/manifests/provider=krx/market=kr/commit.lock
test "$(stat -c '%u:%g:%a' "$evidence")" = 10001:10001:440
test "$(stat -c '%u:%g:%a' "$manifest")" = 10001:10001:640
test "$(stat -c '%u:%g:%a' "$lock")" = 10001:10001:640
/probe/read-only-fsync "$evidence"
printf recovered >> /data/raw/manifests/provider=krx/market=kr/manifest.jsonl
printf lock >> /data/raw/manifests/provider=krx/market=kr/commit.lock
'@
        if ($LASTEXITCODE -ne 0) { throw 'UID 10001 cannot use existing Raw files' }
        & docker run --rm --network none --user 0:0 -v "${outsideVolume}:/outside" $alpineImage /bin/sh -ec @'
test "$(cat /outside/sentinel)" = outside
test "$(stat -c '%u:%g:%a' /outside/sentinel)" = 12345:12345:600
'@
        if ($LASTEXITCODE -ne 0) { throw 'Raw init changed the outside symlink target' }
    }
    finally {
        & docker volume rm -f $rawVolume $outsideVolume $binaryVolume *> $null
    }
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
  (SELECT count(source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT bool_and(b.source_batch_id = source.id) FROM data_batches b CROSS JOIN source WHERE b.provider = 'KRX' AND b.market = 'KR' AND b.batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT string_agg(DISTINCT kind, ',' ORDER BY kind) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT bool_and(v.source_batch_id = source.id) FROM trading_calendar_versions v CROSS JOIN source WHERE v.exchange = 'KRX'),
  (SELECT bool_and(
      c.source_batch_id IS NOT NULL
      AND c.content_sha256 IS NOT NULL
      AND c.retrieved_at IS NOT NULL
      AND EXISTS (
        SELECT 1 FROM data_batches batch
        WHERE batch.source_batch_id = c.source_batch_id
      )
      AND EXISTS (
        SELECT 1 FROM trading_calendar_versions history
        WHERE history.exchange = c.exchange
          AND history.session_date = c.session_date
          AND history.session_type = c.session_type
          AND history.timezone = c.timezone
          AND history.source = c.source
          AND history.source_version = c.source_version
          AND history.content_sha256 = c.content_sha256
      )
    ) FROM trading_calendars c WHERE c.exchange = 'KRX')
) FROM source;
'@
    $value = $sql | & docker compose -p $project -f $composeFile exec -T postgres psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange
    if ($LASTEXITCODE -ne 0) { throw 'publication evidence query failed' }
    $result = "$value".Trim()
    $expected = '1|4|4|t|2|2|CALENDAR,CORPORATE_ACTIONS,EOD,REFERENCE|2020-01-30:TRADING,2020-01-31:TRADING|2020-01-30:TRADING,2020-01-31:TRADING|t|t'
    if ($result -ne $expected) { throw "publication evidence mismatch: $result" }
    $result
}

function Invoke-BuildContextAudit {
    $auditRoot = Join-Path $tempRoot 'context-audit'
    $auditDockerfile = Join-Path $tempRoot 'context-audit.Dockerfile'
    foreach ($directory in @(
        'target', 'credentials', 'secrets', 'data/raw', 'crates/sentinel',
        'data-pipelines/collectors/sentinel', 'apps/api-server/auth/sentinel',
        'tests/fixtures/kr-etf/contract/sentinel', 'scripts/qa'
    )) {
        New-Item -ItemType Directory -Path (Join-Path $auditRoot $directory) -Force | Out-Null
    }
    Copy-Item -LiteralPath $dockerignore -Destination (Join-Path $auditRoot '.dockerignore')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'Cargo.toml'), '[workspace]')
    [IO.File]::WriteAllText((Join-Path $auditRoot '.env'), 'sentinel-not-a-secret')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'target/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'credentials/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'secrets/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'data/raw/sentinel'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'crates/sentinel/context.pem'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'data-pipelines/collectors/sentinel/context.key'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'apps/api-server/auth/sentinel/context.p12'), 'must-not-enter-context')
    [IO.File]::WriteAllText((Join-Path $auditRoot 'tests/fixtures/kr-etf/contract/sentinel/context.pfx'), 'must-not-enter-context')
    Copy-Item -LiteralPath $readOnlyFsyncProbe -Destination (Join-Path $auditRoot 'scripts/qa/read-only-fsync.rs')
    [IO.File]::WriteAllText($auditDockerfile, @'
FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d
COPY . /context
RUN test -f /context/Cargo.toml \
 && test ! -e /context/.env \
 && test ! -e /context/target/sentinel \
 && test ! -e /context/credentials/sentinel \
 && test ! -e /context/secrets/sentinel \
 && test ! -e /context/data/raw/sentinel \
 && test ! -e /context/crates/sentinel/context.pem \
 && test ! -e /context/data-pipelines/collectors/sentinel/context.key \
 && test ! -e /context/apps/api-server/auth/sentinel/context.p12 \
 && test ! -e /context/tests/fixtures/kr-etf/contract/sentinel/context.pfx \
 && test ! -e /context/scripts/qa/read-only-fsync.rs
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
    Invoke-RawInitOwnershipTest
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

    Assert-SchemaGateFails 'an unmigrated database'
    Invoke-Psql @'
CREATE TABLE _sqlx_migrations (
  version bigint PRIMARY KEY,
  description text NOT NULL,
  installed_on timestamptz NOT NULL DEFAULT now(),
  success boolean NOT NULL,
  checksum bytea NOT NULL,
  execution_time bigint NOT NULL
);
'@ | Out-Null

    foreach ($migration in (Get-ChildItem -LiteralPath (Join-Path $root 'migrations') -Filter '*.up.sql' | Sort-Object Name)) {
        $migrationSql = Get-Content -Raw -LiteralPath $migration.FullName
        if ($migrationSql -match '(?m)^-- no-transaction\s*$') {
            $migrationSql | & docker compose -p $project -f $composeFile exec -T -e 'PGOPTIONS=-c lock_timeout=5s' postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange
        }
        else {
            $migrationSql | & docker compose -p $project -f $composeFile exec -T -e 'PGOPTIONS=-c lock_timeout=5s' postgres psql -X -q -1 -v ON_ERROR_STOP=1 -U lagrange -d lagrange
        }
        if ($LASTEXITCODE -ne 0) { throw "migration failed: $($migration.Name)" }
        $version = [int64]$migration.BaseName.Substring(0, 4)
        $description = $migration.BaseName.Substring(5).Replace("'", "''")
        Invoke-Psql "INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time) VALUES ($version, '$description', true, decode(repeat('00', 32), 'hex'), 0);" | Out-Null
    }

    Assert-SchemaGatePasses 'the migrated database'
    Invoke-SchemaGateMutationTests

    Invoke-ResearchCompose build research-worker
    if ($LASTEXITCODE -ne 0) { throw 'research-worker image build failed' }

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'cargo is required to prove the manual --root Raw contract' }
    $manualOutput = @(& cargo run --quiet --locked -p collectors --bin collectors -- ingest-krx --root $rawRoot --date 2020-01-31 --mode synthetic --bundle (Join-Path $root 'tests/fixtures/kr-etf/contract') --now '2020-01-31T08:00:00Z')
    if ($LASTEXITCODE -ne 0) { throw 'manual collectors --root ingest failed' }
    $manual = ($manualOutput -join "`n") | ConvertFrom-Json
    $directManifest = Join-Path $rawRoot 'raw/manifests/provider=krx/market=kr/manifest.jsonl'
    if (-not (Test-Path -LiteralPath $directManifest)) { throw "direct host Raw manifest is missing: $directManifest" }
    if (Test-Path -LiteralPath (Join-Path $rawRoot 'raw/raw')) { throw 'Raw evidence was nested under <data>/raw/raw' }
    if ([IO.Path]::GetFullPath($manual.manifest) -ne [IO.Path]::GetFullPath($directManifest)) {
        throw "manual --root manifest mismatch: $($manual.manifest)"
    }

    Invoke-ResearchCompose run --rm --no-deps research-raw-init
    if ($LASTEXITCODE -ne 0) { throw 'research-raw-init failed' }
    Invoke-ResearchCompose run --rm --no-deps --entrypoint /bin/sh --user 10001:10001 research-worker -ec @'
manifest="$RESEARCH_RAW_ROOT/raw/manifests/provider=krx/market=kr/manifest.jsonl"
test -s "$manifest"
: > "$manifest"
test ! -s "$manifest"
probe="$RESEARCH_RAW_ROOT/raw/.qa-write-probe"
: > "$probe"
rm -f "$probe"
'@
    if ($LASTEXITCODE -ne 0) { throw 'research-worker UID 10001 cannot prepare the startup orphan' }
    Invoke-ResearchCompose up -d research-worker | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'research-worker service failed to start' }

    $healthy = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        & docker compose -p $project -f $composeFile exec -T research-worker /usr/local/bin/research-worker healthcheck *> $null
        if ($LASTEXITCODE -eq 0) { $healthy = $true; break }
        Start-Sleep -Seconds 1
    }
    if (-not $healthy) { throw 'research-worker did not become functionally healthy' }
    Invoke-ResearchCompose exec -T -e "EXPECTED_BATCH_ID=$($manual.batch_id)" research-worker /bin/sh -ec @'
manifest="$RESEARCH_RAW_ROOT/raw/manifests/provider=krx/market=kr/manifest.jsonl"
test "$(grep -Fc "$EXPECTED_BATCH_ID" "$manifest")" -eq 1
'@
    if ($LASTEXITCODE -ne 0) { throw 'startup orphan recovery did not restore the exact manifest row' }

    $before = Get-PublicationEvidence
    Invoke-ResearchCompose run --rm --no-deps research-worker --once --date 2020-01-31
    if ($LASTEXITCODE -ne 0) { throw 'second research-worker one-shot failed' }
    $after = Get-PublicationEvidence
    if ($before -ne $after) { throw "idempotency failed: counts changed from $before to $after" }
    Write-Host "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
}
finally {
    if ($created) {
        & docker compose -p $project -f $composeFile run --rm --no-deps --entrypoint /bin/sh --user 0:0 research-raw-init -ec 'find /data/raw -mindepth 1 -delete' *> $null
        & docker compose -p $project -f $composeFile down -v --remove-orphans --rmi local *> $null
    }
    & docker image rm -f $contextAuditTag *> $null
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
