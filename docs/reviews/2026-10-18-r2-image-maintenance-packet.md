# R2 ab48a79: host access restored and proposed image-maintenance attempt

R2-04/09, Wave91 historical preparation. The Owner subsequently approved this
one-attempt scope. Wave92 executed it once; the normal-stop check failed on
container exit1, and the same container was restored. The image builder never
started. See the [attempt and recovery record](2026-10-18-r2-image-maintenance-attempt.md).
The preparation text below is preserved as the exact approved scope. Candidate
`ab48a7953dd07fc479407cf80a58f37e2c0c6a83` remains accepted for hosted CI and
functional smoke only. No product source, installed release, application
container, database or provider state was changed by this reconciliation.

## Fresh access and bounded observation

The Owner renewed the reviewed temporary sudo grant directly. At
2026-10-17T17:02:48.649962Z (2026-10-18 02:02:48 KST), the agent's exact
`sudo -n /usr/bin/id -u` returned exit0/UID0. The fixed-field read-only packet
completed in0.31seconds, within its five-minute bound and15-second command
timeouts. The earlier13:46 and16:56 failures remain preserved.

The renewed timer was enabled/active/waiting, Persistent=yes. Its scheduled
expiry is **2026-10-20 17:00:57 UTC / 2026-10-21 02:00:57 KST**. This is a
timer observation, not proof of future revocation or an extension of the
maintenance/deployment scope. The account-wide grant was applied by the Owner.

| Observation | Result and limit |
| --- | --- |
| Selected release | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c`; R2 is not deployed |
| Project inventory | Eleven containers: ten running/healthy, restart0, OOM=false; research-worker exited2/unhealthy, restart7935, OOM=false |
| Owner V2 | ID `f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`, image `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`; running/healthy |
| Protected metadata | Manifest parent0755, build-state parent0700, releases/bin0755, all root:root directories; candidate manifest absent |
| Units | Docker/containerd active/running; backup and verification services inactive with recorded success, their timers waiting; candidate build unit not found |
| Resources | Available memory7,694,209,024bytes; used swap4,604,776,448bytes; no listed build/compiler processes |
| Recent OOM query | Last30minutes: zero matching rows, exit1, no stderr. This is the grep/no-match outcome, not a full journal-history or future build guarantee |
| Remaining unknowns | Current jobs/leases, protected operational configuration/base binding, candidate-specific build state/control-directory absence, current source-input hash, drain and recovery outcome |

Private receipt:
`target/r2-daily-beta/wp-7/wave90/host-admission-after-lease-renewal.json`, SHA256
`d8e024e5fccf8210609649cafdd1c16de81caa9909d44ae160004a1da430184a`.
Only fixed metadata, numeric fields and OOM count/presence were persisted.
No project helper, SQL, build, service lifecycle or provider request ran.

## Proposed one-attempt scope

The ordinary eleven-healthy-container build predicate is not satisfied.
The existing [drained-reader contract](../runbooks/production-release-and-backup.md)
allows this attempt only after fresh work settlement and normal Owner V2
shutdown. The old C3 permission and attestation are consumed historical evidence.

The requested new scope is one image-preparation attempt for this exact R2
candidate: bounded read-only runtime/DB aggregate checks; candidate-private
control installation; normal SIGTERM of only the existing Owner V2 container;
official sequential image building; restoration of that same container on
completion or abort; and read-only post-restoration settlement checks.
Research-worker stays stopped. The other nine serving containers stay running.
Existing release selection, runtime configuration and production schema stay
unchanged. Owner V2 daily/signal updates and any active quote collection pause
during the interruption; resumption can restart its existing collection and
database recovery. No new provider surface is authorized by this request.

Proposed timing: begin after Owner approval and fresh checks, with one shared
12-hour window recorded before the independent recovery unit starts. Shutdown
may only begin in its first hour. The build unit has10hours RuntimeMaxSec;
the independent supervisor requests build termination at window hour10.5,
reserving recovery time. The attestation expires at window hour11 and is never
renewed within the attempt. Normal completion restores immediately. This is
a bounded recovery procedure, not a guarantee that host/daemon failure can
never delay restoration; an unmet deadline is explicitly INCOMPLETE.

## Concrete prepared bindings and commands

| Item | Exact binding |
| --- | --- |
| Clean image source | `/data/worktrees/3puw275b/r2-release-ab48a79` |
| Inert env | `/data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/image-preparation-ab48a79/image-only.env` |
| Private preparation | `target/r2-daily-beta/wp-8/maintenance-ab48a79/` |
| New root control directory | `/etc/lagrange/release-manifests/.r2-maintenance-ab48a79-20261018` |
| Build/recovery units | `lagrange-r2-build-ab48a79.service` / `lagrange-r2-recovery-ab48a79.service` |
| New manifest | `/etc/lagrange/release-manifests/ab48a7953dd07fc479407cf80a58f37e2c0c6a83.manifest` |

The private [command ledger](../../target/r2-daily-beta/wp-8/maintenance-ab48a79/command-ledger.json)
contains exact argv, effects, bindings and halt conditions. Its SHA256 is
`7ad7ecfae9aa74384e9561df12acff2d5dd8d5a6e7b534deda3077f128fd38f6`.
The ordered sequence is:

1. Recheck frozen files, clean detached candidate, inert env, all current host
   predicates and absence of candidate outputs/units. Run the bounded existing
   runtime/work-state probe and official candidate `--plan` / `--preflight`.
   Record the actual R2 source-input hash; never substitute the old C3 hash.
2. Require read-only/operator-wide aggregate visibility and settled work.
   Inspect active leases and subscriptions before any shutdown. If identity,
   mode, data or safety preconditions differ, stop before lifecycle actions.
3. `sudo -n /usr/bin/python3 <private-preparation>/install-control.py` creates
   only the new0700 root directory and hash-verified private controls.
4. `sudo -n /usr/bin/python3 <private-preparation>/start-recovery.py` creates
   the shared window and launches the independent recovery unit. Verify its
   actual active state and startup receipt before arming shutdown.
5. Execute the fully expanded `systemd-run` argv in the ledger. Pre-start
   records stop intent before `docker stop --signal SIGTERM --timeout -1`
   against the exact Owner V2 ID. Normal exit0, unchanged other containers and
   post-stop work settlement are required before creating the fresh attestation.
6. The official candidate builder produces twelve images in four batches of
   three, **one Compose service per invocation**, Cargo jobs2/Compose parallel1,
   Nice10/idle I/O/CPU and I/O weights20. Resource, health, OOM and prior process
   termination gates remain enabled. Recover actual image/manifest/saved-byte
   evidence and terminal outcomes.
7. ExecStopPost and the independent supervisor retain same-ID restoration
   responsibility. Do not resume while compilation remains active. Verify
   restored identity/health and repeat read-only work settlement. Preserve
   failures and report incomplete recovery instead of inventing success.

## Preparation verification and boundaries

Eleven draft files were prepared from the final reviewed R1 controls. Candidate,
paths, unit names, labels and dependent hash pins are the only substitutions;
inverse substitutions recover every baseline byte. The three runtime/probe/SQL
files are byte-identical. Maintenance/recovery baselines match
`5d3e83b550c8a53bb5b8817e895359cd5d221457f8c987453beed5f920a11c79` and
`12b474172d6c19a5e284aed23225ad548bf51fe935dde313ff70d3e63e304840`.
The old installer pinned a superseded initial maintenance version; the proposed
installer instead pins the rebound final control. The C3-only prelaunch wrapper
and raw-output-writing launcher were not copied. Explicit current checks and
sanitized in-memory command recovery replace those invocation conveniences.

Preparation receipt SHA256:
`f2a9f3b0ef1eeaaf823229c797cb31a99e7cfbb64d16df90fa2d1dc1d4e24839`.
The291-line mechanical diff and all file pins are preserved privately. This
turn used source/static inspection only; historical24/16 control regressions
were not rerun, and no new independent review or executable acceptance is claimed.

No root control installation, SQL query, stop/start, build or deployment has
occurred. The [G3/G4 rules](../superpowers/plans/2026-10-13-r2-daily-research-beta.md#L454)
require a fresh decision for this service interruption. Approval of this packet
would cover only the described image-maintenance attempt. Actual approved-base
host flow, G4 deployment/migration, Owner use, three consecutive trading days
and final C_R2 remain separate open gates.


At17:10UTC the coordinator rechecked all2125 candidate checkout pins, clean
detached HEAD, inert env hash/mode and eleven prepared control bindings. Static
receipt SHA256:4209309b08cb2fd47a87b5efb99e63f55bea2ff7cca06803c1a9d50252b3a838.
Successor heartbeatc105aa23 was created17:09:06UTC and persisted before the old
b9bb7ac0 deletion was confirmed. Renewal is due2026-10-18T04:30UTC, before
05:09:06UTC expiry. It awaits the new decision and tracks the remaining R2
dependencies; native Ralph remains blocked and no extra worker was created.
