"""The InverseVolatility baseline package (plan Todo 17, FR-STR-004).

Inverse volatility weighting: each eligible ETF receives weight inversely
proportional to its realized volatility (default 60 sessions), so calmer
names receive more capital; rebalanced monthly at the next open.  Consumes
the `vol_60` factor (custom windows consume their matching `vol_<n>` factor).
"""

import json
from pathlib import Path

from strategies._common import StrategyState

STRATEGY_ID = "inverse_volatility"
VERSION = "1.0.0"
NAME = "InverseVolatility"
DESCRIPTION = (
    "Inverse volatility weighting: each eligible ETF receives weight "
    "inversely proportional to its realized volatility (default 60 sessions), "
    "so calmer names receive more capital; rebalanced monthly at the next "
    "open."
)
RISK_DESCRIPTION = (
    "Low-volatility concentration during calm markets; underweights "
    "high-volatility momentum winners; monthly rebalancing turnover; "
    "volatility estimation lag."
)
MARKETS = ["krx"]
ASSET_CLASSES = ["etf"]
CADENCES = ["daily"]
REQUIRED_FACTORS = ["vol_60"]
MINIMUM_LOOKBACK_SESSIONS = 60
DEFAULT_PARAMETERS = {"vol_window": 60, "max_weight": 0.3}
SCHEMA = json.loads(
    (Path(__file__).parent / "schema.json").read_text(encoding="utf-8")
)
GOLDEN_FIXTURE = "nt/strategies/inverse_volatility/golden.json"

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
