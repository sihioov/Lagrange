#[path = "../../../tests/support/candidate_rolling_provider.rs"]
#[allow(dead_code)]
mod candidate_rolling_provider;
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use collectors::{
    CandidateInstrumentCatalog, CandidatePricePublication, CandidateSourcePublication,
    PostgresCandidateSourceSink, PublishOutcome, candidate_raw_manifest_sha256,
    prepare_candidate_batch, publish_candidate_batch,
};
use common::ScratchDb;
use domain::{ContentHash, DatasetId, InstrumentId, TradingDate, UtcTimestamp};
use job_queue::candidate::{
    CandidateOutcome, CandidateRunnerConfig, CandidateRunnerPaths, CandidateScheduleError,
    CandidateScheduleRequest, DatasetSchedulePin, run_once, schedule_candidate_run,
    schedule_latest_candidate_run,
};
use job_queue::{JobQueue, QueueConfig};
use market_data::curate::schema::{read_adjusted_bars, read_bars, write_adjusted_bars, write_bars};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CandidateDocument, CandidateSourcePin, Capability, CurateRequest,
    CurateStore, DatasetManifest, FinancialPeriodKind, FundamentalDocument, FundamentalObservation,
    FundamentalProfile, IndexMembershipDocument, IndexMembershipObservation, IngestRequest,
    InvestorClass, InvestorFlowDocument, InvestorFlowObservation, MARKET_KR, MarketStatusDocument,
    MarketStatusObservation, RawStore, SectorDocument, SectorObservation, StatementScope,
    curate_batch, curation_inputs_from_raw, dataset_manifest_hash, ingest_bundle,
    ingest_bundle_with_kinds, price_curation_evidence,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use tempfile::TempDir;
use uuid::Uuid;

use candidate_rolling_provider::{ROLLING_MEMBERS, RollingCandidateProvider};

const AS_OF: &str = "2021-01-29";
const CUTOFF: &str = "2021-01-29T07:30:00Z";
const RETRIEVED: &str = "2021-01-29T08:00:00Z";
const CANDIDATE_ENTITLEMENT_ID: Uuid = Uuid::from_u128(0x00000000000040008000000000000043);
const CANDIDATE_LICENSE_REF: &str = "fixture://candidate-entitlement";
const MEMBERS: [&str; 8] = [
    "100001.KRX",
    "100002.KRX",
    "100003.KRX",
    "100004.KRX",
    "100005.KRX",
    "100006.KRX",
    "100007.KRX",
    "100008.KRX",
];
const SOURCE_SYMBOLS: [&str; 3] = ["069500.KRX", "229200.KRX", "114260.KRX"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create cloned symbol directory");
    for entry in std::fs::read_dir(from).expect("read source symbol directory") {
        let entry = entry.expect("source entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("clone curated component");
        }
    }
}

fn clone_symbol(store_root: &Path, source: &str, destination: &str) {
    let market = store_root.join("curated/bars/market=kr");
    let target = market.join(format!("symbol={destination}"));
    copy_dir(&market.join(format!("symbol={source}")), &target);
    let store = CurateStore::new(store_root);
    let destination_id = InstrumentId::parse(destination).expect("candidate instrument");
    for year in std::fs::read_dir(&target).expect("read cloned years") {
        let year = year.expect("year entry");
        let Some(year) = year
            .file_name()
            .to_string_lossy()
            .strip_prefix("year=")
            .and_then(|value| value.parse::<i32>().ok())
        else {
            continue;
        };
        let bars_path = store.bars_path("kr", destination, year, 2);
        if bars_path.is_file() {
            let mut rows = read_bars(&bars_path).expect("read cloned raw bars");
            rows.iter_mut()
                .for_each(|row| row.instrument_id = destination_id.clone());
            write_bars(&bars_path, &rows).expect("write cloned raw bars");
        }
        for adjusted in [
            store.adjusted_bars_path("kr", destination, year, 2),
            store.total_return_bars_path("kr", destination, year, 2),
        ] {
            if adjusted.is_file() {
                let mut rows = read_adjusted_bars(&adjusted).expect("read cloned adjusted bars");
                rows.iter_mut()
                    .for_each(|row| row.instrument_id = destination_id.clone());
                write_adjusted_bars(&adjusted, &rows).expect("write cloned adjusted bars");
            }
        }
    }
}

fn trim_symbol_to_as_of(store_root: &Path, symbol: &str) {
    let store = CurateStore::new(store_root);
    for year in [2020, 2021] {
        let bars_path = store.bars_path("kr", symbol, year, 2);
        if bars_path.is_file() {
            let mut rows = read_bars(&bars_path).expect("read newly listed bars");
            rows.retain(|row| row.trading_date.to_iso() == AS_OF);
            write_bars(&bars_path, &rows).expect("write newly listed bars");
        }
        for adjusted in [
            store.adjusted_bars_path("kr", symbol, year, 2),
            store.total_return_bars_path("kr", symbol, year, 2),
        ] {
            if adjusted.is_file() {
                let mut rows = read_adjusted_bars(&adjusted).expect("read newly listed returns");
                rows.retain(|row| row.trading_date.to_iso() == AS_OF);
                write_adjusted_bars(&adjusted, &rows).expect("write newly listed returns");
            }
        }
    }
}

struct CandidateDataset {
    _temp: TempDir,
    root: PathBuf,
    manifest_sha256: String,
    sessions: Vec<TradingDate>,
}

fn candidate_dataset() -> CandidateDataset {
    let repo = repo_root();
    let temp = tempfile::Builder::new()
        .prefix(".candidate-runner-qa-")
        .tempdir_in(&repo)
        .expect("candidate fixture directory");
    let generated = temp.path().join("phase0");
    let output = Command::new(std::env::var_os("PYTHON").unwrap_or_else(|| "python".into()))
        .current_dir(&repo)
        .arg(repo.join("scripts/ci/prepare_phase0.py"))
        .arg("--root")
        .arg(&generated)
        .output()
        .expect("run phase0 generator");
    assert!(
        output.status.success(),
        "phase0 generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = generated.join("curated");
    for (index, member) in MEMBERS.iter().enumerate() {
        clone_symbol(&root, SOURCE_SYMBOLS[index % SOURCE_SYMBOLS.len()], member);
    }
    trim_symbol_to_as_of(&root, MEMBERS[MEMBERS.len() - 1]);
    let store = CurateStore::new(&root);
    let mut sessions = Vec::new();
    for year in [2020, 2021] {
        let path = store.bars_path("kr", MEMBERS[0], year, 2);
        if path.is_file() {
            sessions.extend(
                read_bars(&path)
                    .expect("candidate bars are readable")
                    .into_iter()
                    .map(|row| row.trading_date),
            );
        }
    }
    sessions.sort_unstable();
    sessions.retain(|date| date.to_iso().as_str() <= AS_OF);
    assert!(sessions.len() >= 60, "candidate fixture needs 60 sessions");
    let manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").expect("dataset id"),
        version: 2,
        capability: Capability::PriceReturnOnly,
        created_at: timestamp(RETRIEVED),
        source_batches: Vec::new(),
        bar_count: u64::try_from(MEMBERS.len() * 260).expect("fixture bar count fits u64"),
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"candidate-placeholder"),
    };
    let manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&manifest).expect("manifest hash"),
        ..manifest
    };
    store
        .write_dataset_manifest(&manifest)
        .expect("write candidate manifest");
    let manifest_sha256 = manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .expect("sha256 prefix")
        .to_owned();
    CandidateDataset {
        _temp: temp,
        root,
        manifest_sha256,
        sessions,
    }
}

fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).expect("valid fixture date")
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(value).expect("valid fixture timestamp")
}

fn cutoff() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(CUTOFF)
        .expect("valid cutoff")
        .with_timezone(&Utc)
}

fn pin(dataset_id: &str, version: &str, hash: &str) -> CandidateSourcePin {
    CandidateSourcePin {
        provider: "krx".into(),
        entitlement_id: CANDIDATE_ENTITLEMENT_ID,
        license_ref: CANDIDATE_LICENSE_REF.into(),
        dataset_id: dataset_id.into(),
        dataset_version: version.into(),
        manifest_sha256: hash.into(),
        retrieved_at: timestamp(RETRIEVED),
    }
}

async fn insert_dataset(
    pool: &sqlx::PgPool,
    dataset_id: &str,
    version: &str,
    hash: &str,
    storage_path: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO dataset_versions
         (id, dataset_id, version, status, manifest_sha256, storage_path)
         VALUES (gen_random_uuid(), $1, $2, 'READY', $3, $4) RETURNING id",
    )
    .bind(dataset_id)
    .bind(version)
    .bind(hash)
    .bind(storage_path)
    .fetch_one(pool)
    .await
    .expect("insert candidate dataset version")
}

#[tokio::test]
async fn synthetic_sources_schedule_compute_and_publish_one_atomic_top_five() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL is not set");
        return;
    };
    let dataset = candidate_dataset();
    for member in MEMBERS {
        sqlx::query(
            "INSERT INTO instruments
             (id, symbol, venue, currency, name, asset_class, status, listed_at)
             VALUES ($1,$2,'KRX','KRW',$3,'EQUITY','ACTIVE',DATE '2010-01-01')",
        )
        .bind(member)
        .bind(member.trim_end_matches(".KRX"))
        .bind(format!("Synthetic {member}"))
        .execute(&db.pool)
        .await
        .expect("insert candidate instrument");
    }

    let price_id = insert_dataset(
        &db.pool,
        "krx_eod_bars",
        "qa-v2",
        &dataset.manifest_sha256,
        dataset.root.to_string_lossy().as_ref(),
    )
    .await;
    let status_hash = "b".repeat(64);
    let flow_hash = "c".repeat(64);
    let fundamental_hash = "d".repeat(64);
    let universe_hash = "e".repeat(64);
    let sector_hash = "f".repeat(64);
    let status_id = insert_dataset(
        &db.pool,
        "krx_market_status",
        "fixture-status-v1",
        &status_hash,
        "db://candidate/status",
    )
    .await;
    let flow_id = insert_dataset(
        &db.pool,
        "krx_investor_flows",
        "fixture-flow-v1",
        &flow_hash,
        "db://candidate/flow",
    )
    .await;
    let fundamental_id = insert_dataset(
        &db.pool,
        "krx_fundamentals",
        "fixture-fundamental-v1",
        &fundamental_hash,
        "db://candidate/fundamental",
    )
    .await;
    let universe_dataset_id = insert_dataset(
        &db.pool,
        "krx_kospi200_membership",
        "fixture-universe-v1",
        &universe_hash,
        "db://candidate/universe",
    )
    .await;
    let sector_dataset_id = insert_dataset(
        &db.pool,
        "krx_sector_classification",
        "fixture-sector-v1",
        &sector_hash,
        "db://candidate/sector",
    )
    .await;

    let publisher_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db.role_url("research_writer"))
        .await
        .expect("connect as research_writer");
    let sink = PostgresCandidateSourceSink::new(publisher_pool.clone());
    let as_of = date(AS_OF);
    let cutoff_at = timestamp(CUTOFF);

    sqlx::query(
        "INSERT INTO data_entitlements
         (id, contract_document_sha256, contract_reference, status, covered_datasets,
          covered_uses, effective_from, effective_until, managed_by)
         VALUES ($1,repeat('8',64),$2,'ACTIVE',$3,
                 '[\"candidate\"]'::jsonb,DATE '2020-01-01',DATE '2030-12-31',
                 '00000000-0000-4000-8000-000000000042'::uuid)",
    )
    .bind(CANDIDATE_ENTITLEMENT_ID)
    .bind(CANDIDATE_LICENSE_REF)
    .bind(json!([
        "krx_eod_bars",
        "krx_market_status",
        "krx_investor_flows",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .execute(&db.pool)
    .await
    .expect("insert candidate entitlement before source publication");

    sqlx::query(
        "INSERT INTO candidate_price_publications
         (dataset_version_id, dataset_version, manifest_sha256, market,
          curated_generation, first_session, last_session, provider,
          entitlement_id, license_ref, source_revision, available_at, retrieved_at)
         VALUES ($1,'qa-v2',$2,'kr',2,$3,$4,'synthetic',$5,$6,
                 'fixture-price-1',$7,$8)",
    )
    .bind(price_id)
    .bind(&dataset.manifest_sha256)
    .bind(
        dataset
            .sessions
            .first()
            .expect("candidate fixture first session")
            .as_naive_date(),
    )
    .bind(
        dataset
            .sessions
            .last()
            .expect("candidate fixture last session")
            .as_naive_date(),
    )
    .bind(CANDIDATE_ENTITLEMENT_ID)
    .bind(CANDIDATE_LICENSE_REF)
    .bind(cutoff())
    .bind(
        DateTime::parse_from_rfc3339(RETRIEVED)
            .expect("valid price retrieval instant")
            .with_timezone(&Utc),
    )
    .execute(&db.pool)
    .await
    .expect("publish exact candidate price readiness");
    let coverage_sessions = &dataset.sessions[dataset.sessions.len() - 60..];
    for member in MEMBERS {
        let member_sessions = if member == MEMBERS[MEMBERS.len() - 1] {
            &coverage_sessions[59..]
        } else {
            coverage_sessions
        };
        sqlx::query(
            "INSERT INTO candidate_price_instrument_coverage
             (dataset_version_id,instrument_id,first_session,last_session,session_count)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(price_id)
        .bind(member)
        .bind(member_sessions[0].as_naive_date())
        .bind(member_sessions[member_sessions.len() - 1].as_naive_date())
        .bind(i32::try_from(member_sessions.len()).unwrap())
        .execute(&db.pool)
        .await
        .expect("publish candidate instrument coverage");
        for session in member_sessions {
            sqlx::query(
                "INSERT INTO candidate_price_instrument_sessions
                 (dataset_version_id,instrument_id,session_date) VALUES ($1,$2,$3)",
            )
            .bind(price_id)
            .bind(member)
            .bind(session.as_naive_date())
            .execute(&db.pool)
            .await
            .expect("publish exact candidate price session");
        }
    }

    let memberships = CandidateDocument::IndexMembership(IndexMembershipDocument {
        memberships: MEMBERS
            .iter()
            .map(|member| IndexMembershipObservation {
                index_id: "kospi200".into(),
                instrument: InstrumentId::parse(member).unwrap(),
                announced_at: timestamp("2019-12-01T00:00:00Z"),
                effective_from: date("2020-01-01"),
                effective_until: None,
                available_at: timestamp("2019-12-01T00:01:00Z"),
                source_revision: "fixture-membership-1".into(),
            })
            .collect(),
    });
    let universe_pin = pin(
        "krx_kospi200_membership",
        "fixture-universe-v1",
        &universe_hash,
    );
    let sectors = CandidateDocument::SectorClassification(SectorDocument {
        sectors: MEMBERS
            .iter()
            .map(|member| SectorObservation {
                taxonomy_id: "krx-sector".into(),
                taxonomy_version: "fixture-2021".into(),
                instrument: InstrumentId::parse(member).unwrap(),
                sector_code: "TECH".into(),
                sector_name: "Synthetic technology".into(),
                fundamental_profile: FundamentalProfile::NonFinancial,
                source_revision: "fixture-sector-1".into(),
                effective_from: date("2020-01-01"),
                effective_until: None,
                available_at: timestamp("2019-12-01T00:01:00Z"),
            })
            .collect(),
    });
    let sector_pin = pin(
        "krx_sector_classification",
        "fixture-sector-v1",
        &sector_hash,
    );
    let statuses = CandidateDocument::MarketStatus(MarketStatusDocument {
        statuses: MEMBERS
            .iter()
            .map(|member| MarketStatusObservation {
                instrument: InstrumentId::parse(member).unwrap(),
                trade_date: as_of,
                suspended: false,
                administrative: false,
                liquidation: false,
                inactive: false,
                disqualifying_audit_opinion: false,
                complete_capital_impairment: false,
                source_revision: "fixture-status-1".into(),
                available_at: timestamp("2021-01-29T07:00:00Z"),
            })
            .collect(),
    });
    let status_pin = pin("krx_market_status", "fixture-status-v1", &status_hash);
    let recent_sessions = &dataset.sessions[dataset.sessions.len() - 60..];
    let mut flows = Vec::with_capacity(MEMBERS.len() * recent_sessions.len() * 2);
    for (instrument_index, member) in MEMBERS.iter().enumerate() {
        for (session_index, session) in recent_sessions.iter().enumerate() {
            let rank = (instrument_index + 1) as f64;
            let trend = (session_index + 1) as f64;
            let available_at = timestamp(&format!("{}T07:00:00Z", session.to_iso()));
            for (investor_class, multiplier) in [
                (InvestorClass::Foreign, 1.0),
                (InvestorClass::Institution, 0.7),
            ] {
                flows.push(InvestorFlowObservation {
                    instrument: InstrumentId::parse(member).unwrap(),
                    trade_date: *session,
                    investor_class,
                    net_amount: rank * trend * 1_000_000.0 * multiplier,
                    net_volume: rank * trend * 10.0 * multiplier,
                    currency: "KRW".into(),
                    volume_unit: "SHARE".into(),
                    source_revision: "fixture-flow-1".into(),
                    available_at,
                });
            }
        }
    }
    let flow_document = CandidateDocument::InvestorFlow(InvestorFlowDocument { flows });
    let flow_pin = pin("krx_investor_flows", "fixture-flow-v1", &flow_hash);
    let mut fundamentals = Vec::with_capacity(MEMBERS.len() * 6);
    for (index, member) in MEMBERS.iter().enumerate() {
        let rank = (index + 1) as f64;
        for (metric, value) in [
            ("revenue_growth", rank * 0.03),
            ("operating_margin", 0.05 + rank * 0.01),
            ("roe", 0.04 + rank * 0.015),
            ("debt_ratio", 2.0 - rank * 0.12),
            ("cash_conversion", 0.4 + rank * 0.05),
            ("earnings_yield", 0.02 + rank * 0.01),
        ] {
            fundamentals.push(FundamentalObservation {
                instrument: InstrumentId::parse(member).unwrap(),
                fiscal_period_start: date("2020-01-01"),
                fiscal_period_end: date("2020-12-31"),
                period_kind: FinancialPeriodKind::Annual,
                statement_scope: StatementScope::Consolidated,
                metric: metric.into(),
                value,
                currency: None,
                unit_scale: 1,
                audited: Some(true),
                disclosed_at: timestamp("2021-01-15T00:00:00Z"),
                available_at: timestamp("2021-01-15T00:01:00Z"),
                source_revision: "fixture-fundamental-1".into(),
                restates_source_revision: None,
            });
        }
    }
    let fundamental_document =
        CandidateDocument::Fundamentals(FundamentalDocument { fundamentals });
    let fundamental_pin = pin(
        "krx_fundamentals",
        "fixture-fundamental-v1",
        &fundamental_hash,
    );
    let raw_batch_id = Uuid::new_v4();
    let raw_manifest_sha256 = "f".repeat(64);
    sqlx::query("SELECT public.begin_candidate_raw_batch($1,'source',$2,'synthetic',$3,$4)")
        .bind(raw_batch_id)
        .bind(&raw_manifest_sha256)
        .bind(CANDIDATE_LICENSE_REF)
        .bind(as_of.as_naive_date())
        .execute(&publisher_pool)
        .await
        .expect("begin exact source Raw ledger");
    for (kind, dataset_version_id) in [
        ("index_membership", universe_dataset_id),
        ("sector_classification", sector_dataset_id),
        ("market_status", status_id),
        ("investor_flow", flow_id),
        ("fundamentals", fundamental_id),
    ] {
        sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source',$2,$3,false)")
            .bind(raw_batch_id)
            .bind(kind)
            .bind(dataset_version_id)
            .execute(&publisher_pool)
            .await
            .expect("bind exact source Raw dataset");
    }
    let source_publications = [
        CandidateSourcePublication {
            raw_batch_id,
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id: universe_dataset_id,
            as_of,
            cutoff_at,
            pin: &universe_pin,
            document: &memberships,
        },
        CandidateSourcePublication {
            raw_batch_id,
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id: sector_dataset_id,
            as_of,
            cutoff_at,
            pin: &sector_pin,
            document: &sectors,
        },
        CandidateSourcePublication {
            raw_batch_id,
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id: status_id,
            as_of,
            cutoff_at,
            pin: &status_pin,
            document: &statuses,
        },
        CandidateSourcePublication {
            raw_batch_id,
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id: flow_id,
            as_of,
            cutoff_at,
            pin: &flow_pin,
            document: &flow_document,
        },
        CandidateSourcePublication {
            raw_batch_id,
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id: fundamental_id,
            as_of,
            cutoff_at,
            pin: &fundamental_pin,
            document: &fundamental_document,
        },
    ];
    assert_eq!(
        sink.publish_batch(&source_publications)
            .await
            .expect("publish coherent candidate source batch"),
        PublishOutcome::Published
    );
    let price_raw_batch_id = Uuid::new_v4();
    sqlx::query("SELECT public.begin_candidate_raw_batch($1,'price',$2,'synthetic',$3,$4)")
        .bind(price_raw_batch_id)
        .bind("e".repeat(64))
        .bind(CANDIDATE_LICENSE_REF)
        .bind(as_of.as_naive_date())
        .execute(&publisher_pool)
        .await
        .expect("begin price Raw ledger");
    sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'price','bars',$2,false)")
        .bind(price_raw_batch_id)
        .bind(price_id)
        .execute(&publisher_pool)
        .await
        .expect("bind price Raw dataset");
    sqlx::query("SELECT public.seal_candidate_raw_batch($1,'price',$2,'synthetic')")
        .bind(price_raw_batch_id)
        .bind("e".repeat(64))
        .execute(&publisher_pool)
        .await
        .expect("seal price Raw ledger");

    // Exact all-source replay is accepted and never creates a partial or
    // duplicate observation set.
    assert_eq!(
        sink.publish_batch(&source_publications)
            .await
            .expect("replay coherent candidate source batch"),
        PublishOutcome::AlreadyPublished
    );

    for session in coverage_sessions {
        sqlx::query(
            "INSERT INTO trading_calendars
             (exchange, session_date, session_type, timezone, source, source_version,
              source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX',$1,'TRADING','Asia/Seoul','synthetic','fixture-calendar-v1',
                     $2,repeat('9',64),$3)",
        )
        .bind(session.as_naive_date())
        .bind(Uuid::new_v4())
        .bind(cutoff())
        .execute(&db.pool)
        .await
        .expect("insert confirmed trading session");
    }
    sqlx::query(
        "UPDATE candidate_scheduler_control
            SET active=true, required_fetch_mode='synthetic'
          WHERE control_key='scheduler'",
    )
    .execute(&db.pool)
    .await
    .expect("activate candidate scheduler");
    let mut config_tx = db.pool.begin().await.expect("scoring config fixture tx");
    sqlx::query("SET LOCAL ROLE migration_owner")
        .execute(&mut *config_tx)
        .await
        .expect("use migration owner for immutable config fixture clock");
    sqlx::query(
        "UPDATE candidate_scoring_configs SET created_at=$1 WHERE version='candidate-score-v1'",
    )
    .bind(cutoff())
    .execute(&mut *config_tx)
    .await
    .expect("pin scoring config fixture clock");
    config_tx
        .commit()
        .await
        .expect("scoring config fixture commit");

    let scoring: (String, String) = sqlx::query_as(
        "SELECT version, content_sha256 FROM candidate_scoring_configs
         WHERE version='candidate-score-v1'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("load scoring config");
    let universe_snapshot_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM candidate_universe_snapshots WHERE dataset_version_id=$1",
    )
    .bind(universe_dataset_id)
    .fetch_one(&db.pool)
    .await
    .expect("universe snapshot id");
    let sector_version_id: Uuid =
        sqlx::query_scalar("SELECT id FROM candidate_sector_versions WHERE dataset_version_id=$1")
            .bind(sector_dataset_id)
            .fetch_one(&db.pool)
            .await
            .expect("sector version id");
    let request = CandidateScheduleRequest {
        as_of_date: NaiveDate::parse_from_str(AS_OF, "%Y-%m-%d").unwrap(),
        cutoff_at: cutoff(),
        scoring_config_version: scoring.0,
        scoring_config_sha256: scoring.1,
        universe_snapshot_id,
        price: DatasetSchedulePin {
            id: price_id,
            manifest_sha256: dataset.manifest_sha256.clone(),
        },
        price_curated_version: 2,
        status: DatasetSchedulePin {
            id: status_id,
            manifest_sha256: status_hash,
        },
        flow: DatasetSchedulePin {
            id: flow_id,
            manifest_sha256: flow_hash,
        },
        fundamental: DatasetSchedulePin {
            id: fundamental_id,
            manifest_sha256: fundamental_hash,
        },
        sector_version_id,
    };

    let worker_pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&db.role_url("worker"))
        .await
        .expect("connect as worker");
    sqlx::query(
        "UPDATE candidate_scheduler_control SET required_fetch_mode='credentialed'
          WHERE control_key='scheduler'",
    )
    .execute(&db.pool)
    .await
    .expect("switch production mode requirement");
    let mixed_mode = schedule_candidate_run(&worker_pool, &request)
        .await
        .expect_err("synthetic source pins cannot satisfy credentialed production scheduling");
    assert!(matches!(mixed_mode, CandidateScheduleError::Database(_)));
    sqlx::query(
        "UPDATE candidate_scheduler_control SET required_fetch_mode='synthetic'
          WHERE control_key='scheduler'",
    )
    .execute(&db.pool)
    .await
    .expect("restore explicit QA mode requirement");
    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE id=$1")
        .bind(CANDIDATE_ENTITLEMENT_ID)
        .execute(&db.pool)
        .await
        .expect("revoke exact candidate entitlement");
    let denied = schedule_candidate_run(&worker_pool, &request)
        .await
        .expect_err("candidate scheduling must fail after exact entitlement revocation");
    assert!(matches!(denied, CandidateScheduleError::Database(_)));

    sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE id=$1")
        .bind(CANDIDATE_ENTITLEMENT_ID)
        .execute(&db.pool)
        .await
        .expect("reactivate exact candidate entitlement");
    let original_effective_from: NaiveDate =
        sqlx::query_scalar("SELECT effective_from FROM data_entitlements WHERE id=$1")
            .bind(CANDIDATE_ENTITLEMENT_ID)
            .fetch_one(&db.pool)
            .await
            .expect("read candidate entitlement window");
    sqlx::query("UPDATE data_entitlements SET effective_from=$2 WHERE id=$1")
        .bind(CANDIDATE_ENTITLEMENT_ID)
        .bind(request.as_of_date)
        .execute(&db.pool)
        .await
        .expect("narrow exact candidate history entitlement");
    let narrowed = schedule_candidate_run(&worker_pool, &request)
        .await
        .expect_err("60-session price and flow history requires the full rights window");
    assert!(matches!(narrowed, CandidateScheduleError::Database(_)));
    sqlx::query("UPDATE data_entitlements SET effective_from=$2 WHERE id=$1")
        .bind(CANDIDATE_ENTITLEMENT_ID)
        .bind(original_effective_from)
        .execute(&db.pool)
        .await
        .expect("restore exact candidate history entitlement");
    let mut forged_cutoff = request.clone();
    forged_cutoff.cutoff_at += chrono::Duration::seconds(1);
    let denied_cutoff = schedule_candidate_run(&worker_pool, &forged_cutoff)
        .await
        .expect_err("worker cannot mint another run by varying the pinned cutoff");
    assert!(matches!(denied_cutoff, CandidateScheduleError::Database(_)));

    let (scheduled_a, scheduled_b) = tokio::join!(
        schedule_candidate_run(&worker_pool, &request),
        schedule_candidate_run(&worker_pool, &request)
    );
    let scheduled_a = scheduled_a.expect("first concurrent schedule");
    let scheduled_b = scheduled_b.expect("second concurrent schedule");
    assert_eq!(scheduled_a, scheduled_b, "schedule replay must be exact");

    let queue_config = QueueConfig {
        lease: Duration::from_secs(30),
        backoff_base: Duration::from_millis(10),
    };
    let queue = JobQueue::new(worker_pool.clone(), None, queue_config);
    let runner_config = CandidateRunnerConfig::new(Duration::from_millis(100), queue_config.lease)
        .expect("runner config");
    let outcome = run_once(
        &worker_pool,
        &queue,
        "candidate-integration-worker",
        &CandidateRunnerPaths {
            data_root: dataset.root.clone(),
        },
        &runner_config,
    )
    .await
    .expect("candidate runner completes");
    assert_eq!(
        outcome,
        CandidateOutcome::Succeeded {
            job_id: scheduled_a.job_id,
            run_id: scheduled_a.run_id,
        }
    );

    let run: (String, i64) = sqlx::query_as(
        "SELECT status,
                (SELECT count(*) FROM stock_analysis_snapshots WHERE run_id=$1)
           FROM stock_analysis_runs WHERE id=$1",
    )
    .bind(scheduled_a.run_id)
    .fetch_one(&db.pool)
    .await
    .expect("published run");
    assert_eq!(run, ("SUCCEEDED".into(), MEMBERS.len() as i64));
    let newly_listed: (bool, bool) = sqlx::query_as(
        "SELECT eligible,
                exclusion_codes @> '[\"INSUFFICIENT_PRICE_HISTORY\"]'::jsonb
           FROM stock_analysis_snapshots
          WHERE run_id=$1 AND instrument_id=$2",
    )
    .bind(scheduled_a.run_id)
    .bind(MEMBERS[MEMBERS.len() - 1])
    .fetch_one(&db.pool)
    .await
    .expect("newly listed candidate exclusion");
    assert_eq!(newly_listed, (false, true));
    let feed: (String, i64, i64) = sqlx::query_as(
        "SELECT feed.status,
                (SELECT count(*) FROM candidate_feed_items WHERE feed_id=feed.id),
                (SELECT count(*) FROM candidate_feed_items AS item
                  JOIN stock_analysis_snapshots AS snapshot
                    ON snapshot.id=item.stock_analysis_snapshot_id
                 WHERE item.feed_id=feed.id AND snapshot.run_id=feed.run_id)
           FROM candidate_feed_snapshots AS feed WHERE feed.run_id=$1",
    )
    .bind(scheduled_a.run_id)
    .fetch_one(&db.pool)
    .await
    .expect("published feed");
    assert_eq!(feed, ("PUBLISHED".into(), 5, 5));
    let recommendation_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&db.pool)
        .await
        .expect("recommendation isolation");
    assert_eq!(recommendation_rows, 0);
    let app_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db.role_url("app"))
        .await
        .expect("connect as app");
    let raw_denied = sqlx::query("SELECT * FROM candidate_investor_flows LIMIT 1")
        .execute(&app_pool)
        .await
        .expect_err("app must not read licensed candidate Raw rows directly");
    assert_eq!(
        raw_denied
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    let attribution_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.candidate_published_source_attributions($1)",
    )
    .bind(scheduled_a.run_id)
    .fetch_one(&app_pool)
    .await
    .expect("sealed published-run attribution");
    assert_eq!(attribution_count, 6);
    let foreign_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.candidate_published_source_attributions($1)",
    )
    .bind(Uuid::new_v4())
    .fetch_one(&app_pool)
    .await
    .expect("unknown run has no attribution");
    assert_eq!(foreign_count, 0);
    app_pool.close().await;
    assert_eq!(
        run_once(
            &worker_pool,
            &queue,
            "candidate-integration-worker",
            &CandidateRunnerPaths {
                data_root: dataset.root.clone(),
            },
            &runner_config,
        )
        .await
        .expect("empty candidate queue"),
        CandidateOutcome::Idle
    );

    publisher_pool.close().await;
    worker_pool.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn rolling_raw_sources_curate_seal_schedule_run_and_publish_without_source_sql_seed() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL is not set");
        return;
    };
    let as_of = date("2026-08-14");
    let retrieved = timestamp("2026-08-14T07:30:00Z");
    sqlx::query(
        "INSERT INTO data_entitlements
         (id,contract_document_sha256,contract_reference,status,covered_datasets,
          covered_uses,effective_from,effective_until,managed_by)
         VALUES ($1,repeat('7',64),$2,'ACTIVE',$3,'[\"candidate\"]'::jsonb,
                 DATE '2020-01-01',DATE '2030-12-31',
                 '00000000-0000-4000-8000-000000000042'::uuid)",
    )
    .bind(CANDIDATE_ENTITLEMENT_ID)
    .bind(CANDIDATE_LICENSE_REF)
    .bind(json!([
        "krx_eod_bars",
        "krx_investor_flows",
        "krx_market_status",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .execute(&db.pool)
    .await
    .expect("rolling candidate entitlement");

    let publisher_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db.role_url("research_writer"))
        .await
        .expect("connect rolling publisher");
    let sink = PostgresCandidateSourceSink::new(publisher_pool.clone());
    let root = tempfile::tempdir().expect("rolling Raw/curated root");
    let raw = RawStore::new(root.path());
    let provider = RollingCandidateProvider;
    let source_outcome = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved),
        Some(CANDIDATE_LICENSE_REF),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("rolling candidate Raw");
    let price_outcome = ingest_bundle(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved),
        Some(CANDIDATE_LICENSE_REF),
    )
    .expect("rolling price Raw");
    let (calendar, master) =
        curation_inputs_from_raw(&raw, &price_outcome.entry).expect("rolling curation inputs");
    let dataset_id = DatasetId::parse("krx_eod_bars").expect("price dataset id");
    let curated = CurateStore::new(root.path());
    let curated_outcome = curate_batch(
        &raw,
        &price_outcome.entry,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "synthetic",
            now: retrieved,
        },
    )
    .expect("rolling price curation");
    let price_evidence =
        price_curation_evidence(&raw, &price_outcome.entry, &curated_outcome.manifest)
            .expect("rolling price evidence");
    assert_eq!(
        price_evidence.instrument_coverage.len(),
        ROLLING_MEMBERS.len()
    );
    assert_eq!(
        price_evidence
            .instrument_coverage
            .iter()
            .filter(|coverage| coverage.session_count >= 60)
            .count(),
        ROLLING_MEMBERS.len() - 1
    );
    sink.register_candidate_instruments(&CandidateInstrumentCatalog {
        master: &master,
        entitlement_id: CANDIDATE_ENTITLEMENT_ID,
        contract_reference: CANDIDATE_LICENSE_REF,
        entitlement_date: as_of,
        reference_sha256: price_outcome
            .entry
            .files
            .iter()
            .find(|file| file.kind == market_data::ResponseKind::Reference)
            .and_then(|file| file.content_hash.as_str().strip_prefix("sha256:"))
            .expect("rolling reference hash"),
        source_revision: &price_outcome.batch_id.to_string(),
        retrieved_at: retrieved,
    })
    .await
    .expect("register rolling instruments");
    let bindings = sink
        .catalog_candidate_batch(&source_outcome)
        .await
        .expect("catalog rolling candidate sources");
    let source_batch = prepare_candidate_batch(&source_outcome, as_of, retrieved, &bindings)
        .expect("prepare rolling typed batch");
    assert_eq!(
        publish_candidate_batch(&sink, &source_batch)
            .await
            .expect("seal rolling typed batch"),
        PublishOutcome::Published
    );
    let price_version = curated_outcome.manifest.version.to_string();
    let price_raw_hash =
        candidate_raw_manifest_sha256(&price_outcome.entry).expect("rolling price Raw hash");
    let price_pin = sink
        .publish_price(&CandidatePricePublication {
            raw_batch_id: price_outcome.batch_id.as_uuid(),
            raw_manifest_sha256: &price_raw_hash,
            fetch_mode: price_outcome.entry.mode,
            entitlement_date: price_outcome.entry.date,
            evidence: &price_evidence,
            dataset_version: &price_version,
            storage_path: root.path().to_str().expect("UTF-8 rolling root"),
            provider: "synthetic",
            entitlement_id: CANDIDATE_ENTITLEMENT_ID,
            license_ref: CANDIDATE_LICENSE_REF,
            available_at: retrieved,
            retrieved_at: retrieved,
        })
        .await
        .expect("seal rolling price publication");
    assert_eq!(price_pin.1, PublishOutcome::Published);

    for session in RollingCandidateProvider::sessions(as_of) {
        sqlx::query(
            "INSERT INTO trading_calendars
             (exchange,session_date,session_type,timezone,source,source_version,
              source_batch_id,content_sha256,retrieved_at)
             VALUES ('KRX',$1,'TRADING','Asia/Seoul','synthetic','rolling-v1',
                     $2,repeat('9',64),$3)",
        )
        .bind(session.as_naive_date())
        .bind(Uuid::new_v4())
        .bind(retrieved.as_datetime())
        .execute(&db.pool)
        .await
        .expect("rolling calendar session");
    }
    sqlx::query(
        "UPDATE candidate_scheduler_control
            SET active=true,required_fetch_mode='synthetic'
          WHERE control_key='scheduler'",
    )
    .execute(&db.pool)
    .await
    .expect("activate rolling scheduler");
    let mut config_tx = db.pool.begin().await.expect("rolling config tx");
    sqlx::query("SET LOCAL ROLE migration_owner")
        .execute(&mut *config_tx)
        .await
        .expect("rolling migration owner");
    sqlx::query("UPDATE candidate_scoring_configs SET created_at=$1")
        .bind(retrieved.as_datetime())
        .execute(&mut *config_tx)
        .await
        .expect("rolling config clock");
    config_tx.commit().await.expect("rolling config commit");

    let worker_pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&db.role_url("worker"))
        .await
        .expect("connect rolling worker");
    let seoul = chrono::FixedOffset::east_opt(9 * 60 * 60).unwrap();
    let scheduled = schedule_latest_candidate_run(
        &worker_pool,
        seoul.with_ymd_and_hms(2026, 8, 14, 17, 0, 0).unwrap(),
    )
    .await
    .expect("schedule rolling sealed pins");
    let queue_config = QueueConfig {
        lease: Duration::from_secs(30),
        backoff_base: Duration::from_millis(10),
    };
    let queue = JobQueue::new(worker_pool.clone(), None, queue_config);
    let runner_config = CandidateRunnerConfig::new(Duration::from_millis(100), queue_config.lease)
        .expect("rolling runner config");
    assert_eq!(
        run_once(
            &worker_pool,
            &queue,
            "candidate-rolling-worker",
            &CandidateRunnerPaths {
                data_root: root.path().to_path_buf(),
            },
            &runner_config,
        )
        .await
        .expect("rolling candidate runner"),
        CandidateOutcome::Succeeded {
            job_id: scheduled.job_id,
            run_id: scheduled.run_id,
        }
    );
    let result: (i64, i64, bool) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM stock_analysis_snapshots WHERE run_id=$1),
           (SELECT count(*) FROM candidate_feed_items AS item
             JOIN candidate_feed_snapshots AS feed ON feed.id=item.feed_id
            WHERE feed.run_id=$1),
           (SELECT exclusion_codes @> '[\"INSUFFICIENT_PRICE_HISTORY\"]'::jsonb
              FROM stock_analysis_snapshots
             WHERE run_id=$1 AND instrument_id=$2)",
    )
    .bind(scheduled.run_id)
    .bind(ROLLING_MEMBERS[ROLLING_MEMBERS.len() - 1])
    .fetch_one(&db.pool)
    .await
    .expect("rolling published result");
    assert_eq!(result, (ROLLING_MEMBERS.len() as i64, 5, true));

    let day1_flow_id = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::InvestorFlow)
        .expect("day-one flow pin")
        .dataset_version_id;
    let day1_counts: (i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM candidate_investor_flows),
           (SELECT count(*) FROM candidate_investor_flow_snapshot_rows
             WHERE dataset_version_id=$1)",
    )
    .bind(day1_flow_id)
    .fetch_one(&db.pool)
    .await
    .expect("day-one immutable flow counts");
    let full_member_count = i64::try_from(ROLLING_MEMBERS.len() - 1).expect("small fixture");
    let investor_classes = 2_i64;
    let day1_memberships = full_member_count * 60 * investor_classes + investor_classes;
    assert_eq!(day1_counts, (day1_memberships, day1_memberships));

    let day2 = date("2026-08-17");
    let day2_retrieved = timestamp("2026-08-17T07:30:00Z");
    let day2_source = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), day2, day2_retrieved),
        Some(CANDIDATE_LICENSE_REF),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("day-two rolling candidate Raw");
    let day2_price = ingest_bundle(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), day2, day2_retrieved),
        Some(CANDIDATE_LICENSE_REF),
    )
    .expect("day-two rolling price Raw");
    let (day2_calendar, day2_master) =
        curation_inputs_from_raw(&raw, &day2_price.entry).expect("day-two curation inputs");
    let day2_curated = curate_batch(
        &raw,
        &day2_price.entry,
        &day2_calendar,
        &day2_master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "synthetic",
            now: day2_retrieved,
        },
    )
    .expect("day-two price curation");
    let day2_evidence = price_curation_evidence(&raw, &day2_price.entry, &day2_curated.manifest)
        .expect("day-two price evidence");
    let day2_source_revision = day2_price.batch_id.to_string();
    for instrument in day2_master.instruments() {
        let canonical: (String, String, String, String, String, Option<String>) = sqlx::query_as(
            "SELECT symbol, name, asset_class, status, listed_at::text, delisted_at::text
                   FROM instruments
                  WHERE id=$1",
        )
        .bind(instrument.instrument_id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("day-two canonical instrument");
        assert_eq!(canonical.0, instrument.instrument_id.symbol());
        assert_eq!(canonical.1, instrument.name);
        assert_eq!(canonical.2, "EQUITY");
        assert_eq!(canonical.3, "ACTIVE");
        assert_eq!(canonical.4, instrument.listed_at.to_iso());
        assert_eq!(canonical.5, None);
    }
    sink.register_candidate_instruments(&CandidateInstrumentCatalog {
        master: &day2_master,
        entitlement_id: CANDIDATE_ENTITLEMENT_ID,
        contract_reference: CANDIDATE_LICENSE_REF,
        entitlement_date: day2,
        reference_sha256: day2_price
            .entry
            .files
            .iter()
            .find(|file| file.kind == market_data::ResponseKind::Reference)
            .and_then(|file| file.content_hash.as_str().strip_prefix("sha256:"))
            .expect("day-two reference hash"),
        source_revision: &day2_source_revision,
        retrieved_at: day2_retrieved,
    })
    .await
    .expect("day-two instrument replay");
    let day2_bindings = sink
        .catalog_candidate_batch(&day2_source)
        .await
        .expect("day-two source catalog");
    let day2_batch = prepare_candidate_batch(&day2_source, day2, day2_retrieved, &day2_bindings)
        .expect("day-two typed batch");
    assert_eq!(
        publish_candidate_batch(&sink, &day2_batch)
            .await
            .expect("day-two sealed source publication"),
        PublishOutcome::Published
    );
    let day2_price_version = day2_curated.manifest.version.to_string();
    let day2_price_hash =
        candidate_raw_manifest_sha256(&day2_price.entry).expect("day-two price Raw hash");
    sink.publish_price(&CandidatePricePublication {
        raw_batch_id: day2_price.batch_id.as_uuid(),
        raw_manifest_sha256: &day2_price_hash,
        fetch_mode: day2_price.entry.mode,
        entitlement_date: day2_price.entry.date,
        evidence: &day2_evidence,
        dataset_version: &day2_price_version,
        storage_path: root.path().to_str().expect("UTF-8 rolling root"),
        provider: "synthetic",
        entitlement_id: CANDIDATE_ENTITLEMENT_ID,
        license_ref: CANDIDATE_LICENSE_REF,
        available_at: day2_retrieved,
        retrieved_at: day2_retrieved,
    })
    .await
    .expect("day-two sealed price publication");
    let day2_flow_id = day2_bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::InvestorFlow)
        .expect("day-two flow pin")
        .dataset_version_id;
    let day2_counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM candidate_investor_flows),
           (SELECT count(*) FROM candidate_investor_flow_snapshot_rows
             WHERE dataset_version_id=$2),
           (SELECT count(*)
              FROM candidate_investor_flow_snapshot_rows AS day1
              JOIN candidate_investor_flow_snapshot_rows AS day2
                ON day2.flow_observation_id=day1.flow_observation_id
             WHERE day1.dataset_version_id=$1 AND day2.dataset_version_id=$2)",
    )
    .bind(day1_flow_id)
    .bind(day2_flow_id)
    .fetch_one(&db.pool)
    .await
    .expect("day-two overlap proof");
    let day2_memberships = full_member_count * 60 * investor_classes + 2 * investor_classes;
    let overlapping_observations = full_member_count * 59 * investor_classes + investor_classes;
    let unique_observations = day1_memberships
        + i64::try_from(ROLLING_MEMBERS.len()).expect("small fixture") * investor_classes;
    assert_eq!(
        day2_counts,
        (
            unique_observations,
            day2_memberships,
            overlapping_observations
        )
    );

    sqlx::query(
        "INSERT INTO trading_calendars
         (exchange,session_date,session_type,timezone,source,source_version,
          source_batch_id,content_sha256,retrieved_at)
         VALUES ('KRX',$1,'TRADING','Asia/Seoul','synthetic','rolling-v2',
                 $2,repeat('8',64),$3)",
    )
    .bind(day2.as_naive_date())
    .bind(Uuid::new_v4())
    .bind(day2_retrieved.as_datetime())
    .execute(&db.pool)
    .await
    .expect("day-two calendar session");
    let scheduled_day2 = schedule_latest_candidate_run(
        &worker_pool,
        seoul.with_ymd_and_hms(2026, 8, 17, 17, 0, 0).unwrap(),
    )
    .await
    .expect("schedule day-two rolling pins");
    assert_eq!(
        run_once(
            &worker_pool,
            &queue,
            "candidate-rolling-worker-day-two",
            &CandidateRunnerPaths {
                data_root: root.path().to_path_buf(),
            },
            &runner_config,
        )
        .await
        .expect("day-two rolling candidate runner"),
        CandidateOutcome::Succeeded {
            job_id: scheduled_day2.job_id,
            run_id: scheduled_day2.run_id,
        }
    );
    let app_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db.role_url("app"))
        .await
        .expect("connect rolling app");
    let app_result: (String, i64, i64) = sqlx::query_as(
        "SELECT run.status,
                (SELECT count(*) FROM candidate_feed_items AS item
                  JOIN candidate_feed_snapshots AS feed ON feed.id=item.feed_id
                 WHERE feed.run_id=run.id),
                (SELECT count(*) FROM public.candidate_published_source_attributions(run.id))
           FROM stock_analysis_runs AS run WHERE run.id=$1",
    )
    .bind(scheduled_day2.run_id)
    .fetch_one(&app_pool)
    .await
    .expect("app reads published rolling result");
    assert_eq!(app_result, ("SUCCEEDED".to_owned(), 5, 6));
    let raw_select = sqlx::query("SELECT 1 FROM candidate_investor_flows LIMIT 1")
        .execute(&app_pool)
        .await
        .expect_err("app must not read licensed candidate source rows directly");
    assert_eq!(
        raw_select
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    let foreign_attributions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.candidate_published_source_attributions($1)",
    )
    .bind(uuid::Uuid::new_v4())
    .fetch_one(&app_pool)
    .await
    .expect("foreign unpublished attribution probe");
    assert_eq!(foreign_attributions, 0);
}
