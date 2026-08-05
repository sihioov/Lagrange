//! Owner step-up gate (FR-AUTH-004): sensitive Owner actions require a recent
//! authentication **and** an MFA authentication method, judged from the OIDC
//! `auth_time`/`amr` claims captured on the session at login. Fail-closed:
//! any missing/stale signal denies. There are no browser refresh tokens - a
//! stale session simply means re-login at the provider.

use crate::entitlement::Role;
use crate::sessions::SessionInfo;

/// Default maximum age of the authentication event behind an Owner action.
pub const STEP_UP_MAX_AUTH_AGE_SECS: i64 = 900;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepUpDenial {
    NotOwner,
    MfaMissing,
    AuthTimeAbsent,
    AuthTimeStale { seconds_since_auth: i64 },
}

impl StepUpDenial {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotOwner => "STEP_UP_NOT_OWNER",
            Self::MfaMissing => "STEP_UP_MFA_REQUIRED",
            Self::AuthTimeAbsent => "STEP_UP_AUTH_TIME_ABSENT",
            Self::AuthTimeStale { .. } => "STEP_UP_AUTH_TIME_STALE",
        }
    }
}

impl std::fmt::Display for StepUpDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotOwner => write!(f, "step-up requires the Owner role"),
            Self::MfaMissing => write!(f, "step-up requires an MFA authentication method"),
            Self::AuthTimeAbsent => write!(f, "step-up requires an authentication timestamp"),
            Self::AuthTimeStale { seconds_since_auth } => {
                write!(
                    f,
                    "authentication is {seconds_since_auth}s old - re-authenticate"
                )
            }
        }
    }
}

pub fn require_owner_step_up(
    session: &SessionInfo,
    now_secs: i64,
    max_auth_age_secs: i64,
) -> Result<(), StepUpDenial> {
    if session.role != Role::Owner {
        return Err(StepUpDenial::NotOwner);
    }
    if !session.amr.iter().any(|a| a.eq_ignore_ascii_case("mfa")) {
        return Err(StepUpDenial::MfaMissing);
    }
    let seconds_since_auth = now_secs
        .checked_sub(session.auth_time_secs)
        .unwrap_or(i64::MAX);
    if seconds_since_auth > max_auth_age_secs {
        return Err(StepUpDenial::AuthTimeStale { seconds_since_auth });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::{Role, UserId};

    fn session(role: Role, auth_time_secs: i64, amr: &[&str]) -> SessionInfo {
        SessionInfo {
            user_id: UserId::new("usr_1"),
            role,
            auth_time_secs,
            amr: amr.iter().map(|s| s.to_string()).collect(),
            expires_at_secs: auth_time_secs + 1800,
            csrf_token_hash: String::new(),
        }
    }

    #[test]
    fn fresh_mfa_owner_allowed() {
        let now = 1_000_000;
        let s = session(Role::Owner, now - 60, &["pwd", "mfa"]);
        assert!(require_owner_step_up(&s, now, 900).is_ok());
    }

    #[test]
    fn member_denied_even_with_fresh_mfa() {
        let now = 1_000_000;
        let s = session(Role::Member, now - 60, &["pwd", "mfa"]);
        assert_eq!(
            require_owner_step_up(&s, now, 900),
            Err(StepUpDenial::NotOwner)
        );
    }

    #[test]
    fn missing_mfa_denied() {
        let now = 1_000_000;
        let s = session(Role::Owner, now - 60, &["pwd"]);
        assert_eq!(
            require_owner_step_up(&s, now, 900),
            Err(StepUpDenial::MfaMissing)
        );
    }

    #[test]
    fn stale_auth_time_denied() {
        let now = 1_000_000;
        let s = session(Role::Owner, now - 901, &["pwd", "mfa"]);
        assert!(matches!(
            require_owner_step_up(&s, now, 900),
            Err(StepUpDenial::AuthTimeStale { .. })
        ));
        let fresh = session(Role::Owner, now - 900, &["pwd", "mfa"]);
        assert!(require_owner_step_up(&fresh, now, 900).is_ok());
    }
}
