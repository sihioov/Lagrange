# KIS market stream contract — WS-1

Date: 2026-09-21 KST. Repository baseline: `7757245553ab8cbed5d6f706e46a87849e0cedc6`.
Status: **coordinator adopted for SOURCE/offline implementation; live G1–G5 remain open**.
Adoption 2026-09-21: actual WS-1 idle/report recovered; coordinator reviewed the complete
contract, repository type/publication boundaries, and independently fetched P1 with matching
documentation SHA-256. This accepts the project defaults for implementation and local tests,
not measured performance, current broker capacity, or credentialed operation. Numeric budgets
remain subject to the explicit test/review gates; unexpected contradictions return to review.
This document does not authorize an approval-key request, broker connection, activation,
credential change, or operational-state inspection. It does not change `AGENTS.md`.
The [transition plan](../plans/2026-09-21-kis-market-stream-transition.md), including its
preservation rules and coordinator gates, remains binding.

Labels used below: **E** = observed official documentation/source or baseline repository;
**D** = proposed project decision, frozen for downstream implementation after coordinator
adoption; **G** = unresolved live gate. A project budget is not a broker quota or guarantee.

## 1. Findings that determine the implementation

- **E:** The current portal has **47** `H0STCNT0` response fields. The September 9 notice
  introduces field 47, `MARKET_CLS_CODE`, effective September 14. The August GitHub sample
  still has 46. Implement the current portal schema; reject the obsolete shape explicitly.
- **E:** Subscribe is `tr_type="1"`; unsubscribe is **`"2"`**. The portal and executable
  helper agree. The market function's `"0"` unsubscribe docstring is stale.
- **E:** `BSOP_DATE`, `STCK_CNTG_HOUR`, and `TRHT_YN` exist. A trade date and observed
  halt flag can therefore be represented. `STCK_SDPR` does not exist in this channel;
  neither `VI_STND_PRC` nor price minus change is its replacement.
- **E:** The latest fully retrieved throughput notice, labelled 2026-04-20, says one WS
  session per account/App Key and 41 combined registrations. Thirty distinct symbols on
  one market trade channel fit that stated allowance. The sample's 40 check counts
  function-map keys rather than symbol registrations and is not a reliable quota checker.
- **G:** The live portal list additionally contains a **2026-09-16 throughput notice**.
  Its detail returned HTTP 400 on bounded public-document retrieval attempts. Thus 41 and
  the April REST values are verified statements of the April notice, **not a verified
  assertion that no later restriction applies**. Resolve this before live activation.
- **E:** All four production read-client constructor sites opt into the same coordinator
  in `shared_required` mode. It already enforces **1 second globally and per channel**
  across market GETs. There is no reason to replace that ledger or loosen its settings.
- **D:** New stream tables and types preserve migration 0054 and the old REST contract.
  New board leases contain up to 30 identities. The acquisition process owns one socket,
  keeps bounded private latest values, and publishes through PostgreSQL before API/SSE.
- **D:** No startup REST snapshot, recurring REST quote polling, fallback REST, replay
  service, extra keys, NXT/unified channel, order channel, scanner, or strategy engine.

## 2. Evidence register and authority order

All official sources below were checked on 2026-09-21 KST. GitHub `main` resolved through
the public commits API to **`b4e6249714418aa57833d1cbbbced39cbcc5b125`**, committed
`2026-08-26T06:39:24Z`. Links below pin that revision rather than mutable `main`.
Public portal JSON was read as documentation, without authentication or API test-bed use.
No downloaded sample was executed or copied wholesale.

| ID | Exact source/location | What it establishes |
|---|---|---|
| P1 | [Market trade documentation](https://apiportal.koreainvestment.com/apiservice-apiservice?/tryitout/H0STCNT0), public detail `accessUrl=/tryitout/H0STCNT0`, ID `714d1437-8f62-43db-a73c-cf509d3f6aa7`; [field document](https://apiportal.koreainvestment.com/api/apis/guide/property/714d1437-8f62-43db-a73c-cf509d3f6aa7) | Domains, request headers, numeric direction codes, record packing, ACK example, ordered fields. Detail timestamp is 2025-04-30; field records were modified **2026-09-11T16:31:27+09:00**. |
| P2 | [Approval documentation](https://apiportal.koreainvestment.com/apiservice-apiservice?/oauth2/Approval), ID `5c87ba63-740a-4166-93ac-803510bb9c02`; [field document](https://apiportal.koreainvestment.com/api/apis/guide/property/5c87ba63-740a-4166-93ac-803510bb9c02) | Exact authentication body and 24-hour connection-key validity; an authenticated uninterrupted session does not require periodic key reissuance. Detail modified 2023-06-27; fields 2024-12-13. |
| P3 | [Throughput notice, 2026-04-20 wording](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/d0d1a83f-6f8d-4437-9700-6d26702fd989) | Published 2023-01-11, subsequently revised: live REST 18/sec, mock 1/sec, WS 1 session/App Key, 41 registrations summed across products/channels, approval issuance 1/sec. No license to add keys for this project. |
| P4 | [New-customer notice](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/c1113824-17c7-47a7-b7b8-8880506a847c) | Published 2026-03-20; from April 3, new live API applicants limited to 3 calls/sec for three days; mock excluded. Actual credential cohort was not inspected. |
| P5 | [KRX/NXT September change notice](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/26dfe350-eb72-48e5-8175-34eb27970f3e) | Published 2026-09-09, effective September 14; adds `MARKET_CLS_CODE` to H0STCNT0. Changes to other channels/order types in the same notice are outside scope. |
| P6 | [Latest public notice list](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/), live JSON `GET /api/forums/10000000-0000-0011-0000-000000000001/posts?size=10&sort=createdDate,desc` | Lists `2b641ee8-b594-427a-9d11-ee6e764f0453`, `[중요] API 유량제한 적용 안내`, created 2026-09-16T17:43:45+09:00. Detail unavailable in this research; substantive limits **unknown**. Search-index pages were older than this live list. |
| P7 | [KIS partnership and data-use guidance](https://apiportal.koreainvestment.com/provider), “제휴 대상이 아닌 고객” / “시세 API” | Personal own-asset use is distinct from third-party services. Partner display can require exchange information-use agreements. Does not by itself certify this deployment or an indefinite cache-retention right. |
| S1 | [Market function](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/domestic_stock/domestic_stock_functions_ws.py#L296-L362) | `ccnl_krx`, H0STCNT0, six-digit example, obsolete 46-field list and unsubscribe docstring. |
| S2 | [Official helper](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/kis_auth.py#L463-L589) and [socket loop](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/kis_auth.py#L665-L784) | Approval request at 475–498; JSON envelope 513–534; ACK/UNSUB classifier 538–589; PINGPONG→Pong 698–700; 40-map-key check 709–711; unsubscribe `2` at 778–784. Secret/raw logging and AES helpers must not be imported. |
| S3 | [Public example configuration](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/kis_devlp.yaml#L26-L30), [socket construction](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/domestic_stock/domestic_stock_examples_ws.py#L19-L20) | Live WS port 21000; `/tryitout` socket path. The portal's `/tryitout/H0STCNT0` identifies an API document; it is not the socket upgrade path. |

Hashes of raw **public documentation** property responses (not broker payloads):
P1 `ea945675204c6394b614124012995daaad5080eec7a62339f72c9aa613d6df68`;
P2 `a2f5272fe9eb3b86c99900e2ccf0bf3fea8988b8cc70b5ff8d3a800705a4c7ec`.
These identify retrieved documentation bytes; no public administration metadata, example
credentials, or provider response body is included in this artifact.

Authority: newer dated portal change + current field document, then matching pinned official
source. Record disagreements explicitly; never combine layouts by padding/truncating. The
checked-in August XLSX is historical corroboration, not authority over the September change.
The exact numeric command throughput, reconnect quota, heartbeat cadence, ACK deadline,
maximum server message size/record count, and duplicate-delivery guarantees were **not found**.

## 3. Literal network and authentication proposal

The following is a **D/G approval proposal** for the coordinator. It grants no live permission.

| Operation | Exact proposed allowed surface | Authentication / constraints |
|---|---|---|
| Approval acquisition | `POST https://openapi.koreainvestment.com:9443/oauth2/Approval` | Case-sensitive `Approval`; JSON body only `grant_type="client_credentials"`, `appkey`, `secretkey` (the existing App Secret). `Content-Type: application/json; charset=utf-8`. No REST bearer token or account identifiers. |
| Market socket | WebSocket HTTP `GET` upgrade to **`ws://ops.koreainvestment.com:21000/tryitout`** | No query, fragment, user-info, arbitrary path, redirect, or configurable proxy destination. Only the frame contract below. |
| Subscribe / unsubscribe | On that socket only, `H0STCNT0`, exact approved six-digit short codes | `approval_key`, `custtype="P"`, `tr_type="1"` or `"2"`, `content-type="utf-8"`. |
| Liveness / close | Same established socket | Respond to control Ping/Pong/Close and documented application `PINGPONG`. No HTTP polling heartbeat. |

**E:** The documented live socket uses plain `ws`, not `wss`; market messages and the
connection approval key travel without TLS on that leg. **G:** Coordinator must obtain exact
transport acceptance from the owner, or return for an officially documented encrypted
alternative. Do not invent `wss:21000`, move ports, tunnel through an unapproved third party,
or silently call the mock host. Browser access to KIS is never part of this proposal.
Mock production endpoints, H0STCNI0/H0STCNI9, HTS-ID subscriptions, asking-price/expected-price
TRs, NXT/unified TRs, and every account/order surface are rejected before sending bytes.

Approval success is HTTP 200 with one nonempty bounded string `approval_key`; duplicate JSON
keys, malformed JSON, missing/wrong-type values, redirects, or unknown success shape fail
closed. Keep only typed errors (`APPROVAL_HTTP_REJECTED`, `APPROVAL_RESPONSE_INVALID`,
`APPROVAL_OUTCOME_UNKNOWN`, `APPROVAL_EXPIRED`); never echo body, URL credentials, or prose.
No success/expiry field is fabricated from REST token fields.

**D:** Separate market approval state from REST `state-v1.json`. Resolve the existing secret
references once per explicit credential generation. Protected WS state records issuance
start, safe expiry (issuance start + 24 hours), key generation, attempt counters and ambiguous
outcome. Reuse a valid key for reconnects; require at least 60 seconds of remaining validity
before a new socket attempt. A live authenticated socket is not rotated just because its key
ages. No refresh timer. Credential/key state is not exposed in SQL, API, telemetry or report.
Persist the key only in the dedicated protected WS state (0700 directory, 0600 regular
single-link files, atomic durable update, no symlinks); redact `Debug`, HTTP and WS errors.

**D:** Reserve approval attempts durably before dispatch, minimum 60 seconds apart, maximum
two attempts in a rolling 24 hours; this is deliberately stricter than P3's 1/sec. Timeout
after potential dispatch is indeterminate: no automatic reissue. Explicit authentication
rejection with an unexpired key stops the session for operator review; no guessing from
free-form messages. Expired/missing keys may be issued only within an already authorized
activation and valid scope/day evidence. Restart or midnight does not reset counters.
The shared REST TokenManager and its existing issuance/debt rules remain untouched.

## 4. Wire contract `kis-h0stcnt0-20260914-v1`

### 4.1 Outbound messages and acknowledgements

The synthetic envelope is descriptive, not executable sample code:

```json
{"header":{"approval_key":"SYNTHETIC_ONLY","custtype":"P","tr_type":"1","content-type":"utf-8"},"body":{"input":{"tr_id":"H0STCNT0","tr_key":"005930"}}}
```

Unsubscribe changes only `tr_type` to `"2"`. Never send `"0"`. Approval is a WS JSON header
field, not an HTTP Authorization header. No account, order, REST token, or `appsecret` field
is admitted in a WS message.

**D:** Process one pending command globally at a time. A command stores socket epoch,
operation, symbol, local command ordinal, send timestamp and 5-second deadline. A usable
ACK has `header.tr_id=H0STCNT0`, matching `header.tr_key`, `encrypt="N"`, and body `rt_cd="0"`,
`msg_cd="OPSP0000"`. For subscribe, P1 documents `msg1="SUBSCRIBE SUCCESS"`. For unsubscribe,
S2's success classifier is the prefix `UNSUB`; use that bounded in-memory classifier with
the outstanding unsubscribe, not a fabricated numeric unsubscribe success code. Do not
persist/log `msg1`, `output.key`, or `output.iv`; the latter are not needed for plaintext
market frames. Unknown ACK fields may be ignored after bounded parsing, but duplicate known
keys, mismatched identity, contradictory operation, encryption, or invalid types are errors.

Nonzero `rt_cd`, missing ACK, unexpected successful ACK, already-subscribed rejection, or
duplicate ACK without a pending command must not mark a subscription usable. A rejected
subscribe yields `SUBSCRIPTION_REJECTED`; ambiguous ACK/timeout closes the epoch and stops
publication. Do not resend a pending command on the same socket. There is no wire request ID
or verified ACK sequence; an implementation cannot prove arbitrary delayed duplicate ACKs
belong to a later identical operation. **D:** after unsubscribing a symbol, do not resubscribe
that symbol in that epoch: controlled whole-socket reconnect under the connection budget
is required. Coalescing and the unsubscribe grace make this uncommon. Record the resulting
gap for every affected symbol; no quiet automatic override of the budget.

### 4.2 Data, heartbeat and framing

**E/P1:** A complete text message is `0|H0STCNT0|NNN|record[^record...]`. `NNN` is three ASCII
digits; `^` separates every field across all packed records. It is a pushed record count,
not REST pagination. Accept exactly `47 × NNN` fields, preserving empty fields. No trimming
away a terminal empty field, newline concatenation, partial record publication, or count
repair. Reject prefix `1`, binary data messages, other TRs and malformed UTF-8. Socket-level
fragmentation must be reassembled by the WS transport under the message bound before parsing.

**D (2026-09-21 bounded-work clarification):** A fragmented text message may span at most
64 received frames, counting its initial and final data frames, empty continuations, and
interleaved control frames until completion. Persist this count across partial reads and
returned control messages; both blocking and nonblocking paths use the same counter. The
65th frame fails closed and poisons the epoch before any queued receipt is delivered, without
clearing pending-command ambiguity. In addition, one synchronous available-byte drain may
perform at most 64 successful socket reads; exhausting that budget without reaching a complete
message or WouldBlock fails closed. Existing byte, complete-message, queue and ingress bounds
remain cumulative constraints. These are conservative local SOURCE limits, not documented KIS
fragmentation guarantees; only a separately approved pilot can establish compatibility.

**D:** Validate the entire message before updating any private latest-value slot. Discard
non-admitted/stale observations with typed reasons; a malformed record invalidates its whole
message. A data message received before its symbol's subscribe ACK is not publication evidence.
No AES implementation is needed or permitted through the account sample.

**E/S2:** JSON with `header.tr_id="PINGPONG"` and no body is an application heartbeat, answered
by a WebSocket **Pong control frame carrying the original UTF-8 heartbeat bytes**, not a new
price or subscription. RFC control-frame payload bounds still apply (125 bytes); an oversized
heartbeat is a protocol failure, not a reason to emit an invalid control frame. Incoming
control Ping receives Pong; Close is acknowledged. No heartbeat advances quote counters.
**D:** Send a control Ping after 20 seconds without control-liveness confirmation; require its
matching Pong within 10 seconds, close otherwise. Also close on 90 seconds without any inbound
frame. These are local watchdogs, not documented KIS heartbeat promises. Broker rejection of
this policy in a permitted pilot is a contract issue, not permission to invent another protocol.

### 4.3 Ordered fields and normalization

The exact one-based sequence is:

```text
01 MKSC_SHRN_ISCD                 02 STCK_CNTG_HOUR
03 STCK_PRPR                      04 PRDY_VRSS_SIGN
05 PRDY_VRSS                      06 PRDY_CTRT
07 WGHN_AVRG_STCK_PRC              08 STCK_OPRC
09 STCK_HGPR                      10 STCK_LWPR
11 ASKP1                          12 BIDP1
13 CNTG_VOL                       14 ACML_VOL
15 ACML_TR_PBMN                   16 SELN_CNTG_CSNU
17 SHNU_CNTG_CSNU                 18 NTBY_CNTG_CSNU
19 CTTR                           20 SELN_CNTG_SMTN
21 SHNU_CNTG_SMTN                 22 CCLD_DVSN
23 SHNU_RATE                      24 PRDY_VOL_VRSS_ACML_VOL_RATE
25 OPRC_HOUR                      26 OPRC_VRSS_PRPR_SIGN
27 OPRC_VRSS_PRPR                  28 HGPR_HOUR
29 HGPR_VRSS_PRPR_SIGN             30 HGPR_VRSS_PRPR
31 LWPR_HOUR                      32 LWPR_VRSS_PRPR_SIGN
33 LWPR_VRSS_PRPR                  34 BSOP_DATE
35 NEW_MKOP_CLS_CODE               36 TRHT_YN
37 ASKP_RSQN1                     38 BIDP_RSQN1
39 TOTAL_ASKP_RSQN                 40 TOTAL_BIDP_RSQN
41 VOL_TNRT                       42 PRDY_SMNS_HOUR_ACML_VOL
43 PRDY_SMNS_HOUR_ACML_VOL_RATE    44 HOUR_CLS_CODE
45 MRKT_TRTM_CLS_CODE              46 VI_STND_PRC
47 MARKET_CLS_CODE
```

The names/order are protocol identifiers from P1/P5. Do not use the obsolete S1 list as the
fixture oracle. Schema mismatch (including exactly 46 fields) is `WIRE_SCHEMA_MISMATCH`.

| Field → output | Validation / meaning |
|---|---|
| 1 → symbol | Exact six ASCII digits, matching an ACKed approved KRX identity. The portal's wider ETN possibility is excluded. |
| 2, 34 → provider date/time | Parse valid `HHMMSS` and `YYYYMMDD`; carry original business date and second-resolution time. Interpret with the approved KRX `Asia/Seoul` session contract; timezone is supplied by our venue contract, not a field on the wire. |
| 3 → price | Positive finite canonical decimal, fit SQL `numeric(20,8)`; zero/empty is not a usable price and never becomes a placeholder quote. |
| 4 → direction | `1=LIMIT_UP`, `2=UP`, `3=FLAT`, `4=LIMIT_DOWN`, `5=DOWN`. |
| 5, 6 → change amount/percent | Preserve signed decimal meaning. Reject direction contradictions; allow a rounded zero percentage with nonzero signed price change. Do not derive a sign twice or recompute percent. |
| 13, 14 → trade/cumulative volume | Nonnegative decimal integer strings, fit positive signed bigint range including zero. No account position semantics. |
| 36 → observed halted | Required `Y` or `N`, mapped to boolean only for this observation. Unlike REST, no status-58/temp-stop union is performed. No observation means unknown; quiet trading does not imply halted. |
| 35, 44, 47 → market eligibility | Only regular in-window observations are quote candidates: `NEW_MKOP_CLS_CODE="20"`, `HOUR_CLS_CODE="0"`, `MARKET_CLS_CODE="2"`. Other documented values produce non-regular status and no new quote. This deliberately excludes after-market, expected/VI prices and separately coded closing trades. |
| 45 | May be blank, as in P1's own example. Its undocumented code values are not mapped to open/closed/halted. Nonblank yields `MARKET_CLASS_UNSUPPORTED` pending a documented interpretation. |
| no field → base price | `null`, reason `NOT_PROVIDED_BY_CHANNEL`. Never use field 46, a derived previous close, an old REST quote or zero. |

Unconsumed fields remain bounded text, are not persisted or exposed, and cannot influence
price/session/rights decisions. Numeric-looking auxiliary fields are not an excuse to add
unapproved analytics. Consumed required fields missing/blank/wrong-type fail closed; nullable
application fields do not turn a malformed required wire field into success. Decimal lexical
validation forbids exponent notation, NaN/Infinity and unbounded precision. Validate auxiliary
text length/encoding and exact positional structure without pretending to know every code.

**D:** Accept a receipt only within the current approved half-open regular session window,
with `BSOP_DATE` equal to that session date and the receipt's KST date. Reject a future
provider time more than 2 seconds beyond receipt time (clock-tolerance policy), receipt time
after DB time, previous-date frames, and event times outside the permitted window. Event time
is not a sequence; same-second events are legal. Within a symbol/epoch discard regressing
event time or cumulative volume. Equal tuples can be duplicate delivery; suppress exact
duplicate normalized observations without claiming exactly-once trades. Preserve local receive
ordinals for accepted distinct observations, including equal prices with newer times/volume.

On a new epoch, an initial old trade can be last-known, but cannot establish fresh live state:
require an ACKed receipt whose event time is at least the socket-open second and age <=30s.
There is no documented initial snapshot/replay SLA. An ACK without a trade leaves
`AWAITING_FIRST_TRADE`. Same-day gaps remain explicitly incomplete even after fresh data resumes.

## 5. Market identity, rights, demand and exclusive acquisition

**D:** Keep these identities separate:

1. Market key: `(provider=kis, environment=live, venue=KRX, tr_id=H0STCNT0, symbol)`.
2. Acquisition scope: `(credential_slot_id, credential_generation, endpoint_contract_id,
   owner_user_id, rights_grant_id)`. `credential_slot_id` is an operator-assigned opaque UUID;
   never a public hash of the secret. One slot must identify one existing App Key, with no
   aliases to manufacture extra sessions. Runtime secret binding stays in protected WS state.
3. Publication identity: existing `(owner_user_id, membership_id, generation_id,
   instrument_id, generation)` plus current rights grant/session proof/socket epoch/fence.
4. Consumer: a server lease bound to the authenticated **session**, owner, browser-tab
   `consumer_id` and requested identity set. A consumer ID is an idempotency namespace, not
   an authorization credential or an upstream subscription.

The configured slot has one personal Owner grant. Same-symbol board/detail/tabs share one
upstream registration only inside that scope. No cross-owner fanout, cross-credential merging,
or reuse of an old entitlement hash on a new generation. A future read-only analysis consumer
would use its own authorized lease kind; this release implements only `BROWSER` consumers.
All-tick consumers would need a separate contract; the interface here intentionally coalesces.

### Rights gate

**E:** [ADR-0005](../../decisions/0005-kis-personal-use-entitlement.md) settles existing private
single-owner personal use. Do not reopen that general decision. P7 supplies surrounding source
guidance, not a new blanket license. The old attestation preserves the allowlist; WS is a new
surface. **G:** Coordinator must bind the narrowly approved WS surface, ephemeral latest-value
cache and Owner-only SSE to the existing entitlement reference/document hash, or record the
specific narrow amendment if needed. No fresh private-use attestation is demanded merely
because this is WS. No assumption about other Owners, invited members or indefinite retention.

**D:** New DB `owner_market_stream_grants` binds exactly one owner/slot to an existing
`data_entitlements.id`, its exact reference/document SHA-256, `H0STCNT0`, this wire contract,
approved network-contract SHA-256, allowed exact30 list hash, effective dates, activation
commit, and state `ACTIVE|REVOKED`. It is initially empty. Only the coordinator's later
approved, commit-pinned grant-install command can provision a grant from a reviewed approval input; app/worker
cannot insert/activate it or mutate `data_entitlements`. No `ACTIVE` default or migration seed.
Existing entitlement amendments/history are retained. Grant revocation and entitlement
revocation each independently prevent reads/publication; do not just check READY.
The existing `deploy-production-release.sh:48` explicitly performs no database commands and
keeps that property. Installing a release does not install a grant or activate a socket.

### Demand sets

**D:** `BROWSER` lease TTL 30 seconds, explicit renewal every 15 seconds. One lease contains
1–30 exact READY identities; board requests **all 30**, detail requests one, and a detail on
the same board tab adds no second lease. Up to 20 active leases per Owner (600 lease-item rows),
10 browser tabs are a required proof case. Global unique desired symbols for this slot <=30.
Admission capacity and subscription capacity are separate. Reject an over-capacity mutation
atomically; never silently truncate. Existing REST's 5-identity/20-single-consumer contract stays
unchanged and cannot supply WS demand.

PUT-like set replacement uses the POST route in section 8: `(consumer_id, renewal_sequence,
canonical sorted set)` idempotency, same sequence/same hash replays, same sequence/different
hash is 409, lower sequence is 409, released lease cannot resurrect. Bind the lease to a
canonical session hash (internal only). Another session cannot renew/release it. Expired
lease requires a new consumer ID. Server creates lease UUID; no owner/session ID in bodies.
Use the existing owner policy row as anchor plus a dedicated transaction advisory capacity
lock, not an unauthorized `FOR UPDATE` on a SELECT-only policy table.

Compute desired as the union of unexpired, nonreleased, session-valid leases whose current
admissions/grant remain valid. Reconcile at least once/second, merging changes for 250ms.
State per market key: `DESIRED → PENDING_SUBSCRIBE → ACKED → PENDING_UNSUBSCRIBE → ABSENT`,
plus `REJECTED`/`AMBIGUOUS`. Pending and ACKed registrations count against capacity. An
unsubscribe continues to occupy a slot until its ACK or definite socket closure. Data without
ACK is never published. Last consumer removal suppresses new publication immediately; allow
5 seconds of upstream unsubscribe grace for ordinary tab churn. Revocation has no grace for
read/publication and requests unsubscribe immediately under the command budget. If the whole
desired set becomes empty, close the socket within 5 seconds (no need to wait for 30 separate
unsubscribe ACKs). Lost browser cleanup expires within 30s + 1s reconcile + 5s close.

### One process may own the broker socket

**D (2026-09-21 reviewed amendment):** Supported deployment is one authorized Linux host,
including multiple local worker processes. The installer provisions two distinct persistent
lock anchors, `connection.lock` and `state.lock`, as zero-length root:GID-10001 regular
single-link mode-0440 files under a separate root:GID-10001 mode-0750 directory whose
ancestors are not writable by runtime UID/GID 10001. The Owner runner receives that directory
as a dedicated read-only bind mount. Runtime opens each existing anchor read-only with
no-follow, close-on-exec descriptor-relative operations, verifies UID/GID/mode/type/link/size
and stable distinct `(dev,ino)` identity, and takes an exclusive nonblocking `flock`. Each
acquisition uses a fresh open file description; sharing or duplicating an already-open lock
file between contenders is forbidden. Runtime never creates, repairs, chmods, unlinks,
renames, or replaces an anchor. Acquire the connection anchor, then the DB producer lease
keyed by `credential_slot_id` and endpoint contract, then approval/socket; hold the anchor
until the socket task is closed/joined. Use the separate state anchor around every durable
WS-state transaction. Never acquire the REST coordination lock for the socket lifetime.

All contenders use the same host inodes on a persistent local Linux filesystem: no tmpfs
anchors, per-container copies/copy-up, network-filesystem assumptions, or cross-host failover.
Host ancestors and container bind-target parents are nonreplaceable by runtime UID/GID 10001.
The contract assumes trusted host root and cooperating runtime processes; it does not claim
integrity against a malicious UID-10001 process that ignores the lock and rewrites writable
state. This limitation permits no bypass of locking, ambiguity, budgets, or inode checks.

DB lease TTL 20 seconds, renewal every 5 seconds, strictly increasing bigint fence; socket
epoch is a fresh UUID on each connection and is committed before subscription commands.
The worker closes on failed lease renewal/DB loss and refuses publication with <=5s of lease
life remaining. A new process cannot open while the old process holds the lifetime lock,
even if the DB lease has expired. Process crash releases the OS descriptor/socket; an uncertain
broker-side old session or duplicate-session response stops reconnection for review.
Cross-host failover is **not supported**: a DB TTL alone cannot prove a partitioned process
closed its broker socket. No lease-stealing or additional-host workaround is authorized.

Reconnect invalidates every ACK and private live value, creates a gap/new epoch, and rebuilds
desired subscriptions from DB. Reconnect attempts use minimum 10s spacing, exponential
10/20/40/60s backoff with bounded 0–20% positive jitter, maximum 6 attempts/10min and 20/day.
These are durable project budgets shared by all local contenders, not documented KIS limits.
Backoff exhaustion is a visible stopped state; a tab refresh cannot reset it. Reconnect cannot
change session proof, issue calendar REST calls, clear Sep19 debt, or renew credentials blindly.

## 6. PostgreSQL and typed interfaces

### 6.1 Baseline evidence and reuse boundary

| Existing code at baseline | Consequence |
|---|---|
| `crates/market-data/src/intraday_quotes.rs:36` | REST `IntradayQuote` requires `base_price` and a REST-specific halted calculation; introduce a separate stream type. |
| `crates/job-queue/src/owner_equity_v2/intraday.rs:21`, `:246`, `:508`, `:722`, `:1929` | Five identities; one identity/consumer; publication requires `IntradayAttemptReservation`. Do not dummy-fill or relax it. |
| `migrations/0054_owner_intraday_quotes.up.sql:11`, `:73`, `:146`, `:198`, `:332` | Existing demand unique `(owner,consumer)`, quote payload all-or-none, producer keyed by owner, READY/generation RLS. New tables avoid mutating applied SQL. |
| `crates/job-queue/src/owner_equity_v2/intraday.rs:2715`, `:2863`, `:2963` | Useful policy/capacity mutex, admission/session locking patterns; existing runtime uses `worker`, not `research_writer`, for intraday writes. |
| `migrations/0046_candidate_price_rights_revalidation.up.sql:95` | Existing Boolean predicate checks EOD dataset/use coverage; it is not sufficient authorization for a new WS source. |
| `crates/api-server/src/http/session.rs:120`, `:189`; `http/mod.rs:344` | Canonical admin-backed session resolution, app actor context and CSRF already exist; extend, do not bypass. |
| `crates/api-server/src/http/owner_intraday_quotes.rs:135`; `apps/web/lib/products/intraday-quotes-contracts.ts:1` | REST DTO has mandatory base price and polling contract; WS gets its own strict versioned DTO. |
| `crates/kis-client/src/websocket.rs:1` | Account resync state machine requires order/execution/balance reconciliation. None of it is market-client recovery. |

### 6.2 Migration reservation and tables

**D:** Reserve the currently next-free pair
`migrations/0055_owner_market_stream.up.sql` / `0055_owner_market_stream.down.sql` for WS-3.
At this HEAD only 0001–0054 exist. Recheck before writing; if occupied, return to coordinator
to renumber the contract/brief together. This is not the previously withdrawn audit migration.
Never edit 0054 or recreate that old audit proposal. No DB downgrade as routine rollback.

New tables, all owned by `migration_owner`, ENABLE + FORCE RLS:

| Table | Keys and required content |
|---|---|
| `owner_market_stream_grants` | `id UUID PK`; unique active `credential_slot_id`; positive `credential_generation`; owner FK; exact entitlement binding and approval hashes described above; `effective_from/until`, `state`, immutable grant revision UUID. No secret or account identifier. |
| `owner_market_stream_leases` | `id UUID PK`, unique `(owner_user_id,consumer_id)`; `session_hash` internal FK to canonical session hash, kind BROWSER, sequence bigint, state, expiry, request/idempotency hashes and timestamps. |
| `owner_market_stream_lease_items` | PK `(lease_id,membership_id)`; owner + full admission composite FK exactly matching migration 0053; at most 30 per lease in the repository transaction. |
| `owner_market_stream_producers` | PK `credential_slot_id`; grant ID/revision, owner, holder UUID, monotonic fence bigint, current epoch UUID nullable, session date/proof IDs/hashes, lease expiry/heartbeat, connection state, gap generation, state_version bigint. |
| `owner_market_stream_subscriptions` | PK `(credential_slot_id,symbol)`; full market key, epoch, state, pending operation/ordinal/deadline, ACK time, desired reference count, subscription revision UUID. Contains no prices. |
| `owner_market_stream_cache` | PK `(owner_user_id,membership_id)`; `row_generation uuid NOT NULL`; full admission FK; grant/revision, source/wire version, session proof/date, epoch/fence/subscription revision, receive ordinal, nullable normalized quote, quote_version bigint, state_version bigint, typed failure/status, gap_since, gap_generation, received_at, committed_at. |

`quote_version` increments only for a new committed valid quote and never for ACK, heartbeat,
status, cache read or merely equal-price repaint. `state_version` increments for any committed
visible quote/status transition. Both are positive bounded bigint with checked exhaustion.
Do not recycle them within a row. On GC/recreation, a new row-generation UUID distinguishes
the version namespace; include it in API identity. REST and stream versions are not comparable.

Store decimals as `numeric(20,8)`; volume as nonnegative bigint. SQL CHECKs bind
`source='KIS_MARKET_WS'`, `wire_version='kis-h0stcnt0-20260914-v1'`, `venue='KRX'`, six-digit
symbol, positive generation/fence/receive ordinal, coherent required receipt/quote fields,
`base_price IS NULL`, exact direction set, date/timestamp consistency, and typed enums. A
no-quote cache row may carry status but none of the price/volume/halt/event fields. Last-known
quote fields retain their original epoch, receipt and versions when producer state changes.
Do not replace those with the current socket's identity before a new observation commits.
Cache retention is at most 24h, latest value only; no raw-frame or tick-history table.
All IDs/revisions/epochs are UUIDs, times are `timestamptz`, dates are `date`, state/source/
contract tags and hashes are CHECK-constrained text. Credential generation retains the
existing unsigned-u64 domain as canonical decimal text (it is not a membership bigint).
Receipt-less cache rows have quote_version zero; quoted rows have positive quote_version.
Producer states are `DISCONNECTED|CONNECTING|CONNECTED|BACKOFF|STOPPED`; subscription states
are `DESIRED|PENDING_SUBSCRIBE|ACKED|PENDING_UNSUBSCRIBE|ABSENT|REJECTED|AMBIGUOUS`.
Grant payload and grant revision are immutable; only one-way ACTIVE→REVOKED is allowed.
Grant replacement is a new UUID after revocation, never an edit to a prior approval.

### 6.3 Rights and role enforcement

`app` gets actor-scoped lease CRUD through column grants, item mutations, and SELECT on
authorized cache/status projections. It cannot write quote/subscription/producer/grant rows.
`worker` gets the precise new lease/status/cache DML columns and read-only grant visibility;
it cannot provision grants, alter entitlements, or mutate admissions. Existing `research_writer`
EOD grants and `admin` authentication-only purpose do not expand to general stream writes.

WS-3 adds two narrow SECURITY DEFINER helpers owned by `migration_owner`, fixed
`search_path=pg_catalog`, schema-qualified names, PUBLIC EXECUTE revoked:

- `owner_market_stream_rights_valid(grant_id uuid, owner_id uuid, session_date date) -> boolean`:
  validates grant state/window, owner role, exact entitlement ID/reference/document hash,
  ACTIVE entitlement and date coverage, current reviewed stream capability binding and list
  hash. Does not treat an EOD predicate or any ACTIVE entitlement as enough. Called from RLS
  and publish/read transactions; grants EXECUTE only to app/worker.
- `owner_market_stream_session_valid(session_hash text, owner_id uuid) -> boolean`: checks
  nonrevoked/unexpired web session and current Owner role for that exact owner. No session
  token returned and no broad worker SELECT grant on session/entitlement tables.

**D — 2026-09-22 C1 correction contract:** Nonlocking RLS predicates stay separate
from locking capabilities. Every lock helper is `VOLATILE SECURITY DEFINER`, owned by
`migration_owner`, with `SET search_path=pg_catalog`, schema-qualified static SQL, FORCE RLS
retained, PUBLIC/non-target EXECUTE revoked, and an explicit exact `session_user` check.
Serving processes and tests connect directly as the named role; SET ROLE from a different
login is not a substitute. Reject NULL/nil/malformed or mismatched identity arguments before
locking. Helpers never mutate business rows or expose arbitrary SQL. Restore every temporary
actor/session lookup GUC on normal return, including false/zero results; exceptions abort the
transaction. Do not add broad table/column UPDATE grants to enable locking.

The C1 locking capability signatures and serving-role boundaries are:

- `lock_owner_market_stream_rights(slot_id, grant_id, grant_revision, owner_id,
  session_date) -> boolean`, worker only: validate and lock exactly the ACTIVE immutable
  grant and matching ACTIVE entitlement FOR SHARE, including slot/revision/owner, exact
  entitlement ID/reference/document hash, coverage, Owner role, fixed TR/wire/list binding.
  Worker claim may discover grant identity with its existing SELECT grant, but cannot directly
  FOR SHARE the grant; the helper locks/revalidates that exact identity before mutation.
- `lock_owner_market_stream_session(slot_id, grant_id, grant_revision, owner_id,
  session_hash) -> boolean`, worker only: verify the exact active grant binding without
  relocking grant rows out of order, and lock the exact owner's current nonrevoked/unexpired
  session and Owner-role row. App receives neither worker lock capability.
- `lock_owner_market_stream_app_admission(owner_id, session_hash, membership_id,
  instrument_id, generation) -> exact admission row`, app only: require exact actor and
  market-stream-session GUCs and valid Owner session; lock one current READY membership and
  latest matching admission. Empty result fails typed before DML.
- `lock_owner_market_stream_worker_admission(slot_id, grant_id, grant_revision, owner_id,
  membership_id, generation_id, instrument_id, generation) -> boolean`, worker only:
  verify the exact active grant and full latest READY admission identity, then lock it.
  Admission helpers are invoked in membership UUID order. Because migration_owner is
  NOSUPERUSER/NOBYPASSRLS and 0053 owner policies require actor context, temporarily set only
  the already-authorized owner context and restore it; do not alter applied 0053 policies.
- `lock_owner_market_stream_app_lease_items(owner_id, session_hash, lease_id) -> integer`,
  app only: validate exact actor/session/active owned lease already locked by the repository;
  lock its existing items in membership UUID order. Return 1..30 for a valid existing set;
  zero/missing set or authorization mismatch is failure and repository stops before DML.
- `lock_owner_market_stream_worker_demand(slot_id, grant_id, grant_revision, owner_id,
  membership_ids[]) -> exact active lease/item rows`, worker only: accept 1..30 distinct,
  nonnull, nonnil owned current membership IDs, verify exact active grant binding, and lock
  only active/unexpired relevant leases/items in `(session_hash, lease_id, membership_id)`
  order. Invalid arrays/identities return no rows without useful target locks. Repository
  requires live demand for every requested full admission identity after locking.

A helper's signature does not itself prove that earlier locks are held: all public repository
paths must use the canonical transaction sequence and fresh post-lock checks below. Test wrong
roles, cross-owner inputs, altered slot/grant/revision/full identity, zero rows, restored GUCs,
RLS recursion and bounded concurrency using actual direct-login roles. Down migration must
remove every new helper in dependency-safe order. C1 changes only unpublished migration 0055,
stream repository and focused DB tests. The ACK provenance gap remains separately blocked;
C1 does not accept fabricated ACKs or authorize reopening frozen WS-2.

No caller can select another owner's quote by passing an owner UUID: RLS separately requires
`app.actor_user_id` and latest READY admission. Price SELECT also checks current grant/rights;
historical revoked values disappear. Membership/generation changes hide old rows immediately.

### 6.4 Publication transaction and no memory bypass

Freeze repository methods in new `owner_equity_v2/market_stream.rs` (exports via parent):

```text
replace_stream_lease(owner, session_hash, request) -> StreamLease
release_stream_lease(owner, session_hash, lease_id, sequence) -> ReleaseOutcome
read_stream_demand(slot_id) -> DesiredSet
claim_stream_producer(slot_id, holder, grant_revision) -> StreamProducerLease
renew_stream_producer(lease) -> StreamProducerLease
start_stream_epoch(lease, session_proof, epoch) -> EpochProof
record_subscription(lease, epoch, prepared_or_ack_capability) -> opaque SubscriptionProof
publish_stream_latest(context, observations[1..30]) -> CommitResult
record_stream_status(context, typed_status) -> CommitResult
read_stream_snapshot(owner, session_hash, lease_id) -> StreamSnapshot
```

Concrete type boundaries:

```text
kis-client::market_stream::MarketTradeObservation
  symbol, business_date, trade_time, price, change_amount, change_percent,
  direction, trade_volume, cumulative_volume, halted, market class fields
kis-client::market_stream::MarketReceipt
  epoch UUID, receive_ordinal u64, received_at UTC, local monotonic received instant
market-data::market_stream::StreamQuote
  validated KRX observation + provider_trade_at + nullable base_price (always None)
job-queue::owner_equity_v2::StreamPublicationContext
  producer lease + epoch + grant/revision + admission identity + session proof
  + ACKed subscription proof (including revision and ack_at)
```

Receipt constructors and ACK proofs are private to actual transport/state transitions. Existing
receipt test constructors remain test-only; C2 Prepared/ACK capabilities have no factory even
in test builds. Neither a REST receipt nor a caller-created timestamp proves a WS read.
Observation/receipt structures cannot serialize raw frames or secrets.
The pending/ACK repository operation above belongs to the private C3 producer adapter;
public desired-state input is separate. C1's existing scalar event/proof API is not accepted
as ACK provenance and must be removed or made private in C3 before integration acceptance.

Publication lock order: slot producer → exact grant/entitlement → the same owner-capacity
advisory transaction mutex used by lease mutations → exact memberships/admissions in
membership UUID order → active/unexpired demand sessions in hash order → active leases/items
→ subscriptions in symbol order → cache rows. Hold the owner mutex through commit so lease
replacement/release cannot change the session-bearing set between discovery and row locking.
Retained RELEASED/expired leases neither establish demand nor require a valid session;
active demand with a revoked session still fails closed. Demand expiring during lock
acquisition fails the fresh post-lock check. Lease mutations use owner capacity lock → memberships →
leases/items; they never acquire producer/grant update locks after holding leases. Grant
installation/revocation locks grant/entitlement only; epoch operations start with producer.
The worker uses fresh `clock_timestamp()` after locks. Under the same transaction verify
lease holder/fence/expiry, grant revision/rights, latest READY generation, live demand and
canonical session validity, same KST calendar lineage/window, epoch, ACKed subscription,
subscription revision, receive ordinal and source contract. Subscription changes use the same
producer lock. Expired/retired producer and old-socket frames cannot overwrite the cache.

An observation <=3 seconds old at commit is publishable (local project pipeline bound);
older buffered data is dropped with `PIPELINE_LAG`, not retimestamped. A producer may retain
at most one latest observation per identity while DB is slow. On commit failure/unknown
outcome, do not fan out memory; query committed version/epoch/ordinal before any retry. Exact
same epoch/ordinal is an idempotent no-op, never a second quote_version. Atomic batch errors
roll back the entire batch; prune invalid identities and revalidate on a later bounded cycle.

Execute `pg_notify('owner_market_stream_changed','v1')` **inside** each changing transaction;
PostgreSQL delivers it only after commit. Payload contains no quote, symbol, owner, session or
secret. API LISTEN is a wakeup hint, not a data stream or proof. It always queries the RLS cache
again. API processes also reconcile active SSE scopes at least once/second to recover lost
notifications. There is no direct acquisition-memory-to-SSE route.

### 6.4A C2 transport command and ACK capabilities

**D, adopted after independent SOURCE contract review on 2026-09-22.** C1 is separately
accepted. Reopen only the exact C2 transport seam below; C3 storage/producer integration and
full WS-3 acceptance remain separate gates. Existing framing, endpoint, approval, budget,
domain/anchor provenance, durable schema, receipt parsing and N1–N3 bounds stay unchanged.

```rust
pub enum MarketSubscriptionOperation { Subscribe, Unsubscribe }
#[must_use]
pub struct PreparedMarketSubscriptionCommand { /* private fields */ }
#[must_use]
pub struct MarketSubscriptionAck { /* private fields */ }

impl MarketStreamSession {
    pub fn prepare_subscribe(&mut self, symbol: &str)
        -> Result<Option<PreparedMarketSubscriptionCommand>, MarketStreamError>;
    pub fn prepare_unsubscribe(&mut self, symbol: &str)
        -> Result<Option<PreparedMarketSubscriptionCommand>, MarketStreamError>;
    pub async fn send_prepared(&mut self, command: PreparedMarketSubscriptionCommand)
        -> Result<MarketSubscriptionAck, MarketStreamError>;
}
```

Both capabilities have private fields, no public constructor/mutator/conversion, and no
`Default`, `Clone`, `Copy`, `Serialize` or `Deserialize`. They contain neither a credential,
raw frame nor broker prose. Debug is absent or manual metadata-only. The operation enum may
derive `Debug, Clone, Copy, PartialEq, Eq`; it is metadata, not proof. No test-only ACK or
Prepared factory is added: synthetic proof is minted by the actual loopback transport.

Read-only capability getters expose `credential_slot_id() -> Uuid`,
`epoch() -> ConnectionEpoch`, `symbol() -> &str`, `operation() -> MarketSubscriptionOperation`,
`ordinal() -> u64`, `reserved_at_ms() -> i64`, `reserved_at_monotonic() -> Instant`,
`deadline_at_ms() -> i64`, and `deadline_monotonic() -> Instant`. ACK additionally exposes
`ack_received_at_ms() -> i64` and `ack_received_monotonic() -> Instant`. Copied getter values
cannot substitute for a capability. A private per-session instance nonce is also bound to
the token and session phase and has no public getter. The verified slot comes from the same
approval domain used to construct the session, through only a crate-private, read-only
`MarketStreamDomain::credential_slot_id() -> Uuid` getter. No caller-supplied slot, Debug/path
inference, new state accessor or approval change is permitted.

Preparation is synchronous. Perform existing symbol/no-op/capacity/session checks first.
`None` means only an already-subscribed/already-absent no-op and consumes no attempt/ordinal.
For a real command, capture configured wall time and `Instant::now()` back-to-back as one
reservation pair. Checked conversion of the configured timeout to `i64` milliseconds and
checked addition to both origins must succeed before any reservation. Use this same wall
origin for the existing attempt budget and durable command reservation. These two existing
synchronous locked transactions may remain separate; a crash between them conservatively
consumes the attempt and never writes subscription bytes. Returned Prepared proves both
succeeded. The durable schema's `sent_at_ms` retains its name but stores the conservative
reservation origin, not proof of transmission. Do not change the schema.

The fixed wall and monotonic deadlines include C3 DB-pending persistence delay. Neither is
extended at send, read, dequeue, retry or DB commit. Immediately before the guarded write,
after the bounded pre-send scan, both clocks must lie in their half-open reservation interval:
not before their origins and strictly before their deadlines. Clock regression, disagreement,
overflow or expiry fails closed with no subscription bytes or proof and no ambiguity reset.
The existing configured timeout is used; C2 invents no broker quota or timeout.

The session owns one command phase:

```text
Idle -> Prepared -> WriteStarted -> Sent -> AckObserved -> Idle + one opaque ACK
                      any error/uncertainty -> Poisoned (durable pending retained)
```

Only `send_prepared`'s private implementation drives non-idle phases. It consumes the unique
token and verifies its nonce, verified slot, epoch, symbol, operation, ordinal and timing
against that exact session's pending identity. Another session/epoch or a replay writes no
bytes, mints no proof and clears no pending state. Before subscription write, run the existing
bounded available-message decoder: any already-complete ACK while Prepared is unexpected
and poisons the epoch. An incomplete frame or bytes arriving after that finite scan cannot
be predicted. Enter WriteStarted on the guarded write path and Sent only after its complete
successful frame write. The existing raw socket write-cancellation marker remains intact.

At complete JSON-message processing, capture ACK wall/monotonic receipt instants once before
parsing. Only Sent can validate a matching successful ACK under the existing classifier.
Both captures must be at or after their reservation origins and strictly before their fixed
deadlines; equality is late. The wait uses the same fixed monotonic deadline. The provider
does not echo the local ordinal: this proves the exact single local pending identity, not a
broker-authenticated sequence. Existing ambiguity and no-same-epoch-resubscribe rules remain.

A valid ACK creates only an internal AckObserved result; durable pending is still present.
Use a provisional per-symbol admission view for immediately following data, without exposing
provisional subscription membership as committed. Target data before subscribe ACK stays
suppressed; already-ACKed unrelated symbols stay admitted; provisional unsubscribe suppresses
subsequent target data. Queue receipts with their original one-per-frame wall/monotonic capture,
and emit none from `send_prepared`. Run the existing bounded buffered-message/control scan.
Duplicate/malformed/wrong control, Close, queue/read/scan bound exhaustion, cancellation or
poison discards all queued output and returns no proof, retaining durable pending. Only after
the scan reaches no complete available message may exact `(epoch, ordinal)` durable clear
run. On clear success, synchronously commit in-memory membership, return to Idle and return
one opaque ACK. There is no `await` between clear success and return. Clear failure poisons.
There is no ACK-result queue and no increase in any framing, scan, ingress or event bound.
A future duplicate closes the epoch and blocks later receipts; it cannot retroactively revoke
an ACK already returned. Do not claim otherwise.

Dropped/leaked Prepared leaves a non-idle session; no public reconstruct/resume/reset path
exists. Cancelling send before write, during write, during ACK wait or during its final scan
loses the moved capability and leaves non-idle durable ambiguity. Subsequent public command,
event-read, reconnect or graceful-close calls cannot drive that command or clear pending;
they return a typed error and poison/close uncertain epochs. Non-I/O introspection cannot
advance the phase or expose a proof. Drop closes the socket before releasing the lifetime
lock and never clears ambiguity. No asynchronous destructor or force-clear helper is added.
The existing restart refusal for uncertain durable state remains mandatory.

Convenience `subscribe`/`unsubscribe` delegate to prepare/send and discard only a genuine
successful ACK. They retain no parallel raw writer/parser. Default-consumer compile-fail
checks independently reject capability construction, mutation, cloning and deserialization;
default loopback/domain test APIs remain absent. Actual loopback checks prove reservation
before bytes, identity/no-op/expiry, wrong-session/nonreplay, abandoned/cancelled stages,
pre-send/unexpected/rejected/wrong/duplicate/missing/late ACK, buffered poison before proof,
unrelated-symbol admission and immutable capture. Exact equality uses deterministic timing
helpers plus actual late-path tests, not a scheduler-dependent exact-Instant socket test.
Existing default/test-support library, domain, socket/process and doc tests must still pass
with locked offline dependencies.

C3 separately makes storage pending/ACK mutation and storage proof construction private,
persists the exact Prepared identity before send, consumes the authentic ACK against the
current producer/pending identity, and commits ACK state before first receipt publication.
Only immutable timely transport capture permits DB ACK persistence after the transport
deadline; C3 removes the current fresh-DB-time deadline rejection and keeps strict captured
`acked_at < deadline` plus all producer/grant/session/fence checks. C2 transport acceptance
alone does not establish this DB sequence or accept the present public scalar storage API.

### 6.4B. C3 durable reservation and private storage contract

**D — 2026-09-22 root adoption after independent SOURCE review:** C1 and C2 have
separate SOURCE/offline acceptance. C3 is implemented in two sequential packages,
C3A storage/schema/API closure and C3B owning producer integration. Neither accepts
full WS-3, WS-3B, or a live gate. C2 remains frozen; no new transport getter, nonce
export, ACK factory, clock tolerance, or production operation is authorized.

The still-unpublished migration `0055` gains exactly one subscription column,
`pending_reserved_at timestamptz`. `pending_operation`, `pending_ordinal`,
`pending_reserved_at`, and `pending_deadline` must be all null or all present;
the present ordinal is positive and `pending_reserved_at < pending_deadline`.
The existing exact worker INSERT/UPDATE column grants include the new column.
Every row mapping, SELECT/RETURNING, pending write and pending-clear path includes
it. Clearing pending, epoch replacement, desired-to-absent and unsubscribe success
cannot leave a reservation timestamp behind. All C1 helpers, role identities,
FORCE RLS policies and lock order remain unchanged; no broad UPDATE grant is added.
The down migration is audited and changes only if its existing table drop does not
already cover the delta. Migrations `0001`–`0054` remain frozen.

The durable pending identity binds credential slot, current grant revision, epoch,
symbol, operation, ordinal, exact reserved wall time, exact deadline wall time and
pending subscription revision. Convert wall milliseconds by checked exact
millisecond conversion. `Instant` and the C2 private session nonce are never
persisted. Monotonic origins/deadlines remain in the in-process pending token.
DB `updated_at` is not a transport reservation timestamp.

Remove the public scalar `SubscriptionCommandOrAck` transition surface. Storage
provides only crate-private desired, prepared-commit and ACK-commit operations:

- Desired persistence is idempotent for identical state, does not rotate a revision
  on a true no-op, and rejects changes that would clear or overwrite pending.
- Prepared commit borrows an authentic `PreparedMarketSubscriptionCommand` while
  the caller retains its consuming ownership. Under unchanged C1 locking it checks
  the current producer/grant/session/fence and exact capability identity, persists
  pending, and returns a private, non-Clone `PendingSubscriptionCommit` only after
  known commit success. Failure/unknown commit returns no token. A DB delay may
  exhaust the command deadline; C2 still rejects the subsequent send before bytes.
- ACK commit consumes both that pending token and an authentic
  `MarketSubscriptionAck`. It compares every identity field with the token and
  locked row, including reserved wall/monotonic origins and both deadlines. Capture
  must satisfy `[reserved, deadline)` on both clocks. No caller `accepted` boolean,
  ordinal or timestamp can stand in for either capability. A timely ACK may commit
  after the transport deadline; remove fresh-DB-time deadline rejection and the
  `updated_at` lower bound. Fresh producer/grant/session/fence/date checks remain,
  as does fail-closed rejection of an ACK wall time later than fresh DB time.
- Subscribe success creates only a private committed subscription proof;
  unsubscribe success makes the row ABSENT and returns no publication proof. Both
  clear all four pending fields. Error/unknown outcome permits no publication.

Epoch proofs, subscription proofs, publication items/contexts/observation
construction, pending/committed tokens and raw epoch/pending/ACK/publication/status
repository methods are crate-private or narrower. Replace the parent stream glob
export with explicit safe lease/demand/cache/snapshot/status/error DTO exports and,
in C3B, the safe facade. No default or feature-enabled public proof factory is
allowed. A private committed subscription proof may be cloned for fanout only
because publication revalidates its exact revision, ACKED state, epoch, capture,
current demand, admission generation and producer fence under existing C1 locks.
This is a supported Rust API boundary; it does not claim to defeat arbitrary SQL
by a process already holding the worker DB role.

C3B adds one public `OwnerMarketStreamProducer` facade with private fields, owning
the repository, producer lease, DB session/epoch context, committed proofs and
`Option<MarketStreamSession>`. It is non-Clone and non-serializable. Its public
operations start by consuming a session, apply desired symbol/reference-count
data, and read-and-publish for current admissions. Results expose only committed
rows/status or metadata-only outcomes, never the session, receipt, capability,
pending identity or private proof. There is no `into_inner`, resume, reset,
force-clear or raw transition escape hatch.

**D — C3B integration refinement, 2026-09-23 KST:** changing a subscribed symbol's
positive desired reference count to another positive count is bookkeeping, not a
new transport subscription. Under the existing desired-state transaction and C1
checks, an exact current ACKED row with no pending fields retains its ACK capture,
epoch and subscription revision while updating only the positive count and
`updated_at`. The facade retains its authentic committed proof; no replacement
proof is minted from row scalars. Identical counts remain true no-ops. Pending
changes are still rejected, and transitions through zero retain the existing
desired/absent and authentic unsubscribe/subscribe sequence. This prevents C2's
already-subscribed `None` from leaving a formerly ACKED row in DESIRED. Tests must
prove positive-count changes keep publication valid without additional socket
subscription bytes, and zero then positive requires fresh authentic ACK evidence.

The facade has private `Ready | InFlight | Terminal` lifecycle. Start returns only
after known epoch commit; failure/cancellation/unknown outcome drops its session.
Desired persistence occurs before transport preparation. Its pre-prepare failure
may be reconciled by an exact DB read/idempotent call because no command ambiguity
exists. Once prepare returns `Some`, set InFlight and take the session into the
future before the first await. Preserve the order: pending DB commit, consuming
send, authentic ACK DB commit, then synchronous restoration of session and Ready.
There is no await between known ACK commit and restoration. Only C2's `None`
means no transport attempt/ordinal was reserved.

Before every `next_event` await, likewise take the session and enter InFlight.
Convert a receipt only through existing `StreamQuote::from_receipt`; publish only
with private committed proofs under C1 locking and the existing three-second wall
and monotonic freshness checks. Preserve original capture times/ordinal. Never
expose or fan out a receipt before ACK DB commit. After a known successful or no-op
DB result, restore Ready synchronously. Cancellation/panic, transport error,
pending/ACK/status/quote DB failure, or unknown outcome drops the local session
and leaves the facade non-resumable, including cancellation after receipt dequeue.
The existing one exact read-only epoch/ordinal reread after publication commit
error may prove success; otherwise return no committed result and do not retry.
Drop closes the session without clearing transport or DB ambiguity. C3 adds no
reconnect, scheduler, background task, runner/config or activation behavior.

C3A closes the API before C3B exists. Keep the external safe-DTO boundary test;
relocate the existing real-role case without dropping assertions into new private
`crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs`, wired only under
`cfg(all(test, feature = "market-stream-db-tests"))`. Reuse the existing support
module with crate-local imports and replace fabricated issued/ACK setup with real
C2 prepare/send/ACK capabilities. Tests must run that actual role case via the
library target; a passing external target alone is insufficient after relocation.
Private deterministic timing validators may test exact bounds, but no fixture may
fabricate the tested Prepared, ACK, pending identity, ACKED proof or receipt.

Later explicitly scoped verification uses all 55 migrations plus repository role
bootstrap/bootstrap SQL and direct serving-role logins on the exact owned disposable
cluster. C3A preserves the whole C1 role/RLS/GUC/lock/generation/revoke matrix and
proves new constraints/grants, exact pending identity, strict ACK bounds, delayed
persistence and default-consumer API closure. C3B separately proves actual socket
bytes after pending commit, receipt publication after ACK commit, packed subscribe/
unsubscribe behavior, cancellation at every await and real commit-failure outcomes.
Tests align the existing test-only C2 wall clock to PostgreSQL's sampled millisecond
and KST date/window; production clocks and fail-closed semantics are unchanged.
Both packages require fresh independent review and root evidence acceptance before
the separate WS3B demand/reconnect/runner package proceeds.

## 7. Resource, liveness and REST budgets

All figures here except explicitly cited source limits are **D project defaults**:

| Resource | Bound and overflow behavior |
|---|---|
| Active market keys | 30; desired/pending/ACKed counted explicitly; reject capacity excess. |
| Upstream commands | One pending globally; >=1 second between subscribe/unsubscribe commands; <=120 attempts/rolling 10min, <=1,000/day. Retries consume attempts. Heartbeat control frames have a separate budget. |
| Message/field packing | 256 KiB reassembled message, 100 records/message, 64 bytes/field; reject before allocation growth. Approval HTTP reply <=8 KiB. |
| Input flood guard | <=2,000 data messages/sec, burst 4,000 with a bounded monotonic token bucket; <=10 control/application-heartbeat messages/sec, burst 20. Exceeding guard closes with `LOCAL_INGRESS_LIMIT`, marks gap; not a KIS traffic guarantee. |
| Acquisition workspace | <=30 latest slots, <=2 batches of 30 observations, one bounded transport message, separate <=64 status transitions; explicit stream-owned heap budget <=8 MiB. Oldest price overwritten by latest; status overflow forces resync/close, never silently drops revocation. |
| DB price commits | Coalesce every 250ms; each symbol <=4 quote writes/sec, <=120 quote-row updates/sec for 30; <=4 batched quote transactions/sec. State writes have independent 1Hz coalescing, urgent revocation/control commits permitted under bounded command rates. |
| API memory | <=20 SSE consumers/Owner; each <=30 latest rows + one snapshot + 16 status controls, <=256 KiB pending serialized data. Backpressure replaces price deltas with a fresh snapshot request; >5s unwritable closes that consumer. |
| Notification loss | Requery active committed scopes <=1s; no provider calls. Read on invalidation is batched per owner, not per symbol. |
| UI render | Apply latest accepted versions once per animation frame; no all-tick history; paint timestamps retained only in bounded QA metrics. |
| Disconnect visibility | Transport failure invalidates live immediately in worker; persisted status + API reconciliation propagate when DB works. If worker cannot write, lease expiry/status age forces nonlive; browser/API silence watchdog <=5s makes live display unavailable. |

A process RSS includes Rust runtime, pools and libraries; the 8 MiB requirement is stream-owned
buffers, not total RSS. Soak tests report both measured RSS slope and actual channel capacities.
Quotes with old event times remain `STALE`/last-known even with a healthy socket; a healthy
heartbeat cannot make them `RECENT`. Distinguish `connection=CONNECTED` from
`freshness=RECENT|STALE|UNAVAILABLE` and from session open/closed. First-trade silence is not
proof of a dead connection. Halt is an observed field with its own last-observation time.
Reconnect creates `gap_since`/increments gap_generation; after new live data,
`gap_open=false` but `session_has_gap=true` remains until the next independently proven day.

REST caller evidence (read-only inspection; not runtime verification):

| Caller | Shared construction / role |
|---|---|
| research worker, daily/calendar/bootstrap and range-raw | `data-pipelines/collectors/src/worker.rs:883–965`; same factory, `ReadChannel::ALL`, shared mode branch. |
| corporate-action range | `data-pipelines/collectors/src/bin/kis-action-range-raw.rs:399–464`. |
| fixed-stock daily range | `data-pipelines/collectors/src/bin/kis-stock-price-beta-raw.rs:434–497`. |
| Owner V2 admission/reference/backfill/EOD and legacy intraday | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:492–557`; WS mode suppresses only the legacy recurring quote producer. |
| all coordinated GET dispatch/retries | `crates/kis-client/src/market_data.rs:344`; `read_coordination.rs:816–889`, `:1707–1740`, `:1779–1822`; persisted global debt, channel debt, reservation and dispatch guard. |

Production `shared_required` is mandatory for every credentialed reader using this slot.
The five-service mount set is documented in `docs/runbooks/stock-beta-intraday-quotes.md:140`.
External clients using the same App Key are not observable from source and must be excluded
at activation; no aggregate guarantee extends to unknown clients. Existing REST rate limits,
retry/cooldown, token issuer and ambiguous-debt preservation remain enforced. Token POST and
approval POST have separate issuance limits; the existing 1-second global GET guarantee does
not assert 1 request/sec over all authentication + data HTTP methods combined. Count all
classes in QA. No limiter source change is required for the known market-GET caller set;
an uncovered caller discovered downstream must be returned to coordinator before expanding
ownership. Do not reserve fake REST attempts for WS approval or frames.

Normal load target: receipt→DB coalesce <=250ms + commit/validation <=250ms + API wake/read
<=250ms (notification-loss path <=1s) + network/DOM <=250ms. This is a budget allocation, not
a measured result. Required measured p95 receive-to-DOM <=2s uses committed latest updates;
report coalesced/skipped count separately, including hot-symbol input, not just easy samples.
No claim about broker-to-server latency or complete tick capture.

## 8. Exact HTTP/SSE/browser contract

**D:** New strict application contract `schema_version=2`, `source="KIS_MARKET_WS"`.
Keep existing schema-1 REST routes/types unchanged. WS mode never fills their cache with WS
data; no generic source adapter or `next_poll_after_ms` in stream DTOs.

Prefix: `/api/v1/research/owner-beta/equity-universe-v2`.

| Route | Request / response |
|---|---|
| `POST {prefix}/stream-leases` | Cookie session, Owner role, same-origin + existing `x-csrf-token` and `Idempotency-Key`. JSON `{schema_version:2,consumer_id:uuid,renewal_sequence:u64,identities:[{membership_id:uuid,instrument_id:"NNNNNN.KRX",generation:u64}]}`. Create/replace/renew entire set atomically. Returns 200 `{schema_version:2,lease_id,consumer_id,renewal_sequence,lease_expires_at,renew_after_ms:15000,identities}`. |
| `DELETE {prefix}/stream-leases/{lease_id}` | Same mutation auth; body `{schema_version:2,consumer_id,renewal_sequence}`; 200 `{schema_version:2,lease_id,released:true}` with durable idempotent replay. |
| `GET {prefix}/market-stream?lease_id={uuid}` | Cookie-authenticated same-origin SSE. Lease ID is not secret authorization; owner **and session** must match. GET does not create/renew a demand, enqueue work or call KIS. Initial snapshot then latest-value deltas/status. |

Generations are safe positive JSON integers <=2^53−1. `renewal_sequence` is a safe
nonnegative JSON integer: the initial mutation is 0, and each subsequent replacement or
renewal increments it by one. This matches the existing durable lease contract; release
carries the last accepted sequence, not a new renewal. (Coordinator clarification, 2026-10-03.)
Database counters (`quote_version`, `state_version`, receive ordinal) serialize as canonical
decimal strings <=i64::MAX. All UUIDs canonical; arrays sorted/deduplicated; max JSON body
16 KiB; reject unknown fields, owner/provider/credential fields and malformed URL parameters.
Auth is checked before semantic parsing: 401 no session, 403 non-Owner, 404 unknown/foreign/
generation-mismatched/other-session lease or identity, 409 sequence/identity capacity conflict,
503 infrastructure unavailable. Off mode returns typed `FEATURE_DISABLED`, creates no lease.
Once streaming begins, failures use typed status + connection close, not an HTTP status rewrite.

Every SSE event's JSON is exactly `{schema_version, stream_id, event_sequence, server_time, body}`;
the event kind is the SSE `event:` field. `body` is the corresponding object below. Identity and
row arrays use ascending canonical membership UUID order, with no duplicate memberships or
instruments; display ranking is independent of wire ordering. (Coordinator clarification, 2026-10-03.)
Every SSE event carries `schema_version:2`, `stream_id` (new UUID per HTTP connection),
`event_sequence` (string), `server_time` and one body below. Event ID is
`{stream_id}:{event_sequence}`. This is an API delivery cursor, never a broker sequence.

```text
event: snapshot
body: { lease_id, lease_expires_at, rows: StreamRow[1..30] }
event: delta
body: { rows: StreamRow[1..30] }                    // complete replacement rows
event: status
body: { connection, reason_code, gap_open, session_has_gap, gap_generation }
event: reset
body: { reason_code: "RESYNC_REQUIRED" }           // immediately followed by snapshot
```

`StreamRow` is exactly:

```text
membership_id, instrument_id, generation, row_generation UUID,
venue:"KRX", currency:"KRW", source:"KIS_MARKET_WS", wire_version,
session: null | {date, timezone:"Asia/Seoul", calendar_source:"kis",
  calendar_source_version:"kis-chk-holiday-v1:schema-1",
  calendar_content_sha256, window_contract_sha256},
subscription:"DESIRED"|"PENDING"|"ACKED"|"REJECTED"|"ABSENT",
connection:"DISCONNECTED"|"CONNECTING"|"CONNECTED"|"BACKOFF"|"STOPPED",
market_state:"OPEN"|"CLOSED"|"UNKNOWN",
freshness:"RECENT"|"STALE"|"UNAVAILABLE",
availability:"LIVE"|"LAST_KNOWN"|"AWAITING_FIRST_TRADE"|"UNAVAILABLE",
reason_code:null|<closed enum>, state_version:string,
gap_open:boolean, session_has_gap:boolean, gap_generation:string,
quote:null | {price,base_price:null,base_price_reason:"NOT_PROVIDED_BY_CHANNEL",
  change_from_previous_day,change_percent_from_previous_day,direction,
  trade_volume,cumulative_volume,halted:boolean,
  business_date,trade_time,provider_trade_at,received_at,committed_at,
  epoch:UUID,quote_version:string,receive_ordinal:string}
```

No producer secret/fence/credential slot/session hash/rights-document bytes leave the server.
Public `epoch` is an opaque receipt namespace. `halted` is explicitly “as of this quote”; UI
must not show unknown halt as false. `market_state` comes from proofs; observed halt is a
separate badge. Recent halted observations do not claim tradability.

Closed reason enum: `FEATURE_DISABLED`, `NO_ACTIVE_DEMAND`, `CALENDAR_UNAVAILABLE`,
`SESSION_WINDOW_UNAVAILABLE`, `SESSION_CLOSED`, `AWAITING_FIRST_TRADE`, `QUOTE_STALE`,
`CONNECTION_LOST`, `RECONNECT_GAP`, `SUBSCRIPTION_PENDING`, `SUBSCRIPTION_REJECTED`,
`SUBSCRIPTION_AMBIGUOUS`, `APPROVAL_UNAVAILABLE`, `BUDGET_EXHAUSTED`, `PRODUCER_UNAVAILABLE`,
`PIPELINE_LAG`, `WIRE_SCHEMA_MISMATCH`, `PROVIDER_RESPONSE_INVALID`, `QUOTE_VALUE_INVALID`,
`MARKET_CLASS_UNSUPPORTED`, `LOCAL_INGRESS_LIMIT`, `RESYNC_REQUIRED`, `ACCESS_REVOKED`.
Broker prose/code strings are not accepted as enum variants. Transport errors map to these.

Projection precedence: unauthorized → terminate/purge; feature off or invalid rights/proof →
no quote/unavailable; closed valid window → closed + same-day authorized last-known quote;
no demand or non-ACKed/current unhealthy epoch → last-known only; no valid observation →
awaiting-first-trade; otherwise recent requires <=30s receipt **and event** age, current ACKed
epoch and healthy producer/connection; older observations are stale even on a quiet healthy
socket. Prior-day values are not exposed by this stream (EOD history remains a separate UI).
On reconnect retain same-day authorized last-known with original epoch and a gap marker.

API authenticates at open and **before each emitted data batch**, and at least every second
while idle, using the existing canonical DB session lookup/role checks. Each read transaction
uses actor RLS and rechecks membership/latest generation/grant/date. Revalidation fails closed
on DB outage; discard queued rows before retry/close. Revocation committed before the next
authorization check prevents that batch. Bytes already delivered/in flight cannot be recalled;
do not promise instantaneous network revocation. On lost access send no prices, clear client
state, stop renewal and terminate. No use of a stale Session extractor forever.

For race-free startup: install LISTEN, read a consistent RLS snapshot of lease + rows, then
reconcile dirty notification state before sending subsequent deltas. Snapshot contains full
per-row version namespaces; compare `(row_generation, state_version)` and quote epoch/version.
Replacement snapshot clears rows no longer authorized. Serialize reads per consumer so an
older snapshot cannot overwrite a newer delta. No durable event history: **every reconnect
gets reset + current snapshot**, irrespective of `Last-Event-ID`. Validate/bound but never trust
that header for authorization; no replay query to KIS. Overflow uses the same reset procedure.

SSE: `Content-Type: text/event-stream`, `Cache-Control: no-store`, `X-Accel-Buffering: no`,
comment heartbeat every 15s; enforce expiry watchdogs independently. Emit an authenticated,
DB-revalidated `status` event at least once/second even when there is no trade, so the 5-second
browser silence watchdog does not misclassify a quiet healthy symbol. Such delivery advances
only the SSE event sequence, not quote/state versions unless committed state actually changes.
The client ages freshness from `server_time` and applies connection-status overlays independently
of row-version deduplication. Add an exact nginx route
with `proxy_buffering off`, `proxy_cache off`, gzip off, HTTP/1.1 upstream and 60s read timeout.
Allow only configured same origin; reject conflicting Origin or cross-site Fetch Metadata,
no wildcard CORS. Native EventSource GET cannot add CSRF headers; it is read-only and creates
no lease. Mutations retain existing CSRF. No cookie/approval key in URL, logs or localStorage.

One page-level controller per tab owns board/detail demand and one EventSource. On hidden,
offline, navigation away or logout: close EventSource, issue one bounded lease release when
possible, stop renewals; TTL handles missing cleanup. On resume create a new consumer lease,
then one SSE; do not revive a released ID. Disable old 5-second quote GET loop in WS mode.
No raw quote browser persistence, service-worker cache, demo/Member surface, or public fallback.

## 9. Exact30 onboarding and runtime configuration

**E:** `FIXED_30_INSTRUMENT_IDS` at `crates/market-data/src/fixed_stock_price_beta.rs:29`
and `scripts/qa/owner-equity-v2-live-acceptance.mjs:8` match in order, 30 unique entries.
Use `configs/universes/kr-stock-price-beta-v1.json`; checked-in expected file hash is
`2a0d55143df0274fcfa357f2824ed752e2969469f93254ed7dfa64766a00dde1`, identity-list hash
`0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79`.

```text
005930 000660 373220 207940 005380 000270 105560 055550 068270 035420
035720 005490 051910 006400 012330 028260 012450 329180 034020 015760
017670 030200 066570 009150 096770 036570 090430 011200 003490 000810
```

Normal Owner API onboarding uses pilot `005930.KRX`, wait for actual READY/admission, then
the other 29 sequentially. Reuse existing READY identities, safely resume partial progress;
do not force DB READY, recreate generations, or invoke retry on indeterminate operations.
Admission/reference/history REST calls remain allowed only under their existing gates and
shared budgets; they are not recurring WS quote requests. QA observes the real page's lease,
not an additional hidden QA demand. Thirty ACKs are required for final completion; a rejected
symbol has a typed row and blocks full30 acceptance. Resolve any newly verified capacity below
30 as a product decision; do not reduce to five, rotate symbols or use another key.

**D:** Keep `OWNER_INTRADAY_QUOTES_MODE=off|owner_only`, default off. Add
`OWNER_INTRADAY_QUOTE_TRANSPORT=rest|market_ws`, default rest for compatibility. With
`owner_only` and `market_ws`, API and runner require the same source contract pin and proof; runner daemon
starts exactly the stream producer. `--once` starts no stream. Explicitly reject simultaneous
legacy REST intraday + WS producers for an Owner. Ordinary EOD/reference reader remains.

New nonsecret inputs: API-only `OWNER_MARKET_STREAM_ORIGIN` (exact canonical HTTPS origin; no wildcard, path, credentials or query), `KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID` UUID,
`KIS_MARKET_STREAM_GRANT_ID` UUID, `KIS_MARKET_STREAM_CONTRACT_SHA256` (approved contract pin).
No configurable broker URL/TR, no secrets added to API/Web. Reuse
`KIS_READ_CREDENTIAL_GENERATION` without incrementing it for WS activation. The writable
leaf `${LAGRANGE_RUNTIME_STATE_DIR}/kis-market-stream` mounts read-write at
`/run/lagrange/kis-market-stream` only in `owner-equity-v2-runner`, with directory UID:GID
10001:10001 mode 0700 and regular state/temp files mode 0600. Its sibling
`${LAGRANGE_RUNTIME_STATE_DIR}/kis-market-stream-locks` mounts read-only at
`/run/lagrange/kis-market-stream-locks` only in that runner; it is root:GID-10001 mode 0750
and contains only distinct zero-length root:GID-10001 mode-0440 single-link regular
`connection.lock` and `state.lock` anchors. Both mounts use `create_host_path: false`.
The `approval-state-v1.json` schema requires the configured opaque `credential_slot_id`
and verified anchor-domain binding: approval, quota history, ambiguity, pending command,
and connection epoch all belong to this one domain, separate from the old REST ledger.
Production paths are fixed and have no environment/API override. An opaque
`MarketStreamDomain::open_production(credential_slot_id)` verifies the state and anchor
directories and both anchors. Approval and socket construction share that domain; no public
production API accepts independently selected stores, paths, owner IDs, or anchor names.
All state/lock operations remain crate-private. Synthetic temp-root/loopback domain
construction exists only under test/test-support, absent from default consumer builds.

Official first provisioning may create the domain only with absent/empty writable state.
First initialization is one explicit commit-pinned WS-5 provisioning action, only after
creating a wholly new empty state leaf and the final verified anchors, before any runtime
mount, activation, or possible holder. It atomically creates and fsyncs one complete nonempty
slot/domain-bound state file exactly once. Production runtime only validates that file;
it never initializes missing or zero-length state. After first initialization, missing,
empty, or invalid state is uncertain and stops without write; reapply never initializes.
Synthetic test initialization is allowed only in the same call that creates both previously
absent synthetic directories/anchors, never when reopening an existing layout.
Reapply validates existing objects and preserves anchor inodes and metadata; missing/unsafe/
mismatched anchors, nonempty legacy state without domain binding, or uncertain state stop
without mutation. Runtime/installer never automatically repair, reset, migrate, recreate,
or clear state. A root replacement of a valid anchor is an operator incident, not a new
identity to bless on reapply. No API/Web mount, source file, or image layer contains approval
state. WS-2 synthetic tests do not substitute for WS-5 root ownership/mount/reapply evidence.

**D:** New `scripts/ops/install-owner-market-stream-grant.sh` is a separate installed,
commit-pinned command, default `--plan` without protected reads/DB/network. Its explicit
`--apply --approval-input <protected-file> --expected-commit <40hex>` verifies the installed
manifest/env/actual candidate and reviewed approval artifact, then uses the existing controlled
DB administrative path to insert only the new grant row with parameterized values. Replay of
the same grant ID and exact hash is a no-op; mismatched replay or active slot conflict fails.
An explicit `--revoke --grant-id <uuid>` only revokes the selected grant. It cannot issue keys,
start services, provision another key, amend old entitlements, reset state or infer approval
from flags alone. The approval artifact must be supplied under a coordinator-reviewed action;
it is not generated by the helper or the worker. The immutable release installer remains
DB/provider-free. WS-5 owns this narrow new helper; WS-3 owns its table/role contract and tests.

**D — network-contract pin:** The immutable grant row is the runtime authoritative binding
only after the separate commit-pinned grant installer copies `network_contract_sha256` exactly
from a coordinator-reviewed approval artifact. Runtime must retain that exact slot/grant/
revision binding; a caller cannot supply or override the pin. Hex syntax and synthetic test
hashes establish no live entitlement. There is no invented approved production value, ACTIVE
seed or activation permission in this source amendment; WS-5 installer evidence and G1–G5
remain open.

Current calendar/session proof contract is retained: valid exact KST day, immutable KIS
calendar source/hash and pinned session-window artifact, half-open interval. Checked-in
`configs/market-hours/krx-intraday-session-windows-v1.json` has empty entries; it supplies no
live date. Existing operational proof loader must provide approved current-day evidence.
There is no new automatic KRX browser/HTTP collection, no weekday inference and no timezone
restamping. Long-running next-day operation remains gated on how valid day proofs are supplied.
After-market changes in P5 do not extend this feature's regular-session window or EOD cutoff.

## 10. Downstream ownership and dependency refinements

These are proposed changes for later workers, **not edits performed by WS-1**:

| Worker | Exact owned additions/refinements |
|---|---|
| WS-2 | `crates/kis-client/src/market_stream.rs`, `market_stream_approval.rs`, `market_stream_wire.rs`, `market_stream_state.rs`; `lib.rs`, crate/workspace Cargo and lock; `tests/market_stream*.rs`. Expose only market observation/receipt/control types and bounded real transport. It owns dedicated WS protected-state/lock/budgets implementation; existing `websocket.rs`, REST/auth ledger behavior untouched. |
| WS-3 | `crates/market-data/src/market_stream.rs` and exports; `crates/job-queue/src/owner_equity_v2/market_stream.rs`, `market_stream_producer.rs`, parent exports and necessary common intraday helper visibility; `src/bin/owner-equity-v2-runner.rs`; reserved 0055 pair and actual-role tests. Narrow addition: `crates/kis-client/src/read_coordination_config.rs` for the transport enum/config parse only, after WS-2. No change to old shared-state semantics. |
| WS-4 | New `crates/api-server/src/{http,repos}/owner_market_stream.rs`, bounded session revalidation support in existing `http/session.rs`, router/state/contract/OpenAPI affected seams, API Cargo/lock, tests; `apps/web/lib/products/intraday-stream-{contracts,client}.ts`, quote components/controller/dashboard wiring; exact nginx route; existing Owner acceptance QA trio. Parent plan's Web/Next local-document rule applies before code. |
| WS-5 | Existing ops/compose/env/runbook scope plus **`scripts/ops/provision-linux.sh`** for only the new protected WS leaf and new **`scripts/ops/install-owner-market-stream-grant.sh`** as specified in section 9. Preserve the DB-free immutable installer. No invented approval or old state mutation; source-only dry-run tests with synthetic approval inputs, real-role grant checks supplied by WS-3. |
| Coordinator / WS-6 | Adopt contract and network/rights gates; update applicable policy only after approval; integrate actual diagram evidence/PNGs later; independently verify full-role DB, socket→DOM and source/ops compatibility. No structural diagram edit for this documentation-only WS-1 change. |

Graph refinement: **WS-1 adoption → WS-2 SOURCE/offline → WS-3 → WS-4/5 → WS-6** may proceed
with synthetic keys/grants and loopback endpoints. A new live network/rights grant is needed
before enabling credentialed paths, not to run provider-free source tests. There is no new
execution package or hidden concurrent Cargo owner. If WS-5's grant helper requires
broader permission/DB changes than specified, report to coordinator for a bounded follow-up.

## 11. Required verification, fixtures and acceptance evidence

WS-1 ran no build, implementation test, DB, Docker, browser or provider operation. The
following are **future acceptance requirements**, not PASS claims.

### Documented synthetic fixtures

Create fixtures locally with a manifest stating `synthetic=true`, P1/P5 source IDs/dates,
wire version, expected field count 47, explicit invented numbers and expected outcome. Use
the exact30 public codes; no downloaded live payload or key. The synthetic request above is
not a credential. At minimum describe/generate:

- One regular in-window trade with 47 fields: `005930`, `100001`, positive price, signed
  change/direction, volumes, business date equal to synthetic proof, `20`, `N`, `0`, blank
  field45, and field47=`2`; all required consumed fields populated.
- Four-record packed message (all 188 fields), including same-second distinct volume and
  same-price subsequent trades. Exact duplicate, out-of-order event/volume, stale date,
  future time, blank required value, zero price, signed mismatch and rounded-zero percent.
- Officially documented `TRHT_YN=Y`; no-price/first-trade silence without fabricated halt.
  Nonregular field47=`3` and expected/VI `HOUR_CLS_CODE=C` cannot become a regular trade.
- Legacy 46-field record rejected; 48 fields rejected; bad count, extra pipe, embedded
  newline, invalid UTF-8, overlength field, oversized fragmented message, binary/encrypted
  market message, account/HTS TR, forbidden URL/redirect rejected before publication/send.
- Matching subscribe ACK, unsubscribe success class, wrong-symbol/operation ACK, duplicate
  ACK, nonzero code, missing/late ACK, PINGPONG, protocol Ping/Pong/Close and timeout.
- Fake approval HTTP with secret sentinels in success/error/nested text/redirect and timeout;
  assert captured test wire only, zero secret in Debug/logs/DB/API/report/errors and durable
  outcome/counter handling. A known synthetic key in private test state is expected.

### Actual local execution requirements

| Proof | Must execute the actual boundary |
|---|---|
| WS-2 transport | Real loopback HTTP approval server + WS server exercising sockets, fragmentation/control frames/close/reconnect. Test-only endpoint constructor restricted to loopback; no production env override to arbitrary host. Count approvals/connections/commands; test two contending processes and crash/lock cancellation. |
| WS-3 storage | Disposable PostgreSQL with **all** migrations and repository `role-bootstrap.sql`/`bootstrap.sql`, app/worker/research_writer/admin roles. Production repository methods, never superuser-only substitute tables. Check grant/entitlement/session/membership revocation, fence/epoch/generation/date races, ACK precondition, denied direct app quote writes and cross-owner SELECT. |
| Shared acquisition | 1 consumer versus 10 tabs with identical 30 identities gives 30 upstream registrations and zero recurring quote GETs in both cases. Verify each lease expiry/release, >30 capacity rejection under concurrent mutations, bounded churn/command history, old socket process cannot open a competitor, no REST lock starvation. |
| DB/notification failure | Slow DB, failed/unknown commit, listener loss/restart, cache GC/recreation and epoch rollover. Memory fanout cannot bypass DB; a reconnect/resnapshot converges to committed versions and no stale row is revived. |
| WS-4 delivery | Actual API authentication/HTTP/SSE through local nginx, actual React page in local Chromium, 30 rendered rows and selected detail, second tab, hidden/offline/logout and cross-owner attempts. Fake DOM objects or a standalone fixture SSE server are not the target. |
| Latency/soak | WS-6 runs >=30 minutes through fake provider→real producer→real-role PG→real API/SSE→nginx→Chromium. Include regular 30-symbol traffic, one hot symbol and bursts. Count coalescing/queue occupancy/DB writes/RSS/commands. Measure p95 receive-to-DOM <=2s with correlated epoch/version/receipt; distinguish clock synchronization/measurement error. |
| Regression | Existing calendar acquisition in both morning/EOD orders, actual app-role resolver, exact-source reuse, entitlement amendment replay, EOD publication and restart, REST shared token/debt, V1/off UI, immutable env/image/daemon guards and no DB downgrade rollback. |

Use the coordinator's shared Cargo lock and `CARGO_BUILD_JOBS=2`. Actual DB environment
absence is not a test skip that counts as PASS. WS-4 prepares/executes relevant typecheck,
Vitest/Playwright, API/HTTP tests; WS-5 runs official helpers on disposable fake inputs, not
rewritten copies. Reports identify synthetic versus provider/production evidence explicitly.

QA correlation contains only source/wire version, epoch, quote_version, receive ordinal,
received/committed/DOM times and typed status/counters. Do not persist actual prices, original
frames, secret-bearing envelopes, credential/account identifiers or broker prose in diagnostics.
Real provider acceptance later needs 30 ACKs and at least two subsequent fresh pilot receipts
through DB/SSE/DOM even if price numbers are equal. Other symbols may remain quiet honestly;
all30 does not mean all30 must trade during a bounded observation window.

## 12. Coordinator decision ledger and preservation

| Gate | Exact outstanding decision/evidence | SOURCE/offline impact |
|---|---|---|
| G1 network | Approve literal HTTPS Approval + plaintext WS `/tryitout`, market H0STCNT0 only, with documented cleartext key exposure, or return an officially documented alternative. | No external sockets; loopback transport/parser can proceed after contract adoption. |
| G2 current limits | Recover/confirm September 16 notice and its applicability; confirm this slot has no other session/key-sharing client. Numeric subscribe/reconnect throughput remains undocumented; accept conservative project budgets subject to bounded pilot. If <30 capacity, explicit product decision. | Cap30 state/negative-limit tests can proceed; do not claim verified current live capacity. |
| G3 narrow rights binding | Apply existing ADR-0005 decision to this new source/cache/SSE and record only the necessary scope amendment/approval; reviewed grant input with exact reference/hash and date coverage. No general personal-use reconfirmation. | Synthetic grant, RLS and revocation proof can proceed. |
| G4 daily proof/deployment | Fresh permitted calendar/window source, audited immutable candidate/install inputs, credential-slot binding, backup/restore and resource gates. Long-term next-day proof supply still unresolved. | Source/test/ops dry-run can proceed. |
| G5 actual protocol | Permitted one-symbol pilot must verify 47-field frames, ACK/unsubscribe/heartbeat behavior and current quotas; no auto-downgrade to46 or auth reissue on conflict. | Documented synthetic tests are necessary, not actual-provider proof. |

Do not reclassify any G as granted because this plan was approved. Do not ask the user to do
manual QA instead of preparing the actual local proof. No further decomposition is needed
to complete WS-1; the existing downstream packages can execute the adopted source contract.
Resolution of G1–G5 belongs to coordinator-controlled later work.

Preserved historical failure context: September 19 has a claim without a settled Raw source;
token issuance and one locally completed calendar reservation/dispatch were observed, but the
exact error was erased. A retryable dispatched failure followed by a pre-reservation retry
failure is possible, **not established**. Never retry/reset/delete that claim or alter its
UUID, debt, generation, token/state. The next date is not automatic approval. Installed
`75d28ef...` versus source `7757245`, old `74b39fa` nine healthy/two drained, are historical,
not current observations. Old attestations are expired/nonreusable. Wrapper expiration stays
`2026-09-22 08:54:48 KST`; no extension. WS-1 performed no operational action.

## 13. WP-1 runtime completion amendment — coordinator adopted, 2026-10-01

**Adopted for the approved source/local completion plan; no live or production authority.** Root
recovered the idle WP-1 report, reviewed the source seams and preserved producer hashes, and
adopted the C2 ownership/idle seam, private runtime/status boundary and section14 initializer.
Root did not establish that0055 is unpublished: preserve both0055 files byte-for-byte and put
the section13.3 schema/helper/grant delta in the new append-only pair
`0056_owner_market_stream_runtime.{up,down}.sql`. Fresh fixtures apply all migrations; rollback
tests remove0056 before0055. No current production query or migration is implied. WP-3 runs
through bounded, sequential coordinator-dispatched units, starting with transport/config only.
This amendment
fills only the runtime/initializer seams identified in
[the R111 review](../../reviews/2026-10-01-kis-market-stream-completion-gaps.md). The September 30
handoff supersedes intermediate pending/failure narratives: original1–26 and four pacing cases
are adopted focused evidence, not full WS3/SSE/browser/live acceptance. Sections 6.3–6.4B keep
their accepted lock, role, capability, authentic-capture and terminal-cancellation invariants.
Their historical exclusion of runtime work is extended only by the adopted WP-3 manifest.
G1–G5, wire/endpoint/rights scope and all section 7 bounds remain unchanged.

### 13.1 Entry, ownership and transport seam

The runner selects exactly one daemon quote implementation: off starts neither; owner_only/rest
starts the existing REST producer; owner_only/market_ws starts the new runtime. `--once` starts
neither and must not open WS state or construct an approval client. EOD/reference work continues
independently. Add a separate `IntradayQuoteTransport::from_optional_str(Option<&str>) ->
Result<IntradayQuoteTransport, ReadCoordinationConfigError>` (`Rest|MarketWs`, default Rest);
preserve the existing three-argument `ProductionReadCoordination::from_values` API.

Proposed job-queue signatures (all new types have private fields and typed, redacted errors):

```text
// Public safe orchestration; no receipt, session, proof or SQL setter is returned.
OwnerMarketStreamRuntimeConfig::from_values(
    slot: Uuid, grant: Uuid, contract_sha256: &str, credential_generation: u64,
    holder: Uuid,
) -> Result<Self, MarketStreamRuntimeError>;
OwnerMarketStreamRuntime::new(
    repository: OwnerMarketStreamRepository,
    calendar: OwnerIntradayQuoteRepository,
    windows: IntradaySessionWindowSource,
    approval: ApprovalClient,
    config: OwnerMarketStreamRuntimeConfig,
) -> Result<Self, MarketStreamRuntimeError>;
async OwnerMarketStreamRuntime::run_daemon(self, shutdown: watch::Receiver<bool>)
    -> Result<MarketStreamRuntimeExit, MarketStreamRuntimeError>;
// Inside owner_equity_v2 only; no raw channels exported by the parent module.
async OwnerMarketStreamProducer::start_resolved(
    repository: RuntimeMarketStreamRepository, lease: StreamProducerLease,
    day: ResolvedMarketStreamDay, session: MarketStreamSession,
) -> Result<RuntimeOwnedProducer, MarketStreamProducerError>;
async RuntimeOwnedProducer::run_owned(
    self, demand: watch::Receiver<DesiredSet>,
    lease: watch::Receiver<StreamProducerLease>, shutdown: watch::Receiver<bool>,
) -> Result<RuntimeOwnedExit, MarketStreamProducerError>;
```

The D2-B implementation refines these private seams: a separate `RuntimeOwnedProducer`
preserves the existing focused facade unchanged. `RuntimeOwnedExit` is private and finite;
D3 maps its known-clean outcomes to the public runtime exit. D3 creates one bounded runtime
repository per daemon lifetime and passes clones of that same adapter to lease supervision,
the socket owner and the writer, so deadline/cancellation/unknown-commit terminal state is
shared. The adapter exposes no raw pool or proof factory. Only the socket task owns the
session. Watch senders are private supervisor inputs: they publish every successful fresh
demand observation, including unchanged sets, at least once per second; sender loss or a
two-second observation gap stops the owner. Lease identity and the five-second remaining
margin are checked independently while commands are pending.

The accepted C1 epoch proof includes `gap_generation`; a committed runtime gap transition
can advance that generation. D2-B therefore suppresses publication as soon as any gap-opening
transition is requested, discards the current buffer, and drains this incarnation. It never
rewrites epoch-proof scalars from a status result. Only a definite clean socket close, joined
children and known fenced retirement can return a fresh-epoch outcome; D3 must then obey the
existing backoff/budget and obtain a new resolved epoch and authentic ACKs. Ambiguous close,
forced task abort, DB deadline or unknown commit remains terminal. Likewise an outstanding
batch invalidated by observed demand changes is never replayed or acknowledged as current;
a known successful commit may be retained only as shutdown evidence while this incarnation
is drained. These choices preserve the C1/D1 guards and do not prove runtime recovery until
the later actual-role/loopback gate passes.

The runtime constructor must also reject a mismatched supplied approval client before any
operational client, anchor reservation, DB lease or provider action. Add only
ApprovalClient::matches_runtime_binding(expected_slot: Uuid, expected_generation: u64) -> bool
as a synchronous comparison of the client's immutable in-memory domain slot and generation.
It returns true only for a nonnil expected slot, a positive expected generation, equal slots
and an exact canonical decimal generation string; it does not parse or normalize aliases.
It performs no protected-state, lock, credential, approval or network access and reveals no
actual metadata. OwnerMarketStreamRuntime::new maps false to ConfigurationInvalid before
constructing its operational client. Existing opaque client-generation contracts stay
unchanged. This predicate proves only supplied metadata consistency, not credentials,
rights, durable state or connection authorization. D3-B0 implements this prerequisite;
constructor wiring and runtime behavior remain subject to the later D3-B/D4 gates.

The runtime uses only start_resolved for production; its private day value supplies the
publication window and lineage, without exposing a proof factory. Existing public focused
facade signatures remain available with their existing semantics and no resume path.
The runtime owns one socket-owner task, one serial publication task and bounded lifecycle
supervision. These are local Tokio tasks, not additional producers. It joins every task before
starting a replacement. The session is never cloned, split into independently driven readers/
writers, placed behind competing read/command futures, or exposed by a recovery method.
The supervisor renews the DB lease every 5s (TTL20s) even during ACK wait, reads fresh demand
and rights/day eligibility at least every 1s, and sends only the latest desired set. It accepts
a renewal only if holder/fence/slot/grant/revision are unchanged; private lease replacement is
not a public setter. Child task handles use abort-on-drop ownership; the runner normally signals
then joins them. An externally dropped runtime cannot detach a live socket task; a replacement
still cannot pass the lifetime lock until that task actually drops its session.
Failed/unknown renewal, stale fence or <=5s remaining life stops publication
and closes the socket. Bound every DB operation to <=1s using both transaction-local limits
and an owned client deadline; pool acquisition counts. Timeout/unknown commit is terminal,
not a retry using an old lease. A stalled publication cannot prevent lease-loss shutdown.

Two narrowly reopened kis-client interfaces are required; no Prepared/ACK changes:

```text
// Affine lifetime-lock owner: private fields; no Clone/serde/constructor/getter for the lock.
MarketStreamClient::reserve_connection(&self)
    -> Result<MarketStreamConnectionOwner, MarketStreamError>;
async MarketStreamConnectionOwner::connect(self, proof: MarketStreamSessionProof)
    -> Result<MarketStreamSession, MarketStreamError>;
async MarketStreamSession::next_event_until(&mut self, deadline: std::time::Instant)
    -> Result<Option<MarketStreamEvent>, MarketStreamError>;
```

`reserve_connection` validates the configured domain and takes the existing connection
anchor, with **no approval/network/budget attempt**. Runtime then claims the DB lease and
rechecks grant/demand and independently resolved day/window before consuming `connect` with
that day's transport proof. `connect` checks current-day proof before any attempt/approval.
The owner moves the very same lock into the
session; failed connect drops socket before lock. Existing `connect()` delegates to this path
using its configured proof for compatibility. No arbitrary domain/lock input is accepted. This realizes the section 5
anchor → DB lease → approval/socket order, which current monolithic `connect()` cannot express.

`next_event_until` returns `None` only at a safe idle read boundary, retaining partial frame/
fragment bytes and the session. It bounds socket-read waiting internally using the existing
read-buffer/`readable` mechanism, not `timeout(next_event())`. Check the deadline during
continuous traffic as well as silence; use <=50ms slices in the owner loop. Finish a started
control write under the existing bounded write timeout or fail terminally; never return Idle
with a pending write. Preserve watchdog/control/poison-before-queued-receipt behavior. Existing
`next_event()` retains its API. Dropping either future still cannot justify restoring a facade.

Commands remain serialized barriers: eligibility → desired DB mutation → prepare → pending
DB commit → the existing consuming `send_prepared` → authentic ACK DB commit → Ready.
Keep `send_prepared`'s no-event-output and bounded poison scan; do not add public receipt
callbacks or an ACK queue. While it is in flight, its pinned future is polled to completion;
the supervisor and already-dispatched publication work continue, but no second socket read
or command runs. Buffered unrelated receipts retain original timestamps; the target is usable
only after known ACK DB commit. ACK delay/queue overflow is measured; >3s receipts are dropped
as PIPELINE_LAG and transport overflow closes. This is a control-path interruption, not a
waiver of normal-load latency or a claim of lossless ticks. Failure to meet section 7 in the
required hot-symbol/command soak rejects the implementation; it does not authorize enlarging
queues or weakening C2 to make the test pass.

### 13.2 Demand, terminal states, reconnect and day evidence

| Runtime state | Entry / allowed transition |
|---|---|
| DISABLED | No state/key/socket access; runner preserves EOD. |
| WAITING | No demand or valid day/window absent/closed. Re-read local committed inputs <=1Hz; zero provider calls. Positive demand with missing proof may retain the anchor and a renewed DB control lease solely to persist unavailable status, with no epoch. |
| CLAIMING / CONNECTING | Verified grant and positive demand permit anchor then DB lease; only verified open day permits consuming connect/approval/socket; epoch commit before first command. |
| ACTIVE | Fresh ACKs, bounded read/commit, lease renewal and demand observation. Quiet trading is not disconnection. |
| DRAINING | No demand, shutdown, close time or day change: suppress immediately, drop uncommitted receipts, finish a safe close within 5s. |
| BACKOFF | Only definite clean transport closure, known DB outcomes and valid current proofs permit a **new** facade/epoch after durable eligibility. |
| STOPPED | Ambiguous send/ACK/DB/connection, malformed state, grant revocation, budget exhaustion or unknown prior session; no automatic reset/reissue. |

Observed zero demand suppresses publication immediately through both private buffers and DB
checks. Ordinary individual-symbol removal may wait up to5s before unsubscribe; revocation has
no grace. Positive count→positive count preserves the authentic proof/revision. `Deferred`
preserves committed progress, holds no Prepared, and is revisited only at `not_before_ms` with
fresh demand/lease/rights. Demand changes may be observed during a pending command, but no
second desired mutation for that symbol overtakes its ACK. Zero→positive after an actual
unsubscribe closes the current epoch and requires a fresh epoch/ACK; it never resurrects the
terminal facade or restores old proof from DB scalars. If all demand is gone, close directly
within5s rather than spending30 unsubscribe commands.

Cancellation for a lifecycle stop is terminal for the owned facade. Signal cancellation,
abort if its bounded shutdown expires, await the task's termination, then permit lock release/
replacement; never cancel on every timer tick. A cancelled command retains durable ambiguity.
Only the existing transport's definite clean-close path clears its own clean epoch. A TCP
error, task panic, cancelled write, or prior `current_epoch`/pending attempt left on disk is
STOPPED, not permission to call reconnect. Clean reconnect uses fresh `connect()` and the
existing shared has_connected/backoff accounting (10/20/40/60s, >=10s, 6/10min,20/day);
positive jitter stays within0–20%. API/tab activity cannot reset these histories.

Production day resolution is private `resolve_day(owner, windows_source) ->
Result<DayResolution, MarketStreamRuntimeError>` in the runtime; `DayResolution` is
MissingCalendar | MissingWindow | Closed | Open(ResolvedMarketStreamDay). Reuse the existing
current-day calendar resolver's exact batch/version/hash/36h checks and pinned operational
window loader; do not acquire a calendar or infer a weekday. Load window input anew at date
change. The private resolved value binds calendar lineage, half-open UTC open/close, KST date
and transport proof. `session_proof_id` is a correlation UUID for that resolved lineage, not
evidence; retain it for same-lineage reconnect. `session_proof_sha256` is SHA256 over UTF-8
lines, ending in LF: `owner-market-stream-day-v1`, YYYY-MM-DD, canonical calendar batch UUID,
lowercase calendar hash, prefixed window hash, UTC open and close in RFC3339 seconds with Z.
Storage rechecks the calendar lineage under publication locks and the pinned half-open window
before epoch/quote commit; syntactically valid caller fields alone are insufficient. A private
window value accompanies runtime publication context; no new public proof factory is added.
At close or date change, purge private receipts/proofs, stop old epoch publication, and resolve
the next day independently. Missing inputs persist typed unavailable state, expose no prior-day
quote and produce zero approval/WS/calendar calls. WP-6 supplies reviewed artifacts; WP-9 proves
the actual next-day supply procedure. This amendment does not supply those proofs.

#### D3-B supervisor lifetime and observation refinement (coordinator adopted, 2026-10-02)

The daemon constructs one runtime repository adapter for its entire run and shares clones with
all observations, renewals, epoch start, socket owner and writer. A terminal latch is never
replaced to permit a reconnect. At most one local observation cycle and one lifecycle stage
are in flight. Each cycle concurrently completes the exact current grant, demand and day
reads and a due lease renewal using the existing one-second adapter; all started operations
are awaited, including on a sibling error. A cycle is started at one-second spacing without
catch-up bursts. Due renewals use their independent five-second cadence. An additional real
renewal in the post-claim/pre-connect validation cycle obtains current gap metadata; status
results must never be copied into lease/epoch proof scalars.

Connect, resolved epoch start and the owned producer future remain pinned across observation
ticks. The supervisor does not spawn an outer producer task or impose an outer timeout that
could drop the producer while it is joining children. Once the owned producer exists, it is
driven through its existing cooperative five-second drain, abort if needed, and ALL child
joins. A completed stage also waits for any already-started observation before the next
stage/replacement. Successful unchanged demand observations are forwarded as fresh events;
cached observations are never re-stamped. The supervisor continues real demand observation
during a known-safe drain. A terminal DB result instead latches stop, does not restart
observations or DB work, and still awaits owned producer cleanup.

Renewal authority is slot/grant/revision/owner/holder/fence plus monotone heartbeat/expiry and
more than five seconds of remaining life. A lower gap generation is terminal. A higher gap
generation with unchanged authority stops the old incarnation without forwarding the changed
lease into its old epoch proof. Day lineage changes likewise request drain, never a proof
rewrite. Any retained parent control lease is database-issued and is discarded before a new
claim. Only a known clean producer result, all joins and known retirement permit a fresh
incarnation; source/pure checks alone do not establish that runtime behavior.

Initial zero demand does not reserve the connection anchor or claim a producer lease. Positive
demand permits anchor then claim, followed by a fresh post-claim grant/demand/day/lease check
before connect. Missing/closed day evidence allows an anchor/control lease only for typed
unavailable status and renewal, with no epoch/provider call. The one safe waiting outcome from
connect is the existing State(ReconnectNotReady), which occurs before durable attempt mutation
or approval; it cannot reset accounting and may be checked at most once per observation tick.
Other connection, epoch-start, deadline, unknown-commit and ownership failures are terminal.

If a lifecycle stop is observed during connect, keep the one connect future until it finishes
or the five-second stop deadline. Cancellation at that deadline is terminal ambiguity, never a
reconnect allowance. A successful but no-longer-eligible session is consumed by its existing
bounded clean close before any epoch start. If resolved epoch start succeeds after a stop was
latched, immediately run/drain that owner with stop already set and await its complete cleanup;
never discard an owned producer that needs retirement. Parent stop cause distinguishes actual
daemon shutdown from an internal child shutdown request for a fresh incarnation. Only external
shutdown/closed shutdown sender plus known cleanup returns the public Shutdown exit; other
definite clean outcomes return to local waiting. Errors use finite redacted variants.

### 13.3 Private coalescing and committed status/snapshot boundary

Move authentic receipts, without reconstruction or restamping, into <=30 private latest slots
keyed by current identity/epoch. Retain only greatest valid receive ordinal; count replaced,
stale and rejected observations separately. A 250ms scheduler moves at most30 into one private
publication batch, with at most two batches total including in-flight work. If the writer is
busy, merge into the latest slots instead of enqueuing more work. One serial writer enforces
>=250ms between quote transactions (also after delays, with no catch-up burst), <=4/s and
<=120 row updates/s. A batch uses existing `publish_stream_latest` and committed subscription
proofs; no per-symbol transactions. Pending commands, identities removed/replaced, old epoch,
expiry, rights loss or day change invalidate affected buffers. C1 locks recheck everything at
commit; the existing exact reread is the only recovery of an unknown publication commit.

The transport's existing <=128 event queue and bounded message are separately counted in the
<=8MiB workspace, never multiplied per consumer or used as a general queue. The new latest/
batch buffers still obey30/2×30/64 status limits. No runtime public channel/facade/API emits
uncommitted values. A slow writer never blocks receiving forever: bounded error closes the
owner, while no memory value substitutes for a committed row. Routine status transitions
coalesce at1Hz; urgent lifecycle/revocation control may bypass that cadence, within bounded
control rates. Never silently discard a revocation to stay within64 statuses.

Keep ACK-only `record_stream_status(context, status)` unchanged for quote-attached state.
Add **private** repository methods (all `pub(super)`, no parent export):

```text
async load_runtime_grant(config: &OwnerMarketStreamRuntimeConfig)
    -> Result<RuntimeGrant, MarketStreamStorageError>
async record_runtime_status(lease: &StreamProducerLease, expected_epoch: Option<Uuid>,
    transition: RuntimeTransition)
    -> Result<RuntimeStatusCommit, MarketStreamStorageError>
async retire_stream_producer(lease: &StreamProducerLease, expected_epoch: Option<Uuid>,
    reason: StreamStatusCode)
    -> Result<RuntimeStatusCommit, MarketStreamStorageError>
```

`RuntimeGrant` compares configured grant/slot/generation/contract pin with the actual immutable
grant row, exact30 hash and wire contract; there is no synthetic production default. Transition
constructors are private state-machine operations, not arbitrary availability/quote setters.
Status uses producer→rights→owner-capacity→admission/session/demand→subscription→cache lock
order as applicable. Fence and expected epoch are exact; stale workers cannot mark a new socket
stopped. Nonlive control does not require an ACK or invent session proof. `retire` clears only
the matching DB epoch after socket closure, marks subscriptions nonlive/ABSENT and notifies;
it does not clear protected transport state or reset command history. If rights/DB prevent a
write, close anyway; RLS/read checks and expiry force nonlive rather than bypassing rights.

A narrow **append-only0056 migration** adds producer `status_code` (nullable existing
closed reason enum), `status_at` (timestamptz), `gap_since` (nullable timestamptz), and
`session_has_gap` (boolean, initially false), associated constraints and worker column grants.
Keep all existing tables, role identities, C1 lock functions and FORCE RLS. Do not reset the
monotone gap_generation counter. A transition opening a new gap advances it once, repeated
same-gap status is a no-op; reconnect does not double-count that gap. Same-day recovery clears
gap_open only after committed current-epoch data, retains session_has_gap; independently
validated next-day start resets the flag. Per-row gap stays open until that row's fresh quote.
Claim/retire preserve the prior proven session date and gap metadata for this comparison while
clearing live epoch authority; a new holder must not erase same-day gap history by clearing
lineage first. The first proven epoch is not by itself a gap. Stale proof metadata never grants
publication; only the newly resolved day and committed epoch do.
Persist producer changes and cache overlays in one transaction; status-only commits never
increment quote_version or change quote capture. Add only credential_generation,
network_contract_sha256 and identity_list_sha256 to worker's existing grant SELECT columns.

App still receives **no direct producer/subscription/grant SELECT or write grant**. Add one
bounded SECURITY DEFINER read helper `owner_market_stream_delivery_state(uuid,text,uuid)`
(owner, canonical session hash, lease), returning <=30 authorized lease identities and only
their subscription state/revision/updated_at plus producer connection/reason/status_at/heartbeat/expiry/current
epoch/day/proof hashes/gap flags/gap_generation/state_version. It returns no slot, holder,
fence, entitlement bytes or session hash. Owner/session GUC equality, canonical session, active
lease, rights, READY latest generation and date are checked inside; invalid access returns no
rows. Fix search_path, qualify relations, revoke PUBLIC EXECUTE, grant only app, and use the
existing migration's controlled function owner. Missing producer/proof yields typed nonlive
metadata, never invented proof/quote. Down migration drops the helper before dependencies.

Extend safe `read_stream_snapshot` (same arguments) to read delivery state and cache in one
consistent actor transaction. Return every authorized demanded identity, including never-quoted
rows, with nullable committed cache and committed delivery metadata; WP-4 maps this to the
unchanged schema2. An absent cache uses the committed lease identity namespace (`row_generation`
= lease UUID, state_version="0", quote=null); first cache creation replaces it with the actual
cache namespace/version. Snapshot DTO adds a safe StreamDeliveryRow (identity, optional cache,
subscription state/revision/updated_at and the above producer metadata); these internal fields
do not add HTTP fields. API delta detection compares committed producer/subscription metadata
as well as cache versions. Clarification for WP-4/5: row state_version identifies cache state;
apply connection/subscription/freshness overlays in ordered SSE event_sequence even if that
cache version is unchanged, while quote replacement still obeys epoch/quote_version. Reject an
older event sequence. API status heartbeats do not fabricate row version increments. Producer
lease remaining <=5s, heartbeat age >10s, unhealthy/current-epoch mismatch, invalid day/window
or no ACK overrides cached
LIVE. Notification remains transaction-local `owner_market_stream_changed` payload `v1` only;
WP-4 LISTEN-before-snapshot + <=1s reread, auth revalidation and reset semantics remain required.

The source baseline ends with `0055_owner_market_stream.{up,down}.sql`; its production status
is unknown. The coordinator selected `0056_owner_market_stream_runtime.{up,down}.sql` above
instead of editing0055. Its down file removes only its own helper, column grants, constraints
and columns, preserving0055 behavior and privilege boundaries. Current production application
or rollback remains outside the source/local units. All0001–0055 sources remain immutable.

## 14. WP-1 one-shot production initializer contract — coordinator adopted, 2026-10-01

This replaces only section9's unspecified first-provisioning implementation. Runtime
`MarketStreamDomain::open_production(slot)` remains validate-only; all existing reopen,
uncertain-state, anchor-inode and no-repair rules remain mandatory.

WP-3 owns `crates/kis-client/src/market_stream_provisioning.rs`, a separate opt-in
`market-stream-provisioning` feature, and binary
`crates/kis-client/src/bin/kis-market-stream-state.rs` (required-features set). WP-6 builds/packages
that binary from the exact reviewed release and invokes it through the official provisioning
helper. It must not implement JSON state writing in shell or enable test-support in the image.

```text
kis-market-stream-state initialize-new --credential-slot-id <uuid> --credential-generation <u64>
kis-market-stream-state validate-existing --credential-slot-id <uuid> --credential-generation <u64>
// Feature-gated library entry points; typed inputs, fixed production paths only.
initialize_production_market_stream_domain(slot: Uuid, generation: u64)
    -> Result<ProvisionOutcome, StateError>
validate_production_market_stream_domain(slot: Uuid, generation: u64)
    -> Result<(), StateError>
```

No URL, secret, approval key, owner override, path override, state JSON, reset/repair/force flag
or runtime setter is accepted. UUID must be nonnil/canonical and generation canonical positive
u64, equal to existing configured KIS_READ_CREDENTIAL_GENERATION. `initialize-new` requires
real/effective UID0 in the official installer context; daemon runs UID:GID10001:10001 and
cannot invoke initialization. `validate-existing` permits root or that runtime UID/GID and is
read-only. Output is a closed success/error code, never state bytes or provider messages.

To prove creation freshness without a reusable receipt/factory, **this one invocation creates
both previously absent leaves and their final anchors itself**. WP-6 provisions only the trusted,
persistent root parent, validates the immutable commit/image and explicit first-install mode,
and gives the initializer fixed `/run/lagrange` paths in a bounded network-disabled one-shot
root process. In a container this is a parent bind mount from the configured persistent runtime
state directory; no other child is read/modified. Runtime containers later get only section9's
two leaf mounts, anchors read-only. WP-6 must not pre-create either WS leaf. Reapply always calls
validate-existing, even if a state file or entire leaf has disappeared. An existing domain in
the install record cannot be relabeled first-install after loss.

Initializer checks both leaf names absent using descriptor-relative no-follow operations;
either existing/partial/symlink layout fails with no repair. It creates state UID:GID10001:10001
0700, anchors directory root:10001 0750, two distinct single-link zero-length anchors root:10001
0440, fsyncs objects/parents, revalidates their final device/inode identities, and acquires
connection then state lock. Root ownership/chown is confined to these newly created objects.
It builds the existing schema with actual slot/domain binding and configured generation,
no key/epoch/pending command, empty histories and has_connected=false. It writes one0600 temp
file in that leaf, sets UID:GID10001:10001 before fsync, verifies descriptor/link/mode/size,
then atomically installs the complete file with **no replacement** (e.g. renameat2 NOREPLACE),
fsyncs the directory and reopens through the production validator. Existing runtime atomic
replacement writes remain unchanged. Success requires every fsync and final validation.

Crash, fsync failure or partial layout is an uncertain failure, never successful initialization
or an automatic reapply retry. Do not delete partial evidence, repair anchors or reinitialize
missing/zero-length/corrupt state. Reapply verifies exact slot/generation/domain/inodes and
metadata, preserving file bytes, budgets, ambiguity and anchor identities. The library keeps
serialization, fresh-layout token and state write helpers private; default consumer builds expose
neither provisioning functions nor synthetic factories. WP-3 proves filesystem/CLI invariants
on disposable fixtures; WP-6 separately proves actual root/runtime ownership, mount, immutable
packaging and reapply behavior. Neither test category proves production provisioning happened.


## 15. Initializer release packaging — coordinator adopted, 2026-10-03

The existing initializer source and section14 behavior remain unchanged. The job-queue
package may expose a required-feature binary target named `kis-market-stream-state` pointing
to the same reviewed CLI source. Its opt-in `market-stream-provisioning` feature forwards
only to `kis-client/market-stream-provisioning`; default consumers still exclude provisioning.
The D4 immutable build recipe alone selects that feature and packages the daemon plus this
initializer in the existing runner image. Every request, artifact receipt, bundle, and consumer
guard binds the same feature selection; all other recipes retain their empty-feature contract.
No test-support feature, new service, automatic initialization, activation, or live permission
is introduced. One package/bin producer at a time and the existing production resource policy
remain mandatory. Runtime entrypoint, healthcheck, and UID10001 do not change.

## 16. Provisioning command boundary — coordinator adopted, 2026-10-03

`scripts/ops/provision-owner-market-stream.py` is the separate installed-release command for
section14. The existing `provision-linux.sh` supplies only the protected parent; it does not
pre-create either WS leaf. The new command defaults to plan mode without installed-input reads.
`--initialize-new --expected-commit <40hex>` and `--check --expected-commit <40hex>` require the
current root-owned release, its exact protected env and complete V2 image manifest, mode off,
shared coordination, the canonical slot/generation, and no running runner container. The
local image ID and revision must match; environment overrides cannot select another daemon,
release, image, path, slot, or generation.

Before first invocation a root-only exclusive, fsynced installation record is created outside
the two leaves. An attempted/partial installation remains an incident and cannot be retried or
relabeled first-install after state loss. Only the fixed Rust initializer writes domain state.
Its one-shot container has no network, no healthcheck, read-only root filesystem and bounded
resources; initialization runs as root with only CHOWN/FOWNER/DAC_OVERRIDE added, then validation
runs as10001:10001 with no capabilities and a read-only parent bind. Container cleanup verifies
its exact random operation label, name, image, and ID before removal. An uncertain cleanup is
an error, never silently treated as absence.

Successful initialization records the parent, directory, and anchor device/inode identities.
Reapply always validates, preserves the record and state bytes, and rejects missing/empty/unsafe
or replaced anchors/state. It cannot repair or reset. Synthetic orchestration/metadata tests
are separate from the still-required actual root UID, mount, packaged-image and Rust initializer
verification. This source contract authorizes no production provisioning or activation.

## 17. Grant-install input and transaction — coordinator adopted, 2026-10-03

The separate installed `install-owner-market-stream-grant.sh` command defaults to plan mode,
which reads no approval/installed/protected files and invokes no Docker or DB command.
`--apply --expected-commit <40hex> --approval-input <path> --approval-sha256 <64hex>` requires
one coordinator-reviewed root:root0600 regular single-link file under trusted ancestors.
The hash must match its exact canonical UTF-8 JSON bytes (sorted keys, compact separators,
literal UTF-8 for non-ASCII characters, one trailing newline, no duplicate keys/nonfinite
values/unknown keys). Unicode-escaped alternatives are not canonical. The artifact is an
operator input; the helper never creates an approved input or infers approval from a flag.

The exact keys are `schema_version` (1), `scope` (`owner-market-stream-grant`), `grant_id`,
`grant_revision`, `credential_slot_id`, `credential_generation`, `owner_user_id`,
`entitlement_id`, `entitlement_reference`, `entitlement_document_sha256`, `tr_id`,
`wire_version`, `network_contract_sha256`, `identity_list_sha256`, `effective_from`,
`effective_until`, and `activation_commit`. UUIDs are canonical and nonnil; generation is
canonical positive u64 text; dates are real inclusive ISO dates. TR/wire and exact30 identity
hash must equal the existing 0055 constants. The activation commit equals the executing
installed release; the slot/generation/grant/contract match its protected nonsecret settings.
The protected env file must contain that exact commit; shell fallback cannot supply it.
No credential, account, provider response, or free-form approval field is accepted.

The existing db-migrate service/secret/role path is used, with the installed V2 manifest's
exact local image ID. A temporary private Compose override resets `build`, sets pull never,
and disables its healthcheck. One explicitly named/labeled one-off container runs psql; no
dependency/service is started, no image is built or pulled, and the migration entrypoint is
not invoked. Every attempted operation reconciles container cleanup. Removal requires the
exact full container ID, actual image ID, name and operation label; both ID and name must be
absent afterwards. An uncertain DB outcome or failed cleanup is an error with no automatic
retry. The secret stays inside the container as in db.sh.

One bounded transaction requires migration_owner, serializes the slot, locks the selected
grant then entitlement, and rechecks the exact ACTIVE entitlement/reference/hash/date window
and Owner role. It inserts only the grant's immutable reviewed fields as ACTIVE. Exact ACTIVE
replay compares every canonical artifact field and is a no-op; a revoked/mismatched replay or
active slot/owner conflict fails. Constants plus the exact immutable field comparison determine
the same canonical approval hash without adding a table column. `--apply --revoke --grant-id`
only performs the existing one-way transition for that row; already REVOKED is a no-op.
Revocation requires the verified current installed release/image and an explicit grant ID,
including from an off release without WS settings, and consumes no new approval artifact.
No existing entitlement, admission, cache, role, or schema is changed. Actual-role transaction
tests and G1–G5 approval remain separate from source and synthetic command tests.
