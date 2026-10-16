# R2 Wave74 immutable release database control: source review

Roadmap R2-04/09. Baseline89ff855 after complete CI38836892120 recovery.
Direct coordinator review, not independent integrated review. Seven owned
paths isolate the DB authority boundary before actual runtime activation.

The root-only renderer verifies installed config/commit and exact canonical
policy, reads a protected fixed SQL template, emits values only as hex bytes
and accepts five closed actions. Capacity/lease/backoff are explicit positive
bounded decimal values. Disable rejects those arguments and emits NULLs so a
changed capacity cannot obstruct the kill switch or be silently reset.
No arbitrary statement input, connection, secret, service start or provider call
is added to the helper.

The shipped SQL requires both session_user/current_user=migration_owner before
any input creation. It uses a fixed search path, bounded lock/statement/idle
timeouts and one transaction. Prepare creates only a disabled epoch-zero row;
failed Owner/rights authority rolls the insertion back. Existing identity,
policy JSON/hash, entitlement, first session, trigger and capacity bindings
must match. Check/check-disabled never insert or update. Enable rechecks
current Owner and exact entitlement; disable is permitted after their revocation.
Disable preserves changed capacity/lease/backoff, never repairs identity, and
leaves histories untouched. Existing triggers own epoch increments; repeated
actions do not change timestamps or epochs. Table locking serializes concurrent
prepare/transitions. Runtime callers must require final execution success in
addition to the fixed output, because a printed row is not commit evidence.

Eleven actual-role cases execute the embedded shipped template against fresh
migrated DBs: disabled/idempotent preparation, exact epochs, absent-row refusal,
serving/superuser/SET ROLE denial, binding mismatch, initial-authority rollback,
revoked authority, changed capacity plus preserved admission receipt, concurrent
transitions, quoted/Unicode literal data, and held-table lock timeout followed
by successful recovery. Connections close even after failed transactions.
The existing32 schedule definitions/helpers remain byte-identical; the dedicated
PG lane requires all43 exact names and no SKIP. No schema/grant/role changes.

Six disposable root renderer cases cover trusted exact bytes/CLI output,
closed grammar and bounds, disable NULL inputs, hex data, corrupt pins/template
with no stdout, and actual UID10003 refusal. Existing16 filesystem and12 installer
cases, legacy production-ops fixture, three Rust parity cases and all prior
Rust/PG/Web gates remain required.

Local verification: static source/diff/pin/name inspection, scoped standalone
edition-2024 rustfmt and whitespace only. No local Cargo/compiler/test, project
Python import/helper/checker, SQL/DB/root/Docker/provider/production execution.
All17 new definitions remain WRITTEN_UNEXECUTED until the one exact candidate
hosted CI is fully recovered. Source and compiler acceptance are not claimed.

The fixed SQL is not yet connected to a runtime command. I6B2b must bind the
actual protected API capacity, immutable Compose service identities/mounts/
principals, DB operations and graceful process drain. Consistent daily-file/
policy/DB backup and restore reconciliation, diagrams before main, actual
approved-base threeUID first/continuation flow, independent integrated review,
G4, real Owner use, three actual trading days and final C_R2 remain open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| deploy/db/owner-beta-daily-control.sql | 99 | f2855e670049583c687fa544b33411e843653fb3ab4c506076d0f151fa957974 |
| scripts/ops/lib/owner-beta-daily-release.py | 652 | 1e7a2c921faa42bc67ac685851e03110bf0dbe587d186779f7a6cc2938e15b9e |
| scripts/ops/owner-beta-daily-control-self-test.py | 155 | a6d170c805290bd1c85aa91d94fd54b0b2bf83b19ff6f8762215a94e0b481df0 |
| crates/job-queue/tests/owner_beta_daily_schedule.rs | 1505 | 0f813871c965a05968fd2d15cf4a1c55431adfa6e17425fa2b7c4a7d16990df3 |
| deploy/db/integration-validation/validate.sh | 2039 | a25a026ace19c9c185d69487184779b0a14e33ea58418b07fa91418757135c16 |
| .github/workflows/ci.yml | 242 | 956d209963d6526c9128a083e48b532cc09d82ba4f5b8800055b0a50eb1b770f |
| docs/runbooks/owner-beta-daily-input.md | 160 | a4e4a3a49ffd839e81f6c207274f3b82ee4d374d13b96ad81141af396e1f2f8d |
