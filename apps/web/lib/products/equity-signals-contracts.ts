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
