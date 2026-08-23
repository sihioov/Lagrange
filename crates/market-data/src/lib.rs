//! `market-data` - Lagrange Station market-data domain: instruments, calendars, bars, quality, curation.
//!
//! Todo 8 delivers the **provider-neutral EOD raw contract** and the **KRX
//! provider adapter** with **immutable Raw ingestion**:
//!
//! - [`contract`] - the raw response envelope (bytes, retrieval time, provider
//!   request metadata, batch id, content hash) and response-kind taxonomy.
//! - [`provider`] - the `EodProvider` trait and the `KrxProvider` adapter with a
//!   recorded-synthetic mode (CI) and an Owner-only credentialed mode.
//! - [`storage`] - the immutable raw zone (`data/raw/provider=krx/market=kr/...`)
//!   with append-only manifests.
//! - [`ingest`] - the collector pipeline: fetch -> validate -> store -> manifest.
//! - [`entitlement`] - Todo 5 gate wiring: non-ACTIVE batches are Owner-only and
//!   Member reads fail with `DATA_ENTITLEMENT_REQUIRED`.
//! - [`redact`] - secret/redaction scan for logs (never expose provider keys/data).
//!
//! Todo 9 delivers the **canonical instrument master with KRX/KIS/provider
//! aliases** ([`instrument_master`]) and the **versioned KRX trading
//! calendar** ([`calendar`]):
//!
//! - canonical `InstrumentId = {symbol}.KRX` (design §6.4); a ticker change
//!   remaps the alias and appends versioned alias history — the canonical
//!   identity is never silently changed;
//! - venue/currency/price/size/lot metadata and listing/delisting status with
//!   effective dates (requirements §8.2, FR-DATA-002);
//! - Asia/Seoul calendar with explicit sessions 09:00-15:30 KST, explicit
//!   holiday data with source/version/hash provenance, last/next trading-day
//!   queries, holiday month-end handling, and timezone-aware open/close
//!   instants (FR-DATA-003/005).
//!
//! Todo 10 delivers the **curated zone**: normalized, partitioned, versioned
//! Curated Parquet with point-in-time corporate actions ([`curate`]):
//!
//! - [`curate::curate_batch`]: Raw batch into `data/curated/...` partitions,
//!   all-or-nothing, one dataset version per curation;
//! - raw OHLCV bars (execution) plus split-adjusted and total-return series
//!   (signals); a correction produces a NEW dataset version (old immutable);
//! - corporate actions with `available_at` point-in-time visibility (and an
//!   optional source-provided `announced_at`; future observations rejected);
//! - `PRICE_RETURN_ONLY | TOTAL_RETURN_CAPABLE` capability per version.

pub mod calendar;
pub mod candidate;
pub mod candidate_normalize;
pub mod contract;
pub mod curate;
pub mod entitlement;
pub mod freshness;
pub mod historical_price_only;
mod historical_price_only_artifact;
pub mod ingest;
pub mod instrument_master;
pub mod kind_correction_normalize;
pub mod kind_normalize;
pub mod normalize;
pub mod provider;
pub mod providers;
pub mod publication;
pub mod quality;
pub mod range_normalize;
pub mod range_to_canonical;
pub mod redact;
pub mod storage;
pub mod validate;

pub use calendar::{
    CalendarError, CalendarProvenance, Holiday, KrCalendar, KrCalendarSpec, SessionTimes, krx_2020,
};
pub use candidate::{
    CandidateDataError, CandidateDocument, CandidateSourcePin, CandidateUniverseKey,
    FinancialPeriodKind, FundamentalDocument, FundamentalObservation, FundamentalProfile,
    IndexMembershipDocument, IndexMembershipObservation, InvestorClass, InvestorFlowDocument,
    InvestorFlowObservation, MarketStatusDocument, MarketStatusObservation, SectorDocument,
    SectorObservation, StatementScope, latest_flows_as_of, latest_fundamental_as_of, members_as_of,
    parse_candidate_envelope, sectors_as_of, validate_candidate_document,
};
pub use candidate_normalize::{
    CandidateNormalizationOutcome, CandidateNormalizeError,
    deterministic_kis_candidate_normalized_batch_id, normalize_kis_candidate_batch,
    normalize_kis_candidate_envelopes,
};
pub use contract::{
    ALL_RESPONSE_KINDS, CANDIDATE_MASTER_RESPONSE_KINDS, CANDIDATE_RESPONSE_KINDS,
    DISCLOSURE_RESPONSE_KINDS, EOD_RESPONSE_KINDS, FetchMode, MARKET_KR, PROVIDER_KIND_DISCLOSURE,
    PROVIDER_KIND_DISCLOSURE_CORRECTION, PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
    PROVIDER_KIND_DISCLOSURE_NORMALIZED, PROVIDER_KIS, PROVIDER_KIS_CANDIDATE,
    PROVIDER_KIS_CANDIDATE_NORMALIZED, PROVIDER_KIS_DAILY_RANGE,
    PROVIDER_KIS_DAILY_RANGE_NORMALIZED, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX, PROVIDER_OPENDART,
    RawEnvelope, RequestMetadata, ResponseKind, StoredFile,
};
pub use curate::actions::{CorporateAction, CorporateActionType};
pub use curate::schema::{
    ADJUSTED_BARS_SCHEMA_ID, BARS_SCHEMA_ID, CORPORATE_ACTIONS_SCHEMA_ID,
    CORPORATE_ACTIONS_SCHEMA_VERSION, CORPORATE_ACTIONS_SCHEMA_VERSION_KEY, CuratedBar,
    CuratedSchema, TOTAL_RETURN_BARS_SCHEMA_ID,
};
pub use curate::{
    Capability, CurateError, CurateOutcome, CurateRequest, CurateStore, CuratedArtifactRef,
    DatasetManifest, PriceCurationEvidence, PriceInstrumentCoverage, SourceBatchRef, curate_batch,
    curate_generation, curation_inputs_from_raw, curation_inputs_from_raw_entries,
    dataset_manifest_hash, price_curation_evidence, price_curation_evidence_for_generation,
};
pub use historical_price_only::{
    HISTORICAL_PRICE_ONLY_FACTOR_SCALE, HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION,
    HISTORICAL_PRICE_ONLY_PRICE_SCALE, HistoricalPriceOnlyAudience, HistoricalPriceOnlyBar,
    HistoricalPriceOnlyBonusEvidence, HistoricalPriceOnlyCandidate, HistoricalPriceOnlyError,
    HistoricalPriceOnlyMetadata, HistoricalPriceOnlySessionProvenance,
    materialize_historical_price_only_beta,
};
pub use historical_price_only_artifact::{
    HistoricalPriceOnlyArtifactApprovalSummary, HistoricalPriceOnlyArtifactError,
    VerifiedHistoricalPriceOnlyArtifact, read_historical_price_only_artifact,
    write_historical_price_only_artifact,
};
pub use ingest::{
    IngestError, IngestOutcome, IngestRequest, ingest_bundle, ingest_bundle_with_kinds,
    ingest_kis_action_range, ingest_kis_action_range_with_batch_id, ingest_kis_bundle,
    ingest_kis_candidate_bundle, ingest_kis_candidate_bundle_with_kinds,
    ingest_kis_daily_bars_range, ingest_kis_daily_bars_range_with_batch_id,
};
pub use instrument_master::{
    AliasNamespace, Instrument, InstrumentAlias, InstrumentMaster, ListingReason, MasterError,
    seed_universe,
};
pub use kind_correction_normalize::{
    KindCorrectionMembership, KindCorrectionNormalizationLineage,
    KindCorrectionNormalizationOutcome, KindCorrectionNormalizationSourceFile,
    KindCorrectionVersion, KindCorrectionViewerError,
    deterministic_kind_correction_normalized_batch_id, normalize_kind_correction_batch,
    parse_kind_correction_membership, parse_kind_correction_viewer,
};
pub use kind_normalize::{
    InstrumentIdentity, KindDisclosureObservation, KindNormalizationLineage,
    KindNormalizationOutcome, KindNormalizationSourceFile, KindNormalizeError, RequiredField,
    RowLocation, TimezoneAssumption, deterministic_kind_disclosure_normalized_batch_id,
    normalize_kind_disclosure_batch, parse_kind_disclosure_pages,
};
pub use normalize::{
    NormalizationLineage, NormalizationOutcome, NormalizationSourceFile, NormalizeError,
    deterministic_kis_normalized_batch_id, normalize_kis_batch, normalize_kis_envelopes,
};
pub use provider::{
    CredentialRef, EodProvider, FetchRequest, KrxMode, KrxProvider, ProviderError, RecordedBundle,
};
pub use providers::fsc_krx_listed::{
    FIXED_ETF11, FSC_KRX_LISTED_ENDPOINT, FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
    FSC_KRX_LISTED_PATH, FSC_KRX_LISTED_PROVIDER, FSC_KRX_LISTED_RESPONSE_KIND, FixedEtfIdentity,
    FscKrxListedAvailability, FscKrxListedError, FscKrxListedOutcome, FscKrxListedProvider,
    FscKrxListedRead, ITEM_INFO_MAX_PAGES, ITEM_INFO_PAGE_SIZE,
};
pub use providers::kind::{
    CapturedPage, KIND_CORRECTION_ARTIFACT_KIND, KIND_CORRECTION_ENTRY_URL,
    KIND_CORRECTION_SURFACE, KIND_CORRECTION_TERMINATION, KIND_CORRECTION_TERMINATION_STAGE,
    KIND_CORRECTION_VIEWER_ENDPOINT, KIND_CORRECTION_VIEWER_FILE,
    KIND_CORRECTION_VIEWER_ORIGIN_PATH, KIND_DETAIL_ETF_DISCLOSURE_ENDPOINT,
    KIND_DISCLOSURE_MAX_PAGES, KIND_DISCLOSURE_PAGE_SIZE, KIND_ETF_DISCLOSURE_ENDPOINT,
    KindCorrectionCapture, KindCorrectionResponseDiagnostics, KindError, KindSurface,
    MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT, MAX_KIND_CORRECTION_METADATA_BYTES,
    MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES, MAX_KIND_CORRECTION_VIEWER_BYTES,
    ingest_correction_capture, ingest_disclosure_capture,
};
pub use providers::kis::{
    KIS_ACTION_CLASS_COUNT, KIS_ACTION_MAX_PAGES, KR_ETF_CORE_SYMBOLS, KisActionRangeScope,
    KisProvider, KisRead, MAX_DAILY_BAR_OBSERVATIONS, MAX_DAILY_BAR_WINDOWS,
};
pub use providers::kis_candidate::{
    KIS_CANDIDATE_SUPPORTED_KINDS, KIS_CANDIDATE_UNSUPPORTED_KINDS, KisCandidateProvider,
};
pub use providers::kis_candidate_master::{
    CandidateMarket, CandidateMasterArchive, CandidateMasterArchiveProvenance,
    CandidateMasterError, CandidateMasterProvider, CandidateMasterRead, CandidateMasterRow,
    CandidateMasterSnapshot, CandidateMasterSource, CandidateMembershipFlags,
    CandidateSectorFields, CandidateStatusRawFlags, IDXCODE_EMPTY_SENTINEL, IDXCODE_MASTER_MEMBER,
    IDXCODE_MASTER_URL, IDXCODE_MST_URL, IdxCodeMasterRow, IndexCodeMasterRow,
    KIS_CANDIDATE_MASTER_SOURCES, KOSDAQ_CODE_MST_URL, KOSDAQ_MASTER_MEMBER, KOSDAQ_MASTER_URL,
    KOSPI_CODE_MST_URL, KOSPI_MASTER_MEMBER, KOSPI_MASTER_URL, KisCandidateMasterProvider,
    KisCandidateMasterRead, gate_candidate_master_publication, ingest_kis_candidate_master_bundle,
    parse_candidate_master_batch, parse_candidate_master_envelopes,
    parse_candidate_master_snapshot, parse_kis_candidate_master, require_candidate_master_pit,
    validate_candidate_master_archive,
};
pub use providers::opendart::{
    DISCLOSURE_LIST_MAX_PAGES, DISCLOSURE_LIST_PAGE_COUNT, DisclosureListFilter,
    OPENDART_DISCLOSURE_LIST_ENDPOINT, OPENDART_ENTITY_COMPANY_ENDPOINT,
    OPENDART_ENTITY_CORPCODE_ENDPOINT, OpenDartError, OpenDartOutcome, OpenDartProvider,
    OpenDartRead,
};
pub use publication::{
    CalendarFact, CalendarSessionType, DataBatchKind, PublicationBundle, PublicationError,
    PublicationFile,
};
pub use quality::{
    AdminApproval, ApprovalAudit, DataUse, DataUseDenial, ExclusionRecord, FreshnessPolicy,
    IssueCode, OptionalExclusion, QualityError, QualityGate, QualityIssue, QualityPolicy,
    QualityReport, Severity, apply_approval,
};
pub use range_normalize::{
    ExpectedRangeSessions, RangeNormalizationLineage, RangeNormalizationOutcome,
    RangeNormalizationSourceFile, RangeNormalizationSourceRow, RangeNormalizeError,
    deterministic_range_normalized_batch_id, deterministic_range_normalized_batch_id_with_identity,
    normalize_kis_daily_range, normalize_kis_daily_range_batch,
};
pub use range_to_canonical::{
    HISTORICAL_PRICE_ONLY_BETA_CONTRACT, HISTORICAL_PRICE_ONLY_BETA_END,
    HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT, HISTORICAL_PRICE_ONLY_BETA_SOURCE_BATCH_ID,
    HISTORICAL_PRICE_ONLY_BETA_SOURCE_FILE_COUNT, HISTORICAL_PRICE_ONLY_BETA_START,
    HistoricalPriceOnlyBetaInput, HistoricalPriceOnlyBetaPins, HistoricalPriceOnlySessionWitness,
    NON_STRICT_PIT_POLICY_ID, RANGE_CANONICAL_BRIDGE_VERSION, REQUIRED_ACTION_KINDS, RangeAction,
    RangeCanonicalBarCandidate, RangeCanonicalCandidate, RangeCanonicalError,
    VerifiedRangeCanonicalEvidence, build_range_canonical_candidate,
    discover_historical_price_only_beta_pins, load_verified_range_canonical_evidence,
    verify_historical_price_only_beta_input, write_evidence_package,
};
pub use storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};
