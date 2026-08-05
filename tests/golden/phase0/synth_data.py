"""Deterministic 260-session synthetic daily series for the three seed ETFs.

Data-span decision (plan Todo 14, documented in evidence): the Todo 6 golden
fixture spans 9 sessions (2020-01-20..2020-02-03) - too short for a genuine
200-session moving average, and a warmup-only run would emit ZERO orders,
making the AT-02 next-open fill proof vacuous.  This generator therefore
extends the fixture pattern (option (a) of the plan): the first 9 sessions
are EXACTLY the committed Todo 6 fixture bars, and the remaining sessions are
a deterministic, seeded (42) geometric random walk per instrument with a
documented drift program that GUARANTEES both MA-200 crossovers:

- sessions 1..200: mild negative drift (close stays below MA-200; warmup)
- sessions 201..230: strong positive drift (+2%/day) -> close crosses ABOVE
  the 200-session MA -> LONG signal, executed at the next session open
- sessions 231..260: strong negative drift (-2.5%/day) -> close crosses BELOW
  the 200-session MA -> FLAT/SELL signal

The generator self-checks its contract on every call (deterministic, so it
always passes or always fails): both crossovers must exist and the first nine
sessions must byte-match the committed fixture.

All prices are integer KRW (fixture contract, scale 0); curated rows add the
documented scale-4 fixed-point values and UTC microsecond session instants
(open = session date 00:00:00Z, close = 06:30:00Z - KRX Asia/Seoul +09:00,
no DST).  The random walk uses `random.Random(seed)` + Box-Muller normal
draws, both stable across CPython versions, so the series is reproducible on
any machine.
"""
from __future__ import annotations

import json
import math
import random
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_BARS = REPO_ROOT / "tests" / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"

GENERATOR_VERSION = "1.0.0"
SEED = 42
SESSIONS = 260
DATA_VERSION = "kr-etf-daily-phase0-v1"
MA_PERIOD = 200
PHASE_B_START = 200  # session index (0-based) where the uptrend begins
PHASE_B_SESSIONS = 30
PHASE_C_DRIFT = -0.025
PRICE_SCALE = 4

INSTRUMENTS = ("069500.KRX", "229200.KRX", "114260.KRX")

KRX_HOLIDAYS_2020 = frozenset({
    date(2020, 1, 24), date(2020, 1, 27),  # Seollal (fixture-verified break)
    date(2020, 3, 1),                      # Samiljeol
    date(2020, 4, 30),                     # Buddha's Birthday
    date(2020, 5, 1),                      # Labor Day
    date(2020, 5, 5),                      # Children's Day
    date(2020, 6, 6),                      # Memorial Day
    date(2020, 8, 15),                     # Liberation Day
    date(2020, 9, 30), date(2020, 10, 1), date(2020, 10, 2),  # Chuseok
    date(2020, 10, 3),                     # National Foundation Day
    date(2020, 10, 9),                     # Hangul Day
    date(2020, 12, 25),                    # Christmas
})

_SESSION_OPEN = timezone.utc
_CLOSE_DELTA = timedelta(hours=6, minutes=30)

_FIXTURE_BY_INSTRUMENT: dict[str, list[dict]] = {}
_FIXTURE_DOC = json.loads(FIXTURE_BARS.read_text(encoding="utf-8"))
for _bar in _FIXTURE_DOC["bars"]:
    _FIXTURE_BY_INSTRUMENT.setdefault(_bar["instrument"], []).append(_bar)

_FIXTURE_LAST_CLOSE = {
    iid: int(bars[-1]["close"]) for iid, bars in _FIXTURE_BY_INSTRUMENT.items()
}
_FIXTURE_BASE_VOLUME = {
    iid: int(bars[0]["volume"]) for iid, bars in _FIXTURE_BY_INSTRUMENT.items()
}
_FIXTURE_SESSION_DATES = tuple(_bar["date"] for _bar in _FIXTURE_BY_INSTRUMENT[INSTRUMENTS[0]])


def session_dates(start: date = date(2020, 1, 20), count: int = SESSIONS) -> list[str]:
    """KRX session dates: weekdays minus the documented 2020 holiday set."""
    out: list[str] = []
    cursor = start
    while len(out) < count:
        if cursor.weekday() < 5 and cursor not in KRX_HOLIDAYS_2020:
            out.append(cursor.isoformat())
        cursor += timedelta(days=1)
    return out


def _normal(rng: random.Random) -> float:
    """Standard normal draw (Box-Muller, two uniforms; deterministic)."""
    u1 = rng.random()
    u2 = rng.random()
    return (-2.0 * math.log(u1)) ** 0.5 * math.cos(2.0 * math.pi * u2)


def _walk_close(rng: random.Random, start: int, count: int, drift: float, sigma: float) -> list[int]:
    closes: list[int] = []
    price = float(start)
    for _ in range(count):
        price *= 1.0 + drift + sigma * _normal(rng)
        closes.append(int(round(price)))
    return closes


def generate_closes() -> dict[str, list[int]]:
    """Deterministic full close sequence per instrument (260 sessions, seed 42).

    The first 9 values are the committed fixture closes; the remaining 251
    are the seeded walk (warmup continuation, guaranteed up-cross, then
    down-cross).
    """
    out: dict[str, list[int]] = {}
    for idx, instrument in enumerate(INSTRUMENTS):
        rng = random.Random(SEED + idx * 7)
        fixture = _FIXTURE_BY_INSTRUMENT[instrument]
        last = int(fixture[-1]["close"])
        fixture_len = len(fixture)
        phase_a_len = PHASE_B_START - fixture_len  # 191: warmup continuation
        phase_a = _walk_close(rng, last, phase_a_len, drift=-0.0005, sigma=0.008)
        phase_b = _walk_close(rng, phase_a[-1], PHASE_B_SESSIONS, drift=0.02, sigma=0.008)
        remaining = SESSIONS - fixture_len - phase_a_len - PHASE_B_SESSIONS  # 30
        phase_c = _walk_close(rng, phase_b[-1], remaining, drift=PHASE_C_DRIFT, sigma=0.012)
        out[instrument] = [int(b["close"]) for b in fixture] + phase_a + phase_b + phase_c
    return out


def generate_bars() -> list[dict]:
    """Deterministic bars (scale-0 integer KRW, fixture contract).

    The first 9 sessions are the committed fixture bars verbatim; the walk
    continues from the fixture's last close.  Self-checks the MA-200 contract.
    """
    dates = session_dates()
    closes = generate_closes()
    bars: list[dict] = []
    for instrument in INSTRUMENTS:
        fixture = list(_FIXTURE_BY_INSTRUMENT[instrument])
        for bar in fixture:
            bars.append(dict(bar))
        rng = random.Random(SEED + INSTRUMENTS.index(instrument) * 11 + 3)
        base_volume = _FIXTURE_BASE_VOLUME[instrument]
        prev_close = closes[instrument][len(fixture) - 1]
        for i, session_date in enumerate(dates):
            if i < len(fixture):
                continue
            close = closes[instrument][i]
            open_ = prev_close
            spread = 0.004 + 0.006 * rng.random()
            high = int(round(max(open_, close) * (1.0 + spread)))
            low = int(round(min(open_, close) * (1.0 - spread)))
            volume = int(round(base_volume * (1.0 + 0.3 * rng.random())))
            bars.append({
                "instrument": instrument,
                "date": session_date,
                "open": open_,
                "high": high,
                "low": low,
                "close": close,
                "volume": volume,
                "value": volume * close,
            })
            prev_close = close
    assert_phase0_contract(bars, closes)
    return bars


def compute_ma200_signals(closes: list[int]) -> list[tuple[int, float, str]]:
    """(close, ma200, action) per session once 200 closes exist.

    Same rule as the strategy: action LONG when close > mean(last 200 closes)
    and previously flat; FLAT when close < mean and previously long.
    """
    signals: list[tuple[int, float, str]] = []
    holding = False
    for i in range(len(closes)):
        if i + 1 < MA_PERIOD:
            continue
        ma200 = sum(closes[i + 1 - MA_PERIOD : i + 1]) / MA_PERIOD
        close = closes[i]
        if close > ma200 and not holding:
            holding = True
            signals.append((close, ma200, "LONG"))
        elif close < ma200 and holding:
            holding = False
            signals.append((close, ma200, "FLAT"))
    return signals


def assert_phase0_contract(bars: list[dict], closes: dict[str, list[int]]) -> None:
    """Hard self-check: fixture anchor + both crossovers + OHLC invariants."""
    fixture_dates = list(_FIXTURE_SESSION_DATES)
    for instrument in INSTRUMENTS:
        series = [b for b in bars if b["instrument"] == instrument]
        assert len(series) == SESSIONS, f"{instrument}: {len(series)} != {SESSIONS} sessions"
        for expected, actual in zip(fixture_dates, series[: len(fixture_dates)]):
            assert actual["date"] == expected
            fixture_bar = next(b for b in _FIXTURE_BY_INSTRUMENT[instrument] if b["date"] == expected)
            for key in ("open", "high", "low", "close", "volume", "value"):
                assert actual[key] == fixture_bar[key], f"{instrument} {expected} {key} drifted"
        for bar in series:
            assert bar["high"] >= max(bar["open"], bar["close"]), f"{instrument} {bar['date']} high"
            assert bar["low"] <= min(bar["open"], bar["close"]), f"{instrument} {bar['date']} low"
            assert bar["open"] > 0 and bar["close"] > 0 and bar["volume"] >= 0
        signals = compute_ma200_signals(closes[instrument])
        assert any(action == "LONG" for _, _, action in signals), f"{instrument}: no LONG crossover"
        assert any(action == "FLAT" for _, _, action in signals), f"{instrument}: no FLAT crossover"
        assert signals[0][2] == "LONG", f"{instrument}: first signal must be LONG"


def slipped_open_raw(open_raw4: int, side: str, slippage_bps: int) -> int:
    """AT-02 expectation: raw (scale-4) fill price for a side.

    BUY fills at the ask = raw open + slippage bps; SELL at the bid = raw
    open - slippage bps.  Rounding: Python round (half-even) on integer raw.
    This is the single source of truth for the slippage quantization used by
    the quote generator, the strategy fill records, and the AT-02 gate test.
    """
    factor = 1_000_000 + slippage_bps * 100 if side == "BUY" else 1_000_000 - slippage_bps * 100
    return int(round(open_raw4 * factor / 1_000_000))


def quote_raw_for_open(open_raw4: int, slippage_bps: int) -> tuple[int, int]:
    """(bid_raw4, ask_raw4) quote for a session open (symmetric slippage)."""
    return (
        slipped_open_raw(open_raw4, "SELL", slippage_bps),
        slipped_open_raw(open_raw4, "BUY", slippage_bps),
    )


def generate_curated_rows() -> list[dict]:
    """Curated-zone rows (documented schema) for the full 260-session series.

    Prices scale-4 fixed-point; instants UTC microseconds; deterministic
    provenance fields (zero batch id / hash, ingested 2020-02-10).
    """
    rows: list[dict] = []
    for bar in generate_bars():
        open_dt = datetime.fromisoformat(bar["date"]).replace(tzinfo=_SESSION_OPEN)
        close_dt = open_dt + _CLOSE_DELTA
        rows.append({
            "instrument_id": bar["instrument"],
            "trading_date": bar["date"],
            "market_open_ts": int(open_dt.timestamp() * 1_000_000),
            "market_close_ts": int(close_dt.timestamp() * 1_000_000),
            "open": int(bar["open"]) * 10_000,
            "high": int(bar["high"]) * 10_000,
            "low": int(bar["low"]) * 10_000,
            "close": int(bar["close"]) * 10_000,
            "volume": int(bar["volume"]),
            "trading_value": int(bar["value"]),
            "currency": "KRW",
            "source": "synthetic",
            "ingested_at": int(datetime(2020, 2, 10, tzinfo=timezone.utc).timestamp() * 1_000_000),
            "batch_id": "00000000-0000-0000-0000-000000000000",
            "raw_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        })
    return rows
