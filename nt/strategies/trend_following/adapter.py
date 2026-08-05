"""NT execution adapter of the TrendFollowing package (plan Todo 17)."""

import msgspec

from nautilus_trader.trading.config import StrategyConfig

from strategies._execution import TargetExecutionStrategy
from strategies.trend_following.package import DEFAULT_PARAMETERS, STRATEGY_ID, VERSION


def _default_parameters() -> dict:
    return dict(DEFAULT_PARAMETERS)


class TrendFollowingConfig(StrategyConfig, frozen=True):
    instrument_ids: list = ("069500.KRX",)
    parameters: dict = msgspec.field(default_factory=_default_parameters)
    slippage_bps: int = 10
    lot_size: int = 100
    initial_cash: str = "100000000"
    strategy_version: str = VERSION


class TrendFollowingAdapter(TargetExecutionStrategy):
    """Executes the TrendFollowing target portfolio at the next session open."""

    STRATEGY_ID = STRATEGY_ID
    VERSION = VERSION


__all__ = ["TrendFollowingAdapter", "TrendFollowingConfig"]
