"""Todo 13 red tests: replay through BacktestDataConfig (manual QA channel).

Writes/queries events for the three seed instruments and replays them through
``BacktestDataConfig``; the strategy transcript must show, per instrument,
close(T) -> open(T+1) -> close(T+1) in exact order, and the open callback
must NOT be able to read same-day high/low/close (typed barrier).

The strategy/config classes below are imported by ``resolve_path`` through
``strategy_path="test_replay:ReplayStrategy"`` while pytest holds the same
module in ``sys.modules``, so class identity is stable (no double
registration of the custom data types).
"""
from __future__ import annotations

import importlib

import pytest
from nautilus_trader.model.data import DataType
from nautilus_trader.model.identifiers import ClientId, InstrumentId
from nautilus_trader.trading.config import StrategyConfig
from nautilus_trader.trading.strategy import Strategy

from curated_helpers import equity_from_dict

_session_events = importlib.import_module("custom-data.session_events")
SessionOpenEvent = _session_events.SessionOpenEvent
DailyBarClosedEvent = _session_events.DailyBarClosedEvent

REPLAY_INSTRUMENTS = ["069500.KRX", "229200.KRX", "114260.KRX"]


class ReplayStrategyConfig(StrategyConfig, frozen=True):
    instrument_id: str = "069500.KRX"


class ReplayStrategy(Strategy):
    """Records the custom event transcript and enforces the future-field
    barrier inside the open callback."""

    def __init__(self, config: ReplayStrategyConfig):
        super().__init__(config)
        self.transcript: list[tuple[str, str]] = []  # (kind, trading_date)
        self.barrier_violations: list[str] = []

    def on_start(self) -> None:
        client_id = ClientId("CUSTOM")
        self.subscribe_data(data_type=DataType(SessionOpenEvent), client_id=client_id)
        self.subscribe_data(data_type=DataType(DailyBarClosedEvent), client_id=client_id)

    def on_data(self, data) -> None:
        inner = getattr(data, "data", data)
        if type(inner).__name__ == "SessionOpenEvent":
            self.transcript.append(("open", inner.trading_date))
            for future_field in ("high", "low", "close"):
                if hasattr(inner, future_field):
                    self.barrier_violations.append(future_field)
        elif type(inner).__name__ == "DailyBarClosedEvent":
            self.transcript.append(("close", inner.trading_date))


def _build_replay_catalog(builder, curated_root, tmp_path):
    """Builds a catalog plus Equity definitions for the three seed ETFs."""
    from nautilus_trader.model.instruments import Equity
    from nautilus_trader.persistence.catalog import ParquetDataCatalog

    catalog = tmp_path / "nautilus_catalog"
    builder.build_catalog(curated_root, catalog)
    cat = ParquetDataCatalog(path=str(catalog))
    cat.write_data([Equity.from_dict(equity_from_dict(iid)) for iid in REPLAY_INSTRUMENTS])
    return catalog


def _data_configs(catalog, iid):
    from nautilus_trader.backtest.config import BacktestDataConfig

    return [
        BacktestDataConfig(catalog_path=str(catalog),
                           data_cls="custom-data.session_events:SessionOpenEvent",
                           instrument_id=InstrumentId.from_str(iid), client_id="CUSTOM"),
        BacktestDataConfig(catalog_path=str(catalog),
                           data_cls="custom-data.session_events:DailyBarClosedEvent",
                           instrument_id=InstrumentId.from_str(iid), client_id="CUSTOM"),
    ]


def _run_replay_node(builder, curated_root, tmp_path, iid):
    from nautilus_trader.backtest.config import (
        BacktestEngineConfig, BacktestRunConfig, BacktestVenueConfig,
    )
    from nautilus_trader.backtest.node import BacktestNode
    from nautilus_trader.trading.config import ImportableStrategyConfig

    catalog = _build_replay_catalog(builder, curated_root, tmp_path)
    config = BacktestRunConfig(
        venues=[BacktestVenueConfig(name="KRX", oms_type="HEDGING", account_type="CASH",
                                    starting_balances=["100000000 KRW"])],
        engine=BacktestEngineConfig(strategies=[
            ImportableStrategyConfig(strategy_path="test_replay:ReplayStrategy",
                                     config_path="test_replay:ReplayStrategyConfig",
                                     config={"instrument_id": iid})]),
        data=_data_configs(catalog, iid),
        dispose_on_completion=False,
    )
    node = BacktestNode(configs=[config])
    node.run()
    return node


def _transcript_for(node):
    engine = node.get_engines()[0]
    strat = [s for s in engine.trader.strategies()][0]
    return strat.transcript, strat.barrier_violations


def test_replay_three_seed_instruments(builder, curated_root, tmp_path):
    for iid in REPLAY_INSTRUMENTS:
        node = _run_replay_node(builder, curated_root, tmp_path, iid)
        transcript, violations = _transcript_for(node)
        assert violations == [], f"future-field barrier violated for {iid}: {violations}"
        opens = [d for k, d in transcript if k == "open"]
        closes = [d for k, d in transcript if k == "close"]
        assert len(opens) == 9 and len(closes) == 9, transcript
        # Exact contract on the 2020-01-31 -> 2020-02-03 transition:
        # close(T) precedes open(T+1) which precedes close(T+1) in the
        # delivered transcript order.
        pos_close_t = [i for i, (k, d) in enumerate(transcript)
                       if k == "close" and d == "2020-01-31"][0]
        pos_open_t1 = [i for i, (k, d) in enumerate(transcript)
                       if k == "open" and d == "2020-02-03"][0]
        pos_close_t1 = [i for i, (k, d) in enumerate(transcript)
                        if k == "close" and d == "2020-02-03"][0]
        assert pos_close_t < pos_open_t1 < pos_close_t1
        # First event is the first session's open; last is the last close.
        assert transcript[0][0] == "open" and transcript[0][1] == "2020-01-20"
        assert transcript[-1][0] == "close" and transcript[-1][1] == "2020-02-03"


def test_open_callback_cannot_access_same_day_ohlc(events):
    """Typed barrier: .high/.low/.close on SessionOpenEvent raise AttributeError."""
    from nautilus_trader.model.identifiers import InstrumentId

    SessionOpenEvent = events.SessionOpenEvent
    open_ev = SessionOpenEvent(
        instrument_id=InstrumentId.from_str("069500.KRX"),
        trading_date="2020-02-03",
        session_open_ts=1580688000000000000,
        open_price=103000000,
        currency="KRW",
        data_version="1",
        ts_event=1580688000000000000,
        ts_init=1580688000000000000,
    )
    with pytest.raises(AttributeError):
        _ = open_ev.high
    with pytest.raises(AttributeError):
        _ = open_ev.low
    with pytest.raises(AttributeError):
        _ = open_ev.close
