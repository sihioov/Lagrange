#!/usr/bin/env bash
# Existing-host build-cache benchmark harness.
#
# The default is a read-only plan.  --apply is deliberately guarded: it uses
# detached temporary Git clones, temporary instrumented Dockerfiles, unique
# benchmark-only image tags/cache namespaces, one Docker invocation at a time,
# and a pre-existing low-priority systemd service.  --self-test only uses
# canned logs and shell-function fakes; it never contacts a Docker daemon or
# compiles Rust.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/../.." && pwd -P)
layout_helper=$repo_root/scripts/ops/lib/release-build-layout.sh
manifest_library=$repo_root/scripts/ops/lib/release-image-manifest.sh

readonly product_a_commit=d1baf9da9b13fcb61649b1c26de56aed87a83418
readonly product_b_commit=f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3
readonly benchmark_compose_env_sha256_expected=df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6

# G2 makes the helper's parent-held lock part of the whole-release benchmark
# contract. Sourcing it only exposes library functions; plan/self-test still
# make no Docker or production-service call.
[ -f "$layout_helper" ] && [ ! -L "$layout_helper" ] || {
  echo "build-cache-benchmark: required G2 layout helper is missing: $layout_helper" >&2
  exit 1
}
[ -f "$manifest_library" ] && [ ! -L "$manifest_library" ] || {
  echo "build-cache-benchmark: required strict V2 manifest library is missing: $manifest_library" >&2
  exit 1
}
# shellcheck source=../ops/lib/release-build-layout.sh
source "$layout_helper"

mode=plan
mode_seen=0
cache_mode=warm
cache_mode_seen=0
scenario=rust-leaf
scenario_seen=0
repetitions=1
repetitions_seen=0
measurement_order=baseline-first
measurement_order_seen=0
baseline_commit=
candidate_commit=
output_dir=

# These are conservative harness gates for the documented 14 GiB host, not a
# deployment authorization.  An operator may raise either value, never lower
# it, through the corresponding BENCHMARK_MIN_* variable.
readonly default_min_mem_available_kib=2097152 # 2 GiB
readonly default_min_swap_free_kib=524288      # 512 MiB
readonly journal_lookback_seconds=1800

# --self-test changes this internal value only after the CLI has selected the
# test mode.  It is intentionally not an environment-controlled bypass for
# --apply.
internal_self_test=0

services=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  owner-beta-runner
  owner-equity-v2-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
)
declare -A service_dockerfile=(
  [db-role-bootstrap]=deploy/db/Dockerfile
  [db-migrate]=deploy/db/Dockerfile
  [api-server]=crates/api-server/Dockerfile
  [web]=apps/web/Dockerfile
  [research-worker]=data-pipelines/collectors/Dockerfile
  [recommendation-runner]=crates/job-queue/Dockerfile
  [candidate-runner]=crates/job-queue/Dockerfile
  [owner-beta-runner]=crates/job-queue/Dockerfile.owner-beta-runner
  [owner-equity-v2-runner]=crates/job-queue/Dockerfile.owner-equity-v2-runner
  [nt-backtest-worker-1]=crates/job-queue/Dockerfile.backtest-runner
  [nt-backtest-worker-2]=crates/job-queue/Dockerfile.backtest-runner
  [paper-scheduler]=deploy/runtime/Dockerfile.paper-runner
)
# 0 has no Cargo compiler vertex.  1 is a workspace Cargo build.  2 is the
# database cargo-install compiler vertex, which deliberately has no workspace
# clean contract.
declare -A service_cargo_mode=(
  [db-role-bootstrap]=2 [db-migrate]=2 [api-server]=1 [web]=0
  [research-worker]=1 [recommendation-runner]=1 [candidate-runner]=1
  [owner-beta-runner]=1 [owner-equity-v2-runner]=1
  [nt-backtest-worker-1]=1 [nt-backtest-worker-2]=1 [paper-scheduler]=1
)

# A benchmark invocation measures exactly one representative change.  The
# transformation is identical in each detached baseline/candidate checkout;
# it is committed only in that disposable checkout so the image label and the
# measured tree agree.  These are input-invalidation probes, not application
# feature changes or a substitute for the fixture's semantic assertions.
benchmark_scenarios=(
  commit-only
  rust-leaf
  rust-common-library
  lockfile-comment
  build-script
  configuration-input
  web-only
  python-only
)
declare -A scenario_input_path=(
  [commit-only]=-
  [rust-leaf]=crates/api-server/src/bin/api-server.rs
  [rust-common-library]=crates/domain/src/lib.rs
  [lockfile-comment]=Cargo.lock
  [build-script]=crates/api-server/build.rs
  [configuration-input]=configs/strategies/baseline-v1.json
  [web-only]=apps/web/app/layout.tsx
  [python-only]=nt/backtest-worker/backtest_worker/__init__.py
)
declare -A scenario_transform=(
  [commit-only]=synthetic-child-commit-identical-tree
  [rust-leaf]=append-rust-comment
  [rust-common-library]=append-rust-comment
  [lockfile-comment]=append-toml-comment
  [build-script]=append-rust-comment
  [configuration-input]=prefix-json-whitespace
  [web-only]=append-tsx-comment
  [python-only]=append-python-comment
)
declare -A scenario_expected_scope=(
  [commit-only]=all-image-revision-inputs-no-source-tree-delta
  [rust-leaf]=api-server-source-input
  [rust-common-library]=shared-rust-library-input
  [lockfile-comment]=rust-lockfile-input-only-not-a-dependency-update
  [build-script]=api-server-build-script-input
  [configuration-input]=db-runtime-config-input
  [web-only]=web-source-input
  [python-only]=nt-runtime-python-input
)

tmp_dir=
baseline_checkout=
candidate_checkout=
benchmark_nonce=
evidence_dir=
instrumentation_dir=
metadata_report=
results_report=
resource_report=
resource_samples_report=
peak_resource_report=
phase_report=
comparison_report=
source_identity_report=
instrumentation_report=
probe_spec_report=
instrumentation_manifest_hash=
cargo_units_report=
native_identity_report=
archive_validation_report=
disk_report=
disk_summary_report=
common_layout_report=
common_phase_report=
common_producer_report=
common_manifest_report=
release_total_report=
source_manifest_report=
release_comparison_report=
failure_report=
benchmark_compose_env_file=
benchmark_compose_env_sha256=
apply_record_failures=0
failure_stage=validation
last_build_exit=not-started
min_mem_available_kib=$default_min_mem_available_kib
min_swap_free_kib=$default_min_swap_free_kib

benchmark_script_hash=
bash_identity=
git_identity=
python_identity=
host_platform_identity=
docker_platform_identity=
docker_root_dir=
sampler_pid=
sampler_file=
sampler_failure_file=
active_pair_index=1
journal_since_utc=
journal_first_gate_utc=
journal_established_boot_id=
journal_until_utc=unavailable
journal_probe_status=unavailable
journal_probe_entry_count=unavailable
journal_probe_stdout_sha256=unavailable
journal_probe_stderr_sha256=unavailable
journal_lookback_status=unavailable
journal_lookback_entry_count=unavailable
journal_lookback_oom_count=unavailable
journal_lookback_stdout_sha256=unavailable
journal_lookback_stderr_sha256=unavailable
journal_capture_status=unavailable
journal_capture_sha256=unavailable
journal_capture_failure=unavailable
journal_observation_reason=unavailable
source_manifest_active=0

declare -a tag_list=()
declare -A instrumented_dockerfile=()
declare -A source_dockerfile_hash=()
declare -A instrumented_dockerfile_hash=()
declare -A instrumentation_patch_hash=()
declare -A instrumentation_transform=()
declare -A service_toolchain_identity=()
declare -A source_identity_by_revision_scenario=()
declare -A scenario_commit_by_revision=()
declare -A scenario_commit_by_revision_pair=()
declare -A side_layout_kind=()
declare -A side_product_kind=()
declare -A side_layout_helper=()
declare -A side_layout_helper_hash=()
declare -A side_layout_config_hash=()
declare -A side_layout_artifact_hash=()
declare -A side_manifest_library_hash=()
declare -A common_state_root_by_run=()
declare -A common_bundle_digest_by_recipe=()
declare -A native_identity_source_by_recipe=()
declare -A native_identity_source_hash_by_recipe=()

# The benchmark-wide guard is initialized from the clean coordinator checkout,
# not from either A/B/C measurement checkout.  Its state is persistent for the
# complete invocation while each public gate runs in a fresh helper shell.
benchmark_gate_source_root=
benchmark_gate_source_commit=
benchmark_gate_state_root=
benchmark_gate_namespace=
benchmark_gate_helper=
benchmark_gate_lock_prefix=
benchmark_gate_initialized=0
benchmark_tmp_base=

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-cache-benchmark.sh [--plan|--apply|--self-test]
       --baseline-commit <40 lowercase hex>
       --candidate-commit <40 lowercase hex>
       --output-dir <absolute path>
       [--warm|--cold]
       [--repetitions <1|2|3>]
       [--order <baseline-first|candidate-first|alternating>]
       [--scenario <commit-only|rust-leaf|rust-common-library|lockfile-comment|build-script|configuration-input|web-only|python-only>]

Modes:
  --plan       Validate revisions/paths and print the benchmark plan (default).
  --apply      Run guarded sequential benchmark builds from temporary clones.
  --self-test  Exercise parser/state-machine logic with fakes only; it never
               contacts Docker, starts containers, or compiles Rust.

Warm mode gives baseline and candidate separate, invocation-specific
BUILDKIT_CACHE_MOUNT_NS values.  Each revision is warmed from its unchanged
temporary source, then receives the one selected, recorded source
transformation before its measured build.  Cold mode uses fresh per-revision
namespaces plus one stable, earliest-builder-stage cache nonce per side and
records independent cold measurements; it makes no warm comparison claim. It
never treats --no-cache alone as evidence of an empty Cargo cache. The default
scenario is rust-leaf. Run one
scenario per invocation so its preparation, build, and verification timing
cannot be hidden inside another scenario.

`--repetitions` is bounded to 1..3 and defaults to one paired observation for
compatibility. `--order` defaults to `baseline-first`.  Multiple pairs require
`alternating`, whose exact order is baseline/candidate, candidate/baseline,
then baseline/candidate. `--cold` permits only one pair because it is an
independent cold observation, not a repeated warm comparison.

--apply never starts a service.  Before it can build, the caller must already
be running inside BENCHMARK_SYSTEMD_SERVICE (a .service with positive Nice and
idle or low-priority best-effort I/O scheduling), a comma-separated
BENCHMARK_PRODUCTION_HEALTH_UNITS list, and a comma-separated
BENCHMARK_PRODUCTION_HEALTH_CONTAINERS list of exact current serving container
names or full IDs.  Every named container must be running, have a declared
healthy healthcheck, and belong to Compose project lagrange-station.  The
harness only inspects those bounded status/identity fields.  It also requires
at least 2 GiB MemAvailable and 512 MiB SwapFree by default; BENCHMARK_MIN_*
may only raise those fail-closed gates.  An operator, not this script, may
launch the background unit separately with this shape:
  systemd-run --no-block --unit=lagrange-cache-benchmark --property=Nice=10 \
    --property=IOSchedulingClass=idle \
    --setenv='BENCHMARK_SYSTEMD_SERVICE=lagrange-cache-benchmark.service' \
    --setenv='BENCHMARK_PRODUCTION_HEALTH_UNITS=<operator-control-plane-health-unit-1.service>,<operator-control-plane-health-unit-2.service>' \
    --setenv='BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=<operator-current-production-container-1>,<operator-current-production-container-2>' \
    /path/to/build-cache-benchmark.sh --apply ...
The placeholders are operator-selected current production names.  This script
does not create background production work or grant deployment permission.
EOF
}

record_apply_failure() {
  local message=$1 safe_message
  [ "$apply_record_failures" -eq 1 ] || return 0
  [ -n "$failure_report" ] && [ -d "${failure_report%/*}" ] || return 0
  safe_message=${message//$'\n'/ }
  safe_message=${safe_message//$'\r'/ }
  printf 'stage\t%s\nreason\t%s\n' "$failure_stage" "$safe_message" >"$failure_report" || return 0
  chmod 0600 -- "$failure_report" 2>/dev/null || true
}

die() {
  local message=$*
  record_apply_failure "$message"
  echo "build-cache-benchmark: $message" >&2
  exit 1
}

is_exact_commit() {
  local value=${1:-}
  [ -n "$value" ] || return 1
  [ "$value" != 0000000000000000000000000000000000000000 ] || return 1
  printf '%s' "$value" | grep -Eq '^[0-9a-f]{40}$'
}

is_decimal() {
  printf '%s' "$1" | grep -Eq '^[0-9]+$'
}

validate_measurement_protocol() {
  is_decimal "$repetitions" || die '--repetitions must be an integer in 1..3'
  [ "$repetitions" -ge 1 ] && [ "$repetitions" -le 3 ] ||
    die '--repetitions must be an integer in 1..3'
  case "$measurement_order" in
    baseline-first|candidate-first|alternating) ;;
    *) die '--order must be baseline-first, candidate-first, or alternating' ;;
  esac
  if [ "$repetitions" -gt 1 ] && [ "$measurement_order" != alternating ]; then
    die '--repetitions greater than one requires --order alternating'
  fi
  if [ "$cache_mode" = cold ] && [ "$repetitions" -gt 1 ]; then
    die '--cold cannot be combined with --repetitions greater than one'
  fi
}

pair_order() {
  local pair_index=$1
  is_decimal "$pair_index" && [ "$pair_index" -ge 1 ] && [ "$pair_index" -le "$repetitions" ] ||
    die 'benchmark pair index is outside the selected repetition range'
  case "$measurement_order" in
    baseline-first) printf '%s' baseline,candidate ;;
    candidate-first) printf '%s' candidate,baseline ;;
    alternating)
      case "$pair_index" in
        1|3) printf '%s' baseline,candidate ;;
        2) printf '%s' candidate,baseline ;;
        *) die 'alternating order supports at most three pairs' ;;
      esac
      ;;
    *) die 'benchmark measurement order is invalid' ;;
  esac
}

encode_benchmark_nonce() {
  local suffix=$1 encoded
  printf '%s' "$suffix" | grep -Eq '^[A-Za-z0-9]{6,}$' || return 1
  # Encode every byte of the complete mktemp suffix.  Two lower-hex digits per
  # byte are injective, retain case distinctions, and fit Docker's lowercase
  # repository grammar without truncating the random portion.
  encoded=$(LC_ALL=C printf '%s' "$suffix" | od -An -tx1 | LC_ALL=C tr -d ' \n') || return 1
  printf '%s' "$encoded" | grep -Eq '^[0-9a-f]+$' || return 1
  printf '%s' "$encoded"
}

safe_benchmark_repository() {
  # Benchmark images deliberately have a repository-only reference (no
  # implicit mixed-case tag); keep the generated single component Docker-safe.
  printf '%s' "$1" | grep -Eq '^[a-z0-9]+([._-][a-z0-9]+)*$'
}

safe_absolute_path() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    *$'\n'*|*$'\r'*|*'//'*) die "$label is not a canonical absolute path" ;;
    */../*|*/..|*/./*|*/.) die "$label must not contain dot path components" ;;
    */) die "$label must not have a trailing slash" ;;
  esac
  case "$path" in
    /|/tmp|/var|/var/lib|/usr|/usr/local|/opt) die "$label is too broad: $path" ;;
  esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

validate_output_path() {
  safe_absolute_path "$output_dir" output-dir
  case "$output_dir" in
    "$repo_root"|"$repo_root"/*) die 'output-dir must be outside the original repository checkout' ;;
  esac
  if [ -e "$output_dir" ]; then
    [ -d "$output_dir" ] && [ ! -L "$output_dir" ] ||
      die 'output-dir must be a directory when it already exists'
  else
    local parent=${output_dir%/*}
    [ -n "$parent" ] || parent=/
    [ -d "$parent" ] && [ ! -L "$parent" ] ||
      die 'output-dir parent must exist and must not be a symlink'
  fi
}

validate_revisions() {
  is_exact_commit "$baseline_commit" ||
    die '--baseline-commit must be exactly 40 lowercase hexadecimal characters'
  is_exact_commit "$candidate_commit" ||
    die '--candidate-commit must be exactly 40 lowercase hexadecimal characters'
  [ "$baseline_commit" != "$candidate_commit" ] ||
    die 'baseline and candidate commits must differ; same-input repetition is not a benchmark'
  command -v git >/dev/null 2>&1 || die 'git is required to validate benchmark revisions'
  git -c "safe.directory=$repo_root" -C "$repo_root" \
    cat-file -e "$baseline_commit^{commit}" 2>/dev/null ||
    die 'baseline commit is not present in the repository object database'
  git -c "safe.directory=$repo_root" -C "$repo_root" \
    cat-file -e "$candidate_commit^{commit}" 2>/dev/null ||
    die 'candidate commit is not present in the repository object database'
}

validate_service_contract() {
  local service dockerfile
  for service in "${services[@]}"; do
    dockerfile=$repo_root/${service_dockerfile[$service]}
    [ -f "$dockerfile" ] && [ ! -L "$dockerfile" ] ||
      die "benchmark Dockerfile is missing: ${service_dockerfile[$service]}"
  done
}

safe_scalar() {
  local value=$1 cleaned
  cleaned=$(printf '%s' "$value" | tr -cd '[:alnum:].,_:@/+|=-')
  [ -n "$cleaned" ] && printf '%s' "$cleaned" || printf '%s' unavailable
}

sha256_file() {
  local path=$1 value
  command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required for benchmark evidence'
  value=$(sha256sum -- "$path" | awk '{print $1}')
  printf '%s' "$value" | grep -Eq '^[0-9a-f]{64}$' || die "could not hash benchmark evidence: $path"
  printf '%s' "$value"
}

sha256_text() {
  local value=$1 digest
  digest=$(printf '%s' "$value" | sha256sum | awk '{print $1}')
  printf '%s' "$digest" | grep -Eq '^[0-9a-f]{64}$' || die 'could not hash benchmark metadata'
  printf '%s' "$digest"
}

write_benchmark_compose_env() {
  local destination=$1 parent digest mode
  case "$destination" in /*) ;; *) die 'benchmark image-only Compose env path must be absolute' ;; esac
  parent=${destination%/*}
  [ -d "$parent" ] && [ ! -L "$parent" ] ||
    die 'benchmark image-only Compose env parent is not a private regular directory'
  [ "$(stat -c %a -- "$parent")" = 700 ] ||
    die 'benchmark image-only Compose env parent mode must be 0700'
  [ ! -e "$destination" ] && [ ! -L "$destination" ] ||
    die 'benchmark image-only Compose env already exists; refusing to overwrite it'
  if ! (
    umask 077
    set -o noclobber
    printf 'RESEARCH_ENTITLEMENT_SHA256=%064d\n' 0 >"$destination"
  ); then
    die 'could not create the private benchmark image-only Compose env'
  fi
  chmod 0600 -- "$destination" || die 'could not protect the benchmark image-only Compose env'
  [ -f "$destination" ] && [ ! -L "$destination" ] ||
    die 'benchmark image-only Compose env is not a regular file'
  mode=$(stat -c %a -- "$destination")
  [ "$mode" = 600 ] || die 'benchmark image-only Compose env mode must be 0600'
  digest=$(sha256_file "$destination")
  [ "$digest" = "$benchmark_compose_env_sha256_expected" ] ||
    die 'benchmark image-only Compose env content did not match the frozen inactive sentinel'
  benchmark_compose_env_file=$destination
  benchmark_compose_env_sha256=$digest
}

csv_or_dash() {
  [ -n "$1" ] && printf '%s' "$1" || printf '%s' '-'
}

sanitize_text_file() {
  local source=$1 destination=$2
  # BuildKit output is evidence, not a license to persist an accidental
  # environment/config dump.  Preserve useful Cargo/vertex lines while
  # replacing an entire suspicious line rather than trying to redact values.
  awk '
    {
      line = $0
      lower = tolower(line)
      if (length(line) > 8192) {
        line = substr(line, 1, 8192) " [truncated]"
        lower = tolower(line)
      }
      if (lower ~ /(password|passwd|secret|token|api[_-]?key|crtfc_key|authorization|cookie|cano|acnt_prdt_cd|kis_account_ref|private[ _-]?key)/) {
        print "[redacted sensitive build output]"
      } else {
        print line
      }
    }
  ' "$source" >"$destination"
  chmod 0600 -- "$destination"
}

duration_to_ms() {
  local duration=$1
  if [[ "$duration" =~ ^([0-9]+)ms$ ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  elif [[ "$duration" =~ ^([0-9]+)(\.[0-9]+)?s$ ]]; then
    awk -v seconds="$duration" 'BEGIN { sub(/s$/, "", seconds); printf "%.0f", seconds * 1000 }'
  elif [[ "$duration" =~ ^([0-9]+)m([0-9]+)(\.[0-9]+)?s$ ]]; then
    awk -v minutes="${BASH_REMATCH[1]}" -v seconds="${BASH_REMATCH[2]}${BASH_REMATCH[3]:-}" \
      'BEGIN { printf "%.0f", (minutes * 60 + seconds) * 1000 }'
  else
    return 1
  fi
}

# Globals written by parse_build_events.  Package names are constrained before
# they leave the parser; raw build output is kept only as sanitized evidence.
BENCH_COMPILED_PACKAGES=
BENCH_FRESH_PACKAGES=
BENCH_COMPILED_COUNT=0
BENCH_FRESH_COUNT=0
BENCH_CARGO_STEP_MS=
BENCH_COMPILER_VERTEX=
BENCH_COMPILER_VERTEX_STATE=not-applicable
BENCH_COMPILER_CACHE=not-applicable
BENCH_NONCOMPILER_CACHED=none
BENCH_WORKSPACE_CLEAN=not-applicable
BENCH_PARSE_ERROR=
BENCH_CARGO_UNITS_PATH=
BENCH_CARGO_UNITS_SHA256=not-applicable
BENCH_CARGO_UNITS_COUNT=0
BENCH_CARGO_RAW_LOG_SHA256=

parse_fail() {
  BENCH_PARSE_ERROR=$1
  return 1
}

package_list_for_vertex() {
  local log=$1 vertex=$2 word=$3
  awk -v vertex="$vertex" -v word="$word" '
    $0 ~ "^#" vertex "[[:space:]]+" {
      line = $0
      if (line ~ word "[[:space:]]+") {
        sub(".*" word "[[:space:]]+", "", line)
        split(line, fields, /[[:space:]]+/)
        if (fields[1] ~ /^[A-Za-z0-9_.-]+$/) print fields[1]
      }
    }
  ' "$log" | sort -u | paste -sd, -
}

count_csv() {
  [ -n "$1" ] && awk -F, '{print NF}' <<<"$1" || printf '0'
}

parse_cargo_json_units() {
  local log=$1 vertex=$2 cargo_mode=$3 destination=$4
  [ -n "$destination" ] || return 0
  [ -d "${destination%/*}" ] && [ ! -L "${destination%/*}" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  RBL_BENCH_LOG=$log RBL_BENCH_VERTEX=$vertex RBL_BENCH_CARGO_MODE=$cargo_mode \
    RBL_BENCH_UNITS=$destination python3 - <<'PY'
import hashlib
import json
import os
import re

log_path = os.environ["RBL_BENCH_LOG"]
vertex = os.environ["RBL_BENCH_VERTEX"]
cargo_mode = os.environ["RBL_BENCH_CARGO_MODE"]
out_path = os.environ["RBL_BENCH_UNITS"]

if not re.fullmatch(r"[0-9]+", vertex) or cargo_mode not in {"1", "2"}:
    raise SystemExit("benchmark-cargo-units-arguments-invalid")
raw = open(log_path, "rb").read()
if len(raw) > 256 * 1024 * 1024:
    raise SystemExit("benchmark-cargo-log-too-large")
try:
    text = raw.decode("utf-8")
except UnicodeDecodeError:
    raise SystemExit("benchmark-cargo-log-not-utf8")

def pairs(items):
    value = {}
    for key, item in items:
        if key in value:
            raise ValueError("duplicate-json-key")
        value[key] = item
    return value

def bounded_text(value, label, limit=4096):
    if not isinstance(value, str) or not value or len(value) > limit:
        raise SystemExit("benchmark-cargo-%s-invalid" % label)
    if any(ord(char) < 32 or ord(char) == 127 for char in value):
        raise SystemExit("benchmark-cargo-%s-control" % label)
    return value

def text_list(value, label, limit):
    if not isinstance(value, list) or len(value) > limit:
        raise SystemExit("benchmark-cargo-%s-list-invalid" % label)
    output = []
    for item in value:
        output.append(bounded_text(item, label, 512))
    if len(set(output)) != len(output):
        raise SystemExit("benchmark-cargo-%s-duplicate" % label)
    return sorted(output)

prefix = re.compile(r"^#" + re.escape(vertex) + r"\s+(?:[0-9]+(?:\.[0-9]+)?s?\s+)?(?P<payload>\{.*\})\s*$")
units = []
unit_keys = set()
finished = []
json_lines = 0
for line in text.splitlines():
    match = prefix.match(line)
    if match is None:
        continue
    payload = match.group("payload")
    if len(payload.encode("utf-8")) > 1024 * 1024:
        raise SystemExit("benchmark-cargo-json-line-too-large")
    try:
        event = json.loads(payload, object_pairs_hook=pairs,
                           parse_constant=lambda _value: (_ for _ in ()).throw(ValueError("invalid-json-constant")))
    except (ValueError, json.JSONDecodeError):
        raise SystemExit("benchmark-cargo-json-invalid")
    if not isinstance(event, dict):
        raise SystemExit("benchmark-cargo-json-object-invalid")
    json_lines += 1
    if json_lines > 100000:
        raise SystemExit("benchmark-cargo-json-too-many-events")
    reason = event.get("reason")
    if reason == "compiler-artifact":
        package_id = bounded_text(event.get("package_id"), "package-id")
        target = event.get("target")
        profile = event.get("profile")
        features = event.get("features")
        fresh = event.get("fresh")
        executable = event.get("executable")
        if not isinstance(target, dict) or not {"name", "kind", "crate_types"} <= set(target):
            raise SystemExit("benchmark-cargo-target-invalid")
        target_name = bounded_text(target.get("name"), "target-name", 512)
        kinds = text_list(target.get("kind"), "target-kind", 32)
        crate_types = text_list(target.get("crate_types"), "target-crate-types", 32)
        if not isinstance(profile, dict) or not profile or len(profile) > 24:
            raise SystemExit("benchmark-cargo-profile-invalid")
        safe_profile = {}
        for key, value in profile.items():
            if not isinstance(key, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
                raise SystemExit("benchmark-cargo-profile-key-invalid")
            if isinstance(value, bool) or value is None:
                safe_profile[key] = value
            elif isinstance(value, int) and not isinstance(value, bool) and -1 <= value <= 100:
                safe_profile[key] = value
            elif isinstance(value, str) and len(value) <= 128 and not any(ord(char) < 32 or ord(char) == 127 for char in value):
                safe_profile[key] = value
            else:
                raise SystemExit("benchmark-cargo-profile-value-invalid")
        if not isinstance(fresh, bool):
            raise SystemExit("benchmark-cargo-fresh-invalid")
        if executable is not None:
            executable = bounded_text(executable, "executable")
            if not executable.startswith("/") or "/../" in executable or executable.endswith("/.."):
                raise SystemExit("benchmark-cargo-executable-invalid")
        key = (package_id, target_name, tuple(kinds), tuple(crate_types))
        if key in unit_keys:
            raise SystemExit("benchmark-cargo-unit-duplicate")
        unit_keys.add(key)
        units.append({
            "package_id": package_id,
            "target": {"name": target_name, "kind": kinds, "crate_types": crate_types},
            "features": text_list(features, "features", 512),
            "profile": {key: safe_profile[key] for key in sorted(safe_profile)},
            "fresh": fresh,
            "executable": executable,
            "build_success": None,
        })
    elif reason == "build-finished":
        if set(event) != {"reason", "success"} or not isinstance(event["success"], bool):
            raise SystemExit("benchmark-cargo-finish-invalid")
        finished.append(event["success"])

if not units:
    raise SystemExit("benchmark-cargo-units-missing")
if finished != [True]:
    raise SystemExit("benchmark-cargo-build-success-invalid")
for unit in units:
    unit["build_success"] = True
units.sort(key=lambda item: (item["package_id"], item["target"]["name"], item["target"]["kind"], item["target"]["crate_types"]))
result = {
    "format": "lagrange-benchmark-cargo-units-v1",
    "raw_log_sha256": hashlib.sha256(raw).hexdigest(),
    "compiler_vertex": vertex,
    "cargo_mode": int(cargo_mode),
    "units": units,
}
data = (json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
PY
}

parse_build_events() {
  local log=$1 cargo_mode=$2 revision=$3 units_path=${4:-} vertex_header done_duration duration_ms
  local -a vertices=()

  BENCH_COMPILED_PACKAGES=
  BENCH_FRESH_PACKAGES=
  BENCH_COMPILED_COUNT=0
  BENCH_FRESH_COUNT=0
  BENCH_CARGO_STEP_MS=
  BENCH_COMPILER_VERTEX=
  BENCH_COMPILER_VERTEX_STATE=not-applicable
  BENCH_COMPILER_CACHE=not-applicable
  BENCH_NONCOMPILER_CACHED=none
  BENCH_WORKSPACE_CLEAN=not-applicable
  BENCH_PARSE_ERROR=
  BENCH_CARGO_UNITS_PATH=
  BENCH_CARGO_UNITS_SHA256=not-applicable
  BENCH_CARGO_UNITS_COUNT=0
  BENCH_CARGO_RAW_LOG_SHA256=$(sha256_file "$log" 2>/dev/null || true)

  [ -f "$log" ] || { parse_fail 'build event log is missing'; return 1; }
  case "$cargo_mode" in 0|1|2) ;; *) parse_fail 'unknown Cargo contract mode'; return 1 ;; esac
  case "$revision" in baseline|candidate) ;; *) parse_fail 'unknown revision contract'; return 1 ;; esac

  if [ "$cargo_mode" -eq 0 ]; then
    if grep -Eq '^#[0-9]+[[:space:]]+CACHED([[:space:]]|$)' "$log"; then
      BENCH_NONCOMPILER_CACHED=present
    fi
    return 0
  fi

  while IFS= read -r vertex; do
    [ -n "$vertex" ] && vertices+=("$vertex")
  done < <(
    awk '
      /^#[0-9]+[[:space:]]+.*RUN/ && /cargo[[:space:]]+(build|install)([[:space:]]|$)/ {
        id = $0
        sub(/^#/, "", id)
        sub(/[[:space:]].*$/, "", id)
        print id
      }
    ' "$log" | sort -u
  )
  [ "${#vertices[@]}" -eq 1 ] || {
    parse_fail 'could not identify exactly one Cargo compile/install BuildKit vertex'
    return 1
  }
  BENCH_COMPILER_VERTEX=${vertices[0]}

  if grep -Eq "^#${BENCH_COMPILER_VERTEX}[[:space:]]+.*ERROR" "$log"; then
    parse_fail 'Cargo compiler vertex reported a BuildKit error'
    return 1
  fi
  if grep -Eq "^#${BENCH_COMPILER_VERTEX}[[:space:]]+CACHED([[:space:]]|$)" "$log"; then
    BENCH_COMPILER_VERTEX_STATE=cached
    BENCH_COMPILER_CACHE=hit
    BENCH_CARGO_STEP_MS=0
    BENCH_WORKSPACE_CLEAN=not-executed
  else
    BENCH_COMPILER_VERTEX_STATE=executed
    BENCH_COMPILER_CACHE=miss
    done_duration=$(awk -v vertex="$BENCH_COMPILER_VERTEX" '
      $0 ~ "^#" vertex " DONE " {
        value = $0
        sub("^#" vertex " DONE ", "", value)
        print value
        exit
      }
    ' "$log")
    [ -n "$done_duration" ] || {
      parse_fail 'executed Cargo compiler vertex has no DONE duration'
      return 1
    }
    duration_ms=$(duration_to_ms "$done_duration") || {
      parse_fail 'executed Cargo compiler vertex has malformed DONE duration'
      return 1
    }
    is_decimal "$duration_ms" || {
      parse_fail 'executed Cargo compiler duration is not numeric'
      return 1
    }
    BENCH_CARGO_STEP_MS=$duration_ms

    if [ "$cargo_mode" -eq 1 ]; then
      if ! awk -v vertex="$BENCH_COMPILER_VERTEX" \
        '$0 ~ "^#" vertex "[[:space:]]+" && /Finished/ { found=1 } END { exit !found }' "$log"; then
        parse_fail 'executed Cargo build has no successful finish event'
        return 1
      fi
    else
      if ! awk -v vertex="$BENCH_COMPILER_VERTEX" \
        '$0 ~ "^#" vertex "[[:space:]]+" && /(Installed package|Finished)/ { found=1 } END { exit !found }' "$log"; then
        parse_fail 'executed cargo-install has no successful finish event'
        return 1
      fi
    fi

    vertex_header=$(awk -v vertex="$BENCH_COMPILER_VERTEX" '
      $0 ~ "^#" vertex "[[:space:]]+.*RUN" && /cargo[[:space:]]+(build|install)([[:space:]]|$)/ { print; exit }
    ' "$log")
    [ -n "$vertex_header" ] || {
      parse_fail 'Cargo compiler vertex header disappeared from build log'
      return 1
    }
    if grep -Fq 'cargo clean --workspace --release --locked' <<<"$vertex_header"; then
      BENCH_WORKSPACE_CLEAN=observed
    else
      BENCH_WORKSPACE_CLEAN=absent
    fi
    # The legacy baseline intentionally has no clean contract.  The candidate
    # is required to show its workspace clean only when that vertex executed.
    if [ "$revision" = candidate ] && [ "$cargo_mode" -eq 1 ] &&
      [ "$BENCH_WORKSPACE_CLEAN" != observed ]; then
      parse_fail 'executed candidate Cargo build lacks workspace-clean evidence'
      return 1
    fi
  fi

  if awk -v compiler="$BENCH_COMPILER_VERTEX" '
    /^#[0-9]+[[:space:]]+CACHED([[:space:]]|$)/ {
      id = $0
      sub(/^#/, "", id)
      sub(/[[:space:]].*$/, "", id)
      if (id != compiler) found=1
    }
    END { exit !found }
  ' "$log"; then
    BENCH_NONCOMPILER_CACHED=present
  fi

  BENCH_COMPILED_PACKAGES=$(package_list_for_vertex "$log" "$BENCH_COMPILER_VERTEX" Compiling)
  BENCH_FRESH_PACKAGES=$(package_list_for_vertex "$log" "$BENCH_COMPILER_VERTEX" Fresh)
  BENCH_COMPILED_COUNT=$(count_csv "$BENCH_COMPILED_PACKAGES")
  BENCH_FRESH_COUNT=$(count_csv "$BENCH_FRESH_PACKAGES")
  if [ "$BENCH_COMPILER_VERTEX_STATE" = executed ] && [ -n "$units_path" ]; then
    if ! parse_cargo_json_units "$log" "$BENCH_COMPILER_VERTEX" "$cargo_mode" "$units_path"; then
      parse_fail 'Cargo JSON unit evidence is missing, malformed, or not a successful bounded compile record'
      return 1
    fi
    BENCH_CARGO_UNITS_PATH=$units_path
    BENCH_CARGO_UNITS_SHA256=$(sha256_file "$units_path") || {
      parse_fail 'Cargo JSON unit evidence hash failed'
      return 1
    }
    BENCH_CARGO_UNITS_COUNT=$(python3 - "$units_path" <<'PY'
import json, sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
units=value.get("units")
if not isinstance(units,list): raise SystemExit(1)
print(len(units))
PY
) || {
      parse_fail 'Cargo JSON unit evidence count failed'
      return 1
    }
    is_decimal "$BENCH_CARGO_UNITS_COUNT" || {
      parse_fail 'Cargo JSON unit evidence count is invalid'
      return 1
    }
  elif [ -n "$units_path" ]; then
    # A cached compiler has no executed Cargo JSON stream. Keep that absence
    # explicit instead of fabricating unit evidence from a prior layer.
    BENCH_CARGO_UNITS_PATH=not-executed-cached
  fi
}

scenario_is_known() {
  local known
  for known in "${benchmark_scenarios[@]}"; do
    [ "$known" = "$1" ] && return 0
  done
  return 1
}

scenario_probe_line() {
  local pair_index=${1:-$active_pair_index}
  is_decimal "$pair_index" && [ "$pair_index" -ge 1 ] && [ "$pair_index" -le 3 ] ||
    die 'scenario probe pair index is invalid'
  case "$scenario" in
    rust-leaf|rust-common-library|build-script)
      printf '%s' "// LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915 scenario=$scenario pair=$pair_index"
      ;;
    lockfile-comment|python-only)
      printf '%s' "# LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915 scenario=$scenario pair=$pair_index"
      ;;
    web-only)
      printf '%s' "// LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915 scenario=web-only pair=$pair_index"
      ;;
    commit-only|configuration-input)
      printf '%s' '-'
      ;;
    *) die "unsupported benchmark scenario: $scenario" ;;
  esac
}

validate_scenario_contract() {
  scenario_is_known "$scenario" || die "unsupported benchmark scenario: $scenario"
  [ -n "${scenario_input_path[$scenario]:-}" ] || die "scenario has no input path: $scenario"
  [ -n "${scenario_transform[$scenario]:-}" ] || die "scenario has no transformation: $scenario"
  [ -n "${scenario_expected_scope[$scenario]:-}" ] || die "scenario has no expected scope: $scenario"
}

validate_checkout_contract() {
  local checkout=$1 revision=$2 service dockerfile path line
  for service in "${services[@]}"; do
    dockerfile=$checkout/${service_dockerfile[$service]}
    [ -f "$dockerfile" ] && [ ! -L "$dockerfile" ] ||
      die "$revision temporary checkout lacks Dockerfile: ${service_dockerfile[$service]}"
  done
  path=${scenario_input_path[$scenario]}
  [ "$path" = - ] && return 0
  [ -f "$checkout/$path" ] && [ ! -L "$checkout/$path" ] ||
    die "$revision temporary checkout lacks scenario input: $path"
  case "${scenario_transform[$scenario]}" in
    append-rust-comment|append-toml-comment|append-tsx-comment|append-python-comment)
      line=$(scenario_probe_line)
      if grep -Fqx "$line" "$checkout/$path"; then
        die "$revision scenario probe marker already exists: $path"
      fi
      ;;
    prefix-json-whitespace)
      [ "$(head -c 1 "$checkout/$path")" = '{' ] ||
        die "$revision configuration scenario input must begin with JSON object syntax: $path"
      ;;
    *) die "unsupported scenario transformation: ${scenario_transform[$scenario]}" ;;
  esac
}

detect_checkout_layout() {
  local revision=$1 checkout=$2 helper config artifact manifest present=0 path service
  [ "$revision" = baseline ] || [ "$revision" = candidate ] ||
    die "benchmark layout revision is invalid: $revision"
  [ -d "$checkout" ] && [ ! -L "$checkout" ] ||
    die "benchmark layout checkout is not a private directory: $revision"
  [ -z "$(git -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] ||
    die "benchmark layout checkout is not clean: $revision"
  helper=$checkout/scripts/ops/lib/release-build-layout.sh
  config=$checkout/deploy/build/release-build-layout.json
  artifact=$checkout/deploy/build/Dockerfile.rust-artifacts
  manifest=$checkout/scripts/ops/lib/release-image-manifest.sh
  for path in "$helper" "$config" "$artifact"; do
    if [ -e "$path" ] || [ -L "$path" ]; then
      present=$((present + 1))
    fi
  done
  if [ "$present" -eq 0 ]; then
    side_layout_kind["$revision"]=source
    case "$(git -C "$checkout" rev-parse HEAD)" in
      "$product_a_commit") side_product_kind["$revision"]=A ;;
      "$product_b_commit") side_product_kind["$revision"]=B ;;
      *) side_product_kind["$revision"]=source-unclassified ;;
    esac
    side_layout_helper["$revision"]=
    side_layout_helper_hash["$revision"]=not-applicable
    side_layout_config_hash["$revision"]=not-applicable
    side_layout_artifact_hash["$revision"]=not-applicable
    side_manifest_library_hash["$revision"]=$(sha256_file "$manifest")
    return 0
  fi
  [ "$present" -eq 3 ] ||
    die "benchmark checkout has a partial common-C layout and must not silently use source fallback: $revision"
  for path in "$helper" "$config" "$artifact" "$manifest"; do
    [ -f "$path" ] && [ ! -L "$path" ] ||
      die "benchmark common-C input is not a regular clean-checkout file: $revision/${path#$checkout/}"
  done
  # The frozen helper validates the complete selected layout without a Docker
  # call in plan mode.  Do not accept a merely named config as a C route.
  if [ "$internal_self_test" -eq 1 ] && declare -F benchmark_validate_common_fixture >/dev/null; then
    benchmark_validate_common_fixture "$revision" "$checkout" "$helper" "$config" "$artifact" \
      "$(git -C "$checkout" rev-parse HEAD)" ||
      die "benchmark common-C helper/layout fixture validation failed for clean checkout: $revision"
  elif ! RELEASE_BUILD_LAYOUT_SOURCE_ROOT="$checkout" \
      bash -c 'source "$1"; release_build_layout_plan "$2"' _ "$helper" \
      "$(git -C "$checkout" rev-parse HEAD)" >"$tmp_dir/${revision}-common-plan.log.raw" 2>&1; then
    die "benchmark common-C helper/layout plan validation failed for clean checkout: $revision"
  fi
  side_layout_kind["$revision"]=common
  side_product_kind["$revision"]=C
  side_layout_helper["$revision"]=$helper
  side_layout_helper_hash["$revision"]=$(sha256_file "$helper")
  side_layout_config_hash["$revision"]=$(sha256_file "$config")
  side_layout_artifact_hash["$revision"]=$(sha256_file "$artifact")
  side_manifest_library_hash["$revision"]=$(sha256_file "$manifest")
}

record_layout_identity() {
  local revision=$1 checkout=$2 service dockerfile clean_marker
  clean_marker=$(sha256_text $'lagrange-benchmark-common-c-clean-uninstrumented-v1\n')
  if [ "${side_layout_kind[$revision]:-}" = common ]; then
    for service in "${services[@]}"; do
      dockerfile=$checkout/${service_dockerfile[$service]}
      [ -f "$dockerfile" ] && [ ! -L "$dockerfile" ] ||
        die "common-C checkout Dockerfile is not a regular file: $revision/$service"
      source_dockerfile_hash["$revision:$service"]=$(sha256_file "$dockerfile")
      instrumented_dockerfile_hash["$revision:$service"]=${source_dockerfile_hash["$revision:$service"]}
      instrumentation_patch_hash["$revision:$service"]=$clean_marker
      instrumentation_transform["$revision:$service"]=common-c-clean-uninstrumented
      instrumented_dockerfile["$revision:$service"]=$dockerfile
      service_toolchain_identity["$revision:$service"]=$(safe_scalar \
        "common-helper=${side_layout_helper_hash[$revision]}|layout=${side_layout_config_hash[$revision]}|artifact=${side_layout_artifact_hash[$revision]}")
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$revision" "$service" "${service_dockerfile[$service]}" common-c-clean-uninstrumented \
        "${source_dockerfile_hash[$revision:$service]}" \
        "${instrumented_dockerfile_hash[$revision:$service]}" \
        "${instrumentation_patch_hash[$revision:$service]}" \
        "${service_toolchain_identity[$revision:$service]}" >>"$instrumentation_report"
    done
  else
    instrument_all_dockerfiles "$revision" "$checkout"
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "${side_product_kind[$revision]}" "${side_layout_kind[$revision]}" \
    "${side_layout_helper_hash[$revision]}" "${side_layout_config_hash[$revision]}" \
    "${side_layout_artifact_hash[$revision]}" "${side_manifest_library_hash[$revision]}" >>"$common_layout_report"
}

activate_benchmark_layout_helper() {
  local selected= lock_prefix=${RBL_LOCK_PREFIX:-/tmp/lagrange-production-image-build}
  case "${side_layout_kind[baseline]:-}:${side_layout_kind[candidate]:-}" in
    common:*) selected=${side_layout_helper[baseline]} ;;
    *:common) selected=${side_layout_helper[candidate]} ;;
    *) selected=$layout_helper ;;
  esac
  [ -f "$selected" ] && [ ! -L "$selected" ] ||
    die 'benchmark whole-lock helper is not a regular file'
  # A C side owns the selected helper bytes.  Source it in the parent solely
  # to own the inherited whole-run lock descriptor; each C release below
  # sources its own clean checkout helper again in a fresh subshell for init.
  # shellcheck disable=SC1090
  source "$selected"
  RBL_LOCK_PREFIX=$lock_prefix
  layout_helper=$selected
  if [ "$internal_self_test" -eq 1 ] && \
      { [ "${side_layout_kind[baseline]:-}" = common ] || [ "${side_layout_kind[candidate]:-}" = common ]; } && \
      declare -F benchmark_install_common_fake_api >/dev/null; then
    benchmark_install_common_fake_api "$selected"
  fi
}

record_common_phase() {
  local revision=$1 measurement_phase=$2 interval=$3 started=$4 finished=$5 status=$6 detail=$7
  is_decimal "$started" && is_decimal "$finished" && [ "$finished" -ge "$started" ] ||
    die "common-C phase timing is malformed: $interval"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$interval" "$started" "$finished" \
    "$((finished - started))" "$status" "$detail" >>"$common_phase_report"
}

common_image_prefix() {
  local revision=$1 measurement_phase=$2 prefix
  prefix="lagrange-cb-${benchmark_nonce}-${revision}-${measurement_phase}"
  safe_benchmark_repository "$prefix" || die 'generated common-C benchmark image prefix is not Docker-safe'
  printf '%s' "$prefix"
}

write_common_image_override() {
  local destination=$1 prefix=$2 commit=$3
  [ ! -e "$destination" ] && [ ! -L "$destination" ] ||
    die 'common-C image override already exists; refusing to overwrite it'
  [ -d "${destination%/*}" ] && [ ! -L "${destination%/*}" ] ||
    die 'common-C image override parent is not a private directory'
  RBL_BENCH_OVERRIDE=$destination RBL_BENCH_PREFIX=$prefix RBL_BENCH_COMMIT=$commit \
    RBL_BENCH_SERVICES="${services[*]}" python3 - <<'PY'
import json
import os
import re

path = os.environ["RBL_BENCH_OVERRIDE"]
prefix = os.environ["RBL_BENCH_PREFIX"]
commit = os.environ["RBL_BENCH_COMMIT"]
services = os.environ["RBL_BENCH_SERVICES"].split()
if not re.fullmatch(r"[0-9a-f]{40}", commit) or len(services) != 12 or len(set(services)) != 12:
    raise SystemExit("benchmark-common-image-override-input-invalid")
if not re.fullmatch(r"[a-z0-9]+(?:[._-][a-z0-9]+)*", prefix):
    raise SystemExit("benchmark-common-image-prefix-invalid")
value = {"services": {name: {"image": f"{prefix}-{name}:{commit}"} for name in services}}
for name, record in value["services"].items():
    if set(record) != {"image"} or not re.fullmatch(r"[a-z0-9]+(?:[._-][a-z0-9]+)*:[0-9a-f]{40}", record["image"]):
        raise SystemExit("benchmark-common-image-override-shape-invalid")
data = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
PY
}

write_benchmark_manifest_library() {
  local source=$1 destination=$2 patch=$3 prefix=$4
  [ -f "$source" ] && [ ! -L "$source" ] || die 'benchmark manifest library is not a regular file'
  [ ! -e "$destination" ] && [ ! -L "$destination" ] && [ ! -e "$patch" ] && [ ! -L "$patch" ] ||
    die 'benchmark local manifest evidence already exists; refusing to overwrite it'
  RBL_BENCH_MANIFEST_SOURCE=$source RBL_BENCH_MANIFEST_DESTINATION=$destination \
    RBL_BENCH_MANIFEST_PATCH=$patch RBL_BENCH_PREFIX=$prefix python3 - <<'PY'
import os
import re

source = os.environ["RBL_BENCH_MANIFEST_SOURCE"]
destination = os.environ["RBL_BENCH_MANIFEST_DESTINATION"]
patch = os.environ["RBL_BENCH_MANIFEST_PATCH"]
prefix = os.environ["RBL_BENCH_PREFIX"]
if not re.fullmatch(r"[a-z0-9]+(?:[._-][a-z0-9]+)*", prefix):
    raise SystemExit("benchmark-manifest-prefix-invalid")
raw = open(source, "rb").read()
old = b"printf 'lagrange-station-%s:%s' \"$service\" \"$commit\""
new = ("printf '" + prefix + "-%s:%s' \"$service\" \"$commit\"").encode("ascii")
if raw.count(old) != 1 or old == new:
    raise SystemExit("benchmark-manifest-library-literal-not-unique")
updated = raw.replace(old, new, 1)
for path, data in ((destination, updated),
                   (patch, b"literal-from\\tlagrange-station-\\n" +
                    b"literal-to\\t" + (prefix + "-").encode("ascii") + b"\\n")):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
PY
}

common_compose_build() {
  local checkout=$1 commit=$2 image_override=$3 artifact_override=$4 service=$5 compose_env=$6 compose_env_hash
  local -a args=(compose --env-file "$compose_env" --file "$checkout/deploy/compose/compose.yml" --file "$image_override")
  [ "$compose_env" = "$benchmark_compose_env_file" ] &&
    [ "$benchmark_compose_env_sha256" = "$benchmark_compose_env_sha256_expected" ] &&
    [ -f "$compose_env" ] && [ ! -L "$compose_env" ] &&
    [ "$(stat -c %a -- "$compose_env")" = 600 ] || return 1
  case "$compose_env" in "$checkout"|"$checkout"/*) return 1 ;; esac
  compose_env_hash=$(sha256sum -- "$compose_env" 2>/dev/null | awk '{print $1}') || return 1
  [ "$compose_env_hash" = "$benchmark_compose_env_sha256_expected" ] || return 1
  [ -f "$checkout/deploy/compose/compose.yml" ] && [ ! -L "$checkout/deploy/compose/compose.yml" ] ||
    return 1
  [ -f "$image_override" ] && [ ! -L "$image_override" ] || return 1
  if [ -n "$artifact_override" ]; then
    [ -f "$artifact_override" ] && [ ! -L "$artifact_override" ] || return 1
    args+=(--file "$artifact_override")
  fi
  COMPOSE_PARALLEL_LIMIT=1 \
  LAGRANGE_CODE_COMMIT="$commit" \
  RESEARCH_APP_ENV=prebuild-disabled \
  RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled \
  BACKTEST_MIN_FREE_BYTES=0 \
  BACKTEST_MAX_QUEUED_BACKTESTS=0 \
  BACKTEST_RECONCILE_GRACE_SECS=0 \
  BACKTEST_RECONCILE_INTERVAL_SECS=0 \
  RANGE_RAW_BATCH_ID=compose-config-disabled \
  COMPOSE_PROFILES= \
  LIVE_NODE_MODE=disabled \
  LIVE_NODE_DRY_RUN=1 \
    docker "${args[@]}" build --pull=false "$service"
}

common_gate_environment() {
  RELEASE_BUILD_SYSTEMD_UNIT=${BENCHMARK_SYSTEMD_SERVICE:-}
  RELEASE_BUILD_SYSTEMD_MANAGER=${BENCHMARK_SYSTEMD_MANAGER:-system}
  RELEASE_BUILD_HEALTH_UNITS=${BENCHMARK_PRODUCTION_HEALTH_UNITS:-}
  RELEASE_BUILD_HEALTH_CONTAINERS=${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-}
  RELEASE_BUILD_RESEARCH_EXCEPTION=${BENCHMARK_RESEARCH_EXCEPTION:-}
  case "$RELEASE_BUILD_SYSTEMD_MANAGER" in system|user) ;; *) return 1 ;; esac
  export RELEASE_BUILD_SYSTEMD_UNIT RELEASE_BUILD_SYSTEMD_MANAGER RELEASE_BUILD_HEALTH_UNITS
  export RELEASE_BUILD_HEALTH_CONTAINERS RELEASE_BUILD_RESEARCH_EXCEPTION
}

initialize_benchmark_gate_guard() {
  local current_commit
  common_gate_environment || return 1
  if [ "$internal_self_test" -eq 1 ] && declare -F benchmark_test_shared_init >/dev/null; then
    benchmark_test_shared_init
    return
  fi
  benchmark_gate_source_root=$repo_root
  benchmark_gate_helper=$benchmark_gate_source_root/scripts/ops/lib/release-build-layout.sh
  [ -f "$benchmark_gate_helper" ] && [ ! -L "$benchmark_gate_helper" ] || return 1
  current_commit=$(git -c "safe.directory=$benchmark_gate_source_root" -C "$benchmark_gate_source_root" \
    rev-parse --verify 'HEAD^{commit}') || return 1
  is_exact_commit "$current_commit" || return 1
  [ -z "$(git -c "safe.directory=$benchmark_gate_source_root" -C "$benchmark_gate_source_root" \
    status --porcelain=v1 --untracked-files=all)" ] || return 1
  benchmark_gate_source_commit=$current_commit
  benchmark_gate_state_root=$output_dir/benchmark-gate-state
  benchmark_gate_namespace=lagrange-benchmark-gate-${current_commit:0:12}
  benchmark_gate_lock_prefix=${RBL_LOCK_PREFIX:-/tmp/lagrange-production-image-build}
  [ ! -e "$benchmark_gate_state_root" ] && [ ! -L "$benchmark_gate_state_root" ] || return 1

  # The parent holds the whole-run descriptor for every A/B/C route.  The
  # isolated initializer below must inherit that descriptor; the public helper
  # verifies its canonical identity before creating benchmark-gate-state.
  # shellcheck disable=SC1090
  source "$benchmark_gate_helper"
  RBL_LOCK_PREFIX=$benchmark_gate_lock_prefix
  release_build_layout_lock || return 1
  (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT RELEASE_BUILD_LAYOUT_COMMIT
    unset RELEASE_BUILD_LAYOUT_STATE_ROOT RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE
    unset RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RELEASE_BUILD_LAYOUT_HELPER_SHA256
    unset RELEASE_BUILD_LAYOUT_CONFIG_SHA256 RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    common_gate_environment || exit 1
    # shellcheck disable=SC1090
    source "$benchmark_gate_helper"
    RBL_LOCK_PREFIX=$benchmark_gate_lock_prefix
    release_build_layout_init "$benchmark_gate_source_root" "$benchmark_gate_source_commit" \
      "$benchmark_gate_state_root" "$benchmark_gate_namespace"
  ) >/dev/null 2>&1 || return 1
  benchmark_gate_initialized=1
}

run_benchmark_layout_gate() {
  local label=$1 previous=$2
  [ "$benchmark_gate_initialized" -eq 1 ] || {
    gate_reason=benchmark-layout-gate-not-initialized
    gate_build_service_state=fail
    gate_health_state=fail
    return 1
  }
  case "$previous" in
    ''|*[!0-9]*)
      gate_reason=shared-release-layout-gate-failed
      gate_build_service_state=fail
      gate_health_state=fail
      return 1
      ;;
  esac
  if [ "$internal_self_test" -eq 1 ] && declare -F benchmark_test_shared_gate >/dev/null; then
    if benchmark_test_shared_gate "$label" "$previous"; then
      gate_build_service_state=verified
      gate_health_state=verified
      return 0
    fi
    gate_build_service_state=fail
    gate_health_state=fail
    [ -n "$gate_reason" ] || gate_reason=shared-release-layout-gate-failed
    return 1
  fi
  (
    unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT RELEASE_BUILD_LAYOUT_COMMIT
    unset RELEASE_BUILD_LAYOUT_STATE_ROOT RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE
    unset RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RELEASE_BUILD_LAYOUT_HELPER_SHA256
    unset RELEASE_BUILD_LAYOUT_CONFIG_SHA256 RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
    # Mapping is deliberately repeated in every isolated helper shell.  A
    # caller-controlled RELEASE_BUILD_* value must never replace the mapped
    # BENCHMARK_* contract, and the public helper owns the exact health and
    # research-exception validation.
    common_gate_environment || exit 1
    # shellcheck disable=SC1090
    source "$benchmark_gate_helper"
    RBL_LOCK_PREFIX=$benchmark_gate_lock_prefix
    release_build_layout_init "$benchmark_gate_source_root" "$benchmark_gate_source_commit" \
      "$benchmark_gate_state_root" "$benchmark_gate_namespace" || exit 1
    release_build_layout_gate "$label" "$previous"
  ) >/dev/null 2>&1 || {
    gate_build_service_state=fail
    gate_health_state=fail
    gate_reason=shared-release-layout-gate-failed
    return 1
  }
  gate_build_service_state=verified
  gate_health_state=verified
}

prepare_checkout() {
  local commit=$1 checkout=$2
  # Keep this exact local clone form: it prevents source-tree dirt from being
  # copied while retaining only local Git object/snapshot operations.
  git clone --quiet --no-local "$repo_root" "$checkout" ||
    die 'could not create a temporary local Git checkout for the benchmark'
  git -C "$checkout" checkout --quiet --detach "$commit" ||
    die 'could not detach a temporary checkout at the requested revision'
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] ||
    die 'temporary checkout HEAD did not exactly match the requested revision'
  [ -z "$(git -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] ||
    die 'temporary checkout is not clean before benchmark instrumentation'
}

create_scenario_commit() {
  local checkout=$1 revision=$2 input_commit=$3 pair_index=$4 path tree commit probe_tmp line
  is_decimal "$pair_index" && [ "$pair_index" -ge 1 ] && [ "$pair_index" -le "$repetitions" ] ||
    die "$revision scenario pair index is invalid"
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$input_commit" ] ||
    die "$revision temporary checkout changed before scenario instrumentation"
  path=${scenario_input_path[$scenario]}
  if [ "$path" != - ]; then
    case "${scenario_transform[$scenario]}" in
      append-rust-comment|append-toml-comment|append-tsx-comment|append-python-comment)
        line=$(scenario_probe_line "$pair_index")
        printf '\n%s\n' "$line" >>"$checkout/$path"
        grep -Fqx "$line" "$checkout/$path" ||
          die "$revision scenario probe was not written: $path"
        ;;
      prefix-json-whitespace)
        probe_tmp=$(mktemp "$checkout/.benchmark-config.XXXXXX") ||
          die 'could not create temporary configuration probe file'
        {
          printf '\n'
          cat -- "$checkout/$path"
        } >"$probe_tmp"
        chmod --reference="$checkout/$path" "$probe_tmp"
        mv -- "$probe_tmp" "$checkout/$path"
        [ "$(head -c 2 "$checkout/$path")" = $'\n{' ] ||
          die "$revision configuration probe did not preserve JSON syntax prefix: $path"
        ;;
      *) die "unsupported scenario transformation: ${scenario_transform[$scenario]}" ;;
    esac
    git -C "$checkout" add -- "$path" || die "$revision scenario input could not be staged in its disposable checkout"
  fi
  tree=$(git -C "$checkout" write-tree) || die "$revision scenario tree could not be written"
  commit=$(GIT_AUTHOR_NAME=build-cache-benchmark \
    GIT_AUTHOR_EMAIL=build-cache-benchmark@example.invalid \
    GIT_AUTHOR_DATE="2000-01-01T00:00:0${pair_index}Z" \
    GIT_COMMITTER_NAME=build-cache-benchmark \
    GIT_COMMITTER_EMAIL=build-cache-benchmark@example.invalid \
    GIT_COMMITTER_DATE="2000-01-01T00:00:0${pair_index}Z" \
    git -C "$checkout" commit-tree "$tree" -p "$input_commit" \
      -m "build-cache benchmark $scenario pair-$pair_index") ||
    die "$revision scenario commit could not be created"
  is_exact_commit "$commit" || die "$revision scenario commit is not an exact Git SHA"
  git -C "$checkout" checkout --quiet --detach --force "$commit" ||
    die "$revision scenario commit could not be checked out"
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] ||
    die "$revision scenario checkout does not match its recorded commit"
  [ -z "$(git -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] ||
    die "$revision scenario checkout is not clean after instrumentation"
  scenario_commit_by_revision[$revision]=$commit
  scenario_commit_by_revision_pair["$revision:$pair_index"]=$commit
}

apply_equivalent_scenario() {
  local pair_index=$1 checkout revision input_commit parent_commit
  is_decimal "$pair_index" && [ "$pair_index" -ge 1 ] && [ "$pair_index" -le "$repetitions" ] ||
    die 'scenario pair index is invalid'
  # This specification is byte-for-byte common.  The two revision-relative
  # patches and synthetic commits can legitimately hash differently because
  # their surrounding source differs, so both exact patches and the common
  # transformation spec are retained.
  for checkout in "$baseline_checkout" "$candidate_checkout"; do
    case "$checkout" in
      "$baseline_checkout") revision=baseline; input_commit=$baseline_commit ;;
      *) revision=candidate; input_commit=$candidate_commit ;;
    esac
    if [ "$pair_index" -eq 1 ]; then
      parent_commit=$input_commit
    else
      parent_commit=${scenario_commit_by_revision_pair["$revision:$((pair_index - 1))"]:-}
      [ -n "$parent_commit" ] || die "$revision prior pair commit is unavailable"
    fi
    create_scenario_commit "$checkout" "$revision" "$parent_commit" "$pair_index"
    printf 'BENCHMARK_PROBE_APPLIED revision=%s scenario=%s pair=%s source=%s commit=%s\n' \
      "$revision" "$scenario" "$pair_index" "${scenario_input_path[$scenario]}" \
      "${scenario_commit_by_revision_pair[$revision:$pair_index]}"
  done
}

record_source_snapshot() {
  local revision=$1 scenario=$2 checkout=$3 input_commit=$4 raw_patch evidence_patch
  local head tree patch_hash identity
  raw_patch=$tmp_dir/${revision}-${scenario}.source.patch.raw
  evidence_patch=$evidence_dir/source-${revision}-${scenario}.patch
  head=$(git -C "$checkout" rev-parse HEAD) || die 'could not identify temporary checkout HEAD'
  if ! git -C "$checkout" diff --binary --no-ext-diff --unified=0 "$input_commit" "$head" -- . >"$raw_patch"; then
    die "could not capture $revision/$scenario source patch"
  fi
  sanitize_text_file "$raw_patch" "$evidence_patch"
  patch_hash=$(sha256_file "$raw_patch")
  tree=$(git -C "$checkout" rev-parse 'HEAD^{tree}') || die 'could not identify temporary checkout tree'
  identity=$(sha256_text "${input_commit}\n${head}\n${tree}\n${patch_hash}\n${probe_spec_hash:-none}\n${instrumentation_manifest_hash:-none}")
  source_identity_by_revision_scenario["$revision:$scenario"]=$identity
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario" "$input_commit" "$head" "$tree" "$patch_hash" \
    "${instrumentation_manifest_hash:-none}" "$identity" >>"$source_identity_report"
}

inject_stable_cold_nonce() {
  local dockerfile=$1
  python3 - "$dockerfile" <<'PY'
import os
import re
import sys
import tempfile

path = sys.argv[1]
raw = open(path, "r", encoding="utf-8", newline="").read()
if "LAGRANGE_BENCHMARK_COLD_NONCE" in raw:
    raise SystemExit("benchmark-cold-nonce-already-present")
lines = raw.splitlines(keepends=True)
seen_rust_from = False
insert_at = None
index = 0
while index < len(lines):
    line = lines[index]
    from_match = re.match(r"^[ \t]*FROM[ \t]+([^ \t]+)", line, re.IGNORECASE)
    if from_match:
        seen_rust_from = from_match.group(1).lower().startswith("rust:")
    if seen_rust_from and re.match(r"^[ \t]*RUN(?:[ \t]|$)", line, re.IGNORECASE):
        start = index
        block = line
        while block.rstrip("\r\n").endswith("\\"):
            index += 1
            if index >= len(lines):
                raise SystemExit("benchmark-cold-nonce-unterminated-run")
            block += lines[index]
        if re.search(r"(?:^|[ \t;&|])apk[ \t]+add(?:[ \t]|$)", block):
            insert_at = start
            break
    index += 1
if insert_at is None:
    raise SystemExit("benchmark-cold-nonce-native-apk-run-missing")
snippet = (
    "ARG LAGRANGE_BENCHMARK_COLD_NONCE\n"
    "RUN test -n \"$LAGRANGE_BENCHMARK_COLD_NONCE\"\n"
)
updated = "".join(lines[:insert_at]) + snippet + "".join(lines[insert_at:])
directory = os.path.dirname(path)
fd, temporary = tempfile.mkstemp(prefix=".benchmark-cold-nonce-", dir=directory)
try:
    with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
        handle.write(updated)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
finally:
    if os.path.exists(temporary):
        os.unlink(temporary)
PY
}

inject_cargo_json_diagnostics() {
  local dockerfile=$1
  python3 - "$dockerfile" <<'PY'
import os
import re
import sys
import tempfile

path = sys.argv[1]
raw = open(path, "r", encoding="utf-8", newline="").read()
lines = raw.splitlines(keepends=True)
changed = 0
seen = 0
index = 0
while index < len(lines):
    line = lines[index]
    if not re.match(r"^[ \t]*RUN(?:[ \t]|$)", line, re.IGNORECASE):
        index += 1
        continue
    start = index
    block = line
    while block.rstrip("\r\n").endswith("\\"):
        index += 1
        if index >= len(lines):
            raise SystemExit("benchmark-cargo-json-unterminated-run")
        block += lines[index]

    pattern = re.compile(r"\bcargo\s+(?:build|install)(?=\s)")
    def replace(match):
        global changed, seen
        seen += 1
        tail = block[match.end():]
        boundary = re.search(r"(?:&&|;|\n)", tail)
        command_tail = tail[:boundary.start()] if boundary else tail
        if re.search(r"--message-format(?:=|\s+)json-render-diagnostics(?:\s|$)", command_tail):
            return match.group(0)
        changed += 1
        return match.group(0) + " --message-format=json-render-diagnostics"

    updated = pattern.sub(replace, block)
    if updated != block:
        replacement = updated.splitlines(keepends=True)
        lines[start:index + 1] = replacement
        index = start + len(replacement) - 1
    index += 1
if seen == 0:
    raise SystemExit("benchmark-cargo-json-command-missing")
if changed == 0:
    raise SystemExit(0)
updated = "".join(lines)
directory = os.path.dirname(path)
fd, temporary = tempfile.mkstemp(prefix=".benchmark-cargo-json-", dir=directory)
try:
    with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
        handle.write(updated)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
finally:
    if os.path.exists(temporary):
        os.unlink(temporary)
PY
}

inject_native_identity_marker() {
  local dockerfile=$1
  python3 - "$dockerfile" <<'PY'
import os
import re
import sys
import tempfile

path = sys.argv[1]
raw = open(path, "r", encoding="utf-8", newline="").read()
if "LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN" in raw:
    raise SystemExit("benchmark-native-identity-already-present")
lines = raw.splitlines(keepends=True)
seen_rust_from = False
insert_at = None
index = 0
while index < len(lines):
    line = lines[index]
    from_match = re.match(r"^[ \t]*FROM[ \t]+([^ \t]+)", line, re.IGNORECASE)
    if from_match:
        seen_rust_from = from_match.group(1).lower().startswith("rust:")
    if seen_rust_from and re.match(r"^[ \t]*RUN(?:[ \t]|$)", line, re.IGNORECASE):
        block = line
        while block.rstrip("\r\n").endswith("\\"):
            index += 1
            if index >= len(lines):
                raise SystemExit("benchmark-native-identity-unterminated-run")
            block += lines[index]
        if re.search(r"(?:^|[ \t;&|])apk[ \t]+add(?:[ \t]|$)", block):
            insert_at = index + 1
            break
    index += 1
if insert_at is None:
    raise SystemExit("benchmark-native-identity-apk-run-missing")
snippet = (
    "RUN set -eu; \\\n"
    "    printf '%s\\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN; \\\n"
    "    printf '%s\\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\\n' LAGRANGE_BENCH_NATIVE_RUSTC_END; \\\n"
    "    printf '%s\\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\\n' LAGRANGE_BENCH_NATIVE_CARGO_END; \\\n"
    "    printf '%s\\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\\n' LAGRANGE_BENCH_NATIVE_APK_END; \\\n"
    "    printf '%s\\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END\n"
)
updated = "".join(lines[:insert_at]) + snippet + "".join(lines[insert_at:])
directory = os.path.dirname(path)
fd, temporary = tempfile.mkstemp(prefix=".benchmark-native-identity-", dir=directory)
try:
    with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
        handle.write(updated)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
finally:
    if os.path.exists(temporary):
        os.unlink(temporary)
PY
}

instrument_dockerfile() {
  local revision=$1 checkout=$2 service=$3 index=$4 cargo_mode=${service_cargo_mode[$service]}
  local source target raw_patch evidence_patch transform diff_status base toolchain_file toolchain_hash
  source=$checkout/${service_dockerfile[$service]}
  target=$instrumentation_dir/${revision}-${index}-${service}.Dockerfile
  raw_patch=$tmp_dir/${revision}-${index}-${service}.instrumentation.patch.raw
  evidence_patch=$evidence_dir/instrumentation-${revision}-${index}-${service}.patch
  mkdir -p -- "${target%/*}"
  cp -- "$source" "$target"
  transform=none-no-cargo-vertex
  if [ "$cargo_mode" -gt 0 ]; then
    if grep -Eq 'cargo[[:space:]]+(build|install)[[:space:]]+(-v|--verbose)' "$source"; then
      transform=already-verbose-no-command-change
    else
      # The only edit is a Cargo verbosity flag in a temporary copy.  It does
      # not change targets, package selection, binary copies, or source files.
      sed -E -i 's/(cargo[[:space:]]+(build|install))([[:space:]])/\1 -vv\3/g' "$target"
      transform=add-vv-to-cargo-build-and-install
    fi
    cmp -s -- "$source" "$target" &&
      [ "$transform" != already-verbose-no-command-change ] &&
      die "temporary Cargo instrumentation did not change: ${service_dockerfile[$service]}"
    inject_cargo_json_diagnostics "$target" ||
      die "could not add Cargo JSON diagnostics: ${service_dockerfile[$service]}"
    transform="${transform}+cargo-json-render-diagnostics"
    # A/B recipes receive one fixed, side-specific build argument before the
    # native apk layer. It forces a cold layer miss once without `--no-cache`,
    # then remains identical for that side's warm-up and all measured pairs.
    inject_stable_cold_nonce "$target" ||
      die "could not add stable cold cache nonce before native setup: ${service_dockerfile[$service]}"
    inject_native_identity_marker "$target" ||
      die "could not add native tool identity marker: ${service_dockerfile[$service]}"
    transform="${transform}+stable-cold-nonce-before-native-apk+native-tool-identity"
  fi
  set +e
  diff -u -U0 -- "$source" "$target" >"$raw_patch"
  diff_status=$?
  set -e
  case "$diff_status" in
    0|1) ;;
    *) die "could not diff temporary Dockerfile instrumentation: $service" ;;
  esac
  sanitize_text_file "$raw_patch" "$evidence_patch"
  base=$(awk 'toupper($1) == "FROM" { print $2; exit }' "$target")
  [ -n "$base" ] || base=unavailable
  toolchain_file=$checkout/rust-toolchain.toml
  if [ -f "$toolchain_file" ] && [ ! -L "$toolchain_file" ]; then
    toolchain_hash=$(sha256_file "$toolchain_file")
  else
    toolchain_hash=unavailable
  fi
  source_dockerfile_hash["$revision:$service"]=$(sha256_file "$source")
  instrumented_dockerfile_hash["$revision:$service"]=$(sha256_file "$target")
  instrumentation_patch_hash["$revision:$service"]=$(sha256_file "$raw_patch")
  instrumentation_transform["$revision:$service"]=$transform
  instrumented_dockerfile["$revision:$service"]=$target
  service_toolchain_identity["$revision:$service"]=$(safe_scalar "base=${base}|rust-toolchain=${toolchain_hash}")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$service" "${service_dockerfile[$service]}" "$transform" \
    "${source_dockerfile_hash[$revision:$service]}" \
    "${instrumented_dockerfile_hash[$revision:$service]}" \
    "${instrumentation_patch_hash[$revision:$service]}" \
    "${service_toolchain_identity[$revision:$service]}" >>"$instrumentation_report"
}

instrument_all_dockerfiles() {
  local revision=$1 checkout=$2 service index=0
  for service in "${services[@]}"; do
    index=$((index + 1))
    instrument_dockerfile "$revision" "$checkout" "$service" "$index"
  done
}

derive_cargo_cache_state() {
  local cargo_mode=$1
  if [ "$cargo_mode" -eq 0 ]; then
    printf '%s' not-applicable
  elif [ "$BENCH_COMPILER_VERTEX_STATE" = cached ]; then
    printf '%s' compiler-layer-cached
  elif [ "$BENCH_FRESH_COUNT" -gt 0 ] && [ "$BENCH_COMPILED_COUNT" -gt 0 ]; then
    printf '%s' mixed-compiled-and-fresh
  elif [ "$BENCH_FRESH_COUNT" -gt 0 ]; then
    printf '%s' fresh-packages-observed
  elif [ "$BENCH_COMPILED_COUNT" -gt 0 ]; then
    printf '%s' compiled-packages-observed
  else
    printf '%s' executed-without-package-events
  fi
}

append_result() {
  local revision=$1 measurement_phase=$2 service=$3 index=$4 tag=$5 namespace=$6 commit=$7 source_identity=$8
  local total_ms=$9 cargo_ms=${10} verify_ms=${11} image_id=${12} image_revision=${13}
  local cargo_cache=${14} evidence_rel=${15} selected_scenario=${16}
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$index" "$tag" "$namespace" "$commit" "$source_identity" \
    "$total_ms" "$cargo_ms" "$verify_ms" "$image_id" "$image_revision" \
    "$BENCH_COMPILER_VERTEX" "$BENCH_COMPILER_VERTEX_STATE" \
    "$BENCH_COMPILER_CACHE" "$BENCH_NONCOMPILER_CACHED" "$cargo_cache" \
    "$(csv_or_dash "$BENCH_COMPILED_PACKAGES")" "$(csv_or_dash "$BENCH_FRESH_PACKAGES")" \
    "$BENCH_WORKSPACE_CLEAN" "${service_toolchain_identity[$revision:$service]}" \
    "${source_dockerfile_hash[$revision:$service]}" "${instrumented_dockerfile_hash[$revision:$service]}" \
    "${instrumentation_patch_hash[$revision:$service]}" "$evidence_rel" "$selected_scenario" >>"$results_report"
}

now_ms() {
  local raw seconds nanoseconds
  # Some date implementations ignore the precision width in %3N and emit all
  # nanoseconds, so do not treat `%s%3N` as reliably millisecond-shaped.
  raw=$(date '+%s:%N')
  seconds=${raw%%:*}
  nanoseconds=${raw#*:}
  is_decimal "$seconds" && printf '%s' "$nanoseconds" | grep -Eq '^[0-9]{9}$' ||
    die 'nanosecond clock is unavailable for benchmark timing'
  printf '%s' "$((10#$seconds * 1000 + 10#${nanoseconds:0:3}))"
}

df_available_kib() {
  local path=$1 available
  available=$(df -Pk "$path" 2>/dev/null | awk 'NR == 2 { print $4; exit }') || return 1
  is_decimal "$available" || return 1
  printf '%s' "$available"
}

resource_sample_values() {
  local meminfo mem_available swap_free repository_disk docker_disk
  meminfo=$(read_meminfo) || return 1
  mem_available=$(awk '/^MemAvailable:/ { print $2; exit }' <<<"$meminfo")
  swap_free=$(awk '/^SwapFree:/ { print $2; exit }' <<<"$meminfo")
  repository_disk=$(df_available_kib "$repo_root") || return 1
  docker_disk=$(df_available_kib "$docker_root_dir") || return 1
  is_decimal "$mem_available" && is_decimal "$swap_free" || return 1
  printf '%s\t%s\t%s\t%s' "$mem_available" "$swap_free" "$repository_disk" "$docker_disk"
}

record_resource_sample() {
  local destination=$1 revision=$2 scenario_name=$3 service=$4 point=$5 values timestamp
  values=$(resource_sample_values) || return 1
  timestamp=$(now_ms) || return 1
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario_name" "$service" "$point" "$timestamp" "$values" >>"$destination"
}

start_resource_sampler() {
  local revision=$1 scenario_name=$2 service=$3
  [ -n "$tmp_dir" ] || return 1
  sampler_file=$tmp_dir/resource-${revision}-${scenario_name}-${service}.tsv
  sampler_failure_file=$tmp_dir/resource-${revision}-${scenario_name}-${service}.failure
  : >"$sampler_file"
  : >"$sampler_failure_file"
  chmod 0600 -- "$sampler_file" "$sampler_failure_file"
  (
    # Do not terminate this shell while a command substitution inside a sample
    # still owns its stdout pipe.  That previously produced spurious Broken
    # pipe diagnostics from `now_ms`/resource_sample_values and could hide a
    # real sampling failure in otherwise successful C evidence.  USR1 requests
    # a graceful stop at a command boundary; it is not propagated to children.
    local sampler_stop=0
    trap 'sampler_stop=1' USR1
    while [ "$sampler_stop" -eq 0 ]; do
      if ! record_resource_sample "$sampler_file" "$revision" "$scenario_name" "$service" sample; then
        printf '%s\n' 'resource-sample-unavailable' >"$sampler_failure_file"
        exit 0
      fi
      [ "$sampler_stop" -eq 0 ] || exit 0
      sleep 1
    done
  ) &
  sampler_pid=$!
}

stop_resource_sampler() {
  local status=0
  [ -n "${sampler_pid:-}" ] || return 0
  # Ask the sampler to finish its current bounded sample rather than killing
  # it mid-command-substitution.  A nonzero wait remains a failed sample; it
  # is not treated as a successful cancellation.
  kill -USR1 "$sampler_pid" >/dev/null 2>&1 || true
  wait "$sampler_pid" >/dev/null 2>&1 || status=1
  [ -s "${sampler_file:-}" ] && cat -- "$sampler_file" >>"$resource_samples_report"
  [ ! -s "${sampler_failure_file:-}" ] || status=1
  sampler_pid=
  sampler_file=
  sampler_failure_file=
  return "$status"
}

write_peak_resource_report() {
  if ! awk -F '\t' '
    NR == 1 { next }
    {
      for (field = 6; field <= 9; field++) if ($field !~ /^[0-9]+$/) bad = 1
      count += 1
      if (count == 1 || $6 < min_mem) min_mem = $6
      if (count == 1 || $7 < min_swap) min_swap = $7
      if (count == 1 || $8 < min_repo) min_repo = $8
      if (count == 1 || $9 < min_docker) min_docker = $9
    }
    END {
      if (count == 0 || bad) exit 1
      printf "benchmark-build-samples\t%d\t%s\t%s\t%s\t%s\tminimum available capacity sampled during Docker builds; not process RSS or exact used-memory peak\n", count, min_mem, min_swap, min_repo, min_docker
    }
  ' "$resource_samples_report" >>"$peak_resource_report"; then
    die 'resource peak report is incomplete or malformed'
  fi
  chmod 0600 -- "$peak_resource_report"
}

record_phase() {
  local phase=$1 revision=$2 scenario_name=$3 started=$4 finished=$5 status=$6 detail=$7 elapsed
  is_decimal "$started" && is_decimal "$finished" && [ "$finished" -ge "$started" ] ||
    die "benchmark phase timing is malformed: $phase"
  elapsed=$((finished - started))
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" "$revision" "$scenario_name" "$started" "$finished" "$elapsed" "$status" "$detail" >>"$phase_report"
}

inspect_benchmark_image() {
  local tag=$1 commit=$2 inspected image_id image_revision image_size extra start_ms end_ms verify_ms
  start_ms=$(now_ms)
  if ! inspected=$(docker image inspect --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}|{{.Size}}' -- "$tag" 2>/dev/null); then
    return 1
  fi
  end_ms=$(now_ms)
  verify_ms=$((end_ms - start_ms))
  case "$inspected" in *$'\n'*|*$'\r'*|*[$'\001'-$'\037'$'\177']*) return 1 ;; esac
  image_id= image_revision= image_size= extra=
  IFS='|' read -r image_id image_revision image_size extra <<<"$inspected"
  [ -z "$extra" ] || return 1
  [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || return 1
  [ "$image_revision" = "$commit" ] || return 1
  is_decimal "$image_size" || return 1
  is_decimal "$verify_ms" || return 1
  printf '%s\t%s\t%s\t%s' "$verify_ms" "$image_id" "$image_revision" "$image_size"
}

docker_disk_snapshot() {
  local raw
  raw=$(docker system df --format '{{json .}}' 2>/dev/null) || return 1
  [ "${#raw}" -le 65536 ] || return 1
  RBL_BENCH_DOCKER_DF=$raw python3 - <<'PY'
import decimal
import json
import os
import re

raw=os.environ["RBL_BENCH_DOCKER_DF"]
if not raw: raise SystemExit(1)
units={"B":1,"kB":1000,"KB":1000,"MB":1000**2,"GB":1000**3,"TB":1000**4}
values={}
for line in raw.splitlines():
    if not line or len(line)>8192: raise SystemExit(1)
    item=json.loads(line)
    if not isinstance(item,dict) or set(item)!={"Type","TotalCount","Active","Size","Reclaimable"}: raise SystemExit(1)
    label=item["Type"]
    # Docker reports Containers and Local Volumes as well.  They are outside
    # this benchmark's image/cache cost scope, but their presence must not
    # make a real daemon's otherwise documented `system df` stream unusable.
    if label not in ("Images","Build Cache"):
        continue
    if label in values: raise SystemExit(1)
    match=re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)(B|kB|KB|MB|GB|TB)",item["Size"])
    if not match: raise SystemExit(1)
    value=int(decimal.Decimal(match.group(1))*units[match.group(2)])
    if value<0: raise SystemExit(1)
    values[label]=value
if set(values)!={"Images","Build Cache"}: raise SystemExit(1)
print(f"{values['Images']}\t{values['Build Cache']}\t{values['Images']+values['Build Cache']}")
PY
}

record_disk_snapshot() {
  local revision=$1 measurement_phase=$2 service=$3 point=$4 image_size=$5 snapshot images cache total
  snapshot=$(docker_disk_snapshot) || return 1
  IFS=$'\t' read -r images cache total <<<"$snapshot"
  is_decimal "$images" && is_decimal "$cache" && is_decimal "$total" || return 1
  case "$image_size" in -|*[!0-9]*) [ "$image_size" = - ] || return 1 ;; esac
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$point" "$image_size" "$images" "$cache" "$total" >>"$disk_report"
}

write_disk_summary() {
  if ! awk -F '\t' '
    NR == 1 { next }
    {
      key=$1 SUBSEP $2 SUBSEP $3
      if ($4 == "before-build") {
        if (key in before) bad=1
        before[key]=$0
      }
      else if ($4 == "after-image-save") {
        if (key in after) bad=1
        after[key]=$0
      }
      else if ($4 == "before-prepare") {
        if (key in producer_before) bad=1
        producer_before[key]=$0
      }
      else if ($4 == "after-prepare") {
        if (key in producer_after) bad=1
        producer_after[key]=$0
      }
      else bad=1
      for (field=6; field<=8; field++) if ($field !~ /^[0-9]+$/) bad=1
      if ($5 != "-" && $5 !~ /^[0-9]+$/) bad=1
    }
    END {
      for (key in before) {
        if (!(key in after)) { bad=1; continue }
        split(before[key], b, "\t"); split(after[key], a, "\t")
        if (b[5] != "-" || a[5] !~ /^[0-9]+$/) { bad=1; continue }
        images_delta=a[6]-b[6]; cache_delta=a[7]-b[7]; total_delta=a[8]-b[8]
        image_peak=(a[6]>b[6]?a[6]:b[6]); cache_peak=(a[7]>b[7]?a[7]:b[7]); total_peak=(a[8]>b[8]?a[8]:b[8])
        printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", b[1],b[2],b[3],a[5],images_delta,cache_delta,total_delta,image_peak,cache_peak,total_peak
      }
      for (key in after) if (!(key in before)) bad=1
      for (key in producer_before) {
        if (!(key in producer_after)) { bad=1; continue }
        split(producer_before[key], b, "\t"); split(producer_after[key], a, "\t")
        if (b[5] != "-" || a[5] != "-") { bad=1; continue }
        images_delta=a[6]-b[6]; cache_delta=a[7]-b[7]; total_delta=a[8]-b[8]
        image_peak=(a[6]>b[6]?a[6]:b[6]); cache_peak=(a[7]>b[7]?a[7]:b[7]); total_peak=(a[8]>b[8]?a[8]:b[8])
        printf "%s\t%s\t%s\t-\t%s\t%s\t%s\t%s\t%s\t%s\n", b[1],b[2],b[3],images_delta,cache_delta,total_delta,image_peak,cache_peak,total_peak
      }
      for (key in producer_after) if (!(key in producer_before)) bad=1
      exit bad
    }
  ' "$disk_report" >>"$disk_summary_report"; then
    die 'disk cost evidence is incomplete or malformed'
  fi
  chmod 0600 -- "$disk_summary_report"
}

native_identity_recipe_binding() {
  local dockerfile=$1 namespace=$2 cold_nonce=$3 commit=$4
  RBL_BENCH_DOCKERFILE=$dockerfile \
    RBL_BENCH_NAMESPACE=$namespace \
    RBL_BENCH_COLD_NONCE=$cold_nonce \
    RBL_BENCH_CODE_COMMIT=$commit \
    RBL_BENCH_BUILDER_IDENTITY=$builder_identity \
    RBL_BENCH_BUILDX_IDENTITY=$buildx_identity \
    RBL_BENCH_DOCKER_IDENTITY=$docker_identity \
    RBL_BENCH_DOCKER_PLATFORM=$docker_platform_identity \
    python3 - <<'PY'
import hashlib
import json
import os
import re

path = os.environ["RBL_BENCH_DOCKERFILE"]
raw = open(path, "r", encoding="utf-8", newline="").read()
logical = []
buffer = ""
for physical in raw.splitlines():
    piece = physical.rstrip()
    continued = piece.endswith("\\")
    if continued:
        piece = piece[:-1]
    buffer = (buffer + " " + piece.strip()).strip()
    if continued:
        continue
    if buffer and not buffer.lstrip().startswith("#"):
        logical.append(buffer)
    buffer = ""
if buffer:
    raise SystemExit("benchmark-native-recipe-unterminated-instruction")

instructions = []
in_rust_stage = False
found = False
marker_instruction = None
for line in logical:
    match = re.match(r"^([A-Za-z]+)(?:\s+(.*))?$", line)
    if match is None:
        raise SystemExit("benchmark-native-recipe-instruction-invalid")
    keyword = match.group(1).upper()
    body = re.sub(r"\s+", " ", (match.group(2) or "").strip())
    normalized = keyword + ((" " + body) if body else "")
    if keyword == "FROM":
        image = body.split(" ", 1)[0].lower() if body else ""
        in_rust_stage = image.startswith("rust:")
        instructions = [normalized] if in_rust_stage else []
        continue
    if not in_rust_stage:
        continue
    instructions.append(normalized)
    if keyword == "RUN" and "LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN" in body:
        if found:
            raise SystemExit("benchmark-native-recipe-marker-duplicate")
        found = True
        marker_instruction = normalized
        break
if not found or marker_instruction is None:
    raise SystemExit("benchmark-native-recipe-marker-missing")

referenced = set()
for instruction in instructions:
    referenced.update(re.findall(r"\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))", instruction))
referenced = {left or right for left, right in referenced}
known_args = {
    "LAGRANGE_CODE_COMMIT": os.environ["RBL_BENCH_CODE_COMMIT"],
    "LAGRANGE_BENCHMARK_COLD_NONCE": os.environ["RBL_BENCH_COLD_NONCE"],
    "BUILDKIT_CACHE_MOUNT_NS": os.environ["RBL_BENCH_NAMESPACE"],
    "CARGO_BUILD_JOBS": "2",
}
binding = {
    "format": "lagrange-benchmark-native-recipe-v1",
    "instructions": instructions,
    "build_inputs": {
        "builder_identity": os.environ["RBL_BENCH_BUILDER_IDENTITY"],
        "buildx_identity": os.environ["RBL_BENCH_BUILDX_IDENTITY"],
        "docker_identity": os.environ["RBL_BENCH_DOCKER_IDENTITY"],
        "docker_platform_identity": os.environ["RBL_BENCH_DOCKER_PLATFORM"],
        "cache_mount_namespace": os.environ["RBL_BENCH_NAMESPACE"],
        "cold_nonce": os.environ["RBL_BENCH_COLD_NONCE"],
        "cargo_build_jobs": "2",
        "referenced_build_args": {name: value for name, value in known_args.items() if name in referenced},
    },
}
encoded = json.dumps(binding, sort_keys=True, separators=(",", ":")).encode("utf-8")
print(hashlib.sha256(encoded).hexdigest() + "\t" + hashlib.sha256(marker_instruction.encode("utf-8")).hexdigest())
PY
}

inspect_native_identity_vertex() {
  local log=$1 expected_marker_hash=$2 fields
  fields=$(RBL_BENCH_LOG=$log RBL_BENCH_MARKER_HASH=$expected_marker_hash python3 - <<'PY'
import hashlib
import os
import re

raw = open(os.environ["RBL_BENCH_LOG"], "rb").read()
if len(raw) > 256 * 1024 * 1024:
    raise SystemExit("benchmark-native-log-too-large")
try:
    lines = raw.decode("utf-8").splitlines()
except UnicodeDecodeError:
    raise SystemExit("benchmark-native-log-not-utf8")

markers = {
    "LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN",
    "LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN",
    "LAGRANGE_BENCH_NATIVE_RUSTC_END",
    "LAGRANGE_BENCH_NATIVE_CARGO_BEGIN",
    "LAGRANGE_BENCH_NATIVE_CARGO_END",
    "LAGRANGE_BENCH_NATIVE_APK_BEGIN",
    "LAGRANGE_BENCH_NATIVE_APK_END",
    "LAGRANGE_BENCH_NATIVE_IDENTITY_END",
}
headers = []
for line in lines:
    match = re.match(r"^#([0-9]+)\s+\[[^]]+\]\s+(RUN\s+.*)$", line)
    if match and "LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN" in match.group(2):
        headers.append((match.group(1), match.group(2)))
if len(headers) != 1:
    raise SystemExit("benchmark-native-identity-vertex-invalid")
vertex, command = headers[0]
normalized = re.sub(r"\s+", " ", command.strip())
marker_hash = hashlib.sha256(normalized.encode("utf-8")).hexdigest()
if marker_hash != os.environ["RBL_BENCH_MARKER_HASH"]:
    raise SystemExit("benchmark-native-identity-vertex-recipe-mismatch")

cached = sum(line == f"#{vertex} CACHED" for line in lines)
done = sum(re.match(rf"^#{re.escape(vertex)} DONE [0-9]+(?:\.[0-9]+)?s$", line) is not None for line in lines)
if any(re.match(rf"^#{re.escape(vertex)}\s+.*ERROR", line) for line in lines):
    raise SystemExit("benchmark-native-identity-vertex-error")
outputs = []
prefix = re.compile(r"^#([0-9]+)\s+(?:[0-9]+(?:\.[0-9]+)?s?\s+)?(.*)$")
for line in lines:
    match = prefix.match(line)
    if match and match.group(2) in markers:
        outputs.append((match.group(1), match.group(2)))
if any(item[0] != vertex for item in outputs):
    raise SystemExit("benchmark-native-marker-vertex-mismatch")
if cached:
    if cached != 1 or done != 0 or outputs:
        raise SystemExit("benchmark-native-cached-vertex-output-invalid")
    state = "cached"
else:
    if done != 1:
        raise SystemExit("benchmark-native-executed-vertex-done-invalid")
    state = "executed"
print("\t".join((state, vertex, hashlib.sha256(raw).hexdigest())))
PY
) || return 1
  IFS=$'\t' read -r BENCH_NATIVE_VERTEX_STATE BENCH_NATIVE_VERTEX BENCH_NATIVE_RAW_LOG_SHA256 <<<"$fields"
  case "$BENCH_NATIVE_VERTEX_STATE" in cached|executed) ;; *) return 1 ;; esac
  [[ "$BENCH_NATIVE_VERTEX" =~ ^[0-9]+$ ]] || return 1
  [[ "$BENCH_NATIVE_RAW_LOG_SHA256" =~ ^[0-9a-f]{64}$ ]] || return 1
}

parse_native_identity() {
  local log=$1 destination=$2 recipe_hash=$3 marker_hash=$4 vertex=$5
  local revision=$6 measurement_phase=$7 service=$8 index=$9
  [ -d "${destination%/*}" ] && [ ! -L "${destination%/*}" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  RBL_BENCH_LOG=$log RBL_BENCH_NATIVE=$destination \
    RBL_BENCH_RECIPE_HASH=$recipe_hash RBL_BENCH_MARKER_HASH=$marker_hash \
    RBL_BENCH_VERTEX=$vertex RBL_BENCH_REVISION=$revision \
    RBL_BENCH_PHASE=$measurement_phase RBL_BENCH_SERVICE=$service \
    RBL_BENCH_INDEX=$index python3 - <<'PY'
import hashlib
import json
import os
import re

log_path = os.environ["RBL_BENCH_LOG"]
out_path = os.environ["RBL_BENCH_NATIVE"]
raw = open(log_path, "rb").read()
if len(raw) > 256 * 1024 * 1024:
    raise SystemExit("benchmark-native-log-too-large")
try:
    lines = raw.decode("utf-8").splitlines()
except UnicodeDecodeError:
    raise SystemExit("benchmark-native-log-not-utf8")

prefix = re.compile(r"^#([0-9]+)\s+(?:[0-9]+(?:\.[0-9]+)?s?\s+)?(.*)$")
vertex = os.environ["RBL_BENCH_VERTEX"]
markers = [
    "LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN",
    "LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN",
    "LAGRANGE_BENCH_NATIVE_RUSTC_END",
    "LAGRANGE_BENCH_NATIVE_CARGO_BEGIN",
    "LAGRANGE_BENCH_NATIVE_CARGO_END",
    "LAGRANGE_BENCH_NATIVE_APK_BEGIN",
    "LAGRANGE_BENCH_NATIVE_APK_END",
    "LAGRANGE_BENCH_NATIVE_IDENTITY_END",
]
sections = {"rustc": [], "cargo": [], "apk": []}
expected = 0
active = None
for line in lines:
    match = prefix.match(line)
    if match is None:
        continue
    value = match.group(2)
    if value in markers:
        if match.group(1) != vertex:
            raise SystemExit("benchmark-native-marker-vertex-mismatch")
        if expected >= len(markers) or value != markers[expected]:
            raise SystemExit("benchmark-native-marker-order-invalid")
        expected += 1
        active = {
            "LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN": "rustc",
            "LAGRANGE_BENCH_NATIVE_CARGO_BEGIN": "cargo",
            "LAGRANGE_BENCH_NATIVE_APK_BEGIN": "apk",
        }.get(value)
        if value.endswith("_END"):
            active = None
        continue
    if active is not None:
        if len(value) > 8192 or any(ord(char) < 32 or ord(char) == 127 for char in value):
            raise SystemExit("benchmark-native-output-invalid")
        sections[active].append(value)
if expected != len(markers):
    raise SystemExit("benchmark-native-markers-missing")
if not all(sections.values()) or any(len(value) > 10000 for value in sections.values()):
    raise SystemExit("benchmark-native-sections-invalid")

rustc = "\n".join(sections["rustc"]) + "\n"
cargo = "\n".join(sections["cargo"]) + "\n"
apk = "\n".join(sections["apk"]) + "\n"
hosts = re.findall(r"^host:\s*(\S+)\s*$", rustc, re.MULTILINE)
if hosts != ["x86_64-unknown-linux-musl"]:
    raise SystemExit("benchmark-native-rustc-host-invalid")
if not rustc.startswith("rustc ") or not cargo.startswith("cargo "):
    raise SystemExit("benchmark-native-tool-version-invalid")
if len(apk.encode("utf-8")) > 4 * 1024 * 1024:
    raise SystemExit("benchmark-native-apk-output-too-large")
result = {
    "format": "lagrange-benchmark-native-identity-v1",
    "evidence_mode": "executed",
    "native_recipe_sha256": os.environ["RBL_BENCH_RECIPE_HASH"],
    "native_marker_run_sha256": os.environ["RBL_BENCH_MARKER_HASH"],
    "native_vertex": vertex,
    "native_vertex_state": "executed",
    "build": {
        "revision": os.environ["RBL_BENCH_REVISION"],
        "measurement_phase": os.environ["RBL_BENCH_PHASE"],
        "service": os.environ["RBL_BENCH_SERVICE"],
        "index": int(os.environ["RBL_BENCH_INDEX"]),
    },
    "raw_log_sha256": hashlib.sha256(raw).hexdigest(),
    "rustc_vv": rustc,
    "cargo_version": cargo,
    "apk_info_vv": apk,
    "host_triple": hosts[0],
    "rustc_vv_sha256": hashlib.sha256(rustc.encode("utf-8")).hexdigest(),
    "cargo_version_sha256": hashlib.sha256(cargo.encode("utf-8")).hexdigest(),
    "apk_info_vv_sha256": hashlib.sha256(apk.encode("utf-8")).hexdigest(),
    "apk_package_line_count": len(sections["apk"]),
}
data = (json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
PY
}

reuse_cached_native_identity() {
  local source=$1 source_hash=$2 destination=$3 current_log=$4 recipe_hash=$5 marker_hash=$6 vertex=$7
  local revision=$8 measurement_phase=$9 service=${10} index=${11}
  [ -f "$source" ] && [ ! -L "$source" ] || return 1
  [ "$(sha256_file "$source")" = "$source_hash" ] || return 1
  [ -d "${destination%/*}" ] && [ ! -L "${destination%/*}" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  RBL_BENCH_SOURCE=$source RBL_BENCH_SOURCE_HASH=$source_hash \
    RBL_BENCH_SOURCE_REL=${source#$output_dir/} RBL_BENCH_NATIVE=$destination \
    RBL_BENCH_LOG=$current_log RBL_BENCH_RECIPE_HASH=$recipe_hash \
    RBL_BENCH_MARKER_HASH=$marker_hash RBL_BENCH_VERTEX=$vertex \
    RBL_BENCH_REVISION=$revision RBL_BENCH_PHASE=$measurement_phase \
    RBL_BENCH_SERVICE=$service RBL_BENCH_INDEX=$index python3 - <<'PY'
import hashlib
import json
import os
import re

source_raw = open(os.environ["RBL_BENCH_SOURCE"], "rb").read()
if hashlib.sha256(source_raw).hexdigest() != os.environ["RBL_BENCH_SOURCE_HASH"]:
    raise SystemExit("benchmark-native-source-hash-mismatch")
source = json.loads(source_raw.decode("utf-8"))
if (source.get("format") != "lagrange-benchmark-native-identity-v1" or
        source.get("evidence_mode") != "executed" or
        source.get("native_vertex_state") != "executed" or
        source.get("native_recipe_sha256") != os.environ["RBL_BENCH_RECIPE_HASH"] or
        source.get("native_marker_run_sha256") != os.environ["RBL_BENCH_MARKER_HASH"] or
        not re.fullmatch(r"[0-9]+", source.get("native_vertex", ""))):
    raise SystemExit("benchmark-native-source-provenance-invalid")

text_fields = (
    ("rustc_vv", "rustc_vv_sha256"),
    ("cargo_version", "cargo_version_sha256"),
    ("apk_info_vv", "apk_info_vv_sha256"),
)
for value_name, hash_name in text_fields:
    value = source.get(value_name)
    digest = source.get(hash_name)
    if not isinstance(value, str) or hashlib.sha256(value.encode("utf-8")).hexdigest() != digest:
        raise SystemExit("benchmark-native-source-content-hash-invalid")
if source.get("host_triple") != "x86_64-unknown-linux-musl":
    raise SystemExit("benchmark-native-source-host-invalid")
if not isinstance(source.get("apk_package_line_count"), int) or source["apk_package_line_count"] <= 0:
    raise SystemExit("benchmark-native-source-apk-count-invalid")
if not re.fullmatch(r"[0-9a-f]{64}", source.get("raw_log_sha256", "")):
    raise SystemExit("benchmark-native-source-log-hash-invalid")
if not isinstance(source.get("build"), dict):
    raise SystemExit("benchmark-native-source-build-invalid")

current_raw = open(os.environ["RBL_BENCH_LOG"], "rb").read()
result = {key: source[key] for key in (
    "format", "rustc_vv", "cargo_version", "apk_info_vv", "host_triple",
    "rustc_vv_sha256", "cargo_version_sha256", "apk_info_vv_sha256",
    "apk_package_line_count", "raw_log_sha256",
)}
result.update({
    "evidence_mode": "cached-reuse",
    "native_recipe_sha256": os.environ["RBL_BENCH_RECIPE_HASH"],
    "native_marker_run_sha256": os.environ["RBL_BENCH_MARKER_HASH"],
    "native_vertex": os.environ["RBL_BENCH_VERTEX"],
    "native_vertex_state": "cached",
    "current_raw_log_sha256": hashlib.sha256(current_raw).hexdigest(),
    "build": {
        "revision": os.environ["RBL_BENCH_REVISION"],
        "measurement_phase": os.environ["RBL_BENCH_PHASE"],
        "service": os.environ["RBL_BENCH_SERVICE"],
        "index": int(os.environ["RBL_BENCH_INDEX"]),
    },
    "source_evidence": {
        "path": os.environ["RBL_BENCH_SOURCE_REL"],
        "sha256": os.environ["RBL_BENCH_SOURCE_HASH"],
        "raw_log_sha256": source["raw_log_sha256"],
        "native_vertex": source["native_vertex"],
        "build": source["build"],
    },
})
data = (json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(os.environ["RBL_BENCH_NATIVE"], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
PY
}

record_native_identity() {
  local revision=$1 measurement_phase=$2 service=$3 index=$4 raw_log=$5 cargo_mode=$6
  local dockerfile=$7 namespace=$8 cold_nonce=$9 commit=${10} path hash fields binding recipe_hash marker_hash source source_hash
  if [ "$cargo_mode" -eq 0 ]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$revision" "$measurement_phase" "$service" "$index" not-applicable not-applicable \
      not-applicable not-applicable not-applicable not-applicable >>"$native_identity_report"
    return 0
  fi
  path=$output_dir/native-identities/${revision}-${measurement_phase}-${index}-${service}.json
  binding=$(native_identity_recipe_binding "$dockerfile" "$namespace" "$cold_nonce" "$commit") || return 1
  IFS=$'\t' read -r recipe_hash marker_hash <<<"$binding"
  [[ "$recipe_hash" =~ ^[0-9a-f]{64}$ ]] && [[ "$marker_hash" =~ ^[0-9a-f]{64}$ ]] || return 1
  inspect_native_identity_vertex "$raw_log" "$marker_hash" || return 1
  if [ "$BENCH_NATIVE_VERTEX_STATE" = executed ]; then
    parse_native_identity "$raw_log" "$path" "$recipe_hash" "$marker_hash" "$BENCH_NATIVE_VERTEX" \
      "$revision" "$measurement_phase" "$service" "$index" || return 1
  else
    source=${native_identity_source_by_recipe[$recipe_hash]:-}
    source_hash=${native_identity_source_hash_by_recipe[$recipe_hash]:-}
    [ -n "$source" ] && [[ "$source_hash" =~ ^[0-9a-f]{64}$ ]] || return 1
    reuse_cached_native_identity "$source" "$source_hash" "$path" "$raw_log" \
      "$recipe_hash" "$marker_hash" "$BENCH_NATIVE_VERTEX" \
      "$revision" "$measurement_phase" "$service" "$index" || return 1
  fi
  hash=$(sha256_file "$path") || return 1
  fields=$(python3 - "$path" <<'PY'
import hashlib, json, re, sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
if value.get("format") != "lagrange-benchmark-native-identity-v1": raise SystemExit(1)
items=[value.get("rustc_vv_sha256"),value.get("cargo_version_sha256"),value.get("apk_info_vv_sha256"),value.get("apk_package_line_count")]
if not all(isinstance(item,str) and re.fullmatch(r"[0-9a-f]{64}",item) for item in items[:3]): raise SystemExit(1)
if not isinstance(items[3],int) or items[3] <= 0: raise SystemExit(1)
for name, digest in (("rustc_vv",items[0]),("cargo_version",items[1]),("apk_info_vv",items[2])):
    item=value.get(name)
    if not isinstance(item,str) or hashlib.sha256(item.encode()).hexdigest()!=digest: raise SystemExit(1)
print("\t".join([*items[:3],str(items[3])]))
PY
) || return 1
  if [ "$BENCH_NATIVE_VERTEX_STATE" = executed ]; then
    if [ -n "${native_identity_source_by_recipe[$recipe_hash]:-}" ]; then
      source=${native_identity_source_by_recipe[$recipe_hash]}
      python3 - "$source" "$path" <<'PY' || return 1
import json, sys
left=json.load(open(sys.argv[1],encoding="utf-8")); right=json.load(open(sys.argv[2],encoding="utf-8"))
keys=("rustc_vv_sha256","cargo_version_sha256","apk_info_vv_sha256","apk_package_line_count","host_triple")
if any(left.get(key)!=right.get(key) for key in keys): raise SystemExit(1)
PY
    else
      native_identity_source_by_recipe[$recipe_hash]=$path
      native_identity_source_hash_by_recipe[$recipe_hash]=$hash
    fi
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$index" "${path#$output_dir/}" "$hash" $fields >>"$native_identity_report"
}

record_cargo_units() {
  local revision=$1 measurement_phase=$2 service=$3 index=$4 cargo_mode=$5
  if [ "$cargo_mode" -eq 0 ]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$revision" "$measurement_phase" "$service" "$index" not-applicable not-applicable \
      not-applicable not-applicable not-applicable >>"$cargo_units_report"
    return 0
  fi
  if [ "$BENCH_COMPILER_VERTEX_STATE" = cached ]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$revision" "$measurement_phase" "$service" "$index" cached-not-executed \
      not-applicable "$BENCH_CARGO_RAW_LOG_SHA256" 0 "$BENCH_COMPILER_VERTEX" >>"$cargo_units_report"
    return 0
  fi
  [ "$BENCH_CARGO_UNITS_PATH" != "" ] && [ "$BENCH_CARGO_UNITS_PATH" != not-executed-cached ] || return 1
  [[ "$BENCH_CARGO_UNITS_SHA256" =~ ^[0-9a-f]{64}$ ]] || return 1
  is_decimal "$BENCH_CARGO_UNITS_COUNT" || return 1
  [[ "$BENCH_CARGO_RAW_LOG_SHA256" =~ ^[0-9a-f]{64}$ ]] || return 1
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$index" \
    "${BENCH_CARGO_UNITS_PATH#$output_dir/}" "$BENCH_CARGO_UNITS_SHA256" \
    "$BENCH_CARGO_RAW_LOG_SHA256" "$BENCH_CARGO_UNITS_COUNT" "$BENCH_COMPILER_VERTEX" >>"$cargo_units_report"
}

write_source_archive_request() {
  local checkout=$1 service=$2 commit=$3 destination=$4 runtime_inventory
  [ -d "$checkout" ] && [ ! -L "$checkout" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  runtime_inventory=$(rbl_runtime_payload_inventory \
    "$checkout" "$repo_root/deploy/build/release-build-layout.json") || return 1
  RBL_BENCH_SOURCE_ROOT=$checkout RBL_BENCH_LAYOUT=$repo_root/deploy/build/release-build-layout.json \
    RBL_BENCH_SERVICE=$service RBL_BENCH_COMMIT=$commit RBL_BENCH_REQUEST=$destination \
    RBL_BENCH_RUNTIME_INVENTORY=$runtime_inventory RBL_RUNTIME_FORMAT=$RBL_RUNTIME_INVENTORY_FORMAT \
    RBL_DOCKERIGNORE_HASH=$RBL_DOCKERIGNORE_SHA256 python3 - <<'PY'
import hashlib
import json
import os
import posixpath
import stat
import struct

root = os.path.abspath(os.environ["RBL_BENCH_SOURCE_ROOT"])
layout_path = os.environ["RBL_BENCH_LAYOUT"]
service = os.environ["RBL_BENCH_SERVICE"]
commit = os.environ["RBL_BENCH_COMMIT"]
out_path = os.environ["RBL_BENCH_REQUEST"]
runtime_inventory = json.loads(os.environ["RBL_BENCH_RUNTIME_INVENTORY"])

def exact_commit(value):
    return isinstance(value, str) and len(value) == 40 and all(char in "0123456789abcdef" for char in value)

if not os.path.isdir(root) or os.path.islink(root) or not exact_commit(commit):
    raise SystemExit("benchmark-source-request-input-invalid")
if not os.path.isfile(layout_path) or os.path.islink(layout_path):
    raise SystemExit("benchmark-layout-invalid")
layout = json.load(open(layout_path, encoding="utf-8"))
if layout.get("format") != "lagrange-build-layout-v1" or layout.get("schema_version") != 1:
    raise SystemExit("benchmark-layout-schema-invalid")
if (runtime_inventory.get("format") != os.environ["RBL_RUNTIME_FORMAT"]
        or runtime_inventory.get("dockerignore_sha256") != os.environ["RBL_DOCKERIGNORE_HASH"]):
    raise SystemExit("benchmark-runtime-inventory-invalid")

def image_path(value):
    if not isinstance(value, str) or not value.startswith("/") or value.endswith("/") or "\\" in value:
        raise SystemExit("benchmark-image-path-invalid")
    value = value[1:]
    if (not value or posixpath.normpath(value) != value or value == "." or value.startswith("../")
            or "/../" in value or "/./" in value or any(part in ("", ".", "..") for part in value.split("/"))):
        raise SystemExit("benchmark-image-path-invalid")
    return value

def source_path(value):
    if (not isinstance(value, str) or not value or value.startswith("/") or "\\" in value
            or posixpath.normpath(value) != value or value == "." or value.startswith("../")
            or "/../" in value or "/./" in value):
        raise SystemExit("benchmark-source-path-invalid")
    path = os.path.abspath(os.path.join(root, value))
    if os.path.commonpath((root, path)) != root:
        raise SystemExit("benchmark-source-path-escapes")
    return path

def digest(path):
    result = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()

def elf(path):
    with open(path, "rb") as handle:
        head = handle.read(20)
    return (len(head) >= 20 and head[:4] == b"\x7fELF" and head[4] == 2 and head[5] == 1
            and struct.unpack("<H", head[18:20])[0] == 62)

files = {}
directories = set()

def add_file(path, sha, executable, is_elf, literals):
    path = image_path(path)
    item = {"path": path, "sha256": sha, "executable": bool(executable),
            "elf": bool(is_elf), "contains_hex": sorted(literals)}
    old = files.get(path)
    if old is not None and old != item:
        raise SystemExit("benchmark-source-request-path-conflict")
    files[path] = item

def selected_source(entry):
    path = source_path(entry["path"])
    try: info = os.lstat(path)
    except FileNotFoundError: raise SystemExit("benchmark-source-entry-missing")
    if format(stat.S_IMODE(info.st_mode), "04o") != entry["mode"]:
        raise SystemExit("benchmark-source-mode-changed")
    if entry["kind"] == "directory":
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or entry["sha256"] is not None:
            raise SystemExit("benchmark-source-directory-invalid")
    elif entry["kind"] == "file":
        if (stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode)
                or digest(path) != entry["sha256"]):
            raise SystemExit("benchmark-source-file-invalid")
    else:
        raise SystemExit("benchmark-source-kind-invalid")
    return path, info

def add_source(payload):
    if not isinstance(payload, dict) or set(payload) != {"entries", "image", "source", "tree_sha256"}:
        raise SystemExit("benchmark-runtime-payload-invalid")
    source = payload["source"]
    image = image_path(payload["image"])
    entries = payload["entries"]
    if not isinstance(entries, list) or not entries:
        raise SystemExit("benchmark-runtime-payload-entries-invalid")
    root_entry = next((entry for entry in entries if entry.get("path") == source), None)
    if root_entry is None:
        raise SystemExit("benchmark-runtime-payload-root-missing")
    if root_entry.get("kind") == "file":
        if len(entries) != 1:
            raise SystemExit("benchmark-runtime-file-payload-invalid")
        path, info = selected_source(root_entry)
        add_file("/" + image, root_entry["sha256"],
                 bool(stat.S_IMODE(info.st_mode) & 0o111), elf(path), [])
        return
    if root_entry.get("kind") != "directory":
        raise SystemExit("benchmark-runtime-directory-payload-invalid")
    directories.add(image)
    for entry in entries:
        path, info = selected_source(entry)
        if entry["kind"] != "file":
            continue
        relative = posixpath.relpath(entry["path"], source)
        if relative == "." or relative.startswith("../"):
            raise SystemExit("benchmark-runtime-entry-outside-payload")
        add_file("/" + image + "/" + relative, entry["sha256"],
                 bool(stat.S_IMODE(info.st_mode) & 0o111), elf(path), [])

def add_payloads(name):
    values = runtime_inventory.get("payload_groups", {}).get(name)
    declared = layout.get("runtime_payloads", {}).get(name)
    if not isinstance(values, list) or not isinstance(declared, list):
        raise SystemExit("benchmark-runtime-payload-invalid")
    if ([{"source": item.get("source"), "image": item.get("image")} for item in values]
            != declared):
        raise SystemExit("benchmark-runtime-payload-declaration-mismatch")
    for item in values:
        add_source(item)

record = layout.get("services", {}).get(service)
if not isinstance(record, dict) or set(record) != {"kind", "recipe"}:
    raise SystemExit("benchmark-service-record-invalid")
if record["kind"] == "rust":
    recipe_id = record["recipe"]
    recipe = layout.get("recipes", {}).get(recipe_id)
    if not isinstance(recipe, dict) or not isinstance(recipe.get("runtime_binaries"), list):
        raise SystemExit("benchmark-rust-recipe-invalid")
    for item in recipe["runtime_binaries"]:
        if not isinstance(item, dict) or set(item) != {"bin", "image"}:
            raise SystemExit("benchmark-runtime-binary-invalid")
        literals = [commit.encode("ascii").hex()] if (
            recipe_id == "D5" and recipe.get("backtest_commit_literal") == item["image"]) else []
        add_file(item["image"], None, True, True, literals)
    if "runtime_payload" in recipe:
        add_payloads(recipe["runtime_payload"])
elif record["kind"] == "database":
    add_file("/usr/local/bin/sqlx", None, True, True, [])
    add_payloads("database")
elif record["kind"] == "web":
    add_file("/app/apps/web/server.js", None, False, False, [])
    directories.update(("app/apps/web/.next", "app/apps/web/.next/static"))
else:
    raise SystemExit("benchmark-service-kind-invalid")

request = {"format": "lagrange-image-files-v1",
           "files": [files[key] for key in sorted(files)],
           "nonempty_directories": sorted(directories)}
parent = os.path.dirname(out_path)
if not os.path.isdir(parent) or os.path.islink(parent) or os.path.lexists(out_path):
    raise SystemExit("benchmark-request-output-invalid")
data = (json.dumps(request, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
PY
}

verify_source_benchmark_archive() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 service=$5 index=$6 image_id=$7
  local request result archive save_log request_hash result_hash fields
  request=$output_dir/archive-requests/${revision}-${measurement_phase}-${index}-${service}.json
  result=$output_dir/archive-results/${revision}-${measurement_phase}-${index}-${service}.json
  archive=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.image-save.tar
  save_log=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.image-save.log.raw
  # The request is materialized before the image build.  That makes the exact
  # requested bytes part of the build's auditable preflight and lets the
  # no-daemon fake construct only a root-bound OCI archive for that request.
  [ -f "$request" ] && [ ! -L "$request" ] || return 1
  # The public --apply path always scans every one of its twelve images in
  # every phase.  The private no-daemon state-machine fixture bounds repeated
  # copies of that identical parser exercise, while retaining a complete
  # twelve-service root-bound OCI scan in its primary run and explicit direct
  # and provenance roots.  This is not an environment-controlled apply
  # bypass: `internal_self_test` is set only by --self-test.
  if [ "$internal_self_test" -eq 1 ] && [ -n "${BENCH_TEST_ARCHIVE_SCAN_LIMIT:-}" ]; then
    is_decimal "$BENCH_TEST_ARCHIVE_SCAN_LIMIT" || return 1
    BENCH_TEST_ARCHIVE_SCAN_COUNT=${BENCH_TEST_ARCHIVE_SCAN_COUNT:-0}
    is_decimal "$BENCH_TEST_ARCHIVE_SCAN_COUNT" || return 1
    if [ "$BENCH_TEST_ARCHIVE_SCAN_COUNT" -ge "$BENCH_TEST_ARCHIVE_SCAN_LIMIT" ]; then
      return 0
    fi
    BENCH_TEST_ARCHIVE_SCAN_COUNT=$((BENCH_TEST_ARCHIVE_SCAN_COUNT + 1))
  fi
  request_hash=$(sha256_file "$request") || return 1
  if [ "$internal_self_test" -eq 1 ]; then
    BENCH_TEST_ARCHIVE_REQUEST=$request BENCH_TEST_ARCHIVE_COMMIT=$commit \
      docker image save --output "$archive" "$image_id" >"$save_log" 2>&1 || return 1
  else
    docker image save --output "$archive" "$image_id" >"$save_log" 2>&1 || return 1
  fi
  [ -f "$archive" ] && [ ! -L "$archive" ] || return 1
  release_build_layout_archive_scan "$archive" "$image_id" linux/amd64 "$commit" "$request" "$result" || return 1
  result_hash=$(sha256_file "$result") || return 1
  fields=$(RBL_BENCH_RESULT=$result RBL_BENCH_IMAGE_ID=$image_id RBL_BENCH_REQUEST_HASH=$request_hash python3 - <<'PY'
import json
import os
import re
value=json.load(open(os.environ["RBL_BENCH_RESULT"],encoding="utf-8"))
if value.get("format")!="lagrange-image-files-result-v1" or value.get("image_id")!=os.environ["RBL_BENCH_IMAGE_ID"] or value.get("request_sha256")!=os.environ["RBL_BENCH_REQUEST_HASH"]:
    raise SystemExit(1)
items=[value.get("archive_sha256"),value.get("request_sha256"),value.get("manifest_digest"),value.get("config_digest")]
if not all(isinstance(item,str) and re.fullmatch(r"[0-9a-f]{64}",item) for item in items[:2]): raise SystemExit(1)
if not all(isinstance(item,str) and re.fullmatch(r"sha256:[0-9a-f]{64}",item) for item in items[2:]): raise SystemExit(1)
print("\t".join(items))
PY
) || return 1
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$index" "${result#$output_dir/}" "$result_hash" $fields "$image_id" >>"$archive_validation_report"
}

build_one_service() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 service=$5 index=$6 namespace=$7 cold_nonce=$8 image_prefix=$9
  local tag raw_log evidence_rel evidence_log start_ms end_ms total_ms cargo_ms cargo_cache status
  local image_verification verify_ms image_id image_revision image_size units_path archive_request
  local -a args=()

  tag="${image_prefix}-${service}:${commit}"
  safe_benchmark_repository "${tag%%:*}" || die 'generated benchmark image repository is not valid lowercase Docker grammar'
  [ "${tag##*:}" = "$commit" ] || die 'generated benchmark image tag is not bound to the measured commit'
  raw_log=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.build.log.raw
  evidence_rel="evidence/${revision}-${measurement_phase}-${index}-${service}.build.log"
  evidence_log=$output_dir/$evidence_rel
  archive_request=$output_dir/archive-requests/${revision}-${measurement_phase}-${index}-${service}.json
  if ! write_source_archive_request "$checkout" "$service" "$commit" "$archive_request"; then
    failure_stage=image-archive-request
    die "offline image archive request could not be materialized for $revision/$measurement_phase/$service"
  fi
  tag_list+=("$tag")
  args=(
    build
    --pull=false
    --progress=plain
    --build-arg "LAGRANGE_CODE_COMMIT=$commit"
    --build-arg CARGO_BUILD_JOBS=2
    # BUILDKIT_CACHE_MOUNT_NS is a Docker built-in build ARG.  A host
    # environment assignment is not forwarding, so every invocation passes it.
    --build-arg "BUILDKIT_CACHE_MOUNT_NS=$namespace"
    --build-arg "LAGRANGE_BENCHMARK_COLD_NONCE=$cold_nonce"
    -f "${instrumented_dockerfile[$revision:$service]}"
    -t "$tag"
  )
  args+=("$checkout")

  if ! record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" before-build; then
    failure_stage=resource-sample
    die "resource sampling was unavailable before $revision/$measurement_phase/$service"
  fi
  if ! record_disk_snapshot "$revision" "$measurement_phase" "$service" before-build -; then
    failure_stage=disk-cost-snapshot
    die "Docker image/cache disk evidence was unavailable before $revision/$measurement_phase/$service"
  fi
  if ! start_resource_sampler "$revision" "$measurement_phase" "$service"; then
    failure_stage=resource-sample
    die "resource sampler could not start for $revision/$measurement_phase/$service"
  fi
  start_ms=$(now_ms)
  if [ "$internal_self_test" -eq 1 ]; then
    if BENCH_TEST_ARCHIVE_REQUEST=$archive_request BENCH_TEST_ARCHIVE_COMMIT=$commit \
      CARGO_BUILD_JOBS=2 DOCKER_BUILDKIT=1 docker "${args[@]}" >"$raw_log" 2>&1; then
      last_build_exit=0
    else
      status=$?
      last_build_exit=$status
      stop_resource_sampler || true
      record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" after-build-failure || true
      sanitize_text_file "$raw_log" "$evidence_log"
      failure_stage=build
      die "Docker build failed for $revision/$measurement_phase/$service (sanitized evidence: $evidence_rel)"
    fi
  elif CARGO_BUILD_JOBS=2 DOCKER_BUILDKIT=1 docker "${args[@]}" >"$raw_log" 2>&1; then
    last_build_exit=0
  else
    status=$?
    last_build_exit=$status
    stop_resource_sampler || true
    record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" after-build-failure || true
    sanitize_text_file "$raw_log" "$evidence_log"
    failure_stage=build
    die "Docker build failed for $revision/$measurement_phase/$service (sanitized evidence: $evidence_rel)"
  fi
  end_ms=$(now_ms)
  total_ms=$((end_ms - start_ms))
  is_decimal "$total_ms" || die 'total build time is not numeric'
  if ! stop_resource_sampler; then
    failure_stage=resource-sample
    die "resource sampler was unavailable during $revision/$measurement_phase/$service"
  fi
  if ! record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" after-build; then
    failure_stage=resource-sample
    die "resource sampling was unavailable after $revision/$measurement_phase/$service"
  fi
  sanitize_text_file "$raw_log" "$evidence_log"
  units_path=$output_dir/cargo-units/${revision}-${measurement_phase}-${index}-${service}.json
  if ! parse_build_events "$raw_log" "${service_cargo_mode[$service]}" "$revision" "$units_path"; then
    failure_stage=build-evidence
    die "invalid Cargo/BuildKit evidence for $revision/$measurement_phase/$service: $BENCH_PARSE_ERROR"
  fi
  if ! record_cargo_units "$revision" "$measurement_phase" "$service" "$index" "${service_cargo_mode[$service]}"; then
    failure_stage=cargo-unit-evidence
    die "Cargo JSON unit evidence could not be retained for $revision/$measurement_phase/$service"
  fi
  if ! record_native_identity "$revision" "$measurement_phase" "$service" "$index" "$raw_log" \
    "${service_cargo_mode[$service]}" "${instrumented_dockerfile[$revision:$service]}" "$namespace" "$cold_nonce" "$commit"; then
    failure_stage=native-identity-evidence
    die "native tool identity evidence could not be retained for $revision/$measurement_phase/$service"
  fi
  cargo_ms=${BENCH_CARGO_STEP_MS:--}
  cargo_cache=$(derive_cargo_cache_state "${service_cargo_mode[$service]}")
  if ! image_verification=$(inspect_benchmark_image "$tag" "$commit"); then
    failure_stage=image-verification
    die "temporary image verification failed for $revision/$measurement_phase/$service"
  fi
  IFS=$'\t' read -r verify_ms image_id image_revision image_size <<<"$image_verification"
  if ! verify_source_benchmark_archive "$revision" "$measurement_phase" "$checkout" "$commit" "$service" "$index" "$image_id"; then
    failure_stage=image-archive-verification
    die "offline image-save archive verification failed for $revision/$measurement_phase/$service"
  fi
  if [ "$source_manifest_active" -eq 1 ]; then
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=$tag
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=$image_id
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$image_revision
  fi
  if ! record_disk_snapshot "$revision" "$measurement_phase" "$service" after-image-save "$image_size"; then
    failure_stage=disk-cost-snapshot
    die "Docker image/cache disk evidence was unavailable after image-save for $revision/$measurement_phase/$service"
  fi
  append_result "$revision" "$measurement_phase" "$service" "$index" "$tag" "$namespace" "$commit" \
    "${source_identity_by_revision_scenario[$revision:$measurement_phase]}" "$total_ms" "$cargo_ms" \
    "$verify_ms" "$image_id" "$image_revision" "$cargo_cache" "$evidence_rel" "$scenario"
  printf 'BENCHMARK_SERVICE PASS revision=%s measurement_phase=%s selected_scenario=%s service=%s image_build_ms=%s cargo_ms=%s image_verification_ms=%s compiler_vertex=%s compiler_state=%s cargo_cache=%s compiled=%s fresh=%s\n' \
    "$revision" "$measurement_phase" "$scenario" "$service" "$total_ms" "$cargo_ms" "$verify_ms" "$BENCH_COMPILER_VERTEX" \
    "$BENCH_COMPILER_VERTEX_STATE" "$cargo_cache" "$(csv_or_dash "$BENCH_COMPILED_PACKAGES")" \
    "$(csv_or_dash "$BENCH_FRESH_PACKAGES")"
}

record_common_archive_result() {
  local revision=$1 measurement_phase=$2 service=$3 index=$4 state_root=$5 image_id=$6
  local result result_hash fields
  result=$state_root/verification/$service.json
  [ -f "$result" ] && [ ! -L "$result" ] || return 1
  [ "$(stat -c '%u:%a' -- "$result")" = "$(id -u):600" ] || return 1
  result_hash=$(sha256_file "$result") || return 1
  fields=$(RBL_BENCH_RESULT=$result RBL_BENCH_IMAGE_ID=$image_id python3 - <<'PY'
import json
import os
import re

raw = open(os.environ["RBL_BENCH_RESULT"], "rb").read()
value = json.loads(raw.decode("utf-8"))
if raw != (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8"):
    raise SystemExit(1)
if value.get("format") != "lagrange-image-files-result-v1" or value.get("image_id") != os.environ["RBL_BENCH_IMAGE_ID"]:
    raise SystemExit(1)
fields = [value.get(name) for name in ("archive_sha256", "request_sha256", "manifest_digest", "config_digest")]
if not all(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) for value in fields[:2]):
    raise SystemExit(1)
if not all(isinstance(value, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", value) for value in fields[2:]):
    raise SystemExit(1)
print("\t".join(fields))
PY
) || return 1
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "$service" "$index" "${result#$output_dir/}" "$result_hash" \
    $fields "$image_id" >>"$archive_validation_report"
}

record_common_native_identity() {
  local revision=$1 measurement_phase=$2 state_root=$3 native fields
  native=$state_root/native-identity.json
  [ -f "$native" ] && [ ! -L "$native" ] || return 1
  [ "$(stat -c '%u:%a' -- "$native")" = "$(id -u):600" ] || return 1
  rbl_validate_native_identity "$native" || return 1
  fields=$(RBL_BENCH_NATIVE=$native python3 - <<'PY'
import hashlib
import json
import os

raw = open(os.environ["RBL_BENCH_NATIVE"], "rb").read()
value = json.loads(raw.decode("utf-8"))
if raw != (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8"):
    raise SystemExit(1)
if (value.get("format") != "lagrange-build-layout-native-v3" or
        value.get("target_platform") != "linux/amd64" or
        value.get("host_triple") != "x86_64-unknown-linux-musl" or
        not isinstance(value.get("native_packages"), list) or
        value.get("native_packages") != ["build-base", "musl-dev", "openssl-dev", "pkgconf", "postgresql-dev"] or
        not isinstance(value.get("apk_installed_packages"), list) or
        not value["apk_installed_packages"]):
    raise SystemExit(1)
for key in ("rustc_vv", "cargo_version", "apk_info_vv"):
    if not isinstance(value.get(key), str) or not value[key]:
        raise SystemExit(1)
print("\t".join((hashlib.sha256(raw).hexdigest(),
                  hashlib.sha256(value["rustc_vv"].encode()).hexdigest(),
                  hashlib.sha256(value["cargo_version"].encode()).hexdigest(),
                  hashlib.sha256(value["apk_info_vv"].encode()).hexdigest(),
                  str(len(value["apk_installed_packages"])))))
PY
) || return 1
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" common-native 0 "${native#$output_dir/}" $fields >>"$native_identity_report"
}

record_common_cargo_units() {
  local receipt=$1 cargo_json=$2 destination=$3
  [ -f "$receipt" ] && [ ! -L "$receipt" ] && [ -f "$cargo_json" ] && [ ! -L "$cargo_json" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  RBL_BENCH_RECEIPT=$receipt RBL_BENCH_CARGO_JSON=$cargo_json RBL_BENCH_UNITS=$destination python3 - <<'PY'
import hashlib
import json
import os
import re

receipt_path = os.environ["RBL_BENCH_RECEIPT"]
cargo_path = os.environ["RBL_BENCH_CARGO_JSON"]
out_path = os.environ["RBL_BENCH_UNITS"]
receipt_raw = open(receipt_path, "rb").read()
cargo_raw = open(cargo_path, "rb").read()
if len(cargo_raw) > 256 * 1024 * 1024:
    raise SystemExit("common-cargo-json-too-large")
def pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError("duplicate-key")
        result[key] = value
    return result
receipt = json.loads(receipt_raw.decode("utf-8"), object_pairs_hook=pairs,
                     parse_constant=lambda _value: (_ for _ in ()).throw(ValueError("constant")))
if receipt_raw != (json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8"):
    raise SystemExit("common-receipt-not-canonical")
bin_name = receipt.get("bin")
if not isinstance(bin_name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+", bin_name):
    raise SystemExit("common-receipt-bin-invalid")
units = []
finished = 0
for raw in cargo_raw.splitlines():
    if not raw:
        continue
    event = json.loads(raw.decode("utf-8"), object_pairs_hook=pairs,
                       parse_constant=lambda _value: (_ for _ in ()).throw(ValueError("constant")))
    if not isinstance(event, dict):
        raise SystemExit("common-cargo-event-invalid")
    if event.get("reason") == "compiler-artifact":
        target = event.get("target")
        profile = event.get("profile")
        if (not isinstance(event.get("package_id"), str) or not isinstance(target, dict) or
                not isinstance(target.get("name"), str) or not isinstance(target.get("kind"), list) or
                not isinstance(target.get("crate_types"), list) or not isinstance(profile, dict) or
                not isinstance(event.get("features"), list) or not isinstance(event.get("fresh"), bool) or
                "executable" not in event or
                (event["executable"] is not None and not isinstance(event["executable"], str))):
            raise SystemExit("common-cargo-unit-invalid")
        unit = {"package_id": event["package_id"], "target": {"name": target["name"],
                "kind": sorted(target["kind"]), "crate_types": sorted(target["crate_types"])},
                "features": sorted(event["features"]), "profile": profile,
                "fresh": event["fresh"], "executable": event["executable"]}
        units.append(unit)
    elif event.get("reason") == "build-finished":
        if event.get("success") is not True:
            raise SystemExit("common-cargo-build-failed")
        finished += 1
if finished != 1 or not units:
    raise SystemExit("common-cargo-units-missing")
if not any(item["target"]["name"] == bin_name and item["executable"] == "/cargo-target/release/" + bin_name for item in units):
    raise SystemExit("common-cargo-target-missing")
value = {"format": "lagrange-benchmark-cargo-units-v1", "source": "common-producer-receipt",
         "raw_log_sha256": hashlib.sha256(cargo_raw).hexdigest(), "units": units}
data = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
fd = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "wb") as handle:
    handle.write(data)
    handle.flush()
    os.fsync(handle.fileno())
print("%s\t%s\t%s" % (hashlib.sha256(data).hexdigest(), hashlib.sha256(cargo_raw).hexdigest(), len(units)))
PY
}

record_common_producer_evidence() {
  local revision=$1 measurement_phase=$2 checkout=$3 state_root=$4 recipe bin receipt cargo timing bundle units fields bundle_digest
  local index=0
  record_common_native_identity "$revision" "$measurement_phase" "$state_root" ||
    die 'common-C native identity evidence is missing or malformed'
  while IFS=$'\t' read -r recipe bin; do
    [ -n "$recipe" ] && [ -n "$bin" ] || die 'common-C layout producer list is malformed'
    receipt=$state_root/producers/$recipe/$bin/artifact.json
    cargo=$state_root/producers/$recipe/$bin/cargo.jsonl
    timing=$state_root/producers/$recipe/$bin/timing.json
    bundle=$state_root/bundles/$recipe
    [ -f "$receipt" ] && [ ! -L "$receipt" ] && [ -f "$cargo" ] && [ ! -L "$cargo" ] &&
      [ -f "$timing" ] && [ ! -L "$timing" ] && [ -d "$bundle" ] && [ ! -L "$bundle" ] ||
      die "common-C producer evidence is missing: $recipe/$bin"
    bundle_digest=${common_bundle_digest_by_recipe[$recipe]:-}
    [[ "$bundle_digest" =~ ^[0-9a-f]{64}$ ]] ||
      die "common-C bundle digest evidence is missing: $recipe"
    index=$((index + 1))
    units=$output_dir/cargo-units/${revision}-${measurement_phase}-producer-${recipe}-${bin}.json
    fields=$(record_common_cargo_units "$receipt" "$cargo" "$units") ||
      die "common-C Cargo JSON receipt evidence is malformed: $recipe/$bin"
    RBL_BENCH_TIMING=$timing python3 - <<'PY' >"$tmp_dir/common-cargo-ms.$$.tmp"
import json
import os
value=json.load(open(os.environ["RBL_BENCH_TIMING"],encoding="utf-8"))
if set(value)!={"format","cargo_ms"} or value.get("format")!="lagrange-cargo-timing-v1" or type(value.get("cargo_ms")) is not int or value["cargo_ms"]<0:
    raise SystemExit(1)
print(value["cargo_ms"])
PY
    cargo_ms=$(<"$tmp_dir/common-cargo-ms.$$.tmp") || die 'common-C receipt timing could not be read'
    rm -f -- "$tmp_dir/common-cargo-ms.$$.tmp"
    is_decimal "$cargo_ms" || die 'common-C receipt timing is not numeric'
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$revision" "$measurement_phase" "$recipe" "$bin" "${receipt#$output_dir/}" "$(sha256_file "$receipt")" \
      "${cargo#$output_dir/}" "$(sha256_file "$cargo")" "$cargo_ms" "${bundle#$output_dir/}" "$bundle_digest" >>"$common_producer_report"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$revision" "$measurement_phase" "producer-$recipe-$bin" "$index" \
      "${units#$output_dir/}" $fields "receipt:$recipe/$bin" >>"$cargo_units_report"
  done < <(RBL_BENCH_LAYOUT=$checkout/deploy/build/release-build-layout.json python3 - <<'PY'
import json, os
layout=json.load(open(os.environ["RBL_BENCH_LAYOUT"],encoding="utf-8"))
for recipe in ("D1","D6","D2","D3","D4","D5","D7"):
    value=layout.get("recipes",{}).get(recipe)
    if not isinstance(value,dict) or not isinstance(value.get("bins"),list): raise SystemExit(1)
    for name in value["bins"]:
        if not isinstance(name,str): raise SystemExit(1)
        print(recipe+"\t"+name)
PY
)
}

common_private_directory() {
  local directory=$1
  if [ -e "$directory" ] || [ -L "$directory" ]; then
    [ -d "$directory" ] && [ ! -L "$directory" ] || return 1
  else
    mkdir -m 0700 -- "$directory" || return 1
  fi
  [ "$(stat -c '%u:%a' -- "$directory")" = "$(id -u):700" ]
}

assert_common_checkout_binding() {
  local revision=$1 checkout=$2 commit=$3 helper config artifact manifest
  [ "${side_layout_kind[$revision]:-}" = common ] || return 1
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] || return 1
  [ -z "$(git -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] || return 1
  helper=${side_layout_helper[$revision]:-}
  config=$checkout/deploy/build/release-build-layout.json
  artifact=$checkout/deploy/build/Dockerfile.rust-artifacts
  manifest=$checkout/scripts/ops/lib/release-image-manifest.sh
  [ -f "$helper" ] && [ ! -L "$helper" ] && [ -f "$config" ] && [ ! -L "$config" ] &&
    [ -f "$artifact" ] && [ ! -L "$artifact" ] && [ -f "$manifest" ] && [ ! -L "$manifest" ] || return 1
  [ "$(sha256_file "$helper")" = "${side_layout_helper_hash[$revision]:-}" ] &&
    [ "$(sha256_file "$config")" = "${side_layout_config_hash[$revision]:-}" ] &&
    [ "$(sha256_file "$artifact")" = "${side_layout_artifact_hash[$revision]:-}" ] &&
    [ "$(sha256_file "$manifest")" = "${side_manifest_library_hash[$revision]:-}" ]
}

common_bundle_recipe() {
  local state_root=$1 bundle=$2 recipe
  case "$bundle" in "$state_root"/bundles/D[1-7]) ;; *) return 1 ;; esac
  recipe=${bundle##*/}
  printf '%s' "$recipe"
}

common_manifest_service_order() {
  local service index=0
  [ "${#RELEASE_IMAGE_SERVICES[@]}" -eq "${#services[@]}" ] || return 1
  for service in "${services[@]}"; do
    [ "${RELEASE_IMAGE_SERVICES[$index]:-}" = "$service" ] || return 1
    index=$((index + 1))
  done
}

canonical_benchmark_service_order() {
  [ "${#services[@]}" -eq 12 ] &&
    [ "${services[*]}" = 'db-role-bootstrap db-migrate api-server web research-worker recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler' ]
}

publish_common_manifest_and_revalidate() {
  local library=$1 destination=$2 commit=$3 image_prefix=$4 state_root=$5
  local temporary service expected image_verification verify_ms image_id image_revision image_size
  local parent=${destination%/*}
  [ -f "$library" ] && [ ! -L "$library" ] && [ -d "$parent" ] && [ ! -L "$parent" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  # `run_common_c_release` sourced this instrumented copy before it populated
  # the V2 arrays.  Do not source it here again: its declarations deliberately
  # reset those arrays, which would discard the observed image identities.  Do
  # not reset them here either: this function must publish exactly the twelve
  # IDs already verified against saved image bytes.
  common_manifest_service_order || return 1
  for service in "${services[@]}"; do
    expected=$(release_image_manifest_ref_for "$service" "$commit") || return 1
    [ "$expected" = "$image_prefix-$service:$commit" ] || return 1
    [ "${RELEASE_IMAGE_MANIFEST_REFS[$service]:-}" = "$expected" ] || return 1
    release_image_manifest_is_image_id "${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}" || return 1
    [ "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}" = "$commit" ] || return 1
  done
  temporary=$(mktemp "$parent/.benchmark-v2.XXXXXX") || return 1
  chmod 0600 -- "$temporary" || { rm -f -- "$temporary"; return 1; }
  if ! release_image_manifest_write "$temporary" "$commit"; then
    rm -f -- "$temporary"
    return 1
  fi
  if ! release_image_manifest_load "$temporary" "$commit"; then
    rm -f -- "$temporary"
    return 1
  fi
  # Link is a no-clobber publication primitive in this private output parent.
  if ! ln -- "$temporary" "$destination"; then
    rm -f -- "$temporary"
    return 1
  fi
  rm -f -- "$temporary"
  [ "$(stat -c '%u:%a' -- "$destination")" = "$(id -u):600" ] || return 1
  release_image_manifest_load "$destination" "$commit" || return 1
  for service in "${services[@]}"; do
    expected=$(release_image_manifest_ref_for "$service" "$commit") || return 1
    image_verification=$(inspect_benchmark_image "$expected" "$commit") || return 1
    IFS=$'\t' read -r verify_ms image_id image_revision image_size <<<"$image_verification"
    [ "$image_id" = "${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}" ] &&
      [ "$image_revision" = "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}" ] || return 1
    # Re-run the frozen strict byte binding against the final exact image ID;
    # an existing helper result must byte-match rather than merely be present.
    release_build_layout_verify_image "$service" "$commit" "$image_id" "$state_root" || return 1
  done
}

revalidate_source_benchmark_archive() {
  local revision=$1 measurement_phase=$2 service=$3 index=$4 commit=$5 image_id=$6
  local row result_rel result result_hash archive_hash request_hash manifest_digest config_digest recorded_id extra
  local request archive temporary
  row=$(awk -F '\t' -v revision="$revision" -v phase="$measurement_phase" -v service="$service" -v ordinal="$index" '
    NR > 1 && $1 == revision && $2 == phase && $3 == service && $4 == ordinal {
      if (found++) exit 2
      value=$0
    }
    END {
      if (found == 1) print value
      else if (found > 1) exit 2
    }
  ' "$archive_validation_report") || return 1
  if [ -z "$row" ]; then
    # The no-daemon self-test deliberately performs one complete twelve-image
    # source-layout scan, then bounds identical parser repetitions.  Real
    # --apply has no such branch and must retain a result row for every image.
    if [ "$internal_self_test" -eq 1 ] && [ -n "${BENCH_TEST_ARCHIVE_SCAN_LIMIT:-}" ] &&
       is_decimal "${BENCH_TEST_ARCHIVE_SCAN_COUNT:-}" &&
       [ "$BENCH_TEST_ARCHIVE_SCAN_COUNT" -ge "$BENCH_TEST_ARCHIVE_SCAN_LIMIT" ]; then
      return 0
    fi
    return 1
  fi
  IFS=$'\t' read -r _ _ _ _ result_rel result_hash archive_hash request_hash \
    manifest_digest config_digest recorded_id extra <<<"$row"
  [ -z "$extra" ] && [ "$recorded_id" = "$image_id" ] || return 1
  case "$result_rel" in
    /*) result=$result_rel ;;
    *) result=$output_dir/$result_rel ;;
  esac
  request=$output_dir/archive-requests/${revision}-${measurement_phase}-${index}-${service}.json
  archive=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.image-save.tar
  [ -f "$result" ] && [ ! -L "$result" ] && [ -f "$request" ] && [ ! -L "$request" ] &&
    [ -f "$archive" ] && [ ! -L "$archive" ] || return 1
  [ "$(sha256_file "$result")" = "$result_hash" ] &&
    [ "$(sha256_file "$request")" = "$request_hash" ] &&
    [ "$(sha256_file "$archive")" = "$archive_hash" ] || return 1
  [[ "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]] &&
    [[ "$config_digest" =~ ^sha256:[0-9a-f]{64}$ ]] || return 1
  temporary=$(mktemp "$tmp_dir/.source-archive-revalidation.XXXXXX") || return 1
  rm -f -- "$temporary" || return 1
  if ! release_build_layout_archive_scan "$archive" "$image_id" linux/amd64 "$commit" "$request" "$temporary"; then
    rm -f -- "$temporary"
    return 1
  fi
  if ! cmp -s -- "$temporary" "$result"; then
    rm -f -- "$temporary"
    return 1
  fi
  rm -f -- "$temporary"
}

publish_source_manifest_and_revalidate() {
  local library=$1 destination=$2 commit=$3 image_prefix=$4 revision=$5 measurement_phase=$6
  local parent=${destination%/*} temporary service expected image_verification verify_ms image_id image_revision image_size
  local index=0
  [ -f "$library" ] && [ ! -L "$library" ] && [ -d "$parent" ] && [ ! -L "$parent" ] || return 1
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
  # The caller sourced this exact instrumented library before collecting the
  # twelve IDs.  Re-sourcing it here would reset those arrays and erase the
  # evidence that must be serialized.
  common_manifest_service_order || return 1
  for service in "${services[@]}"; do
    expected=$(release_image_manifest_ref_for "$service" "$commit") || return 1
    [ "$expected" = "$image_prefix-$service:$commit" ] || return 1
    [ "${RELEASE_IMAGE_MANIFEST_REFS[$service]:-}" = "$expected" ] || return 1
    release_image_manifest_is_image_id "${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}" || return 1
    [ "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}" = "$commit" ] || return 1
  done
  temporary=$(mktemp "$parent/.benchmark-v2.XXXXXX") || return 1
  chmod 0600 -- "$temporary" || { rm -f -- "$temporary"; return 1; }
  if ! release_image_manifest_write "$temporary" "$commit" ||
     ! release_image_manifest_load "$temporary" "$commit"; then
    rm -f -- "$temporary"
    return 1
  fi
  if ! ln -- "$temporary" "$destination"; then
    rm -f -- "$temporary"
    return 1
  fi
  rm -f -- "$temporary"
  [ "$(stat -c '%u:%a' -- "$destination")" = "$(id -u):600" ] || return 1
  release_image_manifest_load "$destination" "$commit" || return 1
  for service in "${services[@]}"; do
    index=$((index + 1))
    expected=$(release_image_manifest_ref_for "$service" "$commit") || return 1
    image_verification=$(inspect_benchmark_image "$expected" "$commit") || return 1
    IFS=$'\t' read -r verify_ms image_id image_revision image_size <<<"$image_verification"
    [ "$image_id" = "${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}" ] &&
      [ "$image_revision" = "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}" ] || return 1
    revalidate_source_benchmark_archive "$revision" "$measurement_phase" "$service" "$index" "$commit" "$image_id" ||
      return 1
    [ "$(awk -F '\t' -v revision="$revision" -v phase="$measurement_phase" -v service="$service" -v ordinal="$index" 'NR > 1 && $1 == revision && $2 == phase && $3 == service && $4 == ordinal { count++ } END { print count + 0 }' "$results_report")" -eq 1 ] || return 1
    [ "$(awk -F '\t' -v revision="$revision" -v phase="$measurement_phase" -v service="$service" -v ordinal="$index" 'NR > 1 && $1 == revision && $2 == phase && $3 == service && $4 == ordinal { count++ } END { print count + 0 }' "$cargo_units_report")" -eq 1 ] || return 1
    [ "$(awk -F '\t' -v revision="$revision" -v phase="$measurement_phase" -v service="$service" -v ordinal="$index" 'NR > 1 && $1 == revision && $2 == phase && $3 == service && $4 == ordinal { count++ } END { print count + 0 }' "$native_identity_report")" -eq 1 ] || return 1
  done
}

run_common_c_release() (
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 namespace=$5
  local helper lock_prefix state_root image_override image_prefix local_library local_patch local_manifest
  local overall_started interval_started interval_finished service bundle bundle_digest recipe override
  local raw_log evidence_rel evidence_log image_ref image_verification verify_ms image_id image_revision image_size
  local total_ms cargo_cache status index=0 batch=0

  assert_common_checkout_binding "$revision" "$checkout" "$commit" ||
    die "common-C clean source/helper/layout binding changed before $revision/$measurement_phase"
  helper=${side_layout_helper[$revision]}
  lock_prefix=${RBL_LOCK_PREFIX:-/tmp/lagrange-production-image-build}
  # Every C release gets a fresh process and freshly sourced checkout helper.
  # The parent owns FD 9 for the full benchmark; the helper verifies and reuses
  # that inherited descriptor before it creates any private state.
  unset RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT RELEASE_BUILD_LAYOUT_COMMIT
  unset RELEASE_BUILD_LAYOUT_STATE_ROOT RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE
  unset RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RELEASE_BUILD_LAYOUT_HELPER_SHA256
  unset RELEASE_BUILD_LAYOUT_CONFIG_SHA256
  # shellcheck disable=SC1090
  source "$helper"
  RBL_LOCK_PREFIX=$lock_prefix
  if [ "$internal_self_test" -eq 1 ] && declare -F benchmark_install_common_fake_api >/dev/null; then
    benchmark_install_common_fake_api "$helper"
  fi
  common_gate_environment || die 'common-C release gate environment is invalid'
  # The state root itself must be absent.  Its immediate parent is the private
  # mktemp directory, so the benchmark never pre-creates state to mask helper
  # fresh-output behavior.  init itself verifies the inherited parent lock
  # before any state mutation, exactly as the frozen public API requires.
  state_root=$tmp_dir/common-state-${revision}-${measurement_phase}-${commit}
  [ ! -e "$state_root" ] && [ ! -L "$state_root" ] ||
    die "common-C state root already exists: $revision/$measurement_phase"
  common_state_root_by_run["$revision:$measurement_phase"]=$state_root
  common_private_directory "$tmp_dir/common-compose-overrides" || die 'common-C Compose override directory is unsafe'
  common_private_directory "$tmp_dir/common-manifest-libraries" || die 'common-C manifest library directory is unsafe'
  image_prefix=$(common_image_prefix "$revision" "$measurement_phase")
  image_override=$tmp_dir/common-compose-overrides/${revision}-${measurement_phase}-images.json
  local_library=$tmp_dir/common-manifest-libraries/${revision}-${measurement_phase}-release-image-manifest.sh
  local_patch=$tmp_dir/common-manifest-libraries/${revision}-${measurement_phase}-release-image-manifest.patch
  local_manifest=$output_dir/common-manifests/${revision}-${measurement_phase}.v2
  overall_started=$(now_ms)
  interval_started=$overall_started
  release_build_layout_init "$checkout" "$commit" "$state_root" "$namespace" ||
    die "common-C helper initialization failed: $revision/$measurement_phase"
  release_build_layout_gate run-start 0 || die "common-C initial strict gate failed: $revision/$measurement_phase"
  interval_finished=$(now_ms)
  record_common_phase "$revision" "$measurement_phase" init-and-run-gate "$interval_started" "$interval_finished" PASS \
    'clean-checkout-init-with-inherited-whole-lock'
  write_common_image_override "$image_override" "$image_prefix" "$commit" ||
    die "common-C private twelve-image override could not be written: $revision/$measurement_phase"
  write_benchmark_manifest_library "$checkout/scripts/ops/lib/release-image-manifest.sh" "$local_library" "$local_patch" "$image_prefix" ||
    die "common-C private manifest library could not be written: $revision/$measurement_phase"
  [ "$(stat -c '%u:%a' -- "$image_override" "$local_library" "$local_patch" | sort -u)" = "$(id -u):600" ] ||
    die 'common-C private override or manifest evidence mode is unsafe'
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "${local_manifest#$output_dir/}" \
    "${side_manifest_library_hash[$revision]}" "$(sha256_file "$local_library")" \
    "$(sha256_file "$local_patch")" "$commit" "$image_prefix" >>"$common_manifest_report"
  # The instrumented library copy is intentionally the only manifest code used
  # for this C release.  It differs from the production library in exactly its
  # image repository prefix, before any observed image identity is recorded.
  # shellcheck disable=SC1090
  source "$local_library"
  common_manifest_service_order || die 'common-C private manifest service order is not the frozen twelve-service order'
  release_image_manifest_reset

  for service in "${services[@]}"; do
    if [ $((index % 3)) -eq 0 ]; then
      batch=$((batch + 1))
      gate_batch "$revision" "$measurement_phase" "$batch" before
    fi
    index=$((index + 1))
    interval_started=$(now_ms)
    record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "prepare-$service" before-prepare ||
      die "common-C resource sampling was unavailable before producer preparation: $service"
    record_disk_snapshot "$revision" "$measurement_phase" "prepare-$service" before-prepare - ||
      die "common-C disk evidence was unavailable before producer preparation: $service"
    start_resource_sampler "$revision" "$measurement_phase" "prepare-$service" ||
      die "common-C resource sampler could not start for producer preparation: $service"
    if ! bundle=$(release_build_layout_prepare "$service" "$commit" "$state_root"); then
      stop_resource_sampler || true
      record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "prepare-$service" after-prepare-failure || true
      failure_stage=common-prepare
      die "common-C artifact preparation failed: $revision/$measurement_phase/$service"
    fi
    stop_resource_sampler || die "common-C resource sampler failed during producer preparation: $service"
    record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "prepare-$service" after-prepare ||
      die "common-C resource sampling was unavailable after producer preparation: $service"
    record_disk_snapshot "$revision" "$measurement_phase" "prepare-$service" after-prepare - ||
      die "common-C disk evidence was unavailable after producer preparation: $service"
    interval_finished=$(now_ms)
    record_common_phase "$revision" "$measurement_phase" "prepare-$service" "$interval_started" "$interval_finished" PASS \
      'native-input-guard-producer-export-or-explicit-none'
    override=
    case "$bundle" in
      NONE)
        case "$service" in db-role-bootstrap|db-migrate|web) ;; *) die "common-C helper returned NONE for Rust service: $service" ;; esac
        ;;
      *)
        recipe=$(common_bundle_recipe "$state_root" "$bundle") || die "common-C helper returned an invalid bundle path: $service"
        bundle_digest=$(release_build_layout_verify_bundle "$service" "$commit" "$bundle") ||
          die "common-C bundle verification failed: $service"
        [[ "$bundle_digest" =~ ^[0-9a-f]{64}$ ]] || die "common-C bundle digest is malformed: $service"
        if [ -n "${common_bundle_digest_by_recipe[$recipe]:-}" ] &&
            [ "${common_bundle_digest_by_recipe[$recipe]}" != "$bundle_digest" ]; then
          die "common-C shared producer bundle changed during one release: $recipe"
        fi
        common_bundle_digest_by_recipe["$recipe"]=$bundle_digest
        override=$state_root/overrides/$service.json
        release_build_layout_write_override "$service" "$bundle" "$override" ||
          die "common-C artifact override creation failed: $service"
        [ "$(stat -c '%u:%a' -- "$override")" = "$(id -u):600" ] ||
          die "common-C artifact override mode is unsafe: $service"
        ;;
    esac
    image_ref="${image_prefix}-${service}:${commit}"
    safe_benchmark_repository "${image_ref%%:*}" || die "common-C image reference is unsafe: $service"
    raw_log=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.common-compose.log.raw
    evidence_rel="evidence/${revision}-${measurement_phase}-${index}-${service}.common-compose.log"
    evidence_log=$output_dir/$evidence_rel
    interval_started=$(now_ms)
    record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" before-build ||
      die "common-C resource sampling was unavailable before consumer build: $service"
    record_disk_snapshot "$revision" "$measurement_phase" "$service" before-build - ||
      die "common-C disk evidence was unavailable before consumer build: $service"
    start_resource_sampler "$revision" "$measurement_phase" "$service" ||
      die "common-C resource sampler could not start for consumer build: $service"
    if common_compose_build "$checkout" "$commit" "$image_override" "$override" "$service" "$benchmark_compose_env_file" >"$raw_log" 2>&1; then
      last_build_exit=0
    else
      status=$?
      last_build_exit=$status
      stop_resource_sampler || true
      record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" after-build-failure || true
      sanitize_text_file "$raw_log" "$evidence_log"
      failure_stage=build
      die "common-C Compose build failed for $revision/$measurement_phase/$service (exit=$status; sanitized evidence: $evidence_rel)"
    fi
    stop_resource_sampler || die "common-C resource sampler failed during consumer build: $service"
    record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" after-build ||
      die "common-C resource sampling was unavailable after consumer build: $service"
    sanitize_text_file "$raw_log" "$evidence_log"
    image_verification=$(inspect_benchmark_image "$image_ref" "$commit") || {
      failure_stage=image-verification
      die "common-C temporary image verification failed: $service"
    }
    IFS=$'\t' read -r verify_ms image_id image_revision image_size <<<"$image_verification"
    release_build_layout_verify_image "$service" "$commit" "$image_id" "$state_root" || {
      failure_stage=image-archive-verification
      die "common-C strict saved-image verification failed: $service"
    }
    # The benchmark-local V2 record is populated only after the exact local
    # image ID has passed the helper's saved-image/request verification.  Its
    # private image ref comes from the complete twelve-image override, not the
    # production tag space.
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=$image_ref
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=$image_id
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$image_revision
    record_common_archive_result "$revision" "$measurement_phase" "$service" "$index" "$state_root" "$image_id" ||
      die "common-C image verification record is malformed: $service"
    record_disk_snapshot "$revision" "$measurement_phase" "$service" after-image-save "$image_size" ||
      die "common-C disk evidence was unavailable after strict image verification: $service"
    interval_finished=$(now_ms)
    total_ms=$((interval_finished - interval_started))
    is_decimal "$total_ms" || die 'common-C consumer elapsed time is malformed'
    if [ "$bundle" = NONE ]; then
      BENCH_COMPILER_VERTEX=not-applicable
      BENCH_COMPILER_VERTEX_STATE=not-applicable
      BENCH_COMPILER_CACHE=not-applicable
      BENCH_NONCOMPILER_CACHED=not-applicable
      BENCH_WORKSPACE_CLEAN=not-applicable
      BENCH_COMPILED_PACKAGES=
      BENCH_FRESH_PACKAGES=
      cargo_cache=not-applicable
    else
      BENCH_COMPILER_VERTEX=common-producer-receipt
      BENCH_COMPILER_VERTEX_STATE=shared-receipt
      BENCH_COMPILER_CACHE=receipt-bound
      BENCH_NONCOMPILER_CACHED=not-applicable
      BENCH_WORKSPACE_CLEAN=producer-guard-validated
      BENCH_COMPILED_PACKAGES=recorded-in-common-producers
      BENCH_FRESH_PACKAGES=recorded-in-common-producers
      cargo_cache=common-target-namespace
    fi
    append_result "$revision" "$measurement_phase" "$service" "$index" "$image_ref" "$namespace" "$commit" \
      "${source_identity_by_revision_scenario[$revision:$measurement_phase]}" "$total_ms" - \
      "$verify_ms" "$image_id" "$image_revision" "$cargo_cache" "$evidence_rel" "$scenario"
    record_common_phase "$revision" "$measurement_phase" "consumer-$service" "$interval_started" "$interval_finished" PASS \
      'one-service-compose-artifact-override-inspect-and-strict-bytes'
    printf 'BENCHMARK_SERVICE PASS revision=%s measurement_phase=%s selected_scenario=%s service=%s image_build_ms=%s cargo_ms=- image_verification_ms=%s compiler_vertex=%s compiler_state=%s cargo_cache=%s\n' \
      "$revision" "$measurement_phase" "$scenario" "$service" "$total_ms" "$verify_ms" "$BENCH_COMPILER_VERTEX" \
      "$BENCH_COMPILER_VERTEX_STATE" "$cargo_cache"
    if [ $((index % 3)) -eq 0 ] || [ "$index" -eq "${#services[@]}" ]; then
      interval_started=$(now_ms)
      release_build_layout_gate "batch:B$batch" 0 || die "common-C strict batch gate failed: B$batch"
      interval_finished=$(now_ms)
      record_common_phase "$revision" "$measurement_phase" "batch-gate-B$batch" "$interval_started" "$interval_finished" PASS \
        'helper-strict-health-resource-and-journal-gate'
      gate_batch "$revision" "$measurement_phase" "$batch" after
    fi
  done
  interval_started=$(now_ms)
  record_common_producer_evidence "$revision" "$measurement_phase" "$checkout" "$state_root"
  interval_finished=$(now_ms)
  record_common_phase "$revision" "$measurement_phase" producer-receipts "$interval_started" "$interval_finished" PASS \
    'seven-recipes-seventeen-current-receipts-and-cargo-units'
  interval_started=$(now_ms)
  release_build_layout_gate final-bytes 0 || die "common-C final strict gate failed: $revision/$measurement_phase"
  publish_common_manifest_and_revalidate "$local_library" "$local_manifest" "$commit" "$image_prefix" "$state_root" || {
    failure_stage=common-v2-publication
    die "common-C benchmark-local strict V2 publication/revalidation failed: $revision/$measurement_phase"
  }
  interval_finished=$(now_ms)
  record_common_phase "$revision" "$measurement_phase" final-bytes-v2 "$interval_started" "$interval_finished" PASS \
    'final-helper-gate-private-v2-no-clobber-publication-and-revalidation'
  total_ms=$((interval_finished - overall_started))
  is_decimal "$total_ms" || die 'common-C release elapsed time is malformed'
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "${side_product_kind[$revision]}" common "$overall_started" "$interval_finished" "$total_ms" PASS \
    'whole-release-includes-init-native-producers-export-twelve-consumers-strict-bytes-and-private-v2' \
    >>"$release_total_report"
)

run_source_release() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 namespace=$5 cold_nonce=$6
  local image_prefix local_library local_patch local_manifest service index=0 batch=0
  local overall_started interval_started interval_finished total_ms
  image_prefix=$(common_image_prefix "$revision" "$measurement_phase")
  source_manifest_active=0

  # The scheduling-only two-service self-test is deliberately not a release
  # claim. Every public apply and the full warm/cold self-test routes below use
  # the canonical twelve and therefore must publish strict V2 evidence.
  if ! canonical_benchmark_service_order; then
    [ "$internal_self_test" -eq 1 ] || die 'source benchmark service order is not the canonical release order'
    for service in "${services[@]}"; do
      if [ $((index % 3)) -eq 0 ]; then
        batch=$((batch + 1))
        gate_batch "$revision" "$measurement_phase" "$batch" before
      fi
      index=$((index + 1))
      build_one_service "$revision" "$measurement_phase" "$checkout" "$commit" "$service" "$index" "$namespace" "$cold_nonce" "$image_prefix"
      if [ $((index % 3)) -eq 0 ] || [ "$index" -eq "${#services[@]}" ]; then
        gate_batch "$revision" "$measurement_phase" "$batch" after
      fi
    done
    return 0
  fi

  overall_started=$(now_ms)
  interval_started=$overall_started
  common_private_directory "$tmp_dir/source-manifest-libraries" ||
    die 'source benchmark manifest library directory is unsafe'
  local_library=$tmp_dir/source-manifest-libraries/${revision}-${measurement_phase}-release-image-manifest.sh
  local_patch=$tmp_dir/source-manifest-libraries/${revision}-${measurement_phase}-release-image-manifest.patch
  local_manifest=$output_dir/source-manifests/${revision}-${measurement_phase}.v2
  write_benchmark_manifest_library "$manifest_library" "$local_library" "$local_patch" "$image_prefix" ||
    die "source benchmark private manifest library could not be written: $revision/$measurement_phase"
  [ "$(stat -c '%u:%a' -- "$local_library" "$local_patch" | sort -u)" = "$(id -u):600" ] ||
    die 'source benchmark private manifest evidence mode is unsafe'
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "${side_product_kind[$revision]}" source \
    "${local_manifest#$output_dir/}" "$(sha256_file "$manifest_library")" \
    "$(sha256_file "$local_library")" "$(sha256_file "$local_patch")" "$commit" "$image_prefix" \
    >>"$source_manifest_report"
  # shellcheck disable=SC1090
  source "$local_library"
  common_manifest_service_order || die 'source benchmark private manifest service order is not canonical'
  release_image_manifest_reset
  source_manifest_active=1
  interval_finished=$(now_ms)
  record_common_phase "$revision" "$measurement_phase" source-manifest-setup "$interval_started" "$interval_finished" PASS \
    'current-frozen-library-prefix-only-instrumentation'

  for service in "${services[@]}"; do
    if [ $((index % 3)) -eq 0 ]; then
      batch=$((batch + 1))
      interval_started=$(now_ms)
      gate_batch "$revision" "$measurement_phase" "$batch" before
      interval_finished=$(now_ms)
      record_common_phase "$revision" "$measurement_phase" "source-batch-before-B$batch" "$interval_started" "$interval_finished" PASS \
        'strict-resource-health-and-journal-gate'
    fi
    index=$((index + 1))
    interval_started=$(now_ms)
    build_one_service "$revision" "$measurement_phase" "$checkout" "$commit" "$service" "$index" "$namespace" "$cold_nonce" "$image_prefix"
    interval_finished=$(now_ms)
    record_common_phase "$revision" "$measurement_phase" "source-consumer-$service" "$interval_started" "$interval_finished" PASS \
      'instrumented-source-build-cargo-evidence-image-save-and-strict-bytes'
    if [ $((index % 3)) -eq 0 ] || [ "$index" -eq "${#services[@]}" ]; then
      interval_started=$(now_ms)
      gate_batch "$revision" "$measurement_phase" "$batch" after
      interval_finished=$(now_ms)
      record_common_phase "$revision" "$measurement_phase" "source-batch-after-B$batch" "$interval_started" "$interval_finished" PASS \
        'strict-resource-health-and-journal-gate'
    fi
  done
  interval_started=$(now_ms)
  gate_batch "$revision" "$measurement_phase" final before-v2
  publish_source_manifest_and_revalidate "$local_library" "$local_manifest" "$commit" "$image_prefix" "$revision" "$measurement_phase" || {
    failure_stage=source-v2-publication
    die "source benchmark strict V2 publication/revalidation failed: $revision/$measurement_phase"
  }
  source_manifest_active=0
  interval_finished=$(now_ms)
  record_common_phase "$revision" "$measurement_phase" source-final-bytes-v2 "$interval_started" "$interval_finished" PASS \
    'final-gate-current-source-cargo-and-archive-revalidation-private-v2-no-clobber-publication'
  total_ms=$((interval_finished - overall_started))
  is_decimal "$total_ms" || die 'source benchmark release elapsed time is malformed'
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$measurement_phase" "${side_product_kind[$revision]}" source \
    "$overall_started" "$interval_finished" "$total_ms" PASS \
    'whole-release-includes-native-source-builds-cargo-evidence-twelve-image-saves-strict-bytes-and-private-v2' \
    >>"$release_total_report"
}

safe_unit_name() {
  # A unit name is passed to systemctl.  Require a non-option first byte as
  # well as the accepted service-name grammar.
  printf '%s' "$1" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9@_.:-]*\.service$'
}

safe_container_reference() {
  # Docker container names and full IDs both fit this grammar.  It excludes a
  # leading dash, separators, whitespace, and every control character.
  [[ ${1:-} =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]]
}

read_systemd_value() {
  local unit=$1 property=$2 value manager=${3:-${BENCHMARK_SYSTEMD_MANAGER:-system}}
  safe_unit_name "$unit" || return 1
  case "$manager" in
    system) value=$(systemctl show -p "$property" --value -- "$unit" 2>/dev/null | head -n 1) || return 1 ;;
    user) value=$(systemctl --user show -p "$property" --value -- "$unit" 2>/dev/null | head -n 1) || return 1 ;;
    *) return 1 ;;
  esac
  [ -n "$value" ] || return 1
  printf '%s' "$value"
}

current_cgroup_text() {
  if [ "$internal_self_test" -eq 1 ]; then
    printf '%s\n' "${BENCH_TEST_CGROUP_TEXT:-}"
  else
    cat "/proc/$$/cgroup"
  fi
}

gate_build_service_state=unavailable
gate_health_state=unavailable
gate_compiler_state=unavailable
gate_mem_available=unavailable
gate_swap_free=unavailable
gate_oom_events=unavailable
gate_reason=

cgroup_path_matches_service() {
  local expected=$1 observed=$2
  case "$observed" in
    "$expected"|"$expected"/*) return 0 ;;
    *) return 1 ;;
  esac
}

cgroup_text_contains_service() {
  local cgroups=$1 expected=$2 entry observed
  while IFS= read -r entry; do
    # cgroup v1/v2 rows end with their cgroup path after the final colon.
    observed=${entry##*:}
    cgroup_path_matches_service "$expected" "$observed" && return 0
  done <<<"$cgroups"
  return 1
}

check_running_build_service() {
  local unit=${BENCHMARK_SYSTEMD_SERVICE:-} manager=${BENCHMARK_SYSTEMD_MANAGER:-system} active substate main_pid exit_status nice io_class io_priority control_group cgroups
  gate_build_service_state=fail
  if [ -z "$unit" ]; then
    gate_reason=missing-benchmark-systemd-service
    return 1
  fi
  if ! safe_unit_name "$unit"; then
    gate_reason=invalid-benchmark-systemd-service
    return 1
  fi
  case "$manager" in system|user) ;; *) gate_reason=invalid-benchmark-systemd-manager; return 1 ;; esac
  if ! command -v systemctl >/dev/null 2>&1; then
    gate_reason=systemctl-unavailable
    return 1
  fi
  active=$(read_systemd_value "$unit" ActiveState "$manager") || { gate_reason=build-service-unreadable; return 1; }
  substate=$(read_systemd_value "$unit" SubState "$manager") || { gate_reason=build-service-unreadable; return 1; }
  main_pid=$(read_systemd_value "$unit" MainPID "$manager") || { gate_reason=build-service-unreadable; return 1; }
  exit_status=$(read_systemd_value "$unit" ExecMainStatus "$manager") || { gate_reason=build-service-unreadable; return 1; }
  nice=$(read_systemd_value "$unit" Nice "$manager") || { gate_reason=build-service-unreadable; return 1; }
  io_class=$(read_systemd_value "$unit" IOSchedulingClass "$manager") || { gate_reason=build-service-unreadable; return 1; }
  io_priority=$(read_systemd_value "$unit" IOSchedulingPriority "$manager") || { gate_reason=build-service-unreadable; return 1; }
  control_group=$(read_systemd_value "$unit" ControlGroup "$manager") || { gate_reason=build-service-unreadable; return 1; }
  [ "$active" = active ] && [ "$substate" = running ] || { gate_reason=build-service-not-running; return 1; }
  is_decimal "$main_pid" && [ "$main_pid" -gt 0 ] || { gate_reason=build-service-has-no-main-pid; return 1; }
  [ "$exit_status" = 0 ] || { gate_reason=build-service-previous-exit-nonzero; return 1; }
  is_decimal "$nice" && [ "$nice" -ge 1 ] && [ "$nice" -le 19 ] || { gate_reason=build-service-cpu-priority-not-lowered; return 1; }
  case "$io_class" in
    3|idle) ;;
    2|best-effort)
      is_decimal "$io_priority" && [ "$io_priority" -ge 5 ] && [ "$io_priority" -le 7 ] || {
        gate_reason=build-service-io-priority-not-lowered
        return 1
      }
      ;;
    *) gate_reason=build-service-io-priority-not-lowered; return 1 ;;
  esac
  case "$control_group" in /*) ;; *) gate_reason=build-service-cgroup-unreadable; return 1 ;; esac
  cgroups=$(current_cgroup_text 2>/dev/null) || { gate_reason=build-service-cgroup-unreadable; return 1; }
  cgroup_text_contains_service "$cgroups" "$control_group" || {
    gate_reason=apply-not-running-inside-build-service
    return 1
  }
  gate_build_service_state=verified
}

check_production_health() {
  [ "$#" -eq 2 ] || {
    gate_health_state=fail
    gate_reason=shared-release-layout-gate-failed
    return 1
  }
  run_benchmark_layout_gate "$1" "$2"
}

check_no_active_compilers() {
  local processes active
  gate_compiler_state=unavailable
  if ! processes=$(ps -eo comm= 2>/dev/null); then
    gate_reason=compiler-process-observation-unavailable
    return 1
  fi
  active=$(printf '%s\n' "$processes" | awk '
    /^(cargo|rustc|sccache|cc1|cc1plus|collect2|ld|clang|clang\+\+|gcc|g\+\+|docker|buildx|buildctl)$/ { found=1 }
    END { if (found) print "present"; else print "idle" }
  ')
  gate_compiler_state=$active
  [ "$active" = idle ] || { gate_reason=prior-compiler-process-still-active; return 1; }
}

read_meminfo() {
  if [ "$internal_self_test" -eq 1 ]; then
    printf '%s\n' "${BENCH_TEST_MEMINFO:-}"
  else
    cat /proc/meminfo
  fi
}

reset_journal_observation() {
  journal_until_utc=unavailable
  journal_probe_status=unavailable
  journal_probe_entry_count=unavailable
  journal_probe_stdout_sha256=unavailable
  journal_probe_stderr_sha256=unavailable
  journal_lookback_status=unavailable
  journal_lookback_entry_count=unavailable
  journal_lookback_oom_count=unavailable
  journal_lookback_stdout_sha256=unavailable
  journal_lookback_stderr_sha256=unavailable
  journal_capture_status=unavailable
  journal_capture_sha256=unavailable
  journal_capture_failure=unavailable
  journal_observation_reason=unavailable
}

reset_journal_run_state() {
  journal_since_utc=
  journal_first_gate_utc=
  journal_established_boot_id=
  reset_journal_observation
}

normalize_boot_id() {
  local raw=${1:-} normalized
  normalized=${raw//-/}
  printf '%s' "$normalized" | grep -Eq '^[0-9A-Fa-f]{32}$' || return 1
  printf '%s' "${normalized,,}"
}

read_current_boot_id() {
  if [ "$internal_self_test" -eq 1 ]; then
    [ "${BENCH_TEST_BOOT_ID_MODE:-readable}" = readable ] || return 1
    printf '%s\n' "${BENCH_TEST_BOOT_ID:-}"
  else
    cat /proc/sys/kernel/random/boot_id
  fi
}

journal_gate_utc() {
  local value
  if [ "$internal_self_test" -eq 1 ]; then
    value=${BENCH_TEST_JOURNAL_GATE_UTC:-2030-01-01T00:00:00Z}
  else
    value=$(date -u '+%Y-%m-%dT%H:%M:%SZ') || return 1
  fi
  [[ "$value" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || return 1
  printf '%s' "$value"
}

journal_lookback_start_utc() {
  local until_utc=$1 value
  [[ "$until_utc" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || return 1
  if [ "$internal_self_test" -eq 1 ]; then
    value=${BENCH_TEST_JOURNAL_SINCE_UTC:-2029-12-31T23:30:00Z}
  else
    value=$(date -u -d "$until_utc - ${journal_lookback_seconds} seconds" '+%Y-%m-%dT%H:%M:%SZ') || return 1
  fi
  [[ "$value" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || return 1
  printf '%s' "$value"
}

cleanup_kernel_journal_tmp() {
  local path=${1:-}
  case "$path" in
    /tmp/lagrange-build-cache-journal.*) rm -rf -- "$path" ;;
    *) return 1 ;;
  esac
}

journal_parser_reason() {
  # The JSON parser intentionally reports only fixed codes.  Never place a
  # journal MESSAGE or Python exception text in public evidence or diagnostics.
  case "${1:-}" in
    $'error\tinvalid-utf8') printf '%s' invalid-utf8 ;;
    $'error\tinvalid-json') printf '%s' invalid-json ;;
    $'error\tjson-not-object') printf '%s' json-not-object ;;
    $'error\tblank-json-line') printf '%s' blank-json-line ;;
    $'error\tmissing-required-field') printf '%s' missing-required-field ;;
    $'error\tinvalid-timestamp') printf '%s' invalid-timestamp ;;
    $'error\tinvalid-cursor') printf '%s' invalid-cursor ;;
    $'error\tinvalid-boot-id') printf '%s' invalid-boot-id ;;
    $'error\twrong-boot-id') printf '%s' wrong-boot-id ;;
    $'error\tnot-kernel-entry') printf '%s' not-kernel-entry ;;
    $'error\tinvalid-message') printf '%s' invalid-message ;;
    $'error\tprobe-entry-count') printf '%s' probe-entry-count ;;
    $'error\tinvalid-query-range') printf '%s' invalid-query-range ;;
    $'error\tout-of-range-timestamp') printf '%s' out-of-range-timestamp ;;
    *) printf '%s' parser-failed ;;
  esac
}

journal_json_summary() {
  local raw_file=$1 expected_boot_id=$2 parser_stderr=$3
  command -v python3 >/dev/null 2>&1 || return 127
  python3 - "$raw_file" "$expected_boot_id" 2>"$parser_stderr" <<'PY'
import json
import os
import re
import sys

raw_path, expected_boot = sys.argv[1:]

def fail(code):
    print(f"error\t{code}")
    raise SystemExit(1)

try:
    if os.path.getsize(raw_path) >= 1024 * 1024:
        fail("probe-byte-limit")
    raw = open(raw_path, "rb").read()
except OSError:
    fail("raw-unreadable")

try:
    text = raw.decode("utf-8")
except UnicodeDecodeError:
    fail("invalid-utf8")

if not re.fullmatch(r"[0-9a-f]{32}", expected_boot):
    fail("invalid-boot-id")
lines = text.splitlines()
if not lines:
    fail("probe-entry-count")
if any(not line.strip() for line in lines):
    fail("blank-json-line")
if len(lines) != 1:
    fail("probe-entry-count")

for line in lines:
    try:
        entry = json.loads(line)
    except (TypeError, ValueError, json.JSONDecodeError):
        fail("invalid-json")
    if not isinstance(entry, dict):
        fail("json-not-object")
    timestamp = entry.get("__REALTIME_TIMESTAMP")
    cursor = entry.get("__CURSOR")
    boot_id = entry.get("_BOOT_ID")
    transport = entry.get("_TRANSPORT")
    message = entry.get("MESSAGE")
    if timestamp is None or cursor is None or boot_id is None or transport is None or message is None:
        fail("missing-required-field")
    if not isinstance(timestamp, str) or not re.fullmatch(r"[0-9]+", timestamp) or int(timestamp) <= 0:
        fail("invalid-timestamp")
    if not isinstance(cursor, str) or not cursor:
        fail("invalid-cursor")
    if not isinstance(boot_id, str) or not re.fullmatch(r"[0-9A-Fa-f-]+", boot_id):
        fail("invalid-boot-id")
    normalized_boot = boot_id.replace("-", "").lower()
    if not re.fullmatch(r"[0-9a-f]{32}", normalized_boot):
        fail("invalid-boot-id")
    if normalized_boot != expected_boot:
        fail("wrong-boot-id")
    if transport != "kernel":
        fail("not-kernel-entry")
    if not isinstance(message, str):
        fail("invalid-message")
print("ok\t1")
PY
}

# The G1 tail-limited journal read was a known incomplete proof. This collector
# streams the complete fixed window to private files, bounds every resource,
# waits for EOF and child reaping, and records only hashes/count metadata for
# public evidence. It is deliberately not a generic journal command wrapper.
journal_collect_complete_range() {
  local journal_bin=$1 output=$2 errors=$3 receipt=$4 since_utc=$5 until_utc=$6
  LC_ALL=C python3 - "$journal_bin" "$output" "$errors" "$receipt" "$since_utc" "$until_utc" <<'PY'
import hashlib
import json
import os
import selectors
import subprocess
import sys
import time

command, output, errors, receipt, since, until = sys.argv[1:]
argv = [command, "-k", "-b", "--no-pager", "-o", "json", "--since", since, "--until", until, "--no-tail"]
limits = {"deadline_ms": 10000, "stdout_bytes": 64 * 1024 * 1024,
          "stderr_bytes": 64 * 1024, "line_bytes": 1024 * 1024, "records": 100000}
started = time.monotonic_ns()
deadline = started + limits["deadline_ms"] * 1_000_000
result = {"format": "lagrange-journal-complete-capture-v1", "argv": argv, "lc_all": "C",
          "limits": limits, "bounds_exclusive": True, "complete": False,
          "stdout_eof": False, "stderr_eof": False, "command_exit": None,
          "child_reaped": False, "stop_requested": False, "failure": None,
          "stdout_bytes": 0, "stderr_bytes": 0, "line_count": 0,
          "record_count": 0, "max_line_bytes": 0, "partial_final_line_bytes": 0}
hashes = {name: hashlib.sha256() for name in ("stdout", "stderr")}
child = None
files = {}
selector = selectors.DefaultSelector()
line_bytes = 0
line_nonblank = False

class CaptureFailure(Exception):
    pass

def remaining():
    return max(0, (deadline - time.monotonic_ns()) / 1_000_000_000)

def check_deadline():
    if time.monotonic_ns() >= deadline:
        raise CaptureFailure("deadline")

def count_lines(data):
    global line_bytes, line_nonblank
    parts = data.split(b"\n")
    for index, part in enumerate(parts):
        ended = index < len(parts) - 1
        line_bytes += len(part) + int(ended)
        line_nonblank = line_nonblank or bool(part.strip())
        result["max_line_bytes"] = max(result["max_line_bytes"], line_bytes)
        result["partial_final_line_bytes"] = line_bytes
        if line_bytes >= limits["line_bytes"]:
            raise CaptureFailure("line-byte-limit")
        if ended:
            result["line_count"] += 1
            result["record_count"] += int(line_nonblank)
            if result["record_count"] >= limits["records"]:
                raise CaptureFailure("record-limit")
            line_bytes = 0
            line_nonblank = False
            result["partial_final_line_bytes"] = 0

try:
    files = {"stdout": open(output, "wb", buffering=0), "stderr": open(errors, "wb", buffering=0)}
    child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, env={**os.environ, "LC_ALL": "C"})
    for name, stream in (("stdout", child.stdout), ("stderr", child.stderr)):
        os.set_blocking(stream.fileno(), False)
        selector.register(stream, selectors.EVENT_READ, name)
    while selector.get_map():
        check_deadline()
        events = selector.select(remaining())
        check_deadline()
        for key, unused in events:
            name = key.data
            allowance = limits[name + "_bytes"] - result[name + "_bytes"]
            data = os.read(key.fd, min(32768, allowance))
            if not data:
                result[name + "_eof"] = True
                selector.unregister(key.fileobj)
                key.fileobj.close()
                continue
            files[name].write(data)
            hashes[name].update(data)
            result[name + "_bytes"] += len(data)
            if result[name + "_bytes"] >= limits[name + "_bytes"]:
                raise CaptureFailure(name + "-byte-limit")
            if name == "stdout":
                count_lines(data)
            check_deadline()
    child.wait(timeout=remaining())
    check_deadline()
    if child.returncode != 0:
        raise CaptureFailure("command-nonzero")
    if result["stderr_bytes"]:
        raise CaptureFailure("stderr-nonempty")
    if line_bytes:
        raise CaptureFailure("partial-final-record")
    result["complete"] = True
except CaptureFailure as error:
    result["failure"] = str(error)
except subprocess.TimeoutExpired:
    result["failure"] = "deadline"
except (OSError, ValueError):
    result["failure"] = "capture-io-or-spawn"
finally:
    if child is not None:
        if child.poll() is None:
            result["stop_requested"] = True
            try:
                child.kill()
                if remaining():
                    child.wait(timeout=remaining())
                else:
                    time.sleep(0)
            except (OSError, subprocess.TimeoutExpired):
                pass
        for stream in (child.stdout, child.stderr):
            if stream is not None:
                stream.close()
    selector.close()
    for handle in files.values():
        handle.close()
    if child is not None:
        result["command_exit"] = child.poll()
        result["child_reaped"] = child.returncode is not None
    result["elapsed_ns"] = time.monotonic_ns() - started
    for name in hashes:
        result[name + "_sha256"] = hashes[name].hexdigest()
    result["hash_scope"] = "complete-streams" if result["stdout_eof"] and result["stderr_eof"] else "captured-prefixes"
    if not result["child_reaped"]:
        result["complete"] = False
        result["failure"] = result["failure"] or "termination-unproven"
    if result["elapsed_ns"] >= limits["deadline_ms"] * 1_000_000:
        result["complete"] = False
        result["failure"] = result["failure"] or "deadline"
    with open(receipt, "w", encoding="utf-8") as handle:
        handle.write(json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n")
sys.exit(0 if result["complete"] else 1)
PY
}

journal_capture_summary() {
  local receipt=$1
  python3 - "$receipt" <<'PY'
import json
import re
import sys

try:
    value = json.load(open(sys.argv[1], encoding="utf-8"))
    exact = {"format", "argv", "lc_all", "limits", "bounds_exclusive", "complete", "stdout_eof", "stderr_eof",
             "command_exit", "child_reaped", "stop_requested", "failure", "stdout_bytes", "stderr_bytes", "line_count",
             "record_count", "max_line_bytes", "partial_final_line_bytes", "elapsed_ns", "stdout_sha256", "stderr_sha256", "hash_scope"}
    if not isinstance(value, dict) or set(value) != exact:
        raise ValueError()
    if value["format"] != "lagrange-journal-complete-capture-v1" or value["lc_all"] != "C":
        raise ValueError()
    if not isinstance(value["argv"], list) or len(value["argv"]) != 11:
        raise ValueError()
    if not isinstance(value["argv"][0], str) or not value["argv"][0]:
        raise ValueError()
    if not all(isinstance(value["argv"][index], str) for index in (7, 9)):
        raise ValueError()
    if value["argv"][1:] != ["-k", "-b", "--no-pager", "-o", "json", "--since", value["argv"][7], "--until", value["argv"][9], "--no-tail"]:
        raise ValueError()
    if value["limits"] != {"deadline_ms": 10000, "stdout_bytes": 67108864, "stderr_bytes": 65536, "line_bytes": 1048576, "records": 100000}:
        raise ValueError()
    if not all(value[key] is True for key in ("bounds_exclusive", "complete", "stdout_eof", "stderr_eof", "child_reaped")):
        raise ValueError()
    if value["command_exit"] != 0 or value["failure"] is not None or value["stop_requested"] is not False:
        raise ValueError()
    if value["hash_scope"] != "complete-streams":
        raise ValueError()
    if not all(isinstance(value[key], int) and value[key] >= 0 for key in ("stdout_bytes", "stderr_bytes", "line_count", "record_count", "max_line_bytes", "partial_final_line_bytes", "elapsed_ns")):
        raise ValueError()
    if value["stdout_bytes"] >= 64 * 1024 * 1024 or value["stderr_bytes"] >= 64 * 1024:
        raise ValueError()
    if value["record_count"] >= 100000 or value["max_line_bytes"] >= 1024 * 1024:
        raise ValueError()
    if value["line_count"] < value["record_count"] or value["elapsed_ns"] >= 10_000_000_000:
        raise ValueError()
    if value["partial_final_line_bytes"] != 0 or value["stderr_bytes"] != 0:
        raise ValueError()
    if not all(isinstance(value[key], str) and re.fullmatch(r"[0-9a-f]{64}", value[key]) for key in ("stdout_sha256", "stderr_sha256")):
        raise ValueError()
    print("ok\t" + str(value["record_count"]))
except (OSError, ValueError, KeyError, TypeError, IndexError, json.JSONDecodeError):
    raise SystemExit(1)
PY
}

journal_range_summary() {
  local raw_file=$1 expected_boot_id=$2 since_utc=$3 until_utc=$4 parser_stderr=$5
  python3 - "$raw_file" "$expected_boot_id" "$since_utc" "$until_utc" 2>"$parser_stderr" <<'PY'
import calendar
import datetime
import json
import os
import re
import sys

path, expected_boot, since_text, until_text = sys.argv[1:]
def fail(code):
    print(f"error\t{code}")
    raise SystemExit(1)
def timestamp(text):
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", text):
        fail("invalid-query-range")
    try:
        return calendar.timegm(datetime.datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").timetuple()) * 1_000_000
    except ValueError:
        fail("invalid-query-range")
if not re.fullmatch(r"[0-9a-f]{32}", expected_boot):
    fail("invalid-boot-id")
since = timestamp(since_text)
until = timestamp(until_text)
if until < since:
    fail("invalid-query-range")
try:
    if os.path.getsize(path) >= 64 * 1024 * 1024:
        fail("range-byte-limit")
    count = 0
    oom = 0
    pattern = re.compile(r"out of memory|oom[-_ ]?kill|killed process", re.I)
    with open(path, "rb") as handle:
        while True:
            raw = handle.readline(1024 * 1024)
            if not raw:
                break
            if len(raw) >= 1024 * 1024:
                fail("range-line-limit")
            if not raw.endswith(b"\n"):
                fail("range-partial-final-record")
            if not raw.strip():
                fail("blank-json-line")
            try:
                value = json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                fail("invalid-json")
            required = ("__REALTIME_TIMESTAMP", "__CURSOR", "_BOOT_ID", "_TRANSPORT", "MESSAGE")
            if not isinstance(value, dict): fail("json-not-object")
            if any(key not in value for key in required): fail("missing-required-field")
            stamp = value["__REALTIME_TIMESTAMP"]
            cursor = value["__CURSOR"]
            boot = value["_BOOT_ID"]
            if not isinstance(stamp, str) or not re.fullmatch(r"[0-9]+", stamp) or int(stamp) <= 0: fail("invalid-timestamp")
            if not isinstance(cursor, str) or not cursor: fail("invalid-cursor")
            if not isinstance(boot, str) or not re.fullmatch(r"[0-9A-Fa-f-]+", boot): fail("invalid-boot-id")
            if boot.replace("-", "").lower() != expected_boot: fail("wrong-boot-id")
            if value["_TRANSPORT"] != "kernel": fail("not-kernel-entry")
            if not isinstance(value["MESSAGE"], str): fail("invalid-message")
            point = int(stamp)
            if point < since or point > until: fail("out-of-range-timestamp")
            count += 1
            if count >= 100000: fail("record-limit")
            if pattern.search(value["MESSAGE"]):
                oom += 1
    print(f"ok\t{count}\t{oom}")
except OSError:
    fail("raw-unreadable")
PY
}

observe_kernel_journal() {
  local journal_tmp prior_umask probe_stdout probe_stderr lookback_stdout lookback_stderr parser_stderr capture_receipt
  local raw_boot_id current_boot_id final_raw_boot_id final_boot_id gate_utc parser_summary parser_reason command_status
  local journal_bin capture_summary capture_status
  reset_journal_observation
  journal_bin=journalctl
  if [ "$internal_self_test" -eq 1 ] && [ -n "${BENCH_TEST_JOURNALCTL_BIN:-}" ]; then
    [ -x "$BENCH_TEST_JOURNALCTL_BIN" ] && [ ! -L "$BENCH_TEST_JOURNALCTL_BIN" ] || {
      journal_observation_reason=kernel-journal-unavailable
      return 1
    }
    journal_bin=$BENCH_TEST_JOURNALCTL_BIN
  else
    command -v journalctl >/dev/null 2>&1 || {
      journal_observation_reason=kernel-journal-unavailable
      return 1
    }
  fi
  command -v timeout >/dev/null 2>&1 || {
    journal_observation_reason=kernel-journal-timeout-unavailable
    return 1
  }
  raw_boot_id=$(read_current_boot_id) || {
    journal_observation_reason=kernel-journal-boot-id-unreadable
    return 1
  }
  current_boot_id=$(normalize_boot_id "$raw_boot_id") || {
    journal_observation_reason=kernel-journal-boot-id-invalid
    return 1
  }
  if [ -n "$journal_established_boot_id" ] && [ "$current_boot_id" != "$journal_established_boot_id" ]; then
    journal_observation_reason=kernel-journal-boot-changed
    return 1
  fi
  gate_utc=$(journal_gate_utc) || {
    journal_observation_reason=kernel-journal-gate-time-unavailable
    return 1
  }
  if [ -z "$journal_first_gate_utc" ]; then
    journal_first_gate_utc=$gate_utc
    journal_since_utc=$(journal_lookback_start_utc "$journal_first_gate_utc") || {
      journal_observation_reason=kernel-journal-lookback-time-unavailable
      return 1
    }
  fi
  journal_until_utc=$gate_utc
  journal_tmp=$(mktemp -d /tmp/lagrange-build-cache-journal.XXXXXXXXXX) || {
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  }
  chmod 0700 -- "$journal_tmp" || {
    cleanup_kernel_journal_tmp "$journal_tmp" || true
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  }
  probe_stdout=$journal_tmp/probe.stdout
  probe_stderr=$journal_tmp/probe.stderr
  lookback_stdout=$journal_tmp/lookback.stdout
  lookback_stderr=$journal_tmp/lookback.stderr
  parser_stderr=$journal_tmp/parser.stderr
  capture_receipt=$journal_tmp/range.capture.json
  prior_umask=$(umask)
  umask 077
  : >"$probe_stdout" && : >"$probe_stderr" && : >"$lookback_stdout" && : >"$lookback_stderr" && : >"$parser_stderr" && : >"$capture_receipt"
  umask "$prior_umask"
  if [ ! -f "$probe_stdout" ] || [ ! -f "$probe_stderr" ] || [ ! -f "$lookback_stdout" ] || [ ! -f "$lookback_stderr" ] || [ ! -f "$parser_stderr" ] || [ ! -f "$capture_receipt" ]; then
    cleanup_kernel_journal_tmp "$journal_tmp" || true
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  fi
  chmod 0600 -- "$probe_stdout" "$probe_stderr" "$lookback_stdout" "$lookback_stderr" "$parser_stderr" "$capture_receipt" || {
    cleanup_kernel_journal_tmp "$journal_tmp" || true
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  }

  if LC_ALL=C timeout 10s "$journal_bin" -k -b --no-pager -o json -n 1 >"$probe_stdout" 2>"$probe_stderr"; then
    journal_probe_status=exit-0
  else
    command_status=$?
    journal_probe_status=exit-$command_status
  fi
  journal_probe_stdout_sha256=$(sha256_file "$probe_stdout")
  journal_probe_stderr_sha256=$(sha256_file "$probe_stderr")
  if [ "$journal_probe_status" != exit-0 ]; then
    journal_observation_reason=kernel-journal-probe-nonzero
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ -s "$probe_stderr" ]; then
    journal_observation_reason=kernel-journal-probe-stderr
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ ! -s "$probe_stdout" ]; then
    journal_observation_reason=kernel-journal-probe-empty
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if parser_summary=$(journal_json_summary "$probe_stdout" "$current_boot_id" "$parser_stderr"); then
    :
  else
    parser_reason=$(journal_parser_reason "$parser_summary")
    journal_observation_reason=kernel-journal-probe-$parser_reason
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ -s "$parser_stderr" ]; then
    journal_observation_reason=kernel-journal-probe-parser-stderr
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [[ "$parser_summary" =~ ^ok$'\t'([0-9]+)$ ]]; then
    journal_probe_entry_count=${BASH_REMATCH[1]}
  else
    journal_observation_reason=kernel-journal-probe-parser-output-invalid
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  journal_established_boot_id=$current_boot_id

  if journal_collect_complete_range "$journal_bin" "$lookback_stdout" "$lookback_stderr" "$capture_receipt" "$journal_since_utc" "$journal_until_utc"; then
    journal_lookback_status=exit-0
    journal_capture_status=complete
  else
    command_status=$?
    journal_lookback_status=exit-$command_status
    journal_capture_status=failed
  fi
  journal_lookback_stdout_sha256=$(sha256_file "$lookback_stdout")
  journal_lookback_stderr_sha256=$(sha256_file "$lookback_stderr")
  journal_capture_sha256=$(sha256_file "$capture_receipt")
  if capture_summary=$(journal_capture_summary "$capture_receipt"); then
    journal_capture_failure=none
  else
    journal_capture_failure=receipt-invalid-or-incomplete
  fi
  if [ -s "$lookback_stderr" ]; then
    journal_observation_reason=kernel-journal-lookback-stderr
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ "$journal_lookback_status" != exit-0 ]; then
    journal_observation_reason=kernel-journal-lookback-nonzero
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  [ "$journal_capture_status" = complete ] && [ "$journal_capture_failure" = none ] || {
    journal_observation_reason=kernel-journal-lookback-capture-incomplete
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  }
  if parser_summary=$(journal_range_summary "$lookback_stdout" "$current_boot_id" "$journal_since_utc" "$journal_until_utc" "$parser_stderr"); then
    :
  else
    parser_reason=$(journal_parser_reason "$parser_summary")
    journal_observation_reason=kernel-journal-lookback-$parser_reason
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ -s "$parser_stderr" ]; then
    journal_observation_reason=kernel-journal-lookback-parser-stderr
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [[ "$parser_summary" =~ ^ok$'\t'([0-9]+)$'\t'([0-9]+)$ ]]; then
    journal_lookback_entry_count=${BASH_REMATCH[1]}
    journal_lookback_oom_count=${BASH_REMATCH[2]}
  else
    journal_observation_reason=kernel-journal-lookback-parser-output-invalid
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  final_raw_boot_id=$(read_current_boot_id) || {
    journal_observation_reason=kernel-journal-boot-id-unreadable
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  }
  final_boot_id=$(normalize_boot_id "$final_raw_boot_id") || {
    journal_observation_reason=kernel-journal-boot-id-invalid
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  }
  if [ "$final_boot_id" != "$journal_established_boot_id" ]; then
    journal_observation_reason=kernel-journal-boot-changed
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  gate_oom_events=$journal_lookback_oom_count
  journal_observation_reason=established
  cleanup_kernel_journal_tmp "$journal_tmp" || {
    journal_observation_reason=kernel-journal-private-temp-cleanup-failed
    return 1
  }
}

snapshot_resources() {
  local meminfo
  gate_mem_available=unavailable
  gate_swap_free=unavailable
  gate_oom_events=unavailable
  reset_journal_observation
  if ! meminfo=$(read_meminfo); then
    gate_reason=meminfo-unavailable
    return 1
  fi
  gate_mem_available=$(awk '/^MemAvailable:/ { print $2; exit }' <<<"$meminfo")
  gate_swap_free=$(awk '/^SwapFree:/ { print $2; exit }' <<<"$meminfo")
  is_decimal "$gate_mem_available" || { gate_reason=mem-available-unavailable; return 1; }
  is_decimal "$gate_swap_free" || { gate_reason=swap-free-unavailable; return 1; }
  observe_kernel_journal || { gate_reason=$journal_observation_reason; return 1; }
  is_decimal "$gate_oom_events" || { gate_reason=kernel-oom-observation-invalid; return 1; }
}

gate_batch() {
  local revision=$1 scenario=$2 batch=$3 point=$4 status=pass reason=- previous_exit gate_label
  gate_reason=
  case "$last_build_exit" in
    not-started) previous_exit=0 ;;
    ''|*[!0-9]*) previous_exit=1 ;;
    *) previous_exit=$last_build_exit ;;
  esac
  gate_label=benchmark:$revision:$scenario:$batch:$point
  # Every benchmark gate reaches the public helper. Resource-floor and
  # process checks below remain independent benchmark evidence, while the
  # shared helper owns exact health, exception, system-scope control, identity,
  # restart, journal, and OOM semantics in persistent benchmark-gate-state.
  check_production_health "$gate_label" "$previous_exit" || {
    status=fail
    reason=$gate_reason
  }
  gate_reason=
  if ! snapshot_resources; then
    if [ "$status" = pass ]; then
      status=fail
      reason=$gate_reason
    fi
  fi
  if [ "$status" = pass ] && [ "$gate_mem_available" -lt "$min_mem_available_kib" ]; then
    status=fail; reason=mem-available-below-threshold
  fi
  if [ "$status" = pass ] && [ "$gate_swap_free" -lt "$min_swap_free_kib" ]; then
    status=fail; reason=swap-free-below-threshold
  fi
  if [ "$status" = pass ] && [ "$gate_oom_events" -ne 0 ]; then
    status=fail; reason=recent-kernel-oom-observed
  fi
  if [ "$status" = pass ]; then
    check_no_active_compilers || { status=fail; reason=$gate_reason; }
  fi
  if [ "$status" = pass ]; then
    case "$last_build_exit" in not-started|0) ;; *) status=fail; reason=previous-build-exit-nonzero ;; esac
  fi
  if [ "$status" = pass ]; then
    check_running_build_service || { status=fail; reason=$gate_reason; }
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario" "$batch" "$point" "$gate_mem_available" "$gate_swap_free" \
    "$min_mem_available_kib" "$min_swap_free_kib" "$gate_oom_events" \
    "$journal_probe_status" "$journal_probe_entry_count" "$journal_probe_stdout_sha256" "$journal_probe_stderr_sha256" \
    "$journal_lookback_status" "$journal_lookback_entry_count" "$journal_lookback_oom_count" "$journal_lookback_stdout_sha256" "$journal_lookback_stderr_sha256" \
    "$journal_capture_status" "$journal_capture_sha256" "$journal_capture_failure" \
    "$journal_since_utc" "$journal_until_utc" "$journal_observation_reason" "$last_build_exit" \
    "$gate_compiler_state" "$gate_build_service_state" "$gate_health_state" "$status:$reason" >>"$resource_report"
  if [ "$status" != pass ]; then
    failure_stage=resource-gate
    case "$reason" in
      missing-benchmark-systemd-service|invalid-benchmark-systemd-service|build-service-not-running|apply-not-running-inside-build-service)
        die "resource/health gate failed before $revision/$scenario batch $batch ($point): $reason; launch separately in an operator-approved unit using the documented systemd-run --no-block shape with all BENCHMARK_* health --setenv values"
        ;;
      missing-production-health-units|missing-production-health-containers)
        die "resource/health gate failed before $revision/$scenario batch $batch ($point): $reason; set both BENCHMARK_PRODUCTION_HEALTH_UNITS and BENCHMARK_PRODUCTION_HEALTH_CONTAINERS to operator-approved current production health targets"
        ;;
      *) die "resource/health gate failed before $revision/$scenario batch $batch ($point): $reason" ;;
    esac
  fi
}

namespace_for_revision() {
  local revision=$1 commit=$2
  printf 'lagrange-benchmark-%s-%s-%s' "$benchmark_nonce" "$revision" "${commit:0:12}"
}

cold_nonce_for_revision() {
  local revision=$1
  case "$revision" in baseline|candidate) ;; *) die 'cold nonce revision is invalid' ;; esac
  printf 'lagrange-benchmark-cold-%s-%s' "$benchmark_nonce" "$revision"
}

run_scenario() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 namespace=$5 cold_nonce=$6
  local service image_prefix
  case "${side_layout_kind[$revision]:-source}" in
    common)
      # C never receives A/B's rewritten Dockerfiles or nonce instrumentation.
      # Its helper/config/producer come from this clean current checkout.
      # The release route is deliberately a fresh subshell so it can source a
      # clean helper for each synthetic current commit.  Do not discard that
      # subprocess's status: a failed producer, archive receipt, or V2
      # revalidation must stop the enclosing benchmark just as a source-layout
      # Docker failure does.
      # The release itself must run in a fresh subshell for helper init, so
      # register its deterministic private tags in this parent first.  That
      # keeps cleanup effective after success or any mid-release failure.
      image_prefix=$(common_image_prefix "$revision" "$measurement_phase")
      for service in "${services[@]}"; do
        tag_list+=("$image_prefix-$service:$commit")
      done
      run_common_c_release "$revision" "$measurement_phase" "$checkout" "$commit" "$namespace" || return 1
      return 0
      ;;
    source)
      run_source_release "$revision" "$measurement_phase" "$checkout" "$commit" "$namespace" "$cold_nonce" || return 1
      return 0
      ;;
    *) die "benchmark layout kind is invalid for $revision" ;;
  esac
}

write_warm_comparison_report() {
  local pair_index=$1 pair_sequence=$2 measurement_phase service baseline_fields candidate_fields
  local baseline_total candidate_total baseline_cargo candidate_cargo
  measurement_phase=measured-pair-$pair_index
  if [ ! -e "$comparison_report" ]; then
    printf 'pair_index\tpair_order\tservice\tbaseline_total_ms\tcandidate_total_ms\tdelta_total_ms\tbaseline_cargo_ms\tcandidate_cargo_ms\tdelta_cargo_ms\tbaseline_compiler_cache\tcandidate_compiler_cache\tbaseline_compiled\tcandidate_compiled\tbaseline_fresh\tcandidate_fresh\n' >"$comparison_report"
  fi
  for service in "${services[@]}"; do
    baseline_fields=$(awk -F '\t' -v service="$service" -v phase="$measurement_phase" '$1 == "baseline" && $2 == phase && $3 == service { print; exit }' "$results_report")
    candidate_fields=$(awk -F '\t' -v service="$service" -v phase="$measurement_phase" '$1 == "candidate" && $2 == phase && $3 == service { print; exit }' "$results_report")
    [ -n "$baseline_fields" ] && [ -n "$candidate_fields" ] ||
      die "warm comparison lacks a measured pair record: pair=$pair_index service=$service"
    baseline_total=$(cut -f9 <<<"$baseline_fields")
    candidate_total=$(cut -f9 <<<"$candidate_fields")
    baseline_cargo=$(cut -f10 <<<"$baseline_fields")
    candidate_cargo=$(cut -f10 <<<"$candidate_fields")
    is_decimal "$baseline_total" && is_decimal "$candidate_total" || die 'warm comparison total timing is malformed'
    {
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t' \
        "$pair_index" "$pair_sequence" "$service" "$baseline_total" "$candidate_total" "$((candidate_total - baseline_total))" \
        "$baseline_cargo" "$candidate_cargo"
      if is_decimal "$baseline_cargo" && is_decimal "$candidate_cargo"; then
        printf '%s\t' "$((candidate_cargo - baseline_cargo))"
      else
        printf '%s\t' '-'
      fi
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$(cut -f16 <<<"$baseline_fields")" "$(cut -f16 <<<"$candidate_fields")" \
        "$(cut -f19 <<<"$baseline_fields")" "$(cut -f19 <<<"$candidate_fields")" \
        "$(cut -f20 <<<"$baseline_fields")" "$(cut -f20 <<<"$candidate_fields")"
    } >>"$comparison_report"
  done
  chmod 0600 -- "$comparison_report"
}

write_release_comparison_report() {
  local pair_index=$1 pair_sequence=$2 measurement_phase baseline_fields candidate_fields
  local baseline_total candidate_total
  measurement_phase=measured-pair-$pair_index
  baseline_fields=$(awk -F '\t' -v phase="$measurement_phase" '
    NR > 1 && $1 == "baseline" && $2 == phase { count++; value=$0 }
    END {
      if (count == 1) print value
      else if (count > 1) exit 2
    }
  ' "$release_total_report") || die "release comparison found duplicate baseline totals: pair=$pair_index"
  candidate_fields=$(awk -F '\t' -v phase="$measurement_phase" '
    NR > 1 && $1 == "candidate" && $2 == phase { count++; value=$0 }
    END {
      if (count == 1) print value
      else if (count > 1) exit 2
    }
  ' "$release_total_report") || die "release comparison found duplicate candidate totals: pair=$pair_index"
  if [ -z "$baseline_fields" ] || [ -z "$candidate_fields" ]; then
    if [ "$internal_self_test" -eq 1 ] && ! canonical_benchmark_service_order &&
       [ -z "$baseline_fields" ] && [ -z "$candidate_fields" ]; then
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$pair_index" "$pair_sequence" "${side_product_kind[baseline]}" "${side_product_kind[candidate]}" \
        "${side_layout_kind[baseline]}" "${side_layout_kind[candidate]}" - - - \
        self-test-subset-not-a-release \
        'two-service scheduling-only fixture has no whole-release timing or V2 claim' \
        >>"$release_comparison_report"
      return 0
    fi
    die "release comparison lacks a whole-release total: pair=$pair_index"
  fi
  [ "$(cut -f3 <<<"$baseline_fields")" = "${side_product_kind[baseline]}" ] &&
    [ "$(cut -f3 <<<"$candidate_fields")" = "${side_product_kind[candidate]}" ] &&
    [ "$(cut -f4 <<<"$baseline_fields")" = "${side_layout_kind[baseline]}" ] &&
    [ "$(cut -f4 <<<"$candidate_fields")" = "${side_layout_kind[candidate]}" ] &&
    [ "$(cut -f8 <<<"$baseline_fields")" = PASS ] &&
    [ "$(cut -f8 <<<"$candidate_fields")" = PASS ] ||
      die "release comparison total identity or status is malformed: pair=$pair_index"
  baseline_total=$(cut -f7 <<<"$baseline_fields")
  candidate_total=$(cut -f7 <<<"$candidate_fields")
  is_decimal "$baseline_total" && is_decimal "$candidate_total" ||
    die "release comparison whole-release timing is malformed: pair=$pair_index"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$pair_index" "$pair_sequence" "${side_product_kind[baseline]}" "${side_product_kind[candidate]}" \
    "${side_layout_kind[baseline]}" "${side_layout_kind[candidate]}" \
    "$baseline_total" "$candidate_total" "$((candidate_total - baseline_total))" comparable \
    'whole-release elapsed includes preparation/producers-or-source-builds/all12/image-save/strict-bytes/final-private-V2' \
    >>"$release_comparison_report"
}

run_measured_pair() {
  local pair_index=$1 baseline_namespace=$2 candidate_namespace=$3 pair_sequence revision
  local checkout commit namespace cold_nonce measurement_phase phase_started phase_finished
  local -a pair_revisions=()
  is_decimal "$pair_index" && [ "$pair_index" -ge 1 ] && [ "$pair_index" -le "$repetitions" ] ||
    die 'measured pair index is invalid'
  active_pair_index=$pair_index
  measurement_phase=measured-pair-$pair_index
  pair_sequence=$(pair_order "$pair_index")
  IFS=',' read -r -a pair_revisions <<<"$pair_sequence"
  [ "${#pair_revisions[@]}" -eq 2 ] || die 'measured pair order is malformed'

  phase_started=$(now_ms)
  apply_equivalent_scenario "$pair_index"
  record_source_snapshot baseline "$measurement_phase" "$baseline_checkout" "$baseline_commit"
  record_source_snapshot candidate "$measurement_phase" "$candidate_checkout" "$candidate_commit"
  phase_finished=$(now_ms)
  record_phase scenario-input-preparation pair-$pair_index "$scenario" "$phase_started" "$phase_finished" PASS \
    "pair-indexed-equivalent-temporary-scenario-commits;order=$pair_sequence"

  for revision in "${pair_revisions[@]}"; do
    case "$revision" in
      baseline)
        checkout=$baseline_checkout
        commit=${scenario_commit_by_revision_pair[baseline:$pair_index]}
        namespace=$baseline_namespace
        cold_nonce=$(cold_nonce_for_revision baseline)
        ;;
      candidate)
        checkout=$candidate_checkout
        commit=${scenario_commit_by_revision_pair[candidate:$pair_index]}
        namespace=$candidate_namespace
        cold_nonce=$(cold_nonce_for_revision candidate)
        ;;
      *) die 'measured pair order contains an unknown revision' ;;
    esac
    [ -n "$commit" ] || die "measured pair commit is unavailable: $revision/$pair_index"
    phase_started=$(now_ms)
    run_scenario "$revision" "$measurement_phase" "$checkout" "$commit" "$namespace" "$cold_nonce" || return 1
    phase_finished=$(now_ms)
    record_phase measured-image-builds "$revision" "$scenario" "$phase_started" "$phase_finished" PASS \
      "pair=$pair_index;order=$pair_sequence;sequential-twelve-service-temporary-image-builds-with-per-image-inspection"
  done

  phase_started=$(now_ms)
  write_warm_comparison_report "$pair_index" "$pair_sequence"
  write_release_comparison_report "$pair_index" "$pair_sequence"
  phase_finished=$(now_ms)
  record_phase comparison-report pair-$pair_index "$scenario" "$phase_started" "$phase_finished" PASS \
    "pair=$pair_index;order=$pair_sequence;per-service-and-whole-release-baseline-candidate-deltas"
}

write_cold_comparison_notice() {
  printf 'comparison_status\treason\nnot-comparable\tcold uses separate fresh namespaces and stable earliest-native-layer nonces; no changed/warm comparison is claimed\n' >"$comparison_report"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    1 baseline,candidate "${side_product_kind[baseline]}" "${side_product_kind[candidate]}" \
    "${side_layout_kind[baseline]}" "${side_layout_kind[candidate]}" - - - not-comparable \
    'cold sides use independent fresh namespaces; no baseline/candidate delta is claimed' \
    >>"$release_comparison_report"
  chmod 0600 -- "$comparison_report"
}

cleanup_test_images() {
  local tag
  [ "${#tag_list[@]}" -gt 0 ] || return 0
  command -v docker >/dev/null 2>&1 || return 0
  for tag in "${tag_list[@]}"; do
    # Only tags generated from this invocation's untruncated mktemp suffix are
    # eligible.  No global prune or production tag/cache mutation is allowed.
    docker image rm --force "$tag" >/dev/null 2>&1 || true
  done
}

cleanup_temp_checkouts() {
  [ -n "$tmp_dir" ] || return 0
  [ -n "$benchmark_tmp_base" ] || return 0
  case "$tmp_dir" in
    "$benchmark_tmp_base"/lagrange-build-cache-benchmark.??????????) rm -rf -- "$tmp_dir" ;;
    *) return 0 ;;
  esac
  tmp_dir=
  benchmark_tmp_base=
}

cleanup_apply_run() {
  stop_resource_sampler || true
  cleanup_test_images
  cleanup_temp_checkouts
}

initialize_output() {
  native_identity_source_by_recipe=()
  native_identity_source_hash_by_recipe=()
  if [ -e "$output_dir" ]; then
    [ -d "$output_dir" ] && [ ! -L "$output_dir" ] || die 'output-dir is not a regular directory'
    [ -z "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ] ||
      die 'output-dir must be empty; refusing to overwrite benchmark evidence'
  else
    mkdir -m 0700 -- "$output_dir" || die 'could not create output-dir'
  fi
  chmod 0700 -- "$output_dir"
  evidence_dir=$output_dir/evidence
  mkdir -m 0700 -- "$evidence_dir" || die 'could not create evidence directory'
  mkdir -m 0700 -- "$output_dir/cargo-units" "$output_dir/native-identities" "$output_dir/archive-requests" "$output_dir/archive-results" \
    "$output_dir/inputs" \
    "$output_dir/common-manifests" "$output_dir/source-manifests" ||
    die 'could not create strict benchmark evidence directories'
  write_benchmark_compose_env "$output_dir/inputs/image-only-compose.env"
  metadata_report=$output_dir/metadata.tsv
  results_report=$output_dir/service-results.tsv
  resource_report=$output_dir/batch-resources.tsv
  resource_samples_report=$output_dir/resource-samples.tsv
  peak_resource_report=$output_dir/peak-resources.tsv
  phase_report=$output_dir/phase-timings.tsv
  comparison_report=$output_dir/comparison.tsv
  source_identity_report=$output_dir/source-identities.tsv
  instrumentation_report=$output_dir/instrumentation.tsv
  probe_spec_report=$output_dir/probe-spec.tsv
  cargo_units_report=$output_dir/cargo-units.tsv
  native_identity_report=$output_dir/native-identities.tsv
  archive_validation_report=$output_dir/archive-validations.tsv
  disk_report=$output_dir/disk-costs.tsv
  disk_summary_report=$output_dir/disk-summary.tsv
  common_layout_report=$output_dir/common-layouts.tsv
  common_phase_report=$output_dir/common-phases.tsv
  common_producer_report=$output_dir/common-producers.tsv
  common_manifest_report=$output_dir/common-manifests.tsv
  source_manifest_report=$output_dir/source-manifests.tsv
  release_total_report=$output_dir/release-totals.tsv
  release_comparison_report=$output_dir/release-comparison.tsv
  failure_report=$output_dir/failure.tsv
  apply_record_failures=1
  printf 'revision\tmeasurement_phase\tservice\tindex\timage_tag\tcache_mount_namespace\tbuild_commit\tmeasured_source_identity_sha256\timage_build_ms\tcargo_ms\timage_verification_ms\timage_id\timage_revision\tcompiler_vertex\tcompiler_vertex_state\tcompiler_cache\tnoncompiler_cached_vertices\tcargo_cache_state\tcompiled_packages\tfresh_packages\tworkspace_clean\ttoolchain_identity\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\tsanitized_evidence\tselected_scenario\n' >"$results_report"
  printf 'revision\tmeasurement_phase\tbatch\tgate\tmem_available_kib\tswap_free_kib\tminimum_mem_available_kib\tminimum_swap_free_kib\trecent_kernel_oom_events\tjournal_probe_status\tjournal_probe_entries\tjournal_probe_stdout_sha256\tjournal_probe_stderr_sha256\tjournal_lookback_status\tjournal_lookback_entries\tjournal_lookback_oom_matches\tjournal_lookback_stdout_sha256\tjournal_lookback_stderr_sha256\tjournal_capture_status\tjournal_capture_receipt_sha256\tjournal_capture_failure\tjournal_since_utc\tjournal_until_utc\tjournal_reason\tprevious_build_exit\tcompiler_processes\tbackground_build_service\tproduction_health\tstatus\n' >"$resource_report"
  printf 'revision\tmeasurement_phase\tservice\tpoint\ttimestamp_ms\tmem_available_kib\tswap_free_kib\trepository_disk_available_kib\tdocker_root_disk_available_kib\n' >"$resource_samples_report"
  printf 'scope\tsample_count\tminimum_mem_available_kib\tminimum_swap_free_kib\tminimum_repository_disk_available_kib\tminimum_docker_root_disk_available_kib\tinterpretation\n' >"$peak_resource_report"
  printf 'phase\trevision\tscenario\tstarted_ms\tfinished_ms\telapsed_ms\tstatus\tdetail\n' >"$phase_report"
  printf 'revision\tmeasurement_phase\tinput_commit\tcheckout_head\tcheckout_tree\tprobe_patch_sha256\tinstrumentation_manifest_sha256\tmeasured_source_identity_sha256\n' >"$source_identity_report"
  printf 'revision\tservice\tsource_dockerfile\tinstrumentation\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\ttoolchain_identity\n' >"$instrumentation_report"
  printf 'revision\tmeasurement_phase\tservice\tindex\tstructured_units\tstructured_units_sha256\traw_log_sha256\tunit_count\tcompiler_vertex\n' >"$cargo_units_report"
  printf 'revision\tmeasurement_phase\tservice\tindex\tnative_identity\tnative_identity_sha256\trustc_vv_sha256\tcargo_version_sha256\tapk_info_vv_sha256\tapk_package_line_count\n' >"$native_identity_report"
  printf 'revision\tmeasurement_phase\tservice\tindex\tarchive_result\tarchive_result_sha256\tarchive_sha256\trequest_sha256\tmanifest_digest\tconfig_digest\timage_id\n' >"$archive_validation_report"
  printf 'revision\tmeasurement_phase\tservice\tpoint\timage_size_bytes\tdocker_images_bytes\tdocker_build_cache_bytes\tdocker_total_bytes\n' >"$disk_report"
  printf 'revision\tmeasurement_phase\tservice\timage_size_bytes\timages_delta_bytes\tbuild_cache_delta_bytes\ttotal_delta_bytes\timages_peak_bytes\tbuild_cache_peak_bytes\ttotal_peak_bytes\n' >"$disk_summary_report"
  printf 'revision\tproduct_kind\tlayout_kind\thelper_sha256\tlayout_sha256\tartifact_dockerfile_sha256\tmanifest_library_sha256\n' >"$common_layout_report"
  printf 'revision\tmeasurement_phase\tinterval\tstarted_ms\tfinished_ms\telapsed_ms\tstatus\tdetail\n' >"$common_phase_report"
  printf 'revision\tmeasurement_phase\trecipe\tbin\treceipt\treceipt_sha256\tcargo_json\tcargo_json_sha256\tcargo_ms\tbundle\tbundle_sha256\n' >"$common_producer_report"
  printf 'revision\tmeasurement_phase\tmanifest\toriginal_library_sha256\tinstrumented_library_sha256\tpatch_sha256\tcommit\timage_prefix\n' >"$common_manifest_report"
  printf 'revision\tmeasurement_phase\tproduct_kind\tlayout_kind\tmanifest\toriginal_library_sha256\tinstrumented_library_sha256\tpatch_sha256\tcommit\timage_prefix\n' >"$source_manifest_report"
  printf 'revision\tmeasurement_phase\tproduct_kind\tlayout_kind\tstarted_ms\tfinished_ms\telapsed_ms\tstatus\tdetail\n' >"$release_total_report"
  printf 'pair_index\tpair_order\tbaseline_product_kind\tcandidate_product_kind\tbaseline_layout_kind\tcandidate_layout_kind\tbaseline_release_ms\tcandidate_release_ms\tdelta_release_ms\tstatus\tdetail\n' >"$release_comparison_report"
  chmod 0600 -- "$results_report" "$resource_report" "$resource_samples_report" \
    "$peak_resource_report" "$phase_report" "$source_identity_report" "$instrumentation_report" \
    "$cargo_units_report" "$native_identity_report" "$archive_validation_report" "$disk_report" "$disk_summary_report" \
    "$common_layout_report" "$common_phase_report" "$common_producer_report" "$common_manifest_report" \
    "$source_manifest_report" "$release_total_report" "$release_comparison_report"
}

configure_resource_thresholds() {
  local requested
  requested=${BENCHMARK_MIN_MEM_AVAILABLE_KIB:-$default_min_mem_available_kib}
  is_decimal "$requested" && [ "$requested" -ge "$default_min_mem_available_kib" ] ||
    die "BENCHMARK_MIN_MEM_AVAILABLE_KIB must be an integer at least $default_min_mem_available_kib"
  min_mem_available_kib=$requested
  requested=${BENCHMARK_MIN_SWAP_FREE_KIB:-$default_min_swap_free_kib}
  is_decimal "$requested" && [ "$requested" -ge "$default_min_swap_free_kib" ] ||
    die "BENCHMARK_MIN_SWAP_FREE_KIB must be an integer at least $default_min_swap_free_kib"
  min_swap_free_kib=$requested
}

collect_host_identity() {
  local value
  benchmark_script_hash=$(sha256_file "$script_dir/build-cache-benchmark.sh")
  value=$(bash --version 2>/dev/null | head -n 1) || die 'Bash version is unavailable for benchmark evidence'
  bash_identity=$(safe_scalar "$value")
  [ "$bash_identity" != unavailable ] || die 'Bash version identity is unavailable'
  value=$(git --version 2>/dev/null | head -n 1) || die 'Git version is unavailable for benchmark evidence'
  git_identity=$(safe_scalar "$value")
  [ "$git_identity" != unavailable ] || die 'Git version identity is unavailable'
  value=$(python3 --version 2>&1 | head -n 1) || die 'Python version is unavailable for kernel-journal JSON evidence'
  python_identity=$(safe_scalar "$value")
  [ "$python_identity" != unavailable ] || die 'Python version identity is unavailable for kernel-journal JSON evidence'
  value=$(uname -srm 2>/dev/null) || die 'host OS/platform identity is unavailable for benchmark evidence'
  host_platform_identity=$(safe_scalar "${value// /_}")
  [ "$host_platform_identity" != unavailable ] || die 'host OS/platform identity is unavailable'
}

parse_builder_identity() {
  # Buildx inspect has no --format option. Only the header identifies the
  # builder; Name fields below Nodes belong to individual nodes.
  awk '
    /^Nodes:[[:space:]]*$/ { exit }
    /^Name:[[:space:]]*/ { names++; name=$0; sub(/^Name:[[:space:]]*/, "", name) }
    /^Driver:[[:space:]]*/ { drivers++; driver=$0; sub(/^Driver:[[:space:]]*/, "", driver) }
    END {
      if (names != 1 || drivers != 1 ||
          name !~ /^[A-Za-z0-9][A-Za-z0-9_.-]*$/ ||
          driver !~ /^[A-Za-z0-9][A-Za-z0-9_.-]*$/) exit 1
      print name "|" driver
    }
  '
}

test_builder_identity_parser() {
  local value invalid
  value=$(printf '%s\n' 'Name:          default' 'Driver:        docker' \
    '' 'Nodes:' 'Name:          node-one' 'Name:          node-two' | parse_builder_identity) || return 1
  [ "$value" = 'default|docker' ] || return 1
  value=$(printf '%s\n' 'Name: fixture-builder' 'Driver: docker-container' | parse_builder_identity) || return 1
  [ "$value" = 'fixture-builder|docker-container' ] || return 1
  for invalid in '' $'Name: default' $'Name: default\nDriver:' \
    $'Name: default\nName: duplicate\nDriver: docker' \
    $'Name: default\nDriver: docker\nDriver: duplicate' \
    $'Name: invalid|name\nDriver: docker'; do
    if printf '%s\n' "$invalid" | parse_builder_identity >/dev/null; then return 1; fi
  done
}

collect_docker_identity() {
  local value os_type architecture kernel root extra
  command -v docker >/dev/null 2>&1 || die 'Docker is required for --apply; use --plan or --self-test without a daemon'
  if ! value=$(docker version --format '{{.Client.Version}}|{{.Server.Version}}' 2>/dev/null); then
    die 'Docker daemon is unavailable for --apply'
  fi
  docker_identity=$(safe_scalar "$value")
  [ "$docker_identity" != unavailable ] || die 'Docker version identity is unavailable'
  if ! value=$(docker buildx version 2>/dev/null | head -n 1); then
    die 'Docker Buildx is unavailable for --apply'
  fi
  buildx_identity=$(safe_scalar "$value")
  [ "$buildx_identity" != unavailable ] || die 'Docker Buildx identity is unavailable'
  if ! value=$(docker buildx inspect 2>/dev/null); then
    die 'Docker builder identity is unavailable for --apply'
  fi
  builder_identity=$(printf '%s\n' "$value" | parse_builder_identity) ||
    die 'Docker builder identity is malformed'
  [ "$builder_identity" != unavailable ] || die 'Docker builder identity is unavailable'
  if ! value=$(docker info --format '{{.OSType}}|{{.Architecture}}|{{.KernelVersion}}|{{.DockerRootDir}}' 2>/dev/null); then
    die 'Docker platform identity is unavailable for --apply'
  fi
  case "$value" in *$'\n'*|*$'\r'*|*[$'\001'-$'\037'$'\177']*) die 'Docker platform identity is malformed' ;; esac
  os_type= architecture= kernel= root= extra=
  IFS='|' read -r os_type architecture kernel root extra <<<"$value"
  [ -z "$extra" ] && [ -n "$os_type" ] && [ -n "$architecture" ] && [ -n "$kernel" ] && [ -n "$root" ] ||
    die 'Docker platform identity is incomplete'
  case "$root" in /*) ;; *) die 'Docker root directory is not absolute' ;; esac
  docker_root_dir=$root
  docker_platform_identity=$(safe_scalar "${os_type}|${architecture}|${kernel}")
  [ "$docker_platform_identity" != unavailable ] || die 'Docker platform identity is unavailable'
}

write_metadata() {
  local baseline_namespace=$1 candidate_namespace=$2 baseline_cold_nonce=$3 candidate_cold_nonce=$4 protocol=$5
  {
    printf '%s\n' BUILD_CACHE_BENCHMARK_V5
    printf 'cache_mode\t%s\n' "$cache_mode"
    printf 'repetitions\t%s\n' "$repetitions"
    printf 'measurement_order\t%s\n' "$measurement_order"
    printf 'pair_order_contract\tbaseline/candidate,candidate/baseline,baseline/candidate for alternating pairs 1..3; one-pair explicit baseline-first or candidate-first permitted\n'
    printf 'benchmark_scenario\t%s\n' "$scenario"
    printf 'scenario_input_path\t%s\n' "${scenario_input_path[$scenario]}"
    printf 'scenario_transformation\t%s\n' "${scenario_transform[$scenario]}"
    printf 'scenario_expected_scope\t%s\n' "${scenario_expected_scope[$scenario]}"
    printf 'protocol\t%s\n' "$protocol"
    printf 'baseline_commit\t%s\n' "$baseline_commit"
    printf 'candidate_commit\t%s\n' "$candidate_commit"
    printf 'baseline_product_kind\t%s\n' "${side_product_kind[baseline]}"
    printf 'candidate_product_kind\t%s\n' "${side_product_kind[candidate]}"
    printf 'baseline_layout_kind\t%s\n' "${side_layout_kind[baseline]}"
    printf 'candidate_layout_kind\t%s\n' "${side_layout_kind[candidate]}"
    printf 'baseline_cache_mount_namespace\t%s\n' "$baseline_namespace"
    printf 'candidate_cache_mount_namespace\t%s\n' "$candidate_namespace"
    printf 'baseline_stable_cold_nonce\t%s\n' "$baseline_cold_nonce"
    printf 'candidate_stable_cold_nonce\t%s\n' "$candidate_cold_nonce"
    printf 'cache_mount_argument\t--build-arg BUILDKIT_CACHE_MOUNT_NS=<namespace> on every Docker build\n'
    printf 'cold_layer_policy\tstable side nonce consumed before native apk; fresh namespace in cold mode; no repeated --no-cache\n'
    printf 'docker_identity\t%s\n' "$docker_identity"
    printf 'buildx_identity\t%s\n' "$buildx_identity"
    printf 'builder_identity\t%s\n' "$builder_identity"
    printf 'docker_platform_identity\t%s\n' "$docker_platform_identity"
    printf 'docker_root_dir\t%s\n' "$docker_root_dir"
    printf 'host_platform_identity\t%s\n' "$host_platform_identity"
    printf 'bash_identity\t%s\n' "$bash_identity"
    printf 'git_identity\t%s\n' "$git_identity"
    printf 'python_identity\t%s\n' "$python_identity"
    printf 'benchmark_script_sha256\t%s\n' "$benchmark_script_hash"
    printf 'image_only_compose_env\tinputs/image-only-compose.env\n'
    printf 'image_only_compose_env_sha256\t%s\n' "$benchmark_compose_env_sha256"
    printf 'image_only_compose_env_policy\tprivate-external-input; mode-0600; inactive-research-entitlement-sentinel-only; never copied from an operational environment\n'
    printf 'cargo_build_jobs\t2\n'
    printf 'service_order\t%s\n' "${services[*]}"
    printf 'batch_policy\tup-to-three-services; one Docker invocation at a time\n'
    printf 'resource_min_mem_available_kib\t%s\n' "$min_mem_available_kib"
    printf 'resource_min_swap_free_kib\t%s\n' "$min_swap_free_kib"
    printf 'kernel_journal_probe\tLC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1; require exit-0, no-stderr, exactly-one valid current-boot kernel entry\n'
    printf 'kernel_journal_lookback\tLC_ALL=C bounded complete-stream collector invokes journalctl -k -b --no-pager -o json --since <fixed-first-gate-minus-1800s> --until <gate-utc> --no-tail; require EOF on both streams, child reaping, exit-0, no-stderr, valid bounded current-boot JSON, and a complete capture receipt\n'
    printf 'kernel_journal_lookback_seconds\t%s\n' "$journal_lookback_seconds"
    printf 'kernel_journal_first_gate_utc\t%s\n' "${journal_first_gate_utc:-unavailable}"
    printf 'kernel_journal_since_utc\t%s\n' "${journal_since_utc:-unavailable}"
    printf 'kernel_journal_evidence_policy\tprivate-raw-stdout-stderr-and-complete-capture-receipt-removed-after-hash; public-status-count-range-hash-oom-and-fixed-capture-status-only; any-query-warning-error-incomplete-capture-malformed-entry-or-bound-exceedance-fails-closed\n'
    printf 'build_service_unit\t%s\n' "${BENCHMARK_SYSTEMD_SERVICE:-unconfigured}"
    printf 'build_service_manager\t%s\n' "${BENCHMARK_SYSTEMD_MANAGER:-system}"
    printf 'production_health_units\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_UNITS:-unconfigured}"
    printf 'production_health_containers\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-unconfigured}"
    printf 'research_exception\t%s\n' "${BENCHMARK_RESEARCH_EXCEPTION:-unconfigured}"
    printf 'benchmark_gate_source_root\tcoordinator-repository\n'
    printf 'benchmark_gate_source_commit\t%s\n' "${benchmark_gate_source_commit:-unavailable}"
    printf 'benchmark_gate_state_root\tbenchmark-gate-state\n'
    printf 'benchmark_gate_helper_sha256\t%s\n' "$(sha256_file "$benchmark_gate_helper")"
    printf 'benchmark_gate_policy\tshared-public-release-build-layout-init-and-gate; isolated-per-label-dispatch; persistent-gate-state; no-Docker-init\n'
    printf 'production_container_health_policy\trunning; declared healthy healthcheck; compose project lagrange-station; bounded inspect fields only\n'
    printf 'source_checkout_policy\ttemporary-detached-local-clones\n'
    printf 'probe_policy\tper-pair deterministic common temporary scenario transformation; exact revision-relative patches and hashes retained\n'
    printf 'commit_transition_policy\teach measured pair appends a pair-indexed marker to the previous measured tree and commits a deterministic synthetic child so the temporary image OCI revision and embedded code commit match its tree; commit-only is the transition control\n'
    printf 'probe_spec_sha256\t%s\n' "${probe_spec_hash:-unavailable}"
    printf 'instrumentation_policy\ttemporary A/B Dockerfile copies only; Cargo build/install verbosity plus a stable side nonce before native apk; C helper/config/producer stays clean, is sourced from its own checkout, and uses its namespace for cold isolation\n'
    printf 'instrumentation_manifest_sha256\t%s\n' "${instrumentation_manifest_hash:-unavailable}"
    printf 'evidence_policy\tsanitized BuildKit/Cargo logs retained; raw temporary logs removed with clones\n'
    printf 'image_identity_scope\ttemporary modified-source benchmark image; not an immutable official release image\n'
    printf 'timing_policy\timage_build_ms is one consumer Docker/Compose build through strict bytes; cargo_ms is only the observed compiler RUN duration; release-totals.tsv spans preparation/producers-or-source-builds/all12/image-save/strict-bytes/final-private-V2; warm-up remains separate\n'
    printf 'link_time_policy\tnot-separated; Cargo RUN duration is never reported as linker time\n'
    printf 'resource_sampling_policy\tminimum sampled MemAvailable/SwapFree/repository-disk/docker-root-disk during each Docker build; not process-RSS or exact used-memory peak\n'
    printf 'release_validation_scope\tsource A/B use current source-bound archive requests plus actual Cargo/native evidence; common C additionally binds producer receipts; every canonical release publishes and revalidates an exact private twelve-image V2 with no-clobber mode checks; these are benchmark documents, never official production manifests\n'
    printf 'helper_gate_environment\tBENCHMARK_SYSTEMD_SERVICE->RELEASE_BUILD_SYSTEMD_UNIT; BENCHMARK_SYSTEMD_MANAGER->RELEASE_BUILD_SYSTEMD_MANAGER; BENCHMARK_PRODUCTION_HEALTH_UNITS->RELEASE_BUILD_HEALTH_UNITS; BENCHMARK_PRODUCTION_HEALTH_CONTAINERS->RELEASE_BUILD_HEALTH_CONTAINERS; BENCHMARK_RESEARCH_EXCEPTION->RELEASE_BUILD_RESEARCH_EXCEPTION\n'
    printf 'lifecycle_policy\tno rollout, compose lifecycle, provider access, credential read, or global prune\n'
    # The former foreground sequential arrangement is prohibited; retain that
    # phrase for the repository's static compatibility check while stating the
    # actual policy unambiguously.
    printf 'foreground_sequential_policy\tforbidden; apply must run inside the verified background systemd service\n'
  } >"$metadata_report"
  chmod 0600 -- "$metadata_report"
}

create_temp_run_directory() {
  local requested_base raw_suffix base
  requested_base=${TMPDIR:-/tmp}
  case "$requested_base" in
    /*) ;;
    *) die 'TMPDIR must be an absolute path for benchmark temporary data' ;;
  esac
  [ -d "$requested_base" ] && [ ! -L "$requested_base" ] ||
    die 'TMPDIR must be an existing non-symlink directory for benchmark temporary data'
  benchmark_tmp_base=$(cd -- "$requested_base" && pwd -P) ||
    die 'TMPDIR could not be resolved to a canonical directory'
  [ "$benchmark_tmp_base" != / ] || die 'TMPDIR may not resolve to the filesystem root'
  tmp_dir=$(mktemp -d "$benchmark_tmp_base/lagrange-build-cache-benchmark.XXXXXXXXXX") ||
    die 'could not create temporary benchmark directory'
  base=${tmp_dir##*/}
  case "$base" in
    lagrange-build-cache-benchmark.??????????) ;;
    *) die 'temporary benchmark directory has an unexpected private name' ;;
  esac
  [ "$(stat -c '%u:%a' -- "$tmp_dir")" = "$(id -u):700" ] ||
    die 'temporary benchmark directory ownership or mode is unsafe'
  raw_suffix=${base#lagrange-build-cache-benchmark.}
  benchmark_nonce=$(encode_benchmark_nonce "$raw_suffix") || die 'temporary benchmark nonce could not be encoded safely'
  # Keep and encode the full random suffix.  Truncating the common mktemp
  # prefix makes concurrent invocations collide (the earlier lagrangebu
  # failure mode), while raw mixed case is invalid in a Docker repository.
  instrumentation_dir=$tmp_dir/instrumented-dockerfiles
  mkdir -m 0700 -- "$instrumentation_dir"
  baseline_checkout=$tmp_dir/baseline
  candidate_checkout=$tmp_dir/candidate
}

run_apply() {
  local baseline_namespace candidate_namespace baseline_cold_nonce candidate_cold_nonce protocol probe_line pair_index
  local overall_started prep_started prep_finished phase_started phase_finished
  # A self-test may invoke this function more than once in one shell.  Do not
  # let an earlier run's failure path write into a later non-empty output dir.
  overall_started=$(now_ms)
  prep_started=$overall_started
  apply_record_failures=0
  failure_report=
  failure_stage=preflight
  reset_journal_run_state
  initialize_output
  configure_resource_thresholds
  validate_service_contract
  validate_measurement_protocol
  create_temp_run_directory
  trap cleanup_apply_run EXIT
  common_gate_environment || die 'benchmark helper gate environment is invalid'
  failure_stage=layout-lock
  # Initialize the real public guard before the first benchmark gate.  The
  # coordinator checkout remains clean and the parent keeps its whole-run lock
  # while every isolated gate reuses the persistent benchmark state.
  initialize_benchmark_gate_guard || die 'could not initialize the benchmark-wide public layout guard'
  # Validate paths and all no-build prerequisites before any Docker build.  A
  # failed gate writes failure.tsv and leaves the empty/auditable output run.
  gate_batch preflight prerequisite 0 before
  collect_host_identity
  collect_docker_identity
  prepare_checkout "$baseline_commit" "$baseline_checkout"
  prepare_checkout "$candidate_commit" "$candidate_checkout"
  validate_checkout_contract "$baseline_checkout" baseline
  validate_checkout_contract "$candidate_checkout" candidate
  detect_checkout_layout baseline "$baseline_checkout"
  detect_checkout_layout candidate "$candidate_checkout"
  common_gate_environment || die 'benchmark helper gate environment is invalid'
  activate_benchmark_layout_helper
  probe_line=$(scenario_probe_line)
  printf 'scenario\tinput_path\ttransformation\texpected_scope\tprobe_line\n' >"$probe_spec_report"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$scenario" "${scenario_input_path[$scenario]}" "${scenario_transform[$scenario]}" \
    "${scenario_expected_scope[$scenario]}" "$probe_line" >>"$probe_spec_report"
  chmod 0600 -- "$probe_spec_report"
  probe_spec_hash=$(sha256_file "$probe_spec_report")
  record_layout_identity baseline "$baseline_checkout"
  record_layout_identity candidate "$candidate_checkout"
  instrumentation_manifest_hash=$(sha256_file "$instrumentation_report")
  baseline_namespace=$(namespace_for_revision baseline "$baseline_commit")
  candidate_namespace=$(namespace_for_revision candidate "$candidate_commit")
  baseline_cold_nonce=$(cold_nonce_for_revision baseline)
  candidate_cold_nonce=$(cold_nonce_for_revision candidate)
  prep_finished=$(now_ms)
  record_phase preparation all "$scenario" "$prep_started" "$prep_finished" PASS \
    'gates-identity-detached-checkouts-clean-layout-detection-and-A-B-only-temporary-instrumentation'

  if [ "$cache_mode" = warm ]; then
    protocol=warmup-unmodified-once-then-pair-indexed-common-scenario-commits
    record_source_snapshot baseline warmup "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate warmup "$candidate_checkout" "$candidate_commit"
    write_metadata "$baseline_namespace" "$candidate_namespace" "$baseline_cold_nonce" "$candidate_cold_nonce" "$protocol"
    printf 'BENCHMARK_RUN mode=warm baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    phase_started=$(now_ms)
    run_scenario baseline warmup "$baseline_checkout" "$baseline_commit" "$baseline_namespace" "$baseline_cold_nonce" || return 1
    phase_finished=$(now_ms)
    record_phase warmup-builds baseline "$scenario" "$phase_started" "$phase_finished" PASS \
      'unchanged-source-sequential-twelve-service-cache-warmup'
    phase_started=$(now_ms)
    run_scenario candidate warmup "$candidate_checkout" "$candidate_commit" "$candidate_namespace" "$candidate_cold_nonce" || return 1
    phase_finished=$(now_ms)
    record_phase warmup-builds candidate "$scenario" "$phase_started" "$phase_finished" PASS \
      'unchanged-source-sequential-twelve-service-cache-warmup'
    for ((pair_index = 1; pair_index <= repetitions; pair_index++)); do
      run_measured_pair "$pair_index" "$baseline_namespace" "$candidate_namespace"
    done
  else
    protocol=cold-independent-changed-source-no-warm-comparison
    phase_started=$(now_ms)
    active_pair_index=1
    apply_equivalent_scenario 1
    record_source_snapshot baseline measured-pair-1 "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate measured-pair-1 "$candidate_checkout" "$candidate_commit"
    phase_finished=$(now_ms)
    record_phase scenario-input-preparation all "$scenario" "$phase_started" "$phase_finished" PASS \
      'equivalent-temporary-scenario-commits-and-source-patch-identities'
    write_metadata "$baseline_namespace" "$candidate_namespace" "$baseline_cold_nonce" "$candidate_cold_nonce" "$protocol"
    printf 'BENCHMARK_RUN mode=cold baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    phase_started=$(now_ms)
    run_scenario baseline measured-pair-1 "$baseline_checkout" "${scenario_commit_by_revision_pair[baseline:1]}" "$baseline_namespace" "$baseline_cold_nonce" || return 1
    phase_finished=$(now_ms)
    record_phase measured-image-builds baseline "$scenario" "$phase_started" "$phase_finished" PASS \
      'sequential-twelve-service-temporary-image-builds-with-per-image-inspection'
    phase_started=$(now_ms)
    run_scenario candidate measured-pair-1 "$candidate_checkout" "${scenario_commit_by_revision_pair[candidate:1]}" "$candidate_namespace" "$candidate_cold_nonce" || return 1
    phase_finished=$(now_ms)
    record_phase measured-image-builds candidate "$scenario" "$phase_started" "$phase_finished" PASS \
      'sequential-twelve-service-temporary-image-builds-with-per-image-inspection'
    phase_started=$(now_ms)
    write_cold_comparison_notice
    phase_finished=$(now_ms)
    record_phase comparison-report all "$scenario" "$phase_started" "$phase_finished" PASS \
      'cold-not-comparable-notice; strict-release-manifest-not-measured'
  fi
  phase_started=$(now_ms)
  write_disk_summary
  phase_finished=$(now_ms)
  record_phase disk-cost-summary all "$scenario" "$phase_started" "$phase_finished" PASS \
    'per-service-before-build-and-after-image-save-image-and-build-cache-bytes'
  phase_started=$(now_ms)
  write_peak_resource_report
  phase_finished=$(now_ms)
  record_phase resource-summary all "$scenario" "$phase_started" "$phase_finished" PASS \
    'sampled-available-memory-swap-and-disk-minima'
  phase_finished=$(now_ms)
  record_phase benchmark-overall all "$scenario" "$overall_started" "$phase_finished" PASS \
    'preparation-warmup-or-cold-pair-indexed-measurement-comparison-disk-and-resource-summary; benchmark validation remains non-official'
  printf 'BUILD_CACHE_BENCHMARK_RESULT PASS output_dir=%s records=%s\n' \
    "$output_dir" "$(( $(wc -l <"$results_report") - 1 ))"
  printf '%s\n' 'BUILD_CACHE_BENCHMARK_APPLY_STATUS benchmark-only; no deployment or release validation was performed'
}

expect_parse_failure() {
  local log=$1 cargo_mode=$2 revision=$3 description=$4 units_path=${5:-}
  if parse_build_events "$log" "$cargo_mode" "$revision" "$units_path" >/dev/null 2>&1; then
    die "self-test parser accepted invalid case: $description"
  fi
  [ -n "$BENCH_PARSE_ERROR" ] || die "self-test parser failure lacked a reason: $description"
}

new_self_test_nonce() {
  local dir base suffix nonce
  dir=$(mktemp -d /tmp/lagrange-build-cache-benchmark.self-test-nonce.XXXXXXXXXX)
  base=${dir##*/}
  suffix=${base#lagrange-build-cache-benchmark.self-test-nonce.}
  nonce=$(encode_benchmark_nonce "$suffix") || die 'self-test nonce could not be encoded safely'
  rmdir -- "$dir"
  printf '%s' "$nonce"
}

run_self_test() {
  test_builder_identity_parser || die 'self-test builder identity parser failed'
  local test_dir parser_dir journalctl_bin baseline_log candidate_log cached_log absent_log malformed_log missing_finish_log failed_log invalid_json_log
  local native_test_dir native_dockerfile native_changed_dockerfile native_executed_log native_cached_log native_partial_log native_reordered_log native_source native_source_copy
  local current parent plan_output first_nonce second_nonce foreign_tag build_count remove_count
  local clean_clone clean_clone_input clean_clone_env real_git_bin
  local case_variant_one case_variant_two health_mode health_reason saved_health_containers scenario_case scenario_path
  local -a saved_services=()
  real_git_bin=$(command -v git) || die 'self-test could not resolve the real Git executable'
  test_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-build-cache-benchmark-self-test.XXXXXXXXXX")
  trap 'rm -rf -- "$test_dir"' RETURN
  # Exercise the real lock API without leaving fake-test state at its fixed
  # apply prefix. This assignment is internal to --self-test and never comes
  # from the caller environment.
  RBL_LOCK_PREFIX=$test_dir/whole-release-lock
  reset_self_test_whole_lock() {
    local label=$1
    # Each self-test apply is a separate fake benchmark invocation. Release
    # the prior test shell's inherited descriptor before changing its private
    # namespace; the real --apply path never calls this helper.
    # The close must persist in this test shell, but its error redirection must
    # not.  A bare `exec ... 2>/dev/null` changes the caller's stderr forever,
    # silently hiding every later self-test diagnostic.
    { exec 9>&-; } 2>/dev/null || true
    unset RELEASE_BUILD_LAYOUT_LOCK_HELD RELEASE_BUILD_LAYOUT_LOCK_DIR
    RBL_LOCK_PREFIX=$test_dir/whole-release-lock-$label
  }
  parser_dir=$test_dir/parser
  mkdir -p -- "$parser_dir"
  declare -A BENCH_TEST_IMAGE_COMMIT=()
  declare -A BENCH_TEST_IMAGE_ID=()
  declare -A BENCH_TEST_IMAGE_CONTEXT=()
  declare -A BENCH_TEST_IMAGE_TAG=()
  declare -A BENCH_TEST_IMAGE_ARCHIVE=()
  declare -A BENCH_TEST_IMAGE_REQUEST=()
  declare -A BENCH_TEST_IMAGE_SIZE=()

  # Parser contracts: baseline has no clean requirement; candidate does when
  # executed.  A generic CACHED COPY must not masquerade as a compiler hit.
  baseline_log=$parser_dir/baseline.log
  candidate_log=$parser_dir/candidate.log
  cached_log=$parser_dir/cached.log
  absent_log=$parser_dir/absent.log
  malformed_log=$parser_dir/malformed.log
  missing_finish_log=$parser_dir/missing-finish.log
  failed_log=$parser_dir/failed.log
  invalid_json_log=$parser_dir/invalid-json.log
  cat >"$baseline_log" <<'EOF'
#2 [builder 2/7] COPY Cargo.toml ./
#2 CACHED
#9 [builder 7/7] RUN cargo build -vv --message-format=json-render-diagnostics --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-baseline v0.1.0 (/build)
#9 0.21 {"reason":"compiler-artifact","package_id":"benchmark-baseline 0.1.0 (path+file:///build)","target":{"name":"benchmark-baseline","kind":["bin"],"crate_types":["bin"]},"profile":{"opt_level":"3","debuginfo":0,"debug_assertions":false,"overflow_checks":false,"test":false,"doc":false},"features":[],"fresh":false,"executable":"/cargo-target/release/benchmark-baseline"}
#9 0.22 {"reason":"build-finished","success":true}
#9 0.30 Finished `release` profile [optimized] target(s) in 2.25s
#9 DONE 2.25s
EOF
  cat >"$candidate_log" <<'EOF'
#9 [builder 7/7] RUN cargo clean --workspace --release --locked && cargo build -vv --message-format=json-render-diagnostics --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-candidate v0.1.0 (/build)
#9 0.21 {"reason":"compiler-artifact","package_id":"benchmark-candidate 0.1.0 (path+file:///build)","target":{"name":"benchmark-candidate","kind":["bin"],"crate_types":["bin"]},"profile":{"opt_level":"3","debuginfo":0,"debug_assertions":false,"overflow_checks":false,"test":false,"doc":false},"features":[],"fresh":false,"executable":"/cargo-target/release/benchmark-candidate"}
#9 0.22 {"reason":"build-finished","success":true}
#9 0.30 Finished `release` profile [optimized] target(s) in 1.25s
#9 DONE 1.25s
EOF
  cat >"$cached_log" <<'EOF'
#2 [builder 2/7] COPY Cargo.toml ./
#2 CACHED
#9 [builder 7/7] RUN cargo clean --workspace --release --locked && cargo build -vv --locked --release
#9 CACHED
EOF
  cat >"$absent_log" <<'EOF'
#2 [builder 2/2] COPY . .
#2 CACHED
EOF
  cat >"$malformed_log" <<'EOF'
#9 [builder 7/7] RUN cargo build -vv --locked --release
#9 0.30 Finished `release` profile [optimized] target(s) in 1.25s
#9 DONE not-a-duration
EOF
  cat >"$missing_finish_log" <<'EOF'
#9 [builder 7/7] RUN cargo build -vv --locked --release
#9 0.30 Compiling benchmark-missing-finish v0.1.0 (/build)
#9 DONE 1.25s
EOF
  cat >"$failed_log" <<'EOF'
#9 [builder 7/7] RUN cargo build -vv --locked --release
#9 ERROR: process "/bin/sh -c cargo build" did not complete successfully
EOF
  cat >"$invalid_json_log" <<'EOF'
#9 [builder 7/7] RUN cargo build -vv --message-format=json-render-diagnostics --locked --release
#9 0.10 Compiling benchmark-invalid-json v0.1.0 (/build)
#9 0.20 Finished `release` profile [optimized] target(s) in 1.25s
#9 DONE 1.25s
EOF
  parse_build_events "$baseline_log" 1 baseline "$parser_dir/baseline-units.json"
  [ "$BENCH_COMPILER_VERTEX_STATE" = executed ] || die 'self-test baseline compiler should execute'
  [ "$BENCH_COMPILER_CACHE" = miss ] || die 'self-test unrelated CACHED was counted as compiler hit'
  [ "$BENCH_WORKSPACE_CLEAN" = absent ] || die 'self-test baseline no-clean contract regressed'
  [ "$BENCH_CARGO_STEP_MS" = 2250 ] || die 'self-test baseline Cargo duration regressed'
  [ "$BENCH_CARGO_UNITS_COUNT" = 1 ] && [ -f "$parser_dir/baseline-units.json" ] ||
    die 'self-test baseline Cargo JSON unit record was not retained'
  parse_build_events "$candidate_log" 1 candidate "$parser_dir/candidate-units.json"
  [ "$BENCH_WORKSPACE_CLEAN" = observed ] || die 'self-test candidate clean contract regressed'
  [ "$BENCH_CARGO_STEP_MS" = 1250 ] || die 'self-test candidate Cargo duration regressed'
  parse_build_events "$cached_log" 1 candidate "$parser_dir/cached-units.json"
  [ "$BENCH_COMPILER_VERTEX_STATE" = cached ] || die 'self-test cached compiler vertex was not recognized'
  [ "$BENCH_COMPILER_CACHE" = hit ] || die 'self-test cached compiler cache state regressed'
  [ "$BENCH_CARGO_STEP_MS" = 0 ] || die 'self-test cached compiler did not use explicit zero time'
  [ "$BENCH_WORKSPACE_CLEAN" = not-executed ] || die 'self-test cached compiler claimed clean evidence'
  expect_parse_failure "$baseline_log" 1 candidate candidate-no-clean
  expect_parse_failure "$absent_log" 1 baseline absent-cargo-vertex
  expect_parse_failure "$malformed_log" 1 baseline malformed-duration
  expect_parse_failure "$missing_finish_log" 1 baseline missing-finish
  expect_parse_failure "$failed_log" 1 baseline compiler-error
  expect_parse_failure "$invalid_json_log" 1 baseline missing-cargo-json-units "$parser_dir/invalid-json-units.json"

  # Native identity cache evidence is reusable only from a directly executed
  # vertex with the same complete native recipe binding in this invocation.
  native_test_dir=$parser_dir/native-identity
  mkdir -p -- "$native_test_dir/native-identities"
  output_dir=$native_test_dir
  native_identity_report=$native_test_dir/native-identities.tsv
  printf 'revision\tmeasurement_phase\tservice\tindex\tnative_identity\tnative_identity_sha256\trustc_vv_sha256\tcargo_version_sha256\tapk_info_vv_sha256\tapk_package_line_count\n' >"$native_identity_report"
  builder_identity='fixture-builder|docker'
  buildx_identity='fixture-buildx'
  docker_identity='fixture-client|fixture-server'
  docker_platform_identity='linux|amd64|fixture-kernel'
  native_identity_source_by_recipe=()
  native_identity_source_hash_by_recipe=()
  native_dockerfile=$native_test_dir/native.Dockerfile
  native_changed_dockerfile=$native_test_dir/native-changed.Dockerfile
  cat >"$native_dockerfile" <<'EOF'
FROM rust:1.97.1-alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa AS builder
ARG TARGETPLATFORM
ENV CARGO_BUILD_JOBS=2
ARG LAGRANGE_CODE_COMMIT
RUN test -n "$LAGRANGE_CODE_COMMIT"
ARG LAGRANGE_BENCHMARK_COLD_NONCE
RUN test -n "$LAGRANGE_BENCHMARK_COLD_NONCE"
RUN apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev
RUN set -eu; \
    printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN; \
    printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_END; \
    printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_END; \
    printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_END; \
    printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END
RUN cargo build --locked --release
EOF
  sed 's/postgresql-dev/postgresql-dev git/' "$native_dockerfile" >"$native_changed_dockerfile"
  native_executed_log=$native_test_dir/executed.log
  native_cached_log=$native_test_dir/cached.log
  native_partial_log=$native_test_dir/partial.log
  native_reordered_log=$native_test_dir/reordered.log
  cat >"$native_executed_log" <<'EOF'
#11 [builder 4/5] RUN set -eu;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN;     printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END
#11 0.01 LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN
#11 0.02 LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN
#11 0.03 rustc 1.97.1 (fixture)
#11 0.04 host: x86_64-unknown-linux-musl
#11 0.05 release: 1.97.1
#11 0.06 LAGRANGE_BENCH_NATIVE_RUSTC_END
#11 0.07 LAGRANGE_BENCH_NATIVE_CARGO_BEGIN
#11 0.08 cargo 1.97.1 (fixture)
#11 0.09 LAGRANGE_BENCH_NATIVE_CARGO_END
#11 0.10 LAGRANGE_BENCH_NATIVE_APK_BEGIN
#11 0.11 build-base-0
#11 0.12 musl-dev-0
#11 0.13 LAGRANGE_BENCH_NATIVE_APK_END
#11 0.14 LAGRANGE_BENCH_NATIVE_IDENTITY_END
#11 DONE 0.2s
EOF
  cat >"$native_cached_log" <<'EOF'
#9 [builder 4/5] RUN set -eu;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN;     printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END
#9 CACHED
EOF
  cat >"$native_partial_log" <<'EOF'
#7 [builder 4/5] RUN set -eu;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN;     printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END
#7 0.01 LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN
#7 DONE 0.1s
EOF
  cat >"$native_reordered_log" <<'EOF'
#8 [builder 4/5] RUN set -eu;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN;     printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\n' LAGRANGE_BENCH_NATIVE_RUSTC_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\n' LAGRANGE_BENCH_NATIVE_CARGO_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\n' LAGRANGE_BENCH_NATIVE_APK_END;     printf '%s\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END
#8 0.01 LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN
#8 0.02 LAGRANGE_BENCH_NATIVE_CARGO_BEGIN
#8 DONE 0.1s
EOF
  record_native_identity baseline warmup first-native 1 "$native_executed_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ||
    die 'self-test rejected executed native identity evidence'
  native_source=$native_test_dir/native-identities/baseline-warmup-1-first-native.json
  record_native_identity baseline warmup cached-native 2 "$native_cached_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ||
    die 'self-test rejected same-recipe cached native identity evidence'
  python3 - "$native_test_dir/native-identities/baseline-warmup-2-cached-native.json" "$native_source" <<'PY' ||
import hashlib, json, sys
cached=json.load(open(sys.argv[1],encoding="utf-8")); source=open(sys.argv[2],"rb").read()
if cached.get("evidence_mode")!="cached-reuse" or cached.get("native_vertex_state")!="cached": raise SystemExit(1)
provenance=cached.get("source_evidence",{})
if provenance.get("sha256")!=hashlib.sha256(source).hexdigest() or provenance.get("native_vertex")!="11": raise SystemExit(1)
PY
    die 'self-test cached native identity provenance was malformed'
  if record_native_identity baseline warmup nonce-mismatch 3 "$native_cached_log" 1 "$native_dockerfile" namespace-a nonce-b aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test accepted cached native identity with a different cold nonce'
  fi
  if record_native_identity baseline warmup commit-mismatch 9 "$native_cached_log" 1 "$native_dockerfile" namespace-a nonce-a bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >/dev/null 2>&1; then
    die 'self-test accepted cached native identity with a different relevant build argument'
  fi
  if record_native_identity baseline warmup recipe-mismatch 4 "$native_cached_log" 1 "$native_changed_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test accepted cached native identity with different apk packages'
  fi
  builder_identity='different-builder|docker'
  if record_native_identity baseline warmup builder-mismatch 5 "$native_cached_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test accepted cached native identity from a different builder binding'
  fi
  builder_identity='fixture-builder|docker'
  native_source_copy=$native_test_dir/native-source-copy.json
  cp -- "$native_source" "$native_source_copy"
  printf '%s\n' modified >>"$native_source"
  if record_native_identity baseline warmup modified-source 6 "$native_cached_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test accepted modified prior native identity evidence'
  fi
  mv -- "$native_source_copy" "$native_source"
  if record_native_identity baseline warmup partial-executed 7 "$native_partial_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test reused old native evidence over partial executed markers'
  fi
  if record_native_identity baseline warmup reordered-executed 8 "$native_reordered_log" 1 "$native_dockerfile" namespace-a nonce-a aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa >/dev/null 2>&1; then
    die 'self-test reused old native evidence over reordered executed markers'
  fi
  [ "$(wc -l <"$native_identity_report")" -eq 3 ] ||
    die 'self-test native identity failures published unexpected report rows'

  # Preserve the public default-plan CLI without contacting Docker.
  current=$(git -c "safe.directory=$repo_root" -C "$repo_root" rev-parse HEAD)
  parent=$(git -c "safe.directory=$repo_root" -C "$repo_root" rev-parse HEAD^)
  plan_output=$(bash "$script_dir/build-cache-benchmark.sh" --baseline-commit "$parent" --candidate-commit "$current" --output-dir "$test_dir/plan-output")
  grep -Fq 'BUILD_CACHE_BENCHMARK_PLAN mode=plan' <<<"$plan_output" || die 'self-test default plan CLI regressed'

  # Hex encoding retains every source byte: case-only suffix variants must not
  # collide after their Docker-safe lowercasing transformation.
  case_variant_one=$(encode_benchmark_nonce 'AbCdEf1234') || die 'self-test could not encode mixed-case nonce'
  case_variant_two=$(encode_benchmark_nonce 'aBcDeF1234') || die 'self-test could not encode case variant nonce'
  [ "$case_variant_one" != "$case_variant_two" ] || die 'self-test case-only nonce variants collided'
  printf '%s\n%s\n' "$case_variant_one" "$case_variant_two" | grep -Eq '^[0-9a-f]+$' ||
    die 'self-test nonce encoding was not lowercase hex'
  safe_benchmark_repository "lagrange-cb-${case_variant_one}-baseline-measured-1" ||
    die 'self-test encoded nonce did not form a Docker-safe repository'

  internal_self_test=1
  BENCHMARK_SYSTEMD_SERVICE=benchmark.service
  BENCHMARK_PRODUCTION_HEALTH_UNITS=api.service,web.service
  BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=api-current,web-current
  BENCH_TEST_CGROUP_TEXT='0::/test.slice/benchmark.service'
  BENCH_TEST_MEMINFO=$'MemAvailable:    4194304 kB\nSwapFree:        2097152 kB'
  BENCH_TEST_JOURNAL_MODE=clean
  BENCH_TEST_BOOT_ID=11111111-1111-1111-1111-111111111111
  BENCH_TEST_BOOT_ID_MODE=readable
  BENCH_TEST_JOURNAL_GATE_UTC=2030-01-01T00:00:00Z
  BENCH_TEST_JOURNAL_SINCE_UTC=2029-12-31T23:30:00Z
  BENCH_TEST_PS_NAMES=$'systemd\nbash'
  BENCH_TEST_CONTAINER_MODE=healthy
  BENCH_TEST_DOCKER_RECORD=$test_dir/fake-docker.tsv
  BENCH_TEST_SYSTEMCTL_RECORD=$test_dir/fake-systemctl.tsv
  BENCH_TEST_JOURNAL_RECORD=$test_dir/fake-journal.tsv
  journalctl_bin=$test_dir/fake-journalctl
  BENCH_TEST_JOURNALCTL_BIN=$journalctl_bin
  export BENCH_TEST_JOURNAL_MODE BENCH_TEST_BOOT_ID BENCH_TEST_JOURNAL_GATE_UTC BENCH_TEST_JOURNAL_SINCE_UTC
  export BENCH_TEST_JOURNAL_RECORD BENCH_TEST_JOURNALCTL_BIN
  : >"$BENCH_TEST_DOCKER_RECORD"
  : >"$BENCH_TEST_SYSTEMCTL_RECORD"
  : >"$BENCH_TEST_JOURNAL_RECORD"

  write_fake_image_archive() {
    local archive=$1 request=$2 commit=$3 context=$4 tag=$5
    RBL_BENCH_ARCHIVE=$archive RBL_BENCH_REQUEST=$request \
      RBL_BENCH_COMMIT=$commit RBL_BENCH_CONTEXT=$context RBL_BENCH_TAG=$tag python3 - <<'PY'
import hashlib
import io
import json
import os
import stat
import struct
import tarfile

archive = os.environ["RBL_BENCH_ARCHIVE"]
request_path = os.environ["RBL_BENCH_REQUEST"]
commit = os.environ["RBL_BENCH_COMMIT"]
context = os.environ["RBL_BENCH_CONTEXT"]
tag = os.environ["RBL_BENCH_TAG"]
request = json.load(open(request_path, encoding="utf-8"))
if (request.get("format") != "lagrange-image-files-v1" or not isinstance(request.get("files"), list)
        or not isinstance(request.get("nonempty_directories"), list)):
    raise SystemExit("fixture-request-format-invalid")

source_by_hash = {}
for current, dirs, names in os.walk(context, topdown=True, followlinks=False):
    dirs.sort()
    names.sort()
    for name in names:
        path = os.path.join(current, name)
        info = os.lstat(path)
        if stat.S_ISREG(info.st_mode) and not stat.S_ISLNK(info.st_mode):
            data = open(path, "rb").read()
            source_by_hash.setdefault(hashlib.sha256(data).hexdigest(), data)

def binary(patterns):
    value = b"\x7fELF\x02\x01\x01" + (b"\0" * 9) + b"\x02\x00\x3e\x00\x01\x00\x00\x00"
    for pattern in patterns:
        value += b"\0" + bytes.fromhex(pattern)
    return value

entries = {}
directories = {""}
for item in request["files"]:
    path = item["path"]
    if item["sha256"] is None:
        data = binary(item["contains_hex"]) if item["elf"] else b"generated-benchmark-output\n"
    else:
        data = source_by_hash.get(item["sha256"])
        if data is None:
            raise SystemExit("fixture-source-bytes-missing")
    entries[path] = (data, 0o755 if item["executable"] else 0o644)
    parent = path.rsplit("/", 1)[0] if "/" in path else ""
    while parent:
        directories.add(parent)
        parent = parent.rsplit("/", 1)[0] if "/" in parent else ""
for path in request["nonempty_directories"]:
    directories.add(path)
    parent = path.rsplit("/", 1)[0] if "/" in path else ""
    while parent:
        directories.add(parent)
        parent = parent.rsplit("/", 1)[0] if "/" in parent else ""
    if not any(item.startswith(path + "/") for item in entries):
        entries[path + "/.benchmark-nonempty"] = (b"benchmark-directory\n", 0o644)

layer_stream = io.BytesIO()
with tarfile.open(fileobj=layer_stream, mode="w", format=tarfile.USTAR_FORMAT) as layer:
    for path in sorted(directories, key=lambda value: (value.count("/"), value)):
        if not path:
            continue
        item = tarfile.TarInfo(path)
        item.type = tarfile.DIRTYPE
        item.mode = 0o755
        item.mtime = 0
        layer.addfile(item)
    for path in sorted(entries):
        data, mode = entries[path]
        item = tarfile.TarInfo(path)
        item.size = len(data)
        item.mode = mode
        item.mtime = 0
        layer.addfile(item, io.BytesIO(data))
layer_bytes = layer_stream.getvalue()
layer_digest = hashlib.sha256(layer_bytes).hexdigest()
config = {"architecture": "amd64", "config": {"Labels": {
    "org.opencontainers.image.revision": commit,
    "org.lagrange.benchmark.tag": tag,
}}, "os": "linux", "rootfs": {"type": "layers", "diff_ids": ["sha256:" + layer_digest]}}
config_bytes = json.dumps(config, sort_keys=True, separators=(",", ":")).encode("utf-8")
config_digest = hashlib.sha256(config_bytes).hexdigest()
manifest = json.dumps({"schemaVersion": 2,
    "mediaType": "application/vnd.oci.image.manifest.v1+json",
    "config": {"digest": "sha256:" + config_digest,
               "mediaType": "application/vnd.oci.image.config.v1+json", "size": len(config_bytes)},
    "layers": [{"digest": "sha256:" + layer_digest,
                "mediaType": "application/vnd.oci.image.layer.v1.tar", "size": len(layer_bytes)}]},
    sort_keys=True, separators=(",", ":")).encode("utf-8")
manifest_digest = hashlib.sha256(manifest).hexdigest()
runnable = {"digest": "sha256:" + manifest_digest,
            "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(manifest),
            "platform": {"architecture": "amd64", "os": "linux"}}
# Alternate the two observed root forms by the fixed service ordinal.  Source
# releases now use the same repository:commit reference shape required by the
# private V2 document, so derive the ordinal from the canonical service suffix
# rather than an obsolete numeric-only tag.  Both roots are actual OCI
# authorities; a config digest is deliberately never an image ID or archive
# root after the G2 fail-closed clarification.
services = ["db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
            "recommendation-runner", "candidate-runner", "owner-beta-runner",
            "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
            "paper-scheduler"]
repository = tag.rsplit(":", 1)[0]
ordinals = [index for index, service in enumerate(services, 1)
            if repository.endswith("-" + service)]
if len(ordinals) != 1:
    raise SystemExit("fixture-tag-service-invalid")
service_ordinal = ordinals[0]
if service_ordinal % 2:
    attestation = json.dumps({"schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": "sha256:" + config_digest,
                   "mediaType": "application/vnd.oci.image.config.v1+json", "size": len(config_bytes)},
        "layers": []}, sort_keys=True, separators=(",", ":")).encode("utf-8")
    attestation_digest = hashlib.sha256(attestation).hexdigest()
    root = json.dumps({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [runnable, {"digest": "sha256:" + attestation_digest,
            "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(attestation),
            "platform": {"architecture": "unknown", "os": "unknown"},
            "annotations": {"vnd.docker.reference.digest": "sha256:" + manifest_digest,
                            "vnd.docker.reference.type": "attestation-manifest"}}]},
        sort_keys=True, separators=(",", ":")).encode("utf-8")
    root_digest = hashlib.sha256(root).hexdigest()
    root_media = "application/vnd.oci.image.index.v1+json"
    blobs = {config_digest: config_bytes, layer_digest: layer_bytes, manifest_digest: manifest,
             attestation_digest: attestation, root_digest: root}
else:
    root_digest = manifest_digest
    root_media = "application/vnd.oci.image.manifest.v1+json"
    blobs = {config_digest: config_bytes, layer_digest: layer_bytes, manifest_digest: manifest}
outer_index = json.dumps({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json",
    "manifests": [{"digest": "sha256:" + root_digest, "mediaType": root_media,
                   "size": len(blobs[root_digest])}]}, sort_keys=True, separators=(",", ":")).encode("utf-8")
docker_manifest = json.dumps([{"Config": "blobs/sha256/" + config_digest, "RepoTags": [tag],
                               "Layers": ["blobs/sha256/" + layer_digest]}],
                              sort_keys=True, separators=(",", ":")).encode("utf-8")
with tarfile.open(archive, mode="w") as outer:
    for directory in ("blobs", "blobs/sha256"):
        item = tarfile.TarInfo(directory)
        item.type = tarfile.DIRTYPE
        item.mode = 0o755
        item.mtime = 0
        outer.addfile(item)
    for digest, data in sorted(blobs.items()):
        name = "blobs/sha256/" + digest
        item = tarfile.TarInfo(name)
        item.size = len(data)
        item.mode = 0o644
        item.mtime = 0
        outer.addfile(item, io.BytesIO(data))
    for name, data in (("index.json", outer_index), ("manifest.json", docker_manifest),
                       ("oci-layout", b'{"imageLayoutVersion":"1.0.0"}')):
        item = tarfile.TarInfo(name)
        item.size = len(data)
        item.mode = 0o644
        item.mtime = 0
        outer.addfile(item, io.BytesIO(data))
print("sha256:" + root_digest)
PY
  }

  fake_unscanned_oci_root_id() {
    # Repeated state-machine-only fixture builds retain a canonical OCI
    # manifest/index root ID even though their image-save archive is not
    # requested.  The bounded primary fixture above is the one that actually
    # writes, saves, and scans all twelve archives.
    RBL_BENCH_COMMIT=$1 RBL_BENCH_TAG=$2 python3 - <<'PY'
import hashlib
import json
import os

commit = os.environ["RBL_BENCH_COMMIT"]
tag = os.environ["RBL_BENCH_TAG"]
config = json.dumps({"architecture": "amd64", "config": {"Labels": {
    "org.opencontainers.image.revision": commit, "org.lagrange.benchmark.tag": tag}},
    "os": "linux", "rootfs": {"type": "layers", "diff_ids": []}},
    sort_keys=True, separators=(",", ":")).encode("utf-8")
config_digest = hashlib.sha256(config).hexdigest()
manifest = json.dumps({"schemaVersion": 2,
    "mediaType": "application/vnd.oci.image.manifest.v1+json",
    "config": {"digest": "sha256:" + config_digest,
               "mediaType": "application/vnd.oci.image.config.v1+json", "size": len(config)},
    "layers": []}, sort_keys=True, separators=(",", ":")).encode("utf-8")
manifest_digest = hashlib.sha256(manifest).hexdigest()
services = ["db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
            "recommendation-runner", "candidate-runner", "owner-beta-runner",
            "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
            "paper-scheduler"]
repository = tag.rsplit(":", 1)[0]
ordinals = [index for index, service in enumerate(services, 1)
            if repository.endswith("-" + service)]
ordinal = ordinals[0] if len(ordinals) == 1 else int(hashlib.sha256(tag.encode("utf-8")).hexdigest()[:2], 16)
if ordinal % 2:
    root = json.dumps({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{"digest": "sha256:" + manifest_digest,
                       "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(manifest),
                       "platform": {"architecture": "amd64", "os": "linux"}}]},
        sort_keys=True, separators=(",", ":")).encode("utf-8")
    print("sha256:" + hashlib.sha256(root).hexdigest())
else:
    print("sha256:" + manifest_digest)
PY
  }

  # Use a real detached clone before installing the fake Git function.  This
  # proves the tracked source snapshot neither supplies nor needs the ignored
  # operational Compose .env, while the only image-build interpolation input
  # remains a separately created private file outside that clean checkout.
  clean_clone=$test_dir/clean-clone
  clean_clone_input=$test_dir/clean-clone-input
  clean_clone_env=$clean_clone_input/image-only-compose.env
  git clone --quiet --no-local "$repo_root" "$clean_clone" ||
    die 'self-test could not create a real clean clone for the image-only Compose env contract'
  git -C "$clean_clone" checkout --quiet --detach "$(git -C "$repo_root" rev-parse HEAD)" ||
    die 'self-test could not detach the real clean clone'
  [ -z "$(git -C "$clean_clone" status --porcelain=v1 --untracked-files=all)" ] ||
    die 'self-test real clone was dirty before image-only Compose input creation'
  [ ! -e "$clean_clone/deploy/compose/.env" ] && [ ! -L "$clean_clone/deploy/compose/.env" ] ||
    die 'self-test real clean clone unexpectedly contained an operational Compose .env'
  if git -C "$clean_clone" ls-files --error-unmatch deploy/compose/.env >/dev/null 2>&1; then
    die 'self-test operational Compose .env unexpectedly became a tracked benchmark input'
  fi
  mkdir -m 0700 -- "$clean_clone_input"
  write_benchmark_compose_env "$clean_clone_env"
  case "$clean_clone_env" in "$clean_clone"|"$clean_clone"/*)
    die 'self-test image-only Compose env was created inside the clean checkout'
  esac
  [ "$benchmark_compose_env_sha256" = "$benchmark_compose_env_sha256_expected" ] &&
    [ "$(stat -c %a -- "$clean_clone_env")" = 600 ] ||
    die 'self-test external image-only Compose env binding was malformed'
  [ -z "$(git -C "$clean_clone" status --porcelain=v1 --untracked-files=all)" ] ||
    die 'self-test image-only Compose input dirtied the real clean clone'


  # All following observations are shell fakes.  In particular, `docker` is a
  # function, so no client binary or daemon can be reached by this self-test.
  docker() {
    local command=${1:-} tag= namespace= build_arg=missing cold_nonce=missing context= probe_state=unknown host_namespace image_id=
    shift || true
    case "$command" in
      version) printf '%s\n' '27.0.0|27.0.0' ;;
      info) printf '%s\n' 'linux|amd64|fixture-kernel|/tmp' ;;
      buildx)
        case "${1:-}" in
          version) printf '%s\n' 'github.com/docker/buildx v0.18.0 fixture' ;;
          inspect)
            [ "$#" -eq 1 ] || return 91
            printf '%s\n' 'Name: fixture-builder' 'Driver: docker-container' '' 'Nodes:' 'Name: fixture-node'
            ;;
          *) return 91 ;;
        esac
        ;;
      inspect)
        local arg inspect_target= inspect_format= next=
        for arg in "$@"; do
          if [ -n "$next" ]; then
            case "$next" in
              format) inspect_format=$arg ;;
              type) [ "$arg" = container ] || return 92 ;;
              target) inspect_target=$arg ;;
            esac
            next=
            continue
          fi
          case "$arg" in
            --format) next=format ;;
            --type) next=type ;;
            --) next=target ;;
            *) return 92 ;;
          esac
        done
        [ -n "$inspect_target" ] && [ -n "$inspect_format" ] || return 92
        [ "$inspect_format" = '{{.State.Running}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}|{{index .Config.Labels "com.docker.compose.project"}}' ] || return 92
        printf 'inspect\t%s\t%s\n' "$inspect_target" "$inspect_format" >>"$BENCH_TEST_DOCKER_RECORD"
        case "${BENCH_TEST_CONTAINER_MODE:-healthy}" in
          healthy) printf '%s\n' 'true|healthy|lagrange-station' ;;
          missing) return 98 ;;
          stopped) printf '%s\n' 'false|healthy|lagrange-station' ;;
          no-healthcheck) printf '%s\n' 'true|absent|lagrange-station' ;;
          unhealthy) printf '%s\n' 'true|unhealthy|lagrange-station' ;;
          wrong-project) printf '%s\n' 'true|healthy|other-project' ;;
          *) return 99 ;;
        esac
        ;;
      build)
        local arg next_is_build_arg=0 code_commit= dockerfile=
        for arg in "$@"; do
          if [ "$next_is_build_arg" -eq 1 ]; then
            case "$arg" in
              BUILDKIT_CACHE_MOUNT_NS=*) build_arg=${arg#BUILDKIT_CACHE_MOUNT_NS=} ;;
              LAGRANGE_CODE_COMMIT=*) code_commit=${arg#LAGRANGE_CODE_COMMIT=} ;;
              LAGRANGE_BENCHMARK_COLD_NONCE=*) cold_nonce=${arg#LAGRANGE_BENCHMARK_COLD_NONCE=} ;;
            esac
            next_is_build_arg=0
            continue
          fi
          case "$arg" in
            --build-arg) next_is_build_arg=1 ;;
            -t) next_is_build_arg=2 ;;
            -f) next_is_build_arg=3 ;;
            --no-cache) return 92 ;;
            *)
              if [ "$next_is_build_arg" -eq 2 ]; then tag=$arg; next_is_build_arg=0
              elif [ "$next_is_build_arg" -eq 3 ]; then dockerfile=$arg; next_is_build_arg=0
              elif [ "${arg:0:1}" = / ]; then context=$arg
              fi
              ;;
          esac
        done
        namespace=$build_arg
        host_namespace=${BUILDKIT_CACHE_MOUNT_NS-unset}
        [[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] || return 92
        [ -n "$tag" ] && [ -f "$dockerfile" ] && [ -n "$context" ] && [ -n "${BENCH_TEST_ARCHIVE_REQUEST:-}" ] &&
          [ "$BENCH_TEST_ARCHIVE_COMMIT" = "$code_commit" ] || return 92
        if [ "${BENCH_TEST_ARCHIVE_SCAN_COUNT:-0}" -lt "${BENCH_TEST_ARCHIVE_SCAN_LIMIT:-0}" ]; then
          mkdir -p -- "$test_dir/fake-images" || return 92
          image_id=$(write_fake_image_archive "$test_dir/fake-images/$tag.tar" \
            "$BENCH_TEST_ARCHIVE_REQUEST" "$code_commit" "$context" "$tag") || return 92
          [ -f "$test_dir/fake-images/$tag.tar" ] && [ ! -L "$test_dir/fake-images/$tag.tar" ] || return 92
          BENCH_TEST_IMAGE_ARCHIVE["$image_id"]=$test_dir/fake-images/$tag.tar
          BENCH_TEST_IMAGE_REQUEST["$image_id"]=$BENCH_TEST_ARCHIVE_REQUEST
          BENCH_TEST_IMAGE_SIZE["$image_id"]=$(stat -c '%s' -- "$test_dir/fake-images/$tag.tar") || return 92
        else
          image_id=$(fake_unscanned_oci_root_id "$code_commit" "$tag") || return 92
          BENCH_TEST_IMAGE_SIZE["$image_id"]=1048576
        fi
        [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || return 92
        BENCH_TEST_IMAGE_COMMIT["$tag"]=$code_commit
        BENCH_TEST_IMAGE_ID["$tag"]=$image_id
        BENCH_TEST_IMAGE_CONTEXT["$image_id"]=$context
        BENCH_TEST_IMAGE_TAG["$image_id"]=$tag
        if grep -R -Fq 'LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915' "$context"; then probe_state=perturbed; else probe_state=unchanged; fi
        [ -n "$cold_nonce" ] && [ "$cold_nonce" != missing ] || return 92
        printf 'build\t%s\t%s\t%s\t%s\t%s\t%s\n' "$tag" "$namespace" "$build_arg" "$host_namespace" "$cold_nonce" "$probe_state" >>"$BENCH_TEST_DOCKER_RECORD"
        if [ "${BENCH_TEST_DOCKER_FAIL_BUILD:-0}" = 1 ]; then
          printf '%s\n' '#9 ERROR: fixture build failure'
          return 88
        fi
        printf '%s\n' "#5 [builder 4/7] RUN set -eu;     printf '%s\\n' LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN;     printf '%s\\n' LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN; rustc -vV; printf '%s\\n' LAGRANGE_BENCH_NATIVE_RUSTC_END;     printf '%s\\n' LAGRANGE_BENCH_NATIVE_CARGO_BEGIN; cargo -V; printf '%s\\n' LAGRANGE_BENCH_NATIVE_CARGO_END;     printf '%s\\n' LAGRANGE_BENCH_NATIVE_APK_BEGIN; apk info -vv; printf '%s\\n' LAGRANGE_BENCH_NATIVE_APK_END;     printf '%s\\n' LAGRANGE_BENCH_NATIVE_IDENTITY_END"
        if [[ "$tag" == *-db-migrate:* ]]; then
          printf '%s\n' '#5 CACHED'
        else
          cat <<'EOF'
#2 [builder 2/7] COPY Cargo.toml ./
#2 CACHED
#5 0.01 LAGRANGE_BENCH_NATIVE_IDENTITY_BEGIN
#5 0.02 LAGRANGE_BENCH_NATIVE_RUSTC_BEGIN
#5 0.03 rustc 1.97.1 (fixture)
#5 0.04 binary: rustc
#5 0.05 host: x86_64-unknown-linux-musl
#5 0.06 release: 1.97.1
#5 0.07 LAGRANGE_BENCH_NATIVE_RUSTC_END
#5 0.08 LAGRANGE_BENCH_NATIVE_CARGO_BEGIN
#5 0.09 cargo 1.97.1 (fixture)
#5 0.10 LAGRANGE_BENCH_NATIVE_CARGO_END
#5 0.11 LAGRANGE_BENCH_NATIVE_APK_BEGIN
#5 0.12 build-base-0
#5 0.13 musl-dev-0
#5 0.14 LAGRANGE_BENCH_NATIVE_APK_END
#5 0.15 LAGRANGE_BENCH_NATIVE_IDENTITY_END
#5 DONE 0.15s
EOF
        fi
        cat <<'EOF'
#9 [builder 7/7] RUN cargo clean --workspace --release --locked && cargo build -vv --message-format=json-render-diagnostics --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-fixture v0.1.0 (/build)
#9 0.21 {"reason":"compiler-artifact","package_id":"benchmark-fixture 0.1.0 (path+file:///build)","target":{"name":"benchmark-fixture","kind":["bin"],"crate_types":["bin"]},"profile":{"opt_level":"3","debuginfo":0,"debug_assertions":false,"overflow_checks":false,"test":false,"doc":false},"features":[],"fresh":false,"executable":"/cargo-target/release/benchmark-fixture"}
#9 0.22 {"reason":"build-finished","success":true}
#9 0.30 Finished `release` profile [optimized] target(s) in 0.10s
#9 DONE 0.10s
API_TOKEN=fake-secret-must-not-persist
EOF
        ;;
      compose)
        local env_file= base_file= image_file= artifact_file= service= image_ref= code_commit= expected_artifact= next=
        if [ "${1:-}" = version ]; then
          [ "$#" -eq 1 ] || return 92
          printf '%s\n' 'Docker Compose version v2.40.0 fixture'
          return 0
        fi
        [ "${1:-}" = --env-file ] && [ "$#" -ge 8 ] || return 92
        env_file=$2
        [ "${3:-}" = --file ] || return 92
        base_file=$4
        [ "${5:-}" = --file ] || return 92
        image_file=$6
        shift 6
        if [ "${1:-}" = --file ]; then
          artifact_file=${2:-}
          shift 2
        fi
        [ "${1:-}" = build ] && [ "${2:-}" = --pull=false ] && [ -n "${3:-}" ] && [ "$#" -eq 3 ] || return 92
        service=$3
        [ "$env_file" = "$benchmark_compose_env_file" ] &&
          [ -f "$env_file" ] && [ ! -L "$env_file" ] && [ "$(stat -c %a -- "$env_file")" = 600 ] &&
          [ "$(sha256sum -- "$env_file" | awk '{print $1}')" = "$benchmark_compose_env_sha256_expected" ] &&
          [ -f "$base_file" ] && [ ! -L "$base_file" ] &&
          [ -f "$image_file" ] && [ ! -L "$image_file" ] || return 92
        case "$env_file" in "$baseline_checkout"|"$baseline_checkout"/*|"$candidate_checkout"|"$candidate_checkout"/*) return 92 ;; esac
        [ ! -e "${base_file%/*}/.env" ] && [ ! -L "${base_file%/*}/.env" ] || return 92
        image_ref=$(RBL_BENCH_IMAGE_OVERRIDE=$image_file RBL_BENCH_SERVICE=$service python3 - <<'PY'
import json, os, re
path=os.environ["RBL_BENCH_IMAGE_OVERRIDE"]
service=os.environ["RBL_BENCH_SERVICE"]
raw=open(path,"rb").read()
value=json.loads(raw.decode("utf-8"))
if raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit(1)
expected=["db-role-bootstrap","db-migrate","api-server","web","research-worker","recommendation-runner","candidate-runner","owner-beta-runner","owner-equity-v2-runner","nt-backtest-worker-1","nt-backtest-worker-2","paper-scheduler"]
if set(value)!={"services"} or set(value["services"])!=set(expected) or len(value["services"])!=len(expected):
    raise SystemExit(1)
entry=value["services"].get(service)
if not isinstance(entry,dict) or set(entry)!={"image"} or not isinstance(entry["image"],str):
    raise SystemExit(1)
if not re.fullmatch(r"[a-z0-9]+(?:[._-][a-z0-9]+)*-[a-z0-9][a-z0-9_.-]{0,127}:[0-9a-f]{40}",entry["image"]):
    raise SystemExit(1)
print(entry["image"])
PY
) || return 92
        code_commit=${image_ref##*:}
        [[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] || return 92
        case "$service" in
          db-role-bootstrap|db-migrate|web)
            [ -z "$artifact_file" ] || return 92
            ;;
          *)
            [ -n "$artifact_file" ] && [ -f "$artifact_file" ] && [ ! -L "$artifact_file" ] || return 92
            RBL_BENCH_ARTIFACT_OVERRIDE=$artifact_file RBL_BENCH_SERVICE=$service RBL_BENCH_COMMIT=$code_commit python3 - <<'PY' || return 92
import json, os, re
raw=open(os.environ["RBL_BENCH_ARTIFACT_OVERRIDE"],"rb").read()
value=json.loads(raw.decode("utf-8"))
if raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit(1)
service=os.environ["RBL_BENCH_SERVICE"]
commit=os.environ["RBL_BENCH_COMMIT"]
record=value.get("services",{}).get(service)
if set(value)!={"services"} or not isinstance(record,dict) or set(record)!={"build"}:
    raise SystemExit(1)
build=record["build"]
if set(build)!={"args","additional_contexts"} or set(build["additional_contexts"])!={"release_artifacts"}:
    raise SystemExit(1)
args=build["args"]
if set(args)!={"LAGRANGE_CODE_COMMIT","RUST_ARTIFACT_SOURCE","RUST_ARTIFACT_HELPER_SHA256","RUST_ARTIFACT_BUNDLE_SHA256"}:
    raise SystemExit(1)
if args["LAGRANGE_CODE_COMMIT"]!=commit or args["RUST_ARTIFACT_SOURCE"]!="verified-artifacts":
    raise SystemExit(1)
if not re.fullmatch(r"[0-9a-f]{64}",args["RUST_ARTIFACT_HELPER_SHA256"]) or not re.fullmatch(r"[0-9a-f]{64}",args["RUST_ARTIFACT_BUNDLE_SHA256"]):
    raise SystemExit(1)
bundle=build["additional_contexts"]["release_artifacts"]
if not isinstance(bundle,str) or not bundle.startswith("/") or ".." in bundle.split("/"):
    raise SystemExit(1)
PY
            ;;
        esac
        printf 'compose\t%s\t%s\t%s\t%s\t%s\n' "$service" "$env_file" "$base_file" "$image_file" "${artifact_file:--}" >>"$BENCH_TEST_DOCKER_RECORD"
        if [ "${BENCH_TEST_COMMON_FAIL_SERVICE:-}" = "$service" ]; then
          printf '%s\n' '# common fixture Compose failure'
          return 88
        fi
        mkdir -p -- "$test_dir/fake-common-images" || return 92
        image_id=$(write_fake_common_oci_archive "$test_dir/fake-common-images/$(sha256_text "$image_ref").tar" "$code_commit" "$image_ref") || return 92
        [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || return 92
        BENCH_TEST_IMAGE_COMMIT["$image_ref"]=$code_commit
        BENCH_TEST_IMAGE_ID["$image_ref"]=$image_id
        BENCH_TEST_IMAGE_SIZE["$image_id"]=1048576
        BENCH_TEST_IMAGE_TAG["$image_id"]=$image_ref
        BENCH_TEST_IMAGE_ARCHIVE["$image_id"]=$test_dir/fake-common-images/$(sha256_text "$image_ref").tar
        printf '%s\n' '# common fixture Compose build complete'
        ;;
      image)
        case "${1:-}" in
          inspect)
            shift
            local inspect_format= inspect_target= next=
            for arg in "$@"; do
              if [ -n "$next" ]; then
                case "$next" in
                  format) inspect_format=$arg ;;
                  target) inspect_target=$arg ;;
                esac
                next=
                continue
              fi
              case "$arg" in
                --format) next=format ;;
                --) next=target ;;
                *) return 92 ;;
              esac
            done
            [ "$inspect_format" = '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}|{{.Size}}' ] || return 92
            [ -n "$inspect_target" ] && [ -n "${BENCH_TEST_IMAGE_COMMIT[$inspect_target]:-}" ] || return 92
            image_id=${BENCH_TEST_IMAGE_ID[$inspect_target]}
            printf '%s|%s|%s\n' "$image_id" "${BENCH_TEST_IMAGE_COMMIT[$inspect_target]}" "${BENCH_TEST_IMAGE_SIZE[$image_id]}"
            ;;
          save)
            shift
            local save_output= save_image= next=
            for arg in "$@"; do
              if [ -n "$next" ]; then
                case "$next" in output) save_output=$arg ;; image) save_image=$arg ;; esac
                next=
                continue
              fi
              case "$arg" in --output) next=output ;; *) [ -z "$save_image" ] || return 92; save_image=$arg ;; esac
            done
            [ -n "$save_output" ] && [ -n "$save_image" ] && [ -n "${BENCH_TEST_ARCHIVE_REQUEST:-}" ] && [ -n "${BENCH_TEST_ARCHIVE_COMMIT:-}" ] || return 92
            [ -n "${BENCH_TEST_IMAGE_CONTEXT[$save_image]:-}" ] && [ -n "${BENCH_TEST_IMAGE_ARCHIVE[$save_image]:-}" ] || return 92
            [ "${BENCH_TEST_IMAGE_REQUEST[$save_image]}" = "$BENCH_TEST_ARCHIVE_REQUEST" ] || return 92
            [ "${BENCH_TEST_IMAGE_COMMIT[${BENCH_TEST_IMAGE_TAG[$save_image]}]}" = "$BENCH_TEST_ARCHIVE_COMMIT" ] || return 92
            cp -- "${BENCH_TEST_IMAGE_ARCHIVE[$save_image]}" "$save_output"
            ;;
          rm)
            printf 'remove\t%s\n' "${3:-}" >>"$BENCH_TEST_DOCKER_RECORD"
            ;;
          *) return 92 ;;
        esac
        ;;
      system)
        [ "${1:-}" = df ] || return 92
        [ "${2:-}" = --format ] && [ "${3:-}" = '{{json .}}' ] && [ "$#" -eq 3 ] || return 92
        printf '{"Type":"Images","TotalCount":"%s","Active":"0","Size":"%sB","Reclaimable":"0B"}\n' \
          "${#BENCH_TEST_IMAGE_ID[@]}" "$(( ${#BENCH_TEST_IMAGE_ID[@]} * 1048576 ))"
        printf '%s\n' '{"Type":"Build Cache","TotalCount":"1","Active":"1","Size":"2097152B","Reclaimable":"0B"}'
        printf '%s\n' '{"Type":"Containers","TotalCount":"0","Active":"0","Size":"0B","Reclaimable":"0B"}'
        printf '%s\n' '{"Type":"Local Volumes","TotalCount":"0","Active":"0","Size":"0B","Reclaimable":"0B"}'
        ;;
      *) return 93 ;;
    esac
  }
  systemctl() {
    local command=${1:-} unit= property= end_of_options=0 value=
    if [ "$command" != show ]; then return 94; fi
    shift || true
    while [ "$#" -gt 0 ]; do
      if [ "$end_of_options" -eq 1 ]; then
        [ -z "$unit" ] || return 94
        unit=$1
        shift
        continue
      fi
      case "$1" in
        -p) [ "$#" -ge 2 ] || return 94; property=$2; shift 2 ;;
        --value) shift ;;
        --) end_of_options=1; shift ;;
        *) return 94 ;;
      esac
    done
    [ "$end_of_options" -eq 1 ] && [ -n "$unit" ] && [ -n "$property" ] || return 94
    printf 'systemctl\t%s\t%s\n' "$unit" "$property" >>"$BENCH_TEST_SYSTEMCTL_RECORD"
    case "$property" in
      ActiveState) value=active ;;
      SubState) value=running ;;
      MainPID) value=4242 ;;
      ExecMainStatus) value=0 ;;
      Nice) value=10 ;;
      IOSchedulingClass) value=idle ;;
      IOSchedulingPriority) value=0 ;;
      ControlGroup) value=/test.slice/benchmark.service ;;
      *) return 95 ;;
    esac
    printf '%s\n' "$value"
  }
  timeout() {
    local duration=${1:-}
    [ "$duration" = 10s ] || return 97
    [ "${LC_ALL:-}" = C ] || return 97
    shift
    "$@"
  }
  # The complete-range collector is a Python subprocess, so a shell function
  # would silently exercise the real journalctl.  This private executable
  # checks the frozen argv shape and cannot reach a journal or Docker daemon.
  cat >"$journalctl_bin" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
set +x

mode=${BENCH_TEST_JOURNAL_MODE:-clean}
record=${BENCH_TEST_JOURNAL_RECORD:?missing journal record path}
fixture_entry() {
  local message=$1 boot_id=${2:-11111111111111111111111111111111} timestamp=${3:-1893456000000000}
  printf '{"__REALTIME_TIMESTAMP":"%s","__CURSOR":"s=fixture","_BOOT_ID":"%s","_TRANSPORT":"kernel","MESSAGE":"%s"}\n' \
    "$timestamp" "$boot_id" "$message"
}

if [ "$#" -eq 7 ] && [ "$1" = -k ] && [ "$2" = -b ] && [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] && [ "$6" = -n ] && [ "$7" = 1 ]; then
  phase=probe
elif [ "$#" -eq 10 ] && [ "$1" = -k ] && [ "$2" = -b ] && [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] && [ "$6" = --since ] && [ "$7" = "$BENCH_TEST_JOURNAL_SINCE_UTC" ] && [ "$8" = --until ] && [ "$9" = "$BENCH_TEST_JOURNAL_GATE_UTC" ] && [ "${10}" = --no-tail ]; then
  phase=range
else
  exit 96
fi
printf '%s\t%s\n' "$phase" "$mode" >>"$record"
case "$mode:$phase" in
  probe-empty:probe) exit 0 ;;
  probe-nonzero:probe) exit 71 ;;
  nonzero-query:probe|permission-warning:probe|invalid-json-query:probe|wrong-boot-query:probe|partial-range:probe|oom:probe|clean:probe)
    fixture_entry fixture-probe
    ;;
  nonzero-query:range) exit 72 ;;
  permission-warning:range)
    printf '%s\n' 'fixture-journal-permission-warning-must-not-persist' >&2
    ;;
  invalid-json-query:range) printf '%s\n' '{invalid-json' ;;
  wrong-boot-query:range) fixture_entry fixture-quiet 22222222222222222222222222222222 ;;
  partial-range:range) printf '%s' '{"__REALTIME_TIMESTAMP":"1893456000000000"}' ;;
  oom:range) fixture_entry 'Out of memory: fixture' ;;
  clean:range) exit 0 ;;
  *) exit 98 ;;
esac
EOF
  chmod 0700 -- "$journalctl_bin"
  ps() { printf '%s\n' "${BENCH_TEST_PS_NAMES:-}"; }
  git() {
    local checkout= argument tree commit parent
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -C) checkout=$2; shift 2 ;;
        -c) shift 2 ;;
        *) break ;;
      esac
    done
    argument=${1:-}
    shift || true
    case "$argument" in
      --version) printf '%s\n' 'git version 2.53.0 fixture' ;;
      rev-parse)
        case "${1:-}" in
          HEAD) cat "$checkout/.benchmark-commit" ;;
          'HEAD^{tree}') printf 'fixture-tree-%s\n' "$(cat "$checkout/.benchmark-commit")" ;;
          *) return 96 ;;
        esac
        ;;
      status) return 0 ;;
      add) return 0 ;;
      write-tree)
        printf 'fixture-tree-%s\n' "$(cat "$checkout/.benchmark-commit")"
        ;;
      commit-tree)
        parent=
        while [ "$#" -gt 0 ]; do
          case "$1" in
            -p) [ "$#" -ge 2 ] || return 96; parent=$2; shift 2 ;;
            *) shift ;;
          esac
        done
        case "$parent" in
          1111111111111111111111111111111111111111) printf '%040d\n' 3 ;;
          2222222222222222222222222222222222222222) printf '%040d\n' 4 ;;
          0000000000000000000000000000000000000003) printf '%040d\n' 5 ;;
          0000000000000000000000000000000000000004) printf '%040d\n' 6 ;;
          0000000000000000000000000000000000000005) printf '%040d\n' 7 ;;
          0000000000000000000000000000000000000006) printf '%040d\n' 8 ;;
          *) return 96 ;;
        esac
        ;;
      checkout)
        while [ "$#" -gt 0 ]; do
          commit=$1
          shift
        done
        [ -n "$commit" ] || return 96
        printf '%s\n' "$commit" >"$checkout/.benchmark-commit"
        ;;
      diff)
        if grep -R -Fq 'LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915' "$checkout"; then
          printf 'diff --git a/probe b/probe\n@@ -0,0 +1 @@\n+LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260915\n'
        fi
        ;;
      *) return 97 ;;
    esac
  }
  prepare_checkout() {
    local commit=$1 checkout=$2 service dockerfile path parent tracked_path
    mkdir -p -- "$checkout"
    printf '%s\n' "$commit" >"$checkout/.benchmark-commit"
    printf '%s\n' '[toolchain]' 'channel = "1.97.1"' >"$checkout/rust-toolchain.toml"
    for service in "${services[@]}"; do
      dockerfile=$checkout/${service_dockerfile[$service]}
      mkdir -p -- "${dockerfile%/*}"
      if [ "${service_cargo_mode[$service]}" -gt 0 ]; then
        # Keep a real earliest native setup layer in the fake recipe.  The
        # stable cold nonce must sit before it; a fixture without this layer
        # would only prove a weaker, later cache miss.
        printf '%s\n' 'FROM rust:1.97.1-alpine' \
          'RUN apk add --no-cache build-base' \
          'RUN cargo build --locked --release' >"$dockerfile"
      else
        printf '%s\n' 'FROM node:fixture' 'RUN npm run build' >"$dockerfile"
      fi
    done
    for path in "${scenario_input_path[@]}"; do
      [ "$path" = - ] && continue
      parent=${path%/*}
      [ "$parent" = "$path" ] && parent=.
      mkdir -p -- "$checkout/$parent"
      case "$path" in
        Cargo.lock) printf '%s\n' 'version = 4' >"$checkout/$path" ;;
        *.rs) printf '%s\n' '// fixture source' >"$checkout/$path" ;;
        *.tsx) printf '%s\n' 'export default function FixtureLayout() { return null; }' >"$checkout/$path" ;;
        *.py) printf '%s\n' '# fixture source' >"$checkout/$path" ;;
        *.json) printf '%s\n' '{}' >"$checkout/$path" ;;
        *) return 97 ;;
      esac
    done
    # The strict A/B archive-request writer uses the product Git-tracked runtime
    # selector. Materialize only `git ls-files` results from the real source;
    # never recursively copy the ignored user nt/.venv into test data.
    for path in \
      tests/fixtures/kr-etf/contract tests/fixtures/kr-candidates/contract \
      configs/universes/kr-stock-price-beta-v1.json \
      configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json \
      nt deploy/runtime/paper-runner-entrypoint migrations \
      configs/strategies/baseline-v1.json deploy/db/sync-baseline-strategy-catalog.sql \
      deploy/db/migrate.sh deploy/db/bootstrap-roles.sh; do
      while IFS= read -r -d '' tracked_path; do
        mkdir -p -- "$checkout/${tracked_path%/*}"
        cp -p -- "$repo_root/$tracked_path" "$checkout/$tracked_path"
      done < <("$real_git_bin" -C "$repo_root" ls-files -z -- "$path")
    done
    # Every clean checkout, including an A/B fallback fixture, retains the
    # unchanged manifest library.  A C fixture additionally receives its own
    # helper/layout/producer and Compose input files; no tracked fixture is
    # modified and no source Dockerfile is instrumented on this route.
    mkdir -p -- "$checkout/scripts/ops/lib"
    cp -a -- "$repo_root/scripts/ops/lib/release-image-manifest.sh" \
      "$checkout/scripts/ops/lib/release-image-manifest.sh"
    if [ "${BENCH_TEST_COMMON_LAYOUT:-0}" = 1 ]; then
      mkdir -p -- "$checkout/deploy/build" "$checkout/deploy/compose"
      cp -a -- "$repo_root/scripts/ops/lib/release-build-layout.sh" \
        "$checkout/scripts/ops/lib/release-build-layout.sh"
      cp -a -- "$repo_root/deploy/build/release-build-layout.json" \
        "$checkout/deploy/build/release-build-layout.json"
      cp -a -- "$repo_root/deploy/build/Dockerfile.rust-artifacts" \
        "$checkout/deploy/build/Dockerfile.rust-artifacts"
      printf '%s\n' 'services: {}' >"$checkout/deploy/compose/compose.yml"
    fi
    cp -p -- "$repo_root/.dockerignore" "$checkout/.dockerignore"
    cp -p -- "$repo_root/.gitignore" "$checkout/.gitignore"
    # Python subprocesses invoked by the product inventory helper resolve the
    # real Git executable, so give each otherwise synthetic checkout a real
    # tracked index. The shell-level fake still controls benchmark commit/order
    # behavior and no repository source is changed.
    "$real_git_bin" -C "$checkout" init -q
    "$real_git_bin" -C "$checkout" add -- .
    "$real_git_bin" -C "$checkout" -c user.name=fixture \
      -c user.email=fixture@example.invalid commit -qm fixture
    mkdir -p -- "$checkout/nt/.venv/bin" "$checkout/nt/__pycache__"
    [ -L "$checkout/nt/.venv/bin/python" ] || \
      ln -s ../python-fixture-target "$checkout/nt/.venv/bin/python"
    printf '%s\n' 'ignored benchmark virtualenv bytes' >"$checkout/nt/.venv/ignored.txt"
    printf '%s\n' 'ignored benchmark pycache bytes' >"$checkout/nt/__pycache__/ignored.pyc"
    [ -z "$("$real_git_bin" -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] ||
      return 97
  }
  benchmark_validate_common_fixture() {
    local revision=$1 checkout=$2 helper=$3 config=$4 artifact=$5 commit=$6
    [ "${BENCH_TEST_COMMON_LAYOUT:-0}" = 1 ] || return 1
    [ "$helper" = "$checkout/scripts/ops/lib/release-build-layout.sh" ] &&
      [ "$config" = "$checkout/deploy/build/release-build-layout.json" ] &&
      [ "$artifact" = "$checkout/deploy/build/Dockerfile.rust-artifacts" ] &&
      [ -f "$helper" ] && [ ! -L "$helper" ] && [ -f "$config" ] && [ ! -L "$config" ] &&
      [ -f "$artifact" ] && [ ! -L "$artifact" ] && is_exact_commit "$commit" || return 1
    printf 'common-plan\t%s\t%s\t%s\n' "$revision" "$commit" "$(sha256_file "$helper")" >>"$BENCH_TEST_COMMON_RECORD"
  }
  write_fake_common_oci_archive() {
    local archive=$1 commit=$2 tag=$3
    RBL_BENCH_ARCHIVE=$archive RBL_BENCH_COMMIT=$commit RBL_BENCH_TAG=$tag python3 - <<'PY'
import hashlib
import io
import json
import os
import tarfile

archive=os.environ["RBL_BENCH_ARCHIVE"]
commit=os.environ["RBL_BENCH_COMMIT"]
tag=os.environ["RBL_BENCH_TAG"]
payload=b"\x7fELF\x02\x01\x01"+(b"\0"*9)+b"\x02\x00\x3e\x00"+b"fixture-common-bin\n"
with io.BytesIO() as buffer:
    with tarfile.open(fileobj=buffer,mode="w") as layer:
        for name in ("usr","usr/local","usr/local/bin"):
            entry=tarfile.TarInfo(name); entry.type=tarfile.DIRTYPE; entry.mode=0o755; entry.mtime=0; layer.addfile(entry)
        entry=tarfile.TarInfo("usr/local/bin/test-bin"); entry.size=len(payload); entry.mode=0o755; entry.mtime=0; layer.addfile(entry,io.BytesIO(payload))
    layer_bytes=buffer.getvalue()
layer_digest=hashlib.sha256(layer_bytes).hexdigest()
# A Docker image ID is rooted in the selected OCI manifest/index, not a bare
# config hash.  Real per-service images have distinct root content; bind this
# fixture's service tag into the OCI config so every fake archive and its
# returned root ID remain coherently one-to-one through revalidation.
config=json.dumps({"architecture":"amd64","config":{"Labels":{"org.opencontainers.image.revision":commit,"org.opencontainers.image.title":tag}},"os":"linux","rootfs":{"diff_ids":["sha256:"+layer_digest],"type":"layers"}},sort_keys=True,separators=(",",":")).encode()
config_digest=hashlib.sha256(config).hexdigest()
manifest=json.dumps({"config":{"digest":"sha256:"+config_digest,"mediaType":"application/vnd.oci.image.config.v1+json","size":len(config)},"layers":[{"digest":"sha256:"+layer_digest,"mediaType":"application/vnd.oci.image.layer.v1.tar","size":len(layer_bytes)}],"mediaType":"application/vnd.oci.image.manifest.v1+json","schemaVersion":2},sort_keys=True,separators=(",",":")).encode()
manifest_digest=hashlib.sha256(manifest).hexdigest()
descriptor={"digest":"sha256:"+manifest_digest,"mediaType":"application/vnd.oci.image.manifest.v1+json","platform":{"architecture":"amd64","os":"linux"},"size":len(manifest)}
if int(hashlib.sha256(tag.encode()).hexdigest()[:2],16)%2:
    root=json.dumps({"manifests":[descriptor],"mediaType":"application/vnd.oci.image.index.v1+json","schemaVersion":2},sort_keys=True,separators=(",",":")).encode()
    root_digest=hashlib.sha256(root).hexdigest(); root_media="application/vnd.oci.image.index.v1+json"
    blobs={config_digest:config,layer_digest:layer_bytes,manifest_digest:manifest,root_digest:root}
else:
    root_digest=manifest_digest; root_media="application/vnd.oci.image.manifest.v1+json"
    blobs={config_digest:config,layer_digest:layer_bytes,manifest_digest:manifest}
outer=json.dumps({"manifests":[{"digest":"sha256:"+root_digest,"mediaType":root_media,"size":len(blobs[root_digest])}],"mediaType":"application/vnd.oci.image.index.v1+json","schemaVersion":2},sort_keys=True,separators=(",",":")).encode()
docker_manifest=json.dumps([{"Config":"blobs/sha256/"+config_digest,"Layers":["blobs/sha256/"+layer_digest],"RepoTags":[tag]}],sort_keys=True,separators=(",",":")).encode()
with tarfile.open(archive,mode="w") as outer_tar:
    for name in ("blobs","blobs/sha256"):
        entry=tarfile.TarInfo(name); entry.type=tarfile.DIRTYPE; entry.mode=0o755; entry.mtime=0; outer_tar.addfile(entry)
    for digest,data in sorted(blobs.items()):
        entry=tarfile.TarInfo("blobs/sha256/"+digest); entry.size=len(data); entry.mode=0o644; entry.mtime=0; outer_tar.addfile(entry,io.BytesIO(data))
    for name,data in (("index.json",outer),("manifest.json",docker_manifest),("oci-layout",b'{"imageLayoutVersion":"1.0.0"}')):
        entry=tarfile.TarInfo(name); entry.size=len(data); entry.mode=0o644; entry.mtime=0; outer_tar.addfile(entry,io.BytesIO(data))
print("sha256:"+root_digest)
PY
  }
  write_fake_common_archive_request() {
    local destination=$1
    [ ! -e "$destination" ] && [ ! -L "$destination" ] || return 1
    RBL_BENCH_REQUEST=$destination python3 - <<'PY'
import json, os
value={"files":[{"contains_hex":[],"elf":True,"executable":True,"path":"usr/local/bin/test-bin","sha256":None}],"format":"lagrange-image-files-v1","nonempty_directories":["usr/local/bin"]}
data=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
fd=os.open(os.environ["RBL_BENCH_REQUEST"],os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,"wb") as handle:
    handle.write(data); handle.flush(); os.fsync(handle.fileno())
PY
  }
  benchmark_common_fixture_recipe_for_service() {
    local service=$1 checkout=$2
    RBL_BENCH_LAYOUT=$checkout/deploy/build/release-build-layout.json RBL_BENCH_SERVICE=$service python3 - <<'PY'
import json, os, re
layout=json.load(open(os.environ["RBL_BENCH_LAYOUT"],encoding="utf-8"))
value=layout.get("services",{}).get(os.environ["RBL_BENCH_SERVICE"])
if not isinstance(value,dict) or set(value)!={"kind","recipe"}: raise SystemExit(1)
recipe=value["recipe"]
if recipe is None:
    if value["kind"] not in ("database","web"): raise SystemExit(1)
    print("NONE")
elif isinstance(recipe,str) and re.fullmatch(r"D[1-7]",recipe) and value["kind"]=="rust":
    print(recipe)
else: raise SystemExit(1)
PY
  }
  benchmark_common_fixture_materialize_recipe() {
    local checkout=$1 state_root=$2 recipe=$3 commit=$4 bin producer bundle
    bundle=$state_root/bundles/$recipe
    [ ! -e "$bundle" ] && [ ! -L "$bundle" ] || return 0
    mkdir -m 0700 -- "$state_root/producers/$recipe" "$bundle" "$bundle/.release-build" || return 1
    mkdir -- "$bundle/target" "$bundle/target/release" || return 1
    while IFS= read -r bin; do
      [ -n "$bin" ] || return 1
      producer=$state_root/producers/$recipe/$bin
      mkdir -m 0700 -- "$producer" "$producer/bin" || return 1
      RBL_BENCH_PRODUCER=$producer RBL_BENCH_BIN=$bin RBL_BENCH_RECIPE=$recipe RBL_BENCH_COMMIT=$commit python3 - <<'PY'
import hashlib, json, os
root=os.environ["RBL_BENCH_PRODUCER"]; name=os.environ["RBL_BENCH_BIN"]
binary=b"\x7fELF\x02\x01\x01"+(b"\0"*9)+b"\x02\x00\x3e\x00"+name.encode()+b"\n"
open(os.path.join(root,"bin",name),"wb").write(binary); os.chmod(os.path.join(root,"bin",name),0o755)
artifact={"bin":name}
event={"reason":"compiler-artifact","package_id":"fixture "+name,"target":{"name":name,"kind":["bin"],"crate_types":["bin"]},"profile":{"opt_level":"3"},"features":[],"fresh":False,"executable":"/cargo-target/release/"+name}
raw=(json.dumps(event,sort_keys=True,separators=(",",":"))+"\n"+json.dumps({"reason":"build-finished","success":True},sort_keys=True,separators=(",",":"))+"\n").encode()
for path,value in (("artifact.json",json.dumps(artifact,sort_keys=True,separators=(",",":"))+"\n"),("timing.json",json.dumps({"cargo_ms":1,"format":"lagrange-cargo-timing-v1"},sort_keys=True,separators=(",",":"))+"\n")):
    target=os.path.join(root,path); open(target,"w",encoding="utf-8",newline="\n").write(value); os.chmod(target,0o600)
target=os.path.join(root,"cargo.jsonl"); open(target,"wb").write(raw); os.chmod(target,0o600)
PY
      install -m 0755 -- "$producer/bin/$bin" "$bundle/$(printf 'target/release/%s' "$bin")" || return 1
    done < <(RBL_BENCH_LAYOUT=$checkout/deploy/build/release-build-layout.json RBL_BENCH_RECIPE=$recipe python3 - <<'PY'
import json,os
layout=json.load(open(os.environ["RBL_BENCH_LAYOUT"],encoding="utf-8")); recipe=layout["recipes"][os.environ["RBL_BENCH_RECIPE"]]
for name in recipe["bins"]: print(name)
PY
)
    printf '{"format":"fixture-common-bundle","recipe":"%s"}\n' "$recipe" >"$bundle/.release-build/bundle.json"
    chmod 0600 -- "$bundle/.release-build/bundle.json"
  }
  benchmark_install_common_fake_api() {
    local selected_helper=$1
    release_build_layout_lock() {
      case "${RELEASE_BUILD_LAYOUT_LOCK_HELD:-0}" in
        1) printf 'lock\tinherited\t%s\n' "${RELEASE_BUILD_LAYOUT_LOCK_DIR:-missing}" >>"$BENCH_TEST_COMMON_RECORD" ;;
        0) RELEASE_BUILD_LAYOUT_LOCK_HELD=1; RELEASE_BUILD_LAYOUT_LOCK_DIR="${RBL_LOCK_PREFIX}-$(id -u)"; export RELEASE_BUILD_LAYOUT_LOCK_HELD RELEASE_BUILD_LAYOUT_LOCK_DIR; printf 'lock\tparent\t%s\n' "$RELEASE_BUILD_LAYOUT_LOCK_DIR" >>"$BENCH_TEST_COMMON_RECORD" ;;
        *) return 1 ;;
      esac
    }
    release_build_layout_init() {
      local source_root=$1 commit=$2 state_root=$3 namespace=$4 native
      [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" != 1 ] && [ ! -e "$state_root" ] && [ ! -L "$state_root" ] || return 1
      [ -d "$source_root" ] && [ -f "$source_root/deploy/build/release-build-layout.json" ] &&
        is_exact_commit "$commit" && [[ "$namespace" =~ ^[a-z0-9][a-z0-9_.-]{0,127}$ ]] || return 1
      release_build_layout_lock || return 1
      # This is the fake helper's normal init implementation.  The test setup
      # deliberately did not create this root, so a fresh-output defect cannot
      # be hidden by pre-existing state.
      mkdir -m 0700 -- "$state_root" || return 1
      mkdir -m 0700 -- "$state_root/contexts" "$state_root/producers" "$state_root/bundles" "$state_root/images" "$state_root/verification" "$state_root/gates" "$state_root/logs" "$state_root/overrides" || return 1
      native=$state_root/native-identity.json
      printf '%s\n' '{"apk_info_vv":"build-base-0.5-r4\nmusl-dev-1.2.6-r2\nopenssl-dev-3.5.8-r0\npkgconf-2.5.1-r0\npostgresql18-dev-18.6-r0\n","apk_installed_packages":[{"name":"build-base","provides":[],"status":["installed"],"version":"0.5-r4"},{"name":"musl-dev","provides":[],"status":["installed"],"version":"1.2.6-r2"},{"name":"openssl-dev","provides":[],"status":["installed"],"version":"3.5.8-r0"},{"name":"pkgconf","provides":[],"status":["installed"],"version":"2.5.1-r0"},{"name":"postgresql18-dev","provides":["postgresql-dev"],"status":["installed"],"version":"18.6-r0"}],"cargo_version":"cargo 1.97.1 (fixture)","compiler_env":{"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>"},"format":"lagrange-build-layout-native-v3","host_triple":"x86_64-unknown-linux-musl","native_packages":["build-base","musl-dev","openssl-dev","pkgconf","postgresql-dev"],"rustc_vv":"rustc 1.97.1 (fixture)\nhost: x86_64-unknown-linux-musl\n","target_platform":"linux/amd64"}' >"$native"
      chmod 0600 -- "$native"
      RELEASE_BUILD_LAYOUT_INITIALIZED=1
      RELEASE_BUILD_LAYOUT_SOURCE_ROOT=$source_root
      RELEASE_BUILD_LAYOUT_COMMIT=$commit
      RELEASE_BUILD_LAYOUT_STATE_ROOT=$state_root
      RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE=$namespace
      RELEASE_BUILD_LAYOUT_HELPER_SHA256=$(sha256_file "$source_root/scripts/ops/lib/release-build-layout.sh")
      export RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT RELEASE_BUILD_LAYOUT_COMMIT RELEASE_BUILD_LAYOUT_STATE_ROOT RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RELEASE_BUILD_LAYOUT_HELPER_SHA256
      printf 'init\t%s\t%s\t%s\n' "$source_root" "$commit" "$namespace" >>"$BENCH_TEST_COMMON_RECORD"
    }
    release_build_layout_gate() {
      local label=$1 previous=$2
      [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] && [ "$previous" = 0 ] &&
        [ "${RELEASE_BUILD_SYSTEMD_UNIT:-}" = "${BENCHMARK_SYSTEMD_SERVICE:-}" ] &&
        [ "${RELEASE_BUILD_SYSTEMD_MANAGER:-system}" = "${BENCHMARK_SYSTEMD_MANAGER:-system}" ] &&
        [ "${RELEASE_BUILD_HEALTH_UNITS:-}" = "${BENCHMARK_PRODUCTION_HEALTH_UNITS:-}" ] &&
        [ "${RELEASE_BUILD_HEALTH_CONTAINERS:-}" = "${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-}" ] &&
        [ "${RELEASE_BUILD_RESEARCH_EXCEPTION:-}" = "${BENCHMARK_RESEARCH_EXCEPTION:-}" ] || return 1
      printf 'gate\t%s\t%s\n' "$label" "$previous" >>"$BENCH_TEST_COMMON_RECORD"
    }
    release_build_layout_prepare() {
      local service=$1 commit=$2 state_root=$3 recipe bundle
      [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] && [ "$commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] && [ "$state_root" = "$RELEASE_BUILD_LAYOUT_STATE_ROOT" ] || return 1
      [ "${BENCH_TEST_COMMON_FAIL_PREP_SERVICE:-}" != "$service" ] || return 88
      recipe=$(benchmark_common_fixture_recipe_for_service "$service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT") || return 1
      if [ "$recipe" = NONE ]; then
        printf '%s\n' NONE
        return 0
      fi
      bundle=$state_root/bundles/$recipe
      if [ -d "$bundle" ]; then
        printf 'producer\treuse\t%s\t%s\n' "$recipe" "$service" >>"$BENCH_TEST_COMMON_RECORD"
      else
        benchmark_common_fixture_materialize_recipe "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$state_root" "$recipe" "$commit" || return 1
        printf 'producer\tnew\t%s\t%s\n' "$recipe" "$service" >>"$BENCH_TEST_COMMON_RECORD"
      fi
      printf '%s\n' "$bundle"
    }
    release_build_layout_verify_bundle() {
      local service=$1 commit=$2 bundle=$3 recipe expected
      [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] && [ "$commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] || return 1
      recipe=$(benchmark_common_fixture_recipe_for_service "$service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT") || return 1
      [ "$recipe" != NONE ] || return 1
      expected=$RELEASE_BUILD_LAYOUT_STATE_ROOT/bundles/$recipe
      [ "$bundle" = "$expected" ] && [ -d "$bundle" ] && [ -f "$bundle/.release-build/bundle.json" ] || return 1
      sha256_file "$bundle/.release-build/bundle.json"
    }
    release_build_layout_write_override() {
      local service=$1 bundle=$2 destination=$3 digest
      digest=$(release_build_layout_verify_bundle "$service" "$RELEASE_BUILD_LAYOUT_COMMIT" "$bundle") || return 1
      [ ! -e "$destination" ] && [ ! -L "$destination" ] && [ -d "${destination%/*}" ] && [ ! -L "${destination%/*}" ] || return 1
      RBL_BENCH_OVERRIDE=$destination RBL_BENCH_SERVICE=$service RBL_BENCH_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT RBL_BENCH_HELPER=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 RBL_BENCH_BUNDLE=$bundle RBL_BENCH_DIGEST=$digest python3 - <<'PY'
import json,os
value={"services":{os.environ["RBL_BENCH_SERVICE"]:{"build":{"args":{"LAGRANGE_CODE_COMMIT":os.environ["RBL_BENCH_COMMIT"],"RUST_ARTIFACT_SOURCE":"verified-artifacts","RUST_ARTIFACT_HELPER_SHA256":os.environ["RBL_BENCH_HELPER"],"RUST_ARTIFACT_BUNDLE_SHA256":os.environ["RBL_BENCH_DIGEST"]},"additional_contexts":{"release_artifacts":os.environ["RBL_BENCH_BUNDLE"]}}}}}
data=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
fd=os.open(os.environ["RBL_BENCH_OVERRIDE"],os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,"wb") as handle:
    handle.write(data);handle.flush();os.fsync(handle.fileno())
PY
    }
    release_build_layout_verify_image() {
      local service=$1 commit=$2 image_id=$3 state_root=$4 request archive final temporary tag
      [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] && [ "$commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] && [ "$state_root" = "$RELEASE_BUILD_LAYOUT_STATE_ROOT" ] || return 1
      [ "${BENCH_TEST_COMMON_FAIL_VERIFY_SERVICE:-}" != "$service" ] || return 89
      tag=${BENCH_TEST_IMAGE_TAG[$image_id]:-}
      archive=${BENCH_TEST_IMAGE_ARCHIVE[$image_id]:-}
      [ -n "$tag" ] && [ -f "$archive" ] && [ ! -L "$archive" ] || return 1
      request=$state_root/verification/$service-request.json
      if [ ! -e "$request" ] && [ ! -L "$request" ]; then
        write_fake_common_archive_request "$request" || return 1
      fi
      final=$state_root/verification/$service.json
      temporary=$state_root/verification/.fixture-$service-$$.json
      [ ! -e "$temporary" ] && [ ! -L "$temporary" ] || return 1
      release_build_layout_archive_scan "$archive" "$image_id" linux/amd64 "$commit" "$request" "$temporary" || return 1
      if [ -e "$final" ] || [ -L "$final" ]; then
        cmp -s -- "$temporary" "$final" || { rm -f -- "$temporary"; return 1; }
        rm -f -- "$temporary"
      else
        mv -- "$temporary" "$final" || return 1
      fi
      [ "$(stat -c '%u:%a' -- "$final")" = "$(id -u):600" ]
    }
  }
  [ "$(type -t docker)" = function ] || die 'self-test Docker fake was not installed'
  [ "$(type -t git)" = function ] || die 'self-test Git fake was not installed'
  [ "$(type -t systemctl)" = function ] || die 'self-test systemd fake was not installed'
  [ -x "$journalctl_bin" ] && [ ! -L "$journalctl_bin" ] || die 'self-test journalctl executable fake was not installed'
  [ "$(type -t timeout)" = function ] || die 'self-test timeout fake was not installed'

  # The embedded benchmark self-test does not have permission to model live
  # production controls. Keep its existing no-Docker boundary by faking only
  # the public benchmark-gate initialization/dispatch; the separate gate
  # self-test exercises the real helper with subprocess fixtures.
  benchmark_test_shared_init() {
    benchmark_gate_source_root=$repo_root
    benchmark_gate_source_commit=$("$real_git_bin" -C "$repo_root" rev-parse HEAD) || return 1
    benchmark_gate_state_root=$output_dir/benchmark-gate-state
    benchmark_gate_namespace=lagrange-benchmark-gate-${benchmark_gate_source_commit:0:12}
    benchmark_gate_helper=$layout_helper
    benchmark_gate_lock_prefix=${RBL_LOCK_PREFIX:-/tmp/lagrange-production-image-build}
    release_build_layout_lock || return 1
    if [ "${BENCH_TEST_COMMON_LAYOUT:-0}" = 1 ] && [ -n "${BENCH_TEST_COMMON_RECORD-}" ]; then
      printf 'lock\tparent\t%s\n' "$RELEASE_BUILD_LAYOUT_LOCK_DIR" >>"$BENCH_TEST_COMMON_RECORD"
    fi
    benchmark_gate_initialized=1
  }
  benchmark_test_shared_gate() {
    local label=$1 previous=$2
    [ -n "$label" ] && [ "$previous" != "" ] && [[ "$previous" =~ ^[0-9]+$ ]] || {
      gate_reason=shared-release-layout-gate-failed
      return 1
    }
    gate_build_service_state=verified
    if [ -z "${BENCHMARK_PRODUCTION_HEALTH_UNITS:-}" ]; then
      gate_health_state=fail
      gate_reason=missing-production-health-units
      return 1
    fi
    if [ -z "${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-}" ]; then
      gate_health_state=fail
      gate_reason=missing-production-health-containers
      return 1
    fi
    case "${BENCH_TEST_CONTAINER_MODE:-healthy}" in
      healthy) gate_health_state=verified ;;
      missing) gate_health_state=fail; gate_reason=production-container-unreadable; return 1 ;;
      stopped) gate_health_state=fail; gate_reason=production-container-not-running; return 1 ;;
      no-healthcheck) gate_health_state=fail; gate_reason=production-container-health-unestablished; return 1 ;;
      unhealthy) gate_health_state=fail; gate_reason=production-container-unhealthy; return 1 ;;
      wrong-project) gate_health_state=fail; gate_reason=production-container-wrong-compose-project; return 1 ;;
      *) gate_health_state=fail; gate_reason=shared-release-layout-gate-failed; return 1 ;;
    esac
  }

  expect_journal_snapshot_failure() {
    local mode=$1 expected_reason=$2 description=$3
    BENCH_TEST_JOURNAL_MODE=$mode
    BENCH_TEST_BOOT_ID_MODE=readable
    gate_reason=
    reset_journal_run_state
    if snapshot_resources; then
      die "self-test accepted failed kernel journal observation: $description"
    fi
    [ "$gate_reason" = "$expected_reason" ] ||
      die "self-test kernel journal failure reason regressed for $description: $gate_reason"
  }

  # A genuinely quiet window is zero OOM only after an independently readable,
  # current-boot kernel entry succeeds.  The fake validates the probe timeout
  # and the complete-range --no-tail argv emitted by the bounded collector.
  : >"$BENCH_TEST_JOURNAL_RECORD"
  BENCH_TEST_JOURNAL_MODE=clean
  BENCH_TEST_BOOT_ID_MODE=readable
  gate_reason=
  reset_journal_run_state
  snapshot_resources || die 'self-test rejected readable probe plus legitimate quiet kernel window'
  [ "$gate_oom_events" = 0 ] || die 'self-test quiet kernel window did not record zero OOM matches'
  [ "$journal_probe_status" = exit-0 ] && [ "$journal_probe_entry_count" = 1 ] ||
    die 'self-test readable kernel probe evidence was incomplete'
  [ "$journal_lookback_status" = exit-0 ] && [ "$journal_lookback_entry_count" = 0 ] && [ "$journal_lookback_oom_count" = 0 ] ||
    die 'self-test quiet kernel lookback evidence was incomplete'
  [ "$journal_capture_status" = complete ] && [ "$journal_capture_failure" = none ] &&
    [[ "$journal_capture_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test complete kernel range capture receipt was incomplete'
  [ "$journal_first_gate_utc" = "$BENCH_TEST_JOURNAL_GATE_UTC" ] && [ "$journal_since_utc" = "$BENCH_TEST_JOURNAL_SINCE_UTC" ] ||
    die 'self-test fixed kernel journal range regressed'
  [[ "$journal_probe_stdout_sha256" =~ ^[0-9a-f]{64}$ ]] && [[ "$journal_probe_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] &&
    [[ "$journal_lookback_stdout_sha256" =~ ^[0-9a-f]{64}$ ]] && [[ "$journal_lookback_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test kernel journal hashes were not retained'
  awk -F '\t' '$1 == "probe" && $2 == "clean" { probe++ } $1 == "range" && $2 == "clean" { range++ } END { exit !(probe == 1 && range == 1) }' "$BENCH_TEST_JOURNAL_RECORD" ||
    die 'self-test kernel journal probe/complete-range calls were not both made'

  # Query failure, a successful query with a warning, unavailable boot ID,
  # empty probe output, malformed JSON, and wrong boot metadata are all fail
  # closed.  None can be interpreted as a quiet/OOM-free window.
  expect_journal_snapshot_failure nonzero-query kernel-journal-lookback-nonzero nonzero-lookback-query
  [ "$journal_probe_status" = exit-0 ] && [ "$journal_probe_entry_count" = 1 ] && [ "$journal_lookback_status" = exit-1 ] &&
    [ "$journal_capture_status" = failed ] && [ "$journal_capture_failure" = receipt-invalid-or-incomplete ] ||
    die 'self-test nonzero lookback query evidence regressed'
  expect_journal_snapshot_failure permission-warning kernel-journal-lookback-stderr exit-zero-permission-warning
  [ "$journal_lookback_status" = exit-1 ] && [ "$journal_capture_status" = failed ] &&
    [[ "$journal_lookback_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test warning-bearing lookback did not retain bounded status/hash evidence'
  printf '%s\t%s\t%s\t%s\n' "$journal_lookback_status" "$journal_lookback_stderr_sha256" \
    "$journal_capture_status" "$journal_capture_failure" >"$test_dir/journal-public-evidence.tsv"
  grep -Fq 'fixture-journal-permission-warning-must-not-persist' "$test_dir/journal-public-evidence.tsv" &&
    die 'self-test persisted journal warning text in public journal evidence'
  expect_journal_snapshot_failure probe-empty kernel-journal-probe-empty empty-probe
  [ "$journal_probe_status" = exit-0 ] && [ "$journal_lookback_status" = unavailable ] ||
    die 'self-test empty probe reached or accepted the lookback query'
  expect_journal_snapshot_failure probe-nonzero kernel-journal-probe-nonzero unreadable-probe
  [ "$journal_probe_status" = exit-71 ] && [ "$journal_lookback_status" = unavailable ] ||
    die 'self-test unreadable probe reached or accepted the lookback query'
  BENCH_TEST_JOURNAL_MODE=clean
  BENCH_TEST_BOOT_ID_MODE=unreadable
  gate_reason=
  reset_journal_run_state
  if snapshot_resources; then die 'self-test accepted unreadable current boot ID'; fi
  [ "$gate_reason" = kernel-journal-boot-id-unreadable ] || die 'self-test unreadable boot ID reason regressed'
  [ "$journal_probe_status" = unavailable ] && [ "$journal_lookback_status" = unavailable ] ||
    die 'self-test unreadable boot ID reached journalctl'
  expect_journal_snapshot_failure invalid-json-query kernel-journal-lookback-invalid-json invalid-json-lookback
  [ "$journal_probe_entry_count" = 1 ] && [ "$journal_lookback_status" = exit-0 ] ||
    die 'self-test invalid JSON did not distinguish readable probe from malformed query'
  expect_journal_snapshot_failure wrong-boot-query kernel-journal-lookback-wrong-boot-id wrong-boot-lookback
  [ "$journal_probe_entry_count" = 1 ] && [ "$journal_lookback_status" = exit-0 ] ||
    die 'self-test wrong boot metadata did not distinguish readable probe from invalid query'
  expect_journal_snapshot_failure partial-range kernel-journal-lookback-nonzero partial-complete-range
  [ "$journal_capture_status" = failed ] && [ "$journal_capture_failure" = receipt-invalid-or-incomplete ] &&
    [[ "$journal_capture_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test partial complete-range capture was not retained as a failure'
  BENCH_TEST_JOURNAL_MODE=clean
  BENCH_TEST_BOOT_ID_MODE=readable

  # Every public scenario gets one detached, synthetic source commit per
  # revision.  This checks that a scenario is not silently collapsed back to
  # the old all-service comment probe before any Docker build is attempted.
  for scenario_case in "${benchmark_scenarios[@]}"; do
    scenario=$scenario_case
    baseline_commit=1111111111111111111111111111111111111111
    candidate_commit=2222222222222222222222222222222222222222
    baseline_checkout=$test_dir/scenarios/$scenario_case/baseline
    candidate_checkout=$test_dir/scenarios/$scenario_case/candidate
    prepare_checkout "$baseline_commit" "$baseline_checkout"
    prepare_checkout "$candidate_commit" "$candidate_checkout"
    validate_scenario_contract
    validate_checkout_contract "$baseline_checkout" baseline
    validate_checkout_contract "$candidate_checkout" candidate
    repetitions=1
    active_pair_index=1
    apply_equivalent_scenario 1 >/dev/null
    [ "${scenario_commit_by_revision[baseline]}" != "$baseline_commit" ] ||
      die "self-test scenario did not create a baseline measurement commit: $scenario_case"
    [ "${scenario_commit_by_revision[candidate]}" != "$candidate_commit" ] ||
      die "self-test scenario did not create a candidate measurement commit: $scenario_case"
    [ "${scenario_commit_by_revision[baseline]}" != "${scenario_commit_by_revision[candidate]}" ] ||
      die "self-test scenario collapsed baseline and candidate measurement commits: $scenario_case"
    scenario_path=${scenario_input_path[$scenario_case]}
    case "${scenario_transform[$scenario_case]}" in
      synthetic-child-commit-identical-tree) ;;
      prefix-json-whitespace)
        [ "$(head -c 2 "$baseline_checkout/$scenario_path")" = $'\n{' ] ||
          die "self-test JSON-whitespace scenario changed its required syntax prefix: $scenario_case"
        ;;
      *)
        grep -Fqx "$(scenario_probe_line)" "$baseline_checkout/$scenario_path" ||
          die "self-test scenario probe marker was not retained: $scenario_case"
        ;;
    esac
  done
  scenario=rust-leaf
  scenario_seen=0

  # Service cgroup membership must be the exact path or a descendant, never a
  # string prefix of an unrelated unit.  The systemctl fake requires -- before
  # the unit name, so this also covers option-safe systemctl invocation.
  : >"$BENCH_TEST_SYSTEMCTL_RECORD"
  BENCH_TEST_CGROUP_TEXT='0::/test.slice/benchmark.service'
  check_running_build_service || die 'self-test exact service cgroup was rejected'
  BENCH_TEST_CGROUP_TEXT='0::/test.slice/benchmark.service/worker'
  check_running_build_service || die 'self-test descendant service cgroup was rejected'
  BENCH_TEST_CGROUP_TEXT='0::/test.slice/benchmark.service.other'
  if check_running_build_service; then die 'self-test cgroup prefix collision was accepted'; fi
  [ "$gate_reason" = apply-not-running-inside-build-service ] || die 'self-test cgroup prefix collision had the wrong reason'
  : >"$BENCH_TEST_SYSTEMCTL_RECORD"
  BENCHMARK_SYSTEMD_SERVICE=-Hhost.service
  if check_running_build_service; then die 'self-test option-like systemd unit was accepted'; fi
  [ "$gate_reason" = invalid-benchmark-systemd-service ] || die 'self-test option-like unit had the wrong reason'
  [ ! -s "$BENCH_TEST_SYSTEMCTL_RECORD" ] || die 'self-test option-like unit reached systemctl'
  BENCHMARK_SYSTEMD_SERVICE=benchmark.service
  BENCH_TEST_CGROUP_TEXT='0::/test.slice/benchmark.service'
  safe_container_reference api-current || die 'self-test valid production container was rejected'
  safe_container_reference 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef ||
    die 'self-test full production container ID was rejected'
  if safe_container_reference -api-current; then die 'self-test option-like container was accepted'; fi
  if safe_container_reference $'api-current\nweb-current'; then die 'self-test control-containing container was accepted'; fi

  # A resource failure must happen before a Docker build and leave its reason.
  BENCH_TEST_MEMINFO=$'MemAvailable:    1024 kB\nSwapFree:        2097152 kB'
  output_dir=$test_dir/resource-failure
  baseline_commit=1111111111111111111111111111111111111111
  candidate_commit=2222222222222222222222222222222222222222
  cache_mode=warm
  repetitions=1
  measurement_order=baseline-first
  active_pair_index=1
  tag_list=()
  if ( run_apply ) >"$test_dir/resource-failure.out" 2>&1; then die 'self-test resource gate unexpectedly allowed apply'; fi
  grep -Fq 'mem-available-below-threshold' "$output_dir/failure.tsv" || die 'self-test resource failure was not preserved'
  if grep -Fq $'build\t' "$BENCH_TEST_DOCKER_RECORD"; then die 'self-test resource failure reached Docker build'; fi

  # A recent OOM is independently fatal before Docker.  This also proves the
  # journal observation is a gate, rather than a report-only diagnostic.
  BENCH_TEST_MEMINFO=$'MemAvailable:    4194304 kB\nSwapFree:        2097152 kB'
  BENCH_TEST_JOURNAL_MODE=oom
  output_dir=$test_dir/oom-failure
  if ( run_apply ) >"$test_dir/oom-failure.out" 2>&1; then die 'self-test OOM gate unexpectedly allowed apply'; fi
  grep -Fq 'recent-kernel-oom-observed' "$output_dir/failure.tsv" || die 'self-test OOM failure was not preserved'
  if grep -Fq $'build\t' "$BENCH_TEST_DOCKER_RECORD"; then die 'self-test OOM failure reached Docker build'; fi
  BENCH_TEST_JOURNAL_MODE=clean

  # Production-serving containers are mandatory and every read-only status
  # failure blocks Docker before a build.  The fake inspect accepts only the
  # three bounded fields used by the production gate.
  saved_health_containers=$BENCHMARK_PRODUCTION_HEALTH_CONTAINERS
  BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=
  : >"$BENCH_TEST_DOCKER_RECORD"
  output_dir=$test_dir/missing-container-list
  if ( run_apply ) >"$test_dir/missing-container-list.out" 2>&1; then die 'self-test accepted a missing production container list'; fi
  grep -Fq 'missing-production-health-containers' "$output_dir/failure.tsv" || die 'self-test missing container list was not preserved'
  if grep -Fq $'build\t' "$BENCH_TEST_DOCKER_RECORD"; then die 'self-test missing container list reached Docker build'; fi
  BENCHMARK_PRODUCTION_HEALTH_CONTAINERS=$saved_health_containers
  for health_mode in missing stopped no-healthcheck unhealthy wrong-project; do
    case "$health_mode" in
      missing) health_reason=production-container-unreadable ;;
      stopped) health_reason=production-container-not-running ;;
      no-healthcheck) health_reason=production-container-health-unestablished ;;
      unhealthy) health_reason=production-container-unhealthy ;;
      wrong-project) health_reason=production-container-wrong-compose-project ;;
    esac
    BENCH_TEST_CONTAINER_MODE=$health_mode
    : >"$BENCH_TEST_DOCKER_RECORD"
    output_dir=$test_dir/container-health-$health_mode
    if ( run_apply ) >"$test_dir/container-health-$health_mode.out" 2>&1; then die "self-test accepted $health_mode production container"; fi
    grep -Fq "$health_reason" "$output_dir/failure.tsv" || die "self-test $health_mode production container reason was not preserved"
    if grep -Fq $'build\t' "$BENCH_TEST_DOCKER_RECORD"; then die "self-test $health_mode production container reached Docker build"; fi
  done
  BENCH_TEST_CONTAINER_MODE=healthy

  # A Docker build error fails at the first invocation, preserves sanitized
  # evidence, and prevents later services/batches from progressing.
  : >"$BENCH_TEST_DOCKER_RECORD"
  BENCH_TEST_DOCKER_FAIL_BUILD=1
  output_dir=$test_dir/build-failure
  if ( run_apply ) >"$test_dir/build-failure.out" 2>&1; then die 'self-test Docker failure unexpectedly passed'; fi
  unset BENCH_TEST_DOCKER_FAIL_BUILD
  [ "$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 1 ] ||
    die 'self-test Docker failure did not stop after the first build'
  grep -Fq 'Docker build failed' "$output_dir/failure.tsv" || die 'self-test Docker failure was not preserved'
  find "$output_dir/evidence" -type f -name '*.build.log' -print -quit | grep -q . ||
    die 'self-test Docker failure did not preserve build evidence'

  # A complete warm fake run verifies ordering, namespace build arguments,
  # sanitized evidence, output preservation, and cleanup scope.
  reset_self_test_whole_lock warm
  BENCH_TEST_ARCHIVE_SCAN_LIMIT=12
  BENCH_TEST_ARCHIVE_SCAN_COUNT=0
  BENCH_TEST_MEMINFO=$'MemAvailable:    4194304 kB\nSwapFree:        2097152 kB'
  : >"$BENCH_TEST_DOCKER_RECORD"
  output_dir=$test_dir/warm-output
  cache_mode=warm
  tag_list=()
  run_apply >"$test_dir/warm-run.out" 2>&1
  trap - EXIT
  build_count=$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$build_count" -eq 48 ] || die "self-test warm state machine made $build_count builds, expected 48"
  awk -F '\t' '
    $1 == "build" {
      n++
      if ($2 !~ /^[a-z0-9]+([._-][a-z0-9]+)*:[0-9a-f]{40}$/) bad=1
      if ($3 == "" || $4 != $3 || $5 != "unset" || $6 == "") bad=1
      if ($2 ~ /-baseline-(warmup|measured-pair-[123])-/) {
        if (baseline_namespace == "") baseline_namespace = $3
        else if (baseline_namespace != $3) bad=1
        if (baseline_nonce == "") baseline_nonce = $6
        else if (baseline_nonce != $6) bad=1
      }
      if ($2 ~ /-candidate-(warmup|measured-pair-[123])-/) {
        if (candidate_namespace == "") candidate_namespace = $3
        else if (candidate_namespace != $3) bad=1
        if (candidate_nonce == "") candidate_nonce = $6
        else if (candidate_nonce != $6) bad=1
      }
      if (n <= 24 && ($2 !~ /-warmup-/ || $7 != "unchanged")) bad=1
      if (n > 24 && ($2 !~ /-measured-/ || $7 != "perturbed")) bad=1
    }
    END { exit !(n == 48 && !bad && baseline_namespace != "" && candidate_namespace != "" && baseline_namespace != candidate_namespace && baseline_nonce != "" && candidate_nonce != "" && baseline_nonce != candidate_nonce) }
  ' "$BENCH_TEST_DOCKER_RECORD" || die 'self-test warm namespace, stable cold nonce, repository grammar, or warm-up ordering regressed'
  [ "$(awk -F '\t' '$1 == "build" { seen[$3]=1 } END { for (key in seen) count++; print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 2 ] ||
    die 'self-test warm revisions did not receive distinct namespaces'
  grep -R -Fq 'fake-secret-must-not-persist' "$output_dir" && die 'self-test sanitizer persisted a secret-shaped line'
  grep -R -Fq '[redacted sensitive build output]' "$output_dir/evidence" || die 'self-test sanitizer evidence was not retained'
  grep -Fq $'production_health_containers\tapi-current,web-current' "$output_dir/metadata.tsv" ||
    die 'self-test production container metadata was not retained'
  grep -Fq $'production_container_health_policy\trunning; declared healthy healthcheck; compose project lagrange-station; bounded inspect fields only' "$output_dir/metadata.tsv" ||
    die 'self-test production container health policy metadata was not retained'
  grep -Fq $'benchmark_scenario\trust-leaf' "$output_dir/metadata.tsv" ||
    die 'self-test selected scenario metadata was not retained'
  grep -Fxq 'BUILD_CACHE_BENCHMARK_V5' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal evidence schema version was not retained'
  grep -Fq $'repetitions\t1' "$output_dir/metadata.tsv" ||
    die 'self-test default repetition metadata was not retained'
  grep -Fq $'measurement_order\tbaseline-first' "$output_dir/metadata.tsv" ||
    die 'self-test default pair-order metadata was not retained'
  grep -Fq $'baseline_product_kind\tsource-unclassified' "$output_dir/metadata.tsv" &&
    grep -Fq $'candidate_product_kind\tsource-unclassified' "$output_dir/metadata.tsv" &&
    grep -Fq $'baseline_layout_kind\tsource' "$output_dir/metadata.tsv" &&
    grep -Fq $'candidate_layout_kind\tsource' "$output_dir/metadata.tsv" ||
    die 'self-test actual product/layout identities were not retained in metadata'
  grep -Fq $'kernel_journal_probe\tLC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1; require exit-0, no-stderr, exactly-one valid current-boot kernel entry' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal readability probe metadata was not retained'
  grep -Fq $'kernel_journal_lookback\tLC_ALL=C bounded complete-stream collector invokes journalctl -k -b --no-pager -o json --since <fixed-first-gate-minus-1800s> --until <gate-utc> --no-tail; require EOF on both streams, child reaping, exit-0, no-stderr, valid bounded current-boot JSON, and a complete capture receipt' "$output_dir/metadata.tsv" ||
    die 'self-test complete kernel journal metadata was not retained'
  grep -Fq $'kernel_journal_lookback_seconds\t1800' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal fixed lookback metadata was not retained'
  if ! awk -F '\t' '
    NR == 1 {
      if (NF != 29 || $10 != "journal_probe_status" || $14 != "journal_lookback_status" || $19 != "journal_capture_status" || $24 != "journal_reason") bad=1
      next
    }
    NF != 29 { bad=1 }
    $9 != "0" || $10 != "exit-0" || $11 != "1" || $12 !~ /^[0-9a-f]{64}$/ || $13 !~ /^[0-9a-f]{64}$/ { bad=1 }
    $14 != "exit-0" || $15 != "0" || $16 != "0" || $17 !~ /^[0-9a-f]{64}$/ || $18 !~ /^[0-9a-f]{64}$/ { bad=1 }
    $19 != "complete" || $20 !~ /^[0-9a-f]{64}$/ || $21 != "none" { bad=1 }
    $22 != "2029-12-31T23:30:00Z" || $23 != "2030-01-01T00:00:00Z" || $24 != "established" { bad=1 }
    END { exit !(NR > 1 && !bad) }
  ' "$output_dir/batch-resources.tsv"; then
    die 'self-test bounded kernel journal status/count/hash/reason evidence was malformed'
  fi
  grep -Fq $'commit_transition_policy\teach measured pair appends a pair-indexed marker to the previous measured tree and commits a deterministic synthetic child so the temporary image OCI revision and embedded code commit match its tree; commit-only is the transition control' "$output_dir/metadata.tsv" ||
    die 'self-test synthetic commit transition policy was not retained'
  if ! awk -F '\t' '$1 == "benchmark_script_sha256" && $2 ~ /^[0-9a-f]{64}$/ { found=1 } END { exit !found }' "$output_dir/metadata.tsv"; then
    die 'self-test benchmark tool SHA metadata was not retained'
  fi
  if ! awk -F '\t' '
    NR == 1 { next }
    NF != 27 { bad=1 }
    $9 !~ /^[0-9]+$/ || $11 !~ /^[0-9]+$/ || $12 !~ /^sha256:[0-9a-f]{64}$/ || $13 !~ /^[0-9a-f]{40}$/ { bad=1 }
    $27 != "rust-leaf" { bad=1 }
    END { exit !(NR > 1 && !bad) }
  ' "$output_dir/service-results.tsv"; then
    die 'self-test image build, verification, and image identity evidence was malformed'
  fi
  if ! awk -F '\t' '
    NR == 1 { next }
    NF != 8 { bad=1 }
    $5 !~ /^[0-9a-f]{64}$/ || $6 !~ /^[0-9a-f]{64}$/ || $7 !~ /^[0-9a-f]{64}$/ { bad=1 }
    END { exit !(NR == 25 && !bad) }
  ' "$output_dir/instrumentation.tsv"; then
    die 'self-test Dockerfile instrumentation evidence was malformed'
  fi
  if ! awk -F '\t' '
    NR == 1 { next }
    NF != 9 { bad=1 }
    $5 !~ /^[0-9]+$/ || $6 !~ /^[0-9]+$/ || $7 !~ /^[0-9]+$/ || $8 !~ /^[0-9]+$/ || $9 !~ /^[0-9]+$/ { bad=1 }
    END { exit !(NR > 1 && !bad) }
  ' "$output_dir/resource-samples.tsv"; then
    die 'self-test sampled memory, swap, and disk evidence was malformed'
  fi
  if ! awk -F '\t' 'NR == 2 && $1 == "benchmark-build-samples" && $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ && $4 ~ /^[0-9]+$/ && $5 ~ /^[0-9]+$/ && $6 ~ /^[0-9]+$/ { found=1 } END { exit !found }' "$output_dir/peak-resources.tsv"; then
    die 'self-test peak resource summary was not retained'
  fi
  if ! awk -F '\t' '$1 == "preparation" || $1 == "warmup-builds" || $1 == "scenario-input-preparation" || $1 == "measured-image-builds" || $1 == "comparison-report" || $1 == "benchmark-overall" { seen[$1]=1 } END { exit !(seen["preparation"] && seen["warmup-builds"] && seen["scenario-input-preparation"] && seen["measured-image-builds"] && seen["comparison-report"] && seen["benchmark-overall"]) }' "$output_dir/phase-timings.tsv"; then
    die 'self-test preparation, execution, and comparison phase timing was not retained'
  fi
  if ! awk -F '\t' '$1 == "baseline" && $2 == "measured-pair-1" && $3 == "1111111111111111111111111111111111111111" && $4 != $3 { found=1 } END { exit !found }' "$output_dir/source-identities.tsv"; then
    die 'self-test measured source commit did not differ from its original baseline commit'
  fi
  [ "$(wc -l <"$output_dir/comparison.tsv")" -eq 13 ] || die 'self-test warm comparison lacks twelve measured records'
  awk -F '\t' 'NR == 1 { if (NF != 15 || $1 != "pair_index" || $2 != "pair_order") bad=1; next } NF != 15 || $1 != "1" || $2 != "baseline,candidate" { bad=1 } END { exit !(NR == 13 && !bad) }' "$output_dir/comparison.tsv" ||
    die 'self-test warm comparison report has malformed fields'
  [ "$(find "$output_dir/source-manifests" -maxdepth 1 -type f -name '*.v2' | wc -l)" -eq 4 ] ||
    die 'self-test source route did not publish one private V2 record per complete release'
  for manifest in "$output_dir"/source-manifests/*.v2; do
    [ -f "$manifest" ] && [ ! -L "$manifest" ] &&
      [ "$(head -n 1 -- "$manifest")" = LAGRANGE_RELEASE_MANIFEST_V2 ] &&
      [ "$(wc -l <"$manifest")" -eq 14 ] &&
      [ "$(stat -c %a -- "$manifest")" = 600 ] ||
      die 'self-test source private V2 publication/revalidation record was malformed'
  done
  if ! awk -F '\t' '
    NR == 1 { if (NF != 10 || $1 != "revision" || $3 != "product_kind" || $4 != "layout_kind") bad=1; next }
    NF != 10 || $3 != "source-unclassified" || $4 != "source" ||
      $5 !~ /^source-manifests\/(baseline|candidate)-(warmup|measured-pair-1)\.v2$/ ||
      $6 !~ /^[0-9a-f]{64}$/ || $7 !~ /^[0-9a-f]{64}$/ || $8 !~ /^[0-9a-f]{64}$/ ||
      $9 !~ /^[0-9a-f]{40}$/ || $10 !~ /^[a-z0-9]+([._-][a-z0-9]+)*$/ { bad=1 }
    END { exit !(NR == 5 && !bad) }
  ' "$output_dir/source-manifests.tsv"; then
    die 'self-test source manifest library and private V2 binding evidence was malformed'
  fi
  if ! awk -F '\t' '
    NR == 1 { if (NF != 9 || $3 != "product_kind" || $4 != "layout_kind") bad=1; next }
    NF != 9 || $3 != "source-unclassified" || $4 != "source" ||
      $5 !~ /^[0-9]+$/ || $6 !~ /^[0-9]+$/ || $7 !~ /^[0-9]+$/ || $8 != "PASS" { bad=1 }
    END { exit !(NR == 5 && !bad) }
  ' "$output_dir/release-totals.tsv"; then
    die 'self-test source whole-release timing evidence was malformed'
  fi
  if ! awk -F '\t' '
    NR == 1 { if (NF != 11 || $3 != "baseline_product_kind" || $7 != "baseline_release_ms") bad=1; next }
    NF != 11 || $1 != "1" || $2 != "baseline,candidate" ||
      $3 != "source-unclassified" || $4 != "source-unclassified" || $5 != "source" || $6 != "source" ||
      $7 !~ /^[0-9]+$/ || $8 !~ /^[0-9]+$/ || $9 !~ /^-?[0-9]+$/ || $9 != ($8 - $7) || $10 != "comparable" { bad=1 }
    END { exit !(NR == 2 && !bad) }
  ' "$output_dir/release-comparison.tsv"; then
    die 'self-test source whole-release comparison evidence was malformed'
  fi
  [ "$(wc -l <"$output_dir/common-phases.tsv")" -eq 89 ] ||
    die 'self-test source whole-release phase accounting omitted a setup, gate, consumer, or final V2 interval'
  # Both fake A/B checkouts have a real tracked index plus ignored NT tooling.
  # Archive requests must retain tracked NT bytes while never selecting the
  # synthetic virtualenv or Python cache, and selection must not disturb them.
  for checkout in "$baseline_checkout" "$candidate_checkout"; do
    [ -L "$checkout/nt/.venv/bin/python" ] &&
      [ "$(readlink -- "$checkout/nt/.venv/bin/python")" = ../python-fixture-target ] &&
      grep -Fxq 'ignored benchmark virtualenv bytes' "$checkout/nt/.venv/ignored.txt" &&
      grep -Fxq 'ignored benchmark pycache bytes' "$checkout/nt/__pycache__/ignored.pyc" ||
      die 'self-test tracked runtime selection modified ignored NT tooling'
    if "$real_git_bin" -C "$checkout" ls-files | grep -Eq '(^|/)(\.venv|__pycache__)(/|$)|\.py[cod]$'; then
      die 'self-test ignored NT tooling entered the tracked fixture index'
    fi
  done
  if grep -R -Eq '(^|/)(\.venv|__pycache__)(/|$)|\.py[cod]' "$output_dir/archive-requests"; then
    die 'self-test A/B source archive request selected ignored NT tooling'
  fi
  grep -R -Fq 'opt/lagrange/nt/' "$output_dir/archive-requests" ||
    die 'self-test A/B source archive requests omitted tracked NT payloads'
  if ( run_apply ) >"$test_dir/nonempty-output.out" 2>&1; then die 'self-test accepted a non-empty output directory'; fi
  [ "$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 48 ] ||
    die 'self-test non-empty output check started a build'
  first_nonce=$benchmark_nonce
  second_nonce=$(new_self_test_nonce)
  [ "$first_nonce" != "$second_nonce" ] || die 'self-test invocation nonce collision'
  [ "lagrange-cb-$first_nonce-baseline-measured-1" != "lagrange-cb-$second_nonce-baseline-measured-1" ] ||
    die 'self-test invocation tags are not disjoint'
  [ "lagrange-benchmark-$first_nonce-baseline-${baseline_commit:0:12}" != "lagrange-benchmark-$second_nonce-baseline-${baseline_commit:0:12}" ] ||
    die 'self-test invocation namespaces are not disjoint'
  foreign_tag="lagrange-cb-${second_nonce}-baseline-measured-1"
  cleanup_test_images
  remove_count=$(awk -F '\t' '$1 == "remove" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$remove_count" -eq 48 ] || die 'self-test cleanup did not remove exactly this warm invocation tags'
  grep -Fq "$foreign_tag" "$BENCH_TEST_DOCKER_RECORD" && die 'self-test cleanup touched a different invocation tag'
  cleanup_temp_checkouts
  [ -f "$output_dir/metadata.tsv" ] && [ -d "$output_dir/evidence" ] || die 'self-test cleanup removed preserved warm evidence'

  # G2 permits at most three warm pairs. The order is a fixed anti-drift
  # schedule, not a caller-provided arbitrary sequence, and each pair gets a
  # new deterministic synthetic child commit after the prior measured tree.
  repetitions=2
  measurement_order=baseline-first
  if ( validate_measurement_protocol ) >/dev/null 2>&1; then
    die 'self-test accepted repeated non-alternating warm measurements'
  fi
  repetitions=2
  measurement_order=alternating
  cache_mode=cold
  if ( validate_measurement_protocol ) >/dev/null 2>&1; then
    die 'self-test accepted repeated cold measurements'
  fi
  cache_mode=warm
  repetitions=3
  measurement_order=alternating
  [ "$(pair_order 1)" = baseline,candidate ] || die 'self-test pair one order regressed'
  [ "$(pair_order 2)" = candidate,baseline ] || die 'self-test pair two order regressed'
  [ "$(pair_order 3)" = baseline,candidate ] || die 'self-test pair three order regressed'
  # The preceding default warm fixture already exercised all twelve services.
  # Limit this additional scheduling-only matrix to two dissimilar service
  # classes so the no-daemon test stays bounded while still checking each pair
  # transition, reverse order, cache namespace, and comparison publication.
  saved_services=("${services[@]}")
  services=(api-server web)
  : >"$BENCH_TEST_DOCKER_RECORD"
  output_dir=$test_dir/alternating-output
  reset_self_test_whole_lock alternating
  BENCH_TEST_ARCHIVE_SCAN_LIMIT=2
  BENCH_TEST_ARCHIVE_SCAN_COUNT=0
  tag_list=()
  run_apply >"$test_dir/alternating-run.out" 2>&1
  trap - EXIT
  build_count=$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$build_count" -eq 16 ] || die "self-test alternating state machine made $build_count builds, expected 16"
  if ! awk -F '\t' '
    $1 == "build" {
      n++
      if (n <= 4) next
      if ($2 !~ /-(baseline|candidate)-measured-pair-[123]-(api-server|web):[0-9a-f]{40}$/) bad=1
      side = ($2 ~ /-baseline-measured-pair-[123]-/ ? "baseline" : "candidate")
      pair = $2
      sub(/^.*-measured-pair-/, "", pair)
      sub(/-(api-server|web):[0-9a-f]{40}$/, "", pair)
      if (pair !~ /^[123]$/) bad=1
      group = int((n - 5) / 2) + 1
      expected_side[group] = (group == 1 || group == 4 || group == 5 ? "baseline" : "candidate")
      expected_pair[group] = (group <= 2 ? 1 : (group <= 4 ? 2 : 3))
      if (side != expected_side[group] || pair != expected_pair[group]) bad=1
    }
    END { exit !(n == 16 && !bad) }
  ' "$BENCH_TEST_DOCKER_RECORD"; then
    die 'self-test alternating pair order or pair-indexed tag contract regressed'
  fi
  grep -Fq $'repetitions\t3' "$output_dir/metadata.tsv" || die 'self-test alternating repetition metadata was not retained'
  grep -Fq $'measurement_order\talternating' "$output_dir/metadata.tsv" || die 'self-test alternating order metadata was not retained'
  [ "$(wc -l <"$output_dir/comparison.tsv")" -eq 7 ] || die 'self-test alternating comparison lacks all pair records'
  if ! awk -F '\t' 'NR == 1 { next } $1 ~ /^[123]$/ && (($1 == 1 && $2 == "baseline,candidate") || ($1 == 2 && $2 == "candidate,baseline") || ($1 == 3 && $2 == "baseline,candidate")) { count[$1]++ } END { exit !(count[1] == 2 && count[2] == 2 && count[3] == 2) }' "$output_dir/comparison.tsv"; then
    die 'self-test alternating comparison pairing is malformed'
  fi
  if ! awk -F '\t' '
    NR == 1 { if (NF != 11) bad=1; next }
    NF != 11 || $1 !~ /^[123]$/ || $3 != "source-unclassified" || $4 != "source-unclassified" ||
      $5 != "source" || $6 != "source" || $7 != "-" || $8 != "-" || $9 != "-" ||
      $10 != "self-test-subset-not-a-release" { bad=1 }
    END { exit !(NR == 4 && !bad) }
  ' "$output_dir/release-comparison.tsv"; then
    die 'self-test two-service scheduling fixture was mislabeled as whole-release evidence'
  fi
  [ "$(wc -l <"$output_dir/release-totals.tsv")" -eq 1 ] &&
    [ "$(wc -l <"$output_dir/source-manifests.tsv")" -eq 1 ] &&
    [ -z "$(find "$output_dir/source-manifests" -type f -print -quit)" ] ||
    die 'self-test two-service scheduling fixture emitted a release total or private V2 claim'
  cleanup_test_images
  remove_count=$(awk -F '\t' '$1 == "remove" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$remove_count" -eq 16 ] || die 'self-test alternating cleanup did not remove exactly this invocation tags'
  cleanup_temp_checkouts
  services=("${saved_services[@]}")

  # Cold runs have fresh namespaces and stable earliest-native nonces but no
  # warm-up scenario or comparison delta. They remain fake Docker invocations.
  : >"$BENCH_TEST_DOCKER_RECORD"
  output_dir=$test_dir/cold-output
  reset_self_test_whole_lock cold
  BENCH_TEST_ARCHIVE_SCAN_LIMIT=2
  BENCH_TEST_ARCHIVE_SCAN_COUNT=0
  cache_mode=cold
  repetitions=1
  measurement_order=baseline-first
  tag_list=()
  run_apply >"$test_dir/cold-run.out" 2>&1
  trap - EXIT
  build_count=$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$build_count" -eq 24 ] || die "self-test cold state machine made $build_count builds, expected 24"
  awk -F '\t' '$1 == "build" { count++; if ($2 !~ /^[a-z0-9]+([._-][a-z0-9]+)*:[0-9a-f]{40}$/ || $3 == "" || $4 != $3 || $5 != "unset" || $6 !~ /^lagrange-benchmark-cold-/ || $2 ~ /warmup/) bad=1 } END { exit !(count == 24 && !bad) }' \
    "$BENCH_TEST_DOCKER_RECORD" || die 'self-test cold namespace, repository grammar, or stable nonce contract regressed'
  grep -Fq 'not-comparable' "$output_dir/comparison.tsv" || die 'self-test cold run claimed a warm comparison'
  [ "$(find "$output_dir/source-manifests" -maxdepth 1 -type f -name '*.v2' | wc -l)" -eq 2 ] &&
    [ "$(wc -l <"$output_dir/source-manifests.tsv")" -eq 3 ] &&
    [ "$(wc -l <"$output_dir/release-totals.tsv")" -eq 3 ] ||
    die 'self-test cold source run omitted a complete side V2 or whole-release total'
  if ! awk -F '\t' 'NR == 2 && NF == 11 && $1 == "1" && $2 == "baseline,candidate" &&
      $3 == "source-unclassified" && $4 == "source-unclassified" && $5 == "source" && $6 == "source" &&
      $7 == "-" && $8 == "-" && $9 == "-" && $10 == "not-comparable" { found=1 }
      END { exit !(NR == 2 && found) }' "$output_dir/release-comparison.tsv"; then
    die 'self-test cold whole-release evidence was not explicitly marked non-comparable'
  fi
  cleanup_test_images
  remove_count=$(awk -F '\t' '$1 == "remove" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$remove_count" -eq 24 ] || die 'self-test cold cleanup did not remove exactly this invocation tags'
  cleanup_temp_checkouts
  [ -f "$output_dir/source-identities.tsv" ] || die 'self-test cold evidence was not preserved'

  # The C route is deliberately exercised separately from A/B source fallback:
  # its own clean helper/config/producer must initialize a fresh release state,
  # prepare shared bundles once, issue one-service Compose calls for all twelve
  # temporary tags, and publish only a private strict V2 document.
  BENCH_TEST_COMMON_LAYOUT=1
  BENCH_TEST_COMMON_RECORD=$test_dir/common-route.tsv
  : >"$BENCH_TEST_COMMON_RECORD"
  export BENCH_TEST_COMMON_LAYOUT BENCH_TEST_COMMON_RECORD
  cache_mode=warm
  repetitions=1
  measurement_order=baseline-first
  scenario=rust-leaf
  baseline_commit=1111111111111111111111111111111111111111
  candidate_commit=2222222222222222222222222222222222222222
  BENCHMARK_SYSTEMD_MANAGER=system
  BENCHMARK_RESEARCH_EXCEPTION='research-worker=image-only;reason=approved-existing-incident;expires=none'
  : >"$BENCH_TEST_DOCKER_RECORD"
  reset_self_test_whole_lock common-prep-failure
  output_dir=$test_dir/common-prep-failure
  tag_list=()
  BENCH_TEST_COMMON_FAIL_PREP_SERVICE=api-server
  if ( run_apply ) >"$test_dir/common-prep-failure.out" 2>&1; then
    die 'self-test common-C producer failure unexpectedly passed'
  fi
  unset BENCH_TEST_COMMON_FAIL_PREP_SERVICE
  grep -Fq 'common-C artifact preparation failed' "$output_dir/failure.tsv" ||
    die 'self-test common-C producer failure was not preserved'
  [ "$(awk -F '\t' '$1 == "compose" && $2 == "api-server" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 0 ] ||
    die 'self-test common-C producer failure reached its failed Rust consumer'
  [ "$(awk -F '\t' '$1 == "compose" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 2 ] ||
    die 'self-test common-C producer failure did not stop after the preceding DB services'

  : >"$BENCH_TEST_DOCKER_RECORD"
  reset_self_test_whole_lock common-compose-failure
  output_dir=$test_dir/common-compose-failure
  tag_list=()
  BENCH_TEST_COMMON_FAIL_SERVICE=web
  if ( run_apply ) >"$test_dir/common-compose-failure.out" 2>&1; then
    die 'self-test common-C Compose failure unexpectedly passed'
  fi
  unset BENCH_TEST_COMMON_FAIL_SERVICE
  grep -Fq 'common-C Compose build failed for baseline/warmup/web (exit=88;' "$output_dir/failure.tsv" ||
    die 'self-test common-C Compose failure did not preserve its real exit status'
  [ "$(awk -F '\t' '$1 == "compose" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 4 ] ||
    die 'self-test common-C Compose failure did not stop at the failed web build'
  [ -z "$(find "$output_dir/common-manifests" -type f -print -quit)" ] ||
    die 'self-test common-C Compose failure published a private V2 manifest'

  : >"$BENCH_TEST_DOCKER_RECORD"
  reset_self_test_whole_lock common-verify-failure
  output_dir=$test_dir/common-verify-failure
  tag_list=()
  BENCH_TEST_COMMON_FAIL_VERIFY_SERVICE=api-server
  if ( run_apply ) >"$test_dir/common-verify-failure.out" 2>&1; then
    die 'self-test common-C strict image verification failure unexpectedly passed'
  fi
  unset BENCH_TEST_COMMON_FAIL_VERIFY_SERVICE
  grep -Fq 'common-C strict saved-image verification failed' "$output_dir/failure.tsv" ||
    die 'self-test common-C strict verification failure was not preserved'
  [ -z "$(find "$output_dir/common-manifests" -type f -print -quit)" ] ||
    die 'self-test common-C verification failure published a private V2 manifest'

  : >"$BENCH_TEST_DOCKER_RECORD"
  # Failure-path assertions above retain their own output evidence.  Start a
  # fresh call record for the complete release so its one parent lock and four
  # inherited fresh-helper initializations are counted exactly, not diluted by
  # the deliberately failing predecessor cases.
  : >"$BENCH_TEST_COMMON_RECORD"
  reset_self_test_whole_lock common-success
  output_dir=$test_dir/common-output
  tag_list=()
  run_apply >"$test_dir/common-run.out" 2>&1
  trap - EXIT
  [ "$(awk -F '\t' '$1 == "compose" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")" -eq 48 ] ||
    die 'self-test common-C warm route did not make four complete twelve-service Compose series'
  if ! awk -F '\t' '
    $1 == "compose" {
      count[$2]++
      if ($3 !~ /\/inputs\/image-only-compose\.env$/ || $4 !~ /\/deploy\/compose\/compose\.yml$/ ||
          $5 !~ /\/common-compose-overrides\/.*-images\.json$/) bad=1
      if ($2 == "db-role-bootstrap" || $2 == "db-migrate" || $2 == "web") {
        if ($6 != "-") bad=1
      } else if ($6 !~ /\/common-state-.*\/overrides\/[a-z0-9-]+\.json$/) bad=1
    }
    END {
      expected["db-role-bootstrap"]; expected["db-migrate"]; expected["api-server"]; expected["web"]
      expected["research-worker"]; expected["recommendation-runner"]; expected["candidate-runner"]
      expected["owner-beta-runner"]; expected["owner-equity-v2-runner"]; expected["nt-backtest-worker-1"]
      expected["nt-backtest-worker-2"]; expected["paper-scheduler"]
      for (service in expected) if (count[service] != 4) bad=1
      exit bad
    }
  ' "$BENCH_TEST_DOCKER_RECORD"; then
    die 'self-test common-C Compose argv did not retain base, twelve-image, and Rust-only artifact override structure'
  fi
  [ "$(awk -F '\t' '$1 == "producer" && $2 == "new" && $3 == "D2" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] &&
    [ "$(awk -F '\t' '$1 == "producer" && $2 == "reuse" && $3 == "D2" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] &&
    [ "$(awk -F '\t' '$1 == "producer" && $2 == "new" && $3 == "D5" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] &&
    [ "$(awk -F '\t' '$1 == "producer" && $2 == "reuse" && $3 == "D5" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] ||
    die 'self-test common-C shared producer reuse was not retained for D2/D5'
  [ "$(awk -F '\t' '$1 == "lock" && $2 == "parent" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 1 ] &&
    [ "$(awk -F '\t' '$1 == "lock" && $2 == "inherited" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] ||
    die 'self-test common-C release shells did not retain one parent whole-run lock and four inherited inits'
  [ "$(awk -F '\t' '$1 == "init" { count++ } END { print count + 0 }' "$BENCH_TEST_COMMON_RECORD")" -eq 4 ] ||
    die 'self-test common-C did not initialize each warm-up/current synthetic commit in a fresh shell'
  [ "$(wc -l <"$output_dir/common-layouts.tsv")" -eq 3 ] || die 'self-test common-C clean helper/layout identities were not retained'
  if ! awk -F '\t' 'NR == 1 { if (NF != 7 || $2 != "product_kind" || $3 != "layout_kind") bad=1; next }
      NF != 7 || $2 != "C" || $3 != "common" || $4 !~ /^[0-9a-f]{64}$/ || $5 !~ /^[0-9a-f]{64}$/ ||
      $6 !~ /^[0-9a-f]{64}$/ || $7 !~ /^[0-9a-f]{64}$/ { bad=1 }
      END { exit !(NR == 3 && !bad) }' "$output_dir/common-layouts.tsv"; then
    die 'self-test common-C clean source/helper/config/producer binding evidence was malformed'
  fi
  [ "$(find "$output_dir/common-manifests" -maxdepth 1 -type f -name '*.v2' | wc -l)" -eq 4 ] ||
    die 'self-test common-C did not publish one private V2 record per complete release'
  # Check every private V2 record directly.  A `find | grep -q` pipeline
  # makes find report SIGPIPE under pipefail after grep accepts its first file,
  # turning a valid complete publication into a false harness failure.
  for manifest in "$output_dir"/common-manifests/*.v2; do
    [ -f "$manifest" ] && [ ! -L "$manifest" ] &&
      [ "$(head -n 1 -- "$manifest")" = LAGRANGE_RELEASE_MANIFEST_V2 ] &&
      [ "$(wc -l <"$manifest")" -eq 14 ] &&
      [ "$(stat -c %a -- "$manifest")" = 600 ] ||
      die 'self-test common-C private V2 publication/revalidation record was malformed'
  done
  [ "$(wc -l <"$output_dir/release-totals.tsv")" -eq 5 ] ||
    die 'self-test common-C whole-release elapsed records were not retained'
  if ! awk -F '\t' 'NR == 1 { if (NF != 9 || $3 != "product_kind" || $4 != "layout_kind") bad=1; next }
      NF != 9 || $3 != "C" || $4 != "common" || $5 !~ /^[0-9]+$/ || $6 !~ /^[0-9]+$/ ||
      $7 !~ /^[0-9]+$/ || $8 != "PASS" { bad=1 }
      END { exit !(NR == 5 && !bad) }' "$output_dir/release-totals.tsv"; then
    die 'self-test common-C whole-release timing evidence was malformed'
  fi
  if ! awk -F '\t' 'NR == 2 && NF == 11 && $1 == "1" && $2 == "baseline,candidate" &&
      $3 == "C" && $4 == "C" && $5 == "common" && $6 == "common" &&
      $7 ~ /^[0-9]+$/ && $8 ~ /^[0-9]+$/ && $9 ~ /^-?[0-9]+$/ && $9 == ($8 - $7) && $10 == "comparable" { found=1 }
      END { exit !(NR == 2 && found) }' "$output_dir/release-comparison.tsv"; then
    die 'self-test common-C whole-release comparison evidence was malformed'
  fi
  [ "$(wc -l <"$output_dir/common-phases.tsv")" -eq 125 ] ||
    die 'self-test common-C whole-release phase accounting omitted a preparation, producer, consumer, gate, or V2 interval'
  [ "$(wc -l <"$output_dir/source-manifests.tsv")" -eq 1 ] &&
    [ -z "$(find "$output_dir/source-manifests" -type f -print -quit)" ] ||
    die 'self-test common-C route emitted source-fallback manifest evidence'
  if ! awk -F '\t' 'NR > 1 {
    if (NF != 11 || $3 !~ /^D[1-7]$/ || $4 !~ /^[A-Za-z0-9_.-]+$/ ||
        $5 !~ ("/producers/" $3 "/" $4 "/artifact\\.json$") || $6 !~ /^[0-9a-f]{64}$/ ||
        $7 !~ ("/producers/" $3 "/" $4 "/cargo\\.jsonl$") || $8 !~ /^[0-9a-f]{64}$/ ||
        $9 !~ /^[0-9]+$/ || $10 !~ ("/bundles/" $3 "$") || $11 !~ /^[0-9a-f]{64}$/) bad=1
    rows++
  } END { exit !(rows == 68 && !bad) }' "$output_dir/common-producers.tsv"; then
    die 'self-test common-C producer/Cargo receipt evidence was incomplete or malformed'
  fi
  grep -Fq $'build_service_manager\tsystem' "$output_dir/metadata.tsv" &&
    grep -Fq $'research_exception\tresearch-worker=image-only;reason=approved-existing-incident;expires=none' "$output_dir/metadata.tsv" &&
    grep -Fq $'baseline_product_kind\tC' "$output_dir/metadata.tsv" &&
    grep -Fq $'candidate_product_kind\tC' "$output_dir/metadata.tsv" &&
    grep -Fq 'BENCHMARK_SYSTEMD_SERVICE->RELEASE_BUILD_SYSTEMD_UNIT' "$output_dir/metadata.tsv" ||
    die 'self-test common-C explicit helper gate environment mapping was not retained'
  grep -Fq $'image_only_compose_env\tinputs/image-only-compose.env' "$output_dir/metadata.tsv" &&
    grep -Fq $'image_only_compose_env_sha256\t'"$benchmark_compose_env_sha256_expected" "$output_dir/metadata.tsv" &&
    grep -Fq $'image_only_compose_env_policy\tprivate-external-input; mode-0600; inactive-research-entitlement-sentinel-only; never copied from an operational environment' "$output_dir/metadata.tsv" ||
    die 'self-test common-C private image-only Compose env evidence was not retained'
  if ( run_apply ) >"$test_dir/common-nonempty-output.out" 2>&1; then
    die 'self-test common-C accepted a non-empty output directory for a second private V2 publication'
  fi
  cleanup_test_images
  remove_count=$(awk -F '\t' '$1 == "remove" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$remove_count" -eq 48 ] ||
    die 'self-test common-C cleanup did not remove exactly the parent-registered temporary image references'
  cleanup_temp_checkouts
  unset BENCH_TEST_COMMON_LAYOUT BENCH_TEST_COMMON_RECORD BENCHMARK_RESEARCH_EXCEPTION

  echo 'BUILD_CACHE_BENCHMARK_SELF_TEST: PASS (fake Docker/Git/systemd/resource observations only; no daemon or Rust compilation)'
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--apply|--self-test)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --apply, or --self-test'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --warm|--cold)
      [ "$cache_mode_seen" -eq 0 ] || die 'choose exactly one cache mode: --warm or --cold'
      cache_mode=${1#--}
      cache_mode_seen=1
      shift
      ;;
    --scenario)
      [ "$#" -ge 2 ] || die '--scenario needs a value'
      [ "$scenario_seen" -eq 0 ] || die '--scenario may be provided only once'
      scenario=$2
      scenario_seen=1
      shift 2
      ;;
    --repetitions)
      [ "$#" -ge 2 ] || die '--repetitions needs a value'
      [ "$repetitions_seen" -eq 0 ] || die '--repetitions may be provided only once'
      repetitions=$2
      repetitions_seen=1
      shift 2
      ;;
    --order)
      [ "$#" -ge 2 ] || die '--order needs a value'
      [ "$measurement_order_seen" -eq 0 ] || die '--order may be provided only once'
      measurement_order=$2
      measurement_order_seen=1
      shift 2
      ;;
    --baseline-commit|--candidate-commit|--output-dir)
      [ "$#" -ge 2 ] || die "$1 needs a value"
      case "$1" in
        --baseline-commit) baseline_commit=$2 ;;
        --candidate-commit) candidate_commit=$2 ;;
        --output-dir) output_dir=$2 ;;
      esac
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) die "unknown option: $1" ;;
  esac
done

if [ "$mode" = self-test ]; then
  run_self_test
  exit 0
fi

[ -n "$baseline_commit" ] || die '--baseline-commit is required'
[ -n "$candidate_commit" ] || die '--candidate-commit is required'
[ -n "$output_dir" ] || die '--output-dir is required'
validate_output_path
validate_revisions
validate_service_contract
validate_scenario_contract
validate_measurement_protocol

if [ "$mode" = plan ]; then
  echo 'BUILD_CACHE_BENCHMARK_PLAN mode=plan'
  echo "  baseline_commit=$baseline_commit"
  echo "  candidate_commit=$candidate_commit"
  echo "  output_dir=$output_dir"
  echo "  cache_mode=$cache_mode"
  echo "  repetitions=$repetitions"
  echo "  order=$measurement_order"
  echo "  scenario=$scenario (${scenario_transform[$scenario]}; input=${scenario_input_path[$scenario]}; expected_scope=${scenario_expected_scope[$scenario]})"
  echo '  namespaces=every Docker build receives an explicit --build-arg BUILDKIT_CACHE_MOUNT_NS unique to this invocation and revision'
  echo '  warm=each revision warms unchanged temporary source once, then each pair receives the same deterministic pair-indexed scenario commit before measured builds'
  echo '  cold=fresh revision namespaces plus a stable side nonce consumed before native setup; one independent cold pair only, not a warm changed-source comparison'
  echo '  paired_order=baseline-first or candidate-first for one pair; alternating is B/C,C/B,B/C for up to three warm pairs'
  echo '  instrumentation=A/B temporary Dockerfile copies add Cargo verbosity and a stable pre-native nonce; original Dockerfiles/commands/binary sets remain unchanged; C helper/config/producer remains clean'
  echo '  compose_env=--apply creates one private external image-only env with the inactive research-entitlement sentinel; no checkout or operational .env is read'
  echo '  services=all twelve release services, batches up to three, sequential one-service Docker builds'
  echo "  resource_gates=MemAvailable >= ${default_min_mem_available_kib} KiB; SwapFree >= ${default_min_swap_free_kib} KiB; recent OOM, prior compiler/exit, systemd and health checks fail closed"
  echo '  apply_prerequisite=already-running low-priority systemd service containing this process plus explicit read-only health units; the harness never starts it'
  echo '  outputs=sanitized build evidence, source/probe/instrumentation hashes, tool identities, per-image build/Cargo/inspection timings, sampled resource minima, phase timings, package/cache state'
  echo '  timing_limits=Cargo RUN duration is not linker time; benchmark-local strict V2 image inspection and publication are retained separately from the official release manifest'
  echo '  prohibited=production checkout mutation, production tag/cache use, rollout, provider access, lifecycle commands, credential reads, global cache pruning'
  echo 'PLAN_ONLY: no Docker command, Rust compilation, checkout, output write, or background work made'
else
  run_apply
fi
