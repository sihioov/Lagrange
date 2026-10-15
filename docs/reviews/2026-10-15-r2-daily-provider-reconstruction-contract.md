# R2 I1b-S4 — offline provider reconstruction

2026-10-15; R2-04/09; baseline52cba24093e5e7aea27b4e1eb3fae8311f4b873f.
The Owner requested Ralph continuation through R2 completion. S3 lineage/calendar
and I2 SQL acceptance remain intact. This source slice implements D4 and D5 A1/A6
provider-row reconstruction; source sealing, artifact storage/admission and runtime
remain later dependencies. No provider request or operational activation is added.

## Scope and concrete interfaces

- Price/reference reconstruction consumes private-constructor S3 ResolvedOriginal
  values, exactly one request per fixed ETF11 symbol and kind. D4 validates exact
  requests and original date. Parse bounded duplicate-rejecting JSON; validate
  every bar row before selecting the requested date. Require exactly one target
  row per symbol. Reuse the existing canonical_bar mapper unchanged. Reference
  identity comes independently from inquire-price stck_shrn_iscd; no bar-derived
  name is added. Reconstructed rows/documents follow D5 A6 exactly.
- Action reconstruction consumes complete committed snapshot batches and resolves
  every candidate page through S3. Discovery is by validated F_DT/T_DT, class and
  scope, never partition date. Accept complete7 or77 groups within each committed
  bundle; never assemble an incomplete batch from another batch. Reuse the fixed
  seven-class request matrix and D4 chain validation. Every retained page and row
  must be checked, including non-ETF rows, before filtering.
- Ordered page evidence has one blank initial tr_cont and N continuations, exact M
  predecessor markers, a non-M terminal page, no duplicate bytes and at most10
  pages. Actual captured metadata must support the ordering; no invented page
  ordinal or missing-page inference. All pages remain in their original bundle.
  The existing provider's exact terminal-original filename supplies that ordinal;
  copied files may have different current names. No D4 ordinal field is added.
- For each required civil date/class/ETF cell, compare all complete covering
  chains as full parsed-row multisets with recursively sorted object keys, array
  order/scalars preserved. Duplicate events, out-of-range rows, malformed rows,
  noncash ETF evidence and conflicting overlaps fail closed. Calendar closure is
  not negative action evidence. Equivalent whole chains use D5's deterministic
  tuple; retain every compared chain as a witness and include all source keys.
- Cash exclusions use the existing full dividend field validator and D4 cash row
  validation, preserving empty opaque codes and exact decimals. Never infer an
  ex-date, available_at, adjustment or entitlement. Reconstructed documents use
  D5's fixed schema/version and D4 source commitments; no generated output UUID,
  sampled current time or normalized Raw write.

Results have private constructors and read-only access, retain snapshot borrows,
and convey neither source-frontier completeness nor artifact approval. Errors
remain finite payload-free codes. Source limits fail instead of truncating.
S2 scan budgets and D4 source/JSON/date/page/chain/file/cash/document bounds apply.
Accumulated canonical comparison bytes are additionally capped at D4's 64 MiB
document ceiling and canonical FileEvidence at its 16 MiB manifest ceiling.
Neither limit admits a truncated inventory. Rows outside the requested chain
interval fail. Valid rows inside a broader chain but outside the required extension
do not become its cash exclusions or noncash blockers; duplicate full rows still fail.

## Owned files and validation

Coordinator works directly; no further delegation. New
`crates/market-data/src/daily_price_only/reconstruct.rs` and
`crates/market-data/tests/daily_price_only_reconstruction.rs` separate this slice
from the frozen S3 fixtures. Narrow sequential changes are allowed in verify.rs,
contract.rs, mod.rs and normalize.rs for internal adapters/visibility only.
The new module/test paths refine D5's original verify/evidence file allocation;
no package, dependency or public wire format changes are authorized by that split.
Keep snapshot/storage/SQL/workflow/provider collection and legacy tests unchanged.

Required written fixtures cover exact11 coverage and requested/returned identity;
every malformed surrounding row; duplicate JSON; target missing/duplicate; old
normalizer bytes; original-date copies; deterministic golden documents; complete
7/77, interval/partition mismatch and Friday-to-Monday coverage; page mutation,
duplicate/limit/terminal failures; non-ETF validation, noncash blockers and cash
metadata; equivalent and contradictory overlaps, row-order/array/scalar behavior,
duplicate events, empty/absent distinction; and snapshot/lock/no-write lifetime.

Only scoped standalone rustfmt2024 and inert source/hash/diff/inventory inspection
run locally. After concrete source disposition, freeze named cases and inputs;
one new exact-candidate hosted CI follows an all-event existing-run query and
remote SHA match. Recover every job, checkout, log hash, named result and all
carried S3/I2 regressions. No old run rerun/cancel/duplicate. A clean source review
or a written test is not compiler/runtime acceptance. Final integrated independent
review and every R2 runtime/Owner/three-trading-day gate remain open.
