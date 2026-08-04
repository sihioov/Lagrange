//! The **shared authorization service** consumed by the API, scheduler, report, and
//! artifact layers. It answers *"is this use allowed for this user/dataset on this
//! date"* and **fails closed** whenever the governing entitlement is not `ACTIVE`.
//!
//! Owner-only development paths are authorized for the Owner independent of any
//! entitlement. Rights are never inferred from credentials or API keys - an explicit
//! entitlement record is the only source of permission.

use std::collections::BTreeMap;

use crate::entitlement::date::CalendarDate;
use crate::entitlement::entitlement::Entitlement;
use crate::entitlement::error::{DenialCode, DenialReason, EntitlementDenied};
use crate::entitlement::identity::{Actor, DatasetId, EntitlementId, UserId};
use crate::entitlement::use_registry::{KrMemberSurface, KrUse, KrUseRegistry};
use crate::entitlement::EntitlementState;

/// An access request: who (actor), what dataset, on which date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRequest {
    pub actor: Actor,
    pub dataset: DatasetId,
    pub as_of: CalendarDate,
}

/// A granted Member-visible authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub entitlement_id: EntitlementId,
    pub dataset: DatasetId,
    pub use_kind: KrUse,
    pub actor: UserId,
    pub granted_on: CalendarDate,
    pub effective_until: CalendarDate,
}

/// A granted Owner-only development authorization (no entitlement involved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerDevGrant {
    pub dataset: DatasetId,
    pub use_kind: KrUse,
    pub actor: UserId,
    pub granted_on: CalendarDate,
}

/// The shared authorization service.
///
/// Constructed from a snapshot of `data_entitlements` records; later layers may
/// back it with the PostgreSQL repository (Todo 3) without changing the decision
/// logic, which is a pure function of the request and the records.
#[derive(Debug, Clone)]
pub struct EntitlementService {
    registry: KrUseRegistry,
    entitlements: Vec<Entitlement>,
}

impl EntitlementService {
    pub fn new(entitlements: Vec<Entitlement>) -> Self {
        Self {
            registry: KrUseRegistry::standard(),
            entitlements,
        }
    }

    pub fn with_registry(registry: KrUseRegistry, entitlements: Vec<Entitlement>) -> Self {
        Self { registry, entitlements }
    }

    pub fn entitlements(&self) -> &[Entitlement] {
        &self.entitlements
    }

    pub fn registry(&self) -> &KrUseRegistry {
        &self.registry
    }

    /// Authorize a Member-visible (or any registered) KR-derived use. Fails closed.
    pub fn authorize_use(&self, use_kind: KrUse, req: &AccessRequest) -> Result<Grant, EntitlementDenied> {
        if !self.registry.contains(use_kind) {
            // Unknown/unsupported use: deny (fail closed) rather than guess.
            return Err(EntitlementDenied {
                code: DenialCode::DataEntitlementRequired,
                dataset: req.dataset.clone(),
                use_kind,
                state: None,
                reason: DenialReason::NoEntitlementRecord,
            });
        }
        if use_kind.is_owner_development() {
            // Dev uses must go through `authorize_owner_dev`; using `authorize_use`
            // for them is a caller error - deny (fail closed).
            return Err(EntitlementDenied {
                code: DenialCode::OwnerOnlyDevelopmentPath,
                dataset: req.dataset.clone(),
                use_kind,
                state: None,
                reason: DenialReason::OwnerOnlyDevelopmentPath,
            });
        }
        let _ = (use_kind, req);
        todo!("authorize_use: not implemented (red phase)")
    }

    /// Authorize an Owner-only development path. Allowed for the Owner in **any**
    /// entitlement state; denied for Members.
    pub fn authorize_owner_dev(&self, dev_use: KrUse, req: &AccessRequest) -> Result<OwnerDevGrant, EntitlementDenied> {
        let _ = (dev_use, req);
        todo!("authorize_owner_dev: not implemented (red phase)")
    }

    /// Convenience: gate a Member-visible surface through the same service.
    pub fn surface(&self, surface: KrMemberSurface, req: &AccessRequest) -> Result<Grant, EntitlementDenied> {
        self.authorize_use(surface.use_kind(), req)
    }

    /// Summary used by diagnostics: which entitlement would govern `dataset`, and
    /// in which state, on `as_of`.
    pub fn governing_state(&self, dataset: &DatasetId, as_of: CalendarDate) -> Option<(EntitlementId, EntitlementState)> {
        let candidates = self.candidates_for(dataset);
        let best = candidates.into_iter().max_by_key(|e| self.relevance_score(e, dataset, as_of))?;
        Some((best.id.clone(), best.status_on(as_of)))
    }

    /// Entitlement records covering `dataset`.
    pub(crate) fn candidates_for(&self, dataset: &DatasetId) -> Vec<&Entitlement> {
        self.entitlements
            .iter()
            .filter(|e| e.covered_datasets.contains(dataset))
            .collect()
    }

    /// Deterministic ranking of a candidate for explaining a denial: prefer the
    /// entitlement that is active and covers the most request dimensions.
    pub(crate) fn relevance_score(&self, e: &Entitlement, dataset: &DatasetId, as_of: CalendarDate) -> (u8, u8, u8) {
        let _ = (e, dataset, as_of);
        (0, 0, 0)
    }

    /// Lookup map used by `governing_state` diagnostics.
    pub fn entitlement_map(&self) -> BTreeMap<EntitlementId, EntitlementState> {
        self.entitlements
            .iter()
            .map(|e| (e.id.clone(), e.lifecycle))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::entitlement::{ContractRef, DocumentHash};
    use crate::entitlement::identity::{DataProvider, Role};

    fn hex(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn entitlement(id: &str, lifecycle: EntitlementState, from: &str, until: &str) -> Entitlement {
        Entitlement::builder()
            .id(EntitlementId::new(id))
            .provider(DataProvider::Krx)
            .contract(ContractRef::new(
                DocumentHash::sha256(hex('0')),
                format!("vault://krx-entitlements/{id}.pdf"),
            ))
            .lifecycle(lifecycle)
            .effective(
                CalendarDate::parse(from).unwrap(),
                CalendarDate::parse(until).unwrap(),
            )
            .covered_datasets([DatasetId::krx_eod_bars()])
            .covered_uses(KrUseRegistry::standard().member_visible().to_vec())
            .covered_users(["usr_a", "usr_b", "usr_c", "usr_d", "usr_e"].map(UserId::new))
            .build()
    }

    fn req(actor: Actor, dataset: DatasetId, as_of: &str) -> AccessRequest {
        AccessRequest {
            actor,
            dataset,
            as_of: CalendarDate::parse(as_of).unwrap(),
        }
    }

    fn member_req(as_of: &str) -> AccessRequest {
        req(Actor::member("usr_a"), DatasetId::krx_eod_bars(), as_of)
    }

    // --- RED PHASE: fail-closed matrix (allow paths must fail) -------------------

    #[test]
    fn active_permits_only_listed_uses_for_member() {
        let svc = EntitlementService::new(vec![entitlement(
            "ent_active",
            EntitlementState::Active,
            "2026-01-01",
            "2026-12-31",
        )]);
        let as_of = member_req("2026-06-15");
        // Every listed (member-visible) use is permitted for a covered member.
        for use_kind in KrUseRegistry::standard().member_visible() {
            let grant = svc.authorize_use(*use_kind, &as_of).expect("listed use must be allowed");
            assert_eq!(grant.entitlement_id, EntitlementId::new("ent_active"));
        }
    }

    #[test]
    fn pending_expired_revoked_deny_every_member_use() {
        for (id, lifecycle) in [
            ("ent_pending", EntitlementState::Pending),
            ("ent_expired", EntitlementState::Expired),
            ("ent_revoked", EntitlementState::Revoked),
        ] {
            let svc = EntitlementService::new(vec![entitlement(id, lifecycle, "2026-01-01", "2026-12-31")]);
            let as_of = member_req("2026-06-15");
            for use_kind in KrUseRegistry::standard().member_visible() {
                let denied = svc.authorize_use(*use_kind, &as_of).expect_err("must deny");
                assert_eq!(denied.code, DenialCode::DataEntitlementRequired, "{id} {use_kind:?}");
            }
        }
    }

    #[test]
    fn no_entitlement_fails_closed() {
        let svc = EntitlementService::new(vec![]);
        let as_of = member_req("2026-06-15");
        let denied = svc.authorize_use(KrUse::Recommendation, &as_of).unwrap_err();
        assert_eq!(denied.code, DenialCode::DataEntitlementRequired);
        assert_eq!(denied.reason, DenialReason::NoEntitlementRecord);
        assert_eq!(denied.state, None);
    }

    #[test]
    fn owner_dev_paths_allowed_in_any_state() {
        for (id, lifecycle) in [
            ("ent_pending", EntitlementState::Pending),
            ("ent_active", EntitlementState::Active),
            ("ent_expired", EntitlementState::Expired),
            ("ent_revoked", EntitlementState::Revoked),
        ] {
            let svc = EntitlementService::new(vec![entitlement(id, lifecycle, "2026-01-01", "2026-12-31")]);
            let as_of = req(Actor::owner("own_1"), DatasetId::krx_eod_bars(), "2026-06-15");
            for dev_use in KrUseRegistry::standard().owner_development() {
                svc.authorize_owner_dev(*dev_use, &as_of).expect("owner dev must be allowed");
            }
        }
    }

    #[test]
    fn member_cannot_use_owner_dev_paths() {
        let svc = EntitlementService::new(vec![entitlement(
            "ent_active",
            EntitlementState::Active,
            "2026-01-01",
            "2026-12-31",
        )]);
        let as_of = member_req("2026-06-15");
        for dev_use in KrUseRegistry::standard().owner_development() {
            let denied = svc.authorize_owner_dev(*dev_use, &as_of).unwrap_err();
            assert_eq!(denied.code, DenialCode::OwnerOnlyDevelopmentPath);
        }
    }

    #[test]
    fn active_denies_uncovered_user_and_unlisted_use() {
        let mut partial = entitlement("ent_partial", EntitlementState::Active, "2026-01-01", "2026-12-31");
        partial.covered_uses = [KrUse::Recommendation].into_iter().collect();
        let svc = EntitlementService::new(vec![partial]);
        // Covered user, but use not listed in the contract -> denied.
        let denied = svc
            .authorize_use(KrUse::Backtest, &member_req("2026-06-15"))
            .unwrap_err();
        assert_eq!(denied.code, DenialCode::DataEntitlementRequired);
        assert_eq!(denied.reason, DenialReason::UseNotCovered);
        // Uncovered user (even for a listed use) -> denied.
        let other = req(Actor::member("usr_z"), DatasetId::krx_eod_bars(), "2026-06-15");
        let denied = svc.authorize_use(KrUse::Recommendation, &other).unwrap_err();
        assert_eq!(denied.code, DenialCode::DataEntitlementRequired);
        assert_eq!(denied.reason, DenialReason::UserNotCovered);
    }

    #[test]
    fn role_has_no_effect_on_member_visible_gate() {
        // Owner is gated on Member-visible surfaces too: EXPIRED denies the Owner.
        let svc = EntitlementService::new(vec![entitlement(
            "ent_expired",
            EntitlementState::Expired,
            "2026-01-01",
            "2026-12-31",
        )]);
        let as_of = req(Actor::owner("own_1"), DatasetId::krx_eod_bars(), "2026-06-15");
        let denied = svc.authorize_use(KrUse::Recommendation, &as_of).unwrap_err();
        assert_eq!(denied.code, DenialCode::DataEntitlementRequired);
        let _ = Role::Owner; // role enumerated for clarity
    }
}
