#!/usr/bin/env bash
# Static contract check for the Owner Equity V2 runtime boundary.
# No Docker, database, provider, credential, or production path is invoked.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
ops=$root/scripts/ops
compose_file=$root/deploy/compose/compose.yml
runner_dockerfile=$root/crates/job-queue/Dockerfile.owner-equity-v2-runner
collector_dockerfile=$root/data-pipelines/collectors/Dockerfile
runner_source=$root/crates/job-queue/src/bin/owner-equity-v2-runner.rs
runtime_source=$root/crates/job-queue/src/owner_equity_v2/runtime.rs
runner_logic=$root/crates/job-queue/src/owner_equity_v2/runner.rs
manifest_lib=$ops/lib/release-image-manifest.sh
compose_release=$ops/compose-release.sh
build_script=$ops/build-production-images.sh
build_static=$ops/build-production-images-static-check.sh
provision=$ops/provision-linux.sh
verify=$ops/owner-equity-v2-verify.sh

die() { echo "owner-equity-v2-runtime-static: $*" >&2; exit 1; }

for path in "$compose_file" "$runner_dockerfile" "$collector_dockerfile" \
  "$runner_source" "$runtime_source" "$runner_logic" "$manifest_lib" \
  "$compose_release" "$build_script" "$build_static" "$provision" "$verify"; do
  [ -f "$path" ] || die "required file missing: $path"
done
for script in "$compose_release" "$build_script" "$build_static" "$provision" "$verify"; do
  bash -n "$script" || die "shell syntax failure: $script"
done

grep -Fq 'FROM rust:1.97.1-alpine@sha256:3c38' "$runner_dockerfile" ||
  die 'runner builder base is not the reviewed digest-pinned image'
grep -Fq 'FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d' \
  "$runner_dockerfile" || die 'runner runtime base is not the reviewed digest-pinned image'
grep -Fq 'ARG LAGRANGE_CODE_COMMIT' "$runner_dockerfile" ||
  die 'runner Dockerfile must accept the exact release commit'
grep -Fq 'cargo build --locked --release --package job-queue --bin owner-equity-v2-runner' \
  "$runner_dockerfile" || die 'runner binary build is missing'
grep -Fq "printf '%s' \"\$LAGRANGE_CODE_COMMIT\" | grep -Eq '^[0-9a-f]{40}$'" \
  "$runner_dockerfile" || die 'runner Dockerfile must validate the commit shape'
grep -Fq 'USER 10001:10001' "$runner_dockerfile" || die 'runner must be nonroot UID/GID 10001'
grep -Fq 'STOPSIGNAL SIGTERM' "$runner_dockerfile" || die 'runner graceful stop signal is missing'
grep -Fq 'HEALTHCHECK' "$runner_dockerfile" || die 'runner healthcheck is missing'
grep -Fq 'CMD ["/usr/local/bin/owner-equity-v2-runner", "healthcheck"]' "$runner_dockerfile" ||
  die 'runner healthcheck must use the production binary'
grep -Fq 'apk add --no-cache ca-certificates libgcc libstdc++ libpq libssl3' \
  "$runner_dockerfile" || die 'runner runtime dependencies are not explicit'
runner_runtime=$(sed -n '/^FROM alpine:3\.21@/,$p' "$runner_dockerfile")
if grep -Eq '^COPY (Cargo|crates|data-pipelines|apps|configs|data|tests)' <<<"$runner_runtime"; then
  die 'runner runtime stage must not copy source or compilers'
fi

for binary in owner-equity-v2-check owner-equity-v2-materialize; do
  grep -Fq "cargo build --locked --release --package collectors --bin $binary" \
    "$collector_dockerfile" || die "collector $binary build is missing"
  grep -Fq "COPY --from=builder /build/target/release/$binary /usr/local/bin/$binary" \
    "$collector_dockerfile" || die "collector $binary runtime copy is missing"
done

service_block() {
  local name=$1
  awk -v target="  $name:" '
    $0 == target { inside=1; print; next }
    inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    inside { print }
  ' "$compose_file"
}

v2_block=$(service_block owner-equity-v2-runner)
[ -n "$v2_block" ] || die 'V2 queue service block is missing'
for line in \
  'profiles: ["owner-equity-v2"]' \
  'dockerfile: crates/job-queue/Dockerfile.owner-equity-v2-runner' \
  'init: true' \
  'restart: unless-stopped' \
  'stop_grace_period: 16m' \
  'read_only: true' \
  '  - ALL' \
  'no-new-privileges:true' \
  'OWNER_EQUITY_V2_RAW_ROOT: /data/raw' \
  'OWNER_EQUITY_V2_ARTIFACT_ROOT: /data/owner-equity-v2-artifacts' \
  'OWNER_EQUITY_V2_MAX_ACTIVE: "100"' \
  'OWNER_EQUITY_V2_INITIAL_GET_CEILING: "7"' \
  'OWNER_EQUITY_V2_INCREMENTAL_GET_CEILING: "2"' \
  'OWNER_EQUITY_V2_TOTAL_BACKFILL_GET_CEILING: "700"' \
  'OWNER_EQUITY_V2_CONCURRENCY: "1"' \
  'OWNER_EQUITY_V2_ESTIMATED_BYTES_PER_GET: "1048576"' \
  'OWNER_EQUITY_V2_HEARTBEAT_SECS: "10"' \
  'OWNER_EQUITY_V2_LEASE_SECS: "60"' \
  'OWNER_EQUITY_V2_RECOVERY_SECS: "30"' \
  'OWNER_EQUITY_V2_BACKOFF_SECS: "30"' \
  'OWNER_EQUITY_V2_WORK_TIMEOUT_SECS: "900"' \
  'KIS_APP_KEY_FILE: /run/secrets/kis_app_key' \
  'KIS_APP_SECRET_FILE: /run/secrets/kis_app_secret' \
  'source: owner_beta_db_worker_password' \
  'source: research_kis_app_key' \
  'source: research_kis_app_secret' \
  '/raw:/data/raw:rw' \
  '/owner-equity-v2-artifacts:/data/owner-equity-v2-artifacts:rw' \
  'test: ["CMD", "/usr/local/bin/owner-equity-v2-runner", "healthcheck"]' \
  '- owner-equity-v2-egress'; do
  grep -Fq -- "$line" <<<"$v2_block" || die "V2 service boundary missing: $line"
done
grep -Fq 'DB_PASSWORD_FILE: /run/secrets/db_worker_password' <<<"$v2_block" ||
  die 'V2 worker DB secret must use the worker password file'
if grep -Eiq 'CANO|ACNT_PRDT_CD|KIS_ACCOUNT_REF|DATABASE_URL|DB_PASSWORD=' <<<"$v2_block"; then
  die 'V2 queue service contains a forbidden account/direct-secret channel'
fi
grep -Fq 'owner-equity-v2-egress:' "$compose_file" || die 'V2 egress network is missing'
grep -A2 -F 'owner-equity-v2-egress:' "$compose_file" | grep -Fq 'internal: false' ||
  die 'V2 egress network must permit only the reviewed worker network path'

for service in api-server web; do
  block=$(service_block "$service")
  if grep -Eiq 'KIS_APP_KEY|KIS_APP_SECRET|OWNER_EQUITY_V2_RAW_ROOT|/data/owner-equity-v2-artifacts' <<<"$block"; then
    die "$service receives a V2 credential or Raw/artifact root"
  fi
done
api_block=$(service_block api-server)
grep -Fq 'OWNER_EQUITY_V2_ENTITLEMENT_REFERENCE: ${OWNER_EQUITY_V2_ENTITLEMENT_REFERENCE:-}' <<<"$api_block" ||
  die 'API typed V2 entitlement reference pin is missing'
grep -Fq 'OWNER_EQUITY_V2_ENTITLEMENT_SHA256: ${OWNER_EQUITY_V2_ENTITLEMENT_SHA256:-}' <<<"$api_block" ||
  die 'API typed V2 entitlement hash pin is missing'

grep -A20 '^RELEASE_IMAGE_SERVICES=(' "$manifest_lib" | grep -Fq 'owner-equity-v2-runner' ||
  die 'V2 service is missing from the exact release manifest'
grep -Fq 'local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")' "$build_script" ||
  die 'sequential build script must consume the canonical V2-inclusive service list'
grep -Fq 'owner-equity-v2-runner' "$build_static" || die 'build static map does not mention V2'
grep -Fq 'run_owner_equity_v2_release_gate' "$compose_release" ||
  die 'V2 release approval gate is missing'
grep -Fq 'OWNER_EQUITY_V2_ROLLOUT_CONFIRM' "$compose_release" ||
  die 'V2 rollout confirmation is not process-local and explicit'
grep -Fq 'release_worker_services+=(owner-equity-v2-runner)' "$compose_release" ||
  die 'V2 worker is not selected only in owner-only mode'
grep -Fq 'OWNER_EQUITY_V2_RUNTIME_MODE' "$ops/lib/dotenv.sh" ||
  die 'V2 rollout mode is not protected by the dotenv security key list'
grep -Fq 'OWNER_EQUITY_V2_RUNTIME_MODE' "$ops/validate-production-config.sh" ||
  die 'V2 rollout mode validation is missing'
grep -Fq 'owner-equity-v2-artifacts' "$provision" ||
  die 'dedicated V2 artifact-root provisioning is missing'

for token in \
  'Daemon' '--once' 'Healthcheck' 'recover_owner_equity_claims(&queue).await' \
  'shutdown_signal().await' 'work.await' 'Quota::new(1, 1)' \
  'OWNER_EQUITY_V2_MAX_ACTIVE' 'OWNER_EQUITY_V2_CONCURRENCY'; do
  grep -Fq -- "$token" "$runner_source" || die "runner contract missing: $token"
done
runner_production=$(awk '/^#\[cfg\(test\)\]/{exit} {print}' "$runner_source")
grep -Fq 'response.body' <<<"$runner_production" && die 'runner must not log response bodies'
for token in \
  'self.maximum_active_instruments > 100' \
  'self.total_initial_backfill_get_ceiling > 700' \
  'self.concurrency != 1' \
  'self.estimated_bytes_per_get == 0' \
  'estimated_total_initial_disk_bytes' \
  'initial_get_ceiling_per_job: 7' \
  'incremental_get_ceiling_per_job: 2'; do
  grep -Fq -- "$token" "$runtime_source" || die "runtime preflight contract missing: $token"
done
for token in 'heartbeat_interval >= lease' 'recover_owner_equity_claims' \
  'OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry' \
  "error_code = 'attempts_exhausted'"; do
  grep -Fq -- "$token" "$runner_logic" || die "recovery contract missing: $token"
done

for token in '--network none' '--read-only' '--cap-drop ALL' \
  '--security-opt no-new-privileges:true' '--user 10001:10001' \
  '--pull=never' 'owner-equity-v2-check' \
  'destination=/data/raw,readonly' 'destination=/data/artifacts,readonly' \
  'docker image inspect'; do
  grep -Fq -- "$token" "$verify" || die "verifier boundary missing: $token"
done
if grep -Eiq 'curl|wget|KIS_APP_KEY=|KIS_APP_SECRET=|DB_PASSWORD=|DATABASE_URL|docker compose|docker build|docker start|docker restart' "$verify"; then
  die 'verifier wrapper contains a forbidden provider/secret/lifecycle channel'
fi

echo 'OWNER_EQUITY_V2_RUNTIME_STATIC: PASS'
