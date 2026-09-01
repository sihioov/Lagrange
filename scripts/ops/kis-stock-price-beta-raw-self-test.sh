#!/usr/bin/env bash
# Provider-free self-test for the fixed-stock Raw operator gate.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
wrapper=$script_dir/kis-stock-price-beta-raw.sh

[ -x "$wrapper" ] || {
  printf 'kis-stock-price-beta-raw-self-test: wrapper is not executable\n' >&2
  exit 1
}
bash -n "$wrapper"

plan=$(bash "$wrapper" --plan)
grep -Fq 'range=2025-08-04..2026-08-28 universe=kr-stock-price-beta-v1 symbols=30' <<<"$plan"
grep -Fq 'interval=D windows=window-01:2026-04-21..2026-08-28,window-02:2025-12-12..2026-04-20,window-03:2025-08-04..2025-12-11 planned_gets_before_retries=90' <<<"$plan"
grep -Fq 'GET /uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice' <<<"$plan"
grep -Fq 'tr_id=FHKST03010100' <<<"$plan"
grep -Fq 'FID_ORG_ADJ_PRC=1' <<<"$plan"
grep -Fq 'continuation=blank single_page=yes' <<<"$plan"
grep -Fq 'profile=stock-price-beta-raw' <<<"$plan"
grep -Fq 'no_curated_db_artifact_mounts=yes' <<<"$plan"
grep -Fq 'entitlement=ent_kis_personal_owner_20260821 provider=kis' <<<"$plan"
grep -Fq 'reference=repo://docs/decisions/0005-kis-personal-use-entitlement.md' <<<"$plan"
grep -Fq 'file_sha256=56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e' <<<"$plan"
grep -Fq 'PLAN_ONLY: no Docker' <<<"$plan"
if grep -Eq 'TEST_APP_KEY|TEST_APP_SECRET|access_token|response_body' <<<"$plan"; then
  printf 'kis-stock-price-beta-raw-self-test: plan leaked a sensitive marker\n' >&2
  exit 1
fi

if bash "$wrapper" --start 2026-01-02 --end 2026-01-30 --plan >/dev/null 2>&1; then
  printf 'kis-stock-price-beta-raw-self-test: non-fixed range was accepted\n' >&2
  exit 1
fi

if bash "$wrapper" --start 2026-01-02 --start 2026-01-03 --plan >/dev/null 2>&1; then
  printf 'kis-stock-price-beta-raw-self-test: repeated --start was accepted\n' >&2
  exit 1
fi
if bash "$wrapper" --plan --execute >/dev/null 2>&1; then
  printf 'kis-stock-price-beta-raw-self-test: conflicting action was accepted\n' >&2
  exit 1
fi

grep -Fq 'compose build --pull=false "$compose_service"' "$wrapper"
grep -Fq 'compose run --rm --no-deps' "$wrapper"
grep -Fq 'KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS' "$wrapper"
grep -Fq 'if [ "$mode" = plan ]; then' "$wrapper"

printf 'STOCK_PRICE_BETA_RAW_SELF_TEST: PASS (provider-free)\n'
