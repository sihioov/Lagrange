//! Structured reason codes with localized (ko/en) text (FR-SEL-005: "추천
//! 근거를 설명 가능하게 저장"). Every selection and every exclusion carries
//! a [`Reason`]: a stable machine-readable [`ReasonCode`], sorted params, and
//! deterministic Korean + English text derived from the code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The documented reason taxonomy of the selector. Codes are stable wire
/// values (SCREAMING_SNAKE_CASE) so downstream systems can branch on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCode {
    /// The instrument ranked within the strategy's top N.
    SelectedTopN,
    /// The instrument was excluded because a mandatory factor is NULL.
    ExcludedMandatoryFactorNull,
    /// The instrument ranked beyond the top N (no target weight).
    NotSelectedBeyondTopN,
    /// The instrument's target weight was capped at the per-instrument max.
    WeightCappedAtMax,
    /// Weight-rounding residue was allocated to cash (never silently dropped).
    WeightRoundingResidueToCash,
    /// No eligible instrument: the portfolio is held fully in cash.
    AllCashNoEligible,
    /// The portfolio maintains the declared cash floor.
    CashFloorApplied,
}

/// One structured evidence item: code + params + localized text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub code: ReasonCode,
    /// Canonically sorted interpolation params (deterministic serialization).
    pub params: BTreeMap<String, String>,
    /// Korean text ("국문").
    pub text_ko: String,
    /// English text.
    pub text_en: String,
}

impl Reason {
    /// Builds the reason with both localizations derived from `code`.
    pub fn new(code: ReasonCode, params: BTreeMap<String, String>) -> Self {
        let (text_ko, text_en) = localize(code, &params);
        Self {
            code,
            params,
            text_ko,
            text_en,
        }
    }
}

fn param<'a>(params: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    params.get(key).map(String::as_str).unwrap_or("?")
}

/// The deterministic ko/en text of a code for the given params.
pub fn localize(code: ReasonCode, params: &BTreeMap<String, String>) -> (String, String) {
    match code {
        ReasonCode::SelectedTopN => (
            format!("상위 {}개 이내 선정 (순위 {})", param(params, "top_n"), param(params, "rank")),
            format!("Ranked {} within top {}", param(params, "rank"), param(params, "top_n")),
        ),
        ReasonCode::ExcludedMandatoryFactorNull => (
            format!("필수 팩터 {} 결측(NULL)으로 제외", param(params, "factor")),
            format!("Excluded: mandatory factor {} is NULL", param(params, "factor")),
        ),
        ReasonCode::NotSelectedBeyondTopN => (
            format!("순위 {} — 상위 {} 밖", param(params, "rank"), param(params, "top_n")),
            format!("Rank {} is beyond top {}", param(params, "rank"), param(params, "top_n")),
        ),
        ReasonCode::WeightCappedAtMax => (
            format!("최대 비중 {} 상한 적용", param(params, "max_weight")),
            format!("Weight capped at max {}", param(params, "max_weight")),
        ),
        ReasonCode::WeightRoundingResidueToCash => (
            format!("반올림 잔여 {}을 현금으로 배분", param(params, "residue")),
            format!("Rounding residue {} allocated to cash", param(params, "residue")),
        ),
        ReasonCode::AllCashNoEligible => (
            "선정 가능한 종목이 없어 전액 현금 유지".to_owned(),
            "No eligible instrument; portfolio held in cash".to_owned(),
        ),
        ReasonCode::CashFloorApplied => (
            format!("현금 최소 비중 {} 보장", param(params, "cash_floor")),
            format!("Cash floor {} maintained", param(params, "cash_floor")),
        ),
    }
}
