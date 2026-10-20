//! Offline lineage verification over existing locked Raw snapshots.
//!
//! The result proves committed copy lineage, not provider row semantics or
//! admission. Explicit entitlement/clock context is not installer or DB authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::SecondsFormat;
use domain::UtcTimestamp;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};

use super::contract::{canonical_bytes_limited, canonical_file_evidence};
use super::source::{
    CommittedRawBatch, CommittedRawFile, CommittedRawSnapshot, DailyEvidenceError,
};
use super::{
    DailyEvidenceKindV1, DailyFileEvidenceV1Wire, DailyFileOriginV1Wire,
    DailyRequestMetadataV1Wire, DailyWireTimestampV1,
};
use crate::contract::{FetchMode, ResponseKind, SourceFileReference};

pub(super) const MAX_SOURCE_BYTES: u64 = 67_108_864;
const MAX_COPY_LINKS: usize = 16;
const MAX_JSON_DEPTH: usize = 32;

/// Finite diagnostics only: no provider body, path, entitlement or message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyVerificationError {
    Snapshot(DailyEvidenceError),
    InvalidContext,
    NonCredentialed,
    EntitlementMismatch,
    CopyCycle,
    CopyDepth,
    CopyMismatch,
    FutureRetrieval,
    InvalidEvidence,
    InvalidProviderJson,
    ProviderResponse,
    InvalidCalendar,
    InvalidBars,
    InvalidReference,
    InvalidActions,
    NonCashAction,
    SourceCoverageGap,
    SourceConflict,
    CalendarTargetMissing,
    CalendarConflict,
    CalendarCoverageGap,
    NoTradingSession,
    LimitExceeded,
    InvalidIdentity,
    Encoding,
}

impl DailyVerificationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Snapshot(error) => error.code(),
            Self::InvalidContext => "INVALID_CONTEXT",
            Self::NonCredentialed => "NON_CREDENTIALED",
            Self::EntitlementMismatch => "ENTITLEMENT_MISMATCH",
            Self::CopyCycle => "COPY_CYCLE",
            Self::CopyDepth => "COPY_DEPTH",
            Self::CopyMismatch => "COPY_MISMATCH",
            Self::FutureRetrieval => "FUTURE_RETRIEVAL",
            Self::InvalidEvidence => "INVALID_EVIDENCE",
            Self::InvalidProviderJson => "INVALID_PROVIDER_JSON",
            Self::ProviderResponse => "PROVIDER_RESPONSE",
            Self::InvalidCalendar => "INVALID_CALENDAR",
            Self::InvalidBars => "INVALID_BARS",
            Self::InvalidReference => "INVALID_REFERENCE",
            Self::InvalidActions => "INVALID_ACTIONS",
            Self::NonCashAction => "NONCASH_ACTION",
            Self::SourceCoverageGap => "SOURCE_COVERAGE_GAP",
            Self::SourceConflict => "SOURCE_CONFLICT",
            Self::CalendarTargetMissing => "CALENDAR_TARGET_MISSING",
            Self::CalendarConflict => "CALENDAR_CONFLICT",
            Self::CalendarCoverageGap => "CALENDAR_COVERAGE_GAP",
            Self::NoTradingSession => "NO_TRADING_SESSION",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::InvalidIdentity => "INVALID_IDENTITY",
            Self::Encoding => "ENCODING",
        }
    }
}

impl fmt::Display for DailyVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for DailyVerificationError {}

/// Borrowed complete scopes, captured using S2. Construction performs no I/O.
pub struct RawEvidenceSnapshots<'snapshot> {
    scopes: Vec<&'snapshot CommittedRawSnapshot>,
    entitlement_reference: &'snapshot str,
    observed_at: UtcTimestamp,
}

impl<'snapshot> RawEvidenceSnapshots<'snapshot> {
    pub fn new(
        scopes: &[&'snapshot CommittedRawSnapshot],
        entitlement_reference: &'snapshot str,
        observed_at: UtcTimestamp,
    ) -> Result<Self, DailyVerificationError> {
        if scopes.is_empty() || scopes.len() > 2 || entitlement_reference.trim().is_empty() {
            return Err(DailyVerificationError::InvalidContext);
        }
        let uid = scopes[0].scope().trusted_raw_owner_uid();
        let mut providers = BTreeSet::new();
        for scope in scopes {
            if scope.scope().trusted_raw_owner_uid() != uid
                || !providers.insert(scope.scope().provider())
            {
                return Err(DailyVerificationError::InvalidContext);
            }
        }
        Ok(Self {
            scopes: scopes.to_vec(),
            entitlement_reference,
            observed_at,
        })
    }

    pub(super) fn scopes(&self) -> &[&'snapshot CommittedRawSnapshot] {
        &self.scopes
    }

    /// Discovery selects only the configured entitlement. Other retained
    /// deliveries remain in the fully verified/budgeted snapshot but cannot
    /// supply evidence. Explicit references and every copy hop still use the
    /// strict entitlement check in resolve_original.
    pub fn matches_entitlement(&self, batch: &CommittedRawBatch) -> bool {
        batch.entry().entitlement_reference.as_deref() == Some(self.entitlement_reference)
    }

    pub(super) fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }

    fn lookup(
        &self,
        reference: &SourceFileReference,
    ) -> Result<CommittedRawFile<'snapshot>, DailyVerificationError> {
        let snapshot = self
            .scopes
            .iter()
            .copied()
            .find(|snapshot| {
                snapshot.scope().provider() == reference.provider
                    && snapshot.scope().market() == reference.market
            })
            .ok_or(DailyVerificationError::Snapshot(
                DailyEvidenceError::ScopeMismatch,
            ))?;
        snapshot
            .lookup_exact(reference)
            .map_err(DailyVerificationError::Snapshot)
    }

    pub(super) fn kis_snapshot(
        &self,
    ) -> Result<&'snapshot CommittedRawSnapshot, DailyVerificationError> {
        self.scopes
            .iter()
            .copied()
            .find(|snapshot| snapshot.scope().provider() == "kis")
            .ok_or(DailyVerificationError::InvalidContext)
    }
}

/// Verified lineage with an actual uncopied terminal file. Cannot be deserialized
/// or manufactured from public wire claims. Borrowing keeps snapshots alive.
pub struct ResolvedOriginal<'snapshot> {
    original: CommittedRawFile<'snapshot>,
    evidence: DailyFileEvidenceV1Wire,
    canonical_evidence: Vec<u8>,
    observed_at: UtcTimestamp,
}

impl<'snapshot> ResolvedOriginal<'snapshot> {
    pub fn evidence(&self) -> &DailyFileEvidenceV1Wire {
        &self.evidence
    }

    pub fn canonical_evidence_bytes(&self) -> &[u8] {
        &self.canonical_evidence
    }

    pub fn original_bytes(&self) -> &'snapshot [u8] {
        self.original.bytes()
    }

    pub fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }
}

pub fn resolve_original<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    reference: &SourceFileReference,
) -> Result<ResolvedOriginal<'snapshot>, DailyVerificationError> {
    let mut current = snapshots.lookup(reference)?;
    let mut origins: Vec<DailyFileOriginV1Wire> = Vec::new();
    let mut identities = BTreeSet::new();
    let first_bytes = current.bytes();
    loop {
        let batch = current.batch();
        let entry = batch.entry();
        let file = current.entry();
        if !identities.insert((
            entry.provider.as_str(),
            entry.market.as_str(),
            entry.batch_id,
            current.file_name(),
        )) {
            return Err(DailyVerificationError::CopyCycle);
        }
        if entry.mode != FetchMode::Credentialed || file.request.mode != FetchMode::Credentialed {
            return Err(DailyVerificationError::NonCredentialed);
        }
        if entry.entitlement_reference.as_deref() != Some(snapshots.entitlement_reference) {
            return Err(DailyVerificationError::EntitlementMismatch);
        }
        if entry.retrieved_at > snapshots.observed_at {
            return Err(DailyVerificationError::FutureRetrieval);
        }
        if current.bytes() != first_bytes {
            return Err(DailyVerificationError::CopyMismatch);
        }
        let origin = file_origin(&current)?;
        if origins.first().is_some_and(|first| {
            first.kind != origin.kind
                || first.wire_sha256 != origin.wire_sha256
                || first.size_bytes != origin.size_bytes
                || first.request != origin.request
                || first.response_continuation != origin.response_continuation
                || origins.last().is_some_and(|prior| {
                    prior.retrieved_at.as_timestamp() < origin.retrieved_at.as_timestamp()
                })
        }) {
            return Err(DailyVerificationError::CopyMismatch);
        }
        origins.push(origin);
        let Some(next) = file.copied_from.as_ref() else {
            break;
        };
        if origins.len() > MAX_COPY_LINKS {
            return Err(DailyVerificationError::CopyDepth);
        }
        current = snapshots.lookup(next)?;
    }
    let first = origins.remove(0);
    let original_retrieved_at = origins
        .last()
        .map_or(first.retrieved_at, |last| last.retrieved_at);
    let evidence = DailyFileEvidenceV1Wire {
        provider: first.provider,
        market: first.market,
        batch_id: first.batch_id,
        batch_date: first.batch_date,
        batch_json_sha256: first.batch_json_sha256,
        manifest_line_sha256: first.manifest_line_sha256,
        file_name: first.file_name,
        kind: first.kind,
        wire_sha256: first.wire_sha256,
        size_bytes: first.size_bytes,
        request: first.request,
        response_continuation: first.response_continuation,
        retrieved_at: first.retrieved_at,
        retrieval_precision: first.retrieval_precision,
        original_retrieved_at,
        original_retrieval_precision: "batch".to_owned(),
        copied_from: origins,
    };
    let (evidence, canonical_evidence) =
        canonical_file_evidence(&evidence).map_err(|_| DailyVerificationError::InvalidEvidence)?;
    Ok(ResolvedOriginal {
        original: current,
        evidence,
        canonical_evidence,
        observed_at: snapshots.observed_at,
    })
}

fn file_origin(
    file: &CommittedRawFile<'_>,
) -> Result<DailyFileOriginV1Wire, DailyVerificationError> {
    let batch = file.batch();
    let entry = batch.entry();
    let metadata = file.entry();
    let kind = match file.kind() {
        ResponseKind::Bars => DailyEvidenceKindV1::Bars,
        ResponseKind::Reference => DailyEvidenceKindV1::Reference,
        ResponseKind::Calendar => DailyEvidenceKindV1::Calendar,
        ResponseKind::CorporateActions => DailyEvidenceKindV1::CorporateActions,
        _ => return Err(DailyVerificationError::InvalidEvidence),
    };
    let mut request = DailyRequestMetadataV1Wire {
        endpoint: metadata.request.endpoint.clone(),
        query: metadata.request.query.clone(),
        headers: metadata.request.headers.clone(),
        mode: "credentialed".to_owned(),
    };
    request
        .query
        .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    request
        .headers
        .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let retrieved_at = DailyWireTimestampV1::parse(
        &entry
            .retrieved_at
            .as_datetime()
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
    )
    .map_err(|_| DailyVerificationError::InvalidEvidence)?;
    Ok(DailyFileOriginV1Wire {
        provider: entry.provider.clone(),
        market: entry.market.clone(),
        batch_id: entry.batch_id,
        batch_date: entry.date,
        batch_json_sha256: batch.batch_json_sha256().clone(),
        manifest_line_sha256: batch.manifest_line_sha256().clone(),
        file_name: file.file_name().to_owned(),
        kind,
        wire_sha256: file.content_hash().clone(),
        size_bytes: file.size_bytes(),
        request,
        response_continuation: metadata.response_continuation.clone(),
        retrieved_at,
        retrieval_precision: "batch".to_owned(),
    })
}

/// Reject duplicate keys before reading their second value, including escaped
/// aliases. Provider numbers are not restricted to D4's unsigned wire scalars.
pub(super) fn parse_provider_json(bytes: &[u8]) -> Result<Value, DailyVerificationError> {
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SOURCE_BYTES {
        return Err(DailyVerificationError::LimitExceeded);
    }
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let value = JsonSeed(MAX_JSON_DEPTH)
        .deserialize(&mut parser)
        .map_err(|_| DailyVerificationError::InvalidProviderJson)?;
    parser
        .end()
        .map_err(|_| DailyVerificationError::InvalidProviderJson)?;
    Ok(value)
}

struct JsonSeed(usize);

impl<'de> DeserializeSeed<'de> for JsonSeed {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for JsonSeed {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded provider JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("INVALID_NUMBER"))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        if self.0 == 0 {
            return Err(serde::de::Error::custom("JSON_DEPTH"));
        }
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(JsonSeed(self.0 - 1))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        if self.0 == 0 {
            return Err(serde::de::Error::custom("JSON_DEPTH"));
        }
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            match values.entry(key) {
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(serde::de::Error::custom("DUPLICATE_JSON_KEY"));
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(map.next_value_seed(JsonSeed(self.0 - 1))?);
                }
            }
        }
        Ok(Value::Object(values.into_iter().collect()))
    }
}

/// Sorting does not depend on serde_json's optional preserve_order feature.
struct SortedJson<'a>(&'a Value);

impl Serialize for SortedJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                let mut map = serializer.serialize_map(Some(keys.len()))?;
                for key in keys {
                    map.serialize_entry(key, &SortedJson(&object[key]))?;
                }
                map.end()
            }
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&SortedJson(value))?;
                }
                seq.end()
            }
            scalar => scalar.serialize(serializer),
        }
    }
}

pub(super) fn canonical_json(value: &Value) -> Result<Vec<u8>, DailyVerificationError> {
    canonical_bytes_limited(&SortedJson(value), MAX_SOURCE_BYTES)
        .map_err(|_| DailyVerificationError::Encoding)
}
