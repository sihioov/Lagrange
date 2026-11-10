//! Non-serving RESTORE BEGIN intake. No dispatch or SQL mutation is made here.
//! Root/controller custody selects the owned isolated cluster. Native verifies
//! the real session and one fresh RR RO snapshot; tokens never cross clusters.
use super::*;
use market_data::checked_close::{Attempt, Gap};
use market_data::daily_price_only::source::ScanLimitValues;
use market_data::paper_archive_sources::closure::{
    ArchiveAdmissionContext, ArchiveOriginalMember, ArchiveOriginalOwner, ArchiveOriginalPaths,
    SourceClosureError, SourceClosureLimits,
};
use sqlx::{Connection, Row};
use std::{collections::BTreeMap, time::Instant};

const PACKAGE: &str = "/run/owner-beta-paper-ops/archive";
const AUDIT: &str = "/run/owner-beta-paper-ops/archive/audit.stream";
const CODEC: &str = "/run/owner-beta-paper-ops/archive/files.bundle";
const FILES: &str = "/run/owner-beta-paper-ops/archive/files";
const COORDINATION: &str = "/run/owner-beta-paper-ops/archive/coordination";
const MAX_LINE: usize = 146_800_640;

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

pub(super) async fn receive(
    inputs: &ProtectedInputs,
    intent: &Intent,
    selector: &DatabaseSelector,
    output: &GuardedDirectory,
    operation_started: Instant,
) -> ResultWire {
    let verb = Verb::Begin;
    // Exact descriptive replay remains the pre-existing Readback entrypoint;
    // an existing dispatch is never retransmitted or live-verified here.
    if bound::has_dispatch().unwrap_or(true) {
        return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    }
    let deadline = operation_started + Duration::from_secs(15);
    let attempt = Attempt::default();
    let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
        if selector.host != "/run/postgresql" || !bound::output_is_empty(output)? {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let originals = inputs.bound.as_ref().ok_or(ErrorCode::EvidenceUnavailable)?;
        let mut live = chain::ArchiveIntakeInputs::open(originals, inputs)?;
        let package = GuardedDirectory::open(PACKAGE).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        chain::archive_package_names(&package)?;
        let audit = GuardedFile::open_mode(AUDIT, 0o600).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut audit_sql = GuardedFile::open(AUDIT_SQL_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let sql_bytes = audit_sql.read_bounded(MAX_SOURCE_FILE_BYTES).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let sql = std::str::from_utf8(&sql_bytes).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let context = ArchiveAdmissionContext::open(&attempt, deadline).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let invocation = context.invocation();
        let (caps, raw_caps) = limits();
        let mut owner = ArchiveOriginalOwner::open_archive_in(&invocation, &attempt, &attempt,
            ArchiveOriginalPaths { files_bundle: Path::new(CODEC), database_stream: Path::new(AUDIT),
                materialized: Path::new(FILES), raw_coordination: Path::new(COORDINATION) },
            caps, raw_caps, || Instant::now() < deadline, |_| Ok(()))
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        // Source rows only select real reads on the held clone snapshot.
        let (_, used_bytes) = owner.reference_usage().map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut budget = caps.max_reference_bytes.checked_sub(used_bytes).ok_or(ErrorCode::EvidenceUnavailable)?;
        let mut datasets = BTreeMap::new();
        owner.visit_reference_rows(|| Instant::now() < deadline, |member| {
            if let ArchiveOriginalMember::Reference { table, canonical_row, .. } = member {
                if let Some((id, row)) = dataset_reference(table, canonical_row)? {
                    retain_dataset_reference(&mut datasets, &mut budget, id, row)?;
                }
            }
            Ok(())
        }).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let pool = connect(selector, DatabaseRole::MigrationOwner).await?;
        let connection = pool.acquire().await.map_err(|_| ErrorCode::EvidenceUnavailable)?.detach();
        pool.close().await;
        let mut database = ArchiveConnection { connection: Some(connection), attempt: &attempt };
        let body = async {
            let connection = database.connection()?;
            sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL statement_timeout='15s'; SET LOCAL lock_timeout='5s'; SET LOCAL search_path='pg_catalog, pg_temp';")
                .execute(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let actual: bool = sqlx::query_scalar("SELECT current_user='migration_owner' AND session_user=current_user AND current_setting('role')='none' AND current_setting('transaction_isolation')='repeatable read' AND current_setting('transaction_read_only')='on' AND current_setting('server_encoding')='UTF8' AND current_setting('client_encoding')='UTF8' AND current_setting('search_path')='pg_catalog, pg_temp' AND current_setting('statement_timeout')='15s' AND current_setting('lock_timeout')='5s'")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if !actual { return Err(ErrorCode::EvidenceUnavailable); }
            let actor_before: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            compare_projection(connection, &audit, sql, deadline).await?;
            let actor_after: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if actor_before.as_deref().unwrap_or("") != actor_after.as_deref().unwrap_or("") {
                return Err(ErrorCode::IdentityConflict);
            }
            // Audit SQL independently verifies real role flags, RLS, schema,
            // migrations, full nine-group rows, actor restore and physical EOF.
            for (id, reference) in &datasets {
                let bytes: Option<i64> = sqlx::query_scalar("SELECT (pg_column_size(dataset_id)::bigint+pg_column_size(version)+pg_column_size(status)+pg_column_size(manifest_sha256)+pg_column_size(storage_path)+256) FROM public.dataset_versions WHERE id=$1")
                    .bind(id).fetch_optional(&mut *connection).await.map_err(|_| ErrorCode::EvidenceUnavailable)?;
                if !bytes.is_some_and(|n| n >= 0 && n as u64 <= budget as u64) {
                    return Err(ErrorCode::EvidenceUnavailable);
                }
                let row = crate::recommendation::input::read_dataset_row_on(connection, *id).await
                    .map_err(|_| ErrorCode::EvidenceUnavailable)?.ok_or(ErrorCode::IdentityConflict)?;
                if row.id != *id || row.manifest_sha256 != reference.hash
                    || reference.version.as_ref().is_some_and(|v| v != &row.version) {
                    return Err(ErrorCode::IdentityConflict);
                }
                live.revalidate(originals, inputs)?;
                owner.revalidate(&mut || Instant::now() < deadline).map_err(|_| ErrorCode::EvidenceUnavailable)?;
            }
            // There is still no whole-history Paper/R4 completion certificate.
            // The actual released reader runs, but its final closed guard stays.
            owner.visit_sources(|| Instant::now() < deadline, |_| Ok(()))
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            Err::<(), ErrorCode>(ErrorCode::EvidenceUnavailable)
        }.await;
        // Recheck independent originals even on a domain rejection, then close
        // every owner. No error or successful local close publishes an OK.
        let live_after = live.revalidate(originals, inputs);
        let package_after = chain::archive_package_names(&package);
        let sql_after = audit_sql.revalidate();
        let database_closed = database.finish().await;
        let files_closed = owner.finish_local(|| Instant::now() < deadline);
        // The consumed owner ends the private invocation borrow here.
        let context_closed = context.finish_local();
        if !database_closed || files_closed.is_err() || context_closed.is_err()
            || live_after.is_err() || package_after.is_err() || sql_after.is_err() {
            attempt.gap(Gap::Abandoned);
        }
        body
    }).await;
    let code = match result {
        Ok(Err(code)) => code,
        _ => ErrorCode::EvidenceUnavailable,
    };
    let _terminal = attempt.finish(Err::<(), _>(code));
    let _ = intent; // immutable R is consumed by ArchiveIntakeInputs above.
    ResultWire::new(verb, Outcome::Blocked, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic selectors only. These tests establish no database, Origin,
    // Current, custody or archive-completion authority.
    fn row(hash: &str, version: Option<&str>) -> DatasetReference {
        DatasetReference {
            hash: hash.into(),
            version: version.map(str::to_owned),
        }
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
