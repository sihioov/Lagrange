# Stock Beta intraday current quotes

Status (2026-09-19): source review and integrated local QA passed; production activation is not yet verified.
The owner authorized deployment, owner-only polling, and the operational session-artifact
contract in this work session. This runbook records the implementation, not a production health claim. The implementation is bounded by the [intraday quote contract](../superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md)
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
   `36h`. The quote loop does not call or paginate `chk-holiday`. Calendar-only publication and
   current-day EOD use one durable calendar acquisition under the same day lock: the explicit
   `--calendar-source-batch-id <UUID>` mode either owns that acquisition or replays that exact
   UUID, while `--reuse-existing-source` is reuse-only and never captures. Reuse validates the
   exact provider/market/date/mode/entitlement, nonfuture same-day retrieval time, one calendar
   file, no continuation/copy lineage, and the stored bytes/hash before publishing. See the
   [calendar and identity repository](../../crates/job-queue/src/owner_equity_v2/intraday.rs#L1234-L1304)
   and [calendar lineage checks](../../crates/job-queue/src/owner_equity_v2/intraday.rs#L3002-L3065).
2. A provider-free session artifact. With `OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=release_v1`
   (default), use the existing commit-pinned
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
date. Invalid session evidence does not enable quote production. The calendar bootstrap and EOD
path enforce the single-acquisition/reuse contract described below.

The default configuration is explicit and conservative:

| Setting | Accepted/runtime rule |
| --- | --- |
| `OWNER_INTRADAY_QUOTES_MODE` | `off` or `owner_only`; default `off`; only `owner_only` starts the daemon producer |
| `KIS_READ_COORDINATION_MODE` | `legacy` or `shared_required`; `owner_only` requires `shared_required` |
| `KIS_READ_CREDENTIAL_GENERATION` | In `shared_required`, positive canonical decimal, at most uint64 max `18446744073709551615` |
| `LAGRANGE_RUNTIME_STATE_DIR` | In shared mode, an explicit canonical absolute host root; no fallback path |
| `OWNER_INTRADAY_SESSION_WINDOWS_SOURCE` | `release_v1` (default) or `operational_v1`; unknown/empty values fail closed |
| `OWNER_INTRADAY_SESSION_WINDOWS_SHA256` | Required for `release_v1`: exact whole-file `sha256:` plus 64 lowercase hex; operational mode reads the protected activation pin |

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

Those five use the same coordination bind, mode, and credential generation. The API has no
coordination bind or KIS secret. Web receives the server-side feature mode only. The shared
[Compose selector](../../scripts/ops/lib/kis-read-compose.sh) is used by the installed release,
daily/backfill, range, action, and stock-price wrappers. Order is base, shared intraday overlay,
optional operational overlay, then the immutable image-ID/build-reset override. Legacy/off
omits the overlays; shared mode must not coexist with a legacy reader using divergent state.

## Operational day evidence and manual activation

`operational_v1` reads only `/run/lagrange/intraday-session-windows/activation.json` with exactly
`schema_version: 1` and `window_sha256`. The hash selects `windows-<64 lowercase hex>.json` in
that directory; no path is accepted from the descriptor. The leaf is `0:10001` mode `0750`;
regular single-link files are `0:10001` mode `0640`. Ancestors must be root-owned and not writable
by group/others. Symlinks, special files, oversized/malformed content, bad hashes, and stale
same-day evidence fail closed. Both API and runner use the common provider-free loader.

The [operational overlay](../../deploy/compose/compose.intraday-operational.yml) mounts the leaf
read-only to exactly `research-worker`, `api-server`, and `owner-equity-v2-runner`. The first
mount supports only the explicit calendar command; it does not schedule a new collection job.
The existing release artifact binds remain present but are unused by the operational loader.

Prepare an input with an actual current-KST-day official KRX observation, retrieval instant,
and original-byte hash. Never restamp yesterday's evidence or infer holiday status from regular
hours. The installer validates before mutation, fsyncs a new immutable file, and atomically
replaces activation last. Existing content-addressed files are retained. With the explicitly
configured protected runtime root and a prepared input, run as root:

```bash
/opt/lagrange/current/scripts/ops/install-intraday-session-window.py --apply \
  --root "$LAGRANGE_RUNTIME_STATE_DIR/intraday-session-windows" \
  --date YYYY-MM-DD --input /absolute/path/to/verified-current-day-window.json
/opt/lagrange/current/scripts/ops/install-intraday-session-window.py --check \
  --root "$LAGRANGE_RUNTIME_STATE_DIR/intraday-session-windows" --date YYYY-MM-DD
```

Daily evidence renewal is manual. This change does not automate KRX browsing. The installer
requires the explicit date to equal the current KST date. The immutable installed release must
already select owner-only quotes, shared coordination, operational windows, and Owner V2 mode.
Drain legacy readers before activating shared mode; preserve the token/coordination ledger and
all counters. Do not rotate credentials or reset quota as part of this procedure.

## Calendar-only publication and refresh

The installed wrapper provides `--bootstrap-intraday-calendar` with one retained
`--calendar-source-batch-id` UUID. The research daemon must already be stopped/absent and the
daily timer inactive. The command verifies the installed immutable manifest, then runs exactly
one `research-worker --calendar-once` using `research_writer`, shared KIS coordination, and the
same day proof. This is independent of historical price curation:

```bash
/opt/lagrange/current/scripts/ops/compose-release.sh --scope release \
  --bootstrap-intraday-calendar --calendar-source-batch-id <retained-lowercase-UUID> --preflight
/opt/lagrange/current/scripts/ops/compose-release.sh --scope release \
  --bootstrap-intraday-calendar --calendar-source-batch-id <same-retained-lowercase-UUID> --apply
/opt/lagrange/current/scripts/ops/compose-release.sh --scope release --refresh-intraday --apply
```

Replace placeholders with real values; they are not runnable defaults. Existing Owner V2
rollout gates remain in force. No direct source-checkout activation or manual Compose-up is an
alternate release path. Refresh verifies current API/runner image IDs and revisions before
recreating only those two services sequentially, without building or starting other services.

The [calendar bootstrap](../../data-pipelines/collectors/src/calendar_bootstrap.rs) consumes a
durable date attempt before the sole `chk-holiday` GET. It validates and commits one dedicated
calendar Raw source, derives a canonical single-file calendar publication, and publishes the
exact Raw lineage. The current-day EOD path calls the same locked source resolver. When that
source exists, EOD fetches only bars, reference, and corporate-actions and copies the calendar
file into the EOD batch with its original source UUID, request metadata, bytes/hash, and
retrieval time. The copied-file lineage is explicit; the calendar provider call is not repeated.
General EOD publication still requires all four files.

The two operator modes are intentionally distinct:

- `--calendar-source-batch-id <UUID>` is the explicit acquisition/replay mode. A new UUID may
  claim and capture only when no source or claim exists for that KST date; a matching committed
  source and claim are replayed, and any mismatch fails closed.
- `--reuse-existing-source` is reuse-only. It requires the durable day claim and committed
  calendar source to resolve to the same UUID, then revalidates the immutable source and
  republishes it without a KIS call or recapture.

Claim-only state is indeterminate (`CALENDAR_ATTEMPT_INDETERMINATE`), a missing source in reuse
mode is `CALENDAR_EXISTING_SOURCE_MISSING`, and UUID/source mismatches, multiple sources,
invalid lineage, or malformed Raw are typed failures such as
`CALENDAR_ATTEMPT_ID_CONFLICT`, `CALENDAR_MULTIPLE_SOURCES`, and
`CALENDAR_SOURCE_IDENTITY_INVALID`. An interrupted or failed attempt never triggers a second
calendar acquisition; the operator must reconcile the exact existing source/claim or fail the
run. No mode may recapture to work around an indeterminate state. The research daemon and timer
still follow the stop/absence precondition for the standalone calendar command.

A real Owner-added READY membership and matching generation/admission remain required. An
analysis snapshot is no longer required for the dashboard current-quote widget. If a snapshot
exists, its instrument/generation must match. Never manufacture READY or use a test session to
stand in for the owner. Acceptance requires actual demand, allowed KIS reads, validated cache
updates, and browser price/receipt-time updates; fixture success or healthy containers alone
do not establish that acceptance.

The runtime bounds are part of the contract and must remain unchanged: one in-flight read under
the shared OS lock; at least `1s` global/channel spacing and `5s` intraday spacing; a `3s`
per-attempt quote deadline; at most three total attempts with bounded `Retry-After` handling;
`5,000` quote GET attempts per credential/live-host/path/TR/KST date (the regular `09:00` to
`15:30` target is `4,680` slots); twenty active consumers across at most five identities;
`30s` demand leases with `15s` renewal; and a `20s` producer lease with `5s` heartbeat.
Halted instruments use the `60s` target rather than the normal `5s` target. These are operational bounds, not a latency SLA.

## Offline acceptance commands

Provider-free regression commands:

```bash
bash scripts/ops/stock-beta-intraday-static-check.sh
bash scripts/ops/stock-beta-intraday-self-test.sh
bash scripts/ops/kis-read-compose-self-test.sh
python3 -m unittest discover -s scripts/ops -p test_install_intraday_session_window.py
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

Elapsed cache quiescence is not proof that provider attempts stopped. After an Owner demand is
released, the runtime producer ledger remains the required operational check: inspect the
owner-scoped producer lease/fence and heartbeat state together with the cache's last-attempt
fields, and establish that no lease or attempt began after the demand release. A quiet cache,
an expired cache age, or an unchanged quote version alone cannot establish provider-attempt
quiescence. Record only the sanitized state/result; never copy token, credential, request, or
provider-response contents into the evidence.
