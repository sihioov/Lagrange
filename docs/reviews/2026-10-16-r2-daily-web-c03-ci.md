# R2 Wave70 daily Web C03 complete CI recovery

Roadmap R2-04/09. Candidate f1f55ac9cc268f7531bcf08f409e7770767b62d5;
[CI38829047490 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38829047490).
One dispatch, all eight terminal jobs/full log hashes, seven actual checkout
SHAs,853pins and419candidateblobs recovered and matched.

Six execution jobs passed; Web/required failed. All31 required Rust groups
and chart6 passed. All18 dedicated DB targets exited0/noSKIP;123 named rows,
both62 migration checksum sets,0039,bootstrap83,daily32/subcase73,catalog,
no-op rerun and cleanup passed. Unprinted nested helper rows/exits remain UNKNOWN.

Web OpenAPI/lint/typecheck/unit/build passed. All18 daily UI markers,6carriedWeb
and34supervisor names passed. OFF129 had128passed/1failed;stream14passed.
No skips/retries/flaky results; both cleanup and reservation gates passed.
Only configuration-switch case273 failed: the fixed phase is
R2_DAILY_SWITCH_VERIFY, location384:5 (the step boundary). No inner assertion
location was recovered. All preceding bounded phases completed successfully.

Concrete source inspection shows a contradiction in the test's whole-panel
negative date assertion. The second config has five null session/result/due
fields but still has a legitimate observed_at=2026-10-15T08:01:00Z. The panel
renders that observation separately. Rejecting Oct15 anywhere in the panel
therefore rejects the valid observed time, even with correct config isolation.
This source contradiction is verified; no unavailable inner stack is claimed.

C04 replaces only that assertion with exact Not reported values for all five
session/coverage/result/publication/due fields and positive observed-time
checks. OFF, false preference, no other-config report, actual late response,
control identity, request cancellation and all existing bounds remain required.
No application/fixture/runner changes, local execution or new delegation.
I7 acceptance still pending. I6/approved-base threeUID/integrated review/G4/
Owner/three actual trading days/finalC_R2 remain open.

Private receipt: target/r2-daily-beta/ci/f1f55ac/ci-evidence.json;9195lines;SHA256 32a2fa8ab31103a056f774fe997343b9f73d01da7959d178db7ac10b43c9b60d.
