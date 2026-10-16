#!/usr/bin/env bash
# Fixed image entrypoint. Statements come only from the verified release caller.
# Secret bytes stay in process memory; stderr cannot echo a SQL/input value.
set -euo pipefail
fail() { printf '%s\n' 'DAILY_CONTROL_DATABASE: FAILED' >&2; exit 1; }
[ "$#" -eq 0 ] || fail
[ "$(id -u)" = 999 ] || fail
[ "${DB_USER-}" = migration_owner ] || fail
[ "${DB_HOST-}" = postgres ] || fail
[ "${DB_PORT-}" = 5432 ] || fail
[[ "${DB_NAME-}" =~ ^[A-Za-z_][A-Za-z0-9_]{0,62}$ ]] || fail
[ "${DB_PASSWORD_FILE-}" = /run/secrets/db_migration_owner_password ] || fail
[ -z "${DATABASE_URL-}" ] && [ -z "${DATABASE_URL_FILE-}" ] || fail
[ -f "$DB_PASSWORD_FILE" ] && [ ! -L "$DB_PASSWORD_FILE" ] || fail
[ "$(stat -c '%u:%g:%a:%h' -- "$DB_PASSWORD_FILE")" = 999:999:400:1 ] || fail
bytes=$(stat -c '%s' -- "$DB_PASSWORD_FILE") || fail
[ "$bytes" -gt 0 ] && [ "$bytes" -le 8192 ] || fail
[ "$(wc -l <"$DB_PASSWORD_FILE")" -eq 0 ] || fail
if LC_ALL=C grep -Fq $'\r' "$DB_PASSWORD_FILE"; then fail; fi
LC_ALL=C tr -d '\000' <"$DB_PASSWORD_FILE" | cmp -s - "$DB_PASSWORD_FILE" || fail
password=$(cat -- "$DB_PASSWORD_FILE") || fail
# Remove inherited service/hostaddr/options/password-file settings before using
# fixed component-mode arguments. The image supplies the trusted psql binary.
for name in "${!PG@}"; do unset "$name"; done
export PGPASSWORD="$password" PGCONNECT_TIMEOUT=10 PGPASSFILE=/dev/null
export PGSERVICEFILE=/dev/null PGAPPNAME=lagrange_daily_control
unset password
command -v psql >/dev/null 2>&1 || fail
exec psql --no-psqlrc --no-password --quiet --tuples-only --no-align \
    --host=postgres --port=5432 --username=migration_owner --dbname="$DB_NAME" \
    -v ON_ERROR_STOP=1 2>/dev/null
