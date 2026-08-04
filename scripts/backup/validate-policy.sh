#!/usr/bin/env bash
# validate-policy.sh - POSIX/CI twin of scripts/backup/validate-policy.ps1.
# Validates a backup set against the Lagrange Station backup policy
# (deploy/backup/policy/backup-policy.json + backup-manifest.schema.json) BEFORE any restore.
#
# The policy gate is mandatory: NO restore command (DB PITR, file restore, pre-Member drill,
# pre-Live restore) may start unless this validator exits 0 for the chosen gate.
#
# Exit codes:
#   0  POLICY OK        - every required DB/file class present, every sha256 present and matching,
#                         retention >= policy floor, storage/encryption rules honored, no secret
#                         marker in any archive file or the manifest itself. Restore may proceed.
#   1  POLICY REJECTED  - one or more violations, each printed as VIOLATION[n] <field>: <reason>
#                         with the exact missing/rejected field. Restore MUST NOT start.
#   2  USAGE/LOAD ERROR - set path, manifest, or policy could not be loaded.
#
# Deterministic: identical input always yields byte-identical output (no wall clock used).
# Requires: bash, python3 (JSON parsing), sha256sum (hash comparison) - all present in the
# repo's WSL2/CI shell. No jq needed.
#
# Usage:
#   scripts/backup/validate-policy.sh --set <backup-set-dir-or-manifest> [--gate default|premember|prelive]
# Twin: scripts/backup/validate-policy.ps1
set -u
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
policy="$root/deploy/backup/policy/backup-policy.json"
schema="$root/deploy/backup/policy/backup-manifest.schema.json"
gate="default"
set_path=""

while [ $# -gt 0 ]; do
  case "$1" in
    --set) set_path="$2"; shift 2 ;;
    --gate) gate="$2"; shift 2 ;;
    --policy) policy="$2"; shift 2 ;;
    --schema) schema="$2"; shift 2 ;;
    *) echo "USAGE: $0 --set <path> [--gate default|premember|prelive]" >&2; exit 2 ;;
  esac
done

violations=0

violate() {
  # violate <field> <reason>
  violations=$((violations+1))
  echo "VIOLATION[$violations] $1: $2"
}

fail_usage() {
  echo "USAGE: $1" >&2
  exit 2
}

[ -f "$policy" ]  || fail_usage "policy file not found: $policy"
[ -f "$schema" ]  || fail_usage "manifest schema file not found: $schema"
[ -n "$set_path" ] || fail_usage "--set <path> is required (backup-set directory containing backup-manifest.json, or the manifest file itself)"

if [ -d "$set_path" ]; then
  manifest="$set_path/backup-manifest.json"
  [ -f "$manifest" ] || fail_usage "no backup-manifest.json inside set directory: $set_path"
elif [ -f "$set_path" ]; then
  manifest="$set_path"
else
  fail_usage "set path not found: $set_path"
fi
set_root="$(cd "$(dirname "$manifest")" && pwd)"

# --- JSON dump helpers (python3; outputs deterministic fact lines) ------------
dump_policy() {
  python3 - "$policy" <<'PYEOF'
import json, sys
p = json.load(open(sys.argv[1], encoding='utf-8'))
for c in p.get('required_classes', []) or []:
    print('required_class\t%s' % c)
for c, days in (p.get('retention_days_min', {}) or {}).items():
    print('retention_min\t%s\t%s' % (c, days))
for c, rules in (p.get('storage_rules', {}) or {}).items():
    print('storage_rule\t%s\t%s\t%s' % (c, rules.get('encryption',''), '1' if rules.get('reference_allowed') else '0'))
for m in p.get('secret_markers', []) or []:
    print('secret_marker\t%s' % m)
for seg in p.get('forbidden_path_segments', []) or []:
    print('forbidden_segment\t%s' % seg)
for g, cfg in (p.get('gates', {}) or {}).items():
    print('gate\t%s\t%s\t%s' % (g, ','.join(cfg.get('required_classes', []) or []), cfg.get('startup_mode','') or ''))
PYEOF
}

dump_manifest() {
  python3 - "$manifest" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
print('manifest_version\t%s' % d.get('manifest_version',''))
print('backup_set_id\t%s' % d.get('backup_set_id',''))
for i, c in enumerate(d.get('classes', []) or []):
    st = c.get('storage') or {}
    print('class\t%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s' % (
        i, c.get('class',''), c.get('kind',''), c.get('dataset',''),
        c.get('retention_days',''), c.get('completed_at',''), c.get('expires_at',''),
        st.get('encryption',''), '1' if st.get('reference') else '0', st.get('location',''),
        len(c.get('files', []) or []), c.get('backup_id','')))
for i, c in enumerate(d.get('classes', []) or []):
    for j, f in enumerate(c.get('files', []) or []):
        print('file\t%d\t%d\t%s\t%s' % (i, j, f.get('path',''), f.get('sha256','')))
rp = d.get('restore_policy') or {}
print('isolated_target\t%s' % ('true' if rp.get('isolated_target_required') else 'false'))
prelive = ((rp.get('assertions') or {}).get('prelive') or {})
print('prelive_startup\t%s' % prelive.get('startup_mode',''))
PYEOF
}

# --- load policy facts ---------------------------------------------------------
policy_lines="$(dump_policy)" || { echo "LOAD ERROR: failed to parse policy JSON: $policy" >&2; exit 2; }
manifest_lines="$(dump_manifest)" || { echo "LOAD ERROR: failed to parse manifest JSON: $manifest" >&2; exit 2; }

required_classes=$(printf '%s\n' "$policy_lines" | awk -F'\t' '$1=="required_class"{print $2}')
# gate facts
gate_required=$(printf '%s\n' "$policy_lines" | awk -F'\t' -v g="$gate" '$1=="gate" && $2==g {print $3}')
gate_startup=$(printf '%s\n' "$policy_lines" | awk -F'\t' -v g="$gate" '$1=="gate" && $2==g {print $4}')
if [ -z "$gate_required" ]; then
  fail_usage "unknown gate '$gate'"
fi
# storage rules: <class> <encryption> <reference_allowed>
# retention floors: <class> <days>

# --- manifest structure ---------------------------------------------------------
manifest_version=$(printf '%s\n' "$manifest_lines" | awk -F'\t' '$1=="manifest_version"{print $2}')
backup_set_id=$(printf '%s\n' "$manifest_lines" | awk -F'\t' '$1=="backup_set_id"{print $2}')
[ "$manifest_version" = "1.0" ] || violate 'manifest.manifest_version' "expected '1.0', got '$manifest_version'"

# class lines: idx class kind dataset retention_days completed_at expires_at encryption reference location nfiles backup_id
class_count=$(printf '%s\n' "$manifest_lines" | awk -F'\t' '$1=="class"' | wc -l | tr -d ' ')
file_count=0

# --- required class presence (policy order) ---------------------------------------
while IFS= read -r req; do
  [ -n "$req" ] || continue
  n=$(printf '%s\n' "$manifest_lines" | awk -F'\t' -v c="$req" '$1=="class" && $3==c' | wc -l | tr -d ' ')
  if [ "$n" -eq 0 ]; then
    violate 'classes[?].class' "required class '$req' is missing from the backup set"
  elif [ "$n" -gt 1 ]; then
    violate 'classes[?].class' "duplicate class '$req'"
  fi
done <<< "$required_classes"

# --- per-class checks (policy order) ---------------------------------------------
while IFS= read -r req; do
  [ -n "$req" ] || continue
  cline=$(printf '%s\n' "$manifest_lines" | awk -F'\t' -v c="$req" '$1=="class" && $3==c' | head -n1)
  [ -n "$cline" ] || continue
  ci=$(printf '%s' "$cline" | awk -F'\t' '{print $2}')
  cid=$(printf '%s' "$cline" | awk -F'\t' '{print $3}')
  kind=$(printf '%s' "$cline" | awk -F'\t' '{print $4}')
  ds=$(printf '%s' "$cline" | awk -F'\t' '{print $5}')
  rd=$(printf '%s' "$cline" | awk -F'\t' '{print $6}')
  completed=$(printf '%s' "$cline" | awk -F'\t' '{print $7}')
  expires=$(printf '%s' "$cline" | awk -F'\t' '{print $8}')
  enc=$(printf '%s' "$cline" | awk -F'\t' '{print $9}')
  ref=$(printf '%s' "$cline" | awk -F'\t' '{print $10}')
  loc=$(printf '%s' "$cline" | awk -F'\t' '{print $11}')
  nfiles=$(printf '%s' "$cline" | awk -F'\t' '{print $12}')

  case "$req" in
    db_base|db_wal) exp_kind=db; exp_ds='' ;;
    file_raw|file_curated|file_artifact) exp_kind=file ;;
  esac
  case "$req" in
    file_raw) exp_ds=raw ;;
    file_curated) exp_ds=curated ;;
    file_artifact) exp_ds=artifact ;;
    *) exp_ds='' ;;
  esac
  [ "$kind" = "$exp_kind" ] || violate "classes[$ci].kind" "class '$req' expects kind '$exp_kind', got '$kind'"
  if [ -n "$exp_ds" ]; then
    [ "$ds" = "$exp_ds" ] || violate "classes[$ci].dataset" "class '$req' expects dataset '$exp_ds', got '$ds'"
  fi

  if ! printf '%s' "$rd" | grep -Eq '^[1-9][0-9]*$'; then
    violate "classes[$ci].retention_days" "must be a positive integer, got '$rd'"
  else
    floor=$(printf '%s\n' "$policy_lines" | awk -F'\t' -v c="$req" '$1=="retention_min" && $2==c {print $3}')
    if [ -n "$floor" ] && [ "$rd" -lt "$floor" ]; then
      violate "classes[$ci].retention_days" "declared $rd day(s) is below the policy floor of $floor day(s) for '$req'"
    fi
    if printf '%s' "$completed" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'; then
      expected_expiry=$(python3 - "$completed" "$rd" <<'PYEOF'
import datetime, sys
c = datetime.datetime.strptime(sys.argv[1], '%Y-%m-%dT%H:%M:%SZ')
print((c + datetime.timedelta(days=int(sys.argv[2]))).strftime('%Y-%m-%dT%H:%M:%SZ'))
PYEOF
)
      if [ -n "$expires" ] && [ "$expires" != "$expected_expiry" ]; then
        violate "classes[$ci].expires_at" "expected '$expected_expiry' (completed_at + retention_days), got '$expires'"
      fi
    else
      violate "classes[$ci].completed_at" "not a valid UTC ISO-8601 timestamp: '$completed'"
    fi
  fi

  required_enc=$(printf '%s\n' "$policy_lines" | awk -F'\t' -v c="$req" '$1=="storage_rule" && $2==c {print $3}')
  ref_allowed=$(printf '%s\n' "$policy_lines" | awk -F'\t' -v c="$req" '$1=="storage_rule" && $2==c {print $4}')
  if [ "$enc" = "none" ]; then
    violate "classes[$ci].storage.encryption" "encryption='none' is forbidden for every backup class"
  elif [ "$required_enc" = "required" ] && [ "$enc" != "required" ]; then
    violate "classes[$ci].storage.encryption" "class '$req' requires encryption='required', got '$enc'"
  elif [ "$enc" != "required" ] && [ "$enc" != "allowed" ]; then
    violate "classes[$ci].storage.encryption" "must be 'required' or 'allowed', got '$enc'"
  fi
  [ -n "$loc" ] || violate "classes[$ci].storage.location" 'storage location is required'
  if [ "$ref_allowed" = "0" ] && [ "$ref" = "1" ]; then
    violate "classes[$ci].storage.reference" "reference storage is not allowed for class '$req'"
  fi

  if [ "$nfiles" -lt 1 ]; then
    violate "classes[$ci].files" "class '$req' must declare at least one file"
    continue
  fi

  while IFS=$'\t' read -r fk cidx fi path h; do
    file_count=$((file_count+1))
    unsafe=0
    [ -n "$path" ] || unsafe=1
    case "$path" in /*) unsafe=1;; esac
    case "$path" in \\*) unsafe=1;; esac
    printf '%s' "$path" | grep -Eq '^[A-Za-z]:' && unsafe=1
    printf '%s' "$path" | grep -Fq '\\' && unsafe=1
    printf '%s' "$path" | tr '/\\' '\n' | grep -Fxq '..' && unsafe=1
    if [ "$unsafe" -eq 1 ]; then
      violate "classes[$ci].files[$fi].path" "unsafe path '$path' (must be relative to the set root, no '..', no absolute/drive/backslash paths)"
      continue
    fi

    if ! printf '%s' "$h" | grep -Eq '^[0-9a-fA-F]{64}$'; then
      violate "classes[$ci].files[$fi].sha256" "missing or malformed sha256 for '$path'"
    fi

    abs_path="$set_root/$path"
    if [ ! -f "$abs_path" ]; then
      violate "classes[$ci].files[$fi].path" "file missing on disk: '$path'"
      continue
    fi

    if printf '%s' "$h" | grep -Eq '^[0-9a-fA-F]{64}$'; then
      computed=$(sha256sum "$abs_path" | cut -d' ' -f1)
      [ "$computed" = "$h" ] || violate "classes[$ci].files[$fi].sha256" "hash mismatch for '$path' (declared $h, computed $computed)"
    fi

    while IFS= read -r seg; do
      [ -n "$seg" ] || continue
      if printf '%s' "$path" | grep -qiF "$seg"; then
        violate "classes[$ci].files[$fi].path" "forbidden path segment '$seg' in '$path'"
      fi
    done <<< "$(printf '%s\n' "$policy_lines" | awk -F'\t' '$1=="forbidden_segment"{print $2}')"

    while IFS= read -r marker; do
      [ -n "$marker" ] || continue
      if grep -a -Fq "$marker" "$abs_path"; then
        violate "classes[$ci].files[$fi].content" "file contains secret marker '$marker' (path '$path')"
      fi
    done <<< "$(printf '%s\n' "$policy_lines" | awk -F'\t' '$1=="secret_marker"{print $2}')"
  done < <(printf '%s\n' "$manifest_lines" | awk -F'\t' -v ci="$ci" '$1=="file" && $2==ci')
done <<< "$required_classes"

# --- manifest self-scan ------------------------------------------------------------
while IFS= read -r marker; do
  [ -n "$marker" ] || continue
  if grep -a -Fq "$marker" "$manifest"; then
    violate 'manifest' "manifest itself contains secret marker '$marker'"
  fi
done <<< "$(printf '%s\n' "$policy_lines" | awk -F'\t' '$1=="secret_marker"{print $2}')"

# --- restore policy / gate assertions ----------------------------------------------
isolated_target=$(printf '%s\n' "$manifest_lines" | awk -F'\t' '$1=="isolated_target"{print $2}')
[ "$isolated_target" = "true" ] || violate 'restore_policy.isolated_target_required' 'must be true (isolated restore targets only)'
if [ "$gate" = "prelive" ]; then
  prelive_startup=$(printf '%s\n' "$manifest_lines" | awk -F'\t' '$1=="prelive_startup"{print $2}')
  [ "$prelive_startup" = "reconciliation_only" ] || violate 'restore_policy.assertions.prelive.startup_mode' "must be 'reconciliation_only' for the prelive gate, got '$prelive_startup'"
fi
IFS=',' read -r -a gate_needs <<< "$gate_required"
for need in "${gate_needs[@]}"; do
  [ -n "$need" ] || continue
  n=$(printf '%s\n' "$manifest_lines" | awk -F'\t' -v c="$need" '$1=="class" && $3==c' | wc -l | tr -d ' ')
  if [ "$n" -eq 0 ]; then
    violate 'classes[?].class' "gate '$gate' requires class '$need' which is missing from the backup set"
  fi
done

# --- verdict ------------------------------------------------------------------------
if [ "$violations" -gt 0 ]; then
  echo "POLICY REJECTED: $violations violation(s)"
  exit 1
fi
echo "POLICY OK: backup set $backup_set_id valid for gate $gate ($file_count files, $class_count classes, 0 violations)"
exit 0
