# ADR-0007: Official sources for individual-stock production candidates

- Status: Proposed; **Gate A blocked**
- Date: 2026-08-29
- Scope: KOSPI 200 current-daily activation; KOSDAQ 150 contract-compatible, inactive
- Related: ADR-0003, ADR-0004, ADR-0005, `docs/superpowers/plans/2026-08-29-kospi200-production-candidate-activation.md`

## Context

The database, computation, API, Web, and synthetic-QA vertical for KOSPI 200 and
KOSDAQ 150 stock candidates exists, but no production candidate feed is active.
ADR-0003 requires candidate evidence to stay separate from ETF recommendation
evidence and requires point-in-time (PIT) membership rather than a current member
snapshot. This ADR identifies the narrowest official-source contract that could
activate the KOSPI 200 path without implying approval.

Repository code is not an authorization record. In particular, the KIS investor
flow and finance adapters exist, but their path/TR-ID pairs are outside the root
allowlist. No live request was made while preparing this decision.

## Decision summary

The preferred production contract is a conditional hybrid:

1. KIS read-only REST supplies current daily bars and, after a new exact allowlist
   approval, per-issue investor flow.
2. A signed KRX or authorized Koscom delivery supplies the security master,
   KOSPI 200 membership/change notices, sector taxonomy, and market-status history.
   The order form and data dictionary must prove effective, publication, revision,
   retention, and display semantics. The public KRX website is not a substitute.
3. OpenDART supplies issuer identity and canonical filing fundamentals only if the
   owner separately approves the exact financial endpoint, a scoped compatible TLS
   transport, and the unresolved retention/display position.
4. The six already allowlisted KIS KSD schedule endpoints remain corporate-action
   evidence. Only the currently supported bonus event may normalize automatically;
   any other nonempty action continues to stop the pipeline.
5. The first activation is private, single-owner use. No Member-visible candidate,
   score, Raw, or derived KIS/KRX/OpenDART value is authorized by this ADR.

This is a recommendation, not Gate A approval. The missing signed reference/index
delivery contract is a hard blocker. OpenDART transport and rights are additional
blockers if OpenDART is selected for fundamentals.

## Point-in-time vocabulary

Every admitted record must preserve these distinct facts:

- `event_at`: when the market, filing, or corporate event occurred; for a bar or
  flow row this is the trade date, and for a statement it includes the fiscal
  period end.
- `effective_at`: when membership, classification, status, or another eligibility
  fact begins to govern. It must not be inferred from retrieval.
- `available_at`: the earliest defensible instant the source made the fact
  knowable. If OpenDART supplies only `rcept_dt`, the conservative project rule is
  the following calendar day at `00:00:00 Asia/Seoul`; this is a project cutoff,
  not a provider publication-time claim.
- `retrieved_at`: the collector observation time.
- `source_revision`: the provider revision identifier. When none exists, the
  immutable Raw content hash may identify a retrieved snapshot, but it must be
  labelled as such and must not be represented as a provider revision.

Current daily evidence may use `retrieved_at` as `available_at` only where the
provider has no publication field and only for observations first acquired in
that run. That rule cannot reconstruct historical availability.

## Dataset source matrix

`Pending` means the identifier was verified but the repository does not currently
authorize its production use. `Blocked` means required semantics, rights, or
transport are not established.

| Dataset | Preferred official option and canonical mapping | PIT and revision contract | Rights, ownership, approval |
| --- | --- | --- | --- |
| EOD price | KIS `GET https://openapi.koreainvestment.com:9443/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice`, TR `FHKST03010100`. Map `stck_bsop_date`, OHLC fields, and `acml_vol`; persist `FID_ORG_ADJ_PRC` so adjusted and raw bars cannot be mixed. Single page, blank continuation; official sample documents up to 100 rows. | Trade date is event/effective date. KIS exposes no publication timestamp or revision in this contract; use retrieval as availability and Raw hash as retrieved-snapshot identity. Adjusted history is a current vendor view, not strict historical PIT. | Exact pair is allowlisted, but the existing approval is ETF11-oriented. KOSPI 200 candidate scope and request budget still require Gate A. KIS credentials remain owner-operated in the protected existing path. ADR-0005 permits private personal use only. |
| Investor flow | KIS `GET /uapi/domestic-stock/v1/quotations/investor-trade-by-stock-daily`, TR `FHPTJ04160001`. Map `stck_bsop_date`; foreign `frgn_ntby_tr_pbmn`/`frgn_ntby_qty`; institution `orgn_ntby_tr_pbmn`/`orgn_ntby_qty`; canonical net amount in KRW is the provider amount multiplied by 1,000,000, subject to a focused official-dictionary fixture test. The current official sample recurses for response `tr_cont=M` **and** `F`, sends request `tr_cont=N` in both cases, and caps at ten pages. That contradicts the proposed narrower `M`-only continuation policy, so pagination remains unresolved until Gate A records an approved interpretation and focused tests. | Trade date is event date. No provider publication time or revision was found. For current ingestion only, availability is retrieval and revision identity is the Raw hash. This cannot prove historical correction visibility. | Preferred but **not allowlisted**. Requires exact method/path/TR approval and focused pagination, unit, duplicate-page, and budget tests. Same private KIS rights and credential ownership as price. |
| Fundamentals | Conditional OpenDART `GET https://opendart.fss.or.kr/api/fnlttSinglAcntAll.json`; params `corp_code`, `bsns_year`, `reprt_code`, `fs_div` plus redacted `crtfc_key`. Map `rcept_no`, fiscal year/report kind, `fs_div`, statement/account identifiers and names, current/prior amounts, `currency`; select only metrics with an explicit unit policy. It is a single-page documented response. The documented fields do not by themselves prove exact issuer fiscal-period start/end; a row remains blocked unless approved official evidence supplies both. | Join the exact `rcept_no` to allowlisted `list.json` for `rcept_dt`; never derive a date from the receipt number. Use the conservative day-only availability rule above. A correction is a new opaque receipt; because OpenDART exposes no structured original-correction link, do not populate `restates_source_revision` without separate evidence. | **Blocked**: endpoint is forbidden today; host requires TLS 1.2 static-RSA unavailable in the current rustls transport; period bounds and terms do not resolve the canonical/retention/display requirements. Requires owner decisions on endpoint, scoped TLS backend, supplementary period evidence, and rights. Key remains owner-operated and must be redacted before metadata construction. |
| Index membership | Signed KRX or authorized Koscom index constituent delivery covering both KOSPI 200 and KOSDAQ 150. Required mapping: index ID, ISIN/short code, `announced_at`, `effective_from`, `effective_until`, `available_at`, provider revision and correction/supersession ID. Required delivery includes daily as-of snapshots and change notices. | Contract/data dictionary must distinguish announcement, effective date, first publication, corrections, and restatements. Current snapshots cannot populate history. | **Blocked pending signed order form, delivery specification, entitlement hash, retention term, Raw/derived-display permissions, audience, credentials, transport, and service budget.** Public Data Marketplace UI/export is not authorized automation. |
| Sector | Same licensed KRX/Koscom reference delivery. Map taxonomy ID/version, ISIN/short code, industry code/name, profile, effective interval, availability, and revision. | Taxonomy version and effective-dated changes are mandatory; retrieval-only snapshots are insufficient for history. | Same signed-contract blocker as membership. KIS master snapshot is not accepted as PIT evidence. |
| Market status | Same licensed KRX/Koscom reference/status delivery. It must support suspended, administrative, liquidation, inactive/delisted, disqualifying audit opinion, and complete capital impairment, with reason/effective interval. | Effective start/end, publication availability, corrections, and revision are mandatory for each flag. Missing mandatory state fails closed. | Same signed-contract blocker. Public KRX pages establish that some halt, administrative, and delisting histories exist, but not a complete six-flag contract or automation/display right. |
| Corporate actions | Existing KIS read-only KSD allowlist: `paidin-capin`/`HHKDB669100C0`, `bonus-issue`/`HHKDB669101C0`, `dividend`/`HHKDB669102C0`, `merger-split`/`HHKDB669104C0`, `rev-split`/`HHKDB669105C0`, `cap-dcrs`/`HHKDB669106C0`, all `GET` under `/uapi/domestic-stock/v1/ksdinfo/`. | Schedule dates are event/effective evidence; no publication timestamp or revision is documented, so retrieval is the only safe availability and Raw hash the snapshot identity. KSD continuation is exact `M -> N`, blank unchanged `CTS`, ten-page bound, repeated bytes fail closed. Only bonus normalization is supported. | Exact pairs are allowlisted. Candidate-universe budget/use and private audience still need Gate A. A licensed KRX/KSD action feed with announcement/revision history is the historical alternative. |
| Issuer identity | Current identity join: allowlisted OpenDART `GET /api/corpCode.xml`, joining `stock_code` to `corp_code`; use licensed KRX/Koscom security master for ISIN/short code/instrument lifecycle. `company.json` is optional issuer profile, not instrument identity. | `modify_date` and retrieval describe the current archive; they do not prove a historical effective interval or prior availability. Historical issuer/security identity therefore requires the licensed master history. | OpenDART paths are allowlisted, but in-process transport is blocked by TLS. The archive key must never enter recorded query metadata. KRX master rights remain contract-pending. |

## Rejected or limited alternatives

| Option | Disposition | Reason |
| --- | --- | --- |
| KIS `kospi_code.mst.zip`, `kosdaq_code.mst.zip`, `idxcode.mst.zip` | Reject for PIT membership/sector/status; current reconciliation only | Officially updated current masters do not embed snapshot, announcement, effective, availability, or revision facts. Existing `require_candidate_master_pit` correctly fails closed. |
| KIS balance sheet `FHKST66430100` and income statement `FHKST66430200` | Reject as canonical fundamentals | Path/TR pairs are not allowlisted; response semantics do not establish statement scope, units, disclosure time, correction lineage, or revisions. Existing normalization deliberately rejects them. |
| KRX Data Marketplace public UI/exports | Reject for scheduled production | Terms effective 2026-08-29 prohibit unauthorized automated collection and redistribution; public pages do not establish delivery schema, PIT publication/revision semantics, retention, or display rights. |
| KRX Open API | Reject for this vertical | Official catalog lacks the complete membership/status/sector contract; terms are noncommercial, prohibit third-party provision, cap use at 10,000 calls/day, and do not fit an immutable Raw archive after contract termination. |
| OpenDART alone | Reject as complete candidate source | It is filing evidence, not index membership, exchange status, sector taxonomy, or full corporate-action history. Current allowed endpoints also exclude financials. |
| Current snapshot replay | Prohibited | It leaks future composition/classification/status into earlier dates and cannot support a PIT backtest claim. |

## Rights and audience boundary

| Source | Raw retention | Derived display | Permitted audience now |
| --- | --- | --- | --- |
| KIS | Existing private, rights-scoped immutable evidence only; new datasets await Gate A | Only owner-visible derived candidate output after scope approval; never Raw | Single owner only under ADR-0005 |
| OpenDART | Not established for indefinite production retention by the public terms reviewed | Not established; owner attestation or written permission required | None for the proposed financial path today |
| Licensed KRX/Koscom | Must be stated in the signed order/entitlement annex | Must explicitly cover the intended derived candidate fields and provenance | None until contract states owner-only or broader audience |

The application must never expose Raw provider payloads. Extending output to invited
Members is a separate rights and product decision, even if the technical endpoint is
already authenticated.

## Current daily activation versus historical claims

After Gate A, a current KOSPI 200 run may compute at cutoff `T` only from evidence
whose `available_at <= T`, using effective-dated licensed membership, sector, and
status; trade-date KIS rows retrieved and sealed before the run; and filings admitted
under the conservative OpenDART rule. The initial 60 trading sessions are feature
context for the current run, not 60 reconstructed historical candidate runs.

A historical candidate/backtest claim remains blocked until the project receives
historical membership/change notices, sector taxonomy changes, status intervals,
filing corrections, issuer/security-master history, and corporate-action/revision
evidence with defensible availability. No current snapshot may substitute for any
of those facts. KOSDAQ 150 activation remains a later Gate C with a separate budget,
but the licensed schemas must cover it now to avoid a second contract design.

## Proposed request and delivery budget

For one KOSPI 200 daily run:

- KIS price: 200 single-page GETs; hard ceiling 220 including retries.
- KIS investor flow: 200 first pages; a global hard ceiling of 400 pages, including
  continuations and retries, and ten pages per symbol. Exceeding either fails closed.
- KIS holiday: one GET, once per run.
- KIS KSD actions: six first pages; global hard ceiling 60 pages including retries.
- OpenDART, if approved: up to 220 list/detail requests plus margin, with a hard
  ceiling of 250 requests/day including retries; one `corpCode.xml` refresh per
  week, counted separately and never eager-refreshed.
- Licensed delivery: at most one immutable object/version/day for membership,
  sector, market status, and security master. Protocol/rate limits must come from
  the signed delivery specification.

The resulting KIS hard ceiling is 681 requests/pages per daily run: 220 price,
400 flow, one holiday, and 60 KSD. No unused channel budget transfers to another
channel.

KIS execution remains sequential by default and no faster than one request per
second per endpoint/TR channel, notwithstanding the provider's higher documented
ceiling. The existing access-token manager must reuse its cached token; retries are
bounded, classified, honor `Retry-After`, and count against the hard budget. No
account or order identifier is permitted. KOSDAQ 150 would add 150 price, 150 flow,
and up to 150 filing lookups and is not authorized by this proposal.

## Remaining owner decisions

1. Obtain and approve a signed KRX/Koscom product annex that names KOSPI 200 and
   KOSDAQ 150 membership, security master, sector, and all six market-status facts;
   defines historical depth, delivery, corrections, publication/effective time,
   retention, Raw storage, derived display, audience, and redistribution.
2. Decide whether first activation is strictly owner-only. Broader audience requires
   new written rights from every selected provider.
3. Approve or reject the exact KIS investor-flow method/path/TR pair, resolve the
   official sample's contradictory `M`/`F` continuation behavior, and decide the
   KOSPI 200 extension of the already allowlisted price, calendar, and KSD pairs.
4. Choose fundamentals: (a) OpenDART with the exact financial endpoint, scoped
   compatible TLS transport, day-only cutoff, and rights attestation; or (b) a
   licensed vendor contract with equivalent PIT and revision fields. KIS finance is
   not an acceptable fallback.
5. Accept or change the budgets above and the rule that retrieval-only source facts
   support current incremental operation, not historical reconstruction.
6. Decide whether current vendor-adjusted KIS bars are acceptable for the current
   feature window. Strict PIT backtests require separately contracted historical
   unadjusted bars, actions, and revisions.

## Gate A approval text

There is no safe, fully concrete approval text until the signed delivery documents
and entitlement identifiers exist. The following is the exact form required; every
angle-bracket value must be replaced with an immutable value before approval.
Approval containing a placeholder is invalid.

> I approve Gate A for private, single-owner KOSPI 200 candidate production under
> ADR-0007 and the runbook `candidate-production-sources.md`, using KIS production
> read-only REST at `openapi.koreainvestment.com:9443` only for
> `GET /uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice`
> (`FHKST03010100`),
> `GET /uapi/domestic-stock/v1/quotations/investor-trade-by-stock-daily`
> (`FHPTJ04160001`), `GET /uapi/domestic-stock/v1/quotations/chk-holiday`
> (`CTCA0903R`),
> `GET /uapi/domestic-stock/v1/ksdinfo/paidin-capin` (`HHKDB669100C0`),
> `GET /uapi/domestic-stock/v1/ksdinfo/bonus-issue` (`HHKDB669101C0`),
> `GET /uapi/domestic-stock/v1/ksdinfo/dividend` (`HHKDB669102C0`),
> `GET /uapi/domestic-stock/v1/ksdinfo/merger-split` (`HHKDB669104C0`),
> `GET /uapi/domestic-stock/v1/ksdinfo/rev-split` (`HHKDB669105C0`), and
> `GET /uapi/domestic-stock/v1/ksdinfo/cap-dcrs` (`HHKDB669106C0`); the licensed
> KRX/Koscom delivery described by contract
> `<contract-id>`, order annex hash `<sha256>`, entitlement reference
> `<entitlement-ref>`, data-dictionary hash `<sha256>`, delivery host/protocol
> `<exact-host-and-protocol>`, and credential owner `<owner-role>` for security
> master, KOSPI 200/KOSDAQ 150 membership, sector, and market status; and
> fundamentals option `<OpenDART-or-licensed-vendor>` under evidence document hash
> `<sha256>` and exact endpoint/delivery `<identifier>`. I affirm that these
> documents permit the specified immutable Raw retention, owner-only derived
> candidate display, and historical depth `<dates>`. I accept the PIT semantics,
> fail-closed behavior, current-versus-historical limitation, request budgets, and
> transport/credential ownership in ADR-0007. This approval does not authorize
> KOSDAQ 150 activation, Member-visible output, redistribution, public Raw access,
> account data, orders, WebSockets, any unlisted provider endpoint, or any live call
> before the focused contract and safety tests pass.

If OpenDART is chosen, the final text must additionally replace the fundamentals
placeholders with `/api/fnlttSinglAcntAll.json`, the separately approved TLS
implementation and scope, the owner rights attestation hash, and the existing
allowlisted `/api/list.json` and `/api/corpCode.xml` joins. If a licensed vendor is
chosen, its exact host/delivery identifier and field dictionary replace them.

## Consequences

- Gate A remains blocked; this ADR authorizes no network or production action.
- The design favors defensible PIT evidence over immediate activation.
- Current daily operation can be narrower than historical research, and its UI and
  provenance must say so.
- KOSDAQ 150 is schema- and contract-compatible but remains inactive until Gate C.
- Any missing mandatory timestamp, revision, right, status flag, unit, or delivery
  contract fails closed instead of being inferred.
