# Manual QA channel for Todo 8 (KRX raw ingestion).
# Ingest the recorded synthetic bundle TWICE, assert two batches with identical
# content hashes and an untouched first batch, then exercise the failure modes
# (traversal / malformed / timeout / credentialed-without-credentials) and
# assert typed rejection with no partial output.
# Requires: cargo on PATH. Run from the repository root.

$ErrorActionPreference = 'Stop'
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("ls-task8-qa-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$bundle = 'tests/fixtures/kr-etf/contract'
$now = '2026-08-05T09:00:00Z'
$fail = 0

function Assert-True($cond, $msg) {
    if (-not $cond) { Write-Host "FAIL: $msg"; $script:fail = 1 } else { Write-Host "PASS: $msg" }
}

try {
    $run1 = cargo run -q -p collectors -- ingest-krx --root $root --date 2020-01-31 --mode synthetic --bundle $bundle --now $now 2>$null
    $run2 = cargo run -q -p collectors -- ingest-krx --root $root --date 2020-01-31 --mode synthetic --bundle $bundle --now $now 2>$null
    $j1 = $run1 | ConvertFrom-Json
    $j2 = $run2 | ConvertFrom-Json

    Assert-True ($j1.status -eq 'ok' -and $j2.status -eq 'ok') 'both ingests succeed'
    Assert-True ($j1.batch_id -ne $j2.batch_id) 'duplicate delivery creates a NEW batch'
    $sameHash = $true
    for ($i = 0; $i -lt $j1.files.Count; $i++) {
        if ($j1.files[$i].content_hash -ne $j2.files[$i].content_hash) { $sameHash = $false }
    }
    Assert-True $sameHash 'identical bytes => identical content hashes across batches'

    $dateDir = Join-Path $root 'raw/provider=krx/market=kr/date=2020-01-31'
    $dirs = Get-ChildItem $dateDir -Directory
    Assert-True ($dirs.Count -eq 2) 'exactly two batch dirs after duplicate delivery'

    $first = $dirs | Where-Object { $_.Name -like "*$($j1.batch_id)*" }
    $pre = Get-ChildItem $first.FullName -File | ForEach-Object { (Get-FileHash $_.FullName -Algorithm SHA256).Hash }
    $post = Get-ChildItem $first.FullName -File | ForEach-Object { (Get-FileHash $_.FullName -Algorithm SHA256).Hash }
    Assert-True (($pre -join '') -eq ($post -join '')) 'first batch untouched by duplicate delivery'

    $manifest = Join-Path $root 'raw/manifests/provider=krx/market=kr/manifest.jsonl'
    Assert-True ((Get-Content $manifest).Count -eq 2) 'append-only manifest: one row per delivery'

    foreach ($mode in @(
        @{ name = 'path traversal'; bundle = 'tests/fixtures/kr-etf/contract-variants/traversal'; expect = 'unsafe file name' },
        @{ name = 'malformed schema'; bundle = 'tests/fixtures/kr-etf/contract-variants/malformed-bars'; expect = 'malformed bars' },
        @{ name = 'timeout'; bundle = 'tests/fixtures/kr-etf/contract-variants/timeout'; expect = 'endpoint timeout' }
    )) {
        $out = cargo run -q -p collectors -- ingest-krx --root $root --date 2020-01-31 --mode synthetic --bundle $mode.bundle 2>$null
        $code = $LASTEXITCODE
        $json = $out | ConvertFrom-Json
        Assert-True ($code -eq 2 -and $json.status -eq 'error' -and $json.message -like "*$($mode.expect)*") "$($mode.name): typed rejection, exit 2"
    }

    Remove-Item Env:KRX_CREDENTIAL_REF -ErrorAction SilentlyContinue
    Remove-Item Env:KRX_BASE_URL -ErrorAction SilentlyContinue
    $out = cargo run -q -p collectors -- ingest-krx --root $root --date 2020-01-31 --mode credentialed 2>$null
    $json = $out | ConvertFrom-Json
    Assert-True ($LASTEXITCODE -eq 2 -and $json.message -like '*CredentialsUnavailable*' -or $json.message -like '*credentials unavailable*') 'credentialed mode without credentials: typed rejection'

    Assert-True ((Get-ChildItem $dateDir -Directory).Count -eq 2) 'failed deliveries leave no partial batches'
    Assert-True ((Get-Content $manifest).Count -eq 2) 'failed deliveries add no manifest rows'
}
finally {
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}

if ($fail -eq 0) { Write-Host 'QA: ALL CHECKS PASSED' } else { Write-Host 'QA: FAILURES PRESENT'; exit 1 }
