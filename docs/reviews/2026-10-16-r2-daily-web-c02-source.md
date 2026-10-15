# R2 Wave69 I7-C02 fixture and browser recovery

Roadmap R2-04/09. Baseline3e4cdb3 after complete CI38823547944 failure recovery.
Direct coordinator source review, not independent review. Four owned files;
product/API/backend, schema guards and production browser components unchanged.

The FAILED synthetic run now carries the existing fixed backend code
OWNER_BETA_COMPUTATION_FAILED. PENDING/SUCCEEDED omit the code. The existing
history browser case first reads the real synthetic HTTP detail through the
production schema, checks the exact status/items/code and exercises both
missing-failure-code and unexpected-pending-code rejection. Pagination,
previous successful result and manual historical distinction remain asserted.

The configuration-switch test retains a real synthetic API response via
route.fetch (5second bound), then fulfills after navigation to the other config.
A one-shot URL predicate binds origin/path/config. Interception/completion use
5second assertions; try/finally always releases the gate and removes the route.
Clicks do not wait on the held request. A rejected delivery must have an actual
request cancellation, observed with another bounded assertion. Disabled state,
no old date, false preference and absence of the first configuration result
remain required. This is browser navigation isolation, not a claim that a
full-document navigation proves every in-place React scheduling interleaving.
The old timeout's precise operation remains UNKNOWN.

Hosted failure evidence adds only known test-relative file plus positive
bounded line/column, and only for failed/timedOut/interrupted attempts.
Arbitrary prefixes, messages, stacks, bodies and annotations are not emitted.
The existing two evidence tests cover actual retained locations, no prefix/
message/stack leak, unknown files/types/booleans/zero/negative/oversized/missing
positions. Strict supervisor selection, skip/retry/flaky rejection, ownership
and cleanup logic is byte-identical outside the evidence projection.

All18UI names/markers,11 Owner cases (8new),34supervisor definitions remain.
The other9 Owner test bodies and first32supervisor bodies are byte-identical.
Updated browser/supervisor lines are inventoried;129OFF/14stream remain required.
The fixture delta is exactly one field; all prior frozen pins outside4 owned
paths match. Git whitespace and exact byte/name/line/diff checks passed.
No local project parser/compiler/formatter/test/helper/SQL/Docker or provider
execution. All changed tests remain unexecuted for the new candidate. A single
fresh hosted candidate after all-event run query and remote SHA match is needed.
I6, real approved-base/threeUID integration, independent integrated review,
G4/Owner/three actual trading days and finalC_R2 remain open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| apps/web/tests/e2e/support/recommendation-fixture.mjs | 548 | deb17df81cdb5d30112a39406791f69513cb0577ca8898926eefc41409d5cd41 |
| apps/web/tests/e2e/owner-beta-recommendations.spec.ts | 370 | c9319d8ef315ceab92607ab8a14351d718c28a9ea3eef4c15b0ae35198052d96 |
| scripts/qa/candidate-web-e2e-runner.py | 2234 | c047a6cd47437615e9a0bd9e0b305fa1acb59a992acf88221bbb08c23894edee |
| scripts/qa/test_candidate_web_e2e_runner.py | 1847 | 97c0e732f333379d3fb8b979395582f96633ff6eb2b0fedbfaeb1bdcb9fedd1d |
