# R2 snapshot C03 hosted CI disposition

Candidate `d00b4759ba9e08887d3871f64e8f53bfb1f02fa7`,
[run 38659690888](https://github.com/sihioov/Lagrange/actions/runs/38659690888),
attempt 1, completed with failure. The coordinator recovered all eight jobs,
84 steps and eight completed logs once each. Seven actual checkout log SHAs
match the candidate; all 56 input pins match, including candidate Git blobs.
Raw logs stayed in memory; no local executable checks or new delegation.

## Verified outcomes

| Job | Outcome | Scope |
| --- | --- | --- |
| workspace-tests | PASS | All 99 expected Linux Rust names have one direct ok row in the workspace phase. All six separate chart cases have direct ok rows in their explicit phase; their six workspace repeats are separate rows. QA cleanup succeeded. |
| postgres-integration-validation | PASS | APPROVED, 12 completed DB labels including both daily-admitter helpers, 59 migration checksums matched to candidate bytes, equal second pass/NO_OP, 0039 guard, catalog audits, seven role logins in each lane and PASS_NO_OWNED_CONTAINERS. |
| format | PASS | Hosted cargo fmt --all -- --check. |
| web | PASS | OpenAPI, lint, typecheck, tests, build and E2E steps succeeded. |
| policy | PASS | All wrapper steps and 10 directly printed CI contract unit names succeeded. |
| python-tests | PASS | 485 passed, 10 skipped, seven warnings; backup-policy step succeeded. |
| clippy | FAIL | Strict --workspace --all-targets --all-features --locked -- -D warnings, exit 101. Three lint diagnostics at two source locations. |
| required | FAIL | Aggregate rejected the Clippy failure. |

The 99 expected Rust names comprise 30 wire, 18 S1 Raw, one collector, two
helpers, 19 S2 private and 29 S2 integration cases. The one S2 non-Linux and one
S1 non-Unix case remain excluded on Linux. C03's two E0599 sites and E0425 site
are resolved by the source correction and hosted compilation/test evidence.
Overall strict-lint/source acceptance remains withheld.

## C04 bounded findings

- source.rs:1416:16: `manual_contains`; use the equivalent dates.contains call.
- source.rs:2264:21: `collapsible_match` and `implicit_saturating_sub`; use
  depth.saturating_sub(1), preserving zero depth and positive-depth decrement.

No lint suppression, guard weakening or new dependency is needed. The source
correction is recorded separately in the C04 report after this run's full recovery.

## Evidence limits and preservation

PG's completed labels and source wrapper establish the configured DATABASE_URL
and rejection of SKIP markers. Its nested direct rows and numeric Cargo exits
were not printed and remain UNKNOWN. Quiet Python readiness/SIGTERM individual
nodes and other unprinted policy subcases remain UNKNOWN. Aggregate Rust row
sums are not unique test counts. Runtime/platform/ACL and all later R2 gates
remain open. Previous 245f4d9 failure and a888cd6/c0ac599/929fcd6 partial evidence
remain separate and unchanged.

Private evidence: target/r2-daily-beta/ci/d00b475/ci-evidence.json, 5,852 lines,
SHA256 `8af7bb6b05c7b03b4791755195d59a24ba7399320055682323d81b86b52ba343`.
PG checksum receipt SHA256
`ee694e73e36ddd632abb21100c4d34466d74c70456b5df9e444c54dcad838e54`.
Files are 0600 in a 0700 directory. A sanitized receipt write initially exceeded
the shell argument limit before process creation; bounded chunks completed the
write. No raw log was written. No rerun, cancellation or duplicate dispatch.
