#!/usr/bin/env bash
# prune.sh - retention cleanup for stored backup sets (plan Todo 33).
# POSIX/CI twin of scripts/backup/prune.ps1.
#
# Deletes only sets in which EVERY class has passed its `expires_at`
# (`completed_at + retention_days`, the contract the validator recomputes). A
# set with even one live class is kept whole: classes inside a set are not
# independently restorable, so partially pruning one would leave an artifact
# that looks restorable and is not.
#
# Refuses to prune the newest surviving set even when expired. Retention policy
# exists to bound storage, not to arrive at zero backups; a cleanup that can
# empty the archive is a data-loss tool wearing a maintenance hat.
#
# Defaults to a DRY RUN. Deletion requires --apply.
#
# Exit codes:
#   0  completed (dry run or applied)
#   2  usage / environment error
#
# Usage:
#   scripts/backup/prune.sh --root <dir> [--now <UTC ts>] [--apply] [--keep-min N]
# Twin: scripts/backup/prune.ps1
set -u

root_dir=""
now=""
apply=0
keep_min=1

while [ $# -gt 0 ]; do
  case "$1" in
    --root) root_dir="$2"; shift 2 ;;
    --now) now="$2"; shift 2 ;;
    --apply) apply=1; shift ;;
    --keep-min) keep_min="$2"; shift 2 ;;
    *) echo "USAGE: $0 --root <dir> [--now <ts>] [--apply] [--keep-min N]" >&2; exit 2 ;;
  esac
done

[ -n "$root_dir" ] || { echo "USAGE: --root <dir> is required" >&2; exit 2; }
[ -d "$root_dir" ] || { echo "ENV ERROR: not a directory: $root_dir" >&2; exit 2; }
[ -n "$now" ] || now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Collect (manifest, latest_expiry, set_id) for every set under the root.
manifests="$(find "$root_dir" -type f -name backup-manifest.json | sort)"
[ -n "$manifests" ] || { echo "PRUNE: no backup sets under $root_dir"; exit 0; }

total=0
expired_list=""
while IFS= read -r m; do
  [ -n "$m" ] || continue
  total=$((total+1))
  line="$(python3 - "$m" "$now" <<'PYEOF'
import json, sys, datetime
d = json.load(open(sys.argv[1], encoding='utf-8'))
now = datetime.datetime.strptime(sys.argv[2], '%Y-%m-%dT%H:%M:%SZ')
def p(s): return datetime.datetime.strptime(s, '%Y-%m-%dT%H:%M:%SZ')
exps = [p(c['expires_at']) for c in d.get('classes', []) if c.get('expires_at')]
# A set is prunable only when its LONGEST-lived class has also expired.
latest = max(exps) if exps else None
created = d.get('created_at', '')
print('%s\t%s\t%s\t%s' % (
    d.get('backup_set_id', '?'),
    created,
    latest.strftime('%Y-%m-%dT%H:%M:%SZ') if latest else '',
    '1' if (latest is not None and latest < now) else '0'))
PYEOF
)" || continue
  set_id="$(printf '%s' "$line" | cut -f1)"
  created="$(printf '%s' "$line" | cut -f2)"
  latest="$(printf '%s' "$line" | cut -f3)"
  is_exp="$(printf '%s' "$line" | cut -f4)"
  sdir="$(dirname "$m")"
  if [ "$is_exp" = "1" ]; then
    expired_list="$expired_list$created|$set_id|$latest|$sdir
"
  else
    echo "KEEP    $set_id (newest class expires $latest)"
  fi
done <<< "$manifests"

surviving=$(( total - $(printf '%s' "$expired_list" | grep -c '|' || true) ))

if [ -z "$expired_list" ]; then
  echo "PRUNE: nothing expired as of $now ($total set(s) held)"
  exit 0
fi

# Newest-first, so "always keep the newest" is the first entry we protect.
sorted="$(printf '%s' "$expired_list" | grep '|' | sort -r)"
removed=0
kept_floor=0
while IFS= read -r row; do
  [ -n "$row" ] || continue
  created="$(printf '%s' "$row" | cut -d'|' -f1)"
  set_id="$(printf '%s' "$row" | cut -d'|' -f2)"
  latest="$(printf '%s' "$row" | cut -d'|' -f3)"
  sdir="$(printf '%s' "$row" | cut -d'|' -f4)"
  if [ "$(( surviving + kept_floor ))" -lt "$keep_min" ]; then
    kept_floor=$((kept_floor+1))
    echo "KEEP    $set_id (expired $latest, but retained: fewer than $keep_min set(s) would remain)"
    continue
  fi
  if [ "$apply" -eq 1 ]; then
    rm -rf "$sdir"
    echo "REMOVED $set_id (expired $latest) $sdir"
  else
    echo "WOULD   $set_id (expired $latest) $sdir"
  fi
  removed=$((removed+1))
done <<< "$sorted"

if [ "$apply" -eq 1 ]; then
  echo "PRUNE: removed $removed expired set(s); $(( total - removed )) remain"
else
  echo "PRUNE (dry run): $removed set(s) would be removed; re-run with --apply"
fi
exit 0
