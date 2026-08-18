#!/usr/bin/env bash
# Provision the two production KIS credential source files.
#
# The default mode is a non-mutating plan. --apply and --check are explicit,
# root-only operations. --apply reads both values twice from /dev/tty with
# echo disabled; values are never accepted through argv/environment/stdin and
# are never printed, logged, or sent to KIS. This helper performs no network or
# vendor/API verification.
set -euo pipefail

default_source_dir=/etc/lagrange/secrets
source_dir=$default_source_dir
mode=dry-run
mode_seen=0
staging_dir=
entered_value=

# The worker/client do not impose a provider-specific app-key/app-secret
# length. This local 4 KiB bound rejects accidental pasted files while leaving
# the vendor's exact credential length to the operator/vendor contract.
max_secret_bytes=4096
key_name=kis_app_key
secret_name=kis_app_secret

declare -a installed_targets=()
declare -a installed_signatures=()
declare -a check_failures=()

placeholder_pattern='placeholder|example|todo|change[-_ ]*me|change[-_ ]*this|replace[-_ ]*(me|this|with)|your[-_ ]*(client[-_ ]*)?secret|secret[-_ ]*here|kis[-_ ]*(app[-_ ]*)?(key|secret)|app[-_ ]*(key|secret)[-_ ]*here|<[^>]+>|\$\{[^}]+\}'

usage() {
  cat <<'EOF'
Usage: scripts/ops/provision-kis-credentials.sh [--dry-run|--check|--apply]
       [--source-dir ABSOLUTE_PATH]

Modes:
  --dry-run              Print the two-file plan without changing the host
                         (default; safe to run as a non-root user).
  --check                Validate existing files read-only; requires root.
                         Reports only metadata and safe shape reasons, never
                         values or a derived hash.
  --apply                Read and confirm both values from a hidden terminal
                         prompt; requires root. Never overwrites either file.
  --source-dir PATH      Override /etc/lagrange/secrets for an isolated host
                         or test. PATH must be absolute, contain no '..', and
                         have no symlinked ancestor.

The targets are /etc/lagrange/secrets/kis_app_key and
/etc/lagrange/secrets/kis_app_secret by default. Each value is one non-empty,
printable, whitespace-free line no longer than 4096 bytes, written without a
newline as root:root mode 0600. The two values must differ. The 4096-byte
limit is a local accidental-paste guard, not an assertion about KIS's exact
credential length. Installation is staged and atomic per file with rollback
if the pair cannot be completed. No KIS network/API call is made.
EOF
}

die() {
  echo "provision-kis-credentials: $*" >&2
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
      # Do not echo an unexpected argument: callers must never put a secret
      # value in argv, and an error path must not turn that mistake into a
      # disclosure.
      die 'unknown option (use --help)'
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

  # Inspect every component, including missing ones. A later mkdir must not
  # turn a symlinked ancestor into a write target.
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

source_parent_metadata() {
  local parent=$1 metadata parent_uid parent_mode parent_mode_bits

  safe_path "$parent" source-directory-parent
  [ -d "$parent" ] ||
    die "source directory parent is not a regular directory: $parent"
  [ ! -L "$parent" ] || die "source directory parent must not be a symlink: $parent"
  metadata=$(stat -c '%u:%a' -- "$parent" 2>/dev/null) ||
    die "cannot inspect source directory parent metadata: $parent"
  parent_uid=${metadata%%:*}
  parent_mode=${metadata#*:}
  parent_mode_bits=$((8#$parent_mode))
  [ "$parent_uid" = 0 ] ||
    die "source directory parent must be owned by uid 0: $parent"
  (( (parent_mode_bits & 0022) == 0 )) ||
    die "source directory parent must not be group/other writable: $parent"
}

check_targets_absent() {
  local name target
  for name in "$key_name" "$secret_name"; do
    target=$(target_path "$name")
    safe_path "$target" "KIS credential target $name"
    if target_present "$target"; then
      echo "  existing target: $target" >&2
      return 1
    fi
  done
  return 0
}

is_placeholder_value() {
  local value=$1
  LC_ALL=C grep -Eiq -- "$placeholder_pattern" <<<"$value"
}

validate_value() {
  local LC_ALL=C label=$1 value=$2 byte_count

  [ -n "$value" ] || die "$label must not be empty"
  case "$value" in
    *$'\r'*|*$'\n'*) die "$label must not contain CR or LF" ;;
    *[![:print:]]*) die "$label must contain printable characters only" ;;
    *[[:space:]]*) die "$label must be one non-empty line without whitespace" ;;
  esac
  byte_count=$(printf '%s' "$value" | wc -c) || die "cannot inspect $label length"
  [ "$byte_count" -le "$max_secret_bytes" ] ||
    die "$label exceeds the local maximum of $max_secret_bytes bytes"
  if is_placeholder_value "$value"; then
    die "$label looks like a placeholder"
  fi
}

is_valid_secret_file() {
  local target=$1 byte_count line_count

  byte_count=$(wc -c <"$target" 2>/dev/null) || return 1
  [ "$byte_count" -gt 0 ] || return 1
  [ "$byte_count" -le "$max_secret_bytes" ] || return 1
  line_count=$(wc -l <"$target" 2>/dev/null) || return 1
  [ "$line_count" -eq 0 ] || return 1
  LC_ALL=C grep -Eq '[[:space:]]' -- "$target" && return 1
  LC_ALL=C grep -Eq '[^[:print:]]' -- "$target" && return 1
  LC_ALL=C grep -Eiq -- "$placeholder_pattern" "$target" && return 1
  return 0
}

report_check_failure() {
  local name=$1 reason=$2
  echo "KIS_CREDENTIAL_CHECK: FAIL $name: $reason" >&2
  check_failures+=1
}

check_existing_credential() {
  local name=$1 target metadata

  target=$(target_path "$name")
  safe_path "$target" "KIS credential target $name"
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
  is_valid_secret_file "$target" ||
    report_check_failure "$name" \
      "must be one non-empty printable whitespace-free line of at most $max_secret_bytes bytes with no placeholder"
}

check_existing_credentials() {
  local key_target secret_target

  check_failures=()
  if [ ! -e "$source_dir" ]; then
    report_check_failure source-directory 'missing directory'
  elif [ ! -d "$source_dir" ] || [ -L "$source_dir" ]; then
    report_check_failure source-directory 'must be a regular non-symlink directory'
  else
    if ! source_check_metadata=$(stat -c '%u:%a' -- "$source_dir" 2>/dev/null); then
      report_check_failure source-directory 'cannot inspect ownership or mode'
    else
      source_check_uid=${source_check_metadata%%:*}
      source_check_mode=${source_check_metadata#*:}
      source_check_mode_bits=$((8#$source_check_mode))
      [ "$source_check_uid" = 0 ] ||
        report_check_failure source-directory 'must be owned by uid 0'
      (( (source_check_mode_bits & 0022) == 0 )) ||
        report_check_failure source-directory 'must not be group/other writable'
    fi
  fi

  check_existing_credential "$key_name"
  check_existing_credential "$secret_name"

  if [ "${#check_failures[@]}" -eq 0 ]; then
    key_target=$(target_path "$key_name")
    secret_target=$(target_path "$secret_name")
    if cmp -s -- "$key_target" "$secret_target"; then
      echo "KIS_CREDENTIAL_CHECK: FAIL $key_name,$secret_name: values must differ" >&2
      check_failures+=1
    fi
  fi

  if [ "${#check_failures[@]}" -ne 0 ]; then
    return 1
  fi
  echo 'KIS_CREDENTIAL_CHECK: PASS'
  return 0
}

print_plan() {
  echo "KIS_CREDENTIAL_PROVISION mode=$mode"
  echo "  source=$source_dir"
  echo "  final files: $key_name and $secret_name"
  echo "  each value: printable, whitespace-free, 1..$max_secret_bytes bytes, no newline"
  echo '  owner=root:root mode=0600; values must be distinct'
  echo '  --apply reads hidden terminal input twice per value and never overwrites either target'
  echo '  no KIS network/API call or vendor length verification'
}

read_confirmed() {
  local label=$1 first second

  exec 3</dev/tty || die 'cannot open /dev/tty for hidden credential input'
  printf 'Enter %s: ' "$label" >&2
  if ! IFS= read -r -s -u 3 first; then
    exec 3<&-
    die "could not read $label from /dev/tty"
  fi
  printf '\nConfirm %s: ' "$label" >&2
  if ! IFS= read -r -s -u 3 second; then
    exec 3<&-
    die "could not confirm $label from /dev/tty"
  fi
  printf '\n' >&2
  exec 3<&-
  [ "$first" = "$second" ] || die "$label confirmation did not match"
  entered_value=$first
}

safe_path "$source_dir" source-directory

if [ "$mode" = dry-run ]; then
  print_plan
  if [ -d "$source_dir" ] && [ ! -L "$source_dir" ]; then
    if check_targets_absent; then
      echo 'DRY_RUN: no files created'
    else
      echo 'DRY_RUN: no files created (apply would refuse existing targets)' >&2
    fi
  else
    echo 'DRY_RUN: no files created (source directory is not present)'
  fi
  exit 0
fi

if [ "$mode" = check ]; then
  check_existing_credentials && exit 0
  exit 1
fi

# Apply preflight is read-only until both values have passed validation. For a
# missing override directory, validate its existing parent now and create only
# the final directory after the hidden prompts succeed.
if [ -e "$source_dir" ]; then
  source_metadata
else
  source_parent=${source_dir%/*}
  [ -n "$source_parent" ] || source_parent=/
  source_parent_metadata "$source_parent"
fi
check_targets_absent || die 'refusing to overwrite an existing KIS credential; no files were changed'

read_confirmed 'KIS app key'
app_key=$entered_value
validate_value "$key_name" "$app_key"
read_confirmed 'KIS app secret'
app_secret=$entered_value
validate_value "$secret_name" "$app_secret"
[ "$app_key" != "$app_secret" ] || die 'KIS app key and app secret must differ'

if [ ! -e "$source_dir" ]; then
  install -d -o root -g root -m 0750 -- "$source_dir" ||
    die "cannot create source directory: $source_dir"
fi
source_metadata
check_targets_absent || die 'a KIS credential target appeared during preflight; no files were changed'

umask 077
staging_dir=$(mktemp -d -- "$source_dir/.lagrange-kis-credentials.XXXXXX") ||
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

stage_value() {
  local name=$1 value=$2 raw=$staging_dir/.$1.raw file=$staging_dir/$1

  # printf is a shell builtin; the value goes to the staged file, never into
  # an argv, environment, stdout, or a diagnostic.
  printf '%s' "$value" >"$raw" || die "cannot stage $name"
  install -o root -g root -m 0600 -- "$raw" "$file" ||
    die "cannot install staged $name"
  rm -f -- "$raw"
  [ "$(stat -c '%u:%g:%a' -- "$file")" = '0:0:600' ] ||
    die "staged $name has unsafe ownership or mode"
  is_valid_secret_file "$file" || die "staged $name failed shape validation"
}

stage_value "$key_name" "$app_key"
stage_value "$secret_name" "$app_secret"
cmp -s -- "$staging_dir/$key_name" "$staging_dir/$secret_name" &&
  die 'staged KIS credential values must differ'

# A hard link is an atomic no-clobber install on the same filesystem. If the
# second link fails, EXIT cleanup removes only inodes this invocation linked.
for name in "$key_name" "$secret_name"; do
  staged="$staging_dir/$name"
  target=$(target_path "$name")
  stage_signature=$(stat -c '%d:%i' -- "$staged")
  if ! ln -T -- "$staged" "$target"; then
    die "target appeared or could not be installed: $target"
  fi
  installed_targets+=("$target")
  installed_signatures+=("$stage_signature")
  [ "$(stat -c '%d:%i:%u:%g:%a' -- "$target")" = \
    "${stage_signature}:0:0:600" ] || die "installed $name has unsafe metadata"
done

rm -f -- "$staging_dir/$key_name" "$staging_dir/$secret_name"
rmdir -- "$staging_dir"
staging_dir=

echo "KIS_CREDENTIAL_PROVISION mode=apply source=$source_dir"
echo 'APPLY: installed two distinct KIS credential source files with root-only access'
echo 'APPLY: values were not printed; no KIS network/API call was made'
