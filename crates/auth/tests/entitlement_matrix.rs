//! Manual QA channel: `cargo test -p lagrange-auth entitlement_matrix -- --nocapture`
//!
//! Prints the (state x use x role -> allowed/denied) matrix for the KRX
//! entitlement gate and asserts the fail-closed contract:
//! - ACTIVE  + Member + listed use      -> allowed
//! - EXPIRED + Member + any use         -> denied with `DATA_ENTITLEMENT_REQUIRED`
//! - REVOKED + Member + any use         -> denied
//! - PENDING + Member + any use         -> denied
//! - any state + Owner-only dev path    -> allowed
//! - Member + dev path                  -> denied (`OWNER_ONLY_DEVELOPMENT_PATH`)

use lagrange_auth::entitlement::{
    AccessRequest, Actor, CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash,
    Entitlement, EntitlementId, EntitlementService, EntitlementState, KrUseRegistry, Role, UserId,
};

fn d(s: &str) -> CalendarDate {
    CalendarDate::parse(s).expect("valid date")
}

fn entitlement(id: &str, lifecycle: EntitlementState) -> Entitlement {
    Entitlement::builder()
        .id(EntitlementId::new(id))
        .provider(DataProvider::Krx)
        .contract(ContractRef::new(
            DocumentHash::sha256("00".repeat(32)),
            format!("vault://krx-entitlements/{id}.pdf"),
        ))
        .lifecycle(lifecycle)
        .effective(d("2026-01-01"), d("2026-12-31"))
        .covered_datasets([DatasetId::krx_eod_bars()])
        .covered_uses(KrUseRegistry::standard().member_visible().to_vec())
        .covered_users(["usr_a", "usr_b", "usr_c", "usr_d", "usr_e"].map(UserId::new))
        .build()
}

fn request(role: Role, user: &str, as_of: &str) -> AccessRequest {
    AccessRequest {
        actor: Actor::new(user, role),
        dataset: DatasetId::krx_eod_bars(),
        as_of: d(as_of),
    }
}

#[test]
fn entitlement_matrix() {
    let registry = KrUseRegistry::standard();
    let as_of = "2026-06-15";
    let member_req = request(Role::Member, "usr_a", as_of);
    let owner_req = request(Role::Owner, "own_1", as_of);

    println!();
    println!("KRX data-entitlement gate matrix (as_of = {as_of})");
    println!(
        "{:<9} {:<16} {:<7} {:<10}",
        "STATE", "USE", "ROLE", "DECISION"
    );

    let mut failures: Vec<String> = Vec::new();
    let expect = |failures: &mut Vec<String>, ok: bool, label: String| {
        if !ok {
            failures.push(label);
        }
    };

    // --- Member-visible surfaces across the four lifecycle states ---------------
    for (label, lifecycle) in [
        ("PENDING", EntitlementState::Pending),
        ("ACTIVE", EntitlementState::Active),
        ("EXPIRED", EntitlementState::Expired),
        ("REVOKED", EntitlementState::Revoked),
    ] {
        let svc = EntitlementService::new(vec![entitlement(&format!("ent_{label}"), lifecycle)]);
        for use_kind in registry.member_visible() {
            let member_outcome = svc.authorize_use(*use_kind, &member_req);
            let owner_outcome = svc.authorize_use(*use_kind, &owner_req);
            let member_row = match &member_outcome {
                Ok(_) => "allowed".to_owned(),
                Err(e) => format!("denied({})", e.code),
            };
            let owner_row = match &owner_outcome {
                Ok(_) => "allowed".to_owned(),
                Err(e) => format!("denied({})", e.code),
            };
            println!(
                "{label:<9} {:<16} {:<7} {member_row}",
                use_kind.as_str(),
                "member"
            );
            println!(
                "{label:<9} {:<16} {:<7} {owner_row}",
                use_kind.as_str(),
                "owner"
            );

            // Assert the fail-closed contract.
            let allowed = member_outcome.is_ok();
            match lifecycle {
                EntitlementState::Active => {
                    expect(
                        &mut failures,
                        allowed,
                        format!("ACTIVE member {use_kind:?} must be allowed"),
                    );
                    let denied_code = owner_outcome.as_ref().err().map(|e| e.code);
                    expect(
                        &mut failures,
                        denied_code.is_none(),
                        format!("ACTIVE owner {use_kind:?} must be allowed"),
                    );
                }
                EntitlementState::Pending
                | EntitlementState::Expired
                | EntitlementState::Revoked => {
                    expect(
                        &mut failures,
                        !allowed,
                        format!("{label} member {use_kind:?} must be denied"),
                    );
                    if let Err(e) = &member_outcome {
                        expect(
                            &mut failures,
                            e.code.as_str() == "DATA_ENTITLEMENT_REQUIRED",
                            format!(
                                "{label} member {use_kind:?} must deny with DATA_ENTITLEMENT_REQUIRED"
                            ),
                        );
                    }
                    let owner_denied = owner_outcome.as_ref().err().map(|e| e.code);
                    expect(
                        &mut failures,
                        owner_denied.is_some(),
                        format!("{label} owner {use_kind:?} member-visible surface must be denied"),
                    );
                }
            }
        }
    }

    // --- Owner-only development paths are state-independent ----------------------
    for lifecycle in [
        EntitlementState::Pending,
        EntitlementState::Active,
        EntitlementState::Expired,
        EntitlementState::Revoked,
    ] {
        let label = lifecycle.as_str();
        let svc = EntitlementService::new(vec![entitlement(&format!("ent_{label}"), lifecycle)]);
        for dev_use in registry.owner_development() {
            let owner_outcome = svc.authorize_owner_dev(*dev_use, &owner_req);
            let member_outcome = svc.authorize_owner_dev(*dev_use, &member_req);
            println!(
                "{label:<9} {:<16} {:<7} {}",
                dev_use.as_str(),
                "owner",
                if owner_outcome.is_ok() {
                    "allowed"
                } else {
                    "denied"
                }
            );
            println!(
                "{label:<9} {:<16} {:<7} {}",
                dev_use.as_str(),
                "member",
                if member_outcome.is_ok() {
                    "allowed"
                } else {
                    "denied"
                }
            );
            expect(
                &mut failures,
                owner_outcome.is_ok(),
                format!("{label} owner dev path {dev_use:?} must be allowed"),
            );
            let member_denied = member_outcome.as_ref().err().map(|e| e.code.as_str());
            expect(
                &mut failures,
                member_denied == Some("OWNER_ONLY_DEVELOPMENT_PATH"),
                format!(
                    "{label} member dev path {dev_use:?} must be denied with OWNER_ONLY_DEVELOPMENT_PATH"
                ),
            );
        }
    }

    assert!(
        failures.is_empty(),
        "entitlement matrix contract violations:\n  {}",
        failures.join("\n  ")
    );
}
