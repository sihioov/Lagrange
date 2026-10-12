use domain::{TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KIS_NORMALIZED};
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::provider::{KrxProvider, RecordedBundle};
use market_data::publication::PublicationBundle;
use market_data::storage::RawStore;

pub struct FixtureBundle {
    pub bundle: PublicationBundle,
    _root: tempfile::TempDir,
}

pub fn synthetic_bundle(retrieved_at: &str) -> FixtureBundle {
    let root = tempfile::tempdir().expect("raw fixture root");
    let store = RawStore::new(root.path());
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/kr-etf/contract"
        ))
        .expect("recorded bundle"),
    );
    let outcome = ingest_bundle(
        &store,
        &provider,
        &IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse("2020-01-31").expect("date"),
            UtcTimestamp::parse_rfc3339(retrieved_at).expect("retrieved_at"),
        ),
        None,
    )
    .expect("persist synthetic fixture");
    let manifest = store
        .read_manifest("krx", "kr")
        .expect("read manifest")
        .into_iter()
        .find(|entry| entry.batch_id == outcome.batch_id)
        .expect("persisted manifest");
    let bundle = PublicationBundle::from_raw(&store, &manifest).expect("verified publication");
    FixtureBundle {
        bundle,
        _root: root,
    }
}

pub fn credentialed_normalized_bundle(retrieved_at: &str) -> FixtureBundle {
    let mut fixture = synthetic_bundle(retrieved_at);
    fixture.bundle.provider = PROVIDER_KIS_NORMALIZED.to_owned();
    fixture.bundle.fetch_mode = FetchMode::Credentialed;
    for file in &mut fixture.bundle.files {
        file.file_name = match file.kind {
            market_data::publication::DataBatchKind::Eod
            | market_data::publication::DataBatchKind::EodUnavailable => "bars.json",
            market_data::publication::DataBatchKind::Reference => "reference.json",
            market_data::publication::DataBatchKind::Calendar => "calendar.json",
            market_data::publication::DataBatchKind::CorporateActions => "corporate-actions.json",
            _ => file.file_name.as_str(),
        }
        .to_owned();
    }
    fixture
}
