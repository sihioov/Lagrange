#!/usr/bin/env bash
# Immutable installed-release identity gate for the approved entitlement
# amendment.  This is deliberately narrower than the general release wrapper:
# it only establishes the identity and the exact Compose inputs permitted for
# the amendment's db-migrate invocation.  It never invokes Docker or Git.

_entitlement_amend_guard_source=${BASH_SOURCE[0]}
_entitlement_amend_guard_dir=$(cd -P "$(dirname -- "$_entitlement_amend_guard_source")" && pwd -P)
ENTITLEMENT_AMEND_GUARD_SOURCE_PATH="$_entitlement_amend_guard_dir/$(basename -- "$_entitlement_amend_guard_source")"
unset _entitlement_amend_guard_source _entitlement_amend_guard_dir

# These are the same parser and strict V2 manifest primitives used by the
# release wrapper.  The guard validates their installed copies before it
# accepts any identity derived through them.
source "$(dirname -- "$ENTITLEMENT_AMEND_GUARD_SOURCE_PATH")/dotenv.sh"
source "$(dirname -- "$ENTITLEMENT_AMEND_GUARD_SOURCE_PATH")/release-image-manifest.sh"

ENTITLEMENT_AMEND_RELEASE_CONTEXT_ERROR=
ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION=

entitlement_amend_release_context_fail() {
  ENTITLEMENT_AMEND_RELEASE_CONTEXT_ERROR=$1
  return 1
}

entitlement_amend_release_context_regular_file() {
  local path=$1 label=$2 metadata uid mode mode_value
  [ -f "$path" ] && [ ! -L "$path" ] || {
    entitlement_amend_release_context_fail "$label"
    return 1
  }
  metadata=$(stat -c '%u:%a' -- "$path") || {
    entitlement_amend_release_context_fail "$label"
    return 1
  }
  uid=${metadata%%:*}
  mode=${metadata#*:}
  [ "$uid" = 0 ] || {
    entitlement_amend_release_context_fail "$label"
    return 1
  }
  mode_value=$((8#$mode))
  (( (mode_value & 0022) == 0 )) || {
    entitlement_amend_release_context_fail "$label"
    return 1
  }
}

entitlement_amend_release_context_resolve_file() {
  local path=$1 directory
  [ -f "$path" ] && [ ! -L "$path" ] || return 1
  directory=$(cd -P "$(dirname -- "$path")" && pwd -P) || return 1
  printf '%s/%s' "$directory" "$(basename -- "$path")"
}

# Verify the caller-claimed revision against the trusted installed release and
# bind db.sh to that release's exact Compose/env files.  The claimed value is
# never an authority: it is accepted only after the protected env and V2
# manifest independently establish the same commit.
entitlement_amend_verify_installed_release() {
  local claimed_revision=$1 entrypoint_path=$2
  local release_root configured_release_root release_commit release_dir current_link expected_entry
  local expected_compose expected_env expected_manifest entrypoint_resolved
  local entrypoint_release_dir entrypoint_commit expected_helper expected_guard
  local expected_db_helper expected_dotenv expected_manifest_lib trusted_dir
  local expected_compose_project=lagrange-station

  ENTITLEMENT_AMEND_RELEASE_CONTEXT_ERROR=
  ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION=

  # The physical entrypoint establishes the release root. An environment value
  # may only agree with it; it never selects a different release tree for a
  # check/apply database operation.
  entrypoint_resolved=$(entitlement_amend_release_context_resolve_file "$entrypoint_path") || {
    entitlement_amend_release_context_fail entrypoint_untrusted
    return 1
  }
  case "$entrypoint_resolved" in
    */releases/*/scripts/ops/provision-entitlement.sh) ;;
    *)
      entitlement_amend_release_context_fail entrypoint_root_mismatch
      return 1
      ;;
  esac
  entrypoint_release_dir=${entrypoint_resolved%/scripts/ops/provision-entitlement.sh}
  entrypoint_commit=${entrypoint_release_dir##*/}
  release_image_manifest_is_commit "$entrypoint_commit" || {
    entitlement_amend_release_context_fail entrypoint_commit_invalid
    return 1
  }
  release_root=${entrypoint_release_dir%/releases/$entrypoint_commit}
  [ "$release_root/releases/$entrypoint_commit" = "$entrypoint_release_dir" ] || {
    entitlement_amend_release_context_fail entrypoint_root_mismatch
    return 1
  }
  configured_release_root=${LAGRANGE_RELEASE_ROOT:-}
  if [ -n "$configured_release_root" ] && [ "$configured_release_root" != "$release_root" ]; then
    entitlement_amend_release_context_fail release_root_mismatch
    return 1
  fi
  if ! release_image_manifest_require_absolute_path "$release_root" release-root; then
    entitlement_amend_release_context_fail release_root_invalid
    return 1
  fi
  if [ ! -d "$release_root" ] || [ -L "$release_root" ]; then
    entitlement_amend_release_context_fail release_root_untrusted
    return 1
  fi
  if ! release_image_manifest_trusted_directory "$release_root" release-root; then
    entitlement_amend_release_context_fail release_root_untrusted
    return 1
  fi

  # The protected env is under the physically resolved release tree. Its
  # commit must then agree with the physical entrypoint and strict manifest.
  expected_env=$entrypoint_release_dir/deploy/compose/.env
  if ! release_image_manifest_trusted_file "$expected_env" installed-release-env; then
    entitlement_amend_release_context_fail release_env_untrusted
    return 1
  fi
  if ! dotenv_load "$expected_env" || ! dotenv_validate_shell_overrides; then
    entitlement_amend_release_context_fail release_env_invalid
    return 1
  fi
  release_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
  release_image_manifest_is_commit "$release_commit" || {
    entitlement_amend_release_context_fail release_commit_invalid
    return 1
  }
  [ "$release_commit" = "$entrypoint_commit" ] || {
    entitlement_amend_release_context_fail release_commit_mismatch
    return 1
  }

  release_dir=$release_root/releases/$release_commit
  if ! release_image_manifest_trusted_directory "$release_dir" release-dir; then
    entitlement_amend_release_context_fail release_dir_untrusted
    return 1
  fi
  expected_entry=$release_dir/scripts/ops/provision-entitlement.sh
  expected_compose=$release_dir/deploy/compose/compose.yml
  expected_env=$release_dir/deploy/compose/.env
  expected_manifest=$release_dir/.lagrange-release-manifest
  expected_helper=$release_dir/scripts/ops/lib/entitlement-amend.sh
  expected_guard=$release_dir/scripts/ops/lib/entitlement-amend-installed-release.sh
  expected_db_helper=$release_dir/scripts/ops/lib/db.sh
  expected_dotenv=$release_dir/scripts/ops/lib/dotenv.sh
  expected_manifest_lib=$release_dir/scripts/ops/lib/release-image-manifest.sh

  [ "$entrypoint_resolved" = "$expected_entry" ] || {
    entitlement_amend_release_context_fail entrypoint_root_mismatch
    return 1
  }
  for trusted_dir in "$release_dir/scripts" "$release_dir/scripts/ops" \
      "$release_dir/scripts/ops/lib" "$release_dir/deploy" \
      "$release_dir/deploy/compose"; do
    if ! release_image_manifest_trusted_directory "$trusted_dir" release-dir; then
      entitlement_amend_release_context_fail release_dir_untrusted
      return 1
    fi
  done
  for expected_file in "$expected_entry" "$expected_helper" "$expected_guard" \
      "$expected_db_helper" "$expected_dotenv" "$expected_manifest_lib" "$expected_compose"; do
    entitlement_amend_release_context_regular_file "$expected_file" release_file_untrusted || return 1
  done
  [ "$ENTITLEMENT_AMEND_GUARD_SOURCE_PATH" = "$expected_guard" ] || {
    entitlement_amend_release_context_fail guard_source_mismatch
    return 1
  }
  if [ -n "${AMEND_ENTITLEMENT_HELPER_SOURCE_PATH:-}" ] &&
     [ "$AMEND_ENTITLEMENT_HELPER_SOURCE_PATH" != "$expected_helper" ]; then
    entitlement_amend_release_context_fail amendment_source_mismatch
    return 1
  fi

  current_link=$release_root/current
  [ -L "$current_link" ] &&
    [ "$(readlink -- "$current_link")" = "releases/$release_commit" ] || {
    entitlement_amend_release_context_fail current_link_mismatch
    return 1
  }
  if ! release_image_manifest_trusted_file "$expected_manifest" installed-release-manifest; then
    entitlement_amend_release_context_fail manifest_untrusted
    return 1
  fi
  if ! release_image_manifest_load "$expected_manifest" "$release_commit"; then
    entitlement_amend_release_context_fail manifest_invalid
    return 1
  fi

  [ "$claimed_revision" = "$release_commit" ] || {
    entitlement_amend_release_context_fail claimed_revision_mismatch
    return 1
  }
  if [ -n "${LAGRANGE_COMPOSE_FILE:-}" ] &&
     [ "$LAGRANGE_COMPOSE_FILE" != "$expected_compose" ]; then
    entitlement_amend_release_context_fail compose_input_mismatch
    return 1
  fi
  if [ -n "${LAGRANGE_ENV_FILE:-}" ] &&
     [ "$LAGRANGE_ENV_FILE" != "$expected_env" ]; then
    entitlement_amend_release_context_fail env_input_mismatch
    return 1
  fi
  # db.sh invokes `docker compose` without -p.  Do not let a caller select a
  # different project/network (and therefore a different postgres service)
  # while retaining this release's trusted Compose file.
  if [ -n "${COMPOSE_PROJECT_NAME:-}" ] &&
     [ "$COMPOSE_PROJECT_NAME" != "$expected_compose_project" ]; then
    entitlement_amend_release_context_fail compose_project_mismatch
    return 1
  fi

  # db_init/db_psql consume these exact paths.  Setting them only after every
  # trust check prevents a caller from directing the operator transaction at a
  # separate Compose graph, env file, project/network, or image tag. db.sh
  # otherwise supplies a placeholder LAGRANGE_CODE_COMMIT just to satisfy
  # Compose interpolation; the verified commit must win for this operation.
  export LAGRANGE_COMPOSE_FILE=$expected_compose
  export LAGRANGE_ENV_FILE=$expected_env
  export COMPOSE_PROJECT_NAME=$expected_compose_project
  export LAGRANGE_CODE_COMMIT=$release_commit
  ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION=$release_commit
}
