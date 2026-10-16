# R2 first-rollout fixture correction: hosted verification

Roadmap R2-04/R2-09; Wave83. Status: RUNNING WITH POLICY FAILURE,2026-10-16.

Candidate `2db7e48f1e17f85e37c075e5f986022fc5a9d5bd` is in
[CI38892070778, attempt1](https://github.com/sihioov/Lagrange/actions/runs/38892070778).
Created2026-10-16T11:10:53Z; reporting bound2026-10-16T12:40:53+00:00.
Actual workflow is CI/ci.yml,workflow_dispatch,branch excited-dolphin.
Pre-push and post-push all-event/all-workflow exact-head queries returned0;
remote SHA matched. Dispatch count is exactly1. Initial empty propagation did
not cause redispatch. No older run was rerun,cancelled or duplicated.

## Bounded correction

The [previous full result](2026-10-16-r2-first-rollout-ci.md) is preserved:
new95 definitions were91PASS/4ERROR. The four native cold-recovery cases stopped
at PREUPGRADE_VOLUME_PARENT before clone assertions. Actual failed parent
metadata was unprinted and stays UNKNOWN.

The [native fixture](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py:118)
now binds the fresh labelled local volume and stopped source generation,opens
only that volume's parent relative to a held protected ancestor,requires
root ownership/non-writable ordinary mode/no ACL,and checks inode/name identity
before and after setting0700. It rechecks Docker volume metadata and source
generation. Only finite UID/GID/mode fields are emitted. The shared Docker
ancestor and installed production resources are not changed by this fixture.

The production snapshot library and its positive/negative guard tests remain
byte-identical. All95 first-rollout definition names remain; four native source
anchors moved40 lines. No new definition or operational exception was added.
See the [existing guard and execution limits](../runbooks/r2-preupgrade-recovery.md).

Coordinator direct source/text review and whitespace checks passed. This is
not independent integrated review or executable acceptance. No local project
import,parser,test,build,Docker,DB,provider or production operation occurred.

## Frozen scope and recovery

The correction freeze has1093 pins and503 candidate blobs;1072 prior pins
are unchanged and exactly2 prior files differ. Original failed-run receipts
and inventories remain frozen. The successor definition inventory retains all
current and carried groups; it does not turn their previous outcomes into
results for this candidate.

Private input SHA256:
`4cea519bb4eb6283189e92eefd21b8a12ee79b46492493dc697130f3a2496b8c`.
Private inventory SHA256:
`87e8f2a0315d7c2ad61b1ae54ffebee870c07f0d2886b2b5eca284c56ac759e7`.
Both and the single-dispatch receipt are under
`target/r2-daily-beta/wp-4-integration/wave83-first-rollout-correction-*`.

Recover all8 jobs/steps/full log digests,7 actual checkout SHAs,all95 qualified
first-rollout outcomes and carried Rust/PG/Web/policy gates. Missing or
unprinted nested results remain UNKNOWN. Keep executable inputs frozen until
all jobs terminate and full results are recovered. At the reporting bound,
reconcile this exact run; do not infer termination or launch a replacement.

Actual12 candidate images,approved-base host whole flow,PUML/PNG integration,
independent integrated review,G3/G4 operations,real Owner use and3 consecutive
trading days remain outstanding. R2/final C_R2 are not complete.

## Partial recovery and bounded diagnosis — 2026-10-16

Four completed jobs have full log hashes and actual checkout SHAs matching
`2db7e48`: workspace format, strict Clippy and Python succeeded; policy failed.
Python reported487 passed/10 existing skips/7 warnings. Rust workspace,
PostgreSQL and web jobs were still running at this recovery; the required
aggregate had not started. These are partial results, not candidate acceptance.

Policy printed268 passing unittest rows and a class-setup error at
`PreupgradeDockerTests.setUpClass`. The failing call reaches
[the volume metadata comparison](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py:150)
and raises `PREUPGRADE_FIXTURE_VOLUME_CHANGED` at line153. Earlier descriptor,
ownership, ACL, mode and inode checks completed. All four native recovery
definitions are NOT_EXECUTED because class setup failed; they are not four
independent assertion failures. No parent metadata marker was printed, so the
actual initial mode and the changed Docker metadata field remain UNKNOWN.
The whole-flow boundary step after this failure still passed its19 mocked cases.

Static diagnosis found that this comparison includes `CreatedAt` across an
intentional `fchmod` of the volume parent. The examined
[Moby v28.5.1 local-volume implementation](https://github.com/moby/moby/blob/v28.5.1/volume/local/local_unix.go#L214-L220)
derives `CreatedAt` from that parent's `st_ctim`; its
[upstream change](https://github.com/moby/moby/pull/44719) moved this observation
from `_data` to the parent. Thus unchanged `CreatedAt` is not a valid general
identity invariant across this operation. The runner engine version and actual
changed field were not printed, so this static finding does not identify the
observed changed field as a fact. Preserve the exact label/name/driver/options,
mountpoint, held descriptor identity and stopped source-generation checks in
any correction, and report only finite changed-field names, never field values.

Private sanitized receipts are in `target/r2-daily-beta/ci/2db7e48/`;
the policy full-log SHA256 is
`f96cb071b1662a961b5fdfb4400230f72a5765fca42dd05421756a2245096682`.
Raw logs remain memory-only. Frozen executable inputs are unchanged. Finish
recovery of this exact run before source correction; no rerun, cancellation,
duplicate dispatch, local project execution or production action occurred.
