#!/usr/bin/env pwsh
# validate-policy.ps1 - validate a backup set against the Lagrange Station backup policy
# (deploy/backup/policy/backup-policy.json + backup-manifest.schema.json) BEFORE any restore.
#
# The policy gate is mandatory: NO restore command (DB PITR, file restore, pre-Member drill,
# pre-Live restore) may start unless this validator exits 0 for the chosen gate.
#
# Exit codes:
#   0  POLICY OK     - every required DB/file class present, every sha256 present and matching,
#                      retention >= policy floor, storage/encryption rules honored, no secret
#                      marker in any archive file or the manifest itself. Restore may proceed.
#   1  POLICY REJECTED - one or more violations, each printed as VIOLATION[n] <field>: <reason>
#                      with the exact missing/rejected field. Restore MUST NOT start.
#   2  USAGE/LOAD ERROR - set path, manifest, or policy could not be loaded.
#
# Deterministic: identical input always yields byte-identical output (no wall clock used).
#
# Usage:
#   scripts/backup/validate-policy.ps1 -SetPath <backup-set-dir-or-manifest> [-Gate default|premember|prelive]
# Twin: scripts/backup/validate-policy.sh
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$SetPath,
    [string]$Gate = 'default',
    [string]$PolicyPath = '',
    [string]$SchemaPath = ''
)

$ErrorActionPreference = 'Stop'

# --- path resolution ---------------------------------------------------------
$scriptDir = Split-Path -Parent $PSScriptRoot          # scripts/backup
$root = Split-Path -Parent $scriptDir                  # repo root
if (-not $PolicyPath) { $PolicyPath = Join-Path $root 'deploy\backup\policy\backup-policy.json' }
if (-not $SchemaPath) { $SchemaPath = Join-Path $root 'deploy\backup\policy\backup-manifest.schema.json' }

function Fail-Usage([string]$Message) {
    Write-Error "USAGE: $Message"
    exit 2
}

# ConvertFrom-Json renders ISO-8601 strings as [datetime] on some pwsh versions; this
# normalizes any value back to its canonical UTC ISO-8601 string so checks are
# version-independent and deterministic.
function Get-IsoOrRaw([object]$Value) {
    if ($Value -is [datetime]) {
        return $Value.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
    }
    return [string]$Value
}

if (-not (Test-Path -LiteralPath $PolicyPath -PathType Leaf)) {
    Fail-Usage "policy file not found: $PolicyPath"
}
if (-not (Test-Path -LiteralPath $SchemaPath -PathType Leaf)) {
    Fail-Usage "manifest schema file not found: $SchemaPath"
}

$manifestPath = $null
if (Test-Path -LiteralPath $SetPath -PathType Container) {
    $candidate = Join-Path $SetPath 'backup-manifest.json'
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { $manifestPath = $candidate }
    else { Fail-Usage "no backup-manifest.json inside set directory: $SetPath" }
}
elseif (Test-Path -LiteralPath $SetPath -PathType Leaf) {
    $manifestPath = $SetPath
}
else {
    Fail-Usage "set path not found: $SetPath (pass a backup-set directory containing backup-manifest.json, or the manifest file itself)"
}
$setRoot = Split-Path -Parent $manifestPath

# --- load JSON ---------------------------------------------------------------
$manifest = $null
$policy = $null
try {
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $policy = Get-Content -Raw -LiteralPath $PolicyPath | ConvertFrom-Json
}
catch {
    Write-Error "LOAD ERROR: failed to parse manifest/policy JSON: $($_.Exception.Message)"
    exit 2
}

# --- gate validation ---------------------------------------------------------
if (-not ($policy.gates.PSObject.Properties.Name -contains $Gate)) {
    Fail-Usage "unknown gate '$Gate' (known: $($policy.gates.PSObject.Properties.Name -join ', '))"
}
$gateCfg = $policy.gates.$Gate

$violations = New-Object System.Collections.Generic.List[string]
function Add-Violation([string]$Field, [string]$Reason) {
    $violations.Add("VIOLATION[$($violations.Count + 1)] ${Field}: ${Reason}")
}

# --- manifest structure ------------------------------------------------------
$manifestVersion = if ($null -ne $manifest.manifest_version) { [string]$manifest.manifest_version } else { '' }
if ($manifestVersion -ne '1.0') {
    Add-Violation 'manifest.manifest_version' "expected '1.0', got '$manifestVersion'"
}
if ($null -eq $manifest.classes -or $manifest.classes.GetType().Name -ne 'Object[]') {
    Add-Violation 'manifest.classes' 'must be a non-empty JSON array of backup classes'
}
$classes = @($manifest.classes)

# --- required class presence (policy order) ----------------------------------
$requiredClasses = @($policy.required_classes)
$classIndex = @{}
for ($i = 0; $i -lt $classes.Count; $i++) {
    $cid = [string]$classes[$i].class
    if (-not $classIndex.ContainsKey($cid)) { $classIndex[$cid] = New-Object System.Collections.ArrayList }
    $classIndex[$cid].Add($i) | Out-Null
}
foreach ($req in $requiredClasses) {
    if (-not $classIndex.ContainsKey($req)) {
        Add-Violation 'classes[?].class' "required class '$req' is missing from the backup set"
    }
    elseif ($classIndex[$req].Count -gt 1) {
        Add-Violation "classes[$($classIndex[$req][0])].class" "duplicate class '$req'"
    }
}

# --- per-class checks (policy order) -----------------------------------------
$expectedKind = @{ db_base = 'db'; db_wal = 'db'; file_raw = 'file'; file_curated = 'file'; file_artifact = 'file' }
$expectedDataset = @{ file_raw = 'raw'; file_curated = 'curated'; file_artifact = 'artifact' }

$fileCount = 0
$classCount = 0
foreach ($req in $requiredClasses) {
    if (-not $classIndex.ContainsKey($req)) { continue }
    if ($classIndex[$req].Count -gt 1) { continue }
    $i = $classIndex[$req][0]
    $c = $classes[$i]
    $classCount++

    $kind = if ($null -ne $c.kind) { [string]$c.kind } else { '' }
    if ($kind -ne $expectedKind[$req]) {
        Add-Violation "classes[$i].kind" "class '$req' expects kind '$($expectedKind[$req])', got '$kind'"
    }
    if ($expectedDataset.ContainsKey($req)) {
        $ds = if ($null -ne $c.dataset) { [string]$c.dataset } else { '' }
        if ($ds -ne $expectedDataset[$req]) {
            Add-Violation "classes[$i].dataset" "class '$req' expects dataset '$($expectedDataset[$req])', got '$ds'"
        }
    }

    $rd = 0
    $rdOk = [int]::TryParse([string]$c.retention_days, [ref]$rd)
    if (-not $rdOk) {
        Add-Violation "classes[$i].retention_days" "must be a positive integer, got '$($c.retention_days)'"
    }
    else {
        $floor = [int]$policy.retention_days_min.$req
        if ($rd -lt $floor) {
            Add-Violation "classes[$i].retention_days" "declared $rd day(s) is below the policy floor of $floor day(s) for '$req'"
        }
        # expires_at must equal completed_at + retention_days when present (deterministic retention check)
        $completed = Get-IsoOrRaw $c.completed_at
        $expires = Get-IsoOrRaw $c.expires_at
        if ($completed -match '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$') {
            $cUtc = [datetime]::ParseExact($completed, "yyyy-MM-dd'T'HH:mm:ss'Z'", [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal)
            $expectedExpiry = $cUtc.AddDays($rd).ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
            if ($expires -ne '' -and $expires -ne $expectedExpiry) {
                Add-Violation "classes[$i].expires_at" "expected '$expectedExpiry' (completed_at + retention_days), got '$expires'"
            }
        }
        else {
            Add-Violation "classes[$i].completed_at" "not a valid UTC ISO-8601 timestamp: '$completed'"
        }
    }

    $enc = if ($null -ne $c.storage -and $null -ne $c.storage.encryption) { [string]$c.storage.encryption } else { '' }
    $requiredEnc = [string]$policy.storage_rules.$req.encryption
    if ($enc -eq 'none') {
        Add-Violation "classes[$i].storage.encryption" "encryption='none' is forbidden for every backup class"
    }
    elseif ($requiredEnc -eq 'required' -and $enc -ne 'required') {
        Add-Violation "classes[$i].storage.encryption" "class '$req' requires encryption='required', got '$enc'"
    }
    elseif ($enc -notin @('required', 'allowed')) {
        Add-Violation "classes[$i].storage.encryption" "must be 'required' or 'allowed', got '$enc'"
    }
    if ($null -eq $c.storage -or [string]::IsNullOrEmpty([string]$c.storage.location)) {
        Add-Violation "classes[$i].storage.location" 'storage location is required'
    }
    $refAllowed = [bool]$policy.storage_rules.$req.reference_allowed
    if ($null -ne $c.storage -and $refAllowed -eq $false -and [bool]$c.storage.reference) {
        Add-Violation "classes[$i].storage.reference" "reference storage is not allowed for class '$req'"
    }

    $files = @($c.files)
    if ($files.Count -lt 1) {
        Add-Violation "classes[$i].files" "class '$req' must declare at least one file"
        continue
    }

    for ($j = 0; $j -lt $files.Count; $j++) {
        $fileCount++
        $f = $files[$j]
        $p = [string]$f.path
        $h = [string]$f.sha256

        # path safety: relative to set root only - no '..', no leading / or drive, no backslash
        $unsafe = ($p -eq '') -or
            ($p.StartsWith('/')) -or ($p.StartsWith('\')) -or
            ($p -match '^[A-Za-z]:') -or ($p -match '\\') -or
            ($p -split '[\\/]' | Where-Object { $_ -eq '..' }).Count -gt 0
        if ($unsafe) {
            Add-Violation "classes[$i].files[$j].path" "unsafe path '$p' (must be relative to the set root, no '..', no absolute/drive/backslash paths)"
            continue
        }

        if (-not ($h -match '^[0-9a-fA-F]{64}$')) {
            Add-Violation "classes[$i].files[$j].sha256" "missing or malformed sha256 for '$p'"
        }

        $absPath = Join-Path $setRoot $p
        if (-not (Test-Path -LiteralPath $absPath -PathType Leaf)) {
            Add-Violation "classes[$i].files[$j].path" "file missing on disk: '$p'"
            continue
        }

        if ($h -match '^[0-9a-fA-F]{64}$') {
            $computed = (Get-FileHash -LiteralPath $absPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($computed -ne $h.ToLowerInvariant()) {
                Add-Violation "classes[$i].files[$j].sha256" "hash mismatch for '$p' (declared $($h.ToLowerInvariant()), computed $computed)"
            }
        }

        foreach ($seg in @($policy.forbidden_path_segments)) {
            if ($p.IndexOf([string]$seg, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                Add-Violation "classes[$i].files[$j].path" "forbidden path segment '$seg' in '$p'"
            }
        }

        # secret content scan (Latin-1 decode keeps every byte searchable, deterministic)
        $bytes = [System.IO.File]::ReadAllBytes($absPath)
        $text = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
        foreach ($marker in @($policy.secret_markers)) {
            if ($text.IndexOf([string]$marker, [StringComparison]::Ordinal) -ge 0) {
                Add-Violation "classes[$i].files[$j].content" "file contains secret marker '$marker' (path '$p')"
            }
        }
    }
}

# --- manifest self-scan --------------------------------------------------------
$manifestRaw = [System.IO.File]::ReadAllText($manifestPath, [System.Text.Encoding]::UTF8)
foreach ($marker in @($policy.secret_markers)) {
    if ($manifestRaw.IndexOf([string]$marker, [StringComparison]::Ordinal) -ge 0) {
        Add-Violation 'manifest' "manifest itself contains secret marker '$marker'"
    }
}

# --- restore policy / gate assertions -----------------------------------------
$rp = $manifest.restore_policy
if ($null -eq $rp -or $rp.isolated_target_required -ne $true) {
    Add-Violation 'restore_policy.isolated_target_required' 'must be true (isolated restore targets only)'
}
if ($Gate -eq 'prelive') {
    $preliveStartup = if ($null -ne $rp -and $null -ne $rp.assertions -and $null -ne $rp.assertions.prelive) { [string]$rp.assertions.prelive.startup_mode } else { '' }
    if ($preliveStartup -ne 'reconciliation_only') {
        Add-Violation 'restore_policy.assertions.prelive.startup_mode' "must be 'reconciliation_only' for the prelive gate, got '$preliveStartup'"
    }
}
foreach ($need in @($gateCfg.required_classes)) {
    if (-not $classIndex.ContainsKey([string]$need)) {
        Add-Violation "classes[?].class" "gate '$Gate' requires class '$need' which is missing from the backup set"
    }
}

# --- verdict ------------------------------------------------------------------
if ($violations.Count -gt 0) {
    foreach ($v in $violations) { $v }
    "POLICY REJECTED: $($violations.Count) violation(s)"
    exit 1
}
$setId = Get-IsoOrRaw $manifest.backup_set_id
"POLICY OK: backup set $setId valid for gate $Gate ($fileCount files, $classCount classes, 0 violations)"
exit 0
