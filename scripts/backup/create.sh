#!/usr/bin/env bash
# create.sh - produce a policy-valid Lagrange Station backup set (plan Todo 33).
# POSIX/CI twin of scripts/backup/create.ps1.
#
# Brings up the disposable drill Compose project (deploy/backup/compose/
# drill.compose.yml), takes a real pg_basebackup with a real WAL archive,
# encrypts the DB classes, writes Raw/Curated/Artifact increments, emits a
# manifest, and REFUSES to report success unless scripts/backup/validate-policy.sh
# accepts the result. A backup that would be rejected at restore time is not a
# backup; failing here is the whole point.
#
# All PostgreSQL, hashing, and encryption work runs inside the pinned
# postgres:18.4 container via scripts/backup/lib/create-inside.sh, so this file
# and its PowerShell twin are thin drivers that cannot drift.
#
# Exit codes:
#   0  backup set created AND policy-valid
#   1  backup or validation failed (the set, if any, is left for inspection)
#   2  usage / environment error (docker unavailable, bad arguments)
#
# Usage:
#   scripts/backup/create.sh --out <dir> [--run-id <id>] [--now <UTC ts>]
#                            [--key <passphrase>] [--metrics <file.prom>]
# Twin: scripts/backup/create.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose_file="$root/deploy/backup/compose/drill.compose.yml"
inside="$root/scripts/backup/lib/create-inside.sh"
out_dir=""
run_id=""
now=""
key="lagrange-drill-key"
metrics=""

while [ $# -gt 0 ]; do
  case "$1" in
    --out) out_dir="$2"; shift 2 ;;
    --run-id) run_id="$2"; shift 2 ;;
    --now) now="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    --metrics) metrics="$2"; shift 2 ;;
    *) echo "USAGE: $0 --out <dir> [--run-id <id>] [--now <ts>] [--key <pass>] [--metrics <file>]" >&2; exit 2 ;;
  esac
done

[ -n "$out_dir" ] || { echo "USAGE: --out <dir> is required" >&2; exit 2; }
command -v docker >/dev/null 2>&1 || { echo "ENV ERROR: docker not found on PATH" >&2; exit 2; }
[ -f "$compose_file" ] || { echo "ENV ERROR: drill compose file missing: $compose_file" >&2; exit 2; }
[ -f "$inside" ] || { echo "ENV ERROR: backup engine missing: $inside" >&2; exit 2; }

# The run id doubles as the Compose project name suffix, so two drills - or a
# drill and the production stack - can never share a volume or container.
[ -n "$run_id" ] || run_id="lagrange-drill-$(date -u +%Y%m%dT%H%M%SZ)-$$"
[ -n "$now" ] || now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
project="$(printf '%s' "$run_id" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9-' '-')"

# Git Bash hands POSIX paths to a NATIVE Windows docker.exe, which mangles
# them ("/d/repo" -> "D:\d\repo"). Two rules keep this correct on both hosts:
#   1. every HOST path handed to docker goes through hostpath() first;
#   2. every docker invocation runs with MSYS path conversion OFF, so
#      CONTAINER paths (/backup/set) survive verbatim.
# Doing only one of the two breaks the other kind of path.
hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
# Both switches are set PER DOCKER CALL, never exported: exporting them also
# suppresses conversion for the Windows python3 that validate-policy.sh shells
# out to, which then cannot open a "/d/..." path.
compose_file_host="$(hostpath "$compose_file")"
dc() {
  MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' \
    docker compose -p "$project" -f "$compose_file_host" "$@"
}

started_at="$(date -u +%s)"
cleanup() {
  # A drill NEVER leaves a cluster running: "no production activation" starts
  # with not leaving stray clusters behind.
  dc down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "== drill project: $project =="
if ! dc up -d --wait source; then
  echo "BACKUP FAILED: the source cluster did not become healthy" >&2
  exit 1
fi

if ! dc exec -T \
      -e RUN_ID="$run_id" -e NOW="$now" -e BACKUP_KEY="$key" -e OUT=/backup/set \
      source bash -s < "$inside"; then
  echo "BACKUP FAILED: the backup engine reported an error" >&2
  exit 1
fi

# Copy the finished set out of the container volume onto the host.
mkdir -p "$out_dir"
rm -rf "${out_dir:?}/set" "${out_dir:?}/backup-sidecar.json"
cid="$(dc ps -q source)"
[ -n "$cid" ] || { echo "BACKUP FAILED: source container id not resolvable" >&2; exit 1; }
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker cp "$cid:/backup/set" "$(hostpath "$out_dir/set")" \
  || { echo "BACKUP FAILED: could not copy the set out" >&2; exit 1; }
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker cp "$cid:/backup/backup-sidecar.json" "$(hostpath "$out_dir/backup-sidecar.json")" \
  || { echo "BACKUP FAILED: could not copy the sidecar out" >&2; exit 1; }

# --- self-verification: the policy gate must accept what we just wrote -------
echo "== policy gate =="
gate_out="$(bash "$root/scripts/backup/validate-policy.sh" --set "$out_dir/set" --gate default 2>&1)"
gate_rc=$?
printf '%s\n' "$gate_out"

duration=$(( $(date -u +%s) - started_at ))

# --- metrics ------------------------------------------------------------------
# Prometheus textfile-collector format. Written on success AND failure: a
# backup that stopped running is exactly what the staleness alert must catch,
# and a file that only appears on success can never go stale.
if [ -n "$metrics" ]; then
  mkdir -p "$(dirname "$metrics")"
  cat > "$metrics" <<EOF
# HELP lagrange_backup_last_run_timestamp_seconds Unix time of the last backup attempt.
# TYPE lagrange_backup_last_run_timestamp_seconds gauge
lagrange_backup_last_run_timestamp_seconds $(date -u +%s)
# HELP lagrange_backup_last_success_timestamp_seconds Unix time of the last backup that passed the policy gate.
# TYPE lagrange_backup_last_success_timestamp_seconds gauge
lagrange_backup_last_success_timestamp_seconds $([ "$gate_rc" -eq 0 ] && date -u +%s || echo 0)
# HELP lagrange_backup_duration_seconds Wall-clock duration of the last backup attempt.
# TYPE lagrange_backup_duration_seconds gauge
lagrange_backup_duration_seconds $duration
# HELP lagrange_backup_exit_code Exit code of the last backup attempt (0 = policy-valid).
# TYPE lagrange_backup_exit_code gauge
lagrange_backup_exit_code $gate_rc
EOF
fi

if [ "$gate_rc" -ne 0 ]; then
  echo "BACKUP FAILED: the set was created but the policy gate rejected it (exit $gate_rc)" >&2
  exit 1
fi

echo "BACKUP OK: $run_id -> $out_dir/set (${duration}s)"
exit 0
