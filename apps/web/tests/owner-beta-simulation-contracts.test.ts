import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import {
  formatSimulationMoney,
  formatSimulationPercent,
} from "@/components/backtests/owner-beta-simulation-report";
import { ownerBetaSimulationAutomaticPollAllowed } from "@/components/backtests/owner-beta-simulation-status";
import {
  eligibleOwnerBetaSimulationConfigs,
  OWNER_BETA_SIMULATION_CONTRACT_SHA256,
  OWNER_BETA_SIMULATION_CONTRACT_VERSION,
  OWNER_BETA_SIMULATION_DEFAULT_END_DATE,
  OWNER_BETA_SIMULATION_DEFAULT_START_DATE,
  OWNER_BETA_SIMULATION_RESULT_SCHEMA_VERSION,
  ownerBetaSimulationRunBodySchema,
  ownerBetaSimulationRunMatchesRequest,
  ownerBetaSimulationRunSchema,
  ownerBetaSimulationRunsPath,
  simulationConventionsSchema,
  simulationResultSchema,
} from "@/lib/products/owner-beta-simulation-contracts";

const simulationFixtureUrl = pathToFileURL(
  `${process.cwd()}/tests/e2e/support/owner-beta-simulation-fixture.mjs`,
);
const {
  OWNER_BETA_SIMULATION_CONFIG_ID,
  OWNER_BETA_SIMULATION_RUNS_PATH,
  OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID,
  ownerBetaSimulationResponse,
  resetOwnerBetaSimulationFixture,
} = await import(simulationFixtureUrl.href);

const CONFIG_ID = "00000000-0000-4000-8000-000000000801";
const RUN_ID = "00000000-0000-4000-8000-000000000811";
const JOB_ID = "00000000-0000-4000-8000-000000000821";
function run(overrides: Record<string, unknown> = {}) {
  return {
    audience: "OWNER_ONLY",
    capability: "PRICE_RETURN_ONLY",
    contract_document_sha256: OWNER_BETA_SIMULATION_CONTRACT_SHA256,
    contract_version: OWNER_BETA_SIMULATION_CONTRACT_VERSION,
    created_at: "2026-08-29T06:00:00Z",
    end_date: "2026-08-28",
    error_code: null,
    evaluation_session_count: 2,
    finished_at: null,
    id: RUN_ID,
    job_id: JOB_ID,
    result_schema_version: OWNER_BETA_SIMULATION_RESULT_SCHEMA_VERSION,
    started_at: null,
    start_date: "2026-08-27",
    status: "PENDING",
    strategy_config_id: CONFIG_ID,
    strategy_id: "trend_following",
    strategy_version: "1.0.0",
    strict_pit: false,
    updated_at: "2026-08-29T06:00:00Z",
    vendor_snapshot: true,
    ...overrides,
  };
}

const conventions = {
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
    per_side_rate_denominator: 1000,
    per_side_rate_numerator: 1,
    price_slippage: "0.0000",
    profile: "ILLUSTRATIVE_10BP_PER_SIDE_V1",
    rounding: "CEILING_TO_MONEY_SCALE",
    separate_tax: "0.0000",
  },
  metrics: {
    allowed_metrics: [
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
    ],
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
};

describe("owner-beta simulation contracts", () => {
  it("accepts only the closed request body and independently optional ISO dates", () => {
    expect(ownerBetaSimulationRunBodySchema.parse({ strategy_config_id: CONFIG_ID })).toEqual({
      strategy_config_id: CONFIG_ID,
    });
    expect(
      ownerBetaSimulationRunBodySchema.parse({
        strategy_config_id: CONFIG_ID,
        start_date: undefined,
        end_date: undefined,
      }),
    ).toStrictEqual({ strategy_config_id: CONFIG_ID });
    expect(
      ownerBetaSimulationRunBodySchema.parse({
        strategy_config_id: CONFIG_ID,
        start_date: "2026-08-27",
        end_date: "2026-08-28",
      }),
    ).toStrictEqual({
      strategy_config_id: CONFIG_ID,
      start_date: "2026-08-27",
      end_date: "2026-08-28",
    });
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        strategy_config_id: CONFIG_ID,
        start_date: "2026-08-27",
      }).success,
    ).toBe(true);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        strategy_config_id: CONFIG_ID,
        end_date: "2026-08-28",
      }).success,
    ).toBe(true);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        strategy_config_id: CONFIG_ID,
        start_date: "2026-08-29",
        end_date: "2026-08-28",
      }).success,
    ).toBe(false);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        strategy_config_id: CONFIG_ID,
        end_date: null,
      }).success,
    ).toBe(false);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        strategy_config_id: CONFIG_ID,
        owner_user_id: CONFIG_ID,
      }).success,
    ).toBe(false);
  });

  it("filters configurations by exact active trend configuration without creating defaults", () => {
    const configs = [
      {
        config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
        created_at: "2026-08-19T06:00:00Z",
        id: CONFIG_ID,
        is_active: true,
        strategy_id: "trend_following",
        strategy_version: "1.0.0",
        updated_at: "2026-08-19T06:00:00Z",
      },
      {
        config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200, extra: true },
        created_at: "2026-08-19T06:00:00Z",
        id: "00000000-0000-4000-8000-000000000802",
        is_active: true,
        strategy_id: "trend_following",
        strategy_version: "1.0.0",
        updated_at: "2026-08-19T06:00:00Z",
      },
      {
        config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
        created_at: "2026-08-19T06:00:00Z",
        id: "00000000-0000-4000-8000-000000000803",
        is_active: false,
        strategy_id: "trend_following",
        strategy_version: "1.0.0",
        updated_at: "2026-08-19T06:00:00Z",
      },
      {
        config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
        created_at: "2026-08-19T06:00:00Z",
        id: "00000000-0000-4000-8000-000000000804",
        is_active: true,
        strategy_id: "trend_following",
        strategy_version: "2.0.0",
        updated_at: "2026-08-19T06:00:00Z",
      },
    ];
    expect(eligibleOwnerBetaSimulationConfigs(configs).map((config) => config.id)).toEqual([
      CONFIG_ID,
    ]);
    expect(eligibleOwnerBetaSimulationConfigs([])).toEqual([]);
  });

  it("uses a signed cursor unchanged and bounds requested history pages", () => {
    const path = ownerBetaSimulationRunsPath({ cursor: "opaque.signed.cursor+/=", limit: 20 });
    const parsed = new URL(path, "https://local.invalid");
    expect(parsed.pathname).toBe("/api/v1/backtests/owner-beta/price-only/runs");
    expect(parsed.searchParams.get("cursor")).toBe("opaque.signed.cursor+/=");
    expect(parsed.searchParams.get("limit")).toBe("20");
    expect(ownerBetaSimulationRunsPath()).toBe(
      "/api/v1/backtests/owner-beta/price-only/runs?limit=20",
    );
    expect(() => ownerBetaSimulationRunsPath({ limit: 101 })).toThrow(RangeError);
    expect(() => ownerBetaSimulationRunsPath({ cursor: "" })).toThrow(RangeError);
  });

  it("binds selected metadata to the submitted intent and fixed omitted-date defaults", () => {
    const value = ownerBetaSimulationRunSchema.parse(run());
    expect(
      ownerBetaSimulationRunMatchesRequest(
        ownerBetaSimulationRunSchema.parse(
          run({
            start_date: OWNER_BETA_SIMULATION_DEFAULT_START_DATE,
            end_date: OWNER_BETA_SIMULATION_DEFAULT_END_DATE,
            evaluation_session_count: 2_252,
          }),
        ),
        { strategy_config_id: CONFIG_ID },
        RUN_ID,
      ),
    ).toBe(true);
    expect(
      ownerBetaSimulationRunMatchesRequest(
        value,
        { strategy_config_id: CONFIG_ID, start_date: "2026-08-27", end_date: "2026-08-28" },
        RUN_ID,
      ),
    ).toBe(true);
    expect(
      ownerBetaSimulationRunMatchesRequest(
        value,
        { strategy_config_id: "00000000-0000-4000-8000-000000000802" },
        RUN_ID,
      ),
    ).toBe(false);
    expect(OWNER_BETA_SIMULATION_DEFAULT_START_DATE).toBe("2017-06-22");
    expect(OWNER_BETA_SIMULATION_DEFAULT_END_DATE).toBe("2026-08-28");
  });

  it("emits schema-valid ordered submitted metadata through the actual synthetic route", () => {
    resetOwnerBetaSimulationFixture();
    const scenario = { ownerBetaSimulation: "enabled" };
    const headers = { "x-csrf-token": "synthetic-csrf", "idempotency-key": "synthetic-run" };
    try {
      const enqueue = ownerBetaSimulationResponse({
        body: {
          end_date: "2026-08-28",
          start_date: "2026-08-27",
          strategy_config_id: OWNER_BETA_SIMULATION_CONFIG_ID,
        },
        headers,
        method: "POST",
        pathname: OWNER_BETA_SIMULATION_RUNS_PATH,
        query: "",
        scenario,
      });
      expect(enqueue?.status).toBe(202);

      const metadata = ownerBetaSimulationResponse({
        body: {},
        headers: {},
        method: "GET",
        pathname: `${OWNER_BETA_SIMULATION_RUNS_PATH}/${OWNER_BETA_SIMULATION_SUBMITTED_RUN_ID}`,
        query: "",
        scenario,
      });
      expect(metadata?.status).toBe(200);
      const submitted = ownerBetaSimulationRunSchema.parse(metadata?.body);
      expect(submitted.status).toBe("RUNNING");
      expect(submitted.created_at).toBe("2026-08-29T06:10:00Z");
      expect(Date.parse(submitted.created_at)).toBeLessThanOrEqual(
        Date.parse(submitted.started_at ?? ""),
      );
      expect(Date.parse(submitted.started_at ?? "")).toBeLessThanOrEqual(
        Date.parse(submitted.updated_at),
      );
      expect(submitted.finished_at).toBeNull();
    } finally {
      resetOwnerBetaSimulationFixture();
    }
  });

  it("strictly rejects extra convention and result fields", () => {
    expect(simulationConventionsSchema.safeParse(conventions).success).toBe(true);
    expect(
      simulationConventionsSchema.safeParse({
        ...conventions,
        precision: { ...conventions.precision, unsupported_clock: "09:00" },
      }).success,
    ).toBe(false);
    expect(simulationResultSchema.safeParse({ unsupported_result_field: true }).success).toBe(
      false,
    );
  });

  it("formats percentages and KRW with BigInt ties-to-even and normalized negative zero", () => {
    expect(formatSimulationPercent("-0.000999000000")).toBe("−0.10%");
    expect(formatSimulationPercent("0.000050000000")).toBe("0.00%");
    expect(formatSimulationPercent("0.000150000000")).toBe("0.02%");
    expect(formatSimulationPercent("-0.000050000000")).toBe("0.00%");
    expect(formatSimulationMoney("1250000.0050")).toBe("₩1,250,000.00");
    expect(formatSimulationMoney("1250000.0150")).toBe("₩1,250,000.02");
    expect(formatSimulationMoney("-0.0050")).toBe("₩0.00");
  });

  it("caps automatic poll eligibility at 60 attempts per selection", () => {
    expect(ownerBetaSimulationAutomaticPollAllowed(0)).toBe(true);
    expect(ownerBetaSimulationAutomaticPollAllowed(59)).toBe(true);
    expect(ownerBetaSimulationAutomaticPollAllowed(60)).toBe(false);
    expect(ownerBetaSimulationAutomaticPollAllowed(-1)).toBe(false);
  });
});
