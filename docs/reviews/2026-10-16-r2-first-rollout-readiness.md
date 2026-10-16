# R2 first daily rollout: remaining execution dependencies

Roadmap R2-04/R2-09; coordinator source inspection on 2026-10-16.
Source candidate `70401cf421c90105f7c5d54703a0ec68c3fa25aa` passed
[Wave81 verification](2026-10-16-r2-backup-migration-mode-ci.md). It is not yet
the final `C_R2`. This document prepares the remaining work; no operation below
has been executed or approved by writing it.

The earlier [operations draft](../runbooks/r2-daily-beta-operations.md) remains
dated evidence from before U2 adoption. U2 was subsequently adopted and its
daily path implemented. Its old “awaits adoption/no producer” statements must
not be used as current blockers. Existing read-only KIS and Owner V2 approvals
remain in force. The historical sudo-expiry timestamp is not current root-access
evidence; Ralph or hosted success does not renew it. This does not revoke the
existing read-only/EOD approvals or turn them into new approval requests.

## What the current hosted evidence can establish

The seven native backup cases now have individual PASS rows. They exercise a
real disposable database, encrypted capture, actual networkless restoration,
corruption/failure cleanup and audit comparison. The source-side assertion
checks all62 migration checksums and seven real concurrent-index catalog rows.
The restored database is checked by full `pg_restore --exit-on-error` and the
daily audit; the test does not independently compare every restored catalog or
migration checksum. These are synthetic fixtures, not the operational database
or an approved-base producer-to-recommendation demonstration.

The actual runtime-isolation fixture deliberately uses a shell-process image.
The installer fixture and actual-role SQL cases likewise do not execute the
complete Rust daily flow. Sources:
[native backup test](../../scripts/ops/owner-beta-daily-backup-docker-self-test.py#L248),
[restore entrypoint](../../deploy/db/daily-backup-verify.sh#L37),
[runtime fixture boundary](../../scripts/ops/owner-beta-daily-docker-self-test.py#L2).

## First deployment has two distinct backup states

| State | Available path | Required disposition |
| --- | --- | --- |
| Previously observed production schema59, before daily installation | V1 backup captures PostgreSQL/Raw/Curated. Its verification restores with `--no-owner --no-privileges` and only checks three restored table names. | Preserve existing V1 sets. They alone do not satisfy the pre-upgrade consistent recovery gate. Refresh the actual schema/roles/state before selecting this branch. |
| Installed daily schema62, protected config/state and reader floor present | V2 daily backup retains ownership/ACLs, six file classes and disabled daily audit. | Use only after those prerequisites exist. A successful schema62 fixture cannot prove that this command works before the first upgrade. |

Concrete source:
[V1 restore](../../scripts/ops/run-production-backup.sh#L249),
[V1 capture](../../scripts/ops/run-production-backup.sh#L331),
[V2 required file roots](../../scripts/ops/lib/owner-beta-daily-backup.py#L125),
[V2 full dump](../../scripts/ops/lib/owner-beta-daily-backup.py#L313),
[daily-table audit](../../deploy/db/owner-beta-daily-backup-audit.sql#L19).

Before any upgrade, the required bounded preparation is a maintenance recovery
procedure for the actual pre-upgrade schema. It must capture complete database
state and all referenced Raw/Curated/approved-base files while every relevant
writer is stopped or fenced, and verify them in a disposable isolated restore.
The protected configuration and secret-recovery references are separate inputs;
secret values must not enter public evidence or logs. The exact old release,
images, schema checksums, roles, data/artifact hashes, writer identities and
restore results must be bound to the same recovery point. No new daily table or
control row may be invented to make the V2 controller accept schema59.

The [cold-cluster recovery command and50 regression definitions](../runbooks/r2-preupgrade-recovery.md)
are now written but unexecuted. Whole-cluster recovery preserves roles and
access policy without requiring new daily tables; its additional database
outage belongs in the explicit G4 packet. The bounded maintenance entry point,
writer observations/locks and no-clobber archive publication are now written.
Unmanaged host/external writer fencing still requires a complete G4 inventory.
It requires the fresh host inventory and a reviewed command set. It must
complete before role bootstrap, migration or the first release-link mutation.
The current daily backup controller is retained for post-upgrade operations.

## Existing backup installation needs an explicit version transition

The installer is intentionally fresh-install only: it refuses all six existing
targets before writing. Re-running `--apply` is not an upgrade, and deleting its
targets to evade that guard is not the proposed procedure. See
[no-clobber gate](../../scripts/ops/install-production-backup.sh#L147).

The six targets are the helper `/opt/lagrange/bin/run-production-backup.sh`,
protected `/etc/lagrange/production-backup.conf`, and the backup/verify service
and timer pairs under `/etc/systemd/system/`. The new unit sandbox additionally
allows the existing release lock file; its path and metadata must already be
valid. Both timers are persistent, so starting them can immediately run overdue
work. Sources: [backup service](../../deploy/systemd/lagrange-production-backup.service#L26),
[backup timer](../../deploy/systemd/lagrange-production-backup.timer#L5),
[verify timer](../../deploy/systemd/lagrange-production-backup-verify.timer#L5).

Prepare one bounded version transition, rather than weaken the fresh-install
guard. Its reviewable execution ledger must specify:

1. Expected current hashes/owners/modes for all six files, exact loaded unit
   definitions and timer/service state; refuse an unexpected or replaced file.
2. Exclusive release/backup ownership and a deadline; stop only the approved
   timer/service identities and wait for actual terminal state before replacement.
3. Private old-byte/metadata recovery copies, verified new files bound to the
   target release, and per-file atomic replacement with a transition journal.
4. Daemon reload, exact installed/loaded-byte verification and the official
   installed `--check` path before any timer start. A config must refer to the
   actual selected immutable release and image, never a moving checkout.
5. Failure recovery that restores only a target still matching this transition's
   written bytes. Foreign changes stop recovery. Timer resumption additionally
   requires schema/release compatibility; restoring old helper bytes after
   schema62/daily state exists is not permission to resume V1 backup.

Follow-up source preparation: the [bounded transition and recovery tool](../runbooks/r2-first-backup-transition.md)
and 22 filesystem fixtures are now written, with CI wiring. They are unexecuted
and not independently reviewed. Exact old-byte bindings and the actual unit
observations remain missing. No live installation was inspected or changed.

## Approved-base whole-flow verification

The next integrated test must use the actual compiled V3 approval pins and
unaltered approval API. `produce_daily_once` opens the approved historical root
before constructing the protected prefix; the admitter repeats approval
independently. Neither a dummy base nor a replacement test registry closes this
gate. See [producer](../../crates/job-queue/src/owner_beta/runner/daily_ingest.rs#L513)
and [admitter](../../crates/job-queue/src/owner_beta/runner/daily_ingest.rs#L803).

Source tracing found C04, an actual path-contract mismatch before execution:
the [installed config](../../scripts/ops/lib/owner-beta-daily-release.py#L32)
and [read-only mount](../../deploy/compose/compose.owner-beta-daily.yml#L86)
both name `/data/raw`, while the
[evidence reader](../../crates/market-data/src/daily_price_only/source.rs#L1180)
adds its own `raw` child. Passing the configured leaf unchanged therefore
selected `/data/raw/raw`. The
[ingest boundary](../../crates/job-queue/src/owner_beta/runner/daily_ingest.rs#L231)
now requires the terminal component `raw` and passes only its parent to that
existing reader. Mounts, the policy/approval registry and the reader's held-FD,
no-follow, ownership and shared-lock checks remain unchanged.

Four new regression definitions use the actual reader on private filesystem
fixtures: correct outer scope despite malformed nested decoys, no fallback to
valid nested controls when the selected scope is absent, a symlink refusal and
explicit path-name validation. Three require Linux. They are WRITTEN_UNEXECUTED;
edition2024 rustfmt and whitespace checks are not compiler/runtime acceptance.
The earlier Wave81 hosted success remains scoped evidence and did not prove
this installed path. C04 stays in the same Wave82 candidate, without separate CI.

Prepare a disposable host-local test using read-only approved-base input,
explicitly isolated fixture Raw, an empty schema62 database and unique test
paths. Keep the three real identities: producer10002, admitter10003 and
reader/worker10001. Run the actual candidate executable, with no database
credentials or network on the producer and only the test internal database
network on the other roles. Private base/Raw bytes must not be uploaded to
hosted CI. Any generated extension is labeled synthetic; real market-data
receipt and the three-trading-day observation remain separate.

Require first and subsequent trading-session candidate/admission/frontier
lineage, actual-role DB admission, worker computation/publication, API as_of and
replay identity. Include process restart and a captured-state recovery without
changing source bytes, resetting poll clocks or rewriting run identity. Bound
every process and preserve cleanup evidence. Exit0, healthy processes or a
`NOT_DUE` result alone cannot substitute for those outcomes. The finite CLI
diagnostics are documented by
[the real entrypoint](../../crates/job-queue/src/bin/owner-beta-runner/daily.rs#L221).

This host-local fixture is still to be implemented and reviewed. Source and
hosted tests remain useful evidence, but do not close this integration gap.

## Ordered closeout and the eventual approval packet

1. Wave81 full recovery now accepts its hosted package scope; the
   [final receipt](2026-10-16-r2-backup-migration-mode-ci.md) preserves all limits.
   Batch further executable changes
   for the two maintenance gaps and whole-flow fixture into one coherent
   candidate; preserve earlier failed runs and avoid per-file CI dispatches.
2. Integrate the prepared PUML source/evidence and render both PNGs through the
   permitted local route. The current isolated draft has286 checked citations;
   no rendering or integration acceptance is claimed. Obtain independent
   integrated review without treating coordinator self-review as independent.
   No new worker delegation is authorized by this document.
3. Fix the final candidate, verification evidence and operational-input shapes.
   Prepare the exact host-local integration commands and a fresh read-only host
   admission scope; do not revive an expired observation or grant.
4. Prepare12 images through the official sequential builder: one Rust-producing
   service at a time, `CARGO_BUILD_JOBS=2`, `COMPOSE_PARALLEL_LIMIT=1`, background
   low-priority service and health/resource checks between the established
   batches. Record real image IDs and strict manifest, not a dry-run substitute.
5. Complete the pre-upgrade recovery proof and exact six-file transition. Bind
   all commands to the actual target/previous revisions, image manifest,
   protected config, Owner/entitlement, first automatic session, available Raw,
   request budget, interruption deadline and compatible rollback branch.
6. Seek only the operational authority missing from the prepared packet, then
   apply, verify the first real publication and arrange actual Owner use.
   Begin the three-consecutive-trading-day observation only after the installed
   candidate and first data/serving path are accepted. Calendar time spent on
   source work does not count toward those three days.

The live command ledger is intentionally not presented as ready for approval:
final candidate/images, fresh host identities, maintenance/restore commands,
operational inputs and deadline are not yet fixed. These are remaining work,
not an implicit request for broader permissions. The existing
[G3/G4/G5/G6 conditions](../superpowers/plans/2026-10-13-r2-daily-research-beta.md#L460)
and R2 acceptance criteria are unchanged.
