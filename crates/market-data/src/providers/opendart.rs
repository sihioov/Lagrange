//! OpenDART disclosure provider — Raw-only, JSON-only, three surfaces.
//!
//! Covers exactly three documented `GET` surfaces on `opendart.fss.or.kr`:
//!
//! - `/api/list.json` → [`ResponseKind::DisclosureIndex`] (paginated search)
//! - `/api/corpCode.xml` → [`ResponseKind::DisclosureEntityMaster`] (a ZIP
//!   archive body — stored byte-for-byte, never unzipped or parsed here)
//! - `/api/company.json` → [`ResponseKind::DisclosureEntityProfile`]
//!   (single-page company overview)
//!
//! These three [`ResponseKind`]s are deliberately excluded from
//! [`crate::contract::EOD_RESPONSE_KINDS`], [`crate::contract::CANDIDATE_RESPONSE_KINDS`],
//! [`crate::contract::CANDIDATE_MASTER_RESPONSE_KINDS`], and
//! [`crate::contract::ALL_RESPONSE_KINDS`], and [`crate::validate::validate_response`]
//! deliberately rejects them. This module therefore performs its **own**
//! validation and never routes disclosure bytes through `validate_response`
//! or through `crate::ingest`; the Raw ingest entry points for these three
//! surfaces live here, physically separate from the EOD/candidate paths.
//!
//! Pipeline shape: [`OpenDartRead::get`] returns bytes -> this module's own
//! validation -> [`RawStore::store_batch`] (which, in one atomic call,
//! persists the immutable batch AND appends its manifest row — there is no
//! separate manifest-append step in the happy path) -> a typed
//! [`OpenDartOutcome`].
//!
//! # The API key is a query parameter
//!
//! Unlike KIS, which sends credentials in headers, OpenDART authenticates
//! with a **`crtfc_key` query parameter**. [`RequestMetadata::query`] is
//! persisted into `batch.json` and into the append-only manifest, so a naive
//! adapter would write the key to disk permanently inside an immutable
//! store. [`OpenDartProvider`] is the *only* place this module ever builds a
//! [`RequestMetadata`] value (see `redacted_metadata`), and that constructor
//! always redacts the key to a fixed placeholder before the metadata is
//! recorded — never optionally, never later. It also scans every caller
//! supplied query value against the configured key via
//! [`crate::redact::Redactor`] and fails closed if a leak is detected.

use std::future::Future;

use domain::{BatchId, TradingDate, UtcTimestamp};
use serde_json::Value;

use crate::contract::{FetchMode, PROVIDER_OPENDART, RawEnvelope, RequestMetadata, ResponseKind};
use crate::provider::CredentialRef;
use crate::redact::Redactor;
use crate::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};

/// The OpenDART query parameter carrying the credential. Its value must
/// never be persisted: `RequestMetadata` records [`REDACTED_KEY_PLACEHOLDER`]
/// in its place instead.
const CRTFC_KEY_PARAM: &str = "crtfc_key";
/// Fixed placeholder recorded in place of a live `crtfc_key` value.
const REDACTED_KEY_PLACEHOLDER: &str = "REDACTED";

/// Documented page-size maximum (range 1-100, default 10). This adapter
/// always requests the documented maximum.
pub const DISCLOSURE_LIST_PAGE_COUNT: u32 = 100;
/// Hard walk bound: exceeding this many pages fails closed rather than
/// looping forever (or truncating silently) against a misbehaving response.
pub const DISCLOSURE_LIST_MAX_PAGES: usize = 10;

/// Documented endpoint id for `GET /api/list.json`.
pub const OPENDART_DISCLOSURE_LIST_ENDPOINT: &str = "opendart.disclosure.list.v1";
/// Documented endpoint id for `GET /api/corpCode.xml`.
pub const OPENDART_ENTITY_CORPCODE_ENDPOINT: &str = "opendart.entity.corpcode.v1";
/// Documented endpoint id for `GET /api/company.json`.
pub const OPENDART_ENTITY_COMPANY_ENDPOINT: &str = "opendart.entity.company.v1";

const LIST_JSON_PATH: &str = "/api/list.json";
const CORP_CODE_XML_PATH: &str = "/api/corpCode.xml";
const COMPANY_JSON_PATH: &str = "/api/company.json";

/// ZIP local-file-header magic (`PK\x03\x04`) `corpCode.xml` must start with.
const ZIP_LOCAL_FILE_MAGIC: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];

/// OpenDART success status.
const STATUS_SUCCESS: &str = "000";
/// OpenDART documented "no data found" status (list.json only — see
/// [`ingest_disclosure_index`] for why this is a typed empty outcome only on
/// the first page of a walk).
const STATUS_NO_DATA: &str = "013";

/// Async read seam for OpenDART HTTP GETs. Implemented by fixtures in tests;
/// no implementation in this crate performs live network I/O (see
/// [`OpenDartLiveReader`]).
pub trait OpenDartRead: std::fmt::Debug + Send + Sync {
    /// Issues one GET against `path` with `query` (already including the
    /// live `crtfc_key`, added by [`OpenDartProvider`]) and returns the raw
    /// response bytes, unparsed.
    fn get(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> impl Future<Output = Result<Vec<u8>, OpenDartError>> + Send;
}

/// Placeholder credentialed OpenDART reader.
///
/// No live OpenDART HTTP client exists in this pass: no network I/O, no new
/// dependency. This type exists only so a future credentialed transport has
/// a fixed construction shape — it cannot be built without an explicit
/// [`CredentialRef`] — while every call fails closed with
/// [`OpenDartError::NotConfigured`] rather than attempting any I/O.
#[derive(Debug)]
pub struct OpenDartLiveReader {
    _credential: CredentialRef,
}

impl OpenDartLiveReader {
    /// Requires an explicit credential reference; there is no zero-argument
    /// or `Default` constructor.
    pub fn new(credential: CredentialRef) -> Self {
        Self {
            _credential: credential,
        }
    }
}

impl OpenDartRead for OpenDartLiveReader {
    async fn get(
        &self,
        _path: &str,
        _query: &[(String, String)],
    ) -> Result<Vec<u8>, OpenDartError> {
        Err(OpenDartError::NotConfigured)
    }
}

/// Typed failures from OpenDART disclosure ingestion. Every variant is a
/// closed, structured shape — never a verbatim copy of untrusted provider
/// bytes or a free-form provider message (see `sanitize_status`).
#[derive(Debug)]
pub enum OpenDartError {
    /// No live OpenDART client is configured (see [`OpenDartLiveReader`]).
    NotConfigured,
    /// Response bytes were not valid JSON where JSON was required.
    MalformedJson,
    /// Valid JSON, but it did not match any documented shape for this
    /// surface (wrong top-level type, a missing/mistyped documented field,
    /// an undocumented row shape, ...).
    UndocumentedShape,
    /// The documented `status` field was absent.
    MissingStatus,
    /// `status` carried an undocumented value. Bounded via `sanitize_status`
    /// so this can never smuggle a free-form provider message.
    UnexpectedStatus { status: String },
    /// A single-page surface's response carried a pagination-like marker
    /// (`page_no`, `page_count`, `total_count`, or `total_page`).
    UnexpectedPagination,
    /// `rcept_no` was not exactly 14 ASCII digits.
    InvalidRceptNo,
    /// `total_count`/`total_page` changed between pages of one walk — the
    /// result set shifted mid-walk and completeness cannot be proven.
    InconsistentPagination,
    /// A `013` (no-data) response arrived at a page after at least one page
    /// of this same walk already succeeded. Resolving this to a clean empty
    /// outcome would mask a result set that shifted mid-walk, so it fails
    /// closed instead (see [`ingest_disclosure_index`]).
    UnexpectedEmptyMidWalk { page_no: u32 },
    /// Two different requested pages returned byte-identical responses.
    DuplicatePageBytes { page_no: u32, duplicate_of: u32 },
    /// The walk did not reach a terminal page within
    /// [`DISCLOSURE_LIST_MAX_PAGES`].
    PaginationBoundExceeded { max_pages: usize },
    /// `corpCode.xml` body was neither ZIP-magic-prefixed nor a documented
    /// JSON error body.
    NotAZipArchive,
    /// The response body was empty where content was required.
    EmptyBody,
    /// A defensive redaction scan found the configured key value where only
    /// the redacted placeholder should ever appear.
    KeyLeakDetected,
    /// The immutable Raw store rejected this batch/manifest write.
    Store(StoreError),
}

impl std::fmt::Display for OpenDartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "no live OpenDART client is configured"),
            Self::MalformedJson => write!(f, "opendart response was not valid JSON"),
            Self::UndocumentedShape => {
                write!(f, "opendart response did not match any documented shape")
            }
            Self::MissingStatus => {
                write!(
                    f,
                    "opendart response is missing the documented `status` field"
                )
            }
            Self::UnexpectedStatus { status } => {
                write!(
                    f,
                    "opendart response carried undocumented status {status:?}"
                )
            }
            Self::UnexpectedPagination => write!(
                f,
                "single-page opendart surface carried a pagination-like marker"
            ),
            Self::InvalidRceptNo => {
                write!(f, "opendart rcept_no was not exactly 14 ASCII digits")
            }
            Self::InconsistentPagination => {
                write!(f, "opendart total_count/total_page changed mid-walk")
            }
            Self::UnexpectedEmptyMidWalk { page_no } => write!(
                f,
                "opendart returned no-data status at page {page_no} after prior pages already succeeded"
            ),
            Self::DuplicatePageBytes {
                page_no,
                duplicate_of,
            } => write!(
                f,
                "opendart page {page_no} returned bytes identical to page {duplicate_of}"
            ),
            Self::PaginationBoundExceeded { max_pages } => write!(
                f,
                "opendart list walk did not terminate within {max_pages} pages"
            ),
            Self::NotAZipArchive => {
                write!(f, "opendart corpCode.xml body was not a ZIP archive")
            }
            Self::EmptyBody => write!(f, "opendart response body was empty"),
            Self::KeyLeakDetected => {
                write!(f, "opendart request metadata failed the redaction scan")
            }
            Self::Store(source) => write!(f, "opendart raw store failure: {source}"),
        }
    }
}

impl std::error::Error for OpenDartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(source) => Some(source),
            _ => None,
        }
    }
}

/// Bounds a provider-controlled `status` value before it enters a typed
/// error. Only a short ASCII-alphanumeric status code is kept verbatim;
/// anything else (arbitrarily long, non-ASCII, or a free-form message
/// smuggled into the field) becomes a fixed marker instead. This is what
/// keeps [`OpenDartError::UnexpectedStatus`] a typed error rather than a
/// string dump of untrusted provider output.
fn sanitize_status(status: &str) -> String {
    const MAX_STATUS_LEN: usize = 8;
    if !status.is_empty()
        && status.len() <= MAX_STATUS_LEN
        && status.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        status.to_owned()
    } else {
        "UNDOCUMENTED_STATUS".to_owned()
    }
}

/// Outcome of one Raw disclosure ingest call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenDartOutcome {
    /// A batch was stored; carries the manifest row committed for it.
    Stored(ManifestEntry),
    /// The documented `status=013` no-data outcome. No batch is created.
    Empty,
}

/// Credentialed OpenDART disclosure adapter.
///
/// Holds the live `crtfc_key` value for exactly as long as it takes to build
/// outgoing requests. Every [`RequestMetadata`] this type builds is redacted
/// (see `redacted_metadata`) before it ever reaches [`RawEnvelope`],
/// `batch.json`, or the manifest.
pub struct OpenDartProvider<R: OpenDartRead> {
    reader: R,
    crtfc_key: String,
}

impl<R: OpenDartRead> std::fmt::Debug for OpenDartProvider<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenDartProvider")
            .field("reader", &self.reader)
            .field("crtfc_key", &"[REDACTED]")
            .finish()
    }
}

impl<R: OpenDartRead> OpenDartProvider<R> {
    /// `crtfc_key` is the live credential value. It is stored only to build
    /// outgoing requests and is never present in anything this type returns
    /// or stores.
    pub fn new(reader: R, crtfc_key: impl Into<String>) -> Self {
        Self {
            reader,
            crtfc_key: crtfc_key.into(),
        }
    }

    fn redactor(&self) -> Redactor {
        Redactor::new().with_secrets([self.crtfc_key.clone()])
    }

    /// The query actually sent over the wire: `visible` plus the live key.
    fn live_query(&self, visible: &[(String, String)]) -> Vec<(String, String)> {
        let mut query = visible.to_vec();
        query.push((CRTFC_KEY_PARAM.to_owned(), self.crtfc_key.clone()));
        query
    }

    /// The **only** place this module builds a [`RequestMetadata`]. The live
    /// key never enters `query`: a fixed placeholder is recorded instead.
    /// Defensively scans every caller-supplied `visible` value against the
    /// configured key via [`Redactor`] and fails closed if it finds a leak
    /// (for example, a caller accidentally passing the key itself as a
    /// non-secret filter value).
    fn redacted_metadata(
        &self,
        endpoint: &str,
        visible: &[(String, String)],
        mode: FetchMode,
    ) -> Result<RequestMetadata, OpenDartError> {
        let redactor = self.redactor();
        for (key, value) in visible {
            if !redactor.is_clean(key) || !redactor.is_clean(value) {
                return Err(OpenDartError::KeyLeakDetected);
            }
        }
        let mut query: Vec<(String, String)> = visible.to_vec();
        query.push((
            CRTFC_KEY_PARAM.to_owned(),
            REDACTED_KEY_PLACEHOLDER.to_owned(),
        ));
        Ok(RequestMetadata {
            endpoint: endpoint.to_owned(),
            query,
            headers: Vec::new(),
            mode,
        })
    }

    /// Raw-ingests `GET /api/list.json`, walking `page_no` from 1 while
    /// requesting [`DISCLOSURE_LIST_PAGE_COUNT`] rows per page.
    ///
    /// Terminal when `page_no >= total_page`, or immediately when
    /// `total_page` is `0`. The walk is bounded at
    /// [`DISCLOSURE_LIST_MAX_PAGES`]; exceeding it fails closed. All pages of
    /// one walk must report identical `total_count`/`total_page`, and no two
    /// pages may return identical bytes — either violation fails closed
    /// because the result set cannot be proven complete. Nothing is stored
    /// until the walk reaches a terminal page: the accumulated pages become
    /// one batch (one file per page) with one manifest row.
    ///
    /// `status=013` (documented no-data) is a typed empty outcome — but only
    /// on the walk's first page. A `013` arriving after pages already
    /// succeeded would silently mask a result set that shifted mid-walk, so
    /// that case fails closed instead (see
    /// [`OpenDartError::UnexpectedEmptyMidWalk`]).
    pub async fn ingest_disclosure_index(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let batch_id = BatchId::generate();
        let mut pages: Vec<(u32, RawEnvelope)> = Vec::new();
        let mut expected_total: Option<(u64, u32)> = None;

        for page_no in 1..=(DISCLOSURE_LIST_MAX_PAGES as u32) {
            let visible_query = vec![
                ("page_no".to_owned(), page_no.to_string()),
                (
                    "page_count".to_owned(),
                    DISCLOSURE_LIST_PAGE_COUNT.to_string(),
                ),
            ];
            let live_query = self.live_query(&visible_query);
            let bytes = self.reader.get(LIST_JSON_PATH, &live_query).await?;

            match parse_list_page(&bytes)? {
                ListPageOutcome::Empty => {
                    if pages.is_empty() {
                        return Ok(OpenDartOutcome::Empty);
                    }
                    return Err(OpenDartError::UnexpectedEmptyMidWalk { page_no });
                }
                ListPageOutcome::Page {
                    total_count,
                    total_page,
                } => {
                    match expected_total {
                        Some((count, page)) if count != total_count || page != total_page => {
                            return Err(OpenDartError::InconsistentPagination);
                        }
                        Some(_) => {}
                        None => expected_total = Some((total_count, total_page)),
                    }

                    for (seen_page_no, seen_envelope) in &pages {
                        if seen_envelope.bytes == bytes {
                            return Err(OpenDartError::DuplicatePageBytes {
                                page_no,
                                duplicate_of: *seen_page_no,
                            });
                        }
                    }

                    let request = self.redacted_metadata(
                        OPENDART_DISCLOSURE_LIST_ENDPOINT,
                        &visible_query,
                        mode,
                    )?;
                    let file_name = format!("list-page-{page_no:04}.json");
                    pages.push((
                        page_no,
                        RawEnvelope::new(
                            batch_id,
                            ResponseKind::DisclosureIndex,
                            file_name,
                            bytes,
                            retrieved_at,
                            request,
                        ),
                    ));

                    if total_page == 0 || page_no >= total_page {
                        let envelopes: Vec<RawEnvelope> =
                            pages.into_iter().map(|(_, envelope)| envelope).collect();
                        let spec = BatchSpec {
                            provider: PROVIDER_OPENDART,
                            market,
                            date,
                            batch_id,
                            entitlement_reference: None,
                            mode,
                        };
                        let entry = store
                            .store_batch(&spec, &envelopes)
                            .map_err(OpenDartError::Store)?;
                        return Ok(OpenDartOutcome::Stored(entry));
                    }
                }
            }
        }

        Err(OpenDartError::PaginationBoundExceeded {
            max_pages: DISCLOSURE_LIST_MAX_PAGES,
        })
    }

    /// Raw-ingests `GET /api/corpCode.xml`: single-page, no continuation
    /// parameter sent. The body must be non-empty and start with the ZIP
    /// local-file-header magic `PK\x03\x04`. The archive is **never**
    /// unzipped and its inner XML is **never** parsed — the bytes are stored
    /// exactly as received. If a JSON error body arrives instead (the
    /// documented error path), this fails closed with a typed error rather
    /// than storing it as if it were the archive.
    pub async fn ingest_entity_master(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let visible_query: Vec<(String, String)> = Vec::new();
        let live_query = self.live_query(&visible_query);
        let bytes = self.reader.get(CORP_CODE_XML_PATH, &live_query).await?;

        validate_zip_or_documented_error(&bytes)?;

        let request =
            self.redacted_metadata(OPENDART_ENTITY_CORPCODE_ENDPOINT, &visible_query, mode)?;
        let batch_id = BatchId::generate();
        let envelope = RawEnvelope::new(
            batch_id,
            ResponseKind::DisclosureEntityMaster,
            "corp-code.zip",
            bytes,
            retrieved_at,
            request,
        );
        let spec = BatchSpec {
            provider: PROVIDER_OPENDART,
            market,
            date,
            batch_id,
            entitlement_reference: None,
            mode,
        };
        let entry = store
            .store_batch(&spec, std::slice::from_ref(&envelope))
            .map_err(OpenDartError::Store)?;
        Ok(OpenDartOutcome::Stored(entry))
    }

    /// Raw-ingests `GET /api/company.json` for one `corp_code`: single-page,
    /// no continuation parameter sent. Rejects any pagination-like marker in
    /// the response and any documented status other than success.
    pub async fn ingest_entity_profile(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
        corp_code: &str,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let visible_query = vec![("corp_code".to_owned(), corp_code.to_owned())];
        let live_query = self.live_query(&visible_query);
        let bytes = self.reader.get(COMPANY_JSON_PATH, &live_query).await?;

        validate_single_page_json(&bytes)?;

        let request =
            self.redacted_metadata(OPENDART_ENTITY_COMPANY_ENDPOINT, &visible_query, mode)?;
        let batch_id = BatchId::generate();
        let envelope = RawEnvelope::new(
            batch_id,
            ResponseKind::DisclosureEntityProfile,
            "company.json",
            bytes,
            retrieved_at,
            request,
        );
        let spec = BatchSpec {
            provider: PROVIDER_OPENDART,
            market,
            date,
            batch_id,
            entitlement_reference: None,
            mode,
        };
        let entry = store
            .store_batch(&spec, std::slice::from_ref(&envelope))
            .map_err(OpenDartError::Store)?;
        Ok(OpenDartOutcome::Stored(entry))
    }
}

/// Result of validating one `list.json` page's body.
enum ListPageOutcome {
    /// `status=013`: documented no-data.
    Empty,
    /// `status=000`: a validated page, with its pagination totals.
    Page { total_count: u64, total_page: u32 },
}

/// Validates one `list.json` response body against the documented envelope:
/// `status`, `message`, `page_no`, `page_count`, `total_count`,
/// `total_page`, and a `list` array of rows with `corp_cls`, `corp_name`,
/// `corp_code`, `stock_code`, `report_nm`, `rcept_no`, `flr_nm`, `rcept_dt`,
/// `rm`. Never routes these bytes through `crate::validate::validate_response`
/// — disclosure kinds are deliberately excluded from that validator.
fn parse_list_page(bytes: &[u8]) -> Result<ListPageOutcome, OpenDartError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| OpenDartError::MalformedJson)?;
    let object = value.as_object().ok_or(OpenDartError::UndocumentedShape)?;

    let status = object
        .get("status")
        .ok_or(OpenDartError::MissingStatus)?
        .as_str()
        .ok_or(OpenDartError::UndocumentedShape)?;

    match status {
        STATUS_NO_DATA => Ok(ListPageOutcome::Empty),
        STATUS_SUCCESS => {
            let total_count = documented_envelope_u64(object, "total_count")?;
            let total_page: u32 = u32::try_from(documented_envelope_u64(object, "total_page")?)
                .map_err(|_| OpenDartError::UndocumentedShape)?;
            // Presence/type of `page_no` and `page_count` is validated even
            // though this adapter tracks the *requested* page itself; an
            // envelope missing them is an undocumented shape.
            documented_envelope_u64(object, "page_no")?;
            documented_envelope_u64(object, "page_count")?;

            let rows = object
                .get("list")
                .and_then(Value::as_array)
                .ok_or(OpenDartError::UndocumentedShape)?;
            for row in rows {
                validate_list_row(row)?;
            }

            Ok(ListPageOutcome::Page {
                total_count,
                total_page,
            })
        }
        other => Err(OpenDartError::UnexpectedStatus {
            status: sanitize_status(other),
        }),
    }
}

/// Reads a documented non-negative integer envelope field, accepting either a
/// JSON number or a digit-only JSON string.
///
/// The official `응답 결과` table names `page_no`, `page_count`, `total_count`,
/// and `total_page` but does not state their JSON type, while the matching
/// *request* parameters are documented as `STRING`. Insisting on one
/// representation would therefore be an inference, not a documented rule, and
/// would fail a well-formed response for a reason that protects nothing. Value
/// validation stays strict: anything that is not a well-formed non-negative
/// integer is still an undocumented shape.
fn documented_envelope_u64(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, OpenDartError> {
    let value = object.get(field).ok_or(OpenDartError::UndocumentedShape)?;
    match value {
        Value::Number(_) => value.as_u64().ok_or(OpenDartError::UndocumentedShape),
        Value::String(text) => {
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(OpenDartError::UndocumentedShape);
            }
            text.parse::<u64>()
                .map_err(|_| OpenDartError::UndocumentedShape)
        }
        _ => Err(OpenDartError::UndocumentedShape),
    }
}

/// Validates one documented `list.json` row shape and, for `rcept_no`,
/// opacity (see `validate_rcept_no`).
fn validate_list_row(row: &Value) -> Result<(), OpenDartError> {
    let object = row.as_object().ok_or(OpenDartError::UndocumentedShape)?;
    for field in [
        "corp_cls",
        "corp_name",
        "corp_code",
        "stock_code",
        "report_nm",
        "rcept_no",
        "flr_nm",
        "rcept_dt",
        "rm",
    ] {
        let value = object
            .get(field)
            .and_then(Value::as_str)
            .ok_or(OpenDartError::UndocumentedShape)?;
        if field == "rcept_no" {
            validate_rcept_no(value)?;
        }
    }
    Ok(())
}

/// `rcept_no` is validated **only** as an opaque 14-ASCII-digit token, and
/// must stay that way.
///
/// OpenDART's own documentation gives `rcept_no` only a viewer-link usage
/// example (`.../report.do?rcpNo=<rcept_no>`) and never states that its
/// leading digits encode the receipt date. Parsing, slicing, or otherwise
/// inferring a date from it would fabricate point-in-time evidence this
/// adapter has no license to assert. Do not "improve" this into a date
/// parser.
fn validate_rcept_no(value: &str) -> Result<(), OpenDartError> {
    if value.len() == 14 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(OpenDartError::InvalidRceptNo)
    }
}

/// Validates a single-page JSON surface (`company.json`): rejects any
/// pagination-like marker, requires the documented `status` field, and
/// accepts only the success status.
fn validate_single_page_json(bytes: &[u8]) -> Result<(), OpenDartError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| OpenDartError::MalformedJson)?;
    let object = value.as_object().ok_or(OpenDartError::UndocumentedShape)?;

    for marker in ["page_no", "page_count", "total_count", "total_page"] {
        if object.contains_key(marker) {
            return Err(OpenDartError::UnexpectedPagination);
        }
    }

    let status = object
        .get("status")
        .ok_or(OpenDartError::MissingStatus)?
        .as_str()
        .ok_or(OpenDartError::UndocumentedShape)?;
    match status {
        STATUS_SUCCESS => Ok(()),
        other => Err(OpenDartError::UnexpectedStatus {
            status: sanitize_status(other),
        }),
    }
}

/// Validates a `corpCode.xml` body: either it starts with the ZIP
/// local-file-header magic (accepted, stored byte-for-byte, never unzipped
/// or parsed), or it is the documented JSON error path (rejected with a
/// typed error rather than stored as if it were the archive).
fn validate_zip_or_documented_error(bytes: &[u8]) -> Result<(), OpenDartError> {
    if bytes.is_empty() {
        return Err(OpenDartError::EmptyBody);
    }
    if bytes.starts_with(&ZIP_LOCAL_FILE_MAGIC) {
        return Ok(());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => match object.get("status").and_then(Value::as_str) {
            Some(status) => Err(OpenDartError::UnexpectedStatus {
                status: sanitize_status(status),
            }),
            None => Err(OpenDartError::MissingStatus),
        },
        _ => Err(OpenDartError::NotAZipArchive),
    }
}
