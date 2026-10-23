#!/usr/bin/env bash
# Focused local tests for the append-only backfill resume protocol.
# No production path, KIS, Docker, DB, systemd, or secret is touched.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
progress="$script_dir/lib/backfill-progress.py"
installer="$script_dir/install-kis-backfill-timer.sh"
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

python3 - "$progress" <<'PY'
import ast
import sys
ast.parse(open(sys.argv[1], encoding="utf-8").read(), filename=sys.argv[1])
PY
bash -n "$script_dir/backfill-production.sh" "$installer"

identity=$(printf 'a%.0s' {1..64})
write_state() {
  local path=$1
  printf 'LAGRANGE_BACKFILL_STATE_V4\t%s\n' "$identity" >"$path"
  printf '2026-01-01\tRUNNING\t%s\n' "$identity" >>"$path"
}

deferred_state="$tmp/deferred.tsv"
write_state "$deferred_state"
set +e
printf '%s\n' '{"status":"error","error_code":"KIS_CALENDAR_SNAPSHOT_MISS","target_date":null,"phase":"ingest","class":"permanent","message":"body must not pass","endpoint":"https://must-not-pass"}' |
  python3 "$progress" "$deferred_state" "$identity" 2026-01-01 2026-01-03 >"$tmp/deferred.out" 2>&1
rc=$?
set -e
[ "$rc" -eq 75 ] || { cat "$tmp/deferred.out" >&2; exit 1; }
grep -Fq 'KIS_CALENDAR_SNAPSHOT_MISS' "$tmp/deferred.out"
! grep -Fq 'body must not pass' "$tmp/deferred.out"
! grep -Fq 'https://must-not-pass' "$tmp/deferred.out"
grep -Fq $'2026-01-01\tDEFERRED\t'"$identity"$'\tKIS_CALENDAR_SNAPSHOT_MISS' "$deferred_state"
[ "$(grep -c $'\tFAILED\t' "$deferred_state" || true)" -eq 0 ]

failed_state="$tmp/failed.tsv"
write_state "$failed_state"
set +e
printf '%s\n' '{"status":"error","error_code":"UNSUPPORTED_ACTION","target_date":null,"phase":"ingest","class":"permanent"}' |
  python3 "$progress" "$failed_state" "$identity" 2026-01-01 2026-01-03 >/dev/null 2>&1
rc=$?
set -e
[ "$rc" -eq 1 ]
grep -Fq $'2026-01-01\tFAILED\t'"$identity"$'\tUNSUPPORTED_ACTION' "$failed_state"
[ "$(grep -c $'\tFAILED\t' "$failed_state")" -eq 1 ]
! grep -Fq $'2026-01-02\tFAILED\t' "$failed_state"

incomplete_state="$tmp/incomplete.tsv"
write_state "$incomplete_state"
set +e
printf '%s\n' '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2026-01-01"}' |
  python3 "$progress" "$incomplete_state" "$identity" 2026-01-01 2026-01-03 >/dev/null 2>&1
rc=$?
set -e
[ "$rc" -eq 1 ]
grep -Fq $'2026-01-02\tFAILED\t'"$identity"$'\tBACKFILL_INCOMPLETE' "$incomplete_state"
! grep -Fq $'2026-01-03\tFAILED\t' "$incomplete_state"

session_state="$tmp/session-list.tsv"
write_state "$session_state"
printf '%s\tRUNNING\t%s\n' 2026-01-01 "$identity" >>"$session_state"
printf '%s\tRUNNING\t%s\n' 2026-01-05 "$identity" >>"$session_state"
printf '%s\n' \
  '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2026-01-01"}' \
  '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000002","target_date":"2026-01-05"}' | \
  python3 "$progress" "$session_state" "$identity" 2026-01-01 2026-01-05 \
    2026-01-01,2026-01-05 >/dev/null
grep -Fq $'2026-01-05\tPUBLISHED\t'"$identity" "$session_state"

fake_commit=$(printf 'b%.0s' {1..40})
fake_release="$tmp/releases/$fake_commit"
mkdir -p "$fake_release/scripts/ops"
printf '#!/usr/bin/env bash\n' >"$fake_release/scripts/ops/backfill-production.sh"
chmod 0755 "$fake_release/scripts/ops/backfill-production.sh"
plan=$(
  "$installer" --release-root "$fake_release" \
    --code-commit "$fake_commit" \
    --state-file "$tmp/data/backfill/state.tsv" \
    --start 2020-01-31 --end 2026-08-18 --dry-run
)
grep -Fq 'daily 03:15 Asia/Seoul' <<<"$plan"
grep -Fq 'no KIS/Docker/DB/secret/order call' <<<"$plan"
norep=$(grep -Ei 'app.?secret|app.?key|access.?token|CANO|ACNT_PRDT_CD' <<<"$plan" || true)
[ -z "$norep" ]

# Exercise the exact schedule boundary through the dry-run-only test clock;
# the production --apply path always reads the real Asia/Seoul clock.
early_plan=$(KIS_BACKFILL_TIMER_TEST_NOW=03:14:59 "$installer" \
  --release-root "$fake_release" --code-commit "$fake_commit" \
  --state-file "$tmp/data/backfill/state.tsv" \
  --start 2020-01-31 --end 2026-08-18 --dry-run)
grep -Fq 'apply-window=open' <<<"$early_plan"
late_plan=$(KIS_BACKFILL_TIMER_TEST_NOW=03:15:00 "$installer" \
  --release-root "$fake_release" --code-commit "$fake_commit" \
  --state-file "$tmp/data/backfill/state.tsv" \
  --start 2020-01-31 --end 2026-08-18 --dry-run)
grep -Fq 'apply-window=closed' <<<"$late_plan"
grep -Fq 'SuccessExitStatus=74 75' "$installer"
grep -Fq 'automatic resume is blocked' "$script_dir/backfill-production.sh"

python3 -B - "$progress" "$tmp" <<'PY'
import json, pathlib, subprocess, sys
progress, root = sys.argv[1], pathlib.Path(sys.argv[2])
identity = "a" * 64
dates = ["2026-10-12", "2026-10-13"]
def event(date=dates[0], **changes):
    value = dict(status="event", event="raw_committed", phase="raw_capture",
                 target_date=date, batch_id="00000000-0000-4000-8000-000000000001",
                 reused_existing_source=True, ready=False, publication=False, curated=False, db=False)
    value.update(changes)
    return value
def run(name, records, header="LAGRANGE_EOD_RAW_STATE_V1"):
    state = root / f"raw-{name}.tsv"
    state.write_text(f"{header}\t{identity}\n", encoding="ascii")
    before = state.read_bytes()
    data = "\n".join(json.dumps(v) if isinstance(v, dict) else v for v in records) + "\n"
    proc = subprocess.run([sys.executable, "-B", progress, "--raw-source", str(state),
                           identity, *dates, ",".join(dates)], input=data, text=True, capture_output=True)
    return proc, state.read_text(), before
proc, state, _ = run("success", [event(), event(dates[1])])
assert proc.returncode == 0 and state.count("\tRAW_COMMITTED\t") == 2 and "PUBLISHED" not in state
for label, records in [
    ("publication-flag", [event(publication=True)]),
    ("missing-flags", [{"status":"event", "event":"raw_committed", "phase":"raw_capture"}]),
    ("legacy-event", [event(event="published", phase="canonical_publication")]),
    ("bad-id", [event(batch_id="not-an-id")]),
    ("wrong-date", [event(dates[1])]),
    ("body", ["PROVIDER_BODY_SENTINEL"]),
]:
    proc, state, _ = run(label, records)
    assert proc.returncode != 0 and "\tRAW_COMMITTED\t" not in state, label
    assert "PROVIDER_BODY_SENTINEL" not in proc.stdout + proc.stderr + state, label
proc, state, before = run("foreign", [event()], "LAGRANGE_BACKFILL_STATE_V4")
assert proc.returncode != 0 and state.encode() == before
proc, state, _ = run("partial", [event(), dict(status="error", error_code="KIS_CALENDAR_SNAPSHOT_MISS",
                                             phase="raw_capture", **{"class":"retryable"})])
assert proc.returncode == 75 and state.count("\tRAW_COMMITTED\t") == 1 and "\tDEFERRED\t" in state
print("EOD_RAW_PROGRESS_SELF_TEST: PASS (9 protocol cases)")
PY
echo 'BACKFILL_RESUME_SELF_TEST: PASS'
