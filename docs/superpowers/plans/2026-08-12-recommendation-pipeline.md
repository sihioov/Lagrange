# Fixed-ETF Recommendation Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn queued recommendation requests into deterministic, explainable target portfolios for the fixed 11 Korean ETFs, expose them in the product, and optionally schedule them into opted-in Paper accounts.

**Architecture:** PostgreSQL keeps the durable job/run state. A Rust recommendation runner claims only recommendation jobs, attests a pinned immutable dataset and universe, computes raw factors with `factor-engine`, and invokes an allow-listed Python target generator in a bounded child process. Result publication, Paper planning, and queue settlement are guarded and atomic; production remains blocked without licensed complete KRX data.

**Tech Stack:** Rust 1.97.1, Tokio, SQLx/PostgreSQL, Axum, Polars/Parquet through `factor-engine`, Python 3.12/uv, Next.js/React/TypeScript, Vitest, Playwright, Docker Compose.

---

## File map

- `crates/job-queue/src/queue.rs`: typed claims and transaction-aware settlement.
- `crates/job-queue/src/recommendation/{mod,input,compute,child,validate,publish,schedule}.rs`: focused recommendation pipeline units.
- `crates/job-queue/src/bin/recommendation-runner.rs`: daemon/one-shot entry point.
- `nt/strategies/recommendation_cli.py`: closed-set target-generator child contract.
- `migrations/0026_recommendation_pipeline.{up,down}.sql`: run lineage, uniqueness, queue index, Paper opt-in, and least-privilege grants.
- `crates/api-server/src/http/recommendations.rs`: atomic asynchronous submission and run reads.
- `crates/api-server/src/repos/recommendations.rs`: actor reads plus transaction-safe creation.
- `crates/api-server/src/paper_session.rs`: execution-time entitlement/readiness recheck.
- `apps/web/app/(authenticated)/recommendations/page.tsx`: stable latest-success and first-run page.
- `apps/web/components/recommendations/*`: config selection, polling, cash/provenance, and history.
- `deploy/compose/compose.yml`: real recommendation-runner service and readiness.

### Task 1: Make queue claims job-type safe

**Files:**
- Modify: `crates/job-queue/src/queue.rs`
- Modify: `crates/job-queue/src/runner.rs`
- Modify: `crates/job-queue/src/bin/backtest-runner.rs`
- Test: `crates/job-queue/tests/queue_contract.rs`

- [ ] **Step 1: Write the failing mixed-queue test**

Add a test that submits one `backtest` and one `recommendation` job, then asserts each typed claimant receives only its own type:

```rust
let backtest = queue.submit(job(owner, "backtest", "bt-1")).await?;
let recommendation = queue.submit(job(owner, "recommendation", "rec-1")).await?;
let rec_claim = queue.claim_next_for("rec-worker", "recommendation").await?.unwrap();
let bt_claim = queue.claim_next_for("bt-worker", "backtest").await?.unwrap();
assert_eq!(rec_claim.job.id, recommendation.id);
assert_eq!(bt_claim.job.id, backtest.id);
```

- [ ] **Step 2: Run the test and confirm the missing API**

Run: `cargo test -p job-queue --test queue_contract typed_claims_do_not_steal_other_job_types -- --nocapture`  
Expected: compile failure because `claim_next_for` does not exist.

- [ ] **Step 3: Implement the typed claim**

Add:

```rust
pub async fn claim_next_for(
    &self,
    worker_id: &str,
    job_type: &str,
) -> Result<Option<ClaimedJob>, QueueError>
```

Reuse the existing claim transaction, but add `job_type = $1` before the queued/available predicates. Validate `job_type` with the same `[a-z0-9_-]{1,64}` rule as submission.

- [ ] **Step 4: Switch the backtest runner to typed claims**

Replace its unfiltered claim with:

```rust
let Some(claim) = queue.claim_next_for(worker_id, "backtest").await? else {
    return Ok(Outcome::Idle);
};
```

Keep the existing defensive wrong-type check after claim.

- [ ] **Step 5: Run queue and runner tests**

Run: `cargo test -p job-queue --test queue_contract && cargo test -p job-queue --test backtest_runner`  
Expected: all tests pass, including the mixed queue case.

- [ ] **Step 6: Commit**

```bash
git add crates/job-queue/src/queue.rs crates/job-queue/src/runner.rs crates/job-queue/src/bin/backtest-runner.rs crates/job-queue/tests/queue_contract.rs
git commit -m "fix(queue): claim jobs by worker type"
```

### Task 2: Add recommendation lineage, uniqueness, automation opt-in, and grants

**Files:**
- Create: `migrations/0026_recommendation_pipeline.up.sql`
- Create: `migrations/0026_recommendation_pipeline.down.sql`
- Modify: `tests/integration/migration-contract/tests/migration_contract.rs`
- Modify: `crates/api-server/tests/tenancy_rls.rs`

- [ ] **Step 1: Write migration contract expectations**

Assert these columns and constraints:

```text
recommendation_runs.job_id uuid REFERENCES jobs(id)
recommendation_runs.trigger_kind MANUAL|SCHEDULED
recommendation_runs.dataset_version_id uuid REFERENCES dataset_versions(id)
recommendation_runs.dataset_manifest_sha256 64 lowercase hex
recommendation_items UNIQUE(recommendation_run_id, instrument_id)
target_portfolios UNIQUE(recommendation_run_id)
account_strategy_bindings.auto_apply_recommendations boolean default false
```

Also assert worker can update runs and insert items/target portfolios but cannot insert/delete runs or modify configs.
Assert the worker can execute only a narrowly scoped
`schedule_recommendation_run(...)` function and cannot insert arbitrary jobs or
runs directly.

- [ ] **Step 2: Run the migration contract and verify RED**

Run: `cargo test -p migration-contract recommendation_pipeline -- --nocapture`  
Expected: failures naming missing columns, constraints, index, and grants.

- [ ] **Step 3: Write migration 0026**

The up migration must include:

```sql
ALTER TABLE recommendation_runs
  ADD COLUMN job_id uuid REFERENCES jobs(id),
  ADD COLUMN trigger_kind text NOT NULL DEFAULT 'MANUAL',
  ADD COLUMN dataset_version_id uuid REFERENCES dataset_versions(id),
  ADD COLUMN dataset_manifest_sha256 text;
ALTER TABLE recommendation_runs ADD CONSTRAINT recommendation_runs_trigger_check
  CHECK (trigger_kind IN ('MANUAL','SCHEDULED'));
ALTER TABLE recommendation_items ADD CONSTRAINT recommendation_items_run_instrument_key
  UNIQUE (recommendation_run_id, instrument_id);
CREATE UNIQUE INDEX target_portfolios_one_per_run
  ON target_portfolios(recommendation_run_id) WHERE recommendation_run_id IS NOT NULL;
ALTER TABLE account_strategy_bindings
  ADD COLUMN auto_apply_recommendations boolean NOT NULL DEFAULT false;
CREATE INDEX jobs_typed_claim_idx
  ON jobs(job_type, status, available_at, priority DESC, created_at)
  WHERE status = 'QUEUED';
GRANT SELECT, UPDATE ON recommendation_runs TO worker;
GRANT SELECT, INSERT ON recommendation_items, target_portfolios TO worker;
```

Add a `SECURITY DEFINER` `schedule_recommendation_run(owner, config, as_of,
dataset_version, manifest_hash, idempotency_key)` function owned by the
migration owner. Set a fixed safe `search_path`, verify an active opted-in
PAPER binding and matching dataset row, insert only `trigger_kind='SCHEDULED'`
and `job_type='recommendation'`, and return the run/job IDs. Revoke function
execution from `PUBLIC` and grant it only to `worker`. Add the hex check only
when the manifest hash is non-null. The down migration drops the function,
reverses only 0026 additions, and revokes the new grants.

- [ ] **Step 4: Run migration and RLS tests**

Run: `cargo test -p migration-contract && cargo test -p api-server --test tenancy_rls`  
Expected: all tests pass; worker overreach assertions remain denied.

- [ ] **Step 5: Commit**

```bash
git add migrations/0026_recommendation_pipeline.* tests/integration/migration-contract/tests/migration_contract.rs crates/api-server/tests/tenancy_rls.rs
git commit -m "feat(db): add recommendation pipeline lineage"
```

### Task 3: Define and attest recommendation inputs

**Files:**
- Create: `crates/job-queue/src/recommendation/mod.rs`
- Create: `crates/job-queue/src/recommendation/input.rs`
- Modify: `crates/job-queue/src/lib.rs`
- Modify: `crates/job-queue/src/resolver.rs`
- Test: `crates/job-queue/tests/recommendation_input.rs`

- [ ] **Step 1: Write parsing and ownership tests**

Cover a valid payload and rejection of missing/foreign/mismatched fields:

```rust
let payload = RecommendationPayload::try_from(json!({
  "run_id": run_id,
  "strategy_config_id": config_id,
  "as_of": "2020-12-30",
  "dataset": {
    "id": dataset_uuid,
    "dataset_id": "krx_eod_bars",
    "version": "phase0-v2",
    "curated_version": 2,
    "manifest_sha256": "00...00"
  }
}))?;
assert_eq!(payload.as_of.to_string(), "2020-12-30");
```

- [ ] **Step 2: Run RED**

Run: `cargo test -p job-queue --test recommendation_input -- --nocapture`  
Expected: compile failure because the recommendation input module is absent.

- [ ] **Step 3: Implement strict typed payloads**

Use `#[serde(deny_unknown_fields)]` for `RecommendationPayload` and `DatasetPin`. Parse UUIDs, dates, numeric versions, and 64-character lowercase hashes at the boundary. Do not accept a storage path from the request; resolve it from runner deployment configuration and verify the database row's `storage_path` and manifest hash.

- [ ] **Step 4: Extract owner-bound config lookup**

Expose a common record:

```rust
pub struct ResolvedConfig {
    pub strategy_id: String,
    pub strategy_version: String,
    pub config: serde_json::Value,
}
```

The SQL must bind both `id` and `owner_user_id` and require `is_active`. Keep backtest module/class resolution as a separate allow-list step.

- [ ] **Step 5: Test database attestation failures**

Add cases for missing dataset row, `BLOCKED` status, manifest mismatch, run/config/as-of mismatch, inactive config, and foreign owner. Map data failures to `DataBlocked` and integrity/config failures to permanent failure.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test -p job-queue --test recommendation_input && cargo test -p job-queue --test backtest_runner`  
Expected: all pass.

```bash
git add crates/job-queue/src/recommendation crates/job-queue/src/lib.rs crates/job-queue/src/resolver.rs crates/job-queue/tests/recommendation_input.rs
git commit -m "feat(recommendations): attest pinned run inputs"
```

### Task 4: Compute exact raw factors for one close

**Files:**
- Create: `crates/job-queue/src/recommendation/compute.rs`
- Modify: `crates/job-queue/src/factor_series.rs`
- Test: `crates/job-queue/tests/recommendation_compute.rs`

- [ ] **Step 1: Write strategy parameter matrix tests**

Assert exact requirements:

```text
buy_and_hold -> [] / lookback 0
trend_following fast=100 slow=200 -> [trend_100, trend_200] / 200
relative_momentum lookback=6 -> [return_6m] / 126
relative_momentum lookback=12 -> [momentum_12_1] / 252
dual_momentum lookback=6 -> [return_6m] / 126
inverse_volatility vol_window=120 -> [vol_120] / 120
```

Reject a version other than the shipped immutable version and parameters outside the existing JSON schema.

- [ ] **Step 2: Run RED**

Run: `cargo test -p job-queue --test recommendation_compute requirements -- --nocapture`  
Expected: missing `requirements_for`.

- [ ] **Step 3: Implement parameter-derived factors**

Add `requirements_for(&ResolvedConfig) -> Result<StrategyRequirements, RecommendationError>`. Generalize the factor resolver to parse bounded `trend_<n>` and `vol_<n>` IDs rather than restating only 20/50/60/100/120/200.

- [ ] **Step 4: Write full-11 snapshot tests**

Generate the repository's existing Phase-0 synthetic data into a temp root. Assert the manifest's exact 11 IDs, requested close equality, finite raw values, omitted NULLs, deterministic factor hash, and rejection when one universe member is absent or a future row exists after `as_of`.

- [ ] **Step 5: Implement `compute_close`**

```rust
pub fn compute_close(
    pin: &AttestedDataset,
    universe: &AttestedUniverse,
    as_of: TradingDate,
    requirements: &StrategyRequirements,
) -> Result<ComputedClose, RecommendationError>
```

Use `FactorSnapshotBuilder` with only required factors and return raw values keyed `instrument -> factor`. Run it through `tokio::task::spawn_blocking` from the async orchestrator.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test -p job-queue --test recommendation_compute -- --nocapture`  
Expected: all cases pass and synthetic output is labeled QA-only.

```bash
git add crates/job-queue/src/recommendation/compute.rs crates/job-queue/src/factor_series.rs crates/job-queue/tests/recommendation_compute.rs
git commit -m "feat(recommendations): compute fixed universe factors"
```

### Task 5: Add the isolated target-generator CLI

**Files:**
- Create: `nt/strategies/recommendation_cli.py`
- Create: `nt/strategies/tests/test_recommendation_cli.py`
- Create: `crates/job-queue/src/recommendation/child.rs`
- Test: `crates/job-queue/tests/recommendation_child.rs`

- [ ] **Step 1: Write Python contract tests**

Use temporary request/result files. Cover all five strategy IDs, deterministic identical output, unknown strategy, unknown request field, mismatched version, malformed factors, and a supplied value such as `"module": "os:system"` being rejected before import.

- [ ] **Step 2: Run RED**

Run: `uv run --project nt pytest nt/strategies/tests/test_recommendation_cli.py -q`  
Expected: import/file-not-found failure.

- [ ] **Step 3: Implement the closed-set CLI**

The allow-list must be literal:

```python
GENERATORS = {
    "buy_and_hold": "strategies.buy_and_hold.target",
    "trend_following": "strategies.trend_following.target",
    "relative_momentum": "strategies.relative_momentum.target",
    "dual_momentum": "strategies.dual_momentum.target",
    "inverse_volatility": "strategies.inverse_volatility.target",
}
```

Read one size-bounded JSON file, validate exact keys/types, call `generate_target`, attach Rust-supplied provenance, and atomically replace the result file. On failure, write a bounded status file with a stable code; never echo the full request.

- [ ] **Step 4: Implement Rust child invocation**

Add `TargetChildPaths` and `run_target_child`. Use an explicit `uv_bin`, `--project nt`, a sanitized environment, piped/null stdout, bounded stderr diagnostics, a timeout, and temp files scoped to the job ID. Parse only the result file.

- [ ] **Step 5: Run Python and Rust child tests**

Run: `uv run --project nt pytest nt/strategies/tests/test_recommendation_cli.py -q && cargo test -p job-queue --test recommendation_child -- --nocapture`  
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add nt/strategies/recommendation_cli.py nt/strategies/tests/test_recommendation_cli.py crates/job-queue/src/recommendation/child.rs crates/job-queue/tests/recommendation_child.rs
git commit -m "feat(recommendations): add isolated target generator"
```

### Task 6: Validate and atomically publish recommendation results

**Files:**
- Create: `crates/job-queue/src/recommendation/validate.rs`
- Create: `crates/job-queue/src/recommendation/publish.rs`
- Modify: `crates/job-queue/src/queue.rs`
- Modify: `crates/api-server/src/repos/parity.rs`
- Test: `crates/job-queue/tests/recommendation_publish.rs`

- [ ] **Step 1: Write malicious-output validation tests**

Reject wrong strategy/as-of, foreign or duplicate instruments, non-finite numbers, negative or over-one weights, invalid cash, wrong total, missing provenance, and a changed portfolio hash. Accept an explicit all-cash portfolio.

- [ ] **Step 2: Run RED**

Run: `cargo test -p job-queue --test recommendation_publish validates_child_output -- --nocapture`  
Expected: missing validator/publication APIs.

- [ ] **Step 3: Implement the validator**

Return a typed normalized `ValidatedPortfolio` using decimal strings at six-place scale for database writes. Recompute the canonical portfolio hash in Rust and compare it with the child result.

- [ ] **Step 4: Add transaction-aware queue settlement**

Expose guarded methods that accept `&mut Transaction<'_, Postgres>` and verify job status, lock owner, attempt number, and lease before updating the attempt/job. Existing public settlement methods should call the same internal implementation.

- [ ] **Step 5: Implement one publication transaction**

Within one worker transaction:

```text
lock lease-valid job and attempt
lock matching PENDING recommendation run
verify job/run/config owner and payload fields
insert selected and excluded items
insert exactly one target_portfolio
update summary_json and run status
settle attempt and job
commit
```

Add tests that inject failure before each write boundary and assert no partial rows. Retry the same deterministic result and assert one portfolio and one item per instrument.

- [ ] **Step 6: Normalize parity reads**

Filter non-excluded zero-weight rows or normalize them consistently so eligible-but-unselected instruments do not produce false Paper/backtest divergence.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p job-queue --test recommendation_publish && cargo test -p api-server --test paper_notifications`  
Expected: all pass.

```bash
git add crates/job-queue/src/recommendation/validate.rs crates/job-queue/src/recommendation/publish.rs crates/job-queue/src/queue.rs crates/api-server/src/repos/parity.rs crates/job-queue/tests/recommendation_publish.rs
git commit -m "feat(recommendations): publish results atomically"
```

### Task 7: Build the recommendation runner and error-state lifecycle

**Files:**
- Modify: `crates/job-queue/src/recommendation/mod.rs`
- Create: `crates/job-queue/src/bin/recommendation-runner.rs`
- Modify: `crates/job-queue/Cargo.toml`
- Test: `crates/job-queue/tests/recommendation_runner.rs`

- [ ] **Step 1: Write end-to-end runner tests**

Cover: happy path, revoked entitlement after enqueue, missing/blocked dataset, universe mismatch, insufficient history, invalid config, child deterministic failure, transient launch/DB failure with retry, exhaustion to `FAILED`, cancellation, stale lease, and mixed queue isolation.

- [ ] **Step 2: Run RED**

Run: `cargo test -p job-queue --test recommendation_runner -- --nocapture`  
Expected: missing runner entry point.

- [ ] **Step 3: Implement `run_once`**

Return:

```rust
pub enum RecommendationOutcome {
    Idle,
    Succeeded { job_id: Uuid, run_id: Uuid },
    Blocked { job_id: Uuid, code: String },
    Failed { job_id: Uuid, code: String },
    Retrying { job_id: Uuid, code: String },
}
```

Keep the run `PENDING` during retryable attempts. On final exhaustion, atomically mark it `FAILED`. Map entitlement/data/universe/history to `BLOCKED`, deterministic input/strategy/output failures to `FAILED`, and infrastructure errors to retryable.

- [ ] **Step 4: Implement the daemon**

Support `--once`, continuous polling, heartbeat while CPU/child work runs, periodic orphan sweep, Ctrl-C graceful shutdown between jobs, explicit dataset/repo/uv paths, and sanitized structured events. Add parser tests for invalid durations and missing production pins.

- [ ] **Step 5: Run focused and binary tests**

Run: `cargo test -p job-queue --test recommendation_runner && cargo test -p job-queue --bin recommendation-runner && cargo check -p job-queue --bins`  
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/job-queue/src/recommendation/mod.rs crates/job-queue/src/bin/recommendation-runner.rs crates/job-queue/Cargo.toml crates/job-queue/tests/recommendation_runner.rs
git commit -m "feat(recommendations): run queued recommendations"
```

### Task 8: Make recommendation submission atomic and reads product-correct

**Files:**
- Modify: `crates/api-server/src/http/recommendations.rs`
- Modify: `crates/api-server/src/repos/recommendations.rs`
- Modify: `crates/api-server/src/http/dto.rs`
- Modify: `crates/api-server/src/config.rs`
- Test: `crates/api-server/tests/http_recommendations.rs`
- Test: `crates/api-server/tests/phase1_gate.rs`

- [ ] **Step 1: Replace SQL-simulated success with a real runner test seam**

The happy-path test must call the runner once after POST and then GET the persisted result. Add explicit tests for queue insertion failure leaving no run, job ID persistence, dataset-pin payload, per-owner capacity, and idempotent duplicate POST.

- [ ] **Step 2: Run RED**

Run: `cargo test -p api-server --test http_recommendations -- --nocapture`  
Expected: the new assertions fail because creation and queueing are separate and the worker is absent from the old test.

- [ ] **Step 3: Implement actor-transaction creation**

Add a repository/service method that inserts the run and job in one actor transaction, namespaces the queue idempotency key with `recommendation`, persists `job_id`, and carries the configured attested dataset pin. Return the existing DTO shape plus `trigger_kind` and provenance fields.

- [ ] **Step 4: Fix read ordering and latest semantics**

Return history newest-first with cursor pagination. `latest` returns the most recent successful report plus the newest in-flight/terminal run metadata so a pending/failed run does not hide the last usable result.

- [ ] **Step 5: Run API suites and commit**

Run: `cargo test -p api-server --test http_recommendations && cargo test -p api-server --test phase1_gate`  
Expected: all pass and entitlement revocation still hides payloads.

```bash
git add crates/api-server/src/http/recommendations.rs crates/api-server/src/repos/recommendations.rs crates/api-server/src/http/dto.rs crates/api-server/src/config.rs crates/api-server/tests/http_recommendations.rs crates/api-server/tests/phase1_gate.rs
git commit -m "feat(api): submit recommendation jobs atomically"
```

### Task 9: Add close scheduling, Paper opt-in, and execution-time rechecks

**Files:**
- Create: `crates/job-queue/src/recommendation/schedule.rs`
- Modify: `crates/api-server/src/repos/accounts.rs`
- Modify: `crates/api-server/src/http/paper.rs`
- Modify: `crates/api-server/src/http/dto.rs`
- Modify: `crates/api-server/src/paper_session.rs`
- Modify: `crates/api-server/src/repos/pending_targets.rs`
- Test: `crates/api-server/tests/paper_scheduler.rs`
- Test: `crates/api-server/tests/paper_runner.rs`
- Test: `crates/job-queue/tests/recommendation_schedule.rs`

- [ ] **Step 1: Write opt-in and schedule tests**

Assert default binding is false, explicit true is persisted, only opted-in active PAPER bindings schedule, two concurrent/startup catch-up cycles produce one run per config/close/pin, and `T+1` comes from `trading_calendars` across a holiday.

- [ ] **Step 2: Run RED**

Run: `cargo test -p job-queue --test recommendation_schedule && cargo test -p api-server --test paper_scheduler`  
Expected: missing scheduler and opt-in fields.

- [ ] **Step 3: Implement schedule planning**

At 16:30 KST/catch-up, require a confirmed trading close, the configured attested 11-member dataset pin, active entitlement, and enabled binding. Use a deterministic key of owner/config/as-of/dataset-version and call the narrowly scoped `schedule_recommendation_run` database function, which takes an advisory transaction lock and creates at most one scheduled run/job without granting the worker arbitrary INSERT privileges.

- [ ] **Step 4: Queue Paper targets only after scheduled success**

During atomic publication, for opted-in active PAPER bindings with the exact config/version, write the positive selected weights as decimal strings into `pending_targets`, copy dataset lineage, and use the next published KRX session. Manual runs never write pending targets.

- [ ] **Step 5: Recheck at Paper execution**

Before `execute_session`, reload recommendation entitlement and dataset status. If blocked/revoked, settle the pending target `SKIPPED` with a structured non-executed reason and create no orders/fills.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test -p job-queue --test recommendation_schedule && cargo test -p api-server --test paper_scheduler && cargo test -p api-server --test paper_runner`  
Expected: all pass, including revoked-after-queue.

```bash
git add crates/job-queue/src/recommendation/schedule.rs crates/api-server/src/repos/accounts.rs crates/api-server/src/http/paper.rs crates/api-server/src/http/dto.rs crates/api-server/src/paper_session.rs crates/api-server/src/repos/pending_targets.rs crates/api-server/tests/paper_scheduler.rs crates/api-server/tests/paper_runner.rs crates/job-queue/tests/recommendation_schedule.rs
git commit -m "feat(paper): schedule opted-in recommendations"
```

### Task 10: Complete the recommendation web workflow

**Files:**
- Modify: `apps/web/app/(authenticated)/recommendations/page.tsx`
- Modify: `apps/web/components/recommendations/recommendation-run-form.tsx`
- Modify: `apps/web/components/recommendations/recommendation-report.tsx`
- Modify: `apps/web/components/recommendations/recommendation-history.tsx`
- Create: `apps/web/components/recommendations/recommendation-run-status.tsx`
- Modify: `apps/web/lib/api/product-client.ts`
- Modify: `apps/web/lib/products/contracts.ts`
- Modify: `apps/web/tests/recommendation-surface.test.tsx`
- Modify: `apps/web/tests/e2e/recommendations.spec.ts`
- Modify: `apps/web/tests/e2e/support/recommendation-fixture.mjs`

- [ ] **Step 1: Read the installed Next.js guide required by `apps/web/AGENTS.md`**

Run: `Get-Content -Raw apps/web/node_modules/next/dist/docs/01-app/03-building-your-application/02-data-fetching/index.md`  
Expected: the installed-version data-fetching guidance is available; if the exact file moved, locate it with `rg --files apps/web/node_modules/next/dist/docs | rg 'data-fetch'` and read the matching document before editing.

- [ ] **Step 2: Write failing component tests**

Cover no config, config/no run with enabled form, multiple config selection, pending status polling, latest success retained while another run is pending/failed, all-cash, cash/provenance/synthetic label, blocked no-leak, newest-first history, and clickable run detail.

- [ ] **Step 3: Run RED**

Run: `pnpm --dir apps/web test -- recommendation-surface.test.tsx`  
Expected: failures for first-run form, polling, and cash/provenance.

- [ ] **Step 4: Implement typed client and UI states**

Add `getRecommendationRun(id)`, fetch configs/latest/history together, make `strategy_config_id` a select field, poll only the submitted run with bounded backoff, retain the last successful report, show explicit cash allocation/all-cash copy, and display origin plus dataset/universe/factor/portfolio provenance.

- [ ] **Step 5: Correct stale fixtures**

Use `relative_momentum@1.0.0`, only canonical `kr-etf-core-v1` instruments, correct parameters, and a POST sequence of `PENDING` followed by `SUCCEEDED`. Remove the nonexistent `114800.KRX` fixture row.

- [ ] **Step 6: Run unit and E2E tests**

Run: `pnpm --dir apps/web test -- recommendation-surface.test.tsx && pnpm --dir apps/web exec playwright test tests/e2e/recommendations.spec.ts`  
Expected: all recommendation tests pass.

- [ ] **Step 7: Commit**

```bash
git add apps/web/app/\(authenticated\)/recommendations/page.tsx apps/web/components/recommendations apps/web/lib/api/product-client.ts apps/web/lib/products/contracts.ts apps/web/tests/recommendation-surface.test.tsx apps/web/tests/e2e/recommendations.spec.ts apps/web/tests/e2e/support/recommendation-fixture.mjs
git commit -m "feat(web): complete recommendation workflow"
```

### Task 11: Deploy, document, and verify the full feature

**Files:**
- Modify: `deploy/compose/compose.yml`
- Modify: `deploy/compose/.env.example`
- Create: `deploy/systemd/lagrange-recommendation-runner.service`
- Create: `scripts/qa/recommendation-runner-smoke.ps1`
- Create: `scripts/qa/recommendation-runner-smoke.sh`
- Modify: `deploy/README.md`
- Modify: `docs/STATUS.md`

- [ ] **Step 1: Write a failing deployment smoke**

The smoke must generate the labeled synthetic 11-ETF QA dataset, migrate PostgreSQL, seed the exact universe/dataset/entitlement records, submit a real run, execute `recommendation-runner --once`, and assert one succeeded run, eleven normalized item/exclusion rows in total, one target portfolio, and no secret values in output.

- [ ] **Step 2: Run RED**

Run: `pwsh -File scripts/qa/recommendation-runner-smoke.ps1`  
Expected: failure because the service/binary wiring is absent.

- [ ] **Step 3: Add the real service wiring**

Mount curated/universe paths read-only, use worker DB credentials via `_FILE`, pass explicit dataset pin/repo/uv paths, schedule/poll/lease settings, `APP_ENV`, and no broker credentials. Replace only the recommendation placeholder/service scope; do not claim unrelated web/API stubs are production-ready.

- [ ] **Step 4: Add honest health and docs**

Health reports process/DB reachability, last schedule cycle, queue age, and blocked-data state. Document the 16:30 KST default, startup catch-up, opt-in Paper behavior, synthetic QA labeling, and the fact that licensed real KRX provider/credentials/provisioning are still external blockers.

- [ ] **Step 5: Run focused verification**

Run:

```text
cargo fmt --all -- --check
cargo test -p job-queue
cargo test -p api-server
cargo clippy -p job-queue -p api-server --all-targets --all-features -- -D warnings
uv run --project nt pytest nt/strategies/tests -q
pnpm --dir apps/web test -- recommendation-surface.test.tsx
docker compose --env-file deploy/compose/.env.example -f deploy/compose/compose.yml config --no-interpolate
pwsh -File scripts/qa/recommendation-runner-smoke.ps1
git diff --check
```

Expected: every command exits zero; live/provider tests remain explicitly skipped unless their externally provisioned prerequisites are present.

- [ ] **Step 6: Commit**

```bash
git add deploy/compose/compose.yml deploy/compose/.env.example deploy/systemd/lagrange-recommendation-runner.service scripts/qa/recommendation-runner-smoke.ps1 scripts/qa/recommendation-runner-smoke.sh deploy/README.md docs/STATUS.md
git commit -m "docs: ship recommendation runner operations"
```

- [ ] **Step 7: Request final code review**

Use `superpowers:requesting-code-review` against the complete branch, address only verified findings, rerun the full verification block, and confirm the worktree is clean.
