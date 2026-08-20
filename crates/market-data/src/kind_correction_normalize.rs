//! KIND ETF correction/version viewer evidence.
//!
//! This module parses the rendered `mainDoc` `<select>` from the
//! owner-approved KIND correction viewer. It intentionally models only what
//! the artifact proves: an ordered list of disclosure acceptance values and
//! their literal date-bearing labels. It does not infer that the anchor is a
//! member of the list, does not join a correction chain, and does not derive a
//! date from an acceptance number.
//!
//! The parser is deliberately small and dependency-free, like
//! [`crate::kind_normalize`]. It is not a general HTML parser. The browser
//! stage supplies a rendered DOM snapshot, and the Rust boundary accepts only
//! the narrow select/option shape observed for this surface. Invalid UTF-8,
//! missing/duplicate `mainDoc`, a non-empty placeholder value, malformed values,
//! duplicate acceptance values, missing/duplicate/malformed dates, or an
//! empty version list all fail closed.

use std::collections::BTreeSet;
use std::time::Duration;

use domain::{BatchId, ContentHash, TradingDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contract::{
    PROVIDER_KIND_DISCLOSURE_CORRECTION, PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
    RawEnvelope, RequestMetadata, ResponseKind,
};
use crate::providers::kind::{
    KIND_CORRECTION_ARTIFACT_KIND, KIND_CORRECTION_ENTRY_URL, KIND_CORRECTION_SURFACE,
    KIND_CORRECTION_TERMINATION, KIND_CORRECTION_TERMINATION_STAGE,
    KIND_CORRECTION_VIEWER_ENDPOINT, KIND_CORRECTION_VIEWER_FILE,
    KIND_CORRECTION_VIEWER_ORIGIN_PATH, MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT,
    MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES, MAX_KIND_CORRECTION_VIEWER_BYTES,
};
use crate::storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};

const NORMALIZER: &str = "kind-correction-viewer-to-ordered-membership-v1";
const NORMALIZER_SCHEMA_VERSION: u32 = 1;
const MEMBERSHIP_FILE_NAME: &str = "membership.json";
const COLLISION_RETRIES: usize = 100;
const COLLISION_RETRY_DELAY: Duration = Duration::from_millis(2);

/// One typed ordered version entry from `mainDoc`.
///
/// `date` is validated as a real calendar date, while `date_literal` and
/// `label` retain the source spelling. `acceptance_number` is opaque except
/// for its exact 14-ASCII-digit shape; no date or lineage is inferred from
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCorrectionVersion {
    /// Position in the rendered `mainDoc` option list. The required empty
    /// placeholder is index 0, so admitted versions start at index 1.
    pub option_index: usize,
    /// Opaque KIND acceptance value, exactly 14 ASCII digits.
    pub acceptance_number: String,
    /// Literal option value, including the observed `|Y` marker. The marker
    /// is preserved as opaque evidence and is not interpreted as lineage.
    pub raw_value: String,
    /// Real calendar date represented by the one date token in `label`.
    pub date: TradingDate,
    /// Literal `YYYY.MM.DD` token as rendered in the option label.
    pub date_literal: String,
    /// Literal rendered option label (surrounding whitespace removed only).
    pub label: String,
}

impl KindCorrectionVersion {
    /// Literal option value, including its opaque marker.
    pub fn value(&self) -> &str {
        &self.raw_value
    }

    /// Opaque 14-digit acceptance token.
    pub fn acceptance(&self) -> &str {
        &self.acceptance_number
    }
}

/// One typed ordered-membership result for a correction viewer capture.
///
/// The anchor is retained as opaque evidence. Its relationship to the
/// options is deliberately unresolved: it may be absent from the list, and a
/// one-entry list is not evidence of predecessor/supersedes/withdrawal
/// semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCorrectionMembership {
    /// The requested/ clicked acceptance value, retained without deriving a
    /// date or asserting membership/equivalence.
    pub anchor_acceptance_number: String,
    /// Options in exact rendered order, excluding the required placeholder.
    pub ordered_versions: Vec<KindCorrectionVersion>,
}

/// Source file identity retained in normalized lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCorrectionNormalizationSourceFile {
    pub file_name: String,
    pub content_hash: ContentHash,
}

/// Complete source identity attached to the normalized membership document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCorrectionNormalizationLineage {
    pub schema_version: u32,
    pub normalizer: String,
    pub upstream_provider: String,
    pub upstream_market: String,
    pub upstream_batch_id: BatchId,
    pub upstream_files: Vec<KindCorrectionNormalizationSourceFile>,
}

/// A stored, verified normalized correction-viewer batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindCorrectionNormalizationOutcome {
    pub normalized_batch_id: BatchId,
    pub source_batch_id: BatchId,
    pub source_provider: &'static str,
    pub normalized_provider: &'static str,
    pub membership: KindCorrectionMembership,
    pub lineage: KindCorrectionNormalizationLineage,
    pub entry: ManifestEntry,
}

/// Why a correction-viewer body or normalized source batch failed closed.
#[derive(Debug, thiserror::Error)]
pub enum KindCorrectionViewerError {
    #[error("kind correction viewer bytes are not valid UTF-8")]
    MalformedUtf8,
    #[error("kind correction anchor acceptance must be exactly 14 ASCII digits")]
    InvalidAnchorAcceptance,
    #[error("kind correction viewer has no select identified by id or name mainDoc")]
    MainDocMissing,
    #[error("kind correction viewer has more than one select identified by id or name mainDoc")]
    MainDocDuplicate,
    #[error("kind correction viewer mainDoc select is malformed or has no closing tag")]
    MainDocMalformed,
    #[error("kind correction viewer mainDoc has no option elements")]
    NoOptions,
    #[error("kind correction viewer mainDoc option 0 must have an explicit empty value")]
    PlaceholderInvalid,
    #[error("kind correction viewer mainDoc has zero admitted version options")]
    ZeroVersions,
    #[error("kind correction viewer option {option_index} has a missing or malformed value")]
    InvalidOptionValue { option_index: usize },
    #[error("kind correction viewer option {option_index} uses an unsupported version marker")]
    UnsupportedOptionValue { option_index: usize },
    #[error("kind correction viewer has duplicate acceptance value at option {option_index}")]
    DuplicateAcceptance { option_index: usize },
    #[error("kind correction viewer option {option_index} has no unique YYYY.MM.DD date token")]
    DateTokenMissing { option_index: usize },
    #[error("kind correction viewer option {option_index} has duplicate date tokens")]
    DateTokenDuplicate { option_index: usize },
    #[error(
        "kind correction viewer option {option_index} has a malformed or non-calendar date token"
    )]
    DateTokenInvalid { option_index: usize },
    #[error("kind correction viewer body exceeds {max_bytes} bytes")]
    Oversize { max_bytes: u64 },
    #[error("kind correction source batch must use provider {expected}, got {actual}")]
    UnsupportedScope {
        expected: &'static str,
        actual: String,
    },
    #[error("kind correction source batch must contain exactly one viewer.html file")]
    InvalidSourceFiles,
    #[error("kind correction source manifest file metadata is inconsistent")]
    InvalidSourceMetadata,
    #[error("existing deterministic correction normalized batch {batch_id} conflicts: {reason}")]
    ExistingBatchConflict { batch_id: BatchId, reason: String },
    #[error("correction membership serialization failed: {reason}")]
    Serialization { reason: String },
    #[error("source Raw read failed: {0}")]
    Store(#[from] StoreError),
}

/// Stable deterministic identity for the normalized correction membership.
pub fn deterministic_kind_correction_normalized_batch_id(source_batch_id: BatchId) -> BatchId {
    let name = format!(
        "provider={PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED}\nnormalizer={NORMALIZER}\nsource_batch={source_batch_id}"
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

fn ascii_ieq(a: u8, b: u8) -> bool {
    a.eq_ignore_ascii_case(&b)
}

fn find_ascii_ci(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() || needle.len() > haystack.len() - from {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(&left, &right)| ascii_ieq(left, right))
    })
}

fn tag_name_boundary(byte: Option<u8>) -> bool {
    matches!(
        byte,
        Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
    )
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut quote = None;
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        match quote {
            Some(current) if *byte == current => quote = None,
            None if *byte == b'\'' || *byte == b'"' => quote = Some(*byte),
            None if *byte == b'>' => return Some(offset + 1),
            _ => {}
        }
    }
    None
}

fn find_close_tag(html: &str, tag: &[u8], from: usize) -> Option<(usize, usize)> {
    let bytes = html.as_bytes();
    let mut needle = Vec::with_capacity(tag.len() + 2);
    needle.extend_from_slice(b"</");
    needle.extend_from_slice(tag);
    let mut pos = from;
    while let Some(start) = find_ascii_ci(bytes, &needle, pos) {
        let after = start + needle.len();
        if tag_name_boundary(bytes.get(after).copied()) {
            let end = find_tag_end(html, start)?;
            return Some((start, end));
        }
        pos = after;
    }
    None
}

fn extract_attribute<'a>(tag_html: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag_html.as_bytes();
    let needle = name.as_bytes();
    let mut pos = 0;
    while let Some(start) = find_ascii_ci(bytes, needle, pos) {
        let before = if start == 0 {
            None
        } else {
            bytes.get(start - 1).copied()
        };
        let boundary = matches!(before, None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'<'));
        let mut cursor = start + needle.len();
        while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            cursor += 1;
        }
        if boundary && bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                cursor += 1;
            }
            let quote = *bytes.get(cursor)?;
            if quote != b'\'' && quote != b'"' {
                return None;
            }
            let value_start = cursor + 1;
            let value_end = tag_html[value_start..].find(quote as char)? + value_start;
            return Some(&tag_html[value_start..value_end]);
        }
        pos = start + needle.len();
    }
    None
}

fn strip_tags(input: &str) -> Option<String> {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' if !in_tag => in_tag = true,
            '>' if in_tag => in_tag = false,
            '<' | '>' if in_tag => {}
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    if in_tag { None } else { Some(output) }
}

fn parse_ascii_acceptance(value: &str) -> Option<(&str, bool)> {
    let marker = value.strip_suffix("|Y");
    let unsupported = value.strip_suffix("|N").is_some();
    let Some(number) = marker else {
        return Some(("", unsupported));
    };
    if number.len() == 14 && number.bytes().all(|byte| byte.is_ascii_digit()) {
        Some((number, false))
    } else {
        Some(("", false))
    }
}

fn date_tokens(label: &str) -> Vec<(usize, String)> {
    let bytes = label.as_bytes();
    if bytes.len() < 10 {
        return Vec::new();
    }
    (0..=bytes.len() - 10)
        .filter_map(|start| {
            let slice = &bytes[start..start + 10];
            let shape = slice[0..4].iter().all(u8::is_ascii_digit)
                && slice[4] == b'.'
                && slice[5..7].iter().all(u8::is_ascii_digit)
                && slice[7] == b'.'
                && slice[8..10].iter().all(u8::is_ascii_digit);
            if !shape {
                return None;
            }
            let before_ok = start == 0
                || !bytes[start - 1].is_ascii_alphanumeric()
                    && bytes[start - 1] != b'.'
                    && bytes[start - 1] != b'_';
            let after = start + 10;
            let after_ok = after == bytes.len()
                || !bytes[after].is_ascii_alphanumeric()
                    && bytes[after] != b'.'
                    && bytes[after] != b'_';
            if before_ok && after_ok {
                Some((start, String::from_utf8_lossy(slice).into_owned()))
            } else {
                None
            }
        })
        .collect()
}

fn parse_version_date(
    label: &str,
    option_index: usize,
) -> Result<(TradingDate, String), KindCorrectionViewerError> {
    let tokens = date_tokens(label);
    if tokens.is_empty() {
        return Err(KindCorrectionViewerError::DateTokenMissing { option_index });
    }
    if tokens.len() != 1 {
        return Err(KindCorrectionViewerError::DateTokenDuplicate { option_index });
    }
    let token = tokens[0].1.clone();
    let iso = format!("{}-{}-{}", &token[0..4], &token[5..7], &token[8..10]);
    let date = TradingDate::parse(&iso)
        .map_err(|_| KindCorrectionViewerError::DateTokenInvalid { option_index })?;
    Ok((date, token))
}

/// Parses and validates the rendered viewer body into ordered version entries.
/// `anchor_acceptance_number` is checked only for exact opaque shape and is
/// otherwise not compared with the options.
pub fn parse_kind_correction_viewer(
    bytes: &[u8],
    anchor_acceptance_number: &str,
) -> Result<Vec<KindCorrectionVersion>, KindCorrectionViewerError> {
    if bytes.len() as u64 > MAX_KIND_CORRECTION_VIEWER_BYTES {
        return Err(KindCorrectionViewerError::Oversize {
            max_bytes: MAX_KIND_CORRECTION_VIEWER_BYTES,
        });
    }
    if anchor_acceptance_number.len() != 14
        || !anchor_acceptance_number
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(KindCorrectionViewerError::InvalidAnchorAcceptance);
    }
    let html = std::str::from_utf8(bytes).map_err(|_| KindCorrectionViewerError::MalformedUtf8)?;
    let bytes = html.as_bytes();

    let mut main_doc: Option<(usize, usize)> = None;
    let mut pos = 0;
    while let Some(start) = find_ascii_ci(bytes, b"<select", pos) {
        if !tag_name_boundary(bytes.get(start + 7).copied()) {
            pos = start + 7;
            continue;
        }
        let end = find_tag_end(html, start).ok_or(KindCorrectionViewerError::MainDocMalformed)?;
        let opening = &html[start..end];
        let id = extract_attribute(opening, "id");
        let name = extract_attribute(opening, "name");
        let is_main = id == Some("mainDoc") || name == Some("mainDoc");
        if is_main {
            if main_doc.is_some() {
                return Err(KindCorrectionViewerError::MainDocDuplicate);
            }
            let (close_start, close_end) = find_close_tag(html, b"select", end)
                .ok_or(KindCorrectionViewerError::MainDocMalformed)?;
            main_doc = Some((end, close_start));
            // Continue after this complete select so a second mainDoc is
            // detected even when it occurs after the first closing tag.
            pos = close_end;
        } else {
            pos = end;
        }
    }
    let Some((inner_start, inner_end)) = main_doc else {
        return Err(KindCorrectionViewerError::MainDocMissing);
    };
    let inner = &html[inner_start..inner_end];

    let mut options: Vec<(usize, bool, String, String)> = Vec::new();
    let mut option_pos = 0;
    while let Some(start) = find_ascii_ci(inner.as_bytes(), b"<option", option_pos) {
        if !tag_name_boundary(inner.as_bytes().get(start + 7).copied()) {
            option_pos = start + 7;
            continue;
        }
        let end = find_tag_end(inner, start).ok_or(KindCorrectionViewerError::MainDocMalformed)?;
        let opening = &inner[start..end];
        // Use an explicit option close search here to avoid treating an
        // unrelated tag as an option terminator.
        let close_start = find_ascii_ci(inner.as_bytes(), b"</option", end)
            .filter(|candidate| tag_name_boundary(inner.as_bytes().get(candidate + 8).copied()))
            .ok_or(KindCorrectionViewerError::MainDocMalformed)?;
        let close_end =
            find_tag_end(inner, close_start).ok_or(KindCorrectionViewerError::MainDocMalformed)?;
        let value_attribute = extract_attribute(opening, "value");
        let value_present = value_attribute.is_some();
        let value = value_attribute.unwrap_or("").to_owned();
        let label_attr = extract_attribute(opening, "label");
        let raw_label = label_attr.unwrap_or(&inner[end..close_start]);
        // Keep the source label literal (apart from surrounding whitespace
        // and nested markup delimiters). In particular, do not turn an
        // entity into a newly invented display string in disclosure
        // evidence.
        let label = strip_tags(raw_label)
            .ok_or(KindCorrectionViewerError::MainDocMalformed)?
            .trim()
            .to_owned();
        options.push((options.len(), value_present, value, label));
        option_pos = close_end;
    }
    if options.is_empty() {
        return Err(KindCorrectionViewerError::NoOptions);
    }
    let (placeholder_index, placeholder_value_present, placeholder_value, _) = &options[0];
    if *placeholder_index != 0 || !*placeholder_value_present || !placeholder_value.is_empty() {
        return Err(KindCorrectionViewerError::PlaceholderInvalid);
    }

    let mut seen = BTreeSet::new();
    let mut versions = Vec::with_capacity(options.len().saturating_sub(1));
    for (option_index, value_present, value, label) in options.into_iter().skip(1) {
        if !value_present {
            return Err(KindCorrectionViewerError::InvalidOptionValue { option_index });
        }
        let Some((acceptance, unsupported)) = parse_ascii_acceptance(&value) else {
            return Err(KindCorrectionViewerError::InvalidOptionValue { option_index });
        };
        if unsupported {
            return Err(KindCorrectionViewerError::UnsupportedOptionValue { option_index });
        }
        if acceptance.is_empty() {
            return Err(KindCorrectionViewerError::InvalidOptionValue { option_index });
        }
        if !seen.insert(acceptance.to_owned()) {
            return Err(KindCorrectionViewerError::DuplicateAcceptance { option_index });
        }
        let (date, date_literal) = parse_version_date(&label, option_index)?;
        versions.push(KindCorrectionVersion {
            option_index,
            acceptance_number: acceptance.to_owned(),
            raw_value: value,
            date,
            date_literal,
            label,
        });
    }
    if versions.is_empty() {
        return Err(KindCorrectionViewerError::ZeroVersions);
    }
    Ok(versions)
}

/// Parses a viewer body and wraps its ordered options with the opaque anchor.
pub fn parse_kind_correction_membership(
    bytes: &[u8],
    anchor_acceptance_number: &str,
) -> Result<KindCorrectionMembership, KindCorrectionViewerError> {
    if anchor_acceptance_number.len() != 14
        || !anchor_acceptance_number
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(KindCorrectionViewerError::InvalidAnchorAcceptance);
    }
    Ok(KindCorrectionMembership {
        anchor_acceptance_number: anchor_acceptance_number.to_owned(),
        ordered_versions: parse_kind_correction_viewer(bytes, anchor_acceptance_number)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredKindCorrectionDocument {
    schema_version: u32,
    normalizer: String,
    lineage: KindCorrectionNormalizationLineage,
    membership: KindCorrectionMembership,
}

fn source_lineage(source: &ManifestEntry) -> Vec<KindCorrectionNormalizationSourceFile> {
    source
        .files
        .iter()
        .map(|file| KindCorrectionNormalizationSourceFile {
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
        })
        .collect()
}

fn validate_source(source: &ManifestEntry) -> Result<(), KindCorrectionViewerError> {
    if source.provider != PROVIDER_KIND_DISCLOSURE_CORRECTION {
        return Err(KindCorrectionViewerError::UnsupportedScope {
            expected: PROVIDER_KIND_DISCLOSURE_CORRECTION,
            actual: source.provider.clone(),
        });
    }
    if source.files.len() != 1
        || source.files[0].file_name != KIND_CORRECTION_VIEWER_FILE
        || source.files[0].kind != ResponseKind::DisclosureVersionMembership
        || source.files[0].request.endpoint != KIND_CORRECTION_VIEWER_ENDPOINT
    {
        return Err(KindCorrectionViewerError::InvalidSourceFiles);
    }
    Ok(())
}

fn query_value<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    let mut values = query
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.as_str());
    let value = values.next()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}

fn anchor_from_source(source: &ManifestEntry) -> Result<String, KindCorrectionViewerError> {
    let query = &source.files[0].request.query;
    let allowed_keys = [
        "source",
        "entry_url",
        "surface",
        "requested_from",
        "requested_to",
        "anchor_acceptance_number",
        "viewer_origin_path",
        "artifact_kind",
        "termination",
        "termination_stage",
        "body_size",
        "form_field_count",
        "target_handler_occurrences",
    ];
    if query.len() != allowed_keys.len()
        || query
            .iter()
            .any(|(key, _)| !allowed_keys.contains(&key.as_str()))
        || allowed_keys
            .iter()
            .any(|key| query.iter().filter(|(name, _)| name == key).count() != 1)
    {
        return Err(KindCorrectionViewerError::InvalidSourceMetadata);
    }
    if query_value(query, "source") != Some("kind.krx.co.kr")
        || query_value(query, "entry_url") != Some(KIND_CORRECTION_ENTRY_URL)
        || query_value(query, "surface") != Some(KIND_CORRECTION_SURFACE)
        || query_value(query, "viewer_origin_path") != Some(KIND_CORRECTION_VIEWER_ORIGIN_PATH)
        || query_value(query, "artifact_kind") != Some(KIND_CORRECTION_ARTIFACT_KIND)
        || query_value(query, "termination") != Some(KIND_CORRECTION_TERMINATION)
        || query_value(query, "termination_stage") != Some(KIND_CORRECTION_TERMINATION_STAGE)
    {
        return Err(KindCorrectionViewerError::InvalidSourceMetadata);
    }
    let requested_from = query_value(query, "requested_from")
        .and_then(|value| TradingDate::parse(value).ok())
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    let requested_to = query_value(query, "requested_to")
        .and_then(|value| TradingDate::parse(value).ok())
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    if requested_from > requested_to {
        return Err(KindCorrectionViewerError::InvalidSourceMetadata);
    }
    let anchor = query_value(query, "anchor_acceptance_number")
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    if anchor.len() != 14 || !anchor.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(KindCorrectionViewerError::InvalidSourceMetadata);
    }
    let body_size = query_value(query, "body_size")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    let form_field_count = query_value(query, "form_field_count")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    let target_handler_occurrences = query_value(query, "target_handler_occurrences")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(KindCorrectionViewerError::InvalidSourceMetadata)?;
    if body_size == 0
        || body_size > MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES
        || form_field_count == 0
        || form_field_count > MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT
        || target_handler_occurrences == 0
        || target_handler_occurrences > MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT
    {
        return Err(KindCorrectionViewerError::InvalidSourceMetadata);
    }
    Ok(anchor.to_owned())
}

fn build_document(
    lineage: &KindCorrectionNormalizationLineage,
    membership: &KindCorrectionMembership,
) -> Result<Vec<u8>, KindCorrectionViewerError> {
    serde_json::to_vec(&StoredKindCorrectionDocument {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        lineage: lineage.clone(),
        membership: membership.clone(),
    })
    .map_err(|error| KindCorrectionViewerError::Serialization {
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

fn existing_batch_conflict(
    batch_id: BatchId,
    reason: impl Into<String>,
) -> KindCorrectionViewerError {
    KindCorrectionViewerError::ExistingBatchConflict {
        batch_id,
        reason: reason.into(),
    }
}

fn load_existing(
    raw: &RawStore,
    source: &ManifestEntry,
    expected_entry: &ManifestEntry,
    expected_bytes: &[u8],
    lineage: &KindCorrectionNormalizationLineage,
    membership: &KindCorrectionMembership,
) -> Result<Option<KindCorrectionNormalizationOutcome>, KindCorrectionViewerError> {
    let existing = raw
        .read_reconciled_manifest(
            PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
            &source.market,
        )?
        .into_iter()
        .find(|entry| entry.batch_id == expected_entry.batch_id);
    let Some(entry) = existing else {
        return Ok(None);
    };
    if &entry != expected_entry {
        return Err(existing_batch_conflict(
            entry.batch_id,
            "manifest metadata, canonical shape, lineage, or content hash differs",
        ));
    }
    let files = raw.read_batch_bytes(
        PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
        &source.market,
        &entry,
    )?;
    let Some(file) = files
        .iter()
        .find(|file| file.file_name == MEMBERSHIP_FILE_NAME)
    else {
        return Err(existing_batch_conflict(
            entry.batch_id,
            "canonical membership.json is missing",
        ));
    };
    if file.bytes != expected_bytes {
        return Err(existing_batch_conflict(
            entry.batch_id,
            "canonical membership.json bytes differ",
        ));
    }
    Ok(Some(KindCorrectionNormalizationOutcome {
        normalized_batch_id: entry.batch_id,
        source_batch_id: source.batch_id,
        source_provider: PROVIDER_KIND_DISCLOSURE_CORRECTION,
        normalized_provider: PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
        membership: membership.clone(),
        lineage: lineage.clone(),
        entry,
    }))
}

/// Reads one immutable correction-viewer Raw batch, validates the source body,
/// and stores one deterministic normalized ordered-membership document. A
/// repeat call returns the existing byte-verified deterministic result.
pub fn normalize_kind_correction_batch(
    raw: &RawStore,
    source: &ManifestEntry,
) -> Result<KindCorrectionNormalizationOutcome, KindCorrectionViewerError> {
    validate_source(source)?;
    let anchor = anchor_from_source(source)?;
    let stored = raw.read_batch_bytes(&source.provider, &source.market, source)?;
    let membership = parse_kind_correction_membership(&stored[0].bytes, &anchor)?;
    let lineage = KindCorrectionNormalizationLineage {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        upstream_provider: source.provider.clone(),
        upstream_market: source.market.clone(),
        upstream_batch_id: source.batch_id,
        upstream_files: source_lineage(source),
    };
    let batch_id = deterministic_kind_correction_normalized_batch_id(source.batch_id);
    let bytes = build_document(&lineage, &membership)?;
    let spec = BatchSpec {
        provider: PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
        market: &source.market,
        date: &source.date,
        batch_id,
        entitlement_reference: source.entitlement_reference.as_deref(),
        mode: source.mode,
    };
    let envelope = RawEnvelope::new(
        batch_id,
        ResponseKind::DisclosureVersionMembership,
        MEMBERSHIP_FILE_NAME,
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
    if let Some(outcome) =
        load_existing(raw, source, &expected_entry, &bytes, &lineage, &membership)?
    {
        return Ok(outcome);
    }
    match raw.store_batch(&spec, std::slice::from_ref(&envelope)) {
        Ok(entry) => {
            if entry != expected_entry {
                return Err(existing_batch_conflict(
                    batch_id,
                    "RawStore returned manifest metadata different from deterministic contract",
                ));
            }
            Ok(KindCorrectionNormalizationOutcome {
                normalized_batch_id: batch_id,
                source_batch_id: source.batch_id,
                source_provider: PROVIDER_KIND_DISCLOSURE_CORRECTION,
                normalized_provider: PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
                membership: membership.clone(),
                lineage,
                entry,
            })
        }
        Err(error @ StoreError::FileExists { .. }) => {
            for _ in 0..COLLISION_RETRIES {
                if let Some(outcome) =
                    load_existing(raw, source, &expected_entry, &bytes, &lineage, &membership)?
                {
                    return Ok(outcome);
                }
                std::thread::sleep(COLLISION_RETRY_DELAY);
            }
            Err(KindCorrectionViewerError::Store(error))
        }
        Err(error) => Err(KindCorrectionViewerError::Store(error)),
    }
}
