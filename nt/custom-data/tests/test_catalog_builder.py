"""Todo 13 red tests: the read-only NT catalog builder.

data/nautilus_catalog is built deterministically from the Curated zone
(data/curated/bars/...); a rebuild produces identical content hashes.  The
builder validates ordering (equal/out-of-order timestamps and duplicate
session events hard-fail) and an unregistered class hard-fails before
simulation.
"""
import json

import pytest

from curated_helpers import equity_from_dict, golden_bars_rows, session_instants, write_curated_fixture


def test_build_layout_and_manifest(builder, curated_root, tmp_path):
    catalog = tmp_path / "nautilus_catalog"
    result = builder.build_catalog(curated_root, catalog)
    assert catalog.is_dir()
    manifest = json.loads((catalog / "manifest.json").read_text(encoding="utf-8"))
    assert manifest["schema_version"] == 1
    assert set(manifest["classes"]) == {"SessionOpenEvent", "DailyBarClosedEvent"}
    assert sorted(manifest["instruments"]) == ["069500.KRX", "114260.KRX", "229200.KRX"]
    assert manifest["content_hash"].startswith("sha256:")
    # Per-instrument parquet partitions under data/custom_session_open_event/...
    assert (catalog / "data" / "custom_session_open_event" / "069500.KRX").is_dir()
    assert (catalog / "data" / "custom_daily_bar_closed_event" / "069500.KRX").is_dir()
    assert result["event_count"] == 3 * 18  # 3 instruments x 9 sessions x 2 events


def test_query_roundtrip(builder, curated_root, tmp_path, events):
    from nautilus_trader.persistence.catalog import ParquetDataCatalog

    catalog = tmp_path / "nautilus_catalog"
    builder.build_catalog(curated_root, catalog)
    cat = ParquetDataCatalog(path=str(catalog))
    rows = cat.query(data_cls=events.SessionOpenEvent, identifiers=["069500.KRX"])
    assert len(rows) == 9
    first = getattr(rows[0], "data", rows[0])
    assert first.trading_date == "2020-01-20"
    assert first.open_price == 101500000
    assert first.currency == "KRW"
    assert first.data_version == "1"
    closes = cat.query(data_cls=events.DailyBarClosedEvent, identifiers=["069500.KRX"])
    assert len(closes) == 9
    last_close = getattr(closes[-1], "data", closes[-1])
    assert last_close.trading_date == "2020-02-03"
    assert last_close.close == 103800000
    assert last_close.adjustment_factor == 100000000
    instruments = cat.instruments(instrument_ids=["069500.KRX"])
    assert [i.id.value for i in instruments] == ["069500.KRX"]


def test_rebuild_produces_identical_hashes(builder, curated_root, tmp_path):
    catalog1 = tmp_path / "catalog-a"
    catalog2 = tmp_path / "catalog-b"
    r1 = builder.build_catalog(curated_root, catalog1)
    r2 = builder.build_catalog(curated_root, catalog2)
    assert r1["content_hash"] == r2["content_hash"]
    assert r1["event_count"] == r2["event_count"]
    # Byte-identical parquet files across rebuilds.
    files1 = sorted(p.relative_to(catalog1).as_posix() for p in catalog1.rglob("*.parquet"))
    files2 = sorted(p.relative_to(catalog2).as_posix() for p in catalog2.rglob("*.parquet"))
    assert files1 == files2
    for rel in files1:
        assert (catalog1 / rel).read_bytes() == (catalog2 / rel).read_bytes(), rel
    assert (catalog1 / "manifest.json").read_bytes() == (catalog2 / "manifest.json").read_bytes()


def test_builder_accepts_only_documented_schema(builder, tmp_path):
    """A curated input with an unknown column is rejected (no silent success)."""
    import pyarrow as pa

    from curated_helpers import bars_table

    root = tmp_path / "bad-curated"
    rows = golden_bars_rows()[:1]
    table = bars_table(rows).drop_columns(["raw_hash"])
    part = root / "curated" / "bars" / "market=kr" / "symbol=069500.KRX" / "year=2020" / "version=1"
    part.mkdir(parents=True)
    import pyarrow.parquet as pq

    pq.write_table(table, part / "bars.parquet")
    with pytest.raises(Exception, match="raw_hash|schema"):
        builder.build_catalog(root, tmp_path / "out")


def test_unregistered_class_hard_fails_before_simulation(builder, curated_root, tmp_path):
    """A custom class not registered with the Rust backend fails before the
    simulation starts (never silent, never delivered)."""
    import nautilus_trader.core.data as cd
    from nautilus_trader.model.custom import customdataclass_pyo3
    from nautilus_trader.model.identifiers import InstrumentId

    @customdataclass_pyo3()
    class UnregisteredProbeEvent(cd.Data):
        instrument_id: InstrumentId
        value: int

    catalog = tmp_path / "nautilus_catalog"
    builder.build_catalog(curated_root, catalog)
    from nautilus_trader.persistence.catalog import ParquetDataCatalog

    cat = ParquetDataCatalog(path=str(catalog))
    probe = UnregisteredProbeEvent(
        instrument_id=InstrumentId.from_str("069500.KRX"), value=1,
        ts_event=1577836800000000000, ts_init=1577836800000000000,
    )
    cat.write_data([probe])

    from nautilus_trader.backtest.config import (
        BacktestDataConfig, BacktestEngineConfig, BacktestRunConfig, BacktestVenueConfig,
    )
    from nautilus_trader.backtest.node import BacktestNode
    from nautilus_trader.trading.config import ImportableStrategyConfig

    config = BacktestRunConfig(
        venues=[BacktestVenueConfig(name="KRX", oms_type="HEDGING", account_type="CASH",
                                    starting_balances=["100000000 KRW"])],
        engine=BacktestEngineConfig(strategies=[
            ImportableStrategyConfig(strategy_path="test_replay:ReplayStrategy",
                                     config_path="test_replay:ReplayStrategyConfig",
                                     config={"instrument_id": "069500.KRX"})]),
        data=[BacktestDataConfig(catalog_path=str(catalog), data_cls=UnregisteredProbeEvent,
                                 instrument_id=InstrumentId.from_str("069500.KRX"),
                                 client_id="CUSTOM")],
        # The Rust backend path (streaming) rejects unknown custom types; the
        # pyarrow path would happily read them back, so the hard failure is
        # exercised through the DataBackendSession/custom-file registration.
        chunk_size=1000,
        raise_exception=True,
    )
    node = BacktestNode(configs=[config])
    with pytest.raises(Exception) as excinfo:
        node.run()
    # Hard failure BEFORE any strategy event is delivered.
    assert "UnregisteredProbeEvent" in str(excinfo.value) or "not registered" in str(excinfo.value).lower()


def test_duplicate_session_input_rejected_by_builder(builder, tmp_path, events):
    """A curated input emitting two opens for one session is rejected."""
    rows = [r for r in golden_bars_rows() if r["instrument_id"] == "069500.KRX"][:2]
    dup = dict(rows[0], trading_date="2020-01-21", market_open_ts=session_instants("2020-01-21")[0],
               market_close_ts=session_instants("2020-01-21")[1])
    with pytest.raises(events.DuplicateSessionError):
        builder.build_events_from_curated([*rows, dup])
