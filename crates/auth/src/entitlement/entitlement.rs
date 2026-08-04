//! The redacted `Entitlement` record and contract-document reference.
//!
//! **Redaction contract**: an `Entitlement` stores a `ContractRef` that holds only a
//! document hash and a storage reference - **never contract contents**. Rights are
//! represented by explicit coverage sets (datasets, uses, users) and an effective
//! window; nothing is inferred from API keys or credentials.

use std::collections::BTreeSet;

use crate::entitlement::date::CalendarDate;
use crate::entitlement::error::TransitionError;
use crate::entitlement::identity::{DataProvider, DatasetId, EntitlementId, UserId};
use crate::entitlement::state::{is_allowed_transition, EntitlementState};
use crate::entitlement::use_registry::KrUse;

/// Hash of the signed contract document. The document itself stays outside Git
/// (and outside this crate); only the hash and a reference are recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentHash {
    pub algorithm: &'static str,
    pub hex: String,
}

impl DocumentHash {
    /// A SHA-256 digest. `hex` must be exactly 64 lowercase hex characters.
    pub fn sha256(hex: impl Into<String>) -> Self {
        let hex = hex.into();
        assert!(
            hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "sha256 hex must be exactly 64 lowercase hex characters"
        );
        Self {
            algorithm: "SHA-256",
            hex,
        }
    }
}

/// Reference to the signed contract document - a storage/vault key or URL, never
/// the document contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRef {
    pub document_hash: DocumentHash,
    pub document_reference: String,
}

impl ContractRef {
    pub fn new(hash: DocumentHash, document_reference: impl Into<String>) -> Self {
        Self {
            document_hash: hash,
            document_reference: document_reference.into(),
        }
    }
}

/// A `data_entitlements` record (in-memory mirror of the Todo 3 table row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    pub id: EntitlementId,
    pub provider: DataProvider,
    pub contract: ContractRef,
    pub lifecycle: EntitlementState,
    pub effective_from: CalendarDate,
    pub effective_until: CalendarDate,
    pub covered_datasets: BTreeSet<DatasetId>,
    pub covered_uses: BTreeSet<KrUse>,
    pub covered_users: BTreeSet<UserId>,
}

impl Entitlement {
    /// Start building an entitlement.
    pub fn builder() -> EntitlementBuilder {
        EntitlementBuilder::default()
    }

    /// Effective lifecycle status on `date`, computed fail-closed:
    /// - `REVOKED`/`EXPIRED` are terminal and never become `ACTIVE`;
    /// - a date before `effective_from` is always `PENDING`;
    /// - a date after `effective_until` is always `EXPIRED`;
    /// - otherwise the persisted lifecycle (only `ACTIVE` grants).
    pub fn status_on(&self, _date: CalendarDate) -> EntitlementState {
        todo!("status_on: not implemented (red phase)")
    }

    /// Typed lifecycle transition, applied on `date`:
    /// `Pending -> Active | Expired | Revoked`, `Active -> Expired | Revoked`.
    /// `Pending -> Active` requires `on` inside the effective window.
    pub fn transition(&mut self, to: EntitlementState, on: CalendarDate) -> Result<(), TransitionError> {
        if !is_allowed_transition(self.lifecycle, to) {
            return Err(TransitionError::InvalidTransition {
                from: self.lifecycle,
                to,
            });
        }
        if to == EntitlementState::Active && !(self.effective_from <= on && on <= self.effective_until) {
            return Err(TransitionError::OutsideEffectiveWindow {
                on,
                effective_from: self.effective_from,
                effective_until: self.effective_until,
            });
        }
        self.lifecycle = to;
        Ok(())
    }

    /// Set-level containment check (dataset, use, and user must all be covered).
    pub fn covers(&self, dataset: &DatasetId, user: &UserId, use_kind: KrUse) -> bool {
        self.covered_datasets.contains(dataset)
            && self.covered_uses.contains(&use_kind)
            && self.covered_users.contains(user)
    }
}

/// Builder for [`Entitlement`]; convenient for tests and the admin tooling that
/// will ingest redacted entitlement metadata (Todo 3/27).
#[derive(Debug, Clone, Default)]
pub struct EntitlementBuilder {
    id: Option<EntitlementId>,
    provider: Option<DataProvider>,
    contract: Option<ContractRef>,
    lifecycle: EntitlementState,
    effective_from: Option<CalendarDate>,
    effective_until: Option<CalendarDate>,
    covered_datasets: BTreeSet<DatasetId>,
    covered_uses: BTreeSet<KrUse>,
    covered_users: BTreeSet<UserId>,
}

impl EntitlementBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: EntitlementId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn provider(mut self, provider: DataProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn contract(mut self, contract: ContractRef) -> Self {
        self.contract = Some(contract);
        self
    }

    pub fn lifecycle(mut self, lifecycle: EntitlementState) -> Self {
        self.lifecycle = lifecycle;
        self
    }

    pub fn effective(mut self, from: CalendarDate, until: CalendarDate) -> Self {
        assert!(
            from <= until,
            "effective_from must not be after effective_until ({from} > {until})"
        );
        self.effective_from = Some(from);
        self.effective_until = Some(until);
        self
    }

    pub fn covered_datasets(mut self, datasets: impl IntoIterator<Item = DatasetId>) -> Self {
        self.covered_datasets.extend(datasets);
        self
    }

    pub fn covered_uses(mut self, uses: impl IntoIterator<Item = KrUse>) -> Self {
        self.covered_uses.extend(uses);
        self
    }

    pub fn covered_users(mut self, users: impl IntoIterator<Item = UserId>) -> Self {
        self.covered_users.extend(users);
        self
    }

    /// Panics with a clear message when required fields are missing - this builder
    /// is for tests and trusted admin ingestion of already-validated metadata.
    pub fn build(self) -> Entitlement {
        Entitlement {
            id: self.id.expect("builder: id is required"),
            provider: self.provider.expect("builder: provider is required"),
            contract: self.contract.expect("builder: contract is required"),
            lifecycle: self.lifecycle,
            effective_from: self.effective_from.expect("builder: effective_from is required"),
            effective_until: self.effective_until.expect("builder: effective_until is required"),
            covered_datasets: self.covered_datasets,
            covered_uses: self.covered_uses,
            covered_users: self.covered_users,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::identity::{Actor, Role};

    fn hex_of(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn sample() -> Entitlement {
        Entitlement::builder()
            .id(EntitlementId::new("ent_krx_2026_0001"))
            .provider(DataProvider::Krx)
            .contract(ContractRef::new(
                DocumentHash::sha256(hex_of('a')),
                "vault://krx-entitlements/ent_krx_2026_0001.pdf",
            ))
            .lifecycle(EntitlementState::Active)
            .effective(
                CalendarDate::parse("2026-01-01").unwrap(),
                CalendarDate::parse("2026-12-31").unwrap(),
            )
            .covered_datasets([DatasetId::krx_eod_bars()])
            .covered_uses([KrUse::Recommendation, KrUse::Backtest, KrUse::Report])
            .covered_users([UserId::new("usr_a")])
            .build()
    }

    #[test]
    fn contract_ref_is_redacted() {
        let e = sample();
        // Only a hash + storage reference - no contents field exists on the type.
        assert_eq!(e.contract.document_hash.algorithm, "SHA-256");
        assert_eq!(e.contract.document_hash.hex.len(), 64);
        assert!(e.contract.document_reference.starts_with("vault://"));
    }

    #[test]
    fn sha256_validates_hex() {
        assert!(DocumentHash::sha256(hex_of('b')).hex.len() == 64);
    }

    #[test]
    #[should_panic(expected = "sha256 hex")]
    fn sha256_rejects_bad_hex() {
        let _ = DocumentHash::sha256("not-a-hash");
    }

    #[test]
    fn covers_is_set_containment() {
        let e = sample();
        assert!(e.covers(&DatasetId::krx_eod_bars(), &UserId::new("usr_a"), KrUse::Report));
        assert!(!e.covers(&DatasetId::krx_eod_bars(), &UserId::new("usr_b"), KrUse::Report)); // user not covered
        assert!(!e.covers(&DatasetId::krx_eod_bars(), &UserId::new("usr_a"), KrUse::Download)); // use not covered
        assert!(!e.covers(&DatasetId::new("krx_calendar"), &UserId::new("usr_a"), KrUse::Report)); // dataset not covered
    }

    // --- RED PHASE: lifecycle semantics (fail closed) ----------------------------

    #[test]
    fn status_on_fail_closed_before_and_after_window() {
        let mut e = sample();
        // Active lifecycle: inside window -> ACTIVE
        assert_eq!(e.status_on(CalendarDate::parse("2026-06-15").unwrap()), EntitlementState::Active);
        // Fail closed: before effective_from -> PENDING
        assert_eq!(e.status_on(CalendarDate::parse("2025-12-31").unwrap()), EntitlementState::Pending);
        // Fail closed: after effective_until -> EXPIRED
        assert_eq!(e.status_on(CalendarDate::parse("2027-01-01").unwrap()), EntitlementState::Expired);

        // Pending lifecycle inside window stays PENDING (awaiting activation).
        e.lifecycle = EntitlementState::Pending;
        assert_eq!(e.status_on(CalendarDate::parse("2026-06-15").unwrap()), EntitlementState::Pending);

        // Terminal states never revive.
        e.lifecycle = EntitlementState::Revoked;
        assert_eq!(e.status_on(CalendarDate::parse("2026-06-15").unwrap()), EntitlementState::Revoked);
        e.lifecycle = EntitlementState::Expired;
        assert_eq!(e.status_on(CalendarDate::parse("2026-06-15").unwrap()), EntitlementState::Expired);
    }

    #[test]
    fn typed_transitions_activate_revoke_expire() {
        // Pending -> Active inside window.
        let mut e = sample();
        e.lifecycle = EntitlementState::Pending;
        e.transition(EntitlementState::Active, CalendarDate::parse("2026-03-01").unwrap())
            .unwrap();
        assert_eq!(e.lifecycle, EntitlementState::Active);

        // Active -> Revoked.
        e.transition(EntitlementState::Revoked, CalendarDate::parse("2026-03-02").unwrap()).unwrap();
        assert_eq!(e.lifecycle, EntitlementState::Revoked);

        // Pending -> Expired (window lapsed without activation).
        let mut e2 = sample();
        e2.lifecycle = EntitlementState::Pending;
        e2.transition(EntitlementState::Expired, CalendarDate::parse("2027-01-02").unwrap()).unwrap();
        assert_eq!(e2.lifecycle, EntitlementState::Expired);
    }

    #[test]
    fn activation_outside_window_rejected() {
        let mut e = sample();
        e.lifecycle = EntitlementState::Pending;
        let err = e
            .transition(EntitlementState::Active, CalendarDate::parse("2027-06-01").unwrap())
            .unwrap_err();
        assert!(matches!(err, TransitionError::OutsideEffectiveWindow { .. }));
        assert_eq!(e.lifecycle, EntitlementState::Pending); // unchanged on failure
    }

    #[test]
    fn terminal_states_cannot_revive() {
        for terminal in [EntitlementState::Revoked, EntitlementState::Expired] {
            let mut e = sample();
            e.lifecycle = terminal;
            for target in [EntitlementState::Active, EntitlementState::Pending] {
                let err = e.transition(target, CalendarDate::parse("2026-06-15").unwrap()).unwrap_err();
                assert!(matches!(err, TransitionError::InvalidTransition { .. }), "{terminal:?}->{target:?}");
            }
        }
    }

    #[test]
    fn active_cannot_return_to_pending() {
        let mut e = sample();
        let err = e
            .transition(EntitlementState::Pending, CalendarDate::parse("2026-06-15").unwrap())
            .unwrap_err();
        assert!(matches!(err, TransitionError::InvalidTransition { .. }));
    }

    #[test]
    fn owner_actor_is_independent_of_coverage() {
        // Owner role is a property of the actor, not the entitlement.
        let owner = Actor::new("own_1", Role::Owner);
        assert!(owner.is_owner());
    }
}
