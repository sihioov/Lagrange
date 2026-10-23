# R2 approved-base host integration packet

R2-04/R2-09, Wave96. Candidate `2a165fe577e833529134627e980a129b9717ae6f`.
Status: **AUTHORIZED_ONCE_EXECUTED_FAILED_TERMINAL_PRODUCTION_UNCHANGED**.
Owner approved this packet on2026-10-18; its one attempt is consumed. The
prepared scope below is preserved as the historical authorization boundary.
[Actual failure/recovery and source correction](2026-10-18-r2-wholeflow-source-git-correction.md). The third image attempt is
[accepted and recovered](2026-10-18-r2-third-image-maintenance-attempt.md).
This next packet is one host-local integration attempt using those exact images.

The previous Owner approval covered one image-maintenance interruption and its
restoration. Its [scope](2026-10-18-r2-third-image-maintenance-packet.md#requested-single-attempt-and-service-impact)
does not authorize this new private DB/container execution. The latest Owner
continuation also limits host operations to that approved attempt. The new explicit decision
covered the fresh bounded admission and this integration run together;
the earlier image approval and root-access grant are not requested again.

## Concrete scope and bindings

The unchanged [driver](../../scripts/ops/verify-owner-beta-daily-wholeflow.py)
uses the real approved V3 historical input, adds two explicitly synthetic
sessions, and runs the real producer/admitter/worker/API against a private
database. It verifies publication, process restart, and file/database restoration.
Existing production services remain running, research remains stopped, and the
actual operational database and selected release are unchanged. No provider or
Auth0 request is part of this test. Synthetic sessions are not real market
observations or Owner acceptance.

| Binding | Prepared value |
| --- | --- |
| Candidate checkout | `/data/worktrees/3puw275b/r2-release-2a165fe`, detached, 2132 pinned paths |
| Accepted image manifest | `/etc/lagrange/release-manifests/2a165fe577e833529134627e980a129b9717ae6f.manifest` |
| Manifest SHA256 | `2712ec998a126d53a052e342a9e2c771b8f1a727655606112eb03086f8e2ce49` |
| Existing base selector | Container `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`, service `owner-beta-runner`, its single read-only bind at `/data/owner-beta-artifacts` |
| Base physical path | **UNKNOWN until the new authorized admission**; derive only from that fixed mount, require the `historical-price-beta-root` leaf and existing ownership/path rules; no guessed path or fallback |
| Private result directory | `/var/lib/lagrange-r2-wholeflow-evidence/2a165fe-a1-20261018`, new root-owned 0700 directory |
| Private result | `result.json` in that directory, exclusive creation, root-owned 0600 |
| Background unit | `lagrange-r2-wholeflow-2a165fe-a1.service`, new transient unit, one dispatch |
| PostgreSQL | Existing local `postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a`; no pull/build |
| Driver limits | 1200 seconds; file snapshot limit 2147483648 bytes |

The [command ledger](../../target/r2-daily-beta/wp-8/wholeflow-2a165fe-a1/command-ledger.json)
contains exact selectors, argv and the single typed base-path binding. Its SHA256
is `51bea6e93aecb2e1f209447d51a2e827f15fd798b673def1e61d13bf6132bac7`.
The [preparation receipt](../../target/r2-daily-beta/wp-8/wholeflow-2a165fe-a1/preparation.json)
is `6c4d9cafcf9b523174694866489f4235a8204309f4f743956ce3a9aae132bd04`.
Eleven critical source paths are individually pinned and the full 2132-path
source receipt is referenced. No new helper execution, protected read, test,
installation or lifecycle action occurred during preparation. This is coordinator
static preparation, not a new independent review or host acceptance.

## Historical authorized one-attempt sequence

1. Within a five-minute admission, compare the full clean source, accepted
   manifest/twelve image IDs and OCI revisions, local PostgreSQL digest, and
   the same eleven production container identities/states. Require at least
   4GiB available memory, 10GiB free below `/var/lib`, memory PSI some/full
   avg10 at most 1.0, no active build/compiler, and zero observed OOM events
   in the preceding readable 30-minute kernel range. Failure stops admission;
   it does not authorize freeing resources or stopping production services.
2. Inspect only the fixed base container identity and selected mount. Bind its
   exact physical source into the argv and persist that binding before launch.
   Require one read-only bind, safe metadata and the expected leaf. The unchanged
   driver snapshots actual base bytes, and the real producer and admitter each
   enforce the compiled V3 approval API. A different/missing mount fails closed.
   No production SQL is required for this admission.
3. Require absence of the new unit and output directory. Create only that
   private evidence directory, then dispatch the exact driver once in systemd
   with Nice10, idle I/O, CPU/IO weights20 and umask0077. The driver is bounded
   to 20 minutes; each cleanup Docker call is bounded to 30 seconds. The outer
   unit has RuntimeMaxSec50min, SIGINT, TimeoutStopSec5min and SendSIGKILL=no.
   Recover the full outcome by one hour from dispatch. If cleanup or termination
   remains uncertain, report INCOMPLETE and retain the stage/intents; no retry.
4. The existing driver creates only nonce-labelled private containers/networks
   and a root-private `/var/lib/lagrange-wholeflow-*` stage. Each container has
   1GiB memory, one CPU, 256 PIDs, a read-only root, dropped capabilities and
   no-new-privileges. Producer networking is `none`; DB/API/other processes
   use private internal networks, no published ports, no image pulls and no
   container log persistence. Database bootstrap/migration/SQL apply only to
   the new private database.
5. Recover the exact driver exit, finite phases and sanitized receipt. Require
   first/subsequent admissions and their parent/session/bar-count chain,
   successful recommendation and real HTTP publication, restart replay,
   capture/restore with all 62 migration checksums and preserved publication/
   poll identities, and unchanged original base bytes. Healthy/exit0 alone
   is insufficient.
6. Verify actual absence of the exact nonce-owned containers/networks, private
   stage disposition, unchanged production states/release and bounded OOM
   observations. Cleanup may remove only objects whose exact name and nonce
   match a recorded intent; uncertain or foreign resources are retained.
   Raw command output remains in memory; persist hashes and sanitized metadata.

No production deployment, migration, service interruption, research activation,
new provider access, live trading or image rebuild follows from this packet.
The background unit and the driver's cleanup survive a chat disconnect; neither
is claimed to protect against a host outage. A repeat requires a new disposition.

After this evidence is accepted, prepare the concrete G4 pre-upgrade consistent
recovery, backup-version transition, deployment and bounded catch-up packet.
Actual Owner use, three consecutive trading days and final `C_R2` remain open.
