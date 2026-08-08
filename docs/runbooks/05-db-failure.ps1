# Runbook: the database is unavailable (plan Todo 41)
#
# Runbook: the database is unavailable (plan Todo 41)
#
# Design 16: a failed DB write blocks new Live orders. A decision that cannot
# be recorded must not authorise an order, because after a restart there would
# be nothing to reconcile against.
#
# Note the direction of the failure. The system does not keep trading and log
# later; it stops. An operator arriving at this runbook should expect a HALT,
# and its absence is the emergency -- not its presence.
#
# PowerShell twin of 05-db-failure.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: the database is unavailable"
Write-Host ""
try {

Write-Host "STEP 1 - confirm new orders are blocked, not queued"
Write-Host "        The Risk Gateway denies with NOT_PERSISTED, graded CRITICAL,"
Write-Host "        because an unrecordable decision breaks the audit trail."

Write-Host ""
Write-Host "STEP 2 - the node is still HEALTHY; do not restart it"
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account)
Assert-JsonEq $RunJson "healthy" "true" "the process is fine; the database is not"
Assert-Exit $RunCode 2 "blocked, which is the designed behaviour"

Write-Host ""
Write-Host "STEP 3 - after the database returns, reconcile BEFORE resuming"
$StateFile = Join-Path $LockDir "state.json"
Set-Content -Path $StateFile -Value '{"intent_states": {"oi-inflight": "SUBMITTED"}, "blocking_mismatch_kinds": ["UNRESOLVED_INTENT"], "fills_to_apply": [], "lookups_required": ["oi-inflight"]}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-JsonEq $RunJson "to_sweep" "oi-inflight" "orders in flight during the outage are swept"
Assert-JsonEq $RunJson "may_trade" "false" "trading stays blocked until they are settled"

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "05-db-failure"
