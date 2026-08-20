"""The isolated NT child: runs one backtest and writes raw results (Todo 20).

Executed by the supervisor as `python -m backtest_worker.simulate --request <json>`
with cwd = the run directory. The runtime guard (`guard.py`) is installed
FIRST so every NT import runs under the network / read-only-mount / signal
contract. The dataset path is read-only; the catalog is rebuilt inside the run
directory (temp); results are written to `raw/` plus a structured `status.json`.

Raw-collection contract: the strategy may expose the phase-0 style records
(`orders`, `fills`, `equity_points`, `violations`, `positions`,
`position_snapshots`, `closes`); positions snapshots are derived from fills
when the strategy does not record them. Fills carry `commission_raw`/`tax_raw`
(defaulting to 0). The benchmark is an equal-weight buy-and-hold of the
universe computed from the dataset (read-only).
"""
from __future__ import annotations

import argparse
import importlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .guard import install as install_guard

_SCALE = 4


class SimulateError(RuntimeError):
    """A typed child failure (reported in status.json)."""


def _fmt_ts(date_str: str, suffix: str) -> str:
    return f"{date_str}T{suffix}"


def _read_curated_rows(data_root: Path) -> list[dict[str, Any]]:
    import pyarrow.parquet as pq

    catalog_builder = importlib.import_module("custom-data.catalog_builder")

    # Phase-0 layout: build_catalog treats its argument as the DATA root and
    # reads <data_root>/curated/bars/... (materialized that way by the phase-0
    # runner); mirror it exactly so both readers agree.
    bars_dir = data_root / "curated" / "bars"
    if not bars_dir.is_dir():
        raise SimulateError(f"curated bars zone missing: {bars_dir}")
    rows: list[dict[str, Any]] = []
    for bars_path in sorted(bars_dir.rglob("bars.parquet")):
        table = pq.read_table(bars_path)
        instrument_ids = table.column("instrument_id").to_pylist()
        trading_dates = table.column("trading_date").cast("string").to_pylist()
        opens_ts = [int(v) for v in table.column("market_open_ts").cast("int64").to_pylist()]
        opens = catalog_builder.fixed_to_int(table, "open", _SCALE)
        closes = catalog_builder.fixed_to_int(table, "close", _SCALE)
        for i in range(len(instrument_ids)):
            rows.append({
                "instrument_id": instrument_ids[i],
                "trading_date": trading_dates[i],
                "market_open_ts": opens_ts[i],
                "open": opens[i],
                "close": closes[i],
            })
    if not rows:
        raise SimulateError(f"no curated bars found under {bars_dir}")
    return rows


def _apply_window(
    rows: list[dict[str, Any]], start: str | None, end: str | None
) -> list[dict[str, Any]]:
    """Cut the bars to the period the user asked to simulate.

    One filter, at the source. Fills, the equity curve, `_derive_positions`
    and `_benchmark` all read `rows`, so filtering here is what keeps them
    agreeing with each other; filtering in four places is how they stop.

    Both bounds are INCLUSIVE -- a request for 2020-01-01..2020-12-31 means
    the user wants 2020-12-31's bar, and an exclusive end would silently drop
    the last day of every window anyone ever writes.

    `trading_date` is a parquet `date32` read back as `"2020-01-20"`, and the
    runner has already rejected any bound that is not `YYYY-MM-DD`, so both
    sides of the comparison are zero-padded ISO dates and ordering by string
    is ordering by date. This is worth stating because the ledger check in
    this same worker compared a full timestamp against a date and was false
    forever without ever looking wrong.
    """
    if start is None and end is None:
        return rows
    kept = [r for r in rows if (start is None or r["trading_date"] >= start)
            and (end is None or r["trading_date"] <= end)]
    if not kept:
        # A zero-bar run that reports success would produce a flat equity
        # curve and no fills -- a result indistinguishable from a strategy
        # that chose not to trade. Name what was asked for and what exists.
        dates = [r["trading_date"] for r in rows]
        raise SimulateError(
            f"no curated bars in [{start}, {end}]; "
            f"dataset covers [{min(dates)}, {max(dates)}]"
        )
    return kept


def _materialize_quotes(catalog, rows: list[dict[str, Any]], slippage_bps: int) -> None:
    from nautilus_trader.model.data import QuoteTick
    from nautilus_trader.model.identifiers import InstrumentId
    from nautilus_trader.model.objects import Price, Quantity

    from .synth_utils import quote_raw_for_open

    quotes = []
    for row in rows:
        open_raw4 = row["open"]
        open_ts = row["market_open_ts"] * 1_000
        bid_raw4, ask_raw4 = quote_raw_for_open(open_raw4, slippage_bps)
        quotes.append(QuoteTick(
            instrument_id=InstrumentId.from_str(row["instrument_id"]),
            bid_price=Price.from_str(f"{bid_raw4 / 10_000:.4f}"),
            ask_price=Price.from_str(f"{ask_raw4 / 10_000:.4f}"),
            bid_size=Quantity.from_int(1_000_000),
            ask_size=Quantity.from_int(1_000_000),
            ts_event=open_ts,
            ts_init=open_ts,
        ))
    catalog.write_data(quotes)


def _run_backtest(request: dict[str, Any], run_dir: Path) -> dict[str, Any]:
    import nautilus_trader
    from nautilus_trader.backtest.config import (
        BacktestDataConfig, BacktestEngineConfig, BacktestRunConfig, BacktestVenueConfig,
    )
    from nautilus_trader.backtest.node import BacktestNode
    from nautilus_trader.model.data import DataType, QuoteTick
    from nautilus_trader.model.identifiers import InstrumentId
    from nautilus_trader.persistence.catalog import ParquetDataCatalog
    from nautilus_trader.trading.config import ImportableStrategyConfig

    dataset_path = Path(request["dataset_path"])
    catalog_dir = run_dir / "catalog"

    rows = _read_curated_rows(dataset_path)
    rows = _apply_window(rows, request.get("start_date"), request.get("end_date"))
    instruments = sorted({row["instrument_id"] for row in rows})

    builder = importlib.import_module("custom-data.catalog_builder")
    builder.build_catalog(dataset_path, catalog_dir)
    catalog = ParquetDataCatalog(path=str(catalog_dir))
    slippage_bps = int(request.get("slippage_bps", 10))
    _materialize_quotes(catalog, rows, slippage_bps)

    strategy_path = request["strategy_path"]
    module_name, _, class_name = strategy_path.partition(":")
    config_class_path = request.get("strategy_config_class", f"{module_name}:{class_name}Config")
    config_module_name, _, config_class_name = config_class_path.partition(":")

    strategy_config = dict(request.get("strategy_config", {}))
    strategy_config["instrument_ids"] = instruments
    # The factor values the strategy decides from, computed by the runner.
    # Forwarded like `instrument_ids`: both are things the RUN knows and the
    # stored config cannot.
    if "factor_series" in request:
        strategy_config["factor_series"] = request["factor_series"]
    if "cost_profile" in request:
        strategy_config["cost_profile"] = request["cost_profile"]

    data = []
    for instrument in instruments:
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_dir),
            data_cls=QuoteTick,
            instrument_id=InstrumentId.from_str(instrument),
            client_id="SIM",
        ))
    for instrument in instruments:
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_dir),
            data_cls="custom-data.session_events:SessionOpenEvent",
            instrument_id=InstrumentId.from_str(instrument),
            client_id="CUSTOM",
        ))
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_dir),
            data_cls="custom-data.session_events:DailyBarClosedEvent",
            instrument_id=InstrumentId.from_str(instrument),
            client_id="CUSTOM",
        ))

    initial_cash = str(request.get("initial_cash", "100000000"))
    # Bound the REPLAY, not just the quotes.
    #
    # Filtering `rows` bounds what `_materialize_quotes` writes, and that alone
    # is not enough: `build_catalog` above populates the catalog from the FULL
    # curated root, and the two session-event streams are read back out of that
    # catalog. A run whose quotes stopped at the window still replayed every
    # SessionOpenEvent and DailyBarClosedEvent in the dataset, so the engine
    # produced an equity point per dataset day and the window changed nothing
    # -- 260 points with a window, 260 without. The end-to-end test is what
    # showed this; the unit test on the row filter passed throughout, because
    # the row filter was never the thing that decided what the engine saw.
    #
    # `start`/`end` on the run config bound every stream at the source. The
    # bars are dated at T00:00:00Z (09:00 KST, the Korean open), so the end
    # bound runs to the close of `end_date` rather than its midnight, which
    # would drop the final day.
    window_start, window_end = request.get("start_date"), request.get("end_date")
    config = BacktestRunConfig(
        start=_fmt_ts(window_start, "00:00:00+00:00") if window_start else None,
        end=_fmt_ts(window_end, "23:59:59+00:00") if window_end else None,
        venues=[BacktestVenueConfig(
            name="KRX", oms_type="HEDGING", account_type="CASH",
            starting_balances=[f"{initial_cash} KRW"],
        )],
        engine=BacktestEngineConfig(strategies=[
            ImportableStrategyConfig(
                strategy_path=strategy_path,
                config_path=config_class_path,
                config=strategy_config,
            ),
        ]),
        data=data,
        dispose_on_completion=False,
    )
    node = BacktestNode(configs=[config])
    node.run()
    engine = node.get_engines()[0]
    strategy = [s for s in engine.trader.strategies()][0]

    # A strategy that could not do its job must not be published as a result.
    #
    # NautilusTrader catches whatever an `on_data` handler raises, logs it,
    # and continues -- correct for an engine, because one malformed event
    # should not destroy a run. The consequence here is that a strategy which
    # failed on EVERY event still reaches this line, and its empty `orders`
    # and `fills` normalize into a clean SUCCEEDED backtest that a user cannot
    # tell apart from a strategy that decided not to trade.
    #
    # So the strategy records the failure it could not raise its way out of,
    # and it is turned back into one here, where it fails the run.
    fatal = getattr(strategy, "fatal_error", None)
    if fatal:
        raise SimulateError(
            f"{fatal.get('code', 'STRATEGY_FAILED')}: {fatal.get('detail', '')}"
        )

    raw = _collect_raw(strategy, rows, instruments, request, nautilus_trader.__version__)
    return raw


def _collect_raw(strategy, rows, instruments, request, installed_engine_version: str) -> dict[str, Any]:
    orders = []
    for order in getattr(strategy, "orders", []):
        signal_date = order.get("signal_date")
        created_date = order.get("created_date")
        orders.append({
            "order_id": order.get("order_id"),
            "client_order_id": order.get("client_order_id"),
            "instrument": order["instrument"],
            "side": str(order["side"]).upper(),
            "quantity": int(order["quantity"]),
            "order_type": order.get("order_type", "MARKET"),
            "signal_date": signal_date,
            "created_ts": _fmt_ts(signal_date, "06:30:00Z") if signal_date else None,
            "execution_ts_target": _fmt_ts(created_date, "00:00:00Z") if created_date else None,
            "state": order.get("state", "SUBMITTED"),
        })

    fills = []
    for fill in getattr(strategy, "fills", []):
        fills.append({
            "fill_id": fill.get("fill_id", ""),
            "order_id": fill.get("order_id", ""),
            "client_order_id": fill.get("client_order_id", ""),
            "instrument": fill["instrument"],
            "side": str(fill["side"]).upper(),
            "quantity": int(fill["quantity"]),
            "price_raw": int(fill["price_raw"]),
            "ts": fill["ts"],
            "commission_raw": int(fill.get("commission_raw", 0)),
            "tax_raw": int(fill.get("tax_raw", 0)),
        })

    equity_points = list(getattr(strategy, "equity_points", []))
    if not equity_points:
        raise SimulateError("strategy recorded no equity points")
    equity_points = _dedupe_equity_points(equity_points)
    initial_cash_raw = int(request.get("initial_cash", "100000000")) * 10 ** _SCALE

    position_snapshots = getattr(strategy, "position_snapshots", None)
    positions = (
        list(position_snapshots)
        if position_snapshots
        else _derive_positions(fills, [point["date"] for point in equity_points], instruments)
    )

    fees_items = list(getattr(strategy, "fee_items", []))
    fees = {
        "total_fees_raw": sum(item.get("commission_raw", 0) + item.get("tax_raw", 0) for item in fees_items),
        "items": [
            {
                "ts": item.get("ts"),
                "commission_raw": int(item.get("commission_raw", 0)),
                "tax_raw": int(item.get("tax_raw", 0)),
            }
            for item in fees_items
        ],
    }

    benchmark = _benchmark(rows, initial_cash_raw)

    violations = list(getattr(strategy, "violations", []))
    raw = {
        "orders": {"orders": orders},
        "fills": {"fills": fills},
        "equity": {"initial_cash_raw": initial_cash_raw, "points": equity_points},
        "positions": {"positions": positions},
        "fees": fees,
        "benchmark": benchmark,
        "provenance": {
            "engine": "nautilustrader",
            "engine_version": request["engine_version"],
            "strategy_id": request["strategy_id"],
            "strategy_version": request["strategy_version"],
            "dataset_version": request["dataset_version"],
            "config_hash": request["config_sha256"],
            "code_commit": request["code_commit"],
            "random_seed": int(request["random_seed"]),
            "timezone": request["timezone"],
        },
        "metrics": {
            "num_fills": len(fills),
            "num_orders": len(orders),
            "violations": violations,
            "installed_engine_version": installed_engine_version,
        },
    }
    return raw


def _dedupe_equity_points(points: list[dict]) -> list[dict]:
    """The phase-0 strategy records one equity point per (instrument, session),
    so a session date appears once per instrument. Keep the LAST point per date
    (all instruments marked with their latest closes) and order by date."""
    by_date: dict[str, dict] = {}
    for point in points:
        by_date[point["date"]] = point
    return [by_date[date] for date in sorted(by_date)]


def _derive_positions(fills: list[dict], dates: list[str], instruments: list[str]) -> list[dict]:
    position: dict[str, int] = {instrument: 0 for instrument in instruments}
    by_date: dict[str, list[dict]] = {}
    for fill in fills:
        by_date.setdefault(fill["ts"][:10], []).append(fill)
    out: list[dict] = []
    for date in dates:
        for fill in by_date.get(date, []):
            delta = fill["quantity"] if fill["side"] == "BUY" else -fill["quantity"]
            position[fill["instrument"]] = position.get(fill["instrument"], 0) + delta
        for instrument in instruments:
            out.append({"date": date, "instrument": instrument, "quantity": position[instrument]})
    return out


def _benchmark(rows: list[dict[str, Any]], initial_cash_raw: int) -> dict[str, Any]:
    by_date: dict[str, dict[str, int]] = {}
    first_close: dict[str, int] = {}
    for row in rows:
        by_date.setdefault(row["trading_date"], {})[row["instrument_id"]] = row["close"]
        first_close.setdefault(row["instrument_id"], row["close"])
    dates = sorted(by_date)
    instruments = list(first_close)
    points = []
    for date in dates:
        ratio_sum = 0.0
        for instrument in instruments:
            close = by_date[date].get(instrument, first_close[instrument])
            ratio_sum += close / first_close[instrument]
        value_raw = int(round(initial_cash_raw * ratio_sum / len(instruments)))
        points.append({"date": date, "value_raw": value_raw})
    return {"points": points}


def _write_json(path: Path, obj: Any) -> None:
    path.write_text(json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=True) + "\n", encoding="utf-8")


def _write_status(path: Path, state: str, error: dict | None = None) -> None:
    payload = {"state": state, "pid": os.getpid(), "finished_at": datetime.now(timezone.utc).isoformat()}
    if error:
        payload["error"] = error
    _write_json(path, payload)


def main(argv: list[str] | None = None) -> int:
    install_guard()
    parser = argparse.ArgumentParser(description="isolated NT backtest child")
    parser.add_argument("--request", required=True, type=Path)
    args = parser.parse_args(argv)

    run_dir = Path.cwd()
    status_path = run_dir / "status.json"
    raw_dir = run_dir / "raw"
    _write_status(status_path, "RUNNING")
    try:
        request = json.loads(args.request.read_text(encoding="utf-8"))
        raw = _run_backtest(request, run_dir)
        raw_dir.mkdir(parents=True, exist_ok=True)
        for name, payload in raw.items():
            _write_json(raw_dir / f"{name}.json", payload)
        _write_status(status_path, "SUCCEEDED")
        return 0
    except Exception as exc:  # noqa: BLE001 - typed reporting, never crash silently
        _write_status(status_path, "FAILED", {"kind": type(exc).__name__, "detail": str(exc)})
        return 1


if __name__ == "__main__":
    sys.exit(main())
