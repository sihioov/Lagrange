#!/usr/bin/env bash
# Focused local test for the non-approving backfill review report.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
report="$script_dir/backfill-review-report.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-backfill-review.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

[ -x "$report" ] || { echo 'BACKFILL_REVIEW_SELF_TEST: report is not executable' >&2; exit 1; }
bash -n "$report"
plan=$(
  "$report" --start 2026-08-18 --end 2026-08-18 \
    --state-file "$tmp/state.tsv" --data-root "$tmp/data" --plan
)
grep -Fq 'PLAN_ONLY: no production file read, write, or external service action made' <<<"$plan"

identity="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

python3 - "$tmp" <<'PY'
from __future__ import annotations

import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
data = root / "data"
for provider in ("kis", "kis-normalized"):
    manifest = data / "raw" / "manifests" / f"provider={provider}" / "market=kr" / "manifest.jsonl"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        json.dumps({"provider": provider, "market": "kr", "date": "2026-08-18"}) + "\n",
        encoding="utf-8",
    )

identity = "0123456789abcdef" * 4
(root / "state.tsv").write_text(
    "\n".join(
        (
            f"LAGRANGE_BACKFILL_STATE_V4\t{identity}",
            f"2026-08-18\tRUNNING\t{identity}",
            f"2026-08-18\tRETRYABLE\t{identity}\tTRANSIENT_PROVIDER_ERROR",
            f"2026-08-18\tDEFERRED\t{identity}\tKIS_CALENDAR_SNAPSHOT_MISS",
            f"2026-08-18\tFAILED\t{identity}\tUNSUPPORTED_ACTION",
            f"2026-08-18\tPUBLISHED\t{identity}",
        )
    )
    + "\n",
    encoding="ascii",
)

relative = "bars/market=kr/symbol=069500.KRX/year=2026/version=1/bars.parquet"
artifact = data / "curated" / relative
artifact.parent.mkdir(parents=True, exist_ok=True)
artifact.write_bytes(b"PAR1fixturePAR1")
artifact_hash = hashlib.sha256(artifact.read_bytes()).hexdigest()

manifest = data / "curated" / "datasets" / "krx_eod_bars" / "version=1" / "manifest.json"
manifest.parent.mkdir(parents=True, exist_ok=True)
body = {
    "dataset_id": "krx_eod_bars",
    "version": 1,
    "capability": "PRICE_RETURN_ONLY",
    "created_at": "2026-08-18T00:00:00Z",
    "source_batches": [
        {
            "batch_id": "00000000-0000-4000-8000-000000000001",
            "bars_file": "bars.json",
            "bars_hash": "sha256:" + "a" * 64,
            "actions_file": "corporate-actions.json",
            "actions_hash": "sha256:" + "b" * 64,
        }
    ],
    "artifacts": [
        {
            "path": relative,
            "sha256": "sha256:" + artifact_hash,
            "size_bytes": artifact.stat().st_size,
            "schema": "bars-v1",
        }
    ],
    "bar_count": 1,
    "action_count": 0,
}
canonical = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
body["content_hash"] = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()
manifest.write_text(json.dumps(body, separators=(",", ":")), encoding="utf-8")
PY

complete=$(
  "$report" --start 2026-08-18 --end 2026-08-18 \
    --state-file "$tmp/state.tsv" --data-root "$tmp/data" --check
)
grep -Fq 'BACKFILL_REVIEW: CURATED_CANDIDATE_FOUND_UNAPPROVED' <<<"$complete"
grep -Fq 'DB_READY=NOT_CHECKED' <<<"$complete"
grep -Fq 'historical_failures=1 historical_deferred=1 historical_retryable=1' <<<"$complete"
if grep -Eq 'READY registration succeeded|DATASET_READY|BACKFILL_REVIEW: PASS' <<<"$complete"; then
  echo 'BACKFILL_REVIEW_SELF_TEST: report made an approval/readiness claim' >&2
  exit 1
fi

cp -- "$tmp/state.tsv" "$tmp/unexpected-field.tsv"
printf '%s\n' $'2026-08-18\tPUBLISHED\t'"$identity"$'\tEXTRA' >>"$tmp/unexpected-field.tsv"
if "$report" --start 2026-08-18 --end 2026-08-18 \
  --state-file "$tmp/unexpected-field.tsv" --data-root "$tmp/data" --check >"$tmp/unexpected-field.out" 2>&1; then
  echo 'BACKFILL_REVIEW_SELF_TEST: unexpected state field unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'unexpected error code' "$tmp/unexpected-field.out"

cp -- "$tmp/state.tsv" "$tmp/malformed-code.tsv"
printf '%s\n' $'2026-08-18\tFAILED\t'"$identity"$'\tbad-code' >>"$tmp/malformed-code.tsv"
if "$report" --start 2026-08-18 --end 2026-08-18 \
  --state-file "$tmp/malformed-code.tsv" --data-root "$tmp/data" --check >"$tmp/malformed-code.out" 2>&1; then
  echo 'BACKFILL_REVIEW_SELF_TEST: malformed state error code unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'invalid error code' "$tmp/malformed-code.out"

sed -i $'s/2026-08-18\tPUBLISHED\t/2026-08-18\tRUNNING\t/' "$tmp/state.tsv"
if "$report" --start 2026-08-18 --end 2026-08-18 \
  --state-file "$tmp/state.tsv" --data-root "$tmp/data" --check >"$tmp/incomplete.out" 2>&1; then
  echo 'BACKFILL_REVIEW_SELF_TEST: incomplete state unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'BACKFILL_REVIEW: BLOCKED' "$tmp/incomplete.out"

# An execute-mode weekend selection must stop after local calendar validation:
# even with a fake validator/config and a fake Docker binary in PATH, no Docker
# command may be invoked when the validated session list is empty. The execute
# contract is root-only because it protects root-owned state and lock files.
if [ "$(id -u)" -eq 0 ]; then
no_call_root="$tmp/no-call-root"
mkdir -p "$no_call_root/scripts/ops/lib" "$no_call_root/data/calendars/xkrx" "$no_call_root/fake-bin"
cp -- "$script_dir/backfill-production.sh" "$no_call_root/scripts/ops/backfill-production.sh"
cp -- "$script_dir/xkrx-calendar-bootstrap.py" "$no_call_root/scripts/ops/xkrx-calendar-bootstrap.py"
cp -- "$script_dir/lib/dotenv.sh" "$no_call_root/scripts/ops/lib/dotenv.sh"
cp -- "$script_dir/../../data/calendars/xkrx/calendar.json" \
  "$no_call_root/data/calendars/xkrx/calendar.json"
cp -- "$script_dir/../../data/calendars/xkrx/manifest.json" \
  "$no_call_root/data/calendars/xkrx/manifest.json"
cp -- "$script_dir/../../data/calendars/xkrx/overrides.json" \
  "$no_call_root/data/calendars/xkrx/overrides.json"
cat >"$no_call_root/scripts/ops/validate-production-config.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exit 0
EOF
chmod 0755 "$no_call_root/scripts/ops/backfill-production.sh" \
  "$no_call_root/scripts/ops/xkrx-calendar-bootstrap.py" \
  "$no_call_root/scripts/ops/validate-production-config.sh"
cat >"$no_call_root/fake-bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${NO_CALL_DOCKER_LOG:?}"
exit 99
EOF
chmod 0755 "$no_call_root/fake-bin/docker"
cat >"$no_call_root/production.env" <<EOF
LAGRANGE_DATA_DIR=$no_call_root/data
LAGRANGE_BACKFILL_STATE=$no_call_root/state/backfill/state.tsv
LAGRANGE_CODE_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
RESEARCH_ENTITLEMENT_REFERENCE=fixture-entitlement
EOF
no_call_output=$(
  PATH="$no_call_root/fake-bin:$PATH" \
  NO_CALL_DOCKER_LOG="$no_call_root/docker.log" \
  LAGRANGE_ENV_FILE="$no_call_root/production.env" \
  LAGRANGE_BACKFILL_STATE="$no_call_root/state/backfill/state.tsv" \
  BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
  bash "$no_call_root/scripts/ops/backfill-production.sh" \
    --start 2020-02-01 --end 2020-02-02 --universe etf --execute
)
grep -Fq 'BACKFILL: PASS (sessions=0 skipped_non_sessions=2; no worker/KIS/Docker call)' <<<"$no_call_output"
[ ! -s "$no_call_root/docker.log" ]

clone_no_call_fixture() {
  local target=$1
  cp -a -- "$no_call_root" "$target"
  rm -rf -- "$target/state/backfill"
  sed -i "s|$no_call_root|$target|g" "$target/production.env"
}

expect_no_call_blocked() {
  local fixture=$1 expected=$2 output
  if output=$(PATH="$fixture/fake-bin:$PATH" \
      NO_CALL_DOCKER_LOG="$fixture/docker.log" \
      LAGRANGE_ENV_FILE="$fixture/production.env" \
      LAGRANGE_BACKFILL_STATE="$fixture/state/backfill/state.tsv" \
      BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
      bash "$fixture/scripts/ops/backfill-production.sh" \
        --start 2020-02-01 --end 2020-02-02 --universe etf --execute 2>&1); then
    echo "BACKFILL_REVIEW_SELF_TEST: $fixture unexpectedly passed" >&2
    exit 1
  fi
  grep -Fq "$expected" <<<"$output"
  [ ! -s "$fixture/docker.log" ]
}

bad_dir_mode="$tmp/no-call-bad-dir-mode"
clone_no_call_fixture "$bad_dir_mode"
mkdir -p "$bad_dir_mode/state/backfill"
chown root:root "$bad_dir_mode/state/backfill"
chmod 0750 "$bad_dir_mode/state/backfill"
expect_no_call_blocked "$bad_dir_mode" 'backfill state directory must be root:root mode 0700'

bad_parent_owner="$tmp/no-call-bad-parent-owner"
clone_no_call_fixture "$bad_parent_owner"
chown 65534:65534 "$bad_parent_owner/state"
expect_no_call_blocked "$bad_parent_owner" 'backfill state ancestor must be root:root'

bad_parent_symlink="$tmp/no-call-bad-parent-symlink"
clone_no_call_fixture "$bad_parent_symlink"
mkdir -p "$bad_parent_symlink/state/real"
ln -s real "$bad_parent_symlink/state/backfill"
expect_no_call_blocked "$bad_parent_symlink" 'must not traverse a symlink'

bad_lock_symlink="$tmp/no-call-bad-lock-symlink"
clone_no_call_fixture "$bad_lock_symlink"
mkdir -p "$bad_lock_symlink/state/backfill"
chown root:root "$bad_lock_symlink/state/backfill"
chmod 0700 "$bad_lock_symlink/state/backfill"
ln -s lock-target "$bad_lock_symlink/state/backfill/state.tsv.lock"
expect_no_call_blocked "$bad_lock_symlink" 'backfill-state-lock must not be a symlink'

bad_lock_mode="$tmp/no-call-bad-lock-mode"
clone_no_call_fixture "$bad_lock_mode"
mkdir -p "$bad_lock_mode/state/backfill"
chown root:root "$bad_lock_mode/state/backfill"
chmod 0700 "$bad_lock_mode/state/backfill"
: >"$bad_lock_mode/state/backfill/state.tsv.lock"
chown root:root "$bad_lock_mode/state/backfill/state.tsv.lock"
chmod 0640 "$bad_lock_mode/state/backfill/state.tsv.lock"
expect_no_call_blocked "$bad_lock_mode" 'backfill-state-lock must be root:root mode 0600'

bad_state_owner="$tmp/no-call-bad-state-owner"
clone_no_call_fixture "$bad_state_owner"
mkdir -p "$bad_state_owner/state/backfill"
chown root:root "$bad_state_owner/state/backfill"
chmod 0700 "$bad_state_owner/state/backfill"
: >"$bad_state_owner/state/backfill/state.tsv"
chown 65534:65534 "$bad_state_owner/state/backfill/state.tsv"
chmod 0600 "$bad_state_owner/state/backfill/state.tsv"
expect_no_call_blocked "$bad_state_owner" 'backfill-state must be root:root mode 0600'

bad_state_mode="$tmp/no-call-bad-state-mode"
clone_no_call_fixture "$bad_state_mode"
mkdir -p "$bad_state_mode/state/backfill"
chown root:root "$bad_state_mode/state/backfill"
chmod 0700 "$bad_state_mode/state/backfill"
: >"$bad_state_mode/state/backfill/state.tsv"
chown root:root "$bad_state_mode/state/backfill/state.tsv"
chmod 0640 "$bad_state_mode/state/backfill/state.tsv"
expect_no_call_blocked "$bad_state_mode" 'backfill-state must be root:root mode 0600'
else
echo 'BACKFILL_REVIEW_SELF_TEST: execute no-call fixture skipped for non-root caller'
fi

echo 'BACKFILL_REVIEW_SELF_TEST: PASS (local-only; no DB/provider/service action)'
