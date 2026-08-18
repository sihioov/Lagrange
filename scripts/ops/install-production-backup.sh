#!/usr/bin/env bash
# Install the production backup helper, protected config, and systemd units.
# Applying never creates a backup, prunes data, starts a timer, or calls Docker.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
mode=dry-run
mode_seen=0
helper_source=$script_dir/run-production-backup.sh
config_source=$root/deploy/systemd/production-backup.conf.example
install_bin=/opt/lagrange/bin
systemd_dir=/etc/systemd/system
config_target=/etc/lagrange/production-backup.conf
unit_names=(
  lagrange-production-backup.service
  lagrange-production-backup.timer
  lagrange-production-backup-verify.service
  lagrange-production-backup-verify.timer
)

usage() {
  cat <<'EOF'
Usage: install-production-backup.sh [--dry-run|--check|--apply]
       [--config-source PATH] [--install-bin DIR] [--systemd-dir DIR]
       [--config-target PATH]

--dry-run is the default no-change plan. --check is root-only and read-only.
--apply is root-only, requires a fully customized root:root 0600 config, refuses
all existing targets, installs atomically, daemon-reloads, and enables timers
without starting them. It never invokes Docker, backup, restore, or pruning.
EOF
}
die() { echo "install-production-backup: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run|--check|--apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}; mode_seen=1; shift ;;
    --config-source) [ "$#" -ge 2 ] || die '--config-source needs a path'; config_source=$2; shift 2 ;;
    --install-bin) [ "$#" -ge 2 ] || die '--install-bin needs a path'; install_bin=$2; shift 2 ;;
    --systemd-dir) [ "$#" -ge 2 ] || die '--systemd-dir needs a path'; systemd_dir=$2; shift 2 ;;
    --config-target) [ "$#" -ge 2 ] || die '--config-target needs a path'; config_target=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

if [ "$mode" = dry-run ]; then
  echo 'PRODUCTION_BACKUP_INSTALL_PLAN mode=dry-run'
  echo "  helper=$helper_source -> $install_bin/run-production-backup.sh"
  echo "  config=$config_source -> $config_target (must be customized root:root 0600)"
  for name in "${unit_names[@]}"; do echo "  unit=$root/deploy/systemd/$name -> $systemd_dir/$name"; done
  echo '  --apply daemon-reloads and enables both timers, but never starts them or runs a backup'
  echo 'DRY_RUN: no protected config read, install, Docker, DB, backup, prune, service start, or deletion'
  exit 0
fi
[ "$(id -u)" -eq 0 ] || die "--$mode must run as root"

safe_path() {
  local path=$1 label=$2 probe
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in */../*|*/..) die "$label must not contain '..': $path" ;; esac
  case "$path" in /|/etc|/opt|/var|/var/lib|/tmp|/run) die "$label is too broad: $path" ;; esac
  probe=${path%/}; [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}; [ -n "$probe" ] || probe=/
  done
}
safe_path "$helper_source" helper-source
safe_path "$config_source" config-source
safe_path "$install_bin" install-bin
safe_path "$systemd_dir" systemd-dir
safe_path "$config_target" config-target
[ -f "$helper_source" ] && [ ! -L "$helper_source" ] || die 'helper source is unsafe'
[ -f "$config_source" ] && [ ! -L "$config_source" ] || die 'config source is unsafe'
[ "$(stat -c '%u:%g:%a' -- "$config_source")" = 0:0:600 ] || die 'config source must be root:root mode 0600'
for name in "${unit_names[@]}"; do
  [ -f "$root/deploy/systemd/$name" ] && [ ! -L "$root/deploy/systemd/$name" ] || die "unit source is unsafe: $name"
done

check_parent() {
  local path=$1 label=$2 probe metadata uid bits
  probe=${path%/*}; [ -n "$probe" ] || probe=/
  while [ ! -e "$probe" ]; do probe=${probe%/*}; [ -n "$probe" ] || probe=/; done
  [ -d "$probe" ] && [ ! -L "$probe" ] || die "$label parent is unsafe: $probe"
  metadata=$(stat -c '%u:%a' -- "$probe") || die "cannot inspect $label parent"
  uid=${metadata%%:*}; bits=$((8#${metadata#*:}))
  [ "$uid" = 0 ] || die "$label parent must be root-owned"
  (( (bits & 0022) == 0 )) || die "$label parent must not be group/other writable"
}
check_parent "$install_bin/run-production-backup.sh" install-bin
check_parent "$systemd_dir/lagrange-production-backup.service" systemd-dir
check_parent "$config_target" config-target

helper_target=$install_bin/run-production-backup.sh
check_one() {
  local source=$1 target=$2 mode=$3 label=$4
  [ -f "$target" ] && [ ! -L "$target" ] || die "$label missing or unsafe"
  [ "$(stat -c '%u:%g:%a' -- "$target")" = "0:0:$mode" ] || die "$label metadata mismatch"
  cmp -s -- "$source" "$target" || die "$label differs from approved source"
}

if [ "$mode" = check ]; then
  check_one "$helper_source" "$helper_target" 755 helper
  check_one "$config_source" "$config_target" 600 config
  for name in "${unit_names[@]}"; do check_one "$root/deploy/systemd/$name" "$systemd_dir/$name" 644 "$name"; done
  "$helper_target" --check --config-file "$config_target" >/dev/null
  echo 'PRODUCTION_BACKUP_INSTALL_CHECK: PASS'
  exit 0
fi

# Validate all protected inputs before the first write. The runner check makes
# no Docker/DB call and never prints or hashes the key value.
"$helper_source" --check --config-file "$config_source" >/dev/null

config_value() {
  local wanted=$1 line
  while IFS= read -r line || [ -n "$line" ]; do
    [ "${line%%=*}" = "$wanted" ] || continue
    printf '%s' "${line#*=}"
    return 0
  done <"$config_source"
  return 1
}
configured_backup_root=$(config_value BACKUP_ROOT) || die 'validated config lost BACKUP_ROOT'
configured_data_root=$(config_value DATA_ROOT) || die 'validated config lost DATA_ROOT'
configured_key_file=$(config_value KEY_FILE) || die 'validated config lost KEY_FILE'
configured_lock_file=$(config_value LOCK_FILE) || die 'validated config lost LOCK_FILE'
configured_metrics_file=$(config_value METRICS_FILE) || die 'validated config lost METRICS_FILE'
if [ "$systemd_dir" = /etc/systemd/system ] && [ "$config_target" = /etc/lagrange/production-backup.conf ]; then
  [ "$configured_backup_root" = /srv/backups/lagrange ] || die 'production unit requires BACKUP_ROOT=/srv/backups/lagrange'
  [ "$configured_data_root" = /var/lib/lagrange/data ] || die 'production unit requires DATA_ROOT=/var/lib/lagrange/data'
  [ "$configured_key_file" = /etc/lagrange/secrets/backup_encryption_key ] || die 'production unit requires the canonical backup key path'
  [ "$configured_lock_file" = /var/lib/lagrange/backup-state/production-backup.lock ] || die 'production unit requires the canonical lock path'
  [ "$configured_metrics_file" = /var/lib/lagrange/backup-state/production-backup.prom ] || die 'production unit requires the canonical metrics path'
fi
for target in "$helper_target" "$config_target"; do
  [ ! -e "$target" ] && [ ! -L "$target" ] || die "refusing to overwrite existing target: $target"
done
for name in "${unit_names[@]}"; do
  target=$systemd_dir/$name
  [ ! -e "$target" ] && [ ! -L "$target" ] || die "refusing to overwrite existing target: $target"
done
install -d -o 0 -g 0 -m 0755 -- "$install_bin" "$systemd_dir" "$(dirname -- "$config_target")"
install -d -o 0 -g 0 -m 0700 -- "$configured_backup_root" \
  "$(dirname -- "$configured_lock_file")" "$(dirname -- "$configured_metrics_file")"

installed=()
rollback_new() {
  local path
  for path in "${installed[@]}"; do rm -f -- "$path" 2>/dev/null || true; done
}
trap rollback_new ERR
install -o 0 -g 0 -m 0755 -- "$helper_source" "$helper_target"; installed+=("$helper_target")
install -o 0 -g 0 -m 0600 -- "$config_source" "$config_target"; installed+=("$config_target")
for name in "${unit_names[@]}"; do
  target=$systemd_dir/$name
  install -o 0 -g 0 -m 0644 -- "$root/deploy/systemd/$name" "$target"
  installed+=("$target")
done
systemctl daemon-reload
systemctl enable lagrange-production-backup.timer lagrange-production-backup-verify.timer
trap - ERR
echo 'PRODUCTION_BACKUP_INSTALL_APPLY: PASS timers=enabled-not-started'
