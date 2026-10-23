# Wave49 — complete single-candidate CI recovery

Candidate `edeaa5db456bc4c62e6eef6e7a561e6a49accff4`,
[run38750900179 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38750900179),
CI/ci.yml/workflow_dispatch/excited-dolphin: **FAILURE**, all8jobs terminal.
The coordinator recovered all8 full log bodies in memory,7 exact checkout SHAs,
348 frozen input hashes and261 committed candidate blobs. No raw logs persisted.

| Evidence | Disposition |
| --- | --- |
| workspace tests / Clippy | Both fail E0603 at frontier.rs:875:70 and905:84: continuation::tests::fixture is private. Declaration continuation.rs:599 used pub(super), granting access only within continuation. |
| Workspace execution | Workspace Cargo command observed, no test-binary start and no workspace named result. Compilation failed before test execution. All required workspace groups NOT_EXECUTED, including37S5def,26S5c and2Paper clock cases. |
| Chart | Separate6 named chart tests PASS before the workspace step. |
| Protected fixture | `/` uid0/0755, direct-root anchor uid0/0755, runner child uid1001/0700 observed. Setup and nonrecursive cleanup PASS. Artifact behavior tests remain unexecuted. |
| PostgreSQL | PASS:32 daily named tests,73 fixed subcases,83 bootstrap cases,12 outer exits0/noSKIP,60 migration UP SHA384 checksums in both lanes, NO_OP, direct roles,0039 guard,catalog checks,cleanup. Unprinted nested helper rows/inner exits UNKNOWN. |
| Other jobs | Web, format, policy, Python PASS. Required aggregate FAIL. |

The immutable private receipt is
`target/r2-daily-beta/ci/edeaa5d/ci-evidence.json`,3350lines,
SHA256 `4cf38abd73e5abc4b7539974847a06fd36ad589153641200856acd7da2d6dbd0`.
It contains each required name, all job steps, sanitized diagnostics and log hashes.
The workspace log was read again only to reconcile the execution stage; its complete
body hash matched the first retrieval. Historical failures and accepted5f5f4e8 remain.

After complete recovery only, S5def-C01 widens this one **cfg(test)** fixture function
to `pub(in crate::daily_price_only)`. Production continuation bytes remain identical.
No new runtime/public production constructor, source guard change or assertion change.
The successor combines this correction with I3's separately prepared computation path.
No rerun/cancel/duplicate or acceptance of this failed candidate. All R2 operational,
Owner, three-trading-day and final C_R2 conditions remain open.
