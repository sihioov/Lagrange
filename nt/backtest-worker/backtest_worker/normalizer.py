"""Result normalizer: NT raw results -> the platform common model (Todo 20).

Produces exactly the 13 sections the Rust `result-model` crate declares
(`summary, equity, drawdown, monthly_returns, orders, fills, positions, cash,
fees, benchmark, metrics, warnings, provenance`). Units match the Rust
contract: money as scale-4 decimal strings in JSON (int64 raw in Parquet),
returns/drawdowns as decimal fractions, timestamps RFC 3339 UTC, months
`YYYY-MM`, dates `YYYY-MM-DD`.

Integrity checks (design §6.10, mirrored by the Rust `BacktestResult::validate`):
- every computed metric is finite (NaN/Infinity rejected);
- dates increase monotonically in every timestamped array;
- cash + positions value == equity at every point;
- fill quantities reconcile with position snapshots;
- the cash ledger reconciles with fill notionals + fees;
- summary returns reconcile with the equity curve.

Nothing may be published through [`IntegrityGate`] until validation succeeds.
"""
from __future__ import annotations

import hashlib
import math
from datetime import datetime
from pathlib import Path
from typing import Any

import pyarrow as pa
import pyarrow.parquet as pq

from .raw import SCALE, RawResult

_TRADING_DAYS = 252.0


class NormalizerError(Exception):
    """Base class for normalizer rejections."""

    kind = "normalizer_error"

    def __init__(self, detail: str) -> None:
        super().__init__(detail)
        self.detail = detail


class NonFiniteValueError(NormalizerError):
    kind = "non_finite_value"


class DateRegressionError(NormalizerError):
    kind = "date_regression"


class LedgerMismatchError(NormalizerError):
    kind = "ledger_mismatch"


class PublicationDeniedError(NormalizerError):
    kind = "publication_denied"


def _fmt(raw_int: int) -> str:
    sign = "-" if raw_int < 0 else ""
    magnitude = abs(raw_int)
    return f"{sign}{magnitude // 10 ** SCALE}.{magnitude % 10 ** SCALE:0{SCALE}d}"


def _money(raw_int: int, currency: str = "KRW") -> dict[str, str]:
    """Domain `Money` wire form: {"amount": "scale-4", "currency": "KRW"}."""
    return {"amount": _fmt(raw_int), "currency": currency}


def _metric(name: str, value: float) -> float:
    if not math.isfinite(value):
        raise NonFiniteValueError(f"metric {name} computed a non-finite value {value!r}")
    return value


def _population_std(values: list[float]) -> float:
    if not values:
        return 0.0
    mean = sum(values) / len(values)
    return math.sqrt(sum((v - mean) ** 2 for v in values) / len(values))


class Normalizer:
    """Converts a validated [`RawResult`] into the common model."""

    def normalize(self, raw: RawResult, validate: bool = True) -> dict[str, Any]:
        if validate:
            self._check_raw_ledger(raw)
            self._check_dates(raw)
            self._check_fills_to_positions(raw)
            self._check_cash_ledger(raw)

        equity = [
            {"ts": point["date"] + "T00:00:00Z", "equity": _money(point["equity_raw"])}
            for point in raw.equity["points"]
        ]
        cash = [
            {"ts": point["date"] + "T00:00:00Z", "cash": _money(point["cash_raw"])}
            for point in raw.equity["points"]
        ]
        drawdown = self._drawdown(raw)
        monthly_returns = self._monthly_returns(raw)
        orders = [
            {
                "order_id": o["order_id"],
                "client_order_id": o["client_order_id"],
                "instrument": o["instrument"],
                "side": o["side"],
                "quantity": str(o["quantity"]),
                "order_type": o["order_type"],
                "signal_date": o["signal_date"],
                "created_ts": o["created_ts"],
                "execution_ts_target": o["execution_ts_target"],
                "state": o["state"],
            }
            for o in raw.orders
        ]
        fills = [
            {
                "fill_id": f["fill_id"],
                "order_id": f["order_id"],
                "client_order_id": f["client_order_id"],
                "instrument": f["instrument"],
                "side": f["side"],
                "quantity": str(f["quantity"]),
                "price": _fmt(f["price_raw"]),
                "ts": f["ts"],
                "commission": _money(f["commission_raw"]),
                "tax": _money(f["tax_raw"]),
            }
            for f in raw.fills
        ]
        positions = [
            {"date": p["date"], "instrument": p["instrument"], "quantity": str(p["quantity"])}
            for p in raw.positions
        ]
        fees = [
            {
                "ts": item["ts"] or fill_ts,
                "commission": _money(item["commission_raw"]),
                "tax": _money(item["tax_raw"]),
            }
            for item, fill_ts in self._fee_items_with_ts(raw)
        ]
        benchmark = [
            {"ts": point["date"] + "T00:00:00Z", "value": _money(point["value_raw"])}
            for point in raw.benchmark["points"]
        ]
        summary, metrics = self._summary_and_metrics(raw)

        result = {
            "summary": summary,
            "equity": equity,
            "drawdown": drawdown,
            "monthly_returns": monthly_returns,
            "orders": orders,
            "fills": fills,
            "positions": positions,
            "cash": cash,
            "fees": fees,
            "benchmark": benchmark,
            "metrics": metrics,
            "warnings": [],
            "provenance": {
                "engine": raw.provenance["engine"],
                "engine_version": raw.provenance["engine_version"],
                "strategy_id": raw.provenance["strategy_id"],
                "strategy_version": raw.provenance["strategy_version"],
                "dataset_version": raw.provenance["dataset_version"],
                "config_hash": raw.provenance["config_hash"],
                "code_commit": raw.provenance["code_commit"],
                "random_seed": raw.provenance["random_seed"],
                "timezone": raw.provenance["timezone"],
            },
        }
        if validate:
            self.validate(result)
        return result

    def validate(self, result: dict[str, Any]) -> None:
        self._validate_finite_metrics(result)
        self._validate_result_dates(result)
        self._validate_result_ledger(result)

    # -- raw-level integrity ------------------------------------------------ #

    def _check_raw_ledger(self, raw: RawResult) -> None:
        points = raw.equity["points"]
        if not points:
            raise LedgerMismatchError("equity curve is empty")
        for point in points:
            if point["cash_raw"] + point["positions_value_raw"] != point["equity_raw"]:
                raise LedgerMismatchError(
                    f"ledger mismatch at {point['date']}: cash {point['cash_raw']} + "
                    f"positions value {point['positions_value_raw']} != equity {point['equity_raw']}"
                )

    def _check_dates(self, raw: RawResult) -> None:
        self._monotonic("equity", [p["date"] for p in raw.equity["points"]])
        self._monotonic("positions", [p["date"] for p in raw.positions])
        self._monotonic("benchmark", [p["date"] for p in raw.benchmark["points"]])
        self._monotonic("fills", [f["ts"] for f in raw.fills])

    def _check_fills_to_positions(self, raw: RawResult) -> None:
        cumulative: dict[str, int] = {}
        fill_index = 0
        for position in raw.positions:
            while fill_index < len(raw.fills) and raw.fills[fill_index]["ts"][:10] <= position["date"]:
                fill = raw.fills[fill_index]
                delta = fill["quantity"] if fill["side"] == "BUY" else -fill["quantity"]
                cumulative[fill["instrument"]] = cumulative.get(fill["instrument"], 0) + delta
                fill_index += 1
            expected = cumulative.get(position["instrument"], 0)
            if position["quantity"] != expected:
                raise LedgerMismatchError(
                    f"ledger mismatch: position {position['instrument']} at {position['date']} is "
                    f"{position['quantity']}, fills sum to {expected}"
                )

    def _check_cash_ledger(self, raw: RawResult) -> None:
        initial = raw.equity["initial_cash_raw"]
        spent: list[tuple[str, int]] = []
        for fill in raw.fills:
            notional = fill["quantity"] * fill["price_raw"]
            spent.append((fill["ts"][:10], notional if fill["side"] == "BUY" else -notional))
        for item, fill_ts in self._fee_items_with_ts(raw):
            # Truncated to a DATE, exactly as the fills above are.
            #
            # Without this the comparison below is between a full timestamp
            # and a date -- `"2020-01-21T00:00:00+00:00" <= "2020-01-21"` is
            # False, because the longer string sorts after -- so no fee ever
            # counted toward `spent_by` and the identity this check exists to
            # enforce silently excluded the entire fee side.
            #
            # Invisible until fees became non-zero: every fill charged 0, so a
            # term that was never added always summed to the right answer.
            spent.append(((item["ts"] or fill_ts)[:10], item["commission_raw"] + item["tax_raw"]))
        for point in raw.equity["points"]:
            day = point["date"]
            spent_by = sum(value for (d, value) in spent if d <= day)
            expected = initial - spent_by
            if abs(expected - point["cash_raw"]) > 100:
                raise LedgerMismatchError(
                    f"cash ledger at {day} is {point['cash_raw']}, expected {expected} "
                    f"(initial {initial} minus fills+fees {spent_by})"
                )

    # -- derived sections --------------------------------------------------- #

    def _drawdown(self, raw: RawResult) -> list[dict[str, Any]]:
        peak = 0
        out = []
        for point in raw.equity["points"]:
            peak = max(peak, point["equity_raw"])
            drawdown = point["equity_raw"] / peak - 1.0 if peak > 0 else 0.0
            out.append({"ts": point["date"] + "T00:00:00Z", "drawdown": _metric("drawdown", drawdown)})
        return out

    def _monthly_returns(self, raw: RawResult) -> list[dict[str, Any]]:
        initial = raw.equity["initial_cash_raw"]
        by_month: dict[str, int] = {}
        order: list[str] = []
        for point in raw.equity["points"]:
            month = point["date"][:7]
            by_month[month] = point["equity_raw"]
            if month not in order:
                order.append(month)
        out = []
        previous = initial
        for month in order:
            current = by_month[month]
            out.append({"month": month, "return": _metric(f"monthly_return.{month}", current / previous - 1.0)})
            previous = current
        return out

    def _fee_items_with_ts(self, raw: RawResult) -> list[tuple[dict, str]]:
        if raw.fees["items"]:
            return [(item, "") for item in raw.fees["items"]]
        return [({"commission_raw": f["commission_raw"], "tax_raw": f["tax_raw"], "ts": f["ts"]}, f["ts"]) for f in raw.fills]

    def _summary_and_metrics(self, raw: RawResult) -> tuple[dict[str, Any], dict[str, float]]:
        points = raw.equity["points"]
        initial = raw.equity["initial_cash_raw"]
        final = points[-1]["equity_raw"]
        if initial <= 0:
            raise LedgerMismatchError("initial equity must be positive")

        returns = [points[i]["equity_raw"] / points[i - 1]["equity_raw"] - 1.0 for i in range(1, len(points))]
        total_return = _metric("total_return", final / initial - 1.0)
        n = len(points)
        cagr = _metric("cagr", (final / initial) ** (_TRADING_DAYS / (n - 1)) - 1.0) if n > 1 else 0.0
        mean_return = sum(returns) / len(returns) if returns else 0.0
        std_return = _population_std(returns)
        volatility = _metric("volatility", std_return * math.sqrt(_TRADING_DAYS))
        sharpe = _metric("sharpe", mean_return / std_return * math.sqrt(_TRADING_DAYS) if std_return > 0 else 0.0)
        downside = _population_std([min(r, 0.0) for r in returns])
        sortino = _metric("sortino", mean_return / downside * math.sqrt(_TRADING_DAYS) if downside > 0 else 0.0)
        peak = initial
        min_drawdown = 0.0
        for point in points:
            peak = max(peak, point["equity_raw"])
            min_drawdown = min(min_drawdown, point["equity_raw"] / peak - 1.0 if peak > 0 else 0.0)
        mdd = _metric("max_drawdown", min_drawdown)
        calmar = _metric("calmar", cagr / abs(mdd) if mdd < 0 else 0.0)

        traded = sum(f["quantity"] * f["price_raw"] for f in raw.fills)
        avg_equity = sum(p["equity_raw"] for p in points) / len(points)
        turnover = _metric("turnover", traded / avg_equity if avg_equity > 0 else 0.0)

        total_cost_raw = sum(f["commission_raw"] + f["tax_raw"] for f in raw.fills)

        benchmark = raw.benchmark["points"]
        benchmark_return = 0.0
        benchmark_mdd = 0.0
        if benchmark:
            b_first = benchmark[0]["value_raw"]
            b_last = benchmark[-1]["value_raw"]
            benchmark_return = _metric("benchmark_return", b_last / b_first - 1.0 if b_first > 0 else 0.0)
            b_peak = benchmark[0]["value_raw"]
            for point in benchmark:
                b_peak = max(b_peak, point["value_raw"])
                benchmark_mdd = min(benchmark_mdd, point["value_raw"] / b_peak - 1.0 if b_peak > 0 else 0.0)
        excess_return = _metric("excess_return", total_return - benchmark_return)
        active = []
        for i in range(1, len(points)):
            bench_i = benchmark[i]["value_raw"] / benchmark[i - 1]["value_raw"] - 1.0 if len(benchmark) > i else 0.0
            active.append(returns[i - 1] - bench_i)
        tracking_error = _metric("tracking_error", _population_std(active) * math.sqrt(_TRADING_DAYS))

        metrics = {
            "total_return": total_return,
            "cagr": cagr,
            "max_drawdown": mdd,
            "volatility": volatility,
            "sharpe": sharpe,
            "sortino": sortino,
            "calmar": calmar,
            "turnover": turnover,
            "total_cost": total_cost_raw / 10 ** SCALE,
            "benchmark_return": benchmark_return,
            "excess_return": excess_return,
            "benchmark_max_drawdown": benchmark_mdd,
            "tracking_error": tracking_error,
        }

        summary = {
            "currency": "KRW",
            "initial_equity": _money(initial),
            "final_equity": _money(final),
            "total_return": total_return,
            "cagr": cagr,
            "max_drawdown": mdd,
            "volatility": volatility,
            "sharpe": sharpe,
            "sortino": sortino,
            "calmar": calmar,
            "turnover": turnover,
            "total_cost": _money(total_cost_raw),
            "n_orders": len(raw.orders),
            "n_fills": len(raw.fills),
            "start_date": points[0]["date"],
            "end_date": points[-1]["date"],
        }
        return summary, metrics

    # -- result-level integrity (mirrors Rust BacktestResult::validate) ---- #

    def _validate_finite_metrics(self, result: dict[str, Any]) -> None:
        for key, value in result["metrics"].items():
            _metric(f"metrics.{key}", float(value))
        for field in ("total_return", "cagr", "max_drawdown", "volatility", "sharpe", "sortino", "calmar", "turnover"):
            _metric(f"summary.{field}", float(result["summary"][field]))
        for point in result["drawdown"]:
            _metric("drawdown", float(point["drawdown"]))
        for item in result["monthly_returns"]:
            _metric("monthly_returns", float(item["return"]))

    def _validate_result_dates(self, result: dict[str, Any]) -> None:
        self._monotonic("equity", [p["ts"] for p in result["equity"]])
        self._monotonic("drawdown", [p["ts"] for p in result["drawdown"]])
        self._monotonic("cash", [p["ts"] for p in result["cash"]])
        self._monotonic("fees", [p["ts"] for p in result["fees"]])
        self._monotonic("benchmark", [p["ts"] for p in result["benchmark"]])
        self._monotonic("positions", [p["date"] for p in result["positions"]])
        self._monotonic("monthly_returns", [p["month"] for p in result["monthly_returns"]])
        self._monotonic("fills", [p["ts"] for p in result["fills"]])

    def _validate_result_ledger(self, result: dict[str, Any]) -> None:
        initial = result["summary"]["initial_equity"]
        final = result["summary"]["final_equity"]
        if result["equity"][0]["equity"] != initial:
            raise LedgerMismatchError("summary initial equity does not match the equity curve start")
        if result["equity"][-1]["equity"] != final:
            raise LedgerMismatchError("summary final equity does not match the equity curve end")
        initial_value = float(initial["amount"])
        actual = float(final["amount"]) / initial_value - 1.0
        if abs(actual - float(result["summary"]["total_return"])) > 1e-6:
            raise LedgerMismatchError("summary total_return disagrees with the equity curve")

    def _monotonic(self, field: str, values: list[str]) -> None:
        for i in range(1, len(values)):
            if values[i] < values[i - 1]:
                raise DateRegressionError(f"date regression in {field} at index {i}: {values[i]} < {values[i - 1]}")


class IntegrityGate:
    """Refuses publication until a successful validation (mirrors the Rust gate)."""

    def __init__(self) -> None:
        self._validated = False
        self._failed = False

    def validate(self, result: dict[str, Any]) -> NormalizerError | None:
        try:
            Normalizer().validate(result)
        except NormalizerError as error:
            self._failed = True
            return error
        self._validated = True
        return None

    def publish(self) -> None:
        if self._failed:
            raise PublicationDeniedError("integrity validation failed; the result must not be published")
        if not self._validated:
            raise PublicationDeniedError("result was never validated")


ARTIFACT_TYPES = {
    "equity": "EQUITY_CURVE",
    "drawdown": "DRAWDOWN_CURVE",
    "monthly_returns": "MONTHLY_RETURNS",
    "orders": "ORDERS",
    "fills": "FILLS",
    "positions": "POSITIONS",
    "cash": "CASH_LEDGER",
    "fees": "FEES",
    "benchmark": "BENCHMARK",
}


def _table_for(section: str, rows: list[dict]) -> pa.Table:
    if section in ("equity", "cash", "fees", "benchmark"):
        value_column = {"equity": "equity", "cash": "cash", "fees": "commission", "benchmark": "value"}[section]
        columns = {"ts": [r["ts"] for r in rows]}
        if section == "fees":
            columns["tax"] = [r["tax"]["amount"] for r in rows]
        columns[value_column] = [r[value_column]["amount"] for r in rows]
        schema = pa.schema(
            [pa.field("ts", pa.string())]
            + ([pa.field("tax", pa.string())] if section == "fees" else [])
            + [pa.field(value_column, pa.string())]
        )
        return pa.Table.from_pydict(columns, schema=schema)
    if section == "drawdown":
        return pa.Table.from_pydict(
            {"ts": [r["ts"] for r in rows], "drawdown": [r["drawdown"] for r in rows]}
        )
    if section == "monthly_returns":
        return pa.Table.from_pydict(
            {"month": [r["month"] for r in rows], "return": [r["return"] for r in rows]}
        )
    if section == "orders":
        return pa.Table.from_pydict(
            {
                "order_id": [r["order_id"] for r in rows],
                "client_order_id": [r["client_order_id"] for r in rows],
                "instrument": [r["instrument"] for r in rows],
                "side": [r["side"] for r in rows],
                "quantity": [int(r["quantity"]) for r in rows],
                "order_type": [r["order_type"] for r in rows],
                "signal_date": [r["signal_date"] for r in rows],
                "created_ts": [r["created_ts"] for r in rows],
                "execution_ts_target": [r["execution_ts_target"] for r in rows],
                "state": [r["state"] for r in rows],
            }
        )
    if section == "fills":
        return pa.Table.from_pydict(
            {
                "fill_id": [r["fill_id"] for r in rows],
                "order_id": [r["order_id"] for r in rows],
                "client_order_id": [r["client_order_id"] for r in rows],
                "instrument": [r["instrument"] for r in rows],
                "side": [r["side"] for r in rows],
                "quantity": [int(r["quantity"]) for r in rows],
                "price": [r["price"] for r in rows],
                "ts": [r["ts"] for r in rows],
                "commission": [r["commission"]["amount"] for r in rows],
                "tax": [r["tax"]["amount"] for r in rows],
            }
        )
    if section == "positions":
        return pa.Table.from_pydict(
            {
                "date": [r["date"] for r in rows],
                "instrument": [r["instrument"] for r in rows],
                "quantity": [int(r["quantity"]) for r in rows],
            }
        )
    raise AssertionError(f"unknown artifact section {section}")


def write_parquet_artifacts(result: dict[str, Any], out_dir: Path) -> dict[str, dict]:
    """Materializes the 9 large arrays as Parquet; returns per-artifact stats.

    Money columns are stored as int64 scale-4 raw values (compact, exact,
    matching the curated-zone convention); returns/drawdowns are float64.
    """
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    artifacts: dict[str, dict] = {}
    for section, rows in result.items():
        if section not in ARTIFACT_TYPES:
            continue
        table = _table_for(section, rows)
        path = out_dir / f"{section}.parquet"
        pq.write_table(table, path)
        raw_bytes = path.read_bytes()
        artifacts[section] = {
            "artifact_type": ARTIFACT_TYPES[section],
            "parquet_path": path.name,
            "row_count": table.num_rows,
            "sha256": hashlib.sha256(raw_bytes).hexdigest(),
            "size_bytes": len(raw_bytes),
        }
    return artifacts
