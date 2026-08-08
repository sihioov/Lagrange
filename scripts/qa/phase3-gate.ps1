# phase3-gate.ps1 - Phase 3 Live release gate (plan Todo 42).
# PowerShell twin of scripts/qa/phase3-gate.sh.
#
# Emits ONE machine-readable verdict:
#
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL_CREDENTIALS
#   VERDICT: DENIED
#
# The distinction between the last two is the point of this file. DENIED means
# a Live safety invariant does not hold: something is WRONG and must be fixed.
# BLOCKED_EXTERNAL_CREDENTIALS means every invariant we can prove does hold,
# but the evidence that can only come from a real broker account -- one bounded
# order, actually placed, actually reconciled -- does not exist. Nothing is
# broken; the proof is simply not obtainable here. Reporting the second as the
# first would send someone hunting a bug that is not there, and reporting
# either as APPROVED would put real money behind untested code.
#
# There is deliberately NO flag, environment variable, or override that turns a
# blocked or denied run into an APPROVED one. A gate with an escape hatch is
# not a gate, and this is the gate standing between this system and a live
# brokerage account.
#
# Usage: pwsh -File scripts/qa/phase3-gate.ps1 [-KeepDb]
# Twin:  scripts/qa/phase3-gate.sh

param([switch] $KeepDb)

$ErrorActionPreference = "Continue"
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$evidenceDir = Join-Path $root ".omo/evidence"
$transcriptDir = Join-Path $evidenceDir "task-42-transcripts"
$evPath = Join-Path $evidenceDir "task-42-lagrange-station-implementation.json"
$qaCompose = Join-Path $root "deploy/qa/qa-db.compose.yml"
$qaPort = if ($env:LAGRANGE_QA_DB_PORT) { $env:LAGRANGE_QA_DB_PORT } else { "55432" }

foreach ($tool in @("cargo", "uv")) {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
    Write-Error "ENV ERROR: $tool not found on PATH"; exit 2
  }
}
New-Item -ItemType Directory -Force -Path $evidenceDir, $transcriptDir | Out-Null

if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root "target" }
$env:DATABASE_URL = "postgres://postgres:lagrange@127.0.0.1:$qaPort/postgres"
$env:REPO_ROOT = $root

$script:Checks = @()

function Add-Check {
  param([string] $Id, [string] $Name, [string] $Result, [string] $Detail)
  $script:Checks += [PSCustomObject]@{ id = $Id; name = $Name; result = $Result; detail = $Detail }
  "CHECK {0,-4} {1,-26} = {2,-26} {3}" -f $Id, $Name, $Result, $Detail | Write-Host
}

# PASS only when cargo exits 0 AND at least one test ran. A filter that selects
# nothing exits 0 with "0 passed"; recording that as evidence would let this
# gate approve a LIVE release on the strength of tests that never executed.
# Todo 33 shipped exactly that mistake, and Todo 39 nearly repeated it.
function Invoke-Check {
  param([string] $Id, [string] $Name, [string] $File, [string[]] $CargoArgs)
  $t = Join-Path $transcriptDir $File
  Push-Location $root
  try { & cargo test @CargoArgs -- --test-threads=2 *>&1 | Out-File $t -Encoding utf8; $rc = $LASTEXITCODE }
  finally { Pop-Location }
  $ran = 0
  foreach ($m in (Select-String -Path $t -Pattern '^test result: ok\. (\d+) passed' -AllMatches)) {
    $ran += [int] $m.Matches[0].Groups[1].Value
  }
  if ($rc -ne 0) { Add-Check $Id $Name "FAIL" "cargo exit $rc ($File)" }
  elseif ($ran -eq 0) { Add-Check $Id $Name "FAIL" "0 tests selected ($File)" }
  else { Add-Check $Id $Name "PASS" "$ran assertion(s)" }
}

Write-Host "== Phase 3 Live release gate =="
if (Get-Command docker -ErrorAction SilentlyContinue) {
  & docker compose -p lagrange-qa -f $qaCompose up -d --wait qa-db *> $null
}

try {
  Invoke-Check "L1"   "at08-stale-data"      "l1-stale-data.txt"      @("-p","risk-gateway","at_08")
  Invoke-Check "L2"   "at09-unknown-order"   "l2-unknown.txt"         @("-p","kis-client","--test","live_order_state","unknown")
  Invoke-Check "L2b"  "one-order-per-intent" "l2b-one-order.txt"      @("-p","kis-client","--test","live_order_state","at_most_one")
  Invoke-Check "L3"   "no-member-live"       "l3-rbac.txt"            @("-p","api-server","--test","live_rbac")
  Invoke-Check "L4"   "gate-replay"          "l4-replay.txt"          @("-p","risk-gateway","reproduced_exactly_after_a_restart")
  Invoke-Check "L4b"  "gate-persistence"     "l4b-persistence.txt"    @("-p","api-server","--test","risk_store")
  Invoke-Check "L5"   "reconciliation"       "l5-reconciliation.txt"  @("-p","kis-client","reconciliation")
  Invoke-Check "L5b"  "readiness"            "l5b-readiness.txt"      @("-p","api-server","--test","reconciliation_store")
  Invoke-Check "L6"   "kill-switch"          "l6-kill-switch.txt"     @("-p","api-server","--test","live_rbac","kill_switch")
  Invoke-Check "L7"   "intent-idempotency"   "l7-idempotency.txt"     @("-p","api-server","--test","live_order_state_store")

  # --- L8 live node ---------------------------------------------------------
  $l8t = Join-Path $transcriptDir "l8-live-node.txt"
  Push-Location $root
  try { & uv run --project nt python -m pytest nt/live-node/tests -q *>&1 | Out-File $l8t -Encoding utf8; $l8rc = $LASTEXITCODE }
  finally { Pop-Location }
  $l8ran = 0
  $m = Select-String -Path $l8t -Pattern '(\d+) passed' | Select-Object -First 1
  if ($m) { $l8ran = [int] $m.Matches[0].Groups[1].Value }
  if ($l8rc -ne 0) { Add-Check "L8" "live-node" "FAIL" "pytest exit $l8rc" }
  elseif ($l8ran -eq 0) { Add-Check "L8" "live-node" "FAIL" "0 tests selected" }
  else { Add-Check "L8" "live-node" "PASS" "$l8ran assertion(s)" }

  # --- L9 executable runbooks -----------------------------------------------
  # Run, not read. A procedure nobody can verify is one that has quietly
  # stopped working, and you find that out during the incident it was written
  # for. The PowerShell twins are run here, so this gate proves the shell an
  # operator on Windows would actually reach for.
  $l9t = Join-Path $transcriptDir "l9-runbooks-ps1.txt"
  $l9fail = 0
  $l9lines = @()
  foreach ($rb in (Get-ChildItem (Join-Path $root "docs/runbooks/0*.ps1") | Sort-Object Name)) {
    $out = & pwsh -NoProfile -File $rb.FullName 2>&1
    if ($LASTEXITCODE -ne 0) { $l9fail++ }
    $l9lines += $out
  }
  $l9lines | Out-File $l9t -Encoding utf8
  $l9checks = ($l9lines | Select-String '  ok ').Count
  if ($l9fail -ne 0) { Add-Check "L9" "runbooks" "FAIL" "$l9fail runbook(s) failed" }
  elseif ($l9checks -eq 0) { Add-Check "L9" "runbooks" "FAIL" "runbooks asserted nothing" }
  else { Add-Check "L9" "runbooks" "PASS" "$l9checks assertion(s) across 7 runbooks" }

  Invoke-Check "L10"  "secret-refused"      "l10-secret-refused.txt" @("-p","api-server","--test","live_rbac","a_pasted_secret")
  Invoke-Check "L10b" "no-secret-at-rest"   "l10b-no-secret.txt"     @("-p","api-server","--test","live_rbac","holding_no_secret")
  Invoke-Check "L11"  "migration-contract"  "l11-migrations.txt"     @("-p","migration-contract")

  # --- X1/X2 external evidence ----------------------------------------------
  #
  # The FIRST version of this section could be talked into APPROVED by two
  # files containing arbitrary JSON: it checked that a path existed and that
  # the text `"reconciled": true` appeared somewhere inside. That is exactly
  # the false approval this gate exists to prevent.
  #
  # The rule now: evidence is verified against state the SYSTEM produced,
  # never against an assertion someone wrote down. The claim supplies only an
  # intent_ref -- a pointer -- and verify-live-order.py reads the rest out of
  # the database.
  #
  # X1 is not independently provable (a credential can only be tested by USING
  # it), so it is not allowed to stand alone: it reports what was supplied,
  # and X2 is what actually gates approval.
  if ($env:LAGRANGE_PHASE3_KIS_CREDENTIAL_REF -and (Test-Path $env:LAGRANGE_PHASE3_KIS_CREDENTIAL_REF)) {
    Add-Check "X1" "kis-credentials" "SUPPLIED_UNVERIFIED" "a credential reference was supplied; only X2 can prove it works"
  } else {
    Add-Check "X1" "kis-credentials" "BLOCKED_EXTERNAL_CREDENTIALS" "no real KIS account; set LAGRANGE_PHASE3_KIS_CREDENTIAL_REF"
  }

  $x2 = $env:LAGRANGE_PHASE3_LIVE_ORDER_EVIDENCE
  if (-not $x2 -or -not (Test-Path $x2)) {
    Add-Check "X2" "bounded-live-order" "BLOCKED_EXTERNAL_CREDENTIALS" "no executed low-value order evidence; requires a real account"
  } else {
    $x2t = Join-Path $transcriptDir "x2-live-order-verify.txt"
    & python3 (Join-Path $root "scripts/qa/verify-live-order.py") $x2 *>&1 | Out-File $x2t -Encoding utf8
    $x2rc = $LASTEXITCODE
    Copy-Item $x2 (Join-Path $transcriptDir "x2-live-order-claim.json") -ErrorAction SilentlyContinue
    $why = (Get-Content $x2t -TotalCount 1 -ErrorAction SilentlyContinue)
    if ($x2rc -eq 0) {
      Add-Check "X2" "bounded-live-order" "PASS" "order cross-verified against recorded state"
    } elseif ($x2rc -eq 2) {
      # "Could not check" is NOT "the evidence is not available", and like a
      # FAIL it prevents APPROVED. Approving because verification could not
      # run is the worst outcome available here.
      Add-Check "X2" "bounded-live-order" "UNVERIFIABLE_ENVIRONMENT" "could not verify: $why"
    } else {
      # A claim that exists but does NOT match recorded state is not a missing
      # proof -- it is a false one, and that is a DENIAL, not a block.
      Add-Check "X2" "bounded-live-order" "FAIL" "claim contradicts recorded state: $why"
    }
  }
} finally {
  if (-not $KeepDb -and (Get-Command docker -ErrorAction SilentlyContinue)) {
    & docker compose -p lagrange-qa -f $qaCompose down -v --remove-orphans *> $null
  }
}

# --- verdict ----------------------------------------------------------------
# Order matters. A FAIL outranks a block: if something is actually WRONG, that
# is what an operator must be told, and hiding it behind "waiting on
# credentials" would let a real defect sit unfixed until the credentials
# arrived -- at which point it would be discovered with money at stake.
$fails = ($script:Checks | Where-Object { $_.result -in @("FAIL", "UNVERIFIABLE_ENVIRONMENT") }).Count
$blocked = ($script:Checks | Where-Object { $_.result -eq "BLOCKED_EXTERNAL_CREDENTIALS" }).Count

$verdict = if ($fails -gt 0) { "DENIED" }
           elseif ($blocked -gt 0) { "BLOCKED_EXTERNAL_CREDENTIALS" }
           else { "APPROVED" }

Write-Host ""
Write-Host "VERDICT: $verdict"
Write-Host "EVIDENCE: $evPath"
if ($verdict -eq "APPROVED") {
  Write-Host "LIVE_TRADING: ELIGIBLE (Owner-only; Phase 3 is never exposed to Members)"
} else {
  Write-Host "LIVE_TRADING: DISABLED"
  Write-Host "NOT_APPROVED_BECAUSE:"
  foreach ($c in ($script:Checks | Where-Object { $_.result -ne "PASS" })) {
    Write-Host ("  - {0} {1} {2} {3}" -f $c.id, $c.name, $c.result, $c.detail)
  }
}
if ($verdict -eq "DENIED") {
  Write-Host "NOTE: DENIED means a Live safety invariant does NOT hold, OR that verification could not run at all. Either way the release does not proceed: approving because a check could not be performed is the worst outcome available."
} elseif ($verdict -eq "BLOCKED_EXTERNAL_CREDENTIALS") {
  Write-Host "NOTE: BLOCKED_EXTERNAL_CREDENTIALS means every invariant provable here DOES hold, and the remaining evidence can only come from a real brokerage account. Nothing is broken. This is NOT a release, and it must never be reported as one."
}

$summary = [ordered]@{
  gate = "phase3"; task = 42; verdict = $verdict
  live_trading = $(if ($verdict -eq "APPROVED") { "ELIGIBLE" } else { "DISABLED" })
  member_exposure = "NEVER (Phase 3 is Owner-only by approved scope)"
  emitted_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  checks = $script:Checks
  evidence_dir = $transcriptDir
  approval_rule = "APPROVED requires every Live invariant to pass AND real broker credentials AND one bounded low-value order actually placed and reconciled. A FAIL outranks a block, so a real defect is never hidden behind 'waiting on credentials'. There is no override."
}
$summary | ConvertTo-Json -Depth 6 | Out-File $evPath -Encoding utf8
Write-Host "EVIDENCE_WRITTEN: $evPath"
exit 0
