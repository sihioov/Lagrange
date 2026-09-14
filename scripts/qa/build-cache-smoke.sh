#!/usr/bin/env bash
# Provider-free build-cache smoke harness. --plan is read-only; --apply uses
# only a disposable fixture; --self-test drives --apply through a fake Docker.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/../.." && pwd -P)
default_fixture_dir=$repo_root/tests/fixtures/build-cache

mode=plan
mode_seen=0
fixture_dir=$default_fixture_dir
output_dir=
output_dir_seen=0

tmp_dir=
source_dir=
smoke_nonce=
case_report=
metadata_report=
cleanup_report=
log_dir=
binary_dir=
tool_identity=
docker_client_version=
git_version=
fixture_toolchain=

base_commit=
app_source_commit=
workspace_library_commit=
build_script_commit=
external_dependency_commit=
data_commit=
commit_only_commit=

declare -a owned_image_tags=()

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-cache-smoke.sh [--plan|--apply|--self-test]
       [--fixture-dir ABSOLUTE_PATH] [--output-dir ABSOLUTE_PATH]

Modes:
  --plan       Validate and print the fixture/cache plan (default). An optional
               output directory is validated but never created or written.
  --apply      Build and run disposable fixture cases with Docker. Requires an
               empty output directory outside this original checkout.
  --self-test  Exercise CLI validation, parsing, and the --apply control flow
               with a fake Docker command; it never contacts Docker and never
               compiles Rust.

The apply harness uses only temporary fixture copies, fixture-scoped BuildKit
cache IDs, and one unique temporary image tag per case. It never uses the
production Compose file, production cache IDs/tags, provider credentials, a
prune operation, or a production checkout mutation.
EOF
}

die() {
  echo "build-cache-smoke: $*" >&2
  exit 1
}

is_exact_commit() {
  local value=${1:-}
  [ -n "$value" ] || return 1
  printf '%s' "$value" | grep -Eq '^[0-9a-f]{40}$'
}

safe_absolute_path() {
  local path=$1 label=$2
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
    /|/tmp|/var|/var/lib|/usr|/usr/local|/opt)
      die "$label is too broad: $path"
      ;;
  esac
  local probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

validate_output_dir() {
  safe_absolute_path "$output_dir" output-dir
  case "$output_dir" in
    "$repo_root"|"$repo_root"/*)
      die 'output-dir must be outside the original repository checkout'
      ;;
  esac
  if [ -e "$output_dir" ]; then
    [ -d "$output_dir" ] && [ ! -L "$output_dir" ] ||
      die 'output-dir must be a regular directory when it already exists'
  else
    local parent=${output_dir%/*}
    [ -n "$parent" ] || parent=/
    [ -d "$parent" ] && [ ! -L "$parent" ] ||
      die 'output-dir parent must exist and must not be a symlink'
  fi
}

prepare_output_dir() {
  validate_output_dir
  if [ -e "$output_dir" ]; then
    [ -z "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ] ||
      die 'output-dir must be empty; refusing to overwrite smoke evidence'
  else
    mkdir -m 0700 -- "$output_dir" || die 'could not create output-dir'
  fi
  chmod 0700 -- "$output_dir"
}

validate_fixture() {
  safe_absolute_path "$fixture_dir" fixture-dir
  [ -d "$fixture_dir" ] && [ ! -L "$fixture_dir" ] ||
    die "fixture-dir must be a regular directory: $fixture_dir"
  local required path
  required=(
    Cargo.toml
    Cargo.lock
    rust-toolchain.toml
    Dockerfile
    .dockerignore
    fixture-lib/Cargo.toml
    fixture-lib/src/lib.rs
    fixture-app/Cargo.toml
    fixture-app/build.rs
    fixture-app/data/embedded.txt
    fixture-app/src/bin/cache-bin-a.rs
    fixture-app/src/bin/cache-bin-b.rs
  )
  for path in "${required[@]}"; do
    [ -f "$fixture_dir/$path" ] && [ ! -L "$fixture_dir/$path" ] ||
      die "fixture file is missing or symlinked: $path"
  done
  grep -Fq 'name = "itoa"' "$fixture_dir/Cargo.lock" ||
    die 'fixture lockfile is missing the pinned itoa registry dependency'
  grep -Fq 'version = "1.0.18"' "$fixture_dir/Cargo.lock" ||
    die 'fixture lockfile itoa version is not the repository-approved pin'
  grep -Fq '8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682' \
    "$fixture_dir/Cargo.lock" ||
    die 'fixture lockfile itoa checksum does not match the repository lockfile'
  grep -Fq 'build-cache-fixture-lib' "$fixture_dir/fixture-app/Cargo.toml" ||
    die 'fixture must contain a local workspace dependency'
  grep -Fq '[[bin]]' "$fixture_dir/fixture-app/Cargo.toml" ||
    die 'fixture must declare binary targets'
  [ "$(grep -Fc 'name = "cache-bin-' "$fixture_dir/fixture-app/Cargo.toml")" -eq 2 ] ||
    die 'fixture must contain exactly two binary targets'
  grep -Fq 'include_str!' "$fixture_dir/fixture-app/src/bin/cache-bin-a.rs" ||
    die 'fixture binary A must embed data at compile time'
  grep -Fq 'cargo:rustc-env=CACHE_FIXTURE_COMMIT' "$fixture_dir/fixture-app/build.rs" ||
    die 'fixture build script must emit the compile-time commit marker'
  grep -Fq 'generated-v1|commit=' "$fixture_dir/fixture-app/build.rs" ||
    die 'fixture build script must expose its generated-output source marker'
  grep -Fq 'cargo:rerun-if-changed=data/embedded.txt' "$fixture_dir/fixture-app/build.rs" ||
    die 'fixture build script must watch embedded data'
  grep -Fq 'cargo:rerun-if-env-changed=CACHE_FIXTURE_BUILD_SETTING' \
    "$fixture_dir/fixture-app/build.rs" ||
    die 'fixture build script must watch the build setting'
  grep -Fq 'ENV CARGO_BUILD_JOBS=2' "$fixture_dir/Dockerfile" ||
    die 'fixture Dockerfile must cap Cargo parallelism at two jobs'
  grep -Fq 'ARG CACHE_FIXTURE_RUN_TOKEN=initial' "$fixture_dir/Dockerfile" ||
    die 'fixture Dockerfile must declare the compile RUN invalidation argument'
  grep -Fq 'test -n "$CACHE_FIXTURE_RUN_TOKEN"' "$fixture_dir/Dockerfile" ||
    die 'fixture compile RUN must consume its invalidation argument'
  [ "$(grep -Foc 'cargo clean --workspace --release --locked' "$fixture_dir/Dockerfile" || true)" -eq 1 ] ||
    die 'fixture Dockerfile must clean the workspace exactly once before building'
  [ "$(grep -Foc 'cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-a' "$fixture_dir/Dockerfile" || true)" -eq 1 ] ||
    die 'fixture Dockerfile must build binary A once with verbose Cargo events'
  [ "$(grep -Foc 'cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-b' "$fixture_dir/Dockerfile" || true)" -eq 1 ] ||
    die 'fixture Dockerfile must build binary B once with verbose Cargo events'
  local phase_a_line build_a_line phase_b_line build_b_line staging_dir_line copy_a_line copy_b_line
  phase_a_line=$(grep -nF 'SMOKE_CARGO_PHASE bin=cache-bin-a' "$fixture_dir/Dockerfile" | cut -d: -f1)
  build_a_line=$(grep -nF 'cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-a' "$fixture_dir/Dockerfile" | cut -d: -f1)
  phase_b_line=$(grep -nF 'SMOKE_CARGO_PHASE bin=cache-bin-b' "$fixture_dir/Dockerfile" | cut -d: -f1)
  build_b_line=$(grep -nF 'cargo build --locked --release --verbose --package build-cache-fixture-app --bin cache-bin-b' "$fixture_dir/Dockerfile" | cut -d: -f1)
  [ -n "$phase_a_line" ] && [ -n "$build_a_line" ] && [ -n "$phase_b_line" ] && [ -n "$build_b_line" ] ||
    die 'fixture Dockerfile is missing explicit binary phase evidence'
  [ "$phase_a_line" -lt "$build_a_line" ] && [ "$build_a_line" -lt "$phase_b_line" ] && [ "$phase_b_line" -lt "$build_b_line" ] ||
    die 'fixture binary phase/build order is not exact'
  staging_dir_line=$(grep -nF 'mkdir -p /build/target/release' "$fixture_dir/Dockerfile" | cut -d: -f1)
  copy_a_line=$(grep -nF 'cp /cargo-target/release/cache-bin-a /build/target/release/cache-bin-a' "$fixture_dir/Dockerfile" | cut -d: -f1)
  copy_b_line=$(grep -nF 'cp /cargo-target/release/cache-bin-b /build/target/release/cache-bin-b' "$fixture_dir/Dockerfile" | cut -d: -f1)
  [ -n "$staging_dir_line" ] && [ -n "$copy_a_line" ] && [ -n "$copy_b_line" ] &&
    [ "$build_b_line" -lt "$staging_dir_line" ] && [ "$staging_dir_line" -lt "$copy_a_line" ] && [ "$copy_a_line" -lt "$copy_b_line" ] ||
    die 'fixture binary output staging order is not exact'
  grep -Fq 'cp /cargo-target/release/cache-bin-a /build/target/release/cache-bin-a' \
    "$fixture_dir/Dockerfile" ||
    die 'fixture Dockerfile must stage binary A outside the target cache'
  grep -Fq 'cp /cargo-target/release/cache-bin-b /build/target/release/cache-bin-b' \
    "$fixture_dir/Dockerfile" ||
    die 'fixture Dockerfile must stage binary B outside the target cache'
  for cache_id in build-cache-fixture-registry-v1- build-cache-fixture-git-v1- build-cache-fixture-target-v1-; do
    grep -Fq "id=$cache_id" "$fixture_dir/Dockerfile" || die "fixture cache ID is missing: $cache_id"
  done
  grep -Fq 'target=/usr/local/cargo/registry' "$fixture_dir/Dockerfile" || die 'fixture registry cache target is missing'
  grep -Fq 'target=/usr/local/cargo/git' "$fixture_dir/Dockerfile" || die 'fixture git cache target is missing'
  grep -Fq 'target=/cargo-target' "$fixture_dir/Dockerfile" || die 'fixture target cache target is missing'
  [ "$(grep -Foc 'sharing=locked' "$fixture_dir/Dockerfile" || true)" -eq 3 ] || die 'fixture cache mounts must all use sharing=locked'
  if grep -Eq 'lagrange-cargo-(registry|git|target)' "$fixture_dir/Dockerfile"; then
    die 'fixture must never use a production cache ID'
  fi
}

# Globals filled from fixture logs only: names, vertex state, and durations.
SMOKE_COMPILE_VERTEX=
SMOKE_COMPILE_STATUS=unknown
SMOKE_COMPILED_PACKAGES=
SMOKE_FRESH_PACKAGES=
SMOKE_BIN_A_COMPILED_PACKAGES=
SMOKE_BIN_A_FRESH_PACKAGES=
SMOKE_BIN_B_COMPILED_PACKAGES=
SMOKE_BIN_B_FRESH_PACKAGES=
SMOKE_BIN_A_PHASE_COUNT=0
SMOKE_BIN_B_PHASE_COUNT=0
SMOKE_CARGO_FINISHED_COUNT=0
SMOKE_CARGO_STEP_MS=
SMOKE_PARSE_ERROR=
SMOKE_ASSERT_ERROR=

reset_build_events() {
  SMOKE_COMPILE_VERTEX=
  SMOKE_COMPILE_STATUS=unknown
  SMOKE_COMPILED_PACKAGES=
  SMOKE_FRESH_PACKAGES=
  SMOKE_BIN_A_COMPILED_PACKAGES=
  SMOKE_BIN_A_FRESH_PACKAGES=
  SMOKE_BIN_B_COMPILED_PACKAGES=
  SMOKE_BIN_B_FRESH_PACKAGES=
  SMOKE_BIN_A_PHASE_COUNT=0
  SMOKE_BIN_B_PHASE_COUNT=0
  SMOKE_CARGO_FINISHED_COUNT=0
  SMOKE_CARGO_STEP_MS=
  SMOKE_PARSE_ERROR=
}

csv_has() {
  local list=$1 wanted=$2 value
  IFS=',' read -r -a values <<<"$list"
  for value in "${values[@]}"; do [ "$value" = "$wanted" ] && return 0; done
  return 1
}

csv_or_dash() { [ -n "$1" ] && printf '%s' "$1" || printf '%s' '-'; }

packages_for_phase() {
  local log=$1 wanted_phase=$2 wanted_event=$3
  awk -v vertex="#$SMOKE_COMPILE_VERTEX" -v wanted_phase="$wanted_phase" -v wanted_event="$wanted_event" '
    $1 == vertex {
      if (index($0, "SMOKE_CARGO_PHASE bin=cache-bin-a") > 0) { phase = "a"; next }
      if (index($0, "SMOKE_CARGO_PHASE bin=cache-bin-b") > 0) { phase = "b"; next }
      if (phase == "" || (wanted_phase != "all" && phase != wanted_phase)) next
      for (field_number = 1; field_number < NF; field_number++) {
        if ($field_number == wanted_event) {
          package = $(field_number + 1)
          sub(/[^A-Za-z0-9_.-].*$/, "", package)
          if (package ~ /^[A-Za-z0-9_.-]+$/) print package
        }
      }
    }
  ' "$log" | sort -u | paste -sd, -
}

parse_build_events() {
  local log=$1 vertices vertex_count step_id
  reset_build_events
  if [ ! -f "$log" ]; then SMOKE_PARSE_ERROR='fixture build event log is missing'; return 1; fi
  vertices=$(sed -nE 's/^#([0-9]+)[[:space:]]+\[[^]]*\][[:space:]]+RUN.*cargo clean --workspace --release --locked.*/\1/p' "$log" | sort -u)
  vertex_count=$(printf '%s\n' "$vertices" | sed '/^$/d' | wc -l | tr -d '[:space:]')
  if [ "$vertex_count" -ne 1 ]; then SMOKE_PARSE_ERROR='fixture build log must identify exactly one compile RUN vertex'; return 1; fi
  SMOKE_COMPILE_VERTEX=$vertices
  if grep -Eq "^#${SMOKE_COMPILE_VERTEX}[[:space:]]+CACHED([[:space:]]*)$" "$log"; then
    SMOKE_COMPILE_STATUS=cached
    SMOKE_CARGO_STEP_MS=0
    return 0
  fi
  if ! grep -Eq "^#${SMOKE_COMPILE_VERTEX}[[:space:]]+DONE[[:space:]]+[0-9]+([.][0-9]+)?s([[:space:]]*)$" "$log"; then
    SMOKE_PARSE_ERROR='fixture compile RUN was neither DONE nor CACHED'
    return 1
  fi
  SMOKE_COMPILE_STATUS=done
  SMOKE_CARGO_FINISHED_COUNT=$(grep -Ec 'Finished.*release|Finished `release`' "$log" || true)
  SMOKE_BIN_A_PHASE_COUNT=$(grep -Ec "^#${SMOKE_COMPILE_VERTEX}[[:space:]].*SMOKE_CARGO_PHASE bin=cache-bin-a([[:space:]]*)$" "$log" || true)
  SMOKE_BIN_B_PHASE_COUNT=$(grep -Ec "^#${SMOKE_COMPILE_VERTEX}[[:space:]].*SMOKE_CARGO_PHASE bin=cache-bin-b([[:space:]]*)$" "$log" || true)
  SMOKE_COMPILED_PACKAGES=$(packages_for_phase "$log" all Compiling)
  SMOKE_FRESH_PACKAGES=$(packages_for_phase "$log" all Fresh)
  SMOKE_BIN_A_COMPILED_PACKAGES=$(packages_for_phase "$log" a Compiling)
  SMOKE_BIN_A_FRESH_PACKAGES=$(packages_for_phase "$log" a Fresh)
  SMOKE_BIN_B_COMPILED_PACKAGES=$(packages_for_phase "$log" b Compiling)
  SMOKE_BIN_B_FRESH_PACKAGES=$(packages_for_phase "$log" b Fresh)
  step_id=$SMOKE_COMPILE_VERTEX
  SMOKE_CARGO_STEP_MS=$(awk -v wanted="$step_id" '
    $0 ~ "^#" wanted " DONE [0-9]+([.][0-9]+)?s" {
      line = $0; sub("^#" wanted " DONE ", "", line); sub("s$", "", line)
      printf "%.0f\n", line * 1000; exit
    }
  ' "$log")
  if [[ ! "$SMOKE_CARGO_STEP_MS" =~ ^[0-9]+$ ]]; then
    SMOKE_PARSE_ERROR='fixture compile RUN is missing a bounded Cargo/BuildKit duration'
    return 1
  fi
}

assert_fail() { SMOKE_ASSERT_ERROR=$1; return 1; }

assert_source_at_commit() {
  local source=$1 expected_commit=$2 require_clean=$3 actual status
  actual=$(git -C "$source" rev-parse HEAD 2>/dev/null || true)
  [ "$actual" = "$expected_commit" ] || {
    assert_fail 'temporary fixture HEAD did not match the declared source identity'
    return 1
  }
  if [ "$require_clean" = 1 ]; then
    git -C "$source" diff --quiet || {
      assert_fail 'temporary fixture source must be clean before a success case'
      return 1
    }
    status=$(git -C "$source" status --porcelain --untracked-files=all)
    [ -z "$status" ] || {
      assert_fail 'temporary fixture source has untracked or modified files before a success case'
      return 1
    }
  fi
  return 0
}

source_identity() {
  local source=$1 head tree state
  head=$(git -C "$source" rev-parse HEAD)
  tree=$(git -C "$source" rev-parse 'HEAD^{tree}')
  state=clean
  git -C "$source" diff --quiet || state=dirty
  printf 'head=%s;tree=%s;worktree=%s' "$head" "$tree" "$state"
}

assert_forced_compile_evidence() {
  local name=$1 external_expectation=$2
  [ "$SMOKE_COMPILE_STATUS" = done ] || { assert_fail "forced compile RUN was not executed: $name"; return 1; }
  [ "$SMOKE_CARGO_FINISHED_COUNT" -ge 2 ] || { assert_fail "both Cargo binary phases did not report successful release finishes: $name"; return 1; }
  [[ "$SMOKE_CARGO_STEP_MS" =~ ^[0-9]+$ ]] || { assert_fail "forced compile RUN has no Cargo duration: $name"; return 1; }
  [ "$SMOKE_BIN_A_PHASE_COUNT" -eq 1 ] && [ "$SMOKE_BIN_B_PHASE_COUNT" -eq 1 ] || { assert_fail "explicit binary phase markers are incomplete or ambiguous: $name"; return 1; }
  csv_has "$SMOKE_BIN_A_COMPILED_PACKAGES" build-cache-fixture-lib || { assert_fail "workspace library was not recompiled after cargo clean in binary A phase: $name"; return 1; }
  csv_has "$SMOKE_BIN_A_COMPILED_PACKAGES" build-cache-fixture-app || { assert_fail "application binary A phase did not compile the application package: $name"; return 1; }
  csv_has "$SMOKE_BIN_B_FRESH_PACKAGES" build-cache-fixture-lib || { assert_fail "workspace library was not Fresh in binary B phase of the same compile RUN: $name"; return 1; }
  if csv_has "$SMOKE_BIN_B_COMPILED_PACKAGES" build-cache-fixture-lib; then
    assert_fail "workspace library recompiled in binary B phase instead of being reused within one RUN: $name"
    return 1
  fi
  csv_has "$SMOKE_BIN_B_COMPILED_PACKAGES" build-cache-fixture-app || { assert_fail "application binary B phase did not compile the application package: $name"; return 1; }
  case "$external_expectation" in
    cold)
      csv_has "$SMOKE_BIN_A_COMPILED_PACKAGES" itoa || { assert_fail "cold forced RUN did not compile itoa: $name"; return 1; }
      ;;
    warm)
      csv_has "$SMOKE_FRESH_PACKAGES" itoa || { assert_fail "warm forced RUN did not report Fresh itoa: $name"; return 1; }
      if csv_has "$SMOKE_COMPILED_PACKAGES" itoa; then
        assert_fail "warm forced RUN recompiled itoa instead of reusing the external crate: $name"
        return 1
      fi
      ;;
    new-cfg-if)
      csv_has "$SMOKE_BIN_A_COMPILED_PACKAGES" cfg-if || { assert_fail "external dependency/lockfile case did not compile cfg-if: $name"; return 1; }
      csv_has "$SMOKE_FRESH_PACKAGES" itoa || { assert_fail "external dependency/lockfile case did not retain Fresh itoa: $name"; return 1; }
      if csv_has "$SMOKE_COMPILED_PACKAGES" itoa; then
        assert_fail "external dependency/lockfile case recompiled itoa: $name"
        return 1
      fi
      ;;
    *) assert_fail "unsupported external package expectation: $external_expectation"; return 1 ;;
  esac
  return 0
}

assert_compile_layer_cached() {
  local name=$1
  [ "$SMOKE_COMPILE_STATUS" = cached ] || { assert_fail "same-input repeat did not cache the actual compile RUN vertex: $name"; return 1; }
  [ "$SMOKE_CARGO_FINISHED_COUNT" -eq 0 ] || { assert_fail "layer-cached compile RUN unexpectedly reported Cargo completion: $name"; return 1; }
  [ -z "$SMOKE_COMPILED_PACKAGES" ] && [ -z "$SMOKE_FRESH_PACKAGES" ] || { assert_fail "layer-cached compile RUN unexpectedly exposed Cargo package events: $name"; return 1; }
  [ "$SMOKE_CARGO_STEP_MS" = 0 ] || { assert_fail "layer-cached compile RUN must record zero Cargo execution time: $name"; return 1; }
  return 0
}

assert_missing_source_evidence() {
  local log=$1
  [ -s "$log" ] || { assert_fail 'missing-source fixture failure did not retain a build log'; return 1; }
  if ! grep -Eiq "(could not read|couldn't read|No such file or directory|failed to read).*(fixture-app/src/bin/cache-bin-a[.]rs|cache-bin-a[.]rs)|(fixture-app/src/bin/cache-bin-a[.]rs|cache-bin-a[.]rs).*(could not read|couldn't read|No such file or directory|failed to read)" "$log"; then
    assert_fail 'source-deletion case failed for an unexpected reason instead of the missing Rust source'
    return 1
  fi
  return 0
}

fixture_output() {
  local binary=$1 commit=$2 setting=$3 source_marker=$4 embedded=$5 generated_marker=$6 shared=$7
  printf '%s|commit=%s|setting=%s|source=%s|embedded=%s|%s|commit=%s|setting=%s|embedded=%s|shared=%s' \
    "$binary" "$commit" "$setting" "$source_marker" "$embedded" "$generated_marker" \
    "$commit" "$setting" "$embedded" "$shared"
}

SMOKE_CURRENT_CASE=
SMOKE_CURRENT_SOURCE_ID=
SMOKE_CURRENT_TAG=
SMOKE_CURRENT_NAMESPACE=
SMOKE_CURRENT_LOG=
SMOKE_CURRENT_LOG_REL=-
SMOKE_CURRENT_BIN_A_REL=-
SMOKE_CURRENT_BIN_B_REL=-
SMOKE_LAST_BUILD_COMMAND=-
SMOKE_LAST_TOTAL_MS=-

record_case() {
  local result=$1 cargo_cache=$2 detail=$3
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$SMOKE_CURRENT_CASE" "$result" "$SMOKE_CURRENT_SOURCE_ID" "$SMOKE_LAST_BUILD_COMMAND" \
    "$SMOKE_CURRENT_TAG" "$SMOKE_CURRENT_NAMESPACE" "$(csv_or_dash "$SMOKE_COMPILE_VERTEX")" \
    "$SMOKE_COMPILE_STATUS" "$cargo_cache" "$(csv_or_dash "$SMOKE_COMPILED_PACKAGES")" \
    "$(csv_or_dash "$SMOKE_FRESH_PACKAGES")" "$SMOKE_LAST_TOTAL_MS" \
    "${SMOKE_CARGO_STEP_MS:--}" "$SMOKE_CURRENT_LOG_REL" \
    "$SMOKE_CURRENT_BIN_A_REL" "$SMOKE_CURRENT_BIN_B_REL" "$tool_identity" "$detail" >>"$case_report"
}

begin_current_case() { record_case STARTED pending started; }
finish_current_case() { record_case "$1" "$2" "$3"; }

render_command() {
  local rendered
  printf -v rendered '%q ' "$@"
  printf '%s' "${rendered% }"
}

is_owned_fixture_tag() {
  case "${1:-}" in
    "lagrange-build-cache-smoke-${smoke_nonce}-"*) return 0 ;;
    *) return 1 ;;
  esac
}

register_owned_image_tag() {
  local tag=$1 known
  is_owned_fixture_tag "$tag" || die 'refusing to register a non-fixture image tag for cleanup'
  for known in "${owned_image_tags[@]}"; do
    [ "$known" != "$tag" ] || die 'fixture case attempted to reuse an image tag in one invocation'
  done
  owned_image_tags+=("$tag")
}

cleanup_test_images() {
  local tag
  [ "${#owned_image_tags[@]}" -gt 0 ] || return 0
  command -v docker >/dev/null 2>&1 || return 0
  for tag in "${owned_image_tags[@]}"; do
    is_owned_fixture_tag "$tag" || continue
    # Remove only tags minted by this invocation. Never prune image or BuildKit
    # cache state, and never address a production tag or cache namespace.
    docker image rm --force "$tag" >/dev/null 2>&1 || true
    [ -z "$cleanup_report" ] || printf 'image-rm\t%s\n' "$tag" >>"$cleanup_report"
  done
}

cleanup_apply() {
  local status=$?
  trap - EXIT
  set +e
  cleanup_test_images
  case "${tmp_dir:-}" in
    /tmp/lagrange-build-cache-smoke.apply.*) rm -rf -- "$tmp_dir" ;;
  esac
  exit "$status"
}

run_fixture_build() {
  local source=$1 tag=$2 commit=$3 setting=$4 namespace=$5 log=$6 build_mode=$7
  local args start_ms end_ms status run_token
  # Mode 0 repeats the first cold build's exact inputs. Mode 1 invalidates
  # only the compile RUN through an ARG, preserving Cargo cache mounts.
  # Mode 2 is genuinely cold: a new namespace plus disabled layer reuse.
  case "$build_mode" in
    0) run_token="lagrange-build-cache-smoke-${smoke_nonce}-cache-absent" ;;
    1|2) run_token=$tag ;;
    *) die 'invalid fixture build mode' ;;
  esac
  args=(
    build
    --pull=false
    --progress=plain
    --build-arg "CACHE_FIXTURE_COMMIT=$commit"
    --build-arg "CACHE_FIXTURE_BUILD_SETTING=$setting"
    --build-arg "CACHE_FIXTURE_RUN_TOKEN=$run_token"
    --build-arg "BUILDKIT_CACHE_MOUNT_NS=$namespace"
    -f "$source/Dockerfile"
    -t "$tag"
  )
  [ "$build_mode" -eq 2 ] && args+=(--no-cache)
  args+=("$source")
  SMOKE_LAST_BUILD_COMMAND=$(render_command docker "${args[@]}")
  register_owned_image_tag "$tag"
  reset_build_events
  begin_current_case
  start_ms=$(now_ms)
  # BUILDKIT_CACHE_MOUNT_NS is a built-in build ARG. The explicit array entry
  # above, not a host-environment assignment, forwards it to the frontend.
  if DOCKER_BUILDKIT=1 CARGO_BUILD_JOBS=2 docker "${args[@]}" >"$log" 2>&1; then
    status=0
  else
    status=$?
  fi
  end_ms=$(now_ms)
  SMOKE_LAST_TOTAL_MS=$((end_ms - start_ms))
  return "$status"
}

assert_fixture_binary() {
  local tag=$1 binary=$2 expected=$3 artifact=$4 actual
  if ! actual=$(docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges --entrypoint "/usr/local/bin/$binary" "$tag" 2>/dev/null); then
    assert_fail "fixture binary failed to run: $binary"
    return 1
  fi
  printf '%s\n' "$actual" >"$artifact"
  chmod 0600 -- "$artifact"
  [ "$actual" = "$expected" ] || assert_fail "fixture binary output mismatch: $binary"
}

assert_fixture_binaries() {
  local tag=$1 commit=$2 setting=$3 marker_a=$4 marker_b=$5 embedded=$6 generated=$7 shared=$8
  local output_a output_b
  output_a=$(fixture_output cache-bin-a "$commit" "$setting" "$marker_a" "$embedded" "$generated" "$shared")
  output_b=$(fixture_output cache-bin-b "$commit" "$setting" "$marker_b" "$embedded" "$generated" "$shared")
  assert_fixture_binary "$tag" cache-bin-a "$output_a" "$output_dir/$SMOKE_CURRENT_BIN_A_REL" || return 1
  assert_fixture_binary "$tag" cache-bin-b "$output_b" "$output_dir/$SMOKE_CURRENT_BIN_B_REL"
}

run_success_case() {
  local source=$1 name=$2 commit=$3 setting=$4 marker_a=$5 marker_b=$6 embedded=$7
  local generated=$8 shared=$9 namespace=${10} external_expectation=${11} build_mode=${12}
  local tag log cargo_cache
  SMOKE_ASSERT_ERROR=
  if ! assert_source_at_commit "$source" "$commit" 1; then
    die "source identity precondition failed for $name: $SMOKE_ASSERT_ERROR"
  fi
  tag="lagrange-build-cache-smoke-${smoke_nonce}-${name}"
  log=$log_dir/${name}.build.log
  SMOKE_CURRENT_CASE=$name
  SMOKE_CURRENT_SOURCE_ID=$(source_identity "$source")
  SMOKE_CURRENT_TAG=$tag
  SMOKE_CURRENT_NAMESPACE=$namespace
  SMOKE_CURRENT_LOG=$log
  SMOKE_CURRENT_LOG_REL=logs/${name}.build.log
  SMOKE_CURRENT_BIN_A_REL=binaries/${name}.cache-bin-a.out
  SMOKE_CURRENT_BIN_B_REL=binaries/${name}.cache-bin-b.out
  SMOKE_LAST_BUILD_COMMAND=-
  SMOKE_LAST_TOTAL_MS=-
  if ! run_fixture_build "$source" "$tag" "$commit" "$setting" "$namespace" "$log" "$build_mode"; then
    finish_current_case FAIL build-failed 'docker-build-failed;sanitized-log-retained'
    die "fixture Docker build failed: $name; sanitized evidence retained under $output_dir"
  fi
  if ! parse_build_events "$log"; then
    finish_current_case FAIL event-parse-failed "build-event-parse-failed;${SMOKE_PARSE_ERROR}"
    die "fixture build evidence was invalid: $name; sanitized evidence retained under $output_dir"
  fi
  if [ "$build_mode" -ne 0 ]; then
    if ! assert_forced_compile_evidence "$name" "$external_expectation"; then
      finish_current_case FAIL forced-evidence-failed "$SMOKE_ASSERT_ERROR"
      die "fixture forced-RUN evidence was invalid: $name; sanitized evidence retained under $output_dir"
    fi
    cargo_cache=forced-run-cargo-cache-${external_expectation}
  else
    if ! assert_compile_layer_cached "$name"; then
      finish_current_case FAIL compile-layer-evidence-failed "$SMOKE_ASSERT_ERROR"
      die "fixture layer-cache evidence was invalid: $name; sanitized evidence retained under $output_dir"
    fi
    cargo_cache=compile-layer-cached
  fi
  if ! assert_fixture_binaries "$tag" "$commit" "$setting" "$marker_a" "$marker_b" "$embedded" "$generated" "$shared"; then
    finish_current_case FAIL binary-output-failed "$SMOKE_ASSERT_ERROR"
    die "fixture binary assertion failed: $name; sanitized evidence retained under $output_dir"
  fi
  finish_current_case PASS "$cargo_cache" 'both-binaries-exact-output-asserted'
  printf 'SMOKE_CASE PASS name=%s compile_vertex=%s compile_status=%s total_ms=%s cargo_ms=%s compiled=%s fresh=%s cargo_cache=%s\n' \
    "$name" "$SMOKE_COMPILE_VERTEX" "$SMOKE_COMPILE_STATUS" "$SMOKE_LAST_TOTAL_MS" \
    "$SMOKE_CARGO_STEP_MS" "$(csv_or_dash "$SMOKE_COMPILED_PACKAGES")" \
    "$(csv_or_dash "$SMOKE_FRESH_PACKAGES")" "$cargo_cache"
}

run_missing_source_case() {
  local source=$1 name=$2 commit=$3 setting=$4 namespace=$5 tag log
  SMOKE_ASSERT_ERROR=
  if ! assert_source_at_commit "$source" "$commit" 0; then
    die "source identity precondition failed for $name: $SMOKE_ASSERT_ERROR"
  fi
  tag="lagrange-build-cache-smoke-${smoke_nonce}-${name}"
  log=$log_dir/${name}.build.log
  SMOKE_CURRENT_CASE=$name
  SMOKE_CURRENT_SOURCE_ID=$(source_identity "$source")
  SMOKE_CURRENT_TAG=$tag
  SMOKE_CURRENT_NAMESPACE=$namespace
  SMOKE_CURRENT_LOG=$log
  SMOKE_CURRENT_LOG_REL=logs/${name}.build.log
  SMOKE_CURRENT_BIN_A_REL=-
  SMOKE_CURRENT_BIN_B_REL=-
  SMOKE_LAST_BUILD_COMMAND=-
  SMOKE_LAST_TOTAL_MS=-
  if run_fixture_build "$source" "$tag" "$commit" "$setting" "$namespace" "$log" 1; then
    finish_current_case FAIL source-deletion-unexpected-success 'missing-source-build-succeeded'
    die 'source deletion unexpectedly produced a successful image'
  fi
  if ! assert_missing_source_evidence "$log"; then
    finish_current_case FAIL source-deletion-wrong-failure "$SMOKE_ASSERT_ERROR"
    die "source deletion did not fail for the missing Rust source; sanitized evidence retained under $output_dir"
  fi
  if docker image inspect "$tag" >/dev/null 2>&1; then
    finish_current_case FAIL source-deletion-stale-image 'unique-tag-resolved-to-an-image-after-failure'
    die 'source deletion left a stale executable/image under its unique test tag'
  fi
  finish_current_case EXPECTED_FAILURE expected-missing-source 'missing-rust-source-confirmed;unique-tag-absent'
  printf 'SMOKE_CASE PASS name=%s result=expected-missing-source-failure stale-image=absent total_ms=%s\n' "$name" "$SMOKE_LAST_TOTAL_MS"
}

restore_fixture_commit() {
  local source=$1 commit=$2 actual
  # This helper is used only inside the mktemp disposable fixture. --force is
  # essential after deletion: same-commit checkout without it leaves the file absent.
  git -C "$source" checkout --quiet --force --detach "$commit" || die 'could not force-restore a disposable fixture commit'
  git -C "$source" reset --hard --quiet "$commit" || die 'could not reset a disposable fixture commit'
  git -C "$source" clean -fdq || die 'could not clean a disposable fixture checkout'
  actual=$(git -C "$source" rev-parse HEAD)
  [ "$actual" = "$commit" ] || die 'disposable fixture restore did not reach the requested commit'
  git -C "$source" diff --quiet || die 'disposable fixture restore left tracked changes'
}

replace_fixture_text_once() {
  local path=$1 before=$2 after=$3 count
  count=$(grep -Foc -- "$before" "$path" || true)
  [ "$count" -eq 1 ] || die "fixture mutation expected exactly one source fragment: $before"
  sed -i "s|$before|$after|" "$path"
}

write_cfg_if_fixture_library() {
  local path=$1
  cat >"$path" <<'EOF'
cfg_if::cfg_if! {
    if #[cfg(any())] {
        const SHARED_NUMBER: i32 = 42;
    } else {
        const SHARED_NUMBER: i32 = 43;
    }
}

pub fn shared_number() -> String {
    let mut buffer = itoa::Buffer::new();
    buffer.format(SHARED_NUMBER).to_owned()
}
EOF
}

add_cfg_if_dependency_and_lock_record() {
  local source=$1 manifest=$source/fixture-lib/Cargo.toml lock=$source/Cargo.lock
  [ "$(grep -Fxc 'itoa = "=1.0.18"' "$manifest" || true)" -eq 1 ] ||
    die 'external dependency mutation could not find the exact itoa dependency pin'
  if grep -Fq 'cfg-if = "=1.0.4"' "$manifest" || grep -Fq 'name = "cfg-if"' "$lock"; then
    die 'external dependency mutation found cfg-if before its controlled introduction'
  fi
  sed -i '/^itoa = "=1.0.18"$/a cfg-if = "=1.0.4"' "$manifest"
  [ "$(grep -Fxc ' "itoa",' "$lock" || true)" -eq 1 ] ||
    die 'external dependency mutation could not find the fixture lockfile itoa dependency entry'
  sed -i '/^ "itoa",$/i\ "cfg-if",' "$lock"
  printf '\n[[package]]\nname = "cfg-if"\nversion = "1.0.4"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801"\n' >>"$lock"
  grep -Fq 'cfg-if = "=1.0.4"' "$manifest" || die 'external dependency mutation did not add cfg-if to fixture-lib'
  grep -Fq 'name = "cfg-if"' "$lock" || die 'external dependency mutation did not add cfg-if to Cargo.lock'
  grep -Fq ' "cfg-if",' "$lock" || die 'external dependency mutation did not add cfg-if to the fixture library lock dependencies'
  grep -Fq 'version = "1.0.4"' "$lock" || die 'external dependency mutation changed the approved cfg-if version'
  grep -Fq 'registry+https://github.com/rust-lang/crates.io-index' "$lock" || die 'external dependency mutation changed the approved cfg-if source'
  grep -Fq '9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801' "$lock" || die 'external dependency mutation changed the approved cfg-if checksum'
}

make_fixture_history() {
  local source=$1
  git -C "$source" init -q
  git -C "$source" config user.email build-cache-fixture@example.invalid
  git -C "$source" config user.name build-cache-fixture
  git -C "$source" add --all
  git -C "$source" commit -qm fixture-base
  base_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  replace_fixture_text_once "$source/fixture-app/src/bin/cache-bin-a.rs" source-v1 source-v2
  git -C "$source" add fixture-app/src/bin/cache-bin-a.rs
  git -C "$source" commit -qm fixture-app-rust-source-change
  app_source_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  replace_fixture_text_once "$source/fixture-lib/src/lib.rs" 'buffer.format(42)' 'buffer.format(43)'
  git -C "$source" add fixture-lib/src/lib.rs
  git -C "$source" commit -qm fixture-workspace-library-change
  workspace_library_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  replace_fixture_text_once "$source/fixture-app/build.rs" generated-v1 generated-v2
  git -C "$source" add fixture-app/build.rs
  git -C "$source" commit -qm fixture-build-script-source-change
  build_script_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  add_cfg_if_dependency_and_lock_record "$source"
  write_cfg_if_fixture_library "$source/fixture-lib/src/lib.rs"
  grep -Fq 'cfg_if::cfg_if!' "$source/fixture-lib/src/lib.rs" || die 'external dependency mutation did not add the cfg_if macro'
  git -C "$source" add fixture-lib/Cargo.toml fixture-lib/src/lib.rs Cargo.lock
  git -C "$source" commit -qm fixture-external-dependency-lockfile-change
  external_dependency_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  printf 'embedded-v2\n' >"$source/fixture-app/data/embedded.txt"
  git -C "$source" add fixture-app/data/embedded.txt
  git -C "$source" commit -qm fixture-embedded-data-change
  data_commit=$(git -C "$source" rev-parse HEAD)

  restore_fixture_commit "$source" "$base_commit"
  printf 'commit-only fixture note\n' >"$source/commit-only.txt"
  git -C "$source" add commit-only.txt
  git -C "$source" commit -qm fixture-commit-only-change
  commit_only_commit=$(git -C "$source" rev-parse HEAD)
  restore_fixture_commit "$source" "$base_commit"
}

new_smoke_nonce() {
  local suffix=${tmp_dir##*.}
  [[ "$suffix" =~ ^[A-Za-z0-9]{6,}$ ]] || die 'mktemp did not produce a usable random fixture suffix'
  # Encode every byte injectively: Docker repository names reject uppercase,
  # while simply lowercasing the suffix would collapse distinct invocations.
  smoke_nonce="r$(printf '%s' "$suffix" | od -An -v -tx1 | tr -d ' \n')"
}

safe_scalar() { printf '%s' "$1" | tr -cd '[:alnum:]._-'; }

now_ms() {
  local seconds nanos millis
  seconds=$(date +%s)
  nanos=$(date +%N)
  millis=${nanos:0:3}
  [[ "$seconds" =~ ^[0-9]+$ ]] && [[ "$millis" =~ ^[0-9]{3}$ ]] ||
    die 'clock did not produce a bounded millisecond timestamp'
  printf '%s%03d\n' "$seconds" "$((10#$millis))"
}

write_evidence_headers() {
  local script_hash base_tree
  metadata_report=$output_dir/metadata.tsv
  case_report=$output_dir/cases.tsv
  cleanup_report=$output_dir/cleanup.tsv
  log_dir=$output_dir/logs
  binary_dir=$output_dir/binaries
  mkdir -m 0700 -- "$log_dir" "$binary_dir"
  git_version=$(safe_scalar "$(git --version | awk '{ print $3 }')")
  fixture_toolchain=$(sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$fixture_dir/rust-toolchain.toml" | head -n 1)
  fixture_toolchain=$(safe_scalar "$fixture_toolchain")
  script_hash=$(sha256sum "$script_dir/build-cache-smoke.sh" | awk '{ print $1 }')
  base_tree=$(git -C "$source_dir" rev-parse "${base_commit}^{tree}")
  [ -n "$git_version" ] && [ -n "$fixture_toolchain" ] && [[ "$script_hash" =~ ^[0-9a-f]{64}$ ]] || die 'could not construct safe smoke tool identity evidence'
  tool_identity="docker=${docker_client_version};git=${git_version};rust=${fixture_toolchain};script-sha256=${script_hash}"
  {
    printf '%s\n' BUILD_CACHE_SMOKE_V2
    printf 'mode\tapply\n'
    printf 'fixture_dir\t%s\n' "$fixture_dir"
    printf 'fixture_base_commit\t%s\n' "$base_commit"
    printf 'fixture_base_tree\t%s\n' "$base_tree"
    printf 'run_nonce\t%s\n' "$smoke_nonce"
    printf 'cache_mount_namespace\tbuild-cache-fixture-smoke-%s\n' "$smoke_nonce"
    printf 'repopulate_cache_mount_namespace\tbuild-cache-fixture-repopulate-%s\n' "$smoke_nonce"
    printf 'docker_client_version\t%s\n' "$docker_client_version"
    printf 'git_version\t%s\n' "$git_version"
    printf 'fixture_rust_toolchain\t%s\n' "$fixture_toolchain"
    printf 'script_sha256\t%s\n' "$script_hash"
    printf 'tool_identity\t%s\n' "$tool_identity"
    printf 'cargo_build_jobs\t2\n'
    printf 'cache_mount_policy\tthree-fixture-only-locked-mounts;explicit-BUILDKIT_CACHE_MOUNT_NS-build-arg\n'
    printf 'layer_reuse_policy\tcold-and-semantic-cases-no-cache;repeat-same-source-normal-layer-cache\n'
    printf 'cleanup_policy\texplicit-current-run-fixture-tags-only;no-prune\n'
  } >"$metadata_report"
  printf 'scenario\tresult\tsource_identity\tcommand\timage_tag\tcache_namespace\tcompile_vertex\tcompile_status\tcargo_cache_status\tcompiled_packages\tfresh_packages\ttotal_ms\tcargo_ms\tbuild_log\tbinary_a_output\tbinary_b_output\ttool_identity\tdetail\n' >"$case_report"
  printf 'action\timage_tag\n' >"$cleanup_report"
  chmod 0600 -- "$metadata_report" "$case_report" "$cleanup_report"
}

record_restore_after_deletion() {
  SMOKE_CURRENT_CASE=restore-after-source-deletion
  SMOKE_CURRENT_SOURCE_ID=$(source_identity "$source_dir")
  SMOKE_CURRENT_TAG=-
  SMOKE_CURRENT_NAMESPACE=$1
  SMOKE_CURRENT_LOG=-
  SMOKE_CURRENT_LOG_REL=-
  SMOKE_CURRENT_BIN_A_REL=-
  SMOKE_CURRENT_BIN_B_REL=-
  SMOKE_LAST_BUILD_COMMAND="git checkout --force --detach ${base_commit}; git reset --hard ${base_commit}"
  SMOKE_LAST_TOTAL_MS=0
  reset_build_events
  SMOKE_CARGO_STEP_MS=-
  finish_current_case PASS not-applicable 'tracked-source-restored-in-disposable-fixture-before-repopulation'
}

run_apply() {
  local namespace repopulate_namespace
  [ -n "$output_dir" ] || die '--apply requires --output-dir'
  prepare_output_dir
  command -v git >/dev/null 2>&1 || die 'git is required for fixture history'
  command -v docker >/dev/null 2>&1 || die 'Docker is required for --apply; use --plan or --self-test without a daemon'
  docker_client_version=$(docker version --format '{{.Client.Version}}' 2>/dev/null | head -n 1 || true)
  docker_client_version=$(safe_scalar "$docker_client_version")
  [ -n "$docker_client_version" ] || die 'Docker daemon is unavailable for --apply'
  tmp_dir=$(mktemp -d /tmp/lagrange-build-cache-smoke.apply.XXXXXX)
  trap cleanup_apply EXIT
  new_smoke_nonce
  source_dir=$tmp_dir/source
  mkdir -p "$source_dir"
  cp -a "$fixture_dir/." "$source_dir/"
  make_fixture_history "$source_dir"
  namespace="build-cache-fixture-smoke-${smoke_nonce}"
  repopulate_namespace="build-cache-fixture-repopulate-${smoke_nonce}"
  write_evidence_headers

  echo "SMOKE_RUN fixture=$fixture_dir cache_namespace=$namespace jobs=2 output_dir=$output_dir"
  echo 'SMOKE_RUN cold-no-cache, normal-layer-cached-repeat, forced-RUN Cargo reuse, semantic source changes, missing-source recovery'
  echo 'SMOKE_RUN cases=cache-absent,repeat-same-source,different-bin,changed-local-source,changed-workspace-library,changed-build-script-source,changed-external-dependency-lockfile,changed-embedded-data,changed-build-setting,commit-only-change,older-mtimes-branch-reversion,source-deletion-failure,restore-after-source-deletion,cache-repopulate-absent,cache-repopulate'

  run_success_case "$source_dir" cache-absent "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" cold 2
  run_success_case "$source_dir" repeat-same-source "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 0
  # A new tag alone does not prove within-RUN reuse. This forced RUN proves it
  # from the two phase markers: local lib compiles for A then is Fresh for B.
  run_success_case "$source_dir" different-bin "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 1

  restore_fixture_commit "$source_dir" "$app_source_commit"
  run_success_case "$source_dir" changed-local-source "$app_source_commit" default source-v2 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$workspace_library_commit"
  run_success_case "$source_dir" changed-workspace-library "$workspace_library_commit" default source-v1 source-v1 embedded-v1 generated-v1 43 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$build_script_commit"
  run_success_case "$source_dir" changed-build-script-source "$build_script_commit" default source-v1 source-v1 embedded-v1 generated-v2 42 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$external_dependency_commit"
  run_success_case "$source_dir" changed-external-dependency-lockfile "$external_dependency_commit" default source-v1 source-v1 embedded-v1 generated-v1 43 "$namespace" new-cfg-if 1
  restore_fixture_commit "$source_dir" "$data_commit"
  run_success_case "$source_dir" changed-embedded-data "$data_commit" default source-v1 source-v1 embedded-v2 generated-v1 42 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$base_commit"
  run_success_case "$source_dir" changed-build-setting "$base_commit" changed-setting source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$commit_only_commit"
  run_success_case "$source_dir" commit-only-change "$commit_only_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 1
  restore_fixture_commit "$source_dir" "$base_commit"
  find "$source_dir" -path "$source_dir/.git" -prune -o -type f -exec touch -d @1 {} +
  run_success_case "$source_dir" older-mtimes-branch-reversion "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$namespace" warm 1

  restore_fixture_commit "$source_dir" "$base_commit"
  rm -f -- "$source_dir/fixture-app/src/bin/cache-bin-a.rs"
  run_missing_source_case "$source_dir" source-deletion-failure "$base_commit" default "$namespace"
  restore_fixture_commit "$source_dir" "$base_commit"
  [ -f "$source_dir/fixture-app/src/bin/cache-bin-a.rs" ] || die 'disposable fixture source was not restored after the deletion case'
  record_restore_after_deletion "$namespace"
  run_success_case "$source_dir" cache-repopulate-absent "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$repopulate_namespace" cold 2
  run_success_case "$source_dir" cache-repopulate "$base_commit" default source-v1 source-v1 embedded-v1 generated-v1 42 "$repopulate_namespace" warm 1

  echo "SMOKE_RESULT PASS output_dir=$output_dir case_records=$(($(wc -l <"$case_report") - 1))"
  echo 'SMOKE_APPLY_STATUS live-build=verified-by-this-apply-run'
}

run_self_test() {
  local test_dir fake_bin audit state plan_output default_output
  local cold_log cached_log unrelated_cached_log apply_one apply_two forced_failure
  local namespace_one namespace_two nonce_one nonce_two source_one source_changed repeat_line cold_line
  local audit_before audit_after cleanup_command
  (
    tmp_dir=/tmp/lagrange-build-cache-smoke.apply.AbCd12
    new_smoke_nonce
    first_nonce=$smoke_nonce
    [[ "$first_nonce" =~ ^r[0-9a-f]+$ ]] || die 'nonce violates Docker repository grammar'
    tmp_dir=/tmp/lagrange-build-cache-smoke.apply.abcd12
    new_smoke_nonce
    [ "$first_nonce" != "$smoke_nonce" ] || die 'case-distinct suffixes collide'
  )
  test_dir=$(mktemp -d /tmp/lagrange-build-cache-smoke-self-test.XXXXXX)
  # Self-test owns this exact mktemp directory. Capture the fully quoted path
  # while the local is in scope, so EXIT also cleans up assertion failures.
  printf -v cleanup_command 'rm -rf -- %q' "$test_dir"
  trap "$cleanup_command" EXIT
  fake_bin=$test_dir/fake-bin
  audit=$test_dir/fake-docker.tsv
  state=$test_dir/fake-docker-state
  mkdir -p "$fake_bin" "$state"
  cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

audit=${BUILD_CACHE_SMOKE_FAKE_DOCKER_AUDIT:?}
state=${BUILD_CACHE_SMOKE_FAKE_DOCKER_STATE:?}
mkdir -p "$state"

record() {
  local kind=$1 value
  shift
  printf '%s' "$kind" >>"$audit"
  for value in "$@"; do printf '\t%s' "$value" >>"$audit"; done
  printf '\n' >>"$audit"
}

source_marker() {
  sed -nE 's/.*SOURCE_MARKER: &str = "([^"]+)";.*/\1/p' "$1" | head -n 1
}

case "${1:-}" in
  version)
    shift
    record version "$@"
    printf '%s\n' 99.0.0-fake
    ;;
  build)
    shift
    args=("$@")
    tag= namespace= commit= setting= run_token= no_cache=0
    for ((index = 0; index < ${#args[@]}; index += 1)); do
      argument=${args[$index]}
      case "$argument" in
        --no-cache) no_cache=1 ;;
        -t)
          index=$((index + 1)); tag=${args[$index]}
          ;;
        --build-arg)
          index=$((index + 1)); build_arg=${args[$index]}
          case "$build_arg" in
            CACHE_FIXTURE_COMMIT=*) commit=${build_arg#CACHE_FIXTURE_COMMIT=} ;;
            CACHE_FIXTURE_BUILD_SETTING=*) setting=${build_arg#CACHE_FIXTURE_BUILD_SETTING=} ;;
            CACHE_FIXTURE_RUN_TOKEN=*) run_token=${build_arg#CACHE_FIXTURE_RUN_TOKEN=} ;;
            BUILDKIT_CACHE_MOUNT_NS=*) namespace=${build_arg#BUILDKIT_CACHE_MOUNT_NS=} ;;
          esac
          ;;
      esac
    done
    record build "${args[@]}"
    case "$tag" in
      *-cache-absent|*-cache-repopulate-absent)
        [ "$no_cache" -eq 1 ] && [ "$run_token" = "$tag" ] || exit 64 ;;
      *-repeat-same-source)
        [ "$no_cache" -eq 0 ] && [ "$run_token" = "${tag%-repeat-same-source}-cache-absent" ] || exit 64 ;;
      *)
        [ "$no_cache" -eq 0 ] && [ "$run_token" = "$tag" ] || {
          echo 'fake Docker: warm RUN must preserve mounts and invalidate only the compile step' >&2
          exit 64
        } ;;
    esac
    [[ "$tag" =~ ^lagrange-build-cache-smoke-r[0-9a-f]+-[a-z0-9-]+$ ]] || {
      printf '%s\n' 'fake Docker: invalid fixture repository name' >&2
      exit 64
    }
    case "$namespace" in
      build-cache-fixture-smoke-r*|build-cache-fixture-repopulate-r*) ;;
      *) printf '%s\n' 'fake Docker: missing explicit cache namespace build arg' >&2; exit 64 ;;
    esac
    [ -n "$tag" ] && [ -n "$commit" ] && [ -n "$setting" ] || {
      printf '%s\n' 'fake Docker: required fixture build argument is missing' >&2
      exit 64
    }
    last_index=$((${#args[@]} - 1))
    source=${args[$last_index]}
    case "${BUILD_CACHE_SMOKE_FAKE_FAIL_CASE:-}" in
      '') ;;
      *) case "$tag" in
           *-"${BUILD_CACHE_SMOKE_FAKE_FAIL_CASE}")
             printf '%s\n' '#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build'
             printf '%s\n' '#9 ERROR: controlled fake fixture build failure'
             exit 86
             ;;
         esac ;;
    esac
    if [ ! -f "$source/fixture-app/src/bin/cache-bin-a.rs" ]; then
      printf '%s\n' '#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build'
      printf '%s\n' "#9 0.01 error: couldn't read fixture-app/src/bin/cache-bin-a.rs: No such file or directory (os error 2)"
      printf '%s\n' '#9 ERROR: process failed because the Rust source target is absent'
      exit 73
    fi
    # A layer-cached tag is still a usable image. Record its deterministic
    # fixture outputs before choosing whether this fake build emits Cargo.
    marker_a=$(source_marker "$source/fixture-app/src/bin/cache-bin-a.rs")
    marker_b=$(source_marker "$source/fixture-app/src/bin/cache-bin-b.rs")
    embedded=$(tr -d '\r\n' <"$source/fixture-app/data/embedded.txt")
    generated=generated-v1
    grep -Fq generated-v2 "$source/fixture-app/build.rs" && generated=generated-v2
    shared=42
    if grep -Fq 'cfg_if::cfg_if!' "$source/fixture-lib/src/lib.rs" || grep -Fq 'buffer.format(43)' "$source/fixture-lib/src/lib.rs"; then shared=43; fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$commit" "$setting" "$marker_a" "$marker_b" "$embedded" "$generated" "$shared" >"$state/$tag"
    case "$tag" in
      *-repeat-same-source)
        # Include unrelated cached work to prove the harness keys off #9 only.
        printf '%s\n' '#1 [internal] load build definition from Dockerfile'
        printf '%s\n' '#1 CACHED'
        printf '%s\n' '#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build'
        printf '%s\n' '#9 CACHED'
        ;;
      *)
        printf '%s\n' '#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build'
        printf '%s\n' '#9 0.01 SMOKE_CARGO_PHASE bin=cache-bin-a'
        case "$tag" in
          *-cache-absent|*-cache-repopulate-absent) printf '%s\n' '#9 0.02 Compiling itoa v1.0.18' ;;
          *) printf '%s\n' '#9 0.02 Fresh itoa v1.0.18' ;;
        esac
        case "$tag" in *-changed-external-dependency-lockfile) printf '%s\n' '#9 0.03 Compiling cfg-if v1.0.4' ;; esac
        printf '%s\n' '#9 0.04 Compiling build-cache-fixture-lib v0.1.0 (/build/fixture-lib)'
        printf '%s\n' '#9 0.05 Compiling build-cache-fixture-app v0.1.0 (/build/fixture-app)'
        printf '%s\n' '#9 0.06 Finished `release` profile [optimized] target(s) in 0.60s'
        printf '%s\n' '#9 0.07 SMOKE_CARGO_PHASE bin=cache-bin-b'
        printf '%s\n' '#9 0.08 Fresh build-cache-fixture-lib v0.1.0 (/build/fixture-lib)'
        printf '%s\n' '#9 0.09 Compiling build-cache-fixture-app v0.1.0 (/build/fixture-app)'
        printf '%s\n' '#9 0.10 Finished `release` profile [optimized] target(s) in 0.40s'
        printf '%s\n' '#9 DONE 1.2s'
        ;;
    esac
    ;;
  run)
    shift
    args=("$@")
    entrypoint=
    for ((index = 0; index < ${#args[@]}; index += 1)); do
      if [ "${args[$index]}" = --entrypoint ]; then index=$((index + 1)); entrypoint=${args[$index]}; fi
    done
    last_index=$((${#args[@]} - 1)); tag=${args[$last_index]}
    record run "${args[@]}"
    [ -f "$state/$tag" ] || exit 74
    IFS=$'\t' read -r commit setting marker_a marker_b embedded generated shared <"$state/$tag"
    case "$entrypoint" in
      /usr/local/bin/cache-bin-a) binary=cache-bin-a; marker=$marker_a ;;
      /usr/local/bin/cache-bin-b) binary=cache-bin-b; marker=$marker_b ;;
      *) exit 75 ;;
    esac
    printf '%s|commit=%s|setting=%s|source=%s|embedded=%s|%s|commit=%s|setting=%s|embedded=%s|shared=%s\n' \
      "$binary" "$commit" "$setting" "$marker" "$embedded" "$generated" "$commit" "$setting" "$embedded" "$shared"
    ;;
  image)
    shift
    subcommand=${1:-}
    shift || true
    args=("$@")
    last_index=$((${#args[@]} - 1)); tag=${args[$last_index]}
    case "$subcommand" in
      inspect) record image-inspect "$@"; [ -f "$state/$tag" ] ;;
      rm) record image-rm "$@"; rm -f -- "$state/$tag" ;;
      *) exit 64 ;;
    esac
    ;;
  *) printf '%s\n' "fake Docker: unsupported command ${1:-}" >&2; exit 64 ;;
esac
EOF
  chmod 0755 "$fake_bin/docker"

  plan_output=$(PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --plan --fixture-dir "$fixture_dir" --output-dir "$test_dir/plan-output")
  grep -Fq 'BUILD_CACHE_SMOKE_PLAN mode=plan' <<<"$plan_output" || die 'plan mode did not report its default mode'
  [ ! -e "$test_dir/plan-output" ] || die 'plan mode created an optional output directory'
  [ ! -e "$audit" ] || die 'plan mode invoked fake Docker'
  default_output=$(PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --fixture-dir "$fixture_dir")
  grep -Fq 'mode=plan' <<<"$default_output" || die 'implicit plan mode failed'

  cold_log=$test_dir/cold.log
  cached_log=$test_dir/cached.log
  unrelated_cached_log=$test_dir/unrelated-cached.log
  cat >"$cold_log" <<'EOF'
#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build
#9 0.01 SMOKE_CARGO_PHASE bin=cache-bin-a
#9 0.02 Compiling itoa v1.0.18
#9 0.03 Compiling build-cache-fixture-lib v0.1.0 (/build/fixture-lib)
#9 0.04 Compiling build-cache-fixture-app v0.1.0 (/build/fixture-app)
#9 0.05 Finished `release` profile [optimized] target(s) in 0.50s
#9 0.06 SMOKE_CARGO_PHASE bin=cache-bin-b
#9 0.07 Fresh build-cache-fixture-lib v0.1.0 (/build/fixture-lib)
#9 0.08 Compiling build-cache-fixture-app v0.1.0 (/build/fixture-app)
#9 0.09 Finished `release` profile [optimized] target(s) in 0.40s
#9 DONE 1.4s
EOF
  cat >"$cached_log" <<'EOF'
#1 [internal] load build definition from Dockerfile
#1 CACHED
#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build
#9 CACHED
EOF
  cat >"$unrelated_cached_log" <<'EOF'
#1 [internal] load build definition from Dockerfile
#1 CACHED
#9 [builder 7/7] RUN --mount=type=cache cargo clean --workspace --release --locked && cargo build
#9 DONE 0.2s
#9 Finished `release` profile [optimized] target(s) in 0.20s
EOF
  parse_build_events "$cold_log" || die "self-test cold parser failed: $SMOKE_PARSE_ERROR"
  assert_forced_compile_evidence parser-cold cold || die "self-test cold parser evidence failed: $SMOKE_ASSERT_ERROR"
  [ "$SMOKE_CARGO_STEP_MS" = 1400 ] || die 'self-test cold parser missed Cargo duration'
  parse_build_events "$cached_log" || die "self-test cached parser failed: $SMOKE_PARSE_ERROR"
  assert_compile_layer_cached parser-cached || die "self-test cached parser evidence failed: $SMOKE_ASSERT_ERROR"
  parse_build_events "$unrelated_cached_log" || die "self-test unrelated-cache parser failed: $SMOKE_PARSE_ERROR"
  if assert_compile_layer_cached parser-unrelated-cache; then die 'unrelated FROM/COPY CACHED evidence incorrectly passed as a cached compile RUN'; fi

  if bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" >"$test_dir/missing-output.out" 2>&1; then die 'apply accepted a missing output directory'; fi
  grep -Fq -- '--apply requires --output-dir' "$test_dir/missing-output.out" || die 'missing output-dir failure was not reported'
  if bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$test_dir/missing" --output-dir "$test_dir/missing-fixture-output" >"$test_dir/missing-fixture.out" 2>&1; then die 'apply accepted a missing fixture directory'; fi
  grep -Fq 'fixture-dir must be a regular directory' "$test_dir/missing-fixture.out" || die 'missing fixture failure was not reported'
  if bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" --output-dir "$repo_root/unsafe-smoke-output" >"$test_dir/unsafe-output.out" 2>&1; then die 'apply accepted an output directory inside the original checkout'; fi
  grep -Fq 'outside the original repository checkout' "$test_dir/unsafe-output.out" || die 'unsafe output-dir failure was not reported'
  mkdir -p "$test_dir/nonempty-output"
  : >"$test_dir/nonempty-output/existing-evidence"
  audit_before=0; [ -e "$audit" ] && audit_before=$(wc -l <"$audit")
  if BUILD_CACHE_SMOKE_FAKE_DOCKER_AUDIT="$audit" BUILD_CACHE_SMOKE_FAKE_DOCKER_STATE="$state" PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" --output-dir "$test_dir/nonempty-output" >"$test_dir/nonempty-output.out" 2>&1; then die 'apply accepted a nonempty output directory'; fi
  grep -Fq 'output-dir must be empty' "$test_dir/nonempty-output.out" || die 'nonempty output-dir failure was not reported'
  audit_after=0; [ -e "$audit" ] && audit_after=$(wc -l <"$audit")
  [ "$audit_before" = "$audit_after" ] || die 'nonempty output-dir failure invoked fake Docker'
  if bash "$script_dir/build-cache-smoke.sh" --unknown-option >"$test_dir/unknown.out" 2>&1; then die 'unknown smoke option unexpectedly passed'; fi
  grep -Fq 'unknown option' "$test_dir/unknown.out" || die 'unknown option failure was not reported'
  if bash "$script_dir/build-cache-smoke.sh" --plan --apply >"$test_dir/modes.out" 2>&1; then die 'multiple smoke modes unexpectedly passed'; fi
  grep -Fq 'choose exactly one mode' "$test_dir/modes.out" || die 'multiple-mode failure was not reported'

  apply_one=$test_dir/apply-one
  if ! BUILD_CACHE_SMOKE_FAKE_DOCKER_AUDIT="$audit" BUILD_CACHE_SMOKE_FAKE_DOCKER_STATE="$state" PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" --output-dir "$apply_one" >"$test_dir/apply-one.out" 2>&1; then die 'fake-Docker apply control flow unexpectedly failed'; fi
  grep -Fq 'SMOKE_RESULT PASS' "$test_dir/apply-one.out" || die 'fake-Docker apply did not report success'
  [ -s "$apply_one/metadata.tsv" ] && [ -s "$apply_one/cases.tsv" ] && [ -s "$apply_one/cleanup.tsv" ] || die 'apply did not retain metadata, case, and cleanup evidence'
  grep -Fq 'docker_client_version' "$apply_one/metadata.tsv" || die 'tool version evidence was not retained'
  grep -Fq 'tool_identity' "$apply_one/metadata.tsv" || die 'tool identity evidence was not retained'
  grep -Fq 'source-deletion-failure'$'\t''EXPECTED_FAILURE' "$apply_one/cases.tsv" || die 'expected missing-source failure was not reported in retained case evidence'
  [ -s "$apply_one/logs/source-deletion-failure.build.log" ] || die 'missing-source failure log was removed with the disposable source'
  grep -Fq 'source=source-v2' "$apply_one/binaries/changed-local-source.cache-bin-a.out" || die 'app Rust source mutation did not affect binary A output'
  grep -Fq 'source=source-v1' "$apply_one/binaries/changed-local-source.cache-bin-b.out" || die 'app Rust source mutation incorrectly changed binary B output'
  grep -Fq 'shared=43' "$apply_one/binaries/changed-workspace-library.cache-bin-a.out" || die 'workspace library mutation did not affect binary A output'
  grep -Fq 'shared=43' "$apply_one/binaries/changed-workspace-library.cache-bin-b.out" || die 'workspace library mutation did not affect binary B output'
  grep -Fq 'generated-v2' "$apply_one/binaries/changed-build-script-source.cache-bin-a.out" || die 'build-script source mutation did not affect binary A output'
  grep -Fq 'generated-v2' "$apply_one/binaries/changed-build-script-source.cache-bin-b.out" || die 'build-script source mutation did not affect binary B output'
  grep -Fq 'shared=43' "$apply_one/binaries/changed-external-dependency-lockfile.cache-bin-a.out" || die 'external dependency/lockfile mutation did not affect binary A output'
  grep -Fq 'shared=43' "$apply_one/binaries/changed-external-dependency-lockfile.cache-bin-b.out" || die 'external dependency/lockfile mutation did not affect binary B output'
  if ! awk -F '\t' '$1 == "changed-external-dependency-lockfile" && $2 == "PASS" && $10 ~ /(^|,)cfg-if(,|$)/ && $11 ~ /(^|,)itoa(,|$)/ { found = 1 } END { exit !found }' "$apply_one/cases.tsv"; then die 'external dependency/lockfile case did not retain cfg-if compile and itoa Fresh evidence'; fi
  if ! awk -F '\t' '$1 == "different-bin" && $2 == "PASS" && $10 !~ /(^|,)itoa(,|$)/ && $11 ~ /(^|,)itoa(,|$)/ { found = 1 } END { exit !found }' "$apply_one/cases.tsv"; then die 'forced warm external no-recompile evidence was not retained'; fi
  if ! awk -F '\t' '$1 == "repeat-same-source" && $2 == "PASS" && $8 == "cached" && $9 == "compile-layer-cached" && $13 == "0" { found = 1 } END { exit !found }' "$apply_one/cases.tsv"; then die 'normal same-input cached compile vertex evidence was not retained'; fi
  if ! awk -F '\t' '$1 == "cache-absent" && $2 == "PASS" && $12 ~ /^[0-9]+$/ && $12 < 60000 && $13 == "1200" { found = 1 } END { exit !found }' "$apply_one/cases.tsv"; then die 'total/Cargo timing evidence was not a bounded millisecond record'; fi
  grep -Fq 'restore-after-source-deletion'$'\t''PASS' "$apply_one/cases.tsv" || die 'explicit restore after deletion was not reported'
  [ -s "$apply_one/binaries/cache-repopulate-absent.cache-bin-a.out" ] || die 'cache-repopulation build did not succeed after explicit restore'
  source_one=$(awk -F '\t' '$1 == "cache-absent" && $2 == "PASS" { print $3; exit }' "$apply_one/cases.tsv")
  source_changed=$(awk -F '\t' '$1 == "changed-local-source" && $2 == "PASS" { print $3; exit }' "$apply_one/cases.tsv")
  [ -n "$source_one" ] && [ -n "$source_changed" ] && [ "$source_one" != "$source_changed" ] || die 'source identity evidence did not distinguish the app source mutation'

  repeat_line=$(awk -F '\t' '$1 == "build" { for (field_number = 2; field_number <= NF; field_number++) if ($field_number == "-t" && $(field_number + 1) ~ /-repeat-same-source$/) { print; exit } }' "$audit")
  cold_line=$(awk -F '\t' '$1 == "build" { for (field_number = 2; field_number <= NF; field_number++) if ($field_number == "-t" && $(field_number + 1) ~ /-cache-absent$/) { print; exit } }' "$audit")
  [ -n "$repeat_line" ] && [ -n "$cold_line" ] || die 'fake Docker audit lacks repeat/cold build commands'
  case "$repeat_line" in *$'\t--no-cache\t'*) die 'normal same-input repeat incorrectly passed --no-cache' ;; esac
  case "$cold_line" in *$'\t--no-cache\t'*) ;; *) die 'cold fixture build did not pass --no-cache' ;; esac
  if ! awk -F '\t' '
    $1 == "build" {
      build_count += 1; found = 0
      for (field_number = 2; field_number < NF; field_number++) if ($field_number == "--build-arg" && $(field_number + 1) ~ /^BUILDKIT_CACHE_MOUNT_NS=build-cache-fixture-(smoke|repopulate)-r[A-Za-z0-9]+$/) found = 1
      if (!found) bad = 1
    }
    END { exit (build_count > 0 && !bad) ? 0 : 1 }
  ' "$audit"; then die 'fake-Docker apply audit did not show an explicit cache namespace build argument on every build'; fi

  apply_two=$test_dir/apply-two
  if ! BUILD_CACHE_SMOKE_FAKE_DOCKER_AUDIT="$audit" BUILD_CACHE_SMOKE_FAKE_DOCKER_STATE="$state" PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" --output-dir "$apply_two" >"$test_dir/apply-two.out" 2>&1; then die 'second fake-Docker apply control flow unexpectedly failed'; fi
  namespace_one=$(awk -F '\t' '$1 == "cache_mount_namespace" { print $2; exit }' "$apply_one/metadata.tsv")
  namespace_two=$(awk -F '\t' '$1 == "cache_mount_namespace" { print $2; exit }' "$apply_two/metadata.tsv")
  nonce_one=$(awk -F '\t' '$1 == "run_nonce" { print $2; exit }' "$apply_one/metadata.tsv")
  nonce_two=$(awk -F '\t' '$1 == "run_nonce" { print $2; exit }' "$apply_two/metadata.tsv")
  [ -n "$namespace_one" ] && [ -n "$namespace_two" ] && [ "$namespace_one" != "$namespace_two" ] || die 'separate smoke invocations reused a cache namespace'
  [ -n "$nonce_one" ] && [ -n "$nonce_two" ] && [ "$nonce_one" != "$nonce_two" ] || die 'separate smoke invocations reused a mktemp-derived nonce'
  awk -F '\t' '$2 == "PASS" || $2 == "EXPECTED_FAILURE" || $2 == "FAIL" { if ($5 != "-") print $5 }' "$apply_one/cases.tsv" | sort -u >"$test_dir/tags-one"
  awk -F '\t' '$2 == "PASS" || $2 == "EXPECTED_FAILURE" || $2 == "FAIL" { if ($5 != "-") print $5 }' "$apply_two/cases.tsv" | sort -u >"$test_dir/tags-two"
  while IFS= read -r tag; do
    [ -n "$tag" ] || continue
    case "$tag" in
      "lagrange-build-cache-smoke-${nonce_one}-"*) ;;
      *) die 'first smoke invocation reported a tag outside its explicit nonce ownership' ;;
    esac
    if grep -Fqx "$tag" "$test_dir/tags-two"; then die 'separate smoke invocations reused a temporary image tag'; fi
  done <"$test_dir/tags-one"
  if ! awk -F '\t' '$1 == "image-rm" { cleanup_count += 1; if ($3 !~ /^lagrange-build-cache-smoke-r[A-Za-z0-9]+-/) bad = 1 } $1 == "system" || $1 == "prune" { bad = 1 } END { exit (cleanup_count > 0 && !bad) ? 0 : 1 }' "$audit"; then die 'cleanup audit was not limited to explicitly owned unique fixture tags'; fi
  while IFS= read -r tag; do
    [ -n "$tag" ] || continue
    if ! awk -F '\t' -v wanted="$tag" '$1 == "image-rm" && $3 == wanted { found = 1 } END { exit !found }' "$audit"; then die 'an explicitly owned fixture image tag was not cleaned up'; fi
  done <"$test_dir/tags-one"

  forced_failure=$test_dir/apply-forced-failure
  if BUILD_CACHE_SMOKE_FAKE_DOCKER_AUDIT="$audit" BUILD_CACHE_SMOKE_FAKE_DOCKER_STATE="$state" BUILD_CACHE_SMOKE_FAKE_FAIL_CASE=changed-embedded-data PATH="$fake_bin:$PATH" bash "$script_dir/build-cache-smoke.sh" --apply --fixture-dir "$fixture_dir" --output-dir "$forced_failure" >"$test_dir/forced-failure.out" 2>&1; then die 'controlled fake build failure unexpectedly passed'; fi
  grep -Fq 'changed-embedded-data'$'\t''FAIL' "$forced_failure/cases.tsv" || die 'unexpected build failure did not record a final FAIL case row'
  [ -s "$forced_failure/logs/changed-embedded-data.build.log" ] || die 'unexpected build failure did not retain its build log'
  grep -Fq 'tool_identity' "$forced_failure/metadata.tsv" || die 'unexpected build failure did not retain tool identity evidence'

  echo 'BUILD_CACHE_SMOKE_SELF_TEST: PASS (fake Docker only; no Docker daemon, container, or Rust compilation)'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--apply|--self-test)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --apply, or --self-test'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --fixture-dir)
      [ "$#" -ge 2 ] || die '--fixture-dir needs an absolute path'
      fixture_dir=$2
      shift 2
      ;;
    --output-dir)
      [ "$#" -ge 2 ] || die '--output-dir needs an absolute path'
      [ "$output_dir_seen" -eq 0 ] || die '--output-dir may be provided only once'
      output_dir=$2
      output_dir_seen=1
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) die "unknown option: $1" ;;
  esac
done

validate_fixture
[ -z "$output_dir" ] || validate_output_dir

case "$mode" in
  plan)
    echo 'BUILD_CACHE_SMOKE_PLAN mode=plan'
    echo "  fixture_dir=$fixture_dir"
    [ -z "$output_dir" ] || echo "  output_dir=$output_dir (validated; not created or written)"
    echo '  cache_ids=build-cache-fixture-registry-v1-*,build-cache-fixture-git-v1-*,build-cache-fixture-target-v1-*'
    echo '  cases=cache-absent repeat-same-source different-bin changed-local-source changed-workspace-library changed-build-script-source changed-external-dependency-lockfile changed-embedded-data changed-build-setting commit-only-change older-mtimes-branch-reversion source-deletion-failure restore-after-source-deletion cache-repopulate-absent cache-repopulate'
    echo '  cold=unique mktemp-derived namespace plus --no-cache; normal-repeat=layer reuse enabled and exact compile RUN must be CACHED'
    echo '  forced-warm=one Cargo clean then verbose binary A/B phases; library compiles for A and is Fresh for B; itoa is Fresh not Compiling'
    echo '  evidence=outside-checkout output directory with source identity, command, package/cache state, total/Cargo timing, tool identity, build logs, and both binary outputs'
    echo '  prohibited=production mounts/tags, global cache prune, provider calls, rollout, background work'
    echo 'PLAN_ONLY: no Docker command, Rust compilation, container launch, temporary checkout, or output write made'
    ;;
  apply) run_apply ;;
  self-test) run_self_test ;;
  *) die "unsupported mode: $mode" ;;
esac
