//! Production Owner Equity V2 adapter over the reviewed KIS/Raw seams.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use collectors::owner_equity_v2::artifact::{
    OwnerEquityArtifactError, OwnerEquityArtifactInput, read_owner_equity_artifact,
    write_owner_equity_artifact,
};
use collectors::owner_equity_v2::materialize_owner_equity_from_raw_allow_insufficient;
use domain::{ContentHash, RetryDisposition, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::owner_equity_v2::{
    OwnerEquityCaptureIdentity, OwnerEquityCaptureKind, OwnerEquityCapturePlan,
    OwnerEquityGenerationCandidate, merge_owner_equity_incremental_candidate,
};
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::storage::RawStore;
use uuid::Uuid;

use super::{
    AdmittedGenerationDescriptor, OwnerEquityCoverage, OwnerEquityJobAction, OwnerEquityJobPayload,
    OwnerEquityMaterialization, OwnerEquityPriorCandidate, OwnerEquityWorkFailure,
    OwnerEquityWorkerAdapter, OwnerEquityWorkerError, PreparedOwnerEquityGeneration,
    capture_with_wp2,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerEquityRuntimeLimits {
    pub maximum_active_instruments: u32,
    pub initial_get_ceiling_per_job: usize,
    pub incremental_get_ceiling_per_job: usize,
    pub total_initial_backfill_get_ceiling: usize,
    pub concurrency: usize,
    pub estimated_bytes_per_get: u64,
}

impl Default for OwnerEquityRuntimeLimits {
    fn default() -> Self {
        Self {
            maximum_active_instruments: 100,
            initial_get_ceiling_per_job: 7,
            incremental_get_ceiling_per_job: 2,
            total_initial_backfill_get_ceiling: 700,
            concurrency: 1,
            estimated_bytes_per_get: 1_048_576,
        }
    }
}

impl OwnerEquityRuntimeLimits {
    pub fn validate(self) -> Result<Self, OwnerEquityWorkerError> {
        let total = self
            .initial_get_ceiling_per_job
            .checked_mul(self.maximum_active_instruments as usize)
            .ok_or(OwnerEquityWorkerError::InvalidJob)?;
        if self.maximum_active_instruments == 0
            || self.maximum_active_instruments > 100
            || self.initial_get_ceiling_per_job < 2
            || self.incremental_get_ceiling_per_job < 2
            || self.incremental_get_ceiling_per_job > self.initial_get_ceiling_per_job
            || self.total_initial_backfill_get_ceiling < total
            || self.total_initial_backfill_get_ceiling > 700
            || self.concurrency != 1
            || self.estimated_bytes_per_get == 0
        {
            return Err(OwnerEquityWorkerError::InvalidJob);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerEquityPreflight {
    pub planned_gets: usize,
    pub per_job_get_ceiling: usize,
    pub total_initial_backfill_get_ceiling: usize,
    pub estimated_job_disk_bytes: u64,
    pub estimated_total_initial_disk_bytes: u64,
    pub concurrency: usize,
}

pub struct ProductionOwnerEquityAdapter<R: KisRead> {
    raw_store: RawStore,
    artifact_root: PathBuf,
    reader: Arc<R>,
    limits: OwnerEquityRuntimeLimits,
}

impl<R: KisRead> std::fmt::Debug for ProductionOwnerEquityAdapter<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionOwnerEquityAdapter")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl<R: KisRead> ProductionOwnerEquityAdapter<R> {
    pub fn new(
        raw_root: &Path,
        artifact_root: &Path,
        reader: R,
        limits: OwnerEquityRuntimeLimits,
    ) -> Result<Self, OwnerEquityWorkerError> {
        let limits = limits.validate()?;
        validate_runtime_root(raw_root)?;
        validate_runtime_root(artifact_root)?;
        Ok(Self {
            raw_store: RawStore::new(raw_root),
            artifact_root: artifact_root.to_owned(),
            reader: Arc::new(reader),
            limits,
        })
    }

    pub fn preflight(
        &self,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityPreflight, OwnerEquityWorkerError> {
        payload.validate()?;
        if payload.max_active_instruments > self.limits.maximum_active_instruments {
            return Err(OwnerEquityWorkerError::InvalidJob);
        }
        let plan = capture_plan(payload, prior)?;
        let per_job_get_ceiling = match payload.action {
            OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry => {
                self.limits.initial_get_ceiling_per_job
            }
            OwnerEquityJobAction::Incremental => self.limits.incremental_get_ceiling_per_job,
            OwnerEquityJobAction::DisableSnapshot | OwnerEquityJobAction::DuplicateReceipt => {
                return Err(OwnerEquityWorkerError::InvalidJob);
            }
        };
        if plan.exact_get_ceiling > per_job_get_ceiling {
            return Err(OwnerEquityWorkerError::EvidenceMismatch);
        }
        let estimated_job_disk_bytes = self
            .limits
            .estimated_bytes_per_get
            .checked_mul(plan.exact_get_ceiling as u64)
            .ok_or(OwnerEquityWorkerError::InvalidJob)?;
        let estimated_total_initial_disk_bytes = self
            .limits
            .estimated_bytes_per_get
            .checked_mul(self.limits.total_initial_backfill_get_ceiling as u64)
            .ok_or(OwnerEquityWorkerError::InvalidJob)?;
        Ok(OwnerEquityPreflight {
            planned_gets: plan.exact_get_ceiling,
            per_job_get_ceiling,
            total_initial_backfill_get_ceiling: self.limits.total_initial_backfill_get_ceiling,
            estimated_job_disk_bytes,
            estimated_total_initial_disk_bytes,
            concurrency: self.limits.concurrency,
        })
    }

    fn identity(
        &self,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityCaptureIdentity, OwnerEquityWorkerError> {
        self.preflight(payload, prior)?;
        OwnerEquityCaptureIdentity::new(
            capture_plan(payload, prior)?,
            payload.entitlement_reference.clone(),
            payload.entitlement_hash()?,
            payload.code_revision()?,
        )
        .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)
    }

    fn materialize_impl(
        &self,
        owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        let identity = self.identity(payload, prior).map_err(worker_failure)?;
        let materialized = materialize_owner_equity_from_raw_allow_insufficient(
            &self.raw_store,
            &identity,
            payload.code_revision().map_err(worker_failure)?,
        )
        .map_err(|error| OwnerEquityWorkFailure::from_collector(&error))?;
        let candidate = if identity.plan.kind == OwnerEquityCaptureKind::Incremental {
            let prior = prior.ok_or_else(|| terminal_failure("INCREMENTAL_PRIOR_MISSING"))?;
            let prior_artifact = ContentHash::parse(&prior.descriptor.artifact_manifest_sha256)
                .map_err(|_| terminal_failure("ARTIFACT_PIN_INVALID"))?;
            merge_owner_equity_incremental_candidate(
                &prior.candidate,
                prior_artifact,
                &materialized.candidate,
            )
            .map_err(|error| terminal_failure(error.code()))?
        } else {
            materialized.candidate
        };
        if candidate.observed_sessions < payload.minimum_observed_sessions {
            return Ok(OwnerEquityMaterialization::InsufficientHistory(
                OwnerEquityCoverage {
                    observed_sessions: candidate.observed_sessions,
                    first_session: candidate.bars.first().map(|bar| bar.session_date),
                    last_session: candidate.bars.last().map(|bar| bar.session_date),
                },
            ));
        }
        let generation = payload
            .expected_generation
            .ok_or_else(|| terminal_failure("GENERATION_MISMATCH"))?;
        let verified = write_owner_equity_artifact(
            &self.artifact_root,
            OwnerEquityArtifactInput {
                owner_user_id,
                membership_id: payload.membership_id,
                generation,
                candidate: &candidate,
            },
        )
        .map_err(artifact_failure)?;
        Ok(OwnerEquityMaterialization::Ready(Box::new(
            PreparedOwnerEquityGeneration {
                candidate: verified.candidate,
                artifact_manifest_sha256: verified.manifest_sha256,
            },
        )))
    }
}

#[async_trait]
impl<R: KisRead> OwnerEquityWorkerAdapter for ProductionOwnerEquityAdapter<R> {
    async fn validate(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.preflight(payload, None)
            .map(|_| ())
            .map_err(worker_failure)
    }

    async fn backfill(
        &self,
        _payload: &OwnerEquityJobPayload,
    ) -> Result<(), OwnerEquityWorkFailure> {
        Err(terminal_failure("RUNTIME_CONTEXT_MISSING"))
    }

    async fn materialize(
        &self,
        _payload: &OwnerEquityJobPayload,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        Err(terminal_failure("RUNTIME_CONTEXT_MISSING"))
    }

    async fn load_admitted_candidate(
        &self,
        descriptor: &AdmittedGenerationDescriptor,
    ) -> Result<OwnerEquityGenerationCandidate, OwnerEquityWorkFailure> {
        let manifest_sha256 = ContentHash::parse(&descriptor.artifact_manifest_sha256)
            .map_err(|_| terminal_failure("ARTIFACT_PIN_INVALID"))?;
        let verified = read_owner_equity_artifact(&self.artifact_root, &manifest_sha256)
            .map_err(artifact_failure)?;
        if verified.owner_user_id != descriptor.owner_user_id
            || verified.membership_id != descriptor.membership_id
            || verified.generation != u64::try_from(descriptor.generation).unwrap_or(0)
            || verified.candidate.instrument_id.to_string() != descriptor.instrument_id
        {
            return Err(terminal_failure("ARTIFACT_DESCRIPTOR_MISMATCH"));
        }
        Ok(verified.candidate)
    }

    async fn validate_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.preflight(payload, prior)
            .map(|_| ())
            .map_err(worker_failure)
    }

    async fn backfill_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<(), OwnerEquityWorkFailure> {
        let identity = self.identity(payload, prior).map_err(worker_failure)?;
        let provider = KisProvider::new(
            SharedKisRead(Arc::clone(&self.reader)),
            vec![identity.plan.instrument_id.clone()],
        )
        .map_err(|_| terminal_failure("PROVIDER_CONFIGURATION_INVALID"))?;
        capture_with_wp2(&self.raw_store, &provider, &identity, UtcTimestamp::now())
            .await
            .map(|_| ())
            .map_err(|error| OwnerEquityWorkFailure::from_collector(&error))
    }

    async fn materialize_with_prior(
        &self,
        owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        self.materialize_impl(owner_user_id, payload, prior)
    }
}

struct SharedKisRead<R>(Arc<R>);

impl<R> Clone for SharedKisRead<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R> std::fmt::Debug for SharedKisRead<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedKisRead(<redacted>)")
    }
}

impl<R: KisRead> KisRead for SharedKisRead<R> {
    async fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.0.get(path, tr_id, query, continuation).await
    }
}

fn capture_plan(
    payload: &OwnerEquityJobPayload,
    prior: Option<&OwnerEquityPriorCandidate>,
) -> Result<OwnerEquityCapturePlan, OwnerEquityWorkerError> {
    let policy = payload.policy()?;
    match payload.action {
        OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry => {
            if prior.is_some() {
                return Err(OwnerEquityWorkerError::EvidenceMismatch);
            }
            OwnerEquityCapturePlan::initial_through(
                &payload.instrument_id,
                policy,
                payload.requested_through,
            )
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)
        }
        OwnerEquityJobAction::Incremental => {
            let prior = prior.ok_or(OwnerEquityWorkerError::EvidenceMismatch)?;
            if prior.candidate.instrument_id.to_string() != payload.instrument_id
                || prior.candidate.target_observed_sessions != payload.target_observed_sessions
                || prior.candidate.minimum_observed_sessions != payload.minimum_observed_sessions
            {
                return Err(OwnerEquityWorkerError::EvidenceMismatch);
            }
            OwnerEquityCapturePlan::incremental_through(
                &payload.instrument_id,
                policy,
                prior.candidate.last_observed_date,
                payload.requested_through,
            )
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)
        }
        OwnerEquityJobAction::DisableSnapshot | OwnerEquityJobAction::DuplicateReceipt => {
            Err(OwnerEquityWorkerError::InvalidJob)
        }
    }
}

fn validate_runtime_root(path: &Path) -> Result<(), OwnerEquityWorkerError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| OwnerEquityWorkerError::InvalidJob)?;
    if !path.is_absolute()
        || path == Path::new("/")
        || metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || path
            .canonicalize()
            .map_err(|_| OwnerEquityWorkerError::InvalidJob)?
            != path
    {
        return Err(OwnerEquityWorkerError::InvalidJob);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(OwnerEquityWorkerError::InvalidJob);
        }
    }
    Ok(())
}

fn worker_failure(error: OwnerEquityWorkerError) -> OwnerEquityWorkFailure {
    terminal_failure(error.code())
}

fn terminal_failure(code: &str) -> OwnerEquityWorkFailure {
    OwnerEquityWorkFailure::new(code, RetryDisposition::Terminal).unwrap_or_else(|_| {
        OwnerEquityWorkFailure::new("OWNER_EQUITY_RUNTIME_FAILURE", RetryDisposition::Terminal)
            .expect("static runtime failure code is valid")
    })
}

fn artifact_failure(error: OwnerEquityArtifactError) -> OwnerEquityWorkFailure {
    let (code, retry) = match error {
        OwnerEquityArtifactError::WriteFailed => {
            ("ARTIFACT_WRITE_UNAVAILABLE", RetryDisposition::Retryable)
        }
        OwnerEquityArtifactError::Missing => ("ARTIFACT_MISSING", RetryDisposition::Terminal),
        OwnerEquityArtifactError::UnsafeRoot | OwnerEquityArtifactError::UnsafePermissions => {
            ("ARTIFACT_ROOT_UNSAFE", RetryDisposition::Terminal)
        }
        OwnerEquityArtifactError::Tampered => ("ARTIFACT_TAMPERED", RetryDisposition::Terminal),
        OwnerEquityArtifactError::Conflict => {
            ("ARTIFACT_IMMUTABLE_CONFLICT", RetryDisposition::Terminal)
        }
        OwnerEquityArtifactError::CandidateInvalid => {
            ("ARTIFACT_CANDIDATE_INVALID", RetryDisposition::Terminal)
        }
    };
    OwnerEquityWorkFailure::new(code, retry).expect("static artifact failure code is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{BatchId, CodeCommit, InstrumentId, TradingDate};
    use market_data::owner_equity_v2::{
        OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OwnerEquityBar,
        OwnerEquitySourcePins, PRICE_SEMANTICS,
    };
    use tempfile::tempdir;

    fn secure(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750)).unwrap();
        }
    }

    #[derive(Debug)]
    struct NoProvider;

    impl KisRead for NoProvider {
        async fn get(
            &self,
            _path: &str,
            _tr_id: &str,
            _query: &[(String, String)],
            _continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            panic!("preflight must not reach provider")
        }
    }

    fn payload(action: OwnerEquityJobAction) -> OwnerEquityJobPayload {
        OwnerEquityJobPayload {
            schema_version: super::super::OWNER_EQUITY_V2_JOB_SCHEMA_VERSION,
            action,
            membership_id: Uuid::from_u128(3),
            instrument_id: "005930.KRX".to_owned(),
            expected_generation: Some(if action == OwnerEquityJobAction::Incremental {
                2
            } else {
                1
            }),
            request_body_sha256: "a".repeat(64),
            requested_through: TradingDate::parse("2026-08-31").unwrap(),
            max_active_instruments: 100,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            entitlement_reference: "fixture://entitlement".to_owned(),
            entitlement_sha256: ContentHash::from_bytes(b"entitlement").to_string(),
        }
    }

    fn prior() -> OwnerEquityPriorCandidate {
        let last = TradingDate::parse("2026-08-30").unwrap();
        let bar = OwnerEquityBar {
            session_date: last,
            open: 100,
            high: 105,
            low: 95,
            close: 101,
            volume: 1_000,
        };
        let candidate = OwnerEquityGenerationCandidate {
            candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
            contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
            capture_kind: OwnerEquityCaptureKind::Initial,
            instrument_id: InstrumentId::parse("005930.KRX").unwrap(),
            display_name: None,
            requested_start: last,
            requested_end: last,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            observed_sessions: 1,
            first_observed_date: last,
            last_observed_date: last,
            bars: vec![bar],
            source_pins: OwnerEquitySourcePins {
                capture_identity_sha256: ContentHash::from_bytes(b"identity"),
                raw_batch_id: BatchId::from_uuid(Uuid::from_u128(4)),
                raw_manifest_sha256: ContentHash::from_bytes(b"raw"),
                batch_json_sha256: ContentHash::from_bytes(b"batch"),
                entitlement_reference: "fixture://entitlement".to_owned(),
                entitlement_sha256: ContentHash::from_bytes(b"entitlement"),
                capture_code_commit: CodeCommit::parse("0123456789abcdef0123456789abcdef01234567")
                    .unwrap(),
                materializer_code_commit: CodeCommit::parse(
                    "0123456789abcdef0123456789abcdef01234567",
                )
                .unwrap(),
                prior_candidate_sha256: None,
                prior_artifact_manifest_sha256: None,
                files: vec![],
            },
            price_semantics: PRICE_SEMANTICS.to_owned(),
            owner_only: true,
            vendor_snapshot: true,
            strict_pit: false,
            warnings: vec![],
            claims_not_made: vec![],
        };
        OwnerEquityPriorCandidate {
            descriptor: AdmittedGenerationDescriptor {
                owner_user_id: Uuid::from_u128(2),
                membership_id: Uuid::from_u128(3),
                generation_id: Uuid::from_u128(5),
                instrument_id: "005930.KRX".to_owned(),
                generation: 1,
                raw_manifest_sha256: candidate.source_pins.raw_manifest_sha256.to_string(),
                artifact_manifest_sha256: ContentHash::from_bytes(b"artifact").to_string(),
                entitlement_sha256: candidate.source_pins.entitlement_sha256.to_string(),
                capture_code_commit: candidate.source_pins.capture_code_commit.to_string(),
                materializer_code_commit: candidate
                    .source_pins
                    .materializer_code_commit
                    .to_string(),
            },
            candidate,
        }
    }

    #[test]
    fn preflight_is_provider_free_numeric_and_exactly_bounded() {
        let raw = tempdir().unwrap();
        let artifact = tempdir().unwrap();
        secure(raw.path());
        secure(artifact.path());
        let adapter = ProductionOwnerEquityAdapter::new(
            raw.path(),
            artifact.path(),
            NoProvider,
            OwnerEquityRuntimeLimits::default(),
        )
        .unwrap();
        let initial = adapter
            .preflight(&payload(OwnerEquityJobAction::Add), None)
            .unwrap();
        assert_eq!(initial.planned_gets, 7);
        assert_eq!(initial.per_job_get_ceiling, 7);
        assert_eq!(initial.total_initial_backfill_get_ceiling, 700);
        assert_eq!(initial.concurrency, 1);
        assert_eq!(initial.estimated_job_disk_bytes, 7 * 1_048_576);

        let incremental = adapter
            .preflight(&payload(OwnerEquityJobAction::Incremental), Some(&prior()))
            .unwrap();
        assert_eq!(incremental.planned_gets, 2);
        assert_eq!(incremental.per_job_get_ceiling, 2);
    }

    #[test]
    fn preflight_rejects_active_limit_concurrency_and_incremental_refetch() {
        let raw = tempdir().unwrap();
        let artifact = tempdir().unwrap();
        secure(raw.path());
        secure(artifact.path());
        assert_eq!(
            OwnerEquityRuntimeLimits {
                concurrency: 2,
                ..OwnerEquityRuntimeLimits::default()
            }
            .validate(),
            Err(OwnerEquityWorkerError::InvalidJob)
        );
        let adapter = ProductionOwnerEquityAdapter::new(
            raw.path(),
            artifact.path(),
            NoProvider,
            OwnerEquityRuntimeLimits::default(),
        )
        .unwrap();
        let mut over_active = payload(OwnerEquityJobAction::Add);
        over_active.max_active_instruments = 101;
        assert!(adapter.preflight(&over_active, None).is_err());
        let mut stale = payload(OwnerEquityJobAction::Incremental);
        stale.requested_through = TradingDate::parse("2026-01-01").unwrap();
        assert!(adapter.preflight(&stale, Some(&prior())).is_err());
    }

    #[test]
    fn production_source_has_only_reviewed_read_channels_and_no_forbidden_identifiers() {
        let source = include_str!("runtime.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(!production.contains("http::"));
        assert!(!production.contains("reqwest"));
        let identifiers = production
            .to_ascii_lowercase()
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        for forbidden in [
            concat!("ca", "no"),
            concat!("acnt_", "prdt_cd"),
            concat!("submit_", "order"),
            concat!("account_", "balance"),
        ] {
            assert!(!identifiers.contains(forbidden));
        }
    }
}
