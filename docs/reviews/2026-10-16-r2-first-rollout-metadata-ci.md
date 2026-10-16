# R2 recovery fixture metadata correction: hosted verification

Roadmap R2-04/R2-09; Wave84. Status: POLICY FAILED; other CI jobs still
running at2026-10-16T11:57:54UTC. Full recovery and disposition remain pending.

Candidate `6572bff9d8e75057d15ecb969f287cec8771374f` is in
[CI38896226384, attempt1](https://github.com/sihioov/Lagrange/actions/runs/38896226384).
Created2026-10-16T11:48:32Z; reporting bound2026-10-16T13:18:32Z.
Workflow CI/ci.yml,workflow_dispatch,branch excited-dolphin. All-event and
all-workflow exact-head queries returned0 before and after push; the remote
SHA matched. Dispatch count is exactly1. No prior run was rerun or cancelled.

## Corrected scope

The [previous full failure](2026-10-16-r2-first-rollout-correction-ci.md) is
preserved: all8 jobs/logs and7 checkout SHAs recovered;95 definitions had
91 PASS and4 NOT_EXECUTED. Policy failed during native class setup, and the
required aggregate failed. All six other execution jobs passed. The actual
changed metadata field was unprinted and remains UNKNOWN.

Only the [native recovery fixture](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py)
changes executable bytes. Its comparison now permits the derived `CreatedAt`
timestamp to change across its own permission update; all six stable volume
fields still require equality. The directory descriptor remains open through
Docker and stopped-generation observations, followed by final inode/name,
owner/group/mode and ACL checks. Diagnostics expose only finite field names
and mode values. The [runbook](../runbooks/r2-preupgrade-recovery.md) records
the upstream source evidence and the production boundary.

Three added regressions use actual private root-owned directories and controlled
Docker observations: timestamp-only change, all six stable-field mutations,
and directory replacement during inspection. The four native Docker/PG test
bodies, production recovery code, existing negative guards and workflow are
byte-identical. No operational authority or production behavior is changed.

Coordinator direct source review and whitespace/text checks passed. This is
not independent integrated review or executable acceptance. No local project
import/parser/test/build, Docker, SQL, provider or production operation ran.

## Frozen candidate and required recovery

The freeze has1114 pins and504 committed blobs;1091 previous pins are unchanged
and exactly2 previous files differ. Its98 first-rollout definitions comprise
all95 previous names and3 additions. Previous outcomes are not results for this
candidate. Recover the seven preparation/native cases individually, all carried
named Rust/PG/policy/web requirements, all8 job/step outcomes, full log hashes,
7 actual checkout SHAs, strict lint, formatting and cleanup.

Input SHA256:
`c0bdc16db89cb6e4cce7aa7b4a567f5266e59cf0fcb38322888062fa3ebdf670`.
Inventory SHA256:
`f4ecae20a932c85ec8e1003a2f931267606d152e0c16078b694fce822e73a0bf`.
Frozen inputs/inventory and the launch receipt are under
`target/r2-daily-beta/wp-4-integration/wave84-first-rollout-metadata-*`.

Keep executable inputs frozen until the exact run is terminal and its full
result is recovered. Missing or unprinted nested outcomes remain UNKNOWN.
At the reporting bound, reconcile the actual run; time elapsed does not prove
termination and does not authorize a duplicate run.

Actual candidate images, approved-base host integration, diagram PNGs and
integration, independent review, G3/G4 operations, Owner use and three
consecutive trading days remain open. R2/final C_R2 are not complete.

## Partial recovery and newly exposed test limitation

The policy job completed with failure at11:50:55UTC. Its full log digest is
`f1968cb01dc32fec5508440b458681216313cd3c8e08f58667ac663bd088fafc`.
The three new preparation tests passed. Finite native diagnostics now actually
show `CreatedAt` changing and the owned volume parent moving from0701 to0700
with uid/gid0. This does not retroactively fill the previous run's missing
metadata. Format, strict Clippy and Python jobs passed; their full digests and
actual candidate checkout SHAs were recovered. Python reported487 passed,
10 existing skips and7 warnings. Workspace Rust, PostgreSQL and Web results
are not yet accepted for this candidate.

All four native recovery methods ran. The positive round-trip errored with
`PREUPGRADE_CLUSTER_OWNER` at the recovery library's line56, reached through
`verify_cluster` line188 and the test line276. This is before verifier-container
creation. The actual rejected PGDATA uid/gid/mode tuple was not printed and
remains UNKNOWN; no ownership normalization or guard relaxation is accepted.

The other three native methods printed `ok`, but source review exposes a
shared evidence defect: each accepts any `RuntimeErrorCode`. None asserts its
intended error code or proves the intended clone/create/native-validation stage
was reached. The same early owner rejection can satisfy these assertions.
Their logged outcomes are retained as PASS, while their claimed catalog,
lost-create-response and corrupt-control coverage is **NOT ESTABLISHED**.
They cannot close those recovery requirements.

Keep the current candidate frozen until all jobs and evidence are recovered.
Then correct the ownership compatibility issue and these stage-specific
assertions together, retaining a positive native round-trip and all ownership,
identity, ACL, private-parent and cleanup guarantees. First bind finite source
and restored metadata; do not infer it from an upstream image's current source.
No new dispatch, local project execution, worker or operational action occurred
during this recovery. Remaining operations and the three-day observation have
not started.
