# Owner-beta Paper host lifecycle

R5-04 source implements the host caller. ENABLE's native readiness gate remains
false. Hosted compiler/PG checks, same-candidate private genuine Source evidence,
G4, actual pre3/post3 sessions and Owner acceptance remain separate requirements.
No command in this runbook grants new provider, trading or account permissions.

## Bootstrap and immutable preparation

Use an installed, verified OFF-reader release for initial `configure`. Its
process binding is absent. The native operation creates the actual account at
CONFIGURED/OFF epoch 0; its independent COMMITTED readback contains the original
SQL result. Account consent is obtained through the existing Owner flow. Do not
synthesize either identifier or seed SQL outside that flow.

Prepare a **later distinct immutable release** using those actual account and
consent facts. Its bounded canonical process proposal must be a root:root 0400
regular file under protected parents. Pass it as
`deploy-production-release.sh --apply ... --paper-process-binding-source PATH`.
The installer retains and rechecks the proposal while `paper-release.stage`
validates it and writes process.json before publishing the release directory.
Without this option the release remains an OFF reader. An already published
commit/path cannot be rewritten to add process.json. Installation does not start
Paper services or change native/Compose/API activation gates.

The release and Source installer prepare `.owner-beta-paper-lifecycle` directories
without creating any account, selection, anchor or readiness marker.

The first OFF-reader install on a pre-Paper root (database at migration 0065 or
lower) is admitted by the installer's fixed read-only Paper history inventory,
not by a caller-supplied assertion. When it proves no Paper history it returns
`LEGACY_OFF` and reserves no Paper reader floor; the floor appears only with the
first configured history (the journal created by `configure`), at the next
switch. The empty `.owner-beta-paper-lifecycle` skeleton is not history and does
not by itself require a reader. Any partial, unknown or failed state makes the
install fail closed. Procedure and rollback limits:
`docs/runbooks/production-release-and-backup.md`, "First Paper-capable release on
a pre-Paper root"; contract: `docs/contracts/owner-beta-paper-operations-v1.md`
section 6.1.

## Owner consent preparation

The immutable API release defaults to `OWNER_BETA_PAPER_OFF_CONSENT_ENABLED=false`.
An explicitly prepared installation sets this flag to `true` with
`OWNER_BETA_ACCESS_MODE=owner_only` and `OWNER_BETA_PAPER_MODE=disabled`.
First verify the installed OFF reader, nightly and Paper history compatibility,
and the original `CONFIGURED` result and independent readback. The release
validator pins the 0071 consent migration bytes before it accepts the opt-in. The
normal release applies the migrations before starting the API, which checks
the resulting protected scheduling function before serving consent.

The Owner signs in normally and uses the consent section at `/paper`.
The authenticated OFF view supplies the account and control epoch used by the
existing CSRF-protected consent action. A grant checks current `paper_view`
and `recommendation` rights. The account Owner can revoke recorded consent
after rights withdrawal. The API requires the original configured OFF account
and refuses a prior activation, an in-progress lifecycle transition, or a
process binding. A completed combined backup that only fenced this OFF account
retains eligibility when its original drain and verified OFF events match.
Revocation advances the database epoch immediately. The next ordinary backup
or reconcile operation checks the exact Owner revocation events from the
protected journal's floor and advances that floor under the release lock.
A preflight can recognize this pending advance without changing the journal.
Complete the reconciliation before changing this installation's consent mode.

A restore verifies the audit SQL originally retained in the archive against
its capability and descriptor. The 0071 verifier selects the audit SQL by the
archived schema. It recognizes separate pins for 0069, R2 dividend migration
0070, and R5 consent migration 0071.
The consent path keeps the generic Paper surface, previews, apply and the
Paper runner behind their existing gates. The generic recommendation scheduler
excludes the Owner-beta Paper account's opt-in at both discovery and enqueue. The 0071
downgrade requires a revoked or inactive beta binding and preserves prior
consent events and recommendations.

Verify the resulting consent event independently before preparing the later
distinct immutable Paper process binding. Count actual same-cohort sessions
only after the remaining pre-soak conditions have been met.

## Daily backup before Paper activation

On 2026-11-25 the Owner approved daily full backups throughout pre-activation
observation. Use the [canonical nightly service install and preflight](production-release-and-backup.md#install-the-nightly-simulation-backup-for-an-inactive-daily-installation)
with the release's `daily-simulation` backup profile. Preflight checks the fixed
Daily date and mode on the installed release. A configured account must be
`OFF`, agree with the retained journal floor, and have no active Paper process
binding. Preflight emits `NIGHTLY_PAPER CONFIGURED_OFF` with its source digest. A verified
empty Paper history uses the existing Daily/simulation route. The explicit
combined backup remains available when an operator needs a fresh OFF boundary.

The configured OFF route keeps the original live Paper and Daily control rows
and their epochs. The nightly sequencer fences the exact API and simulation
generations and the original Daily cohort. If the current release has a running
source witness, its next slot must already be `ARMED`, with enough time remaining
for the complete backup lease and the original 60-second dispatch margin. The
backup holds the witness's root-owned `.control/archive.lock` over read-only
file selection. It opens the same protected inode read-only inside the nightly
service sandbox and takes an exclusive lock against the witness's shared lock.
The original daemon, one-use claim, timer and live Current
heartbeat stay in place. A captured Current and claim are historical archive
members. The restored guest starts with its own local custody.

Before pausing the status observer or stopping the cohort, preflight checks that
the nightly state and encrypted destination filesystems are writable and have
the configured minimum free space. It also counts retained set bytes against
the configured cap. The apply preflight checks the protected date claim and
its exact release and configuration binding. The root-only workspace under
`/var/lib/lagrange/backup-state/nightly-simulation/<KST-date>` holds the selected
projection, database dump, file bundle and restore scratch on disk. Size
that filesystem for the plaintext capture and isolated verification, and size
the backup destination for the encrypted set. The runtime still checks free
space during capture and publication, exact retained and new bytes before
publishing, and the descriptor limits during native file verification.
Successful cleanup removes the owned scratch. If a failure leaves scratch,
retain it with the failed date journal for attended recovery.

The archive contains the unchanged PostgreSQL dump and audit from one exported
read-only snapshot, its selected file originals, and the exact Paper/Daily
control commitments. The enclosing six-member encrypted set is unchanged.
Inside it, `owner-beta-paper-archive-v2` uses
`paper-combined-configured-off` and a typed `source_proof`. The journaled
verification requires the archive digest from this nightly attempt. The
networkless restore guest verifies the original database and files first,
performs the guest-only `RESTORED_OFF` transition, and proves that its Daily
control is OFF. If Daily was ON in the original, the guest records that isolated
control change separately. The serving SQLx migration ledger remains at 0071
for the Paper consent release.

The nightly sequence resumes the previously running cohort after archive
verification. Before starting the API, the installed wrapper rechecks the
verified Paper origin and exact Daily mode and epoch under its release lock.
A final readback must reproduce the original control, consent and witness
identity for the date to pass. A refusal preserves the step record for
manual recovery. Acknowledge a failed night only after fresh readback matches
the journaled Paper identity. Before backup starts, the protected preflight
identity supplies that proof. Once backup starts, recovery requires its exact
archive and captured identity. Acknowledgment preserves the failed outcome.
Follow the [attended recovery procedure](production-release-and-backup.md#nightly-backup-on-a-simulation-enabled-release-2026-11-20)
when the original identity is still provable. If consent changed, the witness
advanced to another reservation, or the original identity is uncertain,
retain the failed claim and journal. Report which identity changed using the
protected readback, or report that the difference could not be determined.
The Owner must decide how to resolve the failed attempt. Preserve legitimate
consent changes, source reservations and archived receipts during that review.
Confirm two consecutive successful nights on the same
configured OFF account with a stable epoch and consent. Include one genuine
source witness that remains armed through backup and dispatches its original
claim normally. Check the encrypted set's isolated restore and the services
that returned. Deployment, those completed nights and the actual trading
session observation remain R5-04 evidence.

## Operator and embedded callers

Run the immutable release's `scripts/ops/lib/owner-beta-paper-lifecycle.py` with
one of these commands and `--install-root`. The existing
`owner-beta-paper-operations.py` entry routes to the same owned lifecycle:

- `configure --caller-key K --proposal PATH`
- `observe-reset`
- `enable --caller-key K`
- `recover --caller-key K --proposal PATH`
- `readback --generation G [--sequence 0]`

### Restarting a manual combined backup after a stop timeout

If the protected backup journal reached `STOPPED` but SQL still shows
`DRAINING`, retain the failed generation, its result files and the timeout
record. The original stop inventory expires after 30 seconds. For this case
the backup controller can begin a linked generation and observe writers again.
This requires the same boot and installed release, the original committed
BEGIN, and fresh observations matching the original terminal writers. A later
action, partial handoff or unknown commit needs separate investigation.

During an authorized attended Paper backup, use the original protected backup
configuration and the maintained tool pinned to the current application:

```bash
sudo /usr/bin/python3 -I -S -B \
  "/opt/lagrange/maintenance/${TOOL_COMMIT:?}/scripts/ops/lib/owner-beta-daily-backup.py" \
  --operation backup --mode apply --install-root /opt/lagrange \
  --application-release "/opt/lagrange/releases/${APPLICATION_COMMIT:?}" \
  --config-file "${BACKUP_CONFIG:?}" \
  --paper-restart-generation "${FAILED_GENERATION:?}" --paper-restart-key "${CALLER_KEY:?}"
```

Choose one caller key for that operation and retain it with the original
generation ID. After a lost command response, reuse that exact pair. A
`BACKUP_RESTART_RECORDED` response identifies a previously recorded successor.
Verify the persisted encrypted set independently before claiming a usable
backup. The original failed attempt stays in the journal.

### Lifecycle proposals

Configure and recovery proposals are bounded canonical root0400 originals.
Recovery's supplied G, installation, account, target, epochs and `G:recover` key
must already correlate; the caller does not reconstruct them. Native readers
independently validate every proposal. CONFIGURE/ENABLE/RECOVERY use one original
R/A/E/Q action, with no synthetic BEGIN or stage chain.

Embedded callers use
`open_lifecycle(actual_daily_context, inherited_release_lock_fd, deadline=N)`.
`N` is a built-in integer containing an **absolute `time.monotonic_ns()` value**;
for the genuine fixture owner pass `owner.deadline.value`. The caller retains its
original release lock descriptor. Do not pass a deadline object or create a new
clock after failure. Existing host timeout ceilings and the native 15-second
clock remain unchanged.

Methods return `LifecycleResult(outcome, native_result, readback, artifact,
cleanup)`, containing exact original bytes or `None`. Outcomes are OK, REPLAY,
BLOCKED and COMMIT_UNKNOWN; cleanup is COMPLETE or UNKNOWN. These are descriptive
values. Consume success only after the enclosing context exits successfully.
Final cleanup failure raises `LifecycleError("CLEANUP_UNKNOWN", result=...)` and
retains unresolved local custody; it cannot start services or publish an anchor.

CONFIGURE's native artifact is its original proposal, ENABLE's is the SQL
response, and RECOVER's is its original exact-readback document. Independent
readback verifies the original request/event/result correlations. ACK loss uses
only the original dispatch and exact Q. A retained dispatch is never sent as a
mutation again. Later COMMITTED readback cannot upgrade an uncertain cleanup.
Readback needs no Current, worker password or eligibility check; stale epochs,
Daily OFF and Current INVALID do not change its original Q.

## Source ordering and ownership

The existing Source service starts and owns Current, then runs a short lifecycle
barrier before `Journal.claim` and ARM_INTENT on each future session. The barrier
queries actual inventory. An unconfigured legacy Daily installation proceeds
without inventing a Paper reset. A configured installation selects the real RESET
using the retained local floor and exact readback. Selection is immutable and
content-addressed. An OFF/RESTORED_OFF observation may publish an actual native
anchor only after helper and context cleanup.

After ENABLE, the current epoch can be 1 while the reset remains CONFIGURED at
0. The next Source session reuses the original same-boot anchor for that exact
reset/release/cohort. ENABLED on a new boot without such an original requires an
explicit drain/reset handoff; it cannot stamp an anchor retrospectively. A stale
local floor requires original readback/recovery, not normalization from SQL.
Prior boot anchors are retained. The native mount receives at most three boot
anchors; when more exist, the host selects the boots of the latest three actual
completed matching Source chains using the existing chain parser. Native still
performs all independent Source/SQL/eligibility checks.

The Source unit keeps `ProtectSystem=strict` and `PrivateTmp=true`. Writable paths
are limited to its existing evidence/state/collector lock plus the derived
installation release-lock file, existing Paper journal and lifecycle directory.
All native temporary stages live under that lifecycle directory's `staging/`;
no broad `/run`, installation parent or data-root write grant is added. A pending
stage blocks fresh Source ARM/mutation. Source sessions retain their existing
future-session clocks; no 90-minute controller deadline spans multiple days.

ENABLE/RECOVER retain the original worker historical root selected by the
protected environment. `base.source_artifact` remains entered through native
exit, import, removal and final original recheck. Each invocation creates a
root0 root0700/candidate0700/files0600 view of the exact approved V3 bytes, mounted
read-only at `/data/owner-beta-artifacts`. Persistent Daily roles/layout and the
Source helper's original host projection remain unchanged. Raw/admitted and
fixed config/policy/Current/events aliases are read-only; the helper has group
10001 and no capabilities. RESET/readback/observe use no worker credential or
DAC capability. Archive-owned RESTORE-BEGIN remains a separate unchanged path.

## Durable originals and remaining verification

Under root0700 `.owner-beta-paper-lifecycle/`:

- `queries/UUID/`: root0400 R, optional A/E/Q, selected native-result.json and
  artifact.json, plus root0700 output/ with actual native hash-named JSON files.
  Interrupted queries can lack terminal originals; that absence is preserved.
- `selections/H.json` and `anchors/H.json`: immutable existing native wires,
  root0400, at most 65536 bytes each. There is no mutable current-selection file.
- `staging/`: private temporary invocation inputs; never archive it as authority.

R/A/E bounds are 65536 bytes; Q retains the existing 1048576-byte exact-request
bound. Native stdout is at most 65537 bytes and selected artifacts 1048576 bytes. Anchor inventory and optional Source selection
are bounded to 4096 entries/files and 16 MiB each. The original mutation journal
remains `.owner-beta-paper/operations/G/actions/00000000`; durable anchor originals
are in the lifecycle sibling, not in removed native input stages.

Focused ordinary checks live in `test_owner_beta_paper_lifecycle.py` and the
existing operations/release/evidence suites. Actual UID view tests are in
`owner-beta-daily-base-self-test.py` and require the existing hosted Root lane.
Native/PG/Root/Source/genuine runtime checks were not run locally. The separate
P5 collector/consumer must preserve lifecycle originals descriptively and must
not import source anchors as destination authority. Runtime acceptance and final
source-pin/diagram registration belong to the coordinator.
