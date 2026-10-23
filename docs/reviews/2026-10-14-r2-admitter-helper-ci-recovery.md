# R2 I0c hosted CI recovery and partial acceptance

On 2026-10-14 the coordinator recovered actual idle, no active turn, no pending
permissions, and substantive results for collector `11a8f223` at10:33:30.960UTC.
Its final callback contained only memory-hook status; activity supplied the full
CI result. The [original report](2026-10-14-r2-admitter-helper-ci.md) remains unchanged:
118lines, SHA256 `2587d1804ee035d7e30a618bd79ed7802b48a73596b2aa7767a4b3a682df7200`.
The original private receipt remains SHA256
`77241980b864ac78fcfb1168b2ae49a34f9c1c5ec99dfd7a00aab4474c41d5fb`.
The original report has a final blank line at118, reported by `git diff --check`.
Its bytes are preserved; source whitespace checks exclude this known evidence-file
formatting warning. It is not a compiler, test or runtime failure.

## Independently recovered evidence

Existing [run38545976070](https://github.com/sihioov/Lagrange/actions/runs/38545976070),
attempt1, exact `a888cd66c032c8931a4979434168568387c83a12`, completed successfully.
All seven execution jobs and required succeeded. The coordinator compared all28
source pins against the candidate and working tree, then retrieved workspace,
PostgreSQL, Clippy and policy logs read-only into memory.

- All51 inventoried Linux Rust names have direct `ok` rows:30wire,18S1 Raw,
  one collector and two direct-login helper regressions. The workspace workflow
  configured `DATABASE_URL`, and its QA setup and cleanup steps succeeded.
  This supports the helper tests' direct-login identity, actor-GUC, pool limit,
  privilege-denial and transaction rollback checks in isolated hosted CI.
- Six chart rows passed in the separate chart step. The complete job also runs
  those names in the workspace step, producing12 matches overall. An initial
  coordinator assertion incorrectly required one occurrence over the whole job;
  it failed and was corrected to select the dedicated step. No CI was rerun.
- Strict Clippy's exact `--workspace --all-targets --all-features --locked -- -D warnings`
  command was present. Timestamp/ANSI-normalized primary diagnostic extraction found
  zero warning/error headers; job and required conclusions independently succeeded.
- PostgreSQL's sanitized JSON reports `APPROVED`, both full-release migration counts59,
  identical59-item checksum lists, rerun `NO_OP`,0039 guard `PASS`, skip markers
  `ENFORCED`, and full-release cleanup `PASS_NO_OWNED_CONTAINERS`. Both exact new
  helper labels are present in its12-entry `tests_run` list. This resolves the
  original report's recoverable PG checksum/guard/no-SKIP/cleanup gaps.
- Successful official nested helper rows and nested Cargo exit numbers are not
  printed: those remain UNKNOWN. The single `SKIP` token in the original report
  is not evidence of a skipped helper; the sanitized gate explicitly says ENFORCED.
  Hidden-skip absence is not claimed.
- Policy wrapper markers were recovered: PG self-test and runtime-static PASS
  once each, OPS_STATIC PASS twice. Individual nested subcases and fakeroot
  branches are not independently established by wrapper markers.

Python485passed/10skipped/7warnings, quiet readiness/SIGTERM node gaps, Web evidence,
and aggregate Cargo row counts retain the original report's scope. A direct helper
pass does not establish negative bootstrap/rollback scenarios or first0060 readiness.

The private supplemental receipt is
`target/r2-daily-beta/wp-4-integration/wave22-I0c-CI-coordinator-verification.json`,
SHA256 `63c31c8ccf7c38e505f6052e48ce4381544a3d4260a25efa52a5096f70858eed`.
It retains only allowed status/checksum/test metadata and log byte counts/hashes.
Original worker hashes describe formatted `gh run view` logs; supplemental hashes
describe raw GitHub job-log responses, so those representations have different hashes.
No raw log bodies or credentials were persisted by the coordinator.

## Orchestration deviation and disposition

Despite its explicit direct-only assignment, the collector created Paseo child
`f344b3e1-bd56-4aab-b4ab-79455e05d6b4` and delegated collection. Activity also records
failed extraction attempts, permission decisions, and follow-ups. The original
receipt's single-retrieval wording is not accepted as a complete activity history.
The coordinator recovered this child's substantive final and actual idle/pending0
at10:33:04.058UTC; its parent link points to the collector. No running descendant
or permission wait remains in this recovered scope. This deviation is preserved
in the private recovery receipt and worker ledger; future collection assignments
must explicitly forbid Paseo re-delegation as well as native agents.

The earlier c0ac599 and failed f9dfffb reports/receipts remain historical evidence.
I0c source and the exercised isolated helper behavior are accepted **partially**
at a888cd6. New S2 snapshot files were absent from that candidate and receive no
CI credit. Negative bootstrap/rollback,0060, full snapshot/provider reconstruction,
admission, diagrams before structural main integration, runtime/Owner acceptance
and final C_R2 remain open. No local executable tests, build, SQL/DB, Docker,
provider, production or workflow mutation was performed during recovery.
