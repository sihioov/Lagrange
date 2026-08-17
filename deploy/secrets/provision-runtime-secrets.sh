#!/usr/bin/env bash
# Create service-specific Docker secret copies with native-Linux ownership.
#
# Docker Compose's file-backed secrets are bind mounts on Linux; their
# long-syntax mode fields are not a substitute for host permissions.  This
# script therefore copies each operator secret into a directory owned by the
# UID that consumes it.  Run as root (or through the host secret manager).
# The backfill scope installs only the DB/bootstrap/schema/research-worker
# copies needed before curated dataset approval; release scope installs the
# complete serving inventory and remains the default.
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
Usage: deploy/secrets/provision-runtime-secrets.sh [--scope backfill|release]

  --scope backfill  Install only PostgreSQL/bootstrap/schema/research-worker
                    runtime copies for the pre-approval KIS backfill.
  --scope release   Install every Compose service secret (default).
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs backfill or release'
      scope=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done
case "$scope" in
  backfill|release) ;;
  *) die '--scope must be backfill or release' ;;
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
mkdir -p -- "$runtime_dir"
check_path "$runtime_dir" runtime-directory
chmod 0750 -- "$runtime_dir"

copy_secret() {
  local service=$1 target=$2 source=$3 uid=$4 gid=$5 mode=$6 single_line=$7
  local input="$source_dir/$source" output_dir="$runtime_dir/$service" output="$runtime_dir/$service/$target"
  check_path "$input" "secret source $source"
  [ -f "$input" ] || die "missing secret source: $source"
  if [ "$single_line" = yes ]; then
    [ "$(wc -l < "$input")" -eq 0 ] || die "secret contains LF: $source"
    if LC_ALL=C grep -Fq $'\r' "$input"; then
      die "secret contains CR: $source"
    fi
  fi
  check_path "$output_dir" "runtime service directory $service"
  check_path "$output" "runtime secret $service/$target"
  install -d -o "$uid" -g "$gid" -m 0750 -- "$output_dir"
  check_path "$output_dir" "runtime service directory $service"
  install -o "$uid" -g "$gid" -m "$mode" -- "$input" "$output"
  check_path "$output" "runtime secret $service/$target"
}

# service                         target                    source                         uid   gid   mode  one-line
if [ "$scope" = release ]; then
  copy_secret reverse-proxy         lagrange_tls_cert         tls/lagrange.crt               101   101   0440  no
  copy_secret reverse-proxy         lagrange_tls_key          tls/lagrange.key               101   101   0440  no
  copy_secret api-server            db_app_password           db_app_password                10001 10001 0440  yes
  copy_secret api-server            db_admin_password         db_admin_password              10001 10001 0440  yes
  copy_secret api-server            db_audit_password         db_audit_password              10001 10001 0440  yes
  copy_secret api-server            cursor_secret             cursor_secret                  10001 10001 0440  yes
  copy_secret api-server            session_secret            session_secret                 10001 10001 0440  yes
  copy_secret api-server            csrf_secret               csrf_secret                    10001 10001 0440  yes
  copy_secret api-server            auth0_client_secret       auth0_client_secret            10001 10001 0440  yes
fi
# Both one-shot database jobs run as UID/GID 999 in deploy/db/Dockerfile.
# Keep their private copies readable by that non-root identity only.
copy_secret db-role-bootstrap     postgres_password         postgres_password              999   999   0400  yes
copy_secret db-role-bootstrap     db_migration_owner_password db_migration_owner_password  999   999   0400  yes
copy_secret db-role-bootstrap     db_app_password           db_app_password                999   999   0400  yes
copy_secret db-role-bootstrap     db_worker_password        db_worker_password             999   999   0400  yes
copy_secret db-role-bootstrap     db_audit_password         db_audit_password              999   999   0400  yes
copy_secret db-role-bootstrap     db_research_password      db_research_password           999   999   0400  yes
copy_secret db-role-bootstrap     db_admin_password          db_admin_password              999   999   0400  yes
copy_secret db-migrate             db_migration_owner_password db_migration_owner_password  999   999   0400  yes
copy_secret postgres               postgres_password         postgres_password              999   999   0440  yes
copy_secret research-schema-check  postgres_password         postgres_password              999   999   0440  yes
copy_secret research-worker        db_research_password      db_research_password           10001 10001 0440  yes
copy_secret research-worker        kis_app_key               kis_app_key                    10001 10001 0440  yes
copy_secret research-worker        kis_app_secret            kis_app_secret                 10001 10001 0440  yes
if [ "$scope" = release ]; then
  copy_secret recommendation-runner  db_worker_password        db_worker_password             10001 10001 0440  yes
  copy_secret candidate-runner       db_worker_password        db_worker_password             10001 10001 0440  yes
  copy_secret nt-backtest-worker-1   db_worker_password        db_worker_password             10001 10001 0440  yes
  copy_secret nt-backtest-worker-2   db_worker_password        db_worker_password             10001 10001 0440  yes
  copy_secret paper-scheduler        db_app_password           db_app_password                10001 10001 0440  yes
  copy_secret paper-scheduler        db_worker_password        db_worker_password             10001 10001 0440  yes
  copy_secret paper-scheduler        db_admin_password          db_admin_password              10001 10001 0440  yes
  copy_secret paper-scheduler        db_audit_password          db_audit_password              10001 10001 0440  yes
fi

echo "provision-runtime-secrets: installed scope=$scope service-specific copies under $runtime_dir"
