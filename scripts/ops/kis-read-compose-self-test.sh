#!/usr/bin/env bash
# Focused provider-free tests for the shared KIS read Compose wiring.
set -euo pipefail

unset OWNER_INTRADAY_QUOTES_MODE KIS_READ_COORDINATION_MODE \
  OWNER_INTRADAY_SESSION_WINDOWS_SOURCE

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
helper=$script_dir/lib/kis-read-compose.sh
dotenv=$script_dir/lib/dotenv.sh
compose_release=$script_dir/compose-release.sh

fail() {
  printf 'kis-read-compose-self-test: %s\n' "$*" >&2
  exit 1
}

[ -f "$helper" ] || fail 'helper is missing'
bash -n "$helper" "$compose_release"

for entrypoint in \
  compose-release.sh \
  backfill-production.sh \
  kis-range-raw-backfill.sh \
  kis-action-range-raw-backfill.sh \
  kis-stock-price-beta-raw.sh \
  kis-stock-price-beta-raw-with-worker-pause.sh; do
  path=$script_dir/$entrypoint
  [ -x "$path" ] || fail "entrypoint is not executable: $entrypoint"
  grep -Fq 'source "$script_dir/lib/kis-read-compose.sh"' "$path" ||
    fail "entrypoint does not source the shared helper: $entrypoint"
  grep -Fq 'kis_read_compose_configure' "$path" ||
    fail "entrypoint does not configure the shared helper: $entrypoint"
  grep -Fq 'KIS_READ_COMPOSE_FILE_ARGS' "$path" ||
    fail "entrypoint does not use the shared helper output: $entrypoint"
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-kis-read-compose.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

source "$dotenv"
source "$helper"

fixture_root="$tmp/source root with whitespace"
compose_dir=$fixture_root/deploy/compose
mkdir -p "$compose_dir"
printf '%s\n' 'services: {}' >"$compose_dir/compose.yml"
intraday_overlay=$compose_dir/compose.intraday.yml
operational_overlay=$compose_dir/compose.intraday-operational.yml

write_env() {
  local path=$1
  shift
  printf '%s\n' "$@" >"$path"
}

configure_case() {
  local env_file=$1
  dotenv_load "$env_file" || fail "fixture dotenv did not parse: $env_file"
  kis_read_compose_configure "$fixture_root" ||
    fail "helper rejected expected configuration: $KIS_READ_COMPOSE_ERROR"
}

expect_error() {
  local env_file=$1 expected=$2
  dotenv_load "$env_file" || fail "fixture dotenv did not parse: $env_file"
  if kis_read_compose_configure "$fixture_root"; then
    fail "helper accepted invalid configuration: $expected"
  fi
  [ "$KIS_READ_COMPOSE_ERROR" = "$expected" ] ||
    fail "expected $expected, got $KIS_READ_COMPOSE_ERROR"
}

# The legacy/off defaults do not inspect either artifact. There are no overlay
# files at this point, so this also proves the default path has no artifact
# prerequisite.
default_env=$tmp/default.env
write_env "$default_env" 'LAGRANGE_DATA_DIR=/tmp/fixture-data'
configure_case "$default_env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 0 ] ||
  fail 'legacy/off defaults unexpectedly selected an overlay'

printf '%s\n' 'services: {}' >"$intraday_overlay"
write_env "$tmp/shared-off.env" \
  'OWNER_INTRADAY_QUOTES_MODE=off' \
  'KIS_READ_COORDINATION_MODE=shared_required'
configure_case "$tmp/shared-off.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 2 ] ||
  fail 'shared coordination did not select one intraday overlay'
[ "${KIS_READ_COMPOSE_FILE_ARGS[0]}" = -f ] || fail 'intraday argument flag is not -f'
[ "${KIS_READ_COMPOSE_FILE_ARGS[1]}" = "$intraday_overlay" ] ||
  fail 'intraday overlay path was not derived from the source root'

write_env "$tmp/shared-release-owner.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=release_v1'
configure_case "$tmp/shared-release-owner.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 2 ] ||
  fail 'release_v1 owner-only mode selected an unexpected artifact'

printf '%s\n' 'services: {}' >"$operational_overlay"
write_env "$tmp/shared-operational-owner.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1'
configure_case "$tmp/shared-operational-owner.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 4 ] ||
  fail 'operational owner-only mode did not select two overlays'
[ "${KIS_READ_COMPOSE_FILE_ARGS[1]}" = "$intraday_overlay" ] ||
  fail 'intraday overlay was not first'
[ "${KIS_READ_COMPOSE_FILE_ARGS[2]}" = -f ] || fail 'operational argument flag is not -f'
[ "${KIS_READ_COMPOSE_FILE_ARGS[3]}" = "$operational_overlay" ] ||
  fail 'operational overlay was not second'

write_env "$tmp/shared-operational-off.env" \
  'OWNER_INTRADAY_QUOTES_MODE=off' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1'
configure_case "$tmp/shared-operational-off.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 2 ] ||
  fail 'operational source with quotes off unexpectedly selected its overlay'

write_env "$tmp/owner-without-shared.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=legacy'
expect_error "$tmp/owner-without-shared.env" owner_intraday_quotes_requires_shared

write_env "$tmp/operational-with-legacy.env" \
  'OWNER_INTRADAY_QUOTES_MODE=off' \
  'KIS_READ_COORDINATION_MODE=legacy' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1'
expect_error "$tmp/operational-with-legacy.env" operational_session_windows_requires_shared

write_env "$tmp/unknown-quotes.env" 'OWNER_INTRADAY_QUOTES_MODE=unexpected'
expect_error "$tmp/unknown-quotes.env" owner_intraday_quotes_mode_invalid
write_env "$tmp/empty-quotes.env" 'OWNER_INTRADAY_QUOTES_MODE='
expect_error "$tmp/empty-quotes.env" owner_intraday_quotes_mode_invalid
write_env "$tmp/unknown-coordination.env" 'KIS_READ_COORDINATION_MODE=unexpected'
expect_error "$tmp/unknown-coordination.env" kis_read_coordination_mode_invalid
write_env "$tmp/empty-coordination.env" 'KIS_READ_COORDINATION_MODE='
expect_error "$tmp/empty-coordination.env" kis_read_coordination_mode_invalid
write_env "$tmp/unknown-source.env" 'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=unexpected'
expect_error "$tmp/unknown-source.env" owner_intraday_session_windows_source_invalid
write_env "$tmp/empty-source.env" 'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE='
expect_error "$tmp/empty-source.env" owner_intraday_session_windows_source_invalid

# Ambient shell mode keys must not override the parsed dotenv contract.
OWNER_INTRADAY_QUOTES_MODE=owner_only
expect_error "$default_env" owner_intraday_quotes_mode_shell_override_mismatch
unset OWNER_INTRADAY_QUOTES_MODE

# Refresh tests use a separate installed-release-shaped fixture and a fake
# Docker CLI. The fixture has no provider, daemon, DB, or credentials.
commit=0123456789abcdef0123456789abcdef01234567
install_root="$tmp/refresh-install"
release_root=$install_root/releases/$commit
fake_bin=$tmp/fake-bin
docker_log=$tmp/refresh-docker.log
mkdir -p "$release_root/scripts/ops/lib" "$release_root/deploy/compose" "$fake_bin"
ln -s "releases/$commit" "$install_root/current"
cp "$compose_release" "$release_root/scripts/ops/compose-release.sh"
cp "$dotenv" "$release_root/scripts/ops/lib/dotenv.sh"
cp "$helper" "$release_root/scripts/ops/lib/kis-read-compose.sh"
cp "$script_dir/lib/release-image-manifest.sh" \
  "$release_root/scripts/ops/lib/release-image-manifest.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
  >"$release_root/scripts/ops/validate-production-config.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
  >"$release_root/scripts/ops/provision-linux.sh"
chmod 0755 "$release_root/scripts/ops/compose-release.sh" \
  "$release_root/scripts/ops/validate-production-config.sh" \
  "$release_root/scripts/ops/provision-linux.sh"
printf '%s\n' 'services: {}' >"$release_root/deploy/compose/compose.yml"
printf '%s\n' 'services: {}' >"$release_root/deploy/compose/compose.intraday.yml"
printf '%s\n' 'services: {}' >"$release_root/deploy/compose/compose.intraday-operational.yml"
refresh_env=$release_root/deploy/compose/.env
write_env "$refresh_env" \
  'LAGRANGE_DATA_DIR=/tmp/fixture-data' \
  "LAGRANGE_CODE_COMMIT=$commit" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1' \
  'OWNER_EQUITY_V2_RUNTIME_MODE=owner_only'
chmod 0600 "$refresh_env"

write_manifest() {
  local path=$1 service index=0 image_id
  {
    printf '%s\n' LAGRANGE_RELEASE_MANIFEST_V2
    printf 'commit|%s\n' "$commit"
    for service in db-role-bootstrap db-migrate api-server web research-worker \
      recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner \
      nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
      index=$((index + 1))
      image_id=$(printf 'sha256:%064d' "$index")
      printf 'image|%s|lagrange-station-%s:%s|%s|%s\n' \
        "$service" "$service" "$commit" "$image_id" "$commit"
    done
  } >"$path"
  chmod 0600 "$path"
}
manifest=$release_root/.lagrange-release-manifest
write_manifest "$manifest"

cat >"$fake_bin/id" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = -u ] && printf '0\n'
SH
cat >"$fake_bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
format=
previous=
for argument in "$@"; do
  if [ "$previous" = -c ]; then
    format=$argument
    previous=
    continue
  fi
  previous=$argument
done
case "$format" in
  %u:%a) printf '0:755\n' ;;
  %u:%g:%a) printf '0:0:600\n' ;;
  *) exec /usr/bin/stat "$@" ;;
esac
SH
cat >"$fake_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${FAKE_DOCKER_LOG:?}"

image_id_for_service() {
  local service=$1 index
  case "$service" in
    db-role-bootstrap) index=1 ;;
    db-migrate) index=2 ;;
    api-server) index=3 ;;
    web) index=4 ;;
    research-worker) index=5 ;;
    recommendation-runner) index=6 ;;
    candidate-runner) index=7 ;;
    owner-beta-runner) index=8 ;;
    owner-equity-v2-runner) index=9 ;;
    nt-backtest-worker-1) index=10 ;;
    nt-backtest-worker-2) index=11 ;;
    paper-scheduler) index=12 ;;
    *) return 1 ;;
  esac
  printf 'sha256:%064d' "$index"
}

if [ "${1:-}" = compose ]; then
  command_name=
  for argument in "$@"; do
    case "$argument" in
      version|config|ps|up) command_name=$argument; break ;;
    esac
  done
  case "$command_name" in
    version|config) exit 0 ;;
    ps)
      if [[ " $* " == *' -q '* || " $* " == *' -aq '* ]]; then
        service=${!#}
        [ "${FAKE_MISSING_SERVICE:-}" = "$service" ] && exit 0
        printf 'ctr-%s\n' "$service"
      fi
      exit 0
      ;;
    up) exit 0 ;;
    *) exit 0 ;;
  esac
fi

if [ "${1:-}" = image ] && [ "${2:-}" = inspect ]; then
  target=${!#}
  printf '%s|%s\n' "$target" "${FAKE_COMMIT:?}"
  exit 0
fi

if [ "${1:-}" = inspect ]; then
  if [[ " $* " == *'{{.State.Status}}'* ]]; then
    printf '%s\n' "${FAKE_WORKER_STATUS:-exited}"
    exit 0
  fi
  target=${!#}
  service=${target#ctr-}
  image_id=$(image_id_for_service "$service")
  [ "${FAKE_MISMATCH_SERVICE:-}" = "$service" ] &&
    image_id=sha256:9999999999999999999999999999999999999999999999999999999999999999
  printf '%s|%s\n' "$image_id" "${FAKE_COMMIT:?}"
  exit 0
fi

exit 97
SH
chmod 0755 "$fake_bin/id" "$fake_bin/stat" "$fake_bin/docker"

run_refresh() {
  local output=$1
  shift
  env PATH="$fake_bin:$PATH" \
    FAKE_DOCKER_LOG="$docker_log" FAKE_COMMIT="$commit" \
    LAGRANGE_RELEASE_ROOT="$install_root" \
    "$@" >"$output" 2>&1
}

if run_refresh "$tmp/wrong-scope.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope infrastructure --refresh-intraday --plan; then
  fail 'refresh accepted a non-release scope'
fi
grep -Fq -- '--refresh-intraday is valid only with --scope release' "$tmp/wrong-scope.out"

cp "$refresh_env" "$tmp/refresh-env.saved"
printf '%s\n' \
  'LAGRANGE_DATA_DIR=/tmp/fixture-data' \
  "LAGRANGE_CODE_COMMIT=$commit" \
  'OWNER_INTRADAY_QUOTES_MODE=unexpected' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1' \
  'OWNER_EQUITY_V2_RUNTIME_MODE=owner_only' >"$refresh_env"
chmod 0600 "$refresh_env"
: >"$docker_log"
if run_refresh "$tmp/invalid-mode.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --plan; then
  fail 'refresh accepted an invalid mode'
fi
grep -Fq 'owner_intraday_quotes_mode_invalid' "$tmp/invalid-mode.out"
! grep -Fq 'compose config' "$docker_log"
cp "$tmp/refresh-env.saved" "$refresh_env"
chmod 0600 "$refresh_env"

mv "$manifest" "$tmp/manifest.saved"
: >"$docker_log"
if run_refresh "$tmp/missing-manifest.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --preflight; then
  fail 'refresh accepted a missing immutable manifest'
fi
grep -Fq 'installed-release-manifest must be a regular non-symlink file' \
  "$tmp/missing-manifest.out"
! grep -Fq 'compose config' "$docker_log"
mv "$tmp/manifest.saved" "$manifest"

: >"$docker_log"
run_refresh "$tmp/refresh-plan.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --plan
grep -Fq 'COMPOSE_REFRESH_INTRADAY_ORDER:' "$tmp/refresh-plan.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
run_refresh "$tmp/refresh-preflight.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --preflight
grep -Fq 'COMPOSE_REFRESH_INTRADAY_PREFLIGHT: PASS' "$tmp/refresh-preflight.out"
! grep -Eq '^compose .* up ' "$docker_log"

: >"$docker_log"
if run_refresh "$tmp/missing-running.out" env FAKE_MISSING_SERVICE=owner-equity-v2-runner \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-intraday --apply; then
  fail 'refresh accepted a missing currently running service'
fi
grep -Fq 'persistent service did not resolve to exactly one container: owner-equity-v2-runner' \
  "$tmp/missing-running.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
if run_refresh "$tmp/mismatched-running.out" env FAKE_MISMATCH_SERVICE=owner-equity-v2-runner \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-intraday --apply; then
  fail 'refresh accepted a mismatched currently running service'
fi
grep -Fq 'persistent service image_id mismatch: owner-equity-v2-runner' \
  "$tmp/mismatched-running.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
run_refresh "$tmp/refresh-apply.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --apply
grep -Fq 'COMPOSE_REFRESH_INTRADAY: PASS' "$tmp/refresh-apply.out"
mapfile -t refresh_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#refresh_ups[@]}" -eq 2 ] || fail 'refresh did not issue exactly two Compose up calls'
expected_order="-f $release_root/deploy/compose/compose.yml -f $release_root/deploy/compose/compose.intraday.yml -f $release_root/deploy/compose/compose.intraday-operational.yml -f $install_root/.release-image-override."
case "${refresh_ups[0]}" in
  *"$expected_order"*) ;;
  *) fail "refresh Compose file order was not base/intraday/operational/override: ${refresh_ups[0]}" ;;
esac
case "${refresh_ups[0]}" in
  *'--no-build --pull never --no-deps --force-recreate --wait api-server') ;;
  *) fail "API refresh command was not narrow or ordered: ${refresh_ups[0]}" ;;
esac
case "${refresh_ups[1]}" in
  *'--no-build --pull never --no-deps --force-recreate --wait owner-equity-v2-runner') ;;
  *) fail "owner V2 refresh command was not narrow or ordered: ${refresh_ups[1]}" ;;
esac
! grep -Eq '^compose .* (build|run|down|stop)( |$)' "$docker_log"
if find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .; then
  fail 'refresh image override was not cleaned up'
fi

calendar_batch=00000000-0000-4000-8000-000000000001
: >"$docker_log"
run_refresh "$tmp/calendar-plan.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --calendar-source-batch-id "$calendar_batch" --plan
grep -Fq 'CALENDAR_BOOTSTRAP_PLAN:' "$tmp/calendar-plan.out"
! grep -Eq '^compose .* (run|up|build|stop)( |$)' "$docker_log"
: >"$docker_log"
if run_refresh "$tmp/calendar-running.out" env FAKE_WORKER_STATUS=restarting \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --calendar-source-batch-id "$calendar_batch" --apply; then
  fail 'calendar bootstrap accepted an active research daemon'
fi
grep -Fq 'calendar bootstrap requires the research daemon stopped' "$tmp/calendar-running.out"
! grep -Eq '^compose .* (run|up|build|stop)( |$)' "$docker_log"
: >"$docker_log"
run_refresh "$tmp/calendar-apply.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --calendar-source-batch-id "$calendar_batch" --apply
grep -Fq 'CALENDAR_BOOTSTRAP: PASS' "$tmp/calendar-apply.out"
mapfile -t calendar_runs < <(grep '^compose .* run ' "$docker_log")
[ "${#calendar_runs[@]}" -eq 1 ] || fail 'calendar bootstrap did not run exactly one command'
case "${calendar_runs[0]}" in
  *"$expected_order"*'run --rm --no-deps research-worker --calendar-once --date '*" --source-batch-id $calendar_batch") ;;
  *) fail 'calendar bootstrap did not use the exact immutable calendar-only command' ;;
esac
! grep -Eq '^compose .* (up|build|stop|down)( |$)' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

printf 'KIS_READ_COMPOSE_SELF_TEST: PASS (provider-free helper and refresh fixture)\n'
