//! Real SQL lifecycle regression. Executed only in the established isolated hosted harness.
use super::*;

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0069_restore_preserves_expired_claim_and_exact_quiescent_history() -> Daily0060Result
{
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let paper = enabled_paper(&db, false).await?;
        let (target, _) = auto_target(&db, &paper, false).await?;
        clock.set(initial + 86400).await?;
        let day: String = sqlx::query_scalar("SELECT effective_session::text FROM owner_beta_paper_sessions WHERE pending_target_id=$1")
            .bind(target).fetch_one(&db.owner).await?;
        let execution = seed_witness(&db, paper.config, &paper.cohort, &day,
            Some(&paper.witness.admission), false).await?;
        let packet = settlement_packet(&db, &paper, target, &execution.admission, false).await?;
        assert_eq!(settle(&db, &paper, target, &packet).await?["outcome"], "OK");
        let changed = sqlx::query("UPDATE paper_settlement_outbox SET claim_token=gen_random_uuid(),claim_expires_at=clock_timestamp()-interval '1 hour' WHERE pending_target_id=$1")
            .bind(target).execute(&db.owner).await?.rows_affected();
        assert_eq!(changed, 1, "the regression must retain an actual expired outbox claim");
        let original = operation_image(&db).await?;
        close_fixture_writer_connections(&db).await?;
        let mut connection = db.owner.acquire().await?;
        let off = stop_fixture_at(&mut connection, paper.account, 1).await?;
        assert_eq!(off["control_epoch"], 3);
        drop(connection);
        let source = operation_image(&db).await?;
        assert_eq!(source["paper_settlement_outbox"], original["paper_settlement_outbox"],
            "STOP must not rewrite an already expired claim timestamp");
        let (source_generation, source_installation): (Uuid, Uuid) = sqlx::query_as(
            "SELECT generation,installation_id FROM owner_beta_paper_operations WHERE account_id=$1 ORDER BY created_at DESC,fence_epoch DESC,generation DESC LIMIT 1")
            .bind(paper.account).fetch_one(&db.owner).await?;
        let generation = Uuid::new_v4();
        let installation = Uuid::new_v4();
        assert_ne!(installation, source_installation);
        let intent = json!({"schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":installation,"generation":generation,"account_id":paper.account,
            "predecessor_generation":source_generation,"request_key":format!("{generation}:begin"),
            "purpose":"RESTORE","scope":"COMBINED_BACKUP","expected_epoch":3,"local_epoch_floor":9,
            "release_manifest_sha256":hash('a'),"artifact_sha256":hash('b'),"resume_target_id":null});
        let begin: Value = sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1(3,$1,$2)")
            .bind(&intent).bind(format!("{generation}:begin")).fetch_one(&db.owner).await?;
        assert_eq!(begin["control_epoch"], 10);
        let before = operation_image(&db).await?;
        // A live claim cannot be repaired by RESTORE. Roll back only this
        // deliberately introduced fixture mutation; never normalize audit rows.
        let mut tx = db.owner.begin().await?;
        sqlx::query("UPDATE paper_settlement_outbox SET claim_expires_at=clock_timestamp()+interval '1 hour' WHERE pending_target_id=$1")
            .bind(target).execute(&mut *tx).await?;
        denied(sqlx::query_scalar::<_, Value>("SELECT reconcile_owner_beta_paper_operation_v1($1,10,$2,$3)")
            .bind(generation).bind(hash('c')).bind(format!("{generation}:reconcile:1"))
            .fetch_one(&mut *tx).await, "55000");
        tx.rollback().await?;
        assert_eq!(before, operation_image(&db).await?, "rejected restore left a delta");
        let key = format!("{generation}:reconcile:1");
        let reconciled: Value = sqlx::query_scalar("SELECT reconcile_owner_beta_paper_operation_v1($1,10,$2,$3)")
            .bind(generation).bind(hash('c')).bind(&key).fetch_one(&db.owner).await?;
        assert_eq!(reconciled["outcome"], "OK");
        assert_eq!(reconciled["data"]["summary"]["live_claims"], 0);
        let after = operation_image(&db).await?;
        for (table, rows) in before.as_object().ok_or("row image")? {
            if table != "owner_beta_paper_operation_observations" {
                assert_eq!(*rows, after[table], "unexpected reconciliation delta in {table}");
            }
        }
        assert_eq!(after["owner_beta_paper_operation_observations"].as_array().unwrap().len(),
            before["owner_beta_paper_operation_observations"].as_array().unwrap().len()+1);
        let q = exact_request("RECONCILE", Some(paper.account), json!({"generation":generation,
            "expected_epoch":10,"inventory_sha256":hash('c'),"request_key":key}));
        assert_exact_receipt(&db, &q, &reconciled).await?;
        denied(sqlx::query_scalar::<_, Value>("SELECT reconcile_owner_beta_paper_operation_v1($1,10,$2,$3)")
            .bind(generation).bind(hash('d')).bind(&key).fetch_one(&db.owner).await, "23505");
        assert_eq!(after, operation_image(&db).await?, "conflicting identity changed retained rows");
        let down = include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../../migrations/0069_owner_beta_paper_restore_reconciliation.down.sql"));
        let mut tx = db.owner.begin().await?;
        denied(sqlx::raw_sql(down).execute(&mut *tx).await, "55000");
        tx.rollback().await?;
        Ok(())
    }.await;
    db.close().await?;
    result
}
