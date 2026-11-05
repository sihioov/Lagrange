#!/bin/sh
# Runs only named SQL contract tests in the private PostgreSQL fixture.
set -eu
umask 077

pg=/usr/lib/postgresql/18/bin
data=/var/lib/postgresql/r5-paper
pg_pid=
fixture_status=0
pg_status=125
pg_reaped=false

run_bounded_test() (
    ulimit -f 4096 || exit 70
    exec "$@"
)

write_receipt() {
    receipt=$1
    value=$2
    printf '%s\n' "$value" >"/output/$receipt.tmp"
    chmod 0644 "/output/$receipt.tmp"
    mv "/output/$receipt.tmp" "/output/$receipt"
}

stop_postgres() {
    [ -n "$pg_pid" ] || return 0
    kill -INT "$pg_pid" 2>/dev/null || true
    attempt=0
    while [ "$attempt" -lt 35 ]; do
        state=$(ps -o stat= -p "$pg_pid" 2>/dev/null || true)
        case "$state" in
            ""|Z*) break ;;
        esac
        sleep 1
        attempt=$((attempt + 1))
    done
    state=$(ps -o stat= -p "$pg_pid" 2>/dev/null || true)
    case "$state" in
        ""|Z*) ;;
        *)
            kill -TERM "$pg_pid" 2>/dev/null || true
            sleep 2
            state=$(ps -o stat= -p "$pg_pid" 2>/dev/null || true)
            case "$state" in
                ""|Z*) ;;
                *) kill -KILL "$pg_pid" 2>/dev/null || true ;;
            esac
            ;;
    esac
    pg_status=0
    wait "$pg_pid" || pg_status=$?
    pg_pid=
    pg_reaped=true
    write_receipt pg-exit-code "$pg_status"
    write_receipt pg-reaped true
}

finish() {
    status=$?
    trap - EXIT INT TERM
    set +e
    stop_postgres
    stop_status=$?
    if [ "$status" -eq 0 ] && [ "$fixture_status" -ne 0 ]; then status=$fixture_status; fi
    if [ "$status" -eq 0 ] && [ "$stop_status" -ne 0 ]; then status=125; fi
    write_receipt fixture-exit-code "$status"
    write_receipt pg-exit-code "$pg_status"
    write_receipt pg-reaped "$pg_reaped"
    chmod 0644 /output/pg-init.log /output/postgres.log 2>/dev/null || true
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

case "${R5_03_ISOLATED_NONCE:-}" in
    *[!0-9a-f]*|"") exit 64 ;;
esac
[ "${#R5_03_ISOLATED_NONCE}" -eq 32 ] || exit 64
[ -x /input/tests ] && [ -r /input/guest.sh ] && [ -r /input/libfaketime.so.1 ] || exit 64
[ -r /input/selected-tests.txt ] || exit 64

mkdir -p "$data" /tmp/r5-03-socket /tmp/r5-03-clock
touch /output/pg-init.log /output/postgres.log
"$pg/initdb" -D "$data" --username=postgres --auth-local=trust --auth-host=trust \
    --no-locale --encoding=UTF8 > /output/pg-init.log 2>&1
printf '+0\n' > /tmp/r5-03-clock/offset
chmod 0600 /tmp/r5-03-clock/offset
clock_owner=$(stat -c '%u %h' /tmp/r5-03-clock/offset)
[ "$clock_owner" = '999 1' ] || exit 65

export R5_03_FIXTURE_CLOCK_FILE=/tmp/r5-03-clock/offset
export LD_PRELOAD=/input/libfaketime.so.1
export FAKETIME_TIMESTAMP_FILE=/tmp/r5-03-clock/offset
export FAKETIME_DONT_FAKE_MONOTONIC=1 FAKETIME_NO_CACHE=1
export FAKE_UTIME=0 NO_FAKE_STAT=1 TZ=UTC
"$pg/postgres" -D "$data" -h 127.0.0.1 -p 55438 -k /tmp/r5-03-socket \
    -c shared_buffers=32MB -c max_connections=40 -c log_statement=none \
    -c log_min_error_statement=panic -c log_error_verbosity=terse \
    > /output/postgres.log 2>&1 &
pg_pid=$!

ready=0
attempt=0
while [ "$attempt" -lt 30 ]; do
    if "$pg/pg_isready" -h 127.0.0.1 -p 55438 -U postgres >/dev/null 2>&1; then ready=1; break; fi
    if ! kill -0 "$pg_pid" 2>/dev/null; then break; fi
    sleep 1
    attempt=$((attempt + 1))
done
[ "$ready" -eq 1 ] || { fixture_status=1; exit 1; }

export DATABASE_URL=postgres://postgres:lagrange@127.0.0.1:55438/postgres
export LAGRANGE_QA_DB_PORT=55438 RUST_BACKTRACE=0
test_index=0
while IFS= read -r test_name; do
    [ -n "$test_name" ] || continue
    case "$test_name" in
        paper_0067::*)
            ;;
        *) fixture_status=66; continue ;;
    esac
    test_index=$((test_index + 1))
    test_log=$(printf '/output/test-%02d.log' "$test_index")
    test_receipt=$(printf '/output/test-%02d-exit-code' "$test_index")
    if ! kill -0 "$pg_pid" 2>/dev/null; then
        test_status=125
        printf '%s\n' 'postgres-child-not-running' >"$test_log"
    elif run_bounded_test /input/tests --exact "$test_name" --include-ignored --test-threads=1 --nocapture >"$test_log" 2>&1; then
        test_status=0
    else
        test_status=$?
        [ "$fixture_status" -ne 0 ] || fixture_status=$test_status
    fi
    printf '%s\n' "$test_status" >"$test_receipt"
    chmod 0644 "$test_log" "$test_receipt"
done < /input/selected-tests.txt
[ "$test_index" -gt 0 ] || { fixture_status=67; exit 1; }
[ "$fixture_status" -eq 0 ] || exit "$fixture_status"
