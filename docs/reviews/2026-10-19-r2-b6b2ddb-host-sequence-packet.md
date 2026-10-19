# R2 b6b2ddb image and conditional integration packet

R2-04/R2-09, Wave106. **PREPARED_NOT_AUTHORIZED_NOT_EXECUTED.**
Candidate `b6b2ddbd7a5d63f482f1a0fa43a02cb704a70e93` has
[complete hosted acceptance](2026-10-19-r2-b6b2ddb-hosted-acceptance.md).
One proposed new Owner decision covers an eighth image-maintenance attempt once,
then only after complete successful image/restoration closure, a fifth actual
isolated wholeflow dispatch once. This is the sixth prepared wholeflow packet;
899a2 was never dispatched. No second question between approved phases, renewal
or automatic retry.

Wave104's “승인하니 이어서 진행해” approval authorized the a7825fa image attempt
and its conditional wholeflow, both now consumed. Its image closure succeeded;
the wholeflow failed at producer INVALID_ARTIFACT and closed with its observed
owned resources absent. This new candidate introduces private role copies and
needs its own image set and one new Owner V2 interruption. The standing task
requires a new explicit decision after this concrete packet. Root access alone
is not execution approval.

## Impact and new effect

The existing Owner V2 container stops normally once, pausing its updates and
quote processing during image preparation. Independent recovery must actually
be active before that stop. Restore the same container and image healthy as
soon as the build terminates or fails. Nine other serving containers must stay
healthy; research remains stopped with exit2. The shared12-hour window is a
maximum bound, not a duration estimate. The subsequent isolated driver has20
minutes, with terminal and cleanup recovery within60 minutes.

Source inspection established that daily producer UID10002 and admitter UID10003
cannot satisfy the V3 reader's current-UID ownership contract on the observed
original UID10001 root. The new helper creates two byte-identical copies of only
the fixed approved V3 leaf under fresh private daily state. Each role root is
owned by its role UID/GID with0750; its candidate leaf0700 and files0600. Each role
gets its own readonly mount. Original source bytes/permissions, worker/API base
and Rust approval checks remain unchanged. This is not proof of the exact prior
runtime rejection; that predicate and original512 inner cause remain UNKNOWN.

Before copying, fresh admission must inspect the original fixed candidate leaf
`b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d`:
UID/GID10001:10001, leaf0700, exactly manifest.json and bars.ndjson as regular
single-link files0600, no ACL/symlink, stable descriptor/name metadata; bars size
7672914 and manifest at most2MiB. These leaf/file metadata are currently UNKNOWN.
Mismatch blocks execution; this packet does not repair source permissions.

The accepted driver/helper then reads and verifies the fixed canonical unsigned
manifest pin `53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6`
and bars pin `23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe`.
The manifest pin is not a whole-file hash. Complete pair publication is exclusive
and atomic; partial/different/uncertain stages block reuse and remain retained.
Copies are created before private DB preparation, verified before enable and
included in the existing daily backup class. Restored metadata/bytes and immutable
copy comparisons are required alongside original-base preservation. Existing
release-lock/stopped-cohort requirements continue to govern production operations;
this packet invokes only the disposable isolated driver.

Retain volume `159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`.
Ownership is unproven; original0120 deleted Mounts association is missing. Only
approved exact pre/post metadata and reference comparisons are allowed. No mount,
content read, deletion or prune. Drift blocks admission. Prior0120 cleanup stays
INCOMPLETE regardless of new cleanup. Unexpected new volumes retain associated
containers and intent evidence.

G4 backup/version transition, deployment/migration, research activation, provider
or Auth0 access, trading, Owner use and final acceptance are outside this packet.
Failed historical units, results, accepted source/images/manifest and evidence
must not be reset, relaunched, substituted or overwritten.

## Prepared inputs and evidence

CI39178523729 attempt1 SUCCESS00:20:13UTC and functional39178525169 attempt1
SUCCESS00:49:53UTC were fully recovered00:52UTC:9 logs/8 actual checkouts,
1370 pins/534 blobs,170 required definitions,15 image outcomes/44 wholeflow cases,
all carried Rust/PG/Web groups and actual publication/repeat/10 cleanup0/owned
resource absence. New synthetic regressions do not establish real fixed V3 host
acceptance. All prior exclusions, skips and unprinted nested UNKNOWN are preserved.

Clean detached `/data/worktrees/3puw275b/r2-release-b6b2ddb` has2152 tracked file/
symlink pins matching its commit. Prior a7825fa checkout's2147 pins were checked
against its own commit and preserved. Inert image env remains byte-identical0600.
Eleven image controls have finite candidate/path/unit/date/dependency-hash
rebindings with inverse bytes, expanded argv and unit template matches. Three
runtime/work probeSQL files are byte-identical. Wholeflow has12 critical source
pins, adding the role-copy helper. Admission adds only the exact metadata gate
above to finite rebindings; removing that block and reversing rebindings matches
the prior bytes. Observer changes are finite rebindings only; duplicate-key and
command_failures finite diagnostic validation remain unchanged.

All controls are STATIC ONLY: no local parser/test/control/helper execution,
root/production reads, installation or new independent review. Current host
health, source-input hash, output/unit absence, new12 image IDs and manifest are
UNKNOWN until approved fresh checks. No historical attestation is reusable.

| Receipt under `target/r2-daily-beta/` | SHA-256 |
| --- | --- |
| `ci/b6b2ddb/hosted-terminal-evidence.json` | `4f4f8767b151db98bd1ea9d34ac7c7e4eb21fb347f15c03fd245b782e4ac1d2d` |
| `image-preparation-b6b2ddb/preparation.json` | `8264ed9982325bd2ad32b09f57544e3ef544fcca35ea87e2eb0462d5db85d254` |
| `wp-8/maintenance-b6b2ddb-a8/preparation.json` | `f7d5024d4615b13841a55e4d4a0f20b0a65adcdde8efd2d603b1cdb1af5026da` |
| `wp-8/maintenance-b6b2ddb-a8/command-ledger.json` | `9013e44bb1e7765e4a8d7dd68c1ff60320b72a284ffa63fc1eaf1d96cb0c516f` |
| `wp-8/wholeflow-b6b2ddb-a6/preparation.json` | `7e8faaddeac1e6de92fe7dd5bdeeaf8622476a3fb3322ace411a95321e18110e` |
| `wp-8/wholeflow-b6b2ddb-a6/command-ledger.json` | `b591e30ff9b683471bc705bcc7b5b11eeb71f92278a429ac6ac5f6a3f5624305` |
| `wp-8/wave106/retained-volume-disposition.json` | `32c2346e7f28e1263569799635092050e8520c0588954e4b22ea9dceb1f1325c` |
| `wp-8/wave106/prepared-sequence.json` | `ca11cb5dface37bb689d7152c09340e0a2cb83b15353f2232eb18d1cddd21fec` |
| `wp-8/wave106/final-static-reconciliation.json` | `e95f108af308d964b1029978ab0aba4fd8af97aa2b69cd692ac01fd58f2e7f1c` |

## Ordered execution after approval

1. Bind the new decision and time. Recheck all2152 source pins,11 control files,
   inert env and clean detached checkout; fresh bounded host/no-clobber/READ ONLY
   work checks and official plan/preflight establish the actual source-input hash.
   Never reuse old attestations or C3 wrappers. Install only new controls.
2. Establish a fresh shared12h window and verify independent recovery actually
   active before one stop intent and normal SIGTERM. Owner exit0 is mandatory;
   no forced-kill fallback. Require settled work and valid attestation.
3. Run the official builder through background systemd Nice10/idle IO/weights20,
   COMPOSE_PARALLEL_LIMIT1/CARGO_BUILD_JOBS2. Four batches of3; exactly one service
   per Compose invocation. All health/memory/swap/OOM/termination gates remain.
   Recover12 exact image IDs/OCI revisions, saved bytes/layout and immutable manifest.
4. Recover successful actual build and independent recovery terminal evidence,
   no PID/cgroup processes, same Owner ID/image healthy, exact11 production states
   and postrestore READ ONLY settlement. Missing units alone are insufficient.
   Failure or uncertainty closes the attempt without retry or wholeflow dispatch.
5. Only then fresh five-minute wholeflow admission binds new accepted manifest,
   12 images/2152 source pins/12 critical pins/local PG digest/fixed base including
   new leaf/file metadata/resource gates/11 states/retained volume. Base resolution
   is solely fixed owner-beta-runner `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`,
   its single readonly `/data/owner-beta-artifacts` bind; no path/image fallback.
   Bind ledger argv and dispatch once: driver1200sec/2GiB/private nonce DB/network,
   no provider/Auth0/production SQL; outer50min/SIGINT/stop5min/SendSIGKILLno.
6. Recover actual first/subsequent admissions, publication/HTTP, restart identity,
   capture/restore62 checksums, restored role copies and original base unchanged,
   exact new-owned cleanup, retained volume/11 states/current release unchanged,
   terminal status and no processes. Preserve finite command/diagnostic hashes
   on failure; raw logs stay in RAM. Unobserved phases remain UNKNOWN; deadline
   or cleanup uncertainty is INCOMPLETE.

Rootaux `/etc/lagrange/release-manifests/.r2-maintenance-b6b2ddb-a8-20261019`;
units `lagrange-r2-build-b6b2ddb-a8.service` and
`lagrange-r2-recovery-b6b2ddb-a8.service`. Owner ID
`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`, image
`sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
Stop only in first hour; build10h/watchdog10.5h/attestation11h; no renewal.
Wholeflow dispatch before image hour12, full recovery within60min/by hour13.
Wholeflow unit `lagrange-r2-wholeflow-b6b2ddb-a6.service`; result
`/var/lib/lagrange-r2-wholeflow-evidence/b6b2ddb-a6-20261019/result.json`.

Preserve a7825fa image closure1048f785/manifest6e27840a and failed wholeflow
closure889ad46a, prior512 closures6566857a/4d3770a7 and all0120/899/2a/ab48 evidence.
Root lease observed through2026-10-20T17:00:57Z is access only. No policy probe loop.

Until the single new decision: bounded static no-change reconciliation only,
no repetitive documentation commits/questions or production probes. Overall R2
remains open through G4/deployment/Owner use/three consecutive actual trading days/
final C_R2. Native Ralph is blocked with no resume API; the same-agent heartbeat
provides bounded continuation, not worker or outage protection. Confirm successor,
persist handoff, then delete predecessor; renew before12h expiry/tick47.

Tracking: successorcb45b5b6 confirmed2026-10-19T00:57:46.997Z; handoff7fb0ad62
persisted before successful f208d6f4 deletion. Renew by11:45UTC/beforetick47;
expires12:57:46.992UTC. One new decision is required; no host action authorized.
