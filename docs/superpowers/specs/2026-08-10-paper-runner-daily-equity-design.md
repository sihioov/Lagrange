# Paper Runner and Daily Equity Design

**Date:** 2026-08-10  
**Scope:** STATUS.md §4.3 items 1 and 2

## Goal

Make Paper accounts execute queued `pending_targets` in a real worker daemon
and persist one honest end-of-day `daily_equity` point per Paper account. The
implementation must reuse the existing deterministic Paper session engine,
preserve tenant isolation, remain restart-safe, and refuse to value an account
when a held instrument has no verified close price.

## Context and constraints

- `job_queue::paper_execution::execute_session` already reads a Paper account,
  plans the next-open rebalance, and atomically writes orders, fills, cash, and
  positions using the trusted `worker` role.
- `api_server::paper_session::run_and_settle` already guards target status,
  settles `PENDING -> EXECUTED|SKIPPED`, checks ledger evidence, computes
  parity, and routes notifications. The daemon must call this seam rather
  than duplicate it.
- `portfolio_model::paper_flow::close_valuation_event` already enforces that
  every held position has a close mark before adding an equity point.
- Tenant tables use FORCE RLS. A worker-wide query is allowed only through the
  `worker` role, and every ledger mutation must still bind both
  `account_id` and `owner_user_id`.
- `daily_equity` is a cache of the ledger-derived valuation, not a new
  authority. A historical point is immutable for this release: a repeated
  identical valuation is a no-op; a different value for the same account/date
  is an error.
- A target's effective date is the session date for the open execution. The
  daemon receives one explicit processing date for deterministic replay. It
  accepts `--date YYYY-MM-DD` or `PAPER_DATE`; without either it derives the
  current date in `Asia/Seoul`.
- A close bar is usable only when its `market_close_ts` is not in the future.
  This prevents a current-day process from consuming a close that has not
  happened yet. Missing or unreadable bars leave the account unvalued and are
  reported; they never produce a fabricated mark.

## Recommended architecture

### 1. Worker-facing Paper repositories

Add worker-scoped read helpers in `crates/api-server/src/repos/pending_targets.rs`
and a small Paper-account scan helper. These helpers use a supplied worker
pool and return rows containing the target/account owner UUID needed to create
an actor context. They do not replace the actor-scoped API methods.

The worker scan is read-only and ordered by `(effective_date, account_id, id)`.
It returns only `status = 'PENDING' AND effective_date <= process_date`.
The existing status-guarded `run_and_settle` call remains the race winner:
when another daemon settles first, the loser observes `NotFound` and records
the race without writing a second notification.

### 2. Paper runner library and binary

Create `crates/api-server/src/paper_runner.rs` with a testable one-cycle
function and `crates/api-server/src/bin/paper-runner.rs` as the process entry
point.

The cycle performs these steps:

1. Scan due targets with the worker pool.
2. For each target, construct an actor from its owner UUID (Owner role when
   `user_roles` says `owner`, otherwise Member) and call
   `paper_session::run_and_settle` with the worker pool and dataset root.
3. Scan active Paper accounts with the worker pool.
4. Call `job_queue::paper_valuation::value_account` once per account for the
   processing date. A successful insert or an identical existing row counts as
   success; a missing/future/unreadable close is a per-account blocked result.
5. Return a cycle report with target counts, valuation counts, and structured
   per-item errors. A worker-pool scan/connection failure is a cycle error so
   the daemon backs off; an individual account/target failure is reported and
   the cycle continues.

The binary mirrors `backtest-runner` operational behavior:

- `--once` performs one cycle and exits (used by tests and gates).
- default mode repeats with a short idle delay and a longer error backoff.
- `DATABASE_URL` is the app-role URL used by `ApiState` and actor-scoped
  repositories; `WORKER_DATABASE_URL` is required for worker reads/writes;
  `ADMIN_DATABASE_URL` and `AUDIT_DATABASE_URL` provide the read/notification
  pools required by `ApiState`.
- `LAGRANGE_DATASET_ROOT` selects the curated dataset root.
- Ctrl-C stops between cycles; an in-flight session is allowed to settle.

The process does not claim that a valuation happened when the close timestamp
is still future, and it does not mark a target `EXECUTED` without the existing
ledger-evidence check.

### 3. Ledger-derived valuation module

Add `crates/job-queue/src/paper_valuation.rs` and export it from the crate.
The public operation accepts a worker pool, curated dataset root, account UUID,
owner UUID, and `TradingDate`.

Within one database transaction it:

1. Validates the account is `ACTIVE`, `PAPER`, has a resolvable/version-matched
   cost profile, and has a valid currency.
2. Reads the latest cash ledger balance and recomputed `SUM(amount)` and
   refuses a mismatch or missing history.
3. Reads non-zero positions with the same integer quantity contract used by
   Paper execution.
4. Loads the raw close bar for every held instrument from the immutable
   curated partition, checks the bar's trading date and `market_close_ts`, and
   refuses missing, unreadable, or future data.
5. Builds a `LedgerState`, calls `close_valuation_event`, applies the mark, and
   derives `equity`, `cash`, and `positions_value` from that state.
6. Inserts `daily_equity(account_id, owner_user_id, trading_date, ...)`.
   `ON CONFLICT` is handled by reading the existing row: equal values return an
   idempotent `AlreadyValued`, while any mismatch returns a conflict error.
   The transaction commits only after the complete valuation is validated.

No valuation writes `positions.avg_price`, updates cash, or uses adjusted
prices. The close mark is the raw close used by the Paper flow.

## Error and restart behavior

- A database failure while scanning or committing aborts the cycle and causes
  the daemon's error backoff.
- A target that cannot be parsed, executed, or settled is left auditable by
  `run_and_settle`'s existing `SKIPPED`/notification behavior; the next target
  is still attempted.
- A valuation with missing/future/unreadable data writes no row. The report
  identifies the account and reason so operators can retry after the curated
  close arrives.
- Re-running a settled target cannot trade again because the API seam requires
  `PENDING` and the engine has deterministic ledger evidence. Re-running a
  valuation cannot create a second row because `(account_id, trading_date)` is
  unique and equal values are treated as idempotent.
- A worker crash before a transaction commit leaves no partial ledger or
  valuation rows. A crash after commit but before settlement is recovered by
  the existing `AlreadyExecuted` path and target settlement guard.

## Testing strategy

Test-first work will add the following seams before implementation:

1. A worker due scan sees targets across two owners, orders them
   deterministically, and excludes future/settled targets.
2. A runner cycle invokes the real `run_and_settle` path for a due target and
   leaves the target/ledger/notification in the same state as the existing
   seam tests; a second cycle is a no-op for that target.
3. Valuation writes the exact ledger-derived equity, cash, and positions value;
   a second invocation is idempotent; a conflicting existing row is refused.
4. Valuation refuses a missing held-instrument close and a close whose
   `market_close_ts` is in the future, with no `daily_equity` row left behind.
5. Cross-tenant valuation attempts cannot write another owner's account, and a
   LIVE account is refused by the same account-type guard as Paper execution.
6. The binary's `--once`/date parsing and missing required environment values
   fail clearly without starting a polling loop.

Existing portfolio-model, Paper execution, notification, and RLS suites remain
the regression baseline. Full Rust workspace tests plus clippy are required
before completion.

## Files in scope

- Create: `crates/job-queue/src/paper_valuation.rs`
- Modify: `crates/job-queue/src/lib.rs`
- Create: `crates/api-server/src/paper_runner.rs`
- Create: `crates/api-server/src/bin/paper-runner.rs`
- Modify: `crates/api-server/src/lib.rs`
- Modify: `crates/api-server/src/repos/pending_targets.rs`
- Add focused integration/unit tests alongside the owning crates

Deployment wiring is intentionally limited to the binary contract in this
change; the existing Compose service placeholders remain a separate image and
release packaging concern.
