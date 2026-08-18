#!/usr/bin/env bash
# Provision the production Auth0 confidential-client secret from a hidden
# terminal prompt or an explicitly named, protected legacy file.  The default
# mode is a non-mutating plan; --apply/--import-file are explicit, root-only,
# and never accept a secret through an argument or environment variable.  No
# Auth0 network/API call is made by this script.
set -euo pipefail

default_source_dir=/etc/lagrange/secrets
source_dir=$default_source_dir
target_name=auth0_client_secret
mode=dry-run
mode_seen=0
import_file=
staging_dir=
installed=0
installed_signature=

# These markers cover the repository's example and the common values operators
# accidentally paste into a production secret file.  The check is deliberately
# conservative: a value containing whitespace/control bytes is not accepted.
placeholder_pattern='placeholder|example|todo|change[-_ ]*me|change[-_ ]*this|replace[-_ ]*(me|this|with)|your[-_ ]*(client[-_ ]*)?secret|client[-_ ]*secret|auth0[-_ ]*client[-_ ]*secret|<[^>]+>|\$\{[^}]+\}'

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-auth0-secret.sh [--dry-run|--check|--apply]
       [--import-file ABSOLUTE_PATH] [--source-dir ABSOLUTE_PATH]

Modes:
  --dry-run              Print the one-file plan without changing the host
                         (default; safe to run as a non-root user).
  --check                Validate the existing file read-only; requires root.
                         Reports only metadata and safe shape reasons, never
                         the secret or a derived hash.
  --apply                Read and confirm the secret from a hidden interactive
                         terminal prompt; requires root.  Never overwrites an
                         existing target.
  --import-file PATH     Import an existing legacy secret file; requires root.
                         PATH is metadata/shape-checked and its value is never
                         printed or accepted through argv or the environment.
  --source-dir PATH      Override /etc/lagrange/secrets for an isolated host
                         or test. PATH must be absolute, contain no '..', and
                         have no symlinked ancestor.

The default target is /etc/lagrange/secrets/auth0_client_secret.  --apply and
--import-file write exactly one non-empty, printable, whitespace-free line as
root:root mode 0600, without a newline, using an atomic no-clobber install.
The secret is never read from argv or the environment, printed, logged, or
sent to Auth0; this command performs no Auth0 network/API verification.
EOF
}

die() {
  echo "provision-auth0-secret: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, --apply, or --import-file'
      mode=dry-run
      mode_seen=1
      shift
      ;;
    --check)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, --apply, or --import-file'
      mode=check
      mode_seen=1
      shift
      ;;
    --apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, --apply, or --import-file'
      mode=apply
      mode_seen=1
      shift
      ;;
    --import-file)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, --apply, or --import-file'
      [ "$#" -ge 2 ] || die '--import-file needs an absolute path'
      import_file=$2
      mode=import
      mode_seen=1
      shift 2
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
if [ "$mode" = import ] && [ "$(id -u)" -ne 0 ]; then
  die '--import-file must run as root; use --dry-run for a non-root plan'
fi

# A trailing slash is harmless but normalizing it keeps the target path and
# ancestor checks unambiguous.  '/' itself is rejected by safe_path below.
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

  # Check the path itself and every existing ancestor.  Missing components
  # are included so a later mkdir cannot turn a symlinked ancestor into a
  # write target.
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

target_path() {
  printf '%s/%s' "$source_dir" "$target_name"
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

check_target_absent() {
  local target
  target=$(target_path)
  safe_path "$target" "Auth0 secret target"
  if target_present "$target"; then
    echo "  existing target: $target" >&2
    return 1
  fi
  return 0
}

is_placeholder_value() {
  local value=$1
  LC_ALL=C grep -Eiq -- "$placeholder_pattern" <<<"$value"
}

validate_secret_value() {
  local value=$1

  [ -n "$value" ] || die 'Auth0 client secret must not be empty'
  case "$value" in
    *$'\r'*|*$'\n'*) die 'Auth0 client secret must not contain CR or LF' ;;
  esac
  case "$value" in
    *[![:print:]]*) die 'Auth0 client secret must contain printable characters only' ;;
  esac
  case "$value" in
    *[[:space:]]*) die 'Auth0 client secret must be one non-empty line without whitespace' ;;
  esac
  if is_placeholder_value "$value"; then
    die 'Auth0 client secret looks like a placeholder'
  fi
}

validate_import_file() {
  local metadata source_mode source_mode_bits byte_count

  safe_path "$import_file" 'legacy Auth0 secret source'
  if [ ! -e "$import_file" ]; then
    die "legacy Auth0 secret source is missing: $import_file"
  fi
  if [ ! -f "$import_file" ] || [ -L "$import_file" ]; then
    die 'legacy Auth0 secret source must be a regular non-symlink file'
  fi
  metadata=$(stat -c '%a' -- "$import_file" 2>/dev/null) ||
    die 'cannot inspect legacy Auth0 secret source mode'
  source_mode=$metadata
  source_mode_bits=$((8#$source_mode))
  (( (source_mode_bits & 0077) == 0 )) ||
    die 'legacy Auth0 secret source must not be group/other accessible'
  (( (source_mode_bits & 0400) != 0 )) ||
    die 'legacy Auth0 secret source must be readable by its owner'

  byte_count=$(wc -c <"$import_file" 2>/dev/null) ||
    die 'cannot inspect legacy Auth0 secret source length'
  [ "$byte_count" -gt 0 ] ||
    die 'legacy Auth0 secret source must not be empty'
  ! LC_ALL=C grep -q '[[:space:]]' -- "$import_file" ||
    die 'legacy Auth0 secret source must be one non-empty line without CR, LF, or whitespace'
  ! LC_ALL=C grep -q '[^[:print:]]' -- "$import_file" ||
    die 'legacy Auth0 secret source must contain printable characters only'
  ! LC_ALL=C grep -Eiq -- "$placeholder_pattern" "$import_file" ||
    die 'legacy Auth0 secret source looks like a placeholder'
}

report_check_failure() {
  local name=$1 reason=$2
  echo "AUTH0_SECRET_CHECK: FAIL $name: $reason" >&2
  check_failures+=1
}

check_source_directory() {
  local metadata source_uid source_mode source_mode_bits

  if [ ! -e "$source_dir" ]; then
    report_check_failure source-directory 'missing directory'
    return 0
  fi
  if [ ! -d "$source_dir" ] || [ -L "$source_dir" ]; then
    report_check_failure source-directory 'must be a regular non-symlink directory'
    return 0
  fi
  if ! metadata=$(stat -c '%u:%a' -- "$source_dir" 2>/dev/null); then
    report_check_failure source-directory 'cannot inspect ownership or mode'
    return 0
  fi
  source_uid=${metadata%%:*}
  source_mode=${metadata#*:}
  source_mode_bits=$((8#$source_mode))
  [ "$source_uid" = 0 ] ||
    report_check_failure source-directory 'must be owned by uid 0'
  (( (source_mode_bits & 0022) == 0 )) ||
    report_check_failure source-directory 'must not be group/other writable'
}

check_existing_target() {
  local target metadata byte_count

  target=$(target_path)
  safe_path "$target" 'Auth0 secret target'
  if ! target_present "$target"; then
    report_check_failure "$target_name" 'missing file'
    return 0
  fi
  if [ ! -f "$target" ] || [ -L "$target" ]; then
    report_check_failure "$target_name" 'must be a regular non-symlink file'
    return 0
  fi
  if ! metadata=$(stat -c '%u:%g:%a' -- "$target" 2>/dev/null); then
    report_check_failure "$target_name" 'cannot inspect ownership or mode'
    return 0
  fi
  [ "$metadata" = '0:0:600' ] ||
    report_check_failure "$target_name" 'must be owned by root:root with mode 0600'

  if ! byte_count=$(wc -c <"$target" 2>/dev/null); then
    report_check_failure "$target_name" 'cannot inspect byte length'
    return 0
  elif [ "$byte_count" -eq 0 ]; then
    report_check_failure "$target_name" 'must not be empty'
  elif LC_ALL=C grep -q '[[:space:]]' -- "$target"; then
    report_check_failure "$target_name" 'must be one non-empty line without CR, LF, or whitespace'
  elif LC_ALL=C grep -q '[^[:print:]]' -- "$target"; then
    report_check_failure "$target_name" 'must contain printable characters only'
  elif LC_ALL=C grep -Eiq -- "$placeholder_pattern" "$target"; then
    report_check_failure "$target_name" 'looks like a placeholder'
  fi
}

check_existing_secret() {
  check_failures=()
  check_source_directory
  check_existing_target
  if [ "${#check_failures[@]}" -ne 0 ]; then
    return 1
  fi
  echo 'AUTH0_SECRET_CHECK: PASS'
  return 0
}

print_plan() {
  local target
  target=$(target_path)
  echo "AUTH0_SECRET_PROVISION mode=$mode"
  echo "  source=$source_dir"
  echo "  target=$target"
  echo '  read one Auth0 client secret from a hidden interactive terminal prompt'
  echo '  write one non-empty, printable, whitespace-free line as root:root mode=0600 without a newline'
  echo '  values are never printed; existing targets are never overwritten'
}

read_secret_interactively() {
  exec 3</dev/tty || die '--apply requires an interactive terminal; secret input is never accepted from stdin'
  [ -t 3 ] || {
    exec 3<&-
    die '--apply requires an interactive terminal; secret input is never accepted from stdin'
  }

  if ! IFS= read -r -s -p 'Auth0 Client Secret (hidden): ' auth0_secret <&3; then
    printf '\n' >&2
    exec 3<&-
    die 'could not read Auth0 client secret from the terminal'
  fi
  printf '\n' >&2
  if ! IFS= read -r -s -p 'Confirm Auth0 Client Secret (hidden): ' auth0_secret_confirmation <&3; then
    printf '\n' >&2
    exec 3<&-
    unset auth0_secret
    die 'could not read Auth0 client secret confirmation from the terminal'
  fi
  printf '\n' >&2
  exec 3<&-

  if [ "$auth0_secret" != "$auth0_secret_confirmation" ]; then
    unset auth0_secret auth0_secret_confirmation
    die 'Auth0 client secret confirmation did not match'
  fi
  unset auth0_secret_confirmation
  validate_secret_value "$auth0_secret"
}

cleanup() {
  local status=$?
  set +e

  # If an unexpected post-install failure occurs, remove only the inode this
  # invocation installed.  Never remove a concurrent replacement.
  if [ "$status" -ne 0 ] && [ "$installed" -eq 1 ] && [ -n "$installed_signature" ]; then
    local actual
    actual=$(stat -c '%d:%i' -- "$(target_path)" 2>/dev/null || true)
    if [ "$actual" = "$installed_signature" ]; then
      rm -f -- "$(target_path)" 2>/dev/null || true
    fi
  fi
  if [ -n "$staging_dir" ] && [ -d "$staging_dir" ]; then
    rm -rf -- "$staging_dir" 2>/dev/null || true
  fi
  unset auth0_secret auth0_secret_confirmation
  exit "$status"
}

safe_path "$source_dir" source-directory
target=$(target_path)
safe_path "$target" 'Auth0 secret target'
if [ "$mode" = import ]; then
  safe_path "$import_file" 'legacy Auth0 secret source'
fi

if [ "$mode" = dry-run ]; then
  print_plan
  if check_target_absent; then
    echo 'DRY_RUN: no files created'
  else
    echo 'DRY_RUN: no files created (apply would refuse an existing target)' >&2
  fi
  exit 0
fi

if [ "$mode" = check ]; then
  if check_existing_secret; then
    exit 0
  fi
  exit 1
fi

# --apply/--import-file are explicit and root-only.  Do not create missing ancestors: the
# operator must first prepare the protected parent; only the final source
# directory may be bootstrapped beneath an existing safe parent.
if [ "$mode" = import ]; then
  check_target_absent || die 'refusing to overwrite existing Auth0 client secret; no files were changed'
  validate_import_file
fi
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
check_target_absent || die 'refusing to overwrite existing Auth0 client secret; no files were changed'

if [ "$mode" = apply ]; then
  read_secret_interactively
else
  validate_import_file
fi

umask 077
staging_dir=$(mktemp -d -- "$source_dir/.lagrange-auth0-secret.XXXXXX") ||
  die "cannot create private staging directory under: $source_dir"
chmod 0700 -- "$staging_dir"
trap cleanup EXIT

staged=$staging_dir/$target_name
if [ "$mode" = apply ]; then
  if ! printf '%s' "$auth0_secret" >"$staged"; then
    die 'cannot stage Auth0 client secret; no files were changed'
  fi
else
  if ! cp -- "$import_file" "$staged"; then
    die 'cannot stage legacy Auth0 client secret; no files were changed'
  fi
fi
chown root:root -- "$staged" || die 'cannot set staged Auth0 secret ownership; no files were changed'
chmod 0600 -- "$staged" || die 'cannot set staged Auth0 secret mode; no files were changed'

# Validate the staged bytes before linking them into the target directory.
[ "$(stat -c '%u:%g:%a' -- "$staged")" = '0:0:600' ] ||
  die 'staged Auth0 secret has unsafe ownership or mode; no files were changed'
[ "$(wc -c <"$staged")" -gt 0 ] ||
  die 'staged Auth0 client secret is empty; no files were changed'
! LC_ALL=C grep -q '[[:space:]]' -- "$staged" ||
  die 'staged Auth0 client secret contains whitespace; no files were changed'
! LC_ALL=C grep -q '[^[:print:]]' -- "$staged" ||
  die 'staged Auth0 client secret contains non-printable characters; no files were changed'
! LC_ALL=C grep -Eiq -- "$placeholder_pattern" "$staged" ||
  die 'staged Auth0 client secret looks like a placeholder; no files were changed'

# A hard link is an atomic, same-filesystem, no-clobber install.  Unlike mv,
# ln without -f cannot replace a target that appears after the preflight.
if ! ln -T -- "$staged" "$target"; then
  die "target appeared or could not be installed: $target"
fi
installed=1
installed_signature=$(stat -c '%d:%i' -- "$target")
[ "$(stat -c '%u:%g:%a' -- "$target")" = '0:0:600' ] ||
  die 'installed Auth0 secret has unsafe ownership or mode'
[ "$(wc -c <"$target")" -gt 0 ] ||
  die 'installed Auth0 secret is empty'
! LC_ALL=C grep -q '[[:space:]]' -- "$target" ||
  die 'installed Auth0 secret contains whitespace'
! LC_ALL=C grep -q '[^[:print:]]' -- "$target" ||
  die 'installed Auth0 secret contains non-printable characters'

rm -f -- "$staged"
rmdir -- "$staging_dir"
staging_dir=
unset auth0_secret
if [ "$mode" = import ]; then
  echo "AUTH0_SECRET_PROVISION mode=import target=$target"
  echo 'IMPORT: installed Auth0 client secret atomically as root:root mode 0600 without a newline'
else
  echo "AUTH0_SECRET_PROVISION mode=apply target=$target"
  echo 'APPLY: installed Auth0 client secret atomically as root:root mode 0600 without a newline'
fi
echo 'RESULT: secret value was not printed; no Auth0 network/API call was made'
