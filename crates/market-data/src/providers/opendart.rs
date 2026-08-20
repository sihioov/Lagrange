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
//! # The API key is a query parameter — and never enters this crate
//!
//! Unlike KIS, which sends credentials in headers, OpenDART authenticates
//! with a **`crtfc_key` query parameter**. [`RequestMetadata::query`] is
//! persisted into `batch.json` and into the append-only manifest, so a naive
//! adapter would write the key to disk permanently inside an immutable
//! store. This module never holds or constructs a live `crtfc_key` value at
//! all: [`opendart_client::OpenDartClient`] reads the credential itself and
//! appends it to the outgoing query, *after* this module has finished
//! building the query it passes to [`OpenDartRead::get`]. [`OpenDartProvider`]
//! is the *only* place this module ever builds a [`RequestMetadata`] value
//! (see `redacted_metadata`), and that constructor always records a fixed
//! placeholder in place of the (absent) key — never optionally, never later
//! — because the live request the transport sends does carry the parameter
//! and the manifest should say so. `redacted_metadata` also fails closed
//! with a structural guard: any caller-supplied query pair literally named
//! `crtfc_key` is rejected, since only the transport is ever allowed to add
//! one.

use std::future::Future;

use domain::{BatchId, TradingDate, UtcTimestamp};
use serde_json::Value;

use crate::contract::{FetchMode, PROVIDER_OPENDART, RawEnvelope, RequestMetadata, ResponseKind};
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

/// Async read seam for OpenDART HTTP GETs. Implemented by fixtures in
/// tests, and for live traffic by [`opendart_client::OpenDartClient`]
/// (implemented in that crate, not here — see the impl below).
///
/// The `Debug` supertrait is retained: a credential-holding transport is
/// debug-printable *safely* — [`opendart_client::OpenDartClient`] implements
/// `Debug` by hand and renders its credential as a placeholder — so keeping
/// the bound costs nothing and preserves diagnosability of whatever reader a
/// caller supplies.
pub trait OpenDartRead: std::fmt::Debug + Send + Sync {
    /// Issues one GET against `path` with `query`. `query` carries **no**
    /// credential — the implementation is responsible for appending
    /// `crtfc_key` itself, after this trait's caller has already finished
    /// building and redacting `query`. Returns the raw response bytes,
    /// unparsed.
    fn get(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> impl Future<Output = Result<Vec<u8>, OpenDartError>> + Send;
}

/// Live OpenDART reader: the `opendart-client` transport, wired directly as
/// this module's [`OpenDartRead`]. `market-data` never holds or constructs a
/// `crtfc_key` value; [`opendart_client::OpenDartClient`] reads the
/// credential itself (see that crate's credential handling) and appends it
/// to the outgoing query only after this module has finished building and
/// redacting `query`.
///
/// [`opendart_client::OpenDartTransportError`] is deliberately coarse (a
/// failure class plus, at most, a numeric status) precisely so it is safe to
/// carry across a crate boundary; this impl maps it onto
/// [`OpenDartError::Transport`] verbatim rather than inventing detail.
impl OpenDartRead for opendart_client::OpenDartClient {
    async fn get(&self, path: &str, query: &[(String, String)]) -> Result<Vec<u8>, OpenDartError> {
        opendart_client::OpenDartClient::get(self, path, query)
            .await
            .map_err(OpenDartError::Transport)
    }
}

/// Typed failures from OpenDART disclosure ingestion. Every variant is a
/// closed, structured shape — never a verbatim copy of untrusted provider
/// bytes or a free-form provider message (see `sanitize_status`).
#[derive(Debug)]
pub enum OpenDartError {
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
    /// A `list.json` response identified a different page from the one the
    /// adapter requested, so page completeness cannot be proven.
    ResponsePageMismatch { requested: u32, response: u32 },
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
    /// `corpCode.xml` body was neither ZIP-magic-prefixed nor a recognisable
    /// documented error envelope. Both envelope encodings are recognised: the
    /// JSON form, and the XML form this surface actually returns in practice
    /// (observed 2026-08-20, with HTTP 200).
    NotAZipArchive,
    /// The response body was empty where content was required.
    EmptyBody,
    /// A caller-supplied query pair was itself named `crtfc_key`. Callers
    /// must never supply this parameter — only the live transport
    /// ([`opendart_client::OpenDartClient`]) is ever allowed to add it —
    /// so this is a structural rejection, not a value-content scan.
    KeyLeakDetected,
    /// The caller-supplied entitlement reference was empty or
    /// whitespace-only. This is required, not optional: Stage5's own
    /// production path requires an explicit entitlement reference before a
    /// delivery may be treated as licensed data, and a Raw batch whose
    /// data-use basis is unrecorded could never be admitted later. Fails
    /// closed before any request is issued.
    MissingEntitlementReference,
    /// A live OpenDART HTTP transport failure. Carries the coarse
    /// `opendart_client` error verbatim: that crate keeps its error type
    /// free of any URL, query string, or response body, so nothing more
    /// specific is safe to add here.
    Transport(opendart_client::OpenDartTransportError),
    /// The immutable Raw store rejected this batch/manifest write.
    Store(StoreError),
}

impl std::fmt::Display for OpenDartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
            Self::ResponsePageMismatch {
                requested,
                response,
            } => write!(
                f,
                "opendart response identified page {response}, but requested page was {requested}"
            ),
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
                write!(
                    f,
                    "caller-supplied query pair used the reserved `crtfc_key` parameter name"
                )
            }
            Self::MissingEntitlementReference => write!(
                f,
                "opendart ingest requires a non-empty entitlement reference"
            ),
            Self::Transport(source) => write!(f, "opendart transport failure: {source}"),
            Self::Store(source) => write!(f, "opendart raw store failure: {source}"),
        }
    }
}

impl std::error::Error for OpenDartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(source) => Some(source),
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
        UNDOCUMENTED_STATUS_MARKER.to_owned()
    }
}

/// Fixed marker [`sanitize_status`] substitutes for any status value that
/// fails its short-ASCII-alphanumeric bound. Named (rather than inlined at
/// every call site) so callers that need to distinguish "the provider sent
/// a documented-looking status" from "the provider sent something outside
/// that bound" can compare against it — see
/// [`validate_zip_or_documented_error`]'s XML branch, which reports the
/// latter as [`OpenDartError::UndocumentedShape`] instead of fabricating a
/// status.
const UNDOCUMENTED_STATUS_MARKER: &str = "UNDOCUMENTED_STATUS";

/// Validates a required entitlement reference: every ingest entry point in
/// this module must be given one, and an empty or whitespace-only value
/// fails closed here, before any request is issued. See
/// [`OpenDartError::MissingEntitlementReference`] for why this is required
/// rather than optional. Returns the original (untrimmed) value unchanged
/// on success — only its whitespace-only-ness is judged, not its shape.
fn require_entitlement_reference(value: &str) -> Result<&str, OpenDartError> {
    if value.trim().is_empty() {
        Err(OpenDartError::MissingEntitlementReference)
    } else {
        Ok(value)
    }
}

/// Optional documented filters for `GET /api/list.json`: the `corp_code`
/// (8-digit OpenDART entity code) and `bgn_de`/`end_de` (`YYYYMMDD`) search
/// window bounds. All three are optional per the documented envelope, and
/// are passed through as opaque query values, exactly like the
/// `page_no`/`page_count` this adapter always sends — this module performs
/// no additional shape validation on them beyond what already applies to
/// any query value (they never carry the reserved `crtfc_key` name).
#[derive(Debug, Clone, Copy, Default)]
pub struct DisclosureListFilter<'a> {
    pub corp_code: Option<&'a str>,
    pub bgn_de: Option<&'a str>,
    pub end_de: Option<&'a str>,
}

/// Outcome of one Raw disclosure ingest call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenDartOutcome {
    /// A batch was stored; carries the manifest row committed for it.
    Stored(ManifestEntry),
    /// The documented `status=013` no-data outcome. No batch is created.
    Empty,
}

/// OpenDART disclosure adapter.
///
/// Never holds, constructs, or receives a live `crtfc_key` value — that
/// credential lives entirely on the other side of [`OpenDartRead::get`],
/// inside [`opendart_client::OpenDartClient`]. Every [`RequestMetadata`]
/// this type builds is redacted (see `redacted_metadata`) before it ever
/// reaches [`RawEnvelope`], `batch.json`, or the manifest.
pub struct OpenDartProvider<R: OpenDartRead> {
    reader: R,
}

impl<R: OpenDartRead> std::fmt::Debug for OpenDartProvider<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately opaque even though `OpenDartRead` is `Debug`: a live
        // reader may safely redact its own credential, but this provider does
        // not need to render any reader details to remain diagnosable.
        f.debug_struct("OpenDartProvider").finish_non_exhaustive()
    }
}

impl<R: OpenDartRead> OpenDartProvider<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// The **only** place this module builds a [`RequestMetadata`]. No live
    /// `crtfc_key` value is ever available here to redact: a fixed
    /// placeholder is recorded in its place, because the live request the
    /// transport sends does carry the parameter and the manifest should say
    /// so. Fails closed with a structural guard if any caller-supplied
    /// `visible` pair is itself named `crtfc_key` — callers must never
    /// supply that parameter; only the transport is ever allowed to add it.
    fn redacted_metadata(
        &self,
        endpoint: &str,
        visible: &[(String, String)],
        mode: FetchMode,
    ) -> Result<RequestMetadata, OpenDartError> {
        if visible.iter().any(|(key, _)| key == CRTFC_KEY_PARAM) {
            return Err(OpenDartError::KeyLeakDetected);
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
    ///
    /// `entitlement_reference` is required, not optional: Stage5's own
    /// production path requires an explicit entitlement reference, and a
    /// Raw batch whose data-use basis is unrecorded cannot be admitted
    /// later. An empty or whitespace-only value fails closed with
    /// [`OpenDartError::MissingEntitlementReference`] before any request is
    /// issued.
    // Signature is dictated by the required scope parameters this task
    // adds (entitlement_reference) alongside the surface-specific search
    // filter (`filter`); splitting further would obscure call sites more
    // than the lint aids readability, matching this crate's existing
    // precedent (see e.g. `providers::kis::KisProvider::fetch_pages`).
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_disclosure_index(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
        filter: DisclosureListFilter<'_>,
        entitlement_reference: &str,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let entitlement_reference = require_entitlement_reference(entitlement_reference)?;
        let batch_id = BatchId::generate();
        let mut pages: Vec<(u32, RawEnvelope)> = Vec::new();
        let mut expected_total: Option<(u64, u32)> = None;

        for page_no in 1..=(DISCLOSURE_LIST_MAX_PAGES as u32) {
            let mut visible_query = vec![
                ("page_no".to_owned(), page_no.to_string()),
                (
                    "page_count".to_owned(),
                    DISCLOSURE_LIST_PAGE_COUNT.to_string(),
                ),
            ];
            if let Some(corp_code) = filter.corp_code {
                visible_query.push(("corp_code".to_owned(), corp_code.to_owned()));
            }
            if let Some(bgn_de) = filter.bgn_de {
                visible_query.push(("bgn_de".to_owned(), bgn_de.to_owned()));
            }
            if let Some(end_de) = filter.end_de {
                visible_query.push(("end_de".to_owned(), end_de.to_owned()));
            }
            let bytes = self.reader.get(LIST_JSON_PATH, &visible_query).await?;

            match parse_list_page(&bytes)? {
                ListPageOutcome::Empty => {
                    if pages.is_empty() {
                        return Ok(OpenDartOutcome::Empty);
                    }
                    return Err(OpenDartError::UnexpectedEmptyMidWalk { page_no });
                }
                ListPageOutcome::Page {
                    response_page_no,
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

                    if response_page_no != page_no {
                        return Err(OpenDartError::ResponsePageMismatch {
                            requested: page_no,
                            response: response_page_no,
                        });
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
                            entitlement_reference: Some(entitlement_reference),
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
    /// exactly as received. If a documented error envelope arrives instead,
    /// this fails closed with a typed error rather than storing it as if it
    /// were the archive. In practice this surface returns that envelope as
    /// **XML with HTTP 200** (observed 2026-08-20), so the status line cannot
    /// be relied on; the JSON form is recognised too. Only the status code
    /// crosses into the error — the envelope's `message` prose never does.
    ///
    /// `entitlement_reference` is required, not optional: see
    /// [`ingest_disclosure_index`](Self::ingest_disclosure_index) for the
    /// rationale. An empty or whitespace-only value fails closed with
    /// [`OpenDartError::MissingEntitlementReference`] before any request is
    /// issued.
    pub async fn ingest_entity_master(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
        entitlement_reference: &str,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let entitlement_reference = require_entitlement_reference(entitlement_reference)?;
        let visible_query: Vec<(String, String)> = Vec::new();
        let bytes = self.reader.get(CORP_CODE_XML_PATH, &visible_query).await?;

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
            entitlement_reference: Some(entitlement_reference),
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
    ///
    /// `entitlement_reference` is required, not optional: see
    /// [`ingest_disclosure_index`](Self::ingest_disclosure_index) for the
    /// rationale. An empty or whitespace-only value fails closed with
    /// [`OpenDartError::MissingEntitlementReference`] before any request is
    /// issued.
    // Signature is dictated by the required scope parameters this task
    // adds (entitlement_reference) alongside the surface-specific
    // `corp_code`; splitting further would obscure call sites more than the
    // lint aids readability, matching this crate's existing precedent (see
    // e.g. `providers::kis::KisProvider::fetch_pages`).
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_entity_profile(
        &self,
        store: &RawStore,
        market: &str,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
        corp_code: &str,
        entitlement_reference: &str,
    ) -> Result<OpenDartOutcome, OpenDartError> {
        let entitlement_reference = require_entitlement_reference(entitlement_reference)?;
        let visible_query = vec![("corp_code".to_owned(), corp_code.to_owned())];
        let bytes = self.reader.get(COMPANY_JSON_PATH, &visible_query).await?;

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
            entitlement_reference: Some(entitlement_reference),
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
    /// `status=000`: a validated page, with its response identity and
    /// pagination totals.
    Page {
        response_page_no: u32,
        total_count: u64,
        total_page: u32,
    },
}

/// Validates one `list.json` response body against the documented envelope and
/// retains its success-envelope page identity for the walk to compare against
/// the page it requested:
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
            let response_page_no: u32 = u32::try_from(documented_envelope_u64(object, "page_no")?)
                .map_err(|_| OpenDartError::UndocumentedShape)?;
            documented_envelope_u64(object, "page_count")?;

            let rows = object
                .get("list")
                .and_then(Value::as_array)
                .ok_or(OpenDartError::UndocumentedShape)?;
            for row in rows {
                validate_list_row(row)?;
            }

            Ok(ListPageOutcome::Page {
                response_page_no,
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
/// or parsed), or it is one of two documented error shapes, rejected with a
/// typed status error rather than stored as if it were the archive:
///
/// - JSON: `{"status": "...", "message": "..."}`.
/// - XML: observed 2026-08-20 against the live `.xml` surface, given a
///   deliberately invalid key — HTTP 200 with body
///   `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><result><status>010</status><message>...</message></result>`.
///   Status `010` documents an unregistered (or otherwise invalid) key.
///
/// The XML `<status>` value is pulled out with a minimal, targeted search —
/// find the `<status>` open tag, take the text up to the matching
/// `</status>`, trim it — never a general XML parse of the document, and
/// the sibling `<message>` element is never read: it is untrusted,
/// non-ASCII provider prose and must never reach an error, a log, or a
/// `Debug`/`Display` output.
///
/// Both shapes' extracted status is routed through [`sanitize_status`]
/// before use, exactly like every other OpenDART surface, so neither can
/// smuggle a free-form provider message. A status that fails that bound
/// (not short and ASCII-alphanumeric) is reported as
/// [`OpenDartError::UndocumentedShape`] — the envelope was recognised, but
/// its status content was not — rather than fabricating a status value.
///
/// Precedence: ZIP magic wins; then a recognisable status envelope (JSON or
/// XML) yields the status error; a body that is neither is
/// [`OpenDartError::NotAZipArchive`].
fn validate_zip_or_documented_error(bytes: &[u8]) -> Result<(), OpenDartError> {
    if bytes.is_empty() {
        return Err(OpenDartError::EmptyBody);
    }
    if bytes.starts_with(&ZIP_LOCAL_FILE_MAGIC) {
        return Ok(());
    }
    if let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(bytes) {
        return match object.get("status").and_then(Value::as_str) {
            Some(status) => Err(OpenDartError::UnexpectedStatus {
                status: sanitize_status(status),
            }),
            None => Err(OpenDartError::MissingStatus),
        };
    }
    if let Some(status) = extract_xml_status(bytes) {
        let sanitized = sanitize_status(&status);
        return if sanitized == UNDOCUMENTED_STATUS_MARKER {
            Err(OpenDartError::UndocumentedShape)
        } else {
            Err(OpenDartError::UnexpectedStatus { status: sanitized })
        };
    }
    Err(OpenDartError::NotAZipArchive)
}

/// Documented open/close tags for the single `<status>` element in the
/// `corpCode.xml` surface's XML error envelope (see
/// [`validate_zip_or_documented_error`]).
const XML_STATUS_OPEN_TAG: &str = "<status>";
const XML_STATUS_CLOSE_TAG: &str = "</status>";

/// Extracts the text of a documented `<status>` element from a
/// `corpCode.xml` XML error envelope, without a general XML parser: finds
/// the first `<status>` open tag, takes the bytes up to the first following
/// `</status>`, and trims the result. Deliberately does not touch the
/// sibling `<message>` element at all — only `<status>` is ever read.
///
/// Returns `None` when no matching `<status>...</status>` pair is present,
/// meaning this body is not a recognisable XML status envelope at all (as
/// opposed to one whose status content fails the bound applied by
/// [`sanitize_status`], which is a distinct, later outcome).
fn extract_xml_status(bytes: &[u8]) -> Option<String> {
    let open_at = find_subslice(bytes, XML_STATUS_OPEN_TAG.as_bytes())?;
    let after_open = open_at + XML_STATUS_OPEN_TAG.len();
    let close_at = find_subslice(&bytes[after_open..], XML_STATUS_CLOSE_TAG.as_bytes())?;
    let status_bytes = &bytes[after_open..after_open + close_at];
    Some(String::from_utf8_lossy(status_bytes).trim().to_owned())
}

/// First index at which `needle` occurs in `haystack`, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that must never actually be called: these tests exercise
    /// `redacted_metadata` directly, before any `OpenDartRead::get` call
    /// would happen.
    #[derive(Debug)]
    struct UnreachableReader;

    impl OpenDartRead for UnreachableReader {
        async fn get(
            &self,
            _path: &str,
            _query: &[(String, String)],
        ) -> Result<Vec<u8>, OpenDartError> {
            panic!("UnreachableReader::get must never be called by these tests");
        }
    }

    /// `redacted_metadata`'s structural guard: a caller-supplied query pair
    /// literally named `crtfc_key` is rejected before it can ever reach a
    /// [`RequestMetadata`].
    ///
    /// No public `ingest_*` entry point on [`OpenDartProvider`] can actually
    /// construct such a pair -- every `visible` query this module builds
    /// uses a hardcoded parameter name (`page_no`, `page_count`,
    /// `corp_code`) -- so this guard is unreachable from the public API by
    /// construction. That unreachability is itself the strongest form of
    /// "no `crtfc_key` path exists in `market-data`"; this unit test reaches
    /// past the public API to prove the guard still fires on its own terms,
    /// and would catch a regression if some future `ingest_*` method ever
    /// let a caller choose a query pair's name.
    #[test]
    fn redacted_metadata_rejects_a_caller_supplied_crtfc_key_pair() {
        let provider = OpenDartProvider::new(UnreachableReader);
        let visible = vec![(CRTFC_KEY_PARAM.to_owned(), "irrelevant".to_owned())];

        let result = provider.redacted_metadata("test.endpoint", &visible, FetchMode::Synthetic);

        assert!(matches!(result, Err(OpenDartError::KeyLeakDetected)));
    }

    /// The guard checks the pair's *name*, not its value: this module holds
    /// no configured key for a value to collide with, so a value that
    /// merely looks like a key is not a leak. Only the reserved parameter
    /// name `crtfc_key` triggers the guard.
    #[test]
    fn redacted_metadata_allows_a_query_value_that_merely_resembles_a_key() {
        let provider = OpenDartProvider::new(UnreachableReader);
        let visible = vec![("corp_code".to_owned(), CRTFC_KEY_PARAM.to_owned())];

        let result = provider.redacted_metadata("test.endpoint", &visible, FetchMode::Synthetic);

        assert!(result.is_ok());
    }
}
