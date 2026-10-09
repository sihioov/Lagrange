#!/usr/bin/env bash
set -u
umask 077

pid_file="${1:-}"
metrics_file="${2:-}"
expected_max="${3:-}"
shift 3 || exit 125

qa_path_pattern='^/tmp/lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*/'
case "$pid_file" in
  /tmp/lagrange-service-status-wp5-*) ;;
  *) exit 125 ;;
esac
case "$metrics_file" in
  /tmp/lagrange-service-status-wp5-*) ;;
  *) exit 125 ;;
esac
[[ "$pid_file" =~ $qa_path_pattern && "$metrics_file" =~ $qa_path_pattern ]] || exit 125
[[ "$expected_max" =~ ^(134217728|268435456|805306368|1610612736)$ ]] || exit 125
[[ "$#" -gt 0 && ! -L "$pid_file" && ! -L "$metrics_file" ]] || exit 125

cgroup_path="$(awk -F: '$1 == "0" { print $3; exit }' /proc/self/cgroup)"
if [[ "$cgroup_path" != /user.slice/* || "$cgroup_path" == *..* ]]; then
  printf 'unit_cgroup result=unavailable reason=unsafe_cgroup_path\n' >>"$metrics_file"
  exit 125
fi
cgroup_dir="/sys/fs/cgroup$cgroup_path"
if [[ ! -r "$cgroup_dir/memory.max" || ! -r "$cgroup_dir/memory.swap.max" || ! -r "$cgroup_dir/memory.peak" || ! -r "$cgroup_dir/memory.events" ]]; then
  printf 'unit_cgroup result=unavailable reason=missing_memory_controller\n' >>"$metrics_file"
  exit 125
fi

observed_max="$(cat "$cgroup_dir/memory.max")"
observed_swap_max="$(cat "$cgroup_dir/memory.swap.max")"
events_before="$(cat "$cgroup_dir/memory.events")"
if [[ "$observed_max" != "$expected_max" || "$observed_swap_max" != 0 ]]; then
  printf 'unit_cgroup result=cap_mismatch expected_memory_max_bytes=%s observed_memory_max_bytes=%s observed_swap_max_bytes=%s\n' \
    "$expected_max" "$observed_max" "$observed_swap_max" >>"$metrics_file"
  exit 125
fi

printf 'unit_cgroup result=started time_utc=%s expected_memory_max_bytes=%s observed_memory_max_bytes=%s observed_swap_max_bytes=%s events_before=%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$expected_max" "$observed_max" "$observed_swap_max" \
  "${events_before//$'\n'/,}" >>"$metrics_file"
"$@" &
child_pid=$!
printf '%s\n' "$child_pid" >"$pid_file"
if wait "$child_pid"; then result=0; else result=$?; fi
peak="$(cat "$cgroup_dir/memory.peak")"
current="$(cat "$cgroup_dir/memory.current" 2>/dev/null || printf unavailable)"
events_after="$(cat "$cgroup_dir/memory.events")"
memory_stat="$(awk '$1=="anon" || $1=="file" || $1=="shmem" {if ($2~/^[0-9]+$/) printf "memory_stat_%s_bytes=%s ",$1,$2}' "$cgroup_dir/memory.stat")"
oom="$(awk '$1 == "oom" { print $2 }' <<<"$events_after")"
oom_kill="$(awk '$1 == "oom_kill" { print $2 }' <<<"$events_after")"
printf 'unit_cgroup result=complete time_utc=%s child_exit=%s expected_memory_max_bytes=%s observed_memory_max_bytes=%s observed_swap_max_bytes=%s memory_peak_bytes=%s memory_current_after_bytes=%s oom_events=%s oom_kill_events=%s events_before=%s events_after=%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$result" "$expected_max" "$observed_max" "$observed_swap_max" "$peak" "$current" \
  "${oom:-unavailable}" "${oom_kill:-unavailable}" "${events_before//$'\n'/,}" "${events_after//$'\n'/,}" >>"$metrics_file"
printf '  %s\n' "$memory_stat" >>"$metrics_file"
exit "$result"
