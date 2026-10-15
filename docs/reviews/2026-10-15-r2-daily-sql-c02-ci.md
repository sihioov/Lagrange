# R2 I2-C02 — exact-candidate hosted verification

2026-10-15; candidate a252cfbabcfeb1c94cb006587fe160feb7e6d867.
[CI38694431037](https://github.com/sihioov/Lagrange/actions/runs/38694431037),
attempt1, CI/ci.yml, workflow_dispatch, excited-dolphin, sihioov/Lagrange.

**IN_PROGRESS; no acceptance.** [C02 source disposition](2026-10-15-r2-daily-sql-c02.md)
follows complete recovery of failed6fbc976. All241 frozen pins and202 tracked
candidate blobs matched. All-event exact-head query0 and remote SHA match at
06:46:20 UTC preceded one dispatch; the run was created06:46:22. No rerun/cancel.

Recover8 jobs/steps/logs,7 actual checkout SHAs,32 daily DB names/73 subcases,
28 affected legacy names,99 prior Rust/6 separate chart/83 bootstrap/3 transport,
strict lint/format,60-checksum/no-op/roles/rollback,12 numeric DB exits and cleanup.
Missing rows remain UNKNOWN unless stage evidence proves NOT_EXECUTED. Keep the
workspace and dedicated PG outcomes separate, including unprinted subcases.
Raw logs stay in memory; sanitized receipts live under target/r2-daily-beta/ci/a252cfb.

Frozen Wave39 source inputs have241 pins, SHA256
16a89213d8f546bb3f54d56dd11d46d7e77ad00733e42c2a0cdb890766d2067a;
definition inventory736lines, SHA256
bd173cc7b59c17611e3c0b5ea15073fd5426aaafec6ef562f5232adc05a289c9.
Both are under target/r2-daily-beta/wp-4-integration. The predecessor inputs,
results and all earlier partial successes remain immutable. Direct coordinator
recovery has a75-minute reporting bound; no worker or same-daemon heartbeat is
claimed. Subsequent documentation HEAD cannot change the pinned source candidate.

R2-04/09, I3–I7, runtime/Owner/three trading days and final C_R2 remain open.

## Complete recovery — 07:05:45 UTC

The launch status above is historical. All8 jobs/84 steps/8 log hashes and7
actual checkout SHAs were recovered.241 pins and202 candidate blobs still match.
The run is FAILURE:82 steps succeeded,2 failed (PG validation and required).
All other jobs, including workspace, strict Clippy, format and Web E2E, passed.

Workspace individually passed32 daily definitions,28 formerly failing legacy
names,99 prior R2 Rust names,6 chart and3 transport. No failed named row remains.
Its2974 printed rows are not presented as a unique test count. Dedicated PG
migration-contract also passed all32 names, exit0 and no SKIP. All73 fixed subcase
names have direct PASS log evidence; echoed duplicate rows are not additional
cases. This closes the observed C02 SQL/fixture/isolated compilation conditions
at those stages, while I2 remains unaccepted pending the complete harness.

The sole remaining defect is the harness's subcase collector at validate.sh1533:
`[a-z_]+` omits `receipt_policy_sha256`. The raw test output contains its PASS
at sanitized log line1787, but canonical evidence has only72 rows; the required
exact-name check correctly fails. Later11 DB labels are NOT_EXECUTED. The
correction must admit digits without weakening the exact73-name uniqueness gate.

Both full-release migration passes match60 candidate SHA384 checksums, rerun is
NO_OP, seven direct principals in both lanes,0039 guard, four catalog probes and
0060 role/rollback harness pass. All83 bootstrap cases pass. Cleanup remains
PASS_NO_OWNED_CONTAINERS. The API's config INSERT at
crates/api-server/src/repos/strategy_configs.rs56 already uses a DB-allocated ID,
consistent with the corrected actual-app fixtures.

Private final receipt: target/r2-daily-beta/ci/a252cfb/ci-evidence.json,
3364lines, SHA256894813e488daa8f663ebe65cf07042277802f556ff295a3985c6302cbab88ad5.
All raw logs stayed in memory. Earlier failures, inputs and partial receipts
remain preserved. C03 follows complete recovery and changes one harness line;
no SQL, Rust, privileges, test assertions or workflow changes are indicated.
