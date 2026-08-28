#!/usr/bin/env bash
# Static boundary check for the profile-gated sealed owner-beta worker.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose="$root/deploy/compose/compose.yml"
dockerfile="$root/crates/job-queue/Dockerfile.owner-beta-runner"
binary="$root/crates/job-queue/src/bin/owner-beta-runner.rs"

die() { echo "owner-beta-static-check: $*" >&2; exit 1; }
service_block() {
  awk '$0 == "  owner-beta-runner:" { active=1; print; next }
       active && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
       active { print }' "$compose"
}

[ -f "$compose" ] || die "missing Compose file"
[ -f "$dockerfile" ] || die "missing dedicated Dockerfile"
[ -f "$binary" ] || die "missing dedicated binary"
block="$(service_block)"
[ -n "$block" ] || die "missing owner-beta-runner service"

for required in 'profiles: ["owner-beta"]' 'entrypoint: ["/usr/local/bin/owner-beta-runner"]' \
  'command: ["--artifact-root", "/data/owner-beta-artifacts"]' 'read_only: true' \
  'stop_grace_period: 6m' 'no-new-privileges:true' 'PRICE_BETA_HEALTH_STATE_PATH:' 'owner_beta_db_worker_password' \
  'owner-beta-db' 'historical-price-beta-root:/data/owner-beta-artifacts:ro'; do
  grep -Fq "$required" <<<"$block" || die "service missing $required"
done
grep -Fq 'cap_drop:' <<<"$block" && grep -Fq '      - ALL' <<<"$block" || die "service must drop all capabilities"
[ "$(grep -c 'source: owner_beta_db_worker_password' <<<"$block")" -eq 1 ] || die "service must have exactly one database secret"
[ "$(grep -c 'historical-price-beta-root:/data/owner-beta-artifacts:ro' <<<"$block")" -eq 1 ] || die "service must have one fixed read-only artifact leaf"
[ "$(grep -c '^      - owner-beta-db$' <<<"$block")" -eq 1 ] || die "service may use only owner-beta-db"
if grep -Eqi 'kis|curated|provider|order|DATABASE_URL|DB_PASSWORD:' <<<"$block"; then die "service contains forbidden boundary token"; fi

grep -Fq 'owner-beta-db:' "$compose" || die "missing owner-beta-db network"
awk '/^  postgres:/{active=1; next} active && /^  [^[:space:]][^:]*:/{exit} active{print}' "$compose" \
  | grep -Fxq '      - owner-beta-db' || die "postgres is not attached to owner-beta-db"
for required in 'COPY configs/evidence/kis-range-canonical-approved-manifests.json' \
  'COPY configs/evidence/kis-historical-price-only-beta-approved-artifacts.json' \
  'COPY configs/evidence/kis-historical-price-only-v3-approved-artifacts.json' \
  '--bin owner-beta-runner' 'FROM alpine@sha256:' 'ca-certificates libgcc libstdc++ libpq libssl3' \
  'USER 10001:10001' 'org.opencontainers.image.revision'; do
  grep -Fq -- "$required" "$dockerfile" || die "Dockerfile missing $required"
done
if grep -Eqi 'python|uv|nt/|kis_app|kis_app_secret' "$dockerfile"; then die "Dockerfile contains forbidden runtime input"; fi
for forbidden in 'queue.sweep' 'generic settle' 'provider' 'order' 'KIS' 'Curated' 'Paper'; do
  if grep -Fq "$forbidden" "$binary"; then die "binary contains forbidden token: $forbidden"; fi
done
grep -Fq 'recover_owner_beta_claims(&queue)' "$binary" || die "binary must call dedicated recovery"
grep -Fq 'recovery_failure_exits()' "$binary" || die "binary must fail closed after recovery failure"
grep -Fq 'run_once(&pool, &queue' "$binary" || die "binary must call sealed run_once"

echo 'OWNER_BETA_STATIC: PASS'
