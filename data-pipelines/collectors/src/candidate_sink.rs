//! Atomic PostgreSQL publication for provider-neutral candidate-source documents.
//!
//! Raw bytes are ingested by `market-data`; this boundary publishes only a
//! validated, immutable document tied to one exact curated dataset pin.  It is
//! intentionally independent of a concrete KRX credential or transport.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AssetClass, ContentHash, Currency, InstrumentStatus, TradingDate, UtcTimestamp, Venue,
};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CandidateDocument, CandidateSourcePin, FinancialPeriodKind,
    FundamentalObservation, FundamentalProfile, IndexMembershipObservation, IngestOutcome,
    InstrumentMaster, InvestorClass, InvestorFlowObservation, MarketStatusObservation,
    PriceCurationEvidence, RawEnvelope, ResponseKind, SectorObservation, StatementScope,
    members_as_of, parse_candidate_envelope, sectors_as_of, validate_candidate_document,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{PublishOutcome, SinkError, candidate_pipeline::CandidateDatasetBinding};

const UNIVERSE_DATASET: &str = "krx_kospi200_membership";
const FLOW_DATASET: &str = "krx_investor_flows";
const STATUS_DATASET: &str = "krx_market_status";
const FUNDAMENTAL_DATASET: &str = "krx_fundamentals";
const SECTOR_DATASET: &str = "krx_sector_classification";

/// One curated candidate-source document and the PIT instant used when a
/// point-in-time snapshot must be materialized.
pub struct CandidateSourcePublication<'a> {
    pub raw_batch_id: Uuid,
    pub raw_manifest_sha256: &'a str,
    pub fetch_mode: market_data::FetchMode,
    pub dataset_version_id: Uuid,
    pub as_of: TradingDate,
    pub cutoff_at: UtcTimestamp,
    pub pin: &'a CandidateSourcePin,
    pub document: &'a CandidateDocument,
}

pub struct CandidatePricePublication<'a> {
    pub raw_batch_id: Uuid,
    pub raw_manifest_sha256: &'a str,
    pub fetch_mode: market_data::FetchMode,
    /// Exact delivery date recorded by the immutable Raw manifest. This is
    /// deliberately separate from the first/last price-session rights window.
    pub entitlement_date: TradingDate,
    pub evidence: &'a PriceCurationEvidence,
    pub dataset_version: &'a str,
    pub storage_path: &'a str,
    pub provider: &'a str,
    pub entitlement_id: Uuid,
    pub license_ref: &'a str,
    pub available_at: UtcTimestamp,
    pub retrieved_at: UtcTimestamp,
}

pub struct CandidateInstrumentCatalog<'a> {
    pub master: &'a InstrumentMaster,
    pub entitlement_id: Uuid,
    pub contract_reference: &'a str,
    pub entitlement_date: TradingDate,
    pub reference_sha256: &'a str,
    pub source_revision: &'a str,
    pub retrieved_at: UtcTimestamp,
}

#[derive(Clone)]
pub struct PostgresCandidateSourceSink {
    pool: PgPool,
}

#[derive(FromRow)]
struct PricePublishRow {
    dataset_version_id: Uuid,
    published: bool,
}

impl PostgresCandidateSourceSink {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn raw_batch_is_terminal(
        &self,
        entry: &market_data::ManifestEntry,
        surface: &str,
    ) -> Result<bool, SinkError> {
        let row: Option<(String, String, String, String, NaiveDate)> = sqlx::query_as(
            "SELECT state,raw_manifest_sha256,fetch_mode,entitlement_reference,entitlement_date
               FROM candidate_raw_batch_publications
              WHERE batch_id=$1 AND surface=$2",
        )
        .bind(entry.batch_id.as_uuid())
        .bind(surface)
        .fetch_optional(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)?;
        let Some((state, raw_hash, fetch_mode, contract_reference, entitlement_date)) = row else {
            return Ok(false);
        };
        let expected_contract = entry.entitlement_reference.as_deref().unwrap_or("");
        if raw_hash != candidate_raw_manifest_sha256(entry)?
            || fetch_mode != entry.mode.as_str()
            || contract_reference != expected_contract
            || entitlement_date != date(entry.date)
        {
            return Err(SinkError::Conflict(
                "candidate Raw batch identity differs from its durable publication ledger"
                    .to_owned(),
            ));
        }
        Ok(matches!(state.as_str(), "PUBLISHED" | "BLOCKED"))
    }

    pub async fn block_raw_batch_for_inactive_rights(
        &self,
        entry: &market_data::ManifestEntry,
        surface: &str,
        first_date: TradingDate,
        last_date: TradingDate,
    ) -> Result<(), SinkError> {
        let contract_reference = entry
            .entitlement_reference
            .as_deref()
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                SinkError::Invariant(
                    "candidate Raw terminal block requires its original entitlement".to_owned(),
                )
            })?;
        if last_date < first_date || entry.date < first_date || entry.date > last_date {
            return Err(SinkError::Invariant(
                "candidate Raw terminal block rights window is invalid".to_owned(),
            ));
        }
        sqlx::query(
            "SELECT public.block_candidate_raw_batch_for_inactive_rights(
                $1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(entry.batch_id.as_uuid())
        .bind(surface)
        .bind(candidate_raw_manifest_sha256(entry)?)
        .bind(entry.mode.as_str())
        .bind(contract_reference)
        .bind(date(entry.date))
        .bind(date(first_date))
        .bind(date(last_date))
        .execute(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)?;
        Ok(())
    }

    /// Durable recovery proof for a Raw candidate delivery. Daily flow and
    /// status are mandatory in every operating batch and are published in one
    /// transaction; requiring both exact batch-derived catalog pins prevents a
    /// catalog-only crash from being mistaken for completed publication.
    pub async fn candidate_batch_is_published(&self, batch_id: Uuid) -> Result<bool, SinkError> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM candidate_raw_batch_publications AS batch
                 WHERE batch.batch_id=$1 AND batch.surface='source'
                   AND batch.state='PUBLISHED')",
        )
        .bind(batch_id)
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)
    }

    pub async fn price_batch_is_published(&self, batch_id: Uuid) -> Result<bool, SinkError> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM candidate_raw_batch_publications AS batch
                 WHERE batch.batch_id=$1 AND batch.surface='price'
                   AND batch.state='PUBLISHED')",
        )
        .bind(batch_id)
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)
    }

    pub async fn resolve_contract_entitlement(
        &self,
        contract_reference: &str,
        first_session: TradingDate,
        last_session: TradingDate,
    ) -> Result<Uuid, SinkError> {
        if contract_reference.trim().is_empty() || last_session < first_session {
            return Err(SinkError::Invariant(
                "candidate entitlement scope is invalid".to_owned(),
            ));
        }
        sqlx::query_scalar("SELECT public.resolve_candidate_contract_entitlement($1, $2, $3)")
            .bind(contract_reference)
            .bind(date(first_session))
            .bind(date(last_session))
            .fetch_one(&self.pool)
            .await
            .map_err(SinkError::from_sqlx)
    }

    pub async fn register_candidate_instruments(
        &self,
        catalog: &CandidateInstrumentCatalog<'_>,
    ) -> Result<usize, SinkError> {
        if catalog.entitlement_id.is_nil()
            || catalog.contract_reference.trim().is_empty()
            || catalog.reference_sha256.len() != 64
            || !catalog
                .reference_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || catalog.source_revision.trim().is_empty()
        {
            return Err(SinkError::Invariant(
                "candidate instrument catalog evidence is invalid".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(SinkError::from_sqlx)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        let mut inserted = 0usize;
        for instrument in catalog.master.instruments() {
            if instrument.venue != Venue::Krx
                || instrument.currency != Currency::KRW
                || instrument.status != InstrumentStatus::Listed
            {
                return Err(SinkError::Invariant(
                    "candidate instrument catalog accepts active KRX/KRW records only".to_owned(),
                ));
            }
            let asset_class = match instrument.asset_class {
                AssetClass::Etf => "ETF",
                AssetClass::Equity => "EQUITY",
                _ => {
                    return Err(SinkError::Invariant(
                        "candidate instrument catalog accepts ETF/equity records only".to_owned(),
                    ));
                }
            };
            let published: bool = sqlx::query_scalar(
                "SELECT public.register_candidate_instrument(
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11
                )",
            )
            .bind(instrument.instrument_id.to_string())
            .bind(instrument.instrument_id.symbol())
            .bind(&instrument.name)
            .bind(asset_class)
            .bind(date(instrument.listed_at))
            .bind(catalog.entitlement_id)
            .bind(catalog.contract_reference)
            .bind(date(catalog.entitlement_date))
            .bind(catalog.reference_sha256)
            .bind(catalog.source_revision)
            .bind(timestamp(catalog.retrieved_at))
            .fetch_one(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            inserted += usize::from(published);
        }
        tx.commit().await.map_err(SinkError::from_sqlx)?;
        Ok(inserted)
    }

    /// Register the exact five immutable Raw source artifacts as curated
    /// catalog identities. The underlying procedure accepts only those five
    /// dataset ids; `research_writer` has no broad dataset_versions DML.
    pub async fn catalog_candidate_batch(
        &self,
        outcome: &IngestOutcome,
    ) -> Result<Vec<CandidateDatasetBinding>, SinkError> {
        let contract_reference = outcome
            .entry
            .entitlement_reference
            .as_deref()
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                SinkError::Invariant(
                    "candidate Raw catalog requires an exact entitlement reference".to_owned(),
                )
            })?;
        let (rights_first_date, rights_last_date) = candidate_source_rights_window(outcome)?;
        let entitlement_id = self
            .resolve_contract_entitlement(contract_reference, rights_first_date, rights_last_date)
            .await?;
        let mut tx = self.pool.begin().await.map_err(SinkError::from_sqlx)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        let raw_manifest_sha256 = candidate_raw_manifest_sha256(&outcome.entry)?;
        sqlx::query("SELECT public.begin_candidate_raw_batch($1,'source',$2,$3,$4,$5)")
            .bind(outcome.batch_id.as_uuid())
            .bind(&raw_manifest_sha256)
            .bind(outcome.entry.mode.as_str())
            .bind(contract_reference)
            .bind(date(outcome.entry.date))
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        let requested = outcome
            .entry
            .files
            .iter()
            .map(|file| file.kind)
            .collect::<BTreeSet<_>>();
        if requested.is_empty()
            || requested
                .iter()
                .any(|kind| !CANDIDATE_RESPONSE_KINDS.contains(kind))
        {
            return Err(SinkError::Invariant(
                "candidate Raw catalog contains no supported source kind".to_owned(),
            ));
        }
        let mut bindings = Vec::with_capacity(requested.len());
        for kind in requested {
            let dataset_id = dataset_for_kind(kind).ok_or_else(|| {
                SinkError::Invariant(format!("unsupported candidate source kind {kind}"))
            })?;
            let mut files = outcome
                .entry
                .files
                .iter()
                .filter(|file| file.kind == kind)
                .collect::<Vec<_>>();
            files.sort_by(|left, right| left.file_name.cmp(&right.file_name));
            if files.is_empty() {
                return Err(SinkError::Invariant(format!(
                    "candidate Raw catalog is missing {kind}"
                )));
            }
            let canonical = files
                .iter()
                .map(|file| {
                    (
                        file.file_name.as_str(),
                        file.content_hash.as_str(),
                        file.size_bytes,
                    )
                })
                .collect::<Vec<_>>();
            let canonical = serde_json::to_vec(&canonical).map_err(|error| {
                SinkError::Invariant(format!("candidate catalog serialization failed: {error}"))
            })?;
            let manifest_sha256 = ContentHash::from_bytes(&canonical)
                .as_str()
                .strip_prefix("sha256:")
                .expect("content hashes have a sha256 prefix")
                .to_owned();
            if matches!(
                kind,
                ResponseKind::Fundamentals
                    | ResponseKind::IndexMembership
                    | ResponseKind::SectorClassification
            ) && let Some(existing) = reusable_pit_binding(
                &mut tx,
                kind,
                &manifest_sha256,
                outcome.entry.date,
                entitlement_id,
                contract_reference,
                outcome.entry.mode,
            )
            .await?
            {
                if existing.5 != manifest_sha256 || !existing.6 {
                    return Err(SinkError::Conflict(format!(
                        "candidate {kind} Raw manifest is occupied by different rights"
                    )));
                }
                sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source',$2,$3,true)")
                    .bind(outcome.batch_id.as_uuid())
                    .bind(kind.as_str())
                    .bind(existing.0)
                    .execute(&mut *tx)
                    .await
                    .map_err(SinkError::from_sqlx)?;
                bindings.push(CandidateDatasetBinding {
                    kind,
                    dataset_version_id: existing.0,
                    entitlement_id: existing.1,
                    license_ref: existing.2,
                    dataset_id: dataset_id.to_owned(),
                    dataset_version: existing.4,
                    manifest_sha256: existing.5,
                    reused_existing: true,
                });
                continue;
            }
            let dataset_version = format!(
                "{}:{}:{}",
                outcome.entry.date.to_iso(),
                outcome.batch_id,
                kind.as_str()
            );
            let dataset_version_id: Uuid = sqlx::query_scalar(
                "SELECT public.register_candidate_source_dataset($1, $2, $3, $4, $5, $6)",
            )
            .bind(dataset_id)
            .bind(&dataset_version)
            .bind(&manifest_sha256)
            .bind(entitlement_id)
            .bind(contract_reference)
            .bind(date(outcome.entry.date))
            .fetch_one(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source',$2,$3,false)")
                .bind(outcome.batch_id.as_uuid())
                .bind(kind.as_str())
                .bind(dataset_version_id)
                .execute(&mut *tx)
                .await
                .map_err(SinkError::from_sqlx)?;
            bindings.push(CandidateDatasetBinding {
                kind,
                dataset_version_id,
                entitlement_id,
                license_ref: contract_reference.to_owned(),
                dataset_id: dataset_id.to_owned(),
                dataset_version,
                manifest_sha256,
                reused_existing: false,
            });
        }
        tx.commit().await.map_err(SinkError::from_sqlx)?;
        Ok(bindings)
    }

    pub async fn publish_price(
        &self,
        publication: &CandidatePricePublication<'_>,
    ) -> Result<(Uuid, PublishOutcome), SinkError> {
        if publication.dataset_version.trim().is_empty()
            || publication.storage_path.trim().is_empty()
            || publication.provider.trim().is_empty()
            || publication.license_ref.trim().is_empty()
            || publication.entitlement_id.is_nil()
            || publication.raw_batch_id.is_nil()
            || publication.raw_manifest_sha256.len() != 64
            || publication.evidence.curated_generation == 0
            || publication.evidence.manifest_sha256.len() != 64
            || publication.evidence.last_session < publication.evidence.first_session
            || publication.evidence.instrument_coverage.is_empty()
            || publication.available_at > publication.retrieved_at
        {
            return Err(SinkError::Invariant(
                "candidate price publication is invalid".to_owned(),
            ));
        }
        let mut instrument_ids = BTreeSet::new();
        for coverage in &publication.evidence.instrument_coverage {
            let sessions = coverage.sessions.iter().copied().collect::<BTreeSet<_>>();
            if coverage.instrument_id.trim().is_empty()
                || coverage.session_count == 0
                || usize::try_from(coverage.session_count).ok() != Some(coverage.sessions.len())
                || sessions.len() != coverage.sessions.len()
                || sessions.first().copied() != Some(coverage.first_session)
                || sessions.last().copied() != Some(coverage.last_session)
                || coverage.first_session < publication.evidence.first_session
                || coverage.last_session > publication.evidence.last_session
                || coverage.last_session < coverage.first_session
                || !instrument_ids.insert(&coverage.instrument_id)
            {
                return Err(SinkError::Invariant(
                    "candidate price instrument coverage is invalid".to_owned(),
                ));
            }
        }
        let coverage_json = serde_json::to_value(&publication.evidence.instrument_coverage)
            .map_err(|error| {
                SinkError::Invariant(format!(
                    "candidate price instrument coverage is not serializable: {error}"
                ))
            })?;
        let row: PricePublishRow = sqlx::query_as(
            "SELECT dataset_version_id, published
               FROM public.publish_candidate_price_publication(
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17
               )",
        )
        .bind(publication.dataset_version)
        .bind(&publication.evidence.manifest_sha256)
        .bind(publication.storage_path)
        .bind(i64::from(publication.evidence.curated_generation))
        .bind(date(publication.evidence.first_session))
        .bind(date(publication.evidence.last_session))
        .bind(coverage_json)
        .bind(publication.provider)
        .bind(publication.entitlement_id)
        .bind(publication.license_ref)
        .bind(&publication.evidence.source_revision)
        .bind(publication.raw_batch_id)
        .bind(publication.raw_manifest_sha256)
        .bind(publication.fetch_mode.as_str())
        .bind(date(publication.entitlement_date))
        .bind(timestamp(publication.available_at))
        .bind(timestamp(publication.retrieved_at))
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)?;
        Ok((
            row.dataset_version_id,
            if row.published {
                PublishOutcome::Published
            } else {
                PublishOutcome::AlreadyPublished
            },
        ))
    }

    pub async fn has_price(&self, as_of: TradingDate) -> Result<bool, SinkError> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM candidate_price_publications
                 WHERE first_session <= $1 AND last_session >= $1
                   AND public.candidate_source_entitlement_is_valid(
                       entitlement_id, license_ref, 'krx_eod_bars',
                       first_session, last_session)
            )",
        )
        .bind(date(as_of))
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)
    }

    pub async fn has_complete_sources(
        &self,
        as_of: TradingDate,
        expected_fetch_mode: market_data::FetchMode,
    ) -> Result<bool, SinkError> {
        let cutoff = chrono::DateTime::from_naive_utc_and_offset(
            as_of
                .as_naive_date()
                .and_hms_opt(23, 59, 59)
                .expect("a date has a final second"),
            Utc,
        );
        Ok(self
            .missing_source_kinds(as_of, cutoff, expected_fetch_mode)
            .await?
            .is_empty())
    }

    pub async fn missing_source_kinds(
        &self,
        as_of: TradingDate,
        cutoff_at: DateTime<Utc>,
        expected_fetch_mode: market_data::FetchMode,
    ) -> Result<Vec<ResponseKind>, SinkError> {
        let present: (bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT
                EXISTS (SELECT 1 FROM candidate_universe_snapshots AS universe
                         WHERE universe.as_of_date <= $1
                           AND universe.available_at <= $2
                           AND universe.member_count = (
                               SELECT count(*) FROM candidate_universe_members AS member
                                WHERE member.universe_snapshot_id = universe.id
                                  AND member.effective_from <= $1
                                  AND (member.effective_until IS NULL OR member.effective_until >= $1))
                           AND public.candidate_source_entitlement_is_valid(
                               universe.entitlement_id, universe.license_ref,
                               'krx_kospi200_membership', $1, $1)
                           AND EXISTS (SELECT 1 FROM candidate_raw_batch_datasets AS binding
                               JOIN candidate_raw_batch_publications AS batch
                                 ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                              WHERE binding.dataset_version_id=universe.dataset_version_id
                                AND binding.response_kind='index_membership'
                                AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)),
                EXISTS (SELECT 1 FROM candidate_investor_flows AS flow
                         JOIN candidate_investor_flow_snapshot_rows AS member
                           ON member.flow_observation_id=flow.id
                         WHERE flow.trade_date=$1 AND member.entitlement_date=$1
                           AND flow.available_at <= $2
                           AND public.candidate_source_entitlement_is_valid(
                               member.entitlement_id, member.license_ref,
                               'krx_investor_flows', $1, $1)
                           AND EXISTS (SELECT 1 FROM candidate_raw_batch_datasets AS binding
                               JOIN candidate_raw_batch_publications AS batch
                                 ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                              WHERE binding.dataset_version_id=member.dataset_version_id
                                AND binding.response_kind='investor_flow'
                                AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)),
                EXISTS (SELECT 1 FROM candidate_market_status_observations AS status
                         WHERE status.trade_date=$1 AND status.entitlement_date=$1
                           AND status.available_at <= $2
                           AND public.candidate_source_entitlement_is_valid(
                               status.entitlement_id, status.license_ref,
                               'krx_market_status', $1, $1)
                           AND EXISTS (SELECT 1 FROM candidate_raw_batch_datasets AS binding
                               JOIN candidate_raw_batch_publications AS batch
                                 ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                              WHERE binding.dataset_version_id=status.dataset_version_id
                                AND binding.response_kind='market_status'
                                AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)),
                EXISTS (SELECT 1 FROM candidate_fundamental_observations AS fact
                         WHERE fact.fiscal_period_end <= $1
                           AND fact.available_at <= $2
                           AND public.candidate_source_entitlement_is_valid(
                               fact.entitlement_id, fact.license_ref,
                               'krx_fundamentals', $1, $1)
                           AND EXISTS (SELECT 1 FROM candidate_raw_batch_datasets AS binding
                               JOIN candidate_raw_batch_publications AS batch
                                 ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                              WHERE binding.dataset_version_id=fact.dataset_version_id
                                AND binding.response_kind='fundamentals'
                                AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)),
                EXISTS (SELECT 1 FROM candidate_sector_versions AS sector
                         WHERE sector.effective_from <= $1
                           AND sector.available_at <= $2
                           AND public.candidate_source_entitlement_is_valid(
                               sector.entitlement_id, sector.license_ref,
                               'krx_sector_classification', $1, $1)
                           AND EXISTS (SELECT 1 FROM candidate_raw_batch_datasets AS binding
                               JOIN candidate_raw_batch_publications AS batch
                                 ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                              WHERE binding.dataset_version_id=sector.dataset_version_id
                                AND binding.response_kind='sector_classification'
                                AND batch.state='PUBLISHED' AND batch.fetch_mode=$3))",
        )
        .bind(date(as_of))
        .bind(cutoff_at)
        .bind(expected_fetch_mode.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)?;
        let mut missing = Vec::new();
        for (kind, is_present) in [
            (ResponseKind::IndexMembership, present.0),
            (ResponseKind::InvestorFlow, present.1),
            (ResponseKind::MarketStatus, present.2),
            (ResponseKind::Fundamentals, present.3),
            (ResponseKind::SectorClassification, present.4),
        ] {
            if !is_present {
                missing.push(kind);
            }
        }
        Ok(missing)
    }

    /// Publish one document atomically. Exact replay is accepted; a natural
    /// identity occupied by different bytes or provenance is rejected.
    pub async fn publish(
        &self,
        publication: &CandidateSourcePublication<'_>,
    ) -> Result<PublishOutcome, SinkError> {
        self.publish_batch(std::slice::from_ref(publication)).await
    }

    /// Publish a coherent provider delivery in one database transaction.
    /// This prevents a five-document candidate batch from becoming partially
    /// visible when any one dataset pin or source row is rejected.
    pub async fn publish_batch(
        &self,
        publications: &[CandidateSourcePublication<'_>],
    ) -> Result<PublishOutcome, SinkError> {
        if publications.is_empty() {
            return Err(SinkError::Invariant(
                "candidate publication batch must not be empty".to_owned(),
            ));
        }
        let mut datasets = BTreeSet::new();
        let first = &publications[0];
        for publication in publications {
            validate_publication(publication)?;
            if publication.raw_batch_id != first.raw_batch_id
                || publication.raw_manifest_sha256 != first.raw_manifest_sha256
                || publication.fetch_mode != first.fetch_mode
                || publication.as_of != first.as_of
                || publication.pin.license_ref != first.pin.license_ref
            {
                return Err(SinkError::Invariant(
                    "candidate publication batch must share one exact Raw identity".to_owned(),
                ));
            }
            if !datasets.insert(expected_dataset(publication.document)) {
                return Err(SinkError::Invariant(
                    "candidate publication batch contains a duplicate dataset".to_owned(),
                ));
            }
        }
        let mut tx = self.pool.begin().await.map_err(SinkError::from_sqlx)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        let terminal: Option<(String, String, String, String, NaiveDate)> = sqlx::query_as(
            "SELECT state,raw_manifest_sha256,fetch_mode,entitlement_reference,entitlement_date
               FROM candidate_raw_batch_publications
              WHERE batch_id=$1 AND surface='source'",
        )
        .bind(first.raw_batch_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if let Some((state, raw_hash, fetch_mode, contract_reference, entitlement_date)) =
            terminal.as_ref()
            && state == "PUBLISHED"
        {
            let identity_matches = raw_hash == first.raw_manifest_sha256
                && fetch_mode == first.fetch_mode.as_str()
                && contract_reference.as_str() == first.pin.license_ref
                && *entitlement_date == date(first.as_of);
            let exact_bindings: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM candidate_raw_batch_datasets AS binding
                  WHERE binding.batch_id=$1 AND binding.surface='source'
                    AND (binding.response_kind,binding.dataset_version_id) IN (
                        SELECT * FROM unnest($2::text[],$3::uuid[]))",
            )
            .bind(first.raw_batch_id)
            .bind(
                publications
                    .iter()
                    .map(|publication| response_kind_for_document(publication.document).as_str())
                    .collect::<Vec<_>>(),
            )
            .bind(
                publications
                    .iter()
                    .map(|publication| publication.dataset_version_id)
                    .collect::<Vec<_>>(),
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            if !identity_matches || usize::try_from(exact_bindings).ok() != Some(publications.len())
            {
                return Err(SinkError::Conflict(
                    "candidate published Raw batch replay differs from its sealed identity"
                        .to_owned(),
                ));
            }
            tx.commit().await.map_err(SinkError::from_sqlx)?;
            return Ok(PublishOutcome::AlreadyPublished);
        }
        if terminal.is_some_and(|row| row.0 == "BLOCKED") {
            return Err(SinkError::Conflict(
                "candidate Raw batch is terminally blocked".to_owned(),
            ));
        }
        sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
            .bind(first.raw_batch_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        let mut inserted = false;
        for publication in publications {
            attest_dataset(&mut tx, publication).await?;
            inserted |= match publication.document {
                CandidateDocument::InvestorFlow(document) => {
                    publish_flows(&mut tx, publication, &document.flows).await?
                }
                CandidateDocument::MarketStatus(document) => {
                    publish_statuses(&mut tx, publication, &document.statuses).await?
                }
                CandidateDocument::Fundamentals(document) => {
                    publish_fundamentals(&mut tx, publication, &document.fundamentals).await?
                }
                CandidateDocument::IndexMembership(document) => {
                    publish_universe(&mut tx, publication, &document.memberships).await?
                }
                CandidateDocument::SectorClassification(document) => {
                    publish_sectors(&mut tx, publication, &document.sectors).await?
                }
            };
        }
        sqlx::query("SELECT public.seal_candidate_raw_batch($1,'source',$2,$3)")
            .bind(first.raw_batch_id)
            .bind(first.raw_manifest_sha256)
            .bind(first.fetch_mode.as_str())
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        tx.commit().await.map_err(SinkError::from_sqlx)?;
        Ok(if inserted {
            PublishOutcome::Published
        } else {
            PublishOutcome::AlreadyPublished
        })
    }
}

pub fn candidate_raw_manifest_sha256(
    entry: &market_data::ManifestEntry,
) -> Result<String, SinkError> {
    let canonical = serde_json::to_vec(entry).map_err(|error| {
        SinkError::Invariant(format!(
            "candidate Raw manifest serialization failed: {error}"
        ))
    })?;
    Ok(ContentHash::from_bytes(&canonical)
        .as_str()
        .strip_prefix("sha256:")
        .expect("content hashes have a sha256 prefix")
        .to_owned())
}

/// Candidate source rights must cover the full rolling flow window, not only
/// the delivery date. PIT reference sources remain licensed at the as-of date.
pub fn candidate_source_rights_window(
    outcome: &IngestOutcome,
) -> Result<(TradingDate, TradingDate), SinkError> {
    let mut first_date = outcome.entry.date;
    let mut saw_flow = false;
    for metadata in outcome
        .entry
        .files
        .iter()
        .filter(|file| file.kind == ResponseKind::InvestorFlow)
    {
        let stored = outcome
            .files
            .iter()
            .find(|file| file.file_name == metadata.file_name)
            .ok_or_else(|| {
                SinkError::Invariant(
                    "candidate flow rights window requires exact Raw read-back".to_owned(),
                )
            })?;
        let envelope = RawEnvelope::new(
            outcome.batch_id,
            metadata.kind,
            metadata.file_name.clone(),
            stored.bytes.clone(),
            outcome.entry.retrieved_at,
            metadata.request.clone(),
        );
        if envelope.content_hash != metadata.content_hash {
            return Err(SinkError::Invariant(
                "candidate flow rights window Raw hash differs from its manifest".to_owned(),
            ));
        }
        let document = parse_candidate_envelope(&envelope).map_err(|error| {
            SinkError::Invariant(format!(
                "candidate flow rights window is not typed Raw: {error}"
            ))
        })?;
        let CandidateDocument::InvestorFlow(document) = document else {
            return Err(SinkError::Invariant(
                "candidate flow rights window parsed the wrong response kind".to_owned(),
            ));
        };
        for flow in document.flows {
            if flow.trade_date > outcome.entry.date {
                return Err(SinkError::Invariant(
                    "candidate flow rights window contains a future observation".to_owned(),
                ));
            }
            first_date = first_date.min(flow.trade_date);
            saw_flow = true;
        }
    }
    if !saw_flow {
        return Err(SinkError::Invariant(
            "candidate source Raw has no investor-flow rights window".to_owned(),
        ));
    }
    Ok((first_date, outcome.entry.date))
}

type ReusablePitBinding = (Uuid, Uuid, String, String, String, String, bool);

async fn reusable_pit_binding(
    tx: &mut Transaction<'_, Postgres>,
    kind: ResponseKind,
    manifest_sha256: &str,
    as_of: TradingDate,
    entitlement_id: Uuid,
    contract_reference: &str,
    fetch_mode: market_data::FetchMode,
) -> Result<Option<ReusablePitBinding>, SinkError> {
    let (query, response_kind) = match kind {
        ResponseKind::IndexMembership => (
            "SELECT DISTINCT dataset.id, source.entitlement_id, source.license_ref,
                    dataset.dataset_id, dataset.version, dataset.manifest_sha256,
                    public.candidate_source_entitlement_is_valid(
                        source.entitlement_id, source.license_ref, dataset.dataset_id, $2, $2)
               FROM candidate_universe_snapshots AS source
               JOIN dataset_versions AS dataset ON dataset.id=source.dataset_version_id
               JOIN candidate_raw_batch_datasets AS origin
                 ON origin.dataset_version_id=dataset.id AND NOT origin.reused_existing
               JOIN candidate_raw_batch_publications AS origin_batch
                 ON origin_batch.batch_id=origin.batch_id AND origin_batch.surface=origin.surface
              WHERE dataset.manifest_sha256=$1 AND source.entitlement_id=$3
                AND source.license_ref=$4 AND origin.response_kind=$5
                AND origin_batch.state='PUBLISHED' AND origin_batch.fetch_mode=$6",
            "index_membership",
        ),
        ResponseKind::Fundamentals => (
            "SELECT DISTINCT dataset.id, source.entitlement_id, source.license_ref,
                    dataset.dataset_id, dataset.version, dataset.manifest_sha256,
                    public.candidate_source_entitlement_is_valid(
                        source.entitlement_id, source.license_ref, dataset.dataset_id, $2, $2)
               FROM candidate_fundamental_observations AS source
               JOIN dataset_versions AS dataset ON dataset.id=source.dataset_version_id
               JOIN candidate_raw_batch_datasets AS origin
                 ON origin.dataset_version_id=dataset.id AND NOT origin.reused_existing
               JOIN candidate_raw_batch_publications AS origin_batch
                 ON origin_batch.batch_id=origin.batch_id AND origin_batch.surface=origin.surface
              WHERE dataset.manifest_sha256=$1 AND source.entitlement_id=$3
                AND source.license_ref=$4 AND origin.response_kind=$5
                AND origin_batch.state='PUBLISHED' AND origin_batch.fetch_mode=$6",
            "fundamentals",
        ),
        ResponseKind::SectorClassification => (
            "SELECT DISTINCT dataset.id, source.entitlement_id, source.license_ref,
                    dataset.dataset_id, dataset.version, dataset.manifest_sha256,
                    public.candidate_source_entitlement_is_valid(
                        source.entitlement_id, source.license_ref, dataset.dataset_id, $2, $2)
               FROM candidate_sector_versions AS source
               JOIN dataset_versions AS dataset ON dataset.id=source.dataset_version_id
               JOIN candidate_raw_batch_datasets AS origin
                 ON origin.dataset_version_id=dataset.id AND NOT origin.reused_existing
               JOIN candidate_raw_batch_publications AS origin_batch
                 ON origin_batch.batch_id=origin.batch_id AND origin_batch.surface=origin.surface
              WHERE dataset.manifest_sha256=$1 AND source.entitlement_id=$3
                AND source.license_ref=$4 AND origin.response_kind=$5
                AND origin_batch.state='PUBLISHED' AND origin_batch.fetch_mode=$6",
            "sector_classification",
        ),
        _ => return Ok(None),
    };
    let rows = sqlx::query_as::<_, ReusablePitBinding>(query)
        .bind(manifest_sha256)
        .bind(date(as_of))
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(response_kind)
        .bind(fetch_mode.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
    match rows.len() {
        0 => Ok(None),
        1 => Ok(rows.into_iter().next()),
        _ => Err(SinkError::Conflict(format!(
            "candidate {kind} Raw manifest resolves to multiple immutable pins"
        ))),
    }
}

fn dataset_for_kind(kind: ResponseKind) -> Option<&'static str> {
    match kind {
        ResponseKind::InvestorFlow => Some(FLOW_DATASET),
        ResponseKind::MarketStatus => Some(STATUS_DATASET),
        ResponseKind::Fundamentals => Some(FUNDAMENTAL_DATASET),
        ResponseKind::IndexMembership => Some(UNIVERSE_DATASET),
        ResponseKind::SectorClassification => Some(SECTOR_DATASET),
        _ => None,
    }
}

fn expected_dataset(document: &CandidateDocument) -> &'static str {
    match document {
        CandidateDocument::InvestorFlow(_) => FLOW_DATASET,
        CandidateDocument::MarketStatus(_) => STATUS_DATASET,
        CandidateDocument::Fundamentals(_) => FUNDAMENTAL_DATASET,
        CandidateDocument::IndexMembership(_) => UNIVERSE_DATASET,
        CandidateDocument::SectorClassification(_) => SECTOR_DATASET,
    }
}

const fn response_kind_for_document(document: &CandidateDocument) -> ResponseKind {
    match document {
        CandidateDocument::InvestorFlow(_) => ResponseKind::InvestorFlow,
        CandidateDocument::MarketStatus(_) => ResponseKind::MarketStatus,
        CandidateDocument::Fundamentals(_) => ResponseKind::Fundamentals,
        CandidateDocument::IndexMembership(_) => ResponseKind::IndexMembership,
        CandidateDocument::SectorClassification(_) => ResponseKind::SectorClassification,
    }
}

fn validate_publication(publication: &CandidateSourcePublication<'_>) -> Result<(), SinkError> {
    if publication.raw_batch_id.is_nil()
        || publication.raw_manifest_sha256.len() != 64
        || !publication
            .raw_manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SinkError::Invariant(
            "candidate publication requires an exact Raw batch hash".to_owned(),
        ));
    }
    publication
        .pin
        .validate()
        .map_err(|error| SinkError::Invariant(error.to_string()))?;
    validate_candidate_document(publication.document, publication.pin.retrieved_at)
        .map_err(|error| SinkError::Invariant(error.to_string()))?;
    let expected = expected_dataset(publication.document);
    if publication.pin.dataset_id != expected {
        return Err(SinkError::Invariant(format!(
            "candidate document requires dataset {expected}, got {}",
            publication.pin.dataset_id
        )));
    }
    if publication.cutoff_at > publication.pin.retrieved_at {
        return Err(SinkError::Invariant(
            "candidate PIT cutoff cannot follow the pinned retrieval instant".to_owned(),
        ));
    }
    let nonempty = match publication.document {
        CandidateDocument::InvestorFlow(document) => !document.flows.is_empty(),
        CandidateDocument::MarketStatus(document) => !document.statuses.is_empty(),
        CandidateDocument::Fundamentals(document) => !document.fundamentals.is_empty(),
        CandidateDocument::IndexMembership(document) => !document.memberships.is_empty(),
        CandidateDocument::SectorClassification(document) => !document.sectors.is_empty(),
    };
    if !nonempty {
        return Err(SinkError::Invariant(
            "candidate source document must not be empty".to_owned(),
        ));
    }
    Ok(())
}

async fn attest_dataset(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
) -> Result<(), SinkError> {
    let exact: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM dataset_versions
              WHERE id = $1 AND dataset_id = $2 AND version = $3
                AND manifest_sha256 = $4 AND status IN ('READY', 'WARNING')
                AND public.candidate_source_entitlement_is_valid(
                    $5, $6, $2, $7, $7
                )
         )",
    )
    .bind(publication.dataset_version_id)
    .bind(&publication.pin.dataset_id)
    .bind(&publication.pin.dataset_version)
    .bind(&publication.pin.manifest_sha256)
    .bind(publication.pin.entitlement_id)
    .bind(&publication.pin.license_ref)
    .bind(date(publication.as_of))
    .fetch_one(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)?;
    if !exact {
        return Err(SinkError::Conflict(
            "candidate source does not match an exact usable curated dataset pin".to_owned(),
        ));
    }
    Ok(())
}

fn timestamp(value: UtcTimestamp) -> DateTime<Utc> {
    value.as_datetime()
}

fn date(value: TradingDate) -> NaiveDate {
    value.as_naive_date()
}

fn validate_retrieval(
    pin: &CandidateSourcePin,
    available_at: UtcTimestamp,
    source_revision: Option<&str>,
) -> Result<(), SinkError> {
    if available_at > pin.retrieved_at {
        return Err(SinkError::Invariant(
            "candidate observation cannot be retrieved before it is available".to_owned(),
        ));
    }
    if source_revision.is_some_and(|value| value.trim().is_empty()) {
        return Err(SinkError::Invariant(
            "candidate source revision must not be empty".to_owned(),
        ));
    }
    Ok(())
}

async fn publish_flows(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
    rows: &[InvestorFlowObservation],
) -> Result<bool, SinkError> {
    let mut inserted = false;
    // The SQL boundary locks each immutable natural key. Acquire those locks
    // in one canonical order so concurrent rolling snapshots cannot deadlock
    // while reusing their overlapping observations.
    let mut ordered_rows = rows.iter().collect::<Vec<_>>();
    ordered_rows.sort_by_key(|row| {
        format!(
            "{}|{}|{}|{}",
            row.instrument,
            row.trade_date.to_iso(),
            investor_class(row.investor_class),
            row.source_revision
        )
    });
    for row in ordered_rows {
        validate_retrieval(
            publication.pin,
            row.available_at,
            Some(&row.source_revision),
        )?;
        let published: bool = sqlx::query_scalar(
            "SELECT public.insert_candidate_investor_flow(
                $1,$2,$3,$4::numeric(28,4),$5::numeric(28,4),$6,$7,$8,
                $9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(row.instrument.to_string())
        .bind(date(row.trade_date))
        .bind(investor_class(row.investor_class))
        .bind(row.net_amount)
        .bind(row.net_volume)
        .bind(&row.currency)
        .bind(&row.volume_unit)
        .bind(&publication.pin.provider)
        .bind(publication.pin.entitlement_id)
        .bind(date(publication.as_of))
        .bind(&publication.pin.license_ref)
        .bind(&row.source_revision)
        .bind(timestamp(row.available_at))
        .bind(timestamp(publication.pin.retrieved_at))
        .bind(publication.dataset_version_id)
        .bind(&publication.pin.manifest_sha256)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if published {
            inserted = true;
        } else {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM candidate_investor_flows AS flow
                    JOIN candidate_investor_flow_snapshot_rows AS member
                      ON member.flow_observation_id=flow.id
                    JOIN dataset_versions AS dataset ON dataset.id=member.dataset_version_id
                     WHERE flow.instrument_id=$1 AND flow.trade_date=$2
                       AND flow.investor_class=$3 AND flow.source_revision=$4
                       AND flow.net_amount=$5::numeric(28,4)
                       AND flow.net_volume=$6::numeric(28,4)
                       AND flow.currency=$7 AND flow.volume_unit=$8 AND flow.provider=$9
                       AND flow.available_at=$13 AND member.entitlement_id=$10
                       AND member.entitlement_date=$11 AND member.license_ref=$12
                       AND member.retrieved_at=$14 AND member.dataset_version_id=$15
                       AND member.manifest_sha256=$16 AND dataset.manifest_sha256=$16
                )",
            )
            .bind(row.instrument.to_string())
            .bind(date(row.trade_date))
            .bind(investor_class(row.investor_class))
            .bind(&row.source_revision)
            .bind(row.net_amount)
            .bind(row.net_volume)
            .bind(&row.currency)
            .bind(&row.volume_unit)
            .bind(&publication.pin.provider)
            .bind(publication.pin.entitlement_id)
            .bind(date(publication.as_of))
            .bind(&publication.pin.license_ref)
            .bind(timestamp(row.available_at))
            .bind(timestamp(publication.pin.retrieved_at))
            .bind(publication.dataset_version_id)
            .bind(&publication.pin.manifest_sha256)
            .fetch_one(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            require_replay(exact, "investor-flow")?;
        }
    }
    Ok(inserted)
}

async fn publish_statuses(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
    rows: &[MarketStatusObservation],
) -> Result<bool, SinkError> {
    let mut inserted = false;
    let mut ordered_rows = rows.iter().collect::<Vec<_>>();
    ordered_rows.sort_by_key(|row| {
        format!(
            "{}|{}|{}",
            row.instrument,
            row.trade_date.to_iso(),
            row.source_revision
        )
    });
    for row in ordered_rows {
        validate_retrieval(
            publication.pin,
            row.available_at,
            Some(&row.source_revision),
        )?;
        let published: bool = sqlx::query_scalar(
            "SELECT public.insert_candidate_market_status(
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        )
        .bind(row.instrument.to_string())
        .bind(date(row.trade_date))
        .bind(row.suspended)
        .bind(row.administrative)
        .bind(row.liquidation)
        .bind(row.inactive)
        .bind(row.disqualifying_audit_opinion)
        .bind(row.complete_capital_impairment)
        .bind(&publication.pin.provider)
        .bind(publication.pin.entitlement_id)
        .bind(date(publication.as_of))
        .bind(&publication.pin.license_ref)
        .bind(&row.source_revision)
        .bind(timestamp(row.available_at))
        .bind(timestamp(publication.pin.retrieved_at))
        .bind(publication.dataset_version_id)
        .bind(&publication.pin.manifest_sha256)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if published {
            inserted = true;
        } else {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM candidate_market_status_observations
                     WHERE instrument_id=$1 AND trade_date=$2 AND source_revision=$3
                       AND suspended=$4 AND administrative=$5 AND liquidation=$6
                       AND inactive=$7 AND disqualifying_audit_opinion=$8
                       AND complete_capital_impairment=$9 AND provider=$10
                       AND entitlement_id=$11 AND entitlement_date=$12
                       AND license_ref=$13 AND available_at=$14 AND retrieved_at=$15
                       AND dataset_version_id=$16 AND manifest_sha256=$17
                )",
            )
            .bind(row.instrument.to_string())
            .bind(date(row.trade_date))
            .bind(&row.source_revision)
            .bind(row.suspended)
            .bind(row.administrative)
            .bind(row.liquidation)
            .bind(row.inactive)
            .bind(row.disqualifying_audit_opinion)
            .bind(row.complete_capital_impairment)
            .bind(&publication.pin.provider)
            .bind(publication.pin.entitlement_id)
            .bind(date(publication.as_of))
            .bind(&publication.pin.license_ref)
            .bind(timestamp(row.available_at))
            .bind(timestamp(publication.pin.retrieved_at))
            .bind(publication.dataset_version_id)
            .bind(&publication.pin.manifest_sha256)
            .fetch_one(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            require_replay(exact, "market-status")?;
        }
    }
    Ok(inserted)
}

async fn publish_fundamentals(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
    rows: &[FundamentalObservation],
) -> Result<bool, SinkError> {
    let mut inserted = false;
    let mut ordered_rows = rows.iter().collect::<Vec<_>>();
    ordered_rows.sort_by_key(|row| {
        format!(
            "{}|{}|{}|{}|{}|{}",
            row.instrument,
            row.fiscal_period_end.to_iso(),
            statement_scope(row.statement_scope),
            row.metric,
            row.disclosed_at.as_datetime().to_rfc3339(),
            row.source_revision
        )
    });
    // Restatements must still be inserted after the observation they name.
    // Pre-lock the whole natural-key set canonically, then retain the provider's
    // validated dependency order for the actual writes.
    for row in ordered_rows {
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
                 pg_catalog.jsonb_build_array(
                     'candidate-fundamental',$1,$2,$3,$4,$5,$6
                 )::text,0))",
        )
        .bind(row.instrument.to_string())
        .bind(date(row.fiscal_period_end))
        .bind(statement_scope(row.statement_scope))
        .bind(&row.metric)
        .bind(timestamp(row.disclosed_at))
        .bind(&row.source_revision)
        .execute(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
    }
    for row in rows {
        validate_retrieval(
            publication.pin,
            row.available_at,
            Some(&row.source_revision),
        )?;
        if row.disclosed_at > row.available_at || row.fiscal_period_start > row.fiscal_period_end {
            return Err(SinkError::Invariant(
                "fundamental period and disclosure times must be ordered".to_owned(),
            ));
        }
        let restates_id = if let Some(revision) = row.restates_source_revision.as_deref() {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM candidate_fundamental_observations
                  WHERE instrument_id=$1 AND fiscal_period_end=$2 AND statement_scope=$3
                    AND metric=$4 AND source_revision=$5
                  ORDER BY available_at DESC, id DESC LIMIT 1",
            )
            .bind(row.instrument.to_string())
            .bind(date(row.fiscal_period_end))
            .bind(statement_scope(row.statement_scope))
            .bind(&row.metric)
            .bind(revision)
            .fetch_optional(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?
            .ok_or_else(|| {
                SinkError::Conflict(
                    "fundamental restatement references an unknown source revision".to_owned(),
                )
            })?
            .into()
        } else {
            None
        };
        let unit_scale = i32::try_from(row.unit_scale).map_err(|_| {
            SinkError::Invariant("fundamental unit scale exceeds PostgreSQL integer".to_owned())
        })?;
        let published: bool = sqlx::query_scalar(
            "SELECT public.insert_candidate_fundamental(
                $1,$2,$3,$4,$5,$6,$7::numeric(38,10),$8,$9,$10,$11,$12,$13,
                $14,$15,$16,$17,$18,$19,$20,$21)",
        )
        .bind(row.instrument.to_string())
        .bind(date(row.fiscal_period_start))
        .bind(date(row.fiscal_period_end))
        .bind(period_kind(row.period_kind))
        .bind(statement_scope(row.statement_scope))
        .bind(&row.metric)
        .bind(row.value)
        .bind(&row.currency)
        .bind(unit_scale)
        .bind(row.audited)
        .bind(timestamp(row.disclosed_at))
        .bind(timestamp(row.available_at))
        .bind(timestamp(publication.pin.retrieved_at))
        .bind(&publication.pin.provider)
        .bind(publication.pin.entitlement_id)
        .bind(date(publication.as_of))
        .bind(&publication.pin.license_ref)
        .bind(&row.source_revision)
        .bind(restates_id)
        .bind(publication.dataset_version_id)
        .bind(&publication.pin.manifest_sha256)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if published {
            inserted = true;
        } else {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM candidate_fundamental_observations
                     WHERE instrument_id=$1 AND fiscal_period_start=$2 AND fiscal_period_end=$3
                       AND period_kind=$4 AND statement_scope=$5 AND metric=$6
                       AND value=$7::numeric(38,10)
                       AND currency IS NOT DISTINCT FROM $8 AND unit_scale=$9
                       AND audited IS NOT DISTINCT FROM $10 AND disclosed_at=$11
                       AND available_at=$12 AND retrieved_at=$13 AND provider=$14
                       AND entitlement_id=$15 AND entitlement_date=$16
                       AND license_ref=$17 AND source_revision=$18
                       AND restates_observation_id IS NOT DISTINCT FROM $19
                       AND dataset_version_id=$20 AND manifest_sha256=$21
                )",
            )
            .bind(row.instrument.to_string())
            .bind(date(row.fiscal_period_start))
            .bind(date(row.fiscal_period_end))
            .bind(period_kind(row.period_kind))
            .bind(statement_scope(row.statement_scope))
            .bind(&row.metric)
            .bind(row.value)
            .bind(&row.currency)
            .bind(unit_scale)
            .bind(row.audited)
            .bind(timestamp(row.disclosed_at))
            .bind(timestamp(row.available_at))
            .bind(timestamp(publication.pin.retrieved_at))
            .bind(&publication.pin.provider)
            .bind(publication.pin.entitlement_id)
            .bind(date(publication.as_of))
            .bind(&publication.pin.license_ref)
            .bind(&row.source_revision)
            .bind(restates_id)
            .bind(publication.dataset_version_id)
            .bind(&publication.pin.manifest_sha256)
            .fetch_one(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            require_replay(exact, "fundamental")?;
        }
    }
    Ok(inserted)
}

async fn publish_universe(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
    rows: &[IndexMembershipObservation],
) -> Result<bool, SinkError> {
    let index_ids: BTreeSet<_> = rows.iter().map(|row| row.index_id.as_str()).collect();
    if index_ids != BTreeSet::from(["kospi200"]) {
        return Err(SinkError::Invariant(
            "candidate universe document must contain only kospi200 membership".to_owned(),
        ));
    }
    let members = members_as_of(rows, "kospi200", publication.as_of, publication.cutoff_at);
    if members.is_empty() {
        return Err(SinkError::Invariant(
            "candidate universe resolves to no PIT members".to_owned(),
        ));
    }
    for row in members.values() {
        validate_retrieval(
            publication.pin,
            row.available_at,
            Some(&row.source_revision),
        )?;
    }
    let available_at = members
        .values()
        .map(|row| row.available_at)
        .max()
        .expect("members are nonempty");
    let revision_evidence = members
        .values()
        .map(|row| {
            (
                row.instrument.to_string(),
                row.source_revision.as_str(),
                row.effective_from.to_iso(),
                row.effective_until.map(|value| value.to_iso()),
            )
        })
        .collect::<Vec<_>>();
    let source_revision = revision_set_digest(&revision_evidence)?;
    let member_count = i32::try_from(members.len())
        .map_err(|_| SinkError::Invariant("candidate universe is too large".to_owned()))?;
    let snapshot_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT public.insert_candidate_universe_snapshot(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(date(publication.as_of))
    .bind(publication.dataset_version_id)
    .bind(&publication.pin.manifest_sha256)
    .bind(&publication.pin.provider)
    .bind(publication.pin.entitlement_id)
    .bind(date(publication.as_of))
    .bind(&publication.pin.license_ref)
    .bind(&source_revision)
    .bind(timestamp(available_at))
    .bind(timestamp(publication.pin.retrieved_at))
    .bind(member_count)
    .fetch_one(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)?;
    let inserted = snapshot_id.is_some();
    let snapshot_id = match snapshot_id {
        Some(id) => id,
        None => sqlx::query_scalar(
            "SELECT id FROM candidate_universe_snapshots
              WHERE index_id='kospi200' AND as_of_date=$1 AND dataset_version_id=$2
                AND manifest_sha256=$3 AND provider=$4 AND entitlement_id=$5
                AND entitlement_date=$6 AND license_ref=$7 AND source_revision=$8
                AND available_at=$9 AND retrieved_at=$10 AND member_count=$11",
        )
        .bind(date(publication.as_of))
        .bind(publication.dataset_version_id)
        .bind(&publication.pin.manifest_sha256)
        .bind(&publication.pin.provider)
        .bind(publication.pin.entitlement_id)
        .bind(date(publication.as_of))
        .bind(&publication.pin.license_ref)
        .bind(&source_revision)
        .bind(timestamp(available_at))
        .bind(timestamp(publication.pin.retrieved_at))
        .bind(member_count)
        .fetch_optional(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?
        .ok_or_else(|| SinkError::Conflict("universe snapshot replay differs".to_owned()))?,
    };
    for row in members.values() {
        let published: bool = sqlx::query_scalar(
            "SELECT public.insert_candidate_universe_member($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(snapshot_id)
        .bind(row.instrument.to_string())
        .bind(timestamp(row.announced_at))
        .bind(date(row.effective_from))
        .bind(row.effective_until.map(date))
        .bind(timestamp(row.available_at))
        .bind(&row.source_revision)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if !published {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM candidate_universe_members
                     WHERE universe_snapshot_id=$1 AND instrument_id=$2
                       AND announced_at=$3 AND effective_from=$4
                       AND effective_until IS NOT DISTINCT FROM $5
                       AND available_at=$6 AND source_revision=$7
                )",
            )
            .bind(snapshot_id)
            .bind(row.instrument.to_string())
            .bind(timestamp(row.announced_at))
            .bind(date(row.effective_from))
            .bind(row.effective_until.map(date))
            .bind(timestamp(row.available_at))
            .bind(&row.source_revision)
            .fetch_one(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            require_replay(exact, "universe-member")?;
        }
    }
    Ok(inserted)
}

async fn publish_sectors(
    tx: &mut Transaction<'_, Postgres>,
    publication: &CandidateSourcePublication<'_>,
    rows: &[SectorObservation],
) -> Result<bool, SinkError> {
    let scopes: BTreeSet<_> = rows
        .iter()
        .map(|row| (row.taxonomy_id.as_str(), row.taxonomy_version.as_str()))
        .collect();
    if scopes.len() != 1 {
        return Err(SinkError::Invariant(
            "one sector document must have one taxonomy and version".to_owned(),
        ));
    }
    let (taxonomy_id, taxonomy_version) = *scopes
        .first()
        .expect("nonempty document has one taxonomy scope");
    let sectors = sectors_as_of(rows, taxonomy_id, publication.as_of, publication.cutoff_at);
    if sectors.is_empty() {
        return Err(SinkError::Invariant(
            "sector document resolves to no PIT entries".to_owned(),
        ));
    }
    for row in sectors.values() {
        validate_retrieval(
            publication.pin,
            row.available_at,
            Some(&row.source_revision),
        )?;
    }
    let revision_evidence = sectors
        .values()
        .map(|row| {
            (
                row.instrument.to_string(),
                row.source_revision.as_str(),
                row.effective_from.to_iso(),
                row.effective_until.map(|value| value.to_iso()),
                row.sector_code.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let source_revision = revision_set_digest(&revision_evidence)?;
    let available_at = sectors
        .values()
        .map(|row| row.available_at)
        .max()
        .expect("sectors are nonempty");
    let effective_from = sectors
        .values()
        .map(|row| row.effective_from)
        .min()
        .expect("sectors are nonempty");
    let version_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT public.insert_candidate_sector_version(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(taxonomy_id)
    .bind(taxonomy_version)
    .bind(date(effective_from))
    .bind(timestamp(available_at))
    .bind(timestamp(publication.pin.retrieved_at))
    .bind(&publication.pin.provider)
    .bind(publication.pin.entitlement_id)
    .bind(date(publication.as_of))
    .bind(&publication.pin.license_ref)
    .bind(&source_revision)
    .bind(publication.dataset_version_id)
    .bind(&publication.pin.manifest_sha256)
    .fetch_one(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)?;
    let inserted = version_id.is_some();
    let version_id = match version_id {
        Some(id) => id,
        None => sqlx::query_scalar(
            "SELECT id FROM candidate_sector_versions
              WHERE taxonomy_id=$1 AND taxonomy_version=$2 AND effective_from=$3
                AND available_at=$4 AND retrieved_at=$5 AND provider=$6
                AND entitlement_id=$7 AND entitlement_date=$8 AND license_ref=$9
                AND source_revision=$10 AND dataset_version_id=$11
                AND manifest_sha256=$12",
        )
        .bind(taxonomy_id)
        .bind(taxonomy_version)
        .bind(date(effective_from))
        .bind(timestamp(available_at))
        .bind(timestamp(publication.pin.retrieved_at))
        .bind(&publication.pin.provider)
        .bind(publication.pin.entitlement_id)
        .bind(date(publication.as_of))
        .bind(&publication.pin.license_ref)
        .bind(&source_revision)
        .bind(publication.dataset_version_id)
        .bind(&publication.pin.manifest_sha256)
        .fetch_optional(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?
        .ok_or_else(|| SinkError::Conflict("sector version replay differs".to_owned()))?,
    };
    for row in sectors.values() {
        let published: bool = sqlx::query_scalar(
            "SELECT public.insert_candidate_sector_entry($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(version_id)
        .bind(row.instrument.to_string())
        .bind(&row.sector_code)
        .bind(&row.sector_name)
        .bind(fundamental_profile(row.fundamental_profile))
        .bind(date(row.effective_from))
        .bind(row.effective_until.map(date))
        .bind(timestamp(row.available_at))
        .bind(&row.source_revision)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if !published {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM candidate_sector_entries
                     WHERE sector_version_id=$1 AND instrument_id=$2 AND sector_code=$3
                       AND sector_name=$4 AND fundamental_profile=$5 AND effective_from=$6
                       AND effective_until IS NOT DISTINCT FROM $7
                       AND available_at=$8 AND source_revision=$9
                )",
            )
            .bind(version_id)
            .bind(row.instrument.to_string())
            .bind(&row.sector_code)
            .bind(&row.sector_name)
            .bind(fundamental_profile(row.fundamental_profile))
            .bind(date(row.effective_from))
            .bind(row.effective_until.map(date))
            .bind(timestamp(row.available_at))
            .bind(&row.source_revision)
            .fetch_one(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            require_replay(exact, "sector-entry")?;
        }
    }
    Ok(inserted)
}

fn revision_set_digest<T: serde::Serialize>(evidence: &T) -> Result<String, SinkError> {
    let canonical = serde_json::to_vec(evidence).map_err(|error| {
        SinkError::Invariant(format!(
            "candidate revision set is not serializable: {error}"
        ))
    })?;
    Ok(ContentHash::from_bytes(&canonical)
        .as_str()
        .strip_prefix("sha256:")
        .expect("content hashes have a sha256 prefix")
        .to_owned())
}

fn require_replay(exact: bool, kind: &str) -> Result<(), SinkError> {
    if exact {
        Ok(())
    } else {
        Err(SinkError::Conflict(format!(
            "candidate {kind} identity is occupied by different content"
        )))
    }
}

const fn investor_class(value: InvestorClass) -> &'static str {
    match value {
        InvestorClass::Foreign => "FOREIGN",
        InvestorClass::Institution => "INSTITUTION",
    }
}

const fn period_kind(value: FinancialPeriodKind) -> &'static str {
    match value {
        FinancialPeriodKind::Quarter => "QUARTER",
        FinancialPeriodKind::Half => "HALF",
        FinancialPeriodKind::NineMonth => "NINE_MONTH",
        FinancialPeriodKind::Annual => "ANNUAL",
    }
}

const fn statement_scope(value: StatementScope) -> &'static str {
    match value {
        StatementScope::Consolidated => "CONSOLIDATED",
        StatementScope::Separate => "SEPARATE",
    }
}

const fn fundamental_profile(value: FundamentalProfile) -> &'static str {
    match value {
        FundamentalProfile::NonFinancial => "NON_FINANCIAL",
        FundamentalProfile::Financial => "FINANCIAL",
        FundamentalProfile::Unsupported => "UNSUPPORTED",
    }
}

#[cfg(test)]
mod tests {
    use domain::BatchId;
    use market_data::{
        FetchMode, InvestorFlowDocument, RawEnvelope, RequestMetadata, ResponseKind,
        parse_candidate_envelope,
    };

    use super::*;

    fn time(value: &str) -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339(value).expect("valid timestamp")
    }

    fn trading_date(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("valid date")
    }

    #[test]
    fn publication_rejects_wrong_dataset_and_future_cutoff() {
        let document = CandidateDocument::InvestorFlow(InvestorFlowDocument {
            flows: vec![InvestorFlowObservation {
                instrument: domain::InstrumentId::parse("005930.KRX").unwrap(),
                trade_date: trading_date("2026-08-14"),
                investor_class: InvestorClass::Foreign,
                net_amount: 1.0,
                net_volume: 1.0,
                currency: "KRW".into(),
                volume_unit: "SHARE".into(),
                source_revision: "1".into(),
                available_at: time("2026-08-14T07:00:00Z"),
            }],
        });
        let mut pin = CandidateSourcePin {
            provider: "synthetic".into(),
            entitlement_id: Uuid::from_u128(1),
            license_ref: "fixture-only".into(),
            dataset_id: STATUS_DATASET.into(),
            dataset_version: "fixture-v1".into(),
            manifest_sha256: "a".repeat(64),
            retrieved_at: time("2026-08-14T07:01:00Z"),
        };
        let dataset_version_id = Uuid::new_v4();
        {
            let publication = CandidateSourcePublication {
                raw_batch_id: Uuid::from_u128(1),
                raw_manifest_sha256: &"a".repeat(64),
                fetch_mode: market_data::FetchMode::Synthetic,
                dataset_version_id,
                as_of: trading_date("2026-08-14"),
                cutoff_at: time("2026-08-14T07:00:00Z"),
                pin: &pin,
                document: &document,
            };
            assert!(validate_publication(&publication).is_err());
        }
        pin.dataset_id = FLOW_DATASET.into();
        let publication = CandidateSourcePublication {
            raw_batch_id: Uuid::from_u128(1),
            raw_manifest_sha256: &"a".repeat(64),
            fetch_mode: market_data::FetchMode::Synthetic,
            dataset_version_id,
            as_of: trading_date("2026-08-14"),
            cutoff_at: time("2026-08-14T07:02:00Z"),
            pin: &pin,
            document: &document,
        };
        assert!(validate_publication(&publication).is_err());
    }

    #[test]
    fn parser_to_publication_boundary_keeps_typed_document() {
        let envelope = RawEnvelope::new(
            BatchId::generate(),
            ResponseKind::InvestorFlow,
            "flow.json",
            br#"{"flows":[{"instrument":"005930.KRX","trade_date":"2026-08-14","investor_class":"FOREIGN","net_amount":10,"net_volume":2,"currency":"KRW","volume_unit":"SHARE","source_revision":"1","available_at":"2026-08-14T07:00:00Z"}]}"#.to_vec(),
            time("2026-08-14T07:01:00Z"),
            RequestMetadata {
                endpoint: "fixture".into(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: FetchMode::Synthetic,
            },
        );
        assert!(matches!(
            parse_candidate_envelope(&envelope).unwrap(),
            CandidateDocument::InvestorFlow(_)
        ));
    }

    #[test]
    fn migration_binds_exact_source_and_price_rights() {
        let up = include_str!("../../../migrations/0042_candidate_source_contracts.up.sql");
        let down = include_str!("../../../migrations/0042_candidate_source_contracts.down.sql");
        for token in [
            "CREATE TABLE public.candidate_price_publications",
            "CREATE TABLE public.candidate_price_instrument_coverage",
            "CREATE TABLE public.candidate_price_instrument_sessions",
            "CREATE TABLE public.candidate_investor_flow_snapshot_rows",
            "curated_generation  bigint NOT NULL",
            "candidate_price_generation_check CHECK (curated_generation > 0)",
            "CREATE FUNCTION public.candidate_source_entitlement_is_valid",
            "entitlement.contract_reference = p_contract_reference",
            "entitlement.covered_uses @> '[\"candidate\"]'::jsonb",
            "CREATE FUNCTION public.resolve_candidate_contract_entitlement",
            "CREATE FUNCTION public.register_candidate_instrument",
            "CREATE FUNCTION public.register_candidate_source_dataset",
            "CREATE FUNCTION public.publish_candidate_price_publication",
            "p_instrument_coverage jsonb",
            "candidate price coverage conflicts with immutable generation",
            "v_expected_storage_path := 'db://candidate/'",
            "candidate source catalog requires exact active candidate-use rights",
            "entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id)",
            "UNIQUE (taxonomy_id, taxonomy_version, effective_from, source_revision,",
            "candidate_raw_dataset_single_origin_idx",
            "NOT binding.reused_existing",
            "p_first_date date, p_last_date date",
            "'candidate_price_publications'",
            "'candidate_price_instrument_coverage'",
            "'candidate_price_instrument_sessions'",
        ] {
            assert!(up.contains(token), "0042 up missing {token}");
        }
        assert!(down.contains("EXISTS (SELECT 1 FROM public.candidate_price_publications)"));
        assert!(down.contains("EXISTS (SELECT 1 FROM public.candidate_price_instrument_coverage)"));
        assert!(down.contains("EXISTS (SELECT 1 FROM public.candidate_price_instrument_sessions)"));
        assert!(down.contains("EXISTS (SELECT 1 FROM public.candidate_instrument_registrations)"));
        assert!(
            down.contains("EXISTS (SELECT 1 FROM public.candidate_investor_flow_snapshot_rows)")
        );
        assert!(down.contains("DROP TABLE public.candidate_price_publications"));
        assert!(down.contains("DROP TABLE public.candidate_price_instrument_coverage"));
        assert!(down.contains("DROP TABLE public.candidate_price_instrument_sessions"));
        assert!(!up.contains("GRANT INSERT ON TABLE public.dataset_versions TO research_writer"));
        assert!(!up.contains("GRANT INSERT ON TABLE public.candidate_price_publications"));
    }
}
