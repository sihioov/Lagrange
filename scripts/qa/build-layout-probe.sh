#!/usr/bin/env bash
# Frozen WP-3 build-layout prototype.  --apply is gated future code; --self-test
# substitutes only owned fakes and never reaches Docker, Cargo, or host APIs.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/../.." && pwd -P)
script_path="$script_dir/$(basename "${BASH_SOURCE[0]}")"
fixture_dir="$repo_root/tests/fixtures/build-layout"
helper_path="$fixture_dir/layout-helper.sh"
design_path="$repo_root/docs/design/production-build-layout.md"
baseline_doc="$repo_root/docs/verification/production-build-baseline.md"

layouts=(baseline common grouped)
case_ids=(cold exact-repeat forced-warm commit-only web-only python-only app-source-old-mtime lib-source embedded build-script build-setting manifest-key lock-key config-key feature-switch branch-return compile-fail-p2 export-fail-p2 wrong-commit missing-bin tampered-bin missing-complete stop-after-p2 delete-input delete-source empty-cache broken-ledger route-contract)
declare -A case_numbers=([cold]=01 [exact-repeat]=02 [forced-warm]=03 [commit-only]=04 [web-only]=05 [python-only]=06 [app-source-old-mtime]=07 [lib-source]=08 [embedded]=09 [build-script]=10 [build-setting]=11 [manifest-key]=12 [lock-key]=13 [config-key]=14 [feature-switch]=15 [branch-return]=16 [compile-fail-p2]=17 [export-fail-p2]=18 [wrong-commit]=19 [missing-bin]=20 [tampered-bin]=21 [missing-complete]=22 [stop-after-p2]=23 [delete-input]=24 [delete-source]=25 [empty-cache]=26 [broken-ledger]=27 [route-contract]=28)
declare -A p_bin=([p1]=cache-bin-a [p2]=cache-bin-b [p3]=cache-bin-a [p4]=cache-bin-b)
declare -A p_variant=([p1]=base [p2]=base [p3]=wide [p4]=wide)
declare -A p_runtime=([p1]=web.txt [p2]=python.txt [p3]=web.txt [p4]=python.txt)
declare -A p_group=([p1]=base [p2]=base [p3]=wide [p4]=wide)
phases=(p1 p2 p3 p4)

c1=1111111111111111111111111111111111111111
c2=2222222222222222222222222222222222222222
c3=3333333333333333333333333333333333333333
mode=plan
layout_selection=all
case_selection=all
output_dir=
resume_dir=
layout_seen=0
case_seen=0
output_seen=0
resume_seen=0
docker_bin=${DOCKER_BIN:-docker}
internal_self_test=0

run_dir= run_id= events_file= gate_file= run_json= timing_file=
execution_lock_fd=
origin_head= fixture_sha= tool_sha= design_sha= baseline_sha=
CUR_LAYOUT= CUR_CASE= CUR_SCOPE= CUR_TOKEN= CUR_SOURCE= CUR_ATTEMPT=0 CUR_ATTEMPT_DIR=
CUR_COMMIT= CUR_SETTING= CUR_CONFIG= CUR_COLD=0 CUR_PHASE=
CUR_COMPILE= CUR_LIB= CUR_APP= CUR_RUNTIME= CUR_RECIPE= CUR_GUARD= CUR_K= CUR_H= CUR_HOST= CUR_IDENTITY_SHA=
CUR_SHARED_CACHE= CUR_TARGET_CACHE= CUR_LEDGER_INJECT=none CUR_SOURCE_TRAP=0

source "$helper_path"
# Internal bindings are never inherited from the caller's environment.
LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=

die() { printf '%s\n' "build-layout-probe: $*" >&2; exit 2; }
unresolved() { printf '%s\n' "build-layout-probe: FAILED_UNRESOLVED: $*" >&2; return 1; }
now_ms() { date +%s%3N; }
requested_features() { [ "$1" = wide ] && printf '%s\n' '["wide"]' || printf '%s\n' '[]'; }
resolved_features() { [ "$1" = wide ] && printf '%s\n' '["default","wide"]' || printf '%s\n' '["default"]'; }
compile_env() { printf '{"CACHE_FIXTURE_BUILD_SETTING":"%s","CACHE_FIXTURE_COMMIT":"%s","CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target"}\n' "$1" "$2"; }

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-layout-probe.sh [--plan|--apply|--self-test]
       [--layout baseline|common|grouped|all]
       [--case <01..28 name>|all] --output-dir <new canonical absolute dir>
       [--resume-from <existing canonical absolute dir>]
EOF
}

safe_absolute() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label is empty"
  [[ "$path" = /* && "$path" != *$'\n'* && "$path" != *$'\r'* && "$path" != *'//'* && "$path" != */../* && "$path" != */.. && "$path" != */./* && "$path" != */. && "$path" != */ ]] || die "$label is not canonical"
  case "$path" in /|/tmp|/var|/usr|/usr/local|/opt) die "$label is too broad" ;; esac
  probe=$path
  while [ "$probe" != / ]; do [ ! -L "$probe" ] || die "$label traverses a symlink"; probe=${probe%/*}; [ -n "$probe" ] || probe=/; done
}

validate_output() {
  safe_absolute "$output_dir" output-dir
  [[ "$(basename "$output_dir")" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$ ]] || die 'output-dir basename is not a safe run ID'
  case "$output_dir" in "$repo_root"|"$repo_root"/*) die 'output-dir must be outside the repository' ;; esac
  if [ -e "$output_dir" ]; then
    [ -d "$output_dir" ] && [ ! -L "$output_dir" ] || die 'output-dir is not a regular directory'
    [ -z "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ] || die 'output-dir must be empty'
  else
    local parent=${output_dir%/*}; [ -d "$parent" ] && [ ! -L "$parent" ] || die 'output-dir parent is unavailable'
  fi
}

validate_resume() {
  safe_absolute "$resume_dir" resume-from
  [[ "$(basename "$resume_dir")" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$ ]] || die 'resume-from basename is not a safe run ID'
  [ -d "$resume_dir" ] && [ ! -L "$resume_dir" ] && [ -f "$resume_dir/run.json" ] && [ -f "$resume_dir/run.json.sha256" ] && [ -f "$resume_dir/events.jsonl" ] && [ -f "$resume_dir/gates.jsonl" ] || die 'resume state is incomplete'
  case "$resume_dir" in "$repo_root"|"$repo_root"/*) die 'resume-from must be outside the repository' ;; esac
}

acquire_execution_slot() {
  # One inode for all actual invocations, independent of checkout/run/TMPDIR.
  # Never unlink it: closing the inherited descriptor releases flock on exit.
  local root=/tmp/lagrange-build-layout-execution
  if [ "$#" -gt 0 ]; then
    [ "$internal_self_test" = 1 ] && gate_test_allowed || return 1
    root=$1
    [[ "$root" = /tmp/* ]] || return 1
  fi
  [ -z "$execution_lock_fd" ] || { unresolved 'execution slot already held'; return 1; }
  python3 - "$root" <<'PY'
import os,stat,sys
path=sys.argv[1]
if os.path.realpath(path)!=path:raise SystemExit("execution-slot-path-invalid")
try:os.mkdir(path,0o700)
except FileExistsError:pass
info=os.lstat(path)
if not stat.S_ISDIR(info.st_mode) or info.st_uid!=os.geteuid() or stat.S_IMODE(info.st_mode)!=0o700:
    raise SystemExit("execution-slot-owner-or-mode-invalid")
PY
  [ "$?" -eq 0 ] || return 1
  exec {execution_lock_fd}<"$root" || return 1
  if python3 - "$execution_lock_fd" "$root" <<'PY'
import fcntl,os,stat,sys
fd=int(sys.argv[1]);info=os.fstat(fd);path=os.lstat(sys.argv[2])
if (not stat.S_ISDIR(info.st_mode) or not stat.S_ISDIR(path.st_mode) or
        (info.st_dev,info.st_ino)!=(path.st_dev,path.st_ino) or
        info.st_uid!=os.geteuid() or stat.S_IMODE(info.st_mode)!=0o700):
    raise SystemExit("execution-slot-identity-invalid")
try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
except BlockingIOError:raise SystemExit("execution-slot-busy")
PY
  then return 0
  else
    exec {execution_lock_fd}<&-
    execution_lock_fd=
    unresolved 'execution slot unavailable; no host/build commands started'
    return 1
  fi
}

parse_case() {
  local value=$1 item
  [ "$value" = all ] && { printf '%s\n' all; return; }
  for item in "${case_ids[@]}"; do [ "$item" = "$value" ] && { printf '%s\n' "$item"; return; }; done
  for item in "${!case_numbers[@]}"; do [ "${case_numbers[$item]}" = "$value" ] && { printf '%s\n' "$item"; return; }; done
  die "unknown case: $value"
}

validate_fixture() {
  local path
  [ -d "$fixture_dir" ] && [ ! -L "$fixture_dir" ] || die 'fixture is missing'
  local needed=(Cargo.toml Cargo.lock rust-toolchain.toml layout.json layout-helper.sh compose.yml Dockerfile.baseline Dockerfile.artifacts Dockerfile.consumer fixture-lib/Cargo.toml fixture-lib/src/lib.rs fixture-app/Cargo.toml fixture-app/build.rs fixture-app/data/embedded.txt fixture-app/src/bin/cache-bin-a.rs fixture-app/src/bin/cache-bin-b.rs runtime/web.txt runtime/python.txt)
  for path in "${needed[@]}"; do [ -f "$fixture_dir/$path" ] && [ ! -L "$fixture_dir/$path" ] || die "fixture file missing: $path"; done
  grep -Fq 'FROM scratch AS artifacts' "$fixture_dir/Dockerfile.artifacts" || die 'artifact scratch export missing'
  grep -Fq 'COPY --from=artifact-builder /out/ /' "$fixture_dir/Dockerfile.artifacts" || die 'artifact export is not flat'
  grep -Fq 'apk add --no-cache bash python3' "$fixture_dir/Dockerfile.artifacts" || die 'artifact Bash interpreter missing'
  grep -Fq '/identity/identity.json' "$fixture_dir/Dockerfile.artifacts" || die 'P0 JSON identity export missing'
  grep -Fq 'PROBE_EXPECTED_NATIVE_SHA256' "$fixture_dir/Dockerfile.baseline" || die 'baseline P1 native identity recheck missing'
  grep -Fq 'PROBE_EXPECTED_NATIVE_SHA256' "$fixture_dir/Dockerfile.artifacts" || die 'artifact P1 native identity recheck missing'
  grep -Fq 'PROBE_COMPILE_INPUT_SHA256' "$fixture_dir/Dockerfile.artifacts" || die 'P/H guard input separation missing'
  grep -Fq 'FROM ${RUST_ARTIFACT_SOURCE} AS builder' "$fixture_dir/Dockerfile.consumer" || die 'consumer source/artifact alias missing'
  grep -Fq 'AS runtime' "$fixture_dir/Dockerfile.consumer" || die 'consumer runtime target missing'
  grep -Fq 'cargo clean --workspace --release --locked' "$fixture_dir/Dockerfile.baseline" || die 'baseline clean missing'
  grep -Fq 'build-cache-fixture-lib' "$fixture_dir/Dockerfile.artifacts" || die 'real package name missing'
  if grep -Fq '|| true' "$fixture_dir"/Dockerfile.*; then die 'masked Dockerfile failure detected'; fi
}

hash_inputs() {
  fixture_sha=$(bash "$helper_path" tree-hash "$fixture_dir")
  tool_sha=$(sha256sum -- "$script_path" | awk '{print $1}')
  design_sha=$(sha256sum -- "$design_path" | awk '{print $1}')
  baseline_sha=$(sha256sum -- "$baseline_doc" | awk '{print $1}')
  origin_head=$(git -C "$repo_root" rev-parse HEAD)
}

scope_for() { case "$1" in cold|exact-repeat|forced-warm) printf '%s\n' warm-chain ;; *) printf '%s\n' "$1" ;; esac; }
commit_for() { case "$1" in commit-only|web-only|python-only) printf '%s\n' "$c2" ;; *) printf '%s\n' "$c1" ;; esac; }
setting_for() { [ "$1" = build-setting ] && printf '%s\n' changed || printf '%s\n' default; }
config_for() { [ "$1" = config-key ] && printf '%s\n' build.jobs=2 || printf '%s\n' default; }
service_for() { case "$1" in p1) printf '%s\n' probe-a-base ;; p2) printf '%s\n' probe-b-base ;; p3) printf '%s\n' probe-a-wide ;; p4) printf '%s\n' probe-b-wide ;; esac; }

expected_stop_case() { case "$1" in compile-fail-p2|export-fail-p2|wrong-commit|missing-bin|tampered-bin|missing-complete|stop-after-p2|delete-input|delete-source) return 0 ;; *) return 1 ;; esac; }
needs_warmup() { case "$1" in cold|exact-repeat|forced-warm|empty-cache|route-contract) return 1 ;; *) return 0 ;; esac; }
first_trial_injection() { [ "${RESUMING:-0}" != 1 ] && [[ "$CUR_TOKEN" != *-setup ]]; }

source_copy() {
  local dst=$1
  [ ! -e "$dst" ] || { unresolved "source destination exists: $dst"; return 1; }
  mkdir -p -m 0700 -- "$(dirname "$dst")"
  mkdir -m 0700 -- "$dst"
  cp -a -- "$fixture_dir/." "$dst/"
  [ "$(bash "$helper_path" source-input-hash "$dst")" = "$fixture_sha" ] || { unresolved 'fixture copy identity changed'; return 1; }
}

old_mtime() { find "$1" -type f -exec touch -d '2000-01-01 00:00:00 UTC' {} +; }
mutate_source() {
  local source=$1 case_id=$2 step=${3:-normal}
  case "$case_id" in
    web-only) printf 'web-v2\n' >"$source/runtime/web.txt" ;;
    python-only) printf 'python-v2\n' >"$source/runtime/python.txt" ;;
    app-source-old-mtime) sed -i '0,/source-v1/s//source-v2/' "$source/fixture-app/src/bin/cache-bin-a.rs"; old_mtime "$source" ;;
    lib-source) sed -i 's/else { 42 }/else { 43 }/' "$source/fixture-lib/src/lib.rs" ;;
    embedded) printf 'embedded-v2\n' >"$source/fixture-app/data/embedded.txt" ;;
    build-script) sed -i 's/generated-v1/generated-v2/' "$source/fixture-app/build.rs" ;;
    manifest-key) printf '# layout-manifest-v2\n' >>"$source/fixture-app/Cargo.toml" ;;
    lock-key) printf '# layout-lock-v2\n' >>"$source/Cargo.lock" ;;
    delete-input) rm -f -- "$source/fixture-app/data/embedded.txt"; old_mtime "$source" ;;
    delete-source) rm -f -- "$source/fixture-app/src/bin/cache-bin-b.rs"; old_mtime "$source" ;;
    branch-return) [ "$step" = forward ] && sed -i '0,/source-v1/s//source-v2/' "$source/fixture-app/src/bin/cache-bin-a.rs"; old_mtime "$source" ;;
  esac
}

expected_stdout() {
  local source=$1 bin=$2 variant=$3 commit=$4 setting=$5 marker embedded generated shared
  marker=source-v1; [ "$bin" = cache-bin-a ] && grep -Fq 'SOURCE_MARKER: &str = "source-v2"' "$source/fixture-app/src/bin/cache-bin-a.rs" && marker=source-v2
  embedded=$(tr -d '\n' <"$source/fixture-app/data/embedded.txt") || return 1
  generated=generated-v1; grep -Fq generated-v2 "$source/fixture-app/build.rs" && generated=generated-v2
  shared=42; [ "$variant" = wide ] && shared=84; [ "$variant" = base ] && grep -Fq 'else { 43 }' "$source/fixture-lib/src/lib.rs" && shared=43
  printf '%s\n' "$bin|commit=$commit|setting=$setting|source=$marker|embedded=$embedded|$generated|commit=$commit|setting=$setting|embedded=$embedded|shared=$shared"
}

recipe_hash() {
  local root=${2:-$fixture_dir}
  # Hash stable relative names and bytes.  Absolute copied-source paths are
  # attempt-local evidence, not recipe inputs.
  (
    cd "$root"
    case "$1" in
      baseline) sha256sum -- Dockerfile.baseline layout-helper.sh layout.json ;;
      *) sha256sum -- Dockerfile.artifacts Dockerfile.consumer layout-helper.sh layout.json ;;
    esac
  ) | sha256sum | awk '{print $1}'
}

state_dir() { printf '%s/state/%s/%s/%s\n' "$run_dir" "$1" "$2" "$3"; }
state_phase() { printf '%s/%s.json\n' "$(state_dir "$1" "$2" "$3")" "$4"; }
state_identity() { printf '%s/identity.json\n' "$(state_dir "$1" "$2" "$3")"; }

compute_inputs() {
  local identity=$1
  CUR_COMPILE=$(bash "$helper_path" compile-input-hash "$CUR_SOURCE") || return 1
  CUR_LIB=$(bash "$helper_path" tree-hash "$CUR_SOURCE/fixture-lib") || return 1
  CUR_APP=$(bash "$helper_path" tree-hash "$CUR_SOURCE/fixture-app") || return 1
  CUR_RUNTIME=$(bash "$helper_path" tree-hash "$CUR_SOURCE/runtime") || return 1
  CUR_RECIPE=$(recipe_hash "$CUR_LAYOUT" "$CUR_SOURCE")
  CUR_GUARD=$(sha256sum -- "$helper_path" | awk '{print $1}')
  CUR_HOST=$(bash "$helper_path" identity-validate "$identity" linux/amd64) || return 1
  CUR_IDENTITY_SHA=$(sha256sum -- "$identity" | awk '{print $1}')
  CUR_K=$(bash "$helper_path" compatibility-key "$CUR_SOURCE" "$identity" "$CUR_RECIPE" "$CUR_GUARD" linux/amd64 "$CUR_CONFIG") || return 1
  CUR_H=$(printf '%s\n' "K=$CUR_K" "P-lib=$CUR_LIB" "P-app=$CUR_APP" "bin=${p_bin[$CUR_PHASE]}" "requested-features=$(requested_features "${p_variant[$CUR_PHASE]}")" "resolved-features=$(resolved_features "${p_variant[$CUR_PHASE]}")" "commit=$CUR_COMMIT" "setting=$CUR_SETTING" | sha256sum | awk '{print $1}')
  CUR_SHARED_CACHE="build-layout-$run_id-$CUR_LAYOUT-$CUR_SCOPE-$CUR_K"
  CUR_TARGET_CACHE="$CUR_SHARED_CACHE-target"
  [ "$CUR_LAYOUT" = grouped ] && CUR_TARGET_CACHE="$CUR_TARGET_CACHE-${p_group[$CUR_PHASE]}"
  return 0
}

emit_event() {
  local type=$1 layout=$2 case_id=$3 phase=$4 attempt=$5 status=$6 code=$7 detail=$8
  PROBE_EVENTS_PATH="$events_file" PROBE_TYPE="$type" PROBE_LAYOUT="$layout" PROBE_CASE="$case_id" PROBE_PHASE="$phase" PROBE_ATTEMPT="$attempt" PROBE_STATUS="$status" PROBE_CODE="$code" PROBE_DETAIL="$detail" PROBE_TOKEN="$CUR_TOKEN" python3 - <<'PY' >>"$events_file"
import hashlib,json,os,time
path=os.environ.get("PROBE_EVENTS_PATH")
previous="0"*64; sequence=1
if path and os.path.getsize(path):
    with open(path,encoding="utf-8") as handle:
        lines=handle.read().splitlines()
    last=json.loads(lines[-1]); previous=last["record_sha256"]; sequence=last["sequence"]+1
v={"attempt":int(os.environ["PROBE_ATTEMPT"]),"case":os.environ["PROBE_CASE"],"detail":os.environ["PROBE_DETAIL"],"exit":int(os.environ["PROBE_CODE"]),"layout":os.environ["PROBE_LAYOUT"],"phase":os.environ["PROBE_PHASE"],"previous_sha256":previous,"sequence":sequence,"status":os.environ["PROBE_STATUS"],"time_unix":time.time(),"token":os.environ["PROBE_TOKEN"],"type":os.environ["PROBE_TYPE"]}
v["record_sha256"]=hashlib.sha256(json.dumps(v,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode()).hexdigest()
print(json.dumps(v,sort_keys=True,separators=(",",":"),ensure_ascii=False))
PY
}

validate_event_chain() {
  python3 - "$events_file" <<'PY'
import hashlib,json,re,sys
previous="0"*64; sequence=0
for raw in open(sys.argv[1],encoding="utf-8"):
    sequence+=1
    value=json.loads(raw)
    if value.get("sequence")!=sequence or value.get("previous_sha256")!=previous:raise SystemExit("event-chain-order-invalid")
    digest=value.pop("record_sha256",None)
    if not isinstance(digest,str) or not re.fullmatch(r"[0-9a-f]{64}",digest):raise SystemExit("event-chain-hash-invalid")
    actual=hashlib.sha256(json.dumps(value,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode()).hexdigest()
    if actual!=digest:raise SystemExit("event-chain-content-invalid")
    previous=digest
if sequence==0:raise SystemExit("event-chain-empty")
PY
}

record_timing() {
  local category=$1 scope=$2 started=$3 ended=$4 cargo=${5:-not-separated}
  printf '%s\t%s\t%s\t%s\t%s\n' "$category" "$scope" "$((ended-started))" "$cargo" "$ended" >>"$timing_file"
}

write_run_json() {
  PROBE_RESEARCH="$LAYOUT_RESEARCH_BINDING" PROBE_PATH="$run_json" PROBE_HEAD="$origin_head" PROBE_FIXTURE="$fixture_sha" PROBE_TOOL="$tool_sha" PROBE_DESIGN="$design_sha" PROBE_BASELINE="$baseline_sha" PROBE_RUN="$run_id" PROBE_LAYOUT="$layout_selection" PROBE_CASE="$case_selection" PROBE_UNITS="${BUILD_LAYOUT_HEALTH_UNITS:-}" PROBE_CONTAINERS="${BUILD_LAYOUT_HEALTH_CONTAINERS:-}" PROBE_SINCE_US="$LAYOUT_JOURNAL_SINCE_US" PROBE_SINCE="$LAYOUT_JOURNAL_SINCE" PROBE_HELPER="$(sha256sum -- "$helper_path" | awk '{print $1}')" PROBE_LAYOUT_JSON="$(sha256sum -- "$fixture_dir/layout.json" | awk '{print $1}')" PROBE_COMPOSE="$(sha256sum -- "$fixture_dir/compose.yml" | awk '{print $1}')" PROBE_DOCKER_BASELINE="$(sha256sum -- "$fixture_dir/Dockerfile.baseline" | awk '{print $1}')" PROBE_DOCKER_ARTIFACTS="$(sha256sum -- "$fixture_dir/Dockerfile.artifacts" | awk '{print $1}')" PROBE_DOCKER_CONSUMER="$(sha256sum -- "$fixture_dir/Dockerfile.consumer" | awk '{print $1}')" PROBE_RECIPE_BASELINE="$(recipe_hash baseline)" PROBE_RECIPE_COMMON="$(recipe_hash common)" PROBE_RECIPE_GROUPED="$(recipe_hash grouped)" python3 - <<'PY'
import json,os,time
v={"baseline_document_sha256":os.environ["PROBE_BASELINE"],"cases":["cold","exact-repeat","forced-warm","commit-only","web-only","python-only","app-source-old-mtime","lib-source","embedded","build-script","build-setting","manifest-key","lock-key","config-key","feature-switch","branch-return","compile-fail-p2","export-fail-p2","wrong-commit","missing-bin","tampered-bin","missing-complete","stop-after-p2","delete-input","delete-source","empty-cache","broken-ledger","route-contract"],"command_contract":{"cargo_build_jobs":"2","cargo_target_dir":"/cargo-target","compose_parallel_limit":"1","platform":"linux/amd64","profile":"release","workdir":"/build"},"document_sha256":os.environ["PROBE_DESIGN"],"fixture_source_input_sha256":os.environ["PROBE_FIXTURE"],"gate":{"containers":os.environ["PROBE_CONTAINERS"].split(",") if os.environ["PROBE_CONTAINERS"] else [],"journal_since":os.environ["PROBE_SINCE"],"journal_since_us":int(os.environ["PROBE_SINCE_US"]),"mem_available_kib":2097152,"swap_free_kib":524288,"units":os.environ["PROBE_UNITS"].split(",") if os.environ["PROBE_UNITS"] else []},"initial_state":"created","layouts":["baseline","common","grouped"],"origin_head":os.environ["PROBE_HEAD"],"recipes":{"Dockerfile.artifacts":os.environ["PROBE_DOCKER_ARTIFACTS"],"Dockerfile.baseline":os.environ["PROBE_DOCKER_BASELINE"],"Dockerfile.consumer":os.environ["PROBE_DOCKER_CONSUMER"],"compose.yml":os.environ["PROBE_COMPOSE"],"layout-helper.sh":os.environ["PROBE_HELPER"],"layout.json":os.environ["PROBE_LAYOUT_JSON"],"layout_recipe":{"baseline":os.environ["PROBE_RECIPE_BASELINE"],"common":os.environ["PROBE_RECIPE_COMMON"],"grouped":os.environ["PROBE_RECIPE_GROUPED"]}},"run_id":os.environ["PROBE_RUN"],"schema":"lagrange-build-layout-probe-v3","selection":{"case":os.environ["PROBE_CASE"],"layout":os.environ["PROBE_LAYOUT"]},"source":{"build_setting":"default","commit":"1111111111111111111111111111111111111111","embedded":"embedded-v1","generated_prefix":"generated-v1","lib_base":42,"lib_wide":84,"marker":"source-v1"},"started_at_unix":time.time(),"synthetic_commits":["1111111111111111111111111111111111111111","2222222222222222222222222222222222222222","3333333333333333333333333333333333333333"],"tool_sha256":os.environ["PROBE_TOOL"]}
v["gate"]["research_exception"]=json.loads(os.environ["PROBE_RESEARCH"])
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8",newline="\n") as o:o.write(json.dumps(v,sort_keys=True,separators=(",",":"))+"\n")
PY
  chmod 0600 -- "$run_json"
  sha256sum -- "$run_json" | awk '{print $1}' >"$run_dir/run.json.sha256"
  chmod 0600 -- "$run_dir/run.json.sha256"
}

set_run_status() {
  local status=$1 code=${2:-0}
  emit_event RUN_STATUS "${CUR_LAYOUT:-$layout_selection}" "${CUR_CASE:-$case_selection}" "${CUR_PHASE:-suite}" "${CUR_ATTEMPT:-0}" "$status" "$code" immutable-run-json
}

identity_build() {
  local source=$1 destination=$2 log=$3 dockerfile status
  mkdir -p -m 0700 -- "$destination"
  dockerfile=Dockerfile.artifacts; [ "$CUR_LAYOUT" = baseline ] && dockerfile=Dockerfile.baseline
  local no_cache=(); [ "$CUR_COLD" = 1 ] && no_cache=(--no-cache)
  if DOCKER_BUILDKIT=1 "$docker_bin" buildx build --progress plain "${no_cache[@]}" --file "$source/$dockerfile" --target identity --platform linux/amd64 --output "type=local,dest=$destination,platform-split=false" --build-arg TARGETPLATFORM=linux/amd64 "$source" >"$log" 2>&1; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || return "$status"
  [ -f "$destination/identity.json" ] && [ ! -L "$destination/identity.json" ] || return 1
  bash "$helper_path" identity-validate "$destination/identity.json" linux/amd64 >/dev/null
}

artifact_build() {
  local phase=$1 destination=$2 log=$3 status bin variant runtime
  bin=${p_bin[$phase]} variant=${p_variant[$phase]} runtime=${p_runtime[$phase]}
  mkdir -p -m 0700 -- "$destination"
  # Later phases retain the target mount populated by this cold chain.
  local no_cache=(); [ "$CUR_COLD" = 1 ] && [ "$phase" = p1 ] && no_cache=(--no-cache)
  local inject=none
  if [ "$CUR_CASE/$phase" = export-fail-p2/p2 ] && first_trial_injection; then inject=export-fail-p2; fi
  if DOCKER_BUILDKIT=1 "$docker_bin" buildx build --progress plain "${no_cache[@]}" --file "$CUR_SOURCE/Dockerfile.artifacts" --target artifacts --platform linux/amd64 --output "type=local,dest=$destination,platform-split=false" \
    --build-arg TARGETPLATFORM=linux/amd64 --build-arg "BIN=$bin" --build-arg "VARIANT=$variant" --build-arg "RUNTIME_FILE=$runtime" \
    --build-arg "CACHE_FIXTURE_COMMIT=$CUR_COMMIT" --build-arg "CACHE_FIXTURE_BUILD_SETTING=$CUR_SETTING" --build-arg "CACHE_FIXTURE_RUN_TOKEN=$CUR_TOKEN" \
    --build-arg "PROBE_SHARED_CACHE_ID=$CUR_SHARED_CACHE" --build-arg "PROBE_TARGET_CACHE_ID=$CUR_TARGET_CACHE" --build-arg "PROBE_CACHE_KEY=$CUR_K" \
    --build-arg "PROBE_INPUT_SHA256=$CUR_H" --build-arg "PROBE_COMPILE_INPUT_SHA256=$CUR_COMPILE" --build-arg "PROBE_LIB_HASH=$CUR_LIB" --build-arg "PROBE_APP_HASH=$CUR_APP" \
    --build-arg "PROBE_RECIPE_SHA256=$CUR_RECIPE" --build-arg PROBE_PLATFORM=linux/amd64 --build-arg "PROBE_HOST_TRIPLE=$CUR_HOST" --build-arg "PROBE_EXPECTED_NATIVE_SHA256=$CUR_IDENTITY_SHA" --build-arg "PROBE_CARGO_CONFIG=$CUR_CONFIG" \
    --build-arg "PROBE_LEDGER_INJECT=$CUR_LEDGER_INJECT" --build-arg "PROBE_INJECT=$inject" "$CUR_SOURCE" >"$log" 2>&1; then status=0; else status=$?; fi
  if [ "$status" -ne 0 ]; then
    printf '{"buildx_exit":%s,"helper_exit":null}\n' "$status" >"$log.status.json"
    return "$status"
  fi
  if [ "$inject" = export-fail-p2 ]; then
    if bash "$helper_path" injected-export-failure "$destination" >>"$log" 2>&1; then status=0; else status=$?; fi
    printf '{"buildx_exit":0,"helper_exit":%s}\n' "$status" >"$log.status.json"
    return "$status"
  fi
  printf '{"buildx_exit":0,"helper_exit":null}\n' >"$log.status.json"
  return 0
}

write_override() {
  local path=$1 service=$2 route=$3
  PROBE_PATH="$path" PROBE_SERVICE="$service" PROBE_ROUTE="$route" python3 - <<'PY'
import json,os
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8",newline="\n") as o:
  o.write("services:\n  "+os.environ["PROBE_SERVICE"]+":\n    build:\n      dockerfile: Dockerfile.consumer\n      target: runtime\n")
  if os.environ["PROBE_ROUTE"]!="source":
    o.write("      args:\n        RUST_ARTIFACT_SOURCE: verified-artifacts\n      additional_contexts:\n        release_artifacts: "+json.dumps(os.environ["PROBE_ROUTE"])+"\n")
PY
  chmod 0600 -- "$path"
}

compose_build() {
  local phase=$1 service=$2 tag=$3 route=$4 log=$5 suffix override=() no_cache=() status
  case "$phase" in p1) suffix=A_BASE ;; p2) suffix=B_BASE ;; p3) suffix=A_WIDE ;; p4) suffix=B_WIDE ;; esac
  if [ "$route" != baseline ]; then
    local file="$CUR_ATTEMPT_DIR/compose-$phase-$(printf '%s' "$route" | sha256sum | awk '{print $1}').yml"
    write_override "$file" "$service" "$route" || return 1
    override=(-f "$file")
  fi
  [ "$CUR_COLD" = 1 ] && [ "$phase" = p1 ] && no_cache=(--no-cache)
  local inject=none
  if [ "$CUR_CASE/$phase" = export-fail-p2/p2 ] && first_trial_injection; then inject=export-fail-p2; fi
  if [ "$CUR_LAYOUT" = baseline ] && [ "$phase" = p2 ] && first_trial_injection; then
    case "$CUR_CASE" in wrong-commit|missing-bin|tampered-bin|missing-complete) inject=$CUR_CASE ;; esac
  fi
  local envs=(
    "PROBE_IMAGE_${suffix}=$tag" "PROBE_DOCKERFILE=$([ "$CUR_LAYOUT" = baseline ] && printf Dockerfile.baseline || printf Dockerfile.consumer)" "PROBE_BUILD_TARGET=runtime"
    "PROBE_COMMIT_${suffix}=$CUR_COMMIT" "PROBE_SETTING_${suffix}=$CUR_SETTING" "PROBE_TOKEN_${suffix}=$CUR_TOKEN"
    "PROBE_SHARED_CACHE_ID_${suffix}=$CUR_SHARED_CACHE" "PROBE_TARGET_CACHE_ID_${suffix}=$CUR_TARGET_CACHE" "PROBE_CACHE_KEY_${suffix}=$CUR_K"
    "PROBE_INPUT_${suffix}=$CUR_H" "PROBE_COMPILE_INPUT_${suffix}=$CUR_COMPILE" "PROBE_LIB_${suffix}=$CUR_LIB" "PROBE_APP_${suffix}=$CUR_APP"
    "PROBE_RECIPE_${suffix}=$CUR_RECIPE" "PROBE_PLATFORM_${suffix}=linux/amd64" "PROBE_HOST_${suffix}=$CUR_HOST" "PROBE_NATIVE_${suffix}=$CUR_IDENTITY_SHA" "PROBE_CARGO_CONFIG_${suffix}=$CUR_CONFIG"
    "PROBE_FEATURES_${suffix}=$(requested_features "${p_variant[$phase]}")" "PROBE_SOURCE_TRAP_${suffix}=$CUR_SOURCE_TRAP" "PROBE_LEDGER_INJECT_${suffix}=$CUR_LEDGER_INJECT" "PROBE_INJECT_${suffix}=$inject"
  )
  if env CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1 "${envs[@]}" "$docker_bin" compose -p "build-layout-$run_id-$CUR_LAYOUT-$CUR_SCOPE" -f "$CUR_SOURCE/compose.yml" "${override[@]}" build --progress plain "${no_cache[@]}" "$service" >"$log" 2>&1; then status=0; else status=$?; fi
  printf '{"compose_exit":%s}\n' "$status" >"$log.status.json"
  return "$status"
}

verify_export_failure() {
  local phase=$1 status=$2 log=$3 private=${4:-} summary raw partial_hash=none
  first_trial_injection && [ "$phase" = p2 ] && [ "$status" -ne 0 ] || return 1
  summary="$CUR_ATTEMPT_DIR/results/$phase-export-cargo-summary.json"
  raw="$summary.cargo.jsonl"
  if [ "$CUR_LAYOUT" = baseline ]; then
    python3 - "$log" "$status" "$raw" "$(service_for "$phase")" <<'PY'
import json,re,sys
log,status,output,service=sys.argv[1],int(sys.argv[2]),sys.argv[3],sys.argv[4]
if json.load(open(log+".status.json"))!={"compose_exit":status}:raise SystemExit("export-client-status-mismatch")
lines=open(log,encoding="utf-8").read().splitlines()
marker="BUILD_LAYOUT_INJECTED_EXPORT_FAILURE bin=cache-bin-b inner_exit=73 cargo_success=true"
observed=[m for line in lines if (m:=re.fullmatch(r"(#[0-9]+) [0-9]+(?:\.[0-9]+)? "+re.escape(marker),line))]
if len(observed)!=1:raise SystemExit("export-injected-marker-missing-or-ambiguous")
vertex=observed[0][1]
if sum(bool(re.match(re.escape(vertex)+r" \[(?:"+re.escape(service)+r" )?builder [^]]+\] RUN --mount=type=cache,target=/usr/local/cargo/registry",line)) for line in lines)!=1:
    raise SystemExit("export-compile-vertex-unproven")
errors=[line for line in lines if re.match(r"#[0-9]+ ERROR:",line)]
if len(errors)!=1 or not re.fullmatch(re.escape(vertex)+r' ERROR: process ".*" did not complete successfully: exit code: 73',errors[0]):
    raise SystemExit("export-inner-exit-unproven")
# Only executed output from this exact RUN is Cargo evidence; source excerpts
# and the client's quoted failed command cannot masquerade as compiler output.
stream=[m[1]+"\n" for line in lines if (m:=re.match(re.escape(vertex)+r" [0-9]+(?:\.[0-9]+)? (.*)$",line))]
with open(output,"x",encoding="utf-8") as out:out.writelines(stream)
PY
    [ "$?" -eq 0 ] || return 1
  else
    [ "$status" -eq 73 ] || return 1
    python3 - "$log" <<'PY'
import json,sys
log=sys.argv[1]
if json.load(open(log+".status.json"))!={"buildx_exit":0,"helper_exit":73}:raise SystemExit("export-helper-boundary-unproven")
if open(log,encoding="utf-8").read().splitlines().count("build-layout-helper: injected export failure status=73")!=1:raise SystemExit("export-helper-marker-unproven")
PY
    [ "$?" -eq 0 ] || return 1
    local check
    if bash "$helper_path" injected-export-failure "$private" >"$CUR_ATTEMPT_DIR/results/$phase-export-marker.log" 2>&1; then return 1; else check=$?; fi
    [ "$check" -eq 73 ] || return 1
    raw="$private/cargo.jsonl"
    partial_hash=$(bash "$helper_path" tree-hash "$private") || return 1
  fi
  bash "$helper_path" cargo-summary "$raw" "${p_bin[$phase]}" "$summary" "$CUR_HOST" || return 1
  if [ "$CUR_LAYOUT" != baseline ]; then
    cmp -s -- "$summary" "$private/cargo-summary.json" || { unresolved 'incomplete export Cargo evidence differs'; return 1; }
  fi
  assert_cargo_observation "$summary" "$phase" "$log" "$CUR_ATTEMPT_DIR/results/$phase-export-cargo-observation.json" || return 1
  python3 - "$CUR_LAYOUT" "$status" "$log" "$summary" "$partial_hash" "$CUR_ATTEMPT_DIR/results/$phase-export-failure.json" <<'PY'
import hashlib,json,sys
layout,status,log,summary,partial,output=sys.argv[1:]
record={"boundary":"baseline-run" if layout=="baseline" else "host-helper","outer_status":int(status),"inner_status":73,"cargo_success":True,"command_status":json.load(open(log+".status.json")),"build_log_sha256":hashlib.sha256(open(log,"rb").read()).hexdigest(),"cargo_summary_sha256":hashlib.sha256(open(summary,"rb").read()).hexdigest(),"partial_tree_sha256":partial}
with open(output,"x",encoding="utf-8") as out:out.write(json.dumps(record,sort_keys=True,separators=(",",":"))+"\n")
PY
}

image_absent() {
  local tag=$1 evidence=$2 status
  mkdir -p -m 0700 -- "$evidence"
  if LC_ALL=C "$docker_bin" image inspect "$tag" >"$evidence/stdout" 2>"$evidence/stderr"; then status=0; else status=$?; fi
  python3 - "$tag" "$status" "$evidence" <<'PY'
import hashlib,json,pathlib,sys
tag,status,directory=sys.argv[1],int(sys.argv[2]),pathlib.Path(sys.argv[3])
out=(directory/"stdout").read_bytes();err=(directory/"stderr").read_bytes()
# Docker's single-image inspect emits [] and this exact diagnostic for a miss.
# A daemon/transport/permission failure is never absence, even with exit 1.
missing=status==1 and out in (b"",b"[]\n") and err==f"Error response from daemon: No such image: {tag}\n".encode()
cause="missing-image" if missing else ("image-present" if status==0 else "inspection-failed")
record={"command_exit":status,"cause":cause,"stderr_sha256":hashlib.sha256(err).hexdigest(),"stdout_sha256":hashlib.sha256(out).hexdigest(),"tag":tag}
(directory/"result.json").write_text(json.dumps(record,sort_keys=True,separators=(",",":"))+"\n")
if not missing:raise SystemExit("image-absence-unproven: "+cause)
PY
}

verify_artifact() {
  local artifact=$1 phase=$2
  bash "$helper_path" artifact-verify "$artifact" "$CUR_COMMIT" build-cache-fixture-app "${p_bin[$phase]}" linux/amd64 "$CUR_HOST" release "$(requested_features "${p_variant[$phase]}")" "$(compile_env "$CUR_SETTING" "$CUR_COMMIT")" "$CUR_K" "$CUR_H" "$CUR_RECIPE"
}

h_oracle() {
  local layout=$1 case_id=$2 phase=$3 token=$4 resuming=${5:-0}
  local mode=executed app=false lib=false itoa=true initial=0
  if [ "$case_id/$token" = exact-repeat/cold ]; then
    # The identical Docker vertex may be present; if evicted, Cargo must still
    # prove the specified app/lib/itoa units Fresh.
    printf '%s\n' 'cache-hit-or-fresh,true,true,true'
    return 0
  fi
  case "$token" in cold|*-setup|empty-cache|route-artifact) initial=1 ;; esac
  if [ "$case_id/$token" = route-contract/route-source ]; then
    printf '%s\n' 'executed,false,false,false'
    return 0
  fi
  case "$case_id" in
    manifest-key|lock-key|config-key) [ "$token" != *-setup ] && initial=1 ;;
    broken-ledger) [ "$layout" != baseline ] && [ "$token" != *-setup ] && initial=1 ;;
  esac
  if [ "$layout" = baseline ]; then
    [ "$initial" = 1 ] && [ "$phase" = p1 ] && itoa=false
  elif [ "$resuming" = 1 ]; then
    case "$case_id/$phase" in
      compile-fail-p2/p2|export-fail-p2/p2) app=false; lib=false; itoa=false ;;
      compile-fail-p2/p3|export-fail-p2/p3)
        if [ "$layout" = common ]; then app=false; lib=false; itoa=true
        else app=true; lib=true; itoa=true; fi ;;
      compile-fail-p2/p4|export-fail-p2/p4)
        if [ "$layout" = common ]; then app=false; lib=true; itoa=true
        else app=true; lib=true; itoa=true; fi ;;
      wrong-commit/p2|missing-bin/p2|tampered-bin/p2|missing-complete/p2)
        mode=cache-hit; app=-; lib=-; itoa=- ;;
      *) app=true; lib=true; itoa=true ;;
    esac
  elif [ "$initial" = 1 ]; then
    case "$phase" in
      p1) app=false; lib=false; itoa=false ;;
      p2) app=false; lib=true; itoa=true ;;
      p3) app=false; lib=false; [ "$layout" = grouped ] && itoa=false || itoa=true ;;
      p4) app=false; lib=true; itoa=true ;;
    esac
  else
    case "$case_id" in
      commit-only|web-only|python-only|build-setting)
        app=false; lib=true; itoa=true ;;
      app-source-old-mtime|embedded|build-script|branch-return)
        app=false; lib=true; itoa=true ;;
      lib-source)
        app=false; itoa=true; case "$phase" in p1|p3) lib=false ;; *) lib=true ;; esac ;;
      *) app=true; lib=true; itoa=true ;;
    esac
  fi
  printf '%s,%s,%s,%s\n' "$mode" "$app" "$lib" "$itoa"
}

assert_cargo_observation() {
  local summary=$1 phase=$2 build_log=$3 observation=$4 expected mode app lib itoa
  expected=$(h_oracle "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_TOKEN" "${RESUMING:-0}")
  IFS=, read -r mode app lib itoa <<<"$expected"
  local actual_mode; actual_mode=$(bash "$helper_path" build-log-mode "$build_log") || return 1
  if [ "$mode" = cache-hit-or-fresh ]; then
    case "$actual_mode" in cache-hit) ;; executed) mode=executed ;; *) unresolved "unexpected-recompile-set: expected-mode=cache-hit-or-fresh actual-mode=$actual_mode"; return 1 ;; esac
  else
    [ "$actual_mode" = "$mode" ] || { unresolved "unexpected-recompile-set: expected-mode=$mode actual-mode=$actual_mode"; return 1; }
  fi
  if [ "$actual_mode" = executed ]; then
    bash "$helper_path" cargo-assert "$summary" "${p_bin[$phase]}" "$(requested_features "${p_variant[$phase]}")" "$(resolved_features "${p_variant[$phase]}")" "$CUR_HOST" "$app" "$lib" "$itoa" || { unresolved 'unexpected-recompile-set'; return 1; }
  fi
  PROBE_PATH="$observation" PROBE_MODE="$actual_mode" PROBE_EXPECTED="$expected" PROBE_BUILD_LOG="$build_log" PROBE_SUMMARY="$summary" python3 - <<'PY'
import hashlib,json,os
def digest(path):
    with open(path,"rb") as handle:return hashlib.sha256(handle.read()).hexdigest()
value={"build_log_sha256":digest(os.environ["PROBE_BUILD_LOG"]),"expected_h":os.environ["PROBE_EXPECTED"],"mode":os.environ["PROBE_MODE"],"summary_sha256":digest(os.environ["PROBE_SUMMARY"]) if os.path.isfile(os.environ["PROBE_SUMMARY"]) else None}
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8",newline="\n") as out:out.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
}

mutate_artifact() {
  local kind=$1 artifact=$2 bin=$3
  case "$kind" in
    wrong-commit) sed -i "s/$CUR_COMMIT/$c3/g" "$artifact/artifact.json" ;;
    missing-bin) rm -f -- "$artifact/bin/$bin" ;;
    tampered-bin) printf X >>"$artifact/bin/$bin" ;;
    missing-complete) rm -f -- "$artifact/COMPLETE" ;;
  esac
}

publish_checked() {
  local phase=$1 private=$2 kind=none bin verify_status publish_status published
  bin=${p_bin[$phase]}
  if [ "$phase" = p2 ] && first_trial_injection; then
    case "$CUR_CASE" in wrong-commit|missing-bin|tampered-bin|missing-complete) kind=$CUR_CASE ;; esac
  fi
  [ "$kind" = none ] || mutate_artifact "$kind" "$private" "$bin"
  if verify_artifact "$private" "$phase"; then verify_status=0; else verify_status=$?; fi
  if [ "$kind" != none ]; then
    [ "$verify_status" -ne 0 ] || { unresolved "artifact mutation was accepted: $kind"; return 1; }
    printf 'expected-artifact-%s\n' "$kind"
    return 75
  fi
  [ "$verify_status" -eq 0 ] || return "$verify_status"
  if published=$(bash "$helper_path" artifact-publish "$private" "$run_dir/artifacts/$CUR_LAYOUT/$CUR_SCOPE"); then publish_status=0; else publish_status=$?; fi
  [ "$publish_status" -eq 0 ] || return "$publish_status"
  [ -d "$published" ] && [ ! -L "$published" ] || return 1
  printf '%s\n' "$published"
}

verify_image() {
  local phase=$1 tag=$2 result_dir=$3 bin runtime status image_id revision binary
  bin=${p_bin[$phase]} runtime=${p_runtime[$phase]}
  mkdir -p -m 0700 -- "$result_dir"
  expected_stdout "$CUR_SOURCE" "$bin" "${p_variant[$phase]}" "$CUR_COMMIT" "$CUR_SETTING" >"$result_dir/expected.stdout" || return 1
  cp -- "$CUR_SOURCE/runtime/$runtime" "$result_dir/expected.runtime"
  if "$docker_bin" image inspect --format '{{.Id}}{{printf "\t"}}{{index .Config.Labels "org.opencontainers.image.revision"}}' "$tag" >"$result_dir/inspect.out" 2>"$result_dir/inspect.err"; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || return "$status"
  [ ! -s "$result_dir/inspect.err" ] || return 1
  IFS=$'\t' read -r image_id revision <"$result_dir/inspect.out" || return 1
  [ "$(wc -l <"$result_dir/inspect.out")" -eq 1 ] && [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] && [ "$revision" = "$CUR_COMMIT" ] || return 1
  if timeout 30s "$docker_bin" run --rm --network none --read-only --cap-drop=ALL --security-opt no-new-privileges=true --user 10001:10001 "$tag" >"$result_dir/actual.stdout" 2>"$result_dir/run.err"; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || return "$status"
  [ ! -s "$result_dir/run.err" ] && cmp -s -- "$result_dir/expected.stdout" "$result_dir/actual.stdout" || return 1
  if timeout 30s "$docker_bin" run --rm --network none --read-only --cap-drop=ALL --security-opt no-new-privileges=true --user 10001:10001 --entrypoint /bin/cat "$tag" /opt/fixture/runtime.txt >"$result_dir/actual.runtime" 2>"$result_dir/runtime.err"; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || return "$status"
  [ ! -s "$result_dir/runtime.err" ] && cmp -s -- "$result_dir/expected.runtime" "$result_dir/actual.runtime" || return 1
  if timeout 30s "$docker_bin" run --rm --network none --read-only --cap-drop=ALL --security-opt no-new-privileges=true --user 10001:10001 --entrypoint /usr/bin/sha256sum "$tag" "/usr/local/bin/$bin" >"$result_dir/binary.sha256" 2>"$result_dir/binary.err"; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || return "$status"
  [ ! -s "$result_dir/binary.err" ] || return 1
  binary=$(python3 - "$result_dir/binary.sha256" "/usr/local/bin/$bin" <<'PY'
import re,sys
data=open(sys.argv[1],"rb").read()
match=re.fullmatch(rb"([0-9a-f]{64})  "+re.escape(sys.argv[2].encode())+rb"\n",data)
if not match:raise SystemExit("binary-sha-output-invalid")
print(match.group(1).decode())
PY
) || return 1
  PROBE_PATH="$result_dir/result.json" PROBE_ID="$image_id" PROBE_REVISION="$revision" PROBE_BINARY="$binary" PROBE_STDOUT="$result_dir/actual.stdout" PROBE_RUNTIME="$result_dir/actual.runtime" python3 - <<'PY'
import hashlib,json,os
def read(path):
    with open(path,"rb") as handle:return handle.read()
stdout=read(os.environ["PROBE_STDOUT"]); runtime=read(os.environ["PROBE_RUNTIME"])
value={"binary_sha256":os.environ["PROBE_BINARY"],"binary_stdout":stdout.decode("utf-8"),"image_id":os.environ["PROBE_ID"],"oci_revision":os.environ["PROBE_REVISION"],"runtime_bytes":runtime.decode("utf-8"),"runtime_sha256":hashlib.sha256(runtime).hexdigest(),"stdout_sha256":hashlib.sha256(stdout).hexdigest()}
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8",newline="\n") as out:out.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
}

write_identity() {
  local identity=$1 path; path=$(state_identity "$CUR_LAYOUT" "$CUR_SCOPE" "$CUR_TOKEN")
  mkdir -p -m 0700 -- "$(dirname "$path")"; [ ! -e "$path" ] || { unresolved 'identity state overwrite'; return 1; }
  local host; host=$(bash "$helper_path" identity-validate "$identity" linux/amd64) || return 1
  PROBE_PATH="$path" PROBE_IDENTITY="$identity" PROBE_SHA="$(sha256sum -- "$identity" | awk '{print $1}')" PROBE_HOST="$host" PROBE_SOURCE="$CUR_SOURCE" PROBE_SOURCE_SHA="$(bash "$helper_path" source-input-hash "$CUR_SOURCE")" PROBE_RECIPE="$(recipe_hash "$CUR_LAYOUT" "$CUR_SOURCE")" PROBE_TOOL="$tool_sha" PROBE_FIXTURE="$fixture_sha" python3 - <<'PY'
import json,os
value={"fixture_sha256":os.environ["PROBE_FIXTURE"],"host_triple":os.environ["PROBE_HOST"],"identity_json":os.environ["PROBE_IDENTITY"],"identity_sha256":os.environ["PROBE_SHA"],"recipe_sha256":os.environ["PROBE_RECIPE"],"source_path":os.environ["PROBE_SOURCE"],"source_sha256":os.environ["PROBE_SOURCE_SHA"],"tool_sha256":os.environ["PROBE_TOOL"]}
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8",newline="\n") as o:o.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
}

load_identity() {
  local token=$1 path; path=$(state_identity "$CUR_LAYOUT" "$CUR_SCOPE" "$token")
  [ -f "$path" ] && [ ! -L "$path" ] || { unresolved 'prior P0 identity is missing'; return 1; }
  local native expected source source_expected recipe stored_tool stored_fixture actual serialized
  serialized=$(python3 - "$path" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));print("\t".join(v[k] for k in ("identity_json","identity_sha256","source_path","source_sha256","recipe_sha256","tool_sha256","fixture_sha256")))
PY
) || return 1
  IFS=$'\t' read -r native expected source source_expected recipe stored_tool stored_fixture <<<"$serialized"
  [ -f "$native" ] && [ ! -L "$native" ] || { unresolved 'prior identity output missing'; return 1; }
  actual=$(sha256sum -- "$native" | awk '{print $1}')
  [ "$actual" = "$expected" ] || { unresolved 'prior identity output changed'; return 1; }
  [ -d "$source" ] && [ ! -L "$source" ] && [ "$(bash "$helper_path" source-input-hash "$source")" = "$source_expected" ] || { unresolved 'prior P0 source changed'; return 1; }
  [ "$(recipe_hash "$CUR_LAYOUT" "$source")" = "$recipe" ] && [ "$stored_tool" = "$tool_sha" ] && [ "$stored_fixture" = "$fixture_sha" ] || { unresolved 'prior P0 recipe identity changed'; return 1; }
  bash "$helper_path" identity-validate "$native" linux/amd64 >/dev/null || return 1
  printf '%s\n' "$native"
}

write_phase() {
  local phase=$1 tag=$2 artifact=${3:-} state_token=${4:-$CUR_TOKEN} path result observation artifact_hash=none observation_hash result_hash source_snapshot source_hash
  path=$(state_phase "$CUR_LAYOUT" "$CUR_SCOPE" "$state_token" "$phase")
  result="$CUR_ATTEMPT_DIR/results/$phase/result.json"
  observation="$CUR_ATTEMPT_DIR/results/$phase-cargo-observation.json"
  mkdir -p -m 0700 -- "$(dirname "$path")"
  [ ! -e "$path" ] && [ -f "$result" ] && [ ! -L "$result" ] && [ -f "$observation" ] && [ ! -L "$observation" ] || { unresolved "phase state overwrite or missing result: $phase"; return 1; }
  result_hash=$(sha256sum -- "$result" | awk '{print $1}'); observation_hash=$(sha256sum -- "$observation" | awk '{print $1}')
  source_snapshot="$(dirname "$path")/source-$phase"
  [ ! -e "$source_snapshot" ] || { unresolved "phase source snapshot overwrite: $phase"; return 1; }
  mkdir -m 0700 -- "$source_snapshot"; cp -a -- "$CUR_SOURCE/." "$source_snapshot/"
  source_hash=$(bash "$helper_path" source-input-hash "$source_snapshot") || return 1
  if [ -n "$artifact" ]; then artifact_hash=$(bash "$helper_path" tree-hash "$artifact") || return 1; [ "$(basename "$artifact")" = "$artifact_hash" ] || { unresolved 'published artifact content name mismatch'; return 1; }; fi
  PROBE_PATH="$path" PROBE_RESULT="$result" PROBE_RESULT_SHA="$result_hash" PROBE_OBSERVATION="$observation" PROBE_OBSERVATION_SHA="$observation_hash" PROBE_ARTIFACT="$artifact" PROBE_ARTIFACT_SHA="$artifact_hash" PROBE_TAG="$tag" PROBE_SOURCE_PATH="$source_snapshot" PROBE_SOURCE="$source_hash" PROBE_TOOL="$tool_sha" PROBE_FIXTURE="$fixture_sha" PROBE_K="$CUR_K" PROBE_H="$CUR_H" PROBE_RECIPE="$CUR_RECIPE" PROBE_COMPILE="$CUR_COMPILE" PROBE_NS="$CUR_TARGET_CACHE" PROBE_TOKEN="$CUR_TOKEN" PROBE_LAYOUT="$CUR_LAYOUT" PROBE_CASE="$CUR_CASE" PROBE_PHASE="$phase" PROBE_COMMIT="$CUR_COMMIT" PROBE_SETTING="$CUR_SETTING" PROBE_CONFIG="$CUR_CONFIG" PROBE_HOST="$CUR_HOST" PROBE_NATIVE="$CUR_IDENTITY_SHA" PROBE_BIN="${p_bin[$phase]}" PROBE_VARIANT="${p_variant[$phase]}" PROBE_RUNTIME="${p_runtime[$phase]}" PROBE_REQUESTED="$(requested_features "${p_variant[$phase]}")" PROBE_RESOLVED="$(resolved_features "${p_variant[$phase]}")" python3 - <<'PY'
import json,os
v={"artifact":os.environ["PROBE_ARTIFACT"],"artifact_content_sha256":os.environ["PROBE_ARTIFACT_SHA"],"bin":os.environ["PROBE_BIN"],"case":os.environ["PROBE_CASE"],"commit":os.environ["PROBE_COMMIT"],"compile_input_sha256":os.environ["PROBE_COMPILE"],"config":os.environ["PROBE_CONFIG"],"fixture_sha256":os.environ["PROBE_FIXTURE"],"host_triple":os.environ["PROBE_HOST"],"identity_sha256":os.environ["PROBE_NATIVE"],"input_sha256":os.environ["PROBE_H"],"layout":os.environ["PROBE_LAYOUT"],"observation_path":os.environ["PROBE_OBSERVATION"],"observation_sha256":os.environ["PROBE_OBSERVATION_SHA"],"phase":os.environ["PROBE_PHASE"],"recipe_sha256":os.environ["PROBE_RECIPE"],"requested_features":json.loads(os.environ["PROBE_REQUESTED"]),"resolved_features":json.loads(os.environ["PROBE_RESOLVED"]),"result":json.load(open(os.environ["PROBE_RESULT"])),"result_path":os.environ["PROBE_RESULT"],"result_sha256":os.environ["PROBE_RESULT_SHA"],"runtime":os.environ["PROBE_RUNTIME"],"setting":os.environ["PROBE_SETTING"],"source_path":os.environ["PROBE_SOURCE_PATH"],"source_sha256":os.environ["PROBE_SOURCE"],"tag":os.environ["PROBE_TAG"],"target_namespace":os.environ["PROBE_NS"],"token":os.environ["PROBE_TOKEN"],"tool_sha256":os.environ["PROBE_TOOL"],"variant":os.environ["PROBE_VARIANT"],"compatibility_key":os.environ["PROBE_K"]}
with open(os.environ["PROBE_PATH"],"x",encoding="utf-8") as o:o.write(json.dumps(v,sort_keys=True,separators=(",",":"))+"\n")
PY
}

tag_from_ledger() {
  local phase=$1 token=$2
  python3 - "$events_file" "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$token" <<'PY'
import json,sys
tag=None
for line in open(sys.argv[1]):
 v=json.loads(line)
 if v.get("type")=="VERIFY" and v.get("status")=="PASS" and v.get("layout")==sys.argv[2] and v.get("case")==sys.argv[3] and v.get("phase")==sys.argv[4] and v.get("token")==sys.argv[5]:
  d=v.get("detail","")
  if d.startswith("tag="):tag=d[4:]
if not tag:raise SystemExit(1)
print(tag)
PY
}

validate_prior_phase() {
  local phase=$1 source=$2 path tag artifact stored result_path result_sha observation_path observation_sha artifact_sha serialized
  path=$(state_phase "$CUR_LAYOUT" "$CUR_SCOPE" "$CUR_TOKEN" "$phase")
  [ -f "$path" ] && [ ! -L "$path" ] || { unresolved "resume phase state missing: $phase"; return 1; }
  serialized=$(python3 - "$path" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));print("\t".join((v["source_sha256"],v["artifact"] or "-",v["tag"],v["result_path"],v["result_sha256"],v["observation_path"],v["observation_sha256"],v["artifact_content_sha256"])))
PY
) || return 1
  IFS=$'\t' read -r stored artifact tag result_path result_sha observation_path observation_sha artifact_sha <<<"$serialized"
  [ "$stored" = "$(bash "$helper_path" source-input-hash "$source")" ] || { unresolved 'resume source inventory changed'; return 1; }
  python3 - "$path" "$tool_sha" "$fixture_sha" "$CUR_K" "$CUR_H" "$CUR_RECIPE" "$CUR_COMPILE" "$CUR_COMMIT" "$CUR_SETTING" "$CUR_HOST" "$CUR_IDENTITY_SHA" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]))
keys=("tool_sha256","fixture_sha256","compatibility_key","input_sha256","recipe_sha256","compile_input_sha256","commit","setting","host_triple","identity_sha256")
if any(v[k]!=x for k,x in zip(keys,sys.argv[2:])):raise SystemExit(1)
PY
  [ -f "$result_path" ] && [ ! -L "$result_path" ] && [ "$(sha256sum -- "$result_path" | awk '{print $1}')" = "$result_sha" ] || { unresolved 'resume result evidence changed'; return 1; }
  [ -f "$observation_path" ] && [ ! -L "$observation_path" ] && [ "$(sha256sum -- "$observation_path" | awk '{print $1}')" = "$observation_sha" ] || { unresolved 'resume Cargo observation changed'; return 1; }
  local saved=$CUR_ATTEMPT_DIR
  CUR_ATTEMPT_DIR="$CUR_ATTEMPT_DIR/reused-$phase"
  verify_image "$phase" "$tag" "$CUR_ATTEMPT_DIR/result" || { CUR_ATTEMPT_DIR=$saved; unresolved 'resume image/revision/binary failed'; return 1; }
  CUR_ATTEMPT_DIR=$saved
  if [ "$CUR_LAYOUT" != baseline ]; then
    [ "$artifact" != - ] && [ "$(bash "$helper_path" tree-hash "$artifact")" = "$artifact_sha" ] && [ "$(basename "$artifact")" = "$artifact_sha" ] && verify_artifact "$artifact" "$phase" || { unresolved 'resume receipt failed'; return 1; }
  fi
}

save_failure_source() {
  local dst="$CUR_ATTEMPT_DIR/failure-source"
  [ ! -e "$dst" ] || return 1
  mkdir -p -m 0700 -- "$dst"; cp -a -- "$CUR_SOURCE/." "$dst/"
  bash "$helper_path" source-input-hash "$dst"
}

expected_stop() {
  local phase=$1 code=$2 reason=$3 tag=$4 later hash
  emit_event EXPECTED_CAUSE "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" VERIFIED "$code" "$reason;gate_previous_exit=0"
  # G1 §5.7: only the verified injected command is normalized for the gate.
  # Its real exit remains in EXPECTED_CAUSE; resource/health failures stay fatal.
  if ! probe_gate "$CUR_CASE" "$phase-after-expected-failure" 0; then
    emit_event GATE "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" FAIL 1 "post-failure-gate;original_exit=$code"
    return 1
  fi
  if [ "$reason" = stop-after-p2 ]; then
    "$docker_bin" image inspect "$tag" >/dev/null 2>&1 || { unresolved 'operator stop lost the completed P2 image'; return 1; }
    [ -f "$(state_phase "$CUR_LAYOUT" "$CUR_SCOPE" "$CUR_TOKEN" p2)" ] || { unresolved 'operator stop lost the completed P2 state'; return 1; }
  else
    image_absent "$tag" "$CUR_ATTEMPT_DIR/results/$phase-absence" || { unresolved 'failure image absence is unproven'; return 1; }
  fi
  for later in p3 p4; do [ ! -e "$(state_phase "$CUR_LAYOUT" "$CUR_SCOPE" "$CUR_TOKEN" "$later")" ] || { unresolved 'failure path reached later phase'; return 1; }; done
  hash=$(save_failure_source) || return 1
  emit_event EXPECTED_STOP "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" EXPECTED_STOP "$code" "$reason;failure_source_sha256=$hash;injection_consumed=true"
  return 75
}

inject_phase() {
  local phase=$1
  CUR_LEDGER_INJECT=none
  if [ "$CUR_CASE" = broken-ledger ] && [ "$CUR_LAYOUT" != baseline ] && first_trial_injection; then
    [ "$CUR_LAYOUT/$phase" = common/p1 ] && CUR_LEDGER_INJECT=unknown-pending
    [ "$CUR_LAYOUT" = grouped ] && { [ "$phase" = p1 ] || [ "$phase" = p3 ]; } && CUR_LEDGER_INJECT=unknown-pending
  fi
  if [ "$CUR_CASE/$phase" = compile-fail-p2/p2 ] && first_trial_injection; then printf '%s\n' 'compile_error!("layout-probe-injected");' >>"$CUR_SOURCE/fixture-app/src/bin/cache-bin-b.rs"; fi
}

run_phase() {
  local phase=$1 identity=$2 skip=${3:-0} bin tag status private artifact started ended cargo summary build_log observation build_mode exported_cargo repeat_state repeat_result
  bin=${p_bin[$phase]}
  CUR_PHASE=$phase
  inject_phase "$phase"
  compute_inputs "$identity" || return 1
  tag="build-layout-$run_id-$CUR_LAYOUT-$CUR_SCOPE-$CUR_TOKEN-$phase"
  if [ "$skip" = 1 ]; then
    validate_prior_phase "$phase" "$CUR_SOURCE" || return 1
    emit_event RESUME_REUSE "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" REUSED 0 verified-prior-phase
    return 0
  fi
  if [ "$skip" = 2 ]; then
    repeat_state=$(state_phase "$CUR_LAYOUT" "$CUR_SCOPE" cold "$phase")
    validate_prior_phase "$phase" "$CUR_SOURCE" || return 1
    repeat_result=$(python3 - "$repeat_state" <<'PY'
import json,sys
print(json.load(open(sys.argv[1],encoding="utf-8"))["result_path"])
PY
) || return 1
  fi
  probe_gate "$CUR_CASE" "$phase-before" 0 || return 1
  started=$(now_ms)
  emit_event PHASE "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" START 0 "K=$CUR_K;H=$CUR_H;bin=$bin"
  if [ "$CUR_LAYOUT" = baseline ]; then
    CUR_SOURCE_TRAP=0
    build_log="$CUR_ATTEMPT_DIR/logs/$phase.log"
    if compose_build "$phase" "$(service_for "$phase")" "$tag" baseline "$build_log"; then status=0; else status=$?; fi
    if [ "$status" -ne 0 ]; then
      case "$CUR_CASE/$phase" in
       compile-fail-p2/p2) grep -Fq layout-probe-injected "$CUR_ATTEMPT_DIR/logs/$phase.log" || return 1; expected_stop "$phase" "$status" compile-fail-p2-cargo-nonzero "$tag"; return $? ;;
       export-fail-p2/p2) verify_export_failure "$phase" "$status" "$build_log" || return 1; expected_stop "$phase" "$status" export-fail-p2-after-cargo "$tag"; return $? ;;
       wrong-commit/p2|missing-bin/p2|tampered-bin/p2|missing-complete/p2) expected_stop "$phase" "$status" "baseline-stage-$CUR_CASE-rejected" "$tag"; return $? ;;
       *) emit_event BUILD "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" FAIL "$status" baseline-build-failed; return 1 ;;
      esac
    fi
    summary="$CUR_ATTEMPT_DIR/results/$phase-cargo-summary.json"
    build_mode=$(bash "$helper_path" build-log-mode "$build_log") || return 1
    if [ "$build_mode" = executed ]; then bash "$helper_path" cargo-summary "$build_log" "$bin" "$summary" "$CUR_HOST" || return 1
    else printf '{"cargo_success":true,"mode":"cache-hit"}\n' >"$summary"; fi
    observation="$CUR_ATTEMPT_DIR/results/$phase-cargo-observation.json"
    assert_cargo_observation "$summary" "$phase" "$build_log" "$observation" || return 1
    artifact=
    if [ "$build_mode" = executed ]; then cargo=$(bash "$helper_path" cargo-elapsed-ms "$build_log") || return 1; else cargo=0; fi
  else
    private="$CUR_ATTEMPT_DIR/private-$phase"
    build_log="$CUR_ATTEMPT_DIR/logs/$phase-producer.log"
    if artifact_build "$phase" "$private" "$build_log"; then status=0; else status=$?; fi
    if [ "$status" -ne 0 ]; then
      case "$CUR_CASE/$phase" in
       compile-fail-p2/p2) grep -Fq layout-probe-injected "$CUR_ATTEMPT_DIR/logs/$phase-producer.log" || return 1; expected_stop "$phase" "$status" compile-fail-p2-cargo-nonzero "$tag"; return $? ;;
       export-fail-p2/p2) verify_export_failure "$phase" "$status" "$build_log" "$private" || return 1; expected_stop "$phase" "$status" export-fail-p2-after-cargo "$tag"; return $? ;;
       *) emit_event BUILD "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" FAIL "$status" producer-build-failed; return 1 ;;
      esac
    fi
    summary="$private/cargo-summary.json"
    observation="$CUR_ATTEMPT_DIR/results/$phase-cargo-observation.json"
    assert_cargo_observation "$summary" "$phase" "$build_log" "$observation" || return 1
    if artifact=$(publish_checked "$phase" "$private"); then status=0; else status=$?; fi
    if [ "$status" -eq 75 ]; then expected_stop "$phase" 1 "$artifact" "$tag"; return $?; fi
    [ "$status" -eq 0 ] || return 1
    CUR_SOURCE_TRAP=1
    if compose_build "$phase" "$(service_for "$phase")" "$tag" "$artifact" "$CUR_ATTEMPT_DIR/logs/$phase-consumer.log"; then status=0; else status=$?; fi
    if [ "$status" -ne 0 ]; then emit_event BUILD "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" FAIL "$status" consumer-build-failed; return 1; fi
    build_mode=$(bash "$helper_path" build-log-mode "$build_log") || return 1
    if [ "$build_mode" = executed ]; then
      cargo=$(python3 - "$private/timing.json" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));
if set(v)!={"cargo_ms","finished_ms","started_ms"} or not all(isinstance(v[k],int) for k in v) or v["cargo_ms"]<0 or v["finished_ms"]-v["started_ms"]!=v["cargo_ms"]:raise SystemExit(1)
print(v["cargo_ms"])
PY
      ) || return 1
      [ "$cargo" = "$(bash "$helper_path" cargo-elapsed-ms "$build_log")" ] || { unresolved 'Cargo timing evidence mismatch'; return 1; }
    else cargo=0
    fi
  fi
  ended=$(now_ms); record_timing execution "$CUR_LAYOUT/$CUR_CASE/$CUR_TOKEN/$phase" "$started" "$ended" "$cargo"
  probe_gate "$CUR_CASE" "$phase-after-build" 0 || return 1
  verify_image "$phase" "$tag" "$CUR_ATTEMPT_DIR/results/$phase" || { emit_event VERIFY "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" FAIL 1 image-contract; return 1; }
  if [ "$skip" = 2 ]; then
    cmp -s -- "$repeat_result" "$CUR_ATTEMPT_DIR/results/$phase/result.json" || { unresolved 'exact-repeat image identity or payload changed'; return 1; }
    write_phase "$phase" "$tag" "$artifact" exact-repeat || return 1
    emit_event VERIFY "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" PASS 0 "tag=$tag;exact-repeat-id-stable=true"
  else
    write_phase "$phase" "$tag" "$artifact" || return 1
    emit_event VERIFY "$CUR_LAYOUT" "$CUR_CASE" "$phase" "$CUR_ATTEMPT" PASS 0 "tag=$tag"
  fi
  if [ "$phase" = p2 ]; then
    probe_gate "$CUR_CASE" batch-p2 0 || return 1
    [ "$CUR_CASE" != stop-after-p2 ] || ! first_trial_injection || { expected_stop batch-p2 75 stop-after-p2 "$tag"; return $?; }
  fi
}

run_trial() {
  local layout=$1 case_id=$2 source=$3 attempt_dir=$4 attempt=$5 token=$6 do_p0=$7 identity_token=$8 commit=$9 setting=${10} resume_phase=${11:-}
  CUR_LAYOUT=$layout CUR_CASE=$case_id CUR_SCOPE=$(scope_for "$case_id") CUR_SOURCE=$source CUR_ATTEMPT_DIR=$attempt_dir CUR_ATTEMPT=$attempt CUR_TOKEN=$token CUR_COMMIT=$commit CUR_SETTING=$setting CUR_CONFIG=$(config_for "$case_id")
  # Warm-up establishes the unmodified control.  The config-key trial alone
  # changes the compatibility input after that control has completed.
  case "$token" in *-setup) CUR_CONFIG=default ;; esac
  CUR_COLD=0; case "$token" in cold|*-setup|empty-cache|route-*) CUR_COLD=1 ;; esac
  [ "$case_id" != exact-repeat ] || CUR_COLD=0
  mkdir -p -m 0700 -- "$attempt_dir/logs" "$attempt_dir/results"
  local identity started ended
  if [ "$do_p0" = 1 ]; then
    probe_gate "$case_id" p0-before 0 || return 1
    started=$(now_ms)
    local p0="$attempt_dir/p0-identity"
    if identity_build "$source" "$p0" "$attempt_dir/logs/p0.log"; then :; else local identity_status=$?; emit_event PHASE "$layout" "$case_id" p0 "$attempt" FAIL "$identity_status" identity-build-failed; return 1; fi
    [ -f "$p0/identity.json" ] && [ ! -L "$p0/identity.json" ] || return 1
    write_identity "$p0/identity.json" || return 1
    ended=$(now_ms); record_timing preparation "$layout/$case_id/$token/p0" "$started" "$ended" not-applicable
    emit_event PHASE "$layout" "$case_id" p0 "$attempt" COMPLETE 0 "identity=$(sha256sum -- "$p0/identity.json" | awk '{print $1}')"
    identity="$p0/identity.json"
  else
    identity=$(load_identity "$identity_token") || return 1
  fi
  if [ "$case_id" = delete-input ] && [ ! -f "$CUR_SOURCE/fixture-app/data/embedded.txt" ]; then
    expected_stop p1 1 declared-embedded-input-missing "build-layout-$run_id-$CUR_LAYOUT-$CUR_SCOPE-$CUR_TOKEN-p1"
    return $?
  fi
  if [ "$case_id" = delete-source ] && [ ! -f "$CUR_SOURCE/fixture-app/src/bin/cache-bin-b.rs" ]; then
    expected_stop p1 1 declared-bin-input-missing "build-layout-$run_id-$CUR_LAYOUT-$CUR_SCOPE-$CUR_TOKEN-p1"
    return $?
  fi
  local phase skip
  for phase in "${phases[@]}"; do
    skip=0
    [ "$case_id/$token" = exact-repeat/cold ] && skip=2
    case "$resume_phase/$phase" in p2/p1|p3/p1|p3/p2|p4/p1|p4/p2|p4/p3) skip=1 ;; esac
    if run_phase "$phase" "$identity" "$skip"; then :; else return $?; fi
  done
  probe_gate "$case_id" batch-p4 0 || return 1
  probe_gate "$case_id" final-verification 0 || return 1
  emit_event CASE "$layout" "$case_id" final "$attempt" PASS 0 complete
}

next_attempt() {
  python3 - "$run_dir/$1/cases/$2" <<'PY'
import os,re,sys
n=0
if os.path.isdir(sys.argv[1]):
 for x in os.listdir(sys.argv[1]):
  m=re.fullmatch(r"attempt-(\d{3})",x)
  if m:n=max(n,int(m.group(1)))
print(n+1)
PY
}

seed_cold() {
  local layout=$1 state
  state=$(state_phase "$layout" warm-chain cold p1)
  [ -f "$state" ] && return 0
  local attempt; attempt=$(next_attempt "$layout" cold)
  local dir source
  dir="$run_dir/$layout/cases/cold/attempt-$(printf '%03d' "$attempt")"; source="$dir/source"
  source_copy "$source" || return 1
  RESUMING=0
  run_trial "$layout" cold "$source" "$dir" "$attempt" cold 1 cold "$c1" default
}

warmup() {
  local layout=$1 case_id=$2 scope state
  scope=$(scope_for "$case_id"); state=$(state_phase "$layout" "$scope" "$case_id-setup" p1)
  [ -f "$state" ] && return 0
  local dir source
  dir="$run_dir/$layout/cases/$case_id/setup/attempt-001"; source="$dir/source"
  source_copy "$source" || return 1
  RESUMING=0
  run_trial "$layout" "$case_id" "$source" "$dir" 1 "$case_id-setup" 1 "$case_id-setup" "$c1" default
}

run_route() {
  local layout=$1 case_id=route-contract identity phase=p1 tag private artifact dir source summary observation status
  dir="$run_dir/$layout/cases/route-contract/attempt-001"; source="$dir/source"
  source_copy "$source" || return 1
  CUR_LAYOUT=$layout CUR_CASE=$case_id CUR_SCOPE=route-contract CUR_TOKEN=route-source CUR_SOURCE=$source CUR_ATTEMPT=1 CUR_ATTEMPT_DIR=$dir CUR_COMMIT=$c1 CUR_SETTING=default CUR_CONFIG=default CUR_COLD=1
  mkdir -p -m 0700 -- "$dir/logs" "$dir/results"
  probe_gate "$case_id" p0-before 0 || return 1
  if identity_build "$source" "$dir/p0-identity" "$dir/logs/p0.log"; then status=0; else status=$?; fi
  [ "$status" -eq 0 ] || { emit_event PHASE "$layout" "$case_id" p0 1 FAIL "$status" identity-build-failed; return 1; }
  write_identity "$dir/p0-identity/identity.json" || return 1
  identity="$dir/p0-identity/identity.json"; CUR_PHASE=p1; compute_inputs "$identity" || return 1
  CUR_SOURCE_TRAP=0; tag="build-layout-$run_id-$layout-route-contract-route-source-p1"
  probe_gate "$case_id" source-fallback-before 0 || return 1
  compose_build p1 probe-a-base "$tag" source "$dir/logs/source-fallback.log" || return 1
  summary="$dir/results/source-fallback-cargo-summary.json"; observation="$dir/results/source-fallback-cargo-observation.json"
  bash "$helper_path" cargo-summary "$dir/logs/source-fallback.log" cache-bin-a "$summary" "$CUR_HOST" || return 1
  assert_cargo_observation "$summary" p1 "$dir/logs/source-fallback.log" "$observation" || return 1
  probe_gate "$case_id" source-fallback-after 0 || return 1
  verify_image p1 "$tag" "$dir/results/source-fallback" || return 1
  emit_event ROUTE "$layout" "$case_id" source-fallback 1 PASS 0 no-selector-no-additional-context
  CUR_TOKEN=route-artifact; CUR_COLD=1; CUR_PHASE=p1; compute_inputs "$identity" || return 1
  private="$dir/route-private-p1"; probe_gate "$case_id" route-artifact-before 0 || return 1
  artifact_build p1 "$private" "$dir/logs/prepare-route-artifact.log" || return 1
  observation="$dir/results/route-artifact-cargo-observation.json"
  assert_cargo_observation "$private/cargo-summary.json" p1 "$dir/logs/prepare-route-artifact.log" "$observation" || return 1
  artifact=$(publish_checked p1 "$private") || return 1
  probe_gate "$case_id" route-artifact-after 0 || return 1
  CUR_SOURCE_TRAP=1; tag="build-layout-$run_id-$layout-route-contract-route-artifact-p1"
  probe_gate "$case_id" artifact-present-before 0 || return 1
  compose_build p1 probe-a-base "$tag" "$artifact" "$dir/logs/artifact-present.log" || return 1
  probe_gate "$case_id" artifact-present-after 0 || return 1
  verify_image p1 "$tag" "$dir/results/artifact-present" || return 1
  if grep -Eiq 'source trap|source-builder' "$dir/logs/artifact-present.log"; then unresolved 'artifact route selected source stage'; return 1; fi
  emit_event ROUTE "$layout" "$case_id" artifact-present 1 PASS 0 "source-trap=1;artifact=$artifact"
  probe_gate "$case_id" final-verification 0 || return 1
  emit_event CASE "$layout" "$case_id" final 1 PASS 0 complete
}

run_case() {
  local layout=$1 case_id=$2
  [ "$case_id" = route-contract ] && { run_route "$layout"; return $?; }
  case "$case_id" in exact-repeat|forced-warm) seed_cold "$layout" || return $? ;; esac
  if needs_warmup "$case_id"; then warmup "$layout" "$case_id" || return $?; fi
  local attempt dir source
  local token="$case_id-trial-1" do_p0=0 identity_token="$case_id-setup" commit setting branch=normal resume_phase=
  commit=$(commit_for "$case_id"); setting=$(setting_for "$case_id")
  case "$case_id" in
    cold) token=cold; do_p0=1; identity_token=cold ;;
    exact-repeat) token=cold; identity_token=cold ;;
    forced-warm) token=forced-warm; identity_token=cold ;;
    empty-cache) token=empty-cache; do_p0=1; identity_token=empty-cache ;;
    branch-return) branch=forward ;;
  esac
  if [ "${RESUME_ACTIVE:-0}" = 1 ] && [ "$layout" = "$RESUME_LAYOUT" ] && [ "$case_id" = "$RESUME_CASE" ]; then
    attempt=$(next_attempt "$layout" "$case_id")
    [ "$attempt" -eq $((RESUME_ATTEMPT+1)) ] || { unresolved 'resume attempt sequence is not contiguous'; return 1; }
    dir="$run_dir/$layout/cases/$case_id/attempt-$(printf '%03d' "$attempt")"; source="$dir/source"
    source_copy "$source" || return 1
    token=$RESUME_TOKEN; identity_token=$RESUME_IDENTITY_TOKEN; do_p0=0; resume_phase=$RESUME_PHASE; RESUMING=1
  else
    attempt=$(next_attempt "$layout" "$case_id")
    dir="$run_dir/$layout/cases/$case_id/attempt-$(printf '%03d' "$attempt")"; source="$dir/source"
    source_copy "$source" || return 1
    mutate_source "$source" "$case_id" "$branch"
    RESUMING=0
  fi
  if [ "$case_id" = broken-ledger ] && [ "$layout" = baseline ]; then emit_event CASE "$layout" "$case_id" ledger "$attempt" NOT_APPLICABLE 0 baseline-no-ledger; fi
  run_trial "$layout" "$case_id" "$source" "$dir" "$attempt" "$token" "$do_p0" "$identity_token" "$commit" "$setting" "$resume_phase" || return $?
  if [ "$case_id" = feature-switch ] && [ "$RESUMING" != 1 ]; then
    local second second_source
    second="$run_dir/$layout/cases/$case_id/attempt-$(printf '%03d' $((attempt+1)))"; second_source="$second/source"
    source_copy "$second_source" || return 1
    run_trial "$layout" "$case_id" "$second_source" "$second" "$((attempt+1))" "$case_id-trial-2" 0 "$case_id-setup" "$commit" "$setting" || return $?
  fi
  if [ "$case_id" = branch-return ] && [ "$RESUMING" != 1 ]; then
    local second second_source
    second="$run_dir/$layout/cases/$case_id/attempt-$(printf '%03d' $((attempt+1)))"; second_source="$second/source"
    source_copy "$second_source" || return 1
    mutate_source "$second_source" branch-return return
    run_trial "$layout" "$case_id" "$second_source" "$second" "$((attempt+1))" "$case_id-trial-2" 0 "$case_id-setup" "$commit" "$setting" || return $?
  fi
}

selected_layouts() { [ "$layout_selection" = all ] && printf '%s\n' "${layouts[@]}" || printf '%s\n' "$layout_selection"; }
selected_cases() { [ "$case_selection" = all ] && printf '%s\n' "${case_ids[@]}" || printf '%s\n' "$case_selection"; }

run_suite() {
  local layout case_id status
  while IFS= read -r layout; do
    while IFS= read -r case_id; do
      if [ "${RESUME_ACTIVE:-0}" = 1 ]; then
        [ "$layout" = "$RESUME_LAYOUT" ] || continue
        [ "${case_numbers[$case_id]}" -ge "${case_numbers[$RESUME_CASE]}" ] || continue
      fi
      if run_case "$layout" "$case_id"; then status=0; else status=$?; fi
      if [ "$status" = 75 ]; then set_run_status expected-stop 75; return 75; fi
      [ "$status" = 0 ] || { set_run_status failed 1; return 1; }
      if [ "${RESUME_ACTIVE:-0}" = 1 ] && [ "$layout" = "$RESUME_LAYOUT" ] && [ "$case_id" = "$RESUME_CASE" ]; then RESUME_ACTIVE=0; fi
    done < <(selected_cases)
  done < <(selected_layouts)
  set_run_status complete 0
}

init_run() {
  research_exception_validate_bound || return 1
  run_dir=$output_dir; run_id=$(python3 -c 'import secrets; print(secrets.token_hex(16))') || return 1
  mkdir -p -m 0700 -- "$run_dir"
  LAYOUT_RESEARCH_SNAPSHOT="$run_dir/research-exception.json"
  research_exception_check "$LAYOUT_RESEARCH_BINDING" "$LAYOUT_RESEARCH_SNAPSHOT" create >/dev/null || return 1
  events_file="$run_dir/events.jsonl"; gate_file="$run_dir/gates.jsonl"; run_json="$run_dir/run.json"; timing_file="$run_dir/timing.tsv"
  : >"$events_file"; : >"$gate_file"; printf 'category\tscope\twall_ms\tcargo_ms\tfinished_ms\n' >"$timing_file"
  chmod 0600 -- "$events_file" "$gate_file" "$timing_file"
  local first_gate_us; first_gate_us=$(( $(date -u +%s) * 1000000 ))
  LAYOUT_JOURNAL_SINCE_US=$((first_gate_us - 1800000000))
  LAYOUT_JOURNAL_SINCE=$(date -u -d "@$((LAYOUT_JOURNAL_SINCE_US / 1000000))" '+%Y-%m-%d %H:%M:%S UTC')
  LAYOUT_JOURNAL_BOOT_ID=
  export LAYOUT_JOURNAL_SINCE_US LAYOUT_JOURNAL_SINCE LAYOUT_JOURNAL_BOOT_ID
  write_run_json || return 1
  export LAYOUT_GATE_RECORD_FILE=$gate_file LAYOUT_GATE_STATE_FILE="$run_dir/gate-state.json" LAYOUT_GATE_PRIVATE_DIR="$run_dir/private-gates"
  emit_event RUN "$layout_selection" "$case_selection" prepare 0 START 0 new-run
}

resume_terminal() {
  python3 - "$events_file" <<'PY'
import json,sys
events=[json.loads(line) for line in open(sys.argv[1],encoding="utf-8")]
if not events or events[-1].get("type")!="RUN_STATUS":raise SystemExit("resume-terminal-missing")
terminal=events[-1]
if terminal.get("status")=="complete": print("COMPLETE"); raise SystemExit(0)
if terminal.get("status")=="failed": print("FAILED"); raise SystemExit(0)
if terminal.get("status")!="expected-stop" or len(events)<2:raise SystemExit("resume-terminal-invalid")
stop=events[-2]
if stop.get("type")!="EXPECTED_STOP" or stop.get("status")!="EXPECTED_STOP" or "injection_consumed=true" not in stop.get("detail",""):
    raise SystemExit("resume-expected-stop-invalid")
print("\t".join(("EXPECTED_STOP",stop["layout"],stop["case"],stop["phase"],str(stop["attempt"]),stop["token"],stop["detail"])))
PY
}

validate_failure_snapshot() {
  local source="$run_dir/$RESUME_LAYOUT/cases/$RESUME_CASE/attempt-$(printf '%03d' "$RESUME_ATTEMPT")/failure-source"
  [ -d "$source" ] && [ ! -L "$source" ] || { unresolved 'retained failure source is missing'; return 1; }
  [ "$(bash "$helper_path" source-input-hash "$source")" = "$RESUME_FAILURE_SHA" ] || { unresolved 'retained failure source hash changed'; return 1; }
  python3 - "$fixture_dir" "$source" "$RESUME_CASE" <<'PY'
import os,stat,sys
fixture=os.path.abspath(sys.argv[1]); source=os.path.abspath(sys.argv[2]); case_id=sys.argv[3]
def inventory(root):
    out={}
    for current,dirs,files in os.walk(root,topdown=True,followlinks=False):
        dirs.sort();files.sort()
        for name in dirs:
            path=os.path.join(current,name); info=os.lstat(path)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):raise SystemExit("failure-source-invalid-directory")
        for name in files:
            path=os.path.join(current,name); info=os.lstat(path)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):raise SystemExit("failure-source-invalid-file")
            rel=os.path.relpath(path,root);out[rel]=(stat.S_IMODE(info.st_mode),open(path,"rb").read())
    return out
expected=inventory(fixture);actual=inventory(source)
if case_id=="compile-fail-p2":
    key="fixture-app/src/bin/cache-bin-b.rs"; expected[key]=(expected[key][0],expected[key][1]+b'compile_error!("layout-probe-injected");\n')
elif case_id=="delete-input": expected.pop("fixture-app/data/embedded.txt")
elif case_id=="delete-source": expected.pop("fixture-app/src/bin/cache-bin-b.rs")
if actual!=expected:raise SystemExit("retained-failure-source-contract-mismatch")
PY
}

validate_persisted_state() {
  [ -d "$run_dir/state" ] || { unresolved 'resume state ledger is missing'; return 1; }
  [ -z "$(find "$run_dir/state" -type l -print -quit)" ] || { unresolved 'resume state contains a symlink'; return 1; }
  local state kind identity identity_sha source source_sha recipe stored_tool stored_fixture file_sha artifact artifact_sha result result_sha observation observation_sha serialized
  while IFS= read -r state; do
    [ -f "$state" ] && [ ! -L "$state" ] || return 1
    if [ "$(basename "$state")" = identity.json ]; then
      serialized=$(python3 - "$state" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));required={"fixture_sha256","host_triple","identity_json","identity_sha256","recipe_sha256","source_path","source_sha256","tool_sha256"}
if set(v)!=required:raise SystemExit(1)
for key in ("identity_json","identity_sha256","source_path","source_sha256","recipe_sha256","tool_sha256","fixture_sha256"):print(v[key])
PY
) || return 1
      mapfile -t fields <<<"$serialized"
      identity=${fields[0]} identity_sha=${fields[1]} source=${fields[2]} source_sha=${fields[3]} recipe=${fields[4]} stored_tool=${fields[5]} stored_fixture=${fields[6]}
      [ "$stored_tool" = "$tool_sha" ] && [ "$stored_fixture" = "$fixture_sha" ] || { unresolved 'persisted identity ownership changed'; return 1; }
      [ -f "$identity" ] && [ ! -L "$identity" ] && [ "$(sha256sum -- "$identity" | awk '{print $1}')" = "$identity_sha" ] && bash "$helper_path" identity-validate "$identity" linux/amd64 >/dev/null || { unresolved 'persisted native identity changed'; return 1; }
      [ -d "$source" ] && [ ! -L "$source" ] && [ "$(bash "$helper_path" source-input-hash "$source")" = "$source_sha" ] || { unresolved 'persisted P0 source changed'; return 1; }
      case "$state" in "$run_dir/state/baseline/"*) [ "$(recipe_hash baseline "$source")" = "$recipe" ] ;; "$run_dir/state/common/"*) [ "$(recipe_hash common "$source")" = "$recipe" ] ;; "$run_dir/state/grouped/"*) [ "$(recipe_hash grouped "$source")" = "$recipe" ] ;; *) return 1 ;; esac || { unresolved 'persisted P0 recipe changed'; return 1; }
    else
      serialized=$(python3 - "$state" "$run_id" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));required={"artifact","artifact_content_sha256","bin","case","commit","compatibility_key","compile_input_sha256","config","fixture_sha256","host_triple","identity_sha256","input_sha256","layout","observation_path","observation_sha256","phase","recipe_sha256","requested_features","resolved_features","result","result_path","result_sha256","runtime","setting","source_path","source_sha256","tag","target_namespace","token","tool_sha256","variant"}
if set(v)!=required:raise SystemExit(1)
prefix="build-layout-"+sys.argv[2]+"-"+v["layout"]+"-"
if not v["tag"].startswith(prefix) or not v["target_namespace"].startswith(prefix):raise SystemExit("persisted-run-namespace-mismatch")
for key in ("source_path","source_sha256","result_path","result_sha256","observation_path","observation_sha256","artifact","artifact_content_sha256","tool_sha256","fixture_sha256"):print(v[key])
PY
) || return 1
      mapfile -t fields <<<"$serialized"
      source=${fields[0]} source_sha=${fields[1]} result=${fields[2]} result_sha=${fields[3]} observation=${fields[4]} observation_sha=${fields[5]} artifact=${fields[6]} artifact_sha=${fields[7]} stored_tool=${fields[8]} stored_fixture=${fields[9]}
      [ "$stored_tool" = "$tool_sha" ] && [ "$stored_fixture" = "$fixture_sha" ] || { unresolved 'persisted phase ownership changed'; return 1; }
      [ -d "$source" ] && [ ! -L "$source" ] && [ "$(bash "$helper_path" source-input-hash "$source")" = "$source_sha" ] || { unresolved 'persisted phase source changed'; return 1; }
      [ -f "$result" ] && [ ! -L "$result" ] && [ "$(sha256sum -- "$result" | awk '{print $1}')" = "$result_sha" ] || { unresolved 'persisted image result changed'; return 1; }
      [ -f "$observation" ] && [ ! -L "$observation" ] && [ "$(sha256sum -- "$observation" | awk '{print $1}')" = "$observation_sha" ] || { unresolved 'persisted Cargo evidence changed'; return 1; }
      if [ -n "$artifact" ]; then [ -d "$artifact" ] && [ ! -L "$artifact" ] && [ "$(basename "$artifact")" = "$artifact_sha" ] && [ "$(bash "$helper_path" tree-hash "$artifact")" = "$artifact_sha" ] || { unresolved 'persisted artifact changed'; return 1; }; fi
    fi
  done < <(find "$run_dir/state" -mindepth 4 -maxdepth 4 -type f -name '*.json' | LC_ALL=C sort)
}

validate_all_saved_images() {
  local audit state serialized count=0 saved_layout=$CUR_LAYOUT saved_case=$CUR_CASE saved_phase=$CUR_PHASE saved_source=$CUR_SOURCE saved_commit=$CUR_COMMIT saved_setting=$CUR_SETTING saved_config=$CUR_CONFIG saved_host=$CUR_HOST saved_k=$CUR_K saved_h=$CUR_H saved_recipe=$CUR_RECIPE saved_compile=$CUR_COMPILE saved_identity=$CUR_IDENTITY_SHA artifact result phase bin variant tag
  audit=$(mktemp -d "${TMPDIR:-/tmp}/build-layout-resume-verify.XXXXXX") || return 1
  chmod 0700 -- "$audit"
  while IFS= read -r state; do
    [ "$(basename "$state")" != identity.json ] || continue
    serialized=$(python3 - "$state" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));
for key in ("layout","case","phase","source_path","commit","setting","config","host_triple","compatibility_key","input_sha256","recipe_sha256","compile_input_sha256","identity_sha256","artifact","result_path","tag","bin","variant"):print(v[key])
PY
) || { rm -rf -- "$audit"; return 1; }
    mapfile -t fields <<<"$serialized"
    CUR_LAYOUT=${fields[0]} CUR_CASE=${fields[1]} CUR_PHASE=${fields[2]} CUR_SOURCE=${fields[3]} CUR_COMMIT=${fields[4]} CUR_SETTING=${fields[5]} CUR_CONFIG=${fields[6]} CUR_HOST=${fields[7]} CUR_K=${fields[8]} CUR_H=${fields[9]} CUR_RECIPE=${fields[10]} CUR_COMPILE=${fields[11]} CUR_IDENTITY_SHA=${fields[12]}
    artifact=${fields[13]} result=${fields[14]} tag=${fields[15]} bin=${fields[16]} variant=${fields[17]} phase=$CUR_PHASE
    [ "$bin" = "${p_bin[$phase]}" ] && [ "$variant" = "${p_variant[$phase]}" ] || { rm -rf -- "$audit"; unresolved 'persisted phase mapping changed'; return 1; }
    count=$((count+1))
    verify_image "$phase" "$tag" "$audit/$count" || { rm -rf -- "$audit"; unresolved 'persisted image verification failed'; return 1; }
    cmp -s -- "$result" "$audit/$count/result.json" || { rm -rf -- "$audit"; unresolved 'persisted image result no longer matches'; return 1; }
    if [ -n "$artifact" ]; then verify_artifact "$artifact" "$phase" || { rm -rf -- "$audit"; unresolved 'persisted artifact verification failed'; return 1; }; fi
  done < <(find "$run_dir/state" -mindepth 4 -maxdepth 4 -type f -name '*.json' | LC_ALL=C sort)
  rm -rf -- "$audit"
  CUR_LAYOUT=$saved_layout CUR_CASE=$saved_case CUR_PHASE=$saved_phase CUR_SOURCE=$saved_source CUR_COMMIT=$saved_commit CUR_SETTING=$saved_setting CUR_CONFIG=$saved_config CUR_HOST=$saved_host CUR_K=$saved_k CUR_H=$saved_h CUR_RECIPE=$saved_recipe CUR_COMPILE=$saved_compile CUR_IDENTITY_SHA=$saved_identity
  RESUME_VALIDATED_PHASES=$count
}

load_resume_state() {
  run_dir=$resume_dir; run_id=; events_file="$run_dir/events.jsonl"; gate_file="$run_dir/gates.jsonl"; run_json="$run_dir/run.json"; timing_file="$run_dir/timing.tsv"
  local recorded_hash serialized; recorded_hash=$(tr -d '\n' <"$run_dir/run.json.sha256")
  [[ "$recorded_hash" =~ ^[0-9a-f]{64}$ ]] && [ "$(sha256sum -- "$run_json" | awk '{print $1}')" = "$recorded_hash" ] || { unresolved 'initial run.json changed'; return 1; }
  load_research_exception_binding || return 1
  validate_event_chain || { unresolved 'event ledger changed'; return 1; }
  serialized=$(PROBE_HEAD="$origin_head" PROBE_TOOL="$tool_sha" PROBE_FIXTURE="$fixture_sha" PROBE_DESIGN="$design_sha" PROBE_BASELINE="$baseline_sha" PROBE_UNITS="$BUILD_LAYOUT_HEALTH_UNITS" PROBE_CONTAINERS="$BUILD_LAYOUT_HEALTH_CONTAINERS" PROBE_RUN="$run_id" PROBE_HELPER="$(sha256sum -- "$helper_path" | awk '{print $1}')" PROBE_LAYOUT_JSON="$(sha256sum -- "$fixture_dir/layout.json" | awk '{print $1}')" PROBE_COMPOSE="$(sha256sum -- "$fixture_dir/compose.yml" | awk '{print $1}')" PROBE_DOCKER_BASELINE="$(sha256sum -- "$fixture_dir/Dockerfile.baseline" | awk '{print $1}')" PROBE_DOCKER_ARTIFACTS="$(sha256sum -- "$fixture_dir/Dockerfile.artifacts" | awk '{print $1}')" PROBE_DOCKER_CONSUMER="$(sha256sum -- "$fixture_dir/Dockerfile.consumer" | awk '{print $1}')" python3 - "$run_json" <<'PY'
import json,os,re,sys
v=json.load(open(sys.argv[1]));
if v.get("schema")!="lagrange-build-layout-probe-v3" or not isinstance(v.get("run_id"),str) or not re.fullmatch(r"[0-9a-f]{32}",v["run_id"]):raise SystemExit(1)
checks=(("origin_head","PROBE_HEAD"),("tool_sha256","PROBE_TOOL"),("fixture_source_input_sha256","PROBE_FIXTURE"),("document_sha256","PROBE_DESIGN"),("baseline_document_sha256","PROBE_BASELINE"))
if any(v.get(k)!=os.environ[e] for k,e in checks):raise SystemExit(1)
recipes=v.get("recipes",{}); expected={"Dockerfile.artifacts":os.environ["PROBE_DOCKER_ARTIFACTS"],"Dockerfile.baseline":os.environ["PROBE_DOCKER_BASELINE"],"Dockerfile.consumer":os.environ["PROBE_DOCKER_CONSUMER"],"compose.yml":os.environ["PROBE_COMPOSE"],"layout-helper.sh":os.environ["PROBE_HELPER"],"layout.json":os.environ["PROBE_LAYOUT_JSON"]}
if any(recipes.get(k)!=x for k,x in expected.items()):raise SystemExit(1)
gate=v.get("gate",{});
if gate.get("units")!=os.environ["PROBE_UNITS"].split(",") or gate.get("containers")!=os.environ["PROBE_CONTAINERS"].split(","):raise SystemExit(1)
selection=v.get("selection",{});print(selection.get("layout"));print(selection.get("case"));print(str(gate.get("journal_since_us")));print(gate.get("journal_since"));print(v["run_id"])
PY
) || { unresolved 'resume immutable input, recipe, selection, or health target changed'; return 1; }
  mapfile -t fields <<<"$serialized"
  layout_selection=${fields[0]} case_selection=${fields[1]} LAYOUT_JOURNAL_SINCE_US=${fields[2]} LAYOUT_JOURNAL_SINCE=${fields[3]} run_id=${fields[4]}
  local terminal; terminal=$(resume_terminal) || { unresolved 'resume cursor is invalid'; return 1; }
  if [ "$terminal" = FAILED ]; then unresolved 'unexpected failure is not resumable'; return 1; fi
  RESUME_COMPLETE=0
  if [ "$terminal" = COMPLETE ]; then RESUME_COMPLETE=1
  else
    IFS=$'\t' read -r _ RESUME_LAYOUT RESUME_CASE RESUME_PHASE RESUME_ATTEMPT RESUME_TOKEN RESUME_DETAIL <<<"$terminal"
    RESUME_FAILURE_SHA=$(printf '%s\n' "$RESUME_DETAIL" | sed -n 's/.*failure_source_sha256=\([0-9a-f]\{64\}\).*/\1/p')
    [ -n "$RESUME_FAILURE_SHA" ] || { unresolved 'resume failure source hash is missing'; return 1; }
    [ "$RESUME_PHASE" = batch-p2 ] && RESUME_PHASE=p3
    case "$RESUME_PHASE" in p1|p2|p3|p4) ;; *) unresolved 'invalid resume phase'; return 1 ;; esac
    RESUME_IDENTITY_TOKEN="$RESUME_CASE-setup"
    case "$RESUME_CASE" in cold|exact-repeat|forced-warm) RESUME_IDENTITY_TOKEN=cold ;; empty-cache) RESUME_IDENTITY_TOKEN=empty-cache ;; esac
    validate_failure_snapshot || return 1
  fi
  validate_persisted_state || return 1
  LAYOUT_JOURNAL_BOOT_ID=
  if [ -f "$run_dir/gate-state.json" ]; then
    serialized=$(python3 - "$run_dir/gate-state.json" "$LAYOUT_JOURNAL_SINCE_US" "$LAYOUT_JOURNAL_SINCE" <<'PY'
import json,sys
v=json.load(open(sys.argv[1]));
if v.get("journal_since_us")!=int(sys.argv[2]) or v.get("journal_since")!=sys.argv[3]:raise SystemExit(1)
print(v.get("boot_id",""))
PY
) || { unresolved 'persisted journal origin changed'; return 1; }
    mapfile -t gate_fields <<<"$serialized"
    LAYOUT_JOURNAL_BOOT_ID=${gate_fields[0]}
  fi
  export LAYOUT_JOURNAL_SINCE_US LAYOUT_JOURNAL_SINCE LAYOUT_JOURNAL_BOOT_ID
  export LAYOUT_GATE_RECORD_FILE=$gate_file LAYOUT_GATE_STATE_FILE="$run_dir/gate-state.json" LAYOUT_GATE_PRIVATE_DIR="$run_dir/private-gates"
}

validate_health_inputs() {
  [ -n "${BUILD_LAYOUT_HEALTH_UNITS:-}" ] || die 'BUILD_LAYOUT_HEALTH_UNITS is required for --apply'
  [ -n "${BUILD_LAYOUT_HEALTH_CONTAINERS:-}" ] || die 'BUILD_LAYOUT_HEALTH_CONTAINERS is required for --apply'
  gate_validate_list "$BUILD_LAYOUT_HEALTH_UNITS" unit || die 'invalid BUILD_LAYOUT_HEALTH_UNITS'
  gate_validate_list "$BUILD_LAYOUT_HEALTH_CONTAINERS" container || die 'invalid BUILD_LAYOUT_HEALTH_CONTAINERS'
}

load_research_exception_binding() {
  local expected
  expected=$(python3 - "$run_json" <<'PY'
import json,sys
print(json.dumps(json.load(open(sys.argv[1]))["gate"].get("research_exception"),sort_keys=True,separators=(",",":")))
PY
) || return 1
  LAYOUT_RESEARCH_SNAPSHOT="$run_dir/research-exception.json"
  LAYOUT_RESEARCH_BINDING=$(research_exception_check "$expected" "$LAYOUT_RESEARCH_SNAPSHOT") || return 1
  if [ "$LAYOUT_RESEARCH_BINDING" != null ]; then
    research_exception_resume_check "$run_dir/gate-state.json" "$run_dir/gates.jsonl" "$run_json" || return 1
  fi
}

print_resume_plan() {
  run_dir=$resume_dir; events_file="$run_dir/events.jsonl"; run_json="$run_dir/run.json"
  local recorded_hash terminal
  recorded_hash=$(tr -d '\n' <"$run_dir/run.json.sha256")
  [[ "$recorded_hash" =~ ^[0-9a-f]{64}$ ]] && [ "$(sha256sum -- "$run_json" | awk '{print $1}')" = "$recorded_hash" ] || { unresolved 'initial run.json changed'; return 1; }
  load_research_exception_binding || return 1
  validate_event_chain || { unresolved 'event ledger changed'; return 1; }
  terminal=$(resume_terminal) || { unresolved 'resume cursor is invalid'; return 1; }
  printf 'build-layout-probe: resume plan is read-only: %s\nterminal=%s\nactual_docker_cargo_results=NOT_RUN\nG2=NOT_PASSED\n' "$resume_dir" "${terminal%%$'\t'*}"
}

print_plan() {
  cat <<EOF
build-layout-probe: WP-3 offline plan
origin_head=$origin_head
fixture_source_input_sha256=$fixture_sha
tool_sha256=$tool_sha
design_sha256=$design_sha
baseline_document_sha256=$baseline_sha
layouts=baseline,common,grouped
cases=28 (01..28 fixed order)
sequence=P0,P1/consumer,P2/consumer,gate,P3/consumer,P4/consumer,gate,final
cache_identity=K(compatibility),P(package),H(bin);runtime_payload=separate
actual_docker_cargo_results=NOT_RUN
actual_cache_performance=NOT_RUN
G2=NOT_PASSED
EOF
}

parse_cli() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --plan) [ "$mode" = plan ] || die 'mode flags are mutually exclusive'; mode=plan; shift ;;
      --apply) [ "$mode" = plan ] || die 'mode flags are mutually exclusive'; mode=apply; shift ;;
      --self-test) [ "$mode" = plan ] || die 'mode flags are mutually exclusive'; mode=self-test; shift ;;
      --layout) [ "$layout_seen" = 0 ] && [ "$#" -ge 2 ] || die '--layout needs one value'; layout_selection=$2; layout_seen=1; shift 2 ;;
      --case) [ "$case_seen" = 0 ] && [ "$#" -ge 2 ] || die '--case needs one value'; case_selection=$2; case_seen=1; shift 2 ;;
      --output-dir) [ "$output_seen" = 0 ] && [ "$#" -ge 2 ] || die '--output-dir needs one value'; output_dir=$2; output_seen=1; shift 2 ;;
      --resume-from) [ "$resume_seen" = 0 ] && [ "$#" -ge 2 ] || die '--resume-from needs one value'; resume_dir=$2; resume_seen=1; shift 2 ;;
      -h|--help) usage; exit 0 ;;
      *) die "unknown argument: $1" ;;
    esac
  done
}

sanitize_actual_environment() {
  unset BUILD_LAYOUT_SELF_TEST_ACTIVE LAYOUT_GATE_TEST_SEAM LAYOUT_GATE_MODE LAYOUT_FAKE_GATE_RESULT LAYOUT_GATE_PROC_ROOT LAYOUT_GATE_PS_BIN LAYOUT_GATE_SYSTEMCTL_BIN LAYOUT_GATE_JOURNALCTL_BIN LAYOUT_GATE_DOCKER_BIN
  LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=
}

self_expect_failure() {
  if "$@" >/dev/null 2>&1; then printf '%s\n' "self-test expected failure passed: $*" >&2; return 1; fi
}

create_fake_docker() {
  local path=$1
  cat >"$path" <<'PY'
#!/usr/bin/env python3
import hashlib, json, os, pathlib, re, shutil, stat, subprocess, sys

if os.environ.get("BUILD_LAYOUT_SELF_TEST_ACTIVE") != "1" or os.environ.get("LAYOUT_GATE_TEST_SEAM") != "build-layout-self-test":
    raise SystemExit(98)
root=pathlib.Path(os.environ["FAKE_DOCKER_ROOT"])
root.mkdir(parents=True,exist_ok=True)
calls=pathlib.Path(os.environ["FAKE_DOCKER_CALLS"])
args=sys.argv[1:]

def atomic_json(path,value):
    path=pathlib.Path(path); path.parent.mkdir(parents=True,exist_ok=True)
    temporary=path.with_name(path.name+f".tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n",encoding="utf-8")
    os.replace(temporary,path)

def tree_digest(path):
    path=pathlib.Path(path); records=[]
    for current,dirs,files in os.walk(path,topdown=True,followlinks=False):
        dirs.sort(); files.sort()
        for name in dirs:
            item=pathlib.Path(current)/name; info=item.lstat()
            if item.is_symlink() or not stat.S_ISDIR(info.st_mode): raise SystemExit(91)
            records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{item.relative_to(path)}\t-\n".encode())
        for name in files:
            item=pathlib.Path(current)/name; info=item.lstat()
            if item.is_symlink() or not stat.S_ISREG(info.st_mode): raise SystemExit(91)
            records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{item.relative_to(path)}\t{hashlib.sha256(item.read_bytes()).hexdigest()}\n".encode())
    return hashlib.sha256(b"".join(sorted(records))).hexdigest()

def get_option(name,values=args):
    positions=[i for i,x in enumerate(values) if x==name]
    if len(positions)!=1 or positions[0]+1>=len(values): raise SystemExit(92)
    return values[positions[0]+1]

def build_args(values=args):
    result={}; index=0
    while index<len(values):
        if values[index]=="--build-arg":
            key,value=values[index+1].split("=",1)
            if key in result: raise SystemExit(92)
            result[key]=value; index+=2
        else:index+=1
    return result

def output_dest(value):
    fields={part.split("=",1)[0]:part.split("=",1)[1] for part in value.split(",") if "=" in part}
    if fields.get("type")!="local" or fields.get("platform-split")!="false" or "dest" not in fields:raise SystemExit(92)
    return pathlib.Path(fields["dest"])

def identity_bytes():
    value={"cargo_version":"cargo 1.97.1 (012345678 2026-09-01)","format":"lagrange-build-layout-native-v1","native_packages":["bash-5.2.37-r0","python3-3.12.9-r0"],"rustc_vv":"rustc 1.97.1\nbinary: rustc\ncommit-hash: 0000000000000000000000000000000000000000\nhost: x86_64-unknown-linux-musl\nrelease: 1.97.1\nLLVM version: 21.1.0\n","target_platform":"linux/amd64"}
    return (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode()

def image_dir(tag):return root/"images"/hashlib.sha256(tag.encode()).hexdigest()
def state_path(target):return root/"targets"/(hashlib.sha256(target.encode()).hexdigest()+".json")
def load_state(target,reset=False):
    path=state_path(target)
    if not reset and path.exists():return json.loads(path.read_text())
    return {"app":{},"app_hash":None,"build":{},"build_run":{},"compatibility_key":None,"itoa":False,"lib":{},"lib_hash":None,"pending":False}
def save_state(target,value):atomic_json(state_path(target),value)

def package_profile(opt):return {"debug_assertions":False,"debuginfo":0,"opt_level":opt,"overflow_checks":False,"test":False}
def cargo_lines(bin_name,variant,fresh,run_build_script):
    resolved=["default"]+(["wide"] if variant=="wide" else [])
    app_id="path+file:///build/fixture-app#build-cache-fixture-app@0.1.0"
    events=[("Fresh" if fresh[2] else "Compiling")+" itoa v1.0.18\n"]
    if run_build_script:events.append("Running `/cargo-target/release/build/build-cache-fixture-app-fake/build-script-build`\n")
    def artifact(package,name,kind,src,features,is_fresh,profile,executable=None):
        return {"executable":executable,"features":features,"fresh":is_fresh,"package_id":package,"profile":profile,"reason":"compiler-artifact","target":{"crate_types":["bin" if kind=="custom-build" else kind],"edition":"2024","kind":[kind],"name":name,"src_path":src}}
    events.extend([
      json.dumps(artifact("registry+https://github.com/rust-lang/crates.io-index#itoa@1.0.18","itoa","lib","/usr/local/cargo/registry/src/index.crates.io-fake/itoa-1.0.18/src/lib.rs",[],fresh[2],package_profile("3")),separators=(",",":"))+"\n",
      json.dumps(artifact("path+file:///build/fixture-lib#build-cache-fixture-lib@0.1.0","build_cache_fixture_lib","lib","/build/fixture-lib/src/lib.rs",resolved,fresh[1],package_profile("3")),separators=(",",":"))+"\n",
      json.dumps(artifact(app_id,"build-cache-fixture-app","custom-build","/build/fixture-app/build.rs",resolved,fresh[3],package_profile("0"),"/cargo-target/release/build/build-cache-fixture-app-fake/build-script-build"),separators=(",",":"))+"\n",
      json.dumps({"cfgs":[],"env":[],"linked_libs":[],"linked_paths":[],"out_dir":"/cargo-target/release/build/build-cache-fixture-app-fake/out","package_id":app_id,"reason":"build-script-executed"},separators=(",",":"))+"\n",
      json.dumps(artifact(app_id,bin_name,"bin",f"/build/fixture-app/src/bin/{bin_name}.rs",resolved,fresh[0],package_profile("3"),f"/cargo-target/release/{bin_name}"),separators=(",",":"))+"\n",
      json.dumps({"reason":"build-finished","success":True},separators=(",",":"))+"\n"])
    return events

def expected_stdout(source,bin_name,variant,commit,setting):
    app=(source/"fixture-app/src/bin/cache-bin-a.rs").read_text()
    marker="source-v2" if bin_name=="cache-bin-a" and 'SOURCE_MARKER: &str = "source-v2"' in app else "source-v1"
    embedded=(source/"fixture-app/data/embedded.txt").read_text().strip()
    build=(source/"fixture-app/build.rs").read_text(); generated="generated-v2" if "generated-v2" in build else "generated-v1"
    lib=(source/"fixture-lib/src/lib.rs").read_text(); shared=84 if variant=="wide" else (43 if "else { 43 }" in lib else 42)
    return f"{bin_name}|commit={commit}|setting={setting}|source={marker}|embedded={embedded}|{generated}|commit={commit}|setting={setting}|embedded={embedded}|shared={shared}\n"

def compile_observation(source,values,baseline=False):
    target=values["PROBE_TARGET_CACHE_ID"]; state=load_state(target)
    pending_before=state["pending"]; reset="--no-cache" in args
    # Observed BuildKit behavior: a no-cache source RUN starts an empty target.
    # Verified artifact consumers never enter this source-compilation function.
    if reset:state=load_state(target,reset=True)
    key=values.get("PROBE_CACHE_KEY","source-route")
    lib_hash=values.get("PROBE_LIB_HASH",tree_digest(source/"fixture-lib")); app_hash=values.get("PROBE_APP_HASH",tree_digest(source/"fixture-app"))
    if baseline:
        state["app"]={};state["lib"]={};state["build"]={};state["build_run"]={};state["pending"]=False
    else:
        if values.get("PROBE_LEDGER_INJECT")=="unknown-pending":state["pending"]=True
        # A baseline/source RUN leaves compiled units but never a guard ledger.
        # The first artifact RUN must therefore follow real ledger-plan's reset.
        if not state.get("ledger") or state.get("pending") or state.get("compatibility_key")!=key:
            state={"app":{},"app_hash":None,"build":{},"build_run":{},"compatibility_key":key,"itoa":False,"lib":{},"lib_hash":None,"pending":False}
        elif state.get("lib_hash")!=lib_hash:
            state["app"]={};state["build"]={};state["build_run"]={};state["lib"]={}
        elif state.get("app_hash")!=app_hash:
            state["app"]={};state["build"]={};state["build_run"]={}
        state["pending"]=True
    variant=values["VARIANT"];bin_name=values["BIN"];commit=values["CACHE_FIXTURE_COMMIT"];setting=values["CACHE_FIXTURE_BUILD_SETTING"]
    app_key="|".join((bin_name,variant,app_hash,commit,setting));lib_key="|".join((variant,lib_hash));build_key="|".join((variant,app_hash));run_key="|".join((variant,app_hash,commit,setting))
    fresh=(app_key in state["app"],lib_key in state["lib"],bool(state["itoa"]),build_key in state["build"])
    run_build=run_key not in state["build_run"]
    state["compatibility_key"]=key;state["lib_hash"]=lib_hash;state["app_hash"]=app_hash
    if not baseline:state["ledger"]=True
    record("source-compile",{"baseline":baseline,"bin":bin_name,"fresh":fresh,"no_cache_reset":reset,"pending_before":pending_before,"target_namespace":target,"token":values["CACHE_FIXTURE_RUN_TOKEN"],"variant":variant})
    return state,fresh,run_build,(app_key,lib_key,build_key,run_key)

def complete_compile_state(target,state,keys,pending=False):
    app_key,lib_key,build_key,run_key=keys
    state["app"][app_key]=True;state["lib"][lib_key]=True;state["build"][build_key]=True;state["build_run"][run_key]=True;state["itoa"]=True;state["pending"]=pending
    save_state(target,state)

def write_binary(path,stdout):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(b"FAKE-ELF\x00"+hashlib.sha256(stdout.encode()).digest());path.chmod(0o755)

def write_artifact(destination,source,values,lines,cargo_ms):
    bin_name=values["BIN"]; variant=values["VARIANT"]; destination.mkdir(parents=True,exist_ok=True)
    stdout=expected_stdout(source,bin_name,variant,values["CACHE_FIXTURE_COMMIT"],values["CACHE_FIXTURE_BUILD_SETTING"])
    write_binary(destination/"bin"/bin_name,stdout)
    shutil.copyfile(source/"runtime"/values["RUNTIME_FILE"],destination/"runtime.txt")
    (destination/"cargo.jsonl").write_text("".join(lines),encoding="utf-8")
    helper=os.environ["FAKE_LAYOUT_HELPER"]
    subprocess.run(["bash",helper,"cargo-summary",str(destination/"cargo.jsonl"),bin_name,str(destination/"cargo-summary.json"),values["PROBE_HOST_TRIPLE"]],check=True)
    atomic_json(destination/"timing.json",{"cargo_ms":cargo_ms,"finished_ms":1000+cargo_ms,"started_ms":1000})
    if values.get("PROBE_INJECT")=="export-fail-p2":
        (destination/"INCOMPLETE").write_text("fixture-export-failure\n",encoding="ascii");return
    binary=destination/"bin"/bin_name
    requested=[] if variant=="base" else ["wide"]
    env={"CACHE_FIXTURE_BUILD_SETTING":values["CACHE_FIXTURE_BUILD_SETTING"],"CACHE_FIXTURE_COMMIT":values["CACHE_FIXTURE_COMMIT"],"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target"}
    record={"binary_mode":"0755","binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"bin":bin_name,"cache_key":values["PROBE_CACHE_KEY"],"cargo_success":True,"compile_env":env,"features":requested,"format":"lagrange-rust-artifact-v1","host_triple":values["PROBE_HOST_TRIPLE"],"input_sha256":values["PROBE_INPUT_SHA256"],"package":"build-cache-fixture-app","platform":"linux/amd64","profile":"release","recipe_sha256":values["PROBE_RECIPE_SHA256"],"source_commit":values["CACHE_FIXTURE_COMMIT"]}
    data=(json.dumps(record,sort_keys=True,separators=(",",":"))+"\n").encode();(destination/"artifact.json").write_bytes(data);(destination/"COMPLETE").write_text(hashlib.sha256(data).hexdigest()+"\n",encoding="ascii")

def create_image(tag,binary,runtime,stdout,commit):
    destination=image_dir(tag)
    if destination.exists():shutil.rmtree(destination)
    destination.mkdir(parents=True);shutil.copyfile(binary,destination/"binary");(destination/"binary").chmod(0o755);shutil.copyfile(runtime,destination/"runtime.txt");(destination/"stdout").write_bytes(stdout.encode())
    ident="sha256:"+hashlib.sha256((hashlib.sha256((destination/"binary").read_bytes()).hexdigest()+hashlib.sha256((destination/"runtime.txt").read_bytes()).hexdigest()+commit).encode()).hexdigest()
    atomic_json(destination/"meta.json",{"id":ident,"revision":commit,"tag":tag})

def record(operation,extra=None):
    value={"argv":args,"cargo_build_jobs":os.environ.get("CARGO_BUILD_JOBS"),"compose_parallel_limit":os.environ.get("COMPOSE_PARALLEL_LIMIT"),"operation":operation}
    if extra:value.update(extra)
    calls.parent.mkdir(parents=True,exist_ok=True)
    with calls.open("a",encoding="utf-8") as out:out.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
    if os.environ.get("FAKE_DOCKER_FAIL_OPERATION")==operation:raise SystemExit(int(os.environ.get("FAKE_DOCKER_FAIL_STATUS","99")))

def layer_path(key):return root/"layers"/key
def cache_key(operation,source,values):return hashlib.sha256(json.dumps({"operation":operation,"source":tree_digest(source),"values":values},sort_keys=True,separators=(",",":" )).encode()).hexdigest()

if args[:2]==["buildx","build"]:
    target=get_option("--target");source=pathlib.Path(args[-1]);destination=output_dest(get_option("--output"));values=build_args();record("buildx",{"build_args":values,"source":str(source),"target":target})
    if get_option("--platform")!="linux/amd64" or values.get("TARGETPLATFORM")!="linux/amd64":raise SystemExit(92)
    if target=="identity":
        destination.mkdir(parents=True,exist_ok=True);(destination/"identity.json").write_bytes(identity_bytes());print("#1 identity export complete");raise SystemExit(0)
    if target!="artifacts" or values.get("PROBE_EXPECTED_NATIVE_SHA256")!=hashlib.sha256(identity_bytes()).hexdigest():raise SystemExit(92)
    layer_key=cache_key("artifacts",source,values);layer=layer_path(layer_key)
    if "--no-cache" not in args and layer.exists():
        if destination.exists():shutil.rmtree(destination)
        shutil.copytree(layer,destination);print("#9 [artifact-builder 5/5] RUN --mount=type=cache,target=/usr/local/cargo/registry");print("#9 CACHED");raise SystemExit(0)
    state,fresh,run_build,keys=compile_observation(source,values,False)
    if "layout-probe-injected" in (source/"fixture-app/src/bin/cache-bin-b.rs").read_text():
        state["pending"]=True;save_state(values["PROBE_TARGET_CACHE_ID"],state);print("error: layout-probe-injected");raise SystemExit(42)
    lines=cargo_lines(values["BIN"],values["VARIANT"],fresh,run_build);counter=int((root/"counter").read_text())+1 if (root/"counter").exists() else 1;(root/"counter").write_text(str(counter));cargo_ms=counter
    write_artifact(destination,source,values,lines,cargo_ms)
    incomplete=values.get("PROBE_INJECT")=="export-fail-p2";complete_compile_state(values["PROBE_TARGET_CACHE_ID"],state,keys,incomplete)
    print(f"BUILD_LAYOUT_CARGO_MS={cargo_ms}")
    if not incomplete:
        if layer.exists():shutil.rmtree(layer)
        shutil.copytree(destination,layer)
    raise SystemExit(0)

if args and args[0]=="compose" and "build" in args:
    service=args[-1];mapping={"probe-a-base":("A_BASE","cache-bin-a","base","web.txt"),"probe-b-base":("B_BASE","cache-bin-b","base","python.txt"),"probe-a-wide":("A_WIDE","cache-bin-a","wide","web.txt"),"probe-b-wide":("B_WIDE","cache-bin-b","wide","python.txt")}
    if service not in mapping or args.count("build")!=1:raise SystemExit(92)
    suffix,bin_name,variant,runtime=mapping[service];source=pathlib.Path(args[args.index("-f")+1]).parent
    values={"BIN":bin_name,"VARIANT":variant,"RUNTIME_FILE":runtime,"CACHE_FIXTURE_COMMIT":os.environ[f"PROBE_COMMIT_{suffix}"],"CACHE_FIXTURE_BUILD_SETTING":os.environ[f"PROBE_SETTING_{suffix}"],"CACHE_FIXTURE_RUN_TOKEN":os.environ[f"PROBE_TOKEN_{suffix}"],"PROBE_TARGET_CACHE_ID":os.environ[f"PROBE_TARGET_CACHE_ID_{suffix}"],"PROBE_SHARED_CACHE_ID":os.environ[f"PROBE_SHARED_CACHE_ID_{suffix}"],"PROBE_CACHE_KEY":os.environ[f"PROBE_CACHE_KEY_{suffix}"],"PROBE_INPUT_SHA256":os.environ[f"PROBE_INPUT_{suffix}"],"PROBE_COMPILE_INPUT_SHA256":os.environ[f"PROBE_COMPILE_INPUT_{suffix}"],"PROBE_LIB_HASH":os.environ[f"PROBE_LIB_{suffix}"],"PROBE_APP_HASH":os.environ[f"PROBE_APP_{suffix}"],"PROBE_RECIPE_SHA256":os.environ[f"PROBE_RECIPE_{suffix}"],"PROBE_HOST_TRIPLE":os.environ[f"PROBE_HOST_{suffix}"],"PROBE_EXPECTED_NATIVE_SHA256":os.environ[f"PROBE_NATIVE_{suffix}"],"PROBE_CARGO_CONFIG":os.environ[f"PROBE_CARGO_CONFIG_{suffix}"],"PROBE_LEDGER_INJECT":os.environ[f"PROBE_LEDGER_INJECT_{suffix}"],"PROBE_INJECT":os.environ[f"PROBE_INJECT_{suffix}"]}
    tag=os.environ[f"PROBE_IMAGE_{suffix}"];overrides=[pathlib.Path(args[i+1]) for i,x in enumerate(args) if x=="-f"][1:];route=None
    if overrides:
        text=overrides[-1].read_text()
        match=re.search(r"release_artifacts: (.+)$",text,re.M)
        route=pathlib.Path(json.loads(match.group(1))) if match else pathlib.Path("source")
    dockerfile="consumer" if overrides else ("baseline" if os.environ.get("PROBE_DOCKERFILE")=="Dockerfile.baseline" else "consumer")
    record("compose-build",{"build_args":values,"dockerfile":dockerfile,"route":"source" if route==pathlib.Path("source") else (str(route) if route else "baseline"),"service":service,"source":str(source),"tag":tag})
    if os.environ.get("CARGO_BUILD_JOBS")!="2" or os.environ.get("COMPOSE_PARALLEL_LIMIT")!="1" or values["PROBE_EXPECTED_NATIVE_SHA256"]!=hashlib.sha256(identity_bytes()).hexdigest():raise SystemExit(92)
    if dockerfile=="consumer" and route is not None and route!=pathlib.Path("source"):
        if os.environ[f"PROBE_SOURCE_TRAP_{suffix}"]!="1":raise SystemExit(92)
        binary=route/"bin"/bin_name;runtime_path=route/"runtime.txt";stdout=expected_stdout(source,bin_name,variant,values["CACHE_FIXTURE_COMMIT"],values["CACHE_FIXTURE_BUILD_SETTING"])
        create_image(tag,binary,runtime_path,stdout,values["CACHE_FIXTURE_COMMIT"]);print("# consumer verified-artifacts complete");raise SystemExit(0)
    if dockerfile=="consumer" and route==pathlib.Path("source") and os.environ[f"PROBE_SOURCE_TRAP_{suffix}"]!="0":print("source trap executed");raise SystemExit(70)
    layer_key=cache_key("compose-"+dockerfile,source,values);layer=layer_path(layer_key)
    if "--no-cache" not in args and layer.exists():
        if image_dir(tag).exists():shutil.rmtree(image_dir(tag))
        shutil.copytree(layer/"image",image_dir(tag));print("#12 [builder 5/5] RUN --mount=type=cache,target=/usr/local/cargo/registry");print("#12 CACHED");raise SystemExit(0)
    state,fresh,run_build,keys=compile_observation(source,values,True)
    if "layout-probe-injected" in (source/"fixture-app/src/bin/cache-bin-b.rs").read_text():save_state(values["PROBE_TARGET_CACHE_ID"],state);print("error: layout-probe-injected");raise SystemExit(42)
    lines=cargo_lines(bin_name,variant,fresh,run_build)
    prefix=f"{service} " if values.get("PROBE_INJECT")=="export-fail-p2" else ""
    print(f"#12 [{prefix}builder 5/5] RUN --mount=type=cache,target=/usr/local/cargo/registry")
    for line in lines:sys.stdout.write("#12 0.001 "+line)
    counter=int((root/"counter").read_text())+1 if (root/"counter").exists() else 1;(root/"counter").write_text(str(counter));print(f"#12 0.002 BUILD_LAYOUT_CARGO_MS={counter}")
    complete_compile_state(values["PROBE_TARGET_CACHE_ID"],state,keys,False)
    inject=values.get("PROBE_INJECT","none")
    if inject=="export-fail-p2":
        print("#12 0.003 BUILD_LAYOUT_INJECTED_EXPORT_FAILURE bin=cache-bin-b inner_exit=73 cargo_success=true")
        print('#12 ERROR: process "/bin/bash -o pipefail -c <fixture RUN>" did not complete successfully: exit code: 73')
        record("injected-export-failure",{"boundary":"baseline-run","client_exit":1,"inner_exit":73,"cargo_success":True})
        raise SystemExit(1)
    if inject in ("wrong-commit","missing-bin","tampered-bin","missing-complete"):print("baseline staging verification rejected "+inject);raise SystemExit(65)
    stdout=expected_stdout(source,bin_name,variant,values["CACHE_FIXTURE_COMMIT"],values["CACHE_FIXTURE_BUILD_SETTING"]);temporary=root/"tmp-binary";write_binary(temporary,stdout);create_image(tag,temporary,source/"runtime"/runtime,stdout,values["CACHE_FIXTURE_COMMIT"]);temporary.unlink()
    layer.mkdir(parents=True,exist_ok=True);shutil.copytree(image_dir(tag),layer/"image")
    raise SystemExit(0)

if args[:2]==["image","inspect"]:
    record("image-inspect");tag=args[-1];directory=image_dir(tag)
    if not (directory/"meta.json").exists():
        print("[]");print(f"Error response from daemon: No such image: {tag}",file=sys.stderr);raise SystemExit(1)
    if "--format" in args:
        expected=r'{{.Id}}{{printf "\t"}}{{index .Config.Labels "org.opencontainers.image.revision"}}'
        if args!=["image","inspect","--format",expected,tag]:raise SystemExit(92)
        meta=json.loads((directory/"meta.json").read_text());print(meta["id"]+"\t"+meta["revision"])
    elif args!=["image","inspect",tag]:raise SystemExit(92)
    raise SystemExit(0)

if args and args[0]=="run":
    record("run");entry=None;index=1
    flags={"--rm","--read-only"};takes={"--network","--cap-drop","--security-opt","--user","--entrypoint"}
    while index<len(args):
        if args[index] in flags:index+=1;continue
        if args[index].startswith("--cap-drop=") or args[index].startswith("--security-opt=") or args[index].startswith("--user=") or args[index].startswith("--network="):index+=1;continue
        if args[index] in takes:
            if args[index]=="--entrypoint":entry=args[index+1]
            index+=2;continue
        break
    tag=args[index];rest=args[index+1:];directory=image_dir(tag)
    if not directory.exists():raise SystemExit(1)
    if entry is None:sys.stdout.buffer.write((directory/"stdout").read_bytes())
    elif entry=="/bin/cat" and rest==["/opt/fixture/runtime.txt"]:sys.stdout.buffer.write((directory/"runtime.txt").read_bytes())
    elif entry=="/usr/bin/sha256sum" and len(rest)==1 and re.fullmatch(r"/usr/local/bin/cache-bin-[ab]",rest[0]):print(hashlib.sha256((directory/"binary").read_bytes()).hexdigest()+"  "+rest[0])
    else:raise SystemExit(92)
    raise SystemExit(0)

record("unknown")
raise SystemExit(92)
PY
  chmod 0755 -- "$path"
}

self_gate_boundaries() {
  local root=$1 proc bin gate calls
  proc="$root/proc" bin="$root/gate-bin" gate="$root/gates.jsonl" calls="$root/gate-calls.log"
  mkdir -p -m 0700 -- "$proc/sys/kernel/random" "$bin"
  printf 'MemAvailable: 8388608 kB\nSwapFree: 1048576 kB\n' >"$proc/meminfo"
  printf '01234567-89ab-cdef-0123-456789abcdef\n' >"$proc/sys/kernel/random/boot_id"
  printf '%s\n' '#!/bin/sh' 'printf "ps\t%s\n" "$*" >>"$GATE_CALLS"' 'exit 0' >"$bin/ps"
  printf '%s\n' '#!/bin/sh' 'printf "systemctl\t%s\n" "$*" >>"$GATE_CALLS"' '[ "$1" = show ] && [ "$3" = --property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts ] || exit 90' 'printf "%s\n" "NRestarts=0" "MainPID=123" "SubState=running" "LoadState=loaded" "ExecMainStatus=0" "ActiveState=active"' >"$bin/systemctl"
  printf '%s\n' '#!/bin/sh' 'printf "journalctl\t%s\n" "$*" >>"$GATE_CALLS"' 'python3 -c '"'"'import datetime,json,sys,time;args=sys.argv[1:];until=args[args.index("--until")+1] if "--until" in args else None;t=int(datetime.datetime.strptime(until,"%Y-%m-%d %H:%M:%S UTC").replace(tzinfo=datetime.timezone.utc).timestamp()*1000000)-500000 if until is not None else time.time_ns()//1000;print(json.dumps({"__REALTIME_TIMESTAMP":str(t),"__CURSOR":"fake","_BOOT_ID":"0123456789abcdef0123456789abcdef","_TRANSPORT":"kernel","MESSAGE":"quiet kernel"},separators=(",",":")))'"'"' "$@"' >"$bin/journalctl"
  printf '%s\n' '#!/bin/sh' 'printf "docker\t%s\n" "$*" >>"$GATE_CALLS"' '[ "$1" = inspect ] && [ "$2" = --type ] && [ "$3" = container ] && [ "$4" = --format ] || exit 90' 'printf "%s\ttrue\tfalse\tfalse\thealthy\t0\tlagrange-station\n" 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' >"$bin/docker"
  chmod 0755 "$bin/ps" "$bin/systemctl" "$bin/journalctl" "$bin/docker"
  GATE_CALLS="$calls" BUILD_LAYOUT_SELF_TEST_ACTIVE=1 LAYOUT_GATE_TEST_SEAM=build-layout-self-test LAYOUT_GATE_MODE=real LAYOUT_GATE_PROC_ROOT="$proc" LAYOUT_GATE_PS_BIN="$bin/ps" LAYOUT_GATE_SYSTEMCTL_BIN="$bin/systemctl" LAYOUT_GATE_JOURNALCTL_BIN="$bin/journalctl" LAYOUT_GATE_DOCKER_BIN="$bin/docker" LAYOUT_GATE_RECORD_FILE="$gate" LAYOUT_GATE_STATE_FILE="$root/gate-state.json" LAYOUT_GATE_PRIVATE_DIR="$root/private" BUILD_LAYOUT_HEALTH_UNITS='alpha.service,beta.service' BUILD_LAYOUT_HEALTH_CONTAINERS='lagrange-station-research-worker-1,api-1' probe_gate cold fake-boundary 0
  self_expect_failure gate_validate_list 'a.service,a.service' unit
  self_expect_failure gate_validate_list 'a.service,' unit
  self_expect_failure gate_validate_list $'a.service\nb.service' unit
  self_expect_failure gate_validate_list $'lagrange-station-research-worker-1\nignored' container
  self_expect_failure gate_validate_list 'a.service' container
  printf '%s\n' '{"__REALTIME_TIMESTAMP":"1","__CURSOR":"x","_BOOT_ID":11111111111111111111111111111111,"_TRANSPORT":"kernel","MESSAGE":"x"}' >"$root/numeric-boot.json"
  self_expect_failure gate_parse_probe "$root/numeric-boot.json" 11111111111111111111111111111111
  self_expect_failure gate_parse_range "$root/numeric-boot.json" 11111111111111111111111111111111 0 2
  BUILD_LAYOUT_SELF_TEST_ACTIVE=1 LAYOUT_GATE_TEST_SEAM=build-layout-self-test LAYOUT_GATE_MODE=fake LAYOUT_GATE_RECORD_FILE="$root/fake-prior.jsonl" probe_gate cold fake-previous 1 >/dev/null 2>&1 && return 1
  python3 - "$gate" "$calls" <<'PY'
import json,sys
record=json.loads(open(sys.argv[1],encoding="utf-8").read().splitlines()[-1]);calls=open(sys.argv[2],encoding="utf-8").read()
if record["status"]!="PASS" or record["reason"]!="healthy":raise SystemExit(1)
journal=record["journal"]
if journal["probe"]["exit"]!=0 or journal["range"]["exit"]!=0 or journal["count"]!=1 or journal["oom_count"]!=0:raise SystemExit(1)
capture=journal["range"]["capture"]
if capture["complete"] is not True or capture["stdout_eof"] is not True or capture["stderr_eof"] is not True or capture["child_reaped"] is not True or capture["record_count"]!=1:raise SystemExit(1)
if capture["argv"][1:5]!=["-k","-b","--no-pager","-o"] or capture["argv"][-1]!="--no-tail" or "-n" in capture["argv"]:raise SystemExit(1)
if set(record["evidence"]["units"]["alpha.service"]["selected"])!={"active_state","exec_main_status","load_state","main_pid","n_restarts","sub_state"}:raise SystemExit(1)
if set(record["evidence"]["containers"]["api-1"]["selected"])!={"health_status","id","oom_killed","project","restart_count","restarting","running"}:raise SystemExit(1)
if "quiet kernel" in open(sys.argv[1],encoding="utf-8").read():raise SystemExit(1)
if "journalctl\t-k -b --no-pager -o json -n 1" not in calls or "--since" not in calls or "systemctl\tshow alpha.service --property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts" not in calls or "docker\tinspect --type container --format" not in calls:raise SystemExit(1)
PY
}

reset_runtime_globals_for_resume_test() {
  LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=
  run_dir= run_id= events_file= gate_file= run_json= timing_file=
  layout_selection=all case_selection=all output_dir=
  CUR_LAYOUT= CUR_CASE= CUR_SCOPE= CUR_TOKEN= CUR_SOURCE= CUR_ATTEMPT=0 CUR_ATTEMPT_DIR= CUR_COMMIT= CUR_SETTING= CUR_CONFIG= CUR_COLD=0 CUR_PHASE=
  CUR_COMPILE= CUR_LIB= CUR_APP= CUR_RUNTIME= CUR_RECIPE= CUR_GUARD= CUR_K= CUR_H= CUR_HOST= CUR_IDENTITY_SHA= CUR_SHARED_CACHE= CUR_TARGET_CACHE= CUR_LEDGER_INJECT=none CUR_SOURCE_TRAP=0
  RESUME_ACTIVE=0 RESUMING=0 RESUME_COMPLETE=0
  unset LAYOUT_JOURNAL_SINCE_US LAYOUT_JOURNAL_SINCE LAYOUT_JOURNAL_BOOT_ID LAYOUT_GATE_RECORD_FILE LAYOUT_GATE_STATE_FILE LAYOUT_GATE_PRIVATE_DIR
}

self_controller() {
  local root=$1 layout=$2 case_id=$3 status build_calls_before build_calls_after controller_run_dir controller_run_id
  layout_selection=$layout; case_selection=$case_id; output_dir="$root/$layout-$case_id"; resume_dir=
  controller_run_dir=$output_dir
  init_run
  controller_run_id=$run_id
  if run_suite; then status=0; else status=$?; fi
  if expected_stop_case "$case_id"; then
    [ "$status" = 75 ] || return 1
    resume_dir=$controller_run_dir; reset_runtime_globals_for_resume_test
    load_resume_state || return $?
    [ "$layout_selection" = "$layout" ] && [ "$case_selection" = "$case_id" ] && [ "$run_id" = "$controller_run_id" ] || return 1
    probe_gate "$RESUME_CASE" resume-before 0 || return 1
    validate_all_saved_images || return 1
    probe_gate "$RESUME_CASE" resume-after 0 || return 1
    RESUME_ACTIVE=1
    if run_suite; then status=0; else status=$?; fi
    [ "$status" = 0 ] || return 1
  else
    [ "$status" = 0 ] || return 1
  fi
  build_calls_before=$(python3 - "${FAKE_DOCKER_CALLS:?}" <<'PY'
import json,sys
print(sum(1 for line in open(sys.argv[1]) if json.loads(line).get("operation") in ("buildx","compose-build")))
PY
)
  resume_dir=$controller_run_dir; reset_runtime_globals_for_resume_test
  load_resume_state || return 1
  [ "$run_id" = "$controller_run_id" ] || return 1
  [ "$RESUME_COMPLETE" = 1 ] || return 1
  probe_gate complete resume-before 0 || return 1
  validate_all_saved_images || return 1
  probe_gate complete resume-after 0 || return 1
  build_calls_after=$(python3 - "${FAKE_DOCKER_CALLS:?}" <<'PY'
import json,sys
print(sum(1 for line in open(sys.argv[1]) if json.loads(line).get("operation") in ("buildx","compose-build")))
PY
)
  [ "$build_calls_before" = "$build_calls_after" ] || { unresolved 'already-complete resume invoked a build'; return 1; }
}

self_wrapper_boundaries() {
  local root=$1 source identity status saved_docker=$docker_bin
  source="$root/source" identity="$root/identity"
  mkdir -p -m 0700 -- "$root/logs" "$root/results" "$root/run/artifacts"
  source_copy "$source" || return 1
  run_dir="$root/run" run_id=wrapper CUR_LAYOUT=common CUR_CASE=cold CUR_SCOPE=warm-chain CUR_TOKEN=cold CUR_SOURCE=$source CUR_ATTEMPT=1 CUR_ATTEMPT_DIR=$root CUR_COMMIT=$c1 CUR_SETTING=default CUR_CONFIG=default CUR_COLD=1 CUR_PHASE=p1 CUR_LEDGER_INJECT=none RESUMING=0
  identity_build "$source" "$identity" "$root/logs/identity-ok.log" || return 1
  compute_inputs "$identity/identity.json" || return 1
  docker_bin=/bin/false
  if identity_build "$source" "$root/false-identity" "$root/logs/identity-false.log"; then return 1; else status=$?; fi
  [ "$status" -eq 1 ] || return 1
  if artifact_build p1 "$root/false-artifact" "$root/logs/artifact-false.log"; then return 1; else status=$?; fi
  [ "$status" -eq 1 ] || return 1
  if compose_build p1 probe-a-base wrapper-false baseline "$root/logs/compose-false.log"; then return 1; else status=$?; fi
  [ "$status" -eq 1 ] || return 1
  [ -z "$(find "$root/run/artifacts" -mindepth 1 -print -quit)" ] || return 1
  docker_bin=$saved_docker
  if FAKE_DOCKER_FAIL_OPERATION=buildx FAKE_DOCKER_FAIL_STATUS=37 identity_build "$source" "$root/status-identity" "$root/logs/identity-status.log"; then return 1; else status=$?; fi
  [ "$status" -eq 37 ] || return 1
  if FAKE_DOCKER_FAIL_OPERATION=buildx FAKE_DOCKER_FAIL_STATUS=38 artifact_build p1 "$root/status-artifact" "$root/logs/artifact-status.log"; then return 1; else status=$?; fi
  [ "$status" -eq 38 ] || return 1
  if FAKE_DOCKER_FAIL_OPERATION=compose-build FAKE_DOCKER_FAIL_STATUS=39 compose_build p1 probe-a-base wrapper-status baseline "$root/logs/compose-status.log"; then return 1; else status=$?; fi
  [ "$status" -eq 39 ] || return 1
}

self_publication_contract() {
  local root=$1 private=$2 first second a b p1 p2 changed_a changed_b
  changed_a="$root/evidence-a" changed_b="$root/evidence-b"
  first=$(bash "$helper_path" artifact-publish "$private" "$root/duplicate") || return 1
  second=$(bash "$helper_path" artifact-publish "$private" "$root/duplicate") || return 1
  [ "$first" = "$second" ] && [ "$(basename "$first")" = "$(bash "$helper_path" tree-hash "$private")" ] || return 1
  bash "$helper_path" artifact-publish "$private" "$root/concurrent" >"$root/concurrent-a.log" 2>&1 & p1=$!
  bash "$helper_path" artifact-publish "$private" "$root/concurrent" >"$root/concurrent-b.log" 2>&1 & p2=$!
  if wait "$p1"; then :; else return 1; fi
  if wait "$p2"; then :; else return 1; fi
  cmp -s -- "$root/concurrent-a.log" "$root/concurrent-b.log" || return 1
  cp -a -- "$private" "$changed_a"; cp -a -- "$private" "$changed_b"
  PROBE_PATH="$changed_b/timing.json" python3 - <<'PY'
import json,os
p=os.environ["PROBE_PATH"];v=json.load(open(p));v["cargo_ms"]+=17;v["finished_ms"]+=17
with open(p,"w",encoding="utf-8",newline="\n") as out:out.write(json.dumps(v,sort_keys=True,separators=(",",":"))+"\n")
PY
  a=$(bash "$helper_path" artifact-publish "$changed_a" "$root/evidence") || return 1
  b=$(bash "$helper_path" artifact-publish "$changed_b" "$root/evidence") || return 1
  [ "$a" != "$b" ] && cmp -s -- "$changed_a/artifact.json" "$changed_b/artifact.json" && cmp -s -- "$changed_a/bin/$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["bin"])' "$changed_a/artifact.json")" "$changed_b/bin/$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["bin"])' "$changed_b/artifact.json")" || return 1
  [ "$(find "$root/evidence" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 2 ] || return 1
}

self_cargo_negative_contract() {
  local root=$1 valid=$2 host=x86_64-unknown-linux-musl summary
  summary="$root/summary.json"
  mkdir -p -m 0700 -- "$root"
  bash "$helper_path" cargo-summary "$valid" cache-bin-a "$summary" "$host" || return 1
  bash "$helper_path" cargo-assert "$summary" cache-bin-a '[]' '["default"]' "$host" false false false || return 1
  PROBE_SOURCE="$valid" PROBE_ROOT="$root" python3 - <<'PY'
import json,os,pathlib
source=pathlib.Path(os.environ["PROBE_SOURCE"]).read_text().splitlines(True);root=pathlib.Path(os.environ["PROBE_ROOT"])
def write(name,transform):
    out=[]
    for line in source:
        try:value=json.loads(line)
        except json.JSONDecodeError:out.append(line);continue
        value=transform(value)
        if value is not None:out.append(json.dumps(value,separators=(",",":"))+"\n")
    (root/name).write_text("".join(out),encoding="utf-8")
write("wrong-version.jsonl",lambda v:{**v,"package_id":v["package_id"].replace("itoa@1.0.18","itoa@1.0.180")} if v.get("package_id","").endswith("itoa@1.0.18") else v)
write("wrong-source.jsonl",lambda v:{**v,"package_id":v["package_id"].replace("github.com/rust-lang/crates.io-index","index.invalid")} if "crates.io-index#itoa" in v.get("package_id","") else v)
write("missing-build-unit.jsonl",lambda v:None if v.get("reason")=="compiler-artifact" and v.get("target",{}).get("kind")==["custom-build"] else v)
write("missing-build-event.jsonl",lambda v:None if v.get("reason")=="build-script-executed" else v)
def no_default(v):
    if v.get("package_id","").startswith("path+file:///build/"):v={**v,"features":[]}
    return v
write("missing-default.jsonl",no_default)
def bad_profile(v):
    if v.get("reason")=="compiler-artifact" and v.get("target",{}).get("kind")==["lib"] and "fixture-lib" in v.get("package_id",""):v={**v,"profile":{**v["profile"],"opt_level":"0"}}
    return v
write("bad-profile.jsonl",bad_profile)
(root/"missing-run.jsonl").write_text("".join(line for line in source if "Running `/cargo-target/release/build/" not in line),encoding="utf-8")
PY
  self_expect_failure bash "$helper_path" cargo-summary "$root/wrong-version.jsonl" cache-bin-a "$root/a.json" "$host"
  self_expect_failure bash "$helper_path" cargo-summary "$root/wrong-source.jsonl" cache-bin-a "$root/b.json" "$host"
  self_expect_failure bash "$helper_path" cargo-summary "$root/missing-build-unit.jsonl" cache-bin-a "$root/c.json" "$host"
  self_expect_failure bash "$helper_path" cargo-summary "$root/missing-build-event.jsonl" cache-bin-a "$root/d.json" "$host"
  bash "$helper_path" cargo-summary "$root/missing-default.jsonl" cache-bin-a "$root/e.json" "$host" || return 1
  self_expect_failure bash "$helper_path" cargo-assert "$root/e.json" cache-bin-a '[]' '["default"]' "$host" false false false
  bash "$helper_path" cargo-summary "$root/bad-profile.jsonl" cache-bin-a "$root/f.json" "$host" || return 1
  self_expect_failure bash "$helper_path" cargo-assert "$root/f.json" cache-bin-a '[]' '["default"]' "$host" false false false
  bash "$helper_path" cargo-summary "$root/missing-run.jsonl" cache-bin-a "$root/g.json" "$host" || return 1
  bash "$helper_path" cargo-assert "$root/g.json" cache-bin-a '[]' '["default"]' "$host" false false false
}

self_identity_contract() {
  local root=$1 identity=$2 path status
  mkdir -p -m 0700 -- "$root"
  python3 - "$root" "$identity" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);source=json.load(open(sys.argv[2]))
valid=["cargo 1.97.1","cargo 1.97.1 (012345678 2026-09-01)","cargo 1.98.0-beta.1 (abcdef012 2026-09-02)","cargo 1.99.0-nightly (123abcdef 2026-09-03)","cargo 1.97.1+local (012345678 2026-09-01)"]
invalid=["cargo 1.97", "cargo 1.97.1\n", "cargo 1.97.1\rcargo 2.0.0", "cargo 1.97.1 (012345678 2026-02-30)","cargo 1.97.1 (not-a-hash 2026-09-01)","cargo 1.97.1 (012345678)","cargo 1.97.1 extra", "cargo 1.97.1-nightly\nextra", "cargo 1.97.1-", "cargo 1.97.1 (012345678 2026-09-01) extra", None, 1971]
for kind,values in (("valid",valid),("invalid",invalid)):
    for n,value in enumerate(values):
        (root/f"{kind}-{n}.json").write_text(json.dumps({**source,"cargo_version":value})+"\n")
PY
  for path in "$root"/valid-*.json; do
    local before; before=$(sha256_file "$path")
    bash "$helper_path" identity-validate "$path" linux/amd64 >"$path.out" || return 1
    [ "$before" = "$(sha256_file "$path")" ] || return 1
  done
  for path in "$root"/invalid-*.json; do
    if bash "$helper_path" identity-validate "$path" linux/amd64 >"$path.out" 2>"$path.err"; then return 1; else status=$?; fi
    [ "$status" -eq 1 ] || return 1
  done
}

self_execution_slot() {
  local root=$1
  mkdir -p -m 0700 -- "$root"
  PROBE_LOCK_FUNCTIONS="$(declare -f acquire_execution_slot gate_test_allowed unresolved)" python3 - "$root" <<'PY'
import os,pathlib,select,signal,subprocess,sys
root=pathlib.Path(sys.argv[1]);script=root/"lock-boundary.sh"
script.write_text('set -euo pipefail\nexecution_lock_fd=\ninternal_self_test=1\n'+os.environ["PROBE_LOCK_FUNCTIONS"]+'\nacquire_execution_slot "$1" || exit 1\nprintf "held\\n"\nif [ "$2" = hold ]; then read -r release; fi\nprintf "fake-host-boundary\\n"\n')
env={**os.environ,"BUILD_LAYOUT_SELF_TEST_ACTIVE":"1","LAYOUT_GATE_TEST_SEAM":"build-layout-self-test"}
for ending in ("normal","signal"):
    slot=root/ending
    holder=subprocess.Popen(["bash",str(script),str(slot),"hold"],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        if not select.select([holder.stdout],[],[],10)[0] or holder.stdout.readline()!="held\n":raise SystemExit("lock-holder-not-ready")
        contender=subprocess.run(["bash",str(script),str(slot),"once"],env=env,capture_output=True,text=True,timeout=10)
        (root/f"{ending}-contention.log").write_text(contender.stdout+contender.stderr)
        if contender.returncode!=1 or "fake-host-boundary" in contender.stdout or "execution-slot-busy" not in contender.stderr:raise SystemExit("lock-contention-failed")
        if ending=="normal":holder.stdin.write("release\n");holder.stdin.flush()
        else:holder.send_signal(signal.SIGTERM)
        holder.communicate(timeout=10)
        released=subprocess.run(["bash",str(script),str(slot),"once"],env=env,capture_output=True,text=True,timeout=10)
        (root/f"{ending}-release.log").write_text(released.stdout+released.stderr)
        if released.returncode!=0 or released.stdout!="held\nfake-host-boundary\n":raise SystemExit("lock-release-failed")
    finally:
        if holder.poll() is None:holder.kill();holder.communicate()
bad=root/"bad-mode";bad.mkdir(mode=0o755)
link=root/"link";link.symlink_to(root/"normal",target_is_directory=True)
for path in (bad,link):
    result=subprocess.run(["bash",str(script),str(path),"once"],env=env,capture_output=True,text=True,timeout=10)
    if result.returncode!=1 or "fake-host-boundary" in result.stdout:raise SystemExit("unsafe-lock-path-accepted")
print("F2 PASS: contention before host boundary; normal/signal release; unsafe paths rejected")
PY
}

self_run_namespace() {
  local root=$1
  mkdir -p -m 0700 -- "$root/left" "$root/right"
  self_controller "$root/left" common cold || return 1
  self_controller "$root/right" common cold || return 1
  python3 - "$root" <<'PY'
import json,pathlib,re,sys
root=pathlib.Path(sys.argv[1]);runs=[root/parent/"common-cold" for parent in ("left","right")]
ids=[json.load(open(run/"run.json"))["run_id"] for run in runs]
if ids[0]==ids[1] or any(not re.fullmatch(r"[0-9a-f]{32}",item) for item in ids):raise SystemExit("run-id-collision")
namespaces=[];tags=[]
for run,ident in zip(runs,ids):
    phases=[json.load(open(p)) for p in (run/"state").glob("*/*/*/p[1-4].json")]
    namespaces.append({p["target_namespace"] for p in phases});tags.append({p["tag"] for p in phases})
    if len(phases)!=4 or any(not p["tag"].startswith("build-layout-"+ident+"-") for p in phases):raise SystemExit("run-tag-identity-invalid")
if namespaces[0]&namespaces[1] or tags[0]&tags[1]:raise SystemExit("run-namespace-collision")
print("F1 PASS: same basename, distinct IDs/tags/caches; fresh-state resume retains each ID")
PY
}

self_post_failure_gate() (
  local root=$1 gate_root=$2 outcome status
  export LAYOUT_GATE_MODE=real LAYOUT_GATE_PROC_ROOT="$gate_root/proc" LAYOUT_GATE_PS_BIN="$gate_root/gate-bin/ps" LAYOUT_GATE_SYSTEMCTL_BIN="$gate_root/gate-bin/systemctl" LAYOUT_GATE_JOURNALCTL_BIN="$gate_root/gate-bin/journalctl" LAYOUT_GATE_DOCKER_BIN="$gate_root/gate-bin/docker" GATE_CALLS="$gate_root/gate-calls.log"
  mkdir -p -m 0700 -- "$root"
  for outcome in pass fail; do
    reset_runtime_globals_for_resume_test
    output_dir="$root/$outcome" layout_selection=common case_selection=export-fail-p2
    init_run
    CUR_LAYOUT=common CUR_CASE=export-fail-p2 CUR_SCOPE=export-fail-p2 CUR_TOKEN=export-fail-p2-trial-1 CUR_ATTEMPT=1 CUR_ATTEMPT_DIR="$run_dir/attempt-001" CUR_SOURCE="$run_dir/source"
    source_copy "$CUR_SOURCE" || exit 1
    mkdir -p -m 0700 -- "$CUR_ATTEMPT_DIR/results"
    if [ "$outcome" = fail ]; then printf 'MemAvailable: 1 kB\nSwapFree: 1048576 kB\n' >"$gate_root/proc/meminfo"; fi
    if expected_stop p2 37 verified-injected-cause "build-layout-$run_id-absent"; then exit 1; else status=$?; fi
    printf 'MemAvailable: 8388608 kB\nSwapFree: 1048576 kB\n' >"$gate_root/proc/meminfo"
    if [ "$outcome" = pass ]; then [ "$status" -eq 75 ] || exit 1; set_run_status expected-stop 75
    else [ "$status" -eq 1 ] || exit 1; set_run_status failed 1; [ "$(resume_terminal)" = FAILED ] || exit 1
    fi
  done
  python3 - "$root" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
for kind in ("pass","fail"):
    events=[json.loads(x) for x in (root/kind/"events.jsonl").read_text().splitlines()]
    gates=[json.loads(x) for x in (root/kind/"gates.jsonl").read_text().splitlines()]
    cause=next(v for v in events if v["type"]=="EXPECTED_CAUSE")
    if cause["exit"]!=37 or len(gates)!=1 or gates[0]["previous_exit"]!=0:raise SystemExit("post-failure-status-lost")
    if gates[0]["status"]!=("PASS" if kind=="pass" else "FAIL"):raise SystemExit("post-failure-gate-wrong-result")
    stops=[v for v in events if v["type"]=="EXPECTED_STOP"]
    if (kind=="pass" and (len(stops)!=1 or stops[0]["exit"]!=37)) or (kind=="fail" and stops):raise SystemExit("failed-gate-resumable")
print("F4 PASS: real gate/parser with fake executables, preserved original status, failed gate non-resumable")
PY
)

self_image_inspection() (
  local root=$1 saved_run=$2 tag status saved_docker=$docker_bin
  mkdir -p -m 0700 -- "$root"
  tag=$(python3 - "$saved_run/state/common/warm-chain/cold/p1.json" <<'PY'
import json,sys
print(json.load(open(sys.argv[1]))["tag"])
PY
)
  if "$docker_bin" image inspect --format '{{.Id}}{{printf "\\t"}}{{index .Config.Labels "org.opencontainers.image.revision"}}' "$tag" >"$root/bad-format.out" 2>"$root/bad-format.err"; then exit 1; else status=$?; fi
  [ "$status" -eq 92 ] || exit 1
  self_expect_failure image_absent "$tag" "$root/present" || exit 1
  image_absent build-layout-missing "$root/missing" || exit 1
  for status in 1 7 125; do
    self_expect_failure env FAKE_DOCKER_FAIL_OPERATION=image-inspect FAKE_DOCKER_FAIL_STATUS="$status" "$docker_bin" image inspect build-layout-missing || exit 1
    if FAKE_DOCKER_FAIL_OPERATION=image-inspect FAKE_DOCKER_FAIL_STATUS="$status" image_absent build-layout-missing "$root/unknown-$status"; then exit 1; fi
  done
  docker_bin="$root/inspection-error"
  printf '%s\n' '#!/bin/sh' 'printf "[]\n"' 'printf "%s\n" "Error response from daemon: permission denied" >&2' 'exit 1' >"$docker_bin"
  chmod 0755 -- "$docker_bin"
  self_expect_failure image_absent build-layout-missing "$root/permission" || exit 1
  python3 - "$root" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
for name,status,cause in [("missing",1,"missing-image"),("present",0,"image-present"),("permission",1,"inspection-failed")]+[(f"unknown-{s}",s,"inspection-failed") for s in (1,7,125)]:
    v=json.load(open(root/name/"result.json"))
    if v["command_exit"]!=status or v["cause"]!=cause:raise SystemExit("inspection-cause-lost")
print("F6/F7 PASS: exact template enforced; only precise missing image proves absence")
PY
)

self_export_failures() (
  local root=$1 layout path variant status
  mkdir -p -m 0700 -- "$root/runs" "$root/positive" "$root/negative"
  for layout in baseline common grouped; do self_controller "$root/runs" "$layout" export-fail-p2 || exit 1; done
  python3 - "$root/runs" <<'PY'
import json,pathlib,sys
for layout in ("baseline","common","grouped"):
    path=pathlib.Path(sys.argv[1])/f"{layout}-export-fail-p2"/layout/"cases/export-fail-p2/attempt-001/results/p2-export-failure.json"
    v=json.load(open(path));status=1 if layout=="baseline" else 73
    if v["outer_status"]!=status or v["inner_status"]!=73 or v["cargo_success"] is not True:raise SystemExit("export-status-evidence-invalid")
    if v["command_status"]!=({"compose_exit":1} if layout=="baseline" else {"buildx_exit":0,"helper_exit":73}):raise SystemExit("export-boundary-conflated")
PY
  path="$root/runs/baseline-export-fail-p2/baseline/cases/export-fail-p2/attempt-001/logs/p2.log"
  python3 - "$path" "$root" "$(service_for p2)" <<'PY'
import json,pathlib,sys
source=pathlib.Path(sys.argv[1]).read_text();root=pathlib.Path(sys.argv[2]);service=sys.argv[3]
header=f"#12 [{service} builder 5/5] RUN --mount=type=cache,target=/usr/local/cargo/registry"
if source.splitlines().count(header)!=1:raise SystemExit("fake-compose-service-prefix-missing")
positive={"compose-prefix":source,"unprefixed":source.replace(f"[{service} builder ","[builder ")}
negative={"no-marker":source.replace("BUILD_LAYOUT_INJECTED_EXPORT_FAILURE","REMOVED"),"wrong-vertex":source.replace("#12 ERROR:","#13 ERROR:"),"wrong-header-vertex":source.replace(header,header.replace("#12 ","#13 ")),"wrong-service":source.replace(f"[{service} builder ","[probe-a-base builder "),"wrong-stage":source.replace(f"[{service} builder ",f"[{service} runtime "),"wrong-exit":source.replace("exit code: 73","exit code: 74"),"cargo-failed":source.replace('"success":true','"success":false'),"wrong-bin":source.replace("FAILURE bin=cache-bin-b","FAILURE bin=cache-bin-a"),"wrong-client-status":source,"daemon":"Error response from daemon: connection failed\n"}
for category,variants in (("positive",positive),("negative",negative)):
    for name,text in variants.items():
        p=root/category/name;p.mkdir();(p/"build.log").write_text(text)
        (p/"build.log.status.json").write_text(json.dumps({"compose_exit":73 if name=="wrong-client-status" else 1})+"\n")
PY
  CUR_LAYOUT=baseline CUR_CASE=export-fail-p2 CUR_TOKEN=export-fail-p2-trial-1 CUR_HOST=x86_64-unknown-linux-musl RESUMING=0
  for variant in "$root/positive"/*; do
    CUR_ATTEMPT_DIR=$variant; mkdir -p -m 0700 -- "$variant/results"
    if verify_export_failure p2 1 "$variant/build.log" >"$variant/check.out" 2>"$variant/check.err"; then status=0; else status=$?; fi
    printf '%s\n' "$status" >"$variant/check.exit"
    [ "$status" -eq 0 ] || exit 1
  done
  for variant in "$root/negative"/*; do
    CUR_ATTEMPT_DIR=$variant; mkdir -p -m 0700 -- "$variant/results"
    if verify_export_failure p2 1 "$variant/build.log" >"$variant/check.out" 2>"$variant/check.err"; then status=0; else status=$?; fi
    printf '%s\n' "$status" >"$variant/check.exit"
    [ "$status" -eq 1 ] || exit 1
  done
  printf '%s\n' 'F3 PASS: baseline client=1 / RUN=73; export client=0 / helper=73; unrelated failures rejected; resume PASS'
  printf '%s\n' 'F9 PASS: exact Compose service prefix and unprefixed BuildKit accepted; wrong service/stage/vertex and unrelated failures rejected'
)

self_script_observations() {
  local root=$1 log=$2 path status summary host=x86_64-unknown-linux-musl
  mkdir -p -m 0700 -- "$root"
  python3 - "$root" "$log" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);lines=pathlib.Path(sys.argv[2]).read_text().splitlines(True)
script=next(line for line in lines if line.startswith("Running `"));without=[line for line in lines if line!=script]
for fresh in (False,True):
    for count in (0,1,2):
        output=[]
        for line in without:
            if line.startswith("{"):
                value=json.loads(line)
                if value.get("reason")=="compiler-artifact" and value["target"]["kind"]==["custom-build"]:value["fresh"]=fresh
                line=json.dumps(value,separators=(",",":"))+"\n"
            output.append(line)
        (root/f"valid-{fresh}-{count}.jsonl").write_text("".join([script]*count+output))
(root/"missing-verbose.jsonl").write_text("".join(line for line in lines if line.startswith("{")))
for name in ("cache-bin-a","build_cache_fixture_lib","itoa"):
    output=[]
    for line in lines:
        if line.startswith("{"):
            value=json.loads(line)
            if value.get("reason")=="compiler-artifact" and value["target"]["name"]==name:value["fresh"]=True
            line=json.dumps(value,separators=(",",":"))+"\n"
        output.append(line)
    (root/f"H-{name}.jsonl").write_text("".join(output))
PY
  for path in "$root"/valid-*.jsonl; do
    summary="$path.summary.json"
    bash "$helper_path" cargo-summary "$path" cache-bin-a "$summary" "$host" || return 1
    bash "$helper_path" cargo-assert "$summary" cache-bin-a '[]' '["default"]' "$host" false false false || return 1
  done
  self_expect_failure bash "$helper_path" cargo-summary "$root/missing-verbose.jsonl" cache-bin-a "$root/missing-verbose.json" "$host" || return 1
  for path in "$root"/H-*.jsonl; do
    summary="$path.summary.json"
    bash "$helper_path" cargo-summary "$path" cache-bin-a "$summary" "$host" || return 1
    self_expect_failure bash "$helper_path" cargo-assert "$summary" cache-bin-a '[]' '["default"]' "$host" false false false || return 1
  done
  printf '%s\n' 'F5 PASS: script fresh true/false and 0/1/2 runs observed; missing verbose and all app/lib/itoa H changes rejected'
}

self_cold_chain() (
  local root=$1 layout case_id status
  [ "$internal_self_test" = 1 ] && gate_test_allowed || exit 90
  mkdir -p -m 0700 -- "$root/chains" "$root/resume" "$root/routes" "$root/legacy"
  export FAKE_DOCKER_ROOT="$root/fake-state" FAKE_DOCKER_CALLS="$root/fake-calls.jsonl"
  # Revert only the two predicates in a private function copy.  The loaded
  # controller, oracle and fake executor otherwise remain the corrected source.
  python3 - "$script_path" "$root" <<'PY'
import hashlib,json,pathlib,sys
text=pathlib.Path(sys.argv[1]).read_text();root=pathlib.Path(sys.argv[2])
fixed='[ "$CUR_COLD" = 1 ] && [ "$phase" = p1 ] && no_cache=(--no-cache)'
old='[ "$CUR_COLD" = 1 ] && no_cache=(--no-cache)'
parts=[]
for name,next_name in (("artifact_build","write_override"),("compose_build","verify_export_failure")):
    part=text.split('\n'+name+'() {\n',1)[1].split('\n'+next_name+'() {\n',1)[0]
    part=name+'() {\n'+part
    if part.count(fixed)!=1:raise SystemExit("cold-predicate-copy-ambiguous")
    parts.append(part)
current='\n'.join(parts);legacy=current.replace(fixed,old)
assert current.count(fixed)==2 and legacy.replace(old,fixed)==current
for name,data in (("current-functions.sh",current),("legacy-functions.sh",legacy)):
    (root/name).write_text(data)
(root/"mutation.json").write_text(json.dumps({"controller_sha256":hashlib.sha256(text.encode()).hexdigest(),"current_functions_sha256":hashlib.sha256(current.encode()).hexdigest(),"legacy_functions_sha256":hashlib.sha256(legacy.encode()).hexdigest(),"predicate_replacements":2},sort_keys=True)+"\n")
PY
  [ "$?" = 0 ] || exit 1
  for layout in "${layouts[@]}"; do
    reset_runtime_globals_for_resume_test
    layout_selection=$layout case_selection=all output_dir="$root/chains/$layout"
    init_run || exit 1
    # A bounded prefix via the actual controller, without claiming all/all
    # completion or changing its immutable case inventory and final assertions.
    for case_id in cold exact-repeat forced-warm empty-cache; do
      if run_case "$layout" "$case_id"; then status=0; else status=$?; fi
      printf 'chain\t%s\t%s\t%s\n' "$layout" "$case_id" "$status" >>"$root/exits.tsv"
      [ "$status" = 0 ] || exit 1
    done
    validate_event_chain || exit 1
    printf 'cold-chain controller PASS: %s/cold,exact-repeat,forced-warm,empty-cache\n' "$layout"
    if self_controller "$root/resume" "$layout" compile-fail-p2; then status=0; else status=$?; fi
    printf 'resume\t%s\tcompile-fail-p2\t%s\n' "$layout" "$status" >>"$root/exits.tsv"
    [ "$status" = 0 ] || exit 1
    if self_controller "$root/routes" "$layout" route-contract; then status=0; else status=$?; fi
    printf 'route\t%s\troute-contract\t%s\n' "$layout" "$status" >>"$root/exits.tsv"
    [ "$status" = 0 ] || exit 1
    if (
      source "$root/legacy-functions.sh"
      reset_runtime_globals_for_resume_test
      self_controller "$root/legacy" "$layout" cold
    ) >"$root/legacy-$layout.stdout" 2>"$root/legacy-$layout.stderr"; then status=0; else status=$?; fi
    printf 'legacy\t%s\tcold\t%s\n' "$layout" "$status" >>"$root/exits.tsv"
    [ "$status" = 1 ] || exit 1
    printf 'cold-chain controller PASS: %s/resume,route,legacy-rejected\n' "$layout"
  done
  python3 - "$root" "$fixture_dir" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);fixture=pathlib.Path(sys.argv[2])
def read(path):return json.loads(path.read_text())
def jsonl(path):return [json.loads(line) for line in path.read_text().splitlines()]
def require(ok,why):
    if not ok:raise SystemExit(why)
calls=jsonl(root/"fake-calls.jsonl")
builds=[v for v in calls if v["operation"] in ("buildx","compose-build")]
compiles=[v for v in calls if v["operation"]=="source-compile"]
phase_for={("cache-bin-a","base"):"p1",("cache-bin-b","base"):"p2",("cache-bin-a","wide"):"p3",("cache-bin-b","wide"):"p4"}
services={"p1":"probe-a-base","p2":"probe-b-base","p3":"probe-a-wide","p4":"probe-b-wide"}
for name in ("baseline","artifacts","consumer"):
    require('\nENV CARGO_BUILD_JOBS=2\n' in (fixture/f"Dockerfile.{name}").read_text(),"compile-jobs-changed")
for v in builds:
    argv=v["argv"];source=pathlib.Path(v["source"]);identity=v.get("target")=="identity"
    require(argv.count("--no-cache")<=1,"duplicate-no-cache")
    if identity:
        require("--no-cache" in argv,"cold-p0-cache-policy-changed")
        continue
    values=v["build_args"];phase=phase_for[(values["BIN"],values["VARIANT"])];token=values["CACHE_FIXTURE_RUN_TOKEN"]
    cold=token in ("cold","empty-cache","route-source","route-artifact") or token.endswith("-setup")
    if "exact-repeat" in source.parts:cold=False
    legacy="legacy" in source.relative_to(root).parts
    require(("--no-cache" in argv)==(cold and (legacy or phase=="p1")),"wrong-phase-no-cache")
    if v["operation"]=="compose-build":
        require(v["cargo_build_jobs"]=="2" and v["compose_parallel_limit"]=="1","compile-concurrency-changed")
        require(argv[argv.index("build")+1:]==["--progress","plain"]+(["--no-cache"] if "--no-cache" in argv else [])+[services[phase]],"compose-not-single-service")
    else:
        require(v["target"]=="artifacts" and argv[argv.index("--platform")+1]=="linux/amd64","artifact-target-changed")
    shared=values["PROBE_SHARED_CACHE_ID"];target=values["PROBE_TARGET_CACHE_ID"]
    require(shared.endswith("-"+values["PROBE_CACHE_KEY"]),"cache-key-binding-lost")
    require(target==shared+"-target"+("-"+values["VARIANT"] if "-grouped-" in shared else ""),"target-group-split-changed")
# Every observed compile belongs to its immediately preceding source build;
# artifact-only consumers and Docker hits emit no source-compilation event.
last=None
for v in calls:
    if v["operation"] in ("buildx","compose-build"):last=v
    if v["operation"]=="source-compile":
        require(last is not None and last["argv"]==v["argv"],"compile-without-build")
        require(last.get("target")=="artifacts" or last.get("route") in ("baseline","source"),"artifact-consumer-compiled")
        require(v["no_cache_reset"]==("--no-cache" in v["argv"]),"no-cache-reset-not-modeled")
        v["source"]=last["source"]

def for_source(items,source):return [v for v in items if pathlib.Path(v["source"])==source]
def compile_phases(items):return [phase_for[(v["bin"],v["variant"])] for v in items]
def states(run,layout,scope,token):return [read(run/"state"/layout/scope/token/f"p{i}.json") for i in range(1,5)]
run_ids=[];namespaces=[]
for layout in ("baseline","common","grouped"):
    run=root/"chains"/layout;run_ids.append(read(run/"run.json")["run_id"])
    events=jsonl(run/"events.jsonl")
    passed=[v["case"] for v in events if v["type"]=="CASE" and v["status"]=="PASS"]
    require(passed==["cold","exact-repeat","forced-warm","empty-cache"],"cold-chain-order-changed")
    tokens={"cold":"cold","exact-repeat":"exact-repeat","forced-warm":"forced-warm","empty-cache":"empty-cache"}
    chain=[]
    for case,token in tokens.items():
        source=run/layout/"cases"/case/"attempt-001/source"
        observed=for_source(compiles,source)
        require(compile_phases(observed)==([] if case=="exact-repeat" else ["p1","p2","p3","p4"]),"cold-chain-compile-count")
        s=states(run,layout,"empty-cache" if case=="empty-cache" else "warm-chain",token)
        ns=[v["target_namespace"] for v in s];chain.append(ns)
        require(len(set(ns))==(2 if layout=="grouped" else 1),"wrong-namespace-count")
        require(ns[0]==ns[1] and ns[2]==ns[3],"per-group-namespace-drift")
        if case in ("cold","empty-cache"):
            require([v["no_cache_reset"] for v in observed]==[True,False,False,False],"cold-target-reset-after-p1")
            require([v["fresh"][2] for v in observed]==[False,True,layout!="grouped",True],"cold-itoa-observation-changed")
        if case=="forced-warm":require(all(v["fresh"][2] and not v["no_cache_reset"] for v in observed),"warm-target-lost")
    require(chain[0]==chain[1]==chain[2] and not set(chain[0])&set(chain[3]),"warm-chain-or-empty-cache-namespace-changed")
    for p in range(4):
        originals=states(run,layout,"warm-chain","cold");repeats=states(run,layout,"warm-chain","exact-repeat")
        require(originals[p]["input_sha256"]==repeats[p]["input_sha256"] and originals[p]["result"]==repeats[p]["result"],"exact-repeat-identity-changed")
    namespaces.append(set(chain[0]+chain[3]))
    run=root/"resume"/f"{layout}-compile-fail-p2";run_ids.append(read(run/"run.json")["run_id"])
    events=jsonl(run/"events.jsonl");gates=jsonl(run/"gates.jsonl")
    cause=[v for v in events if v["type"]=="EXPECTED_CAUSE"];stop=[v for v in events if v["type"]=="EXPECTED_STOP"]
    require(len(cause)==len(stop)==1 and cause[0]["exit"]==stop[0]["exit"]==42,"compile-failure-status-lost")
    post=[v for v in gates if v["phase"]=="p2-after-expected-failure"]
    require(len(post)==1 and post[0]["status"]=="PASS" and post[0]["previous_exit"]==0,"post-failure-gate-changed")
    require([(v["status"],v["exit"]) for v in events if v["type"]=="RUN_STATUS"]==[("expected-stop",75),("complete",0)],"resume-status-changed")
    require([(v["phase"],v["attempt"]) for v in events if v["type"]=="RESUME_REUSE"]==[("p1",2)],"resume-reuse-changed")
    case_dir=run/layout/"cases/compile-fail-p2"
    setup=for_source(compiles,case_dir/"setup/attempt-001/source")
    failed=for_source(compiles,case_dir/"attempt-001/source")
    resumed=for_source(compiles,case_dir/"attempt-002/source")
    require(compile_phases(setup)==["p1","p2","p3","p4"] and [v["no_cache_reset"] for v in setup]==[True,False,False,False],"resume-cold-warmup-changed")
    require(compile_phases(failed)==["p1","p2"] and compile_phases(resumed)==["p2","p3","p4"],"failure-or-resume-phase-order")
    require(not any(v["no_cache_reset"] for v in failed+resumed),"resumed-target-reset")
    require(resumed[0]["pending_before"]==(layout!="baseline") and resumed[0]["fresh"][2]==(layout=="baseline"),"pending-rebuild-lost")
    require(resumed[0]["target_namespace"]==failed[-1]["target_namespace"]==setup[1]["target_namespace"],"resume-namespace-drift")
    log=case_dir/("attempt-001/logs/p2.log" if layout=="baseline" else "attempt-001/logs/p2-producer.log")
    require(read(pathlib.Path(str(log)+".status.json"))==({"compose_exit":42} if layout=="baseline" else {"buildx_exit":42,"helper_exit":None}),"command-failure-exit-lost")
    run=root/"routes"/f"{layout}-route-contract";run_ids.append(read(run/"run.json")["run_id"])
    source=run/layout/"cases/route-contract/attempt-001/source"
    route_builds=for_source(builds,source);route_compiles=for_source(compiles,source)
    require(len(route_builds)==4 and all("--no-cache" in v["argv"] for v in route_builds),"single-p1-route-cold-policy-changed")
    require([v["token"] for v in route_compiles]==["route-source","route-artifact"] and all(v["no_cache_reset"] for v in route_compiles),"route-consumer-compiled-or-producer-skipped")
    require([v["phase"] for v in jsonl(run/"events.jsonl") if v["type"]=="ROUTE" and v["status"]=="PASS"]==["source-fallback","artifact-present"],"route-contract-not-proved")
    run=root/"legacy"/f"{layout}-cold";run_ids.append(read(run/"run.json")["run_id"])
    source=run/layout/"cases/cold/attempt-001/source";observed=for_source(compiles,source)
    require(compile_phases(observed)==["p1","p2"] and all(v["no_cache_reset"] and not v["fresh"][2] for v in observed),"legacy-bug-not-reproduced")
    events=jsonl(run/"events.jsonl")
    require(events[-1]["status"]=="failed" and events[-1]["exit"]==1 and not any(v["type"]=="EXPECTED_STOP" for v in events),"legacy-failure-normalized")
    require([v["phase"] for v in events if v["type"]=="VERIFY" and v["status"]=="PASS"]==["p1"],"legacy-ran-after-p2")
    # Artifact P2 expects both lib and itoa Fresh; the unchanged oracle checks
    # lib first.  Baseline reproduces L0's itoa diagnostic, and both paths above
    # must show itoa rebuilt in the retained compiler-artifact observation.
    reason="itoa" if layout=="baseline" else "lib"
    require((root/f"legacy-{layout}.stderr").read_text()==f"unexpected-recompile-set-{reason}\nbuild-layout-probe: FAILED_UNRESOLVED: unexpected-recompile-set\n","legacy-failed-for-wrong-reason")
require(len(set(run_ids))==12 and all(not a&b for i,a in enumerate(namespaces) for b in namespaces[i+1:]),"run-or-layout-cache-collision")
exits=[line.split('\t') for line in (root/"exits.tsv").read_text().splitlines()]
require(len(exits)==21 and all(int(v[-1])==(1 if v[0]=="legacy" else 0) for v in exits),"focused-command-exits")
report={"status":"PASS","controller_calls":len(exits),"build_calls":len(builds),"source_compiles":len(compiles),"legacy_rejections":3,"layouts":["baseline","common","grouped"],"actual_builds":"NOT_RETESTED","G2":"NOT_PASSED"}
(root/"result.json").write_text(json.dumps(report,sort_keys=True,indent=2)+"\n")
print("cold-chain PASS: P0/P1 cold; P2-P4 retained; repeat/warm/empty-cache, pending resume and routes; old predicates rejected at P2 (L0 itoa; L1/L2 lib first, itoa also rebuilt)")
PY
)

self_corrective_regressions() (
  local root=$1
  mkdir -p -m 0700 -- "$root"
  self_identity_contract "$root/identity" "$(dirname "$root")/wrappers/identity/identity.json" || exit 1
  printf '%s\n' 'F8 PASS: realistic P0 plus stable/prerelease identities; malformed/multiline rejected'
  self_execution_slot "$root/lock" || exit 1
  self_run_namespace "$root/namespaces" || exit 1
  self_post_failure_gate "$root/post-failure" "$(dirname "$root")/gate" || exit 1
  self_image_inspection "$root/image" "$root/namespaces/left/common-cold" || exit 1
  self_script_observations "$root/scripts" "$root/namespaces/left/common-cold/common/cases/cold/attempt-001/private-p1/cargo.jsonl" || exit 1
  self_export_failures "$root/export" || exit 1
)

self_journal_complete() (
  local root=$1
  gate_test_allowed || exit 90
  mkdir -p -m 0700 -- "$root/bin" "$root/proc/sys/kernel/random"
  printf 'MemAvailable: 8388608 kB\nSwapFree: 1048576 kB\n' >"$root/proc/meminfo"
  printf '01234567-89ab-cdef-0123-456789abcdef\n' >"$root/proc/sys/kernel/random/boot_id"
  # Use the loaded controller/helper functions, without a public test mode or
  # substituted collector, parser, limits, clock, or process implementation.
  {
    printf 'set -euo pipefail\n'
    declare -f
    cat <<'SH'
gate_test_allowed || exit 90
LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=
unset LAYOUT_JOURNAL_SINCE_US LAYOUT_JOURNAL_SINCE LAYOUT_JOURNAL_BOOT_ID
LAYOUT_GATE_MODE=real
LAYOUT_GATE_RECORD_FILE="$2/gates.jsonl" LAYOUT_GATE_STATE_FILE="$2/gate-state.json" LAYOUT_GATE_PRIVATE_DIR="$2/private"
case "$1" in
  collect) gate_collect_range "$(gate_command journalctl)" "$2/stdout" "$2/stderr" "$2/capture.json" '2026-09-15 02:18:21 UTC' '2026-09-15 02:48:21 UTC' ;;
  parse) gate_parse_range "$2/stdout" 0123456789abcdef0123456789abcdef 1789438701000000 1789440501000000 ;;
  gate) probe_gate cold journal-complete 0 ;;
  prior) probe_gate cold journal-prior 37 ;;
  *) exit 90 ;;
esac
SH
  } >"$root/driver.sh"
  cat >"$root/host-fake.py" <<'PY'
#!/usr/bin/env python3
import datetime,json,os,pathlib,sys,time
if os.environ.get("BUILD_LAYOUT_SELF_TEST_ACTIVE")!="1" or os.environ.get("LAYOUT_GATE_TEST_SEAM")!="build-layout-self-test":sys.exit(90)
name=pathlib.Path(sys.argv[0]).name;args=sys.argv[1:]
spec=json.load(open(os.environ["JOURNAL_TEST_DATA"]))
with open(os.environ["JOURNAL_TEST_CALLS"],"a") as handle:handle.write(json.dumps({"command":name,"argv":args,"pid":os.getpid(),"lc_all":os.environ.get("LC_ALL")})+"\n")
if name=="ps":
    assert args==["-eo","comm="];sys.exit(0)
if name=="systemctl":
    assert len(args)==3 and args[0]=="show" and args[1] in ("alpha.service","beta.service") and args[2]=="--property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts"
    print("LoadState=loaded\nActiveState=active\nSubState=running\nExecMainStatus=0\nMainPID=123\nNRestarts=0");sys.exit(0)
if name=="docker":
    assert len(args)==6 and args[:4]==["inspect","--type","container","--format"]
    print("a"*64+"\ttrue\tfalse\tfalse\thealthy\t0\tlagrange-station");sys.exit(0)
assert name=="journalctl" and os.environ.get("LC_ALL")=="C"
probe=args==["-k","-b","--no-pager","-o","json","-n","1"]
assert probe or (len(args)==10 and args[:6]==["-k","-b","--no-pager","-o","json","--since"] and args[7]=="--until" and args[9]=="--no-tail")
timestamp=time.time_ns()//1000 if probe else int(datetime.datetime.strptime(args[8],"%Y-%m-%d %H:%M:%S UTC").replace(tzinfo=datetime.timezone.utc).timestamp()*1000000)-500000
base={"__REALTIME_TIMESTAMP":str(timestamp),"__CURSOR":"synthetic","_BOOT_ID":"0123456789abcdef0123456789abcdef","_TRANSPORT":"kernel","MESSAGE":"synthetic-journal-message"}
def line(value=None,size=None):
    value=dict(base if value is None else value)
    raw=(json.dumps(value,separators=(",",":"))+"\n").encode()
    if size is not None:
        assert size>=len(raw);value["MESSAGE"]+="x"*(size-len(raw));raw=(json.dumps(value,separators=(",",":"))+"\n").encode();assert len(raw)==size
    return raw
if probe:
    mode=spec.get("probe","")
    if mode=="empty":sys.exit(0)
    if mode=="stderr":sys.stderr.write("synthetic warning\n")
    if mode=="wrong-boot":base["_BOOT_ID"]="f"*32
    sys.stdout.buffer.write(line());sys.exit(0)
mode=spec.get("mode","complete");count=spec.get("count",1)
if mode=="stdout-bytes":
    total=spec["bytes"];size=65536
    while total>size and total-size>=len(line()):sys.stdout.buffer.write(line(size=size));total-=size
    sys.stdout.buffer.write(line(size=total))
elif mode=="line-bytes":sys.stdout.buffer.write(line(size=spec["bytes"]))
elif mode=="stderr-bytes":sys.stderr.buffer.write(b"x"*spec["bytes"])
else:
    for index in range(count):
        value=dict(base)
        if index==0:
            if mode=="early-oom":value["MESSAGE"]="Out of memory: Killed process synthetic"
            if mode=="early-malformed":sys.stdout.buffer.write(b"{\n");continue
            if mode=="wrong-boot":value["_BOOT_ID"]="f"*32
            if mode=="wrong-transport":value["_TRANSPORT"]="stdout"
            if mode=="missing-field":del value["MESSAGE"]
            if mode=="before-since":value["__REALTIME_TIMESTAMP"]=str(int(datetime.datetime.strptime(args[6],"%Y-%m-%d %H:%M:%S UTC").replace(tzinfo=datetime.timezone.utc).timestamp()*1000000)-1)
            if mode=="after-until":value["__REALTIME_TIMESTAMP"]=str(timestamp+500001)
        raw=line(value)
        if index==count-1 and mode=="partial":raw=raw[:-1]
        if index==count-1 and mode=="partial-json":raw=b'{"MESSAGE":'
        sys.stdout.buffer.write(raw)
sys.stdout.buffer.flush();sys.stderr.buffer.flush()
if mode=="stderr":sys.stderr.write("synthetic warning\n");sys.stderr.flush()
if mode in ("eof-nonzero","eof-timeout"):
    os.close(1);os.close(2)
    time.sleep(60 if mode=="eof-timeout" else 0.05)
if spec.get("hold") or mode=="timeout":time.sleep(60)
os._exit(23 if mode in ("nonzero","eof-nonzero") else 0)
PY
  chmod 0755 -- "$root/host-fake.py"
  for name in ps systemctl journalctl docker; do ln -s ../host-fake.py "$root/bin/$name"; done
  python3 - "$root" <<'PY'
import hashlib,json,os,pathlib,subprocess,sys,time
root=pathlib.Path(sys.argv[1]);driver=str(root/"driver.sh");data=root/"data.json";calls=root/"calls.jsonl"
env={**os.environ,"BUILD_LAYOUT_SELF_TEST_ACTIVE":"1","LAYOUT_GATE_TEST_SEAM":"build-layout-self-test","LAYOUT_GATE_MODE":"real","BUILD_LAYOUT_HEALTH_UNITS":"alpha.service,beta.service","BUILD_LAYOUT_HEALTH_CONTAINERS":"lagrange-station-research-worker-1,api-1","LAYOUT_GATE_PROC_ROOT":str(root/"proc"),"JOURNAL_TEST_DATA":str(data),"JOURNAL_TEST_CALLS":str(calls)}
env.pop("BUILD_LAYOUT_RESEARCH_EXCEPTION",None)
for name in ("PS","SYSTEMCTL","JOURNALCTL","DOCKER"):env["LAYOUT_GATE_"+name+"_BIN"]=str(root/"bin"/name.lower())
limits={"deadline_ms":10000,"stdout_bytes":67108864,"stderr_bytes":65536,"line_bytes":1048576,"records":100000}
results=[]
def command(label,operation,directory,expected):
    argv=["bash",driver,operation,str(directory)];started=time.monotonic_ns()
    result=subprocess.run(argv,env=env,capture_output=True,timeout=30)
    (root/(label+".stdout.log")).write_bytes(result.stdout);(root/(label+".stderr.log")).write_bytes(result.stderr)
    results.append({"label":label,"argv":argv,"exit":result.returncode,"elapsed_ns":time.monotonic_ns()-started,"stdout_sha256":hashlib.sha256(result.stdout).hexdigest(),"stderr_sha256":hashlib.sha256(result.stderr).hexdigest()})
    (root/"results.json").write_text(json.dumps(results,indent=2)+"\n")
    assert result.returncode==expected,(label,result.returncode,expected)
    assert b"synthetic-journal-message" not in result.stdout+result.stderr,label
    return result
def captured(label,spec,reason=None,parse_result=0,expected_count=None):
    directory=root/label;directory.mkdir(mode=0o700);data.write_text(json.dumps(spec))
    command(label,"collect",directory,0 if reason is None else 1)
    receipt=json.loads((directory/"capture.json").read_text())
    assert receipt["limits"]==limits and receipt["bounds_exclusive"] is True,label
    assert receipt["failure"]==reason and receipt["complete"]==(reason is None),(label,receipt)
    assert receipt["lc_all"]=="C" and receipt["argv"]==[str(root/"bin/journalctl"),"-k","-b","--no-pager","-o","json","--since","2026-09-15 02:18:21 UTC","--until","2026-09-15 02:48:21 UTC","--no-tail"],label
    for stream in ("stdout","stderr"):
        raw=(directory/stream).read_bytes()
        assert receipt[stream+"_sha256"]==hashlib.sha256(raw).hexdigest() and receipt[stream+"_bytes"]==len(raw),label
        assert len(raw)<=limits[stream+"_bytes"],label
    if reason is None:
        assert receipt["stdout_eof"] and receipt["stderr_eof"] and receipt["command_exit"]==0 and receipt["child_reaped"] and receipt["elapsed_ns"]<10**10,label
        parsed=command(label+"-parse","parse",directory,parse_result)
        if expected_count is not None:assert parsed.stdout==f"{expected_count} {1 if spec.get('mode')=='early-oom' else 0}\n".encode(),label
    else:
        assert receipt["complete"] is False,label
        if reason=="command-nonzero":assert receipt["command_exit"]==23 and receipt["stdout_eof"] and receipt["stderr_eof"],label
        if reason=="deadline":assert receipt["elapsed_ns"]>=10**10 and receipt["stop_requested"],label
    fake_calls=[json.loads(line) for line in calls.read_text().splitlines()]
    child=fake_calls[-1]["pid"]
    try:os.kill(child,0)
    except ProcessLookupError:pass
    else:raise AssertionError((label,"synthetic collector child remains alive or unreaped"))
    if reason!="deadline":assert receipt["child_reaped"],label
    return receipt,directory
for count in (0,999,1000,1014,99999):captured("complete-"+str(count),{"count":count},expected_count=count)
# A complete 1,000-line EOF replaces the obsolete tail-truncation rejection.
# Incompleteness is now exercised by live pipes, missing LF, bounds and failed exit.
captured("early-oom",{"mode":"early-oom","count":1014},expected_count=1014)
for mode in ("early-malformed","wrong-boot","wrong-transport","missing-field","before-since","after-until"):
    captured(mode,{"mode":mode,"count":1014},parse_result=1)
for mode,reason in (("stderr","stderr-nonempty"),("nonzero","command-nonzero"),("eof-nonzero","command-nonzero"),("partial","partial-final-record"),("partial-json","partial-final-record"),("timeout","deadline"),("eof-timeout","deadline")):
    captured(mode,{"mode":mode},reason)
captured("stdout-below",{"mode":"stdout-bytes","bytes":67108863},expected_count=1024)
for size in (67108864,67108865):
    receipt,directory=captured("stdout-"+str(size),{"mode":"stdout-bytes","bytes":size,"hold":True},"stdout-byte-limit")
    assert receipt["stdout_bytes"]==67108864 and receipt["stop_requested"]
captured("line-below",{"mode":"line-bytes","bytes":1048575},expected_count=1)
for size in (1048576,1048577):captured("line-"+str(size),{"mode":"line-bytes","bytes":size,"hold":True},"line-byte-limit")
captured("stderr-below",{"mode":"stderr-bytes","bytes":65535},"stderr-nonempty")
for size in (65536,65537):captured("stderr-"+str(size),{"mode":"stderr-bytes","bytes":size,"hold":True},"stderr-byte-limit")
for count in (100000,100001):captured("records-"+str(count),{"count":count,"hold":True},"record-limit")
# Parser-only defense still rejects bounded files independently of collection.
for label in ("partial","partial-json","line-1048576","stdout-67108864","records-100000"):
    command(label+"-parser-bound","parse",root/label,1)
# Run the actual controller gate functions with synthetic host executables.
gate_cases=[("empty",{"count":0},0,"healthy"),("999",{"count":999},0,"healthy"),("1000",{"count":1000},0,"healthy"),("1014",{"count":1014},0,"healthy"),("oom",{"mode":"early-oom","count":1014},1,"kernel-oom-observed"),("malformed",{"mode":"early-malformed","count":1014},1,"journal-range-invalid-or-oom"),("partial",{"mode":"partial"},1,"journal-range-invalid-or-oom"),("stderr",{"mode":"stderr"},1,"journal-range-invalid-or-oom"),("nonzero",{"mode":"eof-nonzero"},1,"journal-range-invalid-or-oom"),("empty-probe",{"probe":"empty"},1,"kernel-journal-unestablished"),("probe-stderr",{"probe":"stderr"},1,"kernel-journal-unestablished"),("wrong-probe-boot",{"probe":"wrong-boot"},1,"kernel-journal-unestablished")]
for label,spec,status,reason in gate_cases:
    directory=root/("gate-"+label);directory.mkdir(mode=0o700);data.write_text(json.dumps(spec))
    command("gate-"+label,"gate",directory,status)
    raw=(directory/"gates.jsonl").read_bytes();gate=json.loads(raw)
    assert gate["reason"]==reason and gate["status"]==("PASS" if status==0 else "FAIL"),label
    assert b"synthetic-journal-message" not in raw and b"Killed process synthetic" not in raw,label
    assert not list((directory/"private").glob("build-layout-gate.*")),label
    if status==0 or reason=="kernel-oom-observed":
        capture=gate["journal"]["range"]["capture"]
        assert capture["complete"] and capture["child_reaped"] and capture["limits"]==limits,label
        assert gate["journal"]["count"]==spec["count"] and gate["journal"]["oom_count"]==int(reason=="kernel-oom-observed"),label
        assert capture["argv"][7]==gate["journal"]["since"] and capture["argv"][9]==gate["journal"]["until"],label
        for stream in ("stdout","stderr"):assert gate["journal"]["range"][stream+"_sha256"]==capture[stream+"_sha256"],label
directory=root/"prior";directory.mkdir(mode=0o700);before=calls.read_bytes()
command("previous-exit","prior",directory,1)
gate=json.loads((directory/"gates.jsonl").read_text());assert gate["previous_exit"]==37 and gate["reason"]=="previous-step-failed" and calls.read_bytes()==before
print(f"Complete journal PASS: {len(results)} focused invocations; real collector/parser/controller; fixed limits, EOF/exit, early OOM, child reaping and private cleanup")
PY
)

self_research_exception() (
  local root=$1
  mkdir -p -m 0700 -- "$root/bin" "$root/proc/sys/kernel/random"
  # Snapshot the actual loaded functions and explicit non-secret controller inputs.
  # This driver is private to --self-test; it adds no public mode or runtime seam.
  {
    printf 'set -euo pipefail\n'
    declare -p script_path repo_root fixture_dir helper_path design_path baseline_doc layouts case_ids case_numbers p_bin p_variant p_runtime p_group phases c1 c2 c3 origin_head fixture_sha tool_sha design_sha baseline_sha
    declare -f
    cat <<'SH'
[ "${BUILD_LAYOUT_SELF_TEST_ACTIVE-}" = 1 ] && [ "${LAYOUT_GATE_TEST_SEAM-}" = build-layout-self-test ] || exit 90
mode=apply internal_self_test=1 layout_selection=common case_selection=cold execution_lock_fd=
LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=
run_dir= run_id= events_file= gate_file= run_json= timing_file= output_dir= resume_dir=
CUR_LAYOUT= CUR_CASE= CUR_SCOPE= CUR_TOKEN= CUR_SOURCE= CUR_ATTEMPT=0 CUR_ATTEMPT_DIR= CUR_COMMIT= CUR_SETTING= CUR_CONFIG= CUR_COLD=0 CUR_PHASE=
CUR_COMPILE= CUR_LIB= CUR_APP= CUR_RUNTIME= CUR_RECIPE= CUR_GUARD= CUR_K= CUR_H= CUR_HOST= CUR_IDENTITY_SHA= CUR_SHARED_CACHE= CUR_TARGET_CACHE= CUR_LEDGER_INJECT=none CUR_SOURCE_TRAP=0
RESUME_ACTIVE=0 RESUMING=0 RESUME_COMPLETE=0
unset LAYOUT_JOURNAL_SINCE_US LAYOUT_JOURNAL_SINCE LAYOUT_JOURNAL_BOOT_ID
export LAYOUT_GATE_MODE=real
case "$1" in
  check) research_exception_check ;;
  new)
    LAYOUT_RESEARCH_BINDING=$(research_exception_check) || exit 1
    output_dir=$2
    init_run || exit 1
    mkdir -m 0700 -- "$run_dir/state"
    probe_gate cold first 0 || exit 1
    set_run_status complete 0 ;;
  load|sample)
    resume_dir=$2
    load_resume_state || exit 1
    if [ "$1" = sample ]; then probe_gate complete resume-sample 0 || exit 1; fi ;;
  controller)
    LAYOUT_RESEARCH_BINDING=$(research_exception_check) || exit 1
    export FAKE_DOCKER_ROOT="$2/build-state" FAKE_DOCKER_CALLS="$2/build-calls.jsonl" FAKE_LAYOUT_HELPER="$helper_path"
    mkdir -p -m 0700 -- "$FAKE_DOCKER_ROOT" "$2/controller"
    create_fake_docker "$2/build-docker"
    docker_bin="$2/build-docker"
    self_controller "$2/controller" common stop-after-p2 || exit 1 ;;
  *) exit 90 ;;
esac
SH
  } >"$root/driver.sh"
  cat >"$root/host-fake.py" <<'PY'
#!/usr/bin/env python3
import datetime,json,os,pathlib,sys,time
if os.environ.get("BUILD_LAYOUT_SELF_TEST_ACTIVE")!="1" or os.environ.get("LAYOUT_GATE_TEST_SEAM")!="build-layout-self-test":sys.exit(90)
name=pathlib.Path(sys.argv[0]).name;args=sys.argv[1:];s=json.load(open(os.environ["RESEARCH_TEST_DATA"]))
with open(os.environ["RESEARCH_TEST_CALLS"],"a") as f:f.write(json.dumps({"command":name,"argv":args})+"\n")
if name=="ps":
    assert args==["-eo","comm="]
    if s.get("compiler"):print("rustc")
elif name=="systemctl":
    assert len(args)==3 and args[0]=="show" and args[1] in ("alpha.service","beta.service") and args[2]=="--property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts"
    unit={"LoadState":"loaded","ActiveState":"active","SubState":"running","ExecMainStatus":"0","MainPID":"123","NRestarts":"0"}
    if args[1]=="beta.service":unit.update(s.get("unit",{}))
    for k,v in unit.items():print(k+"="+v)
elif name=="journalctl":
    probe=args==["-k","-b","--no-pager","-o","json","-n","1"]
    assert probe or (len(args)==10 and args[:5]==["-k","-b","--no-pager","-o","json"] and args[5]=="--since" and args[7]=="--until" and args[9:]==["--no-tail"])
    mode=s.get("journal","")
    if mode=="denied":print("synthetic permission failure",file=sys.stderr);sys.exit(1)
    if mode=="empty-probe" and probe:sys.exit(0)
    if mode=="bad-range" and not probe:print("{");sys.exit(0)
    t=time.time_ns()//1000 if probe else int(datetime.datetime.strptime(args[8],"%Y-%m-%d %H:%M:%S UTC").replace(tzinfo=datetime.timezone.utc).timestamp()*1000000)-500000
    print(json.dumps({"__REALTIME_TIMESTAMP":str(t),"__CURSOR":"synthetic","_BOOT_ID":("f"*32 if mode=="wrong-boot" else "0123456789abcdef0123456789abcdef"),"_TRANSPORT":"kernel","MESSAGE":("Killed process synthetic" if mode=="oom" and not probe else "quiet kernel")}))
elif name=="docker":
    assert len(args)==6 and args[:4]==["inspect","--type","container","--format"]
    strict=r'{{.Id}}{{printf "\t"}}{{.State.Running}}{{printf "\t"}}{{.State.Restarting}}{{printf "\t"}}{{.State.OOMKilled}}{{printf "\t"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}{{printf "\t"}}{{.RestartCount}}{{printf "\t"}}{{if index .Config.Labels "com.docker.compose.project"}}{{index .Config.Labels "com.docker.compose.project"}}{{else}}absent{{end}}'
    exceptional=args[5]=="lagrange-station-research-worker-1" and bool(os.environ.get("BUILD_LAYOUT_RESEARCH_EXCEPTION"))
    expected=strict+(r'{{printf "\t"}}{{.Image}}{{printf "\t"}}{{.State.ExitCode}}' if exceptional else '')
    assert args[4]==expected
    v=s["research" if args[5]=="lagrange-station-research-worker-1" else "other"]
    if v.get("missing"):sys.exit(1)
    if v.get("delay"):time.sleep(v["delay"])
    row=[v[k] for k in ("id","running","restarting","oom","health","count","project")]
    if exceptional:row += [v["image"],v["exit"]]
    if v.get("short"):row=row[:-1]
    print("\t".join(str(x) for x in row))
else:sys.exit(90)
PY
  chmod 0755 "$root/host-fake.py"
  for name in ps systemctl journalctl docker; do ln -s ../host-fake.py "$root/bin/$name"; done
  python3 - "$root" "$script_path" <<'PY'
import base64,copy,datetime,hashlib,json,os,pathlib,shutil,subprocess,sys,time
root=pathlib.Path(sys.argv[1]);controller=sys.argv[2];driver=str(root/"driver.sh");calls=root/"host-calls.jsonl";data=root/"host-data.json"
name="lagrange-station-research-worker-1";ident="a"*64;image="sha256:"+"b"*64
unit_env="alpha.service,beta.service";container_env=name+",api-1"
def utc(seconds):return datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
now=int(time.time());grant={"format":"lagrange-build-research-exception-v1","scope":"image-build-only","container_name":name,"container_id":ident,"image_id":image,"observed_at_utc":utc(now-3600),"expires_at_utc":utc(now+3600),"initial_restart_count":100,"known_error_code":"PRICE_CURATION_FAILED","known_exit_code":2}
valid=root/"grant.json"
def write_grant(path,fields):
    path.write_bytes(fields if isinstance(fields,bytes) else (json.dumps(fields,indent=2)+"\n").encode());path.chmod(0o600);return str(path)
write_grant(valid,grant);raw=valid.read_bytes();digest=hashlib.sha256(raw).hexdigest()
base={"research":{"id":ident,"image":image,"running":"true","restarting":"true","oom":"false","health":"unhealthy","count":100,"project":"lagrange-station","exit":2},"other":{"id":"c"*64,"running":"true","restarting":"false","oom":"false","health":"healthy","count":0,"project":"lagrange-station"}}
def fixture(state=None,mem=8388608,swap=1048576):
    data.write_text(json.dumps(base if state is None else state));(root/"proc/meminfo").write_text(f"MemAvailable: {mem} kB\nSwapFree: {swap} kB\n");(root/"proc/sys/kernel/random/boot_id").write_text("01234567-89ab-cdef-0123-456789abcdef\n")
env={**os.environ,"BUILD_LAYOUT_SELF_TEST_ACTIVE":"1","LAYOUT_GATE_TEST_SEAM":"build-layout-self-test","BUILD_LAYOUT_HEALTH_UNITS":unit_env,"BUILD_LAYOUT_HEALTH_CONTAINERS":container_env,"RESEARCH_TEST_DATA":str(data),"RESEARCH_TEST_CALLS":str(calls),"LAYOUT_GATE_PROC_ROOT":str(root/"proc")}
for cmd in ("PS","SYSTEMCTL","JOURNALCTL","DOCKER"):env["LAYOUT_GATE_"+cmd+"_BIN"]=str(root/"bin"/cmd.lower())
results=[]
def count_calls():return len(calls.read_text().splitlines()) if calls.exists() else 0
def run(label,args,expected=0,exception=valid,external=None,extra=None):
    e={**env};e.pop("BUILD_LAYOUT_RESEARCH_EXCEPTION",None)
    if exception is not None:e["BUILD_LAYOUT_RESEARCH_EXCEPTION"]=str(exception)
    if extra:e.update(extra)
    before=count_calls();result=subprocess.run(args,env=e,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=180)
    log=result.stdout+result.stderr;(root/(label+".log")).write_bytes(log)
    record={"label":label,"argv":args,"exit":result.returncode,"stdout_sha256":hashlib.sha256(result.stdout).hexdigest(),"stderr_sha256":hashlib.sha256(result.stderr).hexdigest(),"log_sha256":hashlib.sha256(log).hexdigest(),"host_fake_calls":count_calls()-before}
    results.append(record);(root/"results.json").write_text(json.dumps(results,indent=2)+"\n")
    if result.returncode!=expected or (external is False and record["host_fake_calls"]!=0):raise SystemExit(f"research regression failed: {label}, exit={result.returncode}, expected={expected}")
    return result
fixture()
run("default-unhealthy",["bash",driver,"new",str(root/"strict-unhealthy")],1,exception=None)
run("valid",["bash",driver,"new",str(root/"valid")])
def gates(path):return [json.loads(line) for line in (path/"gates.jsonl").read_text().splitlines()]
runroot=root/"valid";metadata=json.loads((runroot/"run.json").read_text());bound=metadata["gate"]["research_exception"]
assert bound=={"path":str(valid),"fields":grant,"sha256":digest,"bytes_base64":base64.b64encode(raw).decode()}
assert (runroot/"research-exception.json").read_bytes()==raw and (runroot/"research-exception.json").stat().st_mode&0o777==0o600
first=gates(runroot)[0];assert first["reason"]=="image-build-only-known-incident" and first["evidence"]["research_exception"]["sha256"]==digest
assert first["evidence"]["containers"][name]["selected"]["restart_count"]=="100"
base["research"]["count"]=102;fixture();run("resume-increase",["bash",driver,"sample",str(runroot)])
state=json.loads((runroot/"gate-state.json").read_text());assert state["containers"][name]["restart_count"]=="100" and state["research_exception"]["latest_observation"]["restart_count"]=="102"
assert (runroot/"run.json").read_text()==json.dumps(metadata,sort_keys=True,separators=(",",":"))+"\n"
# Invalid grants fail in the actual CLI before output creation or any host command.
invalids={"expired":{**grant,"expires_at_utc":utc(now-1)},"future":{**grant,"observed_at_utc":utc(now+60)},"overlong":{**grant,"expires_at_utc":utc(now+86400)},"unknown-key":{**grant,"extra":0},"wrong-name":{**grant,"container_name":"api-1"},"wrong-scope":{**grant,"scope":"release"},"wrong-error":{**grant,"known_error_code":"OTHER"},"wrong-format":{**grant,"format":"other"},"duplicate":raw.rstrip()[:-1]+b',"known_exit_code":2}\n',"malformed":b'{',"array":b'[]',"non-utf8":b'\xff'}
for key in grant:invalids["missing-"+key]={k:v for k,v in grant.items() if k!=key}
for index,value in enumerate((True,-1,1.5,"1",None)):invalids["restart-type-"+str(index)]={**grant,"initial_restart_count":value}
for index,value in enumerate((True,2.0,"2",0)):invalids["exit-type-"+str(index)]={**grant,"known_exit_code":value}
for key,value in grant.items():
    if isinstance(value,str):invalids["string-type-"+key]={**grant,key:2}
for key,value in (("container_id","A"*64),("image_id","sha256:"+"B"*64),("observed_at_utc","2026-02-30T00:00:00Z"),("expires_at_utc",utc(now+600).replace("Z","+00:00")),("observed_at_utc",utc(now-60).replace("Z",".0Z"))):invalids["bad-value-"+key]={**grant,key:value}
for label,value in invalids.items():
    path=root/(label+".json");write_grant(path,value);out=root/("out-"+label)
    run(label,["bash",controller,"--apply","--output-dir",str(out)],1,exception=path,external=False)
    assert not out.exists()
# Filesystem validation uses only private fixtures; wrong UID uses a synthetic
# Python executable running the unchanged parser body with a foreign executing UID.
unsafe={"empty":"","relative":os.path.relpath(valid),"dotdot":str(root/"x/../grant.json"),"double-slash":str(root)+"//grant.json","trailing-slash":str(valid)+"/","missing-file":str(root/"missing.json"),"directory":str(root)}
for label,mode in (("public-mode",0o644),("executable-mode",0o700),("special-mode",0o4600)):
    path=root/(label+".json");write_grant(path,grant);path.chmod(mode);unsafe[label]=str(path)
link=root/"symlink.json";link.symlink_to(valid);unsafe["symlink"]=str(link)
linkdir=root/"linked-parent";linkdir.symlink_to(root,target_is_directory=True);unsafe["symlink-parent"]=str(linkdir/"grant.json")
fifo=root/"fifo";os.mkfifo(fifo,0o600);unsafe["fifo"]=str(fifo)
for label,path in unsafe.items():run("unsafe-"+label,["bash",driver,"check"],1,exception=path,external=False)
shim=root/"uid-shim";shim.mkdir(mode=0o700);python=shim/"python3"
python.write_text('#!'+sys.executable+'\nimport os,sys\nassert os.environ.get("BUILD_LAYOUT_SELF_TEST_ACTIVE")=="1" and os.environ.get("LAYOUT_GATE_TEST_SEAM")=="build-layout-self-test" and sys.argv[1]=="-"\ncode=sys.stdin.read();sys.argv=sys.argv[1:];uid=os.geteuid();os.geteuid=lambda: uid+1\nexec(compile(code,"<actual-exception-parser>","exec"))\n');python.chmod(0o755)
result=run("unsafe-owner",["bash",driver,"check"],1,external=False,extra={"PATH":str(shim)+os.pathsep+env["PATH"]});assert b"research-exception-owner-invalid" in result.stderr
# Validity edges and final-gate expiry use the actual clock, with no time override.
edge={**grant,"observed_at_utc":utc(int(time.time())-5),"expires_at_utc":utc(int(time.time())+600)}
write_grant(valid,edge);good=copy.deepcopy(base);good["research"]["count"]=103;fixture(good)
run("ceil-bound",["bash",driver,"new",str(root/"ceil-bound")])
good["research"]["count"]=10000;fixture(good);run("ceil-overrun",["bash",driver,"new",str(root/"ceil-overrun")],1)
write_grant(valid,{**grant,"expires_at_utc":utc(now-3600+86400)});run("exact-24h",["bash",driver,"check"],external=False)
write_grant(valid,{**grant,"expires_at_utc":utc(int(time.time())+3)});good=copy.deepcopy(base);good["other"]["delay"]=4;fixture(good)
run("gate-expired-during-observation",["bash",driver,"new",str(root/"gate-expired")],1)
assert gates(root/"gate-expired")[-1]["reason"]=="research-exception-invalid"
write_grant(valid,raw)
# Identity/health/exit/rate failures retain typed observations and counts.
changes=[("wrong-container-id","id","d"*64),("wrong-image","image","sha256:"+"e"*64),("wrong-project","project","other"),("stopped","running","false"),("oom","oom","true"),("unknown-health","health","absent"),("unknown-running","running","unknown"),("unknown-restarting","restarting","unknown"),("unknown-oom","oom","unknown"),("other-exit","exit",3),("restart-exit-zero","exit",0),("unknown-exit","exit","unknown"),("negative-count","count",-1),("unknown-count","count","unknown"),("restart-rate","count",10000),("initial-decrease","count",99),("missing-container","missing",True),("missing-field","short",True)]
for label,key,value in changes:
    bad=copy.deepcopy(base);bad["research"][key]=value;fixture(bad);target=root/("gate-"+label)
    run(label,["bash",driver,"new",str(target)],1)
    selected=gates(target)[-1]["evidence"]["containers"][name]["selected"]
    if key=="count" and value==10000:assert selected["restart_count"]=="10000"
bad=copy.deepcopy(base);bad["research"].update(restarting="false",exit=3);fixture(bad)
run("nonrestarting-exit-three",["bash",driver,"new",str(root/"nonrestarting-exit-three")],1)
for health,restarting,exit_code in (("healthy","false",0),("unhealthy","false",2),("starting","true",2),("starting","false",0)):
    good=copy.deepcopy(base);good["research"].update(health=health,restarting=restarting,exit=exit_code);fixture(good)
    run("allowed-"+health+restarting+str(exit_code),["bash",driver,"new",str(root/("allowed-"+health+restarting+str(exit_code)))])
for label,kind,key,value in (("latest-decrease","research","count",101),("other-health","other","health","unhealthy"),("other-restart","other","count",1),("other-id","other","id","d"*64),("unit-restart","unit","NRestarts","1"),("unit-id","unit","MainPID","999"),("unit-health","unit","ActiveState","failed")):
    bad=copy.deepcopy(base);bad.setdefault(kind,{})[key]=value;fixture(bad);clone=root/label;shutil.copytree(runroot,clone)
    run(label,["bash",driver,"sample",str(clone)],1)
for label,settings in (("ram",{"mem":2097151}),("swap",{"swap":524287})):
    fixture(**settings);run(label,["bash",driver,"new",str(root/("resource-"+label))],1)
for mode in ("oom","empty-probe","denied","bad-range","wrong-boot"):
    bad=copy.deepcopy(base);bad["journal"]=mode;fixture(bad);run("kernel-"+mode,["bash",driver,"new",str(root/("kernel-"+mode))],1)
bad=copy.deepcopy(base);bad["compiler"]=True;fixture(bad);run("compiler",["bash",driver,"new",str(root/"compiler")],1)
fixture(mem=2097152,swap=524288);run("exact-resources",["bash",driver,"new",str(root/"exact-resources")])
fixture();run("resume-missing-input",["bash",driver,"load",str(runroot)],1,exception=None,external=False)
valid.unlink();run("resume-missing-file",["bash",driver,"load",str(runroot)],1,external=False);write_grant(valid,raw)
substitute=root/"substitute.json";write_grant(substitute,raw);run("resume-substitute",["bash",driver,"load",str(runroot)],1,exception=substitute,external=False)
for label,content in (("changed",raw+b" "),("extended",{**grant,"expires_at_utc":utc(now+7200)}),("expired",{**grant,"expires_at_utc":utc(now-1)})):
    write_grant(valid,content);run("resume-"+label,["bash",driver,"load",str(runroot)],1,external=False);write_grant(valid,raw)
for label in ("missing-snapshot","changed-snapshot","unsafe-snapshot","missing-state","changed-latest","changed-first","changed-journal"):
    clone=root/label;shutil.copytree(runroot,clone)
    if label=="missing-snapshot":(clone/"research-exception.json").unlink()
    elif label=="changed-snapshot":(clone/"research-exception.json").write_bytes(raw+b" ")
    elif label=="unsafe-snapshot":(clone/"research-exception.json").chmod(0o644)
    elif label=="missing-state":(clone/"gate-state.json").unlink()
    else:
        state=json.loads((clone/"gate-state.json").read_text())
        if label=="changed-latest":state["research_exception"]["latest_observation"]["restart_count"]="100"
        elif label=="changed-first":state["containers"][name]["restart_count"]="101"
        else:state["journal_since_us"]+=1000000
        (clone/"gate-state.json").write_text(json.dumps(state))
    run(label,["bash",driver,"load",str(clone)],1,external=False)
good=copy.deepcopy(base);good["research"].update(health="healthy",restarting="false");fixture(good)
run("strict-healthy",["bash",driver,"new",str(root/"strict-healthy")],exception=None)
run("strict-gain",["bash",driver,"load",str(root/"strict-healthy")],1,external=False)
# Public plans must neither acquire the actual slot nor mutate the saved run.
def hashes(directory):return {str(p.relative_to(directory)):hashlib.sha256(p.read_bytes()).hexdigest() for p in directory.rglob("*") if p.is_file()}
planenv={"BUILD_LAYOUT_HEALTH_UNITS":"","BUILD_LAYOUT_HEALTH_CONTAINERS":""}
for label,args,exception in (("default-plan",["--output-dir",str(root/"never-created")],None),("explicit-plan",["--plan","--output-dir",str(root/"never-created")],valid),("resume-plan",["--plan","--resume-from",str(runroot)],valid),("strict-resume-plan",["--plan","--resume-from",str(root/"strict-healthy")],None)):
    before=hashes(runroot);run(label,["bash",controller,*args],exception=exception,external=False,extra=planenv);assert hashes(runroot)==before and not (root/"never-created").exists()
run("invalid-resume-plan",["bash",controller,"--plan","--resume-from",str(runroot)],1,exception=substitute,external=False,extra=planenv)
fixture();run("full-exception-controller",["bash",driver,"controller",str(root)],external=True)
print(f"Research exception PASS: {len(results)} focused checks; real parser/controller, synthetic executables; image-only incident evidence retained")
PY
)

self_test() {
  unset BUILD_LAYOUT_RESEARCH_EXCEPTION
  LAYOUT_RESEARCH_BINDING=null LAYOUT_RESEARCH_SNAPSHOT=
  validate_fixture
  hash_inputs
  [ "${#case_ids[@]}" = 28 ] || return 1
  local root; root=$(mktemp -d "${TMPDIR:-/tmp}/build-layout-self-test.XXXXXX")
  chmod 0700 -- "$root"
  printf 'build-layout-probe: self-test evidence=%s\n' "$root"
  export BUILD_LAYOUT_SELF_TEST_ACTIVE=1 LAYOUT_GATE_TEST_SEAM=build-layout-self-test LAYOUT_GATE_MODE=fake LAYOUT_FAKE_GATE_RESULT=pass
  export BUILD_LAYOUT_HEALTH_UNITS='alpha.service,beta.service' BUILD_LAYOUT_HEALTH_CONTAINERS='lagrange-station-research-worker-1,api-1'
  export FAKE_DOCKER_ROOT="$root/fake-docker-state" FAKE_DOCKER_CALLS="$root/fake-docker-calls.jsonl" FAKE_LAYOUT_HELPER="$helper_path"
  mkdir -p -m 0700 -- "$FAKE_DOCKER_ROOT" "$root/runs"
  create_fake_docker "$root/fake-docker"
  docker_bin="$root/fake-docker" internal_self_test=1
  self_journal_complete "$root/journal-complete" || return 1
  self_research_exception "$root/research-exception" || return 1
  self_gate_boundaries "$root/gate"
  ( export BUILD_LAYOUT_SELF_TEST_ACTIVE=1 LAYOUT_GATE_TEST_SEAM=build-layout-self-test LAYOUT_GATE_MODE=fake LAYOUT_FAKE_GATE_RESULT=pass LAYOUT_GATE_PROC_ROOT=/forbidden LAYOUT_GATE_DOCKER_BIN=/forbidden; sanitize_actual_environment; [ -z "${BUILD_LAYOUT_SELF_TEST_ACTIVE-}${LAYOUT_GATE_TEST_SEAM-}${LAYOUT_GATE_MODE-}${LAYOUT_FAKE_GATE_RESULT-}${LAYOUT_GATE_PROC_ROOT-}${LAYOUT_GATE_DOCKER_BIN-}" ] ) || return 1
  self_wrapper_boundaries "$root/wrappers" || return 1
  self_corrective_regressions "$root/corrective" || return 1
  self_cold_chain "$root/cold-chain" || return 1
  local layout case_id
  for layout in "${layouts[@]}"; do
    for case_id in "${case_ids[@]}"; do
      self_controller "$root/runs" "$layout" "$case_id" || { printf '%s\n' "self-test controller failed: $layout/$case_id" >&2; return 1; }
      printf 'self-test controller PASS: %s/%s\n' "$layout" "$case_id"
    done
  done
  python3 - "$FAKE_DOCKER_CALLS" <<'PY'
import json,sys
calls=[json.loads(line) for line in open(sys.argv[1])]
if not any(v["operation"]=="buildx" and v.get("target")=="artifacts" for v in calls):raise SystemExit(1)
if not any(v["operation"]=="compose-build" and v.get("dockerfile")=="consumer" and v.get("route")=="source" for v in calls):raise SystemExit(1)
if not any(v["operation"]=="compose-build" and v.get("route","").startswith("/") for v in calls):raise SystemExit(1)
for v in calls:
    if v["operation"]=="compose-build" and (v.get("cargo_build_jobs")!="2" or v.get("compose_parallel_limit")!="1"):raise SystemExit(1)
sha=[v for v in calls if v["operation"]=="run" and "--entrypoint" in v["argv"] and "/usr/bin/sha256sum" in v["argv"]]
if not sha or any("/bin/sh" in v["argv"] or "-c" in v["argv"] for v in sha):raise SystemExit(1)
PY
  local artifact; artifact=$(find "$root/runs/common-cold" -path '*/private-p1/artifact.json' -print -quit)
  [ -n "$artifact" ] || return 1
  local adir; adir=$(dirname "$artifact")
  self_publication_contract "$root/publication" "$adir" || return 1
  self_cargo_negative_contract "$root/cargo-negative" "$adir/cargo.jsonl" || return 1
  local dup="$root/dup" partial="$root/partial" link="$root/link"
  cp -a -- "$adir" "$dup"; printf '{"format":"x","format":"y"}\n' >"$dup/artifact.json"; self_expect_failure bash "$helper_path" artifact-verify "$dup" "$c1" build-cache-fixture-app cache-bin-a linux/amd64 x86_64-unknown-linux-musl release '[]' x x x x x
  cp -a -- "$adir" "$partial"; printf partial >"$partial/COMPLETE"; self_expect_failure bash "$helper_path" artifact-verify "$partial" "$c1" build-cache-fixture-app cache-bin-a linux/amd64 x86_64-unknown-linux-musl release '[]' x x x x x
  cp -a -- "$adir" "$link"; rm -f -- "$link/bin/cache-bin-a"; ln -s /etc/passwd "$link/bin/cache-bin-a"; self_expect_failure bash "$helper_path" artifact-verify "$link" "$c1" build-cache-fixture-app cache-bin-a linux/amd64 x86_64-unknown-linux-musl release '[]' x x x x x
  local sentinel="$root/docker-sentinel" rejected="$root/rejected"
  printf '%s\n' '#!/bin/sh' 'printf invoked >>"$SENTINEL"' >"$sentinel"; chmod 0755 "$sentinel"
  self_expect_failure env -u BUILD_LAYOUT_HEALTH_UNITS -u BUILD_LAYOUT_HEALTH_CONTAINERS DOCKER_BIN="$sentinel" SENTINEL="$root/sentinel.log" bash "$script_path" --apply --layout baseline --case cold --output-dir "$rejected"
  [ ! -e "$rejected" ] && [ ! -e "$root/sentinel.log" ] || return 1
  unset BUILD_LAYOUT_SELF_TEST_ACTIVE LAYOUT_GATE_TEST_SEAM LAYOUT_GATE_MODE LAYOUT_FAKE_GATE_RESULT FAKE_DOCKER_ROOT FAKE_DOCKER_CALLS FAKE_LAYOUT_HELPER
  printf '%s\n' 'build-layout-probe: self-test PASS (Docker/Cargo/host gate NOT_RUN)'
}

main() {
  parse_cli "$@"
  if [ "$mode" = self-test ]; then
    [ "$layout_seen" = 0 ] && [ "$case_seen" = 0 ] && [ "$output_seen" = 0 ] && [ "$resume_seen" = 0 ] || die '--self-test accepts no selectors'
    self_test
    return 0
  fi
  sanitize_actual_environment
  case "$layout_selection" in baseline|common|grouped|all) ;; *) die "unknown layout: $layout_selection" ;; esac
  case_selection=$(parse_case "$case_selection")
  validate_fixture
  hash_inputs
  if [ "$resume_seen" = 1 ]; then
    [ "$output_seen" = 0 ] && [ "$layout_seen" = 0 ] && [ "$case_seen" = 0 ] || die '--resume-from cannot be combined with output-dir/layout/case'
    validate_resume
    if [ "$mode" = plan ]; then print_resume_plan; return $?; fi
    validate_health_inputs
    acquire_execution_slot || return 1
    load_resume_state || return $?
    probe_gate "${RESUME_CASE:-complete}" resume-before 0 || { set_run_status failed 1; return 1; }
    validate_all_saved_images || return 1
    probe_gate "${RESUME_CASE:-complete}" resume-after 0 || { set_run_status failed 1; return 1; }
    if [ "$RESUME_COMPLETE" = 1 ]; then printf 'build-layout-probe: already-complete; validated_phase_count=%s; builds=0\n' "$RESUME_VALIDATED_PHASES"; return 0; fi
    RESUME_ACTIVE=1
    run_suite
    return $?
  fi
  [ "$output_seen" = 1 ] || die '--output-dir is required for a new run'
  validate_output
  LAYOUT_RESEARCH_BINDING=$(research_exception_check) || return 1
  [ "$mode" = plan ] && { print_plan; return 0; }
  validate_health_inputs
  acquire_execution_slot || return 1
  validate_output
  init_run
  RESUME_ACTIVE=0
  run_suite
}

main "$@"
