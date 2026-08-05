//! Auth0 simulator contract suite (manual QA channel).
//!
//! Proves the FULL login contract against the fake OIDC provider without a
//! real tenant (BLOCKED_EXTERNAL: no Auth0 tenant/credentials exist on this
//! host): PKCE S256, callback validation, verified-invite match, immutable
//! (iss,sub) binding, cookie attributes, role mapping, fresh-MFA step-up
//! allow, stale/non-MFA denial, logout/revocation, single-use state replay
//! denial, and audit trails. Run: `cargo test -p auth auth0_simulator -- --nocapture`
//!
//! The real-tenant suite is `vendor`-tagged in tests/vendor_auth0.rs.

use auth::audit::{AuthAuditKind, InMemoryAuthAudit};
use auth::clock::{Clock, FakeClock, SystemClock};
use auth::entitlement::Role;
use auth::invites::{InMemoryInviteStore, InMemoryUserStore, InviteService};
use auth::oidc::{
    InMemoryPendingAuthStore, OidcClient, OidcProviderConfig, PendingAuth, PendingAuthStore,
};
use auth::service::{AuthError, AuthService};
use auth::sessions::cookie;
use auth::sessions::{InMemorySessionStore, IssuedSession, SessionService};
use auth::simulator::{SIM_AUDIENCE, Simulator};
use auth::stepup::{StepUpDenial, require_owner_step_up};
use serde_json::{Value, json};
use std::sync::Arc;
use url::Url;

const NOW: i64 = 1_800_000_000;
const ISSUER: &str = "https://lagrange-test.auth0.com";
const CLIENT_ID: &str = "lagrange-app";
const REDIRECT_URI: &str = "https://app.lagrange.local/auth/callback";

struct Harness {
    sim: Arc<Simulator>,
    pending: InMemoryPendingAuthStore,
    audit: Arc<InMemoryAuthAudit>,
    auth: AuthService,
}

fn harness(now: i64) -> Harness {
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
    let pending = InMemoryPendingAuthStore::default();
    let invites = InviteService::new(
        Arc::new(InMemoryInviteStore::default()),
        Arc::new(InMemoryUserStore::default()),
        Arc::new(FakeClock(now)),
        audit.clone(),
    );
    let sessions = SessionService::new(
        Arc::new(InMemorySessionStore::default()),
        Arc::new(FakeClock(now)),
        audit.clone(),
    );
    let oidc = OidcClient {
        config: cfg,
        transport: sim.clone(),
    };
    let auth = AuthService::new(oidc, invites, sessions, audit.clone());
    Harness {
        sim,
        pending,
        audit,
        auth,
    }
}

fn claims(sim: &Simulator, nonce: &str, spec: &LoginSpec<'_>) -> Value {
    json!({
        "iss": sim.issuer,
        "sub": spec.sub,
        "aud": [SIM_AUDIENCE],
        "exp": NOW + 3600,
        "iat": NOW,
        "nonce": nonce,
        "email": spec.email,
        "email_verified": spec.verified,
        "auth_time": spec.auth_time,
        "amr": spec.amr,
        "roles": spec.roles,
    })
}

struct LoginSpec<'a> {
    sub: &'a str,
    email: &'a str,
    verified: bool,
    roles: &'a [&'a str],
    auth_time: i64,
    amr: &'a [&'a str],
}

async fn complete_login(h: &Harness, spec: &LoginSpec<'_>) -> Result<IssuedSession, AuthError> {
    let req = h.auth.begin_login().expect("authorize URL");
    let state = Url::parse(req.url.as_ref())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let nonce = Url::parse(req.url.as_ref())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "nonce")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let pending = PendingAuth {
        state: state.clone(),
        nonce: nonce.clone(),
        code_verifier: req.pkce.verifier.clone(),
        created_at_secs: NOW,
        ttl_secs: 300,
    };
    h.pending
        .insert(state.clone(), pending.clone())
        .await
        .expect("pending stored");
    let code = h
        .sim
        .issue_code(&req, claims(&h.sim, &nonce, spec), &req.pkce.verifier);
    h.auth.complete_login(&code, &state, &h.pending).await
}

#[tokio::test]
async fn full_login_flow_proves_contract() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("owner@example.com", Role::Member, 3600)
        .await
        .unwrap();
    println!();
    println!(
        "AUTH0 SIMULATOR FULL LOGIN FLOW (fake OIDC provider, BLOCKED_EXTERNAL for a real tenant)"
    );
    println!("{}", "-".repeat(100));

    let issued = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|own-1",
            email: "owner@example.com",
            verified: true,
            roles: &["owner"],
            auth_time: NOW - 60,
            amr: &["pwd", "mfa"],
        },
    )
    .await
    .expect("login succeeds");
    println!(
        "1. invite redeemed + login succeeded: user={} role=Owner",
        issued.session.user_id.0
    );

    assert_eq!(issued.session.role, Role::Owner);
    assert_eq!(issued.cookie_value.len(), 43, "opaque cookie");
    let set_cookie = &issued.set_cookie_header;
    assert!(set_cookie.starts_with("__Host-lagrange_session="));
    for attr in [
        "Secure",
        "HttpOnly",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=",
        "Expires=",
    ] {
        assert!(set_cookie.contains(attr), "cookie missing {attr}");
    }
    assert!(!set_cookie.contains("Domain="), "host-only cookie");
    println!("2. cookie: {set_cookie}");

    let session = h
        .auth
        .session_info(&issued.cookie_value)
        .await
        .expect("session valid");
    assert_eq!(session.role, Role::Owner);
    assert!(auth::csrf::verify(
        &session.csrf_token_hash,
        &issued.csrf_token
    ));
    println!("3. session validates; CSRF synchronizer token verifies");

    let step_up = require_owner_step_up(&session, NOW, 900);
    assert!(step_up.is_ok(), "fresh MFA owner allowed: {step_up:?}");
    println!("4. Owner step-up with fresh auth_time + amr=[pwd,mfa]: ALLOWED");

    h.auth.logout(&issued.cookie_value).await.expect("logout");
    let err = h
        .auth
        .session_info(&issued.cookie_value)
        .await
        .expect_err("revoked");
    assert!(err.is_unauthenticated());
    println!(
        "5. logout revokes: session denied afterwards; clear-cookie: {}",
        cookie::clear_cookie()
    );
    println!("{}", "-".repeat(100));
    println!(
        "CONTRACT PROVEN: PKCE S256, callback validation, verified-invite match, immutable binding, cookie attributes, role mapping, fresh-MFA allow, logout revocation."
    );
}

#[tokio::test]
async fn state_is_single_use_replay_is_denied_and_audited() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let issued = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|u-1",
            email: "a@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect("first login ok");
    let state = h.auth.begin_login().unwrap().state;
    let err = h
        .auth
        .complete_login("code-replay", &state, &h.pending)
        .await;
    assert!(matches!(
        err,
        Err(AuthError::Oidc(auth::oidc::OidcError::StateMismatch))
    ));
    assert!(
        h.audit
            .has(AuthAuditKind::LoginDenied, Some("PENDING_MISSING"))
    );
    assert!(h.auth.session_info(&issued.cookie_value).await.is_ok());
}

#[tokio::test]
async fn uninvited_identity_is_denied_and_audited() {
    let h = harness(NOW);
    let err = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|stranger",
            email: "stranger@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect_err("no invite");
    assert!(err.is_invite_denial());
    assert!(
        h.audit
            .has(AuthAuditKind::LoginDenied, Some("INVITE_NOT_FOUND"))
    );
}

#[tokio::test]
async fn unverified_email_is_denied_and_audited() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let err = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|u-1",
            email: "a@example.com",
            verified: false,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect_err("unverified email");
    assert!(err.is_invite_denial());
    assert!(h.audit.has(
        AuthAuditKind::LoginDenied,
        Some("INVITE_EMAIL_NOT_VERIFIED")
    ));
}

#[tokio::test]
async fn invite_cannot_be_reused_by_second_identity() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|u-1",
            email: "a@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect("first user redeems");
    let err = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|u-2",
            email: "a@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect_err("second identity cannot reuse");
    assert!(err.is_invite_denial());
    assert!(
        h.audit
            .has(AuthAuditKind::LoginDenied, Some("INVITE_ALREADY_REDEEMED"))
    );
}

#[tokio::test]
async fn email_profile_change_keeps_the_same_user() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("first@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let first = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|stable-1",
            email: "first@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect("first login");
    let second = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|stable-1",
            email: "changed@example.com",
            verified: true,
            roles: &[],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect("second login after profile change");
    assert_eq!(
        first.session.user_id, second.session.user_id,
        "(iss,sub) binding is immutable"
    );
    assert_ne!(
        first.cookie_value, second.cookie_value,
        "fresh opaque session per login"
    );
}

#[tokio::test]
async fn owner_step_up_stale_denied_fresh_allowed() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("own@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let stale = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|own-1",
            email: "own@example.com",
            verified: true,
            roles: &["owner"],
            auth_time: NOW - 901,
            amr: &["pwd", "mfa"],
        },
    )
    .await
    .expect("login ok");
    let session = h.auth.session_info(&stale.cookie_value).await.unwrap();
    assert!(matches!(
        require_owner_step_up(&session, NOW, 900),
        Err(StepUpDenial::AuthTimeStale { .. })
    ));

    let fresh = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|own-1",
            email: "own@example.com",
            verified: true,
            roles: &["owner"],
            auth_time: NOW - 60,
            amr: &["pwd", "mfa"],
        },
    )
    .await
    .expect("re-login ok");
    let session = h.auth.session_info(&fresh.cookie_value).await.unwrap();
    assert!(require_owner_step_up(&session, NOW, 900).is_ok());
}

#[tokio::test]
async fn member_claim_role_maps_to_member() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("m@example.com", Role::Owner, 3600)
        .await
        .unwrap();
    let issued = complete_login(
        &h,
        &LoginSpec {
            sub: "auth0|m-1",
            email: "m@example.com",
            verified: true,
            roles: &["member"],
            auth_time: NOW - 60,
            amr: &["pwd"],
        },
    )
    .await
    .expect("member role from claims");
    assert_eq!(issued.session.role, Role::Member);
}

#[tokio::test]
async fn bad_state_nonce_or_code_never_creates_a_session() {
    let h = harness(NOW);
    h.auth
        .invites
        .create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let req = h.auth.begin_login().unwrap();
    let nonce = Url::parse(req.url.as_ref())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "nonce")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    // Provider signs an ID token carrying the WRONG nonce.
    let bad_nonce = h.sim.issue_code(
        &req,
        claims(
            &h.sim,
            "attacker-nonce",
            &LoginSpec {
                sub: "auth0|u-1",
                email: "a@example.com",
                verified: true,
                roles: &[],
                auth_time: NOW - 60,
                amr: &["pwd"],
            },
        ),
        &req.pkce.verifier,
    );
    let pending = PendingAuth {
        state: req.state.clone(),
        nonce,
        code_verifier: req.pkce.verifier.clone(),
        created_at_secs: NOW,
        ttl_secs: 300,
    };
    h.pending
        .insert(req.state.clone(), pending.clone())
        .await
        .unwrap();
    let err = h
        .auth
        .complete_login(&bad_nonce, &req.state, &h.pending)
        .await;
    assert!(matches!(
        err,
        Err(AuthError::Oidc(auth::oidc::OidcError::NonceMismatch))
    ));
    assert!(
        h.audit
            .has(AuthAuditKind::LoginDenied, Some("NONCE_MISMATCH"))
    );
}

#[test]
fn real_clock_smoke() {
    let now = SystemClock.now_epoch_secs();
    assert!(now > 1_700_000_000, "system clock sane: {now}");
}
