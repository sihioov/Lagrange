#!/usr/bin/env bash
# Run one isolated ETF11 KIS KSD action-range capture while preserving the
# ordinary research worker container. Intended for a root-owned transient
# systemd unit.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)

start_date=
end_date=
commit=
env_file=

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-action-range-raw-with-worker-pause.sh \
  --start YYYY-MM-DD --end YYYY-MM-DD --commit 40HEX \
  --env-file ABSOLUTE_PATH

Stops the exact running lagrange-station research-worker Compose container,
runs the read-only ETF11 KIS KSD action-range wrapper, and starts the same
container again on success, failure, termination, or hangup. It writes only an
immutable Raw batch; it does not publish data or access account/order APIs.
EOF
}

die() {
  printf 'kis-action-range-raw-with-worker-pause: %s\n' "$*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --start)
      [ "$#" -ge 2 ] || die '--start needs a value'
      [ -z "$start_date" ] || die '--start must not be repeated'
      start_date=$2
      shift 2
      ;;
    --end)
      [ "$#" -ge 2 ] || die '--end needs a value'
      [ -z "$end_date" ] || die '--end must not be repeated'
      end_date=$2
      shift 2
      ;;
    --commit)
      [ "$#" -ge 2 ] || die '--commit needs a value'
      [ -z "$commit" ] || die '--commit must not be repeated'
      commit=$2
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs a value'
      [ -z "$env_file" ] || die '--env-file must not be repeated'
      env_file=$2
      shift 2
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
[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end'
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die 'invalid --commit'
case "$env_file" in
  /*) ;;
  *) die '--env-file must be absolute' ;;
esac
case "$env_file" in
  *$'\n'*|*$'\r'*|*/../*|*/..) die '--env-file has an unsafe path shape' ;;
esac
[ -f "$env_file" ] && [ ! -L "$env_file" ] ||
  die 'env file is missing or symlinked'

head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}') ||
  die 'repository HEAD is unavailable'
[ "$head" = "$commit" ] || die 'commit does not match repository HEAD'

mapfile -t worker_ids < <(
  docker ps \
    --filter label=com.docker.compose.project=lagrange-station \
    --filter label=com.docker.compose.service=research-worker \
    --format '{{.ID}}'
)
[ "${#worker_ids[@]}" -le 1 ] || die 'multiple running research-worker containers found'
worker_id=${worker_ids[0]:-}
worker_was_stopped=0

restore_worker() {
  local original_status=$? health deadline
  trap - EXIT INT TERM HUP
  if [ "$worker_was_stopped" -eq 1 ]; then
    if ! docker start "$worker_id" >/dev/null; then
      echo 'kis-action-range-raw-with-worker-pause: failed to restart research-worker' >&2
      exit 1
    fi
    deadline=$((SECONDS + 180))
    while [ "$SECONDS" -lt "$deadline" ]; do
      health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$worker_id" 2>/dev/null || true)
      case "$health" in
        healthy|running)
          echo 'KIS_ACTION_RANGE_RAW_WORKER_RESTORE: PASS'
          exit "$original_status"
          ;;
        unhealthy|exited|dead)
          printf 'kis-action-range-raw-with-worker-pause: research-worker restart state=%s\n' "$health" >&2
          exit 1
          ;;
      esac
      sleep 5
    done
    echo 'kis-action-range-raw-with-worker-pause: research-worker health timeout' >&2
    exit 1
  fi
  exit "$original_status"
}
trap restore_worker EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if [ -n "$worker_id" ]; then
  labels=$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project"}}/{{index .Config.Labels "com.docker.compose.service"}}' "$worker_id")
  [ "$labels" = 'lagrange-station/research-worker' ] ||
    die 'research-worker labels changed after discovery'
  worker_was_stopped=1
  docker stop --time 300 "$worker_id" >/dev/null
  echo 'KIS_ACTION_RANGE_RAW_WORKER_PAUSE: PASS'
fi

LAGRANGE_CODE_COMMIT="$commit" \
KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS \
  "$script_dir/kis-action-range-raw-backfill.sh" \
    --env-file "$env_file" \
    --start "$start_date" \
    --end "$end_date" \
    --scope etf11 \
    --execute

echo 'KIS_ACTION_RANGE_RAW_WITH_WORKER_PAUSE: PASS'
