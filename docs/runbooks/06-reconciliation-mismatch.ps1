# Runbook: our books disagree with the broker (plan Todo 41)
#
# Runbook: our books disagree with the broker (plan Todo 41)
#
# Design 16: an internal-vs-broker position mismatch pauses Live strategies
# and requires Owner approval.
#
# The rule is that the BROKER is the truth about the broker. A position we
# cannot account for is a position that really exists in a real account, and
# adopting our own number would hide it. Exactly one difference is resolvable
# automatically -- a fill we simply had not applied -- and everything else
# needs an Owner.
#
# PowerShell twin of 06-reconciliation-mismatch.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: our books disagree with the broker"
Write-Host ""
try {

Write-Host "STEP 1 - identify the mismatch and confirm it blocks"
$StateFile = Join-Path $LockDir "state.json"
Set-Content -Path $StateFile -Value '{"intent_states": {}, "blocking_mismatch_kinds": ["POSITION", "UNMAPPED_BROKER_ORDER"], "fills_to_apply": [], "lookups_required": []}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-Exit $RunCode 2 "an unexplained difference blocks trading"
Assert-JsonEq $RunJson "outcome" "BLOCKED" "this needs a person"
Assert-JsonEq $RunJson "blocking_reasons" "POSITION UNMAPPED_BROKER_ORDER" "both differences are named"

Write-Host ""
Write-Host "STEP 2 - an UNMAPPED_BROKER_ORDER is the most serious kind"
Write-Host "        A real order at the broker that no intent of ours manages."
Write-Host "        Do not cancel it blindly; establish what placed it first."

Write-Host ""
Write-Host "STEP 3 - a missed fill, by contrast, resolves without an Owner"
Set-Content -Path $StateFile -Value '{"intent_states": {}, "blocking_mismatch_kinds": [], "fills_to_apply": ["E-1"], "lookups_required": []}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-JsonEq $RunJson "outcome" "READY" "applying the fill clears it"
Assert-JsonEq $RunJson "may_trade" "true" "nothing needed a judgement call"

Write-Host ""
Write-Host "STEP 4 - re-enabling Live requires a GREEN run, not an explanation"
Write-Host "        POST /api/v1/admin/live/kill-switch/disable answers 409"
Write-Host "        LIVE_RECONCILIATION_REQUIRED until readiness is READY."

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "06-reconciliation-mismatch"
