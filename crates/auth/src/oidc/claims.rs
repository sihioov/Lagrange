//! ID-token claims (OIDC core claims + the Lagrange role claim).
//!
//! `aud` may arrive as a JSON string or an array (Auth0 emits a string for a
//! single audience); `amr`/`roles` are optional arrays. The role claim is a
//! top-level `roles` array; the Auth0 tenant maps `app_metadata.roles` onto it
//! via a custom claim action (documented in docs/decisions/0002-...md).

use serde::{Deserialize, Serialize};

fn deserialize_aud<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Aud {
        Single(String),
        Many(Vec<String>),
    }
    match Aud::deserialize(de)? {
        Aud::Single(s) => Ok(vec![s]),
        Aud::Many(v) => Ok(v),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    #[serde(default, deserialize_with = "deserialize_aud")]
    pub aud: Vec<String>,
    pub exp: i64,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub auth_time: Option<i64>,
    #[serde(default)]
    pub amr: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

impl IdTokenClaims {
    /// First role mapping, owner dominating member; `None` when silent.
    pub fn mapped_role(&self) -> Option<entitlement::Role> {
        use entitlement::Role;
        if self.roles.iter().any(|r| r.eq_ignore_ascii_case("owner")) {
            Some(Role::Owner)
        } else if self.roles.iter().any(|r| r.eq_ignore_ascii_case("member")) {
            Some(Role::Member)
        } else {
            None
        }
    }

    pub fn is_email_verified(&self) -> bool {
        self.email_verified == Some(true)
    }

    pub fn amr_has_mfa(&self) -> bool {
        self.amr.iter().any(|a| a.eq_ignore_ascii_case("mfa"))
    }
}

use crate::entitlement;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aud_accepts_string_and_array() {
        let claims: IdTokenClaims =
            serde_json::from_str(r#"{"iss":"i","sub":"s","aud":"api","exp":1}"#).unwrap();
        assert_eq!(claims.aud, vec!["api".to_string()]);
        let claims: IdTokenClaims =
            serde_json::from_str(r#"{"iss":"i","sub":"s","aud":["a","b"],"exp":1}"#).unwrap();
        assert_eq!(claims.aud, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn role_mapping_owner_dominates_and_unknown_is_ignored() {
        let mut claims = IdTokenClaims {
            iss: "i".into(),
            sub: "s".into(),
            aud: vec![],
            exp: 1,
            iat: None,
            nonce: None,
            email: None,
            email_verified: None,
            auth_time: None,
            amr: vec![],
            roles: vec!["member".into()],
        };
        assert_eq!(claims.mapped_role(), Some(entitlement::Role::Member));
        claims.roles = vec!["member".into(), "owner".into()];
        assert_eq!(claims.mapped_role(), Some(entitlement::Role::Owner));
        claims.roles = vec!["admin".into(), "auditor".into()];
        assert_eq!(claims.mapped_role(), None);
        claims.roles = vec![];
        assert_eq!(claims.mapped_role(), None);
    }

    #[test]
    fn email_verification_and_mfa_flags() {
        let claims = IdTokenClaims {
            iss: "i".into(),
            sub: "s".into(),
            aud: vec![],
            exp: 1,
            iat: None,
            nonce: None,
            email: Some("a@b.c".into()),
            email_verified: Some(true),
            auth_time: Some(5),
            amr: vec!["pwd".into(), "mfa".into()],
            roles: vec![],
        };
        assert!(claims.is_email_verified());
        assert!(claims.amr_has_mfa());
    }
}
