#!/usr/bin/env bash
# Run one isolated KIS historical range capture while preserving the ordinary
# research worker container. Intended for a root-owned transient systemd unit.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)

start_date=
end_date=
commit=
env_file=
state_file=/var/lib/lagrange/state/range-raw/etf11-10y.tsv

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-range-raw-with-worker-pause.sh \
  --start YYYY-MM-DD --end YYYY-MM-DD --commit 40HEX \
  [--env-file ABSOLUTE_PATH] [--state-file ABSOLUTE_PATH]

Stops the exact running lagrange-station research-worker Compose container,
runs the existing read-only Stage5 range wrapper, and starts the same container
again on success, failure, termination, or hangup. It does not publish data.
EOF
}

die() { echo "kis-range-raw-with-worker-pause: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --start) [ "$#" -ge 2 ] || die '--start needs a value'; start_date=$2; shift 2 ;;
    --end) [ "$#" -ge 2 ] || die '--end needs a value'; end_date=$2; shift 2 ;;
    --commit) [ "$#" -ge 2 ] || die '--commit needs a value'; commit=$2; shift 2 ;;
    --env-file) [ "$#" -ge 2 ] || die '--env-file needs a value'; env_file=$2; shift 2 ;;
    --state-file) [ "$#" -ge 2 ] || die '--state-file needs a value'; state_file=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die 'must run as root'
[ -n "$env_file" ] || die '--env-file must name an immutable release env file'
[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end'
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die 'invalid --commit'
case "$env_file:$state_file" in
  /*:/*) ;;
  *) die 'env and state paths must be absolute' ;;
esac
case "$env_file:$state_file" in
  *'/../'*|*'/..:'*|*':../'*|*':/../'*) die "paths must not contain '..'" ;;
esac
[ -f "$env_file" ] && [ ! -L "$env_file" ] || die 'env file is missing or symlinked'

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
      echo 'kis-range-raw-with-worker-pause: failed to restart research-worker' >&2
      exit 1
    fi
    deadline=$((SECONDS + 180))
    while [ "$SECONDS" -lt "$deadline" ]; do
      health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$worker_id" 2>/dev/null || true)
      case "$health" in
        healthy|running)
          echo 'KIS_RANGE_RAW_WORKER_RESTORE: PASS'
          exit "$original_status"
          ;;
        unhealthy|exited|dead)
          echo "kis-range-raw-with-worker-pause: research-worker restart state=$health" >&2
          exit 1
          ;;
      esac
      sleep 5
    done
    echo 'kis-range-raw-with-worker-pause: research-worker health timeout' >&2
    exit 1
  fi
  exit "$original_status"
}
trap restore_worker EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if [ -n "$worker_id" ]; then
  [ "$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project"}}/{{index .Config.Labels "com.docker.compose.service"}}' "$worker_id")" = 'lagrange-station/research-worker' ] ||
    die 'research-worker labels changed after discovery'
  worker_was_stopped=1
  docker stop --time 300 "$worker_id" >/dev/null
  echo 'KIS_RANGE_RAW_WORKER_PAUSE: PASS'
fi

LAGRANGE_CODE_COMMIT="$commit" \
KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS \
  "$script_dir/kis-range-raw-backfill.sh" \
    --env-file "$env_file" \
    --state-file "$state_file" \
    --start "$start_date" \
    --end "$end_date" \
    --execute

echo 'KIS_RANGE_RAW_WITH_WORKER_PAUSE: PASS'
