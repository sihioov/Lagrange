mod common;
mod service_status_reads_support;

use service_status_reads_support::{
    EvidenceCase, Harness, actor_session_pool, assert_raw_source_clocks, call_stream,
    call_stream_with_bounds, check_counts, check_observation_delay, check_quote_sql_boundaries,
    negative_observation_until, remove_active_lease_overflow, repin_fixture_window,
    required_harness, role_pool, row_value, seed_active_lease_overflow, seed_demand_fixture,
    seed_expiring_stream_fixture, seed_identity_overflow, seed_live_quote_fixture,
    seed_newer_admission_fixture, seed_second_demand_fixture, seed_stream_fixture,
    set_stream_evidence,
};
use sqlx::Row;
async fn read_future_diagnostic_stream(
    app: &sqlx::PgPool,
    stream: &service_status_reads_support::StreamFixture,
    null_bounds: bool,
) -> sqlx::postgres::PgRow {
    let mut tx = app.begin().await.expect("app diagnostic read begins");
    for statement in [
        "SET TRANSACTION READ ONLY",
        "SET LOCAL statement_timeout = '250ms'",
        "SET LOCAL lock_timeout = '50ms'",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("bounded read settings");
    }
    let (session_role, current_role, readonly, query_limit, lock_limit): (String, String, String, String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text,current_setting('transaction_read_only'),current_setting('statement_timeout'),current_setting('lock_timeout')")
            .fetch_one(&mut *tx).await.expect("actual app read guard verification");
    assert_eq!(
        (
            session_role.as_str(),
            current_role.as_str(),
            readonly.as_str(),
            query_limit.as_str(),
            lock_limit.as_str()
        ),
        ("app", "app", "on", "250ms", "50ms")
    );
    let row =
        sqlx::query("SELECT * FROM public.owner_service_status_stream($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(stream.owner_id)
            .bind(&stream.session_hash)
            .bind(stream.slot_id)
            .bind(stream.grant_id)
            .bind(&stream.contract_sha256)
            .bind(&stream.window_sha256)
            .bind(if null_bounds {
                None
            } else {
                Some(stream.open_at)
            })
            .bind(if null_bounds {
                None
            } else {
                Some(stream.close_at)
            })
            .fetch_one(&mut *tx)
            .await
            .expect("required populated app-role diagnostic read");
    tx.commit().await.expect("readonly diagnostic read commits");
    row
}

#[tokio::test]
async fn service_status_stream_preserves_future_diagnostics_without_positive_coverage() {
    use chrono::{DateTime, Duration, Utc};
    let h = required_harness().await;
    let stream = seed_stream_fixture(&h).await.expect("pinned stream proof");
    let app = actor_session_pool(&h, stream.owner_id, &stream.session_hash).await;
    let first = seed_demand_fixture(&h, &stream)
        .await
        .expect("first real demanded identity");
    seed_live_quote_fixture(&h, &stream, &first)
        .await
        .expect("complete first quote");

    let seeded = set_stream_evidence(
        &h,
        &stream,
        &[first],
        EvidenceCase {
            received_age_ms: [-5_000, 1_000],
            provider_age_ms: [1_000, 1_000],
            halted: [false, false],
            ..EvidenceCase::default()
        },
    )
    .await
    .expect("constraint-valid future stored receipt");
    let single = read_future_diagnostic_stream(&app, &stream, false).await;
    check_observation_delay(&single, seeded)
        .expect("actual clock remains within explicit fixture margin");
    check_counts(&single, 1, 0, 0).expect("future receipt is never live");
    assert_raw_source_clocks(&h, &stream, &single)
        .await
        .expect("single diagnostic source clocks preserved");
    assert_eq!(
        row_value::<DateTime<Utc>>(&single, "last_received_at"),
        seeded + Duration::seconds(5)
    );
    assert_eq!(
        row_value::<DateTime<Utc>>(&single, "oldest_received_at"),
        seeded + Duration::seconds(5)
    );
    assert_eq!(
        row_value::<DateTime<Utc>>(&single, "valid_until"),
        (seeded + Duration::milliseconds(9_500))
            .min(negative_observation_until(&single, &stream, false))
    );

    let second = seed_second_demand_fixture(&h, &stream, &first)
        .await
        .expect("second real demand");
    seed_live_quote_fixture(&h, &stream, &second)
        .await
        .expect("complete second quote");
    let demands = [first, second];
    for (
        case,
        received,
        provider,
        halted,
        code,
        null_bounds,
        calendar_failure,
        live,
        halted_count,
    ) in [
        (
            "mixed_live",
            [1_000, -5_000],
            [1_000, 1_000],
            [false, false],
            None,
            false,
            false,
            1,
            0,
        ),
        (
            "mixed_halted",
            [1_000, -5_000],
            [1_000, 1_000],
            [true, false],
            None,
            false,
            false,
            1,
            1,
        ),
        (
            "provider_with_producer_failure",
            [1_000, 1_000],
            [-5_000, -5_000],
            [false, false],
            Some("CONNECTION_LOST"),
            false,
            false,
            0,
            0,
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
        ),
        (
            "null_bounds_future_provider",
            [1_000, 1_000],
            [-5_000, -5_000],
            [false, false],
            None,
            true,
            false,
            0,
            0,
        ),
        (
            "future_provider_calendar_failure",
            [1_000, 1_000],
            [-5_000, -5_000],
            [false, false],
            None,
            false,
            true,
            0,
            0,
        ),
    ] {
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
        .expect("constraint-valid stored diagnostic shape");
        if let Some(code) = code {
            let changed=sqlx::query("UPDATE public.owner_market_stream_producers SET status_code=$2,status_at=$3,connection_state=CASE WHEN $2='CONNECTION_LOST' THEN 'DISCONNECTED' ELSE 'CONNECTED' END WHERE credential_slot_id=$1")
                .bind(stream.slot_id).bind(code)
                .bind(if code=="SUBSCRIPTION_PENDING" {seeded+Duration::seconds(5)} else {seeded-Duration::seconds(1)})
                .execute(&h.owner_pool).await.expect("real paired producer code/status clock");
            assert_eq!(changed.rows_affected(), 1);
        }
        if calendar_failure {
            let changed=sqlx::query("UPDATE public.trading_calendars SET content_sha256=repeat('e',64) WHERE exchange='KRX' AND session_date=$1")
                .bind(stream.session_date).execute(&h.owner_pool).await.expect("current projection no longer joins immutable calendar proof");
            assert_eq!(changed.rows_affected(), 1);
        }
        let row = read_future_diagnostic_stream(&app, &stream, null_bounds).await;
        check_observation_delay(&row, seeded).expect("actual source clock fixture margin");
        check_counts(&row, 2, live, halted_count).expect(case);
        assert_raw_source_clocks(&h, &stream, &row)
            .await
            .expect("all raw clocks retained");
        let expected_min_receipt =
            seeded - Duration::milliseconds(i64::from(received[0].max(received[1])));
        let expected_max_receipt =
            seeded - Duration::milliseconds(i64::from(received[0].min(received[1])));
        let expected_provider =
            seeded - Duration::milliseconds(i64::from(provider[0].max(provider[1])));
        assert_eq!(
            row_value::<DateTime<Utc>>(&row, "oldest_received_at"),
            expected_min_receipt,
            "{case}"
        );
        assert_eq!(
            row_value::<DateTime<Utc>>(&row, "last_received_at"),
            expected_max_receipt,
            "{case}"
        );
        assert_eq!(
            row_value::<DateTime<Utc>>(&row, "oldest_provider_trade_at"),
            expected_provider,
            "{case}"
        );
        let expected_until = negative_observation_until(&row, &stream, null_bounds);
        let expected_until = if code.is_none() && !null_bounds && !calendar_failure {
            expected_until.min(seeded + Duration::milliseconds(9_500))
        } else {
            expected_until
        };
        assert_eq!(
            row_value::<DateTime<Utc>>(&row, "valid_until"),
            expected_until,
            "{case}: diagnostics cannot replace assurance or observation deadlines"
        );
        if code == Some("SUBSCRIPTION_PENDING") {
            assert_eq!(
                row_value::<DateTime<Utc>>(&row, "producer_status_at"),
                seeded + Duration::seconds(5)
            );
        }
        if provider[0] < 0 && provider[1] < 0 {
            assert_eq!(
                (live, halted_count),
                (0, 0),
                "a future minimum provider clock forbids positive coverage"
            );
        }
    }
    app.close().await;
    h.teardown().await;
}

async fn preserved_index_and_rights_catalog(
    harness: &Harness,
) -> Result<serde_json::Value, &'static str> {
    sqlx::query_scalar(r#"
      SELECT jsonb_build_object(
        'older_indexes',(SELECT jsonb_agg(to_jsonb(i) ORDER BY indexname)
          FROM pg_catalog.pg_indexes AS i WHERE schemaname='public' AND indexname NOT IN (
            'jobs_service_status_owner_type_status_available_idx','jobs_service_status_owner_type_finished_idx',
            'owner_beta_service_status_owner_finished_idx')),
        'table_rights',(SELECT jsonb_agg(jsonb_build_object('name',c.relname,'owner',c.relowner,
            'acl',c.relacl::text,'rls',c.relrowsecurity,'force_rls',c.relforcerowsecurity) ORDER BY c.relname)
          FROM pg_catalog.pg_class AS c JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
          WHERE n.nspname='public' AND c.relname IN ('jobs','owner_beta_recommendation_runs','data_entitlements',
            'owner_market_stream_grants','owner_market_stream_producers','owner_market_stream_subscriptions',
            'owner_market_stream_leases','owner_market_stream_lease_items','owner_market_stream_cache')),
        'binding_rights',(SELECT jsonb_build_object('owner',p.proowner,'acl',p.proacl::text,
            'security_definer',p.prosecdef,'config',p.proconfig)
          FROM pg_catalog.pg_proc AS p WHERE p.oid=
            'public.owner_market_stream_api_binding_valid(uuid,text,uuid,uuid,text)'::regprocedure))
      "#).fetch_one(&harness.owner_pool).await.map_err(|_| "older index/rights catalog snapshot failed")
}

async fn check_contract(harness: &Harness) -> Result<(), &'static str> {
    let fixture = seed_stream_fixture(harness).await?;
    let app = actor_session_pool(harness, fixture.owner_id, &fixture.session_hash).await;

    let app_exec: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_function_privilege(
             current_user,
             pg_catalog.to_regprocedure(
                 'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'
             ),
             'EXECUTE'
         )",
    )
    .fetch_one(&harness.app_pool)
    .await
    .map_err(|_| "app function privilege could not be checked")?;
    let admin_exec: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_function_privilege(
             current_user,
             pg_catalog.to_regprocedure(
                 'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'
             ),
             'EXECUTE'
         )",
    )
    .fetch_one(&harness.admin_pool)
    .await
    .map_err(|_| "admin function privilege could not be checked")?;
    if !app_exec || admin_exec {
        return Err("stream function execute grant is not app-only");
    }
    for url in [
        harness.worker_url.as_str(),
        harness.research_writer_url.as_str(),
    ] {
        let pool = role_pool(url).await;
        let can_execute: bool = sqlx::query_scalar(
            "SELECT pg_catalog.has_function_privilege(
                 current_user,
                 pg_catalog.to_regprocedure(
                     'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'
                 ),
                 'EXECUTE'
             )",
        )
        .fetch_one(&pool)
        .await
        .map_err(|_| "non-app function privilege could not be checked")?;
        if can_execute {
            return Err("stream read capability was granted outside the app role");
        }
    }
    let can_audit_execute: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_function_privilege(
             current_user,
             pg_catalog.to_regprocedure(
                 'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'
             ),
             'EXECUTE'
         )",
    )
    .fetch_one(&harness.audit_pool)
    .await
    .map_err(|_| "audit_writer function privilege could not be checked")?;
    if can_audit_execute {
        return Err("stream read capability was granted to audit_writer");
    }

    let result_columns: String = sqlx::query_scalar(
        "SELECT pg_catalog.pg_get_function_result(
             'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'::regprocedure
         )",
    )
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "stream function result signature could not be read")?;
    let expected_columns = "TABLE(observed_at timestamp with time zone, demand_count bigint, live_count bigint, halted_count bigint, producer_present boolean, connection_state text, status_code text, producer_status_at timestamp with time zone, producer_heartbeat_at timestamp with time zone, producer_lease_expires_at timestamp with time zone, oldest_received_at timestamp with time zone, oldest_provider_trade_at timestamp with time zone, last_received_at timestamp with time zone, lineage_valid boolean, gap_present boolean, valid_until timestamp with time zone)";
    if result_columns != expected_columns {
        return Err("stream function result columns differ from the accepted contract");
    }
    let (function_owner, security_definer, function_config):
        (String, bool, Option<Vec<String>>) = sqlx::query_as(
        "SELECT owner_role.rolname, proc.prosecdef, proc.proconfig
           FROM pg_catalog.pg_proc AS proc
           JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = proc.proowner
          WHERE proc.oid =
              'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'::regprocedure",
    )
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "stream function security metadata could not be read")?;
    if function_owner != "migration_owner"
        || !security_definer
        || !function_config.is_some_and(|config| {
            config
                .iter()
                .any(|setting| setting == "search_path=pg_catalog")
        })
    {
        return Err("stream function owner, SECURITY DEFINER, or fixed path differs");
    }

    let app_role = role_pool(&harness.app_url).await;
    for query in [
        "SELECT 1 FROM public.owner_market_stream_grants LIMIT 1",
        "SELECT 1 FROM public.owner_market_stream_producers LIMIT 1",
        "SELECT 1 FROM public.owner_market_stream_subscriptions LIMIT 1",
    ] {
        for private_reader in [&app_role, &harness.admin_pool] {
            if sqlx::query(query)
                .fetch_optional(private_reader)
                .await
                .is_ok()
            {
                return Err("app or admin can directly read a private stream table");
            }
        }
    }
    for query in [
        "SELECT 1 FROM public.owner_market_stream_leases LIMIT 1",
        "SELECT 1 FROM public.owner_market_stream_lease_items LIMIT 1",
        "SELECT 1 FROM public.owner_market_stream_cache LIMIT 1",
    ] {
        if sqlx::query(query)
            .fetch_optional(&harness.admin_pool)
            .await
            .is_ok()
        {
            return Err("admin can directly read a private stream table");
        }
    }
    if sqlx::query(
        "SELECT * FROM public.owner_service_status_stream(
             $1, $2, $3, $4, $5, $6, $7, $8
         )",
    )
    .bind(fixture.owner_id)
    .bind(&fixture.session_hash)
    .bind(fixture.slot_id)
    .bind(fixture.grant_id)
    .bind(&fixture.contract_sha256)
    .bind(&fixture.window_sha256)
    .bind(fixture.open_at)
    .bind(fixture.close_at)
    .fetch_optional(&harness.admin_pool)
    .await
    .is_ok()
    {
        return Err("admin can execute the owner stream read capability");
    }

    let mut read_tx = app
        .begin()
        .await
        .map_err(|_| "app read transaction failed")?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *read_tx)
        .await
        .map_err(|_| "app transaction could not be made read-only")?;
    let no_demand = sqlx::query(
        "SELECT * FROM public.owner_service_status_stream(
             $1, $2, $3, $4, $5, $6, $7, $8
         )",
    )
    .bind(fixture.owner_id)
    .bind(&fixture.session_hash)
    .bind(fixture.slot_id)
    .bind(fixture.grant_id)
    .bind(&fixture.contract_sha256)
    .bind(&fixture.window_sha256)
    .bind(fixture.open_at)
    .bind(fixture.close_at)
    .fetch_optional(&mut *read_tx)
    .await
    .map_err(|_| "app stream read failed in a read-only transaction")?;
    let no_demand = no_demand.ok_or("valid Owner no-demand read returned no row")?;
    if row_value::<i64>(&no_demand, "demand_count") != 0
        || row_value::<i64>(&no_demand, "live_count") != 0
        || !row_value::<bool>(&no_demand, "producer_present")
        || row_value::<String>(&no_demand, "connection_state") != "CONNECTED"
        || !row_value::<bool>(&no_demand, "lineage_valid")
    {
        return Err("no-demand aggregate lost the current producer/calendar lineage");
    }
    read_tx
        .commit()
        .await
        .map_err(|_| "read-only stream transaction did not commit")?;

    let wrong_window = format!("sha256:{}", "f".repeat(64));
    let wrong_lineage = call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &wrong_window,
    )
    .await?
    .ok_or("bound Owner read failed closed on a mismatched window")?;
    if row_value::<bool>(&wrong_lineage, "lineage_valid") {
        return Err("mismatched validated-window pin was reported as valid lineage");
    }

    let mismatch_pool =
        actor_session_pool(harness, harness.member.user_id, &fixture.session_hash).await;
    if call_stream(
        &mismatch_pool,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .is_some()
    {
        return Err("stream read returned state for a mismatched actor GUC");
    }
    if call_stream(
        &app,
        &fixture,
        harness.member.user_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .is_some()
    {
        return Err("stream read returned state for a cross-owner binding");
    }
    let malformed = call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        "not-a-session-hash",
        &fixture.window_sha256,
    )
    .await?;
    if malformed.is_some() {
        return Err("stream read returned state for a malformed session pin");
    }
    let member_session_hash = common::sha256_hex(harness.member.cookie_value.as_bytes());
    let member_pool =
        actor_session_pool(harness, harness.member.user_id, &member_session_hash).await;
    if call_stream(
        &member_pool,
        &fixture,
        harness.member.user_id,
        &member_session_hash,
        &fixture.window_sha256,
    )
    .await?
    .is_some()
    {
        return Err("non-Owner browser session returned stream state");
    }

    let wrong_session_pool = actor_session_pool(harness, fixture.owner_id, &"f".repeat(64)).await;
    for (pool, owner, session, window) in [
        (
            &mismatch_pool,
            fixture.owner_id,
            fixture.session_hash.as_str(),
            fixture.window_sha256.as_str(),
        ),
        (
            &app,
            harness.member.user_id,
            fixture.session_hash.as_str(),
            fixture.window_sha256.as_str(),
        ),
        (
            &app,
            fixture.owner_id,
            "not-a-session-hash",
            fixture.window_sha256.as_str(),
        ),
        (
            &member_pool,
            harness.member.user_id,
            member_session_hash.as_str(),
            fixture.window_sha256.as_str(),
        ),
        (
            &wrong_session_pool,
            fixture.owner_id,
            fixture.session_hash.as_str(),
            fixture.window_sha256.as_str(),
        ),
        (
            &app,
            fixture.owner_id,
            fixture.session_hash.as_str(),
            "not-a-window-hash",
        ),
    ] {
        if call_stream_with_bounds(pool, &fixture, owner, session, window, None, None)
            .await?
            .is_some()
        {
            return Err("paired NULL mode bypassed an actor/session/Owner/hash guard");
        }
    }
    let mut wrong_network = fixture.clone();
    wrong_network.contract_sha256 = "f".repeat(64);
    if call_stream_with_bounds(
        &app,
        &wrong_network,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
        None,
        None,
    )
    .await?
    .is_some()
    {
        return Err("paired NULL mode bypassed exact network pin binding");
    }
    for pin in ["slot", "grant", "network", "wrong_slot", "wrong_grant"] {
        let mut denied = fixture.clone();
        match pin {
            "slot" => denied.slot_id = uuid::Uuid::nil(),
            "grant" => denied.grant_id = uuid::Uuid::nil(),
            "wrong_slot" => denied.slot_id = uuid::Uuid::new_v4(),
            "wrong_grant" => denied.grant_id = uuid::Uuid::new_v4(),
            _ => denied.contract_sha256 = "malformed".to_owned(),
        }
        if call_stream_with_bounds(
            &app,
            &denied,
            fixture.owner_id,
            &fixture.session_hash,
            &fixture.window_sha256,
            None,
            None,
        )
        .await?
        .is_some()
        {
            return Err("paired NULL mode accepted a missing/malformed mandatory pin");
        }
    }

    let demand = seed_demand_fixture(harness, &fixture).await?;
    let actor_visible_leases: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*) FROM public.owner_market_stream_leases
          WHERE owner_user_id = $1",
    )
    .bind(fixture.owner_id)
    .fetch_one(&app)
    .await
    .map_err(|_| "existing app lease RLS read failed")?;
    let actor_visible_lease_items: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*) FROM public.owner_market_stream_lease_items
          WHERE owner_user_id = $1",
    )
    .bind(fixture.owner_id)
    .fetch_one(&app)
    .await
    .map_err(|_| "existing app lease-item RLS read failed")?;
    if actor_visible_leases != 1 || actor_visible_lease_items != 1 {
        return Err("existing app lease policies no longer limit reads to its session");
    }
    let demand_row = call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .ok_or("valid demand aggregate returned no row")?;
    if row_value::<i64>(&demand_row, "demand_count") != 1
        || row_value::<i64>(&demand_row, "live_count") != 0
        || !row_value::<bool>(&demand_row, "lineage_valid")
    {
        return Err("duplicate browser leases were not reduced to one expected identity");
    }

    seed_live_quote_fixture(harness, &fixture, &demand).await?;
    let actor_visible_quotes: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*) FROM public.owner_market_stream_cache
          WHERE owner_user_id = $1",
    )
    .bind(fixture.owner_id)
    .fetch_one(&app)
    .await
    .map_err(|_| "existing app quote-cache RLS read failed")?;
    if actor_visible_quotes != 1 {
        return Err("existing app quote-cache policy did not remain actor scoped");
    }
    let live_row = call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .ok_or("valid cached quote aggregate returned no row")?;
    if row_value::<i64>(&live_row, "demand_count") != 1
        || row_value::<i64>(&live_row, "live_count") != 1
        || row_value::<i64>(&live_row, "halted_count") != 0
    {
        return Err("current quote did not satisfy the stream delivery predicates");
    }

    let source_delivery = sqlx::query(
        "SELECT subscription_state, producer_connection_state,
                producer_current_epoch, producer_session_date
           FROM public.owner_market_stream_delivery_state($1, $2, $3)",
    )
    .bind(fixture.owner_id)
    .bind(&fixture.session_hash)
    .bind(demand.original_lease_id)
    .fetch_optional(&app)
    .await
    .map_err(|_| "existing stream delivery behavior could not be read")?
    .ok_or("existing stream delivery helper omitted the fixture lease")?;
    if source_delivery
        .try_get::<String, _>("subscription_state")
        .ok()
        .as_deref()
        != Some("ACKED")
        || source_delivery
            .try_get::<String, _>("producer_connection_state")
            .ok()
            .as_deref()
            != Some("CONNECTED")
        || source_delivery
            .try_get::<Option<uuid::Uuid>, _>("producer_current_epoch")
            .ok()
            .flatten()
            != Some(fixture.producer_epoch)
        || source_delivery
            .try_get::<Option<chrono::NaiveDate>, _>("producer_session_date")
            .ok()
            .flatten()
            != Some(fixture.session_date)
    {
        return Err("live stream fixture differs from current delivery behavior");
    }

    let mut membership_tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "membership-staleness fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(fixture.owner_id.to_string())
        .execute(&mut *membership_tx)
        .await
        .map_err(|_| "membership-staleness actor context failed")?;
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = 'DISABLED', disabled_at = pg_catalog.clock_timestamp(),
                transition_actor_user_id = $2,
                transition_code_commit = 'c3295be',
                transition_entitlement_sha256 = $3
          WHERE id = $1",
    )
    .bind(demand.membership_id)
    .bind(fixture.owner_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .execute(&mut *membership_tx)
    .await
    .map_err(|_| "membership-staleness transition failed")?;
    membership_tx
        .commit()
        .await
        .map_err(|_| "membership-staleness fixture did not commit")?;
    let stale_demand = call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .ok_or("stale admission was hidden as a missing aggregate")?;
    if row_value::<i64>(&stale_demand, "demand_count") != 1
        || row_value::<i64>(&stale_demand, "live_count") != 0
        || row_value::<bool>(&stale_demand, "lineage_valid")
    {
        return Err("stale admission disappeared from demand denominator or stayed live");
    }

    let lease_overflow = seed_active_lease_overflow(harness, &fixture).await?;
    if call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .is_some()
    {
        return Err("more than twenty active leases returned a truncated aggregate");
    }
    if call_stream_with_bounds(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
        None,
        None,
    )
    .await?
    .is_some()
    {
        return Err("paired NULL mode bypassed lease/identity completeness limits");
    }
    remove_active_lease_overflow(harness, fixture.owner_id, &lease_overflow).await?;

    let expired_or_revoked: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*)
           FROM public.owner_market_stream_leases AS lease
          WHERE lease.owner_user_id = $1
            AND lease.state = 'ACTIVE'
            AND lease.lease_expires_at > pg_catalog.clock_timestamp()
            AND NOT public.owner_market_stream_session_valid(
                lease.session_hash, lease.owner_user_id
            )",
    )
    .bind(fixture.owner_id)
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "expired/revoked fixture leases could not be checked")?;
    if expired_or_revoked < 2 {
        return Err("expired and revoked browser sessions were not represented in fixture");
    }

    let mut plan_tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "planner transaction could not start")?;
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *plan_tx)
        .await
        .map_err(|_| "planner fixture setting failed")?;
    let status_plan: Vec<String> = sqlx::query(
        "EXPLAIN (COSTS OFF)
         SELECT id FROM public.jobs
          WHERE owner_user_id = $1 AND job_type = 'owner_equity_v2'
            AND status = 'QUEUED' AND available_at <= pg_catalog.now()",
    )
    .bind(fixture.owner_id)
    .fetch_all(&mut *plan_tx)
    .await
    .map_err(|_| "queue status EXPLAIN failed")?
    .into_iter()
    .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
    .collect();
    if !status_plan.iter().any(|line| line.contains("Index Scan")) {
        return Err("status queue EXPLAIN did not produce an index scan");
    }
    let history_plan: Vec<String> = sqlx::query(
        "EXPLAIN (COSTS OFF)
         SELECT id FROM public.jobs
          WHERE owner_user_id = $1 AND job_type = 'owner_equity_v2'
            AND finished_at IS NOT NULL ORDER BY finished_at DESC LIMIT 1",
    )
    .bind(fixture.owner_id)
    .fetch_all(&mut *plan_tx)
    .await
    .map_err(|_| "queue history EXPLAIN failed")?
    .into_iter()
    .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
    .collect();
    if !history_plan.iter().any(|line| line.contains("Index Scan")) {
        return Err("finished queue EXPLAIN did not produce an index scan");
    }
    let service_status_index_count: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*) FROM pg_catalog.pg_indexes
          WHERE schemaname = 'public'
            AND indexname IN (
                'jobs_service_status_owner_type_status_available_idx',
                'jobs_service_status_owner_type_finished_idx',
                'owner_beta_service_status_owner_finished_idx'
            )",
    )
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "service-status queue index definitions could not be checked")?;
    if service_status_index_count != 3 {
        return Err("one or more required service-status queue indexes are missing");
    }
    let (relation, valid, ready, definition, predicate): (String, bool, bool, String, String) = sqlx::query_as(
        "SELECT t.relname,i.indisvalid,i.indisready,pg_catalog.pg_get_indexdef(i.indexrelid),
                pg_catalog.pg_get_expr(i.indpred,i.indrelid)
           FROM pg_catalog.pg_index AS i JOIN pg_catalog.pg_class AS t ON t.oid=i.indrelid
          WHERE i.indexrelid=pg_catalog.to_regclass('public.owner_beta_service_status_owner_finished_idx')"
    ).fetch_one(&harness.owner_pool).await.map_err(|_| "exact Owner-beta history index metadata is missing")?;
    let predicate: String = predicate
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')'))
        .collect();
    if relation != "owner_beta_recommendation_runs"
        || !valid
        || !ready
        || !definition.contains("USING btree (owner_user_id, finished_at DESC)")
        || predicate != "status='SUCCEEDED'::textANDfinished_atISNOTNULL"
    {
        return Err("Owner-beta history index table/order/partial predicate is incorrect");
    }
    plan_tx
        .commit()
        .await
        .map_err(|_| "planner transaction did not commit")?;

    // Restore the fixture actor before touching its FORCE-RLS web session.
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "session expiry fixture transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(fixture.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "session expiry actor context failed")?;
    let expiry_hash = common::sha256_hex(b"service-status-expired-auth-session");
    let revoked_hash = common::sha256_hex(b"service-status-revoked-auth-session");
    for (hash, csrf, expired) in [(&expiry_hash, "3", true), (&revoked_hash, "4", false)] {
        sqlx::query(
            "INSERT INTO public.web_sessions (
                 user_id, session_hash, csrf_hash, created_at, expires_at, revoked_at
             ) VALUES (
                 $1, $2, $3,
                 CASE WHEN $4 THEN pg_catalog.clock_timestamp() - INTERVAL '2 minutes'
                      ELSE pg_catalog.clock_timestamp() END,
                 CASE WHEN $4 THEN pg_catalog.clock_timestamp() - INTERVAL '1 minute'
                      ELSE pg_catalog.clock_timestamp() + INTERVAL '1 hour' END,
                 CASE WHEN $4 THEN NULL ELSE pg_catalog.clock_timestamp() END
             )",
        )
        .bind(fixture.owner_id)
        .bind(hash)
        .bind(common::sha256_hex(csrf.as_bytes()))
        .bind(expired)
        .execute(&mut *tx)
        .await
        .map_err(|_| "expired/revoked authorization session could not be seeded")?;
    }
    tx.commit()
        .await
        .map_err(|_| "expired/revoked session fixture transaction failed")?;
    for hash in [&expiry_hash, &revoked_hash] {
        let pool = actor_session_pool(harness, fixture.owner_id, hash).await;
        if call_stream(
            &pool,
            &fixture,
            fixture.owner_id,
            hash,
            &fixture.window_sha256,
        )
        .await?
        .is_some()
        {
            return Err("expired or revoked web session returned stream state");
        }
        if call_stream_with_bounds(
            &pool,
            &fixture,
            fixture.owner_id,
            hash,
            &fixture.window_sha256,
            None,
            None,
        )
        .await?
        .is_some()
        {
            return Err("paired NULL mode bypassed expired/revoked session validation");
        }
    }

    seed_identity_overflow(harness, &fixture).await?;
    if call_stream(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
    )
    .await?
    .is_some()
    {
        return Err("more than thirty identities returned a truncated aggregate");
    }
    if call_stream_with_bounds(
        &app,
        &fixture,
        fixture.owner_id,
        &fixture.session_hash,
        &fixture.window_sha256,
        None,
        None,
    )
    .await?
    .is_some()
    {
        return Err("paired NULL mode bypassed lease/identity completeness limits");
    }

    let preserved_before = preserved_index_and_rights_catalog(harness).await?;
    let down_sql = include_str!("../../../migrations/0058_service_status_reads.down.sql");
    let mut down_tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "0058 down-migration transaction could not start")?;
    sqlx::raw_sql(down_sql)
        .execute(&mut *down_tx)
        .await
        .map_err(|_| "0058 down migration failed on the disposable fixture")?;
    down_tx
        .commit()
        .await
        .map_err(|_| "0058 down migration did not commit")?;
    let function_absent: bool = sqlx::query_scalar(
        "SELECT pg_catalog.to_regprocedure(
            'public.owner_service_status_stream(uuid,text,uuid,uuid,text,text,timestamptz,timestamptz)'
         ) IS NULL",
    )
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "0058 down migration function check failed")?;
    let status_indexes: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*) FROM pg_catalog.pg_indexes
          WHERE schemaname = 'public'
            AND indexname IN (
                'jobs_service_status_owner_type_status_available_idx',
                'jobs_service_status_owner_type_finished_idx',
                'owner_beta_service_status_owner_finished_idx'
            )",
    )
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "0058 down migration index check failed")?;
    if !function_absent || status_indexes != 0 {
        return Err("0058 down migration left its function or indexes behind");
    }
    if preserved_index_and_rights_catalog(harness).await? != preserved_before {
        return Err("0058 down changed older indexes or existing table/binding grants and RLS");
    }
    Ok(())
}

#[tokio::test]
async fn service_status_stream_read_obeys_db_capability_contract() {
    let harness = required_harness().await;
    let result = check_contract(&harness).await;
    harness.teardown().await;
    assert_eq!(
        result,
        Ok(()),
        "service-status DB contract assertion failed"
    );
}

async fn check_proof_and_timing(harness: &Harness) -> Result<(), &'static str> {
    let fixture = seed_stream_fixture(harness).await?;
    let app = actor_session_pool(harness, fixture.owner_id, &fixture.session_hash).await;
    let demand = seed_demand_fixture(harness, &fixture).await?;
    seed_live_quote_fixture(harness, &fixture, &demand).await?;
    check_quote_sql_boundaries(harness, &app, &fixture, &demand).await
}

#[tokio::test]
async fn service_status_stream_requires_exact_proof_and_actual_sql_timing() {
    let harness = required_harness().await;
    let result = check_proof_and_timing(&harness).await;
    harness.teardown().await;
    assert_eq!(result, Ok(()), "stream SQL proof/timing assertion failed");
}

async fn check_stream_semantics(harness: &Harness) -> Result<(), &'static str> {
    use chrono::{DateTime, Duration, Utc};
    let stream = seed_stream_fixture(harness).await?;
    let app = actor_session_pool(harness, stream.owner_id, &stream.session_hash).await;
    let (session_role, active_role): (String, String) =
        sqlx::query_as("SELECT session_user::text, current_user::text")
            .fetch_one(&app)
            .await
            .map_err(|_| "actual app session role could not be verified")?;
    if session_role != "app" || active_role != "app" {
        return Err("G1 semantic checks must execute as an actual app session, not SET ROLE");
    }

    // No demand ignores an old stopped producer's success deadlines, while
    // preserving its real diagnostic clocks and independently fresh calendar.
    set_stream_evidence(
        harness,
        &stream,
        &[],
        EvidenceCase {
            heartbeat_age_ms: 600_000,
            lease_remaining_ms: -500_000,
            connection: "STOPPED",
            ..EvidenceCase::default()
        },
    )
    .await?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("old stopped no-demand observation returned no row")?;
    check_counts(&row, 0, 0, 0)?;
    if row_value::<DateTime<Utc>>(&row, "valid_until")
        != negative_observation_until(&row, &stream, false)
    {
        return Err("no-demand observation inherited an old positive deadline");
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;

    let first = seed_demand_fixture(harness, &stream).await?;
    let second = seed_second_demand_fixture(harness, &stream, &first).await?;
    let demands = [first, second];
    seed_live_quote_fixture(harness, &stream, &first).await?;
    seed_live_quote_fixture(harness, &stream, &second).await?;

    let old = EvidenceCase {
        heartbeat_age_ms: 600_000,
        lease_remaining_ms: -500_000,
        received_age_ms: [60_000, 65_000],
        provider_age_ms: [60_000, 65_000],
        connection: "STOPPED",
        ..EvidenceCase::default()
    };
    let seeded = set_stream_evidence(harness, &stream, &demands, old).await?;
    let closed = call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        None,
        None,
    )
    .await?
    .ok_or("valid paired NULL observation absent")?;
    check_observation_delay(&closed, seeded)?;
    check_counts(&closed, 2, 0, 0)?;
    if row_value::<bool>(&closed, "lineage_valid")
        || row_value::<DateTime<Utc>>(&closed, "valid_until")
            != negative_observation_until(&closed, &stream, true)
    {
        return Err("paired NULL observation fabricated proof/coverage or expired at old clocks");
    }
    assert_raw_source_clocks(harness, &stream, &closed).await?;
    // The production capability must also accept nullable mode READ ONLY.
    let mut tx = app
        .begin()
        .await
        .map_err(|_| "nullable read-only transaction failed")?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|_| "nullable READ ONLY could not be set")?;
    let readonly = sqlx::query(
        "SELECT * FROM public.owner_service_status_stream($1,$2,$3,$4,$5,$6,NULL,NULL)",
    )
    .bind(stream.owner_id)
    .bind(&stream.session_hash)
    .bind(stream.slot_id)
    .bind(stream.grant_id)
    .bind(&stream.contract_sha256)
    .bind(&stream.window_sha256)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "nullable G1 performed forbidden read-only side effects")?;
    check_counts(&readonly, 2, 0, 0)?;
    tx.commit()
        .await
        .map_err(|_| "nullable read-only commit failed")?;

    for (open, close) in [
        (None, Some(stream.close_at)),
        (Some(stream.open_at), None),
        (Some(stream.close_at), Some(stream.open_at)),
        (Some(stream.open_at), Some(stream.open_at)),
        (
            Some(stream.open_at - Duration::days(1)),
            Some(stream.close_at),
        ),
        (
            Some(stream.open_at),
            Some(stream.close_at + Duration::days(1)),
        ),
    ] {
        if call_stream_with_bounds(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
            open,
            close,
        )
        .await?
        .is_some()
        {
            return Err("one-NULL/reversed/equal/wrong-day bounds returned state");
        }
    }

    for (halted, live, halted_count) in [([true, true], 2, 2), ([false, true], 2, 1)] {
        let seeded = set_stream_evidence(
            harness,
            &stream,
            &demands,
            EvidenceCase {
                halted,
                ..EvidenceCase::default()
            },
        )
        .await?;
        let row = call_stream(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
        )
        .await?
        .ok_or("validated covered aggregate absent")?;
        check_observation_delay(&row, seeded)?;
        check_counts(&row, 2, live, halted_count)?;
        let heartbeat: DateTime<Utc> = row_value(&row, "producer_heartbeat_at");
        if row_value::<DateTime<Utc>>(&row, "valid_until") != heartbeat + Duration::seconds(10) {
            return Err("normal/halted positive assurance was extended to generic observation TTL");
        }
        assert_raw_source_clocks(harness, &stream, &row).await?;
    }

    // The same stored quotes were just proven fully covered in open mode.
    // NULL mode must suppress their positive coverage/proof without refreshing
    // or discarding the stored clocks, and without assuming zero demand.
    let null_current = call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        None,
        None,
    )
    .await?
    .ok_or("current quote NULL observation absent")?;
    check_counts(&null_current, 2, 0, 0)?;
    if row_value::<bool>(&null_current, "lineage_valid")
        || row_value::<DateTime<Utc>>(&null_current, "valid_until")
            != negative_observation_until(&null_current, &stream, true)
    {
        return Err("NULL mode claimed current proof/coverage or inherited live assurance");
    }
    assert_raw_source_clocks(harness, &stream, &null_current).await?;

    // One eligible halted identity, one old raw quote. The raw minima diagnose
    // staleness but only the covered identity's original provider deadline caps
    // its positive metric (2s margin versus 500ms maximum measured delay).
    let partial = EvidenceCase {
        received_age_ms: [25_000, 35_000],
        provider_age_ms: [28_000, 35_000],
        ..EvidenceCase::default()
    };
    let seeded = set_stream_evidence(harness, &stream, &demands, partial).await?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("partial halted aggregate absent")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 1, 1)?;
    let expected_until = seeded + Duration::seconds(2);
    if row_value::<DateTime<Utc>>(&row, "valid_until") != expected_until
        || row_value::<DateTime<Utc>>(&row, "oldest_provider_trade_at") + Duration::seconds(30)
            >= row_value::<DateTime<Utc>>(&row, "observed_at")
    {
        return Err(
            "filtered eligible deadline did not preserve partial assurance and old raw diagnosis",
        );
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;

    // A halted flag cannot overcome rejected ACK or wrong exact proof identity.
    for invalid in ["subscription", "proof"] {
        set_stream_evidence(harness, &stream, &demands, EvidenceCase::default()).await?;
        if invalid == "subscription" {
            sqlx::query("UPDATE public.owner_market_stream_subscriptions SET state='REJECTED' WHERE credential_slot_id=$1 AND symbol='069501'")
                .bind(stream.slot_id).execute(&harness.owner_pool).await.map_err(|_| "halted subscription denial fixture failed")?;
        } else {
            sqlx::query("UPDATE public.owner_market_stream_cache SET session_proof_id=$2 WHERE owner_user_id=$1 AND membership_id=$3")
                .bind(stream.owner_id).bind(uuid::Uuid::new_v4()).bind(second.membership_id)
                .execute(&harness.owner_pool).await.map_err(|_| "halted proof denial fixture failed")?;
        }
        let row = call_stream(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
        )
        .await?
        .ok_or("invalid halted evidence removed denominator")?;
        check_counts(&row, 2, 1, 1)?;
        let heartbeat: DateTime<Utc> = row_value(&row, "producer_heartbeat_at");
        if row_value::<DateTime<Utc>>(&row, "valid_until") != heartbeat + Duration::seconds(10) {
            return Err("invalid second halt extended the remaining validated metric assurance");
        }
        sqlx::query("UPDATE public.owner_market_stream_subscriptions SET state='ACKED' WHERE credential_slot_id=$1 AND symbol='069501'")
            .bind(stream.slot_id).execute(&harness.owner_pool).await.map_err(|_| "halted ACK restoration failed")?;
        sqlx::query("UPDATE public.owner_market_stream_cache SET session_proof_id=$2 WHERE owner_user_id=$1 AND membership_id=$3")
            .bind(stream.owner_id).bind(stream.session_proof_id).bind(second.membership_id)
            .execute(&harness.owner_pool).await.map_err(|_| "halted proof restoration failed")?;
    }

    for case in [
        EvidenceCase {
            connection: "DISCONNECTED",
            ..EvidenceCase::default()
        },
        EvidenceCase {
            gap: true,
            ..EvidenceCase::default()
        },
        EvidenceCase {
            heartbeat_age_ms: 10_500,
            ..EvidenceCase::default()
        },
        EvidenceCase {
            heartbeat_age_ms: 2_000,
            lease_remaining_ms: -1_000,
            ..EvidenceCase::default()
        },
    ] {
        let seeded = set_stream_evidence(harness, &stream, &demands, case).await?;
        let row = call_stream(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
        )
        .await?
        .ok_or("fresh producer failure observation absent")?;
        check_observation_delay(&row, seeded)?;
        check_counts(&row, 2, 0, 0)?;
        if row_value::<DateTime<Utc>>(&row, "valid_until")
            != negative_observation_until(&row, &stream, false)
        {
            return Err(
                "fresh disconnected/gap/stale/expired producer diagnosis inherited assurance",
            );
        }
        assert_raw_source_clocks(harness, &stream, &row).await?;
    }

    // A typed diagnosed failure also ignores obsolete success assurances;
    // its persisted transition timestamp remains a real diagnostic source.
    let seeded = set_stream_evidence(harness, &stream, &demands, EvidenceCase::default()).await?;
    sqlx::query("UPDATE public.owner_market_stream_producers SET status_code='CONNECTION_LOST', status_at=$2 WHERE credential_slot_id=$1")
        .bind(stream.slot_id).bind(seeded).execute(&harness.owner_pool).await
        .map_err(|_| "typed producer failure fixture failed")?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("typed failure observation absent")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 0, 0)?;
    if row_value::<Option<String>>(&row, "status_code").as_deref() != Some("CONNECTION_LOST")
        || row_value::<Option<DateTime<Utc>>>(&row, "producer_status_at") != Some(seeded)
        || row_value::<DateTime<Utc>>(&row, "valid_until")
            != negative_observation_until(&row, &stream, false)
    {
        return Err("typed failure lost its real transition clock or inherited liveness assurance");
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;

    // No quotes yet, but current producer readiness is still a positive
    // prerequisite of pending readiness and retains its real lease margin.
    let mut pending = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "pending quote fixture transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *pending)
        .await
        .map_err(|_| "pending quote actor failed")?;
    sqlx::query("DELETE FROM public.owner_market_stream_cache WHERE owner_user_id=$1")
        .bind(stream.owner_id)
        .execute(&mut *pending)
        .await
        .map_err(|_| "pending quote fixture removal failed")?;
    pending
        .commit()
        .await
        .map_err(|_| "pending quote fixture commit failed")?;
    let seeded = set_stream_evidence(
        harness,
        &stream,
        &[],
        EvidenceCase {
            lease_remaining_ms: 7_000,
            ..EvidenceCase::default()
        },
    )
    .await?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("pending readiness aggregate absent")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 0, 0)?;
    if !row_value::<bool>(&row, "lineage_valid")
        || row_value::<DateTime<Utc>>(&row, "valid_until") != seeded + Duration::seconds(2)
    {
        return Err("pending readiness was not bounded by current positive lease assurance");
    }

    // Actual demand expiry remains relevant even without a positive quote.
    let expiry: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE public.owner_market_stream_leases SET lease_expires_at=clock_timestamp()+INTERVAL '1 second'
          WHERE id=$1 RETURNING lease_expires_at",
    ).bind(first.original_lease_id).fetch_one(&harness.owner_pool).await
        .map_err(|_| "demand expiry fixture failed")?;
    let row = call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        None,
        None,
    )
    .await?
    .ok_or("demand expiry observation absent")?;
    if row_value::<DateTime<Utc>>(&row, "observed_at") >= expiry
        || row_value::<DateTime<Utc>>(&row, "valid_until") != expiry
    {
        return Err("actual active demand expiry was ignored or fixture margin elapsed");
    }
    check_counts(&row, 2, 0, 0)?;
    sqlx::query("UPDATE public.owner_market_stream_leases SET lease_expires_at=clock_timestamp()+INTERVAL '1 hour' WHERE id=$1")
        .bind(first.original_lease_id).execute(&harness.owner_pool).await.map_err(|_| "demand expiry restoration failed")?;

    let seeded = set_stream_evidence(harness, &stream, &[], EvidenceCase::default()).await?;
    let open = seeded + Duration::seconds(2);
    let preopen = call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        Some(open),
        Some(stream.close_at),
    )
    .await?
    .ok_or("preopen observation absent")?;
    check_observation_delay(&preopen, seeded)?;
    check_counts(&preopen, 2, 0, 0)?;
    if row_value::<bool>(&preopen, "lineage_valid")
        || row_value::<DateTime<Utc>>(&preopen, "valid_until") != open
    {
        return Err("preopen proof/coverage or its opening transition was incorrect");
    }
    let postclose = call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        Some(stream.open_at),
        Some(seeded - Duration::seconds(1)),
    )
    .await?
    .ok_or("postclose observation absent")?;
    check_counts(&postclose, 2, 0, 0)?;
    if row_value::<bool>(&postclose, "lineage_valid") {
        return Err("elapsed window retained positive producer proof");
    }

    // Recreate current quotes without duplicate subscriptions for the close cap.
    sqlx::query("DELETE FROM public.owner_market_stream_subscriptions WHERE credential_slot_id=$1")
        .bind(stream.slot_id)
        .execute(&harness.owner_pool)
        .await
        .map_err(|_| "close quote fixture subscription cleanup failed")?;
    seed_live_quote_fixture(harness, &stream, &first).await?;
    seed_live_quote_fixture(harness, &stream, &second).await?;
    let seeded = set_stream_evidence(harness, &stream, &demands, EvidenceCase::default()).await?;
    let close = seeded + Duration::seconds(3);
    let closing = repin_fixture_window(harness, &stream, close).await?;
    let row = call_stream(
        &app,
        &closing,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("positive imminent-close aggregate absent")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 2, 2)?;
    if !row_value::<bool>(&row, "lineage_valid")
        || row_value::<DateTime<Utc>>(&row, "valid_until") != close
    {
        return Err("currently open validated proof did not retain its actual close boundary");
    }
    repin_fixture_window(harness, &closing, stream.close_at).await?;

    let seeded = set_stream_evidence(harness, &stream, &demands, EvidenceCase::default()).await?;
    seed_newer_admission_fixture(harness, &stream, &first).await?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("old-generation demand disappeared instead of being diagnosed")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 1, 1)?;
    let heartbeat: DateTime<Utc> = row_value(&row, "producer_heartbeat_at");
    if row_value::<bool>(&row, "lineage_valid")
        || row_value::<DateTime<Utc>>(&row, "valid_until") != heartbeat + Duration::seconds(10)
    {
        return Err("old-generation denominator or remaining positive metric assurance changed");
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;

    // Entitlement rights denial is independent of grant activation and of
    // proof freshness: the still-ACTIVE grant must yield no aggregate.
    sqlx::query("UPDATE public.data_entitlements SET status='REVOKED' WHERE id=(SELECT entitlement_id FROM public.owner_market_stream_grants WHERE id=$1)")
        .bind(stream.grant_id).execute(&harness.owner_pool).await
        .map_err(|_| "synthetic entitlement revocation failed")?;
    for (open, close) in [(None, None), (Some(stream.open_at), Some(stream.close_at))] {
        if call_stream_with_bounds(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
            open,
            close,
        )
        .await?
        .is_some()
        {
            return Err("revoked entitlement rights returned state with an active grant");
        }
    }
    sqlx::query("UPDATE public.data_entitlements SET status='ACTIVE' WHERE id=(SELECT entitlement_id FROM public.owner_market_stream_grants WHERE id=$1)")
        .bind(stream.grant_id).execute(&harness.owner_pool).await
        .map_err(|_| "synthetic entitlement rights restoration failed")?;
    if call_stream_with_bounds(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
        None,
        None,
    )
    .await?
    .is_none()
    {
        return Err("restoring entitlement rights failed to restore a non-positive observation");
    }

    // Irreversible synthetic grant revocation is last; it must deny BOTH bound
    // modes. Fixture-only changes do not grant app private-table authority.
    sqlx::query("UPDATE public.owner_market_stream_grants SET state='REVOKED', revoked_at=clock_timestamp() WHERE id=$1")
        .bind(stream.grant_id).execute(&harness.owner_pool).await.map_err(|_| "synthetic rights/grant revocation failed")?;
    for (open, close) in [(None, None), (Some(stream.open_at), Some(stream.close_at))] {
        if call_stream_with_bounds(
            &app,
            &stream,
            stream.owner_id,
            &stream.session_hash,
            &stream.window_sha256,
            open,
            close,
        )
        .await?
        .is_some()
        {
            return Err("revoked rights/grant returned state in nullable or open mode");
        }
    }
    Ok(())
}

#[tokio::test]
async fn service_status_stream_observation_and_positive_assurance_are_distinct() {
    let harness = required_harness().await;
    let result = check_stream_semantics(&harness).await;
    harness.teardown().await;
    assert_eq!(
        result,
        Ok(()),
        "G1 observation/assurance semantic assertion failed"
    );
}

async fn check_proof_deadlines(harness: &Harness) -> Result<(), &'static str> {
    use chrono::{DateTime, Duration, Utc};
    let (stream, expected_expiry) = seed_expiring_stream_fixture(harness).await?;
    let app = actor_session_pool(harness, stream.owner_id, &stream.session_hash).await;
    let first = seed_demand_fixture(harness, &stream).await?;
    let second = seed_second_demand_fixture(harness, &stream, &first).await?;
    seed_live_quote_fixture(harness, &stream, &first).await?;
    seed_live_quote_fixture(harness, &stream, &second).await?;
    let seeded =
        set_stream_evidence(harness, &stream, &[first, second], EvidenceCase::default()).await?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("verified imminent-proof-expiry aggregate absent")?;
    check_observation_delay(&row, seeded)?;
    check_counts(&row, 2, 2, 2)?;
    if row_value::<DateTime<Utc>>(&row, "observed_at") >= expected_expiry {
        return Err("proof fixture preparation exhausted its explicit 8-second calendar margin");
    }
    if !row_value::<bool>(&row, "lineage_valid")
        || row_value::<DateTime<Utc>>(&row, "valid_until") != expected_expiry
    {
        return Err("verified immutable calendar/proof retrieval expiry was not retained");
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;
    // Only the mutable projection is changed. Immutable version and batch
    // retrievals remain byte-for-byte clocks inserted by the seed fixture.
    sqlx::query("UPDATE public.trading_calendars SET retrieved_at=clock_timestamp()-INTERVAL '36 hours 1 second' WHERE source_batch_id=$1")
        .bind(stream.calendar_batch_id).execute(&harness.owner_pool).await.map_err(|_| "stale proof projection fixture failed")?;
    let row = call_stream(
        &app,
        &stream,
        stream.owner_id,
        &stream.session_hash,
        &stream.window_sha256,
    )
    .await?
    .ok_or("stale proof diagnosis absent")?;
    check_counts(&row, 2, 0, 0)?;
    if row_value::<bool>(&row, "lineage_valid")
        || row_value::<DateTime<Utc>>(&row, "valid_until")
            != negative_observation_until(&row, &stream, false)
    {
        return Err("stale proof diagnosis inherited an elapsed proof deadline");
    }
    let immutable: DateTime<Utc> = sqlx::query_scalar(
        "SELECT retrieved_at FROM public.trading_calendar_versions WHERE source_batch_id=$1",
    )
    .bind(stream.calendar_batch_id)
    .fetch_one(&harness.owner_pool)
    .await
    .map_err(|_| "immutable fixture version clock could not be read")?;
    if immutable + Duration::hours(36) != expected_expiry + Duration::seconds(1) {
        return Err("immutable version retrieval clock was rewritten by the proof fixture");
    }
    assert_raw_source_clocks(harness, &stream, &row).await?;
    Ok(())
}

#[tokio::test]
async fn service_status_stream_proof_deadlines_use_immutable_retrievals() {
    let harness = required_harness().await;
    let result = check_proof_deadlines(&harness).await;
    harness.teardown().await;
    assert_eq!(
        result,
        Ok(()),
        "G1 verified proof deadline assertion failed"
    );
}
