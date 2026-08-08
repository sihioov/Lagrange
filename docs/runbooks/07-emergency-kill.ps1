# Runbook: EMERGENCY - stop Live now (plan Todo 41)
#
# Runbook: EMERGENCY - stop Live now (plan Todo 41)
#
# The one runbook that must work when everything else is on fire, so it is
# the shortest and has no preconditions.
#
# Engaging the kill switch is never blocked, never needs a reason, and never
# waits for reconciliation. A precondition on STOPPING is a precondition that
# fails at the worst possible moment. Everything careful in this system is on
# the other direction: turning Live back on.
#
# PowerShell twin of 07-emergency-kill.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: EMERGENCY - stop Live now"
Write-Host ""
try {

Write-Host "STEP 1 - ENGAGE. One call. No reason required."
Write-Host "        POST /api/v1/admin/live/kill-switch/enable"
Write-Host "        Owner + fresh MFA only. Nothing else gates it."

Write-Host ""
Write-Host "STEP 2 - confirm new orders are refused"
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account, "--kill-switch-engaged", "--reconciliation-green")
Assert-Exit $RunCode 2 "killed is not-ready, and that is correct"
Assert-JsonEq $RunJson "ready" "false" "no order may be submitted"
Assert-JsonEq $RunJson "refusal" "LIVE_KILL_SWITCH_ENGAGED" "the kill switch is reported FIRST"
Assert-JsonEq $RunJson "metrics.kill_switch_state" "1" "the gauge reads engaged"
Assert-JsonEq $RunJson "healthy" "true" "the node is healthy; do not restart it"

Write-Host ""
Write-Host "STEP 3 - orders already at the broker follow the cancel policy"
Write-Host "        LEAVE (default) | CANCEL_WORKING | CANCEL_UNFILLED_ONLY."
Write-Host "        No policy touches an UNKNOWN order: we do not have its broker"
Write-Host "        number, so a cancel would fail or name the WRONG order."

Write-Host ""
Write-Host "STEP 4 - to resume, see 06-reconciliation-mismatch"
Write-Host "        Disengaging needs Owner + fresh MFA + a GREEN reconciliation."

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "07-emergency-kill"
