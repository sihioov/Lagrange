use std::path::Path;

use domain::ContentHash;
use market_data::{
    ADJUSTED_BARS_SCHEMA_ID, BARS_SCHEMA_ID, CORPORATE_ACTIONS_SCHEMA_ID, CurateStore,
    CuratedArtifactRef, TOTAL_RETURN_BARS_SCHEMA_ID,
};

/// Build the exact artifact attestation for a test-only curated generation.
///
/// Production readers verify every listed byte and reject unlisted curated
/// files. Fixtures therefore derive their manifest entries from the files they
/// actually wrote instead of weakening that production contract.
pub fn attest_curated_artifacts(store: &CurateStore, version: u32) -> Vec<CuratedArtifactRef> {
    let curated_root = store.root().join("curated");
    let mut artifacts = Vec::new();
    for zone in ["bars", "corporate_actions"] {
        let zone_root = curated_root.join(zone);
        if zone_root.is_dir() {
            collect_artifacts(&curated_root, &zone_root, version, false, &mut artifacts);
        }
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    artifacts
}

fn collect_artifacts(
    curated_root: &Path,
    directory: &Path,
    version: u32,
    inside_version: bool,
    artifacts: &mut Vec<CuratedArtifactRef>,
) {
    let version_component = format!("version={version}");
    for entry in std::fs::read_dir(directory).expect("read curated fixture directory") {
        let entry = entry.expect("read curated fixture entry");
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            collect_artifacts(
                curated_root,
                &path,
                version,
                inside_version || name == version_component,
                artifacts,
            );
            continue;
        }
        if !inside_version {
            continue;
        }
        let schema = match name.as_str() {
            "bars.parquet" => BARS_SCHEMA_ID,
            "adjusted_bars.parquet" => ADJUSTED_BARS_SCHEMA_ID,
            "total_return_bars.parquet" => TOTAL_RETURN_BARS_SCHEMA_ID,
            "corporate_actions.parquet" => CORPORATE_ACTIONS_SCHEMA_ID,
            other => panic!("unrecognized curated fixture artifact: {other}"),
        };
        let bytes = std::fs::read(&path).expect("read curated fixture bytes");
        let relative = path
            .strip_prefix(curated_root)
            .expect("curated fixture artifact stays below curated root")
            .to_str()
            .expect("curated fixture path is UTF-8")
            .to_owned();
        artifacts.push(CuratedArtifactRef {
            path: relative,
            sha256: ContentHash::from_bytes(&bytes),
            size_bytes: u64::try_from(bytes.len()).expect("fixture artifact size fits u64"),
            schema: schema.to_owned(),
        });
    }
}
