# R2 I1b-S5a — complete first candidate CI recovery

2026-10-15; candidate `5a83e0d978444201237d404307f3e7ea8e79f037`,
[run38728478625 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38728478625),
CI/ci.yml workflow_dispatch on excited-dolphin: terminal FAILURE. One dispatch,
no rerun/cancel/duplicate. All8 jobs/full log hashes and7 exact checkouts recovered
before source changes.287 frozen pins/231 candidate blobs match. S5a is not accepted.

Format, strict workspace/all-target/all-feature Clippy -D warnings, policy, Python,
web and PostgreSQL passed. Workspace-tests and required failed.31 of33 S5a named
cases passed. The two failing private observation cases are:

- `s5_observation_commits_metadata_and_time_as_well_as_body`: observation.rs437.
- `s5_observation_copy_keeps_original_and_delivery_identities`: observation.rs451.

Both fail at fixture construction with `InvalidTimestamp`: the positive fixtures
use second-only strings with DailyWireTimestampV1, whose existing D4 contract
requires exactly nine fractional digits. The mutation/copy assertions are not
reached in those cases. This is a concrete fixture defect, not a passing assertion
or evidence to relax the timestamp parser. C01 adds `.000000000` to those two
test timestamps only; the production observer and timestamp validation stay intact.

Each required carried name matched once:32S4,22S3,37normalizer,99prior Rust,
32daily SQL,28affected legacy DB,3transport and6separate chart all ok. Groups may
overlap and are not added into a unique total. Dedicated PG passes32daily names,
73unique subcases,83bootstrap names,12outer DB exits0/no-SKIP, both lanes'60 exact
UP SHA384 checksums, NO_OP rerun, direct roles,0039/catalog/0060 rollback and
PASS_NO_OWNED_CONTAINERS cleanup. Unprinted nested rows/inner exits stay UNKNOWN.
Python485pass/10skip/7warnings and two carried Linux-excluded cases remain explicit.

The sanitized receipt `target/r2-daily-beta/ci/5a83e0d/ci-evidence.json` has3064lines,
SHA256 `2a46e2a6927c9935df2fa2ea8f909d50e4cb5b42d3ca64cd124aa78b706587b6`.
The fixed-field `test-failure-supplement.json` binds both InvalidTimestamp panics
to the same workspace log SHA256
`2b64003c07270c3bc97410d6de1edec04103b8116e1c5900594e056490854562`.
Raw logs remained in memory. Source recovery completed before C01 or registration
of the independent history/candidate preparation. Prior S4/S3/I2 acceptances stand.

Next candidate combines the two fixture corrections with S5b's prepared first
protected historical extension and in-memory candidate. It requires a new frozen
inventory, direct source disposition, exact-head existing-run query and one fresh
dispatch. All later parent/seal/storage/admission/supply/runtime/Owner/final C_R2
work remains open. No local execution, new delegation or provider action occurred.
