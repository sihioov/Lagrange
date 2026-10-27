import type { components } from "@lagrange/api-contract";
import { z } from "zod";

export const OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION =
  "owner-beta-price-only-simulation-v2-draft-1";
export const OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION = "owner-beta-price-only-result-v2";
export const OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256 =
  "sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3";
export const OWNER_BETA_SIMULATION_V2_INITIAL_CAPITAL = "10000000.0000";

export const OWNER_BETA_SIMULATION_V2_STRATEGY_IDS = [
  "buy_and_hold",
  "trend_following",
  "relative_momentum",
  "dual_momentum",
  "inverse_volatility",
] as const;

export const OWNER_BETA_SIMULATION_V2_INSTRUMENTS = [
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

const DEFAULT_STRATEGY_CONFIGS = {
  buy_and_hold: {
    canonicalConfigJson:
      '{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}',
    snapshotSha256: "sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06",
  },
  trend_following: {
    canonicalConfigJson: '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}',
    snapshotSha256: "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0",
  },
  relative_momentum: {
    canonicalConfigJson: '{"lookback_months":12,"top_n":3}',
    snapshotSha256: "sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07",
  },
  dual_momentum: {
    canonicalConfigJson: '{"absolute_threshold":0.0,"lookback_months":12}',
    snapshotSha256: "sha256:ac6a1ac393e02952fe2db6308e5274436f766479f3e631ebe42e429a677426a6",
  },
  inverse_volatility: {
    canonicalConfigJson: '{"max_weight":0.3,"vol_window":60}',
    snapshotSha256: "sha256:6252f27903f5ea0a7755ea7fc6c05a935518a2dbe44edf371a1e71f127d1aa38",
  },
} as const;

export const ownerBetaSimulationV2StrategyIdSchema = z.enum(OWNER_BETA_SIMULATION_V2_STRATEGY_IDS);
export type OwnerBetaSimulationStrategyId = z.infer<typeof ownerBetaSimulationV2StrategyIdSchema>;

export function ownerBetaSimulationDefaultStrategyId(
  strategyId: string,
  version: string,
  config: unknown,
): OwnerBetaSimulationStrategyId | null {
  const parsedStrategy = ownerBetaSimulationV2StrategyIdSchema.safeParse(strategyId);
  if (!parsedStrategy.success || version !== "1.0.0") return null;

  switch (parsedStrategy.data) {
    case "buy_and_hold":
      return z
        .object({
          benchmark_instrument: z.literal("069500.KRX"),
          rebalance_cadence: z.literal("none"),
          target_weight: z.literal(1),
        })
        .strict()
        .safeParse(config).success
        ? parsedStrategy.data
        : null;
    case "trend_following":
      return z
        .object({
          benchmark_instrument: z.literal("069500.KRX"),
          fast_ma: z.literal(50),
          slow_ma: z.literal(200),
        })
        .strict()
        .safeParse(config).success
        ? parsedStrategy.data
        : null;
    case "relative_momentum":
      return z
        .object({ lookback_months: z.literal(12), top_n: z.literal(3) })
        .strict()
        .safeParse(config).success
        ? parsedStrategy.data
        : null;
    case "dual_momentum":
      return z
        .object({ absolute_threshold: z.literal(0), lookback_months: z.literal(12) })
        .strict()
        .safeParse(config).success
        ? parsedStrategy.data
        : null;
    case "inverse_volatility":
      return z
        .object({ max_weight: z.literal(0.3), vol_window: z.literal(60) })
        .strict()
        .safeParse(config).success
        ? parsedStrategy.data
        : null;
  }
}

const DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/;
const GIT_REVISION_PATTERN = /^[0-9a-f]{40}$/;
const ASCII_IDENTIFIER_PATTERN = /^[A-Za-z0-9._+-]+$/;
const MONEY_4_PATTERN = /^(?:0|[1-9][0-9]*)\.[0-9]{4}$/;
const RATIO_12_PATTERN = /^-?(?:0|[1-9][0-9]*)\.[0-9]{12}$/;
const MAX_I128 = (1n << 127n) - 1n;
const MIN_I128_MAGNITUDE = 1n << 127n;

const uuidSchema = z.uuid();
const dateSchema = z.iso.date();
const digestSchema = z.string().regex(DIGEST_PATTERN);
const wholeCountSchema = z
  .number()
  .int()
  .nonnegative()
  .refine(Number.isSafeInteger, "integer exceeds the safe JSON range")
  .refine((value) => !Object.is(value, -0), "negative zero is not canonical");
const positiveCountSchema = wholeCountSchema.min(1);
const money4Schema = z
  .string()
  .regex(MONEY_4_PATTERN)
  .refine((value) => BigInt(value.replace(".", "")) <= MAX_I128, "amount exceeds the Rust range");
const ratio12Schema = z
  .string()
  .regex(RATIO_12_PATTERN)
  .refine((value) => value !== "-0.000000000000", "negative zero is not canonical")
  .refine((value) => {
    const negative = value.startsWith("-");
    const units = BigInt(value.replace(/^-/, "").replace(".", ""));
    return negative ? units <= MIN_I128_MAGNITUDE : units <= MAX_I128;
  }, "ratio exceeds the Rust range");

function sameValue(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function isWeekend(value: string): boolean {
  const day = new Date(`${value}T00:00:00.000Z`).getUTCDay();
  return day === 0 || day === 6;
}

function orderedDates(values: readonly string[]): boolean {
  return values.every((value, index) => {
    const previous = values[index - 1];
    return !isWeekend(value) && (previous === undefined || previous < value);
  });
}

function orderedInstrumentIds(values: readonly { readonly instrument_id: string }[]): boolean {
  const order = new Map<string, number>(
    OWNER_BETA_SIMULATION_V2_INSTRUMENTS.map((instrument, index) => [instrument, index]),
  );
  return values.every((value, index) => {
    const currentOrder = order.get(value.instrument_id);
    const previous = values[index - 1];
    const previousOrder = previous === undefined ? undefined : order.get(previous.instrument_id);
    return (
      currentOrder !== undefined && (previousOrder === undefined || currentOrder > previousOrder)
    );
  });
}

function exactInventory(values: readonly { readonly instrument_id: string }[]): boolean {
  return (
    values.length === OWNER_BETA_SIMULATION_V2_INSTRUMENTS.length &&
    values.every(
      (value, index) => value.instrument_id === OWNER_BETA_SIMULATION_V2_INSTRUMENTS[index],
    )
  );
}

const instrumentIdSchema = z.enum(OWNER_BETA_SIMULATION_V2_INSTRUMENTS);
const costProfileSchema = z.enum(["ILLUSTRATIVE_10BP_PER_SIDE_V1", "ZERO_COST_REFERENCE_V1"]);
const decisionKindSchema = z.enum([
  "INITIAL_COMMON_SESSION",
  "DAILY_TREND_TRANSITION_CHECK",
  "MONTH_END_SIGNAL",
]);

const factorDefinitionSchema = z
  .object({
    name: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    version: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    sha256: digestSchema,
  })
  .strict();

const strategyIdentitySchema = z.discriminatedUnion("strategy", [
  z
    .object({
      strategy: z.literal("buy_and_hold"),
      version: z.literal("1.0.0"),
      canonical_config_json: z.literal(DEFAULT_STRATEGY_CONFIGS.buy_and_hold.canonicalConfigJson),
      snapshot_sha256: z.literal(DEFAULT_STRATEGY_CONFIGS.buy_and_hold.snapshotSha256),
      factors: z.array(z.never()).length(0),
    })
    .strict(),
  z
    .object({
      strategy: z.literal("trend_following"),
      version: z.literal("1.0.0"),
      canonical_config_json: z.literal(
        DEFAULT_STRATEGY_CONFIGS.trend_following.canonicalConfigJson,
      ),
      snapshot_sha256: z.literal(DEFAULT_STRATEGY_CONFIGS.trend_following.snapshotSha256),
      factors: z
        .tuple([
          factorDefinitionSchema.extend({ name: z.literal("trend_50") }),
          factorDefinitionSchema.extend({ name: z.literal("trend_200") }),
        ])
        .rest(z.never()),
    })
    .strict(),
  z
    .object({
      strategy: z.literal("relative_momentum"),
      version: z.literal("1.0.0"),
      canonical_config_json: z.literal(
        DEFAULT_STRATEGY_CONFIGS.relative_momentum.canonicalConfigJson,
      ),
      snapshot_sha256: z.literal(DEFAULT_STRATEGY_CONFIGS.relative_momentum.snapshotSha256),
      factors: z.tuple([factorDefinitionSchema.extend({ name: z.literal("momentum_12_1") })]),
    })
    .strict(),
  z
    .object({
      strategy: z.literal("dual_momentum"),
      version: z.literal("1.0.0"),
      canonical_config_json: z.literal(DEFAULT_STRATEGY_CONFIGS.dual_momentum.canonicalConfigJson),
      snapshot_sha256: z.literal(DEFAULT_STRATEGY_CONFIGS.dual_momentum.snapshotSha256),
      factors: z.tuple([factorDefinitionSchema.extend({ name: z.literal("return_12m") })]),
    })
    .strict(),
  z
    .object({
      strategy: z.literal("inverse_volatility"),
      version: z.literal("1.0.0"),
      canonical_config_json: z.literal(
        DEFAULT_STRATEGY_CONFIGS.inverse_volatility.canonicalConfigJson,
      ),
      snapshot_sha256: z.literal(DEFAULT_STRATEGY_CONFIGS.inverse_volatility.snapshotSha256),
      factors: z.tuple([factorDefinitionSchema.extend({ name: z.literal("vol_60") })]),
    })
    .strict(),
]);

const inputIdentitySchema = z
  .object({
    candidate_content_sha256: z.literal(
      "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
    ),
    artifact_manifest_sha256: z.literal(
      "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
    ),
    stage5_manifest_sha256: z.literal(
      "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
    ),
    action_manifest_sha256: z.literal(
      "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
    ),
    approval_registry_sha256: z.literal(
      "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707",
    ),
    bars_sha256: z.literal(
      "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe",
    ),
    ownership_scope: z.literal("OWNER_ONLY"),
    return_method: z.literal("PRICE_RETURN_ONLY"),
    vendor_snapshot: z.literal(true),
    strict_pit: z.literal(false),
  })
  .strict();

const engineIdentitySchema = z
  .object({
    git_commit: z.string().regex(GIT_REVISION_PATTERN),
    binary_sha256: digestSchema,
    toolchain: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    target: z.string().min(1).max(128).regex(ASCII_IDENTIFIER_PATTERN),
    profile: z.string().min(1).max(64).regex(ASCII_IDENTIFIER_PATTERN),
  })
  .strict();

const scheduleDecisionSchema = z
  .object({
    signal_date: dateSchema,
    execution_date: dateSchema,
    kind: decisionKindSchema,
  })
  .strict();

const scheduleSchema = z
  .object({
    strategy: ownerBetaSimulationV2StrategyIdSchema,
    preparation_session_count: wholeCountSchema.min(200).max(2450),
    first_signal_date: dateSchema,
    preparation_sessions: z.array(dateSchema).min(200).max(2450),
    evaluation_sessions: z.array(dateSchema).min(2).max(2252),
    decisions: z.array(scheduleDecisionSchema).min(1).max(2252),
  })
  .strict()
  .superRefine((schedule, context) => {
    const firstEvaluation = schedule.evaluation_sessions[0];
    const lastPreparation = schedule.preparation_sessions.at(-1);
    if (
      schedule.preparation_session_count !== schedule.preparation_sessions.length ||
      lastPreparation === undefined ||
      schedule.first_signal_date !== lastPreparation ||
      firstEvaluation === undefined ||
      lastPreparation >= firstEvaluation ||
      schedule.preparation_sessions.length + schedule.evaluation_sessions.length > 2452 ||
      !orderedDates(schedule.preparation_sessions) ||
      !orderedDates(schedule.evaluation_sessions)
    ) {
      context.addIssue({
        code: "custom",
        path: ["preparation_sessions"],
        message: "invalid common period",
      });
    }

    const expected: Array<z.infer<typeof scheduleDecisionSchema>> = [];
    if (lastPreparation !== undefined && firstEvaluation !== undefined) {
      expected.push({
        signal_date: lastPreparation,
        execution_date: firstEvaluation,
        kind: "INITIAL_COMMON_SESSION",
      });
      for (const [index, signalDate] of schedule.evaluation_sessions.entries()) {
        const executionDate = schedule.evaluation_sessions[index + 1];
        if (executionDate === undefined) continue;
        if (schedule.strategy === "trend_following") {
          expected.push({
            signal_date: signalDate,
            execution_date: executionDate,
            kind: "DAILY_TREND_TRANSITION_CHECK",
          });
        } else if (
          schedule.strategy !== "buy_and_hold" &&
          signalDate.slice(0, 7) !== executionDate.slice(0, 7)
        ) {
          expected.push({
            signal_date: signalDate,
            execution_date: executionDate,
            kind: "MONTH_END_SIGNAL",
          });
        }
      }
    }
    if (!sameValue(schedule.decisions, expected)) {
      context.addIssue({
        code: "custom",
        path: ["decisions"],
        message: "decisions differ from schedule",
      });
    }
  });

const targetWeightSchema = z
  .object({
    instrument_id: instrumentIdSchema,
    weight_ppm: wholeCountSchema.max(1_000_000),
  })
  .strict();

const targetAllocationSchema = z
  .object({
    target_sha256: digestSchema,
    allocation_sha256: digestSchema,
    lineage_sha256: digestSchema,
    weights: z.array(targetWeightSchema).length(11),
    cash_weight_ppm: wholeCountSchema.max(1_000_000),
  })
  .strict()
  .superRefine((target, context) => {
    if (!exactInventory(target.weights)) {
      context.addIssue({
        code: "custom",
        path: ["weights"],
        message: "target inventory is not ETF11",
      });
    }
    const weightTotal = target.weights.reduce((sum, item) => sum + item.weight_ppm, 0);
    if (weightTotal + target.cash_weight_ppm !== 1_000_000) {
      context.addIssue({
        code: "custom",
        path: ["weights"],
        message: "target weights do not sum to one million ppm",
      });
    }
  });

const positionSchema = z
  .object({ instrument_id: instrumentIdSchema, quantity: positiveCountSchema })
  .strict();
const positionListSchema = z
  .array(positionSchema)
  .max(11)
  .superRefine((positions, context) => {
    if (!orderedInstrumentIds(positions)) {
      context.addIssue({
        code: "custom",
        message: "positions must be unique and instrument-sorted",
      });
    }
  });
const targetQuantityListSchema = z
  .array(z.object({ instrument_id: instrumentIdSchema, quantity: wholeCountSchema }).strict())
  .length(11)
  .superRefine((targets, context) => {
    if (!exactInventory(targets)) {
      context.addIssue({
        code: "custom",
        message: "target quantities must contain the sorted ETF11 inventory",
      });
    }
  });

const closeMarkSchema = z
  .object({ instrument_id: instrumentIdSchema, price: money4Schema })
  .strict();
const closeMarkListSchema = z
  .array(closeMarkSchema)
  .max(11)
  .superRefine((marks, context) => {
    if (!orderedInstrumentIds(marks)) {
      context.addIssue({ code: "custom", message: "marks must be unique and instrument-sorted" });
    }
  });

const marketPriceSchema = z
  .object({
    instrument_id: instrumentIdSchema,
    open: money4Schema,
    close: money4Schema,
    volume: wholeCountSchema,
  })
  .strict();
const marketPriceListSchema = z
  .array(marketPriceSchema)
  .length(11)
  .superRefine((prices, context) => {
    if (!exactInventory(prices)) {
      context.addIssue({ code: "custom", message: "market inventory must be sorted ETF11" });
    }
  });

const dailyRecordSchema = z
  .object({
    date: dateSchema,
    opening_cash: money4Schema,
    opening_positions: positionListSchema,
    opening_marks: closeMarkListSchema,
    opening_nav: money4Schema,
    closing_cash: money4Schema,
    closing_positions: positionListSchema,
    close_marks: closeMarkListSchema,
    closing_nav: money4Schema,
    target_lineage: targetAllocationSchema.nullable(),
  })
  .strict()
  .superRefine((record, context) => {
    if (
      !sameValue(
        record.opening_positions.map((item) => item.instrument_id),
        record.opening_marks.map((item) => item.instrument_id),
      )
    ) {
      context.addIssue({
        code: "custom",
        path: ["opening_marks"],
        message: "opening marks differ from holdings",
      });
    }
    if (
      !sameValue(
        record.closing_positions.map((item) => item.instrument_id),
        record.close_marks.map((item) => item.instrument_id),
      )
    ) {
      context.addIssue({
        code: "custom",
        path: ["close_marks"],
        message: "closing marks differ from holdings",
      });
    }
  });

const orderSchema = z
  .object({
    instrument_id: instrumentIdSchema,
    side: z.enum(["BUY", "SELL"]),
    requested_quantity: positiveCountSchema,
    filled_quantity: wholeCountSchema,
    price: money4Schema,
    notional: money4Schema,
    cost: money4Schema,
    cash_after: money4Schema,
    position_after: wholeCountSchema,
    unfilled_reason: z.literal("INSUFFICIENT_COST_INCLUSIVE_BUDGET").nullable(),
  })
  .strict()
  .superRefine((order, context) => {
    if (order.filled_quantity > order.requested_quantity) {
      context.addIssue({
        code: "custom",
        path: ["filled_quantity"],
        message: "fill exceeds requested quantity",
      });
    }
    if ((order.filled_quantity === 0) !== (order.unfilled_reason !== null)) {
      context.addIssue({
        code: "custom",
        path: ["unfilled_reason"],
        message: "fill and unfilled reason disagree",
      });
    }
  });

const targetObservationSchema = z
  .object({
    signal_date: dateSchema,
    execution_date: dateSchema,
    decision_kind: decisionKindSchema,
    target: targetAllocationSchema,
  })
  .strict();

const rebalanceSchema = z
  .object({
    signal_date: dateSchema,
    execution_date: dateSchema,
    decision_kind: decisionKindSchema,
    target: targetAllocationSchema,
    pre_trade_nav: money4Schema,
    open_prices: marketPriceListSchema,
    target_quantities: targetQuantityListSchema,
    orders: z.array(orderSchema).max(22),
    cash_after: money4Schema,
    positions_after: positionListSchema,
    session_costs: money4Schema,
    buy_notional: money4Schema,
    sell_notional: money4Schema,
  })
  .strict();

const marketSessionSchema = z.object({ date: dateSchema, prices: marketPriceListSchema }).strict();

const dailyReturnSchema = z.object({ session_date: dateSchema, value: ratio12Schema }).strict();

const metricsSchema = z
  .object({
    daily_returns: z.array(dailyReturnSchema).min(2).max(2252),
    period_return: ratio12Schema,
    ending_nav: money4Schema,
    ending_cash: money4Schema,
    ending_holdings: positionListSchema,
    cumulative_costs: money4Schema,
    buy_notional: money4Schema,
    sell_notional: money4Schema,
    maximum_drawdown: ratio12Schema,
    period_turnover: ratio12Schema,
    fill_count: wholeCountSchema,
  })
  .strict()
  .superRefine((metrics, context) => {
    if (
      metrics.maximum_drawdown.startsWith("-") ||
      BigInt(metrics.maximum_drawdown.replace(".", "")) > 1_000_000_000_000n
    ) {
      context.addIssue({
        code: "custom",
        path: ["maximum_drawdown"],
        message: "drawdown is outside its ratio range",
      });
    }
    if (metrics.period_turnover.startsWith("-")) {
      context.addIssue({
        code: "custom",
        path: ["period_turnover"],
        message: "turnover cannot be negative",
      });
    }
  });

const ledgerSchema = z
  .object({
    profile: costProfileSchema,
    initial_capital: z.literal(OWNER_BETA_SIMULATION_V2_INITIAL_CAPITAL),
    daily_records: z.array(dailyRecordSchema).min(2).max(2252),
    rebalances: z.array(rebalanceSchema).max(2252),
    market_sessions: z.array(marketSessionSchema).min(2).max(2252),
    target_observations: z.array(targetObservationSchema).min(1).max(2252),
    metrics: metricsSchema,
  })
  .strict();

function sameTargetAllocation(
  left: z.infer<typeof targetAllocationSchema>,
  right: z.infer<typeof targetAllocationSchema>,
): boolean {
  return sameValue(
    { cash_weight_ppm: left.cash_weight_ppm, weights: left.weights },
    { cash_weight_ppm: right.cash_weight_ppm, weights: right.weights },
  );
}

function addLedgerIssues(
  ledger: z.infer<typeof ledgerSchema>,
  schedule: z.infer<typeof scheduleSchema>,
  strategy: OwnerBetaSimulationStrategyId,
  expectedDecisions: readonly z.infer<typeof scheduleDecisionSchema>[],
  context: z.RefinementCtx,
  path: readonly (string | number)[],
): void {
  const evaluation = schedule.evaluation_sessions;
  if (
    !sameValue(
      ledger.daily_records.map((record) => record.date),
      evaluation,
    ) ||
    !sameValue(
      ledger.market_sessions.map((session) => session.date),
      evaluation,
    ) ||
    !sameValue(
      ledger.metrics.daily_returns.map((daily) => daily.session_date),
      evaluation,
    )
  ) {
    context.addIssue({
      code: "custom",
      path: [...path, "daily_records"],
      message: "ledger dates differ from the evaluation period",
    });
  }
  if (
    ledger.daily_records.length !== evaluation.length ||
    ledger.market_sessions.length !== evaluation.length ||
    ledger.metrics.daily_returns.length !== evaluation.length
  ) {
    context.addIssue({
      code: "custom",
      path: [...path],
      message: "ledger counts differ from the evaluation period",
    });
  }

  const observations = ledger.target_observations;
  if (
    observations.length !== expectedDecisions.length ||
    observations.some((observation, index) => {
      const decision = expectedDecisions[index];
      return (
        decision === undefined ||
        observation.signal_date !== decision.signal_date ||
        observation.execution_date !== decision.execution_date ||
        observation.decision_kind !== decision.kind
      );
    })
  ) {
    context.addIssue({
      code: "custom",
      path: [...path, "target_observations"],
      message: "target observations differ from scheduled decisions",
    });
  }

  const expectedRebalances: Array<z.infer<typeof targetObservationSchema>> = [];
  let previousObservation: z.infer<typeof targetObservationSchema> | undefined;
  for (const observation of observations) {
    if (
      strategy !== "trend_following" ||
      previousObservation === undefined ||
      !sameTargetAllocation(previousObservation.target, observation.target)
    ) {
      expectedRebalances.push(observation);
    }
    previousObservation = observation;
  }
  if (ledger.rebalances.length !== expectedRebalances.length) {
    context.addIssue({
      code: "custom",
      path: [...path, "rebalances"],
      message: "rebalance count differs from target transitions",
    });
  }

  for (const [rebalanceIndex, rebalance] of ledger.rebalances.entries()) {
    const observation = expectedRebalances[rebalanceIndex];
    const executionIndex =
      observation === undefined ? -1 : evaluation.indexOf(observation.execution_date);
    const market = executionIndex < 0 ? undefined : ledger.market_sessions[executionIndex];
    if (
      observation === undefined ||
      market === undefined ||
      rebalance.signal_date !== observation.signal_date ||
      rebalance.execution_date !== observation.execution_date ||
      rebalance.decision_kind !== observation.decision_kind ||
      !sameValue(rebalance.target, observation.target) ||
      !sameValue(rebalance.open_prices, market.prices)
    ) {
      context.addIssue({
        code: "custom",
        path: [...path, "rebalances", rebalanceIndex],
        message: "rebalance lineage differs from its scheduled observation or market session",
      });
    }
  }

  let activeTarget: z.infer<typeof targetAllocationSchema> | null = null;
  let rebalanceIndex = 0;
  for (const [index, date] of evaluation.entries()) {
    const day = ledger.daily_records[index];
    if (day === undefined) continue;
    const rebalance = ledger.rebalances[rebalanceIndex];
    const observation = expectedRebalances[rebalanceIndex];
    if (
      rebalance !== undefined &&
      observation !== undefined &&
      observation.execution_date === date &&
      rebalance.execution_date === date
    ) {
      activeTarget = observation.target;
      rebalanceIndex += 1;
    }
    if (!sameValue(day.target_lineage, activeTarget)) {
      context.addIssue({
        code: "custom",
        path: [...path, "daily_records", index, "target_lineage"],
        message: "daily target lineage differs from the active target",
      });
    }
  }
  if (rebalanceIndex !== ledger.rebalances.length) {
    context.addIssue({
      code: "custom",
      path: [...path, "rebalances"],
      message: "not every rebalance was consumed by an evaluation session",
    });
  }

  const lastRecord = ledger.daily_records.at(-1);
  if (
    lastRecord === undefined ||
    ledger.metrics.ending_nav !== lastRecord.closing_nav ||
    ledger.metrics.ending_cash !== lastRecord.closing_cash ||
    !sameValue(ledger.metrics.ending_holdings, lastRecord.closing_positions)
  ) {
    context.addIssue({
      code: "custom",
      path: [...path, "metrics"],
      message: "ending metrics differ from the final daily record",
    });
  }
}

const simulationResultV2Schema: z.ZodType<components["schemas"]["SimulationResultV2"]> = z
  .object({
    schema: z.literal(OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION),
    contract_version: z.literal(OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION),
    contract_sha256: z.literal(OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256),
    input: inputIdentitySchema,
    strategy: strategyIdentitySchema,
    engine: engineIdentitySchema,
    schedule: scheduleSchema,
    cost_profile: costProfileSchema,
    ledger: ledgerSchema,
    benchmark: ledgerSchema,
    benchmark_return_difference: ratio12Schema,
  })
  .strict()
  .superRefine((result, context) => {
    if (result.strategy.strategy !== result.schedule.strategy) {
      context.addIssue({
        code: "custom",
        path: ["schedule", "strategy"],
        message: "schedule strategy differs from strategy identity",
      });
    }
    if (
      result.ledger.profile !== result.cost_profile ||
      result.benchmark.profile !== result.cost_profile ||
      result.ledger.initial_capital !== result.benchmark.initial_capital
    ) {
      context.addIssue({
        code: "custom",
        path: ["cost_profile"],
        message: "ledger profile metadata is inconsistent",
      });
    }
    if (!sameValue(result.ledger.market_sessions, result.benchmark.market_sessions)) {
      context.addIssue({
        code: "custom",
        path: ["benchmark", "market_sessions"],
        message: "benchmark uses a different market path",
      });
    }

    addLedgerIssues(
      result.ledger,
      result.schedule,
      result.strategy.strategy,
      result.schedule.decisions,
      context,
      ["ledger"],
    );
    addLedgerIssues(
      result.benchmark,
      result.schedule,
      "buy_and_hold",
      result.schedule.decisions.slice(0, 1),
      context,
      ["benchmark"],
    );

    const firstDecision = result.schedule.decisions[0];
    const benchmarkObservation = result.benchmark.target_observations[0];
    if (
      firstDecision === undefined ||
      benchmarkObservation === undefined ||
      result.benchmark.target_observations.length !== 1 ||
      result.benchmark.rebalances.length !== 1 ||
      benchmarkObservation.signal_date !== result.schedule.first_signal_date ||
      benchmarkObservation.execution_date !== result.schedule.evaluation_sessions[0] ||
      benchmarkObservation.decision_kind !== "INITIAL_COMMON_SESSION" ||
      benchmarkObservation.target.cash_weight_ppm !== 0 ||
      benchmarkObservation.target.weights.some(
        (weight) => weight.weight_ppm !== (weight.instrument_id === "069500.KRX" ? 1_000_000 : 0),
      )
    ) {
      context.addIssue({
        code: "custom",
        path: ["benchmark", "target_observations"],
        message: "benchmark is not the fixed buy-and-hold target",
      });
    }
  });

export type OwnerBetaSimulationResultV2 = z.infer<typeof simulationResultV2Schema>;

export const ownerBetaSimulationProfileV2Schema = z
  .object({
    result_content_sha256: digestSchema,
    result: simulationResultV2Schema,
  })
  .strict() satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationProfile"]>;

export const ownerBetaSimulationRunBodyV2Schema = z
  .object({
    strategy_config_id: uuidSchema,
    contract_version: z.literal(OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION),
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
  .transform(({ strategy_config_id, contract_version, start_date, end_date }) => ({
    strategy_config_id,
    contract_version,
    ...(start_date === undefined ? {} : { start_date }),
    ...(end_date === undefined ? {} : { end_date }),
  })) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationRunBodyV2"]>;

const runStatusSchema = z.enum(["PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"]);
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

export const ownerBetaSimulationRunV2Schema = z
  .object({
    id: uuidSchema,
    job_id: uuidSchema,
    strategy_config_id: uuidSchema,
    strategy_id: ownerBetaSimulationV2StrategyIdSchema,
    strategy_version: z.literal("1.0.0"),
    start_date: dateSchema,
    end_date: dateSchema,
    evaluation_session_count: wholeCountSchema.min(2).max(2252),
    status: runStatusSchema,
    audience: z.literal("OWNER_ONLY"),
    capability: z.literal("PRICE_RETURN_ONLY"),
    vendor_snapshot: z.literal(true),
    strict_pit: z.literal(false),
    contract_version: z.literal(OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION),
    contract_document_sha256: z.literal(OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256),
    result_schema_version: z.literal(OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION),
    created_at: z.iso.datetime(),
    started_at: z.iso.datetime().nullable(),
    finished_at: z.iso.datetime().nullable(),
    updated_at: z.iso.datetime(),
    error_code: runErrorCodeSchema.nullable(),
  })
  .strict()
  .superRefine((run, context) => {
    if (run.start_date > run.end_date || run.created_at > run.updated_at) {
      context.addIssue({ code: "custom", message: "run metadata is inconsistent" });
    }
  }) satisfies z.ZodType<components["schemas"]["OwnerBetaSimulationRun"]>;

export function ownerBetaSimulationV2PairBindingsMatch(
  illustrative: OwnerBetaSimulationResultV2,
  zeroCost: OwnerBetaSimulationResultV2,
): boolean {
  return (
    sameValue(illustrative.input, zeroCost.input) &&
    sameValue(illustrative.strategy, zeroCost.strategy) &&
    sameValue(illustrative.engine, zeroCost.engine) &&
    sameValue(illustrative.schedule, zeroCost.schedule) &&
    sameValue(illustrative.ledger.market_sessions, zeroCost.ledger.market_sessions) &&
    sameValue(illustrative.ledger.target_observations, zeroCost.ledger.target_observations) &&
    sameValue(
      illustrative.ledger.daily_records.map((record) => record.target_lineage),
      zeroCost.ledger.daily_records.map((record) => record.target_lineage),
    ) &&
    sameValue(illustrative.benchmark.market_sessions, zeroCost.benchmark.market_sessions) &&
    sameValue(illustrative.benchmark.target_observations, zeroCost.benchmark.target_observations) &&
    sameValue(
      illustrative.benchmark.daily_records.map((record) => record.target_lineage),
      zeroCost.benchmark.daily_records.map((record) => record.target_lineage),
    )
  );
}

export const ownerBetaSimulationResultV2Schema = simulationResultV2Schema;
