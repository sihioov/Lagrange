#!/usr/bin/env bash
# Offline static/self-test for the provider-free combined V3 input checker. It never
# invokes Docker, KIS, a provider, a database, or a production path.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
binary="$root/data-pipelines/collectors/src/bin/kis-historical-price-v3-input-check.rs"
dockerfile="$root/data-pipelines/collectors/Dockerfile"
compose_file="$root/deploy/compose/compose.yml"
wrapper="$script_dir/kis-historical-price-v3-input-check.sh"

die() {
  echo "KIS_HISTORICAL_PRICE_V3_INPUT_CHECK_SELF_TEST: $*" >&2
  exit 1
}

[ -f "$binary" ] || die 'checker source is missing'
[ -x "$wrapper" ] || die 'checker wrapper is missing or not executable'
bash -n "$wrapper" || die 'checker wrapper has shell syntax errors'

for required in \
  'read_committed_manifest' \
  'read_batch_bytes' \
  'verify_historical_price_only_v3_action_input' \
  'verify_historical_price_only_v3_price_input' \
  'PROVIDER_KIS_DAILY_RANGE' \
  'HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID' \
  'HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID' \
  'HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256' \
  'HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256' \
  'PriceSummary' \
  'ActionSummary' \
  'session_count' \
  'bar_count' \
  'bars_sha256' \
  'cash_rows_sha256' \
  'ContentHash::from_bytes' \
  'OFlags::RDONLY' \
  'OFlags::NOFOLLOW' \
  'OFlags::CLOEXEC' \
  'BATCH_JSON_MAX_BYTES: u64 = 1024 * 1024' \
  'MANIFEST_MAX_BYTES: u64 = 64 * 1024 * 1024' \
  'split_inclusive'; do
  grep -Fq -- "$required" "$binary" || die "checker source contract missing: $required"
done

for forbidden in KIS_ACCOUNT_REF CANO ACNT_PRDT_CD DATABASE_URL LiveTransport \
  KisTokenIssuer research-worker; do
  if grep -Fq -- "$forbidden" "$binary" "$wrapper"; then
    die "checker exposes a forbidden runtime surface: $forbidden"
  fi
done

grep -Fq -- \
  'cargo build --locked --release --package collectors --bin kis-historical-price-v3-input-check' \
  "$dockerfile" || die 'checker Docker build is missing'
grep -Fq -- \
  'COPY --from=builder /build/target/release/kis-historical-price-v3-input-check /usr/local/bin/kis-historical-price-v3-input-check' \
  "$dockerfile" || die 'checker Docker copy is missing'

service_block=$(awk '
  $0 == "  research-v3-input-check:" { inside=1; print; next }
  inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
  inside { print }
' "$compose_file")
[ -n "$service_block" ] || die 'checker Compose service is missing'
for required in \
  'profiles: ["v3-input-check"]' \
  'image: lagrange-station-research-v3-input-check:' \
  'entrypoint: ["/usr/local/bin/kis-historical-price-v3-input-check"]' \
  'user: "10001:10001"' \
  'read_only: true' \
  'network_mode: none' \
  '${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw:ro' \
  '      - --raw-root' \
  '      - /data' \
  '      - --check'; do
  grep -Fq -- "$required" <<<"$service_block" ||
    die "checker Compose contract missing: $required"
done
if grep -Eiq '^[[:space:]]+(environment|secrets|depends_on|restart|healthcheck|networks):|DB_|KIS_APP|backend|curated|account|order' <<<"$service_block"; then
  die 'checker Compose service exposes a forbidden dependency or credential surface'
fi

# Plan is intentionally runnable without an installed env or Docker.  Do not
# pass a real path: this proves the plan branch returns before any file read.
plan=$(LAGRANGE_ENV_FILE=/definitely/not/installed bash "$wrapper" --plan) ||
  die 'provider-free plan unexpectedly failed'
grep -Fq 'KIS_HISTORICAL_PRICE_V3_INPUT_CHECK_PLAN mode=plan' <<<"$plan" ||
  die 'plan status is missing'
grep -Fq 'PLAN_ONLY: no installed env, Docker, Raw, KIS, provider, or network action made' <<<"$plan" ||
  die 'plan isolation statement is missing'

echo 'KIS_HISTORICAL_PRICE_V3_INPUT_CHECK_SELF_TEST: PASS (offline source, Compose, wrapper, and plan contracts)'
