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

# The self-test runs before the coordinator commits the new policy module. Use
# the exact working-tree helper/module bytes in this disposable fixture while
# keeping its pinned HEAD and clean-worktree contract intact. The helper is
# hidden from status only inside the disposable clone; the module is ignored
# by that clone's private Git exclude file. No repository commit is made.
cp -p -- "$product_helper" "$fixture/scripts/ops/lib/release-build-layout.sh"
cp -p -- "$self_repo_root/scripts/ops/lib/build-resource-policy.py" \
  "$fixture/scripts/ops/lib/build-resource-policy.py"
git -C "$fixture" update-index --assume-unchanged scripts/ops/lib/release-build-layout.sh
printf '%s\n' 'scripts/ops/lib/build-resource-policy.py' >>"$fixture/.git/info/exclude"
[ -z "$(git -C "$fixture" status --porcelain=v1 --untracked-files=all)" ] ||
  fail 'working-tree policy fixture is not clean after private setup'

# Keep a separate, untouched checkout of the product C10 helper. This is the
# regression target for the common-C binding: copying the current helper here
# would hide the old swap-only stop that blocked the real C10 route.
c10_commit=15c13c524562fa98efb2efa2525e9b3184836293
git cat-file -e "$c10_commit^{commit}" || fail 'frozen C10 commit is unavailable locally'
c10_fixture=$test_dir/frozen-c10-fixture
git clone --quiet --no-local "$self_repo_root" "$c10_fixture" ||
  fail 'could not create the frozen C10 fixture'
git -C "$c10_fixture" checkout --quiet --detach "$c10_commit" ||
  fail 'could not detach the frozen C10 fixture'
[ "$(git -C "$c10_fixture" rev-parse HEAD)" = "$c10_commit" ] ||
  fail 'frozen C10 fixture did not retain its pinned commit'
[ -z "$(git -C "$c10_fixture" status --porcelain=v1 --untracked-files=all)" ] ||
  fail 'frozen C10 fixture is not clean'

fake_bin=$test_dir/fake-bin
mkdir -m 0700 -- "$fake_bin"

fake_python=$test_dir/fake-python
mkdir -m 0700 -- "$fake_python"
cat >"$fake_python/sitecustomize.py" <<'PY'
import builtins
import io
import os

_real_open = builtins.open


def open_resource_fixture(file, mode="r", *args, **kwargs):
    path = os.fspath(file)
    if path == "/proc/meminfo" and "r" in mode:
        value = os.environ.get("WP16_MEMINFO_FIXTURE")
        if value is not None:
            if "b" in mode:
                return io.BytesIO(value.encode("ascii"))
            return io.StringIO(value)
    if path == "/proc/pressure/memory" and "r" in mode:
        value = os.environ.get("WP16_PRESSURE_FIXTURE")
        if value is not None:
            if "b" in mode:
                return io.BytesIO(value.encode("ascii"))
            return io.StringIO(value)
    return _real_open(file, mode, *args, **kwargs)


builtins.open = open_resource_fixture
PY
control_cgroup=$(awk -F: '$1 == "0" { print $3; exit }' /proc/self/cgroup)
[ -n "$control_cgroup" ] && [ "$control_cgroup" != / ] || fail 'test cgroup is not usable'
boot_id=$(tr -d '-' </proc/sys/kernel/random/boot_id)
[[ "$boot_id" =~ ^[0-9a-f]{32}$ ]] || fail 'current boot ID is malformed'

export WP16_BOOT_ID=$boot_id
export WP16_CGROUP=$control_cgroup
export WP16_RESEARCH_ID=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export WP16_RESEARCH_IMAGE=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
export WP16_OWNER_ID=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
export WP16_OWNER_IMAGE=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
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

format=$'{{.Id}}{{printf "\t"}}{{.State.Running}}{{printf "\t"}}{{.State.Restarting}}{{printf "\t"}}{{.State.OOMKilled}}{{printf "\t"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}{{printf "\t"}}{{.RestartCount}}{{printf "\t"}}{{if index .Config.Labels "com.docker.compose.project"}}{{index .Config.Labels "com.docker.compose.project"}}{{else}}absent{{end}}{{printf "\t"}}{{.Image}}{{printf "\t"}}{{.State.ExitCode}}{{printf "\t"}}{{.State.Status}}{{printf "\t"}}{{.State.Paused}}{{printf "\t"}}{{.State.Dead}}{{printf "\t"}}{{.State.StartedAt}}{{printf "\t"}}{{.State.FinishedAt}}'
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
status=running
paused=false
dead=false
project=lagrange-station
started=2026-09-18T00:00:00.000000000Z
finished=0001-01-01T00:00:00.000000000Z
if [ "${WP16_DRAINED_READERS:-0}" = 1 ] &&
   { [ "$name" = lagrange-station-research-worker-1 ] ||
     [ "$name" = lagrange-station-owner-equity-v2-runner-1 ]; }; then
  if [ "$name" = lagrange-station-research-worker-1 ]; then
    ident=$WP16_RESEARCH_ID
    image=$WP16_RESEARCH_IMAGE
    health=unhealthy
    restarts=7
    exit_code=2
  else
    ident=$WP16_OWNER_ID
    image=$WP16_OWNER_IMAGE
    health=unhealthy
    restarts=0
    exit_code=0
  fi
  running=false
  status=exited
  finished=2026-09-18T12:00:00.000000000Z
  case "${WP16_DRAINED_MODE:-valid}" in
    id) [ "$name" = lagrange-station-research-worker-1 ] && ident=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee ;;
    owner-id) [ "$name" = lagrange-station-owner-equity-v2-runner-1 ] && ident=1212121212121212121212121212121212121212121212121212121212121212 ;;
    image) [ "$name" = lagrange-station-research-worker-1 ] && image=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff ;;
    owner-image) [ "$name" = lagrange-station-owner-equity-v2-runner-1 ] && image=sha256:1212121212121212121212121212121212121212121212121212121212121212 ;;
    health) [ "$name" = lagrange-station-research-worker-1 ] && health=healthy ;;
    owner-health) [ "$name" = lagrange-station-owner-equity-v2-runner-1 ] && health=healthy ;;
    exit) [ "$name" = lagrange-station-research-worker-1 ] && exit_code=1 ;;
    owner-exit) [ "$name" = lagrange-station-owner-equity-v2-runner-1 ] && exit_code=1 ;;
    restart) [ "$name" = lagrange-station-research-worker-1 ] && restarts=8 ;;
    owner-restart) [ "$name" = lagrange-station-owner-equity-v2-runner-1 ] && restarts=1 ;;
    started) [ "$name" = lagrange-station-research-worker-1 ] && started=2026-09-18T00:01:00.000000000Z ;;
    finished) [ "$name" = lagrange-station-research-worker-1 ] && finished=2026-09-18T12:01:00.000000000Z ;;
    lifecycle-drift) [ "$name" = lagrange-station-research-worker-1 ] && started=2026-09-18T00:01:00.000000000Z && finished=2026-09-18T12:01:00.000000000Z ;;
    running) [ "$name" = lagrange-station-research-worker-1 ] && running=true ;;
    restarting) [ "$name" = lagrange-station-research-worker-1 ] && restarting=true ;;
    paused) [ "$name" = lagrange-station-research-worker-1 ] && paused=true ;;
    dead) [ "$name" = lagrange-station-research-worker-1 ] && dead=true ;;
    oom) [ "$name" = lagrange-station-research-worker-1 ] && oom=true ;;
    status) [ "$name" = lagrange-station-research-worker-1 ] && status=dead ;;
    project) [ "$name" = lagrange-station-research-worker-1 ] && project=wrong-project ;;
  esac
fi
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "$ident" "$running" "$restarting" "$oom" "$health" "$restarts" "$project" "$image" \
  "$exit_code" "$status" "$paused" "$dead" "$started" "$finished"
EOF

chmod 0700 "$fake_bin/systemctl" "$fake_bin/ps" "$fake_bin/journalctl" "$fake_bin/docker"
export PATH="$fake_bin:$PATH"
export PYTHONPATH="$fake_python"
export WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
export WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'

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

write_drained_attestation() {
  local path=$1 kind=valid
  [ "$#" -ge 2 ] && kind=$2
  WP16_DRAINED_ATTESTATION_PATH=$path WP16_DRAINED_ATTESTATION_KIND=$kind \
  WP16_DRAINED_SOURCE_COMMIT=$source_commit python3 - <<'PY'
import datetime
import json
import os
import stat
import time

path=os.environ["WP16_DRAINED_ATTESTATION_PATH"]
kind=os.environ["WP16_DRAINED_ATTESTATION_KIND"]
now=int(time.time())
observed=now-5
expires=now+7200
if kind=="expired":
    observed=now-7200
    expires=now-3600
value={
    "containers":[
        {
            "container_id":os.environ["WP16_OWNER_ID"],
            "container_name":"lagrange-station-owner-equity-v2-runner-1",
            "dead":False,
            "exit_code":0,
            "finished_at_utc":"2026-09-18T12:00:00.000000000Z",
            "health_status":"unhealthy",
            "image_id":os.environ["WP16_OWNER_IMAGE"],
            "oom_killed":False,
            "paused":False,
            "restarting":False,
            "restart_count":0,
            "running":False,
            "started_at_utc":"2026-09-18T00:00:00.000000000Z",
            "status":"exited",
        },
        {
            "container_id":os.environ["WP16_RESEARCH_ID"],
            "container_name":"lagrange-station-research-worker-1",
            "dead":False,
            "exit_code":2,
            "finished_at_utc":"2026-09-18T12:00:00.000000000Z",
            "health_status":"unhealthy",
            "image_id":os.environ["WP16_RESEARCH_IMAGE"],
            "oom_killed":False,
            "paused":False,
            "restarting":False,
            "restart_count":7,
            "running":False,
            "started_at_utc":"2026-09-18T00:00:00.000000000Z",
            "status":"exited",
        },
    ],
    "expires_at_utc":datetime.datetime.fromtimestamp(expires,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "format":"lagrange-build-drained-readers-attestation-v1",
    "observed_at_utc":datetime.datetime.fromtimestamp(observed,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "project":"lagrange-station",
    "scope":"image-build-only",
    "target_commit":os.environ["WP16_DRAINED_SOURCE_COMMIT"],
}
if kind=="wrong-commit":
    value["target_commit"]="f"*40
elif kind=="missing-research":
    value["containers"]=value["containers"][:1]
elif kind=="missing-owner":
    value["containers"]=value["containers"][1:]
elif kind=="extra":
    extra=dict(value["containers"][0])
    extra["container_name"]="lagrange-station-unrecognized-1"
    value["containers"].append(extra)
elif kind=="duplicate":
    value["containers"][1]["container_name"]="lagrange-station-owner-equity-v2-runner-1"
elif kind=="replace":
    value["containers"][1]["restart_count"]=8
elif kind=="future-observed":
    value["observed_at_utc"]=datetime.datetime.fromtimestamp(now+60,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
elif kind=="long-expiry":
    value["expires_at_utc"]=datetime.datetime.fromtimestamp(observed+13*3600,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
elif kind=="bad-time":
    value["containers"][0]["started_at_utc"]="not-a-docker-timestamp"
elif kind=="unknown-field":
    value["containers"][0]["unexpected"]="reject-me"
elif kind=="bad-json":
    raw=b"{\"format\":\"broken\"}\n"
else:
    raw=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode()
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,"wb") as handle:
        handle.write(raw); handle.flush(); os.fsync(handle.fileno())
    assert stat.S_IMODE(os.stat(path).st_mode)==0o600
    raise SystemExit(0)
fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,"wb") as handle:
    handle.write(raw if kind=="bad-json" else (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode())
    handle.flush(); os.fsync(handle.fileno())
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
if reason in ("previous-step-failed","research-exception-invalid",
              "mem-available-below-floor","memory-pressure-high",
              "resource-observation-invalid","compiler-process-active",
              "drained-readers-attestation-invalid","drained-readers-attestation-added",
              "drained-readers-attestation-removed","drained-readers-attestation-binding-changed",
              "drained-readers-container-inventory-invalid",
              "drained-readers-attestation-mutually-exclusive"):
    raise SystemExit(0)
if not isinstance(evidence.get("build_unit"),dict):
    raise SystemExit("missing-build-evidence")
if not isinstance(evidence.get("units"),dict) or set(evidence["units"]) != {"api.service","web.service"}:
    raise SystemExit("missing-system-health-evidence")
if reason in ("healthy","image-build-only-known-incident","container-health-invalid"):
    if not isinstance(evidence.get("containers"),dict) or not evidence["containers"]:
        raise SystemExit("missing-container-evidence")
if reason=="image-build-only-known-incident":
    binding=evidence.get("research_exception")
    if not isinstance(binding,dict) or binding.get("fields",{}).get("known_error_code")!="PRICE_CURATION_FAILED":
        raise SystemExit("missing-exception-binding")
if reason=="image-build-only-drained-readers":
    binding=evidence.get("drained_readers_attestation")
    if not isinstance(binding,dict) or binding.get("fields",{}).get("scope")!="image-build-only":
        raise SystemExit("missing-drained-attestation-binding")
    expected={
        "lagrange-station-postgres-1","lagrange-station-reverse-proxy-1",
        "lagrange-station-api-server-1","lagrange-station-web-1",
        "lagrange-station-research-worker-1","lagrange-station-recommendation-runner-1",
        "lagrange-station-candidate-runner-1","lagrange-station-owner-beta-runner-1",
        "lagrange-station-owner-equity-v2-runner-1","lagrange-station-nt-backtest-worker-1-1",
        "lagrange-station-nt-backtest-worker-2-1"
    }
    if set(evidence["containers"]) != expected:
        raise SystemExit("incomplete-drained-inventory-evidence")
    for name in ("lagrange-station-research-worker-1","lagrange-station-owner-equity-v2-runner-1"):
        selected=evidence["containers"][name]["selected"]
        if (selected["status"]!="exited" or selected["running"]!="false" or
                selected["restart_count"] < 0 or selected["paused"]!="false" or
                selected["dead"]!="false" or selected["oom_killed"]!="false"):
            raise SystemExit("drained-reader-evidence-not-stopped")
PY
}

assert_resource_evidence() {
  local state=$1 expected_status=$2 expected_reason=$3 expected_mem=$4 expected_swap=$5 expected_full=$6
  WP16_RECORD=$state/gates/gates.jsonl WP16_EXPECTED_STATUS=$expected_status \
    WP16_EXPECTED_REASON=$expected_reason WP16_EXPECTED_MEM=$expected_mem \
WP16_EXPECTED_SWAP=$expected_swap WP16_EXPECTED_FULL=$expected_full python3 - <<'PY'
import json
import os
import re

records=[json.loads(line) for line in open(os.environ["WP16_RECORD"],encoding="utf-8") if line.strip()]
if not records:
    raise SystemExit("empty-gate-record")
record=records[-1]
if record.get("status")!=os.environ["WP16_EXPECTED_STATUS"]:
    raise SystemExit("resource-status")
if record.get("reason")!=os.environ["WP16_EXPECTED_REASON"]:
    raise SystemExit("resource-reason")
evidence=record.get("evidence",{})
if evidence.get("mem_available_kib") != int(os.environ["WP16_EXPECTED_MEM"]):
    raise SystemExit("resource-memory")
if evidence.get("swap_free_kib") != int(os.environ["WP16_EXPECTED_SWAP"]):
    raise SystemExit("resource-swap")
if not isinstance(evidence.get("resource_policy_sha256"),str) or not re.fullmatch(r"[0-9a-f]{64}",evidence["resource_policy_sha256"]):
    raise SystemExit("resource-policy-hash")
if not isinstance(evidence.get("memory_psi_some_avg10"),float) or evidence["memory_psi_some_avg10"] != 0.0:
    raise SystemExit("resource-some-psi")
if not isinstance(evidence.get("memory_psi_full_avg10"),float) or evidence["memory_psi_full_avg10"] != float(os.environ["WP16_EXPECTED_FULL"]):
    raise SystemExit("resource-full-psi")
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
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    unset RELEASE_BUILD_DRAINED_READERS_ATTESTATION
    unset RBL_GATE_DRAINED_STATUS RBL_GATE_DRAINED_BINDING
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

drained_inventory=lagrange-station-postgres-1,lagrange-station-reverse-proxy-1,lagrange-station-api-server-1,lagrange-station-web-1,lagrange-station-research-worker-1,lagrange-station-recommendation-runner-1,lagrange-station-candidate-runner-1,lagrange-station-owner-beta-runner-1,lagrange-station-owner-equity-v2-runner-1,lagrange-station-nt-backtest-worker-1-1,lagrange-station-nt-backtest-worker-2-1

run_drained_gate_case() {
  local name=$1 container_mode=$2 expected_status=$3 expected_reason=$4
  local meminfo=${5:-$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'}
  local pressure=${6:-$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'}
  local ps_output=${7:-} journal_mode=${8:-clean} repeat=${9:-0} drained_mode=${10:-valid}
  local case_dir=$test_dir/drained-cases/$name
  local state=$case_dir/state attestation=$case_dir/attestation.json
  mkdir -m 0700 -p -- "$case_dir"
  write_drained_attestation "$attestation"
  export WP16_MEMINFO_FIXTURE=$meminfo WP16_PRESSURE_FIXTURE=$pressure
  export WP16_CONTAINER_MODE=$container_mode WP16_JOURNAL_MODE=$journal_mode
  export WP16_DRAINED_READERS=1 WP16_DRAINED_MODE=$drained_mode
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  if [ -n "$ps_output" ]; then export WP16_PS_OUTPUT=$ps_output; else unset WP16_PS_OUTPUT; fi
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    unset RELEASE_BUILD_RESEARCH_EXCEPTION
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service
    export RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=$drained_inventory
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$attestation
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" "wp16-drained-$name" >"$case_dir/init.out" 2>"$case_dir/init.err"
    release_build_layout_gate "wp16-drained-$name-1" 0 >"$case_dir/gate1.out" 2>"$case_dir/gate1.err"
    if [ "$repeat" -eq 1 ]; then
      release_build_layout_gate "wp16-drained-$name-2" 0 >"$case_dir/gate2.out" 2>"$case_dir/gate2.err"
    fi
  ); then
    actual_status=PASS
  else
    actual_status=FAIL
  fi
  assert_record "$state" "$expected_status" "$expected_reason" ||
    fail "$name did not produce the expected drained-reader record (observed $actual_status)"
  if [ "$repeat" -eq 1 ]; then
    WP16_RECORD=$state/gates/gates.jsonl WP16_STATE=$state/gates/gate-state.json \
      WP16_RUN=$state/run.json python3 - <<'PY'
import json
import os

records=[json.loads(line) for line in open(os.environ["WP16_RECORD"],encoding="utf-8") if line.strip()]
if len(records)!=2 or any(item["status"]!="PASS" or item["reason"]!="image-build-only-drained-readers" for item in records):
    raise SystemExit("drained-repeat-gate-not-passing")
run=json.load(open(os.environ["WP16_RUN"],encoding="utf-8"))
binding=run["gate_inputs"].get("drained_readers_attestation")
if not isinstance(binding,dict) or not binding.get("path","").endswith("/attestation.json"):
    raise SystemExit("drained-run-binding-missing")
state=json.load(open(os.environ["WP16_STATE"],encoding="utf-8"))
attestation=state.get("drained_readers_attestation")
if not isinstance(attestation,dict) or set(attestation)!={"binding","first_observation","latest_observation"}:
    raise SystemExit("drained-state-binding-missing")
if set(attestation["first_observation"])!={
        "lagrange-station-research-worker-1","lagrange-station-owner-equity-v2-runner-1"}:
    raise SystemExit("drained-state-reader-origin-missing")
PY
  fi
}

run_drained_init_reject() {
  local name=$1 kind=$2 path_kind=${3:-regular}
  local case_dir=$test_dir/drained-init-reject/$name
  local state=$case_dir/state attestation=$case_dir/attestation.json input
  mkdir -m 0700 -p -- "$case_dir"
  if [ "$kind" != missing ]; then
    write_drained_attestation "$attestation" "$kind"
  fi
  case "$path_kind" in
    regular) input=$attestation ;;
    missing) input=$case_dir/missing.json ;;
    noncanonical) input=$case_dir//attestation.json ;;
    symlink)
      mv -- "$attestation" "$case_dir/real-attestation.json"
      ln -s -- "$case_dir/real-attestation.json" "$attestation"
      input=$attestation
      ;;
    unsafe) chmod 0640 -- "$attestation"; input=$attestation ;;
    *) fail "unknown drained init path kind: $path_kind" ;;
  esac
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    unset RELEASE_BUILD_RESEARCH_EXCEPTION
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service
    export RELEASE_BUILD_HEALTH_CONTAINERS=$drained_inventory
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$input
    RBL_LOCK_PREFIX=$case_dir/whole-lock
    export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" "wp16-init-reject-$name" \
      >"$case_dir/init.out" 2>"$case_dir/init.err"
  ); then
    fail "invalid drained attestation unexpectedly initialized: $name"
  fi
  [ ! -e "$state" ] && [ ! -L "$state" ] || fail "invalid attestation created state: $name"
}

run_drained_binding_case() {
  local name=$1 transition=$2
  local case_dir=$test_dir/drained-binding/$name
  local state=$case_dir/state attestation=$case_dir/attestation.json
  local expected_reason=drained-readers-attestation-removed
  [ "$transition" = replace ] && expected_reason=drained-readers-attestation-binding-changed
  [ "$transition" = expire ] && expected_reason=drained-readers-attestation-invalid
  [ "$transition" = start-stop ] && expected_reason=drained-readers-state-invalid
  mkdir -m 0700 -p -- "$case_dir"
  write_drained_attestation "$attestation"
  export WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
  export WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
  export WP16_CONTAINER_MODE=healthy WP16_JOURNAL_MODE=clean WP16_DRAINED_READERS=1 WP16_DRAINED_MODE=valid
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256 RELEASE_BUILD_RESEARCH_EXCEPTION
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service RELEASE_BUILD_HEALTH_CONTAINERS=$drained_inventory
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$attestation
    RBL_LOCK_PREFIX=$case_dir/whole-lock; export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" "wp16-binding-$name" >/dev/null 2>"$case_dir/init.err"
    release_build_layout_gate binding-1 0 >/dev/null 2>"$case_dir/first.err"
    case "$transition" in
      remove) unset RELEASE_BUILD_DRAINED_READERS_ATTESTATION ;;
      replace) rm -f -- "$attestation"; write_drained_attestation "$attestation" replace ;;
      expire) rm -f -- "$attestation"; write_drained_attestation "$attestation" expired ;;
      start-stop) export WP16_DRAINED_MODE=lifecycle-drift ;;
      *) exit 71 ;;
    esac
    release_build_layout_gate binding-2 0 >/dev/null 2>"$case_dir/second.err"
  ); then
    fail "drained binding transition unexpectedly passed: $name"
  fi
  assert_record "$state" FAIL "$expected_reason"
}

run_drained_inventory_reject() {
  local name=$1 inventory=$2 expected_reason=$3
  local case_dir=$test_dir/drained-inventory/$name
  local state=$case_dir/state attestation=$case_dir/attestation.json
  mkdir -m 0700 -p -- "$case_dir"
  write_drained_attestation "$attestation"
  export WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
  export WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
  export WP16_CONTAINER_MODE=healthy WP16_JOURNAL_MODE=clean WP16_DRAINED_READERS=1 WP16_DRAINED_MODE=valid
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256 RELEASE_BUILD_RESEARCH_EXCEPTION
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service RELEASE_BUILD_HEALTH_CONTAINERS=$inventory
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$attestation
    RBL_LOCK_PREFIX=$case_dir/whole-lock; export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" "wp16-inventory-$name" >/dev/null 2>"$case_dir/init.err"
    release_build_layout_gate inventory 0 >/dev/null 2>"$case_dir/gate.err"
  ); then
    fail "invalid drained inventory unexpectedly passed: $name"
  fi
  assert_record "$state" FAIL "$expected_reason"
}

run_drained_legacy_exclusion_case() {
  local name=simultaneous-legacy
  local case_dir=$test_dir/drained-legacy/$name
  local state=$case_dir/state attestation=$case_dir/attestation.json exception=$case_dir/exception
  mkdir -m 0700 -p -- "$case_dir"
  write_drained_attestation "$attestation"
  write_exception "$exception"
  export WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
  export WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
  export WP16_CONTAINER_MODE=research-exception WP16_JOURNAL_MODE=clean WP16_DRAINED_READERS=1 WP16_DRAINED_MODE=valid
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service RELEASE_BUILD_HEALTH_CONTAINERS=$drained_inventory
    export RELEASE_BUILD_RESEARCH_EXCEPTION=$exception
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$attestation
    RBL_LOCK_PREFIX=$case_dir/whole-lock; export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$state" wp16-simultaneous-legacy >/dev/null 2>"$case_dir/init.err"
    release_build_layout_gate legacy 0 >/dev/null 2>"$case_dir/gate.err"
  ); then
    fail 'simultaneous legacy exception and drained attestation unexpectedly passed'
  fi
  assert_record "$state" FAIL drained-readers-attestation-mutually-exclusive
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
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
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
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
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
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
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
if len(fields)!=31 or fields[28]!="verified" or fields[29]!="verified" or fields[30]!="pass:-":
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

run_frozen_c10_gate_case() {
  local name=$1 meminfo=$2 pressure=$3 container_mode=$4 journal_mode=$5
  local expected_status=$6 expected_reason=$7 nested=${8:-0}
  local case_dir=$test_dir/frozen-c10-cases/$name
  local public_state=$case_dir/public-gate-state
  local c10_state=$case_dir/c10-state
  local lock_prefix=$case_dir/whole-lock
  local actual_status
  mkdir -m 0700 -p -- "$case_dir"
  WP16_MEMINFO_FIXTURE=$meminfo
  WP16_PRESSURE_FIXTURE=$pressure
  WP16_CONTAINER_MODE=$container_mode
  WP16_JOURNAL_MODE=$journal_mode
  export WP16_MEMINFO_FIXTURE WP16_PRESSURE_FIXTURE WP16_CONTAINER_MODE WP16_JOURNAL_MODE
  unset WP16_EXCEPTION_PATH
  export WP16_SYSTEMCTL_LOG=$case_dir/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    repo_root=$fixture
    internal_self_test=0
    benchmark_gate_initialized=1
    benchmark_gate_source_root=$fixture
    benchmark_gate_source_commit=$source_commit
    benchmark_gate_state_root=$public_state
    benchmark_gate_namespace=wp16-c10-public-$name
    benchmark_gate_helper=$fixture/scripts/ops/lib/release-build-layout.sh
    benchmark_gate_lock_prefix=$lock_prefix
    BENCHMARK_SYSTEMD_SERVICE=wp16-build.service
    BENCHMARK_SYSTEMD_MANAGER=system
    BENCHMARK_PRODUCTION_HEALTH_UNITS=api.service,web.service
    BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=api-container,lagrange-station-research-worker-1
    BENCHMARK_RESEARCH_EXCEPTION=
    export BENCHMARK_SYSTEMD_SERVICE BENCHMARK_SYSTEMD_MANAGER
    export BENCHMARK_PRODUCTION_HEALTH_UNITS BENCHMARK_PRODUCTION_HEALTH_CONTAINERS
    export BENCHMARK_RESEARCH_EXCEPTION
    RBL_LOCK_PREFIX=$lock_prefix
    export RBL_LOCK_PREFIX
    common_gate_environment || exit 1
    # Hold the same parent lock that run_benchmark_layout_gate inherits, then
    # replace only the helper functions with the untouched frozen C10 copy.
    # The current public gate remains the implementation reached by binding.
    source "$benchmark_gate_helper"
    RBL_LOCK_PREFIX=$lock_prefix
    release_build_layout_lock || exit 1
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256
    unset RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    # shellcheck disable=SC1090
    source "$c10_fixture/scripts/ops/lib/release-build-layout.sh"
    RBL_LOCK_PREFIX=$lock_prefix
    common_gate_environment || exit 1
    common_c_gate_binding baseline warmup || exit 1
    release_build_layout_init "$c10_fixture" "$c10_commit" "$c10_state" "wp16-c10-$name" >/dev/null || exit 1
    if [ "$nested" -eq 1 ]; then
      c10_context_root=$c10_state
      rbl_make_context() { printf '%s\n' "$c10_context_root/contexts/fixture"; }
      rbl_produce_native() { return 0; }
      rbl_produce_bin() { return 0; }
      rbl_bundle_create() { return 0; }
      rbl_bundle_verify() { return 0; }
    fi
    release_build_layout_gate run-start 0 || exit 1
    if [ "$nested" -eq 1 ]; then
      release_build_layout_prepare api-server "$c10_commit" "$c10_state" >/dev/null || exit 1
    fi
  ) >"$case_dir/run.out" 2>"$case_dir/run.err"; then
    actual_status=PASS
  else
    actual_status=FAIL
  fi
  [ "$actual_status" = "$expected_status" ] ||
    fail "frozen C10 $name returned $actual_status, expected $expected_status"
  assert_record "$public_state" "$expected_status" "$expected_reason" ||
    fail "frozen C10 $name did not retain the expected public-gate record"
  [ -f "$public_state/run.json" ] && [ -f "$public_state/gates/gates.jsonl" ] ||
    fail "frozen C10 $name did not use persistent public-gate state"
  if [ "$nested" -eq 1 ]; then
    WP16_RECORD=$public_state/gates/gates.jsonl python3 - <<'PY'
import json
import os

records=[json.loads(line) for line in open(os.environ["WP16_RECORD"],encoding="utf-8") if line.strip()]
labels=[item.get("label") for item in records]
required={
    "common-C:baseline:warmup:run-start",
    "common-C:baseline:warmup:prepare:api-server",
    "common-C:baseline:warmup:native:D1",
}
if not required.issubset(labels):
    raise SystemExit("frozen-c10-direct-or-nested-gate-missing")
if not any(isinstance(label,str) and label.startswith("common-C:baseline:warmup:producer:D1:") for label in labels):
    raise SystemExit("frozen-c10-producer-gate-missing")
if any(item.get("status") != "PASS" for item in records):
    raise SystemExit("frozen-c10-nested-gate-failed")
PY
  fi
}

# Every case below reaches the unchanged public initializer and gate, which
# writes a canonical persistent record. Structured fields, not a status
# string, determine every expected outcome.
WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
run_gate_case swap-zero-plenty-ram healthy none PASS healthy
assert_resource_evidence "$test_dir/cases/swap-zero-plenty-ram/state" PASS healthy 4194304 0 0.0
WP16_MEMINFO_FIXTURE=$'MemAvailable:       1048576 kB\nSwapFree:            8388608 kB'
run_gate_case ram-below-floor healthy none FAIL mem-available-below-floor
assert_resource_evidence "$test_dir/cases/ram-below-floor/state" FAIL mem-available-below-floor 1048576 8388608 0.0
WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=5.00 avg60=0.00 avg300=0.00 total=1'
run_gate_case psi-exact-threshold healthy none FAIL memory-pressure-high
assert_resource_evidence "$test_dir/cases/psi-exact-threshold/state" FAIL memory-pressure-high 4194304 0 5.0
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=4.99 avg60=0.00 avg300=0.00 total=1'
run_gate_case psi-below-threshold healthy none PASS healthy
assert_resource_evidence "$test_dir/cases/psi-below-threshold/state" PASS healthy 4194304 0 4.99
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=NaN avg60=0.00 avg300=0.00 total=1'
run_gate_case psi-nan healthy none FAIL resource-observation-invalid
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
run_gate_case psi-missing-field healthy none FAIL resource-observation-invalid
WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
WP16_MEMINFO_FIXTURE=$'MemAvailable:       8722008 kB\nSwapFree:             499484 kB'
run_gate_case final-sample-low-psi healthy none PASS healthy
assert_resource_evidence "$test_dir/cases/final-sample-low-psi/state" PASS healthy 8722008 499484 0.0
WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB'
run_gate_case meminfo-missing-swap healthy none FAIL resource-observation-invalid
WP16_MEMINFO_FIXTURE=$'SwapFree:                  0 kB'
run_gate_case meminfo-missing-available healthy none FAIL resource-observation-invalid
WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
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

# Source the real frozen C10 helper for the dispatch regression. The first
# case proves zero swap plus low PSI reaches run-start and the frozen helper's
# nested producer gates through the current public gate. The following cases
# prove the public resource/OOM/health stops remain active on that same route.
c10_low_pressure=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
run_frozen_c10_gate_case zero-swap-nested \
  $'MemAvailable:       4194304 kB\nSwapFree:                  0 kB' \
  "$c10_low_pressure" healthy clean PASS healthy 1
run_frozen_c10_gate_case low-ram-plenty-swap \
  $'MemAvailable:       1048576 kB\nSwapFree:            8388608 kB' \
  "$c10_low_pressure" healthy clean FAIL mem-available-below-floor
run_frozen_c10_gate_case high-full-psi \
  $'MemAvailable:       4194304 kB\nSwapFree:                  0 kB' \
  $'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=5.00 avg60=0.00 avg300=0.00 total=1' \
  healthy clean FAIL memory-pressure-high
run_frozen_c10_gate_case kernel-oom \
  $'MemAvailable:       4194304 kB\nSwapFree:                  0 kB' \
  "$c10_low_pressure" healthy oom FAIL kernel-oom-observed
run_frozen_c10_gate_case unhealthy-service \
  $'MemAvailable:       4194304 kB\nSwapFree:                  0 kB' \
  "$c10_low_pressure" nonresearch-unhealthy clean FAIL container-health-invalid

# The drained-reader route reaches the actual checked-in helper with all eleven
# current project containers. Nine retain the ordinary healthy predicate; only
# the two named readers use the exact stopped-state attestation.
run_drained_gate_case positive-repeat healthy PASS image-build-only-drained-readers '' '' '' clean 1
run_drained_gate_case research-identity-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 id
run_drained_gate_case owner-identity-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 owner-id
run_drained_gate_case research-image-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 image
run_drained_gate_case owner-image-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 owner-image
run_drained_gate_case research-health-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 health
run_drained_gate_case owner-health-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 owner-health
run_drained_gate_case research-exit-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 exit
run_drained_gate_case owner-exit-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 owner-exit
run_drained_gate_case research-restart-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 restart
run_drained_gate_case owner-restart-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 owner-restart
run_drained_gate_case project-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 project
run_drained_gate_case started-time-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 started
run_drained_gate_case finished-time-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 finished
run_drained_gate_case running-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 running
run_drained_gate_case restarting-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 restarting
run_drained_gate_case paused-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 paused
run_drained_gate_case dead-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 dead
run_drained_gate_case oom-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 oom
run_drained_gate_case status-drift healthy FAIL drained-readers-state-invalid '' '' '' clean 0 status
run_drained_gate_case ordinary-serving-unhealthy nonresearch-unhealthy FAIL container-health-invalid
run_drained_gate_case drained-low-memory healthy FAIL mem-available-below-floor \
  $'MemAvailable:       1048576 kB\nSwapFree:            8388608 kB'
run_drained_gate_case drained-high-psi healthy FAIL memory-pressure-high \
  $'MemAvailable:       4194304 kB\nSwapFree:                  0 kB' \
  $'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=5.00 avg60=0.00 avg300=0.00 total=1'
run_drained_gate_case drained-compiler healthy FAIL compiler-process-active '' '' cargo
run_drained_gate_case drained-oom healthy FAIL kernel-oom-observed '' '' '' oom
run_drained_init_reject missing-attestation missing regular
run_drained_init_reject unsafe-attestation valid unsafe
run_drained_init_reject noncanonical-attestation valid noncanonical
run_drained_init_reject symlink-attestation valid symlink
run_drained_init_reject expired-attestation expired regular
run_drained_init_reject wrong-commit-attestation wrong-commit regular
run_drained_init_reject missing-reader-attestation missing-research regular
run_drained_init_reject extra-reader-attestation extra regular
run_drained_init_reject duplicate-reader-attestation duplicate regular
run_drained_init_reject invalid-time-attestation bad-time regular
run_drained_init_reject unknown-field-attestation unknown-field regular
run_drained_init_reject invalid-json-attestation bad-json regular
run_drained_inventory_reject missing-owner-inventory \
  lagrange-station-postgres-1,lagrange-station-reverse-proxy-1,lagrange-station-api-server-1,lagrange-station-web-1,lagrange-station-research-worker-1,lagrange-station-recommendation-runner-1,lagrange-station-candidate-runner-1,lagrange-station-owner-beta-runner-1,lagrange-station-nt-backtest-worker-1-1,lagrange-station-nt-backtest-worker-2-1 \
  drained-readers-container-inventory-invalid
run_drained_inventory_reject unknown-inventory "$drained_inventory,lagrange-station-unknown-1" \
  drained-readers-container-inventory-invalid
run_drained_legacy_exclusion_case
run_drained_binding_case removed-binding remove
run_drained_binding_case replaced-binding replace
run_drained_binding_case expired-binding expire
run_drained_binding_case lifecycle-start-then-stop start-stop

# Adding a drained binding after a normal initialized gate is also rejected;
# the build cannot silently acquire the exception after run initialization.
{
  add_case=$test_dir/drained-binding/added-binding
  mkdir -m 0700 -p -- "$add_case"
  add_state=$add_case/state
  add_attestation=$add_case/attestation.json
  write_drained_attestation "$add_attestation"
  export WP16_MEMINFO_FIXTURE=$'MemAvailable:       4194304 kB\nSwapFree:                  0 kB'
  export WP16_PRESSURE_FIXTURE=$'some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1'
  export WP16_CONTAINER_MODE=healthy WP16_JOURNAL_MODE=clean WP16_DRAINED_READERS=0
  export WP16_SYSTEMCTL_LOG=$add_case/systemctl.tsv
  : >"$WP16_SYSTEMCTL_LOG"
  if (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT
    unset RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT
    unset RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256
    unset RELEASE_BUILD_LAYOUT_HELPER_SHA256 RELEASE_BUILD_LAYOUT_CONFIG_SHA256
    unset RELEASE_BUILD_LAYOUT_RESOURCE_POLICY_SHA256 RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    unset RELEASE_BUILD_RESEARCH_EXCEPTION RELEASE_BUILD_DRAINED_READERS_ATTESTATION
    export RELEASE_BUILD_SYSTEMD_UNIT=wp16-build.service RELEASE_BUILD_SYSTEMD_MANAGER=system
    export RELEASE_BUILD_HEALTH_UNITS=api.service,web.service RELEASE_BUILD_HEALTH_CONTAINERS=$drained_inventory
    RBL_LOCK_PREFIX=$add_case/whole-lock; export RBL_LOCK_PREFIX
    source "$product_helper"
    release_build_layout_init "$fixture" "$source_commit" "$add_state" wp16-binding-added >/dev/null 2>"$add_case/init.err"
    release_build_layout_gate added-1 0 >/dev/null 2>"$add_case/first.err"
    export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=$add_attestation WP16_DRAINED_READERS=1 WP16_DRAINED_MODE=valid
    release_build_layout_gate added-2 0 >/dev/null 2>"$add_case/second.err"
  ); then
    fail 'drained attestation added after initialization unexpectedly passed'
  fi
  assert_record "$add_state" FAIL drained-readers-attestation-added
}

# The benchmark has no deliberate drained-reader contract. Reject both a
# benchmark-specific input and accidental inheritance from the production
# environment rather than passing the exception through to the shared helper.
unset RELEASE_BUILD_DRAINED_READERS_ATTESTATION
export BENCHMARK_DRAINED_READERS_ATTESTATION=/tmp/synthetic-drained.json
if common_gate_environment; then
  fail 'benchmark accepted unsupported drained-reader attestation input'
fi
[ "${BENCHMARK_GATE_REJECTION:-}" = production-drained-readers-attestation-unsupported ] ||
  fail 'benchmark did not report explicit drained-reader input rejection'
unset BENCHMARK_DRAINED_READERS_ATTESTATION BENCHMARK_GATE_REJECTION
export RELEASE_BUILD_DRAINED_READERS_ATTESTATION=/tmp/inherited-drained.json
if common_gate_environment; then
  fail 'benchmark inherited production drained-reader attestation input'
fi

printf 'BUILD_CACHE_BENCHMARK_GATE_SELF_TEST: PASS (real public init/gate with full drained-reader matrix; no Docker, Rust, or systemd mutation)\n'
