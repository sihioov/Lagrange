#!/usr/bin/env bash
# restore-prepare-inside.sh - stage a point-in-time recovery INSIDE the drill's
# target container, BEFORE PostgreSQL is ever started (plan Todo 33).
#
# This is what makes the drill a real recovery rather than a data copy: the
# target's PGDATA is populated from the encrypted base tar, the WAL archive is
# decrypted alongside it, and a `recovery.signal` plus a `restore_command` tell
# PostgreSQL to replay the archive up to an explicit LSN on first boot. The
# container's own entrypoint then finds an initialised PGDATA, skips initdb,
# and starts the cluster in recovery.
#
# Run via `docker compose run --rm --entrypoint bash target -s`, so no
# postgres process is alive while PGDATA is being written.
#
# Inputs (environment):
#   BACKUP_KEY  passphrase the db classes were encrypted with
#   TARGET_LSN  recovery target (pg_lsn text, e.g. "0/3000060")
#   SET         backup-set root inside the container (default /backup/set)
#
# Exit codes: 0 staged; 1 staging failed (decryption, missing class, bad tar).
set -eu

: "${BACKUP_KEY:?BACKUP_KEY required}"
: "${TARGET_LSN:?TARGET_LSN required}"
: "${SET:=/backup/set}"

PGDATA_DIR="${PGDATA:-/var/lib/postgresql/18/docker}"
WAL_RESTORE=/wal-archive

echo "== staging restore =="
echo "   set:     $SET"
echo "   pgdata:  $PGDATA_DIR"
echo "   target:  $TARGET_LSN"

[ -d "$SET" ] || { echo "FATAL: backup set not found at $SET" >&2; exit 1; }

decrypt() {
  # A wrong key fails HERE, before anything is written into PGDATA. That is
  # deliberate: an unreadable archive must abort the restore, never produce a
  # half-populated cluster that a later assertion has to catch.
  if ! openssl enc -d -aes-256-cbc -pbkdf2 -iter 100000 \
        -in "$1" -out "$2" -pass env:BACKUP_KEY 2>/dev/null; then
    echo "FATAL: could not decrypt '$1' (wrong key or corrupt archive)" >&2
    rm -f "$2"
    return 1
  fi
}

# --- WAL archive -------------------------------------------------------------
mkdir -p "$WAL_RESTORE"
rm -f "$WAL_RESTORE"/* 2>/dev/null || true
wal_n=0
for enc in $(find "$SET/pg/wal" -type f -name '*.enc' 2>/dev/null | sort); do
  name="$(basename "$enc" .enc)"
  decrypt "$enc" "$WAL_RESTORE/$name" || exit 1
  wal_n=$((wal_n+1))
done
[ "$wal_n" -gt 0 ] || { echo "FATAL: no WAL segments in the set; PITR is impossible" >&2; exit 1; }
echo "   decrypted WAL segments: $wal_n"

# --- base backup -------------------------------------------------------------
base_enc="$(find "$SET/pg/base" -type f -name '*.enc' | sort | head -n1)"
[ -n "$base_enc" ] || { echo "FATAL: no db_base archive in the set" >&2; exit 1; }
decrypt "$base_enc" /tmp/base.tar.gz || exit 1

# PGDATA must be EMPTY before the restore - the runbook records pre-restore
# emptiness as assertion A7. Refuse rather than merge into an existing cluster.
mkdir -p "$PGDATA_DIR"
if [ -f "$PGDATA_DIR/PG_VERSION" ]; then
  echo "FATAL: target PGDATA already holds a cluster; refusing to restore over it" >&2
  exit 1
fi
rm -rf "${PGDATA_DIR:?}"/* 2>/dev/null || true
tar -xzf /tmp/base.tar.gz -C "$PGDATA_DIR"
rm -f /tmp/base.tar.gz
chmod 0700 "$PGDATA_DIR"

[ -f "$PGDATA_DIR/PG_VERSION" ] || { echo "FATAL: base tar did not yield a cluster" >&2; exit 1; }
echo "   restored cluster version: $(cat "$PGDATA_DIR/PG_VERSION")"

# --- recovery configuration ---------------------------------------------------
# recovery_target_lsn, not recovery_target_time: an LSN names an exact point
# between two WAL records and cannot drift with the container clock, so the
# "rows at the target are present, rows after it are absent" assertion is
# deterministic (PITR runbook P2/P3/P4).
cat >> "$PGDATA_DIR/postgresql.auto.conf" <<EOF

# --- Todo 33 restore drill (staged by restore-prepare-inside.sh) ---
restore_command = 'cp $WAL_RESTORE/%f %p'
recovery_target_lsn = '$TARGET_LSN'
recovery_target_action = 'promote'
# OFF, not the default ON. With ON, PostgreSQL stops just AFTER the first
# record at or beyond the target, and a bulk INSERT is a single WAL record --
# so an entire batch written after the target comes back. "Restore to a point"
# means excluding everything at or after it, which is what OFF does.
# (No backticks in this heredoc: it is unquoted so that TARGET_LSN and
# WAL_RESTORE interpolate, which means backticks would be run as commands.)
recovery_target_inclusive = off
archive_mode = off
EOF
touch "$PGDATA_DIR/recovery.signal"

echo "== staged; PostgreSQL will enter recovery on first boot =="
