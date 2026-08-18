//! The KIS candidate master-file source.
//!
//! KIS publishes the domestic stock files as public ZIP downloads rather than
//! as authenticated JSON endpoints.  The files are useful evidence for the
//! candidate vertical, but they are *not* an EOD reference file: they contain
//! no historical effective/announcement/availability facts.  This module
//! consequently has a separate [`ResponseKind::CandidateMaster`] and an
//! explicit Raw ingest path.  Nothing in this module emits a publishable
//! membership, sector, or market-status document.
//!
//! The fixed-width layouts below are the layouts used by KIS's official
//! `kis_kospi_code_mst.py`, `kis_kosdaq_code_mst.py`, and `sector_code.py`
//! examples.  Widths are byte widths in the CP949 member, not Rust character
//! widths.  The official Python examples use `row[-228:]`/`row[-222:]` with
//! the line terminator still attached; after the terminator is removed, the
//! named KOSPI/KOSDAQ tails are 227/221 bytes and the three-field prefix is
//! 9+12+40 bytes.  There is no synthetic reserved byte in the layout.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::{Cursor, Read};

use domain::{BatchId, ContentHash, UtcTimestamp};
use encoding_rs::EUC_KR;
use kis_client::KisError;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::contract::{
    FetchMode, PROVIDER_KIS_CANDIDATE, RawEnvelope, RequestMetadata, ResponseKind,
};
use crate::ingest::IngestRequest;
use crate::provider::{FetchRequest, ProviderError};
use crate::storage::{ManifestEntry, RawStore, StoreError};

/// Public KIS download URL for the KOSPI instrument master.
pub const KOSPI_MASTER_URL: &str =
    "https://new.real.download.dws.co.kr/common/master/kospi_code.mst.zip";
/// Public KIS download URL for the KOSDAQ instrument master.
pub const KOSDAQ_MASTER_URL: &str =
    "https://new.real.download.dws.co.kr/common/master/kosdaq_code.mst.zip";
/// Public KIS download URL for the industry-code master.
pub const IDXCODE_MASTER_URL: &str =
    "https://new.real.download.dws.co.kr/common/master/idxcode.mst.zip";

/// Compatibility aliases used by operators who call these files `*_CODE_MST`.
pub const KOSPI_CODE_MST_URL: &str = KOSPI_MASTER_URL;
pub const KOSDAQ_CODE_MST_URL: &str = KOSDAQ_MASTER_URL;
pub const IDXCODE_MST_URL: &str = IDXCODE_MASTER_URL;

/// The member names are part of the KIS public-file contract.  A ZIP with a
/// different member name is rejected instead of being guessed or extracted.
pub const KOSPI_MASTER_MEMBER: &str = "kospi_code.mst";
pub const KOSDAQ_MASTER_MEMBER: &str = "kosdaq_code.mst";
pub const IDXCODE_MASTER_MEMBER: &str = "idxcode.mst";
/// KIS's `idxcode.mst` uses an empty-name `9999` row as a sentinel rather
/// than an index.  It is skipped only for this exact code/name combination.
pub const IDXCODE_EMPTY_SENTINEL: &str = "9999";

/// Maximum accepted archive and decompressed member sizes.  Current official
/// files are far below these limits; the limits prevent a ZIP bomb from
/// crossing the immutable Raw boundary.
pub const MAX_MASTER_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MASTER_MEMBER_BYTES: usize = 16 * 1024 * 1024;

/// Stable order for the three source downloads and their deterministic
/// lineage.  Do not use a hash-map iteration order for these sources.
pub const KIS_CANDIDATE_MASTER_SOURCES: [CandidateMasterSource; 3] = [
    CandidateMasterSource::Kospi,
    CandidateMasterSource::Kosdaq,
    CandidateMasterSource::IdxCode,
];

/// One of the three KIS master downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateMasterSource {
    Kospi,
    Kosdaq,
    IdxCode,
}

impl CandidateMasterSource {
    pub const fn url(self) -> &'static str {
        match self {
            Self::Kospi => KOSPI_MASTER_URL,
            Self::Kosdaq => KOSDAQ_MASTER_URL,
            Self::IdxCode => IDXCODE_MASTER_URL,
        }
    }

    pub const fn member_name(self) -> &'static str {
        match self {
            Self::Kospi => KOSPI_MASTER_MEMBER,
            Self::Kosdaq => KOSDAQ_MASTER_MEMBER,
            Self::IdxCode => IDXCODE_MASTER_MEMBER,
        }
    }

    pub const fn archive_file_name(self) -> &'static str {
        match self {
            Self::Kospi => "kospi_code.mst.zip",
            Self::Kosdaq => "kosdaq_code.mst.zip",
            Self::IdxCode => "idxcode.mst.zip",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kospi => "kospi",
            Self::Kosdaq => "kosdaq",
            Self::IdxCode => "idxcode",
        }
    }
}

impl std::fmt::Display for CandidateMasterSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// `thiserror` treats a field literally named `source` as an error source.  A
// source label is useful in the typed error variants, so make the small enum a
// harmless, display-only error value rather than renaming every public error
// field away from the established `source` spelling.
impl std::error::Error for CandidateMasterSource {}

/// Async seam for the public KIS ZIP downloads.  Production can install an
/// HTTP implementation; tests install a small in-memory fixture.  The seam
/// returns exact bytes and never exposes a reqwest/client dependency here.
#[allow(async_fn_in_trait)]
pub trait KisCandidateMasterRead: std::fmt::Debug + Send + Sync {
    fn get(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, KisError>> + Send;
}

/// A validated ZIP member, retaining both archive and decompressed hashes for
/// provenance and deterministic replay.
#[derive(Debug, Clone)]
pub struct CandidateMasterArchive {
    pub source: CandidateMasterSource,
    pub url: String,
    pub archive_hash: ContentHash,
    pub member_name: String,
    pub member_hash: ContentHash,
    pub archive_size: u64,
    pub member_size: u64,
    pub member_bytes: Vec<u8>,
    pub retrieved_at: Option<UtcTimestamp>,
}

impl CandidateMasterArchive {
    pub fn provenance(&self) -> CandidateMasterArchiveProvenance {
        CandidateMasterArchiveProvenance {
            source: self.source,
            url: self.url.clone(),
            archive_hash: self.archive_hash.clone(),
            member_name: self.member_name.clone(),
            member_hash: self.member_hash.clone(),
            archive_size: self.archive_size,
            member_size: self.member_size,
            retrieved_at: self.retrieved_at,
        }
    }
}

/// Stable archive/member lineage recorded in a parsed snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateMasterArchiveProvenance {
    pub source: CandidateMasterSource,
    pub url: String,
    pub archive_hash: ContentHash,
    pub member_name: String,
    pub member_hash: ContentHash,
    pub archive_size: u64,
    pub member_size: u64,
    #[serde(default)]
    pub retrieved_at: Option<UtcTimestamp>,
}

/// Typed provider failure for ZIP validation, fixed-width decoding, and the
/// deliberate point-in-time publication gate.
#[derive(Debug, thiserror::Error)]
pub enum CandidateMasterError {
    #[error("candidate master request must contain only candidate_master")]
    InvalidRequest,
    #[error("KIS candidate master transport failed for {source}: {error}")]
    Transport {
        source: CandidateMasterSource,
        #[source]
        error: KisError,
    },
    #[error("candidate master archive for {source} exceeds {limit} bytes")]
    ArchiveTooLarge {
        source: CandidateMasterSource,
        limit: usize,
    },
    #[error("candidate master archive for {source} is invalid: {detail}")]
    InvalidArchive {
        source: CandidateMasterSource,
        detail: String,
    },
    #[error("candidate master archive for {source} has unsafe member path {member:?}")]
    UnsafeMemberPath {
        source: CandidateMasterSource,
        member: String,
    },
    #[error("candidate master archive for {source} has unexpected member {member:?}")]
    UnexpectedMember {
        source: CandidateMasterSource,
        member: String,
    },
    #[error("candidate master member for {source} exceeds {limit} bytes")]
    MemberTooLarge {
        source: CandidateMasterSource,
        limit: usize,
    },
    #[error("candidate master member for {source} is empty")]
    EmptyMember { source: CandidateMasterSource },
    #[error(
        "candidate master member CRC mismatch for {source}: expected {expected:#010x}, got {actual:#010x}"
    )]
    CrcMismatch {
        source: CandidateMasterSource,
        expected: u32,
        actual: u32,
    },
    #[error("candidate master {source} has invalid CP949 at {context}")]
    InvalidEncoding {
        source: CandidateMasterSource,
        context: String,
    },
    #[error("candidate master {source} has invalid decoded width at {context}: {detail}")]
    InvalidDecodedWidth {
        source: CandidateMasterSource,
        context: String,
        detail: String,
    },
    #[error("candidate master {source} has an invalid field domain at {context}: {detail}")]
    InvalidDomain {
        source: CandidateMasterSource,
        context: String,
        detail: String,
    },
    #[error("candidate master {source} has a duplicate key {key:?}")]
    DuplicateKey {
        source: CandidateMasterSource,
        key: String,
    },
    #[error("candidate master source set is incomplete: missing {source}")]
    MissingSource { source: CandidateMasterSource },
    #[error("candidate master source set contains duplicate {source}")]
    DuplicateSource { source: CandidateMasterSource },
    #[error("candidate master raw batch cannot be parsed: {detail}")]
    InvalidBatch { detail: String },
    #[error("candidate master raw store failure: {0}")]
    Store(#[from] StoreError),
    #[error("candidate master is not publishable: insufficient PIT evidence ({detail})")]
    InsufficientPitEvidence { detail: String },
}

/// Credential-free KIS candidate master provider.  It validates every ZIP
/// before returning an envelope but stores the original archive bytes without
/// modification.
#[derive(Debug)]
pub struct KisCandidateMasterProvider<R: KisCandidateMasterRead> {
    reader: R,
}

/// Short aliases for callers that do not need to distinguish this provider
/// from the REST-backed candidate adapter.
pub type CandidateMasterProvider<R> = KisCandidateMasterProvider<R>;
pub use KisCandidateMasterRead as CandidateMasterRead;

impl<R: KisCandidateMasterRead> KisCandidateMasterProvider<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub const fn provider_id(&self) -> &'static str {
        PROVIDER_KIS_CANDIDATE
    }

    pub const fn fetch_mode(&self) -> FetchMode {
        FetchMode::Credentialed
    }

    /// Downloads and validates all three archives in stable source order.
    pub async fn fetch_archives(
        &self,
        req: &FetchRequest,
    ) -> Result<Vec<CandidateMasterArchive>, ProviderError> {
        validate_master_request(req).map_err(|_| ProviderError::InvalidConfiguration {
            detail: "KIS candidate master requests require exactly candidate_master".to_owned(),
        })?;
        let mut archives = Vec::with_capacity(KIS_CANDIDATE_MASTER_SOURCES.len());
        for source in KIS_CANDIDATE_MASTER_SOURCES {
            let bytes =
                self.reader
                    .get(source.url())
                    .await
                    .map_err(|error| ProviderError::Remote {
                        provider: PROVIDER_KIS_CANDIDATE,
                        kind: ResponseKind::CandidateMaster,
                        code: error.code(),
                        retryable: error.is_retryable(kis_client::RequestKind::Read),
                        diagnostic: None,
                        detail: format!("{source}: {error}"),
                    })?;
            let mut archive =
                validate_candidate_master_archive(source, &bytes).map_err(|error| {
                    ProviderError::Remote {
                        provider: PROVIDER_KIS_CANDIDATE,
                        kind: ResponseKind::CandidateMaster,
                        code: "KIS_MASTER_SCHEMA_DRIFT",
                        retryable: false,
                        diagnostic: None,
                        detail: error.to_string(),
                    }
                })?;
            archive.retrieved_at = Some(req.now);
            archives.push(archive);
        }
        Ok(archives)
    }

    /// Fetches the three exact ZIP bodies as `CandidateMaster` Raw envelopes.
    pub async fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        self.fetch_envelopes(req).await
    }
}

fn validate_master_request(req: &FetchRequest) -> Result<(), CandidateMasterError> {
    if req.market != "kr" || req.kinds.len() != 1 || req.kinds[0] != ResponseKind::CandidateMaster {
        return Err(CandidateMasterError::InvalidRequest);
    }
    Ok(())
}

/// Validate and extract one official archive.  The returned member is what
/// the typed parser consumes; the original ZIP remains the immutable Raw body.
pub fn validate_candidate_master_archive(
    source: CandidateMasterSource,
    archive_bytes: &[u8],
) -> Result<CandidateMasterArchive, CandidateMasterError> {
    if archive_bytes.is_empty() {
        return Err(CandidateMasterError::InvalidArchive {
            source,
            detail: "archive is empty".to_owned(),
        });
    }
    if archive_bytes.len() > MAX_MASTER_ARCHIVE_BYTES {
        return Err(CandidateMasterError::ArchiveTooLarge {
            source,
            limit: MAX_MASTER_ARCHIVE_BYTES,
        });
    }

    let mut archive = ZipArchive::new(Cursor::new(archive_bytes)).map_err(|error| {
        CandidateMasterError::InvalidArchive {
            source,
            detail: error.to_string(),
        }
    })?;
    if archive.len() != 1 {
        return Err(CandidateMasterError::InvalidArchive {
            source,
            detail: format!("expected exactly one member, got {}", archive.len()),
        });
    }
    let mut member = archive
        .by_index(0)
        .map_err(|error| CandidateMasterError::InvalidArchive {
            source,
            detail: error.to_string(),
        })?;
    let member_name = member.name().to_owned();
    if member_name.is_empty()
        || member_name.contains('/')
        || member_name.contains('\\')
        || member_name.contains("..")
        || member_name.starts_with('.')
        || member_name.starts_with('/')
    {
        return Err(CandidateMasterError::UnsafeMemberPath {
            source,
            member: member_name,
        });
    }
    if member_name != source.member_name() {
        return Err(CandidateMasterError::UnexpectedMember {
            source,
            member: member_name,
        });
    }
    if member.is_dir() {
        return Err(CandidateMasterError::InvalidArchive {
            source,
            detail: "member is a directory".to_owned(),
        });
    }
    if member
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err(CandidateMasterError::InvalidArchive {
            source,
            detail: "member is a symbolic link".to_owned(),
        });
    }
    let declared_size =
        usize::try_from(member.size()).map_err(|_| CandidateMasterError::MemberTooLarge {
            source,
            limit: MAX_MASTER_MEMBER_BYTES,
        })?;
    if declared_size == 0 {
        return Err(CandidateMasterError::EmptyMember { source });
    }
    if declared_size > MAX_MASTER_MEMBER_BYTES {
        return Err(CandidateMasterError::MemberTooLarge {
            source,
            limit: MAX_MASTER_MEMBER_BYTES,
        });
    }
    let expected_crc = member.crc32();
    let mut member_bytes = Vec::with_capacity(declared_size);
    member.read_to_end(&mut member_bytes).map_err(|error| {
        CandidateMasterError::InvalidArchive {
            source,
            detail: format!("member read failed: {error}"),
        }
    })?;
    if member_bytes.len() != declared_size {
        return Err(CandidateMasterError::InvalidArchive {
            source,
            detail: format!(
                "declared member size {declared_size}, read {}",
                member_bytes.len()
            ),
        });
    }
    let actual_crc = crc32fast::hash(&member_bytes);
    if actual_crc != expected_crc {
        return Err(CandidateMasterError::CrcMismatch {
            source,
            expected: expected_crc,
            actual: actual_crc,
        });
    }
    Ok(CandidateMasterArchive {
        source,
        url: source.url().to_owned(),
        archive_hash: ContentHash::from_bytes(archive_bytes),
        member_name,
        member_hash: ContentHash::from_bytes(&member_bytes),
        archive_size: archive_bytes.len() as u64,
        member_size: member_bytes.len() as u64,
        member_bytes,
        retrieved_at: None,
    })
}

/// A source record as parsed from a KOSPI or KOSDAQ master member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateMasterRow {
    pub market: CandidateMarket,
    pub short_code: String,
    pub standard_code: String,
    pub name: String,
    /// KIS `scrt_grp_cls_code`; only `ST` is treated as ordinary equity.
    pub scrt_grp_cls_code: String,
    pub ordinary_equity: bool,
    pub membership: CandidateMembershipFlags,
    pub sector: CandidateSectorFields,
    pub status: CandidateStatusRawFlags,
}

pub type KisCandidateMasterRow = CandidateMasterRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateMarket {
    Kospi,
    Kosdaq,
}

/// Membership flags retain the raw KIS code as well as the two candidate
/// universe booleans.  A boolean is true only for the documented positive
/// code; no membership is inferred from security type or sector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateMembershipFlags {
    pub kospi200_raw: String,
    pub kospi200: bool,
    pub kospi100_raw: String,
    pub kospi50_raw: String,
    pub krx_raw: String,
    pub kosdaq150_raw: String,
    pub kosdaq150: bool,
    pub krx100_raw: String,
}

/// Raw fixed-width industry fields plus the optional idxcode lookup.  The
/// lookup is descriptive only; it does not turn a current sector into a PIT
/// classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSectorFields {
    pub large_raw: String,
    pub medium_raw: String,
    pub small_raw: String,
    pub kospi200_sector_raw: String,
    pub idxcode_name: Option<String>,
}

/// Raw KIS status/action flags.  They deliberately remain strings because
/// KIS uses both `Y/N` and numeric/blank flag domains across the files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateStatusRawFlags {
    pub trht_yn: String,
    pub sltr_yn: String,
    pub mang_issu_yn: String,
    pub mrkt_alrm_cls_code: String,
    pub mrkt_alrm_risk_adnt_yn: String,
    pub insn_pbnt_yn: String,
    pub byps_lstn_yn: String,
    pub flng_cls_code: String,
    pub fcam_mod_cls_code: String,
    pub icic_cls_code: String,
    /// Final one-byte `stln_able_yn` field in the official tail layout.
    pub stln_able_yn: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCodeMasterRow {
    pub code: String,
    pub name: String,
}

pub type IdxCodeMasterRow = IndexCodeMasterRow;

/// Deterministic typed snapshot assembled from all three master members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateMasterSnapshot {
    pub schema_version: u32,
    pub kospi: Vec<CandidateMasterRow>,
    pub kosdaq: Vec<CandidateMasterRow>,
    pub idxcode: Vec<IndexCodeMasterRow>,
    pub provenance: Vec<CandidateMasterArchiveProvenance>,
}

impl CandidateMasterSnapshot {
    pub fn lineage_hash(&self) -> ContentHash {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"kis-candidate-master-lineage-v1\n");
        for source in KIS_CANDIDATE_MASTER_SOURCES {
            if let Some(provenance) = self.provenance.iter().find(|p| p.source == source) {
                bytes.extend_from_slice(source.as_str().as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(provenance.url.as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(provenance.archive_hash.as_str().as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(provenance.member_name.as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(provenance.member_hash.as_str().as_bytes());
                bytes.push(b'\n');
            }
        }
        ContentHash::from_bytes(&bytes)
    }

    pub fn kospi_rows(&self) -> &[CandidateMasterRow] {
        &self.kospi
    }

    pub fn kosdaq_rows(&self) -> &[CandidateMasterRow] {
        &self.kosdaq
    }

    pub fn idxcode_rows(&self) -> &[IndexCodeMasterRow] {
        &self.idxcode
    }

    /// Deliberately fails closed: current KIS master files do not provide
    /// historical PIT effective/announcement/availability, audit, or capital
    /// impairment evidence required by publication.
    pub fn require_publishable(&self) -> Result<(), CandidateMasterError> {
        require_candidate_master_pit(self)
    }
}

/// The explicit PIT gate for this source.  Keep this function public so a
/// caller cannot accidentally bypass the decision by treating the snapshot as
/// an `IndexMembershipDocument`/`SectorDocument`/`MarketStatusDocument`.
pub fn require_candidate_master_pit(
    _snapshot: &CandidateMasterSnapshot,
) -> Result<(), CandidateMasterError> {
    Err(CandidateMasterError::InsufficientPitEvidence {
        detail: "KIS master snapshots have no historical PIT effective/announcement/available_at, audit, or capital-impairment evidence".to_owned(),
    })
}

pub fn gate_candidate_master_publication(
    snapshot: &CandidateMasterSnapshot,
) -> Result<(), CandidateMasterError> {
    require_candidate_master_pit(snapshot)
}

/// Parse three exact archives into a typed snapshot.  This convenience API is
/// useful for an explicit offline ingest/replay and keeps network out of tests.
pub fn parse_candidate_master_snapshot(
    kospi_zip: &[u8],
    kosdaq_zip: &[u8],
    idxcode_zip: &[u8],
) -> Result<CandidateMasterSnapshot, CandidateMasterError> {
    let archives = [
        validate_candidate_master_archive(CandidateMasterSource::Kospi, kospi_zip)?,
        validate_candidate_master_archive(CandidateMasterSource::Kosdaq, kosdaq_zip)?,
        validate_candidate_master_archive(CandidateMasterSource::IdxCode, idxcode_zip)?,
    ];
    parse_validated_candidate_master_archives(&archives)
}

pub fn parse_kis_candidate_master(
    kospi_zip: &[u8],
    kosdaq_zip: &[u8],
    idxcode_zip: &[u8],
) -> Result<CandidateMasterSnapshot, CandidateMasterError> {
    parse_candidate_master_snapshot(kospi_zip, kosdaq_zip, idxcode_zip)
}

/// Parse already-validated archives, preserving exact source order in
/// provenance while making row ordering equal to the source member order.
pub fn parse_validated_candidate_master_archives(
    archives: &[CandidateMasterArchive],
) -> Result<CandidateMasterSnapshot, CandidateMasterError> {
    let mut by_source = BTreeMap::new();
    for archive in archives {
        if by_source.insert(archive.source, archive).is_some() {
            return Err(CandidateMasterError::DuplicateSource {
                source: archive.source,
            });
        }
    }
    for source in KIS_CANDIDATE_MASTER_SOURCES {
        if !by_source.contains_key(&source) {
            return Err(CandidateMasterError::MissingSource { source });
        }
    }
    if archives.len() != KIS_CANDIDATE_MASTER_SOURCES.len() {
        return Err(CandidateMasterError::InvalidBatch {
            detail: format!("expected three archives, got {}", archives.len()),
        });
    }

    let index = parse_idxcode_member(by_source[&CandidateMasterSource::IdxCode])?;
    let index_names: BTreeMap<_, _> = index
        .iter()
        .map(|row| (row.code.clone(), row.name.clone()))
        .collect();
    let kospi = parse_equity_member(
        CandidateMasterSource::Kospi,
        CandidateMarket::Kospi,
        by_source[&CandidateMasterSource::Kospi],
        &index_names,
    )?;
    let kosdaq = parse_equity_member(
        CandidateMasterSource::Kosdaq,
        CandidateMarket::Kosdaq,
        by_source[&CandidateMasterSource::Kosdaq],
        &index_names,
    )?;
    let provenance = KIS_CANDIDATE_MASTER_SOURCES
        .into_iter()
        .map(|source| by_source[&source].provenance())
        .collect();
    Ok(CandidateMasterSnapshot {
        schema_version: 1,
        kospi,
        kosdaq,
        idxcode: index,
        provenance,
    })
}

/// Parse candidate-master envelopes produced by the explicit provider path.
pub fn parse_candidate_master_envelopes(
    envelopes: &[RawEnvelope],
) -> Result<CandidateMasterSnapshot, CandidateMasterError> {
    let mut archives = Vec::with_capacity(envelopes.len());
    for envelope in envelopes {
        if envelope.kind != ResponseKind::CandidateMaster {
            return Err(CandidateMasterError::InvalidBatch {
                detail: format!(
                    "expected candidate_master envelope, got {} ({})",
                    envelope.kind, envelope.file_name
                ),
            });
        }
        let actual_hash = ContentHash::from_bytes(&envelope.bytes);
        if actual_hash != envelope.content_hash {
            return Err(CandidateMasterError::InvalidBatch {
                detail: format!(
                    "archive envelope {} content hash mismatch: recorded {}, actual {}",
                    envelope.file_name, envelope.content_hash, actual_hash
                ),
            });
        }
        let source = KIS_CANDIDATE_MASTER_SOURCES
            .into_iter()
            .find(|source| source.archive_file_name() == envelope.file_name)
            .ok_or_else(|| CandidateMasterError::InvalidBatch {
                detail: format!(
                    "unknown candidate master archive file {}",
                    envelope.file_name
                ),
            })?;
        let mut archive = validate_candidate_master_archive(source, &envelope.bytes)?;
        archive.retrieved_at = Some(envelope.retrieved_at);
        archives.push(archive);
    }
    parse_validated_candidate_master_archives(&archives)
}

/// Parse the exact three files from an immutable `provider=kis-candidate` Raw
/// batch.  This is intentionally a separate function from candidate JSON
/// normalization and cannot silently emit publishable documents.
pub fn parse_candidate_master_batch(
    store: &RawStore,
    entry: &ManifestEntry,
) -> Result<CandidateMasterSnapshot, CandidateMasterError> {
    if entry.provider != PROVIDER_KIS_CANDIDATE || entry.market != "kr" {
        return Err(CandidateMasterError::InvalidBatch {
            detail: format!(
                "expected {PROVIDER_KIS_CANDIDATE}/kr, got {}/{}",
                entry.provider, entry.market
            ),
        });
    }
    let stored = store.read_batch_bytes(&entry.provider, &entry.market, entry)?;
    let envelopes = entry
        .files
        .iter()
        .zip(stored)
        .map(|(file, stored)| {
            RawEnvelope::new(
                entry.batch_id,
                file.kind,
                file.file_name.clone(),
                stored.bytes,
                entry.retrieved_at,
                file.request.clone(),
            )
        })
        .collect::<Vec<_>>();
    parse_candidate_master_envelopes(&envelopes)
}

/// Explicitly ingest all three candidate master ZIP bodies under the existing
/// `provider=kis-candidate` scope.  The body passed to Raw is the exact byte
/// sequence returned by the transport; ZIP validation is performed before the
/// write and no decompressed member is stored in its place.
pub async fn ingest_kis_candidate_master_bundle<R: KisCandidateMasterRead>(
    store: &RawStore,
    provider: &KisCandidateMasterProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<crate::ingest::IngestOutcome, crate::ingest::IngestError> {
    let batch_id = BatchId::generate();
    let fetch_req = FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: vec![ResponseKind::CandidateMaster],
        now: req.now,
        batch_id,
    };
    // `fetch_archives` validates and returns decompressed members.  We fetch
    // the transport bodies once more only if this provider is used through the
    // lower-level helper; `fetch_envelopes` below is the normal exact-body
    // path.  Keep this wrapper delegated to that helper for one-read behavior.
    let envelopes = provider.fetch_envelopes(&fetch_req).await?;
    crate::ingest::persist_candidate_master_bundle(
        store,
        provider.provider_id(),
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

impl<R: KisCandidateMasterRead> KisCandidateMasterProvider<R> {
    /// Exact-body variant used by the ingest path.  It is separate from the
    /// typed `fetch_archives` API so no decompressed bytes can be mistaken for
    /// Raw evidence.
    pub async fn fetch_envelopes(
        &self,
        req: &FetchRequest,
    ) -> Result<Vec<RawEnvelope>, ProviderError> {
        validate_master_request(req).map_err(|_| ProviderError::InvalidConfiguration {
            detail: "KIS candidate master requests require exactly candidate_master".to_owned(),
        })?;
        let mut output = Vec::with_capacity(3);
        for source in KIS_CANDIDATE_MASTER_SOURCES {
            let bytes =
                self.reader
                    .get(source.url())
                    .await
                    .map_err(|error| ProviderError::Remote {
                        provider: PROVIDER_KIS_CANDIDATE,
                        kind: ResponseKind::CandidateMaster,
                        code: error.code(),
                        retryable: error.is_retryable(kis_client::RequestKind::Read),
                        diagnostic: None,
                        detail: format!("{source}: {error}"),
                    })?;
            validate_candidate_master_archive(source, &bytes).map_err(|error| {
                ProviderError::Remote {
                    provider: PROVIDER_KIS_CANDIDATE,
                    kind: ResponseKind::CandidateMaster,
                    code: "KIS_MASTER_SCHEMA_DRIFT",
                    retryable: false,
                    diagnostic: None,
                    detail: error.to_string(),
                }
            })?;
            output.push(RawEnvelope::new(
                req.batch_id,
                ResponseKind::CandidateMaster,
                source.archive_file_name(),
                bytes,
                req.now,
                RequestMetadata {
                    endpoint: source.url().to_owned(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    mode: FetchMode::Credentialed,
                },
            ));
        }
        Ok(output)
    }
}

// Official KIS fixed-width field specifications.  The first three fields of
// each equity member are [short code 9, standard code 12, Korean name 40].
// The tail lists are the official Python `field_specs`; their sums are the
// post-newline byte widths of the KOSPI/KOSDAQ records.
pub const KOSPI_PREFIX_FIELD_SPECS: [usize; 3] = [9, 12, 40];
pub const KOSDAQ_PREFIX_FIELD_SPECS: [usize; 3] = [9, 12, 40];
pub const KOSPI_TAIL_FIELD_SPECS: [usize; 70] = [
    2, 1, 4, 4, 4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 9,
    5, 5, 1, 1, 1, 2, 1, 1, 1, 2, 2, 2, 3, 1, 3, 12, 12, 8, 15, 21, 2, 7, 1, 1, 1, 1, 1, 9, 9, 9,
    5, 9, 8, 9, 3, 1, 1, 1,
];
pub const KOSDAQ_TAIL_FIELD_SPECS: [usize; 64] = [
    2, 1, 4, 4, 4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 9, 5, 5, 1, 1, 1,
    2, 1, 1, 1, 2, 2, 2, 3, 1, 3, 12, 12, 8, 15, 21, 2, 7, 1, 1, 1, 1, 9, 9, 9, 5, 9, 8, 9, 3, 1,
    1, 1,
];
pub const IDXCODE_FIELD_SPECS: [usize; 3] = [1, 4, 40];
pub const IDXCODE_LINE_WIDTH: usize = 45;

const KOSPI_LINE_WIDTH: usize = 288;
const KOSDAQ_LINE_WIDTH: usize = 282;

fn parse_equity_member(
    source: CandidateMasterSource,
    market: CandidateMarket,
    archive: &CandidateMasterArchive,
    index_names: &BTreeMap<String, String>,
) -> Result<Vec<CandidateMasterRow>, CandidateMasterError> {
    let expected_width = match source {
        CandidateMasterSource::Kospi => KOSPI_LINE_WIDTH,
        CandidateMasterSource::Kosdaq => KOSDAQ_LINE_WIDTH,
        CandidateMasterSource::IdxCode => {
            return Err(CandidateMasterError::InvalidBatch {
                detail: "idxcode cannot be parsed as an equity member".to_owned(),
            });
        }
    };
    let prefix_specs = match source {
        CandidateMasterSource::Kospi => &KOSPI_PREFIX_FIELD_SPECS,
        CandidateMasterSource::Kosdaq => &KOSDAQ_PREFIX_FIELD_SPECS,
        CandidateMasterSource::IdxCode => unreachable!(),
    };
    let tail_specs: &[usize] = match source {
        CandidateMasterSource::Kospi => &KOSPI_TAIL_FIELD_SPECS,
        CandidateMasterSource::Kosdaq => &KOSDAQ_TAIL_FIELD_SPECS,
        CandidateMasterSource::IdxCode => unreachable!(),
    };
    let lines = member_lines(source, &archive.member_bytes)?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(lines.len());
    for (line_no, line) in lines.into_iter().enumerate() {
        if line.len() != expected_width {
            return Err(CandidateMasterError::InvalidDecodedWidth {
                source,
                context: format!("line {line_no}"),
                detail: format!("expected {expected_width} bytes, got {}", line.len()),
            });
        }
        let prefix_len: usize = prefix_specs.iter().sum();
        let prefix = decode_fields(source, &line[..prefix_len], prefix_specs, line_no, "prefix")?;
        let tail = decode_fields(source, &line[prefix_len..], tail_specs, line_no, "tail")?;
        let short_code = trim_ascii(&prefix[0]);
        let standard_code = trim_ascii(&prefix[1]);
        let name = trim_ascii(&prefix[2]);
        validate_ascii_identifier(source, line_no, "short_code", &short_code, 9)?;
        validate_ascii_identifier_exact(source, line_no, "standard_code", &standard_code, 12)?;
        if name.is_empty() {
            return Err(CandidateMasterError::InvalidDomain {
                source,
                context: format!("line {line_no}.name"),
                detail: "name is empty".to_owned(),
            });
        }
        let security_group = trim_ascii(&tail[0]);
        if !matches!(
            security_group.as_str(),
            "ST" | "MF"
                | "RT"
                | "SC"
                | "IF"
                | "DR"
                | "EW"
                | "EF"
                | "SW"
                | "SR"
                | "BC"
                | "FE"
                | "FS"
                | "EN"
                | "PF"
        ) {
            return Err(CandidateMasterError::InvalidDomain {
                source,
                context: format!("line {line_no}.scrt_grp_cls_code"),
                detail: format!("unknown security group {security_group:?}"),
            });
        }
        let status_offset = match source {
            CandidateMasterSource::Kospi => 34,
            CandidateMasterSource::Kosdaq => 29,
            CandidateMasterSource::IdxCode => unreachable!(),
        };
        validate_code_domain(
            source,
            line_no,
            &tail[status_offset + 3],
            "mrkt_alrm_cls_code",
            &["", "00", "01", "02", "03"],
        )?;
        validate_code_domain(
            source,
            line_no,
            &tail[status_offset + 7],
            "flng_cls_code",
            &["", "00", "01", "02", "03", "04", "05", "06", "99"],
        )?;
        validate_code_domain(
            source,
            line_no,
            &tail[status_offset + 8],
            "fcam_mod_cls_code",
            &["", "00", "01", "02", "99"],
        )?;
        validate_code_domain(
            source,
            line_no,
            &tail[status_offset + 9],
            "icic_cls_code",
            &["", "00", "01", "02", "03", "99"],
        )?;
        if !seen.insert(short_code.clone()) {
            return Err(CandidateMasterError::DuplicateKey {
                source,
                key: short_code,
            });
        }

        let (
            kospi200_raw,
            kospi100_raw,
            kospi50_raw,
            krx_raw,
            kosdaq150_raw,
            krx100_raw,
            sector_raw,
        ) = match source {
            CandidateMasterSource::Kospi => (
                trim_ascii(&tail[8]),
                trim_ascii(&tail[9]),
                trim_ascii(&tail[10]),
                trim_ascii(&tail[11]),
                String::new(),
                trim_ascii(&tail[14]),
                trim_ascii(&tail[8]),
            ),
            CandidateMasterSource::Kosdaq => (
                String::new(),
                String::new(),
                String::new(),
                trim_ascii(&tail[7]),
                trim_ascii(&tail[25]),
                trim_ascii(&tail[9]),
                String::new(),
            ),
            CandidateMasterSource::IdxCode => unreachable!(),
        };
        let kospi200 = !kospi200_raw.is_empty() && kospi200_raw != "0";
        let kosdaq150 = kosdaq150_raw == "Y";
        rows.push(CandidateMasterRow {
            market,
            short_code,
            standard_code,
            name,
            scrt_grp_cls_code: security_group.clone(),
            ordinary_equity: security_group == "ST",
            membership: CandidateMembershipFlags {
                kospi200_raw,
                kospi200,
                kospi100_raw,
                kospi50_raw,
                krx_raw,
                kosdaq150_raw,
                kosdaq150,
                krx100_raw,
            },
            sector: CandidateSectorFields {
                large_raw: trim_ascii(&tail[2]),
                medium_raw: trim_ascii(&tail[3]),
                small_raw: trim_ascii(&tail[4]),
                kospi200_sector_raw: sector_raw,
                idxcode_name: index_names.get(trim_ascii(&tail[2]).as_str()).cloned(),
            },
            status: CandidateStatusRawFlags {
                trht_yn: trim_ascii(&tail[status_offset]),
                sltr_yn: trim_ascii(&tail[status_offset + 1]),
                mang_issu_yn: trim_ascii(&tail[status_offset + 2]),
                mrkt_alrm_cls_code: trim_ascii(&tail[status_offset + 3]),
                mrkt_alrm_risk_adnt_yn: trim_ascii(&tail[status_offset + 4]),
                insn_pbnt_yn: trim_ascii(&tail[status_offset + 5]),
                byps_lstn_yn: trim_ascii(&tail[status_offset + 6]),
                flng_cls_code: trim_ascii(&tail[status_offset + 7]),
                fcam_mod_cls_code: trim_ascii(&tail[status_offset + 8]),
                icic_cls_code: trim_ascii(&tail[status_offset + 9]),
                stln_able_yn: trim_ascii(tail.last().expect("validated non-empty tail")),
            },
        });
    }
    if rows.is_empty() {
        return Err(CandidateMasterError::EmptyMember { source });
    }
    Ok(rows)
}

fn parse_idxcode_member(
    archive: &CandidateMasterArchive,
) -> Result<Vec<IndexCodeMasterRow>, CandidateMasterError> {
    let source = CandidateMasterSource::IdxCode;
    let lines = member_lines(source, &archive.member_bytes)?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(lines.len());
    for (line_no, line) in lines.into_iter().enumerate() {
        if line.len() != IDXCODE_LINE_WIDTH {
            return Err(CandidateMasterError::InvalidDecodedWidth {
                source,
                context: format!("line {line_no}"),
                detail: format!("expected {IDXCODE_LINE_WIDTH} bytes, got {}", line.len()),
            });
        }
        let fields = decode_fields(source, &line, &IDXCODE_FIELD_SPECS, line_no, "idxcode")?;
        // KIS's idxcode layout reserves one leading byte; the four-byte code
        // starts at offset 1.  The name starts at offset 5.
        let code = trim_ascii(&fields[1]);
        let name = trim_ascii(&fields[2]);
        if code.len() != 4 || !code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(CandidateMasterError::InvalidDomain {
                source,
                context: format!("line {line_no}.idxcode"),
                detail: format!("expected four ASCII alphanumeric bytes, got {code:?}"),
            });
        }
        if name.is_empty() {
            if code == IDXCODE_EMPTY_SENTINEL {
                continue;
            }
            return Err(CandidateMasterError::InvalidDomain {
                source,
                context: format!("line {line_no}.name"),
                detail: "name is empty".to_owned(),
            });
        }
        if !seen.insert(code.clone()) {
            return Err(CandidateMasterError::DuplicateKey { source, key: code });
        }
        rows.push(IndexCodeMasterRow { code, name });
    }
    if rows.is_empty() {
        return Err(CandidateMasterError::EmptyMember { source });
    }
    Ok(rows)
}

fn member_lines(
    source: CandidateMasterSource,
    bytes: &[u8],
) -> Result<Vec<Vec<u8>>, CandidateMasterError> {
    let mut lines = Vec::new();
    for (index, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if raw.is_empty() && index + 1 == bytes.split(|byte| *byte == b'\n').count() {
            continue;
        }
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            return Err(CandidateMasterError::InvalidDecodedWidth {
                source,
                context: format!("line {index}"),
                detail: "blank line".to_owned(),
            });
        }
        lines.push(line.to_vec());
    }
    if lines.is_empty() {
        return Err(CandidateMasterError::EmptyMember { source });
    }
    Ok(lines)
}

fn decode_fields(
    source: CandidateMasterSource,
    bytes: &[u8],
    widths: &[usize],
    line_no: usize,
    label: &str,
) -> Result<Vec<String>, CandidateMasterError> {
    let expected: usize = widths.iter().sum();
    if bytes.len() != expected {
        return Err(CandidateMasterError::InvalidDecodedWidth {
            source,
            context: format!("{label} line {line_no}"),
            detail: format!("expected {expected} bytes, got {}", bytes.len()),
        });
    }
    let mut output = Vec::with_capacity(widths.len());
    let mut offset = 0;
    for (field_no, width) in widths.iter().copied().enumerate() {
        let field = &bytes[offset..offset + width];
        let decoded = EUC_KR
            .decode_without_bom_handling_and_without_replacement(field)
            .ok_or_else(|| CandidateMasterError::InvalidEncoding {
                source,
                context: format!("{label} line {line_no} field {field_no}"),
            })?
            .into_owned();
        let decoded_width = decoded.chars().count();
        if decoded_width > width || decoded.chars().any(|ch| ch == '\0' || ch == '\u{fffd}') {
            return Err(CandidateMasterError::InvalidDecodedWidth {
                source,
                context: format!("{label} line {line_no} field {field_no}"),
                detail: format!("decoded width {decoded_width} for byte width {width}"),
            });
        }
        output.push(decoded);
        offset += width;
    }
    Ok(output)
}

fn trim_ascii(value: &str) -> String {
    value.trim_matches(' ').to_owned()
}

fn validate_ascii_identifier(
    source: CandidateMasterSource,
    line_no: usize,
    field: &str,
    value: &str,
    max_len: usize,
) -> Result<(), CandidateMasterError> {
    if value.is_empty()
        || value.len() > max_len
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(CandidateMasterError::InvalidDomain {
            source,
            context: format!("line {line_no}.{field}"),
            detail: format!("expected nonempty ASCII alphanumeric <= {max_len}, got {value:?}"),
        });
    }
    Ok(())
}

fn validate_ascii_identifier_exact(
    source: CandidateMasterSource,
    line_no: usize,
    field: &str,
    value: &str,
    width: usize,
) -> Result<(), CandidateMasterError> {
    if value.len() != width || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(CandidateMasterError::InvalidDomain {
            source,
            context: format!("line {line_no}.{field}"),
            detail: format!("expected {width} ASCII alphanumeric bytes, got {value:?}"),
        });
    }
    Ok(())
}

fn validate_code_domain(
    source: CandidateMasterSource,
    line_no: usize,
    value: &str,
    field: &str,
    allowed: &[&str],
) -> Result<(), CandidateMasterError> {
    let value = trim_ascii(value);
    if !allowed.contains(&value.as_str()) {
        return Err(CandidateMasterError::InvalidDomain {
            source,
            context: format!("line {line_no}.{field}"),
            detail: format!("unexpected code {value:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use domain::{BatchId, TradingDate};
    use kis_client::KisError;
    use zip::write::FileOptions;

    use super::*;

    #[derive(Debug, Clone)]
    struct FixtureReader {
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    }

    impl FixtureReader {
        fn new(files: BTreeMap<String, Vec<u8>>) -> Self {
            Self {
                files: Arc::new(Mutex::new(files)),
            }
        }
    }

    impl KisCandidateMasterRead for FixtureReader {
        async fn get(&self, url: &str) -> Result<Vec<u8>, KisError> {
            self.files
                .lock()
                .expect("fixture lock")
                .get(url)
                .cloned()
                .ok_or_else(|| KisError::Connect {
                    reason: format!("missing fixture {url}"),
                })
        }
    }

    fn zip_member(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            zip.start_file(name, FileOptions::<()>::default())
                .expect("start member");
            zip.write_all(body).expect("member body");
            zip.finish().expect("finish zip");
        }
        out.into_inner()
    }

    fn equity_line(source: CandidateMasterSource, short: &str, group: &str) -> Vec<u8> {
        equity_line_with_name(source, short, group, "삼성전자")
    }

    fn equity_line_with_name(
        source: CandidateMasterSource,
        short: &str,
        group: &str,
        name: &str,
    ) -> Vec<u8> {
        let (tail_specs, tail_len) = match source {
            CandidateMasterSource::Kospi => (&KOSPI_TAIL_FIELD_SPECS[..], 227),
            CandidateMasterSource::Kosdaq => (&KOSDAQ_TAIL_FIELD_SPECS[..], 221),
            CandidateMasterSource::IdxCode => unreachable!(),
        };
        let mut line = Vec::with_capacity(61 + tail_len);
        let mut short_bytes = vec![b' '; 9];
        short_bytes[..short.len()].copy_from_slice(short.as_bytes());
        line.extend(short_bytes);
        line.extend_from_slice(b"KR0000000000");
        let (name_bytes, _, had_errors) = EUC_KR.encode(name);
        assert!(!had_errors);
        assert!(name_bytes.len() <= 40);
        line.extend_from_slice(name_bytes.as_ref());
        line.resize(61, b' ');
        assert_eq!(line.len(), 61);
        let mut tail = vec![b' '; tail_len];
        let mut offsets = Vec::new();
        let mut offset = 0;
        for width in tail_specs {
            offsets.push(offset);
            offset += *width;
        }
        tail[offsets[0]..offsets[0] + 2].copy_from_slice(group.as_bytes());
        tail[offsets[2]..offsets[2] + 4].copy_from_slice(b"0001");
        tail[offsets[3]..offsets[3] + 4].copy_from_slice(b"0002");
        tail[offsets[4]..offsets[4] + 4].copy_from_slice(b"0003");
        match source {
            CandidateMasterSource::Kospi => {
                tail[offsets[8]] = b'1';
                tail[offsets[9]] = b'Y';
                tail[offsets[10]] = b'N';
                tail[offsets[11]] = b'Y';
                tail[offsets[14]] = b'Y';
            }
            CandidateMasterSource::Kosdaq => {
                tail[offsets[7]] = b'Y';
                tail[offsets[9]] = b'N';
                tail[offsets[25]] = b'Y';
            }
            CandidateMasterSource::IdxCode => unreachable!(),
        }
        // All status code fields are valid blank/00 values in the fixture.
        let status_indices = match source {
            CandidateMasterSource::Kospi => [37, 41, 42, 43],
            CandidateMasterSource::Kosdaq => [32, 36, 37, 38],
            CandidateMasterSource::IdxCode => unreachable!(),
        };
        for (index, value) in
            status_indices
                .into_iter()
                .zip([b"00".as_slice(), b"00", b"00", b"00"])
        {
            tail[offsets[index]..offsets[index] + value.len()].copy_from_slice(value);
        }
        let last_offset = tail.len() - 1;
        tail[last_offset] = b'Y';
        line.extend(tail);
        line
    }

    fn idx_line(code: &str, name: &str) -> Vec<u8> {
        let (name_bytes, _, had_errors) = EUC_KR.encode(name);
        assert!(!had_errors);
        let mut line = Vec::with_capacity(IDXCODE_LINE_WIDTH);
        line.push(b'0');
        line.extend_from_slice(code.as_bytes());
        line.extend_from_slice(name_bytes.as_ref());
        line.resize(IDXCODE_LINE_WIDTH, b' ');
        line
    }

    fn fixture_archives() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let kospi = [equity_line(CandidateMasterSource::Kospi, "005930", "ST")].concat();
        let kosdaq = [equity_line(CandidateMasterSource::Kosdaq, "123456", "EF")].concat();
        let idx = idx_line("0001", "종합");
        (
            zip_member(KOSPI_MASTER_MEMBER, &format_bytes(&kospi)),
            zip_member(KOSDAQ_MASTER_MEMBER, &format_bytes(&kosdaq)),
            zip_member(IDXCODE_MASTER_MEMBER, &format_bytes(&idx)),
        )
    }

    fn format_bytes(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        out.push(b'\n');
        out
    }

    #[test]
    fn parses_widths_cp949_idx_offset_and_raw_flags() {
        let (kospi, kosdaq, idx) = fixture_archives();
        let snapshot = parse_candidate_master_snapshot(&kospi, &kosdaq, &idx).expect("snapshot");
        assert_eq!(snapshot.kospi.len(), 1);
        assert!(snapshot.kospi[0].ordinary_equity);
        assert!(snapshot.kospi[0].membership.kospi200);
        assert_eq!(snapshot.kospi[0].status.stln_able_yn, "Y");
        assert_eq!(snapshot.kosdaq[0].scrt_grp_cls_code, "EF");
        assert!(!snapshot.kosdaq[0].ordinary_equity);
        assert!(snapshot.kosdaq[0].membership.kosdaq150);
        assert_eq!(snapshot.idxcode[0].code, "0001");
        assert_eq!(snapshot.idxcode[0].name, "종합");
        assert_eq!(snapshot.provenance.len(), 3);
        assert_eq!(snapshot.provenance[0].member_name, KOSPI_MASTER_MEMBER);
    }

    #[test]
    fn uses_official_byte_offsets_for_full_cp949_name_and_tail() {
        let full_name = "가나다라마바사아자차카타파하거너더러머버";
        let (encoded, _, had_errors) = EUC_KR.encode(full_name);
        assert!(!had_errors);
        assert_eq!(encoded.len(), 40);
        let line = equity_line_with_name(CandidateMasterSource::Kospi, "005930", "ST", full_name);
        assert_eq!(line.len(), KOSPI_LINE_WIDTH);
        assert_eq!(&line[..9], b"005930   ");
        assert_eq!(&line[61..63], b"ST");
        assert_eq!(
            &line[61 + KOSPI_TAIL_FIELD_SPECS.iter().sum::<usize>() - 1..],
            b"Y"
        );
        let archive = validate_candidate_master_archive(
            CandidateMasterSource::Kospi,
            &zip_member(KOSPI_MASTER_MEMBER, &format_bytes(&line)),
        )
        .expect("archive");
        let rows = parse_equity_member(
            CandidateMasterSource::Kospi,
            CandidateMarket::Kospi,
            &archive,
            &BTreeMap::new(),
        )
        .expect("row");
        assert_eq!(rows[0].name, full_name);
        assert_eq!(rows[0].scrt_grp_cls_code, "ST");
        assert_eq!(rows[0].status.stln_able_yn, "Y");
    }

    #[test]
    fn accepts_alphanumeric_idxcode_and_skips_only_9999_empty_sentinel() {
        let body = [
            format_bytes(&idx_line("E199", "영문지수")),
            format_bytes(&idx_line(IDXCODE_EMPTY_SENTINEL, "")),
        ]
        .concat();
        let archive = validate_candidate_master_archive(
            CandidateMasterSource::IdxCode,
            &zip_member(IDXCODE_MASTER_MEMBER, &body),
        )
        .expect("archive");
        let rows = parse_idxcode_member(&archive).expect("idx rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "E199");

        let bad_empty = validate_candidate_master_archive(
            CandidateMasterSource::IdxCode,
            &zip_member(IDXCODE_MASTER_MEMBER, &format_bytes(&idx_line("E199", ""))),
        )
        .expect("archive");
        assert!(matches!(
            parse_idxcode_member(&bad_empty),
            Err(CandidateMasterError::InvalidDomain { .. })
        ));
    }

    #[test]
    fn rejects_extra_member_traversal_invalid_encoding_and_domain() {
        let (_kospi, kosdaq, idx) = fixture_archives();
        let mut extra = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut extra);
            zip.start_file(KOSPI_MASTER_MEMBER, FileOptions::<()>::default())
                .unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("../evil", FileOptions::<()>::default())
                .unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        assert!(parse_candidate_master_snapshot(&extra.into_inner(), &kosdaq, &idx).is_err());

        let bad_line = {
            let mut line = equity_line(CandidateMasterSource::Kospi, "005930", "ST");
            line[61] = 0xff;
            line
        };
        let bad = zip_member(KOSPI_MASTER_MEMBER, &format_bytes(&bad_line));
        assert!(matches!(
            parse_candidate_master_snapshot(&bad, &kosdaq, &idx),
            Err(CandidateMasterError::InvalidEncoding { .. })
        ));

        let bad_domain = {
            let line = equity_line(CandidateMasterSource::Kospi, "005930", "XX");
            zip_member(KOSPI_MASTER_MEMBER, &format_bytes(&line))
        };
        assert!(matches!(
            parse_candidate_master_snapshot(&bad_domain, &kosdaq, &idx),
            Err(CandidateMasterError::InvalidDomain { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_rows_and_pit_gate_is_typed() {
        let line = equity_line(CandidateMasterSource::Kospi, "005930", "ST");
        let body = [format_bytes(&line), format_bytes(&line)].concat();
        let (kospi, kosdaq, idx) = fixture_archives();
        let duplicate = zip_member(KOSPI_MASTER_MEMBER, &body);
        assert!(matches!(
            parse_candidate_master_snapshot(&duplicate, &kosdaq, &idx),
            Err(CandidateMasterError::DuplicateKey { .. })
        ));
        let snapshot = parse_candidate_master_snapshot(&kospi, &kosdaq, &idx).unwrap();
        assert!(matches!(
            snapshot.require_publishable(),
            Err(CandidateMasterError::InsufficientPitEvidence { .. })
        ));
    }

    #[tokio::test]
    async fn provider_and_explicit_ingest_keep_exact_archive_bodies() {
        let (kospi, kosdaq, idx) = fixture_archives();
        let files = [
            (KOSPI_MASTER_URL.to_owned(), kospi.clone()),
            (KOSDAQ_MASTER_URL.to_owned(), kosdaq.clone()),
            (IDXCODE_MASTER_URL.to_owned(), idx.clone()),
        ]
        .into_iter()
        .collect();
        let provider = KisCandidateMasterProvider::new(FixtureReader::new(files));
        let request = FetchRequest {
            market: "kr".to_owned(),
            date: TradingDate::parse("2026-08-18").unwrap(),
            kinds: vec![ResponseKind::CandidateMaster],
            now: UtcTimestamp::parse_rfc3339("2026-08-18T00:00:00Z").unwrap(),
            batch_id: BatchId::generate(),
        };
        let envelopes = provider.fetch_envelopes(&request).await.unwrap();
        assert_eq!(envelopes.len(), 3);
        assert_eq!(envelopes[0].bytes, kospi);
        assert_eq!(envelopes[1].bytes, kosdaq);
        assert_eq!(envelopes[2].bytes, idx);
        let snapshot = parse_candidate_master_envelopes(&envelopes).unwrap();
        assert_eq!(snapshot.lineage_hash(), snapshot.lineage_hash());
    }
}
