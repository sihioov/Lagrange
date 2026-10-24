# PostgreSQL integration-validation evidence

Copy this template into the operator-controlled evidence directory before a
Go/No-Go review. Do not copy generated passwords, full `DATABASE_URL` values,
Docker inspect output containing environment, or unredacted command logs.

```text
workflow: postgres-integration-validation
run_id:
started_at_utc:
finished_at_utc:
operator:
host:
postgres_image_digest: postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a
upgrade_cluster: disposable / torn down (yes|no)
test_cluster: disposable / torn down (yes|no)
full_release_cluster: disposable / torn down before DB tests (yes|no)
credentials: generated in private temporary directory (values omitted)
test_harness_password: existing synthetic `lagrange` fixture required by the
  selected job-queue role URL helper (not an operator/production credential)

upgrade:
  baseline: 0038
  0038_applied_count: 38
  preflight_baseline: PASS|FAIL
  0039_applied_count: 39
  preflight_0039: PASS|FAIL
  0039_down_undelivered_guard: PASS|FAIL
  0040_applied_count: 40
  preflight_0040: PASS|FAIL
  identity_cross_owner_boundary: PASS|FAIL
  0041_applied_count: 41
  preflight_0041: PASS|FAIL
  0041_noop_rerun: PASS|FAIL
  0059_history_seed_v2_v3_four_states: PASS|FAIL
  0060_applied_count: 60
  0060_history_and_59_checksums_preserved: PASS|FAIL
  0060_noop_rerun_60_checksums: PASS|FAIL
  0060_ten_tables_owner_rls_force: PASS|FAIL
  0060_actual_logins: app,worker,research_writer,admin,daily_input_admitter
  0060_unused_down_up_exact_restore: PASS|FAIL
  0060_hidden_owner_down_sqlstate_55000: PASS|FAIL
  0060_denied_down_atomicity: PASS|FAIL
  normalized_pending_invite_duplicates: 0
  terminal_paper_targets:
  paper_settlement_outbox_rows:
  paper_settlement_archive_rows:
  terminal_obligation_coverage:
  direct_service_role_logins: migration_owner,app,worker,audit_writer,research_writer,admin,daily_input_admitter
  table_ownership: all public tables owned by migration_owner (yes|no)
  schema_create_boundary: migration_owner only (yes|no)

hazards:
  postgres_version:
  max_connections:
  active_connections:
  free_bytes_before:
  free_bytes_after:
  connection_pool_headroom: PASS|FAIL
  disk_headroom: PASS|FAIL

tests:
  migration_contract: PASS|FAIL|BLOCKED_EXTERNAL
  auth_audit_readiness: PASS|FAIL|BLOCKED_EXTERNAL
  auth_router: PASS|FAIL|BLOCKED_EXTERNAL
  api_tenancy_rls: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_execution: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_notifications: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_scheduler: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_runner: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_contract: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_paper_preview: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_daily_admitter_pool: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_publish: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_daily_schedule: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_daily_publish: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_daily_recovery: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_daily_backup: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_daily_runner: PASS|FAIL|BLOCKED_EXTERNAL
  api_daily_admitter_pool: PASS|FAIL|BLOCKED_EXTERNAL
  api_owner_beta_daily: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_owner_beta_simulation_execution: PASS|FAIL|BLOCKED_EXTERNAL (15 exact named rows)
  api_owner_beta_simulation_replay: PASS|FAIL|BLOCKED_EXTERNAL (one module-qualified exact row)
  api_owner_beta_simulations: PASS|FAIL|BLOCKED_EXTERNAL (7 exact named rows)
  process_exit_rows: 22 labels, each actual exit_code and skip_markers
  daily_0060_named_rows: 32 unique exact names with actual libtest outcome
  daily_0060_subcase_rows: 73 unique fixed labels with actual PASS
  r4_simulation_test_commands: exact three Cargo commands recorded in evidence.json
  r4_simulation_expected_rows: exact 23 expected names recorded in evidence.json
  r4_simulation_named_rows: actual names/results recorded in evidence.json and PG_DB_TEST_ROW markers
  skip_markers: none (yes|no)

R4-05 selected PostgreSQL lanes:
  jobqueue_owner_beta_simulation_execution:
    command: cargo test --locked -p job-queue --test owner_beta_simulation_execution -- --nocapture
    names: r4_actor_rls_and_hash_isolation, r4_role_grants_and_forced_rls, r4_success_pair_is_one_commit, r4_partial_pair_and_transaction_failure_hidden, r4_typed_claim_and_generic_exclusion, r4_lease_retry_exhaustion_and_zombie, r4_cancel_wins_and_late_publish_denied, r4_migration_up_down_guards, r4_read_revalidates_both_profiles, r4_entitlement_covers_every_consumed_session, r4_failure_settlement_retries_only_infrastructure_errors, r4_commit_unknown_reconciliation, r4_invalid_claim_is_terminal_and_attempt_fenced, r4_invalid_recovery_is_terminal_and_continues_to_valid_jobs, r4_frozen_config_edits_allow_failure_cancel_recovery_and_publication
  api_owner_beta_simulation_replay:
    command: cargo test --locked -p api-server --lib http::owner_beta_simulations::tests::simulation_durable_replay_and_intent_binding -- --nocapture
    names: http::owner_beta_simulations::tests::simulation_durable_replay_and_intent_binding
  api_owner_beta_simulations:
    command: cargo test --locked -p api-server --test http_owner_beta_simulations -- --nocapture
    names: simulation_owner_admission_and_closed_body, simulation_http_cross_owner_list_get_cancel_pair, simulation_same_hash_independent_ownership, simulation_pair_only_after_atomic_success, simulation_cancel_publish_and_lease_races, simulation_entitlement_revoke_and_safe_errors, simulation_generic_surfaces_stay_separate

All three R4-05 lanes use the existing isolated test database and teardown. Their
named cases are synthetic fixture-backed API/storage tests; they do not prove
approved-artifact enqueue, runner process/lease acceptance, or production readiness.

full_release:
  first_count_and_latest: 63 / 63
  second_count_and_latest: 63 / 63
  all_checksum_rows_equal: PASS|FAIL
  no_seeded_daily_control_or_optin: PASS|FAIL
  cleanup_no_owned_containers: PASS|FAIL

migration_safety_audit:
  0039_down_undelivered_guard: PASS|FAIL
  0040_actor_binding: PASS|FAIL
  auth_audit_empty_poll_failure_retention: PASS|FAIL

verdict: APPROVED|DENIED|BLOCKED_EXTERNAL
go_no_go_owner:
decision:
blocking_finding:
sanitized_evidence_directory:
```

Go requires every upgrade, hazard, direct-login, and selected-test field to be
`PASS`/present, zero normalized duplicates, complete terminal obligation
coverage, no `SKIP:` marker, and successful teardown. Any failed safety audit,
missing evidence, Docker/PostgreSQL blocker, or retained cluster is No-Go.
