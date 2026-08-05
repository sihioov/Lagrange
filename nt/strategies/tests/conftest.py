"""Shared fixtures for the nt/strategies test suite (plan Todo 17).

The `strategies` package and the hyphenated `custom-data` package both live
under `nt/`, so `nt/` is put on ``sys.path`` and both are imported through
``importlib`` (hyphens cannot be imported with the ``import`` statement).
"""

import importlib
import sys
from datetime import datetime, timezone
from pathlib import Path

import pytest

NT_ROOT = Path(__file__).resolve().parents[2]
if str(NT_ROOT) not in sys.path:
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
    import json

    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


@pytest.fixture(scope="session")
def events():
    """The custom-data.session_events module (Todo 13 custom events)."""
    return importlib.import_module("custom-data.session_events")


@pytest.fixture(scope="session")
def registry_module():
    """The strategies._registry module."""
    return importlib.import_module("strategies._registry")


@pytest.fixture(scope="session")
def execution_module():
    """The strategies._execution module (adapter base)."""
    return importlib.import_module("strategies._execution")


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
