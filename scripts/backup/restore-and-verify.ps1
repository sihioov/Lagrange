<#
.SYNOPSIS
  Restore a backup set into an ISOLATED target and prove it (plan Todo 33).
  PowerShell twin of scripts/backup/restore-and-verify.sh.

.DESCRIPTION
  Order of operations is the contract, not a convenience:
    1. policy gate - validate-policy must exit 0. Nonzero => NO restore command
       runs at all (deploy/backup/runbooks/pre-member-restore-drill.md section 1).
    2. expiry      - the validator is deliberately clockless, so wall-clock
                     expiry is checked here against -Now.
    3. stage       - decrypt + untar into an empty PGDATA with a recovery.signal
                     targeting an explicit LSN.
    4. recover     - start the target; PostgreSQL replays the WAL archive.
    5. assert      - runbook checks P2-P4/P6 and A5-A7 inside the cluster.
    6. file hashes - every declared file class re-hashed (A3/P5).
    7. verdict     - one machine-readable JSON; SUCCESS only if all pass.

  The restore target is ALWAYS torn down, on success and on failure. A drill
  that left a cluster running would be a production activation, which the plan
  forbids outright.

.OUTPUTS
  Exit 0 RESTORE VERIFIED
  Exit 1 restore or an assertion failed (verdict JSON still written)
  Exit 2 usage / environment error
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SetPath,
    [Parameter(Mandatory = $true)][string]$Sidecar,
    [ValidateSet('default', 'premember', 'prelive')][string]$Gate = 'default',
    [string]$Key = 'lagrange-drill-key',
    # Preferred for scheduled runs: a passphrase passed as an argument is
    # visible to every user in the process list.
    [string]$KeyFile,
    [string]$Now,
    [string]$Verdict,
    [string]$Metrics
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$composeFile = Join-Path $root 'deploy/backup/compose/drill.compose.yml'
$prepare = Join-Path $root 'scripts/backup/lib/restore-prepare-inside.sh'
$verifyScript = Join-Path $root 'scripts/backup/lib/restore-verify-inside.sh'

if (-not (Test-Path -PathType Container $SetPath)) { Write-Error "ENV ERROR: backup set not found: $SetPath"; exit 2 }
if (-not (Test-Path -PathType Leaf $Sidecar)) { Write-Error "ENV ERROR: sidecar not found: $Sidecar"; exit 2 }
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { Write-Error 'ENV ERROR: docker not found on PATH'; exit 2 }
if ($KeyFile) {
    if (-not (Test-Path -PathType Leaf $KeyFile)) { Write-Error "ENV ERROR: key file not found: $KeyFile"; exit 2 }
    $Key = (Get-Content -Raw $KeyFile).Trim()
    if (-not $Key) { Write-Error "ENV ERROR: key file is empty: $KeyFile"; exit 2 }
}
if (-not $Now) { $Now = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }

$side = Get-Content -Raw $Sidecar | ConvertFrom-Json
$manifest = Get-Content -Raw (Join-Path $SetPath 'backup-manifest.json') | ConvertFrom-Json

$startedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$status = 'FAILED'
$failedAssertion = $null
$facts = [ordered]@{}
$project = $null

# Deliberately a SIMPLE function using the automatic $args, not an advanced
# function with [Parameter(ValueFromRemainingArguments)]. An advanced function
# treats a leading-dash token as a parameter name and SILENTLY DROPS it when it
# matches nothing: `Invoke-Compose run -d ...` arrived as `run ...`, so the
# staging container ran in the foreground and blocked on its own sleep.
function Invoke-Compose { & docker compose -p $project -f $composeFile @args }

function Set-Failure {
    param([string]$Assertion, [string]$Detail)
    $script:failedAssertion = $Assertion
    Write-Host "RESTORE FAILED [$Assertion]: $Detail"
}

function Write-Verdict {
    $duration = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() - $startedAt
    $body = [ordered]@{
        verdict            = $script:status
        gate               = $Gate
        backup_set_id      = $side.backup_set_id
        evaluated_at       = $Now
        duration_seconds   = $duration
        failed_assertion   = $script:failedAssertion
        isolated_target    = $true
        target_left_running = $false
        facts              = $script:facts
    } | ConvertTo-Json -Depth 6
    Write-Host $body
    if ($Verdict) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Verdict) | Out-Null
        $body | Set-Content -Path $Verdict -Encoding utf8
    }
    if ($Metrics) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Metrics) | Out-Null
        $nowEpoch = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        $successEpoch = if ($script:status -eq 'SUCCESS') { $nowEpoch } else { 0 }
        $verified = if ($script:status -eq 'SUCCESS') { 1 } else { 0 }
        @"
# HELP lagrange_restore_last_run_timestamp_seconds Unix time of the last restore drill.
# TYPE lagrange_restore_last_run_timestamp_seconds gauge
lagrange_restore_last_run_timestamp_seconds $nowEpoch
# HELP lagrange_restore_last_success_timestamp_seconds Unix time of the last VERIFIED restore drill.
# TYPE lagrange_restore_last_success_timestamp_seconds gauge
lagrange_restore_last_success_timestamp_seconds $successEpoch
# HELP lagrange_restore_duration_seconds Wall-clock duration of the last restore drill.
# TYPE lagrange_restore_duration_seconds gauge
lagrange_restore_duration_seconds $duration
# HELP lagrange_restore_verified Whether the last restore drill verified (1) or failed (0).
# TYPE lagrange_restore_verified gauge
lagrange_restore_verified $verified
"@ | Set-Content -Path $Metrics -Encoding utf8
    }
}

try {
    # --- 1. policy gate ------------------------------------------------------
    Write-Host '== policy gate (no restore command runs unless this passes) =='
    $validator = Join-Path $root 'scripts/backup/validate-policy.ps1'
    $gateOut = & $validator -SetPath $SetPath -Gate $Gate 2>&1 | Out-String
    $gateRc = $LASTEXITCODE
    Write-Host $gateOut
    $facts['policy_gate_exit'] = $gateRc
    if ($gateRc -ne 0) {
        Set-Failure 'P1' "policy gate rejected the set (exit $gateRc); no restore was attempted"
        exit 1
    }

    # --- 2. expiry ------------------------------------------------------------
    # The validator enforces retention FLOORS but is deliberately clockless so
    # its transcript is reproducible. Wall-clock expiry belongs here, where -Now
    # makes the check deterministic for tests.
    Write-Host "== retention expiry (as of $Now) =="
    # 'T' and 'Z' must be escaped: unescaped they are parsed as format
    # specifiers, not literals, and ParseExact throws.
    $utcFmt = 'yyyy-MM-dd\THH:mm:ss\Z'
    $nowDt = [datetime]::ParseExact($Now, $utcFmt, [Globalization.CultureInfo]::InvariantCulture)
    $expired = @()
    foreach ($c in $manifest.classes) {
        if ($c.expires_at) {
            # ConvertFrom-Json coerces ISO-8601 strings into [datetime] on its
            # own, so expires_at may already BE a DateTime. Re-parsing its
            # localised string form ("08/22/2026 00:00:00") throws.
            $exp = if ($c.expires_at -is [datetime]) {
                $c.expires_at
            } else {
                [datetime]::ParseExact([string]$c.expires_at, $utcFmt, [Globalization.CultureInfo]::InvariantCulture)
            }
            if ($exp -lt $nowDt) { $expired += "$($c.class) expired at $($c.expires_at)" }
        }
    }
    if ($expired.Count -gt 0) {
        $facts['expired_classes'] = ($expired -join '; ')
        Set-Failure 'RETENTION' ($expired -join '; ')
        exit 1
    }
    $facts['expired_classes'] = $null

    $targetLsn = $side.recovery_target_lsn
    $facts['recovery_target_lsn'] = $targetLsn

    # --- 3/4. stage and recover into a disposable project ---------------------
    $project = "lagrange-restore-$((Get-Date).ToUniversalTime().ToString('yyyyMMddHHmmss'))-$PID".ToLowerInvariant()
    Write-Host "== isolated restore project: $project =="
    $facts['restore_project'] = $project

    Invoke-Compose up -d --wait --no-deps init-perms 2>&1 | Out-Null
    # `compose run -d` writes progress lines to stderr and the container id to
    # stdout, but PowerShell merges native streams unpredictably. Pick the line
    # that IS a container id rather than trusting its position.
    $runOut = Invoke-Compose run -d --no-deps --entrypoint sleep target 600 2>&1
    $staging = ($runOut | ForEach-Object { "$_".Trim() } |
                Where-Object { $_ -match '^[0-9a-f]{64}$' } | Select-Object -Last 1)
    if (-not $staging) {
        Set-Failure 'STAGE' 'could not start a staging container for the restore target'
        exit 1
    }

    & docker cp $SetPath "${staging}:/backup/set" 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        & docker rm -f $staging 2>&1 | Out-Null
        Set-Failure 'STAGE' 'could not copy the backup set into the restore target'
        exit 1
    }

    Write-Host '== staging the point-in-time recovery =='
    # Copied in and exec'd by path, not piped to `bash -s`: PowerShell does not
    # reliably surface a native command's exit code at the end of a pipeline,
    # and this step's exit code is what distinguishes "wrong key" from "staged".
    & docker cp $prepare "${staging}:/tmp/restore-prepare-inside.sh" 2>&1 | Out-Null
    $stageOut = & docker exec -i -e BACKUP_KEY=$Key -e TARGET_LSN=$targetLsn -e SET=/backup/set `
        $staging bash /tmp/restore-prepare-inside.sh 2>&1 | Out-String
    $stageRc = $LASTEXITCODE
    Write-Host $stageOut
    & docker rm -f $staging 2>&1 | Out-Null
    if ($stageRc -ne 0) {
        # A wrong key or a torn archive lands here, before PostgreSQL starts.
        if ($stageOut -match 'could not decrypt') {
            Set-Failure 'DECRYPT' 'the db archives could not be decrypted (wrong key or corrupt)'
        } else {
            Set-Failure 'STAGE' 'staging the recovery failed'
        }
        exit 1
    }

    Write-Host "== recovering (replaying WAL to $targetLsn) =="
    Invoke-Compose up -d --wait target 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Invoke-Compose logs --tail 40 target 2>&1 | Write-Host
        Set-Failure 'P2' 'the restored cluster never finished recovery'
        exit 1
    }

    # --- 5. runbook assertions inside the recovered cluster -------------------
    Write-Host '== runbook assertions =='
    $tcid = (Invoke-Compose ps -q target | Select-Object -First 1)
    & docker cp $verifyScript "${tcid}:/tmp/restore-verify-inside.sh" 2>&1 | Out-Null
    $assertOut = & docker compose -p $project -f $composeFile exec -T `
        -e TARGET_LSN=$targetLsn `
        -e EXPECT_ROWS_AT_TARGET=$($side.pre_target_row_count) `
        -e EXPECT_PROVENANCE=$($side.provenance_row_count) `
        -e EXPECT_DATASET=$($side.dataset_version) `
        -e EXPECT_MIN_LSN=$($side.pre_target_lsn) `
        target bash /tmp/restore-verify-inside.sh 2>&1 | Out-String
    $assertRc = $LASTEXITCODE
    Write-Host $assertOut

    function Get-Fact { param([string]$K)
        ($assertOut -split "`n" | Where-Object { $_ -match "^$K=" } | Select-Object -Last 1) -replace "^$K=", '' -replace "`r", ''
    }
    $facts['rows_at_target'] = Get-Fact 'ROWS_TOTAL'
    $facts['rows_after_target'] = Get-Fact 'ROWS_POST_TARGET'
    $facts['provenance_rows'] = Get-Fact 'PROVENANCE_ROWS'
    $facts['dataset_version'] = Get-Fact 'DATASET_VERSION'
    $facts['secret_marker_hits'] = Get-Fact 'SECRET_MARKER_HITS'

    if ($assertRc -ne 0) {
        $firstFail = ($assertOut -split "`n" | Where-Object { $_ -match '^ASSERT_FAIL' } | Select-Object -First 1)
        $id = if ($firstFail -match '^ASSERT_FAIL (\S+)') { $Matches[1] } else { 'ASSERT' }
        Set-Failure $id 'a runbook assertion failed'
        exit 1
    }

    # --- 6. file-class hashes (A3 / P5) ---------------------------------------
    Write-Host '== file-class hash comparison =='
    $bad = @()
    $checked = 0
    foreach ($c in $manifest.classes) {
        if ($c.kind -ne 'file') { continue }
        foreach ($f in $c.files) {
            $checked++
            $p = Join-Path $SetPath $f.path
            if (-not (Test-Path -PathType Leaf $p)) { $bad += "$($f.path) missing"; continue }
            $h = (Get-FileHash -Algorithm SHA256 -Path $p).Hash.ToLowerInvariant()
            if ($h -ne $f.sha256) { $bad += "$($f.path) declared $($f.sha256) computed $h" }
        }
    }
    $facts['file_classes_checked'] = $checked
    if ($bad.Count -gt 0) {
        $facts['file_hash_mismatches'] = ($bad -join '; ')
        Set-Failure 'A3' ($bad -join '; ')
        exit 1
    }
    $facts['file_hash_mismatches'] = $null

    $status = 'SUCCESS'
    Write-Host "RESTORE VERIFIED: $($side.backup_set_id) at LSN $targetLsn"
}
catch {
    # An unexpected exception must never read as a quiet failure with no named
    # assertion - that is indistinguishable from a clean refusal.
    Set-Failure 'EXCEPTION' $_.Exception.Message
}
finally {
    if ($project) { Invoke-Compose down -v --remove-orphans 2>&1 | Out-Null }
    Write-Verdict
}

exit 0
