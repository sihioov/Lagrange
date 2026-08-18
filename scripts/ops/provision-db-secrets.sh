#!/usr/bin/env bash
# Generate the seven production PostgreSQL source secret files.
#
# The default mode is a non-mutating plan.  --check is a root-only read-only
# validation of the existing DB source files; --apply is deliberately explicit
# and root-only because it writes root-owned credentials under the host secret
# directory.  This script creates only the DB source files; runtime copies and
# database roles are separate operator-approved steps.
set -euo pipefail

default_source_dir=/etc/lagrange/secrets
source_dir=${LAGRANGE_SECRET_SOURCE_DIR:-$default_source_dir}
mode=dry-run
mode_seen=0
staging_dir=

declare -a secret_names=(
  postgres_password
  db_migration_owner_password
  db_app_password
  db_worker_password
  db_audit_password
  db_research_password
  db_admin_password
)
declare -a installed_targets=()
declare -a installed_signatures=()
declare -a check_failures=()

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-db-secrets.sh [--dry-run|--check|--apply]
       [--source-dir ABSOLUTE_PATH]

Modes:
  --dry-run              Print the seven-file plan without changing the host
                         (default; safe to run as a non-root user).
  --check                Validate the existing seven source files read-only;
                         requires root and never prints values or hashes.
  --apply                Generate the files; requires root and never overwrites
                         an existing target.
  --source-dir PATH      Override /etc/lagrange/secrets for an isolated host
                         or test. PATH must be absolute and cannot contain
                         '..' or a symlinked ancestor.

The source directory may also be set with LAGRANGE_SECRET_SOURCE_DIR.  Each
file contains 32 random bytes encoded as exactly 64 lowercase hexadecimal
characters, is mode 0600 and owned by root:root.  Secret values are never
printed.  This command does not create runtime copies or database roles.
EOF
}

die() {
  echo "provision-db-secrets: $*" >&2
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

  # Check the path itself and every existing ancestor.  Checking missing
  # components is intentional: a later mkdir must not be allowed to turn a
  # symlinked ancestor into a write target.
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

check_targets() {
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

report_check_failure() {
  local name=$1 reason=$2
  echo "DB_SECRET_CHECK: FAIL $name: $reason" >&2
  check_failures+=("$name")
}

check_existing_secret() {
  local name=$1 target metadata byte_count
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
  if [ "$metadata" != '0:0:600' ]; then
    report_check_failure "$name" 'must be owned by root:root with mode 0600'
  fi
  if ! byte_count=$(wc -c <"$target" 2>/dev/null); then
    report_check_failure "$name" 'cannot read file length'
    return 0
  fi
  if [ "$byte_count" -ne 64 ]; then
    report_check_failure "$name" 'must contain exactly 64 bytes with no newline'
  fi
  if ! LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$target"; then
    report_check_failure "$name" 'must contain one line of lowercase hexadecimal'
  fi

}

check_existing_secrets() {
  local name left right

  if [ ! -d "$source_dir" ] || [ -L "$source_dir" ]; then
    echo "DB_SECRET_CHECK: FAIL source-directory: $source_dir is not a regular directory" >&2
    return 1
  fi

  for name in "${secret_names[@]}"; do
    check_existing_secret "$name"
  done

  # Only compare files that passed every shape and metadata gate.  A malformed
  # or missing file is already reported by filename above; cmp never prints a
  # credential or a derived hash.
  if [ "${#check_failures[@]}" -eq 0 ]; then
    for ((left = 0; left < ${#secret_names[@]}; left++)); do
      for ((right = left + 1; right < ${#secret_names[@]}; right++)); do
        if cmp -s -- \
          "$(target_path "${secret_names[left]}")" \
          "$(target_path "${secret_names[right]}")"; then
          echo "DB_SECRET_CHECK: FAIL ${secret_names[left]},${secret_names[right]}: values are not distinct" >&2
          check_failures+=("${secret_names[left]},${secret_names[right]}")
        fi
      done
    done
  fi

  if [ "${#check_failures[@]}" -ne 0 ]; then
    return 1
  fi
  echo 'DB_SECRET_CHECK: PASS'
  return 0
}

print_plan() {
  local name
  echo "DB_SECRET_PROVISION mode=$mode"
  echo "  source=$source_dir"
  echo "  generate exactly seven distinct 256-bit values with openssl rand -hex 32"
  echo "  final files: owner=root:root mode=0600, one-line lowercase hex"
  for name in "${secret_names[@]}"; do
    echo "  ensure $source_dir/$name"
  done
  echo "  values are never printed; existing targets are never overwritten"
}

safe_path "$source_dir" source-directory

if [ "$mode" = dry-run ]; then
  print_plan
  if check_targets; then
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

command -v openssl >/dev/null 2>&1 || die 'openssl is required for --apply'

# Do not alter an existing directory's owner or mode.  The host provisioning
# step normally creates this directory; creating it here is only a safe
# bootstrap for an isolated host/test path.
if [ ! -e "$source_dir" ]; then
  install -d -o root -g root -m 0750 -- "$source_dir" ||
    die "cannot create source directory: $source_dir"
fi
safe_path "$source_dir" source-directory
[ -d "$source_dir" ] || die "source path is not a directory: $source_dir"
[ ! -L "$source_dir" ] || die "source directory must not be a symlink: $source_dir"

source_uid=$(stat -c '%u' -- "$source_dir") ||
  die "cannot inspect source directory ownership: $source_dir"
source_mode=$(stat -c '%a' -- "$source_dir") ||
  die "cannot inspect source directory mode: $source_dir"
[ "$source_uid" = 0 ] ||
  die "source directory must be owned by uid 0: $source_dir"
source_mode_bits=$((8#$source_mode))
(( (source_mode_bits & 0022) == 0 )) ||
  die "source directory must not be group/other writable: $source_dir"

# Check every destination before generating anything.  This is the important
# no-overwrite/no-partial-generation fence for rerunning production setup.
if ! check_targets; then
  die 'refusing to overwrite existing DB source secret; no files were changed'
fi

umask 077
staging_dir=$(mktemp -d -- "$source_dir/.lagrange-db-secrets.XXXXXX") ||
  die "cannot create private staging directory under: $source_dir"
chmod 0700 -- "$staging_dir"

cleanup() {
  local status=$?
  set +e

  if [ "$status" -ne 0 ]; then
    local i target expected actual
    for ((i=${#installed_targets[@]} - 1; i >= 0; i--)); do
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

  # tr removes OpenSSL's line terminator so consumers receive one logical
  # line with no embedded LF/CR.  pipefail makes an OpenSSL failure fatal.
  openssl rand -hex 32 | tr -d '\r\n' >"$raw_file" ||
    die "openssl failed while generating $name"
  install -o root -g root -m 0600 -- "$raw_file" "$file" ||
    die "cannot install staged secret: $name"
  rm -f -- "$raw_file"
  [ -f "$file" ] && [ ! -L "$file" ] || die "staged secret is not a regular file: $name"
  [ "$(stat -c '%u:%g:%a' -- "$file")" = '0:0:600' ] ||
    die "staged secret has unsafe ownership or mode: $name"
  [ "$(wc -c <"$file")" -eq 64 ] ||
    die "generated $name is not exactly 64 bytes"
  LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$file" ||
    die "generated $name is not one-line lowercase hex"
}

for name in "${secret_names[@]}"; do
  generate_staged_secret "$name"
done

# Keep the distinctness check silent: comparing files does not expose either
# a credential value or a derived hash in operator output.
for ((i = 0; i < ${#secret_names[@]}; i++)); do
  for ((j = i + 1; j < ${#secret_names[@]}; j++)); do
    if cmp -s "$staging_dir/${secret_names[i]}" "$staging_dir/${secret_names[j]}"; then
      die "generated DB source values are not distinct: ${secret_names[i]} and ${secret_names[j]}"
    fi
  done
done

# Hard-link each staged regular file into place.  ln without -f is an atomic
# no-clobber operation on the same filesystem.  If a concurrent target appears
# after the initial fence, the EXIT trap removes links created by this run only.
for name in "${secret_names[@]}"; do
  staged="$staging_dir/$name"
  target=$(target_path "$name")
  stage_signature=$(stat -c '%d:%i' -- "$staged")
  if ! ln -T -- "$staged" "$target"; then
    die "target appeared or could not be installed: $target"
  fi
  installed_targets+=("$target")
  installed_signatures+=("$stage_signature")
  [ -f "$target" ] && [ ! -L "$target" ] || die "installed secret is not a regular file: $name"
  [ "$(stat -c '%d:%i:%u:%g:%a' -- "$target")" = \
    "${stage_signature}:0:0:600" ] || die "installed secret has unsafe metadata: $name"
done

for name in "${secret_names[@]}"; do
  rm -f -- "$staging_dir/$name"
done
rmdir -- "$staging_dir"
staging_dir=

echo "DB_SECRET_PROVISION mode=apply source=$source_dir"
echo 'APPLY: generated exactly seven distinct DB source secret files'
echo 'APPLY: values were not printed; runtime-copy and database-role steps remain separate'
