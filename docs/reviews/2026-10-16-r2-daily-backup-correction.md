# R2 I6B3-C01 backup verification correction

Roadmap R2-04/R2-09; baseline `977e356792881fa50cad2ff95ababc04434e4c91`.
The [failed exact-candidate run](2026-10-16-r2-daily-backup-ci.md) was fully recovered
before any correction. No prior result is overwritten or rerun.

Six source/test/workflow files are changed. Both fixed maintenance SQL files now
use `pg_catalog.row_security_active('public.users'::regclass)` to distinguish an
actual loss of complete identity visibility from the normal enabled-but-not-forced
RLS catalog flag. Actual login/session role checks, transaction/lock bounds,
per-owner scans, consistency probes and original-reservation identity remain.
There is no policy, grant, schema, BYPASSRLS or row_security configuration change.

The new actual-role Rust case verifies normal migration_owner identity/ownership,
complete visibility and audit/repair success, then forces users RLS only in its
isolated disposable database. The same actual login must see zero identities;
both fixed transactions must reject with SQLSTATE 23514 and their exact visibility
error. History and reservations remain unchanged. Existing eleven SQL definitions
are retained; twelve SQL cases are now written, with no local execution.

The file round-trip fixture again owns its three manifest UID/GID/mode assertions.
The controller ordering fixture supplies the exact image mapping read by its
production call path. Existing failure/order assertions are retained.
Three independent hosted fixtures (files, isolated Docker restore and legacy
operations) run with `!cancelled()` after an earlier step failure. No
`continue-on-error` is allowed, and the workflow contract asserts that boundary;
failed steps still fail the policy and required jobs.

Direct source review, scoped standalone edition-2024 rustfmt and whitespace
checks are the local verification scope. No compiler, Clippy, project parser,
Python fixture, SQL or Docker execution occurred locally. All 86 definitions
for this corrected package remain WRITTEN_UNEXECUTED: 64 file/controller, six
Docker, twelve SQL, two transport and two enable cases. A single coherent hosted
candidate may follow exact-head all-event existing-run and remote-SHA checks.
No new independent review, source acceptance or runtime acceptance is claimed.

The runbook's obsolete foundation-only wording now points to the connected
source and its failed verification. Separate concurrent Archify/DOC-02/DOC-03
changes are excluded from this correction. The original 72-hour host authority
expired at 2026-10-16T05:11:48Z; no production renewal or action is implied.
Whole-flow verification, both source/PNG diagrams, independent integrated review,
concrete G3/G4, actual Owner use and three trading days remain open.
