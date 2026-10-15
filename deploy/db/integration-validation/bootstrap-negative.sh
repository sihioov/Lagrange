#!/usr/bin/env bash
# Actual bootstrap failure contracts. Only the disposable, pre-migration CI
# container calls this file. It never prints SQL output, credentials or hashes.
set -euo pipefail
umask 077

case_name=setup
case_phase=setup
failure_emitted=0
fail() {
  printf 'BOOTSTRAP_CONTRACT_CASE:%s:FAIL\n' "$case_name" >&2
  printf 'BOOTSTRAP_CONTRACT_FAILURE:%s:%s\n' "$case_name" "$case_phase" >&2
  failure_emitted=1
  exit 1
}
[ "$#" -eq 1 ] && [ "$1" = --disposable-ci ] || fail
[ "$(id -u)" -ne 0 ] || fail
input=/tmp/lagrange-bootstrap-contract-inputs
bootstrap=/validation/bootstrap-roles.sh
[ -f "$bootstrap" ] && [ -d "$input" ] || fail
scratch=$(mktemp -d /tmp/lagrange-bootstrap-contract.XXXXXX)
holder_pid=''
cleanup() {
  local status=$?
  trap - EXIT
  if [ "$status" -ne 0 ] && [ "$failure_emitted" -eq 0 ]; then
    printf 'BOOTSTRAP_CONTRACT_CASE:%s:FAIL\n' "$case_name" >&2
    printf 'BOOTSTRAP_CONTRACT_FAILURE:%s:%s\n' "$case_name" "$case_phase" >&2
  fi
  if [ -n "$holder_pid" ]; then
    kill "$holder_pid" 2>/dev/null || true
    wait "$holder_pid" 2>/dev/null || true
  fi
  rm -rf -- "$scratch" "$input"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

export PGHOST=127.0.0.1 PGPORT=5432 PGDATABASE=postgres PGUSER=postgres
export PGCONNECT_TIMEOUT=5 PGOPTIONS='-c statement_timeout=20000'
PGPASSWORD=$(cat "$input/postgres_password")
export PGPASSWORD
export DB_HOST=127.0.0.1 DB_PORT=5432 DB_NAME=postgres DB_ADMIN_USER=postgres
export TMPDIR="$scratch/sql"
mkdir -p "$TMPDIR" "$scratch/attempt"

secret_names=(postgres_password db_migration_owner_password db_app_password
  db_worker_password db_audit_password db_research_password db_admin_password
  db_daily_input_admitter_password)
secret_labels=(DB_ADMIN_PASSWORD migration_owner_PASSWORD app_PASSWORD
  worker_PASSWORD audit_writer_PASSWORD research_writer_PASSWORD admin_PASSWORD
  daily_input_admitter_PASSWORD)
for name in "${secret_names[@]}"; do
  cp -- "$input/$name" "$scratch/attempt/$name"
  chmod 0600 -- "$scratch/attempt/$name"
done
export DB_ADMIN_PASSWORD_FILE="$scratch/attempt/postgres_password"
export DB_MIGRATION_OWNER_PASSWORD_FILE="$scratch/attempt/db_migration_owner_password"
export DB_APP_PASSWORD_FILE="$scratch/attempt/db_app_password"
export DB_WORKER_PASSWORD_FILE="$scratch/attempt/db_worker_password"
export DB_AUDIT_PASSWORD_FILE="$scratch/attempt/db_audit_password"
export DB_RESEARCH_PASSWORD_FILE="$scratch/attempt/db_research_password"
export DB_ADMIN_ROLE_PASSWORD_FILE="$scratch/attempt/db_admin_password"
export DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE="$scratch/attempt/db_daily_input_admitter_password"

sql() {
  # The SQL and its diagnostics stay in this disposable private directory. Even
  # a failed snapshot comparison cannot disclose a password verifier to CI.
  psql -X --no-password -qAt -v ON_ERROR_STOP=1 "$@" \
    >"$scratch/control.raw" 2>&1 || fail
}
pass() { printf 'BOOTSTRAP_CONTRACT_CASE:%s:PASS\n' "$case_name"; }
run_bootstrap() {
  case_phase=bootstrap_execution
  if timeout 25s /bin/bash "$bootstrap" >"$scratch/bootstrap.raw" 2>&1; then
    bootstrap_status=0
  else
    bootstrap_status=$?
  fi
  # A missing cleanup trap in the real script is independently observable.
  [ -z "$(find "$TMPDIR" -mindepth 1 -print -quit)" ] || fail
}
capture() {
  case_phase=catalog_capture
  sql <<'SQL'
TRUNCATE bootstrap_contract.expected;
INSERT INTO bootstrap_contract.expected VALUES (bootstrap_contract.state());
SQL
}
unchanged() {
  case_phase=catalog_unchanged
  sql <<'SQL'
DO $assert$
BEGIN
  IF (SELECT count(*) FROM bootstrap_contract.expected) <> 1
     OR bootstrap_contract.state() IS DISTINCT FROM
        (SELECT value FROM bootstrap_contract.expected) THEN
    RAISE EXCEPTION 'bootstrap changed the captured catalog state';
  END IF;
END
$assert$;
SQL
}
expect_sql_failure() {
  local reason=$1
  run_bootstrap
  case_phase=expected_sql_failure
  [ "$bootstrap_status" = 3 ] || fail
  [ "$(grep -c 'ERROR:' "$scratch/bootstrap.raw")" = 1 ] || fail
  grep -Fq "ERROR:  $reason" "$scratch/bootstrap.raw" || fail
  unchanged
}
catalog_case() {
  local name=$1 seed=$2 restore=$3 reason=$4
  case_name=$name
  case_phase=seed_fixture
  sql -c "$seed"
  capture
  expect_sql_failure "$reason"
  case_phase=restore_fixture
  sql -c "$restore"
  pass
}
positive() {
  case_name=$1
  run_bootstrap
  case_phase=positive_control
  [ "$bootstrap_status" = 0 ] || fail
  grep -Fxq 'db-role-bootstrap: roles and database grants are ready' "$scratch/bootstrap.raw" || fail
  sql <<'SQL'
DO $assert$
BEGIN
  IF NOT EXISTS (
    SELECT FROM pg_roles WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin AND NOT rolinherit AND NOT rolsuper AND NOT rolcreatedb
      AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls
  ) OR EXISTS (
    SELECT FROM pg_auth_members WHERE member = 'daily_input_admitter'::regrole
      OR roleid = 'daily_input_admitter'::regrole
  ) OR NOT has_database_privilege('daily_input_admitter', 'postgres', 'CONNECT')
    OR NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE')
    OR has_database_privilege('daily_input_admitter', 'postgres', 'CREATE')
    OR EXISTS (SELECT FROM pg_namespace
      WHERE has_schema_privilege('daily_input_admitter'::name, oid, 'CREATE')) THEN
    RAISE EXCEPTION 'positive bootstrap control failed';
  END IF;
END
$assert$;
SQL
  pass
}

# Refuse a migrated database or any colliding fixture object. The caller has
# just bootstrapped these seven roles on its own fresh, disposable cluster.
sql <<'SQL'
DO $guard$
BEGIN
  IF current_user <> 'postgres' OR current_database() <> 'postgres'
    OR to_regnamespace('bootstrap_contract') IS NOT NULL
    OR to_regnamespace('bootstrap_contract_other') IS NOT NULL
    OR EXISTS (SELECT FROM pg_roles WHERE rolname = 'bootstrap_contract_bridge')
    OR EXISTS (SELECT FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f')) THEN
    RAISE EXCEPTION 'fixture requires a fresh disposable pre-migration cluster';
  END IF;
END
$guard$;
CREATE SCHEMA bootstrap_contract AUTHORIZATION postgres;
REVOKE ALL ON SCHEMA bootstrap_contract FROM PUBLIC;
CREATE ROLE bootstrap_contract_bridge NOLOGIN NOINHERIT;
CREATE SCHEMA bootstrap_contract_other AUTHORIZATION postgres;
CREATE FUNCTION bootstrap_contract.state() RETURNS jsonb
LANGUAGE sql SET search_path = pg_catalog AS $state$
SELECT jsonb_build_object(
  'roles', (SELECT jsonb_agg(to_jsonb(r) ORDER BY r.oid) FROM pg_authid r
    WHERE rolname IN ('migration_owner', 'app', 'worker', 'audit_writer',
      'research_writer', 'admin', 'daily_input_admitter', 'postgres', 'bootstrap_contract_bridge')),
  'members', (SELECT coalesce(jsonb_agg(to_jsonb(m) ORDER BY m.roleid, m.member, m.grantor), '[]')
    FROM pg_auth_members m),
  'database', (SELECT jsonb_build_object('owner', d.datdba, 'acl',
    (SELECT jsonb_agg(to_jsonb(a) ORDER BY a.grantor, a.grantee, a.privilege_type)
      FROM aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a))
    FROM pg_database d WHERE d.datname = current_database()),
  'schemas', (SELECT jsonb_agg(jsonb_build_object('name', n.nspname, 'owner', n.nspowner,
    'acl', (SELECT jsonb_agg(to_jsonb(a) ORDER BY a.grantor, a.grantee, a.privilege_type)
      FROM aclexplode(coalesce(n.nspacl, acldefault('n', n.nspowner))) a)) ORDER BY n.nspname)
    FROM pg_namespace n WHERE n.nspname IN ('public', 'bootstrap_contract', 'bootstrap_contract_other'))
);
$state$;
REVOKE ALL ON FUNCTION bootstrap_contract.state() FROM PUBLIC;
CREATE TABLE bootstrap_contract.expected(value jsonb NOT NULL);
SQL
positive positive_initial
positive positive_rerun

for attributes in 'NOLOGIN LOGIN' 'INHERIT NOINHERIT' 'SUPERUSER NOSUPERUSER' \
  'CREATEDB NOCREATEDB' 'CREATEROLE NOCREATEROLE' 'REPLICATION NOREPLICATION' \
  'BYPASSRLS NOBYPASSRLS'; do
  read -r invalid valid <<<"$attributes"
  catalog_case "attribute_${invalid,,}" \
    "ALTER ROLE daily_input_admitter $invalid" \
    "ALTER ROLE daily_input_admitter $valid" \
    'daily_input_admitter has unexpected role attributes'
done

catalog_case membership_incoming \
  'GRANT daily_input_admitter TO bootstrap_contract_bridge' \
  'REVOKE daily_input_admitter FROM bootstrap_contract_bridge' \
  'daily_input_admitter has unexpected role membership'
catalog_case membership_outgoing \
  'GRANT bootstrap_contract_bridge TO daily_input_admitter' \
  'REVOKE bootstrap_contract_bridge FROM daily_input_admitter' \
  'daily_input_admitter has unexpected role membership'
for actor in app worker; do
  for protected in migration_owner daily_input_admitter; do
    reason='app or worker reaches a protected migration role'
    if [ "$protected" = daily_input_admitter ]; then
      reason='daily_input_admitter has unexpected role membership'
    fi
    catalog_case "path_${actor}_${protected}_direct" \
      "GRANT $protected TO $actor" "REVOKE $protected FROM $actor" "$reason"
    catalog_case "path_${actor}_${protected}_transitive" \
      "GRANT $protected TO bootstrap_contract_bridge WITH INHERIT FALSE;
       GRANT bootstrap_contract_bridge TO $actor WITH SET TRUE;
       DO \$path\$ BEGIN
         IF NOT EXISTS (SELECT FROM pg_auth_members
           WHERE roleid = '$protected'::regrole
             AND member = 'bootstrap_contract_bridge'::regrole
             AND NOT inherit_option AND set_option)
           OR NOT EXISTS (SELECT FROM pg_auth_members
             WHERE roleid = 'bootstrap_contract_bridge'::regrole
               AND member = '$actor'::regrole AND set_option) THEN
           RAISE EXCEPTION 'fixture did not create the non-inheriting SET ROLE path';
         END IF;
       END \$path\$" \
      "REVOKE bootstrap_contract_bridge FROM $actor; REVOKE $protected FROM bootstrap_contract_bridge" "$reason"
  done
done

catalog_case create_database \
  'GRANT CREATE ON DATABASE postgres TO daily_input_admitter' \
  'REVOKE CREATE ON DATABASE postgres FROM daily_input_admitter' \
  'daily_input_admitter has unexpected database CREATE privilege'
catalog_case create_public_schema \
  'GRANT CREATE ON SCHEMA public TO daily_input_admitter' \
  'REVOKE CREATE ON SCHEMA public FROM daily_input_admitter' \
  'daily_input_admitter has unexpected schema CREATE privilege'
catalog_case create_other_schema \
  'GRANT CREATE ON SCHEMA bootstrap_contract_other TO daily_input_admitter' \
  'REVOKE CREATE ON SCHEMA bootstrap_contract_other FROM daily_input_admitter' \
  'daily_input_admitter has unexpected schema CREATE privilege'
catalog_case create_owned_schema \
  'ALTER SCHEMA bootstrap_contract_other OWNER TO daily_input_admitter' \
  'ALTER SCHEMA bootstrap_contract_other OWNER TO postgres' \
  'daily_input_admitter has unexpected schema CREATE privilege'
# With the admitter absent, these hit the explicit PUBLIC guards, rather than
# the earlier effective-privilege check for an existing admitter.
sql -c 'DROP OWNED BY daily_input_admitter; DROP ROLE daily_input_admitter'
catalog_case create_public_database \
  'GRANT CREATE ON DATABASE postgres TO PUBLIC' \
  'REVOKE CREATE ON DATABASE postgres FROM PUBLIC' \
  'PUBLIC has unexpected database CREATE privilege'
catalog_case create_public_other_schema \
  'GRANT CREATE ON SCHEMA bootstrap_contract_other TO PUBLIC' \
  'REVOKE CREATE ON SCHEMA bootstrap_contract_other FROM PUBLIC' \
  'PUBLIC has unexpected non-public schema CREATE privilege'
# Recreate the valid control before exercising file rejection.
run_bootstrap
[ "$bootstrap_status" = 0 ] || fail

for index in "${!secret_names[@]}"; do
  name=${secret_names[$index]}
  label=${secret_labels[$index]}
  target="$scratch/attempt/$name"
  for fault in missing empty directory symlink lf cr unreadable; do
    case_name="secret_${name}_${fault}"
    case_phase=secret_fixture
    rm -f -- "$target"
    case "$fault" in
      missing) reason='file is missing' ;;
      empty) : >"$target"; reason='file is empty' ;;
      directory) mkdir "$target"; reason='file is missing' ;;
      symlink) ln -s "$input/$name" "$target"; reason='file must not be a symlink' ;;
      lf) cat "$input/$name" >"$target"; printf '\n' >>"$target"; reason='file must contain one line' ;;
      cr) cat "$input/$name" >"$target"; printf '\r' >>"$target"; reason='file must contain one line' ;;
      unreadable)
        cp "$input/$name" "$target"
        chmod 0000 "$target"
        [ -f "$target" ] && [ ! -r "$target" ] || fail
        reason='file is unreadable'
        ;;
    esac
    capture
    run_bootstrap
    case_phase=expected_secret_failure
    [ "$bootstrap_status" = 1 ] || fail
    grep -Fxq "db-role-bootstrap: $label $reason" "$scratch/bootstrap.raw" || fail
    [ "$(wc -l <"$scratch/bootstrap.raw")" -eq 1 ] || fail
    unchanged
    if [ "$fault" = directory ]; then rmdir "$target"; else rm -f -- "$target"; fi
    cp "$input/$name" "$target"
    chmod 0600 "$target"
    pass
  done
done

# Force a real failure after all seven role statements AND an ACL mutation.
# The absent admitter allows the legacy PUBLIC public-schema CREATE cleanup;
# the separate transaction then blocks the database GRANT that follows it.
case_name=late_lock_timeout_rollback
case_phase=seed_fixture
sql <<'SQL'
DROP OWNED BY daily_input_admitter;
DROP ROLE daily_input_admitter;
GRANT CREATE ON SCHEMA public TO PUBLIC;
SQL
capture
case_phase=lock_readiness
coproc BOOTSTRAP_LOCK {
  exec psql -X --no-password -qAt -v ON_ERROR_STOP=1 2>"$scratch/holder.raw"
}
holder_pid=$BOOTSTRAP_LOCK_PID
exec {holder_input}>&"${BOOTSTRAP_LOCK[1]}"
exec {holder_output}<&"${BOOTSTRAP_LOCK[0]}"
printf '%s\n' 'BEGIN;' "SET LOCAL idle_in_transaction_session_timeout = '60s';" \
  'REVOKE CONNECT ON DATABASE postgres FROM PUBLIC;' '\echo LOCK_READY' >&"$holder_input"
IFS= read -r -t 10 -u "$holder_output" acknowledgment || fail
[ "$acknowledgment" = LOCK_READY ] || fail
expect_sql_failure 'canceling statement due to lock timeout'
case_phase=prior_statement_completion
[ "$(grep -c '^DO$' "$scratch/bootstrap.raw")" -eq 8 ] || fail
[ "$(grep -c '^REVOKE$' "$scratch/bootstrap.raw")" -eq 1 ] || fail
case_phase=lock_release
printf '%s\n' 'ROLLBACK;' '\echo LOCK_RELEASED' '\q' >&"$holder_input"
IFS= read -r -t 10 -u "$holder_output" acknowledgment || fail
[ "$acknowledgment" = LOCK_RELEASED ] || fail
wait "$holder_pid" || fail
holder_pid=''
exec {holder_input}>&-
exec {holder_output}<&-
unchanged
pass

positive positive_after_failures
case_name=cleanup
case_phase=fixture_cleanup
sql <<'SQL'
DROP SCHEMA bootstrap_contract CASCADE;
DROP SCHEMA bootstrap_contract_other;
DROP ROLE bootstrap_contract_bridge;
SQL
printf 'BOOTSTRAP_CONTRACT:PASS\n'
