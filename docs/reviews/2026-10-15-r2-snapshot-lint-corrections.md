# R2 snapshot C04 strict-lint corrections

The [C03 hosted run](2026-10-15-r2-snapshot-c03-ci.md) passed all 99 expected
Linux Rust cases and six separate chart cases, but strict Clippy reported three
findings at two expressions. The coordinator fully recovered that failed run
before changing source. This is a direct bounded correction; no worker delegation.

Baseline: `d00b4759ba9e08887d3871f64e8f53bfb1f02fa7`. Exactly two hunks change
[source.rs](../../crates/market-data/src/daily_price_only/source.rs):

- At 1416, dates.contains(&batch.entry.date) compares the same TradingDate values
  as the former iter().any equality predicate. Fallible reserve/insertion remain.
- At 2264, depth.saturating_sub(1) preserves zero and decrements positive values.
  String/escape state, checked depth growth, maximum tracking and subsequent
  strict JSON validation are unchanged. This closes both lint diagnostics.

There are no lint allowances, public API changes, guard changes, new dependencies
or test-definition changes. The C03 ACCESS fixes and independent C+7 regression
oracle remain. Existing P1–P10/FR/R/R12/C01/T conditions carry forward within
these two expressions' demonstrated equivalence. This is coordinator static
disposition, not a new independent worker review or executable acceptance.

The exact two-replacement delta was compared with the baseline. The other 55
Wave31 pins and all 49 S2 names are unchanged. Scoped standalone rustfmt 2024 and
Git whitespace checks passed. No local Cargo/rustc/Clippy/tests/build, project
helpers, SQL/DB or Docker ran. Existing tests cover these expressions; no
implementation-mirroring tests were added.

Source: 4,445 lines, SHA256
`4dec440f07a75cacea82702d9a5ec2ec08c5ce78e994da6bc2049bfbab3e1d94`.
Private source-check receipt: wave32-C04-source-checks.json under
 target/r2-daily-beta/wp-4-integration, 359 lines, SHA256
`cab6cd27416b1fdcdda7656807372a3dd65796259f595db6776e88890d94c913`.

A new exact candidate still requires hosted verification. Query existing runs
across all events before one possible dispatch; never rerun/cancel/duplicate the
prior run. Prior partial passes are not new-candidate passes. R2-04/R2-09 and
negative bootstrap/rollback,0060,reconstruction/admission,diagrams before main,
runtime,Owner and final C_R2 remain open.
