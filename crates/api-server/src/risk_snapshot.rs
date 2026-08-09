//! Building the [`RiskSnapshot`] the Risk Gateway actually evaluates.
//!
//! This module exists because the Live submission path did not have one. It
//! called `risk_gateway::testing::snapshot_all_green()` -- a test fixture whose
//! own documentation is *"A snapshot that passes all twelve checks"* -- and
//! overwrote exactly two fields, `instrument_id` and `correlation_id`, before
//! handing it to the gate. The order that then went to the broker was built
//! separately from the caller's real request.
//!
//! So the gate was asked about a 10-unit buy at 7,250 against a fabricated
//! account, approved that, and a different order was sent. Every value limit,
//! the side, the session, the freshness, the reconciliation state and the
//! limits themselves were the fixture's. The twelve checks ran and meant
//! nothing.
//!
//! # Why `Unknown` is the honest default
//!
//! Four inputs have no source wired yet: the market session, dataset
//! freshness, the strategy's promotion state, and the instrument allowlist.
//! Each is its own source-of-truth decision and none is invented here.
//! `checks.rs` is explicit -- *"Every `Unknown` input denies with
//! `InputUnavailable`. §16 requires missing inputs to deny"* -- so an unwired
//! input closes the gate instead of opening it. A Live order therefore cannot
//! be approved today, and that is the correct state: it could not be safely
//! approved before either, the system just did not say so.
//!
//! # Why a missing row is an error and not a zero
//!
//! [`AccountState`] is the one part with no `Unknown` variant, so there is no
//! way to spell "I could not read this" in the data. Substituting zeros would
//! be the same defect at smaller scale: zero cash happens to deny a buy, but a
//! zero equity DENOMINATOR does not deny the weight check, it corrupts it.
//! When the account has no equity snapshot yet, this refuses and names the
//! account.

use crate::error::{TenancyError, TenancyResult};
use crate::repos::reconciliation::{Readiness, ReconciliationRepo};
use domain::{Currency, Money, Quantity};
use risk_gateway::RiskLimits;
use risk_gateway::snapshot::{
    AccountState, Allowlisted, DataFreshness, IntentConflict, KillSwitch, MarketSession,
    OrderIntent, Reconciliation, RiskSnapshot, Side, StrategyPromotion,
};
use uuid::Uuid;

/// The ONE spelling of an order side this system accepts.
///
/// Callers get exactly two answers and no third. The previous code compared
/// `eq_ignore_ascii_case("SELL")` with a bare `else` arm, so every other value
/// -- a typo, a trailing space, an empty string -- became a BUY. Returning
/// `None` here is what lets the route answer 400 instead of reversing the
/// direction of somebody's order.
pub fn parse_side(raw: &str) -> Option<Side> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "BUY" => Some(Side::Buy),
        "SELL" => Some(Side::Sell),
        _ => None,
    }
}

/// The canonical wire spelling, for the text column the intents table keeps.
pub fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

/// The order as the caller expressed it, already parsed and validated.
///
/// Typed, not stringly. `side` arrives as a [`Side`] because the route rejects
/// anything that is not BUY or SELL, and `quantity` as a [`Quantity`] because
/// `Quantity::parse` refuses a fractional value -- which is what
/// `kis_client::mapping::OrderRequest` documents it wants: *"a fractional
/// quantity is a bug to surface, not round"*.
#[derive(Debug, Clone)]
pub struct GateOrder {
    pub intent_ref: String,
    pub account_id: Uuid,
    pub instrument_id: String,
    pub side: Side,
    pub quantity: Quantity,
    pub price: Option<domain::Price>,
    pub correlation_id: String,
}

/// Reads the account state the value checks divide and compare against.
///
/// One row per source, all keyed on `account_id`; `positions` and
/// `daily_equity` are shared tables, not Paper-only ones.
///
/// NOTE: `daily_equity.cash` is a stored column, and nothing in this
/// repository currently proves it agrees with `cash_ledger`, which is the
/// authority. That gap is real and is tracked separately -- it is not created
/// here, and using the stored value is strictly better than the fabricated one
/// this replaces.
async fn account_state(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    instrument_id: &str,
) -> TenancyResult<AccountState> {
    let unavailable = |what: &str| {
        TenancyError::InvalidState(format!(
            "risk inputs unavailable for account {account_id}: no {what}"
        ))
    };

    let equity_row: Option<(String, String)> = sqlx::query_as(
        "SELECT equity::text, cash::text FROM daily_equity \
         WHERE account_id = $1 ORDER BY trading_date DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(TenancyError::from_sqlx)?;
    let (equity, cash) = equity_row.ok_or_else(|| unavailable("daily_equity row"))?;

    // The previous close, for today's loss. Absent on an account's first day,
    // which is a zero loss rather than a missing input.
    let prev: Option<(String,)> = sqlx::query_as(
        "SELECT equity::text FROM daily_equity \
         WHERE account_id = $1 ORDER BY trading_date DESC OFFSET 1 LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let position: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT quantity::text, avg_price::text FROM positions \
         WHERE account_id = $1 AND instrument_id = $2",
    )
    .bind(account_id)
    .bind(instrument_id)
    .fetch_optional(pool)
    .await
    .map_err(TenancyError::from_sqlx)?;

    // Orders already placed today. Denied intents never reached the broker and
    // must not consume the daily budget; anything from the approval onward did.
    let placed: Option<(String,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(quantity * COALESCE(price, 0)), 0)::text \
         FROM order_intents \
         WHERE account_id = $1 AND created_at::date = CURRENT_DATE \
           AND state NOT IN ('DENIED', 'CLAIMED')",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let krw = |s: &str| -> TenancyResult<Money> {
        Money::parse(s, Currency::KRW)
            .map_err(|e| TenancyError::InvalidState(format!("unreadable money {s:?}: {e}")))
    };

    let equity_money = krw(&equity)?;
    let (qty, avg_price) = match position {
        Some((q, p)) => (q, p),
        None => ("0".to_string(), None),
    };
    let quantity = Quantity::parse(qty.split('.').next().unwrap_or("0"))
        .map_err(|e| TenancyError::InvalidState(format!("unreadable position quantity: {e}")))?;
    // Valued at the average cost this system holds, not a live mark: no market
    // data source is wired here, and inventing a price would be the same class
    // of defect this module exists to remove.
    let position_value = match avg_price {
        Some(p) => {
            let unit = domain::Price::parse(&p).map_err(|e| {
                TenancyError::InvalidState(format!("unreadable position price {p:?}: {e}"))
            })?;
            quantity
                .checked_mul_price(&unit, Currency::KRW)
                .map_err(|e| TenancyError::InvalidState(format!("position value overflows: {e}")))?
        }
        None => krw("0")?,
    };

    let daily_loss = match prev {
        Some((p,)) => {
            let before = krw(&p)?;
            // A profit is zero, not a negative loss, so the limit comparison
            // has exactly one meaning (see AccountState::daily_loss).
            before
                .checked_sub(&equity_money)
                .ok()
                .filter(|d| !d.amount().is_negative())
                .unwrap_or(krw("0")?)
        }
        None => krw("0")?,
    };

    Ok(AccountState {
        equity: equity_money,
        available_cash: krw(&cash)?,
        available_quantity: quantity,
        position_value,
        daily_order_value: krw(&placed.map(|(s,)| s).unwrap_or_else(|| "0".into()))?,
        daily_loss,
    })
}

/// The owner's configured limits, from the `risk_limits` table.
///
/// `RiskLimits::version` documents itself as *"Primary key of `risk_limits`;
/// recorded on every decision"* -- and until now nothing in this server read
/// that table. The submission path passed `risk_gateway::testing::limits()`,
/// so every value check was measured against a fixture's numbers and the
/// version stamped on the recorded decision named a limit set nobody had
/// configured.
///
/// An owner-scoped row wins over the global one (`owner_user_id IS NULL`), so
/// a per-owner policy can exist without a schema change. No row at all is a
/// typed refusal: there is no safe default for "how much may this account lose
/// today", and inventing one is how a limit stops being a decision somebody
/// made.
pub async fn limits_for(pool: &sqlx::PgPool, owner: Uuid) -> TenancyResult<RiskLimits> {
    let row: Option<(String, i32, String, String, String, i32)> = sqlx::query_as(
        "SELECT version, max_symbol_weight_bp, max_order_value::text, \
                max_daily_order_value::text, max_daily_loss::text, max_data_age_secs \
         FROM risk_limits \
         WHERE owner_user_id = $1 OR owner_user_id IS NULL \
         ORDER BY owner_user_id NULLS LAST LIMIT 1",
    )
    .bind(owner)
    .fetch_optional(pool)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let (version, weight_bp, order_value, daily_order_value, daily_loss, data_age) =
        row.ok_or_else(|| {
            TenancyError::InvalidState(format!(
                "no risk_limits row for owner {owner} and no global default; \
                 a Live order cannot be checked against limits nobody configured"
            ))
        })?;

    let krw = |s: &str| -> TenancyResult<Money> {
        Money::parse(s, Currency::KRW)
            .map_err(|e| TenancyError::InvalidState(format!("unreadable limit {s:?}: {e}")))
    };

    RiskLimits::new(
        version,
        u32::try_from(weight_bp).unwrap_or(0),
        krw(&order_value)?,
        krw(&daily_order_value)?,
        krw(&daily_loss)?,
        i64::from(data_age),
    )
    .map_err(|e| TenancyError::InvalidState(format!("configured risk limits are invalid: {e}")))
}

/// Builds the snapshot the gate evaluates for a real submission.
///
/// `kill_switch` and `reconciliation` are read from their repositories. The
/// four unwired inputs are `Unknown`, which denies. Nothing here is a fixture.
pub async fn for_submission(
    pool: &sqlx::PgPool,
    reconciliation: &ReconciliationRepo,
    connection_id: Option<Uuid>,
    kill_switch_engaged: Option<bool>,
    order: &GateOrder,
    now_secs: i64,
) -> TenancyResult<RiskSnapshot> {
    let account = account_state(pool, order.account_id, &order.instrument_id).await?;

    // An unreadable kill switch is `Unknown`, which denies. §16 is fail-closed
    // and "we could not tell" is not permission.
    let kill_switch = match kill_switch_engaged {
        Some(true) => KillSwitch::Engaged,
        Some(false) => KillSwitch::Disengaged,
        None => KillSwitch::Unknown,
    };

    // The same three-way split `repos::reconciliation::gate_input` already
    // makes for the kis-client gate, and for the reasons documented there:
    // a run still in flight is NotGreen rather than Unknown (its answer is
    // simply not in yet), while NEVER having reconciled is a genuine absence
    // of information and denies as Unknown. A readiness that cannot be read
    // at all lands in the same place -- FR-LIVE-004 blocks new orders when the
    // relationship to the broker is unestablished.
    let reconciliation = match reconciliation.readiness(connection_id).await {
        Ok(Readiness::Ready { .. }) => Reconciliation::Green,
        Ok(Readiness::Blocked { .. } | Readiness::Running { .. }) => Reconciliation::NotGreen,
        Ok(Readiness::NeverReconciled) | Err(_) => Reconciliation::Unknown,
    };

    Ok(RiskSnapshot {
        intent: OrderIntent {
            intent_ref: order.intent_ref.clone(),
            account_id: order.account_id.to_string(),
            instrument_id: order.instrument_id.clone(),
            side: order.side,
            quantity: order.quantity,
            price: order.price,
        },
        correlation_id: order.correlation_id.clone(),
        evaluated_at_secs: now_secs,
        kill_switch,
        // --- not wired; each denies until it has a source of truth ----------
        // A market calendar service. Until then an order cannot demonstrate it
        // was placed in a session that accepts orders.
        market_session: MarketSession::Unknown,
        // Dataset staleness for the instrument being traded.
        data_freshness: DataFreshness::Unknown,
        // Whether the strategy behind this order is promoted to Live.
        strategy_promotion: StrategyPromotion::Unknown,
        // The owner's instrument allowlist. An empty allowlist must deny
        // everything, so "not read" and "empty" agree here.
        instrument_allowed: Allowlisted::Unknown,
        // --------------------------------------------------------------------
        reconciliation,
        account,
        // Open-order conflict detection is not wired. Submitting while blind to
        // what is already working is how an account ends up double-filled, so
        // this denies rather than assuming None.
        conflict: IntentConflict::Unknown,
    })
}
