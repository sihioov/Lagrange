//! Wire/hash checks do not fabricate an admitted filesystem capability.
use std::collections::BTreeMap;

use domain::{ContentHash, TradingDate};
use factor_engine::daily_price_only::{
    DAILY_FACTOR_SNAPSHOT_SCHEMA, DAILY_PRICE_ONLY_INPUT_KIND, DailyPriceOnlyFactorSnapshot,
};
use factor_engine::price_only::PRICE_ONLY_CAPABILITY;
use factor_engine::snapshot::{FactorRow, NormalizationMeta};

fn snapshot() -> DailyPriceOnlyFactorSnapshot {
    DailyPriceOnlyFactorSnapshot {
        schema: DAILY_FACTOR_SNAPSHOT_SCHEMA.into(),
        input_kind: DAILY_PRICE_ONLY_INPUT_KIND.into(),
        capability: PRICE_ONLY_CAPABILITY.into(),
        as_of: TradingDate::parse("2026-08-31").unwrap(),
        content_sha256: format!("sha256:{:064x}", 1),
        manifest_sha256: format!("sha256:{:064x}", 2),
        admission_sha256: format!("sha256:{:064x}", 3),
        policy_sha256: format!("sha256:{:064x}", 4),
        factor_versions: BTreeMap::new(),
        normalization: NormalizationMeta {
            id: "none".into(),
            version: "1".into(),
            params: BTreeMap::new(),
        },
        rows: vec![],
        hash: ContentHash::from_bytes(b"not-in-preimage"),
    }
}

#[test]
fn daily_factor_has_an_exact_separate_canonical_preimage() {
    let value = snapshot();
    let expected = concat!(
        "{\"schema\":\"owner-beta-daily-factor-snapshot-v1\",",
        "\"input_kind\":\"owner_beta_daily_etf11_price_only_v1\",\"capability\":\"PRICE_RETURN_ONLY\",\"as_of\":\"2026-08-31\",",
        "\"content_sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000001\",",
        "\"manifest_sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000002\",",
        "\"admission_sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000003\",",
        "\"policy_sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000004\",",
        "\"factor_versions\":{},\"normalization\":{\"id\":\"none\",\"version\":\"1\",\"params\":{}},\"rows\":[]}"
    );
    assert_eq!(value.canonical_bytes().unwrap(), expected.as_bytes());
    assert_eq!(
        value.compute_hash().unwrap(),
        ContentHash::from_bytes(expected.as_bytes())
    );
}

#[test]
fn daily_factor_hash_binds_each_of_the_four_pins() {
    let baseline = snapshot();
    let hash = baseline.compute_hash().unwrap();
    for case in 0..4 {
        let mut changed = baseline.clone();
        let pin = match case {
            0 => &mut changed.content_sha256,
            1 => &mut changed.manifest_sha256,
            2 => &mut changed.admission_sha256,
            _ => &mut changed.policy_sha256,
        };
        *pin = format!("sha256:{:064x}", 99);
        assert_ne!(changed.compute_hash().unwrap(), hash);
    }
}

#[test]
fn daily_factor_hash_binds_date_domain_versions_and_normalization() {
    let baseline = snapshot();
    let hash = baseline.compute_hash().unwrap();
    for case in 0..7 {
        let mut changed = baseline.clone();
        match case {
            0 => changed.schema.push('x'),
            1 => changed.input_kind.push('x'),
            2 => changed.capability.push('x'),
            3 => changed.as_of = TradingDate::parse("2026-09-01").unwrap(),
            4 => {
                changed
                    .factor_versions
                    .insert("return_1m".into(), "1.0.0".into());
            }
            5 => changed.normalization.id.push('x'),
            _ => changed.normalization.version.push('x'),
        }
        assert_ne!(changed.compute_hash().unwrap(), hash);
    }
}

#[test]
fn daily_factor_null_is_not_zero_and_nonfinite_is_rejected() {
    let mut value = snapshot();
    value.rows.push(FactorRow {
        date: "2026-08-31".into(),
        instrument: "069500.KRX".into(),
        factor: "return_1m".into(),
        raw: None,
        normalized: None,
    });
    let null = value.compute_hash().unwrap();
    value.rows[0].raw = Some(0.0);
    assert_ne!(value.compute_hash().unwrap(), null);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        value.rows[0].raw = Some(invalid);
        assert!(value.canonical_bytes().is_err());
        value.rows[0].raw = None;
        value.rows[0].normalized = Some(invalid);
        assert!(value.canonical_bytes().is_err());
        value.rows[0].normalized = None;
    }
}

#[test]
fn stored_daily_factor_hash_is_excluded_from_its_preimage() {
    let mut value = snapshot();
    let before = value.canonical_bytes().unwrap();
    value.hash = ContentHash::from_bytes(b"different-stored-hash");
    assert_eq!(before, value.canonical_bytes().unwrap());
}
