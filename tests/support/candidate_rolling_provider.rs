use chrono::{Datelike, Duration, NaiveDate, Weekday};
use domain::TradingDate;
use market_data::{
    EodProvider, FetchMode, FetchRequest, ProviderError, RawEnvelope, RequestMetadata, ResponseKind,
};
use serde_json::{Value, json};

pub const ROLLING_MEMBERS: [&str; 7] = [
    "100001.KRX",
    "100002.KRX",
    "100003.KRX",
    "100004.KRX",
    "100005.KRX",
    "100006.KRX",
    "100007.KRX",
];

#[derive(Debug, Clone, Copy)]
pub struct RollingCandidateProvider;

#[derive(Debug, Clone, Copy)]
pub struct CredentialedRollingCandidateProvider;

impl RollingCandidateProvider {
    const RECENT_LISTING: &'static str = "2026-08-14";

    pub fn sessions(as_of: TradingDate) -> Vec<TradingDate> {
        let mut cursor = as_of.as_naive_date();
        let mut sessions = Vec::with_capacity(60);
        while sessions.len() < 60 {
            if !matches!(cursor.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions
                    .push(TradingDate::parse(&cursor.to_string()).expect("generated trading date"));
            }
            cursor -= Duration::days(1);
        }
        sessions.reverse();
        sessions
    }

    pub fn price_sessions(as_of: TradingDate) -> Vec<TradingDate> {
        let mut sessions = Self::sessions(as_of);
        let mut cursor = sessions[0].as_naive_date() - Duration::days(1);
        loop {
            if !matches!(cursor.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions.insert(
                    0,
                    TradingDate::parse(&cursor.to_string()).expect("generated price date"),
                );
                return sessions;
            }
            cursor -= Duration::days(1);
        }
    }

    fn body(kind: ResponseKind, request: &FetchRequest) -> Result<Value, ProviderError> {
        let sessions = Self::sessions(request.date);
        let price_sessions = Self::price_sessions(request.date);
        let as_of = request.date.to_iso();
        let available_at = request.now.as_datetime().to_rfc3339();
        let full_members = &ROLLING_MEMBERS[..ROLLING_MEMBERS.len() - 1];
        Ok(match kind {
            ResponseKind::Bars => {
                let mut bars = Vec::new();
                for (instrument_index, instrument) in ROLLING_MEMBERS.iter().enumerate() {
                    let covered = if instrument_index < full_members.len() {
                        price_sessions.as_slice()
                    } else {
                        let first = price_sessions
                            .iter()
                            .position(|session| session.to_iso().as_str() >= Self::RECENT_LISTING)
                            .expect("rolling window includes the recent listing");
                        &price_sessions[first..]
                    };
                    for session in covered {
                        let close = 10_000
                            + instrument_index as i64 * 500
                            + i64::from(session.as_naive_date().num_days_from_ce() % 1_000);
                        bars.push(json!({
                            "instrument": instrument,
                            "date": session.to_iso(),
                            "open": close - 10,
                            "high": close + 20,
                            "low": close - 20,
                            "close": close,
                            "volume": 1_000_000 + instrument_index as i64 * 10_000,
                            "value": close * 1_000_000
                        }));
                    }
                }
                json!({
                    "dataset_id": format!("candidate-rolling-{as_of}"),
                    "schema_version": 1,
                    "source": "synthetic",
                    "rights": "SYNTHETIC_ONLY",
                    "currency": "KRW",
                    "instruments": ROLLING_MEMBERS.iter().map(|instrument| json!({
                        "symbol": instrument, "lot_size": 1, "currency": "KRW"
                    })).collect::<Vec<_>>(),
                    "bars": bars
                })
            }
            ResponseKind::Reference => json!({
                "dataset_id": format!("candidate-reference-{as_of}"),
                "schema_version": 1,
                "source": "synthetic",
                "timezone": "Asia/Seoul",
                "instruments": ROLLING_MEMBERS.iter().enumerate().map(|(index, instrument)| json!({
                    "symbol": instrument,
                    "name": format!("Synthetic candidate {}", index + 1),
                    "lot_size": 1,
                    "currency": "KRW",
                    "kind": "equity",
                    "listed_at": if index == ROLLING_MEMBERS.len() - 1 { Self::RECENT_LISTING } else { "2020-01-02" }
                })).collect::<Vec<_>>()
            }),
            ResponseKind::Calendar => json!({
                "calendar_id": format!("candidate-calendar-{as_of}"),
                "schema_version": 1,
                "source": "synthetic",
                "timezone": "Asia/Seoul",
                "utc_offset": "+09:00",
                "session_times_local": {"open":"09:00:00","close":"15:30:00"},
                "session_times_utc": {"open":"00:00:00","close":"06:30:00"},
                "sessions": price_sessions.iter().map(|session| {
                    let date = session.as_naive_date();
                    json!({
                        "date": session.to_iso(),
                        "weekday": weekday_name(date),
                        "open_utc": format!("{}T00:00:00Z", session.to_iso()),
                        "close_utc": format!("{}T06:30:00Z", session.to_iso())
                    })
                }).collect::<Vec<_>>(),
                "holidays": [],
                "no_dst_note": "Asia/Seoul has no DST"
            }),
            ResponseKind::CorporateActions => json!({
                "schema_version": 1,
                "dataset_id": format!("candidate-rolling-{as_of}"),
                "source": "synthetic",
                "actions": []
            }),
            ResponseKind::InvestorFlow => {
                let mut flows = Vec::new();
                for (instrument_index, instrument) in ROLLING_MEMBERS.iter().enumerate() {
                    let covered = if instrument_index < full_members.len() {
                        sessions.as_slice()
                    } else {
                        let first = sessions
                            .iter()
                            .position(|session| session.to_iso().as_str() >= Self::RECENT_LISTING)
                            .expect("rolling window includes the recent listing");
                        &sessions[first..]
                    };
                    for session in covered {
                        let absolute_day = i64::from(session.as_naive_date().num_days_from_ce());
                        for (class, multiplier) in [("FOREIGN", 10_i64), ("INSTITUTION", 7_i64)] {
                            flows.push(json!({
                                "instrument": instrument,
                                "trade_date": session.to_iso(),
                                "investor_class": class,
                                "net_amount": (instrument_index as i64 + 1) * absolute_day * multiplier * 100_000,
                                "net_volume": (instrument_index as i64 + 1) * absolute_day * multiplier,
                                "currency": "KRW",
                                "volume_unit": "SHARE",
                                "source_revision": format!("rolling-flow-{}", session.to_iso()),
                                "available_at": format!("{}T06:50:00Z", session.to_iso())
                            }));
                        }
                    }
                }
                json!({"flows": flows})
            }
            ResponseKind::MarketStatus => json!({
                "statuses": ROLLING_MEMBERS.iter().map(|instrument| json!({
                    "instrument": instrument,
                    "trade_date": as_of,
                    "suspended": false,
                    "administrative": false,
                    "liquidation": false,
                    "inactive": false,
                    "disqualifying_audit_opinion": false,
                    "complete_capital_impairment": false,
                    "source_revision": format!("rolling-status-{as_of}"),
                    "available_at": available_at
                })).collect::<Vec<_>>()
            }),
            ResponseKind::Fundamentals => {
                let mut fundamentals = Vec::new();
                for (instrument_index, instrument) in ROLLING_MEMBERS.iter().enumerate() {
                    for (metric_index, (metric, base)) in [
                        ("revenue_growth", 0.03),
                        ("operating_margin", 0.08),
                        ("roe", 0.10),
                        ("debt_ratio", 1.20),
                        ("cash_conversion", 0.60),
                        ("earnings_yield", 0.05),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        fundamentals.push(json!({
                            "instrument": instrument,
                            "fiscal_period_start": "2025-01-01",
                            "fiscal_period_end": "2025-12-31",
                            "period_kind": "ANNUAL",
                            "statement_scope": "CONSOLIDATED",
                            "metric": metric,
                            "value": base + instrument_index as f64 * 0.01,
                            "currency": null,
                            "unit_scale": 1,
                            "audited": true,
                            "disclosed_at": "2026-03-15T00:00:00Z",
                            "available_at": "2026-03-15T00:05:00Z",
                            "source_revision": format!("rolling-fundamental-{instrument_index}-{metric_index}"),
                            "restates_source_revision": null
                        }));
                    }
                }
                json!({"fundamentals": fundamentals})
            }
            ResponseKind::IndexMembership => {
                let memberships = ["kospi200", "kosdaq150"].into_iter().flat_map(|universe| {
                    ROLLING_MEMBERS.iter().enumerate().map(move |(index, instrument)| json!({
                        "index_id": universe,
                        "instrument": instrument,
                        "announced_at": if index == ROLLING_MEMBERS.len() - 1 { "2026-08-14T00:00:00Z" } else { "2020-01-01T00:00:00Z" },
                        "effective_from": if index == ROLLING_MEMBERS.len() - 1 { "2026-08-14" } else { "2020-01-02" },
                        "effective_until": null,
                        "available_at": if index == ROLLING_MEMBERS.len() - 1 { "2026-08-14T00:05:00Z" } else { "2020-01-01T00:05:00Z" },
                        "source_revision": format!("rolling-{universe}-membership-v1")
                    }))
                }).collect::<Vec<_>>();
                json!({ "memberships": memberships })
            }
            ResponseKind::SectorClassification => json!({
                "sectors": ROLLING_MEMBERS.iter().enumerate().map(|(index, instrument)| json!({
                    "taxonomy_id": "krx-sector",
                    "taxonomy_version": "rolling-2026",
                    "instrument": instrument,
                    "sector_code": format!("G{:02}", 20 + index),
                    "sector_name": format!("Synthetic sector {index}"),
                    "fundamental_profile": "NON_FINANCIAL",
                    "effective_from": "2020-01-02",
                    "effective_until": null,
                    "available_at": "2020-01-01T00:05:00Z",
                    "source_revision": "rolling-sector-v1"
                })).collect::<Vec<_>>()
            }),
            ResponseKind::CandidateMaster => return Err(ProviderError::UnsupportedKind(kind)),
        })
    }
}

impl EodProvider for RollingCandidateProvider {
    fn provider_id(&self) -> &'static str {
        "krx"
    }

    fn fetch_mode(&self) -> FetchMode {
        FetchMode::Synthetic
    }

    fn fetch(&self, request: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        request
            .kinds
            .iter()
            .copied()
            .map(|kind| -> Result<RawEnvelope, ProviderError> {
                let bytes = serde_json::to_vec(&Self::body(kind, request)?)
                    .expect("rolling candidate fixture serializes");
                Ok(RawEnvelope::new(
                    request.batch_id,
                    kind,
                    format!("{}-response.json", kind.as_str()),
                    bytes,
                    request.now,
                    RequestMetadata {
                        endpoint: format!("fixture.{}.v1", kind.as_str()),
                        query: vec![("date".to_owned(), request.date.to_iso())],
                        headers: vec![("X-Data-License".to_owned(), "redacted".to_owned())],
                        mode: FetchMode::Synthetic,
                    },
                ))
            })
            .collect()
    }
}

impl EodProvider for CredentialedRollingCandidateProvider {
    fn provider_id(&self) -> &'static str {
        "krx"
    }

    fn fetch_mode(&self) -> FetchMode {
        FetchMode::Credentialed
    }

    fn fetch(&self, request: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        request
            .kinds
            .iter()
            .copied()
            .map(|kind| -> Result<RawEnvelope, ProviderError> {
                let bytes = serde_json::to_vec(&RollingCandidateProvider::body(kind, request)?)
                    .expect("rolling candidate fixture serializes");
                Ok(RawEnvelope::new(
                    request.batch_id,
                    kind,
                    format!("{}-response.json", kind.as_str()),
                    bytes,
                    request.now,
                    RequestMetadata {
                        endpoint: format!("fixture.{}.v1", kind.as_str()),
                        query: vec![("date".to_owned(), request.date.to_iso())],
                        headers: vec![("X-Data-License".to_owned(), "redacted".to_owned())],
                        mode: FetchMode::Credentialed,
                    },
                ))
            })
            .collect()
    }
}

fn weekday_name(date: NaiveDate) -> &'static str {
    match date.weekday() {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}
