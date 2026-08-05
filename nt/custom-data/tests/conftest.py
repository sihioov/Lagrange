"""Shared fixtures for the nt/custom-data test suite (Todo 13).

The `custom-data` package lives in a hyphenated directory and is therefore
imported through ``importlib.import_module`` (the ``import`` statement cannot
carry hyphens; ``resolve_path`` in NautilusTrader uses importlib, so the
``data_cls`` strings ``custom-data.session_events:SessionOpenEvent`` resolve
identically).

The synthetic curated input below reproduces the *documented* Curated bar
schema (crates/market-data/src/curate/schema.rs): prices are
``Decimal128(18, 4)``, timestamps are UTC epoch microseconds, dates are
Date32.  It is the builder's read-only input contract; it is NOT a
re-implementation of curation.
"""
from __future__ import annotations

import importlib
import json
import sys
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

import pyarrow as pa
import pytest

CUSTOM_DATA_DIR = Path(__file__).resolve().parents[1]  # nt/custom-data
REPO_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_BARS = REPO_ROOT / "tests" / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"

# Fix Python module resolution for the hyphenated package.
if str(CUSTOM_DATA_DIR) not in sys.path:
    sys.path.insert(0, str(CUSTOM_DATA_DIR))

SESSION_OPEN_UTC = timezone.utc  # market_open_ts = session date 00:00:00Z
SESSION_CLOSE_UTC = timezone.utc  # market_close_ts = session date 06:30:00Z
SESSION_CLOSE_DELTA = timedelta(hours=6, minutes=30)


def load_events_module():
    return importlib.import_module("custom-data.session_events")


def load_builder_module():
    return importlib.import_module("custom-data.catalog_builder")


@pytest.fixture(scope="session")
def events():
    """The custom-data.session_events module (classes + validation)."""
    return load_events_module()


@pytest.fixture(scope="session")
def builder():
    """The custom-data.catalog_builder module."""
    return load_builder_module()


def _micros(dt: datetime) -> int:
    return int(dt.timestamp() * 1_000_000)


def session_instants(iso_date: str) -> tuple[int, int]:
    """(market_open_ts, market_close_ts) in UTC microseconds for a KRX session.

    KRX is Asia/Seoul fixed UTC+09:00 (no DST): 09:00-15:30 KST ==
    00:00-06:30 UTC.  Instants come from the trading calendar (Todo 9);
    Python timezone/DST machinery is never involved in event construction.
    """
    d = date.fromisoformat(iso_date)
    open_dt = datetime(d.year, d.month, d.day, 0, 0, 0, tzinfo=SESSION_OPEN_UTC)
    close_dt = open_dt + SESSION_CLOSE_DELTA
    return _micros(open_dt), _micros(close_dt)


def golden_bars_rows() -> list[dict]:
    """The golden 2020-01-31 fixture bars (3 seed ETFs x 9 KRX sessions).

    Prices are returned as int64 fixed-point scale-4 (KRW/10^4), matching the
    curated Decimal128(18,4) unscaled values.
    """
    with open(FIXTURE_BARS, encoding="utf-8") as fh:
        doc = json.load(fh)
    rows = []
    for bar in doc["bars"]:
        open_ts, close_ts = session_instants(bar["date"])
        rows.append(
            {
                "instrument_id": bar["instrument"],
                "trading_date": bar["date"],
                "market_open_ts": open_ts,
                "market_close_ts": close_ts,
                "open": int(bar["open"]) * 10_000,
                "high": int(bar["high"]) * 10_000,
                "low": int(bar["low"]) * 10_000,
                "close": int(bar["close"]) * 10_000,
                "volume": bar["volume"],
                "trading_value": bar["value"],
                "currency": doc["currency"],
                "source": "synthetic",
                "ingested_at": _micros(datetime(2020, 2, 10, tzinfo=timezone.utc)),
                "batch_id": "00000000-0000-0000-0000-000000000000",
                "raw_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            }
        )
    return rows


def bars_table(rows: list[dict]) -> pa.Table:
    """A pyarrow table with the documented Curated bars schema."""
    schema = pa.schema(
        [
            pa.field("instrument_id", pa.string(), nullable=False),
            pa.field("trading_date", pa.date32(), nullable=False),
            pa.field("market_open_ts", pa.timestamp("us"), nullable=False),
            pa.field("market_close_ts", pa.timestamp("us"), nullable=False),
            pa.field("open", pa.decimal128(18, 4), nullable=False),
            pa.field("high", pa.decimal128(18, 4), nullable=False),
            pa.field("low", pa.decimal128(18, 4), nullable=False),
            pa.field("close", pa.decimal128(18, 4), nullable=False),
            pa.field("volume", pa.int64(), nullable=False),
            pa.field("trading_value", pa.int64(), nullable=True),
            pa.field("currency", pa.string(), nullable=False),
            pa.field("source", pa.string(), nullable=False),
            pa.field("ingested_at", pa.timestamp("us"), nullable=False),
            pa.field("batch_id", pa.string(), nullable=False),
            pa.field("raw_hash", pa.string(), nullable=False),
        ]
    )
    return pa.Table.from_pylist(rows, schema=schema)


def adjusted_rows(bars: list[dict]) -> list[dict]:
    """Adjusted rows: identical bars with cumulative factor 1.0 (scale-8)."""
    out = []
    for row in bars:
        out.append({**row, "adjustment_kind": "NONE", "adjustment_factor": 100_000_000,
                    "adjustment_events": "[]"})
    return out


def adjusted_table(rows: list[dict]) -> pa.Table:
    schema = pa.schema(
        [
            pa.field("instrument_id", pa.string(), nullable=False),
            pa.field("trading_date", pa.date32(), nullable=False),
            pa.field("market_open_ts", pa.timestamp("us"), nullable=False),
            pa.field("market_close_ts", pa.timestamp("us"), nullable=False),
            pa.field("open", pa.decimal128(18, 4), nullable=False),
            pa.field("high", pa.decimal128(18, 4), nullable=False),
            pa.field("low", pa.decimal128(18, 4), nullable=False),
            pa.field("close", pa.decimal128(18, 4), nullable=False),
            pa.field("volume", pa.int64(), nullable=False),
            pa.field("trading_value", pa.int64(), nullable=True),
            pa.field("adjustment_kind", pa.string(), nullable=False),
            pa.field("adjustment_factor", pa.decimal128(18, 8), nullable=False),
            pa.field("adjustment_events", pa.string(), nullable=False),
            pa.field("currency", pa.string(), nullable=False),
            pa.field("source", pa.string(), nullable=False),
            pa.field("ingested_at", pa.timestamp("us"), nullable=False),
            pa.field("batch_id", pa.string(), nullable=False),
            pa.field("raw_hash", pa.string(), nullable=False),
        ]
    )
    return pa.Table.from_pylist(rows, schema=schema)


def write_curated_fixture(root: Path, bars: list[dict] | None = None,
                          version: int = 1) -> dict[str, list[str]]:
    """Writes a deterministic synthetic curated zone under `root/data/curated`.

    Layout matches crates/market-data CurateStore:
      data/curated/bars/market=kr/symbol={iid}/year={yyyy}/version={v}/bars.parquet
      .../adjusted_bars.parquet
    Returns {instrument_id: [written bars.parquet paths]}.
    """
    import pyarrow.parquet as pq

    bars = bars if bars is not None else golden_bars_rows()
    table = bars_table(bars)
    adj_table = adjusted_table(adjusted_rows(bars))
    written: dict[str, list[str]] = {}
    seen = set()
    for i, row in enumerate(bars):
        iid = row["instrument_id"]
        year = row["trading_date"][:4]
        if (iid, year) in seen:
            continue
        seen.add((iid, year))
        row_mask = [r["instrument_id"] == iid and r["trading_date"][:4] == year for r in bars]
        sub = table.filter(pa.array(row_mask))
        adj_sub = adj_table.filter(pa.array(row_mask))
        part = (
            root
            / "data"
            / "curated"
            / "bars"
            / "market=kr"
            / f"symbol={iid}"
            / f"year={year}"
            / f"version={version}"
        )
        part.mkdir(parents=True, exist_ok=True)
        bars_path = part / "bars.parquet"
        adj_path = part / "adjusted_bars.parquet"
        pq.write_table(sub, bars_path)
        pq.write_table(adj_sub, adj_path)
        written.setdefault(iid, []).append(str(bars_path))
    return written


@pytest.fixture(scope="session")
def curated_root(tmp_path_factory):
    """A synthetic curated zone with the golden 3-instrument fixture."""
    root = tmp_path_factory.mktemp("curated-root")
    write_curated_fixture(root)
    return root


def equity_from_dict(iid: str, currency: str = "KRW", lot_size: int = 100) -> dict:
    """The Equity definition dict the catalog builder writes for an instrument."""
    return {
        "id": iid,
        "raw_symbol": iid.split(".")[0],
        "venue": "KRX",
        "currency": currency,
        "price_precision": 4,
        "price_increment": "0.0001",
        "size_precision": 0,
        "lot_size": str(lot_size),
        "is_quoted": False,
        "info": None,
        "ts_event": 1577836800000000000,
        "ts_init": 1577836800000000000,
    }
