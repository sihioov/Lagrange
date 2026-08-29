//! Owner-only, read-only price/volume research signals for the fixed equity list.
//!
//! This module deliberately has no repository or queue dependency.  Each
//! request re-attests the checked-in approval registry and then uses the
//! descriptor-safe readers for the immutable artifact and signal snapshot.

use crate::http::JsonBody;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::Session;
use crate::http::state::{ApiState, OwnerBetaEquitySignalsMode};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use factor_engine::{
    PriceVolumeSignalRow, PriceVolumeSignalSnapshot, ResearchCondition,
    read_fixed_stock_price_beta_snapshot_against,
};
use market_data::fixed_stock_price_beta::FIXED_30_INSTRUMENT_NAMES;
use market_data::{
    FIXED_30_INSTRUMENT_IDS, FixedStockPriceBetaApprovedArtifact,
    parse_fixed_stock_price_beta_approval_registry, read_fixed_stock_price_beta_artifact,
    verify_fixed_stock_price_beta_approval,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

const APPROVAL_REGISTRY_BYTES: &[u8] =
    include_bytes!("../../../../configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json");

/// GET `/api/v1/research/owner-beta/equity-price-signals/latest`.
pub async fn latest(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    latest_with_registry(state, session, headers, APPROVAL_REGISTRY_BYTES).await
}

async fn latest_with_registry(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    registry_bytes: &[u8],
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = denied(&state, &session, &rid) {
        return no_store(response);
    }
    let bundle = match load_bundle(&state, registry_bytes).await {
        Ok(bundle) => bundle,
        Err(error) => return no_store(bundle_error(error, &rid)),
    };
    let rows = bundle.snapshot.rows.iter().map(row_dto).collect::<Vec<_>>();
    no_store(
        (
            StatusCode::OK,
            axum::Json(EquitySignalsLatestDto {
                provenance: provenance_dto(&bundle),
                rows: rows.clone(),
                top5: rows.into_iter().take(5).collect(),
            }),
        )
            .into_response(),
    )
}

/// POST `/api/v1/research/owner-beta/equity-price-signals/screen`.
///
/// This is a read-only filter over the immutable, already-ranked snapshot;
/// it intentionally does not create a screen or evaluate a query engine.
pub async fn screen(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<EquitySignalsScreenBody>,
) -> Response {
    screen_with_registry(state, session, headers, body, APPROVAL_REGISTRY_BYTES).await
}

async fn screen_with_registry(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    body: EquitySignalsScreenBody,
    registry_bytes: &[u8],
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = denied(&state, &session, &rid) {
        return no_store(response);
    }
    if let Err(()) = body.validate() {
        return no_store(api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "invalid equity signals screen",
            &rid,
            None,
        ));
    }
    let bundle = match load_bundle(&state, registry_bytes).await {
        Ok(bundle) => bundle,
        Err(error) => return no_store(bundle_error(error, &rid)),
    };
    let selected = body
        .instrument_ids
        .as_ref()
        .map(|ids| ids.iter().map(String::as_str).collect::<BTreeSet<_>>());
    let selected_conditions = body
        .condition
        .as_ref()
        .map(|conditions| conditions.iter().copied().collect::<BTreeSet<_>>());
    let rows = bundle
        .snapshot
        .rows
        .iter()
        .filter(|row| {
            selected
                .as_ref()
                .is_none_or(|ids| ids.contains(row.instrument_id.as_str()))
                && selected_conditions
                    .as_ref()
                    .is_none_or(|conditions| conditions.contains(&row.condition.into()))
                && body.conditions.matches(row)
        })
        .map(row_dto)
        .collect();
    no_store(
        (
            StatusCode::OK,
            axum::Json(EquitySignalsScreenDto {
                provenance: provenance_dto(&bundle),
                rows,
            }),
        )
            .into_response(),
    )
}

/// GET `/api/v1/research/owner-beta/equity-price-signals/instruments/{instrument_id}`.
pub async fn detail(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(instrument_id): Path<String>,
) -> Response {
    detail_with_registry(
        state,
        session,
        headers,
        instrument_id,
        APPROVAL_REGISTRY_BYTES,
    )
    .await
}

async fn detail_with_registry(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    instrument_id: String,
    registry_bytes: &[u8],
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = denied(&state, &session, &rid) {
        return no_store(response);
    }
    if !FIXED_30_INSTRUMENT_IDS.contains(&instrument_id.as_str()) {
        return no_store(code_error("RESOURCE_NOT_FOUND", "resource not found", &rid));
    }
    let bundle = match load_bundle(&state, registry_bytes).await {
        Ok(bundle) => bundle,
        Err(error) => return no_store(bundle_error(error, &rid)),
    };
    let Some(row) = bundle
        .snapshot
        .rows
        .iter()
        .find(|row| row.instrument_id == instrument_id)
    else {
        return no_store(integrity_error(&rid));
    };
    no_store(
        (
            StatusCode::OK,
            axum::Json(EquitySignalsDetailDto {
                provenance: provenance_dto(&bundle),
                signal: row_dto(row),
                factor_explanations: explanations(row),
                condition_reasons: condition_reasons(row),
            }),
        )
            .into_response(),
    )
}

fn denied(state: &ApiState, session: &Session, rid: &str) -> Option<Response> {
    if !session.actor().is_owner() {
        return Some(code_error("FORBIDDEN", "forbidden", rid));
    }
    if state.cfg.owner_beta_equity_signals != OwnerBetaEquitySignalsMode::SealedV1 {
        return Some(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE",
            "owner-beta equity signals unavailable",
            rid,
            None,
        ));
    }
    None
}

#[derive(Debug)]
enum BundleError {
    Unavailable,
    Integrity,
}

struct VerifiedBundle {
    approval: FixedStockPriceBetaApprovedArtifact,
    registry_sha256: String,
    snapshot: PriceVolumeSignalSnapshot,
}

async fn load_bundle(
    state: &ApiState,
    registry_bytes: &[u8],
) -> Result<VerifiedBundle, BundleError> {
    // Parse first so an empty registry cannot cause any artifact I/O.
    let registry = parse_fixed_stock_price_beta_approval_registry(registry_bytes)
        .map_err(|_| BundleError::Integrity)?;
    if registry.approved_artifacts.is_empty() {
        return Err(BundleError::Unavailable);
    }
    if registry.approved_artifacts.len() != 1 {
        return Err(BundleError::Integrity);
    }
    let approved = registry
        .approved_artifacts
        .into_iter()
        .next()
        .expect("checked length");
    let registry_bytes = registry_bytes.to_vec();
    let root = state.cfg.stock_price_beta_artifact_root.clone();
    let permit = state
        .owner_beta_approval
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| BundleError::Unavailable)?;
    let read = tokio::task::spawn_blocking(move || read_bundle(root, approved, &registry_bytes))
        .await
        .map_err(|_| BundleError::Integrity)?;
    drop(permit);
    read
}

fn read_bundle(
    root: PathBuf,
    approved: FixedStockPriceBetaApprovedArtifact,
    registry_bytes: &[u8],
) -> Result<VerifiedBundle, BundleError> {
    let artifact = read_fixed_stock_price_beta_artifact(&root, &approved.artifact_content_sha256)
        .map_err(|_| BundleError::Integrity)?;
    let snapshot = read_fixed_stock_price_beta_snapshot_against(
        &root,
        &approved.snapshot_content_sha256,
        &artifact,
    )
    .map_err(|_| BundleError::Integrity)?;
    let verified = verify_fixed_stock_price_beta_approval(
        registry_bytes,
        &artifact,
        &snapshot.content_sha256,
        &snapshot.as_of,
        &approved.batch_id,
    )
    .map_err(|_| BundleError::Unavailable)?;
    Ok(VerifiedBundle {
        approval: verified.approval,
        registry_sha256: verified.registry_sha256,
        snapshot,
    })
}

fn bundle_error(error: BundleError, rid: &str) -> Response {
    match error {
        BundleError::Unavailable => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE",
            "owner-beta equity signals unavailable",
            rid,
            None,
        ),
        BundleError::Integrity => integrity_error(rid),
    }
}

fn integrity_error(rid: &str) -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED",
        "owner-beta equity signals integrity check failed",
        rid,
        None,
    )
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    response
}

#[derive(Debug, Clone, Serialize)]
struct EquitySignalsProvenanceDto {
    audience: String,
    capability: String,
    selection_basis: String,
    index_membership: String,
    redistribution: String,
    publication_status: String,
    materialization_status: String,
    registration_status: String,
    universe_sha256: String,
    entitlement_sha256: String,
    registry_sha256: String,
    artifact_content_sha256: String,
    snapshot_content_sha256: String,
    batch_id: String,
    as_of: String,
    factor_version: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    original_price: bool,
    warning: String,
    activity_proxy: String,
}

fn provenance_dto(bundle: &VerifiedBundle) -> EquitySignalsProvenanceDto {
    EquitySignalsProvenanceDto {
        audience: bundle.approval.audience.clone(),
        capability: bundle.approval.capability.clone(),
        selection_basis: bundle.approval.selection_basis.clone(),
        index_membership: bundle.approval.index_membership.clone(),
        redistribution: bundle.approval.redistribution.clone(),
        publication_status: bundle.approval.publication_status.clone(),
        materialization_status: bundle.approval.materialization_status.clone(),
        registration_status: bundle.approval.registration_status.clone(),
        universe_sha256: bundle.approval.universe_sha256.clone(),
        entitlement_sha256: bundle.approval.entitlement_sha256.clone(),
        registry_sha256: bundle.registry_sha256.clone(),
        artifact_content_sha256: bundle.snapshot.artifact_content_sha256.clone(),
        snapshot_content_sha256: bundle.snapshot.content_sha256.clone(),
        batch_id: bundle.approval.batch_id.clone(),
        as_of: bundle.snapshot.as_of.clone(),
        factor_version: bundle.snapshot.factor_version.clone(),
        vendor_snapshot: bundle.snapshot.vendor_snapshot,
        strict_pit: bundle.snapshot.strict_pit,
        original_price: bundle.snapshot.original_price,
        warning: bundle.snapshot.warning.clone(),
        activity_proxy: bundle.snapshot.activity_label.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
struct EquitySignalRowDto {
    instrument_id: String,
    instrument_name: String,
    rank: usize,
    score: f64,
    condition: EquitySignalsCondition,
    return_20: f64,
    return_60: f64,
    return_120: f64,
    volatility_20: f64,
    volatility_60: f64,
    volatility_120: f64,
    max_drawdown_120: f64,
    sma_20: f64,
    sma_60: f64,
    average_volume_20: f64,
    volume_ratio_20_60: f64,
    average_trading_value_20: f64,
}

fn row_dto(row: &PriceVolumeSignalRow) -> EquitySignalRowDto {
    let index = FIXED_30_INSTRUMENT_IDS
        .iter()
        .position(|id| *id == row.instrument_id)
        .expect("verified snapshot has configured IDs");
    EquitySignalRowDto {
        instrument_id: row.instrument_id.clone(),
        instrument_name: FIXED_30_INSTRUMENT_NAMES[index].to_owned(),
        rank: row.rank,
        score: row.score,
        condition: row.condition.into(),
        return_20: row.return_20,
        return_60: row.return_60,
        return_120: row.return_120,
        volatility_20: row.volatility_20,
        volatility_60: row.volatility_60,
        volatility_120: row.volatility_120,
        max_drawdown_120: row.max_drawdown_120,
        sma_20: row.sma_20,
        sma_60: row.sma_60,
        average_volume_20: row.average_volume_20,
        volume_ratio_20_60: row.volume_ratio_20_60,
        average_trading_value_20: row.average_trading_value_20,
    }
}

/// Stable public condition labels.  They intentionally do not inherit the
/// factor snapshot's serde representation, so the HTTP contract stays fixed
/// if the internal engine enum evolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EquitySignalsCondition {
    #[serde(rename = "BULLISH")]
    Bullish,
    #[serde(rename = "NEUTRAL")]
    Neutral,
    #[serde(rename = "BEARISH")]
    Bearish,
}

impl From<ResearchCondition> for EquitySignalsCondition {
    fn from(value: ResearchCondition) -> Self {
        match value {
            ResearchCondition::Bullish => Self::Bullish,
            ResearchCondition::Neutral => Self::Neutral,
            ResearchCondition::Bearish => Self::Bearish,
        }
    }
}

#[derive(Serialize)]
struct EquitySignalsLatestDto {
    provenance: EquitySignalsProvenanceDto,
    rows: Vec<EquitySignalRowDto>,
    top5: Vec<EquitySignalRowDto>,
}

#[derive(Serialize)]
struct EquitySignalsScreenDto {
    provenance: EquitySignalsProvenanceDto,
    rows: Vec<EquitySignalRowDto>,
}

#[derive(Serialize)]
struct EquitySignalsDetailDto {
    provenance: EquitySignalsProvenanceDto,
    signal: EquitySignalRowDto,
    factor_explanations: Vec<FactorExplanationDto>,
    condition_reasons: Vec<String>,
}

#[derive(Serialize)]
struct FactorExplanationDto {
    factor: &'static str,
    value: f64,
    interpretation: &'static str,
}

fn explanations(row: &PriceVolumeSignalRow) -> Vec<FactorExplanationDto> {
    vec![
        FactorExplanationDto {
            factor: "return_20",
            value: row.return_20,
            interpretation: "20-session price return",
        },
        FactorExplanationDto {
            factor: "return_60",
            value: row.return_60,
            interpretation: "60-session price return",
        },
        FactorExplanationDto {
            factor: "return_120",
            value: row.return_120,
            interpretation: "120-session price return",
        },
        FactorExplanationDto {
            factor: "volatility_120",
            value: row.volatility_120,
            interpretation: "120-session annualized volatility",
        },
        FactorExplanationDto {
            factor: "max_drawdown_120",
            value: row.max_drawdown_120,
            interpretation: "120-session maximum drawdown",
        },
        FactorExplanationDto {
            factor: "average_trading_value_20",
            value: row.average_trading_value_20,
            interpretation: "20-session activity proxy, not execution liquidity",
        },
        FactorExplanationDto {
            factor: "trend",
            value: row.sma_20 - row.sma_60,
            interpretation: "sma20 minus sma60",
        },
    ]
}

fn condition_reasons(row: &PriceVolumeSignalRow) -> Vec<String> {
    match row.condition {
        ResearchCondition::Bullish => {
            let mut reasons = Vec::new();
            if row.return_20 >= factor_engine::BULLISH_RETURN_20_MIN {
                reasons.push(format!(
                    "return_20 is at least {:.6}",
                    factor_engine::BULLISH_RETURN_20_MIN
                ));
            }
            if row.sma_20 >= row.sma_60 {
                reasons.push("trend_up is true".to_owned());
            }
            if row.volatility_120 <= factor_engine::BULLISH_VOLATILITY_120_MAX {
                reasons.push(format!(
                    "volatility_120 is at most {:.6}",
                    factor_engine::BULLISH_VOLATILITY_120_MAX
                ));
            }
            reasons
        }
        ResearchCondition::Bearish => {
            let mut reasons = Vec::new();
            if row.return_20 <= factor_engine::BEARISH_RETURN_20_MAX {
                reasons.push(format!(
                    "return_20 is at most {:.6}",
                    factor_engine::BEARISH_RETURN_20_MAX
                ));
            }
            if row.sma_20 < row.sma_60
                && row.max_drawdown_120 <= factor_engine::BEARISH_DRAWDOWN_MAX
            {
                reasons.push("trend_down is true".to_owned());
                reasons.push(format!(
                    "max_drawdown_120 is at most {:.6}",
                    factor_engine::BEARISH_DRAWDOWN_MAX
                ));
            }
            reasons
        }
        ResearchCondition::Neutral => {
            vec!["neither bullish nor bearish threshold set is satisfied".to_owned()]
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquitySignalsScreenBody {
    #[serde(default)]
    pub instrument_ids: Option<Vec<String>>,
    #[serde(default)]
    pub conditions: EquitySignalsScreenConditions,
    /// Optional stable scenario filter.  It preserves snapshot rank order.
    #[serde(default)]
    pub condition: Option<Vec<EquitySignalsCondition>>,
}

impl EquitySignalsScreenBody {
    fn validate(&self) -> Result<(), ()> {
        if let Some(ids) = &self.instrument_ids
            && (ids.len() > FIXED_30_INSTRUMENT_IDS.len()
                || ids
                    .iter()
                    .any(|id| !FIXED_30_INSTRUMENT_IDS.contains(&id.as_str()))
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len())
        {
            return Err(());
        }
        if let Some(conditions) = &self.condition
            && (conditions.is_empty()
                || conditions.iter().collect::<BTreeSet<_>>().len() != conditions.len())
        {
            return Err(());
        }
        self.conditions.validate()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquitySignalsScreenConditions {
    pub score: Option<FiniteRange>,
    pub return_20: Option<FiniteRange>,
    pub return_60: Option<FiniteRange>,
    pub return_120: Option<FiniteRange>,
    pub volatility_20: Option<FiniteRange>,
    pub volatility_60: Option<FiniteRange>,
    pub volatility_120: Option<FiniteRange>,
    pub max_drawdown_120: Option<FiniteRange>,
    /// Activity proxy only; this is not execution-liquidity data.
    pub average_trading_value_20: Option<FiniteRange>,
    pub trend_up: Option<bool>,
}

impl EquitySignalsScreenConditions {
    fn ranges(&self) -> [Option<&FiniteRange>; 9] {
        [
            self.score.as_ref(),
            self.return_20.as_ref(),
            self.return_60.as_ref(),
            self.return_120.as_ref(),
            self.volatility_20.as_ref(),
            self.volatility_60.as_ref(),
            self.volatility_120.as_ref(),
            self.max_drawdown_120.as_ref(),
            self.average_trading_value_20.as_ref(),
        ]
    }

    fn validate(&self) -> Result<(), ()> {
        self.ranges()
            .into_iter()
            .flatten()
            .all(FiniteRange::valid)
            .then_some(())
            .ok_or(())
    }

    fn matches(&self, row: &PriceVolumeSignalRow) -> bool {
        let matches = |range: &Option<FiniteRange>, value: f64| {
            range.as_ref().is_none_or(|range| range.includes(value))
        };
        matches(&self.score, row.score)
            && matches(&self.return_20, row.return_20)
            && matches(&self.return_60, row.return_60)
            && matches(&self.return_120, row.return_120)
            && matches(&self.volatility_20, row.volatility_20)
            && matches(&self.volatility_60, row.volatility_60)
            && matches(&self.volatility_120, row.volatility_120)
            && matches(&self.max_drawdown_120, row.max_drawdown_120)
            && matches(&self.average_trading_value_20, row.average_trading_value_20)
            && self
                .trend_up
                .is_none_or(|trend| trend == (row.sma_20 >= row.sma_60))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl FiniteRange {
    fn valid(&self) -> bool {
        self.min.is_none_or(f64::is_finite)
            && self.max.is_none_or(f64::is_finite)
            && self.min.zip(self.max).is_none_or(|(min, max)| min <= max)
    }

    fn includes(&self, value: f64) -> bool {
        self.min.is_none_or(|min| value >= min) && self.max.is_none_or(|max| value <= max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::state::{OwnerBetaAccessMode, OwnerBetaPaperMode, OwnerBetaPriceInputMode};
    use auth::entitlement::{Role, UserId};
    use auth::sessions::SessionInfo;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use chrono::{Duration, NaiveDate};
    use factor_engine::{PriceVolumeSignalSnapshot, write_fixed_stock_price_beta_snapshot_against};
    use market_data::{
        DailyBar, FixedStockPriceBetaApprovalRegistry, FixedStockPriceBetaArtifact,
        FixedStockPriceBetaRawBatchEvidence, FixedStockPriceBetaRawFileEvidence,
        FixedStockPriceBetaRawSourceFile, FixedStockPriceBetaRawWindow,
        write_fixed_stock_price_beta_artifact,
    };
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tower::ServiceExt;

    const UNIVERSE: &[u8] =
        include_bytes!("../../../../configs/universes/kr-stock-price-beta-v1.json");

    fn session(role: Role) -> Session {
        Session(SessionInfo {
            user_id: UserId("00000000-0000-4000-8000-000000000001".to_owned()),
            role,
            auth_time_secs: 1,
            amr: Vec::new(),
            expires_at_secs: 2,
            csrf_token_hash: "test".to_owned(),
        })
    }

    fn headers() -> HeaderMap {
        HeaderMap::from_iter([(
            "x-request-id".parse().expect("header name"),
            "equity-signals-test".parse().expect("header value"),
        )])
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn sources() -> (
        Vec<FixedStockPriceBetaRawWindow>,
        Vec<FixedStockPriceBetaRawSourceFile>,
    ) {
        let windows = ["one", "three", "two"];
        let evidence_windows = windows
            .iter()
            .map(|window_id| FixedStockPriceBetaRawWindow {
                window_id: (*window_id).to_owned(),
                range_start: "2025-08-04".to_owned(),
                range_end: "2026-08-28".to_owned(),
            })
            .collect();
        let mut sources = FIXED_30_INSTRUMENT_IDS
            .iter()
            .flat_map(|instrument_id| {
                windows
                    .into_iter()
                    .map(move |window_id| FixedStockPriceBetaRawSourceFile {
                        relative_path: format!("daily-bars/{instrument_id}/{window_id}.json"),
                        bytes: format!("raw-{instrument_id}-{window_id}").into_bytes(),
                    })
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        (evidence_windows, sources)
    }

    fn fixture_artifact() -> FixedStockPriceBetaArtifact {
        let (windows, sources) = sources();
        let mut files = sources
            .iter()
            .map(|source| {
                let mut parts = source.relative_path.split('/');
                let _ = parts.next();
                let instrument_id = parts.next().expect("instrument path");
                let window_id = parts.next().expect("window path").trim_end_matches(".json");
                FixedStockPriceBetaRawFileEvidence {
                    relative_path: source.relative_path.clone(),
                    instrument_id: instrument_id.to_owned(),
                    window_id: window_id.to_owned(),
                    page_id: "single".to_owned(),
                    sha256: sha256(&source.bytes),
                    size_bytes: source.bytes.len() as u64,
                    method: "GET".to_owned(),
                    path: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice"
                        .to_owned(),
                    tr_id: "FHKST03010100".to_owned(),
                    query_symbol: instrument_id.trim_end_matches(".KRX").to_owned(),
                    query_range_start: "2025-08-04".to_owned(),
                    query_range_end: "2026-08-28".to_owned(),
                    fid_org_adj_prc: "1".to_owned(),
                    response_continuation: String::new(),
                }
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| {
            (&left.instrument_id, &left.window_id, &left.page_id).cmp(&(
                &right.instrument_id,
                &right.window_id,
                &right.page_id,
            ))
        });
        let evidence = FixedStockPriceBetaRawBatchEvidence {
            contract_version: 1,
            provider_scope: "kis-fixed-stock-price-beta-daily-bars-raw-v1".to_owned(),
            requested_range_start: "2025-08-04".to_owned(),
            requested_range_end: "2026-08-28".to_owned(),
            entitlement_reference: "fixture-entitlement".to_owned(),
            entitlement_sha256: "a".repeat(64),
            capture_commit: "b".repeat(40),
            batch_json_sha256: "c".repeat(64),
            manifest_sha256: "d".repeat(64),
            windows,
            files,
        };
        let start = NaiveDate::from_ymd_opt(2025, 8, 4).expect("date");
        let bars = FIXED_30_INSTRUMENT_IDS
            .iter()
            .enumerate()
            .flat_map(|(position, instrument_id)| {
                (0..121).map(move |day| {
                    let slope = match position % 3 {
                        0 => 20,
                        1 => 0,
                        _ => -20,
                    };
                    let close = 10_000 + position as i64 * 100 + 500 + day * slope;
                    DailyBar {
                        instrument_id: (*instrument_id).to_owned(),
                        date: (start + Duration::days(day)).to_string(),
                        open: close,
                        high: close + 2,
                        low: close - 2,
                        close,
                        volume: 1_000 + position as i64,
                    }
                })
            })
            .collect();
        FixedStockPriceBetaArtifact::build(UNIVERSE, evidence, sources, bars)
            .expect("fixture artifact")
    }

    fn approved_registry(
        artifact: &FixedStockPriceBetaArtifact,
        snapshot: &PriceVolumeSignalSnapshot,
    ) -> Vec<u8> {
        let entry = FixedStockPriceBetaApprovedArtifact {
            status: "APPROVED".to_owned(),
            audience: "OWNER_ONLY".to_owned(),
            vendor_snapshot: true,
            strict_pit: false,
            capability: "PRICE_VOLUME_RESEARCH_ONLY".to_owned(),
            selection_basis: "CONFIGURED_FIXED_LIST".to_owned(),
            index_membership: "NOT_EVALUATED".to_owned(),
            redistribution: "NO_REDISTRIBUTION".to_owned(),
            publication_status: "NOT_PUBLISHED".to_owned(),
            universe_sha256: market_data::FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256.to_owned(),
            entitlement_sha256: artifact.evidence.entitlement_sha256.clone(),
            batch_id: "00000000-0000-4000-8000-000000000002".to_owned(),
            source_file_count: artifact.evidence.files.len(),
            factor_version: "fixed-stock-price-beta-factors-v1".to_owned(),
            capture_commit: artifact.evidence.capture_commit.clone(),
            batch_json_sha256: artifact.evidence.batch_json_sha256.clone(),
            manifest_sha256: artifact.evidence.manifest_sha256.clone(),
            artifact_content_sha256: artifact.content_sha256.clone(),
            snapshot_content_sha256: snapshot.content_sha256.clone(),
            range_start: "2025-08-04".to_owned(),
            range_end: "2026-08-28".to_owned(),
            as_of: snapshot.as_of.clone(),
            instruments: FIXED_30_INSTRUMENT_IDS
                .iter()
                .map(|id| (*id).to_owned())
                .collect(),
            instrument_count: 30,
            session_count: artifact.sessions.len(),
            bar_count: artifact.bars.len(),
            materialization_status: "MATERIALIZED".to_owned(),
            registration_status: "UNREGISTERED".to_owned(),
        };
        serde_json::to_vec(&FixedStockPriceBetaApprovalRegistry {
            schema_id: "kr-stock-price-beta-approved-artifacts".to_owned(),
            schema_version: 1,
            approved_artifacts: vec![entry],
        })
        .expect("registry JSON")
    }

    fn fixture() -> (tempfile::TempDir, Vec<u8>) {
        let root = tempfile::tempdir().expect("temporary artifact root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
                .expect("secure fixture root");
        }
        let artifact = fixture_artifact();
        let snapshot = PriceVolumeSignalSnapshot::compute(
            &artifact,
            artifact.sessions.last().expect("as-of session"),
        )
        .expect("fixture snapshot");
        write_fixed_stock_price_beta_artifact(root.path(), &artifact).expect("write artifact");
        write_fixed_stock_price_beta_snapshot_against(root.path(), &snapshot, &artifact)
            .expect("write snapshot");
        (root, approved_registry(&artifact, &snapshot))
    }

    fn sealed_state(root: &std::path::Path) -> ApiState {
        let mut state = ApiState::test_without_database_with_all_policy_and_equity_signals(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
            OwnerBetaPriceInputMode::Disabled,
            OwnerBetaEquitySignalsMode::SealedV1,
        );
        Arc::get_mut(&mut state.cfg)
            .expect("unique test config")
            .stock_price_beta_artifact_root = root.to_owned();
        state
    }

    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body"),
        )
        .expect("JSON response")
    }

    fn assert_approved_policy(provenance: &serde_json::Value) {
        assert_eq!(provenance["audience"], "OWNER_ONLY");
        assert_eq!(provenance["capability"], "PRICE_VOLUME_RESEARCH_ONLY");
        assert_eq!(provenance["selection_basis"], "CONFIGURED_FIXED_LIST");
        assert_eq!(provenance["index_membership"], "NOT_EVALUATED");
        assert_eq!(provenance["redistribution"], "NO_REDISTRIBUTION");
        assert_eq!(provenance["publication_status"], "NOT_PUBLISHED");
        assert_eq!(provenance["materialization_status"], "MATERIALIZED");
        assert_eq!(provenance["registration_status"], "UNREGISTERED");
        assert_eq!(
            provenance["universe_sha256"],
            market_data::FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
        );
        assert_eq!(provenance["entitlement_sha256"], "a".repeat(64));
    }

    #[tokio::test]
    async fn disabled_empty_and_member_requests_fail_closed_without_database() {
        let disabled = ApiState::test_without_database_with_all_policy_and_equity_signals(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
            OwnerBetaPriceInputMode::Disabled,
            OwnerBetaEquitySignalsMode::Disabled,
        );
        let response =
            latest_with_registry(disabled.clone(), session(Role::Owner), headers(), br#"{}"#).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(response).await["error"]["code"],
            "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE"
        );
        assert_eq!(disabled.app_pool.size(), 0);
        assert_eq!(disabled.admin_pool.size(), 0);
        assert_eq!(disabled.audit_pool.size(), 0);

        let root = tempfile::tempdir().expect("root");
        let sealed = sealed_state(root.path());
        let response = latest_with_registry(sealed.clone(), session(Role::Owner), headers(), br#"{"schema_id":"kr-stock-price-beta-approved-artifacts","schema_version":1,"approved_artifacts":[]}"#).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(response).await["error"]["code"],
            "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE"
        );

        let response =
            latest_with_registry(sealed.clone(), session(Role::Member), headers(), br#"{}"#).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(sealed.app_pool.size(), 0);
        assert_eq!(sealed.admin_pool.size(), 0);
        assert_eq!(sealed.audit_pool.size(), 0);
    }

    #[tokio::test]
    async fn router_unauthenticated_request_is_401_before_any_pool_use() {
        let state = ApiState::test_without_database_with_all_policy_and_equity_signals(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
            OwnerBetaPriceInputMode::Disabled,
            OwnerBetaEquitySignalsMode::SealedV1,
        );
        let app = crate::http::api_router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/research/owner-beta/equity-price-signals/latest")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);
    }

    #[tokio::test]
    async fn fixture_latest_screen_detail_and_tamper_are_deterministic() {
        let (root, registry) = fixture();
        let state = sealed_state(root.path());
        let response =
            latest_with_registry(state.clone(), session(Role::Owner), headers(), &registry).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json(response).await;
        assert_eq!(value["rows"].as_array().expect("rows").len(), 30);
        assert_eq!(value["top5"].as_array().expect("top5").len(), 5);
        let provenance = &value["provenance"];
        assert_approved_policy(provenance);
        assert_eq!(provenance["audience"], "OWNER_ONLY");
        assert_eq!(provenance["capability"], "PRICE_VOLUME_RESEARCH_ONLY");
        assert_eq!(provenance["selection_basis"], "CONFIGURED_FIXED_LIST");
        assert_eq!(provenance["index_membership"], "NOT_EVALUATED");
        assert_eq!(provenance["redistribution"], "NO_REDISTRIBUTION");
        assert_eq!(provenance["publication_status"], "NOT_PUBLISHED");
        assert_eq!(provenance["materialization_status"], "MATERIALIZED");
        assert_eq!(provenance["registration_status"], "UNREGISTERED");
        assert_eq!(
            provenance["universe_sha256"],
            market_data::FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
        );
        assert_eq!(provenance["entitlement_sha256"], "a".repeat(64));
        assert_eq!(
            provenance["batch_id"],
            "00000000-0000-4000-8000-000000000002"
        );
        assert_eq!(
            provenance["factor_version"],
            "fixed-stock-price-beta-factors-v1"
        );
        assert_eq!(provenance["vendor_snapshot"], true);
        assert_eq!(provenance["strict_pit"], false);
        assert_eq!(provenance["original_price"], true);
        assert!(
            provenance["warning"]
                .as_str()
                .expect("warning")
                .contains("unadjusted")
        );
        assert_eq!(
            provenance["activity_proxy"],
            "Activity/liquidity proxy, not execution liquidity"
        );
        for forbidden in ["target", "buy", "sell", "weight", "order"] {
            assert!(
                value.get(forbidden).is_none(),
                "{forbidden} must not be exposed"
            );
        }
        for row in value["rows"].as_array().expect("rows") {
            let id = row["instrument_id"].as_str().expect("instrument id");
            let index = FIXED_30_INSTRUMENT_IDS
                .iter()
                .position(|configured| *configured == id)
                .expect("configured instrument");
            assert_eq!(row["instrument_name"], FIXED_30_INSTRUMENT_NAMES[index]);
            assert!(matches!(
                row["condition"].as_str(),
                Some("BULLISH" | "NEUTRAL" | "BEARISH")
            ));
        }

        let invalid = EquitySignalsScreenBody {
            instrument_ids: None,
            conditions: EquitySignalsScreenConditions {
                score: Some(FiniteRange {
                    min: Some(2.0),
                    max: Some(1.0),
                }),
                ..Default::default()
            },
            condition: None,
        };
        let response = screen_with_registry(
            state.clone(),
            session(Role::Owner),
            headers(),
            invalid,
            &registry,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json(response).await["error"]["code"], "INVALID_PARAMETER");
        assert!(serde_json::from_slice::<EquitySignalsScreenBody>(br#"{"unknown":true}"#).is_err());

        let body = EquitySignalsScreenBody {
            instrument_ids: Some(vec![
                FIXED_30_INSTRUMENT_IDS[29].to_owned(),
                FIXED_30_INSTRUMENT_IDS[0].to_owned(),
            ]),
            conditions: EquitySignalsScreenConditions::default(),
            condition: None,
        };
        let response = screen_with_registry(
            state.clone(),
            session(Role::Owner),
            headers(),
            body,
            &registry,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let screened = json(response).await;
        let ranks = screened["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| row["rank"].as_u64().expect("rank"))
            .collect::<Vec<_>>();
        assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));

        let response = detail_with_registry(
            state.clone(),
            session(Role::Owner),
            headers(),
            "not-an-instrument".to_owned(),
            &registry,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = detail_with_registry(
            state.clone(),
            session(Role::Owner),
            headers(),
            FIXED_30_INSTRUMENT_IDS[0].to_owned(),
            &registry,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let detail = json(response).await;
        assert!(
            !detail["factor_explanations"]
                .as_array()
                .expect("factors")
                .is_empty()
        );
        assert!(
            !detail["condition_reasons"]
                .as_array()
                .expect("reasons")
                .is_empty()
        );

        let mut tampered = registry.clone();
        tampered[0] ^= 1;
        let response =
            latest_with_registry(state.clone(), session(Role::Owner), headers(), &tampered).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(response).await["error"]["code"],
            "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED"
        );

        let approval = parse_fixed_stock_price_beta_approval_registry(&registry)
            .expect("fixture registry")
            .approved_artifacts
            .pop()
            .expect("fixture approval");
        std::fs::write(
            root.path()
                .join(&approval.artifact_content_sha256)
                .join("artifact.json"),
            b"{}",
        )
        .expect("tamper artifact");
        let response =
            latest_with_registry(state.clone(), session(Role::Owner), headers(), &registry).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(response).await["error"]["code"],
            "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED"
        );

        let (snapshot_root, snapshot_registry) = fixture();
        let snapshot_approval = parse_fixed_stock_price_beta_approval_registry(&snapshot_registry)
            .expect("fixture registry")
            .approved_artifacts
            .pop()
            .expect("fixture approval");
        std::fs::write(
            snapshot_root
                .path()
                .join(&snapshot_approval.snapshot_content_sha256)
                .join("snapshot.json"),
            b"{}",
        )
        .expect("tamper snapshot");
        let snapshot_state = sealed_state(snapshot_root.path());
        let response = latest_with_registry(
            snapshot_state,
            session(Role::Owner),
            headers(),
            &snapshot_registry,
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(response).await["error"]["code"],
            "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED"
        );
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);
    }

    #[tokio::test]
    async fn screen_conditions_ranges_and_reasons_are_exact_and_truthful() {
        let (root, registry) = fixture();
        let state = sealed_state(root.path());
        let bundle = load_bundle(&state, &registry)
            .await
            .expect("fixture bundle");
        let representative = bundle.snapshot.rows.first().expect("signal row").clone();

        let exact = |value| FiniteRange {
            min: Some(value),
            max: Some(value),
        };
        for conditions in [
            EquitySignalsScreenConditions {
                score: Some(exact(representative.score)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                return_20: Some(exact(representative.return_20)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                return_60: Some(exact(representative.return_60)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                return_120: Some(exact(representative.return_120)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                volatility_20: Some(exact(representative.volatility_20)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                volatility_60: Some(exact(representative.volatility_60)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                volatility_120: Some(exact(representative.volatility_120)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                max_drawdown_120: Some(exact(representative.max_drawdown_120)),
                ..Default::default()
            },
            EquitySignalsScreenConditions {
                average_trading_value_20: Some(exact(representative.average_trading_value_20)),
                ..Default::default()
            },
        ] {
            assert!(conditions.matches(&representative));
        }
        let trend = EquitySignalsScreenConditions {
            trend_up: Some(representative.sma_20 >= representative.sma_60),
            ..Default::default()
        };
        assert!(trend.matches(&representative));
        let opposite_trend = EquitySignalsScreenConditions {
            trend_up: Some(representative.sma_20 < representative.sma_60),
            ..Default::default()
        };
        assert!(!opposite_trend.matches(&representative));

        for invalid in [
            EquitySignalsScreenBody {
                instrument_ids: Some(vec![
                    FIXED_30_INSTRUMENT_IDS[0].to_owned(),
                    FIXED_30_INSTRUMENT_IDS[0].to_owned(),
                ]),
                conditions: EquitySignalsScreenConditions::default(),
                condition: None,
            },
            EquitySignalsScreenBody {
                instrument_ids: Some(vec!["unknown.KRX".to_owned()]),
                conditions: EquitySignalsScreenConditions::default(),
                condition: None,
            },
            EquitySignalsScreenBody {
                instrument_ids: None,
                conditions: EquitySignalsScreenConditions {
                    score: Some(FiniteRange {
                        min: Some(f64::NAN),
                        max: None,
                    }),
                    ..Default::default()
                },
                condition: None,
            },
            EquitySignalsScreenBody {
                instrument_ids: None,
                conditions: EquitySignalsScreenConditions {
                    score: Some(FiniteRange {
                        min: Some(1.0),
                        max: Some(0.0),
                    }),
                    ..Default::default()
                },
                condition: None,
            },
            EquitySignalsScreenBody {
                instrument_ids: None,
                conditions: EquitySignalsScreenConditions::default(),
                condition: Some(vec![
                    EquitySignalsCondition::Bullish,
                    EquitySignalsCondition::Bullish,
                ]),
            },
        ] {
            assert!(invalid.validate().is_err());
        }
        assert!(
            serde_json::from_slice::<EquitySignalsScreenBody>(br#"{"condition":["UNKNOWN"]}"#)
                .is_err()
        );

        for (condition, label) in [
            (EquitySignalsCondition::Bullish, "BULLISH"),
            (EquitySignalsCondition::Neutral, "NEUTRAL"),
            (EquitySignalsCondition::Bearish, "BEARISH"),
        ] {
            let response = screen_with_registry(
                state.clone(),
                session(Role::Owner),
                headers(),
                EquitySignalsScreenBody {
                    instrument_ids: None,
                    conditions: EquitySignalsScreenConditions::default(),
                    condition: Some(vec![condition]),
                },
                &registry,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let result = json(response).await;
            assert_approved_policy(&result["provenance"]);
            let rows = result["rows"].as_array().expect("rows");
            assert!(!rows.is_empty(), "fixture must exercise {label}");
            assert!(rows.iter().all(|row| row["condition"] == label));
            let ranks = rows
                .iter()
                .map(|row| row["rank"].as_u64().expect("rank"))
                .collect::<Vec<_>>();
            assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));
        }

        for row in &bundle.snapshot.rows {
            let response = detail_with_registry(
                state.clone(),
                session(Role::Owner),
                headers(),
                row.instrument_id.clone(),
                &registry,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let detail = json(response).await;
            assert_approved_policy(&detail["provenance"]);
            let reasons = detail["condition_reasons"]
                .as_array()
                .expect("condition reasons")
                .iter()
                .map(|reason| reason.as_str().expect("reason").to_owned())
                .collect::<Vec<_>>();
            let expected = match row.condition {
                ResearchCondition::Bullish => vec![
                    format!(
                        "return_20 is at least {:.6}",
                        factor_engine::BULLISH_RETURN_20_MIN
                    ),
                    "trend_up is true".to_owned(),
                    format!(
                        "volatility_120 is at most {:.6}",
                        factor_engine::BULLISH_VOLATILITY_120_MAX
                    ),
                ],
                ResearchCondition::Bearish => {
                    let mut expected = Vec::new();
                    if row.return_20 <= factor_engine::BEARISH_RETURN_20_MAX {
                        expected.push(format!(
                            "return_20 is at most {:.6}",
                            factor_engine::BEARISH_RETURN_20_MAX
                        ));
                    }
                    if row.sma_20 < row.sma_60
                        && row.max_drawdown_120 <= factor_engine::BEARISH_DRAWDOWN_MAX
                    {
                        expected.push("trend_down is true".to_owned());
                        expected.push(format!(
                            "max_drawdown_120 is at most {:.6}",
                            factor_engine::BEARISH_DRAWDOWN_MAX
                        ));
                    }
                    expected
                }
                ResearchCondition::Neutral => {
                    vec!["neither bullish nor bearish threshold set is satisfied".to_owned()]
                }
            };
            assert_eq!(reasons, expected);
        }
    }
}
