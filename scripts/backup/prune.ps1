<#
.SYNOPSIS
  Retention cleanup for stored backup sets (plan Todo 33).
  PowerShell twin of scripts/backup/prune.sh.

.DESCRIPTION
  Deletes only sets in which EVERY class has passed its `expires_at`
  (`completed_at + retention_days`, the contract the validator recomputes). A
  set with even one live class is kept whole: classes inside a set are not
  independently restorable, so partially pruning one would leave an artifact
  that looks restorable and is not.

  Refuses to prune below -KeepMin surviving sets even when everything is
  expired. Retention exists to bound storage, not to arrive at zero backups; a
  cleanup that can empty the archive is a data-loss tool wearing a maintenance
  hat.

  Defaults to a DRY RUN. Deletion requires -Apply.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Root,
    [string]$Now,
    [switch]$Apply,
    [int]$KeepMin = 1
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -PathType Container $Root)) { Write-Error "ENV ERROR: not a directory: $Root"; exit 2 }
if (-not $Now) { $Now = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }

$utcFmt = 'yyyy-MM-dd\THH:mm:ss\Z'
$nowDt = [datetime]::ParseExact($Now, $utcFmt, [Globalization.CultureInfo]::InvariantCulture)

function ConvertTo-Utc {
    param($Value)
    # ConvertFrom-Json may already have coerced an ISO-8601 string to [datetime].
    if ($Value -is [datetime]) { return $Value }
    [datetime]::ParseExact([string]$Value, $utcFmt, [Globalization.CultureInfo]::InvariantCulture)
}

$manifests = Get-ChildItem -Recurse -File -Filter 'backup-manifest.json' -Path $Root | Sort-Object FullName
if (-not $manifests) { Write-Host "PRUNE: no backup sets under $Root"; exit 0 }

$sets = @()
foreach ($m in $manifests) {
    $d = Get-Content -Raw $m.FullName | ConvertFrom-Json
    $exps = @($d.classes | Where-Object { $_.expires_at } | ForEach-Object { ConvertTo-Utc $_.expires_at })
    # A set is prunable only when its LONGEST-lived class has also expired.
    $latest = if ($exps.Count -gt 0) { ($exps | Sort-Object -Descending)[0] } else { $null }
    $sets += [pscustomobject]@{
        SetId   = $d.backup_set_id
        Created = $d.created_at
        Latest  = $latest
        Dir     = $m.DirectoryName
        Expired = ($null -ne $latest -and $latest -lt $nowDt)
    }
}

$total = $sets.Count
$live = @($sets | Where-Object { -not $_.Expired })
foreach ($s in $live) {
    Write-Host "KEEP    $($s.SetId) (newest class expires $($s.Latest.ToString($utcFmt)))"
}

$expired = @($sets | Where-Object { $_.Expired } | Sort-Object Created -Descending)
if ($expired.Count -eq 0) {
    Write-Host "PRUNE: nothing expired as of $Now ($total set(s) held)"
    exit 0
}

$surviving = $live.Count
$removed = 0
$keptFloor = 0
foreach ($s in $expired) {
    if (($surviving + $keptFloor) -lt $KeepMin) {
        $keptFloor++
        Write-Host "KEEP    $($s.SetId) (expired $($s.Latest.ToString($utcFmt)), but retained: fewer than $KeepMin set(s) would remain)"
        continue
    }
    if ($Apply) {
        Remove-Item -Recurse -Force $s.Dir
        Write-Host "REMOVED $($s.SetId) (expired $($s.Latest.ToString($utcFmt))) $($s.Dir)"
    } else {
        Write-Host "WOULD   $($s.SetId) (expired $($s.Latest.ToString($utcFmt))) $($s.Dir)"
    }
    $removed++
}

if ($Apply) {
    Write-Host "PRUNE: removed $removed expired set(s); $($total - $removed) remain"
} else {
    Write-Host "PRUNE (dry run): $removed set(s) would be removed; re-run with -Apply"
}
exit 0
