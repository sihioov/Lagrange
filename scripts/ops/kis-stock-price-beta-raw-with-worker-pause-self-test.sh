#!/usr/bin/env bash
# Provider-free fake-Docker tests for the fixed-stock worker-pause wrapper.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
wrapper=$script_dir/kis-stock-price-beta-raw-with-worker-pause.sh
commit=0123456789abcdef0123456789abcdef01234567
entitlement_hash=56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e
entitlement_reference=repo://docs/decisions/0005-kis-personal-use-entitlement.md

[ -x "$wrapper" ] || {
  printf 'kis-stock-price-beta-raw-with-worker-pause-self-test: wrapper is not executable\n' >&2
  exit 1
}
bash -n "$wrapper"

test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT
fake_bin=$test_root/bin
mkdir -p "$fake_bin" "$test_root/state"
release_root=$test_root/releases/$commit
mkdir -p "$release_root/scripts/ops/lib" "$release_root/configs/data-rights" \
  "$release_root/configs/universes" "$release_root/deploy/compose"
cp "$script_dir/kis-stock-price-beta-raw.sh" "$release_root/scripts/ops/kis-stock-price-beta-raw.sh"
cp "$script_dir/lib/dotenv.sh" "$release_root/scripts/ops/lib/dotenv.sh"
cp "$script_dir/../../configs/data-rights/kis.entitlement.json" "$release_root/configs/data-rights/kis.entitlement.json"
cp "$script_dir/../../configs/universes/kr-stock-price-beta-v1.json" "$release_root/configs/universes/kr-stock-price-beta-v1.json"
cp "$script_dir/../../deploy/compose/compose.yml" "$release_root/deploy/compose/compose.yml"
chmod 755 "$release_root/scripts/ops/kis-stock-price-beta-raw.sh"
[ ! -e "$release_root/.git" ] || {
  printf 'self-test fixture must model a gitless installed release\n' >&2
  exit 1
}
wrong_release_root=$test_root/releases/ffffffffffffffffffffffffffffffffffffffff
mkdir -p "$wrong_release_root"
env_file=$test_root/release.env
cat >"$env_file" <<EOF
LAGRANGE_CODE_COMMIT=$commit
RESEARCH_ENTITLEMENT_REFERENCE=$entitlement_reference
RESEARCH_ENTITLEMENT_SHA256=$entitlement_hash
EOF
chmod 600 "$env_file"
wrong_env_file=$test_root/wrong-release.env
sed "s/$commit/ffffffffffffffffffffffffffffffffffffffff/" "$env_file" >"$wrong_env_file"
chmod 600 "$wrong_env_file"

cat >"$fake_bin/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'unexpected git invocation in gitless installed-release test\n' >&2
exit 1
EOF
chmod +x "$fake_bin/git"

cat >"$fake_bin/id" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = -u ] && printf '0\n'
EOF
chmod +x "$fake_bin/id"

cat >"$fake_bin/stat" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  *'%u:%g:%a'*) printf '0:0:600\n' ;;
  *'%u'*) printf '0\n' ;;
  *) /usr/bin/stat "$@" ;;
esac
EOF
chmod +x "$fake_bin/stat"

cat >"$fake_bin/date" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "$FAKE_MODE" = health_timeout ] && [ "$*" = '-u +%s' ]; then
  counter_file=$FAKE_STATE/date-count
  count=0
  [ -f "$counter_file" ] && count=$(cat "$counter_file")
  printf '%s\n' $((1000 + count * 181))
  printf '%s\n' $((count + 1)) >"$counter_file"
  exit 0
fi
exec /bin/date "$@"
EOF
chmod +x "$fake_bin/date"

cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_LOG"

if [ "$1" = ps ]; then
  case "$FAKE_MODE" in
    no_worker) exit 0 ;;
    multiple_worker) printf 'ctr-worker-a\nctr-worker-b\n' ;;
    *) printf 'ctr-worker\n' ;;
  esac
  exit 0
fi

if [ "$1" = stop ]; then
  [ "$FAKE_MODE" = stop_failure ] && exit 1
  exit 0
fi

if [ "$1" = start ]; then
  [ "$FAKE_MODE" = restart_failure ] && exit 1
  exit 0
fi

if [ "$1" = inspect ]; then
  if [[ "$*" == *'.State.Health'* ]]; then
    [ "$FAKE_MODE" = health_timeout ] && printf 'starting\n' || printf 'healthy\n'
  elif [ "$FAKE_MODE" = wrong_labels ]; then
    printf 'other/service|sha256:worker-image\n'
  else
    printf 'lagrange-station/research-worker|sha256:worker-image\n'
  fi
  exit 0
fi

if [ "$1" = image ] && [ "$2" = inspect ]; then
  if [[ "$*" == *'org.opencontainers.image.revision'* ]]; then
    if [ "$FAKE_MODE" = wrong_revision ] || \
       { [ "$FAKE_MODE" = prepared_image_wrong_revision ] && [[ "$*" == *lagrange-station-research-stock-price-beta-raw* ]]; }; then
      printf 'ffffffffffffffffffffffffffffffffffffffff\n'
    else
      printf '%s\n' "$FAKE_COMMIT"
    fi
  elif [[ "$*" == *'.Config.Env'* ]]; then
    printf 'LAGRANGE_CODE_COMMIT=%s\n' "$FAKE_COMMIT"
  fi
  exit 0
fi

if [ "$1" = compose ]; then
  case " $* " in
    *' version '*) printf 'Docker Compose version v2\n' ;;
    *' build '*)
      [ "$FAKE_MODE" = prepare_failure ] && exit 1
      ;;
    *' run '*)
      [ "$FAKE_MODE" = inner_failure ] && exit 1
      ;;
  esac
  exit 0
fi

printf 'unexpected fake docker command: %s\n' "$*" >&2
exit 1
EOF
chmod +x "$fake_bin/docker"

run_case() {
  local mode=$1 selected_env=${2:-$env_file} selected_release_root=${3:-$release_root} output log
  log=$test_root/$mode.log
  : >"$log"
  rm -f "$test_root/state/date-count"
  if output=$(env -u LAGRANGE_CODE_COMMIT -u RESEARCH_ENTITLEMENT_REFERENCE -u RESEARCH_ENTITLEMENT_SHA256 \
    PATH="$fake_bin:$PATH" FAKE_MODE="$mode" FAKE_LOG="$log" FAKE_STATE="$test_root/state" FAKE_COMMIT="$commit" \
    KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS \
    "$wrapper" --execute --commit "$commit" --env-file "$selected_env" --release-root "$selected_release_root" 2>&1); then
    printf '%s' "$output" >"$test_root/$mode.out"
    return 0
  fi
  printf '%s' "$output" >"$test_root/$mode.out"
  return 1
}

plan_log=$test_root/plan.log
: >"$plan_log"
plan=$(env PATH="$fake_bin:$PATH" FAKE_LOG="$plan_log" FAKE_MODE=success FAKE_STATE="$test_root/state" FAKE_COMMIT="$commit" \
  "$wrapper" --plan)
grep -Fq 'range=2025-08-04..2026-08-28' <<<"$plan"
grep -Fq 'PLAN_ONLY: no Docker' <<<"$plan"
[ ! -s "$plan_log" ]

run_case success
grep -Fq 'KIS_STOCK_PRICE_BETA_RAW_WITH_WORKER_PAUSE: PASS' "$test_root/success.out"
grep -Fq 'KIS_STOCK_PRICE_BETA_RAW_WORKER_RESTORE: PASS' "$test_root/success.out"
grep -Fq 'stop --time 300 ctr-worker' "$test_root/success.log"
grep -Fxq 'start ctr-worker' "$test_root/success.log"
grep -Fq 'compose ' "$test_root/success.log"
[ "$(grep -Ec 'compose .* build ' "$test_root/success.log")" -eq 1 ]
build_line=$(grep -n 'compose .* build ' "$test_root/success.log" | head -n1 | cut -d: -f1)
stop_line=$(grep -n 'stop --time 300 ctr-worker' "$test_root/success.log" | head -n1 | cut -d: -f1)
[ "$build_line" -lt "$stop_line" ]

if run_case inner_failure; then
  printf 'self-test: inner failure unexpectedly passed\n' >&2
  exit 1
fi
grep -Fxq 'start ctr-worker' "$test_root/inner_failure.log"
grep -Fq 'compose ' "$test_root/inner_failure.log"

if run_case stop_failure; then
  printf 'self-test: stop failure unexpectedly passed\n' >&2
  exit 1
fi
grep -Fq 'stop --time 300 ctr-worker' "$test_root/stop_failure.log"
if grep -Eq '(^| )start( |$)|compose .* run ' "$test_root/stop_failure.log"; then
  printf 'self-test: stop failure invoked capture or restart\n' >&2
  exit 1
fi

if run_case restart_failure; then
  printf 'self-test: restart failure unexpectedly passed\n' >&2
  exit 1
fi
grep -Fxq 'start ctr-worker' "$test_root/restart_failure.log"

if run_case health_timeout; then
  printf 'self-test: health timeout unexpectedly passed\n' >&2
  exit 1
fi
grep -Fq 'research-worker health timeout or failure' "$test_root/health_timeout.out"

if run_case prepare_failure; then
  printf 'self-test: image preparation failure unexpectedly passed\n' >&2
  exit 1
fi
if grep -Eq '(^| )stop( |$)|compose .* run ' "$test_root/prepare_failure.log"; then
  printf 'self-test: failed image preparation paused the worker or ran capture\n' >&2
  exit 1
fi

if run_case prepared_image_wrong_revision; then
  printf 'self-test: prepared image revision mismatch unexpectedly passed\n' >&2
  exit 1
fi
if grep -Eq '(^| )stop( |$)|compose .* run ' "$test_root/prepared_image_wrong_revision.log"; then
  printf 'self-test: bad prepared image revision paused the worker or ran capture\n' >&2
  exit 1
fi

if run_case wrong_labels; then
  printf 'self-test: wrong labels unexpectedly passed\n' >&2
  exit 1
fi
if grep -Eq '(^| )stop( |$)|compose .* run ' "$test_root/wrong_labels.log"; then
  printf 'self-test: wrong labels reached stop or capture\n' >&2
  exit 1
fi

if run_case wrong_revision; then
  printf 'self-test: wrong revision unexpectedly passed\n' >&2
  exit 1
fi
if grep -Eq '(^| )stop( |$)|compose .* run ' "$test_root/wrong_revision.log"; then
  printf 'self-test: wrong revision reached stop or capture\n' >&2
  exit 1
fi

if run_case multiple_worker; then
  printf 'self-test: multiple workers unexpectedly passed\n' >&2
  exit 1
fi
if grep -Eq '(^| )inspect( |$)|(^| )stop( |$)|compose .* run ' "$test_root/multiple_worker.log"; then
  printf 'self-test: multiple workers reached an unsafe operation\n' >&2
  exit 1
fi

if run_case wrong_release_directory "$env_file" "$wrong_release_root"; then
  printf 'self-test: wrong release directory commit unexpectedly passed\n' >&2
  exit 1
fi
[ ! -s "$test_root/wrong_release_directory.log" ]

if run_case env_release_mismatch "$wrong_env_file"; then
  printf 'self-test: installed env/release commit mismatch unexpectedly passed\n' >&2
  exit 1
fi
[ ! -s "$test_root/env_release_mismatch.log" ]

run_case no_worker
if grep -Eq '(^| )stop( |$)|(^| )start( |$)' "$test_root/no_worker.log"; then
  printf 'self-test: no-worker path invented worker lifecycle\n' >&2
  exit 1
fi
grep -Fq 'compose ' "$test_root/no_worker.log"

printf 'STOCK_PRICE_BETA_RAW_WITH_WORKER_PAUSE_SELF_TEST: PASS (fake Docker, no provider/network)\n'
