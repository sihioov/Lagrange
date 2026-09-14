#!/usr/bin/env bash
# Static contract check for owner-beta local image provenance. This never calls
# Docker, a provider, a database, or a runtime service.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
build=$ops/build-production-images.sh
compose_release=$ops/compose-release.sh
manifest_lib=$ops/lib/release-image-manifest.sh
compose_file=$root/deploy/compose/compose.yml

die() { echo "production-image-build-static: $*" >&2; exit 1; }

for path in "$build" "$compose_release" "$manifest_lib"; do
  [ -f "$path" ] || die "required provenance helper is missing: $path"
  bash -n "$path" || die "shell syntax failure: $path"
done
self_test=$ops/build-production-images-self-test.sh
bash -n "$self_test" || die 'image build self-test has shell syntax errors'
grep -Fq 'TEST_ENVIRONMENT_ERROR: image-build root fixture requires user namespaces or fakeroot' \
  "$self_test" || die 'root fixture must fail rather than skip when unavailable'
if grep -Fq 'PASS (apply fixture skipped' "$self_test" || grep -Fq 'fakeroot "$@"' "$self_test"; then
  die 'image build root fixture must not pass-on-skip or nest fakeroot'
fi

smoke=$root/scripts/qa/build-cache-smoke.sh
benchmark=$root/scripts/qa/build-cache-benchmark.sh
for cache_tool in "$smoke" "$benchmark"; do
  [ -f "$cache_tool" ] || die "build-cache verification tool is missing: $cache_tool"
  [ -x "$cache_tool" ] || die "build-cache verification tool must be executable: $cache_tool"
  bash -n "$cache_tool" || die "build-cache verification tool has shell syntax errors: $cache_tool"
  grep -Fq 'mode=plan' "$cache_tool" || die "build-cache tool must default to plan: $cache_tool"
  grep -Fq -- '--apply' "$cache_tool" || die "build-cache tool apply mode is missing: $cache_tool"
  grep -Fq -- '--self-test' "$cache_tool" || die "build-cache tool self-test mode is missing: $cache_tool"
done
grep -Fq 'BUILDKIT_CACHE_MOUNT_NS' "$smoke" || die 'smoke must exercise a cache-mount namespace'
grep -Fq -- '--build-arg "BUILDKIT_CACHE_MOUNT_NS=$namespace"' "$smoke" ||
  die 'smoke must forward the BuildKit cache namespace as an explicit build argument'
grep -Fq -- '--output-dir' "$smoke" || die 'smoke must retain its evidence output directory contract'
grep -Fq -- '--no-cache' "$smoke" || die 'smoke must disable Docker layer reuse'
grep -Fq 'docker run --rm' "$smoke" || die 'smoke must assert disposable binary output'
grep -Fq 'compile-layer-cached' "$smoke" || die 'smoke must identify a cached compile RUN separately from Cargo events'
grep -Fq 'SMOKE_CARGO_PHASE bin=cache-bin-a' "$smoke" || die 'smoke must retain explicit binary-A phase evidence'
grep -Fq 'SMOKE_CARGO_PHASE bin=cache-bin-b' "$smoke" || die 'smoke must retain explicit binary-B phase evidence'
for smoke_case in cache-absent repeat-same-source different-bin changed-local-source \
  changed-workspace-library changed-build-script-source changed-external-dependency-lockfile \
  changed-embedded-data changed-build-setting commit-only-change older-mtimes-branch-reversion \
  source-deletion-failure restore-after-source-deletion cache-repopulate; do
  grep -Fq "$smoke_case" "$smoke" || die "smoke case is missing: $smoke_case"
done
grep -Fq -- '--baseline-commit' "$benchmark" || die 'benchmark baseline revision option is missing'
grep -Fq -- '--candidate-commit' "$benchmark" || die 'benchmark candidate revision option is missing'
grep -Fq -- '--output-dir' "$benchmark" || die 'benchmark output directory option is missing'
grep -Fq 'git clone --quiet --no-local' "$benchmark" || die 'benchmark must use temporary local clones'
grep -Fq 'foreground sequential' "$benchmark" || die 'benchmark sequential foreground policy is missing'
grep -Fq 'CARGO_BUILD_JOBS=2' "$benchmark" || die 'benchmark jobs=2 policy is missing'
grep -Fq 'BUILDKIT_CACHE_MOUNT_NS' "$benchmark" || die 'benchmark cold namespace control is missing'
grep -Fq -- '--no-cache' "$benchmark" || die 'benchmark cold layer-reuse control is missing'
if grep -Eiq 'docker compose|docker-compose|curl|wget|KIS_APP_(KEY|SECRET)|psql' "$smoke" "$benchmark"; then
  die 'build-cache verification tools contain a forbidden provider/lifecycle channel'
fi

grep -Fq 'mode=plan' "$build" || die 'image build helper must default to plan'
grep -Fq -- '--preflight' "$build" || die 'image build preflight mode missing'
grep -Fq -- '--apply' "$build" || die 'image build apply mode missing'
grep -Fq -- '--manifest-file' "$build" || die 'strict manifest output option missing'
grep -Fq -- '--apply requires --manifest-file' "$build" || die 'apply must require a manifest output file'
grep -Fq 'LAGRANGE_RELEASE_MANIFEST_V2' "$manifest_lib" || die 'strict V2 manifest format missing'
grep -Fq 'canonical whitelist/order' "$manifest_lib" || die 'strict stable service ordering missing'
grep -Fq 'manifest record count is not canonical' "$manifest_lib" || die 'strict record count missing'
grep -Fq 'trailing separator' "$manifest_lib" || die 'strict field-separator validation missing'
grep -Fq 'manifest configured image reference disagrees with Compose' "$manifest_lib" ||
  die 'strict configured image reference check missing'
grep -Fq 'local Docker image ID' "$manifest_lib" || die 'local image_id terminology missing'
grep -Fq 'RepoDigest' "$manifest_lib" || die 'single-host provenance terminology missing'
grep -Fq 'docker image inspect' "$build" || die 'built-image inspection missing'
grep -Fq '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' "$build" ||
  die 'image_id/revision inspection contract missing'
grep -Fq 'built image_id is not an exact local Docker image ID' "$build" ||
  die 'image_id validation missing'
grep -Fq 'built image revision label does not match source commit' "$build" ||
  die 'image revision mismatch failure missing'
grep -Fq 'manifest-file already exists; refusing to overwrite it' "$build" ||
  die 'manifest overwrite refusal missing'
grep -Fq 'git -c "safe.directory=$root" -C "$root"' "$build" ||
  die 'Git command-local safe.directory fence missing'
grep -Fq "rev-parse --verify 'HEAD^{commit}'" "$build" || die 'build HEAD provenance check missing'
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$build" ||
  die 'tracked/untracked clean-worktree check missing'
grep -Fq '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' "$build" ||
  die 'official workbook must be the sole untracked build exception'
grep -Fq 'does not match the build root HEAD' "$build" || die 'commit mismatch failure missing'
grep -Fq 'docker compose --env-file' "$build" || die 'Compose env-file invocation missing'
grep -Fq 'config --quiet' "$build" || die 'Compose config preflight missing'
grep -Fq 'build --pull=false' "$build" || die 'pull=false build contract missing'
grep -Fq 'for service in "${local_image_services[@]}"; do' "$build" ||
  die 'local image builds must be sequential by service'
grep -Fq 'compose build --pull=false "$service"' "$build" ||
  die 'sequential single-service build command missing'
grep -Fq 'RESEARCH_APP_ENV=prebuild-disabled' "$build" || die 'research prebuild sentinel missing'
grep -Fq 'RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled' "$build" ||
  die 'entitlement prebuild sentinel missing'
grep -Fq 'BACKTEST_MIN_FREE_BYTES=0' "$build" || die 'backtest prebuild sentinel missing'
grep -Fq 'never edits the env file' "$build" || die 'env-file no-write documentation missing'
grep -Fq 'network caveat' "$build" || die 'build network caveat missing'
if grep -Fq -- '--repo-root' "$build"; then
  die 'public alternate repo-root override must not exist'
fi
if grep -Eq '\.RepoDigests|RepoDigest[s]?[[:space:]:=]' "$build"; then
  die 'build manifest must not claim Docker RepoDigest provenance'
fi

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
grep -Fq 'local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")' "$build" ||
  die 'build helper must derive scope only from the canonical manifest whitelist'

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { inside=1; next }
    inside && /^  [A-Za-z0-9_-]+:/ { exit }
    inside { print }
  ' "$compose_file"
}

for service in "${services[@]}"; do
  grep -Fq -- "  $service" "$manifest_lib" || die "local build service missing: $service"
  block=$(service_block "$service")
  [ -n "$block" ] || die "Compose service block missing: $service"
  grep -Fq "image: lagrange-station-$service:\${LAGRANGE_CODE_COMMIT:?" <<<"$block" ||
    die "Compose image must use exact commit-specific tag: $service"
  grep -Fq 'LAGRANGE_CODE_COMMIT: ${LAGRANGE_CODE_COMMIT:?' <<<"$block" ||
    die "Compose build arg missing for local image: $service"
done

for dockerfile in \
  "$root/deploy/db/Dockerfile" \
  "$root/crates/api-server/Dockerfile" \
  "$root/crates/job-queue/Dockerfile" \
  "$root/crates/job-queue/Dockerfile.backtest-runner" \
  "$root/crates/job-queue/Dockerfile.owner-beta-runner" \
  "$root/crates/job-queue/Dockerfile.owner-equity-v2-runner" \
  "$root/deploy/runtime/Dockerfile.paper-runner" \
  "$root/data-pipelines/collectors/Dockerfile"
do
  grep -Fq 'ENV CARGO_BUILD_JOBS=2' "$dockerfile" ||
    die "production Rust Dockerfile must cap Cargo parallelism at two jobs: $dockerfile"
done

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
for service in "${services[@]}"; do
  dockerfile=$root/${service_dockerfile[$service]}
  grep -Fq 'ARG LAGRANGE_CODE_COMMIT' "$dockerfile" ||
    die "Dockerfile build ARG missing: $service"
  grep -Fq 'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"' "$dockerfile" ||
    die "Dockerfile OCI revision label missing: $service"
  grep -Fq "grep -Eq '^[0-9a-f]{40}$'" "$dockerfile" ||
    die "Dockerfile exact revision validation missing: $service"
  case "$service" in
    db-role-bootstrap|db-migrate|web) ;;
    *)
      grep -Fq 'COPY configs/evidence/kis-historical-price-only-beta-approved-artifacts.json ./configs/evidence/kis-historical-price-only-beta-approved-artifacts.json' "$dockerfile" ||
        die "Dockerfile missing embedded historical-price-only approval registry: $service"
      grep -Fq 'COPY configs/evidence/kis-historical-price-only-v3-approved-artifacts.json ./configs/evidence/kis-historical-price-only-v3-approved-artifacts.json' "$dockerfile" ||
        die "Dockerfile missing embedded historical-price-only V3 approval registry: $service"
      grep -Fq 'COPY configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json ./configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json' "$dockerfile" ||
        die "Dockerfile missing embedded fixed-stock beta approval registry: $service"
      ;;
  esac
done

# The cache contract is deliberately checked against the seven Dockerfiles
# that compile the release binaries. The database image installs sqlx-cli and
# the web image is Node-only; neither owns a release Rust binary set. Keep the
# expected lists here so a cache edit cannot silently reorder or drop one
# executable while the output staging directory still appears healthy.
cache_registry_path=/usr/local/cargo/registry
cache_git_path=/usr/local/cargo/git
cache_target_path=/cargo-target
cache_registry_id=lagrange-cargo-registry-v1
cache_git_id=lagrange-cargo-git-v1
cache_target_id='lagrange-cargo-target-v1-rust-1.97.1-alpine-3c38f3f82c2f-${TARGETPLATFORM}-release'

rust_build_dockerfiles=(
  crates/api-server/Dockerfile
  crates/job-queue/Dockerfile
  crates/job-queue/Dockerfile.backtest-runner
  crates/job-queue/Dockerfile.owner-beta-runner
  crates/job-queue/Dockerfile.owner-equity-v2-runner
  data-pipelines/collectors/Dockerfile
  deploy/runtime/Dockerfile.paper-runner
)
declare -A expected_build_commands=(
  [crates/api-server/Dockerfile]=$'cargo build --locked --release --package api-server --bin api-server'
  [crates/job-queue/Dockerfile]=$'cargo build --locked --release --package job-queue --bin recommendation-runner\ncargo build --locked --release --package job-queue --bin candidate-runner'
  [crates/job-queue/Dockerfile.backtest-runner]=$'cargo build --locked --release --package job-queue --bin backtest-runner'
  [crates/job-queue/Dockerfile.owner-beta-runner]=$'cargo build --locked --release --package job-queue --bin owner-beta-runner'
  [crates/job-queue/Dockerfile.owner-equity-v2-runner]=$'cargo build --locked --release --package job-queue --bin owner-equity-v2-runner'
  [data-pipelines/collectors/Dockerfile]=$'cargo build --locked --release --package collectors --bin research-worker\ncargo build --locked --release --package collectors --bin kis-historical-price-beta-artifact\ncargo build --locked --release --package collectors --bin kis-historical-price-v3-artifact\ncargo build --locked --release --package collectors --bin kis-historical-price-beta-approval-check\ncargo build --locked --release --package collectors --bin kis-action-range-raw\ncargo build --locked --release --package collectors --bin kis-stock-price-beta-raw\ncargo build --locked --release --package collectors --bin kis-stock-price-beta-materialize\ncargo build --locked --release --package collectors --bin kis-historical-price-v3-input-check\ncargo build --locked --release --package collectors --bin owner-equity-v2-check\ncargo build --locked --release --package collectors --bin owner-equity-v2-materialize'
  [deploy/runtime/Dockerfile.paper-runner]=$'cargo build --locked --release --package api-server --bin paper-runner'
)
declare -A expected_runtime_copies=(
  [crates/api-server/Dockerfile]=$'api-server:/usr/local/bin/api-server'
  [crates/job-queue/Dockerfile]=$'recommendation-runner:/usr/local/bin/recommendation-runner\ncandidate-runner:/usr/local/bin/candidate-runner'
  [crates/job-queue/Dockerfile.backtest-runner]=$'backtest-runner:/usr/local/bin/backtest-runner'
  [crates/job-queue/Dockerfile.owner-beta-runner]=$'owner-beta-runner:/usr/local/bin/owner-beta-runner'
  [crates/job-queue/Dockerfile.owner-equity-v2-runner]=$'owner-equity-v2-runner:/usr/local/bin/owner-equity-v2-runner'
  [data-pipelines/collectors/Dockerfile]=$'research-worker:/usr/local/bin/research-worker\nkis-historical-price-beta-artifact:/usr/local/bin/kis-historical-price-beta-artifact\nkis-historical-price-v3-artifact:/usr/local/bin/kis-historical-price-v3-artifact\nkis-historical-price-beta-approval-check:/usr/local/bin/kis-historical-price-beta-approval-check\nkis-action-range-raw:/usr/local/bin/kis-action-range-raw\nkis-stock-price-beta-raw:/usr/local/bin/kis-stock-price-beta-raw\nkis-stock-price-beta-materialize:/usr/local/bin/kis-stock-price-beta-materialize\nkis-historical-price-v3-input-check:/usr/local/bin/kis-historical-price-v3-input-check\nowner-equity-v2-check:/usr/local/bin/owner-equity-v2-check\nowner-equity-v2-materialize:/usr/local/bin/owner-equity-v2-materialize'
  [deploy/runtime/Dockerfile.paper-runner]=$'paper-runner:/usr/local/bin/paper-runner-bin'
)

# Return each Dockerfile RUN instruction as one record. This keeps the
# one-line Paper instruction and the continued instructions equivalent for the
# assertions below and lets us prove that clean/build/copy are one RUN.
run_instructions() {
  local dockerfile=$1
  awk '
    function flush() {
      if (block != "") {
        printf "%s\034", block
      }
      block = ""
      active = 0
    }
    !active && /^[[:space:]]*RUN([[:space:]]|$)/ {
      block = $0
      active = 1
      if ($0 !~ /\\[[:space:]]*$/) flush()
      next
    }
    active {
      block = block "\n" $0
      if ($0 !~ /\\[[:space:]]*$/) flush()
      next
    }
    END {
      if (active) flush()
    }
  ' "$dockerfile"
}

for relative_dockerfile in "${rust_build_dockerfiles[@]}"; do
  dockerfile=$root/$relative_dockerfile
  [ -f "$dockerfile" ] || die "cache-contract Dockerfile is missing: $relative_dockerfile"
  builder_stage=$(awk '
    /^FROM[[:space:]].*[[:space:]]AS[[:space:]]builder[[:space:]]*$/ { inside = 1; print; next }
    inside && /^FROM[[:space:]]/ { exit }
    inside { print }
  ' "$dockerfile")
  [ -n "$builder_stage" ] || die "cache-contract builder stage is missing: $relative_dockerfile"
  grep -Eq '^[[:space:]]*ARG TARGETPLATFORM([[:space:]]|$)' <<<"$builder_stage" ||
    die "cache-contract TARGETPLATFORM ARG is missing: $relative_dockerfile"
  grep -Eq '^[[:space:]]*ENV CARGO_TARGET_DIR=/cargo-target([[:space:]]|$)' <<<"$builder_stage" ||
    die "cache-contract CARGO_TARGET_DIR=/cargo-target is missing: $relative_dockerfile"
  grep -Fq 'ENV CARGO_BUILD_JOBS=2' <<<"$builder_stage" ||
    die "cache-contract CARGO_BUILD_JOBS=2 is missing: $relative_dockerfile"

  all_builds=$(grep -oE 'cargo build --locked --release --package [[:alnum:]_.-]+ --bin [[:alnum:]_.-]+' \
    "$dockerfile" || true)
  [ "$all_builds" = "${expected_build_commands[$relative_dockerfile]}" ] ||
    die "release binary build set/order changed: $relative_dockerfile"

  clean_count=$(grep -Foc 'cargo clean --workspace --release --locked' "$dockerfile" || true)
  [ "$clean_count" -eq 1 ] ||
    die "cache-contract requires exactly one workspace-only release clean: $relative_dockerfile"

  compile_blocks=$(run_instructions "$dockerfile" |
    awk 'BEGIN { RS = "\034" } index($0, "cargo build --locked --release") { printf "%s\034", $0 }')
  compile_run_count=$(printf '%s' "$compile_blocks" |
    awk 'BEGIN { RS = "\034" } NF { count++ } END { print count + 0 }')
  [ "$compile_run_count" -eq 1 ] ||
    die "cache-contract requires one compile RUN containing the complete build set: $relative_dockerfile"
  compile_block=${compile_blocks%$'\034'}
  grep -Fq 'cargo clean --workspace --release --locked' <<<"$compile_block" ||
    die "workspace-only clean is not in the compile RUN: $relative_dockerfile"
  first_cargo=$(grep -oE 'cargo (clean|build)[[:space:]]' <<<"$compile_block" |
    head -n 1 | sed 's/[[:space:]]*$//' || true)
  [ "$first_cargo" = 'cargo clean' ] ||
    die "workspace-only clean is not the first Cargo action: $relative_dockerfile"
  before_clean=${compile_block%%cargo clean --workspace --release --locked*}
  before_clean=$(printf '%s' "$before_clean" |
    sed -E 's/--mount=type=cache,[^[:space:]]+//g; s/\\//g; s/[[:space:]]+//g; s/^RUN//')
  [ -z "$before_clean" ] ||
    die "compile RUN has an action before cargo clean: $relative_dockerfile"

  mapfile -t cache_mounts < <(grep -oE -- '--mount=type=cache,[^[:space:]]+' <<<"$compile_block" || true)
  [ "${#cache_mounts[@]}" -eq 3 ] ||
    die "compile RUN must have exactly three cache mounts: $relative_dockerfile"
  cache_mount_has() {
    local path=$1 id=$2 mount
    for mount in "${cache_mounts[@]}"; do
      if [[ ",$mount," == *",target=$path,"* &&
        ",$mount," == *",id=$id,"* &&
        ",$mount," == *",sharing=locked,"* ]]; then
        return 0
      fi
    done
    return 1
  }
  cache_mount_has "$cache_registry_path" "$cache_registry_id" ||
    die "registry cache mount/path/id/sharing contract failed: $relative_dockerfile"
  cache_mount_has "$cache_git_path" "$cache_git_id" ||
    die "git cache mount/path/id/sharing contract failed: $relative_dockerfile"
  cache_mount_has "$cache_target_path" "$cache_target_id" ||
    die "target cache mount/path/id/sharing contract failed: $relative_dockerfile"
  if grep -Eq -- '--mount=type=cache,[^[:space:]]*id=[^[:space:],]*LAGRANGE_CODE_COMMIT' <<<"$compile_block"; then
    die "cache mount id must not contain the source commit: $relative_dockerfile"
  fi
  grep -Fq 'mkdir -p /build/target/release' <<<"$compile_block" ||
    die "compile outputs are not staged outside the target cache: $relative_dockerfile"
  # Continued Dockerfile instructions and the one-line Paper instruction have
  # different physical line layouts. Check command order inside the complete
  # RUN record instead of comparing source line numbers.
  after_last_build=${compile_block##*cargo build --locked --release}
  case "$after_last_build" in
    *'mkdir -p /build/target/release'*) ;;
    *) die "compile output staging commands are incomplete: $relative_dockerfile" ;;
  esac
  after_staging_dir=${after_last_build#*mkdir -p /build/target/release}
  case "$after_staging_dir" in
    *'cp /cargo-target/release/'*) ;;
    *) die "compiled outputs are not copied after all successful builds: $relative_dockerfile" ;;
  esac
  expected_copy_order=$(printf '%s\n' "${expected_runtime_copies[$relative_dockerfile]}" |
    sed -E 's/:.*$//')
  actual_copy_order=$(grep -oE 'cp[[:space:]]+(--[[:space:]]+)?/cargo-target/release/[[:alnum:]_.-]+[[:space:]]+/build/target/release/[[:alnum:]_.-]+' \
    <<<"$compile_block" || true)
  actual_copy_order=$(printf '%s\n' "$actual_copy_order" |
    sed -nE 's#.*cargo-target/release/([^[:space:]]+).*#\1#p')
  [ "$actual_copy_order" = "$expected_copy_order" ] ||
    die "staged executable copy set/order changed: $relative_dockerfile"
  expected_runtime_order=$(printf '%s\n' "${expected_runtime_copies[$relative_dockerfile]}" |
    sed -E 's/:.*$//')
  actual_runtime_order=$(grep -E '^COPY --from=builder /build/target/release/[[:alnum:]_.-]+ /usr/local/bin/[[:alnum:]_.-]+$' \
    "$dockerfile" |
    sed -E 's#^COPY --from=builder /build/target/release/([^[:space:]]+).*$#\1#')
  [ "$actual_runtime_order" = "$expected_runtime_order" ] ||
    die "runtime executable COPY set/order changed: $relative_dockerfile"

  while IFS=: read -r binary destination; do
    grep -Eq "(^|[[:space:]&;])cp[[:space:]]+(--[[:space:]]+)?/cargo-target/release/${binary}[[:space:]]+/build/target/release/${binary}([[:space:]&;\\]|$)" \
      <<<"$compile_block" ||
      die "compiled executable is not copied out of cache in the same RUN: $relative_dockerfile ($binary)"
    grep -Fq "COPY --from=builder /build/target/release/$binary $destination" "$dockerfile" ||
      die "runtime binary COPY path changed: $relative_dockerfile ($binary)"
  done <<<"${expected_runtime_copies[$relative_dockerfile]}"
done

# Ordinary recommendation/candidate and backtest images need NT only in their
# runtime stage. It is not a Rust build input and must not be copied into the
# builder or staged executable directory by the cache edit.
for nt_dockerfile in "$root/crates/job-queue/Dockerfile" "$root/crates/job-queue/Dockerfile.backtest-runner"; do
  nt_runtime=$(sed -n '/^FROM python@/,$p' "$nt_dockerfile")
  nt_builder=$(sed -n '1,/^FROM python@/p' "$nt_dockerfile")
  grep -Fqx 'COPY nt ./nt' <<<"$nt_runtime" ||
    die "ordinary jobqueue/backtest NT runtime COPY is missing: $nt_dockerfile"
  if grep -Eq '^COPY nt ./nt$|^COPY --from=builder .*[/]nt ' <<<"$nt_builder"; then
    die "ordinary jobqueue/backtest builder must not copy NT: $nt_dockerfile"
  fi
  if grep -Eq '/(cargo-target|build/target/release)/[^[:space:]]*nt' <<<"$nt_runtime"; then
    die "ordinary jobqueue/backtest NT entered the executable staging path: $nt_dockerfile"
  fi
done

# The runtime deployment diagram cites the Paper compile instruction by line;
# keep that evidence reference live when the build RUN is rewritten.
grep -Fq 'deploy/runtime/Dockerfile.paper-runner:26' "$root/docs/diagrams/runtime_deployment.puml" ||
  die 'Paper runtime diagram no longer cites the Paper Dockerfile compile line'
paper_evidence_line=$(grep -F 'deploy/runtime/Dockerfile.paper-runner:26' "$root/docs/diagrams/runtime_deployment.puml")
grep -Fq 'cargo build' <<<"$paper_evidence_line" ||
  die 'Paper runtime diagram evidence no longer describes the compile command'
paper_line=$(sed -n '26p' "$root/deploy/runtime/Dockerfile.paper-runner")
grep -Fq 'cargo build --locked --release --package api-server --bin paper-runner' <<<"$paper_line" ||
  die 'Paper Dockerfile compile command moved away from the diagram evidence line'

grep -Fqx '!configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json' "$root/.dockerignore" ||
  die 'Docker build context is missing the fixed-stock beta approval registry allowlist entry'

# The profile-gated range service has a deliberately different, operator-gated
# history-capture contract and may not enter the serving build/manifest scope.
range_block=$(service_block research-range-raw)
grep -Fq 'profiles: ["range-raw"]' <<<"$range_block" || die 'range-raw profile gate missing'
grep -Fq 'owner-beta serving manifest' "$compose_file" ||
  die 'range-raw manifest exclusion is undocumented'
if sed -n '/RELEASE_IMAGE_SERVICES=(/,/)/p' "$manifest_lib" | grep -Fq 'research-range-raw'; then
  die 'range-raw must not join the local serving image manifest'
fi

grep -Fq 'prepare_installed_release_manifest' "$compose_release" ||
  die 'installed-manifest release guard missing'
grep -Fq 'release_image_manifest_write_compose_override' "$compose_release" ||
  die 'immutable Compose override generator missing'
grep -Fq 'build: !reset null' "$manifest_lib" || die 'Compose build reset contract missing'
grep -Fq 'compose up --no-build' "$compose_release" || die 'release up must pass --no-build'
run_without_build_count=$(awk '
  /^# Owner-beta serving release:/ { in_release = 1 }
  in_release && /compose run --rm --no-deps/ { count++ }
  END { print count + 0 }
' "$compose_release")
[ "$run_without_build_count" -eq 4 ] || die 'release must have exactly four manifest-bound one-shot runs without --build'
if awk '/^# Owner-beta serving release:/ { in_release = 1 } in_release { print }' \
    "$compose_release" | grep -Eq 'compose run .*--build|compose run --no-build'; then
  die 'release one-shots must omit the opt-in --build and unsupported --no-build flags'
fi
grep -Fq 'verify_manifest_images' "$compose_release" || die 'pre-start image_id verification missing'
grep -Fq "'{{.Image}}|{{index .Config.Labels \"org.opencontainers.image.revision\"}}'" \
  "$compose_release" || die 'running-container image_id/revision inspection missing'
grep -Fq 'verify_running_container api-server' "$compose_release" ||
  die 'API running-container identity check missing'
grep -Fq 'verify_running_container web' "$compose_release" ||
  die 'web running-container identity check missing'
grep -Fq 'persistent service image_id mismatch' "$compose_release" ||
  die 'running-container image_id mismatch failure missing'
grep -Fq 'persistent service image revision mismatch' "$compose_release" ||
  die 'running-container revision mismatch failure missing'
release_apply_block=$(awk '
  /^# Owner-beta serving release:/ { inside=1 }
  inside { print }
' "$compose_release")
if grep -Fq 'compose build' <<<"$release_apply_block"; then
  die 'owner-beta release apply must not rebuild images'
fi
if grep -E '^compose up ' <<<"$release_apply_block" | grep -Fv -- '--no-build' >/dev/null; then
  die 'every owner-beta release up must pass --no-build'
fi
if grep -E '^compose run ' <<<"$release_apply_block" | grep -Eq -- '--build|--no-build'; then
  die 'owner-beta release run must omit both --build and unsupported --no-build'
fi

# Cargo resolves every workspace member manifest while loading a package, even
# when a member is only a dev-dependency. Keep this pre-existing build-context
# contract explicit for every Dockerfile that runs a workspace cargo build.
while IFS= read -r dockerfile; do
  if ! grep -Eq '^[[:space:]]*RUN[[:space:]].*cargo[[:space:]]+build([[:space:]]|$)' "$dockerfile"; then
    continue
  fi
  for copy_contract in \
    'COPY Cargo.toml Cargo.lock rust-toolchain.toml ./' \
    'COPY crates ./crates' \
    'COPY data-pipelines/collectors ./data-pipelines/collectors' \
    'COPY apps/api-server/auth ./apps/api-server/auth' \
    'COPY tests/integration/migration-contract ./tests/integration/migration-contract' \
    'COPY configs/evidence/kis-range-canonical-approved-manifests.json ./configs/evidence/kis-range-canonical-approved-manifests.json' \
    'COPY data/calendars/xkrx/calendar.json ./data/calendars/xkrx/calendar.json' \
    'COPY data/calendars/xkrx/manifest.json ./data/calendars/xkrx/manifest.json' \
    'COPY data/calendars/xkrx/overrides.json ./data/calendars/xkrx/overrides.json'; do
    grep -Fq -- "$copy_contract" "$dockerfile" ||
      die "Rust workspace Dockerfile is missing required copy contract: $dockerfile ($copy_contract)"
  done
done < <(find "$root" -type f \( -name Dockerfile -o -name 'Dockerfile.*' \) \
  -not -path "$root/.git/*" -print | sort)

collector_dockerfile=$root/data-pipelines/collectors/Dockerfile
universe_copy='COPY configs/universes/kr-etf-core-v1.yaml ./configs/universes/kr-etf-core-v1.yaml'
grep -Fq -- "$universe_copy" "$collector_dockerfile" ||
  die "range worker Dockerfile is missing immutable input copy: $universe_copy"
for range_context in \
  '!configs/evidence/kis-range-canonical-approved-manifests.json' \
  '!data/calendars/xkrx/calendar.json' \
  '!data/calendars/xkrx/manifest.json' \
  '!data/calendars/xkrx/overrides.json'; do
  grep -Fqx -- "$range_context" "$root/.dockerignore" ||
    die "Docker build context does not allow required range input: $range_context"
done

if grep -Eq '^[[:space:]]*(docker compose|compose)[[:space:]].*(up|run|restart|start)[[:space:]]' "$build"; then
  die 'image prebuild helper must not invoke a lifecycle command'
fi
for forbidden in kis_app_key kis_app_secret auth0_client_secret psql curl wget; do
  if grep -Eiq "^[^#]*$forbidden" "$build"; then
    die "image prebuild helper must not read or invoke $forbidden"
  fi
done
if grep -Eq '^[[:space:]]*git[[:space:]]+rev-parse' "$build"; then
  die 'image prebuild helper must not derive the commit from a shell checkout command'
fi

echo 'PRODUCTION_IMAGE_BUILD_STATIC: PASS'
