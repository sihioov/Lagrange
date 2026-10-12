# R1 WP-2g Rust lint validation

- Date: 2026-10-12
- Repository: `/data/worktrees/3puw275b/orange-dugong`
- HEAD: `ac0a08364788e6c2f234cf85025a8d975f362d31`
- Scope: validation only; no Rust source edits, fixes, suppressions, staging, or commit.
- Current status: **All six strict Clippy selections, formatter and requested focused follow-up tests passed.** The original failed runs below are historical; the follow-up begins at the notified-follow-up section. Full workspace tests, the actual-serving DB test and hosted CI remain pending.

## Resource and source integrity

- Cargo commands ran serially, offline, with `CARGO_HOME=target/r1-runtime-fxi6ltmv/cargo-home`, `CARGO_TARGET_DIR=target/r1-runtime-fxi6ltmv/cargo-target`, `CARGO_NET_OFFLINE=true`, `CARGO_BUILD_JOBS=2`, and `COMPOSE_PARALLEL_LIMIT=1`. Provider, DB/PG, supervisor, binding, evidence, runtime, KIS, DART, broker, and account environment variables were removed from child environments by name matching; the only matched name present was `XDG_RUNTIME_DIR`. No values were recorded. No RUSTFLAGS or lint cap was introduced.
- SHA-256 inventory covered all 572 tracked and nonignored Rust source paths (also including any non-target untracked Rust files). Before and after aggregate path/content hash: `80c48f13b648ffe8c11f6efdece7ae505dfd3ec8f8e7b3d6bee3aac683a03685`; manifests compare identical.
- Full per-file hash manifests, exact argv, UTC times, exits, bounded logs, and log hashes are in ignored private evidence directory [`rust-lint-validation.20261012T074734Z.3.0`](../../target/r1-runtime-fxi6ltmv/rust-lint-validation.20261012T074734Z.3.0/). Logs were kept mode 0600; largest test log was under 24 KB. No payloads, credentials, or provider response content were captured.
- Before and after checks in the worker process scope found no validation-owned Cargo, rustc, rustdoc, pg_ctl, postgres, Docker, Node, or Playwright process remaining. This is not a host-wide absence claim; pre-existing production services and the foreign Next server remain outside this task. Available memory was 9.3 GiB before and 9.7 GiB after. No owned process needed cleanup.

## Strict Clippy and formatter matrix

Each exact command below ran once with `--locked` where specified, no RUSTFLAGS masking, and no cap-lints. The repeated error in rows 4 and 7 is the same one diagnostic in the source-included parser copy.

| # | Exact command | Exit | Diagnostic summary | Log SHA-256 |
|---:|---|---:|---|---|
| 1 | `cargo clippy -p job-queue -p api-server --all-targets --locked -- -D warnings` | 0 | 0 warnings/errors | `725b36ac1748c74b991f2854a0b84b6f58317a8e722ef0e15230f21c9397a69e` |
| 2 | `cargo clippy -p job-queue --lib --tests --features market-stream-db-tests --locked -- -D warnings` | 0 | 0 warnings/errors | `d20aab6dc95141849b22d9917f744a10f84810da45abc9eef13737681fc8a37f` |
| 3 | `cargo clippy -p job-queue -p kis-client --lib --tests --features job-queue/market-stream-db-tests,kis-client/market-stream-provisioning --locked -- -D warnings` | 0 | 0 warnings/errors | `b2b3c5bae2b958ad5012ea6c44ede59658e22bd705dc081ad4c75c017ba17067` |
| 4 | `cargo clippy -p api-server --all-targets --all-features --locked -- -D warnings` | 101 | One source diagnostic: `unexpected cfg condition value: test-support` at `crates/api-server/tests/owner_market_stream_integrated_support/../../../kis-client/src/market_stream_wire.rs:300`; API test target declares only `market-stream-db-tests`. | `ec27430ffe632583ddcf3690332ccb0357339a617bf3b63f802d9c86a657767f` |
| 5 | `cargo clippy -p kis-client --all-targets --all-features --locked -- -D warnings` | 0 | 0 warnings/errors | `36b8cf4594b8b0c77d087a6b4f9ec08a56d7a08c5cd5c80a7eca16535d30c1b6` |
| 6 | `cargo fmt --all --check` | 0 | clean | `ebe3f4bdf5dc11b3ad9eb0623bdb94149e8b2f592e03616009025d6830f19d22` |
| 7 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 101 | Same one source-included `test-support` unexpected-cfg diagnostic as row 4. | `ca2e67fe842edeb9ec85cbcc63c6ae376c4651e4bc2a54a13712b67df31a2467` |

No source diagnostic was changed or suppressed. Both strict failures point to the accepted C-WIRE source-inclusion concern already noted in the implementation handoff; resolution is outside this validation-only scope.

## Focused tests

Commands below ran once each with the common offline Cargo environment and `--locked`. Counts are `passed / failed / ignored / filtered`; “not applicable” means Cargo reported no test result for that package binary in a multi-package command. A filtered selection is reported separately and is not counted as a skip. No `--ignored` tests were run.

| # | Exact command | Exit | Executed / ignored / filtered | Log SHA-256 |
|---:|---|---:|---:|---|
| 1 | `cargo test -p job-queue --lib --locked` | 0 | 219 / 0 / 0 | `790353e17dbdab393ce096e745ee1c1dc040ecf3434b7318c322effa12b6b069` |
| 2 | `cargo test -p job-queue --bin owner-equity-v2-runner --locked` | 0 | 21 / 0 / 0 | `063566b598a1b4bec4a7d65b7591678075f2608f1a3291ab77303b4d89db387d` |
| 3 | `cargo test -p job-queue --bin owner-equity-v2-runner --all-features --locked` | 0 | 21 / 0 / 0 | `835d09fdbd070594c8c55d61d6cfd46a6fdff95416653c70a684b811ed650312` |
| 4 | `cargo test -p job-queue --test owner_market_stream_boundary --locked` | 0 | 1 / 0 / 0 | `5f4425eb93691cab57c90f44422524e53160387550f3c08463fc302eac1fb82d` |
| 5 | `cargo test -p kis-client --lib --all-features --locked` | 0 | Unit tests 196 / 0 / 0 / 0; additional harness 1 / 0 / 0 / 195 | `56e1dc19ab52e96f16c7137e4c71bd633c2e9169479e556a6e025ff3510aa824` |
| 6 | `cargo test -p api-server --lib --locked` | 101 | 135 / 2 / 0 | `40b19e31d2d925cab2e14eb02bb005f9dbb535c9174a645f1242a4abd2b71eda` |
| 7 | `cargo test -p api-server --test owner_market_stream_delivery --locked` | 0 | 23 / 0 / 0 | `8889b8c1b5ad25ee96181bc5b57fe7a8d589f78dc4b91e4cbb4a69e17218b763` |
| 8 | `cargo test -p job-queue --lib --features market-stream-db-tests loopback_lifetime_tests --locked` | 0 | 4 / 0 / 325 | `2a22611a51183f64e20e48bafeb9d859b91aed0c7ce843b58c3d2fec66f9e0f0` |
| 9 | `cargo test -p job-queue --lib --features market-stream-db-tests c2_fixture::tests --locked` | 0 | 5 / 0 / 324 | `f7d45b937d31763bbf8f647e62a9a464a92dbe6a633e6e75d2ca0dba94a5e754` |
| 10 | `cargo test -p job-queue --lib --features market-stream-db-tests original19_closed_diagnostic_unit_tests --locked` | 0 | 12 / 0 / 317 | `5766459bd41d35e5271e03c8f3b7fc2376d329686d4e586510b89e5619eb7ec6` |
| 11 | `cargo test -p job-queue --lib --features market-stream-db-tests closed_failure_diagnostic_unit_tests --locked` | 0 | 6 / 0 / 323 | `1b80765c6a498f1990a49fac52a5c0e76d4528e6153d9f0d6442010d91ce1ff3` |
| 12 | `cargo test -p job-queue --lib --features market-stream-db-tests original20_stage_diagnostic_unit_tests --locked` | 0 | 6 / 0 / 323 | `7245470672ffccdc26fd54c7f458b829f21c4345a7a82a16897b800659156f65` |
| 13 | `cargo test -p job-queue --lib --all-features original19_closed_diagnostic_unit_tests --locked` | 0 | 12 / 0 / 317 | `d724af19fcc62f0285168d4ef4bc0892e646358896411ec83d8a822bd39de82c` |
| 14 | `cargo test -p job-queue -p kis-client --lib --features job-queue/market-stream-db-tests,kis-client/market-stream-provisioning original19_closed_diagnostic_unit_tests --locked` | 0 | job-queue 12 / 0 / 317; kis-client 0 / 0 / 196 | `762f8ee250d612d76075a92f4f398dbf8899725b13e924b7b5be158aa2650f70` |
| 15 | `cargo test -p api-server --test owner_market_stream_http --features market-stream-db-tests loopback_lifetime_tests --locked` | 0 | 4 / 0 / 11 | `99ce68fa7dcac76cbb722f1adbef24aeb41d2843d00aa34d9f272cf3451604bb` |
| 16 | `cargo test -p api-server --test owner_market_stream_http --features market-stream-db-tests c2_fixture::tests --locked` | 0 | 5 / 0 / 10 | `93abb5c78a1343f7e846e65fbd0066fb3f170e5462096dcf73fe8c9d458140b5` |
| 17 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests loopback_lifetime_tests --locked` | 0 | 4 / 0 / 39 | `92e83505ac70ee0cecd2d1b8b87657d8b69ea18b83c4f542112e66e6e065d142` |
| 18 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests c2_fixture::tests --locked` | 0 | 5 / 0 / 38 | `8e214133ba8a9ff7c2d9ef0479cb105c5785894a37a096cf4f546f91ab4f2f10` |
| 19 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests reviewed_wire_contract::tests --locked` | 0 | 12 / 0 / 31 | `24473ec813adfd58f15e192e7668d7c9de14e1d59c12851ad98e86f067e5b5a5` |
| 20 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests synthetic_wire_tests --locked` | 0 | 2 / 0 / 41 | `4b9dda81b39ea56cf77601748b3dab715dbde71863eb6f1126e19069ee2761ba` |
| 21 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests measurement_transport_summary_omits_event_history --locked` | 0 | 1 / 0 / 42 | `6ca5ebce0e57b38c9520eee9e7a1be9c0c8e4b0bb901b6b898724627311a2dae` |
| 22 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests measurement_final_acceptance_rejects_missing_or_unfinished_probe --locked` | 0 | 1 / 0 / 42 | `616f3cd9cf03f277a24d38c3eeb4d98d55031ff6c78090870c54297db0897f7d` |
| 23 | `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests measurement_publication_document_is_bounded_metadata --locked` | 0 | 1 / 0 / 42 | `08919713380b3160400b7cf70f7f2d6a3a0e6790ca309ec6c5e52a325a241d67` |

For API default `--lib`, one test intentionally panicked because the required `DATABASE_URL` was removed; it did not connect to a database. A second local-listener test failed because the sandbox denied binding (`PermissionDenied`); no listener was started. These are environment constraints, not passing skips. The two checks remain unresolved. No provider/C2 access or database operation occurred. The `c2_fixture::tests` selections above only exercised synthetic binding/path validation and SQLx URL parsing; their fixture code was inspected before execution.

## Deviations and follow-up

- Deviations from the authorized command scope: none. No source fix or retry was made.
- Unresolved: strict API all-features and workspace Clippy remain blocked by the source-included `test-support` cfg diagnostic at `market_stream_wire.rs:300`; API default lib has the two environment failures above. These require coordinator routing to the owning source/test environment.
- Not verified: real DB/role behavior, Docker, provider/runtime behavior, ignored tests, full workspace test acceptance, CI, and the API serving test with its disposable DB. None was attempted.
- Missing inputs: none. Further decomposition: not needed for this validation report.
- Final source manifest matched the initial manifest exactly; no test inventory or source file changed during this validation. No staging or commit was performed.

**VALIDATION COMPLETE — HEAVY SLOT RELEASED.**

## WP-2g notified follow-up — 2026-10-12

The accepted root follow-up changed only `crates/kis-client/src/market_stream_wire.rs` and `crates/api-server/tests/service_status_support/mod.rs` after the earlier validation snapshot. This worker made no source edits. The previous Clippy failures, API library failures, and their original logs above are preserved; the two Clippy failures are superseded by this follow-up matrix, while the broad API default-library run was not repeated.

### Follow-up source and process integrity

- Before/after snapshots each contain 572 tracked and nonignored Rust source files and have the same aggregate path/content SHA-256: `1e07ba690da2e59126a0179710a159d7717034b35da82760301a6f6521f1128d`. The per-file manifests are `rust-sources-before.json` and `rust-sources-after.json` in the ignored private evidence directory below; they compare byte-for-byte identical.
- Compared with the preceding validation snapshot, exactly the two coordinator-reported files differed. Their current hashes matched the supplied expected values: `market_stream_wire.rs` `bd39fa02f99568845760a1f80c5152cc4c99eb50b0161f7968778301fc4bfb53`; `service_status_support/mod.rs` `0d13c5247bf3e36d9a65f798f04e8c8cf3306e9a5bc02c1b8803e99bed4e33fc`.
- Evidence is under the ignored private directory [`rust-lint-validation.followup.20261012T081034Z.3.0`](../../target/r1-runtime-fxi6ltmv/rust-lint-validation.followup.20261012T081034Z.3.0/). Exact argv, sanitized logs, result metadata, and hashes are retained there. Cargo used the same offline cache, target directory, and jobs setting. The only matched environment name removed from child processes was `XDG_RUNTIME_DIR`; no value was logged. No lint masking or extra allowances were used.
- At completion, no validation-owned Cargo, rustc, or rustdoc process remained for the current user. The exact owned test process had exited. This is a worker-process-scope check, not a claim that the host has no foreign services; the existing Next server was not inspected or touched.

### Follow-up strict matrix

All six Clippy selections and the formatter passed. The six strict Clippy rows are distinct from `cargo fmt`.

| # | Exact command | Exit | Result | Log SHA-256 |
|---:|---|---:|---|---|
| 1 | `cargo clippy -p job-queue -p api-server --all-targets --locked -- -D warnings` | 0 | strict Clippy clean | `c9ce61d1372f53e288a1e3b1aea50cca48e2616ca5a59c4c92e0a9a41eef635d` |
| 2 | `cargo clippy -p job-queue --lib --tests --features market-stream-db-tests --locked -- -D warnings` | 0 | strict Clippy clean | `aa36078c67df7b2b86af8b5881b2c271396e50db510c7aadf163fee6e690bec3` |
| 3 | `cargo clippy -p job-queue -p kis-client --lib --tests --features job-queue/market-stream-db-tests,kis-client/market-stream-provisioning --locked -- -D warnings` | 0 | strict Clippy clean | `4078bbcee5236082229f13bbb7ff935efc753a8d8617a2f5627df34640b7dfff` |
| 4 | `cargo clippy -p api-server --all-targets --all-features --locked -- -D warnings` | 0 | strict Clippy clean; source-included test-support cfg now compiles under this target | `ef5e570884d4c86550735afc490ee1145d9b681171d9b946a80b2b843789107c` |
| 5 | `cargo clippy -p kis-client --all-targets --all-features --locked -- -D warnings` | 0 | strict Clippy clean | `79e226c91b858edf7e591c8480770b69de5338c4c1189d0ecf74dfa9e6cb3799` |
| 6 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 0 | strict workspace Clippy clean | `79d355c97cb1d892d8ed5fd82dbffa2f71980ab0226bb060593e0f73cb141418` |
| — | `cargo fmt --all --check` | 0 | clean | `873c7562d48d23e25baf0feb105e9b71503bb3d1ec60b867c8ee4b02f2560403` |

### Follow-up focused tests

Each requested test selection was discovered and executed with a nonzero count. There were no ignored or skip-marked tests. The kis-client command also ran its additional harness, with 1 pass and 195 filtered; its unit-test target ran all 196 tests.

| Exact command | Exit | Executed / failed / ignored / filtered | Log SHA-256 |
|---|---:|---:|---|
| `cargo test -p kis-client --lib --all-features --locked` | 0 | unit tests 196 / 0 / 0 / 0; additional harness 1 / 0 / 0 / 195 | `e8875bd95f49230bea91bec4e48b44a96d32c49c8150157287b0d6788ad25bc6` |
| `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests reviewed_wire_contract::tests --locked` | 0 | 12 / 0 / 0 / 31 | `d5df952bb7d4b259f114164895fbd45ecb139d5bba0e5683c5628b82ab5b0441` |
| `cargo test -p api-server --test owner_market_stream_integrated --features market-stream-db-tests synthetic_wire_tests --locked` | 0 | 2 / 0 / 0 / 41 | `5132f6ce6e2ba17c0fe684c2fe4504c3cb425934540ff6feb8e271b9c281d343` |
| `cargo test -p api-server --lib --locked runtime::tests::graceful_shutdown_forces_a_stuck_request_after_deadline -- --exact --test-threads=1` (initial sandbox attempt) | 101 | 0 / 1 / 0 / 136; exact test discovered, local listener bind denied by sandbox | `724104f05c92923e7e4eaebad29d541bbf84c4304e5838737be932f38e1edbe6` |
| `env -u XDG_RUNTIME_DIR CARGO_HOME=/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/cargo-home CARGO_TARGET_DIR=/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/cargo-target CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1 cargo test -p api-server --lib --locked runtime::tests::graceful_shutdown_forces_a_stuck_request_after_deadline -- --exact --test-threads=1` (scoped native retry) | 0 | 1 / 0 / 0 / 136 | `27234cb2471e447b2f719c5e27d2cc7ab8e73060db062af6f45192ac80e5d7a4` |

The native retry ran only the authorized exact API shutdown test with loopback access; it passed. No DB-serving test, database, provider, Docker, Node, or browser operation ran in this follow-up.

### Follow-up disposition

- Deviations: none. The initial sandbox-denied exact test was retained and retried only through the specifically authorized scoped native invocation.
- Unresolved: the actual-serving test from the original API default-library run still requires the separate database slot. The exact shutdown test that previously failed on the sandbox listener restriction passed in the scoped native retry above; the complete API library selection was not rerun. No fix is proposed here.
- Not verified: full workspace test acceptance, hosted CI, actual DB acceptance, provider/runtime behavior, and overall R1/host acceptance.
- Missing inputs: none. Further decomposition: not required.
- Follow-up report append: lines 74–121. No staging or commit was performed.

**FOLLOW-UP STRICT LINT AND FOCUSED TESTS COMPLETE — HEAVY SLOT RELEASED.**
