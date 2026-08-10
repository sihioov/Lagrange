//! Todo 5 entitlement gate wiring for the raw zone.
//!
//! - [`governing_entitlement_reference`] records the governing licensed-data
//!   contract reference on each manifest row.
//! - [`raw_visibility`] tags a batch: only a governing `ACTIVE` entitlement on
//!   the as-of date makes a batch Member-readable; every other state is
//!   Owner-only (fail closed).
//! - [`read_batch_gated`] is the Member-facing read path: it authorizes the
//!   `dataset` surface through the shared [`EntitlementService`], falls back to
//!   the Owner-only development path, and denies with
//!   [`RawAccessError::DataEntitlementRequired`] otherwise.

pub use auth::entitlement::{AccessRequest, EntitlementService, KrUse};

use domain::{BatchId, TradingDate};

use crate::contract::StoredFile;
use crate::storage::{ManifestEntry, RawStore, StoreError};

/// How Member-facing layers may treat a raw batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawVisibility {
    /// The governing entitlement is ACTIVE on the as-of date.
    MemberReadable,
    /// No ACTIVE entitlement: Owner-only development access only.
    OwnerOnly,
}

impl RawVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MemberReadable => "member_readable",
            Self::OwnerOnly => "owner_only",
        }
    }
}

impl std::fmt::Display for RawVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tags the visibility of `krx_eod_bars` on `as_of`. Fails closed: only ACTIVE
/// grants Member visibility.
pub fn raw_visibility(
    service: &EntitlementService,
    as_of: auth::entitlement::CalendarDate,
) -> RawVisibility {
    use auth::entitlement::{DatasetId, EntitlementState};
    match service.governing_state(&DatasetId::krx_eod_bars(), as_of) {
        Some((_, EntitlementState::Active)) => RawVisibility::MemberReadable,
        _ => RawVisibility::OwnerOnly,
    }
}

/// The contract document reference of the entitlement governing
/// `krx_eod_bars` on `date`, if any. Recorded on manifest rows so every raw
/// batch traces to its licensed contract.
pub fn governing_entitlement_reference(
    service: &EntitlementService,
    date: TradingDate,
) -> Option<String> {
    use auth::entitlement::{CalendarDate, DatasetId};
    let as_of = CalendarDate::parse(&date.to_iso()).ok()?;
    let (id, _) = service.governing_state(&DatasetId::krx_eod_bars(), as_of)?;
    service
        .entitlements()
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.contract.document_reference.clone())
}

/// A typed failure of the Member-facing raw read path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawAccessError {
    /// The request requires an ACTIVE data entitlement on the batch's dataset.
    DataEntitlementRequired { batch_id: BatchId, detail: String },
    /// The requested path is Owner-only development; never available to Members.
    OwnerOnlyDevelopmentPath { batch_id: BatchId, detail: String },
    /// No such batch (not in the manifest).
    NotFound { batch_id: BatchId },
    /// Filesystem or content-verification failure.
    Io { context: String, detail: String },
}

impl std::fmt::Display for RawAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataEntitlementRequired { batch_id, detail } => {
                write!(
                    f,
                    "DATA_ENTITLEMENT_REQUIRED for batch {batch_id}: {detail}"
                )
            }
            Self::OwnerOnlyDevelopmentPath { batch_id, detail } => {
                write!(
                    f,
                    "OWNER_ONLY_DEVELOPMENT_PATH for batch {batch_id}: {detail}"
                )
            }
            Self::NotFound { batch_id } => write!(f, "raw batch {batch_id} not found"),
            Self::Io { context, detail } => write!(f, "raw read io failure ({context}): {detail}"),
        }
    }
}

impl std::error::Error for RawAccessError {}

/// Member-facing read of one raw batch, gated through the shared
/// [`EntitlementService`] (fail closed).
///
/// Resolution order:
/// 1. the `dataset` Member surface — allowed when an ACTIVE entitlement covers
///    the actor (Owner included) on the as-of date;
/// 2. the `dev_ingest` Owner-only development path — allowed for the Owner in
///    **any** entitlement state;
/// 3. otherwise denied: Members get `DataEntitlementRequired`.
pub fn read_batch_gated(
    store: &RawStore,
    entry: &ManifestEntry,
    service: &EntitlementService,
    req: &AccessRequest,
) -> Result<Vec<StoredFile>, RawAccessError> {
    let read = |store: &RawStore| -> Result<Vec<StoredFile>, RawAccessError> {
        store
            .read_batch_bytes(&entry.provider, &entry.market, entry)
            .map_err(|e| match e {
                StoreError::Io { context, source } => RawAccessError::Io {
                    context,
                    detail: source.to_string(),
                },
                other => RawAccessError::Io {
                    context: "raw-read".to_owned(),
                    detail: other.to_string(),
                },
            })
    };

    match service.authorize_use(KrUse::Dataset, req) {
        Ok(_) => read(store),
        Err(member_denial) => match service.authorize_owner_dev(KrUse::DevIngest, req) {
            Ok(_) => read(store),
            Err(_) => {
                let batch_id = entry.batch_id;
                if member_denial.code.as_str() == "OWNER_ONLY_DEVELOPMENT_PATH" {
                    Err(RawAccessError::OwnerOnlyDevelopmentPath {
                        batch_id,
                        detail: member_denial.to_string(),
                    })
                } else {
                    Err(RawAccessError::DataEntitlementRequired {
                        batch_id,
                        detail: member_denial.to_string(),
                    })
                }
            }
        },
    }
}
