#!/usr/bin/env bash
# Fake-Docker self-test for build-production-images.sh. It exercises plan,
# Compose config preflight, commit propagation, exact service selection, and
# the no-lifecycle boundary without contacting Docker or a provider.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source_helper="$script_dir/build-production-images.sh"

if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_IMAGE_BUILD_FAKEROOT_CHILD:-0}" != 1 ] && command -v fakeroot >/dev/null 2>&1; then
    export LAGRANGE_IMAGE_BUILD_FAKEROOT_CHILD=1
    exec fakeroot bash -c '
      id() {
        if [ "${1:-}" = -u ]; then
          echo 0
        else
          command id "$@"
        fi
      }
      export -f id
      exec bash "$1" "${@:2}"
    ' _ "$script_dir/build-production-images-self-test.sh" "$@"
  fi
  echo 'PRODUCTION_IMAGE_BUILD_SELF_TEST: PASS (apply fixture skipped; fakeroot unavailable)'
  exit 0
fi

out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-image-build-self-test.XXXXXX")
trap 'rm -rf -- "$out_dir"' EXIT
repo_dir="$out_dir/repo"
helper="$repo_dir/scripts/ops/build-production-images.sh"
compose_file="$repo_dir/deploy/compose/compose.yml"
env_file="$repo_dir/deploy/compose/.env"
fake_bin="$out_dir/fake-bin"
docker_log="$out_dir/docker.log"

mkdir -p "$fake_bin" "$(dirname "$compose_file")" "$(dirname "$helper")"
cp -- "$source_helper" "$helper"
chmod 0755 "$helper"
printf 'services: {}\n' >"$compose_file"
printf 'COMPOSE_TEST=1\n' >"$env_file"
git -C "$repo_dir" init -q
git -C "$repo_dir" config user.email fixture@example.invalid
git -C "$repo_dir" config user.name fixture
git -C "$repo_dir" add scripts/ops/build-production-images.sh \
  deploy/compose/compose.yml deploy/compose/.env
git -C "$repo_dir" commit -qm fixture
commit=$(git -C "$repo_dir" rev-parse HEAD)

cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
joined=$*
printf 'commit=%s args=%s\n' "${LAGRANGE_CODE_COMMIT:-missing}" "$joined" >>"$IMAGE_BUILD_DOCKER_LOG"
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
export IMAGE_BUILD_DOCKER_LOG="$docker_log"

export LAGRANGE_CODE_COMMIT="$commit"
bash "$helper" --plan --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/plan.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PLAN mode=plan' "$out_dir/plan.out"
grep -Fq 'network caveat:' "$out_dir/plan.out"
[ ! -s "$docker_log" ]

bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/preflight.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS' "$out_dir/preflight.out"
grep -Fq 'config --quiet' "$docker_log"

before_env=$(sha256sum "$env_file")
bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/apply.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD: PASS' "$out_dir/apply.out"
[ "$before_env" = "$(sha256sum "$env_file")" ]

config_count=$(grep -Fc "commit=$commit args=compose --env-file $env_file --file $compose_file config --quiet" "$docker_log")
build_count=$(grep -Fc "commit=$commit args=compose --env-file $env_file --file $compose_file build --pull=false" "$docker_log")
[ "$config_count" -eq 2 ]
[ "$build_count" -eq 1 ]
build_line=$(grep "commit=$commit args=compose --env-file $env_file --file $compose_file build --pull=false" "$docker_log")
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler reverse-proxy; do
  grep -Fq -- " $service" <<<"$build_line"
done
if grep -Eiq ' (up|run|restart|start)( |$)' "$docker_log"; then
  echo 'self-test: image helper invoked a container lifecycle command' >&2
  exit 1
fi

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
[ ! -s "$docker_log" ]
rm -f -- "$repo_dir/untracked.fixture"

mkdir -p "$repo_dir/docs"
printf 'official workbook fixture\n' \
  >"$repo_dir/docs/kis_openapi_entiredocs_20260818_030007.xlsx"
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
[ ! -s "$docker_log" ]

echo 'PRODUCTION_IMAGE_BUILD_SELF_TEST: PASS'
