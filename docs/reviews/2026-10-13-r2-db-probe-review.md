# R2 WP-1b offline aggregate probe review — 2026-10-13

Completed scope: construction and offline review only. **The production probe
remains INERT and unexecuted.** No Docker, sudo, systemd, database connection,
provider/network call, runtime inspection, test/build, Git action or delegation
was performed. Existing WP-2 reports/runbook, product and roadmap were not edited.
G1 authorizes preparing exact SQL/runner for review, not executing it
(`docs/reviews/2026-10-13-r2-execution.md:83–85`).

## 1. Files, exact ranges and SHA256

All private artifacts are under `target/r2-daily-beta/wp-1b/`, directory 0700,
regular files 0600. The table records complete newly constructed file ranges.
The SHA256 of this tracked review is reported separately at handoff to avoid a
self-referential hash. No production result or receipt exists.

| File under private directory | Lines | SHA256 |
| --- | --- | --- |
| `construct-probe.py` | 1–447 | `348f17c27cefecf0bdca270d2af8bc41e20e7cada24e94cc9824881b4b6af20a` |
| `db-probe.py` | 1–450 | `58a846d6c17c627cbfd916f2e19fd61dabf1704bc5d82b4f794f27d13db4cdf8` |
| `db-probe.sql` | 1–701 | `cbdb8cbc8b311a609965529f7a776001dd57c93ff621399dad0fb68b339e1ef4` |
| `migration-source-hashes.json` | 1–359 | `5793c369f863b0965dc8bb65936f2e4e75287360be4444d1351a4c53c2d67300` |
| `query-contract.json` | 1–1457 | `96a068bbed56803cee47743dd063de2d33f6cdb2ebfec51579278abeb3114b0b` |
| `source-provenance.json` | 1–1290 | `c5831a49e760b1fb914c280928375ec8e3f552f0ca24025c6b0ecfac23d25f65` |
| `offline-review.py` | 1–99 | `bbffc5682e0354466662a53f06c783c193b8d0b319c63aba75c709718924937b` |
| `offline-checks.json` | 1–12 | `a98bfcf1b245dde6b5bd805161432d8275096066e0b4d293f01efd21dee49328` |

`construct-probe.py` is an offline constructor, not the transport runner.
It derives the five pins from the current v3 registry and records 59 local
up-migration byte hashes. Reconstructing changes the review contract if source
bytes change; coordinator must review and repin the runner before accepting it.

## 2. Transport, transaction and output boundaries

The draft is disabled in three independent places: `DRAFT_ONLY=True`, an immediate
exit 64 in `main`, and SQL `\quit 64` before BEGIN. Future-only transport functions
also reject while `DRAFT_ONLY` is true. There is no flag/environment bypass and
no automatic removal of the SQL gate (`db-probe.py:16,317–320,374–376,414–421,435–450`;
`db-probe.sql:1–5`). The runner was compiled but never invoked as a program.

Future transport is modeled on the existing R1 local-socket shape, not its
sudo/env inspection or lifecycle behavior
(`target/r1-resume-20261013/maintenance/work-state/work-state-probe.py:76–91`;
`target/r1-resume-20261013/maintenance/work-state/runtime-probe.py:23–41`).
WP-1's skeleton supplied the read-only transaction/visibility guard shape
(`target/r2-daily-beta/wp-1/db-probe.sql:1–28`). Neither prior probe was invoked.
These references are transport precedents, not present runtime attestations.

The future runner checks the release symlink and exact supplied prior container
ID, image, Compose project/service/name and running state before and after the
transaction; start timestamp and restart count must also remain unchanged.
All checks are fresh at execution time; the prior sample is not treated as
current evidence (`db-probe.py:18–25,374–401,414–432`). Only selected metadata is
requested, never `Config.Env` or state error prose. Docker is forced to the local
Unix socket with a clean child environment; no discovery/fallback/new container,
sudo or remote Docker endpoint is used (`db-probe.py:317–330,389–392,424–425`).

Only existing container `POSTGRES_USER`/`POSTGRES_DB` are expanded inside `sh`.
`env -i` removes inherited credentials/options; psql uses `/var/run/postgresql`,
`-X -w`, no password argv and `/dev/null` password/history files. No production
env file is read (`db-probe.py:403–411`). A container requiring another auth route
fails; changing identity or auth transport needs coordinator review.

PGOPTIONS forces `default_transaction_read_only=on`, UTC, statement timeout
10 seconds, lock timeout 1 second. SQL begins REPEATABLE READ READ ONLY and
reasserts those settings, with `search_path=pg_catalog`. The admission guard
checks actual transaction/default read-only, isolation, timeouts, UTC and
`rolsuper OR rolbypassrls`; it never sets an actor GUC or role. Missing global
visibility rejects the whole probe (`db-probe.sql:5–19`). Queries use qualified
public tables and fixed catalog/builtin functions. The only non-SELECT SQL is
BEGIN, fixed SET LOCAL statements and ROLLBACK. No DML, DDL, custom routine,
lock clause, EXPLAIN/ANALYZE or arbitrary function is allowed by the lexical
scope check (`db-probe.py:121–166`). Exact SQL and contract hashes are pinned
and checked (`db-probe.py:17,169–188`). This is not a general SQL parser.

One shared 179-second operation deadline reserves one second for process cleanup
within a 180-second envelope. Output is bounded to 524288 bytes; stderr is
discarded. Timeout/nonzero/partial/invalid output is rejected without diagnostics
(`db-probe.py:317–371,418–432,435–447`). On transport failure the server's socket
disconnect/10-second statement timeout remains the read-only backstop; cleanup
of a Docker client is not a production service stop. No partial result is emitted.

Explicit ROLLBACK precedes the completion marker. That final SELECT runs in a
separate implicit read-only transaction, while all aggregate sections share
the preceding repeatable-read snapshot (`db-probe.sql:700–701`). Full output is
emitted only after completion validation and the post-probe identity check.
JSON validation rejects extra/missing/duplicate keys, unexpected sections,
types/enums, oversized arrays, malformed dates/hashes, incomplete schema entries
or invalid date ordering (`db-probe.py:36–118,206–314`). Counts are bounded integers.
Allowed metadata is fixed schema/index names, version/checksum, dates/times,
counts and booleans. No owner/job ID, payload, config JSON, provider/error prose
or secret is projected. Per-owner joins exist only inside aggregates.

## 3. Source/query map, predicates and costs

Every one of 161 table/column/type prerequisites has a file:line definition in
private `source-provenance.json`. Missing table, column or expected type gives
the affected section `UNAVAILABLE/SCHEMA_MISSING` through psql conditional gates,
not a zero result (`db-probe.sql:30–374`). This gates even pre-0050/pre-0053 schemas.
`AVAILABLE` means the query returned, never an overall health/acceptance verdict.

| Section / SQL lines | Source evidence and exact meaning | Cost/predicate boundary |
| --- | --- | --- |
| header / 5–29 | UTC transaction time, KST date and civil cutoff bound. Existing EOD cutoff is 16:30; stale display deadline is 18:30 (`docs/contracts/service-status-v1.md:278–288`). | Stable transaction clock; not an exchange-calendar inference or SLA verdict. |
| schema / 30–374 | Catalog relation/attribute/type checks for fixed required columns. Column definitions: migrations 0003, 0005, 0022, 0042, 0045, 0049, 0050, 0053; individual citations are in provenance. | Two fixed 161-row catalog joins; only ordinary/partitioned tables admitted. Does not verify all grants, constraints or definitions. |
| ledger / 375–389 | Actual success flag, version and checksum from `_sqlx_migrations`. SQLx 0.9.0 table schema: cached `sqlx-postgres-0.9.0/src/migrate.rs:130–137`; checksum: cached `sqlx-core-0.9.0/src/migrate/migration.rs:42–49,86–87`; dependency `Cargo.lock:4253–4255`. | Ordered LIMIT 1025; reject overflow above 1024. Python compares actual SHA384 with all 59 local `.up.sql` raw UTF-8 byte hashes, without mutating either ledger/source. |
| indexes / 390–400 | All seven concurrent indexes, exact names/table/uniqueness and `indisvalid`/`indisready`: `0024_research_publication_source_index.up.sql:9–11`, `0025_research_calendar_version_lookup.up.sql:9–11`, `0027_recommendation_run_job_index.up.sql:7–8`, `0028_recommendation_scheduled_identity_index.up.sql:4–7`, `0029_recommendation_item_unique_index.up.sql:4–5`, `0031_target_portfolio_run_index.up.sql:4–5`, `0032_jobs_typed_claim_index.up.sql:5–6`. | Seven catalog lookups. Full predicate/operator-class definition comparison is outside this draft. Missing index is explicitly `exists=false`, not valid. |
| pre0050 / 401–413 | Only when version 50 lacks a successful ledger row: global EXISTS on run table. Already applied gives NOT_APPLICABLE. Migration requires globally empty runs and cannot fabricate enqueue snapshots (`0050_owner_beta_strategy_snapshots.up.sql:4–23`). | EXISTS, at most first matching row; no tenant filtering or row dump. Does not grant migration permission or replace the migration's locked guard. |
| eod / 414–434 | Shared KRX/KR credentialed EOD file metadata; first/max dates, exact distinct dates <= KST today, future/incomplete-lineage rows and dates with multiple files (`0003_market_data.up.sql:49–65`; `0022_research_publication.up.sql:13–23`; `crates/api-server/src/repos/service_status.rs:82–90`). | Provider/market/kind/fetch-mode filters; GROUP BY date, LIMIT 10001; overflow >10000 rejected. Aggregate scans matching metadata, not bar files. Multiple files per day are legitimate, not a duplicate-publication defect. |
| etf / 435–480 | Latest candidate exact ETF11 last-session count and unexpected count; exact dataset/version/manifest binding and credentialed PUBLISHED Raw identity/reference/date/time booleans (`crates/api-server/src/repos/service_status.rs:91–123`; `0042_candidate_source_contracts.up.sql:291–409`; binding dataset column `0045_candidate_multi_universe.up.sql:53–60`). | Latest price row LIMIT 1, indexed last-session/session lookups; current latest is selected before validation so invalid latest cannot hide behind an older valid one. READY/WARNING usable; BLOCKED is displayed but not accepted. No filesystem hash or rights renewal claim. |
| published_dates / 481–516 | Separate exact ETF11 session dates from that same latest eligible publication, dataset binding and exact credentialed PUBLISHED Raw lineage. Each included date has exactly 11 rows and 11 distinct required instruments (`0042_candidate_source_contracts.up.sql:343–365`; exact ETF set `crates/api-server/src/repos/service_status.rs:79–81`). | Dataset-specific sessions, bounded first/last range and <=today, GROUP BY date LIMIT 10001. No eligible lineage => UNAVAILABLE/LINEAGE_UNVERIFIED. These are covered session dates, not daily job invocation dates or all historical dataset generations. |
| calendar / 517–561 | Rolling 370 civil dates through KST today, TRADING/CLOSED/MISSING/INVALID, exact calendar/version/credentialed CALENDAR file source/date/hash joins; latest verified eligible trading date; today's verified and 36-hour-fresh proof (`0022_research_publication.up.sql:25–53`; `crates/job-queue/src/owner_equity_v2/intraday.rs:1350–1403`). | 370 day slots with source equality joins and proof count exactly 1. Includes CLOSED; never substitutes weekday logic. Bounded lookups may still hit large history tables; timeout is final guard. No dates-only XKRX artifact read. |
| beta / 562–610 | Global five status counts/max successful as_of and finish time; exact active v3 five pins, run/job owner/config/run/date identity and immutable strategy JSON/hash equality, exact payload/pins/strategy key sets; successful binding count (`0049_owner_beta_price_recommendations.up.sql:4–49`; `0050_owner_beta_strategy_snapshots.up.sql:105–203`; `crates/api-server/src/repos/owner_beta.rs:1017–1043`). | Full run scan plus job PK join; guarded object-key checks consume only internal JSON. Known registry `configs/evidence/kis-historical-price-only-v3-approved-artifacts.json:1`; embedded v3 `historical_price_only_approval.rs:58–60`. Range 2016-08-29 through 2026-08-28 is only a necessary bound, not proof every date is supported. |
| v2 / 611–671 | Active !=DISABLED, READY, latest generation and exact five-field admissions; independently latest admitted generation, minimum last_session; latest published snapshot per owner covering exact current READY IDs/generations, row count and universe hash (`0053_owner_managed_equity_universe_v2.up.sql:39–84,146–274,849–898`; `crates/api-server/src/repos/service_status.rs:157–216`). | Global active memberships, indexed lateral latest-per-membership/owner lookups, owner grouping. No global LIMIT can conceal another owner. Counts both latest-generation admission and any latest admitted generation so older admissions cannot silently satisfy current-generation coverage. Snapshot completeness counts owners with nonempty READY universe only. |
| jobs / 672–695 | Global QUEUED/RUNNING/due counts; active-linked RUNNING/ORPHANED attempts and running jobs lacking latest RUNNING attempt. Fixed five known job-type aggregates plus unknown-type count (`0005_jobs.up.sql:11–57`; `owner_beta/input.rs:16`; `owner_equity_v2.rs:116`; `candidate/runner.rs:122`; `recommendation/runner.rs:220`). | Active-job predicate; attempt job joins and latest attempt by number. Counts attempts linked to active jobs only, not all historical failures; does not attest a completed drain or infer failure cause. |
| calendar_claim / 696; complete / 700–701 | Bootstrap claim is filesystem state, not a modeled DB relation (`crates/market-data/src/ingest.rs:479–497`). Returns UNAVAILABLE/FILESYSTEM_ONLY. Current-day DB source lineage is covered above. | No Raw path reads. Explicit rollback marker, no writes or session spoofing. |

Ledger comparison reports known/matching/success per version, pending or
mismatched local versions and unknown applied versions. Failed/unknown/checksum
mismatch is evidence for coordinator review, never repaired. SQLx source loader
uses full UTF-8 source with default empty ignored-character configuration
(cached `sqlx-core-0.9.0/src/migrate/source.rs:88–97,226–244,256–262`). The production
CLI/version/configuration was not inspected, so actual non-default hashing
would require a decision rather than accepting a mismatch. Version 0051's
separate data-dependent gate is not queried here; it remains a migration review
follow-up (`0051_owner_beta_target_publication.up.sql:4–16`).

## 4. Deviations, unresolved semantics and coordinator follow-up

No scope/permission deviation. Added separate collected vs published date
sections because Raw file dates alone cannot prove ETF11 publication. The shared
EOD metadata has no instrument column; its `eod` dates must not be relabeled as
proven ETF11-only dates. The ETF publication/coverage section provides that
independent attribution. It does not inspect provider bodies to fill this gap.

The calendar window is explicitly bounded to 370 days, and published coverage
to the latest usable publication. Missing-session arithmetic can compare exact
TRADING dates in that window to published dates; dates outside that window or
missing/invalid calendar proof are unknown. Coordinator must choose any broader
calendar scope and obtain the approved dates-only artifact separately. No
holiday, absence, request budget or historical session is invented.

Canonical strategy hash recomputation, strategy execution validity and actual
approved supported-date membership are explicitly UNAVAILABLE. Rust hashes
domain-tagged canonical JSON (`owner_beta/input.rs:257–294`), validates strategy
requirements in compute (`owner_beta/compute.rs:71–99`), and derives supported
dates from approved artifact bars (`api-server/src/http/owner_beta.rs:565–586`).
SQL equality/format/five-pin checks are useful lineage checks but cannot certify
those three facts without an additional reviewed contract. No raw config/bar
export, mutable config reconstruction or custom SQL function is substituted.

Persisted hashes/bindings do not verify physical Raw/Curated files, current
rights, an operational_v1 intraday window, timer scheduling, backup consistency,
complete restore or actual latest production health. Empty AVAILABLE sections
are empty observed populations, not healthy verdicts. V2 zero READY owners does
not prove signals complete; latest snapshot coverage does not prove EOD
freshness without the verified eligible calendar date/cutoff and 18:30 deadline.
Historical exited2 remains historical; no present root cause is inferred.

Before any execution, coordinator must review these exact bytes, resolve any
required extension above, validate SQL grammar against an approved offline
parser/server-free route if available, and authorize a distinct activation
revision. That revision must coherently revise all draft gates/hash checks; this
draft intentionally cannot be enabled by toggling a flag. Then fresh identity,
Unix socket auth, role visibility, schema/index/ledger and query-cost evidence
can be obtained. Drift must abort rather than refreshing pins automatically.
No executable apply/activation command is provided. A root grant, old R1 drain
attestation or prepared aggregate is not lifecycle/migration permission.

## 5. Checks, not verified and remaining work

Executed only local offline commands:

1. `python3 -W error target/r2-daily-beta/wp-1b/construct-probe.py`: constructed
   SQL, output contract and 59 source hashes, no process/network invocation.
2. `python3 -W error target/r2-daily-beta/wp-1b/offline-review.py`: PASS lexical
   SQL scope/draft gates, Python compile/AST, exact contract/SQL/manifest hashes,
   source types for 161 prerequisites, all 59 local source hashes, private modes;
   14 fixed output sections. It loads definitions only, never invokes `main`,
   transport, old probes or production SQL.
3. Local `rg`/`sed` reads and SHA256/line-count generation: source evidence and
   artifact ranges/hashes recorded above. Final file:line citations checked for
   existing files and in-bounds ranges; no Git action.

Offline correction during construction: Python escape/indentation and lexical
grammar-word allowlist issues were caught and fixed before final checks passed.
No production failure or product test is represented by these checks.

Not found/verified: local `pglast` is unavailable; PostgreSQL grammar/plans and
actual SQL output were not verified. Actual role/auth/schema/grants, ledger
versions/checksums/index validity, EOD/calendar/owner/job populations, file
hashes/rights, current runtime identity and query time/cost are all unverified.
Future transport and JSON result validation were not exercised against Docker,
Postgres or synthetic production results in this assignment.

Unresolved/follow-up: coordinator exact-byte review and any later authorization;
strategy/support-date proof and broader calendar scope if needed; separate
data-dependent migration/backup/restore and activation gates remain outstanding.
This offline construction needs no further decomposition. The bounded task is
complete; R2/G1 and production diagnosis are not marked complete.
Background work: **none**. Permission waits: **none**. Native delegation: **none**.
