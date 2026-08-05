"""NT execution adapter of the RelativeMomentum package (plan Todo 17)."""

import msgspec

from nautilus_trader.trading.config import StrategyConfig

from strategies._execution import TargetExecutionStrategy
from strategies.relative_momentum.package import DEFAULT_PARAMETERS, STRATEGY_ID, VERSION


def _default_parameters() -> dict:
    return dict(DEFAULT_PARAMETERS)


class RelativeMomentumConfig(StrategyConfig, frozen=True):
    instrument_ids: list = (
        "069500.KRX", "102110.KRX", "229200.KRX", "143850.KRX", "133690.KRX",
    )
    parameters: dict = msgspec.field(default_factory=_default_parameters)
    slippage_bps: int = 10
    lot_size: int = 100
    initial_cash: str = "100000000"
    strategy_version: str = VERSION


class RelativeMomentumAdapter(TargetExecutionStrategy):
    """Executes the RelativeMomentum target portfolio at the next session open."""

    STRATEGY_ID = STRATEGY_ID
    VERSION = VERSION


__all__ = ["RelativeMomentumAdapter", "RelativeMomentumConfig"]
