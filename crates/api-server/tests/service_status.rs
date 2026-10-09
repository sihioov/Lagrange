mod common;
#[path = "service_status_reads_support/mod.rs"]
mod service_status_reads_support;
mod service_status_support;

use auth::entitlement::Role;
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use service_status_support::{
    actor_session_hash, audit_rows_for_actor, projection_cases, required_harness, seed_v2_range,
    v2_group, wire_cases,
};
use sqlx::Row;
use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const PATH: &str = "/api/v1/admin/service-status";
const BASE_INPUT: &str = include_str!("../../../tests/fixtures/service-status/base-input.json");
const BASE_OUTPUT: &str = include_str!("../../../tests/fixtures/service-status/base-output.json");
#[tokio::test]
async fn actual_stream_diagnostic_merge_preserves_failures_partial_coverage_and_closed_clocks() {
    use service_status_reads_support::{
        EvidenceCase, actor_session_pool, seed_demand_fixture, seed_live_quote_fixture,
        seed_second_demand_fixture, seed_stream_fixture, set_stream_evidence,
    };
    use service_status_support::{
        attach_synthetic_host, configure_stream_window, seed_calendar_days,
    };
    let h = required_harness().await;
    let mut stream = seed_stream_fixture(&h).await.expect("actual pinned stream");
    let mut state = configure_stream_window(&h, &mut stream, false).await;
    let first = seed_demand_fixture(&h, &stream)
        .await
        .expect("first authenticated demand");
    seed_live_quote_fixture(&h, &stream, &first)
        .await
        .expect("first complete quote");
    seed_calendar_days(&h).await;
    let app = actor_session_pool(&h, stream.owner_id, &stream.session_hash).await;
    let (session_role, current_role): (String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text")
            .fetch_one(&app)
            .await
            .expect("real app session");
    assert_eq!(
        (session_role.as_str(), current_role.as_str()),
        ("app", "app")
    );

    let mut demands = vec![first];
    let mut lease_count: Option<i64> = None;
    for (
        case,
        received,
        provider,
        halted,
        code,
        closed,
        calendar_failure,
        live,
        halted_count,
        status,
        work,
        reasons,
    ) in [
        (
            "single_future_receipt",
            [-20_000, 1_000],
            [1_000, 1_000],
            [false, false],
            None,
            false,
            false,
            0,
            0,
            "degraded",
            "stale",
            serde_json::json!(["SOURCE_FUTURE"]),
        ),
        (
            "mixed_live",
            [1_000, -20_000],
            [1_000, 1_000],
            [false, false],
            None,
            false,
            false,
            1,
            0,
            "degraded",
            "stale",
            serde_json::json!(["QUOTE_STALE", "SOURCE_FUTURE"]),
        ),
        (
            "mixed_halted",
            [1_000, -20_000],
            [1_000, 1_000],
            [true, false],
            None,
            false,
            false,
            0,
            1,
            "degraded",
            "stale",
            serde_json::json!(["QUOTE_STALE", "SOURCE_FUTURE"]),
        ),
        (
            "future_provider_producer_failure",
            [1_000, 1_000],
            [-20_000, -20_000],
            [false, false],
            Some("CONNECTION_LOST"),
            false,
            false,
            0,
            0,
            "failed",
            "failed",
            serde_json::json!(["CONNECTION_LOST", "SOURCE_FUTURE"]),
        ),
        (
            "future_producer_status",
            [1_000, 1_000],
            [1_000, 1_000],
            [false, false],
            Some("SUBSCRIPTION_PENDING"),
            false,
            false,
            0,
            0,
            "degraded",
            "degraded",
            serde_json::json!(["SOURCE_FUTURE", "SUBSCRIPTION_PENDING"]),
        ),
        (
            "future_provider_calendar_failure",
            [1_000, 1_000],
            [-20_000, -20_000],
            [false, false],
            None,
            false,
            true,
            0,
            0,
            "failed",
            "failed",
            serde_json::json!(["CALENDAR_UNAVAILABLE", "SOURCE_FUTURE"]),
        ),
        (
            "future_provider_closed",
            [1_000, 1_000],
            [-20_000, -20_000],
            [false, false],
            None,
            true,
            false,
            0,
            0,
            "degraded",
            "stale",
            serde_json::json!(["SOURCE_FUTURE"]),
        ),
    ] {
        if case == "mixed_live" {
            let second = seed_second_demand_fixture(&h, &stream, &first)
                .await
                .expect("second authenticated demand");
            seed_live_quote_fixture(&h, &stream, &second)
                .await
                .expect("second complete quote");
            demands.push(second);
        }
        if closed {
            state = configure_stream_window(&h, &mut stream, true).await;
        }
        let seeded = set_stream_evidence(
            &h,
            &stream,
            &demands,
            EvidenceCase {
                received_age_ms: received,
                provider_age_ms: provider,
                halted,
                ..EvidenceCase::default()
            },
        )
        .await
        .expect("constraint-valid diagnostic clocks");
        if let Some(code) = code {
            let updated=sqlx::query("UPDATE public.owner_market_stream_producers SET status_code=$2,status_at=$3,connection_state=CASE WHEN $2='CONNECTION_LOST' THEN 'DISCONNECTED' ELSE 'CONNECTED' END WHERE credential_slot_id=$1")
                .bind(stream.slot_id).bind(code)
                .bind(if code=="SUBSCRIPTION_PENDING" {seeded+chrono::Duration::seconds(20)} else {seeded-chrono::Duration::seconds(1)})
                .execute(&h.owner_pool).await.expect("paired code/status time within real constraints");
            assert_eq!(updated.rows_affected(), 1);
        }
        if calendar_failure {
            let updated=sqlx::query("UPDATE public.trading_calendars SET content_sha256=repeat('e',64) WHERE exchange='KRX' AND session_date=$1")
                .bind(stream.session_date).execute(&h.owner_pool).await.expect("projection separated from immutable calendar evidence");
            assert_eq!(updated.rows_affected(), 1);
        }
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_market_stream_leases WHERE owner_user_id=$1",
        )
        .bind(stream.owner_id)
        .fetch_one(&h.owner_pool)
        .await
        .unwrap();
        if let Some(count) = lease_count {
            assert_eq!(before, count, "fixture/status reads create no new leases");
        }
        lease_count = Some(before);
        let mut input = api_server::repos::service_status::refresh(
            &state,
            &h.owner.actor(),
            &stream.session_hash,
            Utc::now(),
            None,
        )
        .await;
        let aggregate = &input["db_read_results"]["stream"];
        assert_eq!(
            aggregate["result"], "ok",
            "{case}: populated real source read"
        );
        assert_eq!(
            aggregate["owner_binding_valid"], true,
            "{case}: actual app-role binding"
        );
        assert_eq!(aggregate["demand_count"], demands.len());
        assert_eq!(
            aggregate["live_count"], live,
            "{case}: inclusive coverage converted once"
        );
        assert_eq!(
            aggregate["halted_count"], halted_count,
            "{case}: genuine halted coverage"
        );
        let parse = |name: &str| {
            DateTime::parse_from_rfc3339(aggregate[name].as_str().unwrap())
                .unwrap()
                .with_timezone(&Utc)
        };
        let future_receipt = received[..demands.len()].iter().min().copied().unwrap() < 0;
        let future_provider = provider[..demands.len()].iter().all(|age| *age < 0);
        let expected_last = seeded
            - chrono::Duration::milliseconds(i64::from(
                *received[..demands.len()].iter().min().unwrap(),
            ));
        let expected_provider = seeded
            - chrono::Duration::milliseconds(i64::from(
                *provider[..demands.len()].iter().max().unwrap(),
            ));
        assert_eq!(
            parse("last_received_at"),
            expected_last,
            "{case}: actual SQL maximum clock"
        );
        assert_eq!(
            parse("oldest_provider_trade_at"),
            expected_provider,
            "{case}: actual SQL minimum clock"
        );
        if code == Some("SUBSCRIPTION_PENDING") {
            assert_eq!(
                parse("producer_status_at"),
                seeded + chrono::Duration::seconds(20)
            );
        }
        let bound = parse("valid_until");
        if live + halted_count > 0 {
            assert!(
                bound <= parse("producer_heartbeat_at") + chrono::Duration::seconds(10),
                "{case}: partial positive assurance remains bounded"
            );
        }
        let now = Utc::now();
        assert!(
            now >= seeded && now - seeded < chrono::Duration::seconds(3),
            "{case}: real source refresh stays within explicit future fixture margin"
        );
        if future_receipt {
            assert!(expected_last > now);
        }
        if future_provider {
            assert!(expected_provider > now);
            assert_eq!((live, halted_count), (0, 0));
        }
        let original = aggregate.clone();
        // Independent observation/calendar/window prerequisites may shorten
        // the SQL deadline. The diagnostic fact cannot widen any of them.
        let expected_until = [
            &input["window_read"]["observed_at"],
            &input["db_read_results"]["calendar"]["observed_at"],
        ]
        .into_iter()
        .map(|value| {
            DateTime::parse_from_rfc3339(value.as_str().unwrap())
                .unwrap()
                .with_timezone(&Utc)
                + chrono::Duration::seconds(30)
        })
        .fold(bound, |until, observation_until| {
            until.min(observation_until)
        });
        attach_synthetic_host(&mut input, now);
        let response = api_server::service_status::project(&input, now)
            .expect("actual populated diagnostic projection");
        assert_eq!(response["services"].as_array().unwrap().len(), 22);
        let row = service(&response, "market-stream");
        assert_eq!(row["status"], status, "{case}");
        assert_eq!(row["work_status"], work, "{case}");
        assert_eq!(
            row["reason_codes"], reasons,
            "{case}: independent facts survive severity selection"
        );
        assert_eq!(row["metrics"]["active_demand"], demands.len());
        assert_eq!(row["metrics"]["live_identities"], live);
        assert_eq!(row["metrics"]["halted_identities"], halted_count);
        assert_eq!(
            row["valid_until"],
            expected_until.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            "{case}: retain SQL and independent source/window observation bounds"
        );
        for (metric, source) in [
            ("last_received_at", "last_received_at"),
            ("oldest_provider_event_at", "oldest_provider_trade_at"),
            ("last_state_change_at", "producer_status_at"),
        ] {
            assert_eq!(
                row["metrics"][metric], original[source],
                "{case}: exact raw {source}"
            );
        }
        if case == "future_provider_producer_failure" {
            // Re-evaluate the actual validated DB aggregate with an independent
            // unavailable window observation. This is a projection fault,
            // not a claim that the immutable configured window failed on disk.
            let mut missing_window = input.clone();
            missing_window["window_read"]["result"] = serde_json::json!("missing");
            let response = api_server::service_status::project(&missing_window, now).unwrap();
            let row = service(&response, "market-stream");
            assert_eq!(row["status"], "failed");
            assert_eq!(row["work_status"], "failed");
            assert_eq!(
                row["reason_codes"],
                serde_json::json!([
                    "CONNECTION_LOST",
                    "SESSION_WINDOW_UNAVAILABLE",
                    "SOURCE_FUTURE"
                ])
            );
            assert_eq!(
                row["metrics"]["oldest_provider_event_at"],
                original["oldest_provider_trade_at"]
            );
            assert_eq!(row["metrics"]["live_identities"], 0);
            assert_eq!(row["metrics"]["halted_identities"], 0);
            assert!(
                DateTime::parse_from_rfc3339(row["valid_until"].as_str().unwrap())
                    .unwrap()
                    .with_timezone(&Utc)
                    <= bound
            );
        }
        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_market_stream_leases WHERE owner_user_id=$1",
        )
        .bind(stream.owner_id)
        .fetch_one(&h.owner_pool)
        .await
        .unwrap();
        assert_eq!(
            after, before,
            "{case}: read-only observation creates no demand"
        );
        if calendar_failure {
            let updated=sqlx::query("UPDATE public.trading_calendars SET content_sha256=$2 WHERE exchange='KRX' AND session_date=$1")
                .bind(stream.session_date).bind(&stream.calendar_content_sha256).execute(&h.owner_pool).await.expect("restore only mutable synthetic projection");
            assert_eq!(updated.rows_affected(), 1);
        }
    }
    app.close().await;
    h.teardown().await;
}

// These new expectations are authored independently from the accepted reducer
// rules and immutable base/case outputs. Product projection never creates them.
const FUTURE_METRIC_CASES: &str = r#"[

  {
    "id": "single_future_receipt",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/demand_count": 1,
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/oldest_received_at": "2032-02-12T01:00:01Z",
      "/db_read_results/stream/oldest_provider_trade_at": "2032-02-12T00:59:59Z",
      "/db_read_results/stream/last_received_at": "2032-02-12T01:00:01Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "stale",
      "/services/12/reason_codes": [
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/metrics/active_demand": 1,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "mixed_live_future_maximum_receipt",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/live_count": 1,
      "/db_read_results/stream/last_received_at": "2032-02-12T01:00:01Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "stale",
      "/services/12/reason_codes": [
        "QUOTE_STALE",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 1,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T00:59:57Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "mixed_halted_future_maximum_receipt",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/halted_count": 1,
      "/db_read_results/stream/last_received_at": "2032-02-12T01:00:01Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "stale",
      "/services/12/reason_codes": [
        "QUOTE_STALE",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:09Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 1,
      "/services/12/metrics/last_received_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T00:59:57Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "future_provider_producer_failure",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/connection_state": "DISCONNECTED",
      "/db_read_results/stream/status_code": "CONNECTION_LOST",
      "/db_read_results/stream/producer_status_at": "2032-02-12T00:59:59Z",
      "/db_read_results/stream/oldest_provider_trade_at": "2032-02-12T01:00:01Z",
      "/db_read_results/stream/valid_until": "2032-02-12T01:00:30Z"
    },
    "expected_replacements": {
      "/overall_status": "failed",
      "/services/12/status": "failed",
      "/services/12/work_status": "failed",
      "/services/12/reason_codes": [
        "CONNECTION_LOST",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/last_state_change_at": "2032-02-12T00:59:59Z"
    }
  },
  {
    "id": "future_provider_calendar_failure",
    "seed_id": "calendar_lineage_invalid_no_demand",
    "input_replacements": {
      "/db_read_results/stream/demand_count": 2,
      "/db_read_results/stream/oldest_provider_trade_at": "2032-02-12T01:00:01Z"
    },
    "expected_replacements": {
      "/services/12/status": "failed",
      "/services/12/work_status": "failed",
      "/services/12/reason_codes": [
        "CALENDAR_LINEAGE_INVALID",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "future_provider_closed",
    "seed_id": "market_closed",
    "input_replacements": {
      "/db_read_results/stream/oldest_provider_trade_at": "2032-02-12T01:00:01Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "stale",
      "/services/12/reason_codes": [
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T01:00:01Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "future_producer_status",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/status_code": "SUBSCRIPTION_PENDING",
      "/db_read_results/stream/producer_status_at": "2032-02-12T01:00:01Z",
      "/db_read_results/stream/valid_until": "2032-02-12T01:00:30Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "degraded",
      "/services/12/reason_codes": [
        "SOURCE_FUTURE",
        "SUBSCRIPTION_PENDING"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/metrics/active_demand": 2,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T00:59:57Z",
      "/services/12/metrics/last_state_change_at": "2032-02-12T01:00:01Z"
    }
  },
  {
    "id": "no_demand_future_producer_status",
    "seed_id": "base",
    "input_replacements": {
      "/db_read_results/stream/demand_count": 0,
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/status_code": "NO_ACTIVE_DEMAND",
      "/db_read_results/stream/producer_status_at": "2032-02-12T01:00:01Z",
      "/db_read_results/stream/oldest_received_at": null,
      "/db_read_results/stream/oldest_provider_trade_at": null,
      "/db_read_results/stream/last_received_at": null,
      "/db_read_results/stream/valid_until": "2032-02-12T01:00:30Z"
    },
    "expected_replacements": {
      "/overall_status": "degraded",
      "/services/12/status": "degraded",
      "/services/12/work_status": "degraded",
      "/services/12/reason_codes": [
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:30Z",
      "/services/12/metrics/active_demand": 0,
      "/services/12/metrics/live_identities": 0,
      "/services/12/metrics/halted_identities": 0,
      "/services/12/metrics/last_received_at": null,
      "/services/12/metrics/oldest_provider_event_at": null,
      "/services/12/metrics/last_state_change_at": "2032-02-12T01:00:01Z"
    }
  },
  {
    "id": "expired_mixed_future_receipt",
    "seed_id": "base",
    "input_replacements": {
      "/clock": "2032-02-12T01:00:03.000001Z",
      "/db_read_results/stream/live_count": 1,
      "/db_read_results/stream/last_received_at": "2032-02-12T01:00:05Z",
      "/db_read_results/stream/producer_heartbeat_at": "2032-02-12T00:59:53Z",
      "/db_read_results/stream/valid_until": "2032-02-12T01:00:03Z"
    },
    "expected_replacements": {
      "/generated_at": "2032-02-12T01:00:03.000001Z",
      "/overall_status": "failed",
      "/services/12/status": "failed",
      "/services/12/work_status": "failed",
      "/services/12/reason_codes": [
        "PRODUCER_STALE",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:03Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:03Z",
      "/services/12/metrics/active_demand": null,
      "/services/12/metrics/live_identities": null,
      "/services/12/metrics/halted_identities": null,
      "/services/12/metrics/last_received_at": "2032-02-12T01:00:05Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T00:59:57Z",
      "/services/12/metrics/last_state_change_at": null
    }
  },
  {
    "id": "expired_future_provider",
    "seed_id": "base",
    "input_replacements": {
      "/clock": "2032-02-12T01:00:03.000001Z",
      "/db_read_results/stream/live_count": 0,
      "/db_read_results/stream/oldest_provider_trade_at": "2032-02-12T01:00:05Z",
      "/db_read_results/stream/producer_heartbeat_at": "2032-02-12T00:59:53Z",
      "/db_read_results/stream/valid_until": "2032-02-12T01:00:03Z"
    },
    "expected_replacements": {
      "/generated_at": "2032-02-12T01:00:03.000001Z",
      "/overall_status": "failed",
      "/services/12/status": "failed",
      "/services/12/work_status": "failed",
      "/services/12/reason_codes": [
        "PRODUCER_STALE",
        "SOURCE_FUTURE"
      ],
      "/services/12/valid_until": "2032-02-12T01:00:03Z",
      "/services/12/sources/database/valid_until": "2032-02-12T01:00:03Z",
      "/services/12/metrics/active_demand": null,
      "/services/12/metrics/live_identities": null,
      "/services/12/metrics/halted_identities": null,
      "/services/12/metrics/last_received_at": "2032-02-12T00:59:59Z",
      "/services/12/metrics/oldest_provider_event_at": "2032-02-12T01:00:05Z",
      "/services/12/metrics/last_state_change_at": null
    }
  }
]"#;

fn future_metric_projection_cases() -> Vec<service_status_support::ProjectionCase> {
    let seeds = projection_cases();
    let specifications: Vec<Value> =
        serde_json::from_str(FUTURE_METRIC_CASES).expect("independent diagnostic cases");
    specifications
        .into_iter()
        .map(|specification| {
            let seed = seeds
                .iter()
                .find(|case| case.id == specification["seed_id"].as_str().unwrap())
                .expect("accepted independent base/case seed");
            let mut input = seed.input.clone();
            let mut expected = seed.expected.clone();
            for (document, name) in [
                (&mut input, "input_replacements"),
                (&mut expected, "expected_replacements"),
            ] {
                for (pointer, value) in specification[name].as_object().unwrap() {
                    *document
                        .pointer_mut(pointer)
                        .expect("explicit replacement path exists") = value.clone();
                }
            }
            service_status_support::ProjectionCase {
                id: specification["id"].as_str().unwrap().to_owned(),
                input,
                expected,
            }
        })
        .collect()
}

#[test]
fn future_stream_diagnostics_preserve_independent_facts_counts_clocks_and_bounds() {
    let mut differences = Vec::new();
    let mut artifacts = Vec::new();
    for case in future_metric_projection_cases() {
        let now = DateTime::parse_from_rfc3339(case.input["clock"].as_str().unwrap())
            .expect("fixed diagnostic clock")
            .with_timezone(&Utc);
        let actual = api_server::service_status::project(&case.input, now)
            .expect("diagnostic projection must remain a full response");
        assert_eq!(actual["services"].as_array().unwrap().len(), 22);
        let row = service(&actual, "market-stream");
        assert_eq!(
            row["reason_codes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|reason| reason.as_str() == Some("SOURCE_FUTURE"))
                .count(),
            1,
            "{}: independent diagnosis is deduplicated",
            case.id
        );
        if let Some(difference) = first_difference("$", &actual, &case.expected) {
            differences.push(format!("{}: {difference}", case.id));
        }
        artifacts.push(serde_json::json!({"id":case.id,"actual_response":actual}));
    }
    if let Some(dir) = std::env::var_os("QA_SERVICE_STATUS_ARTIFACT_DIR") {
        std::fs::write(
            PathBuf::from(dir).join("future-metric-actual-responses.json"),
            serde_json::to_vec_pretty(&artifacts).unwrap(),
        )
        .expect("actual cross-layer responses retained");
    }
    assert!(
        differences.is_empty(),
        "independent future diagnostics differ:\n{}",
        differences.join("\n")
    );
}

#[test]
fn future_metric_diagnosis_requires_exposed_valid_source_and_binding() {
    let base: Value = serde_json::from_str(BASE_INPUT).unwrap();
    let now = DateTime::parse_from_rfc3339(base["clock"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    for (group_field, value) in [
        ("result", serde_json::json!("timeout")),
        ("observed_at", serde_json::json!("2032-02-12T01:00:01Z")),
        ("owner_binding_valid", serde_json::json!(false)),
    ] {
        let mut input = base.clone();
        input["db_read_results"]["stream"][group_field] = value;
        for name in [
            "last_received_at",
            "oldest_provider_trade_at",
            "producer_status_at",
        ] {
            input["db_read_results"]["stream"][name] = serde_json::json!("2032-02-12T01:00:05Z");
        }
        let actual = api_server::service_status::project(&input, now).unwrap();
        let metrics = &service(&actual, "market-stream")["metrics"];
        for name in [
            "last_received_at",
            "oldest_provider_event_at",
            "last_state_change_at",
        ] {
            assert!(
                metrics[name].is_null(),
                "{group_field}: unvalidated diagnostics are not exposed"
            );
        }
        // A future observation itself may independently supply SOURCE_FUTURE;
        // the new metric merge must never disclose clocks from that source.
    }
    for (result, reason) in [
        ("missing", "SESSION_WINDOW_UNAVAILABLE"),
        ("invalid", "SESSION_WINDOW_INVALID"),
    ] {
        let mut input = base.clone();
        input["db_read_results"]["stream"]["live_count"] = serde_json::json!(0);
        input["db_read_results"]["stream"]["oldest_provider_trade_at"] =
            serde_json::json!("2032-02-12T01:00:05Z");
        input["window_read"]["result"] = serde_json::json!(result);
        let actual = api_server::service_status::project(&input, now).unwrap();
        let row = service(&actual, "market-stream");
        assert_eq!(row["status"], "failed");
        assert_eq!(row["work_status"], "failed");
        assert_eq!(
            row["reason_codes"],
            serde_json::json!([reason, "SOURCE_FUTURE"])
        );
        assert_eq!(
            row["metrics"]["oldest_provider_event_at"],
            "2032-02-12T01:00:05Z"
        );
        assert_eq!(row["metrics"]["live_identities"], 0);
        assert_eq!(row["metrics"]["halted_identities"], 0);
        assert_eq!(row["valid_until"], "2032-02-12T01:00:09Z");
    }
}

#[test]
fn cached_future_diagnostics_keep_inclusive_assurance_then_null_expired_coverage() {
    use api_server::service_status::ServiceStatusRuntime;
    let root = temp_root();
    let (snapshot, now, release) = snapshot_fixture();
    write_snapshot(&root.0, &snapshot);
    let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
    let runtime = ServiceStatusRuntime::default();
    let at = Instant::now();
    runtime
        .admit("owner", "session", at)
        .expect("actor admitted");
    let host = runtime.observe_host_for_test(&root.0, owner, now, &release);
    let boundary = now + chrono::Duration::seconds(3);
    for (future_field, live, halted, reason) in [
        ("last_received_at", 1, 0, "QUOTE_STALE"),
        ("last_received_at", 0, 1, "QUOTE_STALE"),
        ("oldest_provider_trade_at", 0, 0, "SOURCE_FUTURE"),
    ] {
        let mut input: Value = serde_json::from_str(BASE_INPUT).unwrap();
        input["db_read_results"]["stream"]["live_count"] = serde_json::json!(live);
        input["db_read_results"]["stream"]["halted_count"] = serde_json::json!(halted);
        input["db_read_results"]["stream"][future_field] =
            serde_json::json!("2032-02-12T01:00:05Z");
        input["db_read_results"]["stream"]["producer_heartbeat_at"] =
            serde_json::json!("2032-02-12T00:59:53Z");
        input["db_read_results"]["stream"]["valid_until"] =
            serde_json::json!("2032-02-12T01:00:03Z");
        let original = input["db_read_results"]["stream"].clone();
        runtime.cache_owner_evidence("owner", "session", &host, input, at);
        for clock in [
            boundary - chrono::Duration::microseconds(1),
            boundary,
            boundary + chrono::Duration::microseconds(1),
        ] {
            let cached = runtime
                .cached_owner_evidence(
                    "owner",
                    "session",
                    &host,
                    at + (clock - now).to_std().unwrap(),
                    clock,
                )
                .expect("same actor/session evidence remains within cache age");
            assert_eq!(
                cached["db_read_results"]["stream"], original,
                "cache must not rebase diagnostics or deadlines"
            );
            let response = api_server::service_status::project(&cached, clock).unwrap();
            let row = service(&response, "market-stream");
            assert_eq!(row["valid_until"], "2032-02-12T01:00:03Z");
            let expired = clock > boundary;
            assert_eq!(row["status"], if expired { "failed" } else { "degraded" });
            assert_eq!(row["work_status"], if expired { "failed" } else { "stale" });
            let expected_reasons = if expired {
                serde_json::json!(["PRODUCER_STALE", "SOURCE_FUTURE"])
            } else if reason == "QUOTE_STALE" {
                serde_json::json!(["QUOTE_STALE", "SOURCE_FUTURE"])
            } else {
                serde_json::json!(["SOURCE_FUTURE"])
            };
            assert_eq!(row["reason_codes"], expected_reasons);
            for (metric, count) in [
                ("active_demand", 2),
                ("live_identities", live),
                ("halted_identities", halted),
            ] {
                assert_eq!(
                    row["metrics"][metric],
                    if expired {
                        Value::Null
                    } else {
                        serde_json::json!(count)
                    }
                );
            }
            for (metric, source) in [
                ("last_received_at", "last_received_at"),
                ("oldest_provider_event_at", "oldest_provider_trade_at"),
                ("last_state_change_at", "producer_status_at"),
            ] {
                assert_eq!(
                    row["metrics"][metric], original[source],
                    "original {source} is retained"
                );
            }
        }
    }
}

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_root() -> TempRoot {
    let path = std::env::temp_dir().join(format!(
        "lagrange-service-status-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::create_dir(&path).expect("temporary root created");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750))
        .expect("secure temporary root mode");
    TempRoot(path)
}

fn snapshot_fixture() -> (Value, DateTime<Utc>, String) {
    let input: Value = serde_json::from_str(BASE_INPUT).expect("base input parses");
    let snapshot = input["host_read"]["snapshot"].clone();
    let now = DateTime::parse_from_rfc3339(input["clock"].as_str().expect("fixture clock"))
        .expect("fixed clock parses")
        .with_timezone(&Utc);
    let release = snapshot["policy"]["release_commit"]
        .as_str()
        .expect("fixture release")
        .to_owned();
    (snapshot, now, release)
}

fn write_snapshot(root: &Path, snapshot: &Value) {
    let file = root.join("snapshot.json");
    std::fs::write(
        &file,
        serde_json::to_vec(snapshot).expect("snapshot serializes"),
    )
    .expect("snapshot written");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640))
        .expect("secure snapshot mode");
}

fn assert_same_shape(actual: &Value, expected_shape: &Value) {
    let actual_keys = actual
        .as_object()
        .expect("actual response object")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_keys = expected_shape
        .as_object()
        .expect("expected shape object")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_keys, expected_keys);
    if let (Some(actual_services), Some(expected_services)) = (
        actual["services"].as_array(),
        expected_shape["services"].as_array(),
    ) {
        assert_eq!(actual_services.len(), expected_services.len());
        for (actual_service, expected_service) in actual_services.iter().zip(expected_services) {
            assert_same_shape(actual_service, expected_service);
            assert_same_shape(&actual_service["sources"], &expected_service["sources"]);
            assert_same_shape(&actual_service["metrics"], &expected_service["metrics"]);
        }
    }
}

fn first_difference(path: &str, actual: &Value, expected: &Value) -> Option<String> {
    if actual == expected {
        return None;
    }
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for key in actual.keys().chain(expected.keys()) {
                let child = format!("{path}/{key}");
                match (actual.get(key), expected.get(key)) {
                    (Some(actual), Some(expected)) => {
                        if let Some(difference) = first_difference(&child, actual, expected) {
                            return Some(difference);
                        }
                    }
                    (Some(_), None) => return Some(format!("{child} is unexpected")),
                    (None, Some(_)) => return Some(format!("{child} is missing")),
                    (None, None) => unreachable!("key came from one object"),
                }
            }
            Some(path.to_owned())
        }
        (Value::Array(actual), Value::Array(expected)) => {
            if actual.len() != expected.len() {
                return Some(format!(
                    "{path} length: actual {} != expected {}",
                    actual.len(),
                    expected.len()
                ));
            }
            actual
                .iter()
                .zip(expected)
                .enumerate()
                .find_map(|(index, (actual, expected))| {
                    first_difference(&format!("{path}/{index}"), actual, expected)
                })
        }
        _ => Some(format!("{path}: actual {actual} != expected {expected}")),
    }
}

fn collect_difference_paths(
    path: &str,
    actual: Option<&Value>,
    expected: Option<&Value>,
    paths: &mut Vec<Value>,
) {
    if actual == expected {
        return;
    }
    match (actual, expected) {
        (Some(Value::Object(a)), Some(Value::Object(e))) => {
            let keys = a.keys().chain(e.keys()).collect::<BTreeSet<_>>();
            for key in keys { collect_difference_paths(&format!("{path}/{key}"), a.get(key), e.get(key), paths); }
        }
        (Some(Value::Array(a)), Some(Value::Array(e))) => {
            for i in 0..a.len().max(e.len()) { collect_difference_paths(&format!("{path}/{i}"), a.get(i), e.get(i), paths); }
        }
        _ => paths.push(serde_json::json!({"path":path,"actual":actual,"expected":expected,"actual_missing":actual.is_none(),"expected_missing":expected.is_none()})),
    }
}

#[test]
fn all_124_projection_patches_match_the_independent_fixed_clock_oracles() {
    let cases = projection_cases();
    assert_eq!(cases.len(), 125, "one base plus exactly 124 patched cases");
    let mut differences = Vec::new();
    let mut all_paths = Vec::new();
    let mut evaluated = 0;
    for case in cases {
        evaluated += 1;
        let now = DateTime::parse_from_rfc3339(case.input["clock"].as_str().expect("case clock"))
            .expect("fixed case clock parses")
            .with_timezone(&Utc);
        let host_error = case.input.get("host_error").and_then(Value::as_str);
        let actual =
            match api_server::service_status::project_with_host_error(&case.input, now, host_error)
            {
                Ok(actual) => actual,
                Err(error) => {
                    differences.push(format!("{}: projection failed: {error}", case.id));
                    all_paths.push(serde_json::json!({"id":case.id,"projection_error":error}));
                    continue;
                }
            };
        let difference = first_difference(
            "$/services",
            &actual["services"],
            &case.expected["services"],
        )
        .or_else(|| first_difference("$", &actual, &case.expected));
        if let Some(difference) = difference {
            let mut paths = Vec::new();
            collect_difference_paths("$", Some(&actual), Some(&case.expected), &mut paths);
            all_paths.push(serde_json::json!({"id":case.id,"differences":paths}));
            differences.push(format!("{}: {difference}", case.id));
        }
    }
    if let Some(dir) = std::env::var_os("QA_SERVICE_STATUS_ARTIFACT_DIR") {
        std::fs::write(
            std::path::PathBuf::from(dir).join("projection-differences.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"evaluated":evaluated,"cases":all_paths}),
            )
            .unwrap(),
        )
        .expect("all independent mismatch paths retained");
    }
    assert_eq!(evaluated, 125, "every independent input must be evaluated");
    assert!(
        differences.is_empty(),
        "{} of {evaluated} independent projections differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn independent_calendar_and_window_failures_keep_both_facts_and_real_bounds() {
    let mut input: Value = serde_json::from_str(BASE_INPUT).unwrap();
    let now = chrono::DateTime::parse_from_rfc3339(input["clock"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    input["db_read_results"]["calendar"]["result"] = serde_json::json!("timeout");
    input["window_read"]["result"] = serde_json::json!("missing");
    input["db_read_results"]["stream"]["owner_binding_valid"] = serde_json::json!(false);
    input["db_read_results"]["stream"]["valid_until"] = Value::Null;
    let response = api_server::service_status::project(&input, now).unwrap();
    for id in ["market-stream", "intraday-calendar"] {
        let row = service(&response, id);
        assert_eq!(row["status"], "failed");
        assert_eq!(row["work_status"], "failed");
        assert!(
            row["reason_codes"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("DB_TIMEOUT"))
        );
        assert!(
            row["reason_codes"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("SESSION_WINDOW_UNAVAILABLE"))
        );
    }
    assert!(service(&response, "market-stream")["metrics"]["active_demand"].is_null());
    assert!(service(&response, "market-stream")["sources"]["database"]["valid_until"].is_null());
    assert_eq!(
        service(&response, "market-stream")["valid_until"],
        "2032-02-12T01:00:30Z"
    );
}

#[test]
fn strict_wire_reader_rejects_duplicate_keys_invalid_utf8_and_non_json_numbers() {
    assert!(api_server::service_status::strict_json(br#"{"a":1,"a":2}"#).is_err());
    assert!(api_server::service_status::strict_json(&[b'{', 0xff, b'}']).is_err());
    assert!(api_server::service_status::strict_json(br#"{"number":NaN}"#).is_err());
    assert!(api_server::service_status::strict_json(b"{\"partial\":").is_err());
    assert_eq!(
        wire_cases()["cases"].as_array().expect("wire cases").len(),
        60
    );
}

#[test]
fn safe_reader_reopens_replaced_snapshot_and_checks_owner_mode_and_release() {
    let root = temp_root();
    let (mut snapshot, now, release) = snapshot_fixture();
    write_snapshot(&root.0, &snapshot);
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let first =
        api_server::service_status::read_host_snapshot_for_test(&root.0, now, &release, (uid, gid))
            .expect("secure snapshot accepted");
    assert_eq!(
        first["policy"]["compose_project"],
        snapshot["policy"]["compose_project"]
    );
    let old_inode = std::fs::metadata(root.0.join("snapshot.json"))
        .expect("first snapshot metadata")
        .ino();

    snapshot["policy"]["compose_project"] = Value::String("replacement-project".to_owned());
    let replacement = root.0.join("snapshot.next");
    std::fs::write(
        &replacement,
        serde_json::to_vec(&snapshot).expect("replacement serializes"),
    )
    .expect("replacement written");
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o640))
        .expect("replacement secure mode");
    std::fs::rename(&replacement, root.0.join("snapshot.json")).expect("atomic replacement");
    let second =
        api_server::service_status::read_host_snapshot_for_test(&root.0, now, &release, (uid, gid))
            .expect("reader reopens replacement by name");
    let new_inode = std::fs::metadata(root.0.join("snapshot.json"))
        .expect("replacement metadata")
        .ino();
    assert_ne!(old_inode, new_inode);
    assert_eq!(second["policy"]["compose_project"], "replacement-project");

    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &root.0,
            now,
            &"b".repeat(40),
            (uid, gid),
        )
        .expect_err("wrong release rejected"),
        api_server::service_status::HostReadError::PolicyMismatch
    );
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &root.0,
            now,
            &release,
            (uid.wrapping_add(1), gid),
        )
        .expect_err("wrong directory owner rejected"),
        api_server::service_status::HostReadError::Invalid
    );
}

#[test]
fn safe_reader_rejects_bad_snapshot_inventory_symlink_hardlink_permissions_and_size() {
    let now_root = temp_root();
    let (snapshot, now, release) = snapshot_fixture();
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let mut bad = snapshot.clone();
    bad["unexpected"] = Value::Bool(true);
    write_snapshot(&now_root.0, &bad);
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("unknown field rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    let mut bad = snapshot.clone();
    bad["observations"][0]["id"] = bad["observations"][1]["id"].clone();
    write_snapshot(&now_root.0, &bad);
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("duplicate id rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    let mut bad = snapshot.clone();
    bad["schema_version"] = Value::String("service-status.host.v2".to_owned());
    write_snapshot(&now_root.0, &bad);
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("unsupported host schema rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    let mut bad = snapshot.clone();
    bad["observations"]
        .as_array_mut()
        .expect("host observations")
        .pop();
    write_snapshot(&now_root.0, &bad);
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("missing inventory ID rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    let mut future = snapshot.clone();
    future["generated_at"] = Value::String("2032-02-12T01:00:01Z".to_owned());
    future["collection_started_at"] = Value::String("2032-02-11T23:59:59Z".to_owned());
    write_snapshot(&now_root.0, &future);
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("future snapshot rejected"),
        api_server::service_status::HostReadError::Future
    );

    write_snapshot(&now_root.0, &snapshot);
    std::fs::set_permissions(
        now_root.0.join("snapshot.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("unsafe mode installed");
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("writable-by-api mode rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    write_snapshot(&now_root.0, &snapshot);
    std::fs::hard_link(
        now_root.0.join("snapshot.json"),
        now_root.0.join("snapshot-copy.json"),
    )
    .expect("hard link created");
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("hard link rejected"),
        api_server::service_status::HostReadError::Invalid
    );
    std::fs::remove_file(now_root.0.join("snapshot-copy.json")).expect("hard link removed");

    let target = now_root.0.join("snapshot-target.json");
    std::fs::rename(now_root.0.join("snapshot.json"), &target).expect("snapshot moved");
    std::os::unix::fs::symlink(&target, now_root.0.join("snapshot.json"))
        .expect("snapshot symlink created");
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("symlink rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    let oversize = temp_root();
    std::fs::write(oversize.0.join("snapshot.json"), vec![b'x'; 256 * 1024 + 1])
        .expect("oversize fixture written");
    std::fs::set_permissions(
        oversize.0.join("snapshot.json"),
        std::fs::Permissions::from_mode(0o640),
    )
    .expect("oversize mode set");
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &oversize.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("oversize file rejected"),
        api_server::service_status::HostReadError::Invalid
    );

    std::fs::set_permissions(&now_root.0, std::fs::Permissions::from_mode(0o777))
        .expect("unsafe parent mode installed");
    assert_eq!(
        api_server::service_status::read_host_snapshot_for_test(
            &now_root.0,
            now,
            &release,
            (uid, gid),
        )
        .expect_err("unsafe parent mode rejected"),
        api_server::service_status::HostReadError::Invalid
    );
}

#[test]
fn reboot_history_preserves_verified_success_but_rejects_preboot_current_observations() {
    use api_server::service_status::{HostReadError, read_host_snapshot_for_test};
    let root = temp_root();
    let (mut snapshot, now, release) = snapshot_fixture();
    let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
    let booted = DateTime::parse_from_rfc3339("2032-02-12T00:30:00Z").unwrap();
    snapshot["booted_at"] = serde_json::json!("2032-02-12T00:30:00Z");
    // Every non-null base execution timestamp predates this synthetic reboot.
    // Construct the accepted collector shape: clear current-interval correlation
    // and retain its independently verified success, without loading /tmp data.
    for observation in snapshot["observations"].as_array_mut().unwrap() {
        if let Some(process) = observation
            .get_mut("process")
            .filter(|value| value.is_object())
        {
            let has_execution = ["started_at", "finished_at"]
                .iter()
                .any(|field| process[*field].is_string());
            for field in ["started_at", "finished_at"] {
                if let Some(at) = process[field].as_str() {
                    assert!(DateTime::parse_from_rfc3339(at).unwrap() < booted);
                }
                process[field] = Value::Null;
            }
            if has_execution {
                process["exit_code"] = Value::Null;
            }
        }
        if let Some(timer) = observation
            .get_mut("timer")
            .filter(|value| value.is_object())
        {
            for field in ["last_trigger_at", "last_started_at", "last_finished_at"] {
                if let Some(at) = timer[field].as_str() {
                    assert!(DateTime::parse_from_rfc3339(at).unwrap() < booted);
                }
                timer[field] = Value::Null;
            }
            timer["exit_status"] = Value::Null;
            timer["last_result"] = serde_json::json!("unknown");
        }
    }
    let kis = snapshot["observations"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["id"] == "kis-eod")
        .unwrap();
    assert_eq!(
        snapshot["observations"][kis]["timer"]["last_success_at"],
        "2032-02-11T07:35:00Z"
    );
    write_snapshot(&root.0, &snapshot);
    let accepted = read_host_snapshot_for_test(&root.0, now, &release, owner)
        .expect("verified pre-boot success is history, not a current process observation");
    assert_eq!(
        accepted, snapshot,
        "the reader never manufactures or changes source timestamps"
    );
    let mut input: Value = serde_json::from_str(BASE_INPUT).unwrap();
    input["host_read"]["snapshot"] = accepted;
    let response = api_server::service_status::project(&input, now).unwrap();
    assert_eq!(response["policy_state"], "verified");
    for id in [
        "kis-eod",
        "production-backup",
        "production-backup-verify",
        "tls-renewal",
    ] {
        let prior = snapshot["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == id)
            .unwrap()["timer"]["last_success_at"]
            .clone();
        let row = service(&response, id);
        assert_eq!(
            row["sources"]["host"]["result"], "ok",
            "fresh collection after reboot: {id}"
        );
        assert_eq!(
            row["metrics"]["last_success_at"], prior,
            "independent recorded success: {id}"
        );
        assert!(
            row["metrics"]["last_attempt_at"].is_null(),
            "pre-boot attempts were cleared: {id}"
        );
    }
    assert_eq!(
        service(&response, "kis-eod")["metrics"]["data_as_of"],
        input["db_read_results"]["etf"]["publication_session"],
        "Curated success still needs the independent exact DB binding"
    );
    for (path, value, error) in [
        (
            format!("/observations/{kis}/timer/last_success_at"),
            "2032-02-12T01:00:01Z",
            HostReadError::Future,
        ),
        (
            format!("/observations/{kis}/timer/last_success_at"),
            "2032-02-11T07:35:00.0000001Z",
            HostReadError::Invalid,
        ),
        (
            format!("/observations/{kis}/timer/last_success_at"),
            "2032-02-11T07:35:00+00:00",
            HostReadError::Invalid,
        ),
        (
            "/observations/0/process/started_at".to_owned(),
            "2032-02-12T00:29:59.999999Z",
            HostReadError::Invalid,
        ),
        (
            "/observations/0/process/finished_at".to_owned(),
            "2032-02-12T00:29:59.999999Z",
            HostReadError::Invalid,
        ),
        (
            format!("/observations/{kis}/timer/last_trigger_at"),
            "2032-02-12T00:29:59.999999Z",
            HostReadError::Future,
        ),
        (
            format!("/observations/{kis}/timer/last_started_at"),
            "2032-02-12T00:29:59.999999Z",
            HostReadError::Invalid,
        ),
        (
            format!("/observations/{kis}/timer/last_finished_at"),
            "2032-02-12T00:29:59.999999Z",
            HostReadError::Invalid,
        ),
    ] {
        let mut bad = snapshot.clone();
        *bad.pointer_mut(&path).unwrap() = serde_json::json!(value);
        write_snapshot(&root.0, &bad);
        assert_eq!(
            read_host_snapshot_for_test(&root.0, now, &release, owner).unwrap_err(),
            error,
            "{path}"
        );
    }
    // A history clock later than its source observation is rejected even when
    // the API evaluates it later; generated_at remains the upper bound.
    let mut future = snapshot.clone();
    future["observations"][kis]["timer"]["last_success_at"] =
        serde_json::json!("2032-02-12T01:00:01Z");
    write_snapshot(&root.0, &future);
    assert_eq!(
        read_host_snapshot_for_test(&root.0, now + chrono::Duration::minutes(1), &release, owner)
            .unwrap_err(),
        HostReadError::Future
    );
    let mut completed = snapshot.clone();
    completed["observations"][kis]["process"]["started_at"] =
        serde_json::json!("2032-02-12T00:45:00Z");
    completed["observations"][kis]["process"]["finished_at"] =
        serde_json::json!("2032-02-12T00:46:00Z");
    completed["observations"][kis]["process"]["exit_code"] = serde_json::json!(1);
    completed["observations"][kis]["timer"]["last_started_at"] =
        serde_json::json!("2032-02-12T00:45:00Z");
    completed["observations"][kis]["timer"]["last_finished_at"] =
        serde_json::json!("2032-02-12T00:46:00Z");
    completed["observations"][kis]["timer"]["exit_status"] = serde_json::json!(1);
    completed["observations"][kis]["timer"]["last_result"] = serde_json::json!("failure");
    write_snapshot(&root.0, &completed);
    assert!(
        read_host_snapshot_for_test(&root.0, now, &release, owner).is_ok(),
        "older success may precede a present current completion"
    );
    completed["observations"][kis]["timer"]["last_success_at"] =
        serde_json::json!("2032-02-12T00:50:00Z");
    write_snapshot(&root.0, &completed);
    assert_eq!(
        read_host_snapshot_for_test(&root.0, now, &release, owner).unwrap_err(),
        HostReadError::Invalid,
        "success must not follow a present completion"
    );
}

#[test]
fn per_actor_admission_and_cache_enforce_session_policy_and_refresh_capacity() {
    use api_server::service_status::{AdmissionError, ServiceStatusRuntime};
    let runtime = ServiceStatusRuntime::default();
    let now = Instant::now();
    runtime
        .admit("actor-a", "session-a", now)
        .expect("first admission");
    assert_eq!(
        runtime.admit("actor-a", "session-a", now + Duration::from_secs(1)),
        Err(AdmissionError::RateLimited)
    );
    runtime.cache_input(
        "actor-a",
        "session-a",
        "release-a",
        serde_json::json!({"safe":true}),
        now + Duration::from_millis(2),
    );
    assert_eq!(
        runtime.cached_input(
            "actor-a",
            "session-a",
            "release-a",
            now + Duration::from_secs(1)
        ),
        Some(serde_json::json!({"safe":true}))
    );
    assert!(
        runtime
            .cached_input(
                "actor-a",
                "session-b",
                "release-a",
                now + Duration::from_secs(1)
            )
            .is_none()
    );
    assert!(
        runtime
            .cached_input(
                "actor-a",
                "session-a",
                "release-b",
                now + Duration::from_secs(1)
            )
            .is_none()
    );
    runtime
        .admit("actor-a", "session-b", now + Duration::from_secs(5))
        .expect("new session admission");
    assert!(
        runtime
            .cached_input(
                "actor-a",
                "session-b",
                "release-a",
                now + Duration::from_secs(5)
            )
            .is_none()
    );
    let first = runtime.try_refresh().expect("refresh permit one");
    let second = runtime.try_refresh().expect("refresh permit two");
    assert!(runtime.try_refresh().is_none(), "no third refresh queue");
    drop((first, second));
    let shared = runtime.try_shared_refresh().expect("single shared refresh");
    assert!(
        runtime.try_shared_refresh().is_none(),
        "shared refresh is single-flight"
    );
    drop(shared);

    let global = ServiceStatusRuntime::default();
    for index in 0..10 {
        global
            .admit(&format!("global-{index}"), "session", now)
            .expect("global burst admits ten actors");
    }
    assert_eq!(
        global.admit("global-overflow", "session", now),
        Err(AdmissionError::RateLimited),
        "global burst is capped at ten requests per second"
    );

    let slots = ServiceStatusRuntime::default();
    for index in 0..128 {
        slots
            .admit(
                &format!("slot-{index}"),
                "session",
                now + Duration::from_secs(index),
            )
            .expect("128 actor slots accepted");
    }
    assert_eq!(
        slots.admit("slot-overflow", "session", now + Duration::from_secs(128)),
        Err(AdmissionError::SlotCapacity),
        "actor cache table is capped at 128 slots"
    );
}

#[tokio::test]
async fn full_handler_budget_includes_blocked_session_authentication() {
    let h = required_harness().await;
    let mut blocker = h.owner_pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE web_sessions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let started = Instant::now();
    let response = tokio::time::timeout(Duration::from_millis(3500), h.get(PATH, Some(&h.owner)))
        .await
        .expect("actual full handler has a three-second budget");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.elapsed() >= Duration::from_millis(2900));
    assert!(started.elapsed() < Duration::from_millis(3500));
    let body = common::Harness::body_json(response).await;
    assert_eq!(body["error"]["code"], "SERVICE_STATUS_UNAVAILABLE");
    assert!(body.get("services").is_none());
    assert!(body["error"]["details"].is_null());
    blocker.rollback().await.unwrap();
    assert!(
        audit_rows_for_actor(&h, h.owner.user_id).await.is_empty(),
        "no authenticated/admitted request existed before the timeout"
    );
    h.teardown().await;
}

#[tokio::test]
async fn each_populated_source_times_out_independently_within_the_evidence_budget() {
    use service_status_reads_support::{
        seed_demand_fixture, seed_live_quote_fixture, seed_stream_fixture,
    };
    use service_status_support::{
        attach_synthetic_host, configure_stream_window, refresh_stream_clocks, seed_calendar_days,
        seed_etf_publication, seed_published_candidate_feed, seed_queue_jobs,
    };
    let h = required_harness().await;
    let mut stream = seed_stream_fixture(&h).await.expect("pinned stream source");
    let state = configure_stream_window(&h, &mut stream, false).await;
    let demand = seed_demand_fixture(&h, &stream)
        .await
        .expect("real READY admissions and demand");
    seed_live_quote_fixture(&h, &stream, &demand)
        .await
        .expect("positive live quote path");
    seed_calendar_days(&h).await;
    let (_, _, session) = seed_etf_publication(&h, 11).await;
    seed_published_candidate_feed(&h).await;
    sqlx::query(
        "UPDATE candidate_universe_registry SET enabled=false WHERE universe_key='kosdaq150'",
    )
    .execute(&h.owner_pool)
    .await
    .unwrap();
    sqlx::query("UPDATE candidate_scheduler_control SET active=true WHERE control_key='scheduler'")
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let job = seed_queue_jobs(&h, h.owner.user_id, "recommendation", 1, "QUEUED", 10).await[0];
    let mut fixture_tx = h.owner_pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(h.owner.user_id.to_string())
        .execute(&mut *fixture_tx)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET payload_json=jsonb_build_object('private_diagnostic','status-private-source-sentinel') WHERE id=$1")
        .bind(job).execute(&mut *fixture_tx).await.unwrap();
    fixture_tx.commit().await.unwrap();
    let published: DateTime<Utc> = sqlx::query_scalar("SELECT published_at FROM candidate_raw_batch_publications WHERE surface='price' AND entitlement_date=$1 AND state='PUBLISHED' ORDER BY published_at DESC LIMIT 1")
        .bind(session).fetch_one(&h.owner_pool).await.unwrap();
    let writer = h.research_writer_pool().await;
    sqlx::query("INSERT INTO research_execution_status(id,attempt_started_at,heartbeat_at,attempt_finished_at,attempt_state,last_success_at,target_session,published_session,updated_at) VALUES('kis-eod',$2::timestamptz-interval '2 minutes',$2,$2,'SUCCEEDED',$2,$1,$1,clock_timestamp())")
        .bind(session).bind(published).execute(&writer).await.expect("persisted successful research source");
    let actor = h.owner.actor();
    let hash = actor_session_hash(&actor, &h.owner.cookie_value);
    let baseline =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    let groups = [
        "queue",
        "etf",
        "rights",
        "v2",
        "candidate",
        "research",
        "calendar",
        "stream",
    ];
    for group in groups {
        assert_eq!(
            baseline["db_read_results"][group]["result"], "ok",
            "populated baseline {group}"
        );
        assert!(baseline["db_read_results"][group]["observed_at"].is_string());
    }
    assert_eq!(
        baseline["db_read_results"]["queue"]["groups"]["recommendation"]["queued"],
        1
    );
    assert_eq!(baseline["db_read_results"]["etf"]["covered_count"], 11);
    assert_eq!(baseline["db_read_results"]["rights"]["rights_valid"], true);
    assert_eq!(baseline["db_read_results"]["v2"]["required_count"], 2);
    assert_eq!(baseline["db_read_results"]["v2"]["covered_count"], 2);
    assert_eq!(
        baseline["db_read_results"]["candidate"]["published_universes"],
        1
    );
    assert_eq!(
        baseline["db_read_results"]["research"]["row"]["attempt_state"],
        "SUCCEEDED"
    );
    assert_eq!(
        baseline["db_read_results"]["calendar"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        baseline["db_read_results"]["stream"]["owner_binding_valid"],
        true
    );
    assert_eq!(baseline["db_read_results"]["stream"]["demand_count"], 1);
    assert_eq!(baseline["db_read_results"]["stream"]["live_count"], 1);

    // Rights is unavailable when ETF cannot supply its exact binding. Stream
    // independently reads rights, READY membership and current calendar guards,
    // so those table locks intentionally time out both source transactions.
    let cases: [(&str, &str, &[&str], &[&str]); 8] = [
        ("queue", "jobs", &["queue"], &["recommendation-runner"]),
        (
            "etf",
            "candidate_price_publications",
            &["etf", "rights"],
            &["kis-eod"],
        ),
        (
            "rights",
            "data_entitlements",
            &["rights", "stream"],
            &["kis-eod", "market-stream"],
        ),
        (
            "v2",
            "owner_equity_memberships",
            &["v2", "stream"],
            &["owner-equity-v2-runner", "market-stream"],
        ),
        (
            "candidate",
            "candidate_scheduler_control",
            &["candidate"],
            &["candidate-runner"],
        ),
        (
            "research",
            "research_execution_status",
            &["research"],
            &["kis-eod"],
        ),
        (
            "calendar",
            "trading_calendars",
            &["calendar", "stream"],
            &["intraday-calendar", "market-stream"],
        ),
        (
            "stream",
            "owner_market_stream_producers",
            &["stream"],
            &["market-stream"],
        ),
    ];
    let mut evidence = Vec::new();
    for (target, table, affected, services) in cases {
        refresh_stream_clocks(&h, &stream).await;
        let mut lock_tx = h.owner_pool.begin().await.expect("isolated source lock");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "LOCK TABLE public.{table} IN ACCESS EXCLUSIVE MODE"
        )))
        .execute(&mut *lock_tx)
        .await
        .expect("fixed fixture source table lock");
        let started = Instant::now();
        let mut input = tokio::time::timeout(
            Duration::from_millis(2100),
            api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None),
        )
        .await
        .expect("every source refresh is bounded");
        let elapsed = started.elapsed();
        lock_tx
            .rollback()
            .await
            .expect("source lock released before assertions");
        assert!(
            elapsed < Duration::from_secs(2),
            "{target} evidence deadline: {elapsed:?}"
        );
        for group in groups {
            let expected = if target == "etf" && group == "rights" {
                "unavailable"
            } else if affected.contains(&group) {
                "timeout"
            } else {
                "ok"
            };
            assert_eq!(
                input["db_read_results"][group]["result"], expected,
                "{target} lock: {group}"
            );
            assert_eq!(
                input["db_read_results"][group]["observed_at"].is_string(),
                expected == "ok",
                "failed transactions cannot fabricate clocks or poison independent observations: {target}/{group}"
            );
        }
        let now = Utc::now();
        attach_synthetic_host(&mut input, now);
        let response =
            api_server::service_status::project(&input, now).expect("typed independent failures");
        for &id in services {
            let row = service(&response, id);
            assert!(
                row["reason_codes"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("DB_TIMEOUT")),
                "{target}: {id} needs its source timeout diagnosis: {row}"
            );
            assert_ne!(row["status"], "healthy");
            if id == "market-stream" {
                assert!(row["metrics"]["live_identities"].is_null());
            }
        }
        let shape: Value = serde_json::from_str(BASE_OUTPUT).unwrap();
        assert_same_shape(&response, &shape);
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains(&format!("\"{table}\"")));
        for private in [
            "status-private-source-sentinel",
            "55P03",
            "lock timeout",
            "entitlement_id",
            "failure_code",
        ] {
            assert!(
                !encoded.contains(private),
                "private diagnostics must not escape: {target}/{private}"
            );
        }
        let recovered =
            api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None)
                .await;
        for group in groups {
            assert_eq!(
                recovered["db_read_results"][group]["result"], "ok",
                "{target} rollback recovery: {group}"
            );
        }
        evidence.push(serde_json::json!({"source":target,"affected":affected,"evidence_ms":elapsed.as_millis(),"all_independent_observations":true,"all_transactions_recovered":true}));
    }
    if let Some(dir) = std::env::var_os("QA_SERVICE_STATUS_ARTIFACT_DIR") {
        std::fs::write(
            PathBuf::from(dir).join("source-timeouts.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
    writer.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn owner_api_is_read_only_audited_private_and_reuses_evidence_after_revalidation() {
    let harness = required_harness().await;
    let leases_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_market_stream_leases WHERE owner_user_id=$1",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("initial synthetic Owner lease count");

    let anonymous = harness.get(PATH, None).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let anonymous_body = common::Harness::body_json(anonymous).await;
    assert!(anonymous_body["error"]["details"].is_null());

    let started = Instant::now();
    let first = harness.get(PATH, Some(&harness.owner)).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        first
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );
    assert!(
        first
            .headers()
            .get("vary")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value
                .split(',')
                .any(|field| field.trim().eq_ignore_ascii_case("cookie")))
    );
    assert_eq!(
        first
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("test-rid-1")
    );
    let first_body = common::Harness::body_json(first).await;
    let output_shape: Value =
        serde_json::from_str(BASE_OUTPUT).expect("base response oracle parses");
    assert_same_shape(&first_body, &output_shape);
    assert_eq!(
        first_body["services"].as_array().expect("services").len(),
        22
    );
    for service in first_body["services"].as_array().expect("service rows") {
        if service["sources"]["database"].is_object() {
            assert_eq!(
                service["sources"]["database"]["result"], "ok",
                "mandatory database query source for {}",
                service["id"]
            );
        }
    }
    let first_observed = first_body
        .pointer("/services/11/sources/database/observed_at")
        .cloned()
        .expect("KIS evidence query clock");
    let first_generated = first_body["generated_at"].clone();

    let elapsed = started.elapsed();
    let admission_and_cache_window = Duration::from_millis(5_010);
    if elapsed < admission_and_cache_window {
        tokio::time::sleep(admission_and_cache_window - elapsed).await;
    }
    let second = harness.get(PATH, Some(&harness.owner)).await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = common::Harness::body_json(second).await;
    assert_ne!(second_body["generated_at"], first_generated);
    assert_eq!(
        second_body.pointer("/services/11/sources/database/observed_at"),
        Some(&first_observed),
        "the cached evidence clock is retained while response assembly is fresh"
    );

    let member = harness.get(PATH, Some(&harness.member)).await;
    assert_eq!(member.status(), StatusCode::FORBIDDEN);
    let member_body = common::Harness::body_json(member).await;
    assert_eq!(member_body["error"]["code"], "FORBIDDEN");
    assert!(member_body["error"]["details"].is_null());

    let owner_audits = audit_rows_for_actor(&harness, harness.owner.user_id).await;
    assert_eq!(
        owner_audits.len(),
        2,
        "owner refresh and cache-hit reads audited"
    );
    for row in &owner_audits {
        assert_eq!(
            row.try_get::<String, _>("action").expect("audit action"),
            "admin.service_status.read"
        );
        assert_eq!(
            row.try_get::<String, _>("target_type").expect("audit type"),
            "service_status"
        );
        assert_eq!(
            row.try_get::<String, _>("target_id").expect("audit target"),
            "all"
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("reason")
                .expect("audit reason")
                .as_deref(),
            Some("PARTIAL")
        );
    }
    let member_audits = audit_rows_for_actor(&harness, harness.member.user_id).await;
    assert_eq!(member_audits.len(), 1);
    assert_eq!(
        member_audits[0]
            .try_get::<Option<String>, _>("reason")
            .expect("denial audit reason")
            .as_deref(),
        Some("DENIED")
    );
    let leases_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_market_stream_leases WHERE owner_user_id=$1",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("final synthetic Owner lease count");
    assert_eq!(
        leases_before, leases_after,
        "status GET creates no market demand"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn rate_rejection_has_retry_after_and_does_not_audit_or_refresh() {
    let harness = required_harness().await;
    let first = harness.get(PATH, Some(&harness.owner)).await;
    assert_eq!(first.status(), StatusCode::OK);
    let before = audit_rows_for_actor(&harness, harness.owner.user_id)
        .await
        .len();
    let second = harness.get(PATH, Some(&harness.owner)).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        second
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("5")
    );
    let body = common::Harness::body_json(second).await;
    assert_eq!(body["error"]["code"], "SERVICE_STATUS_RATE_LIMITED");
    assert!(body["services"].is_null());
    assert_eq!(
        audit_rows_for_actor(&harness, harness.owner.user_id)
            .await
            .len(),
        before,
        "admission rejection creates no status audit"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn audit_store_failure_returns_typed_503_without_operational_data() {
    let harness = required_harness().await;
    harness.audit_pool.close().await;
    let response = harness.get(PATH, Some(&harness.owner)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = common::Harness::body_json(response).await;
    assert_eq!(body["error"]["code"], "SERVICE_STATUS_UNAVAILABLE");
    assert!(body.get("services").is_none());
    assert!(body["error"]["details"].is_null());
    harness.teardown().await;
}

#[tokio::test]
async fn blocked_audit_has_its_separate_deadline_and_releases_no_operational_data() {
    let h = required_harness().await;
    let mut lock_tx = h
        .owner_pool
        .begin()
        .await
        .expect("isolated audit table lock");
    sqlx::query("LOCK TABLE public.audit_logs IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock_tx)
        .await
        .unwrap();
    let started = Instant::now();
    let response = tokio::time::timeout(Duration::from_millis(1500), h.get(PATH, Some(&h.owner)))
        .await
        .expect("audit deadline is independent of the three-second handler ceiling");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(450),
        "actual INSERT waited for its audit budget"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "500ms audit plus unlocked bounded evidence: {elapsed:?}"
    );
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = common::Harness::body_json(response).await;
    assert_eq!(body["error"]["code"], "SERVICE_STATUS_UNAVAILABLE");
    assert!(body.get("services").is_none());
    assert!(body["error"]["details"].is_null());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action='admin.service_status.read'",
    )
    .fetch_one(&mut *lock_tx)
    .await
    .unwrap();
    assert_eq!(count, 0, "no audit committed before releasing the lock");
    lock_tx.rollback().await.unwrap();
    if let Some(dir) = std::env::var_os("QA_SERVICE_STATUS_ARTIFACT_DIR") {
        std::fs::write(PathBuf::from(dir).join("audit-timeout.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"handler_ms":elapsed.as_millis(),"audit_budget_ms":500,"released_operational_data":false})).unwrap()).unwrap();
    }
    h.teardown().await;
}

#[tokio::test]
async fn expired_session_is_rejected_before_admission_and_owner_without_entitlement_is_admitted() {
    let harness = required_harness().await;
    let expired = harness
        .seed_user(
            Role::Owner,
            "status-expired@lagrange.test",
            "status-expired-iss",
            "status-expired-sub",
        )
        .await;
    harness
        .seed_tenant(
            &expired,
            &format!(
                "UPDATE web_sessions SET created_at=now()-interval '2 hours', expires_at=now()-interval '1 second' WHERE user_id='{}'",
                expired.user_id
            ),
        )
        .await;
    let response = harness.get(PATH, Some(&expired)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = common::Harness::body_json(response).await;
    assert_eq!(body["error"]["code"], "SESSION_EXPIRED");
    assert!(body["error"]["details"].is_null());
    assert!(
        audit_rows_for_actor(&harness, expired.user_id)
            .await
            .is_empty()
    );

    let second_owner = harness
        .seed_user(
            Role::Owner,
            "status-no-entitlement@lagrange.test",
            "status-no-entitlement-iss",
            "status-no-entitlement-sub",
        )
        .await;
    let response = harness.get(PATH, Some(&second_owner)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::Harness::body_json(response).await;
    assert_eq!(
        body["services"].as_array().expect("Owner response").len(),
        22
    );
    assert_eq!(
        audit_rows_for_actor(&harness, second_owner.user_id)
            .await
            .len(),
        1
    );
    harness.teardown().await;
}

#[tokio::test]
async fn actual_actor_scoped_v2_query_accepts_31_and_100_and_detects_the_101st_sentinel() {
    let harness = required_harness().await;
    let state = harness.state();
    let owner = harness.owner.actor();
    let session_hash = actor_session_hash(&owner, &harness.owner.cookie_value);
    let today: NaiveDate =
        sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&harness.owner_pool)
            .await
            .expect("synthetic fixture date");

    seed_v2_range(
        &harness.owner_pool,
        harness.owner.user_id,
        1,
        31,
        today,
        true,
    )
    .await;
    let now = Utc::now();
    let first =
        api_server::repos::service_status::refresh(&state, &owner, &session_hash, now, None).await;
    assert_eq!(v2_group(&first)["result"], "ok");
    assert_eq!(v2_group(&first)["required_count"], 31);
    assert_eq!(v2_group(&first)["covered_count"], 31);
    assert_eq!(v2_group(&first)["snapshot_as_of"], today.to_string());
    assert_eq!(v2_group(&first)["current_admissions_match"], true);

    let second_owner = harness
        .seed_user(
            Role::Owner,
            "status-cross-owner@lagrange.test",
            "status-cross-owner-iss",
            "status-cross-owner-sub",
        )
        .await;
    let second_actor = second_owner.actor();
    let second_hash = actor_session_hash(&second_actor, &second_owner.cookie_value);
    let foreign = api_server::repos::service_status::refresh(
        &state,
        &second_actor,
        &second_hash,
        Utc::now(),
        None,
    )
    .await;
    assert_eq!(v2_group(&foreign)["required_count"], 0);

    seed_v2_range(
        &harness.owner_pool,
        harness.owner.user_id,
        32,
        100,
        today,
        true,
    )
    .await;
    let second =
        api_server::repos::service_status::refresh(&state, &owner, &session_hash, Utc::now(), None)
            .await;
    assert_eq!(v2_group(&second)["result"], "ok");
    assert_eq!(v2_group(&second)["required_count"], 100);
    assert_eq!(v2_group(&second)["covered_count"], 100);
    assert_eq!(v2_group(&second)["snapshot_as_of"], today.to_string());

    seed_v2_range(
        &harness.owner_pool,
        harness.owner.user_id,
        101,
        101,
        today,
        false,
    )
    .await;
    let third =
        api_server::repos::service_status::refresh(&state, &owner, &session_hash, Utc::now(), None)
            .await;
    assert_eq!(v2_group(&third)["result"], "ok");
    assert_eq!(v2_group(&third)["required_count"], 101);
    assert_eq!(v2_group(&third)["current_admissions_match"], false);

    let mut tx = harness
        .app_pool
        .begin()
        .await
        .expect("EXPLAIN transaction begins");
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .expect("EXPLAIN transaction is read-only");
    api_server::actor_tx::set_actor_guc(&mut tx, &owner)
        .await
        .expect("EXPLAIN actor context");
    sqlx::query("SET LOCAL statement_timeout='250ms'")
        .execute(&mut *tx)
        .await
        .expect("EXPLAIN statement bound");
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {}",
        api_server::repos::service_status::V2_SQL
    )))
    .bind(harness.owner.user_id)
    .fetch_one(&mut *tx)
    .await
    .expect("synthetic 101-identity V2 query plan fits the evidence statement bound");
    assert!(plan.as_array().is_some_and(|items| !items.is_empty()));
    tx.commit()
        .await
        .expect("read-only EXPLAIN transaction commits");
    harness.teardown().await;
}

#[test]
fn current_trusted_file_and_policy_invalidate_actual_serving_caches_on_replacement() {
    use api_server::service_status::ServiceStatusRuntime;
    let root = temp_root();
    let (mut snapshot, now, release) = snapshot_fixture();
    write_snapshot(&root.0, &snapshot);
    let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
    let runtime = ServiceStatusRuntime::default();
    let at = Instant::now();
    runtime
        .admit("owner-a", "session-a", at)
        .expect("actor slot");
    let host = runtime.observe_host_for_test(&root.0, owner, now, &release);
    let input: Value = serde_json::from_str(BASE_INPUT).expect("safe synthetic evidence");
    runtime.cache_owner_evidence("owner-a", "session-a", &host, input.clone(), at);
    runtime.cache_shared_for_host(&host, serde_json::json!({"marker":"first"}), at);
    assert!(
        runtime
            .cached_owner_evidence("owner-a", "session-a", &host, at, now)
            .is_some()
    );
    assert!(
        runtime
            .cached_owner_evidence("owner-a", "session-b", &host, at, now)
            .is_none()
    );
    assert!(
        runtime
            .cached_owner_evidence("owner-b", "session-a", &host, at, now)
            .is_none()
    );
    for mutation in ["lease", "mode", "receipt"] {
        match mutation {
            "lease" => {
                snapshot["policy"]["job_lease_seconds"]["recommendation"] = serde_json::json!(120)
            }
            "mode" => {
                snapshot["policy"]["research_mode"] = serde_json::json!("daemon");
                snapshot["policy"]["profile"] = serde_json::json!("owner_daemon");
            }
            _ => {
                snapshot["policy"]["policy_installed_at"] =
                    serde_json::json!("2032-02-12T00:00:01Z")
            }
        }
        let replacement = root.0.join("snapshot.next");
        std::fs::write(
            &replacement,
            serde_json::to_vec(&snapshot).expect("replacement bytes"),
        )
        .expect("replacement written");
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o640))
            .expect("replacement mode");
        std::fs::rename(&replacement, root.0.join("snapshot.json"))
            .expect("same-commit atomic replacement");
        let current = runtime.observe_host_for_test(&root.0, owner, now, &release);
        assert!(
            runtime
                .cached_owner_evidence("owner-a", "session-a", &current, at, now)
                .is_none(),
            "{mutation} replacement invalidates actor evidence"
        );
        assert!(
            runtime.cached_shared_for_host(&current, at).is_none(),
            "{mutation} replacement invalidates shared evidence"
        );
        runtime.cache_owner_evidence("owner-a", "session-a", &current, input.clone(), at);
        runtime.cache_shared_for_host(&current, serde_json::json!({"marker":mutation}), at);
        let served = runtime
            .cached_owner_evidence("owner-a", "session-a", &current, at, now)
            .expect("current validated cache");
        assert_eq!(
            served["host_read"]["snapshot"]["policy"],
            snapshot["policy"]
        );
    }
    // Every lookup rechecks permissions, even on the identical inode/commit.
    std::fs::set_permissions(
        root.0.join("snapshot.json"),
        std::fs::Permissions::from_mode(0o660),
    )
    .expect("unsafe replacement mode");
    let unsafe_host = runtime.observe_host_for_test(&root.0, owner, now, &release);
    assert!(
        runtime
            .cached_owner_evidence("owner-a", "session-a", &unsafe_host, at, now)
            .is_none()
    );
    assert!(runtime.cached_shared_for_host(&unsafe_host, at).is_none());
    let unsafe_input = {
        let mut value = input;
        runtime.cache_owner_evidence("owner-a", "session-a", &unsafe_host, value.clone(), at);
        value = runtime
            .cached_owner_evidence("owner-a", "session-a", &unsafe_host, at, now)
            .expect("typed unavailable evidence");
        value
    };
    assert_eq!(unsafe_input["host_read"]["result"], "invalid");
    assert!(unsafe_input["host_read"]["snapshot"].is_null());
}

#[test]
fn optional_mount_absence_is_missing_but_symlink_and_unsafe_parent_are_invalid() {
    use api_server::service_status::{HostReadError, read_host_snapshot_for_test};
    let root = temp_root();
    let (_, now, release) = snapshot_fixture();
    let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
    let absent = root.0.join("absent-mount");
    assert_eq!(
        read_host_snapshot_for_test(&absent, now, &release, owner),
        Err(HostReadError::Missing)
    );
    let link = root.0.join("linked-mount");
    std::os::unix::fs::symlink(&absent, &link).expect("synthetic mount symlink");
    assert_eq!(
        read_host_snapshot_for_test(&link, now, &release, owner),
        Err(HostReadError::Invalid)
    );
    std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o770))
        .expect("unsafe parent");
    assert_eq!(
        read_host_snapshot_for_test(&absent, now, &release, owner),
        Err(HostReadError::Invalid)
    );
}

fn service<'a>(response: &'a Value, id: &str) -> &'a Value {
    response["services"]
        .as_array()
        .expect("service rows")
        .iter()
        .find(|row| row["id"] == id)
        .expect("fixed service")
}

#[test]
fn cached_halted_assurance_keeps_original_times_and_inclusive_deadline() {
    use api_server::service_status::ServiceStatusRuntime;
    let root = temp_root();
    let (snapshot, now, release) = snapshot_fixture();
    write_snapshot(&root.0, &snapshot);
    let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
    let runtime = ServiceStatusRuntime::default();
    let at = Instant::now();
    runtime
        .admit("owner", "session", at)
        .expect("actor admitted");
    let host = runtime.observe_host_for_test(&root.0, owner, now, &release);
    let mut input: Value = serde_json::from_str(BASE_INPUT).expect("base evidence");
    input["db_read_results"]["stream"]["live_count"] = serde_json::json!(0);
    input["db_read_results"]["stream"]["halted_count"] = serde_json::json!(2);
    input["db_read_results"]["stream"]["producer_heartbeat_at"] =
        serde_json::json!("2032-02-12T00:59:53Z");
    input["db_read_results"]["stream"]["valid_until"] = serde_json::json!("2032-02-12T01:00:03Z");
    runtime.cache_owner_evidence("owner", "session", &host, input.clone(), at);
    let original = input["db_read_results"]["stream"].clone();
    let boundary = now + chrono::Duration::seconds(3);
    for (clock, status) in [
        (boundary - chrono::Duration::microseconds(1), "waiting"),
        (boundary, "waiting"),
        (boundary + chrono::Duration::microseconds(1), "failed"),
    ] {
        let evidence = runtime
            .cached_owner_evidence(
                "owner",
                "session",
                &host,
                at + (clock - now).to_std().unwrap(),
                clock,
            )
            .expect("evidence reused within cache age");
        assert_eq!(
            evidence["db_read_results"]["stream"], original,
            "cache never advances raw clocks or aggregate bound"
        );
        let response =
            api_server::service_status::project(&evidence, clock).expect("cache-time projection");
        let row = service(&response, "market-stream");
        assert_eq!(row["status"], status);
        assert_eq!(row["valid_until"], "2032-02-12T01:00:03Z");
        if status == "waiting" {
            assert_eq!(row["work_status"], "closed");
            assert_eq!(row["metrics"]["halted_identities"], 2);
        } else {
            assert!(
                row["reason_codes"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("PRODUCER_STALE"))
            );
            assert!(row["metrics"]["halted_identities"].is_null());
        }
    }
}

#[test]
fn accepted_quote_equality_and_strict_lease_margin_never_extend_the_published_bound() {
    let base: Value = serde_json::from_str(BASE_INPUT).expect("base");
    let now = DateTime::parse_from_rfc3339(base["clock"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    for field in ["oldest_received_at", "oldest_provider_trade_at"] {
        let mut input = base.clone();
        input["db_read_results"]["stream"][field] = serde_json::json!(
            (now - chrono::Duration::seconds(30))
                .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        );
        input["db_read_results"]["stream"]["valid_until"] = serde_json::json!(base["clock"]);
        let equal =
            api_server::service_status::project(&input, now).expect("inclusive quote equality");
        assert_eq!(service(&equal, "market-stream")["status"], "healthy");
        assert_eq!(
            service(&equal, "market-stream")["valid_until"],
            base["clock"]
        );
        let later =
            api_server::service_status::project(&input, now + chrono::Duration::microseconds(1))
                .expect("later cached projection");
        assert_eq!(service(&later, "market-stream")["status"], "degraded");
        assert!(service(&later, "market-stream")["metrics"]["live_identities"].is_null());
    }
    let mut input = base.clone();
    let deadline = now + chrono::Duration::microseconds(1);
    input["db_read_results"]["stream"]["producer_lease_expires_at"] = serde_json::json!(
        (deadline + chrono::Duration::seconds(5))
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
    );
    input["db_read_results"]["stream"]["valid_until"] =
        serde_json::json!(deadline.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
    let before =
        api_server::service_status::project(&input, now).expect("strict lease before boundary");
    assert_eq!(service(&before, "market-stream")["status"], "healthy");
    let equal =
        api_server::service_status::project(&input, deadline).expect("strict lease equality");
    assert_eq!(service(&equal, "market-stream")["status"], "failed");
    assert_eq!(
        service(&equal, "market-stream")["valid_until"],
        input["db_read_results"]["stream"]["valid_until"]
    );
}

#[test]
fn queue_summary_cache_refreshes_at_count_changes_without_serializing_raw_jobs() {
    use api_server::service_status::ServiceStatusRuntime;
    let root = temp_root();
    let (snapshot, now, release) = snapshot_fixture();
    write_snapshot(&root.0, &snapshot);
    let runtime = ServiceStatusRuntime::default();
    let at = Instant::now();
    runtime.admit("owner", "session", at).unwrap();
    let host = runtime.observe_host_for_test(
        &root.0,
        (unsafe { libc::geteuid() }, unsafe { libc::getegid() }),
        now,
        &release,
    );
    let mut input: Value = serde_json::from_str(BASE_INPUT).unwrap();
    input["db_read_results"]["queue"] = serde_json::json!({"result":"ok","observed_at":input["clock"],"groups":{"recommendation":{"result":"ok","observed_at":input["clock"],"queued":0,"running":10000,"eligible_queued":0,"recent_failed":0,"expired_claims":0,"oldest_eligible_at":null,"next_available_at":null,"next_claim_expiry_at":"2032-02-12T01:00:01Z","next_failure_expiry_at":null,"recheck_at":"2032-02-12T01:00:01Z","lease_seconds":60,"invalid":false,"last_success_at":null,"last_failure_at":null}}});
    runtime.cache_owner_evidence("owner", "session", &host, input, at);
    assert!(
        runtime
            .cached_owner_evidence(
                "owner",
                "session",
                &host,
                at,
                now + chrono::Duration::microseconds(999999)
            )
            .is_some()
    );
    assert!(
        runtime
            .cached_owner_evidence(
                "owner",
                "session",
                &host,
                at,
                now + chrono::Duration::seconds(1)
            )
            .is_none(),
        "fresh read required at exact count transition"
    );
}

#[tokio::test]
async fn actual_queue_history_uses_current_job_identity_and_preserves_old_terminal_times() {
    use service_status_support::{read_queue_summary, seed_queue_jobs};
    let h = required_harness().await;
    let actor = h.owner.actor();
    let old_success = seed_queue_jobs(
        &h,
        h.owner.user_id,
        "recommendation",
        1,
        "SUCCEEDED",
        172800,
    )
    .await[0];
    let old_failure =
        seed_queue_jobs(&h, h.owner.user_id, "recommendation", 1, "FAILED", 129600).await[0];
    let recent = seed_queue_jobs(&h, h.owner.user_id, "recommendation", 1, "FAILED", 60).await[0];
    seed_queue_jobs(&h, h.owner.user_id, "recommendation", 1, "SUCCEEDED", 10).await;
    let first = read_queue_summary(&h, &actor).await;
    assert_eq!(
        first["recommendation"]["recent_failed"], 1,
        "another job's success cannot recover the failed job"
    );
    assert!(!first["recommendation"]["last_success_at"].is_null());
    let mut tx = h
        .owner_pool
        .begin()
        .await
        .expect("same-job recovery fixture");
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(h.owner.user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE jobs SET status='QUEUED',finished_at=NULL WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(recent)
    .bind(h.owner.user_id)
    .execute(&mut *tx)
    .await
    .expect("same job requeued");
    tx.commit().await.unwrap();
    let recovered = read_queue_summary(&h, &actor).await;
    assert_eq!(recovered["recommendation"]["recent_failed"], 0);
    let latest_failure = DateTime::parse_from_rfc3339(
        recovered["recommendation"]["last_failure_at"]
            .as_str()
            .expect("historical failure retained"),
    )
    .unwrap()
    .with_timezone(&Utc);
    let mut history_tx = h.app_pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *history_tx)
        .await
        .unwrap();
    api_server::actor_tx::set_actor_guc(&mut history_tx, &actor)
        .await
        .unwrap();
    let stored_old: DateTime<Utc> = sqlx::query_scalar("SELECT finished_at FROM jobs WHERE id=$1")
        .bind(old_failure)
        .fetch_one(&mut *history_tx)
        .await
        .unwrap();
    history_tx.commit().await.unwrap();
    assert_eq!(
        latest_failure, stored_old,
        "latest current failure is returned beyond 24h"
    );
    // A separate group containing only old terminal records still has history.
    seed_queue_jobs(&h, h.owner.user_id, "backtest", 1, "SUCCEEDED", 172800).await;
    seed_queue_jobs(&h, h.owner.user_id, "backtest", 1, "FAILED", 129600).await;
    let older = read_queue_summary(&h, &actor).await;
    assert_eq!(older["backtest"]["recent_failed"], 0);
    assert!(older["backtest"]["last_success_at"].is_string());
    assert!(older["backtest"]["last_failure_at"].is_string());
    assert!(old_success != old_failure);
    let foreign = read_queue_summary(&h, &h.member.actor()).await;
    assert_eq!(
        foreign["recommendation"]["queued"], 0,
        "actor RLS refuses an explicitly bound foreign Owner"
    );
    assert!(foreign["recommendation"]["last_failure_at"].is_null());
    h.teardown().await;
}

#[tokio::test]
async fn every_queue_type_has_its_own_10001_sentinel_and_realistic_app_role_plan() {
    use service_status_support::{
        queue_leases, read_queue_summary, seed_beta_history, seed_queue_jobs,
    };
    let h = required_harness().await;
    let kinds = [
        "owner_equity_v2",
        "owner_beta_price_recommendation",
        "candidate_compute",
        "recommendation",
        "backtest",
    ];
    for kind in kinds {
        seed_queue_jobs(&h, h.owner.user_id, kind, 10000, "QUEUED", 10).await;
    }
    let complete = read_queue_summary(&h, &h.owner.actor()).await;
    for kind in kinds {
        assert_eq!(complete[kind]["result"], "ok");
        assert_eq!(complete[kind]["queued"], 10000);
    }
    assert!(
        serde_json::to_vec(&complete).unwrap().len() < 16 * 1024,
        "50,000 source jobs become compact bounded summaries"
    );
    for (index, kind) in kinds.iter().enumerate() {
        seed_queue_jobs(&h, h.owner.user_id, kind, 1, "QUEUED", 10).await;
        let groups = read_queue_summary(&h, &h.owner.actor()).await;
        for (other_index, other) in kinds.iter().enumerate() {
            assert_eq!(
                groups[*other]["result"],
                if other_index <= index { "limit" } else { "ok" },
                "{kind} sentinel must not hide or truncate {other}"
            );
        }
    }
    // Include substantial old mixed-status history to exercise indexed LIMIT 1
    // selections rather than an empty plan or a max() over recent rows.
    seed_queue_jobs(
        &h,
        h.owner.user_id,
        "recommendation",
        20000,
        "SUCCEEDED",
        172800,
    )
    .await;
    seed_queue_jobs(&h, h.owner.user_id, "recommendation", 1, "FAILED", 259200).await;
    seed_beta_history(&h, 1, "SUCCEEDED", 259200).await;
    let latest_beta_success = seed_beta_history(&h, 1, "SUCCEEDED", 172800).await[0];
    seed_beta_history(&h, 1000, "FAILED", 129600).await;
    let actor = h.owner.actor();
    let session = actor_session_hash(&actor, &h.owner.cookie_value);
    let actual =
        api_server::repos::service_status::refresh(&h.state(), &actor, &session, Utc::now(), None)
            .await;
    let returned_beta = DateTime::parse_from_rfc3339(
        actual["db_read_results"]["queue"]["owner_beta_success_at"]
            .as_str()
            .expect("historical beta success"),
    )
    .unwrap()
    .with_timezone(&Utc);
    assert_eq!(
        returned_beta, latest_beta_success,
        "indexed LIMIT 1 is current-status history beyond 24h"
    );
    let mut tx = h.app_pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    api_server::actor_tx::set_actor_guc(&mut tx, &h.owner.actor())
        .await
        .unwrap();
    sqlx::query("SET LOCAL statement_timeout='250ms'")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL lock_timeout='50ms'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let sequential_scans: String = sqlx::query_scalar("SHOW enable_seqscan")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        sequential_scans, "on",
        "populated app-role plan uses the natural planner"
    );
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) {}",
        api_server::repos::service_status::QUEUE_SQL
    )))
    .bind(h.owner.user_id)
    .bind(queue_leases())
    .fetch_one(&mut *tx)
    .await
    .expect("actual 50k+history query meets the fixed statement budget");
    fn has_index(value: &Value, name: &str) -> bool {
        match value {
            Value::Object(values) => {
                values.get("Index Name").and_then(Value::as_str) == Some(name)
                    || values.values().any(|value| has_index(value, name))
            }
            Value::Array(values) => values.iter().any(|value| has_index(value, name)),
            _ => false,
        }
    }
    assert!(
        has_index(&plan, "jobs_service_status_owner_type_finished_idx")
            || has_index(&plan, "jobs_service_status_owner_type_status_finished_idx"),
        "actual terminal history lookup needs its bound index"
    );
    fn contains_relation(node: &Value, name: &str) -> bool {
        node["Relation Name"] == name
            || node["Plans"]
                .as_array()
                .is_some_and(|children| children.iter().any(|child| contains_relation(child, name)))
    }
    fn beta_limit(node: &Value) -> Option<&Value> {
        if node["Node Type"] == "Limit" && contains_relation(node, "owner_beta_recommendation_runs")
        {
            return Some(node);
        }
        node["Plans"].as_array()?.iter().find_map(beta_limit)
    }
    fn has_sort(node: &Value) -> bool {
        matches!(
            node["Node Type"].as_str(),
            Some("Sort" | "Incremental Sort")
        ) || node["Plans"]
            .as_array()
            .is_some_and(|children| children.iter().any(has_sort))
    }
    fn beta_ordered_scan(node: &Value) -> Option<&Value> {
        if node["Relation Name"] == "owner_beta_recommendation_runs"
            && node["Index Name"] == "owner_beta_service_status_owner_finished_idx"
            && matches!(
                node["Node Type"].as_str(),
                Some("Index Scan" | "Index Only Scan")
            )
        {
            return Some(node);
        }
        node["Plans"].as_array()?.iter().find_map(beta_ordered_scan)
    }
    let beta = beta_limit(&plan[0]["Plan"]).expect("real Owner-beta LIMIT 1 subtree is present");
    assert!(
        !has_sort(beta),
        "completion order must come from the Owner-beta index, not a history sort: {beta}"
    );
    let scan = beta_ordered_scan(beta).expect("Owner-beta uses its exact ordered success index");
    assert_eq!(
        scan["Actual Rows"].as_f64(),
        Some(1.0),
        "newest successful completion is found without scanning failed history"
    );
    if let Some(dir) = std::env::var_os("QA_SERVICE_STATUS_ARTIFACT_DIR") {
        std::fs::write(
            std::path::PathBuf::from(dir).join("queue-explain.json"),
            serde_json::to_vec_pretty(&plan).unwrap(),
        )
        .expect("owned bounded EXPLAIN evidence saved");
    }
    tx.commit().await.unwrap();
    h.teardown().await;
}

#[tokio::test]
async fn actual_etf11_publication_and_pinned_rights_are_positive_then_fail_closed() {
    use service_status_support::{attach_synthetic_host, seed_calendar_days, seed_etf_publication};
    let h = required_harness().await;
    seed_calendar_days(&h).await;
    let (dataset, entitlement, session) = seed_etf_publication(&h, 11).await;
    let state = h.state();
    let actor = h.owner.actor();
    let hash = actor_session_hash(&actor, &h.owner.cookie_value);
    let mut positive =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(positive["db_read_results"]["etf"]["required_count"], 11);
    assert_eq!(positive["db_read_results"]["etf"]["covered_count"], 11);
    assert_eq!(
        positive["db_read_results"]["etf"]["publication_session"],
        session.to_string()
    );
    assert_eq!(positive["db_read_results"]["rights"]["rights_valid"], true);
    let now = Utc::now();
    attach_synthetic_host(&mut positive, now);
    let response = api_server::service_status::project(&positive, now)
        .expect("real source positive projection");
    assert_eq!(
        service(&response, "kis-eod")["metrics"]["data_as_of"],
        session.to_string()
    );
    assert!(service(&response, "kis-eod")["metrics"]["last_success_at"].is_string());
    // An unrelated active grant must not conceal revocation of the exact pin.
    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE id=$1")
        .bind(entitlement)
        .execute(&h.owner_pool)
        .await
        .expect("synthetic exact right revocation");
    let mut revoked =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(revoked["db_read_results"]["rights"]["rights_valid"], false);
    let now = Utc::now();
    attach_synthetic_host(&mut revoked, now);
    let response =
        api_server::service_status::project(&revoked, now).expect("real source revoked projection");
    assert!(service(&response, "kis-eod")["metrics"]["data_as_of"].is_null());
    assert!(
        service(&response, "kis-eod")["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("DATA_BLOCKED"))
    );
    sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE id=$1")
        .bind(entitlement)
        .execute(&h.owner_pool)
        .await
        .unwrap();
    // Invalidate the exact mandatory Raw binding after genuine publication;
    // migration_owner mutates only disposable synthetic fixture evidence.
    for raw_mode in ["synthetic", "credentialed"] {
        sqlx::query("UPDATE candidate_raw_batch_publications SET fetch_mode=$2 WHERE batch_id IN (SELECT batch_id FROM candidate_raw_batch_datasets WHERE dataset_version_id=$1)")
            .bind(dataset).bind(raw_mode).execute(&h.owner_pool).await.unwrap();
        let mut input =
            api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None)
                .await;
        assert_eq!(input["db_read_results"]["etf"]["fetch_mode"], raw_mode);
        let now = Utc::now();
        attach_synthetic_host(&mut input, now);
        let output = api_server::service_status::project(&input, now).unwrap();
        if raw_mode == "synthetic" {
            assert!(service(&output, "kis-eod")["metrics"]["data_as_of"].is_null());
            assert!(
                service(&output, "kis-eod")["reason_codes"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("DATA_BLOCKED"))
            );
        } else {
            assert_eq!(
                service(&output, "kis-eod")["metrics"]["data_as_of"],
                session.to_string()
            );
        }
    }
    sqlx::query("UPDATE dataset_versions SET status='BLOCKED' WHERE id=$1")
        .bind(dataset)
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let mut blocked =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(
        blocked["db_read_results"]["etf"]["dataset_state"],
        "BLOCKED"
    );
    let now = Utc::now();
    attach_synthetic_host(&mut blocked, now);
    let response = api_server::service_status::project(&blocked, now).unwrap();
    assert!(service(&response, "kis-eod")["metrics"]["data_as_of"].is_null());
    assert_eq!(
        service(&response, "kis-eod")["metrics"]["last_success_at"],
        positive["db_read_results"]["etf"]["published_at"],
        "blocking current access does not erase the exact recorded publication success"
    );
    assert_ne!(service(&response, "kis-eod")["status"], "healthy");
    h.teardown().await;
}

#[tokio::test]
async fn actual_partial_etf_coverage_cannot_claim_complete_data() {
    use service_status_support::{attach_synthetic_host, seed_calendar_days, seed_etf_publication};
    let h = required_harness().await;
    seed_calendar_days(&h).await;
    seed_etf_publication(&h, 10).await;
    let state = h.state();
    let actor = h.owner.actor();
    let hash = actor_session_hash(&actor, &h.owner.cookie_value);
    let mut input =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(input["db_read_results"]["etf"]["covered_count"], 10);
    let now = Utc::now();
    attach_synthetic_host(&mut input, now);
    let response = api_server::service_status::project(&input, now).unwrap();
    assert_eq!(
        service(&response, "kis-eod")["metrics"]["covered_identities"],
        10
    );
    assert!(service(&response, "kis-eod")["metrics"]["data_as_of"].is_null());
    assert_ne!(service(&response, "kis-eod")["status"], "healthy");
    h.teardown().await;
}

#[tokio::test]
async fn actual_candidate_publication_and_registry_coverage_are_separate_from_etf11() {
    use service_status_support::seed_published_candidate_feed;
    let h = required_harness().await;
    seed_published_candidate_feed(&h).await;
    sqlx::query(
        "UPDATE candidate_universe_registry SET enabled=false WHERE universe_key='kosdaq150'",
    )
    .execute(&h.owner_pool)
    .await
    .unwrap();
    sqlx::query("UPDATE candidate_scheduler_control SET active=true WHERE control_key='scheduler'")
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let state = h.state();
    let actor = h.owner.actor();
    let hash = actor_session_hash(&actor, &h.owner.cookie_value);
    let positive =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    let group = &positive["db_read_results"]["candidate"];
    assert_eq!(group["enabled"], true);
    assert_eq!(group["required_universes"], 1);
    assert_eq!(group["published_universes"], 1);
    assert_eq!(group["as_of"], "2026-08-13");
    assert!(group["published_at"].is_string());
    assert_eq!(
        positive["db_read_results"]["etf"]["covered_count"],
        Value::Null,
        "candidate equities are not ETF11 coverage"
    );
    sqlx::query(
        "UPDATE candidate_universe_registry SET enabled=true WHERE universe_key='kosdaq150'",
    )
    .execute(&h.owner_pool)
    .await
    .unwrap();
    let incomplete =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(
        incomplete["db_read_results"]["candidate"]["required_universes"],
        2
    );
    assert_eq!(
        incomplete["db_read_results"]["candidate"]["published_universes"],
        1
    );
    // The new required universe cannot borrow another universe's same-day feed.
    sqlx::query("UPDATE stock_analysis_runs SET status='FAILED',published_at=NULL,error_code='FIXTURE_FAILURE' WHERE status='SUCCEEDED'")
        .execute(&h.owner_pool)
        .await
        .expect("synthetic unsuccessful published run");
    let invalid =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(
        invalid["db_read_results"]["candidate"]["published_universes"],
        0
    );
    assert!(invalid["db_read_results"]["candidate"]["as_of"].is_null());
    h.teardown().await;
}

#[tokio::test]
async fn actual_calendar_lineage_and_research_attempt_preserve_independent_failed_facts() {
    use service_status_support::{attach_synthetic_host, seed_calendar_days};
    let h = required_harness().await;
    let today = seed_calendar_days(&h).await;
    let writer = h.research_writer_pool().await;
    sqlx::query("INSERT INTO research_execution_status(id,attempt_started_at,heartbeat_at,attempt_finished_at,attempt_state,last_success_at,last_failure_at,failure_code,target_session,published_session,updated_at) VALUES('kis-eod',clock_timestamp()-interval '2 minutes',clock_timestamp()-interval '1 minute',clock_timestamp()-interval '1 second','FAILED',clock_timestamp()-interval '2 days',clock_timestamp()-interval '1 second','PIPELINE_FAILED',$1,$1-1,clock_timestamp())").bind(today).execute(&writer).await.expect("real writer-role persisted failed attempt");
    let state = h.state();
    let actor = h.owner.actor();
    let hash = actor_session_hash(&actor, &h.owner.cookie_value);
    let good =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(
        good["db_read_results"]["calendar"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "actual exact three-source joins"
    );
    assert_eq!(
        good["db_read_results"]["research"]["row"]["attempt_state"],
        "FAILED"
    );
    let stored_success = good["db_read_results"]["research"]["row"]["last_success_at"].clone();
    sqlx::query("UPDATE trading_calendars SET content_sha256=repeat('d',64) WHERE exchange='KRX' AND session_date=$1").bind(today).execute(&h.owner_pool).await.unwrap();
    let mut bad =
        api_server::repos::service_status::refresh(&state, &actor, &hash, Utc::now(), None).await;
    assert_eq!(
        bad["db_read_results"]["calendar"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "mismatched current projection does not join an unrelated immutable version"
    );
    assert_eq!(
        bad["db_read_results"]["research"]["row"]["last_success_at"],
        stored_success
    );
    let now = Utc::now();
    attach_synthetic_host(&mut bad, now);
    let response = api_server::service_status::project(&bad, now)
        .expect("independent source failure projection");
    let row = service(&response, "kis-eod");
    assert_eq!(row["status"], "failed");
    assert!(
        row["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("RUN_FAILED"))
    );
    assert!(
        service(&response, "intraday-calendar")["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("CALENDAR_UNAVAILABLE"))
    );
    assert!(
        row["metrics"]["last_success_at"].is_null(),
        "an attempt success cannot stand in for exact Curated published_at (contract source table)"
    );
    assert!(row["metrics"]["last_failure_at"].is_string());
    writer.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn actual_g1_inclusive_counts_are_adapted_once_for_live_halted_mixed_and_partial_coverage() {
    use service_status_reads_support::{
        actor_session_pool, call_stream, seed_demand_fixture, seed_live_quote_fixture,
        seed_stream_fixture,
    };
    use service_status_support::{
        attach_synthetic_host, configure_stream_window, refresh_stream_clocks, seed_calendar_days,
        seed_second_stream_identity,
    };
    let h = required_harness().await;
    let mut stream = seed_stream_fixture(&h)
        .await
        .expect("real stream source fixture");
    let state = configure_stream_window(&h, &mut stream, false).await;
    let demand = seed_demand_fixture(&h, &stream)
        .await
        .expect("authentic demanded identity");
    seed_live_quote_fixture(&h, &stream, &demand)
        .await
        .expect("authentic positive quote");
    seed_second_stream_identity(&h, &stream, &demand).await;
    seed_calendar_days(&h).await;
    let app = actor_session_pool(&h, stream.owner_id, &stream.session_hash).await;
    let leases_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_market_stream_leases WHERE owner_user_id=$1",
    )
    .bind(stream.owner_id)
    .fetch_one(&h.owner_pool)
    .await
    .unwrap();
    for (case, sql_live, sql_halted, exclusive_live, status) in [
        ("normal", 2, 0, 2, "healthy"),
        ("halted", 2, 2, 0, "waiting"),
        ("mixed", 2, 1, 1, "healthy"),
        ("partial", 1, 1, 0, "degraded"),
    ] {
        refresh_stream_clocks(&h, &stream).await;
        let mut tx = h.owner_pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(stream.owner_id.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE owner_market_stream_cache SET halted=($2='halted' OR ($2 IN ('mixed','partial') AND instrument_id='069500.KRX')) WHERE owner_user_id=$1").bind(stream.owner_id).bind(case).execute(&mut *tx).await.unwrap();
        if case == "partial" {
            sqlx::query("WITH capture AS MATERIALIZED(SELECT clock_timestamp()-interval '31 seconds' AS at) UPDATE owner_market_stream_cache SET received_at=capture.at,provider_trade_at=capture.at,committed_at=capture.at+interval '100 milliseconds',business_date=(capture.at AT TIME ZONE 'Asia/Seoul')::date,trade_time=(capture.at AT TIME ZONE 'Asia/Seoul')::time FROM capture WHERE owner_user_id=$1 AND instrument_id='069501.KRX'").bind(stream.owner_id).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();
        let raw = call_stream(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
        )
        .await
        .expect("actual app-role G1 call")
        .expect("safe bound aggregate");
        assert_eq!(
            raw.try_get::<i64, _>("demand_count").unwrap(),
            2,
            "{case}: full denominator"
        );
        assert_eq!(
            raw.try_get::<i64, _>("live_count").unwrap(),
            sql_live,
            "{case}: SQL inclusive coverage"
        );
        assert_eq!(
            raw.try_get::<i64, _>("halted_count").unwrap(),
            sql_halted,
            "{case}: halted subset"
        );
        let mut input = api_server::repos::service_status::refresh(
            &state,
            &h.owner.actor(),
            &stream.session_hash,
            Utc::now(),
            None,
        )
        .await;
        let adapted = &input["db_read_results"]["stream"];
        assert_eq!(adapted["result"], "ok");
        assert_eq!(adapted["demand_count"], 2);
        assert_eq!(
            adapted["live_count"], exclusive_live,
            "{case}: one conversion only"
        );
        assert_eq!(adapted["halted_count"], sql_halted);
        for field in [
            "producer_heartbeat_at",
            "producer_lease_expires_at",
            "oldest_received_at",
            "oldest_provider_trade_at",
            "last_received_at",
        ] {
            let raw_time: Option<DateTime<Utc>> = raw.try_get(field).unwrap();
            let adapted_time = adapted[field].as_str().map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .unwrap()
                    .with_timezone(&Utc)
            });
            assert_eq!(
                raw_time, adapted_time,
                "{case}: actual {field} is preserved"
            );
        }
        let bound = DateTime::parse_from_rfc3339(adapted["valid_until"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let observed = DateTime::parse_from_rfc3339(adapted["observed_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
        assert!(
            bound >= observed,
            "{case}: old uncovered clocks do not instantly expire fresh diagnosis"
        );
        let now = Utc::now();
        attach_synthetic_host(&mut input, now);
        let response = api_server::service_status::project(&input, now).unwrap();
        let row = service(&response, "market-stream");
        assert_eq!(
            row["status"], status,
            "{case}: positive and negative facts remain separate"
        );
        assert_eq!(row["metrics"]["active_demand"], 2);
        assert_eq!(row["metrics"]["live_identities"], exclusive_live);
        assert_eq!(row["metrics"]["halted_identities"], sql_halted);
        if case == "halted" {
            assert_eq!(row["work_status"], "closed");
            assert!(
                row["reason_codes"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("INSTRUMENT_HALTED"))
            );
        }
        if case == "partial" {
            assert_ne!(row["work_status"], "closed");
        }
    }
    refresh_stream_clocks(&h, &stream).await;
    // A halt flag with an invalid ACK cannot create halted coverage.
    sqlx::query("UPDATE owner_market_stream_subscriptions SET state='DESIRED',acked_at=NULL WHERE credential_slot_id=$1").bind(stream.slot_id).execute(&h.owner_pool).await.unwrap();
    let invalid = api_server::repos::service_status::refresh(
        &state,
        &h.owner.actor(),
        &stream.session_hash,
        Utc::now(),
        None,
    )
    .await;
    assert_eq!(invalid["db_read_results"]["stream"]["live_count"], 0);
    assert_eq!(invalid["db_read_results"]["stream"]["halted_count"], 0);
    let leases_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_market_stream_leases WHERE owner_user_id=$1",
    )
    .bind(stream.owner_id)
    .fetch_one(&h.owner_pool)
    .await
    .unwrap();
    assert_eq!(
        leases_before, leases_after,
        "actual status adapter creates no demand"
    );
    app.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn actual_closed_g1_retains_nonzero_demand_bounds_and_zero_row_binding_failure() {
    use service_status_reads_support::{
        seed_demand_fixture, seed_live_quote_fixture, seed_stream_fixture,
    };
    use service_status_support::{
        attach_synthetic_host, configure_stream_window, seed_calendar_days,
    };
    let h = required_harness().await;
    let mut stream = seed_stream_fixture(&h).await.unwrap();
    let state = configure_stream_window(&h, &mut stream, true).await;
    let demand = seed_demand_fixture(&h, &stream).await.unwrap();
    seed_live_quote_fixture(&h, &stream, &demand).await.unwrap();
    seed_calendar_days(&h).await;
    let mut tx = h.owner_pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE owner_market_stream_producers SET heartbeat_at=clock_timestamp()-interval '1 hour',lease_expires_at=clock_timestamp()-interval '1 minute',connection_state='STOPPED' WHERE credential_slot_id=$1").bind(stream.slot_id).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE owner_market_stream_leases SET lease_expires_at=clock_timestamp()+interval '10 seconds' WHERE owner_user_id=$1 AND state='ACTIVE'").bind(stream.owner_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut input = api_server::repos::service_status::refresh(
        &state,
        &h.owner.actor(),
        &stream.session_hash,
        Utc::now(),
        None,
    )
    .await;
    let aggregate = &input["db_read_results"]["stream"];
    assert_eq!(
        aggregate["owner_binding_valid"], true,
        "validated closed BOTH-NULL entry still executes real app-role binding"
    );
    assert_eq!(aggregate["demand_count"], 1);
    assert_eq!(aggregate["live_count"], 0);
    assert_eq!(aggregate["halted_count"], 0);
    assert_eq!(aggregate["lineage_valid"], false);
    let observed = DateTime::parse_from_rfc3339(aggregate["observed_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let bound = DateTime::parse_from_rfc3339(aggregate["valid_until"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        bound > observed,
        "old producer clocks are a negative/closed observation"
    );
    assert!(
        bound <= observed + chrono::Duration::seconds(10),
        "real demand expiry remains earlier than the observation ceiling"
    );
    let now = Utc::now();
    attach_synthetic_host(&mut input, now);
    let response = api_server::service_status::project(&input, now).unwrap();
    let row = service(&response, "market-stream");
    assert_eq!(row["status"], "waiting");
    assert_eq!(row["work_status"], "closed");
    assert_eq!(row["metrics"]["active_demand"], 1);
    assert_eq!(
        row["valid_until"], input["db_read_results"]["stream"]["valid_until"],
        "API closed path cannot widen the SQL bound"
    );
    let mut revoke = h.owner_pool.begin().await.unwrap();
    api_server::actor_tx::set_actor_guc(&mut revoke, &h.owner.actor())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND session_hash=$2",
    )
    .bind(stream.owner_id)
    .bind(&stream.session_hash)
    .execute(&mut *revoke)
    .await
    .unwrap();
    revoke.commit().await.unwrap();
    let mut denied = api_server::repos::service_status::refresh(
        &state,
        &h.owner.actor(),
        &stream.session_hash,
        Utc::now(),
        None,
    )
    .await;
    assert_eq!(
        denied["db_read_results"]["stream"]["owner_binding_valid"],
        false
    );
    let now = Utc::now();
    attach_synthetic_host(&mut denied, now);
    let response = api_server::service_status::project(&denied, now).unwrap();
    let row = service(&response, "market-stream");
    assert_eq!(row["status"], "failed");
    assert!(
        row["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("STREAM_BINDING_UNAVAILABLE")),
        "closed binding diagnosis: reasons={}, sources={}",
        row["reason_codes"],
        row["sources"]
    );
    for name in [
        "active_demand",
        "live_identities",
        "halted_identities",
        "last_received_at",
        "oldest_provider_event_at",
    ] {
        assert!(
            row["metrics"][name].is_null(),
            "{name}: no failed-binding disclosure even closed"
        );
    }
    h.teardown().await;
}

#[tokio::test]
async fn refresh_capacity_rejection_is_audited_and_returns_no_operational_body() {
    let h = required_harness().await;
    let state = h.state();
    let first = state.hold_service_status_refresh_for_test().unwrap();
    let second = state.hold_service_status_refresh_for_test().unwrap();
    let response = h.get(PATH, Some(&h.owner)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = common::Harness::body_json(response).await;
    assert_eq!(body["error"]["code"], "SERVICE_STATUS_UNAVAILABLE");
    assert!(body.get("services").is_none());
    assert!(body["error"]["details"].is_null());
    let audits = audit_rows_for_actor(&h, h.owner.user_id).await;
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0]
            .try_get::<Option<String>, _>("reason")
            .unwrap()
            .as_deref(),
        Some("SOURCE_UNAVAILABLE")
    );
    drop((first, second));
    h.teardown().await;
}
