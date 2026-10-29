import type { components } from "@lagrange/api-contract";
import { z } from "zod";
import {
  OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256,
  OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
  OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION,
  type OwnerBetaSimulationResultV2,
  type OwnerBetaSimulationStrategyId,
  ownerBetaSimulationDefaultStrategyId,
  ownerBetaSimulationResultV2Schema,
  ownerBetaSimulationRunBodyV2Schema,
  ownerBetaSimulationRunV2Schema,
  ownerBetaSimulationV2PairBindingsMatch,
} from "./owner-beta-simulation-v2-contracts";
import type { StrategyConfigModel } from "./paper-contracts";

export const OWNER_BETA_SIMULATION_BASE_PATH = "/api/v1/backtests/owner-beta/price-only";
export const OWNER_BETA_SIMULATION_RUNS_PATH = `${OWNER_BETA_SIMULATION_BASE_PATH}/runs` as const;

export const OWNER_BETA_SIMULATION_CONTRACT_VERSION = "owner-beta-price-only-simulation-v1-draft-1";
export const OWNER_BETA_SIMULATION_RESULT_SCHEMA_VERSION = "owner-beta-price-only-result-v1";
export const OWNER_BETA_SIMULATION_CONTRACT_SHA256 =
  "sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd";
export const OWNER_BETA_SIMULATION_INITIAL_CAPITAL = "10000000.0000";
export const OWNER_BETA_SIMULATION_DEFAULT_START_DATE = "2017-06-22";
export const OWNER_BETA_SIMULATION_DEFAULT_END_DATE = "2026-08-28";

export const OWNER_BETA_SIMULATION_INSTRUMENTS = [
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
] as const;

const APPROVED_INPUT_PINS = {
  action_manifest_sha256: "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
  approval_registry_sha256:
    "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707",
  artifact_manifest_sha256:
    "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
  candidate_content_sha256:
    "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
  stage5_manifest_sha256: "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
} as const;

const APPROVED_BARS_SHA256 =
  "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe";
const DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/;
const GIT_REVISION_PATTERN = /^[0-9a-f]{40}$/;
const MONEY_4_PATTERN = /^(?:0|[1-9][0-9]*)\.[0-9]{4}$/;
const RATIO_12_PATTERN = /^-?(?:0|[1-9][0-9]*)\.[0-9]{12}$/;
const ASCII_IDENTIFIER_PATTERN = /^[A-Za-z0-9._+-]+$/;
const APPROVED_METRICS = [
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
] as const;
const APPROVED_PHASE_ORDER = [
  "INITIAL_CASH",
  "PRE_OPEN_MARK",
  "SELL",
  "BUY",
  "CLOSE_MARK",
  "SIGNAL_FOR_NEXT_SESSION",
] as const;

const uuidSchema = z.uuid();
const dateSchema = z.iso.date();
const digestSchema = z.string().regex(DIGEST_PATTERN);
const money4Schema = z.string().regex(MONEY_4_PATTERN);
const ratio12Schema = z
  .string()
  .regex(RATIO_12_PATTERN)
  .refine((value) => value !== "-0.000000000000", "negative zero is not canonical");
const nonnegativeIntegerSchema = z
  .number()
  .int()
  .nonnegative()
  .refine(Number.isSafeInteger, "integer exceeds the safe JSON range")
  .refine((value) => !Object.is(value, -0), "negative zero is not canonical");
const positiveIntegerSchema = nonnegativeIntegerSchema.refine((value) => value > 0);

export const ownerBetaSimulationRunBodyV1Schema = z
  .object({
    strategy_config_id: uuidSchema,
    contract_version: z.literal(OWNER_BETA_SIMULATION_CONTRACT_VERSION).optional(),
    start_date: dateSchema.optional(),
    end_date: dateSchema.optional(),
  })
  .strict()
  .superRefine((body, context) => {
    if (
      body.start_date !== undefined &&
      body.end_date !== undefined &&
      body.start_date > body.end_date
    ) {
      context.addIssue({ code: "custom", message: "start_date must not follow end_date" });
    }
  })
  .transform(({ strategy_config_id, start_date, end_date }) => ({
    strategy_config_id,
    ...(start_date === undefined ? {} : { start_date }),
    ...(end_date === undefined ? {} : { end_date }),
  })) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationRunBody"]>;

export const ownerBetaSimulationRunBodySchema = z.union([
  ownerBetaSimulationRunBodyV1Schema,
  ownerBetaSimulationRunBodyV2Schema,
]) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationRunBody"]>;

export type OwnerBetaSimulationRunBodyV1 = z.infer<typeof ownerBetaSimulationRunBodyV1Schema>;
export type OwnerBetaSimulationRunBodyV2 = z.infer<typeof ownerBetaSimulationRunBodyV2Schema>;
export type OwnerBetaSimulationRunBody = z.infer<typeof ownerBetaSimulationRunBodySchema>;

export const ownerBetaSimulationEnqueuedSchema = z
  .object({
    run_id: uuidSchema,
    job_id: uuidSchema,
    status: z.literal("PENDING"),
  })
  .strict() satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationEnqueued"]>;

export type OwnerBetaSimulationEnqueued = z.infer<typeof ownerBetaSimulationEnqueuedSchema>;

export const ownerBetaSimulationRunStatusSchema = z.enum([
  "PENDING",
  "RUNNING",
  "SUCCEEDED",
  "FAILED",
  "CANCELED",
]);
export type OwnerBetaSimulationRunStatus = z.infer<typeof ownerBetaSimulationRunStatusSchema>;

const runErrorCodeSchema = z.enum([
  "SIMULATION_INPUT_INVALID",
  "SIMULATION_ENTITLEMENT_DENIED",
  "SIMULATION_PROVENANCE_INVALID",
  "SIMULATION_RESULT_INVALID",
  "SIMULATION_COMPUTATION_TIMED_OUT",
  "SIMULATION_COMPUTATION_FAILED",
  "SIMULATION_INFRASTRUCTURE_UNAVAILABLE",
  "SIMULATION_ATTEMPTS_EXHAUSTED",
  "CANCELED",
]);

export const ownerBetaSimulationRunSchema = z
  .object({
    id: uuidSchema,
    job_id: uuidSchema,
    strategy_config_id: uuidSchema,
    strategy_id: z.literal("trend_following"),
    strategy_version: z.literal("1.0.0"),
    start_date: dateSchema,
    end_date: dateSchema,
    evaluation_session_count: nonnegativeIntegerSchema.min(2).max(2252),
    status: ownerBetaSimulationRunStatusSchema,
    audience: z.literal("OWNER_ONLY"),
    capability: z.literal("PRICE_RETURN_ONLY"),
    vendor_snapshot: z.literal(true),
    strict_pit: z.literal(false),
    contract_version: z.literal(OWNER_BETA_SIMULATION_CONTRACT_VERSION),
    contract_document_sha256: z.literal(OWNER_BETA_SIMULATION_CONTRACT_SHA256),
    result_schema_version: z.literal(OWNER_BETA_SIMULATION_RESULT_SCHEMA_VERSION),
    created_at: z.iso.datetime(),
    started_at: z.iso.datetime().nullable(),
    finished_at: z.iso.datetime().nullable(),
    updated_at: z.iso.datetime(),
    error_code: runErrorCodeSchema.nullable(),
  })
  .strict()
  .superRefine((run, context) => {
    if (run.start_date > run.end_date) {
      context.addIssue({ code: "custom", message: "run window is reversed" });
    }
    if (run.created_at > run.updated_at) {
      context.addIssue({ code: "custom", message: "run timestamps are inconsistent" });
    }
  }) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationRun"]>;

export type OwnerBetaSimulationRun = z.infer<typeof ownerBetaSimulationRunSchema>;

export const ownerBetaSimulationVersionedRunSchema = z.discriminatedUnion("contract_version", [
  ownerBetaSimulationRunSchema,
  ownerBetaSimulationRunV2Schema,
]);

export type OwnerBetaSimulationVersionedRun = z.infer<typeof ownerBetaSimulationVersionedRunSchema>;

export const ownerBetaSimulationPageSchema = z
  .object({
    items: z.array(ownerBetaSimulationVersionedRunSchema).max(100),
    next_cursor: z.string().min(1).nullable(),
    has_more: z.boolean(),
  })
  .strict()
  .superRefine((page, context) => {
    if (page.has_more !== (page.next_cursor !== null)) {
      context.addIssue({ code: "custom", message: "cursor and has_more disagree" });
    }
  }) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationPage"]>;

export type OwnerBetaSimulationPage = z.infer<typeof ownerBetaSimulationPageSchema>;

export const ownerBetaSimulationCancelSchema = z
  .object({
    run_id: uuidSchema,
    job_id: uuidSchema,
    status: z.literal("CANCEL_REQUESTED"),
  })
  .strict() satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationCancel"]>;

export const ownerBetaSimulationVersionedResultSchema = z.union([
  z.lazy(() => simulationResultSchema),
  ownerBetaSimulationResultV2Schema,
]) satisfies z.ZodType<
  components["schemas"]["SimulationResult"] | components["schemas"]["SimulationResultV2"]
>;

export type OwnerBetaSimulationVersionedResult = z.infer<
  typeof ownerBetaSimulationVersionedResultSchema
>;

export const ownerBetaSimulationProfileSchema = z
  .object({
    result_content_sha256: digestSchema,
    result: z.lazy(() => ownerBetaSimulationVersionedResultSchema),
  })
  .strict() satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationProfile"]>;

export type OwnerBetaSimulationProfile = z.infer<typeof ownerBetaSimulationProfileSchema>;

export const simulationLedgerPhaseSchema = z.enum(APPROVED_PHASE_ORDER);
export type SimulationLedgerPhase = z.infer<typeof simulationLedgerPhaseSchema>;

export const simulationContractIdentitySchema = z
  .object({
    contract_version: z.literal(OWNER_BETA_SIMULATION_CONTRACT_VERSION),
    contract_document_sha256: z.literal(OWNER_BETA_SIMULATION_CONTRACT_SHA256),
    result_schema_version: z.literal(OWNER_BETA_SIMULATION_RESULT_SCHEMA_VERSION),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationContractIdentity"]>;

export const simulationApprovedPinsSchema = z
  .object({
    candidate_content_sha256: z.literal(APPROVED_INPUT_PINS.candidate_content_sha256),
    artifact_manifest_sha256: z.literal(APPROVED_INPUT_PINS.artifact_manifest_sha256),
    stage5_manifest_sha256: z.literal(APPROVED_INPUT_PINS.stage5_manifest_sha256),
    action_manifest_sha256: z.literal(APPROVED_INPUT_PINS.action_manifest_sha256),
    approval_registry_sha256: z.literal(APPROVED_INPUT_PINS.approval_registry_sha256),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationApprovedPins"]>;

export const simulationEvidenceLabelsSchema = z
  .object({
    data_scope: z.literal("OWNER_ONLY"),
    return_basis: z.literal("PRICE_RETURN_ONLY"),
    vendor_snapshot: z.literal(true),
    strict_pit: z.literal(false),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationEvidenceLabels"]>;

export const simulationInputIdentitySchema = z
  .object({
    pins: simulationApprovedPinsSchema,
    bars_sha256: z.literal(APPROVED_BARS_SHA256),
    approved_start_date: z.literal("2016-08-29"),
    approved_end_date: z.literal("2026-08-28"),
    session_count: z.literal(2452),
    bar_count: z.literal(26972),
    instruments: z.array(z.enum(OWNER_BETA_SIMULATION_INSTRUMENTS)).length(11),
    labels: simulationEvidenceLabelsSchema,
  })
  .strict()
  .superRefine((input, context) => {
    if (!sameValue(input.instruments, OWNER_BETA_SIMULATION_INSTRUMENTS)) {
      context.addIssue({
        code: "custom",
        message: "approved instruments must be unique and sorted",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationInputIdentity"]>;

export const simulationEngineIdentitySchema = z
  .object({
    git_commit: z.string().regex(GIT_REVISION_PATTERN),
    binary_sha256: digestSchema,
    toolchain: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    target: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    profile: z.string().min(1).max(64).regex(ASCII_IDENTIFIER_PATTERN),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationEngineIdentity"]>;

export const simulationTrendConfigSchema = z
  .object({
    benchmark_instrument: z.literal("069500.KRX"),
    fast_ma: z.literal(50),
    slow_ma: z.literal(200),
    target_rule: z.literal("BUY_069500_IF_TREND_50_LT_TREND_200_ELSE_CASH"),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationTrendConfig"]>;

export const simulationFactorDefinitionSchema = z
  .object({
    version: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    sha256: digestSchema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationFactorDefinition"]>;

export const simulationStrategyIdentitySchema = z
  .object({
    strategy_id: z.literal("trend_following"),
    version: z.literal("1.0.0"),
    config: simulationTrendConfigSchema,
    strategy_snapshot_sha256: digestSchema,
    factor_definition: simulationFactorDefinitionSchema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationStrategyIdentity"]>;

export const simulationSignalExecutionSchema = z
  .object({ signal_date: dateSchema, execution_date: dateSchema })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationSignalExecution"]>;

function isWeekend(value: string): boolean {
  const weekday = new Date(`${value}T00:00:00Z`).getUTCDay();
  return weekday === 0 || weekday === 6;
}

function ascendingWeekdays(values: readonly string[]): boolean {
  return values.every((value, index) => {
    const previous = values[index - 1];
    return !isWeekend(value) && (previous === undefined || previous < value);
  });
}

export const simulationScheduleSchema = z
  .object({
    warmup_sessions: z.array(dateSchema).min(200).max(2450),
    signal_execution: z.array(simulationSignalExecutionSchema).max(2252),
    evaluation_sessions: z.array(dateSchema).min(2).max(2252),
    phase_order: z.array(simulationLedgerPhaseSchema).length(6),
  })
  .strict()
  .superRefine((schedule, context) => {
    const allDates = [...schedule.warmup_sessions, ...schedule.evaluation_sessions];
    if (allDates.length > 2452) {
      context.addIssue({ code: "custom", message: "schedule exceeds the approved session set" });
    }
    if (!ascendingWeekdays(allDates)) {
      context.addIssue({
        code: "custom",
        message: "schedule dates must be ordered approved weekdays",
      });
    }
    if (!sameValue(schedule.phase_order, APPROVED_PHASE_ORDER)) {
      context.addIssue({ code: "custom", message: "phase order differs from the frozen contract" });
    }
    if (schedule.signal_execution.length !== schedule.evaluation_sessions.length) {
      context.addIssue({
        code: "custom",
        message: "each evaluation session needs one signal pair",
      });
    }
    schedule.signal_execution.forEach((pair, index) => {
      const expectedSignal = allDates[schedule.warmup_sessions.length + index - 1];
      if (expectedSignal === undefined || pair.signal_date !== expectedSignal) {
        context.addIssue({
          code: "custom",
          path: ["signal_execution", index],
          message: "signal date does not match the preceding approved session",
        });
      }
      if (pair.execution_date !== schedule.evaluation_sessions[index]) {
        context.addIssue({
          code: "custom",
          path: ["signal_execution", index],
          message: "execution date does not match the evaluation session",
        });
      }
    });
    const firstWarmup = schedule.warmup_sessions[0];
    const lastEvaluation = schedule.evaluation_sessions.at(-1);
    if (
      (firstWarmup !== undefined && firstWarmup < "2016-08-29") ||
      (lastEvaluation !== undefined && lastEvaluation > "2026-08-28")
    ) {
      context.addIssue({
        code: "custom",
        message: "schedule falls outside the approved input range",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationSchedule"]>;

export const simulationInstrumentTargetSchema = z
  .object({
    instrument_id: z.enum(OWNER_BETA_SIMULATION_INSTRUMENTS),
    weight_ppm: nonnegativeIntegerSchema.max(1_000_000),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationInstrumentTarget"]>;

export const simulationSignalSchema = z
  .object({
    signal_date: dateSchema,
    factor_snapshot_sha256: digestSchema,
    target_snapshot_sha256: digestSchema,
    targets: z.array(simulationInstrumentTargetSchema).length(11),
    cash_weight_ppm: nonnegativeIntegerSchema.max(1_000_000),
  })
  .strict()
  .superRefine((signal, context) => {
    const ids = signal.targets.map((target) => target.instrument_id);
    if (!sameValue(ids, OWNER_BETA_SIMULATION_INSTRUMENTS)) {
      context.addIssue({
        code: "custom",
        message: "signal targets must match the ordered ETF11 universe",
      });
    }
    const total = signal.targets.reduce(
      (sum, target) => sum + target.weight_ppm,
      signal.cash_weight_ppm,
    );
    if (total !== 1_000_000) {
      context.addIssue({
        code: "custom",
        message: "signal target weights must total one million ppm",
      });
    }
    const benchmarkWeight = signal.targets.find(
      (target) => target.instrument_id === "069500.KRX",
    )?.weight_ppm;
    if (
      (benchmarkWeight !== 1_000_000 || signal.cash_weight_ppm !== 0) &&
      (benchmarkWeight !== 0 || signal.cash_weight_ppm !== 1_000_000)
    ) {
      context.addIssue({ code: "custom", message: "trend target must be benchmark or cash" });
    }
    if (
      signal.targets.some(
        (target) => target.instrument_id !== "069500.KRX" && target.weight_ppm !== 0,
      )
    ) {
      context.addIssue({ code: "custom", message: "only the fixed trend instrument may be held" });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationSignal"]>;

export const simulationPrecisionConventionSchema = z
  .object({
    price_scale: z.literal(4),
    money_scale: z.literal(4),
    ratio_scale: z.literal(12),
    quantity_scale: z.literal(0),
    target_weight_scale_ppm: z.literal(1_000_000),
    ratio_rounding: z.literal("TIES_TO_EVEN"),
    negative_zero: z.literal("NORMALIZE_TO_ZERO"),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationPrecisionConvention"]>;

export const simulationPortfolioConventionSchema = z
  .object({
    initial_capital: z.literal(OWNER_BETA_SIMULATION_INITIAL_CAPITAL),
    currency: z.literal("KRW"),
    quantity_unit: z.literal("WHOLE_SHARES"),
    share_sizing: z.literal("MAXIMUM_COST_INCLUSIVE_WHOLE_SHARES"),
    target_transition: z.literal("FULL_POSITION_OR_CASH_ONLY_ON_TARGET_CHANGE"),
    residual_cash: z.literal("NO_ADDITIONAL_PURCHASE_AFTER_INITIAL_FILL"),
    borrowing: z.literal(false),
    short_selling: z.literal(false),
    cash_interest: z.literal("NONE"),
    additional_cash_flows: z.literal(false),
    dividends: z.literal("EXCLUDED"),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationPortfolioConvention"]>;

export const simulationPricingConventionSchema = z
  .object({
    signal_price: z.literal("ADJUSTED_CLOSE"),
    execution_price: z.literal("RAW_OPEN"),
    valuation_price: z.literal("RAW_CLOSE"),
    corporate_actions: z.literal("REJECT_NONCASH_AND_REQUIRE_RAW_ADJUSTED_OHLC_EQUAL"),
    missing_data: z.literal("FAIL_WHOLE_RUN_NO_CARRY_FORWARD"),
    volume: z.literal("REQUIRE_POSITIVE_VOLUME_WHEN_TRADING"),
    market_impact: z.literal("NONE"),
    partial_fills: z.literal(false),
    trade_order: z.literal("SELL_BEFORE_BUY"),
    ending: z.literal("FINAL_CLOSE_NO_LIQUIDATION"),
    warmup_returns_included: z.literal(false),
    market_timezone: z.literal("Asia/Seoul"),
    timestamp_fields_included: z.literal(false),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationPricingConvention"]>;

export const simulationCostConventionSchema = z
  .object({
    profile: z.enum(["ILLUSTRATIVE_10BP_PER_SIDE_V1", "ZERO_COST_REFERENCE_V1"]),
    per_side_rate_numerator: nonnegativeIntegerSchema,
    per_side_rate_denominator: positiveIntegerSchema,
    minimum_cost: z.literal("0.0000"),
    separate_tax: z.literal("0.0000"),
    price_slippage: z.literal("0.0000"),
    rounding: z.literal("CEILING_TO_MONEY_SCALE"),
  })
  .strict()
  .superRefine((cost, context) => {
    const numerator = cost.profile === "ILLUSTRATIVE_10BP_PER_SIDE_V1" ? 1 : 0;
    const denominator = cost.profile === "ILLUSTRATIVE_10BP_PER_SIDE_V1" ? 1000 : 1;
    if (
      cost.per_side_rate_numerator !== numerator ||
      cost.per_side_rate_denominator !== denominator
    ) {
      context.addIssue({ code: "custom", message: "cost rate disagrees with the named profile" });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationCostConvention"]>;

export const simulationBenchmarkConventionSchema = z
  .object({
    instrument_id: z.literal("069500.KRX"),
    policy: z.literal("BUY_AND_HOLD_AT_START_OPEN"),
    initial_purchase: z.literal("COST_INCLUSIVE_MAXIMUM_WHOLE_SHARES"),
    rebalancing: z.literal(false),
    additional_cash_purchases: z.literal(false),
    forced_final_liquidation: z.literal(false),
    same_cost_profile: z.literal(true),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationBenchmarkConvention"]>;

export const simulationTurnoverConventionSchema = z
  .object({
    formula: z.literal("SUM_DAILY_BUY_PLUS_SELL_OVER_TWICE_PRETRADE_OPEN_NAV"),
    include_initial_trade: z.literal(true),
    fees_and_deposits_included: z.literal(false),
    annualized: z.literal(false),
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationTurnoverConvention"]>;

export const simulationMetricsConventionSchema = z
  .object({
    minimum_evaluation_sessions: z.literal(2),
    allowed_metrics: z.array(z.enum(APPROVED_METRICS)).length(APPROVED_METRICS.length),
    cagr_included: z.literal(false),
    annualized_volatility_included: z.literal(false),
    sharpe_included: z.literal(false),
    sortino_included: z.literal(false),
    win_rate_included: z.literal(false),
    trade_return_included: z.literal(false),
    monthly_metrics_included: z.literal(false),
  })
  .strict()
  .superRefine((metrics, context) => {
    if (!sameValue(metrics.allowed_metrics, APPROVED_METRICS)) {
      context.addIssue({
        code: "custom",
        message: "metric list differs from the frozen allowlist",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationMetricsConvention"]>;

export const simulationConventionsSchema = z
  .object({
    precision: simulationPrecisionConventionSchema,
    portfolio: simulationPortfolioConventionSchema,
    pricing: simulationPricingConventionSchema,
    cost: simulationCostConventionSchema,
    benchmark: simulationBenchmarkConventionSchema,
    turnover: simulationTurnoverConventionSchema,
    metrics: simulationMetricsConventionSchema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationConventions"]>;

const instrumentLedgerIdSchema = z.string().regex(/^[0-9]{6}\.KRX$/);

export const simulationTradeEventSchema = z
  .object({
    instrument_id: instrumentLedgerIdSchema,
    side: z.enum(["BUY", "SELL"]),
    quantity: positiveIntegerSchema,
    price: money4Schema,
    notional: money4Schema,
    cost: money4Schema,
    cash_after: money4Schema,
    position_after: nonnegativeIntegerSchema,
  })
  .strict()
  .superRefine((event, context) => {
    if (event.instrument_id !== "069500.KRX") {
      context.addIssue({
        code: "custom",
        message: "trade uses an instrument outside the fixed strategy",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationTradeEvent"]>;

export const simulationPositionSchema = z
  .object({
    instrument_id: instrumentLedgerIdSchema,
    quantity: positiveIntegerSchema,
    price: money4Schema,
    market_value: money4Schema,
  })
  .strict()
  .superRefine((position, context) => {
    if (position.instrument_id !== "069500.KRX") {
      context.addIssue({
        code: "custom",
        message: "position uses an instrument outside the fixed strategy",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationPosition"]>;

export const simulationLedgerRecordSchema = z
  .object({
    session_date: dateSchema,
    phase: simulationLedgerPhaseSchema,
    events: z.array(simulationTradeEventSchema).max(1),
    unfilled_reason: z.enum([
      "NOT_APPLICABLE",
      "NO_ORDER_REQUIRED",
      "ZERO_SHARE_QUANTITY",
      "FILLED",
    ]),
    cash: money4Schema,
    positions: z.array(simulationPositionSchema).max(1),
    nav: money4Schema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationLedgerRecord"]>;

export const simulationDailyReturnSchema = z
  .object({ session_date: dateSchema, value: ratio12Schema })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationDailyReturn"]>;

export const simulationHoldingSchema = z
  .object({
    instrument_id: instrumentLedgerIdSchema,
    quantity: positiveIntegerSchema,
  })
  .strict()
  .superRefine((holding, context) => {
    if (holding.instrument_id !== "069500.KRX") {
      context.addIssue({
        code: "custom",
        message: "holding uses an instrument outside the fixed strategy",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationHolding"]>;

export const simulationLedgerMetricsSchema = z
  .object({
    daily_returns: z.array(simulationDailyReturnSchema).max(2252),
    period_return: ratio12Schema,
    ending_nav: money4Schema,
    ending_cash: money4Schema,
    ending_holdings: z.array(simulationHoldingSchema).max(1),
    cumulative_costs: money4Schema,
    buy_notional: money4Schema,
    sell_notional: money4Schema,
    maximum_drawdown: ratio12Schema,
    period_turnover: ratio12Schema,
    fill_count: nonnegativeIntegerSchema,
  })
  .strict()
  .superRefine((metrics, context) => {
    if (
      ratioUnits(metrics.maximum_drawdown) < 0n ||
      ratioUnits(metrics.maximum_drawdown) > 1_000_000_000_000n
    ) {
      context.addIssue({
        code: "custom",
        path: ["maximum_drawdown"],
        message: "maximum drawdown is outside its ratio range",
      });
    }
    if (ratioUnits(metrics.period_turnover) < 0n) {
      context.addIssue({
        code: "custom",
        path: ["period_turnover"],
        message: "period turnover cannot be negative",
      });
    }
    if (ratioUnits(metrics.period_return) < -1_000_000_000_000n) {
      context.addIssue({
        code: "custom",
        path: ["period_return"],
        message: "period return is below total loss",
      });
    }
    if (metrics.daily_returns.some((daily) => ratioUnits(daily.value) <= -1_000_000_000_000n)) {
      context.addIssue({
        code: "custom",
        path: ["daily_returns"],
        message: "daily return is at or below total loss",
      });
    }
    if (metrics.ending_holdings.length > 1) {
      context.addIssue({
        code: "custom",
        path: ["ending_holdings"],
        message: "fixed strategy has at most one holding",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["SimulationLedgerMetrics"]>;

export type SimulationLedgerMetrics = z.infer<typeof simulationLedgerMetricsSchema>;

export const simulationStrategyLedgerSchema = z
  .object({
    initial_cash: money4Schema,
    records: z.array(simulationLedgerRecordSchema).max(11260),
    metrics: simulationLedgerMetricsSchema,
    benchmark_return_difference: ratio12Schema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationStrategyLedger"]>;

export const simulationBenchmarkLedgerSchema = z
  .object({
    initial_cash: money4Schema,
    records: z.array(simulationLedgerRecordSchema).max(11260),
    metrics: simulationLedgerMetricsSchema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["SimulationBenchmarkLedger"]>;

export const simulationResultSchema: z.ZodType<components["schemas"]["SimulationResult"]> = z
  .object({
    contract: simulationContractIdentitySchema,
    input: simulationInputIdentitySchema,
    engine: simulationEngineIdentitySchema,
    strategy: simulationStrategyIdentitySchema,
    schedule: simulationScheduleSchema,
    conventions: simulationConventionsSchema,
    signals: z.array(simulationSignalSchema).max(2252),
    strategy_ledger: simulationStrategyLedgerSchema,
    benchmark_ledger: simulationBenchmarkLedgerSchema,
  })
  .strict()
  .superRefine((result, context) => {
    const evaluation = result.schedule.evaluation_sessions;
    if (result.signals.length !== result.schedule.signal_execution.length) {
      context.addIssue({
        code: "custom",
        path: ["signals"],
        message: "signals do not match the frozen schedule",
      });
    }
    result.signals.forEach((signal, index) => {
      const scheduled = result.schedule.signal_execution[index];
      if (scheduled === undefined || signal.signal_date !== scheduled.signal_date) {
        context.addIssue({
          code: "custom",
          path: ["signals", index],
          message: "signal date differs from its schedule pair",
        });
      }
    });
    if (result.schedule.warmup_sessions.length + evaluation.length > result.input.session_count) {
      context.addIssue({
        code: "custom",
        path: ["schedule"],
        message: "schedule exceeds the approved input session count",
      });
    }
    if (
      result.strategy_ledger.initial_cash !== OWNER_BETA_SIMULATION_INITIAL_CAPITAL ||
      result.benchmark_ledger.initial_cash !== OWNER_BETA_SIMULATION_INITIAL_CAPITAL
    ) {
      context.addIssue({
        code: "custom",
        message: "ledger initial cash differs from the approved capital",
      });
    }
    checkLedgerBinding(result.strategy_ledger, evaluation, context, ["strategy_ledger"]);
    checkLedgerBinding(result.benchmark_ledger, evaluation, context, ["benchmark_ledger"]);
    const initialCashUnits = moneyUnits(OWNER_BETA_SIMULATION_INITIAL_CAPITAL);
    const strategyNavUnits = moneyUnits(result.strategy_ledger.metrics.ending_nav);
    const benchmarkNavUnits = moneyUnits(result.benchmark_ledger.metrics.ending_nav);
    const strategyReturn = (strategyNavUnits - initialCashUnits) * 10n;
    const benchmarkReturn = (benchmarkNavUnits - initialCashUnits) * 10n;
    if (ratioUnits(result.strategy_ledger.metrics.period_return) !== strategyReturn) {
      context.addIssue({
        code: "custom",
        path: ["strategy_ledger", "metrics", "period_return"],
        message: "period return differs from the final NAV",
      });
    }
    if (ratioUnits(result.benchmark_ledger.metrics.period_return) !== benchmarkReturn) {
      context.addIssue({
        code: "custom",
        path: ["benchmark_ledger", "metrics", "period_return"],
        message: "period return differs from the final NAV",
      });
    }
    if (
      ratioUnits(result.strategy_ledger.benchmark_return_difference) !==
      (strategyNavUnits - benchmarkNavUnits) * 10n
    ) {
      context.addIssue({
        code: "custom",
        path: ["strategy_ledger", "benchmark_return_difference"],
        message: "benchmark difference differs from final NAVs",
      });
    }
    if (!sameRecordSchedule(result.strategy_ledger.records, result.benchmark_ledger.records)) {
      context.addIssue({
        code: "custom",
        message: "strategy and benchmark record schedules differ",
      });
    }
  });

export type SimulationResult = z.infer<typeof simulationResultSchema>;

export const ownerBetaSimulationPairSchema = z
  .object({
    run_id: uuidSchema,
    status: z.literal("SUCCEEDED"),
    illustrative_10bp_per_side: ownerBetaSimulationProfileSchema,
    zero_cost_reference: ownerBetaSimulationProfileSchema,
  })
  .strict()
  .superRefine((pair, context) => {
    const illustrative = pair.illustrative_10bp_per_side.result;
    const zeroCost = pair.zero_cost_reference.result;
    const illustrativeIsV2 = isSimulationResultV2(illustrative);
    const zeroCostIsV2 = isSimulationResultV2(zeroCost);
    if (illustrativeIsV2 !== zeroCostIsV2) {
      context.addIssue({ code: "custom", message: "result pair mixes contract versions" });
      return;
    }
    if (illustrativeIsV2 && zeroCostIsV2) {
      if (illustrative.cost_profile !== "ILLUSTRATIVE_10BP_PER_SIDE_V1") {
        context.addIssue({
          code: "custom",
          path: ["illustrative_10bp_per_side"],
          message: "illustrative slot contains the wrong cost profile",
        });
      }
      if (zeroCost.cost_profile !== "ZERO_COST_REFERENCE_V1") {
        context.addIssue({
          code: "custom",
          path: ["zero_cost_reference"],
          message: "zero-cost slot contains the wrong cost profile",
        });
      }
      if (!ownerBetaSimulationV2PairBindingsMatch(illustrative, zeroCost)) {
        context.addIssue({
          code: "custom",
          message: "v2 cost profiles do not share the same approved run path",
        });
      }
      return;
    }
    if (illustrativeIsV2 || zeroCostIsV2) return;

    if (illustrative.conventions.cost.profile !== "ILLUSTRATIVE_10BP_PER_SIDE_V1") {
      context.addIssue({
        code: "custom",
        path: ["illustrative_10bp_per_side"],
        message: "illustrative slot contains the wrong cost profile",
      });
    }
    if (zeroCost.conventions.cost.profile !== "ZERO_COST_REFERENCE_V1") {
      context.addIssue({
        code: "custom",
        path: ["zero_cost_reference"],
        message: "zero-cost slot contains the wrong cost profile",
      });
    }
    if (!sameValue(commonPairBinding(illustrative), commonPairBinding(zeroCost))) {
      context.addIssue({
        code: "custom",
        message: "cost profiles do not share the same approved run binding",
      });
    }
  }) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationPair"]>;

export type OwnerBetaSimulationPair = z.infer<typeof ownerBetaSimulationPairSchema>;

function isSimulationResultV2(
  result: OwnerBetaSimulationVersionedResult,
): result is OwnerBetaSimulationResultV2 {
  return "schema" in result && result.schema === OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION;
}

export function ownerBetaSimulationPairMatchesRun(
  pair: OwnerBetaSimulationPair,
  run: OwnerBetaSimulationVersionedRun,
): boolean {
  if (pair.run_id !== run.id || run.status !== "SUCCEEDED") return false;
  const results = [pair.illustrative_10bp_per_side.result, pair.zero_cost_reference.result];
  if (run.contract_version === OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION) {
    return results.every((result) => {
      if (!isSimulationResultV2(result)) return false;
      const schedule = result.schedule;
      return (
        result.contract_version === run.contract_version &&
        result.contract_sha256 === OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256 &&
        result.schema === run.result_schema_version &&
        result.strategy.strategy === run.strategy_id &&
        result.strategy.version === run.strategy_version &&
        schedule.evaluation_sessions.length === run.evaluation_session_count &&
        schedule.evaluation_sessions[0] === run.start_date &&
        schedule.evaluation_sessions.at(-1) === run.end_date
      );
    });
  }
  return results.every((result) => {
    if (isSimulationResultV2(result)) return false;
    const schedule = result.schedule;
    return (
      result.contract.contract_version === run.contract_version &&
      result.contract.contract_document_sha256 === run.contract_document_sha256 &&
      result.contract.result_schema_version === run.result_schema_version &&
      result.strategy.strategy_id === run.strategy_id &&
      result.strategy.version === run.strategy_version &&
      schedule.evaluation_sessions.length === run.evaluation_session_count &&
      schedule.evaluation_sessions[0] === run.start_date &&
      schedule.evaluation_sessions.at(-1) === run.end_date
    );
  });
}

export function ownerBetaSimulationRunMatchesRequest(
  run: OwnerBetaSimulationVersionedRun,
  request: OwnerBetaSimulationRunBody,
  runId: string,
  expectedStrategyId?: OwnerBetaSimulationStrategyId,
): boolean {
  if (run.id !== runId || run.strategy_config_id !== request.strategy_config_id) return false;
  if ("contract_version" in request) {
    return (
      request.contract_version === OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION &&
      run.contract_version === OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION &&
      run.result_schema_version === OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION &&
      run.contract_document_sha256 === OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256 &&
      expectedStrategyId !== undefined &&
      run.strategy_id === expectedStrategyId &&
      (request.start_date === undefined || run.start_date === request.start_date) &&
      (request.end_date === undefined || run.end_date === request.end_date)
    );
  }
  return (
    run.contract_version === OWNER_BETA_SIMULATION_CONTRACT_VERSION &&
    run.start_date === (request.start_date ?? OWNER_BETA_SIMULATION_DEFAULT_START_DATE) &&
    run.end_date === (request.end_date ?? OWNER_BETA_SIMULATION_DEFAULT_END_DATE)
  );
}

export type OwnerBetaSimulationRunsQuery = {
  readonly cursor?: string;
  readonly limit?: number;
};

export function ownerBetaSimulationRunsPath(query: OwnerBetaSimulationRunsQuery = {}): string {
  const limit = query.limit ?? 20;
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
    throw new RangeError("simulation page limit must be between 1 and 100");
  }
  const search = new URLSearchParams({ limit: String(limit) });
  if (query.cursor !== undefined) {
    if (query.cursor.length === 0) throw new RangeError("simulation cursor cannot be empty");
    search.set("cursor", query.cursor);
  }
  return `${OWNER_BETA_SIMULATION_RUNS_PATH}?${search.toString()}`;
}

export function ownerBetaSimulationRunPath(
  runId: string,
): `/api/v1/backtests/owner-beta/price-only/runs/${string}` {
  return `${OWNER_BETA_SIMULATION_RUNS_PATH}/${encodeURIComponent(uuidSchema.parse(runId))}`;
}

export function ownerBetaSimulationPairPath(runId: string): string {
  return `${ownerBetaSimulationRunPath(runId)}/pair`;
}

export function ownerBetaSimulationCancelPath(
  runId: string,
): `/api/v1/backtests/owner-beta/price-only/runs/${string}/cancel` {
  return `${ownerBetaSimulationRunPath(runId)}/cancel`;
}

export type OwnerBetaSimulationEligibleConfig = StrategyConfigModel & {
  readonly simulationStrategyId: OwnerBetaSimulationStrategyId;
};

export function eligibleOwnerBetaSimulationConfigs(
  configs: readonly StrategyConfigModel[],
): readonly OwnerBetaSimulationEligibleConfig[] {
  const eligible: OwnerBetaSimulationEligibleConfig[] = [];
  for (const config of configs) {
    if (!config.is_active) continue;
    const simulationStrategyId = ownerBetaSimulationDefaultStrategyId(
      config.strategy_id,
      config.strategy_version,
      config.config,
    );
    if (simulationStrategyId !== null) eligible.push({ ...config, simulationStrategyId });
  }
  return eligible;
}

function ratioUnits(value: string): bigint {
  const negative = value.startsWith("-");
  const digits = value.replace(/^-/, "").replace(".", "");
  const magnitude = BigInt(digits);
  return negative ? -magnitude : magnitude;
}

function moneyUnits(value: string): bigint {
  return BigInt(value.replace(".", ""));
}

function sameValue(left: unknown, right: unknown): boolean {
  return stableValue(left) === stableValue(right);
}

function stableValue(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableValue).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableValue(record[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value) ?? "undefined";
}

function commonPairBinding(result: SimulationResult): unknown {
  const { cost: _cost, ...nonCostConventions } = result.conventions;
  return {
    contract: result.contract,
    input: result.input,
    engine: result.engine,
    strategy: result.strategy,
    schedule: result.schedule,
    conventions: nonCostConventions,
    signals: result.signals,
  };
}

function sameRecordSchedule(
  left: readonly Pick<z.infer<typeof simulationLedgerRecordSchema>, "phase" | "session_date">[],
  right: readonly Pick<z.infer<typeof simulationLedgerRecordSchema>, "phase" | "session_date">[],
): boolean {
  return (
    left.length === right.length &&
    left.every((record, index) => {
      const other = right[index];
      return (
        other !== undefined &&
        record.session_date === other.session_date &&
        record.phase === other.phase
      );
    })
  );
}

function checkLedgerBinding(
  ledger:
    | z.infer<typeof simulationStrategyLedgerSchema>
    | z.infer<typeof simulationBenchmarkLedgerSchema>,
  evaluationDates: readonly string[],
  context: z.RefinementCtx,
  path: readonly (string | number)[],
): void {
  const expected: Array<{ session_date: string; phase: SimulationLedgerPhase }> = [];
  for (const [index, sessionDate] of evaluationDates.entries()) {
    if (index === 0) expected.push({ session_date: sessionDate, phase: "INITIAL_CASH" });
    expected.push({ session_date: sessionDate, phase: "PRE_OPEN_MARK" });
    expected.push({ session_date: sessionDate, phase: "SELL" });
    expected.push({ session_date: sessionDate, phase: "BUY" });
    expected.push({ session_date: sessionDate, phase: "CLOSE_MARK" });
    if (index + 1 < evaluationDates.length) {
      expected.push({ session_date: sessionDate, phase: "SIGNAL_FOR_NEXT_SESSION" });
    }
  }
  if (!sameRecordSchedule(ledger.records, expected)) {
    context.addIssue({
      code: "custom",
      path: [...path, "records"],
      message: "ledger records do not match the date-only phase schedule",
    });
  }
  if (ledger.metrics.daily_returns.length !== evaluationDates.length) {
    context.addIssue({
      code: "custom",
      path: [...path, "metrics", "daily_returns"],
      message: "daily return count differs from evaluation sessions",
    });
  }
  ledger.metrics.daily_returns.forEach((daily, index) => {
    if (daily.session_date !== evaluationDates[index]) {
      context.addIssue({
        code: "custom",
        path: [...path, "metrics", "daily_returns", index],
        message: "daily return date differs from evaluation session",
      });
    }
  });
  const lastClose = [...ledger.records].reverse().find((record) => record.phase === "CLOSE_MARK");
  if (
    lastClose === undefined ||
    lastClose.nav !== ledger.metrics.ending_nav ||
    lastClose.cash !== ledger.metrics.ending_cash ||
    !sameValue(
      lastClose.positions.map(({ instrument_id, quantity }) => ({ instrument_id, quantity })),
      ledger.metrics.ending_holdings,
    )
  ) {
    context.addIssue({
      code: "custom",
      path: [...path, "metrics"],
      message: "ending metrics differ from the final close mark",
    });
  }
}
