# R2 current candidate: remaining host inputs

Candidate `ab48a7953dd07fc479407cf80a58f37e2c0c6a83`; R2-04/09.
This is a prepared successor to the historical
[host admission packet](2026-10-16-r2-host-admission-packet.md).
No production state has changed.

Current Wave90 disposition — 2026-10-17 14:46UTC: exact-candidate required CI
and separate functional smoke are now fully recovered and accepted for hosted
verification. Publication/repeated-run assertions and actual cleanup passed;
see the [complete recovery](2026-10-17-r2-smoke-fixture-ci.md). The next actual
host step is BLOCKED by the recorded access failure. Access restoration was
already requested; there is no new signal and no repeated probe or question.

At2026-10-17T13:46:16UTC the already-authorized noninteractive access probe
`sudo -n /usr/bin/id -u` returned1. No password was requested, no policy was
changed and no privileged follow-up ran. The precise denial reason is unknown.
Restoring actual access is separate from the Owner's already-given consent to
the bounded read-only observation. A new password or secret must not enter a
review, log or chat.

Current-user observations at13:47–13:48UTC found:

- Selected immutable release remains `fce9b90542cf9b1a911d46e4c4d3530db7efba4c`.
- Available memory7,903,965,184bytes; used swap4,446,101,504bytes; no listed
  Cargo/rustc/rustup/Buildx/buildkitd/cc1 processes. These are point-in-time data.
- All ten running project containers were healthy, restart0 and OOM false.
  Research-worker remained exited2/unhealthy, restart7935, OOM false.
- The candidate build systemd unit was not found. Protected output absence,
  protected manifest/state metadata and complete kernel OOM visibility remain
  UNKNOWN. Current-user Docker inspection used only fixed fields and the
  exact production Compose project; no lifecycle action occurred.

The candidate source is prepared at
`/data/worktrees/3puw275b/r2-release-ab48a79`, clean detached HEAD with2125
tracked-file hashes. The inactive build-only env is
`target/r2-daily-beta/image-preparation-ab48a79/image-only.env`,0600 behind a0700
parent; it contains the existing inert sentinel, no operational credentials.
Preparation receipt SHA256:
`e2ea19e5bd84df9b528df798f0e5ae4212f1254131b2038fecd85b6646728153`.
At14:45UTC all2125 tracked pins, clean detached candidate HEAD and the unchanged
inert env hash/modes were checked again using static reads. No builder or
project helper ran. Successor reconciliation receipt:
`target/r2-daily-beta/wp-7/wave90/host-preparation-reconciliation.json`, SHA256
`1bdacd3cd067c738d3cad86a5c26f322251d0c509b8961c759f581e77229b777`.

The intended immutable manifest is
`/etc/lagrange/release-manifests/ab48a7953dd07fc479407cf80a58f37e2c0c6a83.manifest`;
the intended build unit is `lagrange-r2-build-ab48a79.service`. Neither was
created. The official builder and its production gates are unchanged.

The next dependency sequence is concrete:

1. **DONE, hosted scope only:** exact candidate CI39037896827 and functional
   smoke39037899130 were fully recovered, both attempt1 SUCCESS. Nine full-log
   hashes/eight actual checkouts and successful cleanup are recorded. Do not
   rerun, cancel or duplicate either run.
2. After access restoration, repeat the five-minute read-only admission packet
   using this candidate's manifest and unit names. Stop on an access error;
   retain only the fixed fields specified there.
3. If research is still stopped, ordinary eleven-healthy-service admission
   cannot pass. Prepare the exact current Owner V2 task/lease drain and bounded
   stop/resume decision before using the existing drained-reader route. Its
   candidate-bound attestation must be fresh and valid for no more than12hours.
   Do not reuse the consumed C3 permission or manufacture a stopped observation.
4. Only after G3 admission, run the official builder in a low-priority
   background systemd unit. It builds four batches of three services, one
   Compose service at a time, COMPOSE_PARALLEL_LIMIT1/CARGO_BUILD_JOBS2; verify
   health/memory/swap/OOM and termination between batches. Recover all twelve
   actual image IDs/revisions/saved bytes and the manifest.
5. Bind the protected actual approved V3 base and a fresh private receipt to
   the existing1200-second host-local whole-flow driver. Hosted mock fixture
   results are not this execution. Then finish the G4 maintenance/recovery,
   migration/installation and exact catch-up budget packet before any new
   service or database interruption.

The physical base path, current protected configuration/secret references,
settled jobs/leases, complete writer inventory, available maintenance window
and actual missing-session request budget still require the authorized host
observations and concrete G4 disposition. They are not invented here.
Root restoration alone does not authorize a service stop, migration or release
switch. Actual deployment, Owner use and three consecutive trading days start
only after their prerequisites pass; final C_R2 is not yet accepted.

Continuation heartbeatb9bb7ac0 tracks these remaining dependencies every15minutes
through2026-10-18T02:45:54UTC, with renewal/handoff due by02:15UTC. It replaced
14825895 only after complete recovery, actual successor creation and persisted
bounded handoff. Native Ralph remains blocked with no agent resume API. When
access has not changed, the coordinator records a bounded no-change status;
it does not repeat probes, ask the same question, dispatch another CI, or create
documentation commits to imply progress. The same-daemon monitor does not
protect against a daemon outage. R2 remains tracked through its final gates.
