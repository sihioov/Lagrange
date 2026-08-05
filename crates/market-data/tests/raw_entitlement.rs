//! Todo 5 entitlement wiring for the raw zone (Todo 8 acceptance):
//! a batch whose governing entitlement is not ACTIVE is tagged **Owner-only**,
//! and any Member-facing read of that batch is denied with
//! `DATA_ENTITLEMENT_REQUIRED`. Owner-only development reads stay allowed.
//!
//! Manual QA channel: `cargo test -p market-data --test raw_entitlement -- --nocapture`

use std::fs;

use auth::entitlement::{
    AccessRequest, Actor, CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash,
    Entitlement, EntitlementId, EntitlementService, EntitlementState, KrUseRegistry, Role, UserId,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KRX, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::entitlement::{RawAccessError, RawVisibility, raw_visibility, read_batch_gated};
use market_data::storage::{BatchSpec, ManifestEntry, RawStore};

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
        actor: Actor::new("usr_a", Role::Member),
        dataset: DatasetId::krx_eod_bars(),
        as_of: d("2026-06-15"),
    }
}

fn owner_request() -> AccessRequest {
    AccessRequest {
        actor: Actor::new("own_1", Role::Owner),
        dataset: DatasetId::krx_eod_bars(),
        as_of: d("2026-06-15"),
    }
}

fn batch_in_store(tag: &str) -> (RawStore, ManifestEntry) {
    let root = std::env::temp_dir().join(format!("ls-task8-ent-{tag}-{}", BatchId::generate()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let store = RawStore::new(&root);
    let date = TradingDate::parse("2020-01-31").expect("date");
    let b = BatchId::generate();
    let env = RawEnvelope::new(
        b,
        ResponseKind::Bars,
        "bars.json",
        br#"{"bars":[]}"#.to_vec(),
        UtcTimestamp::parse_rfc3339("2026-08-05T08:00:00Z").expect("now"),
        RequestMetadata {
            endpoint: "krx.eod.bars.v1".to_owned(),
            query: vec![],
            headers: vec![],
            mode: FetchMode::Synthetic,
        },
    );
    let spec = BatchSpec {
        provider: PROVIDER_KRX,
        market: MARKET_KR,
        date: &date,
        batch_id: b,
        entitlement_reference: None,
        mode: FetchMode::Synthetic,
    };
    let entry = store.store_batch(&spec, &[env]).expect("stores");
    (store, entry)
}

#[test]
fn without_active_entitlement_batch_is_owner_only() {
    let service = EntitlementService::new(vec![]);
    assert_eq!(raw_visibility(&service, d("2026-06-15")), RawVisibility::OwnerOnly);
}

#[test]
fn every_non_active_state_tags_owner_only() {
    for state in [
        EntitlementState::Pending,
        EntitlementState::Expired,
        EntitlementState::Revoked,
    ] {
        let service = EntitlementService::new(vec![entitlement("ent_x", state)]);
        assert_eq!(
            raw_visibility(&service, d("2026-06-15")),
            RawVisibility::OwnerOnly,
            "{state:?} must tag Owner-only"
        );
    }
}

#[test]
fn active_entitlement_tags_member_readable() {
    let service = EntitlementService::new(vec![entitlement("ent_krx_2026_0001", EntitlementState::Active)]);
    assert_eq!(
        raw_visibility(&service, d("2026-06-15")),
        RawVisibility::MemberReadable
    );
}

#[test]
fn member_read_of_owner_only_batch_denied_with_data_entitlement_required() {
    let service = EntitlementService::new(vec![entitlement("ent_krx_2026_0001", EntitlementState::Expired)]);
    let (store, entry) = batch_in_store("member-deny");

    let err = read_batch_gated(&store, &entry, &service, &member_request())
        .expect_err("Member read of a non-ACTIVE batch must be denied");
    match &err {
        RawAccessError::DataEntitlementRequired { batch_id, detail } => {
            assert_eq!(*batch_id, entry.batch_id);
            assert!(
                detail.contains("DATA_ENTITLEMENT_REQUIRED"),
                "denial must carry the DATA_ENTITLEMENT_REQUIRED code, got {detail}"
            );
        }
        other => panic!("expected DataEntitlementRequired, got {other:?}"),
    }
}

#[test]
fn member_read_denied_with_no_entitlement_at_all() {
    let service = EntitlementService::new(vec![]);
    let (store, entry) = batch_in_store("member-none");
    let err = read_batch_gated(&store, &entry, &service, &member_request())
        .expect_err("no entitlement record must deny Members");
    assert!(matches!(err, RawAccessError::DataEntitlementRequired { .. }));
}

#[test]
fn owner_dev_read_allowed_in_any_state() {
    for state in [
        EntitlementState::Pending,
        EntitlementState::Expired,
        EntitlementState::Revoked,
    ] {
        let service = EntitlementService::new(vec![entitlement("ent_x", state)]);
        let (store, entry) = batch_in_store(&format!("owner-{state:?}"));
        let files = read_batch_gated(&store, &entry, &service, &owner_request())
            .expect("Owner-only development read must be allowed in any state");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name, "bars.json");
    }
}

#[test]
fn member_read_allowed_when_entitlement_active() {
    let service = EntitlementService::new(vec![entitlement("ent_krx_2026_0001", EntitlementState::Active)]);
    let (store, entry) = batch_in_store("member-ok");
    let files = read_batch_gated(&store, &entry, &service, &member_request())
        .expect("ACTIVE entitlement must allow a Member read");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].bytes, br#"{"bars":[]}"#);
}

#[test]
fn expiry_flips_visibility_fail_closed() {
    let mut ent = entitlement("ent_krx_2026_0001", EntitlementState::Active);
    ent.transition(EntitlementState::Expired, d("2026-06-15")).expect("expire");
    let service = EntitlementService::new(vec![ent]);
    let (store, entry) = batch_in_store("expiry");
    let err = read_batch_gated(&store, &entry, &service, &member_request())
        .expect_err("expired entitlement must deny Members");
    assert!(matches!(err, RawAccessError::DataEntitlementRequired { .. }));
}
