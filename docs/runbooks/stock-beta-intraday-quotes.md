# Stock Beta intraday current quotes

Status: source/fixture acceptance and conditional release wiring; not live activation. This document does not
authorize live activation, provider access, credential handling, deployment, or a production
health claim. The implementation is bounded by the [intraday quote contract](../superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md)
and the checked-in source linked below.

## Boundary and source contract

The V1 seam is a private, owner-only, periodically polled current quote for an admitted Stock
Beta V2 membership. Its UI label is `장중 현재가 · 주기적 조회`. The producer is independent of
EOD collection and uses exactly one broker read:

| Item | Frozen value |
| --- | --- |
| Method and path | `GET /uapi/domestic-stock/v1/quotations/inquire-price` |
| TR ID | `FHKST01010100` |
| Query | `FID_COND_MRKT_DIV_CODE=J`, `FID_INPUT_ISCD=<six ASCII digits>` |
| Continuation | Blank fields; no follow-up page |
| Entitlement | Private owner-only, never public, demo, browser-persisted, invited-member, admin, Raw, Curated, logs, or metrics |

The exact request construction is in the [intraday collector](../../data-pipelines/collectors/src/intraday_quotes.rs#L21-L29)
and the [KIS market-data provider](../../crates/market-data/src/providers/kis.rs#L59-L64). The
allowlist remains deny-by-default in [KIS client market data](../../crates/kis-client/src/market_data.rs#L19-L54).
NX, UN, other market divisions, ETF-specific APIs, generic proxies, WebSockets, account/order
APIs, and live-order profiles are outside this seam. ETF11 membership is not implied by this
feature.

The response parser requires one successful object with `rt_cd="0"`, the exact requested
identity, and canonical decimal strings. `stck_prpr` is the positive current price;
`prdy_vrss` and `prdy_ctrt` carry the signed change and rate; `stck_sdpr` is exposed only as
`base_price`; and status `58` or `temp_stop_yn=Y` makes the instrument halted. Provider
timestamps and previous-close semantics are not admitted. The parser's [field and decimal
contract](../../crates/market-data/src/intraday_quotes.rs#L36-L54) and [direction/status
validation](../../crates/market-data/src/intraday_quotes.rs#L192-L300) are the source of truth.

Current quotes never append or replace an EOD candle, close, indicator, rank, snapshot, Raw
batch, Curated record, or generation. The [owner runner](../../crates/job-queue/src/bin/owner-equity-v2-runner.rs#L103-L181)
keeps this loop separate from EOD work; `--once` does not start the quote loop. The API's quote
read is cache-only: `GET /api/v1/research/owner-beta/equity-universe-v2/instruments/{instrument_id}/quote?membership_id={uuid}&generation={u64}`
does not write, enqueue, or call a provider. It returns `200` with `Cache-Control: no-store`
for a valid identity even when no quote is available. Demand is explicit through the literal
POST/DELETE demand routes shown in the [API router](../../crates/api-server/src/http/mod.rs#L344-L354).

No active demand is a producer stop, not a cache-visibility rejection. A previously validated
cache row remains subject to the normal identity, session, generation, and age rules; the
projection reports `NO_ACTIVE_DEMAND` where the frozen C3 precedence requires it.

## C3 state, freshness, and reasons

The [API projection](../../crates/api-server/src/http/owner_intraday_quotes.rs#L270-L459) is the
implementation of the frozen C3 table in the [contract](../superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md).
Apply the table in this order; do not infer a more permissive state from a cache row:

| Condition | State | Session / quote result | Observable reason |
| --- | --- | --- | --- |
| Feature disabled | `UNKNOWN` | No session; no quote / `UNAVAILABLE` | `FEATURE_DISABLED` |
| Calendar missing, stale, invalid, or KST date rollover | `UNKNOWN` | No session; no quote / `UNAVAILABLE` | `CALENDAR_UNAVAILABLE` |
| Window missing, invalid, or disagrees with the calendar | `UNKNOWN` | No session; no quote / `UNAVAILABLE` | `SESSION_WINDOW_UNAVAILABLE` |
| Both proofs valid and session is closed | `CLOSED` | Validated session; no quote / `UNAVAILABLE` | `SESSION_CLOSED` |
| Trading day, valid window, outside its interval | `CLOSED` | Validated session; last good quote only if age-usable | `SESSION_CLOSED` |
| Inside a valid interval and the admitted instrument is halted | `HALTED` | Validated session; last good quote only if age-usable | `INSTRUMENT_HALTED` |
| Inside a valid interval and not halted | `OPEN` | Validated session; age-usable quote or no quote | See priority below |

For `OPEN`, reason priority is: `NO_ACTIVE_DEMAND`; an eligible typed provider failure;
`QUOTE_PENDING` when no quote has succeeded; `QUOTE_STALE` when the last good quote is beyond
the recent threshold; otherwise no reason. Age is independent of state: `RECENT` is age `<=30s`,
`STALE` is age `>30s`, and a logical quote is usable only through `24h`. A future, regressing,
invalid, or over-24-hour timestamp is not usable and fails closed; a future timestamp produces
`UNKNOWN` / `PRODUCER_UNAVAILABLE` rather than a fabricated fresh quote.

The typed provider reasons are exactly `PROVIDER_TIMEOUT`, `PROVIDER_RATE_LIMITED`,
`PROVIDER_UNAVAILABLE`, `PROVIDER_RESPONSE_INVALID`, `QUOTE_VALUE_INVALID`, and
`QUOTE_BUDGET_EXHAUSTED`. The remaining typed reasons are `PRODUCER_UNAVAILABLE`,
`CALENDAR_UNAVAILABLE`, `SESSION_WINDOW_UNAVAILABLE`, `SESSION_CLOSED`,
`INSTRUMENT_HALTED`, `NO_ACTIVE_DEMAND`, `QUOTE_PENDING`, `QUOTE_STALE`, and
`FEATURE_DISABLED`. A `503` is reserved for database/infrastructure cache failure,
`QUOTE_CACHE_UNAVAILABLE`; it is not a provider-response shortcut. Authentication and ownership
checks precede input and lookup; invalid input is `400`, unauthenticated is `401`, a non-owner is
`403`, and unknown/foreign/disabled or generation-mismatched identity is `404`.

## Session evidence and configuration

Intraday eligibility requires two independent proofs:

1. The exact current KST date in `trading_calendars` and immutable
   `trading_calendar_versions`, with exchange `KRX`, timezone `Asia/Seoul`, matching batch and
   hash, source `kis`, source version `kis-chk-holiday-v1:schema-1`, and retrieval no older than
   `36h`. This proof is reused for the daily run; the quote loop does not call or paginate
   `chk-holiday`. See the [calendar and identity repository](../../crates/job-queue/src/owner_equity_v2/intraday.rs#L1234-L1304)
   and [calendar lineage checks](../../crates/job-queue/src/owner_equity_v2/intraday.rs#L3002-L3065).
2. The commit-pinned, provider-free
   `configs/market-hours/krx-intraday-session-windows-v1.json`, whose whole-file SHA-256 must
   equal `OWNER_INTRADAY_SESSION_WINDOWS_SHA256`. Each entry has the same KST civil date,
   timezone, explicit open/close, `REGULAR`, `SPECIAL`, or `CLOSED` disposition, and official
   KRX evidence URL/retrieval/hash. `REGULAR` is exactly `09:00` to `15:30`; `SPECIAL` is a
   positive same-date half-open interval; `CLOSED` has null times. See the [window contract](../../configs/market-hours/krx-intraday-session-windows-v1.schema.json#L1-L140)
   and [runtime window validation](../../data-pipelines/collectors/src/intraday_quotes.rs#L123-L218).

The calendar freshness rule and the same-KST-date/nonfuture evidence rule are separate. Evidence
retrieved on the prior KST date is stale even if only one second old; a UTC date difference is
acceptable when both instants are on the same KST civil date. Runtime must not retrieve, infer
weekday hours, use a historical XKRX substitute, invent dates, or restamp evidence. Missing,
stale, malformed, unpinned, conflicting, or otherwise invalid/out-of-contract evidence yields
`UNKNOWN` and zero provider calls. Valid proof outside the trading interval is not invalid
evidence: it yields `CLOSED`, with zero quote calls. The checked-in [window artifact](../../configs/market-hours/krx-intraday-session-windows-v1.json#L1-L6)
has intentionally empty `entries`; it is an empty/default-off artifact, not evidence for a live
date. EOD behavior is unchanged when this intraday seam is unavailable.

The default configuration is explicit and conservative:

| Setting | Accepted/runtime rule |
| --- | --- |
| `OWNER_INTRADAY_QUOTES_MODE` | `off` or `owner_only`; default `off`; only `owner_only` starts the daemon producer |
| `KIS_READ_COORDINATION_MODE` | `legacy` or `shared_required`; `owner_only` requires `shared_required` |
| `KIS_READ_CREDENTIAL_GENERATION` | In `shared_required`, positive canonical decimal, at most uint64 max `18446744073709551615` |
| `LAGRANGE_RUNTIME_STATE_DIR` | In shared mode, an explicit canonical absolute host root; no fallback path |
| `OWNER_INTRADAY_SESSION_WINDOWS_SHA256` | In `owner_only`, exact `sha256:` plus 64 lowercase hex characters and the actual whole-file hash |

The [environment example](../../deploy/compose/.env.example#L25-L40) is blank/default-off by
design. The protected parent `LAGRANGE_RUNTIME_STATE_DIR` is owned by `0:10001`, mode `0750`;
its fixed coordination leaf `${LAGRANGE_RUNTIME_STATE_DIR}/kis-read-coordination` is owned by
`10001:10001`, mode `0700`, and is mounted at `/run/lagrange/kis-read-coordination`. The lock,
state, and every temporary state file are regular non-symlinks owned by `10001:10001`, mode
`0600`, with link count one. The [provisioner leaf check](../../scripts/ops/provision-linux.sh#L334-L337)
and [validator metadata checks](../../scripts/ops/validate-production-config.sh#L495-L545)
enforce canonical absolute paths, protected-tree overlap rejection, and these ownership/mode
checks. Missing roots, invalid proofs, unsafe overlaps, bad modes, or generation/verifier
mismatches fail closed. Do not chmod, chown, symlink, repoint, delete, or otherwise repair state
as an ad hoc quota reset; rotation and rollback are gated procedures.

The shared credentialed service set is exactly:

`research-worker`, `research-range-raw`, `research-action-range-raw`,
`research-stock-price-beta-raw`, and `owner-equity-v2-runner`.

Those five use the same coordination bind and explicit `KIS_READ_COORDINATION_MODE`,
`KIS_READ_CREDENTIAL_GENERATION`, and `OWNER_INTRADAY_QUOTES_MODE` settings. Of those five,
only `owner-equity-v2-runner` receives the session-window file as a read-only bind and its hash
pin. `api-server` receives only the read-only session-window bind plus the intraday mode/hash
settings; it receives no coordination bind. No other service receives this state, credential
scope, or window. Web receives only the default-off server-side mode, not a build argument or public variable. The exact bind wiring is in [compose.intraday.yml](../../deploy/compose/compose.intraday.yml#L1-L118).

## Preparation overlay and activation gates

The installed [release wrapper](../../scripts/ops/compose-release.sh) selects the fixed
`compose.intraday.yml` only for `--scope release` when the validated, parsed coordination mode
is `shared_required`. Compose order is base, this optional overlay, then the immutable image-ID
and build-reset override. Default-off/legacy releases omit the overlay; no arbitrary overlay
path or alternate source-checkout activation is supported. The base [Web environment](../../deploy/compose/compose.yml#L113-L123)
passes only `OWNER_INTRADAY_QUOTES_MODE`, default `off`, at server runtime.

This selection support is not permission to activate. Existing standalone Raw/backfill/daily
wrappers do not select this overlay: they must not run alongside shared-mode readers until
their invocation paths are reviewed and made consistent. No manual compose-up or alternate
installer is authorized here. Fake-Docker tests establish wrapper selection/order and guard
behavior, not actual engine merging, all-reader coordination, or production readiness.

Before any separately approved activation, an owner/operator review must establish all of the
following without treating this document as authorization:

- separate owner approval for live owner-only polling; verify the already-settled private
  market-data entitlement reference and scope without reopening or requesting reapproval;
- actual engine merge/interpolation, approved installed-release/all-reader wiring, and no
  legacy reader left on a divergent bind, generation, or mode;
- all-reader drain before credential rotation, mount and generation consistency, and exact
  current-KST-date evidence plus its immutable hash;
- migration state, recovery/rollback path, entitlement scope, forbidden-path audit, and the
  no-order/no-account/read-only boundary;
- daily attempt counters, coordination-ledger continuity, lease/renewal behavior, and a
  reviewed rollback plan.

Rotation must atomically replace the shared credential state only after all readers drain;
increment generation; and restart the approved readers so old scopes fail closed. Turning the
feature off and restoring the legacy mode is likewise a reviewed rollback, with the coordination
ledger preserved. Never delete state or reset counters to recover quota. These gates are
requirements only; they have not been executed or verified here.

The runtime bounds are part of the contract and must remain unchanged: one in-flight read under
the shared OS lock; at least `1s` global/channel spacing and `5s` intraday spacing; a `3s`
per-attempt quote deadline; at most three total attempts with bounded `Retry-After` handling;
`5,000` quote GET attempts per credential/live-host/path/TR/KST date (the regular `09:00` to
`15:30` target is `4,680` slots); twenty active consumers across at most five identities;
`30s` demand leases with `15s` renewal; and a `20s` producer lease with `5s` heartbeat.
Halted instruments use the `60s` target rather than the normal `5s` target. These are bounds,
not an SLA or permission to activate.

## Offline acceptance commands

Run only the provider-free checks below when the bounded C1 validation is requested:

```bash
bash scripts/ops/stock-beta-intraday-static-check.sh
bash scripts/ops/stock-beta-intraday-self-test.sh
```

The first command checks the empty artifact, closed schema, exact environment defaults, the
provision/validator safety hooks, script syntax, and the preparation overlay contract. It does
not run a provider, read credentials, contact a network, invoke an engine, or claim runtime
semantics. The second command runs its mutation cases only in throwaway `mktemp` fixtures (using
an available user-namespace/fakeroot guard); it checks fail-closed path, mode, ownership, lock,
sentinel, idempotence, and contract mutations. It also does not install dependencies or touch
operational state. A missing `python3`, namespace/fakeroot capability, or other prerequisite is
reported as a bounded validation failure; do not substitute dependencies or provide links.

The top-level [operations static checker](../../scripts/ops/static-check.sh#L507) invokes the
Stock static check once before its success marker. It intentionally does not nest the full Stock
self-test or repeat the broader seven-suite/API/Rust validation. Passing these offline checks is
source/fixture evidence only, not product or WP6 completion.

## Safety and evidence handling

Never print, log, or persist to diagnostics an App Key, App Secret, account identifier, response
body, coordination state contents, verifier, or free-form broker/provider message. The shared
coordination state persists the actual reusable bearer access-token value with its expiry,
counters, generation, verifier, and other coordination metadata; see [PersistedToken](../../crates/kis-client/src/read_coordination.rs#L1220)
and [PersistedState](../../crates/kis-client/src/read_coordination.rs#L1254). Treat the entire
coordination file and token as secret: never dump it, log it, or persist a copy in diagnostics.
It is not a response cache. Raw and Curated data rights remain owner-confirmed and private. No order,
correction, cancellation, reservation, balance, account, WebSocket, live profile, provider
network, Docker/Compose activation, database operation, root/sudo operation, deployment, or
host-clock procedure is authorized by this runbook.

Actual engine merge/interpolation, all-reader invocation coverage, same-day evidence,
npm regeneration, and runtime/live semantics remain separate deployment/activation gates.
