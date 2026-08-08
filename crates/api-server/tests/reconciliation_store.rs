//! Todo 40, persistence half: runs, readiness, and surviving a restart.
//!
//! Named `reconciliation_*` so the plan's acceptance filter reaches these too.
//! `kis-client`'s suite proves the diff; this proves that readiness is derived
//! the same way the reconciler defines green, and that it survives a restart —
//! which for a stateless repo means: read back from the database, with nothing
//! carried in memory.

mod common;

use api_server::repos::reconciliation::{Readiness, ReconciliationRepo};
use common::{Harness, actor_pool};
use kis_client::reconciliation::{Mismatch, ReconciliationOutcome};
use uuid::Uuid;

fn repo(h: &Harness) -> ReconciliationRepo {
    ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id)
}

fn green() -> ReconciliationOutcome {
    ReconciliationOutcome {
        mismatches: vec![],
        fills_to_apply: vec![],
        lookups_required: vec![],
    }
}

fn with_mismatch() -> ReconciliationOutcome {
    ReconciliationOutcome {
        mismatches: vec![Mismatch::Position {
            instrument_id: "069500.KRX".into(),
            ours: 10,
            brokers: 12,
        }],
        fills_to_apply: vec![],
        lookups_required: vec![],
    }
}

#[tokio::test]
async fn reconciliation_an_account_that_never_reconciled_may_not_trade() {
    // FR-LIVE-004. A fresh install, a restored backup, and a process that
    // crashed before its first run all land here, and all must block --
    // "ready by default" would be exactly the wrong default.
    let Some(h) = Harness::new().await else {
        return;
    };
    let readiness = repo(&h).readiness(None).await.expect("readiness");
    assert_eq!(readiness, Readiness::NeverReconciled);
    assert!(!readiness.may_trade());
    assert_eq!(readiness.reason(), "NEVER_RECONCILED");
}

#[tokio::test]
async fn reconciliation_a_run_in_progress_blocks_rather_than_permits() {
    // The row is written BEFORE the work, so a crash mid-reconciliation
    // leaves RUNNING rather than no trace -- and RUNNING must block, or the
    // crash would be indistinguishable from never having needed a run.
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);
    let run = r.start(None, "STARTUP").await.expect("start");

    let readiness = r.readiness(None).await.expect("readiness");
    assert_eq!(readiness, Readiness::Running { run_id: run.id });
    assert!(!readiness.may_trade());
}

#[tokio::test]
async fn reconciliation_only_a_completed_zero_mismatch_run_permits_trading() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);

    let run = r.start(None, "STARTUP").await.expect("start");
    r.finish(run.id, &green(), None).await.expect("finish");

    let readiness = r.readiness(None).await.expect("readiness");
    assert_eq!(readiness, Readiness::Ready { run_id: run.id });
    assert!(readiness.may_trade());
}

#[tokio::test]
async fn reconciliation_a_mismatch_blocks_and_records_what_it_found() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);

    let run = r.start(None, "STARTUP").await.expect("start");
    let finished = r
        .finish(run.id, &with_mismatch(), Some("s3://reports/run-1.json"))
        .await
        .expect("finish");

    assert_eq!(finished.status, "FAILED");
    assert_eq!(finished.mismatch_count, 1);
    assert_eq!(
        finished.report_path.as_deref(),
        Some("s3://reports/run-1.json")
    );

    let readiness = r.readiness(None).await.expect("readiness");
    assert!(!readiness.may_trade());
    assert_eq!(readiness.reason(), "RECONCILIATION_MISMATCH");
}

#[tokio::test]
async fn reconciliation_readiness_follows_the_latest_run_not_the_best_one() {
    // A green run yesterday does not license trading after a red one today.
    // Taking "any passing run" would make the block trivially escapable by
    // waiting.
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);

    let good = r.start(None, "STARTUP").await.expect("start");
    r.finish(good.id, &green(), None).await.expect("finish");
    assert!(r.readiness(None).await.expect("readiness").may_trade());

    let bad = r.start(None, "SCHEDULED").await.expect("start");
    r.finish(bad.id, &with_mismatch(), None)
        .await
        .expect("finish");

    let readiness = r.readiness(None).await.expect("readiness");
    assert!(
        !readiness.may_trade(),
        "an earlier green run must not license trading after a later red one"
    );
    assert_eq!(
        readiness,
        Readiness::Blocked {
            run_id: bad.id,
            mismatch_count: 1
        }
    );
}

#[tokio::test]
async fn reconciliation_readiness_survives_a_restart_because_it_is_only_ever_read() {
    // "Restart" for a stateless repo means: build a completely new repo over
    // a new connection and ask again. Nothing is cached, so the answer comes
    // from the database or not at all.
    let Some(h) = Harness::new().await else {
        return;
    };
    let run = repo(&h).start(None, "STARTUP").await.expect("start");
    repo(&h)
        .finish(run.id, &with_mismatch(), None)
        .await
        .expect("finish");

    // A brand-new repo -- the post-restart process -- reaches the same
    // verdict, and it is still blocking.
    let after_restart =
        ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let readiness = after_restart.readiness(None).await.expect("readiness");
    assert!(!readiness.may_trade());
    assert_eq!(
        readiness,
        Readiness::Blocked {
            run_id: run.id,
            mismatch_count: 1
        }
    );
}

#[tokio::test]
async fn reconciliation_a_passed_run_that_recorded_mismatches_is_a_contradiction_and_blocks() {
    // Defence in depth against a writer that sets the status without the
    // count, or vice versa. Trusting the status alone would trade through a
    // recorded mismatch.
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);
    let run = r.start(None, "MANUAL").await.expect("start");
    r.finish(run.id, &green(), None).await.expect("finish");

    // Corrupt the row the way a partial write would: PASSED with a count.
    // Through an ACTOR pool: reconciliation_runs is a FORCE-RLS tenant table,
    // and a bare pool would affect zero rows, leaving this test asserting
    // nothing at all.
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let affected = sqlx::query("UPDATE reconciliation_runs SET mismatch_count = 3 WHERE id = $1")
        .bind(run.id)
        .execute(&pool)
        .await
        .expect("update")
        .rows_affected();
    assert_eq!(affected, 1, "the corruption must actually land");

    let readiness = r.readiness(None).await.expect("readiness");
    assert!(
        !readiness.may_trade(),
        "PASSED with a non-zero mismatch count must block, not trade"
    );
}

#[tokio::test]
async fn reconciliation_runs_are_scoped_per_connection() {
    // One connection's green run must not license another's trading.
    let Some(h) = Harness::new().await else {
        return;
    };
    let r = repo(&h);
    let other = Uuid::new_v4();

    let run = r.start(None, "STARTUP").await.expect("start");
    r.finish(run.id, &green(), None).await.expect("finish");

    assert!(r.readiness(None).await.expect("readiness").may_trade());
    assert_eq!(
        r.readiness(Some(other)).await.expect("readiness"),
        Readiness::NeverReconciled,
        "a different connection has its own readiness"
    );
}

#[tokio::test]
async fn reconciliation_maps_readiness_onto_the_gate_input_exactly_once() {
    use api_server::repos::reconciliation::gate_input;
    use kis_client::reconciliation::GateReconciliation;

    // The single mapping between readiness and Risk Gateway check 5. Two
    // mappings, or none, would let the reconciler and the gate disagree about
    // whether trading is allowed.
    let run_id = Uuid::new_v4();
    assert_eq!(
        gate_input(&Readiness::Ready { run_id }),
        GateReconciliation::Green
    );
    assert_eq!(
        gate_input(&Readiness::Blocked {
            run_id,
            mismatch_count: 2
        }),
        GateReconciliation::NotGreen
    );

    // Running is NOT Unknown. We know the state -- a run is in progress -- so
    // it is a policy denial (WARNING), not an absence of information. Grading
    // every overlap of a scheduled run with an order as CRITICAL would be
    // alarm noise that trains people to ignore the grade.
    assert_eq!(
        gate_input(&Readiness::Running { run_id }),
        GateReconciliation::NotGreen
    );

    // NeverReconciled IS an absence of information, and deliberately CRITICAL:
    // an account with no established relationship to the broker is the exact
    // situation FR-LIVE-004 exists to stop.
    assert_eq!(
        gate_input(&Readiness::NeverReconciled),
        GateReconciliation::Unknown
    );

    // Nothing but Ready permits trading, on either side of the mapping.
    for r in [
        Readiness::Blocked {
            run_id,
            mismatch_count: 1,
        },
        Readiness::Running { run_id },
        Readiness::NeverReconciled,
    ] {
        assert!(!r.may_trade());
        assert_ne!(gate_input(&r), GateReconciliation::Green);
    }
}
