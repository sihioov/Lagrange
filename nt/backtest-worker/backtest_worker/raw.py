"""Loading and boundary-validation of NT raw results (plan Todo 20).

The child (`simulate.py`) writes raw NT outputs as JSON artifacts; this module
parses them into a typed [`RawResult`] and rejects malformed or non-finite
values (NaN/Infinity/out-of-range numbers) before any normalization runs, so
corrupt raw data never reaches the common model.
"""
from __future__ import annotations

import json
import math
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SCALE = 4


class RawError(ValueError):
    """A raw NT result is malformed or carries a non-finite value."""


def _finite_number(value: Any, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise RawError(f"{path}: expected a number, got {type(value).__name__}")
    if not math.isfinite(value):
        raise RawError(f"{path}: non-finite value {value!r}")
    return float(value)


def _int_value(value: Any, path: str) -> int:
    _finite_number(value, path)
    if isinstance(value, float):
        if not value.is_integer():
            raise RawError(f"{path}: expected an integer raw value, got {value!r}")
        return int(value)
    return value


def _rfc3339(value: Any, path: str) -> str:
    if not isinstance(value, str):
        raise RawError(f"{path}: expected an RFC 3339 timestamp, got {type(value).__name__}")
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError as exc:
        raise RawError(f"{path}: invalid timestamp {value!r}") from exc
    return value


def _date(value: Any, path: str) -> str:
    if not isinstance(value, str):
        raise RawError(f"{path}: expected a YYYY-MM-DD date, got {type(value).__name__}")
    try:
        datetime.strptime(value, "%Y-%m-%d")
    except ValueError as exc:
        raise RawError(f"{path}: invalid date {value!r}") from exc
    return value


@dataclass
class RawResult:
    """Validated raw NT backtest outputs (int64 scale-4 prices/money)."""

    orders: list[dict] = field(default_factory=list)
    fills: list[dict] = field(default_factory=list)
    equity: dict = field(default_factory=dict)
    positions: list[dict] = field(default_factory=list)
    fees: dict = field(default_factory=dict)
    benchmark: dict = field(default_factory=dict)
    provenance: dict = field(default_factory=dict)
    metrics: dict = field(default_factory=dict)

    @classmethod
    def from_dict(cls, raw: dict) -> "RawResult":
        parsed = cls()
        parsed._load_orders(raw.get("orders", {}).get("orders", []))
        parsed._load_fills(raw.get("fills", {}).get("fills", []))
        parsed._load_equity(raw.get("equity", {}))
        parsed._load_positions(raw.get("positions", {}).get("positions", []))
        parsed._load_fees(raw.get("fees", {}))
        parsed._load_benchmark(raw.get("benchmark", {}))
        parsed._load_provenance(raw.get("provenance", {}))
        parsed.metrics = raw.get("metrics", {})
        return parsed

    @classmethod
    def load(cls, raw_dir: Path) -> "RawResult":
        payload: dict[str, Any] = {}
        for name in ("orders", "fills", "equity", "positions", "fees", "benchmark", "provenance", "metrics"):
            path = raw_dir / f"{name}.json"
            if path.exists():
                payload[name] = json.loads(path.read_text(encoding="utf-8"))
        return cls.from_dict(payload)

    def _load_orders(self, orders: list[dict]) -> None:
        for i, order in enumerate(orders):
            path = f"orders[{i}]"
            self.orders.append({
                "order_id": str(order["order_id"]),
                "client_order_id": str(order["client_order_id"]),
                "instrument": str(order["instrument"]),
                "side": str(order["side"]).upper(),
                "quantity": _int_value(order["quantity"], f"{path}.quantity"),
                "order_type": str(order["order_type"]),
                "signal_date": _date(order["signal_date"], f"{path}.signal_date") if order.get("signal_date") else None,
                "created_ts": _rfc3339(order["created_ts"], f"{path}.created_ts") if order.get("created_ts") else None,
                "execution_ts_target": _rfc3339(order["execution_ts_target"], f"{path}.execution_ts_target")
                if order.get("execution_ts_target") else None,
                "state": str(order["state"]),
            })

    def _load_fills(self, fills: list[dict]) -> None:
        for i, fill in enumerate(fills):
            path = f"fills[{i}]"
            side = str(fill["side"]).upper()
            if side not in ("BUY", "SELL"):
                raise RawError(f"{path}.side: expected BUY/SELL, got {side!r}")
            self.fills.append({
                "fill_id": str(fill["fill_id"]),
                "order_id": str(fill["order_id"]),
                "client_order_id": str(fill["client_order_id"]),
                "instrument": str(fill["instrument"]),
                "side": side,
                "quantity": _int_value(fill["quantity"], f"{path}.quantity"),
                "price_raw": _int_value(fill["price_raw"], f"{path}.price_raw"),
                "ts": _rfc3339(fill["ts"], f"{path}.ts"),
                "commission_raw": _int_value(fill.get("commission_raw", 0), f"{path}.commission_raw"),
                "tax_raw": _int_value(fill.get("tax_raw", 0), f"{path}.tax_raw"),
            })

    def _load_equity(self, equity: dict) -> None:
        self.equity = {
            "initial_cash_raw": _int_value(equity.get("initial_cash_raw", 0), "equity.initial_cash_raw"),
            "points": [],
        }
        for i, point in enumerate(equity.get("points", [])):
            path = f"equity.points[{i}]"
            self.equity["points"].append({
                "date": _date(point["date"], f"{path}.date"),
                "cash_raw": _int_value(point["cash_raw"], f"{path}.cash_raw"),
                "positions_value_raw": _int_value(point["positions_value_raw"], f"{path}.positions_value_raw"),
                "equity_raw": _int_value(point["equity_raw"], f"{path}.equity_raw"),
            })

    def _load_positions(self, positions: list[dict]) -> None:
        for i, position in enumerate(positions):
            path = f"positions[{i}]"
            self.positions.append({
                "date": _date(position["date"], f"{path}.date"),
                "instrument": str(position["instrument"]),
                "quantity": _int_value(position["quantity"], f"{path}.quantity"),
            })

    def _load_fees(self, fees: dict) -> None:
        items = []
        for i, item in enumerate(fees.get("items", [])):
            path = f"fees.items[{i}]"
            items.append({
                "ts": _rfc3339(item.get("ts", ""), f"{path}.ts") if item.get("ts") else None,
                "commission_raw": _int_value(item.get("commission_raw", 0), f"{path}.commission_raw"),
                "tax_raw": _int_value(item.get("tax_raw", 0), f"{path}.tax_raw"),
            })
        self.fees = {"total_fees_raw": _int_value(fees.get("total_fees_raw", 0), "fees.total_fees_raw"), "items": items}

    def _load_benchmark(self, benchmark: dict) -> None:
        points = []
        for i, point in enumerate(benchmark.get("points", [])):
            path = f"benchmark.points[{i}]"
            points.append({
                "date": _date(point["date"], f"{path}.date"),
                "value_raw": _int_value(point["value_raw"], f"{path}.value_raw"),
            })
        self.benchmark = {"points": points}

    def _load_provenance(self, provenance: dict) -> None:
        for key in (
            "engine", "engine_version", "strategy_id", "strategy_version",
            "dataset_version", "config_hash", "code_commit", "random_seed", "timezone",
        ):
            if key not in provenance:
                raise RawError(f"provenance is missing {key!r}")
        self.provenance = {
            "engine": str(provenance["engine"]),
            "engine_version": str(provenance["engine_version"]),
            "strategy_id": str(provenance["strategy_id"]),
            "strategy_version": str(provenance["strategy_version"]),
            "dataset_version": str(provenance["dataset_version"]),
            "config_hash": str(provenance["config_hash"]),
            "code_commit": str(provenance["code_commit"]),
            "random_seed": int(provenance["random_seed"]),
            "timezone": str(provenance["timezone"]),
        }
