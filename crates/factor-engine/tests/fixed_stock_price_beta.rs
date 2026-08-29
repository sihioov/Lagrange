use std::path::Path;

use chrono::{Duration, NaiveDate};
use factor_engine::fixed_stock_price_beta::{
    BULLISH_RETURN_20_MIN, BULLISH_VOLATILITY_120_MAX, FIXED_STOCK_PRICE_BETA_FACTOR_VERSION,
    FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL, FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE,
    FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY, FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP,
    FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID, FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION,
    FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS, FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID,
    PRICE_VOLUME_SIGNAL_AUDIENCE, PRICE_VOLUME_SIGNAL_CAPABILITY, PriceVolumeSignalError,
    PriceVolumeSignalSnapshot, ResearchCondition, WEIGHT_ACTIVITY, WEIGHT_DRAWDOWN,
    WEIGHT_RETURN_20, WEIGHT_RETURN_60, WEIGHT_RETURN_120, WEIGHT_TREND,
    read_fixed_stock_price_beta_snapshot, read_fixed_stock_price_beta_snapshot_against,
    write_fixed_stock_price_beta_snapshot_against,
};
use factor_engine::fixed_stock_price_beta::{
    FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING as SIGNAL_WARNING, PRICE_VOLUME_SIGNAL_WARNING,
};
use market_data::{
    DailyBar, FIXED_30_INSTRUMENT_IDS, FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256,
    FixedStockPriceBetaArtifact, FixedStockPriceBetaRawBatchEvidence,
    FixedStockPriceBetaRawFileEvidence, FixedStockPriceBetaRawSourceFile,
    FixedStockPriceBetaRawWindow, parse_fixed_stock_price_beta_universe,
};
use sha2::{Digest, Sha256};

const UNIVERSE: &[u8] = include_bytes!("../../../configs/universes/kr-stock-price-beta-v1.json");

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sources() -> Vec<FixedStockPriceBetaRawSourceFile> {
    FIXED_30_INSTRUMENT_IDS
        .iter()
        .map(|id| FixedStockPriceBetaRawSourceFile {
            relative_path: format!("daily-bars/{id}/full.json"),
            bytes: format!("raw-{id}").into_bytes(),
        })
        .collect()
}

fn evidence(source: &[FixedStockPriceBetaRawSourceFile]) -> FixedStockPriceBetaRawBatchEvidence {
    let mut files: Vec<_> = source
        .iter()
        .zip(FIXED_30_INSTRUMENT_IDS)
        .map(|(raw, id)| FixedStockPriceBetaRawFileEvidence {
            relative_path: raw.relative_path.clone(),
            instrument_id: (*id).to_owned(),
            window_id: "full".to_owned(),
            page_id: "single".to_owned(),
            sha256: sha256(&raw.bytes),
            size_bytes: raw.bytes.len() as u64,
            method: "GET".to_owned(),
            path: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".to_owned(),
            tr_id: "FHKST03010100".to_owned(),
            query_symbol: id.strip_suffix(".KRX").unwrap().to_owned(),
            query_range_start: "2025-08-04".to_owned(),
            query_range_end: "2026-08-28".to_owned(),
            fid_org_adj_prc: "1".to_owned(),
            response_continuation: String::new(),
        })
        .collect();
    files.sort_by(|left, right| {
        (&left.instrument_id, &left.window_id, &left.page_id).cmp(&(
            &right.instrument_id,
            &right.window_id,
            &right.page_id,
        ))
    });
    FixedStockPriceBetaRawBatchEvidence {
        contract_version: 1,
        provider_scope: "kis-fixed-stock-price-beta-daily-bars-raw-v1".to_owned(),
        requested_range_start: "2025-08-04".to_owned(),
        requested_range_end: "2026-08-28".to_owned(),
        entitlement_reference: "vault://entitlement/fixed-stock-price-beta".to_owned(),
        entitlement_sha256: "a".repeat(64),
        capture_commit: "abcdef1".to_owned(),
        batch_json_sha256: "b".repeat(64),
        manifest_sha256: "c".repeat(64),
        windows: vec![FixedStockPriceBetaRawWindow {
            window_id: "full".to_owned(),
            range_start: "2025-08-04".to_owned(),
            range_end: "2026-08-28".to_owned(),
        }],
        files,
    }
}

fn bars(days: usize, slope: i64) -> Vec<DailyBar> {
    let start = NaiveDate::from_ymd_opt(2025, 8, 4).unwrap();
    FIXED_30_INSTRUMENT_IDS
        .iter()
        .flat_map(|id| {
            (0..days).map(move |day| {
                let close = if slope == 0 {
                    10_000
                } else if slope > 0 {
                    10_000 + slope * day as i64
                } else {
                    20_000 + slope * day as i64
                };
                DailyBar {
                    instrument_id: (*id).to_owned(),
                    date: (start + Duration::days(day as i64)).to_string(),
                    open: close,
                    high: close + 2,
                    low: close - 2,
                    close,
                    volume: 1_000,
                }
            })
        })
        .collect()
}

fn artifact(days: usize, slope: i64) -> FixedStockPriceBetaArtifact {
    let source = sources();
    FixedStockPriceBetaArtifact::build(UNIVERSE, evidence(&source), source, bars(days, slope))
        .unwrap()
}

fn rehash(snapshot: &mut PriceVolumeSignalSnapshot) {
    snapshot.content_sha256 = snapshot.compute_hash().unwrap();
}

fn snapshot(days: usize, slope: i64) -> (FixedStockPriceBetaArtifact, PriceVolumeSignalSnapshot) {
    let value = artifact(days, slope);
    let as_of = value.sessions[120].clone();
    let signal = PriceVolumeSignalSnapshot::compute(&value, &as_of).unwrap();
    (value, signal)
}

#[test]
fn computed_snapshot_is_canonical_across_json_round_trip() {
    let mut value = artifact(261, 1);
    for (index, bar) in value.bars.iter_mut().enumerate() {
        let delta = ((index * 7_919 + index / 17) % 2_003) as i64;
        bar.close += delta;
        bar.open = bar.close - 3;
        bar.high = bar.close + 7;
        bar.low = bar.close - 11;
        bar.volume += ((index * 104_729) % 90_001) as i64;
    }
    value.content_sha256 = value.compute_hash().unwrap();
    let signal =
        PriceVolumeSignalSnapshot::compute(&value, value.sessions.last().unwrap()).unwrap();
    let bytes = serde_json::to_vec(&signal).unwrap();
    let reparsed: PriceVolumeSignalSnapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reparsed.compute_hash().unwrap(), signal.content_sha256);
    reparsed.verify_against(&value).unwrap();
}

#[cfg(unix)]
fn secure_root(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn fixture_is_bound_to_exact_checked_in_universe_and_raw_matrix() {
    let universe = parse_fixed_stock_price_beta_universe(UNIVERSE).unwrap();
    assert_eq!(
        universe.file_sha256,
        FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
    );
    let value = artifact(121, 1);
    assert_eq!(value.instruments.len(), 30);
    assert_eq!(value.sessions.len(), 121);
    assert_eq!(value.bars.len(), 30 * 121);
    assert_eq!(value.universe_id, FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID);
    assert_eq!(
        value.universe_file_sha256,
        FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
    );
    assert_eq!(value.evidence.files.len(), 30);
}

#[test]
fn true_return_windows_require_121_observations_and_boundaries_are_exact() {
    let only_120 = artifact(120, 1);
    let last_120 = only_120.sessions[119].clone();
    assert!(PriceVolumeSignalSnapshot::compute(&only_120, &last_120).is_err());

    let (value, signal) = snapshot(121, 20);
    assert_eq!(signal.as_of, value.sessions[120]);
    let row = &signal.rows[0];
    assert!((row.return_20 - (12_400.0 / 12_000.0 - 1.0)).abs() < 1.0e-15);
    assert!((row.return_60 - (12_400.0 / 11_200.0 - 1.0)).abs() < 1.0e-15);
    assert!((row.return_120 - (12_400.0 / 10_000.0 - 1.0)).abs() < 1.0e-15);
    assert!(signal.verify_against(&value).is_ok());

    let mut with_future = artifact(122, 20);
    let as_of = with_future.sessions[120].clone();
    for bar in &mut with_future.bars {
        if bar.date > as_of {
            bar.open += 500_000;
            bar.close += 500_000;
            bar.high = bar.close + 2;
            bar.low = bar.close - 2;
        }
    }
    with_future.content_sha256 = with_future.compute_hash().unwrap();
    let future_signal = PriceVolumeSignalSnapshot::compute(&with_future, &as_of).unwrap();
    assert_eq!(future_signal.rows, signal.rows);
}

#[test]
fn flat_up_down_conditions_are_exclusive_and_ties_use_id_order() {
    let (_, flat) = snapshot(121, 0);
    assert!(
        flat.rows
            .windows(2)
            .all(|pair| pair[0].score == pair[1].score)
    );
    let ids: Vec<_> = flat
        .rows
        .iter()
        .map(|row| row.instrument_id.as_str())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted);
    assert!(
        flat.rows
            .iter()
            .enumerate()
            .all(|(i, row)| row.rank == i + 1)
    );
    assert!(
        flat.rows
            .iter()
            .all(|row| row.condition == ResearchCondition::Neutral)
    );

    let (_, up) = snapshot(121, 20);
    assert!(
        up.rows
            .iter()
            .all(|row| row.condition == ResearchCondition::Bullish)
    );
    assert!(
        up.rows
            .iter()
            .all(|row| row.return_20 >= BULLISH_RETURN_20_MIN)
    );
    assert!(
        up.rows
            .iter()
            .all(|row| row.volatility_120 <= BULLISH_VOLATILITY_120_MAX)
    );

    let (_, down) = snapshot(121, -20);
    assert!(
        down.rows
            .iter()
            .all(|row| row.condition == ResearchCondition::Bearish)
    );
    for row in &down.rows {
        assert!(
            [
                ResearchCondition::Bullish,
                ResearchCondition::Neutral,
                ResearchCondition::Bearish,
            ]
            .iter()
            .filter(|condition| **condition == row.condition)
            .count()
                == 1
        );
    }
}

#[test]
fn snapshot_metadata_and_public_schema_are_closed() {
    let (value, signal) = snapshot(121, 0);
    assert_eq!(signal.schema_id, FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID);
    assert_eq!(
        signal.schema_version,
        FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION
    );
    assert_eq!(signal.factor_version, FIXED_STOCK_PRICE_BETA_FACTOR_VERSION);
    assert_eq!(signal.audience, FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE);
    assert_eq!(signal.capability, FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY);
    assert_eq!(signal.audience, PRICE_VOLUME_SIGNAL_AUDIENCE);
    assert_eq!(signal.capability, PRICE_VOLUME_SIGNAL_CAPABILITY);
    assert!(signal.vendor_snapshot);
    assert!(!signal.strict_pit);
    assert_eq!(signal.universe_id, value.universe_id);
    assert_eq!(signal.universe_file_sha256, value.universe_file_sha256);
    assert_eq!(
        signal.selection_basis,
        FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS
    );
    assert_eq!(
        signal.index_membership,
        FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP
    );
    assert!(signal.original_price);
    assert_eq!(signal.warning, SIGNAL_WARNING);
    assert_eq!(signal.warning, PRICE_VOLUME_SIGNAL_WARNING);
    assert_eq!(
        signal.activity_label,
        FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL
    );
    assert_eq!(
        signal.activity_label,
        "Activity/liquidity proxy, not execution liquidity"
    );
    assert_eq!(signal.compute_hash().unwrap(), signal.content_sha256);
    assert!(
        (WEIGHT_RETURN_20
            + WEIGHT_RETURN_60
            + WEIGHT_RETURN_120
            + WEIGHT_TREND
            + WEIGHT_ACTIVITY
            + WEIGHT_DRAWDOWN
            - 1.0)
            .abs()
            < f64::EPSILON
    );
}

#[test]
fn structural_verification_recomputes_scores_conditions_ranks_and_exact_ids() {
    let (_, signal) = snapshot(121, 0);
    signal.verify().unwrap();

    let mut score = signal.clone();
    score.rows[0].score += 0.01;
    rehash(&mut score);
    assert_eq!(score.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut rank = signal.clone();
    rank.rows[0].rank = 30;
    rank.rows[29].rank = 1;
    rehash(&mut rank);
    assert_eq!(rank.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut condition = signal.clone();
    condition.rows[0].condition = ResearchCondition::Bullish;
    rehash(&mut condition);
    assert_eq!(condition.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut duplicate = signal.clone();
    duplicate.rows[0].instrument_id = duplicate.rows[1].instrument_id.clone();
    rehash(&mut duplicate);
    assert_eq!(duplicate.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut replaced = signal.clone();
    replaced.rows[0].instrument_id = "999999.KRX".to_owned();
    rehash(&mut replaced);
    assert_eq!(replaced.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut nonfinite = signal.clone();
    nonfinite.rows[0].return_20 = f64::NAN;
    assert_eq!(nonfinite.verify(), Err(PriceVolumeSignalError::Tampered));
    let mut zero_volume = artifact(121, 0);
    for bar in &mut zero_volume.bars {
        bar.volume = 0;
    }
    zero_volume.content_sha256 = zero_volume.compute_hash().unwrap();
    assert!(PriceVolumeSignalSnapshot::compute(&zero_volume, &zero_volume.sessions[120]).is_err());
}

#[test]
fn every_snapshot_metadata_and_provenance_field_is_verified() {
    let (value, signal) = snapshot(121, 0);
    for field in 0..13 {
        let mut tampered = signal.clone();
        match field {
            0 => tampered.schema_id = "other".to_owned(),
            1 => tampered.schema_version = 2,
            2 => tampered.factor_version = "other".to_owned(),
            3 => tampered.audience = "PUBLIC".to_owned(),
            4 => tampered.capability = "TRADING".to_owned(),
            5 => tampered.vendor_snapshot = false,
            6 => tampered.strict_pit = true,
            7 => tampered.universe_id = "other".to_owned(),
            8 => tampered.universe_file_sha256 = "a".repeat(64),
            9 => tampered.selection_basis = "OTHER".to_owned(),
            10 => tampered.index_membership = "KOSPI_200".to_owned(),
            11 => tampered.original_price = false,
            _ => tampered.warning = "warning changed".to_owned(),
        }
        rehash(&mut tampered);
        assert_eq!(tampered.verify(), Err(PriceVolumeSignalError::Tampered));
    }

    let mut activity = signal.clone();
    activity.activity_label = "execution liquidity".to_owned();
    rehash(&mut activity);
    assert_eq!(activity.verify(), Err(PriceVolumeSignalError::Tampered));

    let mut row_fields = signal.clone();
    for field in 0..12 {
        let mut tampered = row_fields.clone();
        let row = &mut tampered.rows[0];
        match field {
            0 => row.return_20 += 0.01,
            1 => row.return_60 += 0.01,
            2 => row.return_120 += 0.01,
            3 => row.volatility_20 += 0.01,
            4 => row.volatility_60 += 0.01,
            5 => row.volatility_120 += 0.01,
            6 => row.max_drawdown_120 -= 0.01,
            7 => row.sma_20 += 0.01,
            8 => row.sma_60 += 0.01,
            9 => row.average_volume_20 += 0.01,
            10 => row.volume_ratio_20_60 += 0.01,
            _ => row.average_trading_value_20 += 0.01,
        }
        rehash(&mut tampered);
        assert_eq!(
            tampered.verify_against(&value),
            Err(PriceVolumeSignalError::Tampered),
            "row factor field {field} must be recomputed from bars"
        );
        row_fields = tampered;
    }

    let mut as_of = signal.clone();
    let later = artifact(122, 0);
    as_of.as_of = later.sessions[121].clone();
    rehash(&mut as_of);
    assert!(as_of.verify().is_ok());
    assert_eq!(
        as_of.verify_against(&later),
        Err(PriceVolumeSignalError::Tampered)
    );

    let mut artifact_hash = signal.clone();
    artifact_hash.artifact_content_sha256 = "f".repeat(64);
    rehash(&mut artifact_hash);
    assert!(artifact_hash.verify().is_ok());
    assert_eq!(
        artifact_hash.verify_against(&value),
        Err(PriceVolumeSignalError::Tampered)
    );
}

#[test]
fn source_bound_verification_recomputes_all_factors_and_rejects_artifact_mismatch() {
    let (value, signal) = snapshot(121, 0);
    signal.verify_against(&value).unwrap();

    // All rows receive the same factor edit, so a self-hash and the derived
    // percentile scores remain internally consistent.  Only the artifact-bound
    // verifier can detect that the raw factor was replaced.
    let mut factor = signal.clone();
    for row in &mut factor.rows {
        row.return_20 += 0.001;
    }
    rehash(&mut factor);
    factor.verify().unwrap();
    assert_eq!(
        factor.verify_against(&value),
        Err(PriceVolumeSignalError::Tampered)
    );

    let other = artifact(121, 1);
    assert_eq!(
        signal.verify_against(&other),
        Err(PriceVolumeSignalError::Tampered)
    );
}

#[test]
fn read_against_rejects_recomputed_snapshot_and_manifest_tampering() {
    let (value, signal) = snapshot(121, 0);
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    secure_root(temp.path());
    let directory =
        write_fixed_stock_price_beta_snapshot_against(temp.path(), &signal, &value).unwrap();
    let original = std::fs::read(directory.join("snapshot.json")).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot_against(temp.path(), &signal.content_sha256, &value)
            .unwrap(),
        signal
    );

    let mut tampered = signal.clone();
    for row in &mut tampered.rows {
        row.return_20 += 0.001;
    }
    rehash(&mut tampered);
    let bytes = serde_json::to_vec(&tampered).unwrap();
    std::fs::write(directory.join("snapshot.json"), &bytes).unwrap();
    std::fs::write(directory.join("snapshot.sha256"), sha256(&bytes)).unwrap();
    let tampered_directory = temp.path().join(&tampered.content_sha256);
    std::fs::rename(&directory, &tampered_directory).unwrap();
    assert!(read_fixed_stock_price_beta_snapshot(temp.path(), &tampered.content_sha256).is_ok());
    assert_eq!(
        read_fixed_stock_price_beta_snapshot_against(temp.path(), &tampered.content_sha256, &value),
        Err(PriceVolumeSignalError::Tampered)
    );
    assert_ne!(original, bytes);
}

#[cfg(unix)]
#[test]
fn descriptor_safe_storage_is_idempotent_and_rejects_tamper_permissions_conflicts_and_orphans() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let (value, signal) = snapshot(121, 0);
    let temp = tempfile::tempdir().unwrap();
    secure_root(temp.path());
    let directory =
        write_fixed_stock_price_beta_snapshot_against(temp.path(), &signal, &value).unwrap();
    let snapshot_bytes = std::fs::read(directory.join("snapshot.json")).unwrap();
    let manifest_bytes = std::fs::read(directory.join("snapshot.sha256")).unwrap();
    assert_eq!(
        directory,
        write_fixed_stock_price_beta_snapshot_against(temp.path(), &signal, &value).unwrap()
    );
    assert_eq!(
        snapshot_bytes,
        std::fs::read(directory.join("snapshot.json")).unwrap()
    );
    assert_eq!(
        manifest_bytes,
        std::fs::read(directory.join("snapshot.sha256")).unwrap()
    );
    assert_eq!(
        std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(directory.join("snapshot.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(directory.join("snapshot.sha256"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    std::fs::set_permissions(
        directory.join("snapshot.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(temp.path(), &signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );
    std::fs::set_permissions(
        directory.join("snapshot.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::set_permissions(
        directory.join("snapshot.sha256"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(temp.path(), &signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );

    std::fs::set_permissions(
        directory.join("snapshot.sha256"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::write(directory.join("snapshot.json"), b"{}").unwrap();
    assert!(read_fixed_stock_price_beta_snapshot(temp.path(), &signal.content_sha256).is_err());
    assert_eq!(
        write_fixed_stock_price_beta_snapshot_against(temp.path(), &signal, &value),
        Err(PriceVolumeSignalError::Conflict)
    );

    let orphan = tempfile::tempdir().unwrap();
    secure_root(orphan.path());
    std::fs::create_dir(orphan.path().join(".orphan-stage")).unwrap();
    assert_eq!(
        write_fixed_stock_price_beta_snapshot_against(orphan.path(), &signal, &value),
        Err(PriceVolumeSignalError::Conflict)
    );

    let root_mode = tempfile::tempdir().unwrap();
    secure_root(root_mode.path());
    let root_mode_signal = snapshot(121, 0).1;
    let _ =
        write_fixed_stock_price_beta_snapshot_against(root_mode.path(), &root_mode_signal, &value);
    std::fs::set_permissions(root_mode.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(root_mode.path(), &root_mode_signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );

    let symlink_root = tempfile::tempdir().unwrap();
    secure_root(symlink_root.path());
    let real = symlink_root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    secure_root(&real);
    let link = symlink_root.path().join("root-link");
    symlink(&real, &link).unwrap();
    assert_eq!(
        write_fixed_stock_price_beta_snapshot_against(&link, &signal, &value),
        Err(PriceVolumeSignalError::UnsafePath)
    );
}

#[cfg(unix)]
#[test]
fn descriptor_safe_storage_rejects_symlinks_at_parent_leaf_snapshot_and_manifest() {
    use std::os::unix::fs::symlink;

    let (value, signal) = snapshot(121, 0);

    let parent = tempfile::tempdir().unwrap();
    secure_root(parent.path());
    let real_parent = parent.path().join("real-parent");
    std::fs::create_dir(&real_parent).unwrap();
    secure_root(&real_parent);
    let real_root = real_parent.join("store");
    std::fs::create_dir(&real_root).unwrap();
    secure_root(&real_root);
    let parent_link = parent.path().join("parent-link");
    symlink(&real_parent, &parent_link).unwrap();
    let through_parent_link = parent_link.join("store");
    assert_eq!(
        write_fixed_stock_price_beta_snapshot_against(&through_parent_link, &signal, &value),
        Err(PriceVolumeSignalError::UnsafePath)
    );

    let leaf_root = tempfile::tempdir().unwrap();
    secure_root(leaf_root.path());
    let leaf_target = leaf_root.path().join("leaf-target");
    std::fs::create_dir(&leaf_target).unwrap();
    secure_root(&leaf_target);
    symlink(&leaf_target, leaf_root.path().join(&signal.content_sha256)).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(leaf_root.path(), &signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );

    let file_root = tempfile::tempdir().unwrap();
    secure_root(file_root.path());
    let directory =
        write_fixed_stock_price_beta_snapshot_against(file_root.path(), &signal, &value).unwrap();
    std::fs::remove_file(directory.join("snapshot.json")).unwrap();
    symlink("/tmp", directory.join("snapshot.json")).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(file_root.path(), &signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );

    let manifest_root = tempfile::tempdir().unwrap();
    secure_root(manifest_root.path());
    let manifest_dir =
        write_fixed_stock_price_beta_snapshot_against(manifest_root.path(), &signal, &value)
            .unwrap();
    std::fs::remove_file(manifest_dir.join("snapshot.sha256")).unwrap();
    symlink("/tmp", manifest_dir.join("snapshot.sha256")).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_snapshot(manifest_root.path(), &signal.content_sha256),
        Err(PriceVolumeSignalError::UnsafePath)
    );
}
