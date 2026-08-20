# ADR-0004: Stage6 official source contracts and day-granularity availability

- **Status:** Partially approved and partly resolved by measurement. The
  OpenDART core surface (`list.json`/`list.xml`, `corpCode.xml`,
  `company.json`) was approved by the owner on 2026-08-19 for fixture-based Raw
  adapter work, and the owner supplied a key on 2026-08-20 authorising a live
  path. **D4 is now settled by one live `corpCode.xml` request: OpenDART does
  not cover the ETF11 universe.** D10 records a hard TLS incompatibility that
  blocks the in-process live path. Every other allowlist row and all remaining
  registrations stay pending; the owner has recorded the use as personal and
  internal, so the licence question is not a blocker (see D8).
- **Date:** 2026-08-19, amended 2026-08-20
- **Deciders:** Product owner (pending), implementation coordinator, Claude Opus 5 research coordination

## Context

`docs/STATUS.md` §0.2 records Stage5 as complete: an immutable KIS Raw batch
covering the fixed 11-ETF universe from `2020-01-31`, normalized to 1,608 XKRX
sessions, carrying the standing contract `vendor_snapshot=true`,
`strict_pit=false`, `ready=false`. KIS returns prices as they stand at
acquisition time; it does not prove the historical instrument state, the public
availability time of a corporate action, or a correction lineage. Stage6 exists
to obtain that missing evidence from official sources before any Curated
publication, five-pin, recommendation, backtest, or Paper connection.

Stage6 Step 1 is "fix the official source contracts from official
documentation". This ADR records what four source-scoped research passes and one
adversarial verification pass could and could not confirm from official
documentation on 2026-08-19, and the decisions that follow.

**Historical research snapshot — 2026-08-19.** To keep the original evidence
trail legible, that pass consulted public documentation pages only. No account
was registered, no API key was requested, no authenticated call and no
data-endpoint call was made, and no form was submitted. Every claim carries a
fetched URL; anything not confirmed from a fetched page is recorded as a typed
gap rather than inferred. The later owner-approved observations are recorded
in D4, D10, and D11. The per-source endpoint tables, verbatim quotes,
verification verdicts, gaps, and the operator checklist live in
`docs/runbooks/stage6-source-contracts.md`.

## Revised premises

Three premises in the `docs/STATUS.md` §0.4 decision are contradicted by the
official documentation. They are corrected here rather than silently redefined.

1. **KRX Open API cannot be the API authority for listing effective dates or
   market actions.** The service catalog is a single flat table of 31 entries
   across 7 categories. It contains no listing, delisting, change-of-listing,
   administrative-issue, or trading-halt endpoint, and no ETF-specific
   `종목기본정보` endpoint — issue basic information exists only for KOSPI,
   KOSDAQ, and KONEX equities. `ETF 일별매매정보` does exist, documented from
   `2010-01-04`.

2. **OpenDART does not supply a first-disclosure *time*.** The disclosure-list
   API documents 15 response fields; `rcept_dt` is `공시 접수일자(YYYYMMDD)` and
   no time-bearing field exists anywhere in the schema. §0.4's assignment of
   "최초 공시시각" to OpenDART is not supported at sub-day granularity.

3. **A direct KSD/SEIBro source is not an independent cross-check against KIS
   `ksdinfo`.** Both trace to KSD as origin, so comparing them validates relay
   fidelity, not independent observation. The same caveat applies to price:
   KIS prices originate from KRX, so a KIS↔KRX price comparison validates
   relay and processing fidelity, not an independent measurement.

## Decisions

### D1: `available_at` is source-granular in Stage6; the day-granular baseline was later tightened for KIND

**Historical public-document conclusion — 2026-08-19 (superseded for KIND by
D11 on 2026-08-20).** No official source examined at that time documented a
sub-day publication timestamp for the data this project needs. OpenDART was
`YYYYMMDD` with no time field. KSD/SEIBro public datasets exposed schedule
dates only. The KIND disclosure documents sampled in that documentation-only
pass showed date-only values. This conclusion is retained as evidence of what
the public documentation supported then; it is not the current KIND observation.

**Amended 2026-08-20 — KIND supplies a minute-granular posting time, and D1
tightens accordingly.** The gap below was closed by driving KIND in a real
browser engine, which its search requires. The disclosure list carries a `시간`
column holding `YYYY-MM-DD HH:MM` per disclosure. Three checks make it usable
rather than incidental: the values **vary within one result page** (`16:11`
alongside `16:09`), so it is a per-record value and not a constant or a render
artifact; they are **present historically** — 15 of 15 rows in a 2020-03-31
window carried a time, and our range starts 2020-01-31; and the range control
accepts `2020-01-31`.

So for KIND-sourced disclosure-list records `available_at` may be
minute-granular, which is exactly the tightening D1 anticipated and permits.
Because the page does not document a timezone, normalization records the
explicit `AssumedAsiaSeoul` assumption alongside the displayed local value; it
does not claim that timezone as provider metadata. Correction *version*
entries remain enumerable only at day granularity, so no minute timestamp is
fabricated for a version (see D11 and the runbook's item 11). What does **not**
change: the prohibition on backdating from a record, payment, listing, or
ex-rights date; and the treatment of feed data, where a documented refresh
cadence still yields `documented_cadence` rather than `strict_pit` (D2).

The original day-granularity reasoning is kept below as the **historical
2026-08-19 public-document record**, because it explains why the browser check
was the deciding step. D11 supersedes its KIND-specific open question.

**Historical 2026-08-19 baseline (superseded for KIND by D11):** Stage6
therefore designed for day granularity. Two things this decision did **not** do:

- It does not claim sub-day precision is unavailable in the disclosure system as
  a whole. At that date, two questions remained open — whether KIND's
  results-table carried a receipt time column, and whether the DART web viewer
  showed a submission time the Open API did not expose. D11 later answered the
  KIND question for disclosure-list rows; the DART viewer question remains a
  gap, not a confirmed absence.
- It does not permit loosening later. A confirmed time source may only tighten
  `available_at`. Nothing may widen it.

The existing prohibition stands unchanged: a record date, payment date, listing
date, or ex-rights date is never written to `available_at`.

### D2: A documented refresh cadence is not per-record knowledge time

The `금융위원회_KRX상장종목정보` dataset page documents that data for a
reference date is published after 13:00 on the following business day. This is
the only quotable availability anchor found across all four sources, and it is a
**service-wide refresh cadence**, not a field on individual records.

It may therefore support a conservative floor — for feeds where it is
documented, `available_at` is no earlier than 13:00 KST on the business day
after the reference date — and it may not be recorded as per-record
point-in-time evidence. Data admitted on this basis carries
`documented_cadence`, not `strict_pit`. Overclaiming here would be inherited by
the five-pin gate, which is exactly the failure Stage6 exists to prevent.

The cadence sentence was verified on the `KRX상장종목정보` mirror only. It could
not be located on the `증권상품시세정보` mirror and must not be assumed to apply
there.

### D3: Prefer the public-data-portal mirror over the KRX Open API where it covers the category

The KRX Open API Terms of Use (effective 2025-12-26) restrict use to
non-commercial purposes (제6조②), prohibit providing received information to
third parties (제11조②), cap requests at 10,000 per key per day (제8조④), and
require attribution of `한국거래소 통계정보` on screens built from results
(제10조③).

Decisive for this architecture, 제11조③ prohibits using information already
received once the usage contract ends. Key validity is one year, and an unused
key may be deleted after twelve months without notice. That clause is in direct
tension with retaining exact provider bytes indefinitely as immutable Raw
evidence — the property the entire lineage design rests on.

The Financial Services Commission mirrors of KRX-sourced data on the public data
portal state `이용허락범위: 제한 없음` and `비용부과유무: 무료`. For the two
categories they cover — listed-issue information and securities-product
(ETF/ETN/ELW) prices — the mirror is the preferred surface, because indefinite
Raw retention does not collide with a post-termination use prohibition.

This is a preference, not a licence conclusion. See D8.

### D4: OpenDART does not cover the ETF11 universe — KIND is the disclosure authority

**Resolved 2026-08-20 by measurement, superseding the pending reading below.**

The owner supplied a key and one live `corpCode.xml` request was made. The
archive holds 118,714 entities, 3,984 of them carrying a non-empty
`stock_code`. **None of the eleven ETF short codes appears as a `stock_code`:
0 of 11.** The result was validated against controls in the same file —
`005930`, `000660`, and `035420` all resolve — so this is a real absence, not a
method failure. 458 entities match `자산운용`, every one of them with no
`stock_code`: the asset manager is the DART filer, and the ETF itself is not a
disclosure entity. No `KODEX` or `상장지수` entity exists at all.

Consequences, which are larger than this one decision:

- OpenDART cannot supply ETF11 instrument identity, and cannot supply an ETF11
  disclosure date. `list.json` and `company.json` are keyed by `corp_code`, and
  the eleven ETFs have none, so the approved core surface has **no ETF11 use**.
- **KIND becomes the only viable disclosure-date authority for ETF11.** That is
  now a dependency, not a preference.
- D5's "KSD event plus a disclosure-backed availability date" therefore rests
  entirely on KIND. D11 confirms minute-granular list times with the explicit
  `AssumedAsiaSeoul` normalization assumption; correction versions remain
  date-only and their acceptance-number linkage is still only partially
  resolved. The remaining correction work is on the critical path.
- `corpCode.xml` keeps standing value as the evidence for this negative, and as
  the identity join for any future individual-stock scope, where issuers *are*
  disclosure entities.

Scope of the claim: this is one snapshot of a master file DART regenerates, so
the finding is "as of the 2026-08-20 master file" and not a statement about
every edition. The archive is retained outside the repository and pinned by
hash in the runbook's checklist item 4, so the measurement is re-checkable
without spending another live request.

The reading below is retained because it records what was knowable from
documentation alone, and why the measurement was the deciding step.

### D4 (superseded): The disclosure sources are not interchangeable, and KIND leads for ETF11

OpenDART's coverage of the 11 ETFs is unresolved: `corp_cls` has only
Y/K/N/E with no fund bucket, `pblntf_ty=G 펀드공시` documents only collective
investment securities registration sub-types, and the structured
securities-registration group excludes collective investment securities.
Whether any of the 11 short codes has its own DART corp code is unknown without
downloading the corp-code file, which needs a key.

KIND does model ETFs as a covered issue type: it has an ETF-scoped disclosure
page, and its detailed-search security-type filter includes ETF.

Therefore KIND is the primary candidate for the "when did this become publicly
knowable" role for the ETF11 universe, and OpenDART is a secondary source whose
applicability must be established by operator verification before it is relied
on. Documents must not write "OpenDART/KIND" as if the two were equivalent.

### D5: KSD supplies the event; a disclosure source supplies its availability

No KSD/SEIBro public field found carries a publication or announcement timestamp
distinct from schedule dates. The documented fields are exercise start and end
dates, share-register closure dates, dividend base date, cash-dividend payment
date, and share-delivery date. The stated daily refresh is a service cadence
(see D2).

Corporate actions are therefore composed, never inferred:

- KSD (directly, or relayed through the approved KIS `ksdinfo` allowlist) is the
  source of the event and its schedule dates and factors.
- A disclosure source supplies the date on which the event became publicly
  knowable, which is what sets `available_at`.

An event without a disclosure-backed availability date stays Raw-only. It is
never promoted to Curated, never pinned, and never reaches recommendation,
backtest, or Paper. This preserves the existing fail-closed rule that an
unsupported non-empty corporate-action type stops the pipeline rather than
inventing a date or a factor.

### D6: Listing and market-action evidence is a file, not an API

Because no official API serves listing, delisting, change-of-listing,
administrative-issue, or trading-halt history (revised premise 1), this category
is obtained as official downloadable artifacts and captured as immutable Raw
files with a recorded request, retrieval time, and content hash — the same
contract the raw zone already enforces for provider bytes.

Two constraints shape how:

- The KRX Data Marketplace Terms of Use list "collecting, copying, or
  distributing information by automated means without authorization" as a
  prohibited act, and require the Exchange's prior permission before copying or
  redistributing site information. KIND's own linked legal notice prohibits
  unauthorized reproduction and redistribution but carries no
  automated-collection clause; whether the Marketplace clause governs KIND is
  unresolved.
- Applying the stricter reading is the fail-closed choice. Stage6 therefore
  treats this category as **operator-driven download**, not scheduled scraping,
  until the user decides otherwise.

Whether an export is byte-stable across repeated identical queries is unknown
and must be measured before any export is treated as hashable Raw; if a
generation timestamp is embedded, a documented normalization step is required
before hashing.

### D7: Cross-validation is described by what it actually proves

Two comparisons in the §0.4 plan are same-origin and will be labelled as
relay-and-processing-fidelity checks, not independent corroboration:
KIS price against KRX price, and KIS `ksdinfo` against a direct KSD feed. Both
remain worth running — a relay defect is a real defect — but neither may be
counted as independent evidence when a five-pin decision is made.

Genuinely cross-source corroboration in Stage6 is the KSD-event against
disclosure-record join described in D5, because KSD and the disclosure systems
are different authorities.

### D8: Rights are recorded, not concluded, and never widened by code

The evidence, to be interpreted by the operator rather than by this document:

- KSD-sourced public-portal datasets carry KOGL Type 2 — attribution required,
  commercial use prohibited. Commercial use is possible only with separate
  permission from the issuing institution.
- KRX Open API: non-commercial only, no third-party provision, no use after the
  contract ends.
- FSC mirrors of KRX data: `이용허락범위: 제한 없음`, free.
- KIS: personal market data is for the customer's own-asset investment use and
  cannot be redistributed (already recorded in `AGENTS.md`).
- `docs/STATUS.md` §0.4 states this project's purpose is personal internal use.

Each admitted source binds to an operator-supplied `entitlement_reference` and
hash, as the raw zone already models. Code does not widen the recorded
entitlement scope, and a source whose licence has not been signed off does not
become admissible by being technically reachable.

### D9: Historical Step1 gate and the post-approval implementation boundary

`AGENTS.md` requires explicit user approval, current official documentation, and
focused tests before a method, path, host, or response contract changes. The
Stage6 plan requires passing per-step review before advancing.

**Historical Step1 gate — 2026-08-19 (superseded by the 2026-08-20 approval and
implementation amendments).** Step 1 therefore ended at documentation. It
created no adapter, called no endpoint, registered no account, requested no
key, changed no allowlist, and touched no database. Every candidate surface in
the runbook was a **proposal**. The proposed allowlist, the four registrations
(public data portal, OpenDART, KRX Data Marketplace, KSD portal), and the
licence interpretation were the operator's decisions.

**Current state — 2026-08-20.** The OpenDART core fixture contract remains
approved, but D4 establishes that it has no ETF11 use and D10 blocks its
in-process live path. D11 is approved for browser-driven KIND collection, and
the capture → immutable Raw → normalization path is implemented. KRX, FSC,
KSD, the full KIND backfill budget, and ETF11 identity decisions remain
deferred; these statuses do not widen any allowlist.

Live and order scope is unchanged and remains forbidden.

### D10: rustls cannot reach OpenDART — the in-process live path is blocked

Verified 2026-08-20 against the live host. `opendart.fss.or.kr` negotiates
**TLS 1.2 only** (a TLS 1.3 attempt is refused with a protocol-version alert)
and selects `AES128-GCM-SHA256` — static RSA key exchange, no forward secrecy.
Restricting the offer to ECDHE suites is refused with a handshake failure.

rustls deliberately implements no non-forward-secret key exchange, so the
`opendart-client` transport — rustls by workspace policy, one TLS stack — cannot
complete a handshake with this host. This is an incompatibility, not a
misconfiguration, and no rustls setting resolves it.

The transport, the gated CLI, and the adapter contract are all correct and
tested; only the final socket is blocked. Options, none taken unilaterally
because each changes a standing rule or an invariant:

1. Introduce a second TLS backend for this one crate. It would work, and it
   contradicts the explicit one-TLS-stack rule recorded in `kis-client`'s
   manifest, widening the supply-chain surface.
2. Fetch with an external tool the operator already trusts and adopt the bytes
   into Raw. That weakens "Raw comes from a recorded in-process request", which
   is the property the lineage design rests on.
3. Leave the live path blocked and treat OpenDART as documentation-only for
   ETF11 — which D4 makes nearly free, since the surface has no ETF11 use.

Given D4, option 3 costs almost nothing today. The choice is the owner's.

**Resolved 2026-08-20: option 3.** The owner chose to leave the live path
blocked and treat OpenDART as documentation-only for ETF11. No second TLS
backend is introduced, so the one-TLS-stack rule stands unchanged. The
`opendart-client` transport, the gated CLI, and the adapter contract remain in
the tree, fixture-tested and unreachable in production — they cost nothing to
keep and are ready if individual-stock scope later makes this surface useful,
where issuers *are* disclosure entities. Should that happen, this decision is
reopened rather than worked around.

### D11: KIND is reached by its own controls, and filtered locally

D4 makes KIND the ETF11 disclosure authority, and the D1 amendment shows its
list carries what `available_at` needs. Two access facts shape how it is used.

KIND cannot be driven over plain HTTP. The search target is discoverable, but the
POST is refused because the endpoint depends on state the page's JavaScript
produces. Reconstructing that state by trial is exactly the probing this project
treats as prohibited, so the rule is: **KIND is exercised through its own search
control in a browser engine, never through a reconstructed request.** That is how
the D1 amendment's evidence was gathered.

Per-issue filtering does not work by supplying a code — neither the visible name
field nor the hidden issue-code field filters, and an early reading that "ETF11
resolves" was the unfiltered list's first row. Rather than reproduce the site's
issue-code popup, the intended design is **fetch the ETF-scoped list over a date
range and filter locally by `종목명`**, since that list is already restricted to
ETF-type issues and carries the name per row. This avoids depending on
undocumented popup state, and it is the owner's chosen direction.

**Amended 2026-08-20: browser-driven KIND collection is approved.** The owner
approved option (A) — automated collection through the site's own controls in a
browser engine, at low volume, never a reconstructed request. The sole approved
scope is the existing ETF-scoped disclosure-list capture and the related
correction-evidence viewer flow; it does not approve 상세검색, 관리종목,
매매거래정지, another KIND page, or a direct endpoint. This supersedes D6's
operator-driven restriction only for that exact D11 scope. D6 continues to
govern `data.krx.co.kr`, whose terms carry an explicit automated-collection
prohibition that KIND's own linked legal notice does not, and every other KIND
surface remains deferred until separately approved.

The risk the owner accepted, recorded plainly: whether the Data Marketplace
anti-automation clause reaches KIND is unresolved. KIND's own legal notice
prohibits unauthorized reproduction and redistribution but says nothing about
automation, and KIND serves no `robots.txt`. Collection therefore stays
deliberately modest — the site's own search control, low request volume, no
parameter probing — and this decision is revisited if KIND publishes a clause of
its own.

Because KIND has no API, the request that Raw records is a **browser
interaction**, which D6 already anticipated for official downloadable artifacts:
the exact bytes, the interaction that produced them, the retrieval time, and a
content hash. The hash is computed by the ingesting Rust path from the bytes on
disk, never accepted from the browser step, so the capture stage cannot
misreport what it retrieved.

**Implementation amendment 2026-08-20 — capture completeness is a required
Raw-admission fact.** `capture.json` records one closed termination value:
`clamped_duplicate`, `page_bound_reached`, `advance_control_missing`, or
`no_response`. The only complete value is `clamped_duplicate`, observed when a
request beyond the final page returns byte-identical bytes. Each requested page
gets two bounded waits separated by exactly one reinvocation of the same
site-provided control. At a configured stored-page bound of at most 40, one
additional probe distinguishes an identical terminal response from a distinct
page that proves truncation; the probe itself is not stored. The latter three
outcomes remain diagnostic staging, exit non-zero, and are rejected by both
`kind-raw` and the Raw provider before commit. A staging directory missing this
fact must be recaptured, not admitted by inference. This is an integrity rule
inside the approved browser interaction; it does not widen D11's collection
approval, rights position, or any other source allowlist.

## Consequences

- Stage5 data keeps `vendor_snapshot=true`, `strict_pit=false`, `ready=false`.
  Nothing in Step 1 changes a contract flag.
- Even after Stage6 completes, per-record strict PIT is reachable only for
  disclosure-backed events. Feed-sourced data reaches `documented_cadence` at
  best. The dataset manifest must be able to express both, so a mixed dataset
  does not silently inherit the stronger claim.
- Because the most decision-relevant technical facts — exact endpoint paths,
  field lists, pagination and terminal conditions, and any correction semantics
  — sit behind a login session or a JS-triggered document download on both KRX
  properties, Step 2's source-contract fixing cannot be completed from public
  documentation alone. It needs operator-supplied specifications.
- OpenDART has no dividend-*decision* event API; the periodic-report group does
  carry a `배당에 관한 사항` API, which is a backward-looking realized-amount
  summary without decision, record, or payment date fields. Dividend
  availability evidence for ETF11 is consequently the weakest link in Stage6 and
  is the first thing Step 2 should resolve.
- No public KSD/SEIBro dataset was found for merger, corporate split, stock
  split, or reverse split, so the `merger-split` and `rev-split` categories in
  the approved KIS allowlist may have no public KSD counterpart to compare
  against at all.

## Production gate

**Historical Step1 gate — 2026-08-19 (retained as history, superseded by the
2026-08-20 amendments).** Stage6 Step 1 delivered documentation only. Curated
publication, DB publication, five-pin, recommendation, backtest, and Paper
connection stayed `BLOCKED`.

The historical advancement condition required, from the operator:

1. approval of the proposed read-only allowlist in
   `docs/runbooks/stage6-source-contracts.md`;
2. a decision on each registration, given D3 and D8;
3. a licence interpretation for KOGL Type 2 and KRX 제11조③ against this
   project's stated personal-internal purpose, recorded as an
   `entitlement_reference`; and
4. resolution of the operator-verification checklist items that Step 2's
   adapters depend on.

**Current gate state — 2026-08-20.** The OpenDART core fixture contract is
approved but has no ETF11 use (D4), and its in-process live path is blocked by
D10. KIND D11 is approved and capture → Raw → normalization is implemented.
KRX/FSC/KSD surfaces, the backfill budget, and ETF11 identity remain deferred.
The Stage5 contract flags are unchanged — `vendor_snapshot=true`,
`strict_pit=false`, `ready=false` — and Curated/publication/five-pin and
downstream recommendation, backtest, and Paper connections remain `BLOCKED`
pending those decisions.
