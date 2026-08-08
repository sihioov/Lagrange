# Shared assertions for the Live runbooks (plan Todo 41), PowerShell twin.
#
# Kept behaviourally identical to assert.sh rather than merely similar: the
# runbooks are the procedure an operator follows during an incident, and one
# that behaves differently depending on which shell they happened to open is
# worse than having only one. The pair exists because this project's execution
# lanes genuinely span both.
#
# The rules, as in the shell twin:
#
#   * an assertion that selects NOTHING fails. Asserting on an absent key is a
#     failure, not a pass comparing $null to $null.
#   * exit code 2 from the CLI means "running but not ready" -- the system
#     working as designed -- and is never escalated as an outage.

$script:RunbookChecks = 0
$script:RunbookFailures = 0

function Write-Pass([string] $Description) {
  $script:RunbookChecks++
  Write-Host "  ok   $Description"
}

function Write-Fail([string] $Description) {
  $script:RunbookChecks++
  $script:RunbookFailures++
  Write-Host "  FAIL $Description" -ForegroundColor Red
}

# The absent-key case is why this takes the parsed object and a property name
# rather than a pre-extracted value: a caller that extracted first would hand
# us $null and we could not tell "absent" from "actually null".
function Assert-JsonEq {
  param(
    [Parameter(Mandatory)] $Json,
    [Parameter(Mandatory)] [string] $Path,
    [Parameter(Mandatory)] [AllowEmptyString()] [string] $Expected,
    [Parameter(Mandatory)] [string] $Description
  )
  # A null document is a FAILED assertion, not an exception. A helper that
  # threw would stop the runbook at its first problem, so an operator would
  # see one error instead of the full picture -- exactly when the full picture
  # is what they need.
  if ($null -eq $Json) {
    Write-Fail "$Description (no output to assert against)"
    return
  }
  $node = $Json
  foreach ($segment in $Path.Split('.')) {
    if ($null -eq $node -or -not ($node.PSObject.Properties.Name -contains $segment)) {
      Write-Fail "$Description (path $Path is absent - assertion selected nothing)"
      return
    }
    $node = $node.$segment
  }
  $actual = if ($node -is [bool]) { $node.ToString().ToLower() } else { [string] $node }
  if ($actual -eq $Expected) { Write-Pass $Description }
  else { Write-Fail "$Description (expected '$Expected', got '$actual')" }
}

function Assert-Exit {
  param([int] $Actual, [int] $Expected, [string] $Description)
  if ($Actual -eq $Expected) { Write-Pass "$Description (exit $Expected)" }
  else { Write-Fail "$Description (expected exit $Expected, got $Actual)" }
}

function Assert-Contains {
  param([string] $Haystack, [string] $Needle, [string] $Description)
  if ($Haystack -like "*$Needle*") { Write-Pass $Description }
  else { Write-Fail "$Description (missing '$Needle')" }
}

function Write-RunbookSummary([string] $Name) {
  Write-Host ""
  Write-Host "${Name}: $script:RunbookChecks checks, $script:RunbookFailures failures"
  if ($script:RunbookChecks -eq 0) {
    Write-Host "FAILED: the runbook asserted nothing at all" -ForegroundColor Red
    exit 1
  }
  if ($script:RunbookFailures -ne 0) { exit 1 }
}

function Invoke-Node {
  param([string[]] $NodeArgs)
  # `live_node` lives in a hyphenated directory and is not installed
  # (`[tool.uv] package = false`), so its parent must be on PYTHONPATH.
  # Without this every invocation fails with ModuleNotFoundError and exits 1,
  # which looks exactly like the node refusing to start -- and would have an
  # operator debugging the wrong thing entirely.
  $previous = $env:PYTHONPATH
  $env:PYTHONPATH = "$env:REPO_ROOT/nt/live-node"
  try {
    $out = & uv run --project "$env:REPO_ROOT/nt" python -m live_node @NodeArgs 2>$null
    $script:RunCode = $LASTEXITCODE
  } finally {
    $env:PYTHONPATH = $previous
  }
  $script:RunOut = ($out | Out-String).Trim()
  if ($script:RunOut) { $script:RunJson = $script:RunOut | ConvertFrom-Json } else { $script:RunJson = $null }
}
