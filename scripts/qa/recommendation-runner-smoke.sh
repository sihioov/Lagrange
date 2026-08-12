#!/usr/bin/env bash
# Static deployment contract and real runner integration smoke.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose="$root/deploy/compose/compose.yml"
env_example="$root/deploy/compose/.env.example"
unit="$root/deploy/systemd/lagrange-recommendation-runner.service"

for command in cargo docker python uv; do
  command -v "$command" >/dev/null 2>&1 || { echo "$command not found on PATH" >&2; exit 2; }
done
for file in "$compose" "$unit" "$root/crates/job-queue/Dockerfile"; do
  [ -f "$file" ] || { echo "missing required deployment file: $file" >&2; exit 2; }
done
grep -Fq '**/.venv' "$root/.dockerignore" || {
  echo '.dockerignore must exclude host Python virtual environments' >&2
  exit 2
}

for required in \
  'recommendation-runner:' \
  'crates/job-queue/Dockerfile' \
  'APP_ENV: ${RECOMMENDATION_APP_ENV:?RECOMMENDATION_APP_ENV must be set}' \
  'DB_PASSWORD_FILE: /run/secrets/db_worker_password' \
  'RECOMMENDATION_HEALTH_STATE_PATH: /run/recommendation-health/health.json' \
  '/data/curated:ro' \
  '/opt/lagrange/configs/universes/kr-etf-core-v1.yaml:ro' \
  'healthcheck' \
  '"/usr/local/bin/recommendation-runner", "healthcheck"'; do
  grep -Fq "$required" "$compose" || { echo "Compose missing: $required" >&2; exit 2; }
done
grep -Fxq 'RECOMMENDATION_APP_ENV=production' "$env_example" || {
  echo 'Compose env example must select production explicitly' >&2
  exit 2
}
for required in \
  'RuntimeDirectory=lagrange-recommendation-runner' \
  'RuntimeDirectory=lagrange-recommendation-runner/tmp' \
  'RECOMMENDATION_HEALTH_STATE_PATH=/run/lagrange-recommendation-runner/health.json' \
  'recommendation-runner --repo-root /opt/lagrange' \
  'ReadOnlyPaths=/var/lib/lagrange/data/curated /etc/lagrange/universes'; do
  grep -Fq "$required" "$unit" || { echo "systemd unit missing: $required" >&2; exit 2; }
done
if grep -Eq '^ExecStartPost=' "$unit"; then
  echo 'systemd unit must not race startup health-state creation with ExecStartPost' >&2
  exit 2
fi

export DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:${LAGRANGE_QA_DB_PORT:-55432}/postgres"
qa_compose="$root/deploy/qa/qa-db.compose.yml"
docker compose -p lagrange-recommendation-qa -f "$qa_compose" up -d --wait qa-db >/dev/null
cleanup() { docker compose -p lagrange-recommendation-qa -f "$qa_compose" down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT

# This integration fixture creates the labeled synthetic 11-ETF QA data,
# migrates a disposable PostgreSQL database, seeds the pinned universe/dataset/
# entitlement/config records, and runs the real queue runner against it.
( cd "$root" && cargo test -p job-queue --test recommendation_runner real_worker_and_uv_publish_all_five_shipped_strategies -- --nocapture )
( cd "$root" && APP_ENV=qa cargo run -p job-queue --bin recommendation-runner -- --help )
echo 'RECOMMENDATION_RUNNER_SMOKE: PASS'
