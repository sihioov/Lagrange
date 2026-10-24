export const OWNER_BETA_SIMULATION_BASE_PATH = "/api/v1/backtests/owner-beta/price-only";
export const OWNER_BETA_SIMULATION_RUNS_PATH = `${OWNER_BETA_SIMULATION_BASE_PATH}/runs`;
export const OWNER_BETA_SIMULATION_CONFIG_ID = "00000000-0000-4000-8000-000000000801";
export const OWNER_BETA_SIMULATION_RUN_ID = "00000000-0000-4000-8000-000000000811";
export const OWNER_BETA_SIMULATION_JOB_ID = "00000000-0000-4000-8000-000000000821";
export const OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID = "00000000-0000-4000-8000-000000000812";
export const OWNER_BETA_SIMULATION_SUBMITTED_JOB_ID = "00000000-0000-4000-8000-000000000822";
const FAILED_RUN_ID = "00000000-0000-4000-8000-000000000813";
const FAILED_JOB_ID = "00000000-0000-4000-8000-000000000823";
const CANCELED_RUN_ID = "00000000-0000-4000-8000-000000000814";
const CANCELED_JOB_ID = "00000000-0000-4000-8000-000000000824";

const DEFAULT_START_DATE = "2017-06-22";
const DEFAULT_END_DATE = "2026-08-28";
const SHA = `sha256:${"a".repeat(64)}`;
const BARS_SHA = "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe";
const CONTRACT_SHA = "sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd";
const INSTRUMENTS = Object.freeze([
  "069500.KRX",
  "102110.KRX",
  "114260.KRX",
  "132030.KRX",
  "133690.KRX",
  "143850.KRX",
  "148070.KRX",
  "153130.KRX",
  "192090.KRX",
  "195930.KRX",
  "229200.KRX",
]);
const PHASE_ORDER = Object.freeze([
  "INITIAL_CASH",
  "PRE_OPEN_MARK",
  "SELL",
  "BUY",
  "CLOSE_MARK",
  "SIGNAL_FOR_NEXT_SESSION",
]);
const ALLOWED_METRICS = Object.freeze([
  "daily_returns",
  "period_return",
  "ending_nav",
  "ending_cash",
  "ending_holdings",
  "cumulative_costs",
  "buy_notional",
  "sell_notional",
  "maximum_drawdown",
  "period_turnover",
  "fill_count",
  "benchmark_return_difference",
]);

let submittedBody = null;
let submittedRunId = null;

export function resetOwnerBetaSimulationFixture() {
  submittedBody = null;
  submittedRunId = null;
}

export function ownerBetaSimulationConfig() {
  return {
    config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
    created_at: "2026-08-19T06:00:00Z",
    id: OWNER_BETA_SIMULATION_CONFIG_ID,
    is_active: true,
    strategy_id: "trend_following",
    strategy_version: "1.0.0",
    updated_at: "2026-08-19T06:00:00Z",
  };
}

function weekdaySessionsBefore(startDate, count) {
  const dates = [];
  const current = new Date(`${startDate}T00:00:00Z`);
  current.setUTCDate(current.getUTCDate() - 1);
  while (dates.length < count) {
    const weekday = current.getUTCDay();
    if (weekday !== 0 && weekday !== 6) dates.push(current.toISOString().slice(0, 10));
    current.setUTCDate(current.getUTCDate() - 1);
  }
  return dates.reverse();
}

function weekdaySessionsBetween(startDate, endDate) {
  const dates = [];
  const current = new Date(`${startDate}T00:00:00Z`);
  const end = new Date(`${endDate}T00:00:00Z`);
  while (current <= end) {
    const weekday = current.getUTCDay();
    if (weekday !== 0 && weekday !== 6) dates.push(current.toISOString().slice(0, 10));
    current.setUTCDate(current.getUTCDate() + 1);
  }
  return dates;
}

function targetForBenchmark() {
  return INSTRUMENTS.map((instrument_id, index) => ({
    instrument_id,
    weight_ppm: index === 0 ? 1_000_000 : 0,
  }));
}

function makeSchedule(startDate, endDate) {
  const evaluation = weekdaySessionsBetween(startDate, endDate);
  if (evaluation.length < 2 || evaluation.length > 2252) {
    throw new Error("Synthetic owner-beta simulation fixture requires 2–2,252 weekday sessions");
  }
  const warmup = weekdaySessionsBefore(startDate, 200);
  const allDates = [...warmup, ...evaluation];
  return {
    evaluation_sessions: evaluation,
    phase_order: [...PHASE_ORDER],
    signal_execution: evaluation.map((execution_date, index) => ({
      execution_date,
      signal_date: allDates[warmup.length + index - 1],
    })),
    warmup_sessions: warmup,
  };
}

function moneyRecord({
  cash,
  date,
  events = [],
  phase,
  quantity = 0,
  nav,
  reason = "NOT_APPLICABLE",
}) {
  const positions =
    quantity === 0
      ? []
      : [
          {
            instrument_id: "069500.KRX",
            market_value: `${quantity * 1000}.0000`,
            price: "1000.0000",
            quantity,
          },
        ];
  return {
    cash,
    events,
    nav,
    phase,
    positions,
    session_date: date,
    unfilled_reason: reason,
  };
}

function tradeEvent({ cashAfter, cost, notional, quantity }) {
  return {
    cash_after: cashAfter,
    cost,
    instrument_id: "069500.KRX",
    notional,
    position_after: quantity,
    price: "1000.0000",
    quantity,
    side: "BUY",
  };
}

function ledger(profile, schedule) {
  const illustrative = profile === "ILLUSTRATIVE_10BP_PER_SIDE_V1";
  const quantity = illustrative ? 9_990 : 10_000;
  const cashAfter = illustrative ? "10.0000" : "0.0000";
  const nav = illustrative ? "9990010.0000" : "10000000.0000";
  const notional = illustrative ? "9990000.0000" : "10000000.0000";
  const cost = illustrative ? "9990.0000" : "0.0000";
  const dailyValue = illustrative ? "-0.000999000000" : "0.000000000000";
  const records = [
    moneyRecord({
      cash: "10000000.0000",
      date: schedule.evaluation_sessions[0],
      nav: "10000000.0000",
      phase: "INITIAL_CASH",
    }),
  ];
  schedule.evaluation_sessions.forEach((date, index) => {
    const first = index === 0;
    const cash = first ? "10000000.0000" : cashAfter;
    const quantityBeforeFill = first ? 0 : quantity;
    const navBeforeFill = first ? "10000000.0000" : nav;
    records.push(
      moneyRecord({
        cash,
        date,
        nav: navBeforeFill,
        phase: "PRE_OPEN_MARK",
        quantity: quantityBeforeFill,
      }),
    );
    records.push(
      moneyRecord({
        cash,
        date,
        nav: navBeforeFill,
        phase: "SELL",
        quantity: quantityBeforeFill,
        reason: "NO_ORDER_REQUIRED",
      }),
    );
    if (first) {
      records.push(
        moneyRecord({
          cash: cashAfter,
          date,
          events: [tradeEvent({ cashAfter, cost, notional, quantity })],
          nav,
          phase: "BUY",
          quantity,
          reason: "FILLED",
        }),
      );
    } else {
      records.push(
        moneyRecord({ cash, date, nav, phase: "BUY", quantity, reason: "NO_ORDER_REQUIRED" }),
      );
    }
    records.push(moneyRecord({ cash: cashAfter, date, nav, phase: "CLOSE_MARK", quantity }));
    if (index + 1 < schedule.evaluation_sessions.length) {
      records.push(
        moneyRecord({ cash: cashAfter, date, nav, phase: "SIGNAL_FOR_NEXT_SESSION", quantity }),
      );
    }
  });
  return {
    initial_cash: "10000000.0000",
    metrics: {
      buy_notional: notional,
      cumulative_costs: cost,
      daily_returns: schedule.evaluation_sessions.map((session_date, index) => ({
        session_date,
        value: index === 0 ? dailyValue : "0.000000000000",
      })),
      ending_cash: cashAfter,
      ending_holdings: [{ instrument_id: "069500.KRX", quantity }],
      ending_nav: nav,
      fill_count: 1,
      maximum_drawdown: illustrative ? "0.000999000000" : "0.000000000000",
      period_return: dailyValue,
      period_turnover: illustrative ? "0.499500000000" : "0.500000000000",
      sell_notional: "0.0000",
    },
    records,
  };
}

function simulationResult(profile, startDate, endDate) {
  const schedule = makeSchedule(startDate, endDate);
  const strategyLedger = ledger(profile, schedule);
  const benchmarkLedger = ledger(profile, schedule);
  return {
    benchmark_ledger: benchmarkLedger,
    contract: {
      contract_document_sha256: CONTRACT_SHA,
      contract_version: "owner-beta-price-only-simulation-v1-draft-1",
      result_schema_version: "owner-beta-price-only-result-v1",
    },
    conventions: {
      benchmark: {
        additional_cash_purchases: false,
        forced_final_liquidation: false,
        initial_purchase: "COST_INCLUSIVE_MAXIMUM_WHOLE_SHARES",
        instrument_id: "069500.KRX",
        policy: "BUY_AND_HOLD_AT_START_OPEN",
        rebalancing: false,
        same_cost_profile: true,
      },
      cost: {
        minimum_cost: "0.0000",
        per_side_rate_denominator: profile === "ILLUSTRATIVE_10BP_PER_SIDE_V1" ? 1000 : 1,
        per_side_rate_numerator: profile === "ILLUSTRATIVE_10BP_PER_SIDE_V1" ? 1 : 0,
        price_slippage: "0.0000",
        profile,
        rounding: "CEILING_TO_MONEY_SCALE",
        separate_tax: "0.0000",
      },
      metrics: {
        allowed_metrics: [...ALLOWED_METRICS],
        annualized_volatility_included: false,
        cagr_included: false,
        minimum_evaluation_sessions: 2,
        monthly_metrics_included: false,
        sharpe_included: false,
        sortino_included: false,
        trade_return_included: false,
        win_rate_included: false,
      },
      portfolio: {
        additional_cash_flows: false,
        borrowing: false,
        cash_interest: "NONE",
        currency: "KRW",
        dividends: "EXCLUDED",
        initial_capital: "10000000.0000",
        quantity_unit: "WHOLE_SHARES",
        residual_cash: "NO_ADDITIONAL_PURCHASE_AFTER_INITIAL_FILL",
        share_sizing: "MAXIMUM_COST_INCLUSIVE_WHOLE_SHARES",
        short_selling: false,
        target_transition: "FULL_POSITION_OR_CASH_ONLY_ON_TARGET_CHANGE",
      },
      precision: {
        money_scale: 4,
        negative_zero: "NORMALIZE_TO_ZERO",
        price_scale: 4,
        quantity_scale: 0,
        ratio_rounding: "TIES_TO_EVEN",
        ratio_scale: 12,
        target_weight_scale_ppm: 1_000_000,
      },
      pricing: {
        corporate_actions: "REJECT_NONCASH_AND_REQUIRE_RAW_ADJUSTED_OHLC_EQUAL",
        ending: "FINAL_CLOSE_NO_LIQUIDATION",
        execution_price: "RAW_OPEN",
        market_impact: "NONE",
        market_timezone: "Asia/Seoul",
        missing_data: "FAIL_WHOLE_RUN_NO_CARRY_FORWARD",
        partial_fills: false,
        signal_price: "ADJUSTED_CLOSE",
        trade_order: "SELL_BEFORE_BUY",
        timestamp_fields_included: false,
        valuation_price: "RAW_CLOSE",
        volume: "REQUIRE_POSITIVE_VOLUME_WHEN_TRADING",
        warmup_returns_included: false,
      },
      turnover: {
        annualized: false,
        fees_and_deposits_included: false,
        formula: "SUM_DAILY_BUY_PLUS_SELL_OVER_TWICE_PRETRADE_OPEN_NAV",
        include_initial_trade: true,
      },
    },
    engine: {
      binary_sha256: SHA,
      git_commit: "c".repeat(40),
      profile: "release",
      target: "x86_64-unknown-linux-gnu",
      toolchain: "rustc-1.92.0",
    },
    input: {
      approved_end_date: DEFAULT_END_DATE,
      approved_start_date: "2016-08-29",
      bar_count: 26972,
      bars_sha256: BARS_SHA,
      instruments: [...INSTRUMENTS],
      labels: {
        data_scope: "OWNER_ONLY",
        return_basis: "PRICE_RETURN_ONLY",
        strict_pit: false,
        vendor_snapshot: true,
      },
      pins: {
        action_manifest_sha256:
          "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
        approval_registry_sha256:
          "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707",
        artifact_manifest_sha256:
          "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
        candidate_content_sha256:
          "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
        stage5_manifest_sha256:
          "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
      },
      session_count: 2452,
    },
    schedule,
    signals: schedule.signal_execution.map(({ signal_date }) => ({
      factor_snapshot_sha256: SHA,
      signal_date,
      target_snapshot_sha256: SHA,
      targets: targetForBenchmark(),
      cash_weight_ppm: 0,
    })),
    strategy: {
      config: {
        benchmark_instrument: "069500.KRX",
        fast_ma: 50,
        slow_ma: 200,
        target_rule: "BUY_069500_IF_TREND_50_LT_TREND_200_ELSE_CASH",
      },
      factor_definition: { sha256: SHA, version: "trend-factor-v1" },
      strategy_id: "trend_following",
      strategy_snapshot_sha256: SHA,
      version: "1.0.0",
    },
    strategy_ledger: {
      ...strategyLedger,
      benchmark_return_difference: "0.000000000000",
    },
  };
}

export function ownerBetaSimulationPair(startDate = "2026-08-27", endDate = "2026-08-28") {
  return {
    illustrative_10bp_per_side: {
      result: simulationResult("ILLUSTRATIVE_10BP_PER_SIDE_V1", startDate, endDate),
      result_content_sha256: SHA,
    },
    run_id: OWNER_BETA_SIMULATION_RUN_ID,
    status: "SUCCEEDED",
    zero_cost_reference: {
      result: simulationResult("ZERO_COST_REFERENCE_V1", startDate, endDate),
      result_content_sha256: `sha256:${"b".repeat(64)}`,
    },
  };
}

function simulationRun({
  id = OWNER_BETA_SIMULATION_RUN_ID,
  jobId = OWNER_BETA_SIMULATION_JOB_ID,
  status = "SUCCEEDED",
  startDate = "2026-08-27",
  endDate = "2026-08-28",
  createdAt = "2026-08-29T06:00:00Z",
} = {}) {
  const terminal = status === "SUCCEEDED" || status === "FAILED" || status === "CANCELED";
  const createdAtMillis = Date.parse(createdAt);
  const startedAt = status === "PENDING" ? null : new Date(createdAtMillis + 30_000).toISOString();
  const finishedAt = terminal ? new Date(createdAtMillis + 60_000).toISOString() : null;
  const updatedAt =
    finishedAt ??
    (status === "RUNNING" ? new Date(createdAtMillis + 60_000).toISOString() : createdAt);
  return {
    audience: "OWNER_ONLY",
    capability: "PRICE_RETURN_ONLY",
    contract_document_sha256: CONTRACT_SHA,
    contract_version: "owner-beta-price-only-simulation-v1-draft-1",
    created_at: createdAt,
    end_date: endDate,
    error_code:
      status === "FAILED"
        ? "SIMULATION_COMPUTATION_FAILED"
        : status === "CANCELED"
          ? "CANCELED"
          : null,
    evaluation_session_count: weekdaySessionsBetween(startDate, endDate).length,
    finished_at: finishedAt,
    id,
    job_id: jobId,
    result_schema_version: "owner-beta-price-only-result-v1",
    started_at: startedAt,
    start_date: startDate,
    status,
    strategy_config_id: OWNER_BETA_SIMULATION_CONFIG_ID,
    strategy_id: "trend_following",
    strategy_version: "1.0.0",
    strict_pit: false,
    updated_at: updatedAt,
    vendor_snapshot: true,
  };
}

function error(status, code, message) {
  return {
    status,
    body: { error: { code, message, request_id: "request-synthetic-owner-beta-simulation" } },
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

export function ownerBetaSimulationResponse({ body, headers, method, pathname, query, scenario }) {
  if (
    pathname !== OWNER_BETA_SIMULATION_RUNS_PATH &&
    !pathname.startsWith(`${OWNER_BETA_SIMULATION_RUNS_PATH}/`)
  ) {
    return null;
  }
  const supportedScenarios = new Set([
    "enabled",
    "pages",
    "empty",
    "no_runs",
    "invalid_pair",
    "partial_pair",
    "wrong_label",
    "wrong_window",
    "wrong_pair_id",
    "not_ready",
    "unavailable",
    "foreign",
    "integrity_failed",
    "pending",
  ]);
  if (!supportedScenarios.has(scenario.ownerBetaSimulation)) {
    return error(403, "FORBIDDEN", "owner beta simulation fixture is disabled");
  }
  if (method === "GET" && scenario.entitlement === "blocked") {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "backtest entitlement is inactive");
  }
  if (method === "GET" && pathname === OWNER_BETA_SIMULATION_RUNS_PATH) {
    const cursor = new URLSearchParams(query).get("cursor");
    if (scenario.ownerBetaSimulation === "pages" && cursor === "qa.signed.cursor+one=") {
      return {
        status: 200,
        body: {
          has_more: true,
          items: [
            simulationRun({
              id: FAILED_RUN_ID,
              jobId: FAILED_JOB_ID,
              status: "FAILED",
              createdAt: "2026-08-28T06:00:00Z",
            }),
          ],
          next_cursor: "qa.signed.cursor+two=",
        },
      };
    }
    if (scenario.ownerBetaSimulation === "pages" && cursor === "qa.signed.cursor+two=") {
      return {
        status: 200,
        body: {
          has_more: false,
          items: [
            simulationRun({
              id: CANCELED_RUN_ID,
              jobId: CANCELED_JOB_ID,
              status: "CANCELED",
              createdAt: "2026-08-27T06:00:00Z",
            }),
          ],
          next_cursor: null,
        },
      };
    }
    if (cursor !== null) return error(400, "INVALID_CURSOR", "synthetic cursor is invalid");
    return {
      status: 200,
      body: {
        has_more: scenario.ownerBetaSimulation === "pages",
        items:
          scenario.ownerBetaSimulation === "no_runs"
            ? []
            : [
                simulationRun(
                  scenario.ownerBetaSimulation === "pending" ? { status: "PENDING" } : {},
                ),
              ],
        next_cursor: scenario.ownerBetaSimulation === "pages" ? "qa.signed.cursor+one=" : null,
      },
    };
  }
  if (method === "POST" && pathname === OWNER_BETA_SIMULATION_RUNS_PATH) {
    if (!mutationAuthorized(headers))
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    const allowed = new Set(["strategy_config_id", "start_date", "end_date"]);
    if (
      body === null ||
      typeof body !== "object" ||
      Object.keys(body).some((key) => !allowed.has(key)) ||
      body.strategy_config_id !== OWNER_BETA_SIMULATION_CONFIG_ID
    ) {
      return error(422, "INVALID_PARAMETER", "simulation request body is invalid");
    }
    submittedBody = body;
    submittedRunId = OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID;
    return {
      status: 202,
      body: {
        job_id: OWNER_BETA_SIMULATION_SUBMITTED_JOB_ID,
        run_id: submittedRunId,
        status: "PENDING",
      },
    };
  }
  if (
    method === "GET" &&
    pathname === `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_RUN_ID}`
  ) {
    if (scenario.ownerBetaSimulation === "foreign") {
      return error(404, "RESOURCE_NOT_FOUND", "synthetic simulation resource was not found");
    }
    if (scenario.ownerBetaSimulation === "integrity_failed") {
      return error(
        422,
        "RESULT_INTEGRITY_FAILED",
        "stored simulation result failed integrity checks",
      );
    }
    if (scenario.ownerBetaSimulation === "unavailable") {
      return error(
        503,
        "SERVICE_STATUS_UNAVAILABLE",
        "synthetic simulation service is unavailable",
      );
    }
    if (scenario.ownerBetaSimulation === "pending") {
      return { status: 200, body: simulationRun({ status: "PENDING" }) };
    }
    return { status: 200, body: simulationRun() };
  }
  if (
    method === "GET" &&
    pathname === `${OWNER_BETA_SIMULATION_RUNS_PATH}/${FAILED_RUN_ID}` &&
    scenario.ownerBetaSimulation === "pages"
  ) {
    return {
      status: 200,
      body: simulationRun({ id: FAILED_RUN_ID, jobId: FAILED_JOB_ID, status: "FAILED" }),
    };
  }
  if (
    method === "GET" &&
    pathname === `${OWNER_BETA_SIMULATION_RUNS_PATH}/${CANCELED_RUN_ID}` &&
    scenario.ownerBetaSimulation === "pages"
  ) {
    return {
      status: 200,
      body: simulationRun({ id: CANCELED_RUN_ID, jobId: CANCELED_JOB_ID, status: "CANCELED" }),
    };
  }
  if (
    method === "GET" &&
    pathname === `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID}` &&
    submittedRunId !== null
  ) {
    return {
      status: 200,
      body: simulationRun({
        createdAt: "2026-08-29T06:10:00Z",
        endDate: submittedBody?.end_date ?? DEFAULT_END_DATE,
        id: OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID,
        jobId: OWNER_BETA_SIMULATION_SUBMITTED_JOB_ID,
        startDate: submittedBody?.start_date ?? DEFAULT_START_DATE,
        status: "RUNNING",
      }),
    };
  }
  if (
    method === "GET" &&
    pathname === `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_RUN_ID}/pair`
  ) {
    if (scenario.ownerBetaSimulation === "not_ready") {
      return error(409, "SIMULATION_RESULT_NOT_READY", "complete simulation results are not ready");
    }
    const pair = ownerBetaSimulationPair();
    if (scenario.ownerBetaSimulation === "invalid_pair") {
      pair.zero_cost_reference.result.engine.git_commit = "d".repeat(40);
    }
    if (scenario.ownerBetaSimulation === "partial_pair") {
      delete pair.zero_cost_reference;
    }
    if (scenario.ownerBetaSimulation === "wrong_label") {
      pair.zero_cost_reference.result.input.labels.strict_pit = true;
    }
    if (scenario.ownerBetaSimulation === "wrong_window") {
      pair.zero_cost_reference.result = simulationResult(
        "ZERO_COST_REFERENCE_V1",
        "2026-08-26",
        "2026-08-27",
      );
    }
    if (scenario.ownerBetaSimulation === "wrong_pair_id") {
      pair.run_id = CANCELED_RUN_ID;
    }
    return { status: 200, body: pair };
  }
  const selectedPendingCancelPath = `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_RUN_ID}/cancel`;
  if (
    method === "POST" &&
    pathname === selectedPendingCancelPath &&
    scenario.ownerBetaSimulation === "pending"
  ) {
    if (!mutationAuthorized(headers))
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    if (Object.keys(body ?? {}).length !== 0)
      return error(422, "INVALID_PARAMETER", "cancel body must be empty");
    return {
      status: 200,
      body: {
        job_id: OWNER_BETA_SIMULATION_JOB_ID,
        run_id: OWNER_BETA_SIMULATION_RUN_ID,
        status: "CANCEL_REQUESTED",
      },
    };
  }
  const cancelPath = `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID}/cancel`;
  if (method === "POST" && pathname === cancelPath && submittedRunId !== null) {
    if (!mutationAuthorized(headers))
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    if (Object.keys(body ?? {}).length !== 0)
      return error(422, "INVALID_PARAMETER", "cancel body must be empty");
    return {
      status: 200,
      body: {
        job_id: OWNER_BETA_SIMULATION_SUBMITTED_JOB_ID,
        run_id: OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID,
        status: "CANCEL_REQUESTED",
      },
    };
  }
  return error(404, "RESOURCE_NOT_FOUND", "synthetic simulation resource was not found");
}
