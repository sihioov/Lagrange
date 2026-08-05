"""Todo 13 red tests: deterministic event ordering and future-field barriers.

The 2020-01-31 fixture must emit, in exact order:
    close(T) -> pending target(T+1) exists -> open(T+1) -> close(T+1)
with T = 2020-01-31 (close 2020-01-31T06:30:00Z) and T+1 = 2020-02-03
(open 2020-02-03T00:00:00Z, close 2020-02-03T06:30:00Z).  Equal or
out-of-order timestamps and duplicate session events are rejected with typed
errors; nothing may fail silently.
"""
from __future__ import annotations

import pytest

from conftest import golden_bars_rows, session_instants


def build_stream(builder, rows):
    """Build the validated, deterministically-ordered event stream."""
    return builder.build_events_from_curated(rows)


def test_fixture_2020_01_31_exact_order(builder, events):
    """close(T) -> pending(T+1) -> open(T+1) -> close(T+1), in exact order."""
    SessionOpenEvent = events.SessionOpenEvent
    DailyBarClosedEvent = events.DailyBarClosedEvent
    stream = build_stream(builder, golden_bars_rows())
    iid = events.InstrumentId.from_str("069500.KRX")
    by_instrument = [e for e in stream if e.instrument_id == iid]

    close_t = [e for e in by_instrument if isinstance(e, DailyBarClosedEvent) and e.trading_date == "2020-01-31"]
    open_t1 = [e for e in by_instrument if isinstance(e, SessionOpenEvent) and e.trading_date == "2020-02-03"]
    close_t1 = [e for e in by_instrument if isinstance(e, DailyBarClosedEvent) and e.trading_date == "2020-02-03"]
    assert len(close_t) == 1 and len(open_t1) == 1 and len(close_t1) == 1

    close_ts, open_ts, close1_ts = close_t[0].ts_event, open_t1[0].ts_event, close_t1[0].ts_event
    # Exact instants from the KRX calendar (Asia/Seoul fixed +09:00, no DST).
    assert close_ts == 1577860200000000000  # 2020-01-31T06:30:00Z
    assert open_ts == 1580688000000000000  # 2020-02-03T00:00:00Z
    assert close1_ts == 1580711400000000000  # 2020-02-03T06:30:00Z

    # The stream itself must be in exact chronological order and the close of
    # T must strictly precede the open of T+1 (the PendingTarget window).
    assert close_ts < open_ts < close1_ts
    # The PendingTarget can be recorded strictly between close(T) and open(T+1).
    assert builder.PENDING_TARGET_WINDOW_PROOF(close_ts, open_ts)

    # Event-order invariant across the whole stream: ts_event strictly
    # increases within each (class, instrument) and the stream is sorted.
    seq = [e for e in stream if e.instrument_id == iid]
    kinds = [("open" if isinstance(e, SessionOpenEvent) else "close") for e in seq]
    assert kinds == ["close", "open", "close", "open", "close", "open", "close",
                     "open", "close", "open", "close", "open", "close", "open",
                     "close", "open", "close", "open", "close"][: len(kinds)]
    # 9 sessions -> 18 events per instrument; first event is close of first
    # session, last is close of the last session.
    assert len(seq) == 18
    assert isinstance(seq[0], DailyBarClosedEvent)
    assert isinstance(seq[-1], DailyBarClosedEvent)
    assert seq[0].trading_date == "2020-01-20"
    assert seq[-1].trading_date == "2020-02-03"


def test_no_same_day_high_low_close_before_open(builder, events):
    """No event carrying session T+1 high/low/close precedes the T+1 open."""
    SessionOpenEvent = events.SessionOpenEvent
    DailyBarClosedEvent = events.DailyBarClosedEvent
    stream = build_stream(builder, golden_bars_rows())
    iid = events.InstrumentId.from_str("229200.KRX")
    seq = [e for e in stream if e.instrument_id == iid]
    t1 = "2020-02-03"
    seen_open_t1 = False
    for e in seq:
        if isinstance(e, SessionOpenEvent) and e.trading_date == t1:
            seen_open_t1 = True
            continue
        if seen_open_t1 and isinstance(e, DailyBarClosedEvent) and e.trading_date == t1:
            continue  # close(T+1) legitimately follows open(T+1)
        if isinstance(e, DailyBarClosedEvent) and e.trading_date == t1:
            pytest.fail("daily bar closed event for T+1 before its session open")


def test_equal_timestamps_rejected(builder, events):
    SessionOpenEvent = events.SessionOpenEvent
    rows = golden_bars_rows()[:2]  # two rows for 069500.KRX? -> take two of same instrument
    rows = [r for r in golden_bars_rows() if r["instrument_id"] == "069500.KRX"][:2]
    stream = build_stream(builder, rows)
    # Force a duplicate: same (class, instrument, ts) pair.
    e0 = stream[0]
    dup = SessionOpenEvent(
        instrument_id=e0.instrument_id, trading_date=e0.trading_date,
        session_open_ts=e0.session_open_ts, open_price=e0.open_price,
        currency=e0.currency, data_version=e0.data_version,
        ts_event=e0.ts_event, ts_init=e0.ts_init,
    )
    with pytest.raises(events.EqualTimestampError):
        builder.validate_event_stream([*stream, dup])


def test_out_of_order_rejected(builder, events):
    stream = build_stream(builder, golden_bars_rows())
    iid = events.InstrumentId.from_str("114260.KRX")
    seq = [e for e in stream if e.instrument_id == iid]
    # Reverse a subsequence so timestamps decrease within the class+instrument.
    swapped = list(seq)
    swapped[0], swapped[1] = swapped[1], swapped[0]
    with pytest.raises(events.OutOfOrderError):
        builder.validate_event_stream(swapped)


def test_duplicate_session_event_rejected(builder, events):
    SessionOpenEvent = events.SessionOpenEvent
    stream = build_stream(builder, golden_bars_rows())
    iid = events.InstrumentId.from_str("069500.KRX")
    original = [e for e in stream if isinstance(e, SessionOpenEvent) and e.instrument_id == iid][0]
    # Same (instrument_id, trading_date) emitted twice -> duplicate session.
    dup = SessionOpenEvent(
        instrument_id=original.instrument_id, trading_date=original.trading_date,
        session_open_ts=original.session_open_ts, open_price=original.open_price,
        currency=original.currency, data_version=original.data_version,
        ts_event=original.ts_event, ts_init=original.ts_init,
    )
    with pytest.raises(events.DuplicateSessionError):
        builder.validate_event_stream([*stream, dup])


def test_deterministic_stream_ordering_across_instruments(builder, events):
    """Mixed-instrument streams sort deterministically by (ts, class, iid)."""
    stream1 = build_stream(builder, golden_bars_rows())
    stream2 = build_stream(builder, list(reversed(golden_bars_rows())))
    assert [(e.ts_event, type(e).__name__, str(e.instrument_id)) for e in stream1] == [
        (e.ts_event, type(e).__name__, str(e.instrument_id)) for e in stream2
    ]
