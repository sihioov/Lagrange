"""Read-only NT catalog builder (plan Todo 13).

Builds `data/nautilus_catalog` deterministically from the Curated zone
(`data/curated/bars/...` raw + adjusted + TR parquet, Todo 10 layout).
The curated zone is READ-ONLY input: the builder never writes back into it.

Contract:

- One `SessionOpenEvent` per (instrument, session) at `market_open_ts` and one
  `DailyBarClosedEvent` at `market_close_ts`; logical Curated decimals are
  converted exactly to int64 fixed-point prices at scale 4 and adjustment
  factors at scale 8 (no floats, no rounding).
- Deterministic ordering is enforced by `validate_event_stream`: equal or
  out-of-order timestamps and duplicate session opens are rejected with typed
  errors (never silent).
- Instruments are registered as `Equity` definitions (fixed seed-universe
  contract: venue KRX, lot 100, price precision 4) so `BacktestDataConfig`
  with `instrument_id` resolves them; custom-data configs never
  auto-load instruments in NT.
- A rebuild produces an identical `content_hash` (sha256 over the canonical
  serialization of every event and instrument, sorted) and byte-identical
  parquet files.
- The manifest lists the registered classes; an unregistered class fails
  hard before simulation (Rust backend rejects unknown custom types).
"""
from __future__ import annotations

import hashlib
import json
import shutil
from pathlib import Path
from typing import Any, Iterable

import pyarrow as pa
import pyarrow.parquet as pq
from nautilus_trader.model.identifiers import InstrumentId

from .session_events import (
    DailyBarClosedEvent,
    SessionOpenEvent,
    sort_event_stream,
    validate_event_stream,
)

__all__ = [
    "CURATED_BARS_COLUMNS",
    "CURATED_ADJUSTED_COLUMNS",
    "EQUITY_VENUE",
    "EQUITY_LOT_SIZE",
    "EQUITY_PRICE_PRECISION",
    "CatalogBuilderError",
    "build_events_from_curated",
    "PENDING_TARGET_WINDOW_PROOF",
    "build_catalog",
    "catalog_content_hash",
]

#: Documented Curated bars columns (crates/market-data/src/curate/schema.rs).
#: The builder reads a subset but validates the FULL documented contract so a
#: structurally deviant input never passes silently.
CURATED_BARS_COLUMNS = (
    "instrument_id",
    "trading_date",
    "market_open_ts",
    "market_close_ts",
    "open",
    "high",
    "low",
    "close",
    "volume",
    "trading_value",
    "currency",
    "source",
    "ingested_at",
    "batch_id",
    "raw_hash",
)
#: Additional adjusted-bars columns (adjustment contract for signals).
CURATED_ADJUSTED_COLUMNS = ("adjustment_factor",)

#: Fixed seed-universe contract for Equity definitions (fixture-encoded in
#: Todo 6, universe-level in Todo 12: 100-lot KRW ETFs on venue KRX).
EQUITY_VENUE = "KRX"
EQUITY_LOT_SIZE = 100
EQUITY_PRICE_PRECISION = 4

_MICROS_TO_NANOS = 1_000


class CatalogBuilderError(Exception):
    """A typed catalog-build failure (bad input, missing columns, ...)."""


def _require_columns(table: pa.Table, columns: Iterable[str], path: Path) -> None:
    names = set(table.column_names)
    for column in columns:
        if column not in names:
            raise CatalogBuilderError(
                f"curated input {path} is missing column {column!r} "
                f"(documented schema violated)"
            )


def _fixed_to_int(table: pa.Table, column: str, scale: int) -> list[int]:
    """Logical Decimal column -> exact fixed-point raw integer values."""
    expected_type = pa.decimal128(18, scale)
    actual_type = table.schema.field(column).type
    if actual_type != expected_type:
        raise CatalogBuilderError(
            f"curated column {column!r} must have type {expected_type}, got {actual_type}"
        )
    values: list[int] = []
    factor = 10 ** scale
    for value in table.column(column).to_pylist():
        if value is None or not value.is_finite():
            raise CatalogBuilderError(f"curated column {column!r} must contain finite decimals")
        scaled = value * factor
        if scaled != scaled.to_integral_value():
            raise CatalogBuilderError(
                f"curated column {column!r} value {value} is not representable at scale {scale}"
            )
        values.append(int(scaled))
    return values


def _micros_to_nanos(table: pa.Table, column: str) -> list[int]:
    return [int(v) * _MICROS_TO_NANOS for v in table.column(column).cast(pa.int64()).to_pylist()]


def build_events_from_curated(bars_rows: Iterable[dict[str, Any]]) -> list[Any]:
    """Builds the validated, deterministically-sorted event stream.

    `bars_rows` are the Curated bars rows (documented schema) as read from
    `bars.parquet` plus `adjustment_factor` from `adjusted_bars.parquet`.
    """
    events: list[Any] = []
    for row in bars_rows:
        instrument_id = InstrumentId.from_str(str(row["instrument_id"]))
        open_ts = int(row["market_open_ts"]) * _MICROS_TO_NANOS
        close_ts = int(row["market_close_ts"]) * _MICROS_TO_NANOS
        version = str(row.get("data_version", "1"))
        events.append(
            SessionOpenEvent(
                instrument_id=instrument_id,
                trading_date=str(row["trading_date"]),
                session_open_ts=open_ts,
                open_price=int(row["open"]),
                currency=str(row["currency"]),
                data_version=version,
                ts_event=open_ts,
                ts_init=open_ts,
            )
        )
        events.append(
            DailyBarClosedEvent(
                instrument_id=instrument_id,
                trading_date=str(row["trading_date"]),
                session_close_ts=close_ts,
                open=int(row["open"]),
                high=int(row["high"]),
                low=int(row["low"]),
                close=int(row["close"]),
                volume=int(row["volume"]),
                adjustment_factor=int(row.get("adjustment_factor", 100_000_000)),
                currency=str(row["currency"]),
                data_version=version,
                ts_event=close_ts,
                ts_init=close_ts,
            )
        )
    return validate_event_stream(events)


def PENDING_TARGET_WINDOW_PROOF(close_ts: int, open_ts: int) -> bool:
    """True when a PendingTarget(effective_date=T+1) can be recorded strictly
    between close(T) and open(T+1) (requirements §9.1.2-9.1.3)."""
    return close_ts < open_ts


def _read_curated_rows(curated_root: Path) -> list[dict[str, Any]]:
    """Reads every bars partition (read-only) plus its adjustment factors.

    Partition layout: data/curated/bars/market={m}/symbol={s}/year={y}/version={v}/.
    """
    bars_dir = curated_root / "curated" / "bars"
    if not bars_dir.is_dir():
        raise CatalogBuilderError(f"curated bars zone missing: {bars_dir}")
    bars_paths = sorted(bars_dir.rglob("bars.parquet"))
    versions = sorted({path.parent.name for path in bars_paths})
    if len(versions) > 1:
        raise CatalogBuilderError(f"mixed curated versions: {', '.join(versions)}")
    rows: list[dict[str, Any]] = []
    for bars_path in bars_paths:
        version = bars_path.parent.name.removeprefix("version=")
        table = pq.read_table(bars_path)
        _require_columns(table, CURATED_BARS_COLUMNS, bars_path)
        adj_path = bars_path.parent / "adjusted_bars.parquet"
        factors: list[int] | None = None
        if adj_path.exists():
            adj = pq.read_table(adj_path)
            _require_columns(adj, CURATED_ADJUSTED_COLUMNS, adj_path)
            factors = _fixed_to_int(adj, "adjustment_factor", 8)
        instrument_ids = table.column("instrument_id").to_pylist()
        trading_dates = table.column("trading_date").cast(pa.string()).to_pylist()
        opens_ts = [int(v) for v in table.column("market_open_ts").cast(pa.int64()).to_pylist()]
        closes_ts = [int(v) for v in table.column("market_close_ts").cast(pa.int64()).to_pylist()]
        opens = _fixed_to_int(table, "open", 4)
        highs = _fixed_to_int(table, "high", 4)
        lows = _fixed_to_int(table, "low", 4)
        closes = _fixed_to_int(table, "close", 4)
        volumes = [int(v) for v in table.column("volume").to_pylist()]
        currencies = table.column("currency").to_pylist()
        for i, _ in enumerate(instrument_ids):
            rows.append(
                {
                    "instrument_id": instrument_ids[i],
                    "trading_date": trading_dates[i],
                    "market_open_ts": opens_ts[i],
                    "market_close_ts": closes_ts[i],
                    "open": opens[i],
                    "high": highs[i],
                    "low": lows[i],
                    "close": closes[i],
                    "volume": volumes[i],
                    "currency": currencies[i],
                    "data_version": version,
                    "adjustment_factor": factors[i] if factors is not None else 100_000_000,
                }
            )
    if not rows:
        raise CatalogBuilderError(f"no curated bars found under {bars_dir}")
    return rows


def _equity_dict(events: list[Any], instrument: str) -> dict[str, Any]:
    """The deterministic Equity definition for one instrument."""
    first_open = min(e.ts_event for e in events if str(e.instrument_id) == instrument)
    currency = next(e.currency for e in events if str(e.instrument_id) == instrument)
    symbol = instrument.split(".")[0]
    return {
        "id": instrument,
        "raw_symbol": symbol,
        "venue": EQUITY_VENUE,
        "currency": currency,
        "price_precision": EQUITY_PRICE_PRECISION,
        "price_increment": "0.0001",
        "size_precision": 0,
        "lot_size": str(EQUITY_LOT_SIZE),
        "is_quoted": False,
        "info": None,
        "ts_event": first_open,
        "ts_init": first_open,
    }


def catalog_content_hash(events: list[Any], equities: list[dict[str, Any]]) -> str:
    """sha256 over the canonical serialization of all events and instruments.

    Deterministic by construction: `to_dict` values are JSON scalars, the
    stream is sorted by `sort_event_stream`, and the instruments are sorted by
    id.  No build timestamps or random ids are included.
    """
    canonical: list[dict[str, Any]] = []
    for event in sort_event_stream(events):
        d = event.to_dict()
        d.pop("type", None)
        canonical.append(d)
    for equity in sorted(equities, key=lambda e: e["id"]):
        canonical.append({"instrument": equity["id"], "venue": equity["venue"],
                          "currency": equity["currency"],
                          "lot_size": equity["lot_size"],
                          "price_precision": equity["price_precision"]})
    payload = json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return f"sha256:{hashlib.sha256(payload).hexdigest()}"


def build_catalog(curated_root: Path, catalog_path: Path) -> dict[str, Any]:
    """Builds the NT catalog at `catalog_path` from the read-only curated zone.

    Returns {"content_hash", "event_count", "instruments", "classes",
    "data_versions"} for callers and the manifest.
    """
    rows = _read_curated_rows(curated_root)
    events = build_events_from_curated(rows)

    instruments = sorted({str(e.instrument_id) for e in events})
    equities = [_equity_dict(events, instrument) for instrument in instruments]
    data_versions = sorted({e.data_version for e in events})
    content_hash = catalog_content_hash(events, equities)

    # Rebuild the catalog directory deterministically (read-only input).
    if catalog_path.exists():
        shutil.rmtree(catalog_path)
    catalog_path.mkdir(parents=True)

    from nautilus_trader.model.instruments import Equity
    from nautilus_trader.persistence.catalog import ParquetDataCatalog

    catalog = ParquetDataCatalog(path=str(catalog_path))
    catalog.write_data(sort_event_stream(events))
    catalog.write_data([Equity.from_dict(e) for e in equities])

    manifest = {
        "schema_version": 1,
        "classes": ["SessionOpenEvent", "DailyBarClosedEvent"],
        "instruments": instruments,
        "data_versions": data_versions,
        "event_count": len(events),
        "content_hash": content_hash,
        "source": "data/curated (read-only)",
    }
    (catalog_path / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    return {
        "content_hash": content_hash,
        "event_count": len(events),
        "instruments": instruments,
        "classes": list(manifest["classes"]),
        "data_versions": data_versions,
    }
