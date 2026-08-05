//! Versioned adjusted / total-return series for signals (Todo 10).
//!
//! Design §9.3/requirements §9.2: **signals** may use adjusted or total-return
//! price series; **execution always uses the raw open**. This module builds
//! the adjusted series used by research:
//!
//! - `split` series: back-adjusts prices BEFORE each split ex-date by
//!   `1/split_factor`, so the series is continuous with post-split prices
//!   (pre-split close 10300 == 2 x post-split 5150 — value preserving);
//! - `total_return` series: additionally reinvests each cash dividend at the
//!   ex-date close: prices before the ex-date are scaled by
//!   `(close_ex + amount) / close_ex`.
//!
//! Point-in-time: [`adjusted_series`] applies EXACTLY the actions it is
//! given — the caller passes `visible_actions(as_of)` so an announcement is
//! never used before `announced_at`. The versioned files on disk are built
//! with the actions announced by curation time (future-announced actions are
//! rejected at curation).
//!
//! Determinism: all arithmetic is fixed-point. Cumulative factors are rounded
//! to [`FACTOR_SCALE`] (8 dp, half-even) after each multiplication; adjusted
//! prices are rounded to the canonical [`PRICE_SCALE`] (4 dp, half-even).
//! Volume/trading value are NEVER adjusted (reported provider units).

use std::collections::BTreeMap;

use domain::{
    BatchId, ContentHash, Currency, FixedPoint, InstrumentId, Price, TradingDate, UtcTimestamp,
};

use super::CurateError;
use super::actions::{CorporateAction, CorporateActionType};
use super::schema::{CuratedBar, FACTOR_SCALE, PRICE_SCALE};

/// Which adjustment the series carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentKind {
    /// Split-adjusted (price-return) series.
    Split,
    /// Split + dividend reinvestment (total-return) series.
    TotalReturn,
}

impl AdjustmentKind {
    /// The stable wire name (`split` | `total_return`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::TotalReturn => "total_return",
        }
    }

    /// Parses the stable wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "split" => Some(Self::Split),
            "total_return" => Some(Self::TotalReturn),
            _ => None,
        }
    }
}

impl std::fmt::Display for AdjustmentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One adjusted bar row (the curated adjusted-bars table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustmentBar {
    pub instrument_id: InstrumentId,
    pub trading_date: TradingDate,
    pub market_open_ts: UtcTimestamp,
    pub market_close_ts: UtcTimestamp,
    /// Adjusted prices (all four scaled by the same cumulative factor).
    pub open: Price,
    pub high: Price,
    pub low: Price,
    pub close: Price,
    /// Reported volume — never adjusted (documented).
    pub volume: i64,
    pub trading_value: Option<i64>,
    pub adjustment_kind: AdjustmentKind,
    /// Cumulative adjustment factor for this date (1.0 = no adjustment).
    pub adjustment_factor: FixedPoint,
    /// Provenance: the actions whose factor applies (`split:2:1:ex=...;
    /// dividend:150.0000:ex=...:pay=...`; empty when no action applies).
    pub adjustment_events: String,
    pub currency: Currency,
    pub source: String,
    pub ingested_at: UtcTimestamp,
    pub batch_id: BatchId,
    pub raw_hash: ContentHash,
}

/// Both adjusted series for one dataset version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustedSeries {
    /// Split-adjusted prices (signals, price-return basis).
    pub split: Vec<AdjustmentBar>,
    /// Total-return-adjusted prices (signals, total-return basis).
    pub total_return: Vec<AdjustmentBar>,
}

/// Builds the split- and total-return-adjusted series for the given bars and
/// corporate actions. Point-in-time by contract: only the actions passed in
/// are applied (see [`crate::curate::actions::visible_actions`]).
pub fn adjusted_series(
    bars: &[CuratedBar],
    actions: &[CorporateAction],
) -> Result<AdjustedSeries, CurateError> {
    // Group actions per instrument, sorted by ex-date ascending.
    let mut by_instrument: BTreeMap<InstrumentId, Vec<&CorporateAction>> = BTreeMap::new();
    for action in actions {
        by_instrument
            .entry(action.instrument_id.clone())
            .or_default()
            .push(action);
    }
    for list in by_instrument.values_mut() {
        list.sort_by_key(|a| (a.ex_date, a.announced_at));
    }

    // Raw close on each ex-date (post-split basis) for dividend factors.
    let close_on: BTreeMap<(InstrumentId, TradingDate), Price> = bars
        .iter()
        .map(|b| ((b.instrument_id.clone(), b.trading_date), b.close))
        .collect();

    let mut split_rows = Vec::with_capacity(bars.len());
    let mut total_return_rows = Vec::with_capacity(bars.len());

    // Bars are sorted by (instrument, date) from the curator; verify and
    // reuse that ordering so the output is deterministic.
    let mut bars: Vec<&CuratedBar> = bars.iter().collect();
    bars.sort_by_key(|b| (b.instrument_id.clone(), b.trading_date));

    for bar in bars {
        let instrument_actions = by_instrument.get(&bar.instrument_id).cloned().unwrap_or_default();
        let (split_factor, split_events) =
            cumulative_split_factor(&bar, &instrument_actions)?;
        let (tr_factor, tr_events) = cumulative_total_return_factor(
            &bar,
            &instrument_actions,
            &close_on,
            &split_factor,
            &split_events,
        )?;

        split_rows.push(adjusted_bar(bar, AdjustmentKind::Split, &split_factor, &split_events)?);
        total_return_rows.push(adjusted_bar(
            bar,
            AdjustmentKind::TotalReturn,
            &tr_factor,
            &tr_events,
        )?);
    }

    Ok(AdjustedSeries {
        split: split_rows,
        total_return: total_return_rows,
    })
}

/// The cumulative split back-adjustment factor for `bar`: the product of
/// `1/split_factor` over every split with `ex_date > bar.trading_date`.
fn cumulative_split_factor(
    bar: &CuratedBar,
    actions: &[&CorporateAction],
) -> Result<(FixedPoint, String), CurateError> {
    let mut factor = FixedPoint::parse("1").expect("one");
    let mut events = Vec::new();
    for action in actions {
        if action.event_type != CorporateActionType::Split {
            continue;
        }
        if action.ex_date <= bar.trading_date {
            continue;
        }
        let split_factor = action.split_factor.as_ref().ok_or_else(|| {
            CurateError::InvalidSplit {
                instrument: action.instrument_id.to_string(),
                detail: "split record without split_factor".to_owned(),
            }
        })?;
        let back = back_adjust_factor(split_factor)?;
        factor = factor.checked_mul(&back)?.with_scale(FACTOR_SCALE)?;
        events.push(format!(
            "split:{}:ex={}",
            action.ratio.as_deref().unwrap_or(&split_factor.to_string()),
            action.ex_date.to_iso()
        ));
    }
    events.sort_unstable();
    Ok((factor, events.join(";")))
}

/// `1 / split_factor` at FACTOR_SCALE (checked: factors must be > 1).
fn back_adjust_factor(split_factor: &FixedPoint) -> Result<FixedPoint, CurateError> {
    if *split_factor <= FixedPoint::parse("1").expect("one") {
        return Err(CurateError::InvalidSplit {
            instrument: "-".to_owned(),
            detail: format!("split_factor must be > 1, got {split_factor}"),
        });
    }
    Ok(FixedPoint::parse("1")
        .expect("one")
        .checked_div(split_factor, FACTOR_SCALE)?)
}

/// The cumulative total-return factor for `bar`: the split factor times the
/// dividend reinvestment factor `(close_ex + amount) / close_ex` over every
/// dividend with `ex_date > bar.trading_date` (reinvested at the ex-date
/// close, post-split basis). The event provenance includes the split events.
fn cumulative_total_return_factor(
    bar: &CuratedBar,
    actions: &[&CorporateAction],
    close_on: &BTreeMap<(InstrumentId, TradingDate), Price>,
    split_factor: &FixedPoint,
    split_events: &str,
) -> Result<(FixedPoint, String), CurateError> {
    let mut factor = *split_factor;
    let mut events: Vec<String> = if split_events.is_empty() {
        Vec::new()
    } else {
        split_events.split(';').map(str::to_owned).collect()
    };
    for action in actions {
        if action.event_type != CorporateActionType::CashDividend {
            continue;
        }
        if action.ex_date <= bar.trading_date {
            continue;
        }
        let amount = action.amount_per_share.as_ref().ok_or_else(|| {
            CurateError::InvalidDividend {
                instrument: action.instrument_id.to_string(),
                detail: "dividend record without amount_per_share".to_owned(),
            }
        })?;
        let close_ex = close_on
            .get(&(bar.instrument_id.clone(), action.ex_date))
            .ok_or_else(|| CurateError::InvalidDividend {
                instrument: action.instrument_id.to_string(),
                detail: format!(
                    "no raw close on ex-date {} for the reinvestment factor",
                    action.ex_date.to_iso()
                ),
            })?;
        // (close_ex + amount) / close_ex at FACTOR_SCALE.
        let numerator = close_ex
            .amount()
            .checked_add(&amount.amount())?;
        let reinvest = numerator
            .checked_div(&close_ex.amount(), FACTOR_SCALE)?;
        factor = factor.checked_mul(&reinvest)?.with_scale(FACTOR_SCALE)?;
        events.push(format!(
            "dividend:{}:ex={}:pay={}",
            amount.as_decimal_string(),
            action.ex_date.to_iso(),
            action
                .pay_date
                .map(|d| d.to_iso())
                .unwrap_or_else(|| "none".to_owned())
        ));
    }
    events.sort_unstable();
    Ok((factor, events.join(";")))
}

/// Applies a cumulative factor to one raw bar (fixed-point, deterministic).
fn adjusted_bar(
    bar: &CuratedBar,
    kind: AdjustmentKind,
    factor: &FixedPoint,
    events: &str,
) -> Result<AdjustmentBar, CurateError> {
    let scale_price = |p: &Price| -> Result<Price, CurateError> {
        Ok(Price::from_fixed(p.amount().checked_mul(factor)?.with_scale(PRICE_SCALE)?)?)
    };
    Ok(AdjustmentBar {
        instrument_id: bar.instrument_id.clone(),
        trading_date: bar.trading_date,
        market_open_ts: bar.market_open_ts,
        market_close_ts: bar.market_close_ts,
        open: scale_price(&bar.open)?,
        high: scale_price(&bar.high)?,
        low: scale_price(&bar.low)?,
        close: scale_price(&bar.close)?,
        volume: bar.volume,
        trading_value: bar.trading_value,
        adjustment_kind: kind,
        adjustment_factor: *factor,
        adjustment_events: events.to_owned(),
        currency: bar.currency,
        source: bar.source.clone(),
        ingested_at: bar.ingested_at,
        batch_id: bar.batch_id,
        raw_hash: bar.raw_hash.clone(),
    })
}
