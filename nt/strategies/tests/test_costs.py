"""The fee formula, pinned to the same numbers the Rust model produces.

`fees_for` mirrors `CostProfile::estimate` (crates/portfolio-model/src/cost.rs)
because fills are produced inside the engine, where Rust cannot reach them.
Two implementations of one formula drift unless something holds them to the
same answers, and that is what this file is for.

The values below are computed by hand from the documented KRX_ETF_DEFAULT
settings, so a drift on EITHER side fails: the Rust unit tests pin the same
arithmetic, and `backtest_runner.rs` pins the end-to-end result at
14914.0992 KRW for the phase-0 buy_and_hold fill.
"""

from decimal import Decimal

from strategies._costs import SCALE, fees_for

#: KRX_ETF_DEFAULT as `CostProfile::krx_etf_default()` resolves it: commission
#: 0.015%, minimum 1,000 KRW, ETF sell tax 0%.
PROFILE = {
    "profile_id": "KRX_ETF_DEFAULT",
    "version": 1,
    "commission_rate": "0.00015",
    "min_commission": "1000.0000",
    "sell_tax_rate": "0",
    "slippage_bps": 10,
}


def _raw(krw: str) -> int:
    return int(Decimal(krw) * SCALE)


def test_commission_is_the_rate_on_notional():
    # 9700 x 10250.24 = 99,427,328 KRW; x 0.00015 = 14,914.0992.
    # The same fill the end-to-end runner test asserts, so the two cannot
    # disagree without one of them failing.
    commission, tax = fees_for(PROFILE, False, 9700, 102_502_400)
    assert commission == _raw("14914.0992")
    assert tax == 0


def test_a_small_trade_pays_the_minimum_not_the_rate():
    # 100 x 1000 = 100,000 KRW; x 0.00015 = 15 KRW, which is below the
    # 1,000 KRW floor. A rate applied without the floor understates the cost
    # of small trades, which is exactly where the floor matters.
    commission, _ = fees_for(PROFILE, False, 100, 10_000_000)
    assert commission == _raw("1000.0000")


def test_the_etf_default_charges_no_sell_tax():
    # 0% is the documented ETF setting, not an omission: a securities tax
    # that silently appeared would make every sell look worse than it is.
    commission, tax = fees_for(PROFILE, True, 9700, 102_502_400)
    assert tax == 0
    assert commission == _raw("14914.0992")


def test_a_sell_tax_is_charged_when_the_profile_carries_one():
    # Guards the branch itself, so the zero above is a SETTING rather than a
    # code path that does nothing.
    profile = dict(PROFILE, sell_tax_rate="0.0018")
    _, tax = fees_for(profile, True, 9700, 102_502_400)
    # 99,427,328 x 0.0018 = 178,969.1904
    assert tax == _raw("178969.1904")


def test_no_profile_charges_nothing():
    # An unpriced run reports no fees rather than invented ones. Silence is
    # visible in the artifacts; a fabricated rate is not.
    assert fees_for({}, False, 9700, 102_502_400) == (0, 0)


def test_fees_are_exact_rather_than_floating():
    # The normalizer asserts `cash == initial - fills - fees` exactly. A float
    # path produces 14914.099199999999 here and fails that identity by a
    # rounding error nobody can trace back to this function.
    commission, _ = fees_for(PROFILE, False, 9700, 102_502_400)
    assert commission == 149_140_992
    assert isinstance(commission, int)
