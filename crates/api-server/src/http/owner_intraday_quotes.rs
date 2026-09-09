//! Owner-only HTTP mutations for bounded intraday quote demand leases.
//!
//! This module owns only the HTTP DTO, privacy/order checks, and response
//! projection. Durable replay, sequence, expiry, release, and capacity rules
//! stay in the job-queue repository and are never mirrored here.

use axum::extract::rejection::PathRejection;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use collectors::intraday_quotes::{
    IntradayMarketState, IntradaySessionDisposition, IntradaySessionWindowContract,
};
use domain::Venue;
use market_data::intraday_quotes::IntradayQuoteDirection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use job_queue::owner_equity_v2::{
    IntradayCacheRecord, IntradayCalendarDisposition, IntradayCalendarReadState,
    IntradayIdentityReadState, IntradayQuoteDemandRequest, IntradayQuoteFailureCode,
    IntradayQuoteReleaseRequest, IntradayStorageError,
};

use crate::http::JsonBody;
use crate::http::error::{code_error, request_id};
use crate::http::idempotency;
use crate::http::session::{Session, require_csrf};
use crate::http::state::{ApiState, OwnerIntradayQuoteReadConfig};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntradayQuoteDemandBody {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub generation: u64,
    pub renewal_sequence: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntradayQuoteReleaseBody {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntradayQuoteDemandDto {
    pub schema_version: u32,
    pub demand_id: Uuid,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
    pub renewal_sequence: u64,
    pub lease_expires_at: chrono::DateTime<chrono::Utc>,
    pub renew_after_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntradayQuoteMarketState {
    Open,
    Closed,
    Halted,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntradayQuoteFreshness {
    Recent,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntradayQuoteReasonCode {
    NoActiveDemand,
    QuotePending,
    QuoteStale,
    ProviderTimeout,
    ProviderRateLimited,
    ProviderUnavailable,
    ProviderResponseInvalid,
    QuoteValueInvalid,
    QuoteBudgetExhausted,
    CalendarUnavailable,
    SessionWindowUnavailable,
    SessionClosed,
    InstrumentHalted,
    ProducerUnavailable,
    FeatureDisabled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntradayQuoteDirectionDto {
    Up,
    Down,
    Flat,
    LimitUp,
    LimitDown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntradayQuoteSessionDto {
    pub date: NaiveDate,
    pub timezone: &'static str,
    pub calendar_source: &'static str,
    pub calendar_source_version: &'static str,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntradayQuotePayloadDto {
    pub price: String,
    pub base_price: String,
    pub change_from_previous_day: String,
    pub change_percent_from_previous_day: String,
    pub direction: IntradayQuoteDirectionDto,
    pub received_at: DateTime<Utc>,
    pub last_success_at: DateTime<Utc>,
    pub quote_version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntradayQuoteDto {
    pub schema_version: u32,
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub venue: &'static str,
    pub currency: &'static str,
    pub generation: u64,
    pub session: Option<IntradayQuoteSessionDto>,
    pub market_state: IntradayQuoteMarketState,
    pub freshness: IntradayQuoteFreshness,
    pub reason_code: Option<IntradayQuoteReasonCode>,
    pub quote: Option<IntradayQuotePayloadDto>,
    pub next_poll_after_ms: i64,
}

const GET_QUOTE_PATH: &str = "/api/v1/research/owner-beta/equity-universe-v2/instruments/";
const KST_TIMEZONE: &str = "Asia/Seoul";
const NEXT_POLL_AFTER_MS: i64 = 5_000;
const MAX_GENERATION: u64 = i64::MAX as u64;

/// Read the current owner cache only.  Authentication is deliberately the
/// first semantic gate: the raw URI is extracted without validation so a
/// member cannot use malformed input to learn anything about this resource.
pub async fn get_current_quote(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    uri: OriginalUri,
) -> Response {
    let rid = request_id(&headers);
    if !session.actor().is_owner() {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }

    let Some((instrument_id, membership_id, generation)) = parse_quote_get_input(&uri) else {
        return code_error("INVALID_PARAMETER", "invalid intraday quote request", &rid);
    };

    let repo = state.owner_intraday_quotes();
    let initial_identity = match repo
        .read_current_identity_state(&session.actor(), membership_id, &instrument_id, generation)
        .await
    {
        Ok(Some(identity)) => identity,
        Ok(None) => return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid),
        Err(error) => return read_storage_error(error, &rid),
    };

    let read_config = state.cfg.owner_intraday_quotes.clone();
    let mut calendar = None;
    let mut cache = None;
    let mut calendar_read_unavailable = false;

    if let OwnerIntradayQuoteReadConfig::OwnerOnly { window } = &read_config {
        calendar = match repo
            .read_current_calendar_disposition(&session.actor())
            .await
        {
            Ok(calendar) => calendar,
            Err(IntradayStorageError::CalendarProofUnavailable)
            | Err(IntradayStorageError::SessionProofInvalid) => {
                calendar_read_unavailable = true;
                None
            }
            Err(error) => return read_storage_error(error, &rid),
        };

        if let (Some(calendar_state), Some(window)) = (calendar.as_ref(), window.as_ref())
            && calendar_state.disposition == IntradayCalendarDisposition::Trading
            && window
                .entry(calendar_state.session_date)
                .is_some_and(|entry| {
                    matches!(
                        entry.disposition,
                        IntradaySessionDisposition::Regular | IntradaySessionDisposition::Special
                    )
                })
        {
            let proof = job_queue::owner_equity_v2::IntradaySessionProof::new(
                calendar_state.session_date,
                calendar_state.calendar_source_batch_id,
                calendar_state.calendar_content_sha256.clone(),
                window.window_contract_sha256().to_owned(),
            );
            match proof {
                Ok(proof) => match repo
                    .read_current_cache(&session.actor(), membership_id, generation, &proof)
                    .await
                {
                    Ok(value) => cache = value,
                    Err(IntradayStorageError::CalendarProofUnavailable)
                    | Err(IntradayStorageError::SessionProofInvalid) => {
                        calendar_read_unavailable = true;
                    }
                    Err(error) => return read_storage_error(error, &rid),
                },
                Err(IntradayStorageError::CalendarProofUnavailable)
                | Err(IntradayStorageError::SessionProofInvalid) => {
                    calendar_read_unavailable = true;
                }
                Err(error) => return read_storage_error(error, &rid),
            }
        }
    }

    // The final identity read is the authoritative invalidation boundary. It
    // also supplies the active-demand bit used by the projection.
    let final_identity = match repo
        .read_current_identity_state(&session.actor(), membership_id, &instrument_id, generation)
        .await
    {
        Ok(Some(identity)) => identity,
        Ok(None) => return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid),
        Err(error) => return read_storage_error(error, &rid),
    };
    if final_identity.identity != initial_identity.identity {
        return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid);
    }

    // This is the one application-clock sample. It happens only after all
    // database reads, so API-date rollover cannot be hidden by an early read.
    let now = (state.cfg.intraday_now)();
    let dto = project_owner_intraday_quote(
        &read_config,
        &final_identity,
        calendar.as_ref(),
        cache.as_ref(),
        calendar_read_unavailable,
        now,
    );
    let mut response = (StatusCode::OK, axum::Json(dto)).into_response();
    response
        .headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    response
}

/// The production projection used by the GET handler.  It is intentionally
/// pure: all database/provider boundaries are completed by the caller, and
/// this function can therefore be exhaustively tested with local fixtures.
pub fn project_owner_intraday_quote(
    config: &OwnerIntradayQuoteReadConfig,
    identity: &IntradayIdentityReadState,
    calendar: Option<&IntradayCalendarReadState>,
    cache: Option<&IntradayCacheRecord>,
    calendar_read_unavailable: bool,
    now: DateTime<Utc>,
) -> IntradayQuoteDto {
    let base = |market_state, freshness, reason_code, session, quote| IntradayQuoteDto {
        schema_version: 1,
        membership_id: identity.identity.membership_id,
        instrument_id: identity.identity.instrument_id.clone(),
        venue: "KRX",
        currency: "KRW",
        generation: identity.identity.generation,
        session,
        market_state,
        freshness,
        reason_code,
        quote,
        next_poll_after_ms: NEXT_POLL_AFTER_MS,
    };

    let OwnerIntradayQuoteReadConfig::OwnerOnly { window } = config else {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::FeatureDisabled),
            None,
            None,
        );
    };

    let Some(calendar) = calendar else {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::CalendarUnavailable),
            None,
            None,
        );
    };
    if calendar_read_unavailable || !valid_calendar(calendar, now) {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::CalendarUnavailable),
            None,
            None,
        );
    }

    let Some(window) = window.as_ref() else {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable),
            None,
            None,
        );
    };
    let Some(entry) = window.entry(calendar.session_date) else {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable),
            None,
            None,
        );
    };
    if !calendar_agrees_with_window(calendar.disposition, entry.disposition)
        || !valid_window_hash(window.window_contract_sha256())
    {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable),
            None,
            None,
        );
    }
    let window_state = window.state_at(now, false);
    if window_state == IntradayMarketState::Unknown {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable),
            None,
            None,
        );
    }

    let session = Some(IntradayQuoteSessionDto {
        date: calendar.session_date,
        timezone: KST_TIMEZONE,
        calendar_source: calendar.calendar_source(),
        calendar_source_version: calendar.calendar_source_version(),
        calendar_content_sha256: calendar.calendar_content_sha256.clone(),
        window_contract_sha256: window.window_contract_sha256().to_owned(),
    });

    if cache_has_future_timestamp(cache, now) {
        return base(
            IntradayQuoteMarketState::Unknown,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::ProducerUnavailable),
            None,
            None,
        );
    }

    let current_cache = cache.filter(|record| {
        cache_matches_identity_and_session(record, &identity.identity, calendar, window)
    });
    let usable_quote = current_cache.and_then(|record| usable_quote(record, now));
    let freshness = usable_quote
        .as_ref()
        .map(|(_, freshness)| *freshness)
        .unwrap_or(IntradayQuoteFreshness::Unavailable);
    let quote = usable_quote
        .as_ref()
        .map(|(record, _)| quote_payload(record));

    if calendar.disposition == IntradayCalendarDisposition::Closed
        && entry.disposition == IntradaySessionDisposition::Closed
    {
        return base(
            IntradayQuoteMarketState::Closed,
            IntradayQuoteFreshness::Unavailable,
            Some(IntradayQuoteReasonCode::SessionClosed),
            session,
            None,
        );
    }

    if window_state == IntradayMarketState::Closed {
        return base(
            IntradayQuoteMarketState::Closed,
            freshness,
            Some(IntradayQuoteReasonCode::SessionClosed),
            session,
            quote,
        );
    }

    if usable_quote
        .as_ref()
        .is_some_and(|(record, _)| record.halted == Some(true))
    {
        return base(
            IntradayQuoteMarketState::Halted,
            freshness,
            Some(IntradayQuoteReasonCode::InstrumentHalted),
            session,
            quote,
        );
    }

    let reason_code = if !identity.has_active_demand {
        Some(IntradayQuoteReasonCode::NoActiveDemand)
    } else if let Some(record) = current_cache {
        eligible_failure(record, now)
            .map(reason_from_failure)
            .or_else(|| {
                let stale_quote = usable_quote
                    .as_ref()
                    .is_some_and(|(_, freshness)| *freshness == IntradayQuoteFreshness::Stale);
                if stale_quote {
                    Some(IntradayQuoteReasonCode::QuoteStale)
                } else if usable_quote.is_none() {
                    Some(IntradayQuoteReasonCode::QuotePending)
                } else {
                    None
                }
            })
    } else {
        Some(IntradayQuoteReasonCode::QuotePending)
    };

    base(
        IntradayQuoteMarketState::Open,
        freshness,
        reason_code,
        session,
        quote,
    )
}

fn parse_quote_get_input(uri: &OriginalUri) -> Option<(String, Uuid, u64)> {
    let path = uri.path();
    let rest = path.strip_prefix(GET_QUOTE_PATH)?;
    let instrument_id = rest.strip_suffix("/quote")?;
    if instrument_id.is_empty() || instrument_id.contains('/') {
        return None;
    }
    let parsed = domain::InstrumentId::parse(instrument_id).ok()?;
    if parsed.venue() != Venue::Krx
        || parsed.symbol().len() != 6
        || !parsed.symbol().bytes().all(|byte| byte.is_ascii_digit())
        || parsed.as_str() != instrument_id
    {
        return None;
    }

    let query = uri.query()?;
    let mut membership_id = None;
    let mut generation = None;
    let mut fields = 0;
    for field in query.split('&') {
        let (key, value) = field.split_once('=')?;
        if key.is_empty() || value.is_empty() {
            return None;
        }
        fields += 1;
        match key {
            "membership_id" if membership_id.is_none() => {
                membership_id = Some(Uuid::parse_str(value).ok()?);
            }
            "generation" if generation.is_none() => {
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                let value = value.parse::<u64>().ok()?;
                if value == 0 || value > MAX_GENERATION {
                    return None;
                }
                generation = Some(value);
            }
            _ => return None,
        }
    }
    if fields != 2 {
        return None;
    }
    Some((instrument_id.to_owned(), membership_id?, generation?))
}

fn valid_calendar(calendar: &IntradayCalendarReadState, now: DateTime<Utc>) -> bool {
    let kst = FixedOffset::east_opt(9 * 60 * 60).expect("valid KST offset");
    calendar.session_date == now.with_timezone(&kst).date_naive()
        && !calendar.calendar_source_batch_id.is_nil()
        && canonical_unprefixed_sha256(&calendar.calendar_content_sha256)
}

fn calendar_agrees_with_window(
    calendar: IntradayCalendarDisposition,
    window: IntradaySessionDisposition,
) -> bool {
    matches!(
        (calendar, window),
        (
            IntradayCalendarDisposition::Trading,
            IntradaySessionDisposition::Regular | IntradaySessionDisposition::Special
        ) | (
            IntradayCalendarDisposition::Closed,
            IntradaySessionDisposition::Closed
        )
    )
}

fn cache_matches_identity_and_session(
    cache: &IntradayCacheRecord,
    identity: &job_queue::owner_equity_v2::IntradayQuoteIdentity,
    calendar: &IntradayCalendarReadState,
    window: &IntradaySessionWindowContract,
) -> bool {
    cache.owner_user_id == identity.owner_user_id
        && cache.membership_id == identity.membership_id
        && cache.generation_id == identity.generation_id
        && cache.instrument_id == identity.instrument_id
        && cache.generation == identity.generation
        && cache.session_date == Some(calendar.session_date)
        && cache.calendar_source.as_deref() == Some(calendar.calendar_source())
        && cache.calendar_source_version.as_deref() == Some(calendar.calendar_source_version())
        && cache.calendar_source_batch_id == Some(calendar.calendar_source_batch_id)
        && cache.calendar_content_sha256.as_deref()
            == Some(calendar.calendar_content_sha256.as_str())
        && cache.window_contract_sha256.as_deref() == Some(window.window_contract_sha256())
}

fn cache_has_future_timestamp(cache: Option<&IntradayCacheRecord>, now: DateTime<Utc>) -> bool {
    cache.is_some_and(|record| {
        record.last_attempt_at > now
            || record.received_at.is_some_and(|value| value > now)
            || record.last_success_at.is_some_and(|value| value > now)
            || record.last_failure_at.is_some_and(|value| value > now)
    })
}

fn usable_quote(
    record: &IntradayCacheRecord,
    now: DateTime<Utc>,
) -> Option<(&IntradayCacheRecord, IntradayQuoteFreshness)> {
    if record.quote_version == 0 || record.quote_version > MAX_GENERATION {
        return None;
    }
    let (
        Some(price),
        Some(base_price),
        Some(change_amount),
        Some(change_percent),
        Some(direction),
        Some(halted),
        Some(received_at),
        Some(last_success_at),
    ) = (
        record.price.as_deref(),
        record.base_price.as_deref(),
        record.change_amount.as_deref(),
        record.change_percent.as_deref(),
        record.direction,
        record.halted,
        record.received_at,
        record.last_success_at,
    )
    else {
        return None;
    };
    if received_at != last_success_at
        || received_at > now
        || last_success_at > now
        || !positive_decimal(price)
        || !positive_decimal(base_price)
        || !decimal(change_amount)
        || !decimal(change_percent)
    {
        return None;
    }
    let age = age_since(last_success_at, now)?;
    if age > Duration::hours(24) {
        return None;
    }
    let freshness = if age <= Duration::seconds(30) {
        IntradayQuoteFreshness::Recent
    } else {
        IntradayQuoteFreshness::Stale
    };
    let _ = (direction, halted);
    Some((record, freshness))
}

fn quote_payload(record: &IntradayCacheRecord) -> IntradayQuotePayloadDto {
    IntradayQuotePayloadDto {
        price: record.price.clone().expect("usable quote price"),
        base_price: record.base_price.clone().expect("usable quote base price"),
        change_from_previous_day: record.change_amount.clone().expect("usable quote change"),
        change_percent_from_previous_day: record
            .change_percent
            .clone()
            .expect("usable quote percent"),
        direction: direction_dto(record.direction.expect("usable quote direction")),
        received_at: record.received_at.expect("usable quote receipt"),
        last_success_at: record.last_success_at.expect("usable quote success"),
        quote_version: record.quote_version.to_string(),
    }
}

fn direction_dto(direction: IntradayQuoteDirection) -> IntradayQuoteDirectionDto {
    match direction {
        IntradayQuoteDirection::Up => IntradayQuoteDirectionDto::Up,
        IntradayQuoteDirection::Down => IntradayQuoteDirectionDto::Down,
        IntradayQuoteDirection::Flat => IntradayQuoteDirectionDto::Flat,
        IntradayQuoteDirection::LimitUp => IntradayQuoteDirectionDto::LimitUp,
        IntradayQuoteDirection::LimitDown => IntradayQuoteDirectionDto::LimitDown,
    }
}

fn eligible_failure(
    record: &IntradayCacheRecord,
    now: DateTime<Utc>,
) -> Option<IntradayQuoteFailureCode> {
    let code = record.last_failure_code?;
    if !matches!(
        code,
        IntradayQuoteFailureCode::ProviderTimeout
            | IntradayQuoteFailureCode::ProviderRateLimited
            | IntradayQuoteFailureCode::ProviderUnavailable
            | IntradayQuoteFailureCode::ProviderResponseInvalid
            | IntradayQuoteFailureCode::QuoteValueInvalid
            | IntradayQuoteFailureCode::QuoteBudgetExhausted
            | IntradayQuoteFailureCode::ProducerUnavailable
    ) {
        return None;
    }
    let failure_at = record.last_failure_at?;
    if failure_at > now
        || record
            .last_success_at
            .is_some_and(|success| failure_at < success)
    {
        return None;
    }
    Some(code)
}

fn reason_from_failure(code: IntradayQuoteFailureCode) -> IntradayQuoteReasonCode {
    match code {
        IntradayQuoteFailureCode::ProviderTimeout => IntradayQuoteReasonCode::ProviderTimeout,
        IntradayQuoteFailureCode::ProviderRateLimited => {
            IntradayQuoteReasonCode::ProviderRateLimited
        }
        IntradayQuoteFailureCode::ProviderUnavailable => {
            IntradayQuoteReasonCode::ProviderUnavailable
        }
        IntradayQuoteFailureCode::ProviderResponseInvalid => {
            IntradayQuoteReasonCode::ProviderResponseInvalid
        }
        IntradayQuoteFailureCode::QuoteValueInvalid => IntradayQuoteReasonCode::QuoteValueInvalid,
        IntradayQuoteFailureCode::QuoteBudgetExhausted => {
            IntradayQuoteReasonCode::QuoteBudgetExhausted
        }
        IntradayQuoteFailureCode::ProducerUnavailable => {
            IntradayQuoteReasonCode::ProducerUnavailable
        }
        _ => unreachable!("eligible_failure filters failure codes"),
    }
}

fn age_since(then: DateTime<Utc>, now: DateTime<Utc>) -> Option<Duration> {
    let age = now.signed_duration_since(then);
    (age >= Duration::zero()).then_some(age)
}

fn canonical_unprefixed_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_window_hash(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(canonical_unprefixed_sha256)
}

fn decimal(value: &str) -> bool {
    let (negative, body) = value
        .strip_prefix('-')
        .map_or((false, value), |body| (true, body));
    let (integer, fraction) = body
        .split_once('.')
        .map_or((body, None), |(i, f)| (i, Some(f)));
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
        || fraction.is_some_and(|part| {
            part.is_empty() || part.len() > 8 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return false;
    }
    let nonzero = integer.bytes().any(|byte| byte != b'0')
        || fraction.is_some_and(|part| part.bytes().any(|byte| byte != b'0'));
    !(negative && !nonzero)
}

fn positive_decimal(value: &str) -> bool {
    decimal(value)
        && value.trim_start_matches('-').split_once('.').map_or_else(
            || value.bytes().any(|byte| byte != b'0'),
            |(integer, fraction)| {
                integer.bytes().any(|byte| byte != b'0')
                    || fraction.bytes().any(|byte| byte != b'0')
            },
        )
        && !value.starts_with('-')
}

fn read_storage_error(error: IntradayStorageError, rid: &str) -> Response {
    match error {
        IntradayStorageError::InvalidInput => {
            code_error("INVALID_PARAMETER", "invalid intraday quote request", rid)
        }
        _ => code_error(
            "QUOTE_CACHE_UNAVAILABLE",
            "intraday quote cache is unavailable",
            rid,
        ),
    }
}

pub async fn create_or_renew(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    body: Result<JsonBody<IntradayQuoteDemandBody>, Response>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    let JsonBody(body) = match body {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return code_error(
            "INVALID_PARAMETER",
            "invalid intraday demand idempotency key",
            &rid,
        );
    };
    let request = match IntradayQuoteDemandRequest::with_schema_version(
        body.schema_version,
        body.consumer_id,
        body.membership_id,
        body.generation,
        body.renewal_sequence,
        key,
    ) {
        Ok(request) => request,
        Err(error) => return storage_error(error, &rid),
    };

    match state
        .owner_intraday_quotes()
        .create_or_renew_demand(&session.actor(), &request)
        .await
    {
        Ok(outcome) => {
            let lease = outcome.lease;
            (
                StatusCode::OK,
                axum::Json(IntradayQuoteDemandDto {
                    schema_version: 1,
                    demand_id: lease.demand_id,
                    consumer_id: lease.consumer_id,
                    membership_id: lease.membership_id,
                    instrument_id: lease.instrument_id,
                    generation: lease.generation,
                    renewal_sequence: lease.renewal_sequence,
                    lease_expires_at: lease.lease_expires_at,
                    renew_after_ms: lease.renew_after_ms,
                }),
            )
                .into_response()
        }
        Err(error) => storage_error(error, &rid),
    }
}

pub async fn release(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<JsonBody<IntradayQuoteReleaseBody>, Response>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    let demand_id = match path {
        Ok(Path(demand_id)) => demand_id,
        Err(_) => return code_error("INVALID_PARAMETER", "invalid intraday demand request", &rid),
    };
    let JsonBody(body) = match body {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return code_error(
            "INVALID_PARAMETER",
            "invalid intraday demand idempotency key",
            &rid,
        );
    };
    if body.schema_version != 1 {
        return storage_error(IntradayStorageError::InvalidInput, &rid);
    }
    let request =
        match IntradayQuoteReleaseRequest::new(body.consumer_id, body.renewal_sequence, key) {
            Ok(request) => request,
            Err(error) => return storage_error(error, &rid),
        };

    match state
        .owner_intraday_quotes()
        .release_demand(&session.actor(), demand_id, &request)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => storage_error(error, &rid),
    }
}

fn mutation_guard(session: &Session, headers: &HeaderMap, rid: &str) -> Option<Response> {
    if !session.actor().is_owner() {
        return Some(code_error("FORBIDDEN", "forbidden", rid));
    }
    require_csrf(headers, &session.0).err()
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    let key = headers
        .get(idempotency::HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)?;
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b'\\')
    {
        return None;
    }
    Some(key)
}

fn storage_error(error: IntradayStorageError, rid: &str) -> Response {
    match error {
        IntradayStorageError::InvalidInput => {
            code_error("INVALID_PARAMETER", "invalid intraday demand request", rid)
        }
        IntradayStorageError::DemandNotFound
        | IntradayStorageError::IdentityMismatch
        | IntradayStorageError::MembershipNotReady => {
            code_error("RESOURCE_NOT_FOUND", "resource not found", rid)
        }
        IntradayStorageError::DemandReleased | IntradayStorageError::SequenceConflict => {
            code_error(
                "QUOTE_DEMAND_SEQUENCE_CONFLICT",
                "intraday demand sequence cannot be applied",
                rid,
            )
        }
        IntradayStorageError::IdempotencyMismatch => code_error(
            "IDEMPOTENCY_MISMATCH",
            "the same Idempotency-Key was already used with a different request",
            rid,
        ),
        IntradayStorageError::DemandCapacity | IntradayStorageError::IdentityCapacity => {
            let mut response = code_error(
                "QUOTE_DEMAND_CAPACITY",
                "intraday quote demand capacity is exhausted",
                rid,
            );
            response
                .headers_mut()
                .insert("Retry-After", HeaderValue::from_static("15"));
            response
        }
        IntradayStorageError::PolicyUnavailable
        | IntradayStorageError::DatabaseUnavailable
        | IntradayStorageError::DatabaseIntegrity
        | IntradayStorageError::PermissionDenied
        | IntradayStorageError::CommitUnknown
        | IntradayStorageError::ProducerLeaseHeld
        | IntradayStorageError::ProducerLeaseLost
        | IntradayStorageError::ActiveDemandRequired
        | IntradayStorageError::SessionProofInvalid
        | IntradayStorageError::CalendarProofUnavailable
        | IntradayStorageError::BudgetProofInvalid
        | IntradayStorageError::ReceiptInvalid
        | IntradayStorageError::QuoteInvalid
        | IntradayStorageError::QuoteReceiptStale
        | IntradayStorageError::CacheNotFound
        | IntradayStorageError::QuoteVersionExhausted
        | IntradayStorageError::ProducerFenceExhausted => code_error(
            "QUOTE_CACHE_UNAVAILABLE",
            "intraday quote demand storage is unavailable",
            rid,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use job_queue::owner_equity_v2::{IntradayQuoteIdentity, IntradaySessionProof};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    const INSTRUMENT: &str = "069500.KRX";
    const CALENDAR_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 5, 0, 0).single().unwrap()
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 8).unwrap()
    }

    fn identity(active: bool) -> IntradayIdentityReadState {
        IntradayIdentityReadState {
            identity: IntradayQuoteIdentity::new(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                INSTRUMENT.to_owned(),
                1,
            )
            .unwrap(),
            has_active_demand: active,
            observed_at: now(),
        }
    }

    fn calendar(
        disposition: IntradayCalendarDisposition,
        observed_at: DateTime<Utc>,
    ) -> IntradayCalendarReadState {
        IntradayCalendarReadState {
            session_date: date(),
            disposition,
            calendar_source_batch_id: Uuid::from_u128(4),
            calendar_content_sha256: CALENDAR_HASH.to_owned(),
            observed_at,
        }
    }

    fn window(
        disposition: &str,
        open: Option<&str>,
        close: Option<&str>,
    ) -> Arc<IntradaySessionWindowContract> {
        window_with_evidence(disposition, open, close, "2026-09-07T15:00:00Z")
    }

    fn window_with_evidence(
        disposition: &str,
        open: Option<&str>,
        close: Option<&str>,
        evidence_retrieved_at: &str,
    ) -> Arc<IntradaySessionWindowContract> {
        let bytes = serde_json::to_vec(&json!({
            "schema_version": 1,
            "exchange": "KRX",
            "timezone": "Asia/Seoul",
            "entries": [{
                "date": date().to_string(),
                "disposition": disposition,
                "open_local": open,
                "close_local": close,
                "evidence_url": "https://global.krx.co.kr/contents/test",
                "evidence_retrieved_at": evidence_retrieved_at,
                "evidence_sha256": format!("sha256:{}", "b".repeat(64)),
            }],
        }))
        .unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
        Arc::new(IntradaySessionWindowContract::from_bytes(&bytes, &hash).unwrap())
    }

    fn kst(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        FixedOffset::east_opt(9 * 60 * 60)
            .unwrap()
            .from_local_datetime(&date().and_hms_opt(hour, minute, second).unwrap())
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn config(window: Option<Arc<IntradaySessionWindowContract>>) -> OwnerIntradayQuoteReadConfig {
        OwnerIntradayQuoteReadConfig::OwnerOnly { window }
    }

    fn cache(
        owner: &IntradayIdentityReadState,
        calendar: &IntradayCalendarReadState,
        window: &IntradaySessionWindowContract,
        at: DateTime<Utc>,
    ) -> IntradayCacheRecord {
        IntradayCacheRecord {
            owner_user_id: owner.identity.owner_user_id,
            membership_id: owner.identity.membership_id,
            generation_id: owner.identity.generation_id,
            instrument_id: owner.identity.instrument_id.clone(),
            generation: owner.identity.generation,
            session_date: Some(calendar.session_date),
            calendar_source: Some(calendar.calendar_source().to_owned()),
            calendar_source_version: Some(calendar.calendar_source_version().to_owned()),
            calendar_source_batch_id: Some(calendar.calendar_source_batch_id),
            calendar_content_sha256: Some(calendar.calendar_content_sha256.clone()),
            window_contract_sha256: Some(window.window_contract_sha256().to_owned()),
            price: Some("100.25".to_owned()),
            base_price: Some("99.00".to_owned()),
            change_amount: Some("1.25".to_owned()),
            change_percent: Some("1.26".to_owned()),
            direction: Some(IntradayQuoteDirection::Up),
            halted: Some(false),
            received_at: Some(at - Duration::seconds(10)),
            last_success_at: Some(at - Duration::seconds(10)),
            quote_version: 1,
            last_attempt_at: at - Duration::seconds(1),
            last_failure_code: None,
            last_failure_at: None,
            producer_fence: 1,
            created_at: at - Duration::seconds(20),
            updated_at: at - Duration::seconds(1),
        }
    }

    fn project(
        active: bool,
        calendar: &IntradayCalendarReadState,
        window: Arc<IntradaySessionWindowContract>,
        cache: Option<&IntradayCacheRecord>,
        at: DateTime<Utc>,
    ) -> IntradayQuoteDto {
        project_owner_intraday_quote(
            &config(Some(window)),
            &identity(active),
            Some(calendar),
            cache,
            false,
            at,
        )
    }

    #[test]
    fn projection_disabled_and_calendar_window_gates_are_fail_closed() {
        let owner = identity(true);
        let disabled = project_owner_intraday_quote(
            &OwnerIntradayQuoteReadConfig::Disabled,
            &owner,
            None,
            None,
            false,
            now(),
        );
        assert_eq!(disabled.market_state, IntradayQuoteMarketState::Unknown);
        assert_eq!(disabled.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            disabled.reason_code,
            Some(IntradayQuoteReasonCode::FeatureDisabled)
        );
        assert!(disabled.session.is_none() && disabled.quote.is_none());

        let missing = project_owner_intraday_quote(&config(None), &owner, None, None, false, now());
        assert_eq!(
            missing.reason_code,
            Some(IntradayQuoteReasonCode::CalendarUnavailable)
        );

        let old_read_time = calendar(
            IntradayCalendarDisposition::Trading,
            now() - Duration::hours(36) - Duration::seconds(1),
        );
        let result = project(
            true,
            &old_read_time,
            window("REGULAR", Some("09:00:00"), Some("15:30:00")),
            None,
            now(),
        );
        assert_eq!(result.market_state, IntradayQuoteMarketState::Open);
        assert_eq!(result.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            result.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
        assert!(result.session.is_some());

        let mut invalid_calendar = calendar(IntradayCalendarDisposition::Trading, now());
        invalid_calendar.calendar_source_batch_id = Uuid::nil();
        let invalid = project(
            true,
            &invalid_calendar,
            window("REGULAR", Some("09:00:00"), Some("15:30:00")),
            None,
            now(),
        );
        assert_eq!(
            invalid.reason_code,
            Some(IntradayQuoteReasonCode::CalendarUnavailable)
        );
        let unavailable = project_owner_intraday_quote(
            &config(Some(window("REGULAR", Some("09:00:00"), Some("15:30:00")))),
            &owner,
            Some(&calendar(IntradayCalendarDisposition::Trading, now())),
            None,
            true,
            now(),
        );
        assert_eq!(
            unavailable.reason_code,
            Some(IntradayQuoteReasonCode::CalendarUnavailable)
        );

        let db_read_after_api_sample = calendar(
            IntradayCalendarDisposition::Trading,
            now() + Duration::milliseconds(1),
        );
        let result = project(
            true,
            &db_read_after_api_sample,
            window("REGULAR", Some("09:00:00"), Some("15:30:00")),
            None,
            now(),
        );
        assert_eq!(result.market_state, IntradayQuoteMarketState::Open);
        assert_eq!(
            result.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
        assert!(result.session.is_some());

        let rollover = calendar(IntradayCalendarDisposition::Trading, now());
        let result = project(
            true,
            &rollover,
            window("REGULAR", Some("09:00:00"), Some("15:30:00")),
            None,
            now() + Duration::days(1),
        );
        assert_eq!(
            result.reason_code,
            Some(IntradayQuoteReasonCode::CalendarUnavailable)
        );
    }

    #[test]
    fn projection_aged_out_quote_is_pending_without_a_usable_last_good() {
        let at = now();
        let cal = calendar(IntradayCalendarDisposition::Trading, at);
        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let mut record = cache(&identity(true), &cal, &win, at);
        let exactly_at_boundary = at - Duration::hours(24);
        record.received_at = Some(exactly_at_boundary);
        record.last_success_at = Some(exactly_at_boundary);
        record.last_attempt_at = at - Duration::seconds(1);
        let stale = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(stale.freshness, IntradayQuoteFreshness::Stale);
        assert_eq!(stale.reason_code, Some(IntradayQuoteReasonCode::QuoteStale));
        assert!(stale.quote.is_some());

        let aged_out = at - Duration::hours(24) - Duration::milliseconds(1);
        record.received_at = Some(aged_out);
        record.last_success_at = Some(aged_out);
        record.last_attempt_at = at - Duration::seconds(1);

        let result = project(true, &cal, win, Some(&record), at);
        assert_eq!(result.market_state, IntradayQuoteMarketState::Open);
        assert_eq!(result.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            result.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
        assert!(result.quote.is_none());
    }

    #[test]
    fn projection_open_freshness_demand_and_failure_precedence_are_independent() {
        let at = now();
        let cal = calendar(IntradayCalendarDisposition::Trading, at);
        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let mut record = cache(&identity(true), &cal, &win, at);

        let recent = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(recent.market_state, IntradayQuoteMarketState::Open);
        assert_eq!(recent.freshness, IntradayQuoteFreshness::Recent);
        assert!(recent.reason_code.is_none() && recent.quote.is_some());

        record.last_success_at = Some(at - Duration::seconds(30));
        record.received_at = record.last_success_at;
        let exact_recent = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(exact_recent.freshness, IntradayQuoteFreshness::Recent);
        assert!(exact_recent.reason_code.is_none());

        record.last_success_at = Some(at - Duration::seconds(30) - Duration::milliseconds(1));
        record.received_at = record.last_success_at;
        let stale = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(stale.freshness, IntradayQuoteFreshness::Stale);
        assert_eq!(stale.reason_code, Some(IntradayQuoteReasonCode::QuoteStale));
        assert!(stale.quote.is_some());

        let no_demand = project(false, &cal, win.clone(), Some(&record), at);
        assert_eq!(
            no_demand.reason_code,
            Some(IntradayQuoteReasonCode::NoActiveDemand)
        );
        assert!(no_demand.quote.is_some());

        record.last_failure_code = Some(IntradayQuoteFailureCode::ProviderTimeout);
        record.last_failure_at = Some(at - Duration::seconds(1));
        let failed = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(
            failed.reason_code,
            Some(IntradayQuoteReasonCode::ProviderTimeout)
        );
        assert!(failed.quote.is_some());

        record.last_failure_at = Some(at - Duration::minutes(3));
        record.last_success_at = Some(at - Duration::minutes(2));
        record.received_at = record.last_success_at;
        let older_failure = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(
            older_failure.reason_code,
            Some(IntradayQuoteReasonCode::QuoteStale)
        );

        record.last_failure_code = Some(IntradayQuoteFailureCode::FeatureDisabled);
        let ignored_failure = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(
            ignored_failure.reason_code,
            Some(IntradayQuoteReasonCode::QuoteStale)
        );

        record.last_success_at = None;
        record.received_at = None;
        let pending = project(true, &cal, win, Some(&record), at);
        assert_eq!(pending.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            pending.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
        assert!(pending.quote.is_none());
    }

    #[test]
    fn projection_maps_all_eligible_failures_and_rejects_future_or_invalid_quotes() {
        let at = now();
        let cal = calendar(IntradayCalendarDisposition::Trading, at);
        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let eligible = [
            (
                IntradayQuoteFailureCode::ProviderTimeout,
                IntradayQuoteReasonCode::ProviderTimeout,
            ),
            (
                IntradayQuoteFailureCode::ProviderRateLimited,
                IntradayQuoteReasonCode::ProviderRateLimited,
            ),
            (
                IntradayQuoteFailureCode::ProviderUnavailable,
                IntradayQuoteReasonCode::ProviderUnavailable,
            ),
            (
                IntradayQuoteFailureCode::ProviderResponseInvalid,
                IntradayQuoteReasonCode::ProviderResponseInvalid,
            ),
            (
                IntradayQuoteFailureCode::QuoteValueInvalid,
                IntradayQuoteReasonCode::QuoteValueInvalid,
            ),
            (
                IntradayQuoteFailureCode::QuoteBudgetExhausted,
                IntradayQuoteReasonCode::QuoteBudgetExhausted,
            ),
            (
                IntradayQuoteFailureCode::ProducerUnavailable,
                IntradayQuoteReasonCode::ProducerUnavailable,
            ),
        ];
        for (failure, expected) in eligible {
            let mut record = cache(&identity(true), &cal, &win, at);
            record.last_failure_code = Some(failure);
            record.last_failure_at = Some(at - Duration::seconds(1));
            let result = project(true, &cal, win.clone(), Some(&record), at);
            assert_eq!(result.reason_code, Some(expected));
            assert!(
                result.quote.is_some(),
                "{failure:?} must preserve last good quote"
            );
        }

        for ignored in [
            IntradayQuoteFailureCode::NoActiveDemand,
            IntradayQuoteFailureCode::QuotePending,
            IntradayQuoteFailureCode::QuoteStale,
            IntradayQuoteFailureCode::CalendarUnavailable,
            IntradayQuoteFailureCode::SessionWindowUnavailable,
            IntradayQuoteFailureCode::SessionClosed,
            IntradayQuoteFailureCode::InstrumentHalted,
            IntradayQuoteFailureCode::FeatureDisabled,
        ] {
            let mut record = cache(&identity(true), &cal, &win, at);
            record.last_failure_code = Some(ignored);
            record.last_failure_at = Some(at - Duration::seconds(1));
            let result = project(true, &cal, win.clone(), Some(&record), at);
            assert_eq!(
                result.reason_code, None,
                "{ignored:?} is not an eligible failure"
            );
        }

        for future_field in 0..4 {
            let mut record = cache(&identity(true), &cal, &win, at);
            match future_field {
                0 => {
                    record.received_at = Some(at + Duration::seconds(1));
                }
                1 => {
                    record.last_success_at = Some(at + Duration::seconds(1));
                }
                2 => record.last_attempt_at = at + Duration::seconds(1),
                3 => record.last_failure_at = Some(at + Duration::seconds(1)),
                _ => unreachable!(),
            }
            let result = project(true, &cal, win.clone(), Some(&record), at);
            assert_eq!(result.market_state, IntradayQuoteMarketState::Unknown);
            assert_eq!(
                result.reason_code,
                Some(IntradayQuoteReasonCode::ProducerUnavailable)
            );
            assert!(result.session.is_none() && result.quote.is_none());
        }

        let mut invalid = cache(&identity(true), &cal, &win, at);
        invalid.price = Some("not-a-decimal".to_owned());
        let invalid_result = project(true, &cal, win.clone(), Some(&invalid), at);
        assert_eq!(
            invalid_result.freshness,
            IntradayQuoteFreshness::Unavailable
        );
        assert_eq!(
            invalid_result.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
        invalid.price = Some("100.25".to_owned());
        invalid.quote_version = MAX_GENERATION + 1;
        let invalid_version = project(true, &cal, win, Some(&invalid), at);
        assert_eq!(
            invalid_version.freshness,
            IntradayQuoteFreshness::Unavailable
        );
        assert_eq!(
            invalid_version.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );
    }

    #[test]
    fn projection_session_boundaries_closed_halted_and_closed_calendar_are_exact() {
        let cal = calendar(IntradayCalendarDisposition::Trading, now());
        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let before_open = project(true, &cal, win.clone(), None, kst(8, 59, 59));
        assert_eq!(before_open.market_state, IntradayQuoteMarketState::Closed);
        assert_eq!(
            before_open.reason_code,
            Some(IntradayQuoteReasonCode::SessionClosed)
        );

        let open_boundary = project(true, &cal, win.clone(), None, kst(9, 0, 0));
        assert_eq!(open_boundary.market_state, IntradayQuoteMarketState::Open);

        let before_close = project(true, &cal, win.clone(), None, kst(15, 29, 59));
        assert_eq!(before_close.market_state, IntradayQuoteMarketState::Open);

        let close = kst(15, 30, 0);
        let record_at_close = cache(&identity(true), &cal, &win, close);
        let closed_boundary = project(true, &cal, win.clone(), Some(&record_at_close), close);
        assert_eq!(
            closed_boundary.market_state,
            IntradayQuoteMarketState::Closed
        );
        assert_eq!(
            closed_boundary.reason_code,
            Some(IntradayQuoteReasonCode::SessionClosed)
        );
        assert!(closed_boundary.quote.is_some());

        let mut record = cache(&identity(true), &cal, &win, now());
        record.halted = Some(true);
        let halted = project(true, &cal, win.clone(), Some(&record), now());
        assert_eq!(halted.market_state, IntradayQuoteMarketState::Halted);
        assert_eq!(
            halted.reason_code,
            Some(IntradayQuoteReasonCode::InstrumentHalted)
        );

        let closed_cal = calendar(IntradayCalendarDisposition::Closed, now());
        let closed_win = window("CLOSED", None, None);
        let closed = project(true, &closed_cal, closed_win, Some(&record), now());
        assert_eq!(closed.market_state, IntradayQuoteMarketState::Closed);
        assert_eq!(closed.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            closed.reason_code,
            Some(IntradayQuoteReasonCode::SessionClosed)
        );
        assert!(closed.quote.is_none());

        let special = window("SPECIAL", Some("10:00:00"), Some("14:00:00"));
        let special_open = project(true, &cal, special.clone(), None, kst(10, 0, 0));
        assert_eq!(special_open.market_state, IntradayQuoteMarketState::Open);
        let special_closed = project(true, &cal, special, None, kst(14, 0, 0));
        assert_eq!(
            special_closed.market_state,
            IntradayQuoteMarketState::Closed
        );
    }

    #[test]
    fn projection_rejects_window_disagreement_lineage_identity_and_future_evidence() {
        let at = now();
        let cal = calendar(IntradayCalendarDisposition::Trading, at);
        let closed = project(true, &cal, window("CLOSED", None, None), None, at);
        assert_eq!(
            closed.reason_code,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable)
        );
        let closed_calendar = calendar(IntradayCalendarDisposition::Closed, at);
        let reverse = project(
            true,
            &closed_calendar,
            window("REGULAR", Some("09:00:00"), Some("15:30:00")),
            None,
            at,
        );
        assert_eq!(
            reverse.reason_code,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable)
        );

        let future_evidence = window_with_evidence(
            "REGULAR",
            Some("09:00:00"),
            Some("15:30:00"),
            "2026-09-08T15:00:00Z",
        );
        let future_window = project(true, &cal, future_evidence, None, at);
        assert_eq!(
            future_window.reason_code,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable)
        );
        let prior_day_evidence = window_with_evidence(
            "REGULAR",
            Some("09:00:00"),
            Some("15:30:00"),
            "2026-09-06T15:00:00Z",
        );
        let prior_window = project(true, &cal, prior_day_evidence, None, at);
        assert_eq!(
            prior_window.reason_code,
            Some(IntradayQuoteReasonCode::SessionWindowUnavailable)
        );

        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let mut record = cache(&identity(true), &cal, &win, at);
        for mismatch in [
            {
                record.owner_user_id = Uuid::from_u128(99);
                record.clone()
            },
            {
                record.owner_user_id = Uuid::from_u128(1);
                record.membership_id = Uuid::from_u128(99);
                record.clone()
            },
            {
                record.membership_id = Uuid::from_u128(1);
                record.generation_id = Uuid::from_u128(99);
                record.clone()
            },
            {
                record.generation_id = Uuid::from_u128(3);
                record.clone()
            },
            {
                record.generation = 2;
                record.clone()
            },
            {
                record.generation = 1;
                record.instrument_id = "229200.KRX".to_owned();
                record.clone()
            },
            {
                record.instrument_id = INSTRUMENT.to_owned();
                record.session_date = Some(date() - Duration::days(1));
                record.clone()
            },
            {
                record.session_date = Some(date());
                record.calendar_source = Some("other".to_owned());
                record.clone()
            },
            {
                record.calendar_source = Some("kis".to_owned());
                record.calendar_source_version = Some("other".to_owned());
                record.clone()
            },
            {
                record.calendar_source_version = Some("kis-chk-holiday-v1:schema-1".to_owned());
                record.calendar_source_batch_id = Some(Uuid::from_u128(99));
                record.clone()
            },
            {
                record.calendar_source_batch_id = Some(Uuid::from_u128(4));
                record.calendar_content_sha256 = Some("d".repeat(64));
                record.clone()
            },
            {
                record.calendar_content_sha256 = Some(CALENDAR_HASH.to_owned());
                record.window_contract_sha256 = Some("sha256:wrong".to_owned());
                record.clone()
            },
        ] {
            let mismatch = project(true, &cal, win.clone(), Some(&mismatch), at);
            assert_eq!(
                mismatch.reason_code,
                Some(IntradayQuoteReasonCode::QuotePending)
            );
            assert!(mismatch.quote.is_none());
        }

        record = cache(&identity(true), &cal, &win, at);
        record.quote_version = 0;
        let invalid = project(true, &cal, win.clone(), Some(&record), at);
        assert_eq!(invalid.freshness, IntradayQuoteFreshness::Unavailable);
        assert_eq!(
            invalid.reason_code,
            Some(IntradayQuoteReasonCode::QuotePending)
        );

        for incomplete_field in 0..8 {
            let mut incomplete = cache(&identity(true), &cal, &win, at);
            match incomplete_field {
                0 => incomplete.price = None,
                1 => incomplete.base_price = None,
                2 => incomplete.change_amount = None,
                3 => incomplete.change_percent = None,
                4 => incomplete.direction = None,
                5 => incomplete.halted = None,
                6 => incomplete.received_at = None,
                7 => incomplete.last_success_at = None,
                _ => unreachable!(),
            }
            let result = project(true, &cal, win.clone(), Some(&incomplete), at);
            assert_eq!(result.freshness, IntradayQuoteFreshness::Unavailable);
            assert_eq!(
                result.reason_code,
                Some(IntradayQuoteReasonCode::QuotePending)
            );
            assert!(result.quote.is_none());
        }

        record.quote_version = 1;
        record.last_success_at = Some(at + Duration::seconds(1));
        record.received_at = record.last_success_at;
        let future = project(true, &cal, win, Some(&record), at);
        assert_eq!(future.market_state, IntradayQuoteMarketState::Unknown);
        assert_eq!(
            future.reason_code,
            Some(IntradayQuoteReasonCode::ProducerUnavailable)
        );
        assert!(future.session.is_none() && future.quote.is_none());
    }

    #[test]
    fn projection_serializes_the_closed_dto_shape_without_private_fields() {
        let owner = identity(true);
        let dto = project_owner_intraday_quote(
            &OwnerIntradayQuoteReadConfig::Disabled,
            &owner,
            None,
            None,
            false,
            now(),
        );
        let value = serde_json::to_value(dto).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            [
                "schema_version",
                "membership_id",
                "instrument_id",
                "venue",
                "currency",
                "generation",
                "session",
                "market_state",
                "freshness",
                "reason_code",
                "quote",
                "next_poll_after_ms",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
        assert_eq!(value["session"], Value::Null);
        assert_eq!(value["quote"], Value::Null);
        assert_eq!(value["reason_code"], "FEATURE_DISABLED");
    }

    #[test]
    fn request_target_rejects_unknown_duplicate_and_out_of_range_fields() {
        let uri = OriginalUri(
            "/api/v1/research/owner-beta/equity-universe-v2/instruments/069500.KRX/quote?membership_id=00000000-0000-0000-0000-000000000002&generation=1"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            parse_quote_get_input(&uri),
            Some((INSTRUMENT.to_owned(), Uuid::from_u128(2), 1,))
        );
        for query in [
            "membership_id=00000000-0000-0000-0000-000000000002&generation=1&x=1",
            "membership_id=00000000-0000-0000-0000-000000000002&membership_id=00000000-0000-0000-0000-000000000002&generation=1",
            "membership_id=00000000-0000-0000-0000-000000000002&generation=9223372036854775808",
        ] {
            let uri = OriginalUri(
                format!(
                    "/api/v1/research/owner-beta/equity-universe-v2/instruments/{INSTRUMENT}/quote?{query}"
                )
                .parse()
                .unwrap(),
            );
            assert!(
                parse_quote_get_input(&uri).is_none(),
                "query must fail: {query}"
            );
        }
    }

    #[test]
    fn session_proof_lineage_used_by_the_projection_is_exact() {
        let at = now();
        let cal = calendar(IntradayCalendarDisposition::Trading, at);
        let win = window("REGULAR", Some("09:00:00"), Some("15:30:00"));
        let proof = IntradaySessionProof::new(
            cal.session_date,
            cal.calendar_source_batch_id,
            cal.calendar_content_sha256.clone(),
            win.window_contract_sha256().to_owned(),
        )
        .unwrap();
        assert_eq!(proof.calendar_source(), "kis");
        assert_eq!(
            proof.calendar_source_version(),
            "kis-chk-holiday-v1:schema-1"
        );
    }
}
