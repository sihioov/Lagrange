//! Todo 22 RED suite: opaque sessions - hashed storage, cookie attributes,
//! fixation resistance, expiry, logout/revocation, CSRF rotation.

use auth::audit::{AuthAuditKind, InMemoryAuthAudit};
use auth::clock::FakeClock;
use auth::csrf;
use auth::entitlement::Role;
use auth::invites::RedeemedIdentity;
use auth::sessions::cookie::{self, NAME};
use auth::sessions::{
    InMemorySessionStore, IssuedSession, SessionError, SessionInfo, SessionService, SessionStore,
};
use std::sync::Arc;

const NOW: i64 = 1_800_000_000;
const TTL: i64 = 1800;

fn identity(user: &str, role: Role) -> RedeemedIdentity {
    RedeemedIdentity {
        user_id: auth::entitlement::UserId::new(user),
        role,
        email: "u@example.com".to_string(),
        binding: "iss|sub".to_string(),
    }
}

fn service(now: i64) -> (SessionService, Arc<InMemorySessionStore>, Arc<InMemoryAuthAudit>, FakeClock) {
    let store = Arc::new(InMemorySessionStore::default());
    let audit = Arc::new(InMemoryAuthAudit::default());
    let clock = FakeClock(now);
    let svc = SessionService::new(store.clone(), Arc::new(clock), audit.clone());
    (svc, store, audit, clock)
}

#[tokio::test]
async fn store_contract_roundtrip() {
    let store = InMemorySessionStore::default();
    let session = auth::sessions::StoredSession {
        token_hash: "h".to_string(),
        user_id: auth::entitlement::UserId::new("u"),
        role: Role::Member,
        auth_time_secs: NOW,
        amr: vec![],
        csrf_token_hash: "c".to_string(),
        created_at_secs: NOW,
        expires_at_secs: NOW + TTL,
    };
    store.insert(session.clone()).await.unwrap();
    assert_eq!(store.lookup("h").await.unwrap(), Some(session.clone()));
    assert_eq!(store.lookup("missing").await.unwrap(), None);
    store.update_csrf("h", "c2").await.unwrap();
    assert_eq!(store.lookup("h").await.unwrap().unwrap().csrf_token_hash, "c2");
    store.revoke("h").await.unwrap();
    assert_eq!(store.lookup("h").await.unwrap(), None);
}

#[tokio::test]
async fn issued_session_is_opaque_short_and_carrying_a_csrf_token() {
    let (svc, _, _, _) = service(NOW);
    let issued: IssuedSession = svc.issue(&identity("usr_1", Role::Member), NOW - 60, vec!["pwd".to_string()]).await.unwrap();
    assert_eq!(issued.cookie_value.len(), 43, "32 random bytes base64url");
    assert!(issued.set_cookie_header.starts_with(&format!("{NAME}={};", issued.cookie_value)));
    assert!(issued.set_cookie_header.contains("Path=/"));
    assert!(issued.set_cookie_header.contains("Secure"));
    assert!(issued.set_cookie_header.contains("HttpOnly"));
    assert!(issued.set_cookie_header.contains("SameSite=Lax"));
    assert!(!issued.set_cookie_header.contains("Domain="), "host-only");
    assert!(issued.set_cookie_header.contains("Max-Age="));
    assert_eq!(issued.csrf_token.len(), 64);
    assert_eq!(issued.session.expires_at_secs, NOW + TTL, "short session");
}

#[tokio::test]
async fn store_holds_only_the_hash_never_the_raw_value() {
    let (svc, store, _, _) = service(NOW);
    let issued = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    let stored = store.lookup(&cookie::hash(&issued.cookie_value)).await.unwrap().expect("lookup by hash");
    assert_eq!(stored.token_hash.len(), 64, "sha256 hex key");
    let snapshot = store.lookup(&issued.cookie_value).await.unwrap();
    assert!(snapshot.is_none(), "raw opaque value must never be a store key");
}

#[tokio::test]
async fn unknown_cookie_is_denied() {
    let (svc, _, _, _) = service(NOW);
    let err = svc.validate("not-a-real-cookie").await.expect_err("unknown");
    assert!(matches!(err, SessionError::UnknownSession));
}

#[tokio::test]
async fn session_fixation_is_impossible_each_login_mints_a_new_value() {
    let (svc, _, _, _) = service(NOW);
    let attacker_known = "attacker-set-value";
    let a = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    let b = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    assert_ne!(a.cookie_value, b.cookie_value);
    assert_ne!(a.cookie_value, attacker_known);
    assert_ne!(b.cookie_value, attacker_known);
    assert!(svc.validate(attacker_known).await.is_err(), "attacker-known value never authenticates");
    assert!(svc.validate(&a.cookie_value).await.is_ok());
    assert!(svc.validate(&b.cookie_value).await.is_ok());
}

#[tokio::test]
async fn expired_session_is_denied_and_requires_relogin() {
    let (svc, _, audit, mut clock) = service(NOW);
    let issued = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    assert!(svc.validate(&issued.cookie_value).await.is_ok());
    clock.advance(TTL);
    let err = svc.validate(&issued.cookie_value).await.expect_err("expired");
    assert!(matches!(err, SessionError::Expired));
    assert!(audit.has(AuthAuditKind::SessionExpired, "SESSION_EXPIRED"));
    assert!(svc.validate(&issued.cookie_value).await.is_err(), "no sliding renewal");
}

#[tokio::test]
async fn logout_revokes_the_session() {
    let (svc, _, audit, _) = service(NOW);
    let issued = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    svc.revoke(&issued.cookie_value).await.unwrap();
    let err = svc.validate(&issued.cookie_value).await.expect_err("revoked");
    assert!(matches!(err, SessionError::UnknownSession));
    assert!(audit.has(AuthAuditKind::SessionRevoked, None));
    let clear = cookie::clear_cookie();
    assert!(clear.contains("Max-Age=0"), "browser cookie cleared: {clear}");
}

#[tokio::test]
async fn csrf_token_rotates_and_old_is_denied() {
    let (svc, _, _, _) = service(NOW);
    let issued = svc.issue(&identity("usr_1", Role::Member), NOW, vec![]).await.unwrap();
    let session: SessionInfo = svc.validate(&issued.cookie_value).await.unwrap();
    assert!(csrf::verify(&session.csrf_token_hash, &issued.csrf_token));
    let rotated = svc.rotate_csrf(&issued.cookie_value).await.unwrap();
    assert_ne!(rotated, issued.csrf_token);
    let after: SessionInfo = svc.validate(&issued.cookie_value).await.unwrap();
    assert!(csrf::verify(&after.csrf_token_hash, &rotated));
    assert!(!csrf::verify(&after.csrf_token_hash, &issued.csrf_token), "rotated token invalidates old");
}

#[tokio::test]
async fn session_carries_auth_time_amr_role_and_actor() {
    let (svc, _, _, _) = service(NOW);
    let issued = svc
        .issue(&identity("own_1", Role::Owner), NOW - 120, vec!["pwd".to_string(), "mfa".to_string()])
        .await
        .unwrap();
    let session = svc.validate(&issued.cookie_value).await.unwrap();
    assert_eq!(session.role, Role::Owner);
    assert_eq!(session.auth_time_secs, NOW - 120);
    assert_eq!(session.amr, vec!["pwd".to_string(), "mfa".to_string()]);
    let actor = session.actor();
    assert!(actor.is_owner());
    assert_eq!(actor.user_id.0, "own_1");
}

#[tokio::test]
async fn rotate_csrf_on_unknown_session_is_denied() {
    let (svc, _, _, _) = service(NOW);
    let err = svc.rotate_csrf("no-such-cookie").await.expect_err("unknown");
    assert!(matches!(err, SessionError::UnknownSession));
}
