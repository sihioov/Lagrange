# R2 I2 — migration 0060 implementation and hosted acceptance

Completed 2026-10-15 (Asia/Seoul), scoped to WP-4/I2 under R2-04/R2-09.
Accepted candidate: `dee78c4531e1cb14eefd8abb3bf90189d382ca6e`.
[CI 38696988981, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38696988981)
completed SUCCESS at 07:33:13 UTC. **I2 source implementation and isolated hosted
verification are complete. R2-04/R2-09 and final C_R2 remain open.**

## Accepted scope

The [fixed implementation contract](2026-10-15-r2-daily-sql-implementation-contract.md),
[D5 amendments](2026-10-14-r2-daily-input-design-amendments.md),
[D6 disposition](2026-10-14-r2-daily-input-amendments-review.md) and
[D4 wire contract](../contracts/owner-beta-daily-etf11-input-v1.md) remain unchanged.
The implementation adds migration 0060, actual-role DB regression cases and the
official sequential validation harness. It does not activate daily collection.

The accepted database boundary covers ten migration-owned FORCE-RLS tables,
disabled defaults, role preflight, epochs, source/calendar admission projections,
receipt and replay identity, permanent reservation deduplication, queue state
consistency, publication authority, cancellation/recovery and unused-only DOWN.
[Source disposition](2026-10-15-r2-daily-sql-source.md) records the concrete guards
and their trust boundaries. All 118 historical UP/DOWN files through 0059 are
preserved. The coordinator performed source disposition and result reconciliation;
this record does not claim a new independent source review.

## Verification

| Evidence | Accepted result |
| --- | --- |
| Candidate identity | 249 frozen input pins, 204 committed blobs and seven actual job checkout SHAs match the candidate |
| Complete CI | Seven execution jobs plus required SUCCESS; 83 successful steps and one expected failure-log-only skip |
| New DB contract | All 32 named definitions pass in workspace and separately in the DATABASE_URL-backed PostgreSQL lane |
| Fixed DB distinctions | All 73 expected canonical subcase names pass exactly once, including `receipt_policy_sha256` |
| DB execution gates | All 12 actual DB labels exit 0 without SKIP markers |
| Existing regressions | 28 affected legacy names, 99 prior R2 Rust names, 6 separate chart names and 3 transport names pass individually |
| Migration and authority | 60 candidate SHA384 checksums match twice; NO_OP rerun; seven direct principals in both lanes; 0039 guard, four catalog negative probes and 0060 role/rollback harness PASS |
| Bootstrap and cleanup | 83 named canonical bootstrap results PASS; no owned containers remain |
| Whole candidate checks | Workspace format, strict Clippy, policy, Python and Web including E2E PASS |

[The complete CI record](2026-10-15-r2-daily-sql-c03-ci.md) describes counting,
platform exclusions and retained unknowns. The private sanitized final receipt is
`target/r2-daily-beta/ci/dee78c4/ci-evidence.json`, 2,265 lines, SHA256
`241dcf762329aa3eb3aba04f6df124ad07e2b5cef6b888577d0929d3abb8712d`.
Raw logs were not persisted. No local Cargo/compiler/Clippy/tests/SQL/DB/Docker,
project helper or provider execution was used. Local executable checks were
limited to the previously recorded scoped rustfmt and shell syntax checks.

## Preserved failures and limits

The first candidate [f5122dd](2026-10-15-r2-daily-sql-ci.md),
[C01 6fbc976](2026-10-15-r2-daily-sql-c01-ci.md) and
[C02 a252cfb](2026-10-15-r2-daily-sql-c02-ci.md) remain failed historical evidence.
Their full recovery preceded each bounded correction. C02 passed the new DB
cases but omitted a digit-bearing name from canonical collection. The final
[C03 delta](2026-10-15-r2-daily-sql-c03.md) changes only that character class,
preserving the exact 73-name uniqueness gate. No run was rerun or canceled.
Earlier I0d/PG59 and snapshot evidence retains its own revision and scope.

- SQL consumes verified projections. It does not open source files or establish
  canonical-byte/hash correspondence, provider truth or copied-file lineage.
  Remaining I1b reconstruction and admission code must establish those facts.
- Clock-boundary fixtures use a private synthetic clock. Real database lock waits
  are observed, but actual market-time transitions have not been observed.
- Isolated DOWN category tests remove dependency constraints within rolled-back
  fixture transactions. They establish guard behavior, not a supported bypass.
- The live exhausted-failure case forces deferred constraints and rolls back;
  the exhausted orphan case commits. These distinctions remain explicit.
- Canonical JSON retains the DB and bootstrap named outcomes. Unprinted nested
  helper cases/inner exits remain UNKNOWN. Two non-Linux cases are excluded on
  Linux. Python's 10 existing skips are not converted to passes.
- No installed release, runtime UID/ACL, provider data, Owner interaction or
  three-trading-day observation is accepted by this result.

## Next boundary

Continue the remaining **I1b offline original/copy/provider reconstruction and
admission library** against D4/D5, preserving the accepted Raw snapshot and SQL
interfaces. Then complete I3–I7 integration, final integrated review, installer
filesystem rollback and runtime UID/ACL evidence. Structural diagrams must be
updated before main integration. Operational activation, actual Owner use and
three consecutive trading days are later gates; the observation period has not
started. Passage of three calendar days alone does not close R2.

R3's permitted source/precondition work can be considered separately under the
roadmap; this acceptance neither starts R3 nor changes the current R2 priority.
No active worker, local background task or pending permission remains from I2.
