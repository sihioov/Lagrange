# R2 daily F1 source repair — 2026-10-13

## Scope

This records the bounded WP-3/F1 source repair for R2-03. The checkout began at
`cea7baa6298402db79d655140ad4c199f57f8697`, with the C3 daily/backfill source
unchanged. The close guard's filtered list was not preserved across the daily
wrapper/backfill boundary: daily passed only the full civil range, and backfill
reconstructed every calendar session not already `PUBLISHED`.

## Reproduction

`bash scripts/ops/backfill-production-self-test.sh --reproduce-before-fix`
stages the wrappers and helpers from the pinned baseline and drives the real
backfill selection code with fake date, calendar, DB, config, and Docker
dependencies in a temporary checkout. It reproduced:

- At 16:29:59 KST, daily reported `exact_missing_session_dates=2026-10-12`.
- The baseline backfill's final `research-worker --backfill-session-dates`
  argument was `2026-10-12,2026-10-13`.
- The test used no live Docker, KIS/provider, production config, or host state.

The captured reproduction is also kept in the private ignored artifact
`target/r2-daily-beta/wp-3/baseline-reproduction.txt`.

## Repair

`backfill-production.sh` accepts an optional `--session-dates` CSV. Before its
execute config gate or protected state operations, it validates a supplied
filter as nonempty canonical ISO dates, strictly increasing and unique,
within the requested range and validated XKRX selection, with at most 10,000
dates and the existing 1,000,000-byte argv bound. It applies that subset before
reconstructing pending work. With no option, selection remains the full
validated XKRX selection.

`kis-daily-production.sh` passes the exact CSV already produced from its
validated, oldest-first pending sessions after the before-close filter. The
full-range V4 identity, canonical daily state filename, published-state skip,
failure/retry handling, lock, token window, recovery flow, and 74/75 exits are
unchanged. Today is not marked `PUBLISHED` to suppress it.

## Verification

The fake-host integration self-test captures the final worker argv through the
real backfill selection path. It covers:

- 16:29:59 excludes today when an older hole exists; 16:30:00 includes today.
- Today-only before close and a no-missing range make no worker call.
- Interior holes remain exact; repeated execution preserves V4 state and runs
  newly eligible today.
- Invalid, empty, malformed, out-of-range, non-session, duplicate, unsorted,
  and over-10,000-date filters fail before state creation, config validation,
  or Docker calls; default no-filter selection is unchanged.
- Permanent `FAILED` refuses auto-resume, and retryable/deferred results
  propagate as 74/75.

Commands and results:

- `bash -n scripts/ops/backfill-production.sh scripts/ops/kis-daily-production.sh scripts/ops/backfill-production-self-test.sh scripts/ops/kis-daily-production-self-test.sh` — PASS.
- `bash scripts/ops/kis-daily-production-self-test.sh` — PASS; includes the new fake-host integration self-test.
- `bash scripts/ops/backfill-production-self-test.sh --reproduce-before-fix` — PASS; confirms the pinned baseline still exhibits F1.

## Boundaries and remaining work

No deviations from the authorized F1 contract. No production host, actual
calendar artifact, real config validator, database, Docker daemon/container,
KIS provider, deployment, service activation, or data publication was exercised.
This is source repair and isolated verification only; it does not satisfy
operational R2-03 acceptance, R2 data receipt, or Owner acceptance. No further
decomposition is needed for this bounded repair.

The shared checkout's HEAD advanced during this work to `7920ba9`, a documentation
commit. I did not create or amend a commit, push, stage, or modify the separate
Rust files shown by `git status`; those and roadmap integration remain outside
this worktree scope.
