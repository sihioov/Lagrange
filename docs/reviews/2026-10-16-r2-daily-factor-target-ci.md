# Wave50 — complete I3 candidate CI failure recovery

Roadmap R2-04/09 remains DOING. Exact candidate
`1c82592321609dc2a46e0f84f63a01ad89a70f42`,
[CI 38755069474 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38755069474),
CI/ci.yml, workflow_dispatch, excited-dolphin, completed **FAILURE**.
Recovery finished 2026-10-15 15:11:11 UTC (2026-10-16 KST). All eight terminal
jobs and their log digests, seven actual checkout SHAs, 399 input pins and 296
committed candidate blobs were recovered. No rerun, cancellation or duplicate.

Clippy (workspace/all-targets/all-features/locked/-D warnings), workspace format,
web, Python, policy and PostgreSQL passed. Workspace tests and required failed.
There were 82 successful, two failed and two skipped steps; the skipped PG failure
log printer was not a skipped validation job. Raw log bodies were not persisted.

## Named evidence

| Group | Result |
| --- | --- |
| New I3 definitions | 19 PASS / 1 FAIL |
| Historical computation | 29 PASS |
| S5d/e/f | 36 PASS / 1 FAIL |
| S5c / S5b / S5a | 26 / 21 / 33 PASS |
| S4 / S3 / normalizer | 32 / 22 / 37 PASS |
| Daily SQL / affected legacy / prior Rust | 32 / 28 / 99 PASS |
| Paper clock / transport | 2 / 3 PASS |
| Chart acceptance, separate invocation | 6 PASS |
| Dedicated PostgreSQL | 32 direct daily rows, 73 subcases, 83 bootstrap cases PASS |
| DB wrapper invocations | 13 exit 0, no SKIP; eight actual owner-beta publication/recovery named rows PASS |

Groups are not summed as distinct cases. Both PostgreSQL lanes matched all 60 UP
checksums; NO_OP, direct roles, 0039/catalog and cleanup passed. Unprinted nested
helper rows and inner exit codes remain UNKNOWN. The protected fixture hierarchy
was root 0:0/0755, isolated anchor 0:0/0755, runner child 1001:1001/0700; setup and
nonrecursive cleanup passed. Platform exclusions remain excluded.

## Four failures and bounded disposition

1. `owner_beta_daily_compute::daily_relative_nulls_are_explicit_all_cash_without_forward_fill`
   failed at line 335 with fieldless `FactorSnapshotInvalid`. The unchanged
   relative-momentum equation rejects an empty ranked set; the new expectation of
   an all-cash result was incorrect. Correct the test to assert rejection with and
   without earlier non-null rows and add a current-row positive control. Keep a
   separate valid trend all-cash case. Do not change the historical equation.
2. `daily_price_only::continuation::tests::s5d_readback_preserves_entire_daily_prefix_and_new_grid`
   failed at line 796. Both vectors had 33 rows; the first differing field was
   symbol. The encoder canonically sorts instrument/date, whereas the expected
   fixture used universe declaration order. Sort the expected sequence explicitly;
   retain exact equality of every row and the production prefix guards.
3. `service_status_stream_requires_exact_proof_and_actual_sql_timing` failed with
   the fixed message `provider event just inside 30s was not live`. Its result was
   printed at 2026-10-15T15:00:27.3148347Z (00:00:27 KST). The fixture subtracts
   29.5 seconds from actual DB time, placing this quote before the current day's
   open; migration 0058 correctly rejects it even though its age is under 30s.
4. `service_status_stream_observation_and_positive_assurance_are_distinct` failed
   with `actual G1 inclusive coverage/subset/denominator differed`; its result
   was printed at 15:00:24.8777090Z (00:00:24 KST). Its partial-coverage fixture
   similarly expects a 28-second-old quote to be live. The emitted generic error
   does not identify the internal assertion; this concrete fixture defect is
   consistent with the timestamp, not a recovered per-assertion DB trace.

The service tests need explicit session-open outcomes for these real-clock cases,
without replacing the production clock, broadening the session window, weakening
TTL/demand/proof assertions, sleeping or serializing the suite. Scope includes the
shared test support module as well as the test target; production SQL stays intact.

## Receipt and remaining work

Private `target/r2-daily-beta/ci/1c82592/ci-evidence.json`: 4262 lines,
SHA256 `80fac796f6ff09c6548509bac62298505a7744b3af33909439027c36b9e2a236`.
Workspace log SHA256
`65752d8f00967b9010c8a68581a69d8d0642ed36ab735a38a84e1e0658c7eae8`;
the bounded timestamp extraction matched this digest. Sanitized failures are in
`failure-diagnostics.json`; no raw logs or provider/credential data were saved.

Full recovery releases the candidate freeze for the bounded Wave51 test correction.
No source acceptance follows from this failed run. I4 drafts remain unregistered
and outside that correction candidate. Actual approved-v3 whole flow, distinct UID/
ACL/crash/installer rollback, I4–I7, integrated independent review, diagrams before
main, G4 operational delta, runtime, Owner use, three trading days and final C_R2
remain open. The Ralph goal continues.
