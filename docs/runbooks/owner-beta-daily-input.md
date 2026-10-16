# Owner daily ETF11 input — controlled installation

Roadmap: R2-04, R2-09. This runbook tracks the I6 installation contract.
Source implementation and hosted fixtures do not establish operational readiness.
Production activation remains the reviewed G4 operation with exact release,
inputs, impact, resource observations, backup and rollback evidence.

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

UID10001 receives no daily write root. Future Compose bindings must preserve
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
steps belong to the next integration package and are not implemented by this
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
Current backup code covers PostgreSQL, Raw and Curated; it does not yet archive
the new daily sibling state or immutable policy. Production use must wait for
that gap to close.

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

The supported release wrapper and DB transport are the mandatory I6B2c follow-up.
Until that is connected, this overlay is not a supported activation route.
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
