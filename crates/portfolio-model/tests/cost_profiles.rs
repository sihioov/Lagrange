//! Red-first contract suite for the versioned cost profiles (design §9.4).
//!
//! Covers: `KRX_ETF_DEFAULT | CUSTOM` profile identity, versioned settings,
//! the minimum-commission edge, sell-side tax only on sells, slippage embedded
//! in the execution price (`buy = open x (1 + slip)`, `sell = open x (1 - slip)`),
//! the fee-balance identity `commission + tax + slippage == total`, JSON
//! round-trip stability, and the cost-monotonicity property (higher costs can
//! never produce a smaller fee total on the same fill).

use domain::{Currency, FixedPoint, Money, Price, Quantity};
use proptest::prelude::*;
use proptest::test_runner::Config as PropConfig;

use portfolio_model::cost::{CostBreakdown, CostProfile, CostProfileId};
use portfolio_model::error::PortfolioError;
use portfolio_model::side::Side;

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn price(amount: &str) -> Price {
    Price::parse(amount).expect("valid price")
}

fn qty(units: u64) -> Quantity {
    Quantity::parse(&units.to_string()).expect("valid quantity")
}

fn default_profile() -> CostProfile {
    CostProfile::krx_etf_default().expect("default profile builds")
}

#[test]
fn krx_etf_default_is_versioned_and_deterministic() {
    let p = default_profile();
    assert_eq!(p.profile_id, CostProfileId::KrxEtfDefault);
    assert_eq!(p.version, 1, "settings are versioned, not code constants");
    assert_eq!(
        p.commission_rate,
        FixedPoint::parse("0.00015").expect("rate")
    );
    assert_eq!(p.min_commission, krw("1000"));
    assert_eq!(p.sell_tax_rate, FixedPoint::parse("0").expect("rate"));
    assert_eq!(p.slippage_bps, 10);
    assert_eq!(p.min_trade, krw("100000"));
    assert_eq!(
        p.rebalance_threshold,
        FixedPoint::parse("0.005").expect("threshold")
    );
    // Two builds are identical (no hidden randomness, no clock).
    assert_eq!(
        default_profile(),
        p,
        "default profile must be deterministic"
    );
}

#[test]
fn custom_profile_parses_explicit_settings() {
    let p = CostProfile::custom("0.003", "2500", "0.0015", 20, "50000", "0.01")
        .expect("custom profile builds");
    assert_eq!(p.profile_id, CostProfileId::Custom);
    assert_eq!(p.version, 1);
    assert_eq!(p.commission_rate, FixedPoint::parse("0.003").expect("rate"));
    assert_eq!(p.min_commission, krw("2500"));
    assert_eq!(p.sell_tax_rate, FixedPoint::parse("0.0015").expect("rate"));
    assert_eq!(p.slippage_bps, 20);
    assert_eq!(p.min_trade, krw("50000"));
    assert_eq!(
        p.rebalance_threshold,
        FixedPoint::parse("0.01").expect("threshold")
    );
}

#[test]
fn negative_or_oversized_settings_are_rejected() {
    assert!(matches!(
        CostProfile::custom("-0.001", "1000", "0", 10, "100000", "0.005"),
        Err(PortfolioError::NegativeRate { .. })
    ));
    assert!(matches!(
        CostProfile::custom("0.00015", "1000", "-0.002", 10, "100000", "0.005"),
        Err(PortfolioError::NegativeRate { .. })
    ));
    assert!(matches!(
        CostProfile::custom("0.00015", "1000", "0", 10_001, "100000", "0.005"),
        Err(PortfolioError::SlippageOutOfRange { .. })
    ));
    assert!(matches!(
        CostProfile::custom("0.00015", "1000", "0", 10, "100000", "-0.01"),
        Err(PortfolioError::NegativeRate { .. })
    ));
}

#[test]
fn minimum_commission_applies_at_the_edge_deterministically() {
    // Notional 1000 KRW: 0.015% commission would be 0.15 KRW < the 1000 KRW
    // minimum, so the minimum applies exactly.
    let b = default_profile()
        .estimate(Side::Buy, &qty(1), &price("1000"))
        .expect("estimate");
    assert_eq!(b.commission, krw("1000"));
    assert_eq!(b.tax, krw("0"));
    // A different tiny fill is charged the SAME exact minimum (deterministic edge).
    let b2 = default_profile()
        .estimate(Side::Buy, &qty(3), &price("321"))
        .expect("estimate");
    assert_eq!(b2.commission, krw("1000"));
    assert_eq!(
        b.commission, b2.commission,
        "minimum-fee edge must be deterministic"
    );
}

#[test]
fn commission_scales_above_the_minimum() {
    // 10,000,000 shares @ 10,000 KRW = 100,000,000,000 KRW notional;
    // 0.015% = 15,000,000 KRW (far above the 1000 KRW minimum).
    let b = default_profile()
        .estimate(Side::Buy, &qty(10_000_000), &price("10000"))
        .expect("estimate");
    assert_eq!(b.commission, krw("15000000"));
}

#[test]
fn sell_tax_only_on_sells() {
    let p = CostProfile::custom("0.001", "0", "0.002", 0, "1000", "0.001").expect("custom profile");
    let buy = p
        .estimate(Side::Buy, &qty(100), &price("5000"))
        .expect("estimate");
    assert_eq!(buy.tax, krw("0"), "no tax on buys");
    let sell = p
        .estimate(Side::Sell, &qty(100), &price("5000"))
        .expect("estimate");
    assert_eq!(sell.tax, krw("1000"), "sell tax = 500,000 x 0.002");
}

#[test]
fn execution_price_embeds_slippage() {
    let p = default_profile(); // 10 bps
    let raw = price("10150.0000");
    assert_eq!(
        p.execution_price(&raw, Side::Buy).expect("buy exec"),
        price("10160.1500")
    );
    assert_eq!(
        p.execution_price(&raw, Side::Sell).expect("sell exec"),
        price("10139.8500")
    );
    // Zero slippage: execution price is the raw open (Todo 10 execution basis).
    let p0 =
        CostProfile::custom("0.00015", "1000", "0", 0, "100000", "0.005").expect("custom profile");
    assert_eq!(p0.execution_price(&raw, Side::Buy).expect("buy exec"), raw);
    assert_eq!(
        p0.execution_price(&raw, Side::Sell).expect("sell exec"),
        raw
    );
}

#[test]
fn sell_slippage_never_goes_nonpositive() {
    let p = CostProfile::custom("0", "0", "0", 10_000, "1000", "0").expect("custom profile");
    assert!(matches!(
        p.execution_price(&price("10150"), Side::Sell),
        Err(PortfolioError::NonPositiveExecutionPrice { .. })
    ));
}

#[test]
fn breakdown_identity_holds() {
    // 50,000 shares @ 10150 = 507,500,000 KRW notional.
    // commission = max(0.00015 x 507,500,000, 1000) = 76,125
    // slippage  (10 bps, informational) = 507,500
    // tax       (sell, 0 for ETF default) = 0
    let b = default_profile()
        .estimate(Side::Sell, &qty(50_000), &price("10150"))
        .expect("estimate");
    assert_eq!(b.commission, krw("76125"));
    assert_eq!(b.tax, krw("0"));
    assert_eq!(b.slippage, krw("507500"));
    assert_eq!(
        b.commission
            .checked_add(&b.tax)
            .and_then(|t| t.checked_add(&b.slippage))
            .expect("sum"),
        b.total,
        "commission + tax + slippage == total"
    );
}

#[test]
fn breakdown_json_round_trip_is_stable() {
    let b: CostBreakdown = default_profile()
        .estimate(Side::Sell, &qty(50_000), &price("10150"))
        .expect("estimate");
    let bytes = serde_json::to_vec(&b).expect("serialize");
    let back: CostBreakdown = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(b, back);
    assert_eq!(
        bytes,
        serde_json::to_vec(&b).expect("reserialize"),
        "byte-stable JSON"
    );
}

#[test]
fn profile_json_round_trip_is_stable() {
    let p = default_profile();
    let bytes = serde_json::to_vec(&p).expect("serialize");
    let back: CostProfile = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(p, back);
    assert_eq!(
        bytes,
        serde_json::to_vec(&p).expect("reserialize"),
        "byte-stable JSON"
    );
}

#[test]
fn higher_fees_never_produce_a_smaller_total_on_the_same_fill() {
    // Deterministic ladder: strictly higher costs -> fee total never decreases.
    let lo = CostProfile::custom("0.00015", "1000", "0", 0, "0", "0").expect("profile");
    let mid = CostProfile::custom("0.0003", "1000", "0", 0, "0", "0").expect("profile");
    let hi = CostProfile::custom("0.003", "5000", "0.002", 0, "0", "0").expect("profile");
    let cases = [
        (Side::Buy, qty(1), price("1000")),
        (Side::Buy, qty(30_000), price("10150")),
        (Side::Sell, qty(30_000), price("10150")),
        (Side::Sell, qty(1), price("999")),
        (Side::Buy, qty(1_000_000), price("24850")),
    ];
    for (side, q, p) in cases {
        let t_lo = lo.estimate(side, &q, &p).expect("estimate").total;
        let t_mid = mid.estimate(side, &q, &p).expect("estimate").total;
        let t_hi = hi.estimate(side, &q, &p).expect("estimate").total;
        assert!(
            t_mid.amount() >= t_lo.amount(),
            "mid >= lo for {side:?} {q} x {p}"
        );
        assert!(
            t_hi.amount() >= t_mid.amount(),
            "hi >= mid for {side:?} {q} x {p}"
        );
    }
}

proptest! {
    #![proptest_config(PropConfig::with_cases(64))]

    /// Cost monotonicity property: for ANY (side, quantity, price) and any
    /// two profiles A (lower costs) and B (higher costs), B's fee total is
    /// never below A's on the same fill.
    #[test]
    fn higher_costs_never_reduce_total(
        side_buy in any::<bool>(),
        shares in 1u64..2_000_000,
        px in 500u64..50_000u64,
    ) {
        let side = if side_buy { Side::Buy } else { Side::Sell };
        let q = qty(shares);
        let p = price(&format!("{px}"));
        let lo = CostProfile::custom("0.00015", "1000", "0", 0, "0", "0").expect("profile");
        let hi = CostProfile::custom("0.004", "8000", "0.0025", 0, "0", "0").expect("profile");
        let t_lo = lo.estimate(side, &q, &p).expect("estimate").total;
        let t_hi = hi.estimate(side, &q, &p).expect("estimate").total;
        prop_assert!(t_hi.amount() >= t_lo.amount());
    }
}
