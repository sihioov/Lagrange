# R2 Wave64 I5-C01 bounded source correction

Roadmap R2-04/09; baseline68c6f64 after full failure recovery of CI38810733003.
Direct coordinator source review; independent review and executable acceptance
are not claimed. No new tests are required for these three exact corrections;
all 25 I5 Rust and six Web definitions and their assertions remain preserved.

- I5-C01a uses the existing public daily_price_only::DailyBasePinsV1Wire export.
  The private contract module remains private; type/decoder/capability unchanged.
- I5-C01b restores the original price-input doc comment directly to its function.
  Daily mode has a correct one-line description. Runtime tokens excluding doc
  comments/whitespace are identical to the baseline.
- I5-C01c matches the hosted Biome diff: one assertion wrap and final blank line.
  Test source normalized for whitespace/trailing argument commas is identical;
  no assertion or marker changed.

The complete three-file diff was read. Exact baseline comparisons, public-export
source check, all683prior pins (680unchanged/3owned changes), all25+6definition names,
scoped standalone Rust2024 formatting and whitespace checks passed.
No local compiler, lint, tests, project scripts or Web formatter ran. New candidate
hosted compilation/strict lint/18PGtargets/Web tests must pass before I5 acceptance.

Previous failure report and all original receipts remain immutable. Successor
inventory updates only candidate metadata and definition line references.
No dependency, migration, role, workflow, provider, activation or policy change.
No delegation. I7/I6, actual approved-base threeUID integration, independent final
review, G4/Owner/three trading days and final C_R2 remain open.

| Changed path | Lines | SHA256 |
| --- | ---: | --- |
| `crates/api-server/src/repos/owner_beta.rs` | 1752 | `8cac024f693a5d018c45a41b9318d175829ecd21ba2e17a5fe18d4b96204815e` |
| `crates/api-server/src/runtime.rs` | 2794 | `eb63b027817d8a1052691ff849aa023ea1b00fa3b8514b9c201bd81cb98f970d` |
| `apps/web/tests/r2-daily-api-contracts.test.ts` | 211 | `61f69345b057159c9728ba2f394748e801f71b285b9454d5e9dc1cb511ee78a7` |
