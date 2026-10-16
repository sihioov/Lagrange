#!/usr/bin/env bash
# Only an archive-restored, networkless disposable PostgreSQL18 clone runs this.
# The production server/volume is never an allowed mount. Stdout is one audit.
set -euo pipefail
umask 077
[[ "$#" -eq 1 && "$1" =~ ^[a-z][a-z0-9_]{0,62}$ && "$(id -u)" -eq 999 ]] || exit 1
for name in "${!PG@}"; do unset "$name"; done
export PGDATA=/restore/18/docker PGHOST=/tmp PGPORT=5432 PGUSER=postgres PGDATABASE="$1"
export PGPASSFILE=/dev/null PGSERVICEFILE=/dev/null PGCONNECT_TIMEOUT=10 LC_ALL=C
[[ -f /verify.sql && -f "$PGDATA/PG_VERSION" && "$(cat "$PGDATA/PG_VERSION")" == 18 ]] || exit 1
for name in postmaster.pid backup_label tablespace_map standby.signal recovery.signal; do
  [[ ! -e "$PGDATA/$name" && ! -L "$PGDATA/$name" ]] || exit 1
done
# Exit73 is reserved for this native control-file/shutdown-state boundary.
# The caller binds it to the exact owned clone; no free-form stderr is retained.
control=$(pg_controldata "$PGDATA" 2>/dev/null) || exit 73
cluster_state=
while IFS= read -r line; do
  if [[ "$line" =~ ^Database\ cluster\ state:[[:space:]]+(.*)$ ]]; then
    [[ -z "$cluster_state" ]] || exit 73
    cluster_state=${BASH_REMATCH[1]}
  fi
done <<<"$control"
[[ "$cluster_state" == 'shut down' ]] || exit 73
unset control line cluster_state
printf '%s\n' 'local all all trust' > /tmp/verify-hba.conf
: > /tmp/verify-ident.conf
started=0
cleanup() {
  local result=$?
  trap - EXIT TERM INT
  if [[ "$started" -eq 1 ]]; then
    pg_ctl -D "$PGDATA" -m fast -w -t 30 stop >/dev/null 2>&1 || result=1
  fi
  exit "$result"
}
trap cleanup EXIT
trap 'exit 1' TERM INT
# The original cluster's roles, password verifiers, ownership, ACLs, databases,
# schema and data remain intact. Only runtime connection/resource/recovery
# settings change. No live network, external data directory or credential mount.
pg_ctl -D "$PGDATA" -w -t 60 -o \
  "-c data_directory=/restore/18/docker -c listen_addresses= -c unix_socket_directories=/tmp -c hba_file=/tmp/verify-hba.conf -c ident_file=/tmp/verify-ident.conf -c external_pid_file= -c ssl=off -c archive_mode=off -c archive_command= -c archive_library= -c restore_command= -c archive_cleanup_command= -c recovery_end_command= -c primary_conninfo= -c primary_slot_name= -c shared_preload_libraries= -c session_preload_libraries= -c local_preload_libraries= -c logging_collector=off -c log_destination=stderr -c autovacuum=off -c default_transaction_read_only=on -c shared_buffers=32MB -c max_connections=10 -c work_mem=4MB -c temp_file_limit=131072" \
  start >/dev/null
started=1
# Exercise logical reads of the entire target database as well as physical-byte
# equality. No dump or data row is emitted, and no schema/data mutation occurs.
timeout --signal=TERM --kill-after=5s 180s pg_dump --format=custom --dbname="$PGDATABASE" >/dev/null
psql -X -v ON_ERROR_STOP=1 --quiet --tuples-only --no-align --file=/verify.sql
