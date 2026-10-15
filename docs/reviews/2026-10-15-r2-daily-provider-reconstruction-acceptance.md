# R2 I1b-S4 — provider reconstruction acceptance

Accepted 2026-10-15 for R2-04/09, limited to offline provider reconstruction.
Candidate `302f29a2d6e9ce8502813a0b1cc0b9ef370fcdea`,
[run38725299504 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38725299504),
CI/ci.yml workflow_dispatch on excited-dolphin, completed SUCCESS. The all-event
exact-head query returned0 and remote SHA matched before the single dispatch at
11:09:10UTC. No rerun/cancel/duplicate occurred. The prior48beb25 failure and its
passing tests remain in the [first CI record](2026-10-15-r2-daily-provider-reconstruction-ci.md).

All eight terminal jobs and full log digests were recovered. Seven execution
checkouts match; required has no checkout.276 frozen input pins and223 committed
blobs match.83 steps passed; only conditional PG failure-log output was skipped.
Workspace formatting and strict workspace/all-target/all-feature Clippy with
-D warnings passed, as did policy, web/OpenAPI/lint/typecheck/tests/build/E2E,
Python485 pass/10 skip/7 warnings and required.

Individually matched names:32 new S4,22 S3,37 normalizer/calendar,99 prior R2,
32 daily SQL,28 affected legacy DB,3 transport and6 separate chart cases all ok.
These inventory groups may overlap and are not a summed unique-test total.
Dedicated PG additionally matches32 daily names/73 unique subcases,83 bootstrap
names and12 outer DB exits0/no-SKIP. All60 candidate UP SHA384 checksums match
in both lanes; rerun NO_OP, direct roles,0039/catalog/0060 rollback and cleanup
PASS_NO_OWNED_CONTAINERS pass. Unprinted nested helper rows/inner exits remain
UNKNOWN; two carried non-Linux exclusions and Python skips remain explicit.

The final sanitized receipt is `target/r2-daily-beta/ci/302f29a/ci-evidence.json`,
2835 lines, SHA256
`aded5ef10f56551258ff86d4ee4dd38866da08b7b26b605c41b91901521b661f`.
Raw logs stayed in memory. The source comparison proves C01 changed only the
fixture tuple representation and two consumers; all production S4 bytes remain
those previously reviewed. The coordinator's [source disposition](2026-10-15-r2-daily-provider-reconstruction-source-review.md)
and [C01 disposition](2026-10-15-r2-daily-provider-reconstruction-c01.md) are direct,
not a new independent review.

Accepted behavior is exact11 price/reference reconstruction and complete7/77
KSD interval/cell validation, cash-only output, deterministic full-row overlap
selection and retained source commitments. It grants no source-frontier seal,
protected-prefix/artifact/admission/supply capability or runtime authority.

S5 observation/discovery preparation was outside the tested candidate and is
not covered by this acceptance. Next is its integration, then protected prefix,
artifact/admission/supply, I3–I7/final independent review, runtime UID/ACL,
installer filesystem rollback, I6 diagrams before main, operational activation,
Owner use and three consecutive trading days/final C_R2. R2-04/09 remain DOING.
No local project execution, new worker/delegation or provider/production action
occurred. Ralph stays active; the observation period has not started.
