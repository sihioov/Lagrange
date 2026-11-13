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

## Operator and embedded callers

Run the immutable release's `scripts/ops/lib/owner-beta-paper-lifecycle.py` with
one of these commands and `--install-root`. The existing
`owner-beta-paper-operations.py` entry routes to the same owned lifecycle:

- `configure --caller-key K --proposal PATH`
- `observe-reset`
- `enable --caller-key K`
- `recover --caller-key K --proposal PATH`
- `readback --generation G [--sequence 0]`

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
