#!/usr/bin/env bash
# Resumable production backfill plan/executor. The default is --plan and does
# not invoke Docker or a provider. --execute is deliberately guarded so a
# sleeping operator cannot accidentally make external API calls.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
state_file=${LAGRANGE_BACKFILL_STATE:-/var/lib/lagrange/data/backfill/state.tsv}
start_date=
end_date=
universes=etf
mode=plan

usage() {
  cat <<'EOF'
Usage: scripts/ops/backfill-production.sh --start YYYY-MM-DD --end YYYY-MM-DD
       [--universe etf|candidate|all] [--plan|--execute]

The plan covers the fixed 11-ETF KIS EOD dataset. Candidate universes
(KOSPI200/KOSDAQ150) remain a separate blocked step until their credentialed
candidate bridge is released; this command never pretends that KIS EOD bars
are candidate source data.

--execute requires BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS
and a valid production config. It uses the existing idempotent worker path;
no order/account endpoint is called. The state file is append-only and each
worker run must complete Raw -> normalized -> DB publication before progress
is recorded.
EOF
}

die() { echo "backfill-production: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --start)
      [ "$#" -ge 2 ] || die '--start needs YYYY-MM-DD'
      start_date=$2
      shift 2
      ;;
    --end)
      [ "$#" -ge 2 ] || die '--end needs YYYY-MM-DD'
      end_date=$2
      shift 2
      ;;
    --universe)
      [ "$#" -ge 2 ] || die '--universe needs etf|candidate|all'
      universes=$2
      shift 2
      ;;
    --plan) mode=plan; shift ;;
    --execute) mode=execute; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$start_date" ] && [ -n "$end_date" ] || die '--start and --end are required'
[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start date'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end date'
case "$universes" in
  etf) ;;
  candidate|all)
    blocked 'KOSPI200/KOSDAQ150 credentialed candidate bridge is not part of the current EOD release; use the candidate runbook after that bridge is released'
    ;;
  *) die 'universe must be etf, candidate, or all' ;;
esac

if [ "$mode" = execute ]; then
  [ "${BACKFILL_CONFIRM_EXTERNAL:-}" = I_UNDERSTAND_READ_ONLY_KIS_CALLS ] ||
    blocked 'execution is disabled until BACKFILL_CONFIRM_EXTERNAL is explicitly set'
  bash "$script_dir/validate-production-config.sh" --env-file "$env_file"
  command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
  docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'
  mkdir -p "$(dirname "$state_file")"
fi

date_list() {
  python3 - "$start_date" "$end_date" <<'PY'
import datetime as dt
import sys
start = dt.date.fromisoformat(sys.argv[1])
end = dt.date.fromisoformat(sys.argv[2])
if end < start:
    raise SystemExit("backfill-production: --end precedes --start")
day = start
while day <= end:
    print(day.isoformat())
    day += dt.timedelta(days=1)
PY
}

if [ "$mode" = plan ]; then
  cat <<EOF
BACKFILL_PLAN: KIS read-only fixed ETF EOD
  source dates: $start_date..$end_date (provider decides trading-day/holiday response)
  raw scope: kis/kr; normalized scope: kis-normalized/kr; DB logical provider: KRX
  idempotency: deterministic normalized batch/source_batch_id and exact manifest replay
  verification: raw manifest+hashes -> normalize -> four canonical docs -> DB publication
  approval: operator reviews manifest/hash/counts, then pins one dataset version for recommendation/backtest/Paper
  state: $state_file (created only by --execute)
PLAN_ONLY: no KIS call, Docker run, file write, or account/order access
EOF
  exit 0
fi

compose=(docker compose --env-file "$env_file" -f "$root/deploy/compose/compose.yml")
while IFS= read -r date; do
  if [ -f "$state_file" ] && grep -Fqx "$date$(printf '\t')PUBLISHED" "$state_file"; then
    echo "BACKFILL_SKIP date=$date state=PUBLISHED"
    continue
  fi
  printf '%s\tRUNNING\n' "$date" >>"$state_file"
  if "${compose[@]}" run --rm --no-deps research-worker --once --date "$date"; then
    printf '%s\tPUBLISHED\n' "$date" >>"$state_file"
    echo "BACKFILL_DONE date=$date"
  else
    printf '%s\tFAILED\n' "$date" >>"$state_file"
    echo "BACKFILL_STOPPED date=$date (rerun resumes after operator review)" >&2
    exit 1
  fi
done < <(date_list)
echo 'BACKFILL: PASS'
