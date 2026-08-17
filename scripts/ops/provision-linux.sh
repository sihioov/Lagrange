#!/usr/bin/env bash
# Prepare the native-Linux Lagrange Station filesystem contract.
#
# This script is intentionally non-destructive. It can create missing system
# accounts and directories, but it never removes, truncates, recursively
# copies, or changes the contents of an existing deployment tree. The default
# mode is --dry-run; --preflight only inspects an already-provisioned host;
# --apply is the explicit, root-only mutation mode an operator may run later.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

service_user=${LAGRANGE_SERVICE_USER:-lagrange}
service_group=${LAGRANGE_SERVICE_GROUP:-lagrange}
config_root=${LAGRANGE_CONFIG_ROOT:-/etc/lagrange}
deploy_root=${LAGRANGE_DEPLOY_ROOT:-/opt/lagrange}
data_root=${LAGRANGE_DATA_ROOT:-/var/lib/lagrange/data}
secret_root=${LAGRANGE_HOST_SECRET_ROOT:-$config_root/secrets}
worker_uid=${LAGRANGE_WORKER_UID:-10001}
worker_gid=${LAGRANGE_WORKER_GID:-10001}
mode=dry-run

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-linux.sh [--dry-run|--preflight|--apply]

Modes:
  --dry-run    Print the idempotent plan without changing the host (default).
  --preflight  Require the account, paths, ownership, and modes to exist.
  --apply      Create the account/paths and ownership fences; root required.

The paths may be overridden for an isolated test with:
  LAGRANGE_CONFIG_ROOT, LAGRANGE_DEPLOY_ROOT, LAGRANGE_DATA_ROOT,
  LAGRANGE_HOST_SECRET_ROOT, LAGRANGE_SERVICE_USER, LAGRANGE_SERVICE_GROUP,
  LAGRANGE_WORKER_UID, LAGRANGE_WORKER_GID.
EOF
}

die() {
  echo "provision-linux: $*" >&2
  exit 1
}

blocked() {
  echo "BLOCKED_EXTERNAL: $*" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) mode=dry-run; shift ;;
    --preflight) mode=preflight; shift ;;
    --apply) mode=apply; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (use --help)" ;;
  esac
done

is_absolute() { [[ "$1" = /* ]]; }
safe_path() {
  local path=$1 label=$2
  is_absolute "$path" || die "$label must be absolute: $path"
  case "$path" in
    */../*|*/..) die "$label must not contain '..': $path" ;;
  esac
  case "$path" in
    /|/etc|/opt|/var|/var/lib|/usr|/usr/local) die "$label is too broad: $path" ;;
  esac
  local probe=$path
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

safe_path "$config_root" LAGRANGE_CONFIG_ROOT
safe_path "$deploy_root" LAGRANGE_DEPLOY_ROOT
safe_path "$data_root" LAGRANGE_DATA_ROOT
safe_path "$secret_root" LAGRANGE_HOST_SECRET_ROOT
case "$worker_uid" in ''|*[!0-9]*) die 'LAGRANGE_WORKER_UID must be numeric' ;; esac
case "$worker_gid" in ''|*[!0-9]*) die 'LAGRANGE_WORKER_GID must be numeric' ;; esac

if [ "$mode" = apply ] && [ "$(id -u)" -ne 0 ]; then
  die '--apply must run as root; use --dry-run or --preflight for non-root checks'
fi

declare -a required_dirs=(
  "$config_root"
  "$config_root/universes"
  "$secret_root"
  "$deploy_root"
  "$deploy_root/bin"
  "$data_root"
  "$data_root/raw"
  "$data_root/curated"
  "$data_root/nautilus_catalog"
  "$data_root/artifacts"
  "$data_root/phase0"
)

for dir in "${required_dirs[@]}"; do
  safe_path "$dir" required-directory
done

print_plan() {
  echo "PROVISION_LINUX mode=$mode"
  echo "  service=$service_user:$service_group worker=$worker_uid:$worker_gid"
  echo "  config=$config_root deploy=$deploy_root data=$data_root"
  echo "  no deletion, truncation, secret generation, Docker start, or API call"
  for dir in "${required_dirs[@]}"; do
    echo "  ensure directory $dir"
  done
}

account_uid() { id -u "$service_user" 2>/dev/null; }
account_gid() { getent group "$service_group" | awk -F: 'NR == 1 { print $3 }'; }
service_group_member() {
  local gid=$1
  id -G "$service_user" 2>/dev/null | tr ' ' '\n' | grep -Fxq "$gid"
}

check_mode_owner() {
  local path=$1 expected_uid=$2 expected_gid=$3 expected_mode=$4 label=$5
  [ -d "$path" ] || blocked "$label is missing: $path"
  [ ! -L "$path" ] || die "$label must not be a symlink: $path"
  local actual
  actual=$(stat -c '%u:%g:%a' -- "$path") || die "cannot stat $label: $path"
  [ "$actual" = "$expected_uid:$expected_gid:$expected_mode" ] ||
    blocked "$label has $actual; expected $expected_uid:$expected_gid:$expected_mode: $path"
}

if [ "$mode" = dry-run ]; then
  print_plan
  echo "DRY_RUN: no host changes made"
  exit 0
fi

if [ "$mode" = preflight ]; then
  [ "$(id -u "$service_user" 2>/dev/null || true)" ] ||
    blocked "service user is missing: $service_user"
  [ -n "$(account_gid)" ] || blocked "service group is missing: $service_group"
  service_uid=$(account_uid)
  service_gid=$(account_gid)
  service_group_member "$service_gid" ||
    blocked "service user is not a member of service group: $service_user:$service_group"
  check_mode_owner "$config_root" 0 "$service_gid" 750 LAGRANGE_CONFIG_ROOT
  check_mode_owner "$config_root/universes" 0 "$service_gid" 750 universes
  check_mode_owner "$secret_root" 0 "$service_gid" 750 host-secrets
  check_mode_owner "$deploy_root" 0 0 755 LAGRANGE_DEPLOY_ROOT
  check_mode_owner "$deploy_root/bin" 0 0 755 deployment-binaries
  check_mode_owner "$data_root" 0 "$service_gid" 750 LAGRANGE_DATA_ROOT
  check_mode_owner "$data_root/raw" "$worker_uid" "$worker_gid" 750 raw
  check_mode_owner "$data_root/curated" "$worker_uid" "$worker_gid" 750 curated
  check_mode_owner "$data_root/nautilus_catalog" "$worker_uid" "$worker_gid" 750 nautilus_catalog
  check_mode_owner "$data_root/artifacts" "$service_uid" "$service_gid" 750 artifacts
  check_mode_owner "$data_root/phase0" "$service_uid" "$service_gid" 750 phase0
  echo "PREFLIGHT: PASS"
  exit 0
fi

print_plan

if ! getent group "$service_group" >/dev/null; then
  groupadd --system "$service_group"
fi
if ! getent passwd "$service_user" >/dev/null; then
  useradd --system --gid "$service_group" --home-dir /nonexistent \
    --shell /usr/sbin/nologin "$service_user"
fi

service_uid=$(account_uid)
service_gid=$(account_gid)
[ -n "$service_uid" ] && [ -n "$service_gid" ] || die 'service account lookup failed after creation'
service_group_member "$service_gid" ||
  blocked "service user is not a member of service group: $service_user:$service_group"

# Existing files are never recursively chowned. Each directory is created or
# ownership-fenced explicitly, which prevents a typo from rewriting a volume.
install -d -o root -g "$service_group" -m 0750 -- "$config_root"
install -d -o root -g "$service_group" -m 0750 -- "$config_root/universes"
install -d -o root -g "$service_group" -m 0750 -- "$secret_root"
install -d -o root -g root -m 0755 -- "$deploy_root"
install -d -o root -g root -m 0755 -- "$deploy_root/bin"
install -d -o root -g "$service_group" -m 0750 -- "$data_root"
install -d -o "$worker_uid" -g "$worker_gid" -m 0750 -- "$data_root/raw"
install -d -o "$worker_uid" -g "$worker_gid" -m 0750 -- "$data_root/curated"
install -d -o "$worker_uid" -g "$worker_gid" -m 0750 -- "$data_root/nautilus_catalog"
install -d -o "$service_uid" -g "$service_gid" -m 0750 -- "$data_root/artifacts"
install -d -o "$service_uid" -g "$service_gid" -m 0750 -- "$data_root/phase0"

echo "APPLY: host paths and service account are ready"
echo "APPLY: next run scripts/ops/validate-production-config.sh before secrets/Compose"
