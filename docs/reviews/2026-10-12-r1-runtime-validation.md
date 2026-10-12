# R1 C2 Runtime and Role Validation

Focused environment-only validation on HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`. No tracked source was edited, staged, or committed. No controller, ignored integrated/browser, provider, production, restart, A/B, fmt, or Clippy command ran in this follow-up.

## Fixture and execution

- Reused run `62f45acd-50b9-486c-b21b-5d24c5c99115`, system ID `7695695737867239441`, data root `/tmp/lagrange-kis-c2-62f45acd50b9486cb21b5d24c5c99115`; no reinitialization.
- Reused binding `/tmp/lagrange-kis-stream-r1-f22f4c7b006550e78059/wp3c2-runtime-binding.json`, SHA-256 `1aa2b11083464780a625256c5163ea180cad29a9462dfe0c95534319884136ec`.
- `target/t.39e90c` was canonical, empty, UID/GID 1000, mode 0700. Worst-case tempfile socket path was 80 bytes; the guard value including `+16` was 96 bytes, strictly below 108.
- Supervised PostgreSQL PID 2726156, `/proc` start ticks 417162406; observed PG 18.6 / server_version_num 180006, role `c2_supervisor`, database `postgres`, Unix socket only, no TCP listener; fsync/full_page_writes/synchronous_commit on.
- Cargo was serial, offline, locked, jobs=2, per-test timeout 240s; ambient PG*, DATABASE_URL and provider/runtime variables were cleared by name. Caches unchanged.
- Exact common command forms: `cargo test -p job-queue --lib --features market-stream-db-tests --locked <full selector> -- --exact --test-threads=1 --nocapture` (1–12); `cargo test -p api-server --test owner_market_stream_http --features market-stream-db-tests --locked <exact test name> -- --exact --test-threads=1 --nocapture` (13–18).

## Exact selections

| # | Exact selector | Exit; pass/fail/ignored | Log SHA-256 |
|---:|---|---:|---|
| 1 | `owner_equity_v2::market_stream_c3a_tests::owner_market_stream_real_role_boundary` | 0; 1/0/0 | `a0032ab28cf93a8785cd63e5dbbb1b9bc5ade19ae055d7d5b3c4ef2a0be65449` |
| 2 | `owner_equity_v2::market_stream_c3a_tests::owner_market_stream_panic_body_still_cleans_generated_database` | 0; 1/0/0 | `214dcafb5aba0715b1bf88b176b79e99c267b84f6dc5a0c783e257c2aee73542` |
| 3 | `owner_equity_v2::market_stream_c3a_tests::owner_market_stream_pending_identity_rejection_and_unsubscribe` | 0; 1/0/0 | `12bef6582a45029b2df81cb982647cc9e48d6a66b57ae2fb6879a294340e6c3c` |
| 4 | `owner_equity_v2::market_stream_c3a_tests::owner_market_stream_ack_cannot_cross_credential_slots` | 0; 1/0/0 | `72303821185551025a5ac0c4a47a7fcf71e7a0ddb5625a5e8a9826f0a667ee5b` |
| 5 | `owner_equity_v2::market_stream_runtime_storage_tests::database_cases::reason_gap_and_retirement_preserve_quote_version_and_capture` | 0; 1/0/0 | `2939e1be18799bfc74e0360cf677ae265db534c14dba68387878375aae72f1f0` |
| 6 | `owner_equity_v2::market_stream_runtime_storage_tests::database_cases::exact_epoch_and_fence_expectations_reject_stale_status_and_retire` | 0; 1/0/0 | `e93b12a404a3cac30125c6d4dbb24f1db2edb00d2263a94c111e99267a2c878a` |
| 7 | `owner_equity_v2::market_stream_runtime_storage_tests::database_cases::resolved_epoch_rechecks_close_after_locked_work_and_rolls_back` | 0; 1/0/0 | `aad30698bffdb7df861c0386e45c3e0315a2b8a8fccaea01af9b4f0d985b3150` |
| 8 | `owner_equity_v2::market_stream_runtime_storage_tests::database_cases::resolved_publication_rechecks_close_after_locked_work_and_rolls_back` | 0; 1/0/0 | `199b3865a2ee0ec98968bd5c0556fefa81797c028f2dd5f1da162599a7fb10b2` |
| 9 | `owner_equity_v2::market_stream_runtime_storage_tests::database_cases::actor_snapshot_contention_keeps_original_deadline_and_rollback` | 0; 1/0/0 | `60a4ed8715da428123e46732b7ea19e5622cd12723295ca4cbfa7d8e106d9f50` |
| 10 | `owner_equity_v2::market_stream_runtime::runtime_database_tests::runtime_no_demand_keeps_transport_unopened` | 0; 1/0/0 | `55a985773002d137d649be9a1f5ff00641e5cf5105e15537d3d03b8aa8f2222e` |
| 11 | `owner_equity_v2::market_stream_runtime::runtime_database_tests::runtime_missing_window_keeps_positive_demand_off_transport` | 0; 1/0/0 | `4b301fc3884f36063ad41702baadb0a75408609a31a3d90ffc5a541e80bfdbe7` |
| 12 | `owner_equity_v2::market_stream_runtime::runtime_database_tests::runtime_thirty_symbols_share_one_socket_across_ten_consumers` | 0; 1/0/0 | `5b4960249b5f4b88472b7067460e067e61504052ba4a0a4173dd5a152c6ff415` |
| 13 | `binding_helper_is_app_only_and_reversible` | 0; 1/0/0 | `1e8009bc1511bcec5f8455a988d773cf81a477dc5f06d1f93282a9aa29c3ccbb` |
| 14 | `leases_authenticate_and_fence_consumer_session_and_sequence` | 0; 1/0/0 | `33923068601c814669077aa110fc60ad285d11e10f17a84e88952aad21ee4f7b` |
| 15 | `sse_initial_snapshot_is_private_and_read_only` | 0; 1/0/0 | `702432dc8fa1d0a55cf61a9ada48118bc6404a0c1b813e9cf6dcd653cd6697c5` |
| 16 | `sse_replacement_and_revocation_clear_old_memberships` | 0; 1/0/0 | `2f9dc0eb485a1e8310b8dcb7a4ce1de693ab20df37ae58adb6f6963355efec74` |
| 17 | `twenty_consumers_are_bounded_and_drops_release_capacity` | 0; 1/0/0 | `95ef772a2471bafbb28d8ce3f4f1da5a23ca31b01375418ddb33e5c126136d6a` |
| 18 | `disabled_or_revoked_binding_denies_demand_and_stream` | 0; 1/0/0 | `7f425e5510f317280c8313164a4edb4dc57077333232b3f8fff0131471853598` |

Result: **18/18 passed**, zero failed, zero ignored, zero skipped, exact discovery one selected test per invocation. Each run used `--exact --test-threads=1 --nocapture`; bounded logs are under `target/r1-runtime-fxi6ltmv/wp2-heavy-validation-20261012-03/runtime-role-20261012-01/logs/`. `matrix.json` contains full argv, elapsed time, exit/count summary, output hashes, and per-run log references.

The intentional panic test at `market_stream_c3a_tests.rs:2914` passed its `JoinError::is_panic()` branch; shared cleanup at lines 71–117 verifies exact generated database absence and zero connections. The selected database storage/runtime cases are the lint-contract selections at `docs/reviews/2026-10-12-r1-rust-lint-contract.md:144–156`; HTTP selectors are the six top-level tests in `crates/api-server/tests/owner_market_stream_http.rs:16–50`. Their source assertions were not modified.

## Source integrity

Before/after manifests are `target/r1-runtime-fxi6ltmv/wp2-heavy-validation-20261012-03/runtime-role-20261012-01/source-hashes-before.json` (SHA-256 `a2a965b0b391a75f6e990adc31181cc6df0d13d7fb4385d8be731ef767ab6e4b`) and `source-hashes-after.json` (SHA-256 `25d74e85aec2960229d8dac1b43b17cf99d887de4894d3127070ee8f466fec8b`); all seven source hashes matched exactly. HEAD remained unchanged.

| Source | SHA-256 (before = after) |
|---|---|
| `crates/api-server/tests/owner_market_stream_http.rs` | `f826fa1e4e7cc3d2c2d90c441a03395898597a7204ad93fbd8cc3efa84404db4` |
| `crates/api-server/tests/owner_market_stream_http_support/mod.rs` | `cb9e8ea2a2a474f69de87491b9f6096f8e2c44395e0301fee513f8c3f3c46855` |
| `crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs` | `af67896c3d9c51a761b155e9f01710db0120e7dcfb3742a94f50b365994d2aed` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs` | `7d0eddfbcbec2a03901e402ac95c44c95a00e44f3a6c689fd3f99e12d9444984` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime_storage_tests.rs` | `e446a1f1f43bb6515a388bd84debb01b8f2c0685a69f2ac741797b81fb05aa07` |
| `crates/job-queue/tests/owner_market_stream_boundary_support/c2_fixture.rs` | `e173ae02af88be7fd1db014f336c19c9a5abc073f02fff3dc1bf14d307c28195` |
| `docs/reviews/2026-10-12-r1-rust-lint-contract.md` | `2c1808d71bf93f909a43f31e3dc1576fece70ebfb6ce5e93c16b5338d49f1ce8` |

## Cleanup, deviations, and scope

- Baseline and postmatrix SQL snapshots matched (SHA-256 `db0bff98f76d1377b6f4583e31ec8998c44553c9f6fb38a3bdb7fa8a8ee80324`): no `lagrange_ws3a_*` databases, zero other client connections, only `postgres` non-template database; fixture role names were identical at baseline and end.
- Graceful `pg_ctl -D <data> -m fast -w -t 30 stop` exited 0; same PID generation absent, supervisor exited 0, `postmaster.pid` absent, socket directory empty, port 55471 listener absent, `pg_controldata` reports shut down with system ID 7695695737867239441. Data and binding retained.
- Harness-only corrections before matrix execution: created its fresh private `restart-evidence` directory; added the already verified PostgreSQL library paths for local `psql`; excluded PostgreSQL background worker processes from the client-connection count. These attempts ran no Cargo selectors and indicated no product failure.
- Old corrupt C2 and legacy WS-3A artifacts were not written; foreign services were not contacted or inspected. Earlier reports/logs were preserved.
- Provenance limitation: the requested contemporaneous inventory of all572 Rust files was not captured; the actual manifests contain six Rust files and the lint contract. Independent retrospective comparison found the same572 paths, with only the accepted C2 fixture rotation changed from the prior strict manifest; the omitted runtime tests match that earlier manifest. This does not reconstruct unrecorded intermediate edits. The [independent review](2026-10-12-r1-review.md#wp-5-focused-runtime18-evidence-review--2026-10-12) accepts the focused18 tests with this limitation. Full workspace, actual browser, CI, and production acceptance remain pending; heavy slot released.
