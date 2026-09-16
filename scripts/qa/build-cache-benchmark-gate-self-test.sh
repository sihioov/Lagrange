#!/usr/bin/env bash
#
# Focused no-daemon contract test for the benchmark's shared release-layout
# gate. The production helper is sourced unchanged; only its subprocess
# observations are replaced with private executable fixtures.
set -euo pipefail

self_script_dir=$(cd "$(dirname "$0")" && pwd -P)
self_repo_root=$(cd "$self_script_dir/../.." && pwd -P)
benchmark_harness=$self_script_dir/build-cache-benchmark.sh
product_helper=$self_repo_root/scripts/ops/lib/release-build-layout.sh

fail() {
  printf 'build-cache-benchmark-gate-self-test: %s\n' "$*" >&2
  exit 1
}

[ -f "$benchmark_harness" ] && [ ! -L "$benchmark_harness" ] || fail 'benchmark harness is missing'
[ -f "$product_helper" ] && [ ! -L "$product_helper" ] || fail 'release layout helper is missing'
# The harness has a source-only boundary for this focused dispatch test.
# shellcheck disable=SC1090
source "$benchmark_harness"

requested_tmp=${TMPDIR:-/tmp}
case "$requested_tmp" in
  /*) ;;
  *) fail 'TMPDIR must be absolute' ;;
esac
[ -d "$requested_tmp" ] && [ ! -L "$requested_tmp" ] || fail 'TMPDIR is not a directory'
test_base=$(cd "$requested_tmp" && pwd -P)
[ "$test_base" != / ] || fail 'refusing root as test base'
test_dir=$(mktemp -d "$test_base/wp16-benchmark-gate-self-test.XXXXXXXXXX")
chmod 0700 "$test_dir"
cleanup() {
  rm -rf -- "$test_dir"
}
trap cleanup EXIT

fixture=$test_dir/coordinator-fixture
source_commit=$(git -C "$self_repo_root" rev-parse --verify 'HEAD^{commit}')
git clone --quiet --no-local "$self_repo_root" "$fixture"
git -C "$fixture" checkout --quiet --detach "$source_commit"
[ "$(git -C "$fixture" rev-parse HEAD)" = "$source_commit" ] ||
  fail 'disposable fixture did not retain coordinator HEAD'
[ -z "$(git -C "$fixture" status --porcelain=v1 --untracked-files=all)" ] ||
  fail 'disposable fixture is not clean'

fake_bin=$test_dir/fake-bin
mkdir -m 0700 -- "$fake_bin"
control_cgroup=$(awk -F: '$1 == "0" { print $3; exit }' /proc/self/cgroup)
[ -n "$control_cgroup" ] && [ "$control_cgroup" != / ] || fail 'test cgroup is not usable'
boot_id=$(tr -d '-' </proc/sys/kernel/random/boot_id)
[[ "$boot_id" =~ ^[0-9a-f]{32}$ ]] || fail 'current boot ID is malformed'

export WP16_BOOT_ID=$boot_id
export WP16_CGROUP=$control_cgroup
export WP16_RESEARCH_ID=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export WP16_RESEARCH_IMAGE=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
export WP16_API_ID=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
export WP16_API_IMAGE=sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd

cat >"$fake_bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

manager=system
if [ "$#" -gt 0 ] && [ "$1" = --user ]; then
  manager=user
  shift
fi
[ "$#" -gt 0 ] && [ "$1" = show ] || exit 91
shift
unit=
properties=
value_mode=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --property=*)
      [ -z "$properties" ] || exit 92
      properties=${1#--property=}
      shift
      ;;
    -p)
      [ "$#" -ge 2 ] && [ -z "$properties" ] || exit 92
      properties=$2
      shift 2
      ;;
    --value)
      value_mode=1
      shift
      ;;
    --)
      shift
      [ "$#" -eq 1 ] && [ -z "$unit" ] || exit 92
      unit=$1
      shift
      ;;
    *)
      [ -z "$unit" ] || exit 92
      unit=$1
      shift
      ;;
  esac
done
[ -n "$unit" ] && [ -n "$properties" ] || exit 93
printf '%s\t%s\t%s\n' "$manager" "$unit" "$properties" >>"$WP16_SYSTEMCTL_LOG"
IFS=, read -r -a property_list <<<"$properties"
for property in "${property_list[@]}"; do
  case "$property" in
    LoadState) value=loaded ;;
    ActiveState) value=active ;;
    SubState) value=running ;;
    ExecMainStatus) value=0 ;;
    MainPID) value=4242 ;;
    ControlGroup) value=$WP16_CGROUP ;;
    Nice) value=10 ;;
    IOSchedulingClass) value=idle ;;
    IOSchedulingPriority) value=0 ;;
    NRestarts) value=0 ;;
    *) exit 94 ;;
  esac
  if [ "$value_mode" -eq 1 ]; then
    printf '%s\n' "$value"
  else
    printf '%s=%s\n' "$property" "$value"
  fi
done
EOF

cat >"$fake_bin/ps" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "$#" -eq 2 ] && [ "$1" = -eo ] && [ "$2" = comm= ] || exit 91
ps_output=bash
[ -n "${WP16_PS_OUTPUT-}" ] && ps_output=$WP16_PS_OUTPUT
printf '%s\n' "$ps_output"
EOF

cat >"$fake_bin/journalctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

mode=clean
[ -n "${WP16_JOURNAL_MODE-}" ] && mode=$WP16_JOURNAL_MODE
if [ "$#" -eq 7 ] && [ "$1" = -k ] && [ "$2" = -b ] &&
   [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] &&
   [ "$6" = -n ] && [ "$7" = 1 ]; then
  printf '{"__REALTIME_TIMESTAMP":"1893456000000000","__CURSOR":"wp16-probe","_BOOT_ID":"%s","_TRANSPORT":"kernel","MESSAGE":"fixture probe"}\n' "$WP16_BOOT_ID"
  exit 0
fi
if [ "$#" -eq 10 ] && [ "$1" = -k ] && [ "$2" = -b ] &&
   [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] &&
   [ "$6" = --since ] && [ "$8" = --until ]; then
  shift 9
  [ "$1" = --no-tail ] || exit 96
  case "$mode" in
    clean) exit 0 ;;
    oom)
      seconds=$(date -u +%s)
      timestamp=$((seconds * 1000000 - 1000000))
      printf '{"__REALTIME_TIMESTAMP":"%s","__CURSOR":"wp16-oom","_BOOT_ID":"%s","_TRANSPORT":"kernel","MESSAGE":"Out of memory: fixture"}\n' \
        "$timestamp" "$WP16_BOOT_ID"
      exit 0
      ;;
    *) exit 95 ;;
  esac
fi
exit 96
EOF

cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

format=$'{{.Id}}{{printf "\t"}}{{.State.Running}}{{printf "\t"}}{{.State.Restarting}}{{printf "\t"}}{{.State.OOMKilled}}{{printf "\t"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}{{printf "\t"}}{{.RestartCount}}{{printf "\t"}}{{if index .Config.Labels "com.docker.compose.project"}}{{index .Config.Labels "com.docker.compose.project"}}{{else}}absent{{end}}{{printf "\t"}}{{.Image}}{{printf "\t"}}{{.State.ExitCode}}'
[ "$#" -gt 0 ] && [ "$1" = inspect ] || exit 91
shift
[ "$#" -ge 2 ] && [ "$1" = --type ] && [ "$2" = container ] || exit 92
shift 2
[ "$#" -ge 2 ] && [ "$1" = --format ] && [ "$2" = "$format" ] || exit 93
shift 2
[ "$#" -eq 1 ] || exit 94
name=$1
mode=healthy
[ -n "${WP16_CONTAINER_MODE-}" ] && mode=$WP16_CONTAINER_MODE
if [ "$name" = lagrange-station-research-worker-1 ]; then
  ident=$WP16_RESEARCH_ID
  image=$WP16_RESEARCH_IMAGE
  running=true
  restarting=false
  oom=false
  health=healthy
  restarts=0
  exit_code=0
  case "$mode" in
    research-exception|exception) health=unhealthy; exit_code=2 ;;
    restart-growth) health=unhealthy; restarts=1; exit_code=2 ;;
    new-exit) health=unhealthy; exit_code=1 ;;
    wrong-id) ident=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee ;;
    wrong-image) image=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff ;;
  esac
else
  ident=$WP16_API_ID
  image=$WP16_API_IMAGE
  running=true
  restarting=false
  oom=false
  health=healthy
  restarts=0
  exit_code=0
  [ "$mode" = nonresearch-unhealthy ] && health=unhealthy
fi
printf '%s\t%s\t%s\t%s\t%s\t%s\tlagrange-station\t%s\t%s\n' \
  "$ident" "$running" "$restarting" "$oom" "$health" "$restarts" "$image" "$exit_code"
EOF

chmod 0700 "$fake_bin/systemctl" "$fake_bin/ps" "$fake_bin/journalctl" "$fake_bin/docker"
export PATH="$fake_bin:$PATH"

write_exception() {
  local path=$1 kind=valid
  [ "$#" -ge 2 ] && kind=$2
  WP16_EXCEPTION_PATH=$path WP16_EXCEPTION_KIND=$kind \
    python3 - <<'PY'
import datetime
import json
import os
import stat
import time

path=os.environ["WP16_EXCEPTION_PATH"]
kind=os.environ["WP16_EXCEPTION_KIND"]
now=int(time.time())
if kind=="expired":
    observed=now-7200
    expires=now-3600
else:
    observed=now-5
    expires=now+7200
value={
    "container_id":os.environ["WP16_RESEARCH_ID"],
    "container_name":"lagrange-station-research-worker-1",
    "expires_at_utc":datetime.datetime.fromtimestamp(expires,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "format":"lagrange-build-research-exception-v1",
    "image_id":os.environ["WP16_RESEARCH_IMAGE"],
    "initial_restart_count":0,
    "known_error_code":"PRICE_CURATION_FAILED",
    "known_exit_code":2,
    "observed_at_utc":datetime.datetime.fromtimestamp(observed,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "scope":"image-build-only",
}
raw=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode()
fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,"wb") as handle:
    handle.write(raw)
    handle.flush()
    os.fsync(handle.fileno())
assert stat.S_IMODE(os.stat(path).st_mode)==0o600
PY
}

assert_record() {
  local state=$1 expected_status=$2 expected_reason=$3
  [ -f "$state/gates/gates.jsonl" ] && [ ! -L "$state/gates/gates.jsonl" ] ||
    fail "gate record missing for $state"
  WP16_RECORD=$state/gates/gates.jsonl WP16_EXPECTED_STATUS=$expected_status \
    WP16_EXPECTED_REASON=$expected_reason python3 - <<'PY'
import json
import os

records=[json.loads(line) for line in open(os.environ["WP16_RECORD"],encoding="utf-8") if line.strip()]
if not records:
    raise SystemExit("empty-gate-record")
record=records[-1]
if record.get("format")!="lagrange-build-gate-record-v2":
    raise SystemExit("wrong-record-format")
if record.get("status")!=os.environ["WP16_EXPECTED_STATUS"]:
    raise SystemExit("wrong-status:"+str(record.get("status")))
if record.get("reason")!=os.environ["WP16_EXPECTED_REASON"]:
    raise SystemExit("wrong-reason:"+str(record.get("reason")))
evidence=record.get("evidence")
if not isinstance(evidence,dict):
    raise SystemExit("missing-evidence")
reason=os.environ["WP16_EXPECTED_REASON"]
if reason in ("previous-step-failed","research-exception-invalid"):
    raise SystemExit(0)
if not isinstance(evidence.get("build_unit"),dict):
    raise SystemExit("missing-build-evidence")
if not isinstance(evidence.get("units"),dict) or set(evidence["units"]) != {"api.service","web.service"}:
    raise SystemExit("missing-system-health-evidence")
if reason in ("healthy","image-build-only-known-incident","container-health-invalid"):
    if not isinstance(evidence.get("containers"),dict) or "api-container" not in evidence["containers"]:
        raise SystemExit("missing-container-evidence")
if reason=="image-build-only-known-incident":
    binding=evidence.get("research_exception")
    if not isinstance(binding,dict) or binding.get("fields",{}).get("known_error_code")!="PRICE_CURATION_FAILED":
        raise SystemExit("missing-exception-binding")
PY
}

run_gate_case() {
  local name=$1 mode=$2 exception_kind=$3 expected_status=$4 expected_reason=$5
  local manager=system previous=0 journal_mode=clean
  [ "$#" -ge 6 ] && manager=$6
  [ "$#" -ge 7 ] && previous=$7
  [ "$#" -ge 8 ] && journal_mode=$8
  local case_dir=$test_dir/cases/$name
  local state=$case_dir/state exception=$case_dir/exception
  mkdir -m 0700 -p -- "$case_dir"
  if [ "$exception_kind" = none ]; then
    unset WP16_EXCEPTION_PATH
    exception=
  else
    write_exception "$exception" "$exception_kind"
    export WP16_EXCEPTION_PATH=$exception
  fi
  export WP16_CONTAINER_MODE=$mode WP16_JOURNAL_MODE=$journal_mode
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service
    export RELEASE_BUILD_SYSTEMD_MANAGER=$manager
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    if [ "$exception_kind" = none ]; then
      unset RELEASE_BUILD_RESEARCH_EXCEPTION
    else
      export RELEASE_BUILD_RESEARCH_EXCEPTION=$exception
    fi
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" "wp16-$name" >"$case_dir/init.out" 2>"$case_dir/init.err"
    release_build_layout_gate "wp16-$name" "$previous" >"$case_dir/gate.out" 2>"$case_dir/gate.err"
  ); then
    actual_status=PASS
  else
    actual_status=FAIL
  fi
  assert_record "$state" "$expected_status" "$expected_reason" ||
    fail "$name did not produce the expected real-helper record (observed $actual_status)"
}

run_restart_growth_case() {
  local name=restart-growth
  local case_dir=$test_dir/cases/$name
  local state=$case_dir/state exception=$case_dir/exception
  mkdir -m 0700 -p -- "$case_dir"
  write_exception "$exception"
  export WP16_CONTAINER_MODE=research-exception WP16_JOURNAL_MODE=clean
  export WP16_EXCEPTION_PATH=$exception WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    export RELEASE_BUILD_RESEARCH_EXCEPTION=$exception
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" wp16-restart-growth >/dev/null 2>"$case_dir/init.err"
    release_build_layout_gate restart-growth-1 0 >/dev/null 2>"$case_dir/first.err"
    export WP16_CONTAINER_MODE=restart-growth
    release_build_layout_gate restart-growth-2 0 >/dev/null 2>"$case_dir/second.err"
  ) || fail 'restart-growth sequence failed unexpectedly'
  WP16_RECORD=$state/gates/gates.jsonl python3 - <<'PY'
import json
import os

records=[json.loads(line) for line in open(os.environ["WP16_RECORD"],encoding="utf-8") if line.strip()]
if len(records)!=2 or any(item["status"]!="PASS" for item in records):
    raise SystemExit("restart-growth-not-passing")
if records[0]["evidence"]["containers"]["lagrange-station-research-worker-1"]["selected"]["restart_count"] != 0:
    raise SystemExit("restart-growth-first-count")
if records[1]["evidence"]["containers"]["lagrange-station-research-worker-1"]["selected"]["restart_count"] != 1:
    raise SystemExit("restart-growth-second-count")
PY
}

run_monotonicity_case() {
  local name=monotonicity
  local case_dir=$test_dir/cases/$name
  local state=$case_dir/state exception=$case_dir/exception
  mkdir -m 0700 -p -- "$case_dir"
  write_exception "$exception"
  export WP16_CONTAINER_MODE=research-exception WP16_JOURNAL_MODE=clean
  export WP16_EXCEPTION_PATH=$exception WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    export RELEASE_BUILD_RESEARCH_EXCEPTION=$exception
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" wp16-monotonicity >/dev/null 2>"$case_dir/init.err"
    release_build_layout_gate monotonicity-1 0 >/dev/null 2>"$case_dir/first.err"
  ) || fail 'monotonicity first gate failed unexpectedly'
  WP16_STATE=$state/gates/gate-state.json python3 - <<'PY'
import datetime
import json
import os
import time

path=os.environ["WP16_STATE"]
value=json.load(open(path,encoding="utf-8"))
future=(int(time.time())+60)*1000000000
sample=value["research_exception"]["latest_observation"]
sample["monitored_at_unix_ns"]=future
sample["monitored_at_utc"]=datetime.datetime.fromtimestamp(
    future//1000000000,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
observed=int(datetime.datetime.strptime(
    value["research_exception"]["binding"]["fields"]["observed_at_utc"],
    "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp())*1000000000
interval=30*1000000000
sample["restart_limit"]=((future-observed)+interval-1)//interval+2
raw=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode()
with open(path,"wb") as handle:
    handle.write(raw)
    handle.flush()
    os.fsync(handle.fileno())
PY
  if (
    export WP16_CONTAINER_MODE=research-exception
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    export RELEASE_BUILD_RESEARCH_EXCEPTION=$exception
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" wp16-monotonicity
    release_build_layout_gate monotonicity-2 0
  ) 2>"$case_dir/second.err"; then
    fail 'monotonicity decrease was accepted'
  fi
  assert_record "$state" FAIL research-restart-or-time-decreased
}

run_dispatch_case() {
  local dispatch=$test_dir/dispatch output=$test_dir/dispatch-output caller_tmp=$test_dir/caller-tmp
  mkdir -m 0700 -- "$dispatch" "$output" "$caller_tmp"
  (
    # These globals mirror --apply while the fixture supplies a clean
    # coordinator HEAD for the real public initializer.
    repo_root=$fixture
    layout_helper=$fixture/scripts/ops/lib/release-build-layout.sh
    output_dir=$output
    resource_report=$output/batch-resources.tsv
    benchmark_gate_initialized=0
    benchmark_gate_source_root=
    benchmark_gate_source_commit=
    benchmark_gate_state_root=
    benchmark_gate_namespace=
    benchmark_gate_helper=
    benchmark_gate_lock_prefix=$dispatch/whole-lock
    benchmark_tmp_base=
    tmp_dir=
    internal_self_test=0
    last_build_exit=not-started
    min_mem_available_kib=0
    min_swap_free_kib=0
    failure_stage=preflight
    BENCHMARK_SYSTEMD_SERVICE=wp16-build.service
    BENCHMARK_SYSTEMD_MANAGER=user
    BENCHMARK_PRODUCTION_HEALTH_UNITS=api.service,web.service
    BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    BENCHMARK_RESEARCH_EXCEPTION=
    export BENCHMARK_SYSTEMD_SERVICE BENCHMARK_SYSTEMD_MANAGER
    export BENCHMARK_PRODUCTION_HEALTH_UNITS BENCHMARK_PRODUCTION_HEALTH_CONTAINERS
    export BENCHMARK_RESEARCH_EXCEPTION
    export RBL_LOCK_PREFIX=$benchmark_gate_lock_prefix
    export WP16_CONTAINER_MODE=healthy WP16_JOURNAL_MODE=clean
    export WP16_SYSTEMCTL_LOG=$dispatch/systemctl.tsv
    : >"$WP16_SYSTEMCTL_LOG"
    common_gate_environment
    initialize_benchmark_gate_guard
    gate_batch dispatch integration 1 before
    TMPDIR=$caller_tmp
    create_temp_run_directory
    case "$tmp_dir" in
      "$caller_tmp"/lagrange-build-cache-benchmark.??????????) ;;
      *) exit 71 ;;
    esac
    cleanup_temp_checkouts
  ) || fail 'actual harness dispatch integration failed'
  WP16_STATE=$output/benchmark-gate-state/gates/gates.jsonl \
    WP16_REPORT=$output/batch-resources.tsv python3 - <<'PY'
import json
import os

records=[json.loads(line) for line in open(os.environ["WP16_STATE"],encoding="utf-8") if line.strip()]
if len(records)!=1 or records[0]["label"]!="benchmark:dispatch:integration:1:before":
    raise SystemExit("dispatch-label")
if records[0]["status"]!="PASS" or records[0]["reason"]!="healthy":
    raise SystemExit("dispatch-gate-status")
fields=open(os.environ["WP16_REPORT"],encoding="utf-8").read().splitlines()[-1].split("\t")
if len(fields)!=29 or fields[26]!="verified" or fields[27]!="verified" or fields[28]!="pass:-":
    raise SystemExit("dispatch-report-fields")
PY
  grep -Fq $'user\twp16-build.service' "$dispatch/systemctl.tsv" ||
    fail 'dispatch did not query the build unit in user scope'
  grep -Fq $'system\tapi.service' "$dispatch/systemctl.tsv" ||
    fail 'dispatch did not query health units in system scope'
  grep -Fq $'system\tweb.service' "$dispatch/systemctl.tsv" ||
    fail 'dispatch did not query all health units in system scope'
  if grep -Fq $'user\tapi.service' "$dispatch/systemctl.tsv" ||
     grep -Fq $'user\tweb.service' "$dispatch/systemctl.tsv"; then
    fail 'dispatch queried a production health unit in user scope'
  fi
}

# Every case below reaches the unchanged public initializer and gate, which
# writes a canonical persistent record. Structured fields, not a status
# string, determine every expected outcome.
run_gate_case valid-exception research-exception valid PASS image-build-only-known-incident
run_gate_case absent-exception research-exception none FAIL container-health-invalid
run_gate_case wrong-container-id wrong-id valid FAIL container-health-invalid
run_gate_case wrong-image wrong-image valid FAIL container-health-invalid
run_gate_case expired-exception research-exception expired FAIL research-exception-invalid
run_gate_case new-container-exit new-exit valid FAIL container-health-invalid
run_gate_case kernel-oom healthy none FAIL kernel-oom-observed system 0 oom
run_gate_case nonresearch-unhealthy nonresearch-unhealthy none FAIL container-health-invalid
run_gate_case healthy-system healthy none PASS healthy system
run_gate_case previous-exit healthy none FAIL previous-step-failed system 17
run_restart_growth_case
run_monotonicity_case
run_dispatch_case

printf 'BUILD_CACHE_BENCHMARK_GATE_SELF_TEST: PASS (real public init/gate with structured subprocess fixtures; no Docker, Rust, or systemd mutation)\n'
