"""MA200Trend - versioned 200-session moving-average next-open strategy.

Phase 0 golden strategy (plan Todo 14).  Consumes the Lagrange-owned
`SessionOpenEvent` / `DailyBarClosedEvent` custom data (Todo 13, ADR-004):

- On `DailyBarClosedEvent(T)` the strategy appends the session close to its
  per-instrument history.  Once 200 closes exist, the 200-session simple
  moving average is computed and a signal is formed: close > MA200 -> LONG
  target, close < MA200 -> FLAT target (exit).  No order is ever placed at
  signal time: the decision is recorded as a pending target.
- On `SessionOpenEvent(T+1)` a pending target is executed with a MARKET
  order, so the fill lands at the T+1 session open quote (raw open plus the
  configured slippage bps on the buy side / minus it on the sell side, see
  the phase0 fill model note in fills.json).
- The future-field barrier holds by construction: `SessionOpenEvent` carries
  only the session open price; there is no same-day high/low/close to read.
  `probe_future_fields=True` deliberately attempts to read `.high/.low/.close`
  and records a typed `FUTURE_FIELD_VIOLATION` (gate-failure probe).

Versioning: `STRATEGY_VERSION` is the immutable semantic version of this
strategy definition; the golden manifest pins it under `versions.strategy`.
"""

import importlib
from collections import deque

from nautilus_trader.core.datetime import unix_nanos_to_dt
from nautilus_trader.model.data import DataType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import ClientId, InstrumentId
from nautilus_trader.model.objects import Quantity
from nautilus_trader.trading.config import StrategyConfig
from nautilus_trader.trading.strategy import Strategy

import msgspec

from strategies._costs import fees_for

STRATEGY_ID = "ma200-trend"
STRATEGY_VERSION = "1.0.0"
MA_PERIOD = 200
TARGET_WEIGHT = 1 / 3
PRICE_SCALE = 4

_session = importlib.import_module("custom-data.session_events")
SessionOpenEvent = _session.SessionOpenEvent
DailyBarClosedEvent = _session.DailyBarClosedEvent

__all__ = ["MA200TrendConfig", "MA200Trend", "STRATEGY_ID", "STRATEGY_VERSION"]


class MA200TrendConfig(StrategyConfig, frozen=True):
    instrument_ids: list[str] = (
        "069500.KRX", "229200.KRX", "114260.KRX",
    )
    ma_period: int = MA_PERIOD
    slippage_bps: int = 10
    lot_size: int = 100
    initial_cash: str = "100000000"
    strategy_version: str = STRATEGY_VERSION
    probe_future_fields: bool = False
    #: The versioned cost profile a fill is charged under, supplied by the
    #: backtest runner. Empty by default, which is what the phase-0 golden
    #: run uses: it constructs this config directly and models no costs, so
    #: its pinned artifact hashes are unaffected by this field existing.
    cost_profile: dict = msgspec.field(default_factory=dict)


class MA200Trend(Strategy):
    """200-session moving-average trend strategy executing at next open."""

    def __init__(self, config: MA200TrendConfig):
        super().__init__(config)
        self.instrument_ids = list(config.instrument_ids)
        self.ma_period = int(config.ma_period)
        self.slippage_bps = int(config.slippage_bps)
        self.lot_size = int(config.lot_size)
        self.initial_cash = int(config.initial_cash)
        self.strategy_version = config.strategy_version
        self.probe_future_fields = bool(config.probe_future_fields)
        self.cost_profile = dict(getattr(config, "cost_profile", {}) or {})

        self.cash = self.initial_cash
        self.closes: dict[str, deque] = {}
        self.positions: dict[str, int] = {}
        self.position_ids: dict[str, object] = {}
        self.pending: dict[str, str] = {}
        self.last_close_date: dict[str, str] = {}
        self.recommendations: list[dict] = []
        self.orders: list[dict] = []
        self.fills: list[dict] = []
        self.equity_points: list[dict] = []
        self.violations: list[str] = []
        self.barrier_held = True
        for instrument_id in self.instrument_ids:
            self.closes[instrument_id] = deque(maxlen=None)
            self.positions[instrument_id] = 0
            self.position_ids[instrument_id] = None
            self.pending[instrument_id] = "FLAT"
            self.last_close_date[instrument_id] = None

    # -- lifecycle ---------------------------------------------------------

    def on_start(self) -> None:
        client_id = ClientId("CUSTOM")
        for instrument_id in self.instrument_ids:
            iid = InstrumentId.from_str(instrument_id)
            self.subscribe_data(DataType(SessionOpenEvent), client_id=client_id, instrument_id=iid)
            self.subscribe_data(DataType(DailyBarClosedEvent), client_id=client_id, instrument_id=iid)

    # -- data --------------------------------------------------------------

    def on_data(self, data) -> None:
        inner = getattr(data, "data", data)
        if type(inner).__name__ == "SessionOpenEvent":
            self._on_session_open(inner)
        elif type(inner).__name__ == "DailyBarClosedEvent":
            self._on_session_close(inner)

    # -- signals -----------------------------------------------------------

    def _on_session_close(self, event) -> None:
        instrument_id = str(event.instrument_id)
        self.last_close_date[instrument_id] = event.trading_date
        self.closes[instrument_id].append(int(event.close))
        self._record_equity_point(event)
        if self.probe_future_fields:
            self._probe_event_fields(event, "DailyBarClosedEvent")
            return
        recent = list(self.closes[instrument_id])[-self.ma_period:]
        if len(recent) < self.ma_period:
            self.pending[instrument_id] = "FLAT"
            return
        ma200 = sum(recent) / self.ma_period
        close = self.closes[instrument_id][-1]
        holding = self.positions[instrument_id] > 0
        if close > ma200 and not holding:
            self.pending[instrument_id] = "LONG"
            self.recommendations.append({
                "signal_date": event.trading_date,
                "effective_date": None,
                "instrument": instrument_id,
                "action": "BUY",
                "close": int(event.close),
                "ma200_raw": int(round(ma200)),
                "reason_codes": ["MA200_TREND_POSITIVE"],
                "warmup": False,
            })
        elif close < ma200 and holding:
            self.pending[instrument_id] = "FLAT"
            self.recommendations.append({
                "signal_date": event.trading_date,
                "effective_date": None,
                "instrument": instrument_id,
                "action": "SELL",
                "close": int(event.close),
                "ma200_raw": int(round(ma200)),
                "reason_codes": ["MA200_TREND_NEGATIVE"],
                "warmup": False,
            })

    def _on_session_open(self, event) -> None:
        instrument_id = str(event.instrument_id)
        for future_field in ("high", "low", "close"):
            if hasattr(event, future_field):
                self._violate(f"FUTURE_FIELD_VIOLATION: open event exposes {future_field}")
        if self.probe_future_fields:
            self._probe_event_fields(event, "SessionOpenEvent")
            return
        target = self.pending[instrument_id]
        signal_date = None
        for rec in self.recommendations:
            if rec["instrument"] == instrument_id and rec["effective_date"] is None:
                signal_date = rec["signal_date"]
                rec["effective_date"] = event.trading_date
                break
        if target == "LONG" and self.positions[instrument_id] == 0:
            quantity = self._target_quantity(instrument_id, event.open_price)
            if quantity > 0:
                self._submit(instrument_id, OrderSide.BUY, quantity, event, signal_date)
        elif target == "FLAT" and self.positions[instrument_id] > 0:
            self._submit(instrument_id, OrderSide.SELL, self.positions[instrument_id], event, signal_date)

    # -- execution ---------------------------------------------------------

    def _target_quantity(self, instrument_id: str, open_raw: int) -> int:
        open_price = open_raw / 10_000
        notional = self.cash * TARGET_WEIGHT
        shares = int(notional / open_price)
        return (shares // self.lot_size) * self.lot_size

    def _submit(self, instrument_id: str, side, quantity: int, event, signal_date: str | None) -> None:
        order = self.order_factory.market(
            InstrumentId.from_str(instrument_id),
            side,
            Quantity.from_int(quantity),
            reduce_only=(side == OrderSide.SELL),
        )
        self.orders.append({
            "order_id": None,
            "client_order_id": None,
            "instrument": instrument_id,
            "side": side.name,
            "quantity": quantity,
            "order_type": "MARKET",
            "signal_date": signal_date,
            "created_date": event.trading_date,
            "state": "SUBMITTED",
        })
        self.submit_order(
            order,
            position_id=self.position_ids.get(instrument_id) if side == OrderSide.SELL else None,
        )

    def on_order_submitted(self, event) -> None:
        for order in self.orders:
            if order["client_order_id"] is None:
                order["client_order_id"] = event.client_order_id.value
                order["state"] = "SUBMITTED"
                break

    def on_order_filled(self, event) -> None:
        instrument_id = str(event.instrument_id)
        qty = int(event.last_qty.as_double())
        price_raw = int(round(event.last_px.as_double() * 10_000))
        ts = unix_nanos_to_dt(event.ts_event)
        commission_raw, tax_raw = fees_for(
            self.cost_profile, event.order_side != OrderSide.BUY, qty, price_raw
        )
        if event.order_side == OrderSide.BUY:
            self.cash -= qty * price_raw / 10_000
            self.positions[instrument_id] += qty
        else:
            self.cash += qty * price_raw / 10_000
            self.positions[instrument_id] -= qty
        # A cash DEBIT on both sides: fees are paid, not netted. Zero when no
        # profile was supplied, which is the phase-0 golden run -- it builds
        # this config directly and models no costs, so its pinned hashes hold.
        self.cash -= (commission_raw + tax_raw) / 10_000
        self.position_ids[instrument_id] = event.position_id
        self.fills.append({
            "fill_id": f"fill-{instrument_id}-{ts.date().isoformat()}",
            "order_id": f"ord-{instrument_id}-{ts.date().isoformat()}",
            "client_order_id": event.client_order_id.value,
            "instrument": instrument_id,
            "side": event.order_side.name,
            "quantity": qty,
            "price_raw": price_raw,
            "date": ts.date().isoformat(),
            "ts": ts.isoformat(),
            "source": "NEXT_SESSION_OPEN",
            "slippage_bps": self.slippage_bps,
            "commission_raw": commission_raw,
            "tax_raw": tax_raw,
            "never_uses": [
                "signal_day_close",
                "execution_day_high",
                "execution_day_low",
                "execution_day_close",
            ],
            "barrier_held": self.barrier_held,
        })
        for order in self.orders:
            if order["client_order_id"] == event.client_order_id.value:
                order["state"] = "FILLED"
                order["order_id"] = f"ord-{instrument_id}-{ts.date().isoformat()}"
                break

    def _record_equity_point(self, event) -> None:
        positions_value = 0
        for instrument_id in self.instrument_ids:
            if instrument_id == str(event.instrument_id):
                positions_value += self.positions[instrument_id] * int(event.close) / 10_000
            elif self.closes[instrument_id]:
                positions_value += self.positions[instrument_id] * self.closes[instrument_id][-1] / 10_000
        self.equity_points.append({
            "date": event.trading_date,
            "cash_raw": int(round(self.cash * 10_000)),
            "positions_value_raw": int(round(positions_value * 10_000)),
            "equity_raw": int(round((self.cash + positions_value) * 10_000)),
        })

    def _probe_event_fields(self, event, kind: str) -> None:
        """Deliberate future-field probe: attempting to read same-day
        high/low/close from an open event must fail the gate."""
        for field in ("high", "low", "close"):
            try:
                _ = getattr(event, field)
            except AttributeError:
                self.violations.append(
                    f"FUTURE_FIELD_VIOLATION: {kind}.{field} raised AttributeError (barrier held)"
                )
                continue
            self._violate(f"FUTURE_FIELD_VIOLATION: {kind} unexpectedly exposes {field}")

    def _violate(self, message: str) -> None:
        self.violations.append(message)
        self.barrier_held = False
        self.log.error(message)
        raise RuntimeError(message)

    def on_stop(self) -> None:
        for rec in self.recommendations:
            if rec["effective_date"] is None:
                rec["effective_date"] = self.last_close_date.get(rec["instrument"])
