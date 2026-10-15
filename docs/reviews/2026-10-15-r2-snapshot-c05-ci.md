# R2 snapshot C05 source and hosted verification acceptance

The coordinator accepts WP-4/I1b-S2's bounded Linux snapshot source and hosted
verification on **2026-10-15**, candidate
`7cdf3a68c754231bd4d828305ff6733638bcfe37`.
[CI run 38664342216](https://github.com/sihioov/Lagrange/actions/runs/38664342216),
attempt 1, completed SUCCESS at 01:32:59 UTC. This is partial R2 acceptance;
R2-04/R2-09 remain DOING and final C_R2 is not accepted.

## Exact candidate and launch

Repository sihioov/Lagrange, branch excited-dolphin, workflow CI/ci.yml,
workflow_dispatch. Remote SHA matched and the all-event exact-head query returned
zero at 01:11:57.229 UTC. Exactly one dispatch created this run at 01:12:05 UTC.
Its first propagation query was empty; no redispatch, rerun or cancellation occurred.
The coordinator worked directly, with no new worker or local compiler, Clippy,
test, project-helper, database or provider execution. Local checks were limited
to source/text/hash inspection and the scoped standalone edition-2024 rustfmt check.

All eight jobs and all 84 step states were recovered: 83 steps succeeded and the
PG failure-log-only step was correctly skipped. All eight completed job logs were
read once. Seven actual `git log -1 --format=%H` checkout rows match the candidate;
the required aggregate does not perform a checkout. All 69 frozen inputs match,
including 52 tracked files compared with the candidate's committed blobs.
Later documentation HEAD changes do not change these source or workflow bytes.

## Observed results

| Job | Outcome | Direct evidence and limits |
| --- | --- | --- |
| workspace-tests | PASS | All 99 expected Linux Rust result names have exactly one direct ok row in the workspace phase. All six chart names have direct ok rows in the separate chart phase; their six workspace repeats remain separate. QA cleanup succeeded. |
| clippy | PASS | Exact --workspace --all-targets --all-features --locked -- -D warnings command completed. No compiler or lint error diagnostics. |
| format | PASS | Hosted cargo fmt --all -- --check completed. |
| postgres-integration-validation | PASS | APPROVED; 12 completed DB labels including both daily-admitter helpers, 59 candidate-matched SHA384 migration checksums, equal second pass/NO_OP, 0039 guard, catalog and seven role-login checks in each lane, PASS_NO_OWNED_CONTAINERS. |
| web | PASS | OpenAPI, lint, typecheck, tests, build and E2E steps completed. |
| policy | PASS | All wrapper steps and 10 directly printed CI-contract unit names passed. Unprinted nested cases are not inferred. |
| python-tests | PASS | 485 passed, 10 skipped, seven warnings; backup-policy step passed. Quiet individual readiness/SIGTERM nodes remain unverified. |
| required | PASS | All seven execution-job results were success. |

The 99 Rust names comprise **30 wire + 18 S1 Raw + one collector + two helpers +
19 S2 private + 29 S2 integration**. Thus all **48 Linux S2 cases passed**. Of the
49 written S2 definitions, `s2_t31_non_linux_returns_unsupported_without_path_io`
is excluded on Linux. S1's `fails_closed_without_unix_descriptor_relative_opens`
is also excluded. The 2,948 direct Rust rows across phases are not a unique-test
count and do not replace the 99-name inventory or six separate chart rows.

PG's pinned run_db_test wrapper at validate.sh:1377–1388 supplies DATABASE_URL,
requires a successful nested Cargo command and rejects SKIP markers before
appending a label. The two helper selections are at lines 1706–1707. The hosted
labels and no-SKIP evidence support those gates; their unprinted nested result
rows and numeric Cargo exits remain **UNKNOWN_NOT_PRINTED**. The two independently
printed helper rows in the workspace phase are separate evidence.

## Source disposition

[Wave28 independent review](2026-10-14-r2-raw-snapshot-second-corrections-review.md)
and [C02 format review](2026-10-15-r2-snapshot-format-review.md) remain immutable.
Their bounded source conclusions carry forward with the coordinator's exact
[C03 compilation repair](2026-10-15-r2-snapshot-compile-corrections.md),
[C04 lint repair](2026-10-15-r2-snapshot-lint-corrections.md) and
[C05 test-only repair](2026-10-15-r2-snapshot-test-lint-corrections.md).
C03/C04/C05 were directly dispositioned by the coordinator; they were not new
independent worker reviews. This candidate's compilation, strict lint, format and
named Linux tests close the identified executable verification gaps for S2.

| Frozen source | Lines | SHA256 |
| --- | ---: | --- |
| crates/market-data/src/daily_price_only/source.rs | 4448 | 33af9bbafba7b2b0a3c85007a1dfd00e9a17adbef09c8e37de885b6745939a98 |
| crates/market-data/tests/daily_price_only_snapshot.rs | 1972 | 7dc0eb24dd4cad1a2ae5568682f7c2b42ff5e05f4de815ae39f68e81dc1bec87 |
| crates/market-data/src/daily_price_only/mod.rs | 9 | 700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a |

The captured result remains opaque read-only evidence. It does not establish
provider truth, reconstruct copies or grant admission rights. Non-Linux and
32-bit execution, actual cross-principal UID/ACL/mount/access arrangements,
allocation/RSS and runtime installer/consumer lifetime policy remain unverified.
This does not claim universal adversarial-filesystem or continuous attestation.

Negative bootstrap/rollback, 0060, provider/copy reconstruction/admission,
I6 diagrams before structural main integration, deployment/runtime, Owner use,
three trading-day observation and final C_R2 remain open. No main merge, workflow
change, production action, provider request or permission expansion occurred.

## Retained receipts and earlier outcomes

Private files are 0600 under 0700 target/r2-daily-beta/ci/7cdf3a6. Raw logs were
kept in memory only; the receipts retain sanitized metadata, hashes and named rows.

- ci-evidence.json: 3,799 lines, SHA256
  `da010b43e68b20b2124032f11158506e02bdd9981455fc2462b84473089a4e49`.
- pg-evidence.json: 422 lines, SHA256
  `ffce0521e26081124d882d664970ab2cc45052a8ff657160088fb7fd2b78be32`.
- Frozen wave33-C05-ci-inputs.json: 301 lines/69 pins, SHA256
  `7be479d3c08665f44571be8f41ae61838a5e45aa01d82be8535ebe91ef3b9c0d`.
- Frozen wave33-C05-ci-test-inventory.json: 981 lines, SHA256
  `b6db33cc8cb014e45bf066b321dbe44bcf76442babb5e224f92683f4160dcea7`.
- Launch receipt SHA256
  `e1d76fa40263439302107f4ae1da974226f23abefc2f23873f8736015a4cefb8`.

The three wave33 files reside under target/r2-daily-beta/wp-4-integration. Their
pre-dispatch inventory states remain historical, while ci-evidence.json explicitly
records this candidate's observed outcomes. No prior receipt was rewritten.

The original 245f4d9 compile failure, C03 and C04 partial successes/strict-lint
failures, older c0ac599/929fcd6 outcomes and a888cd6/run38545976070 I0c-only partial
success remain preserved. This success does not retroactively upgrade them.
No concrete correction remains open within this bounded S2 package. Broader
R2 work and the evidence limits above are outstanding.
