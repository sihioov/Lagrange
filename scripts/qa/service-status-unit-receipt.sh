#!/usr/bin/env bash
set -euo pipefail
umask 077

# Runs inside the owned unit before work, and in ExecStopPost before transient GC.
# Retain the original main-process result before transient-unit GC.
phase="${1:-}"
metrics_file="${2:-}"
expected_max="${3:-}"
unit="${4:-}"
[[ "$phase" == start || "$phase" == end ]] || exit 125
[[ "$metrics_file" =~ ^/tmp/lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9/._-]+$ ]] || exit 125
[[ "$expected_max" =~ ^(134217728|268435456|805306368|1610612736)$ ]] || exit 125
[[ "$unit" =~ ^lagrange-service-status-wp5-[A-Za-z0-9-]+\.service$ ]] || exit 125
qa_basename="${metrics_file#/tmp/}"
qa_root="/tmp/${qa_basename%%/*}"
metrics_parent="$(dirname "$metrics_file")"
[[ ! -L "$metrics_file" && "$(stat -c '%u:%a' "$qa_root")" == "$(id -u):700" ]] || exit 125
[[ "$(realpath -e "$metrics_parent")" == "$metrics_parent" ]] || exit 125

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cgroup_path="$(awk -F: '$1 == "0" { print $3; exit }' /proc/self/cgroup)"
if [[ "$cgroup_path" != /user.slice/* || "$cgroup_path" == *..* ]]; then
  printf 'phase=%s time_utc=%s unit=%s receipt=unavailable reason=unsafe_cgroup_path\n' \
    "$phase" "$timestamp" "$unit" >>"$metrics_file"
  exit 125
fi
cgroup_dir="/sys/fs/cgroup$cgroup_path"
for property in memory.max memory.swap.max memory.current memory.peak memory.events cpu.weight; do
  if [[ ! -r "$cgroup_dir/$property" ]]; then
    printf 'phase=%s time_utc=%s unit=%s receipt=unavailable reason=missing_controller\n' \
      "$phase" "$timestamp" "$unit" >>"$metrics_file"
    exit 125
  fi
done
observed_max="$(cat "$cgroup_dir/memory.max")"
observed_swap="$(cat "$cgroup_dir/memory.swap.max")"
observed_cpu_weight="$(cat "$cgroup_dir/cpu.weight")"
peak="$(cat "$cgroup_dir/memory.peak")"
current="$(cat "$cgroup_dir/memory.current")"
events="$(cat "$cgroup_dir/memory.events")"
memory_stat="$(awk '$1=="anon" || $1=="file" || $1=="shmem" {if ($2~/^[0-9]+$/) printf "memory_stat_%s_bytes=%s ",$1,$2}' "$cgroup_dir/memory.stat")"
[[ "$memory_stat" == *memory_stat_anon_bytes=* && "$memory_stat" == *memory_stat_file_bytes=* && "$memory_stat" == *memory_stat_shmem_bytes=* ]] || exit 125
receipt=verified
if [[ "$observed_max" != "$expected_max" || "$observed_swap" != 0 || "$observed_cpu_weight" != 10 ]]; then
  receipt=cap_mismatch
fi
service_result=not_applicable
exit_code=not_applicable
exit_status=not_applicable
if [[ "$phase" == end ]]; then
  # These are systemd's original main-process result, not this receipt's exit.
  service_result="${SERVICE_RESULT:-unavailable}"
  exit_code="${EXIT_CODE:-unavailable}"
  exit_status="${EXIT_STATUS:-unavailable}"
fi
printf 'phase=%s time_utc=%s unit=%s receipt=%s expected_memory_max_bytes=%s observed_memory_max_bytes=%s observed_swap_max_bytes=%s observed_cpu_weight=%s memory_peak_bytes=%s memory_current_bytes=%s original_service_result=%s original_exit_code=%s original_exit_status=%s memory_events=%s\n' \
  "$phase" "$timestamp" "$unit" "$receipt" "$expected_max" "$observed_max" "$observed_swap" \
  "$observed_cpu_weight" "$peak" "$current" "$service_result" "$exit_code" "$exit_status" \
  "${events//$'\n'/,}" >>"$metrics_file"
printf '  %s\n' "$memory_stat" >>"$metrics_file"

# Explicit safe properties only; never read Environment, ExecStart or journal bodies.
if properties="$(TZ=UTC LC_ALL=C systemctl --user show "$unit" \
  --property=LoadState --property=ActiveState --property=SubState --property=Result \
  --property=ExecMainCode --property=ExecMainStatus \
  --property=ExecMainStartTimestamp --property=ExecMainExitTimestamp \
  --property=MemoryMax --property=MemorySwapMax --property=MemoryPeak \
  --property=CPUWeight --property=RuntimeMaxUSec --property=OOMPolicy 2>/dev/null)"; then
  sed 's/^/  /' <<<"$properties" >>"$metrics_file"
else
  printf '  main_process_timestamps=unavailable\n' >>"$metrics_file"
fi
[[ "$receipt" == verified ]] || exit 125
