# Stage6 official source contracts — evidence and operator checklist

Reference companion to `docs/decisions/0004-stage6-official-source-contracts.md`.
That ADR records the decisions; this file records the evidence they rest on, the
gaps they must not paper over, and the checklist an operator has to clear before
Step 2 can write an adapter.

**Historical approval snapshot — 2026-08-19.** The **OpenDART core** surface
(`list.json`/`list.xml`, `corpCode.xml`, `company.json`) was approved for
fixture-based Raw adapter work. **Every other surface here was deferred** and
awaited the approval `AGENTS.md` requires for any change to a method, path,
host, or response contract. This snapshot is retained as history; see the
allowlist table at the end for the current per-row state.

**Current state — 2026-08-21.** The owner supplied an OpenDART key and approved
the live observation needed to settle ETF coverage. **Exactly one request using
the owner-supplied key has been made**: a single
`GET /api/corpCode.xml`, whose purpose and result are recorded in checklist
item 4 below. No other OpenDART API surface has been contacted, and the key
value and all response bodies are intentionally absent from this runbook. KIND
D11 is approved for browser-driven collection, and its capture → immutable Raw →
normalization path is implemented. The FSC listing diagnostic and exact control
are consumed and reject that source for ETF11 identity; no further FSC live or
Raw path is allowed without new explicit owner approval. KIS daily bars and
reference quotes are the primary ETF11 price/volume source, and any alternate
price API needs the D15 approval conditions. KRX, KSD, the full KIND backfill,
and ETF11 identity decisions remain deferred.

## How this evidence was gathered

**Historical research snapshot — 2026-08-19.** Research ran as four
source-scoped passes plus one adversarial verification pass, all read-only:

- public documentation pages only, fetched over HTTP;
- no account registration, no API-key request, no authenticated call, no
  data-endpoint call, no form submission, no login in that snapshot;
- every claim carries the URL it was fetched from; anything not confirmed from a
  fetched page is a typed gap below rather than an inference.

Fetch timestamps fall in the 09:14–09:35 UTC window on 2026-08-19. The fetch
tooling reported minute granularity at best, so per-claim times in the research
output are ordering approximations within that window, not exact instants. Treat
the date as the citation and re-verify before relying on a quote, since terms
pages change — the KRX Open API terms in force were themselves adopted
2025-12-26.

Adversarial verification re-fetched the pages behind the eight claims that drive
a decision. Its corrections are folded in below and marked
`[verified]`, `[corrected]`, or `[partial]`.

## KRX

Three distinct properties, frequently conflated:

| property | host | what it is |
|---|---|---|
| KRX Open API | `openapi.krx.co.kr` | registration- and approval-gated statistics API |
| KRX Data Marketplace / 정보데이터시스템 | `data.krx.co.kr` | statistics site with Excel/CSV/PDF downloads |
| KIND | `kind.krx.co.kr` | corporate-disclosure channel; see its own section |

### Service catalog — what exists and what does not

`[verified]` The catalog at
`https://openapi.krx.co.kr/contents/OPP/INFO/service/OPPINFO004.cmd` is a single
flat, non-tabbed, non-paginated table of **31 entries across 7 categories**:
지수 (5), 주식 (8), 증권상품 (3), 채권 (3), 파생상품 (6), 일반상품 (3), ESG (3).

Relevant to this project:

| need | in the catalog? | entry |
|---|---|---|
| ETF daily trade data | yes | `ETF 일별매매정보`, documented from `2010-01-04` |
| ETF issue basic info | **no** | `종목기본정보` exists only for 유가증권 / 코스닥 / 코넥스 equities |
| listing / delisting / change-of-listing | **no** | no entry |
| administrative issue (관리종목) | **no** | no entry |
| trading halt (매매거래정지) | **no** | no entry |

The catalog header states overall coverage is `2010년 이후 데이터`.

The absence was checked by enumerating the full table, not by failing to notice
an entry, and re-checked by the verification pass, which independently
enumerated all 31 entries and found no match.

### Terms of use — `openapi.krx.co.kr`

`[verified]` All four articles below matched verbatim on re-fetch of
`https://openapi.krx.co.kr/contents/OPP/INFO/OPPINFO002.jsp`. Effective date:
`이 약관은 2025년 12월 26일부터 시행한다.`

- 제6조② — `API 이용자는 API 서비스를 비상업적인 목적으로만 이용할 수 있으며, API 서비스를 이용한 결과에 대한 대가를 제3자에게 청구해서는 아니된다.`
- 제8조④ — `하나의 키당 1일(매일 0시~24시) 10,000회 이하의 요청으로 제한하며, 이를 초과할 경우 서비스가 중지될 수 있다.`
- 제11조② — `API 이용자는 한국거래소로부터 제공받은 정보를 제3자에게 제공할 수 없다.`
- 제11조③ — `API 이용자는 이용기간 만료, 해지 등의 사유로 이용계약이 종료된 이후에는 한국거래소로부터 제공받은 정보를 이용할 수 없다.`

Also recorded from the same page: key validity is one year from issuance,
renewable; an unused key may be deleted after twelve months without prior
notice; screens built from results must state they use `한국거래소 통계정보`
(제10조③); accuracy, completeness, and continued provision are disclaimed
(제12조②⑤).

제11조③ is the clause that collides with indefinite immutable Raw retention.
See ADR-0004 D3.

### Access path

Key issuance requires Data Marketplace membership, identity or business
verification, administrator approval of the key, and then a further
administrator approval per API service. The key is sent as an `AUTH_KEY`
request header.

### Financial Services Commission mirrors on the public data portal

`[verified]` Both pages state `이용허락범위: 제한 없음` and `비용부과유무: 무료`,
and both leave `시간범위` blank (`-`).

| dataset | page | scope |
|---|---|---|
| 금융위원회_KRX상장종목정보 | `https://www.data.go.kr/data/15094775/openapi.do` | 기준일자, 단축코드, ISIN코드, 시장구분, 종목명, 법인등록번호, 법인명; queried by reference date or ISIN |
| 금융위원회_증권상품시세정보 | `https://www.data.go.kr/data/15094806/openapi.do` | 3 operations — ETF / ETN / ELW price lookup |

`[partial]` The availability anchor —
`데이터 갱신은 기준일자로부터 영업일 하루 뒤 오후 1시 이후에 업데이트됩니다. 예를 들어, 금요일 데이터는 차주 월요일에 제공됩니다.`
— was confirmed on the `KRX상장종목정보` page and **could not be located on the
`증권상품시세정보` page** across repeated fetches. Do not assume it applies to
both. The `증권상품시세정보` page defers field-level detail to a downloadable
`활용가이드` `.docx` that read-only fetching cannot open.

`[verified]` The sentence describes a service-wide refresh cadence, not a
per-record response field. ADR-0004 D2 depends on this distinction.

**Credentialed coverage observation — 2026-08-21.** The owner-approved
`KRX상장종목정보` adapter used only exact `basDt` + `isinCd` JSON queries. ETF
`069500` / `KR7069500007` was absent on the five validated XKRX sessions
`2026-08-20`, `2026-08-19`, `2026-08-18`, `2026-08-14`, and `2026-08-13`.
One separately approved, exact-once, non-persisting official-guide control —
stock `000020` / `KR7000020008` at `2026-08-19` — was present. This rejects the
listing mirror as a complete ETF11 identity source without claiming universal
ETF absence. No key, response body, or provider message was recorded; no FSC
Raw batch or manifest was committed. The diagnostic and control are fully
consumed: no further live FSC query or FSC Raw collection is allowed without a
new explicit owner approval. Offline fixture/contract code may remain. See
ADR-0004 D14.

### Price and volume source priority

KIS read-only daily bars and reference quotes are the primary price/volume
source for ETF11. The overlapping FSC `증권상품시세정보` surface is not a
default follow-up and must not be queried as one. Any other price API
requires a separate explicit owner approval that names its independent
cross-check or fallback purpose, priority, failure behavior, official contract,
entitlement reference, and focused tests.

### Terms of use — `data.krx.co.kr`

`[corrected]` One research pass cited
`MDCINFO003.cmd` as a statistics menu tree. It is not.

| url | what it actually is |
|---|---|
| `https://data.krx.co.kr/contents/MDC/INFO/informationController/MDCINFO003.cmd` | `홈페이지 이용약관` — the Terms of Use, 16 articles in 5 chapters |
| `https://data.krx.co.kr/contents/MDC/INFO/informationController/MDCINFO002.cmd` | `이용안내` — the statistics menu tree (기본 통계 / 이슈 통계 / 쉽게 보는 통계) |

Any downstream citation pointing at `MDCINFO003.cmd` for menu structure is
wrong. From the terms page itself:

- 제10조 (금지행위) ② — `자동화 수단을 이용하여 정보를 무단 수집·복제·배포하는 행위`
- 제12조 (이용자의 의무) ② — `이용자는 당 사이트의 정보를 거래소의 사전 허락 없이 복사·복제·배포·전송·공중송신하여서는 아니 됩니다.`
- 제12조의2 — a separate `마켓데이터 이용약관` governs any purchased or used
  마켓데이터 product. Not fetched; contents unknown.

### Statistics products covering the missing categories

`[verified]` The leaf enumeration below was originally cited to
`MDCINFO003.cmd`, which the verification pass proved is the Terms of Use page,
and the verification pass had confirmed `MDCINFO002.cmd` only at heading level.
The coordinator therefore re-fetched
`https://data.krx.co.kr/contents/MDC/INFO/informationController/MDCINFO002.cmd`
directly on 2026-08-19 (HTTP 200) and confirmed every label below is present in
that page's served content — `전종목 시세`, `전종목 기본정보`,
`개별종목 종합정보`, `괴리율 추이`, `장마감 괴리율 추이`, `추적오차율 추이`,
`PDF(Portfolio Deposit File)`, `신규상장종목 현황`, `상장폐지종목 현황`,
`매매거래정지종목 현황`, `매매거래정지 내역(개별종목)`,
`관리종목 지정 내역(개별종목)`, `지정일전후 등락률`.

From the `이용안내` menu tree: 증권상품 > ETF offers `전종목 시세`,
`전종목 기본정보`, `개별종목 종합정보`, `괴리율 추이`, `장마감 괴리율 추이`,
`추적오차율 추이`, and `PDF(Portfolio Deposit File)`. The 이슈 통계 section
offers `신규상장종목 현황`, `상장폐지종목 현황` and `시세 추이`,
`매매거래정지종목 현황` and `매매거래정지 내역(개별종목)`, `관리종목 현황`,
`관리종목 지정 내역(개별종목)`, and `지정일전후 등락률`.

So the four categories absent from the API do exist as named official statistics
products — as web pages with Excel/CSV/PDF export, not as an API. Leaf URLs,
field schemas, the download mechanism, and any point-in-time metadata are all
unconfirmed; the pages are JS-rendered and were not exercised.

The presence of per-issue `지정 내역` and `지정일전후 등락률` implies a tracked
`지정일` exists as a discrete historical record rather than only a live flag.
Whether it is an announcement date or an effective date, and whether a `해제일`
is tracked separately, is unconfirmed.

## OpenDART

### Disclosure list API

`[verified]` `https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019001`

- `GET https://opendart.fss.or.kr/api/list.json` (also `.xml`)
- auth: `crtfc_key` query parameter
- params: `corp_code`, `bgn_de`, `end_de`, `last_reprt_at`, `pblntf_ty`,
  `pblntf_detail_ty`, `corp_cls`, `sort`, `sort_mth`, `page_no`,
  `page_count` (1–100, default 10)
- pagination: `page_no`, `page_count`, `total_count`, `total_page`
- 15 response fields: `status`, `message`, `page_no`, `page_count`,
  `total_count`, `total_page`, `corp_cls`, `corp_name`, `corp_code`,
  `stock_code`, `report_nm`, `rcept_no`, `flr_nm`, `rcept_dt`, `rm`
- without `corp_code`, the search window is limited to three months
- status codes distinguish no-data (`013`) from errors (`010`–`012`, `014`,
  `020`–`021`, `100`–`101`, `800`, `900`–`901`); `000` is success

### Wire shape of the list envelope

`[verified]` The coordinator re-fetched the guide page on 2026-08-19 (HTTP 200)
to settle the row-array key, which the first research pass had not recorded. The
official `응답 결과` table lists the envelope keys `status`, `message`,
`page_no`, `page_count`, `total_count`, `total_page`, then the key **`list`**,
and under it the per-row keys `corp_cls`, `corp_name`, `corp_code`,
`stock_code`, `report_nm`, `rcept_no`, `flr_nm`, `rcept_dt`, `rm`. So `list` is
the documented row-array key, not an assumption.

Two shape questions the documentation does **not** settle, both first-use risks
for the adapter rather than gaps in the decision record:

- The table names the four envelope integers but never states their JSON type,
  while the matching *request* parameters are typed `STRING`. The adapter
  therefore accepts either a JSON number or a digit-only JSON string for
  `page_no`, `page_count`, `total_count`, and `total_page`, and still fails
  closed on anything that is not a well-formed non-negative integer. Insisting
  on one representation would have been an inference.
- `rm` is listed as a response key, but nothing states whether it is present on
  every row or omitted when there is no remark. The adapter currently requires
  it, which is the fail-closed reading. If real traffic omits it on
  remark-free rows, that surfaces as a typed `UndocumentedShape` on first use
  rather than as silent corruption — see checklist item 16.

Note also a typo in the official table itself: the description column for
`total_count` reads `총 페이지 수` rather than `총 건수`. The key names are
unaffected.

### Timestamp granularity

`[verified]` `rcept_dt` is documented as `공시 접수일자(YYYYMMDD)`. The full
15-field schema was enumerated and **contains no time-bearing field** — nothing
described with 시각, 시간, `HHMM`, or `HHMMSS`. The structured decision-report
APIs do not even expose `rcept_dt`; they expose `rcept_no` only.

Whether `rcept_no`'s leading digits encode the receipt date is a widespread
assumption with **no supporting statement on any fetched page**. It is not
treated as documented.

### Correction and withdrawal lineage

`[verified]` Signalling is advisory text, not a structured reference. The `rm`
(비고) field is a combined-character code:

- `정` — `본 보고서 제출 후 정정신고가 있으니 관련 보고서를 참조하시기 바람`
- `철` — `본 보고서는 철회(간주)되었으니 관련 철회신고서(철회간주안내)를 참고하시기 바람`
- other characters mark the supervising body (유, 코, 채, 넥, 공) or
  consolidated scope (연)

`report_nm` additionally carries bracket tags — `[기재정정]`, `[첨부정정]`,
`[첨부추가]`, `[정정명령부과]`, `[정정제출요구]`.

**There is no field linking a correction to the original filing's `rcept_no`.**
Walking a correction chain programmatically needs matching logic on corp code,
date window, and report subject — not a join.

### Corporate-action decision APIs

`[verified]` The `주요사항보고서 주요정보` group (DS005) lists **36 APIs**. It
covers 유상증자 결정, 무상증자 결정, 유무상증자 결정, 감자 결정, 회사합병 결정,
회사분할 결정, 회사분할합병 결정, 주식교환·이전 결정, and treasury-share,
asset-transfer, and bond-issuance decisions.

It contains **no 배당결정, no 주식분할결정, and no 주식병합결정**. The
verification pass also enumerated DS001 (4), DS002 (30), DS003 (7), DS004 (2),
and DS006 (6) and found no dividend-*decision* endpoint in any of them.

`[corrected]` DS002 does contain a dividend-related API, `배당에 관한 사항`. It
is a periodic-report, backward-looking realized-amount summary
(`thstrm` / `frmtrm` / `stlm_dt`) without decision, record, or payment date
fields. The accurate statement is "no dividend-*decision* event API", not "no
dividend data".

Three DS005 endpoints were opened in detail — `crDecsn` (감자),
`piicDecsn` (유상증자), `cmpMgDecsn` (회사합병). All three carry the same note:
`검색시작 접수일자(YYYYMMDD) ※ 2015년 이후 부터 정보제공`. Their event date
fields include `bddd` / `bdds` (이사회결의일), `cr_std` (감자기준일),
`crsc_nstklstprd` (신주상장예정일), and `mgsc_mgdt` (합병기일).

### Live-path blocker — rustls cannot complete a handshake

`[verified]` Observed against the live host on 2026-08-20:

- TLS 1.3 is refused with a protocol-version alert; the host is **TLS 1.2 only**.
- The negotiated suite is `AES128-GCM-SHA256` — static RSA key exchange, **no
  forward secrecy**.
- Restricting the client offer to ECDHE suites is refused with a handshake
  failure alert.

rustls implements no non-forward-secret key exchange, so the workspace's
one-TLS-stack policy makes the in-process `opendart-client` transport unable to
reach this host at all. The certificate chain itself is fine (GlobalSign Root
CA - R3, and the SAN covers `opendart.fss.or.kr`), so this is a cipher-suite
incompatibility, not a trust problem. ADR-0004 D10 records the options; the
choice is the owner's.

### The `.xml` surface returns XML errors, with HTTP 200

`[verified]` Before the owner key was supplied, a deliberately invalid
non-owner credential diagnostic against `/api/corpCode.xml` established that
the endpoint returns **HTTP 200** with an XML status envelope rather than JSON.
The response bytes and provider message are intentionally not reproduced here;
only the documented status code and envelope kind are retained.

Two consequences. The transport cannot detect this from the status line, so
validation must, and status `010` is the documented "unregistered key" code. The
adapter's non-ZIP branch originally recognised only a JSON envelope and would
have reported a shape complaint instead of the real status — corrected, with the
`<message>` prose deliberately never crossing into an error.

### Corp identification

`GET https://opendart.fss.or.kr/api/corpCode.xml` returns a zip containing XML
with `corp_code` (8 digits), `corp_name`, `corp_eng_name`, `stock_code` (the
6-digit KRX ticker, for listed companies), and `modify_date`. This is the join
key from a KRX short code to a DART entity — and the only way to settle the ETF
coverage question, which needs a key.

### ETF coverage

**Historical documentation gap — 2026-08-19 (superseded by the 2026-08-20
observation below).** `corp_cls` had only `Y`(유가) / `K`(코스닥) / `N`(코넥스)
/ `E`(기타), with no fund bucket. `pblntf_ty` did include `G: 펀드공시`, but
its only documented sub-codes were `G001`–`G003`, all
collective-investment-securities registration statements. The structured
securities-registration group (DS006) excluded collective investment
securities. The corp-code field was described as
`공시대상회사의 고유번호(8자리)` without defining whether an investment trust
qualified.

**Resolved 2026-08-20.** With the owner-supplied key, exactly one
`GET /api/corpCode.xml` observation found 118,714 entities, 3,984 non-empty
`stock_code` values, and none of the eleven ETF short codes. Listed-company
controls in the same archive resolved, so this is a measured absence rather
than an inferred schema gap. OpenDART therefore has no ETF11 use: the approved
core fixture contract remains available for future disclosure-entity scopes,
while D10 blocks its in-process live path for now. The key value and response
body are not recorded here.

### Rights and quota

- `금융감독원이 제공하는 서비스는 원칙적으로 무료입니다`
- Art. 10(4) — `오픈API 서비스는 이용횟수에 허용량 제한이 있으며, 이를 홈페이지에 게시합니다`
- Art. 16(1) — copyright in the API service and related programs belongs to the FSS
- Art. 23(1) — disclosure content is the filer's responsibility; the FSS
  warrants neither accuracy nor completeness

**No numeric daily quota appears on any fetched official page.** The widely
cited 20,000-per-day figure was found only in third-party material and is
deliberately excluded. No clause on indefinite retention or redistribution of
API responses was located in the articles reviewed; that is a gap, not
permission.

## KIND

### No public API

No documented programmatic API exists on `kind.krx.co.kr`. The inspected pages
expose human-facing HTML search forms, but export availability is page-specific:
the detailed-search, administrative-issue, trading-halt, and listed-company
pages expose the `EXCEL` controls recorded in the table below, while the
ETF-scoped disclosure list was verified on 2026-08-20 to have **no `EXCEL`
export**. No export is inferred for a KIND page that is not explicitly listed.
There is no developer portal, no key issuance, and no API documentation link.
`robots.txt` returns 404.

This matches the §0.4 premise that KIND need not be an API.

### Candidate artifacts

| purpose | page | export | operator controls |
|---|---|---|---|
| detailed disclosure search | `https://kind.krx.co.kr/disclosure/details.do?method=searchDetailsMain` | EXCEL (observed) | 기간 date range, market, security type (includes ETF and 주권), disclosure-type checkboxes including `정정공시 요구`, company name/code |
| ETF-scoped disclosure list | `https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf` | **no EXCEL export** (corrected 2026-08-20); the search response itself is the artifact | 기간, and a per-issue field that does not filter |
| administrative issue (관리종목) | `https://kind.krx.co.kr/investwarn/adminissue.do?method=searchAdminIssueList` | EXCEL (observed) | market (전체/유가증권/코스닥), stock name |
| trading halt (매매거래정지) | `https://kind.krx.co.kr/investwarn/tradinghaltissue.do?method=searchTradingHaltIssueMain` | EXCEL (observed) | market, stock name |
| listed-company roster | `https://kind.krx.co.kr/corpgeneral/corpList.do?method=loadInitPage` | EXCEL (observed) | corporate/market classifications |

No dedicated standalone export page was found for new/changed listings or for
issue name changes; those appear to be reachable only through the
disclosure-type filters on 상세검색.

The `export` column is an observation of the named page only. It is not a
site-wide KIND capability claim; the ETF disclosure response is the observed
Raw artifact, not an Excel download.

### ETF coverage

**Historical static-fetch finding — 2026-08-19.** An ETF-scoped disclosure page
existed, and the 상세검색 security-type filter listed ETF alongside 주권.
**None of the 11 short codes was individually queried** in that pass because
static fetching could not drive the AJAX form.

**Current D11 scope — 2026-08-20.** The ETF-scoped list is captured through the
site's own browser controls, but per-issue popup filtering is intentionally not
reproduced. Raw keeps the complete response and normalization filters locally
by `종목명`; this does not claim that any ETF11 identity has been resolved.

### Timestamp granularity

**Historical static-fetch sample — 2026-08-19 (superseded by D11).** Two
disclosure-viewer documents were sampled. Both showed calendar dates only
(`신규상장 (2025.09.26)`; version dates `2025.11.05`, `2026.01.22`) with no
hour, minute, or timezone. This was a **two-document sample**, not a
system-wide conclusion: a receipt time could exist in a results-table column
that static fetching could not render. It was recorded as a gap at that time.

**Current D11 observation — 2026-08-20.** The ETF-scoped results table carries
`시간` as `YYYY-MM-DD HH:MM` per disclosure and varies within a page, so
disclosure-list `available_at` is minute-granular. The page does not document a
timezone; normalization therefore records the explicit `AssumedAsiaSeoul`
assumption. Correction-version entries remain date-only (see item 11), so this
minute value is not fabricated for a version entry.

### Correction linkage

**Historical static-fetch finding — 2026-08-19 (superseded/qualified by item
11).** The integrated-search results list used a `정정있음` marker, and 상세검색
had a `정정공시 요구` category filter, so correction status was tracked at list
level. On the sampled documents, only a textual notice appeared —
`본 공시는 공시내용 기재 불충분 등의 사유로 한국거래소 정정요구를 받은 사항입니다`
— with no machine-followable reference to the pre-correction filing. Whether
such a field existed was a gap in that static pass.

The current browser observation is recorded in item 11: the version chain is
enumerable and ordered, but version labels are date-only and no original
acceptance-number reference was exposed.

### The KIND Raw artifact — settled 2026-08-20

Measured with the site's own search control, so the design rests on observation
rather than assumption.

**There is no EXCEL export on the ETF-scoped disclosure page.** Enumerating every
link, button, and input on it turns up exactly one export-like control,
`뷰어다운로드`, which opens a document-viewer popup. An earlier research pass
reported an EXCEL button here; that is **corrected** — it does not exist on this
page.

**The artifact is the search response itself, and it is byte-stable.** When
`fnSearch()` runs, the page issues:

```
POST https://kind.krx.co.kr/disclosure/disclosurebystocktype.do
method=searchDisclosureByStockTypeEtfSub&forward=disclosurebystocktype_etf_sub
&currentPageSize=15&pageIndex=1&orderMode=1&orderStat=D&etfIsuSrtCd=&…
```

For a closed historical window that response was **13,085 bytes with an
identical SHA-256 across two separate runs**, and it carries both the `시간`
label and the per-disclosure times. So the official server bytes are hashable
immutable Raw, obtained through the site's own request rather than a
reconstructed one — which is exactly what ADR-0004 D11 requires.

**Raw stores the response unfiltered.** Three attempts at per-issue filtering all
returned the unfiltered page: `searchCorpName` with a code, the hidden
`repIsuSrtCd`, and the ETF page's own `etfIsuSrtCd` (the field exists and can be
set, but the page evidently needs companion state its popup supplies). Probing
stopped there. This costs nothing, because filtering belongs after Raw anyway:
the list is already restricted to ETF-type issues and carries `종목명` per row, so
selection happens at normalization, where it is reversible and auditable. Storing
the complete official response is the better lineage in any case — Raw is
provider bytes, not a projection of them.

**Consequence for the ingest path.** The capture stage is necessarily a browser,
so the hash must not be trusted from it: the ingesting Rust path recomputes the
SHA-256 from the bytes on disk and records the interaction (URL, the form fields
the page sent, retrieval time) as the request metadata. Pagination is
`pageIndex` at `currentPageSize=15`, so a date range spans multiple responses,
each one its own Raw file.

### Amendment — capture-completeness contract (2026-08-20)

The browser capture and Rust ingest now share an explicit completeness
contract. This amendment resolves a review finding in the capture boundary; it
does not change D11's browser-only access mechanism, the entitlement boundary,
or the historical observations above. No live provider call was made for this
remediation.

- Every `capture.json` requires `termination` with exactly one of
  `clamped_duplicate`, `page_bound_reached`, `advance_control_missing`, or
  `no_response`. Only `clamped_duplicate` is complete.
- For the initial search and every later page, capture waits once, invokes the
  same site control exactly once more if no response was captured, then waits
  once more. Two missed waits are `no_response`; loss of the page control is
  `advance_control_missing`.
- The configured stored-page limit is at most 40. Capture makes one additional
  probe after the configured limit: bytes identical to the last stored page are
  `clamped_duplicate`; distinct bytes are `page_bound_reached`. The probe page
  is never staged as a stored page.
- Every incomplete outcome is retained only as diagnostic staging and exits
  non-zero. `kind-raw` rejects missing, unknown, or incomplete termination
  before Raw storage, and `market-data` independently rejects an incomplete
  termination before any batch commit. Old staging without `termination` must
  be recaptured; immutable batches are not rewritten.
- `form_fields` records ordered, URL-decoded name/value pairs, preserving
  repeated names and order. It is not a byte-exact copy of the encoded POST
  body.
- A table's leading `번호` remains unsuitable as a result total, especially on
  상세검색. Completeness is established only by the capture termination
  contract; no total is inferred from that cell.

### Backfill volume — a retracted measurement, and what actually holds

**Retracted 2026-08-20.** An earlier version of this section claimed that
narrowing on 상세검색 cut a full backfill from ~475 captures to ~6 — a 60x
reduction — citing ~525 relevant ETF disclosures a year. **That claim was wrong
and is withdrawn.** Both halves of it failed verification:

- The counts were read from the result table's leading `번호` cell on page 1, on
  the assumption that it is the result total. It is not reliable as one here: a
  later page of the same capture carried `번호` values near 9,490 in a set whose
  first page implied 485, and the numbering was discontinuous across pages.
- More seriously, a capture against 상세검색 does not page one result set. A
  request for `2020-02-01..2020-02-29` returned rows spanning only
  `2020-02-17..2020-02-28`, and a request for `2020-01-31..2020-12-31` returned
  December alone. The security-type and date values set before the first search
  are not carried into `fnPageGo`'s later requests, so each page is effectively a
  different query, and any volume figure derived from it is meaningless.

The retarget script was deleted rather than kept: a capture path that silently
returns a different window than the one requested is worse than none. Probing
further at the form state was stopped, as it was for the per-issue filter.

**What does hold** — these are observations of row content, not of counts:

- 상세검색 carries a 유가증권구분 selector (`securities`, ETF = `5`) and the
  disclosure-type checkboxes the ETF-scoped page lacks.
- ETFs pay 분배금, not 배당. There is no ETF 분배금 type — only ETN has one
  (`2010`) — and `0113 배당` surfaces nothing useful for ETFs. Distribution
  evidence appears under `0303 권리락/배당락/기준가격`, whose returned rows are
  `ETF 분배락 기준가격 안내`. A narrow set including `0303` therefore does not
  drop distribution evidence.
- Candidate point-in-time types, by the page's own group and value: `0321`
  신규/추가/변경/재상장, `0328` 상장폐지, `0350` 관리종목, `0303`
  권리락/배당락/기준가격, `0346` 매매거래중단/재개, `0344`/`0345`/`0357`
  매매거래정지 계열, `0364` ETF 투자유의종목, `0318` 소속부변경, `0322` 업종변경,
  `0360` 매매방식변경, `0428` 상호변경, `0120` 상장폐지결정, `0604` 분할, `1305`
  분할/합병.

**What remains verified on the working surface.** The ETF-scoped page pages one
coherent set, checked row by row rather than assumed: 473 rows, `번호` descending
473 to 1 with **no gaps**, and a date span exactly equal to the requested
`2020-02-03..2020-02-07`. Two independent captures of that window were
byte-identical across all 32 pages.

**So the backfill cost stands at the ETF-scoped figure**: ~95 disclosures a day,
roughly 6 days per 40-page capture, therefore about 475 captures and ~15,200
requests for the full range. That is **not** low volume, and ADR-0004 D11 was
granted on low volume. A full backfill therefore needs either a narrowed surface
that actually pages correctly, or an explicit decision about the request budget.
It should not be started on the current evidence.

### How KIND was actually inspected

KIND's search cannot be driven over plain HTTP. The form page itself serves real
HTML, and its search target is discoverable (`method=searchDetailsSub`,
`forward=details_sub`), but the POST is refused with a generic
`잠시 후 다시 이용해 주세요` page even with a session cookie — the endpoint needs
state the page's JavaScript produces. Guessing at parameters was stopped rather
than continued, since that probing is the automated-collection behaviour this
document treats as prohibited.

**D11 diagnostic snapshot — 2026-08-20.** The findings were obtained by
driving the site's **own** search control in a real browser engine (Playwright
with headless Chromium, installed outside the repository under
`~/tools/kind-probe`; the missing `libasound.so.2` was extracted into
`~/tools/pwlibs` without root). That distinction mattered: the page's
`fnSearch()` was invoked rather than a reconstructed request, no export was
downloaded, and no dataset was collected — only rendered table structure was
read, a handful of page loads in total.

**Current state — 2026-08-21.** D11 supersedes D6's operator-only restriction
for the approved low-volume KIND browser path: capture records the site's own
responses, Rust ingest commits immutable Raw, and normalization is implemented.
The capture remains browser-driven and never reconstructs requests. Its runtime
scope is manual operator confirmation for exactly one calendar day. D6 still
governs `data.krx.co.kr`; scheduled/timer activation, bulk capture, and
full-history backfill remain forbidden. The full KIND backfill budget and ETF11
identity remain deferred.

### Rights

KIND's footer links to KRX pages on `info.krx.co.kr`, not to KIND-specific
terms. The legal notice there prohibits unauthorized reproduction and
redistribution and asserts KRX ownership of copyright, but **contains no
automated-collection clause**. The `이용자유의사항` page is account-security
guidance and has none either.

The explicit automated-collection prohibition exists on the sibling
`data.krx.co.kr` terms page (제10조② above). Whether it governs KIND is
unresolved. ADR-0004 D6 applies the stricter reading.

## KSD / SEIBro

### Access surfaces

A registration-gated `증권정보 오픈 API` exists at `api.seibro.or.kr` —
portal membership plus a service key, one key per user, provider-defined traffic
caps, separate development and production account tiers.

KSD data is also republished through the public data portal. Confirmed datasets:

| dataset | page | scope |
|---|---|---|
| 금융위원회_주식권리일정정보 | `https://www.data.go.kr/tcs/dss/selectApiDataDetailView.do?publicDataPk=15059609` | dividend, 무상증자, 유상증자, 주식교환, 감자 schedules |
| 금융위원회_주식배당정보 | `https://www.data.go.kr/data/15043284/openapi.do` | dividend schedule and amounts |
| 한국예탁결제원_주식정보서비스_GW | `https://www.data.go.kr/data/15157413/openapi.do` | 주식, 배당, 종목정보, 의무보호, 상장여부, 주식관련사채, 신주인수권증서 |
| 한국예탁결제원_REPO정보서비스_GW | `https://www.data.go.kr/data/15157427/openapi.do` | REPO only — not corporate actions |

Legacy KSD dataset IDs `15001153`, `15001145`, and `15074626` return HTTP 404 on
both URL forms while the `_GW` siblings load. The reason is unconfirmed; only
third-party material suggested a migration.

### Documented fields, and the availability finding

`[partial]` Visible output fields:

- 주식권리일정정보 — `권리행사시작일자`, `권리행사종료일자`, `주식결산월일`,
  `명부폐쇄시작일자`, `명부폐쇄종료일자`
- 주식배당정보 — `배당기준일자`, `현금배당지급일자`, `주식교부일자`,
  `주식일반배당금액`, `주식차등배당금액`, `배당률`, `주식종류`, `배당사유`
  코드와 명칭, 명의개서대리인 정보

**No field named 공고일, 공시일, or 공표일시 appears in either page's visible
field documentation** — every date is a schedule date. The verification pass
looked for such a field deliberately and did not find one.

This is marked `[partial]` rather than confirmed for an honest reason: both
pages defer the authoritative field list to a downloadable `활용가이드` `.docx`
that read-only fetching cannot open. Nothing found contradicts the finding, but
completeness is unverified. Resolving this is checklist item 6.

Both datasets state a once-daily refresh published from 13:00 on the business
day after the base date. Per ADR-0004 D2 that is a service cadence, not a
per-record timestamp.

### Categories with no confirmed public surface

No public KSD/SEIBro API or download was found for **merger (합병), corporate
split (분할), stock split (액면분할), or reverse split (액면병합)**. The only
confirmed coverage is dividend plus the combined rights schedule. Two of the six
approved KIS `ksdinfo` categories — `merger-split` and `rev-split` — may
therefore have no public KSD counterpart at all.

### Correction and revision semantics

No fetched page documents a revision marker, version, `정정` flag, or
supersession rule for any KSD corporate-action record. Recorded as a gap, not a
confirmed absence — the SEIBro portal itself could not be rendered.

### Portal rendering limitation

`seibro.or.kr` and `openplatform.seibro.or.kr` are WebSquare JS applications.
Every fetch returned a bare title or mis-encoded text, never the page body —
including the `법적고지` page, which is where automated-collection language would
live. `robots.txt` on `seibro.or.kr` and `api.seibro.or.kr` returns HTTP 200
carrying a generic KSD error page rather than crawl directives.

**Absence of evidence here is not evidence of absence.** Any decision touching
SEIBro's own pages needs a JS-capable browser and a human.

### Rights

`[verified]` Every KSD-sourced public-portal dataset examined carries
`공공저작물 : 출처표시, 상업적 이용금지 (제 2유형)`. The KOGL definition of
Type 2 states
`상업적 이용이 금지된 공공저작물은 영리행위와 직접 또는 간접으로 관련된 행위를 위하여 이용될 수 없습니다`,
with commercial use possible only under separate permission from the issuing
institution. One dataset page names `portal@ksd.or.kr` as the contact.

No statement on automated collection or crawling was found for either the SEIBro
portal or the portal-hosted KSD APIs.

### KIS `ksdinfo` versus a direct KSD source

`AGENTS.md` already allowlists six read-only `ksdinfo` GET endpoints —
`paidin-capin` (HHKDB669100C0), `bonus-issue` (HHKDB669101C0),
`dividend` (HHKDB669102C0), `merger-split` (HHKDB669104C0),
`rev-split` (HHKDB669105C0), `cap-dcrs` (HHKDB669106C0) — with a specific
continuation policy, a ten-page bound, and fail-closed handling of repeated
bytes. Only bonus-issue is auto-normalized today; other non-empty types stop the
pipeline.

#### Official `bonus-issue` workbook contract — verified 2026-08-20

The repository-local official KIS workbook
`docs/kis_openapi_entiredocs_20260818_030007.xlsx` was inspected offline. It is
intentionally untracked and was neither modified nor added to Git. The exact
file inspected is pinned as
`sha256:993672501204722da88ebc30753d73406b33b23b83db8ac6670e4d83903fbac3`.
Sheet `예탁원정보(무상증자일정)` (`domestic-stock-144`) specifies:

- live-only `GET /uapi/domestic-stock/v1/ksdinfo/bonus-issue`, TR ID
  `HHKDB669101C0`; the mock environment does not support it;
- request fields `CTS` (blank), `F_DT`, `T_DT`, and optional `SHT_CD`;
- response fields `record_date`, `sht_cd`, `isin_name`, `fix_rate`,
  `odd_rec_price`, `right_dt`, `odd_pay_dt`, `list_date`,
  `tot_issue_stk_qty`, `issue_stk_qty`, and `stk_kind`;
- `fix_rate` is a percentage, not a fractional multiplier. Official examples
  include `50.00`, `100.00`, `1000.0`, and `3900.0`; therefore a 100% bonus
  maps to the canonical split factor `2`, not `101`;
- `odd_pay_dt` may be blank or a displayed date range such as
  `YYYY/MM/DD ~ YYYY/MM/DD`, so it is not a strict `YYYYMMDD` field.

The sheet describes this information as supplied by Korea Securities
Depository. It contains no announcement/publication timestamp, revision ID,
predecessor/supersedes link, correction lineage, or ISIN code (only the display
field `isin_name`). Thus KIS can establish the event schedule and factor, but
cannot by itself establish disclosure availability or correction lineage. The
normalizer continues to use verified retrieval time as the only availability
evidence and cannot promote the event to Curated/PIT without a deterministic
KIND relation.

The workbook says that request/response `tr_cont` cannot be used for a next
lookup. This conflicts with the current official GitHub sample recorded in the
KIS backfill runbook. Per the existing approved contract, the sample's narrow,
bounded KSD continuation rule remains authoritative; the discrepancy is not
generalized to any other endpoint.

| dimension | keep KIS `ksdinfo` | add direct KSD/SEIBro |
|---|---|---|
| independence | official workbook confirms `bonus-issue` is KSD-supplied; the other five categories were not inferred | same origin, so neither corroborates the other independently |
| availability timestamp | `bonus-issue` documented response has none; the other five categories remain unchecked | confirmed absent in visible fields (above) |
| field completeness | one category normalized, five fail closed | dividend plus combined rights schedule only; merger/split/reverse-split absent |
| rights | KIS own-asset use, no redistribution — already recorded | KOGL Type 2, non-commercial, separate permission for commercial use |
| implementation cost | already implemented, rate-limited, allowlisted | new registration, allowlist, parsing, and fail-closed logic |

The workbook closes the earlier documentation question: `bonus-issue` is
KSD-supplied and its documented response has no announcement timestamp or
lineage field. This conclusion comes from the official workbook; no KIS live
request was made for this review.

## Operator verification checklist

Step 2 depends on these. Each needs a human with a browser, or an authorized
credentialed call, because read-only public fetching cannot settle it.

1. **KRX Open API specifications.** Register on Data Marketplace, obtain key
   approval, and capture from the logged-in `개발 명세서`: exact host and path,
   HTTP method, full parameter set, response schema, pagination mechanism and
   terminal condition, and any correction or revision field — for
   `ETF 일별매매정보` at minimum. Everything technical is behind this wall.
2. **FSC mirror specifications.** **CONSUMED / CLOSED FOR ETF11 2026-08-21:**
   the official `KRX상장종목정보` guide and live JSON envelope were resolved,
   then D14 rejected the source as a complete ETF11 identity source. No further
   FSC live query or Raw collection is allowed without new explicit owner
   approval. The separate `금융위원회_증권상품시세정보` ETF operation is not a
   default follow-up; it needs a new independent cross-check/fallback decision
   before any contract work or request.
3. **Snapshot versus live semantics.** **DEFERRED FOR INDIVIDUAL-STOCK SCOPE:**
   two past-date observations are no longer on the ETF11 critical path because
   D14 closed this source for ETF11 completeness. Do not infer validity
   intervals from the diagnostic observations.
4. ~~**ETF coverage in OpenDART.**~~ **CLOSED 2026-08-20 — answer: no.** One
   live `corpCode.xml` request returned a 3,596,991-byte ZIP holding a
   28,585,431-byte `CORPCODE.xml` with 118,714 entities, 3,984 of them carrying
   a non-empty `stock_code`. **None of the eleven ETF short codes appears:
   0 of 11.** Controls in the same file resolve (`005930`, `000660`, `035420`),
   so the absence is real rather than a method failure. 458 entities match
   `자산운용`, all without a `stock_code` — the asset manager is the filer and
   the ETF is not a disclosure entity. `KODEX` and `상장지수` match nothing. See
   ADR-0004 D4 for the consequences; the archive was read only, never unzipped
   into the Raw zone.

   Evidence is pinned by hash rather than by a path in volatile storage. The
   retrieved archive is held outside the repository at
   `~/lagrange-evidence/opendart-corpcode-20260820.zip`, mode `0400`,
   `sha256:10a904780661b2c4002632c46b6d431be184f6a1cd6abed4a6a4c14f33be651d`.
   It carries no credential — the key appears nowhere in the archive, checked
   directly. **Scope caveat:** this is one snapshot of a master file DART
   regenerates. The finding is "as of the 2026-08-20 master file", not a claim
   about every past or future edition.
5. **OpenDART quota.** Find the actual posted numeric limit referenced by
   Art. 10(4), from the key-management dashboard or the homepage notice. Do not
   inherit the third-party figure.
6. **KSD field completeness.** Open the `활용가이드` `.docx` for
   `주식권리일정정보` and `주식배당정보` and confirm whether any announcement or
   publication date field exists beyond the visible schedule dates. A found
   field would materially strengthen Stage6.
7. **KSD catalog completeness.** Browse the portal's KSD-provider dataset list
   in a real browser to enumerate current dataset IDs, and establish whether any
   public surface exists for merger, split, stock split, or reverse split.
   Contact `portal@ksd.or.kr` if not.
8. **SEIBro terms and ETF pages.** Load
   `https://seibro.or.kr/websquare/control.jsp?w2xPath=%2FIPORTAL%2Fuser%2Fetc%2FBIP_CMUC01030V.xml&menuNo=471`
   in a JS-capable browser and record the legal notice, specifically any
   automated-collection clause; then inspect the ETF distribution pages for
   field structure.
9. ~~**KIS `ksdinfo` lineage and timestamps.**~~ **CLOSED FOR THE APPROVED
   `bonus-issue` PILOT 2026-08-20 — answer: KSD-supplied schedule, no
   availability or lineage field.** The official workbook sheet
   `예탁원정보(무상증자일정)` explicitly describes KSD-supplied information and
   documents only schedule/factor fields. It contains no announcement time,
   revision ID, predecessor, supersedes, or correction link. ADR-0004 D5
   therefore still requires a deterministic KIND relation. The other five
   `ksdinfo` categories remain outside this pilot and were not inferred from
   the bonus-issue sheet.
10. ~~**KIND receipt time.**~~ **CLOSED 2026-08-20 — answer: yes, to the
    minute.** KIND's search will not run over plain HTTP (the POST is refused
    with a generic retry page even with a session cookie), so it was driven in a
    real browser engine. The disclosure list's header row is
    `번호 | 시간 | 회사명 | 공시제목 | 제출인 | 차트/주가`, and `시간` holds
    `YYYY-MM-DD HH:MM` per disclosure. Verified as a real per-record value
    rather than a constant: times differ within one page (`2020-03-31 16:11`
    alongside `16:09`). Verified historically: 15 of 15 rows in a 2020-03-31
    window carried a time. This tightens ADR-0004 D1 to minute granularity for
    KIND-sourced disclosures. Two residual imprecisions are recorded, not
    assumed away — the displayed value's timezone is not stated on the page, so
    normalization records the explicit `AssumedAsiaSeoul` assumption, and
    correction versions are enumerable only by date (item 11).
11. **KIND correction linkage — option-level resolution only, 2026-08-20.**
    The approved ETF observation used list-anchor acceptance `20200207000058`.
    The exact page-1 response was validated, and the rendered viewer had exactly
    one `mainDoc` select: option 0 had an explicit empty value (its rendered
    prompt is not evidence), and the sole real option had raw value
    `20200207000081|Y`, acceptance token
    `20200207000081`, and label date `2020.02.07`.
    The four identical target handlers were accepted; a distinct same-acceptance
    handler would fail closed. The list anchor and option acceptance are not
    equal, so no equality, join, predecessor, supersedes, withdrawal, time, or
    timezone semantics may be inferred. `|Y` is opaque beyond this exact shape.
    This proves option-level acceptance resolution and ordered membership shape,
    not an actual multi-version correction chain. Preserve anchor and option
    acceptance separately, keep dates date-only, and never derive dates from IDs.
    The historical non-ETF direct-viewer sample is not sufficient for ETF
    implementation; `20251204000324` was rejected because it lacks ETF-list
    provenance. Rust Raw ingest and the ordered-membership normalizer now retain
    the two acceptances separately, validate the exact `|Y` option shape, and
    reject lineage semantics; focused tests, the real staging `--plan`, and one
    strict ingest into a new `/tmp` Raw root pass. No captured bytes enter Git.
    An actual multi-version ETF viewer observation remains evidence work, not a
    parser inference.
12. ~~**KIND date reach.**~~ **CLOSED 2026-08-20 — answer: yes.** The 상세검색
    and ETF-disclosure range controls accept `2020-01-31`, and a 2020-Q1 query
    returned 2020-03-31 rows, so the reach is real rather than merely accepted
    by the input.
13. **Export byte-stability.** **KIND half CLOSED 2026-08-20, by a different
    route than expected:** the ETF-scoped page has no export at all, so the
    artifact is the search response itself, and that was measured byte-stable —
    two independent captures of `2020-02-03..2020-02-07` produced 32 of 32 pages
    byte-identical, so no normalization step is needed before hashing. The
    `data.krx.co.kr` half remains open: for that site, run an identical query over
    a closed window on two different days, download both exports, and hash them;
    if a generation timestamp is embedded, define the normalization step before
    treating the artifact as hashable immutable Raw.
14. **Automated-collection question.** Decide whether scripted use of a
    site-provided export counts as `무단` automated collection under
    `data.krx.co.kr` 제10조②, and whether that clause governs KIND. Until
    decided, ADR-0004 D6 keeps the `data.krx.co.kr` category operator-driven;
    D11 governs only the approved low-volume KIND browser path.
15. ~~**Licence sign-off.**~~ **CLOSED 2026-08-20.** The owner recorded the use
    as personal and internal, so the licence question does not block progress.
    The `entitlement_reference` in use for OpenDART is
    `opendart:tou-art16-art23:personal-internal:2026-08-20`, owner-endorsed —
    read as source, basis (terms art. 16 copyright and art. 23 no-warranty),
    purpose, and date of record. The underlying constraints stay recorded above,
    unchanged, because they would govern again if the purpose ever changed: KOGL
    Type 2 non-commercial on the KSD portal datasets, and KRX 제11조③
    post-termination use.

    A second reference is now in use, for KIND:
    `kind:krx-legal-notice:personal-internal:2026-08-20`, recorded in the first
    KIND Raw batch's manifest row. It follows the pattern the owner endorsed for
    OpenDART — source, basis, purpose, date — but the string itself was minted
    here rather than supplied, so treat it as **provisional and replaceable at the
    owner's word**. Its basis is KIND's own linked KRX legal notice, which
    prohibits unauthorized reproduction and redistribution and carries no
    automated-collection clause (see ADR-0004 D11 for the accepted risk).
16. **First real `list.json` and `company.json` response — deferred/blocked.**
    The OpenDART core fixture contract is approved, but D4 establishes no ETF11
    use and D10 blocks the in-process live path. If individual-stock scope is
    approved later, capture one real response per surface and confirm whether
    the four envelope integers arrive as JSON numbers or strings (both are
    accepted), and whether `rm` is present on every row or omitted when there is
    no remark. The adapter fails closed with a typed `UndocumentedShape` on a
    mismatch rather than mis-parsing.
17. **KIND per-issue filtering.** Neither typing a six-digit code into
    `searchCorpName` nor setting the hidden `repIsuSrtCd` filtered the result
    set — both returned the unfiltered first page, which is how an earlier
    reading that "ETF11 resolves" turned out to be the unfiltered list's first
    row rather than a match. The site's own issue-code popup evidently sets
    further state that was not reproduced, and probing for it was stopped rather
    than guessed at. **Design note that may make this moot:** the ETF-scoped
    disclosure list is already restricted to ETF-type issues and carries 종목명
    per row, so a date-ranged fetch filtered locally by name would serve the
    pipeline without the popup at all. Confirm the intended approach before
    building either. **Direction chosen 2026-08-20: local filtering** (ADR-0004
    D11). The popup will not be reproduced. A third attempt was made and failed:
    the ETF page's own `etfIsuSrtCd` field exists and can be set, but the page
    still returns the unfiltered list, so it needs companion state the popup
    supplies. Probing stopped at three attempts. **This item is now closed as
    won't-do rather than open** — filtering after Raw is both sufficient and
    better lineage.

## Read-only allowlist — approval state

**Current state — 2026-08-21.** The OpenDART core remains approved for its
fixture-backed contract, but D4 establishes no ETF11 use and D10 blocks the
in-process live path. KIND D11 is approved for the low-volume browser path and
its capture → Raw → normalization implementation is present. The FSC listing
mirror contract and credentialed control are resolved, but D14 rejects it as a
complete ETF11 identity source. KIS is primary for ETF11 price/volume; the
ETF-specific FSC price surface is not selected, and any second price API needs
an explicit independent cross-check/fallback approval. KRX, KSD, `data.krx.co.kr`,
the full KIND backfill, and ETF11 identity remain deferred.
The 2026-08-19 approval snapshot above is retained as historical evidence, not
as the current table state.

`APPROVED` here authorizes only the listed request shape, fixture-backed
contract, and any explicitly stated operator-gated path. It does not add an
endpoint or imply ETF11 applicability. The owner-supplied OpenDART key is not
recorded; exactly one external `corpCode.xml` observation is recorded above,
while D10 blocks the in-process path. KIND approval is through the site's own
browser controls and does not authorize reconstructed requests or unbounded
backfill.

| source | surface | status |
|---|---|---|
| OpenDART | `GET /api/list.json` \| `/api/list.xml` | **APPROVED** — fixture contract; no ETF11 use (D4), in-process live path blocked (D10) |
| OpenDART | `GET /api/corpCode.xml` | **APPROVED** — one external observation settled checklist item 4; no ETF11 use (D4), in-process live path blocked (D10) |
| OpenDART | `GET /api/company.json` \| `.xml` | **APPROVED** — fixture contract; no ETF11 use (D4), in-process live path blocked (D10) |
| OpenDART | `GET /api/crDecsn`, `/api/piicDecsn`, `/api/cmpMgDecsn` | **DEFERRED / NOT ALLOWED** — outside the approved core; D4 establishes no ETF11 use |
| OpenDART | `GET /api/alotMatter` | **DEFERRED / NOT ALLOWED** — outside the approved core; D4 establishes no ETF11 use |
| FSC mirror | `금융위원회_KRX상장종목정보` | **CONSUMED / REJECTED FOR ETF11** — exact diagnostic and non-ETF control are historical; no further live query or Raw collection without new explicit owner approval; no Raw batch committed |
| FSC mirror | `금융위원회_증권상품시세정보` (ETF operation) | **NOT A CANDIDATE / NOT ALLOWED** — only a separately approved independent cross-check/fallback could reopen price-API review |
| KRX Open API | `ETF 일별매매정보` | DEFERRED — endpoint needs checklist item 1; no default comparison path is selected |
| KSD portal | `주식권리일정정보`, `주식배당정보` | DEFERRED — blocked on the KOGL Type 2 entitlement decision |
| KIND | ETF-scoped disclosure list and related correction-evidence viewer flow | **APPROVED 2026-08-20, MANUAL ONE-DAY ONLY** — browser-driven through the site's own controls, low volume, explicit operator confirmation, no scheduled/timer/bulk/full-history path (ADR-0004 D11); ETF disclosure capture → Raw → normalization is the implemented path |
| KIND | 상세검색, 관리종목, 매매거래정지, and every other page/endpoint | **DEFERRED / NOT ALLOWED** — outside the exact D11 exception |
| data.krx.co.kr | 이슈 통계 leaf pages for 신규상장 / 상장폐지 / 매매거래정지 / 관리종목 | DEFERRED — operator-driven export only, per D6; leaf URLs unconfirmed |
| terms pages | KRX Open API 이용약관, data.krx.co.kr 이용약관, KOGL licence, OpenDART 약관 | re-check periodically; terms change |

No candidate is proposed for merger, corporate split, stock split, or reverse
split from a public KSD surface — none was found.

Nothing in this file authorizes an account, order, balance, or execution path.
That scope remains forbidden.
