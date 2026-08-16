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
  normalized_pending_invite_duplicates: 0
  terminal_paper_targets:
  paper_settlement_outbox_rows:
  paper_settlement_archive_rows:
  terminal_obligation_coverage:
  direct_service_role_logins: migration_owner,app,worker,audit_writer,research_writer,admin
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
  api_tenancy_rls: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_execution: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_notifications: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_scheduler: PASS|FAIL|BLOCKED_EXTERNAL
  api_paper_runner: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_contract: PASS|FAIL|BLOCKED_EXTERNAL
  jobqueue_paper_preview: PASS|FAIL|BLOCKED_EXTERNAL
  auth_audit_readiness: PASS|FAIL|BLOCKED_EXTERNAL
  skip_markers: none (yes|no)

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
