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

## WP-3/F1 P2 fixture-isolation follow-up — blocked

The accepted independent review identified inherited `KIS_DAILY_STATE_FILE`
and `KIS_DAILY_EARLIEST_TODAY_KST` overrides, caller-selected `TMPDIR`, and
overbroad fake ownership results in `backfill-production-self-test.sh`. The
requested clean environment, canonical fixture state path, fixed 16:30 cutoff,
fixture-local temporary directory, resolved-path fake checks, and hostile
sentinel regression are not implemented in this follow-up.

Static inspection found that the strict fake-path requirement conflicts with
the existing daily execute guard in this non-root test environment. The daily
guard walks every existing ancestor of its state directory up to `/` and
checks each with `stat` (`kis-daily-production.sh:245–286`). A fixture under
the test's required temporary root reaches the real `/tmp`, whose observed
metadata here is `0:0:1777`. Passing through real `stat` correctly causes the
guard to reject `/tmp` as group/other writable. Returning synthetic root-owned
safe metadata would certify a resolved path outside the current fixture, which
the accepted P2 explicitly forbids. No production guard or external-path fake
was weakened to force the test through.

The existing test root also currently follows inherited `TMPDIR`
(`backfill-production-self-test.sh:16`), so its current integration test was
not run: doing so before fixing staging and path certification would exercise
the unbounded harness identified by the review. The required exact worker
arguments and byte-identical hostile sentinel therefore remain unverified.
The self-test needs a test-only filesystem setup in which the real ancestor
metadata satisfies the daily guard, or a separately reviewed fixture mechanism
that preserves that guard while keeping fake ownership certification inside
the fixture. This needs coordinator scope/design resolution before code repair.

No F1 production script, roadmap, or pre-existing review text was changed.
`bash -n` was run for the two self-test scripts only. No fake-host integration,
daily self-test, Docker, provider, DB, production state, Cargo, root, lifecycle,
or Git staging/commit/push action was run.

## WP-3/F1 P2 fixture-isolation resolution — 2026-10-13

The coordinator resolved the mock-boundary question without changing the
production daily guard. The earlier blocked section above is retained as the
decision history; this section records the approved test-only ancestor mock
and the completed isolation repair.

`backfill-production-self-test.sh` now creates its staging root with the fixed
`/tmp/lagrange-backfill-self-test.XXXXXX` pattern and validates that the result
is the expected canonical real directory before using it or cleaning it up.
Each case has its own `tmp` directory. Every daily and backfill wrapper is
started through `/usr/bin/env -i` with an explicit allowlist, fixture-local
`TMPDIR`, an explicit canonical `KIS_DAILY_STATE_FILE`, and
`KIS_DAILY_EARLIEST_TODAY_KST=16:30`.

For each case, the test precomputes the canonical directory ancestor chain from
`FIXTURE_ROOT` to `/`. The fake `stat` requires the requested ancestor to pass
real `-d`, resolve exactly to its canonical spelling, and match an entry in
that list. Synthetic `0:0:755` metadata is returned only for that chain and
only for the daily guard's `-Lc '%u:%g:%a' -- PATH` directory format. This
allowlist can include `/tmp` and `/`; those values model the test contract and
are not evidence about host ownership or permissions. All fake file metadata
requires the resolved file path to remain inside the active case, including
resolution of `/proc/.../fd/...` lock targets. Fake `chown` likewise requires
the resolved, non-symlink target to be inside the case. Unsupported stat forms,
the outside-case sentinel file, and its sibling directory are rejected.

The pre-close regression inherits `KIS_DAILY_STATE_FILE` pointing to a
test-owned sentinel outside the active case and a hostile `00:00` cutoff. The
allowlisted child environment pins the canonical daily state and 16:30 cutoff.
The test confirms byte-identical sentinel contents, canonical state filename,
the exact `2026-10-12` worker argument at 16:29:59, the exact
`2026-10-13` argument at 16:30, unchanged V4 identity, and successful repeated
execution that admits newly eligible today. Earlier today-only, no-missing,
interior-hole, FAILED, 74/75, invalid-filter, and default no-filter cases
remain in the same fake-host suite.

Commands and results:

- `bash -n scripts/ops/backfill-production-self-test.sh` — PASS.
- `bash -n scripts/ops/kis-daily-production-self-test.sh` — PASS.
- `/usr/bin/env -i PATH=/usr/bin:/bin HOME=/tmp LC_ALL=C bash scripts/ops/backfill-production-self-test.sh` — PASS.
- `/usr/bin/env -i PATH=/usr/bin:/bin HOME=/tmp LC_ALL=C bash scripts/ops/backfill-production-self-test.sh --reproduce-before-fix` — PASS; reproduced the baseline pre-close argv leak.
- `/usr/bin/env -i PATH=/usr/bin:/bin HOME=/tmp LC_ALL=C bash scripts/ops/kis-daily-production-self-test.sh` — PASS; includes the fake-host integration suite.

This follow-up changed only the owned self-test, this appended report section,
and the private receipt; the coordinator's roadmap/integration edits and
concurrent Rust/review changes were left untouched. No production helper bytes
changed. These checks used
synthetic fixtures only: no Docker daemon/container, DB, provider, production
state/config, Cargo, root operation, service lifecycle, or Git write action was
performed. Operational R2-03 acceptance remains separate and unverified.

## Coordinator source acceptance — 2026-10-14 KST

The corrected source is pinned at `f8b9296c14cb48dacdf8cacefab6c2abff92bf24`.
[CI 38420816275](https://github.com/sihioov/Lagrange/actions/runs/38420816275)
completed successfully, including policy and the required aggregate.
[Independent delta review](2026-10-14-r2-fixture-review.md) closes P2-F1 at the exact
`ccbd3762dfa5d2c0aa1503b01180f498f909e96d3578f48d916b99738448d5b0` bytes.
Earlier failures and the temporary fixture-model blocker above remain historical evidence.
F1 source repair and isolated verification are accepted. Actual collection,
deployment and R2-03 operational acceptance remain unperformed.
The [CI evidence report](2026-10-14-r2-fixture-ci.md) distinguishes successful
policy steps from unavailable individual shell-result markers; the local
fake-host/daily self-test evidence above supplies those direct results.
