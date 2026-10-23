# Wave51 — bounded correction after full Wave50 recovery

R2-04/09, baseline `1c82592321609dc2a46e0f84f63a01ad89a70f42`.
The [four failures](2026-10-16-r2-daily-factor-target-ci.md) remain preserved.
Direct coordinator disposition; no independent-review or executable acceptance.

Only four test files/regions change:

- [Daily computation test](../../crates/job-queue/tests/owner_beta_daily_compute.rs:322):
  rename the erroneous all-cash expectation to
  `daily_relative_nulls_fail_closed_without_forward_fill`. Assert the exact
  `FactorSnapshotInvalid` with all current values NULL, both before and after
  adding earlier non-null values, and through the historical builder. One valid
  current identity must then receive all weight while the other ten stay excluded,
  despite their larger historical values. A separate trend-equal-averages case
  checks valid explicit all-cash output against historical items and reasons.
  The old source report's “all-cash null handling” description was incorrect;
  it is preserved as historical evidence and superseded here. No strategy change.
- [Continuation test](../../crates/market-data/src/daily_price_only/continuation.rs:791):
  expected rows are sorted by instrument/date after encoding. Exact equality of
  all decoded values is retained. The entire production prefix is byte-identical.
- [SQL timing test support](../../crates/api-server/tests/service_status_reads_support/mod.rs:1176):
  keep the actual DB clock, exact proof checks, denominator and <250ms observation
  margin. If the seeded receipt/provider timestamp predates this session's open,
  require zero live rows. Otherwise retain every original expected outcome at
  9.5/10.5-second heartbeat, 29.5/30.5-second quote and 5.5/4.5-second lease bounds.
  No waiting, fabricated clock, new session window or TTL change.
- [Partial coverage test](../../crates/api-server/tests/service_status_reads.rs:1289):
  when the 28-second-old provider timestamp precedes the real KST day's open,
  require demand/live/halted = 2/0/0 and the producer readiness heartbeat deadline.
  Otherwise preserve 2/1/1 and the provider's original two-second remaining deadline.
  Both paths retain stale raw-minimum and stored-clock checks. This tests session
  exclusion rather than treating a prior-day quote as live merely because it is young.

The shared support file also serves `service_status.rs`; all 33 definitions there
and all five `service_status_reads` definitions remain required, individually named.
There are now 21 I3 definitions: 19 unchanged names, one explicit rename, one new
trend all-cash case. All 37 S5def names remain. All inherited required groups and
the eight dedicated-DB publication/recovery cases remain required.

Static checks: explicit-file edition-2024 rustfmt and whitespace pass. Of the 399
Wave50 pins, 397 remain byte-identical; only the daily computation test and the
cfg(test) continuation region change among those pins. The two API test files are
additional bounded scope. All non-test production bytes, migration 0058/0060,
historical arithmetic, workflows and DB harness remain unchanged. No local compiler,
tests, DB, project helper, provider or production operation ran. I4 draft files are
unregistered, uncommitted and excluded from the candidate.

Each real-clock test executes whichever side of the day boundary actually applies;
a green later run alone does not prove a separately executed midnight branch.
The original failed-run timestamps support the diagnosis, not a full per-assertion
DB trace. Exact midnight-crossing whole-suite execution, approved-v3 whole flow,
role/ACL/crash/installer evidence, I4–I7, integrated independent review, operations,
Owner use, three trading days and final C_R2 remain separate.

Freeze the successor inventory and inputs, commit only this scope and tracking docs,
query all-event runs for that exact head, verify the pushed SHA and dispatch once.
Do not repeat any previous run. Full recovery determines bounded source acceptance.
