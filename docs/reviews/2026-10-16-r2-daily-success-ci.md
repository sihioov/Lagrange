# R2 I4c daily success boundary — exact candidate CI

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Candidate: `47c7dbca1d1898916c7cc734374254623fd23594`.
[CI 38771904166, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38771904166)
completed SUCCESS. Full evidence was recovered at 2026-10-15T17:13:09.786418Z.
This accepts the bounded source and hosted checks below, not complete I4 or R2.

## Exact execution and evidence

All eight jobs and eight complete log hashes were recovered. The seven execution
jobs checked out the exact candidate. All 488 frozen pins and 318 committed source
blobs matched. The single dispatch followed exact-head all-event queries returning
zero and a matching remote SHA; no run was rerun, canceled or duplicated.

The private receipt is `target/r2-daily-beta/ci/47c7dbc/ci-evidence.json`, 7,195 lines,
SHA256 `58738b0b249af386af29e54e42ac5a80ad40b8057bddb4432cefee4b476a31a7`.
Raw logs were not persisted. The receipt records named rows, safe diagnostics,
checkout identities, per-job hashes and the exact test inventory correspondence.

## Verified scope

- All eight new daily success validation tests passed. The carried 12 daily
  settlement/recovery, 14 schedule, eight settlement-unit and 21 daily computation
  definitions passed, together with every required earlier Rust group. Chart six
  remained a separate execution. Group counts are not a unique suite total.
- Dedicated PostgreSQL verification reported all 16 outer target exits as zero,
  without SKIP. Exact named results included 14 daily schedule, six daily failure
  publication, six daily recovery and eight historical publication tests. Daily
  migration 32 cases / 73 subcases, 83 bootstrap cases, 60 committed UP migration
  SHA384 checksums in both lanes, NO_OP rerun, direct roles, 0039 rollback guard,
  catalog checks and owned-container cleanup passed. Unprinted nested helper
  rows and inner exits remain UNKNOWN.
- Strict workspace Clippy and workspace formatting passed. The protected artifact
  fixture had root/anchor mode 0755 and runner child mode 0700; setup and bounded
  nonrecursive cleanup passed.
- All 34 web supervisor tests passed. The new sanitized receipts independently
  retained all 121 off-lane and 14 market-stream cases, each with one passing
  attempt, no skip/flaky/retry, valid committed file/line locations and successful
  cleanup/reservation checks. The intentional owner-empty overlap is not counted
  twice as a unique case. OpenAPI, lint, typecheck, unit, build and E2E steps passed.
- Python reported 487 passed / 10 existing skipped / seven warnings. Policy passed.
  Of 86 job steps, 85 succeeded; only the PostgreSQL failure-log step was skipped
  because the job succeeded. This is not a skipped test target.

## Acceptance and limits

The [source review](2026-10-16-r2-daily-success-source.md) is direct coordinator
review, not independent review. The success API requires opaque admitted input and
opaque computation; pure validation tests do not prove successful publication
with the actual approved historical base. The unchanged historical publisher and
earlier source/DB constraints remain required.

The [Wave53 web failure](2026-10-16-r2-daily-settlement-ci.md) remains preserved with
UNKNOWN cause. This later success and better diagnostics do not establish its cause.

Next is bounded reopening of the exact sealed daily chain, then full runtime
orchestration and superseded-session cancellation. Actual approved-base integration,
distinct UID/ACL/crash/installer rollback, integrated independent review, I5-I7,
diagrams before main, operational activation, actual Owner use, three consecutive
trading days and final C_R2 remain open. Ralph continues without new delegation or
local project execution.
