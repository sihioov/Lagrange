# R5-03 Paper SQL ABI

Status: source ABI for `0067_owner_beta_paper_ledger_v1`; no DB execution is claimed here.
Values in examples are synthetic. PostgreSQL `jsonb` cannot detect duplicate object keys, so the
worker must decode the original bytes with a duplicate-key rejecting, `deny_unknown_fields`
decoder before binding JSON to these functions.

## Call envelope and principal boundary

All new public actions return the same envelope:

```json
{
  "schema": "owner-beta-paper-action-v1",
  "action": "SNAPSHOT_PREVIEW",
  "outcome": "OK",
  "control_epoch": 1,
  "account_id": "00000000-0000-4000-8000-000000000001",
  "data": {}
}
```

`control_epoch` and version counters are JSON integers. Monetary values, quantities, and weights
are JSON strings. `account_id` and `control_epoch` may be JSON null for an unconfigured/not-found
result. New prefixed content digests use `sha256:<64 lowercase hex>`; the legacy account-state
digest and preview token remain bare lowercase hex.

`process_config_sha256` is the prefixed digest of the exact canonical bytes read by the protected
runtime. SQL compares it with the selected binding; SQL does not reserialize JSONB to manufacture
that digest. Serving principals are actual direct-login `session_user` values with no `SET ROLE`:
`app` or `worker`. The SQL functions check this and derive Owner identity from authenticated
`app.actor_user_id` (app) or the single configured control row (worker). The GUC alone grants no
authority.

Worker actions save the caller actor and select that configured Owner with LOCAL set_config before
tenant reads. Every return explicitly restores the saved actor; exception handlers restore and
rethrow. Tenant-derived return values are computed before restoration. The caller may start with an
unset, empty or foreign actor (PostgreSQL may retain an empty placeholder for an initially unset
custom GUC). No actor function SET option or parameter privilege is required. The migration-owner
lifecycle transition uses the same explicit save/set/restore pattern. App actions retain
their authenticated Owner checks. Generic worker preview fences use a private scoped predicate;
their delegated generic implementation retains its original caller actor and permissions.

| Function | Exact signature | Principal | Purpose |
| --- | --- | --- | --- |
| `configure_owner_beta_paper_v1` | `(uuid,text,jsonb,jsonb,text) -> jsonb` | `migration_owner` | Create the one OFF account and immutable configure event. `process_binding` must be SQL NULL. |
| `record_owner_beta_paper_consent_v1` | `(uuid,bigint,text,text,boolean,text) -> jsonb` | `app` | Owner-authenticated consent event; revocation advances the control epoch. |
| `transition_owner_beta_paper_v1` | `(bigint,text,jsonb,text) -> jsonb` | `migration_owner` | ENABLE, DRAIN, RECOVERY; protected lifecycle boundary. OFF/RESTORED_OFF currently require the missing WP5 receipt contract and fail closed. |
| `read_owner_beta_paper_state_v1` | `() -> jsonb` | `app` | Owner-filtered state and session summary; available while OFF. |
| `submit_owner_beta_paper_preview_v1` | `(uuid,uuid,bigint,text,text) -> jsonb` | `app` | Queue one MANUAL T-close preview. Final argument is process digest. |
| `queue_owner_beta_paper_target_v1` | `(uuid,bigint,text) -> jsonb` | `worker` | Queue one AUTO T-close preview using the same dedicated preview job. It does not create a target from raw recommendation weights. |
| `snapshot_owner_beta_paper_preview_v1` | `(uuid,uuid,text) -> jsonb` | `worker` | Fenced job-attempt snapshot before T capture/calculation. |
| `publish_owner_beta_paper_preview_v1` | `(uuid,uuid,jsonb,text) -> jsonb` | `worker` | Persist the typed computed result. AUTO applies the target through a private helper in this transaction. |
| `fail_owner_beta_paper_preview_v1` | `(uuid,uuid,jsonb,text) -> jsonb` | `worker` | Fence preview failure against the current lease and attempt. |
| `apply_owner_beta_paper_preview_v1` | `(uuid,text,bigint,text) -> jsonb` | `app` | Apply a READY MANUAL preview only; never callable as worker. |
| `apply_owner_beta_paper_preview_auto_v1` | `(uuid,bigint,text) -> jsonb` | `worker` | Retry a saved READY AUTO result after an earlier E is valued. It cannot calculate or publish a preview. |
| `scan_owner_beta_paper_targets_v1` | `(integer,text) -> jsonb` | `worker` | Bounded earliest-unresolved target scan plus persisted READY AUTO preview identities. |
| `snapshot_owner_beta_paper_target_v1` | `(uuid,bigint,text) -> jsonb` | `worker` | Revalidate target, current account state, and available E selector before execution. |
| `record_owner_beta_paper_wait_v1` | `(uuid,bigint,text,text,text) -> jsonb` | `worker` | Append a nonterminal WAITING/BLOCKED/RECOVERY classification. |
| `commit_owner_beta_paper_settlement_v1` | `(uuid,bigint,jsonb,text) -> jsonb` | `worker` | One atomic T/E execution, ledger, receipt, terminal target, and outbox transaction. |
| `commit_owner_beta_paper_valuation_v1` | `(uuid,bigint,jsonb,text) -> jsonb` | `worker` | Separate exact-E close valuation transaction. |

The last argument on every serving preview/economic worker or app action is
`process_config_sha256`, except state-read and consent, which must work while OFF before a protected
process config exists. Configure and transition are migration-owner operations, not serving
actions. Private helpers have no serving EXECUTE grants.

## Bootstrap, consent, and enable

Configuration accepts exactly this closed contract object:

```json
{
  "contract_id": "owner-beta-paper-v1-draft-1",
  "contract_sha256": "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
  "decision_sha256": "sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
  "cost_profile_id": "KRX_ETF_DEFAULT",
  "cost_profile_version": 1
}
```

The new account has initial cash `"10000000.0000"` KRW, strategy `trend_following@1.0.0`,
benchmark `069500.KRX`, and fast/slow windows 50/200. Configuration creates one deposit and a
CONFIGURED event, stays OFF, and stores no process binding.

The runtime process binding is this exact closed object. UUIDs and hashes below are synthetic:

```json
{
  "schema": "owner-beta-paper-process-v1",
  "owner_user_id": "00000000-0000-4000-8000-000000000001",
  "account_id": "00000000-0000-4000-8000-000000000002",
  "strategy_config_id": "00000000-0000-4000-8000-000000000003",
  "consent_event_id": "00000000-0000-4000-8000-000000000004",
  "consent_version": "owner-beta-paper-v1-draft-1",
  "control_epoch": 2,
  "daily_cohort_sha256": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "financial_contract_sha256": "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
  "code_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "release_manifest_sha256": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "api_binary_sha256": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "worker_binary_sha256": "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
}
```

ENABLE evidence is a closed object with `current_cohort`, `process_binding`,
`process_binding_sha256`, `evaluated_at`, `expires_at`, `expected_epoch`, `new_epoch`, and exactly
three consecutive, unique witnessed trading sessions ending at the current expected Daily session
(including today after the 16:30 KST cutoff when verified calendar coverage permits). Each row has `session_date`, `run_id`, `admission_sha256`,
`source_event_sha256`, and `root_event_sha256`. The current full cohort is also stored separately;
its hash must equal the binding's `daily_cohort_sha256`. SQL verifies the exact DB run/reservation/
source/admission joins and freshness. The installer/runtime remains responsible for proving the
protected file and executable identity.

Consent uses event payload schema `owner-beta-paper-consent-v1` with `consent_version`,
`contract_sha256`, `opt_in`, `expected_epoch`, and `resulting_epoch`. A successful grant/revoke
returns `data.event_id` and `data.opt_in`; replay returns the same event identity. Consent is not
activation authority.

RECOVERY retains its existing closed evidence shape: `schema='owner-beta-paper-recovery-v1'`,
`pending_target_id`, `effective_session`, `reason_code`, and `decision_sha256`. It requires the
earliest unresolved expired beta session and current epoch/binding/rights authority. It accepts
either the existing pending execution path or an EXECUTED/SKIPPED target whose session is
SETTLED_TRADE/SETTLED_NO_TRADE and has no valuation. The latter appends the existing recovery
authorization events while preserving the complete settled session, target status, processing
origin, receipt and ledger fingerprints; only its exact-E valuation resumes. Already valued
sessions, other dates/targets and attempts to pass an earlier unresolved session are rejected.
An exact request-key retry returns its original event.

## Preview job, snapshot, and publication

Both manual and automatic jobs use `job_type='owner_beta_paper_preview'` and payload keys
`schema='owner-beta-paper-preview-job-v1'`, `preview_id`, `input_kind='OWNER_BETA_DAILY_V1'`, and
`apply_mode` (`MANUAL` or `AUTO`). AUTO queue retries return the same preview/job/lineage. Generic
workers must not claim this job kind; the root integration adds the owner-scoped queue claim.

Both submit paths consume the canonical Daily item projection: every `target_weight` is nonnull;
excluded items have numeric zero, no rank and a known nonempty exclusion reason. A bullish signal
selects only `069500.KRX` at weight 1 with rank 1; a cash-only signal has eleven excluded zero rows
and cash weight 1. NULL weights, excluded nonzero weights, other instruments with nonzero weights,
fractional allocations and a noncomplementary cash weight are rejected. The historical publication
format retaining NULL for excluded items is unchanged and is not accepted as a Daily allocation.

`snapshot_owner_beta_paper_preview_v1` returns the current job attempt tuple and these stable
`data` fields (plus the common envelope):

```json
{
  "preview_id": "00000000-0000-4000-8000-000000000010",
  "job_id": "00000000-0000-4000-8000-000000000011",
  "lineage_id": "00000000-0000-4000-8000-000000000012",
  "apply_mode": "AUTO",
  "owner_user_id": "00000000-0000-4000-8000-000000000001",
  "account_id": "00000000-0000-4000-8000-000000000002",
  "strategy_config_id": "00000000-0000-4000-8000-000000000003",
  "recommendation_run_id": "00000000-0000-4000-8000-000000000013",
  "signal_date": "2026-11-04",
  "effective_session": "2026-11-05",
  "daily_pins": {
    "admission_sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    "content_sha256": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
    "manifest_sha256": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
    "policy_sha256": "sha256:4444444444444444444444444444444444444444444444444444444444444444"
  },
  "signal_admission_sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
  "factor_snapshot_sha256": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
  "target_snapshot_sha256": "sha256:6666666666666666666666666666666666666666666666666666666666666666",
  "signal_allocation_sha256": "sha256:8888888888888888888888888888888888888888888888888888888888888888",
  "target_weights": [{"instrument_id":"069500.KRX","weight":"1.000000"}],
  "previous_target_weights": null,
  "config_epoch": 4,
  "daily_activation_epoch": 2,
  "cohort_sha256": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "current_cohort": {},
  "consent_event_id": "00000000-0000-4000-8000-000000000004",
  "control_epoch": 2,
  "calendar_evidence_sha256": "sha256:7777777777777777777777777777777777777777777777777777777777777777",
  "calendar_link": {"schema":"owner-beta-paper-calendar-link-v1"},
  "account_state_version": 3,
  "account_state_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "account_state_preimage": {
    "schema_version": 1,
    "paper_state_version": 3,
    "cash": "10000000.0000",
    "positions": {}
  },
  "cost_profile_id": "KRX_ETF_DEFAULT",
  "cost_profile_version": 1,
  "attempt_id": "00000000-0000-4000-8000-000000000014",
  "attempt_no": 1,
  "worker_id": "synthetic-worker"
}
```

`previous_target_weights` is JSON null when there is no earlier applied beta session. Otherwise it
is the target array from the greatest session E strictly less than this preview's E. It is never
read from `last_signal_allocation_sha256`. This lets unchanged signal/cash/lot outcomes pass through
the calculated NO_TRADE result and still create a T/E session receipt, outbox item, and E valuation.

The worker runs genuine retained T-close capture and `calculate_beta_preview` before publication.
The persisted `result` is the existing API `OwnerBetaPaperCalculationResultDto` closed shape:

```json
{
  "schema": "owner-beta-paper-preview-result-v1",
  "target_snapshot_sha256": "sha256:6666666666666666666666666666666666666666666666666666666666666666",
  "signal_allocation_sha256": "sha256:8888888888888888888888888888888888888888888888888888888888888888",
  "plan_status": "NO_TRADE",
  "no_trade_reasons": ["SIGNAL_UNCHANGED"],
  "equity": "10000000.0000",
  "cash_before": "10000000.0000",
  "available_cash": "10000000.0000",
  "leftover_cash": "10000000.0000",
  "buy_notional": "0.0000",
  "sell_notional": "0.0000",
  "explicit_fees": "0.0000",
  "informational_slippage": "0.0000",
  "decisions": [],
  "orders": [],
  "cost_profile_id": "KRX_ETF_DEFAULT",
  "cost_profile_version": 1,
  "price_observations": []
}
```

`decisions` rows have the exact existing `PreviewDecisionV1` keys:
`instrument_id`, `current_quantity`, `current_value`, `current_weight`, `target_value`,
`target_weight`, `delta_value`, `action`, `skip_reason` (string or null). `orders` rows have the
exact existing `PreviewOrderV1` keys: `instrument_id`, `side`, `quantity`, `raw_price`,
`estimated_execution_price`, `notional`, `commission`, `tax`, `informational_slippage` (all
strings). `price_observations` rows have `instrument_id`, `session_date`, `observed_at`; any
present observation must be for T and cannot be future-dated. NO_TRADE requires empty orders and
zero buy/sell/fee/slippage totals. These fields are the shared API DTO shape; the SQL does not add
account-version or date fields inside `result`.

The outer `publication` object is closed and contains `schema='owner-beta-paper-preview-v1'`,
`preview_id`, `lineage_id`, `apply_mode`, numeric `control_epoch`, `attempt_id`, numeric
`attempt_no`, `worker_id`, `process_config_sha256`, numeric `account_state_version`, bare
`account_state_sha256`, the exact `account_state_preimage`, `cost_profile_id`, numeric
`cost_profile_version`, prefixed `target_snapshot_sha256`, prefixed
`signal_allocation_sha256`, bare 64-hex `preview_token`, and `result` above. SQL rechecks the exact
current job, fresh 60-second lock, active attempt, worker claim, process binding, input hashes, and
account fingerprint. Caller must invoke JobQueue `lock_claim_in`, publication, and job success
settlement in one transaction. AUTO publication persists READY result then calls the private
worker-origin AUTO target helper in that same transaction. MANUAL publication remains READY for
the app-only apply function.

If an earlier E is unresolved, AUTO publication leaves the computed preview READY and returns
`WAITING_EARLIER_SESSION`. `scan_owner_beta_paper_targets_v1` returns it in `data.auto_previews`;
the worker can retry with `apply_owner_beta_paper_preview_auto_v1` after that earlier session is
VALUED. The retry cannot recalculate or replace the saved result. Manual apply only selects
`beta_apply_mode='MANUAL'` and requires the Owner's preview token.

## Target scan and wait

`scan_owner_beta_paper_targets_v1` returns `data.today`, ordered earliest-unresolved `data.targets`,
ordered `data.auto_previews`, and current `data.state`. Target records include pending target ID,
E, session state/origin/reason, lineage, T signal admission/W hash, E execution admission, and
settlement/valuation hashes. Auto preview records include preview/job/lineage IDs, mode/status,
T/E, signal allocation/W hashes, and `blocked_by_earlier_session`.

`snapshot_owner_beta_paper_target_v1` returns the following stable `data` shape. Values are
synthetic; SQL returns JSON null for a missing prior target, E admission, settlement/valuation
receipt, or post-settlement fingerprint:

```json
{
  "pending_target_id":"00000000-0000-4000-8000-000000000020",
  "lineage_id":"00000000-0000-4000-8000-000000000012",
  "recommendation_run_id":"00000000-0000-4000-8000-000000000013",
  "owner_user_id":"00000000-0000-4000-8000-000000000001",
  "account_id":"00000000-0000-4000-8000-000000000002",
  "strategy_config_id":"00000000-0000-4000-8000-000000000003",
  "config_epoch":4,
  "daily_activation_epoch":2,
  "control_epoch":2,
  "daily_pins":{"admission_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111"},
  "current_cohort":{"schema":"owner-beta-daily-cohort-v1"},
  "cohort_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "consent_event_id":"00000000-0000-4000-8000-000000000004",
  "activation_event_id":"00000000-0000-4000-8000-000000000005",
  "process_binding_sha256":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "account_state_version":8,
  "account_state_sha256":"8888888888888888888888888888888888888888888888888888888888888888",
  "initial_account_state_sha256":"7777777777777777777777777777777777777777777777777777777777777777",
  "account_state_preimage":{"schema_version":1,"paper_state_version":8,"cash":"9500000.0000","positions":{"069500.KRX":"10.0000"}},
  "target_weights":[{"instrument_id":"069500.KRX","weight":"1.000000"}],
  "previous_target_weights":null,
  "signal_date":"2026-11-04",
  "effective_session":"2026-11-05",
  "signal_admission_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111",
  "execution_admission_sha256":null,
  "factor_snapshot_sha256":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "target_snapshot_sha256":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "calendar_evidence_sha256":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "calendar_link":{"schema":"owner-beta-paper-calendar-link-v1"},
  "target_status":"PENDING",
  "session_state":"PENDING",
  "settlement_sha256":null,
  "ledger_after_sha256":null,
  "ledger_after_version":null,
  "valuation_sha256":null,
  "receipt_json":{"lineage_id":"00000000-0000-4000-8000-000000000012"}
}
```

`account_state_sha256` and `account_state_preimage` describe the **current** account rows read in
this snapshot, including after settlement. SQL computes the hash with the same canonical
`paper_preview::account_state_sha256` serializer. `initial_account_state_sha256` preserves the
immutable account fingerprint captured when the target was first applied; settlement uses that
value as `account_state_sha256` / `ledger_before_sha256`. `settlement_sha256`,
`ledger_after_sha256`, `ledger_after_version`, `valuation_sha256`, and `receipt_json` are readback
fields from the existing session projection and must be used to resolve an unknown commit before
retrying. `previous_target_weights` is taken from the latest applied beta session with E strictly
earlier than this target. It is never inferred from the current account or control's last signal.
Outcomes `WAITING_SESSION` and `WAITING_ADMISSION` preserve the same target. The current E admission
selector is returned only when the retained selected policy considers it usable.
`record_owner_beta_paper_wait_v1` accepts reason codes `WAITING_PRICE`, `WAITING_ADMISSION`, `HELD`,
`BLOCKED`, or `RECOVERY_REQUIRED` plus prefixed `evidence_sha256`; it appends a session event and
never resets a terminal target.

## Settlement and valuation wire

`commit_owner_beta_paper_settlement_v1(target_id, expected_epoch, settlement, process_digest)`
accepts a closed JSON object, at most 256 KiB:

```json
{
  "schema":"owner-beta-paper-settlement-v1",
  "target_id":"00000000-0000-4000-8000-000000000020",
  "lineage_id":"00000000-0000-4000-8000-000000000012",
  "control_epoch":2,
  "execution_admission_sha256":"sha256:9999999999999999999999999999999999999999999999999999999999999999",
  "execution_input_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "account_state_version":3,
  "account_state_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "ledger_before_sha256":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "outcome":"NO_TRADE",
  "reason_code":"SIGNAL_UNCHANGED",
  "observed_input":{
    "original_retrieved_at":"2026-11-05T00:10:00Z",
    "admitted_at":"2026-11-05T00:11:00Z",
    "usable_at":"2026-11-05T00:12:00Z",
    "precision":"APPLICATION_OBSERVED"
  },
  "orders":[],
  "positions_after":[],
  "cash_after":"10000000.0000",
  "calculation_sha256":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
}
```

Observation timestamps are each no later than actual SQL `clock_timestamp()`, and `usable_at` is
at least both `original_retrieved_at` and `admitted_at`; no ordering between original and admitted
is imposed. E is the immutable economic date; SQL records a separate actual processing timestamp.
Settlement rechecks the current enabled config, Owner, consent, T and E admissions/ancestry, source
links, account fingerprint, earlier-session fence, and processing date. Late processing requires
an explicit RECOVERY event. `NO_TRADE` has empty orders, unchanged cash/positions, zero economic
rows, and still writes terminal target/session receipt/outbox. TRADE is one ETF11 benchmark order
at most, integer quantity, fixed4 raw open/fill/cost/cash strings, no shorts, no negative cash,
and exact cash-chain arithmetic. Slippage is informational because it is embedded in fill price.

Trade `order_ref` and `fill_ref` are canonical nonnil UUIDv5 strings created by shared Rust
`PendingTarget::order_id_for` / `fill_id_for`; SQL validates UUIDv5 form, nonidentity, and the
ETF11 one-order uniqueness bound. SQL does not recompute UUIDv5, install an extension, or implement
a second SHA1 engine. Worker strict decoding and shared Rust code compare the exact references.
Database row IDs remain independently generated UUIDs. Exact settlement retry compares the full
immutable wire receipt; changed replay conflicts.

Valuation accepts the closed `owner-beta-paper-valuation-v1` shape:
`target_id`, numeric `control_epoch`, `valuation_admission_sha256`, `valuation_input_sha256`,
`settlement_sha256`, `ledger_after_sha256`, numeric `account_state_version`, bare
`account_state_sha256`, sorted `closes` rows (`instrument_id`,`raw_close`, fixed4 strings),
`equity`, `cash`, `positions_value`, `currency='KRW'`, `observed_input` with the same timestamp
shape, and `calculation_sha256`. The exact E closes must cover every held position. SQL checks
fixed4 totals against the settled ledger, then inserts/equality-checks `daily_equity` and marks the
session VALUED in one transaction. Same retry is a no-op; a different valuation conflicts.

After settlement or valuation is committed, an actual worker may read back its exact persisted
receipt as REPLAY even after binding, rights, consent or control epoch changes. This internal
terminal readback performs no ledger, projection, event, outbox or equity writes. A changed receipt
conflicts with SQLSTATE 23505. An uncommitted settlement/valuation still requires current economic
authority; REPLAY is not evidence of eligibility or activation. HTTP reads keep their separate
current authentication/rights checks.

Settlement and valuation functions return receipt/event identities in the action envelope. A lost
commit acknowledgement is resolved by reading the immutable target/session receipt; callers never
reset terminal state. Raw duplicate-key proof, protected file bytes, sealed market inputs, price
provenance, sizer/cost arithmetic, and UUIDv5 derivation remain the strict worker boundary.

## Grants and rollback

All new tables are migration-owned with FORCE RLS. The app receives SELECT under the configured
Owner policy; worker does not receive direct beta ledger/table DML or direct beta target/preview
SELECT. Internal security-definer functions own the writes. Direct generic Paper routes remain
available only for unbound accounts and are wrapped/fenced before calling their renamed private
implementations.

| Role | New serving function EXECUTE |
| --- | --- |
| `app` | state read, consent, manual submit, manual apply |
| `worker` | auto queue, preview snapshot/publish/fail, AUTO retry, target scan/snapshot/wait, settlement, valuation |
| `migration_owner` | configure, lifecycle transition |
| `PUBLIC`, `admin`, `audit_writer`, `research_writer`, `daily_input_admitter` | none |

`0067.down.sql` checks every beta control/event/lineage/session/target/preview/job history under
the migration-owner FORCE-RLS policy before any destructive DDL. Any such history blocks rollback.
For an empty installation it drops wrappers, restores original generic function names, owners,
search paths, and exact grants, then removes beta columns/constraints/tables.
