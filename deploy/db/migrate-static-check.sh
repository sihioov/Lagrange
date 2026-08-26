#!/usr/bin/env bash
# Fake-secret regression checks for the migration URL boundary.
#
# This never contacts PostgreSQL. A fake sqlx executable captures the URL that
# migrate.sh exports, allowing CI to verify that component-mode credentials
# are percent-encoded without placing a real secret in a process argument or
# log. It also verifies plaintext DATABASE_URL remains rejected.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
compose="$root/deploy/compose/compose.yml"
dockerfile="$root/deploy/db/Dockerfile"
provision="$root/deploy/secrets/provision-runtime-secrets.sh"

die_static() {
  echo "MIGRATE_STATIC: $*" >&2
  exit 1
}

[ -f "$dockerfile" ] || die_static 'missing database one-shot Dockerfile'
grep -Fxq 'USER 999:999' "$dockerfile" \
  || die_static 'database one-shot image must remain non-root UID/GID 999:999'
grep -Fq 'COPY configs/strategies/baseline-v1.json /opt/lagrange/configs/strategies/baseline-v1.json' "$dockerfile" \
  || die_static 'database one-shot image must contain the pinned baseline catalog'
grep -Fq 'COPY deploy/db/sync-baseline-strategy-catalog.sql /opt/lagrange/db/sync-baseline-strategy-catalog.sql' "$dockerfile" \
  || die_static 'database one-shot image must contain the catalog sync contract'
[ -f "$compose" ] || die_static 'missing Compose file'
[ -x "$provision" ] || die_static 'missing executable secret provisioner'

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { in_service=1; print; next }
    in_service && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    in_service { print }
  ' "$compose"
}

# The image, host-side copy, and Compose long syntax must all agree. In
# particular, do not "fix" an unreadable 0400 secret by running the image as
# root: both one-shot jobs are deliberately non-root UID/GID 999.
for service in db-role-bootstrap db-migrate; do
  grep -Eq "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]].*[[:space:]]999[[:space:]]+999[[:space:]]+0400[[:space:]]+yes$" "$provision" \
    || die_static "provisioner must install $service secrets as 999:999 mode 0400"
  expected_count=1
  [ "$service" = db-role-bootstrap ] && expected_count=7
  provision_count=$(grep -Ec "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]]" "$provision" || true)
  [ "$provision_count" -eq "$expected_count" ] \
    || die_static "$service provisioner entry count changed (expected $expected_count)"
  block=$(service_block "$service")
  [ -n "$block" ] || die_static "missing Compose service block: $service"
  if grep -Eq 'uid: "0"|gid: "0"' <<<"$block"; then
    die_static "$service must not mount root-owned secrets"
  fi
  uid_count=$(grep -Ec '^[[:space:]]+uid: "999"$' <<<"$block" || true)
  gid_count=$(grep -Ec '^[[:space:]]+gid: "999"$' <<<"$block" || true)
  mode_count=$(grep -Ec '^[[:space:]]+mode: 0400$' <<<"$block" || true)
  [ "$uid_count" -eq "$expected_count" ] \
    || die_static "$service must mount $expected_count secrets with uid 999"
  [ "$gid_count" -eq "$expected_count" ] \
    || die_static "$service must mount $expected_count secrets with gid 999"
  [ "$mode_count" -eq "$expected_count" ] \
    || die_static "$service must mount $expected_count secrets with mode 0400"
done

# Keep the cluster administrator credential restricted to bootstrap and the
# distinct migration-owner credential restricted to the migration one-shot.
bootstrap_block=$(service_block db-role-bootstrap)
migrate_block=$(service_block db-migrate)
grep -Fq 'DB_ADMIN_PASSWORD_FILE: /run/secrets/postgres_password' <<<"$bootstrap_block" \
  || die_static 'bootstrap must read the PostgreSQL administrator secret'
grep -Fq 'DB_MIGRATION_OWNER_PASSWORD_FILE: /run/secrets/db_migration_owner_password' <<<"$bootstrap_block" \
  || die_static 'bootstrap must receive a separate migration-owner secret'
grep -Fq 'DB_PASSWORD_FILE: /run/secrets/db_migration_owner_password' <<<"$migrate_block" \
  || die_static 'migration one-shot must read migration-owner secret'
grep -Fq 'target: postgres_password' <<<"$bootstrap_block" \
  || die_static 'bootstrap administrator secret target is missing'
grep -Fq 'target: db_migration_owner_password' <<<"$bootstrap_block" \
  || die_static 'bootstrap migration-owner secret target is missing'
grep -Fq 'target: db_migration_owner_password' <<<"$migrate_block" \
  || die_static 'migration-owner secret target is missing'

for script in "$root/deploy/db/bootstrap-roles.sh" "$root/deploy/db/migrate.sh"; do
  [ -f "$script" ] || die_static "missing database runtime script: $script"
  grep -Fq '[ ! -L "$path" ]' "$script" \
    || die_static "runtime script must reject symlinked secret files: $script"
  grep -Fq "\\r'" "$script" \
    || die_static "runtime script must reject CR-containing secrets: $script"
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-migrate-check.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

mkdir -p "$tmp/bin" "$tmp/migrations"
printf '%s\n' \
  '#!/bin/sh' \
  '[ "${FAIL_SQLX:-0}" = 0 ] || exit 42' \
  'printf "%s" "${DATABASE_URL-}" >"${CAPTURE_PATH:?}"' \
  ': >"${CAPTURE_PATH}.sqlx-done"' >"$tmp/bin/sqlx"
printf '%s\n' \
  '#!/bin/sh' \
  '[ -e "${CAPTURE_PATH:?}.sqlx-done" ] || exit 43' \
  'printf "%s|%s|%s|%s|%s" "${PGHOST-}" "${PGPORT-}" "${PGUSER-}" "${PGPASSWORD-}" "${PGDATABASE-}" >"${CAPTURE_PATH}.libpq"' \
  ': >"${CAPTURE_PATH}.psql"' \
  "printf '%s\\n' 'STRATEGY_CATALOG_SYNC: PASS strategies=5'" >"$tmp/bin/psql"
chmod 0755 "$tmp/bin/sqlx" "$tmp/bin/psql"

secret="p@ss:word /%?#[]!\$&'()*+,;="
printf '%s' "$secret" >"$tmp/password"

capture="$tmp/database-url"
PATH="$tmp/bin:$PATH" \
CAPTURE_PATH="$capture" \
DB_HOST='db.example' \
DB_PORT=5432 \
DB_NAME='db/name?' \
DB_USER='u@ser+' \
DB_PASSWORD_FILE="$tmp/password" \
MIGRATIONS_DIR="$tmp/migrations" \
  bash "$root/deploy/db/migrate.sh"

expected='postgresql://u%40ser%2B:p%40ss%3Aword%20%2F%25%3F%23%5B%5D%21%24%26%27%28%29%2A%2B%2C%3B%3D@db.example:5432/db%2Fname%3F'
actual=$(<"$capture")
[ "$actual" = "$expected" ] || {
  echo 'MIGRATE_STATIC: component URL encoding mismatch' >&2
  exit 1
}
[ -e "$capture.psql" ] || {
  echo 'MIGRATE_STATIC: catalog sync did not run after successful migrations' >&2
  exit 1
}
expected_libpq='db.example|5432|u@ser+|p@ss:word /%?#[]!$&'"'"'()*+,;=|db/name?'
actual_libpq=$(<"$capture.libpq")
[ "$actual_libpq" = "$expected_libpq" ] || {
  echo 'MIGRATE_STATIC: component libpq parameters mismatch' >&2
  exit 1
}

failed_capture="$tmp/failed-migration"
if FAIL_SQLX=1 \
  PATH="$tmp/bin:$PATH" \
  CAPTURE_PATH="$failed_capture" \
  DB_HOST='db.example' \
  DB_PORT=5432 \
  DB_NAME='lagrange' \
  DB_USER='migration_owner' \
  DB_PASSWORD_FILE="$tmp/password" \
  MIGRATIONS_DIR="$tmp/migrations" \
    bash "$root/deploy/db/migrate.sh" >/dev/null 2>&1; then
  echo 'MIGRATE_STATIC: failed sqlx migration was accepted' >&2
  exit 1
fi
[ ! -e "$failed_capture.psql" ] || {
  echo 'MIGRATE_STATIC: catalog sync ran after failed migrations' >&2
  exit 1
}

run_url_file_mode() {
  local file=$1 output=$2
  env -u DATABASE_URL -u DATABASE_URL_FILE -u MIGRATION_DATABASE_URL_FILE \
    PATH="$tmp/bin:$PATH" \
    CAPTURE_PATH="$output" \
    DATABASE_URL_FILE="$file" \
    MIGRATIONS_DIR="$tmp/migrations" \
    bash "$root/deploy/db/migrate.sh"
}

printf '%s' 'postgresql://uri%40user:p%40ss%2Fword@db.example:5444/db%2Fname' >"$tmp/database-url-valid"
run_url_file_mode "$tmp/database-url-valid" "$tmp/valid-url"
[ "$(<"$tmp/valid-url.libpq")" = 'db.example|5444|uri@user|p@ss/word|db/name' ] || {
  echo 'MIGRATE_STATIC: URL-file libpq parameters mismatch' >&2
  exit 1
}

printf '%s' 'postgresql://user:password@db.example:5432/lagrange?sslmode=require' >"$tmp/database-url-query"
if run_url_file_mode "$tmp/database-url-query" "$tmp/query-url" >/dev/null 2>&1; then
  echo 'MIGRATE_STATIC: URL-file query component was accepted' >&2
  exit 1
fi
[ ! -e "$tmp/query-url.psql" ] || {
  echo 'MIGRATE_STATIC: psql ran for rejected URL-file query component' >&2
  exit 1
}

for suffix in lf cr crlf; do
  file="$tmp/database-url-$suffix"
  case "$suffix" in
    lf) printf '%s\n' 'postgresql://db.example/lagrange' >"$file" ;;
    cr) printf '%s\r' 'postgresql://db.example/lagrange' >"$file" ;;
    crlf) printf '%s\r\n' 'postgresql://db.example/lagrange' >"$file" ;;
  esac
  output="$tmp/rejected-url-$suffix"
  if run_url_file_mode "$file" "$output" >"$tmp/stdout-$suffix" 2>"$tmp/stderr-$suffix"; then
    echo "MIGRATE_STATIC: DATABASE_URL_FILE accepted $suffix" >&2
    exit 1
  fi
  [ ! -e "$output" ] || {
    echo "MIGRATE_STATIC: sqlx ran for rejected $suffix URL file" >&2
    exit 1
  }
done

run_component_mode() {
  local password_file=$1 output=$2
  env -u DATABASE_URL -u DATABASE_URL_FILE -u MIGRATION_DATABASE_URL_FILE \
    PATH="$tmp/bin:$PATH" \
    CAPTURE_PATH="$output" \
    DB_HOST='db.example' \
    DB_PORT=5432 \
    DB_NAME='lagrange' \
    DB_USER='migration_owner' \
    DB_PASSWORD_FILE="$password_file" \
    MIGRATIONS_DIR="$tmp/migrations" \
    bash "$root/deploy/db/migrate.sh"
}

for suffix in lf cr crlf; do
  file="$tmp/password-$suffix"
  case "$suffix" in
    lf) printf '%s\n' 'fake-password' >"$file" ;;
    cr) printf '%s\r' 'fake-password' >"$file" ;;
    crlf) printf '%s\r\n' 'fake-password' >"$file" ;;
  esac
  output="$tmp/rejected-password-$suffix"
  if run_component_mode "$file" "$output" >"$tmp/password-stdout-$suffix" 2>"$tmp/password-stderr-$suffix"; then
    echo "MIGRATE_STATIC: DB_PASSWORD_FILE accepted $suffix" >&2
    exit 1
  fi
  [ ! -e "$output" ] || {
    echo "MIGRATE_STATIC: sqlx ran for rejected $suffix password file" >&2
    exit 1
  }
done

if DATABASE_URL='postgresql://plaintext:forbidden@db.example/lagrange' \
  MIGRATIONS_DIR="$tmp/migrations" \
  PATH="$tmp/bin:$PATH" \
  CAPTURE_PATH="$tmp/plaintext-url" \
  bash "$root/deploy/db/migrate.sh" >/dev/null 2>&1; then
  echo 'MIGRATE_STATIC: plaintext DATABASE_URL was accepted' >&2
  exit 1
fi

# The documented operator commands deliberately strip OpenSSL's trailing LF;
# strict secret readers reject every CR/LF byte, including a final newline.
command -v openssl >/dev/null 2>&1 || {
  echo 'MIGRATE_STATIC: openssl is required to verify documented generation commands' >&2
  exit 1
}
openssl rand -base64 32 | tr -d '\r\n' >"$tmp/generated-password"
openssl rand -hex 32 | tr -d '\r\n' >"$tmp/generated-cursor"
for generated in "$tmp/generated-password" "$tmp/generated-cursor"; do
  [ -s "$generated" ] || { echo "MIGRATE_STATIC: empty generated secret: $generated" >&2; exit 1; }
  [ "$(wc -l <"$generated")" -eq 0 ] || {
    echo "MIGRATE_STATIC: generated secret contains LF: $generated" >&2
    exit 1
  }
  if LC_ALL=C grep -Fq $'\r' "$generated"; then
    echo "MIGRATE_STATIC: generated secret contains CR: $generated" >&2
    exit 1
  fi
done

echo 'MIGRATE_STATIC: PASS'
