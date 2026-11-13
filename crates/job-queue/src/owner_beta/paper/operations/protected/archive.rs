//! RESTORE BEGIN intake: complete read-only verification precedes the borrowed
//! consuming BEGIN continuation. Original owners survive dispatch and cleanup.
//! Root/controller custody selects the owned isolated cluster. Native verifies
//! the real session and one fresh RR RO snapshot; tokens never cross clusters.
use super::*;
use market_data::checked_close::{Attempt, Gap};
use market_data::daily_price_only::source::ScanLimitValues;
use market_data::paper_archive_sources::closure::{
    ArchiveAdmissionContext, ArchiveDatasetRow, ArchiveOriginalMember, ArchiveOriginalOwner,
    ArchiveOriginalPaths, SourceClosureError, SourceClosureLimits, decode_archive_dataset_rows,
};
use sqlx::{Connection, Row};
use std::{collections::BTreeMap, future::Future, pin::Pin, time::Instant};

#[path = "archive_domains.rs"]
mod domains;

const PACKAGE: &str = "/run/owner-beta-paper-ops/archive";
const ARCHIVE_DOCUMENT: &str = "/run/owner-beta-paper-ops/archive/archive.json";
const AUDIT_DOCUMENT: &str = "/run/owner-beta-paper-ops/archive/audit.json";
const AUDIT: &str = "/run/owner-beta-paper-ops/archive/audit.stream";
const DATASET_ROWS: &str = "/run/owner-beta-paper-ops/archive/dataset-rows.jsonl";
const CODEC: &str = "/run/owner-beta-paper-ops/archive/files.bundle";
const FILES: &str = "/run/owner-beta-paper-ops/archive/files";
const COORDINATION: &str = "/run/owner-beta-paper-ops/archive/coordination";
const MAX_LINE: usize = 146_800_640;

#[cfg(all(test, not(feature = "r4-pg-observer-tests")))]
#[allow(dead_code)]
#[path = "../../../../../tests/common/mod.rs"]
mod test_database;
#[cfg(all(test, feature = "r4-pg-observer-tests"))]
use crate::owner_beta::simulation_execution::pg_test_common as test_database;

// New receiver caps are explicit and below the existing archive parser caps.
// Never truncate/coarsen a cohort to fit them: crossing any cap refuses intake.
fn limits() -> (SourceClosureLimits, ScanLimitValues) {
    (
        SourceClosureLimits {
            max_members: 65_536,
            max_manifest_bytes: 67_108_864,
            max_reference_bytes: 67_108_864,
            max_reference_rows: 1_000_000,
            max_open_descriptors: 131_072,
            max_file_bytes: 1 << 50,
            max_stream_bytes: 1 << 50,
        },
        ScanLimitValues {
            max_manifest_bytes: 67_108_864,
            max_manifest_records: 1_000_000,
            max_batches: 1_000_000,
            max_files_per_batch: 1_000_000,
            max_files: 1_000_000,
            max_batch_metadata_bytes: 67_108_864,
            max_source_file_bytes: 67_108_864,
            max_retained_bytes: 67_108_864,
            max_open_descriptors: 4096,
            max_data_root_bytes: 4096,
            max_data_root_components: 128,
            max_metadata_json_depth: 32,
        },
    )
}

/// Owns a detached connection, never a reusable pool lease. Dropping a
/// cancelled future closes that exact client and poisons the original Attempt.
/// Backend absence is still the host's independent owned-cluster cleanup duty.
struct ArchiveConnection<'a> {
    connection: Option<sqlx::PgConnection>,
    attempt: &'a Attempt,
}
impl ArchiveConnection<'_> {
    fn connection(&mut self) -> Result<&mut sqlx::PgConnection, ErrorCode> {
        self.connection
            .as_mut()
            .ok_or(ErrorCode::EvidenceUnavailable)
    }
    async fn finish(mut self) -> bool {
        let Some(mut connection) = self.connection.take() else {
            return false;
        };
        let rolled_back = tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query("ROLLBACK").execute(&mut connection),
        )
        .await
        .is_ok_and(|v| v.is_ok());
        let closed = tokio::time::timeout(Duration::from_secs(5), connection.close())
            .await
            .is_ok_and(|v| v.is_ok());
        if !rolled_back || !closed {
            self.attempt.gap(Gap::Abandoned);
        }
        rolled_back && closed
    }
}
impl Drop for ArchiveConnection<'_> {
    fn drop(&mut self) {
        if self.connection.is_some() {
            self.attempt.gap(Gap::Abandoned);
            drop(self.connection.take());
        }
    }
}

async fn compare_projection(
    connection: &mut sqlx::PgConnection,
    audit: &GuardedFile,
    sql: &str,
    deadline: Instant,
) -> Result<(), ErrorCode> {
    // SQL comes unchanged from the held audit.sql original whose digest is
    // checked by ArchiveIntakeInputs. No user values are interpolated here.
    let mut stream = sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).fetch_many(connection);
    let mut offset = 0u64;
    // Poll SQLx's existing boxed stream directly; no new dependency/client.
    while let Some(next) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
        if Instant::now() >= deadline {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let item = next.map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if let sqlx::Either::Right(row) = item {
            if row.len() != 1 {
                return Err(ErrorCode::EvidenceUnavailable);
            }
            let line: String = row.try_get(0).map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if line.len() > MAX_LINE || line.contains('\n') || line.contains('\r') {
                return Err(ErrorCode::EvidenceUnavailable);
            }
            audit
                .revalidate()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let mut original = vec![0; line.len() + 1];
            audit
                .file
                .read_exact_at(&mut original, offset)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if &original[..line.len()] != line.as_bytes() || original.last() != Some(&b'\n') {
                return Err(ErrorCode::IdentityConflict);
            }
            offset = offset
                .checked_add(original.len() as u64)
                .ok_or(ErrorCode::EvidenceUnavailable)?;
            audit
                .revalidate()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        }
    }
    if offset != audit.identity.size {
        return Err(ErrorCode::IdentityConflict);
    }
    let mut extra = [0];
    if audit
        .file
        .read_at(&mut extra, offset)
        .map_err(|_| ErrorCode::EvidenceUnavailable)?
        != 0
    {
        return Err(ErrorCode::IdentityConflict);
    }
    audit
        .revalidate()
        .map_err(|_| ErrorCode::EvidenceUnavailable)
}

#[derive(Debug, PartialEq, Eq)]
struct DatasetReference {
    hash: String,
    version: Option<String>,
}
fn dataset_reference(
    table: &str,
    row: &[u8],
) -> Result<Option<(Uuid, DatasetReference)>, SourceClosureError> {
    if !matches!(table, "pending_targets" | "paper_rebalance_previews") {
        return Ok(None);
    }
    let v: Value = serde_json::from_slice(row).map_err(|_| SourceClosureError::DatabaseRow)?;
    if v["dataset_version_id"].is_null() {
        if !v["dataset_manifest_sha256"].is_null() {
            return Err(SourceClosureError::Relationship);
        }
        return Ok(None); // Beta NULL tuples stay NULL.
    }
    let id = v["dataset_version_id"]
        .as_str()
        .and_then(|x| Uuid::parse_str(x).ok())
        .ok_or(SourceClosureError::Relationship)?;
    let hash = v["dataset_manifest_sha256"]
        .as_str()
        .ok_or(SourceClosureError::Relationship)?
        .to_owned();
    let version = v
        .get("dataset_version")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or(SourceClosureError::Relationship)
        })
        .transpose()?;
    Ok(Some((id, DatasetReference { hash, version })))
}

// These are untrusted selectors obtained from the held stream, never checked
// dataset authority. Repeated rows can omit a version but cannot replace one.
fn retain_dataset_reference(
    rows: &mut BTreeMap<Uuid, DatasetReference>,
    budget: &mut usize,
    id: Uuid,
    row: DatasetReference,
) -> Result<(), SourceClosureError> {
    if let Some(old) = rows.get_mut(&id) {
        if old.hash != row.hash
            || old
                .version
                .as_ref()
                .zip(row.version.as_ref())
                .is_some_and(|(a, b)| a != b)
        {
            return Err(SourceClosureError::Relationship);
        }
        if old.version.is_none() {
            let cost = row.version.as_ref().map_or(0, String::len);
            *budget = budget.checked_sub(cost).ok_or(SourceClosureError::Limit)?;
            old.version = row.version;
        }
    } else {
        let cost = 256usize
            .checked_add(row.hash.len())
            .and_then(|n| n.checked_add(row.version.as_ref().map_or(0, String::len)))
            .ok_or(SourceClosureError::Limit)?;
        *budget = budget.checked_sub(cost).ok_or(SourceClosureError::Limit)?;
        rows.insert(id, row);
    }
    Ok(())
}

async fn read_archive_dataset_rows_on(
    connection: &mut sqlx::PgConnection,
    references: &BTreeMap<Uuid, DatasetReference>,
    budget: &mut usize,
    mut after_row: impl FnMut() -> Result<(), ErrorCode>,
) -> Result<Vec<ArchiveDatasetRow>, ErrorCode> {
    let mut retained = Vec::new();
    for (id, reference) in references {
        // pg_column_size reflects TOAST compression, not retained UTF-8 bytes.
        // Check actual string lengths before the descriptive reader allocates.
        let bytes: Option<i64> = sqlx::query_scalar("SELECT (octet_length(dataset_id)::bigint+octet_length(version)+octet_length(status)+octet_length(manifest_sha256)+octet_length(storage_path)+256) FROM public.dataset_versions WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let bytes = bytes.ok_or(ErrorCode::IdentityConflict)?;
        charge_archive_dataset_row_budget(budget, bytes)?;
        let row = crate::recommendation::input::read_dataset_row_on(connection, *id)
            .await
            .map_err(|_| ErrorCode::EvidenceUnavailable)?
            .ok_or(ErrorCode::IdentityConflict)?;
        if row.id != *id
            || row.manifest_sha256 != reference.hash
            || reference
                .version
                .as_ref()
                .is_some_and(|version| version != &row.version)
        {
            return Err(ErrorCode::IdentityConflict);
        }
        retained.push(ArchiveDatasetRow {
            id: row.id,
            dataset_id: row.dataset_id,
            version: row.version,
            status: row.status,
            manifest_sha256: row.manifest_sha256,
            storage_path: row.storage_path,
        });
        after_row()?;
    }
    Ok(retained)
}

fn compare_dataset_rows(
    source: &[ArchiveDatasetRow],
    clone: &[ArchiveDatasetRow],
) -> Result<(), ErrorCode> {
    if source != clone {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

fn charge_archive_dataset_row_budget(
    budget: &mut usize,
    reported_bytes: i64,
) -> Result<(), ErrorCode> {
    let bytes = reported_bytes
        .try_into()
        .map_err(|_| ErrorCode::IdentityConflict)?;
    *budget = budget
        .checked_sub(bytes)
        .ok_or(ErrorCode::EvidenceUnavailable)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DomainBudget {
    rows: usize,
    bytes: usize,
}
impl DomainBudget {
    fn remaining(
        caps: SourceClosureLimits,
        reference_rows: usize,
        source_rows: usize,
        selector_rows: usize,
        clone_rows: usize,
        remaining_bytes: usize,
    ) -> Result<Self, ErrorCode> {
        let rows = [reference_rows, source_rows, selector_rows, clone_rows]
            .into_iter()
            .try_fold(caps.max_reference_rows, |left, used| left.checked_sub(used))
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        if remaining_bytes > caps.max_reference_bytes {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        Ok(Self {
            rows,
            bytes: remaining_bytes,
        })
    }

    fn consume(&mut self, (rows, bytes): (usize, usize)) -> Result<(), ErrorCode> {
        let rows = self
            .rows
            .checked_sub(rows)
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        let bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        *self = Self { rows, bytes };
        Ok(())
    }
}

#[cfg(test)]
fn complete_journal(
    domains: &domains::Domains,
    budget: &mut DomainBudget,
    archive: &Value,
    audit: &Value,
) -> Result<(), ErrorCode> {
    // Observed originals and the finish-only temporary indexes use the SAME
    // limits supplied after the source and clone reads, even on refusal.
    budget.consume(domains.retained_usage())?;
    domains.finish_journal(archive, audit)
}

fn complete_domains(
    domains: &domains::Domains,
    budget: &mut DomainBudget,
    archive: &Value,
    audit: &Value,
) -> Result<(), SourceClosureError> {
    budget
        .consume(domains.retained_usage())
        .map_err(|_| SourceClosureError::Limit)?;
    domains.finish(archive, audit)
}

fn revalidate_capture(file: &mut GuardedFile, original: &[u8]) -> Result<(), ErrorCode> {
    let actual = file
        .digest_bounded(original.len() as u64)
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    if actual != digest(original) {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

async fn verify_archive_migration_on(
    connection: &mut sqlx::PgConnection,
    trusted_bytes: &[u8],
) -> Result<(), ErrorCode> {
    let checksum: Vec<u8> = sqlx::query_scalar(
        "SELECT checksum FROM public._sqlx_migrations WHERE version=69 AND success",
    )
    .fetch_one(connection)
    .await
    .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let expected = sha2::Sha384::digest(trusted_bytes);
    if checksum.as_slice() != &expected[..] {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

#[derive(Default)]
struct FrameRoot {
    hash: Sha256,
    entries: u64,
    bytes: u64,
}
impl FrameRoot {
    fn add(&mut self, table: &str, key: &str, canonical: &[u8]) -> Result<(), SourceClosureError> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or(SourceClosureError::Limit)?;
        for value in [table.as_bytes(), key.as_bytes(), canonical] {
            let prefix = format!("{}:", value.len());
            self.bytes = self
                .bytes
                .checked_add(prefix.len() as u64)
                .and_then(|n| n.checked_add(value.len() as u64))
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or(SourceClosureError::Limit)?;
            self.hash.update(prefix.as_bytes());
            self.hash.update(value);
        }
        Ok(())
    }
    fn description(&self, group: &str) -> Value {
        json!({"group":group,"entries":self.entries,"bytes":self.bytes,
            "sha256":format!("sha256:{:x}",self.hash.clone().finalize())})
    }
}

fn audit_roots() -> BTreeMap<&'static str, FrameRoot> {
    [
        "daily",
        "paper_control",
        "paper_jobs",
        "paper_ledger",
        "paper_lineage",
        "paper_operations",
        "paper_outbox",
        "schema_security",
        "simulation",
    ]
    .into_iter()
    .map(|group| (group, FrameRoot::default()))
    .collect()
}

fn compare_descriptors(
    archive: &Value,
    audit: &Value,
    intent: &Intent,
    sql_bytes: &[u8],
    groups: &BTreeMap<&str, FrameRoot>,
    sources: market_data::paper_archive_sources::closure::ArchiveSourceDescription,
) -> Result<(), ErrorCode> {
    if !exact_keys(
        archive,
        &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "profile",
            "control_epoch",
            "drain_receipt_sha256",
            "created_at",
            "release_manifest_sha256",
            "capability_sha256",
            "database_dump_sha256",
            "database_snapshot_sha256",
            "audit_sha256",
            "files_sha256",
            "classes",
        ],
    ) || !exact_keys(
        audit,
        &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "control_epoch",
            "audit_sql_sha256",
            "migration_0068_sha256",
            "groups",
        ],
    ) || archive["profile"] != "paper-combined"
        || archive["installation_id"] == intent.installation_id.to_string()
        || archive["generation"] != intent.value["predecessor_generation"]
        || archive["account_id"] != intent.value["account_id"]
        || archive["control_epoch"] != intent.expected_epoch
        || !["installation_id", "generation", "account_id"]
            .iter()
            .all(|key| canonical_uuid(&archive[*key]))
        || ![
            "drain_receipt_sha256",
            "release_manifest_sha256",
            "capability_sha256",
            "database_dump_sha256",
            "database_snapshot_sha256",
            "audit_sha256",
            "files_sha256",
        ]
        .iter()
        .all(|key| sha256_wire(&archive[*key]))
        || audit["audit_sql_sha256"] != digest(sql_bytes)
        || audit["migration_0068_sha256"]
            != digest(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../migrations/0068_owner_beta_paper_operations_v1.up.sql"
            )))
        || archive["files_sha256"] != sources.files_sha256
        || archive["classes"] != Value::Array(sources.classes)
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let created = archive["created_at"]
        .as_str()
        .ok_or(ErrorCode::WireInvalid)?;
    let date = chrono::DateTime::parse_from_rfc3339(created).map_err(|_| ErrorCode::WireInvalid)?;
    if created.len() != 20
        || !created.starts_with("20")
        || !created.ends_with('Z')
        || date.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != created
    {
        return Err(ErrorCode::WireInvalid);
    }
    let binding = json!({"domain":"lagrange-paper-snapshot-binding-v1",
        "installation_id":archive["installation_id"],"generation":archive["generation"],
        "account_id":archive["account_id"],"control_epoch":archive["control_epoch"],
        "database_dump_sha256":archive["database_dump_sha256"],
        "audit_stream_sha256":sources.audit_stream_sha256,"files_sha256":archive["files_sha256"]});
    if archive["database_snapshot_sha256"] != digest(canonical_text(&binding)?.as_bytes()) {
        return Err(ErrorCode::IdentityConflict);
    }
    let mut expected = groups
        .iter()
        .map(|(name, root)| root.description(name))
        .collect::<Vec<_>>();
    expected.push(json!({"group":"source_files","sha256":sources.sha256,
        "entries":sources.entries,"bytes":sources.bytes}));
    expected.sort_by(|a, b| a["group"].as_str().cmp(&b["group"].as_str()));
    if audit["groups"] != Value::Array(expected) {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

pub(super) type ArchiveBeginFuture<'a> = Pin<Box<dyn Future<Output = ResultWire> + 'a>>;
pub(super) type ArchiveBeginContinuation = for<'a> fn(CheckedArchive<'a>) -> ArchiveBeginFuture<'a>;

/// The closure borrows the actual original/Raw owners; it is not a detached
/// certificate. Only this module constructs the view after all domain checks.
pub(super) struct CheckedArchive<'a> {
    sandbox: &'a super::archive_sandbox::Guard,
    pub(super) inputs: &'a ProtectedInputs,
    pub(super) intent: &'a Intent,
    pub(super) selector: &'a DatabaseSelector,
    pub(super) output: &'a GuardedDirectory,
    pub(super) operation_started: Instant,
    archive: &'a Value,
    audit: &'a Value,
    original_check: &'a mut dyn FnMut() -> Result<(), ErrorCode>,
}
impl CheckedArchive<'_> {
    pub(super) fn revalidate(&mut self) -> Result<(), ErrorCode> {
        self.sandbox.revalidate()?;
        if self.operation_started.elapsed() >= Duration::from_secs(15)
            || self.archive()["schema"] != "owner-beta-paper-archive-v1"
            || self.audit()["schema"] != "owner-beta-paper-audit-v1"
        {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        (self.original_check)()
    }
    pub(super) fn archive(&self) -> &Value {
        self.archive
    }
    pub(super) fn audit(&self) -> &Value {
        self.audit
    }
}

pub(super) async fn with_verified_archive(
    inputs: &ProtectedInputs,
    intent: &Intent,
    selector: &DatabaseSelector,
    output: &GuardedDirectory,
    operation_started: Instant,
    sandbox: &super::archive_sandbox::Guard,
    continuation: ArchiveBeginContinuation,
) -> ResultWire {
    let verb = Verb::Begin;
    // Exact descriptive replay remains the pre-existing Readback entrypoint;
    // an existing dispatch is never retransmitted or live-verified here.
    if bound::has_dispatch().unwrap_or(true) {
        return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    }
    let deadline = operation_started + Duration::from_secs(15);
    let attempt = Attempt::default();
    let mut delivered = None;
    let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
        sandbox.revalidate()?;
        if selector.host != "/run/postgresql" || !bound::output_is_empty(output)? {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let originals = inputs.bound.as_ref().ok_or(ErrorCode::EvidenceUnavailable)?;
        let mut live = chain::ArchiveIntakeInputs::open(originals, inputs)?;
        let package = GuardedDirectory::open(PACKAGE).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        chain::archive_package_names(&package)?;
        let audit = GuardedFile::open_mode(AUDIT, 0o400).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut archive_file = GuardedFile::open_mode(ARCHIVE_DOCUMENT, 0o400)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let archive_bytes = archive_file.read_bounded(65_536).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if archive_bytes != live.archive_document() { return Err(ErrorCode::IdentityConflict); }
        let archive_value = wire::parse_canonical(&archive_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        let mut audit_file = GuardedFile::open_mode(AUDIT_DOCUMENT, 0o400)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let audit_bytes = audit_file.read_bounded(65_536).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let audit_value = wire::parse_canonical(&audit_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        if archive_value["audit_sha256"] != digest(&audit_bytes)
            || archive_value["schema"] != "owner-beta-paper-archive-v1"
            || audit_value["schema"] != "owner-beta-paper-audit-v1"
            || ["installation_id", "generation", "account_id", "control_epoch"].iter()
                .any(|key| archive_value[*key] != audit_value[*key]) {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut source_datasets = GuardedFile::open_mode(DATASET_ROWS, 0o400)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let source_dataset_bytes = source_datasets.read_bounded(64 * 1024 * 1024)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let source_dataset_rows = decode_archive_dataset_rows(&source_dataset_bytes, limits().0.max_reference_rows)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut audit_sql = GuardedFile::open(AUDIT_SQL_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let sql_bytes = audit_sql.read_bounded(MAX_SOURCE_FILE_BYTES).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let sql = std::str::from_utf8(&sql_bytes).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let context = ArchiveAdmissionContext::open(&attempt, deadline).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let invocation = context.invocation();
        let (mut caps, raw_caps) = limits();
        // Charge the retained source spool, decoded strings and row overhead
        // before the source owner allocates its reference index.
        let dataset_cost = source_dataset_bytes.len().checked_mul(2)
            .and_then(|n| n.checked_add(source_dataset_rows.len().checked_mul(256)?))
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        caps.max_reference_bytes = caps.max_reference_bytes.checked_sub(dataset_cost)
            .filter(|n| *n > 0).ok_or(ErrorCode::EvidenceUnavailable)?;
        let mut groups = audit_roots();
        let mut owner = ArchiveOriginalOwner::open_archive_in(&invocation, &attempt, &attempt,
            ArchiveOriginalPaths { files_bundle: Path::new(CODEC), database_stream: Path::new(AUDIT),
                materialized: Path::new(FILES), raw_coordination: Path::new(COORDINATION) },
            caps, raw_caps, || Instant::now() < deadline, |member| {
                if let ArchiveOriginalMember::Database { group, table, key, canonical_row } = member {
                    groups.get_mut(group).ok_or(SourceClosureError::DatabaseStream)?.add(table,key,canonical_row)?;
                }
                Ok(())
            })
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        // Source rows only select real reads on the held clone snapshot.
        let (used_rows, used_bytes) = owner.reference_usage().map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut budget = caps.max_reference_bytes.checked_sub(used_bytes).ok_or(ErrorCode::EvidenceUnavailable)?;
        let mut datasets = BTreeMap::new();
        owner.visit_reference_rows(|| Instant::now() < deadline, |member| {
            let ArchiveOriginalMember::Reference { table, canonical_row, .. } = member else {
                return Ok(());
            };
            if let Some((id, row)) = dataset_reference(table, canonical_row)? {
                retain_dataset_reference(&mut datasets, &mut budget, id, row)?;
            }
            Ok(())
        }).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let pool = connect(selector, DatabaseRole::MigrationOwner).await?;
        let connection = pool.acquire().await.map_err(|_| ErrorCode::EvidenceUnavailable)?.detach();
        pool.close().await;
        let mut database = ArchiveConnection { connection: Some(connection), attempt: &attempt };
        let mut body = async {
            let connection = database.connection()?;
            sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL statement_timeout='15s'; SET LOCAL lock_timeout='5s'; SET LOCAL search_path='pg_catalog, pg_temp';")
                .execute(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let actual: bool = sqlx::query_scalar("SELECT current_user='migration_owner' AND session_user=current_user AND current_setting('role')='none' AND current_setting('transaction_isolation')='repeatable read' AND current_setting('transaction_read_only')='on' AND current_setting('server_encoding')='UTF8' AND current_setting('client_encoding')='UTF8' AND current_setting('search_path')='pg_catalog, pg_temp' AND current_setting('statement_timeout')='15s' AND current_setting('lock_timeout')='5s'")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if !actual { return Err(ErrorCode::EvidenceUnavailable); }
            let actor_before: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            verify_archive_migration_on(connection, live.migration_0069_bytes()).await?;
            compare_projection(connection, &audit, sql, deadline).await?;
            let actor_after: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if actor_before.as_deref().unwrap_or("") != actor_after.as_deref().unwrap_or("") {
                return Err(ErrorCode::IdentityConflict);
            }
            // Audit SQL independently verifies real role flags, RLS, schema,
            // migrations, full nine-group rows, actor restore and physical EOF.
            let dataset_rows = read_archive_dataset_rows_on(
                connection,
                &datasets,
                &mut budget,
                || {
                live.revalidate(originals, inputs)?;
                owner.revalidate(&mut || Instant::now() < deadline).map_err(|_| ErrorCode::EvidenceUnavailable)?;
                Ok(())
            },
            ).await?;
            compare_dataset_rows(&source_dataset_rows, &dataset_rows)?;
            source_datasets.revalidate().map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let description = owner.source_description(&dataset_rows, &mut || Instant::now() < deadline)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            compare_descriptors(&archive_value, &audit_value, intent, &sql_bytes, &groups, description)?;
            // Construct only after both dataset captures and all preceding
            // retained indexes have consumed their share of the original caps.
            let mut domain_budget = DomainBudget::remaining(caps, used_rows,
                source_dataset_rows.len(), datasets.len(), dataset_rows.len(), budget)?;
            let domains = std::cell::RefCell::new(domains::Domains::until(domain_budget.rows, domain_budget.bytes, deadline));
            owner.visit_database_rows(|| Instant::now() < deadline,
                |member| domains.borrow_mut().observe(member))
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            // SAME collector receives the second reference view and all owned
            // source members. No RefCell borrow crosses an await or callback.
            let traversal = owner.visit_sources_with_domain_consumer(
                &dataset_rows, || Instant::now() < deadline,
                |member| domains.borrow_mut().observe(member),
                || {
                    let domains = domains.borrow();
                    complete_domains(&domains, &mut domain_budget, &archive_value, &audit_value)
                },
            );
            // Only this private complete domain consumer precedes CheckedArchive.
            // Any source, domain, deadline or original failure aborts before BEGIN.
            traversal.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            Ok::<(), ErrorCode>(())
        }.await;
        // The verification connection is closed BEFORE BEGIN; all filesystem
        // and Raw owners below remain alive across the consuming future.
        let database_closed = database.finish().await;
        if body.is_ok() && database_closed {
            let mut revalidate = || {
                sandbox.revalidate()?;
                live.revalidate(originals, inputs)?;
                chain::archive_package_names(&package)?;
                owner.revalidate(&mut || Instant::now() < deadline).map_err(|_| ErrorCode::EvidenceUnavailable)?;
                audit.revalidate().map_err(|_| ErrorCode::EvidenceUnavailable)?;
                revalidate_capture(&mut archive_file, &archive_bytes)?;
                revalidate_capture(&mut audit_file, &audit_bytes)?;
                revalidate_capture(&mut source_datasets, &source_dataset_bytes)?;
                revalidate_capture(&mut audit_sql, &sql_bytes)?;
                Ok(())
            };
            body = revalidate();
            if body.is_ok() {
                delivered = Some(continuation(CheckedArchive {
                    inputs, intent, selector, output, operation_started, sandbox,
                    archive: &archive_value, audit: &audit_value, original_check: &mut revalidate,
                }).await);
                body = revalidate();
            }
        }
        // Recheck independent originals even on a domain rejection, then close
        // every owner. No error or successful local close publishes an OK.
        let live_after = live.revalidate(originals, inputs);
        let package_after = chain::archive_package_names(&package);
        let sql_after = revalidate_capture(&mut audit_sql, &sql_bytes);
        let datasets_after = revalidate_capture(&mut source_datasets, &source_dataset_bytes);
        let documents_after = revalidate_capture(&mut archive_file, &archive_bytes)
            .and(revalidate_capture(&mut audit_file, &audit_bytes));
        let files_closed = owner.finish_local(|| Instant::now() < deadline);
        // The consumed owner ends the private invocation borrow here.
        let context_closed = context.finish_local();
        if Instant::now() >= deadline || !database_closed || files_closed.is_err() || context_closed.is_err()
            || sandbox.revalidate().is_err() || live_after.is_err() || package_after.is_err() || sql_after.is_err() || datasets_after.is_err() || documents_after.is_err() {
            attempt.gap(Gap::Abandoned);
        }
        body
    }).await;
    let body = match result {
        Ok(body) => body,
        Err(_) => Err(ErrorCode::EvidenceUnavailable),
    };
    let terminal = attempt.finish(body);
    match (terminal, delivered) {
        (Ok(()), Some(result)) => result,
        (_, Some(mut result)) => {
            // Retain the exact result identities if cleanup failed after SQL.
            result.outcome = Outcome::CommitUnknown;
            result.error_code = ErrorCode::CommitUnknown;
            result
        }
        _ => dispatch_failure(
            verb,
            intent,
            if bound::has_dispatch().unwrap_or(true) {
                ErrorCode::CommitUnknown
            } else {
                ErrorCode::EvidenceUnavailable
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_budget_starts_after_every_retained_dataset_copy() {
        let mut caps = limits().0;
        caps.max_reference_rows = 10;
        caps.max_reference_bytes = 100;
        assert_eq!(
            DomainBudget::remaining(caps, 2, 1, 3, 4, 17),
            Ok(DomainBudget { rows: 0, bytes: 17 })
        );
        for used in [[3, 1, 3, 4], [0, usize::MAX, 0, 0]] {
            assert_eq!(
                DomainBudget::remaining(caps, used[0], used[1], used[2], used[3], 17),
                Err(ErrorCode::EvidenceUnavailable)
            );
        }
        assert_eq!(
            DomainBudget::remaining(caps, 0, 0, 0, 0, 101),
            Err(ErrorCode::EvidenceUnavailable)
        );
        let initial = DomainBudget { rows: 2, bytes: 10 };
        for excess in [(3, 0), (0, 11), (usize::MAX, usize::MAX)] {
            let mut budget = initial;
            assert_eq!(budget.consume(excess), Err(ErrorCode::EvidenceUnavailable));
            assert_eq!(budget, initial);
        }
        let mut budget = initial;
        budget.consume((2, 10)).unwrap();
        assert_eq!(budget, DomainBudget { rows: 0, bytes: 0 });
    }

    #[test]
    fn one_domain_collector_joins_database_and_source_callbacks_with_shared_limits() {
        // A historical all-null event and an actually empty journal are valid
        // phase1 descriptions. They do not establish a complete source cohort.
        let row = br#"{"request_canonical":null,"request_sha256":null,"result_canonical":null,"result_sha256":null}"#;
        let directory = br#"{"gid":0,"kind":"directory","mode":448,"path":"paper_operations","sha256":null,"size":0,"uid":0}"#;
        let mut budget = DomainBudget {
            rows: 4,
            bytes: 4096,
        };
        let domains = std::cell::RefCell::new(domains::Domains::new(budget.rows, budget.bytes));
        domains
            .borrow_mut()
            .observe(ArchiveOriginalMember::Database {
                group: "paper_control",
                table: "owner_beta_paper_events",
                key: "legacy-event",
                canonical_row: row,
            })
            .unwrap();
        let database_usage = domains.borrow().retained_usage();
        domains
            .borrow_mut()
            .observe(ArchiveOriginalMember::Reference {
                table: "owner_beta_paper_events",
                key: "legacy-event",
                canonical_row: row,
            })
            .unwrap();
        assert_eq!(domains.borrow().retained_usage(), database_usage);
        domains
            .borrow_mut()
            .observe(ArchiveOriginalMember::Member {
                path: "paper_operations",
                canonical_record: directory,
                file: None,
            })
            .unwrap();
        let usage = domains.borrow().retained_usage();
        assert_eq!(usage.0, 2);
        complete_journal(&domains.borrow(), &mut budget, &Value::Null, &Value::Null).unwrap();
        assert_eq!(
            budget,
            DomainBudget {
                rows: 4 - usage.0,
                bytes: 4096 - usage.1
            }
        );
        assert_eq!(
            domains
                .borrow_mut()
                .observe(ArchiveOriginalMember::Reference {
                    table: "owner_beta_paper_events",
                    key: "legacy-event",
                    canonical_row: b"{}",
                }),
            Err(SourceClosureError::Duplicate)
        );
        assert_eq!(domains.borrow().retained_usage(), usage);

        let mut too_small = DomainBudget {
            rows: usage.0,
            bytes: usage.1 - 1,
        };
        let before = too_small;
        assert_eq!(
            complete_journal(
                &domains.borrow(),
                &mut too_small,
                &Value::Null,
                &Value::Null
            ),
            Err(ErrorCode::EvidenceUnavailable)
        );
        assert_eq!(too_small, before);
    }

    #[test]
    fn journal_refusal_keeps_actual_usage_and_does_not_become_completion() {
        let mut budget = DomainBudget {
            rows: 4,
            bytes: 4096,
        };
        let mut domains = domains::Domains::new(budget.rows, budget.bytes);
        // No journal directory original was delivered by the source traversal.
        domains
            .observe(ArchiveOriginalMember::Database {
                group: "paper_control",
                table: "owner_beta_paper_events",
                key: "event",
                canonical_row: b"{}",
            })
            .unwrap();
        let usage = domains.retained_usage();
        assert_eq!(
            complete_journal(&domains, &mut budget, &Value::Null, &Value::Null),
            Err(ErrorCode::EvidenceUnavailable)
        );
        assert_eq!(
            budget,
            DomainBudget {
                rows: 4 - usage.0,
                bytes: 4096 - usage.1
            }
        );
    }

    // Synthetic selectors and disposable database rows. These tests grant no
    // production Origin, Current, custody or archive-completion authority.
    fn row(hash: &str, version: Option<&str>) -> DatasetReference {
        DatasetReference {
            hash: hash.into(),
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn source_clone_equality_compares_every_dataset_field_and_exact_set() {
        let row = ArchiveDatasetRow {
            id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            dataset_id: "krx_eod_bars".into(),
            version: "preview-v1".into(),
            status: "BLOCKED".into(),
            manifest_sha256: "a".repeat(64),
            storage_path: "/data/curated".into(),
        };
        assert_eq!(
            compare_dataset_rows(std::slice::from_ref(&row), std::slice::from_ref(&row)),
            Ok(())
        );
        for field in 0..6 {
            let mut changed = row.clone();
            match field {
                0 => changed.id = Uuid::nil(),
                1 => changed.dataset_id.push('x'),
                2 => changed.version.push('x'),
                3 => changed.status = "READY".into(),
                4 => changed.manifest_sha256 = "b".repeat(64),
                _ => changed.storage_path = "/tmp/untrusted".into(),
            }
            assert_eq!(
                compare_dataset_rows(std::slice::from_ref(&row), &[changed]),
                Err(ErrorCode::IdentityConflict)
            );
        }
        assert_eq!(
            compare_dataset_rows(std::slice::from_ref(&row), &[]),
            Err(ErrorCode::IdentityConflict)
        );
        assert_eq!(
            compare_dataset_rows(&[], &[row]),
            Err(ErrorCode::IdentityConflict)
        );
    }

    #[test]
    fn native_candidate_frame_matches_shared_unicode_golden() {
        let fixture: Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/owner-beta-paper-archive-framing-v1.json"
        )))
        .unwrap();
        let mut root = FrameRoot::default();
        for record in fixture["records"].as_array().unwrap() {
            let canonical = canonical_text(&record[2]).unwrap();
            root.add(
                record[0].as_str().unwrap(),
                record[1].as_str().unwrap(),
                canonical.as_bytes(),
            )
            .unwrap();
        }
        assert_eq!(root.description("source_files"), fixture["description"]);
    }

    #[test]
    fn beta_null_dataset_remains_null_and_partial_tuple_refuses() {
        assert!(
            dataset_reference(
                "pending_targets",
                br#"{"dataset_version_id":null,"dataset_manifest_sha256":null}"#
            )
            .unwrap()
            .is_none()
        );
        assert!(
            dataset_reference(
                "pending_targets",
                br#"{"dataset_version_id":null,"dataset_manifest_sha256":"not-null"}"#
            )
            .is_err()
        );
        assert!(
            dataset_reference(
                "pending_targets",
                br#"{"dataset_version_id":123,"dataset_manifest_sha256":"x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn dataset_selection_is_narrow_and_never_origin() {
        assert!(
            dataset_reference("jobs", br#"{"dataset_version_id":"bad"}"#)
                .unwrap()
                .is_none()
        );
        assert!(
            dataset_reference(
                "paper_rebalance_previews",
                br#"{"dataset_version_id":"bad","dataset_manifest_sha256":"x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn compatible_preview_and_target_versions_join_in_either_order() {
        for versions in [[None, Some("original")], [Some("original"), None]] {
            let mut rows = BTreeMap::new();
            let mut budget = 1024;
            let id = Uuid::nil(); // descriptive unit selector, never a receipt
            for version in versions {
                retain_dataset_reference(&mut rows, &mut budget, id, row("hash", version)).unwrap();
            }
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[&id], row("hash", Some("original")));
            assert_eq!(budget, 1024 - 256 - "hash".len() - "original".len());
        }
    }

    #[test]
    fn conflicting_hash_or_version_cannot_replace_retained_selection() {
        for other in [row("other", Some("original")), row("hash", Some("changed"))] {
            let id = Uuid::nil();
            let mut rows = BTreeMap::new();
            let mut budget = 1024;
            retain_dataset_reference(&mut rows, &mut budget, id, row("hash", Some("original")))
                .unwrap();
            let remaining = budget;
            assert_eq!(
                retain_dataset_reference(&mut rows, &mut budget, id, other),
                Err(SourceClosureError::Relationship)
            );
            assert_eq!(budget, remaining);
            assert_eq!(rows[&id], row("hash", Some("original")));
        }
    }

    #[test]
    fn bounds_refuse_whole_selection_without_partial_replacement() {
        let id = Uuid::nil();
        let mut rows = BTreeMap::new();
        let mut budget = 0;
        assert_eq!(
            retain_dataset_reference(&mut rows, &mut budget, id, row("hash", None)),
            Err(SourceClosureError::Limit)
        );
        assert!(rows.is_empty());
        assert_eq!(budget, 0);
        budget = 260;
        retain_dataset_reference(&mut rows, &mut budget, id, row("hash", None)).unwrap();
        assert_eq!(
            retain_dataset_reference(&mut rows, &mut budget, id, row("hash", Some("original"))),
            Err(SourceClosureError::Limit)
        );
        assert_eq!(rows[&id].version, None);
    }

    #[test]
    fn archive_dataset_row_budget_is_cumulative_and_rejects_invalid_sizes() {
        let mut budget = 100;
        charge_archive_dataset_row_budget(&mut budget, 40).unwrap();
        assert_eq!(budget, 60);
        assert_eq!(
            charge_archive_dataset_row_budget(&mut budget, 61),
            Err(ErrorCode::EvidenceUnavailable)
        );
        assert_eq!(budget, 60);
        assert_eq!(
            charge_archive_dataset_row_budget(&mut budget, -1),
            Err(ErrorCode::IdentityConflict)
        );
        assert_eq!(budget, 60);
    }

    #[tokio::test]
    async fn archive_migration69_matches_only_the_trusted_release_original() {
        let database = super::test_database::ScratchDb::create()
            .await
            .expect("DATABASE_URL must provide the hosted disposable migration fixture");
        let original = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../migrations/0069_owner_beta_paper_restore_reconciliation.up.sql"
        ));
        let mut connection = database.pool.acquire().await.expect("fixture connects");
        assert_eq!(
            verify_archive_migration_on(&mut connection, original).await,
            Ok(())
        );
        assert_eq!(
            verify_archive_migration_on(&mut connection, b"arbitrary69").await,
            Err(ErrorCode::IdentityConflict)
        );
        sqlx::query("BEGIN")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("UPDATE public._sqlx_migrations SET checksum=decode(repeat('00',48),'hex') WHERE version=69")
            .execute(&mut *connection).await.unwrap();
        assert_eq!(
            verify_archive_migration_on(&mut connection, original).await,
            Err(ErrorCode::IdentityConflict)
        );
        sqlx::query("ROLLBACK")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        database.drop_db().await;
    }

    #[tokio::test]
    async fn archive_dataset_compressed_text_cannot_bypass_the_retained_byte_budget() {
        let database = super::test_database::ScratchDb::create()
            .await
            .expect("DATABASE_URL must provide the hosted disposable archive dataset fixture");
        let id = Uuid::new_v4();
        let hash = "a".repeat(64);
        sqlx::query(
            "INSERT INTO public.dataset_versions \
             (id,dataset_id,version,status,manifest_sha256,storage_path) \
             VALUES ($1,$2,$3,'READY',$4,'/data/curated')",
        )
        .bind(id)
        .bind(format!("archive_dataset_compressed_{}", id.simple()))
        .bind("x".repeat(128 * 1024))
        .bind(&hash)
        .execute(&database.pool)
        .await
        .expect("isolated fixture inserts compressible dataset text");
        let (logical, stored): (i32, i32) = sqlx::query_as(
            "SELECT octet_length(version),pg_column_size(version) \
             FROM public.dataset_versions WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&database.pool)
        .await
        .expect("fixture observes logical and compressed byte sizes");
        let mut budget = 4096;
        assert!(logical as usize > budget);
        assert!(stored >= 0 && stored as usize + 512 < budget);
        let references = BTreeMap::from([(
            id,
            DatasetReference {
                hash,
                version: None,
            },
        )]);
        let mut connection = database.pool.acquire().await.expect("fixture connects");
        let mut visited = 0;
        let result =
            read_archive_dataset_rows_on(&mut connection, &references, &mut budget, || {
                visited += 1;
                Ok(())
            })
            .await;
        assert_eq!(result, Err(ErrorCode::EvidenceUnavailable));
        assert_eq!(
            visited, 0,
            "oversized text must be refused before retention"
        );
        assert_eq!(budget, 4096);
        drop(connection);
        database.drop_db().await;
    }

    #[tokio::test]
    async fn archive_dataset_snapshot_reader_retains_old_row_and_fresh_reader_sees_mutation() {
        let database = super::test_database::ScratchDb::create()
            .await
            .expect("DATABASE_URL must provide the hosted disposable archive dataset fixture");
        let id = Uuid::new_v4();
        let dataset_id = format!("archive_dataset_{}", id.simple());
        let old_hash = "a".repeat(64);
        let new_hash = "b".repeat(64);
        let old_version = "preview-v1".to_owned();
        let new_version = "2026-08-11".to_owned();
        sqlx::query(
            "INSERT INTO public.dataset_versions \
             (id,dataset_id,version,status,manifest_sha256,storage_path) \
             VALUES ($1,$2,$3,'READY',$4,$5)",
        )
        .bind(id)
        .bind(&dataset_id)
        .bind(&old_version)
        .bind(&old_hash)
        .bind("fixture storage root one")
        .execute(&database.pool)
        .await
        .expect("isolated fixture dataset row inserts");

        let old_reference = BTreeMap::from([(
            id,
            DatasetReference {
                hash: old_hash.clone(),
                version: Some(old_version.clone()),
            },
        )]);
        let mut held = database
            .pool
            .acquire()
            .await
            .expect("held snapshot connection acquires");
        sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *held)
            .await
            .expect("held snapshot begins read only");
        let held_rows =
            read_archive_dataset_rows_on(&mut held, &old_reference, &mut 1_048_576, || Ok(()))
                .await
                .expect("production caller seam reads the initial row");
        assert_eq!(held_rows.len(), 1);
        assert_eq!(held_rows[0].id, id);
        assert_eq!(held_rows[0].dataset_id, dataset_id);
        assert_eq!(held_rows[0].version, old_version);
        assert_eq!(held_rows[0].status, "READY");
        assert_eq!(held_rows[0].manifest_sha256, old_hash);
        assert_eq!(held_rows[0].storage_path, "fixture storage root one");

        sqlx::query(
            "UPDATE public.dataset_versions \
             SET version=$2,status='BLOCKED',manifest_sha256=$3,storage_path=$4 \
             WHERE id=$1",
        )
        .bind(id)
        .bind(&new_version)
        .bind(&new_hash)
        .bind("fixture storage root two")
        .execute(&database.pool)
        .await
        .expect("separate fixture writer commits the row mutation");

        let held_again =
            read_archive_dataset_rows_on(&mut held, &old_reference, &mut 1_048_576, || Ok(()))
                .await
                .expect("held repeatable read keeps the original row");
        assert_eq!(held_again, held_rows);

        let missing = BTreeMap::from([(
            Uuid::new_v4(),
            DatasetReference {
                hash: old_hash.clone(),
                version: Some(old_version.clone()),
            },
        )]);
        assert_eq!(
            read_archive_dataset_rows_on(&mut held, &missing, &mut 1_048_576, || Ok(())).await,
            Err(ErrorCode::IdentityConflict),
            "required missing row must fail instead of returning a partial success"
        );
        let conflicting = BTreeMap::from([(
            id,
            DatasetReference {
                hash: new_hash.clone(),
                version: Some(new_version.clone()),
            },
        )]);
        assert_eq!(
            read_archive_dataset_rows_on(&mut held, &conflicting, &mut 1_048_576, || Ok(())).await,
            Err(ErrorCode::IdentityConflict),
            "held snapshot must reject selectors for the newer row"
        );
        sqlx::query("ROLLBACK")
            .execute(&mut *held)
            .await
            .expect("held snapshot rolls back");
        drop(held);

        let fresh_reference = BTreeMap::from([(
            id,
            DatasetReference {
                hash: new_hash.clone(),
                version: Some(new_version.clone()),
            },
        )]);
        let mut fresh = database
            .pool
            .acquire()
            .await
            .expect("fresh transaction connection acquires");
        sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *fresh)
            .await
            .expect("fresh transaction begins read only");
        let fresh_rows =
            read_archive_dataset_rows_on(&mut fresh, &fresh_reference, &mut 1_048_576, || Ok(()))
                .await
                .expect("fresh snapshot sees the fixture writer mutation");
        assert_eq!(fresh_rows.len(), 1);
        assert_eq!(fresh_rows[0].version, new_version);
        assert_eq!(fresh_rows[0].status, "BLOCKED");
        assert_eq!(fresh_rows[0].manifest_sha256, new_hash);
        assert_eq!(fresh_rows[0].storage_path, "fixture storage root two");
        sqlx::query("ROLLBACK")
            .execute(&mut *fresh)
            .await
            .expect("fresh snapshot rolls back");
        drop(fresh);
        database.drop_db().await;
    }

    #[test]
    fn receiver_raw_and_aggregate_descriptor_caps_are_independent() {
        let (source, raw) = limits();
        assert!(
            11 * raw.max_open_descriptors + source.max_members + 280 <= source.max_open_descriptors
        );
        assert_eq!(source.max_reference_bytes, 67_108_864);
        assert_eq!(raw.max_metadata_json_depth, 32);
        assert!(market_data::daily_price_only::source::ScanLimits::new(raw).is_ok());
    }
}
