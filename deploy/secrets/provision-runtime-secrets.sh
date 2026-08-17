#!/usr/bin/env bash
# Create service-specific Docker secret copies with native-Linux ownership.
#
# Docker Compose's file-backed secrets are bind mounts on Linux; their
# long-syntax mode fields are not a substitute for host permissions.  This
# script therefore copies each operator secret into a directory owned by the
# UID that consumes it.  Run as root (or through the host secret manager).
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source_dir=${LAGRANGE_SECRET_SOURCE_DIR:-$script_dir}
runtime_dir=${LAGRANGE_RUNTIME_SECRET_DIR:-$source_dir/runtime}

die() {
  echo "provision-runtime-secrets: $*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || die "must run as root to assign service UID ownership"
[ -d "$source_dir" ] || die "source directory is missing: $source_dir"
[ ! -L "$source_dir" ] || die "source directory must not be a symlink"
if [ -e "$runtime_dir" ] && [ -L "$runtime_dir" ]; then
  die "runtime directory must not be a symlink"
fi
mkdir -p -- "$runtime_dir"
chmod 0750 -- "$runtime_dir"

copy_secret() {
  local service=$1 target=$2 source=$3 uid=$4 gid=$5 mode=$6 single_line=$7
  local input="$source_dir/$source" output_dir="$runtime_dir/$service" output="$runtime_dir/$service/$target"
  [ -f "$input" ] || die "missing secret source: $source"
  [ ! -L "$input" ] || die "secret source must not be a symlink: $source"
  if [ "$single_line" = yes ]; then
    [ "$(wc -l < "$input")" -eq 0 ] || die "secret contains LF: $source"
    if LC_ALL=C grep -Fq $'\r' "$input"; then
      die "secret contains CR: $source"
    fi
  fi
  [ ! -L "$output_dir" ] || die "runtime service directory must not be a symlink: $service"
  [ ! -L "$output" ] || die "runtime secret must not be a symlink: $service/$target"
  install -d -o "$uid" -g "$gid" -m 0750 -- "$output_dir"
  install -o "$uid" -g "$gid" -m "$mode" -- "$input" "$output"
}

# service                         target                    source                         uid   gid   mode  one-line
copy_secret reverse-proxy         lagrange_tls_cert         tls/lagrange.crt               101   101   0440  no
copy_secret reverse-proxy         lagrange_tls_key          tls/lagrange.key               101   101   0440  no
copy_secret api-server            db_app_password           db_app_password                10001 10001 0440  yes
copy_secret api-server            db_admin_password         db_admin_password              10001 10001 0440  yes
copy_secret api-server            db_audit_password         db_audit_password              10001 10001 0440  yes
copy_secret api-server            cursor_secret             cursor_secret                  10001 10001 0440  yes
copy_secret api-server            session_secret            session_secret                 10001 10001 0440  yes
copy_secret api-server            csrf_secret               csrf_secret                    10001 10001 0440  yes
copy_secret api-server            auth0_client_secret       auth0_client_secret            10001 10001 0440  yes
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
copy_secret research-worker        krx_api_key               krx_api_key                    10001 10001 0440  yes
copy_secret recommendation-runner  db_worker_password        db_worker_password             10001 10001 0440  yes
copy_secret candidate-runner       db_worker_password        db_worker_password             10001 10001 0440  yes
copy_secret nt-backtest-worker-1   db_worker_password        db_worker_password             10001 10001 0440  yes
copy_secret nt-backtest-worker-2   db_worker_password        db_worker_password             10001 10001 0440  yes
copy_secret paper-scheduler        db_app_password           db_app_password                10001 10001 0440  yes
copy_secret paper-scheduler        db_worker_password        db_worker_password             10001 10001 0440  yes
copy_secret paper-scheduler        db_admin_password          db_admin_password              10001 10001 0440  yes
copy_secret paper-scheduler        db_audit_password          db_audit_password              10001 10001 0440  yes

echo "provision-runtime-secrets: installed service-specific copies under $runtime_dir"
