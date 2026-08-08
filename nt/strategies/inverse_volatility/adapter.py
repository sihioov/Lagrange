"""NT execution adapter of the InverseVolatility package (plan Todo 17)."""

import msgspec

from nautilus_trader.trading.config import StrategyConfig

from strategies._execution import TargetExecutionStrategy
from strategies.inverse_volatility.package import DEFAULT_PARAMETERS, STRATEGY_ID, VERSION


def _default_parameters() -> dict:
    return dict(DEFAULT_PARAMETERS)


class InverseVolatilityConfig(StrategyConfig, frozen=True):
    instrument_ids: list = (
        "069500.KRX", "102110.KRX", "229200.KRX", "143850.KRX", "133690.KRX",
    )
    parameters: dict = msgspec.field(default_factory=_default_parameters)
    slippage_bps: int = 10
    lot_size: int = 100
    initial_cash: str = "100000000"
    strategy_version: str = VERSION
    #: `date -> instrument -> factor -> raw value`, computed by the Rust
    #: factor-engine and passed in by the runner. A real config field rather
    #: than an entry in `parameters`, which is schema-validated with
    #: `additionalProperties: false` and would reject it.
    factor_series: dict = msgspec.field(default_factory=dict)


class InverseVolatilityAdapter(TargetExecutionStrategy):
    """Executes the InverseVolatility target portfolio at the next session open."""

    STRATEGY_ID = STRATEGY_ID
    VERSION = VERSION


__all__ = ["InverseVolatilityAdapter", "InverseVolatilityConfig"]
