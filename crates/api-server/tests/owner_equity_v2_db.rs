//! PostgreSQL-backed WP-4 publication and WP-5 scheduling contracts.
//!
//! The common harness creates a fresh database, applies the repository's
//! embedded migrations, and connects through the production `app` and
//! `worker` roles.  No provider is called: the adapter returns deterministic
//! in-memory candidates and the real WP-3 factor engine runs in the worker.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use api_server::repos::owner_equity_v2::{
    OwnerEquityMutationPins, OwnerEquityRepoError, OwnerEquityV2Repo,
};
use async_trait::async_trait;
use auth::entitlement::Role;
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use common::{Harness, UserCtx, actor_pool};
use domain::{
    BatchId, CodeCommit, ContentHash, InstrumentId, OwnerEquityUniverseHash, RetryDisposition,
    TradingDate,
};
use job_queue::owner_equity_v2::{
    AdmittedGenerationDescriptor, OWNER_EQUITY_V2_JOB_TYPE, OwnerEquityCoverage,
    OwnerEquityJobAction, OwnerEquityJobPayload, OwnerEquityMaterialization,
    OwnerEquityPriorCandidate, OwnerEquityRunOutcome, OwnerEquityScheduleError,
    OwnerEquitySchedulePins, OwnerEquityWorkFailure, OwnerEquityWorkerAdapter,
    OwnerEquityWorkerError, PreparedOwnerEquityGeneration, eligible_schedule_date,
    process_owner_equity_claim, run_owner_equity_schedule_cycle,
};
use job_queue::{AttemptOutcome, JobQueue, JobStatus, QueueConfig};
use market_data::owner_equity_v2::merge_owner_equity_incremental_candidate;
use market_data::owner_equity_v2::{
    OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OWNER_ONLY_WARNING,
    OwnerEquityBar, OwnerEquityCaptureKind, OwnerEquityGenerationCandidate, OwnerEquitySourcePins,
    PRICE_SEMANTICS, RESEARCH_ONLY_WARNING, STRICT_PIT_WARNING, VENDOR_SNAPSHOT_WARNING,
};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;

const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_REFERENCE: &str = "krx-2026-01";
const WP5_SCHEDULE_ENTITLEMENT_REFERENCE: &str = "wp5-owner-equity-scheduler-date-window";
const TARGET_SESSIONS: i32 = 130;
const MINIMUM_SESSIONS: i32 = 121;

#[derive(Clone, Default)]
struct FakeAdapter {
    candidates: Arc<RwLock<BTreeMap<String, OwnerEquityGenerationCandidate>>>,
    insufficient_once: Arc<Mutex<BTreeSet<String>>>,
}

impl FakeAdapter {
    async fn make_insufficient_once(&self, instrument_id: &str) {
        self.insufficient_once
            .lock()
            .await
            .insert(instrument_id.to_owned());
    }
}

#[async_trait]
impl OwnerEquityWorkerAdapter for FakeAdapter {
    async fn validate(
        &self,
        _payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        Ok(())
    }

    async fn backfill(
        &self,
        _payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        Ok(())
    }

    async fn materialize(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        if self
            .insufficient_once
            .lock()
            .await
            .remove(&payload.instrument_id)
        {
            return Ok(OwnerEquityMaterialization::InsufficientHistory(
                OwnerEquityCoverage {
                    observed_sessions: 120,
                    first_session: Some(
                        payload
                            .requested_through
                            .checked_add_days(-119)
                            .expect("fixture coverage start"),
                    ),
                    last_session: Some(payload.requested_through),
                },
            ));
        }
        let candidate = candidate_for(payload);
        self.candidates
            .write()
            .await
            .insert(payload.instrument_id.clone(), candidate.clone());
        Ok(OwnerEquityMaterialization::Ready(Box::new(
            PreparedOwnerEquityGeneration {
                artifact_manifest_sha256: ContentHash::from_bytes(
                    format!(
                        "artifact:{}:{}",
                        payload.instrument_id,
                        payload.expected_generation.expect("generation action")
                    )
                    .as_bytes(),
                ),
                candidate,
            },
        )))
    }

    async fn materialize_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        if payload.action != OwnerEquityJobAction::Incremental {
            return self.materialize(payload).await;
        }

        let prior = prior.ok_or_else(missing_evidence)?;
        let incremental = incremental_candidate_for(payload, prior);
        let prior_artifact = ContentHash::parse(&prior.descriptor.artifact_manifest_sha256)
            .map_err(|_| missing_evidence())?;
        let candidate = merge_owner_equity_incremental_candidate(
            &prior.candidate,
            prior_artifact,
            &incremental,
        )
        .map_err(|_| missing_evidence())?;
        let generation = payload.expected_generation.expect("incremental generation");
        self.candidates
            .write()
            .await
            .insert(payload.instrument_id.clone(), candidate.clone());
        Ok(OwnerEquityMaterialization::Ready(Box::new(
            PreparedOwnerEquityGeneration {
                artifact_manifest_sha256: ContentHash::from_bytes(
                    format!("artifact:{}:{generation}", payload.instrument_id).as_bytes(),
                ),
                candidate,
            },
        )))
    }

    async fn load_admitted_candidate(
        &self,
        descriptor: &AdmittedGenerationDescriptor,
    ) -> Result<OwnerEquityGenerationCandidate, OwnerEquityWorkFailure> {
        self.candidates
            .read()
            .await
            .get(&descriptor.instrument_id)
            .cloned()
            .ok_or_else(missing_evidence)
    }
}

#[derive(Clone)]
struct PublicationGateAdapter {
    inner: FakeAdapter,
    materializing: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl OwnerEquityWorkerAdapter for PublicationGateAdapter {
    async fn validate(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.inner.validate(payload).await
    }

    async fn backfill(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.inner.backfill(payload).await
    }

    async fn materialize(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        self.materializing.notify_one();
        self.release.notified().await;
        self.inner.materialize(payload).await
    }

    async fn load_admitted_candidate(
        &self,
        descriptor: &AdmittedGenerationDescriptor,
    ) -> Result<OwnerEquityGenerationCandidate, OwnerEquityWorkFailure> {
        self.inner.load_admitted_candidate(descriptor).await
    }
}

fn missing_evidence() -> OwnerEquityWorkFailure {
    OwnerEquityWorkFailure::new("EVIDENCE_MISSING", RetryDisposition::Terminal)
        .expect("typed fixture failure")
}

fn pins() -> OwnerEquityMutationPins {
    OwnerEquityMutationPins {
        code_commit: CODE_COMMIT.to_owned(),
        entitlement_reference: ENTITLEMENT_REFERENCE.to_owned(),
        entitlement_sha256: format!("sha256:{}", "d".repeat(64)),
        requested_through: TradingDate::parse("2026-08-31").expect("fixture date"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScheduleFixtureDateWindow {
    eligible: NaiveDate,
    prior_close: NaiveDate,
    initial_session: NaiveDate,
    initial_candidate_start: NaiveDate,
}

fn schedule_fixture_date_window(eligible: NaiveDate) -> ScheduleFixtureDateWindow {
    let prior_close = eligible.pred_opt().expect("prior fixture close");
    let initial_session = prior_close.pred_opt().expect("initial fixture session");
    let initial_candidate_start = initial_session
        .checked_sub_signed(chrono::Duration::days(120))
        .expect("initial candidate coverage start");
    ScheduleFixtureDateWindow {
        eligible,
        prior_close,
        initial_session,
        initial_candidate_start,
    }
}

fn candidate_for(payload: &OwnerEquityJobPayload) -> OwnerEquityGenerationCandidate {
    let generation = payload.expected_generation.expect("generation action");
    let instrument_id = InstrumentId::parse(&payload.instrument_id).expect("fixture instrument");
    let requested_end = payload.requested_through;
    let requested_start = requested_end
        .checked_add_days(-120)
        .expect("fixture history start");
    let symbol_bias = instrument_id
        .symbol()
        .parse::<u64>()
        .expect("numeric KRX fixture")
        % 1_000;
    let bars = (0..121_u64)
        .map(|offset| {
            let close = 10_000 + symbol_bias + offset * (generation + 1);
            OwnerEquityBar {
                session_date: requested_start
                    .checked_add_days(i64::try_from(offset).expect("small fixture offset"))
                    .expect("fixture session"),
                open: close,
                high: close + 10,
                low: close - 10,
                close,
                volume: 100_000 + symbol_bias + offset * 100,
            }
        })
        .collect::<Vec<_>>();
    let code_commit = CodeCommit::parse(&payload.code_commit).expect("fixture commit");
    let entitlement_sha256 =
        ContentHash::parse(&payload.entitlement_sha256).expect("fixture entitlement hash");
    OwnerEquityGenerationCandidate {
        candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
        contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
        capture_kind: if payload.action == OwnerEquityJobAction::Incremental {
            OwnerEquityCaptureKind::Incremental
        } else {
            OwnerEquityCaptureKind::Initial
        },
        instrument_id,
        display_name: None,
        requested_start,
        requested_end,
        target_observed_sessions: u32::try_from(TARGET_SESSIONS).expect("positive target"),
        minimum_observed_sessions: u32::try_from(MINIMUM_SESSIONS).expect("positive minimum"),
        observed_sessions: 121,
        first_observed_date: bars.first().expect("bars").session_date,
        last_observed_date: bars.last().expect("bars").session_date,
        bars,
        source_pins: OwnerEquitySourcePins {
            capture_identity_sha256: ContentHash::from_bytes(
                format!("identity:{}:{generation}", payload.instrument_id).as_bytes(),
            ),
            raw_batch_id: BatchId::from_uuid(Uuid::new_v4()),
            raw_manifest_sha256: ContentHash::from_bytes(
                format!("raw:{}:{generation}", payload.instrument_id).as_bytes(),
            ),
            batch_json_sha256: ContentHash::from_bytes(
                format!("batch:{}:{generation}", payload.instrument_id).as_bytes(),
            ),
            entitlement_reference: payload.entitlement_reference.clone(),
            entitlement_sha256,
            capture_code_commit: code_commit.clone(),
            materializer_code_commit: code_commit,
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

fn incremental_candidate_for(
    payload: &OwnerEquityJobPayload,
    prior: &OwnerEquityPriorCandidate,
) -> OwnerEquityGenerationCandidate {
    let mut candidate = candidate_for(payload);
    let prior_last = prior.candidate.last_observed_date;
    let mut bars = vec![
        prior
            .candidate
            .bars
            .last()
            .expect("admitted prior has bars")
            .clone(),
    ];
    let symbol_bias = InstrumentId::parse(&payload.instrument_id)
        .expect("fixture instrument")
        .symbol()
        .parse::<u64>()
        .expect("numeric KRX fixture")
        % 1_000;
    let generation = payload.expected_generation.expect("generation action");
    let mut date = prior_last.checked_add_days(1).expect("next fixture date");
    let mut offset = 0_u64;
    while date <= payload.requested_through {
        let close = 20_000 + symbol_bias + generation * 100 + offset;
        bars.push(OwnerEquityBar {
            session_date: date,
            open: close,
            high: close + 10,
            low: close - 10,
            close,
            volume: 200_000 + symbol_bias + offset * 100,
        });
        date = date.checked_add_days(1).expect("next fixture date");
        offset += 1;
    }

    candidate.requested_start = prior_last;
    candidate.observed_sessions = u32::try_from(bars.len()).expect("small fixture range");
    candidate.first_observed_date = bars.first().expect("overlap bar").session_date;
    candidate.last_observed_date = bars.last().expect("new fixture bar").session_date;
    candidate.bars = bars;
    candidate
}

async fn provision_policy(harness: &Harness, owner: &UserCtx, maximum: i32) {
    let repo = OwnerEquityV2Repo::new(harness.app_pool.clone());
    let (policy, memberships) = repo
        .list(&owner.actor())
        .await
        .expect("migration-provisioned owner policy exists");
    assert_eq!(policy.max_active_instruments, 100);
    assert_eq!(policy.target_observed_sessions, 261);
    assert_eq!(policy.minimum_observed_sessions, MINIMUM_SESSIONS);
    assert!(memberships.is_empty());

    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_equity_universe_policies \
                 SET max_active_instruments = {maximum}, \
                     target_observed_sessions = {TARGET_SESSIONS}, \
                     minimum_observed_sessions = {MINIMUM_SESSIONS} \
                 WHERE owner_user_id = '{}'",
                owner.user_id
            ),
        )
        .await;

    let (policy, memberships) = repo
        .list(&owner.actor())
        .await
        .expect("updated owner policy remains visible to its actor");
    assert_eq!(policy.max_active_instruments, maximum);
    assert_eq!(policy.target_observed_sessions, TARGET_SESSIONS);
    assert_eq!(policy.minimum_observed_sessions, MINIMUM_SESSIONS);
    assert!(memberships.is_empty());
}

async fn seed_owner(harness: &Harness, suffix: &str, maximum: i32) -> UserCtx {
    let owner = harness
        .seed_user(
            Role::Owner,
            &format!("oev2-{suffix}@lagrange.test"),
            "owner-equity-v2-db",
            suffix,
        )
        .await;
    provision_policy(harness, &owner, maximum).await;
    owner
}

async fn seed_schedule_calendar(harness: &Harness, session_date: NaiveDate, session_type: &str) {
    assert!(matches!(session_type, "TRADING" | "CLOSED"));
    let batch_id = Uuid::new_v4();
    let date = session_date.to_string();
    let hash = "c".repeat(64);
    harness
        .seed_shared(&format!(
            "INSERT INTO data_batches \
             (provider, market, batch_date, kind, storage_path, content_sha256, \
              bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX', 'KR', DATE '{date}', 'CALENDAR', \
                     'synthetic/wp5-owner-v2-calendar', '{hash}', 1, \
                     pg_catalog.clock_timestamp(), '{batch_id}', 'calendar.json', 'credentialed')"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendar_versions \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', DATE '{date}', '{session_type}', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{hash}', \
                     pg_catalog.clock_timestamp())"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendars \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', DATE '{date}', '{session_type}', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{hash}', \
                     pg_catalog.clock_timestamp())"
        ))
        .await;
}

async fn seed_wp5_schedule_entitlement(harness: &Harness, window: ScheduleFixtureDateWindow) {
    let effective_from = window.initial_candidate_start;
    let effective_until = window.eligible;
    let sql = format!(
        "INSERT INTO data_entitlements \
         (id, contract_document_sha256, contract_reference, status, covered_datasets, \
          covered_uses, effective_from, effective_until, managed_by) \
         VALUES (gen_random_uuid(), repeat('d',64), \
                 '{WP5_SCHEDULE_ENTITLEMENT_REFERENCE}', 'ACTIVE', \
                 '[\"krx_eod_bars\"]'::jsonb, \
                 '[\"dataset\",\"factor\",\"recommendation\",\"backtest\",\"report\",\"benchmark\",\"paper_view\",\"payload\",\"download\"]'::jsonb, \
                 DATE '{effective_from}', DATE '{effective_until}', \
                 (SELECT id FROM users WHERE email='owner@lagrange.test'))"
    );
    harness.seed_shared(&sql).await;
}

#[test]
fn schedule_fixture_entitlement_window_covers_year_boundary_candidates() {
    let cases = [
        (
            NaiveDate::from_ymd_opt(2026, 12, 31).expect("2026 boundary"),
            NaiveDate::from_ymd_opt(2026, 12, 30).expect("2026 prior close"),
            NaiveDate::from_ymd_opt(2026, 12, 29).expect("2026 initial session"),
            NaiveDate::from_ymd_opt(2026, 8, 31).expect("2026 initial coverage"),
        ),
        (
            NaiveDate::from_ymd_opt(2027, 1, 1).expect("2027 boundary"),
            NaiveDate::from_ymd_opt(2026, 12, 31).expect("2027 prior close"),
            NaiveDate::from_ymd_opt(2026, 12, 30).expect("2027 initial session"),
            NaiveDate::from_ymd_opt(2026, 9, 1).expect("2027 initial coverage"),
        ),
    ];

    for (eligible, expected_prior_close, expected_initial_session, expected_start) in cases {
        let window = schedule_fixture_date_window(eligible);
        assert_eq!(window.prior_close, expected_prior_close);
        assert_eq!(window.initial_session, expected_initial_session);
        assert_eq!(window.initial_candidate_start, expected_start);
        let current_candidate_start = window
            .eligible
            .checked_sub_signed(chrono::Duration::days(120))
            .expect("current candidate coverage start");
        for candidate_date in [
            window.initial_candidate_start,
            window.initial_session,
            window.prior_close,
            current_candidate_start,
            window.eligible,
        ] {
            assert!(
                window.initial_candidate_start <= candidate_date
                    && candidate_date <= window.eligible
            );
        }
    }
}

async fn table_count(pool: &sqlx::PgPool, table: &str, owner: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM public.{table} WHERE owner_user_id = $1"
    )))
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("fixture table count")
}

#[tokio::test]
async fn owner_equity_v2_repository_queue_and_publication_are_atomic_and_actor_scoped() {
    let Some(harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    provision_policy(&harness, &harness.owner, 2).await;
    let other_owner = seed_owner(&harness, "other-owner", 2).await;
    let retry_owner = seed_owner(&harness, "retry-owner", 1).await;
    let crash_owner = seed_owner(&harness, "crash-owner", 1).await;
    let disabled_owner = seed_owner(&harness, "disabled-owner", 1).await;
    let atomic_owner = seed_owner(&harness, "atomic-owner", 1).await;

    let repo = OwnerEquityV2Repo::new(harness.app_pool.clone());
    let worker_pool = harness.worker_pool().await;
    let queue = JobQueue::new(
        worker_pool.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(5),
            backoff_base: Duration::from_millis(1),
        },
    );
    let adapter = FakeAdapter::default();
    let owner_actor = harness.owner.actor();
    let other_actor = other_owner.actor();
    let body_a = "a".repeat(64);
    let body_b = "b".repeat(64);
    let mutation_pins = pins();

    // Policy provisioning is owner scoped. Member and unscoped sessions see
    // neither the policy nor any future membership through database RLS.
    let (policy, memberships) = repo.list(&owner_actor).await.expect("owner policy");
    assert_eq!(policy.max_active_instruments, 2);
    assert_eq!(policy.target_observed_sessions, TARGET_SESSIONS);
    assert_eq!(policy.minimum_observed_sessions, MINIMUM_SESSIONS);
    assert!(memberships.is_empty());
    assert!(matches!(
        repo.list(&harness.member.actor()).await,
        Err(OwnerEquityRepoError::NotFound)
    ));
    let unscoped: i64 = sqlx::query_scalar("SELECT count(*) FROM owner_equity_universe_policies")
        .fetch_one(&harness.app_pool)
        .await
        .expect("unscoped RLS read");
    assert_eq!(unscoped, 0);

    // Same-key concurrent adds serialize to one membership/job. The durable
    // replay survives another call, while a different body fails closed.
    let (same_one, same_two) = tokio::join!(
        repo.add(
            &owner_actor,
            "005930",
            "concurrent-same-key",
            &body_a,
            &mutation_pins
        ),
        repo.add(
            &owner_actor,
            "005930",
            "concurrent-same-key",
            &body_a,
            &mutation_pins
        )
    );
    let same_one = same_one.expect("first same-key add");
    let same_two = same_two.expect("second same-key add");
    assert_eq!(same_one.membership.id, same_two.membership.id);
    assert_eq!(same_one.job_id, same_two.job_id);
    assert_ne!(same_one.replayed, same_two.replayed);
    let mut owner_lock = harness.app_pool.begin().await.expect("owner lock tx");
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock(\
             pg_catalog.hashtextextended($1::text, 0))",
    )
    .bind(harness.owner.user_id.to_string())
    .execute(&mut *owner_lock)
    .await
    .expect("hold owner advisory lock");
    let replay_repo = repo.clone();
    let replay_actor = owner_actor.clone();
    let replay_body = body_a.clone();
    let replay_pins = mutation_pins.clone();
    let replay_task = tokio::spawn(async move {
        replay_repo
            .add(
                &replay_actor,
                "005930",
                "concurrent-same-key",
                &replay_body,
                &replay_pins,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !replay_task.is_finished(),
        "durable replay must share the owner mutation lock"
    );
    owner_lock.rollback().await.expect("release owner lock");
    let durable_replay = replay_task
        .await
        .expect("durable replay task")
        .expect("durable replay");
    assert!(durable_replay.replayed);
    assert_eq!(durable_replay.job_id, same_one.job_id);
    assert!(matches!(
        repo.add(
            &owner_actor,
            "005930",
            "concurrent-same-key",
            &body_b,
            &mutation_pins
        )
        .await,
        Err(OwnerEquityRepoError::IdempotencyMismatch)
    ));

    // With one slot remaining, two different-key/instrument transactions
    // race under the locked policy row: exactly one commits.
    let (capacity_one, capacity_two) = tokio::join!(
        repo.add(
            &owner_actor,
            "000660",
            "capacity-key-one",
            &body_a,
            &mutation_pins
        ),
        repo.add(
            &owner_actor,
            "035420",
            "capacity-key-two",
            &body_a,
            &mutation_pins
        )
    );
    let capacity_result = match (capacity_one, capacity_two) {
        (Ok(result), Err(OwnerEquityRepoError::CapacityExceeded))
        | (Err(OwnerEquityRepoError::CapacityExceeded), Ok(result)) => result,
        unexpected => panic!("capacity serialization failed: {unexpected:?}"),
    };
    let duplicate = repo
        .add(
            &owner_actor,
            "005930",
            "duplicate-active-key",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("duplicate active receipt");
    assert!(duplicate.duplicate_active);
    assert_eq!(duplicate.membership.id, same_one.membership.id);
    assert_ne!(duplicate.job_id, same_one.job_id);
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_instrument_generations",
            harness.owner.user_id
        )
        .await,
        0,
        "API transactions never create a generation"
    );

    // Different actors cannot read or mutate the owner's row, and direct RLS
    // reads under Member/different-owner GUCs are empty.
    assert!(matches!(
        repo.get(&other_actor, same_one.membership.id).await,
        Err(OwnerEquityRepoError::NotFound)
    ));
    assert!(matches!(
        repo.retry(
            &other_actor,
            same_one.membership.id,
            "other-owner-retry",
            &body_a,
            &mutation_pins
        )
        .await,
        Err(OwnerEquityRepoError::NotFound)
    ));
    assert!(matches!(
        repo.disable(
            &other_actor,
            same_one.membership.id,
            "other-owner-disable",
            &body_a,
            &mutation_pins
        )
        .await,
        Err(OwnerEquityRepoError::NotFound)
    ));
    let member_pool = harness.member_pool().await;
    let different_owner_pool =
        actor_pool(&harness.app_url, &other_owner.user_id.to_string(), 2).await;
    for pool in [&member_pool, &different_owner_pool] {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM owner_equity_memberships")
            .fetch_one(pool)
            .await
            .expect("RLS membership read");
        assert_eq!(count, 0);
    }

    // Two real worker publications commit exact 1-row then 2-row universes.
    // The duplicate receipt has a job but never reaches an adapter/generation.
    let first_claim = queue
        .claim_next_for("oev2-main-one", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim first add")
        .expect("first add queued");
    assert_eq!(first_claim.job.id, same_one.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &first_claim, &adapter)
            .await
            .expect("publish first add"),
        OwnerEquityRunOutcome::Published
    );
    let first_snapshot = repo
        .latest_snapshot(&owner_actor)
        .await
        .expect("first snapshot read")
        .expect("first snapshot present");
    assert_eq!(first_snapshot.rows.len(), 1);
    assert_eq!(first_snapshot.rows[0].rank, 1);

    let second_claim = queue
        .claim_next_for("oev2-main-two", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim second add")
        .expect("second add queued");
    assert_eq!(second_claim.job.id, capacity_result.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &second_claim, &adapter)
            .await
            .expect("publish second add"),
        OwnerEquityRunOutcome::Published
    );
    let latest = repo
        .latest_snapshot(&owner_actor)
        .await
        .expect("latest snapshot read")
        .expect("latest snapshot present");
    assert_eq!(latest.rows.len(), 2);
    assert_eq!(
        latest.rows.iter().map(|row| row.rank).collect::<Vec<_>>(),
        vec![1, 2]
    );
    let expected_universe = OwnerEquityUniverseHash::from_active_ready(
        latest.rows.iter().map(|row| &row.signal.instrument_id),
    )
    .expect("exact universe hash");
    assert_eq!(latest.snapshot.universe_sha256, expected_universe.as_str());
    assert_eq!(latest.snapshot.row_count, 2);
    assert_ne!(latest.snapshot.id, first_snapshot.snapshot.id);
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_instrument_generations",
            harness.owner.user_id
        )
        .await,
        2
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_generation_admissions",
            harness.owner.user_id
        )
        .await,
        2
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_signal_snapshots",
            harness.owner.user_id
        )
        .await,
        2,
        "the last valid snapshot remains while the next exact snapshot publishes"
    );
    let duplicate_status: JobStatus = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(duplicate.job_id)
        .fetch_one(&worker_pool)
        .await
        .expect("duplicate receipt status");
    assert_eq!(duplicate_status, JobStatus::Succeeded);
    assert!(
        queue
            .claim_next_for("oev2-duplicate", OWNER_EQUITY_V2_JOB_TYPE)
            .await
            .expect("scan after duplicate receipt")
            .is_none(),
        "terminal duplicate receipt is never claimable",
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_instrument_generations",
            harness.owner.user_id
        )
        .await,
        2,
        "duplicate add never creates a backfill generation"
    );

    // Insufficient history persists generation 1 without admission. Retry is
    // a new durable job/generation 2 and publishes the first admitted row.
    let retry_actor = retry_owner.actor();
    adapter.make_insufficient_once("123456.KRX").await;
    let retry_add = repo
        .add(
            &retry_actor,
            "123456",
            "insufficient-add",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("insufficient add");
    let insufficient_claim = queue
        .claim_next_for("oev2-insufficient", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim insufficient")
        .expect("insufficient job queued");
    assert_eq!(insufficient_claim.job.id, retry_add.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &insufficient_claim, &adapter)
            .await
            .expect("persist insufficient"),
        OwnerEquityRunOutcome::InsufficientHistory
    );
    let retry_status = repo
        .get(&retry_actor, retry_add.membership.id)
        .await
        .expect("insufficient status")
        .1;
    assert_eq!(retry_status.state, "INSUFFICIENT_HISTORY");
    assert_eq!(retry_status.generation, 1);
    let retry_mutation = repo
        .retry(
            &retry_actor,
            retry_add.membership.id,
            "insufficient-retry",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("retry mutation");
    let retry_claim = queue
        .claim_next_for("oev2-retry", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim retry")
        .expect("retry queued");
    assert_eq!(retry_claim.job.id, retry_mutation.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &retry_claim, &adapter)
            .await
            .expect("publish retry"),
        OwnerEquityRunOutcome::Published
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_instrument_generations",
            retry_owner.user_id
        )
        .await,
        2
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_generation_admissions",
            retry_owner.user_id
        )
        .await,
        1
    );
    let retry_latest = repo
        .latest_snapshot(&retry_actor)
        .await
        .expect("retry latest")
        .expect("retry snapshot");
    assert_eq!(retry_latest.rows[0].generation, 2);

    // A crashed claim is orphaned and requeued. The zombie claim is rejected
    // before adapter/publication work; the recovered claim publishes once.
    let crash_actor = crash_owner.actor();
    let crash_add = repo
        .add(&crash_actor, "222222", "crash-add", &body_a, &mutation_pins)
        .await
        .expect("crash add");
    let crash_queue = JobQueue::new(
        worker_pool.clone(),
        None,
        QueueConfig {
            lease: Duration::from_millis(25),
            backoff_base: Duration::from_millis(1),
        },
    );
    let stale_claim = crash_queue
        .claim_next_for("oev2-crashed", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim crash job")
        .expect("crash job queued");
    assert_eq!(stale_claim.job.id, crash_add.job_id);
    tokio::time::sleep(Duration::from_millis(60)).await;
    let swept = crash_queue.sweep().await.expect("sweep expired claim");
    assert_eq!(swept.attempts_orphaned, 1);
    assert_eq!(swept.jobs_requeued, 1);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &crash_queue, &stale_claim, &adapter).await,
        Err(OwnerEquityWorkerError::StaleClaim)
    );
    let orphaned: AttemptOutcome =
        sqlx::query_scalar("SELECT outcome FROM job_attempts WHERE id = $1")
            .bind(stale_claim.attempt.id)
            .fetch_one(&worker_pool)
            .await
            .expect("orphaned attempt");
    assert_eq!(orphaned, AttemptOutcome::Orphaned);
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_generation_admissions",
            crash_owner.user_id
        )
        .await,
        0
    );
    tokio::time::sleep(Duration::from_millis(5)).await;
    let recovered = crash_queue
        .claim_next_for("oev2-recovered", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim recovered")
        .expect("recovered job available");
    assert_eq!(recovered.job.id, crash_add.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &crash_queue, &recovered, &adapter)
            .await
            .expect("publish recovered"),
        OwnerEquityRunOutcome::Published
    );
    assert_eq!(
        table_count(
            &worker_pool,
            "owner_equity_generation_admissions",
            crash_owner.user_id
        )
        .await,
        1
    );

    // Pause an add after MATERIALIZING, disable it through the owner API repo,
    // then release the stale worker. The publication lock sees DISABLED and
    // no generation/admission/snapshot can escape.
    let disabled_actor = disabled_owner.actor();
    let disable_add = repo
        .add(
            &disabled_actor,
            "333333",
            "disable-race-add",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("disable race add");
    let disable_claim = queue
        .claim_next_for("oev2-disable-race", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim disable race")
        .expect("disable race queued");
    assert_eq!(disable_claim.job.id, disable_add.job_id);
    let materializing = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gated_adapter = PublicationGateAdapter {
        inner: adapter.clone(),
        materializing: materializing.clone(),
        release: release.clone(),
    };
    let task_pool = worker_pool.clone();
    let task_queue = queue.clone();
    let task_claim = disable_claim.clone();
    let worker = tokio::spawn(async move {
        process_owner_equity_claim(&task_pool, &task_queue, &task_claim, &gated_adapter).await
    });
    materializing.notified().await;
    let disable_mutation = repo
        .disable(
            &disabled_actor,
            disable_add.membership.id,
            "disable-race-mutation",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("disable while materializing");
    assert_eq!(disable_mutation.membership.state, "DISABLED");
    release.notify_one();
    assert_eq!(
        worker
            .await
            .expect("disable worker joins")
            .expect("disable outcome"),
        OwnerEquityRunOutcome::Disabled
    );
    for table in [
        "owner_equity_instrument_generations",
        "owner_equity_generation_admissions",
        "owner_equity_signal_snapshots",
        "owner_equity_signal_snapshot_rows",
    ] {
        assert_eq!(
            table_count(&worker_pool, table, disabled_owner.user_id).await,
            0
        );
    }
    let disable_snapshot_claim = queue
        .claim_next_for("oev2-disable-snapshot", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim disable snapshot")
        .expect("disable snapshot queued");
    assert_eq!(disable_snapshot_claim.job.id, disable_mutation.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &disable_snapshot_claim, &adapter)
            .await
            .expect("publish empty disabled universe"),
        OwnerEquityRunOutcome::Published
    );
    let disabled_latest = repo
        .latest_snapshot(&disabled_actor)
        .await
        .expect("disabled latest")
        .expect("empty snapshot is published");
    assert_eq!(disabled_latest.snapshot.row_count, 0);
    assert!(disabled_latest.rows.is_empty());

    // A test-only trigger aborts the last snapshot-row insert. Generation,
    // admission, READY, snapshot, rows, published pointer, and SUCCEEDED job
    // must roll back together; the worker then records a separate FAILED
    // settlement without claiming that publication committed.
    harness
        .seed_shared(
            "CREATE FUNCTION public.wp4_reject_owner_equity_snapshot_row() \
             RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
             RAISE EXCEPTION 'wp4 injected rollback' USING ERRCODE = '23514'; END; $$",
        )
        .await;
    harness
        .seed_shared(
            "CREATE TRIGGER wp4_reject_owner_equity_snapshot_row \
             BEFORE INSERT ON public.owner_equity_signal_snapshot_rows \
             FOR EACH ROW EXECUTE FUNCTION public.wp4_reject_owner_equity_snapshot_row()",
        )
        .await;
    let atomic_actor = atomic_owner.actor();
    let atomic_add = repo
        .add(
            &atomic_actor,
            "444444",
            "atomic-rollback-add",
            &body_a,
            &mutation_pins,
        )
        .await
        .expect("atomic rollback add");
    let atomic_claim = queue
        .claim_next_for("oev2-atomic-rollback", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim atomic rollback")
        .expect("atomic rollback queued");
    assert_eq!(atomic_claim.job.id, atomic_add.job_id);
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &atomic_claim, &adapter)
            .await
            .expect("atomic rollback settles"),
        OwnerEquityRunOutcome::Failed
    );
    harness
        .seed_shared(
            "DROP TRIGGER wp4_reject_owner_equity_snapshot_row \
             ON public.owner_equity_signal_snapshot_rows",
        )
        .await;
    harness
        .seed_shared("DROP FUNCTION public.wp4_reject_owner_equity_snapshot_row()")
        .await;
    for table in [
        "owner_equity_instrument_generations",
        "owner_equity_generation_admissions",
        "owner_equity_signal_snapshots",
        "owner_equity_signal_snapshot_rows",
    ] {
        assert_eq!(
            table_count(&worker_pool, table, atomic_owner.user_id).await,
            0
        );
    }
    let atomic_status = repo
        .get(&atomic_actor, atomic_add.membership.id)
        .await
        .expect("atomic failure status")
        .1;
    assert_eq!(atomic_status.state, "FAILED");
    let atomic_job: JobStatus = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(atomic_add.job_id)
        .fetch_one(&worker_pool)
        .await
        .expect("atomic job status");
    assert_eq!(atomic_job, JobStatus::Failed);
    assert!(
        repo.latest_snapshot(&atomic_actor)
            .await
            .expect("atomic latest read")
            .is_none()
    );

    member_pool.close().await;
    different_owner_pool.close().await;
    worker_pool.close().await;
    harness.teardown().await;
}

#[tokio::test]
async fn owner_equity_schedule_rechecks_late_calendar_after_older_job_settles() {
    let Some(harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    provision_policy(&harness, &harness.owner, 2).await;

    let repo = OwnerEquityV2Repo::new(harness.app_pool.clone());
    let worker_pool = harness.worker_pool().await;
    let queue = JobQueue::new(
        worker_pool.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(30),
            backoff_base: Duration::from_millis(1),
        },
    );
    let adapter = FakeAdapter::default();
    let actor = harness.owner.actor();
    let body = "a".repeat(64);
    let seoul = FixedOffset::east_opt(9 * 60 * 60).expect("KST offset");
    let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&worker_pool)
        .await
        .expect("database clock");
    let date_window = schedule_fixture_date_window(
        eligible_schedule_date(database_now.with_timezone(&seoul))
            .expect("prior eligible date exists"),
    );
    let ScheduleFixtureDateWindow {
        eligible,
        prior_close,
        initial_session,
        ..
    } = date_window;
    let at_close = |date: NaiveDate| {
        seoul
            .from_local_datetime(&date.and_hms_opt(16, 30, 0).expect("close time"))
            .single()
            .expect("fixed-offset date")
    };

    seed_wp5_schedule_entitlement(&harness, date_window).await;

    let mut mutation_pins = pins();
    mutation_pins.entitlement_reference = WP5_SCHEDULE_ENTITLEMENT_REFERENCE.to_owned();
    mutation_pins.entitlement_sha256 = format!("sha256:{}", "d".repeat(64));
    mutation_pins.requested_through =
        TradingDate::parse(&initial_session.to_string()).expect("initial session pin");
    let add = repo
        .add(
            &actor,
            "005930",
            "wp5-schedule-cadence-add",
            &body,
            &mutation_pins,
        )
        .await
        .expect("seed ready membership");
    let initial_claim = queue
        .claim_next_for("wp5-initial", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim initial membership")
        .expect("initial membership queued");
    assert_eq!(initial_claim.job.id, add.job_id);
    let initial_payload: OwnerEquityJobPayload =
        serde_json::from_value(initial_claim.job.payload_json.clone())
            .expect("initial membership payload");
    assert_eq!(
        initial_payload.entitlement_reference,
        WP5_SCHEDULE_ENTITLEMENT_REFERENCE
    );
    assert_eq!(
        initial_payload.entitlement_sha256,
        format!("sha256:{}", "d".repeat(64))
    );
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &initial_claim, &adapter)
            .await
            .expect("publish initial generation"),
        OwnerEquityRunOutcome::Published
    );

    let schedule_pins = OwnerEquitySchedulePins::new(
        CODE_COMMIT.to_owned(),
        WP5_SCHEDULE_ENTITLEMENT_REFERENCE.to_owned(),
        format!("sha256:{}", "d".repeat(64)),
    )
    .expect("valid scheduler pins");
    seed_schedule_calendar(&harness, prior_close, "TRADING").await;

    let bad_rights_pins = OwnerEquitySchedulePins::new(
        CODE_COMMIT.to_owned(),
        "krx-unregistered-fixture".to_owned(),
        format!("sha256:{}", "d".repeat(64)),
    )
    .expect("syntactically valid but unregistered rights pin");
    assert!(matches!(
        run_owner_equity_schedule_cycle(&worker_pool, &bad_rights_pins, at_close(eligible)).await,
        Err(OwnerEquityScheduleError::Database)
    ));

    let first = run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible))
        .await
        .expect("schedule prior confirmed close");
    assert_eq!(first.as_of, prior_close);
    assert_eq!(first.scheduled, 1);

    // The current eligible date becomes available later during this same
    // civil key. The older incremental remains active and blocks duplication.
    seed_schedule_calendar(&harness, eligible, "TRADING").await;
    let blocked = run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible))
        .await
        .expect("recheck after late calendar publication");
    assert_eq!(blocked.as_of, eligible);
    assert_eq!(blocked.scheduled, 0);
    let active_incrementals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.jobs \
          WHERE owner_user_id = $1 AND job_type = $2 \
            AND payload_json ->> 'action' = 'INCREMENTAL' \
            AND status IN ('QUEUED', 'RUNNING')",
    )
    .bind(harness.owner.user_id)
    .bind(OWNER_EQUITY_V2_JOB_TYPE)
    .fetch_one(&worker_pool)
    .await
    .expect("active incremental count");
    assert_eq!(active_incrementals, 1);

    let older_claim = queue
        .claim_next_for("wp5-older-incremental", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim older incremental")
        .expect("older incremental remains queued");
    let older_payload: OwnerEquityJobPayload =
        serde_json::from_value(older_claim.job.payload_json.clone())
            .expect("older incremental payload");
    assert_eq!(older_payload.action, OwnerEquityJobAction::Incremental);
    assert_eq!(
        older_payload.requested_through.to_string(),
        prior_close.to_string()
    );
    assert_eq!(older_payload.code_commit, CODE_COMMIT);
    assert_eq!(
        older_payload.entitlement_reference,
        WP5_SCHEDULE_ENTITLEMENT_REFERENCE
    );
    assert_eq!(
        older_payload.entitlement_sha256,
        format!("sha256:{}", "d".repeat(64))
    );
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &older_claim, &adapter)
            .await
            .expect("settle older incremental"),
        OwnerEquityRunOutcome::Published
    );

    let (first_poll, concurrent_poll) = tokio::join!(
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible)),
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible)),
    );
    let first_poll = first_poll.expect("schedule after older job settles");
    let concurrent_poll = concurrent_poll.expect("concurrent same-key schedule");
    assert_eq!(first_poll.as_of, eligible);
    assert_eq!(concurrent_poll.as_of, eligible);
    let mut inserted = [first_poll.scheduled, concurrent_poll.scheduled];
    inserted.sort();
    assert_eq!(inserted, [0, 1]);
    let duplicate_poll =
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible))
            .await
            .expect("repeat same-key active poll");
    assert_eq!(duplicate_poll.scheduled, 0);

    let current_claim = queue
        .claim_next_for("wp5-current-incremental", OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .expect("claim current incremental")
        .expect("current incremental queued");
    let current_payload: OwnerEquityJobPayload =
        serde_json::from_value(current_claim.job.payload_json.clone())
            .expect("current incremental payload");
    assert_eq!(current_payload.action, OwnerEquityJobAction::Incremental);
    assert_eq!(
        current_payload.requested_through.to_string(),
        eligible.to_string()
    );
    assert_eq!(current_payload.code_commit, CODE_COMMIT);
    assert_eq!(
        current_payload.entitlement_reference,
        WP5_SCHEDULE_ENTITLEMENT_REFERENCE
    );
    assert_eq!(
        current_payload.entitlement_sha256,
        format!("sha256:{}", "d".repeat(64))
    );
    assert_eq!(
        process_owner_equity_claim(&worker_pool, &queue, &current_claim, &adapter)
            .await
            .expect("publish current generation"),
        OwnerEquityRunOutcome::Published
    );

    let complete =
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible))
            .await
            .expect("current generation has no missing coverage");
    assert_eq!(complete.as_of, eligible);
    assert_eq!(complete.scheduled, 0);
    let admitted_current_coverage: NaiveDate = sqlx::query_scalar(
        "SELECT generation.last_session \
           FROM public.owner_equity_instrument_generations AS generation \
           JOIN public.owner_equity_generation_admissions AS admission \
             ON admission.generation_id = generation.id \
            AND admission.owner_user_id = generation.owner_user_id \
            AND admission.membership_id = generation.membership_id \
            AND admission.instrument_id = generation.instrument_id \
            AND admission.generation = generation.generation \
          WHERE generation.owner_user_id = $1 AND generation.membership_id = $2 \
          ORDER BY generation.generation DESC LIMIT 1",
    )
    .bind(harness.owner.user_id)
    .bind(add.membership.id)
    .fetch_one(&worker_pool)
    .await
    .expect("latest fully admitted coverage");
    assert_eq!(admitted_current_coverage, eligible);

    // Simulate an incomplete immutable lineage row: the highest stored
    // generation has no admission. The real scheduler must continue to use
    // the latest generation joined on all admission identity columns.
    let unadmitted_generation_id = Uuid::new_v4();
    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "INSERT INTO public.owner_equity_instrument_generations \
                 (id, membership_id, owner_user_id, instrument_id, generation, \
                  target_observed_sessions, minimum_observed_sessions, observed_sessions, \
                  first_session, last_session) \
                 VALUES ('{unadmitted_generation_id}', '{}', '{}', '005930.KRX', 4, \
                         {TARGET_SESSIONS}, {MINIMUM_SESSIONS}, 121, \
                         DATE '{initial_session}', DATE '{prior_close}')",
                add.membership.id, harness.owner.user_id
            ),
        )
        .await;
    let unadmitted_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_equity_instrument_generations AS generation \
           LEFT JOIN public.owner_equity_generation_admissions AS admission \
             ON admission.generation_id = generation.id \
            AND admission.owner_user_id = generation.owner_user_id \
            AND admission.membership_id = generation.membership_id \
            AND admission.instrument_id = generation.instrument_id \
            AND admission.generation = generation.generation \
          WHERE generation.id = $1 AND admission.generation_id IS NULL",
    )
    .bind(unadmitted_generation_id)
    .fetch_one(&worker_pool)
    .await
    .expect("unadmitted lineage count");
    assert_eq!(unadmitted_rows, 1);
    let no_lineage =
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(eligible))
            .await
            .expect("scheduler ignores the unadmitted generation");
    assert_eq!(no_lineage.as_of, eligible);
    assert_eq!(no_lineage.scheduled, 0);

    let closed_day = eligible.succ_opt().expect("closed fixture date");
    seed_schedule_calendar(&harness, closed_day, "CLOSED").await;
    let closed =
        run_owner_equity_schedule_cycle(&worker_pool, &schedule_pins, at_close(closed_day))
            .await
            .expect("closed day recheck");
    assert_eq!(closed.as_of, eligible);
    assert_eq!(closed.scheduled, 0);

    let total_incrementals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.jobs \
          WHERE owner_user_id = $1 AND job_type = $2 \
            AND payload_json ->> 'action' = 'INCREMENTAL'",
    )
    .bind(harness.owner.user_id)
    .bind(OWNER_EQUITY_V2_JOB_TYPE)
    .fetch_one(&worker_pool)
    .await
    .expect("incremental job count");
    assert_eq!(total_incrementals, 2);
    for session_date in [prior_close, eligible] {
        let jobs_for_date: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.jobs \
              WHERE owner_user_id = $1 AND job_type = $2 \
                AND payload_json ->> 'action' = 'INCREMENTAL' \
                AND payload_json ->> 'requested_through' = $3",
        )
        .bind(harness.owner.user_id)
        .bind(OWNER_EQUITY_V2_JOB_TYPE)
        .bind(session_date.to_string())
        .fetch_one(&worker_pool)
        .await
        .expect("one job per confirmed close");
        assert_eq!(jobs_for_date, 1);
    }

    worker_pool.close().await;
    harness.teardown().await;
}
