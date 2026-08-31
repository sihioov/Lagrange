#!/usr/bin/env bash
# Installed-release, provider-free Owner Equity V2 verifier.
#
# This wrapper owns only the immutable image and host-boundary checks.  The
# collector binary owns the typed Raw/candidate verification contract.  No
# credential file, database setting, provider endpoint, or response body is
# read or passed to the verifier.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
release_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}
mode=plan
mode_seen=0
identity_file=
candidate_file=
materializer_commit=
candidate_sha256=

usage() {
  cat <<'EOF'
Usage: scripts/ops/owner-equity-v2-verify.sh
       [--plan]
       --preflight --identity-file ABSOLUTE_PATH --candidate-file ABSOLUTE_PATH
                    --materializer-commit EXACT_40_HEX
                    --candidate-sha256 sha256:HEX64
       --check       --identity-file ABSOLUTE_PATH --candidate-file ABSOLUTE_PATH
                    --materializer-commit EXACT_40_HEX
                    --candidate-sha256 sha256:HEX64

--plan is the default and does not read the installed env/manifest, host
inputs, or Docker.  --preflight inspects only the installed exact image and
host metadata.  --check runs the installed research-worker image directly by
manifest image ID with network disabled and read-only Raw/artifact mounts.
No mode builds, starts a service, reads credentials, connects to a database,
calls a provider, or prints verifier output/body bytes.
EOF
}

blocked() {
  printf 'OWNER_EQUITY_V2_VERIFY status=blocked reason=%s\n' "$1" >&2
  exit 2
}

die() {
  printf 'OWNER_EQUITY_V2_VERIFY status=invalid reason=%s\n' "$1" >&2
  exit 1
}

commit_shape() { [[ "$1" =~ ^[0-9a-f]{40}$ ]] && [ "$1" != 0000000000000000000000000000000000000000 ]; }
hash_shape() { [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]; }

set_mode() {
  [ "$mode_seen" -eq 0 ] || die mode_repeated
  mode=${1#--}
  mode_seen=1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--preflight|--check)
      set_mode "$1"
      shift
      ;;
    --identity-file)
      [ "$mode" != plan ] || die identity_option_requires_preflight_or_check
      [ "$#" -ge 2 ] || die identity_option_missing_value
      [ -z "$identity_file" ] || die identity_option_repeated
      identity_file=$2
      shift 2
      ;;
    --candidate-file)
      [ "$mode" != plan ] || die candidate_option_requires_preflight_or_check
      [ "$#" -ge 2 ] || die candidate_option_missing_value
      [ -z "$candidate_file" ] || die candidate_option_repeated
      candidate_file=$2
      shift 2
      ;;
    --materializer-commit)
      [ "$mode" != plan ] || die commit_option_requires_preflight_or_check
      [ "$#" -ge 2 ] || die commit_option_missing_value
      [ -z "$materializer_commit" ] || die commit_option_repeated
      materializer_commit=$2
      shift 2
      ;;
    --candidate-sha256)
      [ "$mode" != plan ] || die hash_option_requires_preflight_or_check
      [ "$#" -ge 2 ] || die hash_option_missing_value
      [ -z "$candidate_sha256" ] || die hash_option_repeated
      candidate_sha256=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) die unknown_option ;;
  esac
done

if [ "$mode" = plan ]; then
  cat <<'EOF'
OWNER_EQUITY_V2_VERIFY mode=plan operation=provider_free_check
  image=installed-v2-manifest:research-worker:image-id-and-oci-revision
  raw=dedicated-host-root-read-only artifact=dedicated-host-root-read-only
  network=none user=10001:10001 rootfs=read-only caps=ALL-dropped no-new-privileges=true
  inputs=explicit-immutable-identity-and-candidate-paths candidate-hash=required
PLAN_ONLY: no protected env/manifest read, host input read, Docker invocation, credential read, or provider call made
EOF
  exit 0
fi

[ "$mode" = preflight ] || [ "$mode" = check ] || die mode_invalid
[ -n "$identity_file" ] || die identity_file_required
[ -n "$candidate_file" ] || die candidate_file_required
commit_shape "$materializer_commit" || die materializer_commit_invalid
hash_shape "$candidate_sha256" || die candidate_sha256_invalid

[ "$(id -u)" -eq 0 ] || blocked root_required
[ -f "$root/scripts/ops/lib/dotenv.sh" ] &&
  [ -f "$root/scripts/ops/lib/release-image-manifest.sh" ] ||
  blocked installed_support_missing
source "$root/scripts/ops/lib/dotenv.sh"
source "$root/scripts/ops/lib/release-image-manifest.sh"

release_image_manifest_require_absolute_path "$release_root" release-root ||
  blocked release_root_invalid
[ -d "$release_root" ] && [ ! -L "$release_root" ] || blocked release_root_missing
release_image_manifest_trusted_directory "$release_root" release-root 2>/dev/null ||
  blocked release_root_untrusted

env_file=$root/deploy/compose/.env
[ -f "$env_file" ] && [ ! -L "$env_file" ] || blocked installed_env_missing
release_image_manifest_trusted_file "$env_file" installed-env 2>/dev/null ||
  blocked installed_env_untrusted
dotenv_load "$env_file" || blocked installed_env_invalid
dotenv_validate_shell_overrides || blocked shell_override_mismatch

release_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
commit_shape "$release_commit" || blocked release_commit_invalid
expected_root=$release_root/releases/$release_commit
[ "$root" = "$expected_root" ] || blocked not_current_release
current_link=$release_root/current
[ -L "$current_link" ] || blocked current_link_missing
[ "$(readlink -- "$current_link" 2>/dev/null)" = "releases/$release_commit" ] ||
  blocked current_link_mismatch

manifest=$root/.lagrange-release-manifest
release_image_manifest_trusted_file "$manifest" installed-manifest 2>/dev/null ||
  blocked installed_manifest_untrusted
release_image_manifest_load "$manifest" "$release_commit" 2>/dev/null ||
  blocked installed_manifest_invalid
image_id=${RELEASE_IMAGE_MANIFEST_IDS[research-worker]:-}
image_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[research-worker]:-}
release_image_manifest_is_image_id "$image_id" || blocked research_worker_image_id_missing
commit_shape "$image_revision" || blocked research_worker_image_revision_missing

data_root=$(dotenv_get LAGRANGE_DATA_DIR)
[ -n "$data_root" ] || blocked data_root_missing
release_image_manifest_require_absolute_path "$data_root" data-root || blocked data_root_invalid
release_image_manifest_trusted_directory "$data_root" data-root 2>/dev/null ||
  blocked data_root_untrusted
raw_root=$data_root/raw
artifact_root=$data_root/owner-equity-v2-artifacts

secure_directory() {
  local path=$1 metadata
  [ -d "$path" ] && [ ! -L "$path" ] || blocked host_directory_missing
  metadata=$(stat -c '%u:%g:%a' -- "$path" 2>/dev/null) || blocked host_metadata_missing
  [ "$metadata" = '10001:10001:750' ] || blocked host_directory_ownership
  [ "$(realpath -e -- "$path" 2>/dev/null)" = "$path" ] ||
    blocked host_directory_not_canonical
}

secure_path_components() {
  local path=$1 current= component
  local -a components=()
  IFS=/ read -r -a components <<<"${path#/}"
  current=
  for component in "${components[@]}"; do
    [ -n "$component" ] || continue
    current="${current}/${component}"
    [ ! -L "$current" ] || blocked host_path_symlinked
  done
}

secure_input_file() {
  local path=$1 canonical metadata
  [ -f "$path" ] && [ ! -L "$path" ] || blocked input_not_regular
  secure_path_components "$path"
  canonical=$(realpath -e -- "$path" 2>/dev/null) || blocked input_not_canonical
  [ "$canonical" = "$path" ] || blocked input_not_canonical
  case "$canonical" in
    "$artifact_canonical"/*) ;;
    *) blocked input_outside_artifact_root ;;
  esac
  metadata=$(stat -c '%u:%a' -- "$canonical" 2>/dev/null) || blocked input_metadata_missing
  [ "${metadata%%:*}" = 10001 ] || blocked input_owner_invalid
  (( (8#${metadata#*:} & 0077) == 0 )) || blocked input_permissions_invalid
}

secure_directory "$raw_root"
secure_directory "$artifact_root"
raw_canonical=$(realpath -e -- "$raw_root")
artifact_canonical=$(realpath -e -- "$artifact_root")
[ "$raw_canonical" != "$artifact_canonical" ] || blocked raw_artifact_alias
[[ "$raw_canonical" != "$artifact_canonical"/* ]] || blocked raw_artifact_alias
[[ "$artifact_canonical" != "$raw_canonical"/* ]] || blocked raw_artifact_alias
secure_input_file "$identity_file"
secure_input_file "$candidate_file"

command -v docker >/dev/null 2>&1 || blocked docker_unavailable
inspected=$(docker image inspect \
  --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
  "$image_id" 2>/dev/null) || blocked research_worker_image_unavailable
[ "$inspected" = "$image_id|$image_revision" ] || blocked research_worker_image_identity_mismatch

if [ "$mode" = preflight ]; then
  printf 'OWNER_EQUITY_V2_VERIFY status=ok mode=preflight image=research-worker-manifest-bound network=none mounts=read-only\n'
  exit 0
fi

identity_relative=${identity_file#"$artifact_canonical"/}
candidate_relative=${candidate_file#"$artifact_canonical"/}
[ -n "$identity_relative" ] && [ -n "$candidate_relative" ] || blocked input_path_invalid
docker run \
  --pull=never \
  --rm \
  --init \
  --read-only \
  --network none \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --user 10001:10001 \
  --mount "type=bind,source=$raw_root,destination=/data/raw,readonly" \
  --mount "type=bind,source=$artifact_root,destination=/data/artifacts,readonly" \
  --entrypoint /usr/local/bin/owner-equity-v2-check \
  "$image_id" \
  /data/raw \
  "/data/artifacts/$identity_relative" \
  "$materializer_commit" \
  "/data/artifacts/$candidate_relative" \
  "$candidate_sha256" \
  >/dev/null 2>/dev/null || blocked verifier_failed

printf 'OWNER_EQUITY_V2_VERIFY status=ok mode=check image=research-worker-manifest-bound network=none mounts=read-only provider=none\n'
