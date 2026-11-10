//! Original-backed traversal of the existing Paper combined archive format.
//!
//! The materialization, codec original and database-stream original are opened
//! independently. No caller manifest, row map, digest or callback return can
//! establish their membership. Every output is provisional through consuming
//! cleanup, and through the caller's snapshot/transport/generation completion.
//! Domain reconstruction is a separate mandatory gate; inventory is not closure.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::FileExt;
use std::path::Path;

use domain::ContentHash;
use sha2::{Digest, Sha256};

use super::{ArchiveFile, ArchiveSourceError, FileIdentity, HistoricalUseGuard, RetainedFiles};
use crate::checked_close::{Attempt, Gap};
use crate::raw_custody::RawInvocation;

const MAGIC: &[u8] = b"LAGRANGE_PAPER_COMBINED_FILES_V1\n";
const CLASSES: [&str; 20] = [
    "base", "config", "curated", "daily", "floor", "paper_config", "paper_contract",
    "paper_evidence", "paper_floor", "paper_manifest", "paper_operations", "paper_reader",
    "raw", "simulation_approval", "simulation_config", "simulation_floor", "simulation_input",
    "simulation_manifest", "simulation_reader", "simulation_receipt",
];
const PROJECTION: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
    "/../../docs/contracts/owner-beta-paper-audit-projection-v1.json"));
const MAX_LINE: usize = 146_800_640;
const MAX_NUMBER: usize = 131_072;
const MAX_ROWS: u64 = 10_000_000;

/// Diagnostics contain no source paths, SQL rows, provider prose or file bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceClosureError {
    #[error("ARCHIVE_SOURCE_CUSTODY")]
    Custody,
    #[error("ARCHIVE_SOURCE_NATIVE_BINDING")]
    NativeBinding,
    #[error("ARCHIVE_SOURCE_LIMIT")]
    Limit,
    #[error("ARCHIVE_SOURCE_JSON")]
    Json,
    #[error("ARCHIVE_SOURCE_DUPLICATE")]
    Duplicate,
    #[error("ARCHIVE_SOURCE_CODEC")]
    Codec,
    #[error("ARCHIVE_SOURCE_INVENTORY")]
    Inventory,
    #[error("ARCHIVE_SOURCE_MEMBER")]
    Member,
    #[error("ARCHIVE_SOURCE_METADATA")]
    Metadata,
    #[error("ARCHIVE_SOURCE_DATABASE_STREAM")]
    DatabaseStream,
    #[error("ARCHIVE_SOURCE_DATABASE_ROW")]
    DatabaseRow,
    #[error("ARCHIVE_SOURCE_REFERENCE_SET")]
    ReferenceSet,
    #[error("ARCHIVE_SOURCE_RELATIONSHIP")]
    Relationship,
    #[error("ARCHIVE_SOURCE_HISTORICAL_SELECTION")]
    HistoricalSelection,
    #[error("ARCHIVE_SOURCE_V2_REGISTRY_MISSING")]
    HistoricalV2RegistryMissing,
    #[error("ARCHIVE_SOURCE_V3_REGISTRY_MISSING")]
    HistoricalV3RegistryMissing,
    #[error("ARCHIVE_SOURCE_V2_ARTIFACT_MISSING")]
    HistoricalV2ArtifactMissing,
    #[error("ARCHIVE_SOURCE_V3_ARTIFACT_MISSING")]
    HistoricalV3ArtifactMissing,
    #[error("ARCHIVE_SOURCE_DAILY_POLICY_ORIGINAL_MISSING")]
    DailyPolicyOriginalMissing,
    #[error("ARCHIVE_SOURCE_DAILY_CONFIG_ORIGINAL_MISSING")]
    DailyConfigOriginalMissing,
    #[error("ARCHIVE_SOURCE_DAILY_OBSERVATION_ORIGINAL_MISSING")]
    DailyObservationOriginalMissing,
    #[error("ARCHIVE_SOURCE_DAILY_EXCLUSION_UNPROVEN")]
    DailyExclusionUnproven,
    #[error("ARCHIVE_SOURCE_DAILY_POLL_PREIMAGE")]
    DailyPollPreimage,
    #[error("ARCHIVE_SOURCE_CURATED_DEPENDENCY")]
    CuratedDependency,
    #[error("ARCHIVE_SOURCE_CURATED_PARQUET_READER_UNAVAILABLE")]
    CuratedParquetReaderUnavailable,
    #[error("ARCHIVE_SOURCE_CURATED_LEGACY_ARTIFACTS_UNRETAINED")]
    CuratedLegacyArtifactsUnretained,
    #[error("ARCHIVE_SOURCE_CURATED_NORMALIZED_REPLAY_UNAVAILABLE")]
    CuratedNormalizedReplayUnavailable,
    #[error("ARCHIVE_SOURCE_CURATED_DATASET_BINDING_UNAVAILABLE")]
    CuratedDatasetBindingUnavailable,
    #[error("ARCHIVE_SOURCE_RECONSTRUCTION")]
    Reconstruction,
    #[error("ARCHIVE_SOURCE_INTERRUPTED")]
    Interrupted,
    #[error("ARCHIVE_SOURCE_DOMAIN_CLOSURE_INCOMPLETE")]
    DomainClosureIncomplete,
}
type Result<T> = std::result::Result<T, SourceClosureError>;

fn require(ok: bool, error: SourceClosureError) -> Result<()> {
    if ok { Ok(()) } else { Err(error) }
}
fn digest(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }
fn sha(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| v.len() == 64
        && v.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}
fn ascii(value: &str) -> bool { value.bytes().all(|b| (32..=126).contains(&b)) }

/// Explicit aggregate retention/work bounds. No implicit production default.
#[derive(Clone, Copy, Debug)]
pub struct SourceClosureLimits {
    pub max_members: usize,
    pub max_manifest_bytes: usize,
    pub max_reference_bytes: usize,
    pub max_reference_rows: usize,
    pub max_open_descriptors: usize,
    pub max_file_bytes: u64,
    pub max_stream_bytes: u64,
}
impl SourceClosureLimits {
    fn validate(self) -> Result<Self> {
        require((20..=1_000_000).contains(&self.max_members)
            && (1024..=67_108_864).contains(&self.max_manifest_bytes)
            && (1024..=67_108_864).contains(&self.max_reference_bytes)
            && (1..=1_000_000).contains(&self.max_reference_rows)
            && (24..=1_000_000).contains(&self.max_open_descriptors)
            && self.max_file_bytes > 0 && self.max_file_bytes <= 1 << 50
            && self.max_stream_bytes > 0 && self.max_stream_bytes <= 1 << 50,
            SourceClosureError::Limit)?;
        Ok(self)
    }
}

// Lossless JSON numbers are decimal text, never f32/f64. This is the audit
// codec's base-ten normalization, not a price/financial formatter. Financial
// strings remain opaque and are checked by their existing typed readers.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Json {
    Null, Bool(bool), Number(String), String(String), Array(Vec<Json>), Object(BTreeMap<String, Json>),
}
impl Json {
    fn object(&self) -> Result<&BTreeMap<String, Json>> {
        if let Self::Object(v) = self { Ok(v) } else { Err(SourceClosureError::Json) }
    }
    fn array(&self) -> Result<&[Json]> {
        if let Self::Array(v) = self { Ok(v) } else { Err(SourceClosureError::Json) }
    }
    fn text(&self) -> Result<&str> {
        if let Self::String(v) = self { Ok(v) } else { Err(SourceClosureError::Json) }
    }
    fn integer(&self) -> Result<i64> {
        if let Self::Number(v) = self { v.parse().map_err(|_| SourceClosureError::Json) }
        else { Err(SourceClosureError::Json) }
    }
    fn get(&self, key: &str) -> Result<&Json> {
        self.object()?.get(key).ok_or(SourceClosureError::Json)
    }
    fn keys(&self, names: &[&str]) -> Result<()> {
        let map = self.object()?;
        require(map.len() == names.len() && names.iter().all(|n| map.contains_key(*n)),
            SourceClosureError::Json)
    }
    fn canonical(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new(); self.write(&mut out, false)?; Ok(out)
    }
    fn write(&self, out: &mut Vec<u8>, ascii_strings: bool) -> Result<()> {
        match self {
            Self::Null => out.extend_from_slice(b"null"),
            Self::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
            Self::Number(v) => out.extend_from_slice(v.as_bytes()),
            Self::String(v) => write_string(v, out, ascii_strings)?,
            Self::Array(v) => {
                out.push(b'[');
                for (i, child) in v.iter().enumerate() {
                    if i != 0 { out.push(b','); } child.write(out, ascii_strings)?;
                }
                out.push(b']');
            }
            Self::Object(v) => {
                out.push(b'{');
                for (i, (key, child)) in v.iter().enumerate() {
                    if i != 0 { out.push(b','); }
                    write_string(key, out, ascii_strings)?; out.push(b':'); child.write(out, ascii_strings)?;
                }
                out.push(b'}');
            }
        }
        Ok(())
    }
    fn serde_value(&self) -> Result<serde_json::Value> {
        // Typed wire readers have no floating point input seam. Refuse it here
        // instead of allowing serde_json to round a decimal to binary float.
        match self {
            Self::Number(v) => Ok(serde_json::Value::from(v.parse::<i64>()
                .map_err(|_| SourceClosureError::DatabaseRow)?)),
            Self::Null => Ok(serde_json::Value::Null),
            Self::Bool(v) => Ok((*v).into()), Self::String(v) => Ok(v.clone().into()),
            Self::Array(v) => Ok(serde_json::Value::Array(v.iter().map(Self::serde_value).collect::<Result<_>>()?)),
            Self::Object(v) => Ok(serde_json::Value::Object(v.iter().map(|(k, v)|
                Ok((k.clone(), v.serde_value()?))).collect::<Result<_>>()?)),
        }
    }
}
fn write_string(value: &str, out: &mut Vec<u8>, ascii_strings: bool) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| SourceClosureError::Json)?;
    if !ascii_strings { out.extend_from_slice(&bytes); return Ok(()); }
    // Python's fixed codec uses ensure_ascii=True, including UTF-16 surrogate
    // pairs. Its audit row canonicalizer instead uses original UTF-8.
    for c in std::str::from_utf8(&bytes).map_err(|_| SourceClosureError::Json)?.chars() {
        if c.is_ascii() { out.push(c as u8); }
        else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.extend_from_slice(format!("\\u{unit:04x}").as_bytes());
            }
        }
    }
    Ok(())
}

struct JsonParser<'a> { input: &'a [u8], at: usize, retained: usize, limit: usize }
fn parse(bytes: &[u8]) -> Result<Json> {
    parse_bounded(bytes, MAX_LINE)
}
fn parse_bounded(bytes: &[u8], budget: usize) -> Result<Json> {
    require(budget > 0 && budget <= MAX_LINE && !bytes.is_empty() && bytes.len() <= budget, SourceClosureError::Limit)?;
    let mut parser = JsonParser { input: bytes, at: 0, retained: 0, limit: budget };
    let value = parser.value(0)?; parser.whitespace();
    require(parser.at == bytes.len(), SourceClosureError::Json)?; Ok(value)
}
impl JsonParser<'_> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.retained = self.retained.checked_add(bytes).filter(|n| *n <= self.limit)
            .ok_or(SourceClosureError::Limit)?;
        Ok(())
    }
    fn whitespace(&mut self) {
        while self.input.get(self.at).is_some_and(|b| b" \r\n\t".contains(b)) { self.at += 1; }
    }
    fn eat(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.input.get(self.at) == Some(&byte) { self.at += 1; true } else { false }
    }
    fn string(&mut self) -> Result<String> {
        self.whitespace(); let start = self.at;
        require(self.input.get(self.at) == Some(&b'"'), SourceClosureError::Json)?; self.at += 1;
        loop {
            let b = *self.input.get(self.at).ok_or(SourceClosureError::Json)?; self.at += 1;
            if b == b'"' { break; }
            if b == b'\\' { self.at = self.at.checked_add(1).ok_or(SourceClosureError::Limit)?; }
            require(b >= 32 && self.at <= self.input.len(), SourceClosureError::Json)?;
        }
        let value: String = serde_json::from_slice(&self.input[start..self.at]).map_err(|_| SourceClosureError::Json)?;
        // Bound both owned UTF-8 and its worst-case canonical escaping. Tiny
        // exponent tokens or many short object keys cannot expand without an
        // aggregate cap merely because the original line was small.
        self.charge(value.len().checked_mul(6).and_then(|n| n.checked_add(2)).ok_or(SourceClosureError::Limit)?)?;
        Ok(value)
    }
    fn value(&mut self, depth: usize) -> Result<Json> {
        require(depth <= 64, SourceClosureError::Limit)?;
        self.charge(128)?;
        self.whitespace();
        match self.input.get(self.at).copied().ok_or(SourceClosureError::Json)? {
            b'"' => Ok(Json::String(self.string()?)),
            b'{' => {
                require(depth < 64, SourceClosureError::Limit)?;
                self.at += 1; let mut map = BTreeMap::new();
                if self.eat(b'}') { return Ok(Json::Object(map)); }
                loop {
                    self.charge(128)?;
                    let key = self.string()?;
                    require(self.eat(b':'), SourceClosureError::Json)?;
                    let value = self.value(depth + 1)?;
                    require(map.insert(key, value).is_none(), SourceClosureError::Duplicate)?;
                    if self.eat(b'}') { break; }
                    require(self.eat(b','), SourceClosureError::Json)?;
                }
                Ok(Json::Object(map))
            }
            b'[' => {
                require(depth < 64, SourceClosureError::Limit)?;
                self.at += 1; let mut array = Vec::new();
                if self.eat(b']') { return Ok(Json::Array(array)); }
                loop {
                    array.push(self.value(depth + 1)?);
                    if self.eat(b']') { break; }
                    require(self.eat(b','), SourceClosureError::Json)?;
                }
                Ok(Json::Array(array))
            }
            b'n' => { self.literal(b"null")?; Ok(Json::Null) }
            b't' => { self.literal(b"true")?; Ok(Json::Bool(true)) }
            b'f' => { self.literal(b"false")?; Ok(Json::Bool(false)) }
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while self.input.get(self.at).is_some_and(|b| b.is_ascii_digit() || b"-+.eE".contains(b)) {
                    self.at += 1;
                }
                let number = normalize_number(&self.input[start..self.at])?;
                self.charge(number.len())?;
                Ok(Json::Number(number))
            }
            _ => Err(SourceClosureError::Json),
        }
    }
    fn literal(&mut self, expected: &[u8]) -> Result<()> {
        require(self.input.get(self.at..self.at + expected.len()) == Some(expected), SourceClosureError::Json)?;
        self.at += expected.len(); Ok(())
    }
}

fn normalize_number(bytes: &[u8]) -> Result<String> {
    let mut at = 0;
    let negative = bytes.first() == Some(&b'-'); if negative { at += 1; }
    let first = *bytes.get(at).ok_or(SourceClosureError::Json)?;
    require(first.is_ascii_digit(), SourceClosureError::Json)?;
    let integer_start = at; at += 1;
    if first != b'0' { while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; } }
    let integer_end = at;
    let mut digits = bytes[integer_start..integer_end].to_vec();
    if bytes.get(at) == Some(&b'.') {
        at += 1; let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
        require(at > start, SourceClosureError::Json)?; digits.extend_from_slice(&bytes[start..at]);
    }
    let mut exponent = 0i64;
    if bytes.get(at).is_some_and(|b| *b == b'e' || *b == b'E') {
        at += 1; let start = at;
        if bytes.get(at).is_some_and(|b| *b == b'+' || *b == b'-') { at += 1; }
        let number_start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
        require(at > number_start, SourceClosureError::Json)?;
        exponent = std::str::from_utf8(&bytes[start..at]).map_err(|_| SourceClosureError::Json)?
            .parse().map_err(|_| SourceClosureError::Limit)?;
    }
    require(at == bytes.len() && digits.len() <= MAX_NUMBER, SourceClosureError::Json)?;
    let leading = digits.iter().position(|b| *b != b'0').unwrap_or(digits.len());
    if leading == digits.len() {
        let adjusted = exponent.checked_sub((digits.len() - (integer_end - integer_start)) as i64)
            .ok_or(SourceClosureError::Limit)?;
        require(adjusted.unsigned_abs() <= MAX_NUMBER as u64, SourceClosureError::Limit)?;
        return Ok("0".into());
    }
    let point = ((integer_end - integer_start) as i64).checked_add(exponent)
        .and_then(|v| v.checked_sub(leading as i64)).ok_or(SourceClosureError::Limit)?;
    require(point.checked_sub(1).is_some_and(|n| n.unsigned_abs() <= MAX_NUMBER as u64),
        SourceClosureError::Limit)?;
    digits.drain(..leading);
    while digits.last() == Some(&b'0') { digits.pop(); }
    let mut out = String::new(); if negative { out.push('-'); }
    if point <= 0 {
        out.push_str("0."); out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(std::str::from_utf8(&digits).map_err(|_| SourceClosureError::Json)?);
    } else if point as usize >= digits.len() {
        out.push_str(std::str::from_utf8(&digits).map_err(|_| SourceClosureError::Json)?);
        out.extend(std::iter::repeat_n('0', point as usize - digits.len()));
    } else {
        let point = point as usize;
        out.push_str(std::str::from_utf8(&digits[..point]).map_err(|_| SourceClosureError::Json)?);
        out.push('.'); out.push_str(std::str::from_utf8(&digits[point..]).map_err(|_| SourceClosureError::Json)?);
    }
    Ok(out)
}

#[derive(Clone, Default)]
struct FramedRoot { hash: Sha256, rows: u64, bytes: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
struct RootValue { sha256: String, rows: u64, bytes: u64 }
impl FramedRoot {
    fn add(&mut self, table: &str, key: &str, row: &Json) -> Result<()> {
        require(ascii(table) && ascii(key), SourceClosureError::DatabaseRow)?;
        let row = row.canonical()?;
        require(self.rows < MAX_ROWS, SourceClosureError::Limit)?;
        for part in [table.as_bytes(), key.as_bytes(), row.as_slice()] {
            let prefix = format!("{}:", part.len());
            self.bytes = self.bytes.checked_add(prefix.len() as u64)
                .and_then(|v| v.checked_add(part.len() as u64))
                .filter(|v| *v <= i64::MAX as u64).ok_or(SourceClosureError::Limit)?;
            self.hash.update(prefix.as_bytes()); self.hash.update(part);
        }
        self.rows += 1; Ok(())
    }
    fn value(&self) -> RootValue {
        RootValue { sha256: format!("sha256:{:x}", self.hash.clone().finalize()), rows: self.rows, bytes: self.bytes }
    }
}

#[derive(Clone)]
struct Entry { path: String, directory: bool, uid: u32, gid: u32, mode: u32, size: u64, sha256: Option<String>, row: Json }
struct Manifest { entries: Vec<Entry>, by_path: BTreeMap<String, usize> }
fn safe_parts(path: &str) -> Result<Vec<&str>> {
    require(!path.is_empty() && path.len() <= 4096 && !path.chars().any(|c| c < ' ' || c == '\u{7f}' || c == '\\'),
        SourceClosureError::Member)?;
    let parts: Vec<_> = path.split('/').collect();
    require(parts.len() <= 64 && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".."),
        SourceClosureError::Member)?;
    require(!parts.iter().any(|p| *p == "commit.lock" || [".lock", ".sock", ".socket", ".pem", ".key", ".env"]
        .iter().any(|suffix| p.ends_with(suffix)) || ["credentials", "password", "pgpass"].contains(p)
        || p.starts_with(".pgpass")), SourceClosureError::Member)?;
    Ok(parts)
}
fn manifest(bytes: &[u8], limits: SourceClosureLimits) -> Result<Manifest> {
    require(bytes.len() <= limits.max_manifest_bytes, SourceClosureError::Limit)?;
    let value = parse(bytes)?; value.keys(&["schema", "classes", "entries"])?;
    let mut encoded = Vec::new(); value.write(&mut encoded, true)?;
    require(encoded == bytes && value.get("schema")?.text()? == "lagrange-paper-combined-files-v1",
        SourceClosureError::Codec)?;
    let classes = value.get("classes")?.array()?.iter().map(Json::text).collect::<Result<Vec<_>>>()?;
    require(classes == CLASSES, SourceClosureError::Codec)?;
    let rows = value.get("entries")?.array()?;
    require((20..=limits.max_members).contains(&rows.len()), SourceClosureError::Limit)?;
    let mut entries: Vec<Entry> = Vec::new(); let mut by_path: BTreeMap<String, usize> = BTreeMap::new();
    let mut previous: Option<Vec<String>> = None; let mut total = 0u64;
    for row in rows {
        row.keys(&["path", "kind", "uid", "gid", "mode", "size", "sha256"])?;
        let path = row.get("path")?.text()?; let parts = safe_parts(path)?;
        require(CLASSES.contains(&parts[0]), SourceClosureError::Codec)?;
        let order: Vec<_> = parts.iter().map(|v| v.to_string()).collect();
        require(previous.as_ref().is_none_or(|v| *v < order), SourceClosureError::Inventory)?;
        previous = Some(order);
        let directory = match row.get("kind")?.text()? { "file" => false, "directory" => true,
            _ => return Err(SourceClosureError::Codec) };
        let uid = u32::try_from(row.get("uid")?.integer()?).map_err(|_| SourceClosureError::Metadata)?;
        let gid = u32::try_from(row.get("gid")?.integer()?).map_err(|_| SourceClosureError::Metadata)?;
        let mode = u32::try_from(row.get("mode")?.integer()?).map_err(|_| SourceClosureError::Metadata)?;
        require([0, 10001, 10002, 10003].contains(&uid) && [0, 10001, 10002, 10003].contains(&gid)
            && mode <= 0o777 && mode & 0o022 == 0, SourceClosureError::Metadata)?;
        let size = u64::try_from(row.get("size")?.integer()?).map_err(|_| SourceClosureError::Limit)?;
        let hash = if directory {
            require(size == 0 && row.get("sha256")? == &Json::Null, SourceClosureError::Codec)?; None
        } else {
            let value = row.get("sha256")?.text()?;
            require(sha(value), SourceClosureError::Codec)?;
            total = total.checked_add(size).filter(|n| *n <= limits.max_file_bytes).ok_or(SourceClosureError::Limit)?;
            Some(value.to_string())
        };
        if parts.len() == 1 { require(directory, SourceClosureError::Inventory)?; }
        else {
            let parent = parts[..parts.len() - 1].join("/");
            let index = by_path.get(&parent).copied().ok_or(SourceClosureError::Inventory)?;
            require(entries.get(index).is_some_and(|e: &Entry| e.directory), SourceClosureError::Inventory)?;
        }
        require(by_path.insert(path.to_owned(), entries.len()).is_none(), SourceClosureError::Duplicate)?;
        entries.push(Entry { path: path.to_owned(), directory, uid, gid, mode, size, sha256: hash, row: row.clone() });
    }
    require(CLASSES.iter().all(|class| by_path.get(*class).is_some_and(|i| entries[*i].directory)),
        SourceClosureError::Inventory)?;
    Ok(Manifest { entries, by_path })
}

fn native_scope(invocation: &RawInvocation<'_>, expected: &Attempt, captured: &Attempt) -> Result<()> {
    if !expected.same_scope(captured) || !expected.same_scope(invocation.attempt()) {
        expected.gap(Gap::ScopeMismatch); captured.gap(Gap::ScopeMismatch);
        invocation.attempt().gap(Gap::ScopeMismatch);
        return Err(SourceClosureError::Custody);
    }
    require(expected.check() && captured.check(), SourceClosureError::Custody)?;
    let context = invocation.native_context().ok_or(SourceClosureError::NativeBinding)?;
    context.compare_scopes(invocation, expected).map_err(|_| SourceClosureError::NativeBinding)?;
    context.admit(invocation, expected).map_err(|_| SourceClosureError::NativeBinding)
}

fn retained_directory(path: &Path, expected: &Attempt) -> Result<RetainedFiles> {
    let value = RetainedFiles::open_with_attempt(path, &[], 0, 0o600, 0, Some(expected))
        .map_err(|_| SourceClosureError::Custody)?;
    let (_, _, identity) = value.directories.last().ok_or(SourceClosureError::Custody)?;
    require(identity.uid == 0 && identity.gid == 0 && identity.mode & 0o7777 == 0o700,
        SourceClosureError::Metadata)?;
    Ok(value)
}

/// One actual named original, held independently of a parsed codec/SQL object.
/// Its hash is computed during the first complete read, then independently read
/// again before consuming close. Every individual read checks FD and name.
struct InputOriginal {
    parent: RetainedFiles,
    file: ArchiveFile,
    name: String,
    identity: FileIdentity,
    size: u64,
}
impl InputOriginal {
    fn open(path: &Path, expected: &Attempt, maximum: u64) -> Result<Self> {
        Self::open_mode(path, expected, maximum, 0o600)
    }
    fn open_mode(path: &Path, expected: &Attempt, maximum: u64, mode: u32) -> Result<Self> {
        let name = path.file_name().and_then(|v| v.to_str()).ok_or(SourceClosureError::Member)?;
        require(safe_parts(name)?.len() == 1, SourceClosureError::Member)?;
        let parent = retained_directory(path.parent().ok_or(SourceClosureError::Member)?, expected)?;
        let fd = rustix::fs::openat(&parent.directories.last().ok_or(SourceClosureError::Custody)?.0,
            name, rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty()).map_err(|_| SourceClosureError::Custody)?;
        let file = ArchiveFile::adopt(File::from(fd), Some(expected));
        let info = rustix::fs::fstat(&file).map_err(|_| SourceClosureError::Custody)?;
        require(rustix::fs::FileType::from_raw_mode(info.st_mode) == rustix::fs::FileType::RegularFile
            && info.st_uid == 0 && info.st_gid == 0 && info.st_mode & 0o7777 == mode
            && info.st_nlink == 1 && info.st_size > 0 && info.st_size as u64 <= maximum,
            SourceClosureError::Metadata)?;
        super::archive_no_acl(&file).map_err(|_| SourceClosureError::Metadata)?;
        let value = Self { parent, file, name: name.to_owned(), identity: FileIdentity::from_stat(&info, true),
            size: info.st_size as u64 };
        value.revalidate()?; Ok(value)
    }
    fn revalidate(&self) -> Result<()> {
        self.parent.revalidate().map_err(|_| SourceClosureError::Custody)?;
        let fd = rustix::fs::fstat(&self.file).map_err(|_| SourceClosureError::Custody)?;
        let name = rustix::fs::statat(&self.parent.directories.last().ok_or(SourceClosureError::Custody)?.0,
            self.name.as_str(), rustix::fs::AtFlags::SYMLINK_NOFOLLOW).map_err(|_| SourceClosureError::Custody)?;
        require(FileIdentity::from_stat(&fd, true) == self.identity
            && FileIdentity::from_stat(&name, true) == self.identity, SourceClosureError::Custody)?;
        super::archive_no_acl(&self.file).map_err(|_| SourceClosureError::Metadata)
    }
    fn read_at(&self, bytes: &mut [u8], at: u64) -> Result<usize> {
        self.revalidate()?;
        let result = self.file.read_at(bytes, at).map_err(|_| SourceClosureError::Custody);
        let after = self.revalidate(); after.and(result)
    }
    fn hash(&self, keep: &mut impl FnMut() -> bool) -> Result<String> {
        let mut reader = OriginalReader::new(self, keep);
        let mut buffer = [0u8; 8192];
        while reader.read(&mut buffer).map_err(|_| reader.error.unwrap_or(SourceClosureError::Custody))? != 0 {}
        reader.completed_hash()
    }
}

struct OriginalReader<'a, K: FnMut() -> bool> {
    original: &'a InputOriginal, keep: &'a mut K, at: u64, hash: Sha256,
    error: Option<SourceClosureError>, eof: bool,
}
impl<'a, K: FnMut() -> bool> OriginalReader<'a, K> {
    fn new(original: &'a InputOriginal, keep: &'a mut K) -> Self {
        Self { original, keep, at: 0, hash: Sha256::new(), error: None, eof: false }
    }
    fn completed_hash(&self) -> Result<String> {
        require(self.eof && self.at == self.original.size && self.error.is_none(), SourceClosureError::Codec)?;
        self.original.revalidate()?; Ok(format!("sha256:{:x}", self.hash.clone().finalize()))
    }
}
impl<K: FnMut() -> bool> Read for OriginalReader<'_, K> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let body = (|| {
            require(self.error.is_none(), SourceClosureError::Custody)?;
            require((self.keep)(), SourceClosureError::Interrupted)?;
            if output.is_empty() { return Ok(0); }
            let count = self.original.read_at(output, self.at)?;
            self.at = self.at.checked_add(count as u64).filter(|n| *n <= self.original.size)
                .ok_or(SourceClosureError::Limit)?;
            self.hash.update(&output[..count]);
            if count == 0 { require(self.at == self.original.size, SourceClosureError::Codec)?; self.eof = true; }
            require((self.keep)(), SourceClosureError::Interrupted)?; Ok(count)
        })();
        body.map_err(|error| { self.error = Some(error); std::io::Error::other("ARCHIVE_SOURCE_READ") })
    }
}

struct TreeNode { file: ArchiveFile, identity: FileIdentity, parent: Option<usize>, name: String }
struct Materialized {
    root: RetainedFiles,
    manifest: Manifest,
    nodes: Vec<TreeNode>,
    children: BTreeMap<Option<usize>, BTreeSet<String>>,
}
impl Materialized {
    fn open(path: &Path, manifest: Manifest, expected: &Attempt, limits: SourceClosureLimits,
        keep: &mut impl FnMut() -> bool) -> Result<Self> {
        let root = retained_directory(path, expected)?;
        require(root.directories.len().checked_add(manifest.entries.len())
            .and_then(|n| n.checked_add(140)).is_some_and(|n| n <= limits.max_open_descriptors),
            SourceClosureError::Limit)?;
        let mut result = Self { root, manifest, nodes: Vec::new(), children: BTreeMap::new() };
        let mut identities = BTreeSet::new();
        for entry in &result.manifest.entries {
            require(keep(), SourceClosureError::Interrupted)?;
            let (parent, name) = match entry.path.rsplit_once('/') {
                Some((prefix, name)) => (Some(*result.manifest.by_path.get(prefix).ok_or(SourceClosureError::Inventory)?), name),
                None => (None, entry.path.as_str()),
            };
            let parent_file = result.parent_file(parent)?;
            let mut flags = rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK;
            if entry.directory { flags |= rustix::fs::OFlags::DIRECTORY; }
            let fd = rustix::fs::openat(parent_file, name, flags, rustix::fs::Mode::empty())
                .map_err(|_| SourceClosureError::Custody)?;
            let file = ArchiveFile::adopt(File::from(fd), Some(expected));
            let info = rustix::fs::fstat(&file).map_err(|_| SourceClosureError::Custody)?;
            validate_entry_stat(entry, &info)?;
            super::archive_no_acl(&file).map_err(|_| SourceClosureError::Metadata)?;
            require(identities.insert((info.st_dev, info.st_ino)), SourceClosureError::Inventory)?;
            // Include directory times, sizes and link count in the captured
            // identity as well: an append/delete/recreate cannot be hidden.
            let identity = FileIdentity::from_stat(&info, true);
            result.nodes.push(TreeNode { file, identity, parent, name: name.to_owned() });
            result.children.entry(parent).or_default().insert(name.to_owned());
        }
        result.revalidate(keep)?; Ok(result)
    }
    fn parent_file(&self, parent: Option<usize>) -> Result<&ArchiveFile> {
        match parent {
            Some(i) => self.nodes.get(i).map(|n| &n.file).ok_or(SourceClosureError::Inventory),
            None => self.root.directories.last().map(|n| &n.0).ok_or(SourceClosureError::Custody),
        }
    }
    fn check_node(&self, index: usize) -> Result<()> {
        self.root.revalidate().map_err(|_| SourceClosureError::Custody)?;
        let mut current = Some(index); let mut depth = 0;
        while let Some(index) = current {
            depth += 1; require(depth <= 64, SourceClosureError::Inventory)?;
            self.check_link(index)?;
            current = self.nodes[index].parent;
        }
        self.root.revalidate().map_err(|_| SourceClosureError::Custody)
    }
    fn check_link(&self, index: usize) -> Result<()> {
        let node = self.nodes.get(index).ok_or(SourceClosureError::Member)?;
        let fd = rustix::fs::fstat(&node.file).map_err(|_| SourceClosureError::Custody)?;
        let named = rustix::fs::statat(self.parent_file(node.parent)?, node.name.as_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW).map_err(|_| SourceClosureError::Custody)?;
        require(FileIdentity::from_stat(&fd, true) == node.identity
            && FileIdentity::from_stat(&named, true) == node.identity, SourceClosureError::Custody)?;
        validate_entry_stat(&self.manifest.entries[index], &fd)?;
        super::archive_no_acl(&node.file).map_err(|_| SourceClosureError::Metadata)
    }
    fn names(&self, parent: Option<usize>) -> Result<()> {
        let original = self.parent_file(parent)?;
        let fd = rustix::fs::openat(original, ".", rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty()).map_err(|_| SourceClosureError::Custody)?;
        let fd = ArchiveFile::adopt(File::from(fd), self.root.attempt.as_ref());
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        let mut directory = rustix::fs::RawDir::new(&fd, &mut buffer);
        let empty = BTreeSet::new(); let expected = self.children.get(&parent).unwrap_or(&empty);
        let mut seen = BTreeSet::new();
        while let Some(entry) = directory.next() {
            let entry = entry.map_err(|_| SourceClosureError::Custody)?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." { continue; }
            let name = std::str::from_utf8(bytes).map_err(|_| SourceClosureError::Inventory)?;
            require(expected.contains(name) && seen.insert(name.to_owned()), SourceClosureError::Inventory)?;
        }
        require(&seen == expected, SourceClosureError::Inventory)?;
        drop(directory); drop(fd);
        require(self.root.attempt.as_ref().is_some_and(Attempt::check), SourceClosureError::Custody)
    }
    fn revalidate(&self, keep: &mut impl FnMut() -> bool) -> Result<()> {
        self.root.revalidate().map_err(|_| SourceClosureError::Custody)?;
        self.names(None)?;
        for (i, entry) in self.manifest.entries.iter().enumerate() {
            require(keep(), SourceClosureError::Interrupted)?; self.check_node(i)?;
            if entry.directory { self.names(Some(i))?; self.check_node(i)?; }
        }
        self.root.revalidate().map_err(|_| SourceClosureError::Custody)
    }
    fn read(&self, path: &str, maximum: usize) -> Result<Vec<u8>> {
        // The resolver must select a catalog member first; no SQL path can be
        // opened here. Every descriptor was opened during complete enumeration.
        let i = *self.manifest.by_path.get(path).ok_or(SourceClosureError::Member)?;
        let entry = &self.manifest.entries[i];
        require(!entry.directory && entry.size <= maximum as u64, SourceClosureError::Limit)?;
        self.check_node(i)?;
        let body = (|| {
            let mut bytes = Vec::new(); bytes.try_reserve_exact(entry.size as usize).map_err(|_| SourceClosureError::Limit)?;
            bytes.resize(entry.size as usize, 0);
            self.nodes[i].file.read_exact_at(&mut bytes, 0).map_err(|_| SourceClosureError::Custody)?;
            require(entry.sha256.as_deref() == Some(digest(&bytes).as_str()), SourceClosureError::Member)?; Ok(bytes)
        })();
        let after = self.check_node(i); after.and(body)
    }
    fn hash_member(&self, i: usize, keep: &mut impl FnMut() -> bool) -> Result<()> {
        let entry = &self.manifest.entries[i]; require(!entry.directory, SourceClosureError::Member)?;
        self.check_node(i)?;
        let mut at = 0; let mut hash = Sha256::new(); let mut buffer = [0u8; 8192];
        while at < entry.size {
            require(keep(), SourceClosureError::Interrupted)?; self.check_node(i)?;
            let wanted = ((entry.size - at).min(buffer.len() as u64)) as usize;
            let count = self.nodes[i].file.read_at(&mut buffer[..wanted], at).map_err(|_| SourceClosureError::Custody)?;
            self.check_node(i)?; require(count != 0, SourceClosureError::Member)?;
            hash.update(&buffer[..count]); at += count as u64;
        }
        require(entry.sha256.as_deref() == Some(format!("sha256:{:x}", hash.finalize()).as_str()), SourceClosureError::Member)?;
        self.check_node(i)
    }
    fn verify_curated_values(&self, index: usize, schema_id: &str,
        expected_values: &crate::curate::schema::OriginalArtifactValues,
        limits: SourceClosureLimits, keep: &dyn Fn() -> bool) -> Result<()> {
        let entry = self.manifest.entries.get(index).ok_or(SourceClosureError::Member)?;
        require(!entry.directory, SourceClosureError::Member)?;
        self.hash_member(index, &mut || keep())?;
        let original_check = || self.check_node(index).map_err(|_| crate::curate::CurateError::MalformedParquet {
            context: "retained curated original".into(), detail: "original custody changed".into() });
        let typed = crate::curate::schema::verify_original_parquet(
            crate::curate::schema::OriginalParquetRead { file: &self.nodes[index].file,
                length: entry.size, max_bytes: limits.max_file_bytes.min(limits.max_stream_bytes),
                max_rows: limits.max_reference_rows, check: &original_check, keep },
            schema_id, expected_values).map_err(|_| SourceClosureError::CuratedDependency);
        // Both checks run on parse/value/cancellation failure. The consuming
        // traversal retains and finishes every original owner after this scope.
        let identity = self.check_node(index);
        let content = self.hash_member(index, &mut || keep());
        identity.and(content).and(typed)
    }
    fn visit_member(&self, i: usize, keep: &mut impl FnMut() -> bool,
        consume: &mut impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>) -> Result<()> {
        require(keep(), SourceClosureError::Interrupted)?;
        let entry = &self.manifest.entries[i]; let bytes = entry.row.canonical()?;
        self.check_node(i)?;
        if !entry.directory { self.hash_member(i, keep)?; }
        let body = consume(ArchiveOriginalMember::Member { path: &entry.path, canonical_record: &bytes,
            file: if entry.directory { None } else { Some(&self.nodes[i].file) } });
        let identity = self.check_node(i);
        let content = if entry.directory { Ok(()) } else { self.hash_member(i, keep) };
        identity.and(content).and(body)
    }
    fn verify_codec(&self, input: &mut impl Read, keep: &mut impl FnMut() -> bool) -> Result<()> {
        let mut buffer = [0u8; 8192]; let mut physical = [0u8; 8192];
        for (i, entry) in self.manifest.entries.iter().enumerate() {
            if entry.directory { continue; }
            self.check_node(i)?; let mut at = 0; let mut hash = Sha256::new();
            while at < entry.size {
                require(keep(), SourceClosureError::Interrupted)?; self.check_node(i)?;
                let count = (entry.size - at).min(buffer.len() as u64) as usize;
                input.read_exact(&mut buffer[..count]).map_err(|_| SourceClosureError::Codec)?;
                self.nodes[i].file.read_exact_at(&mut physical[..count], at).map_err(|_| SourceClosureError::Custody)?;
                let after = self.check_node(i);
                require(buffer[..count] == physical[..count], SourceClosureError::Member)?; after?;
                hash.update(&buffer[..count]); at += count as u64;
            }
            require(entry.sha256.as_deref() == Some(format!("sha256:{:x}", hash.finalize()).as_str()), SourceClosureError::Member)?;
            self.check_node(i)?;
        }
        require(input.read(&mut buffer[..1]).map_err(|_| SourceClosureError::Codec)? == 0, SourceClosureError::Codec)?;
        self.revalidate(keep)
    }
}
fn validate_entry_stat(entry: &Entry, info: &rustix::fs::Stat) -> Result<()> {
    let kind = if entry.directory { rustix::fs::FileType::Directory } else { rustix::fs::FileType::RegularFile };
    require(rustix::fs::FileType::from_raw_mode(info.st_mode) == kind
        && info.st_uid == entry.uid && info.st_gid == entry.gid && info.st_mode & 0o7777 == entry.mode
        && (entry.directory || (info.st_nlink == 1 && info.st_size >= 0 && info.st_size as u64 == entry.size)),
        SourceClosureError::Metadata)
}

const REFERENCE_TABLES: [&str; 29] = [
    "account_strategy_bindings", "accounts", "data_entitlements",
    "owner_beta_daily_admission_events", "owner_beta_daily_admissions", "owner_beta_daily_calendar_days",
    "owner_beta_daily_calendar_sources", "owner_beta_daily_control", "owner_beta_daily_poll_receipts",
    "owner_beta_daily_poll_state", "owner_beta_daily_publication_claims", "owner_beta_daily_reservations",
    "owner_beta_daily_source_events", "owner_beta_paper_control", "owner_beta_paper_events",
    "owner_beta_paper_lineage", "owner_beta_paper_operation_observations", "owner_beta_paper_operations",
    "owner_beta_paper_recovery_grants", "owner_beta_paper_sessions", "owner_beta_paper_source_admissions",
    "owner_beta_paper_source_originals", "owner_beta_paper_source_runs", "owner_beta_paper_source_slots",
    "owner_beta_recommendation_runs", "owner_beta_simulation_runs", "paper_rebalance_previews",
    "pending_targets", "user_strategy_configs",
];
const DATABASE_GROUPS: [&str; 9] = ["daily", "paper_control", "paper_jobs", "paper_ledger",
    "paper_lineage", "paper_operations", "paper_outbox", "schema_security", "simulation"];

struct References {
    tables: BTreeMap<String, BTreeMap<String, Json>>,
    jobs: BTreeMap<String, Json>,
    attempts: BTreeMap<String, Json>,
    retained_rows: usize,
    retained_bytes: usize,
    limits: SourceClosureLimits,
}
impl References {
    fn new(limits: SourceClosureLimits) -> Self {
        Self { tables: REFERENCE_TABLES.iter().map(|v| ((*v).into(), BTreeMap::new())).collect(),
            jobs: BTreeMap::new(), attempts: BTreeMap::new(), retained_rows: 0, retained_bytes: 0, limits }
    }
    fn charge(&mut self, table: &str, key: &str, row: &Json) -> Result<()> {
        let bytes = row.canonical()?.len().checked_add(table.len()).and_then(|n| n.checked_add(key.len()))
            .and_then(|n| n.checked_add(16)).ok_or(SourceClosureError::Limit)?;
        self.retained_rows = self.retained_rows.checked_add(1).filter(|n| *n <= self.limits.max_reference_rows)
            .ok_or(SourceClosureError::Limit)?;
        self.retained_bytes = self.retained_bytes.checked_add(bytes).filter(|n| *n <= self.limits.max_reference_bytes)
            .ok_or(SourceClosureError::Limit)?;
        Ok(())
    }
    fn insert(&mut self, table: &str, key: &str, row: Json) -> Result<()> {
        self.charge(table, key, &row)?;
        let target = match table {
            "jobs" => &mut self.jobs, "job_attempts" => &mut self.attempts,
            _ => self.tables.get_mut(table).ok_or(SourceClosureError::ReferenceSet)?,
        };
        require(target.insert(key.into(), row).is_none(), SourceClosureError::Duplicate)
    }
    fn rows(&self, table: &str) -> Result<&BTreeMap<String, Json>> {
        self.tables.get(table).ok_or(SourceClosureError::ReferenceSet)
    }
    fn keyed<'a>(&'a self, table: &str, field: &str) -> Result<BTreeMap<String, &'a Json>> {
        let mut result = BTreeMap::new();
        for row in self.rows(table)?.values() {
            let key = row.get(field)?.text()?;
            require(result.insert(key.into(), row).is_none(), SourceClosureError::Duplicate)?;
        }
        Ok(result)
    }
}

fn line(input: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut output = Vec::new();
    loop {
        let data = input.fill_buf().map_err(|_| SourceClosureError::Custody)?;
        if data.is_empty() {
            return if output.is_empty() { Ok(None) } else { Err(SourceClosureError::DatabaseStream) };
        }
        let end = data.iter().position(|b| *b == b'\n');
        let count = end.map_or(data.len(), |i| i + 1);
        require(output.len().checked_add(count).is_some_and(|n| n <= MAX_LINE + 1), SourceClosureError::Limit)?;
        output.extend_from_slice(&data[..count]); input.consume(count);
        if end.is_some() { output.pop(); return Ok(Some(output)); }
    }
}
fn record(input: &mut impl BufRead) -> Result<Json> {
    parse(&line(input)?.ok_or(SourceClosureError::DatabaseStream)?)
}
fn tag(record: &Json, values: &[&str]) -> Result<bool> {
    Ok(record.array()?.len() == values.len() && record.array()?.iter().zip(values)
        .all(|(actual, wanted)| actual.text().ok() == Some(*wanted)))
}

#[cfg(test)]
fn decode_references(input: &mut impl BufRead, limits: SourceClosureLimits) -> Result<References> {
    decode_references_with_rows(input, limits, &mut |_, _, _, _| Ok(()))
}

// The complete database projection is consumed while its actual original is
// held. Only the bounded reference index is retained; ledger/outbox/catalog
// rows must not disappear merely because they are not one of reference29.
fn decode_references_with_rows(
    input: &mut impl BufRead,
    limits: SourceClosureLimits,
    consume: &mut impl FnMut(&str, &str, &str, &[u8]) -> Result<()>,
) -> Result<References> {
    decode_database_stream(input, limits, true, consume)
}

// Re-reading a held stream must not allocate a second complete reference index
// beside the owner's first index. The nonretaining pass still charges exactly
// the same row/byte budgets and validates all duplicated reference roots/EOF.
fn decode_database_stream(
    input: &mut impl BufRead,
    limits: SourceClosureLimits,
    retain: bool,
    consume: &mut impl FnMut(&str, &str, &str, &[u8]) -> Result<()>,
) -> Result<References> {
    let projection = parse(PROJECTION)?;
    let audit = projection.get("archive_audit_v1")?;
    require(audit.get("source_reference_tables")?.array()?.iter().map(Json::text).collect::<Result<Vec<_>>>()?
        == REFERENCE_TABLES, SourceClosureError::ReferenceSet)?;
    let groups = audit.get("paper_source_table_projection")?.object()?;
    require(groups.keys().map(String::as_str).collect::<Vec<_>>() == DATABASE_GROUPS,
        SourceClosureError::DatabaseStream)?;
    let header = record(input)?;
    require(tag(&header, &["header", "owner-beta-paper-db-stream-v1", digest(PROJECTION).as_str()])?,
        SourceClosureError::DatabaseStream)?;
    let mut database_roots: BTreeMap<String, RootValue> = BTreeMap::new();
    let mut refs = References::new(limits);
    let mut all_rows = 0u64;
    for (group, tables) in groups {
        for table in tables.array()? {
            let table = table.text()?; let mut root = FramedRoot::default(); let mut previous: Option<String> = None;
            loop {
                let rec = record(input)?;
                if tag(&rec, &["table_end", group, table])? { break; }
                let values = rec.array()?;
                require(values.len() == 5 && values[0].text()? == "row"
                    && values[1].text()? == group && values[2].text()? == table, SourceClosureError::DatabaseStream)?;
                let key = values[3].text()?;
                require(ascii(key) && previous.as_deref().is_none_or(|prior| prior < key), SourceClosureError::DatabaseStream)?;
                validate_row(table, key, &values[4], audit)?;
                root.add(table, key, &values[4])?;
                all_rows = all_rows.checked_add(1).filter(|n| *n <= MAX_ROWS).ok_or(SourceClosureError::Limit)?;
                consume(group, table, key, &values[4].canonical()?)?;
                if group == "paper_jobs" && (table == "jobs" || table == "job_attempts") {
                    if retain { refs.insert(table, key, values[4].clone())?; }
                    else { refs.charge(table, key, &values[4])?; }
                }
                previous = Some(key.into());
            }
            let value = root.value();
            if let Some(previous) = database_roots.insert(table.into(), value.clone()) {
                require(previous == value, SourceClosureError::ReferenceSet)?;
            }
        }
        require(tag(&record(input)?, &["group_end", group])?, SourceClosureError::DatabaseStream)?;
    }
    let mut reference_roots: BTreeMap<String, FramedRoot> = REFERENCE_TABLES.iter()
        .map(|t| ((*t).into(), FramedRoot::default())).collect();
    let mut previous: Option<(String, String)> = None;
    loop {
        let rec = record(input)?;
        if tag(&rec, &["references_end"])? { break; }
        let values = rec.array()?;
        require(values.len() == 4 && values[0].text()? == "reference", SourceClosureError::DatabaseStream)?;
        let table = values[1].text()?; let key = values[2].text()?;
        let identity = (table.to_owned(), key.to_owned());
        require(ascii(key) && previous.as_ref().is_none_or(|prior| prior < &identity), SourceClosureError::DatabaseStream)?;
        let root = reference_roots.get_mut(table).ok_or(SourceClosureError::ReferenceSet)?;
        validate_row(table, key, &values[3], audit)?;
        root.add(table, key, &values[3])?;
        if retain { refs.insert(table, key, values[3].clone())?; }
        else { refs.charge(table, key, &values[3])?; }
        previous = Some(identity);
    }
    for (table, root) in &reference_roots {
        require(database_roots.get(table) == Some(&root.value()), SourceClosureError::ReferenceSet)?;
    }
    require(tag(&record(input)?, &["end"])? && line(input)?.is_none(), SourceClosureError::DatabaseStream)?;
    Ok(refs)
}

fn validate_row(table: &str, key: &str, row: &Json, audit: &Json) -> Result<()> {
    let catalog = audit.get("catalog_projection")?.object()?;
    let columns = if let Some(value) = catalog.get(table) { value.get("columns")? }
        else { audit.get("archive_table_columns")?.get(table)? };
    let expected = columns.array()?.iter().map(Json::text).collect::<Result<Vec<_>>>()?;
    row.keys(&expected).map_err(|_| SourceClosureError::DatabaseRow)?;
    if catalog.contains_key(table) { return Ok(()); }
    for (column, kind) in audit.get("archive_column_types")?.get(table)?.object()? {
        let value = row.get(column)?; if value == &Json::Null { continue; }
        if table == "paper_rebalance_previews" && column == "error_json" {
            value.keys(&["sha256"])?; require(sha(value.get("sha256")?.text()?), SourceClosureError::DatabaseRow)?;
        } else if ["error_message", "error_detail", "last_error"].contains(&column.as_str()) {
            require(sha(value.text()?), SourceClosureError::DatabaseRow)?;
        } else {
            match kind.text()? {
                "int2" => { i16::try_from(value.integer()?).map_err(|_| SourceClosureError::DatabaseRow)?; }
                "int4" => { i32::try_from(value.integer()?).map_err(|_| SourceClosureError::DatabaseRow)?; }
                "int8" => { value.integer()?; }
                "bool" => require(matches!(value, Json::Bool(_)), SourceClosureError::DatabaseRow)?,
                "uuid" => {
                    let text = value.text()?;
                    require(uuid::Uuid::parse_str(text).is_ok_and(|v| v.to_string() == text), SourceClosureError::DatabaseRow)?;
                }
                "date" => {
                    let text = value.text()?;
                    require(text.len() == 10 && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                        .is_ok_and(|date| date.to_string() == text), SourceClosureError::DatabaseRow)?;
                }
                "timestamptz" => {
                    let text = value.text()?;
                    require(text.len() == 27 && text.as_bytes()[10] == b'T' && text.as_bytes()[19] == b'.'
                        && text.ends_with('Z') && text.as_bytes()[20..26].iter().all(u8::is_ascii_digit)
                        && chrono::DateTime::parse_from_rfc3339(text).is_ok(), SourceClosureError::DatabaseRow)?;
                }
                "bytea" => { let text = value.text()?; require(text.len() % 2 == 0 && lower_hex(text), SourceClosureError::DatabaseRow)?; }
                "text" | "varchar" => { value.text()?; }
                "numeric" if ["accounts", "cash_ledger", "daily_equity", "fills", "orders", "positions"].contains(&table) => {
                    require(fixed4(value.text()?), SourceClosureError::DatabaseRow)?;
                }
                "numeric" => require(matches!(value, Json::Number(_)), SourceClosureError::DatabaseRow)?,
                "jsonb" => (),
                _ => return Err(SourceClosureError::DatabaseRow),
            }
        }
    }
    let mut parts = Vec::new();
    for component in audit.get("archive_identity_key_components")?.get(table)?.array()? {
        let component = component.text()?;
        let mut text = match row.get(component)? {
            Json::String(v) | Json::Number(v) => v.to_owned(), _ => return Err(SourceClosureError::DatabaseRow),
        };
        if table == "owner_beta_paper_session_events" && component == "event_seq" {
            require(!text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()), SourceClosureError::DatabaseRow)?;
            text = format!("{text:0>20}");
        }
        parts.push(text);
    }
    require(parts.join(":") == key, SourceClosureError::DatabaseRow)?;
    receipt(row)
}
fn lower_hex(text: &str) -> bool { text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn fixed4(text: &str) -> bool {
    let text = text.strip_prefix('-').unwrap_or(text);
    text.split_once('.').is_some_and(|(whole, part)| !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit())
        && (whole.len() == 1 || !whole.starts_with('0')) && part.len() == 4 && part.bytes().all(|b| b.is_ascii_digit()))
}
fn receipt(row: &Json) -> Result<()> {
    if !row.object()?.contains_key("request_canonical") { return Ok(()); }
    let fields = ["request_canonical", "request_sha256", "result_canonical", "result_sha256"];
    let values = fields.iter().map(|name| row.get(name)).collect::<Result<Vec<_>>>()?;
    if values.iter().all(|v| **v == Json::Null) { return Ok(()); }
    for pair in values.chunks_exact(2) {
        let bytes = pair[0].text()?.as_bytes(); let hash = pair[1].text()?;
        require(bytes.len() <= 1_048_576 && ascii(pair[0].text()?) && digest(bytes) == hash
            && parse(bytes)?.canonical()? == bytes, SourceClosureError::DatabaseRow)?;
    }
    Ok(())
}

fn equal_fields(left: &Json, right: &Json, fields: &[(&str, &str)]) -> Result<()> {
    for (a, b) in fields { require(left.get(a)? == right.get(b)?, SourceClosureError::Relationship)?; }
    Ok(())
}
fn linked<'a>(map: &'a BTreeMap<String, &'a Json>, row: &Json, field: &str) -> Result<&'a Json> {
    map.get(row.get(field)?.text()?).copied().ok_or(SourceClosureError::Relationship)
}

/// Cross-row relationships precede every domain callback. Historical rights and
/// policy tuples remain each row's own values; nothing is compared to "today".
fn relationships(refs: &References) -> Result<()> {
    let accounts = refs.keyed("accounts", "id")?;
    let configs = refs.keyed("user_strategy_configs", "id")?;
    let events = refs.keyed("owner_beta_paper_events", "id")?;
    let lineages = refs.keyed("owner_beta_paper_lineage", "id")?;
    let targets = refs.keyed("pending_targets", "id")?;
    let admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    for row in refs.rows("account_strategy_bindings")?.values() {
        let account = linked(&accounts, row, "account_id")?;
        let config = linked(&configs, row, "strategy_config_id")?;
        equal_fields(row, account, &[("owner_user_id", "owner_user_id")])?;
        equal_fields(row, config, &[("owner_user_id", "owner_user_id"), ("strategy_id", "strategy_id"),
            ("strategy_version", "strategy_version")])?;
    }
    for table in ["owner_beta_paper_control", "owner_beta_paper_events", "owner_beta_paper_lineage",
        "owner_beta_paper_sessions", "pending_targets", "paper_rebalance_previews"] {
        for row in refs.rows(table)?.values() {
            let account = linked(&accounts, row, "account_id")?;
            require(account.get("account_type")?.text()? == "PAPER", SourceClosureError::Relationship)?;
            equal_fields(row, account, &[("owner_user_id", "owner_user_id")])?;
            if row.object()?.contains_key("strategy_config_id") {
                equal_fields(row, linked(&configs, row, "strategy_config_id")?, &[("owner_user_id", "owner_user_id")])?;
            }
        }
    }
    for row in refs.rows("owner_beta_paper_control")?.values() {
        equal_fields(row, linked(&accounts, row, "account_id")?, &[("cost_profile_id", "cost_profile_id"),
            ("cost_profile_version", "cost_profile_version")])?;
        for field in ["consent_event_id", "activation_event_id"] {
            if row.get(field)? != &Json::Null {
                equal_fields(row, linked(&events, row, field)?, &[("account_id", "account_id"), ("owner_user_id", "owner_user_id")])?;
            }
        }
    }
    for row in refs.rows("owner_beta_paper_operations")?.values() { linked(&accounts, row, "account_id")?; }
    for row in refs.rows("owner_beta_paper_sessions")?.values() {
        equal_fields(row, linked(&targets, row, "pending_target_id")?, &[("account_id", "account_id"),
            ("owner_user_id", "owner_user_id"), ("effective_session", "effective_date")])?;
        for field in ["execution_admission_sha256", "valuation_admission_sha256"] {
            if row.get(field)? != &Json::Null { linked(&admissions, row, field)?; }
        }
    }
    for row in refs.rows("owner_beta_paper_recovery_grants")?.values() {
        let lineage = linked(&lineages, row, "lineage_id")?;
        let target = linked(&targets, row, "pending_target_id")?;
        equal_fields(row, lineage, &[("account_id", "account_id"), ("effective_session", "effective_session")])?;
        equal_fields(row, target, &[("account_id", "account_id"), ("lineage_id", "beta_lineage_id")])?;
        for field in ["signal_admission_sha256", "execution_admission_sha256"] {
            if row.get(field)? != &Json::Null { linked(&admissions, row, field)?; }
        }
    }
    for row in refs.rows("data_entitlements")?.values() {
        require(sha(row.get("contract_document_sha256")?.text()?) && !row.get("contract_reference")?.text()?.is_empty(),
            SourceClosureError::Relationship)?;
        // EXPIRED/REVOKED remain unchanged descriptive history. This is never
        // the current data-rights/publication decision.
    }
    for rows in refs.tables.values() {
        for row in rows.values() {
            if let Some(id) = row.object()?.get("job_id").filter(|v| **v != Json::Null) {
                let job = refs.jobs.get(id.text()?).ok_or(SourceClosureError::Relationship)?;
                if row.object()?.contains_key("owner_user_id") {
                    equal_fields(row, job, &[("owner_user_id", "owner_user_id")])?;
                }
                for field in ["attempt_id", "published_attempt_id"] {
                    if let Some(attempt_id) = row.object()?.get(field).filter(|v| **v != Json::Null) {
                        let attempt = refs.attempts.get(attempt_id.text()?).ok_or(SourceClosureError::Relationship)?;
                        require(attempt.get("job_id")? == id, SourceClosureError::Relationship)?;
                    }
                }
            }
        }
    }
    for attempt in refs.attempts.values() {
        require(refs.jobs.contains_key(attempt.get("job_id")?.text()?), SourceClosureError::Relationship)?;
    }
    let slots = refs.keyed("owner_beta_paper_source_slots", "slot_id")?;
    let source_admissions = refs.keyed("owner_beta_paper_source_admissions", "slot_id")?;
    let runs = refs.keyed("owner_beta_recommendation_runs", "id")?;
    let reservations = refs.keyed("owner_beta_daily_reservations", "run_id")?;
    let claims = refs.keyed("owner_beta_daily_publication_claims", "run_id")?;
    for row in source_admissions.values() {
        let slot = linked(&slots, row, "slot_id")?;
        let admission = linked(&admissions, row, "admission_sha256")?;
        equal_fields(slot, admission, &[("owner_user_id", "owner_user_id"), ("policy_sha256", "policy_sha256"),
            ("session_date", "range_end")])?;
    }
    for row in refs.rows("owner_beta_paper_source_runs")?.values() {
        let slot = linked(&slots, row, "slot_id")?;
        let admitted = linked(&source_admissions, row, "slot_id")?;
        let run = linked(&runs, row, "run_id")?;
        let reservation = linked(&reservations, row, "run_id")?;
        let claim = linked(&claims, row, "run_id")?;
        let job = refs.jobs.get(row.get("job_id")?.text()?).ok_or(SourceClosureError::Relationship)?;
        let attempt = refs.attempts.get(row.get("attempt_id")?.text()?).ok_or(SourceClosureError::Relationship)?;
        for member in [run, reservation, claim, attempt] { equal_fields(row, member, &[("job_id", "job_id")])?; }
        equal_fields(row, claim, &[("attempt_id", "attempt_id")])?;
        equal_fields(attempt, claim, &[("attempt_no", "attempt_no"), ("claimed_by", "claimed_by")])?;
        for member in [run, reservation, claim, job] { equal_fields(slot, member, &[("owner_user_id", "owner_user_id")])?; }
        equal_fields(run, reservation, &[("strategy_config_id", "strategy_config_id"), ("as_of", "session_date")])?;
        equal_fields(slot, reservation, &[("session_date", "session_date"), ("activation_epoch", "activation_epoch")])?;
        equal_fields(admitted, reservation, &[("admission_sha256", "admission_sha256")])?;
    }
    for target in targets.values() {
        if !["OWNER_BETA_DAILY_MANUAL", "OWNER_BETA_DAILY_AUTO"].contains(&target.get("source_kind")?.text()?) { continue; }
        let lineage = linked(&lineages, target, "beta_lineage_id")?;
        require(["recommendation_run_id", "dataset_version", "dataset_version_id", "dataset_manifest_sha256"]
            .iter().all(|key| target.get(key).is_ok_and(|v| v == &Json::Null)), SourceClosureError::Relationship)?;
        equal_fields(target, lineage, &[("owner_user_id", "owner_user_id"), ("account_id", "account_id"),
            ("strategy_config_id", "strategy_config_id"), ("computed_on", "signal_date"),
            ("effective_date", "effective_session"), ("targets_json", "target_weights_json")])?;
        let run = linked(&runs, lineage, "recommendation_run_id")?;
        require(run.get("status")?.text()? == "SUCCEEDED" && run.get("trigger_kind")?.text()? == "DAILY",
            SourceClosureError::Relationship)?;
    }
    Ok(())
}

fn publication_provider(name: &str) -> Result<crate::daily_price_only::source::PublicationRawProvider> {
    use crate::daily_price_only::source::PublicationRawProvider;
    match name {
        "krx" => Ok(PublicationRawProvider::Krx),
        "kis-normalized" => Ok(PublicationRawProvider::Normalized),
        "kis-calendar-normalized" => Ok(PublicationRawProvider::Calendar),
        _ => Err(SourceClosureError::Member),
    }
}

fn raw_scope_names(tree: &Materialized) -> Result<Vec<&str>> {
    let mut result = BTreeSet::new();
    for entry in &tree.manifest.entries {
        if !entry.path.starts_with("raw/") { continue; }
        let parts: Vec<_> = entry.path.split('/').collect();
        if parts.len() == 5 && parts[1] == "manifests" && parts[4] == "manifest.jsonl" {
            require(parts[3] == "market=kr", SourceClosureError::Member)?;
            let provider = parts[2].strip_prefix("provider=").ok_or(SourceClosureError::Member)?;
            require(super::historical_provider(provider).is_ok() || publication_provider(provider).is_ok(), SourceClosureError::Member)?;
            result.insert(provider);
        }
    }
    Ok(result.into_iter().collect())
}

fn raw_membership(
    tree: &Materialized,
    raw: &super::HistoricalRawReader<'_, '_, '_>,
    refs: &References,
) -> Result<()> {
    let mut covered = BTreeSet::from(["raw".to_owned()]);
    let slots = refs.keyed("owner_beta_paper_source_slots", "slot_id")?;
    let admitted = refs.keyed("owner_beta_paper_source_admissions", "slot_id")?;
    let mut originals: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for source in raw.sources {
        let view = source.view_in(raw.expected, raw.captured).map_err(|_| SourceClosureError::Custody)?;
        let scope = view.scope().ok_or(SourceClosureError::Custody)?;
        let provider = scope.provider();
        let path = format!("raw/manifests/provider={provider}/market=kr/manifest.jsonl");
        let bytes = view.manifest_bytes().ok_or(SourceClosureError::Custody)?;
        require(tree.read(&path, MAX_LINE)? == bytes, SourceClosureError::Member)?;
        covered.insert(path);
        for batch in view.batches() {
            let prefix = format!("raw/provider={provider}/market=kr/date={}/batch={}", batch.entry().date, batch.batch_id());
            let path = format!("{prefix}/batch.json");
            require(tree.read(&path, MAX_LINE)? == batch.batch_json_bytes(), SourceClosureError::Member)?;
            covered.insert(path);
            for file in batch.files() {
                let path = format!("{prefix}/{}", file.file_name());
                let i = *tree.manifest.by_path.get(&path).ok_or(SourceClosureError::Member)?;
                let entry = &tree.manifest.entries[i];
                require(!entry.directory && entry.size == file.bytes().len() as u64
                    && entry.sha256.as_deref() == Some(file.content_hash().as_str())
                    && ContentHash::from_bytes(file.bytes()) == *file.content_hash(), SourceClosureError::Member)?;
                tree.check_node(i)?; covered.insert(path);
            }
        }
    }
    let leaves: Vec<_> = covered.iter().cloned().collect();
    for path in leaves {
        let parts: Vec<_> = path.split('/').collect();
        for n in 1..parts.len() { covered.insert(parts[..n].join("/")); }
    }
    let actual: BTreeSet<_> = tree.manifest.entries.iter().filter(|entry| entry.path == "raw" || entry.path.starts_with("raw/"))
        .map(|entry| entry.path.clone()).collect();
    require(covered == actual, SourceClosureError::Inventory)?;
    raw.verify_copies().map_err(|_| SourceClosureError::Reconstruction)?;
    for row in refs.rows("owner_beta_paper_source_originals")?.values() {
        let slot = linked(&slots, row, "slot_id")?;
        linked(&admitted, row, "slot_id")?;
        let provider = row.get("provider")?.text()?;
        let view = raw.scope(provider, row.get("market")?.text()?).map_err(|_| SourceClosureError::Member)?;
        let id = row.get("batch_id")?.text()?;
        let batch = view.batches().find(|b| b.batch_id().to_string() == id).ok_or(SourceClosureError::Member)?;
        require(batch.entry().date.to_string() == slot.get("session_date")?.text()?, SourceClosureError::Relationship)?;
        let name = row.get("file_name")?.text()?;
        let file = batch.files().find(|f| f.file_name() == name).ok_or(SourceClosureError::Member)?;
        require(file.content_hash().as_str() == row.get("wire_sha256")?.text()?, SourceClosureError::Relationship)?;
        originals.entry(row.get("slot_id")?.text()?.into()).or_default().push(row.serde_value()?);
    }
    for (slot, admission) in admitted {
        let originals = originals.get(&slot).ok_or(SourceClosureError::Relationship)?;
        let hash = crate::paper_source_evidence::original_set_hash(originals).map_err(|_| SourceClosureError::Relationship)?;
        require(hash == admission.get("original_set_sha256")?.text()?, SourceClosureError::Relationship)?;
    }
    Ok(())
}

const HISTORICAL_PIN_FIELDS: [&str; 5] = ["candidate_content_sha256", "artifact_manifest_sha256",
    "stage5_manifest_sha256", "action_manifest_sha256", "approval_registry_sha256"];
const V2_REGISTRY: &str = "kis-historical-price-only-beta-approved-artifacts.json";
const V3_REGISTRY: &str = "kis-historical-price-only-v3-approved-artifacts.json";

struct HistoricalSelection {
    version: super::HistoricalArchiveVersion,
    class: &'static str,
    prefix: String,
    registry: &'static str,
}
fn historical_selections(tree: &Materialized, refs: &References) -> Result<Vec<HistoricalSelection>> {
    use super::HistoricalArchiveVersion::{V2, V3};
    let v2 = crate::historical_price_only_approval::embedded_v2_archive_pins().map_err(|_| SourceClosureError::HistoricalSelection)?;
    let v3 = crate::historical_price_only_approval::embedded_v3_archive_pins().map_err(|_| SourceClosureError::HistoricalSelection)?;
    let pins = [(V2, &v2, V2_REGISTRY), (V3, &v3, V3_REGISTRY)];
    let mut needed = BTreeSet::new();
    for table in ["owner_beta_recommendation_runs", "owner_beta_simulation_runs"] {
        for row in refs.rows(table)?.values() {
            let registry = row.get("approval_registry_sha256")?.text()?;
            let (version, pin, name) = pins.iter().find(|(_, pin, _)| pin.approval_registry_sha256().as_str() == registry)
                .ok_or(SourceClosureError::HistoricalSelection)?;
            let hashes = [pin.candidate_content_sha256(), pin.artifact_manifest_sha256(), pin.stage5_manifest_sha256(),
                pin.action_manifest_sha256(), pin.approval_registry_sha256()];
            for (field, hash) in HISTORICAL_PIN_FIELDS.iter().zip(hashes) {
                require(row.get(field)?.text()? == hash.as_str(), SourceClosureError::HistoricalSelection)?;
            }
            if table == "owner_beta_simulation_runs" { require(*version == V3, SourceClosureError::HistoricalSelection)?; }
            needed.insert(*name);
        }
    }
    let mut result = Vec::new(); let mut allowed_artifacts = BTreeSet::from(["base".to_owned(), "simulation_input".to_owned()]);
    for (version, pin, registry) in pins {
        let registry_path = format!("simulation_approval/{registry}");
        let candidate = pin.candidate_content_sha256().as_str().strip_prefix("sha256:").ok_or(SourceClosureError::HistoricalSelection)?;
        let relative = if version == V2 { format!("kis-historical-price-only-beta/v2/candidate-sha256={candidate}") }
            else { crate::historical_price_only_v3_artifact::candidate_directory_name(pin.candidate_content_sha256())
                .map_err(|_| SourceClosureError::HistoricalSelection)? };
        let mut present = false;
        for class in ["base", "simulation_input"] {
            let prefix = format!("{class}/{relative}");
            if !tree.manifest.by_path.contains_key(&prefix) { continue; }
            present = true;
            for leaf in ["manifest.json", "bars.ndjson"] {
                let path = format!("{prefix}/{leaf}");
                require(tree.manifest.by_path.get(&path).is_some_and(|i| !tree.manifest.entries[*i].directory),
                    SourceClosureError::HistoricalSelection)?;
                allowed_artifacts.insert(path);
            }
            let parts: Vec<_> = prefix.split('/').collect();
            for n in 1..=parts.len() { allowed_artifacts.insert(parts[..n].join("/")); }
            result.push(HistoricalSelection { version, class, prefix, registry });
        }
        if needed.contains(registry) || present || tree.manifest.by_path.contains_key(&registry_path) {
            require(tree.manifest.by_path.contains_key(&registry_path), if version == V2 {
                SourceClosureError::HistoricalV2RegistryMissing
            } else { SourceClosureError::HistoricalV3RegistryMissing })?;
            require(present, if version == V2 { SourceClosureError::HistoricalV2ArtifactMissing }
                else { SourceClosureError::HistoricalV3ArtifactMissing })?;
            require(digest(&tree.read(&registry_path, 262_144)?) == pin.approval_registry_sha256().as_str(),
                SourceClosureError::HistoricalSelection)?;
        }
    }
    for entry in &tree.manifest.entries {
        if entry.path == "base" || entry.path.starts_with("base/") || entry.path == "simulation_input"
            || entry.path.starts_with("simulation_input/") {
            require(allowed_artifacts.contains(&entry.path), SourceClosureError::HistoricalSelection)?;
        }
        if entry.path.starts_with("simulation_approval/") {
            require(entry.path == format!("simulation_approval/{V2_REGISTRY}")
                || entry.path == format!("simulation_approval/{V3_REGISTRY}"), SourceClosureError::HistoricalSelection)?;
        }
    }
    if result.iter().any(|selected| selected.version == V2) {
        // The adopted V2 archive roots overlap/alias by source contract. Preserve
        // both logical classes and their exact metadata/bytes, not just a pin.
        let mut aliases = Vec::new();
        for class in ["base", "simulation_input"] {
            let prefix = format!("{class}/");
            let mut records = BTreeMap::new();
            for entry in &tree.manifest.entries {
                if let Some(relative) = entry.path.strip_prefix(&prefix) {
                    let mut row = entry.row.object()?.clone(); row.remove("path");
                    records.insert(relative.to_owned(), row);
                }
            }
            aliases.push(records);
        }
        require(aliases[0] == aliases[1], SourceClosureError::HistoricalSelection)?;
    }
    Ok(result)
}

fn exact_metadata(tree: &Materialized, path: &str, uid: u32, gid: u32, mode: u32) -> Result<()> {
    let index = *tree.manifest.by_path.get(path).ok_or(SourceClosureError::Member)?;
    let row = &tree.manifest.entries[index];
    require(!row.directory && (row.uid, row.gid, row.mode) == (uid, gid, mode), SourceClosureError::Metadata)?;
    tree.check_node(index)
}

/// Reuses the actual source chain algorithm on complete retained event originals.
/// A current envelope remains a historical observation, never a live OFD fence.
fn source_chains(tree: &Materialized, refs: &References, raw: &super::HistoricalRawReader<'_, '_, '_>) -> Result<()> {
    let mut currents = BTreeMap::new();
    let mut chains: BTreeMap<(String, String), BTreeMap<usize, String>> = BTreeMap::new();
    let mut allowed = BTreeSet::from(["paper_evidence".to_owned()]);
    for entry in &tree.manifest.entries {
        if !entry.path.starts_with("paper_evidence/") || entry.directory { continue; }
        let parts: Vec<_> = entry.path.split('/').collect();
        if parts.len() == 2 && parts[1].starts_with("current-") && parts[1].ends_with(".json") {
            let ordinal = &parts[1][8..parts[1].len() - 5];
            require(ordinal.len() == 4 && ordinal.bytes().all(|b| b.is_ascii_digit()), SourceClosureError::Member)?;
            exact_metadata(tree, &entry.path, 0, 10001, 0o440)?;
            let bytes = tree.read(&entry.path, 65_536)?;
            let current = crate::paper_source_evidence::decode_current_envelope(&bytes).map_err(|_| SourceClosureError::Reconstruction)?;
            let binding = current.value().get("binding").ok_or(SourceClosureError::Reconstruction)?;
            // An unbound current selector is retained history as well. Its
            // null binding cannot authorize a source run, and cannot erase
            // earlier chains from this same physical source root.
            require(currents.insert(format!("source-{ordinal}"), binding.clone()).is_none(), SourceClosureError::Duplicate)?;
            allowed.insert(entry.path.clone());
        } else {
            require(parts.len() == 5 && parts[1].starts_with("source-") && parts[1].len() == 11
                && parts[1][7..].bytes().all(|b| b.is_ascii_digit()) && parts[2] == "events"
                && parts[3].len() == 64 && lower_hex(parts[3]) && parts[4].len() == 11
                && parts[4].ends_with(".json") && parts[4][..6].bytes().all(|b| b.is_ascii_digit()),
                SourceClosureError::Member)?;
            let sequence: usize = parts[4][..6].parse().map_err(|_| SourceClosureError::Member)?;
            require(sequence < 64, SourceClosureError::Limit)?;
            require(chains.entry((parts[1].into(), parts[3].into())).or_default()
                .insert(sequence, entry.path.clone()).is_none(), SourceClosureError::Duplicate)?;
            for n in 1..=parts.len() { allowed.insert(parts[..n].join("/")); }
        }
    }
    for source in currents.keys() {
        allowed.insert(format!("paper_evidence/{source}"));
        allowed.insert(format!("paper_evidence/{source}/events"));
    }
    let slots = refs.keyed("owner_beta_paper_source_slots", "slot_id")?;
    let admissions = refs.keyed("owner_beta_paper_source_admissions", "slot_id")?;
    let runs = refs.keyed("owner_beta_paper_source_runs", "slot_id")?;
    let mut seen_slots = BTreeSet::new();
    for ((source, slot), names) in chains {
        let binding = currents.get(&source).ok_or(SourceClosureError::Relationship)?;
        require(!names.is_empty() && names.keys().copied().eq(0..names.len()), SourceClosureError::Inventory)?;
        let mut events = Vec::new();
        for path in names.values() {
            exact_metadata(tree, path, 0, 10001, 0o440)?;
            let bytes = tree.read(path, crate::paper_source_evidence::SOURCE_LIMIT)?;
            let event = crate::paper_source_evidence::decode_canonical(&bytes, crate::paper_source_evidence::SOURCE_LIMIT)
                .map_err(|_| SourceClosureError::Reconstruction)?;
            require(event["slot_id"].as_str() == Some(format!("sha256:{slot}").as_str()), SourceClosureError::Relationship)?;
            events.push(event);
        }
        crate::paper_source_evidence::verify_chain(&events, None).map_err(|_| SourceClosureError::Reconstruction)?;
        for event in &events {
            if event["kind"] == "SOURCE_FINISHED" {
                for original in event["data"]["originals"].as_array().ok_or(SourceClosureError::Reconstruction)? {
                    source_original_matches(raw, original)?;
                }
            }
        }
        let first = events.first().ok_or(SourceClosureError::Inventory)?;
        // Do not replace a chain's historical cohort with the current selector.
        // The source binding selects a physical root only; every archived slot
        // must match its original cohort and canonical RESERVED root event.
        let _physical_root_selector = binding;
        let key = format!("sha256:{slot}");
        let reserved = events.iter().find(|event| event["kind"] == "DB_RESERVED");
        if let Some(row) = slots.get(&key) {
            let root_event = events.iter().find(|event| event["kind"] == "RESERVED").ok_or(SourceClosureError::Relationship)?;
            require(seen_slots.insert(key.clone()), SourceClosureError::Relationship)?;
            let row_value = row.serde_value()?;
            require(first["cohort"] == row_value["cohort"]
                && first["reservation_id"] == row_value["reservation_id"]
                && crate::paper_source_evidence::digest(&crate::paper_source_evidence::canonical(root_event)
                    .map_err(|_| SourceClosureError::Reconstruction)?) == row.get("root_event_sha256")?.text()?,
                SourceClosureError::Relationship)?;
            for field in ["owner_user_id", "policy_sha256", "activation_epoch", "cohort_sha256"] {
                require(first["cohort"][field] == row_value[field], SourceClosureError::Relationship)?;
            }
            require(first["session"] == row_value["session_date"], SourceClosureError::Relationship)?;
            if let Some(admission) = admissions.get(&key) {
                let event = events.iter().find(|event| event["kind"] == "ADMISSION_LINKED").ok_or(SourceClosureError::Relationship)?;
                let original = admission.serde_value()?;
                for field in ["admission_sha256", "original_set_sha256", "source_event_sha256"] {
                    require(event["data"][field] == original[field], SourceClosureError::Relationship)?;
                }
            }
            if let Some(run) = runs.get(&key) {
                let event = events.iter().find(|event| event["kind"] == "RUN_LINKED").ok_or(SourceClosureError::Relationship)?;
                let original = run.serde_value()?;
                for field in ["run_id", "job_id", "attempt_id"] {
                    require(event["data"][field] == original[field], SourceClosureError::Relationship)?;
                }
            }
        } else {
            // An undispatched/failed chain may be retained history, but a DB
            // reservation or later link cannot disappear from the audit rows.
            require(reserved.is_none() && !events.iter().any(|e| ["ADMISSION_LINKED", "RUN_LINKED", "LINKED"]
                .iter().any(|kind| e["kind"] == *kind)), SourceClosureError::Relationship)?;
        }
    }
    require(seen_slots.len() == slots.len(), SourceClosureError::Relationship)?;
    for entry in &tree.manifest.entries {
        if entry.path == "paper_evidence" || entry.path.starts_with("paper_evidence/") {
            require(allowed.contains(&entry.path), SourceClosureError::Inventory)?;
        }
    }
    Ok(())
}

fn source_original_matches(raw: &super::HistoricalRawReader<'_, '_, '_>, original: &serde_json::Value) -> Result<()> {
    let text = |key: &str| original[key].as_str().ok_or(SourceClosureError::Reconstruction);
    let view = raw.scope(text("provider")?, text("market")?).map_err(|_| SourceClosureError::Member)?;
    let batch = view.batches().find(|b| b.batch_id().to_string() == original["batch_id"].as_str().unwrap_or(""))
        .ok_or(SourceClosureError::Member)?;
    let file = batch.files().find(|f| f.file_name() == original["file_name"].as_str().unwrap_or(""))
        .ok_or(SourceClosureError::Member)?;
    require(["kis", "kis-calendar"].contains(&text("provider")?) && file.entry().copied_from.is_none()
        && [crate::contract::ResponseKind::Bars, crate::contract::ResponseKind::Reference,
            crate::contract::ResponseKind::Calendar].contains(&file.kind()), SourceClosureError::Reconstruction)?;
    let retrieval: domain::UtcTimestamp = serde_json::from_value(original["retrieved_at"].clone())
        .map_err(|_| SourceClosureError::Reconstruction)?;
    require(batch.batch_date().to_string() == text("batch_date")?
        && batch.retrieved_at() == retrieval
        && batch.batch_json_sha256().as_str() == text("batch_json_sha256")?
        && file.content_hash().as_str() == text("wire_sha256")?
        && original["size_bytes"].as_u64() == Some(file.size_bytes()), SourceClosureError::Member)?;
    let pin = ContentHash::parse(text("manifest_line_sha256")?).map_err(|_| SourceClosureError::Reconstruction)?;
    // paper-evidence.py::raw_originals uses splitlines(): the source-event pin
    // excludes LF, unlike V3's adopted source pins. Keep the exact original
    // physical bytes; do not substitute serde_json::to_vec(ManifestEntry).
    let manifest = view.manifest_bytes().ok_or(SourceClosureError::Custody)?;
    let mut found = false;
    for line in manifest.split_inclusive(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if ContentHash::from_bytes(line) == pin {
            let entry: crate::storage::ManifestEntry = serde_json::from_slice(line).map_err(|_| SourceClosureError::Reconstruction)?;
            require(&entry == batch.entry(), SourceClosureError::Member)?; found = true;
        }
    }
    require(found, SourceClosureError::Member)?;
    Ok(())
}

fn daily_database_binding(row: &Json, admission_hash: &str, admission: &Json, manifest: &Json) -> Result<()> {
    require(row.get("admission_sha256")?.text()? == admission_hash, SourceClosureError::Relationship)?;
    equal_fields(row, admission, &[("content_sha256", "content_sha256"), ("manifest_sha256", "manifest_sha256"),
        ("policy_sha256", "policy_sha256"), ("parent_admission_sha256", "parent_admission_sha256")])?;
    equal_fields(row, manifest, &[("range_start", "range_start"), ("range_end", "range_end")])?;
    let evidence = row.get("evidence_json")?;
    require(evidence.get("admission")? == admission && evidence.get("manifest")? == manifest
        && evidence.get("admission_sha256")?.text()? == admission_hash, SourceClosureError::Relationship)?;
    require(row.get("session_count")?.integer()? == manifest.get("sessions")?.array()?.len() as i64
        && row.get("bar_count")? == manifest.get("bars")?.get("row_count")?, SourceClosureError::Relationship)
}

/// Uses the existing checked admission loader, with the installed admitter
/// class's UID/GID rather than verifier euid. This closes original membership
/// and SQL preimages only; it does NOT replace independent Raw reconstruction.
#[expect(clippy::too_many_arguments, reason = "Actual root/invocation and both scope endpoints remain explicit.")]
fn daily_bundle_membership(
    tree: &Materialized, refs: &References, materialized: &Path,
    invocation: &RawInvocation<'_>, expected: &Attempt, captured: &Attempt,
    keep: &mut impl FnMut() -> bool,
) -> Result<()> {
    use crate::daily_price_only::artifact::CheckedDailyAdmissionRoot;
    let rows = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    let mut leaves = BTreeSet::new();
    for entry in &tree.manifest.entries {
        if let Some(relative) = entry.path.strip_prefix("daily/admitted/") {
            let parts: Vec<_> = relative.split('/').collect();
            require((1..=2).contains(&parts.len()) && parts[0].len() == 64 && lower_hex(parts[0])
                && (parts.len() == 1 || ["admission.json", "manifest.json", "bars.ndjson", "normalization.json"].contains(&parts[1])),
                SourceClosureError::Inventory)?;
            leaves.insert(parts[0].to_owned());
        }
    }
    require(rows.keys().all(|hash| hash.strip_prefix("sha256:").is_some_and(|h| leaves.contains(h))),
        SourceClosureError::Member)?;
    if leaves.is_empty() { return Ok(()); }
    // The current source namespace retains exactly this policy original. An
    // older policy must not be synthesized from SQL JSON or replaced by today's
    // bytes; the selector still needs a source-owned historical policy seam.
    require(tree.manifest.by_path.contains_key("config/policy.json"), SourceClosureError::DailyPolicyOriginalMissing)?;
    exact_metadata(tree, "config/policy.json", 0, 10001, 0o440)?;
    let policy_bytes = tree.read("config/policy.json", 262_144)?;
    let policy_hash = ContentHash::from_bytes(&policy_bytes);
    let policy = crate::daily_price_only::admission::PinnedDailyPolicy::from_pinned_bytes(&policy_bytes, &policy_hash)
        .map_err(|_| SourceClosureError::Reconstruction)?;
    let root = CheckedDailyAdmissionRoot::open_in(invocation, expected, captured,
        &materialized.join("daily/admitted"), 10003, 10001).map_err(|_| SourceClosureError::Custody)?;
    let body = (|| {
        for leaf in &leaves {
            require(keep(), SourceClosureError::Interrupted)?;
            let prefix = format!("daily/admitted/{leaf}");
            for file in ["admission.json", "manifest.json", "bars.ndjson", "normalization.json"] {
                exact_metadata(tree, &format!("{prefix}/{file}"), 10003, 10001, 0o440)?;
            }
            let admission_bytes = tree.read(&format!("{prefix}/admission.json"), 1_048_576)?;
            let admission = crate::daily_price_only::parse_daily_price_admission_v1(&admission_bytes)
                .map_err(|_| SourceClosureError::Reconstruction)?;
            require(admission.policy_sha256 == *policy.sha256(), SourceClosureError::DailyPolicyOriginalMissing)?;
            let hash = ContentHash::parse(&format!("sha256:{leaf}")).map_err(|_| SourceClosureError::Member)?;
            let files = root.read_files_in(expected, captured, &hash, &admission.content_sha256)
                .map_err(|_| SourceClosureError::Reconstruction)?;
            let used = (|| {
                files.revalidate_in(expected, captured).map_err(|_| SourceClosureError::Custody)?;
                let view = files.view_in(expected, captured).map_err(|_| SourceClosureError::Custody)?;
                for (name, bytes) in [("admission.json", view.admission_bytes()), ("manifest.json", view.manifest_bytes()),
                    ("normalization.json", view.normalization_bytes()), ("bars.ndjson", view.bars_ndjson())] {
                    require(tree.read(&format!("{prefix}/{name}"), MAX_LINE)? == bytes, SourceClosureError::Member)?;
                }
                let actual_admission = parse(view.admission_bytes())?; let actual_manifest = parse(view.manifest_bytes())?;
                if let Some(row) = rows.get(hash.as_str()) {
                    daily_database_binding(row, hash.as_str(), &actual_admission, &actual_manifest)?;
                }
                if let Some(parent) = admission.parent_admission_sha256.as_ref() {
                    let parent_leaf = parent.as_str().strip_prefix("sha256:").ok_or(SourceClosureError::Relationship)?;
                    require(leaves.contains(parent_leaf), SourceClosureError::Relationship)?;
                    let prior_admission = parse(&tree.read(&format!("daily/admitted/{parent_leaf}/admission.json"), 1_048_576)?)?;
                    let prior = parse(&tree.read(&format!("daily/admitted/{parent_leaf}/manifest.json"), 16_777_216)?)?;
                    equal_fields(&prior_admission, &actual_admission, &[("policy_sha256", "policy_sha256")])?;
                    equal_fields(&prior, &actual_manifest, &[("base", "base"), ("instruments", "instruments")])?;
                    require(prior.get("range_end")?.text()? < actual_manifest.get("range_end")?.text()?, SourceClosureError::Relationship)?;
                    for field in ["sessions", "daily_sources"] {
                        let before = prior.get(field)?.array()?; let after = actual_manifest.get(field)?.array()?;
                        require(after.len() == before.len() + 1 && &after[..before.len()] == before, SourceClosureError::Relationship)?;
                    }
                    if let Some(row) = rows.get(hash.as_str()) {
                        let before = rows.get(parent.as_str()).ok_or(SourceClosureError::Relationship)?;
                        equal_fields(row, before, &[("owner_user_id", "owner_user_id"), ("policy_sha256", "policy_sha256")])?;
                    }
                }
                files.revalidate_in(expected, captured).map_err(|_| SourceClosureError::Custody)?;
                tree.revalidate(keep)?;
                Ok(())
            })();
            let after = files.revalidate_in(expected, captured).map_err(|_| SourceClosureError::Custody);
            let closed = files.finish_local().is_clean();
            require(closed, SourceClosureError::Custody)?; after.and(used)?;
        }
        Ok(())
    })();
    let after = root.revalidate_in(expected, captured).map_err(|_| SourceClosureError::Custody);
    let closed = root.finish_local().is_clean();
    require(closed, SourceClosureError::Custody)?; after.and(body)
}

// PostgreSQL jsonb::text preimages used ONLY by the adopted Daily SQL calls.
// These D4 transports contain integer counts, strings, bool/null and arrays;
// fractional numbers are refused, never rounded or financially reformatted.
fn daily_sql_json(value: &Json) -> Result<Vec<u8>> {
    fn emit(value: &Json, out: &mut Vec<u8>) -> Result<()> {
        match value {
            Json::Object(map) => {
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.as_bytes().cmp(b.as_bytes())));
                out.push(b'{');
                for (i, (key, value)) in pairs.into_iter().enumerate() {
                    if i != 0 { out.extend_from_slice(b", "); }
                    write_string(key, out, false)?; out.extend_from_slice(b": "); emit(value, out)?;
                }
                out.push(b'}');
            }
            Json::Array(values) => {
                out.push(b'[');
                for (i, value) in values.iter().enumerate() {
                    if i != 0 { out.extend_from_slice(b", "); } emit(value, out)?;
                }
                out.push(b']');
            }
            Json::Number(_) => { value.integer()?; value.write(out, false)?; }
            _ => value.write(out, false)?,
        }
        require(out.len() <= 16_777_216, SourceClosureError::Limit)
    }
    let mut out = Vec::new(); emit(value, &mut out)?; Ok(out)
}
fn daily_sql_binding(prefix: &[u8], value: &Json) -> Result<String> {
    let mut hash = Sha256::new(); hash.update(prefix); hash.update(daily_sql_json(value)?);
    Ok(format!("sha256:{:x}", hash.finalize()))
}
fn daily_time(value: &Json) -> Result<domain::UtcTimestamp> {
    domain::UtcTimestamp::parse_rfc3339(value.text()?).map_err(|_| SourceClosureError::DatabaseRow)
}
fn json_wire(value: &impl serde::Serialize) -> Result<Json> {
    parse(&serde_json::to_vec(value).map_err(|_| SourceClosureError::Reconstruction)?)
}
fn daily_frontier_original(tree: &Materialized, hash: &str) -> Result<Json> {
    require(sha(hash), SourceClosureError::DailyPollPreimage)?;
    let path = format!("daily/frontiers/{}/frontier.json", &hash[7..]);
    exact_metadata(tree, &path, 10003, 10001, 0o440)?;
    let value = parse(&tree.read(&path, 1_048_576)?)?;
    require(value.get("observation_sha256")?.text()? == hash, SourceClosureError::DailyPollPreimage)?;
    Ok(value)
}
fn daily_transport_original(tree: &Materialized, frontier: &Json) -> Result<Json> {
    let hash = frontier.get("admission_sha256")?.text()?;
    require(sha(hash), SourceClosureError::DailyPollPreimage)?;
    let prefix = format!("daily/admitted/{}", &hash[7..]);
    let admission_bytes = tree.read(&format!("{prefix}/admission.json"), 1_048_576)?;
    let manifest_bytes = tree.read(&format!("{prefix}/manifest.json"), 16_777_216)?;
    require(digest(&admission_bytes) == hash, SourceClosureError::DailyPollPreimage)?;
    let admission = parse(&admission_bytes)?; let manifest = parse(&manifest_bytes)?;
    require(admission.get("manifest_sha256")?.text()? == digest(&manifest_bytes), SourceClosureError::DailyPollPreimage)?;
    Ok(Json::Object(BTreeMap::from([
        ("admission_sha256".into(), Json::String(hash.into())),
        ("source_frontier_sha256".into(), frontier.get("observation_sha256")?.clone()),
        ("admission".into(), admission), ("manifest".into(), manifest),
    ])))
}

fn daily_calendar_projection<'s>(
    raw: &'s super::RetainedRawEvidence<'s>, row: &Json,
) -> Result<crate::daily_price_only::calendar::VerifiedCalendarProjection<'s>> {
    let evidence = row.get("evidence_json")?;
    let claim: crate::daily_price_only::DailyFileEvidenceV1Wire = serde_json::from_value(evidence.serde_value()?)
        .map_err(|_| SourceClosureError::Reconstruction)?;
    let reference = crate::SourceFileReference { provider: claim.provider.clone(), market: claim.market.clone(),
        batch_id: claim.batch_id, file_name: claim.file_name.clone(), content_hash: claim.wire_sha256.clone(),
        retrieved_at: claim.retrieved_at.as_timestamp() };
    let original = crate::daily_price_only::verify::resolve_original(&raw.evidence, &reference)
        .map_err(|_| SourceClosureError::Reconstruction)?;
    require(original.evidence() == &claim, SourceClosureError::Relationship)?;
    let owner = uuid::Uuid::parse_str(row.get("owner_user_id")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
    let policy = ContentHash::parse(row.get("policy_sha256")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
    crate::daily_price_only::calendar::project_calendar(&original, owner, &policy)
        .map_err(|_| SourceClosureError::Reconstruction)
}
fn daily_calendar_rows(
    all: &[&crate::daily_price_only::source::CommittedRawSnapshot], refs: &References, entitlement: &str,
) -> Result<()> {
    let selected: Vec<_> = all.iter().copied().filter(|s| ["kis", "kis-calendar"].contains(&s.scope().provider())).collect();
    let sources = refs.keyed("owner_beta_daily_calendar_sources", "calendar_evidence_sha256")?;
    let mut covered = BTreeSet::new();
    for row in sources.values() {
        let raw = super::retain_raw_evidence(&selected, entitlement, daily_time(row.get("projected_at")?)?)
            .map_err(|_| SourceClosureError::Reconstruction)?;
        let projection = daily_calendar_projection(&raw, row)?;
        require(projection.evidence_sha256().as_str() == row.get("calendar_evidence_sha256")?.text()?
            && row.get("normalizer_version")?.text()? == "daily-raw-reconstruction-v1", SourceClosureError::Relationship)?;
        let file = json_wire(projection.source_file())?;
        equal_fields(row, &file, &[("source_provider", "provider"), ("source_market", "market"),
            ("source_batch_id", "batch_id"), ("source_batch_date", "batch_date"),
            ("source_file_name", "file_name"), ("source_file_sha256", "wire_sha256"),
            ("source_manifest_line_sha256", "manifest_line_sha256"), ("copy_chain", "copied_from")])?;
        require(daily_time(row.get("source_retrieved_at")?)? == projection.source_file().retrieved_at.as_timestamp(), SourceClosureError::Relationship)?;
        let terminal = projection.source_file().copied_from.last();
        let target = terminal.map(|origin| origin.batch_date).unwrap_or(projection.source_file().batch_date);
        require(row.get("requested_target")?.text()? == target.to_string(), SourceClosureError::Relationship)?;
        for day in projection.days() {
            let key = format!("{}:{}:{}", projection.owner(), projection.evidence_sha256(), day.session_date());
            let original = refs.rows("owner_beta_daily_calendar_days")?.get(&key).ok_or(SourceClosureError::Relationship)?;
            let day = json_wire(day)?;
            for field in ["session_date", "session_type", "evidence_kind", "source_row_ordinal", "source_row_sha256"] {
                require(original.get(field)? == day.get(field)?, SourceClosureError::Relationship)?;
            }
            require(covered.insert(key), SourceClosureError::Duplicate)?;
        }
        raw.revalidate().map_err(|_| SourceClosureError::Custody)?;
    }
    require(covered.len() == refs.rows("owner_beta_daily_calendar_days")?.len(), SourceClosureError::Relationship)
}

fn daily_admission_completion(tree: &Materialized, refs: &References, row: &Json, event: Option<&Json>) -> Result<()> {
    let frontier = daily_frontier_original(tree, row.get("source_frontier_sha256")?.text()?)?;
    equal_fields(row, &frontier, &[("owner_user_id", "owner"), ("policy_sha256", "policy_sha256")])?;
    let transport = daily_transport_original(tree, &frontier)?;
    require(row.get("completion_binding_sha256")?.text()? == daily_sql_binding(b"sql-admission-v1", &transport)?, SourceClosureError::DailyPollPreimage)?;
    let admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    let admission = transport.get("admission")?; let manifest = transport.get("manifest")?;
    let key = transport.get("admission_sha256")?.text()?;
    let stop = manifest.get("range_end")?.text()?;
    let cutoff = daily_time(match event { Some(event) => event.get("recorded_at")?, None => row.get("completed_at")? })?;
    require(frontier.get("range_end")?.text()? == stop && admission.get("policy_sha256")? == row.get("policy_sha256")?, SourceClosureError::Relationship)?;
    let parent_key = admission.get("parent_admission_sha256")?;
    let previous = if parent_key == &Json::Null { crate::range_to_canonical_v3_price::HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END }
        else {
            let parent = admissions.get(parent_key.text()?).ok_or(SourceClosureError::Relationship)?;
            equal_fields(row, parent, &[("owner_user_id", "owner_user_id"), ("policy_sha256", "policy_sha256")])?;
            require(daily_time(parent.get("admitted_at")?)? <= cutoff, SourceClosureError::Relationship)?;
            parent.get("range_end")?.text()?
        };
    let sources = refs.keyed("owner_beta_daily_calendar_sources", "calendar_evidence_sha256")?;
    let mut facts = BTreeSet::new();
    for day in refs.rows("owner_beta_daily_calendar_days")?.values() {
        let source = linked(&sources, day, "calendar_evidence_sha256")?;
        if source.get("owner_user_id")? == row.get("owner_user_id")? && source.get("policy_sha256")? == row.get("policy_sha256")?
            && daily_time(source.get("projected_at")?)? <= cutoff {
            require(day.get("owner_user_id")? == source.get("owner_user_id")?, SourceClosureError::Relationship)?;
            facts.insert((day.get("session_date")?.text()?, day.get("session_type")?.text()?));
        }
    }
    let date = |text| chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| SourceClosureError::DatabaseRow);
    let mut cursor = date(previous)?; let end = date(stop)?;
    require((1..=370).contains(&(end - cursor).num_days()), SourceClosureError::Relationship)?;
    while cursor < end {
        cursor = cursor.succ_opt().ok_or(SourceClosureError::Limit)?;
        require(facts.contains(&(cursor.to_string().as_str(), if cursor == end { "TRADING" } else { "CLOSED" })), SourceClosureError::Relationship)?;
    }
    let mut prior = Vec::new();
    for candidate in admissions.values() {
        if candidate.get("owner_user_id")? == row.get("owner_user_id")? && candidate.get("range_end")?.text()? == stop
            && daily_time(candidate.get("admitted_at")?)? <= cutoff { prior.push(*candidate); }
    }
    require(prior.len() <= 1, SourceClosureError::Relationship)?;
    if row.get("result_code")?.text()? == "VERIFIED" {
        require(event.is_none() && row.get("admission_sha256")?.text()? == key
            && prior.first().is_some_and(|old| old.get("evidence_json").is_ok_and(|value| value == &transport)), SourceClosureError::Relationship)?;
    } else {
        let event = event.ok_or(SourceClosureError::Relationship)?;
        require(event.get("source_kind")?.text()? == "BAR" && event.get("range_start")?.text()? == stop
            && event.get("range_end")?.text()? == stop, SourceClosureError::Relationship)?;
        let changed = prior.first().is_some_and(|old| old.get("admission_sha256").is_ok_and(|value| value != transport.get("admission_sha256").unwrap_or(&Json::Null))
            || old.get("evidence_json").is_ok_and(|value| value != &transport));
        if event.get("code")?.text()? == "SEALED_SOURCE_CHANGED" { return require(changed, SourceClosureError::Relationship); }
        require(event.get("code")?.text()? == "SOURCE_CONFLICT" && !changed, SourceClosureError::Relationship)?;
    }
    let excluded = event.map(|event| event.get("event_sha256")).transpose()?;
    let mut blocked = false;
    for cause in refs.rows("owner_beta_daily_source_events")?.values() {
        blocked |= Some(cause.get("event_sha256")?) != excluded && cause.get("owner_user_id")? == row.get("owner_user_id")?
            && cause.get("range_start")?.text()? <= stop
            && cause.get("range_end")?.text()? >= crate::range_to_canonical_v3_price::HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START
            && daily_time(cause.get("recorded_at")?)? <= cutoff;
    }
    let mut ancestors = Vec::new(); if parent_key != &Json::Null { ancestors.push(parent_key.text()?); }
    if let Some(prior) = prior.first() { ancestors.push(prior.get("admission_sha256")?.text()?); }
    for mut ancestor in ancestors {
        let mut seen = BTreeSet::new();
        loop {
            require(seen.insert(ancestor), SourceClosureError::Relationship)?;
            let original = admissions.get(ancestor).ok_or(SourceClosureError::Relationship)?;
            equal_fields(row, original, &[("owner_user_id", "owner_user_id"), ("policy_sha256", "policy_sha256")])?;
            for event in refs.rows("owner_beta_daily_admission_events")?.values() {
                blocked |= event.get("admission_sha256")?.text()? == ancestor && Some(event.get("evidence_sha256")?) != excluded
                    && daily_time(event.get("created_at")?)? <= cutoff;
            }
            if original.get("parent_admission_sha256")? == &Json::Null { break; }
            ancestor = original.get("parent_admission_sha256")?.text()?;
        }
    }
    require(blocked == (row.get("result_code")?.text()? == "INPUT_BLOCKED"), SourceClosureError::Relationship)
}

fn daily_calendar_completion(
    all: &[&crate::daily_price_only::source::CommittedRawSnapshot], refs: &References, entitlement: &str,
    row: &Json, event: &Json,
) -> Result<()> {
    let selected: Vec<_> = all.iter().copied().filter(|s| ["kis", "kis-calendar"].contains(&s.scope().provider())).collect();
    let raw = super::retain_raw_evidence(&selected, entitlement, daily_time(event.get("recorded_at")?)?)
        .map_err(|_| SourceClosureError::Reconstruction)?;
    let owner = uuid::Uuid::parse_str(row.get("owner_user_id")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
    let policy = ContentHash::parse(row.get("policy_sha256")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
    let mut matched = false;
    for scope in &selected {
        for batch in scope.batches() {
            for file in batch.files() {
                if file.kind() != crate::contract::ResponseKind::Calendar { continue; }
                let reference = crate::SourceFileReference { provider: scope.scope().provider().into(), market: "kr".into(),
                    batch_id: batch.batch_id(), file_name: file.file_name().into(), content_hash: file.content_hash().clone(), retrieved_at: batch.retrieved_at() };
                let original = crate::daily_price_only::verify::resolve_original(&raw.evidence, &reference)
                    .map_err(|_| SourceClosureError::Reconstruction)?;
                let observation = crate::daily_price_only::observation::canonical_source_observation(&[original.evidence().clone()])
                    .map_err(|_| SourceClosureError::Reconstruction)?;
                if observation.sha256().as_str() != row.get("source_frontier_sha256")?.text()? { continue; }
                require(!matched, SourceClosureError::Duplicate)?; matched = true;
                let projection = crate::daily_price_only::calendar::project_calendar(&original, owner, &policy)
                    .map_err(|_| SourceClosureError::Reconstruction)?;
                let file = json_wire(projection.source_file())?; let days = json_wire(&projection.days())?;
                let transport = Json::Array(vec![Json::String(projection.evidence_sha256().to_string()), file.clone(), days.clone()]);
                require(row.get("completion_binding_sha256")?.text()? == daily_sql_binding(b"sql-calendar-v1", &transport)?
                    && event.get("source_kind")?.text()? == "CALENDAR"
                    && projection.days().first().is_some_and(|day| day.session_date().to_string() == event.get("range_start").and_then(Json::text).unwrap_or(""))
                    && projection.days().last().is_some_and(|day| day.session_date().to_string() == event.get("range_end").and_then(Json::text).unwrap_or("")), SourceClosureError::DailyPollPreimage)?;
                let mut prior = BTreeMap::new();
                for source in refs.rows("owner_beta_daily_calendar_sources")?.values() {
                    if source.get("owner_user_id")? == row.get("owner_user_id")?
                        && daily_time(source.get("projected_at")?)? <= daily_time(event.get("recorded_at")?)? {
                        prior.insert(source.get("calendar_evidence_sha256")?.text()?, source);
                    }
                }
                require(!prior.contains_key(projection.evidence_sha256().as_str()), SourceClosureError::Relationship)?;
                let mut conflict = false;
                for old in refs.rows("owner_beta_daily_calendar_days")?.values() {
                    if prior.contains_key(old.get("calendar_evidence_sha256")?.text()?) {
                        for day in days.array()? {
                            conflict |= old.get("session_date")? == day.get("session_date")? && old.get("session_type")? != day.get("session_type")?;
                        }
                    }
                }
                if event.get("code")?.text()? == "CALENDAR_CONFLICT" { require(conflict, SourceClosureError::Relationship)?; }
                else {
                    require(event.get("code")?.text()? == "SEALED_SOURCE_CHANGED" && !conflict, SourceClosureError::Relationship)?;
                    let terminal = |file: &Json| -> Result<Json> {
                        Ok(file.get("copied_from")?.array()?.last().cloned().unwrap_or_else(|| {
                            let mut value = file.object().expect("typed file evidence").clone();
                            for field in ["copied_from", "original_retrieved_at", "original_retrieval_precision"] { value.remove(field); }
                            Json::Object(value)
                        }))
                    };
                    let new = terminal(&file)?; let mut changed = false;
                    for source in prior.values() {
                        let old = terminal(source.get("evidence_json")?)?;
                        changed |= source.get("policy_sha256")? == row.get("policy_sha256")?
                            && ["provider", "market", "batch_id", "file_name"].iter().all(|field| old.get(field).ok() == new.get(field).ok())
                            && old != new;
                    }
                    require(changed, SourceClosureError::DailyObservationOriginalMissing)?;
                }
            }
        }
    }
    raw.revalidate().map_err(|_| SourceClosureError::Custody)?;
    require(matched, SourceClosureError::DailyObservationOriginalMissing)
}

fn daily_history_events(refs: &References) -> Result<()> {
    let admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    let events = refs.keyed("owner_beta_daily_source_events", "event_sha256")?;
    for (key, event) in &events {
        require(["ACTION", "CALENDAR", "BAR", "REFERENCE"].contains(&event.get("source_kind")?.text()?)
            && ["SOURCE_CONFLICT", "SEALED_SOURCE_CHANGED", "SOURCE_REVOKED", "SOURCE_INTEGRITY", "CALENDAR_CONFLICT", "NONCASH_ACTION"].contains(&event.get("code")?.text()?)
            && event.get("range_start")?.text()? <= event.get("range_end")?.text()?, SourceClosureError::Relationship)?;
        let mut preimage = String::from("owner-beta-daily-source-event-v1\n");
        for field in ["owner_user_id", "source_kind", "code", "range_start", "range_end", "evidence_sha256"] {
            let value = event.get(field)?.text()?; require(value.is_ascii(), SourceClosureError::Relationship)?;
            preimage.push_str(value); preimage.push('\n');
        }
        require(digest(preimage.as_bytes()) == *key, SourceClosureError::Relationship)?;
        if event.get("admission_sha256")? != &Json::Null {
            let admission = linked(&admissions, event, "admission_sha256")?;
            require(admission.get("owner_user_id")? == event.get("owner_user_id")?
                && admission.get("range_start")?.text()? <= event.get("range_end")?.text()?
                && admission.get("range_end")?.text()? >= event.get("range_start")?.text()?, SourceClosureError::Relationship)?;
        }
    }
    for event in refs.rows("owner_beta_daily_admission_events")?.values() {
        let admission = linked(&admissions, event, "admission_sha256")?;
        require(event.get("owner_user_id")? == admission.get("owner_user_id")?
            && ["CONFLICT", "REVOKED"].contains(&event.get("kind")?.text()?), SourceClosureError::Relationship)?;
        let preimage = format!("owner-beta-daily-admission-event-v1\n{}\n{}\n{}\n", event.get("kind")?.text()?, event.get("admission_sha256")?.text()?, event.get("evidence_sha256")?.text()?);
        require(digest(preimage.as_bytes()) == event.get("event_sha256")?.text()?, SourceClosureError::Relationship)?;
        if event.get("kind")?.text()? == "CONFLICT" {
            let cause = linked(&events, event, "evidence_sha256")?;
            require(cause.get("owner_user_id")? == event.get("owner_user_id")?
                && admission.get("range_start")?.text()? <= cause.get("range_end")?.text()?
                && admission.get("range_end")?.text()? >= cause.get("range_start")?.text()?, SourceClosureError::Relationship)?;
        }
    }
    Ok(())
}

fn daily_poll_history(tree: &Materialized, refs: &References,
    all: &[&crate::daily_price_only::source::CommittedRawSnapshot], entitlement: &str) -> Result<()> {
    daily_history_events(refs)?;
    let admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    let events = refs.keyed("owner_beta_daily_source_events", "event_sha256")?;
    let receipts = refs.keyed("owner_beta_daily_poll_receipts", "receipt_id")?;
    for row in receipts.values() {
        let fields = ["source_frontier_sha256", "admission_sha256", "event_sha256", "completion_binding_sha256"];
        let stage = row.get("stage")?.text()?;
        require(["ADMISSION", "SCHEDULE"].contains(&stage)
            && (row.get("completed_at")? == &Json::Null) == (row.get("result_code")? == &Json::Null), SourceClosureError::Relationship)?;
        for field in fields { if row.get(field)? != &Json::Null { require(sha(row.get(field)?.text()?), SourceClosureError::Relationship)?; } }
        if row.get("completed_at")? == &Json::Null {
            for field in fields { require(row.get(field)? == &Json::Null, SourceClosureError::Relationship)?; } continue;
        }
        require(daily_time(row.get("completed_at")?)? >= daily_time(row.get("started_at")?)?, SourceClosureError::Relationship)?;
        let result = row.get("result_code")?.text()?;
        if stage == "SCHEDULE" {
            require(["CHECKED", "DEFERRED", "BLOCKED"].contains(&result), SourceClosureError::Relationship)?;
            for field in fields { require(row.get(field)? == &Json::Null, SourceClosureError::Relationship)?; } continue;
        }
        require(["VERIFIED", "INPUT_PENDING", "INPUT_BLOCKED", "IO_FAILED"].contains(&result), SourceClosureError::Relationship)?;
        if result == "VERIFIED" {
            linked(&admissions, row, "admission_sha256")?;
            require(row.get("event_sha256")? == &Json::Null && row.get("completion_binding_sha256")? != &Json::Null, SourceClosureError::Relationship)?;
            daily_admission_completion(tree, refs, row, None)?; continue;
        }
        require(row.get("admission_sha256")? == &Json::Null, SourceClosureError::Relationship)?;
        if row.get("completion_binding_sha256")? == &Json::Null {
            require(row.get("event_sha256")? == &Json::Null, SourceClosureError::Relationship)?; continue;
        }
        require(result == "INPUT_BLOCKED", SourceClosureError::Relationship)?;
        let event = linked(&events, row, "event_sha256")?;
        require(event.get("owner_user_id")? == row.get("owner_user_id")? && event.get("evidence_sha256")? == row.get("source_frontier_sha256")?
            && daily_time(event.get("recorded_at")?)? <= daily_time(row.get("completed_at")?)?, SourceClosureError::Relationship)?;
        let payload = Json::Object(["source_kind", "range_start", "range_end", "code", "evidence_sha256"].iter()
            .map(|field| Ok(((*field).to_owned(), event.get(field)?.clone()))).collect::<Result<_>>()?);
        if event.get("code")?.text()? != "SOURCE_REVOKED" && row.get("completion_binding_sha256")?.text()? == daily_sql_binding(b"sql-block-v1", &payload)? { continue; }
        match event.get("source_kind")?.text()? {
            "CALENDAR" => daily_calendar_completion(all, refs, entitlement, row, event)?,
            "BAR" => daily_admission_completion(tree, refs, row, Some(event))?,
            _ => return Err(SourceClosureError::DailyPollPreimage),
        }
    }
    daily_poll_state(refs)
}

fn daily_poll_state(refs: &References) -> Result<()> {
    let receipts = refs.keyed("owner_beta_daily_poll_receipts", "receipt_id")?;
    let states = refs.keyed("owner_beta_daily_poll_state", "stage")?;
    for row in receipts.values() { require(states.contains_key(row.get("stage")?.text()?), SourceClosureError::Relationship)?; }
    for state in states.values() {
        require(["ADMISSION", "SCHEDULE"].contains(&state.get("stage")?.text()?), SourceClosureError::Relationship)?;
        let projection = ["last_completed_at", "last_result_code", "verified_admission_sha256", "source_frontier_sha256"];
        if state.get("latest_receipt_id")? == &Json::Null {
            for field in projection { require(state.get(field)? == &Json::Null, SourceClosureError::Relationship)?; }
            require(!receipts.values().any(|row| row.get("stage").ok() == state.get("stage").ok()), SourceClosureError::Relationship)?; continue;
        }
        let receipt = linked(&receipts, state, "latest_receipt_id")?;
        require(receipt.get("stage")? == state.get("stage")?, SourceClosureError::Relationship)?;
        for old in receipts.values() {
            if old.get("stage")? == state.get("stage")? { require(daily_time(old.get("started_at")?)? <= daily_time(receipt.get("started_at")?)?, SourceClosureError::Relationship)?; }
        }
        let start = chrono::DateTime::parse_from_rfc3339(receipt.get("started_at")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
        let next = chrono::DateTime::parse_from_rfc3339(state.get("next_poll_at")?.text()?).map_err(|_| SourceClosureError::DatabaseRow)?;
        require(start.checked_add_signed(chrono::Duration::seconds(60)) == Some(next), SourceClosureError::Relationship)?;
        if receipt.get("completed_at")? == &Json::Null {
            for field in projection { require(state.get(field)? == &Json::Null, SourceClosureError::Relationship)?; }
        } else {
            equal_fields(state, receipt, &[("last_completed_at", "completed_at"), ("last_result_code", "result_code"),
                ("verified_admission_sha256", "admission_sha256"), ("source_frontier_sha256", "source_frontier_sha256")])?;
        }
    }
    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "Original source holders and caller native binding remain explicit.")]
fn daily_native_history(
    tree: &Materialized, refs: &References, root: &Path,
    invocation: &RawInvocation<'_>, expected: &Attempt, captured: &Attempt,
    sources: &[crate::daily_price_only::source::CheckedRawSnapshot<'_, '_>],
    limits: SourceClosureLimits, keep: &dyn Fn() -> bool,
) -> Result<()> {
    let has_files = tree.manifest.entries.iter().any(|entry| entry.path.starts_with("daily/admitted/")
        || entry.path.starts_with("daily/frontiers/"));
    let has_rows = ["owner_beta_daily_admissions", "owner_beta_daily_calendar_sources", "owner_beta_daily_calendar_days",
        "owner_beta_daily_poll_receipts", "owner_beta_daily_control"].iter()
        .try_fold(false, |found, table| -> Result<bool> { Ok(found || !refs.rows(table)?.is_empty()) })?;
    if !has_files && !has_rows { return Ok(()); }
    require(tree.manifest.by_path.contains_key("config/config.json"), SourceClosureError::DailyConfigOriginalMissing)?;
    require(tree.manifest.by_path.contains_key("config/policy.json"), SourceClosureError::DailyPolicyOriginalMissing)?;
    exact_metadata(tree, "config/config.json", 0, 10001, 0o440)?;
    exact_metadata(tree, "config/policy.json", 0, 10001, 0o440)?;
    let config = parse(&tree.read("config/config.json", 262_144)?)?;
    let policy_bytes = tree.read("config/policy.json", 262_144)?;
    let hash = ContentHash::from_bytes(&policy_bytes);
    let policy = crate::daily_price_only::admission::PinnedDailyPolicy::from_pinned_bytes(&policy_bytes, &hash)
        .map_err(|_| SourceClosureError::Reconstruction)?;
    require(config.get("policy_sha256")?.text()? == hash.as_str(), SourceClosureError::DailyConfigOriginalMissing)?;
    let owner = config.get("owner")?.text()?; let entitlement = config.get("entitlement_reference")?.text()?;
    require(!entitlement.is_empty(), SourceClosureError::DailyConfigOriginalMissing)?;
    for table in ["owner_beta_daily_admissions", "owner_beta_daily_calendar_sources", "owner_beta_daily_poll_receipts", "owner_beta_daily_control"] {
        for row in refs.rows(table)?.values() {
            // Only the actual installed original is present in this adopted
            // namespace. An older tuple needs its real retained original; no
            // current-policy substitution, today-status test or EMPTY fallback.
            require(row.get("policy_sha256")?.text()? == hash.as_str(), SourceClosureError::DailyPolicyOriginalMissing)?;
            require(row.get("owner_user_id")?.text()? == owner, SourceClosureError::DailyConfigOriginalMissing)?;
        }
    }
    for row in refs.rows("owner_beta_daily_control")?.values() {
        require(row.get("policy_json")? == &parse(&policy_bytes)?, SourceClosureError::Relationship)?;
        equal_fields(row, &config, &[("entitlement_reference", "entitlement_reference"),
            ("entitlement_sha256", "entitlement_sha256"), ("first_auto_session", "first_auto_session")])?;
    }
    // Revoked/expired entries are retained history; this is not an ACTIVE gate.
    require(refs.rows("data_entitlements")?.values().any(|row|
        row.get("contract_reference").is_ok_and(|v| v == config.get("entitlement_reference").unwrap_or(&Json::Null))
        && row.get("contract_document_sha256").is_ok_and(|v| v == config.get("entitlement_sha256").unwrap_or(&Json::Null))),
        SourceClosureError::DailyConfigOriginalMissing)?;
    let admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")?;
    if has_files {
        require(tree.manifest.by_path.contains_key(&format!("simulation_approval/{V3_REGISTRY}")), SourceClosureError::HistoricalV3RegistryMissing)?;
        let budget = u64::try_from(limits.max_reference_bytes).map_err(|_| SourceClosureError::Limit)?;
        let work = budget.checked_mul(limits.max_reference_rows as u64).and_then(|v| v.checked_mul(4)).ok_or(SourceClosureError::Limit)?;
        let history_limits = crate::daily_price_only::replay::DailyHistoryLimits::new(
            limits.max_reference_rows.min(crate::daily_price_only::contract::fixed_limits().max_sessions as usize) as u64,
            budget, work.min(crate::daily_price_only::contract::fixed_limits().max_bars_bytes
                .checked_mul(crate::daily_price_only::contract::fixed_limits().max_sessions).ok_or(SourceClosureError::Limit)?))
            .map_err(|_| SourceClosureError::Limit)?;
        let mut seen = BTreeSet::new(); let failure = std::cell::Cell::new(None);
        let used = super::consume_checked_daily_history_originals(invocation, expected, captured, sources,
            &root.join("base"), &root.join("simulation_approval"), &root.join("daily/admitted"), &root.join("daily/frontiers"),
            &policy, entitlement, |hash| {
                match admissions.get(hash.as_str()).and_then(|row| row.get("admitted_at").ok()).map(daily_time) {
                    Some(Ok(time)) => Ok(time),
                    _ => { failure.set(Some(SourceClosureError::DailyObservationOriginalMissing)); Err(ArchiveSourceError::Context) }
                }
            }, history_limits, keep, |parent, frontier| {
                let result = (|| {
                    tree.revalidate(&mut || keep())?;
                    let hash = parent.pins().admission_sha256.as_str();
                    require(seen.insert(hash.to_owned()), SourceClosureError::Duplicate)?;
                    if let Some(row) = admissions.get(hash) {
                        require(row.get("owner_user_id")?.text()? == frontier.owner().to_string()
                            && row.get("evidence_json")?.get("source_frontier_sha256")?.text()? == frontier.sha256().as_str(),
                            SourceClosureError::Relationship)?;
                        let original = daily_transport_original(tree, &daily_frontier_original(tree, frontier.sha256().as_str())?)?;
                        require(row.get("evidence_json")? == &original, SourceClosureError::Relationship)?;
                        daily_database_binding(row, hash, original.get("admission")?, original.get("manifest")?)?;
                    }
                    tree.revalidate(&mut || keep())
                })();
                if let Err(error) = result { failure.set(Some(error)); return Err(ArchiveSourceError::Pin); } Ok(())
            });
        if let Some(error) = failure.get() { return Err(error); }
        used.map_err(|error| match error {
            ArchiveSourceError::ExclusionUnproven => SourceClosureError::DailyExclusionUnproven,
            ArchiveSourceError::Interrupted => SourceClosureError::Interrupted,
            _ => SourceClosureError::Reconstruction,
        })?;
        require(admissions.keys().all(|key| seen.contains(key)), SourceClosureError::Relationship)?;
    } else { require(admissions.is_empty(), SourceClosureError::Member)?; }
    let checked = crate::daily_price_only::source::with_checked_raw_originals(sources, expected, captured, |all| {
        daily_calendar_rows(all, refs, entitlement)?;
        daily_poll_history(tree, refs, all, entitlement)
    });
    match checked { Ok(()) => Ok(()), Err(error) => Err(error.callback.and_then(std::result::Result::err).unwrap_or(SourceClosureError::Custody)) }
}

fn curated_source_dependencies(
    tree: &Materialized, refs: &References,
    sources: &[crate::daily_price_only::source::CheckedRawSnapshot<'_, '_>],
    expected: &Attempt, captured: &Attempt, limits: SourceClosureLimits, keep: &dyn Fn() -> bool,
) -> Result<()> {
    let paths: Vec<_> = tree.manifest.by_path.keys().filter(|path|
        path.starts_with("curated/datasets/") && path.ends_with("/manifest.json")).collect();
    let references: Vec<_> = ["pending_targets", "paper_rebalance_previews"].into_iter()
        .map(|table| refs.rows(table)).collect::<Result<Vec<_>>>()?.into_iter()
        .flat_map(|rows| rows.values()).filter(|row| row.get("dataset_manifest_sha256").is_ok_and(|v| v != &Json::Null)).collect();
    if paths.is_empty() {
        require(references.is_empty() && !tree.manifest.entries.iter().any(|entry|
            entry.path.starts_with("curated/") && !entry.directory), SourceClosureError::CuratedDependency)?;
        return Ok(());
    }
    let mut covered = BTreeSet::new(); let mut generations = BTreeMap::new();
    let checked = crate::daily_price_only::source::with_checked_raw_originals(sources, expected, captured, |all| {
        let reader = super::CapturedPublicationReader { originals: all, keep_reading: keep };
        let mut batches = BTreeMap::new();
        let mut remaining_rows = limits.max_reference_rows as u64;
        for original in all {
            if !["krx", "kis-normalized"].contains(&original.scope().provider()) { continue; }
            for batch in original.batches() {
                require(batches.insert(batch.batch_id(), batch.entry()).is_none(), SourceClosureError::Duplicate)?;
            }
        }
        for path in paths {
            require(keep(), SourceClosureError::Interrupted)?; tree.revalidate(&mut || keep())?;
            let bytes = tree.read(path, 16_777_216)?;
            let value = parse(&bytes)?;
            // Existing legacy manifests are parseable but do not retain an
            // attested artifact list. Never infer that inventory is EMPTY.
            require(value.object()?.contains_key("artifacts")
                && !value.get("artifacts")?.array()?.is_empty(), SourceClosureError::CuratedLegacyArtifactsUnretained)?;
            value.keys(&["dataset_id", "version", "capability", "created_at", "source_batches", "artifacts", "bar_count", "action_count", "content_hash"])?;
            for source in value.get("source_batches")?.array()? {
                source.keys(&["batch_id", "bars_file", "bars_hash", "actions_file", "actions_hash"])?;
            }
            for artifact in value.get("artifacts")?.array()? { artifact.keys(&["path", "sha256", "size_bytes", "schema"])?; }
            let manifest: crate::curate::DatasetManifest = serde_json::from_value(value.serde_value()?)
                .map_err(|_| SourceClosureError::CuratedDependency)?;
            require(*path == format!("curated/datasets/{}/version={}/manifest.json", manifest.dataset_id, manifest.version), SourceClosureError::CuratedDependency)?;
            let entries = manifest.source_batches.iter().map(|source| batches.get(&source.batch_id)
                .map(|entry| (**entry).clone()).ok_or(SourceClosureError::CuratedDependency)).collect::<Result<Vec<_>>>()?;
            // Reuse the caller's adopted aggregate row-work bound across all
            // generations; never reset a budget for each historical manifest.
            let row_work = manifest.bar_count.checked_mul(3).and_then(|bars| bars.checked_add(manifest.action_count))
                .ok_or(SourceClosureError::Limit)?;
            remaining_rows = remaining_rows.checked_sub(row_work).ok_or(SourceClosureError::Limit)?;
            let values = crate::curate::archived_generation_values(&reader, &entries, &manifest, limits.max_reference_rows)
                .map_err(|_| SourceClosureError::CuratedDependency)?;
            require(generations.insert(values.evidence.manifest_sha256.clone(), (manifest.dataset_id.to_string(), manifest.version)).is_none(), SourceClosureError::Duplicate)?;
            covered.insert((*path).clone());
            for artifact in &manifest.artifacts {
                let member = format!("curated/{}", artifact.path);
                let index = *tree.manifest.by_path.get(&member).ok_or(SourceClosureError::CuratedDependency)?;
                let entry = &tree.manifest.entries[index];
                require(!entry.directory && entry.size == artifact.size_bytes && entry.sha256.as_deref() == Some(artifact.sha256.as_str())
                    && artifact.path.split('/').any(|part| part == format!("version={}", manifest.version)), SourceClosureError::CuratedDependency)?;
                let (_, expected_values) = values.artifacts.get(&artifact.path).ok_or(SourceClosureError::CuratedDependency)?;
                tree.verify_curated_values(index, &artifact.schema, expected_values, limits, keep)?;
                require(covered.insert(member), SourceClosureError::Duplicate)?;
            }
            tree.revalidate(&mut || keep())?;
        }
        for entry in &tree.manifest.entries {
            if entry.path.starts_with("curated/") && !entry.directory { require(covered.contains(&entry.path), SourceClosureError::CuratedDependency)?; }
        }
        for row in &references {
            // Existing generic publication uses canonical plain SHA-256.
            // Beta NULL tuples never create a generic dataset dependency.
            let pin = row.get("dataset_manifest_sha256")?.text()?;
            require(pin.len() == 64 && lower_hex(pin) && generations.contains_key(pin), SourceClosureError::CuratedDependency)?;
        }
        reader.revalidate().map_err(|_| SourceClosureError::Custody)
    });
    if let Err(error) = checked { return Err(error.callback.and_then(std::result::Result::err).unwrap_or(SourceClosureError::Custody)); }
    // The frozen reference projection does not include dataset_versions, so
    // the UUID -> (dataset id, generation) join cannot be replaced by a matching
    // manifest digest. This is a missing read-side source seam, not authority.
    if !references.is_empty() { return Err(SourceClosureError::CuratedDatasetBindingUnavailable); }
    Ok(())
}

fn class_rows(tree: &Materialized) -> Result<Vec<Json>> {
    let mut result = Vec::new();
    for class in CLASSES {
        let mut hash = Sha256::new(); let mut entries = 0u64; let mut bytes = 0u64;
        for entry in &tree.manifest.entries {
            if entry.path == class || !entry.path.starts_with(&format!("{class}/")) { continue; }
            let mut record = Vec::new(); entry.row.write(&mut record, true)?;
            hash.update(format!("{}:", record.len()).as_bytes()); hash.update(record);
            entries = entries.checked_add(1).ok_or(SourceClosureError::Limit)?;
            if !entry.directory { bytes = bytes.checked_add(entry.size).ok_or(SourceClosureError::Limit)?; }
        }
        result.push(Json::Object(BTreeMap::from([
            ("class".into(), Json::String(class.into())), ("entries".into(), Json::Number(entries.to_string())),
            ("bytes".into(), Json::Number(bytes.to_string())),
            ("sha256".into(), Json::String(format!("sha256:{:x}", hash.finalize()))),
        ])));
    }
    Ok(result)
}
fn hex_path(path: &str) -> String { path.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect() }

/// Private until all five domain graphs are implemented. The returned digest
/// can never be promoted by a callback, a caller boolean or a serialized report.
fn provisional_root(tree: &Materialized, refs: &References) -> Result<RootValue> {
    let mut root = FramedRoot::default();
    for row in class_rows(tree)? { root.add("source_class", row.get("class")?.text()?, &row)?; }
    for (path, index) in &tree.manifest.by_path {
        root.add("source_member", &hex_path(path), &tree.manifest.entries[*index].row)?;
    }
    for (table, rows) in &refs.tables {
        for (key, row) in rows { root.add("source_reference", &format!("{table}:{key}"), row)?; }
    }
    Ok(root.value())
}

/// Fixed descriptive callback variants. Their return values are discarded.
/// No success from this traversal completes a source_files audit group.
pub enum ArchiveOriginalMember<'a> {
    /// Provisional row from the full nine-group projection, before reference
    /// duplication/EOF has settled. Never a checked row or SQL authority.
    Database { group: &'a str, table: &'a str, key: &'a str, canonical_row: &'a [u8] },
    Class { name: &'a str, canonical_record: &'a [u8] },
    Member { path: &'a str, canonical_record: &'a [u8], file: Option<&'a File> },
    Reference { table: &'a str, key: &'a str, canonical_row: &'a [u8] },
    Job { table: &'a str, key: &'a str, canonical_row: &'a [u8] },
}

/// Actual existing codec/SQL originals and an independently materialized tree.
/// Paths select physical custody only, never database-declared paths or data
/// authority. The caller owns restoration, native binding and exported snapshot.
pub struct ArchiveOriginalPaths<'a> {
    pub files_bundle: &'a Path,
    pub database_stream: &'a Path,
    pub materialized: &'a Path,
    pub raw_coordination: &'a Path,
}

/// Non-serving archive admission. The fixed source-selected originals and
/// actual executing ELF are reopened here, independently of the native caller's
/// parsed R/A values. A hash/boolean supplied by that caller cannot construct it.
/// Root is the trusted installer/controller boundary; this does not authenticate
/// arbitrary root JSON, prove a stopped cluster, or complete the archive graph.
pub struct ArchiveAdmissionContext<'a> {
    attempt: &'a Attempt,
    originals: Option<Vec<(InputOriginal, String)>>,
    executable: Option<ArchiveFile>,
    executable_identity: FileIdentity,
    deadline: std::time::Instant,
}
/// Deliberately no Deref, Clone, deserializer or RawInvocation accessor.
/// Only the archive owner adapter in this module can borrow its RawInvocation.
pub struct ArchiveInvocation<'a> { raw: RawInvocation<'a> }

impl<'a> ArchiveAdmissionContext<'a> {
    pub fn open(attempt: &'a Attempt, deadline: std::time::Instant) -> Result<Self> {
        let mut failure = HistoricalUseGuard { attempt, armed: true };
        require(deadline > std::time::Instant::now()
            && deadline.saturating_duration_since(std::time::Instant::now()) <= std::time::Duration::from_secs(15), SourceClosureError::Interrupted)?;
        require(unsafe { libc::getuid() == 0 && libc::geteuid() == 0 }, SourceClosureError::NativeBinding)?;
        let mut originals = Vec::new();
        let mut values = Vec::new();
        for name in ["request.json", "action-binding.json", "capability.json", "database.json",
            "operation/intent.json", "operation/actions/00000000/binding.json"] {
            let path = Path::new("/run/owner-beta-paper-ops").join(name);
            let original = InputOriginal::open_mode(&path, attempt, 65_536, 0o400)?;
            let mut bytes = vec![0; original.size as usize];
            let mut at = 0;
            while at < bytes.len() {
                require(std::time::Instant::now() < deadline, SourceClosureError::Interrupted)?;
                let count = original.read_at(&mut bytes[at..], at as u64)?;
                require(count > 0, SourceClosureError::Custody)?;
                at += count;
            }
            let value = parse(&bytes)?;
            require(value.canonical()? == bytes, SourceClosureError::NativeBinding)?;
            originals.push((original, digest(&bytes)));
            values.push(value);
        }
        require(originals[0].1 == originals[4].1 && originals[1].1 == originals[5].1,
            SourceClosureError::NativeBinding)?;
        let (root, action, capability, database) = (&values[0], &values[1], &values[2], &values[3]);
        require(root.get("schema")?.text()? == "owner-beta-paper-operation-intent-v1"
            && root.get("purpose")?.text()? == "RESTORE"
            && root.get("scope")?.text()? == "COMBINED_BACKUP"
            && root.get("artifact_sha256")? != &Json::Null
            && root.get("predecessor_generation")? != &Json::Null
            && action.get("schema")?.text()? == "owner-beta-paper-action-binding-v1"
            && action.get("verb")?.text()? == "begin"
            && action.get("sequence")?.integer()? == 0
            && action.get("previous_binding_sha256")? == &Json::Null
            && action.get("artifact_sha256")? == &Json::Null
            && action.get("root_intent_sha256")?.text()? == originals[0].1,
            SourceClosureError::NativeBinding)?;
        for field in ["installation_id", "generation", "request_key", "expected_epoch"] {
            require(root.get(field)? == action.get(field)?, SourceClosureError::NativeBinding)?;
        }
        database.keys(&["schema", "host", "port", "database"])?;
        require(database.get("schema")?.text()? == "owner-beta-paper-ops-database-v1"
            && database.get("host")?.text()? == "/run/postgresql"
            && database.get("port")?.integer()? == 5432, SourceClosureError::NativeBinding)?;
        capability.keys(&["schema", "minimum", "code_commit", "release_manifest_sha256",
            "operations_schema_sha256", "archive_schema_sha256", "migration_0068_sha256",
            "audit_sql_sha256", "api_binary_sha256", "worker_binary_sha256", "native_binary_sha256"])?;
        require(capability.get("schema")?.text()? == "owner-beta-paper-reader-capability-v1"
            && capability.get("minimum")?.integer()? == 1
            && Some(capability.get("code_commit")?.text()?) == option_env!("LAGRANGE_CODE_COMMIT")
            && root.get("release_manifest_sha256")? == capability.get("release_manifest_sha256")?,
            SourceClosureError::NativeBinding)?;
        // Reopen actual source/release and selected api/worker ELF originals.
        // The caller's parsed capability cannot substitute for these owners.
        for (name, pin, maximum) in [
            ("operations.schema.json", "operations_schema_sha256", 1_048_576),
            ("archive.schema.json", "archive_schema_sha256", 1_048_576),
            ("migration-0068.up.sql", "migration_0068_sha256", 1_048_576),
            ("audit.sql", "audit_sql_sha256", 1_048_576),
            ("api-image.elf", "api_binary_sha256", 536_870_912),
            ("worker-image.elf", "worker_binary_sha256", 536_870_912),
        ] {
            let original = InputOriginal::open_mode(&Path::new("/run/owner-beta-paper-ops").join(name),
                attempt, maximum, 0o400)?;
            if name.ends_with(".elf") {
                let mut magic = [0; 4];
                original.file.read_exact_at(&mut magic, 0).map_err(|_| SourceClosureError::Custody)?;
                require(magic == *b"\x7fELF", SourceClosureError::NativeBinding)?;
            }
            let hash = original.hash(&mut || std::time::Instant::now() < deadline)?;
            require(hash == capability.get(pin)?.text()?, SourceClosureError::NativeBinding)?;
            originals.push((original, hash));
        }
        let stage = InputOriginal::open_mode(Path::new("/run/owner-beta-paper-ops/operation/00000000.json"),
            attempt, 65_536, 0o400)?;
        let stage_hash = stage.hash(&mut || std::time::Instant::now() < deadline)?;
        require(stage_hash == action.get("stage_sha256")?.text()?, SourceClosureError::NativeBinding)?;
        originals.push((stage, stage_hash));
        let file = ArchiveFile::adopt(File::open("/proc/self/exe").map_err(|_| SourceClosureError::Custody)?, Some(attempt));
        let info = rustix::fs::fstat(&file).map_err(|_| SourceClosureError::Custody)?;
        require(rustix::fs::FileType::from_raw_mode(info.st_mode) == rustix::fs::FileType::RegularFile
            && info.st_uid == 0 && info.st_gid == 0 && info.st_nlink == 1
            && info.st_mode & 0o022 == 0 && info.st_size > 4 && info.st_size <= 536_870_912,
            SourceClosureError::NativeBinding)?;
        super::archive_no_acl(&file).map_err(|_| SourceClosureError::Metadata)?;
        let mut magic = [0; 4]; file.read_exact_at(&mut magic, 0).map_err(|_| SourceClosureError::Custody)?;
        require(magic == *b"\x7fELF", SourceClosureError::NativeBinding)?;
        let mut hasher = Sha256::new(); let mut offset = 0u64; let mut buffer = [0; 65_536];
        loop {
            require(std::time::Instant::now() < deadline, SourceClosureError::Interrupted)?;
            let count = file.read_at(&mut buffer, offset).map_err(|_| SourceClosureError::Custody)?;
            if count == 0 { break; }
            offset = offset.checked_add(count as u64).filter(|n| *n <= 536_870_912).ok_or(SourceClosureError::Limit)?;
            hasher.update(&buffer[..count]);
        }
        require(offset == info.st_size as u64
            && capability.get("native_binary_sha256")?.text()? == format!("sha256:{:x}", hasher.finalize()),
            SourceClosureError::NativeBinding)?;
        let owner = Self { attempt, originals: Some(originals), executable: Some(file),
            executable_identity: FileIdentity::from_stat(&info, true), deadline };
        owner.revalidate()?;
        failure.armed = false;
        Ok(owner)
    }
    pub fn invocation(&self) -> ArchiveInvocation<'_> {
        ArchiveInvocation { raw: RawInvocation::with_native_context(self.attempt, self) }
    }
    pub fn revalidate(&self) -> Result<()> {
        require(self.attempt.check() && std::time::Instant::now() < self.deadline, SourceClosureError::Custody)?;
        let mut clean = true;
        // Initial complete hashes bind immutable root0400 originals. Every use
        // compares the retained and named inode/size/mtime/ctime/mode/owner/ACL.
        // Rehashing both entire ELFs per SQL/Raw row would consume the fixed
        // operation budget without strengthening this trusted-root boundary.
        for (original, _) in self.originals.as_ref().ok_or(SourceClosureError::Custody)? {
            clean &= original.revalidate().is_ok();
        }
        let file = self.executable.as_ref().ok_or(SourceClosureError::Custody)?;
        let original = rustix::fs::fstat(file).map_err(|_| SourceClosureError::Custody)?;
        // The new descriptor is also adopted and closed on the SAME Attempt.
        let named = ArchiveFile::adopt(File::open("/proc/self/exe").map_err(|_| SourceClosureError::Custody)?, Some(self.attempt));
        let current = rustix::fs::fstat(&named).map_err(|_| SourceClosureError::Custody)?;
        clean &= FileIdentity::from_stat(&original, true) == self.executable_identity
            && FileIdentity::from_stat(&current, true) == self.executable_identity;
        drop(named);
        require(clean && self.attempt.check(), SourceClosureError::Custody)
    }
    pub fn finish_local(mut self) -> Result<()> {
        let clean = self.revalidate().is_ok();
        drop(self.originals.take()); drop(self.executable.take());
        require(clean && self.attempt.check(), SourceClosureError::Custody)
    }
}
impl crate::raw_custody::NativeAdmissionContext for ArchiveAdmissionContext<'_> {
    fn compare_scopes(&self, invocation: &RawInvocation<'_>, expected: &Attempt)
        -> std::result::Result<(), crate::paper_source_evidence::ExecutionGateFailure> {
        if self.attempt.same_scope(invocation.attempt()) && self.attempt.same_scope(expected)
            && self.attempt.check() && expected.check() { return Ok(()); }
        Err(crate::paper_source_evidence::archive_scope_refusal(invocation, self.attempt, expected))
    }
    fn admit(&self, invocation: &RawInvocation<'_>, expected: &Attempt)
        -> std::result::Result<(), crate::paper_source_evidence::ExecutionGateFailure> {
        self.compare_scopes(invocation, expected)?;
        self.revalidate().map_err(|_| crate::paper_source_evidence::archive_scope_refusal(invocation, self.attempt, expected))
    }
}
impl Drop for ArchiveAdmissionContext<'_> {
    fn drop(&mut self) {
        if self.originals.is_some() || self.executable.is_some() {
            self.attempt.gap(Gap::Abandoned);
            drop(self.originals.take()); drop(self.executable.take());
        }
    }
}

/// Holds the actual opened codec, SQL stream, materialization and Raw owners.
/// This is provisional custody, not a completed source group or permission.
/// In particular no constructor accepts row maps, a verified bit or a digest
/// in place of the genuine native invocation and opened originals. Keeping
/// this value alive permits a protected caller to await its own SQL reads
/// before using the synchronous source readers; it exposes no RawInvocation.
pub struct ArchiveOriginalOwner<'scope, 'attempt> {
    invocation: &'scope RawInvocation<'attempt>,
    expected: &'scope Attempt,
    captured: &'scope Attempt,
    materialized: std::path::PathBuf,
    raw_coordination: std::path::PathBuf,
    limits: SourceClosureLimits,
    raw_limits: crate::daily_price_only::source::ScanLimitValues,
    codec: Option<InputOriginal>,
    database: Option<InputOriginal>,
    tree: Option<Materialized>,
    references: Option<References>,
    sources: Vec<crate::daily_price_only::source::CheckedRawSnapshot<'scope, 'attempt>>,
    codec_hash: Option<String>,
    database_hash: Option<String>,
    retired: bool,
    source_pass_started: bool,
}

// A panic in one cleanup step cannot prevent visiting the remaining owners.
// The caller's original panic/error stays primary; the real ledger records
// cleanup uncertainty. This helper never manufactures a successful close.
fn archive_cleanup_step(attempt: &Attempt, step: impl FnOnce() -> bool) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)) {
        Ok(clean) => clean,
        Err(_) => { attempt.gap(Gap::Abandoned); false }
    }
}

impl<'scope, 'attempt> ArchiveOriginalOwner<'scope, 'attempt> {
    /// Distinct non-serving entrypoint. The private RawInvocation never crosses
    /// the market-data boundary, even while this owner is borrowed across await.
    #[expect(clippy::too_many_arguments, reason = "Keep actual archive scope and bounds explicit.")]
    pub fn open_archive_in(
        invocation: &'scope ArchiveInvocation<'attempt>, expected: &'scope Attempt, captured: &'scope Attempt,
        paths: ArchiveOriginalPaths<'_>, limits: SourceClosureLimits,
        raw_limits: crate::daily_price_only::source::ScanLimitValues,
        keep: impl FnMut() -> bool, consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<Self> {
        require(paths.files_bundle == Path::new("/run/owner-beta-paper-ops/archive/files.bundle")
            && paths.database_stream == Path::new("/run/owner-beta-paper-ops/archive/audit.stream")
            && paths.materialized == Path::new("/run/owner-beta-paper-ops/archive/files")
            && paths.raw_coordination == Path::new("/run/owner-beta-paper-ops/archive/coordination"),
            SourceClosureError::NativeBinding)?;
        Self::open_in(&invocation.raw, expected, captured, paths, limits, raw_limits, keep, consume)
    }

    #[expect(clippy::too_many_arguments, reason = "Keep actual native scope, originals and independent bounds together.")]
    pub fn open_in(
        invocation: &'scope RawInvocation<'attempt>, expected: &'scope Attempt, captured: &'scope Attempt,
        paths: ArchiveOriginalPaths<'_>, limits: SourceClosureLimits,
        raw_limits: crate::daily_price_only::source::ScanLimitValues,
        keep_reading: impl FnMut() -> bool,
        consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<Self> {
        let mut owner = Self {
            invocation, expected, captured,
            materialized: paths.materialized.to_owned(), raw_coordination: paths.raw_coordination.to_owned(),
            limits, raw_limits, codec: None, database: None, tree: None, references: None,
            sources: Vec::new(), codec_hash: None, database_hash: None, retired: false,
            source_pass_started: false,
        };
        // Partial-open failure and unwind drop this SAME owner, including every
        // descriptor already adopted by the caller's original Attempt.
        owner.open_originals(paths, keep_reading, consume)?;
        Ok(owner)
    }

    fn open_originals(
        &mut self, paths: ArchiveOriginalPaths<'_>, keep_reading: impl FnMut() -> bool,
        mut consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<()> {
        let invocation = self.invocation; let expected = self.expected; let captured = self.captured;
        let limits = self.limits; let raw_limits = self.raw_limits;
        let poll = std::cell::RefCell::new(keep_reading);
        let continuing = || (*poll.borrow_mut())();
        native_scope(invocation, expected, captured)?;
        let limits = limits.validate()?;
        crate::daily_price_only::source::ScanLimits::new(raw_limits).map_err(|_| SourceClosureError::Limit)?;
        // At most seven complete archive scopes coexist with the tree;
        // historical consumers may temporarily retain their fixed four more.
        require(raw_limits.max_open_descriptors.checked_mul(11)
            .and_then(|n| n.checked_add(limits.max_members)).and_then(|n| n.checked_add(280))
            .is_some_and(|n| n <= limits.max_open_descriptors), SourceClosureError::Limit)?;
        raw_limits.max_retained_bytes.checked_mul(11)
            .and_then(|n| n.checked_add(limits.max_reference_bytes))
            .filter(|n| *n <= isize::MAX as usize).ok_or(SourceClosureError::Limit)?;
        require(paths.files_bundle != paths.database_stream, SourceClosureError::Member)?;
        self.codec = Some(InputOriginal::open(paths.files_bundle, expected, limits.max_stream_bytes)?);
        self.database = Some(InputOriginal::open(paths.database_stream, expected, limits.max_stream_bytes)?);
        let original = self.codec.as_ref().ok_or(SourceClosureError::Custody)?;
        let mut poll_codec = || continuing();
        let mut input = OriginalReader::new(original, &mut poll_codec);
        let mut magic = vec![0u8; MAGIC.len()]; input.read_exact(&mut magic).map_err(|_| SourceClosureError::Codec)?;
        require(magic == MAGIC, SourceClosureError::Codec)?;
        let mut length = [0u8; 8]; input.read_exact(&mut length).map_err(|_| SourceClosureError::Codec)?;
        let length = u64::from_be_bytes(length);
        require(length > 0 && length <= limits.max_manifest_bytes as u64, SourceClosureError::Limit)?;
        let mut bytes = vec![0u8; length as usize]; input.read_exact(&mut bytes).map_err(|_| SourceClosureError::Codec)?;
        let catalog = manifest(&bytes, limits)?;
        self.tree = Some(Materialized::open(paths.materialized, catalog, expected, limits, &mut || continuing())?);
        let originals = self.tree.as_ref().ok_or(SourceClosureError::Custody)?;
        originals.verify_codec(&mut input, &mut || continuing())?;
        self.codec_hash = Some(input.completed_hash()?); drop(input);
        let mut poll_database = || continuing();
        let mut input = BufReader::with_capacity(16_384, OriginalReader::new(self.database.as_ref().ok_or(SourceClosureError::Custody)?, &mut poll_database));
        let refs = decode_references_with_rows(&mut input, limits, &mut |group, table, key, row| {
            native_scope(invocation, expected, captured)?;
            originals.revalidate(&mut || continuing())?;
            let body = consume(ArchiveOriginalMember::Database {
                group, table, key, canonical_row: row,
            });
            let after = native_scope(invocation, expected, captured)
                .and_then(|()| originals.revalidate(&mut || continuing()));
            after.and(body)
        })?;
        self.database_hash = Some(input.get_ref().completed_hash()?); drop(input);
        relationships(&refs)?;
        for provider in raw_scope_names(originals)? {
            require(continuing(), SourceClosureError::Interrupted)?;
            let reader = crate::daily_price_only::source::CommittedRawReader::new(paths.materialized);
            let scan_limits = crate::daily_price_only::source::ScanLimits::new(raw_limits).map_err(|_| SourceClosureError::Limit)?;
            let source = match super::historical_provider(provider) {
                Ok(provider) => reader.historical_snapshot_checked_in(invocation, expected, captured,
                    paths.raw_coordination, provider, scan_limits),
                Err(_) => reader.publication_snapshot_checked_in(invocation, expected, captured,
                    paths.raw_coordination, publication_provider(provider)?, scan_limits),
            }.map_err(|_| SourceClosureError::Custody)?;
            self.sources.push(source);
        }
        self.references = Some(refs);
        self.revalidate(&mut || continuing())
    }

    /// Descriptive accounting only, for the protected caller's shared budget.
    pub fn reference_usage(&self) -> Result<(usize, usize)> {
        let refs = self.references.as_ref().ok_or(SourceClosureError::Custody)?;
        Ok((refs.retained_rows, refs.retained_bytes))
    }

    /// Reads the original complete SQL stream again while its original owner
    /// and every materialized descriptor stay live. Each row is descriptive;
    /// EOF, the duplicate reference projection and the original hash must all
    /// settle before this local traversal can succeed.
    pub fn visit_database_rows(
        &self, keep_reading: impl FnMut() -> bool,
        mut consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut guard = HistoricalUseGuard { attempt: self.expected, armed: true };
        let poll = std::cell::RefCell::new(keep_reading);
        let continuing = || (*poll.borrow_mut())();
        self.revalidate(&mut || continuing())?;
        let original = self.database.as_ref().ok_or(SourceClosureError::Custody)?;
        let mut keep = || continuing();
        let mut input = BufReader::with_capacity(16_384, OriginalReader::new(original, &mut keep));
        let _counts = decode_database_stream(&mut input, self.limits, false, &mut |group, table, key, row| {
            native_scope(self.invocation, self.expected, self.captured)?;
            let body = consume(ArchiveOriginalMember::Database { group, table, key, canonical_row: row });
            let after = native_scope(self.invocation, self.expected, self.captured);
            after.and(body)
        })?;
        require(Some(&input.get_ref().completed_hash()?) == self.database_hash.as_ref(), SourceClosureError::Custody)?;
        drop(input);
        self.revalidate(&mut || continuing())?;
        guard.armed = false;
        Ok(())
    }

    /// Original-backed reference/job rows, available BEFORE Curated replay.
    /// They can select exact reads on the caller's held SQL transaction; they
    /// do not establish that transaction, a dataset join, or serving authority.
    pub fn visit_reference_rows(
        &self, keep_reading: impl FnMut() -> bool,
        mut consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut guard = HistoricalUseGuard { attempt: self.expected, armed: true };
        let poll = std::cell::RefCell::new(keep_reading);
        let continuing = || (*poll.borrow_mut())();
        self.revalidate(&mut || continuing())?;
        let refs = self.references.as_ref().ok_or(SourceClosureError::Custody)?;
        for (table, rows) in &refs.tables {
            for (key, row) in rows {
                require(continuing(), SourceClosureError::Interrupted)?;
                let bytes = row.canonical()?;
                native_scope(self.invocation, self.expected, self.captured)?;
                let body = consume(ArchiveOriginalMember::Reference { table, key, canonical_row: &bytes });
                let after = native_scope(self.invocation, self.expected, self.captured);
                after.and(body)?;
            }
        }
        for (table, rows) in [("jobs", &refs.jobs), ("job_attempts", &refs.attempts)] {
            for (key, row) in rows {
                require(continuing(), SourceClosureError::Interrupted)?;
                let bytes = row.canonical()?;
                native_scope(self.invocation, self.expected, self.captured)?;
                let body = consume(ArchiveOriginalMember::Job { table, key, canonical_row: &bytes });
                let after = native_scope(self.invocation, self.expected, self.captured);
                after.and(body)?;
            }
        }
        self.revalidate(&mut || continuing())?;
        guard.armed = false;
        Ok(())
    }

    /// Uses the released native Raw/Daily/Curated algorithms under this SAME
    /// owner. The final domain guard is deliberately retained: this method
    /// cannot certify Paper/R4 completion from successful callbacks.
    pub fn visit_sources(
        &mut self, keep_reading: impl FnMut() -> bool,
        mut consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
    ) -> Result<()> {
        let _guard = HistoricalUseGuard { attempt: self.expected, armed: true };
        let invocation = self.invocation; let expected = self.expected; let captured = self.captured;
        let limits = self.limits; let raw_limits = self.raw_limits;
        let materialized = self.materialized.as_path();
        let raw_coordination = self.raw_coordination.as_path();
        let poll = std::cell::RefCell::new(keep_reading);
        let continuing = || (*poll.borrow_mut())();
        self.revalidate(&mut || continuing())?;
        // Exactly one source pass over the Raw leases captured by open_in.
        // A failed pass cannot be restarted on the same provisional owner.
        require(!self.source_pass_started, SourceClosureError::Custody)?;
        self.source_pass_started = true;
        let originals = self.tree.as_ref().ok_or(SourceClosureError::Custody)?;
        let refs = self.references.as_ref().ok_or(SourceClosureError::Custody)?;
        let raw = super::HistoricalRawReader { sources: &self.sources, expected, captured, keep_reading: &continuing };
        raw_membership(originals, &raw, refs)?;
        let selections = historical_selections(originals, refs)?;
        for selection in selections {
            let artifact_root = materialized.join(selection.class);
            let registry_root = materialized.join("simulation_approval");
            let mut callback_error = None;
            let checked = super::consume_historical_originals(invocation, expected, captured, selection.version,
                materialized, raw_coordination, &artifact_root, &registry_root, raw_limits,
                || continuing(), |member| {
                    let result = historical_member_matches(originals, &selection, member);
                    if let Err(error) = result { callback_error = Some(error); return Err(ArchiveSourceError::Pin); }
                    Ok(())
                });
            if let Some(error) = callback_error { return Err(error); }
            checked.map_err(|_| SourceClosureError::Reconstruction)?;
        }
        source_chains(originals, refs, &raw)?;
        daily_bundle_membership(originals, refs, materialized, invocation, expected, captured, &mut || continuing())?;
        daily_native_history(originals, refs, materialized, invocation, expected, captured,
            &self.sources, limits, &continuing)?;
        curated_source_dependencies(originals, refs, &self.sources, expected, captured, limits, &continuing)?;
        // Compute the adopted framing internally to exercise exact coupling.
        // It is intentionally inaccessible until the remaining graphs close.
        let _unexported_inventory_root = provisional_root(originals, refs)?;
        for row in class_rows(originals)? {
            let bytes = row.canonical()?;
            native_scope(invocation, expected, captured)?; originals.revalidate(&mut || continuing())?;
            let result = consume(ArchiveOriginalMember::Class { name: row.get("class")?.text()?, canonical_record: &bytes });
            let after = originals.revalidate(&mut || continuing()); after.and(result)?;
        }
        for (path, index) in &originals.manifest.by_path {
            let _identity_order = path;
            native_scope(invocation, expected, captured)?;
            let body = originals.visit_member(*index, &mut || continuing(), &mut consume);
            let after = native_scope(invocation, expected, captured); after.and(body)?;
        }
        for (table, rows) in &refs.tables {
            for (key, row) in rows {
                require(continuing(), SourceClosureError::Interrupted)?; let bytes = row.canonical()?;
                native_scope(invocation, expected, captured)?;
                let body = consume(ArchiveOriginalMember::Reference { table, key, canonical_row: &bytes });
                let after = native_scope(invocation, expected, captured); after.and(body)?;
            }
        }
        for (table, rows) in [("jobs", &refs.jobs), ("job_attempts", &refs.attempts)] {
            for (key, row) in rows {
                require(continuing(), SourceClosureError::Interrupted)?; let bytes = row.canonical()?;
                native_scope(invocation, expected, captured)?;
                let body = consume(ArchiveOriginalMember::Job { table, key, canonical_row: &bytes });
                let after = native_scope(invocation, expected, captured); after.and(body)?;
            }
        }
        native_scope(invocation, expected, captured)?;
        originals.revalidate(&mut || continuing())?;
        for (i, entry) in originals.manifest.entries.iter().enumerate() {
            if !entry.directory { originals.hash_member(i, &mut || continuing())?; }
        }
        Err(SourceClosureError::DomainClosureIncomplete)

    }

    /// Rechecks the named files, complete bytes and all actual retained Raw
    /// owners. It does not release any owner or settle the terminal Attempt.
    pub fn revalidate(&self, keep: &mut impl FnMut() -> bool) -> Result<()> {
        require(!self.retired, SourceClosureError::Custody)?;
        // Even a poisoned scope must not short-circuit the independent original
        // checks at finalization. Retirement below remains unconditional.
        let mut clean = native_scope(self.invocation, self.expected, self.captured).is_ok();
        for source in &self.sources {
            clean &= source.revalidate_originals_in(self.expected, self.captured).is_ok();
        }
        if let Some(tree) = &self.tree { clean &= tree.revalidate(keep).is_ok(); }
        else { clean = false; }
        for (original, initial) in [(&self.codec, &self.codec_hash), (&self.database, &self.database_hash)] {
            match (original, initial) {
                (Some(original), Some(initial)) => {
                    clean &= original.revalidate().is_ok();
                    clean &= original.hash(keep).is_ok_and(|hash| &hash == initial);
                }
                _ => clean = false,
            }
        }
        clean &= native_scope(self.invocation, self.expected, self.captured).is_ok();
        if !clean { self.expected.gap(Gap::Abandoned); }
        require(clean, SourceClosureError::Custody)
    }

    fn retire(&mut self) -> bool {
        if self.retired { return false; }
        let mut clean = true;
        // Drain/take before each operation so Drop never attempts a second
        // finish for that owner after a cleanup panic.
        for source in self.sources.drain(..) {
            clean &= archive_cleanup_step(self.expected, || source.finish_local().is_clean());
        }
        clean &= archive_cleanup_step(self.expected, || { drop(self.tree.take()); true });
        clean &= archive_cleanup_step(self.expected, || { drop(self.database.take()); true });
        clean &= archive_cleanup_step(self.expected, || { drop(self.codec.take()); true });
        self.references = None;
        self.retired = true;
        clean &= self.expected.check() && self.captured.check();
        clean
    }

    /// Consuming local close only. This does not finish the caller's Attempt,
    /// publish an audit, authorize RESTORE_OFF, or turn provisional rows into
    /// source authority. Every owner is retired even when final checks fail.
    pub fn finish_local(mut self, mut keep: impl FnMut() -> bool) -> Result<()> {
        let valid = archive_cleanup_step(self.expected, || self.revalidate(&mut keep).is_ok());
        if !valid { self.expected.gap(Gap::Abandoned); }
        let closed = self.retire();
        require(valid && closed, SourceClosureError::Custody)
    }
}
impl Drop for ArchiveOriginalOwner<'_, '_> {
    fn drop(&mut self) {
        if !self.retired {
            // Includes future cancellation/unwind between reads and finalization.
            // Drop is never positive completion, but still visits every owner.
            self.expected.gap(Gap::Abandoned);
            self.retire();
        }
    }
}

/// Compatible provisional traversal. The retained owner is consumed only after
/// all callbacks and final checks; panic resumes after explicit owner cleanup.
/// Whole-domain completion remains closed until the protected SQL/domain caller
/// is implemented. Successful callbacks do not clear DomainClosureIncomplete.
#[expect(clippy::too_many_arguments, reason = "Retain actual invocation, attempts, original paths and independent resource caps.")]
pub fn consume_archive_originals(
    invocation: &RawInvocation<'_>, expected: &Attempt, captured: &Attempt,
    paths: ArchiveOriginalPaths<'_>, limits: SourceClosureLimits,
    raw_limits: crate::daily_price_only::source::ScanLimitValues,
    keep_reading: impl FnMut() -> bool,
    mut consume: impl FnMut(ArchiveOriginalMember<'_>) -> Result<()>,
) -> Result<()> {
    let poll = std::cell::RefCell::new(keep_reading);
    let continuing = || (*poll.borrow_mut())();
    let mut owner = ArchiveOriginalOwner::open_in(
        invocation, expected, captured, paths, limits, raw_limits,
        || continuing(), &mut consume,
    )?;
    let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || owner.visit_sources(|| continuing(), &mut consume),
    ));
    let cleanup_keep = || archive_cleanup_step(expected, &continuing);
    let closed = owner.finish_local(cleanup_keep);
    match attempted {
        Err(payload) => std::panic::resume_unwind(payload),
        Ok(Err(primary)) => Err(primary),
        Ok(Ok(())) => closed,
    }
}

fn historical_member_matches(tree: &Materialized, selection: &HistoricalSelection,
    member: super::HistoricalSourceMember<'_>) -> Result<()> {
    use super::HistoricalSourceMember;
    let actual_file = |path: &str, file: &File, size: u64, sha: &ContentHash| -> Result<()> {
        let index = *tree.manifest.by_path.get(path).ok_or(SourceClosureError::HistoricalSelection)?;
        let entry = &tree.manifest.entries[index];
        require(!entry.directory && entry.size == size && entry.sha256.as_deref() == Some(sha.as_str()), SourceClosureError::Member)?;
        let info = rustix::fs::fstat(file).map_err(|_| SourceClosureError::Custody)?;
        require(FileIdentity::from_stat(&info, true) == tree.nodes[index].identity, SourceClosureError::Custody)?;
        tree.check_node(index)
    };
    match member {
        HistoricalSourceMember::ArtifactManifest { file, size, sha256 } => actual_file(&format!("{}/manifest.json", selection.prefix), file, size, sha256),
        HistoricalSourceMember::ArtifactBars { file, size, sha256 } => actual_file(&format!("{}/bars.ndjson", selection.prefix), file, size, sha256),
        HistoricalSourceMember::ApprovalRegistry { file, size, sha256 } => actual_file(&format!("simulation_approval/{}", selection.registry), file, size, sha256),
        HistoricalSourceMember::RawManifest { provider, bytes } => {
            require(tree.read(&format!("raw/manifests/provider={provider}/market=kr/manifest.jsonl"), MAX_LINE)? == bytes,
                SourceClosureError::Member)
        }
        // Complete batch/file membership and copied originals were independently
        // checked against the same retained tree by raw_membership above. These
        // callbacks still check byte equality to the catalog-selected member.
        HistoricalSourceMember::RawBatch { provider, batch_id, bytes } => match_raw_callback(tree, provider, batch_id, "batch.json", bytes),
        HistoricalSourceMember::RawFile { provider, batch_id, name, bytes } => match_raw_callback(tree, provider, batch_id, name, bytes),
    }
}
fn match_raw_callback(tree: &Materialized, provider: &str, id: domain::BatchId, name: &str, bytes: &[u8]) -> Result<()> {
    let prefix = format!("raw/provider={provider}/market=kr/date=");
    let suffix = format!("/batch={id}/{name}");
    let mut matching = tree.manifest.by_path.keys().filter(|p| p.starts_with(&prefix) && p.ends_with(&suffix));
    let path = matching.next().ok_or(SourceClosureError::Member)?;
    require(matching.next().is_none(), SourceClosureError::Duplicate)?;
    require(tree.read(path, MAX_LINE)? == bytes, SourceClosureError::Member)
}

#[cfg(test)]
mod tests;
