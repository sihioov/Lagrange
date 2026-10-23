# R2 independent integrated review packet

Prepared 2026-10-16 for WP-7 and R2-09. **This is a review input, not an
independent review result.** No reviewer has been launched or delegated work.

The candidate is `f215665012d1f4f1b81289dd94606bceb0cb8c46`.
[CI38901117928 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38901117928)
completed successfully at12:55:45UTC. The coordinator recovered all8 job logs,
7 actual checkouts,1137 pins/506 blobs and100 first-rollout named PASS results.
See the [hosted disposition](2026-10-16-r2-preupgrade-ownership-ci.md).
That closes this package's hosted verification, not the release gates below.

## Fixed input and proposed scope

`target/r2-daily-beta/wp-7/f215665/review-inputs.json` binds all202
non-documentation changed paths against the R1 integration baseline `b0a2579`,
their exact candidate blobs and SHA256 hashes, the full CI evidence, the final
definition inventory, and both prepared diagram sources. Its SHA256 is
`f7e598e6900df1393bc36f6f4969b8039fa05fd626bd72ccbd69aa2785c1c1f4`.

The proposed review has a60-minute reporting bound and writes only
`docs/reviews/2026-10-16-r2-integrated-release-review.md`. At the bound it must
report exact inspected/uninspected scope and concrete findings; elapsed time
or a thin summary cannot imply complete review. It performs no source edits,
local project execution, operational action, provider request, Git write or
further delegation. Any later candidate or scope change needs a new binding.

| Roadmap scope | Required independent disposition |
| --- | --- |
| R2-01/02/03 | Installed-mode diagnosis, calendar/window responsibilities, approved daily timing and request-budget boundary; distinguish old observations from current admission. |
| R2-04 | Approved V3 base, Raw evidence, reconstruction, continuation/frontier, admission, scheduling, computation, publication and API provenance/as_of. Preserve historical results and explicit fail-closed behavior. |
| R2-05/06/07 | Owner V2 compatibility, Auth0/CSRF/RLS, UI stale/failure/session behavior and recovery. Synthetic browser success is separate from actual Owner use. |
| R2-08 | Same-candidate first publication and three consecutive trading-day evidence; no calendar-time or manual-repair substitute. |
| R2-09 | Bootstrap/migration/roles, backup version transition, cold pre-upgrade recovery, writer fencing, ownership/ACLs, no-clobber recovery and operation effects. Bind any finding to concrete source and a closure condition. |

Review the full candidate, not only the four most recent corrected files.
Earlier independent component reports remain evidence with their original
revision/scope limits. Coordinator self-review is not an independent result.
The [adopted WP-7/G3 requirements](../superpowers/plans/2026-10-13-r2-daily-research-beta.md#L327)
remain unchanged.

## Operational and structural inputs

- [First-rollout dependency analysis](2026-10-16-r2-first-rollout-readiness.md)
  is dated source preparation; its older unexecuted fixture states are
  superseded only by the current hosted disposition.
- [First backup transition](../runbooks/r2-first-backup-transition.md),
  [pre-upgrade recovery](../runbooks/r2-preupgrade-recovery.md), and
  [approved-base whole flow](../runbooks/r2-approved-base-wholeflow.md) specify
  distinct stages. The actual approved-base host test remains unexecuted.
- [Fresh host admission packet](2026-10-16-r2-host-admission-packet.md)
  records the prepared exact source/build paths and remaining observations.
  Existing read-only KIS/EOD and Owner V2 approvals persist; this packet does
  not renew historical root authority or authorize maintenance interruption.
- Both PUML drafts are in
  `/data/worktrees/3puw275b/r2-diagram-draft-977e356/docs/diagrams/`.
  The303 evidence ranges are bound to this candidate in
  `target/r2-daily-beta/wp-4-integration/I6C-static-checkpoint-f215665.json`.
  Source citations were checked; local PNG rendering, integration and
  independent structural review remain open.

The review must retain unresolved host/image/runtime/Owner evidence as open,
even if it finds no source defect. The no-further-delegation restriction is
still in force; preparing this packet does not create an exception. The
coordinator cannot advance G3 by relabelling its own review as independent.

No executable change or another CI run is proposed by this packet. R2 and
final`C_R2` remain incomplete; R3 is not yet admitted.
