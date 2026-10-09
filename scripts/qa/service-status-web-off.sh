#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
service_status_validate_qa_root "$qa_root"
for command in curl docker grep head node npm pgrep python3 systemd-run systemctl; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done
web_port=33042
api_port=38192
web_url="http://127.0.0.1:${web_port}"
api_url="http://127.0.0.1:${api_port}"
node_path="$(command -v node)"
run_root="$qa_root/web-off"
artifact_root="$(service_status_artifact_root "$qa_root")/web-off"
mkdir -m 700 -p "$run_root/logs" "$run_root/pids" "$artifact_root/screenshots"
api_unit="lagrange-service-status-wp5-web-off-api-$$.service"
web_unit="lagrange-service-status-wp5-web-off-next-$$.service"
ownership_file="$run_root/ownership.txt"
metrics_file="$run_root/logs/unit-metrics.log"
next_root="$artifact_root/next-standalone"
next_server_root=""
slot_owner="web-off:$$"
service_status_acquire_slot_lock "$qa_root" "$slot_owner"

check_free_port() {
  python3 - "$1" <<'PY'
import socket
import sys

port = int(sys.argv[1])
with socket.socket() as sock:
    try:
        sock.bind(("127.0.0.1", port))
    except OSError as error:
        raise SystemExit(f"required isolated QA port {port} is unavailable: {error.__class__.__name__}")
PY
}
api_started=0
web_started=0
playwright_unit=""
playwright_runner_pid=""
cleanup() {
  local status=$?
  local original_status="$status"
  local cleanup_failed=0
  trap - EXIT INT TERM
  if [[ -n "$playwright_unit" ]]; then
    if ! service_status_stop_unit "$playwright_unit" off-playwright "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( web_started )); then
    if ! service_status_stop_unit "$web_unit" off-next "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( api_started )); then
    if ! service_status_stop_unit "$api_unit" off-api "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( cleanup_failed )); then
    if (( status == 0 )); then status=1; fi
    printf 'Off-mode owned unit cleanup is incomplete; retain the serial lock for review: %s\n' "$ownership_file" >&2
  else
    service_status_release_slot_lock "$qa_root" "$slot_owner"
  fi
  if (( status != 0 )); then
    printf 'Off-mode QA stopped with status=%s; owned identities=%s; unit/peak/OOM evidence=%s\n' \
      "$status" "$ownership_file" "$metrics_file" >&2
  fi
  printf 'lane=off original_exit=%s cleanup_failed=%s final_exit=%s time_utc=%s\n' \
    "$original_status" "$cleanup_failed" "$status" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$metrics_file"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

check_free_port "$web_port"
check_free_port "$api_port"
service_status_prepare_next "$qa_root" web-off "$api_url" off market_ws
next_server_root="$(service_status_next_server_root "$next_root")"
printf 'api_unit=%s\napi_port=%s\nnext_unit=%s\nnext_port=%s\nmode=off\napi_internal_url=%s\nnext_root=%s\n' \
  "$api_unit" "$api_port" "$web_unit" "$web_port" "$api_url" "$next_root" >"$ownership_file"
printf 'next_server_root=%s\n' "$next_server_root" >>"$ownership_file"

unit_main_pid() {
  local unit="$1" pid
  for ((attempt = 0; attempt < 30; attempt += 1)); do
    pid="$(systemctl --user show "$unit" --property=MainPID --value 2>/dev/null || true)"
    if [[ "$pid" =~ ^[1-9][0-9]*$ ]]; then printf '%s\n' "$pid"; return 0; fi
    sleep 1
  done
  echo "owned QA systemd unit did not expose a running PID: $unit" >&2
  return 1
}

service_status_preflight "$qa_root" off-runtime-start 1408
service_status_require_unused_unit "$api_unit"
api_started=1
systemd-run --user --quiet --unit="$api_unit" \
  --property=Type=exec \
  --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $run_root/logs/synthetic-api-receipt.tsv 134217728 $api_unit" \
  --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $run_root/logs/synthetic-api-receipt.tsv 134217728 $api_unit" \
  --property=TimeoutStopSec=30s \
  --property=WorkingDirectory="$repo_root/apps/web" \
  --property=MemoryMax=128M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
  --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
  --property="StandardOutput=append:$run_root/logs/synthetic-api.log" \
  --property="StandardError=append:$run_root/logs/synthetic-api.log" \
  -- env PATH="$PATH" NODE_OPTIONS=--max-old-space-size=80 SYNTHETIC_API_PORT="$api_port" \
    "$node_path" tests/e2e/support/synthetic-api.mjs

wait_http() {
  local url="$1" expected="$2" deadline=$((SECONDS + 120)) code
  while (( SECONDS < deadline )); do
    code="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' "$url" 2>/dev/null || true)"
    if [[ "$code" == "$expected" ]]; then return 0; fi
    sleep 1
  done
  echo "timed out waiting for the owned QA endpoint at $url" >&2
  return 1
}
wait_http "$api_url/__test/stock-beta/intraday-state" 200
api_pid="$(unit_main_pid "$api_unit")"
printf 'api_pid=%s\n' "$api_pid" >>"$ownership_file"

service_status_preflight "$qa_root" off-add-next 1280
service_status_require_unused_unit "$web_unit"
web_started=1
systemd-run --user --quiet --unit="$web_unit" \
  --property=Type=exec \
  --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $run_root/logs/next-receipt.tsv 268435456 $web_unit" \
  --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $run_root/logs/next-receipt.tsv 268435456 $web_unit" \
  --property=TimeoutStopSec=30s \
  --property=WorkingDirectory="$next_server_root" \
  --property=MemoryMax=256M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
  --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
  --property="StandardOutput=append:$run_root/logs/next-production.log" \
  --property="StandardError=append:$run_root/logs/next-production.log" \
  -- env PATH="$PATH" NODE_OPTIONS=--max-old-space-size=160 \
    API_INTERNAL_URL="$api_url" OWNER_INTRADAY_QUOTES_MODE=off OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
    PORT="$web_port" HOSTNAME=127.0.0.1 PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN="$api_url" \
    NEXT_TELEMETRY_DISABLED=1 "$node_path" server.js
wait_http "$web_url/" 200
service_status_verify_next_static "$web_url"
web_pid="$(unit_main_pid "$web_unit")"
printf 'next_pid=%s\n' "$web_pid" >>"$ownership_file"

batch_helper="$repo_root/scripts/qa/service-status-playwright-batches.mjs"
inventory_file="$run_root/inventory.json"
plan_file="$run_root/batch-plan.json"
run_browser_unit() {
  local phase="$1" result_file="$2" output_dir="$3" log_file="$4"
  shift 4
  local selection=("$@")
  service_status_preflight "$qa_root" "off-add-$phase" 1024
  local candidate="lagrange-service-status-wp5-web-off-${phase}-$$.service"
  service_status_require_unused_unit "$candidate"
  playwright_unit="$candidate"
  local playwright_pid_file="$run_root/pids/${phase}.pid"
  local playwright_cgroup_metrics="$run_root/logs/${phase}-cgroup.tsv"
  printf 'playwright_unit=%s\nplaywright_pid_file=%s\n' "$playwright_unit" "$playwright_pid_file" >>"$ownership_file"
  systemd-run --user --quiet --wait --pipe --unit="$playwright_unit" \
    --property=Type=exec \
    --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $run_root/logs/${phase}-receipt.tsv 805306368 $playwright_unit" \
    --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $run_root/logs/${phase}-receipt.tsv 805306368 $playwright_unit" \
    --property=TimeoutStopSec=30s \
    --property=WorkingDirectory="$repo_root/apps/web" \
    --property=MemoryMax=768M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
    --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
    -- env PATH="$PATH" NODE_OPTIONS=--max-old-space-size=320 NEXT_TELEMETRY_DISABLED=1 \
      PLAYWRIGHT_JSON_OUTPUT_NAME="$result_file" \
      OWNER_INTRADAY_QUOTES_MODE=off OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
      PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN="$api_url" \
      SERVICE_STATUS_SCREENSHOT_DIR="$artifact_root/screenshots" \
      bash "$repo_root/scripts/qa/service-status-unit-runner.sh" \
      "$playwright_pid_file" "$playwright_cgroup_metrics" 805306368 \
      "$repo_root/node_modules/.bin/playwright" test \
      --project=chromium --workers=1 \
      --reporter=line,json "${selection[@]}" \
      --output "$output_dir" \
      --grep-invert '@actual' \
      tests/e2e/service-status.spec.ts \
      tests/e2e/stock-beta.spec.ts \
      tests/e2e/stock-beta-intraday.spec.ts \
    >"$log_file" 2>&1 &
  playwright_runner_pid=$!
  if service_status_wait_unit "$phase" "$playwright_unit" "$metrics_file" "$playwright_runner_pid"; then result=0; else result=$?; fi
  if (( result != 0 )); then
    printf 'Off browser unit failed: phase=%s original_systemd_run_exit=%s log=%s\n' "$phase" "$result" "$log_file" >&2
    return "$result"
  fi
  service_status_browser_idle "$playwright_unit" "$playwright_pid_file" "$metrics_file"
}
run_browser_unit inventory "$inventory_file" "$artifact_root/inventory-output" "$run_root/inventory.log" --list
node "$batch_helper" plan "$inventory_file" "$plan_file" "$artifact_root" >"$run_root/inventory-summary.json"
batch_count="$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1])).batches.length)' "$plan_file")"
for ((batch = 1; batch <= batch_count; batch += 1)); do
  node "$batch_helper" sources "$plan_file"
  mapfile -t fields < <(node -e 'const b=JSON.parse(require("fs").readFileSync(process.argv[1])).batches[Number(process.argv[2])-1]; for(const k of ["list_file","result_file","output_dir","log_file"]) console.log(b[k]);' "$plan_file" "$batch")
  run_browser_unit "batch-$batch" "${fields[1]}" "${fields[2]}" "${fields[3]}" --test-list "${fields[0]}"
  node "$batch_helper" verify "$plan_file" "$batch"
done
node "$batch_helper" merge "$plan_file"
