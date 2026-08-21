#!/usr/bin/env bash
# Local, provider-free checks for the KIS daily wrapper and installer contract.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
runner="$script_dir/kis-daily-production.sh"
installer="$script_dir/install-kis-daily.sh"

[ -x "$runner" ] || { echo 'KIS_DAILY_SELF_TEST: runner is not executable' >&2; exit 1; }
[ -x "$installer" ] || { echo 'KIS_DAILY_SELF_TEST: installer is not executable' >&2; exit 1; }
bash -n "$runner"
bash -n "$installer"

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
