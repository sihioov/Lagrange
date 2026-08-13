# Recommendation-to-Paper Rebalance Preview Design

**Date:** 2026-08-13

**Status:** Approved for implementation

**Scope:** Backend-only indicative rebalance previews and explicit Paper application for completed recommendations

## 1. Outcome

Lagrange Station will let an authenticated owner take a completed recommendation,
preview how it would rebalance one active Paper account, and explicitly queue that
target for the next published KRX trading session. The preview explains estimated
buys, sells, quantities, fees, slippage, skipped trades, and leftover cash using
the same deterministic sizing and cost model as Paper execution.

This is not an immediate trade endpoint. Applying a preview creates one durable
`pending_targets` row. The existing Paper runner remains the only component that
writes orders, fills, positions, or cash, and it recalculates quantities at the
actual next-session open.

No UI work is included in this change.

## 2. Existing State and Missing Seam

The repository already contains:

- completed recommendation runs, items, immutable target portfolios, and exact
  dataset lineage;
- active Paper accounts and immutable strategy-binding history;
- a fixed-point `plan_rebalance` implementation with integer lots,
  sell-before-buy ordering, affordability, commissions, taxes, slippage,
  minimum-trade rules, and rebalance thresholds;
- a Paper runner that reads authoritative cash and positions, replans at the
  raw session open, and atomically writes ledger rows;
- scheduled recommendation-to-Paper publication for bindings with
  `auto_apply_recommendations=true`;
- transaction-scoped Paper execution preflight for account, binding, dataset,
  entitlement, and pending-target state.

There is no member-facing backend boundary that shows the effect of a completed
recommendation on a specific Paper account before it is applied. Manual
recommendation runs deliberately never enter `pending_targets`, so a user cannot
explicitly adopt one without enabling the scheduled auto-apply path.

The earlier recommendation-pipeline design excluded *automatic* Paper
publication from an ad-hoc manual preview. This design preserves that boundary:
preview computation has no trading side effect, and only a separate authenticated
apply request may create a pending target.

## 3. Chosen Approach

Three approaches were considered:

1. **Compute synchronously in the API process.** This makes the response simple,
   but gives the serving process direct curated-file access, holds an HTTP request
   through filesystem work, and duplicates the worker's data boundary.
2. **Queue a preview for the Paper runner and persist the derived result.** The
   API owns request, tenant, and apply semantics; the trusted Paper worker reads
   curated prices and runs the existing sizing model.
3. **Estimate from database rows only.** The database has positions and cash but
   no authoritative per-instrument price snapshot sufficient for quantities and
   fees, so this would either omit the useful result or fabricate prices.

Approach 2 is selected. It keeps curated market data in the existing worker
boundary, reuses the exact Paper model, and gives retries and replica-safe claims
through the existing job queue.

## 4. Product Scope

### 4.1 Included

- Requesting an indicative preview for one owned, active Paper account and one
  owned, successful recommendation run.
- Requiring the account's active binding to reference the recommendation's exact
  strategy configuration.
- Asynchronous computation by `paper-runner` through a dedicated typed queue job.
- Persisted preview status and a bounded, immutable result document.
- Estimated sells, buys, integer quantities, notionals, explicit fees,
  informational slippage, skipped reasons, total equity, available cash, and
  leftover cash.
- Exact price, cost-profile, recommendation, dataset, target-portfolio, and
  account-state provenance.
- Explicit application of a `READY` preview to one next-session pending target.
- Durable idempotency across process restarts and replica-safe concurrency.
- Execution-time revalidation by the existing Paper runner.
- OpenAPI and backend integration/contract coverage.

### 4.2 Excluded

- Web or mobile UI.
- Live brokerage orders.
- Immediate Paper order/fill creation from an HTTP request.
- Promising that indicative quantities equal next-session quantities.
- Intraday or external quote-provider integration.
- Dynamic stock universes or new strategy formulas.
- Changing scheduled auto-apply semantics.

## 5. Domain Semantics

### 5.1 Indicative price basis

The preview uses the recommendation run's exact `as_of` raw close from its
attested immutable curated dataset. That price exists when the recommendation is
successful and is reproducible from its dataset pin. It is not relabelled as an
open or a live quote.

The preview response therefore carries:

- `price_basis = RECOMMENDATION_CLOSE`;
- `price_date = recommendation_run.as_of`;
- dataset-version UUID and manifest hash;
- a warning that actual orders are replanned at the next session's raw open.

The same raw close is passed as the sizer's price basis. The account cost profile
then applies its versioned side-specific slippage and fee rules exactly as Paper
execution does. Explicit fees and informational slippage remain separate so
slippage is never charged twice.

### 5.2 Preview versus execution

A preview is derived information, not an order and not a ledger event. It may be
rendered and audited, but it never becomes the authority for executed quantity.

Applying a preview persists only canonical target weights and lineage in
`pending_targets`. At the effective session, the existing executor reloads
authoritative cash and positions, reads actual raw opens, and calls the same
planner again. Price or account changes can therefore change actual quantities.

### 5.3 Binding and opt-in semantics

The account must have an active binding to the recommendation run's exact active
strategy configuration both when the preview is requested and when it is
applied.

`auto_apply_recommendations` is not required for explicit manual application.
That flag remains solely the consent boundary for scheduled automatic
publication. A manual apply is a separate, authenticated user action.

## 6. Persistence Model

### 6.1 `paper_rebalance_previews`

A new tenant table stores one immutable preview request/result:

- `id`, `owner_user_id`, `account_id`, `recommendation_run_id`;
- `target_portfolio_id`, `strategy_config_id`;
- `job_id` referencing one `paper_rebalance_preview` queue job;
- status: `PENDING`, `RUNNING`, `READY`, `FAILED`, or `APPLIED`;
- `price_basis`, `price_date`, exact dataset UUID and manifest hash;
- cost-profile ID/version;
- the proposed effective KRX trading date;
- canonical account-state fingerprint;
- canonical target-portfolio fingerprint;
- `preview_token` and bounded `result_json` when ready;
- typed, sanitized `error_json` when failed;
- `pending_target_id`, timestamps, and terminal metadata.

The table uses FORCE RLS on `owner_user_id`. The app role can create and read an
owned request only through actor-scoped transactions. Result/status columns are
worker-owned. Application columns are written through a narrow app-only
`SECURITY DEFINER` boundary. Neither role can rewrite immutable preview identity.

`result_json` is a cache of a deterministic calculation, not a financial ledger.
It has a schema-version field and a strict size limit. Orders and fills continue
to exist only in their current ledger tables.

### 6.2 Queue identity

The API creates the preview row and one `jobs` row atomically under the shared
per-owner job-capacity advisory lock. The job type is
`paper_rebalance_preview`, and its payload contains only the preview UUID.

The durable idempotency namespace is `paper-preview:manual:<public-key>`. A
retry with the same key and the same public request returns the existing preview
and job; a different account/run returns `IDEMPOTENCY_KEY_MISMATCH`. Preview jobs
count toward the existing global per-owner active-job limit.

### 6.3 Pending-target lineage

`pending_targets` gains explicit origin fields:

- `source_kind`: `LEGACY`, `SCHEDULED_RECOMMENDATION`, or
  `MANUAL_RECOMMENDATION`;
- nullable `recommendation_run_id` with an all-or-none lineage constraint;
- the existing exact strategy, computation date, target weights, dataset UUID,
  dataset version, and manifest hash remain mandatory for recommendation
  origins.

The scheduled bridge is updated to write `SCHEDULED_RECOMMENDATION` and its exact
run UUID. Existing rows remain `LEGACY` without fabricated provenance.

The current unique `(account_id, effective_date)` rule remains authoritative. A
manual and scheduled target for the same account/session may resolve to the same
row only if every immutable field is identical. Any other collision fails
closed; neither target overwrites the other.

### 6.4 Paper account state version

`accounts` gains a monotonic `paper_state_version`. Migration-owned triggers on
`cash_ledger` and `positions` increment the owning account's version for every
insert, update, or delete. The trigger function has a fixed search path and does
not trust an application-supplied owner UUID; it resolves the account row by the
foreign key.

Preview computation stores this version together with a canonical cash/position
fingerprint. Apply locks the account row, compares both, and retains that lock
through pending-target insertion. This turns a concurrent ledger mutation into
one of two ordered outcomes: a mutation committed first makes the preview stale;
an apply committed first was valid at its linearization point and the later
mutation is handled by the executor's mandatory replanning.

## 7. API Contract

### 7.1 Request a preview

`POST /api/v1/paper/accounts/{account_id}/recommendation-previews`

Request:

```json
{
  "recommendation_run_id": "uuid"
}
```

An `Idempotency-Key` is required. The response is `202 Accepted` for a new
request and `200 OK` for a durable replay. It contains preview ID, job ID,
status, account ID, run ID, and timestamps. No curated path or internal worker
identity is exposed.

### 7.2 Read a preview

`GET /api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}`

`PENDING` and `RUNNING` return metadata only. `FAILED` returns a stable typed
error code and sanitized message. `READY` and `APPLIED` return:

- pricing basis/date and recommendation/dataset lineage;
- the proposed effective KRX trading date and validity state;
- account equity, cash before, cash available after planned sells, and expected
  leftover cash;
- totals for buy notional, sell notional, explicit fees, informational
  slippage, and order count;
- canonical per-instrument decisions containing current quantity/value/weight,
  target value/weight, signed delta, action, estimated quantity/execution price,
  notional, fee components, and typed skip reason;
- a `preview_token` and mandatory indicative-result warning.

All fixed-point values are decimal strings. Unknown or non-finite numeric JSON is
rejected rather than normalized.

### 7.3 Apply a preview

`POST /api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply`

Request:

```json
{
  "preview_token": "64-lowercase-hex"
}
```

An `Idempotency-Key` is required at the HTTP boundary. The durable authority is
the preview row itself: a successfully applied preview points to exactly one
pending target, so retries return that same target across API restarts.

The response contains preview ID, pending-target ID, effective date, source kind,
and status. It does not contain orders or fills because none exist yet.

## 8. Preview Worker Flow

`paper-runner` claims only `paper_rebalance_preview` jobs in addition to its
existing pending-target and valuation cycle. It never claims recommendation or
backtest jobs.

For one claim it:

1. parses the closed payload containing one preview UUID;
2. locks and reloads the exact preview, owner, account, active binding,
   successful recommendation, target portfolio, dataset pin, and entitlement;
3. takes the shared account-scoped advisory lock and snapshots authoritative
   cash plus canonical positions;
4. validates the account's cost-profile ID and version;
5. resolves and stores the first attested KRX `TRADING` session strictly after
   both the run date and the current Seoul calendar date;
6. releases the database transaction and reads every required raw close from
   the attested curated partition;
7. calls `plan_rebalance` in `spawn_blocking` with the same target, lot-size
   fallback, and cost-profile semantics as Paper execution;
8. builds a versioned result document and hashes its canonical immutable inputs,
   including the proposed effective session;
9. reacquires the account advisory lock and re-reads the account fingerprint;
10. publishes `READY` and settles the queue job atomically only if the fingerprint
   is unchanged.

If account state changed during computation, the worker does not publish a stale
preview. It returns a typed transient outcome so the existing bounded retry
budget can recompute. Deterministic missing-price, invalid-lineage, invalid-cost,
or invalid-target failures settle the preview and job permanently with a stable
code. Lease loss or cancellation prevents publication.

## 9. Apply Transaction

Application runs in one actor transaction and performs no filesystem I/O. A
narrow function/repository boundary:

1. locks the owned preview row;
2. returns the existing pending target for an already applied preview;
3. requires `READY` and an exact preview-token match;
4. takes the same account-scoped advisory lock used by Paper execution;
5. locks and revalidates the active Paper account, exact active binding,
   successful recommendation, immutable target portfolio, dataset readiness,
   entitlement, and published KRX calendar;
6. compares the locked account's monotonic state version and recomputed canonical
   cash/position fingerprint with the preview snapshot;
7. revalidates the preview's proposed effective session as the first attested
   KRX `TRADING` session strictly after both the recommendation date and the
   preview-computation Seoul calendar date, and requires it still to be in the
   future at apply time;
8. inserts the canonical positive target weights and exact lineage as
   `MANUAL_RECOMMENDATION`, or validates every immutable field of an existing
   account/session target;
9. marks the preview `APPLIED` with that pending-target UUID; and
10. commits all changes together.

`REBALANCE_PREVIEW_STALE` is returned when the account state, binding, target,
dataset, entitlement, or proposed execution session no longer matches. A preview
automatically becomes stale when its proposed session date arrives; it is never
retargeted silently to a later session. No pending target, order, fill, cash, or
position row is written on that path. The user must request a fresh preview.

The account advisory lock is also acquired by execution preflight and held
through the existing `execute_session_in_tx` commit. Apply and Paper execution
therefore cannot observe or write through each other.

## 10. Error Classification

Stable public codes include:

- `REBALANCE_PREVIEW_NOT_READY`;
- `REBALANCE_PREVIEW_STALE`;
- `REBALANCE_PREVIEW_FAILED`;
- `REBALANCE_PREVIEW_PRICE_UNAVAILABLE`;
- `REBALANCE_PREVIEW_ACCOUNT_UNAVAILABLE`;
- `REBALANCE_PREVIEW_BINDING_REQUIRED`;
- `REBALANCE_PREVIEW_DATA_BLOCKED`;
- `REBALANCE_PREVIEW_ENTITLEMENT_REQUIRED`;
- `REBALANCE_PREVIEW_CONFLICT`;
- `REBALANCE_PREVIEW_CAPACITY_EXCEEDED`.

Tenant mismatches are `RESOURCE_NOT_FOUND`. Permanent data, schema, target, and
cost errors do not retry. Database connectivity, serialization/deadlock, lease,
and account-changed-during-compute outcomes are transient. Internal paths,
queries, database roles, and raw child/SQL errors are never returned.

## 11. Security and Concurrency

- Actor UUIDs come only from the authenticated session; request bodies never
  supply an owner UUID.
- Preview and target rows use FORCE RLS and owner-local predicates.
- Curated files remain reachable only by trusted workers.
- The app cannot publish a preview result or forge scheduled lineage.
- The worker cannot manufacture an application consent event.
- `SECURITY DEFINER` functions have fixed `pg_catalog, pg_temp` search paths,
  migration-owner ownership, explicit role grants, and exact argument checks.
- Recommendation, target, dataset, entitlement, calendar, binding, and account
  rows are locked before application.
- The account advisory lock serializes apply against Paper execution, while
  `paper_state_version` captures every ledger/position table mutation and its
  locked account row supplies the database serialization point.
- Result documents and queue payloads are strictly bounded before parsing or
  persistence.
- Preview tokens are canonical hashes used as optimistic preconditions; they
  are not authentication secrets.

## 12. Verification

### 12.1 Model and worker

- The preview uses the same planner and cost profile as session execution.
- Sells precede buys, integer lots hold, cash remains non-negative, and fee plus
  slippage totals match the versioned model.
- All-cash and no-trade portfolios remain valid results.
- Missing held-instrument or target prices fail closed.
- Account mutation during compute prevents stale publication and retries.
- Lease loss/cancellation cannot publish a result.
- Two workers publish one result and one job settlement.

### 12.2 API and database

- Foreign account, run, preview, and target IDs are indistinguishable from
  missing resources.
- Inactive/mismatched bindings, failed runs, blocked datasets, and revoked
  entitlements create no preview or pending target.
- Durable preview submission replays after an API restart; payload mismatch is
  rejected without poisoning later correct retries.
- Preview jobs share the global per-owner capacity limit with every API job
  producer.
- Cash or position mutation after `READY` produces
  `REBALANCE_PREVIEW_STALE` and zero target/order/fill writes.
- A preview whose proposed KRX session has arrived cannot be shifted forward and
  must be recomputed.
- Concurrent apply calls create one pending target and both resolve to it.
- Scheduled/manual same-session conflicts validate full immutable identity and
  never overwrite.
- Closed calendar days are skipped when selecting the effective date.
- Applying creates no orders or fills; the later Paper cycle replans at the
  actual open and may legitimately differ from the preview.
- Migration up/down, grant, RLS, search-path, rollback, rerun, and future-ledger
  contracts pass on PostgreSQL 18.

### 12.3 Contract and regression

- OpenAPI authored/generated contracts declare every new route, shape, and
  stable error code.
- Existing recommendation, scheduled auto-apply, Paper execution, Paper
  valuation, parity, queue, and tenancy suites remain green.
- Workspace format, check, and warning-denying Clippy gates pass.

## 13. Rollout and Operations

The schema migration lands before the API and worker binaries. During a rolling
deployment, old Paper runners ignore the new typed jobs and new runners claim
only their exact type. Preview requests remain queued until a new Paper runner
is available; they cannot be misexecuted as targets.

Metrics and logs report counts and stable error codes only: queued/running/ready/
failed previews, compute latency, stale retries, apply outcomes, and oldest
pending-preview age. They never log preview result documents, account balances,
database URLs, tokens, or curated paths.

Rollback first disables new preview submission/claiming, then removes the new
API/worker code, and only then applies the reversible migration after a guard
confirms there are no nonterminal previews or manual recommendation targets that
would lose required lineage.
