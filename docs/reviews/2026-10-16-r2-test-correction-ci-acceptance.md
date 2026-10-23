# R2 Wave51 exact-candidate acceptance

Completed 2026-10-16 KST. Candidate
`bd33596f515afe18437dad03bd940219c630d248`;
[CI 38760423300 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38760423300)
completed SUCCESS at 2026-10-15T15:49:32Z. Repository `sihioov/Lagrange`,
branch `excited-dolphin`, event `workflow_dispatch`, workflow `.github/workflows/ci.yml`.
The launch followed zero existing all-event runs and an exact remote-head match.
No prior or current run was rerun, canceled or duplicated.

All eight terminal jobs and their complete logs were recovered; seven execution
checkout logs name the exact candidate. All 423 frozen pins and 302 committed
candidate blobs match. There are 85 successful steps and one expected skipped
failure-log-printing step in the successful PG job. No Rust test failed.

## Boundaries accepted

The [test corrections](2026-10-16-r2-test-expectation-corrections.md) are accepted
for this exact hosted candidate. I3 daily factor/target and I1b-S5c/def artifact,
continuation, source-frontier and pure supply implementation pass their bounded
source/hosted gates. Source review was direct coordinator work, not independent.
The earlier [Wave50 failure](2026-10-16-r2-daily-factor-target-ci.md) and Wave46-49
failures remain preserved; they are not relabeled as successes.

Each required result name was matched exactly once within its execution phase.
The groups overlap; their counts are not summed into a unique test total.

| Required group | Named results |
| --- | ---: |
| I3 daily computation | 21 PASS |
| Historical factor/input/target computation | 29 PASS |
| S5def continuation/frontier/supply | 37 PASS |
| S5c artifact/admission | 26 PASS |
| Affected service status reads / service status | 5 / 33 PASS |
| Paper clock cases | 2 PASS |
| S5b / S5a | 21 / 33 PASS |
| S4 reconstruction / S3 lineage-calendar | 32 / 22 PASS |
| Affected normalizer | 37 PASS |
| I2 daily SQL definitions / affected legacy | 32 / 28 PASS |
| Prior direct Rust / transport | 99 / 3 PASS |
| Separate chart invocation | 6 PASS |

The actual workspace command, all-target/all-feature strict Clippy with
`-- -D warnings`, workspace formatting, Web OpenAPI/lint/typecheck/tests/build/E2E,
policy job and Python job all passed. Python reports 485 passed / 10 skipped /
7 warnings. Existing platform exclusions and skips remain separate. The protected
artifact fixture's root/anchor/runner identities and nonrecursive cleanup passed.

Dedicated PostgreSQL evidence independently proves all 32 daily names, 73 fixed
subcases, 83 negative bootstrap cases, all 13 outer DB exits zero without SKIP,
and all eight actual-role owner-beta publication names. Both full-release lanes
match the candidate's 60 UP-migration SHA384 checksums. No-op rerun, direct-role
logins, 0039 refusal, catalog assertions and owned-container cleanup passed.
Unprinted nested helper test rows and inner exits remain UNKNOWN.

## Evidence and limits

Private sanitized receipt: `target/r2-daily-beta/ci/bd33596/ci-evidence.json`,
4,433 lines, SHA256
`8bc9980b9a87acac4174899feba0549a8da7ed7b2a120c79ad61ce7e629263ee`.
Workspace log digest:
`f42548c5320fdeb7b5d45bfc0c2c6647cb1490446e8ddf467f23a12963ecd29c`;
PG log digest:
`f78d296e0890d6b5f6c568b12436f7cf7a26114a4dbc75443ab430179fab6a44`.
All eight log digests, qualified names, line references and checkout observations
are retained in the private receipt. Raw log bodies were not persisted.

The service-state correction handles quotes predating the actual KST session
open. This green run does not prove that the narrow midnight branch executed;
that limit is preserved rather than replacing time with a test clock.

The real approved-v3 base through a complete first candidate and multi-day
continuation, distinct-UID admission/readback, actual caller-state supply,
crash/fsync/rename faults and UID/ACL/installer rollback still require integrated
evidence. I4-I7, integrated independent review, diagrams before main, concrete
G4 supply changes, operational activation, actual Owner use, three trading days
and final C_R2 remain open. The observation period has not started. Ralph
continues with [I4a cycle coordination](2026-10-16-r2-daily-cycle-coordination.md).
