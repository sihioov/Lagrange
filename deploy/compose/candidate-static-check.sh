#!/usr/bin/env bash
# Fail-closed static contract for the common stock-candidate production path.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
compose="$root/deploy/compose/compose.yml"
dockerfile="$root/crates/job-queue/Dockerfile"
runner="$root/crates/job-queue/src/bin/candidate-runner.rs"
candidate_input="$root/crates/job-queue/src/candidate/input.rs"
candidate_compute="$root/crates/job-queue/src/candidate/runner.rs"
candidate_pipeline="$root/data-pipelines/collectors/src/candidate_pipeline.rs"
candidate_sink="$root/data-pipelines/collectors/src/candidate_sink.rs"
source_up="$root/migrations/0042_candidate_source_contracts.up.sql"
analysis_up="$root/migrations/0043_candidate_analysis_surfaces.up.sql"
pipeline_up="$root/migrations/0044_candidate_pipeline.up.sql"
multi_universe_up="$root/migrations/0045_candidate_multi_universe.up.sql"
entitlement="$root/configs/data-rights/krx.entitlement.example.json"
env_file="$root/deploy/compose/.env.example"

die() {
  echo "candidate-static-check: $*" >&2
  exit 1
}

for file in "$compose" "$dockerfile" "$runner" "$candidate_input" "$candidate_compute" \
  "$candidate_pipeline" "$candidate_sink" \
  "$source_up" "$analysis_up" "$pipeline_up" "$multi_universe_up" "$entitlement"; do
  [ -f "$file" ] || die "missing $file"
done

grep -Fq -- '--bin candidate-runner' "$dockerfile" || die 'image does not build candidate-runner'
grep -Fq '/usr/local/bin/candidate-runner' "$dockerfile" || die 'image does not install candidate-runner'
grep -Fq '  candidate-runner:' "$compose" || die 'Compose service is absent'
grep -Fq 'entrypoint: ["/usr/local/bin/candidate-runner"]' "$compose" || die 'wrong candidate entrypoint'
grep -Fq 'CANDIDATE_DATA_ROOT: /data' "$compose" \
  || die 'candidate trusted root must match catalog storage_path=/data'
grep -Fq '${LAGRANGE_DATA_DIR:-../data}/curated:/data/curated:ro' "$compose" \
  || die 'candidate runner must receive curated bytes read-only below its trusted root'
if grep -Fq 'CANDIDATE_PRICE_CURATED_VERSION' "$compose" "$env_file" "$runner"; then
  die 'curated generation must come from the exact published price pin, not process configuration'
fi
grep -Fq 'test: ["CMD", "/usr/local/bin/candidate-runner", "healthcheck"]' "$compose" \
  || die 'candidate liveness probe is absent'
grep -Fq 'stop_grace_period: 5m' "$compose" || die 'candidate drain budget is absent'
grep -Fq 'candidate main loop made no recent progress' "$runner" \
  || die 'liveness does not detect a wedged candidate main loop'
grep -Fq 'candidate feed is not current for the latest closed KRX session' "$runner" \
  || die 'readiness does not require the latest closed-session feed'
grep -Fq 'daily_flow.trade_date = $2' "$candidate_input" \
  || die 'daily investor-flow freshness is not exact-date gated'
grep -Fq 'status.trade_date = $2' "$candidate_input" \
  || die 'daily market-status freshness is not exact-date gated'
grep -Fq 'flags.data_stale |= !has_as_of_price' "$candidate_compute" \
  || die 'daily price freshness is not exact-date gated'
grep -Fq 'pub fn prepare_candidate_batch' "$candidate_pipeline" \
  || die 'immutable Raw-to-candidate preparation seam is absent'
grep -Fq 'sink.publish_batch(&publications)' "$candidate_pipeline" \
  || die 'prepared candidate sources are not published as one batch'
grep -Fq 'SET TRANSACTION ISOLATION LEVEL SERIALIZABLE' "$candidate_sink" \
  || die 'candidate source publication is not serializable'
grep -Fq 'source: candidate_db_worker_password' "$compose" || die 'candidate worker secret is absent'
grep -Fq 'candidate-runner/db_worker_password' "$compose" || die 'candidate secret path is not isolated'

for dataset in krx_eod_bars krx_market_status krx_investor_flows krx_fundamentals \
  krx_kospi200_membership krx_sector_classification; do
  grep -Fq "'$dataset'" "$pipeline_up" || die "pipeline does not pin $dataset"
  grep -Fq "\"$dataset\"" "$entitlement" || die "entitlement example omits $dataset"
done
grep -Fq "'krx_kosdaq150_membership'" "$multi_universe_up" \
  || die 'multi-universe migration does not register the KOSDAQ150 membership dataset'
grep -Fq '"krx_kosdaq150_membership"' "$entitlement" \
  || die 'entitlement example omits krx_kosdaq150_membership'
grep -Fq 'candidate_universe_registry' "$multi_universe_up" \
  || die 'multi-universe registry is absent'
grep -Fq 'pub enum CandidateUniverseKey' "$root/crates/market-data/src/candidate.rs" \
  || die 'canonical candidate universe type is absent'
grep -Fq 'candidate_source_validate_dataset_pin' "$source_up" || die 'source pin trigger absent'
grep -Fq 'candidate_price_publications' "$source_up" \
  || die 'price readiness and curated generation publication contract absent'
grep -Fq 'published candidate feed must contain exactly five items' "$analysis_up" \
  || die 'Top-5 deferred publication guard absent'
[ "$(grep -Fc 'entitlement became inactive before publication' "$pipeline_up")" -eq 6 ] \
  || die 'publication-time exact entitlement rechecks are incomplete'

commit=0123456789abcdef0123456789abcdef01234567
LAGRANGE_CODE_COMMIT=$commit RANGE_RAW_BATCH_ID=compose-config-disabled \
  docker compose --env-file "$env_file" -f "$compose" config --quiet

echo 'CANDIDATE_STATIC: PASS'
