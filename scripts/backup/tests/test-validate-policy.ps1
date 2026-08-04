#!/usr/bin/env pwsh
# test-validate-policy.ps1 - red-first acceptance harness for scripts/backup/validate-policy.ps1.
# Proves, with machine-checked assertions on real validator transcripts:
#   1. a synthetic COMPLETE backup set validates (exit 0) - all DB/file classes, hashes,
#      retention, storage rules, and secret exclusions confirmed;
#   2. an incomplete manifest (missing db_wal class) is REJECTED before any restore can start;
#   3. a manifest missing an artifact sha256 is REJECTED, naming the missing field;
#   4. a tampered base-backup hash is REJECTED, naming file + declared vs computed hash;
#   5. an archive containing a fake secret marker is REJECTED, naming marker and file;
#   6. the validator is deterministic: identical input produces byte-identical output.
# Every rejection must happen at the policy gate (validate-policy exit != 0) BEFORE any
# restore command - this harness never starts a restore, only the gate.
# Twin: scripts/backup/tests/test-validate-policy.sh
# Exit 0 only when all assertions hold.
$ErrorActionPreference = 'Stop'
$tests = @()
$failures = @()

function Invoke-Validator([string]$FixturePath) {
    $validator = Join-Path (Split-Path -Parent $PSScriptRoot) 'validate-policy.ps1'
    if (-not (Test-Path -LiteralPath $validator)) {
        return [pscustomobject]@{ Code = 127; Output = "validate-policy.ps1 not found: $validator" }
    }
    $out = & $validator -SetPath $FixturePath -Gate default 2>&1
    $code = $LASTEXITCODE
    return [pscustomobject]@{ Code = $code; Output = ($out | Out-String) }
}

function Assert-ValidatorCase([string]$Name, [string]$Fixture, [int]$ExpectCode, [string[]]$ExpectContains) {
    $script:tests += $Name
    $r = Invoke-Validator $Fixture
    $ok = ($r.Code -eq $ExpectCode)
    $missing = @()
    if ($ok) {
        foreach ($needle in $ExpectContains) {
            if (-not $r.Output.Contains($needle)) { $ok = $false; $missing += $needle }
        }
    }
    if ($ok) {
        Write-Host "PASS $Name" -ForegroundColor Green
    }
    else {
        $script:failures += "$Name`n  expected exit=$ExpectCode contains=[$($ExpectContains -join ', ')] got exit=$($r.Code)`n  output:`n$($r.Output)"
        Write-Host "FAIL $Name" -ForegroundColor Red
        $r.Output | ForEach-Object { Write-Host "    $_" }
    }
}

Assert-ValidatorCase 'complete set validates (all classes, hashes, retention, secrets-excluded)' `
    (Join-Path $PSScriptRoot 'fixtures\complete') 0 @('POLICY OK')

Assert-ValidatorCase 'incomplete manifest (missing db_wal class) rejected before restore' `
    (Join-Path $PSScriptRoot 'fixtures\incomplete-missing-wal') 1 @('POLICY REJECTED', 'db_wal')

Assert-ValidatorCase 'missing artifact sha256 rejected and named' `
    (Join-Path $PSScriptRoot 'fixtures\incomplete-missing-hash') 1 @('POLICY REJECTED', 'sha256')

Assert-ValidatorCase 'tampered base-backup hash rejected and named' `
    (Join-Path $PSScriptRoot 'fixtures\tampered-hash') 1 @('POLICY REJECTED', 'sha256', 'base.tar.gz')

Assert-ValidatorCase 'archive containing fake secret marker rejected, no restore' `
    (Join-Path $PSScriptRoot 'fixtures\fake-secret') 1 @('POLICY REJECTED', 'LAGRANGE_SECRET_MARKER', 'kis-app-secret.plaintext')

# Determinism: same input, twice, must produce byte-identical output and the same exit code.
$tests += 'deterministic on identical input'
$r1 = Invoke-Validator (Join-Path $PSScriptRoot 'fixtures\complete')
$r2 = Invoke-Validator (Join-Path $PSScriptRoot 'fixtures\complete')
if ($r1.Code -eq $r2.Code -and $r1.Code -eq 0 -and $r1.Output -eq $r2.Output) {
    Write-Host 'PASS deterministic on identical input' -ForegroundColor Green
}
else {
    $script:failures += "determinism: run1 exit=$($r1.Code) run2 exit=$($r2.Code) identical=$($r1.Output -eq $r2.Output)"
    Write-Host 'FAIL deterministic on identical input' -ForegroundColor Red
}

if ($failures.Count -gt 0) {
    Write-Host "`nBACKUP POLICY TESTS FAILED ($($tests.Count - $failures.Count)/$($tests.Count))" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "`n---`n$_" -ForegroundColor Red }
    exit 1
}
Write-Host "`nALL BACKUP POLICY TESTS PASSED ($($tests.Count)/$($tests.Count))" -ForegroundColor Green
exit 0
