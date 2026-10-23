# R2 I6B2a release database control verification

Roadmap R2-04/R2-09. Candidate `570e277fed776371d526ef56d11c6a580746210e`.
[CI38839505678 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38839505678)
completed successfully. All eight terminal jobs/full log digests and seven actual
checkouts were recovered; 945 fixed pins and 443 committed blobs matched.

The 11 new direct-role DB control cases passed in the workspace and dedicated PG
lane. The PG lane required all 43 schedule names with no SKIP, plus the remaining
18-target contract: 134 named rows, 62 migration checksums in both installations,
0039 guard, bootstrap83, schema daily32/subcase73, catalog, no-op rerun and cleanup.
Six actual-root control-render cases, sixteen filesystem and twelve installer
cases, the legacy operations fixture and three Rust parity cases passed.
All 32 required Rust groups and separate chart6 passed; strict Clippy and workspace
format passed. Web UI18/contracts6/supervisor34, OFF129/stream14 and exact Owner11/
daily8 browser rows passed without skip/retry/flaky acceptance.

Private sanitized receipt: `target/r2-daily-beta/ci/570e277/ci-evidence.json`,
9574 lines, SHA256 `1e6cefe3368af0d4e18fb7fd85bfd172e724c1285178b99a2e604fa4b1d4d316`.
Raw logs were retained only in memory. Unprinted nested helper rows and inner
exits remain UNKNOWN; aggregate counts are not unique named-test evidence.

Bounded release database control acceptance only. No production activation,
actual container policy/drain, approved-base whole flow, independent integrated
review, G4, Owner use, three actual trading days or final C_R2 acceptance.
All earlier failures and accepted limited results remain preserved.
