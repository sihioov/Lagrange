# R1 WP-5 bounded workspace repair source review — 2026-10-12

**SOURCE REVIEW COMPLETE — F1–F6/U2/U3 source ready for heavy validation; P3 citations corrected.**
This accepts source readiness only for the shared dirty candidate at HEAD
`ac0a08364788e6c2f234cf85025a8d975f362d31`. Compilation, DB, feature/doctest,
focused/workspace and overall R1 acceptance remain pending. No native worker was used.

## Findings, ordered by severity

No blocking defect was found in the eight reviewed source files.

**P3, coordinator documentation follow-up — `docs/diagrams/runtime_deployment.puml:111`.**
The retained `market_stream_state.rs:1513,1747,1754` citations have moved relative to the
anchor-creation evidence. Current1513 is metadata access,1524 is `create_provisioning_anchor`,
1758/1765 are connection/state anchor creation calls. Update that comment to1524/1758/1765
and locally render through the coordinator's allocated slot. The implementation report also
requests line109's1727→1738 update to the directory-creation call; current1727 still names
`initialize_provisioning_at`, so it remains a valid function-level citation. No topology change
or new edge is needed. This is outside the eight-source write scope and does not prevent their
focused validation; diagram reconciliation remains required before final acceptance.

## Reviewed scope and preservation

Read accepted family contracts in `2026-10-12-r1-workspace-followup.md`, the complete
implementation report, every owned new hunk, and surrounding callers/contracts listed below.
F2 and U1 were excluded as instructed: their moving sources were not read or hashed, and their
pending work is not a condition on these seven families. Roadmap context is R1-03/R1-08;
coordinator retains roadmap ownership.

- **F1:** `owner_equity_v2_db.rs:266-298`, helper callers :300-313/:330 and constants.
  `OwnerEquityV2Repo::list` reads the policy through actor transaction and fails
  PolicyUnavailable if absent (:153-171/:588-606). Migration0053 :504-534 provisions
  (100,261,121) on Owner grant. New initial/final actor assertions and empty memberships
  bracket an UPDATE-only owner predicate. Existing migration-owner GUC/commit helper
  `api-server/tests/common/mod.rs:575-590` remains. Absence cannot be silently inserted.
  The rest of tenant/capacity/publication test bytes are unchanged from the preimage.
- **F3:** `research_worker.rs:2009,2103,2168,2285,2346,4145` exact endpoint assertions
  precede scratch DB creation/spawn. Worker inputs :2040,2131,2230,2314,4155 now use55438.
  Only11 string substitutions; env_clear, loopback/roles, password-file handling, synthetic
  provider, timeout/recovery/replay/redaction assertions are preserved. Pure config example
  :1523/:1550 remains55432; prior recovery inputs :2509/:2885 remain55438.
- **F4:** `recommendation_publish.rs:226-258`, all_cash_output :58-95, universe selection
  `recommendation/compute.rs:28-99`, shipped YAML membership/floor, selector canonical fields
  :147-160/:204-225, Rust writer `recommendation/validate.rs:478-543`, and Python
  `nt/strategies/_common.py:175-181`. Live current universe and portfolio hashes are fixed
  literals; historic clone changes only the universe/self-hash and retains the historic golden.
  The hash under test generates actual values only. Existing provenance/universe sensitivity,
  shared rehash/output helpers and production serializer are unchanged.
- **F5:** `kis_range_normalization.rs:14,54-123,332-375`, all helper call sites including
  multi-window query :193. Default wrappers retain adjustment1. The InvalidQuery case commits
  adjustment0 into envelopes before Raw store; the separate caller-only mutation expects
  Store(InvalidBatchMetadata). Both require an empty normalized manifest. Production ordering
  `range_normalize.rs:748-772/:1078-1132` and Raw metadata equality
  `storage.rs:1474-1486/:1574-1585` support these distinct gates. No Raw gate is weakened.
- **F6:** `market_stream.rs:565-616` and `market_stream_state.rs:492-507` doc attributes only.
  Default compile_fail absence examples have concrete constructor arguments; test-support
  no_run presence examples type-check the same APIs without running network/filesystem work.
  Generic domain ambiguity is removed by concrete &Path in the adjacent stream example.
  Constructor cfgs remain any(test,test-support). All other security/no-Clone/no-Default/
  serialization/private-state docs and previously accepted code changes are byte-preserved.
- **U2:** `intraday_quotes_support/mod.rs:26-84,228-343`, endpoint helpers :95-160,
  root SQL :166-203 and cleanup :505-541. Error retains only typed stage plus optional bounded
  five-character alphanumeric SQLSTATE; Debug/Display contain no SQLx message, URL or SQL body.
  Root transaction acquires the exact shared `lagrange-test-role-bootstrap` advisory lock,
  executes existing SQL on that transaction, commits, then creates the scratch database.
  Same-root locking matches job-queue common :180-197 and collectors common :135-144.
  Scratch grants/migrations/pools, exact55438/C2 identity, generated-name guard and normal
  pool-close/drop cleanup remain. Existing endpoint/role helpers still provide stage-only
  errors where they already erase SQLx details; new raw-SQL setup errors preserve SQLSTATE.
  All direct setup consumers use unwrap/expect/Display, so no String `?` conversion was left
  broken by the new return type. Checked run_body, calendar wrapper :174-184, exact restart
  :1541-1548 and lifecycle wrapper :366-399, plus all nine support inclusions/imports.
- **U3:** `market_stream_domain.rs:271-296`, state identity :354-366 and validate_domain
  :699-704. Original anchor File/dev/ino survives unlink and recreation through both assertions;
  replacement dev/ino must differ, mode remains0440, InvalidState and unchanged persisted bytes
  remain mandatory. No retry or production binding change. Original inode reuse is unobserved.

## Frozen snapshot and independent checks

All eight after hashes match both before and after this review. Before identities were
reconstructed in memory and hash-proven against the retained manifest: five files match HEAD;
intraday support retains its accepted C1 visibility delta; reverting only new doc attributes
in the two KIS files recovers their exact preimage hashes, preserving all accepted prior work.
This reconstruction is not a fabricated contemporaneous source snapshot.

| Source | Reviewed SHA256 |
| --- | --- |
| `crates/api-server/tests/owner_equity_v2_db.rs` | `0bd6f078732d4f5a9cb021b0ab60de16f43e6aca2e4e3cf6673f43308ba262d3` |
| `data-pipelines/collectors/tests/research_worker.rs` | `43170279cac6bdcab71b61e883bd099bd5ca68a07d8b699b3b01859ae74a2d93` |
| `crates/job-queue/tests/recommendation_publish.rs` | `9c7ab3e8eb38b5b601805d82dc831eec974fb9ebae989fb76721990293edeb7a` |
| `crates/market-data/tests/kis_range_normalization.rs` | `3071599af56c502862c811ef128c25e7a3fc6f34dbff89626071ffac160cf66c` |
| `crates/kis-client/src/market_stream.rs` | `99bb7d33a88dd55302fd06974934d25b96a4cd9662093cf30538a75719b68d55` |
| `crates/kis-client/src/market_stream_state.rs` | `1a8f395d6bb1176878a3553435d89f215bc6f84d618c033af22a857d40e4dea2` |
| `crates/job-queue/tests/intraday_quotes_support/mod.rs` | `55b68cc21f18ede620e5a2a23aa8ae8c8a77358b1758a1b63ffd90ad10ba877b` |
| `crates/kis-client/tests/market_stream_domain.rs` | `f866178c8568be9431b0d575b4e00a6d219f72e87a3d4662568948e11b1bb45c` |

Evidence root: `target/r1-runtime-fxi6ltmv/workspace-repairs-20261012-01/`.
Before/after receipt hashes match98198a6b…005b65 and3f029d10…c0f0 from the report;
these retained manifests include excluded F2, but that source was not opened.
F4 script SHA256 `5b345a8909df31d908fb00dde2a845ab3f26763801a3735db96ad4151c45b6b4`.
Read its literal fields/member order and imports; no repository code or production hashing is
used. Current reference is2405 bytes/SHA256
`e66b576b95f9f7682557009297be7bc50ad8050ef714dcb877263b8a1c4bed58`.
Parsed/re-encoded retained literal bytes in memory; substituting only the fixed historic universe
ID yields2405 bytes/SHA256 `7f52444f0d5770c849f9e06038e7f2996229a0e98a4a0df82f1b625e3859d3e4`.
This is read-only reference corroboration, not rerunning the evidence script or a product test.
Reference summary SHA256 `355d9b58bc3149989073d1f918bf827e4c231bc543a393a28f87977d434d9135`.

Actual methods: Git HEAD/diff/preimage reads, rg/sed source reads, standard-library AST/JSON/
SHA256 and in-memory reconstruction. No source imports, Cargo/rustc/rustfmt/tests/Clippy,
DB/Docker/browser/provider/runtime/network operations. Worker-provided rustfmt/diff success
is retained evidence only; both captured logs are empty and were not rerun by this reviewer.

## Follow-up, deviations and explicit limits

Run the specified focused families in a separately authorized slot: complete F1/F3/F4/F5 targets;
default/test-support/all-feature KIS docs and complete/repeated exact U3; U2 four originally
failed targets at threads1 then2 on fresh owned QA, then nonignored harness concurrency coverage.
Require real setup/body assertions and cleanup, not missing-DB skips. Original U2 SQLSTATE/
statement and U3 inode history remain unknown; the lock and held-descriptor repairs are not
claimed as proven historical root causes. Coordinator diagram follow-up is recorded above.

Owned file: this new report only; no edit to the historical WP-5 report or any other source/report.
Source deviations: none within completed families. F2 remains coordinator-owned and excluded.
Unread owned new hunks: none. Missing required scoped inputs: none. Not found: original U2
SQLSTATE/statement and U3 before/after inode receipt. Not verified: compilation, focused tests,
doctest feature matrix, DB cleanup/concurrency behavior, overall workspace/CI/production/R1.
Further decomposition: none for this review; serial dynamic validation remains a separate task.
**INTERIM WORKSPACE REPAIR SOURCE REVIEW COMPLETE — actual idle, no owned processes/heavy slot.**

## WP-5 coordinator F2/U1 source review — 2026-10-12

**Findings by severity: none. F2/U1 source ready for focused validation.** This accepts the
adopted clarification/parser policy and scoped implementation, not current-candidate compilation
or DB/test success. Prior seven-family source readiness remains. Earlier P3 diagram citation
finding is resolved in source; PNG render remains pending and has not been counted as passed.

Read the complete `2026-10-12-r1-workspace-coordinator.md` and112-line factor diagnostic,
all new hunks against HEAD and relevant contracts below. Four frozen hashes matched before
and after review; HEAD remains `ac0a08364788e6c2f234cf85025a8d975f362d31`.

| Source | SHA256 |
| --- | --- |
| `crates/api-server/tests/risk_snapshot_seam.rs` | `8b259ee17fdb00569353126384604ce05cf4e07bb64465780f268492f526ba9c` |
| `crates/factor-engine/Cargo.toml` | `15e4372aa3a537677d14a011ea7dc7d659e9e2f2d4d13873ee483fcd75d10dfa` |
| `crates/factor-engine/src/owner_equity_v2.rs` | `2c8efca96f822a358e7352cbbfd91c1118ab68809587a5e323eb6496bc4d92a9` |
| `crates/factor-engine/tests/fixtures/owner_equity_v2_legacy_default.json` | `bcd6c1a7c35df5465a7892c7c3f9d65fe27efc82f6cf2e17518e5df467d44c33` |

### F2 — verified calendar reuse, conflict and replay

Inspected `risk_snapshot_seam.rs:17,41-89,465-600`. Existing raw_publication helper/callers
are unchanged; it commits Raw and derives PublicationBundle through verified from_raw.
Jan31 foundation is prepared before Feb3 unavailable. Calendar file hash/size/bytes/facts
must agree before assembly. Only the cloned Calendar PublicationFile (including actual path
and attached evidence), calendar_facts and bundle CalendarEvidence are replaced with the
foundation's verified objects. Feb3's other files, provider/mode, batch/date/retrieval identity
remain its own. Both RawPublicationFixture values retain their TempDir fields through publication
and replay. No Raw bytes/manifests, timestamps or source IDs are modified or fabricated.

The assembled sink fixture is explicitly identified; it is not claimed as the original from_raw
return. Initial unavailable publish requires Published and preserves Unknown. Third fresh Raw
batch has a source ID distinct from the only published batch and different CalendarEvidence;
its original four-file shape remains intact. The negative requires typed SinkError::Conflict
and unchanged data_batches/history/projection counts, with four published files and positive
calendar counts beforehand. It therefore cannot pass via empty-count assertions or replay
conflict against the existing unavailable batch. Source sink :1142-1174 inserts fresh rows before
history identity checking :804-956; returned Conflict rolls back that transaction. No sink change.

Prior real EOD still establishes Age(259_260). Final assembled unavailable exact replay requires
AlreadyPublished, unchanged counts and unchanged Age(259_260). These are newly specified
assertions: the original test had no final replay block. Read sink bundle consistency checks
:352-450 and persisted calendar/file identity :671-732/:770-956. Calendar evidence is accepted
separately from the publishing bundle identity, preserving the existing shared-calendar seam.
Required verification remains complete risk_snapshot_seam + collectors publication_sink against
fresh authorized QA, reaching these body assertions without missing-DB skip.

### U1 — explicit parser selection and unchanged legacy bytes

`factor-engine/Cargo.toml:17-19` adds only float_roundtrip to existing serde_json version1.0;
raw_value is not added. Cargo.lock remains SHA256
`d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e`.
The new golden is coupled to that adopted parser policy, rather than an unexplained hash swap.
Mechanically compared owner_equity_v2.rs through its cfg(test) boundary with HEAD: production
bytes identical. All old test/function names remain; new helper and legacy test are additions.
Read test section :1115-1217, existing canonicalization/verify :299-346 and rank/shape
contracts :375-415. Math, zero normalization, source/admission/version/rank checks remain.

Current roundtrip test keeps the original one-row fixture and adds the two-row golden, requiring
byte equality and strict verify. Legacy test pins4178-byte full-file digest and the old2e3f…
self-hash; direct from_slice and Value/from_value routes must serialize to the original bytes
and verify. Neither route assigns a new hash to an accepted legacy snapshot. Shared tamper
helper rejects changed hash, one-bit average_volume change, and owner_only=false even after
rehashing that structural tamper. It executes for current and legacy snapshots; verification
is not replaced with rehash-based acceptance. The fixed legacy fixture matches the diagnostic's
`default-capture.snapshot.json` byte-for-byte, retaining old hash and synthetic000070/000071 rows.

### Independently inspected diagnostic evidence and compatibility limits

Evidence root: `target/r1-runtime-fxi6ltmv/factor-diagnostic-ryvs3zjg/`.
Verified report/run-inventory/log hashes, exact expanded argv and counts for all12 archived runs;
three exit101 golden failures remain diagnostic evidence, not current-candidate failed/passed tests.
Float-only reproduces unified hash change; raw-only preserves default. Cross-read receipts match
report digests: roundtrip reader accepts both stored synthetic producers unchanged in direct/Value
routes; default reader rejects new bytes with SnapshotHashMismatch. Bit receipt records two
−1ULP default parse changes and zero roundtrip changes; captured pre-parse bytes are identical.
Tamper receipts retain hash/numeric/structural rejection in both selections.

Exact HEAD tar SHA835c5e94…6d56 matches report; factor source/manifest/lock within it match Git
HEAD, and original-source receipt matches. Independently compared all1771 regular archive files
with that tar: zero differences. Inventory SHA29cb0429…35cf and cross-read hashes45b35dac…f58e /
3bb611c1…1701 match. The original diagnostic's unchanged-source starting hashes are separate
from this new candidate. No archived instrumentation was introduced into production.

Read job-queue publication :1142-1184 and API read :451-480: DB persists immutable header plus
individual row JSON; it does not store/reverify the full candidate content_sha256. No aggregate
DB rehash, migration or legacy fallback is introduced. Real historic payloads, JSONB/SQLx
roundtrips, V1 cross-reader compatibility and other platforms/toolchains remain unverified;
synthetic compatibility does not fill those inputs or justify rewriting old stored rows.

Diagram :109/:111 now cites1738 and1524/1758/1765. Read those exact current source lines:
state-directory creation, anchor helper, connection/state anchor creation calls. Topology is
unchanged. Citation P3 resolved; local PNG rerender pending coordinator's next free slot.

### Methods, follow-up and ownership

Actual methods: read-only source/diff/JSON/retained log reads, SHA256, exact argv/count checks,
Git preimage and tar byte comparison. One private parser check initially omitted digit-containing
v1 labels; it was corrected and all12 rows verified. No compiler/Cargo/tests/DB/Docker/browser/
HTTP/provider/network operation, source edit or heavy slot. Further decomposition: none.

Source/spec deviations: adopted F2 added-replay clarification and explicit U1 parser policy are
recorded, not invented by review. Unread required scoped hunks: none. Missing required source/
diagnostic inputs: none. Not found: real historical full-candidate compatibility fixtures.
Not verified: candidate compilation/factor default/unified/V1/tamper runs, F2 real DB targets,
actual historic/DB compatibility, PNG render, workspace/strict/smoke/hosted/overall R1 acceptance.
Owned writes: top status :3 and this append only. Prior body :4-129 preserved byte-for-byte,
SHA256 `e84d937f41a3b557b7100fb7657b6693cc66b5b0f6b9a4b9a24aec0a4c42d183`.
**INTERIM F2/U1 SOURCE REVIEW COMPLETE — actual idle; no owned processes/heavy slot.**
