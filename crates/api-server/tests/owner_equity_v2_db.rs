//! PostgreSQL-backed WP-4 contract for the owner-managed equity V2 flow.
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
use common::{Harness, UserCtx, actor_pool};
use domain::{
    BatchId, CodeCommit, ContentHash, InstrumentId, OwnerEquityUniverseHash, RetryDisposition,
    TradingDate,
};
use job_queue::owner_equity_v2::{
    AdmittedGenerationDescriptor, OWNER_EQUITY_V2_JOB_TYPE, OwnerEquityCoverage,
    OwnerEquityJobAction, OwnerEquityJobPayload, OwnerEquityMaterialization, OwnerEquityRunOutcome,
    OwnerEquityWorkFailure, OwnerEquityWorkerAdapter, OwnerEquityWorkerError,
    PreparedOwnerEquityGeneration, process_owner_equity_claim,
};
use job_queue::{AttemptOutcome, JobQueue, JobStatus, QueueConfig};
use market_data::owner_equity_v2::{
    OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OWNER_ONLY_WARNING,
    OwnerEquityBar, OwnerEquityCaptureKind, OwnerEquityGenerationCandidate, OwnerEquitySourcePins,
    PRICE_SEMANTICS, RESEARCH_ONLY_WARNING, STRICT_PIT_WARNING, VENDOR_SNAPSHOT_WARNING,
};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;

const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_REFERENCE: &str = "krx-2026-01";
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

async fn provision_policy(harness: &Harness, owner: &UserCtx, maximum: i32) {
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_universe_policies \
                 (owner_user_id, max_active_instruments, target_observed_sessions, \
                  minimum_observed_sessions) \
                 VALUES ('{}', {maximum}, {TARGET_SESSIONS}, {MINIMUM_SESSIONS})",
                owner.user_id
            ),
        )
        .await;
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
