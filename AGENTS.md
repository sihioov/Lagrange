# Repository Agent Instructions

## KIS Open API safety boundary (mandatory)

This release uses KIS only to collect read-only Korean market and corporate-action data.  A live
market-data host or a production App Key does not authorize trading.  Account access and every
order-capable path remain out of scope until the user explicitly starts a separate live-trading
project.

### Allowed network surface

- `POST /oauth2/tokenP` is allowed only to obtain or reuse the REST access token.  Cache and reuse
  the token according to its expiry; never issue a token for every request.
- Market-data requests must be `GET` calls and must match one of these exact path/TR-ID pairs:
  - `/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice` — `FHKST03010100`
  - `/uapi/domestic-stock/v1/quotations/inquire-price` — `FHKST01010100`
  - `/uapi/domestic-stock/v1/quotations/chk-holiday` — `CTCA0903R`
  - `/uapi/domestic-stock/v1/ksdinfo/paidin-capin` — `HHKDB669100C0`
  - `/uapi/domestic-stock/v1/ksdinfo/bonus-issue` — `HHKDB669101C0`
  - `/uapi/domestic-stock/v1/ksdinfo/dividend` — `HHKDB669102C0`
  - `/uapi/domestic-stock/v1/ksdinfo/merger-split` — `HHKDB669104C0`
  - `/uapi/domestic-stock/v1/ksdinfo/rev-split` — `HHKDB669105C0`
  - `/uapi/domestic-stock/v1/ksdinfo/cap-dcrs` — `HHKDB669106C0`
- Treat this as a deny-by-default allowlist.  Changing a method, path, TR ID, host, or response
  contract requires explicit user approval, current official KIS documentation, and focused tests.
- The complete EOD bundle is production-market-data only: KIS marks `chk-holiday` and the KSD
  schedule endpoints as unsupported in the mock environment.  This does not permit any production
  trading API; only the read allowlist above may use the live REST host.

### Forbidden surface

- Never call an order, correction, cancellation, reservation, account, balance, buying-power,
  sellable-quantity, execution-history, order-notification, or account WebSocket API.
- Never add or consume account identifiers such as `CANO`, `ACNT_PRDT_CD`, or `KIS_ACCOUNT_REF`
  in the read-only collector.  Do not enable the Compose `live` profile or any live order node.
- Do not copy or run an official KIS sample wholesale.  The official repository and the full XLSX
  specification intentionally contain both read APIs and real trading examples.
- Never print, log, persist in Git, or place in diagnostics an App Key, App Secret, access token,
  account identifier, request/response body, or free-form broker message.

### Rate, pagination, and validation rules

- KIS documents a 24-hour access-token lifetime, a one-issuance-per-day principle, a six-hour
  renewal interval, and a one-token-request-per-minute safeguard.  Use the existing token manager;
  do not add an eager token-refresh loop.
- KIS currently documents at most 20 live REST requests per second per App Key (2 in mock), while
  the project ceiling is the stricter one request per second per endpoint/TR channel and sequential
  execution by default.  Honor broker throttling and `Retry-After`; retries must be bounded and
  classified.
- `chk-holiday` is single-page for this project.  Send blank continuation fields, consume only the
  first response, require the exact target date, and do not call it more than once per daily run.
  Do not follow contradictory continuation tokens returned by that endpoint.
- Daily bars, reference quotes, and `chk-holiday` remain single-page.  Those requests keep their
  continuation fields blank and never follow a broker marker.  For the six KSD action endpoint
  paths, the user-approved current official GitHub sample takes precedence over the contradictory
  XLSX pagination note: initial `CTS` and `tr_cont` are blank; only an exact response header
  `tr_cont=M` permits one or more next GETs with the unchanged query (including blank `CTS`) and
  request header `tr_cont=N`.  `F`, blank, or any other marker is terminal.  KSD pagination is
  bounded at ten pages and repeated response bytes fail closed before any Raw batch is visible.
  The discrepancy and the exact sample behavior are recorded in the KIS backfill runbook; no
  other endpoint may inherit this continuation policy.
- Malformed JSON, nonzero `rt_cd`, an undocumented response shape, or a missing target observation
  also fails closed.
- Raw data becomes visible only after HTTP success, JSON/schema validation, and immutable hash
  commit.  Only the documented bonus-issue event is automatically normalized today; unsupported
  nonempty corporate-action types stop the pipeline instead of inventing dates or factors.

### Rights and source of truth

- KIS states that personal market data is for the customer's own-asset investment use and cannot
  be redistributed; corporate and partner use can require separate market-data agreements.  Keep
  the configured entitlement reference/hash and do not expose Raw or Curated data beyond the
  user's confirmed rights.
- Primary references are the current KIS API portal, the official
  `koreainvestment/open-trading-api` repository, and
  `docs/kis_openapi_entiredocs_20260818_030007.xlsx`.  When they conflict, choose the narrowest
  fail-closed behavior and record the exact discrepancy in tests or a runbook.

### Cross-cutting ETF source and runtime boundary

- KIS read-only daily bars and reference quotes are the primary ETF11 price/volume source.  A
  second price API is added only after explicit owner approval of its independent cross-check or
  fallback purpose, priority, and failure behavior, with current documentation and focused tests.
- Once-daily incremental KIS EOD collection is owner-approved.  Its runtime activation is created
  only by the commit-pinned immutable-release installer; legacy static KIS unit/env artifacts are
  not an alternate activation path.  This approval remains read-only and does not authorize any
  account or order API.
- KIND D11 remains a low-volume, manual, operator-confirmed one-calendar-day browser capture.
  Scheduled or timer activation, bulk capture, and full-history backfill remain forbidden.

## OpenDART safety boundary (mandatory)

Stage6 adds OpenDART (`opendart.fss.or.kr`) as a read-only disclosure-evidence source for the
fixed 11 ETF universe.  The owner approved only the core surface below, on 2026-08-19, and only
for fixture-backed Raw adapter work.  Approval covers the request shape, the fixture contract,
and an operator-gated live path.  The owner supplied a key on 2026-08-20 and authorized the live
path; exactly one live request has been made since, a single `GET /api/corpCode.xml`.  Its result
settled the ETF coverage question below: **OpenDART does not model the fixed 11 ETF issues as
disclosure entities**, so `list.json` and `company.json`, both keyed by `corp_code`, have no ETF11
use.  Note also that the live host negotiates TLS 1.2 with a static-RSA suite only, which rustls
does not implement, so the in-process transport cannot currently reach it (see ADR-0004 D10).  Decisions and evidence live in
`docs/decisions/0004-stage6-official-source-contracts.md` and
`docs/runbooks/stage6-source-contracts.md`.

### Allowed network surface

Requests must be `GET` and must match one of these exact host/path pairs:

- `opendart.fss.or.kr` — `/api/list.json` and `/api/list.xml` (disclosure search list)
- `opendart.fss.or.kr` — `/api/corpCode.xml` (corp-code reference archive, a ZIP)
- `opendart.fss.or.kr` — `/api/company.json` and `/api/company.xml` (company overview)

Treat this as a deny-by-default allowlist, exactly as for KIS.  Changing a method, path, host, or
response contract requires explicit user approval, current official OpenDART documentation, and
focused tests.

### Forbidden surface

- Never call any other OpenDART endpoint.  `crDecsn`, `piicDecsn`, `cmpMgDecsn`, `alotMatter`, and
  the original-document file API are explicitly DEFERRED, not allowed.
- Never call `data.go.kr`, `openapi.krx.co.kr`, `data.krx.co.kr`, `kind.krx.co.kr`,
  `seibro.or.kr`, or `api.seibro.or.kr` from code.  Those surfaces remain deferred; the KIND and
  Data Marketplace exports are operator-driven downloads, not automated collection.
- **FSC KRX-listed diagnostic consumed (historical, 2026-08-21):** the exact
  `GetKrxListedInfoService/getItemInfo` coverage probes for ETF `069500` /
  `KR7069500007` and the one owner-approved, non-persisting stock control `000020` /
  `KR7000020008` at `basDt=20260819` are fully consumed.  The source is rejected for ETF11
  identity; no further live FSC query or FSC Raw collection is allowed without a new explicit
  owner approval.  Offline fixture and contract code may remain, but it is not a live allowance.
- **Sole KIND exception (owner-approved ADR-0004 D11, 2026-08-20):** the existing
  `data-pipelines/kind-capture` path may drive KIND's own browser controls for low-volume,
  operator-gated ETF disclosure and correction-evidence capture.  It must not reconstruct or send
  direct HTTP requests, and it grants no permission to another KIND page or endpoint.  Bulk,
  scheduled, or full-history KIND capture remains forbidden until the owner explicitly approves a
  request budget and retention scope.
- Never register an account, request an API key, submit a form, or complete an identity check from
  an agent session.  Those are owner actions.
- The disclosure response kinds (`DisclosureIndex`, `DisclosureEntityMaster`,
  `DisclosureEntityProfile`) are disclosure-source evidence.  They must never be admitted into the
  EOD reference normalizer, the candidate path, or any publication kind set.

### The API key is a query parameter — redact before persisting

OpenDART authenticates with a `crtfc_key` **query parameter**, not a header.  Recorded request
metadata is persisted into `batch.json` and the append-only manifest, so a naive adapter writes the
key to disk permanently inside an immutable store.

- The recorded query must never contain the `crtfc_key` value.  Redact or omit it where the request
  metadata is constructed, not afterwards and not optionally.
- Construct request metadata only through the redacting constructor; never let a caller assemble
  metadata carrying a live key.
- A test must assert that a sentinel key value appears in neither the stored bytes, nor
  `batch.json`, nor the manifest JSONL.
- As with KIS: never print, log, persist in Git, or place in diagnostics a key, token, response
  body, or free-form provider message.  Errors are typed, never string-dumped provider output.

### Rate, pagination, and validation rules

- `list.json` / `list.xml` pagination is `page_no`-driven from 1, with `page_count` documented as
  1–100 (default 10).  It is terminal when `page_no >= total_page`, or immediately when
  `total_page` is 0.
- The walk is bounded at ten pages, matching the KSD bound above.  Exceeding it fails closed.
- `total_count` and `total_page` must be identical on every page of one query.  A mid-walk change
  means the result set shifted and completeness cannot be proven: fail closed.
- Identical response bytes for two different requested pages fails closed.
- `corpCode.xml` and `company.json` are single-page.  Send no continuation field and reject any
  pagination-like marker.  `corpCode.xml` must begin with the ZIP magic `PK\x03\x04`; an error body
  in its place fails closed instead of being stored as an archive.  Observed 2026-08-20: the `.xml`
  surface returns its error envelope as **XML with HTTP 200**
  (`<result><status>010</status>...`), so validation cannot rely on the status line, and `010`
  means an unregistered key.  Only the status code may cross into an error; the `<message>` prose
  never does.  The archive is stored
  byte-for-byte and is not unzipped or parsed in the Raw stage.
- Status `000` is success.  `013` is documented no-data and must be a typed empty outcome, distinct
  from an error.  Any other status, a missing `status`, malformed JSON, or an undocumented shape
  fails closed.
- Without `corp_code`, the documented list search window is limited to three months.
- `rcept_no` is opaque.  Validate it only as exactly 14 ASCII digits.  Never parse, slice, or infer
  a date from it: the official documentation gives only a viewer-link example and never states that
  its leading digits encode the receipt date, so deriving a date would fabricate point-in-time
  evidence.
- OpenDART terms art. 10(4) state that usage-count limits exist and are posted on the homepage.  The
  actual number is not confirmed on any official page, and the widely cited 20,000-per-day figure
  appears only in third-party material.  Do not encode an assumed quota; honor returned throttling
  and keep retries bounded and classified.

### Point-in-time limits — do not overclaim

- `rcept_dt` is documented as `공시 접수일자(YYYYMMDD)`.  There is no time-bearing field anywhere in
  the documented response schema, so OpenDART supports day granularity only.
- No field links a correction filing to the original filing's `rcept_no`.  The `rm` field carries
  advisory codes only — `정` means a later correction exists, `철` means the report is deemed
  withdrawn.  Never present these as a structured lineage join.
- A record date, payment date, listing date, or ex-rights date is never written to `available_at`.
- **Resolved 2026-08-20: OpenDART does not cover the 11 ETFs.**  One live `corpCode.xml` request
  returned 118,714 entities, 3,984 with a non-empty `stock_code`, and none of the eleven ETF short
  codes among them; controls in the same file resolve, and the 458 `자산운용` entities all lack a
  `stock_code` because the asset manager is the filer rather than the ETF.  Do not build an ETF11
  identity or disclosure-date path on OpenDART.  `corpCode.xml` remains the identity join for any
  future individual-stock scope, where issuers are disclosure entities.

### Rights and source of truth

- The FSS states the service is free in principle.  Terms art. 16(1) place copyright in the API
  service and related programs with the FSS, and art. 23(1) disclaim accuracy and completeness of
  disclosure content, which is the filer's responsibility.
- No clause permitting or restricting indefinite retention or redistribution of API responses was
  located.  That is a gap, not permission.  Keep the configured entitlement reference and hash, and
  do not expose Raw or Curated data beyond the owner's confirmed rights.
- Primary references are the current OpenDART developer guide pages under
  `opendart.fss.or.kr/guide/`, plus `docs/runbooks/stage6-source-contracts.md` for the recorded
  quotes and gaps.  When sources conflict, choose the narrowest fail-closed behavior and record the
  exact discrepancy in tests or the runbook.

## Architecture diagrams must track structural changes (mandatory)

`docs/diagrams/component_architecture.puml` (static code dependencies) and
`docs/diagrams/runtime_deployment.puml` (compose runtime/deployment) are
evidence-bound: every drawn edge carries a `' evidence: <file:line>` comment
pointing at the concrete code that makes the edge exist, and deliberately
folded edges are listed with evidence in the component file's header.

- Whenever a change merged to `main` alters the structure, update both the
  affected `.puml` and its evidence comments in the same change, before or
  with the merge.  Structural changes include: workspace members or Cargo
  path dependencies; `nt/` package boundaries or the Rust-to-Python spawn
  contract; compose services, profiles, networks, or volume mounts; nginx
  routing; DB roles or table groups; the web-to-API contract surface.
- An edge without a real, current `file:line` evidence citation must not be
  drawn.  Do not leave stale citations behind after moving code; fixing the
  citation is part of the structural change.
- Re-render the PNGs locally and commit them together with the `.puml`
  sources (no network rendering service; diagram content stays local):
  `docker run --rm -v "$PWD/docs/diagrams:/data" plantuml/plantuml -tpng /data/component_architecture.puml /data/runtime_deployment.puml`
- If a change does not alter the structure, leave the diagrams untouched.
