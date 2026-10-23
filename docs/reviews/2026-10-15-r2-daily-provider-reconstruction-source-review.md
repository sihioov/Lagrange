# R2 I1b-S4 source disposition

2026-10-15; R2-04/09; baseline52cba24093e5e7aea27b4e1eb3fae8311f4b873f.
Coordinator direct review of the [bounded contract](2026-10-15-r2-daily-provider-reconstruction-contract.md),
D4 and D5 A1/A6. This is source inspection, not independent or executable
acceptance. No additional worker was delegated. Exact-candidate CI is next.

## Concrete implementation

- [Price reconstruction](../../crates/market-data/src/daily_price_only/reconstruct.rs:77)
  requires11 resolved bar originals and11 reference originals. D4 verifies exact
  requests, unadjusted single date and original date. Every surrounding bar passes
  the unchanged mapper before selection. Missing/duplicate targets fail; reference
  identity comes independently from inquire-price, without names from bars.
- [Action reconstruction](../../crates/market-data/src/daily_price_only/reconstruct.rs:323)
  discovers F_DT/T_DT rather than partition dates. Each captured batch must contain
  the complete7/77 class/symbol matrix at one range/scope. Terminal-original names
  supply page ordinals. D4 checks requests, continuation, terminal page, distinct
  page hashes and common current/original bundle metadata before body interpretation.
- [Row validation](../../crates/market-data/src/daily_price_only/reconstruct.rs:655)
  uses the full existing action-field validator for all rows, including non-ETF
  evidence. Out-of-range or duplicate full rows fail. Only the required extension
  contributes cash rows; ETF noncash evidence blocks it. Fixed-scope aliases agree.
  Decimal strings lose insignificant zeros without floating-point conversion;
  empty opaque codes and future payment dates retain their documented meaning.
- Civil-day/class/ETF cells compare full canonical row multisets, including unknown
  members, nested array order and scalar types. Empty completed evidence differs
  from absent evidence. Equivalent candidates select a whole chain by the D5 tuple;
  all compared chains remain witnesses. There is no newest-retrieval preference.
- Documents use the exact D5 six-field schema and D4 source commitments. Results
  have private constructors and retain source borrows, without frontier/admission
  authority. The later trusted pipeline must bind Owner/policy/context and sealing.

## Preservation and checks

Six source/test files change: reconstruct.rs, its integration test, contract.rs,
verify.rs, mod.rs and normalize.rs. Contract cash sorting/validation/serialization
keeps its original sequence through an internal helper. Normalizer changes are
two visibility declarations only. S2 storage, calendar, provider acquisition, SQL,
dependencies and workflow remain unchanged. Of262 preceding frozen pins, only
the four intentionally changed pre-existing Rust paths differ;258 still match.

All32 new Linux test definitions are WRITTEN_UNEXECUTED at this disposition.
Fixtures cover literal document layouts, complete7/77/partial bundles, weekend
coverage,370/371 civil dates,1/10/11 pages and failures, every noncash class,
malformed non-ETF rows, cash metadata, overlapping full rows, ordering, copies
and read-only snapshot locking. Required hosted regressions retain22 S3,37
normalizer,99 R2 Rust,32 daily SQL,28 legacy DB,3 transport and6 separate chart
rows. These groups may overlap and are not added into a unique-test count.

Scoped standalone Rust2024 formatting and whitespace checks passed. No local
compiler, Clippy, tests, provider parser or project checker ran. These fixtures
do not establish installed UID/ACL, actual source inventory, resource consumption
at every large byte ceiling or non-Linux execution. No source acceptance is
claimed before full hosted recovery. Whole-frontier persistence, artifact/admission/
supply, I3–I7, final integrated independent review, diagrams before main and actual
operational/Owner/three-trading-day evidence remain open, including final C_R2.
