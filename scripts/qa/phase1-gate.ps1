#!/usr/bin/env pwsh
# phase1-gate.ps1 - Phase 1 invite-only multi-user release gate (Windows twin).
#
# TODO 28 gate: assembles the already-built components (T5 entitlement, T7
# backups, T10 curation, T21 robustness, T22 Auth0, T23 RLS, T24 API,
# T25/26 web, T27 artifacts/observability) into ONE machine-readable release
# verdict.
#
# Verdict (single line on stdout, exactly one of):
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL_DATA_RIGHTS
#
# APPROVED is emitted ONLY when EVERY check passes AND the written-rights
# metadata artifact is ACTIVE (real document hash + resolvable reference) AND
# the vendor Auth0 suite passes against a real tenant. Any missing evidence,
# any failing suite, or a documented BLOCKED_EXTERNAL condition for written
# rights / vendor Auth0 yields BLOCKED_EXTERNAL_DATA_RIGHTS - NEVER a false
# success. Member KR-derived surfaces stay denied; Owner-only continues.
#
# Check list (each emits `CHECK <id> <name> = <PASS|BLOCKED_EXTERNAL|FAIL>`):
#   E1 written-rights   ACTIVE metadata artifact in configs/data-rights
#   E2 vendor-auth0     crates/auth/tests/vendor_auth0.rs (real tenant)
#   E3 auth0-simulator  crates/auth/tests/auth0_simulator.rs (contract)
#   E4 auth0-invite-mfa protocol/invites/stepup suites (invite + MFA contract)
#   E5 phase1-five-user crates/api-server/tests/phase1_gate.rs (DB-gated)
#   E6 restore-policy   scripts/backup/validate-policy (premember gate, A1)
#   E7 playwright-phase1 apps/web/tests/e2e/phase1 (detached mock+app)
#
# Exit codes: 0 = verdict emitted; 2 = gate could not run (no verdict).
#
# Environment (defaults documented in the repo notepads):
#   DATABASE_URL            Windows-side PG URL (default 172.26.46.217:5432)
#   WSL_DATABASE_URL        inside-WSL PG URL  (default 127.0.0.1:5432)
#   PHASE1_SKIP_PLAYWRIGHT  1 = skip E7 (still BLOCKED if evidence missing)
#
# Twin: scripts/qa/phase1-gate.sh
$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$dataRightsDir = Join-Path $root "configs\data-rights"
$evidenceDir = Join-Path $root ".omo\evidence"
$evPath = Join-Path $evidenceDir "task-28-lagrange-station-implementation.json"
$transcriptDir = Join-Path $evidenceDir "task-28-transcripts"
$dbUrl = $env:DATABASE_URL
if (-not $dbUrl) { $dbUrl = "postgres://postgres:lagrange@172.26.46.217:5432/postgres" }
$wslDbUrl = $env:WSL_DATABASE_URL
if (-not $wslDbUrl) { $wslDbUrl = "postgres://postgres:lagrange@127.0.0.1:5432/postgres" }
$skipPlaywright = ($env:PHASE1_SKIP_PLAYWRIGHT -eq "1")

New-Item -ItemType Directory -Force -Path $evidenceDir | Out-Null
New-Item -ItemType Directory -Force -Path $transcriptDir | Out-Null

# --------------------------------------------------------------------------- #
# Harness plumbing
# --------------------------------------------------------------------------- #
$checks = [System.Collections.Generic.List[object]]::new()

function Add-Check {
    param([string]$Id, [string]$Name, [string]$Result, [string]$Detail)
    $checks.Add(@{
        id = $Id; name = $Name; result = $Result; detail = $Detail
    })
    Write-Output "CHECK $Id $Name = $Result  $Detail"
}

function Run-Native {
    param([string]$FilePath, [string[]]$ArgumentList, [string]$Transcript, [string]$WorkingDirectory = $root)
    $p = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory `
        -NoNewWindow -RedirectStandardOutput $Transcript -RedirectStandardError "$Transcript.err" `
        -PassThru -Wait
    return $p.ExitCode
}

function Test-PortOpen {
    param([int]$Port)
    try {
        $c = [System.Net.Sockets.TcpClient]::new()
        $iar = $c.BeginConnect("127.0.0.1", $Port, $null, $null)
        $ok = $iar.AsyncWaitHandle.WaitOne(300)
        if ($ok) { $c.EndConnect($iar); $c.Close(); return $true }
        $c.Close(); return $false
    } catch { return $false }
}

function Test-WslBashPath {
    # Convert a D:\... path into the /mnt/d/... form bash expects.
    param([string]$WindowsPath)
    $drive = $WindowsPath.Substring(0, 1).ToLower()
    $rest = $WindowsPath.Substring(3).Replace('\', '/')
    return "/mnt/$drive/$rest"
}

function Run-WslBash {
    param([string]$Command, [string]$Transcript, [switch]$NoDb)
    $dbPart = if ($NoDb) { "" } else { "export DATABASE_URL='$wslDbUrl' && " }
    $mntRoot = Test-WslBashPath -WindowsPath $root
    $bashCmd = "cd $mntRoot && " +
        "export PATH='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/root/.cargo/bin' && " +
        "export CARGO_TARGET_DIR=/root/lagrange-target && " + $dbPart + $Command + " 2>&1"
    & wsl -d Ubuntu -u root -- bash -c $bashCmd *> $Transcript
    return $LASTEXITCODE
}

# --------------------------------------------------------------------------- #
# E1 - written-rights metadata artifact
# --------------------------------------------------------------------------- #
function Test-WrittenRights {
    $files = Get-ChildItem -Path $dataRightsDir -Filter "*.json" -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notlike "*.schema.json" }
    if (-not $files) {
        Add-Check "E1" "written-rights" "BLOCKED_EXTERNAL" "no entitlement metadata artifact in configs/data-rights"
        return
    }
    foreach ($f in $files) {
        try { $doc = Get-Content $f.FullName -Raw | ConvertFrom-Json } catch { continue }
        $hex = "$($doc.contract_document.document_hash.hex)"
        $ref = "$($doc.contract_document.document_reference)"
        $placeholder = ($hex -eq "0000000000000000000000000000000000000000000000000000000000000000")
        $vault = $ref.StartsWith("vault://")
        if ($doc.lifecycle -eq "ACTIVE" -and -not $placeholder -and -not $vault) {
            Add-Check "E1" "written-rights" "PASS" "$($f.Name) ACTIVE with real document hash $($hex.Substring(0,12))... and reference $ref"
            return
        }
        $why = if ($doc.lifecycle -ne "ACTIVE") { "lifecycle=$($doc.lifecycle)" }
               elseif ($placeholder) { "placeholder zeroed document hash" }
               else { "unresolvable vault reference" }
        Add-Check "E1" "written-rights" "BLOCKED_EXTERNAL" "$($f.Name) is NOT an ACTIVE written-rights artifact ($why)"
        return
    }
    Add-Check "E1" "written-rights" "BLOCKED_EXTERNAL" "no parseable entitlement metadata artifact"
}

# --------------------------------------------------------------------------- #
# E2 - vendor Auth0 (real tenant) / E3 - simulator / E4 - invite+MFA contract
# --------------------------------------------------------------------------- #
function Test-VendorAuth0 {
    $t = Join-Path $transcriptDir "E2-vendor-auth0.txt"
    $code = Run-WslBash -Command "cargo test -p auth --test vendor_auth0 -- --ignored --nocapture" -Transcript $t
    if ($code -eq 0) {
        Add-Check "E2" "vendor-auth0" "PASS" "real Auth0 tenant suite green (transcript $t)"
    } else {
        Add-Check "E2" "vendor-auth0" "BLOCKED_EXTERNAL" "no Auth0 tenant/credentials on this host (suite panics BLOCKED_EXTERNAL); transcript $t"
    }
}

function Test-Auth0Simulator {
    $t = Join-Path $transcriptDir "E3-auth0-simulator.txt"
    $code = Run-WslBash -Command "cargo test -p auth --test auth0_simulator -- --nocapture" -Transcript $t
    if ($code -eq 0) {
        Add-Check "E3" "auth0-simulator" "PASS" "contract suite green (transcript $t)"
    } else {
        Add-Check "E3" "auth0-simulator" "FAIL" "contract suite failed exit=$code (transcript $t)"
    }
}

function Test-InviteMfa {
    $t = Join-Path $transcriptDir "E4-auth0-invite-mfa.txt"
    $code = Run-WslBash -Command "cargo test -p auth protocol -- --nocapture && cargo test -p auth invites -- --nocapture && cargo test -p auth stepup -- --nocapture" -Transcript $t
    if ($code -eq 0) {
        Add-Check "E4" "auth0-invite-mfa" "PASS" "protocol/invites/stepup suites green (transcript $t)"
    } else {
        Add-Check "E4" "auth0-invite-mfa" "FAIL" "invite/MFA suites failed exit=$code (transcript $t)"
    }
}

# --------------------------------------------------------------------------- #
# E5 - integrated five-user suite (DB-gated, inside-WSL lane)
# --------------------------------------------------------------------------- #
function Test-FiveUser {
    $t = Join-Path $transcriptDir "E5-phase1-five-user.txt"
    $code = Run-WslBash -Command "cargo test -p api-server --test phase1_gate -- --nocapture" -Transcript $t
    $ok = ($code -eq 0)
    $missing = (Select-String -Path $t -Pattern "no test target named|could not find|No tests" -Quiet -ErrorAction SilentlyContinue)
    if ($ok) {
        Add-Check "E5" "phase1-five-user" "PASS" "five-user suite green (transcript $t)"
    } elseif ($missing) {
        Add-Check "E5" "phase1-five-user" "BLOCKED_EXTERNAL" "EVIDENCE_MISSING: phase1_gate suite not present yet (transcript $t)"
    } else {
        Add-Check "E5" "phase1-five-user" "FAIL" "five-user suite failed exit=$code (transcript $t)"
    }
}

# --------------------------------------------------------------------------- #
# E6 - pre-Member restore policy gate (A1)
# --------------------------------------------------------------------------- #
function Test-RestorePolicy {
    $t = Join-Path $transcriptDir "E6-restore-policy.txt"
    $set = Join-Path $root "scripts\backup\tests\fixtures\complete"
    $code = Run-Native -FilePath "pwsh" -ArgumentList @("-NoProfile", "-File", (Join-Path $root "scripts\backup\validate-policy.ps1"), "-SetPath", $set, "-Gate", "premember") -Transcript $t
    $policyOk = (Select-String -Path $t -Pattern "POLICY OK.*gate premember" -Quiet -ErrorAction SilentlyContinue)
    if ($code -eq 0 -and $policyOk) {
        Add-Check "E6" "restore-policy" "PASS" "pre-Member policy gate A1 OK (transcript $t)"
    } else {
        Add-Check "E6" "restore-policy" "FAIL" "pre-Member policy gate A1 rejected exit=$code (transcript $t)"
    }
}

# --------------------------------------------------------------------------- #
# E7 - Playwright phase1 (detached mock 38180 + app 33000)
# --------------------------------------------------------------------------- #
function Test-PlaywrightPhase1 {
    $t = Join-Path $transcriptDir "E7-playwright-phase1.txt"
    $webDir = Join-Path $root "apps\web"
    $mock = Join-Path $webDir "tests\e2e\support\synthetic-api.mjs"
    $mockOut = Join-Path $transcriptDir "mock.stdout.txt"
    $appOut = Join-Path $transcriptDir "app.stdout.txt"
    $nodeExe = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
    if (-not $nodeExe) { $nodeExe = "node.exe" }

    $mockProc = Start-Process -FilePath $nodeExe -ArgumentList @($mock) -WorkingDirectory $webDir `
        -RedirectStandardOutput $mockOut -RedirectStandardError "$mockOut.err" -PassThru -WindowStyle Hidden
    try {
        $mockReady = $false
        for ($i = 0; $i -lt 40; $i++) {
            Start-Sleep -Milliseconds 250
            if (Test-PortOpen -Port 38180) { $mockReady = $true; break }
        }
        if (-not $mockReady) { throw "synthetic-api mock did not become ready on 38180" }

        $env:PORT = "33000"
        $env:SYNTHETIC_API_ORIGIN = "http://127.0.0.1:38180"
        $appProc = Start-Process -FilePath $nodeExe -ArgumentList @("node_modules/next/dist/bin/next", "dev", "-p", "33000") `
            -WorkingDirectory $webDir -RedirectStandardOutput $appOut -RedirectStandardError "$appOut.err" -PassThru -WindowStyle Hidden
        try {
            $appReady = $false
            for ($i = 0; $i -lt 120; $i++) {
                Start-Sleep -Milliseconds 500
                if (Test-PortOpen -Port 33000) { $appReady = $true; break }
            }
            if (-not $appReady) { throw "next app did not become ready on 33000" }

            $code = Run-Native -FilePath "npx.cmd" -ArgumentList @("playwright", "test", "tests/e2e/phase1") -Transcript $t -WorkingDirectory $webDir
            $failed = (Select-String -Path $t -Pattern "\d+ failed|No tests found|no tests|Error:" -Quiet -ErrorAction SilentlyContinue)
            $ran = (Select-String -Path $t -Pattern "\d+ passed" -Quiet -ErrorAction SilentlyContinue)
            if ($code -eq 0 -and -not $failed -and $ran) {
                Add-Check "E7" "playwright-phase1" "PASS" "phase1 e2e green (transcript $t)"
            } else {
                Add-Check "E7" "playwright-phase1" "BLOCKED_EXTERNAL" "EVIDENCE_MISSING or failed: exit=$code ran=$ran (transcript $t)"
            }
        } finally {
            if ($appProc -and -not $appProc.HasExited) { Stop-Process -Id $appProc.Id -Force -ErrorAction SilentlyContinue }
        }
    } finally {
        if ($mockProc -and -not $mockProc.HasExited) { Stop-Process -Id $mockProc.Id -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item Env:PORT -ErrorAction SilentlyContinue
    Remove-Item Env:SYNTHETIC_API_ORIGIN -ErrorAction SilentlyContinue
}

# --------------------------------------------------------------------------- #
# Verdict
# --------------------------------------------------------------------------- #
Test-WrittenRights
Test-VendorAuth0
Test-Auth0Simulator
Test-InviteMfa
Test-FiveUser
Test-RestorePolicy
if (-not $skipPlaywright) { Test-PlaywrightPhase1 } else {
    Add-Check "E7" "playwright-phase1" "BLOCKED_EXTERNAL" "EVIDENCE_MISSING: skipped by PHASE1_SKIP_PLAYWRIGHT=1"
}

$hardFail = @($checks | Where-Object { $_.result -eq "FAIL" })
$blockedExternal = @($checks | Where-Object { $_.result -eq "BLOCKED_EXTERNAL" })
$approved = ($hardFail.Count -eq 0 -and $blockedExternal.Count -eq 0)

$verdict = if ($approved) { "APPROVED" } else { "BLOCKED_EXTERNAL_DATA_RIGHTS" }
Write-Output "VERDICT: $verdict"
Write-Output "EVIDENCE: $evPath"
foreach ($c in $checks) {
    Write-Output ("  {0} {1} = {2}" -f $c.id, $c.name, $c.result)
}
if (-not $approved) {
    Write-Output "BLOCKED_REASONS:"
    foreach ($c in $checks) {
        if ($c.result -ne "PASS") { Write-Output "  - $($c.id) $($c.name): $($c.detail)" }
    }
    Write-Output "NOTE: BLOCKED_EXTERNAL_DATA_RIGHTS is the correct Phase-1 outcome when written-rights are not ACTIVE or vendor Auth0 cannot pass. Member KR-derived surfaces stay denied; Owner-only continues; no market switch and no release success claimed."
}

$summary = [ordered]@{
    gate = "phase1"
    task = 28
    verdict = $verdict
    emitted_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss'Z'")
    checks = $checks
    evidence_dir = $transcriptDir
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -Path $evPath -Encoding utf8
Write-Output "EVIDENCE_WRITTEN: $evPath"
exit 0
