#!/usr/bin/env pwsh
# evidence.ps1 - write a sanitized evidence text for a GoldenManifest.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$py = Join-Path $root 'nt\.venv\Scripts\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }
& $py (Join-Path $PSScriptRoot 'golden.py') evidence @args
exit $LASTEXITCODE
