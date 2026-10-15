# R2 I4e runner and test-fixture correction — bounded CI acceptance

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Candidate c4f35bb7b81a3e8fb3f6c9297315daade24733f1.
[CI38784769462 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38784769462)
completed SUCCESS. Full evidence recovered at 2026-10-15T18:43:36.629885+00:00.
This accepts the bounded mixed-runner and two-file test correction only.

## Exact candidate and evidence

Eight terminal jobs/full log hashes, seven actual checkout SHAs,548 frozen pins
and328 committed blobs matched. All-event exact-head queries returned zero
before push at18:19:54.391672Z and after remote equality at18:20:13.541787Z.
Exactly one dispatch created this run at18:20:15Z. No rerun/cancel/duplicate.

Private receipt: target/r2-daily-beta/ci/c4f35bb/ci-evidence.json,
7484lines, SHA256 8fae21d9c8ca6d614569019e18ebba4ee47f45d4bd5628ba4ff5ee2a48dbc4e2.
Raw logs were held in memory only. Per-job sanitized receipts preserve named
rows, phase, log line, fixed diagnostics and complete-body hash.

- Workspace format and all-target/all-feature strict Clippy with -D warnings
  passed. Both I4e pure definitions and ten daily runner names passed; all ten
  also passed the dedicated actual-role DB command with exit0/noSKIP.
  Five historical runner names and every frozen carried Rust group passed.
- All three collector service-status actual-DB tests passed. These fixtures
  require DATABASE_URL and have no missing-DB SKIP fallback. This verifies
  the corrected candidate; the old masked bootstrap SQLSTATE/cause remains
  UNKNOWN and is not retrospectively inferred from the success.
- All17 PostgreSQL outer commands exited0/noSKIP. Schedule14/dailyPublish6/
  dailyRecovery6/historicalPublish8/dailyRunner10 exact names passed.
  Daily32/subcases73/bootstrap83 matched. Both full-release runs matched60
  committed UP SHA384 checksums; direct roles,0039,catalog,NO_OP and owned
  container cleanup passed. Unprinted nested helper names/exits remain UNKNOWN.
- Web supervisor34 and individual receipts for121 off-lane and14 stream-lane
  cases passed with exact locations, one attempt each, no skip/flaky/retry,
  cleanup and reservations. Chart6 is separate. Overlapping groups and
  shared Web identities are not summed as unique tests.
- Python reported487passed/10existing skipped/7warnings. Exact per-step
  outcomes remain in the receipt.

[Correction](2026-10-16-r2-daily-runner-lint-correction.md) and
[I4e source scope](2026-10-16-r2-daily-runner-source.md) received direct
coordinator review, not independent review. The
[9343706 failure](2026-10-16-r2-daily-runner-ci.md), all older failures and prior
bounded successes remain unchanged.

## Limits and next dependency

Synthetic actual-role tests do not prove the full approved historical-base,
Raw-to-candidate-to-independent-admission-to-computation happy path. Actual
UID/ACL separation, crash durability, installer rollback, whole I4/I5-I7,
integrated independent review, diagrams before main, operations, Owner use,
consecutive three trading days and final C_R2 remain open.
Next is additive0061 superseded-session cancellation with existing guards
intact, then producer/admitter wiring. Ralph remains active.
