//! Todo 22 RED suite: invite redemption and immutable (iss, sub) binding.
//!
//! Single-use normalized-email invites requiring `email_verified`; identity is
//! keyed by (issuer, subject) so an email profile change keeps the same user.

use auth::audit::{AuthAuditKind, InMemoryAuthAudit};
use auth::clock::FakeClock;
use auth::entitlement::{Role, UserId};
use auth::invites::{
    InMemoryInviteStore, InMemoryUserStore, InviteError, InviteService, InviteStore, UserStore,
};
use auth::oidc::claims::IdTokenClaims;
use std::sync::Arc;

const NOW: i64 = 1_800_000_000;

fn claims(
    iss: &str,
    sub: &str,
    email: &str,
    email_verified: bool,
    roles: Vec<&str>,
) -> IdTokenClaims {
    IdTokenClaims {
        iss: iss.to_string(),
        sub: sub.to_string(),
        aud: vec!["https://api.lagrange.local".to_string()],
        exp: NOW + 3600,
        iat: Some(NOW),
        nonce: Some("nonce-1".to_string()),
        email: Some(email.to_string()),
        email_verified: Some(email_verified),
        auth_time: Some(NOW - 60),
        amr: vec!["pwd".to_string(), "mfa".to_string()],
        roles: roles.iter().map(|r| r.to_string()).collect(),
    }
}

fn service() -> (
    InviteService,
    Arc<InMemoryInviteStore>,
    Arc<InMemoryUserStore>,
    Arc<InMemoryAuthAudit>,
    FakeClock,
) {
    let invites = Arc::new(InMemoryInviteStore::default());
    let users = Arc::new(InMemoryUserStore::default());
    let audit = Arc::new(InMemoryAuthAudit::default());
    let clock = FakeClock(NOW);
    let svc = InviteService::new(
        invites.clone(),
        users.clone(),
        Arc::new(clock),
        audit.clone(),
    );
    (svc, invites, users, audit, clock)
}

#[tokio::test]
async fn redemption_creates_user_and_consumes_invite() {
    let (svc, invites, users, audit, _) = service();
    svc.create_invite("user@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let identity = svc
        .resolve_identity(&claims("iss-1", "sub-1", "user@example.com", true, vec![]))
        .await
        .expect("redeemed");
    assert_eq!(
        identity.user_id.0,
        users
            .find_by_binding("iss-1", "sub-1")
            .await
            .unwrap()
            .unwrap()
            .user_id
            .0
    );
    let invite = invites
        .find_by_email("user@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        invite.redeemed_by,
        Some(("iss-1".to_string(), "sub-1".to_string()))
    );
    assert!(audit.has(AuthAuditKind::InviteRedeemed, None));
}

#[tokio::test]
async fn second_redemption_of_same_invite_is_denied() {
    let (svc, _, _, _, _) = service();
    svc.create_invite("user@example.com", Role::Member, 3600)
        .await
        .unwrap();
    svc.resolve_identity(&claims("iss-1", "sub-1", "user@example.com", true, vec![]))
        .await
        .unwrap();
    let err = svc
        .resolve_identity(&claims("iss-2", "sub-2", "user@example.com", true, vec![]))
        .await
        .expect_err("invite is single-use");
    assert!(matches!(err, InviteError::AlreadyRedeemed), "got {err:?}");
}

#[tokio::test]
async fn expired_invite_is_denied() {
    let (svc, invites, _, _, _) = service();
    let expired = auth::invites::InviteRecord {
        id: "inv-expired".to_string(),
        email: "user@example.com".to_string(),
        role: Role::Member,
        created_at_secs: NOW - 7200,
        expires_at_secs: NOW - 1,
        redeemed_by: None,
        redeemed_at_secs: None,
    };
    invites.insert(expired).await.unwrap();
    let err = svc
        .resolve_identity(&claims("iss-1", "sub-1", "user@example.com", true, vec![]))
        .await
        .expect_err("expired invite denied");
    assert!(matches!(err, InviteError::InviteExpired), "got {err:?}");
}

#[tokio::test]
async fn unverified_email_is_denied() {
    let (svc, _, _, _, _) = service();
    svc.create_invite("user@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let err = svc
        .resolve_identity(&claims("iss-1", "sub-1", "user@example.com", false, vec![]))
        .await
        .expect_err("unverified email denied");
    assert!(matches!(err, InviteError::EmailNotVerified), "got {err:?}");
}

#[tokio::test]
async fn mismatched_email_is_denied() {
    let (svc, _, _, _, _) = service();
    svc.create_invite("invited@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let err = svc
        .resolve_identity(&claims("iss-1", "sub-1", "other@example.com", true, vec![]))
        .await
        .expect_err("uninvited email denied");
    assert!(matches!(err, InviteError::InviteNotFound), "got {err:?}");
}

#[tokio::test]
async fn missing_email_claim_is_denied() {
    let (svc, _, _, _, _) = service();
    svc.create_invite("user@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let mut c = claims("iss-1", "sub-1", "user@example.com", true, vec![]);
    c.email = None;
    let err = svc.resolve_identity(&c).await.expect_err("email required");
    assert!(matches!(err, InviteError::EmailRequired), "got {err:?}");
}

#[tokio::test]
async fn email_is_normalized_before_matching() {
    let (svc, _, users, _, _) = service();
    svc.create_invite("  User@Example.COM ", Role::Member, 3600)
        .await
        .unwrap();
    let identity = svc
        .resolve_identity(&claims("iss-1", "sub-1", "user@example.com", true, vec![]))
        .await
        .expect("normalized match");
    assert_eq!(identity.email, "user@example.com");
    let user = users
        .find_by_binding("iss-1", "sub-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, "user@example.com");
}

#[tokio::test]
async fn binding_is_immutable_across_email_profile_change() {
    let (svc, _, users, _, _) = service();
    svc.create_invite("first@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let first = svc
        .resolve_identity(&claims("iss-1", "sub-1", "first@example.com", true, vec![]))
        .await
        .unwrap();
    let second = svc
        .resolve_identity(&claims(
            "iss-1",
            "sub-1",
            "changed@example.com",
            true,
            vec![],
        ))
        .await
        .expect("same (iss,sub) resolves to the same user");
    assert_eq!(
        first.user_id, second.user_id,
        "(iss,sub) must be the identity key"
    );
    assert_eq!(second.email, "changed@example.com");
    let user = users
        .find_by_binding("iss-1", "sub-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, "changed@example.com");
    assert_eq!(user.user_id, first.user_id);
}

#[tokio::test]
async fn established_binding_needs_no_invite() {
    let (svc, _, users, _, _) = service();
    svc.create_invite("a@example.com", Role::Member, 3600)
        .await
        .unwrap();
    svc.resolve_identity(&claims("iss-1", "sub-1", "a@example.com", true, vec![]))
        .await
        .unwrap();
    let user_before = users
        .find_by_binding("iss-1", "sub-1")
        .await
        .unwrap()
        .unwrap();
    let again = svc
        .resolve_identity(&claims("iss-1", "sub-1", "a@example.com", true, vec![]))
        .await
        .expect("repeat login without invite");
    assert_eq!(again.user_id, user_before.user_id);
}

#[tokio::test]
async fn role_mapping_from_claims_and_invite_fallback() {
    let (svc, _, users, _, _) = service();
    svc.create_invite("owner@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let owner = svc
        .resolve_identity(&claims(
            "iss-1",
            "sub-1",
            "owner@example.com",
            true,
            vec!["owner"],
        ))
        .await
        .unwrap();
    assert_eq!(owner.role, Role::Owner);

    svc.create_invite("member@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let member = svc
        .resolve_identity(&claims(
            "iss-1",
            "sub-2",
            "member@example.com",
            true,
            vec!["member"],
        ))
        .await
        .unwrap();
    assert_eq!(member.role, Role::Member);

    svc.create_invite("fallback@example.com", Role::Owner, 3600)
        .await
        .unwrap();
    let fallback = svc
        .resolve_identity(&claims(
            "iss-1",
            "sub-3",
            "fallback@example.com",
            true,
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(fallback.role, Role::Owner);

    assert_eq!(
        users
            .find_by_binding("iss-1", "sub-1")
            .await
            .unwrap()
            .unwrap()
            .role,
        Role::Owner
    );
}

#[tokio::test]
async fn unknown_role_claim_is_fail_closed() {
    let (svc, _, _, _, _) = service();
    svc.create_invite("x@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let err = svc
        .resolve_identity(&claims(
            "iss-1",
            "sub-1",
            "x@example.com",
            true,
            vec!["admin", "auditor"],
        ))
        .await
        .expect_err("unknown roles deny");
    assert!(matches!(err, InviteError::RoleUnknown), "got {err:?}");
}

#[tokio::test]
async fn invalid_email_address_is_rejected() {
    let (svc, _, _, _, _) = service();
    assert!(
        svc.create_invite("not-an-email", Role::Member, 3600)
            .await
            .is_err()
    );
    assert!(InviteService::normalize_email(" ok@example.com ").unwrap() == "ok@example.com");
    assert!(InviteService::normalize_email("bad").is_err());
}

#[tokio::test]
async fn user_id_is_never_the_email() {
    let (svc, _, users, _, _) = service();
    svc.create_invite("user@example.com", Role::Member, 3600)
        .await
        .unwrap();
    let identity = svc
        .resolve_identity(&claims("iss-1", "sub-1", "user@example.com", true, vec![]))
        .await
        .unwrap();
    assert_ne!(identity.user_id, UserId::new("user@example.com"));
    assert_ne!(identity.user_id.0, identity.email);
    assert!(
        users
            .find_by_binding("iss-1", "sub-1")
            .await
            .unwrap()
            .is_some()
    );
}
