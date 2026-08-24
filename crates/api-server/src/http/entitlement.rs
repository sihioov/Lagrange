//! Entitlement gate for KR-derived Member-visible surfaces (Todo 5 policy,
//! fail closed). The router holds one [`EntitlementService`] built from the
//! `data_entitlements` rows; every route declaring an `entitlement_use` in
//! [`crate::contract::CONTRACT_ROUTES`] calls [`require_use`] before
//! touching any KR-derived data.

use crate::http::error::api_error;
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::validation::parse_date;
use auth::entitlement::{
    AccessRequest, CalendarDate, DatasetId, EntitlementService, KrUse, UserId,
};
use axum::http::HeaderMap;
use axum::response::Response;

/// The canonical KR end-of-day bars dataset for member-visible surfaces.
pub const KRX_EOD_DATASET: &str = "krx_eod_bars";

/// Map a contract use name to a [`KrUse`]; unknown uses fail closed.
pub fn use_from_name(name: &str) -> Option<KrUse> {
    match name {
        "dataset" => Some(KrUse::Dataset),
        "factor" => Some(KrUse::Factor),
        "recommendation" => Some(KrUse::Recommendation),
        "candidate" => Some(KrUse::Candidate),
        "backtest" => Some(KrUse::Backtest),
        "report" => Some(KrUse::Report),
        "benchmark" => Some(KrUse::Benchmark),
        "paper_view" => Some(KrUse::PaperView),
        "payload" => Some(KrUse::Payload),
        "download" => Some(KrUse::Download),
        _ => None,
    }
}

/// Build the entitlement service FRESH from `data_entitlements` so lifecycle
/// changes (revoke/expire) take effect immediately (fail closed).
pub async fn fresh_service(state: &ApiState) -> Result<EntitlementService, Response> {
    let rid = "entitlement-load";
    match crate::repos::entitlements::EntitlementRepo::new(state.app_pool.clone())
        .load()
        .await
    {
        Ok(rows) => Ok(EntitlementService::new(rows)),
        Err(_) => Err(api_error(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            "internal error",
            rid,
            None,
        )),
    }
}

/// Grant access for `use_kind` as-of `date` (ISO `YYYY-MM-DD`); on denial
/// returns the typed 403 `DATA_ENTITLEMENT_REQUIRED` envelope.
pub async fn require_use(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    use_kind: KrUse,
    as_of: &str,
) -> Result<(), Response> {
    require_dataset_use(state, session, headers, use_kind, KRX_EOD_DATASET, as_of).await
}

/// Grant access for one exact logical dataset/use pair. Multi-source surfaces
/// (the stock-candidate vertical) invoke this for every pinned source rather
/// than treating EOD rights as permission for unrelated flow/fundamental data.
pub async fn require_dataset_use(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    use_kind: KrUse,
    dataset_id: &str,
    as_of: &str,
) -> Result<(), Response> {
    let rid = crate::http::error::request_id(headers);
    let service = fresh_service(state).await?;
    let Some(date) = parse_date(as_of) else {
        return Err(api_error(
            axum::http::StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "as_of must be a valid YYYY-MM-DD calendar date",
            &rid,
            None,
        ));
    };
    let req = AccessRequest {
        actor: session.actor(),
        dataset: DatasetId::new(dataset_id),
        as_of: CalendarDate::parse(&date.format("%Y-%m-%d").to_string()).map_err(|_| {
            api_error(
                axum::http::StatusCode::BAD_REQUEST,
                "INVALID_DATE",
                "as_of must be a valid calendar date",
                &rid,
                None,
            )
        })?,
    };
    match service.authorize_use(use_kind, &req) {
        Ok(_) => Ok(()),
        Err(_) => Err(api_error(
            axum::http::StatusCode::FORBIDDEN,
            "DATA_ENTITLEMENT_REQUIRED",
            format!(
                "an ACTIVE KRX entitlement covering use {} is required",
                use_kind.as_str()
            ),
            &rid,
            None,
        )),
    }
}

/// Convenience: today's date (UTC) as `YYYY-MM-DD` for default as-of.
pub fn today_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// The actor's user id string.
pub fn user_id_of(session: &Session) -> UserId {
    session.0.user_id.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::state::{ApiState, OwnerBetaAccessMode};
    use axum::body::to_bytes;

    #[tokio::test]
    async fn entitlement_load_failure_is_static_and_redacted() {
        let state = ApiState::test_without_database(OwnerBetaAccessMode::Disabled);
        let response = fresh_service(&state)
            .await
            .expect_err("unreachable lazy database must fail");
        let body = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded error body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("error JSON");
        assert_eq!(body["error"]["code"], "INTERNAL");
        assert_eq!(body["error"]["message"], "internal error");
        let serialized = serde_json::to_string(&body).expect("error serializes");
        for forbidden in ["sqlx", "postgres", "unused", "127.0.0.1"] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }
}
