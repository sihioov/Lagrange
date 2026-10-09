#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
service_status_validate_qa_root "$qa_root"
for command in curl docker grep head node npm pgrep python3 systemd-run systemctl; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done
web_port=33043
api_port=38193
web_url="http://127.0.0.1:${web_port}"
api_url="http://127.0.0.1:${api_port}"
node_path="$(command -v node)"
run_root="$qa_root/market-stream"
artifact_root="$(service_status_artifact_root "$qa_root")/market-stream"
mkdir -m 700 -p "$run_root/logs" "$run_root/pids" "$artifact_root/screenshots"
api_unit="lagrange-service-status-wp5-market-api-$$.service"
web_unit="lagrange-service-status-wp5-market-next-$$.service"
ownership_file="$run_root/ownership.txt"
metrics_file="$run_root/logs/unit-metrics.log"
slot_owner="market-stream:$$"
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
next_root="$artifact_root/next-standalone"
next_server_root=""

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
    if ! service_status_stop_unit "$playwright_unit" market-playwright "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( web_started )); then
    if ! service_status_stop_unit "$web_unit" market-next "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( api_started )); then
    if ! service_status_stop_unit "$api_unit" market-api "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( cleanup_failed )); then
    if (( status == 0 )); then status=1; fi
    printf 'Market-stream owned unit cleanup is incomplete; retain the serial lock for review: %s\n' "$ownership_file" >&2
  else
    service_status_release_slot_lock "$qa_root" "$slot_owner"
  fi
  if (( status != 0 )); then
    printf 'Market-stream QA stopped with status=%s; owned identities=%s; unit/peak/OOM evidence=%s\n' \
      "$status" "$ownership_file" "$metrics_file" >&2
  fi
  printf 'lane=market original_exit=%s cleanup_failed=%s final_exit=%s time_utc=%s\n' \
    "$original_status" "$cleanup_failed" "$status" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$metrics_file"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

check_free_port "$web_port"
check_free_port "$api_port"
case "${1:-}" in
  "") service_status_prepare_next "$qa_root" market-stream "$api_url" owner_only market_ws ;;
  --reuse-prebuilt)
    service_status_assert_no_runtime_sets "$qa_root"
    python3 - "$repo_root" "$qa_root" "$next_root" "$api_url" <<'PY'
import hashlib, json, re, sys
from pathlib import Path
repo, qa, staged = map(Path, sys.argv[1:4]); api = sys.argv[4]
line = f'lane=market-stream api_internal_url={api} owner_quotes_mode=owner_only owner_quote_transport=market_ws standalone_root={staged} build_unit='
assert any(x.startswith(line) for x in (qa/'ownership.txt').read_text().splitlines())
receipts = (qa/'market-stream/logs/next-build-receipt.tsv').read_text().splitlines()
end = [x for x in receipts if x.startswith('phase=end ')][-1]
assert 'original_service_result=success ' in end and 'original_exit_status=0 ' in end
assert 'observed_memory_max_bytes=1610612736 ' in end and 'oom 0,oom_kill 0,' in end
integrity = json.loads((qa/'accepted-integrity-before-runtime.json').read_text())
for name, expected in integrity['root_web'].items():
    assert hashlib.sha256((repo/name).read_bytes()).hexdigest() == expected, name
css = integrity['coordinator_css']; assert hashlib.sha256((repo/css['path']).read_bytes()).hexdigest() == css['sha256']
manifest = next(staged.rglob('routes-manifest.json')); rewrites = json.loads(manifest.read_text())['rewrites']
rules = rewrites if isinstance(rewrites,list) else sum(rewrites.values(), [])
assert any(x['source'].startswith('/api/') and x['destination'].startswith(api+'/api/') for x in rules)
print('reuse_prebuilt=verified lane=market-stream current_root_web_and_css_hashes=match original_build_exit=0')
PY
    ;;
  *) echo "Choose the default build or --reuse-prebuilt after an E2E-only repair." >&2; exit 2 ;;
esac
next_server_root="$(service_status_next_server_root "$next_root")"
printf 'api_unit=%s\napi_port=%s\nnext_unit=%s\nnext_port=%s\nmode=owner_only+market_ws\napi_internal_url=%s\nnext_root=%s\n' \
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

service_status_preflight "$qa_root" market-runtime-start 1408
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

service_status_preflight "$qa_root" market-add-next 1280
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
    API_INTERNAL_URL="$api_url" OWNER_INTRADAY_QUOTES_MODE=owner_only OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
    PORT="$web_port" HOSTNAME=127.0.0.1 PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN="$api_url" \
    NEXT_TELEMETRY_DISABLED=1 "$node_path" server.js
wait_http "$web_url/" 200
service_status_verify_next_static "$web_url"
web_pid="$(unit_main_pid "$web_unit")"
printf 'next_pid=%s\n' "$web_pid" >>"$ownership_file"

service_status_preflight "$qa_root" market-add-playwright 1024
candidate_playwright_unit="lagrange-service-status-wp5-market-playwright-$$.service"
service_status_require_unused_unit "$candidate_playwright_unit"
playwright_unit="$candidate_playwright_unit"
playwright_pid_file="$run_root/pids/playwright.pid"
playwright_cgroup_metrics="$run_root/logs/playwright-cgroup.tsv"
printf 'playwright_unit=%s\nplaywright_pid_file=%s\n' "$playwright_unit" "$playwright_pid_file" >>"$ownership_file"
systemd-run --user --quiet --wait --pipe --unit="$playwright_unit" \
  --property=Type=exec \
  --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $run_root/logs/playwright-receipt.tsv 805306368 $playwright_unit" \
  --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $run_root/logs/playwright-receipt.tsv 805306368 $playwright_unit" \
  --property=TimeoutStopSec=30s \
  --property=WorkingDirectory="$repo_root/apps/web" \
  --property=MemoryMax=768M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
  --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
  -- env PATH="$PATH" NODE_OPTIONS=--max-old-space-size=320 NEXT_TELEMETRY_DISABLED=1 \
    OWNER_INTRADAY_QUOTES_MODE=owner_only OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
    PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN="$api_url" \
    PLAYWRIGHT_JSON_OUTPUT_NAME="$artifact_root/results.json" \
    bash "$repo_root/scripts/qa/service-status-unit-runner.sh" \
    "$playwright_pid_file" "$playwright_cgroup_metrics" 805306368 \
    "$repo_root/node_modules/.bin/playwright" test \
    --project=chromium --workers=1 --reporter=line,json \
    --output "$artifact_root/playwright-results" \
    tests/e2e/service-status-market-stream.spec.ts \
  >"$run_root/playwright-market-stream.log" 2>&1 &
playwright_runner_pid=$!
if service_status_wait_unit market-playwright "$playwright_unit" "$metrics_file" "$playwright_runner_pid"; then playwright_result=0; else playwright_result=$?; fi
if (( playwright_result != 0 )); then
  printf 'Market-stream browser stage failed: unit=%s exit=%s log=%s metrics=%s\n' \
    "$playwright_unit" "$playwright_result" "$run_root/playwright-market-stream.log" "$metrics_file" >&2
  exit "$playwright_result"
fi
if [[ ! -s "$playwright_pid_file" ]]; then
  echo "The market-stream Playwright unit did not record its PID." >&2
  exit 1
fi
playwright_pid="$(cat "$playwright_pid_file")"
if [[ ! "$playwright_pid" =~ ^[1-9][0-9]*$ ]]; then
  echo "The market-stream Playwright unit recorded an invalid PID." >&2
  exit 1
fi
printf 'playwright_pid=%s\n' "$playwright_pid" >>"$ownership_file"
