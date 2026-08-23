#!/usr/bin/env bash
# Local, provider-free checks for the KIS daily wrapper and installer contract.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
runner="$script_dir/kis-daily-production.sh"
installer="$script_dir/install-kis-daily.sh"
state_helper="$script_dir/lib/kis-daily-state.py"

[ -x "$runner" ] || { echo 'KIS_DAILY_SELF_TEST: runner is not executable' >&2; exit 1; }
[ -x "$installer" ] || { echo 'KIS_DAILY_SELF_TEST: installer is not executable' >&2; exit 1; }
[ -f "$state_helper" ] && [ ! -L "$state_helper" ] || { echo 'KIS_DAILY_SELF_TEST: state helper is missing or unsafe' >&2; exit 1; }
bash -n "$runner"
bash -n "$installer"
python3 -B -c 'import ast, pathlib, sys; ast.parse(pathlib.Path(sys.argv[1]).read_text(encoding="ascii"))' "$state_helper"

help_output=$(bash "$runner" --help)
grep -Fq -- '--plan' <<<"$help_output"
grep -Fq -- '--check' <<<"$help_output"
grep -Fq -- '--execute' <<<"$help_output"

installer_help=$(bash "$installer" --help)
grep -Fq -- '--dry-run' <<<"$installer_help"
grep -Fq -- '--preflight' <<<"$installer_help"
grep -Fq -- '--check' <<<"$installer_help"
grep -Fq -- '--apply' <<<"$installer_help"

test_root=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-kis-daily-self-test.XXXXXX")
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT
printf 'LAGRANGE_DATA_DIR=%s\n' "$test_root/data" >"$test_root/production.env"

set +e
LAGRANGE_ENV_FILE="$test_root/production.env" \
LAGRANGE_XKRX_CALENDAR_DIR="$test_root/missing-calendar" \
  bash "$runner" --plan >"$test_root/plan.out" 2>"$test_root/plan.err"
plan_rc=$?
set -e
[ "$plan_rc" -eq 2 ] || { echo "KIS_DAILY_SELF_TEST: expected blocked plan, rc=$plan_rc" >&2; exit 1; }
grep -Fq 'XKRX scheduler artifact validation failed' "$test_root/plan.err"

mkdir -p "$test_root/releases" "$test_root/calendar" "$test_root/state" "$test_root/systemd"
commit=0123456789abcdef0123456789abcdef01234567
release="$test_root/releases/$commit"
mkdir -p "$release/scripts/ops/lib"
state_identity=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
stale_identity=abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789
state_db="$test_root/state/db.tsv"
printf '%s\n' \
  $'META\t2026-08-18\t2026-08-19\t2\t2' \
  $'DATE\t2026-08-18\t1\t-\t-' \
  $'DATE\t2026-08-19\t1\t-\t-' >"$state_db"

state_failure() {
  local label=$1 expected=$2 state_path=$3
  local output="$test_root/$label.out" error="$test_root/$label.err" rc
  set +e
  python3 -B "$state_helper" "$state_path" "$state_identity" "$state_db" \
    2026-08-18 2026-08-24 >"$output" 2>"$error"
  rc=$?
  set -e
  [ "$rc" -ne 0 ] || { echo "KIS_DAILY_SELF_TEST: $label unexpectedly passed" >&2; exit 1; }
  [ "$(<"$error")" = "$expected" ] || {
    echo "KIS_DAILY_SELF_TEST: $label diagnostic was not typed" >&2
    exit 1
  }
  [ ! -s "$output" ] || { echo "KIS_DAILY_SELF_TEST: $label wrote stdout" >&2; exit 1; }
  for forbidden in "$state_identity" "$stale_identity" PROVIDER_BODY_SENTINEL SECRET_VALUE_SENTINEL Traceback; do
    if grep -Fq -- "$forbidden" "$error"; then
      echo "KIS_DAILY_SELF_TEST: $label leaked $forbidden" >&2
      exit 1
    fi
  done
}

missing_state="$test_root/state/missing.tsv"
state_failure missing DAILY_STATE_MISSING "$missing_state"

stale_state="$test_root/state/stale.tsv"
printf 'LAGRANGE_BACKFILL_STATE_V4\t%s\n' "$stale_identity" >"$stale_state"
state_failure stale DAILY_STATE_STALE "$stale_state"

malformed_state="$test_root/state/malformed.tsv"
printf '%s\n' \
  "LAGRANGE_BACKFILL_STATE_V4	$state_identity" \
  $'2026-08-18\tPUBLISHED\t'"$state_identity"$'\tPROVIDER_BODY_SENTINEL' >"$malformed_state"
state_failure malformed DAILY_STATE_MALFORMED "$malformed_state"

not_appendable_state="$test_root/state/not-appendable.tsv"
mkdir "$not_appendable_state"
state_failure not-appendable DAILY_STATE_NOT_APPENDABLE "$not_appendable_state"

initialized_state="$test_root/state/initialized.tsv"
: >"$initialized_state"
set +e
python3 -B "$state_helper" "$initialized_state" "$state_identity" "$state_db" \
  2026-08-18 2026-08-24 >"$test_root/initialize.out" 2>"$test_root/initialize.err"
initialize_rc=$?
set -e
[ "$initialize_rc" -eq 0 ] || { echo 'KIS_DAILY_SELF_TEST: successful initialization failed' >&2; exit 1; }
[ ! -s "$test_root/initialize.out" ] && [ ! -s "$test_root/initialize.err" ] || {
  echo 'KIS_DAILY_SELF_TEST: successful initialization emitted diagnostics' >&2
  exit 1
}
grep -Fq $'LAGRANGE_BACKFILL_STATE_V4\t'"$state_identity" "$initialized_state"
grep -Fq $'2026-08-18\tPUBLISHED\t'"$state_identity" "$initialized_state"
[ "$(grep -c $'\tPUBLISHED\t' "$initialized_state")" -eq 2 ] || {
  echo 'KIS_DAILY_SELF_TEST: initialization wrote the wrong publication count' >&2
  exit 1
}
cp -- "$initialized_state" "$test_root/initialized-before-replay.tsv"
set +e
python3 -B "$state_helper" "$initialized_state" "$state_identity" "$state_db" \
  2026-08-18 2026-08-24 >"$test_root/replay.out" 2>"$test_root/replay.err"
replay_rc=$?
set -e
[ "$replay_rc" -eq 0 ] || { echo 'KIS_DAILY_SELF_TEST: idempotent replay failed' >&2; exit 1; }
cmp -s "$test_root/initialized-before-replay.tsv" "$initialized_state" || {
  echo 'KIS_DAILY_SELF_TEST: idempotent replay changed state' >&2
  exit 1
}
[ ! -s "$test_root/replay.out" ] && [ ! -s "$test_root/replay.err" ] || {
  echo 'KIS_DAILY_SELF_TEST: idempotent replay emitted diagnostics' >&2
  exit 1
}

for required in \
  scripts/ops/kis-daily-production.sh \
  scripts/ops/backfill-production.sh \
  scripts/ops/validate-production-config.sh \
  scripts/ops/xkrx-calendar-bootstrap.py; do
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$release/$required"
  chmod 0755 "$release/$required"
done
for required in scripts/ops/lib/dotenv.sh scripts/ops/lib/db.sh; do
  printf '%s\n' '# helper fixture' >"$release/$required"
  chmod 0644 "$release/$required"
done
printf '%s\n' '# helper fixture' >"$release/scripts/ops/lib/kis-daily-state.py"
chmod 0644 "$release/scripts/ops/lib/kis-daily-state.py"

installer_args=(
  --release-root "$release"
  --code-commit "$commit"
  --env-file "$test_root/production.env"
  --calendar-dir "$test_root/calendar"
  --state-dir "$test_root/state"
  --lock-file "$test_root/state/kis-daily.lock"
  --systemd-dir "$test_root/systemd"
)
KIS_DAILY_TIMER_TEST_NOW=16:30:00 \
  bash "$installer" --dry-run "${installer_args[@]}" >"$test_root/install-plan.out"
grep -Fq "release=$release code_commit=$commit" "$test_root/install-plan.out"
grep -Fq "WorkingDirectory=$release" "$test_root/install-plan.out"
grep -Fq "ExecStart=$release/scripts/ops/kis-daily-production.sh --execute" "$test_root/install-plan.out"
grep -Fq 'apply-window=closed (at/after 16:30:00 Asia/Seoul; --apply is refused)' "$test_root/install-plan.out"

bash "$installer" --preflight "${installer_args[@]}" >"$test_root/install-preflight.out"
grep -Fq 'KIS_DAILY_INSTALL_PREFLIGHT: PASS' "$test_root/install-preflight.out"

# Make the real --apply clock appear late. The fake systemctl is a marker only;
# the installer must reject before command discovery or invocation. This does
# not run systemd, KIS, Docker, a database, sudo, or any credential path.
mkdir "$test_root/fake-bin"
printf '%s\n' '#!/usr/bin/env bash' 'printf "163000"' >"$test_root/fake-bin/date"
printf '%s\n' '#!/usr/bin/env bash' 'touch "${KIS_DAILY_SELF_TEST_SYSTEMD_MARKER:?}"' 'exit 0' >"$test_root/fake-bin/systemctl"
chmod 0755 "$test_root/fake-bin/date" "$test_root/fake-bin/systemctl"
set +e
KIS_DAILY_SELF_TEST_SYSTEMD_MARKER="$test_root/systemd-called" \
PATH="$test_root/fake-bin:/usr/bin:/bin" \
  bash "$installer" --apply "${installer_args[@]}" >"$test_root/late-apply.out" 2>"$test_root/late-apply.err"
late_apply_rc=$?
set -e
[ "$late_apply_rc" -eq 1 ] || { echo "KIS_DAILY_SELF_TEST: expected late apply refusal, rc=$late_apply_rc" >&2; exit 1; }
grep -Fq 'allowed only before 16:30:00 Asia/Seoul' "$test_root/late-apply.err"
[ ! -e "$test_root/systemd-called" ] || { echo 'KIS_DAILY_SELF_TEST: late apply invoked systemd' >&2; exit 1; }
short_commit=${commit:0:7}
[ ! -e "$test_root/systemd/lagrange-kis-daily-$short_commit.service" ]
[ ! -e "$test_root/systemd/lagrange-kis-daily-$short_commit.timer" ]

grep -Fq 'one systemd activation daily at 16:30 Asia/Seoul' "$runner"
grep -Fq 'Persistent=true' "$installer"
grep -Fq 'SuccessExitStatus=74 75' "$installer"
grep -Fq 'Requires=docker.service' "$installer"
grep -Fq 'flock -n 9' "$runner"
grep -Fq -- '--backfill-session-dates' "$runner"
grep -Fq "fetch_mode='credentialed'" "$runner"
grep -Fq 'KIS_DAILY: PASS' "$runner"
if grep -Eq 'KIS_ACCOUNT_REF|(^|[^[:alnum:]_])CANO([^[:alnum:]_]|$)|ACNT_PRDT_CD|--profile[[:space:]]+live' "$installer"; then
  echo 'KIS_DAILY_SELF_TEST: installer contains a forbidden account/live surface' >&2
  exit 1
fi
echo 'KIS_DAILY_SELF_TEST: PASS (syntax, provider-free plan, immutable release unit paths, late apply gate, lock/calendar/DB/worker contracts)'
