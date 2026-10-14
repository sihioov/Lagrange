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
  [daily_input_admitter]="${DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE:-/run/secrets/db_daily_input_admitter_password}"
)
role_names+=(daily_input_admitter)

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
  printf '%s\n' 'BEGIN;' 'SET LOCAL lock_timeout = '\''5s'\'';' 'SET LOCAL standard_conforming_strings = on;'
  cat <<SQL
DO \$role_preflight\$
DECLARE
  admitter_oid oid;
BEGIN
  SELECT oid INTO admitter_oid
  FROM pg_roles
  WHERE rolname = 'daily_input_admitter';

  IF admitter_oid IS NOT NULL THEN
    IF NOT EXISTS (
      SELECT 1
      FROM pg_roles
      WHERE oid = admitter_oid
        AND rolcanlogin
        AND NOT rolinherit
        AND NOT rolsuper
        AND NOT rolcreatedb
        AND NOT rolcreaterole
        AND NOT rolreplication
        AND NOT rolbypassrls
    ) THEN
      RAISE EXCEPTION 'daily_input_admitter has unexpected role attributes';
    END IF;

    IF EXISTS (
      SELECT 1
      FROM pg_auth_members
      WHERE member = admitter_oid OR roleid = admitter_oid
    ) THEN
      RAISE EXCEPTION 'daily_input_admitter has unexpected role membership';
    END IF;

    IF has_database_privilege('daily_input_admitter', '$db_name', 'CREATE') THEN
      RAISE EXCEPTION 'daily_input_admitter has unexpected database CREATE privilege';
    END IF;
    IF EXISTS (
      SELECT 1
      FROM pg_namespace AS ns
      WHERE has_schema_privilege('daily_input_admitter'::name, ns.oid, 'CREATE')
    ) THEN
      RAISE EXCEPTION 'daily_input_admitter has unexpected schema CREATE privilege';
    END IF;
  END IF;

  -- Treat every membership edge as reachable, including paths usable only
  -- through SET ROLE when a member has NOINHERIT. UNION makes cycles terminate.
  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role';
  END IF;

  -- PUBLIC CREATE on the database or another schema is not part of the
  -- legacy cleanup below, so reject it before changing any role or grant.
  IF EXISTS (
    SELECT 1
    FROM pg_database AS dbrow
    CROSS JOIN LATERAL aclexplode(
      coalesce(dbrow.datacl, acldefault('d', dbrow.datdba))
    ) AS privilege
    WHERE dbrow.datname = '$db_name'
      AND privilege.grantee = 0
      AND privilege.privilege_type = 'CREATE'
  ) THEN
    RAISE EXCEPTION 'PUBLIC has unexpected database CREATE privilege';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM pg_namespace AS ns
    CROSS JOIN LATERAL aclexplode(
      coalesce(ns.nspacl, acldefault('n', ns.nspowner))
    ) AS privilege
    WHERE ns.nspname <> 'public'
      AND privilege.grantee = 0
      AND privilege.privilege_type = 'CREATE'
  ) THEN
    RAISE EXCEPTION 'PUBLIC has unexpected non-public schema CREATE privilege';
  END IF;
END
\$role_preflight\$;
SQL
  for role in "${role_names[@]}"; do
    password=$(read_secret "${role}_PASSWORD" "${role_files[$role]}")
    quoted=$(sql_quote "$password")
    if [ "$role" = daily_input_admitter ]; then
      cat <<SQL
DO \$role\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '$role') THEN
    CREATE ROLE "$role" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOREPLICATION NOBYPASSRLS PASSWORD $quoted;
  ELSE
    ALTER ROLE "$role" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOREPLICATION NOBYPASSRLS PASSWORD $quoted;
  END IF;
END
\$role\$;
SQL
    else
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
    fi
  done
  cat <<'SQL'
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT CONNECT ON DATABASE "__LAGRANGE_DB_NAME__"
  TO migration_owner, app, worker, audit_writer, research_writer, admin,
     daily_input_admitter;
GRANT USAGE ON SCHEMA public
  TO migration_owner, app, worker, audit_writer, research_writer, admin,
     daily_input_admitter;
GRANT CREATE ON SCHEMA public TO migration_owner;
DO $role_postflight$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_roles
    WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin
      AND NOT rolinherit
      AND NOT rolsuper
      AND NOT rolcreatedb
      AND NOT rolcreaterole
      AND NOT rolreplication
      AND NOT rolbypassrls
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter role attributes mismatch after bootstrap';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM pg_auth_members
    WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
       OR roleid = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has role membership after bootstrap';
  END IF;
  IF NOT has_database_privilege('daily_input_admitter', '__LAGRANGE_DB_NAME__', 'CONNECT') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks database CONNECT after bootstrap';
  END IF;
  IF NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks public schema USAGE after bootstrap';
  END IF;
  IF has_database_privilege('daily_input_admitter', '__LAGRANGE_DB_NAME__', 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has database CREATE after bootstrap';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM pg_namespace AS ns
    WHERE has_schema_privilege('daily_input_admitter'::name, ns.oid, 'CREATE')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has schema CREATE after bootstrap';
  END IF;
  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role after bootstrap';
  END IF;
  IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'migration_owner lacks public schema CREATE after bootstrap';
  END IF;
END
$role_postflight$;
COMMIT;
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
