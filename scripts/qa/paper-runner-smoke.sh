#!/usr/bin/env bash
# QA smoke test for the Paper runner deployment unit.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
unit="$root/deploy/systemd/paper-runner.service"
env_example="$root/deploy/systemd/paper-runner.env.example"
qa_compose="$root/deploy/qa/qa-db.compose.yml"
qa_port="${LAGRANGE_QA_DB_PORT:-55432}"
keep_db=0

while [ $# -gt 0 ]; do
  case "$1" in
    --keep-db) keep_db=1; shift ;;
    *) echo "USAGE: $0 [--keep-db]" >&2; exit 2 ;;
  esac
done

command -v docker >/dev/null 2>&1 || { echo 'docker not found on PATH' >&2; exit 2; }
command -v cargo >/dev/null 2>&1 || { echo 'cargo not found on PATH' >&2; exit 2; }
docker version --format '{{.Server.Version}}' >/dev/null 2>&1 || {
  echo 'Docker engine is unavailable or this user cannot access its socket' >&2
  exit 2
}
[ -f "$unit" ] || { echo "missing deployment unit: $unit" >&2; exit 2; }
[ -f "$env_example" ] || { echo "missing env template: $env_example" >&2; exit 2; }

for required in \
  'EnvironmentFile=/etc/lagrange/paper-runner.env' \
  'ExecStart=/opt/lagrange/bin/paper-runner' \
  'Restart=on-failure' \
  'ProtectSystem=strict' \
  'ReadOnlyPaths=/var/lib/lagrange/data/phase0'; do
  grep -Fq "$required" "$unit" || { echo "unit missing: $required" >&2; exit 2; }
done
for required in PAPER_APP_DB_PASSWORD_FILE= PAPER_WORKER_DB_PASSWORD_FILE= \
  PAPER_ADMIN_DB_PASSWORD_FILE= PAPER_AUDIT_DB_PASSWORD_FILE= \
  LAGRANGE_DATASET_ROOT= PAPER_HEALTH_STATE_PATH=; do
  grep -Fq "$required" "$env_example" || { echo "env template missing: $required" >&2; exit 2; }
done

hostpath() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
qc() { dkr compose -p lagrange-qa -f "$(hostpath "$qa_compose")" "$@"; }
cleanup() { [ "$keep_db" -eq 1 ] || qc down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT

export DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:${qa_port}/postgres"
qc up -d --wait qa-db >/dev/null
( cd "$root" && cargo test -p api-server --test paper_runner --test paper_valuation -- --nocapture )
( cd "$root" && cargo run -p api-server --bin paper-runner -- --help )
echo 'PAPER_RUNNER_SMOKE: PASS'
