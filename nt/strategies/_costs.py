"""Charging a fill under a versioned cost profile.

One implementation for every strategy that produces fills, because the
alternative is two that drift. `MA200Trend` and the baseline adapters both
call this; neither carries its own arithmetic.

What is mirrored here is the FORMULA, from `CostProfile::estimate` in
`crates/portfolio-model/src/cost.rs`:

    commission = max(notional x commission_rate, min_commission)
    tax        = sell ? notional x sell_tax_rate : 0

What is NOT mirrored is the RATES. `cost.rs` says it outright -- 세율과
수수료는 변경 가능하므로 코드 상수로 고정하지 않고 설정 버전으로 관리한다 --
so they arrive resolved from the versioned profile the runner looked up. A
rate written into a second place is a rate that changes in only one of them,
and a backtest would then charge fees no ledger agrees with.

Mirroring three lines of arithmetic is the price of fills being produced
inside the engine, where Rust cannot reach them.
"""

from decimal import ROUND_HALF_UP, Decimal
from typing import Mapping

#: Prices and money are carried as integers scaled by 10_000 (design §6.3).
SCALE = 10_000


def fees_for(
    profile: Mapping[str, object],
    side_is_sell: bool,
    quantity: int,
    price_raw: int,
) -> tuple[int, int]:
    """`(commission_raw, tax_raw)` at money scale for one fill.

    An empty profile charges nothing. That is the honest default: a run with
    no fees is visible in the artifacts as exactly that, while a locally
    invented rate would produce fees that look real and reconcile with
    nothing.

    `Decimal` throughout. A commission rate of 0.00015 has no exact binary
    form, and the ledger identity the normalizer asserts --
    ``cash_after - cash_before == +/-notional -/+ (commission + tax)`` -- is
    an exact equality that float rounding fails.
    """
    if not profile:
        return 0, 0
    scale = Decimal(SCALE)
    notional = (Decimal(quantity) * Decimal(price_raw)) / scale
    commission = max(
        notional * Decimal(str(profile.get("commission_rate", "0"))),
        Decimal(str(profile.get("min_commission", "0"))),
    )
    tax = (
        notional * Decimal(str(profile.get("sell_tax_rate", "0")))
        if side_is_sell
        else Decimal(0)
    )
    # Slippage is deliberately absent: it is already inside the execution
    # price the fill was made at, and charging it here would take it twice.
    return _as_raw(commission, scale), _as_raw(tax, scale)


def _as_raw(amount: Decimal, scale: Decimal) -> int:
    """To the scale-4 integer the Rust ledger records money in."""
    return int((amount * scale).quantize(Decimal(1), rounding=ROUND_HALF_UP))
