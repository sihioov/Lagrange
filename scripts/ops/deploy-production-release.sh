#!/usr/bin/env bash
# Install one exact clean Git commit as an immutable production release.
# Default is a no-change plan. --apply and --rollback are explicit root-only
# mutations; neither deletes an older release.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
mode=dry-run
mode_seen=0
release_commit=${LAGRANGE_CODE_COMMIT:-}
env_source=${LAGRANGE_RELEASE_ENV_SOURCE:-$repo_root/deploy/compose/.env}
install_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}

usage() {
  cat <<'EOF'
Usage: scripts/ops/deploy-production-release.sh [--dry-run|--check|--apply|--rollback]
       --commit EXACT_40_HEX [--env-source PATH] [--install-root DIR]

--dry-run  Print the immutable-release plan without reading protected .env (default).
--check    Root-only read-only validation of one installed release/current link.
--apply    Root-only install from an exact tracked-clean repository HEAD and
           atomically switch current. The one named KIS workbook may remain
           untracked and is excluded. Existing releases are never overwritten.
--rollback Root-only atomic current-link switch to an existing validated release.

The release is /opt/lagrange/releases/<commit>, current is one atomic symlink,
and deploy/nt/configs/migrations convenience links resolve through current.
Git archive excludes untracked files, .git, host Raw/Curated, and the untracked
KIS workbook. The protected Compose .env is copied separately as root:root 0600.
TLS/backup configs must pin releases/<commit> paths, not follow current.
No Docker, service, database, provider, or network command is run.
EOF
}

die() { echo "deploy-production-release: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run|--check|--apply|--rollback)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}; mode_seen=1; shift ;;
    --commit) [ "$#" -ge 2 ] || die '--commit needs a value'; release_commit=$2; shift 2 ;;
    --env-source) [ "$#" -ge 2 ] || die '--env-source needs a path'; env_source=$2; shift 2 ;;
    --install-root) [ "$#" -ge 2 ] || die '--install-root needs a path'; install_root=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[[ "$release_commit" =~ ^[0-9a-f]{40}$ ]] || die '--commit must be exactly 40 lowercase hexadecimal characters'
[ "$release_commit" != 0000000000000000000000000000000000000000 ] || die '--commit must not be all zeroes'

safe_absolute() {
  local path=$1 label=$2 probe
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in */../*|*/..) die "$label must not contain '..': $path" ;; esac
  case "$path" in /|/opt|/etc|/var|/var/lib|/tmp) die "$label is too broad: $path" ;; esac
  probe=${path%/}; [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}; [ -n "$probe" ] || probe=/
  done
}

safe_absolute "$install_root" install-root
safe_absolute "$env_source" env-source
release_dir=$install_root/releases/$release_commit
current_link=$install_root/current

if [ "$mode" = dry-run ]; then
  cat <<EOF
PRODUCTION_RELEASE_PLAN commit=$release_commit
  source=$repo_root (apply requires exact clean HEAD)
  env=$env_source -> $release_dir/deploy/compose/.env (root:root 0600)
  release=$release_dir (new immutable directory; never overwritten)
  current=$current_link -> releases/$release_commit (atomic switch)
  rollback=--rollback --commit <existing-exact-commit>
DRY_RUN: no Git archive, protected env read, file write, deletion, Docker, service, DB, or network action
EOF
  exit 0
fi

[ "$(id -u)" -eq 0 ] || die "--$mode must run as root"

check_root_parent() {
  local path=$1 probe metadata uid bits
  probe=$path
  while [ ! -e "$probe" ]; do
    probe=${probe%/*}; [ -n "$probe" ] || probe=/
  done
  [ -d "$probe" ] && [ ! -L "$probe" ] || die "unsafe install-root ancestor: $probe"
  metadata=$(stat -c '%u:%a' -- "$probe") || die 'cannot inspect install-root ancestor'
  uid=${metadata%%:*}; bits=$((8#${metadata#*:}))
  [ "$uid" = 0 ] || die 'install-root ancestor must be root-owned'
  (( (bits & 0022) == 0 )) || die 'install-root ancestor must not be group/other writable'
}
check_root_parent "$install_root"
check_root_parent "$(dirname -- "$env_source")"

validate_release() {
  local target=$1 expected=$2
  [ -d "$target" ] && [ ! -L "$target" ] || die "release is absent or not a directory: $target"
  [ -f "$target/.lagrange-release" ] && [ ! -L "$target/.lagrange-release" ] || die 'release marker missing'
  [ "$(stat -c '%u:%g:%a' -- "$target/.lagrange-release")" = 0:0:600 ] || die 'release marker metadata is unsafe'
  [ "$(tr -d '\r\n' <"$target/.lagrange-release")" = "$expected" ] || die 'release marker commit mismatch'
  [ -f "$target/deploy/compose/compose.yml" ] && [ ! -L "$target/deploy/compose/compose.yml" ] || die 'release Compose file missing'
  [ -f "$target/deploy/compose/.env" ] && [ ! -L "$target/deploy/compose/.env" ] || die 'release protected Compose env missing'
  [ "$(stat -c '%u:%g:%a' -- "$target/deploy/compose/.env")" = 0:0:600 ] || die 'release Compose env metadata is unsafe'
}

validate_current() {
  [ -L "$current_link" ] || die 'current is not a symlink'
  [ "$(readlink -- "$current_link")" = "releases/$release_commit" ] || die 'current does not name the requested release'
  validate_release "$release_dir" "$release_commit"
}

if [ "$mode" = check ]; then
  validate_current
  echo "PRODUCTION_RELEASE_CHECK: PASS commit=$release_commit"
  exit 0
fi

atomic_activate() {
  local temporary
  temporary=$install_root/.current.$release_commit.$$
  [ ! -e "$temporary" ] && [ ! -L "$temporary" ] || die 'temporary current link already exists'
  ln -s -- "releases/$release_commit" "$temporary" || die 'cannot stage current link'
  mv -Tf -- "$temporary" "$current_link" || { rm -f -- "$temporary"; die 'cannot atomically activate release'; }
}

ensure_compatibility_links() {
  local name target
  # Validate the complete set before creating the first convenience link.
  for name in deploy nt configs migrations scripts; do
    [ -e "$release_dir/$name" ] || continue
    target=$install_root/$name
    if [ -L "$target" ]; then
      [ "$(readlink -- "$target")" = "current/$name" ] || die "compatibility link has foreign target: $target"
    elif [ -e "$target" ]; then
      die "refusing to overwrite existing compatibility path: $target"
    fi
  done
  for name in deploy nt configs migrations scripts; do
    [ -e "$release_dir/$name" ] || continue
    target=$install_root/$name
    [ -L "$target" ] || ln -s -- "current/$name" "$target" || die "cannot create compatibility link: $target"
  done
}

if [ "$mode" = rollback ]; then
  validate_release "$release_dir" "$release_commit"
  ensure_compatibility_links
  atomic_activate
  echo "PRODUCTION_RELEASE_ROLLBACK: PASS commit=$release_commit"
  exit 0
fi

# Apply provenance: sudo against a user-owned checkout must not trip Git's
# dubious-ownership fence and must never mutate global Git configuration.
head_commit=$(git -c safe.directory="$repo_root" -C "$repo_root" rev-parse HEAD) || die 'cannot resolve repository HEAD'
[ "$head_commit" = "$release_commit" ] || die 'requested commit does not exactly match repository HEAD'
status_lines=$(git -c safe.directory="$repo_root" -C "$repo_root" status --porcelain=v1 --untracked-files=all) || die 'cannot inspect repository status'
while IFS= read -r status_line; do
  [ -z "$status_line" ] && continue
  # This operator-supplied official workbook is intentionally never deployed.
  # It is the sole untracked exception; tracked changes and every other
  # untracked path remain fail-closed.
  [ "$status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] ||
    die 'repository must have no tracked changes or unapproved untracked files'
done <<<"$status_lines"
[ -f "$env_source" ] && [ ! -L "$env_source" ] || die 'env-source must be a regular non-symlink file'
[ "$(stat -c '%u:%g:%a' -- "$env_source")" = 0:0:600 ] || die 'env-source must be root:root mode 0600'
[ ! -e "$release_dir" ] && [ ! -L "$release_dir" ] || die 'refusing to overwrite existing release directory'
if git -c safe.directory="$repo_root" -C "$repo_root" ls-tree -r --name-only "$release_commit" | grep -Fxq deploy/compose/.env; then
  die 'protected deploy/compose/.env must not be tracked in Git'
fi

install -d -o 0 -g 0 -m 0755 -- "$install_root" "$install_root/releases"
stage=$(mktemp -d -- "$install_root/releases/.staging.$release_commit.XXXXXX") || die 'cannot create release staging directory'
cleanup_stage() { [ -z "${stage:-}" ] || [ ! -d "$stage" ] || rm -rf -- "$stage"; }
trap cleanup_stage EXIT
chmod 0755 -- "$stage"
git -c safe.directory="$repo_root" -C "$repo_root" archive --format=tar "$release_commit" |
  tar -xf - -C "$stage" || die 'cannot materialize exact Git archive'
[ -z "$(find "$stage" -type l -print -quit)" ] || die 'release Git archive must not contain symlinks'
find "$stage" -type d -exec chmod 0755 {} +
find "$stage" -type f -exec chmod go-w {} +
[ -z "$(find "$stage" -type f -perm /022 -print -quit)" ] || die 'release Git archive contains a group/other-writable file'
install -o 0 -g 0 -m 0600 -- "$env_source" "$stage/deploy/compose/.env" || die 'cannot install protected Compose env'
printf '%s\n' "$release_commit" >"$stage/.lagrange-release"
chown -R 0:0 -- "$stage"
chmod 0600 -- "$stage/.lagrange-release"
mv -T -- "$stage" "$release_dir" || die 'cannot publish release directory'
stage=
validate_release "$release_dir" "$release_commit"
ensure_compatibility_links
atomic_activate
echo "PRODUCTION_RELEASE_APPLY: PASS commit=$release_commit release=$release_dir"
