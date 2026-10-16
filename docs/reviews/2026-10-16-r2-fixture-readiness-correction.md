# R2 I6B3-C02 fixture readiness and bounded diagnostics

Baseline `61f9c7f85acee5dcbacbd04c138786ce1a068cdc` failed
[run 38861769625 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38861769625).
All eight jobs and full evidence were recovered before these five files changed.
[The recovery](2026-10-16-r2-daily-backup-correction-ci.md) preserves successful
Rust/PG checks, two database-fixture setup failures and one Web setup-request
failure. Their underlying causes remain UNKNOWN; this change does not relabel
the previous run or claim that a readiness race was proven.

| File | Bounded correction |
|---|---|
| [Transport fixture](../../scripts/ops/owner-beta-daily-transport-self-test.py#L55) | Require authenticated TCP SQL against the target database, exact database/current-user/session-user identity and a TCP server address before role creation. Preserve the 90-second total readiness bound and bound each probe. |
| [Backup fixture](../../scripts/ops/owner-beta-daily-backup-docker-self-test.py#L66) | Apply the same readiness rule. SQL failures expose only a fixed stage, numeric exit and five-character SQLSTATE; SQL, passwords, database values and stderr are never emitted. Migration application still uses the actual unprivileged migration-owner login. |
| [Web evidence projection](../../scripts/qa/candidate-web-e2e-runner.py#L1487) | For a failed known test file and an exact Playwright API-request prefix, emit one of seven fixed request-error categories. No free-form message, URL, stack, call log or body crosses the projection. Failure, retry and cleanup semantics are unchanged. |
| [Web supervisor tests](../../scripts/qa/test_candidate_web_e2e_runner.py#L1828) | Extend two existing definitions with seven category controls and five malformed/untrusted/success exclusions, a private sentinel check, and unchanged failed status/retry count assertions. The supervisor still has 34 definitions. |
| [Dedicated PG harness](../../deploy/db/integration-validation/validate.sh#L1540) | Emit the backup target's individual named rows and require each of its 12 existing names exactly once with success, plus exactly 12 total rows. Preserve target exit/no-SKIP and all prior migration/role/cleanup gates. |

The two actual database fixtures each add
`test_actual_database_probe_requires_target_sql_and_keeps_errors_closed`:
`pg_isready` can report an accepting server for a nonexistent database, whereas
the actual SQL probe must reject it; the real target probe must match identity.
A deliberate missing-table query must produce only the fixed `42P01` diagnostic.
These are two distinct class-qualified tests, not duplicate result rows.

The complete backup package therefore has 88 written definitions: the prior
86 plus these two controls. Full transport has 11 and backup Docker has seven;
the 64 file/controller definitions, 12 actual-role backup SQL definitions and
prior operations cases remain. The new candidate must also retain all carried
Rust, Python, Web, strict-lint, format, PostgreSQL and cleanup checks.
Every new or affected definition remains WRITTEN_UNEXECUTED for this candidate.

Direct source inspection covered the complete five-file delta, bounded polling,
secret transfer through mounted files, SQL failure projection, fixture cleanup,
Web output allowlisting and all 12 PG result names against concrete Rust source.
Text/name/hash bookkeeping and whitespace checks are permitted locally; no
changed Python/shell script, compiler, test, parser, Docker or DB was executed.
Direct coordinator review is not independent review or execution acceptance.

One coherent hosted candidate may follow frozen successor inputs, exact-head
all-event existing-run checks and remote-SHA verification. No old run is rerun,
cancelled or duplicated. Documentation-only updates do not start another run.
Whole approved-base first-day/multiday verification, both PUML/PNG diagrams,
independent integrated review, concrete G3/G4 release transition, production
application, real Owner use and three actual trading days remain open.
The previous 72-hour production authority has expired; this source correction
does not renew it. No provider or production action is included.
