#!/usr/bin/env bash
# restore-and-verify.sh - restore a backup set into an ISOLATED target and
# prove it (plan Todo 33). POSIX/CI twin of scripts/backup/restore-and-verify.ps1.
#
# Order of operations is the contract, not a convenience:
#   1. policy gate      - validate-policy.sh must exit 0. Nonzero => NO restore
#                         command runs at all (pre-member-restore-drill.md §1).
#   2. expiry           - the validator is deliberately clockless, so wall-clock
#                         expiry is checked here against --now.
#   3. stage            - decrypt + untar into an empty PGDATA with a
#                         recovery.signal targeting an explicit LSN.
#   4. recover          - start the target; PostgreSQL replays the WAL archive.
#   5. assert           - runbook checks P2-P4/P6 and A5-A7 inside the cluster.
#   6. file hashes      - every declared file class re-hashed (A3/P5).
#   7. verdict          - one machine-readable JSON; SUCCESS only if all pass.
#
# The restore target is ALWAYS torn down, on success and on failure. A drill
# that left a cluster running would be a production activation, which the plan
# forbids outright.
#
# Exit codes:
#   0  RESTORE VERIFIED
#   1  restore or an assertion failed (verdict JSON still written)
#   2  usage / environment error
#
# Usage:
#   scripts/backup/restore-and-verify.sh --set <dir> --sidecar <file>
#        [--gate default|premember|prelive] [--key <pass>] [--now <UTC ts>]
#        [--verdict <file.json>] [--metrics <file.prom>]
# Twin: scripts/backup/restore-and-verify.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose_file="$root/deploy/backup/compose/drill.compose.yml"
prepare="$root/scripts/backup/lib/restore-prepare-inside.sh"
verify="$root/scripts/backup/lib/restore-verify-inside.sh"
set_dir=""
sidecar=""
gate="default"
key="lagrange-drill-key"
key_file=""
now=""
verdict=""
metrics=""

while [ $# -gt 0 ]; do
  case "$1" in
    --set) set_dir="$2"; shift 2 ;;
    --sidecar) sidecar="$2"; shift 2 ;;
    --gate) gate="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    # Preferred for scheduled runs: a passphrase passed as an argument is
    # visible to every user in `ps`.
    --key-file) key_file="$2"; shift 2 ;;
    --now) now="$2"; shift 2 ;;
    --verdict) verdict="$2"; shift 2 ;;
    --metrics) metrics="$2"; shift 2 ;;
    *) echo "USAGE: $0 --set <dir> --sidecar <file> [--gate g] [--key k] [--now ts] [--verdict f] [--metrics f]" >&2; exit 2 ;;
  esac
done

[ -n "$set_dir" ] || { echo "USAGE: --set <dir> is required" >&2; exit 2; }
[ -n "$sidecar" ] || { echo "USAGE: --sidecar <file> is required" >&2; exit 2; }
[ -d "$set_dir" ] || { echo "ENV ERROR: backup set not found: $set_dir" >&2; exit 2; }
[ -f "$sidecar" ] || { echo "ENV ERROR: sidecar not found: $sidecar" >&2; exit 2; }
command -v docker >/dev/null 2>&1 || { echo "ENV ERROR: docker not found on PATH" >&2; exit 2; }
[ -n "$now" ] || now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if [ -n "$key_file" ]; then
  [ -f "$key_file" ] || { echo "ENV ERROR: key file not found: $key_file" >&2; exit 2; }
  key="$(tr -d '\r\n' < "$key_file")"
  [ -n "$key" ] || { echo "ENV ERROR: key file is empty: $key_file" >&2; exit 2; }
fi

hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
# Git Bash rewrites anything that looks like a POSIX path before a NATIVE
# docker.exe sees it, so a container path like /backup/set arrives as
# "C:/Program Files/Git/backup/set". Every docker call therefore goes through
# `dkr`, and every HOST path through hostpath(). The switches are never
# exported: doing so would also break the Windows python3 this script calls.
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
compose_file_host="$(hostpath "$compose_file")"

started_at="$(date -u +%s)"
status="FAILED"
failed_assertion=""
facts=""
project=""

json_field() {
  # json_field <file> <key> - reads a flat string/number field without jq.
  python3 -c "import json,sys;d=json.load(open(sys.argv[1],encoding='utf-8'));v=d.get(sys.argv[2],'');print(v)" "$1" "$2"
}

fail() {
  failed_assertion="$1"
  echo "RESTORE FAILED [$1]: $2" >&2
}

teardown() {
  if [ -n "$project" ]; then
    dkr compose -p "$project" -f "$compose_file_host" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
}

emit_verdict() {
  local duration=$(( $(date -u +%s) - started_at ))
  local body
  body=$(cat <<EOF
{
  "verdict": "$status",
  "gate": "$gate",
  "backup_set_id": "$(json_field "$sidecar" backup_set_id)",
  "evaluated_at": "$now",
  "duration_seconds": $duration,
  "failed_assertion": $( [ -n "$failed_assertion" ] && printf '"%s"' "$failed_assertion" || echo null ),
  "isolated_target": true,
  "target_left_running": false,
  "facts": {$facts
  }
}
EOF
)
  printf '%s\n' "$body"
  if [ -n "$verdict" ]; then
    mkdir -p "$(dirname "$verdict")"
    printf '%s\n' "$body" > "$verdict"
  fi
  if [ -n "$metrics" ]; then
    mkdir -p "$(dirname "$metrics")"
    cat > "$metrics" <<EOF
# HELP lagrange_restore_last_run_timestamp_seconds Unix time of the last restore drill.
# TYPE lagrange_restore_last_run_timestamp_seconds gauge
lagrange_restore_last_run_timestamp_seconds $(date -u +%s)
# HELP lagrange_restore_last_success_timestamp_seconds Unix time of the last VERIFIED restore drill.
# TYPE lagrange_restore_last_success_timestamp_seconds gauge
lagrange_restore_last_success_timestamp_seconds $([ "$status" = "SUCCESS" ] && date -u +%s || echo 0)
# HELP lagrange_restore_duration_seconds Wall-clock duration of the last restore drill.
# TYPE lagrange_restore_duration_seconds gauge
lagrange_restore_duration_seconds $duration
# HELP lagrange_restore_verified Whether the last restore drill verified (1) or failed (0).
# TYPE lagrange_restore_verified gauge
lagrange_restore_verified $([ "$status" = "SUCCESS" ] && echo 1 || echo 0)
EOF
  fi
}

finish() {
  teardown
  emit_verdict
}
trap finish EXIT

add_fact() {
  [ -z "$facts" ] && facts="
    \"$1\": $2" || facts="$facts,
    \"$1\": $2"
}

# --- 1. policy gate ------------------------------------------------------------
echo "== policy gate (no restore command runs unless this passes) =="
gate_out="$(bash "$root/scripts/backup/validate-policy.sh" --set "$set_dir" --gate "$gate" 2>&1)"
gate_rc=$?
printf '%s\n' "$gate_out"
add_fact policy_gate_exit "$gate_rc"
if [ "$gate_rc" -ne 0 ]; then
  fail "P1" "policy gate rejected the set (exit $gate_rc); no restore was attempted"
  exit 1
fi

# --- 2. expiry ------------------------------------------------------------------
# The validator enforces retention FLOORS but is deliberately clockless so its
# transcript is reproducible. Wall-clock expiry therefore belongs here, where
# --now makes it deterministic for tests.
echo "== retention expiry (as of $now) =="
expired="$(python3 - "$set_dir/backup-manifest.json" "$now" <<'PYEOF'
import json, sys, datetime
d = json.load(open(sys.argv[1], encoding='utf-8'))
now = datetime.datetime.strptime(sys.argv[2], '%Y-%m-%dT%H:%M:%SZ')
bad = []
for c in d.get('classes', []):
    exp = c.get('expires_at')
    if exp and datetime.datetime.strptime(exp, '%Y-%m-%dT%H:%M:%SZ') < now:
        bad.append('%s expired at %s' % (c.get('class'), exp))
print('; '.join(bad))
PYEOF
)"
if [ -n "$expired" ]; then
  add_fact expired_classes "\"$expired\""
  fail "RETENTION" "$expired"
  exit 1
fi
add_fact expired_classes null

target_lsn="$(json_field "$sidecar" recovery_target_lsn)"
expect_rows="$(json_field "$sidecar" pre_target_row_count)"
expect_prov="$(json_field "$sidecar" provenance_row_count)"
expect_dataset="$(json_field "$sidecar" dataset_version)"
expect_min_lsn="$(json_field "$sidecar" pre_target_lsn)"
add_fact recovery_target_lsn "\"$target_lsn\""

# --- 3/4. stage and recover into a disposable project --------------------------
project="lagrange-restore-$(date -u +%Y%m%d%H%M%S)-$$"
project="$(printf '%s' "$project" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9-' '-')"
echo "== isolated restore project: $project =="
add_fact restore_project "\"$project\""

dc() { dkr compose -p "$project" -f "$compose_file_host" "$@"; }

# Materialise the volumes, then push the set in BEFORE any postgres starts.
if ! dc up -d --wait --no-deps init-perms >/dev/null 2>&1; then
  dc run --rm --no-deps init-perms >/dev/null 2>&1 || true
fi
staging="$(dc run -d --no-deps --entrypoint sleep target 600 2>/dev/null | tail -n1 | tr -d '\r')"
if [ -z "$staging" ]; then
  fail "STAGE" "could not start a staging container for the restore target"
  exit 1
fi
if ! dkr cp "$(hostpath "$set_dir")" "$staging:/backup/set" >/dev/null 2>&1; then
  dkr rm -f "$staging" >/dev/null 2>&1 || true
  fail "STAGE" "could not copy the backup set into the restore target"
  exit 1
fi

echo "== staging the point-in-time recovery =="
stage_out="$(dkr exec -i -e BACKUP_KEY="$key" -e TARGET_LSN="$target_lsn" -e SET=/backup/set \
  "$staging" bash -s < "$prepare" 2>&1)"
stage_rc=$?
printf '%s\n' "$stage_out"
dkr rm -f "$staging" >/dev/null 2>&1 || true
if [ "$stage_rc" -ne 0 ]; then
  # A wrong key or a torn archive lands here, before PostgreSQL ever starts.
  case "$stage_out" in
    *"could not decrypt"*) fail "DECRYPT" "the db archives could not be decrypted (wrong key or corrupt)" ;;
    *) fail "STAGE" "staging the recovery failed" ;;
  esac
  exit 1
fi

echo "== recovering (replaying WAL to $target_lsn) =="
if ! dc up -d --wait target >/dev/null 2>&1; then
  echo "--- target logs ---" >&2
  dc logs --tail 40 target >&2 2>&1 || true
  fail "P2" "the restored cluster never finished recovery"
  exit 1
fi

# --- 5. runbook assertions inside the recovered cluster -------------------------
echo "== runbook assertions =="
assert_out="$(dc exec -T \
  -e TARGET_LSN="$target_lsn" -e EXPECT_ROWS_AT_TARGET="$expect_rows" \
  -e EXPECT_PROVENANCE="$expect_prov" -e EXPECT_DATASET="$expect_dataset" \
  -e EXPECT_MIN_LSN="$expect_min_lsn" \
  target bash -s < "$verify" 2>&1)"
assert_rc=$?
printf '%s\n' "$assert_out"

get_fact() { printf '%s\n' "$assert_out" | awk -F= -v k="$1" '$1==k{print $2}' | tail -n1 | tr -d '\r'; }
add_fact rows_at_target "$(get_fact ROWS_TOTAL)"
add_fact rows_after_target "$(get_fact ROWS_POST_TARGET)"
add_fact provenance_rows "$(get_fact PROVENANCE_ROWS)"
add_fact dataset_version "\"$(get_fact DATASET_VERSION)\""
add_fact secret_marker_hits "$(get_fact SECRET_MARKER_HITS)"

if [ "$assert_rc" -ne 0 ]; then
  first_fail="$(printf '%s\n' "$assert_out" | awk '/^ASSERT_FAIL/{print $2; exit}')"
  fail "${first_fail:-ASSERT}" "a runbook assertion failed"
  exit 1
fi

# --- 6. file-class hashes (A3 / P5) ---------------------------------------------
echo "== file-class hash comparison =="
mismatches="$(python3 - "$set_dir/backup-manifest.json" "$set_dir" <<'PYEOF'
import json, sys, hashlib, os
d = json.load(open(sys.argv[1], encoding='utf-8'))
root = sys.argv[2]
bad = []
n = 0
for c in d.get('classes', []):
    if c.get('kind') != 'file':
        continue
    for f in c.get('files', []):
        p = os.path.join(root, f['path'])
        n += 1
        if not os.path.isfile(p):
            bad.append('%s missing' % f['path']); continue
        h = hashlib.sha256(open(p, 'rb').read()).hexdigest()
        if h != f['sha256']:
            bad.append('%s declared %s computed %s' % (f['path'], f['sha256'], h))
print('%d' % n)
print('; '.join(bad))
PYEOF
)"
file_n="$(printf '%s\n' "$mismatches" | head -n1)"
file_bad="$(printf '%s\n' "$mismatches" | tail -n +2)"
add_fact file_classes_checked "${file_n:-0}"
if [ -n "$file_bad" ]; then
  add_fact file_hash_mismatches "\"$file_bad\""
  fail "A3" "$file_bad"
  exit 1
fi
add_fact file_hash_mismatches null

status="SUCCESS"
echo "RESTORE VERIFIED: $(json_field "$sidecar" backup_set_id) at LSN $target_lsn"
exit 0
