#!/usr/bin/env bash
# Focused provider-free tests for the shared KIS read Compose wiring.
set -euo pipefail

unset OWNER_INTRADAY_QUOTES_MODE KIS_READ_COORDINATION_MODE \
  OWNER_INTRADAY_SESSION_WINDOWS_SOURCE OWNER_INTRADAY_QUOTE_TRANSPORT \
  OWNER_MARKET_STREAM_ORIGIN KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID \
  KIS_MARKET_STREAM_GRANT_ID KIS_MARKET_STREAM_CONTRACT_SHA256 \
  DB_APP_MAX_CONNECTIONS DB_APP_MAX_CONNECTIONS_FILE

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
trap 'status=$?; if [ "$status" -eq 0 ]; then rm -rf -- "$tmp"; else printf "fixture retained: %s\n" "$tmp" >&2; fi' EXIT

source "$dotenv"
source "$helper"

fixture_root="$tmp/source root with whitespace"
compose_dir=$fixture_root/deploy/compose
mkdir -p "$compose_dir"
printf '%s\n' 'services: {}' >"$compose_dir/compose.yml"
intraday_overlay=$compose_dir/compose.intraday.yml
operational_overlay=$compose_dir/compose.intraday-operational.yml
market_stream_overlay=$compose_dir/compose.market-stream.yml
stream_settings=(
  'OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws'
  'OWNER_MARKET_STREAM_ORIGIN=https://quotes.example'
  'KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID=00000000-0000-4000-8000-000000000001'
  'KIS_MARKET_STREAM_GRANT_ID=00000000-0000-4000-8000-000000000002'
  'KIS_MARKET_STREAM_CONTRACT_SHA256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
)

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

# Active quotes require the protected dotenv to select WS explicitly. Neither
# the historical REST default nor a matching shell value supplies that choice.
for transport in missing rest invalid; do
  write_env "$tmp/owner-$transport.env" \
    'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
    'KIS_READ_COORDINATION_MODE=shared_required'
  if [ "$transport" != missing ]; then
    printf 'OWNER_INTRADAY_QUOTE_TRANSPORT=%s\n' "$transport" >>"$tmp/owner-$transport.env"
  fi
  expected=owner_intraday_quotes_requires_explicit_market_ws
  [ "$transport" != invalid ] || expected=owner_intraday_quote_transport_invalid
  expect_error "$tmp/owner-$transport.env" "$expected"
done
OWNER_INTRADAY_QUOTE_TRANSPORT=rest
expect_error "$tmp/owner-missing.env" owner_intraday_quotes_requires_explicit_market_ws
unset OWNER_INTRADAY_QUOTE_TRANSPORT
write_env "$tmp/off-rest.env" 'OWNER_INTRADAY_QUOTES_MODE=off' 'OWNER_INTRADAY_QUOTE_TRANSPORT=rest'
configure_case "$tmp/off-rest.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 0 ] || fail 'off/rest selected an overlay'
printf '%s\n' 'services: {}' >"$market_stream_overlay"

write_env "$tmp/shared-release-owner.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=release_v1' "${stream_settings[@]}"
configure_case "$tmp/shared-release-owner.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 4 ] ||
  fail 'release_v1 owner-only mode selected an unexpected artifact'
[ "${KIS_READ_COMPOSE_FILE_ARGS[3]}" = "$market_stream_overlay" ] || fail 'WS overlay was not last'

printf '%s\n' 'services: {}' >"$operational_overlay"
write_env "$tmp/shared-operational-owner.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1' "${stream_settings[@]}"
configure_case "$tmp/shared-operational-owner.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 6 ] ||
  fail 'operational owner-only mode did not select three overlays'
[ "${KIS_READ_COMPOSE_FILE_ARGS[1]}" = "$intraday_overlay" ] ||
  fail 'intraday overlay was not first'
[ "${KIS_READ_COMPOSE_FILE_ARGS[2]}" = -f ] || fail 'operational argument flag is not -f'
[ "${KIS_READ_COMPOSE_FILE_ARGS[3]}" = "$operational_overlay" ] ||
  fail 'operational overlay was not second'
[ "${KIS_READ_COMPOSE_FILE_ARGS[5]}" = "$market_stream_overlay" ] || fail 'WS overlay was not last'

write_env "$tmp/shared-operational-off.env" \
  'OWNER_INTRADAY_QUOTES_MODE=off' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1'
configure_case "$tmp/shared-operational-off.env"
[ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -eq 2 ] ||
  fail 'operational source with quotes off unexpectedly selected its overlay'

write_env "$tmp/owner-without-shared.env" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=legacy' "${stream_settings[@]}"
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
printf '%s\n' 'services: {}' >"$release_root/deploy/compose/compose.market-stream.yml"
refresh_env=$release_root/deploy/compose/.env
write_env "$refresh_env" \
  'LAGRANGE_DATA_DIR=/tmp/fixture-data' \
  "LAGRANGE_CODE_COMMIT=$commit" \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1' \
  'OWNER_EQUITY_V2_RUNTIME_MODE=owner_only' "${stream_settings[@]}"
chmod 0600 "$refresh_env"

write_manifest() {
  local path=$1 manifest_commit=${2:-$commit} service index=${3:-0} image_id
  {
    printf '%s\n' LAGRANGE_RELEASE_MANIFEST_V2
    printf 'commit|%s\n' "$manifest_commit"
    for service in db-role-bootstrap db-migrate api-server web research-worker \
      recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner \
      nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
      index=$((index + 1))
      image_id=$(printf 'sha256:%064d' "$index")
      printf 'image|%s|lagrange-station-%s:%s|%s|%s\n' \
        "$service" "$service" "$manifest_commit" "$image_id" "$manifest_commit"
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
  printf 'sha256:%064d' "$((index + ${FAKE_IMAGE_OFFSET:-0}))"
}

if [ "${1:-}" = compose ]; then
  command_name=
  for argument in "$@"; do
    case "$argument" in
      version|config|ps|up) command_name=$argument; break ;;
    esac
  done
  case "$command_name" in
    version) [ "${FAKE_COMPOSE_UNAVAILABLE:-0}" != 1 ]; exit $? ;;
    config) exit 0 ;;
    ps)
      if [[ " $* " == *' -q '* || " $* " == *' -aq '* ]]; then
        service=${!#}
        [ "${FAKE_MISSING_SERVICE:-}" = "$service" ] && exit 0
        printf 'ctr-%s\n' "$service"
      fi
      exit 0
      ;;
    up)
      if [ -n "${FAKE_PREVIOUS_COMMIT:-}" ]; then
        override=
        for argument in "$@"; do
          [[ "$argument" == */.release-image-override.* ]] && override=$argument
        done
        [ -n "$override" ] || exit 96
        grep -Fq -- "$(image_id_for_service "${!#}")" "$override" || exit 96
        : >"${FAKE_TRANSITION_STATE:?}/${!#}"
      fi
      exit 0
      ;;
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
  revision=${FAKE_COMMIT:?}
  if [ -n "${FAKE_PREVIOUS_COMMIT:-}" ] &&
      [ ! -e "${FAKE_TRANSITION_STATE:?}/$service" ] &&
      [ "${FAKE_MIXED_SERVICE:-}" != "$service" ]; then
    FAKE_IMAGE_OFFSET=100
    revision=$FAKE_PREVIOUS_COMMIT
  fi
  image_id=$(image_id_for_service "$service")
  [ "${FAKE_MISMATCH_SERVICE:-}" = "$service" ] &&
    image_id=sha256:9999999999999999999999999999999999999999999999999999999999999999
  printf '%s|%s\n' "$image_id" "$revision"
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

# Missing, REST, and invalid active transports reject before even the fake
# Docker CLI or an immutable image override can be reached.
for transport in missing rest invalid; do
  sed '/^OWNER_INTRADAY_QUOTE_TRANSPORT=/d' "$tmp/refresh-env.saved" >"$refresh_env"
  if [ "$transport" != missing ]; then
    printf 'OWNER_INTRADAY_QUOTE_TRANSPORT=%s\n' "$transport" >>"$refresh_env"
  fi
  : >"$docker_log"
  if run_refresh "$tmp/rejected-transport.out" bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --apply; then
    fail 'active refresh accepted missing, REST or invalid transport'
  fi
  expected=owner_intraday_quotes_requires_explicit_market_ws
  [ "$transport" != invalid ] || expected=owner_intraday_quote_transport_invalid
  grep -Fq "$expected" "$tmp/rejected-transport.out"
  [ ! -s "$docker_log" ] || fail 'invalid active transport reached Docker'
  ! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .
done
cp "$tmp/refresh-env.saved" "$refresh_env"

mv "$manifest" "$tmp/manifest.saved"
: >"$docker_log"
if run_refresh "$tmp/missing-manifest.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --preflight; then
  fail 'refresh accepted a missing immutable manifest'
fi
grep -Fq 'installed-release-manifest must be a regular non-symlink file' \
  "$tmp/missing-manifest.out"
! grep -Fq 'compose config' "$docker_log"
mv "$tmp/manifest.saved" "$manifest"

: >"$docker_log"
run_refresh "$tmp/refresh-plan.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --plan
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM_ORDER:' "$tmp/refresh-plan.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
run_refresh "$tmp/refresh-preflight.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --preflight
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM_PREFLIGHT: PASS' "$tmp/refresh-preflight.out"
! grep -Eq '^compose .* up ' "$docker_log"

: >"$docker_log"
if run_refresh "$tmp/missing-running.out" env FAKE_MISSING_SERVICE=owner-equity-v2-runner \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --apply; then
  fail 'refresh accepted a missing currently running service'
fi
grep -Fq 'persistent service did not resolve to exactly one container: owner-equity-v2-runner' \
  "$tmp/missing-running.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
if run_refresh "$tmp/mismatched-running.out" env FAKE_MISMATCH_SERVICE=owner-equity-v2-runner \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --apply; then
  fail 'refresh accepted a mismatched currently running service'
fi
grep -Fq 'persistent service image_id mismatch: owner-equity-v2-runner' \
  "$tmp/mismatched-running.out"
! grep -Eq '^compose .* up ' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

: >"$docker_log"
run_refresh "$tmp/refresh-apply.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --apply
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM: PASS' "$tmp/refresh-apply.out"
mapfile -t refresh_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#refresh_ups[@]}" -eq 3 ] || fail 'refresh did not issue exactly three Compose up calls'
expected_order="-f $release_root/deploy/compose/compose.yml -f $release_root/deploy/compose/compose.intraday.yml -f $release_root/deploy/compose/compose.intraday-operational.yml -f $release_root/deploy/compose/compose.market-stream.yml -f $install_root/.release-image-override."
case "${refresh_ups[0]}" in
  *"$expected_order"*) ;;
  *) fail "refresh Compose file order was not base/intraday/operational/WS/override: ${refresh_ups[0]}" ;;
esac
case "${refresh_ups[0]}" in
  *'--no-build --pull never --no-deps --force-recreate --wait api-server') ;;
  *) fail "API refresh command was not narrow or ordered: ${refresh_ups[0]}" ;;
esac
case "${refresh_ups[1]}" in
  *'--no-build --pull never --no-deps --force-recreate --wait web') ;;
  *) fail "Web refresh command was not narrow or ordered: ${refresh_ups[1]}" ;;
esac
case "${refresh_ups[2]}" in
  *'--no-build --pull never --no-deps --force-recreate --wait owner-equity-v2-runner') ;;
  *) fail "owner V2 refresh command was not narrow or ordered: ${refresh_ups[2]}" ;;
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
run_refresh "$tmp/calendar-reuse-plan.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --reuse-existing-source --plan
grep -Fq 'CALENDAR_BOOTSTRAP_PLAN: date=' "$tmp/calendar-reuse-plan.out"
grep -Fq 'mode=reuse-existing-source' "$tmp/calendar-reuse-plan.out"
! grep -Eq '^compose .* (run|up|build|stop)( |$)' "$docker_log"
if run_refresh "$tmp/calendar-conflicting-flags.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --calendar-source-batch-id "$calendar_batch" \
  --reuse-existing-source --plan; then
  fail 'calendar bootstrap accepted both source-selection modes'
fi
grep -Fq 'either an explicit source UUID or --reuse-existing-source' \
  "$tmp/calendar-conflicting-flags.out"
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

: >"$docker_log"
run_refresh "$tmp/calendar-reuse-apply.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --bootstrap-intraday-calendar --reuse-existing-source --apply
grep -Fq 'CALENDAR_BOOTSTRAP: PASS' "$tmp/calendar-reuse-apply.out"
mapfile -t calendar_reuse_runs < <(grep '^compose .* run ' "$docker_log")
[ "${#calendar_reuse_runs[@]}" -eq 1 ] || fail 'calendar reuse did not run exactly one command'
case "${calendar_reuse_runs[0]}" in
  *"$expected_order"*'run --rm --no-deps research-worker --calendar-once --date '*" --reuse-existing-source") ;;
  *) fail 'calendar reuse did not use the exact immutable reuse command' ;;
esac
! grep -Fq -- '--source-batch-id' <<<"${calendar_reuse_runs[0]}"
! grep -Eq '^compose .* (up|build|stop|down)( |$)' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

# A WS rollout refresh includes Web and requires a separate acknowledgement.
# Every command below still reaches only the task-owned fake Docker executable.
cp "$tmp/refresh-env.saved" "$refresh_env"
: >"$docker_log"
run_refresh "$tmp/ws-plan.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --plan
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM_ORDER:' "$tmp/ws-plan.out"
! grep -Eq '^compose .* (up|run|build|stop|down)( |$)' "$docker_log"

: >"$docker_log"
if run_refresh "$tmp/ws-legacy-refresh.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --plan; then
  fail 'legacy REST refresh accepted market WS transport'
fi
grep -Fq 'market_stream_requires_refresh_market_stream' "$tmp/ws-legacy-refresh.out"
! grep -Eq '^compose .* (up|run|build|stop|down)( |$)' "$docker_log"

: >"$docker_log"
if run_refresh "$tmp/ws-missing-confirm.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --apply; then
  fail 'REST acknowledgement alone activated market WS'
fi
grep -Fq 'owner_market_stream_rollout_confirmation_required' "$tmp/ws-missing-confirm.out"
! grep -Eq '^compose .* (up|run|build|stop|down)( |$)' "$docker_log"

: >"$docker_log"
run_refresh "$tmp/ws-apply.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --apply
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM: PASS' "$tmp/ws-apply.out"
mapfile -t ws_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#ws_ups[@]}" -eq 3 ] || fail 'market WS refresh must recreate exactly API, Web and owner V2'
targets=(api-server web owner-equity-v2-runner)
for index in 0 1 2; do
  case "${ws_ups[$index]}" in
    *"-f $release_root/deploy/compose/compose.market-stream.yml -f $install_root/.release-image-override."*"--no-build --pull never --no-deps --force-recreate --wait ${targets[$index]}") ;;
    *) fail 'market WS refresh violated fixed overlay or startup order' ;;
  esac
done
! grep -Eq '^compose .* (build|run|down|stop)( |$)' "$docker_log"
! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q .

# Explicit cross-release refresh binds old running containers to a separate
# trusted installed manifest, while every replacement still uses current.
previous_commit=89abcdef0123456789abcdef0123456789abcdef
previous_root=$install_root/releases/$previous_commit
transition_state=$tmp/transition-state
mkdir -p "$previous_root" "$transition_state"
write_manifest "$previous_root/.lagrange-release-manifest" "$previous_commit" 100
run_transition() {
  local output=$1
  shift
  run_refresh "$output" env FAKE_PREVIOUS_COMMIT="$previous_commit" \
    FAKE_TRANSITION_STATE="$transition_state" \
    OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
    OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS \
    "$@"
}
assert_no_transition_mutation() {
  ! grep -Eq '^compose .* (up|run|build|stop|down)( |$)' "$docker_log" ||
    fail 'rejected/read-only transition mutated a service'
  ! find "$install_root" -maxdepth 1 -name '.release-image-override.*' -print -quit | grep -q . ||
    fail 'rejected/read-only transition created an override'
}

for bad_commit in '' invalid ../escape 0000000000000000000000000000000000000000; do
  : >"$docker_log"
  if run_refresh "$tmp/transition-bad-commit.out" bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --refresh-from-commit "$bad_commit" --plan; then
    fail 'transition accepted an invalid source commit'
  fi
  assert_no_transition_mutation
done
: >"$docker_log"
if run_refresh "$tmp/transition-wrong-scope.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-intraday --refresh-from-commit "$previous_commit" --plan; then
  fail 'transition option was accepted outside market-stream refresh'
fi
assert_no_transition_mutation

: >"$docker_log"
if run_transition "$tmp/transition-unselected.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --apply; then
  fail 'old images were accepted without an explicit source commit'
fi
grep -Fq 'persistent service image_id mismatch:' "$tmp/transition-unselected.out"
assert_no_transition_mutation

mv "$previous_root/.lagrange-release-manifest" "$tmp/previous-manifest.saved"
: >"$docker_log"
if run_transition "$tmp/transition-missing-manifest.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" --apply; then
  fail 'transition accepted a missing previous manifest'
fi
grep -Fq 'refresh-source-manifest must be a regular non-symlink file' "$tmp/transition-missing-manifest.out"
assert_no_transition_mutation
ln -s "$tmp/previous-manifest.saved" "$previous_root/.lagrange-release-manifest"
: >"$docker_log"
if run_transition "$tmp/transition-symlink-manifest.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" --plan; then
  fail 'transition accepted a symlinked previous manifest'
fi
assert_no_transition_mutation
rm -- "$previous_root/.lagrange-release-manifest"
mv "$tmp/previous-manifest.saved" "$previous_root/.lagrange-release-manifest"

for control in FAKE_MISMATCH_SERVICE=web FAKE_MIXED_SERVICE=web; do
  : >"$docker_log"
  if run_transition "$tmp/transition-mismatched.out" env "$control" \
    bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" --apply; then
    fail 'transition accepted a foreign or mixed running image'
  fi
  grep -Fq 'persistent service image_id mismatch: web' "$tmp/transition-mismatched.out"
  assert_no_transition_mutation
done

for inspection in plan preflight; do
  : >"$docker_log"
  run_transition "$tmp/transition-$inspection.out" bash "$release_root/scripts/ops/compose-release.sh" \
    --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" "--$inspection"
  assert_no_transition_mutation
done
: >"$docker_log"
run_transition "$tmp/transition-apply.out" bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" --apply
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM: PASS' "$tmp/transition-apply.out"
mapfile -t transition_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#transition_ups[@]}" -eq 3 ] || fail 'cross-release refresh did not recreate exactly three services'
for index in 0 1 2; do
  [[ "${transition_ups[$index]}" == *"--no-build --pull never --no-deps --force-recreate --wait ${targets[$index]}" ]] ||
    fail 'cross-release refresh changed startup order or build policy'
done

# Turning quotes off needs no WS pins or WS acknowledgement and still refreshes
# Web, so the prior browser transport is not left in the serving container.
write_env "$refresh_env" \
  'LAGRANGE_DATA_DIR=/tmp/fixture-data' "LAGRANGE_CODE_COMMIT=$commit" \
  'OWNER_INTRADAY_QUOTES_MODE=off' 'OWNER_INTRADAY_QUOTE_TRANSPORT=rest' \
  'KIS_READ_COORDINATION_MODE=shared_required' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1' \
  'OWNER_EQUITY_V2_RUNTIME_MODE=owner_only'
: >"$docker_log"
run_refresh "$tmp/ws-off.out" \
  env OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --apply
mapfile -t off_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#off_ups[@]}" -eq 3 ] || fail 'quotes-off rollback did not refresh the same three services'
! grep -Fq 'compose.market-stream.yml' "$docker_log"
! grep -Eq '^compose .* (build|run|down|stop)( |$)' "$docker_log"

rm -- "$transition_state/api-server" "$transition_state/web" "$transition_state/owner-equity-v2-runner"
: >"$docker_log"
run_refresh "$tmp/transition-off.out" env FAKE_PREVIOUS_COMMIT="$previous_commit" \
  FAKE_TRANSITION_STATE="$transition_state" \
  OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS \
  bash "$release_root/scripts/ops/compose-release.sh" \
  --scope release --refresh-market-stream --refresh-from-commit "$previous_commit" --apply
grep -Fq 'COMPOSE_REFRESH_MARKET_STREAM: PASS' "$tmp/transition-off.out"
mapfile -t transition_off_ups < <(grep '^compose .* up ' "$docker_log")
[ "${#transition_off_ups[@]}" -eq 3 ] || fail 'cross-release off rollback did not refresh all three services'
! grep -Fq 'compose.market-stream.yml' "$docker_log"
! grep -Eq '^compose .* (build|run|down|stop)( |$)' "$docker_log"

# Feature-off still checks the normal Compose prerequisite after safe metadata
# selection, while help exits without probing Docker or parsing activation data.
: >"$docker_log"
if run_refresh "$tmp/off-compose-unavailable.out" env FAKE_COMPOSE_UNAVAILABLE=1 \
  bash "$release_root/scripts/ops/compose-release.sh" --scope release --refresh-market-stream --plan; then
  fail 'off mode skipped the Compose prerequisite'
fi
grep -Fq 'Docker Compose v2 is unavailable' "$tmp/off-compose-unavailable.out"
[ "$(cat "$docker_log")" = 'compose version' ] || fail 'unavailable Compose reached another command'
cat >"$tmp/docker-missing.bash" <<'SH'
command() {
  if [ "$#" -eq 2 ] && [ "$1" = -v ] && [ "$2" = docker ]; then
    return 1
  fi
  builtin command "$@"
}
SH
: >"$docker_log"
if run_refresh "$tmp/off-docker-missing.out" env BASH_ENV="$tmp/docker-missing.bash" \
  bash "$release_root/scripts/ops/compose-release.sh" --scope release --refresh-market-stream --plan; then
  fail 'off mode accepted a missing Docker prerequisite'
fi
grep -Fq 'docker is not installed' "$tmp/off-docker-missing.out"
[ ! -s "$docker_log" ] || fail 'missing Docker reached an engine command'
: >"$docker_log"
run_refresh "$tmp/help.out" bash "$release_root/scripts/ops/compose-release.sh" --help
[ ! -s "$docker_log" ] || fail 'help probed Docker'

printf 'KIS_READ_COMPOSE_SELF_TEST: PASS (provider-free helper, WS-only refresh and off rollback fixtures)\n'
