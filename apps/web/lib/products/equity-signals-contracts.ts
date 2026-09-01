import type { components } from "@lagrange/api-contract";
import { z } from "zod";

export type OwnerBetaEquitySignalsLatestContract =
  components["schemas"]["OwnerBetaEquitySignalsLatest"];
export type OwnerBetaEquitySignalsScreenContract =
  components["schemas"]["OwnerBetaEquitySignalsScreen"];
export type OwnerBetaEquitySignalsDetailContract =
  components["schemas"]["OwnerBetaEquitySignalsDetail"];
export type OwnerBetaEquitySignalsScreenBodyContract =
  components["schemas"]["OwnerBetaEquitySignalsScreenBody"];

const finiteNumberSchema = z.number().finite();
const nonEmptyStringSchema = z.string().min(1);

export const OWNER_BETA_EQUITY_SIGNALS_LATEST_PATH =
  "/api/v1/research/owner-beta/equity-price-signals/latest" as const;
export const OWNER_BETA_EQUITY_SIGNALS_SCREEN_PATH =
  "/api/v1/research/owner-beta/equity-price-signals/screen" as const;
export const OWNER_BETA_EQUITY_SIGNALS_DETAIL_PATH =
  "/api/v1/research/owner-beta/equity-price-signals/instruments" as const;

export function ownerBetaEquitySignalsDetailPath(instrumentId: string): string {
  return `${OWNER_BETA_EQUITY_SIGNALS_DETAIL_PATH}/${encodeURIComponent(instrumentId)}`;
}

export const ownerBetaEquitySignalConditionSchema = z.enum(["BULLISH", "NEUTRAL", "BEARISH"]);

export type OwnerBetaEquitySignalCondition = z.infer<typeof ownerBetaEquitySignalConditionSchema>;

const instrumentIdSchema = z.string().regex(/^\d{6}\.KRX$/);

export const ownerBetaEquitySignalsFiniteRangeSchema = z
  .object({
    min: finiteNumberSchema.nullable().optional(),
    max: finiteNumberSchema.nullable().optional(),
  })
  .strict()
  .superRefine((range, context) => {
    if (
      range.min !== undefined &&
      range.min !== null &&
      range.max !== undefined &&
      range.max !== null &&
      range.min > range.max
    ) {
      context.addIssue({
        code: "custom",
        message: "range minimum must not exceed its maximum",
        path: ["min"],
      });
    }
  });

export type OwnerBetaEquitySignalsFiniteRange = z.infer<
  typeof ownerBetaEquitySignalsFiniteRangeSchema
>;

const ownerBetaEquitySignalRowFields = {
  instrument_id: instrumentIdSchema,
  instrument_name: nonEmptyStringSchema,
  rank: z.number().int().min(1).max(30),
  score: finiteNumberSchema,
  condition: ownerBetaEquitySignalConditionSchema,
  return_20: finiteNumberSchema,
  return_60: finiteNumberSchema,
  return_120: finiteNumberSchema,
  volatility_20: finiteNumberSchema,
  volatility_60: finiteNumberSchema,
  volatility_120: finiteNumberSchema,
  max_drawdown_120: finiteNumberSchema,
  sma_20: finiteNumberSchema,
  sma_60: finiteNumberSchema,
  average_volume_20: finiteNumberSchema,
  volume_ratio_20_60: finiteNumberSchema,
  average_trading_value_20: finiteNumberSchema,
} as const;

export const ownerBetaEquitySignalRowSchema = z.object(ownerBetaEquitySignalRowFields).strict();

export type OwnerBetaEquitySignalRowModel = z.infer<typeof ownerBetaEquitySignalRowSchema>;

export const ownerBetaEquitySignalsProvenanceSchema = z
  .object({
    audience: nonEmptyStringSchema,
    capability: nonEmptyStringSchema,
    selection_basis: nonEmptyStringSchema,
    index_membership: nonEmptyStringSchema,
    redistribution: nonEmptyStringSchema,
    publication_status: nonEmptyStringSchema,
    materialization_status: nonEmptyStringSchema,
    registration_status: nonEmptyStringSchema,
    universe_sha256: nonEmptyStringSchema,
    entitlement_sha256: nonEmptyStringSchema,
    registry_sha256: nonEmptyStringSchema,
    artifact_content_sha256: nonEmptyStringSchema,
    snapshot_content_sha256: nonEmptyStringSchema,
    batch_id: nonEmptyStringSchema,
    as_of: z.iso.date(),
    factor_version: nonEmptyStringSchema,
    vendor_snapshot: z.boolean(),
    strict_pit: z.boolean(),
    original_price: z.boolean(),
    warning: nonEmptyStringSchema,
    activity_proxy: nonEmptyStringSchema,
  })
  .strict();

export type OwnerBetaEquitySignalsProvenanceModel = z.infer<
  typeof ownerBetaEquitySignalsProvenanceSchema
>;

export const ownerBetaEquitySignalsLatestSchema = z
  .object({
    provenance: ownerBetaEquitySignalsProvenanceSchema,
    rows: z.array(ownerBetaEquitySignalRowSchema).max(30),
    top5: z.array(ownerBetaEquitySignalRowSchema).max(5),
  })
  .strict();

export type OwnerBetaEquitySignalsLatestModel = z.infer<typeof ownerBetaEquitySignalsLatestSchema>;

export const ownerBetaEquitySignalsScreenSchema = z
  .object({
    provenance: ownerBetaEquitySignalsProvenanceSchema,
    rows: z.array(ownerBetaEquitySignalRowSchema).max(30),
  })
  .strict();

export type OwnerBetaEquitySignalsScreenModel = z.infer<typeof ownerBetaEquitySignalsScreenSchema>;

export const ownerBetaEquitySignalFactorSchema = z
  .object({
    factor: nonEmptyStringSchema,
    value: finiteNumberSchema,
    interpretation: nonEmptyStringSchema,
  })
  .strict();

export type OwnerBetaEquitySignalFactorModel = z.infer<typeof ownerBetaEquitySignalFactorSchema>;

export const ownerBetaEquitySignalsDetailSchema = z
  .object({
    provenance: ownerBetaEquitySignalsProvenanceSchema,
    signal: ownerBetaEquitySignalRowSchema,
    factor_explanations: z.array(ownerBetaEquitySignalFactorSchema).min(1),
    condition_reasons: z.array(nonEmptyStringSchema),
  })
  .strict();

export type OwnerBetaEquitySignalsDetailModel = z.infer<typeof ownerBetaEquitySignalsDetailSchema>;

const uniqueInstrumentIdsSchema = z
  .array(instrumentIdSchema)
  .max(30)
  .superRefine((ids, context) => {
    if (new Set(ids).size !== ids.length) {
      context.addIssue({ code: "custom", message: "instrument_ids must be unique" });
    }
  });

const uniqueConditionsSchema = z
  .array(ownerBetaEquitySignalConditionSchema)
  .min(1)
  .max(3)
  .superRefine((conditions, context) => {
    if (new Set(conditions).size !== conditions.length) {
      context.addIssue({ code: "custom", message: "condition must be unique" });
    }
  });

export const ownerBetaEquitySignalsScreenConditionsSchema = z
  .object({
    score: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    return_20: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    return_60: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    return_120: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    volatility_20: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    volatility_60: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    volatility_120: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    max_drawdown_120: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    average_trading_value_20: ownerBetaEquitySignalsFiniteRangeSchema.nullable().optional(),
    trend_up: z.boolean().nullable().optional(),
  })
  .strict();

export type OwnerBetaEquitySignalsScreenConditions = z.infer<
  typeof ownerBetaEquitySignalsScreenConditionsSchema
>;

export const ownerBetaEquitySignalsScreenBodySchema = z
  .object({
    instrument_ids: uniqueInstrumentIdsSchema.nullable().optional(),
    conditions: ownerBetaEquitySignalsScreenConditionsSchema.optional(),
    condition: uniqueConditionsSchema.nullable().optional(),
  })
  .strict();

export type OwnerBetaEquitySignalsScreenBody = z.infer<
  typeof ownerBetaEquitySignalsScreenBodySchema
>;

type SearchValue = string | readonly string[] | undefined;
export type OwnerBetaEquitySignalsSearchParams = Readonly<Record<string, SearchValue>>;

export type OwnerBetaEquitySignalsRangeKey =
  | "score"
  | "return_20"
  | "return_60"
  | "return_120"
  | "volatility_20"
  | "volatility_60"
  | "volatility_120"
  | "max_drawdown_120"
  | "average_trading_value_20";

export const OWNER_BETA_EQUITY_SIGNALS_RANGE_KEYS: readonly OwnerBetaEquitySignalsRangeKey[] = [
  "score",
  "return_20",
  "return_60",
  "return_120",
  "volatility_20",
  "volatility_60",
  "volatility_120",
  "max_drawdown_120",
  "average_trading_value_20",
];

export type OwnerBetaEquitySignalsFilters = {
  readonly conditions: readonly OwnerBetaEquitySignalCondition[];
  readonly ranges: Partial<
    Record<OwnerBetaEquitySignalsRangeKey, OwnerBetaEquitySignalsFiniteRange>
  >;
  readonly trendUp?: boolean;
};

export class InvalidOwnerBetaEquitySignalsFilters extends Error {
  override readonly name = "InvalidOwnerBetaEquitySignalsFilters";
}

function values(value: SearchValue): readonly string[] {
  if (value === undefined) return [];
  return typeof value === "string" ? [value] : value;
}

function one(
  params: OwnerBetaEquitySignalsSearchParams,
  names: readonly string[],
  label: string,
): string | undefined {
  const found = names.flatMap((name) => values(params[name]));
  if (found.length > 1) {
    throw new InvalidOwnerBetaEquitySignalsFilters(`${label} must be selected once.`);
  }
  const value = found[0];
  return value === undefined || value.trim() === "" ? undefined : value.trim();
}

function numeric(
  params: OwnerBetaEquitySignalsSearchParams,
  names: readonly string[],
  label: string,
): number | undefined {
  const raw = one(params, names, label);
  if (raw === undefined) return undefined;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) {
    throw new InvalidOwnerBetaEquitySignalsFilters(`${label} must be a finite number.`);
  }
  return parsed;
}

function range(
  params: OwnerBetaEquitySignalsSearchParams,
  key: OwnerBetaEquitySignalsRangeKey,
  aliases: readonly string[] = [],
): OwnerBetaEquitySignalsFiniteRange | undefined {
  const min = numeric(
    params,
    [`${key}_min`, `min_${key}`, ...aliases.map((name) => `${name}_min`)],
    `${key} minimum`,
  );
  const max = numeric(
    params,
    [`${key}_max`, `max_${key}`, ...aliases.map((name) => `${name}_max`)],
    `${key} maximum`,
  );
  if (min === undefined && max === undefined) return undefined;
  const parsed = {
    ...(min === undefined ? {} : { min }),
    ...(max === undefined ? {} : { max }),
  };
  const result = ownerBetaEquitySignalsFiniteRangeSchema.safeParse(parsed);
  if (!result.success) {
    throw new InvalidOwnerBetaEquitySignalsFilters(`${key} minimum must not exceed its maximum.`);
  }
  return result.data;
}

function conditionsFrom(
  params: OwnerBetaEquitySignalsSearchParams,
): readonly OwnerBetaEquitySignalCondition[] {
  const raw = values(params["condition"]);
  if (raw.length === 0) return [];
  const conditions = raw.map((value) => {
    const parsed = ownerBetaEquitySignalConditionSchema.safeParse(value.trim().toUpperCase());
    if (!parsed.success) {
      throw new InvalidOwnerBetaEquitySignalsFilters("Scenario condition is invalid.");
    }
    return parsed.data;
  });
  if (new Set(conditions).size !== conditions.length) {
    throw new InvalidOwnerBetaEquitySignalsFilters("Scenario conditions must be unique.");
  }
  return conditions;
}

function trendFrom(params: OwnerBetaEquitySignalsSearchParams): boolean | undefined {
  const raw = one(params, ["trend", "trend_up"], "Trend");
  if (raw === undefined) return undefined;
  if (raw === "up" || raw === "true") return true;
  if (raw === "down" || raw === "false") return false;
  throw new InvalidOwnerBetaEquitySignalsFilters("Trend is invalid.");
}

export function parseOwnerBetaEquitySignalsSearchParams(
  params: OwnerBetaEquitySignalsSearchParams,
): OwnerBetaEquitySignalsFilters {
  const trendUp = trendFrom(params);
  const ranges = {
    score: range(params, "score"),
    return_20: range(params, "return_20"),
    return_60: range(params, "return_60"),
    return_120: range(params, "return_120"),
    volatility_20: range(params, "volatility_20"),
    volatility_60: range(params, "volatility_60"),
    volatility_120: range(params, "volatility_120"),
    max_drawdown_120: range(params, "max_drawdown_120"),
    average_trading_value_20: range(params, "average_trading_value_20", [
      "activity",
      "trading_value",
    ]),
  } satisfies Partial<
    Record<OwnerBetaEquitySignalsRangeKey, OwnerBetaEquitySignalsFiniteRange | undefined>
  >;
  return {
    conditions: conditionsFrom(params),
    ranges: Object.fromEntries(
      Object.entries(ranges).filter(
        (entry): entry is [string, OwnerBetaEquitySignalsFiniteRange] => entry[1] !== undefined,
      ),
    ) as Partial<Record<OwnerBetaEquitySignalsRangeKey, OwnerBetaEquitySignalsFiniteRange>>,
    ...(trendUp === undefined ? {} : { trendUp }),
  };
}

export function ownerBetaEquitySignalsFiltersSelected(
  filters: OwnerBetaEquitySignalsFilters,
): boolean {
  return (
    filters.conditions.length > 0 ||
    Object.keys(filters.ranges).length > 0 ||
    filters.trendUp !== undefined
  );
}

export function ownerBetaEquitySignalsScreenBody(
  filters: OwnerBetaEquitySignalsFilters,
): OwnerBetaEquitySignalsScreenBody {
  const conditions: OwnerBetaEquitySignalsScreenConditions = {
    ...(filters.ranges.score === undefined ? {} : { score: filters.ranges.score }),
    ...(filters.ranges.return_20 === undefined ? {} : { return_20: filters.ranges.return_20 }),
    ...(filters.ranges.return_60 === undefined ? {} : { return_60: filters.ranges.return_60 }),
    ...(filters.ranges.return_120 === undefined ? {} : { return_120: filters.ranges.return_120 }),
    ...(filters.ranges.volatility_20 === undefined
      ? {}
      : { volatility_20: filters.ranges.volatility_20 }),
    ...(filters.ranges.volatility_60 === undefined
      ? {}
      : { volatility_60: filters.ranges.volatility_60 }),
    ...(filters.ranges.volatility_120 === undefined
      ? {}
      : { volatility_120: filters.ranges.volatility_120 }),
    ...(filters.ranges.max_drawdown_120 === undefined
      ? {}
      : { max_drawdown_120: filters.ranges.max_drawdown_120 }),
    ...(filters.ranges.average_trading_value_20 === undefined
      ? {}
      : { average_trading_value_20: filters.ranges.average_trading_value_20 }),
    ...(filters.trendUp === undefined ? {} : { trend_up: filters.trendUp }),
  };
  const body = {
    conditions,
    ...(filters.conditions.length === 0 ? {} : { condition: [...filters.conditions] }),
  };
  return ownerBetaEquitySignalsScreenBodySchema.parse(body);
}

// V2 is intentionally a separate contract. Keep the fixed V1 schemas above
// unchanged so the legacy read path remains byte- and behavior-compatible.
export type OwnerEquityV2AddBodyContract = components["schemas"]["OwnerEquityV2AddBody"];
export type OwnerEquityV2Lifecycle = components["schemas"]["OwnerEquityV2Lifecycle"];
export type OwnerEquityV2PolicyContract = components["schemas"]["OwnerEquityV2Policy"];
export type OwnerEquityV2CoverageContract = components["schemas"]["OwnerEquityV2Coverage"];
export type OwnerEquityV2FailureContract = components["schemas"]["OwnerEquityV2Failure"];
export type OwnerEquityV2MembershipContract = components["schemas"]["OwnerEquityV2Membership"];
export type OwnerEquityV2MembershipListContract =
  components["schemas"]["OwnerEquityV2MembershipList"];
export type OwnerEquityV2MembershipStatusContract =
  components["schemas"]["OwnerEquityV2MembershipStatus"];
export type OwnerEquityV2MutationContract = components["schemas"]["OwnerEquityV2Mutation"];
export type OwnerEquityV2SnapshotContract = components["schemas"]["OwnerEquityV2Snapshot"];
export type OwnerEquityV2SignalContract = components["schemas"]["OwnerEquityV2Signal"];
export type OwnerEquityV2LatestSignalsContract =
  components["schemas"]["OwnerEquityV2LatestSignals"];
export type OwnerEquityV2ScreenSignalsContract =
  components["schemas"]["OwnerEquityV2ScreenSignals"];
export type OwnerEquityV2SignalDetailContract = components["schemas"]["OwnerEquityV2SignalDetail"];
export type OwnerEquityV2ScreenBodyContract = components["schemas"]["OwnerEquityV2ScreenBody"];

export const OWNER_EQUITY_V2_MEMBERSHIPS_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/memberships" as const;
export const OWNER_EQUITY_V2_SIGNALS_LATEST_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/signals/latest" as const;
export const OWNER_EQUITY_V2_SIGNALS_SCREEN_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/signals/screen" as const;
export const OWNER_EQUITY_V2_SIGNALS_DETAIL_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments" as const;

export const OWNER_EQUITY_V2_LIFECYCLE_VALUES = [
  "REQUESTED",
  "VALIDATING",
  "BACKFILLING",
  "MATERIALIZING",
  "READY",
  "INSUFFICIENT_HISTORY",
  "FAILED",
  "DISABLED",
] as const satisfies readonly OwnerEquityV2Lifecycle[];

export const ownerEquityV2LifecycleSchema = z.enum(OWNER_EQUITY_V2_LIFECYCLE_VALUES);

const ownerEquityV2FailureCodeSchema = z.string().regex(/^[A-Z][A-Z0-9_]{0,63}$/);

export const ownerEquityV2AddBodySchema = z
  .object({ instrument_code: z.string().regex(/^\d{6}$/) })
  .strict();

export type OwnerEquityV2AddBody = z.infer<typeof ownerEquityV2AddBodySchema>;

export const ownerEquityV2PolicySchema = z
  .object({
    max_active_instruments: z.number().int().nonnegative(),
    active_instruments: z.number().int().nonnegative(),
    remaining_capacity: z.number().int().nonnegative(),
    target_observed_sessions: z.number().int().nonnegative(),
    minimum_observed_sessions: z.number().int().nonnegative(),
  })
  .strict()
  .superRefine((policy, context) => {
    if (policy.active_instruments > policy.max_active_instruments) {
      context.addIssue({ code: "custom", message: "active instruments exceed policy capacity" });
    }
    if (policy.remaining_capacity !== policy.max_active_instruments - policy.active_instruments) {
      context.addIssue({ code: "custom", message: "remaining capacity does not match policy" });
    }
    if (policy.minimum_observed_sessions > policy.target_observed_sessions) {
      context.addIssue({ code: "custom", message: "minimum coverage exceeds target coverage" });
    }
  });

export type OwnerEquityV2PolicyModel = z.infer<typeof ownerEquityV2PolicySchema>;

export const ownerEquityV2CoverageSchema = z
  .object({
    observed_sessions: z.number().int().nonnegative(),
    target_observed_sessions: z.number().int().nonnegative(),
    minimum_observed_sessions: z.number().int().nonnegative(),
    first_session: z.iso.date().optional(),
    last_session: z.iso.date().optional(),
  })
  .strict()
  .superRefine((coverage, context) => {
    if (coverage.minimum_observed_sessions > coverage.target_observed_sessions) {
      context.addIssue({ code: "custom", message: "minimum coverage exceeds target coverage" });
    }
  });

export type OwnerEquityV2CoverageModel = z.infer<typeof ownerEquityV2CoverageSchema>;

export const ownerEquityV2FailureSchema = z
  .object({ code: ownerEquityV2FailureCodeSchema, retryable: z.boolean() })
  .strict();

export type OwnerEquityV2FailureModel = z.infer<typeof ownerEquityV2FailureSchema>;

export const ownerEquityV2MembershipSchema = z
  .object({
    id: z.uuid(),
    instrument_id: instrumentIdSchema,
    lifecycle: ownerEquityV2LifecycleSchema,
    // A newly accepted membership has no materialized generation yet.  The
    // API deliberately reports that lifecycle as generation 0 until the
    // first immutable candidate is admitted.
    generation: z.number().int().nonnegative(),
    coverage: ownerEquityV2CoverageSchema,
    failure: ownerEquityV2FailureSchema.optional(),
    requested_at: z.iso.datetime(),
    disabled_at: z.iso.datetime().optional(),
    updated_at: z.iso.datetime(),
  })
  .strict();

export type OwnerEquityV2MembershipModel = z.infer<typeof ownerEquityV2MembershipSchema>;

export const ownerEquityV2MembershipListSchema = z
  .object({
    policy: ownerEquityV2PolicySchema,
    memberships: z.array(ownerEquityV2MembershipSchema),
  })
  .strict();

export type OwnerEquityV2MembershipListModel = z.infer<typeof ownerEquityV2MembershipListSchema>;

export const ownerEquityV2MembershipStatusSchema = z
  .object({
    policy: ownerEquityV2PolicySchema,
    membership: ownerEquityV2MembershipSchema,
  })
  .strict();

export type OwnerEquityV2MembershipStatusModel = z.infer<
  typeof ownerEquityV2MembershipStatusSchema
>;

export const ownerEquityV2MutationSchema = z
  .object({
    resource: ownerEquityV2MembershipSchema,
    job_id: z.uuid(),
    duplicate_active: z.boolean(),
  })
  .strict();

export type OwnerEquityV2MutationModel = z.infer<typeof ownerEquityV2MutationSchema>;

export const ownerEquityV2SnapshotSchema = z
  .object({
    snapshot_id: z.uuid(),
    as_of: z.iso.date(),
    universe_sha256: nonEmptyStringSchema,
    row_count: z.number().int().nonnegative(),
    published_at: z.iso.datetime(),
  })
  .strict();

export type OwnerEquityV2SnapshotModel = z.infer<typeof ownerEquityV2SnapshotSchema>;

export const ownerEquityV2SignalSchema = z
  .object({
    instrument_id: instrumentIdSchema,
    generation: z.number().int().positive(),
    rank: z.number().int().positive(),
    score: finiteNumberSchema,
    condition: ownerBetaEquitySignalConditionSchema,
    return_20: finiteNumberSchema,
    return_60: finiteNumberSchema,
    return_120: finiteNumberSchema,
    volatility_20: finiteNumberSchema,
    volatility_60: finiteNumberSchema,
    volatility_120: finiteNumberSchema,
    max_drawdown_120: finiteNumberSchema,
    sma_20: finiteNumberSchema,
    sma_60: finiteNumberSchema,
    average_volume_20: finiteNumberSchema,
    volume_ratio_20_60: finiteNumberSchema,
    average_trading_value_20: finiteNumberSchema,
  })
  .strict();

export type OwnerEquityV2SignalModel = z.infer<typeof ownerEquityV2SignalSchema>;

export const ownerEquityV2LatestSignalsSchema = z
  .object({
    snapshot: ownerEquityV2SnapshotSchema,
    rows: z.array(ownerEquityV2SignalSchema),
    top5: z.array(ownerEquityV2SignalSchema),
  })
  .strict();

export type OwnerEquityV2LatestSignalsModel = z.infer<typeof ownerEquityV2LatestSignalsSchema>;

export const ownerEquityV2ScreenSignalsSchema = z
  .object({
    snapshot: ownerEquityV2SnapshotSchema,
    rows: z.array(ownerEquityV2SignalSchema),
  })
  .strict();

export type OwnerEquityV2ScreenSignalsModel = z.infer<typeof ownerEquityV2ScreenSignalsSchema>;

export const ownerEquityV2SignalDetailSchema = z
  .object({
    snapshot: ownerEquityV2SnapshotSchema,
    signal: ownerEquityV2SignalSchema,
  })
  .strict();

export type OwnerEquityV2SignalDetailModel = z.infer<typeof ownerEquityV2SignalDetailSchema>;

const uniqueOwnerEquityV2InstrumentIdsSchema = z
  .array(instrumentIdSchema)
  .superRefine((ids, context) => {
    if (new Set(ids).size !== ids.length) {
      context.addIssue({ code: "custom", message: "instrument_ids must be unique" });
    }
  });

const uniqueOwnerEquityV2ConditionsSchema = z
  .array(ownerBetaEquitySignalConditionSchema)
  .superRefine((conditions, context) => {
    if (new Set(conditions).size !== conditions.length) {
      context.addIssue({ code: "custom", message: "conditions must be unique" });
    }
  });

export const ownerEquityV2ScreenBodySchema = z
  .object({
    instrument_ids: uniqueOwnerEquityV2InstrumentIdsSchema.nullable().optional(),
    conditions: uniqueOwnerEquityV2ConditionsSchema.nullable().optional(),
  })
  .strict();

export type OwnerEquityV2ScreenBody = z.infer<typeof ownerEquityV2ScreenBodySchema>;

export function ownerEquityV2MembershipPath(membershipId: string): string {
  return `${OWNER_EQUITY_V2_MEMBERSHIPS_PATH}/${encodeURIComponent(membershipId)}`;
}

export function ownerEquityV2RetryPath(membershipId: string): string {
  return `${ownerEquityV2MembershipPath(membershipId)}/retry`;
}

export function ownerEquityV2DisablePath(membershipId: string): string {
  return `${ownerEquityV2MembershipPath(membershipId)}/disable`;
}

export function ownerEquityV2SignalDetailPath(instrumentId: string): string {
  return `${OWNER_EQUITY_V2_SIGNALS_DETAIL_PATH}/${encodeURIComponent(instrumentId)}`;
}
