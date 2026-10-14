# Wave30 exact-candidate hosted CI evidence

- Package / roadmap: WP-4/I1b-S2-CI; R2-04/R2-09.
- Candidate: `245f4d9a911d03d58936f177904588826031d83e` on `excited-dolphin`; remote `sihioov/Lagrange`. Reviewed S2 source commit: `bcc66d327faba34637e05e905197606d6207df39`.
- Existing run: [38594682124, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38594682124), workflow `CI` / `.github/workflows/ci.yml`, event `workflow_dispatch`. GitHub metadata confirms exact head SHA, branch, workflow ID 331572180, attempt 1; the launch receipt confirms the single dispatch and run identity. No second dispatch or retry occurred.
- Final run status: **failure**. Eight jobs were returned on the single jobs page (8 total), including `required`.

## Inputs and candidate identity

The worktree HEAD and remote in the supplied launch receipt match the exact candidate. All 51 input pins were rechecked: 45 candidate Git blobs and six workspace artifact references match both SHA-256 and line count. Missing or conflicting launch/input pins: **none**. A read-only diff against prior I0c commit `a888cd6` shows exactly three non-documentation paths: `source.rs`, the snapshot integration test, and module registration. Those three files have byte-identical blobs at the candidate and reviewed source commit. Launch, input, and test inventory hashes match the supplied values.

- Launch: `target/r2-daily-beta/wp-4-integration/wave30-S2-ci-launch.json` — 62 lines, SHA-256 `225205ad7b05b8e27fd19953d5152bb934888ca1795637a9b23f9115401cc652`, mode 0600.
- Inputs: `target/r2-daily-beta/wp-4-integration/wave30-S2-ci-inputs.json` — 226 lines, SHA-256 `8f96f87e899f15ec6bbf367d5caf64761334cf82fffcd8fffe456bf6d71cfea6`, mode 0600.
- Test inventory: `target/r2-daily-beta/wp-4-integration/wave30-S2-ci-test-inventory.json` — 812 lines, SHA-256 `2446f369c0a00514d82f60f02a87203c3f1841efa335b8b06772b1c17130e8c0`, mode 0600.

## Job outcomes

| Job | Outcome | Relevant steps |
| --- | --- | --- |
| `workspace-tests` | FAIL | Chart HTTP acceptance failed; workspace Cargo test step was skipped; QA compose cleanup succeeded. |
| `web` | PASS | OpenAPI, lint, typecheck, test, build, and E2E steps all succeeded. |
| `policy` | PASS | All listed policy steps, including its self-test wrapper, succeeded. Nested subcase results are not established. |
| `postgres-integration-validation` | FAIL | Validator step failed; sanitized evidence and failure-log print steps succeeded. |
| `python-tests` | PASS | Pytest and backup-policy test steps succeeded. Suite totals and individual readiness/SIGTERM node results are unknown. |
| `format` | PASS | Exact `cargo fmt --all -- --check` step succeeded. |
| `clippy` | FAIL | Exact `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` step failed. Diagnostic codes and locations were not safely retained. |
| `required` | FAIL | `Require all CI jobs` failed. |

The workspace log contained compiler code `E0599` at `crates/market-data/src/daily_price_only/source.rs:918:35` and `:926:28` during the chart acceptance step. No root cause is inferred. Cargo workspace tests did not execute because the following Cargo step is explicitly `skipped`. The strict Clippy step independently failed; its diagnostics are unknown.

The exact 99 qualified Linux Rust cases are all **NOT_EXECUTED** because the Cargo test step was skipped. Counts: PASS 0, FAIL 0, NOT_EXECUTED 99, UNKNOWN 0. This is 30 wire + 18 S1 Raw + 1 collector + 2 helper + 19 S2 private + 29 S2 integration rows. The six chart rows remain **UNKNOWN** because their explicit command step failed and direct named rows were not safely retained. The S1 non-Unix and S2 non-Linux cases remain `EXCLUDED_LINUX`, not passes. Historical V2/V3 test evidence remains unverified.

## Domain limits and evidence gaps

- PostgreSQL validation failed. The log contained an unpaired `DENIED` marker, which cannot be attributed to `jobqueue-daily-admitter-pool` or `api-daily-admitter-pool`. Per-label APPROVED/DENIED outcomes, configured DB, roles/attributes/privileges, migration 59 checksum equality, migration 0039, no-SKIP gate, cleanup, and nested test rows/exits remain UNKNOWN. A successful evidence-print wrapper does not establish hidden skip absence.
- Python suite totals, quiet readiness, and individual SIGTERM nodes remain UNKNOWN despite successful wrapper steps. Policy nested subcases and fakeroot skip-message evidence remain UNKNOWN.
- Web steps passed individually, including E2E. No additional claim is made about runtime or Owner acceptance.
- No negative bootstrap/rollback readiness, migration 0060, provider/copy reconstruction or admission, non-Linux/32-bit/cross-UID/ACL/mount coverage, runtime, Owner acceptance, or final C_R2 evidence is claimed.
- No actual checkout SHA was recoverable from the safely retained checkout logs; all seven execution-job checkout SHAs are UNKNOWN.
- Logs were fetched once per completed job. Raw bodies were not persisted. Full raw size/hash were retained only for PostgreSQL (152,469 bytes; `83effe2485a0b52afb003772c933c4ccac3cd794761004218041ae998ceb3b08`), web (96,734 bytes; `b79757859ea870ae88a0eb1bb30ee959b8c14a8b73a182ffec41a6193c6c731b`), and required (3,069 bytes; `102faeeea916505bebc43279dddd71e44eba7875b940603bacbe68b9248ccfdd`). The workspace log size was 100,583 bytes but its digest was not computed before the one-shot in-memory extraction ended. The full bodies/hashes for policy, Python, format, and Clippy were not retained. No log was fetched a second time.

## Collection method and changes

Read-only commands/checks included `cat AGENTS.md`, `cat docs/ROADMAP.md`, `sed -n '3418,3495p' docs/superpowers/plans/2026-10-14-r2-daily-input-implementation.md`, `git status --short --branch`, `git rev-parse HEAD`, `git remote -v`, `git diff --name-status a888cd6..245f4d9`, `git show <candidate>:<path>` and `git show <reviewed>:<path>` for pinned blobs, `sha256sum`, `wc -l`, `stat -c`, `rg`, and `gh api repos/sihioov/Lagrange/actions/runs/38594682124`, its `/jobs?per_page=100&page=1`, the candidate `/check-runs?per_page=100`, and each returned `/actions/jobs/<id>` metadata endpoint. GitHub connector job-step and log retrieval was performed once per relevant job; inert JSON pin checks used inline Python. The report and private receipt are the only writes by this collector. The initial worktree status was clean at the candidate SHA. During collection, coordinator documentation commits `be07c9c` and `37c2955` advanced local HEAD to `37c29556dc50f52c12afa055fa065bd25e4b1ec6`; `origin/excited-dolphin` remains at candidate `245f4d9a911d03d58936f177904588826031d83e`. Their diff is limited to the roadmap, status, execution review, and implementation plan. This collector did not create those commits, edit those protected files, commit, or push. No local Cargo/rustc/Clippy/tests/build, project helper/checker/parser, repository script, SQL/DB, Docker, or production/provider operation was run. No source repair or Git write occurred.

- Review: `docs/reviews/2026-10-15-r2-snapshot-ci.md` (this file).
- Private evidence: `target/r2-daily-beta/ci/245f4d9/ci-evidence.json`.
- Private artifact directory mode 0700; receipt mode 0600. No background work, pending permission, automatic-review denial, or further decomposition.
- Mem0 context lookup failed with a sanitized local MCP response error; it was not retried under a guessed scope.

R2-04 and R2-09 remain DOING. The hosted candidate has no compiler/test acceptance; the remaining negative, migration, full input linkage, integration, operation, Owner and final C_R2 gates remain open.
