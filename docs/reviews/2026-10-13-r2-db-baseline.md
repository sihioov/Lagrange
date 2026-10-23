# R2 database baseline probe

## Execution

- Observation window: 2026-10-13 14:32:48 UTC / 23:32:48 KST (host clock, immediately after the single runner invocation).
- Approved input hashes matched before execution: `db-probe.py` `8ac22ff72999cd00e6b1d1502eb4d51868594033b66df24182c2b670dced1e94`; `db-probe.sql` `e685570d8a5c17eb6d713388f7347d76b5eb8b38f5fb039d2ebcd578b55765df`; `query-contract.json` `577a1d3526e2d9c7a29326ba02d7dec3654b4578aa3ae06b49db1defc7ab829c`; `migration-source-hashes.json` `5793c369f863b0965dc8bb65936f2e4e75287360be4444d1351a4c53c2d67300`.
- Command: `python3 target/r2-daily-beta/wp-1c/db-probe.py` (invoked exactly once; stdout redirected to the private receipt; stderr discarded).
- Exit status: `70`.
- Receipt: `target/r2-daily-beta/wp-1c/db-probe-receipt.json`, mode `0600`, size 62 bytes, SHA-256 `142af49275405cef0817a97f5b9a1ba9f97aad08856df880f031e4ff7b863ccb`.
- Typed receipt status: `PROBE_REJECTED`; `executed_result_available` is `false`. No validated query result was produced. The rejected probe was not retried, and its rejection cause was not established from the permitted output.

## Database observations

No database aggregates are available from this execution. In particular, schema availability; migration checksums, pending migrations and index status; Raw EOD versus exact ETF publication and coverage; credentialed calendar freshness and gaps; OwnerBeta successful `as_of` and lineage counts; and V2 coverage, snapshot and jobs remain unverified. `AVAILABLE` or zero counts would not by themselves establish health; this receipt contains no such observations.

## Follow-up

The bounded WP-1C attempt is complete with a typed rejection receipt. Coordinator review is needed to decide any further action. No cause, remediation, catch-up request, or additional execution is inferred or authorized here.

## WP-1D bounded diagnostic follow-up

- Observation time: 2026-10-13 14:46:32 UTC / 23:46:32 KST. All four approved WP-1D input hashes matched before execution; inputs were in a mode `0700` directory with mode `0600` files.
- The approved command `python3 target/r2-daily-beta/wp-1d/db-probe.py` was invoked exactly once. Exit status: `70`.
- Private receipt: `target/r2-daily-beta/wp-1d/db-probe-receipt.json`, mode `0600`, 179 bytes, SHA-256 `c80d6cc01a5b238e7b254617799970fa74bdf07aac159c592ffbeaa005e27674`.
- Typed result: `PROBE_REJECTED`; `executed_result_available=false`; phase `DATABASE_PROCESS`; `database_attempted=true`; process exit `3`; SQLSTATE and SQL line are null. No cause is inferred. Outer stderr was discarded, not inspected or persisted.
- No validated aggregates were returned. Schema availability; migration checksums, pending migrations and index state; Raw EOD versus exact ETF publication and coverage; credentialed calendar freshness and gaps; OwnerBeta successful `as_of` and lineage counts; and V2 coverage, snapshot and jobs remain unverified.
- No retry or independent query was performed. Coordinator review is required to determine whether any further action is appropriate.

## WP-1E failure and WP-1F accepted aggregate evidence

Appended on 2026-10-14 KST by the bounded offline reviewer. All original 27 lines / 3,074 bytes above remain byte-for-byte preserved, including both WP-1C and WP-1D failures. They describe those failed attempts, not the subsequently successful WP-1F observation.

- WP-1E receipt SHA-256: `a11f2d8afae8db6be0967d0ff4c08568cbd82da1de0407c8f7234c4de7e77e84`. Typed rejection, no accepted result; `DATABASE_PROCESS`, attempted=true, process exit 3, SQLSTATE **42601**, SQL line **402**. The indexes JSON aggregate in my original SQL lacked one closing parenthesis at original line 398. That draft error is acknowledged; the previous lexical check was not a PostgreSQL grammar parse. Do not infer unrelated service failure causes from it.
- Coordinator corrected only that SQL parenthesis, then the SQL→contract→runner hash chain. Offline E→F comparison confirms the SQL's one-character correction and otherwise unchanged query-contract semantics. Prior C/D/E inputs and receipts remain separate. Coordinator reports one WP-1F execution, **exit 0**; receipt snapshot **2026-10-13T14:51:08.639884Z / 23:51:08.639884 KST**, READ ONLY/operator-visibility flags true, ROLLBACK marker true and pre/post runtime-identity flag true. These are recorded evidence, not a fresh host observation in this review (`target/r2-daily-beta/wp-1f/db-probe.sql:3–24,388–399,698–699`; `target/r2-daily-beta/wp-1f/db-probe.py:207–317,429–448`).

| Exact WP-1F input/output | SHA-256 |
| --- | --- |
| `db-probe.py` | `9992314eb6bbb0db631e7ae7042b392ae711762effa0adc2bc632c8f1f83f13b` |
| `db-probe.sql` | `82cdc98ac69f49564d1e24481ac6eeaad0d7f25a2df20205cb89ae85e9bf16a5` |
| `query-contract.json` | `ee62a4ef2243bf789be938fa5e0351db0941822910d137596ff5178859337d06` |
| `migration-source-hashes.json` | `5793c369f863b0965dc8bb65936f2e4e75287360be4444d1351a4c53c2d67300` |
| `db-probe-receipt.json` | `5f027fd93e56cd098dbb526f7c0b5c89a84c5b5ac973c74f380ececa8116318e` |
| `calendar-selection.json` | `105b5afa978f344c57a71ceb9459d604653a32b2194e11ca3544d22b9eeebe75` |

Offline acceptance of the successful aggregate, within the exact SQL predicates:

- **Schema/migration/index:** 161 prerequisite table/column/type checks pass; 59 actual successful SQLx versions/checksums match the frozen manifest and all 59 current local UP sources; pending/mismatched 0. Seven indexes pass existence/valid/ready/table/unique properties. Full index-column/predicate/operator-class equivalence is unverified. 0050 already applied; pre-0050 global row check is NOT_APPLICABLE (`target/r2-daily-beta/wp-1f/db-probe.sql:26–411`).
- **EOD versus ETF:** credentialed KRX/KR EOD metadata has nine dates **August 18, 19, 20, 21, 24, 26, 27, 28, 31**; no queried future/duplicate-date/incomplete-lineage rows. Latest selected ETF11 candidate publication is **August 28**, READY, exact required 11 coverage, valid dataset/one Raw binding; its exact dates are the preceding list **without August 31**, eight dates. EOD rows are not instrument-filtered; relational provenance is not physical file verification (`target/r2-daily-beta/wp-1f/db-probe.sql:412–513`).
- **Calendar:** 370-day October 9, 2025–October 13, 2026 window has 358 MISSING / 0 INVALID; 12 verified dates include ten TRADING and two CLOSED. Latest verified eligible TRADING date **October 7**; **October 13 missing/not fresh**. Filesystem current-day claim is UNAVAILABLE, not healthy/empty (`target/r2-daily-beta/wp-1f/db-probe.sql:517–558,694`).
- **Owner-beta:** two historical SUCCEEDED runs, maximum `as_of` **August 19**, strategy-bound two, active-v3/input-bound/successfully-bound zero. Input-bound includes active-v3 pin matching; zero does not establish historical corruption or invalid prior recommendations. Strategy execution/canonical hash/supported-date evidence is UNAVAILABLE (`target/r2-daily-beta/wp-1f/db-probe.sql:562–607`).
- **Owner V2/jobs:** active/READY/admitted one, latest admitted session and complete current-universe snapshot **October 6**. Five-key admissions and snapshot generation/universe joins pass. Queued/running/due jobs and running/orphan attempts are zero **at this snapshot**, not current drain permission, Owner acceptance or a service-exit explanation (`target/r2-daily-beta/wp-1f/db-probe.sql:611–692`).

The separately recorded official read-only XKRX selector (14:52:53.958753 UTC) has **37 sessions**, August 18–October 13. Exact set difference from EOD is **28**: August 25 plus 27 later sessions. Difference from the latest exact ETF11 publication is **29**, including August 31 as existing-EOD recovery candidate. The selector's artifact/manifest pins are unchanged in its receipt; physical protected bytes are unverified here. October 13 is eligible at the observed post-16:30 time. Before 16:30 on that same date, exclude it; October 14 needs fresh selection/frontier evidence, not reuse of this dated list. Full ISO sets and limits are in [catch-up readiness](2026-10-13-r2-catch-up-readiness.md) and private `target/r2-daily-beta/wp-1f/analysis/date-sets.json`.

Validation: `python3 target/r2-daily-beta/wp-1f/analysis/verify-evidence.py` **PASS**, offline only; all six exact hashes above, C/D/E hashes, pure fixed-field JSON validation, 59 local SHA-384/SHA-256 sources, compiled-v3 pins, helper hash, sorted full-set subtraction and append-only prefix checked. The approved active runner/SQL were not executed or edited. No host/network/provider/lifecycle/DB/build/test/Git action ran. No further analysis decomposition, background work or permission wait remains. Fresh operational admission, physical artifacts, full index definitions, backup/actual-role restore, U2 adoption and actual Owner/API/UI acceptance remain unresolved; G4 execution approval is not granted by this append.
