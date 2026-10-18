# R2 candidate 512625a image and conditional integration packet

R2-04/R2-09, Wave102. **AUTHORIZED_IMAGE_ACTIVE; CONDITIONAL_WHOLEFLOW_NOT_YET_DISPATCHED.**
Owner “이어서 진행해” approved both ordered phases once; the sixth image attempt
was dispatched once13:01:51UTC. No repeated decision between these phases.
The preparation statements below are historical and remain preserved.

### Historical preparation before this authorization
Candidate `512625a31fdb59d284cbd13fa6f2334a4737cc13` is hosted-accepted only.
The preceding Wave100 approval covered0120b0e and both permitted phases were
consumed. A new decision is required for this candidate and interruption.
Root access is not execution approval. No root/production reads, project/control
execution, new independent review, build or integration occurred in preparation.

## One proposed decision and Owner impact

Approve one sixth image-maintenance attempt and, only after complete image
acceptance and restoration, one third actual isolated wholeflow dispatch.
There is one production interruption: the existing Owner V2 container normally
stops once, pausing its updates and quote processing during image preparation.
Independent recovery must be active before the stop; the same container/image
must return healthy immediately after build termination or failure. Nine other
serving containers remain healthy; research stays stopped with exit2.

The shared image window is at most12 hours, a deadline rather than an estimate.
The isolated integration follows only after all12 images, manifest, same-ID
restoration, successful independent recovery terminal state and read-only work
settlement pass. Its driver limit is20 minutes and complete recovery reporting
is due within60 minutes. Both ordered phases share this one decision; approval
of this packet requires no further question between phases. No automatic retry.

**Retain the previous anonymous volume unchanged.** Volume
`159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`
is not proven to belong to the deleted bootstrap container. Its creation time
aligns, but the deleted Mounts association was not retained. This packet permits
only its exact metadata/reference checks before and after the new integration;
no mounting, content reads, deletion or prune. Unexpected metadata/references
stop admission. Prior cleanup remains INCOMPLETE; successful new cleanup would
not rewrite that history. New unexpected persistent volumes retain their
associated container/intent evidence for disposition instead of erasing it.

Deployment, production migration, research activation, provider/Auth0 access,
trading, G4 backup transition and final Owner acceptance remain outside this
packet. No host chmod/chgrp, permission-policy change or guard weakening.

## Verified inputs

[Hosted evidence](2026-10-18-r2-wholeflow-inherited-volume-ci.md):
CI39117542904 attempt1 SUCCESS10:24:43UTC and functional smoke39117544844
attempt1 SUCCESS10:55:08UTC. All9 full log hashes/8 actual checkout SHAs,
1252 input pins/525 committed blobs,141 required names,15 image outcomes,
32 wholeflow cases and carried Rust/strict Clippy/format/PG/Web groups matched.
Actual publication/repeat and10 cleanup outcomes0/owned-resource absence passed.
The five actual Docker cases create/inspect private hosted fixtures; they do
not establish approved-base integration. No rerun/cancel/duplicate.

The detached checkout `/data/worktrees/3puw275b/r2-release-512625a` has2144
tracked file/symlink pins matching the exact commit. Previous0120's2140 pins
remain unchanged. The inert image env is0600, byte-identical to its predecessor.
Eleven image/recovery controls have only finite candidate/path/attempt/hash
rebindings: inverse bytes, exact expanded argv, unit properties and three
unchanged runtime/probeSQL files were checked. No control was run or installed.

Wholeflow admission preserves candidate/manifest/12-image/restoration guards,
fixed base selection, descriptor-relative ancestor identity/no-follow/no-ACL
checks and unchanged data-root/content/service-parent/PG boundaries. It adds
only exact retained-volume metadata/reference comparison. Static review caught
an absent-label representation issue; the final code normalizes labels in RAM
before emitting the same finite metadata shape. Initial unexecuted drafts remain
preserved. No local parser/test execution or independent review is claimed.

All receipt paths below are relative to `target/r2-daily-beta/`.

| Receipt | SHA-256 |
| --- | --- |
| `ci/512625a/hosted-terminal-evidence.json` | `4d99e5a7b07e20049a26afff264d0f20c9ee494426c1cf51cc1a79a9f2c06170` |
| `image-preparation-512625a/preparation.json` | `e22596b71d98be00861fc74e89dc05e65f51c91c10dd1da64575b69e8ed56d45` |
| `wp-8/maintenance-512625a-a6/preparation.json` | `6929b8d381c222f83085345cc08395144dbe96788eb0d14f29b609ef431c186d` |
| `wp-8/maintenance-512625a-a6/command-ledger.json` | `2c4f6eeb0028f6c1b88534cd172314a01845816d5e72a43eb41eee832157d751` |
| `wp-8/wholeflow-512625a-a4/preparation.json` | `337e990db42548d1435d390e91397b3cbfdc2730a99e52fecb1e9283eabacbf3` |
| `wp-8/wholeflow-512625a-a4/command-ledger.json` | `a78bead08771a87f911c28cf43ea5226c95c2de21e5bc0c3bbd429da112c1e08` |
| `wp-8/wave102/retained-volume-disposition.json` | `4f3bc67c587c099ed879e6072823f7b0bd2566bd88fb2acc333230efc2c770fa` |
| `wp-8/wave102/prepared-sequence.json` | `35c7efae240cb3c171077338d8e55cb2da83dbebcfc1b90bb362407436261bf0` |
| `wp-8/wave102/final-static-reconciliation.json` | `74fb4b872964702e3c12fcd0068ba5f7a2b7b87cd166d075534b257f6b6c3a7a` |

Final static reconciliation matched2144 source pins,11 controls/inverse bytes,
11 critical wholeflow pins, seven sequence references,1252 frozen hosted pins
and the inert env. Current host health, source-input hash, protected output/unit
absence, candidate image IDs and manifest remain UNKNOWN until approved fresh
admission. Historical images or attestations cannot substitute for them.

## Ordered execution after approval

1. Reconcile exact scope/time, all source/control/env pins and clean detached
   checkout. Perform fresh bounded host/no-clobber and READ ONLY work admission;
   observe official plan/preflight/source-input hash afresh. Historical timer
   waiting states alone are not active execution; use exact PIDs/cgroup/process
   evidence without stopping/resetting old units. Install only the new controls.
2. Create a fresh shared12h window before launching independent recovery. Verify
   it actually active before one stop intent/normal SIGTERM. Require Owner exit0,
   no forced-kill fallback, settled work and valid reader attestation.
3. Run the official builder in background systemd, Nice10/idle I/O/CPU+IO weights20,
   `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`. Four planned batches of3;
   each Compose invocation names exactly one service. All memory/swap/OOM/health
   and prior compiler termination gates remain required. Recover12 exact image
   IDs/OCI revisions, saved bytes/layout, source bindings and immutable manifest.
4. Recover actual successful build/ExecStopPost and independent recovery terminal
   outcomes plus zero processes. Verify same Owner ID/image healthy, exact11
   production states and post-restore read-only work settlement. Unit absence
   alone is not terminal proof. Any failure closes this attempt without retry.
5. Only then run fresh five-minute wholeflow admission binding the new accepted
   manifest/12 images/2144 pins/11 critical pins/local PostgreSQL digest, fixed
   base and retained-volume metadata. Bind exact argv and dispatch once. Recover
   actual admissions/publication/HTTP/restart/capture-restore/62 checksums,
   unchanged base, complete new-owned-resource cleanup and11 production states/
   unchanged selected release. Deadline or cleanup uncertainty is INCOMPLETE.

New rootaux `/etc/lagrange/release-manifests/.r2-maintenance-512625a-a6-20261018`;
units `lagrange-r2-build-512625a-a6.service` and
`lagrange-r2-recovery-512625a-a6.service`. Manifest:
`/etc/lagrange/release-manifests/512625a31fdb59d284cbd13fa6f2334a4737cc13.manifest`.
Owner ID `f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`,
image `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
Stop only in the first hour; build RuntimeMaxSec10h, watchdog10.5h,
attestation expiry11h. No renewal. Wholeflow dispatch by image-window hour12,
full recovery within60min and by hour13.

Wholeflow unit `lagrange-r2-wholeflow-512625a-a4.service`, root result
`/var/lib/lagrange-r2-wholeflow-evidence/512625a-a4-20261018/result.json`.
a4 is the fourth prepared packet and would be the third actual dispatch;
899a2 never dispatched. Base resolves only from fixed owner-beta-runner
`1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`'s single
read-only `/data/owner-beta-artifacts` mount. No historical-path fallback.
Driver1200sec/2GiB snapshot; private nonce DB/network/stages only; no pull or
production SQL. Outer RuntimeMaxSec50min, SIGINT,5min stop, SendSIGKILL=no.
Exact DB-client tmpfs covers inherited PostgreSQL volume; pre-create coverage,
post-create mounts and unexpected-volume evidence preservation remain enforced.

## Preserved history and remaining R2 work

0120 images/restoration remain accepted: closureff247181 and manifest57750837.
Its wholeflow closure861396af remains failed/cleanup incomplete; bootstrap was
never started and no later integration phases passed. Preserve failed unit/root
result, old899/2a image sets, unlaunched899 admission, first root-Git failure and
both ab48 image failures/controls/unaccepted image. Never reset/relaunch old units.

The observed root lease ends2026-10-20T17:00:57Z; it grants access only. Native
Ralph stays blocked; same-agent heartbeat is bounded continuation, not a worker
or daemon-outage protection. Keep one decision pending, with no repeated
production probes, documentation commits or permission questions absent new facts.
After this packet, continue G4 consistent pre-upgrade recovery/backup transition,
deployment, Owner use, three consecutive trading days and finalC_R2. These remain
open and are not authorized by this image-plus-isolated-integration packet.

## Bounded continuation

Successor33c0cd33 confirmed11:10:07.401UTC; decision handoff
`wp-8/wave102/decision-handoff.json` SHA256
`718e3b6ba9e78b742fc3dd9ed8dfe76d6b814e6c1b101c6d56a8ea2a8fdaa492`
was persisted before predecessor44965a67 deletion was confirmed. Renew before
22:00UTC/47th tick; expires23:10:07.395UTC. This keeps overallR2 tracked while
the new packet decision is pending; it grants no host execution permission.

## Wave102 approved execution — 2026-10-18 13:07UTC

Owner “이어서 진행해” accepted the concrete two-phase packet. Authorization
`wp-8/wave102/owner-authorization.json` SHA9c10f6655a5e687146d5b82eac0ff01efb7699fb6aa1d91c1077ca549cbd0767
covers one sixth image maintenance and, only after full image/restoration closure,
one third actual isolated wholeflow. No repeat question or automatic retry.

Fresh2144 source/11 control/11 critical pins and inert env matched. Root host
admission, exact11 identities/states, bounded30min OOM0, READ ONLY work settlement
and official12-service plan/preflight passed. Source-input454cad1f was freshly
observed. Two pre-host coordinator preparation errors were corrected and retained:
per-file inverse substitutions avoid a baseline-path collision; sourcePins2144 is
a count whose full entries reside in the separately hashed receipt. Neither error
performed host actions or changed accepted source/controls.

Independent recovery was actually active before the single stop/build dispatch
13:01:51.009414UTC; systemd-run returned0 at13:01:52.202197. Owner V2 exited0 and
work settled. At13:07UTC build/recovery were active; DB bootstrap, DB migration,
API and Web had success markers; research-worker was next. These markers do not
accept the full image set. Shared image window13:01:25.358744UTC through
2026-10-19T01:01:25.358744Z; watchdog23:31:25, attestation00:01:25, no renewal.
Wholeflow remains undispatched, conditional on full image/recovery closure and
fresh admission; dispatch before01:01:25/full recovery within60min/by02:01:25.

Confirmed successor heartbeatcad36e21 at13:04:02.842UTC, persisted handoff
c086fcdedfbf456d28e0fc24fd893ef2403961f8d76a21441a14bc6b4b4928f9, then confirmed
33c0cd33 deletion. Renew by23:30UTC/before47thtick; expires01:04:02.837UTC Oct19.
Same-agent continuation is not a worker or daemon-outage protection. Independent
root recovery protects this maintenance attempt. Retained anonymous volume and
all prior failed/accepted evidence remain unchanged. G4/deployment/Owner use/three
trading days/finalC_R2 remain open and outside this image+isolated integration scope.
