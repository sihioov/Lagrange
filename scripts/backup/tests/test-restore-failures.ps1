<#
.SYNOPSIS
  Prove restore-and-verify.ps1 fails CLOSED (plan Todo 33).
  PowerShell twin of scripts/backup/tests/test-restore-failures.sh.

.DESCRIPTION
  The happy path is proven by running create then restore-and-verify; this
  harness proves the far more important half: that a set which should not be
  restorable is not restored, and that nothing is left running afterwards.

  Scenarios (plan Todo 33 QA "failure" list):
    1. wrong key            -> DECRYPT, and PostgreSQL is never started
    2. missing WAL segment  -> policy gate rejects; no restore command runs
    3. corrupt WAL content  -> hash mismatch at the gate; no restore command runs
    4. secret in an archive -> gate rejects, naming the marker
    5. expired manifest     -> RETENTION, evaluated against -Now
    6. partial DB (no base) -> gate rejects the missing db_base class

  Every scenario additionally asserts that the verdict JSON says FAILED, names
  the failing assertion, and that no drill container survived.

  A good backup set is required. Build one first with scripts/backup/create.ps1.

.OUTPUTS
  Exit 0 only when every scenario fails in exactly the expected way.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SetPath,
    [Parameter(Mandatory = $true)][string]$Sidecar,
    [string]$Key = 'lagrange-drill-key',
    [string]$Work
)

$ErrorActionPreference = 'Continue'
# This file lives at scripts/backup/tests/, so the repo root is THREE levels up
# (the sibling scripts under scripts/backup/ only need two).
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$restore = Join-Path $root 'scripts/backup/restore-and-verify.ps1'
if (-not (Test-Path -PathType Leaf $restore)) {
    Write-Error "ENV ERROR: restore-and-verify.ps1 not found at $restore"; exit 2
}

if (-not (Test-Path -PathType Container $SetPath)) { Write-Error 'USAGE: -SetPath must be an existing backup set directory'; exit 2 }
if (-not (Test-Path -PathType Leaf $Sidecar)) { Write-Error 'USAGE: -Sidecar must be an existing sidecar file'; exit 2 }
if (-not $Work) { $Work = Join-Path $env:TEMP "lagrange-restore-failtests-$PID" }
New-Item -ItemType Directory -Force -Path $Work | Out-Null

$script:tests = 0
$script:fails = 0

# Scoped to THIS scenario's own project, not every lagrange-restore-* container:
# an unrelated drill running concurrently is not evidence that this one leaked.
function Get-RunningDrillCount {
    param([string]$Project)
    if (-not $Project) { return 0 }
    $names = & docker ps --format '{{.Names}}' 2>$null
    if (-not $names) { return 0 }
    @($names | Where-Object { $_ -like "$Project-*" }).Count
}

function Invoke-Scenario {
    param(
        [string]$Name,
        [string]$Dir,
        [string]$Expect,
        [hashtable]$Extra = @{}
    )
    $script:tests++
    $vfile = Join-Path $Work "verdict-$($script:tests).json"
    # NOT $args: that is a PowerShell automatic variable and assigning to it
    # inside a function silently breaks the splat.
    $splat = @{ SetPath = $Dir; Sidecar = $Sidecar; Key = $Key; Verdict = $vfile }
    # Assigned, not `+`: adding two hashtables that share a key throws
    # ("Item has already been added"), and the wrong-key scenario overrides Key.
    foreach ($k in $Extra.Keys) { $splat[$k] = $Extra[$k] }
    $out = & $restore @splat 2>&1 | Out-String
    $rc = $LASTEXITCODE

    $ok = $true
    $why = @()
    $proj = $null
    if ($rc -eq 0) { $ok = $false; $why += 'exit=0(expected nonzero)' }
    if (Test-Path $vfile) {
        $v = Get-Content -Raw $vfile | ConvertFrom-Json
        $proj = $v.facts.restore_project
        if ($v.verdict -ne 'FAILED') { $ok = $false; $why += "verdict=$($v.verdict)" }
        if ($v.failed_assertion -ne $Expect) { $ok = $false; $why += "failed_assertion=$($v.failed_assertion)" }
    } else {
        $ok = $false; $why += 'no-verdict-file'
    }
    $left = Get-RunningDrillCount -Project $proj
    if ($left -ne 0) { $ok = $false; $why += "left $left container(s) of $proj running" }

    if ($ok) {
        Write-Host "PASS $Name (failed_assertion=$Expect)"
    } else {
        $script:fails++
        Write-Host "FAIL $Name - $($why -join ' ')"
        ($out -split "`n" | Select-Object -Last 15) | ForEach-Object { Write-Host "    $_" }
    }
}

function Copy-Set {
    param([string]$Name)
    $dst = Join-Path $Work $Name
    if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
    New-Item -ItemType Directory -Force -Path $dst | Out-Null
    Copy-Item -Recurse -Force (Join-Path $SetPath '*') $dst
    $dst
}

Write-Host '== building mutated backup sets =='

$setMissingWal = Copy-Set 'missing-wal'
$firstWal = Get-ChildItem -Recurse -File -Filter '*.enc' (Join-Path $setMissingWal 'pg/wal') | Sort-Object Name | Select-Object -First 1
Remove-Item -Force $firstWal.FullName

$setCorruptWal = Copy-Set 'corrupt-wal'
$corrupt = Get-ChildItem -Recurse -File -Filter '*.enc' (Join-Path $setCorruptWal 'pg/wal') | Sort-Object Name | Select-Object -First 1
Add-Content -Path $corrupt.FullName -Value 'corrupted-by-the-failure-harness' -NoNewline

$setSecret = Copy-Set 'secret-in-archive'
$secretFile = Get-ChildItem -Recurse -File -Filter '*.increment' (Join-Path $setSecret 'files/artifact') | Select-Object -First 1
Add-Content -Path $secretFile.FullName -Value 'LAGRANGE_SECRET_MARKER=leaked'
# Re-hash so the set fails ONLY on the secret marker, not on a hash mismatch -
# otherwise this scenario would pass for the wrong reason.
$mpath = Join-Path $setSecret 'backup-manifest.json'
$m = Get-Content -Raw $mpath | ConvertFrom-Json
$rel = (Resolve-Path -Relative -Path $secretFile.FullName -RelativeBasePath $setSecret) -replace '^\.[\\/]', '' -replace '\\', '/'
$newHash = (Get-FileHash -Algorithm SHA256 -Path $secretFile.FullName).Hash.ToLowerInvariant()
foreach ($c in $m.classes) {
    foreach ($f in $c.files) {
        if ($f.path -eq $rel) {
            $f.sha256 = $newHash
            $f.size_bytes = (Get-Item $secretFile.FullName).Length
        }
    }
}
$m | ConvertTo-Json -Depth 10 | Set-Content -Path $mpath -Encoding utf8

$setNoBase = Copy-Set 'partial-db'
$mpath2 = Join-Path $setNoBase 'backup-manifest.json'
$m2 = Get-Content -Raw $mpath2 | ConvertFrom-Json
$m2.classes = @($m2.classes | Where-Object { $_.class -ne 'db_base' })
$m2 | ConvertTo-Json -Depth 10 | Set-Content -Path $mpath2 -Encoding utf8

Write-Host '== running failure scenarios =='

# 1. Wrong key. The set is perfectly valid, so the policy gate PASSES and the
#    restore genuinely begins - the one scenario that must be caught by
#    decryption rather than by the gate.
Invoke-Scenario 'wrong decryption key aborts before PostgreSQL starts' $SetPath 'DECRYPT' @{ Key = 'definitely-the-wrong-key' }

# 2-4, 6. Gate rejections: no restore command may run at all.
Invoke-Scenario 'missing WAL segment is rejected at the gate' $setMissingWal 'P1'
Invoke-Scenario 'corrupt WAL content is rejected at the gate' $setCorruptWal 'P1'
Invoke-Scenario 'secret marker in an archive is rejected at the gate' $setSecret 'P1'
Invoke-Scenario 'partial DB (no base backup) is rejected at the gate' $setNoBase 'P1'

# 5. Expired manifest: the validator is clockless by design, so expiry is the
#    restore driver's job and -Now makes the check deterministic.
Invoke-Scenario 'an expired backup set is refused' $SetPath 'RETENTION' @{ Now = '2099-01-01T00:00:00Z' }

Write-Host ''
if ($script:fails -eq 0) {
    Write-Host "ALL RESTORE FAILURE TESTS PASSED ($($script:tests)/$($script:tests))"
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
    exit 0
}
Write-Host "RESTORE FAILURE TESTS FAILED ($($script:tests - $script:fails)/$($script:tests)) - artifacts in $Work"
exit 1
