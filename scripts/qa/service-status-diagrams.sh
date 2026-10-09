#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
service_status_validate_qa_root "$qa_root"
logs="$qa_root/diagrams"
mkdir -m 700 -p "$logs"

if [[ "${1:-}" == --in-unit ]]; then
  name="lagrange-service-status-wp5-diagrams-$$"
  monitor=""
  cleanup_container() {
    local status=$?
    trap - EXIT
    if [[ -n "$monitor" ]]; then kill "$monitor" 2>/dev/null || true; wait "$monitor" 2>/dev/null || true; fi
    service_status_remove_container "$name" plantuml "$logs/container-metrics.log" || { [[ "$status" != 0 ]] || status=1; }
    exit "$status"
  }
  trap cleanup_container EXIT
  trap 'exit 143' TERM
  trap 'exit 130' INT
  docker image inspect plantuml/plantuml --format '{{.Id}}' >"$logs/cached-image.txt"
  docker run --pull=never --detach --init --name "$name" \
    --label com.docker.compose.project=lagrange-service-status-wp5 \
    --network none --read-only --tmpfs /tmp:rw,nosuid,size=64m \
    --memory 768m --memory-swap 768m --cpu-shares 48 --cpus 1 \
    --cap-drop ALL --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
    --mount "type=bind,src=$repo_root/docs/diagrams,dst=/data" \
    --env 'JAVA_TOOL_OPTIONS=-Xmx512m -Djava.io.tmpdir=/tmp -Djava.awt.headless=true' \
    --entrypoint /bin/sh plantuml/plantuml -c '
      set -u
      printf "render_cap_probe memory_max_bytes=%s swap_max_bytes=%s cpu_weight=%s cgroup=%s\n" \
        "$(cat /sys/fs/cgroup/memory.max)" "$(cat /sys/fs/cgroup/memory.swap.max)" \
        "$(cat /sys/fs/cgroup/cpu.weight)" "$(cat /proc/self/cgroup)"
      test "$(cat /sys/fs/cgroup/memory.max)" = 805306368 || exit 125
      test "$(cat /sys/fs/cgroup/memory.swap.max)" = 0 || exit 125
      test "$(cat /sys/fs/cgroup/cpu.weight)" = 10 || exit 125
      java -jar /opt/plantuml.jar -tpng /data/component_architecture.puml /data/runtime_deployment.puml
      result=$?
      printf "render_original_exit=%s memory_peak_bytes=%s memory_max_bytes=%s swap_max_bytes=%s cpu_weight=%s\n" \
        "$result" "$(cat /sys/fs/cgroup/memory.peak)" "$(cat /sys/fs/cgroup/memory.max)" \
        "$(cat /sys/fs/cgroup/memory.swap.max)" "$(cat /sys/fs/cgroup/cpu.weight)"
      awk '\''$1=="anon" || $1=="file" || $1=="shmem" {printf "memory_stat_%s_bytes=%s ",$1,$2}'\'' /sys/fs/cgroup/memory.stat
      printf "\n"
      cat /sys/fs/cgroup/memory.events
      exit "$result"
    ' >"$logs/container-id.txt"
  service_status_monitor_container "$name" plantuml "$logs/container-metrics.log" &
  monitor=$!
  if timeout --signal=TERM 300s docker wait "$name" >"$logs/container-exit.txt"; then :; else exit $?; fi
  docker logs "$name" >"$logs/render.log" 2>&1
  result="$(cat "$logs/container-exit.txt")"
  [[ "$result" =~ ^[0-9]+$ ]] || exit 125
  exit "$result"
fi

service_status_assert_no_runtime_sets "$qa_root"
service_status_preflight "$qa_root" local-diagrams 1024
owner="diagrams:$$"
service_status_acquire_slot_lock "$qa_root" "$owner"
trap 'service_status_release_slot_lock "$qa_root" "$owner"' EXIT
artifact="$(service_status_artifact_root "$qa_root")/diagrams"
mkdir -m 700 -p "$artifact/history"
cp -p "$repo_root/docs/diagrams/component_architecture.png" "$repo_root/docs/diagrams/runtime_deployment.png" "$artifact/history/"
unit="lagrange-service-status-wp5-diagrams-$$.service"
service_status_require_unused_unit "$unit"
systemd-run --user --quiet --wait --unit="$unit" \
  -p Type=exec -p WorkingDirectory="$repo_root" \
  -p "ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $logs/receipt.tsv 805306368 $unit" \
  -p "ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $logs/receipt.tsv 805306368 $unit" \
  -p MemoryMax=768M -p MemorySwapMax=0 -p MemoryAccounting=yes -p CPUWeight=10 \
  -p OOMPolicy=kill -p RuntimeMaxSec=90min -p TimeoutStopSec=30s \
  -p "StandardOutput=append:$logs/unit.log" -p "StandardError=append:$logs/unit.log" \
  -- env PATH="$PATH" SERVICE_STATUS_QA_ROOT="$qa_root" \
  bash "$repo_root/scripts/qa/service-status-unit-runner.sh" "$logs/renderer.pid" "$logs/unit-cgroup.tsv" 805306368 \
  bash "$repo_root/scripts/qa/service-status-diagrams.sh" --in-unit &
runner=$!
service_status_wait_unit local-diagrams "$unit" "$logs/unit-metrics.log" "$runner"
cp -p "$repo_root/docs/diagrams/component_architecture.png" "$repo_root/docs/diagrams/runtime_deployment.png" "$artifact/"
