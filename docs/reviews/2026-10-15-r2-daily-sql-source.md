# R2 I2 — complete 0060 candidate source disposition

2026-10-15; coordinator WP-4/I2, Wave37, R2-04/R2-09.
Baseline `7ffce8811e89ca7ba840a18ee7d13643aa4366fb`.
Status: complete source candidate for hosted verification; all new DB cases
**WRITTEN_UNEXECUTED**. This is coordinator inspection, not an independent review
or compiler, database, runtime or Owner acceptance.

The [fixed I2 contract](2026-10-15-r2-daily-sql-implementation-contract.md),
[D5 overrides](2026-10-14-r2-daily-input-design-amendments.md),
[D6 disposition](2026-10-14-r2-daily-input-amendments-review.md) and
[D4 bytes](../contracts/owner-beta-daily-etf11-input-v1.md) remain preserved.
The last executed candidate remains `57ca4d1390b025e7efbe9ac452c614a8b5618889`,
[run38672658570 attempt1 SUCCESS](https://github.com/sihioov/Lagrange/actions/runs/38672658570).
Its bootstrap83, Rust99, chart6, affected transport3, strict lint/format and PG59
acceptance does not execute this migration.

## Source disposition

| Area | Concrete source and disposition |
| --- | --- |
| Role/catalog/epoch | [UP](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1): direct role preflight, protected membership paths, ten explicit migration-owned FORCE-RLS tables, disabled defaults, narrow config column grants, immutable identities and overflow checks. No login or runtime activation is created. |
| Policy/file projection | [Private validators](../../migrations/0060_owner_beta_daily_inputs.up.sql#L282): exact keys/scalars/constants, redacted request allowlist, bounded copy chain, cycle/identity/future checks. SQL receives verified projections; it does not open source files or prove canonical byte/hash correspondence. |
| Authority and receipts | [Authority](../../migrations/0060_owner_beta_daily_inputs.up.sql#L406): control/Owner/rights locks, current time after waits, stage/latest-receipt fencing, typed completion, separate schedule freshness and permanent receipt history. |
| Calendar and admission | [Calendar](../../migrations/0060_owner_beta_daily_inputs.up.sql#L631), [manifest](../../migrations/0060_owner_beta_daily_inputs.up.sql#L754), [admission](../../migrations/0060_owner_beta_daily_inputs.up.sql#L945): explicit civil-day proof, original source identity, committed conflict events, exact SQL envelope, protected base, cumulative prefix and one-session extension. |
| Reservation/publication | [Reservation](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1123), [publication gate](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1184): permanent dedup key, existing capacity lock, latest eligible input only, current claim and authority, config/activation epochs, receipt freshness and post-wait clock checks. |
| Queue consistency | [Immediate guards](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1231), [deferred matrix](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1437): immutable tags/pins/output, DB lease/backoff stamps, retry/recovery/cancel drain, exact five-state linkage and success claim/items. |
| History and DOWN | [DOWN](../../migrations/0060_owner_beta_daily_inputs.down.sql#L1): finite locks, all-tenant unused-only refusal, restored-OFF history rejection, exact historical function restoration and previous app grants. Old migrations0001–0059 are not edited. |
| Official harness | [staged0060](../../deploy/db/integration-validation/validate.sh#L1469), [history fixture](../../deploy/db/integration-validation/preflight-0060.sql#L1), [direct roles](../../deploy/db/integration-validation/actual-role-0060.sql#L1), [DOWN rehearsal](../../deploy/db/integration-validation/rollback-0060-guard.sql#L1): sequential0059→0060, two60-checksum ledgers, eight historical rows, no-op, real direct-role denials and hidden-Owner rollback. Earlier0039/40/41 and bootstrap83 remain. |

The coordinator additionally found that a completed BLOCKED receipt could otherwise
accept a different admission with the same observation hash. Internal receipts now
carry `completion_binding_sha256`: a domain-separated hash of the SQL operation's
JSONB projection, checked on replay. This is an internal operation identity, **not a
D4 artifact hash**; D4 canonical serialization and supplied external hashes are
unchanged. Calendar and explicit block completion receive the same operation fence.
The new regression changes the admission hash while retaining the frontier and
requires23514. This is a narrow I2-D03/A6 implementation refinement; no new serving
signature or privilege is added. The immutable Wave36 brief remains historical.

Other source corrections made during assembly: NULL rejection, ordered request
pairs, full replay descriptors, post-lock time sampling, current attempt lock order,
unknown daily tag rejection, copy/original binding, duplicate cash rows/document
counts, actual effective CONNECT/USAGE denial fixtures including PUBLIC grants,
and guaranteed scratch cleanup after returned errors or task panic.

## Definition inventory

The following are definitions only. Each named test must have one successful
libtest row in the DATABASE_URL-backed official lane. The harness separately
requires73 unique fixed subcase PASS rows and actual exit/no-SKIP records for all12
existing Cargo labels. It rejects absent or duplicate expected outcomes. Original
test bodies are unchanged; the only pre-existing-section addition is the two0060
`include_str!` constants.

| Named case | Definition | Current evidence |
| --- | --- | --- |
| `daily_0060_role_preflight` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13950) | WRITTEN_UNEXECUTED |
| `daily_0060_up_preserves_histories` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13954) | WRITTEN_UNEXECUTED |
| `daily_0060_repeat_is_noop` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13958) | WRITTEN_UNEXECUTED |
| `daily_0060_catalog_authority` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13962) | WRITTEN_UNEXECUTED |
| `daily_0060_direct_principal_denials` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13966) | WRITTEN_UNEXECUTED |
| `daily_0060_config_owner_optin` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13970) | WRITTEN_UNEXECUTED |
| `daily_0060_config_epoch` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13974) | WRITTEN_UNEXECUTED |
| `daily_0060_control_epoch` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13978) | WRITTEN_UNEXECUTED |
| `daily_0060_calendar_projection` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13982) | WRITTEN_UNEXECUTED |
| `daily_0060_calendar_before_admission` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13986) | WRITTEN_UNEXECUTED |
| `daily_0060_calendar_gap_cutoff` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13990) | WRITTEN_UNEXECUTED |
| `daily_0060_admission_transport` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13994) | WRITTEN_UNEXECUTED |
| `daily_0060_admission_parent_chain` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L13998) | WRITTEN_UNEXECUTED |
| `daily_0060_admission_conflict_replay` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14002) | WRITTEN_UNEXECUTED |
| `daily_0060_poll_rate_and_crash` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14006) | WRITTEN_UNEXECUTED |
| `daily_0060_receipt_authority` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14010) | WRITTEN_UNEXECUTED |
| `daily_0060_event_and_freshness` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14014) | WRITTEN_UNEXECUTED |
| `daily_0060_reservation_dedup` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14018) | WRITTEN_UNEXECUTED |
| `daily_0060_capacity_deferral` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14022) | WRITTEN_UNEXECUTED |
| `daily_0060_latest_only` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14026) | WRITTEN_UNEXECUTED |
| `daily_0060_direct_daily_spoof` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14030) | WRITTEN_UNEXECUTED |
| `daily_0060_valid_publish` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14034) | WRITTEN_UNEXECUTED |
| `daily_0060_terminal_immutability` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14038) | WRITTEN_UNEXECUTED |
| `daily_0060_claim_fences` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14042) | WRITTEN_UNEXECUTED |
| `daily_0060_retry_and_recovery` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14046) | WRITTEN_UNEXECUTED |
| `daily_0060_cancel_drain` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14050) | WRITTEN_UNEXECUTED |
| `daily_0060_deferred_consistency` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14054) | WRITTEN_UNEXECUTED |
| `daily_0060_revoke_publish_races` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14058) | WRITTEN_UNEXECUTED |
| `daily_0060_cutoff_while_waiting` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14062) | WRITTEN_UNEXECUTED |
| `daily_0060_down_unused` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14066) | WRITTEN_UNEXECUTED |
| `daily_0060_down_all_tenants` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14070) | WRITTEN_UNEXECUTED |
| `daily_0060_down_atomicity` | [source](../../tests/integration/migration-contract/tests/migration_contract.rs#L14074) | WRITTEN_UNEXECUTED |

Private frozen inventory: `target/r2-daily-beta/wp-4-integration/wave37-I2-definition-inventory.json`.
It carries every definition line, all73 subcase names and these limits:

- Cutoff/latest-date fixtures adapt a private scratch-DB clock into the unchanged
  pure cutoff calculation. Real two-connection waits are observed through blocking
  PIDs; the fixture is not an observation of an actual market-time boundary.
- Each isolated DOWN category removes only its dependency constraints/triggers in
  a transaction that is rolled back after refusal. This isolates the guard; it is
  not a supported bypass or evidence deletion path. Hidden-Owner tests use real RLS.
- The live exhausted-failure subcase forces deferred constraints immediately and
  rolls back; the exhausted orphan path commits. Earlier transient failure/retry
  and cancellation paths commit their complete states.
- Synthetic base/session/source projections exercise DB gates only. No approved
  artifact, copied file reconstruction, provider response parser, cash normalization,
  on-host ACL, installed release or production data is validated by these fixtures.
- Named categories do not prove every possible JSON or hostile DML permutation.
  Actual SQL/compiler/strict-lint results remain pending; all prior failures remain.

## Static checks and next gate

Allowed checks: scoped standalone edition2024 rustfmt, `bash -n` for the two changed
shell files, whitespace, inert source pin/definition/diff comparisons. No local
Cargo, compiler, Clippy, tests, database, project validator, self-test, Docker or
provider execution occurred. The official workflow is unchanged.

All118 old migration files and19 unchanged baseline references plus the fixed I2
contract retain their hashes (138 preserved pins). Five prior reference files are
authorized implementation targets. The source/input receipt records the ten owned
files, prior pins and definition inventory; previous receipts are not overwritten.

Source assembly/disposition is complete for one exact-candidate hosted attempt after
an all-event existing-run query and remote-head verification. No final I2 acceptance
is claimed until that run's full evidence is recovered. Any failure gets a bounded
new-candidate correction only after recovery; no old run is rerun or canceled.

R2-04/09 remain DOING. Provider/copy reconstruction/admission, I3–I7, runtime UID/ACL,
installer filesystem rollback, I6 diagrams before main integration, operational
activation, Owner use, three trading days and final C_R2 remain open. No further
worker delegation or production operation is authorized by this report.
