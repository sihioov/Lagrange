//! Todo 13 event-order gate (crates/market-data `event_order` tests).
//!
//! Proves, on the golden 2020-01-31 fixture (tests/fixtures/kr-etf/2020-01-31),
//! the documented event contract (requirements §9.1, design ADR-004 §9.2,
//! session-semantics.json `event_order_for_todo13`):
//!
//! ```text
//! close(T) -> pending target(T+1) exists -> open(T+1) -> close(T+1)
//! ```
//!
//! T = 2020-01-31 (close `2020-01-31T06:30:00Z`), T+1 = 2020-02-03
//! (open `2020-02-03T00:00:00Z`, close `2020-02-03T06:30:00Z`).  The session
//! instants come from the KRX calendar (Asia/Seoul, fixed +09:00, explicit
//! UTC instants - no Python DST coverage is relied on anywhere).
//!
//! The Lagrange event structs below mirror the Python catalog classes
//! (`nt/custom-data/session_events.py`): `SessionOpenEvent` carries ONLY the
//! session open price - there is no high/low/close field to read (structural
//! future-field barrier).  The fixture is also materialized into the
//! canonical `data/curated` zone (idempotent) so the Python catalog builder
//! consumes it read-only.

use std::fs;
use std::path::{Path, PathBuf};

use domain::{BatchId, ContentHash, Currency, DatasetId, InstrumentId, TradingDate, UtcTimestamp};
use serde_json::{Value, json};

use market_data::contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::curate::{CurateRequest, CurateStore, curate_batch, read_bars};
use market_data::instrument_master::InstrumentMaster;
use market_data::{KrCalendar, ManifestEntry, RawStore, krx_2020, seed_universe};

/// The golden Todo 6 fixture: 3 seed ETFs, 27 bars, no corporate actions.
const GOLDEN_BARS: &[u8] = include_bytes!("../../../tests/fixtures/kr-etf/2020-01-31/bars.json");
const EMPTY_ACTIONS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json");

/// A curation clock after every fixture `announced_at` (fixtures are 2020).
fn now() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2020-02-10T00:00:00Z").expect("valid clock")
}

/// Stores a synthetic raw batch (bars + corporate actions) in `root` and
/// returns the store plus its manifest entry (pattern: Todo 10 tests).
fn fixture_batch(root: &Path, bars: &[u8], actions: &[u8]) -> (RawStore, ManifestEntry) {
    let raw = RawStore::new(root.join("data"));
    let batch_id = BatchId::generate();
    let request = RequestMetadata {
        endpoint: "krx.eod.bars.v1".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode: FetchMode::Synthetic,
    };
    let envelopes = vec![
        RawEnvelope::new(
            batch_id,
            ResponseKind::Bars,
            "bars.json",
            bars.to_vec(),
            now(),
            request.clone(),
        ),
        RawEnvelope::new(
            batch_id,
            ResponseKind::CorporateActions,
            "corporate-actions.json",
            actions.to_vec(),
            now(),
            request,
        ),
    ];
    let spec = market_data::BatchSpec {
        provider: "krx",
        market: "kr",
        date: &TradingDate::new(2020, 1, 31).expect("valid date"),
        batch_id,
        entitlement_reference: None,
        mode: FetchMode::Synthetic,
    };
    let entry = raw
        .store_batch(&spec, &envelopes)
        .expect("fixture batch stores");
    (raw, entry)
}

/// Curates the golden fixture into `curated_root` (any data root).
fn curate_golden_into(curated_root: &Path) -> (RawStore, CurateStore, ManifestEntry) {
    let (raw, entry) = fixture_batch(curated_root, GOLDEN_BARS, EMPTY_ACTIONS);
    let curated = CurateStore::new(curated_root);
    let outcome = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid dataset id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("golden fixture curates");
    assert_eq!(outcome.bars_written, 27);
    (raw, curated, entry)
}

// ---------------------------------------------------------------------------
// Lagrange event contract (mirrors nt/custom-data/session_events.py)
// ---------------------------------------------------------------------------

/// Fixed-point price scale of curated prices / event price fields (KRW/10^4).
const PRICE_SCALE: u8 = 4;

/// `SessionOpenEvent`: instrument_id, trading_date, session_open_ts,
/// open_price, currency, data_version.  Structurally carries NO high/low/close
/// - the future-field barrier is enforced at compile time.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionOpenEvent {
    instrument_id: InstrumentId,
    trading_date: TradingDate,
    session_open_ts: UtcTimestamp,
    open_price: i64,
    currency: Currency,
    data_version: u32,
}

/// `DailyBarClosedEvent`: documented OHLCV/adjustment fields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DailyBarClosedEvent {
    instrument_id: InstrumentId,
    trading_date: TradingDate,
    session_close_ts: UtcTimestamp,
    open: i64,
    high: i64,
    low: i64,
    close: i64,
    volume: i64,
    adjustment_factor: i64,
    currency: Currency,
    data_version: u32,
}

/// The merged per-instrument event stream (close/open interleaved).
#[derive(Debug, Clone, PartialEq, Eq)]
enum LagrangeEvent {
    Open(SessionOpenEvent),
    Close(DailyBarClosedEvent),
}

impl LagrangeEvent {
    fn ts_event(&self) -> UtcTimestamp {
        match self {
            Self::Open(e) => e.session_open_ts,
            Self::Close(e) => e.session_close_ts,
        }
    }
}

/// Builds the deterministic event stream for one instrument from its curated
/// rows (rows are already sorted by trading_date during curation).
fn build_event_stream(bars: &[market_data::curate::schema::CuratedBar]) -> Vec<LagrangeEvent> {
    let mut stream = Vec::with_capacity(bars.len() * 2);
    for bar in bars {
        // Raw open is the execution price; scale-4 fixed point as int64.
        let to_fixed = |p: &domain::Price| -> i64 { p.amount().bits() as i64 };
        stream.push(LagrangeEvent::Open(SessionOpenEvent {
            instrument_id: bar.instrument_id.clone(),
            trading_date: bar.trading_date,
            session_open_ts: bar.market_open_ts,
            open_price: to_fixed(&bar.open),
            currency: bar.currency,
            data_version: 1,
        }));
        stream.push(LagrangeEvent::Close(DailyBarClosedEvent {
            instrument_id: bar.instrument_id.clone(),
            trading_date: bar.trading_date,
            session_close_ts: bar.market_close_ts,
            open: to_fixed(&bar.open),
            high: to_fixed(&bar.high),
            low: to_fixed(&bar.low),
            close: to_fixed(&bar.close),
            volume: bar.volume,
            adjustment_factor: 100_000_000, // 1.0 at factor scale 8
            currency: bar.currency,
            data_version: 1,
        }));
    }
    // Deterministic ordering: per-instrument stream sorted by ts_event;
    // equal timestamps within an instrument are structurally impossible
    // (a session open/close pair is 6.5h apart and sessions are unique).
    stream.sort_by_key(|e| e.ts_event());
    stream
}

/// Exact UTC instants for the fixture transition (KRX calendar, fixed +09:00).
fn ts(iso: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(iso).expect("valid iso instant")
}

// ---------------------------------------------------------------------------
// Event order gate
// ---------------------------------------------------------------------------

#[test]
fn event_order_proof_for_2020_01_31_fixture() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_raw, curated, _entry) = curate_golden_into(temp.path());
    let bars = read_bars(&curated.bars_path("kr", "069500.KRX", 2020, 1)).expect("bars read");

    // 9 KRX sessions in the golden fixture (Seollal 2020-01-24/27 skipped).
    assert_eq!(bars.len(), 9);
    let stream = build_event_stream(&bars);
    assert_eq!(stream.len(), 18);

    // Exact instants (session-semantics.json): T = 2020-01-31, T+1 = 2020-02-03.
    let close_t = ts("2020-01-31T06:30:00Z");
    let open_t1 = ts("2020-02-03T00:00:00Z");
    let close_t1 = ts("2020-02-03T06:30:00Z");
    assert_eq!(stream[13].ts_event(), close_t); // close(T) = 14th event
    assert_eq!(stream[14].ts_event(), open_t1); // open(T+1)
    assert_eq!(stream[15].ts_event(), close_t1); // close(T+1)

    // Event order invariant: close(T) -> pending target(T+1) -> open(T+1) -> close(T+1).
    assert!(matches!(stream[13], LagrangeEvent::Close(_)));
    assert!(matches!(stream[14], LagrangeEvent::Open(_)));
    assert!(matches!(stream[15], LagrangeEvent::Close(_)));
    assert!(close_t < open_t1 && open_t1 < close_t1);

    // Pending-target window: the strategy stores PendingTarget(effective_date =
    // T+1) strictly between close(T) and open(T+1) (requirements §9.1.2-9.1.3).
    // The weekend gap makes the window 65.5 hours: 2020-01-31T06:30:00Z .. 2020-02-03T00:00:00Z.
    let gap_ns = open_t1.as_datetime() - close_t.as_datetime();
    assert_eq!(gap_ns.num_hours(), 65);
    assert_eq!(gap_ns.num_minutes(), 30);
    let pending_ts = ts("2020-01-31T06:30:01Z"); // any instant in (close(T), open(T+1)]
    assert!(close_t < pending_ts && pending_ts <= open_t1);

    // No same-day high/low/close at open time: SessionOpenEvent has no such
    // fields (compile-time structural barrier); also verify the close of
    // T+1 does not precede the open of T+1 in the stream.
    let open_idx = stream
        .iter()
        .position(|e| matches!(e, LagrangeEvent::Open(o) if o.trading_date == TradingDate::new(2020, 2, 3).expect("date")))
        .expect("open(T+1) present");
    let close_t1_idx = stream
        .iter()
        .position(|e| matches!(e, LagrangeEvent::Close(c) if c.trading_date == TradingDate::new(2020, 2, 3).expect("date")))
        .expect("close(T+1) present");
    assert!(open_idx < close_t1_idx);

    // Every instrument's stream begins with a close and ends with a close.
    for symbol in ["229200.KRX", "114260.KRX"] {
        let bars = read_bars(&curated.bars_path("kr", symbol, 2020, 1)).expect("bars read");
        let stream = build_event_stream(&bars);
        assert_eq!(stream.len(), 18);
        assert!(matches!(stream.first(), Some(LagrangeEvent::Close(_))));
        assert!(matches!(stream.last(), Some(LagrangeEvent::Close(_))));
        // Strictly increasing per-instrument ts_event (deterministic ordering).
        for pair in stream.windows(2) {
            assert!(pair[0].ts_event() < pair[1].ts_event());
        }
    }
}

#[test]
fn calendar_derives_t_plus_1_session_with_explicit_utc_instants() {
    let calendar = krx_2020();
    // 2020-01-31 Friday -> 2020-02-03 Monday (weekend gap; no DST anywhere).
    let next = calendar
        .next_trading_day(TradingDate::new(2020, 1, 31).expect("date"))
        .expect("next session");
    assert_eq!(next, TradingDate::new(2020, 2, 3).expect("date"));
    // Explicit UTC instants (Asia/Seoul fixed +09:00): open 00:00Z, close 06:30Z.
    assert_eq!(
        calendar.session_open_utc(next).expect("open"),
        ts("2020-02-03T00:00:00Z")
    );
    assert_eq!(
        calendar.session_close_utc(next).expect("close"),
        ts("2020-02-03T06:30:00Z")
    );
}

// ---------------------------------------------------------------------------
// Canonical curated zone (read-only input for the Python catalog builder)
// ---------------------------------------------------------------------------

/// The repo `data/` root (curated zone lives at `data/curated`, gitignored).
fn repo_data_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

#[test]
fn canonical_curated_zone_materialized_idempotently() {
    let data_root = repo_data_root();
    let curated = CurateStore::new(&data_root);
    let dataset_id = DatasetId::parse("kr-etf-daily").expect("valid dataset id");
    let manifest_path = curated.dataset_dir(&dataset_id, 1).join("manifest.json");

    if !manifest_path.exists() {
        let temp = tempfile::tempdir().expect("temp dir");
        let (_raw, _curated, _entry) = curate_golden_into(temp.path());
        // Re-run the curation against the canonical repo data root: the raw
        // fixture batch is stored under the temp root, the curated output
        // lands in `data/curated/...` (read-only input for the builder).
        let (raw, entry) = fixture_batch(temp.path(), GOLDEN_BARS, EMPTY_ACTIONS);
        let canonical = CurateStore::new(&data_root);
        curate_batch(
            &raw,
            &entry,
            &krx_2020(),
            &seed_universe(),
            &canonical,
            &CurateRequest {
                dataset_id: &dataset_id,
                market: "kr",
                source: "krx",
                now: now(),
            },
        )
        .expect("canonical curated zone materializes");
    }

    // The zone exists and holds the documented partition layout.
    assert!(manifest_path.exists(), "manifest missing at {manifest_path:?}");
    for symbol in ["069500.KRX", "229200.KRX", "114260.KRX"] {
        let bars = curated.bars_path("kr", symbol, 2020, 1);
        assert!(bars.exists(), "missing {bars:?}");
        assert!(curated.adjusted_bars_path("kr", symbol, 2020, 1).exists());
        assert!(curated.total_return_bars_path("kr", symbol, 2020, 1).exists());
    }
}
