//! Ledger-derived Paper close valuation.
//!
//! A Paper session writes orders, fills, cash, and positions at the next
//! session open. This module is the later `DailyBarClosedEvent` half of that
//! flow: it rebuilds the account state from the worker-visible ledger, marks
//! every held position at the raw close, and persists one immutable daily
//! equity point.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use chrono::Utc;
use domain::{Currency, InstrumentId, Money, Price, Quantity, TradingDate};
use market_data::CurateStore;
use market_data::curate::schema::read_bars;
use portfolio_model::cost::CostProfile;
use portfolio_model::ledger::LedgerState;
use portfolio_model::paper_flow::close_valuation_event;
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::paper_execution::set_paper_transaction_timeouts;
use crate::paper_io::{BlockingIoError, PAPER_CURATED_IO_DEADLINE, run_bounded_blocking};
use crate::phase0::CURATED_VERSION;

const MARKET: &str = "kr";

/// Result of writing (or discovering) one account/date valuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValuationOutcome {
    /// A new immutable daily point was inserted.
    Valued {
        equity: Money,
        cash: Money,
        positions_value: Money,
    },
    /// The same account/date already carries exactly these values.
    AlreadyValued,
}

/// Why a close valuation could not be written.
#[derive(Debug, Error)]
pub enum ValuationError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("account {account_id} cannot be valued: {detail}")]
    AccountUnavailable { account_id: Uuid, detail: String },

    #[error("cost profile unusable: {0}")]
    CostProfile(String),

    #[error("close prices unreadable: {0}")]
    Prices(String),

    #[error("close price unavailable for {instrument_id} on {date}: {detail}")]
    MissingMark {
        instrument_id: InstrumentId,
        date: TradingDate,
        detail: String,
    },

    #[error("close for {instrument_id} on {date} is not available until {close_at}")]
    CloseNotYetAvailable {
        instrument_id: InstrumentId,
        date: TradingDate,
        close_at: String,
    },

    #[error("ledger valuation failed: {0}")]
    Ledger(String),

    #[error("daily_equity already disagrees for account {account_id} on {date}")]
    DailyEquityConflict { account_id: Uuid, date: TradingDate },

    #[error("Paper account {account_id} changed while preparing valuation")]
    AccountChanged { account_id: Uuid },
}

/// Rebuilds one Paper account from the worker-authoritative ledger and writes
/// its close valuation atomically.
///
/// Curated reads happen after a short snapshot transaction has committed.
/// The final transaction locks/rechecks the account before writing, so a
/// filesystem stall cannot retain a database transaction while still
/// preserving the immutable daily-equity write semantics.
pub async fn value_account(
    pool: &PgPool,
    dataset_root: &Path,
    account_id: Uuid,
    owner_user_id: Uuid,
    date: TradingDate,
) -> Result<ValuationOutcome, ValuationError> {
    let snapshot = read_account_snapshot(pool, account_id, owner_user_id).await?;
    let closes = load_session_closes_bounded(dataset_root, &snapshot.state, date).await?;
    let (equity, cash, positions_value) = calculate_valuation(&snapshot.state, date, &closes)?;

    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let current = account_snapshot_in_tx(&mut tx, account_id, owner_user_id).await?;
    if current != snapshot {
        tx.rollback().await?;
        return Err(ValuationError::AccountChanged { account_id });
    }
    let currency = current.state.base_currency;

    let inserted: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO daily_equity \
         (account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) \
         VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6::numeric, $7) \
         ON CONFLICT (account_id, trading_date) DO NOTHING \
         RETURNING id",
    )
    .bind(account_id)
    .bind(owner_user_id)
    .bind(date.as_naive_date())
    .bind(equity.amount().to_string())
    .bind(cash.amount().to_string())
    .bind(positions_value.amount().to_string())
    .bind(currency.code())
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_some() {
        tx.commit().await?;
        return Ok(ValuationOutcome::Valued {
            equity,
            cash,
            positions_value,
        });
    }

    let existing: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT equity::text, cash::text, positions_value::text, currency \
         FROM daily_equity WHERE account_id = $1 AND trading_date = $2",
    )
    .bind(account_id)
    .bind(date.as_naive_date())
    .fetch_optional(&mut *tx)
    .await?;
    let matches = existing
        .as_ref()
        .map(|(old_equity, old_cash, old_positions, old_currency)| {
            old_equity == &equity.amount().to_string()
                && old_cash == &cash.amount().to_string()
                && old_positions == &positions_value.amount().to_string()
                && old_currency == currency.code()
        })
        .unwrap_or(false);
    if matches {
        tx.commit().await?;
        Ok(ValuationOutcome::AlreadyValued)
    } else {
        tx.rollback().await?;
        Err(ValuationError::DailyEquityConflict { account_id, date })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValuationSnapshot {
    state: LedgerState,
}

async fn read_account_snapshot(
    pool: &PgPool,
    account_id: Uuid,
    owner_user_id: Uuid,
) -> Result<ValuationSnapshot, ValuationError> {
    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let snapshot = account_snapshot_in_tx(&mut tx, account_id, owner_user_id).await?;
    tx.commit().await?;
    Ok(snapshot)
}

async fn account_snapshot_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    owner_user_id: Uuid,
) -> Result<ValuationSnapshot, ValuationError> {
    let (profile, currency) = account_profile(tx, account_id, owner_user_id).await?;
    let cash = account_cash(tx, account_id, owner_user_id, currency).await?;
    let positions = account_positions(tx, account_id, owner_user_id).await?;
    Ok(ValuationSnapshot {
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
    })
}

fn calculate_valuation(
    state: &LedgerState,
    date: TradingDate,
    closes: &BTreeMap<InstrumentId, Price>,
) -> Result<(Money, Money, Money), ValuationError> {
    let event = close_valuation_event(state, date, closes)
        .map_err(|e| ValuationError::Ledger(e.to_string()))?;
    let mut applied = state.clone();
    let effect = applied
        .apply(event)
        .map_err(|e| ValuationError::Ledger(e.to_string()))?;
    let equity = effect
        .equity_after
        .ok_or_else(|| ValuationError::Ledger("mark event produced no equity".to_owned()))?;
    let positions_value = equity
        .checked_sub(&state.cash)
        .map_err(|e| ValuationError::Ledger(format!("positions value: {e}")))?;
    Ok((equity, state.cash, positions_value))
}

async fn load_session_closes_bounded(
    dataset_root: &Path,
    state: &LedgerState,
    date: TradingDate,
) -> Result<BTreeMap<InstrumentId, Price>, ValuationError> {
    let dataset_root = dataset_root.to_path_buf();
    let state = state.clone();
    match run_bounded_blocking(PAPER_CURATED_IO_DEADLINE, None, move |canceled| {
        session_closes(&dataset_root, &state, date, &canceled)
    })
    .await
    {
        Ok(closes) => Ok(closes),
        Err(BlockingIoError::Failed(error)) => Err(error),
        Err(BlockingIoError::Canceled) => Err(ValuationError::Prices(
            "curated close read canceled during shutdown".to_owned(),
        )),
        Err(BlockingIoError::TimedOut) => Err(ValuationError::Prices(format!(
            "curated close read exceeded {PAPER_CURATED_IO_DEADLINE:?}"
        ))),
    }
}

async fn account_profile(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    owner_user_id: Uuid,
) -> Result<(CostProfile, Currency), ValuationError> {
    let row: Option<(String, i32, String)> = sqlx::query_as(
        "SELECT cost_profile_id, cost_profile_version, currency FROM accounts \
         WHERE id = $1 AND owner_user_id = $2 AND status = 'ACTIVE' AND account_type = 'PAPER' \
         FOR UPDATE",
    )
    .bind(account_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (profile_id, version, currency_code) =
        row.ok_or_else(|| ValuationError::AccountUnavailable {
            account_id,
            detail: "no ACTIVE Paper account with this owner".to_owned(),
        })?;
    let profile = CostProfile::resolve(&profile_id)
        .map_err(|e| ValuationError::CostProfile(format!("account {account_id}: {e}")))?;
    if i64::from(version) != i64::from(profile.version) {
        return Err(ValuationError::CostProfile(format!(
            "account {account_id} was opened under {profile_id} version {version}, but this build ships version {}",
            profile.version
        )));
    }
    let currency = Currency::from_code(&currency_code).map_err(|e| {
        ValuationError::CostProfile(format!("account currency {currency_code:?}: {e}"))
    })?;
    Ok((profile, currency))
}

async fn account_cash(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    owner_user_id: Uuid,
    currency: Currency,
) -> Result<Money, ValuationError> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT (SELECT balance::text FROM cash_ledger \
                  WHERE account_id = $1 AND owner_user_id = $2 \
                  ORDER BY seq DESC LIMIT 1), \
                COALESCE(SUM(amount), 0)::text \
         FROM cash_ledger WHERE account_id = $1 AND owner_user_id = $2",
    )
    .bind(account_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (running, replayed) = row.ok_or_else(|| ValuationError::AccountUnavailable {
        account_id,
        detail: "no cash_ledger history".to_owned(),
    })?;
    let running = running.ok_or_else(|| ValuationError::AccountUnavailable {
        account_id,
        detail: "no cash_ledger history".to_owned(),
    })?;
    let parse = |value: &str| {
        Money::parse(value, currency).map_err(|e| ValuationError::AccountUnavailable {
            account_id,
            detail: format!("unreadable cash {value:?}: {e}"),
        })
    };
    let cash = parse(&running)?;
    let replayed = parse(&replayed)?;
    if cash != replayed {
        return Err(ValuationError::AccountUnavailable {
            account_id,
            detail: format!(
                "cash_ledger disagrees with itself -- running balance {}, events replay to {}",
                cash.amount(),
                replayed.amount()
            ),
        });
    }
    Ok(cash)
}

async fn account_positions(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    owner_user_id: Uuid,
) -> Result<BTreeMap<InstrumentId, Quantity>, ValuationError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT instrument_id, quantity::text FROM positions \
         WHERE account_id = $1 AND owner_user_id = $2",
    )
    .bind(account_id)
    .bind(owner_user_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut positions = BTreeMap::new();
    for (instrument_code, quantity_text) in rows {
        let instrument = InstrumentId::parse(&instrument_code).map_err(|e| {
            ValuationError::AccountUnavailable {
                account_id,
                detail: format!("unreadable position instrument {instrument_code:?}: {e}"),
            }
        })?;
        let whole = quantity_text.split('.').next().unwrap_or("0");
        let quantity = Quantity::parse(whole).map_err(|e| ValuationError::AccountUnavailable {
            account_id,
            detail: format!("unreadable position quantity {quantity_text:?}: {e}"),
        })?;
        if !quantity.is_zero() {
            positions.insert(instrument, quantity);
        }
    }
    Ok(positions)
}

fn session_closes(
    dataset_root: &Path,
    state: &LedgerState,
    date: TradingDate,
    canceled: &AtomicBool,
) -> Result<BTreeMap<InstrumentId, Price>, ValuationError> {
    let store = CurateStore::new(dataset_root.join("curated"));
    let year = date
        .as_naive_date()
        .format("%Y")
        .to_string()
        .parse::<i32>()
        .map_err(|e| ValuationError::Prices(format!("session year: {e}")))?;
    let mut closes = BTreeMap::new();
    for instrument in state.positions.keys() {
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ValuationError::Prices(
                "curated close read canceled during shutdown".to_owned(),
            ));
        }
        let path = store.bars_path(MARKET, &instrument.to_string(), year, CURATED_VERSION);
        if !path.exists() {
            return Err(ValuationError::MissingMark {
                instrument_id: instrument.clone(),
                date,
                detail: format!("curated file does not exist: {}", path.display()),
            });
        }
        let bars = read_bars(&path)
            .map_err(|e| ValuationError::Prices(format!("read {}: {e}", path.display())))?;
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ValuationError::Prices(
                "curated close read canceled during shutdown".to_owned(),
            ));
        }
        let Some(bar) = bars.iter().find(|bar| bar.trading_date == date) else {
            return Err(ValuationError::MissingMark {
                instrument_id: instrument.clone(),
                date,
                detail: "no bar for the requested date".to_owned(),
            });
        };
        if bar.market_close_ts.as_datetime() > Utc::now() {
            return Err(ValuationError::CloseNotYetAvailable {
                instrument_id: instrument.clone(),
                date,
                close_at: bar.market_close_ts.to_rfc3339(),
            });
        }
        closes.insert(instrument.clone(), bar.close);
    }
    Ok(closes)
}
