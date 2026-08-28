#!/usr/bin/env bash
# Provider-free fixture coverage for immutable release install/start contracts
# and encrypted backups. It never talks to a Docker daemon, DB, systemd, or a
# provider; every Docker/system command below is a fixture binary.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
ops=$root/scripts/ops

# Establish one root-like process for every root-only fixture. Inside it, run
# fixture commands directly; do not nest fakeroot around individual commands.
# A runner absence is an explicit test-environment failure, never PASS-on-skip.
if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_PRODUCTION_OPS_ROOT_FIXTURE_CHILD:-0}" = 1 ]; then
    echo 'TEST_ENVIRONMENT_ERROR: production-ops root fixture did not obtain root identity' >&2
    exit 1
  fi
  if unshare -Ur true >/dev/null 2>&1; then
    exec unshare -Ur env LAGRANGE_PRODUCTION_OPS_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/production-ops-self-test.sh" "$@"
  fi
  if command -v fakeroot >/dev/null 2>&1; then
    exec fakeroot env LAGRANGE_PRODUCTION_OPS_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/production-ops-self-test.sh" "$@"
  fi
  echo 'TEST_ENVIRONMENT_ERROR: production-ops root fixture requires user namespaces or fakeroot' >&2
  exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-production-ops.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

bash "$ops/production-ops-static-check.sh" >/dev/null

plan=$(bash "$ops/deploy-production-release.sh" --dry-run \
  --commit 1111111111111111111111111111111111111111)
grep -Fq 'DRY_RUN: no Git archive' <<<"$plan"
backup_plan=$(bash "$ops/run-production-backup.sh" --plan)
grep -Fq 'PLAN_ONLY: no protected config/key read' <<<"$backup_plan"
install_plan=$(bash "$ops/install-production-backup.sh" --dry-run)
grep -Fq 'never starts them' <<<"$install_plan"

# fakeroot cannot change the host /tmp mode. This fixture-only stat shim models
# the root-owned non-writable test hierarchy while leaving the host untouched.
# It is applied solely to immutable-release fixture subprocesses.
trust_bin=$tmp/trust-bin
mkdir -p "$trust_bin"
real_stat=$(command -v stat)
real_install=$(command -v install)
real_tar=$(command -v tar)
cat >"$trust_bin/stat" <<'SH'
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
case "$target" in
  /|/tmp|"${FAKE_TRUST_ROOT:?}"|"${FAKE_TRUST_ROOT:?}"/*) ;;
  *) exec "${REAL_STAT_BIN:?}" "$@" ;;
esac
mode=755
case "$target" in
  "${FAKE_UNTRUSTED_MANIFEST:-/not-a-fixture-path}") mode=644 ;;
  */.lagrange-release|*/.lagrange-release-manifest|*/.env|*/production*.env|*/image-manifest-*|*/.release-image-override.*)
    mode=600
    ;;
esac
case "$format" in
  %u:%g:%a) printf '0:0:%s\n' "$mode" ;;
  %u:%a) printf '0:%s\n' "$mode" ;;
  %a) printf '%s\n' "$mode" ;;
  *) exec "${REAL_STAT_BIN:?}" "$@" ;;
esac
SH
chmod 0755 "$trust_bin/stat"
cat >"$trust_bin/install" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
filtered=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|-g|--owner|--group) [ "$#" -ge 2 ] || exit 97; shift 2 ;;
    --owner=*|--group=*) shift ;;
    *) filtered+=("$1"); shift ;;
  esac
done
exec "${REAL_INSTALL_BIN:?}" "${filtered[@]}"
SH
cat >"$trust_bin/chown" <<'SH'
#!/usr/bin/env bash
# fakeroot cannot always map chown(2) in restricted CI filesystems. The release
# fixture's stat shim supplies the simulated root metadata, so this is a narrow
# no-op for its one staging ownership normalization call.
set -euo pipefail
exit 0
SH
chmod 0755 "$trust_bin/install" "$trust_bin/chown"

# The fixture is a clean Git repository. Protected env/manifest files stay
# outside Git exactly as in production; the known workbook remains the sole
# allowed untracked path and is excluded from the archive.
release_fixture=$tmp/release
mkdir -p "$release_fixture/repo/scripts/ops/lib" "$release_fixture/repo/deploy/compose" \
  "$release_fixture/repo/nt" "$release_fixture/repo/configs" \
  "$release_fixture/repo/migrations" "$release_fixture/install"
chmod 0700 "$release_fixture"
chmod 0755 "$release_fixture/install"
cp "$ops/deploy-production-release.sh" "$release_fixture/repo/scripts/ops/"
cp "$ops/compose-release.sh" "$release_fixture/repo/scripts/ops/"
cp "$ops/lib/release-image-manifest.sh" "$release_fixture/repo/scripts/ops/lib/"
cp "$ops/lib/dotenv.sh" "$release_fixture/repo/scripts/ops/lib/"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
  >"$release_fixture/repo/scripts/ops/validate-production-config.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
  >"$release_fixture/repo/scripts/ops/provision-linux.sh"
cat >"$release_fixture/repo/scripts/ops/kis-historical-price-beta-artifact.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' approval-check >>"${COMPOSE_FAKE_LOG:?}"
printf '%s\n' "$*" >"${OWNER_BETA_FAKE_ARGS:?}"
[ "${OWNER_BETA_FAKE_APPROVAL_FAIL:-0}" = 0 ] || exit 2
if [ "${OWNER_BETA_FAKE_APPROVAL_BAD_OUTPUT:-0}" = 1 ]; then
  printf 'HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_status=APPROVED\n'
  exit 0
fi
printf '%s\n' 'HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count=11 session_count=2452 bar_count=26972'
SH
chmod 0755 "$release_fixture/repo/scripts/ops/"*.sh
printf '%s\n' 'services: {}' >"$release_fixture/repo/deploy/compose/compose.yml"
printf '%s\n' fixture >"$release_fixture/repo/nt/fixture"
printf '%s\n' fixture >"$release_fixture/repo/configs/fixture"
printf '%s\n' fixture >"$release_fixture/repo/migrations/fixture"
git -C "$release_fixture/repo" init -q
git -C "$release_fixture/repo" config user.name test
git -C "$release_fixture/repo" config user.email test@example.invalid
git -C "$release_fixture/repo" add .
git -C "$release_fixture/repo" commit -qm first
commit_one=$(git -C "$release_fixture/repo" rev-parse HEAD)
mkdir -p "$release_fixture/repo/docs"
printf '%s' official-workbook-fixture \
  >"$release_fixture/repo/docs/kis_openapi_entiredocs_20260818_030007.xlsx"
printf 'LAGRANGE_DATA_DIR=/var/lib/lagrange/data\nLAGRANGE_CODE_COMMIT=%s\n' "$commit_one" \
  >"$release_fixture/production.env"
chmod 0600 "$release_fixture/production.env"

write_image_manifest() {
  local path=$1 commit=$2 service index image_id
  {
    printf '%s\n' LAGRANGE_RELEASE_MANIFEST_V2
    printf 'commit|%s\n' "$commit"
    index=0
    for service in db-role-bootstrap db-migrate api-server web research-worker \
      recommendation-runner candidate-runner owner-beta-runner nt-backtest-worker-1 \
      nt-backtest-worker-2 paper-scheduler; do
      index=$((index + 1))
      image_id=$(printf 'sha256:%064d' "$index")
      printf 'image|%s|lagrange-station-%s:%s|%s|%s\n' \
        "$service" "$service" "$commit" "$image_id" "$commit"
    done
  } >"$path"
  chmod 0600 "$path"
}

run_release() {
  env PATH="$trust_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
    REAL_INSTALL_BIN="$real_install" \
    "$@"
}

manifest_one=$release_fixture/image-manifest-one
write_image_manifest "$manifest_one" "$commit_one"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" >"$tmp/missing-manifest.out" 2>&1; then
  echo 'production-ops-self-test: manifest-less apply unexpectedly passed' >&2
  exit 1
fi
grep -Fq -- '--apply requires --release-manifest' "$tmp/missing-manifest.out"

run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" --release-manifest "$manifest_one"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_one" ]
[ "$(stat -c %a "$release_fixture/install/releases/$commit_one/deploy/compose/.env")" = 600 ]
grep -Fxq "LAGRANGE_CODE_COMMIT=$commit_one" \
  "$release_fixture/install/releases/$commit_one/deploy/compose/.env"
[ "$(stat -c %a "$release_fixture/install/releases/$commit_one/.lagrange-release-manifest")" = 600 ]
[ "$(sha256sum "$manifest_one" | awk '{print $1}')" = \
  "$(sha256sum "$release_fixture/install/releases/$commit_one/.lagrange-release-manifest" | awk '{print $1}')" ]
[ ! -e "$release_fixture/install/releases/$commit_one/docs/kis_openapi_entiredocs_20260818_030007.xlsx" ]

run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --check --commit "$commit_one" --install-root "$release_fixture/install" >"$tmp/check.out"
grep -Fq 'PRODUCTION_RELEASE_CHECK: PASS' "$tmp/check.out"

if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --check --commit "$commit_one" --install-root "$release_fixture/install" \
  --release-manifest "$manifest_one" >"$tmp/external-check.out" 2>&1; then
  echo 'production-ops-self-test: check accepted a second manifest unexpectedly' >&2
  exit 1
fi
grep -Fq -- '--release-manifest is allowed only with --apply' "$tmp/external-check.out"

bad_mode_manifest=$release_fixture/image-manifest-bad-mode
cp "$manifest_one" "$bad_mode_manifest"
chmod 0644 "$bad_mode_manifest"
if env PATH="$trust_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
  REAL_INSTALL_BIN="$real_install" \
  FAKE_UNTRUSTED_MANIFEST="$bad_mode_manifest" \
  bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
    --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
    --install-root "$release_fixture/install" --release-manifest "$bad_mode_manifest" \
    >"$tmp/bad-mode.out" 2>&1; then
  echo 'production-ops-self-test: unsafe external manifest unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'release-manifest must be root:root mode 0600' "$tmp/bad-mode.out"

bad_shape_manifest=$release_fixture/image-manifest-bad-shape
cp "$manifest_one" "$bad_shape_manifest"
printf 'image|api-server|lagrange-station-api-server:%s|sha256:%064d|%s\n' \
  "$commit_one" 99 "$commit_one" >>"$bad_shape_manifest"
chmod 0600 "$bad_shape_manifest"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" --release-manifest "$bad_shape_manifest" \
  >"$tmp/bad-shape.out" 2>&1; then
  echo 'production-ops-self-test: duplicate manifest record unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest record count is not canonical' "$tmp/bad-shape.out"

bad_separator_manifest=$release_fixture/image-manifest-bad-separator
cp "$manifest_one" "$bad_separator_manifest"
sed -i '3s/$/|/' "$bad_separator_manifest"
chmod 0600 "$bad_separator_manifest"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" --release-manifest "$bad_separator_manifest" \
  >"$tmp/bad-separator.out" 2>&1; then
  echo 'production-ops-self-test: trailing manifest separator unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest image record has a trailing separator' "$tmp/bad-separator.out"

printf '%s\n' second >"$release_fixture/repo/second"
git -C "$release_fixture/repo" add second
git -C "$release_fixture/repo" commit -qm second
commit_two=$(git -C "$release_fixture/repo" rev-parse HEAD)
printf 'LAGRANGE_DATA_DIR=/var/lib/lagrange/data\nLAGRANGE_CODE_COMMIT=%s\n' "$commit_two" \
  >"$release_fixture/production-two.env"
chmod 0600 "$release_fixture/production-two.env"
manifest_two=$release_fixture/image-manifest-two
write_image_manifest "$manifest_two" "$commit_two"
run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production-two.env" \
  --install-root "$release_fixture/install" --release-manifest "$manifest_two"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_two" ]
run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --rollback --commit "$commit_one" --install-root "$release_fixture/install"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_one" ]

legacy_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
legacy_release=$release_fixture/install/releases/$legacy_commit
mkdir -p "$legacy_release/deploy/compose"
printf '%s\n' "$legacy_commit" >"$legacy_release/.lagrange-release"
printf '%s\n' 'services: {}' >"$legacy_release/deploy/compose/compose.yml"
printf '%s\n' 'LAGRANGE_DATA_DIR=/var/lib/lagrange/data' >"$legacy_release/deploy/compose/.env"
chmod 0600 "$legacy_release/.lagrange-release" "$legacy_release/deploy/compose/.env"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --rollback --commit "$legacy_commit" --install-root "$release_fixture/install" \
  >"$tmp/legacy.out" 2>&1; then
  echo 'production-ops-self-test: legacy manifest-less rollback unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'legacy manifest-less release is blocked' "$tmp/legacy.out"

touch "$release_fixture/repo/untracked-secret"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production-two.env" \
  --install-root "$release_fixture/install" --release-manifest "$manifest_two" \
  >"$tmp/dirty.out" 2>&1; then
  echo 'production-ops-self-test: dirty release source unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'repository must have no tracked changes or unapproved untracked files' "$tmp/dirty.out"
rm "$release_fixture/repo/untracked-secret"
printf '%s\n' tracked-dirty >>"$release_fixture/repo/second"
if run_release bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production-two.env" \
  --install-root "$release_fixture/install" --release-manifest "$manifest_two" \
  >"$tmp/tracked-dirty.out" 2>&1; then
  echo 'production-ops-self-test: tracked-dirty release source unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'repository must have no tracked changes or unapproved untracked files' "$tmp/tracked-dirty.out"
git -C "$release_fixture/repo" checkout -- second

# The compose fixture starts no real container. It captures the generated
# override and emulates local image/container inspection only.
compose_bin=$release_fixture/compose-bin
compose_log=$release_fixture/compose.log
override_capture=$release_fixture/override.yml
owner_beta_args=$release_fixture/owner-beta-args
mkdir -p "$compose_bin"
cat >"$compose_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${COMPOSE_FAKE_LOG:?}"
image_id_for_service() {
  case "$1" in
    db-role-bootstrap) index=1 ;;
    db-migrate) index=2 ;;
    api-server) index=3 ;;
    web) index=4 ;;
    research-worker) index=5 ;;
    recommendation-runner) index=6 ;;
    candidate-runner) index=7 ;;
    owner-beta-runner) index=8 ;;
    nt-backtest-worker-1) index=9 ;;
    nt-backtest-worker-2) index=10 ;;
    paper-scheduler) index=11 ;;
    *) exit 97 ;;
  esac
  printf 'sha256:%064d' "$index"
}
capture_override() {
  local prior= argument
  for argument in "$@"; do
    if [ "$prior" = -f ] && [[ "$argument" == */.release-image-override.* ]]; then
      cp -- "$argument" "${COMPOSE_OVERRIDE_CAPTURE:?}"
      return 0
    fi
    prior=$argument
  done
}
if [ "${1:-}" = compose ]; then
  shift
  capture_override "$@" || true
  command_name=
  for argument in "$@"; do
    case "$argument" in version|config|up|run|ps) command_name=$argument; break ;; esac
  done
  case "$command_name" in
    version|config|up|run) exit 0 ;;
    ps)
      if [[ " $* " == *' -q '* ]]; then
        service=${!#}
        printf 'ctr-%s\n' "$service"
      fi
      exit 0
      ;;
    *) echo 'unexpected fake Compose command' >&2; exit 98 ;;
  esac
fi
if [ "${1:-}" = image ] && [ "${2:-}" = inspect ]; then
  target=${!#}
  actual=${PRODUCTION_FAKE_IMAGE_ID:-$target}
  printf '%s|%s\n' "$actual" "${PRODUCTION_FAKE_COMMIT:?}"
  exit 0
fi
if [ "${1:-}" = inspect ]; then
  target=${!#}
  service=${target#ctr-}
  image_id=$(image_id_for_service "$service")
  if [ "${PRODUCTION_FAKE_CONTAINER_MISMATCH_SERVICE:-}" = "$service" ]; then
    image_id=$(printf 'sha256:%064d' 99)
  fi
  printf '%s|%s\n' "$image_id" "${PRODUCTION_FAKE_COMMIT:?}"
  exit 0
fi
echo 'unexpected fake Docker command' >&2
exit 98
SH
chmod 0755 "$compose_bin/docker"

env PATH="$trust_bin:$compose_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
  REAL_INSTALL_BIN="$real_install" \
  COMPOSE_FAKE_LOG="$compose_log" COMPOSE_OVERRIDE_CAPTURE="$override_capture" \
  PRODUCTION_FAKE_COMMIT="$commit_one" LAGRANGE_RELEASE_ROOT="$release_fixture/install" \
  bash "$release_fixture/install/current/scripts/ops/compose-release.sh" --scope release --apply \
  >"$tmp/compose-pass.out"
grep -Fq 'COMPOSE_RELEASE: PASS' "$tmp/compose-pass.out"
[ "$(grep -c '^    image: sha256:' "$override_capture")" -eq 11 ]
[ "$(grep -c '^    build: !reset null$' "$override_capture")" -eq 11 ]
if grep -Eq '(^| )build( |$)' "$compose_log"; then
  echo 'production-ops-self-test: immutable release tried to rebuild an image' >&2
  exit 1
fi
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner owner-beta-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  grep -Fq -- "image inspect --format {{.Id}}|{{index .Config.Labels \"org.opencontainers.image.revision\"}} sha256:" \
    "$compose_log"
done
for service in api-server web research-worker recommendation-runner candidate-runner \
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
  grep -Fq "inspect --format {{.Image}}|{{index .Config.Labels \"org.opencontainers.image.revision\"}} ctr-$service" \
    "$compose_log"
done
if grep -Fq 'owner-beta-runner' "$compose_log"; then
  echo 'production-ops-self-test: disabled release activated owner-beta-runner' >&2
  exit 1
fi
if grep -Eiq '(^| )(down|stop)( |$)' "$compose_log"; then
  echo 'production-ops-self-test: successful immutable release stopped a service' >&2
  exit 1
fi

# Switch only the protected fixture policy to owner-only. The release and its
# eleven-image manifest remain unchanged; the host approval gate must run before
# the first Compose up, owner-beta-runner must enter the started subset only
# after that gate, and Paper must remain absent.
installed_env=$release_fixture/install/releases/$commit_one/deploy/compose/.env
cp "$installed_env" "$release_fixture/disabled.env.backup"
printf '%s\n' \
  'OWNER_BETA_ACCESS_MODE=owner_only' \
  'OWNER_BETA_PRICE_INPUT_MODE=sealed_v1' \
  'OWNER_BETA_PAPER_MODE=disabled' \
  >>"$installed_env"
: >"$compose_log"
env PATH="$trust_bin:$compose_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
  REAL_INSTALL_BIN="$real_install" \
  COMPOSE_FAKE_LOG="$compose_log" COMPOSE_OVERRIDE_CAPTURE="$override_capture" \
  OWNER_BETA_FAKE_ARGS="$owner_beta_args" \
  PRODUCTION_FAKE_COMMIT="$commit_one" LAGRANGE_RELEASE_ROOT="$release_fixture/install" \
  bash "$release_fixture/install/current/scripts/ops/compose-release.sh" --scope release --apply \
  >"$tmp/compose-owner-beta-pass.out"
grep -Fxq 'OWNER_BETA_RELEASE_GATE: PASS access=owner_only paper=disabled' \
  "$tmp/compose-owner-beta-pass.out"
grep -Fxq -- '--approval-check' "$owner_beta_args"
approval_line=$(grep -n '^approval-check$' "$compose_log" | cut -d: -f1)
first_up_line=$(grep -n 'compose .* up ' "$compose_log" | head -n1 | cut -d: -f1)
[ -n "$approval_line" ] && [ -n "$first_up_line" ] && [ "$approval_line" -lt "$first_up_line" ]
if grep -Fq 'paper-scheduler' "$compose_log"; then
  echo 'production-ops-self-test: owner-only release reached Paper scheduler' >&2
  exit 1
fi
grep -Fq 'up --no-build --no-deps -d research-worker recommendation-runner candidate-runner owner-beta-runner nt-backtest-worker-1 nt-backtest-worker-2' \
  "$compose_log" || {
  echo 'production-ops-self-test: owner-only release did not explicitly start owner-beta-runner' >&2
  exit 1
}
grep -Fq 'inspect --format {{.Image}}|{{index .Config.Labels "org.opencontainers.image.revision"}} ctr-owner-beta-runner' \
  "$compose_log" || {
  echo 'production-ops-self-test: owner-only release did not verify owner-beta-runner image' >&2
  exit 1
}
if grep -Fq 'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' \
   "$owner_beta_args" "$compose_log" "$tmp/compose-owner-beta-pass.out"; then
  echo 'production-ops-self-test: owner-beta candidate escaped the host approval seam' >&2
  exit 1
fi

for failure_mode in fail bad-output; do
  : >"$compose_log"
  extra_env=(OWNER_BETA_FAKE_APPROVAL_FAIL=0 OWNER_BETA_FAKE_APPROVAL_BAD_OUTPUT=0)
  case "$failure_mode" in
    fail) extra_env=(OWNER_BETA_FAKE_APPROVAL_FAIL=1 OWNER_BETA_FAKE_APPROVAL_BAD_OUTPUT=0) ;;
    bad-output) extra_env=(OWNER_BETA_FAKE_APPROVAL_FAIL=0 OWNER_BETA_FAKE_APPROVAL_BAD_OUTPUT=1) ;;
  esac
  if env PATH="$trust_bin:$compose_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
     REAL_INSTALL_BIN="$real_install" \
     COMPOSE_FAKE_LOG="$compose_log" COMPOSE_OVERRIDE_CAPTURE="$override_capture" \
     OWNER_BETA_FAKE_ARGS="$owner_beta_args" "${extra_env[@]}" \
     PRODUCTION_FAKE_COMMIT="$commit_one" LAGRANGE_RELEASE_ROOT="$release_fixture/install" \
     bash "$release_fixture/install/current/scripts/ops/compose-release.sh" --scope release --apply \
     >"$tmp/compose-owner-beta-$failure_mode.out" 2>&1; then
    echo "production-ops-self-test: owner-beta $failure_mode unexpectedly passed" >&2
    exit 1
  fi
  grep -Fq 'owner_beta_artifact_not_approved' "$tmp/compose-owner-beta-$failure_mode.out"
  if grep -Eq 'compose .* up ' "$compose_log"; then
    echo "production-ops-self-test: owner-beta $failure_mode reached Compose up" >&2
    exit 1
  fi
done
cp "$release_fixture/disabled.env.backup" "$installed_env"

: >"$compose_log"
if env PATH="$trust_bin:$compose_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
  REAL_INSTALL_BIN="$real_install" \
  COMPOSE_FAKE_LOG="$compose_log" COMPOSE_OVERRIDE_CAPTURE="$override_capture" \
  PRODUCTION_FAKE_COMMIT="$commit_one" PRODUCTION_FAKE_CONTAINER_MISMATCH_SERVICE=web \
  LAGRANGE_RELEASE_ROOT="$release_fixture/install" \
  bash "$release_fixture/install/current/scripts/ops/compose-release.sh" --scope release --apply \
  >"$tmp/compose-container-mismatch.out" 2>&1; then
  echo 'production-ops-self-test: running-container image mismatch unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'persistent service image_id mismatch: web' "$tmp/compose-container-mismatch.out"
if grep -Eiq '(^| )(down|stop)( |$)' "$compose_log"; then
  echo 'production-ops-self-test: mismatch path automatically stopped or rolled back services' >&2
  exit 1
fi

: >"$compose_log"
if env PATH="$trust_bin:$compose_bin:$PATH" FAKE_TRUST_ROOT="$tmp" REAL_STAT_BIN="$real_stat" \
  REAL_INSTALL_BIN="$real_install" \
  COMPOSE_FAKE_LOG="$compose_log" COMPOSE_OVERRIDE_CAPTURE="$override_capture" \
  PRODUCTION_FAKE_COMMIT="$commit_one" \
  PRODUCTION_FAKE_IMAGE_ID=sha256:9999999999999999999999999999999999999999999999999999999999999999 \
  LAGRANGE_RELEASE_ROOT="$release_fixture/install" \
  bash "$release_fixture/install/current/scripts/ops/compose-release.sh" --scope release --apply \
  >"$tmp/compose-image-mismatch.out" 2>&1; then
  echo 'production-ops-self-test: pre-start image mismatch unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest image_id mismatch: db-role-bootstrap' "$tmp/compose-image-mismatch.out"
if grep -Eq 'compose .* up ' "$compose_log"; then
  echo 'production-ops-self-test: pre-start image mismatch reached startup' >&2
  exit 1
fi

# Preserve the encrypted backup fixture: fake Docker proves the backup scripts
# use isolated commands without a daemon or protected content in output.
backup_fixture=$tmp/backup
ownership_bin=$tmp/ownership-bin
mkdir -p "$ownership_bin"
cp "$trust_bin/install" "$ownership_bin/install"
cp "$trust_bin/chown" "$ownership_bin/chown"
cat >"$ownership_bin/tar" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  case "$argument" in
    -x|-*x*|--extract)
      exec "${REAL_TAR_BIN:?}" --no-same-owner "$@"
      ;;
  esac
done
exec "${REAL_TAR_BIN:?}" "$@"
SH
chmod 0755 "$ownership_bin/tar"
mkdir -p "$backup_fixture"/{bin,data/raw,data/curated,backups,state,locks,compose}
chmod 0700 "$backup_fixture" "$backup_fixture/backups" "$backup_fixture/state" \
  "$backup_fixture/locks" "$backup_fixture/compose"
printf '%s' raw-fixture-secret >"$backup_fixture/data/raw/row"
printf '%s' curated-fixture-secret >"$backup_fixture/data/curated/row"
printf '%s\n' 'services: {}' >"$backup_fixture/compose/compose.yml"
printf '%s\n' 'APP_ENV=production' >"$backup_fixture/compose/.env"
printf '4%.0s' {1..64} >"$backup_fixture/key"
chmod 0600 "$backup_fixture/compose/.env" "$backup_fixture/key"
cat >"$backup_fixture/bin/docker" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$FAKE_DOCKER_LOG"
if [ "${1:-}" = compose ]; then
  case " $* " in
    *' version '*) exit 0 ;;
    *' exec '*) printf '%s' FAKE_CUSTOM_DATABASE_DUMP; exit 0 ;;
  esac
fi
if [ "${1:-}" = run ]; then exit 0; fi
exit 1
SH
chmod 0755 "$backup_fixture/bin/docker"
write_backup_config() {
  local cap=$1
  cat >"$backup_fixture/backup.conf" <<EOF
BACKUP_ROOT=$backup_fixture/backups
DATA_ROOT=$backup_fixture/data
COMPOSE_FILE=$backup_fixture/compose/compose.yml
COMPOSE_ENV_FILE=$backup_fixture/compose/.env
COMPOSE_PROJECT=lagrange-test
LAGRANGE_CODE_COMMIT=2222222222222222222222222222222222222222
KEY_FILE=$backup_fixture/key
LOCK_FILE=$backup_fixture/locks/backup.lock
METRICS_FILE=$backup_fixture/state/backup.prom
MAX_TOTAL_BYTES=$cap
MIN_FREE_BYTES=1
RETENTION_DAYS=365
MIN_KEEP=1
POSTGRES_SERVICE=postgres
POSTGRES_IMAGE=postgres:18.4
EOF
  chmod 0600 "$backup_fixture/backup.conf"
}
write_backup_config 999999999
export FAKE_DOCKER_LOG=$backup_fixture/docker.log

printf '%s' fixture-backup-secret-must-never-leak >"$backup_fixture/key"
if PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  bash "$ops/run-production-backup.sh" --check \
  --config-file "$backup_fixture/backup.conf" >"$backup_fixture/bad-key.out" 2>&1; then
  echo 'production-ops-self-test: malformed backup key unexpectedly passed' >&2
  exit 1
fi
if grep -Fq fixture-backup-secret-must-never-leak "$backup_fixture/bad-key.out"; then
  echo 'production-ops-self-test: malformed key value leaked' >&2
  exit 1
fi
printf '4%.0s' {1..64} >"$backup_fixture/key"

cat >"$backup_fixture/bin/systemctl" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$FAKE_SYSTEMCTL_LOG"
SH
chmod 0755 "$backup_fixture/bin/systemctl"
mkdir -p "$backup_fixture/install-bin" "$backup_fixture/systemd" "$backup_fixture/etc"
chmod 0755 "$backup_fixture/install-bin" "$backup_fixture/systemd" "$backup_fixture/etc"
export FAKE_SYSTEMCTL_LOG=$backup_fixture/systemctl.log
PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  FAKE_SYSTEMCTL_LOG="$FAKE_SYSTEMCTL_LOG" \
  bash "$ops/install-production-backup.sh" --apply \
  --config-source "$backup_fixture/backup.conf" \
  --install-bin "$backup_fixture/install-bin" --systemd-dir "$backup_fixture/systemd" \
  --config-target "$backup_fixture/etc/backup.conf" >"$backup_fixture/install.out"
grep -Fxq daemon-reload "$FAKE_SYSTEMCTL_LOG"
grep -Fq 'enable lagrange-production-backup.timer lagrange-production-backup-verify.timer' \
  "$FAKE_SYSTEMCTL_LOG"
if grep -Eq -- '--now|(^| )start( |$)' "$FAKE_SYSTEMCTL_LOG"; then
  echo 'production-ops-self-test: installer unexpectedly started a unit' >&2
  exit 1
fi
[ ! -s "$FAKE_DOCKER_LOG" ]
if PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  FAKE_SYSTEMCTL_LOG="$FAKE_SYSTEMCTL_LOG" \
  bash "$ops/install-production-backup.sh" --apply \
  --config-source "$backup_fixture/backup.conf" \
  --install-bin "$backup_fixture/install-bin" --systemd-dir "$backup_fixture/systemd" \
  --config-target "$backup_fixture/etc/backup.conf" >"$backup_fixture/install-existing.out" 2>&1; then
  echo 'production-ops-self-test: installer overwrote existing targets' >&2
  exit 1
fi
grep -Fq 'refusing to overwrite existing target' "$backup_fixture/install-existing.out"

PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --run --config-file "$backup_fixture/backup.conf" \
  >"$backup_fixture/run-one.out"
first_set=$(find "$backup_fixture/backups" -maxdepth 1 -type d -name 'backup-*' -print -quit)
[ -n "$first_set" ]
[ -f "$first_set/COMPLETE" ] && [ -f "$first_set/VERIFIED" ]
if grep -aEq 'raw-fixture-secret|curated-fixture-secret|4444444444' \
  "$first_set"/*.enc "$backup_fixture/run-one.out"; then
  echo 'production-ops-self-test: backup output leaked protected fixture content' >&2
  exit 1
fi
first_size=$(du -sb "$first_set" | awk '{print $1}')
write_backup_config $((first_size + 2048))
sleep 1
PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --run --config-file "$backup_fixture/backup.conf" \
  >"$backup_fixture/run-two.out"
[ "$(find "$backup_fixture/backups" -maxdepth 1 -type d -name 'backup-*' | wc -l)" -eq 1 ]
grep -Fq 'PRODUCTION_BACKUP_PRUNED' "$backup_fixture/run-two.out"
grep -Fq -- '--network none --read-only --user 999:999' "$FAKE_DOCKER_LOG"
PATH="$backup_fixture/bin:$ownership_bin:$PATH" REAL_INSTALL_BIN="$real_install" REAL_TAR_BIN="$real_tar" \
  FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --verify-latest \
  --config-file "$backup_fixture/backup.conf" >"$backup_fixture/verify.out"
grep -Fq 'isolated=true' "$backup_fixture/verify.out"

echo 'PRODUCTION_OPS_SELF_TEST: PASS'
