#!/usr/bin/env bash
# Install one exact clean Git commit as an immutable owner-beta release. The
# default is a no-change plan. --apply and --rollback are explicit root-only
# mutations; neither deletes an older release.
#
# This installer does not build, start, stop, inspect, or contact Docker. The
# separate installed compose-release.sh binds startup to the installed V2
# manifest's host-local image IDs.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/release-image-manifest.sh"

mode=dry-run
mode_seen=0
release_commit=${LAGRANGE_CODE_COMMIT:-}
env_source=${LAGRANGE_RELEASE_ENV_SOURCE:-$repo_root/deploy/compose/.env}
install_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}
release_manifest=${LAGRANGE_RELEASE_MANIFEST:-}
env_source_seen=0
release_manifest_seen=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/deploy-production-release.sh [--dry-run|--check|--apply|--rollback]
       --commit EXACT_40_HEX [--install-root DIR]
       [--env-source PATH --release-manifest ABSOLUTE_PATH]

--dry-run  Print the immutable-release plan without reading protected inputs.
--check    Root-only read-only validation of current and its installed trusted
           V2 manifest. It never accepts an external replacement manifest.
--apply    Root-only install from an exact tracked-clean repository HEAD and
           atomically switch current. It requires a protected env source and a
           strict V2 --release-manifest; existing releases are never replaced.
--rollback Root-only atomic current-link switch to an existing release after
           validating that release's installed trusted V2 manifest.

The manifest must be a regular non-symlink root:root 0600 file below a
root-owned, non-group/other-writable path. Apply validates it before copying it
once into root-owned staging, then validates the installed copy before atomic
activation. Legacy releases without that installed manifest are blocked.

The release is /opt/lagrange/releases/<commit>; current is one relative atomic
symlink. Git archive excludes untracked files, .git, host Raw/Curated, and the
untracked KIS workbook. The protected Compose .env is copied separately as a
root:root 0600 file. No Docker, service, database, provider, or network command
is run by this installer.
EOF
}

die() { echo "deploy-production-release: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run|--check|--apply|--rollback)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --commit)
      [ "$#" -ge 2 ] || die '--commit needs a value'
      release_commit=$2
      shift 2
      ;;
    --env-source)
      [ "$#" -ge 2 ] || die '--env-source needs a path'
      env_source=$2
      env_source_seen=1
      shift 2
      ;;
    --install-root)
      [ "$#" -ge 2 ] || die '--install-root needs a path'
      install_root=$2
      shift 2
      ;;
    --release-manifest)
      [ "$#" -ge 2 ] || die '--release-manifest needs a path'
      release_manifest=$2
      release_manifest_seen=1
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

release_image_manifest_is_commit "$release_commit" ||
  die '--commit must be exactly 40 lowercase hexadecimal characters and not all zeroes'

safe_absolute() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in
    *$'\n'*|*$'\r'*|*'//'*) die "$label is not a canonical absolute path" ;;
    */../*|*/..|*/./*|*/.) die "$label must not contain dot path components" ;;
    */) die "$label must not have a trailing slash" ;;
  esac
  case "$path" in
    /|/opt|/etc|/var|/var/lib|/tmp) die "$label is too broad: $path" ;;
  esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

safe_absolute "$install_root" install-root
release_dir=$install_root/releases/$release_commit
current_link=$install_root/current

if [ "$mode" = dry-run ]; then
  cat <<EOF
PRODUCTION_RELEASE_PLAN commit=$release_commit
  source=$repo_root (apply requires exact clean HEAD)
  env=<root:root 0600 external input> -> $release_dir/deploy/compose/.env
  manifest=<root:root 0600 V2 external input> -> $release_dir/.lagrange-release-manifest
  release=$release_dir (new immutable directory; never overwritten)
  current=$current_link -> releases/$release_commit (atomic switch)
  rollback=--rollback --commit <existing-exact-commit> (installed manifest required)
  startup=installed compose-release.sh --scope release --apply (manifest image_id override; no build)
DRY_RUN: no Git archive, protected env/manifest read, file write, Docker, service, DB, or network action
EOF
  exit 0
fi

[ "$(id -u)" -eq 0 ] || die "--$mode must run as root"

# Only apply consumes external deployment inputs. Check and rollback deliberately
# use the installed manifest rather than allowing a second file to change their
# expectations; source .env is irrelevant after immutable release creation.
if [ "$mode" != apply ]; then
  [ "$env_source_seen" -eq 0 ] || die '--env-source is allowed only with --apply'
  [ "$release_manifest_seen" -eq 0 ] && [ -z "$release_manifest" ] ||
    die '--release-manifest is allowed only with --apply; check/rollback use the installed trusted manifest'
fi

check_root_parent() {
  local path=$1 label=$2 probe
  safe_absolute "$path" "$label"
  probe=$path
  while [ ! -e "$probe" ] && [ ! -L "$probe" ]; do
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  [ -d "$probe" ] && [ ! -L "$probe" ] ||
    die "$label has no safe existing directory ancestor"
  if ! release_image_manifest_trusted_directory "$probe" "$label"; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
}

check_root_parent "$install_root" install-root

validate_external_manifest() {
  local path=$1
  safe_absolute "$path" release-manifest
  if ! release_image_manifest_trusted_file "$path" release-manifest; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  if ! release_image_manifest_load "$path" "$release_commit"; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
}

validate_installed_manifest() {
  local path=$1 label=$2
  if ! release_image_manifest_trusted_file "$path" "$label"; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  if ! release_image_manifest_load "$path" "$release_commit"; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
}

validate_release() {
  local target=$1 expected=$2 marker manifest metadata
  [ "$expected" = "$release_commit" ] || die 'internal release validation commit mismatch'
  [ -d "$target" ] && [ ! -L "$target" ] ||
    die "release is absent or not a directory: $target"
  if ! release_image_manifest_trusted_directory "$target" release-directory; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  marker=$target/.lagrange-release
  if ! release_image_manifest_trusted_file "$marker" release-marker; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  [ "$(tr -d '\r\n' <"$marker")" = "$expected" ] || die 'release marker commit mismatch'
  [ -f "$target/deploy/compose/compose.yml" ] && [ ! -L "$target/deploy/compose/compose.yml" ] ||
    die 'release Compose file missing'
  if ! release_image_manifest_trusted_file "$target/deploy/compose/.env" release-compose-env; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  manifest=$target/.lagrange-release-manifest
  [ -e "$manifest" ] || [ -L "$manifest" ] ||
    die 'legacy manifest-less release is blocked'
  validate_installed_manifest "$manifest" installed-release-manifest
}

validate_current() {
  [ -L "$current_link" ] || die 'current is not a symlink'
  [ "$(readlink -- "$current_link")" = "releases/$release_commit" ] ||
    die 'current does not name the requested release'
  validate_release "$release_dir" "$release_commit"
}

atomic_activate() {
  local temporary existing
  if [ -e "$current_link" ] && [ ! -L "$current_link" ]; then
    die 'current exists but is not a symlink'
  fi
  if [ -L "$current_link" ]; then
    existing=$(readlink -- "$current_link") || die 'cannot read existing current link'
    [[ "$existing" =~ ^releases/[0-9a-f]{40}$ ]] ||
      die 'current has a foreign link target'
  fi
  temporary=$install_root/.current.$release_commit.$$
  [ ! -e "$temporary" ] && [ ! -L "$temporary" ] || die 'temporary current link already exists'
  ln -s -- "releases/$release_commit" "$temporary" || die 'cannot stage current link'
  mv -Tf -- "$temporary" "$current_link" || {
    rm -f -- "$temporary"
    die 'cannot atomically activate release'
  }
}

ensure_compatibility_links() {
  local name target
  # Validate the complete set before creating the first convenience link.
  for name in deploy nt configs migrations scripts; do
    [ -e "$release_dir/$name" ] || continue
    target=$install_root/$name
    if [ -L "$target" ]; then
      [ "$(readlink -- "$target")" = "current/$name" ] ||
        die "compatibility link has foreign target: $target"
    elif [ -e "$target" ]; then
      die "refusing to overwrite existing compatibility path: $target"
    fi
  done
  for name in deploy nt configs migrations scripts; do
    [ -e "$release_dir/$name" ] || continue
    target=$install_root/$name
    [ -L "$target" ] ||
      ln -s -- "current/$name" "$target" || die "cannot create compatibility link: $target"
  done
}

if [ "$mode" = check ]; then
  validate_current
  echo "PRODUCTION_RELEASE_CHECK: PASS commit=$release_commit manifest=installed-v2"
  exit 0
fi

if [ "$mode" = rollback ]; then
  validate_release "$release_dir" "$release_commit"
  ensure_compatibility_links
  atomic_activate
  echo "PRODUCTION_RELEASE_ROLLBACK: PASS commit=$release_commit manifest=installed-v2"
  exit 0
fi

[ -n "$release_manifest" ] ||
  die '--apply requires --release-manifest (or LAGRANGE_RELEASE_MANIFEST)'
safe_absolute "$env_source" env-source
check_root_parent "$(dirname -- "$env_source")" env-source-parent
[ -f "$env_source" ] && [ ! -L "$env_source" ] ||
  die 'env-source must be a regular non-symlink file'
if ! release_image_manifest_trusted_file "$env_source" env-source; then
  die "$RELEASE_IMAGE_MANIFEST_ERROR"
fi
validate_external_manifest "$release_manifest"

# Apply provenance: sudo against a user-owned checkout must not trip Git's
# dubious-ownership fence and must never mutate global Git configuration.
head_commit=$(git -c safe.directory="$repo_root" -C "$repo_root" rev-parse HEAD) ||
  die 'cannot resolve repository HEAD'
[ "$head_commit" = "$release_commit" ] ||
  die 'requested commit does not exactly match repository HEAD'
status_lines=$(git -c safe.directory="$repo_root" -C "$repo_root" \
  status --porcelain=v1 --untracked-files=all) || die 'cannot inspect repository status'
while IFS= read -r status_line; do
  [ -z "$status_line" ] && continue
  # This operator-supplied official workbook is intentionally never deployed.
  [ "$status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] ||
    die 'repository must have no tracked changes or unapproved untracked files'
done <<<"$status_lines"
[ ! -e "$release_dir" ] && [ ! -L "$release_dir" ] ||
  die 'refusing to overwrite existing release directory'
if git -c safe.directory="$repo_root" -C "$repo_root" \
  ls-tree -r --name-only "$release_commit" | grep -Fxq deploy/compose/.env; then
  die 'protected deploy/compose/.env must not be tracked in Git'
fi

install -d -o 0 -g 0 -m 0755 -- "$install_root" "$install_root/releases"
stage=$(mktemp -d -- "$install_root/releases/.staging.$release_commit.XXXXXX") ||
  die 'cannot create release staging directory'
cleanup_stage() { [ -z "${stage:-}" ] || [ ! -d "$stage" ] || rm -rf -- "$stage"; }
trap cleanup_stage EXIT
chmod 0755 -- "$stage"
git -c safe.directory="$repo_root" -C "$repo_root" archive --format=tar "$release_commit" |
  tar --no-same-owner -xf - -C "$stage" || die 'cannot materialize exact Git archive'
[ -z "$(find "$stage" -type l -print -quit)" ] ||
  die 'release Git archive must not contain symlinks'
find "$stage" -type d -exec chmod 0755 {} +
find "$stage" -type f -exec chmod go-w {} +
[ -z "$(find "$stage" -type f -perm /022 -print -quit)" ] ||
  die 'release Git archive contains a group/other-writable file'
install -o 0 -g 0 -m 0600 -- "$env_source" "$stage/deploy/compose/.env" ||
  die 'cannot install protected Compose env'
# The external source was trusted and parsed above. Copy it once; thereafter
# only the root-owned staged/installed copy is read for release validation.
install -o 0 -g 0 -m 0600 -- "$release_manifest" "$stage/.lagrange-release-manifest" ||
  die 'cannot install release image manifest'
validate_installed_manifest "$stage/.lagrange-release-manifest" staged-release-manifest
printf '%s\n' "$release_commit" >"$stage/.lagrange-release"
chown -R 0:0 -- "$stage"
chmod 0600 -- "$stage/.lagrange-release"
mv -T -- "$stage" "$release_dir" || die 'cannot publish release directory'
stage=
validate_release "$release_dir" "$release_commit"
ensure_compatibility_links
atomic_activate
echo "PRODUCTION_RELEASE_APPLY: PASS commit=$release_commit release=$release_dir manifest=installed-v2"
