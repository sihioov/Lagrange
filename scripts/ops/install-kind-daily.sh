#!/usr/bin/env bash
# Coordinator-facing KIND daily artifact installer and readiness check.
#
# This is deliberately independent of systemd activation: --apply installs
# files only and never calls systemctl, starts a unit, installs npm packages,
# downloads a browser, or changes a secret. The capture source and Playwright
# browser directory must already be provisioned and are checked as immutable
# inputs. The coordinator owns the host-level invocation and activation review.
set -euo pipefail
umask 077

script_dir=$(cd "$(dirname -- "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
mode=dry-run
release_root=
capture_source=
browser_source=
install_root=/opt/lagrange
systemd_dir=/etc/systemd/system
env_file=/etc/lagrange/kind-daily.env

usage() {
  cat <<'EOF'
Usage: install-kind-daily.sh [--dry-run|--preflight|--check|--apply]
       [--release-root /opt/lagrange/releases/<40-hex-commit>]
       [--capture-source PATH] [--browser-source PATH]
       [--install-root /opt/lagrange]
       [--systemd-dir /etc/systemd/system]
       [--env-file /etc/lagrange/kind-daily.env]

--preflight/--dry-run validate a release, an already-installed KIND capture
tree (including node_modules/playwright), and a pre-provisioned Playwright
browser directory without changing the host.
--check is read-only and verifies the installed /opt/lagrange layout plus the
coordinator-provisioned 0600 entitlement environment file.
--apply installs the wrapper, three release bins, capture tree, browser tree,
and the manual oneshot service with no-clobber targets. It never runs systemd, npm,
cargo, a browser, a provider, a database, or a secret provisioner.
EOF
}

die() {
  printf 'KIND_DAILY_INSTALL status=blocked reason=%s\n' "$1" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) mode=dry-run; shift ;;
    --preflight) mode=preflight; shift ;;
    --check) mode=check; shift ;;
    --apply) mode=apply; shift ;;
    --release-root)
      [ "$#" -ge 2 ] || die 'release_root_value_missing'
      [ -z "$release_root" ] || die 'release_root_repeated'
      release_root=$2
      shift 2
      ;;
    --capture-source)
      [ "$#" -ge 2 ] || die 'capture_source_value_missing'
      [ -z "$capture_source" ] || die 'capture_source_repeated'
      capture_source=$2
      shift 2
      ;;
    --browser-source)
      [ "$#" -ge 2 ] || die 'browser_source_value_missing'
      [ -z "$browser_source" ] || die 'browser_source_repeated'
      browser_source=$2
      shift 2
      ;;
    --install-root)
      [ "$#" -ge 2 ] || die 'install_root_value_missing'
      install_root=$2
      shift 2
      ;;
    --systemd-dir)
      [ "$#" -ge 2 ] || die 'systemd_dir_value_missing'
      systemd_dir=$2
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die 'env_file_value_missing'
      env_file=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die 'unknown_argument' ;;
  esac
done

safe_path() {
  local path=$1 label=$2 probe
  case "$path" in
    /*) ;;
    *) die 'path_not_absolute' ;;
  esac
  case "$path" in
    */../*|*/..|../*|..|*$'\n'*|*$'\r'*) die 'unsafe_path_shape' ;;
  esac
  probe=$path
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die 'path_traverses_symlink'
    probe=$(dirname -- "$probe")
  done
}

require_dir() {
  local path=$1 label=$2
  [ -d "$path" ] && [ ! -L "$path" ] || die 'required_directory_missing_or_unsafe'
}

require_file() {
  local path=$1 label=$2
  [ -f "$path" ] && [ ! -L "$path" ] || die 'required_file_missing_or_unsafe'
}

require_no_write_tree() {
  local path=$1 label=$2
  if find "$path" -type l -print -quit | grep -q .; then
    die 'tree_contains_symlink'
  fi
  if find "$path" -type f -perm /022 -print -quit | grep -q .; then
    die 'tree_contains_group_or_other_writable_file'
  fi
  if find "$path" -type d -perm /022 -print -quit | grep -q .; then
    die 'tree_contains_group_or_other_writable_directory'
  fi
}

require_root_owned_no_write() {
  local path=$1 metadata uid mode_bits
  [ -e "$path" ] && [ ! -L "$path" ] || die 'required_path_missing_or_unsafe'
  metadata=$(stat -c '%u:%a' -- "$path") || die 'path_metadata_unreadable'
  uid=${metadata%%:*}
  mode_bits=${metadata#*:}
  [ "$uid" = 0 ] || die 'path_not_root_owned'
  (( (8#$mode_bits & 0022) == 0 )) || die 'path_group_or_other_writable'
}

require_root_owned_private_parent() {
  local path=$1 label=$2 parent
  parent=$(dirname -- "$path")
  while :; do
    [ ! -L "$parent" ] || die 'parent_symlink'
    if [ -e "$parent" ]; then
      [ -d "$parent" ] || die 'parent_not_directory'
      require_root_owned_no_write "$parent"
    fi
    [ "$parent" = / ] && break
    parent=$(dirname -- "$parent")
  done
}

require_node() {
  local node_version
  command -v node >/dev/null 2>&1 || die 'node_missing'
  node_version=$(node --version 2>/dev/null) || die 'node_version_unreadable'
  [[ "$node_version" =~ ^v24\.[0-9]+\.[0-9]+$ ]] || die 'node_must_be_24_x'
}

check_source_inputs() {
  [ -n "$release_root" ] || die 'release_root_required'
  [ -n "$capture_source" ] || die 'capture_source_required'
  [ -n "$browser_source" ] || die 'browser_source_required'
  safe_path "$release_root" release_root
  safe_path "$capture_source" capture_source
  safe_path "$browser_source" browser_source
  safe_path "$systemd_dir" systemd_dir
  require_dir "$release_root" release_root
  release_basename=$(basename -- "$release_root")
  [[ "$release_basename" =~ ^[0-9a-f]{40}$ ]] || die 'release_root_basename_must_be_commit'
  require_dir "$capture_source" capture_source
  require_dir "$browser_source" browser_source
  require_file "$release_root/scripts/ops/kind-daily.sh" wrapper_source
  [ -x "$release_root/scripts/ops/kind-daily.sh" ] || die 'wrapper_source_not_executable'
  require_file "$repo_root/deploy/systemd/lagrange-kind-daily.service" service_source
  for binary in kind-raw kind-correction-raw kind-normalize; do
    require_file "$release_root/target/release/$binary" binary_source
    [ -x "$release_root/target/release/$binary" ] || die 'release_binary_not_executable'
  done
  for source_file in capture.mjs capture-correction.mjs capture-logic.mjs \
      correction-capture-logic.mjs correction-output.mjs package.json; do
    require_file "$capture_source/$source_file" capture_source_file
  done
  require_file "$capture_source/node_modules/playwright/package.json" playwright_package
  require_no_write_tree "$capture_source" capture_source
  require_no_write_tree "$browser_source" browser_source
  if ! find "$browser_source" -type f -path '*/chrome-linux/chrome' -perm -111 -print -quit | grep -q .; then
    die 'playwright_chromium_binary_missing'
  fi
  require_node
}

check_installed_layout() {
  safe_path "$install_root" install_root
  safe_path "$systemd_dir" systemd_dir
  safe_path "$env_file" env_file
  require_dir "$install_root" install_root
  require_root_owned_no_write "$install_root"
  require_root_owned_no_write "$install_root/bin"
  require_root_owned_no_write "$install_root/data-pipelines"
  require_root_owned_no_write "$systemd_dir"
  require_file "$install_root/bin/kind-daily.sh" installed_wrapper
  [ "$(stat -c '%u:%g:%a' -- "$install_root/bin/kind-daily.sh")" = 0:0:755 ] ||
    die 'installed_wrapper_metadata_mismatch'
  for binary in kind-raw kind-correction-raw kind-normalize; do
    require_file "$install_root/bin/$binary" installed_binary
    [ "$(stat -c '%u:%g:%a' -- "$install_root/bin/$binary")" = 0:0:755 ] ||
      die 'installed_binary_metadata_mismatch'
  done
  require_dir "$install_root/data-pipelines/kind-capture" installed_capture_tree
  require_no_write_tree "$install_root/data-pipelines/kind-capture" installed_capture_tree
  require_file "$install_root/data-pipelines/kind-capture/node_modules/playwright/package.json" installed_playwright_package
  require_dir "$install_root/playwright-browsers" installed_browser_tree
  require_no_write_tree "$install_root/playwright-browsers" installed_browser_tree
  if ! find "$install_root/playwright-browsers" -type f -path '*/chrome-linux/chrome' -perm -111 -print -quit | grep -q .; then
    die 'installed_playwright_chromium_binary_missing'
  fi
  require_file "$systemd_dir/lagrange-kind-daily.service" installed_unit
  [ "$(stat -c '%u:%g:%a' -- "$systemd_dir/lagrange-kind-daily.service")" = 0:0:644 ] ||
    die 'installed_unit_metadata_mismatch'
  require_file "$env_file" entitlement_env
  [ "$(stat -c '%u:%g:%a' -- "$env_file")" = 0:0:600 ] || die 'entitlement_env_metadata_mismatch'
}

if [ "$mode" = check ]; then
  [ "$(id -u)" -eq 0 ] || die 'check_requires_root'
  check_installed_layout
  echo 'KIND_DAILY_INSTALL_CHECK: PASS installed_unactivated'
  exit 0
fi

check_source_inputs

if [ "$mode" = dry-run ]; then
  printf 'KIND_DAILY_INSTALL_PLAN release=%s\n' "$release_root"
  printf 'KIND_DAILY_INSTALL_PLAN capture_source=%s (node_modules/playwright required)\n' "$capture_source"
  printf 'KIND_DAILY_INSTALL_PLAN browser_source=%s (pre-provisioned Chromium required)\n' "$browser_source"
  printf 'KIND_DAILY_INSTALL_PLAN targets=/opt/lagrange/bin,/opt/lagrange/data-pipelines/kind-capture,/opt/lagrange/playwright-browsers\n'
  printf 'KIND_DAILY_INSTALL_PLAN systemd_service=%s (copied only; never activated here)\n' "$systemd_dir/lagrange-kind-daily.service"
  printf 'KIND_DAILY_INSTALL_PLAN no_network=true no_npm=true no_systemd=true no_secret_write=true\n'
  exit 0
fi

if [ "$mode" = preflight ]; then
  echo 'KIND_DAILY_INSTALL_PREFLIGHT: PASS source_and_runtime_assets'
  exit 0
fi

[ "$(id -u)" -eq 0 ] || die 'apply_requires_root'
safe_path "$install_root" install_root
safe_path "$systemd_dir" systemd_dir
require_root_owned_private_parent "$install_root/bin/kind-daily.sh" install_bin
require_root_owned_private_parent "$systemd_dir/lagrange-kind-daily.service" systemd_dir

for target in "$install_root/bin/kind-daily.sh" "$install_root/bin/kind-raw" \
    "$install_root/bin/kind-correction-raw" "$install_root/bin/kind-normalize" \
    "$install_root/data-pipelines/kind-capture" "$install_root/playwright-browsers" \
    "$systemd_dir/lagrange-kind-daily.service"; do
  [ ! -e "$target" ] && [ ! -L "$target" ] || die 'target_exists_no_clobber'
done

install -d -o 0 -g 0 -m 0755 -- "$install_root" "$install_root/bin" "$install_root/data-pipelines"
install -o 0 -g 0 -m 0755 -- "$release_root/scripts/ops/kind-daily.sh" "$install_root/bin/kind-daily.sh"
install -o 0 -g 0 -m 0755 -- "$release_root/target/release/kind-raw" "$install_root/bin/kind-raw"
install -o 0 -g 0 -m 0755 -- "$release_root/target/release/kind-correction-raw" "$install_root/bin/kind-correction-raw"
install -o 0 -g 0 -m 0755 -- "$release_root/target/release/kind-normalize" "$install_root/bin/kind-normalize"
cp -a -- "$capture_source" "$install_root/data-pipelines/kind-capture"
cp -a -- "$browser_source" "$install_root/playwright-browsers"
chown -R 0:0 -- "$install_root/data-pipelines/kind-capture" "$install_root/playwright-browsers"
find "$install_root/data-pipelines/kind-capture" "$install_root/playwright-browsers" -type f -exec chmod go-w {} +
find "$install_root/data-pipelines/kind-capture" "$install_root/playwright-browsers" -type d -exec chmod go-w {} +
install -o 0 -g 0 -m 0644 -- "$repo_root/deploy/systemd/lagrange-kind-daily.service" "$systemd_dir/lagrange-kind-daily.service"

echo 'KIND_DAILY_INSTALL_APPLY: PASS installed-not-activated'
