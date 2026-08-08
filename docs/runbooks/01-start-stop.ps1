# Runbook: start and stop a Live node (plan Todo 41)
#
# Runbook: start and stop a Live node (plan Todo 41)
#
# Starting a Live node is NOT the same as permitting it to trade, and this
# runbook exists mainly to make that impossible to confuse. `start` leaves the
# node in RECONCILING and exits 2; only a green reconciliation moves it on.
# An operator who read "started" as "trading" would believe orders were
# flowing when nothing had been checked against the broker.
#
# PowerShell twin of 01-start-stop.sh. Same steps, same assertions, same
# descriptions -- so the two outputs can be diffed line for line.

$ErrorActionPreference = "Stop"
$env:REPO_ROOT = (Resolve-Path "$PSScriptRoot/../..").Path
. "$env:REPO_ROOT/docs/runbooks/lib/assert.ps1"
$Account = if ($args.Count -ge 1) { $args[0] } else { "runbook-acct" }
$LockDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runbook-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $LockDir | Out-Null
Write-Host "Runbook: start and stop a Live node"
Write-Host ""
try {

Write-Host "STEP 1 - claim the account and begin reconciling"
Invoke-Node @("--lock-dir", $LockDir, "start", "--account", $Account)
Assert-Exit $RunCode 2 "a fresh start is running but NOT ready"
Assert-JsonEq $RunJson "started" "true" "the account was claimed"
Assert-JsonEq $RunJson "state" "RECONCILING" "a node never starts READY"
Assert-JsonEq $RunJson "ready" "false" "trading is not permitted yet"
Assert-JsonEq $RunJson "healthy" "true" "the process itself is fine"

Write-Host ""
Write-Host "STEP 2 - a second node for the same account must be refused"
# A REAL operating-system pid of a live process, for the same reason the shell
# twin cannot use $$: a lock naming a pid the OS cannot resolve looks stale and
# is reclaimed, so the runbook would "pass" while showing the opposite.
#
# The interpreter is resolved through uv rather than named as "python": on
# Windows, bare `python` is usually the Microsoft Store stub, which starts and
# exits IMMEDIATELY. The lock would then name a dead pid, be reclaimed, and
# this step would report success while proving the exact opposite.
$Interpreter = (& uv run --project "$env:REPO_ROOT/nt" python -c "import sys; print(sys.executable)" | Out-String).Trim()
# The holder is a FILE, not `-c "..."`. Start-Process joins ArgumentList
# elements with spaces and does not quote them, so `-c "import time; ..."`
# arrives as `-c import` followed by garbage and the process dies instantly --
# which is exactly the failure the guard below caught.
$HolderScript = Join-Path $LockDir "holder.py"
Set-Content -Path $HolderScript -Value "import time`ntime.sleep(25)"
$HolderProc = Start-Process -FilePath $Interpreter -ArgumentList @("`"$HolderScript`"") -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 900
if ($HolderProc.HasExited) { throw "the holder process died; this step cannot prove anything" }
$lockPath = Join-Path $LockDir ("live-node-" + $Account + ".lock")
Set-Content -Path $lockPath -Value ("{0}`n{1}" -f $HolderProc.Id, $Account) -NoNewline
Invoke-Node @("--lock-dir", $LockDir, "start", "--account", $Account)
Assert-Exit $RunCode 1 "a duplicate node is a FAILURE, not a safe refusal"
Assert-JsonEq $RunJson "error" "NODE_ALREADY_RUNNING" "the refusal names itself"
Assert-JsonEq $RunJson "started" "false" "nothing was claimed twice"
Assert-JsonEq $RunJson "held_by_pid" "$($HolderProc.Id)" "the refusal names who holds it"

Write-Host ""
Write-Host "STEP 2b - a lock left by a DEAD process is reclaimed, not fatal"
Set-Content -Path $lockPath -Value ("999999999`n" + $Account) -NoNewline
Invoke-Node @("--lock-dir", $LockDir, "start", "--account", $Account)
Assert-Exit $RunCode 2 "a stale lock does not block a restart"
Assert-JsonEq $RunJson "started" "true" "the account was reclaimed"

Write-Host ""
Write-Host "STEP 3 - status reports the running node"
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account)
Assert-Exit $RunCode 2 "running, still not ready"
Assert-JsonEq $RunJson "running" "true" "the lock is held"

Write-Host ""
Write-Host "STEP 4 - stop: release the lock, then the account is free"
Remove-Item $lockPath -Force
Invoke-Node @("--lock-dir", $LockDir, "status", "--account", $Account)
Assert-JsonEq $RunJson "running" "false" "no node holds the account"
Assert-JsonEq $RunJson "state" "STARTING" "absent is not the same as STOPPED"

} finally {
  if ($HolderProc -and -not $HolderProc.HasExited) { $HolderProc.Kill() }
  Remove-Item -Recurse -Force $LockDir -ErrorAction SilentlyContinue
}
Write-RunbookSummary "01-start-stop"
