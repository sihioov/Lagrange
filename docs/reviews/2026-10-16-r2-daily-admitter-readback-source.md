# R2 I4g2 admitter readback — source review

Date: 2026-10-16 KST. Roadmap R2-04/R2-09, Wave60.
Coordinator direct review; not independent review or executable acceptance.
Baseline: `35b64febac11eb258c86dc51a35d4d67ba7fe87b`, run38792738853 attempt1 SUCCESS.
Five source/test/harness files. Existing0060/0061 are unchanged.

| File | Lines | SHA256 |
|---|---:|---|
| `migrations/0062_owner_beta_daily_admitter_readback.up.sql` | 174 | `b93c1bb238f78a57a826a8d03d1cc070253fe98cae2169eaebd0c57e285f3e4b` |
| `migrations/0062_owner_beta_daily_admitter_readback.down.sql` | 6 | `dd589289b015e8325ec3589829974becdc4d108236e5a2a30305074c823e69ad` |
| `crates/job-queue/src/owner_beta/runner/daily_admission.rs` | 776 | `128f931bf329dc82e2ad67ea9a14b3a00159ae621efaeaabac90445c30173e03` |
| `crates/job-queue/tests/owner_beta_daily_schedule.rs` | 942 | `4a55a0c51cb923b4b6660d390ff6049522f5d595bea0b047b10cd1539f43b682` |
| `deploy/db/integration-validation/validate.sh` | 2000 | `5718c601a3df163dcc8af4a7e6bcbb25f141ed36167a9473264acc3057606c4d` |

## Disposition

- Additive0062 exposes exactly three read functions only to the direct
  daily_input_admitter login. Each checks session_user, no SET ROLE and unprivileged
  role attributes, then existing current Owner/exact-rights authority and source
  fence. Fixed definer owner/search_path/timeouts and explicit PUBLIC/runtime-role
  revocations are installed in the migration transaction; no table grants.
- Context requires the exact still-pending current admission receipt/epoch and
  returns navigation metadata. Mode OFF is allowed for preparation. History under
  another policy fails closed. Latest admission identity is not source freshness,
  a sealed filesystem capability or permission to publish.
- Exact committed-input lookup filters current Owner/policy and returns all four
  pins, parent, frontier, ranges/counts and blocked ancestry/source state. Absence is
  explicit. The unblocked-at-read value deliberately excludes freshness/activation;
  the caller must reopen files and independently refresh complete Raw evidence.
- Exact receipt lookup distinguishes started, verified and typed failure records.
  It preserves completed old-epoch history, filters other Owner/policy/stage, and
  validates VERIFIED completion binding against immutable admission evidence.
  matches_current_cycle compares only receipt ID and epoch; it is not eligibility.
- Rust decoders reject malformed identity/hash/result/count/event shapes and expose
  private metadata fields through getters. Fieldless runtime errors retain no raw
  SQL text, paths, provider bodies or credentials. Readback neither takes a slot
  nor finishes/retries a transaction. Existing admission/calendar methods are intact.
- DOWN removes only these three APIs. Populated admission, event, receipt and poll
  state is retained. This isolated additive rollback test does not authorize a
  production schema downgrade or an incompatible release.
- PG harness now requires exactly28 schedule/readback names, each once with noSKIP.
  Full-release ledger requires62 migrations and actual SHA384 comparison; staged
  0060 remains60. Existing runner/publish/recovery and other gates are preserved.

## Written tests

All18 definitions below are WRITTEN_UNEXECUTED. Four are pure malformed-row
decoder checks;14 require real direct service roles and isolated PostgreSQL.
The previous14 schedule definitions remain unchanged. New DB cases cover
pending/default-off context, committed tip, completed/superseded receipts,
principal/SET ROLE/table-grant denial, current Owner/rights revocation,
foreign Owner/policy/stage filtering, started/failure/verified readback,
old epochs, corrupt binding, blocked event identity, exact absence/revocation,
function security and populated additive DOWN/UP preservation.

| Qualified name | Source location | Required lane |
|---|---|---|
| `owner_beta::runner::daily_admission::readback_tests::daily_receipt_decoder_rejects_partial_or_foreign_identity` | `crates/job-queue/src/owner_beta/runner/daily_admission.rs:678` | pure |
| `owner_beta::runner::daily_admission::readback_tests::daily_receipt_decoder_rejects_malformed_hash_and_result_shape` | `crates/job-queue/src/owner_beta/runner/daily_admission.rs:700` | pure |
| `owner_beta::runner::daily_admission::readback_tests::daily_receipt_decoder_preserves_started_vs_noncurrent_completed` | `crates/job-queue/src/owner_beta/runner/daily_admission.rs:726` | pure |
| `owner_beta::runner::daily_admission::readback_tests::daily_receipt_decoder_only_pairs_blocked_event_with_evidence` | `crates/job-queue/src/owner_beta/runner/daily_admission.rs:746` | pure |
| `daily_admitter_context_is_pending_scoped_and_default_off` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:448` | actual DB required |
| `daily_admitter_context_returns_exact_committed_tip` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:485` | actual DB required |
| `daily_admitter_context_rejects_completed_and_superseded_receipts` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:513` | actual DB required |
| `daily_admitter_readback_requires_direct_principal_and_no_table_grants` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:550` | actual DB required |
| `daily_admitter_readback_rechecks_owner_and_exact_rights` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:582` | actual DB required |
| `daily_admitter_readback_filters_foreign_owner_policy_and_stage` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:619` | actual DB required |
| `daily_admitter_receipt_recovery_distinguishes_started_and_completed_failure` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:659` | actual DB required |
| `daily_admitter_receipt_recovery_matches_verified_commit_without_mutation` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:704` | actual DB required |
| `daily_admitter_receipt_keeps_old_epoch_results_historical` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:745` | actual DB required |
| `daily_admitter_receipt_rejects_corrupt_verified_binding` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:780` | actual DB required |
| `daily_admitter_readback_retains_blocked_event_identity` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:794` | actual DB required |
| `daily_admitter_exact_input_absence_and_revocation_are_explicit` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:826` | actual DB required |
| `daily_admitter_readback_security_settings_are_fixed` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:884` | actual DB required |
| `daily_admitter_readback_down_preserves_existing_daily_evidence` | `crates/job-queue/tests/owner_beta_daily_schedule.rs:915` | actual DB required |

## Checks and limits

Full five-file changes and all18 definitions were directly inspected. Scoped
standalone rustfmt2024 and whitespace checks pass. The original SQL-boundary
source and prior integration bodies remain intact (only a new test import).
581 unchanged prior pins match; the three changed prior pins are within this scope.
SQL syntax, compilation, strict lint and actual DB execution remain pending hosted CI.

Tests discard an original successful return and read the committed record afresh;
they do not inject a real network disconnect or kill a process at commit. SQL test
rows are synthetic and do not assert approved filesystem evidence. Future caller
integration must persist/reconcile receipt identity and independently reopen exact
admission/frontier files. No source acceptance is claimed before the candidate CI.

No local Cargo/rustc/Clippy/tests/build/project-helper/SQL/DB execution, provider
request, production change or worker delegation occurred. Full I4 producer/admitter
wiring, persistent state, actual approved-base flow, UID/ACL/crash evidence, I5-I7,
independent integration review, I6 diagrams before main, G4 operations, Owner use,
three trading days and final C_R2 remain open.
