# R2 I4f superseded daily claims — source review

Date: 2026-10-16 KST. Roadmap R2-04/R2-09, Wave58.
Baseline c4f35bb7b81a3e8fb3f6c9297315daade24733f1, fully recovered
[Wave57 bounded acceptance](2026-10-16-r2-daily-runner-correction-ci.md).
Coordinator direct review only; no new delegation or independent review claimed.
All new tests are WRITTEN_UNEXECUTED; compilation and hosted evidence are pending.

## Scope and behavior

Add migration0061 UP/DOWN only; all0060 bytes/guards remain unchanged.
The worker-only SECURITY DEFINER function rejects elevated SET ROLE sessions,
uses migration_owner, fixed search_path and bounded lock/statement settings.
It checks exact job/run/attempt/worker, immutable payload/reservation and no outputs.
Locks follow job→attempt→run→control→common source fence. Real lease and eligible
KST cutoff are sampled after waits. Same policy and a continuously proven newer
eligible session are required; missing/conflicting evidence returns false.

An exact completed supersession is idempotent. Otherwise only a current RUNNING
claim may atomically become job CANCELED/attempt FAILED/run CANCELED with code
DAILY_SESSION_SUPERSEDED and cleared lease. Permanent reservations remain intact;
no items, factor/target/cash output or publication receipt is created. Existing0060
triggers and deferred consistency enforce each transition and final state.
Current publishing permission is not required for cleanup: disabled control,
revoked entitlement or opted-out config cannot force stale work to publish.
Actor GUC is restored on success and exceptions. Expired/foreign claims cannot settle.

Existing daily preflight, success transaction and failure cleanup call this typed
function before their full success gate or ordinary failure settlement. SQLSTATE55000
maps to lost claim; generic23514 never implies supersession. Historical publication
and the existing manual cancellation path are preserved. Source placement of the
success-path call is reviewed; full approved-filesystem success remains untested.

## Written verification and migration compatibility

Fourteen new actual-role DB tests are appended to the existing ten runner tests.
The fixture projects synthetic calendar evidence through the real admitter API.
Only its disposable DB replaces the expected-time wrapper; it still calls the
unchanged0060 expected_at logic. The test clock table grants SELECT only to
migration_owner; the lease clock is always real. This is not live calendar proof.

The tests cover cutoff before/after, exact idempotence, failure cleanup, revoked
authority, missing/conflicting calendar, rollback, waiting on control, foreign
identity/payload, real expired lease, real app/admitter/elevated and impersonated
principal denial, actor restoration, populated61down/up and manual cancel reason.
Two source-fence races verify exact blocking backend PIDs: event first prevents
cancellation; cancellation first holds its shared fence through atomic commit.
No sleep, suite serialization or weakened guard is introduced.

| New DB definition | Location |
| --- | --- |
| daily_supersession_preflight_cancels_and_replays_only_exact_identity | [line 589](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L589) |
| daily_supersession_failure_cleanup_cancels_instead_of_retrying | [line 628](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L628) |
| daily_supersession_cleanup_remains_possible_after_authority_revocation | [line 652](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L652) |
| daily_supersession_missing_calendar_does_not_invent_a_newer_session | [line 676](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L676) |
| daily_supersession_conflicting_calendar_does_not_cancel | [line 696](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L696) |
| daily_supersession_transaction_rollback_restores_every_output_and_lease | [line 724](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L724) |
| daily_supersession_samples_cutoff_after_waiting_for_control | [line 748](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L748) |
| daily_supersession_foreign_worker_attempt_and_payload_cannot_cancel | [line 783](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L783) |
| daily_supersession_expired_real_lease_cannot_settle | [line 832](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L832) |
| daily_supersession_rejects_other_real_principals_and_restores_actor | [line 859](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L859) |
| daily_supersession_additive_migration_down_up_preserves_populated_rows | [line 921](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L921) |
| daily_supersession_does_not_replace_an_audited_manual_cancel_reason | [line 968](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L968) |
| daily_supersession_observes_source_conflict_committed_while_waiting | [line 1014](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L1014) |
| daily_supersession_holds_source_fence_until_atomic_commit | [line 1061](../../crates/job-queue/tests/owner_beta_daily_runner.rs#L1061) |

The dedicated PG lane requires exactly24 runner names once each with exit0/noSKIP.
All previous17 outer targets and named gates remain required. Full-release empty
install/rerun now expects61 and checks committed migration checksums. The staged0060
endpoint remains60. Its existing repeat_is_noop test explicitly calls run_to(60)
so adding61 cannot accidentally change the fixed0060 test boundary. The new test
separately checks0061 down/up, function owner/ACL/settings and unchanged rows.
This isolated downgrade test does not authorize an operational downgrade while
a newer binary remains active.

## Exact files and static results

| File | Lines | SHA256 |
| --- | ---: | --- |
| [migrations/0061_owner_beta_daily_supersession.up.sql](../../migrations/0061_owner_beta_daily_supersession.up.sql) | 159 | f7e62e6a21a41b391290d4af8fd8c25c136bc6f03262813fbbe13b5b95312521 |
| [migrations/0061_owner_beta_daily_supersession.down.sql](../../migrations/0061_owner_beta_daily_supersession.down.sql) | 4 | 18f9b4ac56089c9b6e38ffb0e094ae1a8ace26b85d65c0312deed59d125ad6ea |
| [crates/job-queue/src/owner_beta/publish.rs](../../crates/job-queue/src/owner_beta/publish.rs) | 1614 | 1efb59fa1fc7e389d41d250b95b62fa69f36a754e409c4cf6941f4ca3dad2b2b |
| [crates/job-queue/tests/owner_beta_daily_runner.rs](../../crates/job-queue/tests/owner_beta_daily_runner.rs) | 1114 | 34d1f250ccd178e10ea9c768cd3eba0d322ed0b8d6c3e9ada8a10c6e4f5a6f89 |
| [deploy/db/integration-validation/validate.sh](../../deploy/db/integration-validation/validate.sh) | 1986 | 81161dd70f34b8e6d22daa1d23f1a3d8248e6bc7533085afc4a80574d155ee02 |
| [tests/integration/migration-contract/tests/migration_contract.rs](../../tests/integration/migration-contract/tests/migration_contract.rs) | 14081 | 10fc29834ef1516b82e1c9ad1fc91722c19c9551e01d1101150fdd85a686dd10 |

The historical publication prefix and all previous runner definitions are preserved.
Scoped standalone edition2024 rustfmt and whitespace checks passed. Every changed
hunk, fourteen new definitions and the SQL transition/permission order were directly
read against the concrete0060 source. No local Cargo/rustc/Clippy/tests/SQL/DB/helper
or workflow execution occurred. No provider, production or runtime change occurred.
Inputs and successor inventory will pin these bytes before one exact-candidate hosted
CI, after all-event existing-run query and remote equality. Never rerun/cancel/duplicate.

Full producer/admitter wiring, real approved-base whole flow, distinct UID/ACL/crash
and installer rollback, integrated independent review, I5-I7/diagrams before main,
operations/Owner/three trading days/final C_R2 remain open. Ralph continues.
