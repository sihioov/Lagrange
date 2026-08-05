"""Target generator: dual momentum (design §8.2, FR-STR-004).

Invests fully in the strongest performer of the universe when its return
exceeds the absolute threshold; otherwise holds defensive cash.  Consumes the
`return_12m` factor (default; `return_6m` for the 6-month configuration).
Deterministic tie-break on canonical instrument id.  Outputs a
TargetPortfolio-shaped result (Todo 16 boundary): targets only.
"""

from strategies._common import (
    TargetError,
    build_target_portfolio,
    exclusion_row,
    reason,
    target_row,
    validate_params,
)
from strategies.dual_momentum.package import PACKAGE, STRATEGY_ID, VERSION


def _factor_id(lookback_months: int) -> str:
    return "return_12m" if lookback_months == 12 else "return_6m"


def generate_target(params, factors=None, as_of="", universe=None):
    validate_params(params, PACKAGE["parameter_schema"])
    threshold = float(params["absolute_threshold"])
    factor_id = _factor_id(int(params["lookback_months"]))
    universe = list(universe or [])
    factors = factors or {}
    if not universe:
        raise TargetError("MISSING_UNIVERSE", "dual_momentum requires a universe")
    carried = {
        iid: float(factors[iid][factor_id])
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is not None
    }
    if not carried:
        raise TargetError(
            "MISSING_REQUIRED_FACTOR",
            f"dual_momentum requires factor {factor_id}",
        )
    top = max(carried, key=lambda iid: (carried[iid], iid))
    top_return = carried[top]
    exclusions = [
        exclusion_row(iid, [reason("EXCLUDED_MANDATORY_FACTOR_NULL", factor=factor_id)])
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is None
    ]
    if top_return > threshold:
        targets = [
            target_row(
                top,
                1,
                top_return,
                {factor_id: top_return},
                1.0,
                [
                    reason(
                        "ABSOLUTE_MOMENTUM_PASSED",
                        instrument=top,
                        return_=str(round(top_return, 6)),
                        threshold=str(threshold),
                    )
                ],
            )
        ]
        cash_weight = 0.0
        portfolio_reasons = []
    else:
        targets = []
        cash_weight = 1.0
        portfolio_reasons = [
            reason(
                "DEFENSIVE_CASH_SELECTED",
                instrument=top,
                return_=str(round(top_return, 6)),
                threshold=str(threshold),
            )
        ]
    return build_target_portfolio(
        as_of=as_of,
        strategy_version=f"{STRATEGY_ID}@{VERSION}",
        targets=targets,
        exclusions=exclusions,
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
