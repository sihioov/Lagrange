# R1 Rust feature diagnostics — supplemental implementation contract

Status: **SPECIFICATION_READY**; read-only analysis complete, idle. Date: 2026-10-12 (Asia/Seoul). Repository `/data/worktrees/3puw275b/orange-dugong`, branch `orange-dugong`, HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`. Relevant roadmap tasks: **R1-02 / R1-03**, with final acceptance remaining **R1-08**. This report neither completes those tasks nor authorizes implementation. The worktree already contains concurrent coordinator/WP edits; coordinates below describe the inspected working source, not pristine HEAD.

This assignment writes only this new document. No source, roadmap, diagram, manifest, workflow, other review, or other worker file is changed. No tests, build, Cargo, Docker, DB, npm, network/provider operation, staging, commit, or delegation was performed. WP-1 Web E2E and WP-5 DB SQL/tests remain separate. Migration-contract E0521/E0277 diagnostics are outside this report and were not investigated.

## Evidence and diagnosed boundaries

Read [baseline lines 390–590](2026-10-12-r1-baseline.md), [Rust heavy-validation report from line 198](2026-10-12-r1-rust.md), and all four complete bounded logs under `target/r1-runtime-fxi6ltmv/wp2-heavy-validation-20261012-01/logs/`. These are historical failed commands, not new execution evidence:

| Log | Lines | Scoped diagnostic locations in log | SHA-256 |
| --- | ---: | --- | --- |
| `jobqueue-default-clippy.log` | 105 | E0422 :73; BufferDiagnostics :84; diagnostics :93 | `347813f729f655a0bdc87c322a303159b424f9d9bc9e308a182e98df6ddb5983` |
| `jobqueue-dbtest-clippy.log` | 51 | E0422 :8; endpoint getters :19; c2_endpoint :33; relay resolver :42; target intraday_producer :48 | `ed1069112b943b8c3ec1121fc87cee310207bb45a372847b3992d0a484045912` |
| `jobqueue-all-clippy.log` | 61 | E0422 :8; runner payload :19; test Event variants :36; FakeTask variant :48 | `cfc5ebec6165048ff2bc3f80bf66843b2ba44fc8664373a2c88f5450cebc5ec3` |
| `jobqueue-mixed-clippy.log` | 23 | E0422 :2; duplicate_mod :13; target intraday_calendar_read_state :20 | `91bfc1ab3cf6402a8c03a9e3d2b1149d023f5503c609e6979dbb6ff4ac19bcda` |

All four invocations previously exited 101. Failure/early termination can mask other target diagnostics; absence from one log does not prove that its selection is clean. The duplicate is caused by the target's source inclusion, not by provisioning itself; provisioning is the matrix selection in which the log exposed it.

**Already repaired by coordinator; preserve, do not reimplement:** `market_stream_producer.rs:1171–1173` now gates BufferDiagnostics with `cfg(any(test, feature="market-stream-db-tests"))`; :2130–2131 applies the same gate to LatestKernel::diagnostics. All measurement reads at :1353, :1500, :1507 sit inside that same gate; the remaining :2670 call is inside `cfg(test)` runtime-buffer tests. `runtime_buffer`'s `#[cfg(test)] mod tests` :2204–2207 now imports `crate::owner_equity_v2::market_stream::SubscriptionProof`, used at :2346. Top-level and other independently used SubscriptionProof imports remain. This source is consistent with the stated repairs. Coordinator reports rustfmt check passed; this analyst did not run it. Compilation remains pending.

## 1. C2 fixture: complete inclusion and caller inventory

`c2_fixture.rs` is a shared **test fixture**, loaded separately into multiple Rust crates/targets. An integration test is its own crate: a caller in the job-queue lib-test crate does not count as a use in `intraday_producer`. `cfg(test)` alone cannot distinguish these copies.

| Independently compiled crate/target | Inclusion path and gate | Relay use |
| --- | --- | --- |
| job-queue lib-test | `src/owner_equity_v2.rs:99–101`, `cfg(all(test, feature="market-stream-db-tests"))` → shared boundary `mod.rs:35` → c2_fixture | Yes, via producer support |
| job-queue `intraday_quotes` | root :1 → `intraday_quotes_support/mod.rs:26–28` → c2_fixture, DB-test feature gate | No |
| job-queue `intraday_producer` | root :1 → same support and gate | No; reported lint target |
| job-queue `intraday_read_state` | root :1–2 → same support and gate | No |
| job-queue `intraday_calendar_read_state` | root :1–2 → same support and gate; **also** root :1189–1192 → c2_fixture as eod_restart_binding | No; duplicate in this one crate |
| job-queue `intraday_endurance` | root :3–4 → same support and gate | No |
| job-queue `intraday_producer_pipeline` | root :2–3 → same support and gate | No |
| job-queue `intraday_producer_scheduling` | root :3–4 → same support and gate | No |
| job-queue `intraday_producer_lifecycle` | root :4–5 → same support and gate | No |
| job-queue `intraday_demand_identity` | root :1–2 → same support and gate | No |
| api-server `owner_market_stream_http` | crate-level DB-test feature gate :1; root :11–12 → job-queue boundary mod.rs → c2_fixture | No |
| api-server `owner_market_stream_integrated` | crate-level DB-test feature gate :1; root :21–22 → same boundary mod.rs → c2_fixture | No |

The lib-test boundary is already loaded once. Its consumers use aliases: `market_stream_c3a_tests.rs:3,10`, `market_stream_runtime_storage_tests.rs:236`, `market_stream_runtime_test_support.rs:36`, `market_stream_producer_test_support.rs:5`. Runtime support is included by `market_stream_runtime_tests.rs:18`, itself included by `market_stream_runtime.rs:2757` under its existing test/feature gate. Producer support is included by `market_stream_producer_tests.rs:149–151`, under the DB-test feature; its enclosing file is included under `cfg(test)` by owner_equity_v2.rs. Do not undo that completed unification.

Seven intraday roots already attach `#[allow(dead_code)]` to their support module; intraday_quotes and intraday_producer do not. Those historical allowances explain why redundant helpers do not produce the same diagnostics everywhere. This contract does not add or broaden any module allowance. The calendar's second module also has an existing dead_code allowance; it cannot suppress duplicate_mod.

**Exact relay chain:** boundary `mod.rs:36` re-exports the resolver; producer support `PgRelay::start` :1999–2011 calls it at :2009 and uses both endpoint getters at :2010. Resolver `c2_fixture.rs:498–511` calls private `SupervisorTarget::from_environment_with_inputs` at :504 and `c2_endpoint` at :505. The getter at :146–150 is otherwise uncalled; endpoint getters at :65–71 have no other affected callers. C2Endpoint's fields/type are also used by validation, URL construction, actual identity checking, and the canonical-URL unit test; the type/fields are not obsolete.

The sole PgRelay::start caller is producer tests `start_task_relay` :3563–3569. Its calls are :5723, :6029, :6093, :6157, :6211, :6297, :6472, :6578, :6727, :6811, :6895, :6979. The :6297 helper is exercised by both runtime-claim commit-fault tests. These paths cover real commit withholding/loss, cancellation, rollback/reread and intentional-panic cleanup. No API test or intraday target calls the resolver/getters. Other `.port()` matches belong to listeners/connect options and are unrelated.

**Common fixture calls and visibility:** intraday support :43/:45/:70/:83 uses from_environment/is_c2/connect_supervisor/options. Boundary mod.rs uses from_environment at :151/:176/:1500, options at :177/:1513, is_c2 at :179/:1502; its wrapper at :143–146 delegates to c2_fixture::connect_supervisor, as does exact-name cleanup at :89. Boundary creation uses that wrapper at :178/:1533. In core, connect_supervisor :225–239 calls options :231 and verify_before_ddl :235; verify_before_ddl has no other caller. Calendar child uses from_environment :1274, is_c2 :1276, connect_supervisor :1279, options :1288. The second calendar copy currently makes those methods accessible through its own parent; replacing it with a sibling alias requires the visibility changes in §2.

### Selected small contract; coordinator choice C1

Recommend retaining the existing finite relay adapter with **exactly four item-scoped** allowances, pending coordinator approval. Put the following attribute individually on `C2Endpoint::socket_directory`, `C2Endpoint::port`, `SupervisorTarget::c2_endpoint`, and `relay_endpoint_from_environment` — never on an impl, struct, module, crate, or callsite:

```rust
#[allow(dead_code, reason = "relay adapter is used by lib tests but shared fixture targets compile independently")]
```

Preserve their bodies, signatures and validation. This is a proposal for a new bounded exception, not an already approved allowance. It is needed for this retention contract because those four items are actually used in one crate and unused in others. An unconditional `expect(dead_code)` is unsuitable: the live lib-test relay caller can leave the expectation unfulfilled. The same local `test`/DB-test cfg is true in all the relevant binaries; no existing cfg identifies only relay consumers. Feature/target defaults must not be changed to manufacture such a distinction.

Remove only the **relay resolver re-export** from boundary `mod.rs:36`, retaining `pub(super) use c2_fixture::SupervisorTarget;`. Change the one real caller at producer support :2009 to `boundary::c2_fixture::relay_endpoint_from_environment()?`. The existing c2_fixture module visibility permits this descendant of owner_equity_v2 to access it. This removes an unused relay import from API boundary copies without an additional unused_imports allowance. No helper body is duplicated.

Alternative if C1 is rejected: isolate relay-only adapter code into a module included only by producer test support, sharing the validated core. That requires a separately bounded design for access to private endpoint/binding fields and the private environment constructor, and rerouting inclusions/re-exports in both job-queue and API copies. It trades zero new lint allowances for more source movement and private-interface changes around a safety boundary. This report does **not** authorize that larger refactor or new files. Deleting the adapter, cloning the validator into producer support, inserting fake uses/tests, or blanket dead_code suppression is forbidden. C1 is the only unresolved decision in the selected contract.

## 2. Calendar duplicate module: exact alias and access contract

Under DB-test features, intraday_calendar_read_state includes the same physical c2_fixture source through `intraday_quotes_support::c2_fixture` and root `eod_restart_binding`. These are duplicates **within one integration crate**. The baseline's instruction to leave independent integration inclusions intact did not resolve this intra-target duplication; this supplement supersedes that instruction only for this second calendar load.

At intraday support :26–28 retain the path and cfg, changing `mod c2_fixture;` to `pub(super) mod c2_fixture;`. At calendar :1189–1192 replace the cfg/allow/path/mod block with:

```rust
#[cfg(feature = "market-stream-db-tests")]
use intraday_quotes_support::c2_fixture as eod_restart_binding;
```

The alias alone is insufficient: core methods currently have visibility relative to their defining module, not the alias. Change **only** core `SupervisorTarget::from_environment` :95, `options` :142, `is_c2` :152 and free `connect_supervisor` :225 from `pub(super)` to `pub(crate)`. Keep from_environment_with_inputs private, verify_before_ddl pub(super), fields private, and all other visibility unchanged. These are crate-visible members of private test modules; no production export/public API is added. All inclusion paths in §1 are feature-gated integration code or cfg(test) library code.

Keep **both** SUPERVISOR_ENV constants: intraday support :29–30 remains the lexical parent constant read by core :107 after aliasing; calendar :1187–1188 remains required by run_child's explicit environment forwarding at :1384. The same literal `LAGRANGE_WS3A_SUPERVISOR_URL` is preserved. Do not remove the root constant as an apparent consequence of removing its module.

Keep all restart tests and process semantics. The child remains exactly `eod_restart_tests::owned_recovery_child`; its private config validates owner/mode/nlink/parent PID/generated database prefix and source identity. It still rejects non-C2, verifies actual supervisor identity, connects as an actual research_writer login and checks current_user/session_user. Publish child exits 73 after durable commit/checkpoint; replay is a fresh process exiting 0. env_clear plus the finite forwarded keys, kill_on_drop, 20-second bound, kill/wait/reap and evidence hashing remain. Preserve four data_batches, one history/one projection, unchanged manifests/raw bytes and no refetch/duplicate assertions. The parent :1542–1549 still attempts exact database cleanup after exercise_restart returns, before asserting the body result. Existing intraday cleanup/run_body :395–430 and shared boundary deadline/owned-pool/absence/exact-drop guards are untouched; this contract makes no stronger panic-cleanup claim for intraday run_body.

## 3. Runner: typed payload retention and unused private test vocabulary

Inventory in `owner-equity-v2-runner.rs`:

| Item | Complete affected uses |
| --- | --- |
| QuoteTaskError::RestUnavailable | Constructor :841; declaration :1483; no explicit variant pattern |
| QuoteTaskError::MarketWs | Constructor :856; declaration/payload :1484; no explicit variant pattern or payload read |
| Quote-task results | Finished task take/join :889–907; shutdown join :1013–1026; both `Ok(Err(_)) \| Err(_)` arms emit the same fixed PRODUCER_UNAVAILABLE code and set failure |
| Event::LoadRest / Event::StartRest | Declarations :1538–1539 only; no qualified uses, patterns, constructor, or negative assertion references |
| Event's retained variants | StartMarketWs push :1804/:1832, expected :1816/:1844; ContinueEod push :1760/:1782/:1808/:1836, expected :1766/:1789/:1816/:1844 |
| FakeTask::Rest | Declaration :1546 only; no constructors/patterns/assertion references |
| FakeTask::MarketWs | Constructor :1805; expected :1813; other FakeTask uses are Option annotations/None assertions |

The unread payload is private retention without an inspecting consumer; derived Debug does not count as a read. Retain its actual type and lifetime using a deliberately unused **named field**, with no dead_code allowance:

```rust
enum QuoteTaskError {
    RestUnavailable,
    MarketWs {
        _error: job_queue::owner_equity_v2::MarketStreamRuntimeError,
    },
}
// Replace only the existing .map_err constructor at :856:
.map_err(|error| QuoteTaskError::MarketWs { _error: error })
```

Retain the existing Debug derive. This remains a typed MarketWs task error carrying the original runtime error until the same join result is discarded. Do not convert it to a string/Debug/Display, extract/log provider messages, drop it early in a unit-valued constructor, add error codes, or alter the match arms/exit/cleanup. A tuple payload named `_` in a pattern would not fix field dead_code; a new fake read would manufacture usage. The named `_error` field states intentional typed retention directly.

Remove only Event::LoadRest, Event::StartRest and FakeTask::Rest declarations. Every existing test/assertion stays. Negative REST assertions are executable panic closures, not references to these three variants: disabled-mode closures :1755–1757; REST-selected closures :1778–1780; WS-selected REST closures :1801–1802; failed-WS no-fallback closures :1829–1830. Retain event-order/None assertions :1766/:1783–1789/:1813–1817/:1841–1845.

**Selection/startup is immutable for this repair:** quote_backend :1359–1371 returns Disabled for every REST transport, every non-Daemon mode and Disabled intraday mode; only Daemon/OwnerOnly/MarketWs selects WS. Its callers are load_config :357, build_market_stream_runtime :1453 and run_selected_quote_startup :1514; test calls :1580/:1600. Keep QuoteBackend::Rest itself: it is used explicitly by selected_market_ws_config :1425, run_selected_quote_startup :1516 and the pins-negative test :1609. Keep typed RestUnavailable, its inactive factory, run_intraday_quote_startup and all generic startup seams. This contract neither retires production REST code nor makes it reachable. No REST/trading/live activation, feature/default change, credential reread, transport selection change or new provider authority.

## Closed future edit allowlist

The following six paths are the **entire proposed implementation allowlist**, subject to C1. This worker has edited none of them. Preserve all concurrent edits outside the listed items; no migrations/SQL/tests under WP-5 or Web E2E/workflow files under WP-1 enter this scope.

| Absolute path | Exact permitted changes in current source |
| --- | --- |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/owner_market_stream_boundary_support/c2_fixture.rs` | C1 attributes on four items :65/:69/:146/:498 only; four pub(crate) changes :95/:142/:152/:225 only |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs` | :36 remove relay resolver re-export, retain SupervisorTarget re-export |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs` | :2009 qualify resolver through boundary::c2_fixture; no other change |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/intraday_quotes_support/mod.rs` | :28 module visibility only, preserving cfg/path and parent SUPERVISOR_ENV |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/intraday_calendar_read_state.rs` | :1189–1192 replace duplicate module with gated alias; all restart coverage unchanged |
| `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/src/bin/owner-equity-v2-runner.rs` | :856 constructor; :1484 typed named field; :1538–1539/:1546 three private test variant deletions only |

No new source files are proposed. market_stream_producer.rs's coordinator repairs are excluded from this future list. Pure test helper aliasing and a private error representation do not change workspace members, Cargo edges, services, routing, DB role/table groups, web/API contract, wire/storage behavior or production authority. Existing runner citations in component_architecture.puml:217 and runtime_deployment.puml:269/:271 point before the proposed error/test declarations; the :856 constructor remains on the cited factory boundary with a one-line replacement. No diagram edit/render is needed for this contract; the coordinator must still check final citations if integration formatting shifts them. Roadmap status/history changes belong to the coordinator's integrated change, outside this assignment's one-document write authorization.

### Safety invariants — unchanged, not merely re-tested

Core :24–39 pins remain exact: binding path `/tmp/lagrange-kis-stream-completion-20261001-653ff5d84407/wp3c2-runtime-binding.json`; digest `f173e4eed80a6543535f574fa003d6d77d521bfb41d6c532f6c44442a2f4f268`; run `4b4fed71-6608-4d97-a6b6-7ae3ba294be0`; root `/tmp/lagrange-kis-c2-4b4fed7166084d97a6b67ae3ba294be0`; system ID `7691641528624668685`, rejecting legacy `7687959257259950090`; port 55471; server_version_num 180006; c2_supervisor/postgres; 4096-byte bound and two-second actual identity timeout.

Preserve paired explicit inputs, lowercase/exact digest, unknown-field rejection, canonical UUIDv4/paths/socket-length limit, effective UID/GID, directory 0700/file 0600, regular file/nlink=1/no symlink, safe open flags, before/after descriptor/path identity and SHA-256. Preserve rejection of all foreign PG-prefixed settings, exact Unix-socket URL and SQLx normalized socket/port/user/database/SSL checks, implicit password clearing and SQL timeout options. Both binding variables absent retains the existing legacy path; partial inputs fail closed. Intraday legacy selection still requires its exact existing fixed loopback QA URL; no alternate host/URL is introduced.

Before any DDL, connect_supervisor still queries actual control-system ID, data_directory, empty listen_addresses, Unix socket directory, port, null inet_server_addr, server version, current_user/current_database, effective owner and pinned run. Identity failure closes the pool. Preserve boundary's additional legacy identity checks, generated-name absence checks, owned pools, deadlines, relay descendant join/shutdown, exact-name drop and cleanup error propagation. No raw/provider contents or credentials enter new output; no entitlement/rights/approval is changed.

## Reproducible read-only inventory

Run from the repository root; these searches are the patterns used to establish inclusion/caller completeness. Unrelated port/options matches require checking their receiver/type; they are not C2 caller evidence.

```sh
rg -n 'c2_fixture|eod_restart_binding|C2Endpoint|SupervisorTarget|relay_endpoint_from_environment|SUPERVISOR_ENV' crates/job-queue crates/api-server
rg -n 'owner_market_stream_boundary_support|intraday_quotes_support|market_stream_producer_test_support|market_stream_runtime_test_support|market_stream_runtime_tests' crates/job-queue crates/api-server
rg -n -B 1 'mod intraday_quotes_support' crates/job-queue/tests
rg -n 'PgRelay::start|start_task_relay|relay_endpoint_from_environment|\.c2_endpoint\(|\.socket_directory\(|\.port\(' crates/job-queue crates/api-server
rg -n '\.(options|is_c2|verify_before_ddl)\(|SupervisorTarget::from_environment|connect_supervisor\(' crates/job-queue/tests/owner_market_stream_boundary_support crates/job-queue/tests/intraday_quotes_support crates/job-queue/tests/intraday_calendar_read_state.rs crates/api-server/tests
rg -n 'QuoteTaskError|Event::|FakeTask::|quote_backend|RestUnavailable|MarketWs|intraday_task' crates/job-queue/src/bin/owner-equity-v2-runner.rs
rg -n 'relay_endpoint_from_environment|c2_endpoint|C2Endpoint|QuoteTaskError|Event::LoadRest|Event::StartRest|FakeTask::Rest' --glob '*.rs' crates data-pipelines tests
rg -n 'BufferDiagnostics|fn diagnostics|diagnostics\(|SubscriptionProof|mod tests' crates/job-queue/src/owner_equity_v2/market_stream_producer.rs
rg -n 'owner-equity-v2-runner|intraday_quotes_support|intraday_calendar_read_state|c2_fixture' docs/diagrams/*.puml
```

After implementation repeat inventory: exactly one calendar load, exactly one real relay call through the qualified module, no removed test-variant references, and all original selection/negative assertions remain. Do not count different integration crates as duplicate_mod.

## Future verification matrix — specified, not executed

Use the existing coordinator-owned heavy slot/cache/target, offline/locked resolution and `CARGO_BUILD_JOBS=2`. Run serially, with no builds/tests overlapping another owner's DB/resource slot. Keep `-D warnings`, all original targets and unchanged feature defaults. Record command, revision/worktree evidence, exit, reached targets and counts; compilation blocked elsewhere is not a pass for this repair.

| Selection | Exact required Clippy invocation |
| --- | --- |
| job-queue default | `cargo clippy -p job-queue --all-targets --locked -- -D warnings` |
| job-queue DB-tests only | `cargo clippy -p job-queue --lib --tests --features market-stream-db-tests --locked -- -D warnings` |
| job-queue all features | `cargo clippy -p job-queue --lib --tests --all-features --locked -- -D warnings` |
| Independent provisioning unification; local job-queue provisioning off | `cargo clippy -p job-queue -p kis-client --lib --tests --features job-queue/market-stream-db-tests,kis-client/market-stream-provisioning --locked -- -D warnings` |
| api-server default | `cargo clippy -p api-server --all-targets --locked -- -D warnings` |
| api-server fixture targets | `cargo clippy -p api-server --all-targets --all-features --locked -- -D warnings` |

A separate all-features invocation cannot substitute for mixed unification. Do not add job-queue/market-stream-provisioning to the mixed row. The runner and independent integration binaries must actually be reached; `--lib` alone would miss both failures. If a broad run stops early, focused `--test intraday_producer`, `--test intraday_quotes`, `--test intraday_calendar_read_state`, and `--bin owner-equity-v2-runner` Clippy selections with the corresponding features can localize remaining failures, but do not replace the matrix.

Existing pure selections, with DB/provider environment removed as in the prior heavy slot:

```sh
cargo test -p job-queue --bin owner-equity-v2-runner --locked
cargo test -p job-queue --bin owner-equity-v2-runner --all-features --locked
cargo test -p job-queue --lib --features market-stream-db-tests --locked c2_fixture::tests
cargo test -p job-queue --lib --all-features --locked c2_fixture::tests
cargo test -p job-queue --features market-stream-db-tests --locked --test intraday_quotes --test intraday_producer --test intraday_read_state --test intraday_calendar_read_state --test intraday_endurance --test intraday_producer_pipeline --test intraday_producer_scheduling --test intraday_producer_lifecycle --test intraday_demand_identity c2_fixture::tests
cargo test -p job-queue -p kis-client --features job-queue/market-stream-db-tests,kis-client/market-stream-provisioning --locked --test intraday_calendar_read_state c2_fixture::tests
cargo test -p api-server --features market-stream-db-tests --locked --test owner_market_stream_http --test owner_market_stream_integrated c2_fixture::tests
```

Each included C2 fixture has five existing pure tests: `c2_binding_requires_both_explicit_inputs_and_rejects_legacy_or_other_runs`, `c2_binding_parser_rejects_unknown_fields_wrong_identity_and_bad_paths`, `c2_binding_file_checks_private_owner_mode_link_count_and_digest`, `canonical_c2_url_is_parsed_by_sqlx_as_the_bound_unix_socket`, `c2_binding_parser_rejects_symlink_input`. After aliasing calendar, expect one copy/five tests in that target. Record discovered counts; zero matches is not verification. Do not claim that the file-metadata test exercises every owner/nlink mutation: its existing body directly mutates digest and mode; the checks themselves stay intact.

The runner suite must retain `quote_transport_tests::quote_transport_selection_is_exclusive_across_modes`, `only_daemon_uses_configured_intraday_mode`, `market_ws_startup_pins_are_required_only_for_selected_daemon`, `market_ws_startup_pins_reject_invalid_metadata`, `off_once_and_healthcheck_skip_all_quote_factories`, `rest_transport_is_disabled_and_continues_eod_without_quote_startup`, `market_ws_startup_skips_rest_windows_and_factory`, `market_ws_factory_failure_continues_eod_without_rest_fallback`, `market_ws_snapshot_reuses_resolved_credentials_without_reread`, plus existing startup/window/mode/secret-free logging tests in `tests`. No new tests are needed to manufacture helper reads or mirror the named-field change. Existing startup tests do not directly execute main's quote-error join arms; unchanged handling/drop ownership is a source-inspection invariant, not claimed runtime coverage.

Preserve and rerun the already required diagnostic-unit filters in DB-only/all/mixed selections from the baseline: `original19_closed_diagnostic_unit_tests`; and all-feature `synthetic_post_ack_event_is_monotonic_and_session_bounded`, `closed_failure_diagnostic_unit_tests`, `original20_stage_diagnostic_unit_tests`. These establish the coordinator repair reached test discovery and retain prior D1/D1b/D2/D3 coverage; their implementation is outside this follow-up.

DB/loopback acceptance remains in the coordinator's separately authorized synthetic C2 slot, never with production inputs. The exact restart parent selection is:

```sh
cargo test -p job-queue --test intraday_calendar_read_state --features market-stream-db-tests --locked eod_restart_tests::exact_source_eod_restarts_after_durable_commit_without_refetch -- --exact --test-threads=1
```

Use the existing pinned binding, explicit supervisor and owned evidence inputs. Do not run `owned_recovery_child` standalone or a broad `eod_restart_tests` filter: it needs parent-created private config and is invoked by the parent twice. Record real child exits/reaping, rows/manifests/source preservation and exact DB cleanup; no missing-env skip is acceptance.

For relay coverage, existing full names use prefix `owner_equity_v2::market_stream_producer_tests::database_cases::`. Run each selection with `cargo test -p job-queue --lib --features market-stream-db-tests --locked <full-name> -- --exact --test-threads=1` under that same authorized fixture. Retain these actual relay consumers (no new mirror tests):

- `cancellation_while_pending_and_ack_commit_results_are_withheld_is_terminal`
- `start_epoch_actual_commit_failure_returns_no_facade_and_rolls_back`
- `start_epoch_committed_but_lost_response_returns_no_facade`
- `start_epoch_future_cancellation_drops_owned_socket_without_resume`
- `desired_commit_response_loss_is_terminal_before_command_preparation`
- `runtime_claim_commit_before_forward_is_terminal_and_rolls_back`
- `runtime_claim_lost_commit_response_is_terminal_and_does_not_retry`
- `pending_commit_failure_rolls_back_and_lost_response_preserves_ambiguity`
- `ack_commit_lost_response_never_publishes_buffered_quote_or_returns_proof`
- `committed_quote_lost_response_is_returned_once_by_exact_reread`
- `cancellation_during_actual_quote_commit_returns_no_result_and_is_terminal`
- `quote_commit_failure_rolls_back_and_returns_no_publication`
- `database_cleanup_runs_after_intentional_test_panic`

Required final integration remains `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, formatting and the required CI/workspace tests at the final integrated revision, under their owners. Workspace failure in migration-contract is reported separately; this specification does not diagnose or amend it. Do not run ignored integrated browser/runtime acceptance merely because its pure C2 unit tests are selected.

## Delivery and limits

- Changed file/range: only new `docs/reviews/2026-10-12-r1-rust-feature-followup.md:1–224`; no earlier content exists to preserve. Future source edit spans are closed in the six-path table.
- Deviations: none from assignment ownership/execution restrictions. The necessary four method/function visibility changes accompany the proposed alias; they are explicit rather than left for an implementation worker to discover. C1 is proposed, not assumed approved.
- Checks performed: local branch/HEAD/status reads; roadmap/review/log/source/feature-gate/diagram citation reads; full four-log SHA-256/line counts; repository Rust symbol and target inclusion searches. No execution result for the proposed edits is claimed.
- Unresolved/follow-up: coordinator decides C1, assigns a single source writer for the selected contract, integrates with concurrent edits and updates roadmap evidence, then schedules the matrix/pure/real-fixture validation. No further decomposition is needed for this bounded specification. If C1 is rejected, relay isolation needs a fresh bounded contract before expanding the allowlist. Migration-contract and Web/DB work retain their existing owners.
- Not found: no additional affected Rust inclusion, relay caller, QuoteTaskError constructor/explicit variant pattern, or reference to the three removable test variants in the searched repository Rust paths. No required input file/log was missing. Other not-found items: none.
- Not verified: compilation/lint for coordinator repairs or proposed edits; item allowance behavior on the pinned compiler; default/feature/mixed/API test counts; existing expectation fulfillment; actual C2 identity/roles/DDL, relay cancellation/commit fault behavior, child restart/reaping/cleanup, non-Unix behavior, final workspace/hosted CI or production behavior. Existing log failures are evidence of failure only.

**SPECIFICATION_READY — idle.**
