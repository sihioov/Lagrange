"""Target generator: hold the benchmark at the target weight (FR-STR-004).

Consumes no factors (lookback 0).  Outputs a TargetPortfolio-shaped result
(Todo 16 boundary): targets only — this generator never creates orders.
"""

from strategies._common import (
    TargetError,
    build_target_portfolio,
    exclusion_row,
    reason,
    target_row,
    validate_params,
)
from strategies.buy_and_hold.package import DEFAULT_PARAMETERS, PACKAGE, STRATEGY_ID, VERSION


def generate_target(params, factors=None, as_of="", universe=None):
    """The golden target: `benchmark_instrument` at `target_weight`, the
    remainder in cash."""
    validate_params(params, PACKAGE["parameter_schema"])
    benchmark = params["benchmark_instrument"]
    weight = float(params["target_weight"])
    cash_weight = round(1.0 - weight, 4)
    targets = [
        target_row(
            benchmark,
            1,
            weight,
            {},
            weight,
            [reason("BENCHMARK_HELD", benchmark=benchmark, target_weight=str(weight))],
        )
    ]
    portfolio_reasons = (
        []
        if cash_weight == 0.0
        else [reason("CASH_FLOOR_APPLIED", cash_floor=str(cash_weight))]
    )
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
