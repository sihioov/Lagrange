#!/usr/bin/env bash
# Root-only installed-release seam for the provider-free historical price beta
# artifact.  The Rust CLI owns the artifact bytes and its exact grammar; this
# wrapper owns only the installed image/host isolation boundary.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
release_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}
mode=plan
mode_seen=0
stage5_manifest_sha256=
action_manifest_sha256=
candidate_content_sha256=

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-historical-price-beta-artifact.sh
       [--plan|--preflight]
       [--materialize --stage5-manifest-sha256 HASH --action-manifest-sha256 HASH]
       [--check --candidate-content-sha256 HASH]
       [--approval-check]

The default --plan is a static description and does not read the installed
environment/manifest or invoke Docker.  The other modes are root-only and
accept only the installed current release.  They never build, start a daemon,
publish, register, mark READY, access Curated, inject an environment file, or
call KIS/DB/provider surfaces.
EOF
}

blocked() {
  printf 'HISTORICAL_PRICE_BETA_OPS status=blocked reason=%s\n' "$1" >&2
  exit 2
}

die() {
  printf 'HISTORICAL_PRICE_BETA_OPS status=blocked reason=%s\n' "$1" >&2
  exit 1
}

hash_shape() {
  [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--preflight|--materialize|--check|--approval-check)
      [ "$mode_seen" -eq 0 ] || die mode_repeated
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --stage5-manifest-sha256)
      [ "$mode" = materialize ] || die stage5_option_not_allowed
      [ "$#" -ge 2 ] || die stage5_option_missing_value
      [ -z "$stage5_manifest_sha256" ] || die stage5_option_repeated
      stage5_manifest_sha256=$2
      shift 2
      ;;
    --action-manifest-sha256)
      [ "$mode" = materialize ] || die action_option_not_allowed
      [ "$#" -ge 2 ] || die action_option_missing_value
      [ -z "$action_manifest_sha256" ] || die action_option_repeated
      action_manifest_sha256=$2
      shift 2
      ;;
    --candidate-content-sha256)
      [ "$mode" = check ] || die candidate_option_not_allowed
      [ "$#" -ge 2 ] || die candidate_option_missing_value
      [ -z "$candidate_content_sha256" ] || die candidate_option_repeated
      candidate_content_sha256=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die unknown_option
      ;;
  esac
done

if [ "$mode" = plan ]; then
  cat <<'EOF'
HISTORICAL_PRICE_BETA_OPS mode=plan operation=materialize_or_check
  image=installed-v2-manifest:research-worker:image-id-and-oci-revision
  materialize=Raw-read-only artifact-dedicated-root-read-write network-none uid-gid-10001:10001
  check=Raw-unmounted artifact-dedicated-root-read-only network-none uid-gid-10001:10001
  approval-check=Raw-unmounted artifact-dedicated-root-read-only network-none uid-gid-10001:10001 embedded-registry
  container=read-only-rootfs cap-drop-ALL no-new-privileges fixed-entrypoint no-secrets no-db-env
PLAN_ONLY: no protected env/manifest read, host path read, Docker invocation, or artifact/Raw write made
EOF
  exit 0
fi

[ "$(id -u)" -eq 0 ] || blocked root_required

case "$mode" in
  preflight)
    [ -z "$stage5_manifest_sha256" ] && [ -z "$action_manifest_sha256" ] &&
      [ -z "$candidate_content_sha256" ] || die operation_options_not_allowed
    ;;
  materialize)
    hash_shape "$stage5_manifest_sha256" || die invalid_stage5_manifest_sha256
    hash_shape "$action_manifest_sha256" || die invalid_action_manifest_sha256
    ;;
  check)
    hash_shape "$candidate_content_sha256" || die invalid_candidate_content_sha256
    ;;
  approval-check)
    [ -z "$candidate_content_sha256" ] || die operation_options_not_allowed
    ;;
esac

# The installed env and manifest are protected root-owned files.  This parser
# is deliberately non-evaluating; a value from .env can never become shell
# code or an environment injection into the artifact container.
[ -f "$root/scripts/ops/lib/dotenv.sh" ] && [ -f "$root/scripts/ops/lib/release-image-manifest.sh" ] ||
  blocked installed_support_missing
source "$root/scripts/ops/lib/dotenv.sh"
source "$root/scripts/ops/lib/release-image-manifest.sh"

release_image_manifest_require_absolute_path "$release_root" release-root || blocked release_root_invalid
[ -d "$release_root" ] && [ ! -L "$release_root" ] || blocked release_root_missing
release_image_manifest_trusted_directory "$release_root" release-root 2>/dev/null || blocked release_root_untrusted

env_file=$root/deploy/compose/.env
[ -f "$env_file" ] && [ ! -L "$env_file" ] || blocked installed_env_missing
release_image_manifest_trusted_file "$env_file" installed-env 2>/dev/null || blocked installed_env_untrusted
dotenv_load "$env_file" || blocked installed_env_invalid
dotenv_validate_shell_overrides || blocked shell_override_mismatch

release_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
release_image_manifest_is_commit "$release_commit" || blocked release_commit_invalid
expected_root=$release_root/releases/$release_commit
[ "$root" = "$expected_root" ] || blocked not_current_release
current_link=$release_root/current
[ -L "$current_link" ] || blocked current_link_missing
[ "$(readlink -- "$current_link" 2>/dev/null)" = "releases/$release_commit" ] || blocked current_link_mismatch

manifest=$root/.lagrange-release-manifest
release_image_manifest_trusted_file "$manifest" installed-manifest 2>/dev/null || blocked installed_manifest_untrusted
release_image_manifest_load "$manifest" "$release_commit" 2>/dev/null || blocked installed_manifest_invalid

image_id=${RELEASE_IMAGE_MANIFEST_IDS[research-worker]:-}
image_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[research-worker]:-}
release_image_manifest_is_image_id "$image_id" || blocked research_worker_image_id_missing
release_image_manifest_is_commit "$image_revision" || blocked research_worker_image_revision_missing

data_root=$(dotenv_get LAGRANGE_DATA_DIR)
artifacts_root=$(dotenv_get LAGRANGE_ARTIFACTS_DIR)
[ -n "$artifacts_root" ] || artifacts_root=$data_root/artifacts
release_image_manifest_require_absolute_path "$data_root" data-root || blocked data_root_invalid
release_image_manifest_require_absolute_path "$artifacts_root" artifacts-root || blocked artifacts_root_invalid
case "$data_root:$artifacts_root" in
  *$'\n'*|*$'\r'*|*,*) blocked host_path_invalid ;;
esac

raw_root=$data_root/raw
curated_root=$data_root/curated
artifact_root=$artifacts_root/historical-price-beta-root

require_directory() {
  local path=$1
  [ -d "$path" ] && [ ! -L "$path" ] || blocked host_directory_missing
  [ "$(stat -c '%u:%g:%a' -- "$path" 2>/dev/null)" = '10001:10001:750' ] ||
    blocked host_directory_ownership
}

canonical_directory() {
  realpath -e -- "$1" 2>/dev/null || blocked host_directory_unresolvable
}

path_below_or_equal() {
  local candidate=$1 boundary=$2
  [ "$candidate" = "$boundary" ] || [[ "$candidate" == "$boundary"/* ]]
}

host_separation_gate() {
  local data_canonical raw_canonical curated_canonical artifact_canonical
  local identity ancestor
  data_canonical=$(canonical_directory "$data_root")
  raw_canonical=$(canonical_directory "$raw_root")
  curated_canonical=$(canonical_directory "$curated_root")
  artifact_canonical=$(canonical_directory "$artifact_root")

  path_below_or_equal "$artifact_canonical" "$raw_canonical" &&
    blocked artifact_root_not_separate
  path_below_or_equal "$artifact_canonical" "$curated_canonical" &&
    blocked artifact_root_not_separate
  [ "$artifact_canonical" != "$data_canonical" ] || blocked artifact_root_not_separate

  identity=$(stat -c '%d:%i' -- "$artifact_canonical" 2>/dev/null) || blocked host_identity_unavailable
  for ancestor in "$data_canonical" "$raw_canonical" "$curated_canonical"; do
    [ "$identity" != "$(stat -c '%d:%i' -- "$ancestor" 2>/dev/null)" ] ||
      blocked artifact_root_not_separate
  done

  # Bind mounts make /data/raw and /artifact-root look unrelated in the
  # container.  Check every host artifact ancestor against Raw/Curated
  # identities before Docker is reached.
  ancestor=$artifact_canonical
  while [ "$ancestor" != / ]; do
    for identity in "$raw_canonical" "$curated_canonical"; do
      [ "$(stat -c '%d:%i' -- "$ancestor" 2>/dev/null)" != "$(stat -c '%d:%i' -- "$identity" 2>/dev/null)" ] ||
        blocked artifact_root_not_separate
    done
    ancestor=${ancestor%/*}
    [ -n "$ancestor" ] || ancestor=/
  done
}

require_directory "$artifact_root"
if [ "$mode" = materialize ]; then
  require_directory "$raw_root"
  host_separation_gate
elif [ "$mode" = preflight ]; then
  require_directory "$raw_root"
  host_separation_gate
elif [ "$mode" = check ]; then
  # Check does not bind Raw into the container, but the host path still must
  # be proven separate so a symlink/bind alias cannot turn the artifact-only
  # read into an accidental Raw read.
  require_directory "$raw_root"
  host_separation_gate
elif [ "$mode" = approval-check ]; then
  # Approval reads only the dedicated artifact mount, but the host identity
  # fence still prevents an artifact/Raw/Curated alias from being presented to
  # the checker through its independent bind mount.
  require_directory "$raw_root"
  host_separation_gate
fi

command -v docker >/dev/null 2>&1 || blocked docker_unavailable

verify_image_identity() {
  local inspected actual_id actual_revision
  inspected=$(docker image inspect \
    --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$image_id" 2>/dev/null) || blocked research_worker_image_unavailable
  case "$inspected" in
    "$image_id|$image_revision") ;;
    *) blocked research_worker_image_identity_mismatch ;;
  esac
  actual_id=${inspected%%|*}
  actual_revision=${inspected#*|}
  release_image_manifest_is_image_id "$actual_id" || blocked research_worker_image_id_invalid
  release_image_manifest_is_commit "$actual_revision" || blocked research_worker_image_revision_invalid
}

if [ "$mode" = preflight ]; then
  # No container lifecycle is allowed in preflight.  The image is inspected by
  # its manifest ID and revision, but no Raw/artifact mount is opened by
  # Docker; the host checks above are read-only.
  verify_image_identity
  printf 'HISTORICAL_PRICE_BETA_OPS status=ok mode=preflight image=research-worker-manifest-bound\n'
  exit 0
fi

run_artifact_container() {
  local output success_line line success_count=0
  local entrypoint=/usr/local/bin/kis-historical-price-beta-artifact
  if [ "$mode" = approval-check ]; then
    entrypoint=/usr/local/bin/kis-historical-price-beta-approval-check
  fi
  local -a docker_args=(
    run
    --pull=never
    --rm
    --init
    --read-only
    --network none
    --cap-drop ALL
    --security-opt no-new-privileges:true
    --user 10001:10001
    --entrypoint "$entrypoint"
  )

  if [ "$mode" = materialize ]; then
    docker_args+=(
      --mount "type=bind,source=$raw_root,destination=/data/raw,readonly"
      --mount "type=bind,source=$artifact_root,destination=/artifact-root"
      "$image_id"
      materialize
      --raw-root /data
      --artifact-root /artifact-root
      --stage5-manifest-sha256 "$stage5_manifest_sha256"
      --action-manifest-sha256 "$action_manifest_sha256"
    )
  elif [ "$mode" = check ]; then
    docker_args+=(
      --mount "type=bind,source=$artifact_root,destination=/artifact-root,readonly"
      "$image_id"
      check
      --artifact-root /artifact-root
      --candidate-content-sha256 "$candidate_content_sha256"
    )
  else
    docker_args+=(
      --mount "type=bind,source=$artifact_root,destination=/artifact-root,readonly"
      "$image_id"
      check
      --artifact-root /artifact-root
    )
  fi

  # Inspect immediately before the direct image-ID run.  The ID itself is the
  # image selector, so a mutable Compose tag cannot be substituted between the
  # release gate and execution.
  verify_image_identity
  output=$(docker "${docker_args[@]}" 2>/dev/null) || blocked artifact_container_failed

  while IFS= read -r line; do
    if [ "$mode" = approval-check ]; then
      case "$line" in
        HISTORICAL_PRICE_BETA_APPROVAL\ status=ok\ operation=*)
          success_line=$line
          success_count=$((success_count + 1))
          ;;
      esac
    else
      case "$line" in
        HISTORICAL_PRICE_BETA_ARTIFACT\ status=ok\ operation=*)
          success_line=$line
          success_count=$((success_count + 1))
          ;;
      esac
    fi
  done <<<"$output"
  [ "$success_count" -eq 1 ] || blocked artifact_success_output_invalid

  if [ "$mode" = approval-check ]; then
    [[ "$success_line" =~ ^HISTORICAL_PRICE_BETA_APPROVAL\ status=ok\ operation=check\ approval_registry_sha256=sha256:[0-9a-f]{64}\ approval_status=APPROVED\ audience=OWNER_ONLY\ vendor_snapshot=true\ strict_pit=false\ capability=PRICE_RETURN_ONLY\ materialization_status=MATERIALIZED\ registration_status=UNREGISTERED\ publication_status=NOT_PUBLISHED\ instrument_count=11\ session_count=1608\ bar_count=17688$ ]] ||
      blocked artifact_success_output_invalid
  elif [ "$mode" = materialize ]; then
    [[ "$success_line" =~ ^HISTORICAL_PRICE_BETA_ARTIFACT\ status=ok\ operation=materialize\ candidate_content_sha256=sha256:[0-9a-f]{64}\ stage5_manifest_sha256=$stage5_manifest_sha256\ action_manifest_sha256=$action_manifest_sha256\ instrument_count=11\ session_count=1608\ bar_count=17688\ raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS\ audience=OWNER_ONLY\ vendor_snapshot=true\ strict_pit=false\ capability=PRICE_RETURN_ONLY\ materialization_status=MATERIALIZED\ registration_status=UNREGISTERED\ publication_status=NOT_PUBLISHED$ ]] ||
      blocked artifact_success_output_invalid
  else
    expected_line="HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256=$candidate_content_sha256 instrument_count=11 session_count=1608 bar_count=17688 raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED"
    [ "$success_line" = "$expected_line" ] || blocked artifact_success_output_invalid
  fi
  printf '%s\n' "$success_line"
}

run_artifact_container
