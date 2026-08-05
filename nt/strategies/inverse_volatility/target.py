"""Target generator: inverse volatility weighting (FR-STR-004).

Each eligible ETF receives weight inversely proportional to its realized
volatility (`vol_60` by default; `vol_<n>` for custom windows), capped at
`max_weight`, weights rounded to 4 decimals, residue to cash.  Instruments
with a NULL/absent factor are excluded with `EXCLUDED_MANDATORY_FACTOR_NULL`.
Outputs a TargetPortfolio-shaped result (Todo 16 boundary): targets only.
"""

from strategies._common import (
    TargetError,
    build_target_portfolio,
    exclusion_row,
    reason,
    target_row,
    validate_params,
)
from strategies.inverse_volatility.package import PACKAGE, STRATEGY_ID, VERSION


def generate_target(params, factors=None, as_of="", universe=None):
    validate_params(params, PACKAGE["parameter_schema"])
    vol_window = int(params["vol_window"])
    max_weight = float(params["max_weight"])
    factor_id = f"vol_{vol_window}"
    universe = list(universe or [])
    factors = factors or {}
    if not universe:
        raise TargetError("MISSING_UNIVERSE", "inverse_volatility requires a universe")
    rows = [
        (iid, float(factors[iid][factor_id]))
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is not None
    ]
    if not rows:
        raise TargetError(
            "MISSING_REQUIRED_FACTOR",
            f"inverse_volatility requires factor {factor_id}",
        )
    rows.sort(key=lambda pair: pair[0])
    inverse = [(iid, 1.0 / vol) for iid, vol in rows]
    total = sum(weight for _, weight in inverse)
    raw = {iid: weight / total for iid, weight in inverse}
    capped = {iid: min(weight, max_weight) for iid, weight in raw.items()}
    rounded = {iid: round(weight, 4) for iid, weight in capped.items()}
    cash_weight = round(1.0 - sum(rounded.values()), 4)
    targets = [
        target_row(
            iid,
            index + 1,
            raw[iid],
            {factor_id: raw[iid]},
            rounded[iid],
            [
                reason(
                    "INVERSE_VOL_WEIGHTED",
                    instrument=iid,
                    vol=str(round(vol, 6)),
                    weight=str(rounded[iid]),
                )
            ]
            + (
                [reason("WEIGHT_CAPPED_AT_MAX", max_weight=str(max_weight))]
                if raw[iid] > max_weight
                else []
            ),
        )
        for index, (iid, vol) in enumerate(rows)
    ]
    exclusions = [
        exclusion_row(iid, [reason("EXCLUDED_MANDATORY_FACTOR_NULL", factor=factor_id)])
        for iid in universe
        if factors.get(iid, {}).get(factor_id) is None
    ]
    portfolio_reasons = (
        []
        if cash_weight == 0.0
        else [reason("WEIGHT_ROUNDING_RESIDUE_TO_CASH", residue=str(cash_weight))]
    )
    return build_target_portfolio(
        as_of=as_of,
        strategy_version=f"{STRATEGY_ID}@{VERSION}",
        targets=targets,
        exclusions=exclusions,
        cash_weight=cash_weight,
        constraints={
            "top_n": len(rows),
            "max_weight": max_weight,
            "cash_floor": 0.0,
            "weight_scale": 4,
            "tolerance": 1e-9,
        },
        portfolio_reasons=portfolio_reasons,
    )
