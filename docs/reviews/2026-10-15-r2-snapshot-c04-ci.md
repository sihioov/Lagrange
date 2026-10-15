# R2 snapshot C04 hosted CI disposition

Candidate `d883c17c4217a29681a649fd29641172f53b6355`,
[run 38662224666](https://github.com/sihioov/Lagrange/actions/runs/38662224666),
attempt 1, completed with failure at 2026-10-15 01:04:57 UTC. The coordinator
recovered all eight jobs, 84 steps and eight logs once each. Seven actual checkout
SHAs match the candidate; all 62 input pins and their tracked candidate blobs
match. Raw logs stayed in memory. No local executable checks or new delegation.

| Job | Outcome | Evidence scope |
| --- | --- | --- |
| workspace-tests | PASS | 99 expected Linux Rust names each have one direct ok row in the workspace phase; six chart names pass in the separate chart phase, with six workspace repeats kept separate. QA cleanup succeeded. |
| postgres-integration-validation | PASS | APPROVED; 12 completed DB labels including both daily-admitter helpers, 59 candidate-matched migration checksums, equal second pass/NO_OP, 0039 guard, catalog and role checks, PASS_NO_OWNED_CONTAINERS. |
| format | PASS | Hosted cargo fmt --all -- --check. |
| web | PASS | OpenAPI, lint, typecheck, tests, build and E2E steps succeeded. |
| policy | PASS | All wrapper steps and 10 directly printed CI contract unit names succeeded. |
| python-tests | PASS | 485 passed, 10 skipped, seven warnings; backup-policy step succeeded. |
| clippy | FAIL | Strict --workspace --all-targets --all-features --locked -- -D warnings, exit 101; two test-code lint findings. |
| required | FAIL | Aggregate rejected the Clippy failure. |

The 99 names comprise 30 wire, 18 S1 Raw, one collector, two helpers, 19 S2
private and 29 S2 integration cases. One S2 non-Linux and one S1 non-Unix case
remain excluded on Linux. C04's three production lint conditions are resolved
by this run. Overall strict-lint/source acceptance remains withheld.

## C05 bounded findings

- `daily_price_only_snapshot.rs:1326:13`: needless_borrows_for_generic_args on
  the temporary metadata_path result in the hard-link test.
- `source.rs:2728:33`: type_complexity on the cfg(test) phase-hook storage type.

The bounded follow-up removes the one-use temporary borrow and names the exact
private callback type. It must preserve the stored-PathBuf borrows fixed by C01,
callback lifetime/ownership, hook reset and all assertion semantics. No lint
suppression, guard weakening or production API change is needed.

## Evidence limits

PG labels and the pinned wrapper establish its configured DATABASE_URL and
no-SKIP gates. Unprinted nested direct test rows and numeric Cargo exits remain
UNKNOWN. Quiet Python readiness/SIGTERM individual nodes and unprinted policy
subcases remain UNKNOWN. Aggregate row counts are not unique test evidence.
Broader platform/ACL/runtime, negative bootstrap/rollback, 0060, provider/copy
reconstruction/admission, I6 diagrams before main, Owner and final C_R2 remain
open. All previous failure and partial-success records remain immutable.

Private receipt: target/r2-daily-beta/ci/d883c17/ci-evidence.json, 3,726 lines,
SHA256 `9fd8e99a499e35d9cc4c4c0f34476658dff502d77f8ef61519f4965d2c1eea47`.
PG receipt SHA256
`894151ad56a584cb4cb5a6f330bb80649752b4e19e87272c5be1d2a77ae95c10`.
Files are 0600 in a 0700 directory. The earlier C03 receipt retains initial
inventory-state fields alongside observed rows; its immutable named-outcomes.json
successor (`0addcc098ae8255c1474b31089795878484e3706e4351e906ad0031c04633727`)
explicitly records 99 Rust and six chart PASS outcomes. Neither original changed.
No run was rerun, cancelled or duplicated.
