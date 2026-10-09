#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
service_status_validate_qa_root "$qa_root"
stage="${1:-}"
case "$stage" in
  fixtures) check_command=(node "$repo_root/tests/fixtures/service-status/validate_contract.mjs") ;;
  temporal) check_command=(node "$repo_root/tests/fixtures/service-status/validate_temporal_contract.mjs") ;;
  openapi) check_command=(python3 "$repo_root/scripts/qa/service-status-openapi-check.py") ;;
  *) exit 2 ;;
esac
service_status_assert_no_runtime_sets "$qa_root"
service_status_preflight "$qa_root" "contract-$stage" 1024
owner="contract-$stage:$$"
service_status_acquire_slot_lock "$qa_root" "$owner"
trap 'service_status_release_slot_lock "$qa_root" "$owner"' EXIT
logs="$qa_root/contracts/$stage-$$"
mkdir -m 700 -p "$logs"
unit="lagrange-service-status-wp5-contract-$stage-$$.service"
service_status_require_unused_unit "$unit"
systemd-run --user --quiet --wait --unit="$unit" \
  -p Type=exec -p WorkingDirectory="$repo_root" \
  -p "ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $logs/$stage-receipt.tsv 805306368 $unit" \
  -p "ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $logs/$stage-receipt.tsv 805306368 $unit" \
  -p MemoryMax=768M -p MemorySwapMax=0 -p MemoryAccounting=yes -p CPUWeight=10 \
  -p OOMPolicy=kill -p RuntimeMaxSec=90min -p TimeoutStopSec=30s \
  -p "StandardOutput=append:$logs/$stage.log" -p "StandardError=append:$logs/$stage.log" \
  -- env PATH="$PATH" NODE_OPTIONS=--max-old-space-size=512 npm_config_offline=true \
  bash "$repo_root/scripts/qa/service-status-unit-runner.sh" "$logs/$stage.pid" "$logs/$stage-cgroup.tsv" 805306368 "${check_command[@]}" &
runner=$!
service_status_wait_unit "contract-$stage" "$unit" "$logs/unit-metrics.log" "$runner"
