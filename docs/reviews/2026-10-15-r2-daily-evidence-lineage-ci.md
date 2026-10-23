# R2 I1b-S3 — exact-candidate hosted CI

Completed 2026-10-15 (Asia/Seoul), scoped to R2-04/R2-09.
Candidate: `eb0e839b5e808fb1353451b47baa1850d248ebb9`.
[Run 38711965495, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38711965495)
completed **SUCCESS** at 09:40:29 UTC. Repository sihioov/Lagrange,
branch excited-dolphin, workflow CI/ci.yml, event workflow_dispatch. The all-event
exact-head existing-run query returned zero and the remote SHA matched before
the single dispatch at 09:18:25 UTC. No run was rerun, canceled or duplicated.

## Recovered identity and execution

All eight terminal jobs, 84 steps and eight complete log digests were
recovered. Seven execution-job checkout logs contain the exact candidate SHA;
the required aggregate has no checkout. All 262 frozen input pins and 215
committed candidate blobs match. The later documentation commits do not change
the tested source revision.

| Job | Final result |
| --- | --- |
| policy | SUCCESS |
| python-tests | SUCCESS; 485 passed, 10 existing skips, 7 warnings |
| web | SUCCESS; OpenAPI, lint, typecheck, tests, build and E2E |
| clippy | SUCCESS; workspace/all-targets/all-features with -D warnings |
| workspace-tests | SUCCESS; separate chart step and Cargo workspace step |
| format | SUCCESS; workspace formatting |
| postgres-integration-validation | SUCCESS; actual DB and cleanup gates below |
| required | SUCCESS |

83 steps succeeded. The only skipped step is the conditional
PostgreSQL failure-log dump.

## Individually reconciled results

Each required qualified Rust name was matched exactly once in its execution
phase. These are inventory groups, not an added total of unique tests. Chart
rows belong to their separate chart step.

| Group | Exact named outcomes |
| --- | --- |
| New S3 original/copy/calendar definitions | 22 ok |
| Affected existing normalizer/calendar definitions | 37 ok |
| Prior R2 Rust inventory | 99 ok |
| I2 daily SQL definitions in workspace | 32 ok |
| Affected legacy DB definitions | 28 ok |
| Transport definitions | 3 ok |
| Separate chart acceptance | 6 ok |

The dedicated PostgreSQL lane separately reports exactly 32 daily outcomes and
73 unique fixed subcases, including digit-bearing names. All 12 outer actual-DB
labels exited 0 with no SKIP marker. All 83 canonical bootstrap names passed.
All 60 migration UP SHA384 checksums match candidate blobs in both full-release
lanes; the rerun is NO_OP. Seven service principals logged in directly in both
lanes. The 0039 guard, catalog conflict/preservation/negative probes, 0060
role/rollback harness and PASS_NO_OWNED_CONTAINERS cleanup all passed.

The final private sanitized receipt is
`target/r2-daily-beta/ci/eb0e839/ci-evidence.json`, 2641 lines,
SHA256 `bcf9f0aa778c59a823dc16d9b428ba57f73d77f45e986cc9196f0d12dea10cc6`. It records each required name, job/step result, source
log hash, checkout, input pin and DB gate. Raw log bodies were processed in memory
and were not persisted.

## Disposition and limits

The coordinator accepts the bounded lineage/calendar implementation and its
exact-candidate hosted verification in the
[S3 acceptance record](2026-10-15-r2-daily-evidence-lineage-acceptance.md).
[Source disposition](2026-10-15-r2-daily-evidence-lineage-source.md) was direct
coordinator inspection, not an independent review. Its written-unexecuted state
is preserved as the historical pre-CI record.

All 22 S3 fixtures ran on Linux. Two carried non-Linux cases remain excluded;
Python skips are not passes. Unprinted nested helper case names and inner
process exits remain UNKNOWN; the twelve outer numeric exits are separate
evidence. Synthetic fixtures do not establish installed UID/ACL, live supplier,
Owner or market-time behavior.

No local Cargo/compiler/Clippy/test/Raw parser/SQL/DB/Docker/project-helper or
provider execution occurred. Source-side executable checks were limited to
scoped standalone rustfmt2024. No new worker or delegation occurred.

Prior [I2 acceptance](2026-10-15-r2-daily-sql-acceptance.md) and all older successes
and failures retain their original revisions and limits. Remaining I1b
bars/reference/actions, sealed frontier, artifact/admission/supply, I3–I7,
final integrated independent review, runtime UID/ACL, installer filesystem
rollback, diagrams before main, operational activation, Owner use and three
consecutive trading days/final C_R2 remain open.
