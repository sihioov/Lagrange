#!/usr/bin/env bash
# Private Paper clone only. The host owns the container and every mounted original.
set -euo pipefail
[[ $(id -u) == 999 && $# == 2 ]]
admin=$1
database=$2
[[ $admin =~ ^[A-Za-z_][A-Za-z0-9_]{0,62}$ && $database =~ ^[A-Za-z_][A-Za-z0-9_]{0,62}$ ]]
export PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin
export PGHOST=/run/postgresql PGPORT=5432 PGUSER="$admin" PGDATABASE=postgres
export PGPASSFILE=/dev/null PGSERVICEFILE=/dev/null PGCONNECT_TIMEOUT=10
export PGOPTIONS='-c statement_timeout=900000 -c lock_timeout=5000'
data=/var/lib/postgresql/verify
[[ -f /verify/roles.sql && -f /verify/postgres.dump && -d $data && -d /run/postgresql ]]
[[ ! -e $data/PG_VERSION ]]
initdb -D "$data" --username="$admin" --encoding=UTF8 --auth-local=trust --auth-host=reject >/dev/null
running=false
cleanup() {
    local code=$?
    trap - EXIT INT TERM
    if [[ $running == true ]]; then
        pg_ctl -D "$data" -m immediate -w -t 10 stop >/dev/null 2>&1 || exit 1
    fi
    exit "$code"
}
trap cleanup EXIT INT TERM
pg_ctl -D "$data" -l /dev/null -w -t 60 \
    -o "-c listen_addresses='' -c unix_socket_directories='/run/postgresql' -c max_connections=10 -c shared_buffers=32MB -c work_mem=4MB -c temp_file_limit=131072" start >/dev/null
running=true
psql -X -v ON_ERROR_STOP=1 --file=/verify/roles.sql >/dev/null
pg_restore --exit-on-error --dbname="$database" /verify/postgres.dump >/dev/null
pg_ctl -D "$data" -m fast -w -t 10 stop >/dev/null
running=false
trap - EXIT INT TERM
# The final postmaster replaces this exact owned PID. Host checks its ELF and
# actual listening socket; no ready file is a database or completion receipt.
exec postgres -D "$data" -c listen_addresses='' -c unix_socket_directories=/run/postgresql \
    -c max_connections=10 -c shared_buffers=32MB -c work_mem=4MB -c temp_file_limit=131072
