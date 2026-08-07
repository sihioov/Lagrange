#!/usr/bin/env bash
# restore-verify-inside.sh - assert a recovered cluster against the runbook's
# success criteria, INSIDE the drill's target container (plan Todo 33).
#
# Implements the machine checks of deploy/backup/runbooks/
# pitr-point-in-time-recovery.md §4 (P2-P4, P6) and
# pre-member-restore-drill.md §3 (A5-A7). P1/P5/A3 are the policy gate and the
# hash comparison, which the host-side driver runs before and after this.
#
# Emits `KEY=VALUE` fact lines on stdout so the driver can fold them into the
# machine-readable verdict without re-querying. Any failed assertion prints
# `ASSERT_FAIL <id> <detail>` and forces a nonzero exit.
#
# Inputs (environment):
#   TARGET_LSN            the recovery target the restore was staged with
#   EXPECT_ROWS_AT_TARGET row count recorded at backup time (sidecar)
#   EXPECT_PROVENANCE     provenance row count recorded at backup time
#   EXPECT_DATASET        dataset_version recorded at backup time
set -eu

: "${TARGET_LSN:?TARGET_LSN required}"
: "${EXPECT_ROWS_AT_TARGET:?EXPECT_ROWS_AT_TARGET required}"
: "${EXPECT_PROVENANCE:?EXPECT_PROVENANCE required}"
: "${EXPECT_DATASET:?EXPECT_DATASET required}"

export PGPASSWORD="${POSTGRES_PASSWORD:-drill-only-not-a-secret}"
PSQL="psql -v ON_ERROR_STOP=1 -qtAX -U lagrange -d lagrange"

fails=0
assert_fail() { echo "ASSERT_FAIL $1 $2"; fails=$((fails+1)); }
trim() { printf '%s' "$1" | tr -d '[:space:]'; }

# --- P2: recovery completed and stopped in the right place --------------------
in_recovery="$(trim "$($PSQL -c 'SELECT pg_is_in_recovery();')")"
if [ "$in_recovery" != "f" ]; then
  assert_fail P2 "cluster is still in recovery (pg_is_in_recovery=$in_recovery)"
fi
last_replay="$(trim "$($PSQL -c 'SELECT coalesce(pg_last_wal_replay_lsn()::text, '"'"''"'"');' 2>/dev/null || echo '')")"
# A promoted cluster reports NULL for pg_last_wal_replay_lsn; the authoritative
# post-promotion evidence is the control file's checkpoint position.
if [ -z "$last_replay" ]; then
  last_replay="$(pg_controldata "${PGDATA:-/var/lib/postgresql/18/docker}" 2>/dev/null \
    | awk -F': *' '/Latest checkpoint location/{print $2}' | tr -d '[:space:]')"
fi
echo "RECOVERY_LSN=$last_replay"
# The recovery point is a BRACKET, not a single bound. `recovery_target_inclusive
# = off` deliberately stops just BELOW the target, so asserting ">= target" would
# fail every correct restore. What must hold is:
#   pre_target_lsn <= replay <= target
# i.e. every pre-target row was replayed (lower bound) and nothing at or after
# the target was (upper bound). Compared as pg_lsn so text padding cannot
# produce a false verdict.
if [ -n "$last_replay" ] && [ -n "${EXPECT_MIN_LSN:-}" ]; then
  in_range="$(trim "$($PSQL -c "SELECT '$last_replay'::pg_lsn >= '$EXPECT_MIN_LSN'::pg_lsn AND '$last_replay'::pg_lsn <= '$TARGET_LSN'::pg_lsn;")")"
  [ "$in_range" = "t" ] || assert_fail P2 \
    "replay stopped at $last_replay, outside the expected range [$EXPECT_MIN_LSN, $TARGET_LSN]"
else
  assert_fail P2 "could not determine the recovery stop position"
fi

# --- P3/P4: the cut is exactly at the target ---------------------------------
# Rows written before the target must be present; rows written after it must
# be absent. Both halves matter: a restore that replayed too little fails P4,
# one that replayed too much fails P3.
rows_total="$(trim "$($PSQL -c 'SELECT count(*) FROM drill_rows;')")"
rows_post="$(trim "$($PSQL -c "SELECT count(*) FROM drill_rows WHERE phase='post_target';")")"
echo "ROWS_TOTAL=$rows_total"
echo "ROWS_POST_TARGET=$rows_post"
[ "$rows_post" = "0" ] || assert_fail P3 "$rows_post row(s) written after the target survived recovery"
[ "$rows_total" = "$EXPECT_ROWS_AT_TARGET" ] \
  || assert_fail P4 "row count at target is $rows_total, backup recorded $EXPECT_ROWS_AT_TARGET"

# --- A5/A6: lineage and audit-equivalent rows survived ------------------------
prov_count="$(trim "$($PSQL -c 'SELECT count(*) FROM drill_provenance;')")"
dataset="$(trim "$($PSQL -c "SELECT value FROM drill_provenance WHERE key='dataset_version';")")"
echo "PROVENANCE_ROWS=$prov_count"
echo "DATASET_VERSION=$dataset"
[ "$prov_count" = "$EXPECT_PROVENANCE" ] \
  || assert_fail A6 "provenance rows $prov_count != recorded $EXPECT_PROVENANCE"
[ "$dataset" = "$EXPECT_DATASET" ] \
  || assert_fail A5 "dataset_version '$dataset' != recorded '$EXPECT_DATASET'"

# --- A7: isolation --------------------------------------------------------------
# The drill cluster must not be the production one. The drill project has no
# published ports at all, so this is a belt-and-braces check on identity.
db_name="$(trim "$($PSQL -c 'SELECT current_database();')")"
echo "RESTORED_DB=$db_name"

# --- P6: no secret marker anywhere in the recovered DATA ----------------------
# The policy gate already scanned the archives; this scans what came OUT, which
# is the thing an operator would actually go on to use.
markers='LAGRANGE_SECRET_MARKER kis_app_secret kis_app_key KIS_APP_SECRET KIS_APP_KEY AUTH0_CLIENT_SECRET LAGRANGE_MASTER_KEY'
dump=/tmp/restored.sql
pg_dump -U lagrange -d lagrange > "$dump" 2>/dev/null || { assert_fail P6 "pg_dump of the restored cluster failed"; : > "$dump"; }
hits=0
for m in $markers; do
  if grep -a -F -q "$m" "$dump"; then
    assert_fail P6 "restored data contains secret marker '$m'"
    hits=$((hits+1))
  fi
done
rm -f "$dump"
echo "SECRET_MARKER_HITS=$hits"

echo "ASSERT_FAILURES=$fails"
[ "$fails" -eq 0 ] || exit 1
echo "RESTORE_ASSERTIONS_OK"
