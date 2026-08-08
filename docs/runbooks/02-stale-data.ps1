# Runbook: market data has gone stale (plan Todo 41)
#
# Runbook: market data has gone stale (plan Todo 41)
#
# Stale data blocks new orders (Risk Gateway check 3, AT-08). The node stays
# HEALTHY throughout: nothing is wrong with the process, and restarting it
# would neither refresh the feed nor preserve the record of what happened.
# The instinct to restart is the thing this runbook is written against.
#
# PowerShell twin of 02-stale-data.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: market data has gone stale"
Write-Host ""
try {

Write-Host "STEP 1 - confirm the node refuses to trade on stale data"
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account, "--reconciliation-green", "--data-stale")
Assert-Exit $RunCode 2 "stale data blocks, and blocking is not a fault"
Assert-JsonEq $RunJson "ready" "false" "no order may be submitted"
Assert-JsonEq $RunJson "refusal" "DATA_STALE" "the reason names the feed"
Assert-JsonEq $RunJson "healthy" "true" "do NOT restart the node"

Write-Host ""
Write-Host "STEP 2 - the stale-data block lifts by itself once data is fresh"
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account, "--reconciliation-green")
Assert-JsonEq $RunJson "refusal" "NODE_NOT_READY" "the stale-data refusal is gone"
Assert-JsonEq $RunJson "healthy" "true" "still no reason to restart anything"

Write-Host ""
Write-Host "STEP 3 - the block is counted, so it is visible on a dashboard"
Assert-JsonEq $RunJson "metrics.stale_data_blocks" "0" "the metric is reported, not absent"

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "02-stale-data"
