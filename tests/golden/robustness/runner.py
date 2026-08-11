"""Todo 21: deterministic five-strategy golden runner (§12.2).

One strategy run = ONE fresh process (phase0 convention; a module guard
rejects a second run in the same process). The runner replays the
deterministic 260-session synthetic universe (tests/golden/phase0/
synth_data.py, seed 42) through the strategy's own target generator
(Todo 17 packages) at every session close, executes the targets at the NEXT
session open (sells before buys, integer lots, KRX_ETF_DEFAULT costs:
0.015% commission with a 1,000 KRW minimum, 10 bps slippage), and emits the
§12.2 golden artifacts per strategy:

  recommendation / orders / fills / equity / fees / metrics / provenance

ENGINE HONESTY: these goldens lock the deterministic TARGET->NEXT-OPEN
execution simulation (`lagrange-golden-sim`), NOT the NautilusTrader engine
(which the phase0 goldens lock). An upgrade of the simulation logic bumps
the sim version and fails the committed golden gate until the golden is
regenerated and APPROVED.

Everything is integer scale-4 KRW arithmetic (no floats on money paths);
the only randomness is the seeded data generator, so runs are byte-stable
across CPython versions and processes.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
DIR = Path(__file__).resolve().parent
GOLDEN_SET_OUT = DIR / "golden-set.json"

for _path in (
    str(REPO_ROOT / "tests" / "golden" / "phase0"),
    str(REPO_ROOT / "nt"),
    str(REPO_ROOT / "nt" / "strategies"),
    str(REPO_ROOT / "scripts" / "golden"),
):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import synth_data  # noqa: E402
from strategies._common import STRATEGY_IDS  # noqa: E402
from golden_lib import canonical_json_bytes, hash_bytes  # noqa: E402

SIM_ENGINE = "lagrange-golden-sim"
SIM_VERSION = "1.0.0"
LOT_SIZE = 100
SLIPPAGE_BPS = 10
COMMISSION_RATE = 0.00015  # 0.015% per fill
MIN_COMMISSION_KRW = 1_000
TAX_RATE = 0.0
SCALE4 = 10_000
TIMEZONE = "Asia/Seoul"

ARTIFACTS = (
    "recommendation",
    "orders",
    "fills",
    "equity",
    "fees",
    "metrics",
    "provenance",
)

# Golden configurations per strategy. The default parameters are used where
# they trade on the 260-session synthetic universe; trend_following and
# dual_momentum use schema-valid configurations whose signals fire within
# this dataset (documented in the artifact provenance). Any change here is
# an unapproved golden delta until the goldens are regenerated and approved.
GOLDEN_PARAMS: dict[str, dict] = {
    "buy_and_hold": {},
    "trend_following": {"fast_ma": 20, "slow_ma": 50},
    "relative_momentum": {},
    "dual_momentum": {"absolute_threshold": -0.20, "lookback_months": 12},
    "inverse_volatility": {},
}


class RunnerError(RuntimeError):
    """A typed runner failure (never a silent partial output)."""


_guard_used = False


def assert_fresh_process() -> None:
    """Reject a second golden run inside one process (phase0 isolation)."""
    global _guard_used
    if _guard_used:
        raise RunnerError("second golden run in the same process rejected")
    _guard_used = True


# --------------------------------------------------------------------------- #
# deterministic data + factors
# --------------------------------------------------------------------------- #

def session_schedule(rows: list[dict]) -> list[tuple[str, dict[str, int], dict[str, int]]]:
    """[(date, {iid: open_raw}, {iid: close_raw})] over the shared sessions."""
    by_date: dict[str, dict[str, dict[str, int]]] = {}
    for row in rows:
        day = by_date.setdefault(row["trading_date"], {})
        day[row["instrument_id"]] = {
            "open": synth_data.decimal_to_raw4(row["open"]),
            "close": synth_data.decimal_to_raw4(row["close"]),
        }
    schedule = [
        (date, {iid: bars["open"] for iid, bars in days.items()},
         {iid: bars["close"] for iid, bars in days.items()})
        for date, days in sorted(by_date.items())
    ]
    return schedule


def factor_values(closes: dict[str, list[int]], iid: str, params: dict) -> dict[str, float]:
    """The strategy-contract factors as-of the current T-close. The windows
    are driven by the strategy's parameters (trend_following asks for
    trend_{fast_ma}/trend_{slow_ma}; inverse_volatility for vol_{window})."""
    series = closes[iid]
    values: dict[str, float] = {}
    if len(series) >= 253:
        values["momentum_12_1"] = series[-22] / series[-253] - 1.0
    if len(series) >= 252:
        values["return_12m"] = series[-1] / series[-252] - 1.0
    for window in (int(params["fast_ma"]), int(params["slow_ma"])) if (
        "fast_ma" in params and "slow_ma" in params
    ) else ():
        if window <= len(series) and f"trend_{window}" not in values:
            values[f"trend_{window}"] = (
                series[-1] / (sum(series[-window:]) / window) - 1.0
            )
    if "vol_window" in params:
        window = int(params["vol_window"])
        if len(series) >= window + 1:
            log_returns = [
                series[i] / series[i - 1] - 1.0 for i in range(len(series) - window, len(series))
            ]
            mean = sum(log_returns) / len(log_returns)
            variance = sum((r - mean) ** 2 for r in log_returns) / len(log_returns)
            values[f"vol_{window}"] = (variance ** 0.5) * (252 ** 0.5)
    return values


# --------------------------------------------------------------------------- #
# execution simulation (integer scale-4)
# --------------------------------------------------------------------------- #

def slipped_price(raw_open: int, side: str, bps: int) -> int:
    """Scale-4 execution price from the raw scale-4 open: buy = open x (1 +
    bps/10000), sell = open x (1 - bps/10000). The synth rows carry scale-4
    raw integers (10,150.0000 KRW = 101_500_000)."""
    delta = raw_open * bps // 10_000
    return raw_open + delta if side == "BUY" else raw_open - delta


def commission_scale4(notional: int) -> int:
    """max(min_commission, notional x rate), scale-4 units."""
    min_comm = MIN_COMMISSION_KRW * SCALE4
    return max(min_comm, int(notional * COMMISSION_RATE))


def simulate(strategy_id: str, params: dict, rows: list[dict],
             code_commit: str | None = None) -> dict:
    """Runs the strategy over the synthetic universe; returns the artifacts."""
    instruments = list(synth_data.INSTRUMENTS)
    schedule = session_schedule(rows)
    generator = importlib.import_module(f"strategies.{strategy_id}.target").generate_target

    closes: dict[str, list[int]] = {iid: [] for iid in instruments}
    positions: dict[str, int] = {iid: 0 for iid in instruments}
    initial_cash_raw4 = 100_000_000 * SCALE4  # 100,000,000 KRW
    cash = initial_cash_raw4

    pending: dict[str, float] = {iid: 0.0 for iid in instruments}
    pending_signal_date: dict[str, str | None] = {iid: None for iid in instruments}
    signals: list[dict] = []
    orders: list[dict] = []
    fills: list[dict] = []
    fees: list[dict] = []
    equity_points: list[dict] = []
    last_marks: dict[str, int] = {}

    def current_equity() -> int:
        marked = cash
        for iid in instruments:
            mark = last_marks.get(iid, 0)
            marked += positions[iid] * mark
        return marked

    def record_order_and_fill(iid: str, date: str, side: str, qty: int, price_raw: int) -> None:
        signal_date = pending_signal_date.get(iid)
        notional = qty * price_raw
        fee = commission_scale4(notional)
        if side == "BUY":
            cash_delta = -notional - fee
        else:
            cash_delta = notional - fee
        nonlocal cash
        cash += cash_delta
        order_id = f"ord-{iid}-{date}"
        client_order_id = f"co-{iid}-{date}"
        orders.append({
            "order_id": order_id,
            "client_order_id": client_order_id,
            "instrument": iid,
            "side": side,
            "quantity": qty,
            "order_type": "MARKET",
            "signal_date": signal_date,
            "created_ts": f"{signal_date}T06:30:00Z" if signal_date else None,
            "execution_ts_target": f"{date}T00:00:00Z",
            "state": "FILLED",
        })
        fills.append({
            "fill_id": f"fill-{iid}-{date}",
            "order_id": order_id,
            "client_order_id": client_order_id,
            "instrument": iid,
            "side": side,
            "quantity": qty,
            "price_raw": price_raw,
            "date": date,
            "ts": f"{date}T00:00:00Z",
            "source": "NEXT_SESSION_OPEN",
            "slippage_bps": SLIPPAGE_BPS,
            "never_uses": ["signal_day_close", "execution_day_high",
                           "execution_day_low", "execution_day_close"],
            "barrier_held": True,
        })
        fees.append({
            "ts": f"{date}T00:00:00Z",
            "commission": fee,
            "tax": 0,
        })
        last_marks[iid] = price_raw
        for signal in signals:
            if (signal["instrument"] == iid and signal["signal_date"] == signal_date
                    and signal["effective_date"] is None):
                signal["effective_date"] = date
                break

    for session_index, (date, opens, closes_today) in enumerate(schedule):
        # ---- OPEN: execute the pending targets from the previous close ----
        for iid in instruments:
            weight = pending[iid]
            if weight == 0.0 and positions[iid] > 0:
                qty = positions[iid]
                sell_price = slipped_price(opens[iid], "SELL", SLIPPAGE_BPS)
                record_order_and_fill(iid, date, "SELL", qty, sell_price)
                positions[iid] = 0
            elif weight > 0.0 and positions[iid] == 0:
                exec_price = slipped_price(opens[iid], "BUY", SLIPPAGE_BPS)
                notional_target = int(current_equity() * weight)
                qty = (notional_target // exec_price // LOT_SIZE) * LOT_SIZE
                while qty > 0 and cash < qty * exec_price + commission_scale4(qty * exec_price):
                    qty -= LOT_SIZE
                if qty > 0:
                    record_order_and_fill(iid, date, "BUY", qty, exec_price)
                    positions[iid] = qty

        # ---- CLOSE: compute factors, generate the next pending target ----
        for iid in instruments:
            closes[iid].append(closes_today[iid])
        factors = {iid: factor_values(closes, iid, params) for iid in instruments}
        warmup = False
        try:
            portfolio = generator(
                dict(params), factors=factors, as_of=date, universe=instruments
            )
        except Exception as exc:
            # During lookback warmup the target generators raise
            # MISSING_REQUIRED_FACTOR (code carried on the typed error); the
            # documented behavior is "no signal yet" (all cash, warmup
            # flagged) - same semantics as the ma200-trend warmup path.
            if getattr(exc, "code", "") != "MISSING_REQUIRED_FACTOR":
                raise RunnerError(f"{strategy_id} at {date}: {exc}") from exc
            warmup = True
            portfolio = {"targets": [], "cash_weight": 1.0}
        portfolio_targets = {
            target["instrument_id"]: float(target["target_weight"])
            for target in portfolio["targets"]
        }
        for iid in instruments:
            pending[iid] = portfolio_targets.get(iid, 0.0)
            pending_signal_date[iid] = date
            reasons = next((t["reasons"] for t in portfolio["targets"]
                            if t["instrument_id"] == iid), [])
            signals.append({
                "signal_date": date,
                "effective_date": None,
                "instrument": iid,
                "action": "BUY" if pending[iid] > 0.0 else "FLAT",
                "target_weight": pending[iid],
                "reason_codes": [r["code"] for r in reasons] or (["WARMUP_LOOKBACK"] if warmup else []),
                "warmup": warmup,
            })

        # ---- daily equity at the session close (marks = today's close) ----
        positions_value = sum(
            positions[iid] * closes_today[iid] for iid in instruments
        )
        equity_points.append({
            "date": date,
            "cash_raw": cash,
            "positions_value_raw": positions_value,
            "equity_raw": cash + positions_value,
        })

    return build_artifacts(strategy_id, params, orders, fills, fees, signals,
                           equity_points, initial_cash_raw4, code_commit)


# --------------------------------------------------------------------------- #
# artifacts
# --------------------------------------------------------------------------- #

def _git_head() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=str(REPO_ROOT),
            capture_output=True, text=True, check=True, timeout=10,
        ).stdout.strip()
    except Exception:
        return "unknown"


def build_artifacts(strategy_id: str, params: dict, orders: list[dict], fills: list[dict],
                    fees: list[dict], signals: list[dict], equity_points: list[dict],
                    initial_cash_raw4: int, code_commit: str | None = None) -> dict[str, dict]:
    package = importlib.import_module(f"strategies.{strategy_id}.package")
    strategy_version = package.VERSION
    final_equity = equity_points[-1]["equity_raw"]
    total_return = final_equity / initial_cash_raw4 - 1.0
    peak = initial_cash_raw4
    max_drawdown = 0.0
    for point in equity_points:
        peak = max(peak, point["equity_raw"])
        max_drawdown = min(max_drawdown, point["equity_raw"] / peak - 1.0)
    total_fees = sum(fee["commission"] + fee["tax"] for fee in fees)

    daily_returns = [
        equity_points[i]["equity_raw"] / equity_points[i - 1]["equity_raw"] - 1.0
        for i in range(1, len(equity_points))
    ]
    n = len(daily_returns) or 1
    mean = sum(daily_returns) / n if daily_returns else 0.0
    variance = sum((r - mean) ** 2 for r in daily_returns) / n
    volatility = (variance ** 0.5) * (252 ** 0.5)
    sharpe = mean / volatility * (252 ** 0.5) if volatility > 0 else 0.0
    days = max(1, len(equity_points) - 1)
    cagr = (final_equity / initial_cash_raw4) ** (365.25 / days) - 1.0
    calmar = cagr / abs(max_drawdown) if max_drawdown < 0 else 0.0
    mean_equity = sum(p["equity_raw"] for p in equity_points) / len(equity_points)
    turnover = (sum(f["quantity"] * f["price_raw"] for f in fills) / mean_equity
                if mean_equity > 0 else 0.0)

    config_hash = hash_bytes(canonical_json_bytes({
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "params": params,
        "sim_engine": SIM_ENGINE,
        "sim_version": SIM_VERSION,
        "slippage_bps": SLIPPAGE_BPS,
        "lot_size": LOT_SIZE,
        "initial_cash_raw4": initial_cash_raw4,
    }))

    recommendation = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "currency": "KRW",
        "seed": synth_data.SEED,
        "timezone": TIMEZONE,
        "as_of_date": equity_points[-1]["date"],
        "params": params,
        "signals": signals,
        "target_portfolio": [
            {"instrument": s["instrument"], "target_weight": s["target_weight"],
             "reason_codes": s["reason_codes"]}
            for s in signals[-len(synth_data.INSTRUMENTS):]
        ],
    }

    artifact_orders = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "execution_window": "NEXT_SESSION_OPEN",
        "orders": orders,
    }

    artifact_fills = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "fill_model": {
            "price_source": "NEXT_SESSION_OPEN",
            "slippage_bps": SLIPPAGE_BPS,
            "note": ("Golden sim fills: buy at next-open x (1 + slippage), "
                     "sell at next-open x (1 - slippage); the signal-day close "
                     "and the execution day's high/low/close are never touched."),
        },
        "fills": fills,
    }

    artifact_equity = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "currency": "KRW",
        "initial_cash_raw": initial_cash_raw4,
        "equity_note": ("Daily equity = cash + sum(position shares x session close); "
                        "cash ledger debited/credited at execution prices."),
        "points": equity_points,
    }

    artifact_fees = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "currency": "KRW",
        "cost_profile": {
            "commission_bps": int(COMMISSION_RATE * 10_000),
            "minimum_commission": f"{MIN_COMMISSION_KRW}.00",
            "tax_bps": int(TAX_RATE * 10_000),
            "slippage_bps": SLIPPAGE_BPS,
        },
        "total_fees": f"{total_fees / SCALE4:.4f}",
        "items": fees,
    }

    artifact_metrics = {
        "schema_version": 1,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "currency": "KRW",
        "total_return_pct": f"{total_return * 100:.6f}",
        "max_drawdown_pct": f"{max_drawdown * 100:.6f}",
        "cagr_pct": f"{cagr * 100:.6f}",
        "volatility_pct": f"{volatility * 100:.6f}",
        "sharpe": f"{sharpe:.6f}",
        "calmar": f"{calmar:.6f}",
        "turnover": f"{turnover:.6f}",
        "num_fills": len(fills),
        "total_fees": f"{total_fees / SCALE4:.4f}",
        "final_equity_raw": final_equity,
        "finite": True,
        "nan_free": True,
    }

    artifact_provenance = {
        "schema_version": 1,
        "run_id": f"golden-sim-{strategy_id}-{SIM_VERSION}",
        "engine": SIM_ENGINE,
        "engine_version": SIM_VERSION,
        "strategy_id": strategy_id,
        "strategy_version": strategy_version,
        "dataset_version": synth_data.DATA_VERSION,
        "data_generator": "synth_data",
        "data_generator_version": synth_data.GENERATOR_VERSION,
        "data_seed": synth_data.SEED,
        "config_hash": config_hash,
        "code_commit": code_commit or _git_head(),
        "random_seed": synth_data.SEED,
        "timezone": TIMEZONE,
        "process_model": "one-run-per-process (phase0 convention)",
        "note": ("locks the deterministic target->next-open execution simulation; "
                 "the NautilusTrader engine path is locked by the phase0 goldens."),
    }

    return {
        "recommendation.json": recommendation,
        "orders.json": artifact_orders,
        "fills.json": artifact_fills,
        "equity.json": artifact_equity,
        "fees.json": artifact_fees,
        "metrics.json": artifact_metrics,
        "provenance.json": artifact_provenance,
    }


# --------------------------------------------------------------------------- #
# golden-set.json (Rust core-gate GoldenSet shape)
# --------------------------------------------------------------------------- #

def write_golden_set() -> None:
    artifacts = []
    for strategy in STRATEGY_IDS:
        for artifact in ARTIFACTS:
            path = DIR / "strategies" / strategy / "outputs" / f"{artifact}.json"
            artifacts.append({
                "id": f"{strategy}/{artifact}",
                "path": f"strategies/{strategy}/outputs/{artifact}.json",
                "sha256": f"sha256:{hashlib.sha256(path.read_bytes()).hexdigest()}",
            })
    golden_set = {
        "golden_id": "kr-etf-five-strategies-v1",
        "versions": {
            "data": {"id": synth_data.DATA_VERSION, "version": "1.0.0", "source": "synthetic"},
            "engine": {"name": SIM_ENGINE, "version": SIM_VERSION},
            "config": {"id": "golden-config-five-strategies-v1"},
            "seed": synth_data.SEED,
            "timezone": TIMEZONE,
        },
        "artifacts": artifacts,
    }
    GOLDEN_SET_OUT.write_text(
        json.dumps(golden_set, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


# --------------------------------------------------------------------------- #
# entrypoint
# --------------------------------------------------------------------------- #

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Five-strategy golden sim runner")
    parser.add_argument("--strategy-id", required=True, choices=list(STRATEGY_IDS))
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--code-commit", default=None,
                        help="pin the provenance code commit (hermetic regeneration)")
    parser.add_argument("--write-golden-set", action="store_true",
                        help="regenerate tests/golden/robustness/golden-set.json")
    args = parser.parse_args(argv)

    assert_fresh_process()

    package = importlib.import_module(f"strategies.{args.strategy_id}.package")
    params = dict(package.DEFAULT_PARAMETERS)
    params.update(GOLDEN_PARAMS.get(args.strategy_id, {}))
    rows = synth_data.generate_curated_rows()
    artifacts = simulate(args.strategy_id, params, rows, code_commit=args.code_commit)

    out_dir = Path(args.out_dir)
    outputs_dir = out_dir / "outputs"
    outputs_dir.mkdir(parents=True, exist_ok=True)
    for name in ARTIFACTS:
        key = f"{name}.json"
        path = outputs_dir / key
        path.write_text(
            json.dumps(artifacts[key], indent=2, sort_keys=True, ensure_ascii=True) + "\n",
            encoding="utf-8",
        )
    artifact_hashes = {
        name: hashlib.sha256((outputs_dir / f"{name}.json").read_bytes()).hexdigest()
        for name in ARTIFACTS
    }
    summary = {
        "status": "PASS",
        "strategy_id": args.strategy_id,
        "process": {"pid": os.getpid(), "argv": list(sys.argv)},
        "artifact_hashes": artifact_hashes,
        "versions": {
            "engine": SIM_ENGINE,
            "engine_version": SIM_VERSION,
            "strategy": package.VERSION,
            "data": synth_data.DATA_VERSION,
        },
        "num_fills": len(artifacts["fills.json"]["fills"]),
        "num_signals": len(artifacts["recommendation.json"]["signals"]),
    }
    (out_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if args.write_golden_set:
        write_golden_set()
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
