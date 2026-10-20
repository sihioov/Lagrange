# Owner daily ETF11 input — controlled installation

Roadmap: R2-04, R2-09. This runbook tracks the I6 installation contract.
Source implementation and hosted fixtures do not establish operational readiness.
Production activation remains the reviewed G4 operation with exact release,
inputs, impact, resource observations, backup and rollback evidence.

## R2 source collection and legacy publication

R2 reads verified Raw evidence independently of the legacy EOD normalizer.
A valid cash-dividend source can therefore be useful to R2 while the legacy
`--backfill-session-dates` pipeline correctly stops with an unsupported-action
error. Do not relax that normalizer, reset its failed state, or label Raw
collection as canonical DB publication.

The explicit `research-worker --eod-raw-session-dates CSV` command verifies and
reuses a unique compatible complete Raw source before considering capture. A
conflicting, corrupt or incomplete source fails closed. Executing acquisition
reconciles durable Raw crash residue; the separate
`--eod-raw-check-session-dates CSV` command reads committed bytes only, requires
an existing manifest/lock, and never constructs a provider or DB connection.
Neither command makes the source an admitted daily input.

Use the immutable release's `backfill-production.sh --raw-source` for an explicit
historical session list derived from actual missing evidence. It keeps the
existing protected state/lock, sequential shared token/rate coordination and
calendar snapshot limits. Its distinct `LAGRANGE_EOD_RAW_STATE_V1` state records
`RAW_COMMITTED`, never `PUBLISHED`; V4 state is rejected in either direction.
Current-day capture must be a separate invocation after16:30KST, using the durable
calendar claim. Combining it with historical dates is rejected before dispatch.
Historical Raw captures keep one committed dedicated calendar original and
attach exact `copied_from` references for later covered dates. Both the original
target and each requested target must appear in the verified response before
non-calendar requests; a missing target does not authorize another calendar
request. Legacy same-date canonical ingestion retains its stricter contract.

Discovery retains all physical Raw integrity/budget checks while selecting the
configured entitlement. Existing unlinked cached calendars are never witnesses:
only a byte-identical, valid earlier original under that same entitlement can
make a redundant delivery ignorable. No Raw edits, synthesized lineage, alternate
entitlement fallback or weakened explicit-file validation is permitted.

`kis-daily-production.sh --raw-source` is the current-session source mode. It
checks today's validated XKRX session and cutoff, verifies existing Raw in a
provider-free worker, and captures only an absent source. It does not query the
legacy publication frontier or infer historical catchup. Closed days and
pre-cutoff activations make no worker/provider call. `--check` performs the
provider-free source observation without capture or progress writes; it does
start a one-shot Docker worker after the cutoff.

Pin this choice with `install-kis-daily.sh --raw-source` in the new immutable
release's unit. Existing default legacy units remain distinguishable. All normal
before16:30KST apply and later install-disabled rules still apply. A source
collection receipt is not calendar proof, civil-day action coverage, a readiness
gate or permission for recommendation catchup. Historical recovery, recurring
complete action-range supply, producer/admitter verification and controlled
daily activation remain separately required.

The2026-10-20 `stock_kind` wire correction also requires the matched newly
verified producer/admitter/runtime release. Do not modify an installed release
or mix its older ASCII-only readers with newly admitted Hangul labels.

## Immutable input installation

The production release installer accepts one optional
`--daily-config-source ABSOLUTE_PATH` only with `--apply`.
`OWNER_BETA_DAILY_MODE` is `disabled` when absent; valid explicit values are
`disabled` and `owner_only`. ON also requires Owner access, sealed price input
and disabled Paper. The same closed grammar is used by the release installer
and production configuration validator.

ON apply requires the separate root:root 0600 config. Explicit OFF preparation
may supply it too. The JSON must bind the exact release commit, existing
canonical policy hash, nonzero Owner UUID, entitlement reference/hash, explicit
first automatic session and positive bounded scan/artifact/replay/cycle budgets.
The checked-in fixture values are synthetic and must never become operational
defaults. The first automatic session is not inferred from the current date.
DB capacity, lease and backoff remain separate explicit control inputs.

The immutable release contains `.owner-beta-daily/config.json` and
`policy.json`: root:10001 0440 below a root:10001 0750 directory. The canonical
policy pins the existing approved historical v3 base and ETF11 price-only
contract. All ancestors are root-owned without writable group/other bits or
extended ACLs. Files must be regular, single-link and non-symlinked. Inputs are
bounded, identity checked and copied once after archive ownership normalization.
The original external JSON is never rewritten.

Installed config, when present even in OFF mode, is revalidated against the
requested release commit before publication, after publication and on
`--check` or `--rollback`. A failed staging operation never changes current.
Apply and rollback use the same nonblocking root-owned release lock; a busy
lock aborts instead of overlapping installations. Check remains read-only.

## State preparation and identities

Filesystem preparation is a separate reviewed step using the helper in the
exact installed release and its verified config. It does not start a process,
call a provider, open a DB, create a control row or enable daily scheduling.

The data root is the existing protected root-owned directory
(`provision-linux.sh` already requires root ownership). Daily state is a sibling
of Raw and artifact storage at `<LAGRANGE_DATA_DIR>/owner-beta-daily`; it must
not be placed below a service-writable artifact directory.

| Path | Owner/group | Mode | Purpose |
| --- | --- | --- | --- |
| daily root | 0:10001 | 0750 | protected namespace |
| candidates | 10002:10001 | 0750 | producer publication |
| admitted, frontiers | 10003:10001 | 0750 | independent admission publication |
| producer-state | 10002:10002 | 0700 | producer checkpoint and lock |
| admitter-state | 10003:10003 | 0700 | admitter checkpoint and lock |
| checkpoint.json, poll.lock | stage UID:GID | 0600 | private stage state |

UID10001 receives no daily write root. The daily Compose bindings preserve
read-only access for worker inputs, network-free/no-secret producer isolation
and only the dedicated restricted DB credential for the admitter.

New state is prepared behind a root-only staging ancestor, fsynced and published
with no-replace rename. Existing valid advanced checkpoints are reused unchanged.
A foreign Owner/policy, partial layout, active lock, unexpected leaf or crash
residue fails closed. Do not reset or delete state to make a preflight pass;
runtime recovery owns the exact existing state. An uncertain publication retains
its protected staging tree for reconciliation.

## Compatible rollback

Before the first ON link selection, the installer reserves a durable
`.owner-beta-daily-reader-floor` under the install root. It is root:root 0600.
Once present, both ON and OFF target releases must contain the exact committed
reader declaration covering schema 60/61/62 and historical plus daily input.
A missing declaration, invalid floor, or old reader is rejected before current
changes. A later failure keeps the conservative floor. Never erase the floor
or delete daily history to permit rollback.

`reader-check` verifies compatibility without writing a floor.
`reader-guard` reserves it before an actual link transition.
The floor is conservative installation history, not a DB-history query.
Use the compatible installer to select a rollback target; executing an older
installer is not a supported downgrade route.

An OFF link selection preserves history and changes which release is selected.
Operational OFF also requires the shared DB control change and verified graceful
drain of producer, admitter and daily worker processes. Those controlled runtime
steps use the controlled Compose release operation below, separately from this
installer. Existing processes are never silently counted as stopped.

## Image aliases

The manifest retains exactly twelve canonical image records. Optional daily
Compose override generation maps the two fixed daily aliases to the
`owner-beta-runner` image ID and revision. It adds no build target or image
record. Default OFF override remains twelve entries; the optional ON override
has fourteen runtime service entries. Actual container identity, command,
mount, UID and network checks remain required with runtime integration.

## Verification and remaining release work

The source package adds twelve actual root filesystem/install tests using
temporary `/var/lib/lagrange-daily-integration-*` repositories and release roots.
They exercise OFF/ON input requirements, exact protected bytes, wrong commit/
unsafe config, old-reader rejection, compatible OFF rollback, read-only checks,
busy installation locks, mode input boundaries and canonical image aliases.
The existing sixteen filesystem/principal cases and three Rust parity tests
remain required. The legacy production-ops fixture uses fake Docker/system
commands; its success is not actual runtime or network evidence.

Before G4, complete controlled Compose selection and old-process drain, DB
control synchronization, daily-file/policy/DB backup and restore reconciliation,
actual three-UID approved-base-to-continuation flow, independent integrated
review, architecture source/PNG updates and exact-candidate verification.
The connected daily backup and restored-state path is described in
[Daily backup and restored-state reconciliation](#daily-backup-and-restored-state-reconciliation).
Its first whole candidate failed hosted verification; the effective identity
visibility and fixture corrections are documented in the
[I6B3-C01 report](../reviews/2026-10-16-r2-daily-backup-correction.md).
The corrected candidate has no executable or operational acceptance yet. Its
archive/reference checks prove byte consistency, while only the existing native
admitter may independently reverify extra leaves. Resume permits only the
original healthy three-container cohort after complete drain, with DB enable last.

G4 must enumerate exact immutable revision/manifest, policy and approved-base
pins, verified available evidence/catch-up scope, Owner/entitlement/first-session
and resource budgets, role/grant checks, resource/OOM observations, backup and
the tested disable/recovery procedure. Existing KIS approvals remain bounded to
the adopted read-only surfaces; no new account, order, KIND automation or
provider permission follows from this installation work.

## Release database control boundary

The fixed `deploy/db/owner-beta-daily-control.sql` template is rendered only
after verifying the installed root-owned config, exact commit and canonical
policy. `render-control` emits SQL; it opens no connection and starts no service.
It is an internal building block for the official immutable release wrapper,
not a separate operational activation path. Runtime wiring and G4 remain open.

The transaction accepts only a direct `migration_owner` session. `prepare`
creates a disabled epoch-zero row only when absent. Existing Owner, policy,
entitlement, first session, trigger and declared capacity/lease/backoff must
match exactly. Preparation never repairs or replaces an existing binding.
The future runtime caller must derive capacity from the same protected
`MAX_JOBS_PER_OWNER` value used by the actual API process.

`enable` rechecks current Owner role and exact entitlement. `disable` keeps
the stored capacity/lease/backoff and works after Owner-role or entitlement
revocation. It still verifies the installed identity/policy binding. Both
transitions use the existing epoch trigger; an identical repeat changes no
epoch or timestamp. No admission, reservation, job or historical row is deleted.
`check` and `check-disabled` neither insert nor update a control row.

The renderer encodes config/policy as hex data, rejects open-ended actions and
numeric text, and accepts no capacity arguments for disable. The fixed SQL
has bounded lock/statement timeouts and one atomic transaction. A downstream
caller must require successful final process exit as well as the exact fixed
result; seeing a result line alone cannot prove commit.

Wave74 adds six isolated root renderer tests and eleven real PostgreSQL role,
atomicity, concurrency, revocation, literal-data and lock-timeout cases to the
existing 32 schedule definitions. These are hosted-only checks. No operational
control change, process drain or backup consistency is claimed by this source.

Final R2 still requires actual Owner use and three observed trading days.

## Container observation and graceful drain primitives

The inactive `compose.owner-beta-daily.yml` describes the three separate runtime
identities. Producer UID10002 has no network or credential; admitter UID10003 has
only its restricted DB secret/internal DB network; worker UID10001 has read-only
admitted inputs. All retain read-only root filesystems, dropped capabilities,
no-new-privileges, bounded CPU/memory/logs and exact immutable config mounts.
The aliases continue sharing the one canonical owner-beta-runner image.

`owner-beta-daily-runtime.py` is an internal primitive, not an activation CLI.
Its caller must supply a verified installed manifest/config/project binding and
hold the release operation lock. It inspects the explicit local Docker socket;
ambient Docker host/context variables cannot redirect its observations.

Ownership and runtime policy are separate checks. A wrong image/project/revision
or ambiguous container set fails before mutation. A container demonstrably owned
by that release may still be drained when its UID/mount/network policy is wrong.
Capture binds each ID, creation/start time and restart count. Drain rechecks the
whole cohort, disables every restart policy, sends SIGTERM to all running members
and waits once under a 3670-second ceiling. It never sends SIGKILL or silently
restores restart policies. Replacement, restart, pause, OOM, failed exit or timeout
is not a successful stop. Already absent or cleanly stopped cohorts are checked
without inventing a running-process observation.

The I6B2c wrapper connects this overlay through the controlled operations below.
Its new executable verification is pending in Wave76; no production activation
is accepted from source review alone.
DB disabled alone does not stop admission polling. The caller must attempt DB
disable and drain the already verified owned cohort even if the DB is unavailable,
then report failure rather than claim a disabled/backup-safe state. Preparation
checks that take private poll locks belong after actual drain and before startup.

Hosted tests use both injected failure cases and an actual disposable Docker
fixture. The fixture selects the checked-in overlay and actual worker stanza,
uses a trapped shell process under the pinned Alpine base and verifies UID,
capabilities, mount write denial, internal/no-network routing and real SIGTERM
exit/restart behavior. It does not run the Rust producer/admitter computation or
prove the approved historical-base whole flow, production isolation or G4.


## Controlled prepare, enable and disable (Wave76 verification pending)

Only the installed current release's `compose-release.sh` is the supported operator
entrypoint. New operations are `--daily-prepare`, `--daily-enable` and
`--daily-disable`, with the existing `--plan|--preflight|--apply` modes.
They cannot be combined with infrastructure, backfill, calendar or quote refresh.
`--daily-from-commit <exact installed old commit>` explicitly identifies an
old cohort; no old manifest or project is inferred from a mutable tag. The
protected service-status policy supplies the project, or the original literal
`lagrange-station` when no policy exists. Cross-project daily services fail closed.

Prepare/enable require the installed ON config and explicit protected
`MAX_JOBS_PER_OWNER`, `PRICE_BETA_LEASE_SECS` and `PRICE_BETA_BACKOFF_SECS`.
Data, artifact and secret roots must also be explicit absolute protected values.
Prepare provisions/reuses state without resetting checkpoints and creates only
a disabled control row. It refuses an enabled control. Missing control is a
fresh bootstrap only if the fixed DB history observation and filesystem
checkpoints/publication directories are pristine; missing rows are not repaired
by deleting history. Actor-scoped existing recommendation history is observed
under its existing migration-owner RLS for bounded existing identities.

Enable first validates the protected configuration and approved historical base.
An exact healthy API/cohort may recheck authority idempotently. Otherwise it
disables DB scheduling, drains the verified old three-process cohort, validates
private state and removes only the exact stopped old containers without force
or volume deletion. It starts pinned target API/producer/admitter/worker images
with no build and the existing fixed KIS/service-status overlay selections.
Actual API mode/capacity and strict process/image/UID/mount/network policies must
pass before DB enable. Final generation checks follow enable. Any partial start
or changed generation attempts DB disable and owned target drain. No history,
reservation, artifact or checkpoint reset is a recovery action.

Disable verifies ownership before depending on runtime config. It attempts DB
disable first and still attempts actual owned drain if config rendering or DB
connection fails. An error in either part means failure, even if some processes
stopped. It does not depend on unrelated Auth0 or revoked activation rights.
A successful disabled receipt requires both actual DB result and complete
graceful exit; it is not yet a consistent-backup receipt.

The DB transport uses the pinned current migration image, UID999, one read-only
migration credential, bounded CPU/memory, read-only root, dropped capabilities
and only the verified internal project DB network. PostgreSQL identity/alias and
the local Docker socket are checked. Only fixed rendered transactions and a
fixed read-only status statement are sent over stdin. Password bytes, raw
inspection and diagnostics are never printed/persisted. A one-shot needs a clean
exit and one exact closed result; timeout/error cannot become OFF. Cleanup
re-inspects its unique ownership and removes only a stopped one-shot.

All runtime operations share the installer's nonblocking release lock. Full
release apply retains that lock across process guard, PostgreSQL startup, DB
guard, migrations and serving startup. A fresh legacy OFF installation can
bootstrap; existing daily processes must already be stopped and scheduling
disabled. Normal release Compose has literal daily-OFF API/worker defaults;
it never activates daily work from an ambient mode. Existing refresh paths
also require quiescence before replacing the API.

Hosted tests distinguish injected ordering failures and real root file/lock
checks from a disposable actual Docker/psql transport fixture. The latter uses
the real entrypoint and fixed status SQL against synthetic tables, credentials
and actor RLS. Full migrated authority tests remain a separate existing PG lane.
Neither fixture proves the Rust approved-base whole flow or operational G4.
Consistent backup/restore reconciliation, diagrams, full-flow verification and
independent integrated review remain mandatory before operational acceptance.

## Daily backup and restored-state reconciliation

Source is written; hosted and operational acceptance are still pending. Use only
the installed current release's `compose-release.sh --scope release` entrypoint.
`--daily-backup --plan`, `--daily-backup-verify --plan`, and
`--daily-reconcile --plan` describe the operation without reading protected inputs.
`--preflight` checks protected local inputs, with no Docker/DB execution.
`--apply` is an operational action requiring the applicable approved maintenance
scope. `--backup-config` selects the root:root0600 configuration, defaulting to
`/etc/lagrange/production-backup.conf`; its exact release and image pins must match.
No `--daily-from-commit`, other scope, or refresh operation can be combined.

Backup disables/drains daily, captures PostgreSQL plus all six file classes,
encrypts them and actually restores both into disposable isolated resources.
The historical base's service-owned parent is retained unchanged. Only a verified
previous healthy ON cohort may resume by the same container IDs, with DB enable
last. Any unresolved orphan, failure, originally OFF or incomplete cohort remains
OFF. The API and market-stream are not restarted. A `VERIFIED_DISABLED` result
proves backup verification only, not that daily scheduling resumed.

Restored files must first be restored under a separately reviewed concrete
maintenance plan; these commands never extract over live paths. Afterwards
`--daily-reconcile --apply` checks exact DB-referenced artifacts and repairs only
the original surviving run/job reservation. One native admitter cycle may recover
captured orphan admissions without changing identity. Remaining orphans, Busy,
NotDue or a frontier-only residue remains pending/OFF; do not reset clocks,
delete evidence or substitute a new job. The next ordinary `--daily-enable`
independently checks reconciliation again. Reconciliation itself never enables.

V1 sets remain historical PostgreSQL/Raw/Curated evidence. V2 COMPLETE sets add
daily evidence and are restored using their matching installed source/config.
The existing timer helper routes daily/floor-present installations to the current
controller and refuses legacy fallback. Validate installed helper/unit/config
versions before activation; the installer does not overwrite existing targets.
The service sandbox retains read-only release/data trees and exposes only the
existing `.lagrange-release.lock` file as writable in addition to backup storage.

[Source scope and validation limits](../reviews/2026-10-16-r2-daily-backup-source.md)
are part of the eventual exact-candidate acceptance, not operational evidence.


### Wave134 query-date clarification — 2026-10-21 KST

For PaidinSubscription only, the query/cell date is the strictly parsed
`sub_term_ft` (eight ASCII digits, valid date). GB1=1 is subscription-date
selection; the independent GB1=2 PaidinRecord chain remains mandatory and uses
record_date. Other classes keep record_date. Every original row still passes
record/right/rate/symbol validation before the universe filter. A target ETF
subscription in the extension is NonCashAction even when its record date is
older. No date is rewritten, no source is dropped to hide a conflict, and no
subscription date becomes an availability timestamp. This corrects the blanket
record_date statement above; complete seven-class coverage remains mandatory.

Primary source: [official KIS paid-in query contract](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_llm/domestic_stock/ksdinfo_paidin_capin/ksdinfo_paidin_capin.py)
and [response field mapping](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_llm/domestic_stock/ksdinfo_paidin_capin/chk_ksdinfo_paidin_capin.py).
The official sources were read only, never executed. Eight actual non-ETF rows
across Sep3/7/9/10/11 match this distinction. A changed read-only reconstruction
at15:40:01UTC verifies all five dates; the unchanged Raw manifest hashes remain
2009b5ea…/d85be240…. The Aug29..Oct20 inventory now has13verified civil days and
40missing. Calendar coverage Sep24..Oct6 remains absent; no eligibility or
admission is asserted. Four new regression functions plus33existing cases and
market-data all-target Clippy pass. The two diagnostic copy include-path build
failures and corrected build are retained; production sources were not involved.
