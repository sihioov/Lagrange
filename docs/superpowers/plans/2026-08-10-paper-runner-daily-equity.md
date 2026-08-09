# Paper Runner and Daily Equity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a real Paper worker cycle that executes all due targets and writes verified end-of-day `daily_equity` points.

**Architecture:** Keep session execution in `job-queue`, add a worker-role valuation module there, and add an `api-server` runner library/binary that scans all tenants and reuses `run_and_settle` for settlement/parity/notifications. The cycle receives one deterministic processing date, is restart-safe through existing target guards and the account/date unique key, and reports per-item failures without fabricating rows.

**Tech Stack:** Rust 1.97, Tokio, SQLx/PostgreSQL, `portfolio-model`, curated Parquet bars, existing `api-server` tenancy and notification services.

---

## File map

- Create `crates/job-queue/src/paper_valuation.rs`: worker-role ledger reconstruction, close-bar loading, mark-to-market, immutable/idempotent `daily_equity` persistence.
- Modify `crates/job-queue/src/lib.rs`: export `paper_valuation`.
- Modify `crates/api-server/src/repos/pending_targets.rs`: add worker-wide due-row DTO/query; keep actor-scoped methods unchanged.
- Create `crates/api-server/src/paper_runner.rs`: one-cycle orchestration, owner-role lookup, active Paper account scan, cycle report/error types.
- Modify `crates/api-server/src/lib.rs`: export `paper_runner`.
- Create `crates/api-server/src/bin/paper-runner.rs`: CLI/env parsing, pools, polling/backoff, Ctrl-C handling.
- Create `crates/job-queue/tests/paper_valuation.rs` or add to the existing integration harness: valuation and cross-tenant DB seams.
- Create `crates/api-server/tests/paper_runner.rs`: worker due scan and real cycle integration seams.

## Task 1: Add worker due-scan contract

**Files:**
- Modify: `crates/api-server/src/repos/pending_targets.rs`
- Test: `crates/api-server/tests/paper_runner.rs`

- [x] **Step 1: Write the failing worker-scan test.**

Add a DB-gated test that queues one target for each of two owners, queues one future target, settles one target, then calls the new worker query through a plain worker pool. Assert only the two due `PENDING` rows are returned and that the order is `(effective_date, account_id, id)`; assert each row carries its `owner_user_id`.

```rust
let rows = PendingTargetRepo::due_worker(&h.worker_pool().await, date("2026-01-06"))
    .await
    .expect("worker scan");
assert_eq!(rows.len(), 2);
assert!(rows.windows(2).all(|w| {
    (w[0].effective_date, w[0].account_id, w[0].id)
        <= (w[1].effective_date, w[1].account_id, w[1].id)
}));
assert!(rows.iter().all(|r| r.status == "PENDING"));
```

- [x] **Step 2: Run the focused test to verify it fails.**

Run: `cargo test -p api-server --test paper_runner worker_scan -- --nocapture`  
Expected: compile failure because `due_worker` and the worker DTO do not exist.

- [x] **Step 3: Implement the minimal worker DTO/query.**

Add `WorkerPendingTargetRow` with `owner_user_id` plus the existing target fields and this method:

```rust
pub async fn due_worker(
    pool: &sqlx::PgPool,
    session_date: NaiveDate,
) -> TenancyResult<Vec<WorkerPendingTargetRow>> {
    sqlx::query_as::<_, WorkerPendingTargetRow>(
        "SELECT id, account_id, owner_user_id, strategy_config_id, computed_on,
                effective_date, targets_json, dataset_version, status,
                executed_at, created_at
         FROM pending_targets
         WHERE status = 'PENDING' AND effective_date <= $1
         ORDER BY effective_date, account_id, id",
    )
    .bind(session_date)
    .fetch_all(pool)
    .await
    .map_err(TenancyError::from_sqlx)
}
```

- [x] **Step 4: Run the focused test to verify it passes.**

Run the same command; expected: PASS when `DATABASE_URL` points to the disposable QA database, or the existing explicit SKIP when it is absent.

- [x] **Step 5: Commit.**

```text
git add crates/api-server/src/repos/pending_targets.rs crates/api-server/tests/paper_runner.rs
git commit -m "feat(paper): add worker-wide pending target scan"
```

## Task 2: Implement ledger-derived daily valuation

**Files:**
- Create: `crates/job-queue/src/paper_valuation.rs`
- Modify: `crates/job-queue/src/lib.rs`
- Test: `crates/api-server/tests/paper_valuation.rs`

- [x] **Step 1: Write failing valuation tests.**

Cover the real worker seam with four DB-gated tests: a held position writes exact `equity`, `cash`, and `positions_value`; a second call returns `AlreadyValued`; a conflicting pre-existing row returns `DailyEquityConflict`; and missing/future close data leaves no row. Use the existing curated fixture writer from `paper_execution_seam.rs` so the test does not depend on the known phase-0 scale defect.

```rust
let first = value_account(&h.worker_pool().await, data.root(), account, owner, date("2026-01-06"))
    .await
    .expect("valuation");
assert!(matches!(first, ValuationOutcome::Valued { .. }));
let row: (String, String, String) = sqlx::query_as(
    "SELECT equity::text, cash::text, positions_value::text
     FROM daily_equity WHERE account_id = $1 AND trading_date = $2",
)
.bind(account).bind(date("2026-01-06")).fetch_one(&h.worker_pool().await).await.unwrap();
assert_eq!(row, ("10000000.0000".into(), "9000000.0000".into(), "1000000.0000".into()));
```

- [x] **Step 2: Run the focused tests to verify they fail.**

Run: `cargo test -p api-server --test paper_valuation -- --nocapture`  
Expected: compile failure because `paper_valuation::value_account` and its outcome/error types do not exist.

- [x] **Step 3: Implement typed valuation errors and account loading.**

Add `ValuationOutcome::{Valued { equity, cash, positions_value }, AlreadyValued}` and `ValuationError` variants for database, account unavailable, cost profile, prices, missing mark, and conflicting row. In one transaction, validate `ACTIVE`/`PAPER`, resolve and version-check `CostProfile`, parse currency, verify `cash_ledger` running balance equals `SUM(amount)`, and parse non-zero positions with explicit account/owner predicates.

- [x] **Step 4: Implement close loading and future-close guard.**

Read each held instrument from `CurateStore::new(dataset_root.join("curated"))` using the existing `kr`/version-1 partition convention. Select the row for the requested date, reject unreadable files, reject an absent row as `MissingMark`, and reject `bar.market_close_ts.as_datetime() > Utc::now()` as `CloseNotYetAvailable`.

- [x] **Step 5: Apply the portfolio event and persist atomically.**

Build a `LedgerState` with the loaded cash/positions and resolved profile, call `close_valuation_event`, apply the event, derive `positions_value = equity - cash`, and insert the daily row. On conflict, fetch the existing values inside the same transaction; return `AlreadyValued` only for exact equality, otherwise return `DailyEquityConflict` and roll back.

- [x] **Step 6: Run focused tests and clippy.**

Run: `cargo test -p api-server --test paper_valuation -- --nocapture` and `cargo clippy -p job-queue --all-targets --all-features -- -D warnings`. Expected: all valuation tests PASS and clippy is clean.

- [x] **Step 7: Commit.**

```text
git add crates/job-queue/src/paper_valuation.rs crates/job-queue/src/lib.rs crates/api-server/tests/paper_valuation.rs
git commit -m "feat(paper): persist ledger-derived daily equity"
```

## Task 3: Add one-cycle Paper runner orchestration

**Files:**
- Create: `crates/api-server/src/paper_runner.rs`
- Modify: `crates/api-server/src/lib.rs`
- Test: `crates/api-server/tests/paper_runner.rs`

- [x] **Step 1: Write the failing cycle test.**

Create an active Paper account, a due target, and fixture bars. Call `run_cycle` with the existing `ApiState`, app/admin/audit pools, worker pool, dataset root, and `TradingDate`. Assert the cycle report sees and settles one target, the target becomes `EXECUTED`/`SKIPPED` according to the real engine result, the ledger has the expected order/fill, and a second cycle does not add rows or notifications.

```rust
let first = run_cycle(&services, date("2026-01-06")).await.expect("cycle");
assert_eq!(first.targets_seen, 1);
let second = run_cycle(&services, date("2026-01-06")).await.expect("idempotent cycle");
assert_eq!(second.targets_seen, 0);
```

- [x] **Step 2: Run the focused test to verify it fails.**

Run: `cargo test -p api-server --test paper_runner cycle -- --nocapture`  
Expected: compile failure because `paper_runner::run_cycle` and service/report types do not exist.

- [x] **Step 3: Implement service and report types.**

Define a `RunnerServices` struct containing `ApiState`, worker pool, dataset root, and a report with `targets_seen`, `targets_settled`, `valuations_seen`, `valuations_written`, and `item_errors`. Add worker account scan SQL for `ACTIVE` `PAPER` accounts ordered by `(owner_user_id, id)`.

- [x] **Step 4: Implement target execution using the existing seam.**

For each `WorkerPendingTargetRow`, query whether the owner has the `owner` role, construct `Actor::owner` or `Actor::member`, and call `run_and_settle(&state, &worker_pool, &dataset_root, &actor, row.id)`. Count successful settlements and record `NotFound` as a benign race; record other item errors and continue.

- [x] **Step 5: Implement account valuation in the same cycle.**

For every scanned active Paper account, call `value_account`. Count `Valued` and `AlreadyValued` as successful, record typed valuation blocks/errors per account, and keep processing other accounts. Propagate only worker-wide scan/connection errors.

- [x] **Step 6: Run the focused cycle test and existing seams.**

Run: `cargo test -p api-server --test paper_runner -- --nocapture` and `cargo test -p api-server --test paper_execution_seam -- --nocapture`. Expected: PASS with no duplicate target effects.

- [x] **Step 7: Commit.**

```text
git add crates/api-server/src/paper_runner.rs crates/api-server/src/lib.rs crates/api-server/tests/paper_runner.rs
git commit -m "feat(paper): orchestrate worker execution and valuation"
```

## Task 4: Add the daemon binary and date/config contract

**Files:**
- Create: `crates/api-server/src/bin/paper-runner.rs`
- Test: `crates/api-server/src/paper_runner.rs` unit tests or `crates/api-server/tests/paper_runner.rs`

- [x] **Step 1: Write failing CLI parsing tests.**

Test `--once`, `--date 2026-01-06`, unknown arguments, malformed dates, and missing `DATABASE_URL`/`WORKER_DATABASE_URL` validation through pure parsing helpers.

```rust
assert_eq!(parse_args(["paper-runner", "--once", "--date", "2026-01-06"]).unwrap().date,
           Some(date("2026-01-06")));
assert!(parse_args(["paper-runner", "--date", "not-a-date"]).is_err());
```

- [x] **Step 2: Run tests to verify they fail.**

Run: `cargo test -p api-server paper_runner::tests -- --nocapture`  
Expected: compile failure because the binary/helper parser is absent.

- [x] **Step 3: Implement the binary contract.**

Parse `--once` and `--date`; otherwise use `PAPER_DATE`; otherwise derive `Utc::now().with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()).date_naive()`. Require `DATABASE_URL` and `WORKER_DATABASE_URL`, connect app/worker/admin/audit pools, build `ApiState::from_pools`, resolve `LAGRANGE_DATASET_ROOT` or `<repo>/data/phase0`, and call `run_cycle`.

Use a two-second idle delay, ten-second error backoff, and `tokio::signal::ctrl_c()` checked between cycles. `--once` exits success after one cycle and failure if the cycle-level scan fails.

- [x] **Step 4: Run CLI and compile tests.**

Run: `cargo test -p api-server --bin paper-runner -- --nocapture` and `cargo check -p api-server --bins`. Expected: parser tests PASS and all binaries compile.

- [x] **Step 5: Commit.**

```text
git add crates/api-server/src/bin/paper-runner.rs crates/api-server/src/paper_runner.rs
git commit -m "feat(paper): add Paper runner daemon"
```

## Task 5: Full verification and status handoff

**Files:**
- Modify: `docs/STATUS.md` only after code is verified

- [x] **Step 1: Run focused Rust suites.**

Run: `cargo test -p job-queue --all-targets`, `cargo test -p api-server --all-targets`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Expected: PASS/clean; DB-gated suites may report their existing explicit SKIP when no `DATABASE_URL` is available.

- [x] **Step 2: Run the existing workspace gates.**

Run the repository's documented Rust, Python, web, and `openapi:check` commands from `docs/STATUS.md` after the focused suites pass. Do not run a gate concurrently with an in-progress edit. Rust workspace tests/clippy, Python (239 passed, 1 skipped), web typecheck/Vitest (48 passed), and OpenAPI passed. The existing web Biome lint remains a pre-existing CRLF/formatter baseline failure and is not changed by this task.

- [x] **Step 3: Inspect the diff and status.**

Run `git diff --check`, `git status --short`, and inspect the new binary help output. Confirm no LIVE account can be valued or executed, no future close is written, and no unrelated files changed.

- [ ] **Step 4: Update the status snapshot.**

Change only the Phase 2/§4.3 statements that say the runner and daily equity are absent, record the implementation commit hashes and verification counts, and explicitly leave the external data/auth/KIS blockers and phase-0 approval decisions unchanged.

- [ ] **Step 5: Commit the status update.**

```text
git add docs/STATUS.md
git commit -m "docs: record Paper runner and daily equity completion"
```
