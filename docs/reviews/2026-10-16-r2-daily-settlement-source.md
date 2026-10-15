# R2 I4b daily failure settlement and recovery

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09, WP-4/I4.
Status: direct coordinator source review; compiler and tests unexecuted for this
candidate. This package also carries the three test-only I4a logging corrections
identified in [Wave52 CI](2026-10-16-r2-daily-cycle-ci.md).

## Bounded source change

[publish.rs](../../crates/job-queue/src/owner_beta/publish.rs) adds a separate
daily failure entry point. It validates the exact typed daily input against the
claim, uses existing fenced queue/attempt settlement, then checks the locked
job identity and full pending-run binding: Owner, config, date, canonical
strategy, all four daily pins, trigger/version/epochs, absent historical pins
and absent result fields/items. A mismatch rolls the transaction back.

A retry leaves the entire PENDING run and permanent reservation unchanged.
FAILED/CANCELED mirrors only the terminal result into the existing run; no
replacement reservation/job is created and no lineage is rewritten. Current
control/entitlement revocation cannot prevent an exact live claim from releasing
capacity on failure. It still blocks successful publication through the separate
SQL authority. Stale claims are left for recovery.

Generic canceled-attempt settlement retains its lease. The daily entry point
clears that lease under the existing job/attempt fence before committing the
terminal run. This is required by 0060's daily consistency matrix; queue.rs and
historical settlement are unchanged. Commit uncertainty is a distinct error and
does not authorize an inline retry.

[recovery.rs](../../crates/job-queue/src/owner_beta/recovery.rs) retains existing
claim selection, retry/backoff, exhaustion and result mirrors. Daily rows add
strict schema dispatch and full strategy/provenance/epoch binding. A pending
daily run also requires absent start/finish fields. Reservation identity, date,
epochs and admission equality remain checked by the protected 0060 deferred
consistency trigger on every changed job/attempt/run at commit; worker gains no
direct reservation SELECT privilege.

An unclaimed daily cancellation already has a null lease. Recovery skips the
otherwise forbidden timestamp-only update of that immutable canceled job,
while still mirroring the pending run. Claimed cancellations release their lease
normally. The historical cancellation branch retains its previous behavior.

## Written regressions and evidence requirements

Six [settlement tests](../../crates/job-queue/tests/owner_beta_daily_publish.rs)
cover byte-identical retry rows, terminal failure with retained reservation,
actual expired-claim rejection, cleanup after control disable, mismatched input,
and an audited cancellation winning a failure settlement without outputs.

Six [recovery tests](../../crates/job-queue/tests/owner_beta_daily_recovery.rs)
cover a live claim, same-job requeue with unchanged sealed rows, three-attempt
exhaustion, two racing recoverers, unclaimed audited cancellation and expired
claimed cancellation. Generic sweeping must continue excluding this job type.
Lease tests set the isolated fixture's control lease to one second through its
real epoch guard, then observe actual DB-clock expiry under a five-second bound.
They do not bypass triggers or backdate a live job. This also checks failure
cleanup after an activation-epoch change; it is not production lease tuning.

All twelve definitions are WRITTEN_UNEXECUTED. Dedicated PG adds two exact named
targets, each requiring six unique PASS rows, exit zero and no SKIP. All fourteen
I4a and eight historical publication rows remain independently required, with
the previous DB targets retained. Synthetic SQL admissions test persistence only;
they are not filesystem approval, market-data evidence or runtime activation.

## Limits

Source review is direct coordinator work, not independent review. Only explicit-
file edition-2024 rustfmt and static source/diff checks ran locally. No local
compiler, test, SQL, project checker/helper, Docker or provider execution occurred.

Daily success publication with freshly reopened approved inputs, superseded-session
cancellation, complete worker dispatch, producer/admitter orchestration and lost-
reply readback remain separate work. Actual UID/ACL/crash/installer evidence,
integrated independent review, I5-I7, I6 diagrams before main, concrete G4 supply
changes, operational activation, Owner use, three trading days and final C_R2
remain open. No broader acceptance is inferred from this package.
