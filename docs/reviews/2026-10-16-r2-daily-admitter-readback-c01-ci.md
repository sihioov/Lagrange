# R2 I4g2 C01 exact-candidate CI recovery

R2-04/09, Wave61. Candidate `9d0d4591708cc618b9d050d2e223d5eea8b15b64`,
[CI38798878711 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38798878711),
workflow_dispatch on excited-dolphin. Fully recovered at `2026-10-15T20:29:27.691352+00:00`.
Overall FAILURE. The new readback package passed its named cases, but the complete
candidate is not accepted. No rerun, cancellation, duplicate dispatch or local project execution.

## Direct evidence

All8terminaljobs/logs and7actualcheckoutSHAs match. All620frozenpins and340candidate
blobs match. Receipt `target/r2-daily-beta/ci/9d0d459/ci-evidence.json` has8092lines and
SHA256 `eac40093b025b65ae9e560f86acbef40777a966eb77835e3ed35b89ade29557a`; sanitized metadata only.
Step outcomes:82success,3failure,1skipped. The skip is setup-python post cleanup,
not a required test step. Workspaceformat and strict all-target/all-feature Clippy pass.

| Evidence | Result |
| --- | --- |
| New readback definitions |18/18 direct workspace rows PASS;4pure+14actualDB |
| Dedicated schedule/readback |28/28 named PASS, exit0/noSKIP |
| Existing daily runner |23/24 PASS in workspace and dedicated PG; same one failure |
| Navigation/earlier Rust groups |Every other required row PASS; overlapping groups are not summed |
| Chart |6 separate named PASS |
| Dedicated PG |16targets exit0/noSKIP; daily-runner target exit101/noSKIP |
| PG contracts |32daily/73subcases/83bootstrap,8historicalpublish,6dailypublish,6recovery PASS |
| Full-release migration |62UP checksums match exact blobs on both runs; rerunNO_OP |
| PG safety |directroles,0039guard,catalogprobes,cleanup PASS |
| Web |34supervisor +121OFF/14market-stream; no skip/retry/flaky; cleanup PASS |
| Python |487pass/10existing skips/7warnings |

The sole failing definition is `daily_supersession_additive_migration_down_up_preserves_populated_rows`,
`crates/job-queue/tests/owner_beta_daily_runner.rs:963`. The test reapplies the full
embedded migration set but expects `(count,max)=(61,61)`. The actual tuple is
`(62,62)` after the additive0062 migration. The same source assertion is reported
in both workspace and dedicated PG. No production defect is inferred from that
assertion. The numeric supplemental receipt binds the identical full PG log SHA
`4fb6b6b25410befbd6118cca20898961033760f9f97ce8bdc117fe2aa3c7b178`.

## Next bounded action and limits

After complete recovery, change that assertion to compare the complete ordered
applied-version set with embedded UP migration versions. Combine this correction
with the already prepared I4g3 full producer/admitter/mixed-worker connection in
one registered source scope and one future exact-candidate CI. Keep previous
failures and accepted bounded packages immutable. No old run will be rerun.

Real approved-base complete ingestion, distinct UID/ACL/crash behavior, installed
activation/rollback, independent integration review, I5-I7/I6 diagrams before main,
G4 operations, actual Owner use,3trading days and final C_R2 remain open. Ralph continues.
