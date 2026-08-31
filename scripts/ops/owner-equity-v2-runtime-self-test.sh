#!/usr/bin/env bash
# Provider-free fake-runtime self-test for Owner Equity V2.
# It inspects the checked-in contract and exercises only the verifier's plan
# path behind a fake Docker command.  It never reads credentials or starts a
# production container/provider.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
ops=$root/scripts/ops
compose_file=$root/deploy/compose/compose.yml
runner_source=$root/crates/job-queue/src/bin/owner-equity-v2-runner.rs
runtime_source=$root/crates/job-queue/src/owner_equity_v2/runtime.rs
runner_logic=$root/crates/job-queue/src/owner_equity_v2/runner.rs
verify=$ops/owner-equity-v2-verify.sh
static=$ops/owner-equity-v2-runtime-static-check.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/owner-equity-v2-runtime-self-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

die() { echo "owner-equity-v2-runtime-self-test: $*" >&2; exit 1; }

for path in "$compose_file" "$runner_source" "$runtime_source" "$runner_logic" \
  "$verify" "$static"; do
  [ -f "$path" ] || die "required file missing: $path"
done
bash -n "$verify" "$static" || die 'runtime scripts have shell syntax errors'

service_block() {
  local name=$1
  awk -v target="  $name:" '
    $0 == target { inside=1; print; next }
    inside && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    inside { print }
  ' "$compose_file"
}

v2_block=$(service_block owner-equity-v2-runner)
[ -n "$v2_block" ] || die 'V2 worker service is absent'
for expected in \
  'profiles: ["owner-equity-v2"]' \
  'init: true' 'restart: unless-stopped' 'read_only: true' \
  'stop_grace_period: 16m' 'OWNER_EQUITY_V2_MAX_ACTIVE: "100"' \
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
  '/raw:/data/raw:rw' \
  '/owner-equity-v2-artifacts:/data/owner-equity-v2-artifacts:rw' \
  'test: ["CMD", "/usr/local/bin/owner-equity-v2-runner", "healthcheck"]'; do
  grep -Fq -- "$expected" <<<"$v2_block" || die "V2 boundary missing: $expected"
done

for service in api-server web; do
  block=$(service_block "$service")
  if grep -Eiq 'KIS_APP_KEY|KIS_APP_SECRET|OWNER_EQUITY_V2_RAW_ROOT|/data/owner-equity-v2-artifacts' <<<"$block"; then
    die "$service has a V2 credential or Raw/artifact write root"
  fi
done

for expected in \
  'initial_get_ceiling_per_job: 7' \
  'incremental_get_ceiling_per_job: 2' \
  'total_initial_backfill_get_ceiling: 700' \
  'estimated_bytes_per_get: u64' \
  'estimated_job_disk_bytes' \
  'estimated_total_initial_disk_bytes' \
  'concurrency: usize'; do
  grep -Fq -- "$expected" "$runtime_source" || die "preflight field missing: $expected"
done
[ $((7 * 1048576)) -eq 7340032 ] || die 'initial per-job disk estimate arithmetic changed'
[ $((700 * 1048576)) -eq 734003200 ] || die 'initial total disk estimate arithmetic changed'

for rejection in \
  'self.maximum_active_instruments > 100' \
  'self.initial_get_ceiling_per_job < 2' \
  'self.incremental_get_ceiling_per_job < 2' \
  'self.total_initial_backfill_get_ceiling > 700' \
  'self.concurrency != 1' \
  'self.estimated_bytes_per_get == 0'; do
  grep -Fq -- "$rejection" "$runtime_source" || die "bad-ceiling rejection missing: $rejection"
done

for expected in \
  'Mode::Daemon' 'Mode::Once' 'Mode::Healthcheck' \
  'OWNER_EQUITY_V2_HEALTH_MAX_AGE_SECS' \
  'recover_owner_equity_claims(&queue).await' \
  'shutdown_signal().await' 'work.await' 'Quota::new(1, 1)' \
  'OWNER_EQUITY_V2_HEARTBEAT_SECS' 'OWNER_EQUITY_V2_LEASE_SECS'; do
  grep -Fq -- "$expected" "$runner_source" || die "runner lifecycle contract missing: $expected"
done
for expected in \
  'heartbeat_interval >= lease' \
  'OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry' \
  "error_code = 'attempts_exhausted'"; do
  grep -Fq -- "$expected" "$runner_logic" || die "recovery contract missing: $expected"
done

fake_bin=$tmp/bin
fake_log=$tmp/docker.log
mkdir -p "$fake_bin"
cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${OWNER_EQUITY_V2_FAKE_DOCKER_LOG:?}"
echo 'fake Docker must not be called by verifier plan' >&2
exit 97
EOF
chmod 0755 "$fake_bin/docker"
plan=$(PATH="$fake_bin:$PATH" \
  OWNER_EQUITY_V2_FAKE_DOCKER_LOG="$fake_log" \
  LAGRANGE_RELEASE_ROOT="$tmp/does-not-exist" \
  bash "$verify" --plan) || die 'verifier plan unexpectedly failed'
grep -Fq 'network=none' <<<"$plan" || die 'verifier plan omitted network-none boundary'
grep -Fq 'read-only' <<<"$plan" || die 'verifier plan omitted read-only boundary'
grep -Fq 'candidate-hash=required' <<<"$plan" || die 'verifier plan omitted immutable hash requirement'
[ ! -e "$fake_log" ] || die 'verifier plan invoked Docker'

for expected in \
  '--network none' '--read-only' '--cap-drop ALL' \
  '--security-opt no-new-privileges:true' '--user 10001:10001' \
  'destination=/data/raw,readonly' 'destination=/data/artifacts,readonly' \
  'owner-equity-v2-check'; do
  grep -Fq -- "$expected" "$verify" || die "verifier command boundary missing: $expected"
done
if grep -Eiq 'KIS_APP_KEY=|KIS_APP_SECRET=|DB_PASSWORD=|DATABASE_URL|curl|wget' "$verify"; then
  die 'verifier source contains a direct credential/provider channel'
fi

# Run the real wrapper against a throw-away installed-release fixture and a
# fake Docker binary. Fakeroot supplies the root/10001 metadata contract
# without changing host ownership. The fixture additionally supplies the
# trusted parent metadata for its private release root: a real fixture must
# live below /tmp here, while production deliberately rejects /tmp (1777).
# The fake binary accepts only the expected image-inspect and networkless
# read-only check command; it never starts a container.
command -v fakeroot >/dev/null 2>&1 ||
  die 'TEST_ENVIRONMENT_ERROR: V2 fake-runtime fixture requires fakeroot'
fake_root=$tmp/lagrange
fake_bin=$tmp/fake-docker-bin
fake_log=$tmp/fake-docker.log
fake_run_marker=$tmp/fake-docker-run.marker
mkdir -p "$fake_bin"
cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
log=${OWNER_EQUITY_V2_FAKE_DOCKER_LOG:?}
marker=${OWNER_EQUITY_V2_FAKE_DOCKER_RUN_MARKER:?}
image_id=${OWNER_EQUITY_V2_FAKE_IMAGE_ID:?}
commit=${OWNER_EQUITY_V2_FAKE_COMMIT:?}
printf '%s\n' "$*" >>"$log"
if [ "${1:-}" = image ] && [ "${2:-}" = inspect ]; then
  [ "${!#}" = "$image_id" ] || exit 91
  printf '%s|%s\n' "$image_id" "$commit"
  exit 0
fi
if [ "${1:-}" = run ]; then
  command_line=" $* "
  for required in '--pull=never' '--read-only' '--network none' '--cap-drop ALL' \
    '--security-opt no-new-privileges:true' '--user 10001:10001' \
    'destination=/data/raw,readonly' 'destination=/data/artifacts,readonly' \
    '/usr/local/bin/owner-equity-v2-check'; do
    case "$command_line" in *" $required "*|*"$required"*) ;; *) exit 92 ;; esac
  done
  case "$command_line" in
    *KIS_APP_KEY*|*KIS_APP_SECRET*|*DB_PASSWORD*|*DATABASE_URL*) exit 93 ;;
  esac
  : >"$marker"
  exit 0
fi
exit 94
EOF
chmod 0755 "$fake_bin/docker"
cat >"$fake_bin/stat" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
# The fake installed release is necessarily below /tmp, whose real mode is
# 1777.  Model only that parent as the root-owned, non-writable directory an
# installed release would have; delegate every other metadata check to the
# fakeroot-backed host stat implementation.
if [ "$#" -eq 4 ] && [ "$1" = -c ] && [ "$2" = '%u:%a' ] &&
  [ "$3" = -- ] && [ "$4" = /tmp ]; then
  printf '0:755\n'
  exit 0
fi
exec /usr/bin/stat "$@"
EOF
chmod 0755 "$fake_bin/stat"

fake_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
fake_image_id=sha256:1111111111111111111111111111111111111111111111111111111111111111
fake_fixture_out=$tmp/fake-runtime.out
if ! fakeroot bash -c '
  set -euo pipefail
  fake_root=$1
  source_root=$2
  fake_commit=$3
  fake_image_id=$4
  fake_bin=$5
  fake_log=$6
  fake_marker=$7
  release_dir=$fake_root/releases/$fake_commit
  data_root=$fake_root/data
  mkdir -p "$release_dir/scripts/ops/lib" "$release_dir/deploy/compose" \
    "$data_root/raw" "$data_root/owner-equity-v2-artifacts"
  # The development umask can create directories as 0775.  The verifier is
  # intentionally stricter for an installed release and its data-root parent.
  chmod 0755 "$fake_root" "$fake_root/releases" "$release_dir" \
    "$release_dir/scripts" "$release_dir/scripts/ops" \
    "$release_dir/scripts/ops/lib" "$release_dir/deploy" \
    "$release_dir/deploy/compose" "$data_root"
  cp "$source_root/scripts/ops/owner-equity-v2-verify.sh" "$release_dir/scripts/ops/"
  cp "$source_root/scripts/ops/lib/dotenv.sh" "$release_dir/scripts/ops/lib/"
  cp "$source_root/scripts/ops/lib/release-image-manifest.sh" "$release_dir/scripts/ops/lib/"
  printf "%s\n" "LAGRANGE_DATA_DIR=$data_root" "LAGRANGE_CODE_COMMIT=$fake_commit" \
    >"$release_dir/deploy/compose/.env"
  printf "%s\n" services: {} >"$release_dir/deploy/compose/compose.yml"
  chmod 0600 "$release_dir/deploy/compose/.env"
  ln -s "releases/$fake_commit" "$fake_root/current"
  chown 10001:10001 "$data_root/raw" "$data_root/owner-equity-v2-artifacts" || true
  chmod 0750 "$data_root/raw" "$data_root/owner-equity-v2-artifacts"
  printf identity >"$data_root/owner-equity-v2-artifacts/identity.json"
  printf candidate >"$data_root/owner-equity-v2-artifacts/candidate.json"
  chown 10001:10001 "$data_root/owner-equity-v2-artifacts/identity.json" \
    "$data_root/owner-equity-v2-artifacts/candidate.json" || true
  chmod 0600 "$data_root/owner-equity-v2-artifacts/identity.json" \
    "$data_root/owner-equity-v2-artifacts/candidate.json"
  source "$release_dir/scripts/ops/lib/release-image-manifest.sh"
  release_image_manifest_reset
  index=0
  for service in db-role-bootstrap db-migrate api-server web research-worker \
    recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner \
    nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
    index=$((index + 1))
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=$(release_image_manifest_ref_for "$service" "$fake_commit")
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=$(printf "sha256:%064d" "$index")
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$fake_commit
  done
  RELEASE_IMAGE_MANIFEST_IDS[research-worker]=$fake_image_id
  release_image_manifest_write "$release_dir/.lagrange-release-manifest" "$fake_commit"
  chmod 0600 "$release_dir/.lagrange-release-manifest"
  PATH="$fake_bin:$PATH" \
    OWNER_EQUITY_V2_FAKE_DOCKER_LOG="$fake_log" \
    OWNER_EQUITY_V2_FAKE_DOCKER_RUN_MARKER="$fake_marker" \
    OWNER_EQUITY_V2_FAKE_IMAGE_ID="$fake_image_id" \
    OWNER_EQUITY_V2_FAKE_COMMIT="$fake_commit" \
    LAGRANGE_RELEASE_ROOT="$fake_root" \
    bash "$release_dir/scripts/ops/owner-equity-v2-verify.sh" --check \
      --identity-file "$data_root/owner-equity-v2-artifacts/identity.json" \
      --candidate-file "$data_root/owner-equity-v2-artifacts/candidate.json" \
      --materializer-commit "$fake_commit" \
      --candidate-sha256 sha256:2222222222222222222222222222222222222222222222222222222222222222
' _ "$fake_root" "$root" "$fake_commit" "$fake_image_id" "$fake_bin" "$fake_log" \
  "$fake_run_marker" >"$fake_fixture_out" 2>&1; then
  sed -n '1,80p' "$fake_fixture_out" >&2
  die 'fake installed-release fixture setup failed'
fi
grep -Fq 'OWNER_EQUITY_V2_VERIFY status=ok mode=check' "$fake_fixture_out" ||
  die 'fake verifier did not return sanitized success'
[ -e "$fake_run_marker" ] || die 'fake verifier did not exercise the check command'
grep -Fq -- '--network none' "$fake_log" || die 'fake verifier did not enforce network none'
grep -Fq -- 'destination=/data/raw,readonly' "$fake_log" ||
  die 'fake verifier did not enforce Raw read-only mount'
grep -Fq -- 'destination=/data/artifacts,readonly' "$fake_log" ||
  die 'fake verifier did not enforce artifact read-only mount'
if grep -Eiq 'KIS_APP_KEY|KIS_APP_SECRET|DB_PASSWORD|DATABASE_URL|CANO|ACNT_PRDT_CD|KIS_ACCOUNT_REF' "$fake_log"; then
  die 'fake verifier command exposed a forbidden environment/account channel'
fi

echo 'OWNER_EQUITY_V2_RUNTIME_SELF_TEST: PASS'
