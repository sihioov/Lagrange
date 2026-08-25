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
artifacts_root=${LAGRANGE_ARTIFACTS_DIR:-$data_root/artifacts}
secret_root=${LAGRANGE_HOST_SECRET_ROOT:-$config_root/secrets}
worker_uid=${LAGRANGE_WORKER_UID:-10001}
worker_gid=${LAGRANGE_WORKER_GID:-10001}
data_group=lagrange-data
mode=dry-run

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-linux.sh [--dry-run|--preflight|--apply]

Modes:
  --dry-run    Print the idempotent plan without changing the host (default).
  --preflight  Require the account, paths, ownership, and modes to exist;
               root is required to inspect protected host paths.
  --apply      Create the account/paths and ownership fences; root required.

The --dry-run plan is safe to inspect as a non-root user. Both --preflight and
--apply require root because the provisioned paths are intentionally protected
by root ownership and mode 0750 fences.

The paths may be overridden for an isolated test with:
  LAGRANGE_CONFIG_ROOT, LAGRANGE_DEPLOY_ROOT, LAGRANGE_DATA_ROOT,
  LAGRANGE_ARTIFACTS_DIR,
  LAGRANGE_HOST_SECRET_ROOT, LAGRANGE_SERVICE_USER, LAGRANGE_SERVICE_GROUP,
  LAGRANGE_WORKER_UID, LAGRANGE_WORKER_GID. The worker UID/GID must remain
  exactly 10001 to match the Compose and systemd container identity; the host
  data group is always named lagrange-data with GID 10001.
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

# Preflight must inspect directories whose ancestors are intentionally
# root-owned and mode 0750. Refuse before any path/account checks so an
# unprivileged caller gets an actionable permission message instead of a
# misleading "missing" result for an existing child.
if [ "$(id -u)" -ne 0 ]; then
  case "$mode" in
    preflight)
      die '--preflight must run as root to inspect protected paths; use sudo scripts/ops/provision-linux.sh --preflight (or --dry-run for a non-root plan)'
      ;;
    apply)
      die '--apply must run as root; use --dry-run for non-root checks'
      ;;
  esac
fi

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
safe_path "$artifacts_root" LAGRANGE_ARTIFACTS_DIR
safe_path "$secret_root" LAGRANGE_HOST_SECRET_ROOT
case "$worker_uid" in
  10001) ;;
  ''|*[!0-9]*) die 'LAGRANGE_WORKER_UID must be numeric 10001' ;;
  *) die 'LAGRANGE_WORKER_UID must be exactly 10001' ;;
esac
case "$worker_gid" in
  10001) ;;
  ''|*[!0-9]*) die 'LAGRANGE_WORKER_GID must be numeric 10001' ;;
  *) die 'LAGRANGE_WORKER_GID must be exactly 10001' ;;
esac

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
  "$artifacts_root"
  "$artifacts_root/historical-price-beta-root"
  "$artifacts_root/backtest"
  "$artifacts_root/backtest/runs"
  "$data_root/phase0"
)

for dir in "${required_dirs[@]}"; do
  safe_path "$dir" required-directory
done

print_plan() {
  echo "PROVISION_LINUX mode=$mode"
  echo "  service=$service_user:$service_group worker=$worker_uid:$worker_gid data-group=$data_group:$worker_gid"
  case "$data_group_action" in
    create) echo "  ensure group $data_group with GID $worker_gid" ;;
    use) echo "  use existing group $data_group with GID $worker_gid" ;;
  esac
  echo "  config=$config_root deploy=$deploy_root data=$data_root artifacts=$artifacts_root"
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
group_record_by_gid() {
  local gid=$1
  getent group | awk -F: -v expected_gid="$gid" '$3 == expected_gid { print; exit }'
}
group_record_by_name() {
  getent group "$1" | awk -F: 'NR == 1 { print; exit }'
}
group_name_from_record() {
  printf '%s\n' "$1" | awk -F: 'NR == 1 { print $1 }'
}
group_gid_from_record() {
  printf '%s\n' "$1" | awk -F: 'NR == 1 { print $3 }'
}
group_members_from_record() {
  printf '%s\n' "$1" | awk -F: 'NR == 1 { print $4 }'
}

data_group_action=create
data_group_record=
resolve_data_group() {
  local named_record gid_record members primary_users

  named_record=$(group_record_by_name "$data_group" || true)
  if [ -n "$named_record" ]; then
    [ "$(group_gid_from_record "$named_record")" = "$worker_gid" ] ||
      die "data group name conflict: $data_group has GID $(group_gid_from_record "$named_record"); expected $worker_gid"
    members=$(group_members_from_record "$named_record")
    [ -z "$members" ] || [ "$members" = "$service_user" ] ||
      die "data group $data_group has an unauthorized explicit member list: $members"
    primary_users=$(getent passwd | awk -F: -v expected_gid="$worker_gid" '$4 == expected_gid { print $1; exit }')
    [ -z "$primary_users" ] ||
      die "data group $data_group is a primary group for account: $primary_users"
    data_group_record=$named_record
    data_group_action=use
    return
  fi

  gid_record=$(group_record_by_gid "$worker_gid" || true)
  [ -z "$gid_record" ] ||
    die "data group GID conflict: GID $worker_gid already belongs to $(group_name_from_record "$gid_record"); expected $data_group"

  data_group_record=
  data_group_action=create
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

resolve_data_group

if [ "$mode" = dry-run ]; then
  print_plan
  echo "DRY_RUN: no host changes made"
  exit 0
fi

if [ "$mode" = preflight ]; then
  [ "$(id -u "$service_user" 2>/dev/null || true)" ] ||
    blocked "service user is missing: $service_user"
  [ -n "$(account_gid)" ] || blocked "service group is missing: $service_group"
  data_group_record=$(group_record_by_name "$data_group" || true)
  [ -n "$data_group_record" ] ||
    blocked "data group is missing: $data_group (GID $worker_gid)"
  [ "$(group_gid_from_record "$data_group_record")" = "$worker_gid" ] ||
    blocked "data group has GID $(group_gid_from_record "$data_group_record"); expected $worker_gid: $data_group"
  service_uid=$(account_uid)
  service_gid=$(account_gid)
  service_group_member "$service_gid" ||
    blocked "service user is not a member of service group: $service_user:$service_group"
  service_group_member "$worker_gid" ||
    blocked "service user is not a member of data group: $service_user:$data_group (GID $worker_gid)"
  check_mode_owner "$config_root" 0 "$service_gid" 750 LAGRANGE_CONFIG_ROOT
  check_mode_owner "$config_root/universes" 0 "$service_gid" 750 universes
  check_mode_owner "$secret_root" 0 "$service_gid" 750 host-secrets
  check_mode_owner "$deploy_root" 0 0 755 LAGRANGE_DEPLOY_ROOT
  check_mode_owner "$deploy_root/bin" 0 0 755 deployment-binaries
  check_mode_owner "$data_root" 0 "$service_gid" 750 LAGRANGE_DATA_ROOT
  check_mode_owner "$data_root/raw" "$worker_uid" "$worker_gid" 750 raw
  check_mode_owner "$data_root/curated" "$worker_uid" "$worker_gid" 750 curated
  check_mode_owner "$data_root/nautilus_catalog" "$worker_uid" "$worker_gid" 750 nautilus_catalog
  check_mode_owner "$artifacts_root" "$service_uid" "$service_gid" 750 artifacts
  check_mode_owner "$artifacts_root/historical-price-beta-root" "$worker_uid" "$worker_gid" 750 historical-price-beta-root
  check_mode_owner "$artifacts_root/backtest" "$worker_uid" "$worker_gid" 750 backtest-artifacts
  check_mode_owner "$artifacts_root/backtest/runs" "$worker_uid" "$worker_gid" 750 backtest-runs
  check_mode_owner "$data_root/phase0" "$service_uid" "$service_gid" 750 phase0
  echo "PREFLIGHT: PASS"
  exit 0
fi

print_plan

if [ "$data_group_action" = create ]; then
  groupadd --system --gid "$worker_gid" "$data_group"
fi
data_group_record=$(group_record_by_name "$data_group" || true)
[ -n "$data_group_record" ] || die "data group lookup failed after creation: $data_group"
[ "$(group_gid_from_record "$data_group_record")" = "$worker_gid" ] ||
  die "data group GID changed during provisioning: $data_group"

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
if ! service_group_member "$worker_gid"; then
  usermod --append --groups "$data_group" "$service_user"
fi
service_group_member "$worker_gid" ||
  blocked "service user is not a member of data group: $service_user:$data_group (GID $worker_gid)"

# Existing files are never recursively chowned. Each directory is created or
# ownership-fenced explicitly, which prevents a typo from rewriting a volume.
install -d -o root -g "$service_group" -m 0750 -- "$config_root"
install -d -o root -g "$service_group" -m 0750 -- "$config_root/universes"
install -d -o root -g "$service_group" -m 0750 -- "$secret_root"
install -d -o root -g root -m 0755 -- "$deploy_root"
install -d -o root -g root -m 0755 -- "$deploy_root/bin"
install -d -o root -g "$service_group" -m 0750 -- "$data_root"
# GNU install resolves -o/-g operands as account names and rejects a numeric
# UID that has intentionally not been created on the host. Create the fenced
# paths first, then use chown's numeric-ID form after the data group exists.
install -d -m 0750 -- "$data_root/raw"
chown "$worker_uid:$worker_gid" -- "$data_root/raw"
install -d -m 0750 -- "$data_root/curated"
chown "$worker_uid:$worker_gid" -- "$data_root/curated"
install -d -m 0750 -- "$data_root/nautilus_catalog"
chown "$worker_uid:$worker_gid" -- "$data_root/nautilus_catalog"
if [ -e "$artifacts_root" ] || [ -L "$artifacts_root" ]; then
  check_mode_owner "$artifacts_root" "$service_uid" "$service_gid" 750 artifacts
else
  install -d -o "$service_uid" -g "$service_gid" -m 0750 -- "$artifacts_root"
fi
if [ -e "$artifacts_root/historical-price-beta-root" ] ||
   [ -L "$artifacts_root/historical-price-beta-root" ]; then
  check_mode_owner "$artifacts_root/historical-price-beta-root" "$worker_uid" "$worker_gid" 750 historical-price-beta-root
else
  install -d -m 0750 -- "$artifacts_root/historical-price-beta-root"
  chown "$worker_uid:$worker_gid" -- "$artifacts_root/historical-price-beta-root"
fi
if [ -e "$artifacts_root/backtest" ] || [ -L "$artifacts_root/backtest" ]; then
  check_mode_owner "$artifacts_root/backtest" "$worker_uid" "$worker_gid" 750 backtest-artifacts
else
  install -d -m 0750 -- "$artifacts_root/backtest"
  chown "$worker_uid:$worker_gid" -- "$artifacts_root/backtest"
fi
if [ -e "$artifacts_root/backtest/runs" ] ||
   [ -L "$artifacts_root/backtest/runs" ]; then
  check_mode_owner "$artifacts_root/backtest/runs" "$worker_uid" "$worker_gid" 750 backtest-runs
else
  install -d -m 0750 -- "$artifacts_root/backtest/runs"
  chown "$worker_uid:$worker_gid" -- "$artifacts_root/backtest/runs"
fi
install -d -o "$service_uid" -g "$service_gid" -m 0750 -- "$data_root/phase0"

echo "APPLY: host paths and service account are ready"
echo 'APPLY: next run sudo scripts/ops/provision-db-secrets.sh --apply (or --check if already provisioned)'
echo 'APPLY: then sudo scripts/ops/provision-crypto-secrets.sh --apply (or --check if already provisioned)'
echo 'APPLY: then sudo deploy/secrets/provision-runtime-secrets.sh --scope infrastructure'
echo 'APPLY: then export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"'
echo 'APPLY: then sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" scripts/ops/validate-production-config.sh --scope infrastructure --env-file deploy/compose/.env'
