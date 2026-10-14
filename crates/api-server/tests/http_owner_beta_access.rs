//! Owner-beta admission is one router boundary, not a UI convention.  These
//! tests exercise every contract route in the three gated product groups so a
//! newly added direct API route cannot accidentally become Member-visible.

mod common;

use api_server::contract::{CONTRACT_ROUTES, owner_beta_applies};
use axum::http::StatusCode;
use common::{Harness, status};
use sqlx::Connection;
use std::time::Duration;

fn concrete_path(template: &str) -> String {
    template
        .replace("{run_id}", "00000000-0000-0000-0000-000000000001")
        .replace("{account_id}", "00000000-0000-0000-0000-000000000002")
        .replace("{preview_id}", "00000000-0000-0000-0000-000000000003")
}

#[tokio::test]
async fn http_owner_beta_access_is_central_exhaustive_and_non_leaking() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };

    // The disabled default retains the normal multi-user API contract.
    for path in [
        "/api/v1/recommendations/runs",
        "/api/v1/backtests",
        "/api/v1/paper/accounts",
    ] {
        let response = h.get(path, Some(&h.member)).await;
        assert_eq!(response.status(), StatusCode::OK, "default mode: {path}");
    }

    h.restart_api_with_owner_beta_access().await;

    let protected_routes = CONTRACT_ROUTES
        .iter()
        .filter(|route| owner_beta_applies(route.path))
        .collect::<Vec<_>>();
    assert!(
        !protected_routes.is_empty(),
        "beta route inventory must not be empty"
    );

    for route in protected_routes {
        let path = concrete_path(route.path);
        let member = h
            .send(
                route.method,
                &path,
                Some(&h.member),
                false,
                Some("owner-beta-member"),
                None,
                None,
            )
            .await;
        assert_eq!(
            status(&member),
            StatusCode::FORBIDDEN,
            "Member must be denied before handler input validation: {} {}",
            route.method,
            route.path
        );
        let member_body = Harness::body_json(member).await;
        assert_eq!(Harness::error_code(&member_body), "FORBIDDEN");
        assert_eq!(member_body["error"]["message"], "forbidden");

        let anonymous = h
            .send(
                route.method,
                &path,
                None,
                false,
                Some("owner-beta-anonymous"),
                None,
                None,
            )
            .await;
        assert_eq!(
            status(&anonymous),
            StatusCode::UNAUTHORIZED,
            "unauthenticated request keeps the ordinary session contract: {} {}",
            route.method,
            route.path
        );
        let anonymous_body = Harness::body_json(anonymous).await;
        assert_eq!(Harness::error_code(&anonymous_body), "SESSION_UNKNOWN");
    }

    // Owner reaches recommendation/backtest handlers, while the separate
    // Paper default is rejected centrally before handler input or data access.
    for path in ["/api/v1/recommendations/runs", "/api/v1/backtests"] {
        let response = h.get(path, Some(&h.owner)).await;
        assert_eq!(response.status(), StatusCode::OK, "Owner reaches {path}");
    }
    for route in CONTRACT_ROUTES
        .iter()
        .filter(|route| route.path.starts_with("/api/v1/paper/"))
    {
        let path = concrete_path(route.path);
        let owner = h
            .send(
                route.method,
                &path,
                Some(&h.owner),
                false,
                Some("owner-beta-paper-disabled"),
                None,
                None,
            )
            .await;
        assert_eq!(status(&owner), StatusCode::FORBIDDEN);
        let body = Harness::body_json(owner).await;
        assert_eq!(Harness::error_code(&body), "FORBIDDEN");
        assert_eq!(body["error"]["message"], "forbidden");
    }

    h.restart_api_with_owner_beta_paper().await;
    for path in [
        "/api/v1/recommendations/runs",
        "/api/v1/backtests",
        "/api/v1/paper/accounts",
    ] {
        let response = h.get(path, Some(&h.owner)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "enabled Owner reaches {path}"
        );
    }

    // Authentication and unrelated product routes retain their existing
    // behavior; the temporary beta gate is not a global Member lockout.
    assert_eq!(
        h.get("/api/v1/auth/session", Some(&h.member))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.get("/api/v1/strategies", Some(&h.member)).await.status(),
        StatusCode::OK
    );

    h.teardown().await;
}

#[tokio::test]
async fn http_daily_input_admitter_pool_is_direct_bounded_and_unprivileged() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };

    let pool = h.daily_input_admitter_pool().await;
    let check_result: Result<(), &'static str> = async {
        if pool.options().get_max_connections() != 2 {
            return Err("admitter pool max_connections is not two");
        }

        let direct_identity_sql = "SELECT current_user, session_user, pg_backend_pid(), current_setting('role'), current_setting('app.actor_user_id', true), current_setting('app.actor_id', true)";
        let is_direct_admitter = |identity: &(
            String,
            String,
            i32,
            String,
            Option<String>,
            Option<String>,
        )| {
            identity.0.as_str() == "daily_input_admitter"
                && identity.1.as_str() == "daily_input_admitter"
                && identity.3.as_str() == "none"
                && identity.4.is_none()
                && identity.5.is_none()
        };

        let mut first = pool
            .acquire()
            .await
            .map_err(|_| "first admitter connection could not be acquired")?;
        let mut second = pool
            .acquire()
            .await
            .map_err(|_| "second admitter connection could not be acquired")?;
        if pool.size() > 2 {
            return Err("admitter pool exceeded two connections");
        }

        let first_identity: (String, String, i32, String, Option<String>, Option<String>) =
            sqlx::query_as(direct_identity_sql)
                .fetch_one(&mut *first)
                .await
                .map_err(|_| "first admitter identity query failed")?;
        let second_identity: (String, String, i32, String, Option<String>, Option<String>) =
            sqlx::query_as(direct_identity_sql)
                .fetch_one(&mut *second)
                .await
                .map_err(|_| "second admitter identity query failed")?;
        if !is_direct_admitter(&first_identity) || !is_direct_admitter(&second_identity) {
            return Err("held connections are not direct, actor-free admitter sessions");
        }
        if first_identity.2 == second_identity.2 {
            return Err("held admitter connections share a backend PID");
        }

        if tokio::time::timeout(Duration::from_millis(100), pool.acquire())
            .await
            .is_ok()
        {
            return Err("third admitter acquire completed while both connections were held");
        }

        drop(second);
        let mut third = match tokio::time::timeout(Duration::from_secs(10), pool.acquire()).await {
            Ok(Ok(connection)) => connection,
            _ => return Err("admitter acquire did not resume within ten seconds"),
        };
        if pool.size() > 2 {
            return Err("admitter pool exceeded two connections after reacquire");
        }
        let third_identity: (String, String, i32, String, Option<String>, Option<String>) =
            sqlx::query_as(direct_identity_sql)
                .fetch_one(&mut *third)
                .await
                .map_err(|_| "reacquired admitter identity query failed")?;
        if !is_direct_admitter(&third_identity) {
            return Err("reacquired connection is not a direct, actor-free admitter session");
        }

        let role_attributes: (bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT rolcanlogin, rolinherit, rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls FROM pg_roles WHERE rolname = current_user",
        )
        .fetch_one(&mut *first)
        .await
        .map_err(|_| "admitter role attributes query failed")?;
        if role_attributes != (true, false, false, false, false, false, false) {
            return Err("admitter role attributes do not match the exact restricted set");
        }

        let privileges: (bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_database_privilege(current_user, current_database(), 'CONNECT'), has_database_privilege(current_user, current_database(), 'CREATE'), has_schema_privilege(current_user, 'public', 'USAGE'), has_schema_privilege(current_user, 'public', 'CREATE'), NOT EXISTS (SELECT 1 FROM pg_auth_members WHERE member = (SELECT oid FROM pg_roles WHERE rolname = current_user) OR roleid = (SELECT oid FROM pg_roles WHERE rolname = current_user))",
        )
        .fetch_one(&mut *first)
        .await
        .map_err(|_| "admitter effective privilege query failed")?;
        if privileges != (true, false, true, false, true) {
            return Err("admitter effective privileges or membership edges are unexpected");
        }

        let mut create_tx = first
            .begin()
            .await
            .map_err(|_| "admitter CREATE probe transaction could not begin")?;
        let create_result =
            sqlx::query("CREATE TABLE public.r2_admitter_forbidden_probe(id integer)")
                .execute(&mut *create_tx)
                .await;
        let create_denied = match &create_result {
            Err(error) => {
                error.as_database_error().and_then(|error| error.code()).as_deref()
                    == Some("42501")
            }
            Ok(_) => false,
        };
        create_tx
            .rollback()
            .await
            .map_err(|_| "admitter CREATE probe transaction rollback failed")?;
        if !create_denied {
            return Err("admitter CREATE probe was not denied with SQLSTATE 42501");
        }

        let mut set_role_tx = first
            .begin()
            .await
            .map_err(|_| "admitter SET ROLE probe transaction could not begin")?;
        let set_role_result = sqlx::query("SET ROLE migration_owner")
            .execute(&mut *set_role_tx)
            .await;
        let set_role_denied = match &set_role_result {
            Err(error) => {
                error.as_database_error().and_then(|error| error.code()).as_deref()
                    == Some("42501")
            }
            Ok(_) => false,
        };
        set_role_tx
            .rollback()
            .await
            .map_err(|_| "admitter SET ROLE probe transaction rollback failed")?;
        if !set_role_denied {
            return Err("admitter SET ROLE probe was not denied with SQLSTATE 42501");
        }

        let first_after_rollback: (String, String, i32, String, Option<String>, Option<String>) =
            sqlx::query_as(direct_identity_sql)
                .fetch_one(&mut *first)
                .await
                .map_err(|_| "first post-rollback identity query failed")?;
        let third_after_rollback: (String, String, i32, String, Option<String>, Option<String>) =
            sqlx::query_as(direct_identity_sql)
                .fetch_one(&mut *third)
                .await
                .map_err(|_| "second post-rollback identity query failed")?;
        if !is_direct_admitter(&first_after_rollback)
            || !is_direct_admitter(&third_after_rollback)
        {
            return Err("post-rollback sessions changed identity, role, or actor settings");
        }
        if pool.size() > 2 {
            return Err("admitter pool exceeded two connections after rollback probes");
        }

        Ok(())
    }
    .await;

    pool.close().await;
    h.teardown().await;
    if let Err(reason) = check_result {
        panic!("{reason}");
    }
}
