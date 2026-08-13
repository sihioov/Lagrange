# Recommendation-to-Paper Rebalance Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a backend-only, asynchronous, reproducible rebalance preview for a successful recommendation and an explicit, stale-safe action that queues the target for Paper execution.

**Architecture:** The API atomically creates an owner-scoped preview row plus a typed queue job. `paper-runner` reads immutable recommendation-close prices, calls the existing fixed-point `plan_rebalance`, and atomically publishes a bounded result with queue settlement. A separate app-only PostgreSQL boundary locks and revalidates the preview, account state version, binding, dataset, entitlement, calendar, and target before creating one manual recommendation pending target.

**Tech Stack:** Rust 1.97.1, Axum 0.8, SQLx 0.9, PostgreSQL 18, Tokio, existing `portfolio-model`/`market-data`/`job-queue`, JSON/OpenAPI contract generation.

---

## File Map

- Create `migrations/0038_paper_rebalance_previews.up.sql`: tables, triggers,
  constraints, least-privilege grants, scheduled/manual lineage functions, and
  app/worker security boundaries.
- Create `migrations/0038_paper_rebalance_previews.down.sql`: guarded reversible
  teardown and restoration of the 0037 worker boundary.
- Modify `tests/integration/migration-contract/tests/migration_contract.rs`:
  ledger count, up/down/rerun, role grants, RLS, function metadata, trigger, and
  behavioral contracts.
- Create `crates/job-queue/src/paper_preview.rs`: closed payload, account snapshot,
  immutable price loading, pure preview calculation, canonical hashing, typed
  errors, claim/heartbeat/publish lifecycle.
- Modify `crates/job-queue/src/lib.rs`: export the focused preview module.
- Create `crates/job-queue/tests/paper_preview.rs`: pure calculation, real curated
  data, worker lifecycle, cancellation, retry, lease, and concurrency coverage.
- Create `crates/api-server/src/repos/rebalance_previews.rs`: actor-scoped submit,
  durable replay, capacity, read, and apply repository.
- Modify `crates/api-server/src/repos/mod.rs`: export the repository.
- Modify `crates/api-server/src/http/state.rs`: construct the repository.
- Modify `crates/api-server/src/http/dto.rs`: strict preview, result, decision,
  error, and apply DTOs.
- Modify `crates/api-server/src/http/paper.rs`: preview create/get/apply handlers.
- Modify `crates/api-server/src/http/mod.rs`: register the three routes.
- Modify `crates/api-server/src/observability/metrics.rs`: fixed-label preview
  request/apply counters for the API process.
- Modify `crates/api-server/src/contract.rs`: paths, methods, permissions, and
  stable error-code declarations.
- Create `crates/api-server/tests/http_paper_rebalance.rs`: real PostgreSQL HTTP,
  restart replay, tenant, capacity, stale, apply, and concurrency tests.
- Modify `crates/api-server/tests/common/mod.rs`: expose only the minimal helper
  needed to rebuild state and run the real Paper worker against a QA curated root.
- Modify `crates/api-server/src/paper_runner.rs`: process at most one typed preview
  claim per cycle without changing target/valuation ordering guarantees.
- Modify `crates/api-server/src/bin/paper-runner.rs`: queue configuration and
  preview worker identity/lease controls.
- Modify `crates/job-queue/src/recommendation/publish.rs`: call the new scheduled
  bridge signature with exact run UUID.
- Modify `crates/job-queue/tests/recommendation_publish.rs`: scheduled source/run
  lineage and collision regressions.
- Modify `crates/api-server/tests/paper_runner.rs`: manual preflight does not
  require auto-apply; scheduled preflight still does.
- Modify `apps/api-server/openapi.json`: authored routes/schemas/error enum.
- Regenerate `apps/api-server/generated/openapi.ts` using the existing script.
- Modify `crates/api-server/tests/openapi_contract.rs`: assert the three paths,
  strict component shapes, and stable preview error codes.

---

### Task 1: PostgreSQL schema and least-privilege boundaries

**Files:**
- Create: `migrations/0038_paper_rebalance_previews.up.sql`
- Create: `migrations/0038_paper_rebalance_previews.down.sql`
- Modify: `tests/integration/migration-contract/tests/migration_contract.rs`

- [ ] **Step 1: Write migration RED assertions**

Add a focused contract that applies all migrations and asserts:

```rust
assert_table_rls(&pool, "paper_rebalance_previews", true, true).await;
assert_function_security(
    &pool,
    "public.apply_paper_rebalance_preview(uuid,uuid,text,date)",
    true,
    "migration_owner",
    "pg_catalog, pg_temp",
).await;
assert_execute_matrix(
    &pool,
    "public.apply_paper_rebalance_preview(uuid,uuid,text,date)",
    &["app"],
).await;
assert_column_privilege_absent(&pool, "worker", "paper_rebalance_previews", "owner_user_id").await;
```

Add real role calls proving app cannot publish a result, worker cannot apply a
preview, a direct app insert cannot forge a recommendation-origin pending target,
and every `cash_ledger`/`positions` mutation increments
`accounts.paper_state_version`.

- [ ] **Step 2: Run the migration contract and verify RED**

Run:

```powershell
cargo test -p migration-contract recommendation_preview -- --nocapture --test-threads=1
```

Expected: FAIL because migration 0038, the table, and functions do not exist.

- [ ] **Step 3: Add the 0038 up migration**

Create `paper_rebalance_previews` with this identity and lifecycle:

```sql
CREATE TABLE public.paper_rebalance_previews (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    account_id uuid NOT NULL REFERENCES public.accounts(id) ON DELETE CASCADE,
    recommendation_run_id uuid NOT NULL REFERENCES public.recommendation_runs(id),
    target_portfolio_id uuid NOT NULL REFERENCES public.target_portfolios(id),
    strategy_config_id uuid NOT NULL REFERENCES public.user_strategy_configs(id),
    job_id uuid NOT NULL UNIQUE REFERENCES public.jobs(id),
    status text NOT NULL DEFAULT 'PENDING',
    price_basis text NOT NULL DEFAULT 'RECOMMENDATION_CLOSE',
    price_date date NOT NULL,
    proposed_effective_date date,
    dataset_version_id uuid NOT NULL REFERENCES public.dataset_versions(id),
    dataset_manifest_sha256 text NOT NULL,
    cost_profile_id text,
    cost_profile_version integer,
    account_state_version bigint,
    account_state_sha256 text,
    target_portfolio_sha256 text NOT NULL,
    preview_token text,
    result_json jsonb,
    error_json jsonb,
    pending_target_id uuid REFERENCES public.pending_targets(id),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    started_at timestamptz,
    completed_at timestamptz,
    applied_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT paper_rebalance_previews_status_check
      CHECK (status IN ('PENDING','RUNNING','READY','FAILED','APPLIED')),
    CONSTRAINT paper_rebalance_previews_terminal_shape_check CHECK (
      (status IN ('PENDING','RUNNING') AND result_json IS NULL AND error_json IS NULL AND preview_token IS NULL)
      OR (status = 'READY' AND result_json IS NOT NULL AND error_json IS NULL AND preview_token IS NOT NULL AND pending_target_id IS NULL)
      OR (status = 'FAILED' AND result_json IS NULL AND error_json IS NOT NULL AND preview_token IS NULL AND pending_target_id IS NULL)
      OR (status = 'APPLIED' AND result_json IS NOT NULL AND error_json IS NULL AND preview_token IS NOT NULL AND pending_target_id IS NOT NULL)
    )
);
```

Add length/hash/date/JSON-shape checks, FORCE RLS owner policies, one preview per
job, and indexes for owner history plus worker pending scan. Add
`accounts.paper_state_version bigint NOT NULL DEFAULT 0` and a migration-owned,
fixed-search-path trigger function on `cash_ledger` and `positions` that updates
the referenced account version on INSERT/UPDATE/DELETE.

Extend `pending_targets` with:

```sql
source_kind text NOT NULL DEFAULT 'LEGACY',
recommendation_run_id uuid REFERENCES recommendation_runs(id)
```

Add exact source/lineage checks and a trigger rejecting non-migration-owner writes
of recommendation origin fields.

Create these narrow functions with `SECURITY DEFINER`, owner
`migration_owner`, and `SET search_path = pg_catalog, pg_temp`:

```sql
public.snapshot_paper_rebalance_preview(uuid, uuid)
public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb)
public.fail_paper_rebalance_preview(uuid, uuid, jsonb)
public.apply_paper_rebalance_preview(uuid, uuid, text, date)
public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb)
public.preflight_paper_target(uuid, uuid) -- CREATE OR REPLACE
```

`snapshot`/`publish`/`fail` execute only for worker. `apply` executes only for
app. The new eight-argument scheduled bridge executes only for worker; revoke
worker execution on the old seven-argument bridge. The new bridge records
`SCHEDULED_RECOMMENDATION`; `apply` records `MANUAL_RECOMMENDATION` and does not
require `auto_apply_recommendations`. The replaced execution preflight requires
auto-apply only for scheduled origin and acquires the same account advisory lock
used by apply.

- [ ] **Step 4: Add the guarded down migration**

Refuse rollback when any preview is nonterminal or any manual recommendation
target would lose lineage. Restore the exact 0037 `preflight_paper_target`, grant
the old seven-argument scheduled bridge to worker, revoke/drop the new functions,
triggers, policies, indexes, table, source columns, and account state version in
reverse dependency order.

- [ ] **Step 5: Run focused and full migration GREEN checks**

Run:

```powershell
cargo test -p migration-contract recommendation_preview -- --nocapture --test-threads=1
cargo test -p migration-contract -- --nocapture --test-threads=1
```

Expected: focused behavior passes and the full apply/no-op/down/rerun/future-ledger
suite passes.

- [ ] **Step 6: Commit migration boundary**

```powershell
git add migrations/0038_paper_rebalance_previews.* tests/integration/migration-contract/tests/migration_contract.rs
git commit -m "feat(db): add Paper rebalance preview boundary"
```

---

### Task 2: Deterministic preview calculation

**Files:**
- Create: `crates/job-queue/src/paper_preview.rs`
- Modify: `crates/job-queue/src/lib.rs`
- Create: `crates/job-queue/tests/paper_preview.rs`

- [ ] **Step 1: Write pure calculation RED tests**

Define the wished-for API in tests:

```rust
let result = calculate_preview(PreviewCalculationInput {
    cash: Money::parse("10000000", Currency::KRW)?,
    positions,
    close_prices,
    targets,
    lot_sizes: BTreeMap::new(),
    profile: CostProfile::krx_etf_default()?,
    price_date: TradingDate::parse("2026-05-08")?,
    proposed_effective_date: TradingDate::parse("2026-05-12")?,
    lineage,
})?;
assert_eq!(result.orders[0].side, "SELL");
assert!(result.leftover_cash.parse::<Decimal>()? >= Decimal::ZERO);
assert_eq!(result.warning_code, "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED");
```

Cover sells-before-buys, all cash, no trade, minimum trade, fee/slippage
separation, missing held-instrument price, canonical instrument order, and stable
token under identical inputs.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p job-queue --test paper_preview calculation -- --nocapture
```

Expected: compile failure because `paper_preview` and `calculate_preview` do not
exist.

- [ ] **Step 3: Implement the pure types and calculator**

Add focused types:

```rust
pub struct PreviewLineage {
    pub account_id: Uuid,
    pub recommendation_run_id: Uuid,
    pub target_portfolio_id: Uuid,
    pub strategy_config_id: Uuid,
    pub dataset_version_id: Uuid,
    pub dataset_manifest_sha256: String,
    pub account_state_version: i64,
    pub account_state_sha256: String,
    pub target_portfolio_sha256: String,
}
pub struct PreviewCalculationInput {
    pub cash: Money,
    pub positions: BTreeMap<InstrumentId, Quantity>,
    pub close_prices: BTreeMap<InstrumentId, Price>,
    pub targets: Vec<TargetAllocation>,
    pub lot_sizes: BTreeMap<InstrumentId, u64>,
    pub profile: CostProfile,
    pub price_date: TradingDate,
    pub proposed_effective_date: TradingDate,
    pub lineage: PreviewLineage,
}
pub struct PreviewResultV1 {
    pub schema_version: u32,
    pub price_basis: String,
    pub price_date: String,
    pub proposed_effective_date: String,
    pub equity: String,
    pub cash_before: String,
    pub available_cash: String,
    pub leftover_cash: String,
    pub buy_notional: String,
    pub sell_notional: String,
    pub explicit_fees: String,
    pub informational_slippage: String,
    pub decisions: Vec<PreviewDecisionV1>,
    pub orders: Vec<PreviewOrderV1>,
    pub warning_code: String,
    pub lineage: PreviewLineage,
}
pub struct PreviewDecisionV1 {
    pub instrument_id: String,
    pub current_quantity: String,
    pub current_value: String,
    pub current_weight: String,
    pub target_value: String,
    pub target_weight: String,
    pub delta_value: String,
    pub action: String,
    pub skip_reason: Option<String>,
}
pub struct PreviewOrderV1 {
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub raw_price: String,
    pub estimated_execution_price: String,
    pub notional: String,
    pub commission: String,
    pub tax: String,
    pub informational_slippage: String,
}
pub enum PreviewErrorClass { Transient, DataBlocked, Integrity }
pub enum PaperPreviewError {
    Database(sqlx::Error),
    InvalidPayload(String),
    PreviewUnavailable(String),
    AccountChanged,
    MissingPrice { instrument_id: String },
    MalformedCuratedData(String),
    Plan(String),
    LeaseLost,
    Canceled,
    ResultTooLarge { bytes: usize },
}

pub fn calculate_preview(input: PreviewCalculationInput)
    -> Result<(PreviewResultV1, String), PaperPreviewError>;
```

Call `portfolio_model::sizing::plan_rebalance` exactly once. Derive detailed fee
components using the same `CostProfile`; never implement a second affordability
algorithm. Serialize a versioned canonical structure and SHA-256 it for the
preview token. Reject any serialized result above the schema limit before
returning.

- [ ] **Step 4: Add immutable close loading RED tests**

Using a temporary real `CurateStore`, write raw bars for all target and held
instruments, then assert `load_recommendation_closes(root, version, date, ids)`
returns exact raw closes. Add missing, malformed, wrong instrument partition,
wrong date, and future-close cases.

- [ ] **Step 5: Implement the close loader**

Follow `paper_execution::session_opens` and `paper_valuation::session_closes`, but
accept the attested curated numeric version rather than a caller path. Require
exact instrument/date identity and `market_close_ts <= now`; return typed
DataBlocked/Integrity/Transient variants without message parsing.

- [ ] **Step 6: Run calculation and loader GREEN tests**

Run:

```powershell
cargo test -p job-queue --test paper_preview -- --nocapture
cargo test -p portfolio-model sizing -- --nocapture
```

Expected: all focused preview tests and existing sizing tests pass.

- [ ] **Step 7: Commit deterministic calculation**

```powershell
git add crates/job-queue/src/lib.rs crates/job-queue/src/paper_preview.rs crates/job-queue/tests/paper_preview.rs
git commit -m "feat(paper): calculate deterministic rebalance previews"
```

---

### Task 3: Preview worker claim, retry, and atomic publication

**Files:**
- Modify: `crates/job-queue/src/paper_preview.rs`
- Modify: `crates/job-queue/tests/paper_preview.rs`
- Modify: `crates/api-server/src/paper_runner.rs`
- Modify: `crates/api-server/src/bin/paper-runner.rs`
- Modify: `crates/api-server/tests/paper_runner.rs`

- [ ] **Step 1: Write worker lifecycle RED tests**

Seed a preview plus `paper_rebalance_preview` job in real PostgreSQL and call:

```rust
let outcome = run_preview_once(&queue, &worker_pool, &dataset_root, "paper-preview-test").await?;
assert!(matches!(outcome, PreviewRunOutcome::Published { .. }));
```

Assert preview `READY`, one successful attempt, immutable result/token, and no
orders/fills/pending targets. Add two-worker claim idempotency, lease loss,
cancellation, transient account-state change retry, permanent missing-price
failure, and result-write trigger rollback.

- [ ] **Step 2: Run worker tests and verify RED**

Run:

```powershell
cargo test -p job-queue --test paper_preview worker -- --nocapture --test-threads=1
```

Expected: compile failure because `run_preview_once` is absent.

- [ ] **Step 3: Implement the closed payload and snapshot seam**

Use a strict payload:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaperPreviewPayload { pub preview_id: Uuid }
```

Claim only `paper_rebalance_preview`. Call the migration-owned snapshot function
to obtain exact owner/account/run/target/dataset/profile/state/calendar input.
Reconstruct canonical cash/positions and target allocations without accepting
paths or owner IDs from the queue payload.

- [ ] **Step 4: Implement supervised compute and publication**

Run price loading/calculation in `spawn_blocking` while a heartbeat task owns the
lease. Before publication, reacquire the account lock through the publish
function and compare `paper_state_version`. In one transaction update preview
`READY` and call `JobQueue::settle_success_in`. On typed permanent errors update
preview `FAILED` and settle the attempt permanently; on transient errors use the
queue retry path without exposing a partial result. Cancellation or lease loss
must never publish.

- [ ] **Step 5: Integrate one typed claim into Paper cycles**

Extend `RunnerServices` with a configured `JobQueue`. `run_cycle` processes at
most one preview job before due targets and valuation scans and reports
`previews_seen/published/failed`. Extend CLI/environment parsing with bounded
lease, heartbeat, and preview backoff values using the existing queue config
types; retain `--once` determinism. Emit one sanitized cycle log containing
fixed outcome code, compute milliseconds, and counts, but never owner/account,
balance, token, result JSON, URL, or curated path.

- [ ] **Step 6: Run worker and Paper runner GREEN tests**

Run:

```powershell
cargo test -p job-queue --test paper_preview -- --nocapture --test-threads=1
cargo test -p api-server --test paper_runner -- --nocapture --test-threads=1
cargo test -p api-server --bin paper-runner -- --nocapture
```

Expected: lifecycle, concurrency, existing Paper cycles, and CLI parsing pass.

- [ ] **Step 7: Commit worker lifecycle**

```powershell
git add crates/job-queue/src/paper_preview.rs crates/job-queue/tests/paper_preview.rs crates/api-server/src/paper_runner.rs crates/api-server/src/bin/paper-runner.rs crates/api-server/tests/paper_runner.rs
git commit -m "feat(paper): run queued rebalance previews"
```

---

### Task 4: Actor-scoped preview submission and read API

**Files:**
- Create: `crates/api-server/src/repos/rebalance_previews.rs`
- Modify: `crates/api-server/src/repos/mod.rs`
- Modify: `crates/api-server/src/http/state.rs`
- Modify: `crates/api-server/src/http/dto.rs`
- Modify: `crates/api-server/src/http/paper.rs`
- Modify: `crates/api-server/src/http/mod.rs`
- Modify: `crates/api-server/src/observability/metrics.rs`
- Create: `crates/api-server/tests/http_paper_rebalance.rs`
- Modify: `crates/api-server/tests/common/mod.rs`

- [ ] **Step 1: Write create/read HTTP RED tests**

Use the real router and PostgreSQL harness:

```rust
let created = post_json(
    &h,
    &h.owner,
    &format!("/api/v1/paper/accounts/{account_id}/recommendation-previews"),
    json!({"recommendation_run_id": run_id}),
    Some("preview-key-1"),
).await;
assert_eq!(created.status(), StatusCode::ACCEPTED);
```

Assert row+job atomicity, typed job payload, global owner capacity, foreign
account/run hiding, failed/non-successful run rejection, binding mismatch,
blocked dataset/entitlement rejection, same-key durable replay after
`restart_api`, and mismatch-then-correct retry without cache poisoning. GET must
return metadata only while pending and the strict result after worker completion.

- [ ] **Step 2: Run focused HTTP tests and verify RED**

Run:

```powershell
cargo test -p api-server --test http_paper_rebalance create_preview -- --nocapture --test-threads=1
```

Expected: 404 because the route and repository do not exist.

- [ ] **Step 3: Implement actor-scoped submit and replay**

Create:

```rust
pub struct SubmitRebalancePreview {
    pub account_id: Uuid,
    pub recommendation_run_id: Uuid,
    pub idempotency_key: String,
    pub max_jobs_per_owner: u32,
}

pub enum SubmitRebalancePreviewError {
    Tenancy(TenancyError), CapacityExceeded, IdempotencyMismatch,
    BindingRequired, RunNotReady,
}
```

Inside one actor transaction take `lock_owner_job_capacity`, resolve the existing
job by namespaced public key before capacity counting, compare only public
account/run identity, lock exact account/binding/run/portfolio/dataset/
entitlement/calendar inputs, insert the job and preview atomically, and return
the existing preview on exact replay.

- [ ] **Step 4: Implement strict read DTOs and handlers**

Add request and response DTOs with `deny_unknown_fields` on requests and decimal
strings on every fixed-point value. Register:

```rust
POST /paper/accounts/{account_id}/recommendation-previews
GET  /paper/accounts/{account_id}/recommendation-previews/{preview_id}
```

Require Owner role, CSRF and idempotency on POST, existing licensing gates, and
tenant-local 404 behavior. Map typed repository errors to stable public codes;
never serialize `result_json` without validating it into `PreviewResultV1`.
Add fixed-label `paper_rebalance_preview_requests_total` and
`paper_rebalance_preview_applies_total` counters, seeded only with the closed
outcome sets declared in the metrics module.

- [ ] **Step 5: Run create/read GREEN and adjacent API tests**

Run:

```powershell
cargo test -p api-server --test http_paper_rebalance create_preview -- --nocapture --test-threads=1
cargo test -p api-server --test http_paper -- --nocapture --test-threads=1
cargo test -p api-server --test http_recommendations -- --nocapture --test-threads=1
```

Expected: new create/read scenarios and existing Paper/recommendation contracts pass.

- [ ] **Step 6: Commit request/read API**

```powershell
git add crates/api-server/src/repos/rebalance_previews.rs crates/api-server/src/repos/mod.rs crates/api-server/src/http/state.rs crates/api-server/src/http/dto.rs crates/api-server/src/http/paper.rs crates/api-server/src/http/mod.rs crates/api-server/src/observability/metrics.rs crates/api-server/tests/http_paper_rebalance.rs crates/api-server/tests/common/mod.rs
git commit -m "feat(api): request and read Paper rebalance previews"
```

---

### Task 5: Explicit apply, manual lineage, and concurrency

**Files:**
- Modify: `crates/api-server/src/repos/rebalance_previews.rs`
- Modify: `crates/api-server/src/http/dto.rs`
- Modify: `crates/api-server/src/http/paper.rs`
- Modify: `crates/api-server/tests/http_paper_rebalance.rs`
- Modify: `crates/job-queue/src/recommendation/publish.rs`
- Modify: `crates/job-queue/tests/recommendation_publish.rs`
- Modify: `crates/api-server/tests/paper_runner.rs`

- [ ] **Step 1: Write apply RED tests**

After producing a real `READY` preview, call:

```rust
let applied = post_json(
    &h,
    &h.owner,
    &format!("/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply"),
    json!({"preview_token": token}),
    Some("apply-key-1"),
).await;
assert_eq!(applied.status(), StatusCode::OK);
```

Assert one `MANUAL_RECOMMENDATION` pending target, exact run/dataset/weights,
zero orders/fills, and replay to the same target. Add wrong token/not-ready,
cash mutation, position mutation, account status, binding replacement, target
mutation, dataset block, entitlement revoke, proposed-session arrival,
same-session conflict, foreign tenant, and concurrent two-request cases.

- [ ] **Step 2: Run apply tests and verify RED**

Run:

```powershell
cargo test -p api-server --test http_paper_rebalance apply_preview -- --nocapture --test-threads=1
```

Expected: 404 because the apply route is absent.

- [ ] **Step 3: Implement repository apply and HTTP route**

Add:

```rust
pub async fn apply(
    &self,
    actor: &Actor,
    account_id: Uuid,
    preview_id: Uuid,
    preview_token: &str,
    seoul_today: NaiveDate,
) -> Result<AppliedPreviewRow, ApplyRebalancePreviewError>;
```

Call only the app-authorized migration function in an actor transaction. Map its
typed result to `REBALANCE_PREVIEW_NOT_READY`, `REBALANCE_PREVIEW_STALE`,
`REBALANCE_PREVIEW_CONFLICT`, or the existing target. Register the POST route and
require Owner/CSRF/idempotency. Do not accept target weights, effective dates,
account state, owner, or lineage from the request.

- [ ] **Step 4: Wire exact scheduled source lineage**

Change publication to call the eight-argument scheduled bridge with the current
recommendation run UUID. Assert scheduled rows are
`SCHEDULED_RECOMMENDATION`, manual rows never require auto-apply, scheduled
execution still does, and existing scheduled/manual collisions compare complete
identity.

- [ ] **Step 5: Run apply and scheduled GREEN tests**

Run:

```powershell
cargo test -p api-server --test http_paper_rebalance -- --nocapture --test-threads=1
cargo test -p job-queue --test recommendation_publish -- --nocapture --test-threads=1
cargo test -p api-server --test paper_runner -- --nocapture --test-threads=1
cargo test -p api-server --test paper_execution_seam -- --nocapture --test-threads=1
```

Expected: all apply, source-lineage, execution-preflight, and no-immediate-trade
assertions pass.

- [ ] **Step 6: Commit explicit apply**

```powershell
git add crates/api-server/src/repos/rebalance_previews.rs crates/api-server/src/http/dto.rs crates/api-server/src/http/paper.rs crates/api-server/tests/http_paper_rebalance.rs crates/job-queue/src/recommendation/publish.rs crates/job-queue/tests/recommendation_publish.rs crates/api-server/tests/paper_runner.rs
git commit -m "feat(paper): apply recommendation previews explicitly"
```

---

### Task 6: OpenAPI, full regression, and final high-risk review

**Files:**
- Modify: `crates/api-server/src/contract.rs`
- Modify: `apps/api-server/openapi.json`
- Regenerate: `apps/api-server/generated/openapi.ts`
- Modify: `crates/api-server/tests/openapi_contract.rs`
- Modify: `docs/STATUS.md`

- [ ] **Step 1: Add contract RED assertions**

Declare all three paths, Owner/CSRF/idempotency requirements, request/response
schemas, 202/200/400/403/404/409/429/500 responses, and every new stable error
code in the Rust contract table. Run the OpenAPI contract before editing the
authored document.

- [ ] **Step 2: Verify OpenAPI RED**

Run:

```powershell
cargo test -p api-server --test openapi_contract -- --nocapture
npm run openapi:check --workspace @lagrange/api-contract
```

Expected: FAIL with missing paths/error enum/generated-type drift.

- [ ] **Step 3: Update authored OpenAPI and regenerate types**

Add strict component schemas matching the Rust DTOs and run the repository's
existing generation command:

```powershell
npm run openapi:generate --workspace @lagrange/api-contract
npm run openapi:check --workspace @lagrange/api-contract
```

Expected: operation count, lint, generated TypeScript, and typecheck pass.

- [ ] **Step 4: Update status documentation**

Add one concise `docs/STATUS.md` entry describing backend-only indicative
preview, explicit Paper queueing, next-open replanning, and the absence of UI or
live-order behavior.

- [ ] **Step 5: Run fresh focused PostgreSQL verification**

Run serially against PostgreSQL 18:

```powershell
cargo test -p migration-contract -- --nocapture --test-threads=1
cargo test -p job-queue --test paper_preview -- --nocapture --test-threads=1
cargo test -p job-queue --test recommendation_publish -- --nocapture --test-threads=1
cargo test -p api-server --test http_paper_rebalance -- --nocapture --test-threads=1
cargo test -p api-server --test paper_runner -- --nocapture --test-threads=1
cargo test -p api-server --test openapi_contract -- --nocapture
```

Expected: all focused database, worker, API, and contract tests pass without
skips when `DATABASE_URL` is set.

- [ ] **Step 6: Run fresh broad regression and static gates**

Run:

```powershell
cargo test -p portfolio-model -p job-queue -p api-server
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm run openapi:check --workspace @lagrange/api-contract
git diff --check
```

Expected: zero failures, warnings, format drift, OpenAPI drift, or whitespace
errors.

- [ ] **Step 7: Perform the final security/concurrency audit**

Inspect the complete branch diff and prove each invariant from code plus test:

```text
owner comes from actor only
app cannot publish preview results
worker cannot apply consent
manual apply never needs auto_apply
scheduled execution still needs auto_apply
account state mutation linearizes through paper_state_version/account lock
lease/cancel cannot publish
apply creates no order/fill
manual/scheduled collision never overwrites
down migration restores 0037 and refuses unsafe lineage loss
```

Any finding gets a new failing regression before a fix.

- [ ] **Step 8: Commit contract and documentation**

```powershell
git add crates/api-server/src/contract.rs crates/api-server/tests/openapi_contract.rs apps/api-server/openapi.json apps/api-server/generated/openapi.ts docs/STATUS.md
git commit -m "docs(api): publish Paper rebalance preview contract"
```

- [ ] **Step 9: Record final evidence**

Run:

```powershell
git status --short --branch
git log --oneline -8
git diff --check HEAD~6 HEAD
```

Expected: clean worktree and only the scoped design/plan/implementation commits.
