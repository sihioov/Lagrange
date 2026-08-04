#!/usr/bin/env pwsh
# generate.ps1 - regenerate a GoldenManifest from a config (see golden.py generate).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$py = Join-Path $root 'nt\.venv\Scripts\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }
& $py (Join-Path $PSScriptRoot 'golden.py') generate @args
exit $LASTEXITCODE
