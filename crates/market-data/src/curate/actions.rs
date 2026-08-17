//! Corporate-action records, point-in-time visibility, and capability (Todo 10).
//!
//! Requirements §8.2 기업행사 fields: `instrument_id, event_type, ex_date,
//! record_date, pay_date, ratio_or_amount, currency, announced_at, source`.
//! Point-in-time rule (§8.3): an action is visible to a query only once its
//! source observation is available.  Some primary feeds (including KIS KSD)
//! publish event dates without an announcement timestamp; those rows carry
//! `available_at` and deliberately leave `announced_at` absent rather than
//! manufacturing an announcement instant from ingestion time.
//! Price policy (§9.2): splits adjust holdings on the ex-date preserving
//! total value; dividends credit cash on the configured pay-date (never the
//! ex-date). A dataset version is `TOTAL_RETURN_CAPABLE` only when every
//! dividend carries complete pay-date data.

use domain::{
    ContentHash, Currency, FixedPoint, InstrumentId, Money, Price, Quantity, TradingDate,
    UtcTimestamp,
};
use serde::{Deserialize, Serialize};

use super::Capability;
use super::CurateError;

/// The two corporate-action classes the MVP curates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorporateActionType {
    /// A stock split: `split_factor` new shares per 1 old share (2:1 -> 2.0).
    Split,
    /// A cash dividend: `amount_per_share` credited on `pay_date`.
    CashDividend,
}

impl CorporateActionType {
    /// The stable wire name (`split` | `cash_dividend`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::CashDividend => "cash_dividend",
        }
    }

    /// Parses the stable wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "split" => Some(Self::Split),
            "cash_dividend" => Some(Self::CashDividend),
            _ => None,
        }
    }
}

impl std::fmt::Display for CorporateActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One curated corporate-action record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorporateAction {
    pub instrument_id: InstrumentId,
    pub event_type: CorporateActionType,
    /// The first date the action affects the market (holdings/price basis).
    pub ex_date: TradingDate,
    /// Optional record date (holder-of-record cut-off).
    pub record_date: Option<TradingDate>,
    /// The configured cash-credit date for dividends (never derived from the
    /// ex-date — requirements §9.2 "지급일 또는 정의된 모델").
    pub pay_date: Option<TradingDate>,
    /// Human-readable ratio, e.g. `2:1` (splits).
    pub ratio: Option<String>,
    /// New shares per old share (splits; must be > 1).
    pub split_factor: Option<FixedPoint>,
    /// Cash amount per share (dividends).
    pub amount_per_share: Option<Money>,
    /// Withholding rate in percent (e.g. `15.4`), informational.
    pub tax_withholding_pct: Option<FixedPoint>,
    /// The dividend/settlement currency.
    pub currency: Currency,
    /// The announcement instant, when the source actually supplies one.
    /// This is never inferred from `available_at`.
    pub announced_at: Option<UtcTimestamp>,
    /// The first instant at which this source observation was available to the
    /// curation pipeline.  For KIS KSD rows this is the verified raw response
    /// retrieval instant.
    pub available_at: UtcTimestamp,
    /// Provenance source label (e.g. `krx`).
    pub source: String,
    pub batch_id: domain::BatchId,
    /// SHA-256 of the raw corporate-actions file this record came from.
    pub raw_hash: ContentHash,
    pub ingested_at: UtcTimestamp,
}

/// The actions of a dataset visible as-of `as_of`: only records available on
/// or before `as_of`, and (when supplied) announced on or before `as_of`
/// (requirements §8.3 point-in-time; no look-ahead).
pub fn visible_actions(actions: &[CorporateAction], as_of: UtcTimestamp) -> Vec<CorporateAction> {
    let mut visible: Vec<CorporateAction> = actions
        .iter()
        .filter(|a| {
            a.available_at <= as_of
                && a.announced_at
                    .is_none_or(|announced_at| announced_at <= as_of)
        })
        .cloned()
        .collect();
    visible.sort_by_key(|a| {
        (
            a.ex_date,
            a.announced_at.unwrap_or(a.available_at),
            a.instrument_id.clone(),
        )
    });
    visible
}

/// The dataset capability flag (plan: explicit
/// `PRICE_RETURN_ONLY | TOTAL_RETURN_CAPABLE` per version).
///
/// Total-return capability requires COMPLETE dividend pay-date data: every
/// cash dividend must carry its configured pay-date. A dataset with no
/// dividends is vacuously complete; a dividend without a pay-date caps the
/// version at price returns (design §9.3: "총수익률 기준" requires the full
/// cash-flow schedule).
pub fn dataset_capability(actions: &[CorporateAction]) -> Capability {
    let incomplete = actions
        .iter()
        .any(|a| a.event_type == CorporateActionType::CashDividend && a.pay_date.is_none());
    if incomplete {
        Capability::PriceReturnOnly
    } else {
        Capability::TotalReturnCapable
    }
}

/// Split semantics (requirements §9.2: 액면분할은 포지션 수량과 가격 기준을
/// 조정한다 — value-preserving on the ex-date).
pub struct SplitAdjustment;

impl SplitAdjustment {
    /// The back-adjustment factor for prices BEFORE the ex-date: `1/factor`
    /// at [`crate::curate::schema::FACTOR_SCALE`] precision (2:1 -> 0.5).
    pub fn back_adjust_factor(split_factor: &FixedPoint) -> Result<FixedPoint, CurateError> {
        if *split_factor <= FixedPoint::parse("1").expect("one") {
            return Err(CurateError::InvalidSplit {
                instrument: "-".to_owned(),
                detail: format!("split_factor must be > 1, got {split_factor}"),
            });
        }
        Ok(FixedPoint::parse("1")
            .expect("one")
            .checked_div(split_factor, super::schema::FACTOR_SCALE)?)
    }

    /// Ex-date holding adjustment: `quantity * split_factor` (100 shares at a
    /// 2:1 split -> 200 shares). Fractional results are a typed error.
    pub fn apply_to_holdings(
        quantity: &Quantity,
        split_factor: &FixedPoint,
    ) -> Result<Quantity, CurateError> {
        Quantity::from_fixed(quantity.amount().checked_mul(split_factor)?).map_err(Into::into)
    }

    /// Value preservation: `pre_qty * pre_price == post_qty * post_price`
    /// (100 x 10300 == 200 x 5150 for a 2:1 split on the ex-date).
    pub fn value_preserved(
        pre_qty: &Quantity,
        pre_price: &Price,
        post_qty: &Quantity,
        post_price: &Price,
        currency: Currency,
    ) -> Result<bool, CurateError> {
        let before = pre_qty.checked_mul_price(pre_price, currency)?;
        let after = post_qty.checked_mul_price(post_price, currency)?;
        Ok(before == after)
    }
}

/// Dividend cash-credit semantics (requirements §9.2: 배당은 지급일 또는 정의된
/// 모델에 따라 현금 장부에 반영한다 — credited on the configured pay-date).
pub struct DividendCredit;

impl DividendCredit {
    /// The configured cash-credit date: the pay-date, never the ex-date.
    pub fn credit_date(action: &CorporateAction) -> Result<TradingDate, CurateError> {
        action.pay_date.ok_or_else(|| CurateError::InvalidDividend {
            instrument: action.instrument_id.to_string(),
            detail: "dividend has no pay_date; no configured credit date".to_owned(),
        })
    }

    /// Gross cash credit: `quantity * amount_per_share` (200 shares x 150.00
    /// KRW = 30,000.00 KRW). Withholding/netting is a ledger policy (Todo 18)
    /// — the curated record only carries `tax_withholding_pct` as data.
    pub fn gross_credit(
        quantity: &Quantity,
        amount_per_share: &Money,
    ) -> Result<Money, CurateError> {
        Ok(Money::from_fixed(
            quantity.amount().checked_mul(&amount_per_share.amount())?,
            amount_per_share.currency(),
        )?)
    }
}
