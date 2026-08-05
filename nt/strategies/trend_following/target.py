"""Target generator: trend-following on the benchmark (FR-STR-004).

Long the benchmark at 100% while the fast moving average exceeds the slow
one, defensive cash otherwise.  Consumes the `trend_<fast>` / `trend_<slow>`
factors of the benchmark instrument (defaults: `trend_50` / `trend_200`).
Outputs a TargetPortfolio-shaped result (Todo 16 boundary): targets only.
"""

from strategies._common import (
    TargetError,
    build_target_portfolio,
    reason,
    target_row,
    validate_params,
)
from strategies.trend_following.package import PACKAGE, STRATEGY_ID, VERSION


def generate_target(params, factors=None, as_of="", universe=None):
    validate_params(params, PACKAGE["parameter_schema"])
    benchmark = params["benchmark_instrument"]
    fast = int(params["fast_ma"])
    slow = int(params["slow_ma"])
    factors = factors or {}
    values = factors.get(benchmark, {})
    fast_value = values.get(f"trend_{fast}")
    slow_value = values.get(f"trend_{slow}")
    if fast_value is None or slow_value is None:
        missing = sorted(
            factor
            for factor in (f"trend_{fast}", f"trend_{slow}")
            if factor not in values or values[factor] is None
        )
        raise TargetError(
            "MISSING_REQUIRED_FACTOR",
            f"trend_following requires factors {missing} for {benchmark}",
        )
    if float(fast_value) > float(slow_value):
        targets = [
            target_row(
                benchmark,
                1,
                float(fast_value),
                {f"trend_{fast}": float(fast_value), f"trend_{slow}": float(slow_value)},
                1.0,
                [reason("TREND_POSITIVE", fast=str(fast), slow=str(slow))],
            )
        ]
        cash_weight = 0.0
        portfolio_reasons = []
    else:
        targets = []
        cash_weight = 1.0
        portfolio_reasons = [reason("TREND_NEGATIVE_CASH", fast=str(fast), slow=str(slow))]
    return build_target_portfolio(
        as_of=as_of,
        strategy_version=f"{STRATEGY_ID}@{VERSION}",
        targets=targets,
        exclusions=[],
        cash_weight=cash_weight,
        constraints={
            "top_n": 1,
            "max_weight": 1.0,
            "cash_floor": 0.0,
            "weight_scale": 4,
            "tolerance": 1e-9,
        },
        portfolio_reasons=portfolio_reasons,
    )
