# R2 I4d sealed-chain replay — exact candidate CI

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Candidate: `3073e9e64ca4f12b6a9e8b84398f1696ce99aef8`.
[CI 38776392579, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38776392579)
completed SUCCESS. Full evidence recovered at 2026-10-15T17:44:41.020346Z.
This accepts bounded source and hosted checks; full I4 and R2 remain open.

## Identity and recovery

All eight jobs and complete log hashes were recovered; seven execution jobs
checked out the exact candidate. All 507 frozen pins and 321 committed source
blobs matched. One dispatch followed two exact-head all-event zero-run queries and
remote SHA equality. No run was rerun, canceled or duplicated.

Private receipt: `target/r2-daily-beta/ci/3073e9e/ci-evidence.json`, 7,287 lines,
SHA256 `c65df16e29fddec5a8a87fbdaf8786dcdf97cbfc558b166f7c037a0cf0387917`.
Only sanitized named results, fixed metadata and hashes were persisted, not raw logs.

## Verified scope

- All 15 new definitions passed: four Linux metadata-reader controls and eleven
  pure navigation controls. They cover exact entries and safe metadata, resource
  preflight, canonical/hash/policy binding, parent depth/order and cancellation
  before the next read. Names are recorded individually in the receipt.
- Eight carried I4c, 12 I4b, 14 I4a, eight settlement-unit, 21 I3 and every other
  frozen Rust group passed. The six chart cases remain a separate phase. Group
  counts are not summed as a unique suite total.
- Dedicated PostgreSQL: all 16 outer commands exited zero without SKIP; 14 daily
  schedule, six daily publication-failure, six daily recovery and eight historical
  publication named cases passed. Daily32/subcases73/bootstrap83 matched exactly.
  Both lanes matched all 60 committed UP SHA384 checksums. Direct roles, 0039
  rollback guard, catalog checks, NO_OP rerun and owned-container cleanup passed.
  Unprinted nested helper names and inner exits remain UNKNOWN.
- Strict workspace Clippy, workspace formatting and policy passed. Protected
  artifact fixture hierarchy and bounded nonrecursive cleanup passed.
- Web supervisor34 and both receipts passed: 121 off-lane and 14 stream-lane
  cases, one successful attempt each, no skip/flaky/retry, exact source locations
  and successful cleanup/reservations. Shared owner-empty cases are not double
  counted as a unique total.
- Python: 487 passed / 10 existing skipped / seven warnings. Of 86 job steps,
  85 succeeded; only the success-inapplicable PG failure-log step was skipped.

## Acceptance and remaining work

[Source review](2026-10-16-r2-daily-replay-source.md) is direct coordinator review,
not independent review. Metadata navigation does not confer approval. The actual
factory still requires the fixed approved historical base, then each existing
full sealed verifier, before exposing the tip. No registry, authority or source
guards were weakened.

The tests do not prove a complete actual-approved-base chain, distinct-UID/ACL
admission, crash/fsync faults, installer rollback or successful daily publication.
Earlier failures, including the Wave53 web failure with UNKNOWN cause, remain
preserved. Integrated independent review, full runtime producer/admitter/worker
wiring, superseded-session cancellation, I5-I7, diagrams before main, operational
activation, actual Owner use, three consecutive trading days and final C_R2 remain.

Next: mixed historical/daily consumer with retained sealed lifetimes and a direct
worker DB preflight. No additional worker or local project execution. Ralph continues.
