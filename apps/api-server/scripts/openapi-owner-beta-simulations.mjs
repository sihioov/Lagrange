const base = "/api/v1/backtests/owner-beta/price-only";

export const SIMULATION_ROUTES = [
  ["POST", `${base}/runs`, {
    owner: true,
    entitlement: "backtest",
    ownerBetaSimulation: true,
    mutating: true,
    idem: true,
    audit: true,
  }],
  ["GET", `${base}/runs`, {
    owner: true,
    entitlement: "backtest",
    ownerBetaSimulation: true,
  }],
  ["GET", `${base}/runs/{run_id}`, {
    owner: true,
    entitlement: "backtest",
    ownerBetaSimulation: true,
  }],
  ["POST", `${base}/runs/{run_id}/cancel`, {
    owner: true,
    ownerBetaSimulation: true,
    mutating: true,
    idem: true,
    audit: true,
  }],
  ["GET", `${base}/runs/{run_id}/pair`, {
    owner: true,
    entitlement: "backtest",
    ownerBetaSimulation: true,
  }],
];

const closed = (required, properties) => ({
  type: "object",
  required,
  additionalProperties: false,
  properties,
});
const ref = (name) => ({ $ref: `#/components/schemas/${name}` });
const date = { type: "string", pattern: "^\\d{4}-\\d{2}-\\d{2}$" };
const timestamp = { type: "string", format: "date-time" };
const digest = { type: "string", pattern: "^sha256:[0-9a-f]{64}$" };
const fixed4 = { type: "string", pattern: "^(0|[1-9][0-9]*)\\.[0-9]{4}$" };
const ratio12 = { type: "string", pattern: "^-?(0|[1-9][0-9]*)\\.[0-9]{12}$" };
const dateArray = { type: "array", items: date };
const enumString = (...values) => ({ type: "string", enum: values });
const v2Instruments = ["069500.KRX", "102110.KRX", "114260.KRX", "132030.KRX", "133690.KRX", "143850.KRX", "148070.KRX", "153130.KRX", "192090.KRX", "195930.KRX", "229200.KRX"];
const v2InstrumentId = enumString(...v2Instruments);

const resultSchemas = {
  SimulationResult: closed(
    ["contract", "input", "engine", "strategy", "schedule", "conventions", "signals", "strategy_ledger", "benchmark_ledger"],
    {
      contract: ref("SimulationContractIdentity"),
      input: ref("SimulationInputIdentity"),
      engine: ref("SimulationEngineIdentity"),
      strategy: ref("SimulationStrategyIdentity"),
      schedule: ref("SimulationSchedule"),
      conventions: ref("SimulationConventions"),
      signals: { type: "array", items: ref("SimulationSignal") },
      strategy_ledger: ref("SimulationStrategyLedger"),
      benchmark_ledger: ref("SimulationBenchmarkLedger"),
    },
  ),
  SimulationContractIdentity: closed(
    ["contract_version", "contract_document_sha256", "result_schema_version"],
    {
      contract_version: { type: "string", const: "owner-beta-price-only-simulation-v1-draft-1" },
      contract_document_sha256: digest,
      result_schema_version: { type: "string", const: "owner-beta-price-only-result-v1" },
    },
  ),
  SimulationInputIdentity: closed(
    ["pins", "bars_sha256", "approved_start_date", "approved_end_date", "session_count", "bar_count", "instruments", "labels"],
    {
      pins: ref("SimulationApprovedPins"),
      bars_sha256: digest,
      approved_start_date: date,
      approved_end_date: date,
      session_count: { type: "integer", const: 2452 },
      bar_count: { type: "integer", const: 26972 },
      instruments: { type: "array", items: { type: "string", enum: ["069500.KRX", "102110.KRX", "114260.KRX", "132030.KRX", "133690.KRX", "143850.KRX", "148070.KRX", "153130.KRX", "192090.KRX", "195930.KRX", "229200.KRX"] }, minItems: 11, maxItems: 11 },
      labels: ref("SimulationEvidenceLabels"),
    },
  ),
  SimulationApprovedPins: closed(
    ["candidate_content_sha256", "artifact_manifest_sha256", "stage5_manifest_sha256", "action_manifest_sha256", "approval_registry_sha256"],
    {
      candidate_content_sha256: digest,
      artifact_manifest_sha256: digest,
      stage5_manifest_sha256: digest,
      action_manifest_sha256: digest,
      approval_registry_sha256: digest,
    },
  ),
  SimulationEvidenceLabels: closed(
    ["data_scope", "return_basis", "vendor_snapshot", "strict_pit"],
    {
      data_scope: { type: "string", const: "OWNER_ONLY" },
      return_basis: { type: "string", const: "PRICE_RETURN_ONLY" },
      vendor_snapshot: { type: "boolean", const: true },
      strict_pit: { type: "boolean", const: false },
    },
  ),
  SimulationEngineIdentity: closed(["git_commit", "binary_sha256", "toolchain", "target", "profile"], {
    git_commit: { type: "string", pattern: "^[0-9a-f]{40}$" },
    binary_sha256: digest,
    toolchain: { type: "string", minLength: 1 },
    target: { type: "string", minLength: 1 },
    profile: { type: "string", minLength: 1 },
  }),
  SimulationStrategyIdentity: closed(["strategy_id", "version", "config", "strategy_snapshot_sha256", "factor_definition"], {
    strategy_id: { type: "string", const: "trend_following" },
    version: { type: "string", const: "1.0.0" },
    config: ref("SimulationTrendConfig"),
    strategy_snapshot_sha256: digest,
    factor_definition: ref("SimulationFactorDefinition"),
  }),
  SimulationTrendConfig: closed(["benchmark_instrument", "fast_ma", "slow_ma", "target_rule"], {
    benchmark_instrument: { type: "string", const: "069500.KRX" },
    fast_ma: { type: "integer", const: 50 },
    slow_ma: { type: "integer", const: 200 },
    target_rule: { type: "string", const: "BUY_069500_IF_TREND_50_LT_TREND_200_ELSE_CASH" },
  }),
  SimulationFactorDefinition: closed(["version", "sha256"], {
    version: { type: "string", minLength: 1 },
    sha256: digest,
  }),
  SimulationSchedule: closed(["warmup_sessions", "signal_execution", "evaluation_sessions", "phase_order"], {
    warmup_sessions: { ...dateArray, minItems: 200 },
    signal_execution: { type: "array", items: ref("SimulationSignalExecution") },
    evaluation_sessions: { ...dateArray, minItems: 2 },
    phase_order: { type: "array", items: ref("SimulationLedgerPhase"), minItems: 6, maxItems: 6 },
  }),
  SimulationSignalExecution: closed(["signal_date", "execution_date"], {
    signal_date: date,
    execution_date: date,
  }),
  SimulationLedgerPhase: enumString("INITIAL_CASH", "PRE_OPEN_MARK", "SELL", "BUY", "CLOSE_MARK", "SIGNAL_FOR_NEXT_SESSION"),
  SimulationSignal: closed(["signal_date", "factor_snapshot_sha256", "target_snapshot_sha256", "targets", "cash_weight_ppm"], {
    signal_date: date,
    factor_snapshot_sha256: digest,
    target_snapshot_sha256: digest,
    targets: { type: "array", items: ref("SimulationInstrumentTarget"), minItems: 11, maxItems: 11 },
    cash_weight_ppm: { type: "integer", minimum: 0, maximum: 1000000 },
  }),
  SimulationInstrumentTarget: closed(["instrument_id", "weight_ppm"], {
    instrument_id: { type: "string", enum: ["069500.KRX", "102110.KRX", "114260.KRX", "132030.KRX", "133690.KRX", "143850.KRX", "148070.KRX", "153130.KRX", "192090.KRX", "195930.KRX", "229200.KRX"] },
    weight_ppm: { type: "integer", minimum: 0, maximum: 1000000 },
  }),
  SimulationConventions: closed(["precision", "portfolio", "pricing", "cost", "benchmark", "turnover", "metrics"], {
    precision: ref("SimulationPrecisionConvention"),
    portfolio: ref("SimulationPortfolioConvention"),
    pricing: ref("SimulationPricingConvention"),
    cost: ref("SimulationCostConvention"),
    benchmark: ref("SimulationBenchmarkConvention"),
    turnover: ref("SimulationTurnoverConvention"),
    metrics: ref("SimulationMetricsConvention"),
  }),
  SimulationPrecisionConvention: closed(["price_scale", "money_scale", "ratio_scale", "quantity_scale", "target_weight_scale_ppm", "ratio_rounding", "negative_zero"], {
    price_scale: { type: "integer", const: 4 },
    money_scale: { type: "integer", const: 4 },
    ratio_scale: { type: "integer", const: 12 },
    quantity_scale: { type: "integer", const: 0 },
    target_weight_scale_ppm: { type: "integer", const: 1000000 },
    ratio_rounding: { type: "string", const: "TIES_TO_EVEN" },
    negative_zero: { type: "string", const: "NORMALIZE_TO_ZERO" },
  }),
  SimulationPortfolioConvention: closed(["initial_capital", "currency", "quantity_unit", "share_sizing", "target_transition", "residual_cash", "borrowing", "short_selling", "cash_interest", "additional_cash_flows", "dividends"], {
    initial_capital: { ...fixed4, const: "10000000.0000" },
    currency: { type: "string", const: "KRW" },
    quantity_unit: { type: "string", const: "WHOLE_SHARES" },
    share_sizing: { type: "string", const: "MAXIMUM_COST_INCLUSIVE_WHOLE_SHARES" },
    target_transition: { type: "string", const: "FULL_POSITION_OR_CASH_ONLY_ON_TARGET_CHANGE" },
    residual_cash: { type: "string", const: "NO_ADDITIONAL_PURCHASE_AFTER_INITIAL_FILL" },
    borrowing: { type: "boolean", const: false },
    short_selling: { type: "boolean", const: false },
    cash_interest: { type: "string", const: "NONE" },
    additional_cash_flows: { type: "boolean", const: false },
    dividends: { type: "string", const: "EXCLUDED" },
  }),
  SimulationPricingConvention: closed(["signal_price", "execution_price", "valuation_price", "corporate_actions", "missing_data", "volume", "market_impact", "partial_fills", "trade_order", "ending", "warmup_returns_included", "market_timezone", "timestamp_fields_included"], {
    signal_price: { type: "string", const: "ADJUSTED_CLOSE" },
    execution_price: { type: "string", const: "RAW_OPEN" },
    valuation_price: { type: "string", const: "RAW_CLOSE" },
    corporate_actions: { type: "string", const: "REJECT_NONCASH_AND_REQUIRE_RAW_ADJUSTED_OHLC_EQUAL" },
    missing_data: { type: "string", const: "FAIL_WHOLE_RUN_NO_CARRY_FORWARD" },
    volume: { type: "string", const: "REQUIRE_POSITIVE_VOLUME_WHEN_TRADING" },
    market_impact: { type: "string", const: "NONE" },
    partial_fills: { type: "boolean", const: false },
    trade_order: { type: "string", const: "SELL_BEFORE_BUY" },
    ending: { type: "string", const: "FINAL_CLOSE_NO_LIQUIDATION" },
    warmup_returns_included: { type: "boolean", const: false },
    market_timezone: { type: "string", const: "Asia/Seoul" },
    timestamp_fields_included: { type: "boolean", const: false },
  }),
  SimulationCostConvention: closed(["profile", "per_side_rate_numerator", "per_side_rate_denominator", "minimum_cost", "separate_tax", "price_slippage", "rounding"], {
    profile: enumString("ILLUSTRATIVE_10BP_PER_SIDE_V1", "ZERO_COST_REFERENCE_V1"),
    per_side_rate_numerator: { type: "integer", minimum: 0 },
    per_side_rate_denominator: { type: "integer", minimum: 1 },
    minimum_cost: fixed4,
    separate_tax: fixed4,
    price_slippage: fixed4,
    rounding: { type: "string", const: "CEILING_TO_MONEY_SCALE" },
  }),
  SimulationBenchmarkConvention: closed(["instrument_id", "policy", "initial_purchase", "rebalancing", "additional_cash_purchases", "forced_final_liquidation", "same_cost_profile"], {
    instrument_id: { type: "string", const: "069500.KRX" },
    policy: { type: "string", const: "BUY_AND_HOLD_AT_START_OPEN" },
    initial_purchase: { type: "string", const: "COST_INCLUSIVE_MAXIMUM_WHOLE_SHARES" },
    rebalancing: { type: "boolean", const: false },
    additional_cash_purchases: { type: "boolean", const: false },
    forced_final_liquidation: { type: "boolean", const: false },
    same_cost_profile: { type: "boolean", const: true },
  }),
  SimulationTurnoverConvention: closed(["formula", "include_initial_trade", "fees_and_deposits_included", "annualized"], {
    formula: { type: "string", const: "SUM_DAILY_BUY_PLUS_SELL_OVER_TWICE_PRETRADE_OPEN_NAV" },
    include_initial_trade: { type: "boolean", const: true },
    fees_and_deposits_included: { type: "boolean", const: false },
    annualized: { type: "boolean", const: false },
  }),
  SimulationMetricsConvention: closed(["minimum_evaluation_sessions", "allowed_metrics", "cagr_included", "annualized_volatility_included", "sharpe_included", "sortino_included", "win_rate_included", "trade_return_included", "monthly_metrics_included"], {
    minimum_evaluation_sessions: { type: "integer", const: 2 },
    allowed_metrics: { type: "array", items: enumString("daily_returns", "period_return", "ending_nav", "ending_cash", "ending_holdings", "cumulative_costs", "buy_notional", "sell_notional", "maximum_drawdown", "period_turnover", "fill_count", "benchmark_return_difference") },
    cagr_included: { type: "boolean", const: false },
    annualized_volatility_included: { type: "boolean", const: false },
    sharpe_included: { type: "boolean", const: false },
    sortino_included: { type: "boolean", const: false },
    win_rate_included: { type: "boolean", const: false },
    trade_return_included: { type: "boolean", const: false },
    monthly_metrics_included: { type: "boolean", const: false },
  }),
  SimulationStrategyLedger: closed(["initial_cash", "records", "metrics", "benchmark_return_difference"], {
    initial_cash: fixed4,
    records: { type: "array", items: ref("SimulationLedgerRecord") },
    metrics: ref("SimulationLedgerMetrics"),
    benchmark_return_difference: ratio12,
  }),
  SimulationBenchmarkLedger: closed(["initial_cash", "records", "metrics"], {
    initial_cash: fixed4,
    records: { type: "array", items: ref("SimulationLedgerRecord") },
    metrics: ref("SimulationLedgerMetrics"),
  }),
  SimulationLedgerRecord: closed(["session_date", "phase", "events", "unfilled_reason", "cash", "positions", "nav"], {
    session_date: date,
    phase: ref("SimulationLedgerPhase"),
    events: { type: "array", items: ref("SimulationTradeEvent") },
    unfilled_reason: enumString("NOT_APPLICABLE", "NO_ORDER_REQUIRED", "ZERO_SHARE_QUANTITY", "FILLED"),
    cash: fixed4,
    positions: { type: "array", items: ref("SimulationPosition") },
    nav: fixed4,
  }),
  SimulationTradeEvent: closed(["instrument_id", "side", "quantity", "price", "notional", "cost", "cash_after", "position_after"], {
    instrument_id: { type: "string", pattern: "^[0-9]{6}\\.KRX$" },
    side: enumString("BUY", "SELL"),
    quantity: { type: "integer", minimum: 1 },
    price: fixed4,
    notional: fixed4,
    cost: fixed4,
    cash_after: fixed4,
    position_after: { type: "integer", minimum: 0 },
  }),
  SimulationPosition: closed(["instrument_id", "quantity", "price", "market_value"], {
    instrument_id: { type: "string", pattern: "^[0-9]{6}\\.KRX$" },
    quantity: { type: "integer", minimum: 1 },
    price: fixed4,
    market_value: fixed4,
  }),
  SimulationLedgerMetrics: closed(["daily_returns", "period_return", "ending_nav", "ending_cash", "ending_holdings", "cumulative_costs", "buy_notional", "sell_notional", "maximum_drawdown", "period_turnover", "fill_count"], {
    daily_returns: { type: "array", items: ref("SimulationDailyReturn") },
    period_return: ratio12,
    ending_nav: fixed4,
    ending_cash: fixed4,
    ending_holdings: { type: "array", items: ref("SimulationHolding") },
    cumulative_costs: fixed4,
    buy_notional: fixed4,
    sell_notional: fixed4,
    maximum_drawdown: ratio12,
    period_turnover: ratio12,
    fill_count: { type: "integer", minimum: 0 },
  }),
  SimulationDailyReturn: closed(["session_date", "value"], { session_date: date, value: ratio12 }),
  SimulationHolding: closed(["instrument_id", "quantity"], {
    instrument_id: { type: "string", pattern: "^[0-9]{6}\\.KRX$" },
    quantity: { type: "integer", minimum: 1 },
  }),
};

const v2StrategyIdentity = (strategy, config, factorNames) => closed(
  ["strategy", "version", "canonical_config_json", "snapshot_sha256", "factors"],
  {
    strategy: { type: "string", const: strategy },
    version: { type: "string", const: "1.0.0" },
    canonical_config_json: { type: "string", const: config },
    snapshot_sha256: digest,
    factors: {
      type: "array",
      ...(factorNames.length === 0 ? {} : {
        items: { oneOf: factorNames.map((name) => closed(["name", "version", "sha256"], {
          name: { type: "string", const: name },
          version: { type: "string", minLength: 1 },
          sha256: digest,
        })) },
      }),
      minItems: factorNames.length,
      maxItems: factorNames.length,
    },
  },
);

const v2StrategyIdentitySchema = {
  oneOf: [
    v2StrategyIdentity("buy_and_hold", '{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}', []),
    v2StrategyIdentity("trend_following", '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}', ["trend_50", "trend_200"]),
    v2StrategyIdentity("relative_momentum", '{"lookback_months":12,"top_n":3}', ["momentum_12_1"]),
    v2StrategyIdentity("dual_momentum", '{"absolute_threshold":0.0,"lookback_months":12}', ["return_12m"]),
    v2StrategyIdentity("inverse_volatility", '{"max_weight":0.3,"vol_window":60}', ["vol_60"]),
  ],
};

const v2TargetAllocation = closed(["target_sha256", "allocation_sha256", "lineage_sha256", "weights", "cash_weight_ppm"], {
  target_sha256: digest,
  allocation_sha256: digest,
  lineage_sha256: digest,
  weights: { type: "array", items: closed(["instrument_id", "weight_ppm"], {
    instrument_id: v2InstrumentId,
    weight_ppm: { type: "integer", minimum: 0, maximum: 1000000 },
  }), minItems: 11, maxItems: 11 },
  cash_weight_ppm: { type: "integer", minimum: 0, maximum: 1000000 },
});

const v2Decision = closed(["signal_date", "execution_date", "kind"], {
  signal_date: date,
  execution_date: date,
  kind: enumString("INITIAL_COMMON_SESSION", "DAILY_TREND_TRANSITION_CHECK", "MONTH_END_SIGNAL"),
});

const v2Schedule = closed(["strategy", "preparation_session_count", "first_signal_date", "preparation_sessions", "evaluation_sessions", "decisions"], {
  strategy: enumString("buy_and_hold", "trend_following", "relative_momentum", "dual_momentum", "inverse_volatility"),
  preparation_session_count: { type: "integer", minimum: 200, maximum: 2450 },
  first_signal_date: date,
  preparation_sessions: { ...dateArray, minItems: 200, maxItems: 2450 },
  evaluation_sessions: { ...dateArray, minItems: 2, maxItems: 2252 },
  decisions: { type: "array", items: v2Decision, minItems: 1, maxItems: 2252 },
});

const v2Position = closed(["instrument_id", "quantity"], {
  instrument_id: v2InstrumentId,
  quantity: { type: "integer", minimum: 1 },
});

const v2TargetQuantity = closed(["instrument_id", "quantity"], {
  instrument_id: v2InstrumentId,
  quantity: { type: "integer", minimum: 0 },
});

const v2CloseMark = closed(["instrument_id", "price"], {
  instrument_id: v2InstrumentId,
  price: fixed4,
});

const v2InstrumentPrice = closed(["instrument_id", "open", "close", "volume"], {
  instrument_id: v2InstrumentId,
  open: fixed4,
  close: fixed4,
  volume: { type: "integer", minimum: 0 },
});

const v2MarketSession = closed(["date", "prices"], {
  date,
  prices: { type: "array", items: v2InstrumentPrice, minItems: 11, maxItems: 11 },
});

const v2TargetObservation = closed(["signal_date", "execution_date", "decision_kind", "target"], {
  signal_date: date,
  execution_date: date,
  decision_kind: enumString("INITIAL_COMMON_SESSION", "DAILY_TREND_TRANSITION_CHECK", "MONTH_END_SIGNAL"),
  target: v2TargetAllocation,
});

const v2Order = closed(["instrument_id", "side", "requested_quantity", "filled_quantity", "price", "notional", "cost", "cash_after", "position_after", "unfilled_reason"], {
  instrument_id: v2InstrumentId,
  side: enumString("BUY", "SELL"),
  requested_quantity: { type: "integer", minimum: 1 },
  filled_quantity: { type: "integer", minimum: 0 },
  price: fixed4,
  notional: fixed4,
  cost: fixed4,
  cash_after: fixed4,
  position_after: { type: "integer", minimum: 0 },
  unfilled_reason: { anyOf: [enumString("INSUFFICIENT_COST_INCLUSIVE_BUDGET"), { type: "null" }] },
});

const v2Rebalance = closed(["signal_date", "execution_date", "decision_kind", "target", "pre_trade_nav", "open_prices", "target_quantities", "orders", "cash_after", "positions_after", "session_costs", "buy_notional", "sell_notional"], {
  signal_date: date,
  execution_date: date,
  decision_kind: enumString("INITIAL_COMMON_SESSION", "DAILY_TREND_TRANSITION_CHECK", "MONTH_END_SIGNAL"),
  target: v2TargetAllocation,
  pre_trade_nav: fixed4,
  open_prices: { type: "array", items: v2InstrumentPrice, minItems: 11, maxItems: 11 },
  target_quantities: { type: "array", items: v2TargetQuantity, minItems: 11, maxItems: 11 },
  orders: { type: "array", items: v2Order, maxItems: 22 },
  cash_after: fixed4,
  positions_after: { type: "array", items: v2Position, maxItems: 11 },
  session_costs: fixed4,
  buy_notional: fixed4,
  sell_notional: fixed4,
});

const v2DailyRecord = closed(["date", "opening_cash", "opening_positions", "opening_marks", "opening_nav", "closing_cash", "closing_positions", "close_marks", "closing_nav", "target_lineage"], {
  date,
  opening_cash: fixed4,
  opening_positions: { type: "array", items: v2Position, maxItems: 11 },
  opening_marks: { type: "array", items: v2CloseMark, maxItems: 11 },
  opening_nav: fixed4,
  closing_cash: fixed4,
  closing_positions: { type: "array", items: v2Position, maxItems: 11 },
  close_marks: { type: "array", items: v2CloseMark, maxItems: 11 },
  closing_nav: fixed4,
  target_lineage: { anyOf: [v2TargetAllocation, { type: "null" }] },
});

const v2Metrics = closed(["daily_returns", "period_return", "ending_nav", "ending_cash", "ending_holdings", "cumulative_costs", "buy_notional", "sell_notional", "maximum_drawdown", "period_turnover", "fill_count"], {
  daily_returns: { type: "array", items: closed(["session_date", "value"], { session_date: date, value: ratio12 }), minItems: 2, maxItems: 2252 },
  period_return: ratio12,
  ending_nav: fixed4,
  ending_cash: fixed4,
  ending_holdings: { type: "array", items: v2Position, maxItems: 11 },
  cumulative_costs: fixed4,
  buy_notional: fixed4,
  sell_notional: fixed4,
  maximum_drawdown: ratio12,
  period_turnover: ratio12,
  fill_count: { type: "integer", minimum: 0 },
});

const v2LedgerRun = closed(["profile", "initial_capital", "daily_records", "rebalances", "market_sessions", "target_observations", "metrics"], {
  profile: enumString("ILLUSTRATIVE_10BP_PER_SIDE_V1", "ZERO_COST_REFERENCE_V1"),
  initial_capital: { ...fixed4, const: "10000000.0000" },
  daily_records: { type: "array", items: v2DailyRecord, minItems: 2, maxItems: 2252 },
  rebalances: { type: "array", items: v2Rebalance, maxItems: 2252 },
  market_sessions: { type: "array", items: v2MarketSession, minItems: 2, maxItems: 2252 },
  target_observations: { type: "array", items: v2TargetObservation, minItems: 1, maxItems: 2252 },
  metrics: v2Metrics,
});

const resultSchemasV2 = {
  SimulationResultV2: closed(["schema", "contract_version", "contract_sha256", "input", "strategy", "engine", "schedule", "cost_profile", "ledger", "benchmark", "benchmark_return_difference"], {
    schema: { type: "string", const: "owner-beta-price-only-result-v2" },
    contract_version: { type: "string", const: "owner-beta-price-only-simulation-v2-draft-1" },
    contract_sha256: { type: "string", const: "sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3" },
    input: closed(["candidate_content_sha256", "artifact_manifest_sha256", "stage5_manifest_sha256", "action_manifest_sha256", "approval_registry_sha256", "bars_sha256", "ownership_scope", "return_method", "vendor_snapshot", "strict_pit"], {
      candidate_content_sha256: { type: "string", const: "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d" },
      artifact_manifest_sha256: { type: "string", const: "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6" },
      stage5_manifest_sha256: { type: "string", const: "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a" },
      action_manifest_sha256: { type: "string", const: "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550" },
      approval_registry_sha256: { type: "string", const: "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707" },
      bars_sha256: { type: "string", const: "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe" },
      ownership_scope: { type: "string", const: "OWNER_ONLY" },
      return_method: { type: "string", const: "PRICE_RETURN_ONLY" },
      vendor_snapshot: { type: "boolean", const: true },
      strict_pit: { type: "boolean", const: false },
    }),
    strategy: v2StrategyIdentitySchema,
    engine: ref("SimulationEngineIdentity"),
    schedule: v2Schedule,
    cost_profile: enumString("ILLUSTRATIVE_10BP_PER_SIDE_V1", "ZERO_COST_REFERENCE_V1"),
    ledger: v2LedgerRun,
    benchmark: v2LedgerRun,
    benchmark_return_difference: ratio12,
  }),
};

export const SIMULATION_SCHEMAS = {
  OwnerBetaSimulationRunBody: {
    oneOf: [ref("OwnerBetaSimulationRunBodyV1"), ref("OwnerBetaSimulationRunBodyV2")],
  },
  OwnerBetaSimulationRunBodyV1: closed(["strategy_config_id"], {
    strategy_config_id: { type: "string", format: "uuid" },
    contract_version: enumString("owner-beta-price-only-simulation-v1-draft-1"),
    start_date: { ...date, description: "Omit to use the approved-calendar default; explicit null is invalid." },
    end_date: { ...date, description: "Omit to use the approved-calendar default; explicit null is invalid." },
  }),
  OwnerBetaSimulationRunBodyV2: closed(["strategy_config_id", "contract_version"], {
    strategy_config_id: { type: "string", format: "uuid" },
    contract_version: { type: "string", const: "owner-beta-price-only-simulation-v2-draft-1" },
    start_date: { ...date, description: "Omit to use the approved-calendar default; explicit null is invalid." },
    end_date: { ...date, description: "Omit to use the approved-calendar default; explicit null is invalid." },
  }),
  OwnerBetaSimulationEnqueued: closed(["run_id", "job_id", "status"], {
    run_id: { type: "string", format: "uuid" },
    job_id: { type: "string", format: "uuid" },
    status: { type: "string", const: "PENDING" },
  }),
  OwnerBetaSimulationRun: closed(["id", "job_id", "strategy_config_id", "strategy_id", "strategy_version", "start_date", "end_date", "evaluation_session_count", "status", "audience", "capability", "vendor_snapshot", "strict_pit", "contract_version", "contract_document_sha256", "result_schema_version", "created_at", "started_at", "finished_at", "updated_at", "error_code"], {
    id: { type: "string", format: "uuid" },
    job_id: { type: "string", format: "uuid" },
    strategy_config_id: { type: "string", format: "uuid" },
    strategy_id: enumString("buy_and_hold", "trend_following", "relative_momentum", "dual_momentum", "inverse_volatility"),
    strategy_version: { type: "string", const: "1.0.0" },
    start_date: date,
    end_date: date,
    evaluation_session_count: { type: "integer", minimum: 2, maximum: 2252 },
    status: enumString("PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"),
    audience: { type: "string", const: "OWNER_ONLY" },
    capability: { type: "string", const: "PRICE_RETURN_ONLY" },
    vendor_snapshot: { type: "boolean", const: true },
    strict_pit: { type: "boolean", const: false },
    contract_version: enumString("owner-beta-price-only-simulation-v1-draft-1", "owner-beta-price-only-simulation-v2-draft-1"),
    contract_document_sha256: digest,
    result_schema_version: enumString("owner-beta-price-only-result-v1", "owner-beta-price-only-result-v2"),
    created_at: timestamp,
    started_at: { anyOf: [timestamp, { type: "null" }] },
    finished_at: { anyOf: [timestamp, { type: "null" }] },
    updated_at: timestamp,
    error_code: { anyOf: [enumString("SIMULATION_INPUT_INVALID", "SIMULATION_ENTITLEMENT_DENIED", "SIMULATION_PROVENANCE_INVALID", "SIMULATION_RESULT_INVALID", "SIMULATION_COMPUTATION_TIMED_OUT", "SIMULATION_COMPUTATION_FAILED", "SIMULATION_INFRASTRUCTURE_UNAVAILABLE", "SIMULATION_ATTEMPTS_EXHAUSTED", "CANCELED"), { type: "null" }] },
  }),
  OwnerBetaSimulationPage: closed(["items", "next_cursor", "has_more"], {
    items: { type: "array", items: ref("OwnerBetaSimulationRun") },
    next_cursor: { anyOf: [{ type: "string" }, { type: "null" }] },
    has_more: { type: "boolean" },
  }),
  OwnerBetaSimulationCancel: closed(["run_id", "job_id", "status"], {
    run_id: { type: "string", format: "uuid" },
    job_id: { type: "string", format: "uuid" },
    status: { type: "string", const: "CANCEL_REQUESTED", description: "Acknowledgement only; GET returns current queue/run truth." },
  }),
  OwnerBetaSimulationProfile: closed(["result_content_sha256", "result"], {
    result_content_sha256: digest,
    result: { oneOf: [ref("SimulationResult"), ref("SimulationResultV2")] },
  }),
  OwnerBetaSimulationPair: closed(["run_id", "status", "illustrative_10bp_per_side", "zero_cost_reference"], {
    run_id: { type: "string", format: "uuid" },
    status: { type: "string", const: "SUCCEEDED" },
    illustrative_10bp_per_side: ref("OwnerBetaSimulationProfile"),
    zero_cost_reference: ref("OwnerBetaSimulationProfile"),
  }),
  ...resultSchemas,
  ...resultSchemasV2,
};
