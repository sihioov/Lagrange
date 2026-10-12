# R1 full-workspace follow-up — 2026-10-12

Read-only WP analysis for R1-03/R1-08. HEAD is `ac0a08364788e6c2f234cf85025a8d975f362d31`; this is the shared dirty candidate, not a clean baseline reproduction. No implementation is authorized by this report.

## Evidence and complete failure inventory

The frozen `target/r1-runtime-fxi6ltmv/workspace-followup-20261012/workspace-final.log` is 304,141 bytes, SHA-256 `037bb3591fb6c7dc7243dadce0abdd03383258a8315e9a2cf01c00dc3dcbfca2`. Its 18 failed result blocks, 42 FAILED case lines, 42 failure-detail headers and final 18-target list reconcile. There are **39 ordinary cases and three doctests**; no failed-target/case match is missing. Log lines below are one-based.

Observed invocation: `cargo test --workspace --locked --no-fail-fast --features kis-client/test-support`, offline/jobs 2, test threads 2, disposable loopback PostgreSQL port 55438, `TMPDIR=target/t.39e90c`. The preceding chart HTTP command passed. Execution/cleanup/accounting belongs to `2026-10-12-r1-workspace-validation.md`; this report neither reruns it nor infers a baseline result.

| Failed target | Log span | Cases | Family |
| --- | --- | ---: | --- |
| api-server --lib | 28–185 | 1 | E2 |
| api-server --test http_recommendations | 371–405 | 1 | E1 |
| api-server --test owner_equity_v2_db | 535–554 | 1 | F1 |
| api-server --test risk_snapshot_seam | 717–750 | 1 | F2 |
| api-server --test service_status | 774–869 | 7 | E2 |
| collectors --test research_worker | 1592–1722 | 6 | F3 |
| factor-engine --lib | 1821–1869 | 1 | U1 |
| job-queue --test candidate_runner | 2347–2371 | 1 | E1 |
| job-queue --test intraday_calendar_read_state | 2372–2397 | 1 | U2 |
| job-queue --test intraday_demand_identity | 2398–2420 | 1 | U2 |
| job-queue --test intraday_producer_lifecycle | 2440–2467 | 1 | U2 |
| job-queue --test intraday_quotes | 2496–2538 | 1 | U2 |
| job-queue --test recommendation_compute | 2646–2794 | 12 | E1 |
| job-queue --test recommendation_publish | 2810–2844 | 1 | F4 |
| job-queue --test recommendation_runner | 2845–2877 | 1 | E1 |
| kis-client --test market_stream_domain | 3120–3147 | 1 | U3 |
| market-data --test kis_range_normalization | 3750–3781 | 1 | F5 |
| kis-client --doc | 4690–4735 | 3 | F6 |

Exact names, qualified by their target above:

- E1 / http_recommendations: `http_recommendations_create_queue_result_happy`.
- E1 / candidate_runner: `synthetic_sources_schedule_compute_and_publish_one_atomic_top_five`.
- E1 / recommendation_runner: `real_worker_and_uv_publish_all_five_shipped_strategies`.
- E1 / recommendation_compute (all 12): `bounded_dynamic_factor_ids_compute_a_real_snapshot`; `async_close_uses_the_blocking_boundary`; `close_omits_null_factor_values`; `close_attests_the_pinned_manifest_before_reading_factors`; `close_rejects_an_incomplete_fixed_universe_and_future_rows`; `fixed_universe_close_is_exact_finite_and_deterministic`; `global_lookback_counts_prior_sessions_and_short_members_still_yield_null`; `malformed_parquet_fails_artifact_attestation_as_integrity`; `membership_discovery_is_scoped_to_the_attested_curated_version`; `missing_required_adjusted_component_is_data_blocked`; `semantically_invalid_parquet_value_is_integrity`; `unavailable_dataset_paths_are_transient_without_message_parsing`.
- E2 / api-server lib: `http::service_status::tests::actual_serving_path_reopens_same_commit_policy_and_revalidates_auth_before_cache`.
- E2 / service_status (all seven): `cached_future_diagnostics_keep_inclusive_assurance_then_null_expired_coverage`; `cached_halted_assurance_keeps_original_times_and_inclusive_deadline`; `current_trusted_file_and_policy_invalidate_actual_serving_caches_on_replacement`; `optional_mount_absence_is_missing_but_symlink_and_unsafe_parent_are_invalid`; `reboot_history_preserves_verified_success_but_rejects_preboot_current_observations`; `safe_reader_rejects_bad_snapshot_inventory_symlink_hardlink_permissions_and_size`; `safe_reader_reopens_replaced_snapshot_and_checks_owner_mode_and_release`.
- F1: `owner_equity_v2_repository_queue_and_publication_are_atomic_and_actor_scoped`.
- F2: `eod_unavailable_publication_never_counts_as_fresh_eod`.
- F3 (all six): `worker_cli_once_runs_collection_in_the_bounded_hidden_helper`; `worker_cli_permanent_ingest_failure_streams_failed_then_contextual_error`; `worker_cli_streams_each_validated_recovery_batch_before_cycle_output`; `worker_price_recovery_rejects_future_sessions_before_catalog_publication`; `worker_production_pool_uses_discrete_fields_role_limits_and_timeouts`; `worker_recovery_write_failure_is_retryable_and_exact_replay_is_skipped`.
- U1: `owner_equity_v2::tests::golden_canonical_snapshot_hash`.
- U2 / intraday_calendar_read_state: `calendar_read_returns_current_trading_and_closed_without_quote_prerequisites`.
- U2 / intraday_demand_identity: `legacy_exact_identity_next_renewal_remains_positive`.
- U2 / intraday_producer_lifecycle: `lifecycle_daemon_automatically_dispatches_on_real_five_second_cadence_and_shutdowns`.
- U2 / intraday_quotes: `producer_lease_heartbeat_takeover_and_stale_fence_are_enforced`.
- U3: `replacing_valid_anchor_inode_breaks_persisted_domain_binding`.
- F4: `canonical_hash_matches_python_sorted_json_semantics`.
- F5: `adjusted_price_query_is_rejected_before_normalization`.
- F6 / doctests: `crates/kis-client/src/market_stream.rs — PreparedMarketSubscriptionCommand` at documentation lines **565 and 570**, and `crates/kis-client/src/market_stream_state.rs — MarketStreamDomain` at documentation line **492**; all three are `compile_fail` blocks which compiled successfully.

Accounting by cause family: E1 15 + E2 8 + F1 1 + F2 1 + F3 6 + F4 1 + F5 1 + F6 3 + U1 1 + U2 4 + U3 1 = 42. These are reached failure phases, not 42 demonstrated product defects.

## Disjoint proposed packages and repair contracts

All source allowlists below are absolute and exclusive. The two environment families share one coordinator-owned **new private driver**, `/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/workspace-retry-20261012/run_validation.py`; no tracked-source change is needed for that package. Retain the frozen original driver/logs. Every future command requires a separately granted serial slot, no inherited provider environment, and a newly verified owned disposable QA cluster; no command below was executed here.

### E1 — local interpreter selection, 15 setup failures

Verified: log 392–395, 2358–2361, 2671–2774 and 2864–2867 all stop at `scripts/ci/prepare_phase0.py:8`, importing pyarrow. They have not reached their intended worker/attestation assertions. Driver `workspace-validation-20261012-02/run_validation.py:8–13` copies inherited PATH, omits PYTHON, and records neither PATH nor the resolved executable. The four fixture generators select `PYTHON` or bare `python`: `crates/api-server/tests/http_recommendations.rs:135`, `crates/job-queue/tests/candidate_runner.rs:155`, `recommendation_compute.rs:342–350` (also mutation helper 762–763), `recommendation_runner.rs:200–207`.

The earlier version check used `uv run --project nt python`, not this bare-PATH selection (`python-web-validation-summary.md:17–19`). The validation report independently records omission of `nt/.venv/bin`. **The exact failed executable path/version cannot be recovered from retained metadata**; do not invent `/usr/bin/python` or a version. This is a demonstrated invocation defect, not evidence that pyarrow is absent from the prepared environment. Hosted CI installs pyarrow into setup-python's interpreter before Cargo (`ci.yml:112–121`); that boundary is different and unverified here.

Future driver: pin absolute `PYTHON=/data/worktrees/3puw275b/orange-dugong/nt/.venv/bin/python`, prepend that bin directory to its explicitly whitelisted PATH, and run preflight using the exact resulting child environment. Record only executable/realpath, Python version, pyarrow version and selected safe names. Require 3.12/25.0.0 and successful import before Cargo; no install/resync or regeneration of populated `data/phase0`. Do not replace the production child launcher: `recommendation/child.rs:395–406,498–550` separately chooses verified venv/uv and clears its child environment.

Focused validation: rerun all four affected targets under the pinned environment, including **all 12** recommendation_compute cases, not just a happy path; their existing negative attestation/Parquet checks must execute. Environment-only package has no Rust allowlist.

### E2 — local TMPDIR violates the secure fixture contract, eight cases

Verified source cause: both fixture constructors use `std::env::temp_dir().join(...)` (`crates/api-server/tests/service_status.rs:898–909`, `src/http/service_status.rs:488–492`). The supplied TMPDIR is relative. `src/service_status.rs:626–633` rejects a nonabsolute root before reading any file. Simply making that worktree path absolute is insufficient: current metadata shows `target`, repo and worktree ancestors mode 0775, while 704–717 rejects writable ancestors. This explains early Invalid, HOST_INVALID/POLICY_MISMATCH and null metrics; replacement/cache/history assertions have not established a product regression.

Future driver must allocate a short **absolute**, task-owned mode-0700 directory directly beneath verified `/tmp`; use it for TMPDIR, retain its receipt, and clean only owned descendants. Preflight must run in the executor's actual filesystem namespace: verify `/tmp` is root UID/GID 0 and sticky, every later ancestor belongs to the fixture caller and is not group/world writable, no symlink, final fixture directory 0750/file 0640. Sandbox metadata is not proof of historical host UID mapping. Do not chmod worktree/global ancestors or relax `open_trusted_dir`, no-follow, regular-file, nlink=1, owner/group, exact-mode, size, release, JSON or replacement-race guards (`service_status.rs:500–733`).

Focused validation: api-server lib exact serving case, then complete service_status target in the corrected environment. Preserve all seven inventory cases and the unsafe-parent/symlink/hardlink/replacement negatives. Hosted defaults may satisfy `/tmp` already; no hosted pass/failure has been established. No API/test source edits proposed.

### F3 — collectors test endpoint drift, six pre-body failures

Verified: log 1668–1709 compares the granted 55438 URL against hard-coded 55432 before scratch DB creation. Intraday's exact endpoint is 55438 (`intraday_quotes_support/mod.rs:23,51–67,573–574`), and current CI uses 55438 (`ci.yml:106–107`). Production arbitrary DB configuration is not the repair target.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/data-pipelines/collectors/tests/research_worker.rs` only. Change the six reviewed endpoint assertions at 2009,2103,2168,2285,2346,4145 and their corresponding worker/config DB_PORT inputs at 2040,2131,2230,2314,4155 to the **same exact 55438 QA contract**. A private test constant/helper may eliminate duplication but must still reject absent/wrong URL before DDL or spawning. The two newer recovery cases already use 55438 at 2509/2885; retain them. Leave the pure config example/default assertion at 1523/1550 using 55432: it tests supplied configuration, not a connection or reviewed endpoint.

Callers: the six names in F3 above plus `worker_price_recovery_partitions_entitlements_and_replays_exact_generations` and `worker_price_recovery_resumes_published_anchor_and_rejects_source_set_drift` for consistency. Preserve env_clear, loopback host, generated DB names, role/password-file handling, synthetic provider, hidden-helper timeouts, recovery ordering/replay/source-set/future-session assertions and redaction. Focused command: `cargo test -p collectors --locked --test research_worker`; wrong-port guard must fail before DB/provider work. Do not redirect intraday back to 55432 or accept a range of endpoints.

### U2 — intraday bootstrap diagnostics are lossy; race is a hypothesis

Verified: all four failures stop at `intraday_quotes_support/mod.rs:180–183`, before generated DB creation or migrations. Their wrapper panic sites are calendar_read_state:176, quotes_support:422 and lifecycle_support:368. The helper discards the sqlx error. Root bootstrap includes cluster-global CREATE ROLE checks and repeated public-schema GRANTs (105–138), and concurrent tests execute it without a lock. The general job-queue/collectors harnesses serialize root bootstrap on `pg_advisory_xact_lock(hashtext('lagrange-test-role-bootstrap'))` (`tests/common/mod.rs:180–191`; collectors common:135–144). The same helper then repeats the script in its own scratch DB at 194–197. No DROP ROLE/ALTER ROLE lifecycle was found in the investigated API/job-queue/collectors test trees; cleanup drops generated databases, not roles.

Classification: test-harness/environment concurrency candidate; **actual SQLSTATE and exact failed statement are missing**, so neither a CREATE ROLE race nor concurrent GRANT catalog update is proven. A serial retry passing would support, not identify, the cause. Do not diagnose migration failures when execution never reached migrations.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/intraday_quotes_support/mod.rs` only. First preserve typed setup stage + sqlx database SQLSTATE (no URL/password/raw message/SQL body) in errors. For the root role-bootstrap block, use the existing shared advisory-lock key in a transaction on the **same root pool/database** before the existing SQL, then commit before CREATE DATABASE. Keep fresh-DB schema grants, least-privilege logins, exact endpoint/C2 actual-identity checks, bounded pools/timeouts and generated-name cleanup unchanged. Do not swallow duplicates or treat setup failure as a skip. Further changes require the captured error, not guessing.

Complete direct module callers: intraday_quotes, intraday_calendar_read_state, intraday_demand_identity, intraday_producer, intraday_producer_pipeline, intraday_producer_scheduling, intraday_producer_lifecycle, intraday_read_state and intraday_endurance; pipeline/lifecycle support import this helper. Do not run the ignored 30-minute endurance soak as part of minimal repair verification. Focused matrix: four failed targets first at threads 1 for diagnosis, then threads 2 on a fresh owned cluster; complete nonignored intraday targets and common-harness DB targets at threads 2 to test coexistence. Capture SQLSTATE if any fail; no passing claim yet.

### F1 — owner-policy fixture duplicates migration provisioning

Verified drift: owner test `provision_policy` at `owner_equity_v2_db.rs:266–280` inserts after `Harness::seed_user` grants Owner; migration 0053:504–534 already provisions (100,261,121) through the Owner-role trigger. Log 545 reports the policy primary-key conflict. Helper calls are `seed_owner:290` and main test:310. The migration-owner actor-context seed at common:575–590 is correct and must stay.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/api-server/tests/owner_equity_v2_db.rs` only. Replace the duplicate insert with an actor-scoped migration-owner **UPDATE of the existing row** to the test's maximum/130/121. Assert the provisioned default row exists with its default values before adjustment and the desired row afterwards; absence must fail, not silently insert or ON CONFLICT-ignore. Retain tenant/RLS/capacity race, immutable provenance, queue/publication atomicity, cross-actor denial and actor GUC assertions. Focused command: complete owner_equity_v2_db target against fresh QA; no migration/common-helper/product edit.

### F2 — risk fixture republishes one calendar identity with two provenances

Verified: `risk_snapshot_seam.rs:41–68` ingests the same multi-date recorded KRX calendar into two unrelated Raw roots/batch IDs at two retrieval times; failing test calls at 467 and 492. Their overlapping 2020-01-30 calendar fact has the same source-version string but different immutable source_batch_id/retrieved_at. Sink 885–945 and history_matches enforce those identities; log 741 is the expected Conflict, not evidence EOD_UNAVAILABLE freshness is wrong. Publication.from_raw derives calendar evidence from the verified manifest (`publication.rs:543–599`); it must not be weakened.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/api-server/tests/risk_snapshot_seam.rs` only, local fixture support and this case. At this DB publication seam, prepare the earlier Jan31 verified Raw bundle first as the **single immutable calendar foundation**; prepare Feb3 unavailable evidence separately. Require both calendar files' verified bytes/hash to agree. Assemble the unavailable test PublicationBundle with the foundation's actual Calendar PublicationFile/storage path, calendar_facts and CalendarEvidence; retain its own verified EOD_UNAVAILABLE/reference/actions files and bundle batch/date/retrieval identity. Keep both Raw temp roots alive. Do not fabricate copied_from metadata, change provider scope, edit committed bytes/manifests, rewrite evidence IDs/times or claim the assembled bundle is the original from_raw return. This exercises reuse at the existing public sink test seam, not a new production Raw adapter.

Publish unavailable first (freshness Unknown), then prior real EOD (correct age), and retain the final unavailable/replay assertions. Add a bounded negative using the unassembled conflicting calendar identity: Conflict and unchanged calendar/publication counts. Existing publication_sink `calendar_equal_time_and_source_version_conflicts_roll_back_the_whole_batch` at 545–582 remains independent safety coverage. Other raw_publication callers at 347/390 remain unchanged. Focused commands: complete risk_snapshot_seam and collectors publication_sink. Any demand to make this a new KRX production calendar-copy contract is outside this test-only repair and requires a separate decision.

### F4 — recommendation golden reflects a retired universe snapshot

Verified drift with independent proof: constant `recommendation_publish.rs:225–233` predates commit `91a4ff6`, which changed only the adopted YAML coverage floor from 2020-01-31 to 2016-08-29. `all_cash_output:58–89` reads the current AttestedUniverse, and selector `universe.rs:147–160,204–225` includes that floor in its canonical hash. Rust writer `recommendation/validate.rs:478–543` sorts object keys and uses Python default separators/UTF-8; `_common.py:175–180` is the matching Python hash contract. No production writer change is justified.

Independent stdlib-only construction from those literal fields, without importing/executing repository code, reproduces **both** emitted values: old universe `2746391f…518bc7` yields portfolio `7f52444f…59d3e4`; current universe `6823800e…c3941` yields `e66b576b…bed58` (the log's actual value). Thus this mismatch is not evidence of a JSON serialization regression. CLI wire serialization at recommendation_cli.py:312–324 is compact/newline-terminated and is deliberately separate from the hash's default separators.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/job-queue/tests/recommendation_publish.rs` only, canonical test/local literal reference support. Retain the historic serializer golden with an explicitly pinned historic universe_snapshot_id in a cloned output, and additionally assert the current live-fixture hash against independently constructed Python-compatible literal reference bytes/current floor. Do not change shared all_cash_output, rehash, production validation or universe membership to satisfy the old hash. Keep fixed known hashes plus provenance/universe sensitivity; never derive the expected hash using the same Rust function under test. Focused command: complete recommendation_publish; compare the literal reference using the pinned Python interpreter in the validation slot.

### U1 — factor-engine canonical hash: product determinism question remains open

Observed at log 1857–1860: actual `b4b55dae…846e5`, golden `2e3f4bb3…91bf`. Unlike F4, this fixture uses owner-managed synthetic symbols and has no ETF11 coverage-floor input. `owner_equity_v2.rs:299–306` serializes then **deserializes floats before scoring/hash**; 327–333 serializes the self-hash-cleared struct. The golden at 1115–1124 dates to 4b6fac3. Current HEAD and candidate have no diff in this file; that is not baseline execution evidence.

Feature evidence: frozen executable fingerprints factor-engine-753ffb9b9795ba83 and recommendation_publish-6917999cef35c378 reference serde_json dependency fingerprint `305151920385136781`; its little-endian hash matches serde_json-55dbd9b449be7348, features default/float_roundtrip/raw_value/std. API depends on jsonschema 0.49.5 (`api-server/Cargo.toml:42`); its locked registry manifest enables float_roundtrip. serde_json 1.0.151 de.rs:623–674 has different float parsing paths with/without that feature. **Feature-dependent parse rounding is a source-backed candidate cause, not a proved explanation of these exact bytes.** serde_json version is already 1.0.151 at 4b6fac3, so a simple version-update story is unsupported.

Diagnostic reservation, not closed implementation authorization: `/data/worktrees/3puw275b/orange-dugong/crates/factor-engine/src/owner_equity_v2.rs` (temporary synthetic byte/bit capture seam/tests only) and `/data/worktrees/3puw275b/orange-dugong/crates/factor-engine/Cargo.toml` (only if an explicit canonical parsing policy is later adopted). No lock/version/API/schema change preauthorized. In a reserved slot compare isolated factor-engine golden with the workspace-unified feature selection and exact baseline archive; capture self-hash-cleared synthetic bytes and differing field/float bits, not production payloads. If unequal, require a chosen stable parsing policy and compatibility proof for existing canonical snapshots before correcting a golden. Never blindly replace 2e3f with b4b5 or hide unification by narrowing the workspace command.

Affected production boundary: factor wrappers at 430–442 feed `job-queue/src/owner_equity_v2.rs:861`; API reads/verifies persisted candidates. Retain math, zero normalization, admission/source pins, row ordering, permutations, future-row isolation, serde roundtrip and tamper rejection. Focused matrix: factor-engine full lib alone/default, workspace-unified golden, and default/all-feature roundtrip/permutation/verification tests; archived stored-canonical compatibility is an additional missing input. This family needs further bounded diagnostic decomposition and coordinator decision, not a routine hash edit.

### U3 — inode replacement test is nondeterministic without retaining the old inode

Verified: `market_stream_domain.rs:271–292` unlinks then creates an empty anchor and expects a different binding, but records/retains no old anchor descriptor or inode. `market_stream_state.rs:354–369` defines identity as (dev,ino); 572–591 closes temporary anchor File values when construction returns, and 688–694 compares the persisted binding. A filesystem may reuse an unlinked inode; log 3138 contains only an accepted domain, **no before/after inode receipt**. Hence inode reuse is plausible, not observed, and a changed-inode binding defect remains unexcluded.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/kis-client/tests/market_stream_domain.rs` only, this test. Open and retain the original anchor File before unlinking, record dev/ino, recreate with the existing exact mode, and assert the replacement dev/ino differs while the old descriptor remains alive through the rejection assertion. Require InvalidState and unchanged state bytes as before. This guarantees the actual distinct-inode safety condition; no retry-until-pass, permission relaxation or production binding change. Focused command: complete market_stream_domain plus repeated exact case in the reserved slot with typed inode receipts. If distinct dev/ino is accepted, stop and classify a production defect with state/binding evidence. Guaranteeing detection of replacement even after inode-number reuse would exceed the existing (dev,ino) contract and requires separate design review.

### F5 — caller metadata tampering fails before query semantics

Verified drift: test `kis_range_normalization.rs:315–328` changes only returned ManifestEntry.query after store_batch. Normalizer `range_normalize.rs:756–760` reads Raw before `parse_range_rows:950` reaches `validate_file_request:1078–1124`. Raw reader `storage.rs:1474–1486,1574–1585` compares persisted batch.json with caller metadata; tampering therefore reaches StoreError::InvalidBatchMetadata, not InvalidQuery. The frozen log prints only the failed matches assertion; the exact returned error is inferred from this source path, not captured dynamically.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/market-data/tests/kis_range_normalization.rs` only. Keep fixture_source:70–105 as the unchanged-signature wrapper delegating to a new private fixture_source_with_adjustment(..., adjustment: &str), with default "1". The new helper sets FID_ORG_ADJ_PRC before RawEnvelope/store_batch; use genuinely committed "0" metadata for the existing InvalidQuery case. Keep a separate caller-only tamper case asserting the earlier typed Store/InvalidBatchMetadata failure. Assert neither path publishes normalized batches. All existing helper callers, including multi-window fixtures, retain their current defaults; no production ordering/storage/Raw gate changes. Focused command: complete kis_range_normalization, including replay/concurrency/window/coverage tests.

### F6 — production compile-fail assertions run under opt-in fixture authority

Verified contract drift: endpoint/config loopback constructors and domain for_test are deliberately exposed under `cfg(any(feature="test-support",test))` (`market_stream.rs:154,240`; state.rs:522). The workspace command explicitly enables that feature, as current CI/CI-contract tests require. The three unconditional compile_fail blocks therefore compile. No broader production API leak is established. Neighbor documentation block at market_stream.rs:575 happens to fail with for_test's unresolved generic parameter under this feature; that is not an honest proof of constructor absence.

Allowlist: `/data/worktrees/3puw275b/orange-dugong/crates/kis-client/src/market_stream.rs` and `/data/worktrees/3puw275b/orange-dugong/crates/kis-client/src/market_stream_state.rs`, **documentation attributes/blocks only**. Use feature-conditional doc attributes: without test-support retain correctly typed compile_fail absence examples; with test-support provide positive compile/type checks for those exact fixture constructors. Fix the 575 example too with a concrete `&Path` call/type so its result depends on visibility, not generic inference. Positive domain/config examples may be no_run/type references and must make no provider or filesystem call. Retain every other ownership/no-Clone/no-Default/no-Deserialize/private-state doctest in both selections. No doctest=false, required-features hiding, unconditional failure insertion, API widening or CI workspace command narrowing.

Focused matrix: `cargo test -p kis-client --locked --doc` (production/default), same with `--features test-support`, same with `--all-features`, then exact complete workspace command. The three observed failures plus adjacent fourth presence/absence contract must be accounted for explicitly; all other 23 observed doc results are preserved, not assumed adequate from a successful unrelated type error.

## Validation, limitations and handoff

Actual read-only checks: git rev-parse/status/diff/log/show/blame; targeted rg/sed reads of the cited definitions/callers/history, frozen log and retained driver/preflight/accounting; SHA-256 and complete log parsing; filesystem metadata only; parsing retained Cargo fingerprints; independent stdlib literal JSON/hash calculation for F4. No project module was imported, and no Cargo/test/build/DB/Docker/browser/network/provider/runtime command ran. One shell read failed at quoting before execution and was corrected; guessed module paths were resolved to the real source files. No required source/evidence file was not found.

Source diagram scan found citations into market_stream_state.rs (runtime_deployment:109/111), market_stream.rs:32 (runtime_deployment:269, before the proposed doc edits), and factor-engine dependencies (component_architecture:170/207). Test-only patches add no structural edge; doc-only inserts can shift state-file citations, so coordinator must reconcile exact evidence lines if those inserts occur. No diagram write or architecture change is authorized here. F3/F1/F2/F4/F5/U2 source scopes are mutually disjoint; E1/E2 share only the future private driver; U3 does not overlap F6.

Minimum final gate after focused repairs: retain chart HTTP gate, run exact full workspace command with corrected interpreter/TMPDIR and exact QA55438, offline/jobs2/threads2, all targets and doctests. Separate default-production doctests and strict default/all-feature/mixed feature matrix remain necessary. Existing failure names/assertions must not disappear or become ignored; fixture negatives must reach their intended gate. Counts/cleanup go to the execution worker's report, never reconstructed as passing here.

Mechanical command matrix (future execution only, from repo root in the corrected environment; DB input supplied by the owned-cluster driver, omitted here):

- E1: `cargo test -p api-server --locked --test http_recommendations`; `cargo test -p job-queue --locked --test candidate_runner --test recommendation_compute --test recommendation_runner`.
- E2: `cargo test -p api-server --locked --lib http::service_status::tests::actual_serving_path_reopens_same_commit_policy_and_revalidates_auth_before_cache -- --exact`; `cargo test -p api-server --locked --test service_status`.
- F1/F2/F4/F5: `cargo test -p api-server --locked --test owner_equity_v2_db --test risk_snapshot_seam`; `cargo test -p collectors --locked --test publication_sink`; `cargo test -p job-queue --locked --test recommendation_publish`; `cargo test -p market-data --locked --test kis_range_normalization`.
- U2: `cargo test -p job-queue --locked --test intraday_calendar_read_state --test intraday_demand_identity --test intraday_producer_lifecycle --test intraday_quotes -- --test-threads=1`, then the same with `--test-threads=2`; broaden to the listed nonignored caller targets only after setup passes.
- U1: `cargo test -p factor-engine --locked --lib`; `cargo test --workspace --locked --lib --features kis-client/test-support owner_equity_v2::tests::golden_canonical_snapshot_hash -- --exact`; repeat the latter with `--all-features` added. Isolated factor-engine's own all-features flag alone does **not** recreate workspace dependency unification. Baseline runs must be in the separate exact-revision archive, never checkout/reset this shared worktree.
- U3: `cargo test -p kis-client --locked --features test-support --test market_stream_domain`, then repeated exact `replacing_valid_anchor_inode_breaks_persisted_domain_binding` with `-- --exact` in that target. Use F6's three doc commands separately; final workspace command stays exactly as recorded.

Unresolved/follow-up: U1 canonical bytes/float policy and persisted-snapshot compatibility; U2 original SQLSTATE/statement and confirmation of proposed serialization; U3 original before/after inode identity; E1 exact failed Python binary; corrected-environment dynamic results; exact-baseline reproduction for causal attribution. No claim of baseline, hosted CI, full R1 or production acceptance. Missing inventory matches: **none**. Required files not found: **none**. Not verified: the listed dynamic/compatibility inputs; none are silently treated as passed.

Changed file: this new report only, lines 1–166. Deviations: no implementation, tests or baseline reproduction attempted by design; all frozen evidence retained and other worker reports preserved. Status: **inventory and bounded specifications ready; U1 remains an explicit diagnostic/contract decision**. Further decomposition is necessary for that family and later serial validation; no nested worker was created. Return idle.
