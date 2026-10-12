# WP-2b Market Data Integration Test Repair

Date: 2026-10-12 (Asia/Seoul)
Roadmap item: R1-03 — Rust workspace test recovery
Accepted scope: WP-2b integration-assertion repair specification in `2026-10-12-r1-baseline.md`
Status: **FOCUSED_VALIDATION_COMPLETE** (does not claim full-workspace acceptance)

## Changes

- `crates/market-data/tests/calendar_publication.rs:329-437` — updated invalid calendar response coverage to test missing-date, malformed, and wrong-date responses both with and without response marker `M`, with no Raw visibility. Replaced the stale rejection assertion with successful single-page capture, persisted marker and payload/hash checks, normalization without a marker, and exact target-date publication.
- `crates/market-data/tests/publication.rs:89-197,281-330` — added private Raw snapshot and caller-mutation assertions, a synthetic persisted-size metadata corruption helper, and a request-map helper that runs before `RawEnvelope` construction while leaving existing call sites unchanged.
- `crates/market-data/tests/publication.rs:540-743,1136-1240` — retained exact caller-only `InvalidBatchMetadata` rejection and unchanged Raw snapshots; added reopened, hash-verified persisted size corruption cases for bigint overflow, evidence-before-parse precedence, and size mismatch; added a correct-size malformed-Bars control; retained both caller-mutated and committed malformed-provenance cases as subcases in the existing lineage test.

This task changed only the two assigned test files and this report; all other pre-existing worktree changes were preserved. No tests were added as ignored or skipped.

## Checks executed

Cargo commands used the assigned cache and offline environment: `CARGO_HOME=target/r1-runtime-fxi6ltmv/cargo-home`, `CARGO_TARGET_DIR=target/r1-runtime-fxi6ltmv/cargo-target`, `CARGO_NET_OFFLINE=true`, `CARGO_BUILD_JOBS=2`.

- `cargo test -p market-data --test calendar_publication --test publication --locked --no-fail-fast` — exit 0. Calendar: 7 passed, 0 failed, 0 ignored, 0 filtered. Publication: 24 passed, 0 failed, 0 ignored, 0 filtered. All 31 named test cases reported `ok`. The 7/24 test-count inventory matches the baseline; the stale calendar test was renamed per the accepted spec, and the two lineage cases remain in one existing test. No early-return pass or skipped test was added.
- `cargo clippy -p market-data --all-targets --all-features --locked -- -D warnings` — exit 0.
- `rustfmt --edition 2024 --check crates/market-data/tests/calendar_publication.rs crates/market-data/tests/publication.rs` — exit 0.
- `git diff --check` — exit 0.

Filtered, bounded logs and SHA-256 sidecars are in `target/r1-runtime-fxi6ltmv/wp2b-heavy-validation/`:

- `cargo-test-market-data.log` — 38 lines, 2,905 bytes; SHA-256 `4946a4d5ec4150ee4948b98a5ffa51a0d68339168ff38f6b546cc8e7d74b8464`.
- `cargo-clippy-market-data.log` — 89 lines, 3,369 bytes; SHA-256 `8dd64887e904badadb492dfa60e7302752829ec6f09aea22f722da4ac3138966`.
- `static-checks.log` — 4 lines, 177 bytes; SHA-256 `abd0a5d008a84901f0fe5d65e7adeaee664464c7059feab8397695fd746110eb`.

Only filtered test names/results, compiler progress/error headings, and command metadata were retained; raw command output was removed after successful runs. No Docker, npm, database, provider, or production-runtime operation was run.

## Deviations and follow-up

- Deviations from the accepted specification: **none**.
- Further decomposition: **not needed** for this two-test-file repair.
- Unresolved in assigned scope: **none**.
- Not verified: full-workspace or hosted CI acceptance; this handoff covers only the specified market-data targets and Clippy command.
- Not found: **none**.
