# R2 I4e mixed execution — failed candidate CI recovery

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Candidate: `93437069c7a5bfdf6a1f8d547eb1adb87530f472`.
[CI 38780494397, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38780494397)
completed FAILURE. Full evidence recovered at 2026-10-15T18:10:15.510174Z.
The candidate is not accepted. No rerun, cancellation or duplicate dispatch occurred.

## Identity and concrete failures

All eight terminal jobs and complete log hashes were recovered. Seven execution
jobs checked out the exact candidate; 527 frozen pins and 325 committed blobs
matched. The pre-push exact-head all-event query returned zero at 17:48:47.139843Z;
remote equality and a second zero query at 17:49:13.132163Z preceded one dispatch.

- Strict Clippy rejected unused `next_schedule` and `job_payloads` methods in the
  shared daily fixture when compiled into the new runner integration-test binary.
  `-D warnings` promoted `dead_code` to an error. No production error was reported
  by this diagnostic.
- Workspace tests had one named failure:
  `status_repository_is_fenced_best_effort_and_publication_bound`, in the collector
  service-status test fixture. Its panic at candidate line 870 reported that DB
  roles could not be bootstrapped. The old wrapper discarded the SQLSTATE and
  underlying error; the exact cause remains UNKNOWN. This is not evidence that
  the recommendation calculation or new daily runner failed.
- The required aggregate failed. Format, web, policy, Python and PostgreSQL jobs
  passed. Of 86 job steps, 81 succeeded, three failed and two were skipped.

Private receipt: `target/r2-daily-beta/ci/9343706/ci-evidence.json`, 7,518 lines,
SHA256 `8861013bea51d47594fe0246bd151c2d4b987cba53d3835e2a5c1562a3434dc4`.
The same directory preserves `clippy-diagnostic-exact.json` and
`workspace-diagnostic-supplement.json`. Their additional extraction matched the
original complete log hashes. Raw logs were held in memory only.

## Verified checks and limits

- Both new pure definitions and all ten daily runner DB definitions passed.
  The dedicated actual-role lane printed all ten names, exited zero and had no
  SKIP. Five historical runner tests and every frozen carried Rust group passed.
  Six chart cases are a separate phase; overlapping group counts are not summed.
- All 17 PostgreSQL outer commands exited zero without SKIP. The 14 schedule,
  six daily publication, six daily recovery, eight historical publication and
  ten daily runner names passed. Daily32, subcases73 and bootstrap83 matched.
  Both lanes matched all 60 committed UP SHA384 checksums. Direct roles, 0039,
  catalog, NO_OP and owned-container cleanup passed. Unprinted nested helper
  names and inner exits remain UNKNOWN.
- Web supervisor34 and individual receipts for 121 off-lane and 14 stream-lane
  cases passed: exact locations, one attempt each, no skip/flaky/retry, cleanup
  and reservations passed. Shared cases are not counted twice as unique tests.
- Python reported 487 passed, ten existing skipped and seven warnings.

[Source review](2026-10-16-r2-daily-runner-source.md) is direct coordinator review,
not independent review. These tests do not prove an actual approved-base whole
flow or successful production publication. The two failures require a bounded
new candidate; prior successes and all earlier failures remain preserved.

Next: [test fixture corrections](2026-10-16-r2-daily-runner-lint-correction.md),
then one exact-candidate hosted verification. Superseded-session cancellation,
producer/admitter wiring, full I4, I5-I7, integrated independent review, diagrams
before main, operations, actual Owner use, three trading days and final C_R2 remain.
