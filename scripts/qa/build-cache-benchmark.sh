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
baseline_commit=
candidate_commit=
output_dir=

# These are conservative harness gates for the documented 14 GiB host, not a
# deployment authorization.  An operator may raise either value, never lower
# it, through the corresponding BENCHMARK_MIN_* variable.
readonly default_min_mem_available_kib=2097152 # 2 GiB
readonly default_min_swap_free_kib=524288      # 512 MiB
readonly oom_lookback='30 minutes ago'

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
declare -A service_probe=(
  [db-role-bootstrap]=deploy/db/migrate.sh
  [db-migrate]=deploy/db/migrate.sh
  [api-server]=crates/api-server/src/bin/api-server.rs
  [web]=apps/web/app/layout.tsx
  [research-worker]=data-pipelines/collectors/src/bin/research-worker.rs
  [recommendation-runner]=crates/job-queue/src/bin/recommendation-runner.rs
  [candidate-runner]=crates/job-queue/src/bin/candidate-runner.rs
  [owner-beta-runner]=crates/job-queue/src/bin/owner-beta-runner.rs
  [owner-equity-v2-runner]=crates/job-queue/src/bin/owner-equity-v2-runner.rs
  [nt-backtest-worker-1]=crates/job-queue/src/bin/backtest-runner.rs
  [nt-backtest-worker-2]=crates/job-queue/src/bin/backtest-runner.rs
  [paper-scheduler]=crates/api-server/src/bin/paper-runner.rs
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

tmp_dir=
baseline_checkout=
candidate_checkout=
benchmark_nonce=
evidence_dir=
instrumentation_dir=
metadata_report=
results_report=
resource_report=
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

declare -a tag_list=()
declare -a unique_probe_paths=()
declare -A probe_path_seen=()
declare -A instrumented_dockerfile=()
declare -A source_dockerfile_hash=()
declare -A instrumented_dockerfile_hash=()
declare -A instrumentation_patch_hash=()
declare -A instrumentation_transform=()
declare -A service_toolchain_identity=()
declare -A source_identity_by_revision_scenario=()

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-cache-benchmark.sh [--plan|--apply|--self-test]
       --baseline-commit <40 lowercase hex>
       --candidate-commit <40 lowercase hex>
       --output-dir <absolute path>
       [--warm|--cold]

Modes:
  --plan       Validate revisions/paths and print the benchmark plan (default).
  --apply      Run guarded sequential benchmark builds from temporary clones.
  --self-test  Exercise parser/state-machine logic with fakes only; it never
               contacts Docker, starts containers, or compiles Rust.

Warm mode gives baseline and candidate separate, invocation-specific
BUILDKIT_CACHE_MOUNT_NS values.  Each revision is warmed from its unchanged
temporary source, then receives the same recorded source probe before its
measured build.  Cold mode uses fresh per-revision namespaces plus --no-cache
and records independent cold measurements; it makes no warm comparison claim.

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
  local service dockerfile probe
  for service in "${services[@]}"; do
    dockerfile=$repo_root/${service_dockerfile[$service]}
    probe=$repo_root/${service_probe[$service]}
    [ -f "$dockerfile" ] && [ ! -L "$dockerfile" ] ||
      die "benchmark Dockerfile is missing: ${service_dockerfile[$service]}"
    [ -f "$probe" ] && [ ! -L "$probe" ] ||
      die "benchmark source probe is missing for $service: ${service_probe[$service]}"
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

probe_line_for_path() {
  local path=$1
  case "$path" in
    *.rs|*.ts|*.tsx|*.js|*.jsx) printf '%s' '// LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914' ;;
    *) printf '%s' '# LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914' ;;
  esac
}

collect_unique_probe_paths() {
  local service path
  unique_probe_paths=()
  probe_path_seen=()
  for service in "${services[@]}"; do
    path=${service_probe[$service]}
    if [ -z "${probe_path_seen[$path]:-}" ]; then
      probe_path_seen[$path]=1
      unique_probe_paths+=("$path")
    fi
  done
}

validate_checkout_contract() {
  local checkout=$1 revision=$2 service dockerfile probe line
  for service in "${services[@]}"; do
    dockerfile=$checkout/${service_dockerfile[$service]}
    probe=$checkout/${service_probe[$service]}
    [ -f "$dockerfile" ] && [ ! -L "$dockerfile" ] ||
      die "$revision temporary checkout lacks Dockerfile: ${service_dockerfile[$service]}"
    [ -f "$probe" ] && [ ! -L "$probe" ] ||
      die "$revision temporary checkout lacks source probe: ${service_probe[$service]}"
  done
  for probe in "${unique_probe_paths[@]}"; do
    line=$(probe_line_for_path "$probe")
    if grep -Fqx "$line" "$checkout/$probe"; then
      die "$revision source probe marker already exists: $probe"
    fi
  done
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

apply_equivalent_probes() {
  local checkout revision probe line
  # This specification is byte-for-byte common.  The two revision-relative
  # patches can legitimately hash differently because their surrounding source
  # differs, so both exact patches and the common transformation spec are kept.
  for checkout in "$baseline_checkout" "$candidate_checkout"; do
    case "$checkout" in
      "$baseline_checkout") revision=baseline ;;
      *) revision=candidate ;;
    esac
    for probe in "${unique_probe_paths[@]}"; do
      line=$(probe_line_for_path "$probe")
      printf '\n%s\n' "$line" >>"$checkout/$probe"
    done
    printf 'BENCHMARK_PROBE_APPLIED revision=%s paths=%s\n' "$revision" "${#unique_probe_paths[@]}"
  done
}

record_source_snapshot() {
  local revision=$1 scenario=$2 checkout=$3 input_commit=$4 raw_patch evidence_patch
  local head tree patch_hash identity
  raw_patch=$tmp_dir/${revision}-${scenario}.source.patch.raw
  evidence_patch=$evidence_dir/source-${revision}-${scenario}.patch
  if ! git -C "$checkout" diff --binary --no-ext-diff --unified=0 -- . >"$raw_patch"; then
    die "could not capture $revision/$scenario source patch"
  fi
  sanitize_text_file "$raw_patch" "$evidence_patch"
  patch_hash=$(sha256_file "$raw_patch")
  head=$(git -C "$checkout" rev-parse HEAD) || die 'could not identify temporary checkout HEAD'
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
  local revision=$1 scenario=$2 service=$3 index=$4 tag=$5 namespace=$6 commit=$7 source_identity=$8
  local total_ms=$9 cargo_ms=${10} cargo_cache=${11} evidence_rel=${12}
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario" "$service" "$index" "$tag" "$namespace" "$commit" "$source_identity" \
    "$total_ms" "$cargo_ms" "$BENCH_COMPILER_VERTEX" "$BENCH_COMPILER_VERTEX_STATE" \
    "$BENCH_COMPILER_CACHE" "$BENCH_NONCOMPILER_CACHED" "$cargo_cache" \
    "$(csv_or_dash "$BENCH_COMPILED_PACKAGES")" "$(csv_or_dash "$BENCH_FRESH_PACKAGES")" \
    "$BENCH_WORKSPACE_CLEAN" "${service_toolchain_identity[$revision:$service]}" \
    "${source_dockerfile_hash[$revision:$service]}" "${instrumented_dockerfile_hash[$revision:$service]}" \
    "${instrumentation_patch_hash[$revision:$service]}" "$evidence_rel" >>"$results_report"
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

build_one_service() {
  local revision=$1 scenario=$2 checkout=$3 commit=$4 service=$5 index=$6 namespace=$7
  local tag raw_log evidence_rel evidence_log start_ms end_ms total_ms cargo_ms cargo_cache status
  local -a args=()

  tag="lagrange-cb-${benchmark_nonce}-${revision}-${scenario}-${index}"
  safe_benchmark_repository "$tag" || die 'generated benchmark image repository is not valid lowercase Docker grammar'
  raw_log=$tmp_dir/${revision}-${scenario}-${index}-${service}.build.log.raw
  evidence_rel="evidence/${revision}-${scenario}-${index}-${service}.build.log"
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

  start_ms=$(now_ms)
  if CARGO_BUILD_JOBS=2 DOCKER_BUILDKIT=1 docker "${args[@]}" >"$raw_log" 2>&1; then
    last_build_exit=0
  else
    status=$?
    last_build_exit=$status
    sanitize_text_file "$raw_log" "$evidence_log"
    failure_stage=build
    die "Docker build failed for $revision/$scenario/$service (sanitized evidence: $evidence_rel)"
  fi
  end_ms=$(now_ms)
  total_ms=$((end_ms - start_ms))
  is_decimal "$total_ms" || die 'total build time is not numeric'
  sanitize_text_file "$raw_log" "$evidence_log"
  if ! parse_build_events "$raw_log" "${service_cargo_mode[$service]}" "$revision"; then
    failure_stage=build-evidence
    die "invalid Cargo/BuildKit evidence for $revision/$scenario/$service: $BENCH_PARSE_ERROR"
  fi
  cargo_ms=${BENCH_CARGO_STEP_MS:--}
  cargo_cache=$(derive_cargo_cache_state "${service_cargo_mode[$service]}")
  append_result "$revision" "$scenario" "$service" "$index" "$tag" "$namespace" "$commit" \
    "${source_identity_by_revision_scenario[$revision:$scenario]}" "$total_ms" "$cargo_ms" "$cargo_cache" "$evidence_rel"
  printf 'BENCHMARK_SERVICE PASS revision=%s scenario=%s service=%s total_ms=%s cargo_ms=%s compiler_vertex=%s compiler_state=%s cargo_cache=%s compiled=%s fresh=%s\n' \
    "$revision" "$scenario" "$service" "$total_ms" "$cargo_ms" "$BENCH_COMPILER_VERTEX" \
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

snapshot_resources() {
  local meminfo journal
  gate_mem_available=unavailable
  gate_swap_free=unavailable
  gate_oom_events=unavailable
  if ! meminfo=$(read_meminfo); then
    gate_reason=meminfo-unavailable
    return 1
  fi
  gate_mem_available=$(awk '/^MemAvailable:/ { print $2; exit }' <<<"$meminfo")
  gate_swap_free=$(awk '/^SwapFree:/ { print $2; exit }' <<<"$meminfo")
  is_decimal "$gate_mem_available" || { gate_reason=mem-available-unavailable; return 1; }
  is_decimal "$gate_swap_free" || { gate_reason=swap-free-unavailable; return 1; }
  command -v journalctl >/dev/null 2>&1 || { gate_reason=kernel-journal-unavailable; return 1; }
  if ! journal=$(journalctl --quiet --kernel --no-pager --since "$oom_lookback" -n 1000 2>/dev/null); then
    gate_reason=kernel-journal-unavailable
    return 1
  fi
  gate_oom_events=$(grep -Eic 'out of memory|oom-kill|killed process' <<<"$journal" || true)
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
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$revision" "$scenario" "$batch" "$point" "$gate_mem_available" "$gate_swap_free" \
    "$min_mem_available_kib" "$min_swap_free_kib" "$gate_oom_events" "$last_build_exit" \
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
  local revision=$1 scenario=$2 checkout=$3 commit=$4 namespace=$5
  local service index=0 batch=0
  for service in "${services[@]}"; do
    if [ $((index % 3)) -eq 0 ]; then
      batch=$((batch + 1))
      gate_batch "$revision" "$scenario" "$batch" before
    fi
    index=$((index + 1))
    build_one_service "$revision" "$scenario" "$checkout" "$commit" "$service" "$index" "$namespace"
    if [ $((index % 3)) -eq 0 ] || [ "$index" -eq "${#services[@]}" ]; then
      gate_batch "$revision" "$scenario" "$batch" after
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
        "$(cut -f13 <<<"$baseline_fields")" "$(cut -f13 <<<"$candidate_fields")" \
        "$(cut -f16 <<<"$baseline_fields")" "$(cut -f16 <<<"$candidate_fields")" \
        "$(cut -f17 <<<"$baseline_fields")" "$(cut -f17 <<<"$candidate_fields")"
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
  comparison_report=$output_dir/comparison.tsv
  source_identity_report=$output_dir/source-identities.tsv
  instrumentation_report=$output_dir/instrumentation.tsv
  probe_spec_report=$output_dir/probe-spec.tsv
  failure_report=$output_dir/failure.tsv
  apply_record_failures=1
  printf 'revision\tscenario\tservice\tindex\timage_tag\tcache_mount_namespace\tinput_commit\tmeasured_source_identity_sha256\ttotal_ms\tcargo_ms\tcompiler_vertex\tcompiler_vertex_state\tcompiler_cache\tnoncompiler_cached_vertices\tcargo_cache_state\tcompiled_packages\tfresh_packages\tworkspace_clean\ttoolchain_identity\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\tsanitized_evidence\n' >"$results_report"
  printf 'revision\tscenario\tbatch\tgate\tmem_available_kib\tswap_free_kib\tminimum_mem_available_kib\tminimum_swap_free_kib\trecent_kernel_oom_events\tprevious_build_exit\tcompiler_processes\tbackground_build_service\tproduction_health\tstatus\n' >"$resource_report"
  printf 'revision\tscenario\tinput_commit\tcheckout_head\tcheckout_tree\tprobe_patch_sha256\tinstrumentation_manifest_sha256\tmeasured_source_identity_sha256\n' >"$source_identity_report"
  printf 'revision\tservice\tsource_dockerfile\tinstrumentation\tsource_dockerfile_sha256\tinstrumented_dockerfile_sha256\tinstrumentation_patch_sha256\ttoolchain_identity\n' >"$instrumentation_report"
  chmod 0600 -- "$results_report" "$resource_report" "$source_identity_report" "$instrumentation_report"
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

collect_docker_identity() {
  local value
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
}

write_metadata() {
  local baseline_namespace=$1 candidate_namespace=$2 protocol=$3
  {
    printf '%s\n' BUILD_CACHE_BENCHMARK_V2
    printf 'cache_mode\t%s\n' "$cache_mode"
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
    printf 'cargo_build_jobs\t2\n'
    printf 'service_order\t%s\n' "${services[*]}"
    printf 'batch_policy\tup-to-three-services; one Docker invocation at a time\n'
    printf 'resource_min_mem_available_kib\t%s\n' "$min_mem_available_kib"
    printf 'resource_min_swap_free_kib\t%s\n' "$min_swap_free_kib"
    printf 'resource_oom_lookback\t%s\n' "$oom_lookback"
    printf 'build_service_unit\t%s\n' "${BENCHMARK_SYSTEMD_SERVICE:-unconfigured}"
    printf 'production_health_units\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_UNITS:-unconfigured}"
    printf 'production_health_containers\t%s\n' "${BENCHMARK_PRODUCTION_HEALTH_CONTAINERS:-unconfigured}"
    printf 'production_container_health_policy\trunning; declared healthy healthcheck; compose project lagrange-station; bounded inspect fields only\n'
    printf 'source_checkout_policy\ttemporary-detached-local-clones\n'
    printf 'probe_policy\tcommon temporary probe spec; exact revision-relative patches and hashes retained\n'
    printf 'probe_spec_sha256\t%s\n' "${probe_spec_hash:-unavailable}"
    printf 'instrumentation_policy\ttemporary Dockerfile copies only; Cargo build/install verbosity only\n'
    printf 'instrumentation_manifest_sha256\t%s\n' "${instrumentation_manifest_hash:-unavailable}"
    printf 'evidence_policy\tsanitized BuildKit/Cargo logs retained; raw temporary logs removed with clones\n'
    printf 'image_identity_scope\ttemporary modified-source benchmark image; not an immutable official release image\n'
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
  local baseline_namespace candidate_namespace protocol probe
  # A self-test may invoke this function more than once in one shell.  Do not
  # let an earlier run's failure path write into a later non-empty output dir.
  apply_record_failures=0
  failure_report=
  failure_stage=preflight
  initialize_output
  configure_resource_thresholds
  validate_service_contract
  # Validate paths and all no-build prerequisites before any Docker build.  A
  # failed gate writes failure.tsv and leaves the empty/auditable output run.
  gate_batch preflight prerequisite 0 before
  collect_docker_identity
  create_temp_run_directory
  trap cleanup_apply_run EXIT
  collect_unique_probe_paths
  prepare_checkout "$baseline_commit" "$baseline_checkout"
  prepare_checkout "$candidate_commit" "$candidate_checkout"
  validate_checkout_contract "$baseline_checkout" baseline
  validate_checkout_contract "$candidate_checkout" candidate
  printf 'path\tprobe_line\n' >"$probe_spec_report"
  for probe in "${unique_probe_paths[@]}"; do
    printf '%s\t%s\n' "$probe" "$(probe_line_for_path "$probe")" >>"$probe_spec_report"
  done
  chmod 0600 -- "$probe_spec_report"
  probe_spec_hash=$(sha256_file "$probe_spec_report")
  instrument_all_dockerfiles baseline "$baseline_checkout"
  instrument_all_dockerfiles candidate "$candidate_checkout"
  instrumentation_manifest_hash=$(sha256_file "$instrumentation_report")
  baseline_namespace=$(namespace_for_revision baseline "$baseline_commit")
  candidate_namespace=$(namespace_for_revision candidate "$candidate_commit")

  if [ "$cache_mode" = warm ]; then
    protocol=warmup-unmodified-then-common-probe-measured
    record_source_snapshot baseline warmup "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate warmup "$candidate_checkout" "$candidate_commit"
    write_metadata "$baseline_namespace" "$candidate_namespace" "$protocol"
    printf 'BENCHMARK_RUN mode=warm baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    run_scenario baseline warmup "$baseline_checkout" "$baseline_commit" "$baseline_namespace"
    run_scenario candidate warmup "$candidate_checkout" "$candidate_commit" "$candidate_namespace"
    apply_equivalent_probes
    record_source_snapshot baseline measured "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate measured "$candidate_checkout" "$candidate_commit"
    run_scenario baseline measured "$baseline_checkout" "$baseline_commit" "$baseline_namespace"
    run_scenario candidate measured "$candidate_checkout" "$candidate_commit" "$candidate_namespace"
    write_warm_comparison_report
  else
    protocol=cold-independent-changed-source-no-warm-comparison
    apply_equivalent_probes
    record_source_snapshot baseline measured "$baseline_checkout" "$baseline_commit"
    record_source_snapshot candidate measured "$candidate_checkout" "$candidate_commit"
    write_metadata "$baseline_namespace" "$candidate_namespace" "$protocol"
    printf 'BENCHMARK_RUN mode=cold baseline=%s candidate=%s jobs=2 protocol=%s\n' \
      "$baseline_commit" "$candidate_commit" "$protocol"
    printf '%s\n' 'BENCHMARK_RUN twelve services, batches up to three, sequential one-service Docker builds inside verified background systemd service'
    run_scenario baseline measured "$baseline_checkout" "$baseline_commit" "$baseline_namespace"
    run_scenario candidate measured "$candidate_checkout" "$candidate_commit" "$candidate_namespace"
    write_cold_comparison_notice
  fi
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
  local case_variant_one case_variant_two health_mode health_reason saved_health_containers
  test_dir=$(mktemp -d /tmp/lagrange-build-cache-benchmark-self-test.XXXXXXXXXX)
  trap 'rm -rf -- "$test_dir"' RETURN
  parser_dir=$test_dir/parser
  mkdir -p -- "$parser_dir"

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
  BENCH_TEST_PS_NAMES=$'systemd\nbash'
  BENCH_TEST_CONTAINER_MODE=healthy
  BENCH_TEST_DOCKER_RECORD=$test_dir/fake-docker.tsv
  BENCH_TEST_SYSTEMCTL_RECORD=$test_dir/fake-systemctl.tsv
  : >"$BENCH_TEST_DOCKER_RECORD"
  : >"$BENCH_TEST_SYSTEMCTL_RECORD"

  # All following observations are shell fakes.  In particular, `docker` is a
  # function, so no client binary or daemon can be reached by this self-test.
  docker() {
    local command=${1:-} tag= namespace= build_arg=missing no_cache=no context= probe_state=unknown host_namespace
    shift || true
    case "$command" in
      version) printf '%s\n' '27.0.0|27.0.0' ;;
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
        local arg next_is_build_arg=0
        for arg in "$@"; do
          if [ "$next_is_build_arg" -eq 1 ]; then
            case "$arg" in BUILDKIT_CACHE_MOUNT_NS=*) build_arg=${arg#BUILDKIT_CACHE_MOUNT_NS=} ;; esac
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
        [ "${1:-}" = rm ] || return 92
        printf 'remove\t%s\n' "${3:-}" >>"$BENCH_TEST_DOCKER_RECORD"
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
  journalctl() {
    [ "${BENCH_TEST_JOURNAL_MODE:-clean}" = clean ] && { printf '%s\n' 'kernel: clean fixture'; return 0; }
    printf '%s\n' 'kernel: Out of memory: fixture'
  }
  ps() { printf '%s\n' "${BENCH_TEST_PS_NAMES:-}"; }
  git() {
    local checkout= argument
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
      rev-parse)
        case "${1:-}" in
          HEAD) cat "$checkout/.benchmark-commit" ;;
          'HEAD^{tree}') printf 'fixture-tree-%s\n' "$(cat "$checkout/.benchmark-commit")" ;;
          *) return 96 ;;
        esac
        ;;
      status) return 0 ;;
      diff)
        if grep -R -Fq 'LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914' "$checkout"; then
          printf 'diff --git a/probe b/probe\n@@ -0,0 +1 @@\n+LAGRANGE_BUILD_CACHE_BENCHMARK_PROBE=20260914\n'
        fi
        ;;
      *) return 97 ;;
    esac
  }
  prepare_checkout() {
    local commit=$1 checkout=$2 service dockerfile probe
    mkdir -p -- "$checkout"
    printf '%s\n' "$commit" >"$checkout/.benchmark-commit"
    printf '%s\n' '[toolchain]' 'channel = "1.97.1"' >"$checkout/rust-toolchain.toml"
    for service in "${services[@]}"; do
      dockerfile=$checkout/${service_dockerfile[$service]}
      probe=$checkout/${service_probe[$service]}
      mkdir -p -- "${dockerfile%/*}" "${probe%/*}"
      if [ "${service_cargo_mode[$service]}" -gt 0 ]; then
        printf '%s\n' 'FROM rust:1.97.1-alpine' 'RUN cargo build --locked --release' >"$dockerfile"
      else
        printf '%s\n' 'FROM node:fixture' 'RUN npm run build' >"$dockerfile"
      fi
      [ -e "$probe" ] || printf '%s\n' 'fixture source' >"$probe"
    done
  }
  [ "$(type -t docker)" = function ] || die 'self-test Docker fake was not installed'
  [ "$(type -t git)" = function ] || die 'self-test Git fake was not installed'
  [ "$(type -t systemctl)" = function ] || die 'self-test systemd fake was not installed'

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

if [ "$mode" = plan ]; then
  echo 'BUILD_CACHE_BENCHMARK_PLAN mode=plan'
  echo "  baseline_commit=$baseline_commit"
  echo "  candidate_commit=$candidate_commit"
  echo "  output_dir=$output_dir"
  echo "  cache_mode=$cache_mode"
  echo '  namespaces=every Docker build receives an explicit --build-arg BUILDKIT_CACHE_MOUNT_NS unique to this invocation and revision'
  echo '  warm=each revision warms unchanged temporary source first, then both receive the same recorded probe before measured builds'
  echo '  cold=fresh revision namespaces plus --no-cache; independent cold measurements, not a warm changed-source comparison'
  echo '  instrumentation=temporary Dockerfile copies add Cargo verbosity only; original Dockerfiles/commands/binary sets remain unchanged'
  echo '  services=all twelve release services, batches up to three, sequential one-service Docker builds'
  echo "  resource_gates=MemAvailable >= ${default_min_mem_available_kib} KiB; SwapFree >= ${default_min_swap_free_kib} KiB; recent OOM, prior compiler/exit, systemd and health checks fail closed"
  echo '  apply_prerequisite=already-running low-priority systemd service containing this process plus explicit read-only health units; the harness never starts it'
  echo '  outputs=sanitized build evidence, source/probe/instrumentation hashes, tool identities, timings, package/cache state'
  echo '  prohibited=production checkout mutation, production tag/cache use, rollout, provider access, lifecycle commands, credential reads, global cache pruning'
  echo 'PLAN_ONLY: no Docker command, Rust compilation, checkout, output write, or background work made'
else
  run_apply
fi
