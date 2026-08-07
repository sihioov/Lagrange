<#
.SYNOPSIS
  Phase 2 operational failure injection and recovery drills (plan Todo 34).
  PowerShell twin of scripts/qa/failure-suite.sh.

.DESCRIPTION
  Covers design §17 "Failure Injection" and the §16 fail-closed table, limited
  to what Phase 2 owns (the KIS/WebSocket rows are Phase 3, Todos 36-40):

    F1  DB 일시 장애                api-server  failure_db_outage_*
    F2  워커 강제 종료 / OOM         job-queue   worker_death / zombie / retry_exhaustion
    F3  데이터·아티팩트 손상         api-server  artifact_hash_mismatch_fails_closed
                                    result-model publication_is_refused_*
    F4  Paper 스케줄러 중단          portfolio-model paper_flow crash-resume
    F5  중복 / 순서역전 이벤트       portfolio-model ledger replay + job-queue idempotency
    F6  디스크 풀                    api-server  failure_disk_full_artifact_is_never_served
    F7  알림 중단                    api-server  observability_notification_email_outage_*
    F8  복원 실패                    scripts/backup/tests/test-restore-failures.ps1

  Most scenarios are proven by tests that already exist; this suite RUNS them
  as named fault scenarios rather than duplicating them. A second copy of an
  invariant is a second thing to drift, not a second proof.

  -SelfTest is not decoration. The plan requires that "each deliberately broken
  invariant makes the suite nonzero" - a non-vacuousness requirement on the
  suite itself. A suite that cannot fail is not evidence.

.OUTPUTS
  Exit 0 all scenarios passed (or passed with skips)
  Exit 1 a scenario failed, or a sabotage went undetected
  Exit 2 the suite could not run
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('2')][string]$Phase,
    [switch]$SelfTest,
    [switch]$KeepDb
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$qaCompose = Join-Path $root 'deploy/qa/qa-db.compose.yml'
$evidenceDir = Join-Path $root '.omo/evidence/task-34-transcripts'

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { Write-Error 'ENV ERROR: docker not found on PATH'; exit 2 }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'ENV ERROR: cargo not found on PATH'; exit 2 }
New-Item -ItemType Directory -Force -Path $evidenceDir | Out-Null

$qaPort = if ($env:LAGRANGE_QA_DB_PORT) { $env:LAGRANGE_QA_DB_PORT } else { '55432' }
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = (Join-Path $root 'target') }
$env:DATABASE_URL = "postgres://postgres:lagrange@127.0.0.1:$qaPort/postgres"

# Simple function using the automatic $args: an advanced function silently
# drops leading-dash tokens it cannot bind (see decisions.md 2026-08-08).
function Invoke-QaCompose { & docker compose -p lagrange-qa -f $qaCompose @args }

$script:scenarios = 0
$script:failed = 0
$script:skipped = 0

function Add-Result {
    param([string]$Id, [string]$Name, [string]$Result, [string]$Detail)
    '{0,-4} {1,-34} = {2,-6} {3}' -f "SCENARIO $Id", $Name, $Result, $Detail | Write-Host
    if ($Result -eq 'FAIL') { $script:failed++ }
    if ($Result -eq 'SKIP') { $script:skipped++ }
    $script:scenarios++
}

function Invoke-Scenario {
    # PASS only when cargo exits 0 AND at least one test actually ran. A filter
    # matching nothing exits 0 with "0 passed", which would otherwise be
    # recorded as a passing scenario - the silently-empty-run trap.
    param([string]$Id, [string]$Name, [string]$Transcript, [string[]]$CargoArgs)
    $t = Join-Path $evidenceDir $Transcript
    Push-Location $root
    try {
        & cargo test @CargoArgs -- --test-threads=2 2>&1 | Out-File -FilePath $t -Encoding utf8
        $rc = $LASTEXITCODE
    } finally { Pop-Location }
    $ran = 0
    foreach ($line in (Get-Content $t)) {
        if ($line -match '^test result: ok\. (\d+) passed') { $ran += [int]$Matches[1] }
    }
    if ($rc -ne 0) {
        Add-Result $Id $Name 'FAIL' "cargo exit $rc (see $Transcript)"
    } elseif ($ran -eq 0) {
        Add-Result $Id $Name 'FAIL' "the filter selected 0 tests (see $Transcript)"
    } else {
        Add-Result $Id $Name 'PASS' "$ran assertion(s) ran"
    }
    return $ran
}

Write-Host '== Phase 2 failure suite =='
Write-Host "   QA database: 127.0.0.1:$qaPort (disposable)"

try {
    Invoke-QaCompose up -d --wait qa-db 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Error 'ENV ERROR: the QA database did not become healthy'; exit 2 }

    Invoke-Scenario F1  'DB transient outage'              'f1-db-outage.txt'            @('-p','api-server','--test','failure_injection','failure_db_outage') | Out-Null
    # One filter per scenario: cargo accepts exactly ONE filter positional, and
    # a multi-filter invocation exits nonzero. Splitting also names each
    # invariant separately, which reads better in the transcript.
    Invoke-Scenario F2  'worker death requeues once'       'f2-worker-death.txt'         @('-p','job-queue','--test','queue_contract','worker_death') | Out-Null
    Invoke-Scenario F2b 'zombie cannot settle after sweep' 'f2b-zombie-settle.txt'       @('-p','job-queue','--test','queue_contract','zombie_worker') | Out-Null
    Invoke-Scenario F2c 'retry is bounded by max_attempts' 'f2c-retry-bound.txt'         @('-p','job-queue','--test','queue_contract','retry_exhaustion') | Out-Null
    Invoke-Scenario F2d 'integrity errors never retry'     'f2d-no-retry.txt'            @('-p','job-queue','--test','queue_contract','input_data_integrity') | Out-Null
    Invoke-Scenario F3  'corrupt artifact fails closed'    'f3-artifact-corruption.txt'  @('-p','api-server','--test','artifact_authorization','artifact_hash_mismatch') | Out-Null
    Invoke-Scenario F3b 'corrupt result is never published' 'f3b-result-integrity.txt'   @('-p','result-model','--test','backtest_result_integrity','publication_is_refused') | Out-Null
    Invoke-Scenario F4  'Paper scheduler interruption'     'f4-paper-crash-resume.txt'   @('-p','portfolio-model','--test','paper_flow','crash') | Out-Null
    Invoke-Scenario F4b 'settled target cannot be reclaimed' 'f4b-paper-scheduler.txt'   @('-p','api-server','--test','paper_scheduler','claimed_twice') | Out-Null
    Invoke-Scenario F5  'duplicate / out-of-order events'  'f5-replay.txt'               @('-p','portfolio-model','--test','replay') | Out-Null
    Invoke-Scenario F5b 'duplicate submission is idempotent' 'f5b-idempotency.txt'       @('-p','job-queue','--test','queue_contract','duplicate_idempotency') | Out-Null
    Invoke-Scenario F6  'disk-full artifact never served'  'f6-disk-full.txt'            @('-p','api-server','--test','failure_injection','failure_disk_full') | Out-Null
    Invoke-Scenario F7  'notification outage recorded'     'f7-notification-outage.txt'  @('-p','api-server','--test','notifications','email_outage') | Out-Null

    # F8 reuses Todo 33's harness. It needs a real backup set, so it is
    # skipped-with-a-reason when one was not supplied, never counted as a pass.
    if ($env:LAGRANGE_QA_BACKUP_SET -and $env:LAGRANGE_QA_BACKUP_SIDECAR) {
        $t = Join-Path $evidenceDir 'f8-restore-failures.txt'
        $key = if ($env:LAGRANGE_QA_BACKUP_KEY) { $env:LAGRANGE_QA_BACKUP_KEY } else { 'lagrange-drill-key' }
        & (Join-Path $root 'scripts/backup/tests/test-restore-failures.ps1') `
            -SetPath $env:LAGRANGE_QA_BACKUP_SET -Sidecar $env:LAGRANGE_QA_BACKUP_SIDECAR -Key $key `
            2>&1 | Out-File -FilePath $t -Encoding utf8
        if ($LASTEXITCODE -eq 0) {
            Add-Result F8 'restore failure drills' 'PASS' '6/6 restore faults fail closed'
        } else {
            Add-Result F8 'restore failure drills' 'FAIL' 'see f8-restore-failures.txt'
        }
    } else {
        Add-Result F8 'restore failure drills' 'SKIP' 'set LAGRANGE_QA_BACKUP_SET and _SIDECAR'
    }

    Invoke-Scenario A1 'refusal audited with correlation' 'a1-audit-correlation.txt' @('-p','api-server','--test','failure_injection','failure_refusal_is_audited') | Out-Null

    if ($SelfTest) {
        Write-Host ''
        Write-Host '== self-test: each sabotaged invariant must make the suite nonzero =='
        $stFail = 0; $stRun = 0

        # Sabotage 1: an empty filter must not be recorded as a pass.
        $stRun++
        $before = $script:failed
        Invoke-Scenario SELF1 'empty filter sabotage' 'self-1-empty-filter.txt' `
            @('-p','api-server','--test','failure_injection','this_test_name_does_not_exist') | Out-Null
        if ($script:failed -gt $before) {
            Write-Host 'SELFTEST 1 PASS  an empty filter is detected'
            $script:failed--; $script:scenarios--
        } else { Write-Host 'SELFTEST 1 FAIL  an empty filter was not detected'; $stFail++ }

        # Sabotage 2: a scenario whose cargo invocation fails must be FAIL.
        $stRun++
        $before = $script:failed
        Invoke-Scenario SELF2 'broken scenario sabotage' 'self-2-broken.txt' `
            @('-p','api-server','--test','no_such_test_binary_exists') | Out-Null
        if ($script:failed -gt $before) {
            Write-Host 'SELFTEST 2 PASS  a broken scenario is recorded FAIL'
            $script:failed--; $script:scenarios--
        } else { Write-Host 'SELFTEST 2 FAIL  a broken scenario was not recorded'; $stFail++ }

        Write-Host "SELFTEST: $($stRun - $stFail)/$stRun sabotages detected"
        if ($stFail -ne 0) {
            Write-Host ''
            Write-Host "VERDICT: SUITE_NOT_TRUSTWORTHY ($stFail sabotage(s) undetected)"
            exit 1
        }
    }
}
finally {
    if (-not $KeepDb) { Invoke-QaCompose down -v --remove-orphans 2>&1 | Out-Null }
}

Write-Host ''
$passed = $script:scenarios - $script:failed - $script:skipped
"SCENARIOS: {0} passed, {1} failed, {2} skipped (of {3})" -f $passed, $script:failed, $script:skipped, $script:scenarios | Write-Host

if ($script:failed -ne 0) { Write-Host 'VERDICT: PHASE2_FAULTS_FAILED'; exit 1 }
if ($script:skipped -ne 0) {
    # A skip is not a pass: a partial run must never be quotable as full
    # Phase 2 fault coverage.
    Write-Host 'VERDICT: PHASE2_FAULTS_INCOMPLETE'; exit 0
}
Write-Host 'VERDICT: PHASE2_FAULTS_PASSED'
exit 0
