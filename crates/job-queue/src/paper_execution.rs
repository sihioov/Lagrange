//! Executing one Paper session against the database (plan Todo 31/32; design
//! §9.2 processing order, §9.3 execution price, §10.2).
//!
//! `portfolio_model::paper_flow` already decides WHAT a session does: given a
//! ledger state, a target and the session's raw opens it produces the whole
//! sells-then-buys event stream deterministically. Nothing called it. This
//! module is the missing half — the persistence seam that reads the account's
//! real state out of Postgres, hands it to that planner, and writes the
//! resulting orders, fills and cash movements back.
//!
//! # Why this lives in `job-queue`
//!
//! The paper-runner is a worker daemon, not a request. `api-server` depends on
//! this crate (never the reverse), so the engine belongs here and the API layer
//! composes it with settlement.
//!
//! # The role and the predicate
//!
//! Every statement runs on a plain `worker`-role pool and binds BOTH
//! `account_id` and `owner_user_id`. `resolver.rs` states the rule this follows:
//! the `worker` role's RLS policy on these tables is `USING (true)` — it serves
//! every tenant and has no `app.actor_user_id` to be filtered by — so the
//! predicate is the ONLY thing standing between a session and another tenant's
//! ledger.
//!
//! # Two different idempotency questions
//!
//! A session can be asked for twice, and the two ways that happens need
//! different answers:
//!
//! - The target is no longer `PENDING` (it settled EXECUTED or SKIPPED). That
//!   is the CALLER's guard, not this function's — see
//!   `api_server::paper_session::run_and_settle`. Re-running a SKIPPED target
//!   here would trade a stale session at today's call, and nothing in the plan
//!   would catch it: `plan_session_open`'s date guard compares the target
//!   against the session date the caller passes, which is the target's own.
//! - The runner crashed between this function's COMMIT and the settle. The
//!   target is still `PENDING` and the orders are already in the ledger. That
//!   is what [`ExecutionOutcome::AlreadyExecuted`] is for: the same predicate
//!   `paper_session::ledger_evidence` uses answers it, and nothing is written a
//!   second time.
//!
//! # What this deliberately does not write
//!
//! `daily_equity`. The close valuation is a SEPARATE, later step of the
//! documented flow (`DailyBarClosedEvent(T+1)`), and
//! `paper_flow::close_valuation_event` needs close prices the session open does
//! not have. Marking positions at the open would put a fabricated point on the
//! equity curve, which is the class of second-source-of-truth defect this
//! codebase keeps removing. `positions.avg_price` is left NULL for the same
//! reason: no cost-basis policy exists in `portfolio-model`, and a number
//! invented here would be worse than the honest "not reported" the readers
//! already render.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use chrono::NaiveDate;
use domain::{Currency, InstrumentId, Money, Price, Quantity, TradingDate, Weight};
use market_data::CurateStore;
use market_data::curate::schema::read_bars;
use portfolio_model::cost::CostProfile;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::paper_flow::{PendingTarget, plan_session_open};
use portfolio_model::side::Side;
use portfolio_model::sizing::TargetAllocation;
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::paper_io::{BlockingIoError, PAPER_CURATED_IO_DEADLINE, run_bounded_blocking};
use crate::phase0::CURATED_VERSION;

/// PostgreSQL limits used by every Paper transaction.
///
/// These are deliberately applied with `set_config(..., true)` (the SQL
/// equivalent of `SET LOCAL`) after a transaction is opened.  A Paper write
/// must either commit all of its ledger rows or none of them; a timeout aborts
/// the current statement and the caller rolls the transaction back.
pub const PAPER_STATEMENT_TIMEOUT: &str = "15s";
pub const PAPER_LOCK_TIMEOUT: &str = "5s";

/// Apply the Paper transaction limits without changing the pooled connection's
/// defaults.  Keeping this helper in the worker crate lets the API settlement
/// seam and the valuation/preview engines use exactly the same limits.
pub async fn set_paper_transaction_timeouts(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('statement_timeout', $1, true), \
                set_config('lock_timeout', $2, true)",
    )
    .bind(PAPER_STATEMENT_TIMEOUT)
    .bind(PAPER_LOCK_TIMEOUT)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

/// The curated market partition this product trades.
///
/// One market, one curated dataset version: the same constants
/// `factor-engine`'s fixtures and the backtest runner use.
const MARKET: &str = "kr";

/// The wall-clock instant a session's rows are stamped with.
///
/// NOT `now()`. The runner may execute a session on a different calendar day
/// than the session it is processing (a catch-up or a backfill), and
/// `paper_session::ledger_evidence` looks for orders by
/// `created_at::date = effective_date`. Pinning the timestamp to the session
/// keeps that check true for the session that actually happened, rather than
/// for the day the process happened to run.
const SESSION_STAMP_TIME: &str = "00:30:00";

/// One session to execute: the target, the account, and who owns both.
#[derive(Debug, Clone)]
pub struct SessionInput {
    /// The Paper account whose ledger this session moves.
    pub account_id: Uuid,
    /// The account's owner. Bound as a predicate on every statement.
    pub owner_user_id: Uuid,
    /// The session at whose OPEN the target executes.
    pub effective_date: TradingDate,
    /// The target weights, parsed from `pending_targets.targets_json`.
    pub targets: Vec<TargetAllocation>,
}

/// What one call to [`execute_session`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutcome {
    /// Orders were placed, filled, and the ledger was updated.
    Executed {
        /// Orders written to `orders`.
        orders: usize,
        /// Fills written to `fills`.
        fills: usize,
    },
    /// The session's orders were ALREADY in the ledger, so nothing was
    /// written. The session executed; this call simply arrived second.
    AlreadyExecuted {
        /// Orders already recorded for this session.
        orders: usize,
    },
    /// The plan produced no orders — every instrument was inside the
    /// rebalance threshold or below the minimum trade. A deliberate no-trade,
    /// not a failure: nothing was written.
    NoTrade,
}

/// Why a session could not be executed. Typed, never stringly at the seam.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// A statement failed (connection, constraint, or a grant denial).
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// The account, or the cash history it must be replayed from, cannot be
    /// read — or contradicts itself.
    #[error("account {account_id} is not executable: {detail}")]
    AccountUnavailable {
        /// The account.
        account_id: Uuid,
        /// What was missing or inconsistent.
        detail: String,
    },

    /// The account's configured cost profile does not resolve, or resolves to
    /// a version the account was not opened under.
    #[error("cost profile unusable: {0}")]
    CostProfile(String),

    /// The curated price zone could not be read (corrupt or unreadable file).
    /// A merely ABSENT price is not this: it reaches the planner, which names
    /// the instrument in a typed `MissingPrice`.
    #[error("curated prices unreadable: {0}")]
    Prices(String),

    /// `targets_json` is not a weight vector this system can execute.
    #[error("target weights are unreadable: {0}")]
    Targets(String),

    /// The planner refused the session (wrong session date, missing price,
    /// zero equity, ...). Nothing was written.
    #[error("the session could not be planned: {0}")]
    Plan(String),

    /// The ledger rejected an event the planner produced. Unreachable while
    /// the planner's output is self-consistent — surfaced rather than
    /// unwrapped, because "impossible" is where the money goes missing.
    #[error("the ledger rejected a planned event: {0}")]
    Ledger(String),

    /// The account changed after its database snapshot was read and before
    /// curated prices finished loading. The caller must retry from a fresh
    /// snapshot; no ledger rows were written.
    #[error("Paper account {account_id} changed while preparing the session")]
    AccountChanged { account_id: Uuid },

    /// A dataset/entitlement preflight changed while curated prices were
    /// being loaded. This is kept typed so the API can settle a denied target
    /// as a deliberate skip without branching on SQL text.
    #[error("Paper execution preflight denied: {code}: {message}")]
    PreflightDenied { code: String, message: String },
}

/// Parses `pending_targets.targets_json` into the sizer's own target type.
///
/// The stored shape is `[{"instrument_id": "069500.KRX", "weight": "0.6"}]` —
/// [`TargetAllocation`]'s wire shape, as migration 0014 documents. Weights are
/// decimal STRINGS, never floats: `Weight` is scale-6 fixed point and a float
/// round-trip is exactly the precision loss this codebase's money types exist
/// to prevent.
pub fn targets_from_json(
    value: &serde_json::Value,
) -> Result<Vec<TargetAllocation>, ExecutionError> {
    let rows = value.as_array().ok_or_else(|| {
        ExecutionError::Targets("expected a JSON array of allocations".to_owned())
    })?;
    let mut targets = Vec::with_capacity(rows.len());
    for row in rows {
        let instrument = row
            .get("instrument_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ExecutionError::Targets(format!("missing instrument_id in {row}")))?;
        let weight = row.get("weight").and_then(|v| v.as_str()).ok_or_else(|| {
            ExecutionError::Targets(format!("weight must be a decimal string in {row}"))
        })?;
        targets.push(TargetAllocation {
            instrument_id: InstrumentId::parse(instrument)
                .map_err(|e| ExecutionError::Targets(format!("{instrument:?}: {e}")))?,
            weight: Weight::parse(weight)
                .map_err(|e| ExecutionError::Targets(format!("{weight:?}: {e}")))?,
        });
    }
    Ok(targets)
}

/// Executes one Paper session's open and persists everything it produced.
///
/// Database snapshots and curated-file reads are deliberately separate. The
/// file read is bounded blocking work with no transaction held; a short final
/// transaction revalidates the account snapshot and writes every order, fill,
/// cash row, and position atomically. A concurrent executor is serialized by
/// the account row lock and the deterministic order-ref uniqueness fence.
pub async fn execute_session(
    pool: &PgPool,
    dataset_root: &Path,
    input: &SessionInput,
) -> Result<ExecutionOutcome, ExecutionError> {
    let snapshot = read_execution_snapshot(pool, input).await?;
    if snapshot.already > 0 {
        return Ok(ExecutionOutcome::AlreadyExecuted {
            orders: snapshot.already as usize,
        });
    }
    let target = PendingTarget {
        account_id: input.account_id,
        effective_date: input.effective_date,
        targets: input.targets.clone(),
    };
    let open_prices = load_session_opens_bounded(dataset_root, &snapshot.state, &target).await?;

    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let current = execution_snapshot_in_tx(&mut tx, input).await?;
    if current.already > 0 {
        tx.rollback().await?;
        return Ok(ExecutionOutcome::AlreadyExecuted {
            orders: current.already as usize,
        });
    }
    if current.state != snapshot.state {
        tx.rollback().await?;
        return Err(ExecutionError::AccountChanged {
            account_id: input.account_id,
        });
    }
    let outcome =
        execute_prepared_in_tx(&mut tx, input, &current.state, &target, open_prices).await;
    match outcome {
        Ok(executed @ ExecutionOutcome::Executed { .. }) => {
            tx.commit().await?;
            Ok(executed)
        }
        Ok(other) => {
            tx.rollback().await?;
            Ok(other)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

/// Executes a target whose recommendation lineage requires the database
/// preflight gate. The gate is checked once before any file work for fast
/// denial, then checked again in the final write transaction. That second
/// check preserves the entitlement/target lock fence without holding it while
/// a curated partition is read.
pub async fn execute_session_with_preflight(
    pool: &PgPool,
    dataset_root: &Path,
    target_id: Uuid,
    input: &SessionInput,
) -> Result<ExecutionOutcome, ExecutionError> {
    {
        let mut tx = pool.begin().await?;
        set_paper_transaction_timeouts(&mut tx).await?;
        if let Some(denial) = preflight_in_tx(&mut tx, target_id, input.owner_user_id).await? {
            // The trusted preflight function may have prepared a SKIPPED
            // update, but the Paper API must commit that terminal state only
            // together with its durable settlement-notification outbox row.
            // Roll this probe back; `paper_session` performs the atomic
            // terminal transition and enqueue after it has the typed reason.
            tx.rollback().await?;
            return Err(ExecutionError::PreflightDenied {
                code: denial.code,
                message: denial.message,
            });
        }
        tx.commit().await?;
    }

    let snapshot = read_execution_snapshot(pool, input).await?;
    if snapshot.already > 0 {
        return Ok(ExecutionOutcome::AlreadyExecuted {
            orders: snapshot.already as usize,
        });
    }
    let target = PendingTarget {
        account_id: input.account_id,
        effective_date: input.effective_date,
        targets: input.targets.clone(),
    };
    let open_prices = load_session_opens_bounded(dataset_root, &snapshot.state, &target).await?;

    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    if let Some(denial) = preflight_in_tx(&mut tx, target_id, input.owner_user_id).await? {
        tx.rollback().await?;
        return Err(ExecutionError::PreflightDenied {
            code: denial.code,
            message: denial.message,
        });
    }
    let current = execution_snapshot_in_tx(&mut tx, input).await?;
    if current.already > 0 {
        tx.rollback().await?;
        return Ok(ExecutionOutcome::AlreadyExecuted {
            orders: current.already as usize,
        });
    }
    if current.state != snapshot.state {
        tx.rollback().await?;
        return Err(ExecutionError::AccountChanged {
            account_id: input.account_id,
        });
    }
    let outcome =
        execute_prepared_in_tx(&mut tx, input, &current.state, &target, open_prices).await;
    match outcome {
        Ok(executed @ ExecutionOutcome::Executed { .. }) => {
            tx.commit().await?;
            Ok(executed)
        }
        Ok(other) => {
            tx.rollback().await?;
            Ok(other)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionSnapshot {
    state: LedgerState,
    already: i64,
}

#[derive(Debug)]
struct PreflightDenial {
    code: String,
    message: String,
}

async fn read_execution_snapshot(
    pool: &PgPool,
    input: &SessionInput,
) -> Result<ExecutionSnapshot, ExecutionError> {
    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let snapshot = execution_snapshot_in_tx(&mut tx, input).await?;
    tx.commit().await?;
    Ok(snapshot)
}

async fn execution_snapshot_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
) -> Result<ExecutionSnapshot, ExecutionError> {
    let (profile, currency) = account_profile(tx, input).await?;
    let cash = account_cash(tx, input, currency).await?;
    let positions = account_positions(tx, input).await?;
    let already: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM orders \
         WHERE account_id = $1 AND owner_user_id = $2 AND created_at::date = $3::date",
    )
    .bind(input.account_id)
    .bind(input.owner_user_id)
    .bind(input.effective_date.as_naive_date())
    .fetch_one(&mut **tx)
    .await?;
    Ok(ExecutionSnapshot {
        state: LedgerState {
            base_currency: currency,
            cost_profile: profile,
            cash,
            positions,
            orders: BTreeMap::new(),
            fills: Vec::new(),
            marks: BTreeMap::new(),
            equity_curve: BTreeMap::new(),
            last_seq: 0,
        },
        already,
    })
}

async fn preflight_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target_id: Uuid,
    owner_user_id: Uuid,
) -> Result<Option<PreflightDenial>, ExecutionError> {
    let (authorized, reason): (bool, Option<serde_json::Value>) =
        sqlx::query_as("SELECT authorized, reason FROM public.preflight_paper_target($1, $2)")
            .bind(target_id)
            .bind(owner_user_id)
            .fetch_one(&mut **tx)
            .await?;
    if authorized {
        return Ok(None);
    }
    let code = reason
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("PAPER_PREFLIGHT_DENIED")
        .to_owned();
    let message = reason
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Paper execution preflight denied execution")
        .to_owned();
    Ok(Some(PreflightDenial { code, message }))
}

async fn execute_prepared_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
    state: &LedgerState,
    target: &PendingTarget,
    open_prices: BTreeMap<InstrumentId, Price>,
) -> Result<ExecutionOutcome, ExecutionError> {
    // No instrument-master lot-size source is wired in this repository yet,
    // and `sizing.rs` documents the fallback: "absent = 1". Inventing one
    // here would be a second instrument master.
    let lot_sizes: BTreeMap<InstrumentId, u64> = BTreeMap::new();
    let plan = plan_session_open(
        state,
        target,
        &input.effective_date,
        &open_prices,
        &lot_sizes,
    )
    .map_err(|e| ExecutionError::Plan(e.to_string()))?;
    if plan.report.orders.is_empty() {
        return Ok(ExecutionOutcome::NoTrade);
    }

    // Apply before writing: the ledger validates cash, positions and fees, and
    // on a reject leaves the state untouched. The rows below are what a VALID
    // ledger produced, never a hopeful transcription of the plan.
    let mut applied = state.clone();
    for event in &plan.events {
        applied
            .apply(event.clone())
            .map_err(|e| ExecutionError::Ledger(e.to_string()))?;
    }
    persist(tx, input, &plan.events, &applied, state).await?;
    Ok(ExecutionOutcome::Executed {
        orders: plan.report.orders.len(),
        fills: applied.fills.len(),
    })
}

async fn load_session_opens_bounded(
    dataset_root: &Path,
    state: &LedgerState,
    target: &PendingTarget,
) -> Result<BTreeMap<InstrumentId, Price>, ExecutionError> {
    let dataset_root = dataset_root.to_path_buf();
    let state = state.clone();
    let target = target.clone();
    match run_bounded_blocking(PAPER_CURATED_IO_DEADLINE, None, move |canceled| {
        session_opens(&dataset_root, &state, &target, &canceled)
    })
    .await
    {
        Ok(opens) => Ok(opens),
        Err(BlockingIoError::Failed(error)) => Err(error),
        Err(BlockingIoError::Canceled) => Err(ExecutionError::Prices(
            "curated price read canceled during shutdown".to_owned(),
        )),
        Err(BlockingIoError::TimedOut) => Err(ExecutionError::Prices(format!(
            "curated price read exceeded {PAPER_CURATED_IO_DEADLINE:?}"
        ))),
    }
}

/// The account's cost profile and currency.
///
/// The stored `cost_profile_version` is CHECKED, not merely read.
/// `cost.rs` is explicit that rates are configuration and that any change is a
/// new version; filling an account at a version it was never opened under would
/// charge fees nobody agreed to, silently.
///
/// `account_type = 'PAPER'` is checked here, not assumed. This engine is the
/// only writer of `orders`/`fills`/`cash_ledger` in the repository, and those
/// same tables are what `risk_snapshot::account_state` reads to build the
/// LIVE Risk Gateway's cash and position inputs. Nothing upstream of this
/// function currently constrains `pending_targets.account_id` to a PAPER
/// account (`repos::pending_targets::queue` performs no type check, and the
/// migration puts no CHECK on it), so without this predicate a target queued
/// against a LIVE account would let a simulator write FILLED orders and cash
/// movements that never happened directly into the ledger the safety gate
/// trusts. "No caller reaches this today" is not a reason to leave it open --
/// that was exactly how `risk_gateway::testing::snapshot_all_green()` reached
/// production earlier in this codebase's history.
async fn account_profile(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
) -> Result<(CostProfile, Currency), ExecutionError> {
    let row: Option<(String, i32, String)> = sqlx::query_as(
        "SELECT cost_profile_id, cost_profile_version, currency FROM accounts \
         WHERE id = $1 AND owner_user_id = $2 AND status = 'ACTIVE' AND account_type = 'PAPER' \
         FOR UPDATE",
    )
    .bind(input.account_id)
    .bind(input.owner_user_id)
    .fetch_optional(&mut **tx)
    .await?;

    let (profile_id, version, currency) =
        row.ok_or_else(|| ExecutionError::AccountUnavailable {
            account_id: input.account_id,
            detail: "no ACTIVE Paper account with this owner".to_owned(),
        })?;

    let profile = CostProfile::resolve(&profile_id)
        .map_err(|e| ExecutionError::CostProfile(format!("account {}: {e}", input.account_id)))?;
    if i64::from(version) != i64::from(profile.version) {
        return Err(ExecutionError::CostProfile(format!(
            "account {} was opened under {profile_id} version {version}, but this build ships \
             version {}; the rates a fill would be charged are not the ones the account agreed to",
            input.account_id, profile.version
        )));
    }
    let currency = Currency::from_code(&currency)
        .map_err(|e| ExecutionError::CostProfile(format!("account currency {currency:?}: {e}")))?;
    Ok((profile, currency))
}

/// The account's cash, from the AUTHORITY and cross-checked against itself.
///
/// `balance` is the running total the writer maintained; `SUM(amount)` is that
/// same total recomputed from the events. They agree or the ledger contradicts
/// itself, and a session must not trade against cash nobody can agree on. This
/// is the check `risk_snapshot::account_state` makes for the Live gate, made
/// here for the same reason.
async fn account_cash(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
    currency: Currency,
) -> Result<Money, ExecutionError> {
    let unavailable = |detail: String| ExecutionError::AccountUnavailable {
        account_id: input.account_id,
        detail,
    };
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT (SELECT balance::text FROM cash_ledger \
                  WHERE account_id = $1 AND owner_user_id = $2 \
                  ORDER BY seq DESC LIMIT 1), \
                COALESCE(SUM(amount), 0)::text \
         FROM cash_ledger WHERE account_id = $1 AND owner_user_id = $2",
    )
    .bind(input.account_id)
    .bind(input.owner_user_id)
    .fetch_optional(&mut **tx)
    .await?;

    let (running, replayed) =
        row.ok_or_else(|| unavailable("no cash_ledger history".to_owned()))?;
    let running = running.ok_or_else(|| unavailable("no cash_ledger history".to_owned()))?;

    let parse = |s: &str| -> Result<Money, ExecutionError> {
        Money::parse(s, currency).map_err(|e| unavailable(format!("unreadable cash {s:?}: {e}")))
    };
    let cash = parse(&running)?;
    let replayed = parse(&replayed)?;
    if cash != replayed {
        return Err(unavailable(format!(
            "cash_ledger disagrees with itself -- running balance {}, events replay to {}",
            cash.amount(),
            replayed.amount()
        )));
    }
    Ok(cash)
}

/// The account's current positions.
///
/// `numeric(18, 4)` renders as `"10.0000"`, and `Quantity` is scale 0 by type,
/// so the integer part is taken exactly as `risk_snapshot` takes it. A row that
/// has settled to zero is NOT a position (the ledger spells flat as absent), so
/// it is skipped rather than carried as a zero.
async fn account_positions(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
) -> Result<BTreeMap<InstrumentId, Quantity>, ExecutionError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT instrument_id, quantity::text FROM positions \
         WHERE account_id = $1 AND owner_user_id = $2",
    )
    .bind(input.account_id)
    .bind(input.owner_user_id)
    .fetch_all(&mut **tx)
    .await?;

    let unavailable = |detail: String| ExecutionError::AccountUnavailable {
        account_id: input.account_id,
        detail,
    };
    let mut positions = BTreeMap::new();
    for (id, quantity) in rows {
        let instrument = InstrumentId::parse(&id)
            .map_err(|e| unavailable(format!("unreadable position instrument {id:?}: {e}")))?;
        let whole = quantity.split('.').next().unwrap_or("0");
        let quantity = Quantity::parse(whole)
            .map_err(|e| unavailable(format!("unreadable position quantity {quantity:?}: {e}")))?;
        if quantity.is_zero() {
            continue;
        }
        positions.insert(instrument, quantity);
    }
    Ok(positions)
}

/// The RAW opens of every instrument this session could touch.
///
/// Raw, not split-adjusted: design §9.3 executes at the 원시 가격 and
/// `CostProfile::execution_price` applies slippage to that open itself.
///
/// An instrument whose curated file or session row is ABSENT is simply left out
/// of the map. That is not silence — `plan_rebalance` validates that every
/// target and every held position has an open price and fails the whole session
/// closed with a typed `MissingPrice` naming the instrument. A file that exists
/// but cannot be read is a different thing and surfaces as [`ExecutionError::Prices`].
fn session_opens(
    dataset_root: &Path,
    state: &LedgerState,
    target: &PendingTarget,
    canceled: &AtomicBool,
) -> Result<BTreeMap<InstrumentId, Price>, ExecutionError> {
    // The worker is given the dataset root and the curated zone sits one level
    // in, exactly as `runner.rs` reaches it for the factor series.
    let store = CurateStore::new(dataset_root.join("curated"));
    let year = target
        .effective_date
        .as_naive_date()
        .format("%Y")
        .to_string();
    let year: i32 = year
        .parse()
        .map_err(|e| ExecutionError::Prices(format!("session year {year:?}: {e}")))?;

    let mut needed: Vec<InstrumentId> = target
        .targets
        .iter()
        .map(|t| t.instrument_id.clone())
        .collect();
    needed.extend(state.positions.keys().cloned());
    needed.sort();
    needed.dedup();

    let mut opens = BTreeMap::new();
    for instrument in needed {
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ExecutionError::Prices(
                "curated price read canceled during shutdown".to_owned(),
            ));
        }
        let path = store.bars_path(MARKET, &instrument.to_string(), year, CURATED_VERSION);
        if !path.exists() {
            continue;
        }
        let bars = read_bars(&path)
            .map_err(|e| ExecutionError::Prices(format!("read {}: {e}", path.display())))?;
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ExecutionError::Prices(
                "curated price read canceled during shutdown".to_owned(),
            ));
        }
        if let Some(bar) = bars
            .iter()
            .find(|b| b.trading_date == target.effective_date)
        {
            opens.insert(instrument, bar.open);
        }
    }
    Ok(opens)
}

/// Writes the session: one `orders` row per placed order, one `fills` row per
/// fill, one `cash_ledger` row per fill, and the resulting position snapshot.
async fn persist(
    tx: &mut Transaction<'_, Postgres>,
    input: &SessionInput,
    events: &[LedgerEvent],
    applied: &LedgerState,
    before: &LedgerState,
) -> Result<(), ExecutionError> {
    let stamp = session_timestamp(input.effective_date);

    // The account's ACTUAL next sequence, not the planner's local baseline.
    let last_seq: Option<i64> = sqlx::query_scalar(
        "SELECT max(seq) FROM cash_ledger WHERE account_id = $1 AND owner_user_id = $2",
    )
    .bind(input.account_id)
    .bind(input.owner_user_id)
    .fetch_one(&mut **tx)
    .await?;
    let mut next_seq = last_seq.unwrap_or(0) + 1;

    // `order_ref` is the deterministic uuid5 id `paper_flow` mints, so
    // UNIQUE (account_id, order_ref) makes a double-executed session a loud
    // constraint violation that rolls the whole transaction back rather than a
    // second set of fills.
    let mut order_rows: BTreeMap<String, Uuid> = BTreeMap::new();
    for event in events {
        let LedgerEvent::OrderPlaced {
            order_id,
            instrument_id,
            side,
            quantity,
            ..
        } = event
        else {
            continue;
        };
        // The price a Paper order is placed at is the price it fills at: the
        // whole order executes at the modeled open in one execution.
        let fill = applied
            .fills
            .iter()
            .find(|f| f.order_id == *order_id)
            .ok_or_else(|| {
                ExecutionError::Ledger(format!("order {order_id} was placed but never filled"))
            })?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO orders \
             (account_id, owner_user_id, order_ref, instrument_id, side, quantity, price, \
              status, submitted_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6::numeric, $7::numeric, 'FILLED', $8, $8, $8) \
             RETURNING id",
        )
        .bind(input.account_id)
        .bind(input.owner_user_id)
        .bind(order_id.to_string())
        .bind(instrument_id.to_string())
        .bind(side_code(*side))
        .bind(quantity.amount().to_string())
        .bind(fill.price.amount().to_string())
        .bind(stamp)
        .fetch_one(&mut **tx)
        .await?;
        order_rows.insert(order_id.to_string(), id);
    }

    for fill in &applied.fills {
        let order_row = order_rows.get(&fill.order_id.to_string()).ok_or_else(|| {
            ExecutionError::Ledger(format!("fill {} has no placed order", fill.fill_id))
        })?;
        let fees = fill
            .commission
            .checked_add(&fill.tax)
            .map_err(|e| ExecutionError::Ledger(format!("fee total overflows: {e}")))?;
        let fill_row: Uuid = sqlx::query_scalar(
            "INSERT INTO fills \
             (account_id, owner_user_id, order_id, instrument_id, fill_ref, side, quantity, \
              price, fees, ts, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8::numeric, $9::numeric, $10, $10) \
             RETURNING id",
        )
        .bind(input.account_id)
        .bind(input.owner_user_id)
        .bind(order_row)
        .bind(fill.instrument_id.to_string())
        .bind(fill.fill_id.to_string())
        .bind(side_code(fill.side))
        .bind(fill.quantity.amount().to_string())
        .bind(fill.price.amount().to_string())
        .bind(fees.amount().to_string())
        .bind(stamp)
        .fetch_one(&mut **tx)
        .await?;

        // The cash MOVEMENT, taken from the ledger's own before/after rather
        // than recomputed here: `amount` is what the fill did to cash (fees
        // included, signed), `balance` is where it left it. Recomputing either
        // would be a second arithmetic that can disagree with the ledger.
        let amount = fill
            .cash_after
            .amount()
            .checked_sub(&fill.cash_before.amount())
            .map_err(|e| ExecutionError::Ledger(format!("cash delta overflows: {e}")))?;
        sqlx::query(
            "INSERT INTO cash_ledger \
             (account_id, owner_user_id, seq, event_type, amount, balance, currency, \
              reference_id, ts, created_at) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6::numeric, $7, $8, $9, $9)",
        )
        .bind(input.account_id)
        .bind(input.owner_user_id)
        .bind(next_seq)
        .bind(side_code(fill.side))
        .bind(amount.to_string())
        .bind(fill.cash_after.amount().to_string())
        .bind(applied.base_currency.code())
        .bind(fill_row)
        .bind(stamp)
        .execute(&mut **tx)
        .await?;
        next_seq += 1;
    }

    // The position snapshot after every fill. An instrument the session sold
    // out of is absent from the ledger's map (flat is spelled by absence), and
    // its stored row is set to zero rather than deleted: migration 0014 grants
    // the `worker` role INSERT and UPDATE on `positions`, never DELETE.
    let mut instruments: Vec<&InstrumentId> = applied.positions.keys().collect();
    instruments.extend(before.positions.keys());
    instruments.sort();
    instruments.dedup();
    for instrument in instruments {
        let quantity = applied
            .positions
            .get(instrument)
            .map(|q| q.amount().to_string())
            .unwrap_or_else(|| "0".to_owned());
        sqlx::query(
            "INSERT INTO positions \
             (account_id, owner_user_id, instrument_id, quantity, updated_at) \
             VALUES ($1, $2, $3, $4::numeric, $5) \
             ON CONFLICT (account_id, instrument_id) \
             DO UPDATE SET quantity = EXCLUDED.quantity, updated_at = EXCLUDED.updated_at",
        )
        .bind(input.account_id)
        .bind(input.owner_user_id)
        .bind(instrument.to_string())
        .bind(quantity)
        .bind(stamp)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// The session's rows are stamped at its own date, never at the runner's.
fn session_timestamp(date: TradingDate) -> chrono::DateTime<chrono::Utc> {
    let naive: NaiveDate = date.as_naive_date();
    let time = chrono::NaiveTime::parse_from_str(SESSION_STAMP_TIME, "%H:%M:%S")
        .expect("the session stamp time is a literal");
    naive.and_time(time).and_utc()
}

/// The wire spelling of a side, shared by `orders`, `fills` and the cash
/// ledger's `event_type` (both CHECK-constrained to BUY/SELL).
fn side_code(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_parse_from_the_stored_wire_shape() {
        let json = serde_json::json!([
            { "instrument_id": "069500.KRX", "weight": "0.600000" },
            { "instrument_id": "229200.KRX", "weight": "0.400000" }
        ]);
        let targets = targets_from_json(&json).expect("the stored shape parses");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].instrument_id.to_string(), "069500.KRX");
        assert_eq!(targets[0].weight.amount().to_string(), "0.600000");
    }

    #[test]
    fn a_float_weight_is_refused_rather_than_rounded() {
        // A JSON number cannot round-trip a scale-6 weight exactly, and a
        // silently rounded weight is a silently wrong order size.
        let json = serde_json::json!([{ "instrument_id": "069500.KRX", "weight": 0.6 }]);
        assert!(matches!(
            targets_from_json(&json),
            Err(ExecutionError::Targets(_))
        ));
    }

    #[test]
    fn an_unknown_instrument_fails_the_whole_vector() {
        let json = serde_json::json!([{ "instrument_id": "not-an-id", "weight": "1.0" }]);
        assert!(matches!(
            targets_from_json(&json),
            Err(ExecutionError::Targets(_))
        ));
    }

    #[test]
    fn the_session_stamp_lands_on_the_session_date() {
        let date = TradingDate::parse("2020-01-21").expect("valid date");
        let stamp = session_timestamp(date);
        assert_eq!(
            stamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "2020-01-21T00:30:00Z",
            "ledger_evidence matches orders by created_at::date = effective_date"
        );
    }
}
