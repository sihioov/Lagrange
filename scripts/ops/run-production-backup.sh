#!/usr/bin/env bash
# Encrypted PostgreSQL + Raw + Curated production backup runner.
# Default plan and --check are non-mutating. --run creates and verifies one set;
# --verify-latest performs an isolated restore verification only.
set -euo pipefail
umask 077

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
mode=plan
mode_seen=0
config_file=${LAGRANGE_BACKUP_CONFIG:-/etc/lagrange/production-backup.conf}

usage() {
  cat <<'EOF'
Usage: run-production-backup.sh [--plan|--check|--run|--verify-latest]
       [--config-file ABSOLUTE_PATH]

--plan          Print the backup contract without reading protected config (default).
--check         Root-only read-only config/source/key/metadata validation; no Docker.
--run           Root-only encrypted backup, isolated restore verification, then bounded prune.
--verify-latest Root-only repeat of the isolated restore verification for latest.

No KIS, order/account, Auth0, TLS, or application API call is made. Secrets are
never placed in argv, environment, archives, manifests, metrics, or output.
EOF
}
die() { echo "run-production-backup: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--check|--run|--verify-latest)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}; mode_seen=1; shift ;;
    --config-file) [ "$#" -ge 2 ] || die '--config-file needs a path'; config_file=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

if [ "$mode" = plan ]; then
  cat <<EOF
PRODUCTION_BACKUP_PLAN
  config=$config_file (root:root 0600; read only by check/run/verify)
  classes=PostgreSQL custom dump + exact Raw archive + exact Curated archive
  encryption=OpenSSL AES-256-CBC PBKDF2 using backup_encryption_key file
  verification=hash/decrypt/extract + isolated networkless PostgreSQL restore
  retention=age and MAX_TOTAL_BYTES, preserving MIN_KEEP verified sets
PLAN_ONLY: no protected config/key read, Docker, DB, file write, prune, or service action
EOF
  exit 0
fi
[ "$(id -u)" -eq 0 ] || die "--$mode must run as root"

safe_path() {
  local path=$1 label=$2 probe
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in */../*|*/..) die "$label must not contain '..': $path" ;; esac
  case "$path" in /|/etc|/opt|/var|/var/lib|/srv|/tmp|/run) die "$label is too broad: $path" ;; esac
  probe=${path%/}; [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}; [ -n "$probe" ] || probe=/
  done
}

check_secure_parent() {
  local path=$1 label=$2 probe metadata uid bits
  probe=$path
  while [ ! -e "$probe" ]; do probe=${probe%/*}; [ -n "$probe" ] || probe=/; done
  [ -d "$probe" ] && [ ! -L "$probe" ] || die "$label ancestor is unsafe: $probe"
  metadata=$(stat -c '%u:%a' -- "$probe") || die "cannot inspect $label ancestor"
  uid=${metadata%%:*}; bits=$((8#${metadata#*:}))
  [ "$uid" = 0 ] || die "$label ancestor must be root-owned"
  (( (bits & 0022) == 0 )) || die "$label ancestor must not be group/other writable"
}

safe_path "$config_file" config-file
check_secure_parent "$(dirname -- "$config_file")" config-file
[ -f "$config_file" ] && [ ! -L "$config_file" ] || die 'config file must be a regular non-symlink file'
[ "$(stat -c '%u:%g:%a' -- "$config_file")" = 0:0:600 ] || die 'config file must be root:root mode 0600'

declare -A cfg=()
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in ''|\#*) continue ;; *=*) ;; *) die 'config contains malformed line' ;; esac
  key=${line%%=*}; value=${line#*=}
  case "$key" in
    BACKUP_ROOT|DATA_ROOT|COMPOSE_FILE|COMPOSE_ENV_FILE|COMPOSE_PROJECT|LAGRANGE_CODE_COMMIT|KEY_FILE|LOCK_FILE|METRICS_FILE|MAX_TOTAL_BYTES|MIN_FREE_BYTES|RETENTION_DAYS|MIN_KEEP|POSTGRES_SERVICE|POSTGRES_IMAGE) ;;
    *) die "config contains unsupported key: $key" ;;
  esac
  [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || die 'config key shape is invalid'
  case "$value" in *$'\r'*|*$'\n'*) die "config value contains a line break: $key" ;; esac
  case "$value" in *[[:space:]]*) die "config value contains whitespace: $key" ;; esac
  [ -z "${cfg[$key]+set}" ] || die "config repeats key: $key"
  cfg[$key]=$value
done <"$config_file"
for key in BACKUP_ROOT DATA_ROOT COMPOSE_FILE COMPOSE_ENV_FILE COMPOSE_PROJECT \
  LAGRANGE_CODE_COMMIT KEY_FILE LOCK_FILE METRICS_FILE MAX_TOTAL_BYTES \
  MIN_FREE_BYTES RETENTION_DAYS MIN_KEEP POSTGRES_SERVICE POSTGRES_IMAGE; do
  [ -n "${cfg[$key]:-}" ] || die "config is missing: $key"
done

backup_root=${cfg[BACKUP_ROOT]}
data_root=${cfg[DATA_ROOT]}
compose_file=${cfg[COMPOSE_FILE]}
compose_env=${cfg[COMPOSE_ENV_FILE]}
compose_project=${cfg[COMPOSE_PROJECT]}
code_commit=${cfg[LAGRANGE_CODE_COMMIT]}
key_file=${cfg[KEY_FILE]}
lock_file=${cfg[LOCK_FILE]}
metrics_file=${cfg[METRICS_FILE]}
max_total=${cfg[MAX_TOTAL_BYTES]}
min_free=${cfg[MIN_FREE_BYTES]}
retention_days=${cfg[RETENTION_DAYS]}
min_keep=${cfg[MIN_KEEP]}
postgres_service=${cfg[POSTGRES_SERVICE]}
postgres_image=${cfg[POSTGRES_IMAGE]}

for pair in "$backup_root:BACKUP_ROOT" "$data_root:DATA_ROOT" "$compose_file:COMPOSE_FILE" \
  "$compose_env:COMPOSE_ENV_FILE" "$key_file:KEY_FILE" "$lock_file:LOCK_FILE" \
  "$metrics_file:METRICS_FILE"; do
  safe_path "${pair%%:*}" "${pair#*:}"
done
[[ "$compose_project" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,62}$ ]] || die 'COMPOSE_PROJECT shape is invalid'
[[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] || die 'LAGRANGE_CODE_COMMIT must be exact 40 lowercase hex'
[ "$code_commit" != 0000000000000000000000000000000000000000 ] || die 'LAGRANGE_CODE_COMMIT must not be zero'
[[ "$max_total" =~ ^[1-9][0-9]*$ ]] || die 'MAX_TOTAL_BYTES must be positive integer'
[[ "$min_free" =~ ^[1-9][0-9]*$ ]] || die 'MIN_FREE_BYTES must be positive integer'
[[ "$retention_days" =~ ^[1-9][0-9]*$ ]] || die 'RETENTION_DAYS must be positive integer'
[[ "$min_keep" =~ ^[1-9][0-9]*$ ]] || die 'MIN_KEEP must be positive integer'
[[ "$postgres_service" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,62}$ ]] || die 'POSTGRES_SERVICE shape is invalid'
[[ "$postgres_image" =~ ^postgres:[0-9]+([.][0-9]+)?$ ]] || die 'POSTGRES_IMAGE must be an explicit official postgres tag'
[ -d "$data_root/raw" ] && [ ! -L "$data_root/raw" ] || die 'Raw source directory missing or unsafe'
[ -d "$data_root/curated" ] && [ ! -L "$data_root/curated" ] || die 'Curated source directory missing or unsafe'
[ -f "$compose_file" ] && [ ! -L "$compose_file" ] || die 'Compose file missing or unsafe'
[ -f "$compose_env" ] && [ ! -L "$compose_env" ] || die 'Compose env missing or unsafe'
[ "$(stat -c '%u:%g:%a' -- "$compose_env")" = 0:0:600 ] || die 'Compose env must be root:root mode 0600'
[ -f "$key_file" ] && [ ! -L "$key_file" ] || die 'backup key file missing or unsafe'
[ "$(stat -c '%u:%g:%a' -- "$key_file")" = 0:0:600 ] || die 'backup key must be root:root mode 0600'
key_value=$(tr -d '\r\n' <"$key_file")
[[ "$key_value" =~ ^[0-9a-f]{64}$ ]] || die 'backup key must be exactly one 256-bit lowercase hex value with no newline'
[ "$(wc -c <"$key_file")" -eq 64 ] || die 'backup key must have no trailing newline'
unset key_value
if find "$data_root/raw" "$data_root/curated" -type l -print -quit | grep -q .; then
  die 'Raw/Curated backup sources must not contain symlinks'
fi

check_secure_parent "$backup_root" BACKUP_ROOT
check_secure_parent "$(dirname -- "$lock_file")" LOCK_FILE
check_secure_parent "$(dirname -- "$metrics_file")" METRICS_FILE
check_secure_parent "$(dirname -- "$compose_env")" COMPOSE_ENV_FILE
check_secure_parent "$(dirname -- "$key_file")" KEY_FILE

if [ "$mode" = check ]; then
  echo "PRODUCTION_BACKUP_CHECK: PASS commit=$code_commit classes=postgres,raw,curated"
  exit 0
fi

command -v docker >/dev/null 2>&1 || die 'docker is required'
docker compose version >/dev/null 2>&1 || die 'Docker Compose v2 is required'
for command in openssl tar sha256sum flock mktemp find sort du df; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

install -d -o 0 -g 0 -m 0700 -- "$backup_root" "$(dirname -- "$lock_file")" "$(dirname -- "$metrics_file")"
exec 9>>"$lock_file" || die 'cannot open backup lock'
chmod 0600 "$lock_file"
flock -n 9 || die 'another production backup/verification holds the lock'

# Compose interpolates the whole file even for `exec` against one service, so
# every required variable must resolve or the backup aborts before reaching
# PostgreSQL. LAGRANGE_CODE_COMMIT is already supplied per-call from the
# validated config value; RANGE_RAW_BATCH_ID had no source at all. It belongs to
# the Stage5 range services, which this script never runs. Latent today only
# because production-backup.conf still pins a release predating the requirement.
export RANGE_RAW_BATCH_ID=${RANGE_RAW_BATCH_ID:-compose-config-disabled}
compose=(docker compose -p "$compose_project" --env-file "$compose_env" -f "$compose_file")
encrypt() { openssl enc -aes-256-cbc -salt -pbkdf2 -iter 200000 -pass "file:$key_file"; }
decrypt() { openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -pass "file:$key_file"; }

validate_archive_names() {
  python3 -c '
import sys
for line in sys.stdin:
    value=line.rstrip("\n")
    if value.startswith("/") or any(part == ".." for part in value.split("/")):
        raise SystemExit("unsafe archive member")
' || die 'archive contains an unsafe member path'
}

verify_set() {
  local set_dir=$1 temporary db_dump
  [ -d "$set_dir" ] && [ ! -L "$set_dir" ] || die 'verification set is unsafe'
  [ -f "$set_dir/manifest.sha256" ] || die 'backup manifest missing'
  (cd "$set_dir" && sha256sum -c manifest.sha256 >/dev/null) || die 'encrypted archive hash verification failed'
  temporary=$(mktemp -d -- "$backup_root/.verify.XXXXXX") || die 'cannot create restore verification staging'
  chmod 0700 "$temporary"
  verify_tmp=$temporary
  decrypt <"$set_dir/postgres.dump.enc" >"$temporary/postgres.dump" || die 'PostgreSQL backup decryption failed'
  for class in raw curated; do
    mkdir "$temporary/$class"
    decrypt <"$set_dir/$class.tar.enc" | tar -tf - | validate_archive_names
    decrypt <"$set_dir/$class.tar.enc" | tar -xf - -C "$temporary/$class" || die "$class restore extraction failed"
  done
  db_dump=$temporary/postgres.dump
  # The parent remains 0700; the dump is briefly world-readable only through
  # the explicit read-only container bind mount, so uid 999 can restore it.
  chmod 0444 "$db_dump"
  docker run --rm --network none --read-only --user 999:999 \
    --tmpfs /tmp:rw,exec,size=1g -v "$db_dump:/verify.dump:ro" \
    "$postgres_image" bash -euc '
      export PGHOST=/tmp
      initdb -D /tmp/pgdata >/dev/null
      pg_ctl -D /tmp/pgdata \
        -o "-c listen_addresses= -c unix_socket_directories=/tmp" \
        -w start >/dev/null
      # pg_dump --no-owner/--no-privileges still preserves role references in
      # RLS policies and object definitions.  Recreate only the exact
      # application roles from bootstrap-roles.sh as inert cluster roles: no
      # password, no LOGIN, no privileges, and no network is available here.
      # An unexpected role reference therefore remains a fail-closed restore
      # error instead of being silently invented.
      psql -d postgres -v ON_ERROR_STOP=1 -c "
        CREATE ROLE migration_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
        CREATE ROLE app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
        CREATE ROLE worker NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
        CREATE ROLE audit_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
        CREATE ROLE research_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
        CREATE ROLE admin NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
      "
      createdb verify
      pg_restore --exit-on-error --no-owner --no-privileges -d verify /verify.dump
      restored_tables=$(psql -d verify -Atqc "
        SELECT table_name
          FROM information_schema.tables
         WHERE table_schema = current_schema()
      ")
      for required_table in _sqlx_migrations data_batches dataset_versions; do
        printf "%s\\n" "$restored_tables" | grep -Fxq "$required_table"
      done
      pg_ctl -D /tmp/pgdata -m fast -w stop >/dev/null
    ' || die 'isolated PostgreSQL restore verification failed'
  rm -rf -- "$temporary"
  verify_tmp=
}

list_sets() {
  local path name
  while IFS= read -r path; do
    name=${path##*/}
    [[ "$name" =~ ^backup-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}$ ]] || continue
    printf '%s\n' "$name"
  done < <(find "$backup_root" -mindepth 1 -maxdepth 1 -type d -name 'backup-*' -print)
}

latest_set() {
  list_sets | sort | tail -n1
}

verify_tmp=
stage=
cleanup() {
  [ -z "$verify_tmp" ] || [ ! -d "$verify_tmp" ] || rm -rf -- "$verify_tmp"
  [ -z "$stage" ] || [ ! -d "$stage" ] || rm -rf -- "$stage"
}
trap cleanup EXIT

if [ "$mode" = verify-latest ]; then
  latest=$(latest_set)
  [ -n "$latest" ] || die 'no completed production backup set exists'
  verify_set "$backup_root/$latest"
  verified_tmp=$(mktemp -- "$backup_root/$latest/.VERIFIED.XXXXXX") || die 'cannot stage verification marker'
  chmod 0600 "$verified_tmp"
  printf '%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$verified_tmp"
  mv -Tf -- "$verified_tmp" "$backup_root/$latest/VERIFIED"
  echo "PRODUCTION_BACKUP_VERIFY: PASS set=$latest isolated=true"
  exit 0
fi

available=$(df -PB1 "$backup_root" | awk 'NR==2 {print $4}')
[[ "$available" =~ ^[0-9]+$ ]] || die 'cannot determine backup filesystem free bytes'
[ "$available" -ge "$min_free" ] || die 'backup filesystem is below MIN_FREE_BYTES before backup'

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
set_name=backup-$timestamp-$code_commit
final=$backup_root/$set_name
[ ! -e "$final" ] && [ ! -L "$final" ] || die 'refusing to overwrite an existing backup set'
stage=$(mktemp -d -- "$backup_root/.staging.$set_name.XXXXXX") || die 'cannot create backup staging directory'
chmod 0700 "$stage"

started=$(date -u +%s)
LAGRANGE_CODE_COMMIT=$code_commit "${compose[@]}" exec -T "$postgres_service" \
  sh -euc 'pg_dump --format=custom --no-owner --no-privileges -U "$POSTGRES_USER" -d "$POSTGRES_DB"' |
  encrypt >"$stage/postgres.dump.enc" || die 'PostgreSQL dump/encryption failed'
tar -C "$data_root/raw" -cf - . | encrypt >"$stage/raw.tar.enc" || die 'Raw archive/encryption failed'
tar -C "$data_root/curated" -cf - . | encrypt >"$stage/curated.tar.enc" || die 'Curated archive/encryption failed'
(cd "$stage" && sha256sum postgres.dump.enc raw.tar.enc curated.tar.enc >manifest.sha256)
cat >"$stage/metadata" <<EOF
format=LAGRANGE_PRODUCTION_BACKUP_V1
created_at=${timestamp}
code_commit=${code_commit}
classes=postgres,raw,curated
encryption=AES-256-CBC-PBKDF2-200000
EOF
chmod 0600 "$stage"/*
verify_set "$stage"
printf '%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$stage/VERIFIED"
printf '%s\n' LAGRANGE_PRODUCTION_BACKUP_V1 >"$stage/COMPLETE"
chmod 0600 "$stage/VERIFIED" "$stage/COMPLETE"
mv -T -- "$stage" "$final" || die 'cannot atomically publish backup set'
stage=

latest_tmp=$backup_root/.latest.$$
[ ! -e "$backup_root/latest" ] || [ -L "$backup_root/latest" ] || die 'refusing to overwrite non-symlink latest path'
if [ -L "$backup_root/latest" ]; then
  [[ "$(readlink -- "$backup_root/latest")" =~ ^backup-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}$ ]] || die 'latest symlink has an unsafe target'
fi
ln -s -- "$set_name" "$latest_tmp"
mv -Tf -- "$latest_tmp" "$backup_root/latest"

# Prune only complete, verified, strictly named sets under the configured root.
# MIN_KEEP wins over both age and byte caps; an unsatisfiable cap is an error.
mapfile -t sets < <(list_sets | sort)
now_epoch=$(date -u +%s)
while [ "${#sets[@]}" -gt "$min_keep" ]; do
  oldest=${sets[0]}
  oldest_path=$backup_root/$oldest
  [ ! -L "$oldest_path" ] && [ -f "$oldest_path/COMPLETE" ] && [ -f "$oldest_path/VERIFIED" ] || die 'retention encountered an unverified or unsafe set'
  set_paths=(); for name in "${sets[@]}"; do set_paths+=("$backup_root/$name"); done
  total=$(du -sb -- "${set_paths[@]}" | awk '{s+=$1} END {print s+0}')
  mtime=$(stat -c %Y -- "$oldest_path")
  age_days=$(( (now_epoch - mtime) / 86400 ))
  if [ "$total" -le "$max_total" ] && [ "$age_days" -le "$retention_days" ]; then
    break
  fi
  rm -rf -- "$oldest_path"
  echo "PRODUCTION_BACKUP_PRUNED set=$oldest recoverable=false reason=retention"
  sets=("${sets[@]:1}")
done
set_paths=(); for name in "${sets[@]}"; do set_paths+=("$backup_root/$name"); done
total=$(du -sb -- "${set_paths[@]}" | awk '{s+=$1} END {print s+0}')
[ "$total" -le "$max_total" ] || die 'MAX_TOTAL_BYTES cannot be satisfied while preserving MIN_KEEP'

duration=$(( $(date -u +%s) - started ))
metrics_tmp=$(mktemp -- "$(dirname -- "$metrics_file")/.production-backup.prom.XXXXXX") || die 'cannot stage backup metrics'
cat >"$metrics_tmp" <<EOF
lagrange_backup_last_success_timestamp_seconds $(date -u +%s)
lagrange_backup_duration_seconds $duration
lagrange_backup_total_bytes $total
lagrange_backup_retained_sets ${#sets[@]}
EOF
chmod 0600 "$metrics_tmp"
mv -Tf -- "$metrics_tmp" "$metrics_file"
echo "PRODUCTION_BACKUP_RUN: PASS set=$set_name verified=true encrypted=true duration_seconds=$duration"
