# R2 I6B3 consistent backup and reconciliation source

Roadmap R2-04/R2-09. Direct coordinator work, no new worker. This coherent
candidate includes the Wave77 Owner-equity recovery fixture correction
`3c100be`, the backup foundation `645e4a2`, and the connected controller below.
The previous candidate/run remains failed; no previous run is repeated.

The installed `compose-release` owns three commands: `--daily-backup`,
`--daily-backup-verify`, and `--daily-reconcile`. Existing backup timers route
through this wrapper whenever daily state or the reader floor exists. A missing
helper, stale candidate/config, foreign path or mixed operation fails; it cannot
fall back to the historical three-class backup. Existing V1 sets are preserved
and remain subject to the original bounded retention policy.

The controller holds the release and backup locks, disables the DB control and
drains its exact owned cohort. It retains both private poll locks and existing
Raw publication locks during capture. The six file classes are Raw, Curated,
daily state/candidates/admissions/frontiers, installed config/policy, reader floor,
and the approved historical base. Before/copy/after byte inventories bind names,
owners, modes, hashes, native identities and protected ancestors. The existing
service-owned artifact container directory is bound explicitly; other ancestors
remain root-owned. Nothing is recursively chowned or reset.

The complete PostgreSQL dump retains owners and ACLs. It and the file bundle and
DB-reference record are encrypted with the existing key-file/PBKDF2 contract in
private staging. Verification actually decrypts and reconstructs the complete
file tree in an empty disposable directory, then inventories it again. A pinned
PostgreSQL container with network=none, UID999, read-only root, bounded resources,
and only fixed verification inputs restores the dump and executes the same
audit. Its result must equal the live disabled audit before publication. The
source DB audit and final source-file inventory must still match before the
atomic COMPLETE set publication. Plaintext staging and owned verification
containers have bounded cleanup; failures are never verification success.

Only a complete, previously healthy ON cohort can resume. Its exact original
container IDs/creation identities, policy, API generation and current DB authority
are checked; control is enabled last. Initially OFF/partial/unhealthy state,
orphan evidence, retention failure, changed source, failed restore or failed
resume stays OFF. No API/market-stream restart, Compose replacement, provider
request, grant, schema change or trading path is introduced by backup.

Reconciliation is separate and always leaves control OFF. A fixed transaction
repairs only the reservation of an original surviving run/job. An extra admitted
leaf can use one bounded native independent-admitter cycle; the actual DB keys
must be exactly the prior keys plus the captured orphan keys. A successful process
exit alone is insufficient. Busy/NotDue, multiple remaining orphans, or an
unreferenced frontier alone stays pending. There is no poll-clock reset, new
candidate request, replacement enqueue, history rewrite, or automatic enabling.
The ordinary enable path now checks the full disabled file/DB reconciliation
before replacing or starting daily members. The already-healthy idempotent path
also requires the DB already be ENABLED.

Written validation comprises 85 new definitions since the failed Wave77
candidate: 64 file/reference/restore/resume/controller/routing/retention cases,
6 actual Docker encryption/full-schema restore cases, 11 actual-role maintenance
SQL cases, 2 additional fixed-SQL transport cases and 2 enable-gate cases. The
existing Owner-equity recovery case is corrected, not counted as a new case.
All 85 are **WRITTEN_UNEXECUTED**. The policy job gets a bounded 20-minute budget
for its actual disposable fixtures. The dedicated DB lane adds the backup SQL
target to its existing 18 targets. All previous tests and no-SKIP rules remain.

Only direct source/text/diff/whitespace review and scoped Rustfmt checks have
been used locally. No Python project import/parser, shell helper, compiler,
test, SQL, Docker or production execution occurred. One fresh exact-candidate
hosted CI may follow the all-event existing-run query and remote-SHA check.
Independent integrated review, approved-base first/multiday whole flow, required
PUML/PNG evidence, concrete operational G4, actual Owner use, three trading days
and final C_R2 remain open. Source review is not executable or runtime acceptance.

Operational preparation must reconcile the installed backup helper/unit/config
versions: the existing installer still refuses to overwrite targets. This source
change does not silently upgrade installed files or renew the expired historical
72-hour host authorization. No production operation has been attempted.
