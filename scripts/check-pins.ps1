#!/usr/bin/env pwsh
# check-pins.ps1 — assert every pinned toolchain/package matches the APPROVED lines.
# Approved pins (draft 2026-08-04 line 40): Rust 1.97.1, CPython 3.12, Node >=24 <25,
# nautilus_trader==1.231.0, polars>=0.54,<0.55.
# Two-sided drift detection:
#   (A) the pin FILE must still hold the approved constant (catches file edits);
#   (B) the INSTALLED toolchain must match the pin file (catches toolchain drift).
# Exit 0 when all pins hold; exit 1 NAMING every drifting field otherwise.
# Run from anywhere in the repo; root is resolved as the parent of scripts/.
# Twin script: scripts/check-pins.sh (CI / clean containers).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$drifts = @()

$APPROVED_RUST = '1.97.1'
$APPROVED_PY  = '3.12'
$APPROVED_NODE = '>=24 <25'
$APPROVED_NT  = 'nautilus_trader==1.231.0'
$APPROVED_POLARS = 'polars>=0.54,<0.55'

function Invoke-ToolVersion([string]$Command, [string[]]$Arguments) {
    try {
        $out = & $Command @Arguments 2>&1 | Out-String
        return $out.Trim()
    } catch {
        return $null
    }
}

# --- Rust --------------------------------------------------------------------
$rt = Join-Path $root 'rust-toolchain.toml'
if (-not (Test-Path $rt)) {
    $drifts += 'rust-toolchain.toml: missing'
} else {
    $m = [regex]::Match((Get-Content $rt -Raw), 'channel\s*=\s*"([^"]+)"')
    if (-not $m.Success) {
        $drifts += 'rust-toolchain.toml: no channel= pin found'
    } else {
        $pin = $m.Groups[1].Value
        if ($pin -ne $APPROVED_RUST) {
            $drifts += "rust-toolchain.toml: approved rust pin is $APPROVED_RUST but channel is $pin"
        }
        # Installed toolchain probe; fall back to `stable` when the pinned one is absent.
        $out = Invoke-ToolVersion 'rustc' @('--version')
        $am = [regex]::Match($out, 'rustc (\d+\.\d+\.\d+)')
        if (-not $am.Success) {
            $out = Invoke-ToolVersion 'rustc' @('+stable', '--version')
            $am = [regex]::Match($out, 'rustc (\d+\.\d+\.\d+)')
        }
        if (-not $am.Success) {
            $drifts += "rustc: could not read installed version (raw: '$out')"
        } elseif ($am.Groups[1].Value -ne $pin) {
            $drifts += "rustc: pin $pin (rust-toolchain.toml) but installed $($am.Groups[1].Value)"
        }
    }
}

# --- Python ------------------------------------------------------------------
$pv = Join-Path $root '.python-version'
if (-not (Test-Path $pv)) {
    $drifts += '.python-version: missing'
} else {
    $pin = (Get-Content $pv -Raw).Trim()
    if ($pin -ne $APPROVED_PY) {
        $drifts += ".python-version: approved python pin is $APPROVED_PY but file says $pin"
    }
    $out = Invoke-ToolVersion 'python' @('--version')
    $am = [regex]::Match($out, 'Python (\d+\.\d+\.\d+)')
    if (-not $am.Success) {
        $drifts += "python: could not read installed version (raw: '$out')"
    } else {
        $actual = $am.Groups[1].Value
        # .python-version may hold a minor pin like "3.12"; treat it as a prefix match.
        if ($actual -ne $pin -and -not $actual.StartsWith($pin + '.')) {
            $drifts += "python: pin $pin (.python-version) but installed $actual"
        }
    }
}

# --- Node --------------------------------------------------------------------
$pj = Join-Path $root 'package.json'
if (-not (Test-Path $pj)) {
    $drifts += 'package.json: missing (no node pin)'
} else {
    try {
        $json = Get-Content $pj -Raw | ConvertFrom-Json
        $range = $json.engines.node
    } catch {
        $drifts += 'package.json: unparseable JSON'
        $range = $null
    }
    if ($null -eq $range -or '' -eq $range.ToString()) {
        $drifts += "package.json: engines.node missing (approved: '$APPROVED_NODE')"
    } elseif ($range.ToString().Trim() -ne $APPROVED_NODE) {
        $drifts += "package.json: approved engines.node is '$APPROVED_NODE' but found '$($range.ToString().Trim())'"
    }
    $out = Invoke-ToolVersion 'node' @('--version')
    $am = [regex]::Match($out, 'v(\d+)\.(\d+)\.(\d+)')
    if (-not $am.Success) {
        $drifts += "node: could not read installed version (raw: '$out')"
    } elseif ($range) {
        $major = [int]$am.Groups[1].Value
        $minMajor = $null; $maxMajor = $null
        if ($range.ToString() -match '>=\s*(\d+)') { $minMajor = [int]$Matches[1] }
        if ($range.ToString() -match '<\s*(\d+)')  { $maxMajor = [int]$Matches[1] }
        $ok = $true
        if ($null -ne $minMajor -and $major -lt $minMajor) { $ok = $false }
        if ($null -ne $maxMajor -and $major -ge $maxMajor) { $ok = $false }
        if (-not $ok) {
            $drifts += "node: engines '$($range.ToString())' (package.json) but installed $($am.Groups[1].Value).$($am.Groups[2].Value).$($am.Groups[3].Value)"
        }
    }
}

# --- NautilusTrader / Polars pins (nt project) --------------------------------
$nt = Join-Path $root 'nt\pyproject.toml'
if (-not (Test-Path $nt)) {
    $drifts += 'nt/pyproject.toml: missing (no NT pin)'
} else {
    $content = Get-Content $nt -Raw
    if ($content -notmatch 'nautilus[_\-]trader\s*==\s*1\.231\.0') {
        $drifts += "nt/pyproject.toml: nautilus_trader not pinned to $APPROVED_NT"
    }
    if ($content -notmatch 'polars\s*>=\s*0\.54[^\r\n]*<\s*0\.55') {
        $drifts += "nt/pyproject.toml: polars not pinned to $APPROVED_POLARS"
    }
}

if ($drifts.Count -gt 0) {
    Write-Host 'PIN DRIFT DETECTED:' -ForegroundColor Red
    $drifts | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
    exit 1
}

Write-Host 'ALL PINS OK (rustc/python/node/NT)' -ForegroundColor Green
exit 0
