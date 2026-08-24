#!/usr/bin/env bash
# Build the eleven owner-beta serving images without starting a container. The
# default is a read-only plan; --preflight only expands Compose; --apply is the
# only mode that builds and writes a strict V2 host-local image manifest.
#
# This is Bash/GNU/Linux tooling. It does not invoke the production validator:
# image prebuild needs no provider credentials, dataset pins, database state,
# runtime secrets, or container lifecycle action.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/release-image-manifest.sh"

compose_file=$root/deploy/compose/compose.yml
env_file=$root/deploy/compose/.env
manifest_file=${LAGRANGE_IMAGE_MANIFEST_FILE:-}
mode=plan
mode_seen=0

# This manifest scope is intentionally exact. `research-range-raw` is the
# separately operator-gated historical-capture profile, not owner-beta serving;
# live-node/live remains forbidden; upstream postgres/reverse-proxy are already
# content-pinned and are not locally-built image records.
local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")

usage() {
  cat <<'EOF'
Usage: scripts/ops/build-production-images.sh [--plan|--preflight|--apply]
       [--compose-file ABSOLUTE_PATH] [--env-file ABSOLUTE_PATH]
       [--manifest-file ABSOLUTE_PATH]

Modes:
  --plan       Validate local inputs and print the exact build plan (default).
  --preflight  Read-only Docker/Compose availability and config check. It does
               not build or start a container.
  --apply      Root-only Compose image build with --pull=false. It requires a
               new --manifest-file and never runs up, run, restart, start,
               migration, database, provider, or secret-provisioning work.

LAGRANGE_CODE_COMMIT must already be an exact lowercase 40-hex Git commit and
must equal the clean build-context HEAD. The script passes it only process-
locally to Compose and never edits the env file. Docker may fetch pinned base
or language dependencies if its cache is incomplete; no provider/API credential
is read or used.

The V2 manifest has one canonical record for each of the eleven local serving
images: exact configured commit tag, exact Docker local image_id (`sha256:...`),
and OCI revision. This is host-local single-platform provenance, not a
RepoDigest, registry digest, or multi-architecture claim.
EOF
}

die() { echo "build-production-images: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--preflight|--apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --preflight, or --apply'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --compose-file|--env-file|--manifest-file)
      [ "$#" -ge 2 ] || die "$1 needs an absolute path"
      case "$1" in
        --compose-file) compose_file=$2 ;;
        --env-file) env_file=$2 ;;
        --manifest-file) manifest_file=$2 ;;
      esac
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die 'unknown option (use --help)' ;;
  esac
done

if [ "$mode" = apply ]; then
  [ "$(id -u)" -eq 0 ] ||
    die '--apply must run as root; use --plan or --preflight for read-only checks'
  [ -n "$manifest_file" ] ||
    die '--apply requires --manifest-file for the strict V2 release manifest'
fi

safe_path() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in
    *$'\n'*|*$'\r'*|*'//'*) die "$label is not a canonical absolute path" ;;
    */../*|*/..|*/./*|*/.) die "$label must not contain dot path components" ;;
    */) die "$label must not have a trailing slash" ;;
  esac
  case "$path" in
    /|/etc|/opt|/usr|/usr/local|/var|/var/lib|/tmp|/run)
      die "$label is too broad: $path"
      ;;
  esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

check_inputs() {
  local commit=${LAGRANGE_CODE_COMMIT:-} head status
  safe_path "$compose_file" compose-file
  safe_path "$env_file" env-file
  [ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
    die 'Compose file must be a regular non-symlink file'
  [ -r "$compose_file" ] || die 'Compose file is not readable'
  [ -f "$env_file" ] && [ ! -L "$env_file" ] ||
    die 'Compose env file must be a regular non-symlink file'
  [ -r "$env_file" ] || die 'Compose env file is not readable'
  release_image_manifest_is_commit "$commit" ||
    die 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters and not all zeroes'
  command -v git >/dev/null 2>&1 || die 'git is required to verify build provenance'
  head=$(git -c "safe.directory=$root" -C "$root" \
    rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
    die 'build root is not a Git worktree with a commit'
  [ "$head" = "$commit" ] ||
    die 'LAGRANGE_CODE_COMMIT does not match the build root HEAD'
  status=$(git -c "safe.directory=$root" -C "$root" \
    status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
    die 'cannot inspect build root worktree status'
  while IFS= read -r status_line; do
    [ -z "$status_line" ] && continue
    # This operator-supplied workbook is the sole untracked exception and is
    # never deployed or added to a Docker context. Every other change fails.
    [ "$status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] ||
      die 'build root worktree is not clean (tracked or unapproved untracked changes present)'
  done <<<"$status"
  if [ -n "$manifest_file" ]; then
    safe_path "$manifest_file" manifest-file
  fi
}

inspect_built_images() {
  local service image_ref inspected image_id revision
  command -v docker >/dev/null 2>&1 || die 'docker is not installed'
  release_image_manifest_reset
  for service in "${local_image_services[@]}"; do
    image_ref=$(release_image_manifest_ref_for "$service" "$LAGRANGE_CODE_COMMIT") ||
      die "cannot derive configured image reference: $service"
    inspected=$(docker image inspect \
      --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$image_ref") || die "cannot inspect built image: $service"
    case "$inspected" in
      *'|'*) ;;
      *) die "built image inspection omitted its revision label: $service" ;;
    esac
    image_id=${inspected%%|*}
    revision=${inspected#*|}
    release_image_manifest_is_image_id "$image_id" ||
      die "built image_id is not an exact local Docker image ID: $service"
    release_image_manifest_is_commit "$revision" ||
      die "built image revision label is missing or invalid: $service"
    [ "$revision" = "$LAGRANGE_CODE_COMMIT" ] ||
      die "built image revision label does not match source commit: $service"
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=$image_ref
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=$image_id
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$revision
  done
}

write_manifest() {
  local temporary parent
  safe_path "$manifest_file" manifest-file
  [ ! -e "$manifest_file" ] && [ ! -L "$manifest_file" ] ||
    die 'manifest-file already exists; refusing to overwrite it'
  parent=$(dirname -- "$manifest_file")
  [ -d "$parent" ] && [ ! -L "$parent" ] ||
    die 'manifest-file parent directory is missing or a symlink'
  temporary=$(mktemp -- "$parent/.lagrange-release-manifest.XXXXXX") ||
    die 'cannot create manifest staging file'
  chmod 0600 -- "$temporary"
  if ! release_image_manifest_write "$temporary" "$LAGRANGE_CODE_COMMIT"; then
    rm -f -- "$temporary"
    die "cannot write strict V2 manifest: $RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  if ! release_image_manifest_load "$temporary" "$LAGRANGE_CODE_COMMIT"; then
    rm -f -- "$temporary"
    die "written strict V2 manifest did not revalidate: $RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  # `ln` is no-clobber for a new name in this same parent directory. This
  # closes the check-to-publish race without ever replacing another manifest.
  ln -- "$temporary" "$manifest_file" || {
    rm -f -- "$temporary"
    die 'manifest-file already exists; refusing to overwrite it'
  }
  rm -f -- "$temporary"
  [ "$(stat -c '%a' -- "$manifest_file")" = 600 ] ||
    die 'published manifest does not have mode 0600'
}

print_plan() {
  echo 'PRODUCTION_IMAGE_BUILD_PLAN mode=plan'
  echo "  compose_file=$compose_file"
  echo "  env_file=$env_file"
  echo "  build_root=$root (clean HEAD provenance verified)"
  echo '  LAGRANGE_CODE_COMMIT=validated-process-value'
  echo "  services=${local_image_services[*]}"
  echo '  excluded=postgres reverse-proxy (upstream content-pinned); research-range-raw (operator-gated historical capture); live-node/live (forbidden)'
  echo '  command: docker compose --env-file <env> --file <compose> build --pull=false <eleven local services>'
  echo '  no up/run/restart/start, migration, database, provider/API, or secret provisioning action'
  echo '  network caveat: Docker may fetch base/language dependencies when its cache is incomplete'
  echo 'PLAN_ONLY: no Docker command or env-file write made'
}

compose() {
  # Compose expands inactive services. These process-local values are inert,
  # never written to .env, and are not passed to a container lifecycle command.
  LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  RESEARCH_APP_ENV=prebuild-disabled \
  RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled \
  BACKTEST_MIN_FREE_BYTES=0 \
  BACKTEST_MAX_QUEUED_BACKTESTS=0 \
  BACKTEST_RECONCILE_GRACE_SECS=0 \
  BACKTEST_RECONCILE_INTERVAL_SECS=0 \
  RANGE_RAW_BATCH_ID=compose-config-disabled \
  COMPOSE_PROFILES= \
  LIVE_NODE_MODE=disabled \
  LIVE_NODE_DRY_RUN=1 \
    docker compose --env-file "$env_file" --file "$compose_file" "$@"
}

check_inputs

if [ "$mode" = plan ]; then
  print_plan
  exit 0
fi

command -v docker >/dev/null 2>&1 || die 'docker is not installed'
docker compose version >/dev/null 2>&1 || die 'Docker Compose v2 is unavailable'
compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS (no build or container lifecycle action)'
  exit 0
fi

compose build --pull=false "${local_image_services[@]}"
inspect_built_images
write_manifest
echo "PRODUCTION_IMAGE_BUILD: PASS (eleven images built and V2 manifest written: $manifest_file)"
