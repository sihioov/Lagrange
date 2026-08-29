#!/usr/bin/env bash
# Safely pause only the exact ordinary research-worker for one fixed-stock
# KIS Raw capture. The inner fixed-stock wrapper remains the sole capture and
# Compose lifecycle owner.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
source "$script_dir/lib/dotenv.sh"

fixed_start=2025-08-04
fixed_end=2026-08-28
confirmation=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS
stop_timeout_secs=300
health_timeout_secs=180
health_poll_secs=5
commit=
env_file=
release_root=/opt/lagrange/current
release_root_seen=0
mode=plan
mode_seen=0
worker_id=
worker_was_stopped=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-stock-price-beta-raw-with-worker-pause.sh \
       [--plan] | --execute --commit 40HEX --env-file ABSOLUTE_PATH \
       [--release-root ABSOLUTE_PATH]

The default --plan is local: it does not read a production env, inspect
Docker, stop a container, or invoke KIS. --execute validates the requested
source and installed-release identities, then may pause only the exact running
lagrange-station/research-worker Compose container. It always restores that
same container after the synchronous fixed-stock Raw wrapper returns. The
release root defaults to /opt/lagrange/current; an override is accepted only
for a root-owned installed/test release root.
EOF
}

die() {
  printf 'kis-stock-price-beta-raw-with-worker-pause: %s\n' "$*" >&2
  exit 1
}

blocked() {
  printf 'BLOCKED_EXTERNAL: %s\n' "$*" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --commit)
      [ "$#" -ge 2 ] || die '--commit needs 40 lowercase hexadecimal characters'
      [ -z "$commit" ] || die '--commit must not be repeated'
      commit=$2
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs an absolute path'
      [ -z "$env_file" ] || die '--env-file must not be repeated'
      env_file=$2
      shift 2
      ;;
    --release-root)
      [ "$#" -ge 2 ] || die '--release-root needs an absolute path'
      [ "$release_root_seen" -eq 0 ] || die '--release-root must not be repeated'
      release_root=$2
      release_root_seen=1
      shift 2
      ;;
    --plan|--execute)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die 'must run as root'

if [ "$mode" = plan ]; then
  printf 'KIS_STOCK_PRICE_BETA_RAW_WITH_WORKER_PAUSE_PLAN mode=plan\n'
  printf '  range=%s..%s inner_wrapper=kis-stock-price-beta-raw.sh\n' "$fixed_start" "$fixed_end"
  printf '  target=lagrange-station/research-worker exact-container-only=yes\n'
  printf 'PLAN_ONLY: no Docker, secret, production-env, data-root write, or network action made\n'
  exit 0
fi

[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die '--commit must be exactly 40 lowercase hexadecimal characters'
[ "$commit" != 0000000000000000000000000000000000000000 ] || die '--commit must not be all zeroes'
case "$env_file" in
  /*) ;;
  *) die '--env-file must be absolute' ;;
esac
case "$env_file" in
  *$'\n'*|*$'\r'*|*/../*|*/..) die '--env-file has an unsafe path shape' ;;
esac
[ -f "$env_file" ] && [ ! -L "$env_file" ] || die 'env file is missing or symlinked'
env_metadata=$(stat -c '%u:%g:%a' -- "$env_file") || die 'env file metadata is unreadable'
[ "$env_metadata" = 0:0:600 ] || die 'env file must be root:root mode 0600'
case "$release_root" in
  /*) ;;
  *) die '--release-root must be absolute' ;;
esac
case "$release_root" in
  *$'\n'*|*$'\r'*|*/../*|*/..) die '--release-root has an unsafe path shape' ;;
esac
release_root=$(readlink -f -- "$release_root") || die 'release root cannot be resolved'
[ -d "$release_root" ] || die 'release root is missing'
release_owner=$(stat -c '%u' -- "$release_root") || die 'release root ownership is unreadable'
[ "$release_owner" = 0 ] || die 'release root must be root-owned'
release_parent=${release_root%/*}
release_name=${release_root##*/}
[ "${release_parent##*/}" = releases ] ||
  die 'release root must resolve below a releases directory'
[[ "$release_name" =~ ^[0-9a-f]{40}$ ]] ||
  die 'release root basename must be exactly 40 lowercase hexadecimal characters'
[ "$release_name" = "$commit" ] ||
  die 'release root basename does not match the requested commit'
release_parent_owner=$(stat -c '%u' -- "$release_parent") || die 'releases directory ownership is unreadable'
[ "$release_parent_owner" = 0 ] || die 'releases directory must be root-owned'
inner_wrapper=$release_root/scripts/ops/kis-stock-price-beta-raw.sh
compose_file=$release_root/deploy/compose/compose.yml
[ -x "$inner_wrapper" ] && [ ! -L "$inner_wrapper" ] ||
  die 'installed fixed-stock Raw inner wrapper is missing or unsafe'
[ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
  die 'installed Compose file is missing or unsafe'
inner_owner=$(stat -c '%u' -- "$inner_wrapper") || die 'installed fixed-stock Raw inner wrapper ownership is unreadable'
compose_owner=$(stat -c '%u' -- "$compose_file") || die 'installed Compose file ownership is unreadable'
[ "$inner_owner" = 0 ] && [ "$compose_owner" = 0 ] ||
  die 'installed release control files must be root-owned'

if ! dotenv_load "$env_file"; then
  die 'installed env file is malformed'
fi
if ! dotenv_validate_shell_overrides; then
  die 'shell overrides do not match the installed env file'
fi
installed_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
[[ "$installed_commit" =~ ^[0-9a-f]{40}$ ]] ||
  die 'installed LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
[ "$installed_commit" = "$commit" ] || die 'requested commit does not match installed release'
[ "${KIS_STOCK_PRICE_BETA_CONFIRM:-}" = "$confirmation" ] ||
  blocked 'set KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS for execute'

command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'

compose() {
  LAGRANGE_CODE_COMMIT="$commit" \
    docker compose --profile stock-price-beta-raw --env-file "$env_file" \
    --file "$compose_file" "$@"
}

prepare_exact_raw_image() {
  local image revision image_commit
  compose config --quiet || die 'installed Compose interpolation/config validation failed'
  compose build --pull=false research-stock-price-beta-raw ||
    die 'fixed-stock Raw image preparation failed'
  image="lagrange-station-research-stock-price-beta-raw:$commit"
  docker image inspect "$image" >/dev/null 2>&1 ||
    die 'prepared fixed-stock Raw image is missing for the requested commit'
  revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$image" 2>/dev/null) || die 'cannot inspect prepared fixed-stock Raw image OCI revision'
  [ "$revision" = "$commit" ] ||
    die 'prepared fixed-stock Raw image OCI revision does not match the requested commit'
  image_commit=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
    "$image" 2>/dev/null | awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }')
  [ "$image_commit" = "$commit" ] ||
    die 'prepared fixed-stock Raw image ENV LAGRANGE_CODE_COMMIT does not match the requested commit'
}

worker_identity_is_expected() {
  local labels image_id revision image_commit
  labels=$(docker inspect \
    --format '{{index .Config.Labels "com.docker.compose.project"}}/{{index .Config.Labels "com.docker.compose.service"}}|{{.Image}}' \
    "$worker_id" 2>/dev/null) || return 1
  case "$labels" in
    lagrange-station/research-worker\|sha256:*) image_id=${labels#*|} ;;
    *) return 1 ;;
  esac
  revision=$(docker image inspect \
    --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$image_id" 2>/dev/null) || return 1
  image_commit=$(docker image inspect \
    --format '{{range .Config.Env}}{{println .}}{{end}}' \
    "$image_id" 2>/dev/null | awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }') || return 1
  [ "$revision" = "$commit" ] && [ "$image_commit" = "$commit" ]
}

require_worker_identity() {
  worker_identity_is_expected || die 'research-worker container/image identity does not match the requested release'
}

wait_for_worker_healthy() {
  local now deadline health
  now=$(date -u +%s) || return 1
  [[ "$now" =~ ^[0-9]+$ ]] || return 1
  deadline=$((now + health_timeout_secs))
  while :; do
    health=$(docker inspect \
      --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' \
      "$worker_id" 2>/dev/null || true)
    case "$health" in
      healthy|running) return 0 ;;
      unhealthy|exited|dead) return 1 ;;
    esac
    now=$(date -u +%s) || return 1
    [[ "$now" =~ ^[0-9]+$ ]] || return 1
    [ "$now" -lt "$deadline" ] || return 1
    sleep "$health_poll_secs" || return 1
  done
}

restore_worker() {
  local original_status=$?
  trap - EXIT INT TERM HUP
  if [ "$worker_was_stopped" -eq 1 ]; then
    if ! worker_identity_is_expected; then
      printf 'kis-stock-price-beta-raw-with-worker-pause: worker identity changed before restore\n' >&2
      exit 1
    fi
    if ! docker start "$worker_id" >/dev/null; then
      printf 'kis-stock-price-beta-raw-with-worker-pause: failed to restart research-worker\n' >&2
      exit 1
    fi
    if ! wait_for_worker_healthy; then
      printf 'kis-stock-price-beta-raw-with-worker-pause: research-worker health timeout or failure\n' >&2
      exit 1
    fi
    if ! worker_identity_is_expected; then
      printf 'kis-stock-price-beta-raw-with-worker-pause: worker identity changed after restore\n' >&2
      exit 1
    fi
    printf 'KIS_STOCK_PRICE_BETA_RAW_WORKER_RESTORE: PASS\n'
  fi
  exit "$original_status"
}
worker_listing=$(docker ps \
  --filter label=com.docker.compose.project=lagrange-station \
  --filter label=com.docker.compose.service=research-worker \
  --format '{{.ID}}') || die 'cannot discover running research-worker containers'
if [ -n "$worker_listing" ]; then
  mapfile -t worker_ids <<<"$worker_listing"
else
  worker_ids=()
fi
[ "${#worker_ids[@]}" -le 1 ] || die 'multiple running research-worker containers found'
worker_id=${worker_ids[0]:-}

if [ -n "$worker_id" ]; then
  require_worker_identity
fi

# A potentially cold Compose build is deliberately complete before pausing the
# ordinary worker. The inner wrapper receives this reviewed prepare seam and
# still re-checks the exact immutable image before its one-shot run.
prepare_exact_raw_image

if [ -n "$worker_id" ]; then
  require_worker_identity
  trap restore_worker EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  docker stop --time "$stop_timeout_secs" "$worker_id" >/dev/null ||
    die 'failed to stop the exact research-worker container'
  worker_was_stopped=1
  printf 'KIS_STOCK_PRICE_BETA_RAW_WORKER_PAUSE: PASS\n'
fi

KIS_STOCK_PRICE_BETA_CONFIRM="$confirmation" \
KIS_STOCK_PRICE_BETA_IMAGE_PREPARED=1 \
KIS_STOCK_PRICE_BETA_PREPARED_RELEASE_ROOT="$release_root" \
  "$inner_wrapper" \
    --env-file "$env_file" \
    --start "$fixed_start" \
    --end "$fixed_end" \
    --execute

printf 'KIS_STOCK_PRICE_BETA_RAW_WITH_WORKER_PAUSE: PASS\n'
