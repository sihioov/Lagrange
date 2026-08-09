"""The requested backtest period is the period that gets simulated.

`POST /api/v1/backtests` has always required `start_date` and `end_date`, and
has always put them in the job payload. Nothing downstream read them. A run
requested for 2020-01-01..2020-06-30 covered whatever the dataset happened to
hold, and the stored row still said 2020-01-01..2020-06-30 -- a number that is
wrong while looking exactly like the number that was asked for.

These pin the filter itself. The properties that matter are the boundaries
(a window that names a bar's date includes that bar), the absent case (no
window means the whole dataset, unchanged), and the empty case (a window
outside the data fails loudly rather than reporting a flat curve).
"""
from __future__ import annotations

import sys
from pathlib import Path

import pytest

WORKER_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(WORKER_ROOT))

from backtest_worker.simulate import SimulateError, _apply_window  # noqa: E402


def _rows(*dates: str) -> list[dict]:
    return [{"instrument_id": "069500.KRX", "trading_date": d, "close": 10_000} for d in dates]


DATASET = _rows("2020-01-20", "2020-01-21", "2020-01-22", "2020-01-23", "2020-02-03")


def _dates(rows: list[dict]) -> list[str]:
    return [r["trading_date"] for r in rows]


def test_a_window_keeps_only_the_bars_inside_it():
    kept = _apply_window(DATASET, "2020-01-21", "2020-01-22")
    assert _dates(kept) == ["2020-01-21", "2020-01-22"]


def test_both_bounds_are_inclusive():
    """The whole dataset, addressed by its own first and last dates.

    An exclusive end would drop 2020-02-03 here and the last day of every
    window anyone ever writes -- the kind of off-by-one that survives because
    each individual result still looks plausible.
    """
    kept = _apply_window(DATASET, "2020-01-20", "2020-02-03")
    assert _dates(kept) == _dates(DATASET)


def test_a_bound_that_falls_between_trading_days_keeps_the_days_that_exist():
    """2020-01-24..2020-02-02 are weekend/holiday: no bars, and that is fine."""
    kept = _apply_window(DATASET, "2020-01-23", "2020-02-02")
    assert _dates(kept) == ["2020-01-23"]


def test_no_window_is_the_whole_dataset():
    assert _apply_window(DATASET, None, None) is DATASET


def test_a_window_outside_the_data_fails_and_says_what_exists():
    """Not a successful zero-bar run.

    Zero bars produces a flat equity curve and no fills, which is exactly what
    a strategy that decided not to trade produces. The two must not look the
    same, so the error names both the request and the coverage.
    """
    with pytest.raises(SimulateError) as excinfo:
        _apply_window(DATASET, "2021-01-01", "2021-12-31")
    message = str(excinfo.value)
    assert "2021-01-01" in message and "2021-12-31" in message
    assert "2020-01-20" in message and "2020-02-03" in message


def test_string_comparison_is_date_comparison():
    """The trap this project already fell into once.

    The ledger check in this same worker compared `'2020-01-21T00:00:00+00:00'`
    against `'2020-01-21'` and was False forever. Both sides here are
    zero-padded ISO dates from a parquet `date32`, so ordering by string is
    ordering by date -- including across a month boundary, where a naive
    numeric intuition ('0203 < 0121'?) would go wrong.
    """
    kept = _apply_window(DATASET, "2020-01-23", "2020-02-03")
    assert _dates(kept) == ["2020-01-23", "2020-02-03"]
