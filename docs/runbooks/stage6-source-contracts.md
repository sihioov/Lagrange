# Stage6 official source contracts — evidence and operator checklist

Reference companion to `docs/decisions/0004-stage6-official-source-contracts.md`.
That ADR records the decisions; this file records the evidence they rest on, the
gaps they must not paper over, and the checklist an operator has to clear before
Step 2 can write an adapter.

Approval state, as of 2026-08-19: the **OpenDART core** surface
(`list.json`/`list.xml`, `corpCode.xml`, `company.json`) is approved for
fixture-based Raw adapter work. **Every other surface here is deferred** and
still awaits the approval `AGENTS.md` requires for any change to a method, path,
host, or response contract. See the allowlist table at the end for the per-row
state.

As of 2026-08-20 the owner has supplied an OpenDART key. **Exactly one live
request has been made against any surface in this document**: a single
`GET /api/corpCode.xml`, whose purpose and result are recorded in checklist
item 4 below. No other surface has been contacted, and no key exists for any of
them.

## How this evidence was gathered

Research ran on 2026-08-19 as four source-scoped passes plus one adversarial
verification pass, all read-only:

- public documentation pages only, fetched over HTTP;
- no account registration, no API-key request, no authenticated call, no
  data-endpoint call, no form submission, no login;
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

`[verified]` A deliberately invalid key against `/api/corpCode.xml` returned
**HTTP 200** with an XML envelope, not JSON:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?><result><status>010</status><message>...</message></result>
```

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

Unresolved. `corp_cls` has only `Y`(유가) / `K`(코스닥) / `N`(코넥스) /
`E`(기타) — no fund bucket. `pblntf_ty` does include `G: 펀드공시`, but its only
documented sub-codes are `G001`–`G003`, all collective-investment-securities
registration statements. The structured securities-registration group (DS006)
excludes collective investment securities. The corp-code field is described as
`공시대상회사의 고유번호(8자리)` without defining whether an investment trust
qualifies.

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

No documented programmatic API exists on `kind.krx.co.kr`. Every page inspected
— main, 공시통합검색, 상세검색, 관리종목, ETF disclosure, 상장법인목록 — exposed
a human-facing HTML search form with an `EXCEL` export button. No developer
portal, no key issuance, no API documentation link. `robots.txt` returns 404.

This matches the §0.4 premise that KIND need not be an API.

### Candidate artifacts

| purpose | page | export | operator controls |
|---|---|---|---|
| detailed disclosure search | `https://kind.krx.co.kr/disclosure/details.do?method=searchDetailsMain` | EXCEL | 기간 date range, market, security type (includes ETF and 주권), disclosure-type checkboxes including `정정공시 요구`, company name/code |
| ETF-scoped disclosure list | `https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf` | EXCEL | stock name, report name, 기간 |
| administrative issue (관리종목) | `https://kind.krx.co.kr/investwarn/adminissue.do?method=searchAdminIssueList` | EXCEL | market (전체/유가증권/코스닥), stock name |
| trading halt (매매거래정지) | `https://kind.krx.co.kr/investwarn/tradinghaltissue.do?method=searchTradingHaltIssueMain` | EXCEL | market, stock name |
| listed-company roster | `https://kind.krx.co.kr/corpgeneral/corpList.do?method=loadInitPage` | EXCEL | corporate/market classifications |

No dedicated standalone export page was found for new/changed listings or for
issue name changes; those appear to be reachable only through the
disclosure-type filters on 상세검색.

### ETF coverage

Confirmed at the type level: an ETF-scoped disclosure page exists, and the
상세검색 security-type filter lists ETF alongside 주권. **None of the 11 short
codes was individually queried** — that needs the AJAX form, which read-only
static fetching cannot drive.

### Timestamp granularity

Two disclosure-viewer documents were sampled. Both showed calendar dates only
(`신규상장 (2025.09.26)`; version dates `2025.11.05`, `2026.01.22`) with no
hour, minute, or timezone. This is a **two-document sample**, not a
system-wide conclusion: a receipt time may exist in a results-table column that
static fetching cannot render. Recorded as a gap.

### Correction linkage

Partial evidence. The integrated-search results list uses a `정정있음` marker,
and 상세검색 has a `정정공시 요구` category filter, so correction status is
tracked at list level. On the sampled documents, only a textual notice appeared
—
`본 공시는 공시내용 기재 불충분 등의 사유로 한국거래소 정정요구를 받은 사항입니다`
— with no machine-followable reference to the pre-correction filing. Whether
such a field exists is a gap.

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

| dimension | keep KIS `ksdinfo` | add direct KSD/SEIBro |
|---|---|---|
| independence | endpoint naming indicates KSD-origin data relayed by the broker — **inferred, not confirmed** | same origin, so neither corroborates the other independently |
| availability timestamp | unconfirmed; not checked in the KSD-scoped pass | confirmed absent in visible fields (above) |
| field completeness | one category normalized, five fail closed | dividend plus combined rights schedule only; merger/split/reverse-split absent |
| rights | KIS own-asset use, no redistribution — already recorded | KOGL Type 2, non-commercial, separate permission for commercial use |
| implementation cost | already implemented, rate-limited, allowlisted | new registration, allowlist, parsing, and fail-closed logic |

Unresolved for the operator: whether `ksdinfo` is literally a KSD relay, and
whether `ksdinfo` responses carry any announcement timestamp. Neither was
checked — the KSD-scoped pass fetched no KIS page. This is checklist item 9.

## Operator verification checklist

Step 2 depends on these. Each needs a human with a browser, or an authorized
credentialed call, because read-only public fetching cannot settle it.

1. **KRX Open API specifications.** Register on Data Marketplace, obtain key
   approval, and capture from the logged-in `개발 명세서`: exact host and path,
   HTTP method, full parameter set, response schema, pagination mechanism and
   terminal condition, and any correction or revision field — for
   `ETF 일별매매정보` at minimum. Everything technical is behind this wall.
2. **FSC mirror specifications.** Download the `활용자가이드` `.docx` for
   `금융위원회_KRX상장종목정보` and `금융위원회_증권상품시세정보` and capture the
   same facts, plus whether the response envelope uses
   `numOfRows` / `pageNo` / `totalCount` — a common portal pattern that must not
   be assumed here.
3. **Snapshot versus live semantics.** Query `KRX상장종목정보` for the same ISIN
   at two past `기준일자` values and check whether market classification or name
   can differ across dates. That determines whether it is a true point-in-time
   query or a date filter over a current table — which decides whether it can
   anchor instrument identity validity intervals.
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
9. **KIS `ksdinfo` lineage and timestamps.** From current official KIS
   documentation, establish whether the six `ksdinfo` endpoints relay KSD data
   and whether any response field carries an announcement time. This decides
   whether ADR-0004 D5 can be satisfied without a new source.
10. **KIND receipt time.** Run a query on 상세검색 in a real browser and inspect
    the results-table column headers for a time field. This is the open question
    that could tighten ADR-0004 D1.
11. **KIND correction linkage.** Open a known `[정정]` filing and record whether
    a `관련공시` or original-`acptNo` reference exists in any followable form.
12. **KIND date reach.** Confirm the date pickers on 상세검색, the ETF
    disclosure page, and 관리종목 accept `2020-01-31` as a start date.
13. **Export byte-stability.** For KIND and for `data.krx.co.kr`, run an
    identical query over a closed historical window on two different days,
    download both exports, and hash them. If a generation timestamp is embedded,
    define the normalization step before the artifact is treated as hashable
    immutable Raw.
14. **Automated-collection question.** Decide whether scripted use of a
    site-provided export counts as `무단` automated collection under
    `data.krx.co.kr` 제10조②, and whether that clause governs KIND. Until
    decided, ADR-0004 D6 keeps this category operator-driven.
15. **Licence sign-off.** Record an `entitlement_reference` covering KOGL Type 2
    non-commercial use and KRX 제11조③ post-termination use, interpreted against
    this project's stated personal-internal purpose.
16. **First real `list.json` and `company.json` response.** With a key issued,
    capture one real response per surface and confirm the adapter's shape
    assumptions against it: whether the four envelope integers arrive as JSON
    numbers or strings (both are accepted), and whether `rm` is present on every
    row or omitted when there is no remark. The adapter fails closed with a
    typed `UndocumentedShape` on a mismatch rather than mis-parsing, so this is a
    first-use verification, not a correctness risk.

## Read-only allowlist — approval state

Documentation-confirmed paths. The owner approved the **OpenDART core** on
2026-08-19 for fixture-based Raw adapter work; everything else is **DEFERRED**
and may not be called, registered for, or coded against.

`APPROVED` here authorizes the request shape, the fixture-backed contract, and
an operator-gated live path. It does not authorize a live call: no key has been
issued, so no request has been sent to any of these surfaces.

| source | surface | status |
|---|---|---|
| OpenDART | `GET /api/list.json` \| `/api/list.xml` | **APPROVED** — path, params, schema documented |
| OpenDART | `GET /api/corpCode.xml` | **APPROVED** — needed for checklist item 4 |
| OpenDART | `GET /api/company.json` \| `.xml` | **APPROVED** — documented |
| OpenDART | `GET /api/crDecsn`, `/api/piicDecsn`, `/api/cmpMgDecsn` | DEFERRED — applicability to ETF11 unresolved |
| OpenDART | `GET /api/alotMatter` | DEFERRED — periodic realized amounts, limited use |
| FSC mirror | `금융위원회_KRX상장종목정보` | DEFERRED — mirror-vs-origin decision open; endpoint needs checklist item 2 |
| FSC mirror | `금융위원회_증권상품시세정보` (ETF operation) | DEFERRED — mirror-vs-origin decision open; endpoint needs checklist item 2 |
| KRX Open API | `ETF 일별매매정보` | DEFERRED — endpoint needs checklist item 1; prefer the FSC mirror per D3 |
| KSD portal | `주식권리일정정보`, `주식배당정보` | DEFERRED — blocked on the KOGL Type 2 entitlement decision |
| KIND | 상세검색, ETF disclosure, 관리종목, 매매거래정지 pages | DEFERRED — operator-driven EXCEL export only, per D6 |
| data.krx.co.kr | 이슈 통계 leaf pages for 신규상장 / 상장폐지 / 매매거래정지 / 관리종목 | DEFERRED — operator-driven export only, per D6; leaf URLs unconfirmed |
| terms pages | KRX Open API 이용약관, data.krx.co.kr 이용약관, KOGL licence, OpenDART 약관 | re-check periodically; terms change |

No candidate is proposed for merger, corporate split, stock split, or reverse
split from a public KSD surface — none was found.

Nothing in this file authorizes an account, order, balance, or execution path.
That scope remains forbidden.
