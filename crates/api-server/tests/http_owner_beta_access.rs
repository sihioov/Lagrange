//! Owner-beta admission is one router boundary, not a UI convention.  These
//! tests exercise every contract route in the three gated product groups so a
//! newly added direct API route cannot accidentally become Member-visible.

mod common;

use api_server::contract::{CONTRACT_ROUTES, owner_beta_applies};
use axum::http::StatusCode;
use common::{Harness, status};

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
