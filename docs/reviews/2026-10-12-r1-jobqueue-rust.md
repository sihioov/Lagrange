# R1 job-queue Rust repair handoff

**Status:** `READY_FOR_HEAVY_VALIDATION`. This is the source/static phase for the accepted WP-2c D/E/F package. It advances roadmap items R1-02 and R1-03; their roadmap status is left to the coordinator because `docs/ROADMAP.md` is outside this package's writable-file allowlist.

## Changed files and implementation

- `crates/job-queue/src/owner_equity_v2/market_stream.rs` — test-gated `StreamPublicationContext::single`, `RuntimeStatusCommit::changed`, and the owner repository status writer/helper; removed the three unused `RuntimeGrant` accessors, `RuntimeStatusCommit::state_version`, and the unused runtime status wrapper. Renamed the 19 unread `FromRow` Rust fields with underscore prefixes and exact `sqlx(rename = "original")` attributes, plus only the three `CacheDbRow` fixture keys. Removed only the `_cache` delivery argument and returned `StreamStatus::new` directly from the option-map closure. Added the approved enum-item expectation to `RuntimeTransition`. Main changed spans: 628, 1081-1091, 1087-1091, 1315-1326, 3340, 3600, 3896, 4136-4495, 4673-4687, 5971, 6455, and 6783-6794.
- `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs` — boxed the `DayResolution::Open` payload and both full `StageResult::Connected` / `Started` results; consuming matches use `*result`, and the next-open-day copy clones the underlying resolved value. Collapsed the lease predicate without moving renewal/time checks. Kept the adopted legacy resolver and added only the approved item-scoped expectation to `resolve_day`. Main changed spans: 110, 236-239, 380-386, 813-814, 1464-1473, 1495, 1839, 1864-1867, and 2032-2039.
- `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` — removed only the specified nested imports and unused wrappers; gated the named test helpers and `PendingBinding.kind` plus both initializer entries; replaced the two specified forwarding blocks with `.await?`; boxed `PublicationBatch` once at handoff and carried that same box through `WriterAction::Publish`, consuming it with `complete_success(*batch)` only after known success and a currentness check. Collapsed the two approved predicates and added one approved expectation to each retained `InvalidationCause` variant. The pre-existing format-only producer edits were preserved. Main changed spans: 500-567, 1132, 1161-1201, 1562-1567, 1600, 1658-1692, 1743, 2945, 3046, 3116, 3263-3278, 3409, 3543, 3840, 4446, and 4508.
- `docs/reviews/2026-10-12-r1-jobqueue-rust.md` — this implementation and verification handoff.

All requested D/E/F edits follow `docs/reviews/2026-10-12-r1-baseline.md` D/E/F at lines 442-495, its coordinate correction at 523-538, and the D1/D1b/D2 approval recorded in the supplement at 539-541. The only lint expectations added are the approved `RuntimeTransition` enum expectation, the two `InvalidationCause` variant expectations, and the `resolve_day` item expectation. No public API, feature default, serde/wire shape, SQL column, query text, lock order, bind, or authority/time gate changed. The 19 SQLx fields continue decoding the original required columns; no query projection was removed.

## Commands and results

- `rustfmt --edition 2024 --config skip_children=true crates/job-queue/src/owner_equity_v2/market_stream.rs crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` — exit 0.
- `git diff --check -- crates/job-queue/src/owner_equity_v2/market_stream.rs crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` — exit 0.
- Targeted `rg` caller/constructor inventories for the removed wrappers, boxed `DayResolution` / `StageResult` / publication variants, and test-only helpers — exit 0; the reported constructors and consumers are the only ones found in the owned files.
- `git diff --unified=2 --` on the three owned Rust sources — exit 0; reviewed the resulting patch and confirmed query text is untouched. The shared worktree contains other workers' dirty files; this worker wrote only the three owned Rust sources and this report.

No Cargo, rustc, Clippy, test, Docker, database, or build command was run, as required for the source/static phase. Formatting and diff checks do not establish compilation or behavior.

## Follow-up and not verified

- Heavy validation must check default, test, and all-feature expectation fulfillment and propagation. In particular, only `resolve_day` received an expectation; if a later compiler run reports `dead_code` for its retained legacy variant or helper, resolve that from the actual diagnostic without preemptive suppressions. D3's mixed-feature test mapper remains with its separate owner.
- SQLx decoding after the private field renames, existing unit/DB tests, runtime ownership/drop/cancellation behavior, allocator effects, full Clippy, and final CI acceptance remain unverified. No test or compile result is inferred.
- The component diagram evidence at `docs/diagrams/component_architecture.puml:217` cites `market_stream_runtime.rs:1836`; the owned-anchor connect call is now at line 1839. The coordinator should refresh that evidence and re-render the diagram under its ownership. No diagram was changed here.
- No required cross-file source edit or unlisted callsite was found. Not found otherwise: **none**. No staging, commit, or provider/network action occurred.
