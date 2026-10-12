# R1 WP-5R independent Rust lint A/B review — 2026-10-12

**Source behavior review passes; no demonstrated product/security regression. One P3 byte-preservation contract discrepancy remains.** This review covers all23 accepted A/B files and all112 canonical diagnostic groups (A74/B38), against the accepted A1–A14/B1–B8 contract and HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`. It does **not** grant overall R1 approval or dynamic acceptance. Strict matrix, item-lint propagation/fulfillment, DB-gated tests and runtime checks remain with validation worker4807300d; no execution slot or child worker was used here. Earlier accepted repairs are retained rather than reopened.

## Findings, in priority order

**R-OBS1 — P3, exact SQL-text preservation differs in B8.** `crates/api-server/tests/service_status_support/mod.rs:279-282` removes four leading spaces from each of the three continuation lines inside the audit SQL string while moving the existing query out of the async-move block. This is the sole raw SQL-literal difference found among433 SQL-keyword-led literals in the23-file HEAD comparison. SQL tokens, audit predicates, actor bind, ordering, pool and awaited execution remain identical; there is no source evidence of an SQL behavior or security change. Nevertheless the contract's no-SQL/text-preservation rule and B's “SQL text and bindings … were preserved” claim are not byte-exact. **Minimal remedy:** restore those four spaces inside each of the three string continuation lines, preserving the new async fn and all surrounding code. Alternatively the coordinator must explicitly record acceptance of this whitespace-only variance before claiming byte-identical SQL. This reviewer changes neither source nor B's report. The current literal-token SHA-256 is `972a0cb4fd6455a121cc5ddd393cf19a647385aa043dfc95bdc402f66a522b8d`; HEAD's token is `d1ea0daf3afe6facfd86198aa9a49756d085320ac6f3545efde27b57e0bcd50c` (including quotation delimiters).

Blocking behavior/security findings: **none**. Unexpected additional suppression, dropped safety check, lost test/assertion, SQL bind/type/nullability change, production feature widening, public wire/API change attributable to A/B: **none found**. The C-WIRE unexpected_cfgs item's effectiveness is expressly a dynamic acceptance question below, not an invented source defect. No architecture change is requested.

## Scope and inspected hunks

Read the complete accepted contract (350 lines, including ownership/hash/group ledger), A/B implementation reports, inventory report/JSON and prior independent acceptance `2026-10-12-r1-review.md:531-701`. All15 A current hashes match A's final table. B supplied no final table: the current eight B hashes were pinned at review start and are included below. The23-file HEAD diff includes earlier accepted G/D3/private-error/row/boxing/formatting repairs; contract pre-edit hashes equal HEAD in nine files and represent an already dirty source in the other14. Accordingly the complete HEAD diff was inspected while new A/B repairs were compared with the finite item contracts, without treating prior accepted changes as new unauthorized work.

All changed hunks were inspected, including formatter-expanded producer-test hunks and the retained earlier changes. The table gives the envelope of **current-line zero-context hunk ranges**, plus the number of such ranges; it does not claim the entire envelope was modified or that every unchanged whole-file body was reread. Relevant adjacent definitions/callers/cfg/guards were also read as described below. Total425 zero-context hunks; the default-context scoped diff contains5638 lines.

| Contract | Exact repository-relative file | Current hunk envelope; count |
| --- | --- | --- |
| A1 | crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs |36–2029;39 |
| A2 | crates/job-queue/src/bin/owner-equity-v2-runner.rs |856–1552;5 |
| A3 | crates/job-queue/src/owner_equity_v2/market_stream.rs |123–6793;29 |
| A4 | crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs |3–2810;6 |
| A5 | crates/job-queue/src/owner_equity_v2/market_stream_producer.rs |12–5696;44 |
| A6 | crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs |5–2008;11 |
| A7 | crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs |129–7405;138 |
| A8 | crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs |110–2040;14 |
| A9 | crates/job-queue/src/owner_equity_v2/market_stream_runtime_storage_tests.rs |236–1611;6 |
| A10 | crates/job-queue/src/owner_equity_v2/market_stream_runtime_test_support.rs |32–1219;9 |
| A11 | crates/job-queue/src/owner_equity_v2/market_stream_runtime_tests.rs |257–278;2 |
| A12 | crates/job-queue/tests/owner_market_stream_boundary.rs |5 (deletion boundary);1 |
| A13 | crates/kis-client/src/market_stream_approval.rs |508;1 |
| A13 | crates/kis-client/src/market_stream_provisioning.rs |405–556;2 |
| A14 | crates/kis-client/src/market_stream_wire.rs |211–986;6 |
| B1 | crates/api-server/src/http/owner_market_stream.rs |28–727;19 |
| B2 | crates/api-server/src/http/owner_market_stream_delivery.rs |52–645;17 |
| B3 | crates/api-server/src/repos/service_status.rs |373–999;7 |
| B4 | crates/api-server/src/runtime.rs |306–307;1 |
| B5 | crates/api-server/src/service_status.rs |1030–4551;19 |
| B6 | crates/api-server/tests/owner_market_stream_http_support/mod.rs |13–1922;29 |
| B7 | crates/api-server/tests/owner_market_stream_integrated_support/mod.rs |28–2045;18 |
| B8 | crates/api-server/tests/service_status_support/mod.rs |274–287;2 |

## Argument, identity, response and SQL preservation

Independent read-only lexical argument parsing compared the HEAD positional calls with current records after flattening only the accepted argument group. It retains literal contents, identifier/path/punctuation order and shorthand field values; it is a source comparison, not a Rust compiler or lifetime proof. All11 FixtureRequest and six ActorSessionContext records match all original arguments in order. The one EodRuntimeInputs call, one producer loopback clock tuple, production encode call and all11 source-test encode calls likewise match.

B1 `owner_market_stream.rs:87-181,226-303,597-646,658-734`: only configuration/key private error types are Box<Response>; all original error construction/status/code/body/request-ID expressions remain, and three configuration/two key callers return *r. The async body helper and public handler signatures remain unchanged. StreamDriveIdentity contains the exact ApiState and Arc<SessionIdentity>, destructures before the same driver body, and receives the same two ordered clones at spawn. Remaining repo/listener/lease/read_gate/mailbox/end arguments, mutability,250ms/1s watches, unlisten, reauthorization/backpressure and cancellation ownership stay intact. Private boxing adds the agreed allocation; response contents/owned body are returned unchanged.

B2 `owner_market_stream_delivery.rs:50-61,97-101,151-210,310-642`, production caller `owner_market_stream.rs:572-582`: replace_pending remains pulse>last_pulse.saturating_add(1); heartbeat remains the existing variable; injected now() is evaluated before those fields in the same position. The11 test flags at record lines319,341,368,381,399,434,447,467,585,603,635 are respectively FF,TF,FF,FT,FF,FF,FF,FF,FF,FF,FF. ConsumerPermit keeps mutex scope, saturating decrement and remove-at-zero. Only LatestMailbox::pending is newly cfg(test); its existing test remains, including the independent source-included delivery target. No encoder body/event/reset/cursor/byte-bound assertion changes beyond the argument grouping.

B6 `owner_market_stream_http_support/mod.rs:519-547,621-728,772-778,847-1114,1253-1834,1909-1924`: request helper still builds the same host/content-type/origin/cookie/CSRF/key/body; send retains operation_deadline, router.clone().oneshot and await/error order. Temporary &format! paths remain borrowed in the immediately awaited call statement; there is no stored borrowed request or task spawn. Every record is accounted for:

| FixtureRequest line | Unchanged request/safety input |
| --- | --- |
|621 | POST stream-leases; supplied session/CSRF/key/quotes origin; bytes |
|723 | DELETE same lease path; supplied session/CSRF/key/quotes origin; body |
|1256 | Invalid-lease POST; supplied session/CSRF/key/quotes origin; bytes |
|1283 | POST with no session/no CSRF, named key, quotes origin, not-json body |
|1302 | Member POST; CSRF, member key, quotes origin, valid_bytes.clone() |
|1321 | Owner POST with CSRF=false; named no-CSRF key; same body |
|1340 | Owner POST with attacker origin, unchanged key/body |
|1359 | Owner POST without idempotency key; same origin/body |
|1396 | Owner POST with unexpected query; same valid body and named key |
|1523 | Owner oversized POST; same CSRF/key/origin and oversized buffer |
|1826 | GET wrong stream query; owner session, no CSRF/key, quotes origin, empty body |

ActorSessionContext records at1008/1037/1054/1071/1087/1103 retain actual actor/session separately from desired owner/session: exact match; immutable-binding mismatch loop; wrong desired owner; wrong desired session; wrong actual actor GUC; wrong actual session GUC. The six call expressions include the original slot/grant/source-hash loop, not six reduced runtime cases. Destructuring aliases actual fields back to actor_guc/session_guc; set_actor and set_config precede the original helper query whose five binds stay owner/session_hash/slot/grant/source_hash, followed by the same commit. Every denial and no-read assertion remains. Unit-binding removal retains the full timeout/await/?? and subsequent terminal EOF; SSE let-chain preserves framing/JSON validation.

B3 `repos/service_status.rs:370-385,773-788,899-1008`: only set_actor_guc uses autoderef; SQLx executors still use &mut *tx. actor_uuid is evaluated once via .ok(); owner None retains all denied/no-read paths, owner Some retains the same pool/actor/session/time calls. StreamPinsRow preserves all six original optional element types/order. A4/A9 aliases preserve six-element ACK state, optional three-element capture, six-element producer state and seven-element retired lineage, with fully qualified chrono types where required. A5's private AdmissionMetadataSnapshot and B5's seven-element ScheduledFact preserve return expression/types/order and every observation/assertion.

All433 keyword-led SQL literal tokens were compared in occurrence order across the23 files. All are byte-identical except the single B8 whitespace variance R-OBS1; all433 are whitespace-normalized equal. Physical hunk inspection found no changed SQL bind chain or predicate. The six previously accepted FromRow types in A3 were independently compared after undoing only exact sqlx(rename)/underscore annotations: all fields, ordered types and Option/nullability remain equal to HEAD. No field was deleted to satisfy dead code. B8 `service_status_support/mod.rs:274-287` retains actor_id bind, admin_pool query, audit action/order and borrowed harness future; all eight caller expressions in `tests/service_status.rs:1658,1995,2021,2049,2065,2158,2179,3399` still await it.

## Cfg, exceptions, retained tests and cleanup

**C-SHARED is applied exactly35 times**, each on a specified constant, field, struct, function or method, never an impl/module/crate. Normalized attributes all use all(test,feature="market-stream-db-tests")/allow(dead_code) and the exact accepted reason. Item declaration lines are56,64,72,177,210,371,396,404,412,424,723,745,789,830,906,928,943,977,1018,1128,1161,1176,1190,1210,1231,1256,1336,1347,1372,1442,1467,1490,1506,1538,1568. grant_revision is annotated once despite two warning contexts. The four-test loopback_lifetime_tests block at1936–2029 is byte-identical to HEAD after trimming only positioning whitespace. Direct supervisor return and !failures.is_empty preserve error/cancellation semantics; no fixture body/safety check is deleted. Job-queue's single shared inclusion and the two independent API inclusions remain present.

**C-REST** is only the Rest variant at runner1353–1363: cfg_attr(not(test),expect(dead_code)) and exact accepted reason. Negative test still constructs Rest (`market_ws_startup_pins_are_required_only_for_selected_daemon:1610-1630`); selector never activates it and existing rejection branches remain. No enum/module-wide replacement allowance.

**C-WIRE** is only the SessionProofError item at wire211–223 and synthetic_default item at293–301, before its existing cfg(any(feature="test-support",test)), with the exact two allowed lints/reasons. DateInvalid/WindowInvalid/MidnightInvalid remain unchanged; synthetic_default is not narrowed to test-only. API integrated source inclusion at support2048–2050 remains, with parser tests. ParseContext default/update rewrite retains the stale-epoch assertion. The previous private receipt tuple and its sole transport caller at `kis-client/src/market_stream.rs:1519-1528` still pass wall/monotonic receipt together in the original order.

A3 gates exactly seven private DB seams; register/clear/single/changed/status/snapshot/upsert callers are still enclosed by the existing DB-test modules or gated methods. Underlying gate storage and cfg(test) empty wait hook remain. A5 narrows only the TestGate control impl and five named DB accessors; TestGate fields/hook and ProducerTransportErrorProbe/collapse/take/default pure observation tests remain cfg(test). All15 real gate constructor sites, their installations/wait/resume and late-publication controls remain in the DB inner module; no default pure path is hidden. A8's private loader cfg(not(test)) matches its sole already-cfg(not(test)) caller/import; synthetic test snapshots do not load production files.

A6 `producer_test_support.rs:1185-1205,1283-1300` preserves the captured origin/Instant anchor, TestClock Arc, date/plans/channel/early-close inputs and spawned-task ownership. No new Instant/clone/await is introduced by the tuple. A10 preserves all timestamp captures, counter increments/reads and digest computation while underscoring only five unused observation fields; header_value still removes exactly one leading ASCII space and returns the same borrowed slice. A12 removes exactly six unused imports; its DTO boundary test remains. A13 changes only the three named assertion/borrow forms, retaining secrecy, symlink, private mode and partial-evidence checks.

A7 `producer_tests.rs:311-323,1254-1262,1685,1996,2303,3569,4088` preserves lock poisoning handling, terminal-is-none immutability, pointer-equality assertions and exact error/relay return types. expect_err operates on unchanged unit-success results; it adds no provider-payload formatting. Previously accepted D3 finite diagnostic mapping, local provisioning gates and arm-only fallback remain unchanged in behavior; whitespace expansion retains the nine Budget labels, observer consumption once and unavailable/redaction assertions. Source test-name multisets match HEAD in all23 files:280 lexical #[test]/#[tokio::test] declarations, none removed or added by this diff. This is not a run count or cfg-resolved collection; independent source inclusions compile copies.

A11 cleanup at runtime_tests275–280 still takes the database once, awaits cleanup_until at the same deadline, and retains the first prior error after close/join. B7 `integrated_support/mod.rs:997-1080,1136-1209` preserves daemon→API→approval→WS ordering, shutdown notifications, ownership takes, shared deadline/forced_abort, socket/file/state drops before pool/database cleanup, and later active-task checks. Each let-chain invokes the same awaited operation only after Some and preserves get_or_insert precedence. Removed wildcard was unreachable for the unchanged single-variant RuntimeExit; future variants require explicit handling. publication_failure is assigned in None/Err/Ok branches before use; initial validation, bounded encode/atomic publish, get_or_insert, final revalidation and primary→publication→resource/cleanup precedence are retained. read_private_file still uses O_NOFOLLOW/CLOEXEC/NONBLOCK, mode/UID/nlink/size checks and bounded take; mut removal changes no handle ownership.

C2 guard chain was inspected read-only at shared mod210–283,1640–1933 and c2_fixture103–151,168–252,255–420: paired exact binding path/digest, canonical no-follow/private owner/inode checks, hash/schema/run validation, Unix endpoint/foreign-PG-environment rejection, and actual system/data/socket/port/version/user/database identity verification before DDL remain. Both create and create_until still call the verified connect helper; generated-name/ownership/deadline cleanup checks remain. No C2 source change is attributed to A/B; separate coordinator pin/runtime work outside these23 files is not granted acceptance here.

B4 keeps default-off/invalid REST/non-Unicode ordering before window reads. B5 retains timestamp-future rejection, queue retain/push sequence, failure priority, stream min-validity bound, all_ok separately from contains("ok"), publication-blocked/rights/raw-only/due precedence, and duplicate window-failure exclusion. No terminal transition, assertion or independent branch is silently retired.

## Snapshot manifest and verification limits

All paths are repository-relative; complete-file SHA-256 at review start and end are identical. A15 entries also equal A's reported final hashes. B8 entries are newly pinned by this independent review.

```text
B  crates/api-server/src/http/owner_market_stream.rs  8d90f0abc2a2c48a0c6714857eee23975af6547d887a93d3b862262f27484f30
B  crates/api-server/src/http/owner_market_stream_delivery.rs  2b269bfb20f52021620585843b6411e9fa1694e8200cee772a8f8e4426e68509
B  crates/api-server/src/repos/service_status.rs  6bfd25bdbd7d639561c112efcf6916f662d5d8ac0467e24e32d95dbb57916593
B  crates/api-server/src/runtime.rs  e8c3def659d3ee1b9e052d9392015576e777882e57714d6ab649f7ec3968e0e2
B  crates/api-server/src/service_status.rs  d4a6b5cf29274ab9e3eb045d3597b97122e1e3dd05268a1b4e874d1fe50eeff9
B  crates/api-server/tests/owner_market_stream_http_support/mod.rs  cb9e8ea2a2a474f69de87491b9f6096f8e2c44395e0301fee513f8c3f3c46855
B  crates/api-server/tests/owner_market_stream_integrated_support/mod.rs  01d5b8a7098b80e3d779f536eee509c84241b020f56f116480b2a502282ce565
B  crates/api-server/tests/service_status_support/mod.rs  c73299423dbf8d498041bf07fda78d6ae8ce51da1a842cf6a2418f0a51599301
A  crates/job-queue/src/bin/owner-equity-v2-runner.rs  f21d135c11a7cbfd2180aef4067bfd8c2e1f7da0db75c78f28213069f9fdd3da
A  crates/job-queue/src/owner_equity_v2/market_stream.rs  11f1e2d7f1e95ef98f4092760710ccb6a6ce6ef0cc692140e2ae8104d0a9e84c
A  crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs  af67896c3d9c51a761b155e9f01710db0120e7dcfb3742a94f50b365994d2aed
A  crates/job-queue/src/owner_equity_v2/market_stream_producer.rs  ba394ab4f8b6b5d5eea8f0fc43bc0c6d3814121643afa5e7923c8ce3efede7e8
A  crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs  8695e035c73d8b0096f1e509e636a4bfa25c23f5e9a9f834cbc8fbb8410c7660
A  crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs  b966c213df98bbd060c2f67a80b383cccbfb961d524eca7800706fedc24ef96a
A  crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs  7d0eddfbcbec2a03901e402ac95c44c95a00e44f3a6c689fd3f99e12d9444984
A  crates/job-queue/src/owner_equity_v2/market_stream_runtime_storage_tests.rs  e446a1f1f43bb6515a388bd84debb01b8f2c0685a69f2ac741797b81fb05aa07
A  crates/job-queue/src/owner_equity_v2/market_stream_runtime_test_support.rs  1d3f97c7cb5536b9cfb09f8e5c027937c03e6344df990a27bc525c3aa073790a
A  crates/job-queue/src/owner_equity_v2/market_stream_runtime_tests.rs  9b09e01718b8a5d945aae302945cbafe6dfe27a9fc7d256321f451a72851169c
A  crates/job-queue/tests/owner_market_stream_boundary.rs  517c61a2e7bfd7d02db94cd7b5bfc69b114718287b0e7494e9b047f12e71fbdf
A  crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs  a438aa74bb4bffbacead26cc67a22a6684d80aa7e0b6bd544d2cec88a217d1ed
A  crates/kis-client/src/market_stream_approval.rs  02a93dc56bbe4bdd8382f2b4d4d9e3809a0be8dd71f2ff1eecd377647df5ff04
A  crates/kis-client/src/market_stream_provisioning.rs  a1fce11c93dbc8af474f8eba3788e55257842c8d9e5e6f79578a4d01eb057f08
A  crates/kis-client/src/market_stream_wire.rs  1dff19badfbbc05225cfdf861ad8a491b14a7b932187e52a50edc5d15a7aa476
```

Exact scoped-diff SHA-256: `fdcabfb84e995e37554bb4491732ad52ce80e829b0795e9127d3f53639e1a747`. Digest input: raw stdout of `git diff --no-ext-diff --no-color --src-prefix=a/ --dst-prefix=b/ -- <the23 table paths>`; no whitespace normalization. HEAD, all23 source hashes and this digest were rechecked unchanged at review completion. This pins the reviewed frozen source, not a commit containing the current worktree edits; modifications outside these23 paths are not part of this identity.

Inventory JSON SHA-256 `c0e29828a713e92a718c9ab4d9734a84813a4238555551d2dd79358181a54726` matches the accepted contract. Independent pathname/code/message/location normalization yields118 original/112 canonical groups, aliases57/58/78/79/80/118, matching A74/B38. Captured289 warning records/zero errors were diagnostic-cap evidence only; this review does not promote them to strict acceptance. No compiler/test logs from the active validation worker were consumed as a substitute for its final receipt.

Actual commands/checks: git rev-parse/diff/show; bounded source/report reads and rg caller/cfg/inclusion searches; standard-library Python file/JSON/hash/lexical comparisons and argument/row/test-name/SQL checks. All successful comparisons described above returned0. One end-snapshot output was initially truncated by a too-small output budget, so JSON parsing failed; the read-only hash command was rerun with sufficient output and matched all23. No missing check was treated as a pass. No Cargo/rustc/rustfmt/test/build/DB/Docker/npm/browser/provider/project-code execution, staging/commit or delegation ran. No operational configuration, credentials, provider response bodies or raw runtime logs were read or included. Memory finish is an operational subagent skip, not a durable finding save.

Diagram citation checks were read-only: component_architecture.puml217 now cites actual runtime anchor.connect1840; runtime_deployment.puml175 cites current API LISTEN693/695. The prior stale owned-connect citation has been corrected by coordinator; A/B adds no structural edge or feature/manifest change. Neither diagram was edited here.

Only writable output: this new `docs/reviews/2026-10-12-r1-rust-lint-review.md`; line span is reported in the completion message. Deviations from review action/ownership restrictions: **none**. Implementation byte variance: only R-OBS1 identified above. Prior accepted source hunks were not reopened without evidence. Web, six-line DB patch, migrations/CI and other worker reports remain outside this review.

Unread/missing scoped changed hunks, required definitions, call records or reports: **none**. Unchanged whole-file bodies beyond the cited context were not all reread. Additional caller requiring another argument conversion or unexpected item suppression: **none found**.

Unresolved/follow-up: coordinator disposition/minimal restoration for R-OBS1; validation worker's serial strict default/DB/all-feature/independently unified provisioning matrix, including API source-included copies and non-test runner expectation. In particular wire synthetic_default's item-level unexpected_cfgs allowance must be proved effective on the actual included target; if not, return to coordinator without broadening the exception. Require expected nonzero pure/DB test selections, no absent-DB early-return acceptance, real-role GUC/RLS/identity and rollback/deadline/panic/cancellation/runtime/resource/publication results on the pinned final source. A source hash change after this snapshot requires the corresponding bounded re-review. No further decomposition is needed for this source review.

Not verified: Rust compilation, formatter/lint success, expected lint fulfillment/propagation, feature-resolved test inventory, allocator/drop/cancellation execution, DB or provider/runtime behavior, hosted CI and final R1 acceptance. **No dynamic acceptance is granted.** Return idle; no review-owned child process/heavy slot remains.
