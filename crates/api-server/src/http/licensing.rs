//! Licensing-status route: the actor's fail-closed KR data entitlement view,
//! derived from the in-memory [`EntitlementService`] (loaded from
//! `data_entitlements`). One row per (dataset, use) with the governing
//! lifecycle state and coverage verdict.

use crate::http::dto::{LicensingDatasetDto, LicensingStatusDto};
use crate::http::entitlement::today_iso;
use crate::http::session::Session;
use crate::http::state::ApiState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use std::collections::BTreeMap;

pub async fn status(State(state): State<ApiState>, session: Session) -> Response {
    let as_of = today_iso();
    let date = NaiveDate::parse_from_str(&as_of, "%Y-%m-%d")
        .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixed fallback"));
    let calendar = auth::entitlement::CalendarDate::parse(&as_of)
        .unwrap_or_else(|_| auth::entitlement::CalendarDate::parse("2026-01-01").expect("fixed"));
    let actor = session.actor();
    // Reload from data_entitlements so lifecycle changes reflect immediately.
    let service = match crate::http::entitlement::fresh_service(&state).await {
        Ok(s) => s,
        Err(r) => return r,
    };

    // Deterministic ordering: dataset, use, latest effective_until first.
    let mut rows: BTreeMap<(String, String), (String, String, String, bool)> = BTreeMap::new();
    for e in service.entitlements() {
        let state_on = e.status_on(calendar);
        let state_str = match state_on {
            auth::entitlement::EntitlementState::Pending => "PENDING",
            auth::entitlement::EntitlementState::Active => "ACTIVE",
            auth::entitlement::EntitlementState::Expired => "EXPIRED",
            auth::entitlement::EntitlementState::Revoked => "REVOKED",
        };
        for dataset in &e.covered_datasets {
            for use_kind in &e.covered_uses {
                let covered = service
                    .authorize_use(
                        *use_kind,
                        &auth::entitlement::AccessRequest {
                            actor: actor.clone(),
                            dataset: dataset.clone(),
                            as_of: calendar,
                        },
                    )
                    .is_ok();
                let key = (dataset.0.clone(), use_kind.as_str().to_string());
                let entry = (
                    state_str.to_string(),
                    e.effective_from.to_string(),
                    e.effective_until.to_string(),
                    covered,
                );
                rows.entry(key)
                    .and_modify(|existing| {
                        if entry.2 > existing.2 {
                            *existing = entry.clone();
                        }
                    })
                    .or_insert(entry);
            }
        }
    }

    let datasets = rows
        .into_iter()
        .map(
            |((dataset_id, use_kind), (state, from, until, covered))| LicensingDatasetDto {
                dataset_id,
                use_kind,
                state,
                effective_from: NaiveDate::parse_from_str(&from, "%Y-%m-%d").ok(),
                effective_until: NaiveDate::parse_from_str(&until, "%Y-%m-%d").ok(),
                covered,
            },
        )
        .collect();
    (
        StatusCode::OK,
        Json(LicensingStatusDto {
            as_of: date,
            datasets,
        }),
    )
        .into_response()
}
