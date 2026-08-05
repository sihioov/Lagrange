"""Red-first suite: the NT execution adapters (FR-STR-005).  Adapters consume
Todo 13 custom events (`SessionOpenEvent` / `DailyBarClosedEvent`) and Todo 16
target portfolios; they translate target weights into next-open order
intents.  Target generators never create orders — the adapter is the only
strategy-side order boundary."""

import pytest

from conftest import STRATEGIES, load_adapter, load_golden, load_target, make_close, make_open


@pytest.mark.parametrize("sid", STRATEGIES)
def test_adapter_imports_and_subclasses_base(sid, execution_module):
    mod = load_adapter(sid)
    assert mod.STRATEGY_ID == sid
    assert mod.VERSION == "1.0.0"
    adapter_class = getattr(mod, mod.__all__[0])
    assert issubclass(adapter_class, execution_module.TargetExecutionStrategy)
    config_class = getattr(mod, "".join(part.title() for part in mod.STRATEGY_ID.split("_")) + "Config")
    assert config_class.__name__.endswith("Config")


@pytest.mark.parametrize("sid", STRATEGIES)
def test_adapter_default_config_matches_package_defaults(sid):
    from conftest import load_package

    mod = load_adapter(sid)
    adapter_class = getattr(mod, mod.__all__[0])
    config_class = getattr(mod, "".join(part.title() for part in mod.STRATEGY_ID.split("_")) + "Config")
    cfg = config_class()
    assert cfg.parameters == load_package(sid).PACKAGE["default_parameters"]
    assert cfg.instrument_ids, "at least one instrument"


def test_buy_and_hold_adapter_executes_golden_target_at_next_open(events, execution_module):
    mod = load_adapter("buy_and_hold")
    golden = load_golden("buy_and_hold")
    gen = load_target("buy_and_hold")
    case = golden["cases"][0]

    cfg = mod.BuyAndHoldConfig(instrument_ids=["069500.KRX"])
    strategy = mod.BuyAndHoldAdapter(cfg)
    portfolio = gen.generate_target(
        case["params"], case.get("factors", {}), case["as_of"], case.get("universe")
    )
    strategy.set_target_portfolio(portfolio)

    strategy.on_data(make_open(events))
    assert strategy.order_intents == [
        {
            "instrument": "069500.KRX",
            "side": "BUY",
            "quantity": 10700,
            "reduce_only": False,
            "source": "NEXT_SESSION_OPEN",
        }
    ]
    # The pending target is consumed: a second open with no new target emits
    # nothing (T-close signal executes once at T+1 open).
    strategy.on_data(make_open(events))
    assert strategy.order_intents == [
        {
            "instrument": "069500.KRX",
            "side": "BUY",
            "quantity": 10700,
            "reduce_only": False,
            "source": "NEXT_SESSION_OPEN",
        }
    ]


def test_adapter_exits_position_when_target_is_cash(events):
    mod = load_adapter("buy_and_hold")

    cfg = mod.BuyAndHoldConfig(instrument_ids=["069500.KRX"])
    strategy = mod.BuyAndHoldAdapter(cfg)
    strategy.positions["069500.KRX"] = 10700  # engine-reported filled position

    # An exit-to-cash target arrives as an empty-targets portfolio (the
    # schema forbids target_weight 0.0 by design).
    portfolio = {
        "strategy_version": "buy_and_hold@1.0.0",
        "targets": [],
        "exclusions": [],
        "cash_weight": 1.0,
        "portfolio_reasons": [],
        "as_of": "2020-02-03",
        "universe_snapshot_id": "",
        "factor_snapshot_hash": "",
        "dataset_id": "",
        "dataset_version": 0,
        "constraints": {},
        "portfolio_snapshot_id": "",
    }
    strategy.set_target_portfolio(portfolio)
    strategy.on_data(make_open(events))
    assert strategy.order_intents == [
        {
            "instrument": "069500.KRX",
            "side": "SELL",
            "quantity": 10700,
            "reduce_only": True,
            "source": "NEXT_SESSION_OPEN",
        }
    ]


def test_adapter_consumes_close_events_for_history(events):
    mod = load_adapter("trend_following")
    cfg = mod.TrendFollowingConfig(instrument_ids=["069500.KRX"])
    strategy = mod.TrendFollowingAdapter(cfg)
    strategy.on_data(make_close(events, date="2020-01-31", close_raw=93500000))
    strategy.on_data(make_close(events, date="2020-02-03", close_raw=94000000))
    assert strategy.closes["069500.KRX"] == [93500000, 94000000]


def test_adapter_rejects_foreign_strategy_targets(events):
    mod = load_adapter("buy_and_hold")
    gen = load_target("relative_momentum")
    cfg = mod.BuyAndHoldConfig(instrument_ids=["069500.KRX"])
    strategy = mod.BuyAndHoldAdapter(cfg)
    foreign = gen.generate_target(
        {"top_n": 2, "lookback_months": 12},
        {"069500.KRX": {"momentum_12_1": 0.15}, "102110.KRX": {"momentum_12_1": 0.10}},
        "2020-02-03",
        ["069500.KRX", "102110.KRX"],
    )
    with pytest.raises(Exception) as exc:
        strategy.set_target_portfolio(foreign)
    assert getattr(exc.value, "code", None) == "MISMATCHED_STRATEGY"


@pytest.mark.parametrize("sid", STRATEGIES)
def test_all_adapters_accept_their_own_golden_targets(events, sid):
    mod = load_adapter(sid)
    gen = load_target(sid)
    golden = load_golden(sid)
    for case in golden["cases"]:
        cfg_class = getattr(mod, "".join(part.title() for part in mod.STRATEGY_ID.split("_")) + "Config")
        cfg = cfg_class(instrument_ids=case.get("universe", ["069500.KRX"]))
        strategy = getattr(mod, mod.__all__[0])(cfg)
        portfolio = gen.generate_target(
            case["params"], case.get("factors", {}), case["as_of"], case.get("universe")
        )
        strategy.set_target_portfolio(portfolio)
        strategy.on_data(make_open(events, iid=strategy.instrument_ids[0]))
        assert isinstance(strategy.order_intents, list)
