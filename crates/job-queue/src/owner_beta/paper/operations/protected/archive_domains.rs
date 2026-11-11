//! Descriptive historical journal checks. This module is not an authority or permit.
//! The original archive owner must remain alive and revalidate after consumption.
use super::super::{ErrorCode, wire};
use serde_json::Value;

use market_data::paper_archive_sources::closure::{ArchiveOriginalMember, SourceClosureError};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::FileExt,
};

const JOURNAL: &str = "paper_operations";
#[path = "archive_lifecycle.rs"]
mod lifecycle;

const LIFECYCLE: &str = "paper_evidence/lifecycle";
const RECEIPT_TABLES: &[&str] = &[
    "owner_beta_paper_operations",
    "owner_beta_paper_operation_observations",
    "owner_beta_paper_events",
];
const MAX_JOURNAL_MEMBER: usize = 3 * 1_048_576;

struct Member {
    // None denotes a directory; an empty file is distinct.
    bytes: Option<Vec<u8>>,
}

struct ReleaseMember {
    directory: bool,
    bytes: Option<Vec<u8>>,
    hash: Option<String>,
}

struct SimulationPairOriginal {
    row_hash: String,
    owner: String,
    illustrative: Vec<u8>,
    illustrative_sha256: String,
    zero: Vec<u8>,
    zero_sha256: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRow<'a> {
    #[serde(borrow)]
    run_id: &'a str,
    #[serde(borrow)]
    owner_user_id: &'a str,
    #[serde(borrow)]
    illustrative_canonical: &'a str,
    #[serde(borrow)]
    illustrative_sha256: &'a str,
    #[serde(borrow)]
    zero_cost_canonical: &'a str,
    #[serde(borrow)]
    zero_cost_sha256: &'a str,
    #[serde(borrow)]
    created_at: &'a str,
}

/// Bounded descriptive originals. This never represents a permission to restore.
pub(super) struct Domains {
    max_rows: usize,
    max_bytes: usize,
    used_rows: usize,
    used_bytes: usize,
    rows: BTreeMap<(String, String), Vec<u8>>,
    members: BTreeMap<String, Member>,
    release_members: BTreeMap<String, ReleaseMember>,
    simulation_pairs: BTreeMap<String, SimulationPairOriginal>,
    recommendations: BTreeSet<String>,
    deadline: Option<std::time::Instant>,
}
impl Domains {
    pub(super) fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            max_rows,
            max_bytes,
            used_rows: 0,
            used_bytes: 0,
            rows: BTreeMap::new(),
            members: BTreeMap::new(),
            release_members: BTreeMap::new(),
            simulation_pairs: BTreeMap::new(),
            recommendations: BTreeSet::new(),
            deadline: None,
        }
    }
    pub(super) fn until(max_rows: usize, max_bytes: usize, deadline: std::time::Instant) -> Self {
        let mut domains = Self::new(max_rows, max_bytes);
        domains.deadline = Some(deadline);
        domains
    }
    fn check_time(&self) -> Result<(), SourceClosureError> {
        if self
            .deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
        {
            Err(SourceClosureError::Interrupted)
        } else {
            Ok(())
        }
    }
    pub(super) fn retained_usage(&self) -> (usize, usize) {
        (self.used_rows, self.used_bytes)
    }
    fn charge(&mut self, n: usize) -> Result<(), SourceClosureError> {
        let rows = self
            .used_rows
            .checked_add(1)
            .ok_or(SourceClosureError::Limit)?;
        let bytes = self
            .used_bytes
            .checked_add(n)
            .and_then(|v| v.checked_add(256))
            .ok_or(SourceClosureError::Limit)?;
        if rows > self.max_rows || bytes > self.max_bytes {
            return Err(SourceClosureError::Limit);
        }
        self.used_rows = rows;
        self.used_bytes = bytes;
        Ok(())
    }
    pub(super) fn observe(
        &mut self,
        member: ArchiveOriginalMember<'_>,
    ) -> Result<(), SourceClosureError> {
        self.check_time()?;
        match member {
            ArchiveOriginalMember::HistoricalMaterial { artifact } => {
                self.recompute(RecommendationMaterial::Historical(artifact))
            }
            ArchiveOriginalMember::DailyMaterial { input } => {
                self.recompute(RecommendationMaterial::Daily(input))
            }
            ArchiveOriginalMember::Database {
                table: "owner_beta_simulation_result_pairs",
                key,
                canonical_row,
                ..
            } => self.observe_pair(key, canonical_row),
            ArchiveOriginalMember::Database {
                table,
                key,
                canonical_row,
                ..
            }
            | ArchiveOriginalMember::Reference {
                table,
                key,
                canonical_row,
            }
            | ArchiveOriginalMember::Job {
                table,
                key,
                canonical_row,
            } if RECEIPT_TABLES.contains(&table)
                || [
                    "owner_beta_paper_control",
                    "owner_beta_paper_lineage",
                    "owner_beta_recommendation_runs",
                    "owner_beta_recommendation_items",
                    "owner_beta_daily_reservations",
                    "owner_beta_daily_publication_claims",
                    "owner_beta_paper_source_runs",
                    "owner_beta_paper_source_slots",
                    "owner_beta_paper_source_admissions",
                    "user_strategy_configs",
                    "owner_beta_daily_admissions",
                    "owner_beta_simulation_runs",
                    "jobs",
                    "job_attempts",
                ]
                .contains(&table) =>
            {
                let k = (table.to_owned(), key.to_owned());
                if let Some(old) = self.rows.get(&k) {
                    return if old == canonical_row {
                        Ok(())
                    } else {
                        Err(SourceClosureError::Duplicate)
                    };
                }
                let cost = table
                    .len()
                    .checked_add(key.len())
                    .and_then(|n| n.checked_add(canonical_row.len().checked_mul(4)?))
                    .ok_or(SourceClosureError::Limit)?;
                self.charge(cost)?;
                let row: Value = serde_json::from_slice(canonical_row)
                    .map_err(|_| SourceClosureError::DatabaseRow)?;
                if !row.is_object() {
                    return Err(SourceClosureError::DatabaseRow);
                }
                self.rows.insert(k, canonical_row.to_vec());
                Ok(())
            }
            ArchiveOriginalMember::Member {
                path,
                canonical_record,
                file,
            } if path == JOURNAL
                || path.starts_with("paper_operations/")
                || path == LIFECYCLE
                || path.starts_with("paper_evidence/lifecycle/")
                || (path.starts_with("paper_evidence/source-")
                    && path.contains("/events/")
                    && file.is_some()) =>
            {
                if self.members.contains_key(path) {
                    return Err(SourceClosureError::Duplicate);
                }
                let record: Value = serde_json::from_slice(canonical_record)
                    .map_err(|_| SourceClosureError::Json)?;
                if !fields(
                    &record,
                    &["path", "kind", "uid", "gid", "mode", "size", "sha256"],
                ) || record["path"] != path
                    || record["uid"] != 0
                    || record["gid"]
                        != if path.starts_with("paper_evidence/source-") {
                            10001
                        } else {
                            0
                        }
                {
                    return Err(SourceClosureError::Metadata);
                }
                let size = record["size"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(SourceClosureError::Metadata)?;
                let directory = record["kind"] == "directory";
                let mode = if directory {
                    0o700
                } else if path.starts_with("paper_evidence/source-") {
                    0o440
                } else if path == "paper_operations/installation-id"
                    || path == "paper_operations/epoch-floor"
                {
                    0o600
                } else {
                    0o400
                };
                if (!directory && record["kind"] != "file")
                    || (directory && !record["sha256"].is_null())
                    || (!directory && !sha(&record["sha256"]))
                    || record["mode"] != mode
                    || directory != file.is_none()
                    || (directory && size != 0)
                    || size
                        > if path == LIFECYCLE || path.starts_with("paper_evidence/lifecycle/") {
                            if path.ends_with("/artifact.json")
                                || path.ends_with("/sql-request.json")
                                || path.contains("/output/")
                            {
                                1_048_576
                            } else if path.ends_with("/native-result.json") {
                                65_537
                            } else {
                                65_536
                            }
                        } else {
                            MAX_JOURNAL_MEMBER
                        }
                {
                    return Err(SourceClosureError::Metadata);
                }
                let cost = path
                    .len()
                    .checked_add(size)
                    .ok_or(SourceClosureError::Limit)?;
                self.charge(cost)?;
                let bytes = if let Some(file) = file {
                    let mut data = vec![0; size];
                    let mut at = 0;
                    while at < size {
                        self.check_time()?;
                        let n = file
                            .read_at(&mut data[at..], at as u64)
                            .map_err(|_| SourceClosureError::Custody)?;
                        if n == 0 {
                            return Err(SourceClosureError::Member);
                        }
                        at += n;
                    }
                    let mut extra = [0];
                    if file
                        .read_at(&mut extra, size as u64)
                        .map_err(|_| SourceClosureError::Custody)?
                        != 0
                    {
                        return Err(SourceClosureError::Member);
                    }
                    if record["sha256"] != crate::owner_beta::paper::source_evidence::digest(&data)
                    {
                        return Err(SourceClosureError::Member);
                    }
                    Some(data)
                } else {
                    None
                };
                self.members.insert(path.to_owned(), Member { bytes });
                Ok(())
            }
            ArchiveOriginalMember::Member {
                path,
                canonical_record,
                file,
            } if [
                "paper_config",
                "paper_contract",
                "paper_manifest",
                "paper_reader",
            ]
            .iter()
            .any(|class| {
                path == *class
                    || path
                        .strip_prefix(class)
                        .is_some_and(|rest| rest.starts_with('/'))
            }) =>
            {
                self.observe_release_member(path, canonical_record, file)
            }
            _ => Ok(()), // Other graph owners validate their own originals.
        }
    }
    fn file(&self, path: &str) -> Result<&[u8], ErrorCode> {
        self.members
            .get(path)
            .and_then(|m| m.bytes.as_deref())
            .ok_or(ErrorCode::EvidenceUnavailable)
    }
    fn directory(&self, path: &str) -> bool {
        self.members.get(path).is_some_and(|m| m.bytes.is_none())
    }
    fn children<'a>(&'a self, parent: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let prefix = format!("{parent}/");
        let stop_prefix = prefix.clone();
        self.members
            .range(prefix.clone()..)
            .take_while(move |(path, _)| path.starts_with(&stop_prefix))
            .filter_map(move |(path, _)| path.strip_prefix(&prefix))
            .filter(|name| !name.contains('/'))
    }
}

impl Domains {
    fn observe_release_member(
        &mut self,
        path: &str,
        record: &[u8],
        file: Option<&std::fs::File>,
    ) -> Result<(), SourceClosureError> {
        use sha2::{Digest, Sha256};
        if self.release_members.contains_key(path) {
            return Err(SourceClosureError::Duplicate);
        }
        let v: Value = serde_json::from_slice(record).map_err(|_| SourceClosureError::Json)?;
        if !fields(
            &v,
            &["path", "kind", "uid", "gid", "mode", "size", "sha256"],
        ) || v["path"] != path
            || v["uid"] != 0
        {
            return Err(SourceClosureError::Metadata);
        }
        let directory = v["kind"] == "directory";
        let mode = v["mode"].as_u64().ok_or(SourceClosureError::Metadata)?;
        let size = v["size"].as_u64().ok_or(SourceClosureError::Metadata)?;
        let elf = matches!(
            path,
            "paper_reader/api-image.elf"
                | "paper_reader/worker-image.elf"
                | "paper_reader/native-image.elf"
        );
        let expected_mode = match path {
            "paper_config/process.json" | "paper_config/config.json" => Some(0o440),
            "paper_reader/capability.json" | "paper_reader/migration-0069.up.sql" => Some(0o444),
            "paper_reader/api-image.elf" | "paper_reader/worker-image.elf" => Some(0o400),
            "paper_reader/native-image.elf" => Some(0o500),
            "paper_reader/install-receipt.json" | "paper_manifest/.lagrange-release-manifest" => {
                Some(0o600)
            }
            _ if path.starts_with("paper_reader/provenance/") && !directory => Some(0o444),
            _ if directory => Some(0o700),
            _ => None,
        };
        let group = if path.starts_with("paper_config/") && !directory {
            10001
        } else {
            0
        };
        if v["gid"] != group
            || mode > 0o777
            || mode & 0o022 != 0
            || expected_mode.is_some_and(|m| m != mode)
            || directory != file.is_none()
            || (directory && (size != 0 || !v["sha256"].is_null()))
            || (!directory && (v["kind"] != "file" || !sha(&v["sha256"])))
        {
            return Err(SourceClosureError::Metadata);
        }
        let maximum = if elf {
            128_u64 * 1024 * 1024
        } else {
            1_048_576
        };
        if size > maximum {
            return Err(SourceClosureError::Limit);
        }
        let keep_size = if elf {
            71
        } else {
            usize::try_from(size).map_err(|_| SourceClosureError::Limit)?
        };
        self.charge(
            path.len()
                .checked_add(keep_size)
                .ok_or(SourceClosureError::Limit)?,
        )?;
        let (bytes, hash) = if let Some(fd) = file {
            let mut offset = 0u64;
            let mut out = if elf {
                Vec::new()
            } else {
                Vec::with_capacity(keep_size)
            };
            let mut buffer = [0u8; 16_384];
            let mut digest = Sha256::new();
            let mut header = [0u8; 20];
            while offset < size {
                let wanted = usize::try_from((size - offset).min(buffer.len() as u64))
                    .map_err(|_| SourceClosureError::Limit)?;
                let n = fd
                    .read_at(&mut buffer[..wanted], offset)
                    .map_err(|_| SourceClosureError::Custody)?;
                if n == 0 {
                    return Err(SourceClosureError::Member);
                }
                if offset < 20 {
                    let begin = offset as usize;
                    let count = n.min(20 - begin);
                    header[begin..begin + count].copy_from_slice(&buffer[..count]);
                }
                digest.update(&buffer[..n]);
                if !elf {
                    out.extend_from_slice(&buffer[..n]);
                }
                offset += n as u64;
            }
            let mut extra = [0u8; 1];
            if fd
                .read_at(&mut extra, offset)
                .map_err(|_| SourceClosureError::Custody)?
                != 0
            {
                return Err(SourceClosureError::Member);
            }
            let hash = format!("sha256:{:x}", digest.finalize());
            if v["sha256"] != hash
                || (elf
                    && (size < 20
                        || &header[..7] != b"\x7fELF\x02\x01\x01"
                        || header[18..20] != [62, 0]))
            {
                return Err(SourceClosureError::Member);
            }
            (if elf { None } else { Some(out) }, Some(hash))
        } else {
            (None, None)
        };
        self.release_members.insert(
            path.to_owned(),
            ReleaseMember {
                directory,
                bytes,
                hash,
            },
        );
        Ok(())
    }
    fn release_bytes(&self, path: &str) -> Result<&[u8], ErrorCode> {
        self.release_members
            .get(path)
            .and_then(|m| m.bytes.as_deref())
            .ok_or(ErrorCode::EvidenceUnavailable)
    }
    fn release_hash(&self, path: &str) -> Result<&str, ErrorCode> {
        self.release_members
            .get(path)
            .and_then(|m| m.hash.as_deref())
            .ok_or(ErrorCode::EvidenceUnavailable)
    }
}

fn decode_hex(text: &str) -> Result<Vec<u8>, SourceClosureError> {
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(SourceClosureError::DatabaseRow);
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let digit = |b| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        decoded.push(
            (digit(pair[0]).ok_or(SourceClosureError::DatabaseRow)? << 4)
                | digit(pair[1]).ok_or(SourceClosureError::DatabaseRow)?,
        );
    }
    Ok(decoded)
}
impl Domains {
    fn observe_pair(&mut self, key: &str, raw: &[u8]) -> Result<(), SourceClosureError> {
        let parsed: PairRow<'_> =
            serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
        if parsed.run_id != key
            || !uuid(&Value::String(parsed.run_id.to_owned()))
            || !uuid(&Value::String(parsed.owner_user_id.to_owned()))
            || parsed.created_at.is_empty()
        {
            return Err(SourceClosureError::DatabaseRow);
        }
        let row_hash = digest_bytes(raw);
        if let Some(old) = self.simulation_pairs.get(parsed.run_id) {
            return if old.row_hash == row_hash {
                Ok(())
            } else {
                Err(SourceClosureError::Duplicate)
            };
        }
        let total = parsed
            .illustrative_canonical
            .len()
            .checked_add(parsed.zero_cost_canonical.len())
            .ok_or(SourceClosureError::Limit)?;
        if total > 2 * 67_108_864 || total % 2 != 0 {
            return Err(SourceClosureError::Limit);
        }
        let cost = total
            .checked_div(2)
            .and_then(|v| v.checked_add(4 * 71 + parsed.run_id.len() + parsed.owner_user_id.len()))
            .ok_or(SourceClosureError::Limit)?;
        self.charge(cost)?;
        let illustrative = decode_hex(parsed.illustrative_canonical)?;
        let zero = decode_hex(parsed.zero_cost_canonical)?;
        if digest_bytes(&illustrative) != parsed.illustrative_sha256
            || digest_bytes(&zero) != parsed.zero_cost_sha256
        {
            return Err(SourceClosureError::Relationship);
        }
        self.simulation_pairs.insert(
            parsed.run_id.to_owned(),
            SimulationPairOriginal {
                row_hash,
                owner: parsed.owner_user_id.to_owned(),
                illustrative,
                illustrative_sha256: parsed.illustrative_sha256.to_owned(),
                zero,
                zero_sha256: parsed.zero_cost_sha256.to_owned(),
            },
        );
        Ok(())
    }
}

fn historical_root(bytes: &[u8]) -> Result<wire::RootIntent, ErrorCode> {
    let v = wire::parse_canonical(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    let keys = [
        "schema",
        "installation_id",
        "generation",
        "account_id",
        "predecessor_generation",
        "request_key",
        "purpose",
        "scope",
        "expected_epoch",
        "local_epoch_floor",
        "release_manifest_sha256",
        "artifact_sha256",
        "resume_target_id",
    ];
    let text = |key: &str| v[key].as_str().ok_or(ErrorCode::WireInvalid);
    let parse_id = |key: &str| -> Result<Option<uuid::Uuid>, ErrorCode> {
        if v[key].is_null() {
            return Ok(None);
        }
        if !uuid(&v[key]) {
            return Err(ErrorCode::WireInvalid);
        }
        Ok(Some(
            uuid::Uuid::parse_str(text(key)?).map_err(|_| ErrorCode::WireInvalid)?,
        ))
    };
    if !fields(&v, &keys)
        || v["schema"] != "owner-beta-paper-operation-intent-v1"
        || !sha(&v["release_manifest_sha256"])
        || !nullable_sha(&v["artifact_sha256"])
    {
        return Err(ErrorCode::WireInvalid);
    }
    let installation_id = parse_id("installation_id")?.ok_or(ErrorCode::WireInvalid)?;
    let generation = parse_id("generation")?.ok_or(ErrorCode::WireInvalid)?;
    let account_id = parse_id("account_id")?;
    let predecessor = parse_id("predecessor_generation")?;
    parse_id("resume_target_id")?;
    let key = text("request_key")?;
    let purpose = text("purpose")?;
    let scope = text("scope")?;
    let epoch = v["expected_epoch"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or(ErrorCode::WireInvalid)?;
    let floor = v["local_epoch_floor"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or(ErrorCode::WireInvalid)?;
    if predecessor == Some(generation)
        || key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        || !matches!(
            purpose,
            "CONFIGURE" | "INSPECT" | "ENABLE" | "STOP" | "BACKUP" | "RESTORE" | "RECOVERY"
        )
        || !matches!(scope, "PAPER_STOP" | "COMBINED_BACKUP")
        || (purpose != "RESTORE" && floor != epoch)
    {
        return Err(ErrorCode::IdentityConflict);
    }
    if matches!(purpose, "CONFIGURE" | "INSPECT") {
        if (account_id.is_none() && epoch != 0)
            || (purpose == "CONFIGURE" && account_id.is_some())
            || predecessor.is_some()
        {
            return Err(ErrorCode::IdentityConflict);
        }
    } else if account_id.is_none()
        || (matches!(purpose, "ENABLE" | "RECOVERY") && predecessor.is_some())
    {
        return Err(ErrorCode::IdentityConflict);
    }
    if (purpose == "STOP" && scope != "PAPER_STOP")
        || (matches!(purpose, "BACKUP" | "RESTORE") && scope != "COMBINED_BACKUP")
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let suffix = key.strip_prefix(&format!("{generation}:"));
    let valid_key = match purpose {
        "ENABLE" => suffix == Some("enable"),
        "RECOVERY" => suffix == Some("recover"),
        "STOP" | "BACKUP" | "RESTORE" => match suffix {
            Some("begin") => true,
            Some("off") => purpose != "RESTORE",
            Some("restore-off") => purpose == "RESTORE",
            Some(s) => s.strip_prefix("reconcile:").is_some_and(|n| {
                !n.is_empty() && n.len() <= 18 && n.bytes().all(|b| b.is_ascii_digit())
            }),
            None => false,
        },
        _ => true,
    };
    if !valid_key {
        return Err(ErrorCode::IdentityConflict);
    }
    let intent = wire::Intent {
        installation_id,
        generation,
        account_id,
        request_key: key.to_owned(),
        purpose: purpose.to_owned(),
        scope: scope.to_owned(),
        expected_epoch: epoch,
        local_epoch_floor: floor,
        release_manifest_sha256: text("release_manifest_sha256")?.to_owned(),
        artifact_sha256: v["artifact_sha256"].as_str().map(str::to_owned),
        value: v.clone(),
    };
    Ok(wire::RootIntent {
        intent,
        canonical: String::from_utf8(bytes.to_vec()).map_err(|_| ErrorCode::WireInvalid)?,
        sha256: crate::owner_beta::paper::source_evidence::digest(bytes),
    })
}

const STAGE_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "sequence",
    "previous_sha256",
    "intent_sha256",
    "stage",
    "boot_id",
    "observed",
    "control_epoch",
    "artifact_sha256",
    "event_id",
    "error_code",
];

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.strip_prefix("sha256:").is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
    })
}
fn nullable_sha(value: &Value) -> bool {
    value.is_null() || sha(value)
}
fn uuid(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok_and(|u| !u.is_nil() && u.to_string() == s))
}
fn fields(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
}

/// Checks one original in order, including its link to the actual prior bytes.
/// It does not check file inventory, action/receipt links, or present custody.
struct StageChain<'a> {
    root: &'a wire::RootIntent,
    previous: Option<(String, String, i64)>,
    sequence: i64,
    floor: i64,
    bound: bool,
}
impl<'a> StageChain<'a> {
    fn new(root: &'a wire::RootIntent, floor: i64) -> Result<Self, ErrorCode> {
        if floor < root.intent.local_epoch_floor {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(Self {
            root,
            previous: None,
            sequence: 0,
            floor,
            bound: true,
        })
    }
    fn observe(&mut self, bytes: &[u8]) -> Result<(), ErrorCode> {
        let v = wire::parse_canonical(bytes).map_err(|_| ErrorCode::WireInvalid)?;
        let root = &self.root.intent;
        let stage = v["stage"].as_str().ok_or(ErrorCode::WireInvalid)?;
        let error = v["error_code"].as_str().ok_or(ErrorCode::WireInvalid)?;
        let epoch = v["control_epoch"]
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or(ErrorCode::WireInvalid)?;
        let prior = self.previous.as_ref();
        let prev = prior
            .map(|p| Value::String(p.1.clone()))
            .unwrap_or(Value::Null);
        let valid_error = matches!(
            error,
            "NONE"
                | "WIRE_INVALID"
                | "IDENTITY_CONFLICT"
                | "STALE_EPOCH"
                | "CLOCK_INVALID"
                | "WRITER_UNKNOWN"
                | "COMMIT_UNKNOWN"
                | "READER_INCOMPATIBLE"
                | "RECOVERY_AUTHORITY_UNAVAILABLE"
                | "EVIDENCE_UNAVAILABLE"
                | "INTERNAL_FAILURE"
        );
        if !fields(&v, STAGE_KEYS)
            || v["schema"] != "owner-beta-paper-operation-stage-v1"
            || v["installation_id"] != root.installation_id.to_string()
            || v["generation"] != root.generation.to_string()
            || root
                .account_id
                .is_none_or(|id| v["account_id"] != id.to_string())
            || v["sequence"] != self.sequence
            || v["previous_sha256"] != prev
            || v["intent_sha256"] != self.root.sha256
            || !uuid(&v["boot_id"])
            || !(v["event_id"].is_null() || uuid(&v["event_id"]))
            || !nullable_sha(&v["artifact_sha256"])
            || !valid_error
            || (stage == "FAILED") != (error != "NONE")
            || epoch > self.floor
            || !fields(
                &v["observed"],
                &["realtime_ns", "monotonic_ns", "boottime_ns"],
            )
            || ["realtime_ns", "monotonic_ns", "boottime_ns"]
                .iter()
                .any(|k| !v["observed"][*k].as_i64().is_some_and(|n| n >= 0))
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let purpose = root.purpose.as_str();
        let allowed = match purpose {
            "STOP" => matches!(
                stage,
                "INTENT" | "FENCED" | "STOPPED" | "RECONCILED" | "OFF_COMMITTED" | "FAILED"
            ),
            "BACKUP" => matches!(
                stage,
                "INTENT"
                    | "FENCED"
                    | "STOPPED"
                    | "RECONCILED"
                    | "OFF_COMMITTED"
                    | "SNAPSHOT"
                    | "FAILED"
            ),
            "RESTORE" => matches!(
                stage,
                "INTENT"
                    | "FENCED"
                    | "STOPPED"
                    | "RECONCILED"
                    | "RESTORE_VERIFIED"
                    | "RESTORED_COMMITTED"
                    | "FAILED"
            ),
            _ => {
                !self.bound
                    && matches!(
                        stage,
                        "INTENT"
                            | "FENCED"
                            | "STOPPED"
                            | "RECONCILED"
                            | "OFF_COMMITTED"
                            | "SNAPSHOT"
                            | "RESTORE_VERIFIED"
                            | "RESTORED_COMMITTED"
                            | "FAILED"
                    )
            }
        };
        let advances = match (prior.map(|p| p.0.as_str()), stage) {
            (None, "INTENT") => true,
            (Some("INTENT"), "FENCED")
            | (Some("FENCED"), "STOPPED")
            | (Some("STOPPED"), "RECONCILED")
            | (Some("RECONCILED"), "RECONCILED") => true,
            (Some("RECONCILED"), "OFF_COMMITTED") => purpose != "RESTORE",
            (Some("OFF_COMMITTED"), "SNAPSHOT") => purpose != "STOP" && purpose != "RESTORE",
            (Some("SNAPSHOT"), "RESTORE_VERIFIED") => {
                purpose != "STOP" && purpose != "BACKUP" && purpose != "RESTORE"
            }
            (Some("RECONCILED"), "RESTORE_VERIFIED") => purpose == "RESTORE",
            (Some("RESTORE_VERIFIED"), "RESTORED_COMMITTED") => {
                purpose != "STOP" && purpose != "BACKUP"
            }
            (Some(p), "FAILED") => {
                !matches!(p, "FAILED" | "SNAPSHOT" | "RESTORED_COMMITTED")
                    && !(p == "OFF_COMMITTED" && purpose == "STOP")
            }
            _ => false,
        };
        if !allowed || !advances {
            return Err(ErrorCode::IdentityConflict);
        }
        if self.bound {
            let fence = self
                .root
                .fence_epoch()
                .map_err(|_| ErrorCode::IdentityConflict)?;
            let expected = match stage {
                "INTENT" => root.expected_epoch,
                "FAILED" => prior.ok_or(ErrorCode::IdentityConflict)?.2,
                "OFF_COMMITTED" | "SNAPSHOT" | "RESTORED_COMMITTED" => {
                    fence.checked_add(1).ok_or(ErrorCode::IdentityConflict)?
                }
                _ => fence,
            };
            if epoch != expected {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ErrorCode::IdentityConflict)?;
        self.previous = Some((
            stage.to_owned(),
            crate::owner_beta::paper::source_evidence::digest(bytes),
            epoch,
        ));
        Ok(())
    }
}

/// Validates ordered binding originals and an optional corresponding dispatch.
/// Inventory and SQL receipt ownership remain separate obligations.
struct ActionChain<'a> {
    root: &'a wire::RootIntent,
    sequence: i64,
    previous: Option<String>,
}
impl<'a> ActionChain<'a> {
    fn new(root: &'a wire::RootIntent) -> Self {
        Self {
            root,
            sequence: 0,
            previous: None,
        }
    }
    fn observe(&mut self, binding: &[u8], dispatch: Option<&[u8]>) -> Result<(), ErrorCode> {
        let action = wire::parse_action(binding, self.root).map_err(|_| ErrorCode::WireInvalid)?;
        if action.sequence != self.sequence || action.previous_binding_sha256 != self.previous {
            return Err(ErrorCode::IdentityConflict);
        }
        if let Some(bytes) = dispatch {
            let parsed = wire::BoundDispatch::parse(bytes).map_err(|_| ErrorCode::WireInvalid)?;
            if parsed.root.canonical != self.root.canonical
                || parsed.action.canonical != action.canonical
            {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ErrorCode::IdentityConflict)?;
        self.previous = Some(action.sha256);
        Ok(())
    }
}

struct JournalIndex<'a> {
    rows: Vec<(&'a str, Value)>,
    operations: BTreeMap<String, usize>,
    callers: std::collections::BTreeSet<String>,
    requests: BTreeMap<String, &'a [u8]>,
    objects: BTreeMap<String, &'a [u8]>,
    // Exact response text borrowed semantically from retained COMMITTED
    // envelopes, keyed by original Q. These descriptions confer no authority.
    enclosed_results: BTreeMap<String, String>,
    extra_rows: usize,
    extra_bytes: usize,
}
impl<'a> JournalIndex<'a> {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            operations: BTreeMap::new(),
            callers: std::collections::BTreeSet::new(),
            requests: BTreeMap::new(),
            objects: BTreeMap::new(),
            enclosed_results: BTreeMap::new(),
            extra_rows: 0,
            extra_bytes: 0,
        }
    }
    fn charge(&mut self, domains: &Domains, bytes: usize) -> Result<(), ErrorCode> {
        let rows = self
            .extra_rows
            .checked_add(1)
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        let bytes = self
            .extra_bytes
            .checked_add(bytes)
            .and_then(|n| n.checked_add(512))
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        if domains
            .used_rows
            .checked_add(rows)
            .is_none_or(|n| n > domains.max_rows)
            || domains
                .used_bytes
                .checked_add(bytes)
                .is_none_or(|n| n > domains.max_bytes)
        {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        self.extra_rows = rows;
        self.extra_bytes = bytes;
        Ok(())
    }
}
impl Domains {
    fn journal_index<'a>(
        &'a self,
        callers: &str,
        installation: &str,
    ) -> Result<JournalIndex<'a>, ErrorCode> {
        let mut index = JournalIndex::new();
        for ((table, _), bytes) in &self.rows {
            if !RECEIPT_TABLES.contains(&table.as_str()) {
                continue;
            }
            let cost = bytes
                .len()
                .checked_mul(8)
                .ok_or(ErrorCode::EvidenceUnavailable)?;
            index.charge(self, cost)?;
            let row: Value = serde_json::from_slice(bytes).map_err(|_| ErrorCode::WireInvalid)?;
            if table == RECEIPT_TABLES[0] {
                let generation = row["generation"].as_str().ok_or(ErrorCode::WireInvalid)?;
                index.charge(self, generation.len())?;
                if index
                    .operations
                    .insert(generation.to_owned(), index.rows.len())
                    .is_some()
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
            index.rows.push((table.as_str(), row));
        }
        for name in self.children(callers) {
            let v = self.validate_caller(&format!("{callers}/{name}"), installation)?;
            let generation = v["generation"].as_str().ok_or(ErrorCode::WireInvalid)?;
            if !index.callers.contains(generation) {
                index.charge(self, generation.len())?;
                index.callers.insert(generation.to_owned());
            }
        }
        for (path, member) in &self.members {
            let Some(relative) = path.strip_prefix("paper_operations/operations/") else {
                continue;
            };
            let parts: Vec<_> = relative.split('/').collect();
            let is_output = parts.len() == 5 && parts[1] == "actions" && parts[3] == "output";
            if is_output && parts[4] == "dispatch.json" {
                let bytes = member.bytes.as_deref().ok_or(ErrorCode::IdentityConflict)?;
                let dispatch =
                    wire::BoundDispatch::parse(bytes).map_err(|_| ErrorCode::WireInvalid)?;
                index.charge(self, dispatch.request.sha256.len())?;
                if index
                    .requests
                    .insert(dispatch.request.sha256, bytes)
                    .is_some()
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            } else if (parts.len() == 2 || is_output)
                && named_digest(parts[parts.len() - 1]).is_some()
            {
                let hash = named_digest(parts[parts.len() - 1]).ok_or(ErrorCode::WireInvalid)?;
                let bytes = member.bytes.as_deref().ok_or(ErrorCode::IdentityConflict)?;
                if digest_bytes(bytes) != hash {
                    return Err(ErrorCode::IdentityConflict);
                }
                wire::parse_canonical_limit(bytes, MAX_JOURNAL_MEMBER)
                    .map_err(|_| ErrorCode::WireInvalid)?;
                if let Some(old) = index.objects.get(&hash) {
                    if *old != bytes {
                        return Err(ErrorCode::IdentityConflict);
                    }
                } else {
                    index.charge(self, hash.len())?;
                    index.objects.insert(hash, bytes);
                }
            }
        }
        // Normal CONFIGURE and RECOVER retain the whole ExactReadback, not
        // a second file containing only its SQL response. Build this index only
        // after every original Q/dispatch/object is available, without trusting
        // stdout or requiring the nested result to have its own filename.
        for ((table, _), bytes) in &self.rows {
            if table == "owner_beta_paper_control" {
                index.charge(
                    self,
                    bytes
                        .len()
                        .checked_mul(8)
                        .ok_or(ErrorCode::EvidenceUnavailable)?,
                )?;
            }
        }
        // One reusable parse workspace; only distinct admitted nested strings
        // stay allocated. Do not charge a fresh full parse for every old copy.
        let largest = index
            .objects
            .values()
            .map(|bytes| bytes.len())
            .max()
            .unwrap_or(0);
        if largest > 0 {
            index.charge(
                self,
                largest
                    .checked_mul(16)
                    .ok_or(ErrorCode::EvidenceUnavailable)?,
            )?;
        }
        for (path, member) in &self.members {
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let Some(relative) = path.strip_prefix("paper_operations/operations/") else {
                continue;
            };
            let parts: Vec<_> = relative.split('/').collect();
            if parts.len() != 5
                || parts[1] != "actions"
                || parts[3] != "output"
                || named_digest(parts[4]).is_none()
            {
                continue;
            }
            let bytes = member.bytes.as_deref().ok_or(ErrorCode::IdentityConflict)?;
            let value = wire::parse_canonical_limit(bytes, MAX_JOURNAL_MEMBER)
                .map_err(|_| ErrorCode::WireInvalid)?;
            if value["schema"] != "owner-beta-paper-exact-readback-v1" {
                continue;
            }
            let qhash = value["request_sha256"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?;
            let raw = index
                .requests
                .get(qhash)
                .copied()
                .ok_or(ErrorCode::EvidenceUnavailable)?;
            let dispatch = wire::BoundDispatch::parse(raw).map_err(|_| ErrorCode::WireInvalid)?;
            if parts[0] != dispatch.root.intent.generation.to_string()
                || parts[2] != format!("{:08}", dispatch.action.sequence)
            {
                return Err(ErrorCode::IdentityConflict);
            }
            let read = wire::ExactReadback::parse(value, &dispatch.request)
                .map_err(|_| ErrorCode::WireInvalid)?;
            read.correlate_original(&dispatch.root.intent)
                .map_err(|_| ErrorCode::IdentityConflict)?;
            self.check_readback_current_epoch(&read.value)?;
            if read.value["status"] != "COMMITTED" {
                continue;
            }
            self.check_committed_readback(&read, &dispatch, &index)?;
            let result = read.value["result_canonical"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?;
            if let Some(old) = index.enclosed_results.get(&dispatch.request.sha256) {
                if old != result {
                    return Err(ErrorCode::IdentityConflict);
                }
            } else {
                index.charge(
                    self,
                    dispatch
                        .request
                        .sha256
                        .len()
                        .checked_add(result.len())
                        .ok_or(ErrorCode::EvidenceUnavailable)?,
                )?;
                index
                    .enclosed_results
                    .insert(dispatch.request.sha256, result.to_owned());
            }
        }
        Ok(index)
    }
}

fn digest_bytes(bytes: &[u8]) -> String {
    crate::owner_beta::paper::source_evidence::digest(bytes)
}
fn named_digest(name: &str) -> Option<String> {
    let hex = name.strip_suffix(".json")?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    Some(format!("sha256:{hex}"))
}
fn request_key(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    })
}
impl Domains {
    fn copy_at(&self, directory: &str, bytes: &[u8]) -> Result<(), ErrorCode> {
        let hash = digest_bytes(bytes);
        let path = format!("{directory}/{}.json", &hash[7..]);
        if self.file(&path)? != bytes {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
    fn validate_caller(&self, path: &str, installation: &str) -> Result<Value, ErrorCode> {
        let v = wire::parse_canonical(self.file(path)?).map_err(|_| ErrorCode::WireInvalid)?;
        if !fields(
            &v,
            &[
                "schema",
                "installation_id",
                "account_id",
                "caller_key",
                "generation",
                "root_intent_sha256",
                "command_canonical",
            ],
        ) || v["schema"] != "owner-beta-paper-caller-binding-v1"
            || v["installation_id"] != installation
            || !(v["account_id"].is_null() || uuid(&v["account_id"]))
            || !uuid(&v["generation"])
            || !sha(&v["root_intent_sha256"])
            || !request_key(&v["caller_key"])
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let projection = serde_json::json!({"installation_id":v["installation_id"],"account_id":v["account_id"],"caller_key":v["caller_key"]});
        let canonical = crate::owner_beta::paper::source_evidence::canonical(&projection)
            .map_err(|_| ErrorCode::WireInvalid)?;
        let expected = digest_bytes(&canonical);
        if path.rsplit('/').next() != Some(format!("{}.json", &expected[7..]).as_str()) {
            return Err(ErrorCode::IdentityConflict);
        }
        let generation = v["generation"].as_str().ok_or(ErrorCode::WireInvalid)?;
        let root_path = format!("{JOURNAL}/operations/{generation}/intent.json");
        let root =
            wire::parse_root_intent(self.file(&root_path)?).map_err(|_| ErrorCode::WireInvalid)?;
        if root.intent.purpose == "INSPECT"
            || root.intent.installation_id.to_string() != installation
            || v["root_intent_sha256"] != root.sha256
            || v["account_id"] != root.intent.value["account_id"]
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut command = root.intent.value.clone();
        let map = command.as_object_mut().ok_or(ErrorCode::WireInvalid)?;
        map.remove("generation");
        map.remove("request_key");
        let bytes = crate::owner_beta::paper::source_evidence::canonical(&command)
            .map_err(|_| ErrorCode::WireInvalid)?;
        if v["command_canonical"].as_str().map(str::as_bytes) != Some(bytes.as_slice())
            || (root.intent.purpose == "CONFIGURE" && v["caller_key"] != root.intent.request_key)
        {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(v)
    }
    fn row_values(&self) -> impl Iterator<Item = Result<(&str, Value), ErrorCode>> {
        self.rows
            .iter()
            .filter(|((table, _), _)| RECEIPT_TABLES.contains(&table.as_str()))
            .map(|((table, _), bytes)| {
                serde_json::from_slice(bytes)
                    .map(|v| (table.as_str(), v))
                    .map_err(|_| ErrorCode::WireInvalid)
            })
    }
    pub(super) fn finish_journal(&self, _archive: &Value, _audit: &Value) -> Result<(), ErrorCode> {
        let base = JOURNAL;
        if !self.directory(base) {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let names: Vec<_> = self.children(base).collect();
        if names.is_empty() {
            for row in self.row_values() {
                let (table, v) = row?;
                if table == RECEIPT_TABLES[0]
                    || [
                        "request_canonical",
                        "request_sha256",
                        "result_canonical",
                        "result_sha256",
                    ]
                    .iter()
                    .any(|k| !v[*k].is_null())
                {
                    return Err(ErrorCode::EvidenceUnavailable);
                }
            }
            return Ok(());
        }
        if names
            .iter()
            .any(|n| !["installation-id", "epoch-floor", "operations", "callers"].contains(n))
            || !self.directory(&format!("{base}/operations"))
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let ident = self.file(&format!("{base}/installation-id"))?;
        let installation = std::str::from_utf8(ident)
            .ok()
            .and_then(|s| s.strip_suffix('\n'))
            .ok_or(ErrorCode::WireInvalid)?;
        if !uuid(&Value::String(installation.to_owned())) {
            return Err(ErrorCode::WireInvalid);
        }
        let floor_raw = self.file(&format!("{base}/epoch-floor"))?;
        let floor_text = std::str::from_utf8(floor_raw)
            .ok()
            .and_then(|s| s.strip_suffix('\n'))
            .ok_or(ErrorCode::WireInvalid)?;
        if floor_text.is_empty()
            || floor_text.len() > 19
            || (floor_text.len() > 1 && floor_text.starts_with('0'))
            || !floor_text.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ErrorCode::WireInvalid);
        }
        let floor: i64 = floor_text.parse().map_err(|_| ErrorCode::WireInvalid)?;
        let callers = format!("{base}/callers");
        if names.contains(&"callers") && !self.directory(&callers) {
            return Err(ErrorCode::IdentityConflict);
        }
        let index = self.journal_index(&callers, installation)?;
        let operations = format!("{base}/operations");
        for generation in self.children(&operations) {
            if !uuid(&Value::String(generation.to_owned())) {
                return Err(ErrorCode::WireInvalid);
            }
            let dir = format!("{operations}/{generation}");
            if !self.directory(&dir) {
                return Err(ErrorCode::IdentityConflict);
            }
            let root_bytes = self.file(&format!("{dir}/intent.json"))?;
            let root = historical_root(root_bytes)?;
            if root.intent.generation.to_string() != generation
                || root.intent.installation_id.to_string() != installation
                || root.intent.local_epoch_floor > floor
            {
                return Err(ErrorCode::IdentityConflict);
            }
            let actions = format!("{dir}/actions");
            let has_actions = self.members.contains_key(&actions);
            if has_actions && !self.directory(&actions) {
                return Err(ErrorCode::IdentityConflict);
            }
            let bound = has_actions || index.callers.contains(generation);
            if bound {
                wire::parse_root_intent(root_bytes).map_err(|_| ErrorCode::WireInvalid)?;
                if root.intent.local_epoch_floor < root.intent.expected_epoch {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
            let mut stages = StageChain::new(&root, floor)?;
            stages.bound = bound;
            for name in self.children(&dir) {
                if name == "intent.json" || name == "actions" {
                    continue;
                }
                if let Some(hash) = named_digest(name) {
                    let bytes = self.file(&format!("{dir}/{name}"))?;
                    wire::parse_canonical_limit(bytes, MAX_JOURNAL_MEMBER)
                        .map_err(|_| ErrorCode::WireInvalid)?;
                    if digest_bytes(bytes) != hash {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    continue;
                }
                let expected = format!("{:08}.json", stages.sequence);
                if name != expected {
                    return Err(ErrorCode::IdentityConflict);
                }
                let bytes = self.file(&format!("{dir}/{name}"))?;
                stages.observe(bytes)?;
                self.copy_at(&dir, bytes)?;
            }
            let mut chain = ActionChain::new(&root);
            for name in self.children(&actions) {
                if name != format!("{:08}", chain.sequence) {
                    return Err(ErrorCode::IdentityConflict);
                }
                let action_dir = format!("{actions}/{name}");
                let output = format!("{action_dir}/output");
                if !self.directory(&action_dir) || !self.directory(&output) {
                    return Err(ErrorCode::IdentityConflict);
                }
                let binding = self.file(&format!("{action_dir}/binding.json"))?;
                let action =
                    wire::parse_action(binding, &root).map_err(|_| ErrorCode::WireInvalid)?;
                let child_names: Vec<_> = self.children(&action_dir).collect();
                if child_names.iter().any(|n| {
                    !["binding.json", "evidence.json", "output", "import.json"].contains(n)
                }) {
                    return Err(ErrorCode::IdentityConflict);
                }
                match action.artifact_sha256.as_deref() {
                    Some(hash) => {
                        let bytes = self.file(&format!("{action_dir}/evidence.json"))?;
                        if digest_bytes(bytes) != hash {
                            return Err(ErrorCode::IdentityConflict);
                        }
                    }
                    None if child_names.contains(&"evidence.json") => {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    None => (),
                }
                if let Some(hash) = action.stage_sha256.as_deref() {
                    let path = format!("{dir}/{}.json", &hash[7..]);
                    if digest_bytes(self.file(&path)?) != hash {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
                let dispatch_path = format!("{output}/dispatch.json");
                let dispatch = self.members.get(&dispatch_path);
                if dispatch.is_none() {
                    if self.children(&output).next().is_some()
                        || child_names.contains(&"import.json")
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    chain.observe(binding, None)?;
                    continue;
                }
                let raw = self.file(&dispatch_path)?;
                chain.observe(binding, Some(raw))?;
                let parsed = wire::BoundDispatch::parse(raw).map_err(|_| ErrorCode::WireInvalid)?;
                if let Some(hash) = action.artifact_sha256.as_deref() {
                    let evidence = self.file(&format!("{action_dir}/evidence.json"))?;
                    if digest_bytes(evidence) != hash {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    self.copy_at(&output, evidence)?;
                }
                self.copy_at(&output, root_bytes)?;
                self.copy_at(&output, binding)?;
                self.copy_at(&output, parsed.request.canonical.as_bytes())?;
                if child_names.contains(&"import.json")
                    && self.file(&format!("{action_dir}/import.json"))? != raw
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                for item in self.children(&output) {
                    if item == "dispatch.json" {
                        continue;
                    }
                    let hash = named_digest(item).ok_or(ErrorCode::IdentityConflict)?;
                    let bytes = self.file(&format!("{output}/{item}"))?;
                    wire::parse_canonical_limit(bytes, MAX_JOURNAL_MEMBER)
                        .map_err(|_| ErrorCode::WireInvalid)?;
                    if digest_bytes(bytes) != hash {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
            }
            if let Some(&row_index) = index.operations.get(generation) {
                let v = &index.rows[row_index].1;
                if v["intent"] != root.intent.value
                    || v["intent_sha256"] != root.sha256
                    || [
                        "installation_id",
                        "account_id",
                        "request_key",
                        "purpose",
                        "scope",
                        "expected_epoch",
                        "local_epoch_floor",
                        "release_manifest_sha256",
                    ]
                    .iter()
                    .any(|k| v[*k] != root.intent.value[*k])
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
        }

        for (table, value) in &index.rows {
            if *table == RECEIPT_TABLES[0] {
                let generation = value["generation"].as_str().ok_or(ErrorCode::WireInvalid)?;
                if !self.directory(&format!("{operations}/{generation}")) {
                    return Err(ErrorCode::EvidenceUnavailable);
                }
            }
            self.check_receipt(table, value, &index)?;
        }
        Ok(())
    }
}

impl Domains {
    fn check_readback_current_epoch(&self, read: &Value) -> Result<(), ErrorCode> {
        for ((table, _), raw) in &self.rows {
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if table != "owner_beta_paper_control" {
                continue;
            }
            let control: Value = serde_json::from_slice(raw).map_err(|_| ErrorCode::WireInvalid)?;
            if control["account_id"] == read["account_id"]
                && read["current_epoch"]
                    .as_i64()
                    .ok_or(ErrorCode::WireInvalid)?
                    > control["control_epoch"]
                        .as_i64()
                        .ok_or(ErrorCode::WireInvalid)?
            {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        Ok(())
    }

    fn check_committed_readback(
        &self,
        read: &wire::ExactReadback,
        dispatch: &wire::BoundDispatch,
        index: &JournalIndex<'_>,
    ) -> Result<(), ErrorCode> {
        self.check_time()
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        read.correlate_original(&dispatch.root.intent)
            .map_err(|_| ErrorCode::IdentityConflict)?;
        let value = &read.value;
        if value["status"] != "COMMITTED" || value["request_sha256"] != dispatch.request.sha256 {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut found = None;
        for (table, row) in &index.rows {
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if row["request_sha256"] == dispatch.request.sha256 {
                if found.is_some() {
                    return Err(ErrorCode::IdentityConflict);
                }
                found = Some((*table, row));
            }
        }
        let (table, row) = found.ok_or(ErrorCode::EvidenceUnavailable)?;
        self.validate_receipt(table, row, index)?;
        let event = if table == RECEIPT_TABLES[0] {
            &row["fence_event_id"]
        } else if table == RECEIPT_TABLES[1] {
            &row["observation_id"]
        } else {
            &row["id"]
        };
        let result = wire::parse_canonical_limit(
            row["result_canonical"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?
                .as_bytes(),
            wire::MAX_EXACT_BYTES,
        )
        .map_err(|_| ErrorCode::WireInvalid)?;
        if value["event_id"] != *event
            || value["account_id"] != result["account_id"]
            || value["event_epoch"] != result["control_epoch"]
            || value["result_canonical"] != row["result_canonical"]
            || value["result_sha256"] != row["result_sha256"]
        {
            return Err(ErrorCode::IdentityConflict);
        }
        self.check_readback_current_epoch(value)
    }

    fn check_receipt(
        &self,
        table: &str,
        row: &Value,
        index: &JournalIndex<'_>,
    ) -> Result<(), ErrorCode> {
        self.validate_receipt(table, row, index)?;
        // All-null historical receipts keep their adopted descriptive semantics.
        if row["result_canonical"].is_null() {
            return Ok(());
        }
        let qhash = row["request_sha256"]
            .as_str()
            .ok_or(ErrorCode::WireInvalid)?;
        let rhash = row["result_sha256"]
            .as_str()
            .ok_or(ErrorCode::WireInvalid)?;
        let result = row["result_canonical"]
            .as_str()
            .ok_or(ErrorCode::WireInvalid)?;
        if index.objects.get(rhash).copied() == Some(result.as_bytes())
            || index.enclosed_results.get(qhash).map(String::as_str) == Some(result)
        {
            Ok(())
        } else {
            Err(ErrorCode::EvidenceUnavailable)
        }
    }

    // Validate SQL, exact Q and dispatch independently of where result evidence
    // was retained. Envelope admission invokes this without recursive credit.
    fn validate_receipt(
        &self,
        table: &str,
        row: &Value,
        index: &JournalIndex<'_>,
    ) -> Result<(), ErrorCode> {
        let keys = [
            "request_canonical",
            "request_sha256",
            "result_canonical",
            "result_sha256",
        ];
        if keys.iter().all(|k| row.get(*k).is_some_and(Value::is_null)) {
            return Ok(());
        }
        let get = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .ok_or(ErrorCode::WireInvalid)
        };
        let q = get(keys[0])?;
        let qhash = get(keys[1])?;
        let r = get(keys[2])?;
        let rhash = get(keys[3])?;
        if !q.is_ascii()
            || !r.is_ascii()
            || digest_bytes(q.as_bytes()) != qhash
            || digest_bytes(r.as_bytes()) != rhash
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let result = wire::parse_canonical_limit(r.as_bytes(), wire::MAX_EXACT_BYTES)
            .map_err(|_| ErrorCode::WireInvalid)?;
        if index.objects.get(qhash).copied() != Some(q.as_bytes()) {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let raw = index
            .requests
            .get(qhash)
            .copied()
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        let dispatch = wire::BoundDispatch::parse(raw).map_err(|_| ErrorCode::WireInvalid)?;
        if dispatch.request.canonical != q {
            return Err(ErrorCode::IdentityConflict);
        }
        let req = &dispatch.request.value;
        let args = &req["args"];
        let data = &result["data"];
        let kind = req["kind"].as_str().ok_or(ErrorCode::WireInvalid)?;
        let expected_action = match kind {
            "BEGIN" => "BEGIN_OPERATION",
            "RECONCILE" => "RECONCILE_OPERATION",
            other => other,
        };
        if !fields(
            &result,
            &[
                "schema",
                "action",
                "outcome",
                "control_epoch",
                "account_id",
                "data",
            ],
        ) || result["schema"] != "owner-beta-paper-action-v1"
            || result["outcome"] != "OK"
            || result["action"] != expected_action
            || !uuid(&result["account_id"])
            || !data.is_object()
            || args["request_key"] != row["request_key"]
            || (!req["account_id"].is_null() && req["account_id"] != result["account_id"])
            || (row.get("generation").is_some()
                && row["generation"] != dispatch.root.intent.generation.to_string())
            || (row.get("account_id").is_some() && row["account_id"] != result["account_id"])
            || (table == RECEIPT_TABLES[0] && kind != "BEGIN")
            || (table == RECEIPT_TABLES[1] && kind != "RECONCILE")
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let expected_epoch = match kind {
            "BEGIN" => {
                let epoch = dispatch
                    .root
                    .fence_epoch()
                    .map_err(|_| ErrorCode::IdentityConflict)?;
                if !fields(
                    data,
                    &[
                        "generation",
                        "operation_event_id",
                        "intent_sha256",
                        "fenced_epoch",
                    ],
                ) || data["generation"] != dispatch.root.intent.generation.to_string()
                    || data["intent_sha256"] != dispatch.root.sha256
                    || data["fenced_epoch"] != epoch
                    || !uuid(&data["operation_event_id"])
                    || (table == RECEIPT_TABLES[0]
                        && (data["operation_event_id"] != row["fence_event_id"]
                            || row["fence_epoch"] != epoch))
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                epoch
            }
            "RECONCILE" => {
                let summary = &data["summary"];
                let bytes = crate::owner_beta::paper::source_evidence::canonical(summary)
                    .map_err(|_| ErrorCode::WireInvalid)?;
                if !fields(data, &["observation_id", "summary", "summary_sha256"])
                    || !uuid(&data["observation_id"])
                    || wire::validate_summary(summary).is_err()
                    || summary["installation_id"]
                        != dispatch.root.intent.installation_id.to_string()
                    || summary["observation_id"] != data["observation_id"]
                    || summary["account_id"] != result["account_id"]
                    || summary["generation"] != args["generation"]
                    || summary["inventory_sha256"] != args["inventory_sha256"]
                    || summary["control_epoch"] != args["expected_epoch"]
                    || data["summary_sha256"] != digest_bytes(&bytes)
                    || (table == RECEIPT_TABLES[1]
                        && (row["observation_id"] != data["observation_id"]
                            || row["summary"] != *summary
                            || row["summary_sha256"] != data["summary_sha256"]))
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                args["expected_epoch"]
                    .as_i64()
                    .ok_or(ErrorCode::WireInvalid)?
            }
            "CONFIGURE" => {
                if !fields(
                    data,
                    &[
                        "account_id",
                        "event_id",
                        "state",
                        "initial_cash",
                        "currency",
                    ],
                ) || data["account_id"] != result["account_id"]
                    || data["state"] != "OFF"
                    || data["initial_cash"] != "10000000.0000"
                    || data["currency"] != "KRW"
                    || !uuid(&data["event_id"])
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                0
            }
            "TRANSITION" => {
                let transition = args["transition"].as_str().ok_or(ErrorCode::WireInvalid)?;
                let raw = args["evidence_canonical"]
                    .as_str()
                    .ok_or(ErrorCode::WireInvalid)?;
                let evidence = wire::parse_canonical_limit(raw.as_bytes(), wire::MAX_EXACT_BYTES)
                    .map_err(|_| ErrorCode::WireInvalid)?;
                let recovery = transition == "RECOVERY"
                    && evidence["schema"] == "owner-beta-paper-recovery-v2";
                if (recovery
                    && (!fields(
                        data,
                        &["event_id", "state", "pending_target_id", "grant_id"],
                    ) || data["pending_target_id"] != evidence["pending_target_id"]
                        || !uuid(&data["grant_id"])))
                    || (!recovery && !fields(data, &["event_id", "state"]))
                    || !uuid(&data["event_id"])
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                let state = data["state"].as_str().ok_or(ErrorCode::WireInvalid)?;
                let expected_state = match transition {
                    "ENABLE" => "ENABLED",
                    "DRAIN" => "DRAINING",
                    "OFF" => "OFF",
                    "RESTORED_OFF" => "RESTORED_OFF",
                    "RECOVERY" => state,
                    _ => return Err(ErrorCode::IdentityConflict),
                };
                if state != expected_state
                    || !["OFF", "ENABLED", "DRAINING", "RESTORED_OFF"].contains(&state)
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                args["expected_epoch"]
                    .as_i64()
                    .and_then(|n| n.checked_add(i64::from(transition != "RECOVERY")))
                    .ok_or(ErrorCode::WireInvalid)?
            }
            _ => return Err(ErrorCode::IdentityConflict),
        };
        if result["control_epoch"] != expected_epoch
            || (table == RECEIPT_TABLES[2]
                && (row["control_epoch"] != expected_epoch
                    || (matches!(kind, "CONFIGURE" | "TRANSITION")
                        && data["event_id"] != row["id"])))
        {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
}

const RELEASE_SERVICES: [&str; 13] = [
    "db-role-bootstrap",
    "db-migrate",
    "api-server",
    "web",
    "research-worker",
    "recommendation-runner",
    "candidate-runner",
    "owner-beta-runner",
    "owner-equity-v2-runner",
    "nt-backtest-worker-1",
    "nt-backtest-worker-2",
    "paper-scheduler",
    "owner-beta-simulation-runner",
];
const RELEASE_PRODUCERS: [(&str, &str); 3] = [
    ("api-server", "api_binary_sha256"),
    ("paper-runner", "worker_binary_sha256"),
    ("owner-beta-paper-ops-native", "native_binary_sha256"),
];
const PROCESS_FIELDS: &[&str] = &[
    "schema",
    "owner_user_id",
    "account_id",
    "strategy_config_id",
    "consent_event_id",
    "consent_version",
    "control_epoch",
    "daily_cohort_sha256",
    "financial_contract_sha256",
    "code_commit",
    "release_manifest_sha256",
    "api_binary_sha256",
    "worker_binary_sha256",
];
fn process_shape(v: &Value) -> bool {
    fields(v, PROCESS_FIELDS)
        && v["schema"] == "owner-beta-paper-process-v1"
        && v["consent_version"] == "owner-beta-paper-v1-draft-1"
        && [
            "owner_user_id",
            "account_id",
            "strategy_config_id",
            "consent_event_id",
        ]
        .iter()
        .all(|k| uuid(&v[*k]))
        && [
            "daily_cohort_sha256",
            "financial_contract_sha256",
            "release_manifest_sha256",
            "api_binary_sha256",
            "worker_binary_sha256",
        ]
        .iter()
        .all(|k| sha(&v[*k]))
        && v["control_epoch"].as_i64().is_some_and(|n| n >= 1)
        && v["code_commit"].as_str().is_some_and(|s| {
            s.len() == 40
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
impl Domains {
    pub(super) fn finish_release(&self, _archive: &Value, _audit: &Value) -> Result<(), ErrorCode> {
        let expected = [
            "paper_config",
            "paper_config/process.json",
            "paper_config/config.json",
            "paper_contract",
            "paper_contract/financial-contract.md",
            "paper_contract/audit-projection.json",
            "paper_contract/operations.schema.json",
            "paper_contract/archive.schema.json",
            "paper_contract/migration-0068.up.sql",
            "paper_contract/audit.sql",
            "paper_manifest",
            "paper_manifest/.lagrange-release-manifest",
            "paper_reader",
            "paper_reader/capability.json",
            "paper_reader/api-image.elf",
            "paper_reader/worker-image.elf",
            "paper_reader/native-image.elf",
            "paper_reader/migration-0069.up.sql",
            "paper_reader/install-receipt.json",
            "paper_reader/provenance",
            "paper_reader/provenance/api-server",
            "paper_reader/provenance/api-server/artifact.json",
            "paper_reader/provenance/api-server/COMPLETE",
            "paper_reader/provenance/paper-runner",
            "paper_reader/provenance/paper-runner/artifact.json",
            "paper_reader/provenance/paper-runner/COMPLETE",
            "paper_reader/provenance/owner-beta-paper-ops-native",
            "paper_reader/provenance/owner-beta-paper-ops-native/artifact.json",
            "paper_reader/provenance/owner-beta-paper-ops-native/COMPLETE",
        ];
        if self.release_members.len() != expected.len()
            || expected
                .iter()
                .any(|p| !self.release_members.contains_key(*p))
        {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        for path in [
            "paper_config",
            "paper_contract",
            "paper_manifest",
            "paper_reader",
            "paper_reader/provenance",
            "paper_reader/provenance/api-server",
            "paper_reader/provenance/paper-runner",
            "paper_reader/provenance/owner-beta-paper-ops-native",
        ] {
            if !self.release_members.get(path).is_some_and(|m| m.directory) {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        let cap_raw = self.release_bytes("paper_reader/capability.json")?;
        let cap = wire::parse_capability(cap_raw).map_err(|_| ErrorCode::WireInvalid)?;
        let commit = cap["code_commit"].as_str().ok_or(ErrorCode::WireInvalid)?;
        let manifest = self.release_bytes("paper_manifest/.lagrange-release-manifest")?;
        if digest_bytes(manifest) != cap["release_manifest_sha256"] {
            return Err(ErrorCode::IdentityConflict);
        }
        let text = std::str::from_utf8(manifest).map_err(|_| ErrorCode::WireInvalid)?;
        if !text.ends_with('\n') || text.ends_with("\n\n") {
            return Err(ErrorCode::WireInvalid);
        }
        let mut lines = text.lines();
        if lines.next() != Some("LAGRANGE_RELEASE_MANIFEST_V3")
            || lines.next() != Some(format!("commit|{commit}").as_str())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        for service in RELEASE_SERVICES {
            let line = lines.next().ok_or(ErrorCode::WireInvalid)?;
            let parts: Vec<_> = line.split('|').collect();
            if parts.len() != 5
                || parts[0] != "image"
                || parts[1] != service
                || parts[2] != format!("lagrange-station-{service}:{commit}")
                || !sha(&Value::String(parts[3].to_owned()))
                || parts[4] != commit
            {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        if lines.next().is_some() {
            return Err(ErrorCode::IdentityConflict);
        }
        for (path, key) in [
            (
                "paper_contract/operations.schema.json",
                "operations_schema_sha256",
            ),
            (
                "paper_contract/archive.schema.json",
                "archive_schema_sha256",
            ),
            (
                "paper_contract/migration-0068.up.sql",
                "migration_0068_sha256",
            ),
            ("paper_contract/audit.sql", "audit_sql_sha256"),
            ("paper_reader/api-image.elf", "api_binary_sha256"),
            ("paper_reader/worker-image.elf", "worker_binary_sha256"),
            ("paper_reader/native-image.elf", "native_binary_sha256"),
        ] {
            if cap[key].as_str() != Some(self.release_hash(path)?) {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        let process_raw = self.release_bytes("paper_config/process.json")?;
        if process_raw != self.release_bytes("paper_config/config.json")? {
            return Err(ErrorCode::IdentityConflict);
        }
        let process = wire::parse_canonical(process_raw).map_err(|_| ErrorCode::WireInvalid)?;
        if self.release_hash("paper_contract/financial-contract.md")?
            != "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0"
            || !process_shape(&process)
            || process["financial_contract_sha256"].as_str()
                != Some(self.release_hash("paper_contract/financial-contract.md")?)
            || [
                "code_commit",
                "release_manifest_sha256",
                "api_binary_sha256",
                "worker_binary_sha256",
            ]
            .iter()
            .any(|k| process[*k] != cap[*k])
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut digest_map = serde_json::Map::new();
        for (name, key) in RELEASE_PRODUCERS {
            let artifact =
                self.release_bytes(&format!("paper_reader/provenance/{name}/artifact.json"))?;
            let complete =
                self.release_bytes(&format!("paper_reader/provenance/{name}/COMPLETE"))?;
            super::super::validate_producer_receipt(
                artifact,
                complete,
                name,
                commit,
                cap[key].as_str().ok_or(ErrorCode::WireInvalid)?,
            )?;
            digest_map.insert(name.to_owned(), serde_json::json!({"artifact.json":digest_bytes(artifact),"COMPLETE":digest_bytes(complete)}));
        }
        let digest_map =
            crate::owner_beta::paper::source_evidence::canonical(&Value::Object(digest_map))
                .map_err(|_| ErrorCode::WireInvalid)?;
        let receipt =
            wire::parse_canonical(self.release_bytes("paper_reader/install-receipt.json")?)
                .map_err(|_| ErrorCode::WireInvalid)?;
        let expected_receipt = serde_json::json!({"schema":"owner-beta-paper-install-receipt-v1", "code_commit":commit, "release_manifest_sha256":cap["release_manifest_sha256"], "capability_sha256":digest_bytes(cap_raw), "audit_projection_sha256":self.release_hash("paper_contract/audit-projection.json")?, "process_sha256":digest_bytes(process_raw), "producer_receipts_sha256":digest_bytes(&digest_map)});
        if receipt != expected_receipt {
            return Err(ErrorCode::IdentityConflict);
        }
        self.historical_paper_originals(&process)
    }
}

impl Domains {
    fn historical_paper_originals(&self, _installed_process: &Value) -> Result<(), ErrorCode> {
        let mut budget = JournalIndex::new();
        let mut controls = BTreeMap::<String, Value>::new();
        let mut events = BTreeMap::<String, Value>::new();
        for ((table, _), raw) in &self.rows {
            if table != "owner_beta_paper_control" && table != "owner_beta_paper_events" {
                continue;
            }
            budget.charge(
                self,
                raw.len()
                    .checked_mul(8)
                    .ok_or(ErrorCode::EvidenceUnavailable)?,
            )?;
            let v: Value = serde_json::from_slice(raw).map_err(|_| ErrorCode::WireInvalid)?;
            let (map, key) = if table == "owner_beta_paper_control" {
                (&mut controls, "account_id")
            } else {
                (&mut events, "id")
            };
            let id = v[key].as_str().ok_or(ErrorCode::WireInvalid)?;
            if !uuid(&v[key]) {
                return Err(ErrorCode::WireInvalid);
            }
            budget.charge(self, id.len())?;
            if map.insert(id.to_owned(), v).is_some() {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        let contract_digest = self.release_hash("paper_contract/financial-contract.md")?;
        let cost = digest_bytes(b"KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000");
        for control in controls.values() {
            let contract = &control["contract_json"];
            if !fields(
                contract,
                &[
                    "contract_id",
                    "contract_sha256",
                    "decision_sha256",
                    "cost_profile_id",
                    "cost_profile_version",
                ],
            ) || contract["contract_id"] != "owner-beta-paper-v1-draft-1"
                || contract["contract_sha256"] != contract_digest
                || contract["decision_sha256"]
                    != "sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394"
                || contract["cost_profile_id"] != "KRX_ETF_DEFAULT"
                || contract["cost_profile_version"] != 1
                || control["contract_id"] != contract["contract_id"]
                || control["contract_sha256"] != contract["contract_sha256"]
                || control["cost_profile_id"] != contract["cost_profile_id"]
                || control["cost_profile_version"] != contract["cost_profile_version"]
                || control["cost_profile_sha256"] != cost
            {
                return Err(ErrorCode::IdentityConflict);
            }
            let configured = events
                .values()
                .filter(|e| e["kind"] == "CONFIGURED" && e["account_id"] == control["account_id"])
                .count();
            if configured != 1 {
                return Err(ErrorCode::EvidenceUnavailable);
            }
            if !control["consent_event_id"].is_null() {
                let consent = events
                    .get(
                        control["consent_event_id"]
                            .as_str()
                            .ok_or(ErrorCode::WireInvalid)?,
                    )
                    .ok_or(ErrorCode::EvidenceUnavailable)?;
                if consent["kind"] != "CONSENT_GRANTED"
                    || consent["account_id"] != control["account_id"]
                    || consent["owner_user_id"] != control["owner_user_id"]
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
            if !control["process_binding"].is_null()
                || !control["process_binding_sha256"].is_null()
                || !control["activation_event_id"].is_null()
            {
                let event = events
                    .get(
                        control["activation_event_id"]
                            .as_str()
                            .ok_or(ErrorCode::EvidenceUnavailable)?,
                    )
                    .ok_or(ErrorCode::EvidenceUnavailable)?;
                if event["kind"] != "ENABLED"
                    || event["account_id"] != control["account_id"]
                    || event["payload"]["process_binding"] != control["process_binding"]
                    || event["payload"]["process_binding_sha256"]
                        != control["process_binding_sha256"]
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
        }
        for event in events.values() {
            let kind = event["kind"].as_str().ok_or(ErrorCode::WireInvalid)?;
            if !matches!(
                kind,
                "CONFIGURED" | "CONSENT_GRANTED" | "CONSENT_REVOKED" | "ENABLED"
            ) {
                continue;
            }
            let control = controls
                .get(event["account_id"].as_str().ok_or(ErrorCode::WireInvalid)?)
                .ok_or(ErrorCode::EvidenceUnavailable)?;
            if event["owner_user_id"] != control["owner_user_id"] {
                return Err(ErrorCode::IdentityConflict);
            }
            let payload = &event["payload"];
            match kind {
                "CONFIGURED" => {
                    if payload["schema"] != "owner-beta-paper-configured-v1"
                        || payload["contract"] != control["contract_json"]
                        || payload["strategy_config_id"] != control["strategy_config_id"]
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
                "CONSENT_GRANTED" | "CONSENT_REVOKED" => {
                    if payload["schema"] != "owner-beta-paper-consent-v1"
                        || payload["consent_version"] != control["contract_id"]
                        || payload["contract_sha256"] != control["contract_sha256"]
                        || payload["opt_in"] != (kind == "CONSENT_GRANTED")
                        || event["actor_user_id"] != event["owner_user_id"]
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
                "ENABLED" => {
                    let binding = &payload["process_binding"];
                    if !process_shape(binding)
                        || binding["account_id"] != event["account_id"]
                        || binding["owner_user_id"] != event["owner_user_id"]
                        || binding["strategy_config_id"] != control["strategy_config_id"]
                        || binding["financial_contract_sha256"] != control["contract_sha256"]
                        || binding["consent_version"] != control["contract_id"]
                        || binding["control_epoch"] != event["control_epoch"]
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    let canonical = crate::owner_beta::paper::source_evidence::canonical(binding)
                        .map_err(|_| ErrorCode::WireInvalid)?;
                    if payload["process_binding_sha256"] != digest_bytes(&canonical) {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    let consent = events
                        .get(
                            binding["consent_event_id"]
                                .as_str()
                                .ok_or(ErrorCode::WireInvalid)?,
                        )
                        .ok_or(ErrorCode::EvidenceUnavailable)?;
                    if consent["kind"] != "CONSENT_GRANTED"
                        || consent["account_id"] != event["account_id"]
                        || consent["payload"]["contract_sha256"]
                            != binding["financial_contract_sha256"]
                        || consent["payload"]["consent_version"] != binding["consent_version"]
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }
}

impl Domains {
    /// Descriptive lineage-to-original recommendation identities only. Result-pair
    /// ownership and every other R4 graph remain separate prerequisites.
    pub(super) fn finish_r4_lineages(&self) -> Result<(), ErrorCode> {
        let mut budget = JournalIndex::new();
        let mut runs = BTreeMap::<String, Value>::new();
        let mut lineages = Vec::<Value>::new();
        for ((table, _), raw) in &self.rows {
            if table != "owner_beta_paper_lineage" && table != "owner_beta_recommendation_runs" {
                continue;
            }
            budget.charge(
                self,
                raw.len()
                    .checked_mul(8)
                    .ok_or(ErrorCode::EvidenceUnavailable)?,
            )?;
            let row: Value = serde_json::from_slice(raw).map_err(|_| ErrorCode::WireInvalid)?;
            if table == "owner_beta_paper_lineage" {
                lineages.push(row);
            } else {
                let id = row["id"].as_str().ok_or(ErrorCode::WireInvalid)?;
                if !uuid(&row["id"]) {
                    return Err(ErrorCode::WireInvalid);
                }
                budget.charge(self, id.len())?;
                if runs.insert(id.to_owned(), row).is_some() {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
        }
        for lineage in &lineages {
            let id = lineage["recommendation_run_id"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?;
            let run = runs.get(id).ok_or(ErrorCode::EvidenceUnavailable)?;
            if run["status"] != "SUCCEEDED"
                || run["trigger_kind"] != "DAILY"
                || run["owner_user_id"] != lineage["owner_user_id"]
                || run["strategy_config_id"] != lineage["strategy_config_id"]
                || run["as_of"] != lineage["signal_date"]
                || run["daily_pins"]["admission_sha256"] != lineage["signal_admission_sha256"]
                || run["factor_snapshot_sha256"] != lineage["factor_snapshot_sha256"]
                || run["target_snapshot_sha256"] != lineage["target_snapshot_sha256"]
                || !sha(&lineage["signal_admission_sha256"])
                || !sha(&lineage["factor_snapshot_sha256"])
                || !sha(&lineage["target_snapshot_sha256"])
            {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        Ok(())
    }
}

impl Domains {
    // Archive-row adapter of publish::validate_raw_job_run_binding. That helper
    // and its SQL row type are private to simulation_execution. Reuse the same
    // public version/pins/default strategy definitions without creating a valid
    // SimulationJobInput for a payload the strict parser has rejected.
    fn invalid_simulation_raw_binding(&self, run: &Value, job: &Value) -> Result<(), ErrorCode> {
        use crate::owner_beta::{
            OwnerBetaStrategySnapshot,
            simulation::{
                APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
                APPROVED_BARS_SHA256, APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_END_DATE,
                APPROVED_REGISTRY_SHA256, APPROVED_STAGE5_MANIFEST_SHA256, APPROVED_START_DATE,
                v2::V2Strategy,
            },
            simulation_execution::{
                OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, SimulationContractVersion,
            },
        };
        let payload = &job["payload_json"];
        let version = [SimulationContractVersion::V1, SimulationContractVersion::V2]
            .into_iter()
            .find(|v| payload["schema"] == v.queue_schema())
            .ok_or(ErrorCode::IdentityConflict)?;
        if !fields(
            payload,
            &[
                "schema",
                "run_id",
                "strategy_config_id",
                "evaluation_start",
                "evaluation_end",
                "pins",
                "contract_document_sha256",
                "bars_sha256",
                "strategy",
            ],
        ) || !fields(
            &payload["pins"],
            &[
                "candidate_content_sha256",
                "artifact_manifest_sha256",
                "stage5_manifest_sha256",
                "action_manifest_sha256",
                "approval_registry_sha256",
            ],
        ) || !fields(
            &payload["strategy"],
            &[
                "strategy_id",
                "strategy_version",
                "config_json",
                "config_sha256",
            ],
        ) || job["job_type"] != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
            || job["max_attempts"] != 3
            || job["id"] != run["job_id"]
            || job["owner_user_id"] != run["owner_user_id"]
            || !uuid(&run["id"])
            || !uuid(&run["owner_user_id"])
            || !uuid(&run["strategy_config_id"])
            || job["idempotency_key"]
                .as_str()
                .is_none_or(|key| !key.starts_with(version.idempotency_prefix()))
            || run["input_kind"] != "owner_beta_historical_price_only_v1"
            || run["capability"] != "PRICE_RETURN_ONLY"
            || run["audience"] != "OWNER_ONLY"
            || run["vendor_snapshot"] != true
            || run["strict_pit"] != false
            || run["action_count"] != 0
            || run["session_count"]
                .as_i64()
                .is_none_or(|n| !(202..=2452).contains(&n))
        {
            return Err(ErrorCode::IdentityConflict);
        }
        for (input, column) in [
            ("run_id", "id"),
            ("strategy_config_id", "strategy_config_id"),
            ("evaluation_start", "evaluation_start"),
            ("evaluation_end", "evaluation_end"),
            ("contract_document_sha256", "contract_document_sha256"),
            ("bars_sha256", "bars_sha256"),
        ] {
            if payload[input] != run[column] {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        for (field, expected) in [
            (
                "candidate_content_sha256",
                APPROVED_CANDIDATE_CONTENT_SHA256,
            ),
            (
                "artifact_manifest_sha256",
                APPROVED_ARTIFACT_MANIFEST_SHA256,
            ),
            ("stage5_manifest_sha256", APPROVED_STAGE5_MANIFEST_SHA256),
            ("action_manifest_sha256", APPROVED_ACTION_MANIFEST_SHA256),
            ("approval_registry_sha256", APPROVED_REGISTRY_SHA256),
        ] {
            if run[field] != expected || payload["pins"][field] != run[field] {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        if run["contract_document_sha256"] != version.contract_sha256()
            || run["bars_sha256"] != APPROVED_BARS_SHA256
        {
            return Err(ErrorCode::IdentityConflict);
        }
        for (input, column) in [
            ("strategy_id", "strategy_id"),
            ("strategy_version", "strategy_version"),
            ("config_json", "strategy_config_json"),
            ("config_sha256", "strategy_config_sha256"),
        ] {
            if payload["strategy"][input] != run[column] {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        let date = |s: &str| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| ErrorCode::WireInvalid)
        };
        let start = date(
            run["evaluation_start"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?,
        )?;
        let end = date(
            run["evaluation_end"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?,
        )?;
        if start < date(APPROVED_START_DATE)?
            || end > date(APPROVED_END_DATE)?
            || end < start
            || run["evaluation_start"] != start.to_string()
            || run["evaluation_end"] != end.to_string()
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let matches_strategy = |id: &str, config: Value| -> Result<bool, ErrorCode> {
            let snapshot =
                OwnerBetaStrategySnapshot::from_resolved_config(&crate::resolver::ResolvedConfig {
                    strategy_id: id.to_owned(),
                    strategy_version: "1.0.0".into(),
                    config,
                })
                .map_err(|_| ErrorCode::WireInvalid)?;
            Ok(run["strategy_id"] == snapshot.strategy_id()
                && run["strategy_version"] == snapshot.strategy_version()
                && run["strategy_config_json"] == *snapshot.config_json()
                && run["strategy_config_sha256"] == snapshot.config_sha256().as_str())
        };
        let strategy_matches = match version {
            SimulationContractVersion::V1 => matches_strategy(
                "trend_following",
                serde_json::json!({
                    "benchmark_instrument":"069500.KRX", "fast_ma":50, "slow_ma":200
                }),
            )?,
            SimulationContractVersion::V2 => {
                let mut matched = false;
                for strategy in V2Strategy::ALL {
                    self.check_time()
                        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
                    matched |= matches_strategy(
                        strategy.as_str(),
                        serde_json::from_str(strategy.default_config_json())
                            .map_err(|_| ErrorCode::WireInvalid)?,
                    )?;
                }
                matched
            }
        };
        if !strategy_matches {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }

    // Called only on strict input rejection. The source snapshot describes the
    // producer's committed failure/recovery result, never permission to execute.
    fn invalid_simulation_terminal(
        &self,
        run: &Value,
        job: &Value,
        attempts: &BTreeMap<String, Value>,
    ) -> Result<(), ErrorCode> {
        use crate::owner_beta::simulation_execution::SimulationPublicationFailure;
        self.check_time()
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        self.invalid_simulation_raw_binding(run, job)?;
        if !["FAILED", "CANCELED"].iter().any(|s| run["status"] == *s)
            || run["status"] != job["status"]
            || !run["engine_json"].is_null()
            || !run["published_attempt_id"].is_null()
            || self
                .simulation_pairs
                .contains_key(run["id"].as_str().ok_or(ErrorCode::WireInvalid)?)
            || run["started_at"].as_str().is_none()
            || run["finished_at"].as_str().is_none()
            || run["updated_at"] != run["finished_at"]
            || job["finished_at"].as_str().is_none()
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let count = job["attempt_count"]
            .as_i64()
            .filter(|n| (0..=3).contains(n))
            .ok_or(ErrorCode::WireInvalid)?;
        let mut latest = None;
        for attempt in attempts.values().filter(|a| a["job_id"] == job["id"]) {
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let number = attempt["attempt_no"]
                .as_i64()
                .filter(|n| *n > 0)
                .ok_or(ErrorCode::WireInvalid)?;
            if number > count {
                return Err(ErrorCode::IdentityConflict);
            }
            if number == count {
                if latest.is_some() {
                    return Err(ErrorCode::IdentityConflict);
                }
                latest = Some(attempt);
            }
        }
        if job["status"] == "CANCELED" {
            // recover_canceled also handles never-claimed queued cancellation
            // and already terminal attempts. It preserves their earlier error,
            // time and worker facts; do not impose the direct-settlement clock.
            if run["error_code"] != "CANCELED"
                || job["locked_at"].is_null() != job["locked_by"].is_null()
                || (!job["locked_by"].is_null()
                    && job["locked_by"]
                        .as_str()
                        .is_none_or(|s| s.trim().is_empty()))
            {
                return Err(ErrorCode::IdentityConflict);
            }
            match latest {
                None if count == 0 && job["locked_at"].is_null() => {}
                Some(a)
                    if ["FAILED", "ORPHANED"].iter().any(|s| a["outcome"] == *s)
                        && a["finished_at"].as_str().is_some() => {}
                _ => return Err(ErrorCode::IdentityConflict),
            }
        } else {
            let attempt = latest.ok_or(ErrorCode::EvidenceUnavailable)?;
            if !job["locked_by"].is_null()
                || !job["locked_at"].is_null()
                || attempt["finished_at"].as_str().is_none()
            {
                return Err(ErrorCode::IdentityConflict);
            }
            if job["error_code"] == SimulationPublicationFailure::InputInvalid.code() {
                // Direct publisher failure and expired-invalid recovery write
                // job, attempt and run in the same transaction. Recovery changes
                // a RUNNING attempt to ORPHANED, preserving its null errors.
                if run["error_code"] != job["error_code"]
                    || job["error_message"] != digest_bytes(b"owner-beta simulation input rejected")
                    || attempt["claimed_by"]
                        .as_str()
                        .is_none_or(|s| s.trim().is_empty())
                    || job["finished_at"] != attempt["finished_at"]
                    || job["updated_at"] != job["finished_at"]
                    || run["finished_at"] != job["finished_at"]
                    || !((attempt["outcome"] == "FAILED"
                        && attempt["error_code"] == job["error_code"]
                        && attempt["error_message"] == job["error_message"])
                        || (attempt["outcome"] == "ORPHANED"
                            && attempt["error_code"].is_null()
                            && attempt["error_message"].is_null()))
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            } else if job["error_code"] == "attempts_exhausted" {
                // heal_exhausted_run mirrors the already terminal job; its
                // final attempt and job timestamps/errors are not rewritten.
                if run["error_code"] != "SIMULATION_ATTEMPTS_EXHAUSTED"
                    || !["FAILED", "ORPHANED"]
                        .iter()
                        .any(|s| attempt["outcome"] == *s)
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            } else {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        self.check_time()
            .map_err(|_| ErrorCode::EvidenceUnavailable)
    }
}

impl Domains {
    pub(super) fn finish_r4_pairs(&self) -> Result<(), ErrorCode> {
        use crate::owner_beta::{
            simulation::EngineIdentity,
            simulation_execution::{
                OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, VersionedSimulationJobInput,
                check_stored_versioned_simulation_pair,
            },
        };
        let mut budget = JournalIndex::new();
        let mut runs = BTreeMap::<String, Value>::new();
        let mut jobs = BTreeMap::<String, Value>::new();
        let mut attempts = BTreeMap::<String, Value>::new();
        for ((table, _), raw) in &self.rows {
            let destination = match table.as_str() {
                "owner_beta_simulation_runs" => &mut runs,
                "jobs" => &mut jobs,
                "job_attempts" => &mut attempts,
                _ => continue,
            };
            budget.charge(
                self,
                raw.len()
                    .checked_mul(8)
                    .ok_or(ErrorCode::EvidenceUnavailable)?,
            )?;
            let row: Value = serde_json::from_slice(raw).map_err(|_| ErrorCode::WireInvalid)?;
            let id = row["id"].as_str().ok_or(ErrorCode::WireInvalid)?;
            if !uuid(&row["id"]) {
                return Err(ErrorCode::WireInvalid);
            }
            budget.charge(self, id.len())?;
            if destination.insert(id.to_owned(), row).is_some() {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        for (id, run) in &runs {
            let job = jobs
                .get(run["job_id"].as_str().ok_or(ErrorCode::WireInvalid)?)
                .ok_or(ErrorCode::EvidenceUnavailable)?;
            if job["id"] != run["job_id"]
                || job["owner_user_id"] != run["owner_user_id"]
                || job["job_type"] != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
            {
                return Err(ErrorCode::IdentityConflict);
            }
            let payload =
                serde_json::to_vec(&job["payload_json"]).map_err(|_| ErrorCode::WireInvalid)?;
            // Bound calendar/default-snapshot parsing beside the retained rows.
            budget.charge(self, 65_536)?;
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let input = match VersionedSimulationJobInput::from_json(&payload) {
                Ok(input) => input,
                Err(_) => {
                    self.invalid_simulation_terminal(run, job, &attempts)?;
                    continue;
                }
            };
            let strategy = input.strategy();
            let pins = input.pins();
            if input.run_id().to_string() != *id
                || input.strategy_config_id().to_string() != run["strategy_config_id"]
                || input.evaluation_start().to_string() != run["evaluation_start"]
                || input.evaluation_end().to_string() != run["evaluation_end"]
                || input.required_dates().len() as u64
                    != run["session_count"]
                        .as_u64()
                        .ok_or(ErrorCode::WireInvalid)?
                || Some(strategy.strategy_id()) != run["strategy_id"].as_str()
                || Some(strategy.strategy_version()) != run["strategy_version"].as_str()
                || strategy.config_json() != &run["strategy_config_json"]
                || Some(strategy.config_sha256().as_str()) != run["strategy_config_sha256"].as_str()
                || Some(pins.candidate_content_sha256().as_str())
                    != run["candidate_content_sha256"].as_str()
                || Some(pins.artifact_manifest_sha256().as_str())
                    != run["artifact_manifest_sha256"].as_str()
                || Some(pins.stage5_manifest_sha256().as_str())
                    != run["stage5_manifest_sha256"].as_str()
                || Some(pins.action_manifest_sha256().as_str())
                    != run["action_manifest_sha256"].as_str()
                || Some(pins.approval_registry_sha256().as_str())
                    != run["approval_registry_sha256"].as_str()
                || Some(input.contract_document_sha256().as_str())
                    != run["contract_document_sha256"].as_str()
                || Some(input.bars_sha256().as_str()) != run["bars_sha256"].as_str()
                || run["input_kind"] != "owner_beta_historical_price_only_v1"
                || run["capability"] != "PRICE_RETURN_ONLY"
                || run["audience"] != "OWNER_ONLY"
                || run["vendor_snapshot"] != true
                || run["strict_pit"] != false
                || run["action_count"] != 0
                || job["idempotency_key"]
                    .as_str()
                    .and_then(|v| v.strip_prefix(input.version().idempotency_prefix()))
                    .is_none_or(str::is_empty)
            {
                return Err(ErrorCode::IdentityConflict);
            }
            let pair = self.simulation_pairs.get(id);
            if run["status"] == "SUCCEEDED" {
                let pair = pair.ok_or(ErrorCode::EvidenceUnavailable)?;
                let attempt = attempts
                    .get(
                        run["published_attempt_id"]
                            .as_str()
                            .ok_or(ErrorCode::WireInvalid)?,
                    )
                    .ok_or(ErrorCode::EvidenceUnavailable)?;
                if Some(pair.owner.as_str()) != run["owner_user_id"].as_str()
                    || job["status"] != "SUCCEEDED"
                    || attempt["job_id"] != run["job_id"]
                    || attempt["outcome"] != "SUCCEEDED"
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                let engine = &run["engine_json"];
                if !fields(
                    engine,
                    &[
                        "git_commit",
                        "binary_sha256",
                        "toolchain",
                        "target",
                        "profile",
                    ],
                ) {
                    return Err(ErrorCode::WireInvalid);
                }
                let text = |key: &str| engine[key].as_str().ok_or(ErrorCode::WireInvalid);
                let identity = EngineIdentity::new(
                    text("git_commit")?,
                    text("binary_sha256")?,
                    text("toolchain")?,
                    text("target")?,
                    text("profile")?,
                )
                .map_err(|_| ErrorCode::WireInvalid)?;
                // The parser materializes both results; reserve conservative transient space.
                let transient = pair
                    .illustrative
                    .len()
                    .checked_add(pair.zero.len())
                    .and_then(|n| n.checked_mul(8))
                    .ok_or(ErrorCode::EvidenceUnavailable)?;
                if self
                    .used_bytes
                    .checked_add(budget.extra_bytes)
                    .and_then(|n| n.checked_add(transient))
                    .is_none_or(|n| n > self.max_bytes)
                {
                    return Err(ErrorCode::EvidenceUnavailable);
                }
                check_stored_versioned_simulation_pair(
                    &pair.illustrative,
                    &pair.illustrative_sha256,
                    &pair.zero,
                    &pair.zero_sha256,
                    &input,
                    &identity,
                )
                .map_err(|_| ErrorCode::IdentityConflict)?;
            } else if pair.is_some() {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        for job in jobs
            .values()
            .filter(|row| row["job_type"] == OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
        {
            let id = job["payload_json"]["run_id"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?;
            if !runs.get(id).is_some_and(|run| run["job_id"] == job["id"]) {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        if self
            .simulation_pairs
            .keys()
            .any(|id| !runs.contains_key(id))
        {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        Ok(())
    }
}

enum RecommendationMaterial<'a> {
    Historical(&'a market_data::ApprovedHistoricalPriceOnlyArtifact),
    Daily(&'a market_data::daily_price_only::input::AdmittedDailyPriceInputV1<'a, 'a>),
}

fn numeric_equal(actual: &Value, expected: &str) -> bool {
    actual.is_number()
        && actual
            .to_string()
            .parse::<rust_decimal::Decimal>()
            .ok()
            .zip(expected.parse::<rust_decimal::Decimal>().ok())
            .is_some_and(|(a, b)| a == b)
}

impl Domains {
    fn original_row(&self, table: &str, id: &Value) -> Result<Value, SourceClosureError> {
        let id = id.as_str().ok_or(SourceClosureError::DatabaseRow)?;
        let raw = self
            .rows
            .get(&(table.to_owned(), id.to_owned()))
            .ok_or(SourceClosureError::Relationship)?;
        serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)
    }

    fn recommendation_input(
        &self,
        run: &Value,
    ) -> Result<crate::owner_beta::daily_input::OwnerBetaExecutionInput, SourceClosureError> {
        use crate::owner_beta::daily_input::{
            OWNER_BETA_DAILY_TRIGGER_VERSION, OwnerBetaExecutionInput,
            decode_owner_beta_execution_input,
        };
        let job = self.original_row("jobs", &run["job_id"])?;
        let config = self.original_row("user_strategy_configs", &run["strategy_config_id"])?;
        let payload =
            serde_json::to_vec(&job["payload_json"]).map_err(|_| SourceClosureError::Json)?;
        let input = decode_owner_beta_execution_input(&payload)
            .map_err(|_| SourceClosureError::RecommendationInput)?;
        let strategy = input.strategy_snapshot();
        if job["job_type"] != crate::owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
            || job["owner_user_id"] != run["owner_user_id"]
            || config["owner_user_id"] != run["owner_user_id"]
            || input.run_id().to_string() != run["id"]
            || input.strategy_config_id().to_string() != run["strategy_config_id"]
            || input.as_of().to_string() != run["as_of"]
            || Some(strategy.strategy_id()) != run["strategy_id"].as_str()
            || Some(strategy.strategy_version()) != run["strategy_version"].as_str()
            || strategy.config_json() != &run["strategy_config_json"]
            || Some(strategy.config_sha256().as_str()) != run["strategy_config_sha256"].as_str()
            || run["capability"] != "PRICE_RETURN_ONLY"
            || run["audience"] != "OWNER_ONLY"
            || run["vendor_snapshot"] != true
            || run["strict_pit"] != false
            || (run["status"] == "SUCCEEDED" && job["status"] != "SUCCEEDED")
        {
            return Err(SourceClosureError::RecommendationInput);
        }
        match &input {
            OwnerBetaExecutionInput::Historical(input) => {
                let pins =
                    serde_json::to_value(input.pins()).map_err(|_| SourceClosureError::Json)?;
                if run["input_kind"] != "owner_beta_historical_price_only_v1"
                    || run["trigger_kind"] != "MANUAL"
                    || !run["daily_pins"].is_null()
                    || pins
                        .as_object()
                        .ok_or(SourceClosureError::Json)?
                        .iter()
                        .any(|(k, v)| run[k] != *v)
                {
                    return Err(SourceClosureError::RecommendationInput);
                }
            }
            OwnerBetaExecutionInput::Daily(input) => {
                let p = input.pins();
                let pins = serde_json::json!({"content_sha256":p.content_sha256(), "manifest_sha256":p.manifest_sha256(), "admission_sha256":p.admission_sha256(), "policy_sha256":p.policy_sha256()});
                if run["input_kind"] != "owner_beta_daily_etf11_price_only_v1"
                    || run["trigger_kind"] != "DAILY"
                    || run["daily_pins"] != pins
                    || run["daily_config_epoch"] != input.config_epoch()
                    || run["daily_activation_epoch"] != input.activation_epoch()
                    || run["daily_trigger_version"] != OWNER_BETA_DAILY_TRIGGER_VERSION
                    || [
                        "candidate_content_sha256",
                        "artifact_manifest_sha256",
                        "stage5_manifest_sha256",
                        "action_manifest_sha256",
                        "approval_registry_sha256",
                    ]
                    .iter()
                    .any(|k| !run[*k].is_null())
                {
                    return Err(SourceClosureError::RecommendationInput);
                }
            }
        }
        Ok(input)
    }

    fn reserve_recommendation_work(&self, bars: usize) -> Result<(), SourceClosureError> {
        // Conservative peak for parsed rows, factor builder/grouping, target output
        // and coverage keys. Material bars themselves remain borrowed, never copied here.
        let transient = self
            .rows
            .values()
            .try_fold(65_536usize, |n, v| n.checked_add(v.len().checked_mul(8)?))
            .and_then(|n| n.checked_add(bars.checked_mul(1024)?))
            .ok_or(SourceClosureError::Limit)?;
        if self
            .used_bytes
            .checked_add(transient)
            .is_none_or(|n| n > self.max_bytes)
        {
            return Err(SourceClosureError::Limit);
        }
        Ok(())
    }

    fn recommendation_for_recomputation(
        &self,
        run: &Value,
    ) -> Result<Option<crate::owner_beta::daily_input::OwnerBetaExecutionInput>, SourceClosureError>
    {
        self.check_time()?;
        let job = self.original_row("jobs", &run["job_id"])?;
        if self.malformed_historical_job_settled(&job)? {
            // This is the actual optional mirror, selected by job/owner, not a
            // fabricated decoded payload. It never enters computed coverage.
            if self.original_row("owner_beta_recommendation_runs", &run["id"])? != *run
                || run["owner_user_id"] != job["owner_user_id"]
            {
                return Err(SourceClosureError::RecommendationInput);
            }
            return Ok(None);
        }
        self.recommendation_input(run).map(Some)
    }

    fn recompute(
        &mut self,
        material: RecommendationMaterial<'_>,
    ) -> Result<(), SourceClosureError> {
        use crate::owner_beta::{
            compute::compute_owner_beta_price_recommendation_with_approved,
            daily_compute::compute_owner_beta_daily_recommendation,
            daily_input::OwnerBetaExecutionInput,
        };
        let bars = match &material {
            RecommendationMaterial::Historical(a) => a.bars().len(),
            RecommendationMaterial::Daily(a) => a.bars().len(),
        };
        if !self.rows.keys().any(|(table, key)| {
            table == "owner_beta_recommendation_runs" && !self.recommendations.contains(key)
        }) {
            return self.check_time();
        }
        self.reserve_recommendation_work(bars)?;
        let mut covered = Vec::new();
        for ((table, key), raw) in &self.rows {
            if table != "owner_beta_recommendation_runs" {
                continue;
            }
            self.check_time()?;
            let run: Value =
                serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
            let Some(input) = self.recommendation_for_recomputation(&run)? else {
                continue;
            };
            if self.recommendations.contains(key) {
                continue;
            }
            match (&material, &input) {
                (
                    RecommendationMaterial::Historical(artifact),
                    OwnerBetaExecutionInput::Historical(input),
                ) => {
                    if input.validate_approved_artifact(artifact).is_err() {
                        continue;
                    }
                    if run["status"] == "SUCCEEDED" {
                        let computed =
                            compute_owner_beta_price_recommendation_with_approved(artifact, input)
                                .map_err(|_| SourceClosureError::RecommendationCompute)?;
                        self.check_time()?;
                        let target = computed.target_snapshot();
                        self.compare_recommendation_output(
                            &run,
                            computed.factor_snapshot().hash.as_str(),
                            target.target_snapshot_sha256().as_str(),
                            &target.cash_weight(),
                            target.items(),
                            false,
                        )?;
                    } else {
                        self.require_unpublished(&run)?;
                    }
                }
                (
                    RecommendationMaterial::Daily(admitted),
                    OwnerBetaExecutionInput::Daily(input),
                ) => {
                    if input.validate_admitted(admitted).is_err() {
                        continue;
                    }
                    if run["status"] == "SUCCEEDED" {
                        let computed = compute_owner_beta_daily_recommendation(admitted, input)
                            .map_err(|_| SourceClosureError::RecommendationCompute)?;
                        self.check_time()?;
                        let target = computed.target_snapshot();
                        self.compare_recommendation_output(
                            &run,
                            computed.factor_snapshot().hash.as_str(),
                            target.target_snapshot_sha256().as_str(),
                            &target.cash_weight(),
                            target.items(),
                            true,
                        )?;
                    } else {
                        self.require_unpublished(&run)?;
                    }
                }
                _ => continue,
            }
            self.check_time()?;
            covered.push(key.clone());
        }
        for key in covered {
            self.charge(key.len())?;
            self.recommendations.insert(key);
        }
        Ok(())
    }

    fn require_unpublished(&self, run: &Value) -> Result<(), SourceClosureError> {
        if !["PENDING", "RUNNING", "FAILED", "CANCELED"]
            .iter()
            .any(|s| run["status"] == *s)
            || [
                "factor_snapshot_sha256",
                "target_snapshot_sha256",
                "cash_weight",
            ]
            .iter()
            .any(|k| !run[*k].is_null())
        {
            return Err(SourceClosureError::RecommendationOutput);
        }
        for ((table, _), raw) in &self.rows {
            if table == "owner_beta_recommendation_items" {
                self.check_time()?;
                let row: Value =
                    serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
                if row["recommendation_run_id"] == run["id"] {
                    return Err(SourceClosureError::RecommendationOutput);
                }
            }
        }
        Ok(())
    }

    fn compare_recommendation_output(
        &self,
        run: &Value,
        factor: &str,
        target: &str,
        cash: &str,
        items: &[crate::owner_beta::OwnerBetaTargetItem],
        daily: bool,
    ) -> Result<(), SourceClosureError> {
        if run["factor_snapshot_sha256"] != factor
            || run["target_snapshot_sha256"] != target
            || !numeric_equal(&run["cash_weight"], cash)
        {
            return Err(SourceClosureError::RecommendationOutput);
        }
        let mut seen = BTreeSet::new();
        let mut succeeded = false;
        for ((table, _), raw) in &self.rows {
            if ![
                "owner_beta_recommendation_items",
                "job_attempts",
                "owner_beta_paper_lineage",
            ]
            .contains(&table.as_str())
            {
                continue;
            }
            let row: Value =
                serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
            if table == "job_attempts" {
                succeeded |= row["job_id"] == run["job_id"] && row["outcome"] == "SUCCEEDED";
                continue;
            }
            if row["recommendation_run_id"] != run["id"] {
                continue;
            }
            if table == "owner_beta_paper_lineage" {
                let weights: Vec<_> = items.iter().filter(|i| i.target_weight_ppm().is_some_and(|w| w > 0))
                    .map(|i| serde_json::json!({"instrument_id":i.instrument_id(), "weight":i.target_weight()})).collect();
                if !daily
                    || row["target_weights_json"] != Value::Array(weights)
                    || row["config_epoch"] != run["daily_config_epoch"]
                    || row["daily_activation_epoch"] != run["daily_activation_epoch"]
                {
                    return Err(SourceClosureError::RecommendationOutput);
                }
                continue;
            }
            let instrument = row["instrument_id"]
                .as_str()
                .ok_or(SourceClosureError::RecommendationOutput)?;
            let item = items
                .iter()
                .find(|i| i.instrument_id() == instrument)
                .ok_or(SourceClosureError::RecommendationOutput)?;
            let reasons: Vec<_> = item.reasons().iter().map(|r| r.code()).collect();
            let excluded = item.target_weight_ppm().is_none();
            let weight = if daily {
                Some(item.target_weight().unwrap_or_else(|| "0.000000".into()))
            } else {
                item.target_weight()
            };
            let exclusion = if excluded {
                item.reasons().first().map(|r| r.code())
            } else {
                None
            };
            if !seen.insert(instrument.to_owned())
                || row["owner_user_id"] != run["owner_user_id"]
                || row["rank"] != serde_json::json!(item.rank())
                || row["reason_codes"] != serde_json::json!(reasons)
                || row["factors_json"]
                    != serde_json::to_value(item.factors()).map_err(|_| SourceClosureError::Json)?
                || row["excluded"] != excluded
                || row["exclusion_reason"] != serde_json::json!(exclusion)
                || !weight
                    .as_deref()
                    .map_or(row["target_weight"].is_null(), |v| {
                        numeric_equal(&row["target_weight"], v)
                    })
            {
                return Err(SourceClosureError::RecommendationOutput);
            }
        }
        if seen.len() != items.len() || !succeeded {
            return Err(SourceClosureError::RecommendationOutput);
        }
        Ok(())
    }

    fn malformed_historical_job_settled(&self, job: &Value) -> Result<bool, SourceClosureError> {
        // runner::run_next historical decode failures settle the queue first;
        // publish::settle_malformed_claim_in may then mirror that outcome into
        // the existing run selected only by job_id/owner. Never decode a new
        // input for this rejection, or count it as successfully recomputed.
        self.check_time()?;
        let failed = job["status"] == "FAILED" && job["error_code"] == "OWNER_BETA_INPUT_INVALID";
        let canceled = job["status"] == "CANCELED";
        if job["job_type"] != crate::owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
            || !(failed || canceled)
            // Daily uses the same queue job type and can legitimately fail or
            // be canceled. Use the runner's complete schema dispatch so such
            // a valid Daily input is never mistaken for malformed historical.
            || serde_json::from_value::<crate::owner_beta::daily_input::OwnerBetaExecutionInput>(
                job["payload_json"].clone(),
            )
            .is_ok()
        {
            return Ok(false);
        }
        let attempt_no = job["attempt_count"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or(SourceClosureError::RecommendationInput)?;
        if !uuid(&job["id"])
            || !uuid(&job["owner_user_id"])
            || job["finished_at"].as_str().is_none()
            || (failed
                && (job["error_message"] != digest_bytes(b"owner-beta input rejected")
                    || !job["locked_by"].is_null()
                    || !job["locked_at"].is_null()))
            || (canceled
                && (job["locked_by"].as_str().is_none_or(str::is_empty)
                    || job["locked_at"].as_str().is_none()))
        {
            return Err(SourceClosureError::RecommendationInput);
        }
        let mut settled = None;
        let mut mirror = None;
        for ((table, key), raw) in &self.rows {
            self.check_time()?;
            if !["owner_beta_recommendation_runs", "job_attempts"].contains(&table.as_str()) {
                continue;
            }
            let row: Value =
                serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
            if row["job_id"] != job["id"] {
                continue;
            }
            if table == "owner_beta_recommendation_runs" {
                if mirror.is_some() || row["id"].as_str() != Some(key.as_str()) {
                    return Err(SourceClosureError::RecommendationInput);
                }
                mirror = Some(row);
            } else if row["attempt_no"].as_i64() == Some(attempt_no) {
                // Cancellation commits the job first, then settles the still
                // owned attempt. It preserves locks, errors and the earlier
                // job time. The optional run uses the attempt transaction time.
                if settled.is_some()
                    || !uuid(&row["id"])
                    || row["id"].as_str() != Some(key.as_str())
                    || row["claimed_by"].as_str().is_none_or(str::is_empty)
                    || row["outcome"] != "FAILED"
                    || (failed
                        && (row["error_code"] != job["error_code"]
                            || row["error_message"] != job["error_message"]
                            || row["finished_at"] != job["finished_at"]))
                    || (canceled
                        && (row["error_code"] != "canceled"
                            || row["error_message"] != digest_bytes(b"job canceled by request")
                            || row["claimed_by"] != job["locked_by"]
                            || row["finished_at"].as_str().is_none()))
                {
                    return Err(SourceClosureError::RecommendationInput);
                }
                settled = Some(row);
            }
        }
        let attempt = settled.ok_or(SourceClosureError::RecommendationInput)?;
        if let Some(run) = &mirror {
            let config = self.original_row("user_strategy_configs", &run["strategy_config_id"])?;
            if !uuid(&run["id"])
                || run["owner_user_id"] != job["owner_user_id"]
                || config["id"] != run["strategy_config_id"]
                || config["owner_user_id"] != run["owner_user_id"]
                || run["status"] != job["status"]
                || run["error_code"]
                    != if failed {
                        "OWNER_BETA_INPUT_INVALID"
                    } else {
                        "CANCELED"
                    }
                || run["input_kind"] != "owner_beta_historical_price_only_v1"
                || run["capability"] != "PRICE_RETURN_ONLY"
                || run["audience"] != "OWNER_ONLY"
                || run["vendor_snapshot"] != true
                || run["strict_pit"] != false
                || run["trigger_kind"] != "MANUAL"
                || !run["daily_pins"].is_null()
                || run["started_at"].as_str().is_none()
                || run["finished_at"] != attempt["finished_at"]
                || run["updated_at"] != attempt["finished_at"]
            {
                return Err(SourceClosureError::RecommendationInput);
            }
            self.require_unpublished(run)?;
        }
        // A rejected historical claim cannot have a Daily reservation/claim,
        // a published Paper source run or a lineage. These are actual retained
        // rows, charged to the same collector budget as every other original.
        for ((table, _), raw) in &self.rows {
            self.check_time()?;
            let field = match table.as_str() {
                "owner_beta_paper_lineage" => "recommendation_run_id",
                "owner_beta_daily_reservations"
                | "owner_beta_daily_publication_claims"
                | "owner_beta_paper_source_runs" => "run_id",
                _ => continue,
            };
            let row: Value =
                serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
            if row["job_id"] == job["id"]
                || mirror.as_ref().is_some_and(|run| row[field] == run["id"])
            {
                return Err(SourceClosureError::RecommendationOutput);
            }
        }
        self.check_time()?;
        Ok(true)
    }

    fn finish_recommendations(&self) -> Result<(), SourceClosureError> {
        self.reserve_recommendation_work(0)?;
        for ((table, key), raw) in &self.rows {
            self.check_time()?;
            if table == "owner_beta_recommendation_runs" && !self.recommendations.contains(key) {
                let run: Value =
                    serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
                if !uuid(&run["job_id"])
                    || !self.malformed_historical_job_settled(
                        &self.original_row("jobs", &run["job_id"])?,
                    )?
                {
                    return Err(SourceClosureError::RecommendationMaterialMissing);
                }
            }
            if ["owner_beta_recommendation_items", "jobs"].contains(&table.as_str()) {
                let row: Value =
                    serde_json::from_slice(raw).map_err(|_| SourceClosureError::DatabaseRow)?;
                let id = if table == "jobs" {
                    if row["job_type"]
                        != crate::owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
                    {
                        continue;
                    }
                    if self.malformed_historical_job_settled(&row)? {
                        continue;
                    }
                    &row["payload_json"]["run_id"]
                } else {
                    &row["recommendation_run_id"]
                };
                let run = self.original_row("owner_beta_recommendation_runs", id)?;
                if table == "jobs" && run["job_id"] != row["id"] {
                    return Err(SourceClosureError::RecommendationInput);
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(&self, archive: &Value, audit: &Value) -> Result<(), SourceClosureError> {
        self.check_time()?;
        self.finish_lifecycle()?;
        self.finish_journal(archive, audit)
            .map_err(|_| SourceClosureError::Journal)?;
        self.finish_release(archive, audit)
            .map_err(|_| SourceClosureError::Release)?;
        self.finish_r4_pairs()
            .map_err(|_| SourceClosureError::SimulationPair)?;
        self.finish_r4_lineages()
            .map_err(|_| SourceClosureError::Relationship)?;
        self.finish_recommendations()?;
        self.check_time()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair_fixture_rows() -> Vec<(&'static str, String, Value)> {
        use crate::owner_beta::{
            OwnerBetaStrategySnapshot, simulation::*, simulation_execution::SimulationJobInput,
        };
        let strategy = OwnerBetaStrategySnapshot::from_resolved_config(&crate::resolver::ResolvedConfig {
            strategy_id: "trend_following".into(), strategy_version: "1.0.0".into(),
            config: serde_json::json!({"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}),
        }).unwrap();
        let id = uuid::Uuid::from_u128(1).to_string();
        let owner = uuid::Uuid::from_u128(2).to_string();
        let job = uuid::Uuid::from_u128(3).to_string();
        let attempt = uuid::Uuid::from_u128(4).to_string();
        let config = uuid::Uuid::from_u128(5).to_string();
        let payload = serde_json::json!({"schema":"owner-beta-price-only-simulation-job-v1", "run_id":id,"strategy_config_id":config,
            "evaluation_start":"2026-08-27","evaluation_end":"2026-08-28",
            "pins":{"candidate_content_sha256":APPROVED_CANDIDATE_CONTENT_SHA256,"artifact_manifest_sha256":APPROVED_ARTIFACT_MANIFEST_SHA256,
            "stage5_manifest_sha256":APPROVED_STAGE5_MANIFEST_SHA256,"action_manifest_sha256":APPROVED_ACTION_MANIFEST_SHA256,"approval_registry_sha256":APPROVED_REGISTRY_SHA256},
            "contract_document_sha256":APPROVED_CONTRACT_SHA256,"bars_sha256":APPROVED_BARS_SHA256,"strategy":strategy});
        let input = SimulationJobInput::from_json(&serde_json::to_vec(&payload).unwrap()).unwrap();
        let engine = EngineIdentity::new(
            &"a".repeat(40),
            &format!("sha256:{}", "b".repeat(64)),
            "rustc-test",
            "x86_64-unknown-linux-gnu",
            "test",
        )
        .unwrap();
        let schedule = input.schedule();
        let signals: Vec<_> = schedule
            .signal_execution_pairs()
            .iter()
            .map(|(date, _)| {
                let mut weights: Vec<_> = market_data::KR_ETF_CORE_SYMBOLS
                    .iter()
                    .map(|s| {
                        (
                            format!("{s}.KRX"),
                            if *s == "069500" { 1_000_000 } else { 0 },
                        )
                    })
                    .collect();
                weights.sort();
                SignalSnapshot::new(
                    &date.to_iso(),
                    &format!("sha256:{}", "c".repeat(64)),
                    &format!("sha256:{}", "d".repeat(64)),
                    weights,
                    0,
                )
                .unwrap()
            })
            .collect();
        let prices: Vec<_> = schedule
            .evaluation_sessions()
            .iter()
            .map(|date| {
                EvaluationPriceRow::new(
                    &date.to_iso(),
                    Price4::parse("100.0000").unwrap(),
                    Price4::parse("101.0000").unwrap(),
                    1_000_000,
                )
                .unwrap()
            })
            .collect();
        let ledgers = simulate_ledgers(schedule, &signals, &prices).unwrap();
        let identity = StrategyIdentity::trend_following_v1(
            input.strategy().config_sha256().as_str(),
            "1.0.0",
            &format!("sha256:{}", "e".repeat(64)),
        )
        .unwrap();
        let make = |profile, ledger: &CostProfileLedgers| {
            SimulationResult::new(
                engine.clone(),
                identity.clone(),
                schedule.clone(),
                profile,
                signals.clone(),
                ledger.strategy().clone(),
                ledger.benchmark().clone(),
            )
            .unwrap()
            .canonical_json()
            .unwrap()
        };
        let illustrative = make(
            CostProfile::Illustrative10BpPerSideV1,
            ledgers.illustrative_10bp_per_side(),
        );
        let zero = make(
            CostProfile::ZeroCostReferenceV1,
            ledgers.zero_cost_reference(),
        );
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut run = serde_json::json!({"id":id,"owner_user_id":owner,"job_id":job,"strategy_config_id":config,"evaluation_start":payload["evaluation_start"],"evaluation_end":payload["evaluation_end"],
            "session_count":schedule.warmup_sessions().len()+schedule.evaluation_sessions().len(),"contract_document_sha256":APPROVED_CONTRACT_SHA256,"bars_sha256":APPROVED_BARS_SHA256,
            "strategy_id":strategy.strategy_id(),"strategy_version":strategy.strategy_version(),"strategy_config_json":strategy.config_json(),"strategy_config_sha256":strategy.config_sha256(),
            "input_kind":"owner_beta_historical_price_only_v1","capability":"PRICE_RETURN_ONLY","audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,"action_count":0,"status":"SUCCEEDED","published_attempt_id":attempt,"engine_json":engine});
        for (key, value) in payload["pins"].as_object().unwrap() {
            run[key] = value.clone();
        }
        vec![
            ("owner_beta_simulation_runs", id.clone(), run),
            (
                "jobs",
                job.clone(),
                serde_json::json!({"id":job,"owner_user_id":owner,"job_type":"owner_beta_price_only_simulation","status":"SUCCEEDED","payload_json":payload,"idempotency_key":"owner-beta:price-only-simulation:v1:fixture"}),
            ),
            (
                "job_attempts",
                attempt.clone(),
                serde_json::json!({"id":attempt,"job_id":job,"outcome":"SUCCEEDED"}),
            ),
            (
                "owner_beta_simulation_result_pairs",
                id.clone(),
                serde_json::json!({"run_id":id,"owner_user_id":owner,"illustrative_canonical":hex(&illustrative),"illustrative_sha256":digest_bytes(&illustrative),"zero_cost_canonical":hex(&zero),"zero_cost_sha256":digest_bytes(&zero),"created_at":"2026-11-11T00:00:00.000000Z"}),
            ),
        ]
    }
    fn add_rows(domains: &mut Domains, rows: &[(&str, String, Value)]) {
        for (table, key, row) in rows {
            domains
                .observe(ArchiveOriginalMember::Database {
                    group: "simulation",
                    table,
                    key,
                    canonical_row: &serde_json::to_vec(row).unwrap(),
                })
                .unwrap();
        }
    }
    fn invalid_simulation_rows(
        version: crate::owner_beta::simulation_execution::SimulationContractVersion,
    ) -> Vec<(&'static str, String, Value)> {
        use crate::owner_beta::{simulation::*, simulation_execution::SimulationContractVersion};
        let mut rows = pair_fixture_rows();
        rows.pop(); // A settled input rejection has no output pair.
        let sessions = market_data::range_normalize::ExpectedRangeSessions::approved_xkrx(
            domain::TradingDate::parse(APPROVED_START_DATE).unwrap(),
            domain::TradingDate::parse(APPROVED_END_DATE).unwrap(),
        )
        .unwrap()
        .sessions;
        // Same SQL-admitted early-start vector as the existing v2 execution
        // regression (indices 199/201); closed calendar decoding must reject.
        let start = sessions[199].to_iso();
        let end = sessions[201].to_iso();
        let mut payload = rows[1].2["payload_json"].clone();
        payload["schema"] = serde_json::json!(version.queue_schema());
        payload["contract_document_sha256"] = serde_json::json!(version.contract_sha256());
        payload["evaluation_start"] = serde_json::json!(start);
        payload["evaluation_end"] = serde_json::json!(end);
        if version == SimulationContractVersion::V2 {
            let strategy = v2::V2Strategy::BuyAndHold;
            let snapshot = crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
                &crate::resolver::ResolvedConfig {
                    strategy_id: strategy.as_str().into(),
                    strategy_version: "1.0.0".into(),
                    config: serde_json::from_str(strategy.default_config_json()).unwrap(),
                },
            )
            .unwrap();
            payload["strategy"] = serde_json::to_value(snapshot).unwrap();
        }
        let time = "2026-11-11T01:02:04.000005Z";
        let code =
            crate::owner_beta::simulation_execution::SimulationPublicationFailure::InputInvalid
                .code();
        let message = digest_bytes(b"owner-beta simulation input rejected");
        let run = &mut rows[0].2;
        run["evaluation_start"] = serde_json::json!(start);
        run["evaluation_end"] = serde_json::json!(end);
        run["session_count"] = serde_json::json!(202);
        run["contract_document_sha256"] = payload["contract_document_sha256"].clone();
        for (field, column) in [
            ("strategy_id", "strategy_id"),
            ("strategy_version", "strategy_version"),
            ("config_json", "strategy_config_json"),
            ("config_sha256", "strategy_config_sha256"),
        ] {
            run[column] = payload["strategy"][field].clone();
        }
        run["status"] = serde_json::json!("FAILED");
        run["error_code"] = serde_json::json!(code);
        run["engine_json"] = Value::Null;
        run["published_attempt_id"] = Value::Null;
        run["started_at"] = serde_json::json!("2026-11-11T01:02:01.000001Z");
        run["finished_at"] = serde_json::json!(time);
        run["updated_at"] = serde_json::json!(time);
        let job = &mut rows[1].2;
        job["payload_json"] = payload;
        job["idempotency_key"] =
            serde_json::json!(format!("{}invalid-calendar", version.idempotency_prefix()));
        job["max_attempts"] = serde_json::json!(3);
        job["attempt_count"] = serde_json::json!(1);
        job["status"] = serde_json::json!("FAILED");
        job["error_code"] = serde_json::json!(code);
        job["error_message"] = serde_json::json!(message);
        job["locked_by"] = Value::Null;
        job["locked_at"] = Value::Null;
        job["finished_at"] = serde_json::json!(time);
        job["updated_at"] = serde_json::json!(time);
        let attempt = &mut rows[2].2;
        attempt["attempt_no"] = serde_json::json!(1);
        attempt["outcome"] = serde_json::json!("FAILED");
        attempt["claimed_by"] = serde_json::json!("simulation-worker");
        attempt["error_code"] = serde_json::json!(code);
        attempt["error_message"] = serde_json::json!(message);
        attempt["finished_at"] = serde_json::json!(time);
        rows
    }

    #[test]
    fn invalid_simulation_calendar_preserves_publisher_and_recovery_history() {
        use crate::owner_beta::simulation_execution::{
            SimulationContractVersion, VersionedSimulationJobInput,
        };
        for version in [SimulationContractVersion::V1, SimulationContractVersion::V2] {
            let original = invalid_simulation_rows(version);
            assert!(
                VersionedSimulationJobInput::from_json(
                    &serde_json::to_vec(&original[1].2["payload_json"]).unwrap()
                )
                .is_err()
            );
            for path in 0..9 {
                let mut rows = original.clone();
                if path == 1 {
                    rows[2].2["outcome"] = serde_json::json!("ORPHANED");
                    rows[2].2["error_code"] = Value::Null;
                    rows[2].2["error_message"] = Value::Null;
                } else if (2..=6).contains(&path) {
                    rows[0].2["status"] = serde_json::json!("CANCELED");
                    rows[0].2["error_code"] = serde_json::json!("CANCELED");
                    rows[1].2["status"] = serde_json::json!("CANCELED");
                    rows[1].2["error_code"] = Value::Null;
                    rows[1].2["error_message"] = Value::Null;
                    rows[1].2["finished_at"] = serde_json::json!("2026-11-11T01:02:03.000004Z");
                    rows[1].2["updated_at"] = rows[1].2["finished_at"].clone();
                    if path <= 3 {
                        rows[1].2["locked_by"] = serde_json::json!("simulation-worker");
                        rows[1].2["locked_at"] = serde_json::json!("2026-11-11T01:02:01.000001Z");
                        rows[2].2["error_code"] = serde_json::json!("canceled");
                        rows[2].2["error_message"] = serde_json::json!(digest_bytes(
                            b"owner-beta simulation input rejected"
                        ));
                    }
                    if path == 3 || path == 6 {
                        rows[2].2["outcome"] = serde_json::json!("ORPHANED");
                        rows[2].2["error_code"] = Value::Null;
                        rows[2].2["error_message"] = Value::Null;
                    }
                    if path == 4 || path == 6 {
                        // Recovery may mirror an older already terminal attempt,
                        // preserving independent job and attempt facts.
                        rows[2].2["finished_at"] = serde_json::json!("2026-11-11T01:02:02.000003Z");
                    }
                    if path == 5 {
                        rows[1].2["attempt_count"] = serde_json::json!(0);
                        rows.pop(); // Canceled while still queued, never claimed.
                    }
                } else if path >= 7 {
                    rows[0].2["error_code"] = serde_json::json!("SIMULATION_ATTEMPTS_EXHAUSTED");
                    rows[1].2["error_code"] = serde_json::json!("attempts_exhausted");
                    rows[1].2["error_message"] =
                        serde_json::json!(digest_bytes(b"worker crash exhausted retries"));
                    rows[1].2["attempt_count"] = serde_json::json!(3);
                    rows[2].2["attempt_no"] = serde_json::json!(3);
                    rows[2].2["finished_at"] = serde_json::json!("2026-11-11T01:02:02.000003Z");
                    rows[1].2["finished_at"] = serde_json::json!("2026-11-11T01:02:03.000004Z");
                    rows[1].2["updated_at"] = rows[1].2["finished_at"].clone();
                    if path == 7 {
                        rows[2].2["outcome"] = serde_json::json!("ORPHANED");
                        rows[2].2["error_code"] = Value::Null;
                        rows[2].2["error_message"] = Value::Null;
                    }
                }
                let domains = consume_malformed_history(&rows);
                assert_eq!(domains.finish_r4_pairs(), Ok(()), "{version:?}/{path}");
                assert_eq!(
                    domains.finish(&Value::Null, &Value::Null),
                    Ok(()),
                    "{version:?}/{path}"
                );
                assert!(domains.simulation_pairs.is_empty());
                assert_eq!(rows[1].2["payload_json"], original[1].2["payload_json"]);
            }
        }
    }

    #[test]
    fn invalid_simulation_identity_settlement_and_output_mutants_refuse() {
        use crate::owner_beta::simulation_execution::SimulationContractVersion;
        let rows = invalid_simulation_rows(SimulationContractVersion::V2);
        let other = serde_json::json!(uuid::Uuid::from_u128(99));
        for (index, field, value) in [
            (0, "status", serde_json::json!("SUCCEEDED")),
            (0, "status", serde_json::json!("PENDING")),
            (0, "owner_user_id", other.clone()),
            (0, "strategy_config_id", other.clone()),
            (0, "session_count", serde_json::json!(201)),
            (0, "error_code", serde_json::json!("OTHER_ERROR")),
            (0, "engine_json", serde_json::json!({})),
            (0, "published_attempt_id", rows[2].2["id"].clone()),
            (0, "started_at", Value::Null),
            (
                0,
                "finished_at",
                serde_json::json!("2026-11-11T01:02:05.000005Z"),
            ),
            (0, "updated_at", Value::Null),
            (1, "max_attempts", serde_json::json!(4)),
            (1, "job_type", serde_json::json!("simulation")),
            (
                1,
                "idempotency_key",
                serde_json::json!("owner-beta:price-only-simulation:v1:wrong"),
            ),
            (1, "error_code", serde_json::json!("OTHER_ERROR")),
            (
                1,
                "error_message",
                serde_json::json!(digest_bytes(b"different")),
            ),
            (1, "locked_by", serde_json::json!("still-owned")),
            (1, "attempt_count", serde_json::json!(2)),
            (2, "job_id", other),
            (2, "attempt_no", serde_json::json!(2)),
            (2, "outcome", serde_json::json!("SUCCEEDED")),
            (2, "outcome", serde_json::json!("RUNNING")),
            (2, "claimed_by", Value::Null),
            (2, "error_code", serde_json::json!("OTHER_ERROR")),
            (
                2,
                "error_message",
                serde_json::json!(digest_bytes(b"different")),
            ),
            (
                2,
                "finished_at",
                serde_json::json!("2026-11-11T01:02:05.000005Z"),
            ),
        ] {
            let mut changed = rows.clone();
            changed[index].2[field] = value;
            let domains = consume_malformed_history(&changed);
            assert!(domains.finish_r4_pairs().is_err(), "{index}/{field}");
            assert_eq!(
                domains.finish(&Value::Null, &Value::Null),
                Err(SourceClosureError::SimulationPair)
            );
        }
        for change in 0..7 {
            let mut changed = rows.clone();
            match change {
                0 => {
                    changed[1].2["payload_json"]["unexpected"] = Value::Null;
                }
                1 => {
                    changed[1].2["payload_json"]["schema"] = serde_json::json!("unknown-version");
                }
                2 => {
                    changed[1].2["payload_json"]["run_id"] =
                        serde_json::json!(uuid::Uuid::from_u128(99));
                }
                3 => {
                    changed[0].2["candidate_content_sha256"] =
                        serde_json::json!(digest_bytes(b"wrong pin"));
                    changed[1].2["payload_json"]["pins"]["candidate_content_sha256"] =
                        changed[0].2["candidate_content_sha256"].clone();
                }
                4 => {
                    changed[0].2["contract_document_sha256"] =
                        serde_json::json!(SimulationContractVersion::V1.contract_sha256());
                    changed[1].2["payload_json"]["contract_document_sha256"] =
                        changed[0].2["contract_document_sha256"].clone();
                }
                5 => {
                    changed[0].2["strategy_config_sha256"] =
                        serde_json::json!(digest_bytes(b"wrong snapshot"));
                    changed[1].2["payload_json"]["strategy"]["config_sha256"] =
                        changed[0].2["strategy_config_sha256"].clone();
                }
                _ => {
                    changed[0].2["evaluation_start"] = serde_json::json!("2015-01-01");
                    changed[1].2["payload_json"]["evaluation_start"] =
                        changed[0].2["evaluation_start"].clone();
                }
            }
            assert!(
                consume_malformed_history(&changed)
                    .finish_r4_pairs()
                    .is_err()
            );
        }
        for missing in [0, 1, 2] {
            let mut changed = rows.clone();
            changed.remove(missing);
            assert!(
                consume_malformed_history(&changed)
                    .finish_r4_pairs()
                    .is_err()
            );
        }
        let mut duplicate = rows.clone();
        let mut attempt = rows[2].clone();
        attempt.1 = uuid::Uuid::from_u128(99).to_string();
        attempt.2["id"] = serde_json::json!(attempt.1);
        duplicate.push(attempt);
        assert!(
            consume_malformed_history(&duplicate)
                .finish_r4_pairs()
                .is_err()
        );
        let mut paired = rows;
        paired.push(pair_fixture_rows().pop().unwrap());
        assert!(
            consume_malformed_history(&paired)
                .finish_r4_pairs()
                .is_err()
        );
    }

    #[test]
    fn invalid_simulation_recovery_does_not_erase_final_attempt_or_claim_guards() {
        use crate::owner_beta::simulation_execution::SimulationContractVersion;
        let mut canceled = invalid_simulation_rows(SimulationContractVersion::V2);
        canceled[0].2["status"] = serde_json::json!("CANCELED");
        canceled[0].2["error_code"] = serde_json::json!("CANCELED");
        canceled[1].2["status"] = serde_json::json!("CANCELED");
        for (index, field, value) in [
            (
                0,
                "error_code",
                serde_json::json!("SIMULATION_INPUT_INVALID"),
            ),
            (1, "locked_by", serde_json::json!("worker-without-lock")),
            (
                1,
                "locked_at",
                serde_json::json!("2026-11-11T01:02:01.000001Z"),
            ),
            (1, "attempt_count", serde_json::json!(0)),
            (2, "outcome", serde_json::json!("RUNNING")),
            (2, "outcome", serde_json::json!("SUCCEEDED")),
            (2, "finished_at", Value::Null),
        ] {
            let mut changed = canceled.clone();
            changed[index].2[field] = value;
            assert!(
                consume_malformed_history(&changed)
                    .finish_r4_pairs()
                    .is_err()
            );
        }
        let mut expired = invalid_simulation_rows(SimulationContractVersion::V2);
        expired[2].2["outcome"] = serde_json::json!("ORPHANED");
        // Orphaning a RUNNING attempt preserves null errors, not a failed
        // attempt's input error. A mismatched final-state combination rejects.
        assert!(
            consume_malformed_history(&expired)
                .finish_r4_pairs()
                .is_err()
        );
        let mut domains = consume_malformed_history(&canceled);
        domains.deadline = Some(std::time::Instant::now());
        assert!(domains.finish_r4_pairs().is_err());
        let mut domains = consume_malformed_history(&canceled);
        domains.max_bytes = domains.used_bytes;
        assert!(domains.finish_r4_pairs().is_err());
    }

    #[test]
    fn invalid_simulation_exception_keeps_valid_input_and_success_coverage_strict() {
        let original = pair_fixture_rows();
        let mut no_pair = original.clone();
        no_pair.pop();
        assert!(
            consume_malformed_history(&no_pair)
                .finish_r4_pairs()
                .is_err()
        );
        let mut orphan = original.clone();
        orphan.remove(0);
        assert!(
            consume_malformed_history(&orphan)
                .finish_r4_pairs()
                .is_err()
        );
        // Known valid terminal input still takes the full parser and all raw
        // input/run comparisons rather than the invalid-input settlement path.
        no_pair[0].2["status"] = serde_json::json!("FAILED");
        no_pair[0].2["engine_json"] = Value::Null;
        no_pair[0].2["published_attempt_id"] = Value::Null;
        no_pair[1].2["status"] = serde_json::json!("FAILED");
        assert_eq!(
            consume_malformed_history(&no_pair).finish_r4_pairs(),
            Ok(())
        );
        no_pair[0].2["evaluation_start"] = serde_json::json!("2026-08-26");
        assert!(
            consume_malformed_history(&no_pair)
                .finish_r4_pairs()
                .is_err()
        );
    }

    #[test]
    fn complete_result_pair_uses_own_input_engine_and_both_original_bodies() {
        let rows = pair_fixture_rows();
        let mut domains = Domains::new(1000, 16 * 1024 * 1024);
        add_rows(&mut domains, &rows);
        assert_eq!(domains.finish_r4_pairs(), Ok(()));
        for mutation in 0..7 {
            let mut changed = rows.clone();
            match mutation {
                0 => {
                    changed.pop();
                }
                1 => changed[0].2["owner_user_id"] = serde_json::json!(uuid::Uuid::from_u128(99)),
                2 => {
                    changed[0].2["engine_json"]["binary_sha256"] =
                        serde_json::json!(format!("sha256:{}", "f".repeat(64)))
                }
                3 => changed[2].2["outcome"] = serde_json::json!("FAILED"),
                4 => {
                    changed[1].2["payload_json"]["strategy"]["config_json"]["fast_ma"] =
                        serde_json::json!(49)
                }
                5 => {
                    let mut orphan = changed[1].clone();
                    orphan.1 = uuid::Uuid::from_u128(99).to_string();
                    orphan.2["id"] = Value::String(orphan.1.clone());
                    changed.push(orphan);
                }
                _ => {
                    let mut bytes =
                        decode_hex(changed[3].2["illustrative_canonical"].as_str().unwrap())
                            .unwrap();
                    bytes.push(b' '); // Rehashed but noncanonical body must still be rejected.
                    changed[3].2["illustrative_canonical"] =
                        Value::String(bytes.iter().map(|b| format!("{b:02x}")).collect());
                    changed[3].2["illustrative_sha256"] = Value::String(digest_bytes(&bytes));
                }
            }
            let mut bad = Domains::new(1000, 16 * 1024 * 1024);
            add_rows(&mut bad, &changed);
            assert!(bad.finish_r4_pairs().is_err());
        }
    }
    #[test]
    fn complete_domains_accept_nonempty_release_and_result_pair_but_not_missing_graph() {
        let original = release_fixture();
        let mut domains = consume_release(&original);
        domains.observe(ArchiveOriginalMember::Member {path:"paper_operations",canonical_record:br#"{"gid":0,"kind":"directory","mode":448,"path":"paper_operations","sha256":null,"size":0,"uid":0}"#,file:None}).unwrap();
        add_rows(&mut domains, &pair_fixture_rows());
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
        domains.release_members.remove("paper_config/process.json");
        assert_eq!(
            domains.finish(&Value::Null, &Value::Null),
            Err(SourceClosureError::Release)
        );
    }
    fn lifecycle_dir(domains: &mut Domains, path: &str) -> Result<(), SourceClosureError> {
        let record = serde_json::to_vec(&serde_json::json!({"path":path,"kind":"directory","uid":0,"gid":0,"mode":448,"size":0,"sha256":null})).unwrap();
        domains.observe(ArchiveOriginalMember::Member {
            path,
            canonical_record: &record,
            file: None,
        })
    }

    #[test]
    fn lifecycle_member_callback_reaches_complete_finish_and_rejects_unsupported() {
        let mut domains = consume_release(&release_fixture());
        lifecycle_dir(&mut domains, "paper_operations").unwrap();
        add_rows(&mut domains, &pair_fixture_rows());
        for path in [
            "paper_evidence/lifecycle",
            "paper_evidence/lifecycle/queries",
            "paper_evidence/lifecycle/selections",
            "paper_evidence/lifecycle/anchors",
            "paper_evidence/lifecycle/queries/00000000-0000-4000-8000-000000000001",
        ] {
            lifecycle_dir(&mut domains, path).unwrap();
        }
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
        lifecycle_dir(
            &mut domains,
            "paper_evidence/lifecycle/selections/unexpected",
        )
        .unwrap();
        assert_eq!(
            domains.finish(&Value::Null, &Value::Null),
            Err(SourceClosureError::Journal)
        );
    }

    fn lifecycle_file(domains: &mut Domains, path: &str, bytes: &[u8]) {
        use std::io::Write;
        let mut fd = tempfile::tempfile().unwrap();
        fd.write_all(bytes).unwrap();
        let mode = if path.ends_with("installation-id") || path.ends_with("epoch-floor") {
            0o600
        } else {
            0o400
        };
        let record = serde_json::to_vec(&serde_json::json!({"path":path,"kind":"file","uid":0,"gid":0,"mode":mode,"size":bytes.len(),"sha256":digest_bytes(bytes)})).unwrap();
        domains
            .observe(ArchiveOriginalMember::Member {
                path,
                canonical_record: &record,
                file: Some(&fd),
            })
            .unwrap();
    }
    fn lifecycle_complete_fixture() -> (Domains, String, String) {
        let files = release_fixture();
        let mut process: Value =
            serde_json::from_slice(files["paper_config/process.json"].as_ref().unwrap()).unwrap();
        process["control_epoch"] = serde_json::json!(2);
        let process_original =
            crate::owner_beta::paper::source_evidence::canonical(&process).unwrap();
        let process_bytes = &process_original;
        let mut domains = consume_release(&files);
        add_rows(&mut domains, &pair_fixture_rows());
        for p in [
            "paper_operations",
            "paper_operations/operations",
            LIFECYCLE,
            "paper_evidence/lifecycle/queries",
            "paper_evidence/lifecycle/selections",
            "paper_evidence/lifecycle/anchors",
        ] {
            lifecycle_dir(&mut domains, p).unwrap();
        }
        let installation = "00000000-0000-4000-8000-000000000002";
        lifecycle_file(
            &mut domains,
            "paper_operations/installation-id",
            format!("{installation}\n").as_bytes(),
        );
        lifecycle_file(&mut domains, "paper_operations/epoch-floor", b"5\n");
        let event_id = "00000000-0000-4000-8000-000000000121";
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-01T00:00:00.123456Z")
            .unwrap()
            .timestamp_micros();
        let reset = serde_json::json!({"account_id":process["account_id"],"event_id":event_id,"kind":"OFF","control_epoch":1,"occurred_at_us":at,"payload_sha256":format!("sha256:{}", "a".repeat(64))});
        let row = serde_json::json!({"id":event_id,"account_id":process["account_id"],"kind":"OFF","control_epoch":1,"occurred_at":"2026-11-01T00:00:00.123456Z","payload_sha256":reset["payload_sha256"],"request_canonical":null,"request_sha256":null,"result_canonical":null,"result_sha256":null});
        domains
            .observe(ArchiveOriginalMember::Database {
                group: "paper",
                table: "owner_beta_paper_events",
                key: event_id,
                canonical_row: &serde_json::to_vec(&row).unwrap(),
            })
            .unwrap();
        let canonical =
            |v: &Value| crate::owner_beta::paper::source_evidence::canonical(v).unwrap();
        let selection = serde_json::json!({"schema":"owner-beta-paper-reset-selection-v1","installation_id":installation,"reset":reset,"release_manifest_sha256":process["release_manifest_sha256"],"cohort_sha256":process["daily_cohort_sha256"]});
        let selection_bytes = canonical(&selection);
        let selection_path = format!(
            "{LIFECYCLE}/selections/{}.json",
            &digest_bytes(&selection_bytes)[7..]
        );
        lifecycle_file(&mut domains, &selection_path, &selection_bytes);
        let mut anchor = selection.clone();
        anchor["schema"] = serde_json::json!("owner-beta-paper-reset-anchor-v1");
        anchor["boot_id"] = serde_json::json!("00000000-0000-4000-8000-000000000122");
        anchor["observed"] =
            serde_json::json!({"realtime_ns":at*1000+1,"monotonic_ns":1,"boottime_ns":1});
        let anchor_bytes = canonical(&anchor);
        let anchor_path = format!(
            "{LIFECYCLE}/anchors/{}.json",
            &digest_bytes(&anchor_bytes)[7..]
        );
        lifecycle_file(&mut domains, &anchor_path, &anchor_bytes);
        for (n, verb) in [(130u128, "readback"), (131, "observe-reset")] {
            let id = uuid::Uuid::from_u128(n).to_string();
            let dir = format!("{LIFECYCLE}/queries/{id}");
            lifecycle_dir(&mut domains, &dir).unwrap();
            lifecycle_dir(&mut domains, &format!("{dir}/output")).unwrap();
            let root = serde_json::json!({"schema":"owner-beta-paper-operation-intent-v1","installation_id":installation,"generation":id,"account_id":process["account_id"],"predecessor_generation":null,"request_key":format!("{id}:inspect"),"purpose":"INSPECT","scope":"PAPER_STOP","expected_epoch":1,"local_epoch_floor":1,"release_manifest_sha256":process["release_manifest_sha256"],"artifact_sha256":if verb=="readback"{Value::Null}else{serde_json::json!(digest_bytes(process_bytes))},"resume_target_id":null});
            lifecycle_file(
                &mut domains,
                &format!("{dir}/request.json"),
                &canonical(&root),
            );
            let artifact = if verb == "readback" {
                let q = wire::ExactRequest::reset(
                    uuid::Uuid::parse_str(process["account_id"].as_str().unwrap()).unwrap(),
                    1,
                )
                .unwrap();
                lifecycle_file(
                    &mut domains,
                    &format!("{dir}/sql-request.json"),
                    q.canonical.as_bytes(),
                );
                canonical(
                    &serde_json::json!({"schema":"owner-beta-paper-exact-readback-v1","kind":"RESET","request_sha256":q.sha256,"status":"RESET","account_id":process["account_id"],"current_epoch":1,"current_state":"OFF","event_id":null,"event_epoch":null,"result_canonical":null,"result_sha256":null,"reset":reset}),
                )
            } else {
                lifecycle_file(&mut domains, &format!("{dir}/evidence.json"), process_bytes);
                anchor_bytes.clone()
            };
            let hash = digest_bytes(&artifact);
            lifecycle_file(
                &mut domains,
                &format!("{dir}/output/{}.json", &hash[7..]),
                &artifact,
            );
            lifecycle_file(&mut domains, &format!("{dir}/artifact.json"), &artifact);
            // Real ResultWire order (not sorted); optional terminal LF.
            let native = format!(
                "{{\"schema\":\"owner-beta-paper-ops-query-result-v1\",\"verb\":\"{verb}\",\"outcome\":\"OK\",\"event_id\":{},\"generation\":\"{id}\",\"control_epoch\":1,\"artifact_sha256\":\"{hash}\",\"error_code\":\"NONE\"}}\n",
                if verb == "readback" {
                    "null".into()
                } else {
                    format!("\"{event_id}\"")
                }
            );
            lifecycle_file(
                &mut domains,
                &format!("{dir}/native-result.json"),
                native.as_bytes(),
            );
        }
        (domains, selection_path, anchor_path)
    }
    #[test]
    fn lifecycle_complete_original_callbacks_reach_mandatory_finish() {
        let (domains, _, _) = lifecycle_complete_fixture();
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
    }
    #[test]
    fn lifecycle_correlations_sql_names_order_and_shared_limits_refuse() {
        for mutation in 0..9 {
            let (mut domains, selection, anchor) = lifecycle_complete_fixture();
            match mutation {
                0 => {
                    domains.members.remove(&selection);
                }
                1 => {
                    let v = domains.members.remove(&anchor).unwrap();
                    domains
                        .members
                        .insert(format!("{LIFECYCLE}/anchors/{}.json", "f".repeat(64)), v);
                }
                2 | 3 | 4 => {
                    let raw = domains
                        .rows
                        .get_mut(&(
                            "owner_beta_paper_events".into(),
                            "00000000-0000-4000-8000-000000000121".into(),
                        ))
                        .unwrap();
                    let mut row: Value = serde_json::from_slice(raw).unwrap();
                    if mutation == 2 {
                        row["payload_sha256"] =
                            serde_json::json!(format!("sha256:{}", "b".repeat(64)));
                    }
                    if mutation == 3 {
                        row["occurred_at"] = serde_json::json!("2026-11-01T00:00:00.123457Z");
                    }
                    if mutation == 4 {
                        row["control_epoch"] = serde_json::json!(2);
                    }
                    *raw = serde_json::to_vec(&row).unwrap();
                }
                5 => {
                    domains
                        .members
                        .retain(|p, _| !p.ends_with("/native-result.json"));
                }
                6 => {
                    domains.max_bytes = domains.used_bytes;
                }
                7 => {
                    domains.deadline = Some(std::time::Instant::now());
                }
                _ => {
                    let p = domains
                        .members
                        .keys()
                        .find(|p| p.ends_with("/native-result.json"))
                        .unwrap()
                        .clone();
                    let raw = domains.members.get_mut(&p).unwrap().bytes.as_mut().unwrap();
                    raw.splice(1..1, b"\"verb\":\"readback\",".iter().copied());
                }
            }
            assert!(
                domains.finish(&Value::Null, &Value::Null).is_err(),
                "mutation {mutation}"
            );
        }
    }
    #[test]
    fn lifecycle_producer_prefixes_and_missing_query_copy_are_descriptions() {
        let (mut domains, _, _) = lifecycle_complete_fixture();
        // A query can stop after creating its dir or after any ordered input.
        // Output is written by native before stdout and the selected copy.
        domains.members.retain(|p, _| {
            !p.starts_with("paper_evidence/lifecycle/anchors/")
                && !p.starts_with("paper_evidence/lifecycle/selections/")
        });
        domains.members.retain(|p, _| {
            !p.ends_with("/native-result.json")
                && !p.ends_with("/artifact.json")
                && !p.contains("/output/")
        });
        // observe-reset's selection was already published before its query;
        // retain only the artifact-free RESET readback input prefix here.
        let observation = uuid::Uuid::from_u128(131).to_string();
        domains.members.retain(|p, _| !p.contains(&observation));
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
    }

    fn add_bound_readback_query(
        domains: &mut Domains,
        root: &[u8],
        binding: &[u8],
        evidence: Option<&[u8]>,
        q: &[u8],
        read: &Value,
        id: u128,
        uncertain: bool,
    ) -> String {
        let root_value = wire::parse_root_intent(root).unwrap();
        let action = wire::parse_action(binding, &root_value).unwrap();
        let out = format!(
            "paper_operations/operations/{}/actions/{:08}/output",
            root_value.intent.generation, action.sequence
        );
        let bytes = crate::owner_beta::paper::source_evidence::canonical(read).unwrap();
        let hash = digest_bytes(&bytes);
        let original = format!("{out}/{}.json", &hash[7..]);
        if !domains.members.contains_key(&original) {
            lifecycle_file(domains, &original, &bytes);
        }
        let dir = format!("{LIFECYCLE}/queries/{}", uuid::Uuid::from_u128(id));
        lifecycle_dir(domains, &dir).unwrap();
        lifecycle_dir(domains, &format!("{dir}/output")).unwrap();
        for (leaf, raw) in [
            ("request.json", root),
            ("action-binding.json", binding),
            ("sql-request.json", q),
        ] {
            lifecycle_file(domains, &format!("{dir}/{leaf}"), raw);
        }
        if let Some(evidence) = evidence {
            lifecycle_file(domains, &format!("{dir}/evidence.json"), evidence);
        }
        lifecycle_file(domains, &format!("{dir}/artifact.json"), &bytes);
        let outcome = if uncertain || read["status"] != "COMMITTED" {
            "COMMIT_UNKNOWN"
        } else {
            "REPLAY"
        };
        let error = if outcome == "COMMIT_UNKNOWN" {
            "COMMIT_UNKNOWN"
        } else {
            "NONE"
        };
        let native = format!(
            "{{\"schema\":\"owner-beta-paper-ops-query-result-v1\",\"verb\":\"readback\",\"outcome\":\"{outcome}\",\"event_id\":{},\"generation\":\"{}\",\"control_epoch\":{},\"artifact_sha256\":\"{hash}\",\"error_code\":\"{error}\"}}",
            read["event_id"], root_value.intent.generation, read["current_epoch"]
        );
        lifecycle_file(
            domains,
            &format!("{dir}/native-result.json"),
            native.as_bytes(),
        );
        dir
    }

    fn lifecycle_mutations_fixture(copy_q: bool) -> (Domains, String) {
        let (mut domains, _, _) = lifecycle_complete_fixture();
        let f = fixtures();
        let configured: Value = serde_json::from_str(object(&f, "configure.result")).unwrap();
        let proposal: Value = serde_json::from_str(object(&f, "configure.artifact")).unwrap();
        let control = serde_json::json!({"account_id":configured["account_id"],"owner_user_id":"00000000-0000-4000-8000-000000000011","strategy_config_id":proposal["strategy_config_id"],"contract_id":proposal["contract"]["contract_id"],"contract_sha256":proposal["contract"]["contract_sha256"],"contract_json":proposal["contract"],"cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1,"cost_profile_sha256":digest_bytes(b"KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000"),"consent_event_id":null,"process_binding":null,"process_binding_sha256":null,"activation_event_id":null,"control_epoch":5});
        domains
            .observe(ArchiveOriginalMember::Database {
                group: "paper",
                table: "owner_beta_paper_control",
                key: "mutation-account",
                canonical_row: &serde_json::to_vec(&control).unwrap(),
            })
            .unwrap();
        let mut last_query = String::new();
        for (verb, n) in [("configure", 140u128), ("recover", 141)] {
            let root = object(&f, &format!("{verb}.root")).as_bytes();
            let parsed = wire::parse_root_intent(root).unwrap();
            let binding = object(&f, &format!("{verb}.action0")).as_bytes();
            let evidence = object(&f, &format!("{verb}.artifact")).as_bytes();
            let q = object(&f, &format!("{verb}.action0.sql")).as_bytes();
            let sql = object(&f, &format!("{verb}.result")).as_bytes();
            let result: Value = serde_json::from_slice(sql).unwrap();
            let base = format!("paper_operations/operations/{}", parsed.intent.generation);
            let action = format!("{base}/actions/00000000");
            let out = format!("{action}/output");
            for p in [&base, &format!("{base}/actions"), &action, &out] {
                lifecycle_dir(&mut domains, p).unwrap();
            }
            lifecycle_file(&mut domains, &format!("{base}/intent.json"), root);
            lifecycle_file(&mut domains, &format!("{action}/binding.json"), binding);
            lifecycle_file(&mut domains, &format!("{action}/evidence.json"), evidence);
            lifecycle_file(
                &mut domains,
                &format!("{out}/dispatch.json"),
                object(&f, &format!("{verb}.action0.dispatch")).as_bytes(),
            );
            lifecycle_file(
                &mut domains,
                &format!("{action}/import.json"),
                object(&f, &format!("{verb}.action0.dispatch")).as_bytes(),
            );
            for raw in [
                root,
                binding,
                evidence,
                q,
                object(&f, &format!("{verb}.action0.dispatch")).as_bytes(),
            ] {
                lifecycle_file(
                    &mut domains,
                    &format!("{out}/{}.json", &digest_bytes(raw)[7..]),
                    raw,
                );
            }
            let mut row = serde_json::json!({"id":result["data"]["event_id"],"account_id":result["account_id"],"owner_user_id":control["owner_user_id"],"control_epoch":result["control_epoch"],"kind":if verb=="configure"{"CONFIGURED"}else{"RECOVERY_AUTHORIZED"},"request_key":parsed.intent.request_key,"request_canonical":std::str::from_utf8(q).unwrap(),"request_sha256":digest_bytes(q),"result_canonical":std::str::from_utf8(sql).unwrap(),"result_sha256":digest_bytes(sql)});
            if verb == "configure" {
                row["payload"] = serde_json::json!({"schema":"owner-beta-paper-configured-v1","strategy_config_id":proposal["strategy_config_id"],"contract":proposal["contract"]});
            }
            domains
                .observe(ArchiveOriginalMember::Database {
                    group: "paper",
                    table: "owner_beta_paper_events",
                    key: verb,
                    canonical_row: &serde_json::to_vec(&row).unwrap(),
                })
                .unwrap();
            let artifact = if verb == "configure" {
                evidence.to_vec()
            } else {
                crate::owner_beta::paper::source_evidence::canonical(&serde_json::json!({"schema":"owner-beta-paper-exact-readback-v1","kind":"TRANSITION","request_sha256":digest_bytes(q),"status":"COMMITTED","account_id":result["account_id"],"current_epoch":4,"current_state":"OFF","event_id":result["data"]["event_id"],"event_epoch":3,"result_canonical":std::str::from_utf8(sql).unwrap(),"result_sha256":digest_bytes(sql),"reset":null})).unwrap()
            };
            let artifact_path = format!("{out}/{}.json", &digest_bytes(&artifact)[7..]);
            if !domains.members.contains_key(&artifact_path) {
                lifecycle_file(&mut domains, &artifact_path, &artifact);
            }
            let dir = format!("{LIFECYCLE}/queries/{}", uuid::Uuid::from_u128(n));
            lifecycle_dir(&mut domains, &dir).unwrap();
            lifecycle_dir(&mut domains, &format!("{dir}/output")).unwrap();
            for (name, raw) in [
                ("request.json", root),
                ("evidence.json", evidence),
                ("action-binding.json", binding),
            ] {
                lifecycle_file(&mut domains, &format!("{dir}/{name}"), raw);
            }
            if copy_q {
                lifecycle_file(&mut domains, &format!("{dir}/sql-request.json"), q);
            }
            lifecycle_file(&mut domains, &format!("{dir}/artifact.json"), &artifact);
            let generation = if verb == "configure" {
                "null".into()
            } else {
                format!("\"{}\"", parsed.intent.generation)
            };
            let epoch = if verb == "configure" { 0 } else { 4 };
            let outcome = if verb == "recover" { "REPLAY" } else { "OK" };
            let native = format!(
                "{{\"schema\":\"owner-beta-paper-ops-result-v1\",\"verb\":\"{verb}\",\"outcome\":\"{outcome}\",\"event_id\":\"{}\",\"generation\":{generation},\"control_epoch\":{epoch},\"artifact_sha256\":\"{}\",\"error_code\":\"NONE\"}}",
                result["data"]["event_id"].as_str().unwrap(),
                digest_bytes(&artifact)
            );
            lifecycle_file(
                &mut domains,
                &format!("{dir}/native-result.json"),
                native.as_bytes(),
            );
            let read = serde_json::json!({"schema":"owner-beta-paper-exact-readback-v1","kind":if verb=="configure"{"CONFIGURE"}else{"TRANSITION"},"request_sha256":digest_bytes(q),"status":"COMMITTED","account_id":result["account_id"],"current_epoch":if verb=="configure"{0}else{5},"current_state":"OFF","event_id":result["data"]["event_id"],"event_epoch":result["control_epoch"],"result_canonical":std::str::from_utf8(sql).unwrap(),"result_sha256":digest_bytes(sql),"reset":null});
            add_bound_readback_query(
                &mut domains,
                root,
                binding,
                Some(evidence),
                q,
                &read,
                n + 100,
                false,
            );
            last_query = dir;
        }
        (domains, last_query)
    }
    #[test]
    fn lifecycle_mutation_originals_and_crash_before_q_copy_reach_complete_finish() {
        for copy_q in [false, true] {
            let (domains, _) = lifecycle_mutations_fixture(copy_q);
            let index = domains
                .journal_index(
                    "paper_operations/callers",
                    "00000000-0000-4000-8000-000000000002",
                )
                .unwrap();
            for verb in ["configure", "recover"] {
                let sql = object(&fixtures(), &format!("{verb}.result")).to_owned();
                assert!(!index.objects.contains_key(&digest_bytes(sql.as_bytes())));
                let q = object(&fixtures(), &format!("{verb}.action0.sql")).to_owned();
                assert_eq!(
                    index
                        .enclosed_results
                        .get(&digest_bytes(q.as_bytes()))
                        .map(String::as_str),
                    Some(sql.as_str())
                );
            }
            assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
        }
    }
    #[test]
    fn lifecycle_mutation_journal_sql_and_result_fields_are_independent() {
        for mutant in 0..5 {
            let (mut domains, dir) = lifecycle_mutations_fixture(true);
            match mutant {
                0 => {
                    domains
                        .members
                        .get_mut(&format!("{dir}/evidence.json"))
                        .unwrap()
                        .bytes = Some(b"{}".to_vec());
                }
                1 => {
                    domains
                        .members
                        .get_mut(&format!("{dir}/action-binding.json"))
                        .unwrap()
                        .bytes = Some(b"{}".to_vec());
                }
                2 => {
                    domains
                        .members
                        .get_mut(&format!("{dir}/sql-request.json"))
                        .unwrap()
                        .bytes = Some(b"{}".to_vec());
                }
                3 => {
                    domains
                        .rows
                        .remove(&("owner_beta_paper_events".into(), "recover".into()));
                }
                _ => {
                    let raw = domains
                        .members
                        .get_mut(&format!("{dir}/native-result.json"))
                        .unwrap()
                        .bytes
                        .as_mut()
                        .unwrap();
                    let value = String::from_utf8(raw.clone())
                        .unwrap()
                        .replace("\"control_epoch\":4", "\"control_epoch\":5");
                    *raw = value.into_bytes();
                }
            }
            assert!(
                domains.finish(&Value::Null, &Value::Null).is_err(),
                "mutant {mutant}"
            );
        }
    }

    fn replace_journal_envelope(domains: &mut Domains, path: &str, raw: Vec<u8>) {
        domains.members.remove(path).unwrap();
        let parent = path.rsplit_once('/').unwrap().0;
        domains.members.insert(
            format!("{parent}/{}.json", &digest_bytes(&raw)[7..]),
            Member { bytes: Some(raw) },
        );
    }
    fn configure_envelope(domains: &Domains) -> (String, Value) {
        domains.members.iter().find_map(|(path,member)| {
            if !path.starts_with("paper_operations/operations/00000000-0000-4000-8000-000000000040/actions/00000000/output/") {return None;}
            let value:Value=serde_json::from_slice(member.bytes.as_deref()?).ok()?;
            (value["schema"]=="owner-beta-paper-exact-readback-v1").then(||(path.clone(),value))
        }).unwrap()
    }
    #[test]
    fn envelope_only_receipts_reject_request_event_result_snapshot_and_canonical_mutants() {
        for mutant in 0..11 {
            let (mut domains, _) = lifecycle_mutations_fixture(true);
            let (path, mut read) = configure_envelope(&domains);
            match mutant {
                0 => read["request_sha256"] = serde_json::json!(digest_bytes(b"different Q")),
                1 => read["event_id"] = serde_json::json!("00000000-0000-4000-8000-000000000999"),
                2 => read["result_sha256"] = serde_json::json!(digest_bytes(b"different SQL")),
                3 => {
                    let mut nested: Value =
                        serde_json::from_str(read["result_canonical"].as_str().unwrap()).unwrap();
                    nested["data"]["event_id"] =
                        serde_json::json!("00000000-0000-4000-8000-000000000999");
                    let text = String::from_utf8(
                        crate::owner_beta::paper::source_evidence::canonical(&nested).unwrap(),
                    )
                    .unwrap();
                    read["event_id"] = nested["data"]["event_id"].clone();
                    read["result_sha256"] = serde_json::json!(digest_bytes(text.as_bytes()));
                    read["result_canonical"] = serde_json::json!(text);
                }
                4 => {
                    let text = format!("{} ", read["result_canonical"].as_str().unwrap());
                    read["result_sha256"] = serde_json::json!(digest_bytes(text.as_bytes()));
                    read["result_canonical"] = serde_json::json!(text);
                }
                5 => read["current_epoch"] = serde_json::json!(6),
                6 => read["unexpected"] = serde_json::json!(null),
                7 => {
                    let raw = domains
                        .rows
                        .get_mut(&(RECEIPT_TABLES[2].into(), "configure".into()))
                        .unwrap();
                    let mut row: Value = serde_json::from_slice(raw).unwrap();
                    row["id"] = serde_json::json!("00000000-0000-4000-8000-000000000999");
                    *raw = serde_json::to_vec(&row).unwrap();
                }
                8 => {
                    let q = object(&fixtures(), "configure.action0.sql").to_owned();
                    domains.members.remove(&format!("paper_operations/operations/00000000-0000-4000-8000-000000000040/actions/00000000/output/{}.json",&digest_bytes(q.as_bytes())[7..]));
                }
                _ => {}
            }
            let mut raw = crate::owner_beta::paper::source_evidence::canonical(&read).unwrap();
            if mutant == 9 {
                raw.splice(1..1, b"\"status\":\"COMMITTED\",".iter().copied());
            }
            if mutant == 10 {
                raw.push(b'\n');
            }
            replace_journal_envelope(&mut domains, &path, raw);
            // Test the shared index directly, so rejections cannot be credited
            // to a later stale lifecycle selected-artifact hash.
            assert!(
                domains
                    .journal_index(
                        "paper_operations/callers",
                        "00000000-0000-4000-8000-000000000002"
                    )
                    .is_err(),
                "mutant {mutant}"
            );
            assert!(domains.finish(&Value::Null, &Value::Null).is_err());
        }
    }
    #[test]
    fn noncommitted_envelopes_never_substitute_for_sql_but_uncertain_histories_remain() {
        let (mut domains, _) = lifecycle_mutations_fixture(true);
        let f = fixtures();
        let (_, mut read) = configure_envelope(&domains);
        read["status"] = serde_json::json!("UNPROVEN");
        for key in [
            "event_id",
            "event_epoch",
            "result_canonical",
            "result_sha256",
            "reset",
        ] {
            read[key] = Value::Null;
        }
        add_bound_readback_query(
            &mut domains,
            object(&f, "configure.root").as_bytes(),
            object(&f, "configure.action0").as_bytes(),
            Some(object(&f, "configure.artifact").as_bytes()),
            object(&f, "configure.action0.sql").as_bytes(),
            &read,
            242,
            true,
        );
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
        // Remove every COMMITTED configure envelope, preserving actual
        // UNPROVEN descriptions without granting them result credit.
        domains.members.retain(|path,member| {
            if !path.starts_with("paper_operations/operations/00000000-0000-4000-8000-000000000040/actions/00000000/output/") {return true;}
            let Some(raw)=member.bytes.as_deref() else{return true;};
            let v:Value=serde_json::from_slice(raw).unwrap();
            !(v["schema"]=="owner-beta-paper-exact-readback-v1" && v["status"]=="COMMITTED")
        });
        let index = domains
            .journal_index(
                "paper_operations/callers",
                "00000000-0000-4000-8000-000000000002",
            )
            .unwrap();
        let row: Value =
            serde_json::from_slice(&domains.rows[&(RECEIPT_TABLES[2].into(), "configure".into())])
                .unwrap();
        assert!(
            !index
                .enclosed_results
                .contains_key(row["request_sha256"].as_str().unwrap())
        );
        assert_eq!(
            domains.check_receipt(RECEIPT_TABLES[2], &row, &index),
            Err(ErrorCode::EvidenceUnavailable)
        );
    }

    fn maintenance_readbacks_fixture(prefix: &str, uncertain: bool) -> (Domains, Vec<String>) {
        let mut domains = consume_release(&release_fixture());
        add_rows(&mut domains, &pair_fixture_rows());
        for path in [
            JOURNAL,
            "paper_operations/operations",
            LIFECYCLE,
            "paper_evidence/lifecycle/queries",
            "paper_evidence/lifecycle/selections",
            "paper_evidence/lifecycle/anchors",
        ] {
            lifecycle_dir(&mut domains, path).unwrap();
        }
        let f = fixtures();
        let root = object(&f, &format!("{prefix}.root")).as_bytes();
        let parsed = wire::parse_root_intent(root).unwrap();
        lifecycle_file(
            &mut domains,
            "paper_operations/installation-id",
            format!("{}\n", parsed.intent.installation_id).as_bytes(),
        );
        let current = parsed.fence_epoch().unwrap() + 1;
        lifecycle_file(
            &mut domains,
            "paper_operations/epoch-floor",
            format!("{current}\n").as_bytes(),
        );
        let base = format!("{JOURNAL}/operations/{}", parsed.intent.generation);
        lifecycle_dir(&mut domains, &base).unwrap();
        lifecycle_dir(&mut domains, &format!("{base}/actions")).unwrap();
        lifecycle_file(&mut domains, &format!("{base}/intent.json"), root);
        for n in 0..=5 {
            let raw = object(&f, &format!("{prefix}.stage{n}")).as_bytes();
            lifecycle_file(&mut domains, &format!("{base}/{n:08}.json"), raw);
            lifecycle_file(
                &mut domains,
                &format!("{base}/{}.json", &digest_bytes(raw)[7..]),
                raw,
            );
        }
        let count = if prefix == "stop" { 4 } else { 3 };
        let mut queries = Vec::new();
        for n in 0..count {
            let binding = object(&f, &format!("{prefix}.action{n}")).as_bytes();
            let dispatch_bytes = object(&f, &format!("{prefix}.action{n}.dispatch")).as_bytes();
            let dispatch = wire::BoundDispatch::parse(dispatch_bytes).unwrap();
            let action = format!("{base}/actions/{n:08}");
            let output = format!("{action}/output");
            lifecycle_dir(&mut domains, &action).unwrap();
            lifecycle_dir(&mut domains, &output).unwrap();
            lifecycle_file(&mut domains, &format!("{action}/binding.json"), binding);
            lifecycle_file(
                &mut domains,
                &format!("{action}/import.json"),
                dispatch_bytes,
            );
            lifecycle_file(
                &mut domains,
                &format!("{output}/dispatch.json"),
                dispatch_bytes,
            );
            lifecycle_file(
                &mut domains,
                &format!("{output}/{}.json", &digest_bytes(dispatch_bytes)[7..]),
                dispatch_bytes,
            );
            let envelope: Value = serde_json::from_slice(dispatch_bytes).unwrap();
            for key in [
                "root_intent_canonical",
                "action_canonical",
                "artifact_canonical",
                "sql_request_canonical",
            ] {
                if let Some(raw) = envelope[key].as_str() {
                    lifecycle_file(
                        &mut domains,
                        &format!("{output}/{}.json", &digest_bytes(raw.as_bytes())[7..]),
                        raw.as_bytes(),
                    );
                    if key == "artifact_canonical" {
                        lifecycle_file(
                            &mut domains,
                            &format!("{action}/evidence.json"),
                            raw.as_bytes(),
                        );
                    }
                }
            }
            let result = if n == 0 {
                serde_json::from_str(object(&f, &format!("{prefix}.begin-result"))).unwrap()
            } else if n == count - 1 {
                serde_json::from_str(object(&f, &format!("{prefix}.result"))).unwrap()
            } else {
                let summary: Value =
                    serde_json::from_str(object(&f, &format!("{prefix}.summary{n}"))).unwrap();
                let sha = digest_bytes(
                    &crate::owner_beta::paper::source_evidence::canonical(&summary).unwrap(),
                );
                serde_json::json!({"schema":"owner-beta-paper-action-v1","action":"RECONCILE_OPERATION","outcome":"OK","account_id":summary["account_id"],"control_epoch":summary["control_epoch"],"data":{"observation_id":summary["observation_id"],"summary":summary,"summary_sha256":sha}})
            };
            let sql = crate::owner_beta::paper::source_evidence::canonical(&result).unwrap();
            lifecycle_file(
                &mut domains,
                &format!("{output}/{}.json", &digest_bytes(&sql)[7..]),
                &sql,
            );
            let mut row = if n == 0 {
                let mut row = parsed.intent.value.clone();
                row["intent"] = parsed.intent.value.clone();
                row["intent_sha256"] = serde_json::json!(parsed.sha256);
                row["fence_event_id"] = result["data"]["operation_event_id"].clone();
                row["fence_epoch"] = result["control_epoch"].clone();
                row
            } else if n == count - 1 {
                serde_json::json!({"id":result["data"]["event_id"],"account_id":result["account_id"],"control_epoch":result["control_epoch"],"kind":if prefix=="restore"{"RESTORED_OFF"}else{"OFF"}})
            } else {
                serde_json::json!({"generation":parsed.intent.generation,"observation_id":result["data"]["observation_id"],"summary":result["data"]["summary"],"summary_sha256":result["data"]["summary_sha256"]})
            };
            row["request_key"] = dispatch.request.value["args"]["request_key"].clone();
            row["request_canonical"] = serde_json::json!(dispatch.request.canonical);
            row["request_sha256"] = serde_json::json!(dispatch.request.sha256);
            row["result_canonical"] = serde_json::json!(std::str::from_utf8(&sql).unwrap());
            row["result_sha256"] = serde_json::json!(digest_bytes(&sql));
            let table = if n == 0 {
                RECEIPT_TABLES[0]
            } else if n == count - 1 {
                RECEIPT_TABLES[2]
            } else {
                RECEIPT_TABLES[1]
            };
            domains
                .observe(ArchiveOriginalMember::Database {
                    group: "paper",
                    table,
                    key: &n.to_string(),
                    canonical_row: &serde_json::to_vec(&row).unwrap(),
                })
                .unwrap();
            let event = if n == 0 {
                &result["data"]["operation_event_id"]
            } else if n == count - 1 {
                &result["data"]["event_id"]
            } else {
                &result["data"]["observation_id"]
            };
            let read = serde_json::json!({"schema":"owner-beta-paper-exact-readback-v1","kind":dispatch.request.value["kind"],"request_sha256":dispatch.request.sha256,"status":"COMMITTED","account_id":result["account_id"],"current_epoch":current,"current_state":if prefix=="restore"{"RESTORED_OFF"}else{"OFF"},"event_id":event,"event_epoch":result["control_epoch"],"result_canonical":std::str::from_utf8(&sql).unwrap(),"result_sha256":digest_bytes(&sql),"reset":null});
            queries.push(add_bound_readback_query(
                &mut domains,
                root,
                binding,
                envelope["artifact_canonical"].as_str().map(str::as_bytes),
                dispatch.request.canonical.as_bytes(),
                &read,
                300 + n as u128,
                uncertain,
            ));
        }
        (domains, queries)
    }
    #[test]
    fn maintenance_readbacks_keep_begin_optional_evidence_and_actual_off_sequence() {
        for prefix in ["stop", "backup", "restore"] {
            for uncertain in [false, true] {
                let (domains, queries) = maintenance_readbacks_fixture(prefix, uncertain);
                assert!(
                    !domains
                        .members
                        .contains_key(&format!("{}/evidence.json", queries[0]))
                );
                if prefix == "backup" {
                    let binding = domains
                        .file(&format!("{}/action-binding.json", queries[2]))
                        .unwrap();
                    let root = wire::parse_root_intent(
                        domains
                            .file(&format!("{}/request.json", queries[2]))
                            .unwrap(),
                    )
                    .unwrap();
                    let action = wire::parse_action(binding, &root).unwrap();
                    assert_eq!(action.sequence, 2);
                    assert_eq!(action.verb.as_str(), "off");
                }
                assert_eq!(
                    domains.finish(&Value::Null, &Value::Null),
                    Ok(()),
                    "{prefix}, uncertain={uncertain}"
                );
            }
        }
    }
    #[test]
    fn maintenance_readback_correlations_and_mutation_allowlist_remain_closed() {
        for mutant in 0..7 {
            let (mut domains, queries) = maintenance_readbacks_fixture("backup", false);
            let dir = &queries[2];
            match mutant {
                0 => {
                    let raw = domains
                        .members
                        .get_mut(&format!("{dir}/action-binding.json"))
                        .unwrap()
                        .bytes
                        .as_mut()
                        .unwrap();
                    let mut a: Value = serde_json::from_slice(raw).unwrap();
                    a["sequence"] = serde_json::json!(1);
                    *raw = crate::owner_beta::paper::source_evidence::canonical(&a).unwrap();
                }
                1 => {
                    let wrong = domains
                        .file(&format!("{}/action-binding.json", queries[0]))
                        .unwrap()
                        .to_vec();
                    domains
                        .members
                        .get_mut(&format!("{dir}/action-binding.json"))
                        .unwrap()
                        .bytes = Some(wrong);
                }
                2 => {
                    let wrong = domains
                        .file(&format!("{}/sql-request.json", queries[0]))
                        .unwrap()
                        .to_vec();
                    domains
                        .members
                        .get_mut(&format!("{dir}/sql-request.json"))
                        .unwrap()
                        .bytes = Some(wrong);
                }
                3 => {
                    domains.members.remove("paper_operations/operations/00000000-0000-4000-8000-000000000020/actions/00000002/output/dispatch.json");
                }
                4 => {
                    let raw = domains
                        .members
                        .get_mut(&format!("{dir}/native-result.json"))
                        .unwrap()
                        .bytes
                        .as_mut()
                        .unwrap();
                    let text = String::from_utf8(raw.clone())
                        .unwrap()
                        .replace("query-result-v1", "result-v1")
                        .replace("\"verb\":\"readback\"", "\"verb\":\"off\"");
                    *raw = text.into_bytes();
                }
                5 => {
                    lifecycle_file(
                        &mut domains,
                        &format!("{}/evidence.json", queries[0]),
                        b"{}",
                    );
                }
                _ => {
                    let raw = domains
                        .members
                        .get_mut(&format!("{dir}/request.json"))
                        .unwrap()
                        .bytes
                        .as_mut()
                        .unwrap();
                    let mut root: Value = serde_json::from_slice(raw).unwrap();
                    root["release_manifest_sha256"] =
                        serde_json::json!(digest_bytes(b"wrong original"));
                    *raw = crate::owner_beta::paper::source_evidence::canonical(&root).unwrap();
                }
            }
            assert!(
                domains.finish(&Value::Null, &Value::Null).is_err(),
                "mutant {mutant}"
            );
        }
    }
    #[test]
    fn maintenance_readback_ordered_partial_inputs_are_retained() {
        for sequence in [0, 2] {
            for keep in 0..5 {
                let (mut domains, queries) = maintenance_readbacks_fixture("backup", false);
                let dir = &queries[sequence];
                let mut ordered = vec!["request.json"];
                if sequence != 0 {
                    ordered.push("evidence.json");
                }
                ordered.extend(["sql-request.json", "action-binding.json", "output"]);
                let selected = &ordered[..keep.min(ordered.len())];
                domains.members.retain(|path, _| {
                    let Some(leaf) = path.strip_prefix(&format!("{dir}/")) else {
                        return true;
                    };
                    selected.contains(&leaf)
                });
                assert_eq!(
                    domains.finish(&Value::Null, &Value::Null),
                    Ok(()),
                    "sequence {sequence}, keep {keep}"
                );
            }
        }
    }

    #[test]
    fn maintenance_readback_uncertainty_is_descriptive_even_after_later_commit() {
        let (mut domains, queries) = maintenance_readbacks_fixture("backup", false);
        let f = fixtures();
        let q = object(&f, "backup.action0.sql").as_bytes();
        let read = serde_json::json!({"schema":"owner-beta-paper-exact-readback-v1","kind":"BEGIN","request_sha256":digest_bytes(q),"status":"UNPROVEN","account_id":"00000000-0000-4000-8000-000000000001","current_epoch":0,"current_state":"OFF","event_id":null,"event_epoch":null,"result_canonical":null,"result_sha256":null,"reset":null});
        let dir = add_bound_readback_query(
            &mut domains,
            object(&f, "backup.root").as_bytes(),
            object(&f, "backup.action0").as_bytes(),
            None,
            q,
            &read,
            350,
            true,
        );
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
        let result = domains.file(&format!("{dir}/native-result.json")).unwrap();
        let value: Value = serde_json::from_slice(result).unwrap();
        assert_eq!(value["outcome"], "COMMIT_UNKNOWN");
        // A failed native connection can retain no selected readback at all.
        let unknown = &queries[0];
        let raw = domains
            .members
            .get_mut(&format!("{unknown}/native-result.json"))
            .unwrap()
            .bytes
            .as_mut()
            .unwrap();
        let v: Value = serde_json::from_slice(raw).unwrap();
        // Preserve native field order, including explicit nulls.
        *raw=format!("{{\"schema\":\"owner-beta-paper-ops-query-result-v1\",\"verb\":\"readback\",\"outcome\":\"COMMIT_UNKNOWN\",\"event_id\":null,\"generation\":{},\"control_epoch\":null,\"artifact_sha256\":null,\"error_code\":\"COMMIT_UNKNOWN\"}}",v["generation"]).into_bytes();
        domains.members.remove(&format!("{unknown}/artifact.json"));
        assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
    }

    #[test]
    fn lifecycle_enable_consumes_original_source_arms_and_snapshot_links() {
        use std::io::Write;
        let f = fixtures();
        let root = wire::parse_root_intent(object(&f, "enable.root").as_bytes()).unwrap();
        let action = wire::parse_action(object(&f, "enable.action0").as_bytes(), &root).unwrap();
        let q =
            wire::parse_exact_bound(object(&f, "enable.action0.sql").as_bytes(), &root, &action)
                .unwrap();
        let evidence: Value =
            serde_json::from_str(q.value["args"]["evidence_canonical"].as_str().unwrap()).unwrap();
        let source: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json"
        )))
        .unwrap();
        let canonical =
            |v: &Value| crate::owner_beta::paper::source_evidence::canonical(v).unwrap();
        let selection = serde_json::json!({"schema":"owner-beta-paper-reset-selection-v1","installation_id":root.intent.installation_id,"reset":{"account_id":root.intent.account_id,"event_id":"00000000-0000-4000-8000-000000000121","kind":"OFF","control_epoch":2,"occurred_at_us":1,"payload_sha256":format!("sha256:{}","a".repeat(64))},"release_manifest_sha256":root.intent.release_manifest_sha256,"cohort_sha256":evidence["current_cohort"]["cohort_sha256"]});
        let selected = wire::ResetSelection::parse(&canonical(&selection)).unwrap();
        for mutant in 0..6 {
            let mut domains = Domains::new(1000, 16 * 1024 * 1024);
            let mut anchor = selection.clone();
            anchor["schema"] = serde_json::json!("owner-beta-paper-reset-anchor-v1");
            anchor["boot_id"] = source["events"][1]["arm"]["boot_id"].clone();
            anchor["observed"] =
                serde_json::json!({"realtime_ns":2000,"monotonic_ns":1,"boottime_ns":1});
            if mutant == 1 {
                anchor["boot_id"] = serde_json::json!("00000000-0000-4000-8000-000000000123");
            }
            let raw = canonical(&anchor);
            let path = format!("{LIFECYCLE}/anchors/{}.json", &digest_bytes(&raw)[7..]);
            if mutant != 2 {
                lifecycle_file(&mut domains, &path, &raw);
            }
            for (i, session) in evidence["session_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let slot = digest_bytes(session["session_date"].as_str().unwrap().as_bytes());
                let mut slot_row = serde_json::json!({"slot_id":slot,"session_date":session["session_date"],"root_event_sha256":session["root_event_sha256"],"cohort":evidence["current_cohort"]});
                if mutant == 3 {
                    slot_row["root_event_sha256"] = serde_json::json!(digest_bytes(b"different"));
                }
                for (table, row) in [
                    (
                        "owner_beta_paper_source_runs",
                        serde_json::json!({"slot_id":slot,"run_id":session["run_id"]}),
                    ),
                    ("owner_beta_paper_source_slots", slot_row),
                    (
                        "owner_beta_paper_source_admissions",
                        serde_json::json!({"slot_id":slot,"source_event_sha256":session["source_event_sha256"],"admission_sha256":session["admission_sha256"]}),
                    ),
                ] {
                    domains
                        .observe(ArchiveOriginalMember::Database {
                            group: "paper",
                            table,
                            key: &i.to_string(),
                            canonical_row: &canonical(&row),
                        })
                        .unwrap();
                }
                for n in 1..=2 {
                    let mut event = source["events"][n].clone();
                    event["cohort"] = evidence["current_cohort"].clone();
                    event["session"] = session["session_date"].clone();
                    event["slot_id"] = serde_json::json!(slot);
                    if mutant == 4 {
                        event["observed"]["monotonic_ns"] = serde_json::json!(0);
                    }
                    if mutant == 5 {
                        event["kind"] = serde_json::json!("ARM_INTENT");
                    }
                    let raw = canonical(&event);
                    let path = format!(
                        "paper_evidence/source-0000/events/{}/{:06}.json",
                        &slot[7..],
                        n
                    );
                    let mut fd = tempfile::tempfile().unwrap();
                    fd.write_all(&raw).unwrap();
                    let record=serde_json::to_vec(&serde_json::json!({"path":path,"kind":"file","uid":0,"gid":10001,"mode":0o440,"size":raw.len(),"sha256":digest_bytes(&raw)})).unwrap();
                    domains
                        .observe(ArchiveOriginalMember::Member {
                            path: &path,
                            canonical_record: &record,
                            file: Some(&fd),
                        })
                        .unwrap();
                }
            }
            assert_eq!(
                domains
                    .lifecycle_enable(&root, &q, std::slice::from_ref(&selected))
                    .is_ok(),
                mutant == 0,
                "mutant {mutant}"
            );
        }
    }

    #[test]
    fn lifecycle_member_rejects_bad_metadata_duplicate_and_orphan() {
        let mut domains = Domains::new(2, 4096);
        let path = "paper_evidence/lifecycle/queries";
        let bad = serde_json::to_vec(&serde_json::json!({"path":path,"kind":"directory","uid":0,"gid":0,"mode":493,"size":0,"sha256":null})).unwrap();
        assert_eq!(
            domains.observe(ArchiveOriginalMember::Member {
                path,
                canonical_record: &bad,
                file: None
            }),
            Err(SourceClosureError::Metadata)
        );
        lifecycle_dir(&mut domains, path).unwrap();
        assert_eq!(
            lifecycle_dir(&mut domains, path),
            Err(SourceClosureError::Duplicate)
        );
        assert_eq!(domains.finish_lifecycle(), Err(SourceClosureError::Journal));
        lifecycle_dir(&mut domains, "paper_evidence/lifecycle").unwrap();
        assert_eq!(
            lifecycle_dir(&mut domains, "paper_evidence/lifecycle/anchors"),
            Err(SourceClosureError::Limit)
        );
    }

    #[test]
    fn missing_recommendation_material_and_expired_original_clock_refuse() {
        let mut domains = Domains::new(10, 1_048_576);
        add_rows(
            &mut domains,
            &[(
                "owner_beta_recommendation_runs",
                "run".into(),
                serde_json::json!({"id":"run"}),
            )],
        );
        assert_eq!(
            domains.finish_recommendations(),
            Err(SourceClosureError::RecommendationMaterialMissing)
        );
        let expired = Domains::until(10, 4096, std::time::Instant::now());
        assert_eq!(expired.check_time(), Err(SourceClosureError::Interrupted));
    }

    #[test]
    fn malformed_historical_failure_needs_exact_settlement_not_an_invented_run() {
        let job_id = uuid::Uuid::from_u128(41).to_string();
        let attempt_id = uuid::Uuid::from_u128(42).to_string();
        let finished = "2026-08-24T01:02:03.000004Z";
        let message = digest_bytes(b"owner-beta input rejected");
        let job = serde_json::json!({
            "id":job_id,"owner_user_id":uuid::Uuid::from_u128(43),
            "job_type":crate::owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            "payload_json":{},"status":"FAILED","attempt_count":1,
            "locked_by":null,"locked_at":null,"finished_at":finished,
            "error_code":"OWNER_BETA_INPUT_INVALID","error_message":message
        });
        let attempt = serde_json::json!({
            "id":attempt_id,"job_id":job_id,"attempt_no":1,"outcome":"FAILED",
            "claimed_by":"own-worker", "error_code":"OWNER_BETA_INPUT_INVALID","error_message":message,
            "finished_at":finished
        });
        let check = |job: &Value, attempt: Option<&Value>| {
            let mut domains = Domains::new(20, 1_048_576);
            let mut rows = vec![("jobs", job_id.clone(), job.clone())];
            if let Some(attempt) = attempt {
                rows.push(("job_attempts", attempt_id.clone(), attempt.clone()));
            }
            add_rows(&mut domains, &rows);
            domains.finish_recommendations()
        };
        assert_eq!(check(&job, Some(&attempt)), Ok(()));
        assert!(check(&job, None).is_err());
        let mut canceled = job.clone();
        canceled["status"] = serde_json::json!("CANCELED");
        canceled["locked_by"] = serde_json::json!("own-worker");
        canceled["locked_at"] = serde_json::json!("2026-08-24T01:02:01.000004Z");
        let mut canceled_attempt = attempt.clone();
        canceled_attempt["error_code"] = serde_json::json!("canceled");
        canceled_attempt["error_message"] =
            serde_json::json!(digest_bytes(b"job canceled by request"));
        canceled_attempt["claimed_by"] = serde_json::json!("own-worker");
        canceled_attempt["finished_at"] = serde_json::json!("2026-08-24T01:02:04.000004Z");
        assert_eq!(check(&canceled, Some(&canceled_attempt)), Ok(()));
        assert!(check(&canceled, None).is_err());
        assert!(check(&canceled, Some(&attempt)).is_err());
        for (key, value) in [
            ("error_code", serde_json::json!("CANCELED")),
            ("claimed_by", serde_json::json!("other-worker")),
            (
                "error_message",
                serde_json::json!(digest_bytes(b"different")),
            ),
            ("finished_at", Value::Null),
        ] {
            let mut changed = canceled_attempt.clone();
            changed[key] = value;
            assert!(check(&canceled, Some(&changed)).is_err());
        }
        let mut no_captured_claim = canceled.clone();
        no_captured_claim["locked_by"] = Value::Null;
        assert!(check(&no_captured_claim, Some(&canceled_attempt)).is_err());
        for (key, value) in [
            ("status", serde_json::json!("SUCCEEDED")),
            ("status", serde_json::json!("RUNNING")),
            ("error_code", serde_json::json!("OTHER_FAILURE")),
            ("attempt_count", serde_json::json!(2)),
            ("locked_by", serde_json::json!("still-owned")),
            ("finished_at", Value::Null),
        ] {
            let mut changed = job.clone();
            changed[key] = value;
            assert!(check(&changed, Some(&attempt)).is_err());
        }
        for (key, value) in [
            ("outcome", serde_json::json!("SUCCEEDED")),
            ("error_code", serde_json::json!("OTHER_FAILURE")),
            (
                "error_message",
                serde_json::json!(digest_bytes(b"different")),
            ),
            (
                "finished_at",
                serde_json::json!("2026-08-24T01:02:04.000004Z"),
            ),
        ] {
            let mut changed = attempt.clone();
            changed[key] = value;
            assert!(check(&job, Some(&changed)).is_err());
        }
        let strategy = crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
            &crate::resolver::ResolvedConfig {
                strategy_id: "buy_and_hold".into(),
                strategy_version: "1.0.0".into(),
                config: serde_json::json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
            },
        )
        .unwrap();
        let pin = format!("sha256:{}", "a".repeat(64));
        let payload = serde_json::json!({
            "run_id":uuid::Uuid::from_u128(44),"strategy_config_id":uuid::Uuid::from_u128(45),
            "as_of":"2026-08-24","strategy":strategy,
            "pins":{"candidate_content_sha256":pin,"artifact_manifest_sha256":pin,
                "stage5_manifest_sha256":pin,"action_manifest_sha256":pin,"approval_registry_sha256":pin}
        });
        assert!(
            serde_json::from_value::<crate::owner_beta::OwnerBetaPriceRecommendationInput>(
                payload.clone()
            )
            .is_ok()
        );
        let mut valid_but_orphaned = job;
        valid_but_orphaned["payload_json"] = payload.clone();
        assert!(check(&valid_but_orphaned, Some(&attempt)).is_err());
        canceled["payload_json"] = payload;
        assert!(check(&canceled, Some(&canceled_attempt)).is_err());
    }

    fn malformed_history_rows(canceled: bool, linked: bool) -> Vec<(&'static str, String, Value)> {
        let job = uuid::Uuid::from_u128(41).to_string();
        let attempt = uuid::Uuid::from_u128(42).to_string();
        let owner = uuid::Uuid::from_u128(43).to_string();
        let run = uuid::Uuid::from_u128(44).to_string();
        let config = uuid::Uuid::from_u128(45).to_string();
        let job_finished = "2026-08-24T01:02:03.000004Z";
        let attempt_finished = if canceled {
            "2026-08-24T01:02:04.000005Z"
        } else {
            job_finished
        };
        let status = if canceled { "CANCELED" } else { "FAILED" };
        let mut rows = vec![
            (
                "jobs",
                job.clone(),
                serde_json::json!({
                    "id":job,"owner_user_id":owner,
                    "job_type":crate::owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    // Deliberately neither a sealed payload nor a usable run ID.
                    "payload_json":{"run_id":"malformed"},"status":status,"attempt_count":1,
                    "locked_by":if canceled { Some("own-worker") } else { None },
                    "locked_at":if canceled { Some("2026-08-24T01:02:01.000003Z") } else { None },
                    "finished_at":job_finished,
                    "error_code":if canceled { None } else { Some("OWNER_BETA_INPUT_INVALID") },
                    "error_message":if canceled { None } else { Some(digest_bytes(b"owner-beta input rejected")) }
                }),
            ),
            (
                "job_attempts",
                attempt.clone(),
                serde_json::json!({
                    "id":attempt,"job_id":job,"attempt_no":1,"claimed_by":"own-worker",
                    "outcome":"FAILED","finished_at":attempt_finished,
                    "error_code":if canceled { "canceled" } else { "OWNER_BETA_INPUT_INVALID" },
                    "error_message":digest_bytes(if canceled { b"job canceled by request" } else { b"owner-beta input rejected" })
                }),
            ),
        ];
        if linked {
            let strategy = crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
                &crate::resolver::ResolvedConfig {
                    strategy_id: "buy_and_hold".into(), strategy_version: "1.0.0".into(),
                    config: serde_json::json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
                },
            ).unwrap();
            let pin = format!("sha256:{}", "a".repeat(64));
            rows.push(("owner_beta_recommendation_runs", run.clone(), serde_json::json!({
                "id":run,"job_id":job,"owner_user_id":owner,"strategy_config_id":config,
                "status":status,"error_code":if canceled { "CANCELED" } else { "OWNER_BETA_INPUT_INVALID" },
                "input_kind":"owner_beta_historical_price_only_v1","trigger_kind":"MANUAL","daily_pins":null,
                "as_of":"2026-08-24","strategy_id":strategy.strategy_id(),"strategy_version":strategy.strategy_version(),
                "strategy_config_json":strategy.config_json(),"strategy_config_sha256":strategy.config_sha256(),
                "capability":"PRICE_RETURN_ONLY","audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,
                "candidate_content_sha256":pin,"artifact_manifest_sha256":pin,"stage5_manifest_sha256":pin,
                "action_manifest_sha256":pin,"approval_registry_sha256":pin,
                "factor_snapshot_sha256":null,"target_snapshot_sha256":null,"cash_weight":null,
                // COALESCE preserves a prior start, but finish/update share the
                // queue settlement transaction's time, not the cancel request.
                "started_at":"2026-08-24T01:02:02.000001Z",
                "finished_at":attempt_finished,"updated_at":attempt_finished
            })));
            rows.push((
                "user_strategy_configs",
                config.clone(),
                serde_json::json!({"id":config,"owner_user_id":owner}),
            ));
        }
        rows
    }

    fn consume_malformed_history(rows: &[(&str, String, Value)]) -> Domains {
        // Descriptive SQL rows and existing file fixtures only. No approved
        // artifact/admitted object or material success is fabricated here.
        let mut domains = consume_release(&release_fixture());
        domains.observe(ArchiveOriginalMember::Member {path:"paper_operations",canonical_record:br#"{"gid":0,"kind":"directory","mode":448,"path":"paper_operations","sha256":null,"size":0,"uid":0}"#,file:None}).unwrap();
        add_rows(&mut domains, rows);
        domains
    }

    #[test]
    fn malformed_historical_terminal_settlements_traverse_and_finish_all_four_paths() {
        for canceled in [false, true] {
            for linked in [false, true] {
                let rows = malformed_history_rows(canceled, linked);
                let domains = consume_malformed_history(&rows);
                assert!(
                    domains
                        .malformed_historical_job_settled(&rows[0].2)
                        .unwrap()
                );
                if linked {
                    // This is the exact entry used by recompute, before parsing
                    // payload/run_id or consuming any genuine borrowed material.
                    assert!(
                        domains
                            .recommendation_for_recomputation(&rows[2].2)
                            .unwrap()
                            .is_none()
                    );
                }
                assert_eq!(domains.finish(&Value::Null, &Value::Null), Ok(()));
                assert!(domains.recommendations.is_empty());
                let mut missing_attempt = rows.clone();
                missing_attempt.remove(1);
                assert!(
                    consume_malformed_history(&missing_attempt)
                        .finish(&Value::Null, &Value::Null)
                        .is_err()
                );
                let mut published = rows.clone();
                published.push((
                    "owner_beta_daily_publication_claims",
                    "claim".into(),
                    serde_json::json!({"job_id":rows[0].2["id"]}),
                ));
                assert!(
                    consume_malformed_history(&published)
                        .finish(&Value::Null, &Value::Null)
                        .is_err()
                );
                // Independent request and attempt times, plus preserved job
                // errors, are valid only for the cancellation race.
                if canceled {
                    let mut preserved = rows.clone();
                    preserved[0].2["error_code"] = serde_json::json!("EARLIER_ERROR");
                    preserved[0].2["error_message"] =
                        serde_json::json!(digest_bytes(b"earlier error"));
                    assert_eq!(
                        consume_malformed_history(&preserved).finish(&Value::Null, &Value::Null),
                        Ok(())
                    );
                }
            }
        }
    }

    #[test]
    fn malformed_historical_mirrors_reject_corrupt_bindings_and_publication() {
        for canceled in [false, true] {
            let rows = malformed_history_rows(canceled, true);
            let other = serde_json::json!(uuid::Uuid::from_u128(99));
            for (index, key, value) in [
                (0, "job_type", serde_json::json!("recommendation")),
                (0, "owner_user_id", other.clone()),
                (0, "status", serde_json::json!("SUCCEEDED")),
                (0, "attempt_count", serde_json::json!(2)),
                (1, "job_id", other.clone()),
                (1, "outcome", serde_json::json!("SUCCEEDED")),
                (
                    1,
                    "error_message",
                    serde_json::json!(digest_bytes(b"different")),
                ),
                (1, "claimed_by", Value::Null),
                (2, "job_id", other.clone()),
                (2, "owner_user_id", other.clone()),
                (2, "status", serde_json::json!("PENDING")),
                (2, "status", serde_json::json!("SUCCEEDED")),
                (2, "error_code", serde_json::json!("OTHER_FAILURE")),
                (2, "trigger_kind", serde_json::json!("DAILY")),
                (
                    2,
                    "input_kind",
                    serde_json::json!("owner_beta_daily_etf11_price_only_v1"),
                ),
                (2, "started_at", Value::Null),
                (2, "capability", serde_json::json!("OTHER")),
                (2, "audience", serde_json::json!("PUBLIC")),
                (2, "vendor_snapshot", serde_json::json!(false)),
                (2, "strict_pit", serde_json::json!(true)),
                (
                    2,
                    "finished_at",
                    serde_json::json!("2026-08-24T01:02:05.000001Z"),
                ),
                (2, "updated_at", Value::Null),
                (
                    2,
                    "factor_snapshot_sha256",
                    serde_json::json!(digest_bytes(b"factor")),
                ),
                (
                    2,
                    "target_snapshot_sha256",
                    serde_json::json!(digest_bytes(b"target")),
                ),
                (2, "cash_weight", serde_json::json!(0)),
                (3, "owner_user_id", other),
            ] {
                let mut changed = rows.clone();
                changed[index].2[key] = value;
                let domains = consume_malformed_history(&changed);
                assert!(
                    domains
                        .recommendation_for_recomputation(&changed[2].2)
                        .is_err(),
                    "{canceled}/{index}/{key}"
                );
                assert!(
                    domains.finish(&Value::Null, &Value::Null).is_err(),
                    "{canceled}/{index}/{key}"
                );
                assert!(domains.recommendations.is_empty());
            }
            for missing in [0, 1, 3] {
                let mut changed = rows.clone();
                changed.remove(missing);
                let domains = consume_malformed_history(&changed);
                assert!(
                    domains
                        .recommendation_for_recomputation(&rows[2].2)
                        .is_err()
                );
                assert!(domains.finish(&Value::Null, &Value::Null).is_err());
            }
            for (table, field) in [
                ("owner_beta_recommendation_items", "recommendation_run_id"),
                ("owner_beta_paper_lineage", "recommendation_run_id"),
                ("owner_beta_daily_reservations", "run_id"),
                ("owner_beta_daily_publication_claims", "run_id"),
                ("owner_beta_paper_source_runs", "run_id"),
            ] {
                let mut changed = rows.clone();
                let mut publication =
                    serde_json::json!({"owner_user_id":rows[2].2["owner_user_id"]});
                publication[field] = rows[2].2["id"].clone();
                changed.push((table, uuid::Uuid::from_u128(90).to_string(), publication));
                let domains = consume_malformed_history(&changed);
                assert!(
                    domains
                        .recommendation_for_recomputation(&rows[2].2)
                        .is_err(),
                    "{table}"
                );
                assert!(
                    domains.finish(&Value::Null, &Value::Null).is_err(),
                    "{table}"
                );
            }
            // Neither two final attempts nor two optional mirrors represent the
            // single producer transaction, even if their contents agree.
            for duplicate in [1, 2] {
                let mut changed = rows.clone();
                let mut extra = rows[duplicate].clone();
                extra.1 = uuid::Uuid::from_u128(98).to_string();
                extra.2["id"] = serde_json::json!(extra.1);
                changed.push(extra);
                let domains = consume_malformed_history(&changed);
                assert!(
                    domains
                        .recommendation_for_recomputation(&rows[2].2)
                        .is_err()
                );
                assert!(domains.finish(&Value::Null, &Value::Null).is_err());
            }
        }
    }

    #[test]
    fn malformed_history_exception_never_covers_valid_inputs_or_missing_success() {
        let mut rows = malformed_history_rows(false, true);
        let run = rows[2].2.clone();
        let strategy = crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
            &crate::resolver::ResolvedConfig {
                strategy_id: "buy_and_hold".into(),
                strategy_version: "1.0.0".into(),
                config: run["strategy_config_json"].clone(),
            },
        )
        .unwrap();
        rows[0].2["payload_json"] = serde_json::json!({
            "run_id":run["id"],"strategy_config_id":run["strategy_config_id"],
            "as_of":run["as_of"],"strategy":strategy,
            "pins":{"candidate_content_sha256":run["candidate_content_sha256"],
                "artifact_manifest_sha256":run["artifact_manifest_sha256"],
                "stage5_manifest_sha256":run["stage5_manifest_sha256"],
                "action_manifest_sha256":run["action_manifest_sha256"],
                "approval_registry_sha256":run["approval_registry_sha256"]}
        });
        for succeeded in [false, true] {
            if succeeded {
                rows[0].2["status"] = serde_json::json!("SUCCEEDED");
                rows[2].2["status"] = serde_json::json!("SUCCEEDED");
            }
            let domains = consume_malformed_history(&rows);
            assert!(
                !domains
                    .malformed_historical_job_settled(&rows[0].2)
                    .unwrap()
            );
            assert!(
                domains
                    .recommendation_for_recomputation(&rows[2].2)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                domains.finish_recommendations(),
                Err(SourceClosureError::RecommendationMaterialMissing)
            );
            assert!(domains.finish(&Value::Null, &Value::Null).is_err());
            assert!(domains.recommendations.is_empty());
            let mut orphan = rows.clone();
            orphan.remove(2);
            assert!(
                consume_malformed_history(&orphan)
                    .finish(&Value::Null, &Value::Null)
                    .is_err()
            );
        }
    }

    #[test]
    fn malformed_history_obeys_original_deadline_and_shared_row_budget() {
        let rows = malformed_history_rows(false, true);
        let mut domains = consume_malformed_history(&rows);
        domains.deadline = Some(std::time::Instant::now());
        assert!(matches!(
            domains.recommendation_for_recomputation(&rows[2].2),
            Err(SourceClosureError::Interrupted)
        ));
        assert_eq!(
            domains.finish(&Value::Null, &Value::Null),
            Err(SourceClosureError::Interrupted)
        );
        let mut domains = consume_malformed_history(&rows);
        domains.max_bytes = domains.used_bytes;
        assert_eq!(
            domains.finish_recommendations(),
            Err(SourceClosureError::Limit)
        );
        let mut bounded = Domains::new(0, 1_048_576);
        let claim = serde_json::to_vec(&serde_json::json!({"job_id":rows[0].2["id"]})).unwrap();
        assert_eq!(
            bounded.observe(ArchiveOriginalMember::Reference {
                table: "owner_beta_daily_publication_claims",
                key: "claim",
                canonical_row: &claim,
            }),
            Err(SourceClosureError::Limit)
        );
    }

    #[test]
    fn valid_daily_terminal_input_keeps_its_original_recomputation_path() {
        let strategy = crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
            &crate::resolver::ResolvedConfig {
                strategy_id: "buy_and_hold".into(),
                strategy_version: "1.0.0".into(),
                config: serde_json::json!({}),
            },
        )
        .unwrap();
        let pin = format!("sha256:{}", "a".repeat(64));
        let payload = serde_json::json!({
            "schema":"owner-beta-daily-recommendation-v1",
            "run_id":uuid::Uuid::from_u128(71),"strategy_config_id":uuid::Uuid::from_u128(72),
            "as_of":"2026-08-31","strategy":strategy,
            "input":{"kind":"owner_beta_daily_etf11_price_only_v1",
                "content_sha256":pin,"manifest_sha256":pin,"admission_sha256":pin,"policy_sha256":pin},
            "trigger":{"kind":"DAILY","version":"owner-beta-daily-trigger-v1",
                "config_epoch":0,"activation_epoch":0}
        });
        assert!(matches!(
            serde_json::from_value::<crate::owner_beta::daily_input::OwnerBetaExecutionInput>(
                payload.clone()
            ),
            Ok(crate::owner_beta::daily_input::OwnerBetaExecutionInput::Daily(_))
        ));
        // This is an untrusted decoded job claim, never admitted material.
        for canceled in [false, true] {
            let mut rows = malformed_history_rows(canceled, true);
            rows[0].2["payload_json"] = payload.clone();
            let domains = consume_malformed_history(&rows);
            assert!(
                !domains
                    .malformed_historical_job_settled(&rows[0].2)
                    .unwrap()
            );
        }
    }

    #[test]
    fn recommendation_projection_checks_real_target_builder_output_and_every_item() {
        // Descriptive factor/target constructors test publication comparison.
        // No ApprovedHistoricalPriceOnlyArtifact or admitted authority is fabricated.
        use crate::owner_beta::{
            OwnerBetaPriceRecommendationInput, OwnerBetaStrategySnapshot,
            target::build_target_snapshot,
        };
        use factor_engine::{PriceOnlyFactorSnapshot, snapshot::NormalizationMeta};
        let strategy = OwnerBetaStrategySnapshot::from_resolved_config(&crate::resolver::ResolvedConfig {
            strategy_id:"buy_and_hold".into(), strategy_version:"1.0.0".into(),
            config:serde_json::json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
        }).unwrap();
        let pin = format!("sha256:{}", "a".repeat(64));
        let input: OwnerBetaPriceRecommendationInput = serde_json::from_value(serde_json::json!({
            "run_id":uuid::Uuid::from_u128(1),"strategy_config_id":uuid::Uuid::from_u128(2),"as_of":"2026-08-24", "strategy":strategy,
            "pins":{"candidate_content_sha256":pin,"artifact_manifest_sha256":pin,"stage5_manifest_sha256":pin,"action_manifest_sha256":pin,"approval_registry_sha256":pin}
        })).unwrap();
        let mut factor = PriceOnlyFactorSnapshot {
            input_kind: "owner_beta_historical_price_only_v1".into(),
            capability: "PRICE_RETURN_ONLY".into(),
            as_of: input.as_of(),
            candidate_content_sha256: pin.clone(),
            artifact_manifest_sha256: pin.clone(),
            stage5_manifest_sha256: pin.clone(),
            action_manifest_sha256: pin.clone(),
            approval_registry_sha256: pin,
            factor_versions: BTreeMap::new(),
            normalization: NormalizationMeta {
                id: "test".into(),
                version: "1.0.0".into(),
                params: BTreeMap::new(),
            },
            rows: Vec::new(),
            hash: domain::ContentHash::from_bytes(b"placeholder"),
        };
        factor.hash = factor.compute_hash().unwrap();
        let target = build_target_snapshot(&input, &factor).unwrap();
        let owner = uuid::Uuid::from_u128(3).to_string();
        let job = uuid::Uuid::from_u128(4).to_string();
        let run = serde_json::json!({"id":input.run_id(),"owner_user_id":owner,"job_id":job,"factor_snapshot_sha256":factor.hash,"target_snapshot_sha256":target.target_snapshot_sha256(),"cash_weight":0});
        let mut rows = Vec::new();
        for (n, item) in target.items().iter().enumerate() {
            let weight: Value = item
                .target_weight()
                .map(|w| serde_json::from_str(&w).unwrap())
                .unwrap_or(Value::Null);
            let excluded = item.target_weight_ppm().is_none();
            let exclusion = if excluded {
                item.reasons().first().map(|r| r.code())
            } else {
                None
            };
            rows.push(("owner_beta_recommendation_items",uuid::Uuid::from_u128(100+n as u128).to_string(),serde_json::json!({
                "recommendation_run_id":input.run_id(),"owner_user_id":owner,"instrument_id":item.instrument_id(),"rank":item.rank(),"target_weight":weight,
                "reason_codes":item.reasons().iter().map(|r|r.code()).collect::<Vec<_>>(),"factors_json":item.factors(),"excluded":excluded,"exclusion_reason":exclusion
            })));
        }
        rows.push((
            "job_attempts",
            uuid::Uuid::from_u128(9).to_string(),
            serde_json::json!({"job_id":job,"outcome":"SUCCEEDED"}),
        ));
        let compare = |rows: &[(&str, String, Value)], run: &Value| {
            let mut d = Domains::new(100, 1_048_576);
            add_rows(&mut d, rows);
            d.compare_recommendation_output(
                run,
                factor.hash.as_str(),
                target.target_snapshot_sha256().as_str(),
                &target.cash_weight(),
                target.items(),
                false,
            )
        };
        assert_eq!(compare(&rows, &run), Ok(()));
        for mutation in 0..7 {
            let mut changed = rows.clone();
            let mut changed_run = run.clone();
            match mutation {
                0 => {
                    changed.remove(0);
                }
                1 => changed[0].2["target_weight"] = serde_json::json!(0.5),
                2 => changed[0].2["reason_codes"] = serde_json::json!([]),
                3 => changed[0].2["factors_json"] = serde_json::json!({"unknown":"1"}),
                4 => changed[0].2["owner_user_id"] = serde_json::json!(uuid::Uuid::from_u128(99)),
                5 => {
                    changed_run["target_snapshot_sha256"] =
                        serde_json::json!(format!("sha256:{}", "f".repeat(64)))
                }
                _ => changed.last_mut().unwrap().2["outcome"] = serde_json::json!("FAILED"),
            }
            assert_eq!(
                compare(&changed, &changed_run),
                Err(SourceClosureError::RecommendationOutput)
            );
        }
    }
    #[test]
    fn unrelated_generic_jobs_do_not_require_beta_material() {
        let mut d = Domains::new(10, 1_048_576);
        add_rows(
            &mut d,
            &[(
                "jobs",
                "generic".into(),
                serde_json::json!({"job_type":"recommendation","payload_json":{"dataset_version":"arbitrary"}}),
            )],
        );
        assert_eq!(d.finish_recommendations(), Ok(()));
    }

    fn fixtures() -> Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
        )))
        .unwrap()
    }
    fn object<'a>(f: &'a Value, name: &str) -> &'a str {
        f["objects"][name]["canonical"].as_str().unwrap()
    }
    #[test]
    fn frozen_restore_stage_chain_accepts_and_rejects_wrong_link() {
        let f = fixtures();
        let root = wire::parse_root_intent(object(&f, "restore.root").as_bytes()).unwrap();
        let mut chain = StageChain::new(&root, 11).unwrap();
        for n in 0..=5 {
            chain
                .observe(object(&f, &format!("restore.stage{n}")).as_bytes())
                .unwrap();
        }
        let mut chain = StageChain::new(&root, 11).unwrap();
        chain
            .observe(object(&f, "restore.stage0").as_bytes())
            .unwrap();
        assert!(
            chain
                .observe(object(&f, "restore.stage2").as_bytes())
                .is_err()
        );
    }
    #[test]
    fn frozen_backup_snapshot_is_settled_history() {
        let f = fixtures();
        let root = wire::parse_root_intent(object(&f, "backup.root").as_bytes()).unwrap();
        let mut chain = StageChain::new(&root, 2).unwrap();
        for n in 0..=5 {
            chain
                .observe(object(&f, &format!("backup.stage{n}")).as_bytes())
                .unwrap();
        }
    }
    #[test]
    fn frozen_stage_rejects_epoch_and_identity_mutations() {
        let f = fixtures();
        let root = wire::parse_root_intent(object(&f, "restore.root").as_bytes()).unwrap();
        for (key, value) in [
            ("control_epoch", serde_json::json!(1)),
            (
                "account_id",
                serde_json::json!("00000000-0000-4000-8000-000000000099"),
            ),
        ] {
            let mut stage: Value = serde_json::from_str(object(&f, "restore.stage0")).unwrap();
            stage[key] = value;
            let bytes = crate::owner_beta::paper::source_evidence::canonical(&stage).unwrap();
            assert!(StageChain::new(&root, 11).unwrap().observe(&bytes).is_err());
        }
    }
    #[test]
    fn frozen_actions_require_order_and_exact_dispatch_binding() {
        let f = fixtures();
        let root = wire::parse_root_intent(object(&f, "stop.root").as_bytes()).unwrap();
        let mut chain = ActionChain::new(&root);
        chain
            .observe(
                object(&f, "stop.action0").as_bytes(),
                Some(object(&f, "stop.action0.dispatch").as_bytes()),
            )
            .unwrap();
        assert!(
            chain
                .observe(object(&f, "stop.action2").as_bytes(), None)
                .is_err()
        );
        assert!(
            chain
                .observe(
                    object(&f, "stop.action1").as_bytes(),
                    Some(object(&f, "stop.action2.dispatch").as_bytes())
                )
                .is_err()
        );
        chain
            .observe(
                object(&f, "stop.action1").as_bytes(),
                Some(object(&f, "stop.action1.dispatch").as_bytes()),
            )
            .unwrap();
    }
    fn additional_receipts(f: &Value) -> (Value, Value, Vec<u8>, Vec<u8>) {
        let summary: Value = serde_json::from_str(object(f, "backup.summary1")).unwrap();
        let q1 = object(f, "backup.action1.sql");
        let q2 = object(f, "backup.action2.sql");
        let request1: Value = serde_json::from_str(q1).unwrap();
        let request2: Value = serde_json::from_str(q2).unwrap();
        let summary_bytes = crate::owner_beta::paper::source_evidence::canonical(&summary).unwrap();
        let summary_hash = digest_bytes(&summary_bytes);
        let result1 = serde_json::json!({"schema":"owner-beta-paper-action-v1","action":"RECONCILE_OPERATION","outcome":"OK","control_epoch":1,"account_id":summary["account_id"],"data":{"observation_id":summary["observation_id"],"summary":summary,"summary_sha256":summary_hash}});
        let b1 = crate::owner_beta::paper::source_evidence::canonical(&result1).unwrap();
        let b2 = object(f, "backup.result").as_bytes().to_vec();
        let event_result: Value = serde_json::from_slice(&b2).unwrap();
        let obs = serde_json::json!({"generation":summary["generation"],"observation_id":summary["observation_id"],"summary":summary,"summary_sha256":summary_hash,"request_key":request1["args"]["request_key"],"request_canonical":q1,"request_sha256":digest_bytes(q1.as_bytes()),"result_canonical":std::str::from_utf8(&b1).unwrap(),"result_sha256":digest_bytes(&b1)});
        let event = serde_json::json!({"account_id":event_result["account_id"],"id":event_result["data"]["event_id"],"control_epoch":2,"request_key":request2["args"]["request_key"],"request_canonical":q2,"request_sha256":digest_bytes(q2.as_bytes()),"result_canonical":std::str::from_utf8(&b2).unwrap(),"result_sha256":digest_bytes(&b2)});
        (obs, event, b1, b2)
    }
    fn journal_inputs(
        legacy: bool,
    ) -> (std::collections::BTreeMap<String, Option<Vec<u8>>>, Value) {
        use std::collections::BTreeMap;
        let f = fixtures();
        let mut entries = BTreeMap::<String, Option<Vec<u8>>>::new();
        let base = "paper_operations";
        let generation = "00000000-0000-4000-8000-000000000020";
        let dir = format!("{base}/operations/{generation}");
        for p in [base.to_owned(), format!("{base}/operations"), dir.clone()] {
            entries.insert(p, None);
        }
        entries.insert(
            format!("{base}/installation-id"),
            Some(b"00000000-0000-4000-8000-000000000002\n".to_vec()),
        );
        entries.insert(format!("{base}/epoch-floor"), Some(b"2\n".to_vec()));
        let mut root: Value = serde_json::from_str(object(&f, "backup.root")).unwrap();
        if legacy {
            root["request_key"] = serde_json::json!(format!("{generation}:off"));
        }
        let root_bytes = crate::owner_beta::paper::source_evidence::canonical(&root).unwrap();
        let root_hash = digest_bytes(&root_bytes);
        entries.insert(format!("{dir}/intent.json"), Some(root_bytes.clone()));
        let count = if legacy { 1 } else { 6 };
        for n in 0..count {
            let mut stage: Value =
                serde_json::from_str(object(&f, &format!("backup.stage{n}"))).unwrap();
            if legacy {
                stage["intent_sha256"] = serde_json::json!(root_hash);
                stage["control_epoch"] = serde_json::json!(1);
            }
            let bytes = crate::owner_beta::paper::source_evidence::canonical(&stage).unwrap();
            entries.insert(format!("{dir}/{n:08}.json"), Some(bytes.clone()));
            entries.insert(
                format!("{dir}/{}.json", &digest_bytes(&bytes)[7..]),
                Some(bytes),
            );
        }
        let mut row = root.as_object().unwrap().clone();
        row.insert("intent".into(), root.clone());
        row.insert("intent_sha256".into(), serde_json::json!(root_hash));
        row.insert(
            "fence_event_id".into(),
            serde_json::json!("00000000-0000-4000-8000-000000000120"),
        );
        row.insert("fence_epoch".into(), serde_json::json!(1));
        for key in [
            "request_canonical",
            "request_sha256",
            "result_canonical",
            "result_sha256",
        ] {
            row.insert(key.into(), Value::Null);
        }
        if !legacy {
            let callers = format!("{base}/callers");
            entries.insert(callers.clone(), None);
            let caller = object(&f, "backup.caller").as_bytes();
            let c: Value = serde_json::from_slice(caller).unwrap();
            let projection = serde_json::json!({"installation_id":c["installation_id"],"account_id":c["account_id"],"caller_key":c["caller_key"]});
            let hash = digest_bytes(
                &crate::owner_beta::paper::source_evidence::canonical(&projection).unwrap(),
            );
            entries.insert(
                format!("{callers}/{}.json", &hash[7..]),
                Some(caller.to_vec()),
            );
            let actions = format!("{dir}/actions");
            entries.insert(actions.clone(), None);
            for n in 0..3 {
                let path = format!("{actions}/{n:08}");
                let output = format!("{path}/output");
                entries.insert(path.clone(), None);
                entries.insert(output.clone(), None);
                let binding = object(&f, &format!("backup.action{n}")).as_bytes();
                let dispatch = object(&f, &format!("backup.action{n}.dispatch")).as_bytes();
                let d: Value = serde_json::from_slice(dispatch).unwrap();
                entries.insert(format!("{path}/binding.json"), Some(binding.to_vec()));
                entries.insert(format!("{output}/dispatch.json"), Some(dispatch.to_vec()));
                for key in [
                    "root_intent_canonical",
                    "action_canonical",
                    "artifact_canonical",
                    "sql_request_canonical",
                ] {
                    if let Some(v) = d[key].as_str() {
                        let b = v.as_bytes();
                        entries.insert(
                            format!("{output}/{}.json", &digest_bytes(b)[7..]),
                            Some(b.to_vec()),
                        );
                        if key == "artifact_canonical" {
                            entries.insert(format!("{path}/evidence.json"), Some(b.to_vec()));
                        }
                    }
                }
                if n == 1 || n == 2 {
                    let (_, _, result1, result2) = additional_receipts(&f);
                    let bytes = if n == 1 { result1 } else { result2 };
                    entries.insert(
                        format!("{output}/{}.json", &digest_bytes(&bytes)[7..]),
                        Some(bytes),
                    );
                }
                if n == 0 {
                    let q = d["sql_request_canonical"].as_str().unwrap();
                    let result = object(&f, "backup.begin-result");
                    entries.insert(
                        format!("{output}/{}.json", &digest_bytes(result.as_bytes())[7..]),
                        Some(result.as_bytes().to_vec()),
                    );
                    row.insert("request_canonical".into(), serde_json::json!(q));
                    row.insert(
                        "request_sha256".into(),
                        serde_json::json!(digest_bytes(q.as_bytes())),
                    );
                    row.insert("result_canonical".into(), serde_json::json!(result));
                    row.insert(
                        "result_sha256".into(),
                        serde_json::json!(digest_bytes(result.as_bytes())),
                    );
                }
            }
        }
        (entries, Value::Object(row))
    }
    fn consume_inputs(
        entries: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
        row: &Value,
    ) -> Domains {
        use std::io::Write;
        let mut domains = Domains::new(1000, 8 * 1024 * 1024);
        for (path, body) in entries {
            let mode = if body.is_none() {
                0o700
            } else if path.ends_with("/installation-id") || path.ends_with("/epoch-floor") {
                0o600
            } else {
                0o400
            };
            let record = serde_json::to_vec(&serde_json::json!({"path":path,"kind":if body.is_some(){"file"}else{"directory"},"uid":0,"gid":0,"mode":mode,"size":body.as_ref().map_or(0,Vec::len),"sha256":body.as_ref().map(|b| digest_bytes(b))})).unwrap();
            if let Some(bytes) = body {
                let mut fd = tempfile::tempfile().unwrap();
                fd.write_all(bytes).unwrap();
                domains
                    .observe(ArchiveOriginalMember::Member {
                        path,
                        canonical_record: &record,
                        file: Some(&fd),
                    })
                    .unwrap();
            } else {
                domains
                    .observe(ArchiveOriginalMember::Member {
                        path,
                        canonical_record: &record,
                        file: None,
                    })
                    .unwrap();
            }
        }
        let bytes = serde_json::to_vec(row).unwrap();
        domains
            .observe(ArchiveOriginalMember::Database {
                group: "paper",
                table: RECEIPT_TABLES[0],
                key: "generation",
                canonical_row: &bytes,
            })
            .unwrap();
        domains
            .observe(ArchiveOriginalMember::Reference {
                table: RECEIPT_TABLES[0],
                key: "generation",
                canonical_row: &bytes,
            })
            .unwrap();
        if !row["request_canonical"].is_null() {
            let (observation, event, _, _) = additional_receipts(&fixtures());
            for (table, row) in [(RECEIPT_TABLES[1], observation), (RECEIPT_TABLES[2], event)] {
                let bytes = serde_json::to_vec(&row).unwrap();
                domains
                    .observe(ArchiveOriginalMember::Reference {
                        table,
                        key: "other",
                        canonical_row: &bytes,
                    })
                    .unwrap();
            }
        }
        domains
    }
    #[test]
    fn callback_collector_accepts_complete_and_legacy_rejects_missing_original() {
        for legacy in [false, true] {
            let (mut entries, row) = journal_inputs(legacy);
            let domains = consume_inputs(&entries, &row);
            assert!(domains.finish_journal(&Value::Null, &Value::Null).is_ok());
            if !legacy {
                let mut changed = row.clone();
                changed["fence_epoch"] = serde_json::json!(42);
                assert!(
                    consume_inputs(&entries, &changed)
                        .finish_journal(&Value::Null, &Value::Null)
                        .is_err()
                );
                let mut incomplete = entries.clone();
                let intent_copy = incomplete
                    .keys()
                    .find(|p| {
                        p.contains("/operations/")
                            && p.ends_with(&format!(
                                "{}.json",
                                &row["intent_sha256"].as_str().unwrap()[7..]
                            ))
                    })
                    .unwrap()
                    .clone();
                incomplete.remove(&intent_copy);
                assert!(
                    consume_inputs(&incomplete, &row)
                        .finish_journal(&Value::Null, &Value::Null)
                        .is_err()
                );
                let mut bad_action = entries.clone();
                let binding = bad_action
                    .keys()
                    .find(|p| p.ends_with("/actions/00000001/binding.json"))
                    .unwrap()
                    .clone();
                *bad_action.get_mut(&binding).unwrap() = Some(b"{}".to_vec());
                assert!(
                    consume_inputs(&bad_action, &row)
                        .finish_journal(&Value::Null, &Value::Null)
                        .is_err()
                );
                let result = row["result_sha256"].as_str().unwrap();
                let missing = entries
                    .keys()
                    .find(|p| p.ends_with(&format!("{}.json", &result[7..])))
                    .unwrap()
                    .clone();
                entries.remove(&missing);
                assert!(
                    consume_inputs(&entries, &row)
                        .finish_journal(&Value::Null, &Value::Null)
                        .is_err()
                );
            }
        }
    }
    #[test]
    fn callback_budget_and_changed_duplicate_fail() {
        let mut d = Domains::new(1, 1024);
        d.observe(ArchiveOriginalMember::Database {
            group: "paper",
            table: RECEIPT_TABLES[0],
            key: "a",
            canonical_row: br#"{}"#,
        })
        .unwrap();
        assert!(
            d.observe(ArchiveOriginalMember::Reference {
                table: RECEIPT_TABLES[0],
                key: "a",
                canonical_row: br#"{"x":1}"#
            })
            .is_err()
        );
        assert!(
            d.observe(ArchiveOriginalMember::Reference {
                table: RECEIPT_TABLES[0],
                key: "b",
                canonical_row: br#"{}"#
            })
            .is_err()
        );
    }
    fn release_fixture() -> std::collections::BTreeMap<String, Option<Vec<u8>>> {
        let mut files = std::collections::BTreeMap::new();
        for path in [
            "paper_config",
            "paper_contract",
            "paper_manifest",
            "paper_reader",
            "paper_reader/provenance",
            "paper_reader/provenance/api-server",
            "paper_reader/provenance/paper-runner",
            "paper_reader/provenance/owner-beta-paper-ops-native",
        ] {
            files.insert(path.to_owned(), None);
        }
        let commit = "a".repeat(40);
        let mut manifest = format!("LAGRANGE_RELEASE_MANIFEST_V3\ncommit|{commit}\n");
        for name in RELEASE_SERVICES {
            manifest.push_str(&format!(
                "image|{name}|lagrange-station-{name}:{commit}|sha256:{}|{commit}\n",
                "b".repeat(64)
            ));
        }
        files.insert(
            "paper_manifest/.lagrange-release-manifest".into(),
            Some(manifest.as_bytes().to_vec()),
        );
        for name in [
            "financial-contract.md",
            "audit-projection.json",
            "operations.schema.json",
            "archive.schema.json",
            "migration-0068.up.sql",
            "audit.sql",
        ] {
            files.insert(
                format!("paper_contract/{name}"),
                Some(format!("fixture-{name}").into_bytes()),
            );
        }
        files.insert(
            "paper_contract/financial-contract.md".into(),
            Some(
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/contracts/owner-beta-paper-v1.md"
                ))
                .to_vec(),
            ),
        );
        files.insert(
            "paper_reader/migration-0069.up.sql".into(),
            Some(b"fixture-0069".to_vec()),
        );
        let mut elf = vec![0u8; 20];
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[18] = 62;
        for name in ["api-image.elf", "worker-image.elf", "native-image.elf"] {
            files.insert(format!("paper_reader/{name}"), Some(elf.clone()));
        }
        let projection_hash = digest_bytes(
            files["paper_contract/audit-projection.json"]
                .as_ref()
                .unwrap(),
        );
        let h = |path: &str| digest_bytes(files[path].as_ref().unwrap());
        let cap = serde_json::json!({"schema":"owner-beta-paper-reader-capability-v1", "minimum":1, "code_commit":commit, "release_manifest_sha256":digest_bytes(manifest.as_bytes()), "operations_schema_sha256":h("paper_contract/operations.schema.json"), "archive_schema_sha256":h("paper_contract/archive.schema.json"), "migration_0068_sha256":h("paper_contract/migration-0068.up.sql"), "audit_sql_sha256":h("paper_contract/audit.sql"), "api_binary_sha256":digest_bytes(&elf), "worker_binary_sha256":digest_bytes(&elf), "native_binary_sha256":digest_bytes(&elf)});
        let process = serde_json::json!({"schema":"owner-beta-paper-process-v1", "owner_user_id":"00000000-0000-4000-8000-000000000011", "account_id":"00000000-0000-4000-8000-000000000012", "strategy_config_id":"00000000-0000-4000-8000-000000000013", "consent_event_id":"00000000-0000-4000-8000-000000000014", "consent_version":"owner-beta-paper-v1-draft-1", "control_epoch":1, "daily_cohort_sha256":format!("sha256:{}", "c".repeat(64)), "financial_contract_sha256":h("paper_contract/financial-contract.md"), "code_commit":commit, "release_manifest_sha256":cap["release_manifest_sha256"], "api_binary_sha256":cap["api_binary_sha256"], "worker_binary_sha256":cap["worker_binary_sha256"]});
        let cap_bytes = crate::owner_beta::paper::source_evidence::canonical(&cap).unwrap();
        let process_bytes = crate::owner_beta::paper::source_evidence::canonical(&process).unwrap();
        files.insert(
            "paper_reader/capability.json".into(),
            Some(cap_bytes.clone()),
        );
        files.insert(
            "paper_config/process.json".into(),
            Some(process_bytes.clone()),
        );
        files.insert(
            "paper_config/config.json".into(),
            Some(process_bytes.clone()),
        );
        let mut hashes = serde_json::Map::new();
        for (name, _) in RELEASE_PRODUCERS {
            let hex = "d".repeat(64);
            let artifact = serde_json::json!({"format":"lagrange-rust-artifact-v1", "source_commit":commit, "package":"api-server", "bin":name, "platform":"linux/amd64", "host_triple":"x86_64-unknown-linux-musl", "profile":"release", "features":[], "compile_env":{"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>","LAGRANGE_CODE_COMMIT":commit}, "cargo_success":true, "binary_mode":"0755", "binary_sha256":&digest_bytes(&elf)[7..], "cache_key":hex, "h_sha256":hex, "input_sha256":hex, "recipe_sha256":hex});
            let mut raw = crate::owner_beta::paper::source_evidence::canonical(&artifact).unwrap();
            raw.push(b'\n');
            let complete = format!("{}\n", &digest_bytes(&raw)[7..]).into_bytes();
            hashes.insert(name.into(), serde_json::json!({"artifact.json":digest_bytes(&raw),"COMPLETE":digest_bytes(&complete)}));
            files.insert(
                format!("paper_reader/provenance/{name}/artifact.json"),
                Some(raw),
            );
            files.insert(
                format!("paper_reader/provenance/{name}/COMPLETE"),
                Some(complete),
            );
        }
        let producer =
            crate::owner_beta::paper::source_evidence::canonical(&Value::Object(hashes)).unwrap();
        let receipt = serde_json::json!({"schema":"owner-beta-paper-install-receipt-v1","code_commit":commit,"release_manifest_sha256":cap["release_manifest_sha256"],"capability_sha256":digest_bytes(&cap_bytes),"audit_projection_sha256":projection_hash,"process_sha256":digest_bytes(&process_bytes),"producer_receipts_sha256":digest_bytes(&producer)});
        files.insert(
            "paper_reader/install-receipt.json".into(),
            Some(crate::owner_beta::paper::source_evidence::canonical(&receipt).unwrap()),
        );
        files
    }
    fn consume_release(files: &std::collections::BTreeMap<String, Option<Vec<u8>>>) -> Domains {
        use std::io::Write;
        let mut domains = Domains::new(1000, 16 * 1024 * 1024);
        for (path, body) in files {
            let dir = body.is_none();
            let config = path.starts_with("paper_config/");
            let mode = if dir {
                0o700
            } else if config {
                0o440
            } else if path == "paper_reader/capability.json"
                || path == "paper_reader/migration-0069.up.sql"
                || path.starts_with("paper_reader/provenance/")
            {
                0o444
            } else if path == "paper_reader/api-image.elf"
                || path == "paper_reader/worker-image.elf"
            {
                0o400
            } else if path == "paper_reader/native-image.elf" {
                0o500
            } else if path == "paper_reader/install-receipt.json"
                || path == "paper_manifest/.lagrange-release-manifest"
            {
                0o600
            } else {
                0o644
            };
            let record = serde_json::to_vec(&serde_json::json!({"path":path,"kind":if dir {"directory"} else {"file"},"uid":0,"gid":if config {10001}else{0},"mode":mode,"size":body.as_ref().map_or(0,Vec::len),"sha256":body.as_ref().map(|b| digest_bytes(b))})).unwrap();
            if let Some(bytes) = body {
                let mut file = tempfile::tempfile().unwrap();
                file.write_all(bytes).unwrap();
                domains
                    .observe(ArchiveOriginalMember::Member {
                        path,
                        canonical_record: &record,
                        file: Some(&file),
                    })
                    .unwrap();
            } else {
                domains
                    .observe(ArchiveOriginalMember::Member {
                        path,
                        canonical_record: &record,
                        file: None,
                    })
                    .unwrap();
            }
        }
        domains
    }
    #[test]
    fn release_originals_accept_and_reject_alias_and_producer_mutations() {
        let original = release_fixture();
        assert!(
            consume_release(&original)
                .finish_release(&Value::Null, &Value::Null)
                .is_ok()
        );
        let mut bad = original.clone();
        bad.insert("paper_config/config.json".into(), Some(b"{}".to_vec()));
        assert!(
            consume_release(&bad)
                .finish_release(&Value::Null, &Value::Null)
                .is_err()
        );
        let mut bad = original.clone();
        bad.insert(
            "paper_reader/provenance/api-server/COMPLETE".into(),
            Some(b"bad\n".to_vec()),
        );
        assert!(
            consume_release(&bad)
                .finish_release(&Value::Null, &Value::Null)
                .is_err()
        );
        let mut bad = original.clone();
        bad.remove("paper_contract/migration-0068.up.sql");
        assert!(
            consume_release(&bad)
                .finish_release(&Value::Null, &Value::Null)
                .is_err()
        );
    }
    fn history_rows(
        files: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
    ) -> Vec<(&'static str, Value)> {
        let process: Value =
            serde_json::from_slice(files["paper_config/process.json"].as_ref().unwrap()).unwrap();
        let process_hash = digest_bytes(files["paper_config/process.json"].as_ref().unwrap());
        let activation = "00000000-0000-4000-8000-000000000015";
        let configured = "00000000-0000-4000-8000-000000000016";
        let contract = serde_json::json!({"contract_id":"owner-beta-paper-v1-draft-1","contract_sha256":process["financial_contract_sha256"],"decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394","cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1});
        let control = serde_json::json!({"account_id":process["account_id"],"owner_user_id":process["owner_user_id"],"strategy_config_id":process["strategy_config_id"],"contract_id":contract["contract_id"],"contract_sha256":contract["contract_sha256"],"contract_json":contract,"cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1,"cost_profile_sha256":digest_bytes(b"KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000"),"consent_event_id":process["consent_event_id"],"activation_event_id":activation,"process_binding":process,"process_binding_sha256":process_hash});
        let event = |id: &Value, kind: &str, payload: Value| serde_json::json!({"id":id,"account_id":process["account_id"],"owner_user_id":process["owner_user_id"],"actor_user_id":process["owner_user_id"],"control_epoch":1,"kind":kind,"payload":payload});
        let e0 = event(
            &serde_json::json!(configured),
            "CONFIGURED",
            serde_json::json!({"schema":"owner-beta-paper-configured-v1","strategy_config_id":process["strategy_config_id"],"contract":contract}),
        );
        let e1 = event(
            &process["consent_event_id"],
            "CONSENT_GRANTED",
            serde_json::json!({"schema":"owner-beta-paper-consent-v1","consent_version":"owner-beta-paper-v1-draft-1","contract_sha256":process["financial_contract_sha256"],"opt_in":true}),
        );
        let e2 = event(
            &serde_json::json!(activation),
            "ENABLED",
            serde_json::json!({"process_binding":process,"process_binding_sha256":process_hash}),
        );
        vec![
            ("owner_beta_paper_control", control),
            ("owner_beta_paper_events", e0),
            ("owner_beta_paper_events", e1),
            ("owner_beta_paper_events", e2),
        ]
    }
    fn add_history(domains: &mut Domains, rows: &[(&str, Value)]) {
        for (index, (table, row)) in rows.iter().enumerate() {
            let bytes = serde_json::to_vec(row).unwrap();
            let key = index.to_string();
            domains
                .observe(ArchiveOriginalMember::Reference {
                    table,
                    key: &key,
                    canonical_row: &bytes,
                })
                .unwrap();
        }
    }
    #[test]
    fn historical_process_uses_its_own_event_and_consent() {
        let files = release_fixture();
        let rows = history_rows(&files);
        let mut d = consume_release(&files);
        add_history(&mut d, &rows);
        assert!(d.finish_release(&Value::Null, &Value::Null).is_ok());
        let mut altered = rows.clone();
        altered[3].1["payload"]["process_binding_sha256"] =
            serde_json::json!(format!("sha256:{}", "f".repeat(64)));
        let mut d = consume_release(&files);
        add_history(&mut d, &altered);
        assert!(d.finish_release(&Value::Null, &Value::Null).is_err());
    }
    #[test]
    fn lineage_pins_follow_the_original_run() {
        let run_id = "00000000-0000-4000-8000-000000000021";
        let owner = "00000000-0000-4000-8000-000000000022";
        let config = "00000000-0000-4000-8000-000000000023";
        let admission = format!("sha256:{}", "a".repeat(64));
        let factor = format!("sha256:{}", "b".repeat(64));
        let target = format!("sha256:{}", "c".repeat(64));
        let run = serde_json::json!({"id":run_id,"owner_user_id":owner,"strategy_config_id":config,"status":"SUCCEEDED","trigger_kind":"DAILY","as_of":"2026-11-10","daily_pins":{"admission_sha256":admission},"factor_snapshot_sha256":factor,"target_snapshot_sha256":target});
        let mut lineage = serde_json::json!({"recommendation_run_id":run_id,"owner_user_id":owner,"strategy_config_id":config,"signal_date":"2026-11-10","signal_admission_sha256":admission,"factor_snapshot_sha256":factor,"target_snapshot_sha256":target});
        let mut domain = Domains::new(20, 65536);
        let run_bytes = serde_json::to_vec(&run).unwrap();
        domain
            .observe(ArchiveOriginalMember::Reference {
                table: "owner_beta_recommendation_runs",
                key: run_id,
                canonical_row: &run_bytes,
            })
            .unwrap();
        let lineage_bytes = serde_json::to_vec(&lineage).unwrap();
        domain
            .observe(ArchiveOriginalMember::Reference {
                table: "owner_beta_paper_lineage",
                key: "l",
                canonical_row: &lineage_bytes,
            })
            .unwrap();
        assert!(domain.finish_r4_lineages().is_ok());
        lineage["target_snapshot_sha256"] = serde_json::json!(format!("sha256:{}", "d".repeat(64)));
        let mut bad = Domains::new(20, 65536);
        let bytes = serde_json::to_vec(&lineage).unwrap();
        bad.observe(ArchiveOriginalMember::Reference {
            table: "owner_beta_recommendation_runs",
            key: run_id,
            canonical_row: &run_bytes,
        })
        .unwrap();
        bad.observe(ArchiveOriginalMember::Reference {
            table: "owner_beta_paper_lineage",
            key: "l",
            canonical_row: &bytes,
        })
        .unwrap();
        assert!(bad.finish_r4_lineages().is_err());
    }
    #[test]
    fn nonmaintenance_action_does_not_require_maintenance_stage() {
        let f = fixtures();
        for name in ["configure", "enable", "recover"] {
            let root =
                wire::parse_root_intent(object(&f, &format!("{name}.root")).as_bytes()).unwrap();
            let _empty_stages = StageChain::new(&root, root.intent.local_epoch_floor).unwrap();
            let mut chain = ActionChain::new(&root);
            chain
                .observe(
                    object(&f, &format!("{name}.action0")).as_bytes(),
                    Some(object(&f, &format!("{name}.action0.dispatch")).as_bytes()),
                )
                .unwrap();
        }
    }
    #[test]
    fn simulation_pair_callback_preserves_exact_original_and_rejects_mutations() {
        let id = "00000000-0000-4000-8000-000000000031";
        let owner = "00000000-0000-4000-8000-000000000032";
        let mut value = serde_json::json!({"run_id":id,"owner_user_id":owner,"illustrative_canonical":"7b7d","illustrative_sha256":digest_bytes(b"{}"),"zero_cost_canonical":"5b5d","zero_cost_sha256":digest_bytes(b"[]"),"created_at":"2026-11-11T00:00:00.000000Z"});
        let bytes = serde_json::to_vec(&value).unwrap();
        let mut d = Domains::new(20, 65536);
        d.observe(ArchiveOriginalMember::Database {
            group: "simulation",
            table: "owner_beta_simulation_result_pairs",
            key: id,
            canonical_row: &bytes,
        })
        .unwrap();
        assert!(d.finish_r4_pairs().is_err()); // A pair alone is not the run/input/engine chain.
        value["illustrative_sha256"] = serde_json::json!(format!("sha256:{}", "a".repeat(64)));
        let bad = serde_json::to_vec(&value).unwrap();
        assert!(
            Domains::new(20, 65536)
                .observe(ArchiveOriginalMember::Database {
                    group: "simulation",
                    table: "owner_beta_simulation_result_pairs",
                    key: id,
                    canonical_row: &bad
                })
                .is_err()
        );
    }
}
