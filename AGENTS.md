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
- Daily bars, reference quotes, and KSD schedules are single-page.  KSD requests keep `CTS` blank;
  an absent/blank continuation marker is terminal, while any nonempty marker fails closed before
  Raw visibility.  Never follow generic sample-code `M`/`F` handling for these reviewed endpoints.
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
