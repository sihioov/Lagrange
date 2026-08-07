<#
.SYNOPSIS
  Produce a policy-valid Lagrange Station backup set (plan Todo 33).
  PowerShell twin of scripts/backup/create.sh.

.DESCRIPTION
  Brings up the disposable drill Compose project (deploy/backup/compose/
  drill.compose.yml), takes a real pg_basebackup with a real WAL archive,
  encrypts the DB classes, writes Raw/Curated/Artifact increments, emits a
  manifest, and REFUSES to report success unless scripts/backup/validate-policy
  accepts the result. A backup that would be rejected at restore time is not a
  backup; failing here is the whole point.

  All PostgreSQL, hashing, and encryption work runs inside the pinned
  postgres:18.4 container via scripts/backup/lib/create-inside.sh - the SAME
  engine the bash twin uses - so the two drivers cannot drift.

  Unlike the bash twin this script needs no MSYS path guards: PowerShell hands
  arguments to docker.exe unmodified.

.OUTPUTS
  Exit 0 backup set created AND policy-valid
  Exit 1 backup or validation failed (the set, if any, is left for inspection)
  Exit 2 usage / environment error
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$RunId,
    [string]$Now,
    [string]$Key = 'lagrange-drill-key',
    # Preferred for scheduled runs: a passphrase passed as an argument is
    # visible to every user in the process list.
    [string]$KeyFile,
    [string]$Metrics
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$composeFile = Join-Path $root 'deploy/backup/compose/drill.compose.yml'
$inside = Join-Path $root 'scripts/backup/lib/create-inside.sh'

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Error 'ENV ERROR: docker not found on PATH'; exit 2
}
if (-not (Test-Path $composeFile)) { Write-Error "ENV ERROR: drill compose file missing: $composeFile"; exit 2 }
if (-not (Test-Path $inside)) { Write-Error "ENV ERROR: backup engine missing: $inside"; exit 2 }

if ($KeyFile) {
    if (-not (Test-Path -PathType Leaf $KeyFile)) { Write-Error "ENV ERROR: key file not found: $KeyFile"; exit 2 }
    $Key = (Get-Content -Raw $KeyFile).Trim()
    if (-not $Key) { Write-Error "ENV ERROR: key file is empty: $KeyFile"; exit 2 }
}
if (-not $RunId) { $RunId = "lagrange-drill-$((Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ'))-$PID" }
if (-not $Now) { $Now = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }

# The run id doubles as the Compose project name, so two drills - or a drill and
# the production stack - can never share a volume or container.
$project = ($RunId.ToLowerInvariant() -replace '[^a-z0-9-]', '-')

# Deliberately a SIMPLE function using the automatic $args, not an advanced
# function with [Parameter(ValueFromRemainingArguments)]. An advanced function
# treats a leading-dash token as a parameter name and SILENTLY DROPS it when it
# matches nothing, so `up -d` would arrive as `up`.
function Invoke-Compose { & docker compose -p $project -f $composeFile @args }

$startedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$gateRc = 1
$gateOut = ''

try {
    Write-Host "== drill project: $project =="
    Invoke-Compose up -d --wait source
    if ($LASTEXITCODE -ne 0) {
        Write-Error 'BACKUP FAILED: the source cluster did not become healthy'; exit 1
    }

    # Copy the engine in and exec it by path rather than piping it to `bash -s`.
    # PowerShell does not reliably surface a native command's exit code when it
    # sits at the end of a pipeline, and a backup driver that cannot tell
    # success from failure is worse than no driver at all. The bash twin pipes
    # because there $? is dependable; both run the SAME engine file.
    $cid = (Invoke-Compose ps -q source | Select-Object -First 1)
    if (-not $cid) { Write-Error 'BACKUP FAILED: source container id not resolvable'; exit 1 }
    & docker cp $inside "${cid}:/tmp/create-inside.sh"
    if ($LASTEXITCODE -ne 0) { Write-Error 'BACKUP FAILED: could not stage the backup engine'; exit 1 }
    & docker compose -p $project -f $composeFile exec -T `
        -e RUN_ID=$RunId -e NOW=$Now -e BACKUP_KEY=$Key -e OUT=/backup/set `
        source bash /tmp/create-inside.sh
    if ($LASTEXITCODE -ne 0) {
        Write-Error 'BACKUP FAILED: the backup engine reported an error'; exit 1
    }

    New-Item -ItemType Directory -Force -Path $Out | Out-Null
    $setDir = Join-Path $Out 'set'
    $sidecar = Join-Path $Out 'backup-sidecar.json'
    if (Test-Path $setDir) { Remove-Item -Recurse -Force $setDir }
    if (Test-Path $sidecar) { Remove-Item -Force $sidecar }

    & docker cp "${cid}:/backup/set" $setDir
    if ($LASTEXITCODE -ne 0) { Write-Error 'BACKUP FAILED: could not copy the set out'; exit 1 }
    & docker cp "${cid}:/backup/backup-sidecar.json" $sidecar
    if ($LASTEXITCODE -ne 0) { Write-Error 'BACKUP FAILED: could not copy the sidecar out'; exit 1 }

    # --- self-verification: the policy gate must accept what we just wrote ---
    Write-Host '== policy gate =='
    $validator = Join-Path $root 'scripts/backup/validate-policy.ps1'
    $gateOut = & $validator -SetPath $setDir -Gate default 2>&1 | Out-String
    $gateRc = $LASTEXITCODE
    Write-Host $gateOut
}
finally {
    # A drill NEVER leaves a cluster running: "no production activation" starts
    # with not leaving stray clusters behind.
    Invoke-Compose down -v --remove-orphans 2>&1 | Out-Null
}

$duration = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() - $startedAt

# Prometheus textfile-collector format. Written on success AND failure: a backup
# that stopped running is exactly what the staleness alert must catch, and a
# file that only appears on success can never go stale.
if ($Metrics) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Metrics) | Out-Null
    $nowEpoch = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $successEpoch = if ($gateRc -eq 0) { $nowEpoch } else { 0 }
    @"
# HELP lagrange_backup_last_run_timestamp_seconds Unix time of the last backup attempt.
# TYPE lagrange_backup_last_run_timestamp_seconds gauge
lagrange_backup_last_run_timestamp_seconds $nowEpoch
# HELP lagrange_backup_last_success_timestamp_seconds Unix time of the last backup that passed the policy gate.
# TYPE lagrange_backup_last_success_timestamp_seconds gauge
lagrange_backup_last_success_timestamp_seconds $successEpoch
# HELP lagrange_backup_duration_seconds Wall-clock duration of the last backup attempt.
# TYPE lagrange_backup_duration_seconds gauge
lagrange_backup_duration_seconds $duration
# HELP lagrange_backup_exit_code Exit code of the last backup attempt (0 = policy-valid).
# TYPE lagrange_backup_exit_code gauge
lagrange_backup_exit_code $gateRc
"@ | Set-Content -Path $Metrics -Encoding utf8
}

if ($gateRc -ne 0) {
    Write-Error "BACKUP FAILED: the set was created but the policy gate rejected it (exit $gateRc)"
    exit 1
}
Write-Host "BACKUP OK: $RunId -> $(Join-Path $Out 'set') (${duration}s)"
exit 0
