//! Todo 9 red/green integration tests: canonical instrument master, KRX/KIS/
//! provider aliases with effective intervals, listing lifecycle, ticker-change
//! alias history, and constraint rejection (overlap / invalid intervals).
//!
//! Acceptance contract (plan Todo 9 + FR-DATA-002/006 + design §6.4):
//! - canonical `InstrumentId = {symbol}.KRX`, never silently re-created on a
//!   ticker change — alias history records the remap instead;
//! - delisted / not-yet-listed instruments are excluded on historical dates;
//! - duplicate active aliases on the same effective interval are rejected;
//! - invalid date ranges are rejected.

use market_data::instrument_master::{
    AliasNamespace, Instrument, InstrumentAlias, InstrumentMaster, ListingReason, MasterError,
    seed_universe,
};

use domain::{
    AssetClass, Currency, InstrumentId, InstrumentStatus, Price, Quantity, TradingDate, Venue,
};

fn d(s: &str) -> TradingDate {
    TradingDate::parse(s).unwrap()
}

/// A 3-instrument fixture master exercising listing lifecycle + KIS aliasing.
fn lifecycle_master() -> InstrumentMaster {
    let mut master = InstrumentMaster::new();
    let price_inc = Price::parse("1").unwrap();
    let size_inc = Quantity::parse("1").unwrap();
    let lot = Quantity::parse("100").unwrap();

    // Delisted before 2020-01-31.
    master
        .register_instrument(Instrument {
            instrument_id: InstrumentId::parse("999001.KRX").unwrap(),
            name: "SYNTHETIC-DELISTED".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2019-01-01"),
            delisted_at: Some(d("2019-12-31")),
            price_increment: price_inc,
            size_increment: size_inc,
            lot_size: lot,
            status: InstrumentStatus::Delisted,
            reference_source: "krx-reference-2019-v1".to_owned(),
        })
        .unwrap();
    // Not yet listed on 2020-01-31 (lists 2020-03-01).
    master
        .register_instrument(Instrument {
            instrument_id: InstrumentId::parse("999002.KRX").unwrap(),
            name: "SYNTHETIC-FUTURE".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2020-03-01"),
            delisted_at: None,
            price_increment: price_inc,
            size_increment: size_inc,
            lot_size: lot,
            status: InstrumentStatus::Listed,
            reference_source: "krx-reference-2020-v1".to_owned(),
        })
        .unwrap();
    // Active through the whole window.
    master
        .register_instrument(Instrument {
            instrument_id: InstrumentId::parse("999003.KRX").unwrap(),
            name: "SYNTHETIC-ACTIVE".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2019-01-01"),
            delisted_at: None,
            price_increment: price_inc,
            size_increment: size_inc,
            lot_size: lot,
            status: InstrumentStatus::Listed,
            reference_source: "krx-reference-2019-v1".to_owned(),
        })
        .unwrap();
    master
        .register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("999001.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "999001".to_owned(),
            effective_from: d("2019-01-01"),
            effective_until: Some(d("2019-12-31")),
            source: "krx-reference-2019-v1".to_owned(),
            version: 1,
        })
        .unwrap();
    master
        .register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("999003.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "999003".to_owned(),
            effective_from: d("2019-01-01"),
            effective_until: None,
            source: "krx-reference-2019-v1".to_owned(),
            version: 1,
        })
        .unwrap();
    master
}

#[test]
fn alias_resolution_through_krx_kis_and_provider_on_effective_date() {
    // (a) resolution through KRX and KIS aliases on an effective date.
    let master = seed_universe();
    let date = d("2020-01-31");
    let canonical = InstrumentId::parse("069500.KRX").unwrap();

    assert_eq!(
        master.resolve(AliasNamespace::Krx, "069500", date).unwrap(),
        canonical
    );
    assert_eq!(
        master.resolve(AliasNamespace::Kis, "069500", date).unwrap(),
        canonical
    );
    assert_eq!(
        master
            .resolve(AliasNamespace::Provider, "KODEX200", date)
            .unwrap(),
        canonical
    );
    assert_eq!(
        master.resolve(AliasNamespace::Krx, "229200", date).unwrap(),
        InstrumentId::parse("229200.KRX").unwrap()
    );

    // Instrument metadata on the effective date.
    let instrument = master.instrument_on(&canonical, date).unwrap();
    assert_eq!(instrument.venue, Venue::Krx);
    assert_eq!(instrument.currency, Currency::KRW);
    assert_eq!(instrument.lot_size, Quantity::parse("100").unwrap());
    assert_eq!(instrument.size_increment, Quantity::parse("1").unwrap());
    assert_eq!(instrument.price_increment, Price::parse("1").unwrap());
    assert_eq!(instrument.status, InstrumentStatus::Listed);
}

#[test]
fn unknown_alias_on_date_is_typed_error() {
    let master = seed_universe();
    assert!(matches!(
        master.resolve(AliasNamespace::Krx, "999999", d("2020-01-31")),
        Err(MasterError::UnknownAlias { .. })
    ));
    // Alias 069500 does not exist in the provider namespace.
    assert!(matches!(
        master.resolve(AliasNamespace::Provider, "069500", d("2020-01-31")),
        Err(MasterError::UnknownAlias { .. })
    ));
}

#[test]
fn ticker_change_keeps_identity_and_appends_alias_history() {
    // (b) a ticker change remaps the alias, never the canonical identity.
    let mut master = seed_universe();
    let canonical = InstrumentId::parse("069500.KRX").unwrap();
    let change_date = d("2020-06-01");

    let new_alias = master
        .change_ticker(
            &canonical,
            AliasNamespace::Krx,
            "069500",
            "069501",
            change_date,
            "krx-reference-2020-v2",
        )
        .expect("ticker change must succeed");

    // New alias resolves after the change date...
    assert_eq!(
        master
            .resolve(AliasNamespace::Krx, "069501", change_date)
            .unwrap(),
        canonical
    );
    assert_eq!(
        master
            .resolve(AliasNamespace::Krx, "069501", d("2020-12-31"))
            .unwrap(),
        canonical
    );
    // ...the old alias still resolves during its effective interval...
    assert_eq!(
        master
            .resolve(AliasNamespace::Krx, "069500", d("2020-05-31"))
            .unwrap(),
        canonical
    );
    // ...and no longer resolves after the change (its interval was closed).
    assert!(matches!(
        master.resolve(AliasNamespace::Krx, "069500", change_date),
        Err(MasterError::UnknownAlias { .. })
    ));

    // Canonical identity is untouched: same id, same metadata, other
    // namespaces unaffected (the KIS alias keeps its own history).
    let instrument = master.instrument_on(&canonical, change_date).unwrap();
    assert_eq!(instrument.instrument_id, canonical);
    assert_eq!(instrument.venue, Venue::Krx);
    assert_eq!(
        master
            .resolve(AliasNamespace::Kis, "069500", change_date)
            .unwrap(),
        canonical
    );

    // Alias history was appended (never overwritten): the KRX namespace for
    // 069500.KRX now carries two ordered versions.
    let history: Vec<&InstrumentAlias> = master.alias_history(&canonical);
    let krx_history: Vec<&InstrumentAlias> = history
        .iter()
        .copied()
        .filter(|a| a.namespace == AliasNamespace::Krx)
        .collect();
    assert_eq!(krx_history.len(), 2);
    assert_eq!(krx_history[0].symbol, "069500");
    assert_eq!(krx_history[0].effective_until, Some(d("2020-05-31")));
    assert_eq!(krx_history[0].version, 1);
    assert_eq!(krx_history[1].symbol, "069501");
    assert_eq!(krx_history[1].effective_from, change_date);
    assert_eq!(krx_history[1].effective_until, None);
    assert_eq!(krx_history[1].version, 2);
    assert_eq!(new_alias.symbol, "069501");
}

#[test]
fn ticker_change_rejects_unknown_old_symbol() {
    let mut master = seed_universe();
    let canonical = InstrumentId::parse("069500.KRX").unwrap();
    assert!(matches!(
        master.change_ticker(
            &canonical,
            AliasNamespace::Krx,
            "999999",
            "069501",
            d("2020-06-01"),
            "krx-reference-2020-v2",
        ),
        Err(MasterError::UnknownAlias { .. })
    ));
}

#[test]
fn delisted_and_not_yet_listed_symbols_excluded() {
    // (d) instruments invalid on the backtest date are excluded / typed error.
    let master = lifecycle_master();
    let date = d("2020-01-31");
    let delisted = InstrumentId::parse("999001.KRX").unwrap();
    let future = InstrumentId::parse("999002.KRX").unwrap();
    let active = InstrumentId::parse("999003.KRX").unwrap();

    assert!(matches!(
        master.instrument_on(&delisted, date),
        Err(MasterError::NotListed {
            reason: ListingReason::Delisted,
            ..
        })
    ));
    assert!(matches!(
        master.instrument_on(&future, date),
        Err(MasterError::NotListed {
            reason: ListingReason::NotYetListed,
            ..
        })
    ));
    assert_eq!(
        master.instrument_on(&active, date).unwrap().instrument_id,
        active
    );
    assert!(master.instrument_on(&delisted, d("2019-06-01")).is_ok());
    assert!(master.instrument_on(&future, d("2020-06-01")).is_ok());

    // Enumeration excludes invalid instruments on the date.
    let active_ids: Vec<InstrumentId> = master
        .active_on(date)
        .into_iter()
        .map(|i| i.instrument_id.clone())
        .collect();
    assert!(active_ids.contains(&active));
    assert!(!active_ids.contains(&delisted));
    assert!(!active_ids.contains(&future));
    assert_eq!(active_ids.len(), 1);
}

#[test]
fn overlapping_active_alias_intervals_rejected() {
    // (f) duplicate active aliases for the same instrument on the same
    // effective interval are rejected.
    let mut master = InstrumentMaster::new();
    master
        .register_instrument(Instrument {
            instrument_id: InstrumentId::parse("069500.KRX").unwrap(),
            name: "SYNTHETIC-KODEX200".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2019-01-01"),
            delisted_at: None,
            price_increment: Price::parse("1").unwrap(),
            size_increment: Quantity::parse("1").unwrap(),
            lot_size: Quantity::parse("100").unwrap(),
            status: InstrumentStatus::Listed,
            reference_source: "krx-reference-2019-v1".to_owned(),
        })
        .unwrap();
    master
        .register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("069500.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "069500".to_owned(),
            effective_from: d("2019-01-01"),
            effective_until: None,
            source: "krx-reference-2019-v1".to_owned(),
            version: 1,
        })
        .unwrap();

    // Exact duplicate of the open-ended alias.
    assert!(matches!(
        master.register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("069500.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "069500".to_owned(),
            effective_from: d("2019-01-01"),
            effective_until: None,
            source: "krx-reference-2019-v1".to_owned(),
            version: 1,
        }),
        Err(MasterError::OverlappingAlias { .. })
    ));
    // Partially overlapping interval (starts inside the open-ended one).
    assert!(matches!(
        master.register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("069500.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "069500".to_owned(),
            effective_from: d("2020-06-01"),
            effective_until: None,
            source: "krx-reference-2020-v1".to_owned(),
            version: 1,
        }),
        Err(MasterError::OverlappingAlias { .. })
    ));
}

#[test]
fn invalid_alias_date_ranges_rejected() {
    // (f) effective_until before effective_from is an invalid range.
    let mut master = InstrumentMaster::new();
    master
        .register_instrument(Instrument {
            instrument_id: InstrumentId::parse("069500.KRX").unwrap(),
            name: "SYNTHETIC-KODEX200".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2019-01-01"),
            delisted_at: None,
            price_increment: Price::parse("1").unwrap(),
            size_increment: Quantity::parse("1").unwrap(),
            lot_size: Quantity::parse("100").unwrap(),
            status: InstrumentStatus::Listed,
            reference_source: "krx-reference-2019-v1".to_owned(),
        })
        .unwrap();
    assert!(matches!(
        master.register_alias(InstrumentAlias {
            instrument: InstrumentId::parse("069500.KRX").unwrap(),
            namespace: AliasNamespace::Krx,
            symbol: "069500".to_owned(),
            effective_from: d("2020-06-01"),
            effective_until: Some(d("2020-01-01")),
            source: "krx-reference-2020-v1".to_owned(),
            version: 1,
        }),
        Err(MasterError::InvalidAliasInterval { .. })
    ));

    // Invalid listing lifecycle: delisted_at before listed_at is rejected by
    // the registry.
    assert!(matches!(
        master.register_instrument(Instrument {
            instrument_id: InstrumentId::parse("999004.KRX").unwrap(),
            name: "SYNTHETIC-BROKEN".to_owned(),
            asset_class: AssetClass::Etf,
            currency: Currency::KRW,
            venue: Venue::Krx,
            listed_at: d("2019-01-01"),
            delisted_at: Some(d("2018-01-01")),
            price_increment: Price::parse("1").unwrap(),
            size_increment: Quantity::parse("1").unwrap(),
            lot_size: Quantity::parse("100").unwrap(),
            status: InstrumentStatus::Delisted,
            reference_source: "krx-reference-2019-v1".to_owned(),
        }),
        Err(MasterError::InvalidListingInterval { .. })
    ));
}
