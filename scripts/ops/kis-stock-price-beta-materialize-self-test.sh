#!/usr/bin/env bash
set -euo pipefail

d=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
w=$d/kis-stock-price-beta-materialize.sh
bash -n "$w"

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
commit=0123456789abcdef0123456789abcdef01234567
other_commit=ffffffffffffffffffffffffffffffffffffffff
release=$root/releases/$commit
data=$root/data
fake_path=$root/bin:/usr/bin:/bin
log=$root/command.log
audit=$root/audit.log
case_output=$root/case.out

fail() {
  echo "STOCK_PRICE_BETA_MATERIALIZE_SELF_TEST: FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local needle=$1 file=$2
  grep -Fq -- "$needle" "$file" || fail "missing command log entry: $needle"
}

expect_reject() {
  local label=$1
  shift
  : >"$log"
  if invoke "$@" >"$case_output" 2>&1; then
    fail "$label was accepted"
  fi
}

expect_status() {
  local label=$1 expected=$2 actual
  shift 2
  : >"$log"
  set +e
  invoke "$@" >"$case_output" 2>&1
  actual=$?
  set -e
  [ "$actual" -eq "$expected" ] || fail "$label returned $actual, expected $expected"
}

mkdir -p \
  "$release/deploy/compose" \
  "$release/configs/universes" \
  "$release/configs/data-rights" \
  "$release/configs/evidence" \
  "$data/raw" \
  "$data/stock-price-beta-artifacts" \
  "$root/out" \
  "$root/bin"
: >"$release/deploy/compose/compose.yml"
: >"$release/configs/universes/kr-stock-price-beta-v1.json"
: >"$release/configs/data-rights/kis.entitlement.json"
: >"$release/configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json"
: >"$audit"
[ ! -e "$release/.git" ] || fail 'disposable release is not gitless'

envfile=$root/env
printf 'LAGRANGE_CODE_COMMIT=%s\nLAGRANGE_DATA_DIR=%s\n' "$commit" "$data" >"$envfile"
bad_envfile=$root/bad-env
printf 'LAGRANGE_CODE_COMMIT=%s\nLAGRANGE_DATA_DIR=%s\n' "$other_commit" "$data" >"$bad_envfile"

cat >"$root/bin/id" <<'EOF'
#!/usr/bin/env bash
{
  printf 'id'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_LOG"
{
  printf 'id'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_AUDIT_LOG"
echo "${FAKE_UID:-0}"
EOF

cat >"$root/bin/stat" <<'EOF'
#!/usr/bin/env bash
{
  printf 'stat'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_LOG"
{
  printf 'stat'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_AUDIT_LOG"
target=${!#}
if [ -n "${FAKE_ENV_PATH:-}" ] && [ "$target" = "$FAKE_ENV_PATH" ] && [ -n "${FAKE_ENV_STAT:-}" ]; then
  echo "$FAKE_ENV_STAT"
elif [ "${1:-}" = -c ] && [ "${2:-}" = %u:%g:%a ]; then
  echo 0:0:600
elif [ "${1:-}" = -c ] && [ "${2:-}" = %u ]; then
  echo 0
else
  exit 96
fi
EOF

cat >"$root/bin/readlink" <<'EOF'
#!/usr/bin/env bash
{
  printf 'readlink'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_LOG"
{
  printf 'readlink'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_AUDIT_LOG"
[ "$#" -eq 2 ] && [ "$1" = -f ] || exit 96
echo "$2"
EOF

cat >"$root/bin/install" <<'EOF'
#!/usr/bin/env bash
{
  printf 'install'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_LOG"
{
  printf 'install'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_AUDIT_LOG"
exit "${FAKE_INSTALL_FAIL:-0}"
EOF

cat >"$root/bin/docker" <<'EOF'
#!/usr/bin/env bash
{
  printf 'docker'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_LOG"
{
  printf 'docker'
  printf ' %q' "$@"
  printf '\n'
} >>"$FAKE_AUDIT_LOG"

if [ "${1:-}" = image ] && [ "${2:-}" = inspect ]; then
  case "${4:-}" in
    *org.opencontainers.image.revision*)
      echo "${FAKE_REVISION:-$FAKE_EXPECTED}"
      exit 0
      ;;
    *Config.Env*)
      echo "LAGRANGE_CODE_COMMIT=${FAKE_IMAGE_COMMIT:-$FAKE_EXPECTED}"
      exit 0
      ;;
    *) exit 96 ;;
  esac
fi

[ "${1:-}" = compose ] || exit 96
shift
if [ "${1:-}" = version ] && [ "$#" -eq 1 ]; then
  exit 0
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile|--env-file|--file)
      [ "$#" -ge 2 ] || exit 96
      shift 2
      ;;
    *) break ;;
  esac
done

subcommand=${1:-}
shift || true
case "$subcommand" in
  config)
    [ "$#" -eq 1 ] && [ "$1" = --quiet ] || exit 96
    exit "${FAKE_CONFIG_FAIL:-0}"
    ;;
  build)
    [ "$#" -eq 2 ] && [ "$1" = --pull=false ] && [ "$2" = research-stock-price-beta-materialize ] || exit 96
    exit "${FAKE_BUILD_FAIL:-0}"
    ;;
  run)
    if [ "${FAKE_CHECK_OUTCOME:-}" = empty ]; then
      exit 20
    elif [ "${FAKE_CHECK_OUTCOME:-}" = unapproved ]; then
      exit 21
    fi
    exit "${FAKE_RUN_FAIL:-0}"
    ;;
  *) exit 96 ;;
esac
EOF
chmod +x "$root/bin/"*

invoke() {
  FAKE_LOG="$log" \
  FAKE_AUDIT_LOG="$audit" \
  FAKE_EXPECTED="$commit" \
  FAKE_UID="${FAKE_UID:-0}" \
  FAKE_ENV_PATH="${FAKE_ENV_PATH:-}" \
  FAKE_ENV_STAT="${FAKE_ENV_STAT:-}" \
  FAKE_CONFIG_FAIL="${FAKE_CONFIG_FAIL:-0}" \
  FAKE_BUILD_FAIL="${FAKE_BUILD_FAIL:-0}" \
  FAKE_RUN_FAIL="${FAKE_RUN_FAIL:-0}" \
  FAKE_CHECK_OUTCOME="${FAKE_CHECK_OUTCOME:-}" \
  FAKE_REVISION="${FAKE_REVISION:-$commit}" \
  FAKE_IMAGE_COMMIT="${FAKE_IMAGE_COMMIT:-$commit}" \
  PATH="$fake_path" \
  STOCK_PRICE_BETA_MATERIALIZE_CONFIRM="${TEST_CONFIRM-I_CONFIRM_PROVIDER_FREE_RAW_MATERIALIZATION}" \
  "$w" "$@"
}

common=(
  --commit "$commit"
  --env-file "$envfile"
  --release-root "$release"
  --raw-root "$data/raw"
  --artifact-root "$data/stock-price-beta-artifacts"
  --universe "$release/configs/universes/kr-stock-price-beta-v1.json"
  --entitlement "$release/configs/data-rights/kis.entitlement.json"
  --batch-id 11111111-1111-1111-1111-111111111111
  --capture-commit "$commit"
)
registry=$release/configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json

# Plan is deliberately local: it must not touch identity, filesystem metadata,
# install, Docker, or Compose through the fake external-command surface.
: >"$log"
plan=$(invoke --plan)
grep -Fq 'docker=no env=no mutation=no' <<<"$plan" || fail 'local plan output changed'
[ ! -s "$log" ] || fail 'local plan invoked a fake external command'

: >"$log"
invoke --preflight "${common[@]}"
assert_contains "docker compose --profile stock-price-beta-materialize --env-file $envfile --file $release/deploy/compose/compose.yml config --quiet" "$log"

: >"$log"
invoke --materialize "${common[@]}"
materialize_run="docker compose --profile stock-price-beta-materialize --env-file $envfile --file $release/deploy/compose/compose.yml run --rm --no-deps research-stock-price-beta-materialize materialize --raw-root /data --artifact-root /data/artifacts --universe /opt/lagrange/configs/universes/kr-stock-price-beta-v1.json --entitlement /opt/lagrange/configs/data-rights/kis.entitlement.json --batch-id 11111111-1111-1111-1111-111111111111 --capture-commit $commit --confirm I_CONFIRM_PROVIDER_FREE_RAW_MATERIALIZATION"
assert_contains "$materialize_run" "$log"
build_line=$(grep -nF ' build --pull=false research-stock-price-beta-materialize' "$log" | cut -d: -f1)
label_line=$(grep -nF 'org.opencontainers.image.revision' "$log" | cut -d: -f1)
env_line=$(grep -nF 'Config.Env' "$log" | cut -d: -f1)
run_line=$(grep -nF "$materialize_run" "$log" | cut -d: -f1)
[ "$build_line" -lt "$label_line" ] && [ "$label_line" -lt "$env_line" ] && [ "$env_line" -lt "$run_line" ] || fail 'build/provenance did not precede run'

: >"$log"
invoke --check "${common[@]}" --registry "$registry"
check_run="docker compose --profile stock-price-beta-materialize --env-file $envfile --file $release/deploy/compose/compose.yml run --rm --no-deps research-stock-price-beta-materialize check --raw-root /data --artifact-root /data/artifacts --universe /opt/lagrange/configs/universes/kr-stock-price-beta-v1.json --entitlement /opt/lagrange/configs/data-rights/kis.entitlement.json --batch-id 11111111-1111-1111-1111-111111111111 --capture-commit $commit --registry /opt/lagrange/configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json"
assert_contains "$check_run" "$log"
FAKE_CHECK_OUTCOME=empty expect_status 'empty check result' 20 --check "${common[@]}" --registry "$registry"
FAKE_CHECK_OUTCOME=unapproved expect_status 'unapproved check result' 21 --check "${common[@]}" --registry "$registry"

: >"$log"
invoke --proposal "${common[@]}" --output "$root/out/proposal.json"
proposal_run="docker compose --profile stock-price-beta-materialize --env-file $envfile --file $release/deploy/compose/compose.yml run --rm --no-deps --volume $root/out:/data/proposals:rw research-stock-price-beta-materialize proposal --raw-root /data --artifact-root /data/artifacts --universe /opt/lagrange/configs/universes/kr-stock-price-beta-v1.json --entitlement /opt/lagrange/configs/data-rights/kis.entitlement.json --batch-id 11111111-1111-1111-1111-111111111111 --capture-commit $commit --output /data/proposals/proposal.json --confirm I_CONFIRM_PROVIDER_FREE_RAW_MATERIALIZATION"
assert_contains "$proposal_run" "$log"

expect_reject 'wrong release directory commit' --check "${common[@]}" --registry "$registry" --release-root "$root/releases/$other_commit"
expect_reject 'environment commit mismatch' --preflight "${common[@]}" --env-file "$bad_envfile"
FAKE_UID=1000 expect_reject 'non-root identity' --preflight "${common[@]}"
FAKE_ENV_PATH=$envfile FAKE_ENV_STAT=0:0:640 expect_reject 'wrong environment permissions' --preflight "${common[@]}"
expect_reject 'raw host mismatch' --check "${common[@]}" --registry "$registry" --raw-root "$root/bad-raw"
expect_reject 'artifact host mismatch' --check "${common[@]}" --registry "$registry" --artifact-root "$root/bad-artifacts"
TEST_CONFIRM= expect_reject 'missing confirmation' --materialize "${common[@]}"

mv "$release/deploy/compose/compose.yml" "$release/deploy/compose/compose.real.yml"
ln -s compose.real.yml "$release/deploy/compose/compose.yml"
expect_reject 'symlinked Compose control' --preflight "${common[@]}"
rm "$release/deploy/compose/compose.yml"
mv "$release/deploy/compose/compose.real.yml" "$release/deploy/compose/compose.yml"

mv "$release/configs/universes/kr-stock-price-beta-v1.json" "$release/configs/universes/kr-stock-price-beta-v1.real.json"
ln -s kr-stock-price-beta-v1.real.json "$release/configs/universes/kr-stock-price-beta-v1.json"
expect_reject 'symlinked universe input' --preflight "${common[@]}"
rm "$release/configs/universes/kr-stock-price-beta-v1.json"
mv "$release/configs/universes/kr-stock-price-beta-v1.real.json" "$release/configs/universes/kr-stock-price-beta-v1.json"

FAKE_CONFIG_FAIL=30 expect_reject 'Compose config failure' --materialize "${common[@]}"
FAKE_BUILD_FAIL=31 expect_reject 'cold build failure' --materialize "${common[@]}"
FAKE_REVISION=bad expect_reject 'OCI revision label mismatch' --materialize "${common[@]}"
FAKE_IMAGE_COMMIT=bad expect_reject 'image ENV commit mismatch' --materialize "${common[@]}"
FAKE_RUN_FAIL=32 expect_status 'Compose run failure' 32 --materialize "${common[@]}"

if grep -Eqi -- 'KIS.*(KEY|SECRET|TOKEN)|APP[_-]?(KEY|SECRET)|ACCESS[_-]?TOKEN|(^|[^[:alnum:]_])(TOKEN|DB|BODY)([^[:alnum:]_]|$)|DATABASE|POSTGRES|CURATED|BACKEND|--network|network_mode|CANO|ACNT_PRDT_CD|KIS_ACCOUNT_REF|(^|[[:space:]])(account|order)([[:space:]]|$)|request[_-]?body|response[_-]?body' "$audit"; then
  fail 'command log contains a forbidden secret, data surface, network, account/order, or body reference'
fi

echo 'STOCK_PRICE_BETA_MATERIALIZE_SELF_TEST: PASS (fake Docker/Compose)'
