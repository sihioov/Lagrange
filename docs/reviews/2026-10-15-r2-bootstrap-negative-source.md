# R2 I0d — actual negative bootstrap fixture source disposition

Baseline: `d6bd790440be816c92a790dfc21ed64fb298b3e7`; prior accepted Linux S2
candidate: `7cdf3a68c754231bd4d828305ff6733638bcfe37`. This change addresses the
unwritten behavioral cases in the [role wiring review](2026-10-14-r2-daily-role-wiring-review.md)
and the remaining gate in [I0c recovery](2026-10-14-r2-admitter-helper-ci-recovery.md).
The [adopted A5 contract](2026-10-14-r2-daily-input-design-amendments.md#a5-d3-05--first-migration-readiness-and-validation-ownership)
requires role readiness before first0060 execution.

Coordinator source disposition: ready for one exact-candidate hosted run.
All **83 named cases are WRITTEN_UNEXECUTED** at this source handoff. No new
independent worker review, DB behavior or executable acceptance is claimed.

## Exact change

- New [bootstrap-negative.sh](../../deploy/db/integration-validation/bootstrap-negative.sh)
  calls the unchanged production bootstrap in the official disposable cluster.
- [validate.sh](../../deploy/db/integration-validation/validate.sh) mounts the
  fixture, prepares private non-root-readable synthetic inputs, calls it before
  migrations, rejects missing/duplicate/malformed outcomes, and records each
  case in JSON/TSV plus a sanitized named log.
- [static-check.sh](../../deploy/db/integration-validation/static-check.sh)
  requires the fixture and its wiring and checks shell syntax. The validation
  [README](../../deploy/db/integration-validation/README.md) states the coverage.

No production bootstrap, provisioner, production Compose, workflow, migration,
Rust source, credential policy or service activation changes. No application
dependency/runtime structure change requiring a diagram update occurs here.
I6 diagrams remain required before the later structural main merge.

## Written scenario inventory

| Group | Count | Required distinction |
| --- | ---: | --- |
| Positive initial, rerun, after-failures | 3 | Real bootstrap exit0/final marker and safe admitter attributes/privileges |
| Invalid attributes | 7 | NOLOGIN, INHERIT, SUPERUSER, CREATEDB, CREATEROLE, REPLICATION, BYPASSRLS rejected without repair |
| Incoming/outgoing membership | 2 | Neither direction silently revoked/accepted |
| Protected-role paths | 8 | app/worker × migration_owner/admitter × direct/transitive; transitive bridge explicitly non-inheriting with SET permission |
| CREATE privileges | 6 | Direct database/public schema/other schema, schema ownership, PUBLIC database/other schema with absent admitter |
| Credential-file failures | 56 | Eight files × missing/empty/directory/symlink/LF/CR/unreadable |
| Late transaction failure | 1 | Actual database-ACL lock timeout after seven role statements and public-schema CREATE revocation; full rollback |

The SQL oracle captures role rows including password verifiers, memberships,
database owner/ACL and public/fixture schema owners/ACLs inside a supervisor-only
fixture schema. It compares the full JSONB value before/after each rejection;
password values/verifiers and snapshots never cross into CI output. ACL entries
are sorted effective grants, so incidental array ordering is not the oracle.
Invalid attributes/grants themselves remain present after rejection, proving
that the bootstrap did not silently repair them. Fixture restoration is separate.

All eight file paths use the real read_secret path. The fixture runs as the
non-root postgres OS user and asserts regular-file unreadability after chmod0000;
this is not a root-only/directory substitute. Shell rejection must return1 and
one exact fixed diagnostic. SQL preflight rejection must return3 and one expected
fixed reason. Every real bootstrap invocation must remove its SQL temporary file.
The post-fixture seven direct-role login and admitter catalog/GUC checks remain.

The late rollback fixture drops the admitter and grants PUBLIC CREATE on public
in the disposable database. A separate psql connection acknowledges its uncommitted
database-ACL change before bootstrap begins. The bootstrap's existing lock_timeout
must reject the database GRANT after eight DO completions (preflight + seven roles)
and the schema REVOKE. Catalog equality is checked before and after holder ROLLBACK;
the new admitter must still be absent and the revoked public CREATE restored.
The final positive bootstrap recreates the valid role and removes legacy public
CREATE. No production hook, mock psql, SQL rewrite or timing-only sleep is used.

Bounds: connect5s, control statement20s, bootstrap25s, acknowledgments10s,
holder idle60s, whole hosted fixture300s. Exit cleanup terminates/waits for any
holder and removes private inputs. On failure the official caller retains only
fixed case IDs and phase tokens, not raw bootstrap/SQL diagnostics.

## Checks and limits

Coordinator inspected all fixture/caller paths, exact production preflight and
mutation order, role maps, error labels, expected exits, shell quoting, non-root
file semantics, bounded lock handshake, snapshot isolation and cleanup. Three
scoped `bash -n` checks and whitespace inspection passed. These are syntax/source
checks only; no local fixture, project checker, provisioner, SQL/DB, Docker,
Cargo/compiler/lint/test, provider or production command was executed.

Prior I0b and I0c owners were reconfirmed actual idle/activeTurn null/pending0.
No delegation or new worker. Related-history lookup failed with MCP_ERROR and
was not retried; no memory-save claim. Original reviews, prior failed CI and
C05 bounded S2 acceptance remain intact.

The freeze receipt records exact hashes and all 83 names. Hosted acceptance
requires every named result plus full CI recovery at the pinned candidate,
unchanged S2/99 Rust/6 separate chart expectations, strict lint/format, migration
checksums, existing role/helper gates and cleanup. Aggregate counts alone do
not establish individual results. Unprinted nested helper rows/exits stay UNKNOWN.

Not covered: production roles/secrets, provisioner filesystem rollback, whole
runtime-tree atomicity, real runtime10002/10003 ACL/mount access, migration0060
UP/DOWN or its races, provider/copy reconstruction/admission, runtime/Owner and
final C_R2. Membership-cycle fixtures are outside this bounded matrix; existing
UNION termination remains a source conclusion. R2-04/09 remain DOING.
