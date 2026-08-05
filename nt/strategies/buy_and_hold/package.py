"""The BuyAndHold baseline package (plan Todo 17, FR-STR-004).

Benchmark buy-and-hold: hold the benchmark ETF (069500.KRX) at the declared
target weight from the first session open; no market timing.  No factors are
required and the lookback is zero.
"""

import json
from pathlib import Path

from strategies._common import StrategyState

STRATEGY_ID = "buy_and_hold"
VERSION = "1.0.0"
NAME = "BuyAndHold"
DESCRIPTION = (
    "Benchmark buy-and-hold: hold the benchmark ETF (069500.KRX) at the "
    "declared target weight from the first session open; no market timing."
)
RISK_DESCRIPTION = (
    "Full market exposure with no drawdown control; single-instrument "
    "concentration in the benchmark; no exit mechanism beyond the optional "
    "monthly rebalance."
)
MARKETS = ["krx"]
ASSET_CLASSES = ["etf"]
CADENCES = ["daily"]
REQUIRED_FACTORS = []
MINIMUM_LOOKBACK_SESSIONS = 0
DEFAULT_PARAMETERS = {
    "benchmark_instrument": "069500.KRX",
    "target_weight": 1.0,
    "rebalance_cadence": "none",
}
SCHEMA = json.loads(
    (Path(__file__).parent / "schema.json").read_text(encoding="utf-8")
)
GOLDEN_FIXTURE = "nt/strategies/buy_and_hold/golden.json"

PACKAGE = {
    "strategy_id": STRATEGY_ID,
    "version": VERSION,
    "name": NAME,
    "description": DESCRIPTION,
    "risk_description": RISK_DESCRIPTION,
    "parameter_schema": SCHEMA,
    "default_parameters": dict(DEFAULT_PARAMETERS),
    "markets": list(MARKETS),
    "asset_classes": list(ASSET_CLASSES),
    "cadences": list(CADENCES),
    "required_factors": list(REQUIRED_FACTORS),
    "minimum_lookback_sessions": MINIMUM_LOOKBACK_SESSIONS,
    "target_generator_ref": f"nt.strategies.{STRATEGY_ID}.target:generate_target",
    "nt_adapter_ref": f"nt.strategies.{STRATEGY_ID}.adapter:{NAME}Adapter",
    "golden_fixture_refs": [GOLDEN_FIXTURE],
    "state": StrategyState.DRAFT,
    "canonical_hash": "",
}
