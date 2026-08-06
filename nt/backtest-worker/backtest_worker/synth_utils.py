"""Slippage quantization shared by the worker's quote liquidity (Todo 20).

Mirrors the phase-0 single source of truth (`tests/golden/phase0/synth_data.py`
`slipped_open_raw` / `quote_raw_for_open`) so worker-produced fill prices obey
the documented AT-02 rule: BUY fills at the ask (open + slippage bps), SELL at
the bid (open - slippage bps), rounded half-even on integer scale-4 raw.
Deliberately duplicated (not imported) so the production worker never depends
on test fixtures.
"""

_ASK_FACTOR_BASE = 1_000_000
_ASK_FACTOR_BPS = 100


def slipped_open_raw(open_raw4: int, side: str, slippage_bps: int) -> int:
    factor = (
        _ASK_FACTOR_BASE + slippage_bps * _ASK_FACTOR_BPS
        if side == "BUY"
        else _ASK_FACTOR_BASE - slippage_bps * _ASK_FACTOR_BPS
    )
    return int(round(open_raw4 * factor / _ASK_FACTOR_BASE))


def quote_raw_for_open(open_raw4: int, slippage_bps: int) -> tuple[int, int]:
    return (
        slipped_open_raw(open_raw4, "SELL", slippage_bps),
        slipped_open_raw(open_raw4, "BUY", slippage_bps),
    )
