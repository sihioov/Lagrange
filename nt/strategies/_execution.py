"""The engine-independent NT execution adapter base (FR-STR-005, design §8.3).

Target generators produce `TargetPortfolio`s (Todo 16); this adapter is the
strategy-side order boundary that executes them at the next session open.
It consumes the Lagrange-owned `SessionOpenEvent` / `DailyBarClosedEvent`
custom data (Todo 13): close events maintain per-instrument history, open
events execute pending targets.  Sells are planned before buys and are
reduce-only; integer lot sizing; pending targets are consumed once (a
T-close signal executes at the T+1 open exactly once).

Two modes, one class
--------------------
The adapter is used two ways and behaves differently in each, which is
deliberate rather than incidental.

**Unregistered** — constructed directly by a unit test, with no engine behind
it.  It records `order_intents` and nothing else: a decision log that can be
asserted against a golden fixture without a backtest.  This is what the Todo
17 suite exercises.

**Registered with a NautilusTrader engine** — the adapter additionally submits
REAL orders and records `orders`/`fills`/`equity_points`, which is what the
backtest worker collects.  It also drives its own rebalance, because inside a
backtest nobody else can: `set_target_portfolio` is called by an operator or a
test, and there is no operator in a simulation.

`order_factory` is `None` until NT registers the strategy, so the mode is read
from that rather than from a flag someone has to remember to set.

Why the split matters
---------------------
Before this, the adapter had ONLY the first mode.  Run through the worker it
placed no orders, took no fills, and moved no positions — and the worker
collects results with ``getattr(strategy, "orders", [])``, whose default
turned that into an empty list rather than an error.  A baseline backtest
therefore completed, reported SUCCEEDED, and produced artifacts with zero
orders: a result the user cannot tell apart from a strategy that decided not
to trade.

A strategy that needs factors nobody supplies now raises instead, for the same
reason: the failure a user can act on is the one that says what is missing.
"""

import importlib
from typing import Dict, List, Optional

from nautilus_trader.core.datetime import unix_nanos_to_dt
from nautilus_trader.model.data import DataType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import ClientId, InstrumentId
from nautilus_trader.model.objects import Quantity
from nautilus_trader.trading.strategy import Strategy

from strategies._costs import fees_for

_session = importlib.import_module("custom-data.session_events")
SessionOpenEvent = _session.SessionOpenEvent
DailyBarClosedEvent = _session.DailyBarClosedEvent

#: Prices are carried as integers scaled by 10_000 (design §6.3), so every
#: conversion to a real price goes through this rather than a literal.
PRICE_SCALE = 10_000


class AdapterError(Exception):
    """A typed adapter failure (never a panic)."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


class TargetExecutionStrategy(Strategy):
    """Executes externally-computed target portfolios at the next open."""

    STRATEGY_ID = "?"
    VERSION = "0.0.0"

    def __init__(self, config):
        super().__init__(config)
        self.instrument_ids = list(config.instrument_ids)
        self.parameters = dict(config.parameters)
        self.slippage_bps = int(config.slippage_bps)
        self.lot_size = int(config.lot_size)
        self.initial_cash = str(config.initial_cash)
        self.strategy_version = str(config.strategy_version)
        self.pending_targets: Dict[str, Optional[float]] = {}
        self.positions: Dict[str, int] = {}
        self.closes: Dict[str, List[int]] = {}
        self.order_intents: List[dict] = []
        # Collected by the backtest worker. Named to match what it reads --
        # it uses `getattr(strategy, name, [])`, so a rename here silently
        # empties an artifact instead of failing.
        self.orders: List[dict] = []
        self.fills: List[dict] = []
        self.equity_points: List[dict] = []
        self.cash: float = float(self.initial_cash)
        self.position_ids: Dict[str, object] = {}
        self.last_close_date: Dict[str, Optional[str]] = {}
        self.signal_dates: Dict[str, Optional[str]] = {}
        self._rebalanced_on: Optional[str] = None
        #: `date -> instrument -> factor -> raw value`, computed by the Rust
        #: factor-engine and handed in by the runner. Empty for a strategy
        #: that needs no factors, and for every unit test.
        self.factor_series: Dict[str, Dict[str, Dict[str, float]]] = dict(
            getattr(config, "factor_series", {}) or {}
        )
        #: Scheduled dates not yet acted on. The series doubles as the
        #: rebalance schedule, so cadence is decided once, in Rust, from the
        #: dataset's own sessions -- rather than by month-end arithmetic
        #: reimplemented here against a calendar this process cannot see.
        self._pending_schedule: List[str] = sorted(self.factor_series)
        #: The versioned cost profile a fill is charged under, resolved by the
        #: runner. Rates are NEVER defaulted here: an empty profile charges
        #: nothing, which is visible in the artifacts, while a locally-invented
        #: rate would produce fees that look real and match no ledger.
        self.cost_profile: Dict[str, object] = dict(
            getattr(config, "cost_profile", {}) or {}
        )
        # A failure the run must not survive, recorded rather than only
        # raised. NautilusTrader catches and logs whatever an `on_data`
        # handler throws so one bad event cannot kill an engine -- which
        # means an exception alone leaves the backtest running to a clean,
        # empty, SUCCEEDED finish. `simulate.py` reads this after the run.
        self.fatal_error: Optional[dict] = None
        for instrument_id in self.instrument_ids:
            self.pending_targets[instrument_id] = None
            self.positions[instrument_id] = 0
            self.closes[instrument_id] = []
            self.position_ids[instrument_id] = None
            self.last_close_date[instrument_id] = None
            self.signal_dates[instrument_id] = None

    # -- mode --------------------------------------------------------------

    def _engine_attached(self) -> bool:
        """Whether NT has registered this strategy.

        `order_factory` is `None` until registration, which makes it the
        honest test: a strategy that cannot build an order cannot submit one.
        """
        return self.order_factory is not None

    def on_start(self) -> None:
        client_id = ClientId("CUSTOM")
        for instrument_id in self.instrument_ids:
            iid = InstrumentId.from_str(instrument_id)
            self.subscribe_data(
                DataType(SessionOpenEvent), client_id=client_id, instrument_id=iid
            )
            self.subscribe_data(
                DataType(DailyBarClosedEvent), client_id=client_id, instrument_id=iid
            )

    def on_data(self, data) -> None:
        inner = getattr(data, "data", data)
        if type(inner).__name__ == "SessionOpenEvent":
            self._on_session_open(inner)
        elif type(inner).__name__ == "DailyBarClosedEvent":
            self._on_session_close(inner)

    def set_target_portfolio(self, portfolio: dict) -> None:
        """Installs a Todo 16 target portfolio as pending weights."""
        expected = f"{self.STRATEGY_ID}@{self.VERSION}"
        actual = portfolio.get("strategy_version")
        if actual != expected:
            raise AdapterError(
                "MISMATCHED_STRATEGY",
                f"target portfolio {actual} does not match adapter {expected}",
            )
        weights = {
            target["instrument_id"]: float(target["target_weight"])
            for target in portfolio.get("targets", [])
        }
        for instrument_id in self.instrument_ids:
            self.pending_targets[instrument_id] = weights.get(instrument_id, 0.0)

    # -- rebalance ---------------------------------------------------------

    def _required_factors(self) -> List[str]:
        package = importlib.import_module(f"strategies.{self.STRATEGY_ID}.package")
        return list(package.REQUIRED_FACTORS)

    def _factors_for(self, as_of: str) -> Dict[str, Dict[str, float]]:
        """Factor values for `as_of`, keyed by instrument.

        Read from the series the runner computed with the Rust
        `factor-engine`.  Deliberately NOT computed here: a second
        implementation of `return_12m` would make a backtest disagree with the
        paper and live paths that use the Rust one, and a strategy cannot be
        promoted on a backtest that does not describe how it will actually
        behave -- the Paper gate is a parity check.
        """
        return self.factor_series.get(as_of, {})

    def _rebalance_due(self, as_of: str) -> bool:
        """Whether to recompute targets at this close.

        With a series, the schedule is the series: rebalance on the dates the
        runner picked.  The comparison is ``<=`` rather than ``==`` so a
        scheduled date that never arrives as a close event -- a holiday, an
        instrument that did not trade -- is executed at the next close instead
        of being skipped in silence.

        Without a series, once, on the first close.  That is exactly right for
        a buy-and-hold target, which is the case that has no factors and
        therefore no schedule.
        """
        if not self._pending_schedule:
            return not self.factor_series and self._rebalanced_on is None
        return self._pending_schedule[0] <= as_of

    def _rebalance(self, as_of: str) -> None:
        # Consume the schedule entry FIRST, so a date that fails to produce a
        # target is not retried at every subsequent close.
        scheduled = as_of
        if self._pending_schedule:
            scheduled = self._pending_schedule.pop(0)
        required = self._required_factors()
        # The factors of the SCHEDULED date, not of the close that triggered
        # it: when a scheduled date never arrives as a close event the target
        # is still the one that date's data implies.
        factors = self._factors_for(scheduled)
        missing = [f for f in required if not any(f in v for v in factors.values())]
        if missing:
            # Loud, not empty. Returning here would leave `pending_targets`
            # unset, and the run would finish SUCCEEDED with no orders -- a
            # result indistinguishable from a strategy that chose to hold
            # cash, which is the failure this whole path exists to remove.
            #
            # Recorded BEFORE it is raised: the raise only unwinds this
            # handler, and NT logs it and carries on to the next event.
            self.fatal_error = {
                "code": "MISSING_FACTOR_SUPPLY",
                "detail": (
                    f"{self.STRATEGY_ID} requires {', '.join(missing)} and the "
                    f"factor series carries none of them for {scheduled}; a "
                    f"backtest cannot be run from an empty factor set"
                ),
            }
            raise AdapterError(
                self.fatal_error["code"], self.fatal_error["detail"]
            )
        target_module = importlib.import_module(f"strategies.{self.STRATEGY_ID}.target")
        try:
            portfolio = target_module.generate_target(
                self.parameters, factors, scheduled, list(self.instrument_ids)
            )
            self.set_target_portfolio(portfolio)
        except Exception as exc:
            # ANY failure to produce a target is fatal to the run, not just the
            # missing-factor case checked above -- and the check above is not
            # enough on its own.
            #
            # It compares against the package's STATIC `required_factors`,
            # while a generator looks factors up from the PARAMETERS: with
            # `fast_ma=100`, which `trend_following`'s schema permits, the
            # generator wants `trend_100` and the static list never mentioned
            # it. The guard passes, `generate_target` raises
            # MISSING_REQUIRED_FACTOR, NautilusTrader swallows it, and the run
            # finishes SUCCEEDED with zero orders -- the exact silent wrong
            # answer, reachable by a configuration the product allows.
            #
            # Recorded then re-raised: the raise only unwinds this handler.
            if self.fatal_error is None:
                self.fatal_error = {
                    "code": getattr(exc, "code", "TARGET_GENERATION_FAILED"),
                    "detail": f"{self.STRATEGY_ID} could not compute a target "
                    f"for {scheduled}: {exc}",
                }
            raise
        self._rebalanced_on = scheduled
        for instrument_id in self.instrument_ids:
            # The date the DECISION was made, which is what an order's
            # `signal_date` means; the fill lands at the next open.
            self.signal_dates[instrument_id] = scheduled

    def _on_session_close(self, event) -> None:
        instrument_id = str(event.instrument_id)
        self.last_close_date[instrument_id] = event.trading_date
        self.closes[instrument_id].append(int(event.close))
        if not self._engine_attached():
            # Unregistered: history only. A unit test drives targets itself,
            # and rebalancing under it would replace what it just installed.
            return
        self._record_equity_point(event)
        if self._rebalance_due(event.trading_date):
            self._rebalance(event.trading_date)

    def _on_session_open(self, event) -> None:
        instrument_id = str(event.instrument_id)
        pending = self.pending_targets.get(instrument_id)
        if pending is None:
            return
        self.pending_targets[instrument_id] = None
        open_raw = int(event.open_price)
        if pending > 0.0 and self.positions[instrument_id] == 0:
            quantity = self._target_quantity(instrument_id, open_raw, pending)
            if quantity > 0:
                self.order_intents.append(
                    {
                        "instrument": instrument_id,
                        "side": "BUY",
                        "quantity": quantity,
                        "reduce_only": False,
                        "source": "NEXT_SESSION_OPEN",
                    }
                )
                self._submit(instrument_id, OrderSide.BUY, quantity, event)
        elif pending == 0.0 and self.positions[instrument_id] > 0:
            quantity = self.positions[instrument_id]
            self.order_intents.append(
                {
                    "instrument": instrument_id,
                    "side": "SELL",
                    "quantity": quantity,
                    "reduce_only": True,
                    "source": "NEXT_SESSION_OPEN",
                }
            )
            self._submit(instrument_id, OrderSide.SELL, quantity, event)

    # -- execution ---------------------------------------------------------

    def _submit(self, instrument_id: str, side, quantity: int, event) -> None:
        """Places the order the intent describes.

        A no-op when unregistered, so the intent log stays the whole story for
        a unit test while a backtest gets a real order.
        """
        if not self._engine_attached():
            return
        order = self.order_factory.market(
            InstrumentId.from_str(instrument_id),
            side,
            Quantity.from_int(quantity),
            reduce_only=(side == OrderSide.SELL),
        )
        self.orders.append(
            {
                "order_id": None,
                "client_order_id": None,
                "instrument": instrument_id,
                "side": side.name,
                "quantity": quantity,
                "order_type": "MARKET",
                "signal_date": self.signal_dates.get(instrument_id),
                "created_date": event.trading_date,
                "state": "SUBMITTED",
            }
        )
        self.submit_order(
            order,
            # A sell must reduce the position it was planned against rather
            # than open a short: this venue is CASH and the account cannot
            # borrow.
            position_id=self.position_ids.get(instrument_id)
            if side == OrderSide.SELL
            else None,
        )

    def on_order_submitted(self, event) -> None:
        for order in self.orders:
            if order["client_order_id"] is None:
                order["client_order_id"] = event.client_order_id.value
                order["state"] = "SUBMITTED"
                break

    def _fees_for(self, side_is_sell: bool, quantity: int, price_raw: int) -> tuple[int, int]:
        """What this fill is charged, per the resolved cost profile."""
        return fees_for(self.cost_profile, side_is_sell, quantity, price_raw)

    def on_order_filled(self, event) -> None:
        instrument_id = str(event.instrument_id)
        quantity = int(event.last_qty.as_double())
        price_raw = int(round(event.last_px.as_double() * PRICE_SCALE))
        ts = unix_nanos_to_dt(event.ts_event)
        is_sell = event.order_side != OrderSide.BUY
        commission_raw, tax_raw = self._fees_for(is_sell, quantity, price_raw)
        # Cash and positions move HERE, not at submission. An order that is
        # placed and not filled must not change either, or the equity curve
        # reports money that was never spent.
        #
        # Fees are a cash DEBIT on both sides -- they are paid, not netted --
        # so a sell credits the proceeds and pays out of them. Slippage is not
        # charged: it is already inside the execution price, and charging it
        # again would take it twice.
        if event.order_side == OrderSide.BUY:
            self.cash -= quantity * price_raw / PRICE_SCALE
            self.positions[instrument_id] += quantity
        else:
            self.cash += quantity * price_raw / PRICE_SCALE
            self.positions[instrument_id] -= quantity
        self.cash -= (commission_raw + tax_raw) / PRICE_SCALE
        self.position_ids[instrument_id] = event.position_id
        self.fills.append(
            {
                "fill_id": f"fill-{instrument_id}-{ts.date().isoformat()}",
                "order_id": f"ord-{instrument_id}-{ts.date().isoformat()}",
                "client_order_id": event.client_order_id.value,
                "instrument": instrument_id,
                "side": event.order_side.name,
                "quantity": quantity,
                "price_raw": price_raw,
                "date": ts.date().isoformat(),
                "ts": ts.isoformat(),
                "source": "NEXT_SESSION_OPEN",
                "slippage_bps": self.slippage_bps,
                "commission_raw": commission_raw,
                "tax_raw": tax_raw,
            }
        )
        for order in self.orders:
            if order["client_order_id"] == event.client_order_id.value:
                order["state"] = "FILLED"
                order["order_id"] = f"ord-{instrument_id}-{ts.date().isoformat()}"
                break

    def _record_equity_point(self, event) -> None:
        positions_value = 0.0
        for instrument_id in self.instrument_ids:
            if instrument_id == str(event.instrument_id):
                close_raw = int(event.close)
            elif self.closes[instrument_id]:
                close_raw = self.closes[instrument_id][-1]
            else:
                continue
            positions_value += self.positions[instrument_id] * close_raw / PRICE_SCALE
        self.equity_points.append(
            {
                "date": event.trading_date,
                "cash_raw": int(round(self.cash * PRICE_SCALE)),
                "positions_value_raw": int(round(positions_value * PRICE_SCALE)),
                "equity_raw": int(round((self.cash + positions_value) * PRICE_SCALE)),
            }
        )

    def _target_quantity(self, instrument_id: str, open_raw: int, weight: float) -> int:
        price = open_raw / 10_000
        notional = float(self.initial_cash) * weight
        shares = int(notional / price)
        return (shares // self.lot_size) * self.lot_size
