#!/usr/bin/env bash
# Fake-only contract test for kind-daily.sh. It never starts Node/Playwright,
# opens a browser, contacts a provider, uses a database, or calls systemd.
set -euo pipefail
umask 077

script_dir=$(cd "$(dirname -- "$0")" && pwd)
wrapper=$script_dir/kind-daily.sh
[ -x "$wrapper" ] || { echo 'KIND_DAILY_SELF_TEST: wrapper is not executable' >&2; exit 1; }
service_file=$script_dir/../../deploy/systemd/lagrange-kind-daily.service
timer_file=$script_dir/../../deploy/systemd/lagrange-kind-daily.timer
installer=$script_dir/install-kind-daily.sh
[ ! -e "$timer_file" ] || { echo 'KIND_DAILY_SELF_TEST: timer file still exists' >&2; exit 1; }
grep -Fq 'Type=oneshot' "$service_file"
grep -Fq 'ExecStart=/opt/lagrange/bin/kind-daily.sh --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION' \
  "$service_file"
if grep -Eiq 'timer|OnCalendar=|Persistent=|WantedBy=|^\[Install\]' "$service_file"; then
  echo 'KIND_DAILY_SELF_TEST: manual service has an automatic activation path' >&2
  exit 1
fi
if grep -Eiq 'timer|OnCalendar=|Persistent=|WantedBy=|^\[Install\]' "$installer"; then
  echo 'KIND_DAILY_SELF_TEST: installer still references an automatic activation path' >&2
  exit 1
fi

test_root=$(mktemp -d -- "${TMPDIR:-/tmp}/lagrange-kind-daily.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT

state_root=$test_root/state
raw_root=$test_root/data
capture_root=$test_root/capture
bin_root=$test_root/bin
date_file=$test_root/target-date
fake_log=$test_root/fake.log
fake_raw_mode=0750
fake_bad_owner_path=
mkdir -p -- "$state_root/candidates" "$raw_root" "$capture_root" "$bin_root"
chmod 700 -- "$state_root" "$state_root/candidates" "$capture_root" "$bin_root"
chmod 750 -- "$raw_root"
printf '2026-08-21\n' >"$date_file"
printf '20260821000001\n' >"$state_root/candidates/2026-08-21.txt"
: >"$fake_log"
chmod 600 -- "$date_file" "$state_root/candidates/2026-08-21.txt" "$fake_log"

: >"$capture_root/capture.mjs"
: >"$capture_root/capture-correction.mjs"

cat >"$bin_root/stat" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = -c ] && [ "${2:-}" = '%u:%a' ] && [ "${3:-}" = -- ]; then
  path=${4:-}
  actual_mode=$(/usr/bin/stat -c '%a' -- "$path")
  safe_mode=$(printf '%o' $((8#$actual_mode & 0755)))
  if [ "$path" = "$KIND_FAKE_RAW_ROOT" ]; then
    printf '0:%s\n' "$KIND_FAKE_RAW_MODE"
    exit 0
  fi
  if [ -n "${KIND_FAKE_BAD_OWNER_PATH:-}" ] \
    && [ "$path" = "$KIND_FAKE_BAD_OWNER_PATH" ]; then
    printf '1000:%s\n' "$safe_mode"
    exit 0
  fi
  if [ "${KIND_FAKE_ROOT_OWNERSHIP:-0}" = 1 ]; then
    printf '0:%s\n' "$safe_mode"
    exit 0
  fi
fi
exec /usr/bin/stat "$@"
EOF

cat >"$bin_root/fake-node" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
log=$KIND_FAKE_LOG
script=$(basename -- "$1")
printf 'node %s\n' "$script" >>"$log"
shift
out=
from=
to=
acceptance=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift 2 ;;
    --from) from=$2; shift 2 ;;
    --to) to=$2; shift 2 ;;
    --acceptance) acceptance=$2; shift 2 ;;
    *) shift ;;
  esac
done
[ "$from" = 2026-08-21 ] && [ "$to" = 2026-08-21 ]
mkdir -p -- "$out"
if [ "$script" = capture.mjs ]; then
  printf 'fake page\n' >"$out/page-0001.html"
  printf '%s\n' '{"source":"kind.krx.co.kr","entry_url":"https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf","surface":"etf-disclosure-list","requested_range":{"from":"2026-08-21","to":"2026-08-21"},"termination":"clamped_duplicate","pages":[{"page_index":1,"file":"page-0001.html","retrieved_at":"2026-08-21T08:00:00Z","form_fields":[]}]}' >"$out/capture.json"
elif [ "$script" = capture-correction.mjs ]; then
  [ "$acceptance" = 20260821000001 ]
  printf '<select id="mainDoc"><option value=""></option><option value="20260821000001|Y">2026.08.21</option></select>\n' >"$out/viewer.html"
  printf '%s\n' '{"source":"kind.krx.co.kr","entry_url":"https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf","surface":"etf-disclosure-correction-viewer","requested_range":{"from":"2026-08-21","to":"2026-08-21"},"anchor_acceptance_number":"20260821000001","viewer_origin_path":"/common/disclsviewer.do","artifact_kind":"rendered_dom_snapshot","retrieved_at":"2026-08-21T08:00:00Z","termination":"viewer_loaded","termination_stage":"viewer","response_diagnostics":{"body_size":10,"form_field_count":1,"target_handler_occurrences":1},"file":"viewer.html"}' >"$out/capture.json"
else
  exit 1
fi
EOF

cat >"$bin_root/fake-kind-raw" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'kind-raw\n' >>"$KIND_FAKE_LOG"
printf 'batch_id: 11111111-1111-4111-8111-111111111111\n'
EOF

cat >"$bin_root/fake-kind-correction-raw" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'kind-correction-raw\n' >>"$KIND_FAKE_LOG"
printf 'batch_id: 22222222-2222-4222-8222-222222222222\n'
EOF

cat >"$bin_root/fake-kind-normalize" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
mode=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --mode) mode=$2; shift 2 ;;
    *) shift ;;
  esac
done
printf 'normalize %s\n' "$mode" >>"$KIND_FAKE_LOG"
EOF
chmod 700 -- "$bin_root/stat" "$bin_root/fake-node" "$bin_root/fake-kind-raw" \
  "$bin_root/fake-kind-correction-raw" "$bin_root/fake-kind-normalize"

run_wrapper() {
  KIND_DAILY_REPO_ROOT=$test_root \
  KIND_DAILY_CAPTURE_ROOT=$capture_root \
  KIND_DAILY_NODE_BIN=$bin_root/fake-node \
  KIND_DAILY_KIND_RAW_BIN=$bin_root/fake-kind-raw \
  KIND_DAILY_KIND_CORRECTION_RAW_BIN=$bin_root/fake-kind-correction-raw \
  KIND_DAILY_NORMALIZE_BIN=$bin_root/fake-kind-normalize \
  KIND_DAILY_RAW_ROOT=$raw_root \
  KIND_DAILY_TEST_MODE=1 \
  KIND_DAILY_TEST_STATE_ROOT=$state_root \
  KIND_DAILY_ENTITLEMENT_REFERENCE=test-entitlement-reference \
  KIND_FAKE_LOG=$fake_log \
  KIND_FAKE_RAW_ROOT=$raw_root \
  KIND_FAKE_RAW_MODE=$fake_raw_mode \
  KIND_FAKE_ROOT_OWNERSHIP=1 \
  KIND_FAKE_BAD_OWNER_PATH=$fake_bad_owner_path \
  PATH=$bin_root:$PATH \
  "$wrapper" --target-date-file "$date_file" "$@"
}

assert_no_calls() {
  ! grep -Eq '^(node|kind-raw|kind-correction-raw|normalize) ' "$fake_log"
}

run_wrapper --plan >"$test_root/plan.out"
grep -Fq 'no_browser=true no_network=true no_write=true' "$test_root/plan.out"
assert_no_calls
[ ! -e "$state_root/run.lock" ]

run_wrapper --check >"$test_root/check.out"
grep -Fq 'KIND_DAILY_CHECK status=pass' "$test_root/check.out"
assert_no_calls

: >"$fake_log"
if run_wrapper --execute >"$test_root/missing-confirmation.out" 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: missing execute confirmation unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'execute_confirmation_required' "$test_root/missing-confirmation.out"
assert_no_calls
[ ! -e "$state_root/run.lock" ]
if compgen -G "$state_root/run-2026-08-21.*" >/dev/null; then
  echo 'KIND_DAILY_SELF_TEST: missing confirmation created staging' >&2
  exit 1
fi

run_wrapper --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION >"$test_root/execute.out"
grep -Fq 'KIND_DAILY status=complete target_date=2026-08-21 list_pages=1 correction_candidates=1' \
  "$test_root/execute.out"
grep -Fq 'node capture.mjs' "$fake_log"
grep -Fq 'node capture-correction.mjs' "$fake_log"
grep -Fq 'normalize disclosure' "$fake_log"
grep -Fq 'normalize correction' "$fake_log"
grep -Fxq '20260821000001' "$state_root/candidates/2026-08-21.txt"
if compgen -G "$state_root/run-2026-08-21.*" >/dev/null; then
  echo 'KIND_DAILY_SELF_TEST: successful private staging was not removed' >&2
  exit 1
fi

fake_bad_owner_path=$state_root
: >"$fake_log"
if run_wrapper --check >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: non-root state root unexpectedly passed' >&2
  exit 1
fi
assert_no_calls

fake_bad_owner_path=$state_root/candidates/2026-08-21.txt
if run_wrapper --check >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: non-root candidate file unexpectedly passed' >&2
  exit 1
fi
assert_no_calls

fake_bad_owner_path=$state_root/run.lock
if run_wrapper --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: non-root run lock unexpectedly passed' >&2
  exit 1
fi
assert_no_calls
fake_bad_owner_path=

fake_raw_mode=0770
if run_wrapper --check >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: group-writable raw root unexpectedly passed' >&2
  exit 1
fi
fake_raw_mode=0750

rm -f -- "$state_root/candidates/2026-08-21.txt"
run_wrapper --plan >"$test_root/missing-plan.out"
[ ! -e "$state_root/candidates/2026-08-21.txt" ]
: >"$fake_log"
run_wrapper --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION >"$test_root/missing-execute.out"
grep -Fq 'correction_candidates=0' "$test_root/missing-execute.out"
[ -f "$state_root/candidates/2026-08-21.txt" ]
[ ! -s "$state_root/candidates/2026-08-21.txt" ]
[ "$(stat -c '%a' -- "$state_root/candidates/2026-08-21.txt")" = 600 ]
grep -Fq 'node capture.mjs' "$fake_log"
if grep -Fq 'capture-correction.mjs' "$fake_log" || grep -Fq 'normalize correction' "$fake_log"; then
  echo 'KIND_DAILY_SELF_TEST: empty candidate file unexpectedly launched correction capture' >&2
  exit 1
fi

printf '2026-08-21\n2026-08-22\n' >"$test_root/bad-date"
if run_wrapper --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION \
    --target-date-file "$test_root/bad-date" >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: invalid date file unexpectedly passed' >&2
  exit 1
fi

for index in 1 2 3 4 5 6; do
  printf '20260821%06d\n' "$index"
done >"$state_root/candidates/2026-08-21.txt"
if run_wrapper --execute --confirm KIND_DAILY_OPERATOR_CONFIRMATION >/dev/null 2>&1; then
  echo 'KIND_DAILY_SELF_TEST: candidate budget unexpectedly passed' >&2
  exit 1
fi

echo 'KIND_DAILY_SELF_TEST: PASS'
