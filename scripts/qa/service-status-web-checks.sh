#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
stage="${1:-all}"
service_status_validate_qa_root "$qa_root"
case "$stage" in
  all|unit|typecheck|lint|build) ;;
  *) echo "Choose one Web check: all, unit, typecheck, lint, or build." >&2; exit 2 ;;
esac
slot_owner="web-checks:$$"
service_status_acquire_slot_lock "$qa_root" "$slot_owner"
release_slot_lock() {
  local status=$?
  trap - EXIT
  service_status_release_slot_lock "$qa_root" "$slot_owner"
  exit "$status"
}
trap release_slot_lock EXIT
for command in npm pgrep systemd-run systemctl docker; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done

run_check() {
  local name="$1" memory="$2" required="$3" heap="$4"
  shift 4
  local unit="lagrange-service-status-wp5-web-${name}-$$.service"
  local run_root="$qa_root/web-checks" log_file="$qa_root/web-checks/logs/${name}.log"
  local metrics="$qa_root/web-checks/logs/unit-metrics.log" pid_file="$qa_root/web-checks/pids/${name}.pid"
  local cgroup_metrics="$run_root/logs/${name}-cgroup.tsv" cap_bytes result runner_pid
  case "$memory" in
    768M) cap_bytes=805306368 ;;
    1536M) cap_bytes=1610612736 ;;
    *) echo "Unexpected Web stage cap: $memory" >&2; return 2 ;;
  esac
  service_status_assert_no_runtime_sets "$qa_root"
  service_status_preflight "$qa_root" "web-$name" "$required"
  service_status_require_unused_unit "$unit"
  mkdir -m 700 -p "$run_root/logs" "$run_root/pids"
  systemd-run --user --quiet --wait --unit="$unit" \
    --property=Type=exec \
    --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $run_root/logs/${name}-receipt.tsv $cap_bytes $unit" \
    --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $run_root/logs/${name}-receipt.tsv $cap_bytes $unit" \
    --property=TimeoutStopSec=30s \
    --property=WorkingDirectory="$repo_root" \
    --property="MemoryMax=$memory" --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
    --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
    --property="StandardOutput=append:$log_file" \
    --property="StandardError=append:$log_file" \
    -- env PATH="$PATH" NEXT_TELEMETRY_DISABLED=1 \
      API_INTERNAL_URL=http://127.0.0.1:8080 \
      OWNER_INTRADAY_QUOTES_MODE=off OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
      NODE_OPTIONS="--max-old-space-size=$heap" \
      bash "$repo_root/scripts/qa/service-status-unit-runner.sh" \
      "$pid_file" "$cgroup_metrics" "$cap_bytes" "$@" &
  runner_pid=$!
  if service_status_wait_unit "web-$name" "$unit" "$metrics" "$runner_pid"; then result=0; else result=$?; fi
  if [[ -s "$pid_file" ]]; then
    printf 'stage=%s unit=%s pid=%s log=%s\n' "$name" "$unit" "$(cat "$pid_file")" "$log_file" \
      >>"$run_root/ownership.txt"
  fi
  if (( result != 0 )); then
    printf 'Web stage failed: stage=%s unit=%s exit=%s log=%s metrics=%s\n' "$name" "$unit" "$result" "$log_file" "$metrics" >&2
    return "$result"
  fi
}

run_named_stage() {
  case "$1" in
    unit) run_check unit 768M 1024 512 npm test --workspace=@lagrange/web -- --maxWorkers=1 ;;
    typecheck) run_check typecheck 768M 1024 512 npm run typecheck --workspace=@lagrange/web ;;
    lint) run_check lint 768M 1024 512 npm run lint --workspace=@lagrange/web ;;
    build) run_check build 1536M 1792 1024 npm run build --workspace=@lagrange/web ;;
  esac
}

if [[ "$stage" == all ]]; then
  for name in unit typecheck lint build; do run_named_stage "$name"; done
else
  run_named_stage "$stage"
fi
