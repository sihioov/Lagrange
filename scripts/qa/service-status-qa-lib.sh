#!/usr/bin/env bash

service_status_artifact_root() {
  local qa_root="$1" artifact_root="${SERVICE_STATUS_QA_ARTIFACT_ROOT:-}"
  python3 - "$repo_root" "$qa_root" "$artifact_root" <<'PY'
import json, os, re, stat, sys
from pathlib import Path
repo, qa, artifact = map(Path, sys.argv[1:])
prefix = repo / 'test-results/service-status-qa'
if not re.fullmatch(r'lagrange-service-status-wp5-[A-Za-z0-9._-]+', artifact.name) or artifact.parent != prefix:
    raise SystemExit('Artifact root must be one unique directory under repository test-results/service-status-qa')
for path in [repo / 'test-results', prefix, artifact]:
    if not path.exists(): path.mkdir(mode=0o700)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or path.resolve() != path:
        raise SystemExit('Unsafe QA artifact directory or symlink ancestor')
index = qa / 'artifact-index.json'
value = {'qa_root': str(qa), 'artifact_root': str(artifact), 'credentials': 'private /tmp only'}
if index.exists() and json.loads(index.read_text()) != value:
    raise SystemExit('Refusing a changed artifact root for this QA run')
index.write_text(json.dumps(value, indent=2) + '\n')
print(artifact)
PY
}

service_status_memory_stat() {
  local file="$1/memory.stat"
  awk 'BEGIN {anon="unavailable"; file="unavailable"; shmem="unavailable"}
    $1=="anon" && $2~/^[0-9]+$/ {anon=$2}
    $1=="file" && $2~/^[0-9]+$/ {file=$2}
    $1=="shmem" && $2~/^[0-9]+$/ {shmem=$2}
    END {printf "memory_stat_anon_bytes=%s memory_stat_file_bytes=%s memory_stat_shmem_bytes=%s",anon,file,shmem}' "$file" 2>/dev/null || true
}

service_status_browser_idle() {
  local unit="$1" pid_file="$2" evidence="$3" state pid process_state cgroup_path
  state="$(systemctl --user show "$unit" --property=ActiveState --value 2>/dev/null || true)"
  case "$state" in active|activating|deactivating|reloading) return 1 ;; esac
  pid="$(cat "$pid_file")"
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
  process_state="$(ps -o stat= -p "$pid" 2>/dev/null | awk '{print $1}' || true)"
  [[ -z "$process_state" || "$process_state" == Z* ]] || return 1
  cgroup_path="$(systemctl --user show "$unit" --property=ControlGroup --value 2>/dev/null || true)"
  if [[ -n "$cgroup_path" ]]; then
    [[ "$cgroup_path" == /user.slice/* && "$cgroup_path" != *..* ]] || return 1
    if [[ -f "/sys/fs/cgroup$cgroup_path/cgroup.procs" && -r "/sys/fs/cgroup$cgroup_path/cgroup.procs" ]]; then
      [[ -z "$(cat "/sys/fs/cgroup$cgroup_path/cgroup.procs")" ]] || return 1
    fi
  fi
  printf 'browser_lifetime=disposed unit=%s main_pid=%s pid_state=%s unit_state=%s cgroup_processes=absent_or_empty time_utc=%s\n' \
    "$unit" "$pid" "${process_state:-absent}" "${state:-not-found}" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$evidence"
}

service_status_validate_qa_root() {
  local qa_root="$1"
  if [[ ! "$qa_root" =~ ^/tmp/lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*$ || -L "$qa_root" || ! -d "$qa_root" ]]; then
    echo "Set SERVICE_STATUS_QA_ROOT to the existing unique WP-5 /tmp task directory." >&2
    return 2
  fi
  if [[ "$(stat -c '%u:%a' "$qa_root")" != "$(id -u):700" ]]; then
    echo "The WP-5 QA root must be owned by this user with mode 0700." >&2
    return 2
  fi
}

service_status_require_slot() {
  if [[ "${SERVICE_STATUS_QA_SLOT_CONFIRMED:-}" != yes ]]; then
    echo "Wait for the coordinator's explicit heavy-slot handoff, then set SERVICE_STATUS_QA_SLOT_CONFIRMED=yes." >&2
    return 2
  fi
  if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
    echo "An active Cargo/Rust compiler exists; stop before starting this QA stage." >&2
    return 2
  fi
}

service_status_acquire_slot_lock() {
  local qa_root="$1" owner="$2" lock="$1/slot.lock"
  if [[ -L "$lock" ]]; then
    echo "Refusing a symlink at the WP-5 serial slot lock." >&2
    return 2
  fi
  if ! mkdir -m 700 "$lock" 2>/dev/null; then
    if [[ -d "$lock" && ! -L "$lock" && -f "$lock/owner.txt" && ! -L "$lock/owner.txt" ]]; then
      printf 'Another WP-5 stage owns the serial slot: %s\n' "$(cat "$lock/owner.txt")" >&2
    else
      echo "Another or invalid WP-5 serial slot lock already exists." >&2
    fi
    return 2
  fi
  printf 'owner=%s\npid=%s\n' "$owner" "$$" >"$lock/owner.txt"
}

service_status_release_slot_lock() {
  local qa_root="$1" owner="$2" lock="$1/slot.lock" recorded
  [[ -d "$lock" && ! -L "$lock" && -f "$lock/owner.txt" && ! -L "$lock/owner.txt" ]] || return 0
  recorded="$(sed -n 's/^owner=//p' "$lock/owner.txt")"
  [[ "$recorded" == "$owner" ]] || return 0
  rm -f "$lock/owner.txt"
  rmdir "$lock" 2>/dev/null || true
}

service_status_require_unused_unit() {
  local unit="$1" load_state
  load_state="$(systemctl --user show "$unit" --property=LoadState --value 2>/dev/null || true)"
  if [[ -n "$load_state" && "$load_state" != not-found ]]; then
    printf 'Refusing to reuse a systemd unit that is already registered: %s (%s)\n' "$unit" "$load_state" >&2
    return 2
  fi
}

service_status_stop_unit() {
  local unit="$1" stage="$2" evidence_file="$3" load_state active_state
  service_status_capture_unit "$stage-before-stop" "$unit" "$evidence_file"
  load_state="$(systemctl --user show "$unit" --property=LoadState --value 2>/dev/null || true)"
  if [[ -z "$load_state" || "$load_state" == not-found ]]; then return 0; fi
  if ! systemctl --user stop "$unit" >/dev/null 2>&1; then
    service_status_capture_unit "$stage-stop-failed" "$unit" "$evidence_file"
    return 1
  fi
  active_state="$(systemctl --user show "$unit" --property=ActiveState --value 2>/dev/null || true)"
  service_status_capture_unit "$stage-after-stop" "$unit" "$evidence_file"
  case "$active_state" in
    active|activating|deactivating|reloading) return 1 ;;
  esac
}

service_status_assert_no_runtime_sets() {
  local qa_root="$1"
  for command in docker systemctl; do
    command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; return 2; }
  done
  local units containers
  units="$(systemctl --user list-units --state=active --type=service --plain --no-legend 'lagrange-service-status-wp5-*' 2>/dev/null | awk '{print $1}' || true)"
  containers="$(docker ps --all --quiet --filter label=com.docker.compose.project=lagrange-service-status-wp5 2>/dev/null || true)"
  if [[ -n "$units" || -n "$containers" ]]; then
    printf 'WP-5 runtime sets must be stopped before this build stage; see %s/ownership.txt\n' "$qa_root" >&2
    [[ -z "$units" ]] || printf 'active_owned_units=%s\n' "${units//$'\n'/,}" >&2
    [[ -z "$containers" ]] || printf 'owned_containers=%s\n' "${containers//$'\n'/,}" >&2
    return 2
  fi
}

service_status_preflight() {
  local qa_root="$1" stage="$2" required_mib="$3"
  local available_kib available_mib timestamp record
  service_status_require_slot || return $?
  available_kib="$(awk '/MemAvailable:/ { print $2; exit }' /proc/meminfo)"
  if [[ ! "$available_kib" =~ ^[0-9]+$ ]]; then
    echo "Could not read MemAvailable for stage $stage." >&2
    return 2
  fi
  available_mib=$((available_kib / 1024))
  timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record="stage=$stage available_mib=$available_mib required_mib=$required_mib time_utc=$timestamp"
  printf '%s\n' "$record" | tee -a "$qa_root/preflight.tsv"
  if (( available_kib < required_mib * 1024 )); then
    echo "Preflight failed safely: $record; do not stop unrelated services, change swap/cache, or raise caps." >&2
    return 2
  fi
}

service_status_capture_unit() {
  local stage="$1" unit="$2" evidence_file="$3" properties load_state control_group cgroup_dir
  local max current peak swap_max events cpu_weight timestamp
  timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  load_state="$(systemctl --user show "$unit" --property=LoadState --value 2>/dev/null || true)"
  properties="$(TZ=UTC LC_ALL=C systemctl --user show "$unit" \
    --property=ActiveState --property=SubState --property=Result \
    --property=ExecMainCode --property=ExecMainStatus \
    --property=ExecMainStartTimestamp --property=ExecMainExitTimestamp \
    --property=MemoryAccounting --property=MemoryCurrent --property=MemoryPeak \
    --property=MemoryMax --property=MemorySwapMax --property=CPUWeight \
    --property=OOMPolicy --property=RuntimeMaxUSec --property=ControlGroup 2>/dev/null || true)"
  control_group="$(sed -n 's/^ControlGroup=//p' <<<"$properties" | head -n 1)"
  max=unavailable
  current=unavailable
  peak=unavailable
  swap_max=unavailable
  events=unavailable
  cpu_weight=unavailable
  if [[ "$control_group" == /user.slice/* && "$control_group" != *..* ]]; then
    cgroup_dir="/sys/fs/cgroup$control_group"
    [[ -r "$cgroup_dir/memory.max" ]] && max="$(cat "$cgroup_dir/memory.max" 2>/dev/null || printf unavailable)"
    [[ -r "$cgroup_dir/memory.current" ]] && current="$(cat "$cgroup_dir/memory.current" 2>/dev/null || printf unavailable)"
    [[ -r "$cgroup_dir/memory.peak" ]] && peak="$(cat "$cgroup_dir/memory.peak" 2>/dev/null || printf unavailable)"
    [[ -r "$cgroup_dir/memory.swap.max" ]] && swap_max="$(cat "$cgroup_dir/memory.swap.max" 2>/dev/null || printf unavailable)"
    [[ -r "$cgroup_dir/memory.events" ]] && events="$(cat "$cgroup_dir/memory.events" 2>/dev/null || printf unavailable)"
    [[ -r "$cgroup_dir/cpu.weight" ]] && cpu_weight="$(cat "$cgroup_dir/cpu.weight" 2>/dev/null || printf unavailable)"
  fi
  {
    printf 'stage=%s unit=%s time_utc=%s\n' "$stage" "$unit" "$timestamp"
    if [[ -n "$properties" && "$load_state" != not-found ]]; then
      sed 's/^/  /' <<<"$properties"
      printf '  cgroup_memory_max_bytes=%s cgroup_memory_current_bytes=%s cgroup_memory_peak_bytes=%s cgroup_swap_max_bytes=%s\n' \
        "$max" "$current" "$peak" "$swap_max"
      printf '  cgroup_cpu_weight=%s cgroup_memory_events=%s\n' "$cpu_weight" "${events//$'\n'/,}"
      [[ -z "$control_group" ]] || { printf '  '; service_status_memory_stat "/sys/fs/cgroup$control_group"; printf '\n'; }
    else
      printf '  unit_metrics=unavailable load_state=%s\n' "${load_state:-unknown}"
    fi
  } >>"$evidence_file"
}

service_status_wait_unit() {
  local stage="$1" unit="$2" evidence_file="$3" runner_pid="$4"
  local active_state runner_state active_seen=0 result attempt
  for ((attempt = 0; attempt < 21600; attempt += 1)); do
    active_state="$(systemctl --user show "$unit" --property=ActiveState --value 2>/dev/null || true)"
    case "$active_state" in
      active|activating|deactivating)
        active_seen=1
        service_status_capture_unit "$stage-sample" "$unit" "$evidence_file"
        ;;
      inactive|failed|not-found|"")
        if (( active_seen )); then break; fi
        runner_state="$(ps -o stat= -p "$runner_pid" 2>/dev/null | awk '{print $1}' || true)"
        if [[ -z "$runner_state" || "$runner_state" == Z* ]]; then break; fi
        ;;
    esac
    sleep 0.25
  done
  if wait "$runner_pid"; then result=0; else result=$?; fi
  printf 'stage=%s unit=%s systemd_run_exit=%s time_utc=%s\n' "$stage" "$unit" "$result" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$evidence_file"
  service_status_capture_unit "$stage-final" "$unit" "$evidence_file"
  return "$result"
}

service_status_container_cgroup_dir() {
  local container_id="$1" pid="$2" cgroup full_id candidate
  if [[ "$pid" =~ ^[1-9][0-9]*$ && -r "/proc/$pid/cgroup" ]]; then
    cgroup="$(awk -F: '$1 == "0" { print $3; exit }' "/proc/$pid/cgroup")"
    if [[ "$cgroup" == /* && "$cgroup" != *..* && -r "/sys/fs/cgroup$cgroup/memory.peak" ]]; then
      printf '/sys/fs/cgroup%s\n' "$cgroup"
      return 0
    fi
  fi
  full_id="$(docker inspect --format '{{.Id}}' "$container_id" 2>/dev/null || true)"
  if [[ "$full_id" =~ ^[a-f0-9]{64}$ ]]; then
    for candidate in \
      "/sys/fs/cgroup/system.slice/docker-${full_id}.scope" \
      "/sys/fs/cgroup/docker.slice/docker-${full_id}.scope" \
      "/sys/fs/cgroup/docker/$full_id" \
      "/sys/fs/cgroup/$full_id"; do
      if [[ -r "$candidate/memory.peak" ]]; then printf '%s\n' "$candidate"; return 0; fi
    done
  fi
  return 1
}

service_status_verify_next_static() {
  local web_url="$1" asset code
  asset="$(curl --silent --show-error --max-time 15 "$web_url/" 2>/dev/null \
    | grep -oE '/_next/static/[^"? ]+\.js' | head -n 1 || true)"
  if [[ ! "$asset" =~ ^/_next/static/[A-Za-z0-9_./-]+\.js$ ]]; then
    echo "Production Next HTML did not expose a local JavaScript asset URL." >&2
    return 1
  fi
  code="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' "$web_url$asset" 2>/dev/null || true)"
  if [[ "$code" != 200 ]]; then
    echo "Production Next static asset failed: path=$asset http_status=${code:-unavailable}." >&2
    return 1
  fi
}

service_status_next_server_root() {
  local output_root="$1" root_server=0 nested_server=0
  if [[ -f "$output_root/server.js" && -d "$output_root/.next/static" ]]; then root_server=1; fi
  if [[ -f "$output_root/apps/web/server.js" && -d "$output_root/apps/web/.next/static" ]]; then nested_server=1; fi
  if (( root_server + nested_server != 1 )); then
    echo "The staged standalone tree must contain one unambiguous server.js and matching static assets." >&2
    return 1
  fi
  if (( root_server )); then printf '%s\n' "$output_root"; else printf '%s\n' "$output_root/apps/web"; fi
}

service_status_container_snapshot() {
  local container_id="$1" label="$2" evidence_file="$3"
  local inspected status oom exit_code limit swap_limit pid started finished cgroup_dir cgroup_max peak current swap_max events timestamp
  timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  inspected="$(docker inspect --format '{{.State.Status}} {{.State.OOMKilled}} {{.State.ExitCode}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}} {{.State.Pid}} {{.State.StartedAt}} {{.State.FinishedAt}}' "$container_id" 2>/dev/null || true)"
  if [[ -z "$inspected" ]]; then
    printf 'container=%s id=%s time_utc=%s state=unavailable oom=unavailable exit=unavailable memory_limit=unavailable peak_bytes=unavailable\n' \
      "$label" "$container_id" "$timestamp" >>"$evidence_file"
    return
  fi
  read -r status oom exit_code limit swap_limit pid started finished <<<"$inspected"
  peak=unavailable
  current=unavailable
  cgroup_max=unavailable
  swap_max=unavailable
  events=unavailable
  cgroup_dir="$(service_status_container_cgroup_dir "$container_id" "$pid" || true)"
  if [[ -n "$cgroup_dir" ]]; then
    peak="$(cat "$cgroup_dir/memory.peak" 2>/dev/null || printf unavailable)"
    cgroup_max="$(cat "$cgroup_dir/memory.max" 2>/dev/null || printf unavailable)"
    current="$(cat "$cgroup_dir/memory.current" 2>/dev/null || printf unavailable)"
    swap_max="$(cat "$cgroup_dir/memory.swap.max" 2>/dev/null || printf unavailable)"
    events="$(cat "$cgroup_dir/memory.events" 2>/dev/null || printf unavailable)"
  fi
  printf 'container=%s id=%s time_utc=%s started_at_utc=%s finished_at_utc=%s state=%s oom=%s exit=%s memory_limit_bytes=%s memory_and_swap_limit_bytes=%s cgroup_memory_max_bytes=%s cgroup_swap_max_bytes=%s current_bytes=%s peak_bytes=%s memory_events=%s\n' \
    "$label" "$container_id" "$timestamp" "$started" "$finished" "$status" "$oom" "$exit_code" \
    "$limit" "$swap_limit" "$cgroup_max" "$swap_max" "$current" "$peak" "${events//$'\n'/,}" >>"$evidence_file"
  [[ -z "$cgroup_dir" ]] || { printf '  '; service_status_memory_stat "$cgroup_dir"; printf '\n'; } >>"$evidence_file"
}

service_status_remove_container() {
  local container_id="$1" label="$2" evidence_file="$3" state
  if ! docker container inspect "$container_id" >/dev/null 2>&1; then return 0; fi
  service_status_container_snapshot "$container_id" "$label" "$evidence_file"
  state="$(docker inspect --format '{{.State.Status}}' "$container_id" 2>/dev/null || true)"
  if [[ "$state" == running || "$state" == paused ]]; then
    if ! docker stop --time 30 "$container_id" >/dev/null 2>&1; then
      service_status_container_snapshot "$container_id" "$label-stop-failed" "$evidence_file"
      return 1
    fi
  fi
  service_status_container_snapshot "$container_id" "$label-final-before-remove" "$evidence_file"
  if ! docker rm "$container_id" >/dev/null 2>&1; then
    service_status_container_snapshot "$container_id" "$label-remove-failed" "$evidence_file"
    return 1
  fi
  if docker container inspect "$container_id" >/dev/null 2>&1; then
    service_status_container_snapshot "$container_id" "$label-remains" "$evidence_file"
    return 1
  fi
}

service_status_monitor_container() {
  local container_id="$1" label="$2" evidence_file="$3"
  local previous_peak=0 inspected status oom exit_code limit pid cgroup_dir peak
  while :; do
    inspected="$(docker inspect --format '{{.State.Status}} {{.State.OOMKilled}} {{.State.ExitCode}} {{.HostConfig.Memory}} {{.State.Pid}}' "$container_id" 2>/dev/null || true)"
    [[ -n "$inspected" ]] || break
    read -r status oom exit_code limit pid <<<"$inspected"
    cgroup_dir="$(service_status_container_cgroup_dir "$container_id" "$pid" || true)"
    if [[ -n "$cgroup_dir" ]]; then
      peak="$(cat "$cgroup_dir/memory.peak" 2>/dev/null || printf unavailable)"
    else
      peak=unavailable
    fi
    if [[ "$peak" =~ ^[0-9]+$ ]] && (( peak > previous_peak )); then
      service_status_container_snapshot "$container_id" "$label" "$evidence_file"
      previous_peak="$peak"
    elif [[ "$status" != running || "$oom" == true || "$exit_code" != 0 ]]; then
      service_status_container_snapshot "$container_id" "$label" "$evidence_file"
    fi
    [[ "$status" == running ]] || break
    sleep 0.2
  done
}

service_status_prepare_next() {
  local qa_root="$1" lane="$2" api_url="$3" quotes_mode="$4" quote_transport="$5"
  local lane_root="$qa_root/$lane" output_root
  output_root="$(service_status_artifact_root "$qa_root")/$lane/next-standalone"
  local logs="$lane_root/logs" unit log metrics result runner_pid cap_bytes pid_file cgroup_metrics
  if [[ "$lane" != web-off && "$lane" != market-stream && "$lane" != actual ]]; then
    echo "Unknown service-status browser build lane." >&2
    return 2
  fi
  command -v npm >/dev/null || { echo "required QA command is unavailable: npm" >&2; return 2; }
  service_status_assert_no_runtime_sets "$qa_root" || return $?
  service_status_preflight "$qa_root" "$lane-next-production-build" 1792 || return $?
  mkdir -m 700 -p "$logs"
  unit="lagrange-service-status-wp5-${lane}-next-build-$$.service"
  log="$logs/next-build.log"
  metrics="$logs/unit-metrics.log"
  cap_bytes=1610612736
  pid_file="$logs/next-build.pid"
  cgroup_metrics="$logs/next-build-cgroup.tsv"
  service_status_require_unused_unit "$unit" || return $?
  if [[ -e "$output_root" || -L "$output_root" ]]; then
    if [[ -L "$output_root" ]]; then
      echo "Refusing a symlink at the owned standalone staging path." >&2
      return 2
    fi
    rm -rf -- "$output_root"
  fi
  # Next 16.3 derives its page-worker default from CIRCLE_NODE_TOTAL minus one.
  # Use one page worker within the fixed 1536 MiB QA build cap.
  systemd-run --user --quiet --wait --unit="$unit" \
    --property=Type=exec \
    --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $logs/next-build-receipt.tsv $cap_bytes $unit" \
    --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $logs/next-build-receipt.tsv $cap_bytes $unit" \
    --property=TimeoutStopSec=30s \
    --property=WorkingDirectory="$repo_root" \
    --property=MemoryMax=1536M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
    --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
    --property="StandardOutput=append:$log" \
    --property="StandardError=append:$log" \
    -- env PATH="$PATH" API_INTERNAL_URL="$api_url" \
      OWNER_INTRADAY_QUOTES_MODE="$quotes_mode" OWNER_INTRADAY_QUOTE_TRANSPORT="$quote_transport" \
      NODE_OPTIONS=--max-old-space-size=1024 NEXT_TELEMETRY_DISABLED=1 CIRCLE_NODE_TOTAL=2 \
      bash "$repo_root/scripts/qa/service-status-unit-runner.sh" \
      "$pid_file" "$cgroup_metrics" "$cap_bytes" npm run build --workspace=@lagrange/web &
  runner_pid=$!
  if service_status_wait_unit "$lane-next-production-build" "$unit" "$metrics" "$runner_pid"; then result=0; else result=$?; fi
  if (( result != 0 )); then
    printf 'Next build failed: lane=%s unit=%s exit=%s log=%s metrics=%s\n' "$lane" "$unit" "$result" "$log" "$metrics" >&2
    return "$result"
  fi
  local source_standalone="$repo_root/apps/web/.next/standalone" source_server_relative staged_server_root
  if [[ -f "$source_standalone/server.js" ]]; then
    source_server_relative=.
  elif [[ -f "$source_standalone/apps/web/server.js" ]]; then
    source_server_relative=apps/web
  else
    echo "The standalone build did not produce its server.js; no runtime was started." >&2
    return 1
  fi
  mkdir -m 700 -p "$output_root"
  cp -a "$source_standalone/." "$output_root/"
  staged_server_root="$output_root/$source_server_relative"
  mkdir -p "$staged_server_root/.next"
  rm -rf -- "$staged_server_root/.next/static"
  cp -a "$repo_root/apps/web/.next/static" "$staged_server_root/.next/static"
  if [[ -d "$repo_root/apps/web/public" ]]; then
    rm -rf -- "$staged_server_root/public"
    cp -a "$repo_root/apps/web/public" "$staged_server_root/public"
  fi
  service_status_next_server_root "$output_root" >/dev/null || return 1
  printf 'lane=%s api_internal_url=%s owner_quotes_mode=%s owner_quote_transport=%s standalone_root=%s build_unit=%s\n' \
    "$lane" "$api_url" "$quotes_mode" "$quote_transport" "$output_root" "$unit" \
    >>"$qa_root/ownership.txt"
}
