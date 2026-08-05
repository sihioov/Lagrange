"""Todo 13 red tests: the Lagrange-owned custom data class contract.

These tests pin the documented class surface (plan Todo 13, design ADR-004
section 9.2) BEFORE the implementation exists.  SessionOpenEvent must carry
exactly (instrument_id, trading_date, session_open_ts, open_price, currency,
data_version); DailyBarClosedEvent carries the documented OHLCV/adjustment
fields.  Both are registered PyO3 catalog data classes with ts_event/ts_init.
They are LAGRANGE-OWNED: they must not be NautilusTrader built-ins
(is_nautilus_class == False), and SessionOpenEvent must structurally expose
no same-day high/low/close (the future-field barrier).
"""
import pytest


def test_session_open_event_fields(events):
    SessionOpenEvent = events.SessionOpenEvent
    inst = SessionOpenEvent(
        instrument_id=events.InstrumentId.from_str("069500.KRX"),
        trading_date="2020-01-31",
        session_open_ts=1577836800000000000,
        open_price=102700000,
        currency="KRW",
        data_version="1",
        ts_event=1577836800000000000,
        ts_init=1577836800000000000,
    )
    d = inst.to_dict()
    assert d["instrument_id"] == "069500.KRX"
    assert d["trading_date"] == "2020-01-31"
    assert d["session_open_ts"] == 1577836800000000000
    assert d["open_price"] == 102700000
    assert d["currency"] == "KRW"
    assert d["data_version"] == "1"
    assert d["ts_event"] == 1577836800000000000
    assert d["ts_init"] == 1577836800000000000
    # Serialization contract: type tag + from_dict round trip.
    assert d["type"] == "SessionOpenEvent"
    assert SessionOpenEvent.from_dict(d) == inst


def test_daily_bar_closed_event_documented_fields(events):
    DailyBarClosedEvent = events.DailyBarClosedEvent
    inst = DailyBarClosedEvent(
        instrument_id=events.InstrumentId.from_str("069500.KRX"),
        trading_date="2020-01-31",
        session_close_ts=1577860200000000000,
        open=102700000,
        high=103400000,
        low=102300000,
        close=103000000,
        volume=1220000,
        adjustment_factor=100000000,
        currency="KRW",
        data_version="1",
        ts_event=1577860200000000000,
        ts_init=1577860200000000000,
    )
    d = inst.to_dict()
    # Documented OHLCV/adjustment fields (design section 9.2).
    for key in ("open", "high", "low", "close", "volume", "adjustment_factor"):
        assert key in d, key
    assert d["type"] == "DailyBarClosedEvent"
    assert DailyBarClosedEvent.from_dict(d) == inst


def test_open_event_has_no_future_fields(events):
    """At open time no same-day high/low/close exists in the event API."""
    SessionOpenEvent = events.SessionOpenEvent
    inst = SessionOpenEvent(
        instrument_id=events.InstrumentId.from_str("069500.KRX"),
        trading_date="2020-02-03",
        session_open_ts=1580688000000000000,
        open_price=103000000,
        currency="KRW",
        data_version="1",
        ts_event=1580688000000000000,
        ts_init=1580688000000000000,
    )
    for future_field in ("high", "low", "close"):
        assert not hasattr(inst, future_field), (
            f"SessionOpenEvent must not expose {future_field!r}"
        )
        with pytest.raises(AttributeError):
            getattr(inst, future_field)
        assert future_field not in inst.to_dict()


def test_events_are_data_subclasses_with_ts(events):
    import nautilus_trader.core.data as cd

    assert issubclass(events.SessionOpenEvent, cd.Data)
    assert issubclass(events.DailyBarClosedEvent, cd.Data)
    assert events.SessionOpenEvent.type_name_static() == "SessionOpenEvent"
    assert events.DailyBarClosedEvent.type_name_static() == "DailyBarClosedEvent"


def test_not_nautilus_builtins(events):
    """These are Lagrange-owned classes, NOT NautilusTrader built-ins."""
    from nautilus_trader.core.inspect import is_nautilus_class

    assert not is_nautilus_class(events.SessionOpenEvent)
    assert not is_nautilus_class(events.DailyBarClosedEvent)
    assert events.SessionOpenEvent.__module__.startswith("custom-data")
    assert events.DailyBarClosedEvent.__module__.startswith("custom-data")


def test_registered_with_rust_backend_and_arrow_roundtrip(events):
    """Classes are registered PyO3 catalog classes: to_arrow/from_arrow work."""
    SessionOpenEvent = events.SessionOpenEvent
    inst = SessionOpenEvent(
        instrument_id=events.InstrumentId.from_str("069500.KRX"),
        trading_date="2020-01-31",
        session_open_ts=1577836800000000000,
        open_price=102700000,
        currency="KRW",
        data_version="1",
        ts_event=1577836800000000000,
        ts_init=1577836800000000000,
    )
    batch = inst.to_arrow()
    assert batch.schema.field("ts_event").type == pa_int64()
    assert batch.num_rows == 1
    restored = SessionOpenEvent.from_arrow(batch.to_table())
    assert restored == [inst]


def pa_int64():
    import pyarrow as pa

    return pa.int64()


def test_prices_are_fixed_point_scale4_ints(events):
    """Price fields are int64 fixed-point at scale 4 (KRW/10^4), no floats."""
    assert events.PRICE_SCALE == 4
    assert events.FACTOR_SCALE == 8
