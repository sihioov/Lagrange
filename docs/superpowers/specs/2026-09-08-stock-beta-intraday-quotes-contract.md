# Stock Beta intraday current-quote contract

Date: 2026-09-08 (Asia/Seoul)

Status: WP-2 shared coordination and WP-5 fixture Web integrated; owner approved the seven-field parser and fixture tests on 2026-09-08; production activation remains unapproved

Baseline: `b5f32abf833f86de5dc46fc0ba7fd88923fcbaf8`

Applies to: WP-2 through WP-8 of the [execution plan](../plans/2026-09-08-stock-beta-intraday-quotes.md)

## 1. Decision and safety boundary

V1 is an owner-only, periodically polled **REST current quote** for the selected, admitted Stock
Beta V2 membership on `/stock-beta` and `/stock-beta/[instrument]`. It is not tick streaming,
does not promise zero latency, and must be labelled `장중 현재가 · 주기적 조회`. The only broker
request is the already allowlisted exact pair:

- `GET /uapi/domestic-stock/v1/quotations/inquire-price`
- `tr_id: FHKST01010100`
- query: `FID_COND_MRKT_DIV_CODE=J` and the six-digit symbol derived from the admitted
  `{owner, membership, generation, instrument}` identity
- blank continuation input and no continuation follow-up

`J` is deliberately fixed. `NX`, `UN`, another host/path/TR, an ETF-specific endpoint, a generic
proxy, WebSocket, account or order data, and the Compose `live` profile remain forbidden. An ETF
is eligible only when it is already the selected, READY V2 membership and passes this same KRX
contract. ETF11 membership is neither implied nor created.

The private-single-owner entitlement in [ADR-0005](../../decisions/0005-kis-personal-use-entitlement.md)
is settled and is not reopened. Quote data stays inside the authenticated owner's private product.
It is never put in a public/demo response, browser persistence, logs, metrics labels, Raw/Curated
publication, or an invited Member/admin surface.

The new path is deliberately independent from immutable EOD bars, indicators, ranks, signal
snapshots, generations, and Raw publication. A current quote never appends or replaces an EOD
candle, becomes a close, triggers signal computation, changes a rank, or advances a snapshot.

## 2. Evidence and identified gaps

### 2.1 Official sources checked

All public documentation below was read on 2026-09-08. No broker endpoint or official sample was
executed.

| Evidence | Revision/pin available | Contract evidence |
| --- | --- | --- |
| KIS [current-price example](https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/inquire_price/inquire_price.py) | `main`, file says `Created on 20250112`; Git commit SHA was not exposed by the rendered page | exact path, `FHKST01010100`, `J`/`NX`/`UN` market selector, six-digit input, and explicit advice to use WebSocket for realtime |
| KIS [field mapping](https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/inquire_price/chk_inquire_price.py) | same branch/access date; Git commit SHA not verified | `stck_prpr`, `prdy_vrss`, `prdy_vrss_sign`, `prdy_ctrt`, `stck_sdpr`, `stck_shrn_iscd`, `iscd_stat_cls_code`, and `temp_stop_yn` names |
| [KIS developer portal](https://apiportal.koreainvestment.com/apiservice) | live dynamic portal, checked 2026-09-08 | REST current-price catalog and separation from realtime WebSocket |
| owner-supplied `docs/kis_openapi_entiredocs_20260818_030007.xlsx` | SHA-256 `993672501204722da88ebc30753d73406b33b23b83db8ac6670e4d83903fbac3`; workbook date in filename 2026-08-18 | sheet `주식현재가 시세` (`v1_국내주식-008`): GET/path/TR/query, string response fields and example; another workbook sheet supplies the sign-code legend |
| KRX [current trading-hours page](https://global.krx.co.kr/contents/GLB/06/0602/0602020204/GLB0602020204T1.jsp) | current public page checked 2026-09-08; no immutable revision | regular stock-market trading is 09:00-15:30; pre/post-market are separate |

The official GitHub mapping intentionally leaves numeric conversion empty, and the XLSX declares
these output values as strings. JSON numbers are therefore not accepted as a convenience. The
XLSX example also omits some fields marked required by its schema, so V1 validates the explicit
whitelist below rather than requiring the entire output object.

The current-price sheet has no business date, session date, last-trade time, exchange sequence, or
previous-close date. `stck_sdpr` is labelled **stock base price**, not previous close. V1 must not
invent any of those meanings.

The `prdy_vrss_sign` legend (`1` upper limit, `2` rise, `3` flat, `4` lower limit, `5` fall) was
found elsewhere in the same official workbook, not in the current-price sheet itself. That is
corroborating evidence, not a direct field definition. It is included in the response-contract
approval gate in section 12.

### 2.2 Repository evidence

| Fact | Source anchor |
| --- | --- |
| exact pair is in the deny-by-default read allowlist | `crates/kis-client/src/market_data.rs:19-54` |
| generic client validates HTTP/JSON/`rt_cd` then returns bytes | `crates/kis-client/src/market_data.rs:101-203` |
| process-local token mutex and issue guard | `crates/kis-client/src/auth.rs:52-137` |
| process-local per-channel buckets | `crates/kis-client/src/rate_limit.rs:73-139` |
| current generic read retry is four total attempts, so it is not the V1 quote policy | `crates/kis-client/src/retry.rs:22-39` |
| live transport accepts an explicit whole-request timeout | `crates/kis-client/src/live_transport.rs:120-143` |
| provider uses `J`, exact path/TR and six-digit symbol | `crates/market-data/src/providers/kis.rs:59-64`, `crates/market-data/src/providers/kis.rs:144-170`, `crates/market-data/src/providers/kis.rs:248-265` |
| current shape validation requires only `output` object | `crates/market-data/src/providers/kis.rs:1303-1360` |
| existing normalizer consumes current-price only as ETF11 identity evidence | `crates/market-data/src/normalize.rs:591-679` |
| KIS daily calendar normalizer emits one target observation and hard-coded regular times | `crates/market-data/src/normalize.rs:727-849` |
| published calendar projection stores date/type/provenance, not per-date open/close | `migrations/0022_research_publication.up.sql:26-53` |
| bundled XKRX artifact is historical-only and ends 2026-08-28 | `data/calendars/xkrx/manifest.json:5-37` |

There is consequently no current, production-ready, date-specific special-session window proof in
the baseline. A KIS `TRADING` day plus a wall clock is insufficient to prove that a particular day
has ordinary hours. Section 7 freezes fail-closed behavior and the exact evidence seam; it does
not pretend this missing operational input already exists.

## 3. Current-price response contract

The parser is a new `intraday_quotes` parser. It must not widen
`normalize_reference`, `ResponseKind::Reference`, or any Raw publication schema.

### 3.1 Required envelope and identity

The transport must already have established HTTP 2xx and an allowlisted request. The parser then
requires:

1. a top-level JSON object;
2. string `rt_cd` exactly `"0"`;
3. top-level `output` exactly one object;
4. no continuation request, regardless of a response marker;
5. string `stck_shrn_iscd` exactly equal to the requested six-digit symbol; and
6. the membership still READY, the exact admitted generation still current, the producer fence
   still held, and the session evidence still identical immediately before DB publication.

Malformed JSON, provider failure, a missing/duplicate/wrong-type field, identity mismatch, an
unknown sign code, or a failed publication fence rejects the new observation. The response body
and provider prose must not be logged or stored.

### 3.2 Parsed fields and numeric grammar

| KIS field | V1 meaning | Validation and storage |
| --- | --- | --- |
| `stck_shrn_iscd` | returned identity | exactly six ASCII digits and equal to request |
| `stck_prpr` | current price | required canonical decimal string, `> 0`, precision at most 20 and scale at most 8 |
| `prdy_vrss` | change from previous day | required signed canonical decimal string, precision 20/scale 8 |
| `prdy_ctrt` | percent change from previous day | required signed canonical decimal string, precision 20/scale 8 |
| `prdy_vrss_sign` | provider direction/limit classification | required one of `1..5`; mapped below and checked against both signed values |
| `stck_sdpr` | provider stock base price | required canonical decimal string `> 0`; exposed only as `base_price`, never as previous close |
| `iscd_stat_cls_code` | security status classification | required two or three ASCII digits; exact `58` is halted, all other values are neither interpreted nor exposed |
| `temp_stop_yn` | temporary-stop flag | required `Y` or `N`; `Y` is halted |

Canonical decimal strings match `-?(0|[1-9][0-9]*)([.][0-9]{1,8})?`, reject leading `+`, commas,
whitespace, exponent notation, `-0`, NaN/infinity, and excess precision/scale, and are converted to
PostgreSQL `numeric(20,8)`. API and Web continue to exchange decimal **strings**; JavaScript
floating point is never the source of truth.

The sign mapping is exact:

| Code | DTO direction | Consistency rule |
| --- | --- | --- |
| `1` | `LIMIT_UP` | amount and percent must both be positive |
| `2` | `UP` | amount and percent must both be positive |
| `3` | `FLAT` | amount and percent must both be exactly zero |
| `4` | `LIMIT_DOWN` | amount and percent must both be negative |
| `5` | `DOWN` | amount and percent must both be negative |

The signed strings are used as supplied; the sign code is not applied a second time. V1 does not
recompute one value from another because provider rounding rules are not documented. A zero or
empty current price is `QUOTE_VALUE_INVALID`, not proof of no trade. The reviewed fields do not
prove a no-trade state, so V1 has no `NO_TRADE` classification.

`HALTED` takes precedence over `OPEN` when either the exact status code is `58` or
`temp_stop_yn=Y`. A halted instrument is probed no more frequently than once per 60 seconds so a
resume can be detected without spending the five-second budget. Unknown/malformed stop values
reject the observation.

### 3.3 Time semantics

`received_at` is captured from the worker clock immediately after the complete response bytes are
received and before parsing. It becomes visible only if parsing and fenced publication succeed.
It is a transport receipt time, not a quote creation time or last-trade time.

V1 has no `provider_trade_at`, provider sequence, previous-close date, or derived previous-close
field. The API session date comes exclusively from section 7's session evidence and is not read
from the quote payload. A successful observation sets `last_success_at=received_at`; a failure
updates only typed attempt status and never advances either time.

## 4. Complete credentialed construction and call map

The production code has four non-test market-data constructor implementations reached through the
six lifecycles below. All must use the shared boundary before intraday mode may be enabled.

| Executable path | Construction | Actual call path/lifecycle |
| --- | --- | --- |
| `research-worker` daemon and `--once` | `data-pipelines/collectors/src/worker.rs:876-938`, constructed for credentialed internal ingest at `data-pipelines/collectors/src/worker.rs:2462-2496` | daemon/once calls `WorkerBackend::ingest`, which spawns `__research-internal-ingest` at `data-pipelines/collectors/src/worker.rs:1217-1237`; CLI dispatch is `data-pipelines/collectors/src/bin/research-worker.rs:194-208`, `data-pipelines/collectors/src/bin/research-worker.rs:577-582` |
| `research-worker --backfill-session-dates` | `data-pipelines/collectors/src/worker.rs:1385-1465`, constructor at line 1412 | CLI at `data-pipelines/collectors/src/bin/research-worker.rs:405-435`; one process reuses its client across dates |
| `research-worker --range-raw` | `data-pipelines/collectors/src/worker.rs:1487-1533`, constructor at lines 1518-1521 | CLI at `data-pipelines/collectors/src/bin/research-worker.rs:438-479`; provider-free existing-source replay constructs no client |
| `kis-action-range-raw --execute` | `data-pipelines/collectors/src/bin/kis-action-range-raw.rs:394-429` | execute constructs/calls at `data-pipelines/collectors/src/bin/kis-action-range-raw.rs:302-324` |
| `kis-stock-price-beta-raw --execute` | `data-pipelines/collectors/src/bin/kis-stock-price-beta-raw.rs:429-460` | execute constructs/calls at `data-pipelines/collectors/src/bin/kis-stock-price-beta-raw.rs:323-339` |
| `owner-equity-v2-runner` daemon and `--once` | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:399-427`, startup at lines 598-612 | one-instrument KIS provider and capture at `crates/job-queue/src/owner_equity_v2/runtime.rs:286-301`; reference then daily-window calls at `data-pipelines/collectors/src/owner_equity_v2.rs:42-113` |

`crates/api-server/src/live_order.rs:419-465` constructs a separate order-capable `RestClient` when
a non-dry-run order path is enabled. It is not a market-data read caller and WP-2 must not alter or
reuse it. Intraday activation must instead prove the `live` profile/order path is disabled; no
claim of shared read quota is valid while an independently token-issuing live path can coexist.

Test-only constructors and `crates/kis-client/src/rest.rs` are not production read callers. Health
checks and provider-free existing-source/materialization paths do not construct a KIS client.

## 5. Process-shared credential arbitration

Adding another `Arc` is insufficient: the current mutexes are process-local. WP-2 shall add one
opt-in, fail-closed read coordinator inside `kis-client` and connect every production constructor
in section 4.

### 5.1 Protected state and credential equivalence

- Production coordination directory: `/run/lagrange/kis-read-coordination`, bind-backed for every
  credentialed read container by the same absolute host leaf
  `${LAGRANGE_RUNTIME_STATE_DIR}/kis-read-coordination`, owned `10001:10001`, mode `0700`.
  `LAGRANGE_RUNTIME_STATE_DIR` has no permissive/default production fallback.
- Stable `coordination.lock` is a regular, non-symlink file with link count one and mode `0600`.
  Canonical state is `state-v1.json`. State and
  same-directory temporary files are `0600`; atomic replace is `write -> fsync file -> rename ->
  fsync directory` while the stable lock inode remains locked.
- The state contains the reusable access token and expiry, last token-attempt time, global and
  per-channel attempt times, broker cooldown, KST daily quote-attempt ledger, credential generation,
  and a credential verifier. It contains no response body, request query, App Key, App Secret,
  account value, or provider prose.
- The verifier is `HMAC-SHA256(key=resolved trimmed App Secret UTF-8 bytes,
  data="lagrange-kis-read-v1\0" || resolved trimmed App Key UTF-8 bytes)`. Computing from values,
  rather than environment/file references, prevents aliases and duplicated secret files from
  creating separate quota identities. The verifier is never emitted in a path, log, metric, error,
  or API. All production readers use one configured `KIS_READ_CREDENTIAL_GENERATION` integer and
  the one canonical directory.
- A verifier mismatch at the same/lower generation is `KIS_READ_CREDENTIAL_SCOPE_MISMATCH` and
  sends zero network requests. Rotation is drain-all-readers, atomically replace every protected
  credential copy, increment the generation, then restart. The first higher-generation holder
  clears the token but retains conservative issue/rate/cooldown/daily counters; older processes
  fail closed. There is no unguarded fallback.

The protected token state is permitted only because it is a dedicated runtime secret store with
the checks above. A PostgreSQL application table, Redis, browser storage, Raw root, ordinary JSON
config, unlocked file, or diagnostics output is forbidden.

### 5.2 Lock and crash protocol

One OS advisory exclusive lock covers token decision/issuance and every market-data HTTP request.
The read attempt is reserved and durably counted **before** bytes can leave the process; the lock is
held across the request. Thus all read processes have one request in flight globally. A crash
releases the kernel lock but leaves the conservative reservation, preventing an immediate retry.

Under the lock:

1. validate directory, inode, permissions, state schema, credential generation/verifier, and
   non-regressing wall time;
2. reuse a token while it is valid beyond the existing 60-second refresh margin;
3. before token POST, persist `last_issue_attempt`; all ambiguous/failed issues retain the existing
   60-second minimum interval and no eager refresh loop is added;
4. enforce at least one second between all read attempts and at most one attempt/second for each
   exact endpoint/TR channel;
5. enforce five seconds between **intraday-class** current-quote attempts for this credential and
   count only that class in the intraday daily ledger; EOD reference reads on the same path still
   share the one-second channel limit and single in-flight lock;
6. persist a 429 `Retry-After` cooldown before releasing the lock; and
7. on a broker-authoritative 401, invalidate the shared token under the lock. Reissue still obeys
   the persisted one-minute guard.

If state is corrupt, permissions are broad, time moves behind the persisted high-water mark, the
lock cannot be obtained within the caller's deadline, or the shared mount is missing, production
returns a typed disabled/error result and performs no network request. Tests may explicitly use an
isolated temporary coordinator; production may not silently use the old in-memory-only path when
intraday mode is `owner_only`.

The production live market-data constructor requires an explicit coordinated mode. Any retained
uncoordinated constructor is test/mock-only and cannot accept `LiveTransport`; static checks cover
every site in section 4 so a future caller cannot silently bypass the gate.

The quote loop uses a non-blocking lock attempt at its scheduled slot and skips a busy slot; it does
not queue ahead of a blocking EOD/manual reader. An already-started quote can occupy the lock only
for its three-second request deadline. The quote loop never pauses, cancels, or spends the EOD
job's own request ceiling.

## 6. Demand, scheduling, retry, and budget

### 6.1 Browser demand leases

- A mounted visible/online quote widget owns one random UUID `consumer_id` in memory. It is not
  persisted across reload/logout and is regenerated when the selected identity changes.
- Server lease is exactly 30 seconds from a fresh DB clock sampled inside the mutation transaction
  after its blocking locks are acquired, not from transaction-start `now()`. Renewal is every 15 seconds.
- Mutations for one consumer are serialized. Unmount, hide, offline, logout, or selection change
  stops polling/renewal and sends a best-effort release.
- Each consumer has a separate row. Collection demand is the merge of all non-expired ACTIVE rows
  by exact `{owner, membership, generation, instrument, venue, session}`. One consumer release
  can affect only its `demand_id` and can never remove another consumer's lease.
- Hard bounds per owner: **20 active consumer leases** and **5 distinct active instrument
  identities**. Renewal of an already-active lease is allowed at the cap; a new lease/identity is
  rejected atomically with `QUOTE_DEMAND_CAPACITY` and `Retry-After: 15`.
- Expiry is authoritative even without a release. After the last lease expires, no new provider
  request may begin; an already in-flight response must recheck demand and is discarded if none
  remains.

The 20-lease limit is the explicit amendment missing from the plan's five-instrument default. It
allows several independent dashboard/detail/tab consumers while keeping rows bounded.

### 6.2 Producer placement and fairness

Only `owner-equity-v2-runner` daemon mode may run the producer, behind
`OWNER_INTRADAY_QUOTES_MODE=owner_only`. `--once` shares credential arbitration for its EOD work
but does not start the quote loop. No new service, image, Redis, broker, API egress, or API secret
is introduced.

The daemon spawns an independent quote task with its own cancellation and DB producer lease; it
must not wait for a 15-minute membership job to finish. It shares the `Arc` KIS reader, whose new
process-shared coordinator arbitrates with the job and other OS processes. A per-owner PostgreSQL
producer lease uses a monotonically increasing fencing token. Default producer lease is 20 seconds
with a five-second heartbeat; loss or expiry prohibits publication.

Eligible distinct identities are ordered by the time they were last attempted, then instrument ID,
and scheduled round-robin. One identity targets five seconds; five identities target approximately
25 seconds each. Duplicate consumers do not add turns. Halted identities target 60 seconds. These
are targets, not broker latency SLAs.

### 6.3 Attempt deadline and retry

- Quote HTTP request deadline: **3 seconds per attempt** using a dedicated transport instance.
- Quote retry policy: at most **three total attempts** (initial plus two retries).
- Every retry reacquires the shared gate and counts as a quote attempt. The five-second current
  channel spacing dominates the local 100/200 ms exponential backoff.
- `Retry-After` is authoritative. If it moves the next attempt beyond the current session, daily
  budget, demand lease, producer lease, or current retry cycle, stop the cycle and retain the
  cooldown; never shorten it to meet a UI target.
- 401 follows one shared invalidation/reissue opportunity but still cannot exceed three GET
  attempts. Schema/identity/integrity failures are not retried in the same cycle.

The plan's 11-second acceptance applies only when one active instrument's first provider attempt
starts in the next five-second slot, returns within one second, and the next non-overlapping Web
poll occurs within five seconds: `5 + 1 + 5 = 11`. It is not a failure/retry guarantee. With a
three-second timeout and two retries spaced at five-second starts, the attempt chain alone can end
13 seconds after its first start; including initial scheduler and Web phases gives up to 23
seconds. A longer broker `Retry-After` has no display-time bound and must become stale/unavailable.

### 6.4 Daily attempt budget

The hard budget is **5,000 current-quote GET attempts per resolved credential, live host, exact
path/TR, and KST civil date**. Initial calls, 401 repeats, 429s, timeouts, and all other retries
count when reserved, even if the process crashes. Token POST and other allowlisted endpoint/TR
channels do not consume this quote-specific ledger, but remain under shared serialization and
their own limits.

The supported regular half-open window `[09:00, 15:30)` is 23,400 seconds, hence
`23,400 / 5 = 4,680` normal slots. Five continuously demanded instruments share those same 4,680
attempts, about 936 first attempts each; they do not multiply the credential total. The 5,000 cap
leaves only 320 attempts beyond that regular-slot schedule and is still absolute on special or
extended sessions. Budget resets only on a non-regressing transition to the next KST date.

## 7. Session proof and market state

### 7.1 Two independent proofs

A provider attempt may begin only when both are present and agree:

1. **Trading-day proof:** exact current KST date in both `trading_calendars` and its immutable
   `trading_calendar_versions` source row, `exchange=KRX`, `timezone=Asia/Seoul`, non-null matching
   batch/hash/retrieval lineage, source `kis`, source version
   `kis-chk-holiday-v1:schema-1`, and `retrieved_at <= now` no more than 36 hours old. The approved
   daily result is reused; the quote loop never calls or paginates `chk-holiday`.
2. **Date-specific session-window proof:** a commit-pinned, owner-approved, provider-free file
   `configs/market-hours/krx-intraday-session-windows-v1.json`, passed by exact SHA-256, with an
   entry for that date, `Asia/Seoul`, explicit open/close, disposition `REGULAR`, `SPECIAL`, or
   `CLOSED`, and official KRX evidence URL/retrieval/hash metadata. No weekday/clock inference and
   no network fetch occurs at runtime.

The file's schema is closed (`additionalProperties=false`) and exact:

```json
{
  "schema_version": 1,
  "exchange": "KRX",
  "timezone": "Asia/Seoul",
  "entries": [
    {
      "date": "YYYY-MM-DD",
      "disposition": "REGULAR",
      "open_local": "09:00:00",
      "close_local": "15:30:00",
      "evidence_url": "https://global.krx.co.kr/...",
      "evidence_retrieved_at": "RFC3339 UTC",
      "evidence_sha256": "sha256:64-lowercase-hex"
    }
  ]
}
```

`entries` is sorted by unique date. `CLOSED` requires both time fields null; `REGULAR` requires
exactly 09:00:00/15:30:00; `SPECIAL` requires a positive same-date half-open interval. The whole
file hash must equal `OWNER_INTRADAY_SESSION_WINDOWS_SHA256`, and the runtime path is fixed to
`/opt/lagrange/configs/market-hours/krx-intraday-session-windows-v1.json`.

The file is a proposed WP-6 operational artifact and starts empty/default-off. Each date is an
explicit assertion so delayed openings and exceptional closures cannot silently inherit regular
hours. A regular entry uses 09:00-15:30 only with the current official KRX hours evidence; a special
entry supplies its exact reviewed window. Missing, stale, malformed, unpinned, conflicting, or
out-of-range proof yields `UNKNOWN` and **zero provider calls**.

Owner-approved clarification (2026-09-09): session-window evidence must have been checked on
the **same KST civil date** as the entry and the current evaluation date. Use the existing
`evidence_retrieved_at` as the actual evidence retrieval/check timestamp; its instant must be
`<= now`, and its `Asia/Seoul` date must equal `entry.date` and the current KST date. Prior-day
evidence is stale even if only one second old; this is neither a rolling 24-hour allowance nor
the separate calendar proof's 36-hour allowance. An entry with future or prior-day evidence
resolves to `UNKNOWN`, including a nominal `CLOSED` entry. A UTC date difference alone does not
make evidence stale when both instants are on the same KST date. At KST midnight, the preceding
date's evidence cannot authorize the new date. All existing exact hash, schema, calendar lineage,
and half-open session-window checks remain required.

This does not authorize automatic evidence retrieval or restamping old evidence. An operator
must genuinely check the official evidence on that date before producing and pinning the entry.
Absent such evidence, only intraday quote collection stays off; existing EOD collection behavior
is unchanged. Fixture implementation may proceed; actual provider activation remains separately
gated.

The baseline does not yet produce same-day pre-session KIS calendar rows reliably: the recorded
daily job is after market close, and its production recovery is incomplete. Nor does it contain the
date-specific file above. Therefore fixture implementation may proceed after the response gate,
but production intraday activation remains blocked until this seam is populated and verified. The
historical XKRX file is not a substitute.

### 7.2 Market-state precedence

`market_state` and `freshness` are independent:

1. invalid/missing proofs or clock regression -> `UNKNOWN`;
2. trading-day/session-window says closed, or now is outside the proven half-open interval ->
   `CLOSED`;
3. within the proven interval and last validated same-session quote reports an exact halt ->
   `HALTED`;
4. otherwise within the proven interval -> `OPEN`.

Only `OPEN` or a due `HALTED` probe may call KIS. `CLOSED` and `UNKNOWN` call zero times. At close,
the last same-session quote may remain visibly stale for up to the cache retention limit with an
explicit CLOSED label; it is never called a close. A new KST date cannot reuse the preceding
session's quote.

## 8. Application API contract

The actual router is mounted under `/api/v1` and the existing product prefix is visible at
`crates/api-server/src/http/mod.rs:325-357`. This replaces the plan's placeholder `/api/v2/...`
paths.

### 8.1 Literal routes

| Method and path | Purpose |
| --- | --- |
| `POST /api/v1/research/owner-beta/equity-universe-v2/quote-demands` | create or renew one consumer lease |
| `DELETE /api/v1/research/owner-beta/equity-universe-v2/quote-demands/{demand_id}` | release only that consumer lease |
| `GET /api/v1/research/owner-beta/equity-universe-v2/instruments/{instrument_id}/quote?membership_id={uuid}&generation={u64}` | cache-only current state |

All request structs use `deny_unknown_fields`. No route accepts owner ID, provider host/path/TR,
market selector, account, free-form query, lease duration, poll interval, URL, or budget override.

### 8.2 Demand mutation DTO and idempotency

POST body:

```json
{
  "schema_version": 1,
  "consumer_id": "uuid",
  "membership_id": "uuid",
  "generation": 1,
  "renewal_sequence": 0
}
```

Response is HTTP 200 for create, renew, and exact replay:

```json
{
  "schema_version": 1,
  "demand_id": "uuid",
  "consumer_id": "uuid",
  "membership_id": "uuid",
  "instrument_id": "005930.KRX",
  "generation": 1,
  "renewal_sequence": 0,
  "lease_expires_at": "2026-09-08T01:00:30Z",
  "renew_after_ms": 15000
}
```

The server derives instrument/owner from a READY membership and exact admitted current generation.
`renewal_sequence` starts at zero and must increase by one for that consumer. Both POST and DELETE
require the authenticated owner session, `X-CSRF-Token`, and an `Idempotency-Key` satisfying the
existing 1..128 visible-ASCII rule excluding colon and backslash
(`crates/api-server/src/http/owner_equity_v2.rs:968-995`). Only a digest is stored.

The most recently accepted same key, sequence, and body replays the exact prior response without
extending again. Same key with different input is `IDEMPOTENCY_MISMATCH`; a superseded, lower,
skipped, or reused sequence is `QUOTE_DEMAND_SEQUENCE_CONFLICT`. The row records the terminal
sequence on release, so a delayed renewal cannot resurrect it. A changed selection creates a new
in-memory consumer UUID and demand.

Coordinator clarification (2026-09-08, application contract only): lease expiry is not an
explicit RELEASED tombstone. While the same consumer remains mounted/visible/online and its
membership remains READY, a retained, expired ACTIVE row may be renewed with exactly the next
sequence. The transaction rechecks owner, membership/generation and both capacity limits as
a new active lease, and grants 30 seconds from its fresh post-lock DB clock. Exact replay still returns
the original expiry without extending it; resolve an ambiguous prior mutation by replaying its
same key/body/sequence before advancing. RELEASED rows never reactivate; missing/GC rows return
404 and stop that consumer without automatic identity churn. Expired ACTIVE rows remain logically
inactive and are eligible for tombstone GC after 24 hours; the worker must not convert mere expiry
into explicit release before that recovery window. WP-3/WP-4 must implement and DB-test this rule.

At browser-observed expiry, abort/fence cache GETs but retain the serialized mutation context.
Do not enqueue a concurrent mutation while one is pending. After a known successful renewal with
a future expiry, resume with a fresh GET epoch; an old pre-expiry GET cannot become valid again.
Expired exact replay advances only the known accepted sequence and schedules a new renewal.
Recovery attempts are bounded (three consecutive attempts, at least 15 seconds apart, honoring
Retry-After); exhaustion is visibly unavailable, not silent idle. Lifecycle cleanup always wins.
This clarification changes neither the KIS response contract nor production authorization.

DELETE body is:

```json
{
  "schema_version": 1,
  "consumer_id": "uuid",
  "renewal_sequence": 3
}
```

It returns 204, including exact replay, and marks that row released; it never deletes by instrument
or consumer wildcard. Unknown/not-owned ID is the same 404 as any nonexistent resource.

### 8.3 Cache GET DTO

A valid current membership identity returns HTTP 200 and `Cache-Control: no-store` even when a
quote is unavailable:

```json
{
  "schema_version": 1,
  "membership_id": "uuid",
  "instrument_id": "005930.KRX",
  "venue": "KRX",
  "currency": "KRW",
  "generation": 1,
  "session": {
    "date": "2026-09-08",
    "timezone": "Asia/Seoul",
    "calendar_source": "kis",
    "calendar_source_version": "kis-chk-holiday-v1:schema-1",
    "calendar_content_sha256": "64-lowercase-hex",
    "window_contract_sha256": "sha256:64-lowercase-hex"
  },
  "market_state": "OPEN",
  "freshness": "RECENT",
  "reason_code": null,
  "quote": {
    "price": "72500",
    "base_price": "71000",
    "change_from_previous_day": "1500",
    "change_percent_from_previous_day": "2.11",
    "direction": "UP",
    "received_at": "2026-09-08T01:00:00.125Z",
    "last_success_at": "2026-09-08T01:00:00.125Z",
    "quote_version": "17"
  },
  "next_poll_after_ms": 5000
}
```

`quote_version` is a decimal string because it is a DB monotonic `bigint`, not a provider
sequence. `quote` may be null. `session` is null when evidence is unknown. There is deliberately no
provider timestamp, previous close, EOD date/value, snapshot date/value, raw provider status, or
free-form message in this DTO.

Freshness is calculated by the API clock against `last_success_at`: `RECENT` at age `<=30s`,
`STALE` above 30 seconds when an identity- and session-matching last good remains, otherwise
`UNAVAILABLE`. Future times, a regressing version, another session/generation, or cache older than
24 hours is unavailable. Typed `reason_code` is one of:

`NO_ACTIVE_DEMAND`, `QUOTE_PENDING`, `QUOTE_STALE`, `PROVIDER_TIMEOUT`,
`PROVIDER_RATE_LIMITED`, `PROVIDER_UNAVAILABLE`, `PROVIDER_RESPONSE_INVALID`,
`QUOTE_VALUE_INVALID`, `QUOTE_BUDGET_EXHAUSTED`, `CALENDAR_UNAVAILABLE`,
`SESSION_WINDOW_UNAVAILABLE`, `SESSION_CLOSED`, `INSTRUMENT_HALTED`,
`PRODUCER_UNAVAILABLE`, or `FEATURE_DISABLED`.

### 8.4 HTTP failures and privacy order

| Status | Stable code and condition |
| --- | --- |
| 400 | `INVALID_PARAMETER`, malformed UUID/instrument/generation/sequence/body/header |
| 401 | existing unauthenticated/expired-session response, before lookup |
| 403 | `FORBIDDEN` for authenticated non-owner, or existing CSRF failure, before lookup |
| 404 | `RESOURCE_NOT_FOUND` for unknown/not-owned/disabled membership, mismatched instrument or generation, and unknown/not-owned demand |
| 409 | `IDEMPOTENCY_MISMATCH` or `QUOTE_DEMAND_SEQUENCE_CONFLICT` |
| 429 | `QUOTE_DEMAND_CAPACITY`, `Retry-After: 15` |
| 503 | `QUOTE_CACHE_UNAVAILABLE` only for DB/feature infrastructure failure |

The API checks authentication and owner role before resource access. It uses the existing actor
transaction/RLS for every query. Non-owner, invited Member, another owner, and unauthenticated
clients cannot distinguish whether a membership, demand, cache, or price exists. Repeated GETs,
including 401/403/404 requests, perform no writes, enqueue no job, and cause zero provider/token
calls. Logout/401/403 clears quote state in the Web client.

## 9. Migration 0054 and storage contract

The baseline's highest migration is 0053, so WP-3 owns exactly:

- `migrations/0054_owner_intraday_quotes.up.sql`
- `migrations/0054_owner_intraday_quotes.down.sql`

If another integration claims 0054 first, work stops for coordinator renumbering; a worker must not
choose a number independently.

### 9.1 Tables and keys

The frozen logical column map is:

| Table | Required columns and PostgreSQL types |
| --- | --- |
| `owner_intraday_quote_demands` | `id uuid PK`, `owner_user_id uuid`, `consumer_id uuid`, `membership_id uuid`, `generation_id uuid`, `instrument_id text`, `generation bigint`, `state text`, `renewal_sequence bigint`, `lease_expires_at timestamptz`, `released_at timestamptz null`, `idempotency_key_sha256 text`, `request_sha256 text`, `created_at/updated_at timestamptz` |
| `owner_intraday_quote_cache` | `owner_user_id uuid`, `membership_id uuid`, `generation_id uuid`, `instrument_id text`, `generation bigint`, `session_date date null`, `calendar_source text null`, `calendar_source_version text null`, `calendar_source_batch_id uuid null`, `calendar_content_sha256 text null`, `window_contract_sha256 text null`, `price/base_price numeric(20,8) null`, `change_amount/change_percent numeric(20,8) null`, `direction text null`, `halted boolean null`, `received_at/last_success_at timestamptz null`, `quote_version bigint`, `last_attempt_at timestamptz`, `last_failure_code text null`, `last_failure_at timestamptz null`, `producer_fence bigint`, `created_at/updated_at timestamptz`; PK `{owner_user_id,membership_id}` |
| `owner_intraday_quote_producers` | `owner_user_id uuid PK`, `holder_id uuid`, `fencing_token bigint`, `lease_expires_at/heartbeat_at/updated_at timestamptz` |

UUID ownership and lineage columns use composite foreign keys to 0053. Hash checks use
`^sha256:[0-9a-f]{64}$` for the new window/idempotency/request hashes and the existing calendar
projection's unprefixed `^[0-9a-f]{64}$` convention for `calendar_content_sha256`. Generations,
sequences, versions and fences are nonnegative, with admitted membership generation positive.
Quote numeric/halt/direction/success fields are all-null or all-present; failure code/time are both
null or both present. Session/evidence fields are all-null when proof is unavailable or all-present
and mutually consistent when proof exists. State/direction/failure values use closed CHECK sets
from this contract.

`owner_intraday_quote_demands` contains demand UUID, owner UUID, consumer UUID, membership and
admitted generation identity, state (`ACTIVE` or `RELEASED`), renewal sequence, lease expiry,
release time, latest idempotency/request digests, and timestamps. Its own fields reconstruct the
most recent idempotent response.
It has a composite FK through the 0053 admission lineage, unique `{owner_user_id, consumer_id}`,
and indexes on `{owner_user_id, state, lease_expires_at}` and the exact merge identity. Capacity is
serialized by an owner-scoped transaction advisory mutex. Coordinator amendment (2026-09-08):
use `pg_advisory_xact_lock(hashtextextended('owner-intraday-demand-cap|' || canonical_owner_uuid, 0))`
for every demand create, renew, and release before observing/modifying consumer state or capacity.
The policy row is only an existence anchor; this mutex does not serialize policy administration
or replace the existing READY/current-admission checks and publication membership locks.
The fixed 20-consumer/5-distinct-identity bounds do not derive from mutable 0053 policy values.
0053 does not write intraday demand rows. Any future path that adds/reactivates such rows must
take this same mutex; expiry and GC only reduce capacity. A hash collision may conservatively
serialize different owners, never combine their counts or actor scopes. No policy UPDATE grant
or privilege-bypass helper is added. This replaces the earlier policy-row-lock requirement,
which is incompatible with the existing SELECT-only app policy grants.

`owner_intraday_quote_cache` has one latest/status row per exact owner membership. Its composite FK
binds generation admission. It stores instrument, generation/session and both evidence hashes,
nullable all-or-none decimal quote fields, direction, halt flag, successful `received_at` and
`last_success_at`, `quote_version`, last attempt/failure code/time, producer fencing token, and
timestamps. A success increments `quote_version`; a failure changes only attempt/failure status.
For the same exact identity/session, an older receipt must return a typed stale-receipt rejection
without changing price, success timestamps, version, or failure fields. The comparison must be
atomic with the cache update; a higher version cannot make an older receipt current.
It stores no JSON provider body, previous-close claim, provider message, Raw pointer, EOD value, or
browser consumer identity.

`owner_intraday_quote_producers` contains one row per owner with holder UUID, monotonic fencing
token, lease expiry, heartbeat, and update time. Claim/takeover increments the token. Quote publish
locks this row and the membership, verifies unexpired producer lease, READY membership, current
admission generation, current active demand, exact session evidence, and budget reservation, then
upserts cache in one transaction. Disable or generation change racing an HTTP request therefore
causes discard, not a late publish.

Expiry is wall-time authoritative even after lock waits. Acquire required blocking rows (including
the cache row when present), then sample a fresh database `clock_timestamp()` and revalidate
producer/demand expiry, receipt, and current-session/evidence age before mutation. A clock expression
evaluated before a blocking row lock is not a post-lock check. Producer claim/heartbeat and demand
renewal must likewise use fresh post-lock time for eligibility and lease extension; exact demand
replay alone preserves its previously committed expiry. Do not substitute the application clock.

No stored procedure, trigger, sequence, or `SECURITY DEFINER` helper is added. The job-queue repo
uses explicit typed SQL inside transactions and treats a zero-row fenced UPDATE/UPSERT as a lost
lease. This follows the existing 0053 worker-role model without creating another privilege-bypass
surface.

Every table is owned by `migration_owner`, has ENABLE and FORCE RLS, begins with REVOKE ALL from
PUBLIC and all application roles, and uses explicit column grants:

- `app`: SELECT/INSERT/UPDATE on its actor-scoped demand rows; SELECT only on its actor-scoped
  quote cache. No producer-table access and no cache write.
- `worker`: SELECT on demand/cache/producer and the existing membership/admission/calendar inputs;
  explicit-column INSERT/UPDATE/DELETE on the three new operational tables under worker RLS.
- `admin`, `audit_writer`, `research_writer`: no grants and therefore no quote visibility.
- `migration_owner`: actor-scoped maintenance policies consistent with 0053; no broad serving API.

The app and worker policies must be tested using real role connections, not only table-owner tests.
No new login role, schema, extension, service, Redis instance, or sequence grant is required.

### 9.2 Retention and restart

- Reads reject expired demand immediately. Worker treats expired ACTIVE leases as inactive and deletes
  RELEASED/expired demand tombstones after 24 hours.
- Cache is latest-only. It is unreadable immediately after membership disable/generation mismatch
  and deleted by worker after 24 hours from the later of last attempt/success; null never-success
  rows follow last-attempt time.
- On restart, a same-session, same-generation last good may be returned only with freshness
  recomputed from wall time. It never becomes RECENT merely because the process restarted.
- New session evidence fences the old row; old-session data is not returned as current. Physical
  cleanup may lag while logical reads remain blocked.
- Producer takeover increments its fence. A response from a dead/old holder cannot publish.

These deletes affect only ephemeral quote tables. EOD/Raw/Curated/snapshot tables are never GC
targets.

## 10. Web contract and extensibility

The browser polls the cache every five seconds only while authenticated, owner-authorized,
visible, online, and mounted. It schedules the next request only after the previous one settles, so
there is no overlap. Demand renewal runs independently every 15 seconds with the serialized
sequence contract. Browser retries never call KIS directly.

Selection A -> B aborts A's cache request, releases A best-effort, creates a new consumer UUID,
and increments a local request epoch. A response is applied only when membership ID, instrument,
generation, session, and epoch still match. Hide/unmount/offline/logout removes timers and clears
the view model as appropriate.

`CurrentQuoteWidget` is one reusable presentation component registered independently in both the
dashboard and detail catalogs. A dedicated `quote-load-coordinator`/hook owns I/O and timers.
`stock-beta-workspace.tsx` and `stock-beta-detail.tsx` only compose it. No quote state machine may
be added to `chart-load-coordinator.ts`, `signal-refresh-coordinator.ts`, the EOD chart geometry,
or one monolithic workspace loop. Catalog add/remove/reorder/hide must drive consumer lifecycle.

## 11. Exact downstream file ownership map

No file is shared for mutation by two concurrent packages. A needed file not listed here is a
scope amendment returned to the coordinator before editing.

### WP-2 — shared read boundary

- `crates/kis-client/Cargo.toml`, `Cargo.lock`; use already locked-compatible `fs2 0.4`,
  `hmac 0.12`, and `sha2 0.10` rather than a new coordination daemon/library family
- new `crates/kis-client/src/read_coordination.rs` and focused tests; registration in
  `crates/kis-client/src/lib.rs`
- `crates/kis-client/src/auth.rs`, `rate_limit.rs`, `market_data.rs`, `token_issuer.rs`,
  `retry.rs`, and only if a typed variant is necessary `error.rs`
- constructor wiring only in `data-pipelines/collectors/src/worker.rs`,
  `data-pipelines/collectors/src/bin/kis-action-range-raw.rs`,
  `data-pipelines/collectors/src/bin/kis-stock-price-beta-raw.rs`, and
  `crates/job-queue/src/bin/owner-equity-v2-runner.rs`
- focused kis-client/constructor/cross-process fixture tests; no provider call

WP-2 must not edit `live_order.rs`, REST order code, Compose, DB, parser, API, or Web.

### WP-3 — parser, demand/cache, producer

- the two exact 0054 migration files
- new `crates/market-data/src/intraday_quotes.rs` and registration in
  `crates/market-data/src/lib.rs`
- new `data-pipelines/collectors/src/intraday_quotes.rs` and registration in
  `data-pipelines/collectors/src/lib.rs`
- new `crates/job-queue/src/owner_equity_v2/intraday.rs`, registration only in
  `crates/job-queue/src/owner_equity_v2.rs`, and quote-loop wiring only in
  `crates/job-queue/src/bin/owner-equity-v2-runner.rs` after WP-2 is integrated
- new focused unit/DB tests named for `intraday_quotes`; existing test files are not repurposed

The shared runner file is sequential ownership: WP-2 completes and is integrated before WP-3.
WP-3 does not touch Compose, API, OpenAPI, or Web.

### WP-4 — API/OpenAPI

- new `crates/api-server/src/http/owner_intraday_quotes.rs` and
  `crates/api-server/src/repos/owner_intraday_quotes.rs`
- module/route/state connection points only in `crates/api-server/src/http/mod.rs`,
  `crates/api-server/src/repos/mod.rs`, `crates/api-server/src/http/state.rs`,
  `crates/api-server/src/contract.rs`, and if required `crates/api-server/src/lib.rs`
- `apps/api-server/scripts/openapi-spec.mjs`, `apps/api-server/openapi.json`,
  `apps/api-server/generated/openapi.ts`
- new `crates/api-server/tests/http_owner_intraday_quotes.rs` and scoped additions to
  `crates/api-server/tests/openapi_contract.rs`

WP-4 does not modify migration/producer/provider/Web files and never adds KIS configuration.

### WP-5 — Web widget and client

- new `apps/web/lib/products/intraday-quotes-contracts.ts` and
  `apps/web/lib/products/intraday-quotes-client.ts`
- new `apps/web/components/stock-beta/quote/` files for coordinator, hook, view model, widget, and
  CSS
- minimal connections in `apps/web/components/stock-beta/shared/widget-types.ts`, dashboard and
  detail `types.ts`/`widget-registry.ts`, `stock-beta-workspace.tsx`,
  `stock-beta-detail.tsx`, and `apps/web/lib/i18n/dictionaries/stock-beta.ts`
- `apps/web/lib/api/contracts.ts` only if needed for mutation path registration
- new `apps/web/tests/stock-beta-intraday-*.test.ts`/`.tsx`

WP-5 does not edit existing E2E, EOD chart/signal coordinators, geometry/renderer, Rust, OpenAPI,
or deployment. It must first read the installed Next documentation required by `apps/web/AGENTS.md`.

### WP-6 — default-off runtime and operations

- `deploy/compose/compose.yml`, `deploy/compose/.env.example`
- `scripts/ops/provision-linux.sh`, `scripts/ops/validate-production-config.sh`,
  `scripts/ops/owner-equity-v2-runtime-static-check.sh`,
  `scripts/ops/owner-equity-v2-runtime-self-test.sh`, and the minimal invocation in
  `scripts/ops/static-check.sh`
- new `configs/market-hours/krx-intraday-session-windows-v1.json`,
  `configs/market-hours/krx-intraday-session-windows-v1.schema.json`,
  `scripts/ops/stock-beta-intraday-static-check.sh`, and
  `scripts/ops/stock-beta-intraday-self-test.sh`; the data file must remain empty/disabled until
  section 12 approval and exact date evidence exist
- new `docs/runbooks/stock-beta-intraday-quotes.md`
- `docs/diagrams/component_architecture.puml`, `runtime_deployment.puml`, and their two locally
  rendered PNGs with current evidence anchors

The protected coordination bind is added only to credentialed read services from section 4, never
API/Web or a provider-free service. `provision-linux.sh` creates and verifies its 0700 host leaf;
the production validator verifies its files without printing content. No new service/image/login is
allowed. Production activation, migration execution, or service restart is not part of WP-6.

### WP-7 and WP-8

- WP-7 writes only
  `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes-review.md`.
- WP-8 owns new `apps/web/tests/e2e/stock-beta-intraday.spec.ts`, new intraday synthetic fixture
  files, the minimal routing addition in `apps/web/tests/e2e/support/synthetic-api.mjs`, and
  `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes-qa.md`.
- Any implementation correction discovered by WP-7/8 returns to WP-2..6 ownership; reviewers do
  not patch another package's source.

## 12. Approval delta and launch gates

| Delta from already approved system | Status after WP-1 | Required action |
| --- | --- | --- |
| private single-owner KIS personal-use entitlement | already settled by ADR-0005 | do not reopen; preserve exact entitlement reference/hash and private serving boundary |
| parse `stck_prpr`, `prdy_vrss`, `prdy_vrss_sign`, `prdy_ctrt`, `stck_sdpr`, `iscd_stat_cls_code`, `temp_stop_yn` rather than identity only | **owner approved parser and fixture tests, 2026-09-08** | user's “승인해” answers the exact seven-field, no-live-call question; implement this whitelist/sign legend/fail-closed contract only; live activation remains separate |
| periodic production current-price calls, five-second slot and 5,000-attempt daily cap | product requested, but **live high-frequency activation not authorized by this docs task** | separate activation approval with exact instrument/time/budget and current official docs; fixtures remain network-free |
| process-shared token persistence and coordination bind | proposed security/runtime change | coordinator accepts WP-2 design; cross-process crash/rotation tests must pass before producer work |
| date-specific session-window evidence file | proposed fail-closed seam; baseline input absent | coordinator accepts seam, then owner-reviewed official evidence populates exact dates; otherwise `UNKNOWN`, zero calls |
| API route prefix | **amended** from plan placeholder to existing `/api/v1/research/owner-beta/equity-universe-v2` | coordinator accepts literal paths before WP-4/5 integration |
| 11-second timing | **narrowed** to first-attempt <=1s synthetic happy path | tests must not claim 11 seconds across timeout/retry/Retry-After; use the arithmetic in section 6 |
| active demand bound | five identities retained; explicit 20 consumer leases added | coordinator accepts before DB/API/Web work |
| new provider, NXT/unified, WebSocket, order/account/live profile, automatic ETF11 | not proposed and not authorized | a separate project/approval is required; no downstream worker may add it |

WP-2 may implement and test the provider-free shared primitive after the contract gate. Any branch
that parses the newly listed live response fields or enables production polling waits for the
explicit approvals above. WP-3/4/5 may use synthetic fixtures only after the coordinator confirms
the exact DTO; production remains default-off through WP-6 and a separate release gate.

The 2026-09-07 [stale-release incident](../../runbooks/kis-daily-stale-release-20260907.md) and
[release-readiness plan](../plans/2026-09-07-stock-beta-production-release-readiness.md) are records
of incomplete rollout/recovery, not evidence of current production health. No production host was
inspected for WP-1. Production activation must freshly verify installed immutable release,
database migration, same-day calendar/window evidence, daily recovery, coordination mount and
permissions, entitlement pins, disabled order/live paths, request counters, and rollback.

## 13. Deterministic acceptance matrix

Minimum fixture-backed cases before static acceptance:

| Layer | Required cases |
| --- | --- |
| response parser | valid UP/DOWN/FLAT/LIMIT_UP/LIMIT_DOWN; signed-value mismatch; zero/empty/number/exponent/overflow; missing/wrong identity; nonzero/missing `rt_cd`; malformed JSON; missing/extra tolerated output fields; status 58 and temporary halt; unknown stop value; no provider time fabricated |
| shared credential | two real OS processes at same instant issue one token and one read; alias paths resolve to one scope; 401; 429/Retry-After; timeout; crash before/after reservation; corrupt/broad/symlink state; lock timeout; clock rollback; credential generation rotation; missing mount => zero network |
| scheduler/budget | 1 and 5 identity cadence/fairness; 20 leases/5 identities; duplicate merge; one-tab release isolation; 30/15 expiry/renew; no-demand zero calls; every retry counted; 4,680 regular slots and 5,000 hard stop; halt 60s; EOD contention; producer takeover and late-response discard |
| session | matching current KIS lineage plus exact pinned window; missing/stale/conflicting calendar; missing/incorrect hash; regular/special/closed; before open/at open/before close/at close; KST date rollover; no `chk-holiday` call |
| DB/RLS | 0054 up/down on disposable PostgreSQL; app actor isolation; Member/admin/other owner denial; worker grants; capacity transaction race; generation/disable/in-flight fence; restart stale; 24h logical/physical GC; no write to EOD/Raw/snapshot tables |
| API | exact DTO/OpenAPI; CSRF/idempotency/sequence replay; 401/403/404 indistinguishability; capacity; cache-only 1/10/100 GETs produce zero provider/token/demand writes; no-store; decimal strings; unavailable/stale/halted states |
| Web | fake clock 5s/15s/30s; no overlap; visibility/offline/unmount/logout; A->B late response; generation change/disable/re-add; multi-tab merge/release; catalog add/remove/reorder/hide; bilingual/accessibility; no EOD chart/signal rerun |
| integration | 30 compressed minutes have bounded rows/memory/attempts; invalid quote never replaces last good; current quote beside old EOD remains separately labelled; provider/network count is zero in ordinary QA; forbidden host/path/TR/account/order/WebSocket count is zero |

The following invariants are release blockers: API/Web receive no KIS secret or Raw write root;
quote GET never reaches provider; all credentialed read constructors use one verified coordination
directory when enabled; no current quote mutates an EOD/publication/signal table; and absent session
proof always produces zero broker calls.
