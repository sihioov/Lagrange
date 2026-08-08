# Runbook: an order is in UNKNOWN state (plan Todo 41)
#
# Runbook: an order is in UNKNOWN state (plan Todo 41)
#
# The single most dangerous state in the system. A timeout proves NOTHING:
# the order may be live at the broker. Resubmitting places a second real
# order against an account that may already hold the first.
#
# There is exactly one way out, and it is a broker lookup. The machine has no
# transition from UNKNOWN back into the submission path -- not a check that
# could be skipped, an edge that does not exist -- so this runbook cannot
# tell you to retry even if you want to.
#
# PowerShell twin of 03-unknown-order.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: an order is in UNKNOWN state"
Write-Host ""
try {

Write-Host "STEP 1 - see what a restart WOULD do before doing it"
$StateFile = Join-Path $LockDir "state.json"
Set-Content -Path $StateFile -Value '{"intent_states": {"oi-timedout": "SUBMITTING"}, "blocking_mismatch_kinds": ["UNRESOLVED_INTENT"], "fills_to_apply": [], "lookups_required": ["oi-timedout"]}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-Exit $RunCode 2 "an unresolved intent blocks trading"
Assert-JsonEq $RunJson "outcome" "LOOKUPS_REQUIRED" "the next action is a LOOKUP, not a retry"
Assert-JsonEq $RunJson "to_sweep" "oi-timedout" "the in-flight intent is swept to UNKNOWN first"
Assert-JsonEq $RunJson "lookups_required" "oi-timedout" "the broker must be asked about this order"
Assert-JsonEq $RunJson "may_trade" "false" "nothing trades until it is settled"

Write-Host ""
Write-Host "STEP 2 - ask the broker (operator action)"
Write-Host "        Query the order by its client order id. Do NOT resubmit."
Write-Host "        Design 16: resubmission is forbidden before a lookup resolves it."

Write-Host ""
Write-Host "STEP 3 - once resolved, the intent is settled and startup proceeds"
Set-Content -Path $StateFile -Value '{"intent_states": {"oi-timedout": "ACCEPTED"}, "blocking_mismatch_kinds": [], "fills_to_apply": [], "lookups_required": []}'
Invoke-Node @("--lock-dir", $LockDir, "plan-startup", "--account", $Account, "--input", $StateFile)
Assert-Exit $RunCode 0 "a settled intent no longer blocks"
Assert-JsonEq $RunJson "outcome" "READY" "a settled order is not swept"

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "03-unknown-order"
