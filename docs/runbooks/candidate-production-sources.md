# Candidate production source contract runbook

Date: 2026-08-29
Decision: `docs/decisions/0007-individual-stock-production-sources.md`
Gate state: **Gate A blocked; no production activation or provider call authorized**

## Purpose and safety boundary

This runbook is the evidence ledger and owner checklist for activating the existing
KOSPI 200 individual-stock candidate vertical. It keeps the same contract shapes
ready for later KOSDAQ 150 activation, but does not authorize KOSDAQ collection.

Candidate data must not enter the ETF recommendation evidence domain. Provider
payloads remain immutable Raw evidence and are never returned by product APIs.
Repository code, test fixtures, public documentation, and credentials do not grant
network authority. The root `AGENTS.md` deny-by-default allowlists remain controlling.

Do not:

- call a provider, download a data payload, use a credential, accept terms, buy a
  product, or register an account while executing this runbook;
- use KIS account/order paths, identifiers, or WebSockets;
- automate a KRX Data Marketplace page or reconstruct its browser requests;
- treat a current master/member snapshot as historical evidence;
- add a source, host, method, path, TR ID, transport, audience, or historical claim
  without a new owner approval and focused tests.

## Evidence ledger

All pages below were inspected as public documentation on 2026-08-29. No API or
data-delivery endpoint was called and no response body was copied into this file.
“Page date” is stated only where the official source exposes one.

| Authority | Official evidence | Page/document date | What it establishes | Gap that remains |
| --- | --- | --- | --- | --- |
| KIS API portal | <https://apiportal.koreainvestment.com/apiservice-summary> | Checked 2026-08-29 | Production REST host `openapi.koreainvestment.com:9443`, TLS 1.2/1.3, service categories, and daily master update notices | It does not make a repository path allowed or prove candidate display rights |
| KIS daily-bar sample | <https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/inquire_daily_itemchartprice/inquire_daily_itemchartprice.py> | Sample header 2025-01-01; checked 2026-08-29 | Exact GET path/TR, adjusted-price selector, no continuation in the sample, up to 100 observations | No provider publication timestamp or revision field |
| KIS flow sample | <https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/investor_trade_by_stock_daily/investor_trade_by_stock_daily.py> | Sample header 2025-08-21; checked 2026-08-29 | Exact GET path/TR; the code recurses for response `M` and `F`, sends request `N` in both cases, and has a ten-page bound | `F`-continuation contradicts the proposed narrower terminal policy; stable rows/page, availability, revisions, and the approved continuation interpretation remain unresolved |
| KIS flow field checker | <https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/investor_trade_by_stock_daily/chk_investor_trade_by_stock_daily.py> | Checked 2026-08-29 | Official field names for trade date and foreign/institution quantity/amount | Unit must be preserved from the official dictionary and covered by a fixture test |
| KIS balance sheet sample | <https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/finance_balance_sheet/finance_balance_sheet.py> | Sample header 2025-06-17; checked 2026-08-29 | Exact GET path/TR and annual/quarterly selector | No defensible disclosure time, statement scope/unit contract, or correction lineage |
| KIS income statement sample | <https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/finance_income_statement/finance_income_statement.py> | Sample header 2025-06-17; checked 2026-08-29 | Exact GET path/TR and annual/quarterly selector | Same canonical-fundamentals gaps as balance sheet |
| OpenDART full statements | <https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS003&apiId=2019020> | API registered 2020-11-20; checked 2026-08-29 | `fnlttSinglAcntAll.json`, parameters, report codes, OFS/CFS, account fields, currency, opaque receipt number; no page controls | Endpoint is forbidden today; date-only availability requires a conservative rule; no correction link |
| OpenDART disclosure list | <https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019001> | Checked 2026-08-29 | `rcept_dt`, opaque `rcept_no`, page number/count, status contract | Day granularity only; no publication time or structured correction lineage |
| OpenDART corp code | <https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019018> | Checked 2026-08-29 | Corp-code archive and current `stock_code` join | Current archive is not historical issuer/security identity |
| OpenDART terms | <https://opendart.fss.or.kr/intro/terms.do> | Checked 2026-08-29 | Usage limits exist; FSS program/service copyright and disclosure-content disclaimer | No explicit indefinite Raw retention or derived-display permission was located |
| KRX Data Marketplace terms | <https://data.krx.co.kr/contents/MDC/INFO/informationController/MDCINFO003.cmd> | Effective 2026-08-29; checked same day | Unauthorized automated collection/copy/distribution is prohibited; purchased market data follows separate terms | Public UI/export is not a scheduled-production entitlement |
| KRX halt history | <https://data.krx.co.kr/contents/MDC/STAT/issue/MDCSTAT213.jsp> | Checked 2026-08-29 | Official human-facing history includes issue, market, stop, and resume dates | No approved delivery contract, publication time, revisions, or complete status flags |
| KRX administrative history | <https://data.krx.co.kr/contents/MDC/STAT/issue/MDCSTAT215.jsp> | Checked 2026-08-29 | Official human-facing history includes designation category/reason | Same delivery and PIT gaps |
| KRX delisting | <https://data.krx.co.kr/contents/MDC/STAT/issue/MDCSTAT238.jsp> | Checked 2026-08-29 | Official human-facing listing/delisting facts | Same delivery and PIT gaps |
| KRX Open API catalog | <https://openapi.krx.co.kr/contents/OPP/INFO/service/OPPINFO004.cmd> | Checked 2026-08-29 | Catalog of 31 services and coverage beginning in 2010 for listed daily/basic datasets | Does not provide the full membership/sector/status contract |
| KRX Open API terms | <https://openapi.krx.co.kr/contents/OPP/INFO/OPPINFO002.jsp> | Effective 2025-12-26; checked 2026-08-29 | Noncommercial use, 10,000/day maximum, no third-party provision, use ends with contract | Incompatible with the intended immutable Raw archive and incomplete datasets |
| KRX market-data policy | <https://data.krx.co.kr/inc/datasale/Market%20Data%20Usage%20Polices_ko.pdf?v=20230121_1> | Document dated 2024-01-01; checked 2026-08-29 | Distinguishes internal final-user and distribution uses | General policy is not the required signed product/order entitlement |

Repository evidence also includes ADR-0003’s canonical candidate fields,
ADR-0004’s official-source/TLS findings, ADR-0005’s private KIS entitlement, the
root allowlists, and the existing candidate providers/normalizers. The KIS master
ZIP URLs are official public download surfaces, but their records contain no
historical announcement/effective/availability/revision evidence, so they remain
reconciliation inputs only.

## Canonical admission semantics

The following columns are mandatory where applicable. A source-specific mapping
must never collapse them:

| Canonical fact | Admission rule |
| --- | --- |
| `event_at` | Provider event/trade date or fiscal period boundary; never retrieval time |
| `effective_at` / interval | Provider-defined eligibility/classification/status start and end; never inferred from a snapshot |
| `available_at` | Explicit publication timestamp when supplied. For OpenDART `rcept_dt`, next calendar day `00:00 Asia/Seoul` is the conservative project cutoff. For a newly retrieved KIS observation with no publication field, retrieval is allowed only for current incremental use. |
| `retrieved_at` | Collector clock after successful response receipt and before immutable Raw commit |
| `source_revision` | Provider revision/correction ID; otherwise immutable Raw hash labelled “retrieved snapshot,” not provider revision |
| supersession | Provider-linked correction only. Similar periods or names do not prove a restatement link. |

Raw becomes visible to normalization only after HTTP success, typed status/schema
validation, secret-redacted request metadata construction, content hashing, and
immutable commit. Repeated page bytes, shifting totals, unsupported nonempty action
types, missing target observations, missing mandatory units/times, or budget excess
fail the complete run closed.

## Dataset contracts and options

### 1. EOD price

**Preferred:** KIS production read-only REST.

- Identifier: `GET /uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice`,
  TR `FHKST03010100`, host `openapi.koreainvestment.com:9443`.
- Request: market code, six-digit issue code, inclusive date range, period divisor,
  and explicit `FID_ORG_ADJ_PRC`. Continuation fields stay blank and are ignored.
- Response/mapping: `stck_bsop_date -> event_at`; `stck_oprc`, `stck_hgpr`,
  `stck_lwpr`, `stck_clpr`, `acml_vol` -> daily bar; persist adjustment mode and
  provider identity. Official sample supports at most 100 rows.
- PIT/revision: retrieval is availability; Raw hash is snapshot identity. The
  provider exposes no publication or revision field. Adjusted history is therefore
  not a strict historical PIT series.
- Rights/credentials: private owner use under ADR-0005, protected KIS secret path,
  no Member display or Raw exposure. Exact endpoint is allowlisted, but KOSPI 200
  scope/budget is pending Gate A.
- Rejected fallback: no second price API is approved. Current KIS master files are
  identity/reference snapshots, not prices.

### 2. Investor flow

**Preferred:** KIS production read-only REST after a new allowlist approval.

- Identifier: `GET /uapi/domestic-stock/v1/quotations/investor-trade-by-stock-daily`,
  TR `FHPTJ04160001`.
- Request/continuation: market divisor `J`, issue code, input date, and documented
  blank selectors. The current official sample recurses for response `tr_cont=M`
  and `F` and sends request header `tr_cont=N` in both cases. The proposed narrower
  policy would allow only exact `M` to continue and treat `F`, blank, and every
  other marker as terminal, but it must not be called official behavior or enabled
  until Gate A approves the discrepancy with current documentation and focused
  tests. Cap any approved walk at ten pages/symbol, the lower official-sample bound
  rather than the existing code’s 32-page bound; reject repeated Raw bytes.
- Mapping: `stck_bsop_date`; foreign amount/quantity
  `frgn_ntby_tr_pbmn`/`frgn_ntby_qty`; institution amount/quantity
  `orgn_ntby_tr_pbmn`/`orgn_ntby_qty`. Canonical net amount in KRW is the provider
  amount multiplied by 1,000,000; this million-KRW scale must be fixture-proven
  against the official field dictionary before activation.
- PIT/revision: trade date is event date; retrieval is current availability; Raw
  hash is snapshot identity. No historical correction visibility claim.
- Rights/status: owner-only KIS terms; credentials stay with the existing token
  manager. **Not currently allowlisted.** Code existence is not approval.
- Rejected alternatives: KRX public UI automation is prohibited; KRX Open API does
  not establish an equivalent complete contract in its public catalog.

### 3. Fundamentals

**Preferred if owner resolves its blockers:** OpenDART full financial statements.

- Identifier: `GET https://opendart.fss.or.kr/api/fnlttSinglAcntAll.json`.
- Request: redacted `crtfc_key`, `corp_code`, business year from 2015 onward,
  `reprt_code` (`11013`, `11012`, `11014`, `11011`), and `fs_div` (`OFS`/`CFS`).
  The documented response is single-page.
- Mapping: exact `rcept_no`; fiscal year/report kind; OFS/CFS statement scope;
  statement/account IDs and names; current/prior amounts; order; currency. A metric
  is admitted only after an explicit canonical account and unit policy exists. The
  documented response does not by itself prove exact issuer fiscal-period start and
  end, so the canonical row remains blocked unless supplementary approved official
  evidence supplies both; business year/report code must not be expanded into dates.
- Availability/revision: obtain `rcept_dt` by exact opaque receipt join to the
  already allowlisted `list.json`; never parse the receipt number. Apply the
  conservative next-day cutoff. A new receipt is a new revision identity, but no
  structured original link exists, so do not invent `restates_source_revision`.
- Pagination/rate: statement endpoint is single-page. `list.json` is page-number
  driven, maximum ten pages, stable totals required, and repeated bytes rejected.
  The official guide describes a general daily limit but also provider-specific
  adjustment; encode the project’s lower 250-request/day ceiling, not a presumed
  entitlement.
- Rights/transport/status: **blocked.** The statement endpoint is forbidden by the
  root boundary. OpenDART’s live host was observed to require TLS 1.2 static-RSA,
  unsupported by the workspace rustls transport. Public terms do not establish
  indefinite Raw retention/derived display. The owner must approve the endpoint,
  a host-scoped compatible TLS implementation, and a rights attestation. The key
  is owner-operated and omitted/redacted at metadata construction.
- Rejected KIS fallback: `finance/balance-sheet` (`FHKST66430100`) and
  `finance/income-statement` (`FHKST66430200`) lack defensible disclosure time,
  statement unit/scope, and correction semantics and are not allowlisted.
- Contract alternative: a licensed vendor may replace OpenDART only if its signed
  dictionary supplies the same PIT, unit, statement-scope, revision, retention,
  audience, and delivery facts.

### 4. KOSPI 200/KOSDAQ 150 membership

**Preferred:** signed KRX or authorized Koscom index delivery.

The delivery must cover both index IDs even though only KOSPI 200 activates now. It
must provide ISIN/short code, daily as-of constituents, change notices,
`announced_at`, `effective_from`, `effective_until`, first publication time,
revision/correction ID, and a deterministic supersession rule. The delivery method,
host or object store, authentication, pagination/file completeness, rate, checksum,
cutoff calendar, late corrections, and replay behavior must be in the signed data
dictionary and operating schedule.

**Blocked:** no signed product/order annex or exact delivery specification is in the
repository. KRX’s public “index constituents” menu establishes that a human-facing
dataset exists, not its automation rights or PIT contract. KIS `idxcode.mst.zip` and
the market masters are current snapshots and cannot seed historical membership.

### 5. Sector

**Preferred:** the same licensed KRX/Koscom reference delivery, keyed through the
security master. Required fields are taxonomy ID and version, ISIN/short code,
industry code/name, profile, effective interval, availability, revision, correction,
and deletion semantics. A taxonomy change is a new effective-dated fact, not an
in-place rewrite.

**Blocked:** public industry-classification pages and KIS master fields do not prove
historical taxonomy/effective/publication/revision semantics or scheduled-use
rights. No current snapshot substitution is allowed.

### 6. Market status

**Preferred:** the licensed KRX/Koscom reference/status history delivery. It must
map every ADR-0003 flag: `suspended`, `administrative`, `liquidation`, `inactive`,
`disqualifying_audit_opinion`, and `complete_capital_impairment`, including reason,
effective start/end, publication availability, revision, and correction.

**Blocked:** the public KRX halt, administrative, and delisting pages cover only a
subset and do not establish approved automation/PIT revisions. KIS master flags are
retrieval snapshots. OpenDART cannot serve as a complete exchange-status source and
the evidence endpoints that might support audit/capital facts are not allowed.

### 7. Corporate actions

**Preferred for current incremental protection:** the existing KIS KSD allowlist:

| Path suffix under `/uapi/domestic-stock/v1/ksdinfo/` | TR ID |
| --- | --- |
| `paidin-capin` | `HHKDB669100C0` |
| `bonus-issue` | `HHKDB669101C0` |
| `dividend` | `HHKDB669102C0` |
| `merger-split` | `HHKDB669104C0` |
| `rev-split` | `HHKDB669105C0` |
| `cap-dcrs` | `HHKDB669106C0` |

All are GETs. Initial `CTS` and request `tr_cont` are blank. Only exact response
header `M` permits another GET with the unchanged query, blank `CTS`, and request
header `N`; ten pages maximum, repeated bytes fail closed. Schedule fields can map
event/effective dates only when their official endpoint dictionary supports the
meaning. Retrieval is availability and Raw hash is snapshot identity; no provider
revision is claimed. Only the documented bonus issue normalizes automatically.
Every other nonempty event remains a candidate-run blocker until separately mapped.

For historical research, procure a licensed KRX/KSD action delivery with original
announcement, revisions/cancellations, effective dates, and factor methodology.
OpenDART decision endpoints are forbidden and are not a complete action source.

### 8. Issuer identity

**Preferred current join:** licensed KRX/Koscom security master for instrument/ISIN,
plus OpenDART `GET /api/corpCode.xml` for current `stock_code -> corp_code` and,
optionally, `GET /api/company.json` for issuer profile. Those OpenDART paths are
already allowlisted, but the host’s TLS mismatch still blocks in-process transport.

The corp-code archive fields provide a current join and `modify_date`; they do not
establish historical effective/publication intervals. Historical identity must come
from the licensed security-master history. `company.json` identifies a disclosure
entity, not an exchange instrument. The OpenDART key must never appear in recorded
query metadata, logs, batch JSON, manifests, or errors.

## Rights, credential, and transport ownership

| Provider | Raw/retention/display boundary | Credential and transport owner | Current state |
| --- | --- | --- | --- |
| KIS | Private owner evidence only under ADR-0005; no Raw or Member-visible derived value; candidate scope pending | Existing protected owner secret and token manager; production REST TLS 1.2/1.3; no account fields | Exact price/calendar/KSD identifiers allowed; flow pending |
| OpenDART | Public terms reviewed do not explicitly settle indefinite Raw retention or derived display; absence is a gap, not permission | Owner key; redacting request constructor; a new endpoint-scoped compatible TLS transport would require approval | list/corpCode/company identifiers allowed, live transport blocked; financial endpoint forbidden |
| KRX/Koscom licensed delivery | Signed order must state history, retention, Raw archive, derived output, audience, redistribution, termination handling | Named owner role, exact host/protocol, credential rotation, checksum/signature and incident contact required in annex | No contract/delivery evidence; blocked |

No provider credential, account identifier, payload, free-form broker message, or
entitlement secret may appear in this runbook, Git, logs, diagnostics, or UI.

## Request-budget proposal

The following is a proposed hard ceiling, not authority to execute:

| Channel | Nominal KOSPI 200 daily work | Hard ceiling and behavior |
| --- | --- | --- |
| KIS price | 200 single-page requests | 220 total including retries; no continuation |
| KIS investor flow | 200 first pages | 400 total pages/day including continuation and retries; maximum ten/symbol; fail closed on excess |
| KIS holiday | 1 request | Once/run, exact target date, first page only |
| KIS KSD actions | 6 first pages | 60 total pages/day including retries; ten/endpoint, sequential |
| OpenDART | Up to 200 list lookups and at most 20 statement/detail calls on changed issuers | 250 requests/day including errors/retries; one corp-code archive/week outside daily eager path |
| Licensed files | One version each for membership, sector, status, security master | Four immutable objects/day; provider protocol budget must be contracted |

KIS nominal first-page work is 407 requests/day. KIS runs sequentially by default
and no faster than one request/second per endpoint/TR channel. All retries are
bounded, classified, honor `Retry-After`, and consume budget. The access token is
cached/reused under the existing 24-hour/renewal safeguards and is never issued per
request. A preflight must measure the actual investor-flow page count using approved
fixture/replay evidence before setting a historical acquisition budget.

The KIS daily hard ceiling is 681 requests/pages: 220 price, 400 flow, one
holiday, and 60 KSD. Unused budget is not transferable across channels.

Later KOSDAQ 150 activation would add nominally 150 price, 150 flow, and up to 150
filing-list requests. It requires Gate C and a new capacity decision; this runbook
only reserves compatible schemas.

## Current-daily and historical modes

### Current daily mode after Gate A

For cutoff `T`, admit only:

1. membership, sector, security identity, and all status flags whose licensed
   `available_at <= T` and whose effective interval contains the evaluation date;
2. KIS bars and flow observations retrieved, validated, and immutably sealed before
   the run cutoff;
3. fundamentals whose conservative availability cutoff is no later than `T`;
4. action evidence with no unsupported nonempty event.

The 60-session input window is context for one current candidate computation. UI
provenance must label retrieval-based availability and must not call the result a
historical reconstruction.

### Historical/backtest mode

Keep disabled until all of the following are contracted and loaded with their own
availability/revision facts: historical index change notices and snapshots, sector
taxonomy changes, all market-status intervals, security/issuer lifecycle, filing
corrections, unadjusted bars, and corporate-action revisions/factors. A current KIS
master, current DART corp archive, or current adjusted bar view cannot be replayed
backward. No current-snapshot historical substitution is permitted.

## Gate A evidence and acceptance checklist

Before the owner can use the exact approval form in ADR-0007, attach immutable
copies/hashes of all selected contracts and complete every item:

- [ ] Signed KRX/Koscom order and data dictionary name both index universes and all
      membership, security-master, sector, and six status fields.
- [ ] Documents define announcement/publication, effective intervals, revisions,
      corrections, historical depth, late data, delivery completeness, and replay.
- [ ] Rights explicitly cover immutable Raw retention, owner-only derived display,
      audience, termination, and any redistribution prohibition.
- [ ] Exact delivery host/protocol, credential owner, rate/file budget, checksum,
      schedule, and support owner are recorded; no placeholder remains.
- [ ] Owner chooses OpenDART or a licensed fundamentals vendor. For OpenDART, the
      exact endpoint, TLS implementation/scope, day-only cutoff, key redaction, and
      retention/display attestation are recorded.
- [ ] Owner approves the exact KIS flow pair, records the accepted `M`/`F`
      continuation interpretation and evidence, and approves candidate scope for
      the existing KIS price/calendar/KSD pairs; no account/order surface is added.
- [ ] Owner accepts or changes the request ceilings and current-vs-history claims.
- [ ] Fixture-backed tests cover exact identifiers, headers, pagination terminal
      markers, repeated pages, units, schema failures, redaction, Raw commit order,
      corrections, cutoff selection, and complete-run fail closure.
- [ ] A credential-free dry run and synthetic QA pass without fallback or snapshot
      substitution; product activation remains off during verification.
- [ ] The completed approval sentence contains contract IDs and hashes, exact
      endpoint/delivery identifiers, historical dates, entitlement reference,
      credential owner, and no angle-bracket placeholder.

Until every item is complete, the only valid owner decision is to leave Gate A
blocked or authorize contract/document acquisition outside the runtime. Neither is
production activation.

## Owner decision record template

Record decisions without credentials or payloads:

| Decision | Required value |
| --- | --- |
| Audience | `single-owner` or a separately contracted audience |
| KRX/Koscom contract/order ID and hashes | Immutable identifiers; no secret |
| Historical coverage | Exact inclusive dates per dataset |
| Raw retention and termination | Exact signed clauses/reference |
| Derived display | Exact permitted fields/audience |
| Delivery | Host/protocol, object naming, checksum, schedule, owner role |
| Fundamentals | OpenDART exact scoped contract or licensed vendor exact delivery |
| Transport | Existing KIS path; OpenDART TLS decision; licensed-delivery client owner |
| Budgets | Approved numeric daily/page/retry ceilings |
| Current/history claim | Accepted wording and prohibited substitutions |

Use the copy-ready Gate A text in ADR-0007 only after this table and checklist are
complete. The ADR intentionally makes placeholder-bearing approval invalid so that
missing rights or delivery semantics cannot be silently inferred.

## Stop conditions and escalation

Stop the affected branch and preserve typed evidence when:

- official sources conflict;
- announcement, effective, availability, unit, revision, or supersession semantics
  are missing;
- rights do not explicitly cover the intended Raw, retention, derived display, and
  audience;
- TLS or delivery transport cannot satisfy the approved host/path boundary;
- pagination, totals, target dates, page bytes, checksums, or schemas are abnormal;
- a provider returns a throttle, unsupported action, or undocumented shape;
- the daily or per-symbol budget would be exceeded.

The narrower safe alternatives are: omit the optional metric if the model contract
permits omission; keep the candidate run unpublished; use a signed licensed source;
or defer activation. Never broaden a host/path, infer a timestamp, scrape a public
page, or replace history with a snapshot.
