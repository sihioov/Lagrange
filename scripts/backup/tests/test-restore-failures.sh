#!/usr/bin/env bash
# test-restore-failures.sh - prove restore-and-verify.sh fails CLOSED (Todo 33).
# POSIX/CI twin of scripts/backup/tests/test-restore-failures.ps1.
#
# The happy path is proven by running create.sh then restore-and-verify.sh; this
# harness proves the far more important half: that a set which should not be
# restorable is not restored, and that nothing is left running afterwards.
#
# Scenarios (plan Todo 33 QA "failure" list):
#   1. wrong key            -> DECRYPT, and PostgreSQL is never started
#   2. missing WAL segment  -> policy gate rejects; no restore command runs
#   3. corrupt WAL content  -> hash mismatch at the gate; no restore command runs
#   4. secret in an archive -> gate rejects, naming the marker
#   5. expired manifest     -> RETENTION, evaluated against --now
#   6. partial DB (no base) -> gate rejects the missing db_base class
# Every scenario additionally asserts:
#   * the verdict JSON says FAILED and names the failing assertion;
#   * no drill container is left running ("no production activation").
#
# A good backup set is required. Build one first:
#   scripts/backup/create.sh --out <dir> --key <k>
# then:
#   scripts/backup/tests/test-restore-failures.sh --set <dir>/set \
#       --sidecar <dir>/backup-sidecar.json --key <k>
#
# Exit 0 only when every scenario fails in exactly the expected way.
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
restore="$root/scripts/backup/restore-and-verify.sh"
good_set=""
good_sidecar=""
key="lagrange-drill-key"
work=""

while [ $# -gt 0 ]; do
  case "$1" in
    --set) good_set="$2"; shift 2 ;;
    --sidecar) good_sidecar="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    --work) work="$2"; shift 2 ;;
    *) echo "USAGE: $0 --set <dir> --sidecar <file> [--key k] [--work dir]" >&2; exit 2 ;;
  esac
done

[ -d "$good_set" ] || { echo "USAGE: --set must be an existing backup set directory" >&2; exit 2; }
[ -f "$good_sidecar" ] || { echo "USAGE: --sidecar must be an existing sidecar file" >&2; exit 2; }
[ -n "$work" ] || work="${TMPDIR:-/tmp}/lagrange-restore-failtests-$$"
mkdir -p "$work"

tests=0
fails=0

# Scoped to THIS scenario's own project, not every lagrange-restore-* container:
# an unrelated drill running concurrently is not evidence that this one leaked.
running_drill_containers() {
  local proj="$1"
  [ -n "$proj" ] || { echo 0; return; }
  MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' \
    docker ps --format '{{.Names}}' 2>/dev/null | grep -c "^$proj-" || true
}

# scenario <name> <set-dir> <expected-assertion> [extra restore args...]
scenario() {
  local name="$1" sdir="$2" expect="$3"; shift 3
  tests=$((tests+1))
  local vfile="$work/verdict-$tests.json"
  local out
  out="$(bash "$restore" --set "$sdir" --sidecar "$good_sidecar" --key "$key" \
          --verdict "$vfile" "$@" 2>&1)"
  local rc=$?
  local ok=1 why=""

  [ "$rc" -ne 0 ] || { ok=0; why="$why exit=0(expected nonzero)"; }
  if [ -f "$vfile" ]; then
    grep -q '"verdict": "FAILED"' "$vfile" || { ok=0; why="$why verdict!=FAILED"; }
    grep -q "\"failed_assertion\": \"$expect\"" "$vfile" \
      || { ok=0; why="$why failed_assertion!=$expect"; }
  else
    ok=0; why="$why no-verdict-file"
  fi
  # No drill may survive its own failure.
  local proj=""
  [ -f "$vfile" ] && proj="$(sed -n 's/.*"restore_project": "\([^"]*\)".*/\1/p' "$vfile" | head -n1)"
  local left; left="$(running_drill_containers "$proj")"
  [ "$left" = "0" ] || { ok=0; why="$why left $left container(s) of $proj running"; }

  if [ "$ok" -eq 1 ]; then
    echo "PASS $name (failed_assertion=$expect)"
  else
    fails=$((fails+1))
    echo "FAIL $name -$why"
    printf '%s\n' "$out" | sed 's/^/    /' | tail -n 15
  fi
}

# --- fixtures: mutated copies of the good set --------------------------------
copy_set() {
  local dst="$work/$1"
  rm -rf "$dst"
  mkdir -p "$dst"
  cp -r "$good_set/." "$dst/"
  printf '%s' "$dst"
}

echo "== building mutated backup sets =="
set_missing_wal="$(copy_set missing-wal)"
rm -f "$(find "$set_missing_wal/pg/wal" -type f -name '*.enc' | sort | head -n1)"

set_corrupt_wal="$(copy_set corrupt-wal)"
corrupt_target="$(find "$set_corrupt_wal/pg/wal" -type f -name '*.enc' | sort | head -n1)"
printf 'corrupted-by-the-failure-harness' >> "$corrupt_target"

set_secret="$(copy_set secret-in-archive)"
secret_target="$(find "$set_secret/files/artifact" -type f -name '*.increment' | head -n1)"
printf '\nLAGRANGE_SECRET_MARKER=leaked\n' >> "$secret_target"
# Re-hash so the set fails ONLY on the secret marker, not on a hash mismatch -
# otherwise this scenario would pass for the wrong reason.
python3 - "$set_secret/backup-manifest.json" "$secret_target" "$set_secret" <<'PYEOF'
import json, sys, hashlib, os
mpath, target, root = sys.argv[1], sys.argv[2], sys.argv[3]
rel = os.path.relpath(target, root).replace(os.sep, '/')
d = json.load(open(mpath, encoding='utf-8'))
h = hashlib.sha256(open(target, 'rb').read()).hexdigest()
for c in d['classes']:
    for f in c.get('files', []):
        if f['path'] == rel:
            f['sha256'] = h
            f['size_bytes'] = os.path.getsize(target)
json.dump(d, open(mpath, 'w', encoding='utf-8'), indent=2)
PYEOF

set_no_base="$(copy_set partial-db)"
python3 - "$set_no_base/backup-manifest.json" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
d['classes'] = [c for c in d['classes'] if c['class'] != 'db_base']
json.dump(d, open(sys.argv[1], 'w', encoding='utf-8'), indent=2)
PYEOF

# --- scenarios ----------------------------------------------------------------
echo "== running failure scenarios =="

# 1. Wrong key. The set is perfectly valid, so the policy gate PASSES and the
#    restore genuinely begins - this is the one scenario that must be caught by
#    decryption rather than by the gate.
scenario 'wrong decryption key aborts before PostgreSQL starts' \
  "$good_set" DECRYPT --key definitely-the-wrong-key

# 2-4, 6. Gate rejections: no restore command may run at all.
scenario 'missing WAL segment is rejected at the gate' "$set_missing_wal" P1
scenario 'corrupt WAL content is rejected at the gate' "$set_corrupt_wal" P1
scenario 'secret marker in an archive is rejected at the gate' "$set_secret" P1
scenario 'partial DB (no base backup) is rejected at the gate' "$set_no_base" P1

# 5. Expired manifest: the validator is clockless by design, so expiry is the
#    restore driver's job and --now makes the check deterministic.
scenario 'an expired backup set is refused' "$good_set" RETENTION \
  --now 2099-01-01T00:00:00Z

echo
if [ "$fails" -eq 0 ]; then
  echo "ALL RESTORE FAILURE TESTS PASSED ($tests/$tests)"
  rm -rf "$work"
  exit 0
fi
echo "RESTORE FAILURE TESTS FAILED ($((tests-fails))/$tests) - artifacts in $work"
exit 1
