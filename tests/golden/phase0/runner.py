"""Phase 0 isolated NautilusTrader backtest runner (plan Todo 14).

One golden run = ONE fresh process.  The runner is designed to be executed
via subprocess (see tests/golden/phase0/test_phase0_gate.py) so every run
starts from a pristine NT kernel - ADR-005 per-job process isolation.  A
module-level guard additionally rejects a second run inside the same process.

Pipeline (all deterministic):
  1. synth_data.generate_curated_rows() -> curated parquet zone
     (data/<data-root>/curated, gitignored)
  2. custom-data.catalog_builder.build_catalog -> NT catalog with
     SessionOpenEvent / DailyBarClosedEvent / Equity definitions
  3. QuoteTick per session open (bid/ask = raw open -/+ configured slippage
     bps) written into the catalog - the execution liquidity that the
     matching engine uses to fill the MARKET orders at the next session open
  4. BacktestNode run with the versioned MA-200 strategy
  5. recommendation/order/fill/equity/fee/metric/provenance artifacts written
     to --out-dir, plus summary.json

Exit codes: 0 = gate run PASS; 1 = gate FAIL (future-field violations,
unfilled orders, missing artifacts); 2 = usage/runner error.
"""
from __future__ import annotations

import argparse
import importlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
PHASE0_DIR = Path(__file__).resolve().parent

for _path in (
    str(REPO_ROOT / "nt"),
    str(REPO_ROOT / "nt" / "strategies"),
    str(REPO_ROOT / "scripts" / "golden"),
    str(PHASE0_DIR),
):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import phase0_dataset  # noqa: E402
import synth_data  # noqa: E402
from golden_lib import canonical_json_bytes, hash_bytes  # noqa: E402

_catalog_builder = importlib.import_module("custom-data.catalog_builder")
_session_events = importlib.import_module("custom-data.session_events")
SessionOpenEvent = _session_events.SessionOpenEvent
DailyBarClosedEvent = _session_events.DailyBarClosedEvent

import ma200_trend  # noqa: E402

from nautilus_trader.model.data import DataType, QuoteTick  # noqa: E402
from nautilus_trader.model.identifiers import InstrumentId  # noqa: E402
from nautilus_trader.model.objects import Price, Quantity  # noqa: E402
from nautilus_trader.persistence.catalog import ParquetDataCatalog  # noqa: E402
from nautilus_trader.trading.config import ImportableStrategyConfig  # noqa: E402

ARTIFACTS = (
    "recommendation.json",
    "orders.json",
    "fills.json",
    "equity.json",
    "fees.json",
    "metrics.json",
    "provenance.json",
)
TIMEZONE = "Asia/Seoul"


class Phase0IsolationError(RuntimeError):
    """A second golden run was attempted inside the same process (ADR-005)."""


_guard_used = False


def assert_fresh_process() -> None:
    """Reject a second golden run inside one process (ADR-005 isolation)."""
    global _guard_used
    if _guard_used:
        raise Phase0IsolationError(
            "second golden run in the same process rejected: per-job process "
            "isolation (ADR-005) requires one fresh process per run"
        )
    _guard_used = True


def materialize_quotes(catalog: object, rows: list[dict], slippage_bps: int) -> None:
    """QuoteTick per session open: bid = open - slip, ask = open + slip."""
    quotes = []
    for row in rows:
        open_raw4 = int(row["open"])
        open_ts = int(row["market_open_ts"]) * 1_000
        bid_raw4, ask_raw4 = synth_data.quote_raw_for_open(open_raw4, slippage_bps)
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


# --------------------------------------------------------------------------- #
# backtest
# --------------------------------------------------------------------------- #

def effective_config(args: argparse.Namespace) -> dict:
    return {
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "ma_period": args.ma_period,
        "slippage_bps": args.slippage_bps,
        "lot_size": args.lot_size,
        "initial_cash": args.initial_cash,
        "seed": args.seed,
        "data_version": synth_data.DATA_VERSION,
        "data_generator": "synth_data",
        "data_generator_version": synth_data.GENERATOR_VERSION,
        "engine_version": args.engine_version,
        "timezone": TIMEZONE,
        "instruments": list(synth_data.INSTRUMENTS),
    }


def run_golden(args: argparse.Namespace) -> tuple[object, dict, dict]:
    import nautilus_trader
    from nautilus_trader.backtest.config import (
        BacktestDataConfig, BacktestEngineConfig, BacktestRunConfig, BacktestVenueConfig,
    )
    from nautilus_trader.backtest.node import BacktestNode

    data_root = Path(args.data_root)
    curated_root = data_root / "curated"
    catalog_path = data_root / "catalog"

    rows = synth_data.generate_curated_rows()
    phase0_dataset.materialize_curated_zone(rows, curated_root)
    catalog = _catalog_builder.build_catalog(curated_root, catalog_path)
    catalog_obj = ParquetDataCatalog(path=str(catalog_path))
    materialize_quotes(catalog_obj, rows, args.slippage_bps)

    strategy_config = {
        "instrument_ids": list(synth_data.INSTRUMENTS),
        "ma_period": args.ma_period,
        "slippage_bps": args.slippage_bps,
        "lot_size": args.lot_size,
        "initial_cash": args.initial_cash,
        "strategy_version": args.strategy_version,
        "probe_future_fields": args.probe_future_fields,
    }

    data = []
    for iid in synth_data.INSTRUMENTS:
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_path),
            data_cls=QuoteTick,
            instrument_id=InstrumentId.from_str(iid),
            client_id="SIM",
        ))
    for iid in synth_data.INSTRUMENTS:
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_path),
            data_cls="custom-data.session_events:SessionOpenEvent",
            instrument_id=InstrumentId.from_str(iid),
            client_id="CUSTOM",
        ))
        data.append(BacktestDataConfig(
            catalog_path=str(catalog_path),
            data_cls="custom-data.session_events:DailyBarClosedEvent",
            instrument_id=InstrumentId.from_str(iid),
            client_id="CUSTOM",
        ))

    config = BacktestRunConfig(
        venues=[BacktestVenueConfig(
            name="KRX", oms_type="HEDGING", account_type="CASH",
            starting_balances=[f"{args.initial_cash} KRW"],
        )],
        engine=BacktestEngineConfig(strategies=[
            ImportableStrategyConfig(
                strategy_path="ma200_trend:MA200Trend",
                config_path="ma200_trend:MA200TrendConfig",
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

    effective = effective_config(args)
    effective["engine_name"] = "nautilustrader"
    effective["installed_engine_version"] = nautilus_trader.__version__
    return strategy, catalog, effective


# --------------------------------------------------------------------------- #
# artifacts
# --------------------------------------------------------------------------- #

def _iso_open(date_str: str) -> str:
    return f"{date_str}T00:00:00Z"


def _iso_close(date_str: str) -> str:
    return f"{date_str}T06:30:00Z"


def build_artifacts(strategy: object, catalog: dict, effective: dict, args: argparse.Namespace) -> dict[str, dict]:
    rec_signals = []
    for rec in strategy.recommendations:
        rec_signals.append({
            "signal_date": rec["signal_date"],
            "effective_date": rec["effective_date"],
            "instrument": rec["instrument"],
            "action": rec["action"],
            "close": rec["close"],
            "ma200_raw": rec["ma200_raw"],
            "reason_codes": rec["reason_codes"],
            "warmup": rec["warmup"],
        })
    as_of_date = max((s["signal_date"] for s in rec_signals), default=None)

    held = {iid: strategy.positions[iid] > 0 for iid in synth_data.INSTRUMENTS}
    target_portfolio = []
    for iid in synth_data.INSTRUMENTS:
        target_portfolio.append({
            "instrument": iid,
            "target_weight": "0.333333" if held[iid] else "0.000000",
            "held": held[iid],
        })
    target_portfolio.append({"instrument": "CASH", "target_weight": "0.000000", "held": False})

    recommendation = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "currency": "KRW",
        "seed": args.seed,
        "timezone": TIMEZONE,
        "as_of_date": as_of_date,
        "ma_period": args.ma_period,
        "signals": rec_signals,
        "target_portfolio": target_portfolio,
    }

    orders = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "execution_window": "NEXT_SESSION_OPEN",
        "orders": [
            {
                "order_id": o["order_id"],
                "client_order_id": o["client_order_id"],
                "instrument": o["instrument"],
                "side": o["side"],
                "quantity": o["quantity"],
                "order_type": o["order_type"],
                "signal_date": o["signal_date"],
                "created_ts": _iso_close(o["signal_date"]) if o["signal_date"] else None,
                "execution_ts_target": _iso_open(o["created_date"]),
                "state": o["state"],
            }
            for o in strategy.orders
        ],
    }

    fills = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "fill_model": {
            "price_source": "NEXT_SESSION_OPEN",
            "slippage_bps": args.slippage_bps,
            "note": (
                "AT-02: fill price equals the next KRX session raw open plus the "
                "configured slippage bps (buy: ask = open + slippage; sell: "
                "bid = open - slippage). The signal-day close and the execution "
                "day's high/low/close are never touched."
            ),
        },
        # Selected explicitly, like every neighbouring artifact.
        #
        # This used to be `list(strategy.fills)`, so the approved golden's
        # shape was whatever the strategy happened to carry -- adding one
        # attribute to `MA200Trend` changed a PINNED artifact hash without
        # touching this file. An approved baseline has to be pinned to a
        # schema, not to a class's internals.
        #
        # Fee fields are deliberately absent: the phase-0 run passes no cost
        # profile and therefore makes no claim about costs. Adding them here
        # is a golden re-approval, which is its own deliberate act.
        "fills": [
            {
                "fill_id": f["fill_id"],
                "order_id": f["order_id"],
                "client_order_id": f["client_order_id"],
                "instrument": f["instrument"],
                "side": f["side"],
                "quantity": f["quantity"],
                "price_raw": f["price_raw"],
                "date": f["date"],
                "ts": f["ts"],
                "source": f["source"],
                "slippage_bps": f["slippage_bps"],
                "never_uses": f["never_uses"],
                "barrier_held": f["barrier_held"],
            }
            for f in strategy.fills
        ],
    }

    equity_points = [
        {
            "date": p["date"],
            "cash_raw": p["cash_raw"],
            "positions_value_raw": p["positions_value_raw"],
            "equity_raw": p["equity_raw"],
        }
        for p in strategy.equity_points
    ]
    initial_raw = int(args.initial_cash) * 10_000
    final_equity_raw = equity_points[-1]["equity_raw"] if equity_points else initial_raw
    total_return_pct = round((final_equity_raw / initial_raw - 1.0) * 100.0, 6)
    peak = initial_raw
    max_drawdown_pct = 0.0
    for p in equity_points:
        peak = max(peak, p["equity_raw"])
        if peak > 0:
            max_drawdown_pct = min(max_drawdown_pct, round((p["equity_raw"] / peak - 1.0) * 100.0, 6))

    equity = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "currency": "KRW",
        "initial_cash_raw": initial_raw,
        "equity_note": "Daily equity = cash + sum(position shares x session close); cash ledger debited/credited at fill prices.",
        "points": equity_points,
    }

    fees = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "currency": "KRW",
        "cost_profile": {
            "commission_bps": 0,
            "minimum_commission": "0.00",
            "tax_bps": 0,
            "slippage_bps": args.slippage_bps,
        },
        "total_fees": "0.00",
        "items": [],
        "note": "Phase 0 gate runs a zero-fee model; the cost model lands in Todo 18.",
    }

    metrics = {
        "schema_version": 1,
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "currency": "KRW",
        "total_return_pct": f"{total_return_pct:.6f}",
        "max_drawdown_pct": f"{max_drawdown_pct:.6f}",
        "num_fills": len(strategy.fills),
        "total_fees": "0.00",
        "final_equity_raw": final_equity_raw,
        "finite": True,
        "nan_free": True,
    }

    provenance = {
        "schema_version": 1,
        "run_id": f"phase0-{args.seed}-{args.slippage_bps}bps",
        "engine": "nautilustrader",
        "engine_version": args.engine_version,
        "installed_engine_version": effective["installed_engine_version"],
        "strategy_id": ma200_trend.STRATEGY_ID,
        "strategy_version": args.strategy_version,
        "dataset_version": synth_data.DATA_VERSION,
        "data_generator": "synth_data",
        "data_generator_version": synth_data.GENERATOR_VERSION,
        "data_seed": args.seed,
        "catalog_content_hash": catalog["content_hash"],
        "config_hash": hash_bytes(canonical_json_bytes({k: v for k, v in effective.items() if k != "installed_engine_version"})),
        "code_commit": args.code_commit,
        "random_seed": args.seed,
        "timezone": TIMEZONE,
        "process_model": "one-run-per-process (ADR-005)",
        "barrier": {"future_field_violations": list(strategy.violations)},
        "signals": rec_signals,
    }

    return {
        "recommendation.json": recommendation,
        "orders.json": orders,
        "fills.json": fills,
        "equity.json": equity,
        "fees.json": fees,
        "metrics.json": metrics,
        "provenance.json": provenance,
    }


def _git_head() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=str(REPO_ROOT),
            capture_output=True, text=True, check=True, timeout=10,
        ).stdout.strip()
    except Exception:
        return "unknown"


def _write_json(path: Path, obj: dict) -> None:
    path.write_text(
        json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=True) + "\n",
        encoding="utf-8",
    )


# --------------------------------------------------------------------------- #
# entrypoint
# --------------------------------------------------------------------------- #

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Phase 0 isolated NT backtest runner")
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--data-root", type=Path, default=REPO_ROOT / "data" / "phase0")
    parser.add_argument("--strategy-version", default=ma200_trend.STRATEGY_VERSION)
    parser.add_argument("--engine-version", default=None)
    parser.add_argument("--slippage-bps", type=int, default=10)
    parser.add_argument("--seed", type=int, default=synth_data.SEED)
    parser.add_argument("--ma-period", type=int, default=synth_data.MA_PERIOD)
    parser.add_argument("--lot-size", type=int, default=100)
    parser.add_argument("--initial-cash", type=str, default="100000000")
    parser.add_argument("--probe-future-fields", action="store_true")
    parser.add_argument("--code-commit", default=None)
    args = parser.parse_args(argv)

    assert_fresh_process()

    import nautilus_trader
    if args.engine_version is None:
        args.engine_version = nautilus_trader.__version__
    if args.code_commit is None:
        args.code_commit = _git_head()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        strategy, catalog, effective = run_golden(args)
        if os.environ.get("PHASE0_DEBUG_DUMP"):
            _write_json(out_dir / "debug_closes.json", {
                iid: list(strategy.closes[iid]) for iid in synth_data.INSTRUMENTS
            })
        artifacts = build_artifacts(strategy, catalog, effective, args)
        for name in ARTIFACTS:
            _write_json(out_dir / name, artifacts[name])

        violations = list(strategy.violations)
        failed = [o for o in strategy.orders if o["state"] != "FILLED"]
        status = "PASS"
        if args.probe_future_fields:
            status = "FAIL"
        if violations:
            status = "FAIL"
        if failed:
            status = "FAIL"
            for o in failed:
                print(f"UNFILLED_ORDER: {o['instrument']} {o['side']} {o['quantity']} {o['signal_date']}", file=sys.stderr)

        if violations:
            for v in violations:
                print(v, file=sys.stderr)

        import hashlib

        artifact_hashes = {}
        for name in ARTIFACTS:
            artifact_hashes[name] = hashlib.sha256((out_dir / name).read_bytes()).hexdigest()
        summary = {
            "status": status,
            "process": {"pid": os.getpid(), "argv": list(sys.argv)},
            "artifact_hashes": artifact_hashes,
            "versions": {
                "engine": args.engine_version,
                "installed_engine": nautilus_trader.__version__,
                "strategy": args.strategy_version,
                "data": synth_data.DATA_VERSION,
            },
            "barrier_violations": violations,
            "unfilled_orders": [o["instrument"] for o in failed],
            "num_fills": len(strategy.fills),
            "num_signals": len(strategy.recommendations),
            "catalog_content_hash": catalog["content_hash"],
        }
        _write_json(out_dir / "summary.json", summary)
        print(json.dumps(summary, sort_keys=True))
        return 0 if status == "PASS" else 1
    except Exception as exc:
        print(f"PHASE0_RUNNER_ERROR: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
