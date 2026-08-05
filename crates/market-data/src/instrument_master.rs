//! Canonical instrument master and the KRX/KIS/provider alias registry (Todo 9).
//!
//! Design §6.4 separates the internal identity from provider tickers:
//! `InstrumentId = {canonical_symbol}.{venue}` (e.g. `069500.KRX`). A ticker
//! change REMAPS the alias — appending a new versioned alias record and
//! closing the previous one's effective interval — it never creates a new
//! identity and never silently mutates the canonical `InstrumentId`.
//!
//! Instrument master fields follow requirements §8.2 (`instrument_id, symbol,
//! venue, asset_class, currency, listed_at, delisted_at, price_increment,
//! size_increment, lot_size, status`). Delisted instruments are KEPT for
//! point-in-time correctness (FR-DATA-006 / §8.3): they resolve on dates
//! inside their listing window and are excluded outside it.
//!
//! Constraints enforced on registration (FR-DATA-002 acceptance):
//! - an alias interval must not be inverted (`effective_until < effective_from`
//!   is rejected);
//! - two aliases of the same `(namespace, symbol)` with overlapping effective
//!   intervals are rejected (duplicate active alias);
//! - a listing window must not be inverted (`delisted_at < listed_at`).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use domain::{
    AssetClass, Currency, InstrumentId, InstrumentStatus, Price, Quantity, TradingDate, Venue,
};

/// The alias namespace of a ticker: the exchange code, the broker code, or a
/// data-provider symbol (design §6.4 mapping table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasNamespace {
    /// KRX exchange code (six-digit, e.g. `069500`).
    Krx,
    /// KIS broker code (Owner-only broker channel).
    Kis,
    /// Data-provider symbol (e.g. `KODEX200`).
    Provider,
}

impl AliasNamespace {
    /// The stable wire name (`krx` | `kis` | `provider`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Krx => "krx",
            Self::Kis => "kis",
            Self::Provider => "provider",
        }
    }
}

impl fmt::Display for AliasNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A canonical instrument master record (requirements §8.2 instrument master).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    /// The canonical internal identity — never re-created on a ticker change.
    pub instrument_id: InstrumentId,
    /// Human-readable name (synthetic seed names use the `SYNTHETIC-` prefix).
    pub name: String,
    /// Asset class (the fixed Korean ETF universe is `Etf`).
    pub asset_class: AssetClass,
    /// Trading currency (KRW for the Korean universe).
    pub currency: Currency,
    /// Listing venue (KRX for canonical instruments; KIS aliases are broker
    /// codes of the same canonical instrument).
    pub venue: Venue,
    /// The first date the instrument was tradable on the venue.
    pub listed_at: TradingDate,
    /// The last date the instrument was tradable, if it has been delisted.
    pub delisted_at: Option<TradingDate>,
    /// Minimum price increment (tick size) of the venue's price scale.
    pub price_increment: Price,
    /// Minimum size increment of the venue's quantity scale.
    pub size_increment: Quantity,
    /// Minimum order size (KRX ETF seed: 100 units, matching the Todo 6
    /// fixture `lot_size`).
    pub lot_size: Quantity,
    /// Current listing status (`listed` | `delisted` | `suspended`).
    pub status: InstrumentStatus,
    /// Provenance: the reference source that defined this record.
    pub reference_source: String,
}

impl Instrument {
    /// Whether the instrument was tradable on `date` (point-in-time).
    pub fn is_listed_on(&self, date: TradingDate) -> bool {
        date >= self.listed_at
            && match self.delisted_at {
                Some(delisted_at) => date < delisted_at,
                None => true,
            }
    }
}

/// Why an instrument is not tradable on a historical date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingReason {
    /// `date < listed_at`: the instrument had not listed yet.
    NotYetListed,
    /// `date >= delisted_at`: the instrument had already been delisted.
    Delisted,
}

impl fmt::Display for ListingReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotYetListed => f.write_str("not yet listed"),
            Self::Delisted => f.write_str("delisted"),
        }
    }
}

/// A single versioned alias record: `(namespace, symbol)` resolves to the
/// canonical instrument during `[effective_from, effective_until]` (open-ended
/// when `effective_until` is `None`).
///
/// Records are versioned per `(instrument, namespace)`: a ticker change closes
/// the previous record (finalizing its `effective_until` exactly once) and
/// appends the next record, so alias history is preserved and queryable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentAlias {
    /// The canonical instrument this alias resolves to.
    pub instrument: InstrumentId,
    /// Which namespace this ticker belongs to.
    pub namespace: AliasNamespace,
    /// The ticker/symbol inside that namespace.
    pub symbol: String,
    /// First effective date (inclusive).
    pub effective_from: TradingDate,
    /// Last effective date (inclusive); `None` = open-ended.
    pub effective_until: Option<TradingDate>,
    /// Provenance of this alias version (e.g. `krx-reference-2020-v2`).
    pub source: String,
    /// Per-`(instrument, namespace)` history ordinal, 1-based.
    pub version: u32,
}

/// Typed errors of the instrument master / alias registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MasterError {
    /// The alias interval is inverted (`effective_until < effective_from`).
    #[error("invalid alias interval for {namespace}:{symbol}: until {until} precedes from {from}")]
    InvalidAliasInterval {
        namespace: AliasNamespace,
        symbol: String,
        from: String,
        until: String,
    },
    /// A duplicate active alias: the interval overlaps an existing alias of
    /// the same `(namespace, symbol)`.
    #[error("overlapping active alias {namespace}:{symbol} for [{from}, {until})")]
    OverlappingAlias {
        namespace: AliasNamespace,
        symbol: String,
        from: String,
        until: String,
    },
    /// No alias `(namespace, symbol)` was active on the requested date.
    #[error("unknown alias {namespace}:{symbol} on {date}")]
    UnknownAlias {
        namespace: AliasNamespace,
        symbol: String,
        date: String,
    },
    /// The instrument id is not registered in this master.
    #[error("unknown instrument {id}")]
    UnknownInstrument { id: String },
    /// The instrument is not tradable on the requested date.
    #[error("instrument {id} not listed on {date} ({reason})")]
    NotListed {
        id: String,
        date: String,
        reason: ListingReason,
    },
    /// The listing window is inverted (`delisted_at < listed_at`).
    #[error("invalid listing interval for {id}: delisted_at precedes listed_at")]
    InvalidListingInterval { id: String },
}

/// The instrument master: canonical instruments plus their alias histories.
#[derive(Debug, Clone, Default)]
pub struct InstrumentMaster {
    instruments: BTreeMap<InstrumentId, Instrument>,
    aliases: Vec<InstrumentAlias>,
}

impl InstrumentMaster {
    /// An empty master.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a canonical instrument, validating its listing window.
    pub fn register_instrument(&mut self, instrument: Instrument) -> Result<(), MasterError> {
        if let Some(delisted_at) = instrument.delisted_at {
            if delisted_at < instrument.listed_at {
                return Err(MasterError::InvalidListingInterval {
                    id: instrument.instrument_id.to_string(),
                });
            }
        }
        if self.instruments.contains_key(&instrument.instrument_id) {
            return Err(MasterError::UnknownInstrument {
                id: format!("duplicate registration of {}", instrument.instrument_id),
            });
        }
        self.instruments.insert(instrument.instrument_id.clone(), instrument);
        Ok(())
    }

    /// Registers an alias record, rejecting inverted intervals and overlaps
    /// with any existing alias of the same `(namespace, symbol)`.
    pub fn register_alias(&mut self, alias: InstrumentAlias) -> Result<(), MasterError> {
        validate_alias_interval(&alias)?;
        for existing in self.aliases.iter().filter(|a| {
            a.namespace == alias.namespace && a.symbol == alias.symbol
        }) {
            if intervals_overlap(
                existing.effective_from,
                existing.effective_until,
                alias.effective_from,
                alias.effective_until,
            ) {
                return Err(MasterError::OverlappingAlias {
                    namespace: alias.namespace,
                    symbol: alias.symbol.clone(),
                    from: alias.effective_from.to_iso(),
                    until: alias
                        .effective_until
                        .map(|d| d.to_iso())
                        .unwrap_or_else(|| "open".to_owned()),
                });
            }
        }
        self.aliases.push(alias);
        Ok(())
    }

    /// A ticker change REMAPS `old_symbol` -> `new_symbol` in `namespace` for
    /// the canonical instrument, effective on `effective_date`.
    ///
    /// This is the only sanctioned write to an existing alias record: the old
    /// record's effective interval is finalized (closed the day before the
    /// change) and the new record is appended with the next history version.
    /// The canonical `InstrumentId` is never touched.
    pub fn change_ticker(
        &mut self,
        instrument: &InstrumentId,
        namespace: AliasNamespace,
        old_symbol: &str,
        new_symbol: &str,
        effective_date: TradingDate,
        source: &str,
    ) -> Result<InstrumentAlias, MasterError> {
        let old = self
            .aliases
            .iter_mut()
            .find(|a| {
                a.instrument == *instrument
                    && a.namespace == namespace
                    && a.symbol == old_symbol
                    && alias_active_at(a, effective_date)
            })
            .ok_or_else(|| MasterError::UnknownAlias {
                namespace,
                symbol: old_symbol.to_owned(),
                date: effective_date.to_iso(),
            })?;

        old.effective_until = Some(effective_date.previous_day());
        old.source = source.to_owned();

        let new_alias = InstrumentAlias {
            instrument: instrument.clone(),
            namespace,
            symbol: new_symbol.to_owned(),
            effective_from: effective_date,
            effective_until: None,
            source: source.to_owned(),
            version: old.version + 1,
        };
        // The new symbol must not collide with any alias in the namespace.
        for existing in self
            .aliases
            .iter()
            .filter(|a| a.namespace == namespace && a.symbol == new_symbol)
        {
            if intervals_overlap(
                existing.effective_from,
                existing.effective_until,
                new_alias.effective_from,
                new_alias.effective_until,
            ) {
                return Err(MasterError::OverlappingAlias {
                    namespace,
                    symbol: new_symbol.to_owned(),
                    from: new_alias.effective_from.to_iso(),
                    until: new_alias
                        .effective_until
                        .map(|d| d.to_iso())
                        .unwrap_or_else(|| "open".to_owned()),
                });
            }
        }
        self.aliases.push(new_alias.clone());
        Ok(new_alias)
    }

    /// Resolves `(namespace, symbol)` to the canonical instrument active on
    /// `date` (design §6.4 mapping table lookup).
    pub fn resolve(
        &self,
        namespace: AliasNamespace,
        symbol: &str,
        date: TradingDate,
    ) -> Result<InstrumentId, MasterError> {
        self.aliases
            .iter()
            .find(|a| {
                a.namespace == namespace && a.symbol == symbol && alias_active_at(a, date)
            })
            .map(|a| a.instrument.clone())
            .ok_or_else(|| MasterError::UnknownAlias {
                namespace,
                symbol: symbol.to_owned(),
                date: date.to_iso(),
            })
    }

    /// The canonical instrument record, point-in-time: delisted or not-yet-
    /// listed instruments are a typed error on `date` (FR-DATA-002).
    pub fn instrument_on(
        &self,
        instrument: &InstrumentId,
        date: TradingDate,
    ) -> Result<&Instrument, MasterError> {
        let record = self
            .instruments
            .get(instrument)
            .ok_or_else(|| MasterError::UnknownInstrument {
                id: instrument.to_string(),
            })?;
        if !record.is_listed_on(date) {
            let reason = if date < record.listed_at {
                ListingReason::NotYetListed
            } else {
                ListingReason::Delisted
            };
            return Err(MasterError::NotListed {
                id: instrument.to_string(),
                date: date.to_iso(),
                reason,
            });
        }
        Ok(record)
    }

    /// Every instrument tradable on `date` (the point-in-time candidate set).
    pub fn active_on(&self, date: TradingDate) -> Vec<&Instrument> {
        self.instruments
            .values()
            .filter(|i| i.is_listed_on(date))
            .collect()
    }

    /// The full alias history of a canonical instrument, ordered by version.
    /// A ticker change APPENDS to this history — it never overwrites it.
    pub fn alias_history(&self, instrument: &InstrumentId) -> Vec<&InstrumentAlias> {
        let mut history: Vec<&InstrumentAlias> = self
            .aliases
            .iter()
            .filter(|a| a.instrument == *instrument)
            .collect();
        history.sort_by_key(|a| (a.version, a.effective_from, a.symbol.clone()));
        history
    }
}

fn validate_alias_interval(alias: &InstrumentAlias) -> Result<(), MasterError> {
    if let Some(until) = alias.effective_until {
        if until < alias.effective_from {
            return Err(MasterError::InvalidAliasInterval {
                namespace: alias.namespace,
                symbol: alias.symbol.clone(),
                from: alias.effective_from.to_iso(),
                until: until.to_iso(),
            });
        }
    }
    Ok(())
}

fn alias_active_at(alias: &InstrumentAlias, date: TradingDate) -> bool {
    date >= alias.effective_from
        && match alias.effective_until {
            Some(until) => date <= until,
            None => true,
        }
}

/// Interval overlap with an open-ended interval treated as unbounded.
fn intervals_overlap(
    a_from: TradingDate,
    a_until: Option<TradingDate>,
    b_from: TradingDate,
    b_until: Option<TradingDate>,
) -> bool {
    let a_end = a_until.unwrap_or(TradingDate::new(9999, 12, 31).expect("max date"));
    let b_end = b_until.unwrap_or(TradingDate::new(9999, 12, 31).expect("max date"));
    a_from <= b_end && b_from <= a_end
}

/// The fixed Korean ETF v1 universe seed (plan Todo 12 symbol set), all
/// active on 2020-01-31: canonical KRX records, KRX + provider aliases, and
/// KIS broker aliases for the benchmark-related seeds.
///
/// Metadata follows the Todo 6 fixture: `lot_size = 100`, KRW currency,
/// integer-KRW price ticks. Names are synthetic (`SYNTHETIC-` prefix); real
/// ticker codes are public identifiers used solely for universe shape.
pub fn seed_universe() -> InstrumentMaster {
    let mut master = InstrumentMaster::new();
    let price_inc = Price::parse("1").expect("valid tick");
    let size_inc = Quantity::parse("1").expect("valid size");
    let lot = Quantity::parse("100").expect("valid lot");
    let listed_at = TradingDate::new(2019, 1, 1).expect("valid listing date");
    let source = "krx-reference-2019-v1";

    let symbols = [
        ("069500", "SYNTHETIC-KODEX200", "KODEX200"),
        ("102110", "SYNTHETIC-TIGER200", "TIGER200"),
        ("229200", "SYNTHETIC-KODEX-KOSPI200", "KODEX200TR"),
        ("143850", "SYNTHETIC-ETF143850", "SYN143850"),
        ("133690", "SYNTHETIC-ETF133690", "SYN133690"),
        ("195930", "SYNTHETIC-ETF195930", "SYN195930"),
        ("192090", "SYNTHETIC-ETF192090", "SYN192090"),
        ("148070", "SYNTHETIC-ETF148070", "SYN148070"),
        ("114260", "SYNTHETIC-KODEX-STBOND", "KODEXSTBOND"),
        ("153130", "SYNTHETIC-ETF153130", "SYN153130"),
        ("132030", "SYNTHETIC-ETF132030", "SYN132030"),
    ];

    for (symbol, name, provider_symbol) in symbols {
        let instrument_id = InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id");
        master
            .register_instrument(Instrument {
                instrument_id: instrument_id.clone(),
                name: name.to_owned(),
                asset_class: AssetClass::Etf,
                currency: Currency::KRW,
                venue: Venue::Krx,
                listed_at,
                delisted_at: None,
                price_increment: price_inc,
                size_increment: size_inc,
                lot_size: lot,
                status: InstrumentStatus::Listed,
                reference_source: source.to_owned(),
            })
            .expect("seed instrument registers");

        master
            .register_alias(InstrumentAlias {
                instrument: instrument_id.clone(),
                namespace: AliasNamespace::Krx,
                symbol: symbol.to_owned(),
                effective_from: listed_at,
                effective_until: None,
                source: source.to_owned(),
                version: 1,
            })
            .expect("seed krx alias registers");
        master
            .register_alias(InstrumentAlias {
                instrument: instrument_id.clone(),
                namespace: AliasNamespace::Provider,
                symbol: provider_symbol.to_owned(),
                effective_from: listed_at,
                effective_until: None,
                source: source.to_owned(),
                version: 1,
            })
            .expect("seed provider alias registers");
    }

    // KIS broker aliases (design §6.4 mapping table; Owner-only broker channel).
    for symbol in ["069500", "229200", "114260"] {
        let instrument_id = InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id");
        master
            .register_alias(InstrumentAlias {
                instrument: instrument_id,
                namespace: AliasNamespace::Kis,
                symbol: symbol.to_owned(),
                effective_from: listed_at,
                effective_until: None,
                source: source.to_owned(),
                version: 1,
            })
            .expect("seed kis alias registers");
    }

    master
}
