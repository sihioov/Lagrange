use super::*;
use serde_json::{Value, json};

const UP: &str = include_str!("../../../../migrations/0070_owner_beta_daily_dividend_cells.up.sql");
const DOWN: &str =
    include_str!("../../../../migrations/0070_owner_beta_daily_dividend_cells.down.sql");

const SYMBOL: &str = "069500";
const DATE: &str = "2026-08-31";

fn sessions(input: &mut Value) -> &mut Value {
    &mut input["manifest"]["daily_sources"][0]
}

fn set_selected(input: &mut Value, count: u64) {
    let documents = sessions(input)["normalized_documents"]
        .as_array_mut()
        .expect("fixed normalized documents fixture");
    for document in documents {
        if document["kind"] == "corporate_actions" {
            document["row_count"] = json!(count);
        }
    }
}

fn cash_row(page: &Value, amount: &str) -> Value {
    json!({
        "symbol": SYMBOL, "record_date": DATE, "dividend_kind": "01",
        "face_value": "1000", "cash_dividend_amount": amount,
        "cash_dividend_rate": "0", "stock_dividend_rate": "0", "cash_payment_date": null,
        "stock_payment_date": null, "odd_lot_payment_date": null, "stock_kind": "01",
        "high_dividend_flag": "0", "source_file": daily_0060_file_key_fixture(page)
    })
}

fn append_overlap(input: &mut Value) -> Daily0060Result {
    let session = sessions(input);
    let actions = session["actions"].as_array_mut().ok_or("fixture actions")?;
    let original = &mut actions[3];
    assert_eq!(original["logical_class"], "dividend");

    let scoped_page = daily_0060_file_fixture(
        "corporate_actions",
        "2026-08-29",
        "dividend-scope.json",
        "ksdinfo/dividend",
        "HHKDB669102C0",
        vec![
            ("CTS", String::new()),
            ("F_DT", "20260829".into()),
            ("T_DT", "20260831".into()),
            ("SHT_CD", SYMBOL.into()),
            ("GB1", "0".into()),
            ("HIGH_GB", String::new()),
        ],
    );
    original["cash_dividends"] = json!([cash_row(&original["pages"][0], "25")]);
    let mut scoped = original.clone();
    scoped["scope"] = json!("etf11_symbol");
    scoped["symbol"] = json!(SYMBOL);
    scoped["pages"] = json!([scoped_page]);
    scoped["cash_dividends"] = json!([cash_row(&scoped_page, "25")]);
    actions.push(scoped);
    let documents = session["normalized_documents"]
        .as_array_mut()
        .ok_or("fixture documents")?;
    let corporate_actions = documents
        .iter_mut()
        .find(|doc| doc["kind"] == "corporate_actions")
        .ok_or("fixture action document")?;
    corporate_actions["source_files"]
        .as_array_mut()
        .ok_or("fixture source keys")?
        .push(daily_0060_file_key_fixture(&scoped_page));
    set_selected(input, 1);
    let counts = &mut input["admission"]["counts"];
    counts["cash_dividend_row_count"] = json!(2);
    counts["action_chain_count"] = json!(8);
    counts["action_page_count"] = json!(8);
    counts["source_file_count"] = json!(31);
    Ok(())
}

async fn check_manifest(db: &Daily0060Db, input: &Value) -> Daily0060Result<Value> {
    Ok(
        sqlx::query_scalar("SELECT public.daily_0060_manifest($1,$2)")
            .bind(&input["manifest"])
            .bind(daily_0060_policy())
            .fetch_one(&db.owner)
            .await?,
    )
}

pub(super) async fn dividend_cells(db: &Daily0060Db) -> Daily0060Result {
    db.control(false).await?;
    let receipt = db.slot().await?;
    let (empty, calendar, days) = daily_0060_bundle(db, DATE).await?;
    assert_eq!(db.project(receipt, &calendar, &days).await?, "PROJECTED");
    let mut overlap = empty.clone();
    append_overlap(&mut overlap)?;
    daily_0060_expect_state(db.admit(receipt, &overlap).await, "23514");

    let mut drift = db.owner.begin().await?;
    sqlx::raw_sql(
        "DO $$ DECLARE d text:=pg_get_functiondef('public.daily_0060_manifest(jsonb,jsonb)'::regprocedure);
         BEGIN EXECUTE replace(d,'daily manifest shape','daily manifest drift'); END $$;",
    ).execute(&mut *drift).await?;
    daily_0060_expect_state(
        sqlx::raw_sql(UP)
            .execute(&mut *drift)
            .await
            .map_err(Into::into),
        "55000",
    );
    drift.rollback().await?;
    MIGRATOR.run_to(70, &db.owner).await?;

    for role in [&db.app, &db.worker, &db.admitter] {
        daily_0060_denied(
            role,
            "SELECT public.daily_0070_selected_cash_count('[]')",
            "42501",
        )
        .await?;
    }
    let mut entry = db.owner.begin().await?;
    sqlx::raw_sql("SELECT 1 FROM public.owner_beta_daily_control FOR SHARE")
        .execute(&mut *entry)
        .await?;
    daily_0060_denied(&db.owner, DOWN, "55P03").await?;
    entry.rollback().await?;

    // The exact reversible function rewrite is valid before any new history exists.
    let mut reversible = db.owner.begin().await?;
    sqlx::raw_sql(DOWN).execute(&mut *reversible).await?;
    reversible.rollback().await?;
    assert_eq!(
        check_manifest(db, &empty).await?,
        empty["admission"]["counts"]
    );
    assert_eq!(
        check_manifest(db, &overlap).await?,
        overlap["admission"]["counts"]
    );

    let mut multiple = overlap.clone();
    let chains = sessions(&mut multiple)["actions"]
        .as_array_mut()
        .ok_or("fixture actions")?;
    for idx in [3, 7] {
        let chain = &mut chains[idx];
        let mut next = cash_row(&chain["pages"][0], "35");
        if idx == 7 {
            next["cash_payment_date"] = json!("2026-09-01");
        }
        chain["cash_dividends"]
            .as_array_mut()
            .ok_or("fixture cash rows")?
            .push(next);
    }
    set_selected(&mut multiple, 2);
    multiple["admission"]["counts"]["cash_dividend_row_count"] = json!(4);
    // A different second row conflicts even when both witnesses have equal cardinality.
    daily_0060_expect_state(check_manifest(db, &multiple).await, "23514");
    sessions(&mut multiple)["actions"][7]["cash_dividends"][1]["cash_payment_date"] = Value::Null;
    assert_eq!(
        check_manifest(db, &multiple).await?,
        multiple["admission"]["counts"]
    );

    let mut missing = overlap.clone();
    sessions(&mut missing)["actions"][7]["cash_dividends"] = json!([]);
    missing["admission"]["counts"]["cash_dividend_row_count"] = json!(1);
    let mut wrong_origin = overlap.clone();
    let origin =
        sessions(&mut wrong_origin)["actions"][3]["cash_dividends"][0]["source_file"].clone();
    sessions(&mut wrong_origin)["actions"][7]["cash_dividends"][0]["source_file"] = origin;
    let mut wrong_count = overlap.clone();
    set_selected(&mut wrong_count, 2);
    let mut wrong_support_count = overlap.clone();
    wrong_support_count["admission"]["counts"]["cash_dividend_row_count"] = json!(1);
    let mut wrong_document_keys = overlap.clone();
    let documents = sessions(&mut wrong_document_keys)["normalized_documents"]
        .as_array_mut()
        .ok_or("fixture documents")?;
    documents
        .iter_mut()
        .find(|doc| doc["kind"] == "corporate_actions")
        .ok_or("fixture action document")?["source_files"]
        .as_array_mut()
        .ok_or("fixture keys")?
        .pop();
    for invalid in [
        missing,
        wrong_origin,
        wrong_count,
        wrong_support_count,
        wrong_document_keys,
    ] {
        daily_0060_expect_state(db.admit(receipt, &invalid).await, "23514");
    }
    assert_eq!(db.admit(receipt, &overlap).await?, "ADMITTED");
    assert_eq!(db.admit(receipt, &overlap).await?, "REPLAY");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(
            "SELECT evidence_json FROM public.owner_beta_daily_admissions"
        )
        .fetch_one(&db.owner)
        .await?,
        overlap
    );
    daily_0060_denied(&db.owner, DOWN, "55000").await?;
    Ok(())
}
