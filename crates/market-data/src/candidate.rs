//! Provider-neutral point-in-time contracts for stock research candidates.
//!
//! These records are deliberately separate from ETF recommendation output.
//! They model licensed source observations and expose cutoff-aware resolvers;
//! no function silently selects the newest revision known today.

use std::collections::{BTreeMap, BTreeSet};

use domain::{InstrumentId, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{RawEnvelope, ResponseKind};

/// The finite candidate universes supported by the source contract.
///
/// This is intentionally an enum rather than an arbitrary string: registry
/// rows, membership datasets, and downstream run identity must agree on one
/// canonical spelling and must fail closed for unknown indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CandidateUniverseKey {
    #[serde(rename = "kospi200")]
    Kospi200,
    #[serde(rename = "kosdaq150")]
    Kosdaq150,
}

impl CandidateUniverseKey {
    pub const ALL: [Self; 2] = [Self::Kospi200, Self::Kosdaq150];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kospi200 => "kospi200",
            Self::Kosdaq150 => "kosdaq150",
        }
    }

    pub const fn dataset_id(self) -> &'static str {
        match self {
            Self::Kospi200 => "krx_kospi200_membership",
            Self::Kosdaq150 => "krx_kosdaq150_membership",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "kospi200" => Some(Self::Kospi200),
            "kosdaq150" => Some(Self::Kosdaq150),
            _ => None,
        }
    }
}

impl std::fmt::Display for CandidateUniverseKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSourcePin {
    pub provider: String,
    pub entitlement_id: Uuid,
    pub license_ref: String,
    pub dataset_id: String,
    pub dataset_version: String,
    pub manifest_sha256: String,
    pub retrieved_at: UtcTimestamp,
}

impl CandidateSourcePin {
    pub fn validate(&self) -> Result<(), CandidateDataError> {
        if self.entitlement_id.is_nil() {
            return Err(CandidateDataError::InvalidField {
                field: "entitlement_id".to_owned(),
                detail: "must be a nonnil UUID".to_owned(),
            });
        }
        for (field, value) in [
            ("provider", self.provider.as_str()),
            ("license_ref", self.license_ref.as_str()),
            ("dataset_id", self.dataset_id.as_str()),
            ("dataset_version", self.dataset_version.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(CandidateDataError::InvalidField {
                    field: field.to_owned(),
                    detail: "must not be empty".to_owned(),
                });
            }
        }
        if !is_sha256(&self.manifest_sha256) {
            return Err(CandidateDataError::InvalidField {
                field: "manifest_sha256".to_owned(),
                detail: "must be lowercase 64-hex".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum InvestorClass {
    Foreign,
    Institution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestorFlowObservation {
    pub instrument: InstrumentId,
    pub trade_date: TradingDate,
    pub investor_class: InvestorClass,
    pub net_amount: f64,
    pub net_volume: f64,
    pub currency: String,
    pub volume_unit: String,
    pub source_revision: String,
    pub available_at: UtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestorFlowDocument {
    pub flows: Vec<InvestorFlowObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketStatusObservation {
    pub instrument: InstrumentId,
    pub trade_date: TradingDate,
    pub suspended: bool,
    pub administrative: bool,
    pub liquidation: bool,
    pub inactive: bool,
    pub disqualifying_audit_opinion: bool,
    pub complete_capital_impairment: bool,
    pub source_revision: String,
    pub available_at: UtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketStatusDocument {
    pub statuses: Vec<MarketStatusObservation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FinancialPeriodKind {
    Quarter,
    Half,
    NineMonth,
    Annual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum StatementScope {
    Consolidated,
    Separate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundamentalObservation {
    pub instrument: InstrumentId,
    pub fiscal_period_start: TradingDate,
    pub fiscal_period_end: TradingDate,
    pub period_kind: FinancialPeriodKind,
    pub statement_scope: StatementScope,
    pub metric: String,
    pub value: f64,
    pub currency: Option<String>,
    pub unit_scale: u64,
    pub audited: Option<bool>,
    pub disclosed_at: UtcTimestamp,
    pub available_at: UtcTimestamp,
    pub source_revision: String,
    pub restates_source_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundamentalDocument {
    pub fundamentals: Vec<FundamentalObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexMembershipObservation {
    pub index_id: String,
    pub instrument: InstrumentId,
    pub announced_at: UtcTimestamp,
    pub effective_from: TradingDate,
    pub effective_until: Option<TradingDate>,
    pub available_at: UtcTimestamp,
    pub source_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexMembershipDocument {
    pub memberships: Vec<IndexMembershipObservation>,
}

impl IndexMembershipDocument {
    /// Split one provider document into canonical, deterministically ordered
    /// universe partitions.  The index id is part of each partition identity;
    /// the same instrument may therefore occur once in each universe.
    pub fn partition_by_universe(
        &self,
    ) -> Result<BTreeMap<CandidateUniverseKey, Self>, CandidateDataError> {
        let mut partitions = BTreeMap::<CandidateUniverseKey, Self>::new();
        let mut natural_keys = BTreeSet::new();
        for row in &self.memberships {
            let universe = CandidateUniverseKey::parse(&row.index_id).ok_or_else(|| {
                CandidateDataError::InvalidField {
                    field: "membership.index_id".to_owned(),
                    detail: format!("unsupported candidate universe {:?}", row.index_id),
                }
            })?;
            if !natural_keys.insert((
                row.index_id.clone(),
                row.instrument.clone(),
                row.effective_from,
                row.source_revision.clone(),
            )) {
                return Err(CandidateDataError::InvalidField {
                    field: "memberships".to_owned(),
                    detail: "contains a duplicate natural identity".to_owned(),
                });
            }
            partitions
                .entry(universe)
                .or_insert_with(|| Self {
                    memberships: Vec::new(),
                })
                .memberships
                .push(row.clone());
        }
        for document in partitions.values_mut() {
            document.memberships.sort_by(|left, right| {
                (
                    &left.instrument,
                    left.effective_from,
                    left.effective_until,
                    left.announced_at,
                    left.available_at,
                    &left.source_revision,
                )
                    .cmp(&(
                        &right.instrument,
                        right.effective_from,
                        right.effective_until,
                        right.announced_at,
                        right.available_at,
                        &right.source_revision,
                    ))
            });
        }
        Ok(partitions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorObservation {
    pub taxonomy_id: String,
    pub taxonomy_version: String,
    pub instrument: InstrumentId,
    pub sector_code: String,
    pub sector_name: String,
    pub fundamental_profile: FundamentalProfile,
    pub effective_from: TradingDate,
    pub effective_until: Option<TradingDate>,
    pub available_at: UtcTimestamp,
    pub source_revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FundamentalProfile {
    NonFinancial,
    Financial,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorDocument {
    pub sectors: Vec<SectorObservation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CandidateDocument {
    InvestorFlow(InvestorFlowDocument),
    MarketStatus(MarketStatusDocument),
    Fundamentals(FundamentalDocument),
    IndexMembership(IndexMembershipDocument),
    SectorClassification(SectorDocument),
}

#[derive(Debug, thiserror::Error)]
pub enum CandidateDataError {
    #[error("candidate source kind {0} is unsupported")]
    UnsupportedKind(ResponseKind),
    #[error("candidate source JSON for {kind} is invalid: {detail}")]
    InvalidJson { kind: ResponseKind, detail: String },
    #[error("candidate source field {field} is invalid: {detail}")]
    InvalidField { field: String, detail: String },
}

pub fn parse_candidate_envelope(
    envelope: &RawEnvelope,
) -> Result<CandidateDocument, CandidateDataError> {
    crate::validate::validate_response(envelope.kind, &envelope.bytes).map_err(|error| {
        CandidateDataError::InvalidJson {
            kind: envelope.kind,
            detail: error.reason,
        }
    })?;
    let document =
        match envelope.kind {
            ResponseKind::InvestorFlow => parse_document::<InvestorFlowDocument>(envelope)
                .map(CandidateDocument::InvestorFlow),
            ResponseKind::MarketStatus => parse_document::<MarketStatusDocument>(envelope)
                .map(CandidateDocument::MarketStatus),
            ResponseKind::Fundamentals => {
                parse_document::<FundamentalDocument>(envelope).map(CandidateDocument::Fundamentals)
            }
            ResponseKind::IndexMembership => parse_document::<IndexMembershipDocument>(envelope)
                .map(CandidateDocument::IndexMembership),
            ResponseKind::SectorClassification => parse_document::<SectorDocument>(envelope)
                .map(CandidateDocument::SectorClassification),
            other => Err(CandidateDataError::UnsupportedKind(other)),
        }?;
    validate_candidate_document(&document, envelope.retrieved_at)?;
    Ok(document)
}

/// Validate semantic source invariants before a typed document can enter a
/// curated publisher. Raw storage may still retain malformed provider bytes
/// for evidence, but no candidate observation is built from them.
pub fn validate_candidate_document(
    document: &CandidateDocument,
    retrieved_at: UtcTimestamp,
) -> Result<(), CandidateDataError> {
    match document {
        CandidateDocument::InvestorFlow(document) => {
            require_nonempty("flows", &document.flows)?;
            let mut identities = BTreeMap::new();
            for row in &document.flows {
                require_available(row.available_at, retrieved_at, "flow.available_at")?;
                require_revision(&row.source_revision)?;
                if !row.net_amount.is_finite() || !row.net_volume.is_finite() {
                    return invalid("flow.value", "must be finite");
                }
                if row.currency.len() != 3
                    || !row.currency.bytes().all(|byte| byte.is_ascii_uppercase())
                    || row.volume_unit != "SHARE"
                {
                    return invalid(
                        "flow.unit",
                        "currency must be uppercase ISO-3 and volume_unit must be SHARE",
                    );
                }
                let key = (
                    row.instrument.clone(),
                    row.trade_date,
                    row.investor_class,
                    row.source_revision.as_str(),
                );
                if identities.insert(key, ()).is_some() {
                    return invalid("flows", "contains a duplicate natural identity");
                }
            }
        }
        CandidateDocument::MarketStatus(document) => {
            require_nonempty("statuses", &document.statuses)?;
            let mut identities = BTreeMap::new();
            for row in &document.statuses {
                require_available(row.available_at, retrieved_at, "status.available_at")?;
                require_revision(&row.source_revision)?;
                let key = (
                    row.instrument.clone(),
                    row.trade_date,
                    row.source_revision.as_str(),
                );
                if identities.insert(key, ()).is_some() {
                    return invalid("statuses", "contains a duplicate natural identity");
                }
            }
        }
        CandidateDocument::Fundamentals(document) => {
            require_nonempty("fundamentals", &document.fundamentals)?;
            let mut identities = BTreeMap::new();
            for row in &document.fundamentals {
                require_available(row.available_at, retrieved_at, "fundamental.available_at")?;
                require_revision(&row.source_revision)?;
                if row.fiscal_period_end < row.fiscal_period_start
                    || row.available_at < row.disclosed_at
                    || !row.value.is_finite()
                {
                    return invalid(
                        "fundamental",
                        "period/disclosure times must be ordered and value finite",
                    );
                }
                if !canonical_id(&row.metric)
                    || !matches!(row.unit_scale, 1 | 1_000 | 1_000_000 | 1_000_000_000)
                    || row.currency.as_ref().is_some_and(|currency| {
                        currency.len() != 3
                            || !currency.bytes().all(|byte| byte.is_ascii_uppercase())
                    })
                {
                    return invalid(
                        "fundamental.metadata",
                        "metric, currency, or unit is invalid",
                    );
                }
                if row
                    .restates_source_revision
                    .as_ref()
                    .is_some_and(|revision| revision.trim().is_empty())
                {
                    return invalid("restates_source_revision", "must not be empty");
                }
                let key = (
                    row.instrument.clone(),
                    row.fiscal_period_end,
                    row.statement_scope,
                    row.metric.as_str(),
                    row.disclosed_at,
                    row.source_revision.as_str(),
                );
                if identities.insert(key, ()).is_some() {
                    return invalid("fundamentals", "contains a duplicate natural identity");
                }
            }
        }
        CandidateDocument::IndexMembership(document) => {
            require_nonempty("memberships", &document.memberships)?;
            let mut identities = BTreeMap::new();
            for row in &document.memberships {
                require_available(row.available_at, retrieved_at, "membership.available_at")?;
                require_revision(&row.source_revision)?;
                if CandidateUniverseKey::parse(&row.index_id).is_none() {
                    return invalid("membership.index_id", "unsupported candidate universe");
                }
                if !canonical_id(&row.index_id)
                    || row.available_at < row.announced_at
                    || row
                        .effective_until
                        .is_some_and(|until| until < row.effective_from)
                {
                    return invalid("membership", "identity or effective times are invalid");
                }
                let key = (
                    row.index_id.as_str(),
                    row.instrument.clone(),
                    row.effective_from,
                    row.source_revision.as_str(),
                );
                if identities.insert(key, ()).is_some() {
                    return invalid("memberships", "contains a duplicate natural identity");
                }
            }
        }
        CandidateDocument::SectorClassification(document) => {
            require_nonempty("sectors", &document.sectors)?;
            let mut identities = BTreeMap::new();
            for row in &document.sectors {
                require_available(row.available_at, retrieved_at, "sector.available_at")?;
                require_revision(&row.source_revision)?;
                if !canonical_id(&row.taxonomy_id)
                    || row.taxonomy_version.trim().is_empty()
                    || !canonical_id(&row.sector_code)
                    || row.sector_name.trim().is_empty()
                    || row
                        .effective_until
                        .is_some_and(|until| until < row.effective_from)
                {
                    return invalid("sector", "identity or effective times are invalid");
                }
                let key = (
                    row.taxonomy_id.as_str(),
                    row.taxonomy_version.as_str(),
                    row.instrument.clone(),
                    row.effective_from,
                    row.source_revision.as_str(),
                );
                if identities.insert(key, ()).is_some() {
                    return invalid("sectors", "contains a duplicate natural identity");
                }
            }
        }
    }
    Ok(())
}

fn require_nonempty<T>(field: &str, rows: &[T]) -> Result<(), CandidateDataError> {
    if rows.is_empty() {
        invalid(field, "must not be empty")
    } else {
        Ok(())
    }
}

fn require_available(
    available_at: UtcTimestamp,
    retrieved_at: UtcTimestamp,
    field: &str,
) -> Result<(), CandidateDataError> {
    if available_at > retrieved_at {
        invalid(field, "cannot follow retrieved_at")
    } else {
        Ok(())
    }
}

fn require_revision(value: &str) -> Result<(), CandidateDataError> {
    if value.trim().is_empty() || value.len() > 128 {
        invalid("source_revision", "must contain 1 to 128 characters")
    } else {
        Ok(())
    }
}

fn canonical_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-'))
}

fn invalid<T>(field: &str, detail: &str) -> Result<T, CandidateDataError> {
    Err(CandidateDataError::InvalidField {
        field: field.to_owned(),
        detail: detail.to_owned(),
    })
}

fn parse_document<T>(envelope: &RawEnvelope) -> Result<T, CandidateDataError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(&envelope.bytes).map_err(|error| CandidateDataError::InvalidJson {
        kind: envelope.kind,
        detail: error.to_string(),
    })
}

pub fn latest_flows_as_of(
    observations: &[InvestorFlowObservation],
    trade_date: TradingDate,
    cutoff: UtcTimestamp,
) -> BTreeMap<(InstrumentId, InvestorClass), InvestorFlowObservation> {
    let mut resolved = BTreeMap::new();
    for row in observations {
        if row.trade_date != trade_date || row.available_at > cutoff {
            continue;
        }
        let key = (row.instrument.clone(), row.investor_class);
        let replace = resolved
            .get(&key)
            .is_none_or(|existing: &InvestorFlowObservation| {
                (row.available_at, row.source_revision.as_str())
                    > (existing.available_at, existing.source_revision.as_str())
            });
        if replace {
            resolved.insert(key, row.clone());
        }
    }
    resolved
}

pub fn latest_fundamental_as_of<'a>(
    observations: &'a [FundamentalObservation],
    instrument: &InstrumentId,
    metric: &str,
    as_of: TradingDate,
    cutoff: UtcTimestamp,
) -> Option<&'a FundamentalObservation> {
    observations
        .iter()
        .filter(|row| {
            &row.instrument == instrument
                && row.metric == metric
                && row.fiscal_period_end <= as_of
                && row.disclosed_at <= cutoff
                && row.available_at <= cutoff
        })
        .max_by(|left, right| {
            (
                left.fiscal_period_end,
                left.disclosed_at,
                left.available_at,
                left.source_revision.as_str(),
            )
                .cmp(&(
                    right.fiscal_period_end,
                    right.disclosed_at,
                    right.available_at,
                    right.source_revision.as_str(),
                ))
        })
}

pub fn members_as_of(
    observations: &[IndexMembershipObservation],
    index_id: &str,
    as_of: TradingDate,
    cutoff: UtcTimestamp,
) -> BTreeMap<InstrumentId, IndexMembershipObservation> {
    let mut members = BTreeMap::new();
    for row in observations {
        if row.index_id != index_id
            || row.available_at > cutoff
            || row.announced_at > cutoff
            || row.effective_from > as_of
            || row.effective_until.is_some_and(|until| as_of > until)
        {
            continue;
        }
        let replace =
            members
                .get(&row.instrument)
                .is_none_or(|existing: &IndexMembershipObservation| {
                    (
                        row.effective_from,
                        row.available_at,
                        row.source_revision.as_str(),
                    ) > (
                        existing.effective_from,
                        existing.available_at,
                        existing.source_revision.as_str(),
                    )
                });
        if replace {
            members.insert(row.instrument.clone(), row.clone());
        }
    }
    members
}

pub fn sectors_as_of(
    observations: &[SectorObservation],
    taxonomy_id: &str,
    as_of: TradingDate,
    cutoff: UtcTimestamp,
) -> BTreeMap<InstrumentId, SectorObservation> {
    let mut sectors = BTreeMap::new();
    for row in observations {
        if row.taxonomy_id != taxonomy_id
            || row.available_at > cutoff
            || row.effective_from > as_of
            || row.effective_until.is_some_and(|until| as_of > until)
        {
            continue;
        }
        let replace = sectors
            .get(&row.instrument)
            .is_none_or(|existing: &SectorObservation| {
                (
                    row.effective_from,
                    row.available_at,
                    row.taxonomy_version.as_str(),
                    row.source_revision.as_str(),
                ) > (
                    existing.effective_from,
                    existing.available_at,
                    existing.taxonomy_version.as_str(),
                    existing.source_revision.as_str(),
                )
            });
        if replace {
            sectors.insert(row.instrument.clone(), row.clone());
        }
    }
    sectors
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::BatchId;

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("valid date")
    }

    fn timestamp(value: &str) -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339(value).expect("valid timestamp")
    }

    fn instrument() -> InstrumentId {
        InstrumentId::parse("005930.KRX").expect("valid instrument")
    }

    #[test]
    fn future_flow_revision_is_not_visible() {
        let base = InvestorFlowObservation {
            instrument: instrument(),
            trade_date: date("2026-08-14"),
            investor_class: InvestorClass::Foreign,
            net_amount: 10.0,
            net_volume: 2.0,
            currency: "KRW".to_owned(),
            volume_unit: "SHARE".to_owned(),
            source_revision: "1".to_owned(),
            available_at: timestamp("2026-08-14T07:00:00Z"),
        };
        let mut correction = base.clone();
        correction.net_amount = 99.0;
        correction.source_revision = "2".to_owned();
        correction.available_at = timestamp("2026-08-15T01:00:00Z");
        let rows = [base, correction];
        let resolved =
            latest_flows_as_of(&rows, date("2026-08-14"), timestamp("2026-08-14T09:00:00Z"));
        assert_eq!(
            resolved[&(instrument(), InvestorClass::Foreign)].net_amount,
            10.0
        );
    }

    #[test]
    fn future_membership_never_leaks_into_prior_universe() {
        let row = IndexMembershipObservation {
            index_id: "kospi200".to_owned(),
            instrument: instrument(),
            announced_at: timestamp("2026-08-10T00:00:00Z"),
            effective_from: date("2026-09-01"),
            effective_until: None,
            available_at: timestamp("2026-08-10T00:01:00Z"),
            source_revision: "1".to_owned(),
        };
        assert!(
            members_as_of(
                &[row],
                "kospi200",
                date("2026-08-14"),
                timestamp("2026-08-14T09:00:00Z")
            )
            .is_empty()
        );
    }

    #[test]
    fn sector_correction_is_append_only_and_resolved_by_cutoff() {
        let base = SectorObservation {
            taxonomy_id: "krx-sector".to_owned(),
            taxonomy_version: "2026-h2".to_owned(),
            instrument: instrument(),
            sector_code: "G25".to_owned(),
            sector_name: "Technology".to_owned(),
            fundamental_profile: FundamentalProfile::NonFinancial,
            effective_from: date("2026-06-12"),
            effective_until: None,
            available_at: timestamp("2026-06-01T00:05:00Z"),
            source_revision: "r1".to_owned(),
        };
        let mut correction = base.clone();
        correction.sector_code = "G45".to_owned();
        correction.sector_name = "Information Technology".to_owned();
        correction.source_revision = "r2".to_owned();
        correction.available_at = timestamp("2026-08-15T00:05:00Z");
        let rows = [base.clone(), correction.clone()];
        assert_eq!(
            sectors_as_of(
                &rows,
                "krx-sector",
                date("2026-08-14"),
                timestamp("2026-08-14T07:00:00Z")
            )[&instrument()]
                .source_revision,
            "r1"
        );
        assert_eq!(
            sectors_as_of(
                &rows,
                "krx-sector",
                date("2026-08-18"),
                timestamp("2026-08-18T07:00:00Z")
            )[&instrument()]
                .source_revision,
            "r2"
        );
        let duplicate = CandidateDocument::SectorClassification(SectorDocument {
            sectors: vec![base.clone(), base],
        });
        assert!(
            validate_candidate_document(&duplicate, timestamp("2026-08-14T07:00:00Z")).is_err()
        );
    }

    #[test]
    fn candidate_envelope_is_typed_and_strict() {
        let bytes = br#"{"flows":[{"instrument":"005930.KRX","trade_date":"2026-08-14","investor_class":"FOREIGN","net_amount":10,"net_volume":2,"currency":"KRW","volume_unit":"SHARE","source_revision":"1","available_at":"2026-08-14T07:00:00Z"}]}"#.to_vec();
        let envelope = RawEnvelope::new(
            BatchId::generate(),
            ResponseKind::InvestorFlow,
            "flow.json",
            bytes,
            timestamp("2026-08-14T07:01:00Z"),
            crate::RequestMetadata {
                endpoint: "fixture".to_owned(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: crate::FetchMode::Synthetic,
            },
        );
        let CandidateDocument::InvestorFlow(document) =
            parse_candidate_envelope(&envelope).expect("typed document")
        else {
            panic!("wrong document kind")
        };
        assert_eq!(document.flows.len(), 1);
    }

    #[test]
    fn candidate_envelope_rejects_future_availability_and_duplicate_identity() {
        let future = br#"{"flows":[{"instrument":"005930.KRX","trade_date":"2026-08-14","investor_class":"FOREIGN","net_amount":10,"net_volume":2,"currency":"KRW","volume_unit":"SHARE","source_revision":"1","available_at":"2026-08-14T08:00:00Z"}]}"#.to_vec();
        let envelope = RawEnvelope::new(
            BatchId::generate(),
            ResponseKind::InvestorFlow,
            "future-flow.json",
            future,
            timestamp("2026-08-14T07:01:00Z"),
            crate::RequestMetadata {
                endpoint: "fixture".to_owned(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: crate::FetchMode::Synthetic,
            },
        );
        assert!(matches!(
            parse_candidate_envelope(&envelope),
            Err(CandidateDataError::InvalidField { .. })
        ));

        let row = InvestorFlowObservation {
            instrument: instrument(),
            trade_date: date("2026-08-14"),
            investor_class: InvestorClass::Foreign,
            net_amount: 10.0,
            net_volume: 2.0,
            currency: "KRW".to_owned(),
            volume_unit: "SHARE".to_owned(),
            source_revision: "1".to_owned(),
            available_at: timestamp("2026-08-14T07:00:00Z"),
        };
        let duplicate = CandidateDocument::InvestorFlow(InvestorFlowDocument {
            flows: vec![row.clone(), row],
        });
        assert!(matches!(
            validate_candidate_document(&duplicate, timestamp("2026-08-14T07:01:00Z")),
            Err(CandidateDataError::InvalidField { .. })
        ));
    }
}
