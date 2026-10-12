# R1 WP-5 — ENV1 and private full-smoke preparation review

2026-10-12; independent bounded source review. HEAD
`ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.
**REVIEW_COMPLETE: ENV1 source accepted; private full-smoke preparation requires S1
before execution handoff.** No browser/smoke or overall R1 acceptance.

## Findings, ordered by severity

**S1 / P2 — zero child exit can hide a builder/cleanup supervision failure.**
`target/r1-runtime-fxi6ltmv/smoke-full-preparation-c1ddee5b4e/supervise.py:226,229`
checks only builder `original_rc` and `timed_out`. `waited():60–61,80–87` separately
records startup/wait/termination errors, including an interrupt, then preserves the
child's actual exit at89. A receipt with rc0, no timeout and nonempty
`supervisor_errors` therefore passes these stage gates. Bootstrap/full smoke can
continue after that failure; the final acceptance expression296–298 checks only
`script_result` errors, so the failed builder receipt cannot prevent acceptance.
Cleanup receipts at274/284 similarly check only rc: their supervision errors or a
timeout ending in rc0 do not enter `issues`. This is a statically identified branch;
no such event is claimed observed in the retained fake checks or a real run.

Minimum correction: reject nonempty supervision errors as well as timeout/nonzero
rc at both builder stage gates, retaining the original receipts and stopping
subsequent product stages. For builder/image cleanup, append a typed failure for
those receipt conditions and continue other owned cleanup attempts. Do not overwrite
`original_rc`, turn observer failures into product failures, or add broad cleanup.
Verification needed before handoff: small fake-stage regressions returning rc0 with
recorded supervision errors at create/bootstrap must prevent the next stage and
acceptance; cleanup error/timeout receipts must prevent acceptance while preserving
rc0 and remaining cleanup attempts. The existing rc7/rc0 and observer-reap checks
must remain. No general-purpose supervisor framework is required.

ENV1 blocking defects: **none**. Other actionable defects found: **none**.

## Frozen snapshot and inspected coverage

All five assigned hashes matched before and after review; no reviewed input drift.
`P` below is `target/r1-runtime-fxi6ltmv/smoke-full-preparation-c1ddee5b4e/`.

| File | SHA-256 |
| --- | --- |
| `scripts/qa/candidate-web-e2e-runner.py` | `c8c7e32631fae2d392c19ea059e34314d46d10fbda517738abce92398b23afb4` |
| `scripts/qa/test_candidate_web_e2e_runner.py` | `592039b4047f0ef5e39d6ea76826786619f16833619868cc02a0d92dd537633f` |
| `P/supervise.py` | `9fe20bb9c9ea3523f38c3318f3609d8b3785230d9b33590f9f60e6f4e1e999b0` |
| `P/later-full-plan.md` | `9d083d381f774990d6bf094e204465c2c26a9a7a9c4aa82f72853aadd758de10` |
| `scripts/qa/research-worker-smoke.sh` | `72df4e8f93e8f97399ed425bafc4edaf352d30545663adf3c76df6daa0c51c0f` |

Read ENV report1–18, browser diagnosis82–115, layout1–52 (hash
`7d2680ae0d5440f7d67e628b412c77fb7bec352d3a40495a1ca85f44ea2f36ce`),
runner1440–1495/1515–1565/1615–1707, regression1540–1612 and its entire scoped diff.
Read all supervisor1–303, plan1–18, fake-check source1–25, checks/final-syntax JSON,
three fake result receipts and both preparation manifests. Read smoke report95–114
and historical Stage1 disposition; supporting official source1–58/390–489/612–665/
1120–1143/1360–1415 plus targeted whole-file searches; Dockerfiles collectors1–110
and job-queue1–75; `.dockerignore`1–90; root Cargo1–70 and probe manifest1–14.
Compose282–408 and build/dependency locations were checked as supporting context.
Repository AGENTS.md and roadmap1–75 were read; R1-06/R1-07 remain pending.

## Preserved contracts and actual read-only checks

- Removing only runner1460 reproduces accepted SHA
  `afc6a13ee0f05c4bd210b7dd1c53f9e6e509cbce13706c2d08d52287d760f789`.
  Regression preimage `web-port-isolation-20261012/after.py` matches accepted
  `448b63bb80f9e1c3dea606ce64f2e297c2b3fcfd79e9e8f06872fe8fcebd6156`;
  diff contains only the environment assertion block1573–1585 (13 added physical
  lines, versus the assignment's approximate15). All31 test names are unchanged.
- Literal opt-out1 reaches inventory, synthetic API, both Next lanes and browser
  children through the existing minimal_env callsites. Layout12–13/36–48 uses
  that exact value to omit both development CDN scripts. The four ambient values
  and three environment selections are checked without inheriting the sentinel.
  Provider-free guards, inventory, lane identity, owned cleanup and ports are
  byte-preserved outside that insertion. The diagnosis's52/53/5 counts are retained
  coordinator evidence, not a fresh trace audit or proof that body assertions pass.
- Supervisor logs/results open before launch; finally waits/reaps and fsyncs exact
  rc/timing. Optional inspection failures disable only observation; exact-ID
  missing errors are distinguished from permission/parsing/other failures. The
  prestarted Unix stream retains only matching PID/project volume names/actions/
  timestamps, excluding Actor.Attributes/full payloads. No foreign signal path,
  broad kill/prune, force global deletion, provider or foreign33001 access added.
- The2400-second deadline is a graceful TERM request after PID/PGID/session/start
  verification, followed by wait/reap; it is not a hard completion bound. This is
  the adopted plan, not a new KILL requirement. Cleanup failures remain failed
  evidence for coordinator recovery; the private preparation does not authorize
  execution. Dedicated builder/private config, pinned BuildKit, no --use, live4GiB/
  swap-total4GiB and CPU-cap checks, synthetic refs absent-before and removal without
  force are present. Official builds1131–1134 remain serial one-service calls.
- Filename-only manifest comparison found no omitted safe input under all18
  collectors and16 job-queue Docker COPY sources. Standalone probe manifest/lock/
  source and all three adopted factor inputs are included; secret/env/key exclusions
  were inspected without reading real secrets. All nine dated preparation input
  hashes still match. This observation is not the future execution manifest.
- Host `CARGO_NET_OFFLINE=true` applies to the official manual host cargo run; it
  does not imply all Docker image builds are offline. Raw-probe locked registry
  preparation has only read-only source/private cache mounts; its separate compiler
  uses locked/offline/network-none with private home/target/binary volumes. Freshness
  uses absent-before names/create events/dedicated mounts, not direct empty-directory
  enumeration. Real cache sufficiency and mount observations remain unverified.
- Read-only rg/sed/git metadata, hashlib/difflib and AST parsing only: three Python
  ASTs parsed; no reviewed module was imported/executed. Preparation manifest SHA
  `3689110df33b6edf64fffcfca66e374c21553d90e41606386be3cb36213a4607`
  matches all14 entries. Retained fake receipts prove recorded rc7 plus ValueError
  observer failure, clean rc0, and startup FileNotFoundError/rcnull. Fake source
  covers scoped event redaction/exact missing-ID classification. The recorded first
  fake-check exit0, second existing-directory refusal exit1 and syntax-only exit0
  were inspected, not rerun; they do not cover S1 or real Docker compatibility.
- Historical Stage1 manifest SHA
  `537ad6a1f768fcfb34a854541626a3d0a63ec9aac908c1fd02e1102f9b1cd8ff`
  matches all15 retained entries. PASS/cleanup does not recover its lost exact exit.
  The accepted later full run naturally repeats the new-volume probe; no redundant
  raw-only rerun is requested.

## Limits, ownership and follow-up

Only owned write: this new report, lines1–end; no source or other report edit.
Deviations: physical insertion is13 lines; no scope/execution deviation.
Missing inputs: **none for this bounded source review**. Unread scoped hunks:
**none**. Unrelated unchanged supervisor/official smoke bodies and moving TS/Rust
were not broadly re-reviewed. No runtime/resource/process/network probe performed.
Not verified: S1 correction/regressions, ENV1 unittest/browser outcomes, real stream/
inspect/builder compatibility and caps, future complete before/after source manifest,
full cold/functional smoke and owned cleanup, kernel OOM history, hosted CI/overall R1.
Further decomposition: **none**; S1 is a bounded private-supervisor follow-up.
**REVIEW_COMPLETE; idle with no owned processes or heavy slot.**
