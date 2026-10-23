#!/usr/bin/env bash
# Only the pinned networkless disposable PostgreSQL container runs this file.
# Stdout contains exactly the fixed audit JSON; no rows, commands or credentials.
set -euo pipefail
umask 077
[ "$#" -eq 0 ] && [ "$(id -u)" -eq 999 ] || exit 1
[ -f /verify.dump ] && [ -f /verify.sql ] && [ -d /restore ] || exit 1
for name in "${!PG@}"; do unset "$name"; done
export PGHOST=/tmp PGPORT=5432 PGUSER=postgres PGDATABASE=postgres
export PGPASSFILE=/dev/null PGSERVICEFILE=/dev/null PGCONNECT_TIMEOUT=10
started=0
cleanup() {
  local result=$?
  trap - EXIT TERM INT
  if [ "$started" -eq 1 ]; then
    pg_ctl -D /restore/pgdata -m fast -w -t 30 stop >/dev/null 2>&1 || result=1
  fi
  exit "$result"
}
trap cleanup EXIT
trap 'exit 1' TERM INT
initdb -D /restore/pgdata --auth-local=trust --auth-host=reject >/dev/null
pg_ctl -D /restore/pgdata -w -t 60 -o \
  "-c listen_addresses= -c unix_socket_directories=/tmp -c shared_buffers=32MB -c max_connections=10 -c work_mem=4MB -c temp_file_limit=131072" \
  start >/dev/null
started=1
psql -X -v ON_ERROR_STOP=1 --quiet >/dev/null <<'SQL'
-- Only this disposable cluster receives the source administrator identity
-- needed by restored ownership/ACL statements; no login or admin rights.
CREATE ROLE lagrange NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE migration_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE worker NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE audit_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE research_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE admin NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE ROLE daily_input_admitter NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE DATABASE verify;
SQL
# Ownership, policies and ACLs are restored as well as data. Unknown roles or
# missing extensions fail; no --no-owner/--no-privileges bypass is used here.
pg_restore --exit-on-error --dbname=verify /verify.dump >/dev/null
psql -X -v ON_ERROR_STOP=1 --quiet --tuples-only --no-align --dbname=verify \
  --file=/verify.sql
