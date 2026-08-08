//! The twelve checks of design §6.13.
//!
//! Every check is a pure function of `(&RiskSnapshot, &RiskLimits)` returning
//! `Option<DenyReason>` — `None` passes. Purity is the point: nothing here
//! reads a clock or a database, so replaying a persisted snapshot reproduces
//! the decision exactly, which is what makes the restart clause of the
//! approved gatekeeper decision testable.
//!
//! Every `Unknown` input denies with `InputUnavailable`. §16 requires missing
//! or stale state to block, and `InputUnavailable` is kept distinct from the
//! policy reasons so an outage is never filed as a rejection.

use crate::decision::{Check, DenyReason};
use crate::limits::{RiskLimits, exceeds_basis_points, order_value};
use crate::snapshot::{
    Allowlisted, DataFreshness, IntentConflict, KillSwitch, MarketSession, Reconciliation,
    RiskSnapshot, Side, StrategyPromotion,
};

/// Runs one check. The `Check` enum and this function are the only two places
/// that need to know a check exists.
pub fn run(check: Check, snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    match check {
        Check::KillSwitch => kill_switch(snap),
        Check::MarketSession => market_session(snap),
        Check::DataFreshness => data_freshness(snap, limits),
        Check::StrategyPromotion => strategy_promotion(snap),
        Check::Reconciliation => reconciliation(snap),
        Check::InstrumentAllowlist => instrument_allowlist(snap),
        Check::SymbolMaxWeight => symbol_max_weight(snap, limits),
        Check::OrderMaxValue => order_max_value(snap, limits),
        Check::DailyOrderValue => daily_order_value(snap, limits),
        Check::DailyLoss => daily_loss(snap, limits),
        Check::AvailableFunds => available_funds(snap, limits),
        Check::DuplicateIntent => duplicate_intent(snap),
    }
}

/// 1. The system kill switch (FR-LIVE-006).
fn kill_switch(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.kill_switch {
        KillSwitch::Disengaged => None,
        KillSwitch::Engaged => Some(DenyReason::LiveKillSwitchEngaged),
        KillSwitch::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 2. Market and session state.
fn market_session(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.market_session {
        MarketSession::Open => None,
        MarketSession::Closed | MarketSession::Halted => Some(DenyReason::MarketSessionClosed),
        MarketSession::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 3. Data freshness (AT-08).
///
/// Age is compared inclusively: data exactly at the limit is still fresh. The
/// boundary is stated here because the alternative reading — "older than or
/// equal to blocks" — differs by one second and would be indistinguishable in
/// a test that only checked obviously-stale data.
///
/// Negative age means data timestamped in the future, which is a clock or
/// feed fault rather than very fresh data, so it denies as unavailable.
fn data_freshness(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    match snap.data_freshness {
        DataFreshness::Age(secs) if secs < 0 => Some(DenyReason::InputUnavailable),
        DataFreshness::Age(secs) if secs <= limits.max_data_age_secs => None,
        DataFreshness::Age(_) => Some(DenyReason::DataStale),
        DataFreshness::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 4. Strategy promotion state.
fn strategy_promotion(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.strategy_promotion {
        StrategyPromotion::LiveCandidate => None,
        StrategyPromotion::NotPromoted => Some(DenyReason::StrategyNotLiveCandidate),
        StrategyPromotion::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 5. Account reconciliation (FR-LIVE-004).
fn reconciliation(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.reconciliation {
        Reconciliation::Green => None,
        Reconciliation::NotGreen => Some(DenyReason::LiveReconciliationRequired),
        // A system that has just restarted has not reconciled yet and lands
        // here, which is precisely the case FR-LIVE-004 requires blocking.
        Reconciliation::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 6. Instrument allowlist.
fn instrument_allowlist(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.instrument_allowed {
        Allowlisted::Allowed => None,
        Allowlisted::NotAllowed => Some(DenyReason::InstrumentNotAllowed),
        Allowlisted::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// 7. Maximum weight per symbol.
///
/// Measured on the position that would exist AFTER this order fills, not the
/// one that exists now — a check against the current position would approve
/// an order that itself creates the breach. Only buys can increase
/// concentration, so a sell is not weight-checked.
fn symbol_max_weight(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    if snap.intent.side == Side::Sell {
        return None;
    }
    let Some(value) = intent_value(snap, limits) else {
        // A market buy has no price to value the resulting position with. It
        // cannot be shown to be within the limit, so it is not approved.
        return Some(DenyReason::InputUnavailable);
    };
    let Ok(resulting) = snap
        .account
        .position_value
        .amount()
        .checked_add(&value.amount())
    else {
        return Some(DenyReason::InputUnavailable);
    };
    let Ok(resulting_equity) = snap.account.equity.amount().checked_add(&value.amount()) else {
        return Some(DenyReason::InputUnavailable);
    };
    // A buy converts cash into position: total equity is unchanged, but if the
    // account is being valued before the cash leaves, the denominator must
    // include it. Using the larger of the two is the conservative direction
    // only if it makes the ratio smaller, so the pre-trade equity is used and
    // the post-trade equity is used only when equity was zero (a fresh
    // deposit-then-trade sequence), where the ratio would otherwise be
    // infinite for a legitimate first purchase.
    let denominator = if snap.account.equity.amount().is_zero() {
        resulting_equity
    } else {
        snap.account.equity.amount()
    };
    match exceeds_basis_points(&resulting, &denominator, limits.max_symbol_weight_bp) {
        Ok(true) => Some(DenyReason::RiskLimitExceeded),
        Ok(false) => None,
        Err(_) => Some(DenyReason::InputUnavailable),
    }
}

/// 8. Maximum value per order.
fn order_max_value(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    let Some(value) = intent_value(snap, limits) else {
        // A market order's value is unknown until it fills, so it cannot be
        // proven to be under the per-order limit. Denying is the only answer
        // consistent with §16; permitting it would leave the largest possible
        // order the least constrained one.
        return Some(DenyReason::InputUnavailable);
    };
    if value.amount() > limits.max_order_value.amount() {
        Some(DenyReason::RiskLimitExceeded)
    } else {
        None
    }
}

/// 9. Cumulative order value today.
fn daily_order_value(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    let Some(value) = intent_value(snap, limits) else {
        return Some(DenyReason::InputUnavailable);
    };
    let Ok(total) = snap
        .account
        .daily_order_value
        .amount()
        .checked_add(&value.amount())
    else {
        return Some(DenyReason::InputUnavailable);
    };
    if total > limits.max_daily_order_value.amount() {
        Some(DenyReason::RiskLimitExceeded)
    } else {
        None
    }
}

/// 10. Daily loss limit.
///
/// Checked against the loss already realised, not a projection: the order is
/// blocked once the account has lost enough today, whatever this particular
/// order would do. At the limit exactly, trading stops — a loss limit that
/// permits one more order at the limit is not a limit.
fn daily_loss(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    if snap.account.daily_loss.amount() >= limits.max_daily_loss.amount() {
        Some(DenyReason::RiskLimitExceeded)
    } else {
        None
    }
}

/// 11. Cash for a buy, or units for a sell.
///
/// A market buy is denied here for the same reason as check 8: its cost is
/// unknown, so sufficient cash cannot be demonstrated.
fn available_funds(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<DenyReason> {
    match snap.intent.side {
        Side::Sell => {
            // Selling more than is settled and unencumbered would either fail
            // at the broker or, worse, succeed and leave a short position in
            // an account that is not permitted to hold one.
            if snap.intent.quantity.amount() > snap.account.available_quantity.amount() {
                Some(DenyReason::RiskLimitExceeded)
            } else {
                None
            }
        }
        Side::Buy => {
            let Some(cost) = intent_value(snap, limits) else {
                return Some(DenyReason::InputUnavailable);
            };
            if cost.amount() > snap.account.available_cash.amount() {
                Some(DenyReason::RiskLimitExceeded)
            } else {
                None
            }
        }
    }
}

/// 12. Duplicate or conflicting intent (FR-LIVE-003).
fn duplicate_intent(snap: &RiskSnapshot) -> Option<DenyReason> {
    match snap.conflict {
        IntentConflict::None => None,
        IntentConflict::Conflicting => Some(DenyReason::DuplicateIntent),
        IntentConflict::Unknown => Some(DenyReason::InputUnavailable),
    }
}

/// Value of the order being asked about: `quantity * price`.
///
/// `None` when there is no limit price (a market order) or the multiplication
/// overflows. Every caller treats `None` as a denial rather than as zero,
/// which is why this returns `Option` and not a defaulted value.
fn intent_value(snap: &RiskSnapshot, limits: &RiskLimits) -> Option<domain::Money> {
    let price = snap.intent.price?;
    order_value(
        &snap.intent.quantity.amount(),
        &price.amount(),
        limits.currency(),
    )
    .ok()
}
