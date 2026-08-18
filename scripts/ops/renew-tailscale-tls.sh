#!/usr/bin/env bash
# Renew the Tailscale HTTPS certificate into the protected source and
# reverse-proxy runtime secret pair. The default is a no-change plan; --check
# is read-only and --renew is the only mode that can issue/replace TLS files.
# No database, Auth0, KIS, or other application service is involved.
set -euo pipefail

expected_domain=l1nnx-sh.taild74a33.ts.net
checkend_seconds=2592000
min_validity=720h
default_config_file=/etc/lagrange/tailscale-tls-renewal.conf
mode=dry-run
mode_seen=0
config_file=$default_config_file

source_dir=
runtime_dir=
compose_file=
env_file=
compose_project=
code_commit=
lock_file=
domain=
source_cert=
source_key=
runtime_cert=
runtime_key=
openssl_bin=

source_stage_dir=
runtime_stage_dir=
backup_dir=
transaction_active=0
source_cert_installed_signature=
source_key_installed_signature=
runtime_cert_installed_signature=
runtime_key_installed_signature=
replace_count=0
source_cert_had_old=0
source_key_had_old=0
runtime_cert_had_old=0
runtime_key_had_old=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/renew-tailscale-tls.sh [--dry-run|--check|--renew]
       [--config-file ABSOLUTE_PATH]

Modes:
  --dry-run       Print the renewal plan without reading or changing TLS files
                  (default; safe for a non-root caller).
  --check         Root-only read-only validation of the source/runtime pair.
  --renew         Root-only renewal/reconciliation. Tailscale writes only a
                  private staging pair; final source/runtime files are changed
                  only after validation.
  --config-file   Protected root-owned 0600 configuration file. It contains
  only the fixed domain, approved 40-hex code commit, and absolute non-secret paths.

The fixed certificate SAN is l1nnx-sh.taild74a33.ts.net. The source pair is
/etc/lagrange/secrets/tls/lagrange.crt and lagrange.key by convention; runtime
files are reverse-proxy/lagrange_tls_cert and lagrange_tls_key. Production
paths and the Compose project/file/env identity must come from the protected
configuration file, never from the current worktree or an arbitrary default.

--renew requests `tailscale cert --min-validity=720h`, validates the exact SAN,
30-day remaining validity, public-key match, and metadata, then reconciles only
the TLS source/runtime pair. A running reverse-proxy is force-recreated with
--no-deps; an absent reverse-proxy is never started. No KIS/API/database call
is made by this helper.
EOF
}

die() {
  echo "renew-tailscale-tls: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --renew'
      mode=dry-run
      mode_seen=1
      shift
      ;;
    --check)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --renew'
      mode=check
      mode_seen=1
      shift
      ;;
    --renew)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --dry-run, --check, or --renew'
      mode=renew
      mode_seen=1
      shift
      ;;
    --config-file)
      [ "$#" -ge 2 ] || die '--config-file needs an absolute path'
      config_file=$2
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

check_existing_parent() {
  local path=$1 label=$2 probe metadata uid mode_bits

  probe=${path%/*}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ] && [ ! -e "$probe" ]; do
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  [ -d "$probe" ] && [ ! -L "$probe" ] || die "$label parent is not a regular directory: $probe"
  metadata=$(stat -c '%u:%a' -- "$probe" 2>/dev/null) ||
    die "cannot inspect $label parent metadata: $probe"
  uid=${metadata%%:*}
  mode_bits=$((8#${metadata#*:}))
  [ "$uid" = 0 ] || die "$label parent must be owned by uid 0: $probe"
  (( (mode_bits & 0022) == 0 )) ||
    die "$label parent must not be group/other writable: $probe"
}

check_config_metadata() {
  safe_path "$config_file" config-file
  check_existing_parent "$config_file" config-file
  [ -f "$config_file" ] && [ ! -L "$config_file" ] ||
    die 'config file must be a regular non-symlink file'
  [ "$(stat -c '%u:%g:%a' -- "$config_file" 2>/dev/null)" = '0:0:600' ] ||
    die 'config file must be owned by root:root with mode 0600'
}

parse_config() {
  local line key value
  declare -A values=()

  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      '') continue ;;
      \#*) continue ;;
    esac
    case "$line" in
      *=*) ;;
      *) die 'config file contains a malformed assignment' ;;
    esac
    key=${line%%=*}
    value=${line#*=}
    [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || die 'config file contains an invalid key'
    case "$key" in
      TLS_DOMAIN|TLS_SOURCE_DIR|TLS_RUNTIME_DIR|COMPOSE_FILE|COMPOSE_ENV_FILE|COMPOSE_PROJECT|LAGRANGE_CODE_COMMIT|LOCK_FILE) ;;
      *) die 'config file contains an unsupported key' ;;
    esac
    case "$value" in
      *$'\r'*|*$'\n'*|*[[:space:]]*) die "config value is not a single whitespace-free value: $key" ;;
    esac
    [ -z "${values[$key]+set}" ] || die "config key is duplicated: $key"
    values[$key]=$value
  done <"$config_file"

  for key in TLS_DOMAIN TLS_SOURCE_DIR TLS_RUNTIME_DIR COMPOSE_FILE \
    COMPOSE_ENV_FILE COMPOSE_PROJECT LAGRANGE_CODE_COMMIT LOCK_FILE; do
    [ -n "${values[$key]:-}" ] || die "config key is missing: $key"
  done
  [ "${values[TLS_DOMAIN]}" = "$expected_domain" ] ||
    die 'config TLS_DOMAIN does not match the fixed expected domain'
  [[ "${values[COMPOSE_PROJECT]}" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,62}$ ]] ||
    die 'config COMPOSE_PROJECT has an unsafe shape'

  domain=${values[TLS_DOMAIN]}
  source_dir=${values[TLS_SOURCE_DIR]}
  runtime_dir=${values[TLS_RUNTIME_DIR]}
  compose_file=${values[COMPOSE_FILE]}
  env_file=${values[COMPOSE_ENV_FILE]}
  compose_project=${values[COMPOSE_PROJECT]}
  code_commit=${values[LAGRANGE_CODE_COMMIT]}
  lock_file=${values[LOCK_FILE]}

  safe_path "$source_dir" TLS_SOURCE_DIR
  safe_path "$runtime_dir" TLS_RUNTIME_DIR
  safe_path "$compose_file" COMPOSE_FILE
  safe_path "$env_file" COMPOSE_ENV_FILE
  safe_path "$lock_file" LOCK_FILE
  [[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] ||
    die 'config LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
  [ "$code_commit" != 0000000000000000000000000000000000000000 ] ||
    die 'config LAGRANGE_CODE_COMMIT must not be all zeroes'
  source_cert=$source_dir/lagrange.crt
  source_key=$source_dir/lagrange.key
  runtime_cert=$runtime_dir/lagrange_tls_cert
  runtime_key=$runtime_dir/lagrange_tls_key
  safe_path "$source_cert" source-certificate
  safe_path "$source_key" source-key
  safe_path "$runtime_cert" runtime-certificate
  safe_path "$runtime_key" runtime-key
}

check_regular_file() {
  local path=$1 label=$2
  [ -f "$path" ] && [ ! -L "$path" ] || die "$label must be a regular non-symlink file: $path"
}

check_protected_file() {
  local path=$1 label=$2 metadata uid gid mode mode_bits
  check_regular_file "$path" "$label"
  metadata=$(stat -c '%u:%g:%a' -- "$path" 2>/dev/null) ||
    die "cannot inspect $label metadata: $path"
  uid=${metadata%%:*}
  gid=${metadata#*:}
  gid=${gid%%:*}
  [ "$uid" = 0 ] && [ "$gid" = 0 ] || die "$label must be owned by root:root: $path"
  mode=${metadata##*:}
  mode_bits=$((8#$mode))
  (( (mode_bits & 0022) == 0 )) || die "$label must not be group/other writable: $path"
}

check_source_directory() {
  local metadata uid gid mode mode_bits
  [ -d "$source_dir" ] && [ ! -L "$source_dir" ] ||
    die "TLS source directory must be a regular non-symlink directory: $source_dir"
  metadata=$(stat -c '%u:%g:%a' -- "$source_dir" 2>/dev/null) ||
    die 'cannot inspect TLS source directory metadata'
  uid=${metadata%%:*}
  gid=${metadata#*:}
  gid=${gid%%:*}
  mode=${metadata##*:}
  mode_bits=$((8#$mode))
  [ "$uid" = 0 ] && [ "$gid" = 0 ] ||
    die 'TLS source directory must be owned by root:root'
  (( (mode_bits & 0022) == 0 )) ||
    die 'TLS source directory must not be group/other writable'
}

check_runtime_directory() {
  local metadata uid gid mode mode_bits
  [ -d "$runtime_dir" ] && [ ! -L "$runtime_dir" ] ||
    die "TLS runtime directory must be a regular non-symlink directory: $runtime_dir"
  metadata=$(stat -c '%u:%g:%a' -- "$runtime_dir" 2>/dev/null) ||
    die 'cannot inspect TLS runtime directory metadata'
  uid=${metadata%%:*}
  gid=${metadata#*:}
  gid=${gid%%:*}
  [ "$uid" = 101 ] && [ "$gid" = 101 ] ||
    die 'TLS runtime directory must be owned by numeric 101:101'
  mode=${metadata##*:}
  mode_bits=$((8#$mode))
  (( (mode_bits & 0022) == 0 )) ||
    die 'TLS runtime directory must not be group/other writable'
}

check_config_paths() {
  check_source_directory
  check_runtime_directory
  check_protected_file "$compose_file" Compose-file
  check_protected_file "$env_file" Compose-env-file
  check_existing_parent "$lock_file" LOCK_FILE
  if [ -e "$lock_file" ] || [ -L "$lock_file" ]; then
    check_regular_file "$lock_file" LOCK_FILE
    check_protected_file "$lock_file" LOCK_FILE
  fi
}

check_optional_pair_metadata() {
  local cert=$1 key=$2 label=$3 expected=$4
  local target metadata
  for target in "$cert" "$key"; do
    if [ ! -e "$target" ] && [ ! -L "$target" ]; then
      continue
    fi
    check_regular_file "$target" "$label TLS file"
    metadata=$(stat -c '%u:%g:%a' -- "$target" 2>/dev/null) ||
      die "cannot inspect $label TLS file metadata"
    [ "$metadata" = "$expected" ] ||
      die "$label TLS file must have metadata $expected"
  done
}

preflight_tls_paths() {
  check_config_paths
  check_optional_pair_metadata "$source_cert" "$source_key" source '0:0:600'
  check_optional_pair_metadata "$runtime_cert" "$runtime_key" runtime '101:101:440'
}

checkend_and_san_pair() {
  local cert=$1 key=$2 workdir=$3 san_values pubdir

  [ -f "$cert" ] && [ ! -L "$cert" ] || return 1
  [ -f "$key" ] && [ ! -L "$key" ] || return 1
  "$openssl_bin" x509 -in "$cert" -noout >/dev/null 2>&1 || return 1
  "$openssl_bin" pkey -in "$key" -noout -check >/dev/null 2>&1 || return 1
  "$openssl_bin" x509 -in "$cert" -checkend "$checkend_seconds" -noout >/dev/null 2>&1 || return 1
  san_values=$(
    "$openssl_bin" x509 -in "$cert" -noout -ext subjectAltName 2>/dev/null |
      awk '/Subject Alternative Name:/{seen=1; next} seen {gsub(/[[:space:]]/, ""); if ($0 != "") print}'
  ) || return 1
  [ "$san_values" = "DNS:$domain" ] || return 1

  pubdir=$(mktemp -d -- "$workdir/.lagrange-tls-public-key.XXXXXX") || return 1
  if ! "$openssl_bin" x509 -in "$cert" -pubkey -noout |
      "$openssl_bin" pkey -pubin -outform DER >"$pubdir/cert.der" 2>/dev/null; then
    rm -rf -- "$pubdir"
    return 1
  fi
  if ! "$openssl_bin" pkey -in "$key" -pubout |
      "$openssl_bin" pkey -pubin -outform DER >"$pubdir/key.der" 2>/dev/null; then
    rm -rf -- "$pubdir"
    return 1
  fi
  cmp -s -- "$pubdir/cert.der" "$pubdir/key.der"
  local result=$?
  rm -rf -- "$pubdir"
  return "$result"
}

pair_metadata_valid() {
  local cert=$1 key=$2 expected=$3 metadata
  [ -f "$cert" ] && [ ! -L "$cert" ] || return 1
  [ -f "$key" ] && [ ! -L "$key" ] || return 1
  metadata=$(stat -c '%u:%g:%a' -- "$cert" 2>/dev/null) || return 1
  [ "$metadata" = "$expected" ] || return 1
  metadata=$(stat -c '%u:%g:%a' -- "$key" 2>/dev/null) || return 1
  [ "$metadata" = "$expected" ]
}

pair_matches() {
  local cert=$1 key=$2 staged_cert=$3 staged_key=$4
  cmp -s -- "$cert" "$staged_cert" && cmp -s -- "$key" "$staged_key"
}

check_state() {
  local failures=0 source_ok=0 runtime_ok=0
  if ! pair_metadata_valid "$source_cert" "$source_key" '0:0:600' ||
     ! checkend_and_san_pair "$source_cert" "$source_key" "$source_dir"; then
    echo 'TLS_CHECK: FAIL source pair metadata/certificate/SAN/key/30-day validity contract' >&2
    failures=$((failures + 1))
  else
    source_ok=1
  fi
  if ! pair_metadata_valid "$runtime_cert" "$runtime_key" '101:101:440' ||
     ! checkend_and_san_pair "$runtime_cert" "$runtime_key" "$runtime_dir"; then
    echo 'TLS_CHECK: FAIL runtime pair metadata/certificate/SAN/key/30-day validity contract' >&2
    failures=$((failures + 1))
  else
    runtime_ok=1
  fi
  if [ "$source_ok" -eq 1 ] && [ "$runtime_ok" -eq 1 ] &&
     ! pair_matches "$runtime_cert" "$runtime_key" "$source_cert" "$source_key"; then
    echo 'TLS_CHECK: FAIL source/runtime TLS pairs differ' >&2
    failures=$((failures + 1))
  fi
  if [ "$failures" -ne 0 ]; then
    return 1
  fi
  echo "TLS_CHECK: PASS domain=$domain source=$source_dir runtime=$runtime_dir"
}

ensure_tools() {
  openssl_bin=$(command -v openssl) || die 'openssl is required'
  command -v flock >/dev/null 2>&1 || die 'flock is required'
  command -v awk >/dev/null 2>&1 || die 'awk is required'
}

make_staging_dirs() {
  umask 077
  source_stage_dir=$(mktemp -d -- "$source_dir/.lagrange-tls-renewal.XXXXXX") ||
    die 'cannot create TLS source staging directory'
  chmod 0700 -- "$source_stage_dir"
  runtime_stage_dir=$(mktemp -d -- "$runtime_dir/.lagrange-tls-renewal.XXXXXX") ||
    die 'cannot create TLS runtime staging directory'
  chmod 0700 -- "$runtime_stage_dir"
}

stage_source_from_current() {
  cp --no-dereference -- "$source_cert" "$source_stage_dir/lagrange.crt" ||
    die 'cannot stage current TLS certificate'
  cp --no-dereference -- "$source_key" "$source_stage_dir/lagrange.key" ||
    die 'cannot stage current TLS key'
  chown --no-dereference root:root "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" ||
    die 'cannot set staged TLS source ownership'
  chmod 0600 "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key"
}

issue_staged_certificate() {
  command -v tailscale >/dev/null 2>&1 || die 'tailscale is required for certificate renewal'
  if ! tailscale cert \
    --cert-file="$source_stage_dir/lagrange.crt" \
    --key-file="$source_stage_dir/lagrange.key" \
    --min-validity=720h "$domain" \
    >"$source_stage_dir/tailscale.stdout" 2>"$source_stage_dir/tailscale.stderr"; then
    die 'tailscale cert failed; no TLS files were changed'
  fi
  rm -f -- "$source_stage_dir/tailscale.stdout" "$source_stage_dir/tailscale.stderr"
  check_regular_file "$source_stage_dir/lagrange.crt" 'renewed TLS certificate'
  check_regular_file "$source_stage_dir/lagrange.key" 'renewed TLS key'
  chown --no-dereference root:root "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" ||
    die 'cannot set renewed TLS source ownership'
  chmod 0600 "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key"
  pair_metadata_valid "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" '0:0:600' ||
    die 'renewed TLS source pair has unsafe metadata'
  checkend_and_san_pair "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" "$source_stage_dir" ||
    die 'renewed TLS pair failed certificate, SAN, key-match, or 30-day validation'
}

stage_runtime_from_source() {
  cp --no-dereference -- "$source_stage_dir/lagrange.crt" "$runtime_stage_dir/lagrange_tls_cert" ||
    die 'cannot stage reverse-proxy TLS certificate'
  cp --no-dereference -- "$source_stage_dir/lagrange.key" "$runtime_stage_dir/lagrange_tls_key" ||
    die 'cannot stage reverse-proxy TLS key'
  chown --no-dereference 101:101 "$runtime_stage_dir/lagrange_tls_cert" "$runtime_stage_dir/lagrange_tls_key" ||
    die 'cannot set numeric reverse-proxy TLS ownership'
  chmod 0440 "$runtime_stage_dir/lagrange_tls_cert" "$runtime_stage_dir/lagrange_tls_key"
  pair_metadata_valid "$runtime_stage_dir/lagrange_tls_cert" "$runtime_stage_dir/lagrange_tls_key" '101:101:440' ||
    die 'staged reverse-proxy TLS pair has unsafe metadata'
  checkend_and_san_pair "$runtime_stage_dir/lagrange_tls_cert" "$runtime_stage_dir/lagrange_tls_key" "$runtime_stage_dir" ||
    die 'staged reverse-proxy TLS pair failed certificate, SAN, key-match, or 30-day validation'
}

backup_one() {
  local target=$1 backup=$2 had_name=$3 metadata
  if [ -e "$target" ] || [ -L "$target" ]; then
    [ -f "$target" ] && [ ! -L "$target" ] || die 'TLS target changed to a non-regular file during preflight'
    metadata=$(stat -c '%u:%g:%a' -- "$target" 2>/dev/null) ||
      die 'cannot record current TLS file metadata'
    cp --no-dereference -- "$target" "$backup" || die 'cannot back up current TLS file'
    printf '%s\n' "$metadata" >"$backup.metadata" || die 'cannot record TLS rollback metadata'
    printf -v "$had_name" '%s' 1
  else
    printf -v "$had_name" '%s' 0
  fi
}

begin_transaction() {
  backup_dir=$(mktemp -d -- "$source_dir/.lagrange-tls-backup.XXXXXX") ||
    die 'cannot create TLS rollback directory'
  chmod 0700 -- "$backup_dir"
  backup_one "$source_cert" "$backup_dir/source.crt" source_cert_had_old
  backup_one "$source_key" "$backup_dir/source.key" source_key_had_old
  backup_one "$runtime_cert" "$backup_dir/runtime.crt" runtime_cert_had_old
  backup_one "$runtime_key" "$backup_dir/runtime.key" runtime_key_had_old
  transaction_active=1
}

replace_one() {
  local staged=$1 target=$2 slot=$3
  [ -f "$staged" ] && [ ! -L "$staged" ] || die 'TLS staged target is not a regular file'
  [ ! -L "$target" ] || die 'TLS final target became a symlink during renewal'
  mv -T -- "$staged" "$target" || die 'cannot atomically install TLS file'
  printf -v "$slot" '%s' "$(stat -c '%d:%i' -- "$target")"
  replace_count=$((replace_count + 1))
  if [ "${LAGRANGE_TLS_TEST_FAIL_AFTER_REPLACE:-}" = "$replace_count" ]; then
    die 'test replacement failure injection'
  fi
}

rollback_one() {
  local target=$1 backup=$2 had_old=$3 installed_signature=$4 metadata_file=$5
  local actual metadata uid gid mode
  [ -n "$installed_signature" ] || return 0
  actual=$(stat -c '%d:%i' -- "$target" 2>/dev/null || true)
  [ "$actual" = "$installed_signature" ] || return 1
  if [ "$had_old" -eq 1 ]; then
    [ -f "$metadata_file" ] && [ ! -L "$metadata_file" ] || return 1
    IFS=: read -r uid gid mode <"$metadata_file" || return 1
    [[ "$uid" =~ ^[0-9]+$ && "$gid" =~ ^[0-9]+$ && "$mode" =~ ^[0-7]{3,4}$ ]] || return 1
    mv -T -- "$backup" "$target" || return 1
    chown --no-dereference "$uid:$gid" -- "$target" || return 1
    chmod "$mode" -- "$target" || return 1
  else
    rm -f -- "$target" || return 1
  fi
}

rollback_transaction() {
  local ok=0
  rollback_one "$runtime_key" "$backup_dir/runtime.key" "$runtime_key_had_old" \
    "$runtime_key_installed_signature" "$backup_dir/runtime.key.metadata" || ok=1
  rollback_one "$runtime_cert" "$backup_dir/runtime.crt" "$runtime_cert_had_old" \
    "$runtime_cert_installed_signature" "$backup_dir/runtime.crt.metadata" || ok=1
  rollback_one "$source_key" "$backup_dir/source.key" "$source_key_had_old" \
    "$source_key_installed_signature" "$backup_dir/source.key.metadata" || ok=1
  rollback_one "$source_cert" "$backup_dir/source.crt" "$source_cert_had_old" \
    "$source_cert_installed_signature" "$backup_dir/source.crt.metadata" || ok=1
  if [ "$ok" -ne 0 ]; then
    echo 'TLS_RENEWAL: rollback could not prove every installed inode; inspect source/runtime TLS pair' >&2
    return 1
  fi
  return 0
}

cleanup() {
  local status=$? rollback_status=0
  set +e
  if [ "$status" -ne 0 ] && [ "$transaction_active" -eq 1 ]; then
    rollback_transaction || rollback_status=1
  fi
  [ -z "$source_stage_dir" ] || [ ! -d "$source_stage_dir" ] || rm -rf -- "$source_stage_dir"
  [ -z "$runtime_stage_dir" ] || [ ! -d "$runtime_stage_dir" ] || rm -rf -- "$runtime_stage_dir"
  [ -z "$backup_dir" ] || [ ! -d "$backup_dir" ] || rm -rf -- "$backup_dir"
  if [ "$rollback_status" -ne 0 ]; then
    exit 1
  fi
  exit "$status"
}

print_plan() {
  echo 'TLS_RENEWAL_PLAN mode=dry-run'
  echo "  expected_domain=$expected_domain"
  echo "  config=$config_file"
  echo '  renew command: tailscale cert --min-validity=720h into private staging only'
  echo '  validate: root-owned source 0600, runtime 101:101 0440, exact SAN, key match, checkend 30d'
  echo '  changed files: TLS source pair and reverse-proxy runtime pair only'
  echo '  running reverse-proxy: force-recreate --no-deps; absent reverse-proxy: never start'
  echo '  no KIS/Auth0/database/API call; no files changed'
}

safe_path "$config_file" config-file
if [ "$mode" = dry-run ]; then
  print_plan
  if [ ! -e "$config_file" ]; then
    echo 'DRY_RUN: config is absent or protected from current user; root check/renew will verify it'
  else
    echo 'DRY_RUN: no files changed'
  fi
  exit 0
fi

check_config_metadata
parse_config
check_config_paths
source_cert=$source_dir/lagrange.crt
source_key=$source_dir/lagrange.key
runtime_cert=$runtime_dir/lagrange_tls_cert
runtime_key=$runtime_dir/lagrange_tls_key
ensure_tools
command -v docker >/dev/null 2>&1 || die 'docker is required for renewal state inspection'

if [ "$mode" = check ]; then
  if check_state; then
    exit 0
  fi
  exit 1
fi

umask 077
exec 9>"$lock_file" || die 'cannot open TLS renewal lock file'
flock -n 9 || die 'another TLS renewal is already running'
trap cleanup EXIT

source_pair_valid=0
if pair_metadata_valid "$source_cert" "$source_key" '0:0:600' &&
   checkend_and_san_pair "$source_cert" "$source_key" "$source_dir"; then
  source_pair_valid=1
fi
runtime_pair_valid=0
if pair_metadata_valid "$runtime_cert" "$runtime_key" '101:101:440' &&
   checkend_and_san_pair "$runtime_cert" "$runtime_key" "$runtime_dir"; then
  runtime_pair_valid=1
fi

if [ "$source_pair_valid" -eq 1 ] && [ "$runtime_pair_valid" -eq 1 ] &&
   pair_matches "$runtime_cert" "$runtime_key" "$source_cert" "$source_key"; then
  echo "TLS_RENEWAL: NOOP domain=$domain (source/runtime pair already valid for at least 30 days)"
  exit 0
fi

make_staging_dirs
if [ "$source_pair_valid" -eq 1 ]; then
  stage_source_from_current
else
  issue_staged_certificate
fi
pair_metadata_valid "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" '0:0:600' ||
  die 'staged TLS source pair has unsafe metadata'
checkend_and_san_pair "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key" "$source_stage_dir" ||
  die 'staged TLS source pair failed certificate, SAN, key-match, or 30-day validation'
stage_runtime_from_source

source_changed=1
if [ "$source_pair_valid" -eq 1 ] &&
   pair_matches "$source_cert" "$source_key" \
     "$source_stage_dir/lagrange.crt" "$source_stage_dir/lagrange.key"; then
  source_changed=0
fi
runtime_changed=1
if [ "$runtime_pair_valid" -eq 1 ] &&
   pair_matches "$runtime_cert" "$runtime_key" \
     "$runtime_stage_dir/lagrange_tls_cert" "$runtime_stage_dir/lagrange_tls_key"; then
  runtime_changed=0
fi

if [ "$source_changed" -eq 0 ] && [ "$runtime_changed" -eq 0 ]; then
  echo "TLS_RENEWAL: NOOP domain=$domain (validated staged pair is unchanged)"
  exit 0
fi

begin_transaction
if [ "$source_changed" -eq 1 ]; then
  replace_one "$source_stage_dir/lagrange.crt" "$source_cert" source_cert_installed_signature
  replace_one "$source_stage_dir/lagrange.key" "$source_key" source_key_installed_signature
fi
if [ "$runtime_changed" -eq 1 ]; then
  replace_one "$runtime_stage_dir/lagrange_tls_cert" "$runtime_cert" runtime_cert_installed_signature
  replace_one "$runtime_stage_dir/lagrange_tls_key" "$runtime_key" runtime_key_installed_signature
fi

pair_metadata_valid "$source_cert" "$source_key" '0:0:600' || die 'installed source TLS metadata validation failed'
checkend_and_san_pair "$source_cert" "$source_key" "$source_dir" || die 'installed source TLS certificate validation failed'
pair_metadata_valid "$runtime_cert" "$runtime_key" '101:101:440' || die 'installed runtime TLS metadata validation failed'
checkend_and_san_pair "$runtime_cert" "$runtime_key" "$runtime_dir" || die 'installed runtime TLS certificate validation failed'
pair_matches "$runtime_cert" "$runtime_key" "$source_cert" "$source_key" ||
  die 'installed source/runtime TLS pair diverged'

# The file transaction is now converged. A Compose failure must not roll back
# a valid pair underneath a possibly recreated container; report it and leave
# the pair ready for an explicit operator retry.
transaction_active=0

proxy_action=absent-no-start
if command -v docker >/dev/null 2>&1; then
  compose_state=$(mktemp -- "$source_stage_dir/.lagrange-tls-compose-state.XXXXXX") ||
    die 'cannot create private Compose state staging file'
  if ! LAGRANGE_CODE_COMMIT="$code_commit" docker compose --project-name "$compose_project" \
    --env-file "$env_file" --file "$compose_file" ps --services \
    --filter status=running >"$compose_state" 2>/dev/null; then
    rm -f -- "$compose_state"
    die 'cannot inspect Compose reverse-proxy state; TLS pair remains converged'
  fi
  if grep -Fxq reverse-proxy "$compose_state"; then
    if ! LAGRANGE_CODE_COMMIT="$code_commit" docker compose --project-name "$compose_project" \
      --env-file "$env_file" --file "$compose_file" up --detach --no-deps \
      --force-recreate --no-build --pull never --wait reverse-proxy \
      >"$source_stage_dir/.lagrange-tls-compose-refresh.$$" 2>&1; then
      rm -f -- "$compose_state" "$source_stage_dir/.lagrange-tls-compose-refresh.$$"
      die 'reverse-proxy force-recreate failed; TLS source/runtime pair remains converged'
    fi
    rm -f -- "$source_stage_dir/.lagrange-tls-compose-refresh.$$"
    proxy_action=force-recreate-reverse-proxy
  fi
  rm -f -- "$compose_state"
fi

if [ "$source_changed" -eq 1 ] && [ "$runtime_changed" -eq 1 ]; then
  change_summary=source-and-runtime
elif [ "$source_changed" -eq 1 ]; then
  change_summary=source-only
else
  change_summary=runtime-only
fi
echo "TLS_RENEWAL: PASS domain=$domain changed=$change_summary proxy_action=$proxy_action"
