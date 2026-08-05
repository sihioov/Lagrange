"""The TrendFollowing baseline package (plan Todo 17, FR-STR-004).

Moving-average trend following on the benchmark ETF: long the benchmark while
the fast average exceeds the slow average, cash otherwise; signals at T-close
execute at T+1 open.  Consumes the `trend_50` / `trend_200` factors (defaults;
custom windows consume their matching `trend_<n>` factor).
"""

import json
from pathlib import Path

from strategies._common import StrategyState

STRATEGY_ID = "trend_following"
VERSION = "1.0.0"
NAME = "TrendFollowing"
DESCRIPTION = (
    "Moving-average trend following on the benchmark ETF: long the benchmark "
    "while the fast average exceeds the slow average, cash otherwise; "
    "signals at T-close execute at T+1 open."
)
RISK_DESCRIPTION = (
    "Whipsaw losses in choppy sideways regimes; concentrated single-position "
    "exposure; cash drag when out of the market; one-session signal lag "
    "(T-close -> T+1 open) by construction."
)
MARKETS = ["krx"]
ASSET_CLASSES = ["etf"]
CADENCES = ["daily"]
REQUIRED_FACTORS = ["trend_50", "trend_200"]
MINIMUM_LOOKBACK_SESSIONS = 200
DEFAULT_PARAMETERS = {
    "benchmark_instrument": "069500.KRX",
    "fast_ma": 50,
    "slow_ma": 200,
}
SCHEMA = json.loads(
    (Path(__file__).parent / "schema.json").read_text(encoding="utf-8")
)
GOLDEN_FIXTURE = "nt/strategies/trend_following/golden.json"

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
