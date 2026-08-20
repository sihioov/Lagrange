//! KIND ETF disclosure-search **parsing** half of Stage6 step 3: turns a
//! stored `provider=kind-disclosure` Raw batch of HTML search-result pages
//! into typed [`KindDisclosureObservation`]s, and writes a second immutable
//! `provider=kind-disclosure-normalized` batch containing them. The wire
//! batch (the exact HTML bytes the browser-capture stage retrieved) is never
//! rewritten or deleted — see [`crate::providers::kind`].
//!
//! # The header contract lives in `summary`, not `<th>`
//!
//! A stored KIND disclosure page's `<thead>` carries exactly one `<tr>`, and
//! it is **empty** (`<tr id="title-contents"></tr>`) — the column titles a
//! *rendered* page shows are injected into it client-side by
//! `fn_InitTitle("번호,시간,종목명,공시제목,제출인", ...)`. An earlier pass
//! validated the header by reading `<th>` cells because that is what a
//! rendered page's DOM shows; against real stored bytes that fails with
//! zero `<th>` cells found, because the DOM injection never happened to the
//! artifact — only to a browser that later ran the script.
//!
//! This module never parses `fn_InitTitle`'s call: one contract source is
//! enough, and an HTML attribute is a better one than a JS call argument.
//! Instead, the same five column labels are present on the `<table>`
//! element's own `summary` attribute (e.g. `summary="번호, 시간, 종목명,
//! 공시제목,제출인"` — real captured spacing is already inconsistent around
//! the commas), which *is* present in the stored bytes — see
//! [`validate_table_summary`]. A missing `summary`, or any other column
//! list, is [`KindNormalizeError::UnsupportedHeader`].
//!
//! # The timestamp is honest about what it assumes
//!
//! Every KIND disclosure row carries a `시간` cell: a local calendar date and
//! wall-clock time to the minute (`YYYY-MM-DD HH:MM`). **KIND documents no
//! timezone anywhere for this value** — that was checked, not assumed away.
//! So every observation carries two separate things rather than one
//! conflated value:
//!
//! - [`KindDisclosureObservation::posted_local`] (plus
//!   [`KindDisclosureObservation::posted_local_raw`]): the literal local
//!   date/time, exactly as printed, with **no** timezone attached. This is
//!   the source-of-record value and is never silently treated as UTC.
//! - [`KindDisclosureObservation::posted_at_instant`]: the UTC instant that
//!   local time denotes **under an explicit, recorded assumption** —
//!   currently [`TimezoneAssumption::AssumedAsiaSeoul`] — carried alongside
//!   it in [`KindDisclosureObservation::timezone_assumption`]. A later,
//!   confirmed timezone can add a new [`TimezoneAssumption`] variant and
//!   re-derive `posted_at_instant` from the untouched `posted_local` without
//!   rewriting history, and any reader can see at a glance that the instant
//!   rests on an assumption rather than documentation.
//!
//! No code path in this module ever produces an instant without an attached
//! [`TimezoneAssumption`] — [`KindDisclosureObservation`] simply has no
//! constructor that would allow it.
//!
//! # Two identifiers the stored bytes carry, each validated and kept
//!
//! Every row's title cell embeds an `openDisclsViewer('<number>','')`
//! `onclick` handler and its name cell embeds an
//! `etfisusummary_open('<number>')` one. Both are real per-row identifiers
//! present in the bytes, and both are extracted and validated before
//! anything is written — a row missing either, or carrying a malformed one,
//! fails the whole batch with its own distinct [`KindNormalizeError`]
//! variant, and nothing is written if any row fails either check:
//!
//! - [`KindDisclosureObservation::disclosure_acceptance_number`]: KIND's own
//!   disclosure acceptance number, validated as exactly 14 ASCII digits (see
//!   [`KindNormalizeError::InvalidDisclosureAcceptanceNumber`]). This is the
//!   disclosure's own stable id and the join key to its correction chain —
//!   but, exactly like this codebase already treats OpenDART's `rcept_no`
//!   (see `crate::providers::opendart`'s `validate_rcept_no`), KIND
//!   documents no structure for it anywhere, so **no date is ever parsed or
//!   inferred from it** here, even though its leading digits happen to
//!   resemble one in every observed sample (e.g. `"20200207000058"`). Doing
//!   so would fabricate point-in-time evidence this normalizer has no
//!   license to assert.
//! - [`KindDisclosureObservation::kind_internal_issue_key`]: KIND's own
//!   internal issue key (it scopes KIND's own `etfisusummary_open()` popup),
//!   validated as 1-12 ASCII digits with leading zeros preserved verbatim
//!   (see [`KindNormalizeError::InvalidKindInternalIssueKey`]). **This is
//!   not a KRX six-digit short code and must never be treated as an
//!   instrument identifier** — see "Instrument identity is out of scope"
//!   below.
//!
//! # Instrument identity is out of scope, on purpose
//!
//! KIND's `종목명` column is an issue *name*, and the only other per-issue
//! identifier the stored bytes carry is
//! [`KindDisclosureObservation::kind_internal_issue_key`]. **Neither is a
//! six-digit KRX instrument code, and a real captured page contains no such
//! code anywhere** — that was checked against the stored bytes, not assumed
//! away. This repository also has no authoritative ETF11 name-to-code
//! mapping: `seed_universe`'s names are explicit synthetic placeholders
//! (`SYNTHETIC-KODEX200`, ...) and `configs/universes/kr-etf-core-v1.yaml`
//! states real names are resolved from KRX at build time — KRX is a
//! deferred source in this repository. So even with both identifiers in
//! hand, this module cannot resolve an instrument id, and mapping, matching,
//! or guessing one would be exactly the unsourced inference this project
//! forbids: [`KindDisclosureObservation::issue_name`] and
//! [`KindDisclosureObservation::kind_internal_issue_key`] are both carried
//! literally (trimmed only, never mapped or fuzzy-matched), and
//! [`KindDisclosureObservation::instrument_identity`] is always
//! [`InstrumentIdentity::Unresolved`] — a visible marker, not merely an
//! absent field, so nothing downstream can mistake "not resolved yet" for
//! "resolved to nothing".
//!
//! # Fail-closed parsing
//!
//! Every page's `summary` header contract, every row's cell count, every
//! `시간` value, every `번호` value, the `번호` sequence across the whole
//! batch, and each row's two identifiers are validated before anything is
//! written. A single bad row fails the whole batch — see
//! [`KindNormalizeError`] — rather than silently under-reporting disclosure
//! evidence by skipping it.

use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use domain::{BatchId, ContentHash, DomainError, UtcTimestamp, Venue, VenueTimestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contract::{
    PROVIDER_KIND_DISCLOSURE, PROVIDER_KIND_DISCLOSURE_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind,
};
use crate::storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};

/// This normalizer's stable identity, folded into the deterministic batch-id
/// UUIDv5 name so a future parsing-rule change cannot silently reuse bytes
/// produced by this version.
const NORMALIZER: &str = "kind-disclosure-html-to-observations-v1";
const NORMALIZER_SCHEMA_VERSION: u32 = 1;
/// The exact five column labels the ETF-scoped KIND search surface's result
/// table's `summary` attribute must list, in order, once each comma-
/// separated part is trimmed. Any other list (e.g. from the
/// `차트/주가`-bearing surface) is an unsupported layout — see
/// [`KindNormalizeError::UnsupportedHeader`] and [`validate_table_summary`].
const EXPECTED_SUMMARY_COLUMNS: [&str; 5] = ["번호", "시간", "종목명", "공시제목", "제출인"];
/// The number of `<td>` cells every data row must carry — one per
/// [`EXPECTED_SUMMARY_COLUMNS`] entry.
const EXPECTED_COLUMN_COUNT: usize = EXPECTED_SUMMARY_COLUMNS.len();
/// The single file name every normalized batch stores its observations
/// under.
const OBSERVATIONS_FILE_NAME: &str = "observations.json";
const COLLISION_RETRIES: usize = 100;
const COLLISION_RETRY_DELAY: Duration = Duration::from_millis(2);

/// Which required column was empty. See
/// [`KindNormalizeError::EmptyRequiredField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredField {
    /// `종목명`.
    IssueName,
    /// `공시제목`.
    DisclosureTitle,
}

impl std::fmt::Display for RequiredField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::IssueName => "종목명",
            Self::DisclosureTitle => "공시제목",
        })
    }
}

/// One page/row location, used only to point a fail-closed error at the
/// exact cell that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowLocation {
    /// The source page's stored file name (e.g. `page-0002.html`).
    pub file_name: String,
    /// 0-based index of this row among that page's *data* rows (the
    /// `<thead>` placeholder row is never counted).
    pub row_index: usize,
}

impl std::fmt::Display for RowLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}[row {}]", self.file_name, self.row_index)
    }
}

/// Why a KIND disclosure Raw batch could not be parsed into observations.
/// Every variant is a distinct, fail-closed reason — nothing is written to
/// the normalized provider scope unless parsing succeeds completely.
#[derive(Debug, thiserror::Error)]
pub enum KindNormalizeError {
    #[error("kind normalization supports only provider {expected}, got {actual}")]
    UnsupportedScope {
        expected: &'static str,
        actual: String,
    },
    #[error(
        "kind disclosure page {file_name} has no <table> to parse (unsupported or empty layout)"
    )]
    MissingTable { file_name: String },
    #[error(
        "kind disclosure page {file_name} table `summary` attribute, once split on ',' and each \
         part trimmed, is not exactly {expected:?}, got {actual:?}"
    )]
    UnsupportedHeader {
        file_name: String,
        expected: &'static [&'static str],
        actual: Vec<String>,
    },
    #[error(
        "kind disclosure page {file_name} is missing its required empty <thead> placeholder row or that row contains cells"
    )]
    InvalidPlaceholderRow { file_name: String },
    #[error("kind disclosure row {location} has {actual} cells, expected {expected}")]
    RowCellCountMismatch {
        location: RowLocation,
        expected: usize,
        actual: usize,
    },
    #[error(
        "kind disclosure row {location} has an unparseable 시간 value {value:?}, expected YYYY-MM-DD HH:MM"
    )]
    InvalidTimestamp {
        location: RowLocation,
        value: String,
    },
    #[error("kind disclosure row {location} has an empty {field}")]
    EmptyRequiredField {
        location: RowLocation,
        field: RequiredField,
    },
    #[error("kind disclosure row {location} has a 번호 that is not a positive integer: {value:?}")]
    InvalidSequenceNumber {
        location: RowLocation,
        value: String,
    },
    /// The name cell's `etfisusummary_open(...)` `onclick` handler was
    /// absent (`value: None`) or present but not exactly 1-12 ASCII digits
    /// (`value: Some(...)`, carrying whatever was actually captured).
    #[error(
        "kind disclosure row {location} has a missing or malformed KIND-internal issue key (etfisusummary_open argument) in the 종목명 cell: {value:?}, expected 1-12 ASCII digits"
    )]
    InvalidKindInternalIssueKey {
        location: RowLocation,
        value: Option<String>,
    },
    /// The title cell's `openDisclsViewer(...)` `onclick` handler was absent
    /// (`value: None`) or present but not exactly 14 ASCII digits (`value:
    /// Some(...)`, carrying whatever was actually captured).
    #[error(
        "kind disclosure row {location} has a missing or malformed disclosure acceptance number (openDisclsViewer argument) in the 공시제목 cell: {value:?}, expected exactly 14 ASCII digits"
    )]
    InvalidDisclosureAcceptanceNumber {
        location: RowLocation,
        value: Option<String>,
    },
    #[error(
        "kind disclosure 번호 sequence broke between {previous_location} (번호 {previous_value}) and {actual_location} (번호 {actual_value}); expected {expected_value}"
    )]
    SequenceNumberOutOfOrder {
        previous_location: RowLocation,
        previous_value: u64,
        actual_location: RowLocation,
        actual_value: u64,
        expected_value: u64,
    },
    #[error("kind disclosure batch has zero rows across every page")]
    EmptyBatch,
    #[error(
        "kind disclosure row {location} local time {local} cannot be interpreted under the {assumption:?} assumption: {source}"
    )]
    LocalTimeAssumptionFailed {
        location: RowLocation,
        local: NaiveDateTime,
        assumption: TimezoneAssumption,
        #[source]
        source: DomainError,
    },
    #[error("kind disclosure observation serialization failed: {reason}")]
    Serialization { reason: String },
    #[error("existing deterministic normalized batch {batch_id} conflicts: {reason}")]
    ExistingBatchConflict { batch_id: BatchId, reason: String },
    #[error("source Raw read failed: {0}")]
    Store(#[from] StoreError),
}

/// The named assumption a [`KindDisclosureObservation::posted_at_instant`]
/// rests on. KIND documents no timezone for its `시간` column anywhere, so
/// this is recorded explicitly rather than silently baked into the instant.
/// A confirmed timezone gets a *new* variant here, never a silent rewrite of
/// this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimezoneAssumption {
    /// `Asia/Seoul` (KRX's own timezone) was assumed because KIND is
    /// KRX-operated and no other zone is plausible for a KRX disclosure
    /// site — but this has never been confirmed against KIND's own
    /// documentation, because KIND documents no timezone at all.
    AssumedAsiaSeoul,
}

/// Whether an observation's instrument identity has been resolved to a
/// durable instrument id. Always [`Unresolved`](Self::Unresolved) today —
/// see the module-level docs for why resolving it is blocked, not merely
/// deferred by oversight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum InstrumentIdentity {
    /// No authoritative KIND `종목명`/`kind_internal_issue_key` ->
    /// instrument-id mapping exists in this repository (KRX is the only
    /// candidate source and is a deferred decision — see
    /// `configs/universes/kr-etf-core-v1.yaml`). Verified against the real
    /// captured bytes, not merely assumed: **no six-digit KRX code appears
    /// anywhere in a stored KIND disclosure page** — the only per-issue
    /// identifier the artifact itself carries is
    /// [`KindDisclosureObservation::kind_internal_issue_key`], which is a
    /// KIND-internal key, not a KRX code. So even KIND's own artifact
    /// cannot resolve the instrument — this is stronger than "this
    /// repository hasn't built the mapping yet". `reason` is carried on the
    /// value itself (not just in comments) so a downstream consumer that
    /// only looks at data, never source, still sees why.
    Unresolved { reason: String },
}

/// Why [`InstrumentIdentity::Unresolved`] is emitted, carried on the value
/// itself (see [`InstrumentIdentity::Unresolved`]'s docs).
const UNRESOLVED_INSTRUMENT_IDENTITY_REASON: &str = "no authoritative KIND 종목명/kind_internal_issue_key-to-instrument-id mapping exists in \
     this repository; KRX is the only candidate source and is a deferred decision; moreover the \
     stored KIND bytes themselves contain no six-digit KRX code anywhere — only the \
     KIND-internal issue key — so even KIND's own artifact cannot resolve the instrument";

impl InstrumentIdentity {
    fn unresolved() -> Self {
        Self::Unresolved {
            reason: UNRESOLVED_INSTRUMENT_IDENTITY_REASON.to_owned(),
        }
    }
}

/// One normalized KIND disclosure-search row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindDisclosureObservation {
    /// The `번호` running sequence number, exactly as printed. Descends by
    /// exactly 1 across the whole batch — enforced before anything is
    /// written, see [`KindNormalizeError::SequenceNumberOutOfOrder`].
    pub sequence_number: u64,
    /// The `종목명` cell's display text (see [`cell_display_text`]): the
    /// wrapping anchor's `title` attribute where present, else its inner
    /// text, either way trimmed. Never mapped to an instrument id — see
    /// [`Self::instrument_identity`] and the module-level docs.
    pub issue_name: String,
    /// KIND's own internal issue key, extracted from the `종목명` cell's
    /// `etfisusummary_open('<key>')` `onclick` handler and validated as
    /// 1-12 ASCII digits — leading zeros are preserved verbatim (e.g.
    /// `"06966"`) because this is an opaque key, never parsed as an
    /// integer.
    ///
    /// **This is a KIND-internal identifier, not a KRX six-digit short
    /// code, and must never be treated as an instrument identifier.** KIND
    /// scopes its own summary popup by this key; it carries no documented
    /// relationship to any KRX instrument code, and this repository has no
    /// authoritative mapping from it to one — see [`Self::instrument_identity`]
    /// and the module-level docs on why the stored bytes cannot resolve the
    /// instrument even with this key in hand.
    pub kind_internal_issue_key: String,
    /// Always [`InstrumentIdentity::Unresolved`] today. Present as an
    /// explicit field (not merely an absent one) so "not resolved yet" can
    /// never be mistaken for "resolved to nothing".
    pub instrument_identity: InstrumentIdentity,
    /// The `공시제목` cell's display text (see [`cell_display_text`]).
    pub disclosure_title: String,
    /// KIND's own disclosure acceptance number, extracted from the
    /// `공시제목` cell's `openDisclsViewer('<no>','')` `onclick` handler and
    /// validated as exactly 14 ASCII digits. This is the disclosure's own
    /// stable id and the join key to its correction chain.
    ///
    /// **Never parse or infer a date from this value**, even though its
    /// leading digits happen to resemble one in every observed sample (e.g.
    /// `"20200207000058"`). KIND documents no structure for this number
    /// anywhere — exactly the same rule this codebase already applies to
    /// OpenDART's `rcept_no` (see `crate::providers::opendart`'s
    /// `validate_rcept_no` doc comment) — so slicing a date out of it would
    /// fabricate point-in-time evidence this normalizer has no license to
    /// assert. Do not "improve" this into a date parser.
    pub disclosure_acceptance_number: String,
    /// The `제출인` cell's display text (see [`cell_display_text`]).
    pub filer_name: String,
    /// The exact `시간` cell text as it appeared on the page, trimmed of
    /// surrounding whitespace only (e.g. `"2020-02-07 14:46"`).
    pub posted_local_raw: String,
    /// [`Self::posted_local_raw`] parsed into a local calendar date and
    /// wall-clock time to the minute — **no timezone attached**. Minute
    /// granularity is the ceiling; no second component is ever fabricated.
    pub posted_local: NaiveDateTime,
    /// The UTC instant [`Self::posted_local`] denotes, computed **only**
    /// under [`Self::timezone_assumption`]. Never read this without also
    /// reading that field.
    pub posted_at_instant: UtcTimestamp,
    /// The named assumption [`Self::posted_at_instant`] rests on. KIND
    /// documents no timezone anywhere for `시간`.
    pub timezone_assumption: TimezoneAssumption,
    /// The source page this observation was parsed from (e.g.
    /// `page-0002.html`), for traceability back to the immutable Raw batch.
    pub source_file_name: String,
    /// 0-based index of this row among that page's data rows (the `<thead>`
    /// placeholder row excluded), for traceability.
    pub source_row_index: usize,
}

/// One immutable source file recorded in normalization lineage, mirroring
/// [`crate::normalize::NormalizationSourceFile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindNormalizationSourceFile {
    pub file_name: String,
    pub content_hash: ContentHash,
}

/// The source identity a normalized KIND batch was derived from, mirroring
/// [`crate::normalize::NormalizationLineage`]'s shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindNormalizationLineage {
    pub schema_version: u32,
    pub normalizer: String,
    pub upstream_provider: String,
    pub upstream_market: String,
    pub upstream_batch_id: BatchId,
    pub upstream_files: Vec<KindNormalizationSourceFile>,
}

/// The JSON document persisted as the normalized batch's single file
/// (`observations.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredKindDisclosureDocument {
    schema_version: u32,
    normalizer: String,
    lineage: KindNormalizationLineage,
    row_count: usize,
    observations: Vec<KindDisclosureObservation>,
}

/// A stored, verified normalized KIND disclosure batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindNormalizationOutcome {
    /// The deterministic identity of the normalized batch (see
    /// [`deterministic_kind_disclosure_normalized_batch_id`]).
    pub normalized_batch_id: BatchId,
    /// The source `provider=kind-disclosure` batch this was derived from.
    pub source_batch_id: BatchId,
    /// [`PROVIDER_KIND_DISCLOSURE`].
    pub source_provider: &'static str,
    /// [`PROVIDER_KIND_DISCLOSURE_NORMALIZED`].
    pub normalized_provider: &'static str,
    /// `observations.len()`.
    pub row_count: usize,
    /// The parsed observations, in source page/row order.
    pub observations: Vec<KindDisclosureObservation>,
    /// Full upstream lineage (schema version, normalizer id, source file
    /// hashes) — see [`KindNormalizationLineage`].
    pub lineage: KindNormalizationLineage,
    /// The stored manifest row for the normalized batch.
    pub entry: ManifestEntry,
}

/// Returns the stable normalized-batch identity for one KIND source batch.
/// Deterministic in the source batch id (and this normalizer's own
/// version), so re-normalizing the same Raw batch always yields the same
/// identity instead of appending a new manifest row each time.
pub fn deterministic_kind_disclosure_normalized_batch_id(source_batch_id: BatchId) -> BatchId {
    let name = format!(
        "provider={PROVIDER_KIND_DISCLOSURE_NORMALIZED}\nnormalizer={NORMALIZER}\nsource_batch={source_batch_id}"
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

// ---------------------------------------------------------------------
// Minimal, dependency-free HTML table extraction.
//
// KIND pages are simple, machine-generated search-result fragments: one
// `<table>` whose rows are `<tr>...</tr>` and whose cells are `<td>`/`<th>`.
// This crate has no HTML-parsing dependency (and may not add one), so this
// is a small hand-rolled scanner rather than a general HTML parser.
//
// Every needle searched for below is pure ASCII (`<table`, `<tr`, `</td`,
// ...). Hangul (and every other non-ASCII character KIND's page text can
// contain) is encoded in UTF-8 using only bytes >= 0x80, which can never
// equal an ASCII byte — so a byte-level case-insensitive match against an
// ASCII needle can only start and end at byte offsets that already fall on
// UTF-8 character boundaries. Slicing the original `&str` at those offsets
// is therefore always safe.
// ---------------------------------------------------------------------

fn ascii_ieq(a: u8, b: u8) -> bool {
    a.eq_ignore_ascii_case(&b)
}

/// Case-insensitive byte search for a pure-ASCII `needle` in `haystack`,
/// starting at byte offset `from`. See the module-level note above for why
/// this is safe on UTF-8 input containing non-ASCII text.
fn find_ascii_ci(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() || needle.len() > haystack.len() - from {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(&h, &n)| ascii_ieq(h, n))
    })
}

/// Whether the byte right after a matched tag name (`<td`, `<tr`, ...)
/// legitimately ends the tag name, rather than merely being a longer tag
/// name's prefix (e.g. `<table` must not match as `<t`... it doesn't here,
/// but `<tr` must not match inside a hypothetical `<track>`).
fn ends_tag_name(byte: Option<u8>) -> bool {
    matches!(
        byte,
        Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
    )
}

/// The literal text of one `<table>`'s opening tag, plus everything between
/// it and the matching `</table` close tag.
struct TableParts<'a> {
    /// The opening tag's literal text, e.g. `<table class="list"
    /// summary="번호, 시간, 종목명, 공시제목,제출인">` — used to extract the
    /// `summary` attribute (see [`validate_table_summary`]).
    opening_tag: &'a str,
    /// Everything between the opening tag and `</table`.
    inner: &'a str,
}

/// Locates the first `<table>...</table>` in `html`. `None` if no table tag
/// is present at all.
fn extract_first_table(html: &str) -> Option<TableParts<'_>> {
    let bytes = html.as_bytes();
    let open_start = find_ascii_ci(bytes, b"<table", 0)?;
    let open_end = bytes[open_start..].iter().position(|&b| b == b'>')? + open_start + 1;
    let close_start = find_ascii_ci(bytes, b"</table", open_end)?;
    Some(TableParts {
        opening_tag: &html[open_start..open_end],
        inner: &html[open_end..close_start],
    })
}

/// Extracts the inner HTML of every top-level `<tr>...</tr>` in `table_html`,
/// in document order.
fn extract_rows(table_html: &str) -> Vec<&str> {
    let bytes = table_html.as_bytes();
    let mut rows = Vec::new();
    let mut pos = 0usize;
    while let Some(open_start) = find_ascii_ci(bytes, b"<tr", pos) {
        if !ends_tag_name(bytes.get(open_start + 3).copied()) {
            pos = open_start + 3;
            continue;
        }
        let Some(open_end_rel) = bytes[open_start..].iter().position(|&b| b == b'>') else {
            break;
        };
        let open_end = open_start + open_end_rel + 1;
        let Some(close_start) = find_ascii_ci(bytes, b"</tr", open_end) else {
            break;
        };
        rows.push(&table_html[open_end..close_start]);
        let Some(close_end_rel) = bytes[close_start..].iter().position(|&b| b == b'>') else {
            break;
        };
        pos = close_start + close_end_rel + 1;
    }
    rows
}

/// Extracts every `<td>`/`<th>` cell's **raw, unstripped** inner HTML from
/// one row's inner HTML, in document order. Deliberately returns raw HTML
/// rather than cleaned text: callers extract display text (see
/// [`cell_display_text`]) and/or scan the raw HTML for `onclick`-embedded
/// identifiers (see [`extract_call_single_quoted_arg`]) as needed, and
/// cleaning here would destroy the `onclick`/`title` attributes the second
/// use needs.
fn extract_cells_raw(row_html: &str) -> Vec<&str> {
    let bytes = row_html.as_bytes();
    let mut cells = Vec::new();
    let mut pos = 0usize;
    loop {
        let td = find_ascii_ci(bytes, b"<td", pos);
        let th = find_ascii_ci(bytes, b"<th", pos);
        let (open_start, close_needle): (usize, &[u8]) = match (td, th) {
            (Some(t), Some(h)) if h < t => (h, b"</th"),
            (Some(t), Some(_)) => (t, b"</td"),
            (Some(t), None) => (t, b"</td"),
            (None, Some(h)) => (h, b"</th"),
            (None, None) => break,
        };
        if !ends_tag_name(bytes.get(open_start + 3).copied()) {
            pos = open_start + 3;
            continue;
        }
        let Some(open_end_rel) = bytes[open_start..].iter().position(|&b| b == b'>') else {
            break;
        };
        let open_end = open_start + open_end_rel + 1;
        let Some(close_start) = find_ascii_ci(bytes, close_needle, open_end) else {
            break;
        };
        cells.push(&row_html[open_end..close_start]);
        let after_close = close_start + close_needle.len();
        let Some(close_end_rel) = bytes[after_close..].iter().position(|&b| b == b'>') else {
            break;
        };
        pos = after_close + close_end_rel + 1;
    }
    cells
}

/// Whether a row contains a syntactically delimited `<td>` or `<th>` opening
/// tag. Unlike [`extract_cells_raw`], this does not require a matching close
/// tag: the placeholder contract rejects a cell opening even in malformed
/// markup, rather than silently discarding that row.
fn has_cell_opening_tag(row_html: &str) -> bool {
    let bytes = row_html.as_bytes();
    for needle in [b"<td".as_slice(), b"<th".as_slice()] {
        let mut pos = 0usize;
        while let Some(open_start) = find_ascii_ci(bytes, needle, pos) {
            if ends_tag_name(bytes.get(open_start + 3).copied()) {
                return true;
            }
            pos = open_start + 3;
        }
    }
    false
}

/// The literal text of one `<a>`'s opening tag, plus its inner text.
struct AnchorParts<'a> {
    /// The opening tag's literal text, e.g. `<a ... title='ACE 200'>` — used
    /// to extract the `title` attribute (see [`cell_display_text`]).
    opening_tag: &'a str,
    /// Everything between the opening tag and `</a`.
    inner_text: &'a str,
}

/// Locates the first `<a>...</a>` in `cell_html`. `None` if no anchor is
/// present.
fn extract_first_anchor(cell_html: &str) -> Option<AnchorParts<'_>> {
    let bytes = cell_html.as_bytes();
    let mut pos = 0usize;
    loop {
        let open_start = find_ascii_ci(bytes, b"<a", pos)?;
        if !ends_tag_name(bytes.get(open_start + 2).copied()) {
            pos = open_start + 2;
            continue;
        }
        let open_end_rel = bytes[open_start..].iter().position(|&b| b == b'>')?;
        let open_end = open_start + open_end_rel + 1;
        let close_start = find_ascii_ci(bytes, b"</a", open_end)?;
        return Some(AnchorParts {
            opening_tag: &cell_html[open_start..open_end],
            inner_text: &cell_html[open_end..close_start],
        });
    }
}

/// Extracts a double- or single-quoted HTML attribute's value from
/// `tag_html` (the literal text of one opening tag, e.g. `<table
/// summary="...">` or `<a ... title='...'>`), matching the attribute name
/// case-insensitively and only at a real attribute boundary (preceded by
/// tag-start or whitespace, followed by optional whitespace then `=`) —
/// never merely a substring of a longer attribute name (e.g. a hypothetical
/// `data-summary` must not match `summary`). Returns `None` if the
/// attribute is absent, or present with an unquoted value (KIND's own
/// markup always quotes attribute values, so an unquoted one is treated as
/// absent rather than guessed at).
fn extract_attribute<'a>(tag_html: &'a str, attr_name: &str) -> Option<&'a str> {
    let bytes = tag_html.as_bytes();
    let needle = attr_name.as_bytes();
    let mut pos = 0usize;
    loop {
        let start = find_ascii_ci(bytes, needle, pos)?;
        let preceded_by_boundary =
            start == 0 || matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r' | b'<');
        let mut i = start + needle.len();
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        if preceded_by_boundary && bytes.get(i) == Some(&b'=') {
            i += 1;
            while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                i += 1;
            }
            let quote = *bytes.get(i)?;
            if quote == b'"' || quote == b'\'' {
                let value_start = i + 1;
                let value_end = tag_html[value_start..].find(quote as char)? + value_start;
                return Some(&tag_html[value_start..value_end]);
            }
        }
        pos = start + needle.len();
    }
}

/// Strips every `<...>` tag from `input`, leaving only text content.
fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Decodes the small set of HTML entities KIND's pages plausibly use.
fn decode_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

/// Cleans one already-isolated span of text: strips any nested tags, decodes
/// entities, then trims surrounding whitespace only (never collapses
/// internal whitespace — the byte-faithful requirement is about not
/// inventing content, not about reformatting it).
fn clean_cell_text(inner: &str) -> String {
    decode_entities(&strip_tags(inner)).trim().to_owned()
}

/// Extracts one cell's *display* text: if the cell contains an anchor,
/// prefers that anchor's `title` attribute (KIND uses it to carry the full,
/// untruncated value, useful when the visible text is later truncated) and
/// falls back to the anchor's own inner text only when `title` is absent or
/// blank; a cell with no anchor at all falls back to the whole cell's
/// tag-stripped text. Either way the result is trimmed of surrounding
/// whitespace only.
fn cell_display_text(cell_html: &str) -> String {
    let Some(anchor) = extract_first_anchor(cell_html) else {
        return clean_cell_text(cell_html);
    };
    if let Some(title) = extract_attribute(anchor.opening_tag, "title") {
        let cleaned = decode_entities(title).trim().to_owned();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    clean_cell_text(anchor.inner_text)
}

/// Extracts the first single-quoted argument of a `<call_prefix>'value'...)`
/// JS call embedded anywhere in `html` (e.g. an `onclick` attribute),
/// verbatim and un-validated — callers validate the shape themselves so a
/// failure can be attributed to the specific field it came from. `None` if
/// `call_prefix` (which must include the trailing `(`) does not appear at
/// all, or is not immediately followed by a single-quoted value.
fn extract_call_single_quoted_arg(html: &str, call_prefix: &str) -> Option<String> {
    let start = find_ascii_ci(html.as_bytes(), call_prefix.as_bytes(), 0)?;
    let rest = html[start + call_prefix.len()..].strip_prefix('\'')?;
    let end = rest.find('\'')?;
    Some(rest[..end].to_owned())
}

/// Whether `value` is composed only of ASCII digits and its length falls
/// within `min_len..=max_len` (inclusive).
fn is_ascii_digit_string(value: &str, min_len: usize, max_len: usize) -> bool {
    (min_len..=max_len).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

/// Extracts and validates the name cell's KIND-internal issue key (see
/// [`KindDisclosureObservation::kind_internal_issue_key`]'s docs): the
/// argument of an `etfisusummary_open('<key>')` call, required to be 1-12
/// ASCII digits.
fn extract_and_validate_kind_internal_issue_key(
    name_cell_html: &str,
    location: &RowLocation,
) -> Result<String, KindNormalizeError> {
    let raw = extract_call_single_quoted_arg(name_cell_html, "etfisusummary_open(");
    match &raw {
        Some(value) if is_ascii_digit_string(value, 1, 12) => Ok(value.clone()),
        _ => Err(KindNormalizeError::InvalidKindInternalIssueKey {
            location: location.clone(),
            value: raw,
        }),
    }
}

/// Extracts and validates the title cell's disclosure acceptance number (see
/// [`KindDisclosureObservation::disclosure_acceptance_number`]'s docs): the
/// first argument of an `openDisclsViewer('<number>','...')` call, required
/// to be exactly 14 ASCII digits.
fn extract_and_validate_disclosure_acceptance_number(
    title_cell_html: &str,
    location: &RowLocation,
) -> Result<String, KindNormalizeError> {
    let raw = extract_call_single_quoted_arg(title_cell_html, "openDisclsViewer(");
    match &raw {
        Some(value) if is_ascii_digit_string(value, 14, 14) => Ok(value.clone()),
        _ => Err(KindNormalizeError::InvalidDisclosureAcceptanceNumber {
            location: location.clone(),
            value: raw,
        }),
    }
}

/// Validates the table's `summary` attribute against
/// [`EXPECTED_SUMMARY_COLUMNS`]: split on `,`, trim each part, and require
/// an exact match. A missing `summary` attribute, or any other column list
/// (wrong count, wrong labels, wrong order), is
/// [`KindNormalizeError::UnsupportedHeader`]. Real captured bytes have been
/// observed with inconsistent spacing around the commas (`"번호, 시간,
/// 종목명, 공시제목,제출인"`), so only the trimmed parts are ever compared,
/// never the raw string.
fn validate_table_summary(file_name: &str, opening_tag: &str) -> Result<(), KindNormalizeError> {
    let actual: Vec<String> = match extract_attribute(opening_tag, "summary") {
        Some(raw) => decode_entities(raw)
            .split(',')
            .map(|part| part.trim().to_owned())
            .collect(),
        None => Vec::new(),
    };
    let matches = actual.len() == EXPECTED_SUMMARY_COLUMNS.len()
        && actual
            .iter()
            .zip(EXPECTED_SUMMARY_COLUMNS.iter())
            .all(|(a, e)| a == e);
    if matches {
        Ok(())
    } else {
        Err(KindNormalizeError::UnsupportedHeader {
            file_name: file_name.to_owned(),
            expected: &EXPECTED_SUMMARY_COLUMNS,
            actual,
        })
    }
}

/// Strictly parses a `시간` cell as `YYYY-MM-DD HH:MM` — exactly 16 bytes in
/// that fixed shape, never chrono's lenient numeric-width parsing. Returns
/// `None` for anything else, including valid-looking values with extra
/// characters, single-digit components, or a seconds component.
fn parse_kind_local_datetime(raw: &str) -> Option<NaiveDateTime> {
    let bytes = raw.as_bytes();
    if bytes.len() != 16 {
        return None;
    }
    let digit = |i: usize| bytes[i].is_ascii_digit();
    if !(0..4).all(digit) || bytes[4] != b'-' {
        return None;
    }
    if !(5..7).all(digit) || bytes[7] != b'-' {
        return None;
    }
    if !(8..10).all(digit) || bytes[10] != b' ' {
        return None;
    }
    if !(11..13).all(digit) || bytes[13] != b':' {
        return None;
    }
    if !(14..16).all(digit) {
        return None;
    }
    let year: i32 = raw[0..4].parse().ok()?;
    let month: u32 = raw[5..7].parse().ok()?;
    let day: u32 = raw[8..10].parse().ok()?;
    let hour: u32 = raw[11..13].parse().ok()?;
    let minute: u32 = raw[14..16].parse().ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    Some(NaiveDateTime::new(date, time))
}

/// Parses a `번호` cell: must be composed only of ASCII digits and denote a
/// value strictly greater than zero.
fn parse_sequence_number(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: u64 = raw.parse().ok()?;
    if value == 0 { None } else { Some(value) }
}

/// Parses one page's HTML body into observations, without checking the
/// cross-page `번호` sequence (the caller does that once over the whole
/// flattened batch).
fn parse_page(
    file_name: &str,
    html: &str,
) -> Result<Vec<KindDisclosureObservation>, KindNormalizeError> {
    let table = extract_first_table(html).ok_or_else(|| KindNormalizeError::MissingTable {
        file_name: file_name.to_owned(),
    })?;
    validate_table_summary(file_name, table.opening_tag)?;

    let rows = extract_rows(table.inner);
    let (placeholder_row, data_rows) =
        rows.split_first()
            .ok_or_else(|| KindNormalizeError::InvalidPlaceholderRow {
                file_name: file_name.to_owned(),
            })?;
    // The first `<tr>` is always the `<thead>` placeholder row. Real KIND
    // markup ships it empty (`<tr id="title-contents"></tr>`) — the column
    // labels live only in the table's `summary` attribute, already
    // validated above, never in `<th>` cells (see the module-level docs).
    // Do not discard it until proving it is that empty placeholder. Otherwise
    // a first data row could be silently omitted while the surviving sequence
    // still appears valid.
    if has_cell_opening_tag(placeholder_row) {
        return Err(KindNormalizeError::InvalidPlaceholderRow {
            file_name: file_name.to_owned(),
        });
    }

    let mut observations = Vec::new();
    for (row_index, row_html) in data_rows.iter().enumerate() {
        let location = RowLocation {
            file_name: file_name.to_owned(),
            row_index,
        };
        let cells = extract_cells_raw(row_html);
        if cells.len() != EXPECTED_COLUMN_COUNT {
            return Err(KindNormalizeError::RowCellCountMismatch {
                location,
                expected: EXPECTED_COLUMN_COUNT,
                actual: cells.len(),
            });
        }
        let [number_cell, time_cell, name_cell, title_cell, filer_cell] =
            <[&str; 5]>::try_from(cells).expect("checked length above");

        let number_raw = cell_display_text(number_cell);
        let time_raw = cell_display_text(time_cell);
        let issue_name = cell_display_text(name_cell);
        let disclosure_title = cell_display_text(title_cell);
        let filer_name = cell_display_text(filer_cell);

        let sequence_number = parse_sequence_number(&number_raw).ok_or_else(|| {
            KindNormalizeError::InvalidSequenceNumber {
                location: location.clone(),
                value: number_raw.clone(),
            }
        })?;
        let posted_local = parse_kind_local_datetime(&time_raw).ok_or_else(|| {
            KindNormalizeError::InvalidTimestamp {
                location: location.clone(),
                value: time_raw.clone(),
            }
        })?;
        if issue_name.is_empty() {
            return Err(KindNormalizeError::EmptyRequiredField {
                location: location.clone(),
                field: RequiredField::IssueName,
            });
        }
        let kind_internal_issue_key =
            extract_and_validate_kind_internal_issue_key(name_cell, &location)?;
        if disclosure_title.is_empty() {
            return Err(KindNormalizeError::EmptyRequiredField {
                location: location.clone(),
                field: RequiredField::DisclosureTitle,
            });
        }
        let disclosure_acceptance_number =
            extract_and_validate_disclosure_acceptance_number(title_cell, &location)?;

        let assumption = TimezoneAssumption::AssumedAsiaSeoul;
        let venue_local =
            VenueTimestamp::from_naive_local(Venue::Krx, posted_local).map_err(|source| {
                KindNormalizeError::LocalTimeAssumptionFailed {
                    location: RowLocation {
                        file_name: file_name.to_owned(),
                        row_index,
                    },
                    local: posted_local,
                    assumption,
                    source,
                }
            })?;

        observations.push(KindDisclosureObservation {
            sequence_number,
            issue_name,
            kind_internal_issue_key,
            instrument_identity: InstrumentIdentity::unresolved(),
            disclosure_title,
            disclosure_acceptance_number,
            filer_name,
            posted_local_raw: time_raw,
            posted_local,
            posted_at_instant: venue_local.to_utc(),
            timezone_assumption: assumption,
            source_file_name: file_name.to_owned(),
            source_row_index: row_index,
        });
    }
    Ok(observations)
}

/// Validates the `번호` sequence across the whole flattened, page-ordered
/// batch: strictly descending by exactly 1, no gaps, no duplicates.
fn validate_sequence(observations: &[KindDisclosureObservation]) -> Result<(), KindNormalizeError> {
    for pair in observations.windows(2) {
        let [previous, current] = pair else {
            unreachable!("windows(2) always yields 2 elements")
        };
        let expected = previous.sequence_number.saturating_sub(1);
        if current.sequence_number != expected {
            return Err(KindNormalizeError::SequenceNumberOutOfOrder {
                previous_location: RowLocation {
                    file_name: previous.source_file_name.clone(),
                    row_index: previous.source_row_index,
                },
                previous_value: previous.sequence_number,
                actual_location: RowLocation {
                    file_name: current.source_file_name.clone(),
                    row_index: current.source_row_index,
                },
                actual_value: current.sequence_number,
                expected_value: expected,
            });
        }
    }
    Ok(())
}

/// Parses every page of an already-verified KIND source batch (in stored
/// file-name order) into the full, page/row-ordered observation list, and
/// checks the cross-page `번호` invariant. Pure: performs no I/O and writes
/// nothing.
pub fn parse_kind_disclosure_pages(
    pages: &[(String, Vec<u8>)],
) -> Result<Vec<KindDisclosureObservation>, KindNormalizeError> {
    let mut ordered: Vec<&(String, Vec<u8>)> = pages.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut observations = Vec::new();
    for (file_name, bytes) in ordered {
        let html = String::from_utf8_lossy(bytes);
        observations.extend(parse_page(file_name, &html)?);
    }
    if observations.is_empty() {
        return Err(KindNormalizeError::EmptyBatch);
    }
    validate_sequence(&observations)?;
    Ok(observations)
}

fn source_lineage(source: &ManifestEntry) -> Vec<KindNormalizationSourceFile> {
    source
        .files
        .iter()
        .map(|file| KindNormalizationSourceFile {
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
        })
        .collect()
}

fn validate_source_scope(source: &ManifestEntry) -> Result<(), KindNormalizeError> {
    if source.provider != PROVIDER_KIND_DISCLOSURE {
        return Err(KindNormalizeError::UnsupportedScope {
            expected: PROVIDER_KIND_DISCLOSURE,
            actual: source.provider.clone(),
        });
    }
    Ok(())
}

fn build_document(
    lineage: &KindNormalizationLineage,
    observations: &[KindDisclosureObservation],
) -> Result<Vec<u8>, KindNormalizeError> {
    let document = StoredKindDisclosureDocument {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        lineage: lineage.clone(),
        row_count: observations.len(),
        observations: observations.to_vec(),
    };
    serde_json::to_vec(&document).map_err(|error| KindNormalizeError::Serialization {
        reason: error.to_string(),
    })
}

fn expected_manifest_entry(
    source: &ManifestEntry,
    spec: &BatchSpec<'_>,
    envelope: &RawEnvelope,
) -> ManifestEntry {
    ManifestEntry {
        batch_id: spec.batch_id,
        provider: spec.provider.to_owned(),
        market: spec.market.to_owned(),
        date: *spec.date,
        retrieved_at: source.retrieved_at,
        mode: spec.mode,
        entitlement_reference: spec.entitlement_reference.map(str::to_owned),
        files: vec![FileEntry {
            kind: envelope.kind,
            file_name: envelope.file_name.clone(),
            content_hash: envelope.content_hash.clone(),
            size_bytes: envelope.bytes.len() as u64,
            request: envelope.request.clone(),
        }],
    }
}

fn existing_batch_conflict(batch_id: BatchId, reason: impl Into<String>) -> KindNormalizeError {
    KindNormalizeError::ExistingBatchConflict {
        batch_id,
        reason: reason.into(),
    }
}

fn load_existing_normalized_batch(
    raw: &RawStore,
    source: &ManifestEntry,
    expected_entry: &ManifestEntry,
    expected_bytes: &[u8],
    lineage: &KindNormalizationLineage,
    observations: &[KindDisclosureObservation],
) -> Result<Option<KindNormalizationOutcome>, KindNormalizeError> {
    let existing = raw
        .read_reconciled_manifest(PROVIDER_KIND_DISCLOSURE_NORMALIZED, &source.market)?
        .into_iter()
        .find(|entry| entry.batch_id == expected_entry.batch_id);
    let Some(entry) = existing else {
        return Ok(None);
    };
    if &entry != expected_entry {
        return Err(existing_batch_conflict(
            entry.batch_id,
            "manifest metadata, canonical shape, or lineage differs",
        ));
    }
    let files =
        raw.read_batch_bytes(PROVIDER_KIND_DISCLOSURE_NORMALIZED, &source.market, &entry)?;
    let Some(stored) = files
        .iter()
        .find(|file| file.file_name == OBSERVATIONS_FILE_NAME)
    else {
        return Err(existing_batch_conflict(
            entry.batch_id,
            format!("canonical file {OBSERVATIONS_FILE_NAME} is missing"),
        ));
    };
    if stored.bytes != expected_bytes {
        return Err(existing_batch_conflict(
            entry.batch_id,
            format!("canonical file {OBSERVATIONS_FILE_NAME} bytes differ"),
        ));
    }
    Ok(Some(KindNormalizationOutcome {
        normalized_batch_id: entry.batch_id,
        source_batch_id: source.batch_id,
        source_provider: PROVIDER_KIND_DISCLOSURE,
        normalized_provider: PROVIDER_KIND_DISCLOSURE_NORMALIZED,
        row_count: observations.len(),
        observations: observations.to_vec(),
        lineage: lineage.clone(),
        entry,
    }))
}

/// Reads one stored `provider=kind-disclosure` batch, parses every page into
/// observations, and stores one immutable `provider=kind-disclosure-normalized`
/// batch containing them. The normalized identity is deterministic for the
/// source batch (see [`deterministic_kind_disclosure_normalized_batch_id`]),
/// so calling this again for the same source batch returns the already
/// verified immutable result instead of appending a second manifest row.
///
/// Nothing is written unless every page and every row parses successfully —
/// see the module-level docs and [`KindNormalizeError`].
pub fn normalize_kind_disclosure_batch(
    raw: &RawStore,
    source: &ManifestEntry,
) -> Result<KindNormalizationOutcome, KindNormalizeError> {
    validate_source_scope(source)?;
    let stored = raw.read_batch_bytes(&source.provider, &source.market, source)?;
    let pages: Vec<(String, Vec<u8>)> = stored
        .iter()
        .map(|file| (file.file_name.clone(), file.bytes.clone()))
        .collect();
    let observations = parse_kind_disclosure_pages(&pages)?;

    let batch_id = deterministic_kind_disclosure_normalized_batch_id(source.batch_id);
    let lineage = KindNormalizationLineage {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        upstream_provider: source.provider.clone(),
        upstream_market: source.market.clone(),
        upstream_batch_id: source.batch_id,
        upstream_files: source_lineage(source),
    };
    let bytes = build_document(&lineage, &observations)?;

    let spec = BatchSpec {
        provider: PROVIDER_KIND_DISCLOSURE_NORMALIZED,
        market: &source.market,
        date: &source.date,
        batch_id,
        entitlement_reference: source.entitlement_reference.as_deref(),
        mode: source.mode,
    };
    let envelope = RawEnvelope::new(
        batch_id,
        ResponseKind::DisclosureIndex,
        OBSERVATIONS_FILE_NAME,
        bytes.clone(),
        source.retrieved_at,
        RequestMetadata {
            endpoint: NORMALIZER.to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            mode: source.mode,
        },
    );
    let expected_entry = expected_manifest_entry(source, &spec, &envelope);

    if let Some(outcome) = load_existing_normalized_batch(
        raw,
        source,
        &expected_entry,
        &bytes,
        &lineage,
        &observations,
    )? {
        return Ok(outcome);
    }

    match raw.store_batch(&spec, std::slice::from_ref(&envelope)) {
        Ok(entry) => {
            if entry != expected_entry {
                return Err(existing_batch_conflict(
                    batch_id,
                    "RawStore returned manifest metadata different from the deterministic contract",
                ));
            }
            Ok(KindNormalizationOutcome {
                normalized_batch_id: batch_id,
                source_batch_id: source.batch_id,
                source_provider: PROVIDER_KIND_DISCLOSURE,
                normalized_provider: PROVIDER_KIND_DISCLOSURE_NORMALIZED,
                row_count: observations.len(),
                observations,
                lineage,
                entry,
            })
        }
        Err(error @ StoreError::FileExists { .. }) => {
            // Another caller can create the deterministic directory before
            // its manifest line becomes visible. Re-read a few times so
            // concurrent retries converge once the durable metadata is
            // exposed, mirroring `crate::normalize`'s same-shaped race.
            for _ in 0..COLLISION_RETRIES {
                if let Some(outcome) = load_existing_normalized_batch(
                    raw,
                    source,
                    &expected_entry,
                    &bytes,
                    &lineage,
                    &observations,
                )? {
                    return Ok(outcome);
                }
                std::thread::sleep(COLLISION_RETRY_DELAY);
            }
            Err(KindNormalizeError::Store(error))
        }
        Err(error) => Err(KindNormalizeError::Store(error)),
    }
}
