"""Dependency-light Phase 0 curated Parquet materialization."""
from __future__ import annotations

from datetime import date
from decimal import Decimal
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


def _bars_table(rows: list[dict]) -> pa.Table:
    schema = pa.schema([
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
    ])
    rows = [{**r, "trading_date": date.fromisoformat(r["trading_date"])} for r in rows]
    return pa.Table.from_pylist(rows, schema=schema)


def _adjusted_table(rows: list[dict]) -> pa.Table:
    schema = pa.schema([
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
    ])
    rows = [{**r, "trading_date": date.fromisoformat(r["trading_date"])} for r in rows]
    return pa.Table.from_pylist(rows, schema=schema)


def materialize_curated_zone(rows: list[dict], curated_root: Path, *, version: int) -> None:
    """Write deterministic curated Parquet partitions in the Phase 0 layout."""
    bars_table = _bars_table(rows)
    adj_table = _adjusted_table(
        [{**r, "adjustment_kind": "split", "adjustment_factor": Decimal("1.00000000"),
          "adjustment_events": "[]"} for r in rows]
    )
    seen: set[tuple[str, str]] = set()
    for row in rows:
        iid = row["instrument_id"]
        year = row["trading_date"][:4]
        if (iid, year) in seen:
            continue
        seen.add((iid, year))
        mask = pa.array(
            [r["instrument_id"] == iid and r["trading_date"][:4] == year for r in rows]
        )
        part = (
            curated_root
            / "curated"
            / "bars"
            / "market=kr"
            / f"symbol={iid}"
            / f"year={year}"
            / f"version={version}"
        )
        part.mkdir(parents=True, exist_ok=True)
        pq.write_table(bars_table.filter(mask), part / "bars.parquet")
        pq.write_table(adj_table.filter(mask), part / "adjusted_bars.parquet")
