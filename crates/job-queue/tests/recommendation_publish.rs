use chrono::NaiveDate;
use job_queue::recommendation::child::{
    ConstraintSummary, ExclusionRow, Reason, TargetChildOutput, TargetProvenance, TargetRow,
};
use job_queue::recommendation::compute::AttestedUniverse;
use job_queue::recommendation::input::{AttestedDataset, AttestedDatasetStatus};
use job_queue::recommendation::input::{
    AttestedRecommendationInput, DatasetPin, RecommendationPayload,
};
use job_queue::recommendation::publish::{PublicationOutcome, publish_recommendation};
use job_queue::recommendation::validate::{
    RecommendationValidationError, canonical_portfolio_snapshot_id, validate_target_output,
};
use job_queue::resolver::ResolvedConfig;
use job_queue::types::ErrorClass;
use job_queue::{JobQueue, QueueConfig, SubmitJob};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

const DATASET_VERSION_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

fn universe() -> AttestedUniverse {
    AttestedUniverse::from_manifest_yaml(include_str!(
        "../../../configs/universes/kr-etf-core-v1.yaml"
    ))
    .expect("shipped universe")
}

fn dataset() -> AttestedDataset {
    AttestedDataset {
        id: Uuid::parse_str(DATASET_VERSION_ID).unwrap(),
        dataset_id: "krx_eod_bars".into(),
        version: "phase0-v2".into(),
        curated_version: 2,
        status: AttestedDatasetStatus::Ready,
        manifest_sha256: "c".repeat(64),
        storage_path: "curated/phase0".into(),
    }
}

fn reason(code: &str) -> Reason {
    Reason {
        code: code.into(),
        params: BTreeMap::new(),
        text_ko: "제외".into(),
        text_en: "Excluded".into(),
    }
}

fn all_cash_output() -> TargetChildOutput {
    let u = universe();
    let mut output = TargetChildOutput {
        as_of: "2026-08-11".into(),
        strategy_version: "dual_momentum@1.0.0".into(),
        universe_snapshot_id: u.snapshot_id,
        factor_snapshot_hash: format!("sha256:{}", "b".repeat(64)),
        dataset_version_id: Uuid::parse_str(DATASET_VERSION_ID).unwrap(),
        dataset_id: "krx_eod_bars".into(),
        dataset_version: "phase0-v2".into(),
        curated_version: 2,
        dataset_manifest_sha256: "c".repeat(64),
        targets: vec![],
        exclusions: u
            .members
            .into_iter()
            .map(|instrument_id| ExclusionRow {
                instrument_id,
                reasons: vec![reason("EXCLUDED_MANDATORY_FACTOR_NULL")],
            })
            .collect(),
        cash_weight: 1.0,
        constraints: ConstraintSummary {
            top_n: 1,
            max_weight: 1.0,
            cash_floor: 0.0,
            weight_scale: 4,
            tolerance: 1e-9,
        },
        portfolio_reasons: vec![reason("ALL_CASH_NO_ELIGIBLE")],
        portfolio_snapshot_id: String::new(),
    };
    output.portfolio_snapshot_id = canonical_portfolio_snapshot_id(&output).unwrap();
    output
}

fn provenance() -> TargetProvenance {
    let u = universe();
    TargetProvenance {
        dataset_version_id: Uuid::parse_str(DATASET_VERSION_ID).unwrap(),
        dataset_id: "krx_eod_bars".into(),
        dataset_version: "phase0-v2".into(),
        curated_version: 2,
        dataset_manifest_sha256: "c".repeat(64),
        universe_snapshot_id: u.snapshot_id,
        factor_snapshot_hash: format!("sha256:{}", "b".repeat(64)),
    }
}

fn validate(output: TargetChildOutput) -> Result<(), RecommendationValidationError> {
    validate_target_output(
        output,
        "dual_momentum",
        "1.0.0",
        "2026-08-11",
        &universe(),
        &dataset(),
        &provenance(),
    )
    .map(|_| ())
}

fn rehash(output: &mut TargetChildOutput) {
    output.portfolio_snapshot_id = canonical_portfolio_snapshot_id(output).unwrap();
}

#[test]
fn validates_child_output_and_accepts_explicit_all_cash() {
    let output = all_cash_output();
    let validated = validate_target_output(
        output,
        "dual_momentum",
        "1.0.0",
        "2026-08-11",
        &universe(),
        &dataset(),
        &provenance(),
    )
    .expect("explicit all-cash is valid");
    assert_eq!(validated.items().len(), 11);
    assert_eq!(validated.selected_count(), 0);
    assert_eq!(validated.excluded_count(), 11);
    assert_eq!(validated.cash_weight(), "1.000000");
    assert!(validated.items().iter().all(|item| item.excluded()));
}

#[test]
fn all_cash_must_be_explicit_and_selected_targets_must_be_positive() {
    let mut implicit = all_cash_output();
    implicit.portfolio_reasons.clear();
    rehash(&mut implicit);
    assert_eq!(
        validate(implicit).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut zero_target = all_cash_output();
    let member = zero_target.exclusions.remove(0).instrument_id;
    zero_target.targets.push(TargetRow {
        instrument_id: member,
        rank: 1,
        score: 0.5,
        factors: BTreeMap::new(),
        target_weight: 0.0,
        reasons: vec![reason("SELECTED_TOP_N")],
    });
    rehash(&mut zero_target);
    assert_eq!(
        validate(zero_target).unwrap_err().class(),
        ErrorClass::Integrity
    );
}

#[test]
fn canonical_hash_matches_python_sorted_json_semantics() {
    let output = all_cash_output();
    assert_eq!(
        canonical_portfolio_snapshot_id(&output).unwrap(),
        "sha256:7f52444f0d5770c849f9e06038e7f2996229a0e98a4a0df82f1b625e3859d3e4"
    );
    assert!(output.portfolio_snapshot_id.starts_with("sha256:"));
    assert_eq!(output.portfolio_snapshot_id.len(), 71);
}

#[test]
fn validator_rejects_wrong_identity_and_provenance_as_integrity() {
    let mut cases = Vec::new();
    let mut wrong_strategy = all_cash_output();
    wrong_strategy.strategy_version = "relative_momentum@1.0.0".into();
    rehash(&mut wrong_strategy);
    cases.push(wrong_strategy);
    let mut wrong_date = all_cash_output();
    wrong_date.as_of = "2026-08-10".into();
    rehash(&mut wrong_date);
    cases.push(wrong_date);
    let mut wrong_manifest = all_cash_output();
    wrong_manifest.dataset_manifest_sha256 = "d".repeat(64);
    rehash(&mut wrong_manifest);
    cases.push(wrong_manifest);
    let mut wrong_factor = all_cash_output();
    wrong_factor.factor_snapshot_hash = format!("sha256:{}", "d".repeat(64));
    rehash(&mut wrong_factor);
    cases.push(wrong_factor);

    for output in cases {
        let error = validate(output).expect_err("mismatch must fail");
        assert_eq!(error.class(), ErrorClass::Integrity);
        assert!(!error.class().retryable());
    }
}

#[test]
fn validator_rejects_changed_hash_as_determinism() {
    let mut output = all_cash_output();
    output.portfolio_snapshot_id = format!("sha256:{}", "0".repeat(64));
    let error = validate(output).expect_err("changed child hash must fail");
    assert_eq!(error.class(), ErrorClass::Determinism);
    assert_eq!(error.code(), "RECOMMENDATION_PORTFOLIO_HASH_MISMATCH");
}

#[test]
fn validator_rejects_foreign_duplicate_and_cross_list_instruments_but_normalizes_missing() {
    let mut foreign = all_cash_output();
    foreign.exclusions[0].instrument_id = "SPY.XNAS".into();
    rehash(&mut foreign);
    assert_eq!(
        validate(foreign).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut duplicate = all_cash_output();
    duplicate.exclusions[1].instrument_id = duplicate.exclusions[0].instrument_id.clone();
    rehash(&mut duplicate);
    assert_eq!(
        validate(duplicate).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut missing = all_cash_output();
    missing.exclusions.pop();
    rehash(&mut missing);
    let normalized = validate_target_output(
        missing,
        "dual_momentum",
        "1.0.0",
        "2026-08-11",
        &universe(),
        &dataset(),
        &provenance(),
    )
    .expect("omitted canonical members are normalized at the Rust boundary");
    assert_eq!(normalized.items().len(), 11);
    assert_eq!(normalized.excluded_count(), 11);
    assert!(
        normalized
            .items()
            .iter()
            .any(|item| item.reason_codes() == &serde_json::json!(["NOT_SELECTED_BY_STRATEGY"]))
    );

    let mut cross_list = all_cash_output();
    let instrument_id = cross_list.exclusions[0].instrument_id.clone();
    cross_list.targets.push(TargetRow {
        instrument_id,
        rank: 1,
        score: 0.5,
        factors: BTreeMap::new(),
        target_weight: 0.0,
        reasons: vec![reason("SELECTED_TOP_N")],
    });
    rehash(&mut cross_list);
    assert_eq!(
        validate(cross_list).unwrap_err().class(),
        ErrorClass::Integrity
    );
}

#[test]
fn five_strategy_shapes_normalize_to_exactly_eleven_database_items() {
    for strategy_id in [
        "buy_and_hold",
        "trend_following",
        "relative_momentum",
        "dual_momentum",
        "inverse_volatility",
    ] {
        let mut output = all_cash_output();
        output.strategy_version = format!("{strategy_id}@1.0.0");
        output.exclusions.clear();
        rehash(&mut output);
        let validated = validate_target_output(
            output,
            strategy_id,
            "1.0.0",
            "2026-08-11",
            &universe(),
            &dataset(),
            &provenance(),
        )
        .unwrap_or_else(|error| panic!("{strategy_id} shape must normalize: {error}"));
        assert_eq!(validated.items().len(), 11, "{strategy_id}");
        assert_eq!(validated.excluded_count(), 11, "{strategy_id}");
        assert_eq!(validated.selected_count(), 0, "{strategy_id}");
    }
}

#[test]
fn validator_rejects_invalid_weights_constraints_ranks_reasons_and_factor_ids() {
    let member = universe().members[0].clone();
    let mut invalid_weight = all_cash_output();
    invalid_weight.exclusions.remove(0);
    invalid_weight.targets.push(TargetRow {
        instrument_id: member.clone(),
        rank: 1,
        score: 0.5,
        factors: BTreeMap::from([("return_6m".into(), 0.5)]),
        target_weight: -0.1,
        reasons: vec![reason("SELECTED_TOP_N")],
    });
    invalid_weight.cash_weight = 1.1;
    rehash(&mut invalid_weight);
    assert_eq!(
        validate(invalid_weight).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut wrong_sum = all_cash_output();
    wrong_sum.cash_weight = 0.9;
    rehash(&mut wrong_sum);
    assert_eq!(
        validate(wrong_sum).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut invalid_constraints = all_cash_output();
    invalid_constraints.constraints.tolerance = 0.1;
    rehash(&mut invalid_constraints);
    assert_eq!(
        validate(invalid_constraints).unwrap_err().class(),
        ErrorClass::Input
    );

    let mut over_top_n = all_cash_output();
    over_top_n.constraints.top_n = 1;
    for rank in 1..=2 {
        let member = over_top_n.exclusions.remove(0).instrument_id;
        over_top_n.targets.push(TargetRow {
            instrument_id: member,
            rank,
            score: 0.5,
            factors: BTreeMap::new(),
            target_weight: 0.5,
            reasons: vec![reason("SELECTED_TOP_N")],
        });
    }
    over_top_n.cash_weight = 0.0;
    rehash(&mut over_top_n);
    assert_eq!(validate(over_top_n).unwrap_err().class(), ErrorClass::Input);

    let mut duplicate_reason = all_cash_output();
    duplicate_reason.exclusions[0]
        .reasons
        .push(reason("EXCLUDED_MANDATORY_FACTOR_NULL"));
    rehash(&mut duplicate_reason);
    assert_eq!(
        validate(duplicate_reason).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut missing_target_reason = all_cash_output();
    let missing_reason_member = missing_target_reason.exclusions.remove(0).instrument_id;
    missing_target_reason.targets.push(TargetRow {
        instrument_id: missing_reason_member,
        rank: 1,
        score: 1.0,
        factors: BTreeMap::new(),
        target_weight: 1.0,
        reasons: vec![],
    });
    missing_target_reason.cash_weight = 0.0;
    rehash(&mut missing_target_reason);
    assert_eq!(
        validate(missing_target_reason).unwrap_err().class(),
        ErrorClass::Integrity
    );

    let mut invalid_factor = all_cash_output();
    invalid_factor.exclusions.remove(0);
    invalid_factor.targets.push(TargetRow {
        instrument_id: member,
        rank: 0,
        score: 0.5,
        factors: BTreeMap::from([("INVALID-FACTOR".into(), 0.5)]),
        target_weight: 0.0,
        reasons: vec![reason("SELECTED_TOP_N")],
    });
    invalid_factor.cash_weight = 1.0;
    rehash(&mut invalid_factor);
    assert_eq!(
        validate(invalid_factor).unwrap_err().class(),
        ErrorClass::Integrity
    );
}

#[test]
fn typed_child_boundary_rejects_duplicate_factor_and_reason_parameter_ids() {
    let mut output = all_cash_output();
    output.exclusions[0].reasons[0]
        .params
        .insert("factor".into(), "return_6m".into());
    let encoded = serde_json::to_string(&output).unwrap();
    let duplicate_params = encoded.replacen(
        "\"params\":{\"factor\":\"return_6m\"}",
        "\"params\":{\"factor\":\"return_6m\",\"factor\":\"vol_120\"}",
        1,
    );
    assert_ne!(encoded, duplicate_params);
    assert!(serde_json::from_str::<TargetChildOutput>(&duplicate_params).is_err());

    let mut output = all_cash_output();
    let member = output.exclusions.remove(0).instrument_id;
    output.targets.push(TargetRow {
        instrument_id: member,
        rank: 1,
        score: 0.5,
        factors: BTreeMap::from([("return_6m".into(), 0.5)]),
        target_weight: 0.0,
        reasons: vec![reason("SELECTED_TOP_N")],
    });
    let encoded = serde_json::to_string(&output).unwrap();
    let duplicate_factors = encoded.replacen(
        "\"factors\":{\"return_6m\":0.5}",
        "\"factors\":{\"return_6m\":0.5,\"return_6m\":0.6}",
        1,
    );
    assert_ne!(encoded, duplicate_factors);
    assert!(serde_json::from_str::<TargetChildOutput>(&duplicate_factors).is_err());
}

#[tokio::test]
async fn transaction_aware_settlement_is_guarded_and_does_not_nest() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("publish-settle-{}", Uuid::new_v4()))
    .bind(format!("publish-settle-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(30),
            backoff_base: Duration::from_millis(10),
        },
    );
    let job = JobQueue::new(db.pool.clone(), None, QueueConfig::default())
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::json!({}),
            priority: 0,
            idempotency_key: None,
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    let claim = queue
        .claim_next_for("recommendation-publisher", "recommendation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.job.id, job.id);

    let mut tx = worker.begin().await.unwrap();
    queue
        .settle_success_in(&mut tx, &claim)
        .await
        .expect("settles inside caller transaction");
    tx.rollback().await.unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(job.id)
        .fetch_one(&worker)
        .await
        .unwrap();
    assert_eq!(status, "RUNNING", "outer rollback owns atomicity");

    sqlx::query("UPDATE jobs SET locked_at = now() - interval '1 minute' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut tx = worker.begin().await.unwrap();
    let error = queue
        .settle_success_in(&mut tx, &claim)
        .await
        .expect_err("expired claim is stale");
    assert!(matches!(error, job_queue::QueueError::StaleClaim(id) if id == job.id));
    tx.rollback().await.unwrap();
    let attempt: String = sqlx::query_scalar(
        "SELECT outcome FROM job_attempts WHERE job_id = $1 AND attempt_no = $2",
    )
    .bind(job.id)
    .bind(claim.attempt.attempt_no)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert_eq!(attempt, "RUNNING");

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn publishes_exactly_eleven_rows_and_settles_everything_atomically() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("publisher-{}", Uuid::new_v4()))
    .bind(format!("publisher-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for member in &universe().members {
        let symbol = member.trim_end_matches(".KRX");
        sqlx::query(
            "INSERT INTO instruments (id, symbol, venue, currency) VALUES ($1, $2, 'KRX', 'KRW')",
        )
        .bind(member)
        .bind(symbol)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO strategies (id, display_name, state) VALUES ('dual_momentum', 'Dual momentum', 'Paper')")
        .execute(&db.pool)
        .await
        .unwrap();
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs (owner_user_id, strategy_id, strategy_version, config_json) VALUES ($1, 'dual_momentum', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let dataset_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions (id, dataset_id, version, status, manifest_sha256, storage_path) VALUES ($1, 'krx_eod_bars', 'phase0-v2', 'READY', repeat('c', 64), 'curated/phase0') RETURNING id",
    )
    .bind(Uuid::parse_str(DATASET_VERSION_ID).unwrap())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let u = universe();
    sqlx::query(
        "INSERT INTO universe_snapshots (snapshot_id, universe_manifest_sha256, instruments_json, published_by) VALUES ($1, repeat('d', 64), $2, $3)",
    )
    .bind(&u.snapshot_id)
    .bind(serde_json::json!(u.members))
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let run_id = Uuid::new_v4();
    let payload = RecommendationPayload {
        run_id,
        strategy_config_id: config_id,
        as_of: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
        dataset: DatasetPin {
            id: dataset_id,
            dataset_id: "krx_eod_bars".into(),
            version: "phase0-v2".into(),
            curated_version: 2,
            manifest_sha256: "c".repeat(64),
        },
    };
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::to_value(&payload).unwrap(),
            priority: 0,
            idempotency_key: Some(format!("publish-{run_id}")),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO recommendation_runs (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, repeat('c', 64))",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(payload.as_of)
    .bind(job.id)
    .bind(dataset_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = JobQueue::new(worker.clone(), None, QueueConfig::default());
    let claim = queue
        .claim_next_for("recommendation-publisher", "recommendation")
        .await
        .unwrap()
        .unwrap();
    let attested = AttestedRecommendationInput {
        payload,
        resolved_config: ResolvedConfig {
            strategy_id: "dual_momentum".into(),
            strategy_version: "1.0.0".into(),
            config: serde_json::json!({}),
        },
        dataset: dataset(),
    };
    let mut output = all_cash_output();
    let selected = output.exclusions.remove(0).instrument_id;
    output.targets.push(TargetRow {
        instrument_id: selected,
        rank: 1,
        score: 0.25,
        factors: BTreeMap::from([("return_6m".into(), 0.25)]),
        target_weight: 1.0,
        reasons: vec![reason("ABSOLUTE_MOMENTUM_PASSED")],
    });
    output.cash_weight = 0.0;
    output.portfolio_reasons.clear();
    rehash(&mut output);
    let validated = validate_target_output(
        output,
        "dual_momentum",
        "1.0.0",
        "2026-08-11",
        &u,
        &attested.dataset,
        &provenance(),
    )
    .unwrap();

    let mut wrong_owner = claim.clone();
    wrong_owner.job.owner_user_id = Uuid::new_v4();
    publish_recommendation(&worker, &queue, &wrong_owner, &attested, &u, &validated)
        .await
        .expect_err("wrong owner cannot publish");
    assert_unpublished(&worker, run_id, job.id, claim.attempt.attempt_no).await;

    sqlx::query("UPDATE jobs SET locked_at = now() - interval '2 minutes' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
        .await
        .expect_err("expired lease cannot publish");
    assert_unpublished(&worker, run_id, job.id, claim.attempt.attempt_no).await;
    sqlx::query("UPDATE jobs SET locked_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();

    sqlx::query("UPDATE recommendation_runs SET status = 'FAILED' WHERE id = $1")
        .bind(run_id)
        .execute(&db.pool)
        .await
        .unwrap();
    publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
        .await
        .expect_err("stale run cannot publish");
    sqlx::query("UPDATE recommendation_runs SET status = 'PENDING' WHERE id = $1")
        .bind(run_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_unpublished(&worker, run_id, job.id, claim.attempt.attempt_no).await;

    sqlx::query("UPDATE jobs SET status = 'CANCELED' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
        .await
        .expect_err("canceled claim cannot publish");
    sqlx::query("UPDATE jobs SET status = 'RUNNING', locked_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_unpublished(&worker, run_id, job.id, claim.attempt.attempt_no).await;

    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION test_fail_recommendation_publish() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected publication failure'; END $$",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    for (create_trigger, drop_trigger) in [
        (
            "CREATE TRIGGER test_fail_items BEFORE INSERT ON recommendation_items FOR EACH STATEMENT EXECUTE FUNCTION test_fail_recommendation_publish()",
            "DROP TRIGGER test_fail_items ON recommendation_items",
        ),
        (
            "CREATE TRIGGER test_fail_portfolio BEFORE INSERT ON target_portfolios FOR EACH STATEMENT EXECUTE FUNCTION test_fail_recommendation_publish()",
            "DROP TRIGGER test_fail_portfolio ON target_portfolios",
        ),
        (
            "CREATE TRIGGER test_fail_run BEFORE UPDATE ON recommendation_runs FOR EACH ROW WHEN (NEW.status = 'SUCCEEDED') EXECUTE FUNCTION test_fail_recommendation_publish()",
            "DROP TRIGGER test_fail_run ON recommendation_runs",
        ),
        (
            "CREATE TRIGGER test_fail_job BEFORE UPDATE ON jobs FOR EACH ROW WHEN (NEW.status = 'SUCCEEDED') EXECUTE FUNCTION test_fail_recommendation_publish()",
            "DROP TRIGGER test_fail_job ON jobs",
        ),
    ] {
        sqlx::raw_sql(create_trigger)
            .execute(&db.pool)
            .await
            .unwrap();
        publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
            .await
            .expect_err("injected boundary failure rolls back");
        assert_unpublished(&worker, run_id, job.id, claim.attempt.attempt_no).await;
        sqlx::raw_sql(drop_trigger).execute(&db.pool).await.unwrap();
    }

    let first = publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
        .await
        .expect("publication succeeds");
    assert_eq!(first, PublicationOutcome::Published);
    let second = publish_recommendation(&worker, &queue, &claim, &attested, &u, &validated)
        .await
        .expect("identical retry observes committed result");
    assert_eq!(second, PublicationOutcome::AlreadyPublished);

    let (items, portfolios): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = $1), (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1)",
    )
    .bind(run_id)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert_eq!((items, portfolios), (11, 1));
    let (distinct_items, selected_items, excluded_items): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT instrument_id), count(*) FILTER (WHERE excluded = false), \
                count(*) FILTER (WHERE excluded = true) \
         FROM recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert_eq!(
        (distinct_items, selected_items, excluded_items),
        (11, 1, 10)
    );
    let summary: serde_json::Value =
        sqlx::query_scalar("SELECT summary_json FROM recommendation_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&worker)
            .await
            .unwrap();
    assert_eq!(summary["dataset_version"], "phase0-v2");
    assert_eq!(summary["curated_version"], 2);
    assert_eq!(summary["manifest_sha256"], "c".repeat(64));
    assert_eq!(summary["universe_snapshot_id"], u.snapshot_id);
    assert_eq!(
        summary["factor_snapshot_hash"],
        format!("sha256:{}", "b".repeat(64))
    );
    assert_eq!(
        summary["portfolio_snapshot_id"],
        validated.portfolio_snapshot_id()
    );
    assert_eq!(summary["selected_count"], 1);
    assert_eq!(summary["excluded_count"], 10);
    assert_eq!(summary["cash_weight"], "0.000000");
    assert_eq!(summary["trigger_kind"], "MANUAL");
    assert_eq!(summary["warnings"], serde_json::json!([]));
    let (run_status, job_status, attempt_outcome): (String, String, String) = sqlx::query_as(
        "SELECT r.status, j.status, a.outcome FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id JOIN job_attempts a ON a.job_id = j.id WHERE r.id = $1 AND a.attempt_no = $2",
    )
    .bind(run_id)
    .bind(claim.attempt.attempt_no)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert_eq!(
        (
            run_status.as_str(),
            job_status.as_str(),
            attempt_outcome.as_str()
        ),
        ("SUCCEEDED", "SUCCEEDED", "SUCCEEDED")
    );

    worker.close().await;
    db.drop_db().await;
}

async fn assert_unpublished(pool: &PgPool, run_id: Uuid, job_id: Uuid, attempt_no: i32) {
    let (items, portfolios, run_status, job_status, attempt): (i64, i64, String, String, String) =
        sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = $1), \
             (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1), \
             (SELECT status FROM recommendation_runs WHERE id = $1), \
             (SELECT status FROM jobs WHERE id = $2), \
             (SELECT outcome FROM job_attempts WHERE job_id = $2 AND attempt_no = $3)",
        )
        .bind(run_id)
        .bind(job_id)
        .bind(attempt_no)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!((items, portfolios), (0, 0));
    assert_eq!(run_status, "PENDING");
    assert_eq!(job_status, "RUNNING");
    assert_eq!(attempt, "RUNNING");
}
mod common;

use common::ScratchDb;
