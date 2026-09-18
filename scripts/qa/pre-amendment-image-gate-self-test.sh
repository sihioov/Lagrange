#!/usr/bin/env bash
# Provider-free execution coverage for the documented pre-amendment Docker gate.
# The test extracts the exact runbook fence. Its fixture guard only supplies
# trusted release-parser state; it does not duplicate Docker/image predicates.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
runbook=$root/docs/runbooks/production-release-and-backup.md
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-pre-amendment-image-gate.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

gate=$tmp/pre-amendment-image-gate.sh
awk '
  $0 == "### Pre-amendment Docker image and daemon gate" { section = 1; next }
  section && $0 == "```bash" { fence = 1; next }
  fence && $0 == "```" { exit }
  fence { print }
' "$runbook" >"$gate"
[ -s "$gate" ] || {
  echo 'pre-amendment-image-gate-self-test: gate fence was not found' >&2
  exit 1
}
bash -n "$gate"

fake_bin=$tmp/bin
state=$tmp/state
release_root=$tmp/release-root
release_commit=0123456789abcdef0123456789abcdef01234567
release_dir=$release_root/releases/$release_commit
fixture_id="sha256:$(printf '%064d' 7)"
fixture_retagged_id="sha256:$(printf '%064d' 8)"
mkdir -p "$fake_bin" "$state" "$release_dir/scripts/ops/lib" "$release_dir/deploy/compose"
ln -s "releases/$release_commit" "$release_root/current"

# The copied parser is real. The fixture guard deliberately does not repeat any
# release or Docker predicate: those contracts have their own guard tests.
cp -- "$root/scripts/ops/lib/dotenv.sh" "$release_dir/scripts/ops/lib/dotenv.sh"
cp -- "$root/scripts/ops/lib/release-image-manifest.sh" \
  "$release_dir/scripts/ops/lib/release-image-manifest.sh"
cat >"$release_dir/scripts/ops/lib/entitlement-amend-installed-release.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
fixture_release_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
fixture_release_commit=${fixture_release_dir##*/}
source "$(dirname "${BASH_SOURCE[0]}")/dotenv.sh"
source "$(dirname "${BASH_SOURCE[0]}")/release-image-manifest.sh"
dotenv_load "$fixture_release_dir/deploy/compose/.env"
release_image_manifest_load "$fixture_release_dir/.lagrange-release-manifest" "$fixture_release_commit"

# Test seam only: the installed-release guard is not the subject of this test.
# It intentionally contains no current-link, Docker, image, daemon, or Compose
# predicate; the extracted runbook fence supplies those predicates itself.
entitlement_amend_verify_installed_release() {
  export COMPOSE_PROJECT_NAME=lagrange-station
  export ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION="$fixture_release_commit"
  export LAGRANGE_CODE_COMMIT="$fixture_release_commit"
  export LAGRANGE_COMPOSE_FILE="$fixture_release_dir/deploy/compose/compose.yml"
  export LAGRANGE_ENV_FILE="$fixture_release_dir/deploy/compose/.env"
}
SH
cat >"$release_dir/scripts/ops/provision-entitlement.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${FAKE_DOCKER_STATE:?}/amend.calls"
SH
chmod 0755 \
  "$release_dir/scripts/ops/lib/entitlement-amend-installed-release.sh" \
  "$release_dir/scripts/ops/provision-entitlement.sh"
printf 'LAGRANGE_CODE_COMMIT=%s\n' "$release_commit" >"$release_dir/deploy/compose/.env"
printf '%s\n' 'services:' '  db-migrate:' '    image: fixture' >"$release_dir/deploy/compose/compose.yml"

source "$root/scripts/ops/lib/release-image-manifest.sh"
release_image_manifest_reset
for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
  RELEASE_IMAGE_MANIFEST_REFS[$service]=$(release_image_manifest_ref_for "$service" "$release_commit")
  RELEASE_IMAGE_MANIFEST_IDS[$service]=$fixture_id
  RELEASE_IMAGE_MANIFEST_REVISIONS[$service]=$release_commit
done
release_image_manifest_write "$release_dir/.lagrange-release-manifest" "$release_commit"

cat >"$fake_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
state=${FAKE_DOCKER_STATE:?}

next_value() {
  local kind=$1 count_file="$state/$1.calls" values_file="$state/$1.values" count value
  count=$(<"$count_file")
  count=$((count + 1))
  printf '%s\n' "$count" >"$count_file"
  value=$(sed -n "${count}p" "$values_file")
  if [ -z "$value" ]; then value=$(tail -n 1 "$values_file"); fi
  printf '%s\n' "$value"
}

command_name=${1:-}
shift || true
case "$command_name" in
  context)
    [ "${1:-}" = show ] || exit 91
    next_value context
    ;;
  info)
    [ "${1:-}" = --format ] || exit 92
    next_value daemon
    ;;
  image)
    [ "${1:-}" = inspect ] || exit 93
    printf '%s|%s\n' "$(<"$state/image_id")" "$(<"$state/revision")"
    ;;
  compose)
    files=0
    while [ "$#" -gt 0 ]; do
      if [ "$1" = -f ]; then
        shift
        [ "$#" -gt 0 ] || exit 94
        files=$((files + 1))
      fi
      shift
    done
    if [ "$files" -ge 2 ]; then
      ref=$(<"$state/override_ref")
    else
      ref=$(<"$state/base_ref")
    fi
    printf '{"services":{"db-migrate":{"image":"%s"}}}\n' "$ref"
    ;;
  *)
    exit 95
    ;;
esac
SH
chmod 0755 "$fake_bin/docker"

reset_state() {
  ln -sfn "releases/$release_commit" "$release_root/current"
  printf '0\n' >"$state/context.calls"
  printf '0\n' >"$state/daemon.calls"
  printf '%s\n' fixture-context >"$state/context.values"
  printf '%s\n' fixture-daemon >"$state/daemon.values"
  printf '%s\n' "$fixture_id" >"$state/image_id"
  printf '%s\n' "$release_commit" >"$state/revision"
  printf '%s\n' "$(release_image_manifest_ref_for db-migrate "$release_commit")" >"$state/base_ref"
  printf '%s\n' "$fixture_id" >"$state/override_ref"
  : >"$state/amend.calls"
}

fixture_environment() {
  export FAKE_DOCKER_STATE=$state
  export LAGRANGE_RELEASE_ROOT=$release_root
  export PATH="$fake_bin:$PATH"
}

expect_failure() {
  local name=$1 expected=$2
  shift 2
  if "$@" >"$tmp/$name.out" 2>&1; then
    echo "pre-amendment-image-gate-self-test: $name unexpectedly passed" >&2
    exit 1
  fi
  grep -Fq "$expected" "$tmp/$name.out" || {
    echo "pre-amendment-image-gate-self-test: $name did not report its expected failure" >&2
    exit 1
  }
}

run_default_context() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
  [ "$DOCKER_CONTEXT" = fixture-context ]
)

run_explicit_context() (
  unset DOCKER_HOST
  export DOCKER_CONTEXT=fixture-explicit
  fixture_environment
  . "$gate"
  [ "$DOCKER_CONTEXT" = fixture-explicit ]
)

run_lone_host() (
  unset DOCKER_CONTEXT
  export DOCKER_HOST=fixture-host
  fixture_environment
  . "$gate"
  [ "$DOCKER_HOST" = fixture-host ]
)

run_conflicting_overrides() (
  export DOCKER_HOST=fixture-host DOCKER_CONTEXT=fixture-context
  fixture_environment
  . "$gate"
)

run_context_environment_drift() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
  export DOCKER_CONTEXT=changed-context
  pre_amendment_image_daemon_gate_revalidate
)

run_host_environment_drift() (
  unset DOCKER_CONTEXT
  export DOCKER_HOST=fixture-host
  fixture_environment
  . "$gate"
  export DOCKER_HOST=changed-host
  pre_amendment_image_daemon_gate_revalidate
)

run_context_daemon_drift() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
)

run_current_release_drift() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
  ln -sfn releases/ffffffffffffffffffffffffffffffffffffffff "$release_root/current"
  pre_amendment_image_daemon_gate_revalidate
)

run_image_id_mismatch() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
)

run_image_revision_mismatch() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
)

run_base_tag_mismatch() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
)

run_exact_override_mismatch() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
)

run_retag_between_check_and_apply() (
  unset DOCKER_HOST DOCKER_CONTEXT
  fixture_environment
  . "$gate"
  "$current_link/scripts/ops/provision-entitlement.sh" amend --check --synthetic
  printf '%s\n' "$fixture_retagged_id" >"$state/image_id"
  pre_amendment_image_daemon_gate_revalidate
)

reset_state
run_default_context

reset_state
printf '%s\n' fixture-explicit >"$state/context.values"
run_explicit_context

reset_state
run_lone_host

reset_state
expect_failure conflicting-overrides 'DOCKER_HOST and DOCKER_CONTEXT are both set' run_conflicting_overrides

reset_state
expect_failure context-environment-drift 'Docker context changed' run_context_environment_drift

reset_state
expect_failure host-environment-drift 'Docker host changed' run_host_environment_drift

reset_state
printf '%s\n' fixture-daemon changed-daemon >"$state/daemon.values"
expect_failure daemon-drift 'Docker daemon changed' run_context_daemon_drift

reset_state
expect_failure current-release-drift 'current release changed' run_current_release_drift

reset_state
printf '%s\n' "$fixture_retagged_id" >"$state/image_id"
expect_failure image-id-mismatch 'local image ID changed' run_image_id_mismatch

reset_state
printf '%s\n' 89abcdef0123456789abcdef0123456789abcdef >"$state/revision"
expect_failure image-revision-mismatch 'local image revision changed' run_image_revision_mismatch

reset_state
printf '%s\n' wrong-db-migrate-tag >"$state/base_ref"
expect_failure base-tag-mismatch 'db-migrate base tag changed' run_base_tag_mismatch

reset_state
printf '%s\n' "$fixture_retagged_id" >"$state/override_ref"
expect_failure exact-override-mismatch 'db-migrate exact-ID override changed' run_exact_override_mismatch

reset_state
expect_failure retag-between-check-and-apply 'local image ID changed' run_retag_between_check_and_apply
grep -Fq 'amend --check --synthetic' "$state/amend.calls"

printf '%s\n' 'PRE_AMENDMENT_IMAGE_GATE_SELF_TEST: PASS (extracted gate; selection, release/image, daemon, Compose, and check-to-apply retag fixtures)'
