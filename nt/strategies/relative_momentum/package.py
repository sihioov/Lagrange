"""The RelativeMomentum baseline package (plan Todo 17, FR-STR-004).

Relative momentum: rank the fixed Korean ETF universe by 12-minus-1 month
momentum and hold the top N equally weighted, rebalanced at the monthly close
and executed at the next open.  Consumes the `momentum_12_1` factor (the 6-
month configuration consumes `return_6m`, same month-end convention).
"""

import json
from pathlib import Path

from strategies._common import StrategyState

STRATEGY_ID = "relative_momentum"
VERSION = "1.0.0"
NAME = "RelativeMomentum"
DESCRIPTION = (
    "Relative momentum: rank the fixed Korean ETF universe by 12-minus-1 "
    "month momentum and hold the top N equally weighted, rebalanced at the "
    "monthly close and executed at the next open."
)
RISK_DESCRIPTION = (
    "Momentum crash and reversal risk; concentration in the top-N names; "
    "monthly rotation turnover and cost drag; the lookback excludes the most "
    "recent month (12-minus-1) by design."
)
MARKETS = ["krx"]
ASSET_CLASSES = ["etf"]
CADENCES = ["daily"]
REQUIRED_FACTORS = ["momentum_12_1"]
MINIMUM_LOOKBACK_SESSIONS = 252
DEFAULT_PARAMETERS = {"top_n": 3, "lookback_months": 12}
SCHEMA = json.loads(
    (Path(__file__).parent / "schema.json").read_text(encoding="utf-8")
)
GOLDEN_FIXTURE = "nt/strategies/relative_momentum/golden.json"

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
