#!/usr/bin/env bash
# Generate the four production cryptographic source secrets.
#
# The default mode is a non-mutating plan. --check is root-only and read-only;
# --apply is explicit, root-only, and writes independently generated values
# only when every target is absent. Values never appear in operator output.
set -euo pipefail

default_source_dir=/etc/lagrange/secrets
source_dir=$default_source_dir
mode=dry-run
mode_seen=0
staging_dir=

declare -a secret_names=(
  session_secret
  csrf_secret
  cursor_secret
  backup_encryption_key
)
declare -a installed_targets=()
declare -a installed_signatures=()
declare -a check_failures=()

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-crypto-secrets.sh [--dry-run|--check|--apply]
       [--source-dir ABSOLUTE_PATH]

Modes:
  --dry-run              Print the four-file plan without changing the host
                         (default; safe to run as a non-root user).
  --check                Validate the existing files read-only; requires root.
                         It never prints secret values or hashes.
  --apply                Generate and install four distinct values; requires
                         root and never overwrites an existing target.
  --source-dir PATH      Override /etc/lagrange/secrets for an isolated host
                         or test. PATH must be absolute and cannot contain
                         '..' or a symlinked ancestor.

All four files contain exactly 64 lowercase hexadecimal characters: an
independently generated 256-bit value without a trailing newline. Files are
root:root mode 0600. No external network call is made by this command.
EOF
}

die() {
  echo "provision-crypto-secrets: $*" >&2
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
    --source-dir)
      [ "$#" -ge 2 ] || die '--source-dir needs an absolute path'
      source_dir=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1 (use --help)"
      ;;
  esac
done

if [ "$mode" = apply ] && [ "$(id -u)" -ne 0 ]; then
  die '--apply must run as root; use --dry-run for a non-root plan'
fi
if [ "$mode" = check ] && [ "$(id -u)" -ne 0 ]; then
  die '--check must run as root; use --dry-run for a non-root plan'
fi

while [ "$source_dir" != / ] && [[ "$source_dir" == */ ]]; do
  source_dir=${source_dir%/}
done

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
    /|/etc|/opt|/var|/var/lib|/usr|/usr/local|/tmp)
      die "$label is too broad: $path"
      ;;
  esac

  # Check every existing component, including missing-path components, so a
  # later mkdir cannot turn a symlinked ancestor into a write target.
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

target_path() {
  printf '%s/%s' "$source_dir" "$1"
}

target_present() {
  local target=$1
  [ -e "$target" ] || [ -L "$target" ]
}

source_metadata() {
  local metadata source_uid source_mode source_mode_bits

  [ -d "$source_dir" ] || die "source directory is not a regular directory: $source_dir"
  [ ! -L "$source_dir" ] || die "source directory must not be a symlink: $source_dir"
  metadata=$(stat -c '%u:%a' -- "$source_dir" 2>/dev/null) ||
    die "cannot inspect source directory metadata: $source_dir"
  source_uid=${metadata%%:*}
  source_mode=${metadata#*:}
  source_mode_bits=$((8#$source_mode))
  [ "$source_uid" = 0 ] ||
    die "source directory must be owned by uid 0: $source_dir"
  (( (source_mode_bits & 0022) == 0 )) ||
    die "source directory must not be group/other writable: $source_dir"
}

check_targets_absent() {
  local name target
  for name in "${secret_names[@]}"; do
    target=$(target_path "$name")
    safe_path "$target" "secret target $name"
    if target_present "$target"; then
      echo "  existing target: $target" >&2
      return 1
    fi
  done
  return 0
}

placeholder_pattern='placeholder|example|todo|change[-_ ]*me|change[-_ ]*this|replace[-_ ]*(me|this|with)|your[-_ ]*(client[-_ ]*)?secret|secret[-_ ]*here|auth0[-_ ]*client[-_ ]*secret|<[^>]+>|\$\{[^}]+\}'

is_valid_secret_file() {
  local target=$1 byte_count

  byte_count=$(wc -c <"$target" 2>/dev/null) || return 1
  [ "$byte_count" -eq 64 ] || return 1
  LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$target" || return 1
  ! LC_ALL=C grep -Eiq -- "$placeholder_pattern" "$target"
}

report_check_failure() {
  local name=$1 reason=$2
  echo "CRYPTO_SECRET_CHECK: FAIL $name: $reason" >&2
  check_failures+=1
}

check_existing_secret() {
  local name=$1 target metadata

  target=$(target_path "$name")
  safe_path "$target" "secret target $name"
  if ! target_present "$target"; then
    report_check_failure "$name" 'missing file'
    return 0
  fi
  if [ ! -f "$target" ] || [ -L "$target" ]; then
    report_check_failure "$name" 'must be a regular non-symlink file'
    return 0
  fi
  if ! metadata=$(stat -c '%u:%g:%a' -- "$target" 2>/dev/null); then
    report_check_failure "$name" 'cannot inspect ownership or mode'
    return 0
  fi
  [ "$metadata" = '0:0:600' ] ||
    report_check_failure "$name" 'must be owned by root:root with mode 0600'
  if ! is_valid_secret_file "$target"; then
    report_check_failure "$name" \
      'must contain exactly 64 lowercase hex characters with no newline or placeholder'
  fi
}

check_existing_secrets() {
  local name left right

  check_failures=()
  if [ ! -d "$source_dir" ] || [ -L "$source_dir" ]; then
    report_check_failure source-directory 'must be a regular non-symlink directory'
  else
    source_metadata_check=0
    if ! source_metadata_check=$(stat -c '%u:%a' -- "$source_dir" 2>/dev/null); then
      report_check_failure source-directory 'cannot inspect ownership or mode'
    else
      source_uid=${source_metadata_check%%:*}
      source_mode=${source_metadata_check#*:}
      source_mode_bits=$((8#$source_mode))
      [ "$source_uid" = 0 ] ||
        report_check_failure source-directory 'must be owned by uid 0'
      (( (source_mode_bits & 0022) == 0 )) ||
        report_check_failure source-directory 'must not be group/other writable'
    fi
  fi

  for name in "${secret_names[@]}"; do
    check_existing_secret "$name"
  done

  if [ "${#check_failures[@]}" -eq 0 ]; then
    for ((left = 0; left < ${#secret_names[@]}; left++)); do
      for ((right = left + 1; right < ${#secret_names[@]}; right++)); do
        if cmp -s -- \
          "$(target_path "${secret_names[left]}")" \
          "$(target_path "${secret_names[right]}")"; then
          echo "CRYPTO_SECRET_CHECK: FAIL ${secret_names[left]},${secret_names[right]}: values are not distinct" >&2
          check_failures+=1
        fi
      done
    done
  fi

  if [ "${#check_failures[@]}" -ne 0 ]; then
    return 1
  fi
  echo 'CRYPTO_SECRET_CHECK: PASS'
  return 0
}

print_plan() {
  local name
  echo "CRYPTO_SECRET_PROVISION mode=$mode"
  echo "  source=$source_dir"
  echo '  generate four independent 256-bit values with openssl rand -hex 32'
  echo '  final files: owner=root:root mode=0600, exactly 64 lowercase hex bytes'
  for name in "${secret_names[@]}"; do
    echo "  ensure $source_dir/$name"
  done
  echo '  values are never printed; existing targets are never overwritten'
}

safe_path "$source_dir" source-directory

if [ "$mode" = dry-run ]; then
  print_plan
  if check_targets_absent; then
    echo 'DRY_RUN: no files created'
  else
    echo 'DRY_RUN: no files created (apply would refuse existing targets)' >&2
  fi
  exit 0
fi

if [ "$mode" = check ]; then
  if check_existing_secrets; then
    exit 0
  fi
  exit 1
fi

# Explicit root apply. Create only the final source directory beneath an
# existing safe parent; never create broad or missing ancestor paths.
if [ ! -e "$source_dir" ]; then
  source_parent=${source_dir%/*}
  [ -n "$source_parent" ] || source_parent=/
  safe_path "$source_parent" source-directory-parent
  [ -d "$source_parent" ] || die "source directory parent is not a regular directory: $source_parent"
  [ ! -L "$source_parent" ] || die "source directory parent must not be a symlink: $source_parent"
  parent_metadata=$(stat -c '%u:%a' -- "$source_parent" 2>/dev/null) ||
    die "cannot inspect source directory parent metadata: $source_parent"
  parent_uid=${parent_metadata%%:*}
  parent_mode=${parent_metadata#*:}
  parent_mode_bits=$((8#$parent_mode))
  [ "$parent_uid" = 0 ] ||
    die "source directory parent must be owned by uid 0: $source_parent"
  (( (parent_mode_bits & 0022) == 0 )) ||
    die "source directory parent must not be group/other writable: $source_parent"
  install -d -o root -g root -m 0750 -- "$source_dir" ||
    die "cannot create source directory: $source_dir"
fi

source_metadata
check_targets_absent || die 'refusing to overwrite existing crypto secret; no files were changed'

command -v openssl >/dev/null 2>&1 || die 'openssl is required for --apply'
umask 077
staging_dir=$(mktemp -d -- "$source_dir/.lagrange-crypto-secrets.XXXXXX") ||
  die "cannot create private staging directory under: $source_dir"
chmod 0700 -- "$staging_dir"

cleanup() {
  local status=$?
  set +e
  if [ "$status" -ne 0 ]; then
    local i target expected actual
    for ((i = ${#installed_targets[@]} - 1; i >= 0; i--)); do
      target=${installed_targets[i]}
      expected=${installed_signatures[i]}
      actual=$(stat -c '%d:%i' -- "$target" 2>/dev/null || true)
      if [ -n "$expected" ] && [ "$actual" = "$expected" ]; then
        rm -f -- "$target" 2>/dev/null || true
      fi
    done
  fi
  if [ -n "$staging_dir" ] && [ -d "$staging_dir" ]; then
    rm -rf -- "$staging_dir" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT

generate_staged_secret() {
  local name=$1 raw_file=$staging_dir/.$1.raw file=$staging_dir/$1

  openssl rand -hex 32 | tr -d '\r\n' >"$raw_file" ||
    die "openssl failed while generating $name"
  install -o root -g root -m 0600 -- "$raw_file" "$file" ||
    die "cannot install staged secret: $name"
  rm -f -- "$raw_file"
  [ "$(stat -c '%u:%g:%a' -- "$file")" = '0:0:600' ] ||
    die "staged secret has unsafe ownership or mode: $name"
  is_valid_secret_file "$file" ||
    die "generated $name is not exactly 64 lowercase hex characters"
}

for name in "${secret_names[@]}"; do
  generate_staged_secret "$name"
done

for ((i = 0; i < ${#secret_names[@]}; i++)); do
  for ((j = i + 1; j < ${#secret_names[@]}; j++)); do
    if cmp -s -- "$staging_dir/${secret_names[i]}" "$staging_dir/${secret_names[j]}"; then
      die "generated crypto source values are not distinct: ${secret_names[i]} and ${secret_names[j]}"
    fi
  done
done

# Hard-linking a staged inode is atomic and cannot clobber a target that
# appears after the preflight. All staged files are on the target filesystem.
for name in "${secret_names[@]}"; do
  staged="$staging_dir/$name"
  target=$(target_path "$name")
  stage_signature=$(stat -c '%d:%i' -- "$staged")
  if ! ln -T -- "$staged" "$target"; then
    die "target appeared or could not be installed: $target"
  fi
  installed_targets+=("$target")
  installed_signatures+=("$stage_signature")
  [ "$(stat -c '%d:%i:%u:%g:%a' -- "$target")" = \
    "${stage_signature}:0:0:600" ] ||
    die "installed secret has unsafe metadata: $name"
done

for name in "${secret_names[@]}"; do
  rm -f -- "$staging_dir/$name"
done
rmdir -- "$staging_dir"
staging_dir=

echo "CRYPTO_SECRET_PROVISION mode=apply source=$source_dir"
echo 'APPLY: generated exactly four distinct 256-bit crypto source secret files'
echo 'APPLY: values were not printed; no external network call was made'
