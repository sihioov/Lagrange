#!/usr/bin/env bash
# Browser proof for the synthetic candidate research surfaces. The fixture is
# intentionally QA-only and cannot satisfy the production source-readiness gate.
# Despite the "candidate" name, this now runs the entire tests/e2e/ directory
# (candidates, backtests, recommendations, paper, live-kill-switch,
# no-member-live, and the phase1 multi-user specs) against the one shared
# synthetic API + Next dev server brought up below, in a single serial
# Playwright invocation. Kept under the original name to avoid touching
# ci.yml's exact step list, which scripts/ci/test_ci_contract.py asserts by
# equality.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
web_dir="$root/apps/web"
api_port="${CANDIDATE_E2E_API_PORT:-38182}"
web_port="${CANDIDATE_E2E_WEB_PORT:-33001}"
evidence_dir=$(mktemp -d)
mock_pid=''
web_pid=''

cleanup() {
  if [ -n "$web_pid" ]; then
    kill "$web_pid" 2>/dev/null || true
  fi
  if [ -n "$mock_pid" ]; then
    kill "$mock_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

wait_for_url() {
  local url="$1"
  local attempts="$2"
  local count=0
  until curl --fail --silent "$url" >/dev/null 2>&1; do
    count=$((count + 1))
    if [ "$count" -ge "$attempts" ]; then
      return 1
    fi
    sleep 0.25
  done
}

(
  cd "$web_dir"
  exec env SYNTHETIC_API_PORT="$api_port" node tests/e2e/support/synthetic-api.mjs
) >"$evidence_dir/synthetic-api.log" 2>&1 &
mock_pid=$!
if ! wait_for_url "http://127.0.0.1:$api_port/api/v1/auth/session" 80; then
  cat "$evidence_dir/synthetic-api.log" >&2
  echo 'candidate-web-e2e: synthetic API did not become ready' >&2
  exit 1
fi

(
  cd "$web_dir"
  exec env API_INTERNAL_URL="http://127.0.0.1:$api_port" PORT="$web_port" \
    npx next dev -p "$web_port"
) >"$evidence_dir/web.log" 2>&1 &
web_pid=$!
if ! wait_for_url "http://127.0.0.1:$web_port/healthz" 160; then
  cat "$evidence_dir/web.log" >&2
  echo 'candidate-web-e2e: Next application did not become ready' >&2
  exit 1
fi

cd "$web_dir"
SYNTHETIC_API_ORIGIN="http://127.0.0.1:$api_port" \
PLAYWRIGHT_BASE_URL="http://127.0.0.1:$web_port" \
npx playwright test tests/e2e/
