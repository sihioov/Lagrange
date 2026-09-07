# KIS daily stale-release incident — 2026-09-07

Status: cause identified; existing source fix verified; obsolete timer disabled;
replacement timer and data replay still pending.

## Operator follow-up — 2026-09-07 21:28 KST

The operator reported running the scoped disable command. Coordinator read-only
verification confirmed `lagrange-kis-daily-66b2a8c.timer` is inactive/disabled with
no next trigger. The associated service retains its old failed status; no job is
running. No failure history was cleared. Current remains release 69ad55d.

This stops the obsolete activation, but **daily collection is now disabled**.
No replacement timer has been installed, started, or scheduled by the coordinator.
The official installer refuses apply at/after 16:30 KST. The next eligible window
is 2026-09-08 before 16:30 KST; revalidate current release and protected settings
then. Missing-session replay and actual publication checks remain outstanding.

## Evidence and cause

- The then-enabled `lagrange-kis-daily-66b2a8c.timer` last triggered at 2026-09-07
  16:30:07 KST. Its service failed at 16:30:09 with exit 1.
- ExecStart uses immutable release `66b2a8c61dfd1e8b5f79d6b1baa9f080b0a9e268`;
  its environment pins the same commit but reads `/etc/lagrange/compose.env.pending`.
  The running release is `69ad55db478e16306c22bbe41affccc2907f8742`.
- Release 66b's research-worker Compose entry has `build:` but no pinned `image:`.
  It does not identify the same image as the long-lived research container. The
  local default research-worker image has revision `6c2d18b5a1e05786205750d39ed3bae6d11b5380`.
  The removed one-shot container's exact image ID was not captured, so the
  default image is a candidate, not an independently attested executed image.
- Daily missing sessions were 2026-08-25, 09-01, 09-02, 09-03, 09-04, 09-07.
  The first failed with `KIS_NORMALIZE_MISSING_RESPONSE`, publication/permanent.
- Live Raw manifest metadata contains 11 ordinary EOD batches and two historical
  corporate-action-only batches in the same `kis/kr` scope: 2020-01-31 (7 files)
  and 2016-08-29 (77 files). Neither contains bars/reference/calendar.
  No Aug. 25 wire batch or directory exists in the inspected live Raw root.
- Release 66b's `recover_kis_normalization` blindly normalizes the entire scope,
  including these evidence-only batches. `normalize_bars` then raises MissingKind.
  Recovery precedes new-date ingestion. Aug. 25 is the pending target, not the
  date of the offending historical evidence. Do not delete these valid batches.
- Commit `db7de37` already excludes nonempty all-CorporateActions batches from EOD
  recovery while retaining failure for mixed/incomplete EOD bundles. This fix is
  present in current 69ad and the pending Stock Beta release 14f5bd5.

Code evidence: `data-pipelines/collectors/src/pipeline.rs:439`,
`crates/market-data/src/normalize.rs:1346`,
`data-pipelines/collectors/tests/kis_recovery.rs:321`.

## Diagnostic correction

`scripts/ops/backfill-production.sh` pipes worker output through
`scripts/ops/lib/backfill-progress.py:59`. That relay intentionally retains only
code/date/phase/class, dropping batch ID and response-kind/detail fields.
Their absence in **scheduled service output** is not evidence of a stale binary.
The separate long-lived container's no-detail curation failure remains unclassified;
do not claim that repairing this timer has already resolved it.

## Verification performed

Provider-free existing regression and full recovery suite on the current worktree:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --offline -p collectors --test kis_recovery
```

Result: exit 0, 9 passed, 0 failed/ignored. No source changes were necessary for
this existing fix. No live collection, data deletion, state rewrite, or worker
restart was performed for this incident.

## Runtime repair still required

1. Completed: the obsolete `lagrange-kis-daily-66b2a8c.timer` was disabled,
   preserving its unit files and all state/evidence. No daily job was running
   at the operator follow-up check. Recheck runtime state before replacement.
2. Select the actually installed current immutable release containing the fix;
   verify its manifest/image binding and use that release's `deploy/compose/.env`,
   not mutable pending configuration. Do not treat an OCI label alone as runtime QA.
3. Run official daily installer preflight/check. Installation and activation must
   follow its before-16:30 KST window; do not bypass the clock guard or enable a
   persistent catch-up timer unexpectedly after that cutoff.
4. Before an authorized provider-backed replay, coordinate the existing research
   daemon and token manager, inspect the permanent-failure state, and determine the
   exact missing-session/request budget. Preserve state; do not mark dates PUBLISHED
   manually or delete failures to make automatic resume pass.
5. Run the reviewed missing-session replay using the corrected pinned image; verify
   actual published dates and ETF11 coverage. The other curation failure may still
   require diagnosis. Success is new verified publication, not a green timer alone.

Initially the coordinator attempted `sudo -n /usr/bin/systemctl disable --now
lagrange-kis-daily-66b2a8c.timer`; OS authentication was required, so that attempt
changed no unit. The subsequent operator disable is verified above.
Existing 72-hour grants cover preflight/image builds only, not timer replacement or
provider execution. Administrator authentication is required for this runtime repair.
