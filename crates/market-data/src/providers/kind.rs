//! KIND (`kind.krx.co.kr`) ETF disclosure search — Raw-only **capture
//! ingest**, no network, no reader trait.
//!
//! KIND has no API: its ETF disclosure search (`fnSearch()`) runs only
//! inside a browser, issuing `POST
//! https://kind.krx.co.kr/disclosure/disclosurebystocktype.do` with
//! `method=searchDisclosureByStockTypeEtfSub`. Because the interaction can
//! only happen in a browser, a **separate browser-capture stage** (not part
//! of this crate) drives that interaction and hands this module the bytes
//! and form fields it already captured, one call per response page. This
//! module performs no HTTP of its own and defines no reader trait — there is
//! nothing left to fetch, only already-retrieved evidence to validate and
//! commit to the immutable Raw zone.
//!
//! Pipeline shape: caller-supplied [`CapturedPage`]s -> this module's own
//! validation -> [`RawStore::store_batch`] (one atomic call that persists
//! the batch and appends its manifest row) -> a [`ManifestEntry`]. Nothing
//! is written unless every page validates; see [`ingest_disclosure_capture`].
//!
//! [`crate::contract::ResponseKind::DisclosureIndex`] is reused rather than
//! given a new variant: a KIND disclosure search-index page and an OpenDART
//! `list.json` page are the same response class (a paginated
//! disclosure-search index), and the batch's `provider` field
//! ([`PROVIDER_KIND_DISCLOSURE`] vs. `opendart`) distinguishes which source
//! produced it.
//!
//! # The browser is untrusted, even though it captured real bytes
//!
//! The capture stage is a browser automating a live page, not a documented
//! API client — so this module treats every field it supplies as untrusted
//! input, exactly like a hostile network response would be treated
//! elsewhere in this crate:
//!
//! - **No caller-supplied content hash is ever accepted.** [`CapturedPage`]
//!   has no hash field at all — [`RawEnvelope::new`] is the only place a
//!   hash is produced, and it always computes `sha256` from the exact bytes
//!   being stored (see [`domain::ContentHash::from_bytes`]). There is no
//!   code path by which a value the browser merely *reports* as a hash could
//!   reach `batch.json` or the manifest.
//! - **A captured credential must never reach the manifest.** KIND's
//!   documented form fields need none, so [`validate_form_fields`] rejects
//!   (rather than silently redacts) any field whose *name* merely looks
//!   credential-shaped, before any bytes are written anywhere.

use domain::{BatchId, TradingDate, UtcTimestamp};

use crate::contract::{
    FetchMode, PROVIDER_KIND_DISCLOSURE, PROVIDER_KIND_DISCLOSURE_CORRECTION, RawEnvelope,
    RequestMetadata, ResponseKind,
};
use crate::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};

/// Documented endpoint id for the KIND ETF disclosure-search capture surface
/// (`POST disclosurebystocktype.do`, `method=searchDisclosureByStockTypeEtfSub`).
pub const KIND_ETF_DISCLOSURE_ENDPOINT: &str = "kind.disclosure.etf.list.v1";
/// Exact browser entry URL approved for the ETF-scoped disclosure-list
/// capture. The browser stage repeats this literal in its staging metadata;
/// the Rust boundary remains the authoritative admission check.
pub const KIND_ETF_DISCLOSURE_ENTRY_URL: &str = "https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf";
/// The exact `method` value observed in the ETF disclosure-list request.
pub const KIND_ETF_DISCLOSURE_METHOD: &str = "searchDisclosureByStockTypeEtfSub";
/// The exact `forward` value observed in the ETF disclosure-list request.
pub const KIND_ETF_DISCLOSURE_FORWARD: &str = "disclosurebystocktype_etf_sub";
/// Documented endpoint id for the KIND 상세검색 (`details.do`) capture
/// surface, filtered to the ETF security type and disclosure-type filters.
pub const KIND_DETAIL_ETF_DISCLOSURE_ENDPOINT: &str = "kind.disclosure.detail.etf.v1";
/// The page's own `fnSearch()` request always sends `currentPageSize=15`.
pub const KIND_DISCLOSURE_PAGE_SIZE: u32 = 15;
/// Hard walk bound: a capture claiming more pages than this fails closed
/// rather than being trusted at face value, mirroring the OpenDART walk
/// bound (`DISCLOSURE_LIST_MAX_PAGES`) for the same reason — an unbounded
/// pagination claim can never be proven complete.
pub const KIND_DISCLOSURE_MAX_PAGES: usize = 40;

/// Exact browser entry URL for the one owner-approved correction/version
/// viewer capture surface. The browser stage must navigate to this URL and
/// the Rust boundary repeats the literal check before any Raw bytes are
/// written.
pub const KIND_CORRECTION_ENTRY_URL: &str = "https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf";
/// Stable endpoint id recorded in `RequestMetadata` for the correction
/// viewer. It is intentionally distinct from the Raw provider id.
pub const KIND_CORRECTION_VIEWER_ENDPOINT: &str = "kind.disclosure.correction.viewer.v1";
/// Staging `surface` identifier for the rendered correction/version viewer.
pub const KIND_CORRECTION_SURFACE: &str = "etf-disclosure-correction-viewer";
/// The only accepted rendered artifact for this surface.
pub const KIND_CORRECTION_ARTIFACT_KIND: &str = "rendered_dom_snapshot";
/// The only accepted viewer origin path (the query string is intentionally
/// opaque and is not persisted as an asserted semantic).
pub const KIND_CORRECTION_VIEWER_ORIGIN_PATH: &str = "/common/disclsviewer.do";
/// The only accepted completion value for a correction capture.
pub const KIND_CORRECTION_TERMINATION: &str = "viewer_loaded";
/// The only accepted completion stage for a correction capture.
pub const KIND_CORRECTION_TERMINATION_STAGE: &str = "viewer";
/// The only accepted file name in a complete correction staging directory.
pub const KIND_CORRECTION_VIEWER_FILE: &str = "viewer.html";
/// Maximum bytes admitted for a rendered correction viewer snapshot.
pub const MAX_KIND_CORRECTION_VIEWER_BYTES: u64 = 1024 * 1024;
/// Maximum bytes admitted for the exact ETF-list response used to resolve the
/// correction anchor. The response itself is not persisted by this surface;
/// only this bounded diagnostic size crosses the staging boundary.
pub const MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES: u64 = 1024 * 1024;
/// Maximum bytes admitted for correction `capture.json` metadata. This is a
/// staging guard only; the metadata is never stored as provider evidence.
pub const MAX_KIND_CORRECTION_METADATA_BYTES: u64 = 64 * 1024;
/// Maximum diagnostic counter accepted from untrusted capture metadata.
pub const MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT: u64 = 1_000_000;

/// Why the browser capture stage stopped its KIND page walk. Only a duplicate
/// response after advancing past the final page proves that the result set was
/// complete; every other value is an incomplete staging result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindCaptureTermination {
    /// KIND clamped a request beyond the final page and returned the prior
    /// page's exact bytes. This is the sole complete capture outcome.
    ClampedDuplicate,
    /// A distinct page arrived after the configured stored-page bound.
    PageBoundReached,
    /// Neither KIND's paging function nor its numeric page anchor was found.
    AdvanceControlMissing,
    /// A requested page did not arrive after the bounded retry.
    NoResponse,
}

impl KindCaptureTermination {
    /// The required string value written in the browser stage's
    /// `capture.json`.
    pub const fn staging_id(self) -> &'static str {
        match self {
            Self::ClampedDuplicate => "clamped_duplicate",
            Self::PageBoundReached => "page_bound_reached",
            Self::AdvanceControlMissing => "advance_control_missing",
            Self::NoResponse => "no_response",
        }
    }

    /// Parses an exact `capture.json` termination identifier.
    pub fn parse_staging_id(value: &str) -> Option<Self> {
        [
            Self::ClampedDuplicate,
            Self::PageBoundReached,
            Self::AdvanceControlMissing,
            Self::NoResponse,
        ]
        .into_iter()
        .find(|termination| termination.staging_id() == value)
    }

    /// Whether this termination proves a complete page walk.
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::ClampedDuplicate)
    }
}

/// Which KIND search surface a capture names. `DetailEtf` remains a known
/// discriminator for historical/normalization compatibility, but only the
/// ETF-scoped list is admitted to this Raw provider; see
/// [`KindSurface::ensure_raw_admitted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindSurface {
    /// The ETF-scoped disclosure-search list surface (`POST
    /// disclosurebystocktype.do`, `method=searchDisclosureByStockTypeEtfSub`).
    EtfList,
    /// The 상세검색 (`details.do`) surface, filtered to the ETF security type
    /// and disclosure-type filters.
    DetailEtf,
}

impl KindSurface {
    /// The value that identifies this surface in a staging `capture.json`'s
    /// `surface` field.
    pub const fn staging_id(self) -> &'static str {
        match self {
            Self::EtfList => "etf-disclosure-list",
            Self::DetailEtf => "etf-pit-disclosure-search",
        }
    }

    /// The endpoint id associated with this compatibility surface. Raw ingest
    /// calls [`KindSurface::ensure_raw_admitted`] before this is recorded, so
    /// the DetailEtf endpoint cannot enter a new Raw batch.
    pub const fn endpoint_id(self) -> &'static str {
        match self {
            Self::EtfList => KIND_ETF_DISCLOSURE_ENDPOINT,
            Self::DetailEtf => KIND_DETAIL_ETF_DISCLOSURE_ENDPOINT,
        }
    }

    /// Checks whether this compatibility surface is admitted to the current
    /// Raw provider. `DetailEtf` remains parseable for historical data and
    /// normalization compatibility, but it is deferred/not allowed for this
    /// capture-to-Raw path.
    pub fn ensure_raw_admitted(self) -> Result<(), KindError> {
        match self {
            Self::EtfList => Ok(()),
            Self::DetailEtf => Err(KindError::UnsupportedRawSurface { surface: self }),
        }
    }

    /// Parses a staging `capture.json`'s `surface` field into a
    /// [`KindSurface`], or `None` if it names neither known surface.
    pub fn parse_staging_id(s: &str) -> Option<Self> {
        [Self::EtfList, Self::DetailEtf]
            .into_iter()
            .find(|surface| surface.staging_id() == s)
    }
}

/// The `시간` (per-disclosure timestamp) column label every captured page's
/// decoded body must contain. This is the entire reason KIND is the chosen
/// source for ETF11 disclosure dates, so a body missing it is rejected
/// rather than stored — see [`KindError::MissingTimeColumn`].
const TIME_COLUMN_LABEL: &str = "시간";

/// Case-insensitive substrings that mark a form-field *name* as
/// credential-shaped. KIND's documented search form needs none of these, so
/// any match rejects the whole capture — see
/// [`KindError::CredentialLikeFormField`]. Listed individually (rather than
/// relying on one implying another): `password` does not contain `passwd`
/// as a contiguous substring, so both must be named explicitly.
const CREDENTIAL_FIELD_NAME_MARKERS: [&str; 5] = ["key", "token", "secret", "passwd", "password"];

/// One already-captured KIND response page, exactly as the browser-capture
/// stage retrieved it. Carries no hash: [`RawEnvelope::new`] always
/// recomputes `sha256` from `bytes` itself (see the module-level docs), so
/// there is no field here for a caller-supplied hash to occupy in the first
/// place.
pub struct CapturedPage {
    /// The requested `pageIndex`, 1-based. A full capture's page indices
    /// must start at 1 and increase by exactly one with no gaps or
    /// duplicates — see [`KindError::PageIndexOutOfSequence`].
    pub page_index: u32,
    /// The exact response bytes as captured, stored byte-for-byte.
    pub bytes: Vec<u8>,
    /// UTC instant this page was retrieved by the capture stage.
    pub retrieved_at: UtcTimestamp,
    /// The form fields the page itself sent to produce this response (e.g.
    /// `method`, `forward`, `currentPageSize`, `pageIndex`, `orderMode`,
    /// `orderStat`), as captured. Persisted verbatim into [`RequestMetadata::query`]
    /// once validated — see [`KindError::EmptyFormFields`] and
    /// [`KindError::CredentialLikeFormField`].
    pub form_fields: Vec<(String, String)>,
}

/// Typed failures from KIND ETF disclosure capture ingestion. Every variant
/// is a closed, structured shape.
#[derive(Debug)]
pub enum KindError {
    /// The surface is known for compatibility but is outside the owner-
    /// approved ETF-scoped Raw admission scope.
    UnsupportedRawSurface { surface: KindSurface },
    /// The caller-supplied entitlement reference was empty or
    /// whitespace-only. Required, not optional, before any capture is
    /// admitted as licensed evidence.
    MissingEntitlementReference,
    /// `pages` was empty: there is nothing to ingest.
    EmptyCapture,
    /// The browser capture did not observe the only terminal condition that
    /// proves completeness. This is rejected before any Raw-store work.
    IncompleteCapture { termination: KindCaptureTermination },
    /// `pages` claimed more pages than [`KIND_DISCLOSURE_MAX_PAGES`].
    TooManyPages { max_pages: usize, actual: usize },
    /// `page_index` values did not start at 1 and increase by exactly one:
    /// carries the page-position's expected value and the value actually
    /// found there. Covers a sequence that starts elsewhere, skips a
    /// number, or repeats one.
    PageIndexOutOfSequence { expected: u32, actual: u32 },
    /// A page's captured bytes were empty.
    EmptyPageBytes { page_index: u32 },
    /// A page's decoded body did not contain the documented `시간` column
    /// label — see [`TIME_COLUMN_LABEL`].
    MissingTimeColumn { page_index: u32 },
    /// Two different requested pages returned byte-identical responses,
    /// meaning pagination did not advance (mirrors the OpenDART walk's
    /// `DuplicatePageBytes`).
    DuplicatePageBytes { page_index: u32, duplicate_of: u32 },
    /// A page's `form_fields` was empty.
    EmptyFormFields { page_index: u32 },
    /// A page's `form_fields` contained a field whose *name* looks
    /// credential-shaped (see [`CREDENTIAL_FIELD_NAME_MARKERS`]). KIND needs
    /// no credential, so this is rejected outright rather than redacted —
    /// the surprise of a captured credential must stay visible, not be
    /// silently scrubbed. Only the field's name is carried here, never its
    /// value.
    CredentialLikeFormField { page_index: u32, field_name: String },
    /// A correction-viewer metadata field did not match the exact approved
    /// browser-capture contract. Only the field name crosses this boundary;
    /// untrusted values are deliberately not rendered in diagnostics.
    InvalidCorrectionMetadata { field: &'static str },
    /// A correction viewer diagnostic counter was zero or exceeded the
    /// bounded metadata contract.
    InvalidCorrectionDiagnostics { field: &'static str },
    /// The correction viewer was not strict UTF-8. Raw must fail closed
    /// rather than commit bytes the typed normalizer cannot inspect.
    CorrectionMalformedUtf8,
    /// The correction viewer body did not satisfy the exact `mainDoc`
    /// ordered-option contract.
    CorrectionViewerInvalid {
        reason: crate::kind_correction_normalize::KindCorrectionViewerError,
    },
    /// The immutable Raw store rejected this batch/manifest write.
    Store(StoreError),
}

impl std::fmt::Display for KindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedRawSurface { surface } => write!(
                f,
                "kind raw ingestion does not admit surface {}",
                surface.staging_id()
            ),
            Self::MissingEntitlementReference => write!(
                f,
                "kind disclosure capture ingest requires a non-empty entitlement reference"
            ),
            Self::EmptyCapture => write!(f, "kind disclosure capture had no pages"),
            Self::IncompleteCapture { termination } => write!(
                f,
                "kind disclosure capture ended as {}; only clamped_duplicate may be ingested",
                termination.staging_id()
            ),
            Self::TooManyPages { max_pages, actual } => write!(
                f,
                "kind disclosure capture claimed {actual} pages, exceeding the bound of {max_pages}"
            ),
            Self::PageIndexOutOfSequence { expected, actual } => write!(
                f,
                "kind disclosure capture page sequence broke: expected page_index {expected}, found {actual}"
            ),
            Self::EmptyPageBytes { page_index } => {
                write!(
                    f,
                    "kind disclosure capture page {page_index} had empty bytes"
                )
            }
            Self::MissingTimeColumn { page_index } => write!(
                f,
                "kind disclosure capture page {page_index} is missing the documented `시간` column"
            ),
            Self::DuplicatePageBytes {
                page_index,
                duplicate_of,
            } => write!(
                f,
                "kind disclosure capture page {page_index} returned bytes identical to page {duplicate_of}"
            ),
            Self::EmptyFormFields { page_index } => write!(
                f,
                "kind disclosure capture page {page_index} carried no form fields"
            ),
            Self::CredentialLikeFormField {
                page_index,
                field_name,
            } => write!(
                f,
                "kind disclosure capture page {page_index} carried a credential-like form field name {field_name:?}"
            ),
            Self::InvalidCorrectionMetadata { field } => write!(
                f,
                "kind correction capture metadata field {field} does not match the approved contract"
            ),
            Self::InvalidCorrectionDiagnostics { field } => write!(
                f,
                "kind correction capture diagnostic {field} is outside the bounded positive contract"
            ),
            Self::CorrectionMalformedUtf8 => {
                f.write_str("kind correction viewer bytes are not valid UTF-8")
            }
            Self::CorrectionViewerInvalid { reason } => {
                write!(f, "kind correction viewer failed validation: {reason}")
            }
            Self::Store(source) => write!(f, "kind disclosure raw store failure: {source}"),
        }
    }
}

impl std::error::Error for KindError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(source) => Some(source),
            Self::CorrectionViewerInvalid { reason } => Some(reason),
            _ => None,
        }
    }
}

/// Diagnostic counters recorded by the browser capture stage. `body_size` is
/// the exact ETF-list response size used to resolve the anchor, not the size
/// of `viewer.html`. The list response is not retained by this surface; the
/// rendered viewer is stored and bounded independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCorrectionResponseDiagnostics {
    pub body_size: u64,
    pub form_field_count: u64,
    pub target_handler_occurrences: u64,
}

/// One complete, already-captured KIND correction/version viewer.
///
/// This type intentionally contains no network client and no browser handle.
/// All fields are untrusted staging metadata and are validated again by
/// [`ingest_correction_capture`] immediately before the one atomic Raw-store
/// call. The viewer bytes are kept byte-for-byte; no lossy UTF-8 conversion is
/// permitted at this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindCorrectionCapture {
    pub source: String,
    pub entry_url: String,
    pub surface: String,
    pub requested_from: TradingDate,
    pub requested_to: TradingDate,
    pub anchor_acceptance_number: String,
    pub viewer_origin_path: String,
    pub artifact_kind: String,
    pub retrieved_at: UtcTimestamp,
    pub termination: String,
    pub termination_stage: String,
    pub response_diagnostics: KindCorrectionResponseDiagnostics,
    pub file_name: String,
    pub viewer_bytes: Vec<u8>,
}

/// Validates a required entitlement reference: an empty or whitespace-only
/// value fails closed before any page is examined. Returns the original
/// (untrimmed) value unchanged on success.
fn require_entitlement_reference(value: &str) -> Result<&str, KindError> {
    if value.trim().is_empty() {
        Err(KindError::MissingEntitlementReference)
    } else {
        Ok(value)
    }
}

/// Validates the capture's page-index sequence: non-empty, within
/// [`KIND_DISCLOSURE_MAX_PAGES`], and starting at 1 with no gaps or
/// duplicates.
fn validate_page_sequence(pages: &[CapturedPage]) -> Result<(), KindError> {
    if pages.is_empty() {
        return Err(KindError::EmptyCapture);
    }
    if pages.len() > KIND_DISCLOSURE_MAX_PAGES {
        return Err(KindError::TooManyPages {
            max_pages: KIND_DISCLOSURE_MAX_PAGES,
            actual: pages.len(),
        });
    }
    for (position, page) in pages.iter().enumerate() {
        let expected = position as u32 + 1;
        if page.page_index != expected {
            return Err(KindError::PageIndexOutOfSequence {
                expected,
                actual: page.page_index,
            });
        }
    }
    Ok(())
}

/// Validates one page's body: non-empty bytes, and a decoded body that
/// contains the documented `시간` column label. Decodes leniently
/// (`from_utf8_lossy`) for this check only — the bytes stored are always the
/// exact bytes supplied, never the decoded/lossy form.
fn validate_page_body(page: &CapturedPage) -> Result<(), KindError> {
    if page.bytes.is_empty() {
        return Err(KindError::EmptyPageBytes {
            page_index: page.page_index,
        });
    }
    let decoded = String::from_utf8_lossy(&page.bytes);
    if !decoded.contains(TIME_COLUMN_LABEL) {
        return Err(KindError::MissingTimeColumn {
            page_index: page.page_index,
        });
    }
    Ok(())
}

/// A form-field name is credential-shaped if it contains any of
/// [`CREDENTIAL_FIELD_NAME_MARKERS`], case-insensitively.
fn is_credential_like_field_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    CREDENTIAL_FIELD_NAME_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Validates one page's `form_fields`: non-empty, and no field name looks
/// credential-shaped.
fn validate_form_fields(page: &CapturedPage) -> Result<(), KindError> {
    if page.form_fields.is_empty() {
        return Err(KindError::EmptyFormFields {
            page_index: page.page_index,
        });
    }
    for (name, _) in &page.form_fields {
        if is_credential_like_field_name(name) {
            return Err(KindError::CredentialLikeFormField {
                page_index: page.page_index,
                field_name: name.clone(),
            });
        }
    }
    Ok(())
}

/// Raw-ingests one already-captured KIND disclosure-search date range from
/// one [`KindSurface`]: one batch, one file per page (`page-0001.html`,
/// `page-0002.html`, ...).
///
/// Validates every page before writing anything: page-index sequencing and
/// bound (see [`validate_page_sequence`]), then, per page, non-empty
/// `시간`-bearing bytes (see [`validate_page_body`]), non-empty
/// credential-free form fields (see [`validate_form_fields`]), and that its
/// bytes are not byte-identical to any earlier page in this same capture
/// (mirroring the OpenDART walk's duplicate-page guard — identical pages
/// mean the pagination did not advance). Only once every page has passed
/// does this call [`RawStore::store_batch`] once, atomically; any rejection
/// leaves nothing on disk.
///
/// `entitlement_reference` is required, not optional, and an empty or
/// whitespace-only value fails closed before any page is examined.
///
/// `surface` identifies which approved KIND search surface produced `pages`;
/// the closed [`KindSurface::ensure_raw_admitted`] gate prevents deferred
/// compatibility surfaces from reaching [`KindSurface::endpoint_id`] or Raw
/// storage.
// Keep the capture/Raw contract fields explicit instead of hiding them in a
// request struct: callers must visibly supply the completion termination that
// authorizes immutable Raw storage.
#[allow(clippy::too_many_arguments)]
pub fn ingest_disclosure_capture(
    store: &RawStore,
    market: &str,
    date: &TradingDate,
    entitlement_reference: &str,
    mode: FetchMode,
    surface: KindSurface,
    termination: KindCaptureTermination,
    pages: &[CapturedPage],
) -> Result<ManifestEntry, KindError> {
    // Keep this as the first gate: a known-but-deferred surface must not reach
    // any Raw validation or store preparation, even if another input is also
    // malformed.
    surface.ensure_raw_admitted()?;
    let entitlement_reference = require_entitlement_reference(entitlement_reference)?;
    if !termination.is_complete() {
        return Err(KindError::IncompleteCapture { termination });
    }
    validate_page_sequence(pages)?;

    let batch_id = BatchId::generate();
    let mut envelopes: Vec<(u32, RawEnvelope)> = Vec::with_capacity(pages.len());

    for page in pages {
        validate_page_body(page)?;
        validate_form_fields(page)?;

        for (seen_page_index, seen_envelope) in &envelopes {
            if seen_envelope.bytes == page.bytes {
                return Err(KindError::DuplicatePageBytes {
                    page_index: page.page_index,
                    duplicate_of: *seen_page_index,
                });
            }
        }

        let request = RequestMetadata {
            endpoint: surface.endpoint_id().to_owned(),
            query: page.form_fields.clone(),
            headers: Vec::new(),
            mode,
        };
        let file_name = format!("page-{:04}.html", page.page_index);
        // `RawEnvelope::new` computes the content hash itself, from exactly
        // these bytes — never from any value the caller might have supplied
        // (there is no such field on `CapturedPage` to begin with).
        let envelope = RawEnvelope::new(
            batch_id,
            ResponseKind::DisclosureIndex,
            file_name,
            page.bytes.clone(),
            page.retrieved_at,
            request,
        );
        envelopes.push((page.page_index, envelope));
    }

    let envelopes: Vec<RawEnvelope> = envelopes
        .into_iter()
        .map(|(_, envelope)| envelope)
        .collect();
    let spec = BatchSpec {
        provider: PROVIDER_KIND_DISCLOSURE,
        market,
        date,
        batch_id,
        entitlement_reference: Some(entitlement_reference),
        mode,
    };
    store
        .store_batch(&spec, &envelopes)
        .map_err(KindError::Store)
}

fn require_correction_ascii_digits(value: &str) -> bool {
    value.len() == 14 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_correction_capture_metadata(capture: &KindCorrectionCapture) -> Result<(), KindError> {
    if capture.source != "kind.krx.co.kr" {
        return Err(KindError::InvalidCorrectionMetadata { field: "source" });
    }
    if capture.entry_url != KIND_CORRECTION_ENTRY_URL {
        return Err(KindError::InvalidCorrectionMetadata { field: "entry_url" });
    }
    if capture.surface != KIND_CORRECTION_SURFACE {
        return Err(KindError::InvalidCorrectionMetadata { field: "surface" });
    }
    if capture.requested_from > capture.requested_to {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "requested_range",
        });
    }
    if !require_correction_ascii_digits(&capture.anchor_acceptance_number) {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "anchor_acceptance_number",
        });
    }
    if capture.viewer_origin_path != KIND_CORRECTION_VIEWER_ORIGIN_PATH {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "viewer_origin_path",
        });
    }
    if capture.artifact_kind != KIND_CORRECTION_ARTIFACT_KIND {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "artifact_kind",
        });
    }
    if capture.termination != KIND_CORRECTION_TERMINATION {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "termination",
        });
    }
    if capture.termination_stage != KIND_CORRECTION_TERMINATION_STAGE {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "termination_stage",
        });
    }
    if capture.file_name != KIND_CORRECTION_VIEWER_FILE {
        return Err(KindError::InvalidCorrectionMetadata { field: "file" });
    }
    let actual = capture.viewer_bytes.len() as u64;
    if actual == 0 || actual > MAX_KIND_CORRECTION_VIEWER_BYTES {
        return Err(KindError::InvalidCorrectionMetadata {
            field: "viewer.html",
        });
    }
    if capture.response_diagnostics.body_size == 0
        || capture.response_diagnostics.body_size > MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES
    {
        return Err(KindError::InvalidCorrectionDiagnostics { field: "body_size" });
    }
    for (field, value) in [
        (
            "form_field_count",
            capture.response_diagnostics.form_field_count,
        ),
        (
            "target_handler_occurrences",
            capture.response_diagnostics.target_handler_occurrences,
        ),
    ] {
        if value == 0 || value > MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT {
            return Err(KindError::InvalidCorrectionDiagnostics { field });
        }
    }
    if std::str::from_utf8(&capture.viewer_bytes).is_err() {
        return Err(KindError::CorrectionMalformedUtf8);
    }
    Ok(())
}

/// Raw-ingests one complete, operator-gated KIND correction/version viewer.
///
/// The function performs no network work. It validates every metadata field,
/// the bounded exact body, and the strict `mainDoc` option contract before a
/// single immutable Raw-store call. The output is deliberately a separate
/// provider/response-kind scope from the KIND disclosure list and is never
/// accepted by EOD, candidate, or publication paths.
#[allow(clippy::too_many_arguments)]
pub fn ingest_correction_capture(
    store: &RawStore,
    market: &str,
    date: &TradingDate,
    entitlement_reference: &str,
    mode: FetchMode,
    capture: &KindCorrectionCapture,
) -> Result<ManifestEntry, KindError> {
    let entitlement_reference = require_entitlement_reference(entitlement_reference)?;
    validate_correction_capture_metadata(capture)?;
    let parsed = crate::kind_correction_normalize::parse_kind_correction_viewer(
        &capture.viewer_bytes,
        &capture.anchor_acceptance_number,
    )
    .map_err(|reason| KindError::CorrectionViewerInvalid { reason })?;
    if parsed.is_empty() {
        return Err(KindError::CorrectionViewerInvalid {
            reason: crate::kind_correction_normalize::KindCorrectionViewerError::ZeroVersions,
        });
    }

    let batch_id = BatchId::generate();
    let query = vec![
        ("source".to_owned(), capture.source.clone()),
        ("entry_url".to_owned(), capture.entry_url.clone()),
        ("surface".to_owned(), capture.surface.clone()),
        (
            "requested_from".to_owned(),
            capture.requested_from.to_string(),
        ),
        ("requested_to".to_owned(), capture.requested_to.to_string()),
        (
            "anchor_acceptance_number".to_owned(),
            capture.anchor_acceptance_number.clone(),
        ),
        (
            "viewer_origin_path".to_owned(),
            capture.viewer_origin_path.clone(),
        ),
        ("artifact_kind".to_owned(), capture.artifact_kind.clone()),
        ("termination".to_owned(), capture.termination.clone()),
        (
            "termination_stage".to_owned(),
            capture.termination_stage.clone(),
        ),
        (
            "body_size".to_owned(),
            capture.response_diagnostics.body_size.to_string(),
        ),
        (
            "form_field_count".to_owned(),
            capture.response_diagnostics.form_field_count.to_string(),
        ),
        (
            "target_handler_occurrences".to_owned(),
            capture
                .response_diagnostics
                .target_handler_occurrences
                .to_string(),
        ),
    ];
    let envelope = RawEnvelope::new(
        batch_id,
        ResponseKind::DisclosureVersionMembership,
        KIND_CORRECTION_VIEWER_FILE,
        capture.viewer_bytes.clone(),
        capture.retrieved_at,
        RequestMetadata {
            endpoint: KIND_CORRECTION_VIEWER_ENDPOINT.to_owned(),
            query,
            headers: Vec::new(),
            mode,
        },
    );
    let spec = BatchSpec {
        provider: PROVIDER_KIND_DISCLOSURE_CORRECTION,
        market,
        date,
        batch_id,
        entitlement_reference: Some(entitlement_reference),
        mode,
    };
    store
        .store_batch(&spec, std::slice::from_ref(&envelope))
        .map_err(KindError::Store)
}
