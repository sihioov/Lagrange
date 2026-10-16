# R2 fresh host admission packet

Prepared 2026-10-16 for R2-04/R2-09 and G3. **Unprivileged observations only;
full host admission is incomplete.** The source candidate is
`f215665012d1f4f1b81289dd94606bceb0cb8c46`; its exact hosted verification is
[CI38901117928](https://github.com/sihioov/Lagrange/actions/runs/38901117928).
Full CI recovery completed successfully at12:55:45UTC; see the
[scoped disposition](2026-10-16-r2-preupgrade-ownership-ci.md). Independent
integrated review remains a prerequisite for accepting the release. This
packet prepares the missing host observations, not deployment approval.

## Prepared inputs

- Clean detached source: `/data/worktrees/3puw275b/r2-release-f215665`.
  All 2,115 tracked files were inventoried without invoking project code.
- Build-only interpolation file:
  `/data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/image-preparation-f215665/image-only.env`.
  Its private parent is mode 0700 and the file is mode 0600. It contains only
  an inactive zero-hash sentinel; no operational environment or credential was copied.
- Intended future manifest:
  `/etc/lagrange/release-manifests/f215665012d1f4f1b81289dd94606bceb0cb8c46.manifest`.
- Intended future build unit: `lagrange-r2-build-f215665.service`.
- Preparation receipt and source hashes:
  `target/r2-daily-beta/image-preparation-f215665/preparation.json`.

No official plan/preflight, Docker command, compiler, build, service action,
database operation or provider request was executed to prepare these inputs.
The intended protected output's absence has not yet been observed.

At13:01:28UTC the unprivileged read-only subset observed current still selecting
`fce9b90542cf9b1a911d46e4c4d3530db7efba4c`,6,265,417,728 available memory bytes,
1,499,156,480 used swap bytes and no listed compiler/build processes. Docker
and containerd units were active; both backup timers were waiting and their
services inactive with recorded success. The intended build unit was not found.
These are finite point-in-time observations, not container-health or root
admission. They do not show the R2 candidate deployed. The private receipt is
`target/r2-daily-beta/host-admission-f215665/unprivileged-observation.json`.
Root access, protected parents/output, container health and kernel OOM evidence
remain unqueried; no sudo, Docker command or service mutation was performed.
Repeat the unprivileged observations in the actual admission window; this
earlier receipt cannot establish readiness for a later build.

## Bounded read-only observation

The observation window is at most five minutes. Each command is bounded to
15 seconds; stop on access denial or ambiguous ownership. Do not retry by
changing permissions, renewing sudo policy, starting services or substituting
a different release. Save only the fields below in a private receipt. Raw
journal output, container environments and configuration contents are excluded.

| Query | Exact scope and retained fields |
| --- | --- |
| Root access | `sudo -n /usr/bin/id -u`; retain exit code and numeric UID only. No password prompt, credential-cache invalidation or policy mutation. |
| Selected release | `/usr/bin/readlink --canonicalize-existing /opt/lagrange/current`; retain the selected immutable path for subsequent source/manifest binding. |
| Protected parents and output | Use the exact stat invocation below. Missing paths are recorded, never created during observation. |
| Control units | `/usr/bin/systemctl show` for `docker.service`, `containerd.service`, `lagrange-production-backup.service`, `lagrange-production-backup.timer`, `lagrange-production-backup-verify.service`, `lagrange-production-backup-verify.timer`, and the exact intended build unit. Properties only: `LoadState,ActiveState,SubState,MainPID,ExecMainStatus,Result,NRestarts`. |
| Owned production containers | `sudo -n /usr/bin/docker --host unix:///var/run/docker.sock ps --all --no-trunc --filter label=com.docker.compose.project=lagrange-station --format '{{.ID}}'`; accept only64-hex IDs. Inspect each returned ID with the fixed format below. |
| Resources | `/usr/bin/free --bytes` and `/usr/bin/df -B1 --output=avail,target /data /var /etc`; retain numeric memory/swap/free-space measurements. |
| Concurrent build processes | `/usr/bin/ps -eo pid=,ppid=,comm=`; retain only `cargo`, `rustc`, `rustup`, `docker-buildx`, `buildkitd` and `cc1` process names/IDs. No command arguments or environment. |
| Recent OOM evidence | Use the bounded kernel query below; consume in memory and retain presence/count only. Missing journal access is UNKNOWN. |

```sh
sudo -n /usr/bin/stat -c '%F|%u|%g|%a|%n' \
  /etc/lagrange/release-manifests \
  /etc/lagrange/release-manifests/.lagrange-build-state \
  /opt/lagrange/releases /opt/lagrange/bin \
  /etc/lagrange/release-manifests/f215665012d1f4f1b81289dd94606bceb0cb8c46.manifest
sudo -n /usr/bin/journalctl -k --since '-30 minutes' --no-pager \
  --grep 'Out of memory|Killed process|oom-kill'
```

The only permitted container inspection format for this observation is:

```sh
sudo -n /usr/bin/docker --host unix:///var/run/docker.sock inspect --format \
  '[{{json .Id}},{{json .Image}},{{json .State.Status}},{{json .State.Running}},{{json .State.ExitCode}},{{json .State.OOMKilled}},{{json .RestartCount}},{{if .State.Health}}{{json .State.Health.Status}}{{else}}null{{end}},{{json (index .Config.Labels "com.docker.compose.project")}},{{json (index .Config.Labels "com.docker.compose.service")}}]' \
  RETURNED_VALIDATED_64_HEX_ID
```

The last argument is bound only to an ID returned by the scoped query, never
an arbitrary container. Retain the exact ID/image, finite state and numeric
fields, health status and the two named Compose labels. Do not inspect full
labels, command arguments, environment, mounts or logs.

The fixed queries do not query application rows, secrets, Raw/Curated/base
contents or provider endpoints. They do not stop writers, acquire maintenance
locks, install files, change Docker resources or activate a timer. The result
is an admission input, not permission to perform those operations.

## Decisions after observation

The [adopted G3/G4 rules](../superpowers/plans/2026-10-13-r2-daily-research-beta.md#L454)
require fresh health/concurrency/root/no-clobber evidence and a concrete command
ledger. The historical 72-hour sudo arrangement ended at its recorded expiry;
it is not current access evidence. Existing KIS read-only/EOD and Owner V2
approvals are preserved. An unhealthy service or expired maintenance exception
must not be repaired or revived under this observation scope.

After the observations and independent review are accepted, bind the official
image builder to this clean source, current health cohort and new manifest.
Use its existing four batches of three services, exactly one Compose build per
service, Cargo jobs 2 and Compose parallelism 1, in a low-priority background
systemd unit with resource/health gates. This packet does not launch it.

Actual twelve-image evidence is then needed for the
[approved-base host-local whole flow](../runbooks/r2-approved-base-wholeflow.md).
Protected base/config/writer identities and the maintenance interruption budget
must be bound before any G4 action. In particular, cold recovery requires a
separate, explicit database/writer outage scope. Actual deployment, Owner use
and three consecutive trading days remain unstarted.

Diagram source citations are prepared separately against this candidate;
local PNG rendering, integration and independent review are still required.
No new worker or delegation is authorized by this packet.
