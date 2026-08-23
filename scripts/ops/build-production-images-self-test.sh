#!/usr/bin/env bash
# Fake-Docker self-test for the strict V2 owner-beta image build. It exercises
# only throw-away Git fixtures and never contacts a Docker daemon or provider.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source_helper=$script_dir/build-production-images.sh
source_manifest_lib=$script_dir/lib/release-image-manifest.sh

# The build helper's --apply contract is root-only. Establish one root-like
# fixture process once; commands inside that child run directly, never through
# a nested fakeroot wrapper. A missing runner is a test-environment failure,
# not a passing skipped test.
if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD:-0}" = 1 ]; then
    echo 'TEST_ENVIRONMENT_ERROR: image-build root fixture did not obtain root identity' >&2
    exit 1
  fi
  if unshare -Ur true >/dev/null 2>&1; then
    exec unshare -Ur env LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/build-production-images-self-test.sh" "$@"
  fi
  if command -v fakeroot >/dev/null 2>&1; then
    exec fakeroot env LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/build-production-images-self-test.sh" "$@"
  fi
  echo 'TEST_ENVIRONMENT_ERROR: image-build root fixture requires user namespaces or fakeroot' >&2
  exit 1
fi

out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-image-build-self-test.XXXXXX")
trap 'rm -rf -- "$out_dir"' EXIT
repo_dir=$out_dir/repo
helper=$repo_dir/scripts/ops/build-production-images.sh
manifest_lib=$repo_dir/scripts/ops/lib/release-image-manifest.sh
compose_file=$repo_dir/deploy/compose/compose.yml
env_file=$repo_dir/deploy/compose/.env
fake_bin=$out_dir/fake-bin
docker_log=$out_dir/docker.log
manifest_file=$out_dir/production-images.manifest

mkdir -p "$fake_bin" "$(dirname "$compose_file")" "$(dirname "$helper")" \
  "$(dirname "$manifest_lib")"
cp -- "$source_helper" "$helper"
cp -- "$source_manifest_lib" "$manifest_lib"
chmod 0755 "$helper"
printf 'services: {}\n' >"$compose_file"
printf 'COMPOSE_TEST=1\n' >"$env_file"
git -C "$repo_dir" init -q
git -C "$repo_dir" config user.email fixture@example.invalid
git -C "$repo_dir" config user.name fixture
git -C "$repo_dir" add scripts/ops/build-production-images.sh \
  scripts/ops/lib/release-image-manifest.sh deploy/compose/compose.yml deploy/compose/.env
git -C "$repo_dir" commit -qm fixture
commit=$(git -C "$repo_dir" rev-parse HEAD)

cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
joined=$*
printf 'commit=%s args=%s\n' "${LAGRANGE_CODE_COMMIT:-missing}" "$joined" >>"${IMAGE_BUILD_DOCKER_LOG:?}"
if [ "${1:-}" = image ] && [ "${2:-}" = inspect ]; then
  image_ref=${!#}
  case "$image_ref" in
    lagrange-station-db-role-bootstrap:"${LAGRANGE_CODE_COMMIT}") index=1 ;;
    lagrange-station-db-migrate:"${LAGRANGE_CODE_COMMIT}") index=2 ;;
    lagrange-station-api-server:"${LAGRANGE_CODE_COMMIT}") index=3 ;;
    lagrange-station-web:"${LAGRANGE_CODE_COMMIT}") index=4 ;;
    lagrange-station-research-worker:"${LAGRANGE_CODE_COMMIT}") index=5 ;;
    lagrange-station-recommendation-runner:"${LAGRANGE_CODE_COMMIT}") index=6 ;;
    lagrange-station-candidate-runner:"${LAGRANGE_CODE_COMMIT}") index=7 ;;
    lagrange-station-nt-backtest-worker-1:"${LAGRANGE_CODE_COMMIT}") index=8 ;;
    lagrange-station-nt-backtest-worker-2:"${LAGRANGE_CODE_COMMIT}") index=9 ;;
    lagrange-station-paper-scheduler:"${LAGRANGE_CODE_COMMIT}") index=10 ;;
    *) echo "unexpected fake Docker image: $image_ref" >&2; exit 1 ;;
  esac
  image_id=$(printf 'sha256:%064d' "$index")
  printf '%s|%s\n' "${IMAGE_BUILD_FAKE_IMAGE_ID:-$image_id}" \
    "${IMAGE_BUILD_FAKE_REVISION:-${LAGRANGE_CODE_COMMIT:-missing}}"
  exit 0
fi
case "$joined" in
  *' version') exit 0 ;;
  *' config --quiet') exit 0 ;;
  *' build --pull=false '*) exit 0 ;;
  *)
    echo "unexpected fake Docker command: $joined" >&2
    exit 1
    ;;
esac
EOF
chmod 0755 "$fake_bin/docker"
export PATH="$fake_bin:$PATH"
export IMAGE_BUILD_DOCKER_LOG=$docker_log
export LAGRANGE_CODE_COMMIT=$commit

bash "$helper" --plan --compose-file "$compose_file" --env-file "$env_file" >"$out_dir/plan.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PLAN mode=plan' "$out_dir/plan.out"
grep -Fq 'research-range-raw' "$out_dir/plan.out"
grep -Fq 'network caveat:' "$out_dir/plan.out"
[ ! -s "$docker_log" ]

bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" >"$out_dir/preflight.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS' "$out_dir/preflight.out"
grep -Fq 'config --quiet' "$docker_log"

before_env=$(sha256sum "$env_file")
bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$manifest_file" >"$out_dir/apply.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD: PASS' "$out_dir/apply.out"
[ "$before_env" = "$(sha256sum "$env_file")" ]
[ "$(stat -c %a "$manifest_file")" = 600 ]
grep -Fxq 'LAGRANGE_RELEASE_MANIFEST_V2' "$manifest_file"
grep -Fxq "commit|$commit" "$manifest_file"
[ "$(grep -c '^image|' "$manifest_file")" -eq 10 ]
index=0
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  index=$((index + 1))
  image_id=$(printf 'sha256:%064d' "$index")
  grep -Fxq "image|$service|lagrange-station-$service:$commit|$image_id|$commit" \
    "$manifest_file"
done

config_count=$(grep -Fc "commit=$commit args=compose --env-file $env_file --file $compose_file config --quiet" "$docker_log")
build_count=$(grep -Fc "commit=$commit args=compose --env-file $env_file --file $compose_file build --pull=false" "$docker_log")
[ "$config_count" -eq 2 ]
[ "$build_count" -eq 1 ]
build_line=$(grep "commit=$commit args=compose --env-file $env_file --file $compose_file build --pull=false" "$docker_log")
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  grep -Fq -- " $service" <<<"$build_line"
done
for excluded in reverse-proxy postgres research-range-raw live-node-owner; do
  if grep -Fq -- " $excluded" <<<"$build_line"; then
    echo "self-test: excluded service entered local image build: $excluded" >&2
    exit 1
  fi
done
if grep -Eiq ' (up|run|restart|start)( |$)' "$docker_log"; then
  echo 'self-test: image helper invoked a container lifecycle command' >&2
  exit 1
fi

if bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/missing-manifest.out" 2>&1; then
  echo 'self-test: apply without manifest path unexpectedly passed' >&2
  exit 1
fi
grep -Fq -- '--apply requires --manifest-file' "$out_dir/missing-manifest.out"

if IMAGE_BUILD_FAKE_REVISION=0123456789abcdef0123456789abcdef01234567 \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$out_dir/revision-mismatch.manifest" \
  >"$out_dir/revision-mismatch.out" 2>&1; then
  echo 'self-test: mismatched image revision unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'built image revision label does not match source commit' "$out_dir/revision-mismatch.out"
[ ! -e "$out_dir/revision-mismatch.manifest" ]

if IMAGE_BUILD_FAKE_IMAGE_ID=sha256:not-a-real-image-id \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$out_dir/image-id-mismatch.manifest" \
  >"$out_dir/image-id-mismatch.out" 2>&1; then
  echo 'self-test: malformed image_id unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'built image_id is not an exact local Docker image ID' "$out_dir/image-id-mismatch.out"
[ ! -e "$out_dir/image-id-mismatch.manifest" ]

: >"$docker_log"
if LAGRANGE_CODE_COMMIT=0123456789abcdef0123456789abcdef01234567 \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/mismatch.out" 2>&1; then
  echo 'self-test: mismatched commit unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'does not match the build root HEAD' "$out_dir/mismatch.out"
[ ! -s "$docker_log" ]

printf 'untracked fixture\n' >"$repo_dir/untracked.fixture"
if LAGRANGE_CODE_COMMIT="$commit" \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/dirty.out" 2>&1; then
  echo 'self-test: dirty build root unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'worktree is not clean' "$out_dir/dirty.out"
rm -f -- "$repo_dir/untracked.fixture"

mkdir -p "$repo_dir/docs"
printf 'official workbook fixture\n' >"$repo_dir/docs/kis_openapi_entiredocs_20260818_030007.xlsx"
LAGRANGE_CODE_COMMIT="$commit" \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/allowed-workbook.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS' "$out_dir/allowed-workbook.out"
rm -f -- "$repo_dir/docs/kis_openapi_entiredocs_20260818_030007.xlsx"

if LAGRANGE_CODE_COMMIT=not-a-commit \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/invalid.out" 2>&1; then
  echo 'self-test: invalid commit unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'exactly 40 lowercase hexadecimal' "$out_dir/invalid.out"

echo 'PRODUCTION_IMAGE_BUILD_SELF_TEST: PASS'
