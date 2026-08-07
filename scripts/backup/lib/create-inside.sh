#!/usr/bin/env bash
# create-inside.sh - the backup engine, executed INSIDE the drill's postgres
# container by scripts/backup/create.sh / create.ps1 (plan Todo 33).
#
# Everything that touches PostgreSQL, hashes bytes, or encrypts an archive
# happens here, in the pinned postgres:18.4 image. That is what lets the two
# host-side twins stay thin drivers that cannot drift from each other: they
# pipe this file into `docker compose exec -T` and copy the result out. It also
# means the drill needs no pg_basebackup, openssl, or sha256sum on the host.
#
# The image has no python3, so the manifest is emitted with printf. The shape
# is fixed by deploy/backup/policy/backup-manifest.schema.json and is validated
# by scripts/backup/validate-policy.* immediately after this script returns.
#
# Inputs (environment):
#   RUN_ID      unique id for this backup set
#   NOW         UTC timestamp (YYYY-MM-DDTHH:MM:SSZ) stamped into the manifest
#   BACKUP_KEY  passphrase for db_base/db_wal encryption (policy: required)
#   OUT         directory the finished set is written to (a mounted volume)
#
# Exit 0 only when the set is complete and internally consistent.
set -eu

: "${RUN_ID:?RUN_ID required}"
: "${NOW:?NOW required}"
: "${BACKUP_KEY:?BACKUP_KEY required}"
: "${OUT:=/backup/set}"

export PGPASSWORD="${POSTGRES_PASSWORD:-drill-only-not-a-secret}"
PSQL="psql -v ON_ERROR_STOP=1 -qtAX -U lagrange -d lagrange"

WORK=/backup/work
rm -rf "$WORK" "$OUT"
mkdir -p "$WORK" "$OUT/pg/base" "$OUT/pg/wal"
mkdir -p "$OUT/files/raw" "$OUT/files/curated" "$OUT/files/artifact"

# Retention per class. Each is at or above the policy floor in
# deploy/backup/policy/backup-policy.json (7/14/30/90/180); the manifest's
# expires_at must equal completed_at + retention_days exactly, which the
# validator recomputes.
RET_BASE=14
RET_WAL=14
RET_RAW=90
RET_CURATED=180
RET_ARTIFACT=365

expiry() { date -u -d "$1 + $2 days" +%Y-%m-%dT%H:%M:%SZ; }
hash_of() { sha256sum "$1" | cut -d' ' -f1; }
size_of() { stat -c %s "$1"; }

# Encryption for the db classes. The policy marks db_base/db_wal
# encryption='required' and reference storage forbidden, so these bytes must be
# unreadable at rest. -pbkdf2 is not optional: without it openssl falls back to
# a single-pass MD5 KDF. The key never enters the set (secret-recovery.md).
encrypt() {
  openssl enc -aes-256-cbc -pbkdf2 -iter 100000 -salt \
    -in "$1" -out "$2" -pass env:BACKUP_KEY
  rm -f "$1"
}

echo "== seeding the source cluster =="
# A deterministic, self-describing dataset. `drill_provenance` carries the
# lineage a restore must reproduce; `drill_rows` carries the rows PITR must cut
# at. Nothing here resembles a secret - the policy's marker scan runs over the
# finished archives and a hit would fail the set.
$PSQL <<'SQL'
CREATE TABLE IF NOT EXISTS drill_provenance (
    key         text PRIMARY KEY,
    value       text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS drill_rows (
    id       integer PRIMARY KEY,
    phase    text NOT NULL,
    payload  text NOT NULL
);
INSERT INTO drill_provenance (key, value) VALUES
    ('dataset_version', 'krx_eod_bars@2026-01-01'),
    ('strategy_version', 'dual_momentum@2.3.1'),
    ('engine_version',  'nautilus@1.231.0')
ON CONFLICT (key) DO NOTHING;
SQL

# --- rows that exist BEFORE the base backup ---------------------------------
$PSQL -c "INSERT INTO drill_rows (id, phase, payload) SELECT g, 'pre_base', 'row-'||g FROM generate_series(1,50) g ON CONFLICT DO NOTHING;"
$PSQL -c "CHECKPOINT;"

echo "== pg_basebackup (-X none: WAL comes from the archive, never the base) =="
# -X none is deliberate. If the base carried its own WAL, a gap in the archive
# would be invisible and the drill would prove nothing about WAL shipping.
pg_basebackup -U lagrange -h 127.0.0.1 -D "$WORK/base" -Ft -z -Xnone -c fast

# --- rows that exist AT the recovery target ---------------------------------
$PSQL -c "INSERT INTO drill_rows (id, phase, payload) SELECT g, 'pre_target', 'row-'||g FROM generate_series(51,80) g ON CONFLICT DO NOTHING;"

# The recovery target is an LSN, not a timestamp. An LSN cannot drift with the
# container clock, and "a point between two WAL records" is exactly what it
# names - which is the PITR runbook's happy-path requirement (P2/P3/P4).
TARGET_LSN="$($PSQL -c "SELECT pg_current_wal_insert_lsn();")"
PRE_TARGET_COUNT="$($PSQL -c "SELECT count(*) FROM drill_rows;")"
PROVENANCE_COUNT="$($PSQL -c "SELECT count(*) FROM drill_provenance;")"
DATASET_VERSION="$($PSQL -c "SELECT value FROM drill_provenance WHERE key='dataset_version';")"

# --- rows that exist ONLY AFTER the recovery target -------------------------
$PSQL -c "INSERT INTO drill_rows (id, phase, payload) SELECT g, 'post_target', 'row-'||g FROM generate_series(81,120) g ON CONFLICT DO NOTHING;"
POST_TARGET_COUNT="$($PSQL -c "SELECT count(*) FROM drill_rows;")"

# Force every segment we care about into the archive: switch, checkpoint, then
# wait for the archiver to actually drain. Polling the archiver instead of
# sleeping keeps the run deterministic on a slow host.
$PSQL -c "SELECT pg_switch_wal();" >/dev/null
$PSQL -c "CHECKPOINT;"
$PSQL -c "SELECT pg_switch_wal();" >/dev/null
for _ in $(seq 1 60); do
  pending="$($PSQL -c "SELECT count(*) FROM pg_stat_archiver WHERE last_failed_time > last_archived_time;")"
  archived="$($PSQL -c "SELECT last_archived_wal FROM pg_stat_archiver;")"
  if [ "$pending" = "0" ] && [ -n "$archived" ]; then break; fi
  sleep 1
done
$PSQL -c "SELECT pg_switch_wal();" >/dev/null
sleep 2

wal_count="$(find /wal-archive -type f -name '0*' | wc -l)"
if [ "$wal_count" -lt 2 ]; then
  echo "FATAL: WAL archive holds $wal_count segment(s); PITR needs a contiguous archive" >&2
  exit 1
fi
echo "   archived WAL segments: $wal_count"

echo "== assembling the backup set =="
BASE_TS="$(echo "$NOW" | tr ':' '-')"
BASE_DIR="$OUT/pg/base/$BASE_TS"
WAL_DIR="$OUT/pg/wal/$BASE_TS"
mkdir -p "$BASE_DIR" "$WAL_DIR"

cp "$WORK/base/base.tar.gz" "$BASE_DIR/base.tar.gz.plain"
encrypt "$BASE_DIR/base.tar.gz.plain" "$BASE_DIR/base.tar.gz.enc"

# WAL segments, in name order. Name order IS LSN order for a single timeline,
# so a gap is detectable by inspection and by recovery failing to reach target.
for seg in $(find /wal-archive -type f -name '0*' | sort); do
  name="$(basename "$seg")"
  cp "$seg" "$WAL_DIR/$name.plain"
  encrypt "$WAL_DIR/$name.plain" "$WAL_DIR/$name.enc"
done

# --- synthetic Raw / Curated / Artifact increments --------------------------
# The file classes are content increments, matching the manifest contract's
# `.increment` shape. Deterministic content so a restore can hash-compare.
mk_increment() {
  # mk_increment <dataset> <retention> <out-subdir>
  local ds="$1" dir="$OUT/files/$1/$BASE_TS"
  mkdir -p "$dir"
  {
    echo "lagrange-station $ds increment"
    echo "backup_set_id=$RUN_ID"
    echo "dataset_version=$DATASET_VERSION"
    echo "generated_for=restore-drill"
    for i in $(seq 1 20); do echo "$ds-record-$i"; done
  } > "$dir/$ds-$BASE_TS.increment"
}
mk_increment raw
mk_increment curated
mk_increment artifact

# --- manifest ----------------------------------------------------------------
BASE_FILE="pg/base/$BASE_TS/base.tar.gz.enc"
BASE_HASH="$(hash_of "$OUT/$BASE_FILE")"
BASE_SIZE="$(size_of "$OUT/$BASE_FILE")"

wal_entries=""
first=1
for seg in $(find "$WAL_DIR" -type f -name '*.enc' | sort); do
  rel="pg/wal/$BASE_TS/$(basename "$seg")"
  [ $first -eq 1 ] || wal_entries="$wal_entries,"
  first=0
  wal_entries="$wal_entries
        { \"path\": \"$rel\", \"sha256\": \"$(hash_of "$seg")\", \"size_bytes\": $(size_of "$seg") }"
done

file_entry() {
  # file_entry <dataset>
  local ds="$1" rel="files/$1/$BASE_TS/$1-$BASE_TS.increment"
  printf '{ "path": "%s", "sha256": "%s", "size_bytes": %s }' \
    "$rel" "$(hash_of "$OUT/$rel")" "$(size_of "$OUT/$rel")"
}

file_class() {
  # file_class <class> <dataset> <retention>
  cat <<EOF
    {
      "class": "$1",
      "kind": "file",
      "dataset": "$2",
      "backup_id": "$2-$RUN_ID",
      "started_at": "$NOW",
      "completed_at": "$NOW",
      "retention_days": $3,
      "expires_at": "$(expiry "$NOW" "$3")",
      "incremental_from": null,
      "storage": { "encryption": "allowed", "reference": true, "location": "backup://lagrange/files/$2/$BASE_TS" },
      "files": [
        $(file_entry "$2")
      ]
    }
EOF
}

cat > "$OUT/backup-manifest.json" <<EOF
{
  "manifest_version": "1.0",
  "backup_set_id": "$RUN_ID",
  "created_at": "$NOW",
  "host": "lagrange-restore-drill",
  "scope": "full",
  "classes": [
    {
      "class": "db_base",
      "kind": "db",
      "backup_id": "base-$RUN_ID",
      "started_at": "$NOW",
      "completed_at": "$NOW",
      "retention_days": $RET_BASE,
      "expires_at": "$(expiry "$NOW" "$RET_BASE")",
      "storage": { "encryption": "required", "location": "backup://lagrange/db/base/$BASE_TS" },
      "files": [
        { "path": "$BASE_FILE", "sha256": "$BASE_HASH", "size_bytes": $BASE_SIZE }
      ]
    },
    {
      "class": "db_wal",
      "kind": "db",
      "backup_id": "wal-$RUN_ID",
      "started_at": "$NOW",
      "completed_at": "$NOW",
      "retention_days": $RET_WAL,
      "expires_at": "$(expiry "$NOW" "$RET_WAL")",
      "storage": { "encryption": "required", "location": "backup://lagrange/db/wal/$BASE_TS" },
      "files": [$wal_entries
      ]
    },
$(file_class file_raw raw $RET_RAW),
$(file_class file_curated curated $RET_CURATED),
$(file_class file_artifact artifact $RET_ARTIFACT)
  ],
  "restore_policy": {
    "isolated_target_required": true,
    "assertions": {
      "premember": {
        "required_classes": ["db_base", "db_wal", "file_raw", "file_curated", "file_artifact"],
        "full_restore": true
      },
      "prelive": {
        "required_classes": ["db_base", "db_wal", "file_raw", "file_curated", "file_artifact"],
        "full_restore": true,
        "startup_mode": "reconciliation_only"
      }
    }
  },
  "secret_exclusion": {
    "policy": "Secrets never enter ordinary backup archives. Recover via deploy/backup/runbooks/secret-recovery.md.",
    "marker_scan": true
  }
}
EOF

# --- sidecar -----------------------------------------------------------------
# Recorded AT BACKUP TIME, and deliberately NOT part of the backup set: the
# restore verifier compares what it recovered against these numbers, so keeping
# them outside the set means a tampered set cannot also rewrite its own
# expected answers. Runbook assertions P3/P4 and A6 read from here.
cat > "$OUT/../backup-sidecar.json" <<EOF
{
  "backup_set_id": "$RUN_ID",
  "created_at": "$NOW",
  "recovery_target_lsn": "$TARGET_LSN",
  "pre_target_row_count": $PRE_TARGET_COUNT,
  "post_target_row_count": $POST_TARGET_COUNT,
  "provenance_row_count": $PROVENANCE_COUNT,
  "dataset_version": "$DATASET_VERSION",
  "wal_segments": $wal_count
}
EOF

echo "== backup set complete =="
echo "   set:        $OUT"
echo "   target LSN: $TARGET_LSN"
echo "   rows at target: $PRE_TARGET_COUNT ; after target: $POST_TARGET_COUNT"
