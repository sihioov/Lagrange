# Owner-beta daily ETF11 input v1 — pure wire contract

D4, 2026-10-14; source baseline `619634db32526032aae07ee2024a10f69fc560c4`,
inspection HEAD `b476cdaa438056f92b98170e3364c1c049e14243`. Scope and authority:
[adopted U2](../reviews/2026-10-13-r2-daily-input-decision.md),
[D1 §§3–4](../reviews/2026-10-14-r2-daily-input-design.md#L77),
[accepted D3-06](../reviews/2026-10-14-r2-daily-input-design-review.md#L240), and
[D4/I1a assignment](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L175).
The coordinator accepts this document before I1a launch. It fixes the new byte
contract; it neither activates a policy nor changes any historical v2/v3 bytes.

## 1. Scope, notation and canonical bytes

I1a parses, structurally validates, canonicalizes, encodes and hashes **unapproved
values only**. Every `Wire` type below is unapproved, including `AdmissionV1Wire`.
No constructor of an approved input, file read/write, Raw/body parsing, normalization
execution, provider request, registry trust, DB gate, Owner/rights/control or activation
belongs here. Schema-valid hashes are not trusted pins. An actual Raw verifier must
later reopen/match all cryptographic commitments, revalidate provider rows, reconstruct
normalization, verify the historical prefix/parent and obtain independent admission.

Tables list **every field in canonical key order**. All fields are required, including
nullable fields. `null` and `[]` are different; absent nullable keys are errors.
No extra object keys, duplicate keys (including escaped equivalent keys), JSON comments,
BOM, trailing JSON, invalid UTF-8 or lone surrogate escapes. Key matching is case-sensitive.
No arbitrary `serde_json::Value` maps occur in this wire contract. Constant boolean
flags are the only boolean fields; all numbers otherwise are unsigned JSON integers.

`C(x)` is compact UTF-8 JSON: declared struct key order, canonical arrays, no whitespace
or newline, no escaped `/`, non-ASCII Unicode emitted directly, `"`/`\` escaped, JSON
control escaping as serde_json. String values here reject control characters, so their
encoder does not choose between control escapes. Do not normalize Unicode strings.
`H(b)` is `ContentHash::from_bytes(b)` yielding `sha256:` plus 64 lowercase hex digits.
No extra delimiter, filename, BOM or trailing newline enters any preimage.

Parsing accepts object-key order/whitespace variations and arbitrary order of **set
arrays**; it sorts those arrays using §7 and rejects duplicates before returning.
Encoders also sort/validate public struct values, without mutating the caller. Ordered
page arrays and copied-source chains are **not** sorted; bad order is rejected.
Hashes are over canonical values, not the incoming transport JSON. An external file
verifier must additionally require `incoming_file_bytes == C(parsed_value)` before
accepting stored manifest/policy/admission bytes as canonical immutable evidence.
Thus transport reordering may converge while a stored noncanonical file fails closed.

| Scalar | Exact wire rule | Rust representation |
|---|---|---|
| H | `sha256:[0-9a-f]{64}`, exact length 71; no uppercase | `domain::ContentHash`, strict lexical check before domain parsing |
| D | Real Gregorian `YYYY-MM-DD`, years 0001–9999, exact 10 bytes; no offset/time | `domain::TradingDate`, lexical check plus parse/round-trip |
| T | Real UTC `YYYY-MM-DDTHH:MM:SS.dddddddddZ`, exactly nine fractional digits; seconds 00–59, no leap seconds/offset | `DailyWireTimestampV1` wrapping `domain::UtcTimestamp`; custom nano-preserving serializer |
| U | Lowercase hyphenated UUID, 36 bytes, nonnil; no version restriction | `domain::BatchId`, lexical check plus parse/round-trip |
| G | `[0-9a-f]{40}`, including synthetic all-zero value | `domain::CodeCommit` with stricter lexical check |
| N | JSON integer token `0` or `[1-9][0-9]*`, u64 range; no `-0`, exponent/fraction/string/bool | `u64`; narrow casts checked |
| I | One of the 11 exact instrument strings below | `domain::InstrumentId` |
| S | One of the 11 exact six-digit symbols below | `String` |
| code | ASCII `[A-Za-z0-9_-]{1,32}`; opaque recorded provider code, no invented interpretation | `String` |
| decimal | String matching `(0\|[1-9][0-9]*)(\.[0-9]*[1-9])?`; max scale 28 and nonnegative i128 mantissa bound; `0.5` allowed, `0.0`/`1.00`/sign/exponent forbidden | `String`, validate with existing `FixedPoint::parse` |
| filename | ASCII `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`, no `..` anywhere; no separators/absolute path | `String` |

Dates need not be trading days merely because D parses. T uses formatting zeros
without asserting request-level precision. `retrieval_precision="batch"` records
the actual available acquisition granularity. Do not use the domain timestamp default
serializer: [time.rs:110](../../crates/domain/src/time.rs#L110) drops subsecond values.
Use `as_datetime().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)` after checking
the year/second restrictions. [ContentHash::parse](../../crates/domain/src/hash.rs#L29)
and UUID domain parsing are intentionally broader than these new lexical rules.

Exact ascending I list (lexicographic UTF-8 bytes), corresponding S list before `.KRX`:

```text
069500.KRX 102110.KRX 114260.KRX 132030.KRX 133690.KRX 143850.KRX
148070.KRX 153130.KRX 192090.KRX 195930.KRX 229200.KRX
```

These are the sorted existing [KR_ETF_CORE_SYMBOLS](../../crates/market-data/src/providers/kis.rs#L22),
not a new universe. Domain scale/IDs/old timestamp serialization remain untouched.

## 2. Policy and base objects

`DailyBasePinsV1Wire` is a new unapproved struct, not the existing approved pin type.

| BasePins key order | Type / value |
|---|---|
| candidate_content_sha256 | H |
| artifact_manifest_sha256 | H |
| stage5_manifest_sha256 | H |
| action_manifest_sha256 | H |
| approval_registry_sha256 | H |

| `DailyBaseV1Wire` key order | Type / value |
|---|---|
| kind | string, exactly `historical_price_only_v3` |
| pins | BasePins |

The five field meanings are unchanged. I1a accepts synthetic values but never changes
the actual registry. Later trust must require the complete existing v3 pin set and
2016-08-29..2026-08-28, 2,452 sessions/26,972 bars. I1a does not infer those historical
rows from hash strings or manufacture missing per-day base evidence.

| `DailyPricePolicyV1Wire` key order | Type / value |
|---|---|
| schema | string `owner-beta-daily-etf11-policy-v1` |
| capability | string `PRICE_RETURN_ONLY` |
| audience | string `OWNER_ONLY` |
| vendor_snapshot | boolean true |
| strict_pit | boolean false |
| base | Base |
| instruments | Vec<I>, exact ETF11 set |
| price_semantics | string `KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC` |
| cash_action_policy | string `CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1` |
| normalizer_version | string `daily-raw-reconstruction-v1` |
| materializer_version | string `daily-price-only-v1` |
| source_kinds | Vec<SourceKind>, exact set specified below |
| limits | Limits, exact v1 values below |

`DailySourceKindV1Wire`: keys `provider: String`, `market: String`,
`kind: DailyEvidenceKindV1`. Exact source_kinds set:
`(kis,kr,bars)`, `(kis,kr,calendar)`, `(kis,kr,corporate_actions)`,
`(kis,kr,reference)`, `(kis-calendar,kr,calendar)`.
`DailyEvidenceKindV1` is `bars|reference|calendar|corporate_actions`, using existing
[ResponseKind spellings](../../crates/market-data/src/contract.rs#L154). No disclosure,
candidate, account, quote-stream, adjusted Curated or unrelated source is included.

`DailyResourceLimitsV1Wire` has the following N fields in this exact order. Values are
parser/resource guards, **not acquisition quotas, retention/rights or runtime budget**.
V1 accepts only this exact object; a different guard contract requires a new schema.

| Key | Fixed value |
|---|---:|
| max_policy_bytes | 262144 |
| max_manifest_bytes | 16777216 |
| max_admission_bytes | 1048576 |
| max_normalization_bytes | 16777216 |
| max_bars_bytes | 134217728 |
| max_sessions | 20000 |
| max_civil_dates_per_addition | 370 |
| max_action_chains_per_session | 32768 |
| max_file_evidence_per_session | 262144 |
| max_source_file_bytes | 67108864 |
| max_normalized_document_bytes | 67108864 |
| max_copy_depth | 16 |
| max_pages_per_action_chain | 10 |
| max_cash_rows_per_chain | 100000 |
| max_json_depth | 32 |

Input byte limits apply before deserialization; count/depth limits apply during it
where possible and before canonical output allocation. Canonical output also obeys
its byte limit. Never truncate, silently saturate counts or relax an exceeded limit.
JSON depth counts containers: root object depth=1, each nested object/array adds one;
scalars add zero and query/header pair arrays count as containers. An isolated wrong
fixed schema/enum/constant uses InvalidLiteral; a byte/count/depth guard overflow uses
LimitExceeded. A non-u64 JSON numeric token uses InvalidType (bad JSON syntax still
uses InvalidJson). Array duplicates use DuplicateEntry; unsortable page/chain order
uses InvalidOrder; linkage/count/request contradictions use InconsistentFields.

## 3. Source-file identity, request and copied precision

`DailyFileOriginV1Wire` fields, in exact order:

| Key | Type / value |
|---|---|
| provider | string `kis` or `kis-calendar` |
| market | string `kr` |
| batch_id | U |
| batch_date | D, original committed batch date, not inferred request interval |
| batch_json_sha256 | H over original exact batch.json bytes |
| manifest_line_sha256 | H over original exact JSONL record bytes **excluding its terminating LF** |
| file_name | filename |
| kind | EvidenceKind; `kis-calendar` only permits calendar |
| wire_sha256 | H over exact stored provider bytes |
| size_bytes | N, 1..max_source_file_bytes |
| request | Request |
| response_continuation | nullable string, null or ASCII `[A-Za-z0-9]{0,8}` |
| retrieved_at | T, recorded retrieval instant of the batch identified by this Origin |
| retrieval_precision | string `batch` |

LF exclusion does **not** strip CR, spaces or re-encode JSON. A CR before LF belongs
to the exact hashed record bytes. For a complete unterminated JSON record accepted
by the existing [Raw manifest reader](../../crates/market-data/src/storage.rs#L1779),
hash the whole record (no LF exists to exclude). A syntactically incomplete final
JSONL tail is not a committed record. Reading, identifying and matching these bytes
belongs to I1b, not I1a.

`DailyFileEvidenceV1Wire` repeats **all Origin fields in the same order**, then:

| Key | Type / value |
|---|---|
| original_retrieved_at | T; if copied_from empty, equals retrieved_at; otherwise equals last origin.retrieved_at |
| original_retrieval_precision | string `batch` |
| copied_from | Vec<Origin>, immediate source first, terminal original last; 0..16 |

Chain order is meaningful. Every hop keeps kind, wire hash/size and request/response
metadata equal; timestamps are nonincreasing from current delivery to original.
No repeated `(provider,market,batch_id,file_name)` across current+chain. Provider/batch
date may differ (e.g. dedicated calendar reuse); request target/query is not rewritten.
This is structural consistency of claims; I1b must verify all source references and
that the listed chain actually ends at an uncopied committed original. Do not use
source batch UUID order to imply chronology. Missing copy evidence cannot be replaced
by setting today's retrieval instant. Recorded lexical Raw timestamps remain protected
by batch/manifest hashes, even when the new T encoding adds zero fractional digits.

`DailyFileKeyV1Wire` keys: `provider: String`, `market: String`, `batch_id: BatchId`,
`file_name: String`, `wire_sha256: ContentHash`, with the same scalar rules. It identifies
a top-level FileEvidence, not a path or an authorization token. Repeated use of one
file across civil-day proofs is permitted only with identical full FileEvidence.

| `DailyRequestMetadataV1Wire` key order | Type / value |
|---|---|
| endpoint | string, one exact path in the table below; no host/query fragment/URL |
| query | Vec<(String,String)>, each JSON element exactly a two-string array `[name,value]` |
| headers | Vec<(String,String)>, same pair shape |
| mode | string `credentialed` |

Both pair lists are canonicalized by name's ASCII bytes; duplicate names are rejected,
even if values agree. Query/header sets below are **exact**, with no extra fields.
Headers always have exactly `appkey="[REDACTED]"`, `appsecret="[REDACTED]"`,
`authorization="[REDACTED]"`, `tr_cont` and `tr_id`. No key/token/account/body/prose
field exists. The structural parser rejects any non-redacted credential value.
HTTP method/host are not recorded by existing RequestMetadata and are **not invented**
wire evidence; later provider/mode verification enforces the existing allowlist.

| Kind/class | endpoint suffix (prefix `/uapi/domestic-stock/v1/`) | tr_id | Exact query |
|---|---|---|---|
| bars | quotations/inquire-daily-itemchartprice | FHKST03010100 | FID_COND_MRKT_DIV_CODE=J, FID_INPUT_ISCD=S, FID_INPUT_DATE_1=YYYYMMDD, FID_INPUT_DATE_2=same, FID_PERIOD_DIV_CODE=D, FID_ORG_ADJ_PRC=1 |
| reference | quotations/inquire-price | FHKST01010100 | FID_COND_MRKT_DIV_CODE=J, FID_INPUT_ISCD=S |
| calendar | quotations/chk-holiday | CTCA0903R | BASS_DT=YYYYMMDD, CTX_AREA_FK="", CTX_AREA_NK="" |
| paidin-subscription | ksdinfo/paidin-capin | HHKDB669100C0 | CTS="", F_DT=YYYYMMDD, T_DT=YYYYMMDD, SHT_CD="" or S, GB1=1 |
| paidin-record | ksdinfo/paidin-capin | HHKDB669100C0 | same common fields, GB1=2 |
| bonus-issue | ksdinfo/bonus-issue | HHKDB669101C0 | same common fields, no extras |
| dividend | ksdinfo/dividend | HHKDB669102C0 | same common fields, GB1=0, HIGH_GB="" |
| merger-split | ksdinfo/merger-split | HHKDB669104C0 | same common fields, no extras |
| reverse-split | ksdinfo/rev-split | HHKDB669105C0 | same common fields, MARKET_GB=0 |
| capital-decrease | ksdinfo/cap-dcrs | HHKDB669106C0 | same common fields, no extras |

YYYYMMDD values must be valid D after insertion of hyphens. Bars/reference symbols
must match their session's identity set. Bars query date equals session_date. Calendar
BASS_DT equals the **terminal original** source batch_date; surrounding civil rows
may have different dates. Request rules match [existing query constructors](../../crates/market-data/src/providers/kis.rs#L223)
and [KSD specs](../../crates/market-data/src/providers/kis.rs#L1003); this does not
authorize changed requests. Single-page bars/reference/calendar use tr_cont="".
Preserve their response_continuation under the Origin scalar rule, including `M` or
`F`, without requiring or following another page. This matches SinglePage at
[kis.rs:652](../../crates/market-data/src/providers/kis.rs#L652), which records the
marker but only follows it for KSD. It is not StrictSinglePage. KSD handling is §4.

## 4. Action and civil-day claims

`DailyActionClassV1`: exact strings `paidin-subscription|paidin-record|bonus-issue|
dividend|merger-split|reverse-split|capital-decrease`. Canonical class sorting uses
literal ASCII order, **not** enum declaration or existing acquisition order.
`DailyActionScopeV1`: `whole_market|etf11_symbol`.

| `DailyActionChainV1Wire` key order | Type / value |
|---|---|
| logical_class | ActionClass |
| scope | ActionScope |
| symbol | nullable S; null iff whole_market |
| range_start | D |
| range_end | D, >= range_start |
| pages | Vec<FileEvidence>, 1..10, ordered page 1 to last |
| cash_dividends | Vec<CashDividendRow>, 0..100000; empty except dividend |
| cash_dividend_rows_sha256 | H = H(C(cash_dividends)); empty is H(b"[]") |

Every page has corporate_actions kind, the class's endpoint/TR/extras, F_DT/T_DT
equal chain range, SHT_CD="" for whole_market or exact symbol for etf11_symbol,
CTS blank. All pages share current provider/batch ID/batch date/query and terminal
original provider/batch ID/batch date/retrieval instant/precision; individual filenames,
wire hashes and response markers may differ. Request tr_cont is blank for
page 1, N thereafter. Response marker exactly M requires the next page; every non-M
marker (including null/blank/F) is terminal. A last M, missing page, duplicate page
wire hash or page >10 is an error. Filenames need not encode a page number: list index
is authoritative **claimed** page order, checked against real evidence by I1b.

I1a checks that the **union of claimed chain intervals**, for each of seven classes
and each ETF symbol, covers every civil date `(previous_frontier,session_date]`.
Whole-market chains cover every symbol for this structural check. Request intervals
may begin before or end after the required interval, bounded by dates and byte guards;
this does not claim relevant response completeness. Range batch_date may be its start
and outside the addition window. An overlap with different evidence is not silently
discarded: preserve all chains; the later verifier decides conflicts from actual bytes.
Known exact duplicate chains are rejected; no last-writer-wins selection.

| `DailyCashDividendRowV1Wire` key order | Type / value |
|---|---|
| symbol | S, matches chain symbol if etf11_symbol |
| record_date | D within both chain range and required addition interval |
| dividend_kind | code, opaque |
| face_value | decimal |
| cash_dividend_amount | decimal |
| cash_dividend_rate | decimal |
| stock_dividend_rate | decimal, exactly `0` |
| cash_payment_date | nullable D |
| stock_payment_date | nullable D, exactly null for cash-only |
| odd_lot_payment_date | nullable D, exactly null for cash-only |
| stock_kind | code, opaque |
| high_dividend_flag | code, opaque |
| source_file | FileKey naming one page of this chain |

No ex-date/available_at/factor exists. No field presumes a receipt timestamp.
Recorded cash rows are claimed exclusions, not newly trusted normalization.
Shared [field validation](../../crates/market-data/src/normalize.rs#L1396) and complete
non-ETF row validation remain mandatory in I1b; generic action behavior is untouched.

| `DailyCalendarBindingV1Wire` key order | Type / value |
|---|---|
| civil_date | D |
| session_type | enum `TRADING|CLOSED` |
| exchange | string `KRX` |
| timezone | string `Asia/Seoul` |
| source_file | FileEvidence of kind calendar |
| normalized_document_sha256 | H naming this date's calendar DocumentCommitment |
| legacy_projection | nullable LegacyCalendarProjection |

Exactly one binding per civil date in the required interval; its sole TRADING date
is session_date, all earlier civil dates CLOSED. This is claimed coverage, not a
calendar truth derived from weekday or the clock. Every source's requested BASS_DT
must itself appear as a claimed calendar binding somewhere in cumulative daily_sources
when it lies in the added range; if outside it, I1b must verify the original target in
Raw. I1a never inserts DB rows. D5 owns offline daily projection/SQL joins; null
legacy_projection is permitted and does not grant scheduling eligibility.

`DailyLegacyCalendarProjectionV1Wire` keys in order:
`source: String="kis"`, `source_version: String="kis-chk-holiday-v1:schema-1"`,
`source_batch_id: BatchId`, `source_file_name: String="calendar.json"`,
`document_sha256: ContentHash`, `retrieved_at: DailyWireTimestampV1`.
The source_version is the current [KIS_CALENDAR_SOURCE_VERSION](../../crates/market-data/src/normalize.rs#L38).
This records one common claimed tuple for calendar/version/data_batches, not three
invented records. Its hash is a **normalized document** hash, not source_file.wire_sha256.
The literal source_version must match the existing source contract; see §10 source check.
This legacy document_sha256 is an independent opaque commitment: it need not equal
the binding's new daily reconstructed document hash. The legacy calendar document
has a different schema ([normalize.rs:1191](../../crates/market-data/src/normalize.rs#L1191)).
Later DB verification must confirm the legacy three-way tuple and its original
lineage; I1a cannot distinguish a substituted schema-valid hash without those bytes.
Null is appropriate when the daily proof uses surrounding Raw rows.

## 5. Normalization, session, manifest and admission

| `DailyNormalizedDocumentV1Wire` key order | Type / value |
|---|---|
| kind | EvidenceKind |
| session_date | D; calendar may be a CLOSED civil date |
| normalizer_version | string `daily-raw-reconstruction-v1` |
| sha256 | H of reconstructed document's exact bytes, opaque to I1a |
| size_bytes | N, 1..max_normalized_document_bytes |
| row_count | N |
| source_files | Vec<FileKey>, nonempty set identifying only this kind's top-level FileEvidence |
| source_evidence_sha256 | H of SourceCommitment preimage below |

`DailyNormalizedSourceCommitmentV1Wire` keys are `schema: String=
"owner-beta-daily-normalized-source-v1"`, `kind: EvidenceKind`,
`session_date: TradingDate`, `source_files: Vec<FileKey>` in that order.
source_evidence_sha256=H(C(SourceCommitment)). No generated normalized batch UUID
is assigned by I1a. D5/I1b owns document reconstruction/identity; changing that algorithm
must change its normalizer version/commitment, not reinterpret this version silently.

| `DailySessionEvidenceV1Wire` key order | Type / value |
|---|---|
| session_date | D, >2026-08-28 |
| previous_frontier | D, >=2026-08-28 and <session_date |
| instruments | Vec<I>, exact ETF11 set |
| bars | Vec<FileEvidence>, exactly 11, one request symbol each |
| reference | Vec<FileEvidence>, exactly 11, one request symbol each |
| calendar | Vec<CalendarBinding>, 1..370 |
| actions | Vec<ActionChain>, 7..32768, coverage as §4 |
| normalized_documents | Vec<DocumentCommitment>, exactly one bars, reference, corporate_actions for session_date, plus one calendar per civil binding |

Bars/reference original batch_date equals session_date (for copies use terminal
original). Other file dates are their actual committed partition dates. Repeated file
identities anywhere in one document must have identical complete metadata. Normalized
bars/reference row_count=11, calendar row_count=1. Corporate_actions row_count is
bounded by 0..sum of cash_dividends counts: equivalent overlapping witness chains
may repeat an observed cash row, while D5/I1b selects one semantic contribution.
I1a checks that bound only; I1b verifies the exact selected row count and bytes.
Admission cash_dividend_row_count still counts all claimed witness occurrences.
Source_files must equal the unique corresponding FileKeys;
calendar commitments each name that binding's file. Bound top-level FileEvidence
occurrences per session by max_file_evidence_per_session (bars+reference+calendar
bindings+all action pages); copied hops do not count as top-level occurrences.

`DailyBarsDescriptorV1Wire`: ordered keys `file: String="bars.ndjson"`,
`sha256: ContentHash`, `size_bytes: u64` in 1..max_bars_bytes,
`row_count: u64` equal 11*manifest.sessions.len(). No bars are parsed by I1a.
`DailyNormalizationDescriptorV1Wire`: keys `file: String="normalization.json"`,
`sha256: ContentHash`, `size_bytes: u64` in 1..max_normalization_bytes,
`row_count: u64` equal the flattened normalized_documents count (not sum of data rows).

| `DailyPriceManifestV1Wire` key order | Type / value |
|---|---|
| schema | string `owner-beta-daily-etf11-input-v1` |
| capability | string `PRICE_RETURN_ONLY` |
| audience | string `OWNER_ONLY` |
| vendor_snapshot | boolean true |
| strict_pit | boolean false |
| base | Base |
| parent_admission_sha256 | nullable H; null iff exactly one added session |
| range_start | D, exactly 2016-08-29 |
| range_end | D, >2026-08-28; equals last session and last daily_sources.session_date |
| instruments | Vec<I>, exact ETF11 set |
| sessions | Vec<D>, 3..20000; first=range_start, contains 2026-08-28, last=range_end |
| daily_sources | Vec<SessionEvidence>, exactly all sessions >2026-08-28 |
| bars | BarsDescriptor |
| normalization | NormalizationDescriptor |
| policy_sha256 | H |
| materializer_version | string `daily-price-only-v1` |
| code_commit | G, producer release lineage, not current process HEAD inferred by parser |
| content_sha256 | H = H(content preimage) |

`DailyPriceManifestDraftV1Wire` has the identical fields/order **without** content_sha256.
First previous_frontier=2026-08-28; each later previous_frontier equals prior added
session. The final input extends one session beyond its external parent; I1a can only
check this internal chain and null rule. Actual parent bytes/prefix, base calendar and
full 2,452-session history remain I1b trust checks. The intentionally small synthetic
fixture therefore parses but is never an approved historical continuation.

`DailyEvidenceCommitmentV1Wire` keys `schema: String=
"owner-beta-daily-etf11-evidence-v1"`, `daily_sources: Vec<SessionEvidence>`.
`DailyNormalizationProofV1Wire` keys `schema: String=
"owner-beta-daily-etf11-normalization-v1"`, `normalizer_version: String=
"daily-raw-reconstruction-v1"`, `evidence_sha256: ContentHash`,
`documents: Vec<DocumentCommitment>` (flattened all sessions, sorted/deduplicated by §7).
These are the complete bytes of normalization.json, a commitment proof, not a generic
READY publication or a normalized provider body. sha256/size/row_count must exactly
match the manifest descriptor when structurally checking the bundle.

| `DailyAdmissionCountsV1Wire` key order | Type / value |
|---|---|
| instrument_count | N, exactly 11 |
| session_count | N, manifest.sessions.len() |
| bar_count | N, 11*session_count |
| added_session_count | N, daily_sources.len() |
| source_file_count | N, unique top-level FileKeys across all evidence, excluding copied hops |
| calendar_row_count | N, sum calendar bindings |
| action_chain_count | N, sum action chains |
| action_page_count | N, sum pages |
| cash_dividend_row_count | N, sum cash_dividends lengths |
| normalized_document_count | N, flattened unique document commitments |

| `DailyPriceAdmissionV1Wire` key order | Type / value |
|---|---|
| schema | string `owner-beta-daily-etf11-admission-v1` |
| content_sha256 | H |
| manifest_sha256 | H |
| policy_sha256 | H |
| parent_admission_sha256 | nullable H |
| base | Base |
| verifier_code_commit | G, explicit input; never inferred by parser |
| normalizer_version | string `daily-raw-reconstruction-v1` |
| evidence_sha256 | H |
| counts | AdmissionCounts |

Admission parse alone enforces instrument_count=11, session_count=3..20000,
bar_count=11*session_count, added_session_count=1..(session_count-2),
calendar_row_count=added_session_count..370*added_session_count,
action_chain_count=7*added_session_count..32768*added_session_count,
action_page_count=action_chain_count..10*action_chain_count,
source_file_count=1..262144*added_session_count,
cash_dividend_row_count=0..100000*action_chain_count, and
normalized_document_count=3*added_session_count+calendar_row_count. Its parent is
null iff added_session_count=1. All arithmetic is checked u64. Exact values/evidence
require the bundle function, not these local bounds.

No admitted_at, ID, DB status, owner, entitlement, epoch, reservation, signature,
path, self-hash or mutable latest pointer enters admission bytes. Those are separate
authorities. A wire admission is a claim; its parser conveys no admission authority.
`DailyPricePinsV1Wire` keys in order: `content_sha256`, `manifest_sha256`,
`admission_sha256`, `policy_sha256`, all ContentHash.

## 6. Exact hash graph and Rust API

| Hash / output | Exact preimage |
|---|---|
| policy_sha256 | C(Policy) |
| source_evidence_sha256 | C(NormalizedSourceCommitment), §5 |
| cash_dividend_rows_sha256 | C(sorted CashDividendRow array), including source_file |
| evidence_sha256 | C(EvidenceCommitment), includes all normalized document commitments, copy chains, times and requests |
| normalization.sha256 | C(NormalizationProof), including evidence_sha256 and flattened documents |
| content_sha256 | C(ManifestDraft); omit **only** content_sha256, with all other keys present |
| manifest_sha256 | C(Manifest), including its content_sha256 |
| admission_sha256 | C(Admission); no self-hash key exists |
| raw/batch/manifest-line/document/bars descriptors | Exact external byte commitments, **not checked by I1a** |

There is no circular dependency: normalized document byte commitments depend on
their original source lineage, not the eventual EvidenceCommitment/manifest/admission;
EvidenceCommitment includes those document commitments; NormalizationProof includes
the evidence hash; ManifestDraft commits that proof's descriptor; Admission commits
the final manifest and evidence. No timestamp generated by I1a participates.

All table objects are public Rust structs with public fields in the declared order,
`Clone, Debug, PartialEq, Eq`, except private scalar timestamp wrapper. Vec/object
types follow the exact tables; constants use String; nullable uses Option<T>, never
skip_serializing_if/default. Enums are declared public and serialized to their exact
literals. Field aliases are not accepted. Implement typed duplicate-aware deserialization
without a lossy Value intermediate; serde derive alone must not make absent Option
keys silently legal. No public struct/type/function contains `Approved`, `Admitted`,
`Verified`, trusted registry, filesystem root, DB handle or approval capability.

Exact struct export inventory from `market_data::daily_price_only` (25 table/draft
structs; names are part of the interface):

```text
DailyBasePinsV1Wire DailyBaseV1Wire DailyPricePolicyV1Wire DailySourceKindV1Wire
DailyResourceLimitsV1Wire DailyFileOriginV1Wire DailyFileEvidenceV1Wire
DailyFileKeyV1Wire DailyRequestMetadataV1Wire DailyActionChainV1Wire
DailyCashDividendRowV1Wire DailyCalendarBindingV1Wire DailyLegacyCalendarProjectionV1Wire
DailyNormalizedDocumentV1Wire DailyNormalizedSourceCommitmentV1Wire
DailySessionEvidenceV1Wire DailyBarsDescriptorV1Wire DailyNormalizationDescriptorV1Wire
DailyPriceManifestV1Wire DailyPriceManifestDraftV1Wire DailyEvidenceCommitmentV1Wire
DailyNormalizationProofV1Wire DailyAdmissionCountsV1Wire DailyPriceAdmissionV1Wire
DailyPricePinsV1Wire
```

Exact remaining exports (module registration only in I1a):

```rust
// Every *V1Wire table type is exported; serde literal mappings are §§2/4.
pub enum DailyEvidenceKindV1 { Bars, Reference, Calendar, CorporateActions }
pub enum DailyActionClassV1 {
    PaidinSubscription, PaidinRecord, BonusIssue, Dividend,
    MergerSplit, ReverseSplit, CapitalDecrease,
}
pub enum DailyActionScopeV1 { WholeMarket, Etf11Symbol }
pub enum DailySessionTypeV1 { Trading, Closed }
pub struct DailyWireTimestampV1(/* private domain::UtcTimestamp */);
impl DailyWireTimestampV1 {
    pub fn parse(value: &str) -> Result<Self, DailyWireErrorV1>;
    pub fn as_timestamp(&self) -> domain::UtcTimestamp;
}
pub enum DailyWireErrorV1 {
    InvalidJson, DuplicateKey, UnknownField, MissingField, InvalidType,
    InvalidLiteral, InvalidHash, InvalidDate, InvalidTimestamp, InvalidUuid,
    InvalidCommit, InvalidDecimal, InvalidName, DuplicateEntry, InvalidOrder,
    LimitExceeded, InconsistentFields, HashMismatch,
}
pub fn parse_daily_price_policy_v1(bytes: &[u8])
    -> Result<DailyPricePolicyV1Wire, DailyWireErrorV1>;
pub fn encode_daily_price_policy_v1(value: &DailyPricePolicyV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn hash_daily_price_policy_v1(value: &DailyPricePolicyV1Wire)
    -> Result<domain::ContentHash, DailyWireErrorV1>;
pub fn parse_daily_price_manifest_v1(bytes: &[u8])
    -> Result<DailyPriceManifestV1Wire, DailyWireErrorV1>;
pub fn encode_daily_price_manifest_v1(value: &DailyPriceManifestV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn daily_price_content_preimage_v1(value: &DailyPriceManifestDraftV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn seal_daily_price_manifest_v1(value: &DailyPriceManifestDraftV1Wire)
    -> Result<DailyPriceManifestV1Wire, DailyWireErrorV1>;
pub fn hash_daily_price_manifest_v1(value: &DailyPriceManifestV1Wire)
    -> Result<domain::ContentHash, DailyWireErrorV1>;
pub fn parse_daily_price_admission_v1(bytes: &[u8])
    -> Result<DailyPriceAdmissionV1Wire, DailyWireErrorV1>;
pub fn encode_daily_price_admission_v1(value: &DailyPriceAdmissionV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn hash_daily_price_admission_v1(value: &DailyPriceAdmissionV1Wire)
    -> Result<domain::ContentHash, DailyWireErrorV1>;
pub fn daily_price_evidence_preimage_v1(value: &[DailySessionEvidenceV1Wire])
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn daily_normalized_source_preimage_v1(value: &DailyNormalizedSourceCommitmentV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn daily_cash_dividend_rows_preimage_v1(value: &[DailyCashDividendRowV1Wire])
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn parse_daily_normalization_proof_v1(bytes: &[u8])
    -> Result<DailyNormalizationProofV1Wire, DailyWireErrorV1>;
pub fn encode_daily_normalization_proof_v1(value: &DailyNormalizationProofV1Wire)
    -> Result<Vec<u8>, DailyWireErrorV1>;
pub fn validate_daily_price_wire_bundle_v1(
    policy: &DailyPricePolicyV1Wire,
    manifest: &DailyPriceManifestV1Wire,
    proof: &DailyNormalizationProofV1Wire,
    admission: &DailyPriceAdmissionV1Wire,
    pins: &DailyPricePinsV1Wire,
) -> Result<(), DailyWireErrorV1>;
```

Seal means adding a **self-consistency hash to an unapproved value**, not admission.
Manifest parse/encode/hash validates its declared content hash; seal computes it from
the draft. The bundle function validates all structural equalities in this document,
policy/base/pins/hash links, normalization proof and exact derived counts. Individual
admission/policy parsers validate shapes/local bounds only; they cannot check external
manifest/policy equality. The bundle function still checks no external bytes/registry.
Pins have no standalone parser API: their struct values are validated by bundle.

Errors are payload-free and Display is exactly the Rust variant name. Do not retain
serde/domain error strings, submitted values, paths or provider prose. Within a valid
JSON object, report duplicate/unknown/missing/type violations before semantic checks;
then lexical errors, bounds/order/duplicates, field consistency, internal hashes and
finally bundle links. Tests involving multiple different simultaneous violations must
assert rejection rather than a traversal-dependent first code. Isolated single-fault
tests assert the corresponding enum. Invalid syntax/UTF-8 is InvalidJson.

## 7. Canonical arrays and duplicate identity

All lexicographic comparisons use UTF-8 bytes; dates use real date order, integer
comparisons numeric. Sort on the complete tuple specified here, not serde enum Ord.

| Array | Canonical sort key / duplicate rule |
|---|---|
| instruments | I literal bytes; duplicate or different set rejected |
| sessions | D ascending; duplicate rejected |
| daily_sources | session_date ascending; duplicate date rejected |
| source_kinds | (provider,market,kind literal) ascending; duplicate tuple rejected |
| query / headers | pair name ascending; duplicate name rejected |
| bars / reference | (provider,market,batch_id,file_name) ascending; duplicate identity rejected; also unique request symbol |
| calendar | civil_date ascending; duplicate date rejected |
| actions | (logical_class literal,scope literal,symbol with null before strings,range_start,range_end,first page FileKey tuple) ascending; identical key rejected |
| cash_dividends | (symbol,record_date,dividend_kind,source_file tuple,C(row) bytes) ascending; identical canonical row rejected |
| source_files | (provider,market,batch_id,file_name,wire_sha256) ascending; duplicate identity rejected even with different hash |
| normalized_documents / proof.documents | (session_date,kind literal) ascending; duplicate key rejected |
| pages | preserve index; repeated file identity/wire hash rejected |
| copied_from | preserve immediate→original; repeated identity rejected |

FileKey tuple order is the five declared keys. File identity always omits wire_sha256
for uniqueness: changing a hash is a conflict, not a second identity. A file may occur
in different bindings/chains only if full metadata is identical; copied-source origin
references use the same identity discipline. Structural interval coverage and internal
equality operate on canonicalized sets, so reordered sets yield identical bytes/hashes.
Action chains with identical class/scope/symbol/range but genuinely different first
file identity remain preserved for later conflict verification; I1a does not select one.

## 8. Synthetic golden construction

The following complete Python construction is normative for the golden **values and
preimages**, using insertion-ordered dictionaries whose order is the tables above.
`C` generates the exact compact UTF-8 bytes; no file/network/clock access occurs.
It is not an implementation of the Rust parser/validator. Its synthetic wire bodies
are deliberately not KIS JSON and its three historical dates are not the 2,452-session
registry input: passing I1a does not authorize Raw trust, admission or activation.
Expected bytes are the deterministic variables named in the output table; print only
sizes/hashes for review, not market/provider bodies. No sentinel hash is a production pin.

```python
import json, hashlib
def C(x):
    return json.dumps(x, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
def H(b):
    return "sha256:" + hashlib.sha256(b).hexdigest()
def SH(n):
    return "sha256:" + format(n, "064x")
symbols = ["069500", "102110", "114260", "132030", "133690", "143850",
           "148070", "153130", "192090", "195930", "229200"]
instruments = [s + ".KRX" for s in symbols]
base = {"kind": "historical_price_only_v3", "pins": {
    "candidate_content_sha256": SH(1), "artifact_manifest_sha256": SH(2),
    "stage5_manifest_sha256": SH(3), "action_manifest_sha256": SH(4),
    "approval_registry_sha256": SH(5)}}
limits = dict(zip([
    "max_policy_bytes", "max_manifest_bytes", "max_admission_bytes",
    "max_normalization_bytes", "max_bars_bytes", "max_sessions",
    "max_civil_dates_per_addition", "max_action_chains_per_session",
    "max_file_evidence_per_session", "max_source_file_bytes",
    "max_normalized_document_bytes", "max_copy_depth",
    "max_pages_per_action_chain", "max_cash_rows_per_chain", "max_json_depth"],
    [262144, 16777216, 1048576, 16777216, 134217728, 20000, 370,
     32768, 262144, 67108864, 67108864, 16, 10, 100000, 32]))
source_kinds = [{"provider": p, "market": "kr", "kind": k}
    for p,k in [("kis","bars"), ("kis","calendar"), ("kis","corporate_actions"),
                ("kis","reference"), ("kis-calendar","calendar")]]
policy = {"schema": "owner-beta-daily-etf11-policy-v1", "capability": "PRICE_RETURN_ONLY",
    "audience": "OWNER_ONLY", "vendor_snapshot": True, "strict_pit": False,
    "base": base, "instruments": instruments,
    "price_semantics": "KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC",
    "cash_action_policy": "CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1",
    "normalizer_version": "daily-raw-reconstruction-v1",
    "materializer_version": "daily-price-only-v1", "source_kinds": source_kinds,
    "limits": limits}
prefix = "/uapi/domestic-stock/v1/"
ts = "2026-08-31T09:00:00.123456789Z"
batch = "00000000-0000-4000-8000-000000000001"
def F(kind, name, path, tr, query):
    request = {"endpoint": prefix+path, "query": sorted(query.items()),
        "headers": sorted({"authorization":"[REDACTED]", "appkey":"[REDACTED]",
            "appsecret":"[REDACTED]", "tr_id":tr, "tr_cont":""}.items()),
        "mode":"credentialed"}
    body = ("synthetic-wire:"+name).encode()
    return {"provider":"kis", "market":"kr", "batch_id":batch,
        "batch_date":"2026-08-31", "batch_json_sha256":SH(20),
        "manifest_line_sha256":SH(21), "file_name":name, "kind":kind,
        "wire_sha256":H(body), "size_bytes":len(body), "request":request,
        "response_continuation":"", "retrieved_at":ts, "retrieval_precision":"batch",
        "original_retrieved_at":ts, "original_retrieval_precision":"batch",
        "copied_from":[]}
def K(f):
    return {k:f[k] for k in ["provider","market","batch_id","file_name","wire_sha256"]}
def key(f):
    return tuple(f[k] for k in ["provider","market","batch_id","file_name"])
bars = sorted([F("bars","bars-"+s+".json", "quotations/inquire-daily-itemchartprice",
    "FHKST03010100", {"FID_COND_MRKT_DIV_CODE":"J", "FID_INPUT_ISCD":s,
    "FID_INPUT_DATE_1":"20260831", "FID_INPUT_DATE_2":"20260831",
    "FID_PERIOD_DIV_CODE":"D", "FID_ORG_ADJ_PRC":"1"}) for s in symbols], key=key)
reference = sorted([F("reference","reference-"+s+".json", "quotations/inquire-price",
    "FHKST01010100", {"FID_COND_MRKT_DIV_CODE":"J", "FID_INPUT_ISCD":s})
    for s in symbols], key=key)
calfile = F("calendar","calendar-page-01.json", "quotations/chk-holiday", "CTCA0903R",
    {"BASS_DT":"20260831", "CTX_AREA_FK":"", "CTX_AREA_NK":""})
specs = [
    ("bonus-issue","bonus-issue","HHKDB669101C0",{}),
    ("capital-decrease","cap-dcrs","HHKDB669106C0",{}),
    ("dividend","dividend","HHKDB669102C0",{"GB1":"0","HIGH_GB":""}),
    ("merger-split","merger-split","HHKDB669104C0",{}),
    ("paidin-record","paidin-capin","HHKDB669100C0",{"GB1":"2"}),
    ("paidin-subscription","paidin-capin","HHKDB669100C0",{"GB1":"1"}),
    ("reverse-split","rev-split","HHKDB669105C0",{"MARKET_GB":"0"})]
actions = []
for cls,path,tr,extra in specs:
    page = F("corporate_actions",cls+"-page-01.json", "ksdinfo/"+path,tr,
        {"CTS":"", "F_DT":"20260829", "T_DT":"20260831", "SHT_CD":"", **extra})
    actions.append({"logical_class":cls, "scope":"whole_market", "symbol":None,
        "range_start":"2026-08-29", "range_end":"2026-08-31", "pages":[page],
        "cash_dividends":[], "cash_dividend_rows_sha256":H(b"[]")})
def N(kind,date,files,rows):
    keys = sorted([K(f) for f in files], key=lambda f: tuple(f.values()))
    src = {"schema":"owner-beta-daily-normalized-source-v1", "kind":kind,
           "session_date":date, "source_files":keys}
    body = ("synthetic-normalized:"+kind+":"+date).encode()
    return {"kind":kind, "session_date":date,
        "normalizer_version":"daily-raw-reconstruction-v1", "sha256":H(body),
        "size_bytes":len(body), "row_count":rows, "source_files":keys,
        "source_evidence_sha256":H(C(src))}
documents = [N("bars","2026-08-31",bars,11), N("reference","2026-08-31",reference,11),
    N("corporate_actions","2026-08-31",[a["pages"][0] for a in actions],0)]
calendar = []
for date,typ in [("2026-08-29","CLOSED"),("2026-08-30","CLOSED"),("2026-08-31","TRADING")]:
    doc = N("calendar",date,[calfile],1)
    documents.append(doc)
    calendar.append({"civil_date":date, "session_type":typ, "exchange":"KRX",
        "timezone":"Asia/Seoul", "source_file":calfile,
        "normalized_document_sha256":doc["sha256"], "legacy_projection":None})
documents.sort(key=lambda d:(d["session_date"],d["kind"]))
session = {"session_date":"2026-08-31", "previous_frontier":"2026-08-28",
    "instruments":instruments, "bars":bars, "reference":reference,
    "calendar":calendar, "actions":actions, "normalized_documents":documents}
evidence = {"schema":"owner-beta-daily-etf11-evidence-v1", "daily_sources":[session]}
proof = {"schema":"owner-beta-daily-etf11-normalization-v1",
    "normalizer_version":"daily-raw-reconstruction-v1", "evidence_sha256":H(C(evidence)),
    "documents":documents}
bars_body = b"synthetic-bars-not-approved"
draft = {"schema":"owner-beta-daily-etf11-input-v1", "capability":"PRICE_RETURN_ONLY",
    "audience":"OWNER_ONLY", "vendor_snapshot":True, "strict_pit":False,
    "base":base, "parent_admission_sha256":None, "range_start":"2016-08-29",
    "range_end":"2026-08-31", "instruments":instruments,
    "sessions":["2016-08-29","2026-08-28","2026-08-31"], "daily_sources":[session],
    "bars":{"file":"bars.ndjson", "sha256":H(bars_body), "size_bytes":len(bars_body),
            "row_count":33},
    "normalization":{"file":"normalization.json", "sha256":H(C(proof)),
        "size_bytes":len(C(proof)), "row_count":6}, "policy_sha256":H(C(policy)),
    "materializer_version":"daily-price-only-v1", "code_commit":"1"*40}
manifest = {**draft, "content_sha256":H(C(draft))}
counts = {"instrument_count":11, "session_count":3, "bar_count":33,
    "added_session_count":1, "source_file_count":30, "calendar_row_count":3,
    "action_chain_count":7, "action_page_count":7, "cash_dividend_row_count":0,
    "normalized_document_count":6}
admission = {"schema":"owner-beta-daily-etf11-admission-v1",
    "content_sha256":H(C(draft)), "manifest_sha256":H(C(manifest)),
    "policy_sha256":H(C(policy)), "parent_admission_sha256":None, "base":base,
    "verifier_code_commit":"2"*40, "normalizer_version":"daily-raw-reconstruction-v1",
    "evidence_sha256":H(C(evidence)), "counts":counts}
pins = {"content_sha256":H(C(draft)), "manifest_sha256":H(C(manifest)),
    "admission_sha256":H(C(admission)), "policy_sha256":H(C(policy))}
for name,obj in [("policy",policy),("evidence",evidence),("normalization",proof),
                 ("content_preimage",draft),("manifest",manifest),("admission",admission)]:
    print(name, len(C(obj)), H(C(obj)))
print("empty_cash_rows", 2, H(b"[]"))
```

Actual Python calculation of the preceding construction produced these exact values.
The construction, rather than a pretty JSON rendering, is the complete fixture. All
hashes include the `sha256:` prefix shown here. Copied-source and legacy projection
fixtures are separate required tests with the exact nested schemas above.

| Preimage variable | Bytes | H(canonical bytes) |
|---|---:|---|
| policy | 1827 | `sha256:b9189f229d2863922a6fcc3d839c7e12b25521bc0b1ba0c0a9534b3fd565f474` |
| evidence | 44176 | `sha256:9bf59d097b59b70fd0978f3ae9afe27d93243af78f251b8ca50d8eb1b10d7b34` |
| proof (normalization.json) | 8790 | `sha256:f1baaf8e73ada15381d59b6936f95463e31dcd8810063024b0fecd09d5e05f59` |
| draft (content preimage) | 45616 | `sha256:e8d61163bd89509d012d7a1f84d47748e866c5c69119a1d97afd73755bc4efdf` |
| manifest | 45707 | `sha256:2e1e5763e443c5ff7ca2d44e39c15bcd9fd7a8477b4e3d3bbff76a067a392daf` |
| admission | 1353 | `sha256:3409c138a55d73bd9d354c7e9f08101e4613d96047ced5075d2e54bc826b2654` |
| empty cash array `[]` | 2 | `sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945` |

Exact additional fixture recipes (all starting from the construction above):

- **Second addition:** retain the first session verbatim. Create the same 11 bars,
  11 references and seven whole-market one-page chains for 2026-09-01, all with batch
  UUID `00000000-0000-4000-8000-000000000002`, batch_date=2026-09-01,
  batch_json_sha256=SH(22), manifest_line_sha256=SH(23),
  retrieved_at=original_retrieved_at=`2026-09-01T09:00:00.123456789Z`, copied_from=[].
  Keep filenames and synthetic wire bodies/hashes/sizes unchanged; use bar/calendar
  query date `20260901`, action F_DT=T_DT=`20260901`, chain range_start=range_end=
  2026-09-01. New session.previous_frontier=2026-08-31. Its one calendar binding is
  2026-09-01/TRADING; legacy_projection=null. Create four commitments with `N` above
  (bars=11, reference=11, calendar=1, corporate_actions=0), using the new FileKeys.
  Set its calendar hash to that calendar commitment. Append the new date/session;
  parent_admission_sha256=pins.admission_sha256, range_end=2026-09-01, bars.row_count=44.
  Rebuild EvidenceCommitment, NormalizationProof and its descriptor, then content,
  manifest and admission hashes. Counts in §5 order become
  `[11,4,44,2,60,4,14,14,0,10]`. Change this parent to SH(99), reseal manifest and
  change admission parent/content/manifest fields accordingly. Only content/manifest/
  admission hashes change; policy/evidence/proof remain byte-identical.
- **Copy precision:** replace the first fixture's calendar FileEvidence consistently
  everywhere with a copy whose current batch UUID ends in `0003`, current batch_date=
  2026-09-01 and retrieved_at=`2026-09-01T09:00:00.123456789Z`; retain current batch
  hashes SH(20)/SH(21) as synthetic claims. copied_from=[the original calfile's 14
  Origin fields in table order]; original_retrieved_at stays the original T. Request,
  response, wire hash/size and kind stay unchanged; thus terminal BASS_DT/batch date
  are still 20260831/2026-08-31. Keep the original civil bindings/row counts. Regenerate
  their N source lists/source hashes with the copied top-level FileKey, then proof/
  descriptors/content/manifest/admission. Current T changing within allowed ordering
  changes evidence and descendants even if document byte commitments stay unchanged.
- **Non-null legacy tuple:** on the first fixture's 2026-08-31 calendar binding only,
  set legacy_projection to `{source:"kis",source_version:"kis-chk-holiday-v1:schema-1",
  source_batch_id:batch,source_file_name:"calendar.json",document_sha256:SH(42),
  retrieved_at:ts}` in that key order. Other bindings remain
  null. Regenerate evidence/proof/descriptor and downstream hashes. This still asserts
  no DB existence. Omit any tuple key to test MissingField. Substituting another
  schema-valid legacy hash and resealing remains structurally valid; I1b must reject
  a mismatch against the actual legacy evidence. The daily hash remains separate.

## 9. Required deterministic tests (future I1a, not D4 execution)

1. Golden parse→encode returns exactly the construction's bytes; all six hashes/lengths
   match the final table. Bundle succeeds while making no Raw/registry/DB trust claim.
2. Reverse all set arrays, reorder every object key and add JSON whitespace: same
   canonical bytes/hashes. Reorder page sequence or copied chain: InvalidOrder or
   InconsistentFields, not silently sorted. Same one-page chain remains valid.
3. Duplicate JSON keys at root and every nested object (also `schema` vs `\u0073chema`)
   fail DuplicateKey; unknown single keys fail UnknownField; remove each key including
   a nullable key fails MissingField. Null on nonnullable, wrong scalar/array/pair type
   fails InvalidType. No lossy Value parsing before duplicate detection.
4. Isolated uppercase/short hashes, offset or variable-fraction T, impossible date,
   noncanonical/nil UUID, abbreviated commit, `-0`/float/exponent/overflow N, decimal
   noncanonical zeros and unsafe names fail their lexical/type errors. Preserve a
   nonzero nanosecond T round trip exactly. Zero-nanosecond format has nine zero digits.
5. Duplicate identity with changed hash/metadata fails; an identical shared calendar
   file across distinct civil dates passes. Missing ETF symbol or normalized commitment,
   missing civil date/class/scope interval, wrong request/continuation, incomplete page
   chain, copy cycle/time/original mismatch, or claimed stock dividend fails structurally.
6. Exercise every fixed guard at its bound and bound+1; reject overlarge incoming and
   canonical bytes. Use count fixtures without external files. Declared row counts can
   be mathematically checked, but fabricated provider bytes cannot be detected by I1a.
7. Change parent H on a valid two-addition fixture: content/manifest/admission hashes
   change, policy/evidence/proof hashes do not. First-addition nonnull parent is rejected.
   Changing the final added session requires corresponding internal dates/coverage.
8. Change one schema-valid policy base pin and matching manifest/admission base, recompute
   policy/content/manifest/admission: those hashes change; evidence/proof do not. Old
   declared content/policy links fail HashMismatch. This is wire sensitivity, not trust.
9. Change producer commit: content/manifest/admission change, policy/evidence/proof do not.
   Change only verifier commit: only admission hash changes. Change original retrieval,
   copied hop, request or manifest-line commitment consistently: evidence/proof/content/
   manifest/admission change; policy unchanged. Stale internal/bundle hashes fail.
10. Replace opaque bars or document commitment by another schema-valid hash and update
    links: structural success is allowed; later independent verification must reject
    mismatched actual bytes. This negative trust test prevents accidental approved types.
11. Verify empty cash preimage exactly `[]`; nonempty row/source/time changes affect its
    defined hash. No wall clock, generated UUID, filesystem traversal/mtime or map
    iteration order influences preimages. No legacy serde/registry/normalizer dependency
    is modified: unchanged existing historical v2/v3 golden byte/hash fixtures remain
    mandatory hosted regression evidence after implementation.
12. Single-page bars/reference/calendar with a retained `M` or `F` response marker
    remains structurally valid after resealing, while nonblank request tr_cont fails.
    KSD last-page `M` without the next page still fails. Equivalent overlapping action
    witnesses may contain two claimed cash rows and a reconstructed row_count of one;
    counts record two witness occurrences, and the selected count remains I1b's check.

## 10. Source alignment and D4 handoff

Pure wire additions are new types, not expansions of existing historical deserializers.
Existing source confirms RequestMetadata pair arrays, recorded batch/copy timestamps,
seven KSD classes, ETF11 identities and broader domain parsers. D5 independently owns
actual supplier, normalization identity/reconstruction, offline projection, ACL/SQL/
publication/harness. Their implementation must consume these exact commitments or
return a named versioned-contract conflict; it must not silently reinterpret bytes.
Owner/control/budget/release trust values remain typed inputs outside I1a and need
not exist to implement this schema. Historical replay is never dispatched to this parser.

D4 wrote only this contract. Deviations: none; the broader existing hash/UUID/time
parsers were handled by strict new wire checks without modifying domain/legacy code.
Actual checks: bounded source/document reads; pure local Python JSON/hash calculations
for all seven golden preimages; relative source links/line bounds, schema field/type
inventory, key/sort/limit consistency and whitespace/fence checks. Golden lengths/
hashes, fixture ETF11/count/pair-order/redaction, producer/verifier/policy-base hash
sensitivity and copied-origin precision recipe checks passed. The final handoff
reports the complete line range. These are document/fixture calculations, not Rust
test, normalization or runtime results.

Unresolved wire contradictions: none identified after the source alignment above.
Follow-ups: coordinator accepts/freezes this file and launches I1a; D5/I1b resolves
the deliberately separate normalization/projection/supply/trust/runtime contracts.
Not found/unverified: there is no existing daily wire implementation; Rust compile/
tests and provider/Raw/DB/storage/activation/Owner/runtime behavior are not verified.
None is claimed. No product source/test/config/plan/progress/report was modified;
no Git stage/commit/push, root/DB/Docker/provider/network/production read, build or
runtime execution, or delegation was performed. Background/permission waits: none.

## 11. Coordinator integration corrections — 2026-10-14

The original D4 handoff was 876 lines, SHA256
`541bdfb7a7fbf435699e82820462c336f73e270cd202bb4f00b807e99177d1fb`.
After recovering idle/pending0, the coordinator corrected three cross-contract rules:
preserve ignored SinglePage response markers; permit the selected action row count
to differ from the count of equivalent witness occurrences; keep the legacy calendar
document hash distinct from the new reconstructed schema's hash. D5 A1/A2/A6 and the
linked existing provider/normalizer support these corrections. No field, signature,
canonical preimage or primary empty-cash golden value changes. The exact revised hash
is frozen in the I1a launch brief. Source review and Rust execution remain separate.
