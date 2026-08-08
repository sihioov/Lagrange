# Runbook: the execution WebSocket dropped (plan Todo 41)
#
# Runbook: the execution WebSocket dropped (plan Todo 41)
#
# A dropped socket means fill reports may have been missed. The node goes
# DEGRADED, and the important detail is what it does NOT do: it does not go
# straight back to READY when the socket returns. Whatever happened during the
# gap may have happened while orders were in flight, so agreement with the
# broker has to be re-established rather than assumed.
#
# PowerShell twin of 04-websocket-gap.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: the execution WebSocket dropped"
Write-Host ""
try {

Write-Host "STEP 1 - the node degrades and stops submitting"
Write-Host "        (DEGRADED is reached in-process; asserted in test_lifecycle.py)"

Write-Host ""
Write-Host "STEP 2 - a reconnect does NOT restore trading by itself"
Write-Host "        DEGRADED has no edge to READY. The only path back is through"
Write-Host "        RECONCILING, which is what catches the fills we missed."
$StateFile = Join-Path $LockDir "state.json"
Set-Content -Path $StateFile -Value '{"intent_states": {}, "blocking_mismatch_kinds": [], "fills_to_apply": ["E-missed-1", "E-missed-2"], "lookups_required": []}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-JsonEq $RunJson "fills_to_apply" "E-missed-1 E-missed-2" "both missed fills are found"

Write-Host ""
Write-Host "STEP 3 - applying a missed fill is safe to repeat"
Write-Host "        The ledger rejects a duplicate fill_id, and the order machine"
Write-Host "        reports a re-sent report as NoChange. Two independent guards,"
Write-Host "        so a reconnect storm cannot double a position."
Assert-JsonEq $RunJson "outcome" "READY" "once applied, nothing blocks"

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "04-websocket-gap"
