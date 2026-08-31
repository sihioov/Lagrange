//! Provider-independent contracts for one owner-managed KRX instrument.
//!
//! Network access and immutable filesystem admission live in the collectors
//! crate.  This module plans the exact request budget, validates already
//! captured KIS wire evidence, and deterministically materializes one
//! per-instrument generation candidate.

use std::collections::{BTreeMap, BTreeSet};

use domain::{
    BatchId, CodeCommit, ContentHash, InstrumentId, OwnerEquityFailureCode,
    OwnerEquityUniversePolicy, RetryDisposition, TradingDate, Venue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{FetchMode, RequestMetadata, ResponseKind};

pub const OWNER_EQUITY_V2_CONTRACT_VERSION: &str = "owner-equity-v2-raw-v1";
pub const OWNER_EQUITY_V2_CANDIDATE_VERSION: &str = "owner-equity-v2-candidate-v1";
pub const OWNER_EQUITY_V2_PROVIDER_SCOPE: &str = "kis-owner-equity-v2";
pub const OWNER_EQUITY_V2_MARKET: &str = "kr";
pub const REFERENCE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
pub const REFERENCE_TR_ID: &str = "FHKST01010100";
pub const DAILY_BARS_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
pub const DAILY_BARS_TR_ID: &str = "FHKST03010100";
pub const FID_ORG_ADJ_PRC: &str = "1";
pub const MAX_CALENDAR_DAYS_PER_WINDOW: usize = 100;
pub const MAX_DAILY_WINDOWS: usize = 1_024;
pub const MAX_CAPTURE_GETS: usize = MAX_DAILY_WINDOWS + 1;
/// Initial capture uses a deterministic two-calendar-day allowance per
/// desired observed session. The resulting candidate is still admitted only
/// from verified returned sessions and is trimmed to the exact policy target.
pub const INITIAL_CALENDAR_DAYS_PER_TARGET_SESSION: u32 = 2;
pub const PRICE_SEMANTICS: &str = "FID_ORG_ADJ_PRC=1_ORIGINAL_UNADJUSTED";
pub const OWNER_ONLY_WARNING: &str = "OWNER_ONLY";
pub const VENDOR_SNAPSHOT_WARNING: &str = "VENDOR_SNAPSHOT";
pub const STRICT_PIT_WARNING: &str = "STRICT_PIT_FALSE";
pub const RESEARCH_ONLY_WARNING: &str = "PRICE_VOLUME_RESEARCH_ONLY";

const BATCH_NAMESPACE: Uuid = Uuid::from_u128(0x7b2f0e6c_1bb7_4f2c_b39e_07130542c36d);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnerEquityCaptureKind {
    Initial,
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityCaptureWindow {
    pub sequence: u32,
    pub start: TradingDate,
    pub end: TradingDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityCapturePlan {
    pub contract_version: String,
    pub kind: OwnerEquityCaptureKind,
    pub instrument_id: InstrumentId,
    pub requested_start: TradingDate,
    pub requested_end: TradingDate,
    pub target_observed_sessions: u32,
    pub minimum_observed_sessions: u32,
    pub windows: Vec<OwnerEquityCaptureWindow>,
    pub reference_gets: usize,
    pub daily_bar_gets: usize,
    pub exact_get_ceiling: usize,
}

impl OwnerEquityCapturePlan {
    pub fn initial_through(
        canonical_instrument_id: &str,
        policy: OwnerEquityUniversePolicy,
        requested_through: TradingDate,
    ) -> Result<Self, OwnerEquityV2Error> {
        let calendar_days = policy
            .target_observed_sessions()
            .checked_mul(INITIAL_CALENDAR_DAYS_PER_TARGET_SESSION)
            .ok_or(OwnerEquityV2Error::RangeOverflow)?;
        let start = requested_through
            .checked_add_days(-i64::from(calendar_days.saturating_sub(1)))
            .map_err(|_| OwnerEquityV2Error::RangeOverflow)?;
        Self::build(
            canonical_instrument_id,
            policy,
            OwnerEquityCaptureKind::Initial,
            start,
            requested_through,
        )
    }

    /// Incremental capture starts at the exact last admitted observation. That
    /// one boundary overlap is mandatory evidence: it proves that immutable
    /// prior history did not change while every later date is newly missing.
    pub fn incremental_through(
        canonical_instrument_id: &str,
        policy: OwnerEquityUniversePolicy,
        prior_last_observed: TradingDate,
        requested_through: TradingDate,
    ) -> Result<Self, OwnerEquityV2Error> {
        if requested_through <= prior_last_observed {
            return Err(OwnerEquityV2Error::IncrementalRangeEmpty);
        }
        Self::build(
            canonical_instrument_id,
            policy,
            OwnerEquityCaptureKind::Incremental,
            prior_last_observed,
            requested_through,
        )
    }

    pub fn build(
        canonical_instrument_id: &str,
        policy: OwnerEquityUniversePolicy,
        kind: OwnerEquityCaptureKind,
        requested_start: TradingDate,
        requested_end: TradingDate,
    ) -> Result<Self, OwnerEquityV2Error> {
        let instrument_id = parse_canonical_instrument(canonical_instrument_id)?;
        if requested_end < requested_start {
            return Err(OwnerEquityV2Error::RangeInvalid);
        }
        let inclusive_days = requested_end
            .as_naive_date()
            .signed_duration_since(requested_start.as_naive_date())
            .num_days()
            .checked_add(1)
            .ok_or(OwnerEquityV2Error::RangeOverflow)?;
        let inclusive_days =
            usize::try_from(inclusive_days).map_err(|_| OwnerEquityV2Error::RangeOverflow)?;
        let window_count = inclusive_days
            .checked_add(MAX_CALENDAR_DAYS_PER_WINDOW - 1)
            .ok_or(OwnerEquityV2Error::RangeOverflow)?
            / MAX_CALENDAR_DAYS_PER_WINDOW;
        if window_count == 0 || window_count > MAX_DAILY_WINDOWS {
            return Err(OwnerEquityV2Error::RequestBudgetExceeded);
        }
        let exact_get_ceiling = checked_get_ceiling(window_count)?;
        let mut windows = Vec::with_capacity(window_count);
        let mut start = requested_start;
        for index in 0..window_count {
            let remaining_days = requested_end
                .as_naive_date()
                .signed_duration_since(start.as_naive_date())
                .num_days();
            let advance = remaining_days.min((MAX_CALENDAR_DAYS_PER_WINDOW - 1) as i64);
            let end = start
                .checked_add_days(advance)
                .map_err(|_| OwnerEquityV2Error::RangeOverflow)?;
            windows.push(OwnerEquityCaptureWindow {
                sequence: u32::try_from(index + 1)
                    .map_err(|_| OwnerEquityV2Error::RequestBudgetExceeded)?,
                start,
                end,
            });
            if end == requested_end {
                break;
            }
            start = end
                .checked_add_days(1)
                .map_err(|_| OwnerEquityV2Error::RangeOverflow)?;
        }
        let plan = Self {
            contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
            kind,
            instrument_id,
            requested_start,
            requested_end,
            target_observed_sessions: policy.target_observed_sessions(),
            minimum_observed_sessions: policy.minimum_observed_sessions(),
            windows,
            reference_gets: 1,
            daily_bar_gets: window_count,
            exact_get_ceiling,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), OwnerEquityV2Error> {
        parse_canonical_instrument(&self.instrument_id.to_string())?;
        if self.contract_version != OWNER_EQUITY_V2_CONTRACT_VERSION
            || self.requested_end < self.requested_start
            || self.minimum_observed_sessions < domain::MINIMUM_OBSERVED_SESSIONS
            || self.target_observed_sessions < self.minimum_observed_sessions
            || self.reference_gets != 1
            || self.windows.is_empty()
            || self.windows.len() > MAX_DAILY_WINDOWS
            || self.daily_bar_gets != self.windows.len()
            || self.exact_get_ceiling != checked_get_ceiling(self.windows.len())?
        {
            return Err(OwnerEquityV2Error::PlanInvalid);
        }
        let mut expected_start = self.requested_start;
        for (index, window) in self.windows.iter().enumerate() {
            let days = window
                .end
                .as_naive_date()
                .signed_duration_since(window.start.as_naive_date())
                .num_days()
                .checked_add(1)
                .ok_or(OwnerEquityV2Error::RangeOverflow)?;
            if window.sequence as usize != index + 1
                || window.start != expected_start
                || window.end < window.start
                || days > MAX_CALENDAR_DAYS_PER_WINDOW as i64
            {
                return Err(OwnerEquityV2Error::PlanInvalid);
            }
            expected_start = window
                .end
                .checked_add_days(1)
                .map_err(|_| OwnerEquityV2Error::RangeOverflow)?;
        }
        if self.windows.last().map(|window| window.end) != Some(self.requested_end) {
            return Err(OwnerEquityV2Error::PlanInvalid);
        }
        Ok(())
    }
}

pub fn checked_get_ceiling(daily_windows: usize) -> Result<usize, OwnerEquityV2Error> {
    let gets = daily_windows
        .checked_add(1)
        .ok_or(OwnerEquityV2Error::RequestBudgetExceeded)?;
    if daily_windows == 0 || daily_windows > MAX_DAILY_WINDOWS || gets > MAX_CAPTURE_GETS {
        Err(OwnerEquityV2Error::RequestBudgetExceeded)
    } else {
        Ok(gets)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityCaptureIdentity {
    pub plan: OwnerEquityCapturePlan,
    pub entitlement_reference: String,
    pub entitlement_sha256: ContentHash,
    pub capture_code_commit: CodeCommit,
    pub reference_path: String,
    pub reference_tr_id: String,
    pub daily_bars_path: String,
    pub daily_bars_tr_id: String,
    pub fid_org_adj_prc: String,
}

impl OwnerEquityCaptureIdentity {
    pub fn new(
        plan: OwnerEquityCapturePlan,
        entitlement_reference: impl Into<String>,
        entitlement_sha256: ContentHash,
        capture_code_commit: CodeCommit,
    ) -> Result<Self, OwnerEquityV2Error> {
        let identity = Self {
            plan,
            entitlement_reference: entitlement_reference.into(),
            entitlement_sha256,
            capture_code_commit,
            reference_path: REFERENCE_PATH.to_owned(),
            reference_tr_id: REFERENCE_TR_ID.to_owned(),
            daily_bars_path: DAILY_BARS_PATH.to_owned(),
            daily_bars_tr_id: DAILY_BARS_TR_ID.to_owned(),
            fid_org_adj_prc: FID_ORG_ADJ_PRC.to_owned(),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), OwnerEquityV2Error> {
        self.plan.validate()?;
        if self.entitlement_reference.is_empty()
            || self.entitlement_reference.len() > 512
            || self.entitlement_reference.chars().any(char::is_control)
            || self.reference_path != REFERENCE_PATH
            || self.reference_tr_id != REFERENCE_TR_ID
            || self.daily_bars_path != DAILY_BARS_PATH
            || self.daily_bars_tr_id != DAILY_BARS_TR_ID
            || self.fid_org_adj_prc != FID_ORG_ADJ_PRC
        {
            return Err(OwnerEquityV2Error::IdentityInvalid);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OwnerEquityV2Error> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| OwnerEquityV2Error::CanonicalizationFailed)
    }

    pub fn identity_sha256(&self) -> Result<ContentHash, OwnerEquityV2Error> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }

    pub fn batch_id(&self) -> Result<BatchId, OwnerEquityV2Error> {
        Ok(BatchId::from_uuid(Uuid::new_v5(
            &BATCH_NAMESPACE,
            &self.canonical_bytes()?,
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityRawFile {
    pub kind: ResponseKind,
    pub file_name: String,
    pub content_hash: ContentHash,
    pub request: RequestMetadata,
    pub response_continuation: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityRawEvidence {
    pub batch_id: BatchId,
    pub raw_manifest_sha256: ContentHash,
    pub batch_json_sha256: ContentHash,
    pub files: Vec<OwnerEquityRawFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityBar {
    pub session_date: TradingDate,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityRawFilePin {
    pub kind: ResponseKind,
    pub window_sequence: Option<u32>,
    pub file_name: String,
    pub sha256: ContentHash,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquitySourcePins {
    pub capture_identity_sha256: ContentHash,
    pub raw_batch_id: BatchId,
    pub raw_manifest_sha256: ContentHash,
    pub batch_json_sha256: ContentHash,
    pub entitlement_reference: String,
    pub entitlement_sha256: ContentHash,
    pub capture_code_commit: CodeCommit,
    pub materializer_code_commit: CodeCommit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_candidate_sha256: Option<ContentHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_artifact_manifest_sha256: Option<ContentHash>,
    pub files: Vec<OwnerEquityRawFilePin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityGenerationCandidate {
    pub candidate_version: String,
    pub contract_version: String,
    pub capture_kind: OwnerEquityCaptureKind,
    pub instrument_id: InstrumentId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub requested_start: TradingDate,
    pub requested_end: TradingDate,
    pub target_observed_sessions: u32,
    pub minimum_observed_sessions: u32,
    pub observed_sessions: u32,
    pub first_observed_date: TradingDate,
    pub last_observed_date: TradingDate,
    pub bars: Vec<OwnerEquityBar>,
    pub source_pins: OwnerEquitySourcePins,
    pub price_semantics: String,
    pub owner_only: bool,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub warnings: Vec<String>,
    pub claims_not_made: Vec<String>,
}

impl OwnerEquityGenerationCandidate {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OwnerEquityV2Error> {
        serde_json::to_vec(self).map_err(|_| OwnerEquityV2Error::CanonicalizationFailed)
    }

    pub fn content_sha256(&self) -> Result<ContentHash, OwnerEquityV2Error> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }
}

pub fn materialize_owner_equity_candidate(
    identity: &OwnerEquityCaptureIdentity,
    evidence: &OwnerEquityRawEvidence,
    materializer_code_commit: CodeCommit,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityV2Error> {
    materialize_owner_equity_candidate_inner(identity, evidence, materializer_code_commit, true)
}

pub fn materialize_owner_equity_candidate_allow_insufficient(
    identity: &OwnerEquityCaptureIdentity,
    evidence: &OwnerEquityRawEvidence,
    materializer_code_commit: CodeCommit,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityV2Error> {
    materialize_owner_equity_candidate_inner(identity, evidence, materializer_code_commit, false)
}

fn materialize_owner_equity_candidate_inner(
    identity: &OwnerEquityCaptureIdentity,
    evidence: &OwnerEquityRawEvidence,
    materializer_code_commit: CodeCommit,
    enforce_initial_minimum: bool,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityV2Error> {
    identity.validate()?;
    if evidence.batch_id != identity.batch_id()? {
        return Err(OwnerEquityV2Error::EvidenceIdentityMismatch);
    }
    let validated = validate_owner_equity_raw_evidence(identity, evidence)?;
    let mut bars = validated.bars;
    if bars.len() > identity.plan.target_observed_sessions as usize {
        bars = bars.split_off(bars.len() - identity.plan.target_observed_sessions as usize);
    }
    let observed_sessions =
        u32::try_from(bars.len()).map_err(|_| OwnerEquityV2Error::CoverageOverflow)?;
    if enforce_initial_minimum
        && identity.plan.kind == OwnerEquityCaptureKind::Initial
        && observed_sessions < identity.plan.minimum_observed_sessions
    {
        return Err(OwnerEquityV2Error::InsufficientHistory);
    }
    let first_observed_date = bars
        .first()
        .map(|bar| bar.session_date)
        .ok_or(OwnerEquityV2Error::EvidenceMissing)?;
    let last_observed_date = bars
        .last()
        .map(|bar| bar.session_date)
        .ok_or(OwnerEquityV2Error::EvidenceMissing)?;
    Ok(OwnerEquityGenerationCandidate {
        candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
        contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
        capture_kind: identity.plan.kind,
        instrument_id: identity.plan.instrument_id.clone(),
        display_name: validated.display_name,
        requested_start: identity.plan.requested_start,
        requested_end: identity.plan.requested_end,
        target_observed_sessions: identity.plan.target_observed_sessions,
        minimum_observed_sessions: identity.plan.minimum_observed_sessions,
        observed_sessions,
        first_observed_date,
        last_observed_date,
        bars,
        source_pins: OwnerEquitySourcePins {
            capture_identity_sha256: identity.identity_sha256()?,
            raw_batch_id: evidence.batch_id,
            raw_manifest_sha256: evidence.raw_manifest_sha256.clone(),
            batch_json_sha256: evidence.batch_json_sha256.clone(),
            entitlement_reference: identity.entitlement_reference.clone(),
            entitlement_sha256: identity.entitlement_sha256.clone(),
            capture_code_commit: identity.capture_code_commit.clone(),
            materializer_code_commit,
            prior_candidate_sha256: None,
            prior_artifact_manifest_sha256: None,
            files: validated.file_pins,
        },
        price_semantics: PRICE_SEMANTICS.to_owned(),
        owner_only: true,
        vendor_snapshot: true,
        strict_pit: false,
        warnings: vec![
            OWNER_ONLY_WARNING.to_owned(),
            VENDOR_SNAPSHOT_WARNING.to_owned(),
            STRICT_PIT_WARNING.to_owned(),
            RESEARCH_ONLY_WARNING.to_owned(),
        ],
        claims_not_made: vec![
            "ADJUSTED_RETURN".to_owned(),
            "COMMON_SHARE_TYPE".to_owned(),
            "EXCHANGE_CALENDAR".to_owned(),
            "INDEX_MEMBERSHIP".to_owned(),
            "LISTING_STATUS".to_owned(),
            "STRICT_POINT_IN_TIME".to_owned(),
        ],
    })
}

/// Merge a small, verified incremental capture with the exact previously
/// admitted immutable candidate. Duplicate boundary observations must be byte-
/// for-byte equal; a changed overlap is evidence conflict, never a rewrite.
pub fn merge_owner_equity_incremental_candidate(
    prior: &OwnerEquityGenerationCandidate,
    prior_artifact_manifest_sha256: ContentHash,
    incremental: &OwnerEquityGenerationCandidate,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityV2Error> {
    if incremental.capture_kind != OwnerEquityCaptureKind::Incremental
        || prior.instrument_id != incremental.instrument_id
        || prior.target_observed_sessions != incremental.target_observed_sessions
        || prior.minimum_observed_sessions != incremental.minimum_observed_sessions
        || prior.price_semantics != PRICE_SEMANTICS
        || incremental.price_semantics != PRICE_SEMANTICS
        || !prior.owner_only
        || !incremental.owner_only
        || prior.strict_pit
        || incremental.strict_pit
        || incremental.requested_start != prior.last_observed_date
        || incremental.requested_end <= prior.last_observed_date
    {
        return Err(OwnerEquityV2Error::IncrementalContractMismatch);
    }
    if let (Some(prior_name), Some(next_name)) = (&prior.display_name, &incremental.display_name)
        && prior_name != next_name
    {
        return Err(OwnerEquityV2Error::IncrementalContractMismatch);
    }
    let mut merged = BTreeMap::<TradingDate, OwnerEquityBar>::new();
    for bar in &prior.bars {
        if merged.insert(bar.session_date, bar.clone()).is_some() {
            return Err(OwnerEquityV2Error::DuplicateObservation);
        }
    }
    let mut overlap = false;
    for bar in &incremental.bars {
        match merged.get(&bar.session_date) {
            Some(existing) if existing == bar => overlap = true,
            Some(_) => return Err(OwnerEquityV2Error::IncrementalOverlapChanged),
            None => {
                merged.insert(bar.session_date, bar.clone());
            }
        }
    }
    if !overlap {
        return Err(OwnerEquityV2Error::IncrementalOverlapMissing);
    }
    let target = incremental.target_observed_sessions as usize;
    let mut bars = merged.into_values().collect::<Vec<_>>();
    if bars.len() > target {
        bars = bars.split_off(bars.len() - target);
    }
    if bars.len() < incremental.minimum_observed_sessions as usize {
        return Err(OwnerEquityV2Error::InsufficientHistory);
    }
    let first_observed_date = bars
        .first()
        .map(|bar| bar.session_date)
        .ok_or(OwnerEquityV2Error::EvidenceMissing)?;
    let last_observed_date = bars
        .last()
        .map(|bar| bar.session_date)
        .ok_or(OwnerEquityV2Error::EvidenceMissing)?;
    let mut result = incremental.clone();
    result.display_name = incremental
        .display_name
        .clone()
        .or_else(|| prior.display_name.clone());
    result.requested_start = prior.requested_start;
    result.observed_sessions =
        u32::try_from(bars.len()).map_err(|_| OwnerEquityV2Error::CoverageOverflow)?;
    result.first_observed_date = first_observed_date;
    result.last_observed_date = last_observed_date;
    result.bars = bars;
    result.source_pins.prior_candidate_sha256 = Some(prior.content_sha256()?);
    result.source_pins.prior_artifact_manifest_sha256 = Some(prior_artifact_manifest_sha256);
    Ok(result)
}

pub fn verify_owner_equity_candidate(
    identity: &OwnerEquityCaptureIdentity,
    evidence: &OwnerEquityRawEvidence,
    materializer_code_commit: CodeCommit,
    expected_bytes: &[u8],
    expected_sha256: &ContentHash,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityV2Error> {
    if ContentHash::from_bytes(expected_bytes) != *expected_sha256 {
        return Err(OwnerEquityV2Error::CandidateMismatch);
    }
    let expected: OwnerEquityGenerationCandidate = serde_json::from_slice(expected_bytes)
        .map_err(|_| OwnerEquityV2Error::CandidateMismatch)?;
    let actual = materialize_owner_equity_candidate(identity, evidence, materializer_code_commit)?;
    let actual_bytes = actual.canonical_bytes()?;
    if actual_bytes != expected_bytes || actual != expected {
        return Err(OwnerEquityV2Error::CandidateMismatch);
    }
    Ok(actual)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOwnerEquityEvidence {
    pub display_name: Option<String>,
    pub bars: Vec<OwnerEquityBar>,
    pub file_pins: Vec<OwnerEquityRawFilePin>,
}

pub fn validate_owner_equity_raw_evidence(
    identity: &OwnerEquityCaptureIdentity,
    evidence: &OwnerEquityRawEvidence,
) -> Result<ValidatedOwnerEquityEvidence, OwnerEquityV2Error> {
    let expected_count = identity.plan.exact_get_ceiling;
    if evidence.files.len() != expected_count {
        return Err(OwnerEquityV2Error::EvidenceMissing);
    }
    let symbol = identity.plan.instrument_id.symbol();
    let mut reference = None;
    let mut daily = BTreeMap::<u32, &OwnerEquityRawFile>::new();
    for file in &evidence.files {
        if file.content_hash != ContentHash::from_bytes(&file.bytes) {
            return Err(OwnerEquityV2Error::RawTamper);
        }
        match file.kind {
            ResponseKind::Reference => {
                if reference.replace(file).is_some() {
                    return Err(OwnerEquityV2Error::EvidenceUnexpected);
                }
            }
            ResponseKind::Bars => {
                let sequence = daily_window_sequence(file, symbol)?;
                if daily.insert(sequence, file).is_some() {
                    return Err(OwnerEquityV2Error::EvidenceUnexpected);
                }
            }
            _ => return Err(OwnerEquityV2Error::EvidenceUnexpected),
        }
    }
    let reference = reference.ok_or(OwnerEquityV2Error::EvidenceMissing)?;
    validate_reference_file(reference, symbol)?;
    if daily.len() != identity.plan.windows.len() {
        return Err(OwnerEquityV2Error::EvidenceMissing);
    }
    let mut seen_bytes = BTreeSet::new();
    let mut bars = BTreeMap::<TradingDate, OwnerEquityBar>::new();
    let mut display_name = None;
    let mut file_pins = vec![file_pin(reference, None)?];
    for window in &identity.plan.windows {
        let file = daily
            .get(&window.sequence)
            .copied()
            .ok_or(OwnerEquityV2Error::EvidenceMissing)?;
        validate_daily_request(file, symbol, window)?;
        if !seen_bytes.insert(file.content_hash.clone()) {
            return Err(OwnerEquityV2Error::RepeatedResponseBytes);
        }
        let parsed = parse_daily_body(file, symbol, window)?;
        if parsed.bars.is_empty() {
            return Err(OwnerEquityV2Error::EvidenceMissing);
        }
        if let Some(name) = parsed.display_name {
            match &display_name {
                Some(previous) if previous != &name => {
                    return Err(OwnerEquityV2Error::DisplayNameInvalid);
                }
                None => display_name = Some(name),
                _ => {}
            }
        }
        for bar in parsed.bars {
            if bars.insert(bar.session_date, bar).is_some() {
                return Err(OwnerEquityV2Error::DuplicateObservation);
            }
        }
        file_pins.push(file_pin(file, Some(window.sequence))?);
    }
    file_pins.sort_by(|left, right| {
        (left.kind, left.window_sequence, &left.file_name).cmp(&(
            right.kind,
            right.window_sequence,
            &right.file_name,
        ))
    });
    Ok(ValidatedOwnerEquityEvidence {
        display_name,
        bars: bars.into_values().collect(),
        file_pins,
    })
}

fn validate_reference_file(
    file: &OwnerEquityRawFile,
    symbol: &str,
) -> Result<(), OwnerEquityV2Error> {
    let expected_query = vec![
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
        ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
    ];
    validate_request_metadata(file, REFERENCE_PATH, REFERENCE_TR_ID, &expected_query)?;
    if file.file_name != format!("reference-{symbol}-page-01.json") {
        return Err(OwnerEquityV2Error::RequestContractMismatch);
    }
    let object = response_object(&file.bytes)?;
    require_success(&object)?;
    reject_body_cursor(&object)?;
    let returned = object
        .get("output")
        .and_then(Value::as_object)
        .and_then(|output| output.get("stck_shrn_iscd"))
        .and_then(Value::as_str)
        .ok_or(OwnerEquityV2Error::ResponseSchemaInvalid)?;
    if returned != symbol {
        return Err(OwnerEquityV2Error::SymbolMismatch);
    }
    Ok(())
}

fn validate_daily_request(
    file: &OwnerEquityRawFile,
    symbol: &str,
    window: &OwnerEquityCaptureWindow,
) -> Result<(), OwnerEquityV2Error> {
    let expected_query = vec![
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
        ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
        (
            "FID_INPUT_DATE_1".to_owned(),
            window.start.to_iso().replace('-', ""),
        ),
        (
            "FID_INPUT_DATE_2".to_owned(),
            window.end.to_iso().replace('-', ""),
        ),
        ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
        ("FID_ORG_ADJ_PRC".to_owned(), FID_ORG_ADJ_PRC.to_owned()),
    ];
    validate_request_metadata(file, DAILY_BARS_PATH, DAILY_BARS_TR_ID, &expected_query)
}

fn validate_request_metadata(
    file: &OwnerEquityRawFile,
    path: &str,
    tr_id: &str,
    expected_query: &[(String, String)],
) -> Result<(), OwnerEquityV2Error> {
    let expected_headers = vec![
        ("authorization".to_owned(), "[REDACTED]".to_owned()),
        ("appkey".to_owned(), "[REDACTED]".to_owned()),
        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
        ("tr_id".to_owned(), tr_id.to_owned()),
        ("tr_cont".to_owned(), String::new()),
    ];
    if file.request.endpoint != path
        || file.request.query != expected_query
        || file.request.headers != expected_headers
        || file.request.mode != FetchMode::Credentialed
        || file
            .response_continuation
            .as_deref()
            .is_some_and(|marker| !marker.is_empty())
    {
        return Err(OwnerEquityV2Error::RequestContractMismatch);
    }
    Ok(())
}

fn daily_window_sequence(
    file: &OwnerEquityRawFile,
    symbol: &str,
) -> Result<u32, OwnerEquityV2Error> {
    let prefix = "daily-bars-window-";
    let suffix = format!("-{symbol}-page-01.json");
    let middle = file
        .file_name
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(&suffix))
        .ok_or(OwnerEquityV2Error::RequestContractMismatch)?;
    if middle.len() != 4 || !middle.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OwnerEquityV2Error::RequestContractMismatch);
    }
    middle
        .parse()
        .map_err(|_| OwnerEquityV2Error::RequestContractMismatch)
}

struct ParsedDailyBody {
    display_name: Option<String>,
    bars: Vec<OwnerEquityBar>,
}

fn parse_daily_body(
    file: &OwnerEquityRawFile,
    symbol: &str,
    window: &OwnerEquityCaptureWindow,
) -> Result<ParsedDailyBody, OwnerEquityV2Error> {
    let object = response_object(&file.bytes)?;
    require_success(&object)?;
    reject_body_cursor(&object)?;
    let output1 = object
        .get("output1")
        .and_then(Value::as_object)
        .ok_or(OwnerEquityV2Error::ResponseSchemaInvalid)?;
    if output1.get("stck_shrn_iscd").and_then(Value::as_str) != Some(symbol) {
        return Err(OwnerEquityV2Error::SymbolMismatch);
    }
    let display_name = output1
        .get("hts_kor_isnm")
        .map(|value| {
            let name = value
                .as_str()
                .ok_or(OwnerEquityV2Error::DisplayNameInvalid)?;
            if name.is_empty()
                || name.trim() != name
                || name.chars().count() > 120
                || name.chars().any(char::is_control)
            {
                return Err(OwnerEquityV2Error::DisplayNameInvalid);
            }
            Ok(name.to_owned())
        })
        .transpose()?;
    let rows = object
        .get("output2")
        .and_then(Value::as_array)
        .ok_or(OwnerEquityV2Error::ResponseSchemaInvalid)?;
    if rows.len() > MAX_CALENDAR_DAYS_PER_WINDOW {
        return Err(OwnerEquityV2Error::ResponseSchemaInvalid);
    }
    let mut previous = None;
    let mut bars = Vec::with_capacity(rows.len());
    for row in rows {
        let row = row
            .as_object()
            .ok_or(OwnerEquityV2Error::ResponseSchemaInvalid)?;
        let date = parse_kis_date(required_text(row, "stck_bsop_date")?)?;
        if date < window.start || date > window.end {
            return Err(OwnerEquityV2Error::DateOutsideRange);
        }
        if previous.is_some_and(|prior| date >= prior) {
            return Err(OwnerEquityV2Error::DuplicateObservation);
        }
        previous = Some(date);
        let open = positive_integer(row, "stck_oprc")?;
        let high = positive_integer(row, "stck_hgpr")?;
        let low = positive_integer(row, "stck_lwpr")?;
        let close = positive_integer(row, "stck_clpr")?;
        let volume = nonnegative_integer(row, "acml_vol")?;
        if low > high || open < low || open > high || close < low || close > high {
            return Err(OwnerEquityV2Error::OhlcvInvalid);
        }
        bars.push(OwnerEquityBar {
            session_date: date,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(ParsedDailyBody { display_name, bars })
}

fn response_object(bytes: &[u8]) -> Result<serde_json::Map<String, Value>, OwnerEquityV2Error> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| OwnerEquityV2Error::MalformedJson)?;
    value
        .as_object()
        .cloned()
        .ok_or(OwnerEquityV2Error::ResponseSchemaInvalid)
}

fn require_success(object: &serde_json::Map<String, Value>) -> Result<(), OwnerEquityV2Error> {
    if object.get("rt_cd").and_then(Value::as_str) == Some("0") {
        Ok(())
    } else {
        Err(OwnerEquityV2Error::ResponseStatusInvalid)
    }
}

fn reject_body_cursor(object: &serde_json::Map<String, Value>) -> Result<(), OwnerEquityV2Error> {
    let present = object.iter().any(|(key, value)| {
        let key = key.to_ascii_lowercase();
        let cursor = key.contains("ctx")
            || key.contains("cts")
            || key.contains("continu")
            || key == "next"
            || key == "has_more"
            || key == "more";
        cursor
            && match value {
                Value::Null => false,
                Value::String(text) => !text.is_empty(),
                Value::Bool(value) => *value,
                Value::Number(value) => value.as_u64().is_none_or(|number| number != 0),
                Value::Array(value) => !value.is_empty(),
                Value::Object(value) => !value.is_empty(),
            }
    });
    if present {
        Err(OwnerEquityV2Error::ContinuationNonblank)
    } else {
        Ok(())
    }
}

fn required_text<'a>(
    row: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, OwnerEquityV2Error> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or(OwnerEquityV2Error::OhlcvInvalid)
}

fn positive_integer(
    row: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, OwnerEquityV2Error> {
    let value = required_text(row, field)?
        .parse::<u64>()
        .map_err(|_| OwnerEquityV2Error::OhlcvInvalid)?;
    if value == 0 {
        Err(OwnerEquityV2Error::OhlcvInvalid)
    } else {
        Ok(value)
    }
}

fn nonnegative_integer(
    row: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, OwnerEquityV2Error> {
    required_text(row, field)?
        .parse::<u64>()
        .map_err(|_| OwnerEquityV2Error::OhlcvInvalid)
}

fn parse_kis_date(value: &str) -> Result<TradingDate, OwnerEquityV2Error> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OwnerEquityV2Error::DateInvalid);
    }
    TradingDate::parse(&format!(
        "{}-{}-{}",
        &value[..4],
        &value[4..6],
        &value[6..8]
    ))
    .map_err(|_| OwnerEquityV2Error::DateInvalid)
}

fn file_pin(
    file: &OwnerEquityRawFile,
    window_sequence: Option<u32>,
) -> Result<OwnerEquityRawFilePin, OwnerEquityV2Error> {
    Ok(OwnerEquityRawFilePin {
        kind: file.kind,
        window_sequence,
        file_name: file.file_name.clone(),
        sha256: file.content_hash.clone(),
        size_bytes: u64::try_from(file.bytes.len())
            .map_err(|_| OwnerEquityV2Error::CoverageOverflow)?,
    })
}

fn parse_canonical_instrument(value: &str) -> Result<InstrumentId, OwnerEquityV2Error> {
    if value.len() != 10
        || !value.ends_with(".KRX")
        || !value.as_bytes()[..6]
            .iter()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(OwnerEquityV2Error::InstrumentInvalid);
    }
    let instrument =
        InstrumentId::parse(value).map_err(|_| OwnerEquityV2Error::InstrumentInvalid)?;
    if instrument.venue() != Venue::Krx || instrument.symbol().len() != 6 {
        return Err(OwnerEquityV2Error::InstrumentInvalid);
    }
    Ok(instrument)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OwnerEquityV2Error {
    #[error("owner equity instrument is invalid")]
    InstrumentInvalid,
    #[error("owner equity requested range is invalid")]
    RangeInvalid,
    #[error("owner equity requested range overflowed")]
    RangeOverflow,
    #[error("owner equity request budget was exceeded")]
    RequestBudgetExceeded,
    #[error("owner equity capture plan is invalid")]
    PlanInvalid,
    #[error("owner equity capture identity is invalid")]
    IdentityInvalid,
    #[error("owner equity evidence identity differs")]
    EvidenceIdentityMismatch,
    #[error("owner equity evidence is missing")]
    EvidenceMissing,
    #[error("owner equity evidence is unexpected")]
    EvidenceUnexpected,
    #[error("owner equity request contract differs")]
    RequestContractMismatch,
    #[error("owner equity response is malformed JSON")]
    MalformedJson,
    #[error("owner equity response status is invalid")]
    ResponseStatusInvalid,
    #[error("owner equity response schema is invalid")]
    ResponseSchemaInvalid,
    #[error("owner equity response symbol differs")]
    SymbolMismatch,
    #[error("owner equity response date is invalid")]
    DateInvalid,
    #[error("owner equity response date is outside the requested range")]
    DateOutsideRange,
    #[error("owner equity continuation is nonblank")]
    ContinuationNonblank,
    #[error("owner equity response contains duplicate observations")]
    DuplicateObservation,
    #[error("owner equity response bytes repeat")]
    RepeatedResponseBytes,
    #[error("owner equity OHLCV is invalid")]
    OhlcvInvalid,
    #[error("owner equity display name is invalid")]
    DisplayNameInvalid,
    #[error("owner equity Raw evidence was tampered")]
    RawTamper,
    #[error("owner equity history is below the admissible minimum")]
    InsufficientHistory,
    #[error("owner equity coverage overflowed")]
    CoverageOverflow,
    #[error("owner equity candidate differs from exact inputs")]
    CandidateMismatch,
    #[error("owner equity incremental range has no missing date")]
    IncrementalRangeEmpty,
    #[error("owner equity incremental contract differs from admitted history")]
    IncrementalContractMismatch,
    #[error("owner equity incremental overlap is missing")]
    IncrementalOverlapMissing,
    #[error("owner equity incremental overlap changed")]
    IncrementalOverlapChanged,
    #[error("owner equity canonicalization failed")]
    CanonicalizationFailed,
}

impl OwnerEquityV2Error {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InstrumentInvalid => "INSTRUMENT_INVALID",
            Self::RangeInvalid => "RANGE_INVALID",
            Self::RangeOverflow => "RANGE_OVERFLOW",
            Self::RequestBudgetExceeded => "REQUEST_BUDGET_EXCEEDED",
            Self::PlanInvalid => "PLAN_INVALID",
            Self::IdentityInvalid => "IDENTITY_INVALID",
            Self::EvidenceIdentityMismatch => "EVIDENCE_IDENTITY_MISMATCH",
            Self::EvidenceMissing => "EVIDENCE_MISSING",
            Self::EvidenceUnexpected => "EVIDENCE_UNEXPECTED",
            Self::RequestContractMismatch => "REQUEST_CONTRACT_MISMATCH",
            Self::MalformedJson => "RESPONSE_MALFORMED_JSON",
            Self::ResponseStatusInvalid => "RESPONSE_STATUS_INVALID",
            Self::ResponseSchemaInvalid => "RESPONSE_SCHEMA_INVALID",
            Self::SymbolMismatch => "RESPONSE_SYMBOL_MISMATCH",
            Self::DateInvalid => "RESPONSE_DATE_INVALID",
            Self::DateOutsideRange => "RESPONSE_DATE_OUT_OF_RANGE",
            Self::ContinuationNonblank => "CONTINUATION_NONBLANK",
            Self::DuplicateObservation => "OBSERVATION_DUPLICATE",
            Self::RepeatedResponseBytes => "RESPONSE_BYTES_REPEATED",
            Self::OhlcvInvalid => "OHLCV_INVALID",
            Self::DisplayNameInvalid => "DISPLAY_NAME_INVALID",
            Self::RawTamper => "RAW_TAMPERED",
            Self::InsufficientHistory => "INSUFFICIENT_HISTORY",
            Self::CoverageOverflow => "COVERAGE_OVERFLOW",
            Self::CandidateMismatch => "CANDIDATE_MISMATCH",
            Self::IncrementalRangeEmpty => "INCREMENTAL_RANGE_EMPTY",
            Self::IncrementalContractMismatch => "INCREMENTAL_CONTRACT_MISMATCH",
            Self::IncrementalOverlapMissing => "INCREMENTAL_OVERLAP_MISSING",
            Self::IncrementalOverlapChanged => "INCREMENTAL_OVERLAP_CHANGED",
            Self::CanonicalizationFailed => "CANONICALIZATION_FAILED",
        }
    }

    pub const fn retry_disposition(self) -> RetryDisposition {
        RetryDisposition::Terminal
    }

    pub fn failure_code(self) -> OwnerEquityFailureCode {
        OwnerEquityFailureCode::parse(self.code()).expect("static failure code is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CAPTURE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const MATERIALIZER_COMMIT: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn plan(start: &str, end: &str) -> OwnerEquityCapturePlan {
        OwnerEquityCapturePlan::build(
            "005930.KRX",
            OwnerEquityUniversePolicy::default(),
            OwnerEquityCaptureKind::Initial,
            TradingDate::parse(start).unwrap(),
            TradingDate::parse(end).unwrap(),
        )
        .unwrap()
    }

    fn identity(start: &str, end: &str) -> OwnerEquityCaptureIdentity {
        OwnerEquityCaptureIdentity::new(
            plan(start, end),
            "vault://entitlements/kis-owner-equity-v2",
            ContentHash::from_bytes(b"entitlement"),
            CodeCommit::parse(CAPTURE_COMMIT).unwrap(),
        )
        .unwrap()
    }

    fn headers(tr_id: &str) -> Vec<(String, String)> {
        vec![
            ("authorization".into(), "[REDACTED]".into()),
            ("appkey".into(), "[REDACTED]".into()),
            ("appsecret".into(), "[REDACTED]".into()),
            ("tr_id".into(), tr_id.into()),
            ("tr_cont".into(), String::new()),
        ]
    }

    fn reference(identity: &OwnerEquityCaptureIdentity) -> OwnerEquityRawFile {
        let symbol = identity.plan.instrument_id.symbol();
        raw_file(
            ResponseKind::Reference,
            format!("reference-{symbol}-page-01.json"),
            RequestMetadata {
                endpoint: REFERENCE_PATH.into(),
                query: vec![
                    ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
                    ("FID_INPUT_ISCD".into(), symbol.into()),
                ],
                headers: headers(REFERENCE_TR_ID),
                mode: FetchMode::Credentialed,
            },
            serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output": {"stck_shrn_iscd": symbol}
            }))
            .unwrap(),
        )
    }

    fn daily(
        identity: &OwnerEquityCaptureIdentity,
        window: &OwnerEquityCaptureWindow,
        dates: &[TradingDate],
    ) -> OwnerEquityRawFile {
        let symbol = identity.plan.instrument_id.symbol();
        let rows = dates
            .iter()
            .rev()
            .map(|date| {
                json!({
                    "stck_bsop_date": date.to_iso().replace('-', ""),
                    "stck_oprc": "100",
                    "stck_hgpr": "105",
                    "stck_lwpr": "95",
                    "stck_clpr": "101",
                    "acml_vol": "1000"
                })
            })
            .collect::<Vec<_>>();
        raw_file(
            ResponseKind::Bars,
            format!(
                "daily-bars-window-{:04}-{symbol}-page-01.json",
                window.sequence
            ),
            RequestMetadata {
                endpoint: DAILY_BARS_PATH.into(),
                query: vec![
                    ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
                    ("FID_INPUT_ISCD".into(), symbol.into()),
                    (
                        "FID_INPUT_DATE_1".into(),
                        window.start.to_iso().replace('-', ""),
                    ),
                    (
                        "FID_INPUT_DATE_2".into(),
                        window.end.to_iso().replace('-', ""),
                    ),
                    ("FID_PERIOD_DIV_CODE".into(), "D".into()),
                    ("FID_ORG_ADJ_PRC".into(), FID_ORG_ADJ_PRC.into()),
                ],
                headers: headers(DAILY_BARS_TR_ID),
                mode: FetchMode::Credentialed,
            },
            serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output1": {
                    "stck_shrn_iscd": symbol,
                    "hts_kor_isnm": "삼성전자"
                },
                "output2": rows
            }))
            .unwrap(),
        )
    }

    fn raw_file(
        kind: ResponseKind,
        file_name: String,
        request: RequestMetadata,
        bytes: Vec<u8>,
    ) -> OwnerEquityRawFile {
        OwnerEquityRawFile {
            kind,
            file_name,
            content_hash: ContentHash::from_bytes(&bytes),
            request,
            response_continuation: None,
            bytes,
        }
    }

    fn evidence(
        identity: &OwnerEquityCaptureIdentity,
        observations: usize,
    ) -> OwnerEquityRawEvidence {
        let mut files = vec![reference(identity)];
        let mut remaining = observations;
        for (index, window) in identity.plan.windows.iter().enumerate() {
            let mut dates = Vec::new();
            let mut date = window.start;
            let later_windows = identity.plan.windows.len() - index - 1;
            let take = remaining.saturating_sub(later_windows);
            while date <= window.end && dates.len() < take {
                dates.push(date);
                remaining -= 1;
                date = date.checked_add_days(1).unwrap();
            }
            files.push(daily(identity, window, &dates));
        }
        assert_eq!(remaining, 0);
        OwnerEquityRawEvidence {
            batch_id: identity.batch_id().unwrap(),
            raw_manifest_sha256: ContentHash::from_bytes(b"manifest"),
            batch_json_sha256: ContentHash::from_bytes(b"batch"),
            files,
        }
    }

    #[test]
    fn canonical_instrument_and_one_three_dynamic_window_plans() {
        assert_eq!(plan("2026-08-31", "2026-08-31").windows.len(), 1);
        let three = plan("2026-02-12", "2026-08-31");
        assert_eq!(three.windows.len(), 3);
        assert_eq!(three.exact_get_ceiling, 4);
        assert_eq!(three.windows[0].start.to_iso(), "2026-02-12");
        assert_eq!(three.windows[2].end.to_iso(), "2026-08-31");
        assert_eq!(plan("2026-01-01", "2026-04-10").windows.len(), 1);
        assert_eq!(plan("2026-01-01", "2026-04-11").windows.len(), 2);
        let incremental = OwnerEquityCapturePlan::build(
            "005930.KRX",
            OwnerEquityUniversePolicy::default(),
            OwnerEquityCaptureKind::Incremental,
            TradingDate::parse("2026-08-31").unwrap(),
            TradingDate::parse("2026-08-31").unwrap(),
        )
        .unwrap();
        assert_eq!(incremental.kind, OwnerEquityCaptureKind::Incremental);
        assert_eq!(incremental.exact_get_ceiling, 2);
        for invalid in ["5930.KRX", "005930", "005930.krx", "ABC930.KRX"] {
            assert!(matches!(
                OwnerEquityCapturePlan::build(
                    invalid,
                    OwnerEquityUniversePolicy::default(),
                    OwnerEquityCaptureKind::Initial,
                    TradingDate::parse("2026-01-01").unwrap(),
                    TradingDate::parse("2026-01-01").unwrap(),
                ),
                Err(OwnerEquityV2Error::InstrumentInvalid)
            ));
        }
    }

    #[test]
    fn request_budget_and_range_overflow_fail_typed() {
        assert_eq!(
            checked_get_ceiling(usize::MAX),
            Err(OwnerEquityV2Error::RequestBudgetExceeded)
        );
        assert!(matches!(
            OwnerEquityCapturePlan::build(
                "005930.KRX",
                OwnerEquityUniversePolicy::default(),
                OwnerEquityCaptureKind::Initial,
                TradingDate::parse("1700-01-01").unwrap(),
                TradingDate::parse("2100-01-01").unwrap(),
            ),
            Err(OwnerEquityV2Error::RequestBudgetExceeded)
        ));
    }

    #[test]
    fn valid_121_observation_candidate_is_deterministic_and_pinned() {
        let identity = identity("2026-04-01", "2026-08-31");
        let evidence = evidence(&identity, 121);
        let commit = CodeCommit::parse(MATERIALIZER_COMMIT).unwrap();
        let first =
            materialize_owner_equity_candidate(&identity, &evidence, commit.clone()).unwrap();
        let mut permuted = evidence.clone();
        permuted.files.reverse();
        let second = materialize_owner_equity_candidate(&identity, &permuted, commit).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.observed_sessions, 121);
        assert_eq!(first.target_observed_sessions, 261);
        assert_eq!(first.minimum_observed_sessions, 121);
        assert_eq!(first.first_observed_date.to_iso(), "2026-04-01");
        assert_eq!(first.last_observed_date.to_iso(), "2026-07-30");
        assert_eq!(first.display_name.as_deref(), Some("삼성전자"));
        assert!(first.owner_only && first.vendor_snapshot && !first.strict_pit);
        assert_eq!(first.price_semantics, PRICE_SEMANTICS);
        assert_eq!(
            first.canonical_bytes().unwrap(),
            second.canonical_bytes().unwrap()
        );
        assert_eq!(
            first.content_sha256().unwrap(),
            second.content_sha256().unwrap()
        );
        let bytes = first.canonical_bytes().unwrap();
        verify_owner_equity_candidate(
            &identity,
            &evidence,
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            &bytes,
            &first.content_sha256().unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn initial_plan_and_materialization_trim_to_exact_261_target() {
        let policy = OwnerEquityUniversePolicy::default();
        let through = TradingDate::parse("2026-08-31").unwrap();
        let plan = OwnerEquityCapturePlan::initial_through("005930.KRX", policy, through).unwrap();
        assert_eq!(plan.exact_get_ceiling, 7);
        assert_eq!(plan.requested_end, through);
        let identity = OwnerEquityCaptureIdentity::new(
            plan,
            "vault://entitlements/kis-owner-equity-v2",
            ContentHash::from_bytes(b"entitlement"),
            CodeCommit::parse(CAPTURE_COMMIT).unwrap(),
        )
        .unwrap();
        let candidate = materialize_owner_equity_candidate(
            &identity,
            &evidence(&identity, 300),
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        assert_eq!(candidate.observed_sessions, 261);
        assert_eq!(candidate.bars.len(), 261);
        assert_eq!(
            candidate.first_observed_date,
            candidate.bars[0].session_date
        );
        assert_eq!(
            candidate.last_observed_date,
            candidate.bars[260].session_date
        );
    }

    #[test]
    fn incremental_plan_is_boundary_plus_missing_range_and_merge_is_exact() {
        let prior_identity = identity("2025-01-01", "2025-12-31");
        let prior = materialize_owner_equity_candidate(
            &prior_identity,
            &evidence(&prior_identity, 261),
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        let requested_through = prior.last_observed_date.checked_add_days(1).unwrap();
        let plan = OwnerEquityCapturePlan::incremental_through(
            "005930.KRX",
            OwnerEquityUniversePolicy::default(),
            prior.last_observed_date,
            requested_through,
        )
        .unwrap();
        assert_eq!(plan.requested_start, prior.last_observed_date);
        assert_eq!(plan.exact_get_ceiling, 2);
        let incremental_identity = OwnerEquityCaptureIdentity::new(
            plan,
            prior_identity.entitlement_reference.clone(),
            prior_identity.entitlement_sha256.clone(),
            prior_identity.capture_code_commit.clone(),
        )
        .unwrap();
        let incremental = materialize_owner_equity_candidate(
            &incremental_identity,
            &evidence(&incremental_identity, 2),
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        let artifact_pin = ContentHash::from_bytes(b"prior artifact");
        let merged =
            merge_owner_equity_incremental_candidate(&prior, artifact_pin.clone(), &incremental)
                .unwrap();
        assert_eq!(merged.observed_sessions, 261);
        assert_eq!(merged.last_observed_date, requested_through);
        assert_eq!(
            merged.source_pins.prior_candidate_sha256,
            Some(prior.content_sha256().unwrap())
        );
        assert_eq!(
            merged.source_pins.prior_artifact_manifest_sha256,
            Some(artifact_pin)
        );

        let mut changed = incremental.clone();
        changed.bars[0].close += 1;
        assert_eq!(
            merge_owner_equity_incremental_candidate(
                &prior,
                ContentHash::from_bytes(b"prior artifact"),
                &changed,
            ),
            Err(OwnerEquityV2Error::IncrementalOverlapChanged)
        );
        let mut missing = incremental;
        missing.bars.remove(0);
        assert_eq!(
            merge_owner_equity_incremental_candidate(
                &prior,
                ContentHash::from_bytes(b"prior artifact"),
                &missing,
            ),
            Err(OwnerEquityV2Error::IncrementalOverlapMissing)
        );
    }

    #[test]
    fn one_hundred_twenty_observations_are_insufficient() {
        let identity = identity("2026-04-01", "2026-08-31");
        assert!(matches!(
            materialize_owner_equity_candidate(
                &identity,
                &evidence(&identity, 120),
                CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            ),
            Err(OwnerEquityV2Error::InsufficientHistory)
        ));
    }

    #[test]
    fn malformed_status_symbol_date_cursor_continuation_and_ohlcv_fail_closed() {
        type EvidenceMutation = Box<dyn Fn(&mut OwnerEquityRawEvidence)>;

        let identity = identity("2026-04-01", "2026-08-31");
        let commit = CodeCommit::parse(MATERIALIZER_COMMIT).unwrap();
        let base = evidence(&identity, 121);
        let cases: Vec<(OwnerEquityV2Error, EvidenceMutation)> = vec![
            (
                OwnerEquityV2Error::MalformedJson,
                Box::new(|e| e.files[0].bytes = b"{".to_vec()),
            ),
            (
                OwnerEquityV2Error::ResponseStatusInvalid,
                Box::new(|e| mutate_json(&mut e.files[0], |v| v["rt_cd"] = json!("1"))),
            ),
            (
                OwnerEquityV2Error::SymbolMismatch,
                Box::new(|e| {
                    mutate_json(&mut e.files[0], |v| {
                        v["output"]["stck_shrn_iscd"] = json!("999999")
                    })
                }),
            ),
            (
                OwnerEquityV2Error::DateOutsideRange,
                Box::new(|e| {
                    mutate_json(&mut e.files[1], |v| {
                        v["output2"][0]["stck_bsop_date"] = json!("19990101")
                    })
                }),
            ),
            (
                OwnerEquityV2Error::ContinuationNonblank,
                Box::new(|e| mutate_json(&mut e.files[1], |v| v["CTS"] = json!("next"))),
            ),
            (
                OwnerEquityV2Error::RequestContractMismatch,
                Box::new(|e| e.files[1].response_continuation = Some("M".into())),
            ),
            (
                OwnerEquityV2Error::DuplicateObservation,
                Box::new(|e| {
                    mutate_json(&mut e.files[1], |v| {
                        v["output2"][1]["stck_bsop_date"] =
                            v["output2"][0]["stck_bsop_date"].clone()
                    })
                }),
            ),
            (
                OwnerEquityV2Error::OhlcvInvalid,
                Box::new(|e| {
                    mutate_json(&mut e.files[1], |v| {
                        v["output2"][0]["stck_lwpr"] = json!("106")
                    })
                }),
            ),
        ];
        for (expected, mutate) in cases {
            let mut evidence = base.clone();
            mutate(&mut evidence);
            evidence.files.iter_mut().for_each(|file| {
                file.content_hash = ContentHash::from_bytes(&file.bytes);
            });
            assert_eq!(
                materialize_owner_equity_candidate(&identity, &evidence, commit.clone()),
                Err(expected)
            );
        }
    }

    #[test]
    fn repeated_bytes_tamper_missing_or_changed_pins_fail_closed() {
        let identity = identity("2026-02-12", "2026-08-31");
        let commit = CodeCommit::parse(MATERIALIZER_COMMIT).unwrap();
        let base = evidence(&identity, 121);

        let mut repeated = base.clone();
        repeated.files[2].bytes = repeated.files[1].bytes.clone();
        repeated.files[2].content_hash = repeated.files[1].content_hash.clone();
        assert_eq!(
            materialize_owner_equity_candidate(&identity, &repeated, commit.clone()),
            Err(OwnerEquityV2Error::RepeatedResponseBytes)
        );

        let mut tampered = base.clone();
        tampered.files[1].bytes.push(b' ');
        assert_eq!(
            materialize_owner_equity_candidate(&identity, &tampered, commit.clone()),
            Err(OwnerEquityV2Error::RawTamper)
        );

        let mut missing = base.clone();
        missing.files.pop();
        assert_eq!(
            materialize_owner_equity_candidate(&identity, &missing, commit.clone()),
            Err(OwnerEquityV2Error::EvidenceMissing)
        );

        let changed = OwnerEquityCaptureIdentity::new(
            identity.plan.clone(),
            identity.entitlement_reference.clone(),
            ContentHash::from_bytes(b"changed entitlement"),
            identity.capture_code_commit.clone(),
        )
        .unwrap();
        assert_eq!(
            materialize_owner_equity_candidate(&changed, &base, commit),
            Err(OwnerEquityV2Error::EvidenceIdentityMismatch)
        );
    }

    #[test]
    fn changed_commit_range_contract_and_candidate_pin_fail_closed() {
        let identity = identity("2026-04-01", "2026-08-31");
        let evidence = evidence(&identity, 121);
        let materializer = CodeCommit::parse(MATERIALIZER_COMMIT).unwrap();
        let candidate =
            materialize_owner_equity_candidate(&identity, &evidence, materializer.clone()).unwrap();
        let candidate_bytes = candidate.canonical_bytes().unwrap();
        let candidate_hash = candidate.content_sha256().unwrap();

        let changed_commit = OwnerEquityCaptureIdentity::new(
            identity.plan.clone(),
            identity.entitlement_reference.clone(),
            identity.entitlement_sha256.clone(),
            CodeCommit::parse("fedcba9876543210fedcba9876543210fedcba98").unwrap(),
        )
        .unwrap();
        assert_eq!(
            materialize_owner_equity_candidate(&changed_commit, &evidence, materializer.clone()),
            Err(OwnerEquityV2Error::EvidenceIdentityMismatch)
        );

        let changed_range = OwnerEquityCaptureIdentity::new(
            plan("2026-04-02", "2026-08-31"),
            identity.entitlement_reference.clone(),
            identity.entitlement_sha256.clone(),
            identity.capture_code_commit.clone(),
        )
        .unwrap();
        assert_eq!(
            materialize_owner_equity_candidate(&changed_range, &evidence, materializer.clone()),
            Err(OwnerEquityV2Error::EvidenceIdentityMismatch)
        );

        let mut changed_contract = identity.clone();
        changed_contract.daily_bars_tr_id = "UNAPPROVED".to_owned();
        assert_eq!(
            changed_contract.validate(),
            Err(OwnerEquityV2Error::IdentityInvalid)
        );

        assert_eq!(
            verify_owner_equity_candidate(
                &identity,
                &evidence,
                CodeCommit::parse("abcdef0123456789abcdef0123456789abcdef01").unwrap(),
                &candidate_bytes,
                &candidate_hash,
            ),
            Err(OwnerEquityV2Error::CandidateMismatch)
        );
    }

    #[test]
    fn every_engine_failure_is_a_bounded_uppercase_terminal_code() {
        let failures = [
            OwnerEquityV2Error::InstrumentInvalid,
            OwnerEquityV2Error::RangeInvalid,
            OwnerEquityV2Error::RangeOverflow,
            OwnerEquityV2Error::RequestBudgetExceeded,
            OwnerEquityV2Error::PlanInvalid,
            OwnerEquityV2Error::IdentityInvalid,
            OwnerEquityV2Error::EvidenceIdentityMismatch,
            OwnerEquityV2Error::EvidenceMissing,
            OwnerEquityV2Error::EvidenceUnexpected,
            OwnerEquityV2Error::RequestContractMismatch,
            OwnerEquityV2Error::MalformedJson,
            OwnerEquityV2Error::ResponseStatusInvalid,
            OwnerEquityV2Error::ResponseSchemaInvalid,
            OwnerEquityV2Error::SymbolMismatch,
            OwnerEquityV2Error::DateInvalid,
            OwnerEquityV2Error::DateOutsideRange,
            OwnerEquityV2Error::ContinuationNonblank,
            OwnerEquityV2Error::DuplicateObservation,
            OwnerEquityV2Error::RepeatedResponseBytes,
            OwnerEquityV2Error::OhlcvInvalid,
            OwnerEquityV2Error::DisplayNameInvalid,
            OwnerEquityV2Error::RawTamper,
            OwnerEquityV2Error::InsufficientHistory,
            OwnerEquityV2Error::CoverageOverflow,
            OwnerEquityV2Error::CandidateMismatch,
            OwnerEquityV2Error::IncrementalRangeEmpty,
            OwnerEquityV2Error::IncrementalContractMismatch,
            OwnerEquityV2Error::IncrementalOverlapMissing,
            OwnerEquityV2Error::IncrementalOverlapChanged,
            OwnerEquityV2Error::CanonicalizationFailed,
        ];
        for failure in failures {
            assert_eq!(failure.failure_code().as_str(), failure.code());
            assert_eq!(failure.retry_disposition(), RetryDisposition::Terminal);
        }
    }

    #[test]
    fn secret_sentinel_is_absent_from_metadata_candidate_and_errors() {
        const SECRET: &str = "sentinel-live-secret-value";
        let identity = identity("2026-04-01", "2026-08-31");
        let evidence = evidence(&identity, 121);
        let candidate = materialize_owner_equity_candidate(
            &identity,
            &evidence,
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        assert!(
            !String::from_utf8(candidate.canonical_bytes().unwrap())
                .unwrap()
                .contains(SECRET)
        );
        assert!(
            !serde_json::to_string(&evidence.files[0].request)
                .unwrap()
                .contains(SECRET)
        );
        assert!(
            !OwnerEquityV2Error::MalformedJson
                .to_string()
                .contains(SECRET)
        );
    }

    fn mutate_json(file: &mut OwnerEquityRawFile, mutate: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_slice(&file.bytes).unwrap();
        mutate(&mut value);
        file.bytes = serde_json::to_vec(&value).unwrap();
    }
}
