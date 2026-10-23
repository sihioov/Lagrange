# R2 image verification contract correction

R2-04/09, Wave93. The Owner's “이어서 다시 진행해” authorized the separately
prepared second maintenance attempt. It ran exactly once against
`ab48a7953dd07fc479407cf80a58f37e2c0c6a83`. Normal shutdown succeeded this time,
but verification rejected the first built image with
`product-image-mode-contract-drift`. The original Owner V2 container was
restored; the other eleven candidate images never started.

## Second attempt and recovery

Times are UTC on2026-10-18; add nine hours for KST.

| Observation | Result |
| --- | --- |
| Source/control preparation | 2,125 source pins and eleven control files/inverse substitutions matched; clean detached candidate and inert env preserved |
| Fresh admission | Exact eleven containers, ten healthy and research stopped2; resource policy passed, no compiler or recent OOM |
| Historical retained units | Initial broad active-unit check stopped. Seven old `RemainAfterExit=yes` units were then proven exited successfully with main/control PID0 and no cgroup process; no unit was changed |
| Work/plan/preflight | READ ONLY operator-wide work/stream settlement and both official commands passed; source-input hash `454cad1f9cb8f12ddafca6729897acc9ef2eccd5947adf40942020b1e9a1cf76`; all source pins matched again |
| Recovery readiness | Active before dispatch; shared window00:42:55.850405–12:42:55.850405, stop first hour, build10h, watchdog10.5h, attestation11h |
| Dispatch | Exactly once,00:43:20.244943; `lagrange-r2-build-ab48a79-a2.service` |
| Normal drain | Exit0; stopped00:43:20.719700682; before/after READ ONLY settlement and drained-reader attestation succeeded |
| Main builder | Started00:43:21, exited1 at00:43:25; run-start gate PASS; first service `db-role-bootstrap` failed verification |
| Normal restoration | ExecStopPost exit0; same ID/image healthy00:43:31.247624 |
| Independent recovery | Same-ID healthy receipt00:43:40.938161 and actual successful deactivation; later `not-found` alone was not acceptance |
| After recovery | All eleven fixed container states restored; no compiler/OOM; READ ONLY running jobs/attempts, unexpired producer, epoch and pending subscription operations all0 |

One candidate-tagged image exists, with revision `ab48a79` and ID
`sha256:8e1e5635fffe6819ab46d98b3ed1267966dca52c2d3a0b114d67a980bba01b8a`.
It is **not accepted**. Request generation failed before image archive saving
and byte validation. Verified images remain0/12 and no manifest exists.
The remaining eleven candidate image references were absent when inspected.

The private second-attempt closure
`target/r2-daily-beta/wp-8/maintenance-ab48a79-a2/attempt-closure.json`
pins eighteen receipts; SHA256
`7f5227c44d30b3b3b464f0d2b87f8fdb651819d6e873a0f1f81f1b651f71c5a6`.
Full finite unit-journal capture SHA256:
`cde749b0db0a53bf1383663f596569eb51df5a6515f0401ae3369ac4892bfc54`.
Only structured evidence, hashes and allowlisted diagnostics were retained.
First-attempt evidence, both failed units, root controls, candidate image/state
and historical hosted acceptance are preserved. No third attempt is authorized.

## Source diagnosis and correction

The R2 DB Dockerfile already copies `deploy/db/daily-control.sh` and chmods
three wrappers. The image verifier still expected the earlier exact one-line
chmod of two wrappers. Its exact-instruction check therefore rejected the
current continued instruction. Independently, the database runtime inventory
omitted daily-control, so its bytes and final mode were not selected for
verification. Both defects are corrected together in this successor source.

| File | Bounded change |
| --- | --- |
| `deploy/db/Dockerfile` | Express the same three-path0755 chmod as one canonical instruction; runtime command, user and permissions are unchanged |
| `deploy/build/release-build-layout.json` | Include daily-control source/destination in the database runtime payload |
| `scripts/ops/lib/release-build-layout.sh` | Bind daily-control0755 for both DB services and the exact three-path Dockerfile instruction |
| `scripts/ops/build-production-images-self-test.sh` | Independently model the final mode, retain0775 source mode, verify selected bytes/mode, reject0775 final modes for both services, and reject omission from the chmod instruction |
| `.github/workflows/ci.yml` | Run the complete existing synthetic-Docker image-builder suite as a required policy step; process-local Git trust is limited to the checked-out workspace |
| `scripts/ci/test_ci_contract.py` | Require that fixture step exactly once, with failure propagation and execution after unrelated failures |

The validator's exact instruction count, destination checks, byte digests,
mode equality, archive parser, no-clobber publication and production resource
guards remain mandatory. The original accepted checkout is unchanged. There
is no runtime dependency or deployment topology change; diagrams are unchanged.

The earlier CI ran the image builder's static check, which checks self-test
syntax but does not execute the whole synthetic image path. The new required
step closes this specific coverage gap. Synthetic images do not establish
actual Docker image or production acceptance.

Coordinator source review covered all six complete deltas, both DB services,
the literal Dockerfile/contract binding, source-to-image payload selection,
independent fixture mode construction, negative-case routing and CI wiring.
This is a coordinator disposition, not a new independent review. Existing
integrated review remains evidence for the earlier candidate only.

One new Python test and four named shell outcomes are written and unexecuted:

- `WorkflowContractTests.test_image_builder_fixture_is_a_required_policy_step`
- `PRODUCTION_DAILY_CONTROL_BOOTSTRAP_MODE_REJECTION`
- `PRODUCTION_DAILY_CONTROL_MIGRATE_MODE_REJECTION`
- `PRODUCTION_DAILY_CONTROL_PAYLOAD_BYTES_AND_MODE`
- `PRODUCTION_DAILY_CONTROL_RECIPE_OMISSION_REJECTION`

All prior required tests and the complete image-builder fixture remain required.
Only static source/whitespace checks have run for this correction; no local
compiler, test, parser, project checker, SQL or Docker execution was added.
Freeze the successor candidate and inventories, query exact-head runs and
remote SHA, then recover one new hosted CI and separate functional smoke.
Never rerun or duplicate the earlier successful or failed runs.

After hosted verification, prepare a candidate-specific maintenance proposal
with fresh controls and guards before requesting a third interruption. The
consumed second approval does not authorize it. Actual images, approved-base
host flow, G4 deployment, Owner use, three consecutive trading days and final
C_R2 remain open.
