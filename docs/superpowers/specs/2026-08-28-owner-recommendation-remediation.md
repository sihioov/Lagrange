# Owner-beta recommendation remediation: metadata and display contract

**Status:** `USER_ACCEPTANCE_FAILED / REMEDIATION_REQUIRED`.

This specifies remediation for the existing owner-only, sealed, price-return-only recommendation display. It does not authorize changes to selection, weights, ETF11 membership, provider access, or automatic exclusion.

## Authority and boundaries

The sole owner-beta source is the value returned by `market_data::approve_historical_price_only_artifact`. Its compiled-in approval registry validates five pins and its fixed envelope requires ETF11, `OWNER_ONLY`, `PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false`, and range end `2026-08-19` (`crates/market-data/src/historical_price_only_approval.rs`).

`licensing.as_of` is entitlement/status data only. It must not select, default, constrain, or describe the owner-beta artifact. The current page incorrectly passes it to `OwnerBetaRunForm.defaultAsOf`.

| Display field | Exact local authority | Required behavior |
| --- | --- | --- |
| Canonical ID | `configs/universes/kr-etf-core-v1.yaml` and approved ETF11 bars | Display exact `instrument_id`; never substitute a product. |
| Name | `public.instruments.name` | Fetch matching ID. Null/no row displays `종목명 미제공`; retain the ID. |
| Asset class | `public.instruments.asset_class` | Display only when present; otherwise `자산군 미제공`. |
| Tracking index | Absent | Display `추적지수: 미제공`; never infer it. |
| Exposure group | Absent | Do not calculate/display a group; see blocked branch. |

`migrations/0003_market_data.up.sql` provides ID, symbol, venue, currency, name, asset class, status, and listing dates only. The universe YAML has identifiers/eligibility, not product names, tracking indices, or exposures.

## Supported-as-of discovery and default date

Add this owner-only read endpoint:

```
GET /api/v1/recommendations/owner-beta/price-only/supported-as-of
200 { "default_as_of": "YYYY-MM-DD", "supported_as_of": ["YYYY-MM-DD", ...] }
```

Apply the existing owner-beta read policy and recommendation entitlement gate, then call the same `approve_artifact` path used by enqueue. From approved bars, make the sorted intersection of `session_date` across all exact `KR_ETF_CORE_SYMBOLS`. Return it as `supported_as_of`; its maximum is `default_as_of`. Approval is the only authority: no client input, database dataset/path, or `licensing.as_of` may affect it.

Use a compact response only if it losslessly represents non-trading dates; otherwise return every date. The form must start at `default_as_of` and permit only `supported_as_of`; it must not fall back to `licensing.as_of`. POST retains the current exact-session re-approval check and rejects an absent date with `OWNER_BETA_PRICE_INPUT_UNAVAILABLE`.

If approval fails or the intersection is empty, return the existing sanitized unavailable result, hide/disable the form, create no run, and choose no nearest date. Label it `봉인된 입력 기준일`; label the maximum `지원되는 최신 기준일`, never a market-current date.

## Item projection and display

Add a display-only owner-beta detail projection; it is not a selector, target, or hash field:

```json
"instrument": {
  "id": "069500.KRX",
  "name": "... | null",
  "asset_class": "... | null",
  "tracking_index": null,
  "exposure_group": null
}
```

The server makes one bounded read for the eleven `instruments` rows. `instrument.id` equals existing `instrument_id`; the strict Web schema accepts only ETF11 IDs and nullable fields. Name/asset-class display needs no migration. Render name (or missing label), canonical ID beneath it, then asset class (or missing label). Missing metadata never changes rank, target weight, `excluded`, reason, cash, or selector behavior.

### Duplicate-exposure branch is blocked

The observed possible KOSPI200 overlap of `102110.KRX` and `069500.KRX` is not a grouping catalogue. Do not hard-code this pair, assign either a tracking index, create a selector rule, reweight, or exclude either item.

Only after the owner approves an authoritative local source with effective-date/maintenance terms may a separate migration add a versioned exposure mapping: canonical ID, approved group ID/label, source reference, and effective interval. Until then this branch is **blocked**. Afterwards, two or more selected items in the same non-null approved group produce a Korean warning naming canonical IDs and group label only. It preserves every item, rank, weight, reason, cash, and target hash and cannot affect `excluded` or target construction.

## Factor evidence

Display only finite raw values from the item's canonical decimal-string `factors` map, never normalized snapshot values. Unknown IDs or non-string/non-finite values fail the detail parse/render; excluded items may legitimately have no factor map.

| Factor IDs that can occur | Korean display name | Lookback / meaning | Semantic unit and display |
| --- | --- | --- | --- |
| none (`buy_and_hold`) | 표시할 팩터 없음 | No factor required. | `팩터 근거 없음(매수·보유 전략)`; no invented score. |
| `trend_N`, configured fast/slow `N=5..500` (defaults 50, 200) | `N일 이동평균 대비 가격 괴리` | `close / SMA_N(close) - 1`; N trading days, full window. | Signed decimal deviation; percent to two places, e.g. `+1.23%`. |
| `momentum_12_1` | `12개월-최근 1개월 제외 모멘텀` | `close(1개월 전) / close(12개월 전) - 1`; both calendar references required. | Signed decimal return; percent to two places. |
| `return_12m` | `12개월 수익률` | `close / close(12개월 전) - 1`; calendar reference required. | Signed decimal return; percent to two places. |
| `vol_N`, `N ∈ {20,60,120}` (default 60) | `N거래일 연율화 실현변동성` | `sqrt(252) × sample_std(log returns, trailing N)`; N returns need N+1 closes. | Annualized decimal volatility; unsigned percent to two places. |

Use configured `N`, not a default label. Do not render registry-only factors (`return_1m`, `return_3m`, `return_6m`, unconfigured `trend_100`, `drawdown`) as strategy evidence, nor call a raw factor total return, liquidity, a z-score, or a score.

## Korean reason-code mapping

`crates/job-queue/src/owner_beta/publish.rs` persists only the reason-code array and
`factors_json`, plus rank, target weight, `excluded`, and `exclusion_reason`: its item INSERT does
not persist `OwnerBetaReason.params`, `text_ko`, or `text_en`. The repository read row exposes the
same limited fields (`crates/api-server/src/repos/owner_beta.rs`), and the read DTO/API projection
returns only `instrument_id`, rank, target weight, excluded/exclusion reason, reason codes, and
factors (`crates/api-server/src/http/dto.rs`, `crates/api-server/src/http/owner_beta.rs`). The
read model deliberately omits `strategy_config_json`.

Consequently, the current durable-run mapping uses only the persisted code, `rank`,
`target_weight`, `factors`, `excluded`, and `exclusion_reason`. Render the durable rank and target
weight in their existing fields and render persisted factor values through the factor contract; do
not promise parameter interpolation in reason copy. Never reconstruct `top_n`, a missing factor
name, max/cash/rounding amount, benchmark, moving-average windows, return/threshold, volatility,
or weight from a mutable strategy config, current configuration, a later artifact, or a guess.

| Code | Korean explanation |
| --- | --- |
| `SELECTED_TOP_N` | 선정 기준에 따라 선택되었습니다. 순위는 별도 순위 필드에 표시합니다. |
| `NOT_SELECTED_BEYOND_TOP_N` | 선정 기준에 들지 않았습니다. |
| `EXCLUDED_MANDATORY_FACTOR_NULL` | 필수 팩터 값이 없어 제외되었습니다. |
| `ALL_CASH_NO_ELIGIBLE` | 선정 가능한 종목이 없어 전액 현금을 유지합니다. |
| `WEIGHT_CAPPED_AT_MAX` | 최대 비중 상한을 적용했습니다. |
| `WEIGHT_ROUNDING_RESIDUE_TO_CASH` | 반올림 잔여를 현금으로 배분했습니다. |
| `CASH_FLOOR_APPLIED` | 현금 최소 비중을 유지했습니다. |
| `BENCHMARK_HELD` | 이 종목을 벤치마크로 보유합니다. 비중은 별도 목표 비중 필드에 표시합니다. |
| `TREND_POSITIVE` | 상승 추세 조건을 충족했습니다. |
| `TREND_NEGATIVE_CASH` | 추세 조건을 충족하지 않아 현금을 유지합니다. |
| `ABSOLUTE_MOMENTUM_PASSED` | 절대 모멘텀 조건을 충족했습니다. |
| `DEFENSIVE_CASH_SELECTED` | 절대 모멘텀 조건을 충족하지 않아 방어적으로 현금을 선택했습니다. |
| `INVERSE_VOL_WEIGHTED` | 변동성을 고려한 역변동성 방식으로 비중을 배정했습니다. |
| `NOT_SELECTED_BY_STRATEGY` | 이 고정 유니버스 종목은 해당 전략에서 선택하지 않았습니다. |

Unknown, duplicate, malformed, or required-but-missing codes are contract violations: fail into existing unavailable/error UI; do not show raw code text as user explanation.

Parameter-rich reason explanations require a future, separately approved, versioned persistence
contract and migration that stores the canonical reason code, lexical parameters, and localized
text (or a versioned localized-template identity) atomically with the target publication. That work
must preserve historical-row semantics and define its API/read migration; it is not part of WP-3 or
this display-only remediation. Until it is approved and implemented, existing durable runs use only
the static mapping above.

## Provenance and hashes

Keep strategy ID/version, run as-of, audience, capability, vendor-snapshot/strict-PIT warnings, weights/cash, and timestamps in the main report. Move internal identifiers/hashes into a collapsed keyboard-accessible `감사 세부정보` disclosure: run ID, job ID, strategy-config ID, strategy-config hash, candidate-content hash, artifact/stage5/action/approval-registry hashes, factor-snapshot hash, and target-snapshot hash. Preserve each value verbatim, copyable, owner-only, and not removed/truncated/recomputed.

## Deterministic acceptance tests

1. An approved fixture returns only common ETF11 sessions and maximum default (`2026-08-19` today); a different licensing as-of is ignored.
2. Approval failure, empty intersection, malformed bar identity, or POST date absent from re-approved sessions returns sanitized unavailable output and creates no run; no nearest-date fallback.
3. A Web test proves the form uses discovery, never `licensing.as_of`, and blocks when discovery is unavailable.
4. Present/null/absent instrument-row fixtures retain IDs and do not affect selector, target, exclusion, rank, weight, or cash.
5. Contract tests enforce ETF11-only IDs and `instrument.id == instrument_id`, while allowing nullable display metadata.
6. Tests cover every factor pattern, raw-percent formatting, and no normalized/registry-only factor; unknown factors fail closed.
7. Parameterized tests cover all fourteen codes using only durable item fields. They prove no
   rendered reason interpolates a missing parameter, reads a mutable strategy config, or guesses a
   factor/window/threshold; rank, target weight, and factors remain separately rendered from their
   persisted values. Invalid taxonomy fails closed.
8. Before exposure approval, tests prove no pair-specific group, selector change, or automatic exclusion. After approval, warning tests prove unchanged target/result bytes.
9. Snapshot tests prove the primary provenance grid has no hash/ID commitments and audit disclosure retains every existing value, including null/not-reported snapshots.

## Implementation sequence and blocker

Implement discovery and API/Web tests; wire the form default; add bounded name/asset-class projection and missing-value UI; add factor/reason dictionaries/tests; then audit disclosure. Tracking-index/exposure display and any 102110/069500 warning remain blocked pending owner-approved authority and a separate migration. No step permits selector or automatic-exclusion changes.
