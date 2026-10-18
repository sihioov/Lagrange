# R2 third image-maintenance proposal for 2a165fe

R2-04/09, Wave95. Status: **PREPARED_NOT_AUTHORIZED_NOT_EXECUTED**.
Candidate `2a165fe577e833529134627e980a129b9717ae6f` has complete
[hosted acceptance](2026-10-18-r2-d4-fixture-ci.md): CI39088097930 and separate
functional smoke39088099684, attempt1, both SUCCESS. Nine full log hashes,
eight actual checkouts,1174 input pins/519 committed blobs,118 required names,
15 image outcomes, publication/repeat invariance and ten cleanup outcomes
were recovered before this preparation. The terminal receipt SHA256 is
`f352936ddfe5b0952f9acf0757817455731f70e5a238d5a5c0667e4f601631cf`.

## Why this is a new decision

The two previous approvals each authorized one production interruption and
have been consumed. The first failed the normal-exit0 admission before the
builder started. The second passed normal exit0 and work settlement but
failed verification of its first DB image with
`product-image-mode-contract-drift`. Both restored the same original Owner V2
container and recovered the independent supervisor's terminal outcome.
Their failed units, root controls, receipts and unaccepted image are preserved.
The latest Owner continuation instruction explicitly leaves a third
production interruption unauthorized.

The successor fixes the daily-control image inventory/mode contract and the
synthetic D4 feature fixture. Its complete hosted verification now passes.
This supports another bounded image attempt; it does not prove production
image success or guarantee that another admission/build failure cannot occur.

## Requested single attempt and service impact

Approve one attempt for this exact candidate: fresh bounded read-only host,
runtime and aggregate SQL admission; official plan/preflight; new private
control installation; normal SIGTERM of the exact existing Owner V2 container;
the official serial twelve-image build; and restoration of that same container
on completion or failure, followed by read-only settlement checks.

Owner V2 daily/signal updates and its existing quote collection pause during
the interruption. Restoring the same container can resume its existing
collection and database recovery. Research-worker stays stopped and the other
nine serving containers stay healthy. The selected release and operational
configuration are preserved. Deployment, migration, research-worker start,
new provider access and root-policy changes remain separate permissions.

| Timing/control | Bound |
| --- | --- |
| Shared maintenance window |12hours, recorded once before independent recovery starts |
| Latest time to begin shutdown |Within the first hour |
| Build service runtime |10hours; background systemd, Nice10, idle I/O, CPU/IO weights20 |
| Independent watchdog |Requests build termination at hour10.5, reserves restoration time |
| Drained-reader attestation |Expires at hour11, no renewal |
| Normal completion or failure |Restore immediately after build/compiler termination |
| Build concurrency |Exactly one Compose service per invocation, Compose parallel1/Cargo jobs2; four batches of three |

Normal exit0, settled work and all official resource/health/OOM/termination
gates remain mandatory. No forced-kill fallback is introduced. The recovery
unit must be actually active before stop intent/SIGTERM. Recovery must prove
the same container ID/image is healthy and the independent unit completed;
unit disappearance alone is insufficient. A deadline failure is INCOMPLETE,
never reported as restored. A host/daemon outage can still delay recovery.

## Exact prepared artifacts

| Binding | Value |
| --- | --- |
| Source |`/data/worktrees/3puw275b/r2-release-2a165fe`, clean detached exact candidate |
| Source pins |2132 tracked file/symlink bytes and Git blob/mode identities matched |
| Inert image-only env |`target/r2-daily-beta/image-preparation-2a165fe/image-only.env`,0600, byte-identical to the earlier inert input |
| Private controls |`target/r2-daily-beta/wp-8/maintenance-2a165fe-a3/` |
| New root control directory |`/etc/lagrange/release-manifests/.r2-maintenance-2a165fe-a3-20261018` |
| New build unit |`lagrange-r2-build-2a165fe-a3.service` |
| New recovery unit |`lagrange-r2-recovery-2a165fe-a3.service` |
| New manifest |`/etc/lagrange/release-manifests/2a165fe577e833529134627e980a129b9717ae6f.manifest` |
| Required Owner container ID |`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4` |
| Required Owner image |`sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50` |

The [command ledger](../../target/r2-daily-beta/wp-8/maintenance-2a165fe-a3/command-ledger.json)
contains expanded argv, effects, preconditions and halt conditions. SHA256:
`62c77df8b1a4e09c2ccfa9b5442abea45050b87cac123140ebf410073d6b1d74`.
The [eleven-file preparation receipt](../../target/r2-daily-beta/wp-8/maintenance-2a165fe-a3/preparation.json)
SHA256 is
`744d4edbd1a86b35505929f8655b1fc67ddaee55cdfaebd499b0337ffe6266d1`.
The [source preparation](../../target/r2-daily-beta/image-preparation-2a165fe/preparation.json)
SHA256 is
`b7048920c1f122c86ca015e715a8054a387ebb8737ff17622622d241792490b3`.

All eleven controls were mechanically rebound from the preceding prepared
controls. Only candidate/path/unit/label bindings and dependent SHA pins changed;
inverse substitutions recover every baseline byte. The runtime probe, work
probe and SQL are byte-identical. The complete203-line diff was inspected,
expanded build argv matches the launch config, and the service template agrees
with that config. The original2125-file ab48a79 checkout and all old controls
were reconfirmed unchanged. No control was imported or executed, no local test
or project checker ran, and no new independent review is claimed.

## Ordered execution after a new approval

1. Reconcile the exact approval and current time. Recheck all2132 source pins,
   eleven controls, detached HEAD and inert env. Freshly observe root access,
   exact eleven container states/identities, memory/swap/OOM/compiler state,
   protected parents and absence of candidate manifest/build state/controls/
   units. These current production fields remain UNKNOWN during preparation.
2. Run the existing bounded runtime and explicit READ ONLY/repeatable-read
   aggregate probes. Require operator-wide visibility, settled jobs/attempts
   and the exact mode/identity contracts. Run official candidate plan/preflight
   and record its actual source-input hash; do not reuse an old hash or C3
   wrapper. Reconcile source and env before/after. Any mismatch stops the attempt.
3. Install only the new private controls with their pinned hashes. Start the
   independent recovery service and create the shared window once. Recover
   its actual active state and startup receipt before the single build dispatch.
4. The pre-start control records stop intent, requests normal SIGTERM and
   requires exit0. Check unchanged other containers and post-stop work/stream
   settlement before issuing the candidate-bound attestation.
5. Build one service at a time through the official builder. Between batches
   verify memory/swap, OOM, serving health and prior process termination.
   Recover every actual image identity, OCI revision, saved-byte/layout result,
   immutable manifest and terminal build outcome before image acceptance.
6. Recover ExecStopPost and independent restoration outcomes, exact original
   container health and post-restoration READ ONLY settlement. Preserve any
   failure; this authorization would not allow another interruption/retry.

No production reads, SQL, root installation, stop/start, image build, provider
request or deployment occurred in this preparation. The recorded sudo lease
expiry remains2026-10-20T17:00:57Z; it is not maintenance permission or proof
of current access. Only a new explicit Owner answer can authorize this packet.

After this package, continue actual approved-base host whole flow, G4
deployment, Owner use, three consecutive trading days and final C_R2. R2
remains incomplete. The same coordinator maintains bounded continuation;
native Ralph remains blocked and no additional worker is delegated.


Continuation handoff: same-agent heartbeat `cad91dfe` was confirmed active at
2026-10-18T03:08:43.219Z. The handoff was persisted before predecessor
`c4f4db00` deletion was confirmed. Renew by2026-10-18T14:00:00Z, before its
15:08:43.217Z expiry and47th tick. If the Owner has not answered, preserve the
single pending decision and perform only bounded no-change reconciliation.
