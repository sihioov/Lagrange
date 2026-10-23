# WP-4/I0c-CI hosted evidence — Wave22

## Run identity

- Repository `sihioov/Lagrange`; [run 38545976070](https://github.com/sihioov/Lagrange/actions/runs/38545976070), attempt 1.
- `ci.yml`, `workflow_dispatch`, branch `excited-dolphin`, created `2026-10-14T09:56:07Z`.
- Exact head `a888cd66c032c8931a4979434168568387c83a12`; run completed with success. Launch receipt records one dispatch; no workflow action was taken during this collection.

## Integrity and execution jobs

- Inventory SHA-256 `3cd93ddca6dc197bce885675884612356e1b358eefb8a0481e8dbd296c51d756`; launch receipt SHA-256 `647783cea0475c37ebdea5f01e7b17af5ade2e2b264d0133ee6513c6268483f4`; both link to the exact candidate/run. All 28 candidate source hashes matched.
- Each execution-job log contained the full candidate SHA. Logs stayed in memory; only byte lengths and SHA-256 values were retained.

| Job | Conclusion | Bytes | SHA-256 | Full SHA matches |
|---|---|---:|---|---:|
| `format` | success | 26249 | `305a30f002f4ae8a4f8899596813f64d8f093ac9f67db9a60ca8cf2db32af93c` | 4 |
| `policy` | success | 67856 | `feecfd4615c3e182d7145d2b197224cb1c8fb5dd5892827c858f0a85d86bd824` | 4 |
| `clippy` | success | 158366 | `83e07dcc6ad87853d72e61f5e0b635943595b61b4be8af7db3f53467e8764f4c` | 4 |
| `workspace-tests` | success | 1054930 | `09ca3d76809b86ca5c98e5b34c0dcdbbd4a37b4365ca20732ada98a56f95bee8` | 4 |
| `python-tests` | success | 50885 | `754e9d59df6bd0837436653712956ede6b89bcda3befad87d8ea4ef93729697e` | 2 |
| `postgres-integration-validation` | success | 48394 | `d4903a07db16914eb30be2e0493c995adc797d4b9355b4519760ba6bfe2eb6ff` | 5 |
| `web` | success | 149755 | `1bab2ee6204b52997960a7a4f6a50390de4feedfc5cb39b4121f5b1fc2fa6e9d` | 4 |

All seven execution jobs and required aggregate job `115800445406` succeeded. Complete job and step states are in the private receipt; the PostgreSQL failure-log step was skipped.

## Direct Linux test results

One direct `ok` result row was matched for every inventory name.

| Group | Source | Exact inventory name | Status |
|---|---|---|---|
| wire | `crates/market-data/src/daily_price_only/contract.rs` | `every_collection_guard_accepts_its_bound_and_rejects_one_more` | ok |
| wire | `crates/market-data/src/daily_price_only/contract.rs` | `every_declared_size_guard_accepts_its_bound_and_rejects_one_more` | ok |
| wire | `crates/market-data/src/daily_price_only/contract.rs` | `bounded_json_depth_and_civil_dates_accept_the_bound_and_reject_plus_one` | ok |
| wire | `crates/market-data/src/daily_price_only/contract.rs` | `json_array_evidence_count_guard_accepts_its_bound_and_rejects_one_more` | ok |
| wire | `crates/market-data/src/daily_price_only/contract.rs` | `bounded_json_input_and_canonical_output_allow_exact_bytes_only` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `synthetic_fixture_matches_pinned_bytes_hashes_and_bundle_links` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `every_json_input_byte_limit_accepts_exact_size_and_rejects_one_more` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `legal_proof_canonical_output_crosses_only_the_normalization_byte_bound` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `bundle_counts_follow_all_manifest_sessions_with_more_than_two_base_dates` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `declared_source_document_bars_and_normalization_sizes_hit_exact_limits` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `seal_rejects_a_valid_draft_whose_final_manifest_exceeds_the_byte_limit` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `present_empty_cash_codes_match_separate_golden_and_reseal_bundle` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `each_cash_code_accepts_empty_and_valid_32_byte_values` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `cash_code_json_and_typed_paths_return_exact_errors_before_hash_checks` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `policy_parser_canonicalizes_set_arrays_and_keeps_struct_order` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `parser_rejects_duplicate_unknown_missing_nullable_and_wrong_types` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `lexical_bounds_timestamp_precision_and_json_limits_are_fail_closed` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `proof_date_typed_and_json_paths_match_domain_year_bounds` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `single_page_markers_are_retained_and_legacy_hash_is_independent` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `request_interval_etf11_and_normalization_conflicts_are_rejected` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `ordered_action_pages_and_copy_hops_are_never_sorted` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `ten_ordered_pages_and_sixteen_distinct_copy_hops_hit_their_bounds` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `copied_calendar_keeps_terminal_request_target_and_changes_source_commitment` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `multi_hop_copy_order_compares_each_nanosecond` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `page_copy_and_count_guards_fail_before_claims_are_hashed` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `admission_aggregate_counts_accept_each_upper_bound_and_reject_plus_one` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `overlapping_cash_witnesses_can_outnumber_selected_normalized_rows` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `opaque_external_hashes_may_be_self_consistent_without_trust` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `parent_policy_code_and_source_commitments_change_only_descendants` | ok |
| wire | `crates/market-data/tests/daily_price_only.rs` | `verifier_commit_only_changes_admission_hash_and_stale_links_fail` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `returns_exact_crlf_whitespace_and_reordered_json_line_hash` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `accepts_complete_unterminated_record_and_ignores_torn_eof_tail` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_malformed_scope_and_filename_records_without_echoing_metadata` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `malformed_typed_rows_do_not_expose_submitted_values` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `returns_first_equal_duplicate_and_rejects_conflicting_metadata` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `preserves_first_occurrence_order_across_unique_batches_and_duplicates` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `enforces_exact_byte_and_record_bounds_before_deduplication` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_zero_and_overflowing_limits_before_filesystem_access` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `missing_tree_and_missing_lock_are_not_created_or_repaired` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_symlinked_raw_root_and_manifest_redirect` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_control_directory_symlink_redirect` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_ordinary_non_directory_path_component` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_non_regular_controls_wrong_expected_uid_and_writable_modes` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `valid_read_is_byte_for_byte_read_only` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `outside_raw_data_root_mode_is_not_treated_as_raw_owner_policy` | ok |
| raw | `crates/market-data/tests/daily_price_only_storage.rs` | `rejects_relative_and_parent_component_roots` | ok |
| raw | `crates/market-data/src/storage.rs` | `committed_manifest_evidence_rejects_lock_replacement_before_locking` | ok |
| raw | `crates/market-data/src/storage.rs` | `committed_manifest_evidence_rejects_manifest_replacement_before_read` | ok |
| collector | `data-pipelines/collectors/src/worker.rs` | `worker::process_tests::committed_manifest_store_errors_are_permanent_and_diagnostic_redacted` | ok |
| helper | `crates/job-queue/tests/owner_beta_publish.rs` | `daily_input_admitter_pool_is_direct_bounded_and_unprivileged` | ok |
| helper | `crates/api-server/tests/http_owner_beta_access.rs` | `http_daily_input_admitter_pool_is_direct_bounded_and_unprivileged` | ok |

**51/51 `ok`**: 30 wire, 18 Raw, 1 collector, 2 helper; 0 failed, ignored, not executed, or unknown. Raw non-Unix test `fails_closed_without_unix_descriptor_relative_opens` is `EXCLUDED_LINUX`, not passed.

### Chart direct rows (separate step)

| Exact test name | Status |
|---|---|
| `chart_fails_closed_for_tampered_artifacts` | ok |
| `chart_enforces_owner_and_exact_query_contract_before_artifact_io` | ok |
| `chart_keeps_newer_owner_admission_when_shared_confirmed_close_lags` | ok |
| `chart_rejects_as_of_beyond_requested_cutoff_with_or_without_close_reference` | ok |
| `chart_rejects_every_artifact_visible_lineage_mismatch` | ok |
| `chart_returns_only_verified_snapshot_pinned_eod_projection` | ok |

All six chart rows matched one direct `ok` line. The workspace had 237 Cargo result summaries; summed rows (2,943 passed, 0 failed, 6 ignored, 0 measured, 180 filtered) are not unique tests. Eighteen historical V2/V3 direct rows (18 `ok`) are kept separate and do not establish full historical acceptance.

## Checks

- Format check succeeded: `cargo fmt --all -- --check`. Clippy succeeded: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; diagnostic-header extraction is UNKNOWN.
- Policy job succeeded. Safe markers: `PG_VALIDATION_SELF_TEST: PASS` ×1, `SECRETS_RUNTIME_STATIC: PASS` ×1, `OPS_STATIC: PASS` ×2. No hidden-skip absence claim.
- Python pytest summary: 485 passed, 10 skipped, 7 warnings; failed count not printed. No direct readiness/SIGTERM result rows recovered; individual outcomes UNKNOWN.
- Web OpenAPI, lint, typecheck, tests, build, and candidate E2E steps succeeded; 61/61 test files and 487/487 test rows.

## PostgreSQL

- Validation and sanitized-evidence steps succeeded; failure-log step skipped. Exact official labels `jobqueue-daily-admitter-pool` and `api-daily-admitter-pool` each appeared once. Sanitized output had APPROVED ×2, DENIED ×0, and one SKIP token.
- Workflow configures disposable QA Compose plus `DATABASE_URL`; workspace DB up/down steps succeeded and both helper direct test rows passed in that configured context. Common test helpers contain absent-DB skip guards; absent-DB behavior was not executed.
- No explicit no-SKIP result marker was recovered. The source contains the no-SKIP gate, but official nested helper rows and nested Cargo exit are UNKNOWN.
- Sanitized output mentions migration 59, `0039`, and cleanup. Exact migration-59 checksum equality, `0039` outcome, and cleanup outcome are UNKNOWN; wrapper success does not fill those fields.

## Preservation and limits

- Prior report SHA-256 unchanged: `989a3891493f3ece231de15050f6bb3d139f1034edf3c0fe87e4aef4b686a47c`; prior receipt SHA-256 unchanged: `9016b6c2c97c5e2b05763dec131f0e7199d8d9ed0017da53470e7b2694ab7b0e`. `c0ac599` remains historical baseline only.
- Private receipt: `target/r2-daily-beta/ci/a888cd6/ci-evidence.json`. No raw logs, credentials, fixture secret literals, provider text, or DB/error bodies were persisted.
- Not established: hidden-skip absence, negative bootstrap/rollback readiness, migration 0060, snapshot/provider reconstruction or admission, runtime/production, Owner acceptance, or final C_R2 acceptance. No source/workflow/Git writes or further decomposition.
