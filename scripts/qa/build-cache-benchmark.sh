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

mode=plan
mode_seen=0
cache_mode=warm
cache_mode_seen=0
scenario=rust-leaf
scenario_seen=0
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
failure_report=
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
journal_observation_reason=unavailable

declare -a tag_list=()
declare -A instrumented_dockerfile=()
declare -A source_dockerfile_hash=()
declare -A instrumented_dockerfile_hash=()
declare -A instrumentation_patch_hash=()
declare -A instrumentation_transform=()
declare -A service_toolchain_identity=()
declare -A source_identity_by_revision_scenario=()
declare -A scenario_commit_by_revision=()

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-cache-benchmark.sh [--plan|--apply|--self-test]
       --baseline-commit <40 lowercase hex>
       --candidate-commit <40 lowercase hex>
       --output-dir <absolute path>
       [--warm|--cold]
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
namespaces plus --no-cache and records independent cold measurements; it makes
no warm comparison claim.  The default scenario is rust-leaf.  Run one
scenario per invocation so its preparation, build, and verification timing
cannot be hidden inside another scenario.

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

parse_build_events() {
  local log=$1 cargo_mode=$2 revision=$3 vertex_header done_duration duration_ms
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
}

scenario_is_known() {
  local known
  for known in "${benchmark_scenarios[@]}"; do
    [ "$known" = "$1" ] && return 0
  done
  return 1
}

scenario_probe_line() {
  case "$scenario" in
    rust-leaf|rust-common-library|build-script)
      printf '%s' "// LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914 scenario=$scenario"
      ;;
    lockfile-comment|python-only)
      printf '%s' "# LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914 scenario=$scenario"
      ;;
    web-only)
      printf '%s' "// LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914 scenario=web-only"
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
  local checkout=$1 revision=$2 input_commit=$3 path tree commit probe_tmp line
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$input_commit" ] ||
    die "$revision temporary checkout changed before scenario instrumentation"
  path=${scenario_input_path[$scenario]}
  if [ "$path" != - ]; then
    case "${scenario_transform[$scenario]}" in
      append-rust-comment|append-toml-comment|append-tsx-comment|append-python-comment)
        line=$(scenario_probe_line)
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
    GIT_AUTHOR_DATE='2000-01-01T00:00:00Z' \
    GIT_COMMITTER_NAME=build-cache-benchmark \
    GIT_COMMITTER_EMAIL=build-cache-benchmark@example.invalid \
    GIT_COMMITTER_DATE='2000-01-01T00:00:00Z' \
    git -C "$checkout" commit-tree "$tree" -p "$input_commit" \
      -m "build-cache benchmark $scenario $revision") ||
    die "$revision scenario commit could not be created"
  is_exact_commit "$commit" || die "$revision scenario commit is not an exact Git SHA"
  git -C "$checkout" checkout --quiet --detach --force "$commit" ||
    die "$revision scenario commit could not be checked out"
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] ||
    die "$revision scenario checkout does not match its recorded commit"
  [ -z "$(git -C "$checkout" status --porcelain=v1 --untracked-files=all)" ] ||
    die "$revision scenario checkout is not clean after instrumentation"
  scenario_commit_by_revision[$revision]=$commit
}

apply_equivalent_scenario() {
  local checkout revision input_commit
  # This specification is byte-for-byte common.  The two revision-relative
  # patches and synthetic commits can legitimately hash differently because
  # their surrounding source differs, so both exact patches and the common
  # transformation spec are retained.
  for checkout in "$baseline_checkout" "$candidate_checkout"; do
    case "$checkout" in
      "$baseline_checkout") revision=baseline; input_commit=$baseline_commit ;;
      *) revision=candidate; input_commit=$candidate_commit ;;
    esac
    create_scenario_commit "$checkout" "$revision" "$input_commit"
    printf 'BENCHMARK_PROBE_APPLIED revision=%s scenario=%s source=%s commit=%s\n' \
      "$revision" "$scenario" "${scenario_input_path[$scenario]}" "${scenario_commit_by_revision[$revision]}"
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
    while :; do
      if ! record_resource_sample "$sampler_file" "$revision" "$scenario_name" "$service" sample; then
        printf '%s\n' 'resource-sample-unavailable' >"$sampler_failure_file"
        exit 0
      fi
      sleep 1
    done
  ) &
  sampler_pid=$!
}

stop_resource_sampler() {
  local status=0
  [ -n "${sampler_pid:-}" ] || return 0
  kill "$sampler_pid" >/dev/null 2>&1 || true
  wait "$sampler_pid" >/dev/null 2>&1 || true
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
  local tag=$1 commit=$2 inspected image_id image_revision extra start_ms end_ms verify_ms
  start_ms=$(now_ms)
  if ! inspected=$(docker image inspect --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' -- "$tag" 2>/dev/null); then
    return 1
  fi
  end_ms=$(now_ms)
  verify_ms=$((end_ms - start_ms))
  case "$inspected" in *$'\n'*|*$'\r'*|*[$'\001'-$'\037'$'\177']*) return 1 ;; esac
  image_id= image_revision= extra=
  IFS='|' read -r image_id image_revision extra <<<"$inspected"
  [ -z "$extra" ] || return 1
  [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || return 1
  [ "$image_revision" = "$commit" ] || return 1
  is_decimal "$verify_ms" || return 1
  printf '%s\t%s\t%s' "$verify_ms" "$image_id" "$image_revision"
}

build_one_service() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 service=$5 index=$6 namespace=$7
  local tag raw_log evidence_rel evidence_log start_ms end_ms total_ms cargo_ms cargo_cache status
  local image_verification verify_ms image_id image_revision
  local -a args=()

  tag="lagrange-cb-${benchmark_nonce}-${revision}-${measurement_phase}-${index}"
  safe_benchmark_repository "$tag" || die 'generated benchmark image repository is not valid lowercase Docker grammar'
  raw_log=$tmp_dir/${revision}-${measurement_phase}-${index}-${service}.build.log.raw
  evidence_rel="evidence/${revision}-${measurement_phase}-${index}-${service}.build.log"
  evidence_log=$output_dir/$evidence_rel
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
    -f "${instrumented_dockerfile[$revision:$service]}"
    -t "$tag"
  )
  [ "$cache_mode" = cold ] && args+=(--no-cache)
  args+=("$checkout")

  if ! record_resource_sample "$resource_samples_report" "$revision" "$measurement_phase" "$service" before-build; then
    failure_stage=resource-sample
    die "resource sampling was unavailable before $revision/$measurement_phase/$service"
  fi
  if ! start_resource_sampler "$revision" "$measurement_phase" "$service"; then
    failure_stage=resource-sample
    die "resource sampler could not start for $revision/$measurement_phase/$service"
  fi
  start_ms=$(now_ms)
  if CARGO_BUILD_JOBS=2 DOCKER_BUILDKIT=1 docker "${args[@]}" >"$raw_log" 2>&1; then
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
  if ! parse_build_events "$raw_log" "${service_cargo_mode[$service]}" "$revision"; then
    failure_stage=build-evidence
    die "invalid Cargo/BuildKit evidence for $revision/$measurement_phase/$service: $BENCH_PARSE_ERROR"
  fi
  cargo_ms=${BENCH_CARGO_STEP_MS:--}
  cargo_cache=$(derive_cargo_cache_state "${service_cargo_mode[$service]}")
  if ! image_verification=$(inspect_benchmark_image "$tag" "$commit"); then
    failure_stage=image-verification
    die "temporary image verification failed for $revision/$measurement_phase/$service"
  fi
  IFS=$'\t' read -r verify_ms image_id image_revision <<<"$image_verification"
  append_result "$revision" "$measurement_phase" "$service" "$index" "$tag" "$namespace" "$commit" \
    "${source_identity_by_revision_scenario[$revision:$measurement_phase]}" "$total_ms" "$cargo_ms" \
    "$verify_ms" "$image_id" "$image_revision" "$cargo_cache" "$evidence_rel" "$scenario"
  printf 'BENCHMARK_SERVICE PASS revision=%s measurement_phase=%s selected_scenario=%s service=%s image_build_ms=%s cargo_ms=%s image_verification_ms=%s compiler_vertex=%s compiler_state=%s cargo_cache=%s compiled=%s fresh=%s\n' \
    "$revision" "$measurement_phase" "$scenario" "$service" "$total_ms" "$cargo_ms" "$verify_ms" "$BENCH_COMPILER_VERTEX" \
    "$BENCH_COMPILER_VERTEX_STATE" "$cargo_cache" "$(csv_or_dash "$BENCH_COMPILED_PACKAGES")" \
    "$(csv_or_dash "$BENCH_FRESH_PACKAGES")"
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
  local unit=$1 property=$2 value
  safe_unit_name "$unit" || return 1
  value=$(systemctl show -p "$property" --value -- "$unit" 2>/dev/null | head -n 1) || return 1
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
  local unit=${BENCHMARK_SYSTEMD_SERVICE:-} active substate main_pid exit_status nice io_class io_priority control_group cgroups
  gate_build_service_state=fail
  if [ -z "$unit" ]; then
    gate_reason=missing-benchmark-systemd-service
    return 1
  fi
  if ! safe_unit_name "$unit"; then
    gate_reason=invalid-benchmark-systemd-service
    return 1
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    gate_reason=systemctl-unavailable
    return 1
  fi
  active=$(read_systemd_value "$unit" ActiveState) || { gate_reason=build-service-unreadable; return 1; }
  substate=$(read_systemd_value "$unit" SubState) || { gate_reason=build-service-unreadable; return 1; }
  main_pid=$(read_systemd_value "$unit" MainPID) || { gate_reason=build-service-unreadable; return 1; }
  exit_status=$(read_systemd_value "$unit" ExecMainStatus) || { gate_reason=build-service-unreadable; return 1; }
  nice=$(read_systemd_value "$unit" Nice) || { gate_reason=build-service-unreadable; return 1; }
  io_class=$(read_systemd_value "$unit" IOSchedulingClass) || { gate_reason=build-service-unreadable; return 1; }
  io_priority=$(read_systemd_value "$unit" IOSchedulingPriority) || { gate_reason=build-service-unreadable; return 1; }
  control_group=$(read_systemd_value "$unit" ControlGroup) || { gate_reason=build-service-unreadable; return 1; }
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
  local units=${BENCHMARK_PRODUCTION_HEALTH_UNITS:-} containers=${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-}
  local unit container active exit_status inspected running health project extra
  local -a health_units=() health_containers=()
  gate_health_state=fail
  if [ -z "$units" ]; then
    gate_reason=missing-production-health-units
    return 1
  fi
  IFS=, read -r -a health_units <<<"$units"
  [ "${#health_units[@]}" -gt 0 ] || { gate_reason=missing-production-health-units; return 1; }
  for unit in "${health_units[@]}"; do
    safe_unit_name "$unit" || { gate_reason=invalid-production-health-unit; return 1; }
    active=$(read_systemd_value "$unit" ActiveState) || { gate_reason=production-health-unreadable; return 1; }
    exit_status=$(read_systemd_value "$unit" ExecMainStatus) || { gate_reason=production-health-unreadable; return 1; }
    [ "$active" = active ] && [ "$exit_status" = 0 ] || { gate_reason=production-health-unhealthy; return 1; }
  done
  if [ -z "$containers" ]; then
    gate_reason=missing-production-health-containers
    return 1
  fi
  case "$containers" in
    ,*|*,|*,,*) gate_reason=invalid-production-health-container; return 1 ;;
  esac
  IFS=, read -r -a health_containers <<<"$containers"
  [ "${#health_containers[@]}" -gt 0 ] || { gate_reason=missing-production-health-containers; return 1; }
  command -v docker >/dev/null 2>&1 || { gate_reason=production-container-inspect-unavailable; return 1; }
  for container in "${health_containers[@]}"; do
    safe_container_reference "$container" || { gate_reason=invalid-production-health-container; return 1; }
    # Inspect only the serving-state boolean, declared health status, and the
    # Compose project identity.  Never retrieve Config.Env, health logs, or
    # full inspect JSON into evidence or diagnostics.
    if ! inspected=$(docker inspect --type container --format '{{.State.Running}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}|{{index .Config.Labels "com.docker.compose.project"}}' -- "$container" 2>/dev/null); then
      gate_reason=production-container-unreadable
      return 1
    fi
    case "$inspected" in
      *$'\n'*|*$'\r'*|*[$'\001'-$'\037'$'\177']*) gate_reason=production-container-status-malformed; return 1 ;;
    esac
    running= health= project= extra=
    IFS='|' read -r running health project extra <<<"$inspected"
    [ -z "$extra" ] || { gate_reason=production-container-status-malformed; return 1; }
    [ "$running" = true ] || { gate_reason=production-container-not-running; return 1; }
    [ "$health" != absent ] || { gate_reason=production-container-health-unestablished; return 1; }
    [ "$health" = healthy ] || { gate_reason=production-container-unhealthy; return 1; }
    [ "$project" = lagrange-station ] || { gate_reason=production-container-wrong-compose-project; return 1; }
  done
  gate_health_state=verified
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
    $'error\tentry-limit-reached') printf '%s' entry-limit-reached ;;
    *) printf '%s' parser-failed ;;
  esac
}

journal_json_summary() {
  local kind=$1 raw_file=$2 expected_boot_id=$3 since_utc=$4 until_utc=$5 parser_stderr=$6
  command -v python3 >/dev/null 2>&1 || return 127
  python3 - "$kind" "$raw_file" "$expected_boot_id" "$since_utc" "$until_utc" 2>"$parser_stderr" <<'PY'
import calendar
import datetime
import json
import re
import sys

kind, raw_path, expected_boot, since_text, until_text = sys.argv[1:]

def fail(code):
    print(f"error\t{code}")
    raise SystemExit(1)

try:
    raw = open(raw_path, "rb").read()
except OSError:
    fail("raw-unreadable")

try:
    text = raw.decode("utf-8")
except UnicodeDecodeError:
    fail("invalid-utf8")

def utc_to_us(value):
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value):
        fail("invalid-query-range")
    try:
        parsed = datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        fail("invalid-query-range")
    return calendar.timegm(parsed.timetuple()) * 1_000_000

if kind not in {"probe", "lookback"}:
    fail("invalid-kind")
if not re.fullmatch(r"[0-9a-f]{32}", expected_boot):
    fail("invalid-boot-id")
since_us = utc_to_us(since_text)
until_us = utc_to_us(until_text)
if until_us < since_us:
    fail("invalid-query-range")

lines = text.splitlines()
if not lines:
    if kind == "probe":
        fail("probe-entry-count")
    print("ok\t0\t0")
    raise SystemExit(0)
if any(not line.strip() for line in lines):
    fail("blank-json-line")
if kind == "probe" and len(lines) != 1:
    fail("probe-entry-count")
if kind == "lookback" and len(lines) >= 1000:
    fail("entry-limit-reached")

oom_pattern = re.compile(r"out of memory|oom[-_ ]?kill|killed process", re.IGNORECASE)
oom_count = 0
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
    if kind == "lookback":
        entry_us = int(timestamp)
        if entry_us < since_us or entry_us > until_us:
            fail("out-of-range-timestamp")
        if oom_pattern.search(message):
            oom_count += 1

print(f"ok\t{len(lines)}\t{oom_count}")
PY
}

observe_kernel_journal() {
  local journal_tmp prior_umask probe_stdout probe_stderr lookback_stdout lookback_stderr parser_stderr
  local raw_boot_id current_boot_id final_raw_boot_id final_boot_id gate_utc parser_summary parser_reason command_status
  reset_journal_observation
  command -v journalctl >/dev/null 2>&1 || {
    journal_observation_reason=kernel-journal-unavailable
    return 1
  }
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
  prior_umask=$(umask)
  umask 077
  : >"$probe_stdout" && : >"$probe_stderr" && : >"$lookback_stdout" && : >"$lookback_stderr" && : >"$parser_stderr"
  umask "$prior_umask"
  if [ ! -f "$probe_stdout" ] || [ ! -f "$probe_stderr" ] || [ ! -f "$lookback_stdout" ] || [ ! -f "$lookback_stderr" ] || [ ! -f "$parser_stderr" ]; then
    cleanup_kernel_journal_tmp "$journal_tmp" || true
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  fi
  chmod 0600 -- "$probe_stdout" "$probe_stderr" "$lookback_stdout" "$lookback_stderr" "$parser_stderr" || {
    cleanup_kernel_journal_tmp "$journal_tmp" || true
    journal_observation_reason=kernel-journal-private-temp-unavailable
    return 1
  }

  if LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1 >"$probe_stdout" 2>"$probe_stderr"; then
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
  if parser_summary=$(journal_json_summary probe "$probe_stdout" "$current_boot_id" "$journal_since_utc" "$journal_until_utc" "$parser_stderr"); then
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
  if [[ "$parser_summary" =~ ^ok$'\t'([0-9]+)$'\t'0$ ]]; then
    journal_probe_entry_count=${BASH_REMATCH[1]}
  else
    journal_observation_reason=kernel-journal-probe-parser-output-invalid
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  journal_established_boot_id=$current_boot_id

  if LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json --since "$journal_since_utc" --until "$journal_until_utc" -n 1000 >"$lookback_stdout" 2>"$lookback_stderr"; then
    journal_lookback_status=exit-0
  else
    command_status=$?
    journal_lookback_status=exit-$command_status
  fi
  journal_lookback_stdout_sha256=$(sha256_file "$lookback_stdout")
  journal_lookback_stderr_sha256=$(sha256_file "$lookback_stderr")
  if [ "$journal_lookback_status" != exit-0 ]; then
    journal_observation_reason=kernel-journal-lookback-nonzero
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if [ -s "$lookback_stderr" ]; then
    journal_observation_reason=kernel-journal-lookback-stderr
    cleanup_kernel_journal_tmp "$journal_tmp" || return 1
    return 1
  fi
  if parser_summary=$(journal_json_summary lookback "$lookback_stdout" "$current_boot_id" "$journal_since_utc" "$journal_until_utc" "$parser_stderr"); then
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
  local revision=$1 scenario=$2 batch=$3 point=$4 status=pass reason=-
  gate_reason=
  snapshot_resources || { status=fail; reason=$gate_reason; }
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
  if [ "$status" = pass ]; then
    check_production_health || { status=fail; reason=$gate_reason; }
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario" "$batch" "$point" "$gate_mem_available" "$gate_swap_free" \
    "$min_mem_available_kib" "$min_swap_free_kib" "$gate_oom_events" \
    "$journal_probe_status" "$journal_probe_entry_count" "$journal_probe_stdout_sha256" "$journal_probe_stderr_sha256" \
    "$journal_lookback_status" "$journal_lookback_entry_count" "$journal_lookback_oom_count" "$journal_lookback_stdout_sha256" "$journal_lookback_stderr_sha256" \
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

run_scenario() {
  local revision=$1 measurement_phase=$2 checkout=$3 commit=$4 namespace=$5
  local service index=0 batch=0
  for service in "${services[@]}"; do
    if [ $((index % 3)) -eq 0 ]; then
      batch=$((batch + 1))
      gate_batch "$revision" "$measurement_phase" "$batch" before
    fi
    index=$((index + 1))
    build_one_service "$revision" "$measurement_phase" "$checkout" "$commit" "$service" "$index" "$namespace"
    if [ $((index % 3)) -eq 0 ] || [ "$index" -eq "${#services[@]}" ]; then
      gate_batch "$revision" "$measurement_phase" "$batch" after
    fi
  done
}

write_warm_comparison_report() {
  local service baseline_fields candidate_fields baseline_total candidate_total baseline_cargo candidate_cargo
  printf 'service\tbaseline_total_ms\tcandidate_total_ms\tdelta_total_ms\tbaseline_cargo_ms\tcandidate_cargo_ms\tdelta_cargo_ms\tbaseline_compiler_cache\tcandidate_compiler_cache\tbaseline_compiled\tcandidate_compiled\tbaseline_fresh\tcandidate_fresh\n' >"$comparison_report"
  for service in "${services[@]}"; do
    baseline_fields=$(awk -F '\t' -v service="$service" '$1 == "baseline" && $2 == "measured" && $3 == service { print; exit }' "$results_report")
    candidate_fields=$(awk -F '\t' -v service="$service" '$1 == "candidate" && $2 == "measured" && $3 == service { print; exit }' "$results_report")
    [ -n "$baseline_fields" ] && [ -n "$candidate_fields" ] ||
      die "warm comparison lacks a measured revision record: $service"
    baseline_total=$(cut -f9 <<<"$baseline_fields")
    candidate_total=$(cut -f9 <<<"$candidate_fields")
    baseline_cargo=$(cut -f10 <<<"$baseline_fields")
    candidate_cargo=$(cut -f10 <<<"$candidate_fields")
    is_decimal "$baseline_total" && is_decimal "$candidate_total" || die 'warm comparison total timing is malformed'
    {
      printf '%s\t%s\t%s\t%s\t%s\t%s\t' \
        "$service" "$baseline_total" "$candidate_total" "$((candidate_total - baseline_total))" \
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

write_cold_comparison_notice() {
  printf 'comparison_status\treason\nnot-comparable\tcold uses separate fresh namespaces and --no-cache; no changed/warm comparison is claimed\n' >"$comparison_report"
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
  case "$tmp_dir" in
    /tmp/lagrange-build-cache-benchmark.*) rm -rf -- "$tmp_dir" ;;
    *) return 0 ;;
  esac
  tmp_dir=
}

cleanup_apply_run() {
  stop_resource_sampler || true
  cleanup_test_images
  cleanup_temp_checkouts
}

initialize_output() {
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
  failure_report=$output_dir/failure.tsv
  apply_record_failures=1
  printf 'revision\tmeasurement_phase\tservice\tindex\timage_tag\tcache_mount_namespace\tbuild_commit\tmeasured_source_identity_sha256\timage_build_ms\tcargo_ms\timage_verification_ms\timage_id\timage_revision\tcompiler_vertex\tcompiler_vertex_state\tcompiler_cache\tnoncompiler_cached_vertices\tcargo_cache_state\tcompiled_packages\tfresh_packages\tworkspace_clean\ttoolchain_identity\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\tsanitized_evidence\tselected_scenario\n' >"$results_report"
  printf 'revision\tmeasurement_phase\tbatch\tgate\tmem_available_kib\tswap_free_kib\tminimum_mem_available_kib\tminimum_swap_free_kib\trecent_kernel_oom_events\tjournal_probe_status\tjournal_probe_entries\tjournal_probe_stdout_sha256\tjournal_probe_stderr_sha256\tjournal_lookback_status\tjournal_lookback_entries\tjournal_lookback_oom_matches\tjournal_lookback_stdout_sha256\tjournal_lookback_stderr_sha256\tjournal_since_utc\tjournal_until_utc\tjournal_reason\tprevious_build_exit\tcompiler_processes\tbackground_build_service\tproduction_health\tstatus\n' >"$resource_report"
  printf 'revision\tmeasurement_phase\tservice\tpoint\ttimestamp_ms\tmem_available_kib\tswap_free_kib\trepository_disk_available_kib\tdocker_root_disk_available_kib\n' >"$resource_samples_report"
  printf 'scope\tsample_count\tminimum_mem_available_kib\tminimum_swap_free_kib\tminimum_repository_disk_available_kib\tminimum_docker_root_disk_available_kib\tinterpretation\n' >"$peak_resource_report"
  printf 'phase\trevision\tscenario\tstarted_ms\tfinished_ms\telapsed_ms\tstatus\tdetail\n' >"$phase_report"
  printf 'revision\tmeasurement_phase\tinput_commit\tcheckout_head\tcheckout_tree\tprobe_patch_sha256\tinstrumentation_manifest_sha256\tmeasured_source_identity_sha256\n' >"$source_identity_report"
  printf 'revision\tservice\tsource_dockerfile\tinstrumentation\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\ttoolchain_identity\n' >"$instrumentation_report"
  chmod 0600 -- "$results_report" "$resource_report" "$resource_samples_report" \
    "$peak_resource_report" "$phase_report" "$source_identity_report" "$instrumentation_report"
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
  if ! value=$(docker buildx inspect --format '{{.Name}}|{{.Driver}}' 2>/dev/null); then
    die 'Docker builder identity is unavailable for --apply'
  fi
  builder_identity=$(safe_scalar "$value")
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
  local baseline_namespace=$1 candidate_namespace=$2 protocol=$3
  {
    printf '%s\n' BUILD_CACHE_BENCHMARK_V4
    printf 'cache_mode\t%s\n' "$cache_mode"
    printf 'benchmark_scenario\t%s\n' "$scenario"
    printf 'scenario_input_path\t%s\n' "${scenario_input_path[$scenario]}"
    printf 'scenario_transformation\t%s\n' "${scenario_transform[$scenario]}"
    printf 'scenario_expected_scope\t%s\n' "${scenario_expected_scope[$scenario]}"
    printf 'protocol\t%s\n' "$protocol"
    printf 'baseline_commit\t%s\n' "$baseline_commit"
    printf 'candidate_commit\t%s\n' "$candidate_commit"
    printf 'baseline_cache_mount_namespace\t%s\n' "$baseline_namespace"
    printf 'candidate_cache_mount_namespace\t%s\n' "$candidate_namespace"
    printf 'cache_mount_argument\t--build-arg BUILDKIT_CACHE_MOUNT_NS=<namespace> on every Docker build\n'
    printf 'cold_layer_policy\t%s\n' "$([ "$cache_mode" = cold ] && printf -- '--no-cache' || printf normal-layer-reuse)"
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
    printf 'cargo_build_jobs\t2\n'
    printf 'service_order\t%s\n' "${services[*]}"
    printf 'batch_policy\tup-to-three-services; one Docker invocation at a time\n'
    printf 'resource_min_mem_available_kib\t%s\n' "$min_mem_available_kib"
    printf 'resource_min_swap_free_kib\t%s\n' "$min_swap_free_kib"
    printf 'kernel_journal_probe\tLC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1; require exit-0, no-stderr, exactly-one valid current-boot kernel entry\n'
    printf 'kernel_journal_lookback\tLC_ALL=C timeout 10s journalctl -k -b --no-pager -o json --since <fixed-first-gate-minus-1800s> --until <gate-utc> -n 1000; require exit-0, no-stderr, valid bounded current-boot JSON, and fewer-than-1000 entries\n'
    printf 'kernel_journal_lookback_seconds\t%s\n' "$journal_lookback_seconds"
    printf 'kernel_journal_first_gate_utc\t%s\n' "${journal_first_gate_utc:-unavailable}"
    printf 'kernel_journal_since_utc\t%s\n' "${journal_since_utc:-unavailable}"
    printf 'kernel_journal_evidence_policy\tprivate-raw-stdout-stderr-removed-after-hash; public-status-count-range-hash-oom-and-reason-only; any-query-warning-error-malformed-entry-or-1000-entry-window-fails-closed\n'
    printf 'build_service_unit\t%s\n' "${BENCHMARK_SYSTEMD_SERVICE:-unconfigured}"
    printf 'production_health_units\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_UNITS:-unconfigured}"
    printf 'production_health_containers\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-unconfigured}"
    printf 'production_container_health_policy\trunning; declared healthy healthcheck; compose project lagrange-station; bounded inspect fields only\n'
    printf 'source_checkout_policy\ttemporary-detached-local-clones\n'
    printf 'probe_policy\tone selected common temporary scenario transformation; exact revision-relative patches and hashes retained\n'
    printf 'commit_transition_policy\tmeasured source is committed as a synthetic child so the temporary image OCI revision and embedded code commit match its tree; commit-only is the transition control\n'
    printf 'probe_spec_sha256\t%s\n' "${probe_spec_hash:-unavailable}"
    printf 'instrumentation_policy\ttemporary Dockerfile copies only; Cargo build/install verbosity only\n'
    printf 'instrumentation_manifest_sha256\t%s\n' "${instrumentation_manifest_hash:-unavailable}"
    printf 'evidence_policy\tsanitized BuildKit/Cargo logs retained; raw temporary logs removed with clones\n'
    printf 'image_identity_scope\ttemporary modified-source benchmark image; not an immutable official release image\n'
    printf 'timing_policy\timage_build_ms is end-to-end Docker build wall time; cargo_ms is only the observed compiler RUN duration; image_verification_ms is bounded temporary image ID/revision inspection\n'
    printf 'link_time_policy\tnot-separated; Cargo RUN duration is never reported as linker time\n'
    printf 'resource_sampling_policy\tminimum sampled MemAvailable/SwapFree/repository-disk/docker-root-disk during each Docker build; not process-RSS or exact used-memory peak\n'
    printf 'release_validation_scope\tnot-measured-by-this-benchmark; strict V2 manifest and official image checks require the release procedure\n'
    printf 'lifecycle_policy\tno rollout, compose lifecycle, provider access, credential read, or global prune\n'
    # The former foreground sequential arrangement is prohibited; retain that
    # phrase for the repository's static compatibility check while stating the
    # actual policy unambiguously.
    printf 'foreground_sequential_policy\tforbidden; apply must run inside the verified background systemd service\n'
  } >"$metadata_report"
  chmod 0600 -- "$metadata_report"
}

create_temp_run_directory() {
  local base raw_suffix
  tmp_dir=$(mktemp -d /tmp/lagrange-build-cache-benchmark.XXXXXXXXXX) || die 'could not create temporary benchmark directory'
  base=${tmp_dir##*/}
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
  local baseline_namespace candidate_namespace protocol probe_line
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
  # Validate paths and all no-build prerequisites before any Docker build.  A
  # failed gate writes failure.tsv and leaves the empty/auditable output run.
  gate_batch preflight prerequisite 0 before
  collect_host_identity
  collect_docker_identity
  create_temp_run_directory
  trap cleanup_apply_run EXIT
  prepare_checkout "$baseline_commit" "$baseline_checkout"
  prepare_checkout "$candidate_commit" "$candidate_checkout"
  validate_checkout_contract "$baseline_checkout" baseline
  validate_checkout_contract "$candidate_checkout" candidate
  probe_line=$(scenario_probe_line)
  printf 'scenario\tinput_path\ttransformation\texpected_scope\tprobe_line\n' >"$probe_spec_report"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$scenario" "${scenario_input_path[$scenario]}" "${scenario_transform[$scenario]}" \
    "${scenario_expected_scope[$scenario]}" "$probe_line" >>"$probe_spec_report"
  chmod 0600 -- "$probe_spec_report"
  probe_spec_hash=$(sha256_file "$probe_spec_report")
  instrument_all_dockerfiles baseline "$baseline_checkout"
  instrument_all_dockerfiles candidate "$candidate_checkout"
  instrumentation_manifest_hash=$(sha256_file "$instrumentation_report")
  baseline_namespace=$(namespace_for_revision baseline "$baseline_commit")
  candidate_namespace=$(namespace_for_revision candidate "$candidate_commit")
  prep_finished=$(now_ms)
  record_phase preparation all "$scenario" "$prep_started" "$prep_finished" PASS \
    'gates-identity-detached-checkouts-scenario-spec-and-temporary-instrumentation'

  if [ "$cache_mode" = warm ]; then
    protocol=warmup-unmodified-then-common-scenario-commit-measured
    record_source_snapshot baseline warmup "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate warmup "$candidate_checkout" "$candidate_commit"
    write_metadata "$baseline_namespace" "$candidate_namespace" "$protocol"
    printf 'BENCHMARK_RUN mode=warm baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    phase_started=$(now_ms)
    run_scenario baseline warmup "$baseline_checkout" "$baseline_commit" "$baseline_namespace"
    phase_finished=$(now_ms)
    record_phase warmup-builds baseline "$scenario" "$phase_started" "$phase_finished" PASS \
      'unchanged-source-sequential-twelve-service-cache-warmup'
    phase_started=$(now_ms)
    run_scenario candidate warmup "$candidate_checkout" "$candidate_commit" "$candidate_namespace"
    phase_finished=$(now_ms)
    record_phase warmup-builds candidate "$scenario" "$phase_started" "$phase_finished" PASS \
      'unchanged-source-sequential-twelve-service-cache-warmup'
    phase_started=$(now_ms)
    apply_equivalent_scenario
    record_source_snapshot baseline measured "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate measured "$candidate_checkout" "$candidate_commit"
    phase_finished=$(now_ms)
    record_phase scenario-input-preparation all "$scenario" "$phase_started" "$phase_finished" PASS \
      'equivalent-temporary-scenario-commits-and-source-patch-identities'
    phase_started=$(now_ms)
    run_scenario baseline measured "$baseline_checkout" "${scenario_commit_by_revision[baseline]}" "$baseline_namespace"
    phase_finished=$(now_ms)
    record_phase measured-image-builds baseline "$scenario" "$phase_started" "$phase_finished" PASS \
      'sequential-twelve-service-temporary-image-builds-with-per-image-inspection'
    phase_started=$(now_ms)
    run_scenario candidate measured "$candidate_checkout" "${scenario_commit_by_revision[candidate]}" "$candidate_namespace"
    phase_finished=$(now_ms)
    record_phase measured-image-builds candidate "$scenario" "$phase_started" "$phase_finished" PASS \
      'sequential-twelve-service-temporary-image-builds-with-per-image-inspection'
    phase_started=$(now_ms)
    write_warm_comparison_report
    phase_finished=$(now_ms)
    record_phase comparison-report all "$scenario" "$phase_started" "$phase_finished" PASS \
      'per-service-baseline-candidate-deltas; strict-release-manifest-not-measured'
  else
    protocol=cold-independent-changed-source-no-warm-comparison
    phase_started=$(now_ms)
    apply_equivalent_scenario
    record_source_snapshot baseline measured "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate measured "$candidate_checkout" "$candidate_commit"
    phase_finished=$(now_ms)
    record_phase scenario-input-preparation all "$scenario" "$phase_started" "$phase_finished" PASS \
      'equivalent-temporary-scenario-commits-and-source-patch-identities'
    write_metadata "$baseline_namespace" "$candidate_namespace" "$protocol"
    printf 'BENCHMARK_RUN mode=cold baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    phase_started=$(now_ms)
    run_scenario baseline measured "$baseline_checkout" "${scenario_commit_by_revision[baseline]}" "$baseline_namespace"
    phase_finished=$(now_ms)
    record_phase measured-image-builds baseline "$scenario" "$phase_started" "$phase_finished" PASS \
      'sequential-twelve-service-temporary-image-builds-with-per-image-inspection'
    phase_started=$(now_ms)
    run_scenario candidate measured "$candidate_checkout" "${scenario_commit_by_revision[candidate]}" "$candidate_namespace"
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
  write_peak_resource_report
  phase_finished=$(now_ms)
  record_phase resource-summary all "$scenario" "$phase_started" "$phase_finished" PASS \
    'sampled-available-memory-swap-and-disk-minima'
  phase_finished=$(now_ms)
  record_phase benchmark-overall all "$scenario" "$overall_started" "$phase_finished" PASS \
    'preparation-warmup-or-cold-measurement-comparison-and-resource-summary; no-strict-V2-manifest'
  printf 'BUILD_CACHE_BENCHMARK_RESULT PASS output_dir=%s records=%s\n' \
    "$output_dir" "$(( $(wc -l <"$results_report") - 1 ))"
  printf '%s\n' 'BUILD_CACHE_BENCHMARK_APPLY_STATUS benchmark-only; no deployment or release validation was performed'
}

expect_parse_failure() {
  local log=$1 cargo_mode=$2 revision=$3 description=$4
  if parse_build_events "$log" "$cargo_mode" "$revision"; then
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
  local test_dir parser_dir baseline_log candidate_log cached_log absent_log malformed_log missing_finish_log failed_log
  local current parent plan_output first_nonce second_nonce foreign_tag build_count remove_count
  local case_variant_one case_variant_two health_mode health_reason saved_health_containers scenario_case scenario_path
  test_dir=$(mktemp -d /tmp/lagrange-build-cache-benchmark-self-test.XXXXXXXXXX)
  trap 'rm -rf -- "$test_dir"' RETURN
  parser_dir=$test_dir/parser
  mkdir -p -- "$parser_dir"
  declare -A BENCH_TEST_IMAGE_COMMIT=()

  # Parser contracts: baseline has no clean requirement; candidate does when
  # executed.  A generic CACHED COPY must not masquerade as a compiler hit.
  baseline_log=$parser_dir/baseline.log
  candidate_log=$parser_dir/candidate.log
  cached_log=$parser_dir/cached.log
  absent_log=$parser_dir/absent.log
  malformed_log=$parser_dir/malformed.log
  missing_finish_log=$parser_dir/missing-finish.log
  failed_log=$parser_dir/failed.log
  cat >"$baseline_log" <<'EOF'
#2 [builder 2/7] COPY Cargo.toml ./
#2 CACHED
#9 [builder 7/7] RUN cargo build -vv --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-baseline v0.1.0 (/build)
#9 0.30 Finished `release` profile [optimized] target(s) in 2.25s
#9 DONE 2.25s
EOF
  cat >"$candidate_log" <<'EOF'
#9 [builder 7/7] RUN cargo clean --workspace --release --locked && cargo build -vv --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-candidate v0.1.0 (/build)
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
  parse_build_events "$baseline_log" 1 baseline
  [ "$BENCH_COMPILER_VERTEX_STATE" = executed ] || die 'self-test baseline compiler should execute'
  [ "$BENCH_COMPILER_CACHE" = miss ] || die 'self-test unrelated CACHED was counted as compiler hit'
  [ "$BENCH_WORKSPACE_CLEAN" = absent ] || die 'self-test baseline no-clean contract regressed'
  [ "$BENCH_CARGO_STEP_MS" = 2250 ] || die 'self-test baseline Cargo duration regressed'
  parse_build_events "$candidate_log" 1 candidate
  [ "$BENCH_WORKSPACE_CLEAN" = observed ] || die 'self-test candidate clean contract regressed'
  [ "$BENCH_CARGO_STEP_MS" = 1250 ] || die 'self-test candidate Cargo duration regressed'
  parse_build_events "$cached_log" 1 candidate
  [ "$BENCH_COMPILER_VERTEX_STATE" = cached ] || die 'self-test cached compiler vertex was not recognized'
  [ "$BENCH_COMPILER_CACHE" = hit ] || die 'self-test cached compiler cache state regressed'
  [ "$BENCH_CARGO_STEP_MS" = 0 ] || die 'self-test cached compiler did not use explicit zero time'
  [ "$BENCH_WORKSPACE_CLEAN" = not-executed ] || die 'self-test cached compiler claimed clean evidence'
  expect_parse_failure "$baseline_log" 1 candidate candidate-no-clean
  expect_parse_failure "$absent_log" 1 baseline absent-cargo-vertex
  expect_parse_failure "$malformed_log" 1 baseline malformed-duration
  expect_parse_failure "$missing_finish_log" 1 baseline missing-finish
  expect_parse_failure "$failed_log" 1 baseline compiler-error

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
  : >"$BENCH_TEST_DOCKER_RECORD"
  : >"$BENCH_TEST_SYSTEMCTL_RECORD"
  : >"$BENCH_TEST_JOURNAL_RECORD"

  # All following observations are shell fakes.  In particular, `docker` is a
  # function, so no client binary or daemon can be reached by this self-test.
  docker() {
    local command=${1:-} tag= namespace= build_arg=missing no_cache=no context= probe_state=unknown host_namespace
    shift || true
    case "$command" in
      version) printf '%s\n' '27.0.0|27.0.0' ;;
      info) printf '%s\n' 'linux|amd64|fixture-kernel|/tmp' ;;
      buildx)
        case "${1:-}" in
          version) printf '%s\n' 'github.com/docker/buildx v0.18.0 fixture' ;;
          inspect) printf '%s\n' 'fixture-builder|docker-container' ;;
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
        local arg next_is_build_arg=0 code_commit=
        for arg in "$@"; do
          if [ "$next_is_build_arg" -eq 1 ]; then
            case "$arg" in
              BUILDKIT_CACHE_MOUNT_NS=*) build_arg=${arg#BUILDKIT_CACHE_MOUNT_NS=} ;;
              LAGRANGE_CODE_COMMIT=*) code_commit=${arg#LAGRANGE_CODE_COMMIT=} ;;
            esac
            next_is_build_arg=0
            continue
          fi
          case "$arg" in
            --build-arg) next_is_build_arg=1 ;;
            -t) next_is_build_arg=2 ;;
            --no-cache) no_cache=yes ;;
            *)
              if [ "$next_is_build_arg" -eq 2 ]; then tag=$arg; next_is_build_arg=0
              elif [ "${arg:0:1}" = / ]; then context=$arg
              fi
              ;;
          esac
        done
        namespace=$build_arg
        host_namespace=${BUILDKIT_CACHE_MOUNT_NS-unset}
        [[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] || return 92
        BENCH_TEST_IMAGE_COMMIT["$tag"]=$code_commit
        if grep -R -Fq 'LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914' "$context"; then probe_state=perturbed; else probe_state=unchanged; fi
        printf 'build\t%s\t%s\t%s\t%s\t%s\t%s\n' "$tag" "$namespace" "$build_arg" "$host_namespace" "$no_cache" "$probe_state" >>"$BENCH_TEST_DOCKER_RECORD"
        if [ "${BENCH_TEST_DOCKER_FAIL_BUILD:-0}" = 1 ]; then
          printf '%s\n' '#9 ERROR: fixture build failure'
          return 88
        fi
        cat <<'EOF'
#2 [builder 2/7] COPY Cargo.toml ./
#2 CACHED
#9 [builder 7/7] RUN cargo clean --workspace --release --locked && cargo build -vv --locked --release
#9 0.10 Fresh itoa v1.0.18
#9 0.20 Compiling benchmark-fixture v0.1.0 (/build)
#9 0.30 Finished `release` profile [optimized] target(s) in 0.10s
#9 DONE 0.10s
API_TOKEN=fake-secret-must-not-persist
EOF
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
            [ "$inspect_format" = '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' ] || return 92
            [ -n "$inspect_target" ] && [ -n "${BENCH_TEST_IMAGE_COMMIT[$inspect_target]:-}" ] || return 92
            printf 'sha256:%064d|%s\n' 1 "${BENCH_TEST_IMAGE_COMMIT[$inspect_target]}"
            ;;
          rm)
            printf 'remove\t%s\n' "${3:-}" >>"$BENCH_TEST_DOCKER_RECORD"
            ;;
          *) return 92 ;;
        esac
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
  fixture_journal_entry() {
    local message=$1 boot_id=${2:-11111111111111111111111111111111} timestamp=${3:-1893456000000000}
    printf '{"__REALTIME_TIMESTAMP":"%s","__CURSOR":"s=fixture","_BOOT_ID":"%s","_TRANSPORT":"kernel","MESSAGE":"%s"}\n' \
      "$timestamp" "$boot_id" "$message"
  }
  timeout() {
    local duration=${1:-}
    [ "$duration" = 10s ] || return 97
    [ "${LC_ALL:-}" = C ] || return 97
    shift
    "$@"
  }
  journalctl() {
    local mode=${BENCH_TEST_JOURNAL_MODE:-clean} phase=
    if [ "$#" -eq 7 ] && [ "$1" = -k ] && [ "$2" = -b ] && [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] && [ "$6" = -n ] && [ "$7" = 1 ]; then
      phase=probe
    elif [ "$#" -eq 11 ] && [ "$1" = -k ] && [ "$2" = -b ] && [ "$3" = --no-pager ] && [ "$4" = -o ] && [ "$5" = json ] && [ "$6" = --since ] && [ "$7" = "$BENCH_TEST_JOURNAL_SINCE_UTC" ] && [ "$8" = --until ] && [ "$9" = "$BENCH_TEST_JOURNAL_GATE_UTC" ] && [ "${10}" = -n ] && [ "${11}" = 1000 ]; then
      phase=lookback
    else
      return 96
    fi
    printf '%s\t%s\n' "$phase" "$mode" >>"$BENCH_TEST_JOURNAL_RECORD"
    case "$mode:$phase" in
      probe-empty:probe) return 0 ;;
      probe-nonzero:probe) return 71 ;;
      nonzero-query:probe|permission-warning:probe|invalid-json-query:probe|wrong-boot-query:probe|oom:probe|clean:probe)
        fixture_journal_entry fixture-probe
        ;;
      nonzero-query:lookback) return 72 ;;
      permission-warning:lookback)
        printf '%s\n' 'fixture-journal-permission-warning-must-not-persist' >&2
        ;;
      invalid-json-query:lookback) printf '%s\n' '{invalid-json' ;;
      wrong-boot-query:lookback) fixture_journal_entry fixture-quiet 22222222222222222222222222222222 ;;
      oom:lookback) fixture_journal_entry 'Out of memory: fixture' ;;
      clean:lookback) return 0 ;;
      *) return 98 ;;
    esac
  }
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
        if grep -R -Fq 'LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914' "$checkout"; then
          printf 'diff --git a/probe b/probe\n@@ -0,0 +1 @@\n+LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914\n'
        fi
        ;;
      *) return 97 ;;
    esac
  }
  prepare_checkout() {
    local commit=$1 checkout=$2 service dockerfile path parent
    mkdir -p -- "$checkout"
    printf '%s\n' "$commit" >"$checkout/.benchmark-commit"
    printf '%s\n' '[toolchain]' 'channel = "1.97.1"' >"$checkout/rust-toolchain.toml"
    for service in "${services[@]}"; do
      dockerfile=$checkout/${service_dockerfile[$service]}
      mkdir -p -- "${dockerfile%/*}"
      if [ "${service_cargo_mode[$service]}" -gt 0 ]; then
        printf '%s\n' 'FROM rust:1.97.1-alpine' 'RUN cargo build --locked --release' >"$dockerfile"
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
  }
  [ "$(type -t docker)" = function ] || die 'self-test Docker fake was not installed'
  [ "$(type -t git)" = function ] || die 'self-test Git fake was not installed'
  [ "$(type -t systemctl)" = function ] || die 'self-test systemd fake was not installed'
  [ "$(type -t journalctl)" = function ] || die 'self-test journalctl fake was not installed'
  [ "$(type -t timeout)" = function ] || die 'self-test timeout fake was not installed'

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
  # current-boot kernel entry succeeds.  The fake validates both exact bounded
  # journalctl argv forms and the 10-second LC_ALL=C timeout wrapper.
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
  [ "$journal_first_gate_utc" = "$BENCH_TEST_JOURNAL_GATE_UTC" ] && [ "$journal_since_utc" = "$BENCH_TEST_JOURNAL_SINCE_UTC" ] ||
    die 'self-test fixed kernel journal range regressed'
  [[ "$journal_probe_stdout_sha256" =~ ^[0-9a-f]{64}$ ]] && [[ "$journal_probe_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] &&
    [[ "$journal_lookback_stdout_sha256" =~ ^[0-9a-f]{64}$ ]] && [[ "$journal_lookback_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test kernel journal hashes were not retained'
  awk -F '\t' '$1 == "probe" && $2 == "clean" { probe++ } $1 == "lookback" && $2 == "clean" { lookback++ } END { exit !(probe == 1 && lookback == 1) }' "$BENCH_TEST_JOURNAL_RECORD" ||
    die 'self-test kernel journal probe/lookback calls were not both made'

  # Query failure, a successful query with a warning, unavailable boot ID,
  # empty probe output, malformed JSON, and wrong boot metadata are all fail
  # closed.  None can be interpreted as a quiet/OOM-free window.
  expect_journal_snapshot_failure nonzero-query kernel-journal-lookback-nonzero nonzero-lookback-query
  [ "$journal_probe_status" = exit-0 ] && [ "$journal_probe_entry_count" = 1 ] && [ "$journal_lookback_status" = exit-72 ] ||
    die 'self-test nonzero lookback query evidence regressed'
  expect_journal_snapshot_failure permission-warning kernel-journal-lookback-stderr exit-zero-permission-warning
  [ "$journal_lookback_status" = exit-0 ] && [[ "$journal_lookback_stderr_sha256" =~ ^[0-9a-f]{64}$ ]] ||
    die 'self-test warning-bearing lookback did not retain bounded status/hash evidence'
  grep -R -Fq 'fixture-journal-permission-warning-must-not-persist' "$test_dir" &&
    die 'self-test persisted journal warning text outside its private temporary file'
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
    apply_equivalent_scenario >/dev/null
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
      if ($2 !~ /^[a-z0-9]+([._-][a-z0-9]+)*$/) bad=1
      if ($3 == "" || $4 != $3 || $5 != "unset" || $6 != "no") bad=1
      if ($2 ~ /-baseline-/) {
        if (baseline_namespace == "") baseline_namespace = $3
        else if (baseline_namespace != $3) bad=1
      }
      if ($2 ~ /-candidate-/) {
        if (candidate_namespace == "") candidate_namespace = $3
        else if (candidate_namespace != $3) bad=1
      }
      if (n <= 24 && ($2 !~ /-warmup-/ || $7 != "unchanged")) bad=1
      if (n > 24 && ($2 !~ /-measured-/ || $7 != "perturbed")) bad=1
    }
    END { exit !(n == 48 && !bad && baseline_namespace != "" && candidate_namespace != "" && baseline_namespace != candidate_namespace) }
  ' "$BENCH_TEST_DOCKER_RECORD" || die 'self-test warm namespace, repository grammar, or warm-up ordering regressed'
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
  grep -Fxq 'BUILD_CACHE_BENCHMARK_V4' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal evidence schema version was not retained'
  grep -Fq $'kernel_journal_probe\tLC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1; require exit-0, no-stderr, exactly-one valid current-boot kernel entry' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal readability probe metadata was not retained'
  grep -Fq $'kernel_journal_lookback_seconds\t1800' "$output_dir/metadata.tsv" ||
    die 'self-test kernel journal fixed lookback metadata was not retained'
  if ! awk -F '\t' '
    NR == 1 {
      if (NF != 26 || $10 != "journal_probe_status" || $14 != "journal_lookback_status" || $21 != "journal_reason") bad=1
      next
    }
    NF != 26 { bad=1 }
    $9 != "0" || $10 != "exit-0" || $11 != "1" || $12 !~ /^[0-9a-f]{64}$/ || $13 !~ /^[0-9a-f]{64}$/ { bad=1 }
    $14 != "exit-0" || $15 != "0" || $16 != "0" || $17 !~ /^[0-9a-f]{64}$/ || $18 !~ /^[0-9a-f]{64}$/ { bad=1 }
    $19 != "2029-12-31T23:30:00Z" || $20 != "2030-01-01T00:00:00Z" || $21 != "established" { bad=1 }
    END { exit !(NR > 1 && !bad) }
  ' "$output_dir/batch-resources.tsv"; then
    die 'self-test bounded kernel journal status/count/hash/reason evidence was malformed'
  fi
  grep -Fq $'commit_transition_policy\tmeasured source is committed as a synthetic child so the temporary image OCI revision and embedded code commit match its tree; commit-only is the transition control' "$output_dir/metadata.tsv" ||
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
  if ! awk -F '\t' '$1 == "baseline" && $2 == "measured" && $3 == "1111111111111111111111111111111111111111" && $4 != $3 { found=1 } END { exit !found }' "$output_dir/source-identities.tsv"; then
    die 'self-test measured source commit did not differ from its original baseline commit'
  fi
  [ "$(wc -l <"$output_dir/comparison.tsv")" -eq 13 ] || die 'self-test warm comparison lacks twelve measured records'
  awk -F '\t' 'NF != 13 { bad=1 } END { exit !(NR == 13 && !bad) }' "$output_dir/comparison.tsv" ||
    die 'self-test warm comparison report has malformed fields'
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

  # Cold runs have fresh namespaces and --no-cache but no warm-up scenario or
  # comparison delta.  They remain fake Docker invocations.
  : >"$BENCH_TEST_DOCKER_RECORD"
  output_dir=$test_dir/cold-output
  cache_mode=cold
  tag_list=()
  run_apply >"$test_dir/cold-run.out" 2>&1
  trap - EXIT
  build_count=$(awk -F '\t' '$1 == "build" { count++ } END { print count + 0 }' "$BENCH_TEST_DOCKER_RECORD")
  [ "$build_count" -eq 24 ] || die "self-test cold state machine made $build_count builds, expected 24"
  awk -F '\t' '$1 == "build" { count++; if ($2 !~ /^[a-z0-9]+([._-][a-z0-9]+)*$/ || $3 == "" || $4 != $3 || $5 != "unset" || $6 != "yes" || $2 ~ /warmup/) bad=1 } END { exit !(count == 24 && !bad) }' \
    "$BENCH_TEST_DOCKER_RECORD" || die 'self-test cold namespace, repository grammar, or no-cache contract regressed'
  grep -Fq 'not-comparable' "$output_dir/comparison.tsv" || die 'self-test cold run claimed a warm comparison'
  cleanup_test_images
  cleanup_temp_checkouts
  [ -f "$output_dir/source-identities.tsv" ] || die 'self-test cold evidence was not preserved'

  echo 'BUILD_CACHE_BENCHMARK_SELF_TEST: PASS (fake Docker/Git/systemd/resource observations only; no daemon or Rust compilation)'
}

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

if [ "$mode" = plan ]; then
  echo 'BUILD_CACHE_BENCHMARK_PLAN mode=plan'
  echo "  baseline_commit=$baseline_commit"
  echo "  candidate_commit=$candidate_commit"
  echo "  output_dir=$output_dir"
  echo "  cache_mode=$cache_mode"
  echo "  scenario=$scenario (${scenario_transform[$scenario]}; input=${scenario_input_path[$scenario]}; expected_scope=${scenario_expected_scope[$scenario]})"
  echo '  namespaces=every Docker build receives an explicit --build-arg BUILDKIT_CACHE_MOUNT_NS unique to this invocation and revision'
  echo '  warm=each revision warms unchanged temporary source first, then both receive the same recorded scenario commit before measured builds'
  echo '  cold=fresh revision namespaces plus --no-cache; independent cold measurements, not a warm changed-source comparison'
  echo '  instrumentation=temporary Dockerfile copies add Cargo verbosity only; original Dockerfiles/commands/binary sets remain unchanged'
  echo '  services=all twelve release services, batches up to three, sequential one-service Docker builds'
  echo "  resource_gates=MemAvailable >= ${default_min_mem_available_kib} KiB; SwapFree >= ${default_min_swap_free_kib} KiB; recent OOM, prior compiler/exit, systemd and health checks fail closed"
  echo '  apply_prerequisite=already-running low-priority systemd service containing this process plus explicit read-only health units; the harness never starts it'
  echo '  outputs=sanitized build evidence, source/probe/instrumentation hashes, tool identities, per-image build/Cargo/inspection timings, sampled resource minima, phase timings, package/cache state'
  echo '  timing_limits=Cargo RUN duration is not linker time; strict V2 final-image inspection and manifest issuance remain in the official release flow'
  echo '  prohibited=production checkout mutation, production tag/cache use, rollout, provider access, lifecycle commands, credential reads, global cache pruning'
  echo 'PLAN_ONLY: no Docker command, Rust compilation, checkout, output write, or background work made'
else
  run_apply
fi
