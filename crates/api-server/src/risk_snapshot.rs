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
//! Two inputs -- `strategy_promotion` and `instrument_allowed` -- now read
//! from this repository's existing sources of truth (`active_binding`, the
//! fixed KRX ETF v1 universe). The remaining two, the market session and
//! dataset freshness, have no source wired yet: each needs a market-calendar
//! service and a dataset-staleness check that do not exist in this repo, and
//! neither is invented here. `checks.rs` is explicit -- *"Every `Unknown`
//! input denies with `InputUnavailable`. §16 requires missing inputs to
//! deny"* -- so an unwired input closes the gate instead of opening it. A
//! Live order therefore still cannot be approved today, and that remains the
//! correct state: it could not be safely approved before either, the system
//! just did not say so.
//!
//! # Why a missing row is an error and not a zero
//!
//! [`AccountState`] is the one part with no `Unknown` variant, so there is no
//! way to spell "I could not read this" in the data. Substituting zeros would
//! be the same defect at smaller scale: zero cash happens to deny a buy, but a
//! zero equity DENOMINATOR does not deny the weight check, it corrupts it.
//! When the account has no equity snapshot yet, this refuses and names the
//! account.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use crate::http::validation::in_fixed_universe;
use crate::repos::accounts::AccountRepo;
use crate::repos::reconciliation::{Readiness, ReconciliationRepo};
use auth::entitlement::Actor;
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
/// Every number comes from the AUTHORITY, never from a snapshot of it.
/// `repos::accounts` states the rule this system runs on: *"current cash is
/// never cached here -- it is always derived by replaying `cash_ledger`"*. An
/// earlier draft of this function read `daily_equity.cash`, a stored column
/// that nothing in this repository writes outside tests, and which is exactly
/// the second source of truth the ledger contract exists to prevent. The
/// gate's affordability check would have depended on a number nobody
/// maintains.
///
/// Every read runs inside an ACTOR transaction: `cash_ledger`, `positions` and
/// `daily_equity` are under FORCE RLS (migration 0010), so a query on a bare
/// pool sees zero rows and this would report "unavailable" for every account
/// that exists. A test caught that; a production Live order would have failed
/// the same way.
async fn account_state(
    pool: &sqlx::PgPool,
    actor: &Actor,
    account_id: Uuid,
    instrument_id: &str,
) -> TenancyResult<AccountState> {
    let mut tx = begin_actor_tx(pool, actor).await?;
    let unavailable = |what: &str| {
        TenancyError::InvalidState(format!(
            "risk inputs unavailable for account {account_id}: {what}"
        ))
    };
    let krw = |s: &str| -> TenancyResult<Money> {
        Money::parse(s, Currency::KRW)
            .map_err(|e| TenancyError::InvalidState(format!("unreadable money {s:?}: {e}")))
    };

    // Cash, twice, from one table by two routes.
    //
    // `balance` is the running total the writer maintained; `SUM(amount)` is
    // that same total recomputed from the events themselves. They agree or the
    // ledger contradicts itself, and an account whose own cash cannot be
    // agreed on must not have an order approved against it.
    //
    // This is the reconciliation the gate check NAMED "ledger-reconciliation"
    // does not perform: that suite runs `portfolio-model` in memory and never
    // touches Postgres, so no test in this repository compares a stored cash
    // figure against the events it is supposed to summarise.
    let cash_row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT (SELECT balance::text FROM cash_ledger WHERE account_id = $1 \
                  ORDER BY seq DESC LIMIT 1), \
                COALESCE(SUM(amount), 0)::text \
         FROM cash_ledger WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let (running, replayed) = cash_row.ok_or_else(|| unavailable("no cash_ledger history"))?;
    let running = running.ok_or_else(|| unavailable("no cash_ledger history"))?;

    let cash = krw(&running)?;
    let replayed_cash = krw(&replayed)?;
    if cash != replayed_cash {
        return Err(unavailable(&format!(
            "cash_ledger disagrees with itself -- running balance {}, events replay to {}",
            cash.amount(),
            replayed_cash.amount()
        )));
    }

    // Positions at the cost basis this system holds. No market data source is
    // wired here, and inventing a mark would be the same class of defect this
    // module exists to remove. Cost basis is conservative for the weight check
    // and is stated rather than implied.
    let positions: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT instrument_id, quantity::text, avg_price::text FROM positions \
         WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let mut positions_value = krw("0")?;
    let mut available_quantity = Quantity::parse("0").expect("zero parses");
    let mut position_value = krw("0")?;
    for (id, qty, avg) in &positions {
        let quantity = Quantity::parse(qty.split('.').next().unwrap_or("0")).map_err(|e| {
            TenancyError::InvalidState(format!("unreadable position quantity {qty:?}: {e}"))
        })?;
        let value = match avg {
            Some(p) => {
                let unit = domain::Price::parse(p).map_err(|e| {
                    TenancyError::InvalidState(format!("unreadable position price {p:?}: {e}"))
                })?;
                quantity.checked_mul_price(&unit, Currency::KRW).map_err(|e| {
                    TenancyError::InvalidState(format!("position value overflows: {e}"))
                })?
            }
            None => krw("0")?,
        };
        positions_value = positions_value
            .checked_add(&value)
            .map_err(|e| TenancyError::InvalidState(format!("equity overflows: {e}")))?;
        if id == instrument_id {
            available_quantity = quantity;
            position_value = value;
        }
    }

    let equity = cash
        .checked_add(&positions_value)
        .map_err(|e| TenancyError::InvalidState(format!("equity overflows: {e}")))?;

    // Orders already placed today, counted from the APPROVAL onward.
    //
    // The states are `kis_client::order_state::OrderIntentState`'s own strings
    // and migration 0019 constrains the column to them. Two are excluded and
    // for different reasons: `DENIED` never reached the broker, and
    // `INTENT_CREATED` is an intent that was claimed and has not been through
    // the gate -- a crashed submission leaves one, and charging it against the
    // daily budget would shrink tomorrow's headroom for an order that never
    // existed.
    //
    // An earlier draft of this query excluded 'CLAIMED', which is not a state
    // this system has. It matched nothing, so INTENT_CREATED rows were counted
    // -- the opposite of what the comment above it claimed.
    let placed: Option<(String,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(quantity * COALESCE(price, 0)), 0)::text \
         FROM order_intents \
         WHERE account_id = $1 AND created_at::date = CURRENT_DATE \
           AND state NOT IN ('DENIED', 'INTENT_CREATED')",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(TenancyError::from_sqlx)?;

    // Today's loss needs a start-of-day baseline, and the only one this schema
    // carries is a prior `daily_equity` row.
    //
    // An account whose ledger begins today has no prior day, and zero is then
    // the true answer rather than a filler. An account with history but no
    // snapshot has a genuinely missing baseline, and this refuses: a silent
    // zero would mean check 10, the daily-loss limit, could never fire -- the
    // same shape as the fee-summing check in this codebase that compared a
    // timestamp against a date and was false forever.
    let prior: Option<(String,)> = sqlx::query_as(
        "SELECT equity::text FROM daily_equity \
         WHERE account_id = $1 AND trading_date < CURRENT_DATE \
         ORDER BY trading_date DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(TenancyError::from_sqlx)?;

    let daily_loss = match prior {
        Some((p,)) => {
            let before = krw(&p)?;
            // A profit is zero, not a negative loss, so the comparison against
            // the limit has exactly one meaning (see AccountState::daily_loss).
            match before.checked_sub(&equity) {
                Ok(d) if !d.amount().is_negative() => d,
                _ => krw("0")?,
            }
        }
        None => {
            let older: Option<(bool,)> = sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM cash_ledger \
                 WHERE account_id = $1 AND ts::date < CURRENT_DATE)",
            )
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if older.map(|(b,)| b).unwrap_or(false) {
                return Err(unavailable(
                    "no start-of-day equity baseline, so today's loss cannot be measured",
                ));
            }
            krw("0")?
        }
    };

    Ok(AccountState {
        equity,
        available_cash: cash,
        available_quantity,
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
pub async fn limits_for(
    pool: &sqlx::PgPool,
    actor: &Actor,
    owner: Uuid,
) -> TenancyResult<RiskLimits> {
    let mut tx = begin_actor_tx(pool, actor).await?;
    let row: Option<(String, i32, String, String, String, i32)> = sqlx::query_as(
        "SELECT version, max_symbol_weight_bp, max_order_value::text, \
                max_daily_order_value::text, max_daily_loss::text, max_data_age_secs \
         FROM risk_limits \
         WHERE owner_user_id = $1 OR owner_user_id IS NULL \
         ORDER BY owner_user_id NULLS LAST LIMIT 1",
    )
    .bind(owner)
    .fetch_optional(&mut *tx)
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
    actor: &Actor,
    reconciliation: &ReconciliationRepo,
    connection_id: Option<Uuid>,
    kill_switch_engaged: Option<bool>,
    order: &GateOrder,
    now_secs: i64,
) -> TenancyResult<RiskSnapshot> {
    let account = account_state(pool, actor, order.account_id, &order.instrument_id).await?;

    // A bound, ACTIVE strategy is this account's evidence of promotion to
    // Live -- `account_strategy_bindings` already carries exactly that fact
    // (`unbound_at IS NULL`), and it is scoped by the same actor transaction
    // every other read here uses. An account with no active binding has
    // nothing behind its order but a human typing into a form, which is
    // `NotPromoted`, not `Unknown` -- the lookup SUCCEEDED and answered "no".
    let strategy_promotion = match AccountRepo::new(pool.clone())
        .active_binding(actor, order.account_id)
        .await
    {
        Ok(Some(_)) => StrategyPromotion::LiveCandidate,
        Ok(None) => StrategyPromotion::NotPromoted,
        Err(_) => StrategyPromotion::Unknown,
    };

    // The fixed KRX ETF v1 universe (Todo 12) is a compiled-in constant, the
    // same one `backtests.rs` checks a benchmark against -- not a fresh read
    // of the manifest file, which would need its own IO error handling for a
    // check that never actually goes stale between builds. There is
    // therefore no "cannot be read" case for this input in this build; a
    // future release that makes the universe a runtime-editable per-owner
    // allowlist gets to reintroduce `Unknown` for its own IO failures.
    let instrument_allowed = if in_fixed_universe(&order.instrument_id) {
        Allowlisted::Allowed
    } else {
        Allowlisted::NotAllowed
    };

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
        strategy_promotion,
        instrument_allowed,
        // --------------------------------------------------------------------
        reconciliation,
        account,
        // Open-order conflict detection is not wired. Submitting while blind to
        // what is already working is how an account ends up double-filled, so
        // this denies rather than assuming None.
        conflict: IntentConflict::Unknown,
    })
}
