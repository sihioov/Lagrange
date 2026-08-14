#!/usr/bin/env bash
# test-validate-policy.sh - red-first acceptance harness for scripts/backup/validate-policy.sh.
# POSIX twin of scripts/backup/tests/test-validate-policy.ps1. Proves, with machine-checked
# assertions on real validator transcripts:
#   1. a synthetic COMPLETE backup set validates (exit 0) - all DB/file classes, hashes,
#      retention, storage rules, and secret exclusions confirmed;
#   2. an incomplete manifest (missing db_wal class) is REJECTED before any restore can start;
#   3. a manifest missing an artifact sha256 is REJECTED, naming the missing field;
#   4. a tampered base-backup hash is REJECTED, naming file + declared vs computed hash;
#   5. an archive containing a fake secret marker is REJECTED, naming marker and file;
#   6. the validator is deterministic: identical input produces byte-identical output.
# Every rejection happens at the policy gate (validate-policy exit != 0) BEFORE any restore
# command - this harness never starts a restore, only the gate.
# Requires: bash, sha256sum, python3 (JSON parsing) - all present in the repo's WSL2/CI shell.
# Exit 0 only when all assertions hold.
set -u
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
validator="$root/scripts/backup/validate-policy.sh"
fixtures="$root/scripts/backup/tests/fixtures"
tests=0
fails=0

# The repository intentionally ignores every `secrets/` directory. Build the
# secret-bearing fixture at runtime so a clean clone can still exercise the
# marker scanner without committing a file that looks like a real secret.
fake_secret_fixture="$(mktemp -d)"
trap 'rm -rf -- "$fake_secret_fixture"' EXIT
cp -R "$fixtures/fake-secret/." "$fake_secret_fixture/"
mkdir -p "$fake_secret_fixture/secrets"
printf 'LAGRANGE_SECRET_MARKER=leaked\n' > "$fake_secret_fixture/secrets/kis-app-secret.plaintext"

run_validator() {
  # usage: run_validator <fixture> ; sets rc / out
  local fixture="$1"
  if [ ! -f "$validator" ]; then
    rc=127
    out="validate-policy.sh not found: $validator"
    return
  fi
  out="$(bash "$validator" --set "$fixture" --gate default 2>&1)"
  rc=$?
}

check() {
  # usage: check <name> <fixture> <expect_rc> <needle1> [needle2 ...]
  local name="$1" fixture="$2" expect_rc="$3"; shift 3
  tests=$((tests+1))
  run_validator "$fixture"
  local ok=1 missing=""
  if [ "$rc" -ne "$expect_rc" ]; then ok=0; fi
  if [ "$ok" -eq 1 ]; then
    for needle in "$@"; do
      if ! printf '%s' "$out" | grep -Fq -- "$needle"; then ok=0; missing="$missing $needle"; fi
    done
  fi
  if [ "$ok" -eq 1 ]; then
    echo "PASS $name"
  else
    fails=$((fails+1))
    echo "FAIL $name"
    echo "  expected exit=$expect_rc contains=[$*] got exit=$rc missing=[$missing]"
    printf '%s\n' "$out" | sed 's/^/    /'
  fi
}

check 'complete set validates (all classes, hashes, retention, secrets-excluded)' \
  "$fixtures/complete" 0 'POLICY OK'

check 'incomplete manifest (missing db_wal class) rejected before restore' \
  "$fixtures/incomplete-missing-wal" 1 'POLICY REJECTED' 'db_wal'

check 'missing artifact sha256 rejected and named' \
  "$fixtures/incomplete-missing-hash" 1 'POLICY REJECTED' 'sha256'

check 'tampered base-backup hash rejected and named' \
  "$fixtures/tampered-hash" 1 'POLICY REJECTED' 'sha256' 'base.tar.gz'

check 'archive containing fake secret marker rejected, no restore' \
  "$fake_secret_fixture" 1 'POLICY REJECTED' 'LAGRANGE_SECRET_MARKER' 'kis-app-secret.plaintext'

# Determinism: same input, twice, must produce byte-identical output and the same exit code.
tests=$((tests+1))
run_validator "$fixtures/complete"
r1="$out"; rc1=$rc
run_validator "$fixtures/complete"
r2="$out"; rc2=$rc
if [ "$rc1" -eq 0 ] && [ "$rc2" -eq 0 ] && [ "$r1" = "$r2" ]; then
  echo 'PASS deterministic on identical input'
else
  fails=$((fails+1))
  echo 'FAIL deterministic on identical input'
  echo "  run1 exit=$rc1 run2 exit=$rc2 identical=$([ "$r1" = "$r2" ] && echo yes || echo no)"
fi

if [ "$fails" -gt 0 ]; then
  echo
  echo "BACKUP POLICY TESTS FAILED ($((tests-fails))/$tests)"
  exit 1
fi
echo
echo "ALL BACKUP POLICY TESTS PASSED ($tests/$tests)"
exit 0
