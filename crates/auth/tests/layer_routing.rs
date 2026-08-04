//! Layer-routing proof for the KRX entitlement gate: every Member-visible surface -
//! API (dataset/factor/paper/payload), scheduler (recommendation/backtest), report
//! (report/benchmark), artifact (download) - gates through the **same**
//! [`EntitlementService`], and every denial is auditable.

use lagrange_auth::entitlement::{
    AccessRequest, Actor, AuditDecision, AuditLog, CalendarDate, ContractRef, DataProvider,
    DatasetId, DocumentHash, Entitlement, EntitlementId, EntitlementService, EntitlementState,
    KrMemberSurface, KrUseRegistry, Layer, UserId, audit_event_for,
};

fn d(s: &str) -> CalendarDate {
    CalendarDate::parse(s).expect("valid date")
}

fn entitlement(id: &str, lifecycle: EntitlementState) -> Entitlement {
    Entitlement::builder()
        .id(EntitlementId::new(id))
        .provider(DataProvider::Krx)
        .contract(ContractRef::new(
            DocumentHash::sha256("ab".repeat(32)),
            format!("vault://krx-entitlements/{id}.pdf"),
        ))
        .lifecycle(lifecycle)
        .effective(d("2026-01-01"), d("2026-12-31"))
        .covered_datasets([DatasetId::krx_eod_bars()])
        .covered_uses(KrUseRegistry::standard().member_visible().to_vec())
        .covered_users(["usr_a", "usr_b", "usr_c", "usr_d", "usr_e"].map(UserId::new))
        .build()
}

fn member_request() -> AccessRequest {
    AccessRequest {
        actor: Actor::member("usr_a"),
        dataset: DatasetId::krx_eod_bars(),
        as_of: d("2026-06-15"),
    }
}

#[test]
fn active_allows_every_surface_in_every_layer() {
    let svc = EntitlementService::new(vec![entitlement("ent_active", EntitlementState::Active)]);
    let req = member_request();
    for surface in KrMemberSurface::ALL {
        let grant = svc
            .surface(surface, &req)
            .unwrap_or_else(|e| panic!("{surface:?} must be allowed: {e}"));
        assert_eq!(grant.entitlement_id, EntitlementId::new("ent_active"));
    }
}

#[test]
fn expired_denies_every_surface_with_typed_code_and_audits_each() {
    let svc = EntitlementService::new(vec![entitlement("ent_expired", EntitlementState::Expired)]);
    let req = member_request();
    let mut audit = AuditLog::new();
    for surface in KrMemberSurface::ALL {
        let outcome = svc.surface(surface, &req);
        let denied = outcome
            .as_ref()
            .expect_err("expired must deny every surface");
        assert_eq!(
            denied.code.as_str(),
            "DATA_ENTITLEMENT_REQUIRED",
            "{surface:?}"
        );
        assert_eq!(denied.state, Some(EntitlementState::Expired), "{surface:?}");
        audit.record(audit_event_for(&req, surface.use_kind(), &outcome));
    }
    assert_eq!(audit.events().len(), KrMemberSurface::ALL.len());
    assert!(audit.events().iter().all(|e| {
        matches!(
            e.decision,
            AuditDecision::Denied(lagrange_auth::entitlement::DenialCode::DataEntitlementRequired)
        )
    }));
}

#[test]
fn registry_covers_all_four_consuming_layers() {
    let mut layers = std::collections::BTreeSet::new();
    for surface in KrMemberSurface::ALL {
        layers.insert(surface.layer());
    }
    for layer in [Layer::Api, Layer::Scheduler, Layer::Report, Layer::Artifact] {
        assert!(
            layers.contains(&layer),
            "no surface mapped to layer {layer:?}"
        );
    }
}
