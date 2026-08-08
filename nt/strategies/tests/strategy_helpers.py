"""Shared helpers for the nt/strategies test suite (plan Todo 17).

Deliberately NOT in `conftest.py`, for the reason
`nt/backtest-worker/tests/helpers.py` already records: pytest puts each test
directory on ``sys.path`` and imports conftest by BASENAME, so every
`conftest.py` under `nt/` competes for the single module name `conftest`.
A test that writes ``from conftest import ...`` therefore gets whichever
directory pytest collected first.

That is not a hypothetical. Running `pytest nt/custom-data/tests` and
`pytest nt/strategies/tests` separately passed while running `pytest nt`
failed at IMPORT, because `nt/custom-data/tests/test_catalog_builder.py`
resolved `conftest` to this suite's file and could not find `bars_table`.
Two green commands, one red one, and the difference was collection order.

Fixtures stay in `conftest.py` — pytest resolves those by path, not by module
name, so they never collide. Only the plain helpers move here, where the
module name is unique across the tree.
"""

import importlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

NT_ROOT = Path(__file__).resolve().parents[2]
if str(NT_ROOT) not in sys.path:
    # `strategies` and the hyphenated `custom-data` package both live under
    # `nt/`; hyphens cannot be imported with the `import` statement, so both
    # are reached through importlib with `nt/` on the path.
    sys.path.insert(0, str(NT_ROOT))

#: The five baseline strategy ids in canonical order (mirrors the Rust
#: `selector::baseline::BASELINE_STRATEGY_IDS`).
STRATEGIES = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
]


def load_package(sid: str):
    return importlib.import_module(f"strategies.{sid}.package")


def load_target(sid: str):
    return importlib.import_module(f"strategies.{sid}.target")


def load_adapter(sid: str):
    return importlib.import_module(f"strategies.{sid}.adapter")


def load_golden(sid: str) -> dict:
    path = NT_ROOT / "strategies" / sid / "golden.json"
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def _micros_to_nanos(dt: datetime) -> int:
    return int(dt.timestamp() * 1_000_000_000)


def make_open(events, iid: str = "069500.KRX", date: str = "2020-02-03",
              open_raw: int = 92980000) -> object:
    """A real SessionOpenEvent (Todo 13) for the 2020-02-03 session."""
    from nautilus_trader.model.identifiers import InstrumentId

    ts = _micros_to_nanos(datetime(2020, 2, 3, 0, 0, 0, tzinfo=timezone.utc))
    return events.SessionOpenEvent(
        instrument_id=InstrumentId.from_str(iid),
        trading_date=date,
        session_open_ts=ts // 1000,
        open_price=open_raw,
        currency="KRW",
        data_version="v1",
    )


def make_close(events, iid: str = "069500.KRX", date: str = "2020-01-31",
               close_raw: int = 93500000) -> object:
    """A real DailyBarClosedEvent (Todo 13) for the 2020-01-31 session."""
    from nautilus_trader.model.identifiers import InstrumentId

    ts = _micros_to_nanos(datetime(2020, 1, 31, 6, 30, 0, tzinfo=timezone.utc))
    return events.DailyBarClosedEvent(
        instrument_id=InstrumentId.from_str(iid),
        trading_date=date,
        session_close_ts=ts // 1000,
        open=92980000,
        high=94000000,
        low=92500000,
        close=close_raw,
        volume=1000,
        adjustment_factor=100000000,
        currency="KRW",
        data_version="v1",
    )
