#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
if [[ ! "$qa_root" =~ ^/tmp/lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*$ || -L "$qa_root" ]]; then
  echo "Set SERVICE_STATUS_QA_ROOT to a unique /tmp/lagrange-service-status-wp5-* directory." >&2
  exit 2
fi
if [[ ! -e "$qa_root" ]]; then mkdir -m 700 "$qa_root"; fi
if [[ ! -d "$qa_root" || "$(stat -c '%u:%a' "$qa_root")" != "$(id -u):700" ]]; then
  echo "The WP-5 QA root must be a nonsymlink directory owned by this user with mode 0700." >&2
  exit 2
fi

actual_root="$qa_root/actual"
if [[ "${1:-}" == --reuse-prebuilt ]]; then
  [[ -d "$actual_root" && ! -L "$actual_root" && "$(stat -c '%u:%a' "$actual_root")" == "$(id -u):700" ]]
else
  mkdir -m 700 "$actual_root"
fi
artifact_root="$(service_status_artifact_root "$qa_root")/actual"
mkdir -m 700 -p "$actual_root/logs" "$actual_root/pids"
mkdir -m 700 -p "$artifact_root/screenshots"
snapshot_root="$actual_root/host-snapshot"
handoff_root="$actual_root/handoff"
loader_root="$actual_root/loader-runtime"
session_file="$actual_root/owner-session.json"
proof_file="$actual_root/actual-api-proof.json"
audit_file="$handoff_root/audit-proof.json"
publisher="$repo_root/scripts/qa/service-status-snapshot-publisher.py"
binary="$repo_root/target/debug/examples/service-status-browser-harness"
project="lagrange-service-status-wp5"
api_container="lagrange-service-status-wp5-api"
image="postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
loader_bundle="$repo_root/scripts/qa/service-status-loader-bundle.py"
db_port=55445
api_port=38194
web_port=33044
db_override="$actual_root/qa-db-resource-override.yml"
web_url="http://127.0.0.1:${web_port}"
api_url="http://127.0.0.1:${api_port}"
next_unit="lagrange-service-status-wp5-next-$$.service"
web_unit_started=0
compose_created=0
api_created=0
browser_unit=""
ownership_file="$actual_root/ownership.txt"
loader_probe_container="lagrange-service-status-wp5-loader-$$"
loader_probe_started=0
loader_probe_id=""
loader_probe_monitor_pid=""
loader_fallback=0
loader_name=""
db_id=""
db_monitor_pid=""
api_monitor_pid=""
metrics_file="$actual_root/logs/runtime-metrics.log"
slot_owner=""

verify_container_caps() {
  local id="$1" label="$2" expected="$3" pid directory max swap weight
  pid="$(docker inspect --format '{{.State.Pid}}' "$id")"
  directory="$(service_status_container_cgroup_dir "$id" "$pid" || true)"
  if [[ -z "$directory" ]]; then
    printf 'container_live_caps label=%s id=%s state=unavailable-after-exit\n' "$label" "$id" >>"$metrics_file"
    [[ "$(docker inspect --format '{{.State.Status}}' "$id")" == exited ]]
    return
  fi
  max="$(cat "$directory/memory.max")"
  swap="$(cat "$directory/memory.swap.max")"
  weight="$(cat "$directory/cpu.weight")"
  printf 'container_live_caps label=%s id=%s expected_memory_max_bytes=%s memory_max_bytes=%s swap_max_bytes=%s cpu_weight=%s\n' \
    "$label" "$id" "$expected" "$max" "$swap" "$weight" >>"$metrics_file"
  [[ "$max" == "$expected" && "$swap" == 0 && "$weight" == 10 ]]
}

cleanup() {
  local status=$?
  local original_status="$status"
  local probe_ref="$loader_probe_id"
  local cleanup_failed=0 db_cleanup_ok=1
  trap - EXIT INT TERM
  if [[ -n "$browser_unit" ]]; then
    if ! service_status_stop_unit "$browser_unit" actual-browser "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( loader_probe_started )); then
    if [[ -n "$loader_probe_monitor_pid" ]]; then kill "$loader_probe_monitor_pid" >/dev/null 2>&1 || true; wait "$loader_probe_monitor_pid" 2>/dev/null || true; fi
    if [[ -z "$probe_ref" ]] && docker container inspect "$loader_probe_container" >/dev/null 2>&1; then probe_ref="$loader_probe_container"; fi
    if [[ -n "$probe_ref" ]] && ! service_status_remove_container "$probe_ref" loader-probe "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( web_unit_started )); then
    if ! service_status_stop_unit "$next_unit" actual-next "$metrics_file"; then cleanup_failed=1; fi
  fi
  if (( api_created )); then
    if [[ -n "$api_monitor_pid" ]]; then kill "$api_monitor_pid" >/dev/null 2>&1 || true; wait "$api_monitor_pid" 2>/dev/null || true; fi
    if service_status_remove_container "$api_container" api "$metrics_file"; then api_created=0; else cleanup_failed=1; fi
  fi
  if (( compose_created )); then
    if [[ -n "$db_monitor_pid" ]]; then kill "$db_monitor_pid" >/dev/null 2>&1 || true; wait "$db_monitor_pid" 2>/dev/null || true; fi
    if [[ -n "$db_id" ]]; then
      service_status_container_snapshot "$db_id" qa-db "$metrics_file"
    else
      while IFS= read -r candidate_id; do
        [[ -z "$candidate_id" ]] || service_status_container_snapshot "$candidate_id" qa-db "$metrics_file"
      done < <(LAGRANGE_QA_DB_PORT="$db_port" docker compose \
        --project-name "$project" \
        --file "$repo_root/deploy/qa/qa-db.compose.yml" \
        --file "$db_override" \
        ps --all --quiet qa-db 2>/dev/null || true)
    fi
    if ! LAGRANGE_QA_DB_PORT="$db_port" docker compose \
      --project-name "$project" \
      --file "$repo_root/deploy/qa/qa-db.compose.yml" \
      --file "$db_override" \
      stop --timeout 30 qa-db >/dev/null 2>&1; then
      cleanup_failed=1
      db_cleanup_ok=0
    fi
    if [[ -n "$db_id" ]]; then
      service_status_container_snapshot "$db_id" qa-db-final-before-remove "$metrics_file"
    fi
    if ! LAGRANGE_QA_DB_PORT="$db_port" docker compose \
      --project-name "$project" \
      --file "$repo_root/deploy/qa/qa-db.compose.yml" \
      --file "$db_override" \
      down --volumes --remove-orphans >/dev/null 2>&1; then
      cleanup_failed=1
      db_cleanup_ok=0
    fi
    remaining_project_containers="$(docker ps --all --quiet --filter "label=com.docker.compose.project=$project")"
    remaining_project_networks="$(docker network ls --quiet --filter "label=com.docker.compose.project=$project")"
    remaining_project_volumes="$(docker volume ls --quiet --filter "label=com.docker.compose.project=$project")"
    if [[ -n "$remaining_project_containers$remaining_project_networks$remaining_project_volumes" ]]; then
      cleanup_failed=1
      db_cleanup_ok=0
      printf 'Owned Compose resources remain after cleanup: project=%s containers=%s networks=%s volumes=%s\n' \
        "$project" "${remaining_project_containers//$'\n'/,}" "${remaining_project_networks//$'\n'/,}" \
        "${remaining_project_volumes//$'\n'/,}" >&2
    else
      compose_created=0
    fi
  fi
  if [[ -f "$publisher" && ( -d "$snapshot_root" || -d "$handoff_root" ) ]]; then
    if ! python3 "$publisher" cleanup "$snapshot_root" >/dev/null; then
      echo "Confined root-owned WP-5 QA fixture cleanup failed." >&2
      if (( status == 0 )); then status=1; fi
      cleanup_failed=1
    fi
  fi
  rm -f "$session_file"
  if (( db_cleanup_ok )); then rm -f "$db_override"; fi
  if (( cleanup_failed )); then
    if (( status == 0 )); then status=1; fi
    printf 'Owned cleanup is incomplete; retain the serial lock for review: %s\n' "$ownership_file" >&2
  elif [[ -n "$slot_owner" ]]; then
    service_status_release_slot_lock "$qa_root" "$slot_owner"
  fi
  if (( status != 0 )); then
    printf 'Actual API QA stopped with status=%s; owned identities=%s; container/unit peak/OOM evidence=%s\n' \
      "$status" "$ownership_file" "$metrics_file" >&2
  fi
  printf 'lane=actual original_exit=%s cleanup_failed=%s final_exit=%s time_utc=%s\n' \
    "$original_status" "$cleanup_failed" "$status" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$metrics_file"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ ! -x "$binary" ]]; then
  echo "Build the accepted QA example with the DB stopped before running this harness." >&2
  exit 2
fi
for command in docker head ldd node npm pgrep python3 readelf systemd-run systemctl curl jq timeout grep; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done
service_status_require_slot
slot_owner="actual:$$"
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
for port in "$db_port" "$api_port" "$web_port"; do check_free_port "$port"; done
if docker container inspect "$api_container" >/dev/null 2>&1; then
  echo "The reserved WP-5 API container name already exists; inspect the earlier task cleanup." >&2
  exit 2
fi
existing_project_containers="$(docker ps --all --quiet --filter "label=com.docker.compose.project=$project")"
existing_project_networks="$(docker network ls --quiet --filter "label=com.docker.compose.project=$project")"
existing_project_volumes="$(docker volume ls --quiet --filter "label=com.docker.compose.project=$project")"
if [[ -n "$existing_project_containers$existing_project_networks$existing_project_volumes" ]]; then
  echo "The reserved WP-5 Compose project already has Docker resources; inspect earlier cleanup." >&2
  exit 2
fi
if docker image inspect "$image" >/dev/null 2>&1; then :; else
  echo "The pinned QA image is not cached; this harness will not pull it." >&2
  exit 2
fi
if docker container inspect "$loader_probe_container" >/dev/null 2>&1; then
  echo "The reserved WP-5 loader-probe container name already exists; inspect earlier cleanup." >&2
  exit 2
fi

case "${1:-}" in
  "") service_status_prepare_next "$qa_root" actual "$api_url" off market_ws ;;
  --reuse-prebuilt)
    service_status_assert_no_runtime_sets "$qa_root"
    python3 - "$repo_root" "$qa_root" "$artifact_root/next-standalone" "$api_url" <<'PYCODE'
import hashlib, json, sys
from pathlib import Path
repo, qa, staged = map(Path, sys.argv[1:4]); api = sys.argv[4]
line = f'lane=actual api_internal_url={api} owner_quotes_mode=off owner_quote_transport=market_ws standalone_root={staged} build_unit='
assert any(x.startswith(line) for x in (qa/'ownership.txt').read_text().splitlines())
end = [x for x in (qa/'actual/logs/next-build-receipt.tsv').read_text().splitlines() if x.startswith('phase=end ')][-1]
assert 'original_service_result=success ' in end and 'original_exit_status=0 ' in end
assert 'observed_memory_max_bytes=1610612736 ' in end and 'oom 0,oom_kill 0,' in end
integrity = json.loads((qa/'accepted-integrity-before-runtime.json').read_text())
for name, expected in integrity['root_web'].items():
    assert hashlib.sha256((repo/name).read_bytes()).hexdigest() == expected, name
css = integrity['coordinator_css']; assert hashlib.sha256((repo/css['path']).read_bytes()).hexdigest() == css['sha256']
manifest = next(staged.rglob('routes-manifest.json')); rewrites = json.loads(manifest.read_text())['rewrites']
rules = rewrites if isinstance(rewrites,list) else sum(rewrites.values(), [])
assert any(x['source'].startswith('/api/') and x['destination'].startswith(api+'/api/') for x in rules)
print('reuse_prebuilt=verified lane=actual current_root_web_and_css_hashes=match original_build_exit=0')
PYCODE
    ;;
  *) echo "Choose the default build or --reuse-prebuilt after an owned harness-only repair." >&2; exit 2 ;;
esac
next_root="$artifact_root/next-standalone"
next_server_root="$(service_status_next_server_root "$next_root")"
printf 'compose_project=%s\nqa_db_port=%s\napi_container=%s\napi_port=%s\nweb_port=%s\nloader_probe_container=%s\nnext_root=%s\napi_internal_url=%s\nmode=off+market_ws\n' \
  "$project" "$db_port" "$api_container" "$api_port" "$web_port" "$loader_probe_container" "$next_root" "$api_url" >"$ownership_file"
loader_probe_started=1
if ! timeout 30s docker run --pull=never --detach --init \
  --name "$loader_probe_container" \
  --network none --cpus 0.5 --cpu-shares 48 --memory 256m --memory-swap 256m \
  --read-only --user 10001:10001 \
  --mount "type=bind,src=${binary},dst=/qa/service-status-browser-harness,readonly" \
  --entrypoint /bin/sh "$image" -c '
    set -eu
    printf "loader_live_caps memory_max_bytes=%s swap_max_bytes=%s cpu_weight=%s\n" "$(cat /sys/fs/cgroup/memory.max)" "$(cat /sys/fs/cgroup/memory.swap.max)" "$(cat /sys/fs/cgroup/cpu.weight)"
    test "$(cat /sys/fs/cgroup/memory.max)" = 268435456
    test "$(cat /sys/fs/cgroup/memory.swap.max)" = 0
    test "$(cat /sys/fs/cgroup/cpu.weight)" = 10
    set +e
    /usr/bin/ldd /qa/service-status-browser-harness
    original=$?
    printf "loader_original_exit=%s memory_peak_bytes=%s memory_events=%s\n" "$original" "$(cat /sys/fs/cgroup/memory.peak)" "$(tr "\n" "," </sys/fs/cgroup/memory.events)"
    awk '\''$1=="anon" || $1=="file" || $1=="shmem" {printf "loader_memory_stat_%s_bytes=%s\n",$1,$2}'\'' /sys/fs/cgroup/memory.stat
    exit "$original"
  ' \
  >"$actual_root/logs/loader-probe-start.log" 2>&1; then
  echo "The pinned cached image could not start its bounded loader probe; no DB has been started." >&2
  exit 2
fi
loader_probe_id="$(docker inspect --format '{{.Id}}' "$loader_probe_container")"
verify_container_caps "$loader_probe_id" loader-probe 268435456
service_status_monitor_container "$loader_probe_id" loader-probe "$metrics_file" &
loader_probe_monitor_pid=$!
printf 'loader_probe_id=%s\nloader_probe_peak_monitor_pid=%s\n' "$loader_probe_id" "$loader_probe_monitor_pid" >>"$ownership_file"
if timeout 30s docker wait "$loader_probe_id" >"$actual_root/logs/loader-probe-exit.txt"; then
  loader_probe_wait_status=0
else
  loader_probe_wait_status=$?
fi
docker logs "$loader_probe_id" >"$actual_root/logs/api-loader-check.log" 2>&1 || true
kill "$loader_probe_monitor_pid" >/dev/null 2>&1 || true
wait "$loader_probe_monitor_pid" 2>/dev/null || true
loader_probe_monitor_pid=""
service_status_container_snapshot "$loader_probe_id" loader-probe "$metrics_file"
loader_probe_oom="$(docker inspect --format '{{.State.OOMKilled}}' "$loader_probe_id" 2>/dev/null || printf unavailable)"
if [[ "$loader_probe_oom" == true || "$loader_probe_wait_status" == 124 ]]; then
  printf 'Loader probe capped or timed out: container=%s oom=%s wait_status=%s metrics=%s\n' \
    "$loader_probe_id" "$loader_probe_oom" "$loader_probe_wait_status" "$metrics_file" >&2
  exit 2
fi
if (( loader_probe_wait_status != 0 )); then
  printf 'Loader probe wait failed: container=%s wait_status=%s metrics=%s; no fallback or DB startup.\n' \
    "$loader_probe_id" "$loader_probe_wait_status" "$metrics_file" >&2
  exit 2
fi
loader_probe_result="$(cat "$actual_root/logs/loader-probe-exit.txt" 2>/dev/null || printf unavailable)"
docker rm "$loader_probe_id" >/dev/null
loader_probe_started=0
if (( loader_probe_result == 0 )) && ! grep -Eq 'not found|bad ELF interpreter' "$actual_root/logs/api-loader-check.log"; then
  printf 'loader_path=cached-image-compatible\n' >>"$ownership_file"
else
  if ! python3 "$loader_bundle" "$binary" "$loader_root" >"$actual_root/logs/api-loader-dependencies.log" 2>&1; then
    echo "Cached image loader compatibility failed and the exact host loader dependency bundle could not be prepared; no DB has been started. See the bounded dependency report." >&2
    exit 2
  fi
  loader_name="$(awk -F= '$1 == "loader" { print $2; exit }' "$actual_root/logs/api-loader-dependencies.log")"
  if [[ ! "$loader_name" =~ ^[A-Za-z0-9_.+-]+$ || ! -x "$loader_root/$loader_name" || ! -d "$loader_root/lib" ]]; then
    echo "The bounded loader bundle did not resolve to one regular executable loader; no DB has been started." >&2
    exit 2
  fi
  loader_fallback=1
  printf 'loader_path=temporary-exact-dependencies\nloader_root=%s\nloader_name=%s\n' \
    "$loader_root" "$loader_name" >>"$ownership_file"
fi

cat >"$db_override" <<'YAML'
services:
  qa-db:
    mem_limit: 512m
    memswap_limit: 512m
    cpu_shares: 48
    cpus: 1
YAML

compose_created=1
service_status_preflight "$qa_root" actual-runtime-start 2048
LAGRANGE_QA_DB_PORT="$db_port" docker compose \
  --project-name "$project" \
  --file "$repo_root/deploy/qa/qa-db.compose.yml" \
  --file "$db_override" \
  create --pull never
LAGRANGE_QA_DB_PORT="$db_port" docker compose \
  --project-name "$project" \
  --file "$repo_root/deploy/qa/qa-db.compose.yml" \
  --file "$db_override" \
  start
db_id="$(LAGRANGE_QA_DB_PORT="$db_port" docker compose \
  --project-name "$project" \
  --file "$repo_root/deploy/qa/qa-db.compose.yml" \
  --file "$db_override" \
  ps --quiet qa-db)"
db_pid="$(docker inspect --format '{{.State.Pid}}' "$db_id")"
verify_container_caps "$db_id" qa-db 536870912
printf 'qa_db_container_id=%s\nqa_db_pid=%s\n' "$db_id" "$db_pid" >>"$ownership_file"
service_status_monitor_container "$db_id" qa-db "$metrics_file" &
db_monitor_pid=$!
printf 'qa_db_peak_monitor_pid=%s\n' "$db_monitor_pid" >>"$ownership_file"
for ((attempt = 0; attempt < 90; attempt += 1)); do
  db_health="$(docker inspect --format '{{.State.Health.Status}}' "$db_id" 2>/dev/null || true)"
  if [[ "$db_health" == healthy ]]; then break; fi
  sleep 1
done
if [[ "${db_health:-}" != healthy ]]; then
  echo "The owned QA Postgres service did not become healthy." >&2
  exit 1
fi

python3 "$publisher" create "$snapshot_root"

service_status_preflight "$qa_root" actual-add-api 1536
api_created=1
api_run=(docker run --pull=never --detach --init \
  --name "$api_container" \
  --network "${project}_default" \
  --publish "127.0.0.1:${api_port}:8080" \
  --memory 256m --memory-swap 256m --cpu-shares 48 --cpus 1 \
  --read-only --tmpfs /tmp:rw,noexec,nosuid,size=256m,uid=10001,gid=10001 \
  --cap-drop ALL --security-opt no-new-privileges \
  --user 10001:10001 \
  --mount "type=bind,src=${binary},dst=/qa/service-status-browser-harness,readonly" \
  --mount "type=bind,src=${snapshot_root},dst=/run/lagrange/service-status,readonly" \
  --mount "type=bind,src=${handoff_root},dst=/qa-handoff" \
  --env DATABASE_URL=postgres://postgres:lagrange@qa-db:5432/postgres \
  --env SERVICE_STATUS_SESSION_HANDOFF=/qa-handoff/owner-session.json \
  --env SERVICE_STATUS_AUDIT_PROOF=/qa-handoff/audit-proof.json)
if (( loader_fallback )); then
  api_run+=(--mount "type=bind,src=${loader_root},dst=/qa/runtime,readonly")
  api_run+=(--entrypoint "/qa/runtime/$loader_name" "$image" --library-path /qa/runtime/lib /qa/service-status-browser-harness)
else
  api_run+=(--entrypoint /qa/service-status-browser-harness "$image")
fi
api_id="$("${api_run[@]}")"
api_pid="$(docker inspect --format '{{.State.Pid}}' "$api_id")"
verify_container_caps "$api_id" api 268435456
printf 'api_container_id=%s\napi_pid=%s\napi_memory_cap=256m\napi_memory_swap=256m\napi_uid_gid=10001:10001\nsnapshot_mount=/run/lagrange/service-status:ro\napi_owned=1\n' \
  "$api_id" "$api_pid" >>"$ownership_file"
service_status_monitor_container "$api_id" api "$metrics_file" &
api_monitor_pid=$!
printf 'api_peak_monitor_pid=%s\n' "$api_monitor_pid" >>"$ownership_file"

for ((attempt = 0; attempt < 90; attempt += 1)); do
  if docker exec "$api_container" /usr/bin/test -s /qa-handoff/owner-session.json; then break; fi
  api_state="$(docker inspect --format '{{.State.Status}}' "$api_container" 2>/dev/null || true)"
  if [[ "$api_state" != running ]]; then
    echo "The owned API harness exited before writing its private session handoff." >&2
    exit 1
  fi
  sleep 1
done
for ((attempt = 0; attempt < 30; attempt += 1)); do
  if python3 - "$api_port" <<'PY'
import socket
import sys

with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1):
    pass
PY
  then break; fi
  sleep 1
done
if ! python3 - "$api_port" <<'PY'
import socket
import sys

try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1):
        pass
except OSError:
    raise SystemExit(1)
PY
then
  echo "The owned API harness did not bind its reserved loopback port." >&2
  exit 1
fi
python3 "$publisher" copy-session "$snapshot_root"

service_status_preflight "$qa_root" actual-add-next 1280
service_status_require_unused_unit "$next_unit"
web_unit_started=1
systemd-run --user --quiet --unit="$next_unit" \
  --property=Type=exec \
  --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $actual_root/logs/next-receipt.tsv 268435456 $next_unit" \
  --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $actual_root/logs/next-receipt.tsv 268435456 $next_unit" \
  --property=TimeoutStopSec=30s \
  --property=WorkingDirectory="$next_server_root" \
  --property=MemoryMax=256M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
  --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
  --property="StandardOutput=append:$actual_root/logs/next-production.log" \
  --property="StandardError=append:$actual_root/logs/next-production.log" \
  -- env API_INTERNAL_URL="$api_url" NODE_OPTIONS=--max-old-space-size=160 \
    OWNER_INTRADAY_QUOTES_MODE=off OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
    PORT="$web_port" HOSTNAME=127.0.0.1 \
    PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN=http://127.0.0.1:38192 \
    NEXT_TELEMETRY_DISABLED=1 \
    "$(command -v node)" server.js
# The actual router has no anonymous synthetic session. Probe the protected
# HTML and its real static asset with the confined Owner handoff, without logging it.
python3 - "$web_url" "$session_file" "$actual_root/next-readiness.json" <<'PYCODE'
import json, re, sys, time, urllib.error, urllib.request
from pathlib import Path
base, session_path, proof_path = sys.argv[1:]
session = json.loads(Path(session_path).read_text())
request = urllib.request.Request(base+'/', headers={
    'Cookie': session['cookie_name']+'='+session['cookie_value'],
})
last_status = None
for attempt in range(120):
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            last_status = response.status
            if response.url == base+'/' and last_status == 200:
                html = response.read().decode(); break
    except urllib.error.HTTPError as error:
        last_status = error.code
    except OSError:
        last_status = 'network'
    time.sleep(1)
else:
    raise SystemExit(f'Owned authenticated Next readiness failed: last_status={last_status}')
asset = re.search(r'/_next/static/[A-Za-z0-9_./-]+\.js', html)
if asset is None:
    raise SystemExit('Owned Next HTML did not expose a local JavaScript asset')
with urllib.request.urlopen(base+asset.group(), timeout=15) as response:
    if response.status != 200: raise SystemExit('Owned Next static asset unavailable')
Path(proof_path).write_text(json.dumps({
    'html_path':'/','html_status':last_status,
    'asset_path':asset.group(),'asset_status':200,'credential_logged':False,
},indent=2)+'\n')
PYCODE
next_pid="$(systemctl --user show "$next_unit" --property=MainPID --value)"
if [[ ! "$next_pid" =~ ^[1-9][0-9]*$ ]]; then
  echo "The owned Next QA unit did not expose its MainPID." >&2
  exit 1
fi
printf 'next_unit=%s\nnext_pid=%s\nnext_memory_cap=256M\nnext_root=%s\nnext_server_root=%s\n' \
  "$next_unit" "$next_pid" "$next_root" "$next_server_root" >>"$ownership_file"

run_actual_browser() {
  local phase="$1"
  local candidate_browser_unit="lagrange-service-status-wp5-${phase}-browser-$$.service"
  local pid_file="$actual_root/pids/${phase}-browser.pid"
  local cgroup_metrics="$actual_root/logs/${phase}-browser-cgroup.tsv"
  local result runner_pid
  service_status_preflight "$qa_root" "actual-add-browser-$phase" 1024
  service_status_require_unused_unit "$candidate_browser_unit"
  browser_unit="$candidate_browser_unit"
  printf 'browser_unit=%s\nbrowser_pid_file=%s\n' "$browser_unit" "$pid_file" >>"$ownership_file"
  systemd-run --user --quiet --wait --pipe --unit="$browser_unit" \
    --property=Type=exec \
    --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $actual_root/logs/${phase}-browser-receipt.tsv 805306368 $browser_unit" \
    --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $actual_root/logs/${phase}-browser-receipt.tsv 805306368 $browser_unit" \
    --property=TimeoutStopSec=30s \
    --property=WorkingDirectory="$repo_root/apps/web" \
    --property=MemoryMax=768M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
    --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
    -- env API_INTERNAL_URL="$api_url" NODE_OPTIONS=--max-old-space-size=320 NEXT_TELEMETRY_DISABLED=1 \
      OWNER_INTRADAY_QUOTES_MODE=off OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws \
      PLAYWRIGHT_BASE_URL="$web_url" SYNTHETIC_API_ORIGIN=http://127.0.0.1:38192 \
      SERVICE_STATUS_SCREENSHOT_DIR="$artifact_root/screenshots" \
      SERVICE_STATUS_REAL_PHASE="$phase" \
      SERVICE_STATUS_SESSION_FILE="$session_file" \
      SERVICE_STATUS_ACTUAL_PROOF_FILE="$proof_file" \
      PLAYWRIGHT_JSON_OUTPUT_NAME="$artifact_root/${phase}-results.json" \
      bash "$repo_root/scripts/qa/service-status-unit-runner.sh" \
      "$pid_file" "$cgroup_metrics" 805306368 \
      "$repo_root/node_modules/.bin/playwright" test \
      --project=chromium --workers=1 --reporter=line,json \
      --output "$artifact_root/${phase}-playwright-results" \
      --grep '@actual' tests/e2e/service-status.spec.ts \
    >"$actual_root/${phase}-browser.log" 2>&1 &
  runner_pid=$!
  if service_status_wait_unit "actual-browser-$phase" "$browser_unit" "$metrics_file" "$runner_pid"; then result=0; else result=$?; fi
  if (( result != 0 )); then
    printf 'Actual browser stage failed: phase=%s unit=%s exit=%s metrics=%s\n' "$phase" "$browser_unit" "$result" "$metrics_file" >&2
    return "$result"
  fi
  if [[ ! -s "$pid_file" ]]; then
    echo "The bounded actual-browser unit did not record its PID." >&2
    return 1
  fi
  local browser_pid
  browser_pid="$(cat "$pid_file")"
  if [[ ! "$browser_pid" =~ ^[1-9][0-9]*$ ]]; then
    echo "The bounded actual-browser unit recorded an invalid PID." >&2
    return 1
  fi
  printf 'browser_pid=%s\n' "$browser_pid" >>"$ownership_file"
}

run_actual_browser initial >"$actual_root/initial-browser.log" 2>&1
python3 "$publisher" replace-policy "$snapshot_root"
# The initial Owner read happened before the initial browser process exited.
# Keep a full six-second margin before the same Owner/session is read again.
sleep 6
run_actual_browser updated >"$actual_root/updated-browser.log" 2>&1

if [[ -n "$api_monitor_pid" ]]; then kill "$api_monitor_pid" >/dev/null 2>&1 || true; wait "$api_monitor_pid" 2>/dev/null || true; api_monitor_pid=""; fi
service_status_container_snapshot "$api_id" api "$metrics_file"
docker stop --time 30 "$api_container" >/dev/null
service_status_container_snapshot "$api_id" api-final-before-remove "$metrics_file"
docker rm "$api_container" >/dev/null
api_created=0
python3 "$publisher" copy-audit "$snapshot_root"
jq -e '.owner_status_reads >= 2' "$actual_root/audit-proof.json" >/dev/null
