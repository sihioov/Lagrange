# R2 Wave76 controlled daily activation: source review

Roadmap R2-04/09. Baseline606d3f6 after full CI38842068730 recovery.
Direct coordinator source review; independent integrated review remains open.
Twelve changed owned paths. The prior daily Compose overlay is unchanged.

The sole compose-release wrapper now exposes daily prepare/enable/disable.
Plan is static; preflight observes protected inputs and DB state. Apply requires
the installed current commit, strict canonical12-image manifest, root-protected
env/config/policy and the same installation lock as the immutable installer.
The prior process owner must be the current commit or an explicit installed
--daily-from-commit. Project/image/revision/generation ownership is exact.
Aliases reuse the worker image; migration transport reuses db-migrate.

Preparation leaves control disabled, requires actual process quiescence and
explicit capacity/lease/backoff. Initial absence is accepted only when all ten
daily tables are absent or present coherently, there is no retained daily
history, and private checkpoints/shared leaves prove pristine filesystem state.
Existing actor-scoped recommendation-run RLS is preserved: the bounded existing
user IDs are selected from the non-RLS identity table to inspect each actor.
A missing reservation/control row with a surviving daily run is not bootstrap.
No new grant, role, policy bypass, checkpoint reset or admission is introduced.

Enable validates the existing production config and embedded approved artifact
(2452 sessions/26972 bars), preserving the existing market-stream process-local
restart confirmation. The approval wrapper receives the verified installation
root in a clean environment, not the caller's override. Equal healthy current
API/cohort can be re-enabled without a restart or epoch reset. Otherwise old DB
control is disabled, all owned processes drain, existing private state and
target disabled binding are checked, and only clean stopped old containers are
removed. The target API and three processes start using pinned images/no build.
Actual runtime/API policy and generation pass before DB enable; generations are
checked again afterwards. Partial startup, policy failure or post-enable change
attempts DB disable and owned drain. Foreign containers are never taken over.

Shutdown does not depend on unrelated Auth0/provider activation validation or
new capacity settings. DB disable failure or protected context failure still
attempts complete owned drain; neither failure is reported as successful OFF.
Target cleanup attempts DB disable before capture so a replaced container cannot
prevent the DB shutdown attempt. No force kill, history deletion or automatic
new work is introduced. The prior bounded drain implementation remains unchanged.

Normal release apply now holds the shared installation lock through its full
operation, with inherited-descriptor identity verification. It checks actual
daily process quiescence before changing any service, then DB/filesystem
quiescence after PostgreSQL is available and before migrations/startup.
First legacy OFF with no reader floor or daily processes needs no DB query.
The base Compose API/worker are explicitly daily-disabled; only the controlled
overlay enables them. Existing explicit non-daily activation gates remain.

The fixed DB transport runs one uniquely labelled migration image container as
999:999, read-only root, no capabilities, no-new-privileges, init, restart=no,
128MiB/one CPU, internal owner-beta-db network and one read-only migration secret.
It verifies the actual unique labelled PostgreSQL endpoint and pinned image.
Temporary paths include a mask of the PostgreSQL image's declared data volume.
No raw environment/inspection/SQL/password is printed or persisted. Statements
travel over stdin. The entrypoint accepts only fixed component-mode DB arguments,
rejects URL alternatives, removes inherited PG settings and suppresses psql
diagnostic bodies. One closed response plus actual exit0 is required.
Cleanup rechecks nonce/image/creation ownership; a client timeout is not treated
as proof of process exit. Only the owned one-shot is signalled/removed.

Twenty-nine injected/control-root definitions cover ordering, partial failures,
revocation/generation change, safe cleanup, idempotence, data-root mismatch,
market-stream confirmation, clean approval environment, preparation/quiescence,
actual shared-lock exclusion and retained inherited ownership, protected context,
literal dotenv rejection, pristine history, manifest and closed diagnostics.
Eight actual Docker/PostgreSQL definitions exercise the real entrypoint and
status SQL, four states, partial/orphaned/actor-RLS history, wrong password/SQL,
container isolation, URL/role rejection and owned fixture cleanup.
The transport uses synthetic disposable tables/credentials and a small image
built from the same pinned PostgreSQL base. It does not validate the complete
installed production image, full migration schema or approved-base Rust flow.
Those distinctions remain separate from existing real-migration PG CI gates.

The legacy operations fixture changes only its fake daemon host flag and
explicit no-history guard stub; it does not prove the daily controller.
New unit/Docker names must each be recovered once with ok. Existing root58
(including prior runtime24), actual-role PG schedule43 and all other
Rust/PG/Web/format/strict-lint/cleanup gates remain required.

Local checks are source/diff/hash/name bookkeeping and whitespace only.
All37 new definitions are WRITTEN_UNEXECUTED; no local project import/parser,
helper/checker/test/build/compiler/Docker/DB/provisioner/provider/production
execution occurred. No operational activation, database mutation or release
installation occurred. Source review is not compiler/test/runtime acceptance.

Consistent backup/reconciliation, both evidence-bound diagrams, approved-base
first and multi-day whole flow, independent integrated review, concrete G4
operational disposition, Owner use, three actual trading days and final C_R2
remain open. The continuous Ralph objective does not end at this package.

| File | Lines | SHA256 |
| --- | ---: | --- |
| scripts/ops/compose-release.sh | 844 | dee2b569ad8283e76f0097f670c00c52872c44a54a01eb3c56d5294f3a5e6002 |
| scripts/ops/lib/owner-beta-daily-operations.py | 770 | 4ca598038407fca0532dc41486679e34a59f457789b6b0354c11b06f31605089 |
| scripts/ops/lib/owner-beta-daily-context.sh | 36 | 7c5f78070300c35cfd5a4cf98dd65dc3e7d999442ba5e97f85c1236c84648ac2 |
| deploy/db/owner-beta-daily-control-status.sql | 81 | 068832d3783f31612d791e4e09af722b79cc3f1d5d3ced9e9d376311ee3bbc99 |
| deploy/db/daily-control.sh | 31 | 802a78c67ca68300f515dd5d603628b31646d89564703f2905235176e6e099c6 |
| deploy/db/Dockerfile | 33 | 37c36b0f475c078fa9a4e39a71667edc08fb427aa4aee9df214405dc07ca196d |
| deploy/compose/compose.yml | 1636 | d76fc74c0f68ab57b84ce021764084513455bf8fa344c71a73f89b5b93c2fede |
| scripts/ops/owner-beta-daily-operations-self-test.py | 379 | 422325a9303c372421cdca223fb6fb5e1ee0ba9c99c4c7365b3b6e83eea1cfc2 |
| scripts/ops/owner-beta-daily-transport-self-test.py | 278 | 233a7b580c37a943230af686a67c7c4b5a15c6efb772a59b8a07d6a88241c61d |
| .github/workflows/ci.yml | 250 | 89f333d86a0832de821b3e0d6ce6e659a69e7306791d5039a3f13a93687282ed |
| docs/runbooks/owner-beta-daily-input.md | 264 | c7a3cb9a7992de117da393bd3745c387d99626880ec64f00040d913ab081b85d |
| scripts/ops/production-ops-self-test.sh | 1251 | 0d83fcdd67e5be530dc7ceff067d745f6659754148bf43972dbdbaf5e596de0e |
