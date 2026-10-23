# R2 I4g1 bounded artifact navigation — source review

Date: 2026-10-16 KST. Roadmap R2-04/R2-09, Wave59.
Coordinator direct source review; not an independent review or executable acceptance.
Baseline: `7a5b347ca5e42ddc7ef0d5e1ceedb8dc2182fdf4`, accepted with run38788565250 attempt1.
Only three Rust files change; insertions total +1064/-0. No DB, binary, workflow,
activation, provider, rights or approval constructor changes.

| File | Lines | SHA256 |
|---|---:|---|
| `crates/market-data/src/daily_price_only/artifact.rs` | 2316 | `638bb773607bc086f745cb2bba8c65cefd756b3c7db1d05e931fc9833e0a9fc4` |
| `crates/market-data/src/daily_price_only/frontier.rs` | 1273 | `823e8eed8e51290ece768df4ab89e3b0b3673765a2620d245377c71a5c023cff` |
| `crates/market-data/src/daily_price_only/replay.rs` | 944 | `c6540c5f8306a5d1c2490f364299df1a4a5c6dd35ee024f001f223b8ed10f33f` |

## Disposition

- `artifact.rs` adds an explicit nonzero name budget bounded by the fixed session
  ceiling. Both committed and recognized staging entries consume it. Capture uses
  independent descriptor-relative directory enumeration with a fixed buffer;
  canonical committed names require no-follow directories, exact owner/group/mode,
  ACL rejection and identity snapshots. The root and captured entry snapshots are
  rechecked. It never chooses a latest entry or verifies artifact contents.
- Canonical `.pending-PID-sequence` entries are invisible and unopened. Even a
  staging symlink is not followed; the API does not delete or promote staging.
  Unknown/malformed names fail closed. In-place changes inside a pending entry
  are intentionally outside name capture; each eventual committed artifact still
  requires full guarded reading. A name capture is not an immutable source proof.
- Frontier metadata reads require the existing exact frontier leaf filenames and
  all existing record-file guards, then canonical record encoding, schema, owner,
  range and expected observation hash. The observation payload is deliberately
  not opened. Record-only success therefore conveys no sealed frontier capability.
- Paired replay walks the bounded admission ancestry and its exact frontier
  ancestry once each, rejecting owner/policy/admission mismatch, missing/extra
  parents, cycles and inconsistent range progression. The forward pass calls
  every existing full admission and frontier verifier and retains only the current
  cumulative input/frontier. Explicit pins are required; no DB admission is inferred.
- Cancellation is checked around metadata/full-leaf operations. Synchronous single
  bounded reads are not preemptible. A DB receipt, current source refresh and
  activation/rights gates remain mandatory for runtime use.
- Existing single-chain replay, full frontier read/write and admission metadata
  reading remain byte-identical (boundary whitespace excluded). No lint allows,
  unsafe bypass, fabricated approved historical artifact or synthetic sealed input
  was introduced. Existing SQL migrations and prior tests are unchanged.

## Written tests

All rows below are WRITTEN_UNEXECUTED for this candidate. The first ten are Linux
native protected-directory metadata tests; the remaining nine are pure metadata
navigation tests. Names and source locations are source bookkeeping, not execution.

| Qualified name | Source | Line | Platform |
|---|---|---:|---|
| `daily_price_only::artifact::linux::tests::i4g_catalog_limits_are_explicit_and_bounded` | `crates/market-data/src/daily_price_only/artifact.rs` | 2049 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_empty_and_sorted_names_are_navigation_only` | `crates/market-data/src/daily_price_only/artifact.rs` | 2072 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_staging_is_invisible_unopened_and_counted` | `crates/market-data/src/daily_price_only/artifact.rs` | 2096 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_unknown_names_and_malformed_staging_fail_closed` | `crates/market-data/src/daily_price_only/artifact.rs` | 2121 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_committed_names_require_guarded_directories` | `crates/market-data/src/daily_price_only/artifact.rs` | 2153 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_detects_replacement_and_nested_name_mutation` | `crates/market-data/src/daily_price_only/artifact.rs` | 2174 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_rejects_root_rebinding_after_capture` | `crates/market-data/src/daily_price_only/artifact.rs` | 2195 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_catalog_public_roots_keep_distinct_layouts` | `crates/market-data/src/daily_price_only/artifact.rs` | 2206 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_frontier_navigation_opens_only_record_with_full_record_guards` | `crates/market-data/src/daily_price_only/artifact.rs` | 2236 | Linux |
| `daily_price_only::artifact::linux::tests::i4g_frontier_navigation_rejects_extras_symlinks_and_oversize` | `crates/market-data/src/daily_price_only/artifact.rs` | 2262 | Linux |
| `daily_price_only::frontier::tests::i4g_frontier_navigation_is_canonical_record_only` | `crates/market-data/src/daily_price_only/frontier.rs` | 1204 | all |
| `daily_price_only::frontier::tests::i4g_frontier_navigation_rejects_noncanonical_unknown_duplicate_and_hash` | `crates/market-data/src/daily_price_only/frontier.rs` | 1222 | all |
| `daily_price_only::frontier::tests::i4g_frontier_navigation_rejects_invalid_record_envelope_before_planning` | `crates/market-data/src/daily_price_only/frontier.rs` | 1252 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_returns_exact_matching_ancestry` | `crates/market-data/src/daily_price_only/replay.rs` | 736 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_rejects_missing_parent_extra_parent_and_cycle` | `crates/market-data/src/daily_price_only/replay.rs` | 759 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_rejects_foreign_owner_policy_and_admission` | `crates/market-data/src/daily_price_only/replay.rs` | 801 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_rejects_range_regression_and_changed_start` | `crates/market-data/src/daily_price_only/replay.rs` | 843 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_cancellation_stops_before_or_after_each_read` | `crates/market-data/src/daily_price_only/replay.rs` | 879 | all |
| `daily_price_only::replay::tests::i4g_paired_navigation_invalid_request_and_errors_do_not_become_proof` | `crates/market-data/src/daily_price_only/replay.rs` | 913 | all |

## Verification and limits

Scoped standalone `rustfmt --edition 2024 --config skip_children=true --check`
for the three files and `git diff --check` passed. Full changed function bodies,
existing verification calls, error conversion, descriptor ownership, platform
branches and all nineteen test definitions were directly inspected.

Compilation, strict Clippy and all new test execution are pending one fresh
exact-candidate hosted CI after pin/blob and all-event existing-run checks.
No local Cargo/compiler/test/parser/project-helper/SQL/DB/runtime execution occurred.
No worker or subagent was delegated. No claim of independent review is made.

The tests exercise protected name and record handling and unapproved navigation.
They do not execute a returned paired capability with the real approved v3 base.
Distinct runtime UID/ACL, crashes/fsync fault injection, producer/admitter cycle
selection, receipt reconciliation, installer rollback, I5-I7 and integrated review
remain open. Runtime/Owner/three trading days/final C_R2 remain open.
