"""The engine-independent NT execution adapter base (FR-STR-005, design §8.3).

Target generators produce `TargetPortfolio`s (Todo 16); this adapter is the
strategy-side order boundary that executes them at the next session open.
It consumes the Lagrange-owned `SessionOpenEvent` / `DailyBarClosedEvent`
custom data (Todo 13): close events maintain per-instrument history, open
events execute pending targets.  Sells are planned before buys and are
reduce-only; integer lot sizing; pending targets are consumed once (a
T-close signal executes at the T+1 open exactly once).
"""

import importlib
from typing import Dict, List, Optional

from nautilus_trader.model.data import DataType
from nautilus_trader.model.identifiers import ClientId, InstrumentId
from nautilus_trader.trading.strategy import Strategy

_session = importlib.import_module("custom-data.session_events")
SessionOpenEvent = _session.SessionOpenEvent
DailyBarClosedEvent = _session.DailyBarClosedEvent


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
        for instrument_id in self.instrument_ids:
            self.pending_targets[instrument_id] = None
            self.positions[instrument_id] = 0
            self.closes[instrument_id] = []

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

    def _on_session_close(self, event) -> None:
        instrument_id = str(event.instrument_id)
        self.closes[instrument_id].append(int(event.close))

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
        elif pending == 0.0 and self.positions[instrument_id] > 0:
            self.order_intents.append(
                {
                    "instrument": instrument_id,
                    "side": "SELL",
                    "quantity": self.positions[instrument_id],
                    "reduce_only": True,
                    "source": "NEXT_SESSION_OPEN",
                }
            )

    def _target_quantity(self, instrument_id: str, open_raw: int, weight: float) -> int:
        price = open_raw / 10_000
        notional = float(self.initial_cash) * weight
        shares = int(notional / price)
        return (shares // self.lot_size) * self.lot_size
