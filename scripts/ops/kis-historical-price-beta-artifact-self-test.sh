#!/usr/bin/env bash
# Fake-Docker/no-infrastructure coverage for the installed historical artifact
# seam.  It exercises a throw-away release/data tree only; it never starts a
# container, reads a protected production file, or contacts a provider/DB.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source_wrapper=$script_dir/kis-historical-price-beta-artifact.sh
source_manifest=$script_dir/lib/release-image-manifest.sh
source_dotenv=$script_dir/lib/dotenv.sh

if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_HISTORICAL_ARTIFACT_ROOT_FIXTURE_CHILD:-0}" = 1 ]; then
    echo 'TEST_ENVIRONMENT_ERROR: historical artifact root fixture did not obtain root identity' >&2
    exit 1
  fi
  if command -v fakeroot >/dev/null 2>&1; then
    exec fakeroot env LAGRANGE_HISTORICAL_ARTIFACT_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/kis-historical-price-beta-artifact-self-test.sh" "$@"
  fi
  echo 'TEST_ENVIRONMENT_ERROR: historical artifact root fixture requires fakeroot' >&2
  exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-historical-artifact.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

install_root=$tmp/install
commit=0123456789abcdef0123456789abcdef01234567
release_dir=$install_root/releases/$commit
data_root=$tmp/data
raw_root=$data_root/raw
curated_root=$data_root/curated
artifacts_root=$data_root/artifacts
artifact_root=$artifacts_root/historical-price-beta-root
fake_bin=$tmp/fake-bin
docker_log=$tmp/docker.log
mkdir -p "$release_dir/scripts/ops/lib" "$release_dir/deploy/compose" \
  "$raw_root" "$curated_root" "$artifact_root" "$fake_bin"
mkdir -p "$install_root"
ln -s "releases/$commit" "$install_root/current"

cp -- "$source_wrapper" "$release_dir/scripts/ops/kis-historical-price-beta-artifact.sh"
cp -- "$source_manifest" "$release_dir/scripts/ops/lib/release-image-manifest.sh"
cp -- "$source_dotenv" "$release_dir/scripts/ops/lib/dotenv.sh"
chmod 0755 "$release_dir/scripts/ops/kis-historical-price-beta-artifact.sh"

image_id=sha256:1111111111111111111111111111111111111111111111111111111111111111
stage5_hash=sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4
action_hash=sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd
candidate_hash=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050
artifact_hash=sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67
ignored_dividend_rows_hash=sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48
approval_registry_hash=sha256:4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380
{
  printf 'LAGRANGE_DATA_DIR=%s\n' "$data_root"
  printf 'LAGRANGE_ARTIFACTS_DIR=%s\n' "$artifacts_root"
  printf 'LAGRANGE_CODE_COMMIT=%s\n' "$commit"
} >"$release_dir/deploy/compose/.env"

{
  printf '%s\n' LAGRANGE_RELEASE_MANIFEST_V2
  printf 'commit|%s\n' "$commit"
  index=0
  for service in db-role-bootstrap db-migrate api-server web research-worker \
    recommendation-runner candidate-runner owner-beta-runner nt-backtest-worker-1 \
    nt-backtest-worker-2 paper-scheduler; do
    index=$((index + 1))
    current_id=$image_id
    [ "$service" = research-worker ] ||
      current_id=$(printf 'sha256:%064d' "$index")
    printf 'image|%s|lagrange-station-%s:%s|%s|%s\n' \
      "$service" "$service" "$commit" "$current_id" "$commit"
  done
} >"$release_dir/.lagrange-release-manifest"

# The fixture files are intentionally not host-root-owned under /tmp.  This
# narrow stat shim models the production root-owned release fence and the
# worker-owned Raw/artifact leaves while preserving real device/inode values.
real_stat=$(command -v stat)
real_id=$(command -v id)
cat >"$fake_bin/id" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [ -f "${HISTORICAL_ARTIFACT_NONROOT_FILE:-}" ] && [ "${1:-}" = -u ]; then
  printf '1000\n'
  exit 0
fi
exec "${HISTORICAL_ARTIFACT_REAL_ID:?}" "$@"
SH
chmod 0755 "$fake_bin/id"

cat >"$fake_bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
format=
previous=
for argument in "$@"; do
  if [ "$previous" = -c ]; then
    format=$argument
    previous=
    continue
  fi
  previous=$argument
done
target=${!#}
if [[ "$target" == "${HISTORICAL_ARTIFACT_FIXTURE_ROOT:?}"* || "$target" = /tmp ]]; then
  case "$format" in
    %d:%i) exec "${HISTORICAL_ARTIFACT_REAL_STAT:?}" "$@" ;;
    %u:%g:%a)
      case "$target" in
        */raw|*/curated|*/historical-price-beta-root)
          if [ "${HISTORICAL_ARTIFACT_BAD_OWNER:-0}" = 1 ] &&
             [[ "$target" == */historical-price-beta-root ]]; then
            printf '10001:10001:755\n'
          else
            printf '10001:10001:750\n'
          fi
          ;;
        */.env|*/.lagrange-release-manifest) printf '0:0:600\n' ;;
        *) printf '0:0:755\n' ;;
      esac
      ;;
    %u:%a)
      case "$target" in
        */.env|*/.lagrange-release-manifest) printf '0:600\n' ;;
        *) printf '0:755\n' ;;
      esac
      ;;
    *) exec "${HISTORICAL_ARTIFACT_REAL_STAT:?}" "$@" ;;
  esac
else
  exec "${HISTORICAL_ARTIFACT_REAL_STAT:?}" "$@"
fi
SH
chmod 0755 "$fake_bin/stat"

cat >"$fake_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${HISTORICAL_ARTIFACT_DOCKER_LOG:?}"
case "${1:-}" in
  image)
    [ "${2:-}" = inspect ] || exit 97
    expected_id=${HISTORICAL_ARTIFACT_IMAGE_ID:?}
    [ "${!#}" = "$expected_id" ] || exit 98
    printf '%s|%s\n' "$expected_id" "${HISTORICAL_ARTIFACT_COMMIT:?}"
    ;;
  run)
    for argument in "$@"; do
      case "$argument" in
        compose|build|up|start|restart|--env-file|-e|--approval-registry)
          echo "forbidden docker argument" >&2
          exit 99
          ;;
      esac
    done
    if printf '%s\n' "$*" | grep -Fq -- '/usr/local/bin/kis-historical-price-beta-approval-check'; then
      if [ -f "${HISTORICAL_ARTIFACT_BAD_APPROVAL_OUTPUT_FILE:-}" ]; then
        printf '%s\n' 'approval-secret-sentinel should be discarded by wrapper'
        printf '%s\n' 'HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count=11 session_count=1608 bar_count=17688'
      else
        printf '%s\n' 'approval-secret-sentinel should be discarded by wrapper'
        printf '%s\n' 'HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256=sha256:4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380 approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count=11 session_count=1608 bar_count=17688'
      fi
    elif printf '%s\n' "$*" | grep -Fq -- 'candidate-content-sha256'; then
      artifact_variant=
      if [ -f "${HISTORICAL_ARTIFACT_BAD_ARTIFACT_OUTPUT_FILE:-}" ]; then
        artifact_variant=$(<"${HISTORICAL_ARTIFACT_BAD_ARTIFACT_OUTPUT_FILE}")
      fi
      artifact_line='HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050 artifact_manifest_sha256=sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67 instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=1 ignored_cash_dividend_rows_sha256=sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48 raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED'
      case "$artifact_variant" in
        '') ;;
        old-v1)
          artifact_line='HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050 instrument_count=11 session_count=1608 bar_count=17688 raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED'
          ;;
        artifact-manifest)
          artifact_line=${artifact_line/artifact_manifest_sha256=sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67/artifact_manifest_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee}
          ;;
        treatment)
          artifact_line=${artifact_line/cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1/cash_dividend_treatment=DIVIDENDS_INCLUDED}
          ;;
        count)
          artifact_line=${artifact_line/ignored_cash_dividends=1/ignored_cash_dividends=2}
          ;;
        rows-hash)
          artifact_line=${artifact_line/ignored_cash_dividend_rows_sha256=sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48/ignored_cash_dividend_rows_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee}
          ;;
        candidate)
          artifact_line=${artifact_line/candidate_content_sha256=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050/candidate_content_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee}
          ;;
        *)
          echo 'unknown artifact output variant' >&2
          exit 95
          ;;
      esac
      [ -z "$artifact_variant" ] || printf '%s\n' 'artifact-cli-sensitive-sentinel should be discarded by wrapper'
      printf '%s\n' "$artifact_line"
    else
      artifact_variant=
      if [ -f "${HISTORICAL_ARTIFACT_BAD_ARTIFACT_OUTPUT_FILE:-}" ]; then
        artifact_variant=$(<"${HISTORICAL_ARTIFACT_BAD_ARTIFACT_OUTPUT_FILE}")
      fi
      artifact_line='HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=materialize candidate_content_sha256=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050 artifact_manifest_sha256=sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67 stage5_manifest_sha256=sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4 action_manifest_sha256=sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=1 ignored_cash_dividend_rows_sha256=sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48 raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED'
      case "$artifact_variant" in
        '') ;;
        old-v1)
          artifact_line='HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=materialize candidate_content_sha256=sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050 stage5_manifest_sha256=sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4 action_manifest_sha256=sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd instrument_count=11 session_count=1608 bar_count=17688 raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED'
          ;;
        artifact-manifest)
          artifact_line=${artifact_line/artifact_manifest_sha256=sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67/artifact_manifest_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee}
          ;;
        treatment)
          artifact_line=${artifact_line/cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1/cash_dividend_treatment=DIVIDENDS_INCLUDED}
          ;;
        count)
          artifact_line=${artifact_line/ignored_cash_dividends=1/ignored_cash_dividends=2}
          ;;
        rows-hash)
          artifact_line=${artifact_line/ignored_cash_dividend_rows_sha256=sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48/ignored_cash_dividend_rows_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee}
          ;;
        source-stage5)
          artifact_line=${artifact_line/stage5_manifest_sha256=sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4/stage5_manifest_sha256=sha256:9999999999999999999999999999999999999999999999999999999999999999}
          ;;
        source-action)
          artifact_line=${artifact_line/action_manifest_sha256=sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd/action_manifest_sha256=sha256:8888888888888888888888888888888888888888888888888888888888888888}
          ;;
        *)
          echo 'unknown artifact output variant' >&2
          exit 95
          ;;
      esac
      [ -z "$artifact_variant" ] || printf '%s\n' 'artifact-cli-sensitive-sentinel should be discarded by wrapper'
      printf '%s\n' "$artifact_line"
    fi
    ;;
  *) exit 96 ;;
esac
SH
chmod 0755 "$fake_bin/docker"

write_manifest_override() {
  local replacement=$1
  sed "s|$image_id|$replacement|" \
    "$release_dir/.lagrange-release-manifest" >"$tmp/manifest.tmp"
  mv -- "$tmp/manifest.tmp" "$release_dir/.lagrange-release-manifest"
}

run_wrapper() {
  env -u LAGRANGE_CODE_COMMIT -u LAGRANGE_DATA_DIR -u LAGRANGE_ARTIFACTS_DIR \
    PATH="$fake_bin:$PATH" \
    HISTORICAL_ARTIFACT_NONROOT_FILE="$tmp/nonroot" \
    HISTORICAL_ARTIFACT_FIXTURE_ROOT="$tmp" \
    HISTORICAL_ARTIFACT_REAL_STAT="$real_stat" \
    HISTORICAL_ARTIFACT_REAL_ID="$real_id" \
    HISTORICAL_ARTIFACT_DOCKER_LOG="$docker_log" \
    HISTORICAL_ARTIFACT_BAD_APPROVAL_OUTPUT_FILE="$tmp/bad-approval-output" \
    HISTORICAL_ARTIFACT_BAD_ARTIFACT_OUTPUT_FILE="$tmp/bad-artifact-output" \
    HISTORICAL_ARTIFACT_IMAGE_ID="$image_id" \
    HISTORICAL_ARTIFACT_COMMIT="$commit" \
    LAGRANGE_RELEASE_ROOT="$install_root" \
    bash "$release_dir/scripts/ops/kis-historical-price-beta-artifact.sh" "$@"
}

expect_artifact_output_rejected() {
  local variant=$1 operation=$2 output
  printf '%s\n' "$variant" >"$tmp/bad-artifact-output"
  : >"$docker_log"
  if [ "$operation" = materialize ]; then
    if output=$(run_wrapper --materialize \
      --stage5-manifest-sha256 "$stage5_hash" \
      --action-manifest-sha256 "$action_hash" 2>&1); then
      echo "historical artifact self-test: $variant materialize output unexpectedly accepted" >&2
      exit 1
    fi
  else
    if output=$(run_wrapper --check \
      --candidate-content-sha256 "$candidate_hash" 2>&1); then
      echo "historical artifact self-test: $variant check output unexpectedly accepted" >&2
      exit 1
    fi
  fi
  [ -s "$docker_log" ]
  ! grep -Fq artifact-cli-sensitive-sentinel <<<"$output"
  rm -f -- "$tmp/bad-artifact-output"
}

plan=$(run_wrapper --plan)
grep -Fq 'PLAN_ONLY: no protected env/manifest read' <<<"$plan"
[ ! -e "$docker_log" ]

if run_wrapper --materialize \
  --stage5-manifest-sha256 sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --action-manifest-sha256 sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  --extra >/dev/null 2>&1; then
  echo 'historical artifact self-test: extra option unexpectedly accepted' >&2
  exit 1
fi
[ ! -e "$docker_log" ]

if run_wrapper --materialize \
  --stage5-manifest-sha256 sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --stage5-manifest-sha256 sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --action-manifest-sha256 sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: duplicate option unexpectedly accepted' >&2
  exit 1
fi
[ ! -e "$docker_log" ]

if run_wrapper --approval-check \
  --candidate-content-sha256 sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: approval-check candidate option unexpectedly accepted' >&2
  exit 1
fi
[ ! -e "$docker_log" ]

if run_wrapper --approval-check --extra >/dev/null 2>&1; then
  echo 'historical artifact self-test: approval-check extra option unexpectedly accepted' >&2
  exit 1
fi
[ ! -e "$docker_log" ]

if run_wrapper --approval-check --approval-registry /tmp/caller-supplied-registry >/dev/null 2>&1; then
  echo 'historical artifact self-test: caller registry path unexpectedly accepted' >&2
  exit 1
fi
[ ! -e "$docker_log" ]

preflight=$(run_wrapper --preflight)
grep -Fq 'HISTORICAL_PRICE_BETA_OPS status=ok mode=preflight' <<<"$preflight"
! grep -Fq 'run ' "$docker_log"

# Root failure must be rejected before protected-input loading or Docker image
# inspection.  The fixture's id shim models an unprivileged operator without
# changing the identity of this test process.
: >"$docker_log"
: >"$tmp/nonroot"
if run_wrapper --preflight >/dev/null 2>&1; then
  echo 'historical artifact self-test: non-root preflight unexpectedly passed' >&2
  exit 1
fi
rm -f -- "$tmp/nonroot"
[ ! -s "$docker_log" ]

materialize=$(run_wrapper --materialize \
  --stage5-manifest-sha256 "$stage5_hash" \
  --action-manifest-sha256 "$action_hash")
expected_materialize="HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=materialize candidate_content_sha256=$candidate_hash artifact_manifest_sha256=$artifact_hash stage5_manifest_sha256=$stage5_hash action_manifest_sha256=$action_hash instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=1 ignored_cash_dividend_rows_sha256=$ignored_dividend_rows_hash raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED"
[ "$materialize" = "$expected_materialize" ]
! grep -Fq secret-sentinel <<<"$materialize"
! grep -Fq artifact-cli-sensitive-sentinel <<<"$materialize"
run_line=$(grep -F 'run ' "$docker_log" | tail -n1)
grep -Fq -- '--network none' <<<"$run_line"
grep -Fq -- '--cap-drop ALL' <<<"$run_line"
grep -Fq -- '--security-opt no-new-privileges:true' <<<"$run_line"
grep -Fq -- '--read-only' <<<"$run_line"
grep -Fq -- '--user 10001:10001' <<<"$run_line"
grep -Fq 'destination=/data/raw,readonly' <<<"$run_line"
grep -Fq 'destination=/artifact-root' <<<"$run_line"
! grep -Fq '/data/curated' <<<"$run_line"
! grep -Eiq 'kis_app|kis_secret|db_password|DATABASE_URL|--env-file| -e ' <<<"$run_line"
grep -Fq -- '/usr/local/bin/kis-historical-price-beta-artifact' <<<"$run_line"
grep -Fq -- "$image_id" <<<"$run_line"

check=$(run_wrapper --check \
  --candidate-content-sha256 "$candidate_hash")
expected_check="HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256=$candidate_hash artifact_manifest_sha256=$artifact_hash instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=1 ignored_cash_dividend_rows_sha256=$ignored_dividend_rows_hash raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED"
[ "$check" = "$expected_check" ]
! grep -Fq artifact-cli-sensitive-sentinel <<<"$check"
check_line=$(grep -F 'run ' "$docker_log" | tail -n1)
! grep -Fq '/data/raw' <<<"$check_line"
grep -Fq 'destination=/artifact-root,readonly' <<<"$check_line"
grep -Fq -- '--network none' <<<"$check_line"

# The production seam must reject the legacy v1 line and every v2 field that
# is malformed or does not bind to the operation's expected pins.
expect_artifact_output_rejected old-v1 materialize
expect_artifact_output_rejected artifact-manifest materialize
expect_artifact_output_rejected treatment materialize
expect_artifact_output_rejected count materialize
expect_artifact_output_rejected rows-hash materialize
expect_artifact_output_rejected source-stage5 materialize
expect_artifact_output_rejected source-action materialize
expect_artifact_output_rejected old-v1 check
expect_artifact_output_rejected artifact-manifest check
expect_artifact_output_rejected treatment check
expect_artifact_output_rejected count check
expect_artifact_output_rejected rows-hash check
expect_artifact_output_rejected candidate check

approval=$(run_wrapper --approval-check)
grep -Fq 'HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check' <<<"$approval"
! grep -Fq 'candidate_content_sha256=' <<<"$approval"
! grep -Fq 'artifact_manifest_sha256=' <<<"$approval"
! grep -Fq 'stage5_manifest_sha256=' <<<"$approval"
! grep -Fq 'action_manifest_sha256=' <<<"$approval"
! grep -Fq "$candidate_hash" <<<"$approval"
grep -Fq "approval_registry_sha256=$approval_registry_hash" <<<"$approval"
grep -Fq 'approval_status=APPROVED' <<<"$approval"
! grep -Fq approval-secret-sentinel <<<"$approval"
approval_line=$(grep -F 'run ' "$docker_log" | tail -n1)
grep -Fq -- '--network none' <<<"$approval_line"
grep -Fq -- '--cap-drop ALL' <<<"$approval_line"
grep -Fq -- '--security-opt no-new-privileges:true' <<<"$approval_line"
grep -Fq -- '--read-only' <<<"$approval_line"
grep -Fq -- '--user 10001:10001' <<<"$approval_line"
grep -Fq -- '--entrypoint /usr/local/bin/kis-historical-price-beta-approval-check' <<<"$approval_line"
! grep -Fq '/data/raw' <<<"$approval_line"
! grep -Fq '/data/curated' <<<"$approval_line"
grep -Fq 'destination=/artifact-root,readonly' <<<"$approval_line"
! grep -Fq -- '--approval-registry' <<<"$approval_line"
! grep -Fq -- '--candidate-content-sha256' <<<"$approval_line"
! grep -Fq "$candidate_hash" <<<"$approval_line"
grep -Fq -- "$image_id" <<<"$approval_line"

# A well-formed but different approval-registry hash is discarded and causes a
# static failure without changing the dedicated artifact leaf.
approval_snapshot=$(find "$artifact_root" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
: >"$tmp/bad-approval-output"
: >"$docker_log"
if run_wrapper --approval-check >/dev/null 2>&1; then
  echo 'historical artifact self-test: malformed approval output unexpectedly passed' >&2
  exit 1
fi
[ "$approval_snapshot" = "$(find "$artifact_root" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" ]
rm -f -- "$tmp/bad-approval-output"

# Manifest image/revision mismatch must fail before a container run.
: >"$docker_log"
write_manifest_override sha256:2222222222222222222222222222222222222222222222222222222222222222
if run_wrapper --check \
  --candidate-content-sha256 "$candidate_hash" \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: image mismatch unexpectedly passed' >&2
  exit 1
fi
! grep -Fq 'run ' "$docker_log"

write_manifest_revision_override() {
  local replacement=$1
  awk -F'|' -v OFS='|' -v revision="$replacement" '
    $1 == "image" && $2 == "research-worker" { $5 = revision }
    { print }
  ' "$release_dir/.lagrange-release-manifest" >"$tmp/manifest.tmp"
  mv -- "$tmp/manifest.tmp" "$release_dir/.lagrange-release-manifest"
}

write_manifest_revision_override fedcba9876543210fedcba9876543210fedcba98
: >"$docker_log"
if run_wrapper --check \
  --candidate-content-sha256 "$candidate_hash" \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: revision mismatch unexpectedly passed' >&2
  exit 1
fi
! grep -Fq 'run ' "$docker_log"

# Restore the manifest and make the dedicated leaf unsafe.  Ownership failure
# must happen before image inspection and therefore before any Docker run.
write_manifest_override "$image_id"
write_manifest_revision_override "$commit"
: >"$docker_log"
if HISTORICAL_ARTIFACT_BAD_OWNER=1 run_wrapper --materialize \
  --stage5-manifest-sha256 "$stage5_hash" \
  --action-manifest-sha256 "$action_hash" \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: ownership failure unexpectedly passed' >&2
  exit 1
fi
[ ! -s "$docker_log" ]

# A host artifact leaf below Raw must fail the independent host-canonical
# separation gate before image inspection or a container run.
cp -- "$release_dir/deploy/compose/.env" "$tmp/env.backup"
mkdir -p "$raw_root/historical-price-beta-root"
sed "s|LAGRANGE_ARTIFACTS_DIR=.*|LAGRANGE_ARTIFACTS_DIR=$raw_root|" \
  "$tmp/env.backup" >"$release_dir/deploy/compose/.env"
: >"$docker_log"
if run_wrapper --materialize \
  --stage5-manifest-sha256 "$stage5_hash" \
  --action-manifest-sha256 "$action_hash" \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: Raw/Curated separation failure unexpectedly passed' >&2
  exit 1
fi
[ ! -s "$docker_log" ]
mv -- "$tmp/env.backup" "$release_dir/deploy/compose/.env"

# Docker's --mount grammar uses commas as field delimiters.  Reject a host
# path containing one before image inspection so it cannot inject mount
# options into the otherwise fixed argument vector.
cp -- "$release_dir/deploy/compose/.env" "$tmp/env.backup"
sed "s|LAGRANGE_ARTIFACTS_DIR=.*|LAGRANGE_ARTIFACTS_DIR=$data_root/artifacts,unsafe|" \
  "$tmp/env.backup" >"$release_dir/deploy/compose/.env"
: >"$docker_log"
if run_wrapper --check \
  --candidate-content-sha256 "$candidate_hash" \
  >/dev/null 2>&1; then
  echo 'historical artifact self-test: comma mount path unexpectedly passed' >&2
  exit 1
fi
[ ! -s "$docker_log" ]
mv -- "$tmp/env.backup" "$release_dir/deploy/compose/.env"

echo 'HISTORICAL_PRICE_BETA_ARTIFACT_SELF_TEST: PASS'
