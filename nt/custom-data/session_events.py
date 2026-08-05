"""Lagrange-owned custom data classes for the NT catalog (plan Todo 13).

These are **Lagrange-owned** registered PyO3 catalog data classes, NOT
NautilusTrader built-ins.  They encode design ADR-004 (section 9.2): a daily
bar is split into a `SessionOpenEvent` (session open instant + open price
ONLY) and a `DailyBarClosedEvent` (session close instant + full OHLCV and the
cumulative adjustment factor), so a T-close signal can only execute at the
T+1 session open and can never read that session's high/low/close.

API surface relied on (NautilusTrader 1.231.0, verified against the installed
package and nautilustrader.io/docs/latest/concepts/data/):

- `nautilus_trader.model.custom.customdataclass_pyo3` (called WITH parens:
  bare usage double-applies the decorator and crashes with a TypeError in
  1.231.0) provides `ts_event`/`ts_init` properties, `to_dict`/`from_dict`,
  `to_bytes`/`from_bytes`, `to_arrow`/`from_arrow` and the pyo3 catalog
  encode/decode hooks.  Field types map to an Arrow schema: InstrumentId ->
  string, str/int/float/bool/bytes/dict -> native types.
- `nautilus_trader.core.nautilus_pyo3.model.register_custom_data_class`
  registers the class with the Rust catalog/engine backend.
- Subclassing `nautilus_trader.core.data.Data` makes the objects flow through
  `ParquetDataCatalog.write_data/query` and the backtest engine (the query
  path wraps `Data` instances in `CustomData`; `is_nautilus_class` stays
  False for Lagrange classes).
- `BacktestDataConfig(catalog_path=..., data_cls="custom-data.session_events:SessionOpenEvent",
  instrument_id=..., client_id=...)` - `client_id` is REQUIRED for non-Nautilus
  classes; `resolve_path` resolves the hyphenated module via importlib.
- Strategies subscribe with `subscribe_data(data_type=DataType(SessionOpenEvent),
  client_id=ClientId("CUSTOM"))` and receive the raw objects in `on_data`.

Prices are int64 fixed-point at scale 4 (KRW/10^4) exactly as stored by the
Curated zone (Decimal128(18,4)); adjustment factors are int64 fixed-point at
scale 8.  No floats and no Python DST machinery are involved: all instants
come from the Todo 9 KRX calendar (Asia/Seoul, fixed +09:00, explicit UTC).

Deterministic ordering: per (class, instrument) the stream must be strictly
increasing in `ts_event`; equal timestamps and out-of-order input are
rejected with typed errors, as are duplicate session-open events.
"""
from collections import defaultdict
from typing import Any, Iterable

import nautilus_trader.core.data as cd
from nautilus_trader.core.nautilus_pyo3.model import register_custom_data_class
from nautilus_trader.model.custom import customdataclass_pyo3
from nautilus_trader.model.identifiers import InstrumentId

__all__ = [
    "PRICE_SCALE",
    "FACTOR_SCALE",
    "SessionOpenEvent",
    "DailyBarClosedEvent",
    "EVENT_CLASSES",
    "EventStreamError",
    "EqualTimestampError",
    "OutOfOrderError",
    "DuplicateSessionError",
    "validate_event_stream",
    "sort_event_stream",
]

#: Fixed-point scale of price fields (KRW/10^4), matching curated Decimal128(18,4).
PRICE_SCALE = 4
#: Fixed-point scale of adjustment factors, matching curated Decimal128(18,8).
FACTOR_SCALE = 8


@customdataclass_pyo3()
class SessionOpenEvent(cd.Data):
    """A KRX session open (design ADR-004; plan Todo 13).

    Structurally carries ONLY the session open price - there are no
    high/low/close fields, so the future-field barrier holds by
    construction (requirements §9.1.3).
    """

    instrument_id: InstrumentId
    trading_date: str
    session_open_ts: int
    open_price: int
    currency: str
    data_version: str


@customdataclass_pyo3()
class DailyBarClosedEvent(cd.Data):
    """A KRX session close with the full documented OHLCV/adjustment fields
    (design section 9.2)."""

    instrument_id: InstrumentId
    trading_date: str
    session_close_ts: int
    open: int
    high: int
    low: int
    close: int
    volume: int
    adjustment_factor: int
    currency: str
    data_version: str


#: Canonical class ordering for deterministic mixed streams.
EVENT_CLASSES: tuple[type, ...] = (SessionOpenEvent, DailyBarClosedEvent)

register_custom_data_class(SessionOpenEvent)
register_custom_data_class(DailyBarClosedEvent)


class EventStreamError(Exception):
    """Base class for deterministic-ordering violations (never silent)."""


class EqualTimestampError(EventStreamError):
    """Two events of the same class+instrument share an equal ts_event."""


class OutOfOrderError(EventStreamError):
    """ts_event decreases within a class+instrument stream."""


class DuplicateSessionError(EventStreamError):
    """A session-open event for (instrument_id, trading_date) is emitted twice."""


def _class_rank(event: Any) -> int:
    for i, cls in enumerate(EVENT_CLASSES):
        if isinstance(event, cls):
            return i
    raise TypeError(f"not a Lagrange event class: {type(event).__name__}")


def sort_event_stream(events: Iterable[Any]) -> list[Any]:
    """Deterministic sort: (ts_event, class rank, instrument_id, trading_date).

    Cross-instrument ties (all instruments share a session's instants) break
    on instrument id, so mixed-instrument streams are fully deterministic.
    """
    return sorted(
        events,
        key=lambda e: (
            e.ts_event,
            _class_rank(e),
            str(e.instrument_id),
            e.trading_date,
        ),
    )


def validate_event_stream(events: Iterable[Any]) -> list[Any]:
    """Validates deterministic ordering and returns the sorted stream.

    Rules (all violations raise typed errors - never silent, never a silent
    reorder):

    1. A session-open event may appear only once per (instrument_id,
       trading_date) -> `DuplicateSessionError`.
    2. Within each instrument the input stream must already be strictly
       increasing in `ts_event` (equal -> `EqualTimestampError`,
       decreasing -> `OutOfOrderError`).
    3. The returned stream is deterministically sorted by
       `sort_event_stream` (cross-instrument ties break on instrument id).
    """
    events = list(events)
    seen_opens: dict[tuple[str, str], None] = {}
    by_instrument: dict[str, list[Any]] = defaultdict(list)
    for e in events:
        if isinstance(e, SessionOpenEvent):
            key = (str(e.instrument_id), e.trading_date)
            if key in seen_opens:
                raise DuplicateSessionError(
                    f"duplicate session open for instrument={key[0]} trading_date={key[1]}"
                )
            seen_opens[key] = None
        by_instrument[str(e.instrument_id)].append(e)

    for instrument_id, group in by_instrument.items():
        for prev, cur in zip(group, group[1:]):
            if prev.ts_event == cur.ts_event:
                raise EqualTimestampError(
                    f"equal ts_event {prev.ts_event} in {instrument_id} "
                    f"({type(prev).__name__}, {type(cur).__name__})"
                )
            if prev.ts_event > cur.ts_event:
                raise OutOfOrderError(
                    f"out-of-order ts_event {cur.ts_event} < {prev.ts_event} "
                    f"in {instrument_id}"
                )

    return sort_event_stream(events)
