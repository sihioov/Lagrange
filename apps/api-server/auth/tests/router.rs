//! Todo 22 RED suite: the Axum auth router (login/callback/logout/session/
//! CSRF/invites/step-up) driven over HTTP with the Auth0 simulator.
//!
//! Manual QA channel: `cargo test -p api-server-auth router_qa -- --nocapture`
//! simulates one full login flow - invite redemption -> PKCE callback ->
//! session cookie issued -> authenticated request with CSRF -> logout ->
//! revoked session denied; then Owner step-up with stale auth_time denied and
//! fresh MFA allowed.

use api_server_auth::{RouterState, router};
use auth::audit::{AuthAudit, AuthAuditError, AuthAuditEvent, AuthAuditKind, InMemoryAuthAudit};
use auth::clock::FakeClock;
use auth::entitlement::{Role, UserId};
use auth::invites::{
    InMemoryInviteStore, InMemoryUserStore, InviteError, InviteService, UserRecord, UserStore,
};
use auth::oidc::{
    InMemoryPendingAuthStore, OidcClient, OidcProviderConfig, PendingAuth, PendingAuthStore,
};
use auth::service::AuthService;
use auth::sessions::{InMemorySessionStore, SessionService};
use auth::simulator::{SIM_AUDIENCE, Simulator};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use url::Url;

const NOW: i64 = 1_800_000_000;
const ISSUER: &str = "https://lagrange-test.auth0.com";
const CLIENT_ID: &str = "lagrange-app";
const REDIRECT_URI: &str = "https://app.lagrange.local/auth/callback";

struct TestApp {
    sim: Arc<Simulator>,
    audit: Arc<InMemoryAuthAudit>,
    state: RouterState,
}

struct FailingAudit;

#[async_trait::async_trait]
impl AuthAudit for FailingAudit {
    fn record(&self, _event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        Err(AuthAuditError::Unavailable)
    }

    async fn record_durable(&self, _event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        Err(AuthAuditError::Unavailable)
    }
}

struct CanonicalUserStore {
    inner: InMemoryUserStore,
    canonical_id: UserId,
}

impl CanonicalUserStore {
    fn new(canonical_id: &str) -> Self {
        Self {
            inner: InMemoryUserStore::default(),
            canonical_id: UserId::new(canonical_id),
        }
    }
}

#[async_trait::async_trait]
impl UserStore for CanonicalUserStore {
    async fn find_by_binding(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<UserRecord>, InviteError> {
        self.inner.find_by_binding(issuer, subject).await
    }

    async fn insert_user(&self, mut user: UserRecord) -> Result<UserId, InviteError> {
        user.user_id = self.canonical_id.clone();
        self.inner.insert_user(user).await?;
        Ok(self.canonical_id.clone())
    }

    async fn update_profile(&self, user_id: &str, email: &str) -> Result<(), InviteError> {
        self.inner.update_profile(user_id, email).await
    }
}

fn app() -> TestApp {
    app_with_users(Arc::new(InMemoryUserStore::default()))
}

fn canonical_app() -> TestApp {
    app_with_users(Arc::new(CanonicalUserStore::new(
        "00000000-0000-0000-0000-000000000042",
    )))
}

fn app_with_users(users: Arc<dyn UserStore>) -> TestApp {
    let sim = Arc::new(Simulator::new(ISSUER, CLIENT_ID, REDIRECT_URI));
    let cfg = OidcProviderConfig {
        issuer: ISSUER.to_string(),
        client_id: CLIENT_ID.to_string(),
        redirect_uri: REDIRECT_URI.to_string(),
        authorize_url: format!("{ISSUER}/authorize"),
        token_url: format!("{ISSUER}/oauth/token"),
        jwks_url: format!("{ISSUER}/.well-known/jwks.json"),
        audience: Some(SIM_AUDIENCE.to_string()),
        clock_skew_secs: 60,
    };
    let audit = Arc::new(InMemoryAuthAudit::default());
    let pending = Arc::new(InMemoryPendingAuthStore::default());
    let invites = InviteService::new(
        Arc::new(InMemoryInviteStore::default()),
        users,
        Arc::new(FakeClock(NOW)),
        audit.clone(),
    );
    let sessions = SessionService::new(
        Arc::new(InMemorySessionStore::default()),
        Arc::new(FakeClock(NOW)),
        audit.clone(),
    );
    let oidc = OidcClient {
        config: cfg,
        transport: sim.clone(),
    };
    let auth = AuthService::new(oidc, invites, sessions, audit.clone());
    let state = RouterState {
        auth: Arc::new(auth),
        pending,
        audit: audit.clone(),
        step_up_max_auth_age_secs: 900,
        transaction_cookie_key: Arc::new([0x42; 32]),
        durable_audit: None,
    };
    TestApp { sim, audit, state }
}

async fn get(app: &TestApp, path: &str) -> (StatusCode, Value, Vec<(String, String)>) {
    let res = router(app.state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    respond(res).await
}

async fn get_with_cookie(
    app: &TestApp,
    path: &str,
    cookie: &str,
    csrf: Option<&str>,
) -> (StatusCode, Value, Vec<(String, String)>) {
    let mut builder = Request::builder().uri(path);
    builder = builder.header(header::COOKIE, cookie);
    if let Some(t) = csrf {
        builder = builder.header("X-CSRF-Token", t);
    }
    let res = router(app.state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    respond(res).await
}

async fn post_json(
    app: &TestApp,
    path: &str,
    cookie: &str,
    csrf: &str,
    body: Value,
) -> (StatusCode, Value, Vec<(String, String)>) {
    let res = router(app.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header("X-CSRF-Token", csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    respond(res).await
}

async fn respond(res: axum::response::Response) -> (StatusCode, Value, Vec<(String, String)>) {
    let status = res.status();
    let headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value, headers)
}

fn set_cookie_value(headers: &[(String, String)]) -> String {
    let set = headers
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.clone())
        .expect("Set-Cookie header");
    set.split(';').next().unwrap().trim().to_string()
}

#[tokio::test]
async fn oidc_transaction_cookie_is_secure_short_lived_and_bound_to_browser() {
    let app = app();
    let (status, _, first_headers) = get(&app, "/auth/login").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let first_cookie = first_headers
        .iter()
        .find(|(key, _)| key == "set-cookie")
        .map(|(_, value)| value.clone())
        .expect("transaction cookie");
    assert!(first_cookie.starts_with("__Host-lagrange_oidc_tx="));
    assert!(first_cookie.contains("Path=/"));
    assert!(first_cookie.contains("Secure"));
    assert!(first_cookie.contains("HttpOnly"));
    assert!(first_cookie.contains("SameSite=Lax"));
    assert!(first_cookie.contains("Max-Age=300"));

    let (_, _, second_headers) = get(&app, "/auth/login").await;
    let second_cookie = second_headers
        .iter()
        .find(|(key, _)| key == "set-cookie")
        .map(|(_, value)| value.clone())
        .expect("second transaction cookie");
    assert_ne!(
        first_cookie, second_cookie,
        "each browser transaction is unique"
    );

    let location = first_headers
        .iter()
        .find(|(key, _)| key == "location")
        .map(|(_, value)| value.clone())
        .expect("login location");
    let state = Url::parse(&location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let (status, body, headers) = get_with_cookie(
        &app,
        &format!("/auth/callback?code=irrelevant&state={state}"),
        "__Host-lagrange_oidc_tx=attacker-value",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "OIDC_TRANSACTION_MISMATCH");
    assert!(headers.iter().any(|(key, value)| {
        key == "set-cookie"
            && value.starts_with("__Host-lagrange_oidc_tx=")
            && value.contains("Max-Age=0")
            && value.contains("Secure")
            && value.contains("HttpOnly")
            && value.contains("SameSite=Lax")
    }));
}

/// Drives one full login over HTTP: /auth/login -> /auth/callback ->
/// session cookie + CSRF -> authenticated /auth/session.
async fn http_login(
    app: &TestApp,
    sub: &str,
    email: &str,
    roles: &[&str],
    auth_time: i64,
    amr: &[&str],
) -> (String, String) {
    let (status, _, headers) = get(app, "/auth/login").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let transaction_cookie = set_cookie_value(&headers);
    let location = headers
        .iter()
        .find(|(k, _)| k == "location")
        .map(|(_, v)| v.clone())
        .expect("Location header");
    let url = Url::parse(&location).unwrap();
    let q = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    let state = q("state");
    let nonce = q("nonce");
    // The provider "logs the user in": mint the auth code bound to the
    // verifier the SERVER generated (read from the pending store).
    let pending = {
        let store = app.state.pending.clone();
        take_pending(&*store, &state)
            .await
            .expect("pending stored at login")
    };
    let code = app.sim.issue_code(
        json!({
            "iss": ISSUER,
            "sub": sub,
            "aud": [SIM_AUDIENCE],
            "exp": NOW + 3600,
            "iat": NOW,
            "nonce": nonce,
            "email": email,
            "email_verified": true,
            "auth_time": auth_time,
            "amr": amr,
            "roles": roles,
        }),
        &pending.code_verifier,
    );

    let (status, _, headers) = get_with_cookie(
        app,
        &format!("/auth/callback?code={code}&state={state}"),
        &transaction_cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "callback redirects home");
    let cookie = set_cookie_value(&headers);
    let csrf = headers
        .iter()
        .find(|(k, _)| k == "x-csrf-token")
        .map(|(_, v)| v.clone())
        .expect("X-CSRF-Token header");
    (cookie, csrf)
}

async fn take_pending(store: &dyn PendingAuthStore, state: &str) -> Option<PendingAuth> {
    // Peek without consuming: the callback consumes the record itself.
    let p = store.take(state).await.unwrap();
    if let Some(p) = p {
        store.insert(state.to_string(), p.clone()).await.unwrap();
        Some(p)
    } else {
        None
    }
}

#[tokio::test]
async fn first_callback_uses_canonical_user_id_and_replay_is_denied() {
    const CANONICAL_ID: &str = "00000000-0000-0000-0000-000000000042";

    let app = canonical_app();
    app.state
        .auth
        .invites
        .create_invite("canonical@example.com", Role::Member, 3600)
        .await
        .unwrap();

    let (status, _, headers) = get(&app, "/auth/login").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let transaction_cookie = set_cookie_value(&headers);
    let location = headers
        .iter()
        .find(|(key, _)| key == "location")
        .map(|(_, value)| value.clone())
        .expect("login location");
    let url = Url::parse(&location).unwrap();
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("login state");
    let nonce = url
        .query_pairs()
        .find(|(key, _)| key == "nonce")
        .map(|(_, value)| value.into_owned())
        .expect("login nonce");
    let pending = take_pending(&*app.state.pending, &state)
        .await
        .expect("pending login");
    let code = app.sim.issue_code(
        json!({
            "iss": ISSUER,
            "sub": "auth0|canonical-1",
            "aud": [SIM_AUDIENCE],
            "exp": NOW + 3600,
            "iat": NOW,
            "nonce": nonce,
            "email": "canonical@example.com",
            "email_verified": true,
            "auth_time": NOW - 60,
            "amr": ["pwd"],
            "roles": ["member"],
        }),
        &pending.code_verifier,
    );
    let callback_path = format!("/auth/callback?code={code}&state={state}");

    let (status, _, headers) =
        get_with_cookie(&app, &callback_path, &transaction_cookie, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = set_cookie_value(&headers);
    let (status, body, _) = get_with_cookie(&app, "/auth/session", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user_id"], CANONICAL_ID);

    let (status, body, _) = get_with_cookie(&app, &callback_path, &transaction_cookie, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "STATE_MISMATCH");
}

#[tokio::test]
async fn router_qa_full_login_logout_and_step_up() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("own@example.com", Role::Member, 3600)
        .await
        .unwrap();
    println!();
    println!("ROUTER QA: full HTTP login flow via the Auth0 simulator");
    println!("{}", "-".repeat(96));

    let (status, body, _) = get(&app, "/auth/login").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    println!("1. GET /auth/login -> 303 {body:?}");

    let (cookie, csrf) = http_login(
        &app,
        "auth0|own-1",
        "own@example.com",
        &["owner"],
        NOW - 60,
        &["pwd", "mfa"],
    )
    .await;
    assert!(cookie.starts_with("__Host-lagrange_session="));
    println!("2. GET /auth/callback -> session cookie + X-CSRF-Token issued");
    println!("   cookie: {cookie}");
    println!("   csrf:   {csrf}");

    let (status, body, _) = get_with_cookie(&app, "/auth/session", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    let uid = body["user_id"].as_str().unwrap();
    assert!(
        uid.starts_with("usr_"),
        "internal user id, not email: {uid}"
    );
    assert_ne!(uid, "own@example.com");
    assert_eq!(body["role"], "owner");
    println!("3. GET /auth/session -> 200 {body}");

    let (status, _, _) = post_json(&app, "/auth/logout", &cookie, "wrong-csrf", json!({})).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "logout without valid CSRF denied"
    );
    println!("4. POST /auth/logout with wrong CSRF -> 403");

    let (status, _, headers) = post_json(&app, "/auth/logout", &cookie, &csrf, json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let clear = headers
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert!(clear.contains("Max-Age=0"));
    println!("5. POST /auth/logout with CSRF -> 204, cookie cleared");

    let (status, body, _) = get_with_cookie(&app, "/auth/session", &cookie, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "revoked session denied");
    println!("6. GET /auth/session after logout -> 401 {body}");

    // Owner step-up over HTTP: stale auth_time denied, fresh MFA allowed.
    let (stale_cookie, _) = http_login(
        &app,
        "auth0|own-1",
        "own@example.com",
        &["owner"],
        NOW - 901,
        &["pwd", "mfa"],
    )
    .await;
    let (status, body, _) = get_with_cookie(&app, "/auth/step-up-check", &stale_cookie, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "STEP_UP_AUTH_TIME_STALE");
    println!("7. Owner step-up with stale auth_time -> 403 {body}");

    let (fresh_cookie, _) = http_login(
        &app,
        "auth0|own-1",
        "own@example.com",
        &["owner"],
        NOW - 60,
        &["pwd", "mfa"],
    )
    .await;
    let (status, body, _) = get_with_cookie(&app, "/auth/step-up-check", &fresh_cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    println!("8. Owner step-up with fresh MFA -> 200 {body}");
    println!("{}", "-".repeat(96));
    println!(
        "ROUTER QA PASSED: login -> cookie -> CSRF -> logout -> revoked denied; stale step-up denied; fresh MFA allowed."
    );
}

#[tokio::test]
async fn logout_failure_does_not_clear_live_cookie() {
    let sim = Arc::new(Simulator::new(ISSUER, CLIENT_ID, REDIRECT_URI));
    let audit: Arc<dyn AuthAudit> = Arc::new(FailingAudit);
    let cfg = OidcProviderConfig {
        issuer: ISSUER.to_string(),
        client_id: CLIENT_ID.to_string(),
        redirect_uri: REDIRECT_URI.to_string(),
        authorize_url: format!("{ISSUER}/authorize"),
        token_url: format!("{ISSUER}/oauth/token"),
        jwks_url: format!("{ISSUER}/.well-known/jwks.json"),
        audience: Some(SIM_AUDIENCE.to_string()),
        clock_skew_secs: 60,
    };
    let auth = AuthService::new(
        OidcClient {
            config: cfg,
            transport: sim,
        },
        InviteService::new(
            Arc::new(InMemoryInviteStore::default()),
            Arc::new(InMemoryUserStore::default()),
            Arc::new(FakeClock(NOW)),
            audit.clone(),
        ),
        SessionService::new(
            Arc::new(InMemorySessionStore::default()),
            Arc::new(FakeClock(NOW)),
            audit,
        ),
        Arc::new(FailingAudit),
    );
    let state = RouterState {
        auth: Arc::new(auth),
        pending: Arc::new(InMemoryPendingAuthStore::default()),
        audit: Arc::new(FailingAudit),
        step_up_max_auth_age_secs: 900,
        transaction_cookie_key: Arc::new([0x42; 32]),
        durable_audit: None,
    };
    let identity = auth::invites::RedeemedIdentity {
        user_id: UserId::new("00000000-0000-0000-0000-000000000099"),
        role: Role::Owner,
        email: "owner@example.test".to_string(),
        binding: "issuer|subject".to_string(),
    };
    let issued = state
        .auth
        .sessions
        .issue(&identity, NOW, vec!["mfa".to_string()])
        .await
        .expect("session issue");
    let cookie_header = format!("{}={}", auth::sessions::cookie::NAME, issued.cookie_value);
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header(header::COOKIE, cookie_header)
                .header("X-CSRF-Token", issued.csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        !response.headers().contains_key(header::SET_COOKIE),
        "failed durable logout must not clear the browser cookie"
    );
    assert!(
        state.auth.session_info(&issued.cookie_value).await.is_ok(),
        "session remains live when durable revocation fails"
    );
}

#[tokio::test]
async fn login_redirects_with_pkce_and_exact_redirect() {
    let app = app();
    let (status, _, headers) = get(&app, "/auth/login").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers
        .iter()
        .find(|(k, _)| k == "location")
        .unwrap()
        .1
        .clone();
    let url = Url::parse(&location).unwrap();
    let q = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.into_owned())
    };
    assert_eq!(q("redirect_uri").as_deref(), Some(REDIRECT_URI));
    assert_eq!(q("code_challenge_method").as_deref(), Some("S256"));
    assert!(q("code_challenge").unwrap().len() >= 40);
    assert!(q("state").is_some());
    assert!(q("nonce").is_some());
}

#[tokio::test]
async fn callback_without_state_is_denied() {
    let app = app();
    let (status, body, _) = get(&app, "/auth/callback?code=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing state param");
    assert_eq!(body["error"]["code"], "MISSING_STATE");
}

#[tokio::test]
async fn session_endpoint_requires_auth() {
    let app = app();
    let (status, body, _) = get(&app, "/auth/session").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "SESSION_UNKNOWN");
}

#[tokio::test]
async fn csrf_endpoint_requires_auth_and_rotates() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("m@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let (cookie, csrf1) = http_login(
        &app,
        "auth0|m-1",
        "m@example.com",
        &["member"],
        NOW - 60,
        &["pwd"],
    )
    .await;
    let (status, body, _) = get_with_cookie(&app, "/auth/csrf", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    let rotated = body["csrf_token"].as_str().unwrap().to_string();
    assert_ne!(rotated, csrf1, "rotation issues a fresh token");
    // The rotated token is the one that now verifies.
    let (status, _, _) = post_json(&app, "/auth/logout", &cookie, &csrf1, json!({})).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "stale csrf denied after rotation"
    );
    let (status, _, _) = post_json(&app, "/auth/logout", &cookie, &rotated, json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn invites_are_owner_only_and_csrf_protected() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("own@example.com", Role::Owner, 3600)
        .await
        .unwrap();
    app.state
        .auth
        .invites
        .create_invite("mem@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let (member_cookie, member_csrf) = http_login(
        &app,
        "auth0|m-1",
        "mem@example.com",
        &["member"],
        NOW - 60,
        &["pwd"],
    )
    .await;
    let (owner_cookie, owner_csrf) = http_login(
        &app,
        "auth0|o-1",
        "own@example.com",
        &["owner"],
        NOW - 60,
        &["pwd", "mfa"],
    )
    .await;

    let (status, body, _) = post_json(
        &app,
        "/auth/invites",
        &member_cookie,
        &member_csrf,
        json!({"email": "new@example.com", "role": "member"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Member cannot invite: {body}"
    );
    assert_eq!(body["error"]["code"], "INVITE_NOT_OWNER");

    let (status, body, _) = post_json(
        &app,
        "/auth/invites",
        &owner_cookie,
        "wrong-token",
        json!({"email": "new@example.com", "role": "member"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "missing CSRF denied: {body}");
    assert_eq!(body["error"]["code"], "CSRF_DENIED");
    assert!(
        app.audit
            .has(AuthAuditKind::CsrfDenied, Some("CSRF_DENIED"))
    );

    let (status, body, _) = post_json(
        &app,
        "/auth/invites",
        &owner_cookie,
        &owner_csrf,
        json!({"email": "new@example.com", "role": "member"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "owner creates invite: {body}");
    assert_eq!(body["email"], "new@example.com");
    assert_eq!(body["role"], "member");
    assert!(
        app.audit
            .events()
            .iter()
            .any(|event| event.kind == AuthAuditKind::InviteCreated && event.user.is_some())
    );
}

#[tokio::test]
async fn invalid_invite_body_is_rejected() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("own@example.com", Role::Owner, 3600)
        .await
        .unwrap();
    let (owner_cookie, owner_csrf) = http_login(
        &app,
        "auth0|o-1",
        "own@example.com",
        &["owner"],
        NOW - 60,
        &["pwd", "mfa"],
    )
    .await;
    let (status, body, _) = post_json(
        &app,
        "/auth/invites",
        &owner_cookie,
        &owner_csrf,
        json!({"email": "not-an-email", "role": "owner"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid email: {body}");
    let (status, _, _) = post_json(
        &app,
        "/auth/invites",
        &owner_cookie,
        &owner_csrf,
        json!({"email": "a@b.com", "role": "superadmin"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid role");
}

#[tokio::test]
async fn login_after_email_profile_change_resolves_same_user_over_http() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("first@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let (cookie1, _) = http_login(
        &app,
        "auth0|stable-1",
        "first@example.com",
        &[],
        NOW - 60,
        &["pwd"],
    )
    .await;
    let (_, body1, _) = get_with_cookie(&app, "/auth/session", &cookie1, None).await;
    let (cookie2, _) = http_login(
        &app,
        "auth0|stable-1",
        "changed@example.com",
        &[],
        NOW - 60,
        &["pwd"],
    )
    .await;
    let (_, body2, _) = get_with_cookie(&app, "/auth/session", &cookie2, None).await;
    assert_eq!(
        body1["user_id"], body2["user_id"],
        "same (iss,sub) -> same internal user over HTTP"
    );
    assert_ne!(cookie1, cookie2, "fresh cookie per login");
}

#[tokio::test]
async fn fixation_attack_cookie_never_becomes_the_session() {
    let app = app();
    app.state
        .auth
        .invites
        .create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    // Attacker pre-sets a victim cookie value; after the victim logs in, the
    // served cookie must differ (new value minted server-side).
    let attacker = "__Host-lagrange_session=attacker-chosen-value";
    let (cookie, _) = http_login(&app, "auth0|u-1", "a@example.com", &[], NOW - 60, &["pwd"]).await;
    assert_ne!(cookie, attacker);
    let (status, _, _) = get_with_cookie(&app, "/auth/session", attacker, None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "attacker-chosen cookie never authenticates"
    );
}

#[tokio::test]
async fn uninvited_login_via_http_is_denied_and_audited() {
    let app = app();
    let (status, body) = http_login_denied(&app, "auth0|stranger", "stranger@example.com").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "INVITE_NOT_FOUND");
    assert!(
        app.audit
            .has(AuthAuditKind::LoginDenied, Some("INVITE_NOT_FOUND"))
    );
}

async fn http_login_denied(app: &TestApp, sub: &str, email: &str) -> (StatusCode, Value) {
    let (_, _, headers) = get(app, "/auth/login").await;
    let transaction_cookie = set_cookie_value(&headers);
    let location = headers
        .iter()
        .find(|(k, _)| k == "location")
        .unwrap()
        .1
        .clone();
    let url = Url::parse(&location).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let nonce = url
        .query_pairs()
        .find(|(k, _)| k == "nonce")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let pending = {
        let store = app.state.pending.clone();
        take_pending(&*store, &state).await.unwrap()
    };
    let code = app.sim.issue_code(
        json!({
            "iss": ISSUER,
            "sub": sub,
            "aud": [SIM_AUDIENCE],
            "exp": NOW + 3600,
            "iat": NOW,
            "nonce": nonce,
            "email": email,
            "email_verified": true,
            "auth_time": NOW - 60,
            "amr": ["pwd"],
            "roles": [],
        }),
        &pending.code_verifier,
    );
    let (status, body, _) = get_with_cookie(
        app,
        &format!("/auth/callback?code={code}&state={state}"),
        &transaction_cookie,
        None,
    )
    .await;
    (status, body)
}
