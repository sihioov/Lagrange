//! HTTP contract coverage for the snapshot-pinned Owner Equity V2 chart.
//!
//! Fixtures build only immutable candidates and database admissions. They do
//! not start a worker or make any provider request.

mod common;

use auth::entitlement::Role;
use chrono::{Datelike, NaiveDate, Weekday};
use collectors::owner_equity_v2::artifact::{
    OwnerEquityArtifactInput, VerifiedOwnerEquityArtifact, write_owner_equity_artifact,
};
use common::{Harness, UserCtx, sha256_hex, status};
use domain::{BatchId, CodeCommit, ContentHash, InstrumentId, TradingDate};
use market_data::owner_equity_v2::{
    OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OWNER_ONLY_WARNING,
    OwnerEquityBar, OwnerEquityCaptureKind, OwnerEquityGenerationCandidate, OwnerEquitySourcePins,
    PRICE_SEMANTICS, RESEARCH_ONLY_WARNING, STRICT_PIT_WARNING, VENDOR_SNAPSHOT_WARNING,
};
use std::path::Path;
use uuid::Uuid;

const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_SHA256: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const INSTRUMENT_ID: &str = "005930.KRX";

#[derive(Debug, Clone, Copy)]
enum LineageMismatch {
    Owner,
    Membership,
    Generation,
    Instrument,
    RawManifest,
    Entitlement,
    CaptureCommit,
    MaterializerCommit,
}

fn aug_13() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 13).expect("fixture date")
}

fn aug_14() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 14).expect("fixture date")
}

fn session_closed() -> bool {
    true
}

fn business_dates_through(as_of: NaiveDate, count: usize) -> Vec<NaiveDate> {
    let mut dates = Vec::with_capacity(count);
    let mut date = as_of;
    while dates.len() < count {
        if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            dates.push(date);
        }
        date = date.pred_opt().expect("fixture dates stay representable");
    }
    dates.reverse();
    dates
}

fn candidate(as_of: NaiveDate) -> OwnerEquityGenerationCandidate {
    let dates = business_dates_through(as_of, 261);
    let bars = dates
        .iter()
        .enumerate()
        .map(|(index, date)| {
            let close = u64::try_from(10_000 + index).expect("fixture close");
            OwnerEquityBar {
                session_date: TradingDate::new(date.year(), date.month(), date.day())
                    .expect("fixture trading date"),
                open: close - 2,
                high: close + 4,
                low: close - 4,
                close,
                volume: 100_000 + u64::try_from(index).expect("fixture volume"),
            }
        })
        .collect::<Vec<_>>();
    let commit = CodeCommit::parse(CODE_COMMIT).expect("fixture commit");
    OwnerEquityGenerationCandidate {
        candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
        contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
        capture_kind: OwnerEquityCaptureKind::Initial,
        instrument_id: InstrumentId::parse(INSTRUMENT_ID).expect("fixture instrument"),
        display_name: Some("fixture".to_owned()),
        requested_start: bars.first().expect("fixture bars").session_date,
        requested_end: bars.last().expect("fixture bars").session_date,
        target_observed_sessions: 261,
        minimum_observed_sessions: 121,
        observed_sessions: u32::try_from(bars.len()).expect("fixture count"),
        first_observed_date: bars.first().expect("fixture bars").session_date,
        last_observed_date: bars.last().expect("fixture bars").session_date,
        bars,
        source_pins: OwnerEquitySourcePins {
            capture_identity_sha256: ContentHash::from_bytes(b"chart-capture-identity"),
            raw_batch_id: BatchId::from_uuid(Uuid::new_v4()),
            raw_manifest_sha256: ContentHash::from_bytes(b"chart-raw-manifest"),
            batch_json_sha256: ContentHash::from_bytes(b"chart-batch-json"),
            entitlement_reference: "chart-fixture-entitlement".to_owned(),
            entitlement_sha256: ContentHash::parse(ENTITLEMENT_SHA256)
                .expect("fixture entitlement hash"),
            capture_code_commit: commit.clone(),
            materializer_code_commit: commit,
            prior_candidate_sha256: None,
            prior_artifact_manifest_sha256: None,
            files: Vec::new(),
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
        claims_not_made: Vec::new(),
    }
}

fn write_candidate_artifact(
    root: &Path,
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation: u64,
    candidate: &OwnerEquityGenerationCandidate,
) -> VerifiedOwnerEquityArtifact {
    write_owner_equity_artifact(
        root,
        OwnerEquityArtifactInput {
            owner_user_id,
            membership_id,
            generation,
            candidate,
        },
    )
    .expect("fixture artifact writes")
}

async fn seed_chart_lineage(
    harness: &Harness,
    owner: &UserCtx,
    candidate: &OwnerEquityGenerationCandidate,
    artifact_manifest_sha256: &ContentHash,
) -> Uuid {
    seed_chart_lineage_with_identity(
        harness,
        owner,
        candidate,
        artifact_manifest_sha256,
        Uuid::new_v4(),
        Uuid::new_v4(),
    )
    .await
}

async fn seed_chart_lineage_with_identity(
    harness: &Harness,
    owner: &UserCtx,
    candidate: &OwnerEquityGenerationCandidate,
    artifact_manifest_sha256: &ContentHash,
    membership_id: Uuid,
    generation_id: Uuid,
) -> Uuid {
    let snapshot_id = Uuid::new_v4();
    let instrument_id = candidate.instrument_id.to_string();
    let as_of = candidate.last_observed_date.to_iso();
    let source = &candidate.source_pins;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_memberships \
                 (id, owner_user_id, instrument_id, transition_actor_user_id, \
                  transition_code_commit, transition_entitlement_sha256) \
                 VALUES ('{membership_id}', '{}', '{instrument_id}', '{}', '{CODE_COMMIT}', '{ENTITLEMENT_SHA256}')",
                owner.user_id, owner.user_id
            ),
        )
        .await;
    for state in ["VALIDATING", "BACKFILLING"] {
        harness
            .seed_migration_owner(
                owner,
                &format!(
                    "UPDATE owner_equity_memberships SET state = '{state}', updated_at = now() \
                     WHERE id = '{membership_id}'"
                ),
            )
            .await;
    }
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_instrument_generations \
                 (id, membership_id, owner_user_id, instrument_id, generation, \
                  target_observed_sessions, minimum_observed_sessions, observed_sessions, \
                  first_session, last_session) \
                 VALUES ('{generation_id}', '{membership_id}', '{}', '{instrument_id}', 1, \
                  {}, {}, {}, '{}', '{}')",
                owner.user_id,
                candidate.target_observed_sessions,
                candidate.minimum_observed_sessions,
                candidate.observed_sessions,
                candidate.first_observed_date.to_iso(),
                candidate.last_observed_date.to_iso(),
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_generation_admissions \
                 (generation_id, owner_user_id, membership_id, instrument_id, generation, \
                  raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256, \
                  capture_code_commit, materializer_code_commit) \
                 VALUES ('{generation_id}', '{}', '{membership_id}', '{instrument_id}', 1, \
                  '{}', '{}', '{}', '{}', '{}')",
                owner.user_id,
                source.raw_manifest_sha256.as_str(),
                artifact_manifest_sha256.as_str(),
                source.entitlement_sha256.as_str(),
                source.capture_code_commit.as_str(),
                source.materializer_code_commit.as_str(),
            ),
        )
        .await;
    for state in ["MATERIALIZING", "READY"] {
        harness
            .seed_migration_owner(
                owner,
                &format!(
                    "UPDATE owner_equity_memberships SET state = '{state}', updated_at = now() \
                     WHERE id = '{membership_id}'"
                ),
            )
            .await;
    }
    let universe_sha256 = format!("sha256:{}", sha256_hex(instrument_id.as_bytes()));
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_signal_snapshots \
                 (id, owner_user_id, as_of_session, universe_sha256, row_count, signal_code_commit) \
                 VALUES ('{snapshot_id}', '{}', '{as_of}', '{universe_sha256}', 1, '{CODE_COMMIT}')",
                owner.user_id
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_signal_snapshot_rows \
                 (snapshot_id, owner_user_id, instrument_id, membership_id, generation_id, generation, rank, signals_json) \
                 VALUES ('{snapshot_id}', '{}', '{instrument_id}', '{membership_id}', '{generation_id}', 1, 1, '{{}}'::jsonb)",
                owner.user_id
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_equity_signal_snapshots SET published_at = now() WHERE id = '{snapshot_id}'"
            ),
        )
        .await;
    snapshot_id
}

async fn seed_confirmed_close(harness: &Harness, date: NaiveDate) {
    let batch_id = Uuid::new_v4();
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendars \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '{}', 'TRADING', 'Asia/Seoul', 'fixture', 'chart', \
              '{batch_id}', repeat('a', 64), now())",
            date.format("%F")
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO data_batches \
             (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, \
              retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX', 'KR', '{}', 'EOD', 'raw/chart-fixture', repeat('b', 64), 1, \
              now(), '{batch_id}', 'bars.json', 'credentialed')",
            date.format("%F")
        ))
        .await;
}

fn chart_path(snapshot_id: Uuid, range: &str) -> String {
    chart_path_for(snapshot_id, INSTRUMENT_ID, range)
}

fn chart_path_for(snapshot_id: Uuid, instrument_id: &str, range: &str) -> String {
    format!(
        "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/{instrument_id}/chart?snapshot_id={snapshot_id}&range={range}"
    )
}

async fn required_harness() -> Harness {
    Harness::new().await.unwrap_or_else(|| {
        panic!(
            "Owner Equity V2 chart HTTP acceptance tests require DATABASE_URL; start the isolated QA DB with `docker compose -f deploy/qa/qa-db.compose.yml up -d --wait` or run the workspace-tests CI workflow job"
        )
    })
}

async fn assert_lineage_mismatch(harness: &Harness, sequence: usize, mismatch: LineageMismatch) {
    let owner = harness
        .seed_user(
            Role::Owner,
            &format!("chart-lineage-{sequence}@lagrange.test"),
            "chart-lineage",
            &format!("owner-{sequence}"),
        )
        .await;
    let admitted_candidate = candidate(aug_13());
    let membership_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let mut artifact_candidate = admitted_candidate.clone();
    let mut artifact_owner = owner.user_id;
    let mut artifact_membership = membership_id;
    let mut artifact_generation = 1;
    match mismatch {
        LineageMismatch::Owner => artifact_owner = Uuid::new_v4(),
        LineageMismatch::Membership => artifact_membership = Uuid::new_v4(),
        LineageMismatch::Generation => artifact_generation = 2,
        LineageMismatch::Instrument => {
            artifact_candidate.instrument_id =
                InstrumentId::parse("000660.KRX").expect("fixture instrument");
        }
        LineageMismatch::RawManifest => {
            artifact_candidate.source_pins.raw_manifest_sha256 =
                ContentHash::from_bytes(b"mismatched raw manifest");
        }
        LineageMismatch::Entitlement => {
            artifact_candidate.source_pins.entitlement_sha256 =
                ContentHash::from_bytes(b"mismatched entitlement");
        }
        LineageMismatch::CaptureCommit => {
            artifact_candidate.source_pins.capture_code_commit =
                CodeCommit::parse("1111111111111111111111111111111111111111")
                    .expect("fixture capture commit");
        }
        LineageMismatch::MaterializerCommit => {
            artifact_candidate.source_pins.materializer_code_commit =
                CodeCommit::parse("2222222222222222222222222222222222222222")
                    .expect("fixture materializer commit");
        }
    }
    let artifact = write_candidate_artifact(
        &harness.artifact_root,
        artifact_owner,
        artifact_membership,
        artifact_generation,
        &artifact_candidate,
    );
    let snapshot_id = seed_chart_lineage_with_identity(
        harness,
        &owner,
        &admitted_candidate,
        &artifact.manifest_sha256,
        membership_id,
        generation_id,
    )
    .await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&owner))
        .await;
    assert_eq!(
        status(&response),
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "{mismatch:?} must fail closed"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = Harness::body_json(response).await;
    assert_eq!(
        Harness::error_code(&body),
        "OWNER_EQUITY_INTEGRITY_FAILED",
        "{mismatch:?} must not be exposed"
    );
    let serialized = body.to_string();
    assert!(!serialized.contains("sha256:"));
    assert!(!serialized.contains("owner-equity-v2"));
}

#[tokio::test]
async fn chart_enforces_owner_and_exact_query_contract_before_artifact_io() {
    let mut harness = required_harness().await;
    let valid_snapshot = Uuid::new_v4();
    let base = format!(
        "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/{INSTRUMENT_ID}/chart?snapshot_id={valid_snapshot}&range=1m"
    );
    for (path, expected) in [
        (base.as_str(), "FORBIDDEN"),
        (
            "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/not-an-instrument/chart?snapshot_id=00000000-0000-0000-0000-000000000000&range=1m",
            "INVALID_PARAMETER",
        ),
        (
            "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=nope&range=1m",
            "INVALID_PARAMETER",
        ),
        (
            "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=00000000-0000-0000-0000-000000000000&range=5m",
            "INVALID_PARAMETER",
        ),
        (
            "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=00000000-0000-0000-0000-000000000000&range=1m&unexpected=x",
            "INVALID_PARAMETER",
        ),
    ] {
        let user = if expected == "FORBIDDEN" {
            Some(&harness.member)
        } else {
            Some(&harness.owner)
        };
        let response = harness.get(path, user).await;
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            Harness::error_code(&Harness::body_json(response).await),
            expected
        );
    }
    let anonymous = harness.get(&base, None).await;
    assert_eq!(status(&anonymous), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        Harness::error_code(&Harness::body_json(anonymous).await),
        "SESSION_UNKNOWN"
    );

    let candidate = candidate(aug_13());
    let missing = ContentHash::from_bytes(b"absent chart artifact");
    let snapshot_id = seed_chart_lineage(&harness, &harness.owner, &candidate, &missing).await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    assert_eq!(
        status(&response),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "OWNER_EQUITY_CHART_UNAVAILABLE"
    );

    harness
        .restart_api_with_owner_equity_v2_artifact_root(Some(harness.artifact_root.clone()))
        .await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "OWNER_EQUITY_CHART_UNAVAILABLE"
    );

    let response = harness
        .get(&chart_path(Uuid::new_v4(), "1m"), Some(&harness.owner))
        .await;
    assert_eq!(status(&response), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "RESOURCE_NOT_FOUND"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn chart_rejects_every_artifact_visible_lineage_mismatch() {
    let mut harness = required_harness().await;
    harness
        .restart_api_with_owner_equity_v2_artifact_root(Some(harness.artifact_root.clone()))
        .await;
    for (sequence, mismatch) in [
        LineageMismatch::Owner,
        LineageMismatch::Membership,
        LineageMismatch::Generation,
        LineageMismatch::Instrument,
        LineageMismatch::RawManifest,
        LineageMismatch::Entitlement,
        LineageMismatch::CaptureCommit,
        LineageMismatch::MaterializerCommit,
    ]
    .into_iter()
    .enumerate()
    {
        assert_lineage_mismatch(&harness, sequence, mismatch).await;
    }
    harness.teardown().await;
}

#[tokio::test]
async fn chart_returns_only_verified_snapshot_pinned_eod_projection() {
    let mut harness = required_harness().await;
    let candidate = candidate(aug_13());
    let membership_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let artifact = write_candidate_artifact(
        &harness.artifact_root,
        harness.owner.user_id,
        membership_id,
        1,
        &candidate,
    );
    let snapshot_id = seed_chart_lineage_with_identity(
        &harness,
        &harness.owner,
        &candidate,
        &artifact.manifest_sha256,
        membership_id,
        generation_id,
    )
    .await;
    harness
        .restart_api_with_owner_equity_v2_artifact_root(Some(harness.artifact_root.clone()))
        .await;

    let response = harness
        .get(
            &chart_path_for(snapshot_id, "000660.KRX", "1m"),
            Some(&harness.owner),
        )
        .await;
    assert_eq!(status(&response), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "RESOURCE_NOT_FOUND"
    );

    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    assert_eq!(status(&response), axum::http::StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = Harness::body_json(response).await;
    assert_eq!(body["snapshot_id"], snapshot_id.to_string());
    assert_eq!(body["instrument_id"], INSTRUMENT_ID);
    assert_eq!(body["generation"], 1);
    assert_eq!(body["range"], "1m");
    assert_eq!(body["as_of"], "2026-08-13");
    assert_eq!(body["freshness"], "UNVERIFIABLE");
    assert!(body["expected_as_of"].is_null());
    assert_eq!(body["price_semantics"], "ORIGINAL_UNADJUSTED");
    assert_eq!(
        body["warnings"],
        serde_json::json!([
            "NOT_REALTIME",
            "CORPORATE_ACTIONS_NOT_ADJUSTED",
            "RESEARCH_ONLY"
        ])
    );
    assert_eq!(body["latest"]["session_date"], "2026-08-13");
    assert_eq!(body["latest"]["change"], 1);
    assert!(
        body["latest"]["change_rate"]
            .as_f64()
            .expect("ratio")
            .is_sign_positive()
    );
    let bars = body["bars"].as_array().expect("bars array");
    assert!(!bars.is_empty() && bars.len() < 60);
    assert!(bars.iter().all(|bar| bar["sma_20"].is_number()));
    assert!(bars.iter().all(|bar| bar["sma_60"].is_number()));
    let serialized = body.to_string();
    for forbidden in [
        "sha256:",
        "artifact_manifest",
        "raw_manifest",
        "entitlement",
        "capture_code_commit",
        "materializer_code_commit",
        "owner_user_id",
        "membership_id",
        "generation_id",
        "path",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "response leaked {forbidden}"
        );
    }

    for range in ["3m", "6m", "1y"] {
        let response = harness
            .get(&chart_path(snapshot_id, range), Some(&harness.owner))
            .await;
        assert_eq!(status(&response), axum::http::StatusCode::OK);
        let body = Harness::body_json(response).await;
        let bars = body["bars"].as_array().expect("bars array");
        assert!(!bars.is_empty() && bars.len() <= 261);
        assert_eq!(bars.last().expect("last bar")["session_date"], "2026-08-13");
    }

    seed_confirmed_close(&harness, aug_13()).await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    let body = Harness::body_json(response).await;
    assert_eq!(body["freshness"], "CURRENT");
    assert_eq!(body["expected_as_of"], "2026-08-13");

    seed_confirmed_close(&harness, aug_14()).await;
    harness
        .restart_api_with_candidate_clock(aug_14, session_closed)
        .await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    let body = Harness::body_json(response).await;
    assert_eq!(body["freshness"], "STALE");
    assert_eq!(body["expected_as_of"], "2026-08-14");

    let other_owner = harness
        .seed_user(
            Role::Owner,
            "other-chart@lagrange.test",
            "chart",
            "other-owner",
        )
        .await;
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&other_owner))
        .await;
    assert_eq!(status(&response), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "RESOURCE_NOT_FOUND"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn chart_fails_closed_for_tampered_artifacts() {
    let mut harness = required_harness().await;
    let candidate = candidate(aug_13());
    let membership_id = Uuid::new_v4();
    let artifact = write_candidate_artifact(
        &harness.artifact_root,
        harness.owner.user_id,
        membership_id,
        1,
        &candidate,
    );
    let snapshot_id = seed_chart_lineage_with_identity(
        &harness,
        &harness.owner,
        &candidate,
        &artifact.manifest_sha256,
        membership_id,
        Uuid::new_v4(),
    )
    .await;
    harness
        .restart_api_with_owner_equity_v2_artifact_root(Some(harness.artifact_root.clone()))
        .await;
    let candidate_path = harness
        .artifact_root
        .join("owner-equity-v2")
        .join(
            artifact
                .manifest_sha256
                .as_str()
                .trim_start_matches("sha256:"),
        )
        .join("candidate.json");
    std::fs::write(&candidate_path, b"{}")
        .expect("fixture may only tamper its own immutable artifact copy");
    let response = harness
        .get(&chart_path(snapshot_id, "1m"), Some(&harness.owner))
        .await;
    assert_eq!(
        status(&response),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let body = Harness::body_json(response).await;
    assert_eq!(Harness::error_code(&body), "OWNER_EQUITY_INTEGRITY_FAILED");
    assert!(!body.to_string().contains("candidate.json"));
    harness.teardown().await;
}
