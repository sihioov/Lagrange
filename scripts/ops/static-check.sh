#!/usr/bin/env bash
# Static contract check for production operator workflows; no Docker/root/API.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops="$root/scripts/ops"
die() { echo "OPS_STATIC: $*" >&2; exit 1; }

[ -f "$ops/lib/dotenv.sh" ] || die 'shared dotenv helper is missing'
bash -n "$ops/lib/dotenv.sh" || die 'shared dotenv helper has shell syntax errors'
grep -Fq 'uses Compose interpolation, quote, escape' "$ops/lib/dotenv.sh" \
  || die 'dotenv parser must reject Compose interpretation syntax'

for script in provision-linux.sh provision-db-secrets.sh provision-auth0-secret.sh \
  provision-crypto-secrets.sh provision-kis-credentials.sh validate-production-config.sh compose-release.sh \
  backfill-production.sh install-kis-backfill-timer.sh backfill-resume-self-test.sh \
  post-backfill-health.sh backfill-review-report.sh \
  backfill-review-report-self-test.sh self-test.sh renew-tailscale-tls.sh \
  install-tailscale-tls-renewal.sh tailscale-tls-self-test.sh \
  build-production-images.sh build-production-images-static-check.sh \
  build-production-images-self-test.sh deploy-production-release.sh \
  run-production-backup.sh install-production-backup.sh \
  production-ops-static-check.sh production-ops-self-test.sh \
  kis-range-raw-backfill.sh kis-action-range-raw-backfill.sh \
  kis-action-range-raw-with-worker-pause.sh \
  kis-daily-production.sh kis-daily-production-self-test.sh \
  kis-daily-calendar-refresh.sh install-kis-daily.sh \
  fsc-krx-listed-self-test.sh kind-daily.sh install-kind-daily.sh \
  kind-daily-self-test.sh kis-historical-price-beta-artifact.sh \
  kis-historical-price-beta-artifact-self-test.sh; do
  path="$ops/$script"
  [ -x "$path" ] || die "$script must be executable"
  [ ! -L "$path" ] || die "$script must not be a symlink"
  bash -n "$path" || die "$script has shell syntax errors"
done

range_raw="$ops/kis-range-raw-backfill.sh"
action_range_raw="$ops/kis-action-range-raw-backfill.sh"
action_range_guard="$ops/kis-action-range-raw-with-worker-pause.sh"
grep -Fq 'KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS' "$range_raw" \
  || die 'Stage5 range raw execute confirmation missing'
grep -Fq 'research-range-raw' "$range_raw" \
  || die 'Stage5 must use the dedicated range-raw Compose service'
grep -Fq 'compose_service=research-range-raw' "$range_raw" \
  || die 'Stage5 capture service selection missing'
grep -Fq 'compose_service=research-range-raw-recovery' "$range_raw" \
  || die 'Stage5 recovery service selection missing'
grep -Fq 'compose_profile=range-raw-recovery' "$range_raw" \
  || die 'Stage5 recovery Compose profile missing'
grep -Fq 'validation_scope=range-raw-recovery' "$range_raw" \
  || die 'Stage5 recovery validator scope missing'
grep -Fq 'run --rm --no-deps "$compose_service"' "$range_raw" \
  || die 'Stage5 must run one selected isolated no-deps container'
grep -Fq 'research-worker daemon is running' "$range_raw" \
  || die 'Stage5 daemon overlap guard missing'
grep -Fq 'read_reconciled_manifest' "$root/data-pipelines/collectors/src/worker.rs" \
  || die 'Stage5 must reconcile/reuse committed Raw before refetch'
grep -Fq 'LAGRANGE_CODE_COMMIT' "$range_raw" \
  || die 'Stage5 code commit provenance guard missing'
worker_dockerfile="$root/data-pipelines/collectors/Dockerfile"
artifact_ops="$ops/kis-historical-price-beta-artifact.sh"
artifact_self_test="$ops/kis-historical-price-beta-artifact-self-test.sh"
manifest_lib="$ops/lib/release-image-manifest.sh"
if sed -n '/RELEASE_IMAGE_SERVICES=(/,/)/p' "$manifest_lib" |
   grep -Fq 'research-historical-price-beta-artifact'; then
  die 'historical artifact one-shot must not join the ten-image serving manifest'
fi
if grep -Fq 'research-historical-price-beta-artifact:' "$root/deploy/compose/compose.yml"; then
  die 'historical artifact must have no alternate Compose execution path'
fi
grep -Fq 'kis-historical-price-beta-artifact' "$worker_dockerfile" \
  || die 'historical artifact binary is missing from the research-worker image'
grep -Fq 'cargo build --locked --release --package collectors --bin kis-historical-price-beta-artifact' \
  "$worker_dockerfile" || die 'historical artifact binary build is missing'
grep -Fq 'COPY --from=builder /build/target/release/kis-historical-price-beta-artifact' \
  "$worker_dockerfile" || die 'historical artifact binary copy is missing'
grep -Fq 'cargo build --locked --release --package collectors --bin kis-historical-price-beta-approval-check' \
  "$worker_dockerfile" || die 'historical approval-check binary build is missing'
grep -Fq 'COPY --from=builder /build/target/release/kis-historical-price-beta-approval-check /usr/local/bin/kis-historical-price-beta-approval-check' \
  "$worker_dockerfile" || die 'historical approval-check binary copy is missing'
grep -Fq 'historical-price-beta-root' "$root/scripts/ops/provision-linux.sh" \
  || die 'dedicated historical artifact provisioning is missing'
grep -Fq 'worker_uid" "$worker_gid" 750 historical-price-beta-root' \
  "$root/scripts/ops/provision-linux.sh" || die 'dedicated historical artifact ownership fence is missing'
grep -Fq 'worker_uid" "$worker_gid" 750 backtest-artifacts' \
  "$root/scripts/ops/provision-linux.sh" || die 'backtest artifact ownership fence is missing'
grep -Fq 'worker_uid" "$worker_gid" 750 backtest-runs' \
  "$root/scripts/ops/provision-linux.sh" || die 'backtest run ownership fence is missing'
grep -Fq 'docker image inspect' "$artifact_ops" || die 'historical artifact image gate is missing'
grep -Fq 'org.opencontainers.image.revision' "$artifact_ops" \
  || die 'historical artifact revision gate is missing'
grep -Fq -- '--pull=never' "$artifact_ops" || die 'historical artifact must refuse image pulls'
grep -Fq -- '--network none' "$artifact_ops" || die 'historical artifact direct run network fence is missing'
grep -Fq -- '--cap-drop ALL' "$artifact_ops" || die 'historical artifact direct run cap fence is missing'
grep -Fq -- '--security-opt no-new-privileges:true' "$artifact_ops" \
  || die 'historical artifact direct run privilege fence is missing'
grep -Fq -- '--read-only' "$artifact_ops" || die 'historical artifact direct run rootfs fence is missing'
grep -Fq -- '--user 10001:10001' "$artifact_ops" || die 'historical artifact direct run UID/GID fence is missing'
grep -Fq 'destination=/data/raw,readonly' "$artifact_ops" \
  || die 'historical artifact materialize Raw read-only mount is missing'
grep -Fq 'destination=/artifact-root,readonly' "$artifact_ops" \
  || die 'historical artifact check read-only mount is missing'
grep -Fq -- '--approval-check' "$artifact_ops" || die 'historical approval-check mode is missing'
grep -Fq 'entrypoint=/usr/local/bin/kis-historical-price-beta-approval-check' "$artifact_ops" \
  || die 'historical approval-check entrypoint is missing'
if grep -Fq -- '--approval-registry' "$artifact_ops"; then
  die 'historical approval-check must use only its compile-time embedded registry'
fi
grep -Fq 'not_current_release' "$artifact_ops" || die 'installed current release gate is missing'
grep -Fq 'release_image_manifest_load' "$artifact_ops" || die 'installed V2 manifest gate is missing'
if grep -Eq 'docker[[:space:]]+compose|docker[[:space:]]+(build|up|start|restart)' "$artifact_ops"; then
  die 'historical artifact wrapper must not build or start Compose services'
fi
grep -Fq 'HISTORICAL_PRICE_BETA_ARTIFACT_SELF_TEST: PASS' "$artifact_self_test" \
  || die 'historical artifact fake-Docker self-test is missing'
grep -Fq 'ARG LAGRANGE_CODE_COMMIT' "$worker_dockerfile" \
  || die 'research-worker image must accept an exact commit build argument'
grep -Fq 'org.opencontainers.image.revision' "$worker_dockerfile" \
  || die 'research-worker image OCI revision provenance is missing'
grep -Fq "grep -Eq '^[0-9a-f]{40}$'" "$worker_dockerfile" \
  || die 'research-worker image must validate the exact lowercase commit shape'
grep -Fq 'write_state RUNNING' "$range_raw" \
  || die 'Stage5 must persist its source BatchId before the container/network step'
grep -Fq 'RANGE_RAW_BATCH_ID="$stored_batch_id" compose run' "$range_raw" \
  || die 'Stage5 must pass the persisted source BatchId into the one-shot'
grep -Fq 'stored_batch_id=$(python3' "$range_raw" \
  || die 'Stage5 source BatchId must be deterministic rather than random'
grep -Fq 'uuid.uuid5' "$range_raw" \
  || die 'Stage5 deterministic UUIDv5 derivation is missing'
grep -Fq 'verify_image_provenance' "$range_raw" \
  || die 'Stage5 image provenance gate is missing'
grep -Fq 'lagrange-station-research-range-raw:${commit}' "$range_raw" \
  || die 'Stage5 must inspect the immutable commit-tagged range image directly'
grep -Fq 'image: lagrange-station-research-range-raw:' "$root/deploy/compose/compose.yml" \
  || die 'Stage5 Compose service must have an immutable commit-tagged image name'
if grep -Fq 'compose images -q research-range-raw' "$range_raw"; then
  die 'Stage5 must not resolve provenance through a mutable Compose image lookup'
fi
grep -Fq 'org.opencontainers.image.revision' "$range_raw" \
  || die 'Stage5 image OCI revision inspection is missing'
grep -Fq 'image_commit' "$range_raw" \
  || die 'Stage5 image LAGRANGE_CODE_COMMIT inspection is missing'
grep -Fq 'ensure_state_directory' "$range_raw" \
  || die 'Stage5 state directory hardening is missing'
grep -Fq 'ensure_protected_state_file' "$range_raw" \
  || die 'Stage5 state/lock protected-file hardening is missing'
grep -Fq 'verify_lock_fd_identity' "$range_raw" \
  || die 'Stage5 state lock FD identity check is missing'
grep -Fq 'verify_state_file_identity' "$range_raw" \
  || die 'Stage5 state FD identity check is missing'
grep -Fq 'range-raw-egress' "$root/deploy/compose/compose.yml" \
  || die 'Stage5 dedicated egress network is missing'
grep -Fq -- '--existing-source-batch-id' "$range_raw" \
  || die 'Stage5 explicit immutable-source recovery option is missing'
grep -Fq 'state_version=V3' "$range_raw" \
  || die 'Stage5 explicit recovery state version is missing'
grep -Fq 'reused_existing_source' "$root/data-pipelines/collectors/src/bin/research-worker.rs" \
  || die 'Stage5 recovery machine output flag is missing'
grep -Fq 'run_existing_daily_range_raw_stream' "$root/data-pipelines/collectors/src/worker.rs" \
  || die 'Stage5 provider-free existing-source worker path is missing'
grep -Fq 'networks:' "$root/deploy/compose/compose.yml" \
  || die 'Compose network contract is missing'
range_service_block=$(awk '
  $0 == "  research-range-raw:" { inside=1; print; next }
  inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
  inside { print }
' "$root/deploy/compose/compose.yml")
grep -Fq '      - range-raw-egress' <<<"$range_service_block" \
  || die 'Stage5 service is not attached to its dedicated egress network'
if grep -Fq '      - backend' <<<"$range_service_block"; then
  die 'Stage5 service must not attach to the ordinary backend network'
fi
if grep -Fq 'depends_on:' <<<"$range_service_block"; then
  die 'Stage5 service must not depend on PostgreSQL or another Compose service'
fi

# KIS KSD action-range capture has a separate one-shot image/profile and must
# never inherit Stage5 daily-bars lifecycle or account/order surfaces.
[ -f "$action_range_raw" ] || die 'KIS action-range Raw wrapper is missing'
grep -Fq 'KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS' \
  "$action_range_raw" || die 'KIS action-range execute confirmation missing'
grep -Fq 'compose_profile=action-range-raw' "$action_range_raw" \
  || die 'KIS action-range Compose profile selection missing'
grep -Fq 'compose_service=research-action-range-raw' "$action_range_raw" \
  || die 'KIS action-range Compose service selection missing'
grep -Fq 'compose build --pull=false "$compose_service"' "$action_range_raw" \
  || die 'KIS action-range image build gate missing'
grep -Fq 'docker image inspect "$image"' "$action_range_raw" \
  || die 'KIS action-range image provenance gate missing'
grep -Fq 'compose run --rm --no-deps' "$action_range_raw" \
  || die 'KIS action-range must run one isolated no-deps container'
grep -Fq 'research-worker daemon is running' "$action_range_raw" \
  || die 'KIS action-range ordinary-worker overlap guard missing'
grep -Fq 'another research-action-range-raw one-shot is already running' "$action_range_raw" \
  || die 'KIS action-range duplicate-run guard missing'
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$action_range_raw" \
  || die 'KIS action-range clean-tree guard missing'
grep -Fq -- '--scope range-raw --env-file' "$action_range_raw" \
  || die 'KIS action-range production read-only scope gate missing'
if grep -Eiq 'docker[[:space:]]+compose[^\n]*(up|start|stop|restart)|systemctl|sudo' "$action_range_raw"; then
  die 'KIS action-range wrapper must not manage worker/container lifecycle'
fi
if grep -Eiq 'KIS_ACCOUNT_REF|(^|[^[:alnum:]_])CANO([^[:alnum:]_]|$)|ACNT_PRDT_CD|--profile[[:space:]]+live' "$action_range_raw"; then
  die 'KIS action-range wrapper must not add account/order/live surface'
fi
grep -Fq 'cargo build --locked --release --package collectors --bin kis-action-range-raw' \
  "$worker_dockerfile" || die 'KIS action-range binary build is missing'
grep -Fq 'COPY --from=builder /build/target/release/kis-action-range-raw /usr/local/bin/kis-action-range-raw' \
  "$worker_dockerfile" || die 'KIS action-range binary copy is missing'
action_service_block=$(awk '
  $0 == "  research-action-range-raw:" { inside=1; print; next }
  inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
  inside { print }
' "$root/deploy/compose/compose.yml")
grep -Fq 'profiles: ["action-range-raw"]' <<<"$action_service_block" \
  || die 'KIS action-range service profile is missing'
grep -Fq 'image: lagrange-station-research-action-range-raw:' <<<"$action_service_block" \
  || die 'KIS action-range service image tag is missing'
grep -Fq 'entrypoint: ["/usr/local/bin/kis-action-range-raw"]' <<<"$action_service_block" \
  || die 'KIS action-range service entrypoint is missing'
grep -Fq 'user: "10001:10001"' <<<"$action_service_block" \
  || die 'KIS action-range service UID/GID fence is missing'
grep -Fq 'read_only: true' <<<"$action_service_block" \
  || die 'KIS action-range service rootfs must be read-only'
grep -Fq '      - /tmp' <<<"$action_service_block" \
  || die 'KIS action-range service tmpfs is missing'
grep -Fq '${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw' <<<"$action_service_block" \
  || die 'KIS action-range Raw write mount is missing'
grep -Fq '      - range-raw-egress' <<<"$action_service_block" \
  || die 'KIS action-range service must use dedicated Raw egress'
if grep -Eiq '^[[:space:]]+- backend$|depends_on:|restart:|healthcheck:|RESEARCH_CURATED_ROOT|DB_|KIS_ACCOUNT_REF|CANO|ACNT_PRDT_CD|COMPOSE_PROFILES|--profile[[:space:]]+live' <<<"$action_service_block"; then
  die 'KIS action-range service exposes a forbidden dependency, credential, or lifecycle surface'
fi
[ "$(grep -Ec '^      - source:' <<<"$action_service_block")" -eq 2 ] \
  || die 'KIS action-range service must mount exactly two KIS secrets'
grep -Fq 'source: research_range_raw_kis_app_key' <<<"$action_service_block" \
  || die 'KIS action-range service must reuse the existing KIS key secret'
grep -Fq 'source: research_range_raw_kis_app_secret' <<<"$action_service_block" \
  || die 'KIS action-range service must reuse the existing KIS secret secret'
[ -f "$action_range_guard" ] || die 'KIS action-range worker protection wrapper is missing'
grep -Fq 'lagrange-station/research-worker' "$action_range_guard" \
  || die 'KIS action-range worker protection must verify exact Compose labels'
grep -Fq 'docker stop --time 300 "$worker_id"' "$action_range_guard" \
  || die 'KIS action-range worker protection must stop the exact worker'
grep -Fq 'docker start "$worker_id"' "$action_range_guard" \
  || die 'KIS action-range worker protection must restore the same worker'
grep -Fq 'KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS' \
  "$action_range_guard" || die 'KIS action-range worker protection confirmation missing'
grep -Fq -- '--scope etf11' "$action_range_guard" \
  || die 'KIS action-range worker protection scope must stay ETF11'
if grep -Eiq 'KIS_ACCOUNT_REF|(^|[^[:alnum:]_])CANO([^[:alnum:]_]|$)|ACNT_PRDT_CD|--profile[[:space:]]+live' "$action_range_guard"; then
  die 'KIS action-range worker protection must not add account/order/live surface'
fi
state_line=$(grep -nF 'write_state RUNNING' "$range_raw" | head -n1 | cut -d: -f1)
build_line=$(grep -nF 'compose build --pull=false "$compose_service"' "$range_raw" | head -n1 | cut -d: -f1)
run_line=$(grep -nF 'RANGE_RAW_BATCH_ID="$stored_batch_id" compose run' "$range_raw" | head -n1 | cut -d: -f1)
[ -n "$state_line" ] && [ -n "$build_line" ] && [ "$state_line" -lt "$build_line" ] \
  || die 'Stage5 must write RUNNING state before image build/network work'
[ -n "$run_line" ] && [ "$build_line" -lt "$run_line" ] \
  || die 'Stage5 must invoke the one-shot only after its image build'
grep -Fq 'args:' "$root/deploy/compose/compose.yml" \
  || die 'Compose build argument blocks are missing'
grep -Fq 'kis-daily-range-to-session-bars-v2' "$range_raw" \
  || die 'Stage5 normalizer identity missing'
grep -Fq 'strict PIT' "$range_raw" \
  || die 'Stage5 non-PIT limitation missing'
stage5_doc="$root/docs/runbooks/kis-range-raw-stage5.md"
[ -f "$stage5_doc" ] || die 'Stage5 runbook is missing'
grep -Fq 'research-range-raw' "$stage5_doc" || die 'Stage5 runbook service boundary missing'
grep -Fq 'cannot claim strict historical PIT' "$stage5_doc" \
  || die 'Stage5 runbook PIT limitation missing'
grep -Fq 'Any non-empty continuation marker' "$stage5_doc" \
  || die 'Stage5 runbook pagination limitation missing'
grep -Fq 'reused_existing_source=true' "$stage5_doc" \
  || die 'Stage5 explicit recovery output contract missing'

grep -Fq -- 'range-raw-recovery' "$root/scripts/ops/validate-production-config.sh" \
  || die 'Stage5 recovery validator scope is missing'
grep -Fq -- 'range-raw-recovery' "$root/deploy/secrets/provision-runtime-secrets.sh" \
  || die 'Stage5 recovery runtime scope is missing'
recovery_service_block=$(awk '
  $0 == "  research-range-raw-recovery:" { inside=1; print; next }
  inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
  inside { print }
' "$root/deploy/compose/compose.yml")
grep -Fq 'profiles: ["range-raw-recovery"]' <<<"$recovery_service_block" \
  || die 'Stage5 recovery service profile is missing'
grep -Fq 'network_mode: none' <<<"$recovery_service_block" \
  || die 'Stage5 recovery service must use network_mode:none'
if grep -Eiq 'KIS_APP|secrets:|range-raw-egress|backend:|postgres' <<<"$recovery_service_block"; then
  die 'Stage5 recovery service must not expose KIS secrets, networks, or DB dependencies'
fi

xkrx_override="$root/data/calendars/xkrx/overrides.json"
[ -f "$xkrx_override" ] || die 'XKRX source-backed override ledger is missing'
grep -Fq 'national_election_day' "$xkrx_override" \
  || die 'XKRX election-day override is missing'
grep -Fq 'constitution_day_public_holiday' "$xkrx_override" \
  || die 'XKRX Constitution-Day override is missing'

bash "$ops/build-production-images-static-check.sh" >/dev/null ||
  die 'production image build static check failed'
bash "$ops/production-ops-static-check.sh" >/dev/null ||
  die 'production release/backup static check failed'

xkrx_bootstrap="$ops/xkrx-calendar-bootstrap.py"
[ -x "$xkrx_bootstrap" ] || die 'XKRX calendar bootstrap must be executable'
python3 "$xkrx_bootstrap" --check --end 2026-08-28 >/dev/null ||
  die 'checked-in XKRX calendar bootstrap artifact failed validation'
grep -Fq 'exchange_calendars==4.13.2' "$root/nt/pyproject.toml" ||
  die 'XKRX bootstrap dependency pin is missing'
grep -Fq 'fc5a2ad0d61b5c3a6539a3061cd4cbb55c59f4a903455cec7926e4b798919996' "$xkrx_bootstrap" ||
  die 'XKRX bootstrap wheel hash is missing'
grep -Fq 'a9459425dd64142cd54fbc639847403c7e0c33d60fbc326c94fc1d6bd127f002' "$xkrx_bootstrap" ||
  die 'XKRX bootstrap source hash is missing'
grep -Fq 'source_authority' "$xkrx_bootstrap" ||
  die 'XKRX bootstrap third-party provenance is missing'
grep -Fq 'historical-session-dates-only' "$xkrx_bootstrap" ||
  die 'XKRX bootstrap dates-only contract is missing'
grep -Fq "get_calendar(CALENDAR_NAME, start=str(start), end=str(end))" "$xkrx_bootstrap" ||
  die 'XKRX bootstrap must query explicit calendar bounds'
if grep -Fq 'sessions_in_range' "$xkrx_bootstrap"; then
  die 'XKRX bootstrap must validate explicit schedule labels without a second bounded API query'
fi
if grep -Fq 'package_available' "$xkrx_bootstrap"; then
  die 'XKRX bootstrap must not bypass uv with a global package'
fi
grep -Fq 'uv' "$xkrx_bootstrap" || die 'XKRX bootstrap locked uv execution is missing'
grep -Fq -- '--emit-sessions' "$xkrx_bootstrap" || die 'XKRX validated session emitter is missing'
grep -Fq 'requested_range' "$xkrx_bootstrap" || die 'XKRX session metadata must identify the requested range'
grep -Fq 'artifact_sha256' "$xkrx_bootstrap" || die 'XKRX session metadata must expose the artifact hash'

tls_static="$root/deploy/systemd/tailscale-tls-renewal-static-check.sh"
[ -x "$tls_static" ] || die 'Tailscale TLS renewal static check must be executable'
bash "$tls_static" >/dev/null || die 'Tailscale TLS renewal static check failed'

auth0_secret="$ops/provision-auth0-secret.sh"
grep -Fq 'mode=dry-run' "$auth0_secret" \
  || die 'Auth0 secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$auth0_secret" \
  || die 'Auth0 secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$auth0_secret" \
  || die 'Auth0 secret check root guard missing'
grep -Fq -- '--apply must run as root' "$auth0_secret" \
  || die 'Auth0 secret apply root guard missing'
grep -Fq -- '--import-file must run as root' "$auth0_secret" \
  || die 'Auth0 secret import root guard missing'
grep -Fq -- '--import-file' "$auth0_secret" \
  || die 'Auth0 secret import mode missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$auth0_secret" \
  || die 'Auth0 secret source default missing'
grep -Fq 'target_name=auth0_client_secret' "$auth0_secret" \
  || die 'Auth0 secret target name missing'
grep -Fq 'must not contain' "$auth0_secret" \
  || die 'Auth0 secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$auth0_secret" \
  || die 'Auth0 secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$auth0_secret" \
  || die 'Auth0 secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$auth0_secret" \
  || die 'Auth0 secret source write fence missing'
grep -Fq 'read -r -s' "$auth0_secret" \
  || die 'Auth0 secret apply must use hidden terminal input'
grep -Fq '/dev/tty' "$auth0_secret" \
  || die 'Auth0 secret apply must read from a terminal'
grep -Fq 'placeholder_pattern' "$auth0_secret" \
  || die 'Auth0 secret placeholder rejection missing'
grep -Fq 'ln -T' "$auth0_secret" \
  || die 'Auth0 secret atomic no-clobber install missing'
grep -Fq 'AUTH0_SECRET_CHECK: PASS' "$auth0_secret" \
  || die 'Auth0 secret check pass output missing'
grep -Fq "'%u:%g:%a'" "$auth0_secret" \
  || die 'Auth0 secret check ownership/mode inspection missing'
grep -Fq 'wc -c' "$auth0_secret" \
  || die 'Auth0 secret check byte-length inspection missing'
grep -Fq 'legacy Auth0 secret source must not be group/other accessible' "$auth0_secret" \
  || die 'Auth0 legacy source mode fence missing'
grep -Fq 'cp -- "$import_file" "$staged"' "$auth0_secret" \
  || die 'Auth0 import staged-copy fence missing'
for forbidden in curl wget docker psql openssl; do
  if grep -Eiq "^[^#]*($forbidden)" "$auth0_secret"; then
    die "Auth0 secret provisioner must not reference $forbidden"
  fi
done

crypto_secrets="$ops/provision-crypto-secrets.sh"
grep -Fq 'mode=dry-run' "$crypto_secrets" \
  || die 'crypto secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$crypto_secrets" \
  || die 'crypto secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$crypto_secrets" \
  || die 'crypto secret check root guard missing'
grep -Fq -- '--apply must run as root' "$crypto_secrets" \
  || die 'crypto secret apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$crypto_secrets" \
  || die 'crypto secret source default missing'
grep -Fq 'session_secret' "$crypto_secrets" \
  || die 'session secret inventory missing'
grep -Fq 'csrf_secret' "$crypto_secrets" \
  || die 'CSRF secret inventory missing'
grep -Fq 'cursor_secret' "$crypto_secrets" \
  || die 'cursor secret inventory missing'
grep -Fq 'backup_encryption_key' "$crypto_secrets" \
  || die 'backup encryption key inventory missing'
grep -Fq 'must not contain' "$crypto_secrets" \
  || die 'crypto secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$crypto_secrets" \
  || die 'crypto secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$crypto_secrets" \
  || die 'crypto secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$crypto_secrets" \
  || die 'crypto secret source write fence missing'
grep -Fq 'openssl rand -hex 32' "$crypto_secrets" \
  || die 'crypto secret generator must use 256-bit OpenSSL values'
grep -Fq 'cmp -s' "$crypto_secrets" \
  || die 'crypto secret distinctness check missing'
grep -Fq 'CRYPTO_SECRET_CHECK: PASS' "$crypto_secrets" \
  || die 'crypto secret check pass output missing'
grep -Fq 'ln -T' "$crypto_secrets" \
  || die 'crypto secret atomic no-clobber install missing'
grep -Fq "'%u:%g:%a'" "$crypto_secrets" \
  || die 'crypto secret ownership/mode inspection missing'
grep -Fq 'wc -c' "$crypto_secrets" \
  || die 'crypto secret shape inspection missing'
for forbidden in curl wget docker psql; do
  if grep -Eiq "^[^#]*($forbidden)" "$crypto_secrets"; then
    die "crypto secret provisioner must not reference $forbidden"
  fi
done

kis_credentials="$ops/provision-kis-credentials.sh"
grep -Fq 'mode=dry-run' "$kis_credentials" \
  || die 'KIS credential provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$kis_credentials" \
  || die 'KIS credential read-only check mode missing'
grep -Fq -- '--check must run as root' "$kis_credentials" \
  || die 'KIS credential check root guard missing'
grep -Fq -- '--apply must run as root' "$kis_credentials" \
  || die 'KIS credential apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$kis_credentials" \
  || die 'KIS credential source default missing'
grep -Fq 'key_name=kis_app_key' "$kis_credentials" \
  || die 'KIS app-key inventory missing'
grep -Fq 'secret_name=kis_app_secret' "$kis_credentials" \
  || die 'KIS app-secret inventory missing'
grep -Fq 'must not contain' "$kis_credentials" \
  || die 'KIS credential dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$kis_credentials" \
  || die 'KIS credential ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$kis_credentials" \
  || die 'KIS credential source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$kis_credentials" \
  || die 'KIS credential source write fence missing'
grep -Fq 'read -r -s -u 3' "$kis_credentials" \
  || die 'KIS credential apply must use hidden terminal input'
grep -Fq '/dev/tty' "$kis_credentials" \
  || die 'KIS credential apply must read from a terminal'
grep -Fq 'placeholder_pattern' "$kis_credentials" \
  || die 'KIS credential placeholder rejection missing'
grep -Fq 'max_secret_bytes=4096' "$kis_credentials" \
  || die 'KIS credential local length guard missing'
grep -Fq 'cmp -s' "$kis_credentials" \
  || die 'KIS credential pair distinctness check missing'
grep -Fq 'ln -T' "$kis_credentials" \
  || die 'KIS credential atomic no-clobber install missing'
grep -Fq 'installed_signatures' "$kis_credentials" \
  || die 'KIS credential pair rollback tracking missing'
grep -Fq 'KIS_CREDENTIAL_CHECK: PASS' "$kis_credentials" \
  || die 'KIS credential check pass output missing'
grep -Fq 'source directory is absent or protected from current user' "$kis_credentials" \
  || die 'KIS credential dry-run must not infer absence from access denial'
grep -Fq "'%u:%g:%a'" "$kis_credentials" \
  || die 'KIS credential ownership/mode inspection missing'
grep -Fq 'wc -c' "$kis_credentials" \
  || die 'KIS credential byte-length inspection missing'
grep -Fq 'wc -l' "$kis_credentials" \
  || die 'KIS credential newline-shape inspection missing'
grep -Fq 'install -o root -g root -m 0600' "$kis_credentials" \
  || die 'KIS credential owner/mode install fence missing'
for forbidden in curl wget docker psql openssl tailscale; do
  if grep -Eiq "^[^#]*($forbidden)" "$kis_credentials"; then
    die "KIS credential provisioner must not reference $forbidden"
  fi
done

db_secrets="$ops/provision-db-secrets.sh"
grep -Fq 'mode=dry-run' "$db_secrets" \
  || die 'DB secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$db_secrets" \
  || die 'DB secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$db_secrets" \
  || die 'DB secret check root guard missing'
grep -Fq 'mode=normalize' "$db_secrets" \
  || die 'DB secret newline normalizer mode missing'
grep -Fq -- '--strip-trailing-newline' "$db_secrets" \
  || die 'DB secret newline normalizer option missing'
grep -Fq -- '--apply must run as root' "$db_secrets" \
  || die 'DB secret apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$db_secrets" \
  || die 'DB secret source default missing'
grep -Fq 'must not contain' "$db_secrets" \
  || die 'DB secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$db_secrets" \
  || die 'DB secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$db_secrets" \
  || die 'DB secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$db_secrets" \
  || die 'DB secret source write fence missing'
grep -Fq 'source_mode_bits & 0022' "$db_secrets" \
  || die 'DB secret source write mask must preserve group/other read access'
grep -Fq 'openssl rand -hex 32' "$db_secrets" \
  || die 'DB secret generator must use 256-bit OpenSSL values'
grep -Fq 'cmp -s' "$db_secrets" \
  || die 'DB secret distinctness check missing'
grep -Fq 'cmp -s --' "$db_secrets" \
  || die 'DB secret read-only equality check must use silent cmp'
grep -Fq 'DB_SECRET_CHECK: PASS' "$db_secrets" \
  || die 'DB secret check pass output missing'
grep -Fq 'DB_SECRET_NORMALIZE: PASS' "$db_secrets" \
  || die 'DB secret normalizer pass output missing'
grep -Fq 'base64 --decode' "$db_secrets" \
  || die 'DB secret Base64 decoder check missing'
grep -Fq 'has_single_trailing_newline' "$db_secrets" \
  || die 'DB secret newline-shape check missing'
grep -Fq 'mv -T' "$db_secrets" \
  || die 'DB secret normalizer atomic replacement missing'
grep -Fq "'%u:%g:%a'" "$db_secrets" \
  || die 'DB secret check ownership/mode inspection missing'
grep -Fq "wc -c <\"\$target\"" "$db_secrets" \
  || die 'DB secret check byte-length inspection missing'
grep -Fq 'install -o root -g root -m 0600' "$db_secrets" \
  || die 'DB secret owner fence missing'
grep -Fq '0600' "$db_secrets" \
  || die 'DB secret mode fence missing'
for forbidden in docker curl psql kis api; do
  if grep -Eiq "^[^#]*(\\$forbidden|$forbidden)" "$db_secrets"; then
    die "DB secret provisioner must not reference $forbidden"
  fi
done

grep -Fq 'DRY_RUN: no host changes made' "$ops/provision-linux.sh" || die 'provision dry-run contract missing'
grep -Fq -- '--apply must run as root' "$ops/provision-linux.sh" || die 'provision root guard missing'
grep -Fq -- '--preflight must run as root' "$ops/provision-linux.sh" || die 'provision preflight root guard missing'
grep -Fq 'must not traverse a symlink' "$ops/provision-linux.sh" || die 'provision ancestor symlink fence missing'
grep -Fq 'service user is not a member of service group' "$ops/provision-linux.sh" || die 'service group membership fence missing'
grep -Fq 'BLOCKED_EXTERNAL' "$ops/validate-production-config.sh" || die 'config blocker contract missing'
grep -Fq -- '--scope infrastructure|serving-prereqs|backfill|range-raw|range-raw-recovery|release' "$ops/validate-production-config.sh" || die 'config scope contract missing'
grep -Fq -- 'validation must run as root to inspect protected production paths' "$ops/validate-production-config.sh" \
  || die 'config validator root guard missing'
grep -Fq 'LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"' "$ops/validate-production-config.sh" \
  || die 'config validator sudo commit-preservation guidance missing'
grep -Fq 'validator fixture checks skipped for non-root caller' "$ops/self-test.sh" \
  || die 'self-test must account for the validator root contract'
grep -Fq 'dotenv_validate_shell_overrides' "$ops/validate-production-config.sh" || die 'shell/env-file precedence fence missing'
grep -Fq 'KIS read-only' "$ops/validate-production-config.sh" || die 'KIS read-only contract missing'
grep -Fq 'mode 0400 or 0600' "$ops/validate-production-config.sh" || die 'source secret mode contract missing'
grep -Fq 'runtime secret' "$ops/validate-production-config.sh" || die 'runtime secret validation missing'
grep -Fq 'serving-prereqs scope checks Auth0/TLS' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs readiness contract missing'
grep -Fq 'backup_encryption_key' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs source inventory missing backup key'
grep -Fq 'research-worker/db_research_password:10001:10001:440' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs runtime inventory missing research DB copy'
grep -Fq 'crypto_placeholder_pattern' "$ops/validate-production-config.sh" \
  || die 'validator crypto placeholder contract is missing'
grep -Fq "grep -Eq '^[0-9a-f]{64}$'" "$ops/validate-production-config.sh" \
  || die 'validator crypto lowercase-hex contract is missing'
grep -Fq 'crypto source secrets must be distinct' "$ops/validate-production-config.sh" \
  || die 'validator crypto distinctness contract is missing'
grep -Fq 'db_secret_names=' "$ops/validate-production-config.sh" || die 'DB secret distinctness inventory missing'
grep -Fq 'DB source secrets must be distinct' "$ops/validate-production-config.sh" || die 'DB secret distinctness blocker missing'
grep -Fq 'cmp -s' "$ops/validate-production-config.sh" || die 'DB secret equality check missing'
grep -Fq 'run --rm --no-deps db-role-bootstrap' "$ops/compose-release.sh" || die 'role bootstrap ordering missing'
grep -Fq 'run --rm --no-deps db-migrate' "$ops/compose-release.sh" || die 'migration ordering missing'
grep -Fq 'build --pull=false \' "$ops/compose-release.sh" || die 'Compose build gate missing'
grep -Fq 'db-role-bootstrap db-migrate' "$ops/compose-release.sh" || die 'one-shot images are not built before run'
grep -Fq 'up --wait --no-deps api-server' "$ops/compose-release.sh" || die 'serving stage must not rerun removed one-shots'
grep -Fq -- '--scope infrastructure|backfill|release' "$ops/compose-release.sh" || die 'Compose scope contract missing'
if grep -Fq 'serving-prereqs' "$ops/compose-release.sh"; then
  die 'serving-prereqs must remain copy/readiness-only and absent from Compose execution'
fi
grep -Fq 'LAGRANGE_DATA_ROOT="$data_dir"' "$ops/compose-release.sh" || die 'Compose preflight must use env-file data root'
grep -Fq 'COMPOSE_BACKFILL_BOOTSTRAP_ORDER' "$ops/compose-release.sh" || die 'backfill Compose bootstrap order missing'
grep -Fq 'COMPOSE_INFRASTRUCTURE_ORDER' "$ops/compose-release.sh" || die 'infrastructure Compose order missing'
grep -Fq 'compose build --pull=false db-role-bootstrap db-migrate' "$ops/compose-release.sh" \
  || die 'infrastructure Compose build gate missing'
grep -Fq 'COMPOSE_INFRASTRUCTURE: PASS' "$ops/compose-release.sh" \
  || die 'infrastructure Compose apply gate missing'
grep -Fq 'RESEARCH_APP_ENV=infrastructure-disabled' "$ops/compose-release.sh" \
  || die 'infrastructure Compose research sentinel missing'
grep -Fq 'RESEARCH_ENTITLEMENT_REFERENCE=infrastructure-disabled' "$ops/compose-release.sh" \
  || die 'infrastructure Compose entitlement sentinel missing'
for key in BACKTEST_MIN_FREE_BYTES BACKTEST_MAX_QUEUED_BACKTESTS \
  BACKTEST_RECONCILE_GRACE_SECS BACKTEST_RECONCILE_INTERVAL_SECS; do
  grep -Fq "$key=0" "$ops/compose-release.sh" \
    || die "infrastructure Compose $key sentinel missing"
done
grep -Fq 'process-local, fail-closed sentinels' "$ops/compose-release.sh" \
  || die 'infrastructure Compose sentinel scope documentation missing'
grep -Fq 'up --no-deps -d research-worker recommendation-runner candidate-runner' "$ops/compose-release.sh" \
  || die 'data-dependent services must bootstrap without a clean-install health wait'
grep -Fq 'disabled leaves owner-beta-runner inactive' "$ops/compose-release.sh" \
  || die 'release plan must retain disabled owner-beta-runner behavior'
grep -Fq 'candidate-runner owner-beta-runner' "$ops/compose-release.sh" \
  || die 'owner-only release worker order must include owner-beta-runner'
grep -Fq 'post-backfill-health.sh --check' "$ops/compose-release.sh" \
  || die 'post-backfill data readiness gate is not documented in Compose release'
grep -Fq 'research-worker healthcheck' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must invoke the existing worker healthcheck'
[ "$(stat -c '%a' "$ops/post-backfill-health.sh")" = 755 ] \
  || die 'post-backfill-health.sh must have exact mode 0755'
grep -Fq -- '--scope backfill|release' "$ops/post-backfill-health.sh" \
  || die 'post-backfill scope contract missing'
grep -Fq 'run --rm --no-deps research-worker healthcheck' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must avoid dependency restarts'
grep -Fq 'does not require a worker daemon' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must not require a worker daemon'
review_report="$ops/backfill-review-report.sh"
grep -Fq 'CURATED_CANDIDATE_FOUND_UNAPPROVED' "$review_report" \
  || die 'backfill review report must remain explicitly non-approving'
grep -Fq 'DB_READY=NOT_CHECKED' "$review_report" \
  || die 'backfill review report must not claim database readiness'
grep -Fq 'PLAN_ONLY: no production file read, write, or external service action made' \
  "$review_report" || die 'backfill review plan must be local-only'
for forbidden in docker psql curl wget tailscale systemctl; do
  if grep -Eiq "^[^#]*$forbidden" "$review_report"; then
    die "backfill review report must not invoke $forbidden"
  fi
done
[ "$(stat -c '%a' "$review_report")" = 755 ] \
  || die 'backfill-review-report.sh must have exact mode 0755'
grep -Fq 'PLAN_ONLY: no KIS call' "$ops/backfill-production.sh" || die 'backfill must default to no-call plan'
grep -Fq 'KOSPI200/KOSDAQ150 credentialed candidate bridge' "$ops/backfill-production.sh" || die 'candidate blocker missing'
grep -Fq 'LAGRANGE_BACKFILL_STATE_V4' "$ops/backfill-production.sh" || die 'backfill state identity schema missing'
grep -Fq -- '--scope backfill' "$ops/backfill-production.sh" || die 'backfill must use backfill config scope'
grep -Fq 'state_file=/var/lib/lagrange/state/backfill/state.tsv' "$ops/backfill-production.sh" \
  || die 'backfill state default must use the root-owned state tree'
grep -Fq 'validate_trusted_state_ancestors' "$ops/backfill-production.sh" \
  || die 'backfill state ancestors must be trust-boundary checked'
grep -Fq 'dotenv_validate_shell_overrides' "$ops/backfill-production.sh" \
  || die 'backfill must share shell/env-file precedence fence'
grep -Fq 'start_date=$start_date' "$ops/backfill-production.sh" || die 'backfill identity must bind the requested date range'
grep -Fq 'calendar_artifact_sha256=$calendar_artifact_sha256' "$ops/backfill-production.sh" \
  || die 'backfill identity must bind the XKRX artifact hash'
grep -Fq 'calendar_artifact_range=$calendar_artifact_range' "$ops/backfill-production.sh" \
  || die 'backfill identity must bind the XKRX artifact range'
grep -Fq 'calendar_dir=${LAGRANGE_XKRX_CALENDAR_DIR' "$ops/backfill-production.sh" \
  || die 'backfill XKRX artifact path override is missing'
grep -Fq 'non-session skips:' "$ops/backfill-production.sh" \
  || die 'backfill plan must report non-session skips'
grep -Fq 'dataset_version_id' "$ops/backfill-production.sh" && die 'backfill identity must not bind future dataset pins'
grep -Fq 'flock -n 9' "$ops/backfill-production.sh" || die 'backfill state lock missing'
grep -Fq 'ensure_state_directory' "$ops/backfill-production.sh" \
  || die 'backfill state directory hardening missing'
grep -Fq 'ensure_protected_state_file' "$ops/backfill-production.sh" \
  || die 'backfill state/lock protected-file hardening missing'
grep -Fq 'verify_lock_fd_identity' "$ops/backfill-production.sh" \
  || die 'backfill lock descriptor identity check missing'
grep -Fq 'set -C' "$ops/backfill-production.sh" \
  || die 'backfill state/lock creation must use exclusive noclobber semantics'
grep -Fq -- '--emit-sessions --start "$start_date" --end "$end_date"' \
  "$ops/backfill-production.sh" || die 'backfill must use the validated XKRX session emitter'
grep -Fq 'session_dates_csv' "$ops/backfill-production.sh" \
  || die 'backfill must preserve the exact validated session list'
grep -Fq -- '--backfill-session-dates "$session_dates_csv"' \
  "$ops/backfill-production.sh" || die 'backfill must pass only validated session dates to the worker'
grep -Fq 'SESSION_DATES_CSV' "$ops/lib/backfill-progress.py" \
  || die 'backfill progress must validate the exact session sequence'
if grep -Fq -- '--backfill-range' "$root/data-pipelines/collectors/src/bin/research-worker.rs"; then
  die 'public research-worker backfill range bypass must be removed'
fi
if grep -Fq -- 'research-worker --once --date "$date"' "$ops/backfill-production.sh"; then
  die 'backfill must not create one token-owning worker process per date'
fi
grep -Fq 'ps --status running --services' "$ops/backfill-production.sh" \
  || die 'backfill must refuse a concurrently running research-worker daemon'
grep -Fq 'token_window_file="${state_file}.token-window"' "$ops/backfill-production.sh" \
  || die 'backfill cross-process token issue window missing'
grep -Fq 'chmod 0600 "$token_window_tmp"' "$ops/backfill-production.sh" \
  || die 'backfill token issue window mode contract missing'
grep -Fq 'MIN_ISSUE_INTERVAL_MS: i64 = 60_000' "$root/crates/kis-client/src/auth.rs" \
  || die 'KIS token manager one-minute issue safeguard missing'
grep -Fq 'DEFAULT_TTL_SECS: i64 = 86_400' "$root/crates/kis-client/src/token_issuer.rs" \
  || die 'KIS token fallback TTL must match the documented 24-hour lifetime'
grep -Fq 'backfill-progress.py' "$ops/backfill-production.sh" \
  || die 'backfill must durably consume per-date worker progress'
grep -Fq 'os.fsync(state.fileno())' "$ops/lib/backfill-progress.py" \
  || die 'backfill per-date progress must be durable before the next date'
grep -Fq 'record.get("phase") == "canonical_publication"' \
  "$ops/lib/backfill-progress.py" \
  || die 'backfill progress must distinguish canonical EOD from final Curated recovery'
grep -Fq 'KIS_CALENDAR_SNAPSHOT_MISS' "$ops/lib/backfill-progress.py" \
  || die 'backfill progress must identify the only deferred calendar error'
grep -Fq 'DEFERRED_EXIT = 75' "$ops/lib/backfill-progress.py" \
  || die 'backfill deferred exit contract missing'
grep -Fq 'automatic resume is blocked' "$ops/backfill-production.sh" \
  || die 'backfill automatic permanent-error halt missing'
grep -Fq 'check_auto_resume_state' "$ops/backfill-production.sh" \
  || die 'backfill automatic resume state gate missing'
grep -Fq 'printf '\''%s\\tFAILED\\t%s'\'' "$date" "$run_identity"' \
  "$ops/backfill-production.sh" && die 'backfill must not mark every untouched pending date failed'
grep -Fq 'OnCalendar=*-*-* 03:15:00 Asia/Seoul' "$ops/install-kis-backfill-timer.sh" \
  || die 'recurring KIS backfill timer schedule missing'
grep -Fq 'network-online.target' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill timer network ordering missing'
grep -Fq -- '--auto-resume' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill timer auto-resume flag missing'
grep -Fq 'service=not-started' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill installer must not start the service'
grep -Fq 'SuccessExitStatus=74 75' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill expected retry/deferred exit contract missing'
grep -Fq 'apply_window_is_open' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill installer schedule safety gate missing'
grep -Fq -- '--apply is allowed only before 03:15:00 Asia/Seoul' \
  "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill installer must reject catch-up-prone late installation'
grep -Fq 'KIS_BACKFILL_TIMER_TEST_NOW' "$ops/install-kis-backfill-timer.sh" \
  || die 'KIS backfill timer boundary test hook missing'
if grep -Eq 'compose[^#]*--profile[[:space:]]+live|--profile[[:space:]]+live' "$ops"/*.sh; then
  die 'operator workflow must not enable the live profile'
fi
fsc_self_test="$ops/fsc-krx-listed-self-test.sh"
fsc_collector="$root/data-pipelines/collectors/src/bin/fsc-krx-listed-raw.rs"
[ -x "$fsc_self_test" ] || die 'FSC offline self-test must be executable'
[ -f "$fsc_collector" ] || die 'FSC collector source is missing'
for forbidden in '--approve-live' '--approve-live-probe' 'DataGoClient' \
  'I_UNDERSTAND_READ_ONLY_FSC_KRX_LISTED_CALLS'; do
  if grep -Fq -- "$forbidden" "$fsc_collector"; then
    die "FSC collector must not contain $forbidden"
  fi
done
for forbidden_wrapper in grant-fsc-krx-listed-temporary-access.sh \
  provision-fsc-krx-listed-key.sh; do
  [ ! -e "$ops/$forbidden_wrapper" ] && [ ! -L "$ops/$forbidden_wrapper" ] ||
    die "obsolete FSC live wrapper must be absent: $forbidden_wrapper"
done
fsc_runner="$ops/run-fsc-krx-listed.sh"
[ -x "$fsc_runner" ] || die 'FSC offline runner must be executable'
grep -Fq -- '--plan|--check' "$fsc_runner" || die 'FSC runner must be offline-only'
if grep -Eq -- '--approve-live|--approve-live-probe|--live|--execute|FSC_KRX_LISTED_KEY_FILE|FSC_KRX_LISTED_CONFIRM|systemctl|sudo' "$fsc_runner"; then
  die 'FSC runner must not retain a live/provider activation path'
fi
bash "$fsc_self_test" >/dev/null || die 'FSC offline self-test failed'

kis_daily="$ops/kis-daily-production.sh"
kis_daily_installer="$ops/install-kis-daily.sh"
kis_daily_self_test="$ops/kis-daily-production-self-test.sh"
kis_daily_state="$ops/lib/kis-daily-state.py"
[ -x "$kis_daily" ] || die 'KIS daily wrapper must be executable'
[ -x "$kis_daily_installer" ] || die 'KIS daily installer must be executable'
[ -x "$kis_daily_self_test" ] || die 'KIS daily self-test must be executable'
[ -f "$kis_daily_state" ] && [ ! -L "$kis_daily_state" ] || die 'KIS daily state helper is missing or unsafe'
for state_diagnostic in DAILY_STATE_MISSING DAILY_STATE_STALE DAILY_STATE_MALFORMED DAILY_STATE_NOT_APPENDABLE; do
  grep -Fq "$state_diagnostic" "$kis_daily" "$kis_daily_state" \
    || die "KIS daily state diagnostic is missing: $state_diagnostic"
done
grep -Fq 'LAGRANGE_BACKFILL_STATE_V4' "$kis_daily_state" \
  || die 'KIS daily state helper must preserve the V4 state contract'
grep -Fq -- '--plan' "$kis_daily" || die 'KIS daily plan mode is missing'
grep -Fq -- '--check' "$kis_daily" || die 'KIS daily check mode is missing'
grep -Fq -- '--execute' "$kis_daily" || die 'KIS daily execute mode is missing'
grep -Fq 'source "$script_dir/lib/db.sh"' "$kis_daily" || die 'KIS daily must reuse the shared DB helper'
grep -Fq 'xkrx-calendar-bootstrap.py' "$kis_daily" || die 'KIS daily must use the validated XKRX calendar artifact'
grep -Fq "fetch_mode='credentialed'" "$kis_daily" || die 'KIS daily DB snapshot must select credentialed publications'
grep -Fq "provider='KRX'" "$kis_daily" || die 'KIS daily DB snapshot provider scope is missing'
grep -Fq "market='KR'" "$kis_daily" || die 'KIS daily DB snapshot market scope is missing'
grep -Fq 'ensure_protected_file' "$kis_daily" || die 'KIS daily protected file hardening is missing'
grep -Fq 'verify_lock_fd_identity' "$kis_daily" || die 'KIS daily lock descriptor identity check is missing'
grep -Fq 'flock -n 9' "$kis_daily" || die 'KIS daily single-run lock is missing'
grep -Fq 'max_sessions=10000' "$kis_daily" || die 'KIS daily worker bound is missing'
grep -Fq -- '--backfill-session-dates' "$kis_daily" || die 'KIS daily exact worker session contract is missing'
grep -Fq 'LAGRANGE_BACKFILL_STATE' "$kis_daily" || die 'KIS daily must reuse the protected backfill state contract'
grep -Fq 'BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS' "$kis_daily" \
  || die 'KIS daily execute confirmation is missing'
grep -Fq 'no worker/Docker/KIS call' "$kis_daily" || die 'KIS daily no-op idempotency message is missing'
if grep -Eq 'KIS_ACCOUNT_REF|(^|[^[:alnum:]_])CANO([^[:alnum:]_]|$)|ACNT_PRDT_CD|--profile[[:space:]]+live|KIS_APP_KEY_FILE|KIS_APP_SECRET_FILE' "$kis_daily"; then
  die 'KIS daily wrapper must not add account/live/credential-file surface'
fi
for installer_marker in '--dry-run' '--preflight' '--check' '--apply' \
  'release_root' 'code_commit' 'short_commit' 'service_name' 'timer_name' \
  'LAGRANGE_CODE_COMMIT' 'BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS'; do
  grep -Fq -- "$installer_marker" "$kis_daily_installer" ||
    die "KIS daily installer contract is missing: $installer_marker"
done
if grep -Eq 'deploy/systemd/lagrange-kis-daily\.(service|timer)|kis-daily\.env\.example' "$kis_daily_installer"; then
  die 'KIS daily installer must not recreate legacy static unit/env artifacts'
fi
for legacy in "$root/deploy/systemd/lagrange-kis-daily.service" \
  "$root/deploy/systemd/lagrange-kis-daily.timer" "$root/deploy/systemd/kis-daily.env.example"; do
  [ ! -e "$legacy" ] && [ ! -L "$legacy" ] || die "legacy KIS daily artifact must be absent: $legacy"
done

kis_daily_calendar_refresh="$ops/kis-daily-calendar-refresh.sh"
[ -x "$kis_daily_calendar_refresh" ] || die 'KIS daily protected calendar refresh must be executable'
grep -Fq -- '--plan' "$kis_daily_calendar_refresh" || die 'KIS daily calendar refresh plan mode is missing'
grep -Fq -- '--check' "$kis_daily_calendar_refresh" || die 'KIS daily calendar refresh check mode is missing'
grep -Fq -- '--apply' "$kis_daily_calendar_refresh" || die 'KIS daily calendar refresh apply mode is missing'
grep -Fq 'end_date=2027-12-31' "$kis_daily_calendar_refresh" \
  || die 'KIS daily calendar refresh horizon must reach 2027-12-31'
grep -Fq 'xkrx-calendar-bootstrap.py' "$kis_daily_calendar_refresh" \
  || die 'KIS daily calendar refresh must reuse the pinned XKRX bootstrap'
grep -Fq 'XKRX_CALENDAR_BOOTSTRAP_REEXEC' "$kis_daily_calendar_refresh" \
  || die 'KIS daily calendar refresh must use the locked local Python environment'
refresh_code=$(grep -Ev '^[[:space:]]*#' "$kis_daily_calendar_refresh")
if grep -Eq 'curl|wget|playwright|selenium|browser|KIS.*(GET|POST)|oauth2/token|(^|[^[:alnum:]_])CANO([^[:alnum:]_]|$)|ACNT_PRDT_CD' <<<"$refresh_code"; then
  die 'KIS daily calendar refresh must not add network/browser/account surface'
fi

kind_wrapper="$ops/kind-daily.sh"
kind_installer="$ops/install-kind-daily.sh"
kind_self_test="$ops/kind-daily-self-test.sh"
kind_service="$root/deploy/systemd/lagrange-kind-daily.service"
kind_timer="$root/deploy/systemd/lagrange-kind-daily.timer"
[ -x "$kind_wrapper" ] || die 'KIND manual wrapper must be executable'
[ -x "$kind_installer" ] || die 'KIND installer must be executable'
[ -x "$kind_self_test" ] || die 'KIND self-test must be executable'
grep -Fq -- '--plan' "$kind_wrapper" || die 'KIND plan mode is missing'
grep -Fq -- '--check' "$kind_wrapper" || die 'KIND check mode is missing'
grep -Fq -- '--execute' "$kind_wrapper" || die 'KIND execute mode is missing'
grep -Fq 'target-date-file' "$kind_wrapper" || die 'KIND one-day target input is missing'
grep -Fq 'window_days=1' "$kind_wrapper" || die 'KIND one-day window contract is missing'
grep -Fq 'release_root' "$kind_installer" || die 'KIND installer release input is missing'
grep -Fq 'no_systemd=true' "$kind_installer" || die 'KIND installer no-systemd plan contract is missing'
grep -Fq 'lagrange-kind-daily.service' "$kind_installer" || die 'KIND manual service install contract is missing'
if grep -Eq 'lagrange-kind-daily\.timer|OnCalendar=|Persistent=true' "$kind_installer"; then
  die 'KIND installer must not recreate scheduled/timer activation'
fi
[ -f "$kind_service" ] || die 'KIND manual service is missing'
[ ! -L "$kind_service" ] || die 'KIND manual service must not be a symlink'
[ ! -e "$kind_timer" ] && [ ! -L "$kind_timer" ] || die 'KIND timer must be absent'
grep -Fq 'Type=oneshot' "$kind_service" || die 'KIND service must be a oneshot service'
grep -Fq 'ExecStart=' "$kind_service" || die 'KIND manual service command is missing'
grep -Fq -- '--confirm KIND_DAILY_OPERATOR_CONFIRMATION' "$kind_service" \
  || die 'KIND manual service confirmation is missing'
bash "$kis_daily_self_test" >/dev/null || die 'KIS daily focused self-test failed'
bash "$kind_self_test" >/dev/null || die 'KIND focused self-test failed'
echo 'OPS_STATIC: PASS'
