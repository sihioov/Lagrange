import { z } from "zod";

const sha256Schema = z.string().regex(/^sha256:[0-9a-f]{64}$/);
const fixedWeightSchema = z.string().regex(/^(?:0\.\d{6}|1\.000000)$/);
export const ownerBetaInstrumentIdSchema = z.enum([
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
export type OwnerBetaInstrumentId = z.infer<typeof ownerBetaInstrumentIdSchema>;

export const ownerBetaInstrumentSchema = z
  .object({
    asset_class: z.string().nullable(),
    exposure_group: z.null(),
    id: ownerBetaInstrumentIdSchema,
    name: z.string().nullable(),
    tracking_index: z.null(),
  })
  .strict();

export type OwnerBetaInstrumentModel = z.infer<typeof ownerBetaInstrumentSchema>;

export const ownerBetaReasonCodeSchema = z.enum([
  "SELECTED_TOP_N",
  "NOT_SELECTED_BEYOND_TOP_N",
  "EXCLUDED_MANDATORY_FACTOR_NULL",
  "ALL_CASH_NO_ELIGIBLE",
  "WEIGHT_CAPPED_AT_MAX",
  "WEIGHT_ROUNDING_RESIDUE_TO_CASH",
  "CASH_FLOOR_APPLIED",
  "BENCHMARK_HELD",
  "TREND_POSITIVE",
  "TREND_NEGATIVE_CASH",
  "ABSOLUTE_MOMENTUM_PASSED",
  "DEFENSIVE_CASH_SELECTED",
  "INVERSE_VOL_WEIGHTED",
  "NOT_SELECTED_BY_STRATEGY",
]);
export type OwnerBetaReasonCode = z.infer<typeof ownerBetaReasonCodeSchema>;

const ownerBetaReasonCodesSchema = z
  .array(ownerBetaReasonCodeSchema)
  .min(1)
  .max(14)
  .refine((codes) => new Set(codes).size === codes.length, "reason codes must be unique");
// Mirrors the finite `python_float_string` text persisted by the durable
// producer: a plain f64 spelling always has a fractional part, while a
// scientific spelling uses lowercase `e`, a mandatory sign, and at least two
// exponent digits.
const CANONICAL_FACTOR_DECIMAL_PATTERN =
  /^-?(?:(?:0|[1-9]\d*)\.\d+|(?:0|[1-9]\d*)(?:\.\d+)?e[+-]\d{2,})$/;
const TREND_FACTOR_ID_PATTERN = /^trend_([1-9]\d*)$/;

function hasSafeFactorExponent(value: string): boolean {
  const exponent = /e([+-]\d{2,})$/.exec(value)?.[1];
  return exponent === undefined || Number.isSafeInteger(Number(exponent));
}

const ownerBetaFactorValueSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(CANONICAL_FACTOR_DECIMAL_PATTERN)
  .refine(hasSafeFactorExponent, "factor exponent must be safe")
  .refine((value) => Number.isFinite(Number(value)), "factor value must be finite");

function isAllowedOwnerBetaFactorId(id: string): boolean {
  const trend = TREND_FACTOR_ID_PATTERN.exec(id);
  if (trend !== null) {
    const window = Number(trend[1]);
    return Number.isSafeInteger(window) && window >= 5 && window <= 500;
  }
  return (
    id === "momentum_12_1" ||
    id === "return_12m" ||
    id === "vol_20" ||
    id === "vol_60" ||
    id === "vol_120"
  );
}

export const ownerBetaFactorsSchema = z
  .record(z.string().min(1).max(64), ownerBetaFactorValueSchema)
  .superRefine((factors, context) => {
    if (Object.keys(factors).length > 64) {
      context.addIssue({ code: "custom", message: "too many factor values" });
    }
    for (const id of Object.keys(factors)) {
      if (!isAllowedOwnerBetaFactorId(id)) {
        context.addIssue({ code: "custom", message: "unknown owner-beta factor" });
      } else {
        const factor = factors[id];
        if (factor === undefined) {
          context.addIssue({ code: "custom", message: "missing owner-beta factor" });
          continue;
        }
        if (id.startsWith("vol_") && factor.startsWith("-")) {
          context.addIssue({ code: "custom", message: "volatility factor must not be negative" });
        }
      }
    }
  });

export type OwnerBetaFactors = z.infer<typeof ownerBetaFactorsSchema>;

const ownerBetaSupportedAsOfDateSchema = z.iso.date();

export const ownerBetaSupportedAsOfSchema = z
  .object({
    default_as_of: ownerBetaSupportedAsOfDateSchema,
    supported_as_of: z.array(ownerBetaSupportedAsOfDateSchema).min(1),
  })
  .strict()
  .superRefine((discovery, context) => {
    const supported = discovery.supported_as_of;
    if (new Set(supported).size !== supported.length) {
      context.addIssue({ code: "custom", message: "supported_as_of values must be unique" });
    }
    const sorted = [...supported].sort();
    if (sorted.some((date, index) => date !== supported[index])) {
      context.addIssue({ code: "custom", message: "supported_as_of values must be sorted" });
    }
    if (discovery.default_as_of !== sorted.at(-1)) {
      context.addIssue({
        code: "custom",
        message: "default_as_of must be the maximum supported date",
      });
    }
  });

export type OwnerBetaSupportedAsOfModel = z.infer<typeof ownerBetaSupportedAsOfSchema>;

/**
 * The owner-beta response is intentionally independent from the ordinary
 * recommendation contract.  The API owns the input pins and only exposes
 * their immutable commitments here; clients cannot supply or resolve them.
 */
export const ownerBetaRunStatusSchema = z.enum([
  "PENDING",
  "RUNNING",
  "SUCCEEDED",
  "FAILED",
  "CANCELED",
]);

export type OwnerBetaRunStatus = z.infer<typeof ownerBetaRunStatusSchema>;

export const ownerBetaItemSchema = z
  .object({
    excluded: z.boolean(),
    exclusion_reason: ownerBetaReasonCodeSchema.nullable().optional(),
    factors: ownerBetaFactorsSchema,
    instrument: ownerBetaInstrumentSchema,
    instrument_id: ownerBetaInstrumentIdSchema,
    rank: z.number().int().min(1).max(11).nullable().optional(),
    reason_codes: ownerBetaReasonCodesSchema,
    target_weight: fixedWeightSchema.nullable().optional(),
  })
  .strict()
  .superRefine((item, context) => {
    if (item.instrument.id !== item.instrument_id) {
      context.addIssue({
        code: "custom",
        message: "instrument metadata id must match instrument_id",
      });
    }
    const hasRank = item.rank !== undefined && item.rank !== null;
    const hasWeight = item.target_weight !== undefined && item.target_weight !== null;
    const hasExclusion = item.exclusion_reason !== undefined && item.exclusion_reason !== null;
    if (item.excluded) {
      if (hasRank || hasWeight || !hasExclusion || item.exclusion_reason !== item.reason_codes[0]) {
        context.addIssue({ code: "custom", message: "invalid excluded owner-beta item" });
      }
    } else if (!hasRank || !hasWeight || hasExclusion) {
      context.addIssue({ code: "custom", message: "invalid selected owner-beta item" });
    }
  });

export type OwnerBetaItemModel = z.infer<typeof ownerBetaItemSchema>;

const ownerBetaRunFields = {
  action_manifest_sha256: sha256Schema,
  approval_registry_sha256: sha256Schema,
  artifact_manifest_sha256: sha256Schema,
  as_of: z.iso.date(),
  audience: z.literal("OWNER_ONLY"),
  candidate_content_sha256: sha256Schema,
  capability: z.literal("PRICE_RETURN_ONLY"),
  created_at: z.iso.datetime(),
  error_code: z
    .string()
    .regex(/^[A-Z][A-Z0-9_]{0,63}$/)
    .nullable()
    .optional(),
  factor_snapshot_sha256: sha256Schema.nullable().optional(),
  finished_at: z.iso.datetime().nullable().optional(),
  id: z.uuid(),
  input_kind: z.literal("owner_beta_historical_price_only_v1"),
  items: z.array(ownerBetaItemSchema),
  job_id: z.uuid(),
  stage5_manifest_sha256: sha256Schema,
  started_at: z.iso.datetime().nullable().optional(),
  status: ownerBetaRunStatusSchema,
  strategy_config_id: z.uuid(),
  strategy_config_sha256: sha256Schema,
  strategy_id: z.string().min(1),
  strategy_version: z.string().min(1),
  target_snapshot_sha256: sha256Schema.nullable().optional(),
  updated_at: z.iso.datetime(),
  vendor_snapshot: z.literal(true),
  strict_pit: z.literal(false),
  cash_weight: fixedWeightSchema.nullable().optional(),
} as const;

function weightUnits(value: string): number {
  return value === "1.000000" ? 1_000_000 : Number.parseInt(value.slice(2), 10);
}

const ownerBetaRunObjectSchema = z.object(ownerBetaRunFields).strict();

export const ownerBetaRunSchema = ownerBetaRunObjectSchema.superRefine((run, context) => {
  if (run.strategy_id === "buy_and_hold") {
    for (const item of run.items) {
      if (Object.keys(item.factors).length !== 0) {
        context.addIssue({
          code: "custom",
          message: "buy-and-hold items cannot carry factor evidence",
        });
        break;
      }
    }
  } else {
    for (const item of run.items) {
      if (!item.excluded && Object.keys(item.factors).length === 0) {
        context.addIssue({
          code: "custom",
          message: "selected owner-beta item lacks factor evidence",
        });
        break;
      }
    }
  }

  if (run.status === "SUCCEEDED") {
    const instrumentIds = new Set(run.items.map((item) => item.instrument_id));
    const ranks = run.items.flatMap((item) => (item.rank == null ? [] : [item.rank]));
    const totalWeight = run.items.reduce(
      (total, item) =>
        item.target_weight == null ? total : total + weightUnits(item.target_weight),
      run.cash_weight == null ? 0 : weightUnits(run.cash_weight),
    );
    if (
      run.items.length !== ownerBetaInstrumentIdSchema.options.length ||
      instrumentIds.size !== ownerBetaInstrumentIdSchema.options.length ||
      new Set(ranks).size !== ranks.length ||
      run.factor_snapshot_sha256 === undefined ||
      run.factor_snapshot_sha256 === null ||
      run.target_snapshot_sha256 === undefined ||
      run.target_snapshot_sha256 === null ||
      run.cash_weight === undefined ||
      run.cash_weight === null ||
      (run.error_code !== undefined && run.error_code !== null) ||
      totalWeight !== 1_000_000
    ) {
      context.addIssue({ code: "custom", message: "invalid successful owner-beta run" });
    }
    return;
  }

  const hasResult =
    (run.factor_snapshot_sha256 !== undefined && run.factor_snapshot_sha256 !== null) ||
    (run.target_snapshot_sha256 !== undefined && run.target_snapshot_sha256 !== null) ||
    (run.cash_weight !== undefined && run.cash_weight !== null) ||
    run.items.length !== 0;
  const requiresError = run.status === "FAILED" || run.status === "CANCELED";
  const hasError = run.error_code !== undefined && run.error_code !== null;
  if (hasResult || requiresError !== hasError) {
    context.addIssue({ code: "custom", message: "invalid unsettled owner-beta run" });
  }
});

export type OwnerBetaRunModel = z.infer<typeof ownerBetaRunSchema>;

/** List rows carry immutable run metadata but never item payloads. */
export const ownerBetaRunListItemSchema = ownerBetaRunObjectSchema.omit({ items: true });

export type OwnerBetaRunListItemModel = z.infer<typeof ownerBetaRunListItemSchema>;

export const ownerBetaRunPageSchema = z
  .object({
    has_more: z.boolean(),
    items: z.array(ownerBetaRunListItemSchema),
    next_cursor: z.string().nullable(),
  })
  .strict();

export type OwnerBetaRunPageModel = z.infer<typeof ownerBetaRunPageSchema>;

export const ownerBetaPriceOnlyRunBodySchema = z
  .object({
    as_of: z.iso.date(),
    strategy_config_id: z.uuid(),
  })
  .strict();

export type OwnerBetaPriceOnlyRunBody = z.infer<typeof ownerBetaPriceOnlyRunBodySchema>;

export const ownerBetaPriceOnlyRunResponseSchema = z
  .object({
    job_id: z.uuid(),
    run_id: z.uuid(),
    status: z.literal("PENDING"),
  })
  .strict();

export type OwnerBetaPriceOnlyRunResponse = z.infer<typeof ownerBetaPriceOnlyRunResponseSchema>;

/** The one server-owned route used by both owner-beta reads and its form. */
export const OWNER_BETA_PRICE_ONLY_RUNS_PATH =
  "/api/v1/recommendations/owner-beta/price-only/runs" as const;

export const OWNER_BETA_PRICE_ONLY_SUPPORTED_AS_OF_PATH =
  "/api/v1/recommendations/owner-beta/price-only/supported-as-of" as const;

export function ownerBetaRunPath(runId: string): string {
  return `${OWNER_BETA_PRICE_ONLY_RUNS_PATH}/${encodeURIComponent(runId)}`;
}
