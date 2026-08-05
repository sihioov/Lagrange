"""The DualMomentum baseline package (plan Todo 17, design §8.2).

Dual momentum: invest fully in the strongest 12-month performer of the
universe when its return exceeds the absolute threshold; otherwise hold
defensive cash.  Rebalanced monthly, executed at the next open.  Consumes the
`return_12m` factor (the 6-month configuration consumes `return_6m`).
"""

import json
from pathlib import Path

from strategies._common import StrategyState

STRATEGY_ID = "dual_momentum"
VERSION = "1.0.0"
NAME = "DualMomentum"
DESCRIPTION = (
    "Dual momentum (design §8.2): invest fully in the strongest 12-month "
    "performer of the universe when its return exceeds the absolute "
    "threshold; otherwise hold defensive cash.  Rebalanced monthly, executed "
    "at the next open."
)
RISK_DESCRIPTION = (
    "All-or-nothing switching between a single risky asset and cash; "
    "threshold flips can trigger large full repositions; no diversification; "
    "single-name failure risk while invested."
)
MARKETS = ["krx"]
ASSET_CLASSES = ["etf"]
CADENCES = ["daily"]
REQUIRED_FACTORS = ["return_12m"]
MINIMUM_LOOKBACK_SESSIONS = 252
DEFAULT_PARAMETERS = {"absolute_threshold": 0.0, "lookback_months": 12}
SCHEMA = json.loads(
    (Path(__file__).parent / "schema.json").read_text(encoding="utf-8")
)
GOLDEN_FIXTURE = "nt/strategies/dual_momentum/golden.json"

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
