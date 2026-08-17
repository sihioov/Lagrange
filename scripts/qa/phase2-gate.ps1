<#
.SYNOPSIS
  Phase 2 Paper and recovery release gate (plan Todo 35).
  PowerShell twin of scripts/qa/phase2-gate.sh.

.DESCRIPTION
  Assembles the Phase 2 evidence bundle and emits ONE machine-readable verdict:
  APPROVED or OWNER_ONLY_BLOCKED_EXTERNAL.

  APPROVED requires EVERY Phase 2 check to pass AND an active data entitlement
  AND five-user Phase 1 evidence. Phase 2 can be proven Owner-only while Phase 1
  is externally blocked (the KIS broker entitlement and the Auth0 tenant),
  but that state is NOT a release: Member production stays disabled and the gate
  says so in the verdict rather than in a footnote. There is deliberately no
  flag, override, or environment variable that turns a blocked run into an
  APPROVED one - a gate with an escape hatch is not a gate.

  Checks: P1 backtest-vs-Paper parity, P2 AT-07 isolation, P3 ordering/cost/
  ledger reconciliation, P4 scheduler restart idempotency, P5 notification
  delivery outcomes, P6 PITR + file restore, P7 Phase 2 fault suite,
  E1 data entitlement (external), E2 five-user Phase 1 evidence (external).

.OUTPUTS
  Exit 0 a verdict was emitted (OWNER_ONLY_BLOCKED_EXTERNAL is a legitimate
         outcome, not an error); exit 2 the gate could not run.
#>
[CmdletBinding()]
param([switch]$KeepDb)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$evidenceDir = Join-Path $root '.omo/evidence'
$transcriptDir = Join-Path $evidenceDir 'task-35-transcripts'
$evPath = Join-Path $evidenceDir 'task-35-lagrange-station-implementation.json'
$qaCompose = Join-Path $root 'deploy/qa/qa-db.compose.yml'
$dataRightsDir = Join-Path $root 'configs/data-rights'

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { Write-Error 'ENV ERROR: docker not found on PATH'; exit 2 }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'ENV ERROR: cargo not found on PATH'; exit 2 }
New-Item -ItemType Directory -Force -Path $evidenceDir, $transcriptDir | Out-Null

$qaPort = if ($env:LAGRANGE_QA_DB_PORT) { $env:LAGRANGE_QA_DB_PORT } else { '55432' }
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = (Join-Path $root 'target') }
$env:DATABASE_URL = "postgres://postgres:lagrange@127.0.0.1:$qaPort/postgres"

# Simple function using the automatic $args: an advanced function silently drops
# leading-dash tokens it cannot bind (decisions.md 2026-08-08).
function Invoke-QaCompose { & docker compose -p lagrange-qa -f $qaCompose @args }

$script:checks = @()
function Add-Check {
    param([string]$Id, [string]$Name, [string]$Result, [string]$Detail)
    $script:checks += [pscustomobject]@{ id = $Id; name = $Name; result = $Result; detail = $Detail }
    '{0,-9} {1,-26} = {2,-18} {3}' -f "CHECK $Id", $Name, $Result, $Detail | Write-Host
}

function Invoke-Check {
    # PASS only when cargo exits 0 AND at least one test ran. A filter that
    # selects nothing exits 0 with "0 passed"; recording that as evidence would
    # let the gate approve on the strength of tests that never executed.
    param([string]$Id, [string]$Name, [string]$File, [string[]]$CargoArgs)
    $t = Join-Path $transcriptDir $File
    Push-Location $root
    try {
        & cargo test @CargoArgs -- --test-threads=2 2>&1 | Out-File -FilePath $t -Encoding utf8
        $rc = $LASTEXITCODE
    } finally { Pop-Location }
    $ran = 0
    foreach ($line in (Get-Content $t)) {
        if ($line -match '^test result: ok\. (\d+) passed') { $ran += [int]$Matches[1] }
    }
    if ($rc -ne 0) { Add-Check $Id $Name 'FAIL' "cargo exit $rc ($File)" }
    elseif ($ran -eq 0) { Add-Check $Id $Name 'FAIL' "0 tests selected ($File)" }
    else { Add-Check $Id $Name 'PASS' "$ran assertion(s)" }
}

Write-Host '== Phase 2 release gate =='
try {
    Invoke-QaCompose up -d --wait qa-db 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Error 'ENV ERROR: the QA database did not become healthy'; exit 2 }

    Invoke-Check P1   'paper-parity'          'p1-paper-parity.txt'    @('-p','result-model','paper_parity')
    Invoke-Check P1b  'parity-route'          'p1b-parity-route.txt'   @('-p','api-server','--test','http_paper')
    Invoke-Check P2   'at07-isolation'        'p2-isolation.txt'       @('-p','api-server','--test','paper_notifications','two_members')
    Invoke-Check P2b  'tenancy-rls'           'p2b-tenancy.txt'        @('-p','api-server','--test','tenancy_rls')
    Invoke-Check P3   'sell-before-buy'       'p3-sell-before-buy.txt' @('-p','portfolio-model','--test','paper_flow','sells_before_buys')
    Invoke-Check P3b  'ledger-reconciliation' 'p3b-ledger.txt'         @('-p','portfolio-model','--test','ledger')
    Invoke-Check P4   'restart-idempotency'   'p4-restart.txt'         @('-p','portfolio-model','--test','paper_flow','crash')
    Invoke-Check P4b  'target-claim-once'     'p4b-claim.txt'          @('-p','api-server','--test','paper_scheduler','claimed_twice')
    Invoke-Check P5   'delivery-outcomes'     'p5-notifications.txt'   @('-p','api-server','--test','notifications')

    # P6 reads Todo 33's restore VERDICT; it does not re-derive it. A gate that
    # recomputed its own evidence could not detect that none was ever produced.
    $p6 = $env:LAGRANGE_PHASE2_RESTORE_VERDICT
    if ($p6 -and (Test-Path -PathType Leaf $p6)) {
        $v = Get-Content -Raw $p6 | ConvertFrom-Json
        if ($v.verdict -eq 'SUCCESS') {
            Copy-Item $p6 (Join-Path $transcriptDir 'p6-restore-verdict.json') -Force -ErrorAction SilentlyContinue
            Add-Check P6 'pitr-restore' 'PASS' "verified at LSN $($v.facts.recovery_target_lsn)"
        } else {
            Add-Check P6 'pitr-restore' 'FAIL' 'restore verdict is not SUCCESS'
        }
    } else {
        Add-Check P6 'pitr-restore' 'MISSING_EVIDENCE' 'set LAGRANGE_PHASE2_RESTORE_VERDICT to a restore-and-verify verdict JSON'
    }

    $p7out = Join-Path $transcriptDir 'p7-fault-suite.txt'
    & (Join-Path $root 'scripts/qa/failure-suite.ps1') -Phase 2 -KeepDb 2>&1 | Out-File -FilePath $p7out -Encoding utf8
    $p7rc = $LASTEXITCODE
    $p7pass = Select-String -Path $p7out -Pattern '^VERDICT: PHASE2_FAULTS_PASSED' -Quiet
    if ($p7rc -ne 0) { Add-Check P7 'fault-suite' 'FAIL' 'fault suite nonzero (see p7-fault-suite.txt)' }
    elseif ($p7pass) { Add-Check P7 'fault-suite' 'PASS' 'all Phase 2 faults fail closed' }
    else { Add-Check P7 'fault-suite' 'MISSING_EVIDENCE' 'fault suite incomplete (see p7-fault-suite.txt)' }

    # E1: KIS (or legacy KRX-compatible) rights metadata must be ACTIVE with a
    # real document hash. A placeholder with a zeroed hash is not entitlement.
    $e1 = 'BLOCKED_EXTERNAL'; $e1d = 'no ACTIVE KIS entitlement artifact in configs/data-rights/'
    if (Test-Path $dataRightsDir) {
        foreach ($f in Get-ChildItem -File -Filter '*.json' $dataRightsDir) {
            if ($f.Name -like '*.schema.json') { continue }
            try { $doc = Get-Content -Raw $f.FullName | ConvertFrom-Json } catch { continue }
            $provider = "$($doc.provider)"
            $lifecycle = "$($doc.lifecycle)"
            $hash = "$($doc.contract_document.document_hash.hex)"
            if (($provider -eq 'kis' -or $provider -eq 'krx') -and
                $lifecycle -eq 'ACTIVE' -and $hash -match '^[0-9a-f]{64}$' -and
                $hash -ne ('0' * 64)) {
                $e1 = 'PASS'; $e1d = "ACTIVE provider=$provider rights: $($f.Name)"; break
            }
        }
    }
    Add-Check E1 'data-entitlement' $e1 $e1d

    $e2 = 'BLOCKED_EXTERNAL'; $e2d = 'Phase 1 gate has not emitted APPROVED'
    $p1ev = Join-Path $evidenceDir 'task-28-lagrange-station-implementation.json'
    if ((Test-Path -PathType Leaf $p1ev) -and ((Get-Content -Raw $p1ev) -match '"verdict"\s*:\s*"APPROVED"')) {
        $e2 = 'PASS'; $e2d = 'Phase 1 APPROVED with five-user evidence'
    }
    Add-Check E2 'phase1-five-user' $e2 $e2d
}
finally {
    if (-not $KeepDb) { Invoke-QaCompose down -v --remove-orphans 2>&1 | Out-Null }
}

$hardFail = @($script:checks | Where-Object { $_.result -eq 'FAIL' }).Count
$missing = @($script:checks | Where-Object { $_.result -eq 'MISSING_EVIDENCE' }).Count
$blocked = @($script:checks | Where-Object { $_.result -eq 'BLOCKED_EXTERNAL' }).Count
$verdict = if (($hardFail + $missing + $blocked) -eq 0) { 'APPROVED' } else { 'OWNER_ONLY_BLOCKED_EXTERNAL' }

Write-Host ''
Write-Host "VERDICT: $verdict"
Write-Host "EVIDENCE: $evPath"
if ($verdict -ne 'APPROVED') {
    Write-Host 'NOT_APPROVED_BECAUSE:'
    foreach ($c in ($script:checks | Where-Object { $_.result -ne 'PASS' })) {
        Write-Host "  - $($c.id) $($c.name) $($c.result) $($c.detail)"
    }
    Write-Host 'MEMBER_PRODUCTION: DISABLED'
    Write-Host 'NOTE: OWNER_ONLY_BLOCKED_EXTERNAL means the Phase 2 Paper and recovery invariants may hold while the EXTERNAL Phase 1 preconditions do not. Owner-only work continues; Member KR-derived surfaces stay denied; this state must never be reported as a release.'
} else {
    Write-Host 'MEMBER_PRODUCTION: ELIGIBLE'
}

[pscustomobject]@{
    gate = 'phase2'; task = 35; verdict = $verdict
    member_production = if ($verdict -eq 'APPROVED') { 'ELIGIBLE' } else { 'DISABLED' }
    emitted_at = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd\THH:mm:ss\Z')
    checks = $script:checks
    evidence_dir = $transcriptDir
    approval_rule = 'APPROVED requires every Phase 2 check to pass AND an ACTIVE data entitlement AND five-user Phase 1 evidence. There is no override.'
} | ConvertTo-Json -Depth 6 | Set-Content -Path $evPath -Encoding utf8
Write-Host "EVIDENCE_WRITTEN: $evPath"
exit 0
