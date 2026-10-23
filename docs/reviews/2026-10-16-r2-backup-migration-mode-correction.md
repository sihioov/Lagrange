# R2 I6B3-C03 backup fixture migration execution

R2-04/09; direct coordinator correction after full recovery of
[4336fbe/run38866283743 attempt1](2026-10-16-r2-fixture-readiness-ci.md).
Policy setup reported `DAILY_BACKUP_FIXTURE_SQL:migration:3:25001`; all seven
native backup bodies were NOT_EXECUTED. No retry or production action occurred.

The only changed executable file is
[the backup Docker fixture](../../scripts/ops/owner-beta-daily-backup-docker-self-test.py:148).
It previously wrapped every migration in BEGIN/COMMIT. Existing migrations
0024/0025/0027/0028/0029/0031/0032 each begin with `-- no-transaction` and contain
one concurrent-index statement. The observed error is consistent with this
source defect; the old diagnostic does not directly identify the failing number.

The fixture now sends those declared nontransactional scripts without an outer
transaction. All other scripts retain BEGIN/COMMIT, including 0030's SET LOCAL
and constraint attachment. Migration source bytes, SQLx production execution,
roles, grants and the fail-closed preparation gate are unchanged.

The existing
[full-schema encrypted restore case](../../scripts/ops/owner-beta-daily-backup-docker-self-test.py:248)
now also reads the actual62-row migration ledger and compares every version,
SHA384 checksum and success flag. It checks all seven concurrent indexes in
the PostgreSQL catalog for expected table/owner, validity, readiness, liveness
and uniqueness before the dump. Existing encryption, actual networkless restore,
full audit equality and container-cleanup assertions remain. There are still
seven native backup definitions and88 package definitions; none are bypassed.

Direct source/diff review and whitespace checks passed. No local Python import,
parser, SQL, Docker, compiler or test ran. This is not independent review or
execution acceptance. One exact-candidate hosted run follows input freezing,
all-event existing-run query and remote-SHA verification; old runs remain intact.

Actual approved-base first/multiday three-UID execution, both PUML/PNG diagrams,
independent integrated review, concrete G3/G4 preparation, operational application,
Owner use and three real consecutive trading days remain outstanding. No runtime
or final R2 acceptance is claimed.
