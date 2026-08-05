//! Todo 22 RED suite: Owner step-up gate (FR-AUTH-004) over real sessions.
//!
//! Stale `auth_time` or missing `amr` MFA denies; fresh MFA allows. Member
//! sessions always deny. Sessions are short, so step-up means re-login at the
//! provider - never a browser refresh token.

use auth::clock::FakeClock;
use auth::entitlement::Role;
use auth::invites::RedeemedIdentity;
use auth::sessions::{InMemorySessionStore, SessionError, SessionInfo, SessionService};
use auth::stepup::{StepUpDenial, require_owner_step_up};
use std::sync::Arc;

const NOW: i64 = 1_800_000_000;

fn identity(user: &str, role: Role) -> RedeemedIdentity {
    RedeemedIdentity {
        user_id: auth::entitlement::UserId::new(user),
        role,
        email: "u@example.com".to_string(),
        binding: "iss|sub".to_string(),
    }
}

async fn issued_session(user: &str, role: Role, auth_time: i64, amr: &[&str]) -> SessionInfo {
    let store = Arc::new(InMemorySessionStore::default());
    let audit = Arc::new(auth::audit::InMemoryAuthAudit::default());
    let svc = SessionService::new(store, Arc::new(FakeClock(NOW)), audit);
    let issued = svc
        .issue(
            &identity(user, role),
            auth_time,
            amr.iter().map(|s| s.to_string()).collect(),
        )
        .await
        .unwrap();
    svc.validate(&issued.cookie_value).await.unwrap()
}

#[tokio::test]
async fn fresh_mfa_owner_allowed() {
    let session = issued_session("own_1", Role::Owner, NOW - 60, &["pwd", "mfa"]).await;
    assert!(require_owner_step_up(&session, NOW, 900).is_ok());
}

#[tokio::test]
async fn stale_auth_time_denied() {
    let session = issued_session("own_1", Role::Owner, NOW - 901, &["pwd", "mfa"]).await;
    assert!(matches!(
        require_owner_step_up(&session, NOW, 900),
        Err(StepUpDenial::AuthTimeStale { .. })
    ));
}

#[tokio::test]
async fn missing_mfa_denied() {
    let session = issued_session("own_1", Role::Owner, NOW - 30, &["pwd"]).await;
    assert_eq!(
        require_owner_step_up(&session, NOW, 900),
        Err(StepUpDenial::MfaMissing)
    );
}

#[tokio::test]
async fn member_denied_even_with_fresh_mfa() {
    let session = issued_session("mem_1", Role::Member, NOW - 30, &["pwd", "mfa"]).await;
    assert_eq!(
        require_owner_step_up(&session, NOW, 900),
        Err(StepUpDenial::NotOwner)
    );
}

#[tokio::test]
async fn expired_session_cannot_reach_step_up_at_all() {
    let store = Arc::new(InMemorySessionStore::default());
    let audit = Arc::new(auth::audit::InMemoryAuthAudit::default());
    let early = SessionService::new(store.clone(), Arc::new(FakeClock(NOW)), audit.clone());
    let issued = early
        .issue(
            &identity("own_1", Role::Owner),
            NOW,
            vec!["mfa".to_string()],
        )
        .await
        .unwrap();
    let late = SessionService::new(store, Arc::new(FakeClock(NOW + 1801)), audit);
    let err = late
        .validate(&issued.cookie_value)
        .await
        .expect_err("expired");
    assert!(
        matches!(err, SessionError::Expired),
        "short session forces re-login, not step-up"
    );
}
