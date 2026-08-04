#!/usr/bin/env pwsh
# verify.ps1 - verify fixtures/artifacts against a GoldenManifest.
# Exit 0 when unchanged; exit 1 with a field-level diff on any drift.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$py = Join-Path $root 'nt\.venv\Scripts\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }
& $py (Join-Path $PSScriptRoot 'golden.py') verify @args
exit $LASTEXITCODE
