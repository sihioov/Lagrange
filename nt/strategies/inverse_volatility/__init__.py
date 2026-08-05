"""The InverseVolatility baseline package."""

from strategies.inverse_volatility.package import (
    DEFAULT_PARAMETERS,
    PACKAGE,
    STRATEGY_ID,
    VERSION,
)


def generate_target(*args, **kwargs):
    from strategies.inverse_volatility.target import generate_target as _generate

    return _generate(*args, **kwargs)


__all__ = ["PACKAGE", "STRATEGY_ID", "VERSION", "DEFAULT_PARAMETERS", "generate_target"]
