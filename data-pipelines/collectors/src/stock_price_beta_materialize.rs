//! Provider-free fixed-stock beta materializer.  This module never constructs
//! a provider client, reads credentials, or performs network I/O.
use crate::stock_price_beta_raw::{
    CaptureIdentity, DAILY_BARS_PATH, DAILY_BARS_TR_ID, ENTITLEMENT_DOCUMENT_REFERENCE,
    FID_ORG_ADJ_PRC, FIXED_CAPTURE_WINDOWS, FIXED_RANGE_START, FIXED_SYMBOL_COUNT, RAW_MARKET,
    RAW_PROVIDER, parse_entitlement_bytes, parse_universe_bytes,
};
use domain::{BatchId, ContentHash};
use factor_engine::{
    PriceVolumeSignalSnapshot, read_fixed_stock_price_beta_snapshot_against,
    write_fixed_stock_price_beta_snapshot_against,
};
use market_data::storage::ManifestEntry;
use market_data::{
    FixedStockPriceBetaArtifact, FixedStockPriceBetaRawBatchEvidence,
    FixedStockPriceBetaRawFileEvidence, FixedStockPriceBetaRawSourceFile,
    FixedStockPriceBetaRawWindow, RawStore, parse_fixed_stock_price_beta_raw_sources,
    read_fixed_stock_price_beta_artifact, write_fixed_stock_price_beta_artifact,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterializeError {
    #[error("invalid provider-free materialization input")]
    Invalid,
    #[error("immutable Raw data is missing or tampered")]
    Raw,
    #[error("artifact or snapshot verification failed")]
    Artifact,
    #[error("filesystem I/O failed")]
    Io,
}

#[derive(Debug, Clone)]
pub struct MaterializeRequest {
    pub raw_root: PathBuf,
    pub artifact_root: PathBuf,
    pub universe_bytes: Vec<u8>,
    pub entitlement_bytes: Vec<u8>,
    pub batch_id: BatchId,
    pub capture_commit: String,
}
#[derive(Debug, Clone)]
pub struct MaterializeOutcome {
    pub batch_id: BatchId,
    pub artifact: FixedStockPriceBetaArtifact,
    pub snapshot: PriceVolumeSignalSnapshot,
    pub artifact_path: PathBuf,
    pub snapshot_path: PathBuf,
}

pub fn materialize(request: &MaterializeRequest) -> Result<MaterializeOutcome, MaterializeError> {
    validate_request(request)?;
    let store = RawStore::new(&request.raw_root);
    let entry = find_entry(&store, request.batch_id)?;
    let (batch_hash, manifest_hash) = verify_metadata(&store, &entry)?;
    if !entry_contract_ok(&entry)
        || hash(&request.entitlement_bytes) != crate::stock_price_beta_raw::ENTITLEMENT_FILE_SHA256
    {
        return Err(MaterializeError::Invalid);
    }
    if !entry_contract_ok(&entry) {
        return Err(MaterializeError::Raw);
    }
    let (evidence, sources) = evidence_and_sources(
        &store,
        &entry,
        &request.entitlement_bytes,
        &request.capture_commit,
        batch_hash,
        manifest_hash,
    )?;
    let bars = parse_fixed_stock_price_beta_raw_sources(&evidence, &sources)
        .map_err(|_| MaterializeError::Raw)?;
    // A successful parser result has the exact common session matrix.  121 is
    // needed for the 120-observation signal return (not merely 120 bars).
    if bars.len() / FIXED_SYMBOL_COUNT < 121 {
        return Err(MaterializeError::Raw);
    }
    let artifact = FixedStockPriceBetaArtifact::build(
        &request.universe_bytes,
        evidence,
        sources.clone(),
        bars,
    )
    .map_err(|_| MaterializeError::Artifact)?;
    artifact
        .verify_against_raw_sources(&request.universe_bytes, &sources)
        .map_err(|_| MaterializeError::Artifact)?;
    let as_of = artifact.sessions.last().ok_or(MaterializeError::Artifact)?;
    let snapshot = PriceVolumeSignalSnapshot::compute(&artifact, as_of)
        .map_err(|_| MaterializeError::Artifact)?;
    let artifact_path = write_fixed_stock_price_beta_artifact(&request.artifact_root, &artifact)
        .map_err(|_| MaterializeError::Artifact)?;
    let snapshot_path =
        write_fixed_stock_price_beta_snapshot_against(&request.artifact_root, &snapshot, &artifact)
            .map_err(|_| MaterializeError::Artifact)?;
    let reopened =
        read_fixed_stock_price_beta_artifact(&request.artifact_root, &artifact.content_sha256)
            .map_err(|_| MaterializeError::Artifact)?;
    reopened
        .verify_against_raw_sources(&request.universe_bytes, &sources)
        .map_err(|_| MaterializeError::Artifact)?;
    read_fixed_stock_price_beta_snapshot_against(
        &request.artifact_root,
        &snapshot.content_sha256,
        &reopened,
    )
    .map_err(|_| MaterializeError::Artifact)?;
    Ok(MaterializeOutcome {
        batch_id: request.batch_id,
        artifact,
        snapshot,
        artifact_path,
        snapshot_path,
    })
}
fn entry_contract_ok(entry: &ManifestEntry) -> bool {
    entry.mode == market_data::FetchMode::Credentialed
        && entry.entitlement_reference.as_deref() == Some(ENTITLEMENT_DOCUMENT_REFERENCE)
        && entry.files.len() == 90
}

/// Full read-only approval check.  Registry hashes alone cannot approve: this
/// always reopens Raw, re-parses bodies, and recomputes the factor snapshot.
pub fn check(request: &MaterializeRequest, registry_bytes: &[u8]) -> Result<(), MaterializeError> {
    validate_request(request)?;
    let store = RawStore::new(&request.raw_root);
    let entry = find_entry(&store, request.batch_id)?;
    let (batch_hash, manifest_hash) = verify_metadata(&store, &entry)?;
    let (evidence, sources) = evidence_and_sources(
        &store,
        &entry,
        &request.entitlement_bytes,
        &request.capture_commit,
        batch_hash,
        manifest_hash,
    )?;
    let bars = parse_fixed_stock_price_beta_raw_sources(&evidence, &sources)
        .map_err(|_| MaterializeError::Raw)?;
    if bars.len() / FIXED_SYMBOL_COUNT < 121 {
        return Err(MaterializeError::Raw);
    }
    // Evidence does not name an artifact content hash; the registry does.  It
    // is therefore the only permitted selector, and it must contain one match.
    let registry = market_data::parse_fixed_stock_price_beta_approval_registry(registry_bytes)
        .map_err(|_| MaterializeError::Artifact)?;
    if registry.approved_artifacts.len() != 1 {
        return Err(MaterializeError::Artifact);
    }
    let pin = &registry.approved_artifacts[0];
    let artifact =
        read_fixed_stock_price_beta_artifact(&request.artifact_root, &pin.artifact_content_sha256)
            .map_err(|_| MaterializeError::Artifact)?;
    artifact
        .verify_against_raw_sources(&request.universe_bytes, &sources)
        .map_err(|_| MaterializeError::Artifact)?;
    let snapshot = read_fixed_stock_price_beta_snapshot_against(
        &request.artifact_root,
        &pin.snapshot_content_sha256,
        &artifact,
    )
    .map_err(|_| MaterializeError::Artifact)?;
    market_data::verify_fixed_stock_price_beta_approval(
        registry_bytes,
        &artifact,
        &snapshot.content_sha256,
        &snapshot.as_of,
        &request.batch_id.to_string(),
    )
    .map_err(|_| MaterializeError::Artifact)?;
    Ok(())
}

fn validate_request(request: &MaterializeRequest) -> Result<(), MaterializeError> {
    if !request.raw_root.is_absolute()
        || !request.artifact_root.is_absolute()
        || request.raw_root == request.artifact_root
    {
        return Err(MaterializeError::Invalid);
    }
    let universe =
        parse_universe_bytes(&request.universe_bytes).map_err(|_| MaterializeError::Invalid)?;
    let entitlement = parse_entitlement_bytes(&request.entitlement_bytes)
        .map_err(|_| MaterializeError::Invalid)?;
    let start =
        domain::TradingDate::parse(FIXED_RANGE_START).map_err(|_| MaterializeError::Invalid)?;
    let end = domain::TradingDate::parse(crate::stock_price_beta_raw::FIXED_RANGE_END)
        .map_err(|_| MaterializeError::Invalid)?;
    let identity = CaptureIdentity::new(
        &universe,
        start,
        end,
        entitlement.document_reference,
        entitlement.file_sha256,
        request.capture_commit.clone(),
    )
    .map_err(|_| MaterializeError::Invalid)?;
    if identity.batch_id() != request.batch_id {
        return Err(MaterializeError::Invalid);
    }
    Ok(())
}

fn find_entry(store: &RawStore, id: BatchId) -> Result<ManifestEntry, MaterializeError> {
    let entries = store
        .read_committed_manifest(RAW_PROVIDER, RAW_MARKET)
        .map_err(|_| MaterializeError::Raw)?;
    let matches: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            e.batch_id == id
                && e.provider == RAW_PROVIDER
                && e.market == RAW_MARKET
                && e.date.to_iso() == FIXED_RANGE_START
        })
        .collect();
    if matches.len() != 1 {
        Err(MaterializeError::Raw)
    } else {
        Ok(matches.into_iter().next().unwrap())
    }
}
fn verify_metadata(
    store: &RawStore,
    entry: &ManifestEntry,
) -> Result<(String, String), MaterializeError> {
    let batch_path = store
        .batch_dir(RAW_PROVIDER, RAW_MARKET, &entry.date, &entry.batch_id)
        .join("batch.json");
    safe_metadata_file(&batch_path)?;
    let batch = fs::read(&batch_path).map_err(|_| MaterializeError::Raw)?;
    let parsed: ManifestEntry =
        serde_json::from_slice(&batch).map_err(|_| MaterializeError::Raw)?;
    if &parsed != entry {
        return Err(MaterializeError::Raw);
    }
    let manifest_path = store.manifest_path(RAW_PROVIDER, RAW_MARKET);
    safe_metadata_file(&manifest_path)?;
    let manifest = fs::read(manifest_path).map_err(|_| MaterializeError::Raw)?;
    let lines: Vec<_> = manifest
        .split_inclusive(|b| *b == b'\n')
        .filter(|line| serde_json::from_slice::<ManifestEntry>(line).ok().as_ref() == Some(entry))
        .collect();
    if lines.len() != 1 {
        return Err(MaterializeError::Raw);
    }
    Ok((hash(&batch), hash(lines[0])))
}
fn safe_metadata_file(path: &std::path::Path) -> Result<(), MaterializeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| MaterializeError::Raw)?;
    if !metadata.file_type().is_file() {
        return Err(MaterializeError::Raw);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(MaterializeError::Raw);
        }
    }
    Ok(())
}
fn evidence_and_sources(
    store: &RawStore,
    entry: &ManifestEntry,
    entitlement: &[u8],
    commit: &str,
    batch_hash: String,
    manifest_hash: String,
) -> Result<
    (
        FixedStockPriceBetaRawBatchEvidence,
        Vec<FixedStockPriceBetaRawSourceFile>,
    ),
    MaterializeError,
> {
    if entry.files.len() != 90 {
        return Err(MaterializeError::Raw);
    }
    let stored = store
        .read_batch_bytes(RAW_PROVIDER, RAW_MARKET, entry)
        .map_err(|_| MaterializeError::Raw)?;
    let mut evidence_files = Vec::new();
    let mut sources = Vec::new();
    for (index, symbol) in crate::stock_price_beta_raw::FIXED_STOCK_SYMBOLS
        .iter()
        .enumerate()
    {
        let instrument_id = format!("{symbol}.KRX");
        for (windex, window) in FIXED_CAPTURE_WINDOWS.iter().enumerate() {
            let position = index * 3 + windex;
            let f = entry.files.get(position).ok_or(MaterializeError::Raw)?;
            let raw = stored.get(position).ok_or(MaterializeError::Raw)?;
            let expected_name = format!("daily-bars-{}-{symbol}-page-01.json", window.id);
            if f.file_name != expected_name
                || raw.file_name != expected_name
                || f.size_bytes != raw.bytes.len() as u64
                || f.content_hash != ContentHash::from_bytes(&raw.bytes)
                || f.kind != market_data::ResponseKind::Bars
                || !f.response_continuation.as_deref().unwrap_or("").is_empty()
                || f.request.endpoint != DAILY_BARS_PATH
                || f.request.mode != market_data::FetchMode::Credentialed
                || !exact_request(&f.request.query, symbol, window.start, window.end)
            {
                return Err(MaterializeError::Raw);
            }
            if !exact_headers(&f.request.headers) {
                return Err(MaterializeError::Raw);
            }
            let rel = format!("daily-bars/{instrument_id}/{}.json", window.id);
            evidence_files.push(FixedStockPriceBetaRawFileEvidence {
                relative_path: rel.clone(),
                instrument_id: instrument_id.clone(),
                window_id: window.id.into(),
                page_id: "single".into(),
                sha256: hash(&raw.bytes),
                size_bytes: raw.bytes.len() as u64,
                method: "GET".into(),
                path: DAILY_BARS_PATH.into(),
                tr_id: DAILY_BARS_TR_ID.into(),
                query_symbol: (*symbol).into(),
                query_range_start: window.start.into(),
                query_range_end: window.end.into(),
                fid_org_adj_prc: FID_ORG_ADJ_PRC.into(),
                response_continuation: String::new(),
            });
            sources.push(FixedStockPriceBetaRawSourceFile {
                relative_path: rel,
                bytes: raw.bytes.clone(),
            });
        }
    }
    evidence_files.sort_by(|a, b| {
        (&a.instrument_id, &a.window_id, &a.page_id).cmp(&(
            &b.instrument_id,
            &b.window_id,
            &b.page_id,
        ))
    });
    let evidence = FixedStockPriceBetaRawBatchEvidence {
        contract_version: 1,
        provider_scope: RAW_PROVIDER.into(),
        requested_range_start: crate::stock_price_beta_raw::FIXED_RANGE_START.into(),
        requested_range_end: crate::stock_price_beta_raw::FIXED_RANGE_END.into(),
        entitlement_reference: ENTITLEMENT_DOCUMENT_REFERENCE.into(),
        entitlement_sha256: hash(entitlement),
        capture_commit: commit.into(),
        batch_json_sha256: batch_hash,
        manifest_sha256: manifest_hash,
        windows: FIXED_CAPTURE_WINDOWS
            .iter()
            .map(|w| FixedStockPriceBetaRawWindow {
                window_id: w.id.into(),
                range_start: w.start.into(),
                range_end: w.end.into(),
            })
            .collect(),
        files: evidence_files,
    };
    Ok((evidence, sources))
}
fn exact_headers(headers: &[(String, String)]) -> bool {
    headers
        == [
            ("authorization".into(), "[REDACTED]".into()),
            ("appkey".into(), "[REDACTED]".into()),
            ("appsecret".into(), "[REDACTED]".into()),
            ("tr_id".into(), DAILY_BARS_TR_ID.into()),
            ("tr_cont".into(), String::new()),
        ]
}
fn exact_request(query: &[(String, String)], symbol: &str, start: &str, end: &str) -> bool {
    query
        == [
            ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
            ("FID_INPUT_ISCD".into(), symbol.into()),
            ("FID_INPUT_DATE_1".into(), start.replace('-', "")),
            ("FID_INPUT_DATE_2".into(), end.replace('-', "")),
            ("FID_PERIOD_DIV_CODE".into(), "D".into()),
            ("FID_ORG_ADJ_PRC".into(), "1".into()),
        ]
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
pub struct Proposal<'a> {
    pub schema_id: &'static str,
    pub schema_version: u32,
    pub approved_artifacts: Vec<market_data::FixedStockPriceBetaApprovedArtifact>,
    #[serde(skip)]
    pub marker: std::marker::PhantomData<&'a ()>,
}
pub fn proposal(outcome: &MaterializeOutcome) -> Proposal<'static> {
    let a = &outcome.artifact;
    Proposal {
        schema_id:
            market_data::fixed_stock_price_beta_approval::FIXED_STOCK_PRICE_BETA_APPROVAL_SCHEMA_ID,
        schema_version: 1,
        approved_artifacts: vec![market_data::FixedStockPriceBetaApprovedArtifact {
            status: "APPROVED".into(),
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            capability: "PRICE_VOLUME_RESEARCH_ONLY".into(),
            selection_basis: "CONFIGURED_FIXED_LIST".into(),
            index_membership: "NOT_EVALUATED".into(),
            redistribution: "NO_REDISTRIBUTION".into(),
            publication_status: "NOT_PUBLISHED".into(),
            universe_sha256: a.universe_file_sha256.clone(),
            entitlement_sha256: a.evidence.entitlement_sha256.clone(),
            batch_id: outcome.batch_id.to_string(),
            source_file_count: a.evidence.files.len(),
            factor_version: factor_engine::FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION.into(),
            capture_commit: a.evidence.capture_commit.clone(),
            batch_json_sha256: a.evidence.batch_json_sha256.clone(),
            manifest_sha256: a.evidence.manifest_sha256.clone(),
            artifact_content_sha256: a.content_sha256.clone(),
            snapshot_content_sha256: outcome.snapshot.content_sha256.clone(),
            range_start: a.range_start.clone(),
            range_end: a.range_end.clone(),
            as_of: outcome.snapshot.as_of.clone(),
            instruments: a.instruments.clone(),
            instrument_count: 30,
            session_count: a.sessions.len(),
            bar_count: a.bars.len(),
            materialization_status: "MATERIALIZED".into(),
            registration_status: "UNREGISTERED".into(),
        }],
        marker: std::marker::PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate};
    use domain::UtcTimestamp;
    use market_data::{BatchSpec, FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
    use tempfile::tempdir;
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    fn fixture() -> (tempfile::TempDir, MaterializeRequest) {
        let root = tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let universe =
            include_bytes!("../../../configs/universes/kr-stock-price-beta-v1.json").to_vec();
        let entitlement =
            include_bytes!("../../../configs/data-rights/kis.entitlement.json").to_vec();
        let u = parse_universe_bytes(&universe).unwrap();
        let e = parse_entitlement_bytes(&entitlement).unwrap();
        let id = CaptureIdentity::new(
            &u,
            domain::TradingDate::parse(FIXED_RANGE_START).unwrap(),
            domain::TradingDate::parse(crate::stock_price_beta_raw::FIXED_RANGE_END).unwrap(),
            e.document_reference,
            e.file_sha256,
            COMMIT.into(),
        )
        .unwrap()
        .batch_id();
        let mut envelopes = Vec::new();
        for symbol in crate::stock_price_beta_raw::FIXED_STOCK_SYMBOLS {
            for window in FIXED_CAPTURE_WINDOWS {
                let start = NaiveDate::parse_from_str(window.start, "%Y-%m-%d").unwrap();
                let rows: Vec<_> = (0..41).rev().map(|d| serde_json::json!({"stck_bsop_date":(start+Duration::days(d)).format("%Y%m%d").to_string(),"stck_oprc":"10","stck_hgpr":"11","stck_lwpr":"9","stck_clpr":"10","acml_vol":"1"})).collect();
                let body=serde_json::to_vec(&serde_json::json!({"rt_cd":"0","output1":{"stck_shrn_iscd":symbol},"output2":rows})).unwrap();
                let query = vec![
                    ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
                    ("FID_INPUT_ISCD".into(), symbol.into()),
                    ("FID_INPUT_DATE_1".into(), window.start.replace('-', "")),
                    ("FID_INPUT_DATE_2".into(), window.end.replace('-', "")),
                    ("FID_PERIOD_DIV_CODE".into(), "D".into()),
                    ("FID_ORG_ADJ_PRC".into(), "1".into()),
                ];
                let headers = vec![
                    ("authorization".into(), "[REDACTED]".into()),
                    ("appkey".into(), "[REDACTED]".into()),
                    ("appsecret".into(), "[REDACTED]".into()),
                    ("tr_id".into(), DAILY_BARS_TR_ID.into()),
                    ("tr_cont".into(), String::new()),
                ];
                envelopes.push(RawEnvelope::new(
                    id,
                    ResponseKind::Bars,
                    format!("daily-bars-{}-{symbol}-page-01.json", window.id),
                    body,
                    UtcTimestamp::parse_rfc3339("2026-08-30T00:00:00Z").unwrap(),
                    RequestMetadata {
                        endpoint: DAILY_BARS_PATH.into(),
                        query,
                        headers,
                        mode: FetchMode::Credentialed,
                    },
                ));
            }
        }
        let store = RawStore::new(root.path());
        let date = domain::TradingDate::parse(FIXED_RANGE_START).unwrap();
        store
            .store_batch(
                &BatchSpec {
                    provider: RAW_PROVIDER,
                    market: RAW_MARKET,
                    date: &date,
                    batch_id: id,
                    entitlement_reference: Some(ENTITLEMENT_DOCUMENT_REFERENCE),
                    mode: FetchMode::Credentialed,
                },
                &envelopes,
            )
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                store.manifest_path(RAW_PROVIDER, RAW_MARKET),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let artifact_root = root.path().join("artifacts");
        fs::create_dir(&artifact_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let raw_root = root.path().to_path_buf();
        (
            root,
            MaterializeRequest {
                raw_root,
                artifact_root,
                universe_bytes: universe,
                entitlement_bytes: entitlement,
                batch_id: id,
                capture_commit: COMMIT.into(),
            },
        )
    }
    #[test]
    fn full_30x3_fixture_materializes_and_proposal_is_canonical() {
        let (_root, request) = fixture();
        validate_request(&request).unwrap();
        let store = RawStore::new(&request.raw_root);
        let entry = find_entry(&store, request.batch_id).unwrap();
        let (batch_hash, manifest_hash) = verify_metadata(&store, &entry).unwrap();
        let (evidence, sources) = evidence_and_sources(
            &store,
            &entry,
            &request.entitlement_bytes,
            &request.capture_commit,
            batch_hash,
            manifest_hash,
        )
        .unwrap();
        assert_eq!(
            parse_fixed_stock_price_beta_raw_sources(&evidence, &sources)
                .unwrap()
                .len(),
            3690
        );
        let out = materialize(&request).unwrap();
        assert_eq!(out.artifact.evidence.files.len(), 90);
        assert_eq!(out.artifact.sessions.len(), 123);
        assert_eq!(out.artifact.bars.len(), 3690);
        let registry = serde_json::to_vec(&proposal(&out)).unwrap();
        check(&request, &registry).unwrap();
        assert_eq!(
            serde_json::to_vec(&proposal(&out)).unwrap(),
            serde_json::to_vec(&proposal(&out)).unwrap()
        );
    }

    #[test]
    fn tamper_matrix_fails_closed_at_every_boundary() {
        let (_root, request) = fixture();
        let store = RawStore::new(&request.raw_root);
        let entry = find_entry(&store, request.batch_id).unwrap();
        let (batch_hash, manifest_hash) = verify_metadata(&store, &entry).unwrap();
        let (evidence, sources) = evidence_and_sources(
            &store,
            &entry,
            &request.entitlement_bytes,
            &request.capture_commit,
            batch_hash,
            manifest_hash,
        )
        .unwrap();
        let mut wrong_id = request.clone();
        wrong_id.batch_id = BatchId::generate();
        assert!(validate_request(&wrong_id).is_err());
        let mut no_rights = entry.clone();
        no_rights.entitlement_reference = None;
        assert!(!entry_contract_ok(&no_rights));
        let mut synthetic = entry.clone();
        synthetic.mode = FetchMode::Synthetic;
        assert!(!entry_contract_ok(&synthetic));
        let mut cases: Vec<fn(&mut market_data::FileEntry)> = Vec::new();
        cases.push(|f| f.file_name = "bad.json".into());
        cases.push(|f| f.request.endpoint = "bad".into());
        cases.push(|f| f.request.query.clear());
        cases.push(|f| f.request.headers.clear());
        cases.push(|f| f.response_continuation = Some("M".into()));
        for mutate in cases {
            let mut e = entry.clone();
            mutate(&mut e.files[0]);
            assert!(
                evidence_and_sources(
                    &store,
                    &e,
                    &request.entitlement_bytes,
                    &request.capture_commit,
                    "a".repeat(64),
                    "b".repeat(64)
                )
                .is_err()
            );
        }
        for body in [
            b"{".to_vec(),
            b"{\"rt_cd\":\"1\"}".to_vec(),
            b"{\"rt_cd\":\"0\",\"output1\":{},\"output2\":[]}".to_vec(),
        ] {
            let mut ss = sources.clone();
            ss[0].bytes = body;
            assert!(parse_fixed_stock_price_beta_raw_sources(&evidence, &ss).is_err());
        }
        let mut bad_hash = evidence.clone();
        bad_hash.files[0].sha256 = "0".repeat(64);
        assert!(parse_fixed_stock_price_beta_raw_sources(&bad_hash, &sources).is_err());
        let mut missing = sources.clone();
        missing.pop();
        assert!(parse_fixed_stock_price_beta_raw_sources(&evidence, &missing).is_err());
        let mut extra = sources.clone();
        extra.push(sources[0].clone());
        assert!(parse_fixed_stock_price_beta_raw_sources(&evidence, &extra).is_err());
        let mut duplicate = sources.clone();
        duplicate[1].relative_path = duplicate[0].relative_path.clone();
        assert!(parse_fixed_stock_price_beta_raw_sources(&evidence, &duplicate).is_err());
        let artifact = FixedStockPriceBetaArtifact::build(
            &request.universe_bytes,
            evidence.clone(),
            sources.clone(),
            parse_fixed_stock_price_beta_raw_sources(&evidence, &sources).unwrap(),
        )
        .unwrap();
        let mut artifact_tamper = artifact.clone();
        artifact_tamper.bars[0].close += 1;
        artifact_tamper.content_sha256 = artifact_tamper.compute_hash().unwrap();
        assert!(
            artifact_tamper
                .verify_against_raw_sources(&request.universe_bytes, &sources)
                .is_err()
        );
        let mut snapshot =
            PriceVolumeSignalSnapshot::compute(&artifact, artifact.sessions.last().unwrap())
                .unwrap();
        snapshot.rows[0].rank += 1;
        snapshot.content_sha256 = snapshot.compute_hash().unwrap();
        assert!(snapshot.verify_against(&artifact).is_err());
    }

    #[test]
    fn rehashed_semantic_raw_body_mutations_reach_the_parser() {
        let (_root, request) = fixture();
        let store = RawStore::new(&request.raw_root);
        let entry = find_entry(&store, request.batch_id).unwrap();
        let (batch_hash, manifest_hash) = verify_metadata(&store, &entry).unwrap();
        let (evidence, sources) = evidence_and_sources(
            &store,
            &entry,
            &request.entitlement_bytes,
            &request.capture_commit,
            batch_hash,
            manifest_hash,
        )
        .unwrap();
        let mut cases: Vec<fn(&mut serde_json::Value)> = Vec::new();
        cases.push(|value| {
            value.as_object_mut().unwrap().insert(
                "ctx_area_fk100".into(),
                serde_json::Value::String("cursor".into()),
            );
        });
        cases.push(|value| {
            value["output2"][0]["stck_hgpr"] = serde_json::Value::String("8".into());
        });
        cases.push(|value| {
            value["output2"][0]["acml_vol"] = serde_json::Value::String("-1".into());
        });
        cases.push(|value| {
            value["output2"][1]["stck_bsop_date"] = value["output2"][0]["stck_bsop_date"].clone();
        });
        cases.push(|value| {
            value["output1"]["stck_shrn_iscd"] = serde_json::Value::String("999999".into());
        });
        for mutate in cases {
            let mut sources = sources.clone();
            let mut evidence = evidence.clone();
            let mut body: serde_json::Value = serde_json::from_slice(&sources[0].bytes).unwrap();
            mutate(&mut body);
            sources[0].bytes = serde_json::to_vec(&body).unwrap();
            let index = evidence
                .files
                .iter()
                .position(|file| file.relative_path == sources[0].relative_path)
                .unwrap();
            evidence.files[index].sha256 = hash(&sources[0].bytes);
            evidence.files[index].size_bytes = sources[0].bytes.len() as u64;
            assert!(parse_fixed_stock_price_beta_raw_sources(&evidence, &sources).is_err());
        }
    }
}
