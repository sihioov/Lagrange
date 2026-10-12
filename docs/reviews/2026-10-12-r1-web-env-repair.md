# R1 Web environment correction — 2026-10-12

The [retained-trace diagnosis](2026-10-12-r1-web-browser-followup.md) correlates all 52 failed cases with attempted development instrumentation requests to unpkg. The existing root layout already supports disabling those scripts. The candidate runner creates its own child environment, so an outer environment override alone is ineffective.

The coordinator added `NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS=1` to `Harness.minimal_env`. This applies to inventory, off and market-stream children. No layout, external-request allowlist, fixture, provider boundary or browser assertion changed. The existing environment regression now checks all three selections against absent, zero, one and unexpected ambient values, and verifies an unrelated ambient sentinel is not inherited. The suite still has its existing 31 top-level tests.

| Source | SHA256 after correction |
| --- | --- |
| `scripts/qa/candidate-web-e2e-runner.py` | `c8c7e32631fae2d392c19ea059e34314d46d10fbda517738abce92398b23afb4` |
| `scripts/qa/test_candidate_web_e2e_runner.py` | `592039b4047f0ef5e39d6ea76826786619f16833619868cc02a0d92dd537633f` |

Both Python AST parses and `git diff --check` passed. Runtime regressions and browsers await the next Web slot; the old failures remain failed evidence. This source change does not establish that later body assertions pass.

The coordinator also adopts BODY1/BODY2 from the diagnosis: preserve formatted decimal precision, and prove REQUESTED from the actual UI-triggered POST receipt before observing the four rendered lifecycle states. CLOCK1/CLOCK2 remain bounded scheduling/navigation hypotheses with finite diagnostic receipts, not confirmed product defects. These TS changes are assigned separately.

ACCOUNTING1's optional continue-after-failure change is not adopted. Existing fail-fast behavior correctly explains 134 actual executions versus 135 planned: every required synthetic identity was attempted, but the owner-empty test's second mode was not reached. Final acceptance still requires all 135 executions, all 134 synthetic identities and both owner modes to pass. No selection reduction, retry, skip or failure-to-success conversion is authorized.

No structural dependency or runtime deployment change; diagrams require no change for this correction. No source outside the two listed files, lockfile, staging or commit was modified by this package. [Independent review](2026-10-12-r1-web-env-smoke-review.md) accepted ENV1 after verifying both frozen hashes, the complete delta and all environment callsites. Dynamic regression and browser acceptance remain pending.
