#!/usr/bin/env bash
# Create service-specific Docker secret copies with native-Linux ownership.
#
# Docker Compose's file-backed secrets are bind mounts on Linux; their
# long-syntax mode fields are not a substitute for host permissions.  This
# script therefore copies each operator secret into a directory owned by the
# UID that consumes it.  Run as root (or through the host secret manager).
# The infrastructure scope installs only the DB/bootstrap/schema copies needed
# before KIS credentials or curated dataset approval; serving-prereqs stages the
# non-KIS serving copies without starting anything; backfill adds the
# research-worker KIS copies, and release installs the complete inventory.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
path_base=$(pwd -P)
source_dir=${LAGRANGE_SECRET_SOURCE_DIR:-$script_dir}
runtime_dir=${LAGRANGE_RUNTIME_SECRET_DIR:-$source_dir/runtime}
scope=release

die() {
  echo "provision-runtime-secrets: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: deploy/secrets/provision-runtime-secrets.sh
       [--scope infrastructure|serving-prereqs|backfill|release]

  --scope infrastructure
                    Install only PostgreSQL/bootstrap/schema runtime copies;
                    KIS, Auth0/TLS, and serving dataset inputs are not needed.
  --scope serving-prereqs
                    Pre-stage every non-KIS serving/runtime copy, including
                    TLS, Auth0, API, worker, and research DB inputs. This
                    scope never starts Compose services or requires KIS,
                    RESEARCH_*, entitlement, or dataset-pin values.
  --scope backfill  Add research-worker runtime copies for the pre-approval
                    KIS backfill.
  --scope release   Install every Compose service secret (default).
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs infrastructure, serving-prereqs, backfill, or release'
      scope=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done
case "$scope" in
  infrastructure|serving-prereqs|backfill|release) ;;
  *) die '--scope must be infrastructure, serving-prereqs, backfill, or release' ;;
esac

[ "$(id -u)" -eq 0 ] || die "must run as root to assign service UID ownership"

reject_dotdot() {
  local path=$1 label=$2
  case "$path" in
    ''|..|../*|*/../*|*/..) die "$label must not contain '..': $path" ;;
  esac
}

absolute_path() {
  local path=$1
  case "$path" in
    /*) printf '%s' "$path" ;;
    *) printf '%s/%s' "$path_base" "$path" ;;
  esac
}

check_path() {
  local path=$1 label=$2 probe
  reject_dotdot "$path" "$label"
  case "$path" in
    /*) ;;
    *) die "$label must resolve to an absolute path: $path" ;;
  esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

# Resolve relative operator paths against the caller's physical working
# directory, preserving the old relative-path behavior without allowing `..`
# aliases or a symlinked ancestor to reach a root-owned copy operation.
source_dir=$(absolute_path "$source_dir")
runtime_dir=$(absolute_path "$runtime_dir")
check_path "$source_dir" source-directory
check_path "$runtime_dir" runtime-directory
[ -d "$source_dir" ] || die "source directory is missing: $source_dir"
copy_specs=()
extra_source_specs=()

add_copy() {
  copy_specs+=("$1 $2 $3 $4 $5 $6 $7")
}

add_extra_source() {
  extra_source_specs+=("$1 $2")
}

add_infrastructure_copies() {
  # Both one-shot database jobs run as UID/GID 999 in deploy/db/Dockerfile.
  # Keep their private copies readable by that non-root identity only.
  add_copy db-role-bootstrap postgres_password postgres_password 999 999 0400 yes
  add_copy db-role-bootstrap db_migration_owner_password db_migration_owner_password 999 999 0400 yes
  add_copy db-role-bootstrap db_app_password db_app_password 999 999 0400 yes
  add_copy db-role-bootstrap db_worker_password db_worker_password 999 999 0400 yes
  add_copy db-role-bootstrap db_audit_password db_audit_password 999 999 0400 yes
  add_copy db-role-bootstrap db_research_password db_research_password 999 999 0400 yes
  add_copy db-role-bootstrap db_admin_password db_admin_password 999 999 0400 yes
  add_copy db-migrate db_migration_owner_password db_migration_owner_password 999 999 0400 yes
  add_copy postgres postgres_password postgres_password 999 999 0440 yes
  add_copy research-schema-check postgres_password postgres_password 999 999 0440 yes
}

add_serving_copies() {
  add_copy reverse-proxy lagrange_tls_cert tls/lagrange.crt 101 101 0440 no
  add_copy reverse-proxy lagrange_tls_key tls/lagrange.key 101 101 0440 no
  add_copy api-server db_app_password db_app_password 10001 10001 0440 yes
  add_copy api-server db_admin_password db_admin_password 10001 10001 0440 yes
  add_copy api-server db_audit_password db_audit_password 10001 10001 0440 yes
  add_copy api-server cursor_secret cursor_secret 10001 10001 0440 yes
  add_copy api-server session_secret session_secret 10001 10001 0440 yes
  add_copy api-server csrf_secret csrf_secret 10001 10001 0440 yes
  add_copy api-server auth0_client_secret auth0_client_secret 10001 10001 0440 yes
  add_copy research-worker db_research_password db_research_password 10001 10001 0440 yes
  add_copy recommendation-runner db_worker_password db_worker_password 10001 10001 0440 yes
  add_copy candidate-runner db_worker_password db_worker_password 10001 10001 0440 yes
  add_copy nt-backtest-worker-1 db_worker_password db_worker_password 10001 10001 0440 yes
  add_copy nt-backtest-worker-2 db_worker_password db_worker_password 10001 10001 0440 yes
  add_copy paper-scheduler db_app_password db_app_password 10001 10001 0440 yes
  add_copy paper-scheduler db_worker_password db_worker_password 10001 10001 0440 yes
  add_copy paper-scheduler db_admin_password db_admin_password 10001 10001 0440 yes
  add_copy paper-scheduler db_audit_password db_audit_password 10001 10001 0440 yes
}

case "$scope" in
  infrastructure)
    add_infrastructure_copies
    ;;
  serving-prereqs)
    add_serving_copies
    add_infrastructure_copies
    add_extra_source backup_encryption_key yes
    ;;
  backfill)
    add_infrastructure_copies
    add_copy research-worker db_research_password db_research_password 10001 10001 0440 yes
    add_copy research-worker kis_app_key kis_app_key 10001 10001 0440 yes
    add_copy research-worker kis_app_secret kis_app_secret 10001 10001 0440 yes
    ;;
  release)
    add_serving_copies
    add_infrastructure_copies
    add_copy research-worker kis_app_key kis_app_key 10001 10001 0440 yes
    add_copy research-worker kis_app_secret kis_app_secret 10001 10001 0440 yes
    add_extra_source backup_encryption_key yes
    ;;
esac

placeholder_pattern='REPLACE_WITH|CHANGE_ME|YOUR_|example|placeholder'
crypto_placeholder_pattern='placeholder|example|todo|change[-_ ]*me|change[-_ ]*this|replace[-_ ]*(me|this|with)|your[-_ ]*(client[-_ ]*)?secret|secret[-_ ]*here|auth0[-_ ]*client[-_ ]*secret|<[^>]+>|\$\{[^}]+\}'
crypto_scope=no
crypto_source_names=(session_secret csrf_secret cursor_secret backup_encryption_key)
if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
  crypto_scope=yes
fi

is_crypto_source() {
  local source=$1 name
  for name in "${crypto_source_names[@]}"; do
    [ "$source" = "$name" ] && return 0
  done
  return 1
}

check_crypto_source_shape() {
  local source=$1 input="$source_dir/$source" byte_count
  byte_count=$(wc -c <"$input") || die "cannot inspect crypto source: $source"
  [ "$byte_count" -eq 64 ] || die "crypto source $source must contain exactly 64 lowercase hex characters with no newline or placeholder"
  LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$input" ||
    die "crypto source $source must contain exactly 64 lowercase hex characters with no newline or placeholder"
  ! LC_ALL=C grep -Eiq -- "$crypto_placeholder_pattern" "$input" ||
    die "crypto source $source must contain exactly 64 lowercase hex characters with no newline or placeholder"
}

preflight_source() {
  local source=$1 single_line=$2 input="$source_dir/$source" source_mode
  check_path "$input" "secret source $source"
  [ ! -L "$input" ] || die "secret source must not be a symlink: $source"
  [ -f "$input" ] || die "missing secret source: $source"
  [ -s "$input" ] || die "secret source is empty: $source"
  source_mode=$(stat -c '%a' -- "$input") || die "cannot stat secret source: $source"
  case "$source_mode" in
    400|600) ;;
    *) die "secret source must be mode 0400 or 0600: $source" ;;
  esac
  if [ "$single_line" = yes ]; then
    [ "$(wc -l < "$input")" -eq 0 ] || die "secret contains LF: $source"
    if LC_ALL=C grep -Fq $'\r' "$input"; then
      die "secret contains CR: $source"
    fi
  fi
  if LC_ALL=C grep -Eiq "$placeholder_pattern" -- "$input"; then
    die "secret source contains a placeholder: $source"
  fi
  if [ "$crypto_scope" = yes ] && is_crypto_source "$source"; then
    check_crypto_source_shape "$source"
  fi
}

preflight_output() {
  local service=$1 target=$2
  local output_dir="$runtime_dir/$service" output="$runtime_dir/$service/$target"
  check_path "$output_dir" "runtime service directory $service"
  if [ -e "$output_dir" ] && [ ! -d "$output_dir" ]; then
    die "runtime service path is not a directory: $service"
  fi
  check_path "$output" "runtime secret $service/$target"
  if [ -e "$output" ] && [ ! -f "$output" ]; then
    die "runtime secret path is not a regular file: $service/$target"
  fi
}

# Preflight every selected source and destination before creating the runtime
# tree or replacing a single existing target. This prevents a missing/malformed
# source or unsafe path later in the inventory from leaving a partial scope.
for spec in "${copy_specs[@]}"; do
  read -r service target source uid gid mode single_line <<<"$spec"
  preflight_source "$source" "$single_line"
  preflight_output "$service" "$target"
done
for spec in "${extra_source_specs[@]}"; do
  read -r source single_line <<<"$spec"
  preflight_source "$source" "$single_line"
done

if [ "$crypto_scope" = yes ]; then
  for ((left = 0; left < ${#crypto_source_names[@]}; left++)); do
    for ((right = left + 1; right < ${#crypto_source_names[@]}; right++)); do
      left_name=${crypto_source_names[left]}
      right_name=${crypto_source_names[right]}
      if cmp -s -- "$source_dir/$left_name" "$source_dir/$right_name"; then
        die "crypto source secrets must be distinct: $left_name conflicts with $right_name"
      fi
    done
  done
fi

mkdir -p -- "$runtime_dir"
check_path "$runtime_dir" runtime-directory
chmod 0750 -- "$runtime_dir"

copy_secret() {
  local service=$1 target=$2 source=$3 uid=$4 gid=$5 mode=$6
  local input="$source_dir/$source" output_dir="$runtime_dir/$service" output="$runtime_dir/$service/$target"
  check_path "$input" "secret source $source"
  check_path "$output_dir" "runtime service directory $service"
  check_path "$output" "runtime secret $service/$target"
  install -d -o "$uid" -g "$gid" -m 0750 -- "$output_dir"
  check_path "$output_dir" "runtime service directory $service"
  install -o "$uid" -g "$gid" -m "$mode" -- "$input" "$output"
  check_path "$output" "runtime secret $service/$target"
}

for spec in "${copy_specs[@]}"; do
  read -r service target source uid gid mode single_line <<<"$spec"
  copy_secret "$service" "$target" "$source" "$uid" "$gid" "$mode"
done

echo "provision-runtime-secrets: installed scope=$scope service-specific copies under $runtime_dir"
