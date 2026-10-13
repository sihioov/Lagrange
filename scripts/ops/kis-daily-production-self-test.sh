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
bash -n "$script_dir/backfill-production.sh"
bash -n "$script_dir/backfill-production-self-test.sh"
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
grep -Fq -- '--install-disabled' <<<"$installer_help"

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

# Fake host commands are confined to this local test fixture. The late --apply
# case must stop before systemctl; disabled-install cases use only fake
# systemctl and synthetic stat/chown/id results.
mkdir "$test_root/fake-bin"
cat >"$test_root/fake-bin/date" <<'EOF'
#!/usr/bin/env bash
printf '%s' "${KIS_DAILY_SELF_TEST_NOW_HMS:-164500}"
EOF
cat >"$test_root/fake-bin/id" <<'EOF'
#!/usr/bin/env bash
[ "$#" -eq 1 ] && [ "$1" = -u ] || exit 90
printf '%s\n' "${KIS_DAILY_SELF_TEST_UID:-0}"
EOF
cat >"$test_root/fake-bin/chown" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${KIS_DAILY_SELF_TEST_CHOWN_LOG:?}"
[ "$#" -eq 3 ] && [ "$1" = 0:0 ] && [ "$2" = -- ] || exit 90
case "$3" in
  "${KIS_DAILY_SELF_TEST_SYSTEMD_DIR:?}"/*) ;;
  *) exit 91 ;;
esac
EOF
cat >"$test_root/fake-bin/stat" <<'EOF'
#!/usr/bin/env bash
if [ "$#" -eq 4 ] && [ "$1" = -c ] && [ "$2" = '%u:%g:%a' ] && [ "$3" = -- ]; then
  case "$4" in
    "${KIS_DAILY_SELF_TEST_SYSTEMD_DIR:?}"/*)
      printf '0:0:644\n'
      exit 0
      ;;
  esac
fi
exec /usr/bin/stat "$@"
EOF
cat >"$test_root/fake-bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${KIS_DAILY_SELF_TEST_SYSTEMD_LOG:?}"
if [ "${1-}" = show ]; then
  [ "$#" -eq 4 ] &&
    [ "$2" = --no-pager ] &&
    [ "$3" = --property=LoadState,ActiveState,UnitFileState ] || exit 90
  mode=${KIS_DAILY_SELF_TEST_SYSTEMD_MODE:-normal}
  reloaded=0
  [ ! -e "${KIS_DAILY_SELF_TEST_RELOAD_MARKER:?}" ] || reloaded=1
  case "$mode" in
    query-failure) exit 1 ;;
    malformed)
      printf 'LoadState=not-found\nActiveState=inactive\nUnexpected=value\n'
      ;;
    duplicate)
      printf 'LoadState=not-found\nActiveState=inactive\nLoadState=not-found\n'
      ;;
    missing)
      printf 'LoadState=not-found\nActiveState=inactive\n'
      ;;
    extra-blank)
      printf 'LoadState=not-found\nActiveState=inactive\nUnitFileState=\n\n'
      ;;
    loaded-collision)
      printf 'LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n'
      ;;
    active-collision)
      printf 'LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n'
      ;;
    enabled-collision)
      printf 'LoadState=not-found\nActiveState=inactive\nUnitFileState=enabled\n'
      ;;
    post-enabled)
      if [ "$reloaded" -eq 1 ]; then
        printf 'LoadState=loaded\nActiveState=inactive\nUnitFileState=enabled\n'
      else
        printf 'LoadState=not-found\nActiveState=inactive\nUnitFileState=\n'
      fi
      ;;
    normal)
      if [ "$reloaded" -eq 1 ]; then
        printf 'LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n'
      else
        printf 'LoadState=not-found\nActiveState=inactive\nUnitFileState=\n'
      fi
      ;;
    *) exit 91 ;;
  esac
  exit 0
fi
if [ "$#" -eq 1 ] && [ "$1" = daemon-reload ]; then
  touch "${KIS_DAILY_SELF_TEST_RELOAD_MARKER:?}"
  exit 0
fi
exit 92
EOF
chmod 0755 "$test_root/fake-bin/date" "$test_root/fake-bin/id" \
  "$test_root/fake-bin/chown" "$test_root/fake-bin/stat" \
  "$test_root/fake-bin/systemctl"
set +e
KIS_DAILY_SELF_TEST_SYSTEMD_LOG="$test_root/late-apply.systemctl.log" \
KIS_DAILY_SELF_TEST_RELOAD_MARKER="$test_root/late-apply.reloaded" \
KIS_DAILY_SELF_TEST_NOW_HMS=164500 \
KIS_DAILY_TIMER_TEST_NOW=00:00:00 \
PATH="$test_root/fake-bin:/usr/bin:/bin" \
  bash "$installer" --apply "${installer_args[@]}" >"$test_root/late-apply.out" 2>"$test_root/late-apply.err"
late_apply_rc=$?
set -e
[ "$late_apply_rc" -eq 1 ] || { echo "KIS_DAILY_SELF_TEST: expected late apply refusal, rc=$late_apply_rc" >&2; exit 1; }
grep -Fq 'allowed only before 16:30:00 Asia/Seoul' "$test_root/late-apply.err"
[ ! -s "$test_root/late-apply.systemctl.log" ] || { echo 'KIS_DAILY_SELF_TEST: late apply invoked systemd' >&2; exit 1; }
short_commit=${commit:0:7}
[ ! -e "$test_root/systemd/lagrange-kis-daily-$short_commit.service" ]
[ ! -e "$test_root/systemd/lagrange-kis-daily-$short_commit.timer" ]

run_disabled_install() {
  local fixture_systemd_dir=$1 fixture_log=$2 fixture_reload=$3 fixture_chown=$4
  local fake_uid=$5 fake_mode=$6
  shift 6
  mkdir -p "$fixture_systemd_dir"
  KIS_DAILY_SELF_TEST_SYSTEMD_DIR="$fixture_systemd_dir" \
  KIS_DAILY_SELF_TEST_SYSTEMD_LOG="$fixture_log" \
  KIS_DAILY_SELF_TEST_RELOAD_MARKER="$fixture_reload" \
  KIS_DAILY_SELF_TEST_CHOWN_LOG="$fixture_chown" \
  KIS_DAILY_SELF_TEST_SYSTEMD_MODE="$fake_mode" \
  KIS_DAILY_SELF_TEST_UID="$fake_uid" \
  KIS_DAILY_SELF_TEST_NOW_HMS=164500 \
  KIS_DAILY_TIMER_TEST_NOW=not-a-time \
  PATH="$test_root/fake-bin:/usr/bin:/bin" \
    bash "$installer" --install-disabled \
      --release-root "$release" \
      --code-commit "$commit" \
      --env-file "$test_root/production.env" \
      --calendar-dir "$test_root/calendar" \
      --state-dir "$test_root/state" \
      --lock-file "$test_root/state/kis-daily.lock" \
      --systemd-dir "$fixture_systemd_dir" "$@"
}

expect_disabled_failure() {
  local label=$1 fixture_systemd_dir=$2 fixture_log=$3 fixture_reload=$4
  local fixture_chown=$5 fake_uid=$6 fake_mode=$7 expected=$8
  shift 8
  local output="$test_root/$label.out" error="$test_root/$label.err" rc staged
  set +e
  run_disabled_install "$fixture_systemd_dir" "$fixture_log" "$fixture_reload" \
    "$fixture_chown" "$fake_uid" "$fake_mode" "$@" >"$output" 2>"$error"
  rc=$?
  set -e
  [ "$rc" -eq 1 ] || { echo "KIS_DAILY_SELF_TEST: $label expected rc=1, got $rc" >&2; exit 1; }
  grep -Fq -- "$expected" "$error" || {
    echo "KIS_DAILY_SELF_TEST: $label missing typed failure $expected" >&2
    exit 1
  }
  for staged in "$fixture_systemd_dir"/.lagrange-kis-daily.*; do
    [ ! -e "$staged" ] && [ ! -L "$staged" ] || {
      echo "KIS_DAILY_SELF_TEST: $label left a staged unit" >&2
      exit 1
    }
  done
}

disabled_dir="$test_root/systemd-disabled"
disabled_log="$test_root/disabled.systemctl.log"
disabled_reload="$test_root/disabled.reloaded"
disabled_chown="$test_root/disabled.chown.log"
run_disabled_install "$disabled_dir" "$disabled_log" "$disabled_reload" \
  "$disabled_chown" 0 normal >"$test_root/disabled-install.out" \
  2>"$test_root/disabled-install.err"
grep -Fq 'KIS_DAILY_INSTALL_DISABLED: PASS' "$test_root/disabled-install.out"
[ -e "$disabled_reload" ] || { echo 'KIS_DAILY_SELF_TEST: disabled install did not reload fake systemd' >&2; exit 1; }
service_unit="$disabled_dir/lagrange-kis-daily-$short_commit.service"
timer_unit="$disabled_dir/lagrange-kis-daily-$short_commit.timer"
[ -f "$service_unit" ] && [ ! -L "$service_unit" ]
[ -f "$timer_unit" ] && [ ! -L "$timer_unit" ]
[ "$(/usr/bin/stat -c '%a' -- "$service_unit")" = 644 ]
[ "$(/usr/bin/stat -c '%a' -- "$timer_unit")" = 644 ]
[ "$(find "$disabled_dir" -maxdepth 1 -type f | wc -l)" -eq 2 ] || {
  echo 'KIS_DAILY_SELF_TEST: disabled install wrote unexpected files' >&2
  exit 1
}
{
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.service"
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.timer"
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.service"
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.timer"
  printf 'daemon-reload\n'
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.service"
  printf 'show --no-pager --property=LoadState,ActiveState,UnitFileState %s\n' "lagrange-kis-daily-$short_commit.timer"
} >"$test_root/disabled.expected-actions"
cmp -s "$test_root/disabled.expected-actions" "$disabled_log" || {
  echo 'KIS_DAILY_SELF_TEST: disabled install made an unexpected systemctl call' >&2
  exit 1
}
cp -- "$disabled_log" "$test_root/disabled.before-check.log"
KIS_DAILY_SELF_TEST_SYSTEMD_DIR="$disabled_dir" \
KIS_DAILY_SELF_TEST_SYSTEMD_LOG="$disabled_log" \
KIS_DAILY_SELF_TEST_RELOAD_MARKER="$disabled_reload" \
KIS_DAILY_SELF_TEST_CHOWN_LOG="$disabled_chown" \
PATH="$test_root/fake-bin:/usr/bin:/bin" \
  bash "$installer" --check \
    --release-root "$release" \
    --code-commit "$commit" \
    --env-file "$test_root/production.env" \
    --calendar-dir "$test_root/calendar" \
    --state-dir "$test_root/state" \
    --lock-file "$test_root/state/kis-daily.lock" \
    --systemd-dir "$disabled_dir" >"$test_root/disabled-check.out"
grep -Fq 'KIS_DAILY_INSTALL_CHECK: PASS' "$test_root/disabled-check.out"
cmp -s "$test_root/disabled.before-check.log" "$disabled_log" || {
  echo 'KIS_DAILY_SELF_TEST: --check unexpectedly called systemctl' >&2
  exit 1
}

nonroot_dir="$test_root/systemd-nonroot"
expect_disabled_failure nonroot "$nonroot_dir" "$test_root/nonroot.systemctl.log" \
  "$test_root/nonroot.reloaded" "$test_root/nonroot.chown.log" 1000 normal \
  '--install-disabled must run as root'
[ ! -s "$test_root/nonroot.systemctl.log" ] || {
  echo 'KIS_DAILY_SELF_TEST: nonroot check called systemctl' >&2
  exit 1
}

target_dir="$test_root/systemd-existing-target"
mkdir -p "$target_dir"
printf 'existing unit sentinel\n' >"$target_dir/lagrange-kis-daily-$short_commit.service"
expect_disabled_failure existing-target "$target_dir" "$test_root/existing-target.systemctl.log" \
  "$test_root/existing-target.reloaded" "$test_root/existing-target.chown.log" 0 normal \
  'KIS_DAILY_INSTALL_DISABLED_TARGET_COLLISION'
[ "$(<"$target_dir/lagrange-kis-daily-$short_commit.service")" = 'existing unit sentinel' ]
[ ! -s "$test_root/existing-target.systemctl.log" ]

symlink_dir="$test_root/systemd-existing-symlink"
mkdir -p "$symlink_dir"
ln -s "$test_root/missing-unit-target" "$symlink_dir/lagrange-kis-daily-$short_commit.timer"
expect_disabled_failure existing-symlink "$symlink_dir" "$test_root/existing-symlink.systemctl.log" \
  "$test_root/existing-symlink.reloaded" "$test_root/existing-symlink.chown.log" 0 normal \
  'KIS_DAILY_INSTALL_DISABLED_TARGET_COLLISION'
[ -L "$symlink_dir/lagrange-kis-daily-$short_commit.timer" ]
[ ! -s "$test_root/existing-symlink.systemctl.log" ]

replace_dir="$test_root/systemd-replace-existing"
expect_disabled_failure replace-existing "$replace_dir" "$test_root/replace-existing.systemctl.log" \
  "$test_root/replace-existing.reloaded" "$test_root/replace-existing.chown.log" 0 normal \
  '--replace-existing is not allowed with --install-disabled' --replace-existing
[ ! -s "$test_root/replace-existing.systemctl.log" ]

for dependency_kind in wants requires upholds; do
  for unit_kind in service timer; do
    label="existing-$dependency_kind-$unit_kind"
    dependency_dir="$test_root/systemd-$label"
    unit_name="lagrange-kis-daily-$short_commit.$unit_kind"
    dependency_target="$dependency_dir/$unit_name"
    dependency_link="$dependency_dir/multi-user.target.$dependency_kind/$unit_name"
    mkdir -p "$dependency_dir/multi-user.target.$dependency_kind"
    ln -s "$dependency_target" "$dependency_link"
    expect_disabled_failure "$label" "$dependency_dir" "$test_root/$label.systemctl.log" \
      "$test_root/$label.reloaded" "$test_root/$label.chown.log" 0 normal \
      'KIS_DAILY_INSTALL_DISABLED_UNIT_COLLISION'
    [ -L "$dependency_link" ] && [ "$(readlink -- "$dependency_link")" = "$dependency_target" ]
    [ ! -e "$dependency_dir/lagrange-kis-daily-$short_commit.service" ]
    [ ! -e "$dependency_dir/lagrange-kis-daily-$short_commit.timer" ]
    [ ! -s "$test_root/$label.systemctl.log" ]
    [ ! -s "$test_root/$label.chown.log" ]
    [ ! -e "$test_root/$label.reloaded" ]
    [ -z "$(find "$dependency_dir" -maxdepth 1 -name '.lagrange-kis-daily.*' -print)" ]
  done
done

for fake_mode in loaded-collision active-collision enabled-collision malformed duplicate missing extra-blank query-failure; do
  collision_dir="$test_root/systemd-$fake_mode"
  case "$fake_mode" in
    *collision) expected='KIS_DAILY_INSTALL_DISABLED_UNIT_COLLISION' ;;
    query-failure) expected='KIS_DAILY_INSTALL_DISABLED_SYSTEMD_QUERY_FAILED' ;;
    *) expected='KIS_DAILY_INSTALL_DISABLED_SYSTEMD_PROPERTIES_INVALID' ;;
  esac
  expect_disabled_failure "$fake_mode" "$collision_dir" \
    "$test_root/$fake_mode.systemctl.log" "$test_root/$fake_mode.reloaded" \
    "$test_root/$fake_mode.chown.log" 0 "$fake_mode" "$expected"
  [ ! -e "$collision_dir/lagrange-kis-daily-$short_commit.service" ]
  [ ! -e "$collision_dir/lagrange-kis-daily-$short_commit.timer" ]
  [ ! -e "$test_root/$fake_mode.reloaded" ]
  if grep -Fq 'daemon-reload' "$test_root/$fake_mode.systemctl.log"; then
    echo "KIS_DAILY_SELF_TEST: $fake_mode collision reached systemd reload" >&2
    exit 1
  fi
done

post_dir="$test_root/systemd-postcheck"
expect_disabled_failure postcheck "$post_dir" "$test_root/postcheck.systemctl.log" \
  "$test_root/postcheck.reloaded" "$test_root/postcheck.chown.log" 0 post-enabled \
  'KIS_DAILY_INSTALL_DISABLED_POSTCHECK_FAILED'
[ -f "$post_dir/lagrange-kis-daily-$short_commit.service" ]
[ -f "$post_dir/lagrange-kis-daily-$short_commit.timer" ]
[ -e "$test_root/postcheck.reloaded" ] || {
  echo 'KIS_DAILY_SELF_TEST: postcheck fixture did not reach reload' >&2
  exit 1
}
if grep -Eq '^(enable|start|stop|restart|disable)( |$)' "$test_root/postcheck.systemctl.log"; then
  echo 'KIS_DAILY_SELF_TEST: postcheck used a forbidden activation command' >&2
  exit 1
fi

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
bash "$script_dir/backfill-production-self-test.sh"
echo 'KIS_DAILY_SELF_TEST: PASS (syntax, provider-free plan, immutable release unit paths, late apply gate, lock/calendar/DB/worker contracts)'
