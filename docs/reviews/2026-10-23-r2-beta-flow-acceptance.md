# R2 beta flow acceptance — Waves176–177

2026-10-23; R2-07/R2-09. API/Web candidate
`a6da63c134fa471ff91e28a32c4b6a2ad9c08ba7`, preserved Owner V2
`7fa779cc7f0ff3cc291f1e444257beb5f7412c7c`.

The six-flow review reconciled existing source, isolated checks and the accepted
Wave174 deployment. It found no additional confirmed defect within those six
flows; this is not a claim that every product path is defect-free. Accepted CI,
images and browser tests were not rerun.

| Flow | Existing evidence | Remaining boundary |
| --- | --- | --- |
| Membership changes and recovery | Chromium49, current source and scoped deployment | Actual Owner acceptance |
| Recommendation creation | Chromium20, current source and scoped deployment | Accepted production inputs and Owner use |
| Recommendation polling | Chromium28; actual callers remount by run ID | Actual Owner acceptance |
| Daily preferences | Chromium28, current source and scoped deployment | Production Daily activation remains OFF |
| Daily state | Existing E2E plus Wave1758 response-lifetime cases | Synthetic visibility seam is not real tab acceptance |
| Service status | Existing controller/E2E and deployment HTTP checks | No continuous whole-service health claim |

The matrix is `target/r2-daily-beta/wp-8/wave176/acceptance-evidence-matrix.json`,
18623bytes, SHA256 `fc5184d5ce95f1ba57c2884cb1203d3025393bb2e6160964e6c7b21ef21099e0`.

## Current-candidate isolated wholeflow

The historical approved-base wholeflow accepted `e639169`, not the current
candidate. Changes in the executable daily path justified one distinct run of
the current candidate, using its existing exact images and the real approved
base with a synthetic two-session extension. No image build, provider call,
production database write, authentication or service restart was performed.

Original Wave177 a1 ran once from12:15:51UTC. The official driver reported PASS
at12:19:38UTC: first and subsequent admission, HTTP publication, restart replay,
exact private-state restore and restored replay all passed. The wrong mount
destination in preflight-a1 failed before creating runtime resources; corrected
preflight-a2 used the historical runner's actual read-only base mount. Both
preflight records remain preserved.

The12:21:12UTC independent observation confirmed all21 temporary containers and
two networks were removed, the private stage and driver were absent, the11
production states and21-container inventory were unchanged, and bounded OOM
checks were empty. The unrelated unproven volume was inspected only as metadata
and remained unchanged. At12:21:57UTC the original unit's exact cgroup was absent.
The unit had already been garbage-collected: its default not-found exit fields
are not used as evidence. Original loaded PID observations, exact successful
systemd deactivation and the driver's saved PASS establish termination.

**Functional PASS and cleanup are accepted; complete independent lifecycle
acceptance is incomplete.** Periodic Docker event snapshots retained full
create/start/die/destroy records for15 of21 containers. Six have missing events,
including five without an independently recorded exit code. The strict closure
failed and is preserved. A bounded daemon-journal metadata recovery did not
recover explicit exits. The driver's own exit checks cannot replace missing
independent records. No restoration is owed; the original is consumed and must
not be rerun to fill the gap.

Evidence: `target/r2-daily-beta/wp-8/wave177/functional-and-cleanup-closure.json`,
23034bytes, SHA256 `45b8c903caa43543d5779fc2eed313288314727b2990d74ebc59fe47ca75182d`.
It pins original observations, launch, failure, cleanup and source records.

Docker documents finite historical event retention and a real-time JSON event
stream. Future distinct wholeflows need bounded continuous sanitized recording,
with receipt of the first owned network event before starting containers and
complete lifecycle reconciliation before acceptance. This does not reconstruct
Wave177's missing records. [Docker event contract](https://docs.docker.com/reference/cli/docker/system/events/).

## Open acceptance

DailyOFF/epoch0, KIS disabled, inputs unadmitted, firstSessionnull and actual
three-day count0 remain the last accepted state. D-06 dividend completeness,
the005930 historical price conflict and D-07 real market reception/lifecycle/
30-instrument acceptance remain deferred. Whole R2-07, R2/R3 FINAL and Owner
acceptance remain open. These follow-ups do not block independent beta work.
