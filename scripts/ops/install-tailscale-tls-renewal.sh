#!/usr/bin/env bash
# Install the Tailscale TLS renewal helper, protected configuration, and
# systemd service/timer. This installer never requests a certificate. The
# default is a plan; --check is read-only; --apply is the only mutating mode.
# Applying enables the timer but deliberately does not start it.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
mode=dry-run
mode_seen=0
helper_source=$script_dir/renew-tailscale-tls.sh
service_source=$root/deploy/systemd/lagrange-tailscale-tls-renewal.service
timer_source=$root/deploy/systemd/lagrange-tailscale-tls-renewal.timer
config_source=$root/deploy/systemd/tailscale-tls-renewal.conf.example
install_bin_dir=/opt/lagrange/bin
systemd_dir=/etc/systemd/system
config_target=/etc/lagrange/tailscale-tls-renewal.conf

usage() {
  cat <<'EOF'
Usage: scripts/ops/install-tailscale-tls-renewal.sh [--dry-run|--check|--apply]
       [--helper-source PATH] [--service-source PATH] [--timer-source PATH]
       [--config-source PATH] [--install-bin DIR] [--systemd-dir DIR]
       [--config-target PATH]

Modes:
  --dry-run       Print the install plan without changing the host (default).
  --check         Root-only read-only check of installed artifacts and config.
  --apply         Root-only atomic copy of the helper, units, and protected
                  config, followed by systemd daemon-reload and timer enable
                  (never starts the timer or renewal service).

The installer performs no tailscale cert issuance, Docker call, service start,
or KIS/Auth0/database operation. It never deletes files. Production defaults
install the helper at /opt/lagrange/bin, units under /etc/systemd/system, and
the root-owned 0600 config at /etc/lagrange/tailscale-tls-renewal.conf.
EOF
}

die() {
  echo "install-tailscale-tls-renewal: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --apply'
      mode=dry-run
      mode_seen=1
      shift
      ;;
    --check)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --apply'
      mode=check
      mode_seen=1
      shift
      ;;
    --apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --apply'
      mode=apply
      mode_seen=1
      shift
      ;;
    --helper-source|--service-source|--timer-source|--config-source|--install-bin|--systemd-dir|--config-target)
      [ "$#" -ge 2 ] || die "$1 needs a path"
      case "$1" in
        --helper-source) helper_source=$2 ;;
        --service-source) service_source=$2 ;;
        --timer-source) timer_source=$2 ;;
        --config-source) config_source=$2 ;;
        --install-bin) install_bin_dir=$2 ;;
        --systemd-dir) systemd_dir=$2 ;;
        --config-target) config_target=$2 ;;
      esac
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die 'unknown option (use --help)'
      ;;
  esac
done

if [ "$mode" != dry-run ] && [ "$(id -u)" -ne 0 ]; then
  die "--$mode must run as root; use --dry-run for a non-root plan"
fi

safe_path() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..) die "$label must not contain '..': $path" ;;
  esac
  case "$path" in
    /|/etc|/opt|/var|/var/lib|/usr|/usr/local|/tmp|/run|/run/lock)
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

check_parent() {
  local path=$1 label=$2 probe metadata uid mode_bits
  probe=${path%/*}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ] && [ ! -e "$probe" ]; do
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  [ -d "$probe" ] && [ ! -L "$probe" ] || die "$label parent is not a directory: $probe"
  metadata=$(stat -c '%u:%a' -- "$probe" 2>/dev/null) || die "cannot inspect $label parent"
  uid=${metadata%%:*}
  mode_bits=$((8#${metadata#*:}))
  [ "$uid" = 0 ] || die "$label parent must be owned by uid 0"
  (( (mode_bits & 0022) == 0 )) || die "$label parent must not be group/other writable"
}

check_source_artifact() {
  local path=$1 label=$2
  safe_path "$path" "$label"
  [ -f "$path" ] && [ ! -L "$path" ] || die "$label must be a regular non-symlink file"
}

declare -A config_values=()
validate_config_source() {
  local line key value metadata
  config_values=()
  check_source_artifact "$config_source" config-source
  metadata=$(stat -c '%u:%g:%a' -- "$config_source" 2>/dev/null) ||
    die 'cannot inspect config-source metadata'
  [ "$metadata" = '0:0:600' ] ||
    die 'config-source must be root-owned with mode 0600; copy and customize the example first'
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ''|\#*) continue ;;
      *=*) ;;
      *) die 'config source contains a malformed assignment' ;;
    esac
    key=${line%%=*}
    value=${line#*=}
    case "$key" in
      TLS_DOMAIN|TLS_SOURCE_DIR|TLS_RUNTIME_DIR|COMPOSE_FILE|COMPOSE_ENV_FILE|COMPOSE_PROJECT|LAGRANGE_CODE_COMMIT|LOCK_FILE) ;;
      *) die 'config source contains an unsupported key' ;;
    esac
    [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || die 'config source contains an invalid key'
    case "$value" in
      *$'\r'*|*$'\n'*|*[[:space:]]*) die "config source contains whitespace: $key" ;;
    esac
    [ -z "${config_values[$key]+set}" ] || die "config source repeats a key: $key"
    config_values[$key]=$value
  done <"$config_source"
  for key in TLS_DOMAIN TLS_SOURCE_DIR TLS_RUNTIME_DIR COMPOSE_FILE \
    COMPOSE_ENV_FILE COMPOSE_PROJECT LAGRANGE_CODE_COMMIT LOCK_FILE; do
    [ -n "${config_values[$key]:-}" ] || die "config source is missing: $key"
  done
  [ "${config_values[TLS_DOMAIN]}" = 'l1nnx-sh.taild74a33.ts.net' ] ||
    die 'config source TLS_DOMAIN does not match the fixed expected domain'
  [[ "${config_values[COMPOSE_PROJECT]}" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,62}$ ]] ||
    die 'config source COMPOSE_PROJECT has an unsafe shape'
  [[ "${config_values[LAGRANGE_CODE_COMMIT]}" =~ ^[0-9a-f]{40}$ ]] ||
    die 'config source LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
  [ "${config_values[LAGRANGE_CODE_COMMIT]}" != 0000000000000000000000000000000000000000 ] ||
    die 'config source LAGRANGE_CODE_COMMIT must not be all zeroes'
  for key in TLS_SOURCE_DIR TLS_RUNTIME_DIR COMPOSE_FILE COMPOSE_ENV_FILE LOCK_FILE; do
    safe_path "${config_values[$key]}" "$key"
  done
}

validate_inputs() {
  check_source_artifact "$helper_source" helper-source
  check_source_artifact "$service_source" service-source
  check_source_artifact "$timer_source" timer-source
  validate_config_source
  safe_path "$install_bin_dir" install-bin-directory
  safe_path "$systemd_dir" systemd-directory
  safe_path "$config_target" config-target
  check_parent "$install_bin_dir/renew-tailscale-tls.sh" install-bin-target
  check_parent "$systemd_dir/lagrange-tailscale-tls-renewal.service" systemd-target
  check_parent "$config_target" config-target
}

target_paths() {
  helper_target=$install_bin_dir/renew-tailscale-tls.sh
  service_target=$systemd_dir/lagrange-tailscale-tls-renewal.service
  timer_target=$systemd_dir/lagrange-tailscale-tls-renewal.timer
}

check_installed_target() {
  local source=$1 target=$2 expected_mode=$3 label=$4
  [ -f "$target" ] && [ ! -L "$target" ] || die "$label is missing or not a regular file"
  [ "$(stat -c '%u:%g:%a' -- "$target" 2>/dev/null)" = "0:0:$expected_mode" ] ||
    die "$label has unsafe owner or mode"
  cmp -s -- "$source" "$target" || die "$label differs from the approved source"
}

print_plan() {
  echo 'TLS_RENEWAL_INSTALL_PLAN mode=dry-run'
  echo "  helper=$helper_source -> $helper_target"
  echo "  service=$service_source -> $service_target"
  echo "  timer=$timer_source -> $timer_target"
  echo "  config=$config_source -> $config_target (root:root 0600)"
  echo '  --apply copies artifacts atomically, daemon-reloads, and enables (but does not start) the timer'
  echo '  no certificate issuance, Docker call, service start, deletion, or KIS/Auth0/database operation'
}

target_paths
safe_path "$config_target" config-target
if [ "$mode" = dry-run ]; then
  print_plan
  echo 'DRY_RUN: no host files or systemd state changed'
  exit 0
fi

validate_inputs

if [ "$mode" = check ]; then
  check_installed_target "$helper_source" "$helper_target" 755 helper-target
  check_installed_target "$service_source" "$service_target" 644 service-target
  check_installed_target "$timer_source" "$timer_target" 644 timer-target
  [ -f "$config_target" ] && [ ! -L "$config_target" ] || die 'config-target is missing'
  [ "$(stat -c '%u:%g:%a' -- "$config_target")" = '0:0:600' ] || die 'config-target has unsafe owner or mode'
  validate_config_source
  cmp -s -- "$config_source" "$config_target" || die 'config-target differs from approved config source'
  echo 'TLS_RENEWAL_INSTALL_CHECK: PASS'
  exit 0
fi

if [ -e "$config_target" ] || [ -L "$config_target" ]; then
  die 'config-target already exists; refusing to overwrite protected configuration'
fi

command -v systemctl >/dev/null 2>&1 || die 'systemctl is required for --apply'
install -d -o root -g root -m 0755 -- "$install_bin_dir" || die 'cannot create install-bin directory'

copy_atomic() {
  local source=$1 target=$2 mode_bits=$3 staging
  [ ! -L "$target" ] || die 'refusing to replace a symlinked install target'
  staging=$(mktemp -- "${target%/*}/.lagrange-tls-install.XXXXXX") ||
    die 'cannot create install staging file'
  if ! cp --no-dereference -- "$source" "$staging"; then
    rm -f -- "$staging"
    die 'cannot stage installation artifact'
  fi
  chown root:root -- "$staging" || die 'cannot set installation owner'
  chmod "$mode_bits" -- "$staging" || die 'cannot set installation mode'
  mv -T -- "$staging" "$target" || die 'cannot atomically install artifact'
}

copy_atomic "$helper_source" "$helper_target" 0755
copy_atomic "$service_source" "$service_target" 0644
copy_atomic "$timer_source" "$timer_target" 0644
copy_atomic "$config_source" "$config_target" 0600

systemctl daemon-reload >/dev/null 2>&1 || die 'systemd daemon-reload failed; artifacts remain installed'
systemctl enable lagrange-tailscale-tls-renewal.timer >/dev/null 2>&1 ||
  die 'systemd timer enable failed; artifacts remain installed'
echo 'TLS_RENEWAL_INSTALL: PASS (artifacts installed; timer enabled but not started; no certificate was issued)'
