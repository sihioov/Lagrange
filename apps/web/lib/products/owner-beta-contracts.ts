import { z } from "zod";

const sha256Schema = z.string().regex(/^sha256:[0-9a-f]{64}$/);
const fixedWeightSchema = z.string().regex(/^(?:0\.\d{6}|1\.000000)$/);
const ownerBetaInstrumentSchema = z.enum([
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
const ownerBetaReasonCodeSchema = z.enum([
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
const ownerBetaReasonCodesSchema = z
  .array(ownerBetaReasonCodeSchema)
  .min(1)
  .max(16)
  .refine((codes) => new Set(codes).size === codes.length, "reason codes must be unique");
const ownerBetaFactorsSchema = z
  .record(z.string().min(1).max(64), z.string().min(1).max(64))
  .refine((factors) => Object.keys(factors).length <= 64, "too many factor values");

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
    instrument_id: ownerBetaInstrumentSchema,
    rank: z.number().int().min(1).max(11).nullable().optional(),
    reason_codes: ownerBetaReasonCodesSchema,
    target_weight: fixedWeightSchema.nullable().optional(),
  })
  .strict()
  .superRefine((item, context) => {
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
  if (run.status === "SUCCEEDED") {
    const instrumentIds = new Set(run.items.map((item) => item.instrument_id));
    const ranks = run.items.flatMap((item) => (item.rank == null ? [] : [item.rank]));
    const totalWeight = run.items.reduce(
      (total, item) =>
        item.target_weight == null ? total : total + weightUnits(item.target_weight),
      run.cash_weight == null ? 0 : weightUnits(run.cash_weight),
    );
    if (
      run.items.length !== ownerBetaInstrumentSchema.options.length ||
      instrumentIds.size !== ownerBetaInstrumentSchema.options.length ||
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

export function ownerBetaRunPath(runId: string): string {
  return `${OWNER_BETA_PRICE_ONLY_RUNS_PATH}/${encodeURIComponent(runId)}`;
}
