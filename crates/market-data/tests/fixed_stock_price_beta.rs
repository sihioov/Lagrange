use std::path::Path;

use chrono::{Duration, NaiveDate};
use market_data::{
    DailyBar, FIXED_30_ID_LIST_SHA256, FIXED_30_INSTRUMENT_IDS,
    FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256, FixedStockPriceBetaArtifact,
    FixedStockPriceBetaError, FixedStockPriceBetaRawBatchEvidence,
    FixedStockPriceBetaRawFileEvidence, FixedStockPriceBetaRawSourceFile,
    FixedStockPriceBetaRawWindow, ORIGINAL_PRICE_WARNING, parse_fixed_stock_price_beta_universe,
    read_fixed_stock_price_beta_artifact, write_fixed_stock_price_beta_artifact,
};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn universe() -> &'static [u8] {
    include_bytes!("../../../configs/universes/kr-stock-price-beta-v1.json")
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
        .map(|(s, id)| FixedStockPriceBetaRawFileEvidence {
            relative_path: s.relative_path.clone(),
            instrument_id: (*id).to_owned(),
            window_id: "full".to_owned(),
            page_id: "single".to_owned(),
            sha256: hex(&s.bytes),
            size_bytes: s.bytes.len() as u64,
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
    files.sort_by(|a, b| {
        (&a.instrument_id, &a.window_id, &a.page_id).cmp(&(
            &b.instrument_id,
            &b.window_id,
            &b.page_id,
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
fn bars(days: usize) -> Vec<DailyBar> {
    let start = NaiveDate::from_ymd_opt(2025, 8, 4).unwrap();
    FIXED_30_INSTRUMENT_IDS
        .iter()
        .flat_map(|id| {
            (0..days).map(move |d| {
                let c = 10_000 + d as i64;
                DailyBar {
                    instrument_id: (*id).to_owned(),
                    date: (start + Duration::days(d as i64)).to_string(),
                    open: c,
                    high: c + 10,
                    low: c - 10,
                    close: c,
                    volume: 100 + d as i64,
                }
            })
        })
        .collect()
}
fn artifact(days: usize) -> FixedStockPriceBetaArtifact {
    let s = sources();
    FixedStockPriceBetaArtifact::build(universe(), evidence(&s), s, bars(days)).unwrap()
}
#[cfg(unix)]
fn secure_root(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn universe_exact_bytes_and_claims_are_pinned() {
    let parsed = parse_fixed_stock_price_beta_universe(universe()).unwrap();
    assert_eq!(
        parsed.file_sha256,
        FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
    );
    assert_eq!(parsed.instruments.len(), 30);
    assert_eq!(
        hex(format!("{}\n", FIXED_30_INSTRUMENT_IDS.join("\n")).as_bytes()),
        FIXED_30_ID_LIST_SHA256
    );
    for offset in [0, 80, 200, universe().len() - 2] {
        let mut b = universe().to_vec();
        b[offset] ^= 1;
        assert!(parse_fixed_stock_price_beta_universe(&b).is_err());
    }
    for (from, to) in [
        ("005930.KRX", "000001.KRX"),
        ("삼성전자", "다른이름"),
        ("000660.KRX", "005930.KRX"),
        ("OWNER_ONLY", "PUBLIC"),
    ] {
        let changed = String::from_utf8(universe().to_vec())
            .unwrap()
            .replacen(from, to, 1);
        assert!(parse_fixed_stock_price_beta_universe(changed.as_bytes()).is_err());
    }
}
#[test]
fn accepts_120_and_rejects_119_and_incomplete_matrix() {
    assert!(
        FixedStockPriceBetaArtifact::build(universe(), evidence(&sources()), sources(), bars(119))
            .is_err()
    );
    let a = artifact(120);
    assert_eq!(a.sessions.len(), 120);
    assert_eq!(a.bars.len(), 3600);
    let mut b = bars(120);
    b.pop();
    assert!(
        FixedStockPriceBetaArtifact::build(universe(), evidence(&sources()), sources(), b).is_err()
    );
}
#[test]
fn raw_source_bijection_and_request_contract_fail_closed() {
    let s = sources();
    let e = evidence(&s);
    let mut missing = s.clone();
    missing.pop();
    assert_eq!(
        FixedStockPriceBetaArtifact::build(universe(), e.clone(), missing, bars(120)),
        Err(FixedStockPriceBetaError::SourceTampered)
    );
    let mut extra = s.clone();
    extra.push(FixedStockPriceBetaRawSourceFile {
        relative_path: "extra".into(),
        bytes: vec![1],
    });
    assert_eq!(
        FixedStockPriceBetaArtifact::build(universe(), e.clone(), extra, bars(120)),
        Err(FixedStockPriceBetaError::SourceTampered)
    );
    let mut duplicate = s.clone();
    duplicate.push(duplicate[0].clone());
    assert_eq!(
        FixedStockPriceBetaArtifact::build(universe(), e.clone(), duplicate, bars(120)),
        Err(FixedStockPriceBetaError::SourceTampered)
    );
    for mutate in [0, 1, 2, 3, 4, 5, 6] {
        let mut x = e.clone();
        match mutate {
            0 => x.files[0].sha256 = "0".repeat(64),
            1 => x.files[0].size_bytes += 1,
            2 => x.files[0].query_symbol = "000000".into(),
            3 => x.files[0].query_range_end = "2025-08-27".into(),
            4 => x.files[0].response_continuation = "M".into(),
            5 => x.files[0].instrument_id = "999999.KRX".into(),
            _ => x.files[0].window_id = "other".into(),
        };
        assert!(FixedStockPriceBetaArtifact::build(universe(), x, s.clone(), bars(120)).is_err());
    }
}
#[test]
fn evidence_binds_actual_complete_symbol_window_matrix_without_three_window_claim() {
    let s = sources();
    let mut e = evidence(&s);
    e.windows.push(FixedStockPriceBetaRawWindow {
        window_id: "later".into(),
        range_start: "2026-01-01".into(),
        range_end: "2026-08-28".into(),
    });
    assert!(FixedStockPriceBetaArtifact::build(universe(), e, s, bars(120)).is_err());
    let a = artifact(120);
    assert_eq!(a.evidence.windows.len(), 1);
    assert!(a.original_price);
    assert_eq!(a.warning, ORIGINAL_PRICE_WARNING);
    assert_eq!(a.index_membership, "NOT_EVALUATED");
}
#[test]
fn recomputed_content_hash_does_not_bypass_derived_invariants() {
    for change in 0..6 {
        let mut a = artifact(120);
        match change {
            0 => a.sessions.reverse(),
            1 => a.bars.reverse(),
            2 => a.instruments.reverse(),
            3 => a.evidence.files[0].method = "POST".into(),
            4 => a.range_end = "2026-08-27".into(),
            _ => a.bars[0].close = a.bars[0].high + 1,
        };
        a.content_sha256 = a.compute_hash().unwrap();
        assert_eq!(a.verify(), Err(FixedStockPriceBetaError::Tampered));
    }
}
#[test]
fn immutable_descriptor_artifact_round_trip_permissions_conflict_and_orphan() {
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    secure_root(temp.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let a = artifact(120);
    let p = write_fixed_stock_price_beta_artifact(temp.path(), &a).unwrap();
    assert_eq!(
        p,
        write_fixed_stock_price_beta_artifact(temp.path(), &a).unwrap()
    );
    assert_eq!(
        read_fixed_stock_price_beta_artifact(temp.path(), &a.content_sha256).unwrap(),
        a
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(p.join("artifact.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::create_dir(temp.path().join(".orphan-stage")).unwrap();
    assert_eq!(
        write_fixed_stock_price_beta_artifact(temp.path(), &a).unwrap(),
        p
    );
    std::fs::write(p.join("artifact.json"), b"{}").unwrap();
    assert!(read_fixed_stock_price_beta_artifact(temp.path(), &a.content_sha256).is_err());
    assert_eq!(
        write_fixed_stock_price_beta_artifact(temp.path(), &a),
        Err(FixedStockPriceBetaError::Conflict)
    );
}
#[cfg(unix)]
#[test]
fn rejects_root_leaf_and_file_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    secure_root(temp.path());
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let root = temp.path().join("root-link");
    symlink(&real, &root).unwrap();
    assert_eq!(
        write_fixed_stock_price_beta_artifact(Path::new(&root), &artifact(120)),
        Err(FixedStockPriceBetaError::UnsafePath)
    );
    let a = artifact(120);
    let leaf_root = tempfile::tempdir().unwrap();
    secure_root(leaf_root.path());
    let leaf = leaf_root.path().join(&a.content_sha256);
    symlink(&real, &leaf).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_artifact(leaf_root.path(), &a.content_sha256),
        Err(FixedStockPriceBetaError::UnsafePath)
    );
    let b = artifact(121);
    let p = write_fixed_stock_price_beta_artifact(temp.path(), &b).unwrap();
    std::fs::remove_file(p.join("artifact.json")).unwrap();
    symlink("/tmp", p.join("artifact.json")).unwrap();
    assert_eq!(
        read_fixed_stock_price_beta_artifact(temp.path(), &b.content_sha256),
        Err(FixedStockPriceBetaError::UnsafePath)
    );
}
