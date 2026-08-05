"""Target generator: relative momentum (FR-STR-004).

Ranks the fixed universe by the momentum factor (`momentum_12_1` by default;
`return_6m` for the 6-month configuration), holds the top N equally weighted
(1/N each, rounded to 4 decimals), deterministic tie-break on canonical
instrument id.  Instruments with a NULL/absent factor are excluded with
`EXCLUDED_MANDATORY_FACTOR_NULL`.  Outputs a TargetPortfolio-shaped result
(Todo 16 boundary): targets only.
"""

from strategies._common import (
    TargetError,
    build_target_portfolio,
    exclusion_row,
    reason,
    target_row,
    validate_params,
)
from strategies.relative_momentum.package import PACKAGE, STRATEGY_ID, VERSION


def _factor_id(lookback_months: int) -> str:
    return "momentum_12_1" if lookback_months == 12 else "return_6m"


def generate_target(params, factors=None, as_of="", universe=None):
    validate_params(params, PACKAGE["parameter_schema"])
    top_n = int(params["top_n"])
    factor_id = _factor_id(int(params["lookback_months"]))
    universe = list(universe or [])
    factors = factors or {}
    if not universe:
        raise TargetError("MISSING_UNIVERSE", "relative_momentum requires a universe")
    carried = [
        iid
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is not None
    ]
    if not carried:
        raise TargetError(
            "MISSING_REQUIRED_FACTOR",
            f"relative_momentum requires factor {factor_id}",
        )
    ranked = sorted(carried, key=lambda iid: (-float(factors[iid][factor_id]), iid))
    top = ranked[:top_n]
    weight = round(1.0 / len(top), 4)
    targets = [
        target_row(
            iid,
            rank + 1,
            float(factors[iid][factor_id]),
            {factor_id: float(factors[iid][factor_id])},
            weight,
            [reason("SELECTED_TOP_N", top_n=str(top_n), rank=str(rank + 1))],
        )
        for rank, iid in enumerate(top)
    ]
    exclusions = [
        exclusion_row(
            iid,
            [
                reason(
                    "NOT_SELECTED_BEYOND_TOP_N",
                    rank=str(rank + 1),
                    top_n=str(top_n),
                )
            ],
        )
        for rank, iid in enumerate(ranked[top_n:])
    ]
    exclusions += [
        exclusion_row(iid, [reason("EXCLUDED_MANDATORY_FACTOR_NULL", factor=factor_id)])
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is None
    ]
    total_weight = round(weight * len(top), 4)
    cash_weight = round(1.0 - total_weight, 4)
    portfolio_reasons = (
        []
        if top
        else [reason("ALL_CASH_NO_ELIGIBLE")]
    )
    return build_target_portfolio(
        as_of=as_of,
        strategy_version=f"{STRATEGY_ID}@{VERSION}",
        targets=targets,
        exclusions=exclusions,
        cash_weight=cash_weight,
        constraints={
            "top_n": top_n,
            "max_weight": weight,
            "cash_floor": 0.0,
            "weight_scale": 4,
            "tolerance": 1e-9,
        },
        portfolio_reasons=portfolio_reasons,
    )
