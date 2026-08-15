#!/usr/bin/env bash
# Idempotent cluster-role bootstrap. Run as the PostgreSQL administrator before
# migrations. Every credential is read from a mounted file; no password is
# accepted from an environment value or command-line argument.
set -euo pipefail
umask 077

die() {
  echo "db-role-bootstrap: $*" >&2
  exit 1
}

reject_line_endings() {
  local label=$1 path=$2 line_count
  line_count=$(LC_ALL=C wc -l < "$path") || die "$label file is unreadable"
  [ "$line_count" -eq 0 ] || die "$label file must contain one line"
  if LC_ALL=C grep -Fq $'\r' "$path"; then
    die "$label file must contain one line"
  fi
}

read_secret() {
  local label=$1 path=$2 value
  [ -n "$path" ] || die "$label file path is required"
  [ -f "$path" ] || die "$label file is missing"
  [ ! -L "$path" ] || die "$label file must not be a symlink"
  [ -r "$path" ] || die "$label file is unreadable"
  reject_line_endings "$label" "$path"
  value=$(cat -- "$path") || die "$label file is unreadable"
  [ -n "$value" ] || die "$label file is empty"
  # A password must be one logical line. Silently stripping an embedded
  # newline would create a credential different from the operator's file.
  case "$value" in
    *$'\n'*|*$'\r'*) die "$label file must contain one line" ;;
  esac
  printf '%s' "$value"
}

identifier() {
  local label=$1 value=$2
  case "$value" in
    ''|*[!A-Za-z0-9_]*|[0-9]*) die "$label must be a simple PostgreSQL identifier" ;;
  esac
}

host_name() {
  local value=$1
  case "$value" in
    ''|*[!A-Za-z0-9_.-]*) die 'DB_HOST must be a DNS name or IPv4 address' ;;
  esac
}

db_host=${DB_HOST:-postgres}
db_port=${DB_PORT:-5432}
db_name=${DB_NAME:-${POSTGRES_DB:-lagrange}}
db_user=${DB_ADMIN_USER:-${POSTGRES_USER:-lagrange}}
admin_password_file=${DB_ADMIN_PASSWORD_FILE:-${POSTGRES_PASSWORD_FILE:-/run/secrets/postgres_password}}

host_name "$db_host"
identifier DB_NAME "$db_name"
identifier DB_ADMIN_USER "$db_user"
case "$db_port" in *[!0-9]*|'') die 'DB_PORT must be numeric' ;; esac

admin_password=$(read_secret DB_ADMIN_PASSWORD "$admin_password_file")

declare -a role_names=(migration_owner app worker audit_writer research_writer admin)
declare -A role_files=(
  [migration_owner]="${DB_MIGRATION_OWNER_PASSWORD_FILE:-/run/secrets/db_migration_owner_password}"
  [app]="${DB_APP_PASSWORD_FILE:-/run/secrets/db_app_password}"
  [worker]="${DB_WORKER_PASSWORD_FILE:-/run/secrets/db_worker_password}"
  [audit_writer]="${DB_AUDIT_PASSWORD_FILE:-/run/secrets/db_audit_password}"
  [research_writer]="${DB_RESEARCH_PASSWORD_FILE:-/run/secrets/db_research_password}"
  [admin]="${DB_ADMIN_ROLE_PASSWORD_FILE:-/run/secrets/db_admin_password}"
)

sql_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-role-bootstrap.XXXXXX.sql")
trap 'rm -f -- "$sql_file"' EXIT HUP INT TERM

sql_quote() {
  # PostgreSQL's standard_conforming_strings is enabled by default and the
  # only SQL metacharacter in a single-quoted literal is a single quote.
  local value=$1
  value=$(printf '%s' "$value" | sed "s/'/''/g")
  printf "'%s'" "$value"
}

{
  printf '%s\n' 'SET lock_timeout = '\''5s'\'';' 'SET standard_conforming_strings = on;'
  for role in "${role_names[@]}"; do
    password=$(read_secret "${role}_PASSWORD" "${role_files[$role]}")
    quoted=$(sql_quote "$password")
    cat <<SQL
DO \$role\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '$role') THEN
    CREATE ROLE "$role" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD $quoted;
  ELSE
    ALTER ROLE "$role" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD $quoted;
  END IF;
END
\$role\$;
SQL
  done
  cat <<'SQL'
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT CONNECT ON DATABASE "__LAGRANGE_DB_NAME__"
  TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT USAGE ON SCHEMA public
  TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
SQL
} >"$sql_file"

# The database identifier was validated above, so this substitution cannot
# introduce SQL syntax. It keeps the password values out of process arguments.
sed "s/__LAGRANGE_DB_NAME__/$db_name/g" "$sql_file" >"$sql_file.rendered"
mv -- "$sql_file.rendered" "$sql_file"

PGPASSWORD="$admin_password" PGAPPNAME=lagrange-role-bootstrap \
  psql -X --no-password -v ON_ERROR_STOP=1 \
    -h "$db_host" -p "$db_port" -U "$db_user" -d "$db_name" -f "$sql_file"
echo 'db-role-bootstrap: roles and database grants are ready'
