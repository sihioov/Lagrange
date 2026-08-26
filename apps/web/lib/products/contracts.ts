import type { components } from "@lagrange/api-contract";
import { z } from "zod";

export type Strategy = components["schemas"]["Strategy"];
export type StrategyConfig = components["schemas"]["StrategyConfig"];
export type NewStrategyConfig = components["schemas"]["NewStrategyConfigBody"];
export type RecommendationItem = components["schemas"]["RecommendationItem"];
export type RecommendationRun = components["schemas"]["RecommendationRun"];
export type RecommendationRunBody = components["schemas"]["RecommendationRunBody"];
export type LicensingStatus = components["schemas"]["LicensingStatus"];

export const parameterDefinitionSchema = z
  .object({
    default: z.union([z.boolean(), z.number(), z.string()]).optional(),
    description: z.string().optional(),
    enum: z.array(z.union([z.number(), z.string()])).optional(),
    exclusiveMinimum: z.number().optional(),
    maximum: z.number().optional(),
    minimum: z.number().optional(),
    pattern: z.string().optional(),
    title: z.string().min(1).optional(),
    type: z.enum(["boolean", "integer", "number", "string"]),
  })
  .strict();

export type ParameterDefinition = z.infer<typeof parameterDefinitionSchema>;

export const parameterSchema = z
  .object({
    $schema: z.string().url().optional(),
    additionalProperties: z.boolean().optional(),
    properties: z.record(z.string(), parameterDefinitionSchema),
    required: z.array(z.string()),
    type: z.literal("object"),
  })
  .strict();

export type ParameterSchema = z.infer<typeof parameterSchema>;
export const strategySchema = z
  .object({
    description: z.string().optional(),
    default_parameters: z
      .record(z.string(), z.union([z.boolean(), z.number(), z.string()]))
      .optional(),
    display_name: z.string(),
    id: z.string(),
    latest_version: z.string().nullable().optional(),
    parameter_schema: parameterSchema.optional(),
    risk_description: z.string().optional(),
    state: z.enum(["Draft", "Validated", "Paper", "LiveCandidate", "Retired"]),
  })
  .strict();

export type StrategyCatalogItem = z.infer<typeof strategySchema>;

export const strategyConfigSchema = z
  .object({
    config: z.record(z.string(), z.unknown()),
    created_at: z.iso.datetime().optional(),
    id: z.uuid(),
    is_active: z.boolean(),
    strategy_id: z.string(),
    strategy_version: z.string(),
    updated_at: z.iso.datetime().optional(),
  })
  .strict();

export const recommendationItemSchema = z
  .object({
    excluded: z.boolean(),
    exclusion_reason: z.string().nullable().optional(),
    factors: z.record(z.string(), z.unknown()).optional(),
    instrument_id: z.string(),
    rank: z.number().int().nullable().optional(),
    reason_codes: z.array(z.string()).optional(),
    target_weight: z.string().nullable().optional(),
  })
  .strict();

export type RecommendationItemModel = z.infer<typeof recommendationItemSchema>;

export const recommendationRunSchema = z
  .object({
    as_of: z.iso.date(),
    created_at: z.iso.datetime(),
    id: z.uuid(),
    items: z.array(recommendationItemSchema).optional(),
    job_id: z.uuid().nullable().optional(),
    provenance: z
      .object({
        dataset_manifest_sha256: z
          .string()
          .regex(/^[0-9a-f]{64}$/)
          .optional(),
        dataset_version_id: z.uuid().optional(),
      })
      .strict(),
    status: z.enum(["PENDING", "SUCCEEDED", "FAILED", "BLOCKED"]),
    strategy_config_id: z.uuid().nullable().optional(),
    summary: z.record(z.string(), z.unknown()).optional(),
    trigger_kind: z.enum(["MANUAL", "SCHEDULED"]),
  })
  .strict();

export type RecommendationRunModel = z.infer<typeof recommendationRunSchema>;

export const licensingStatusSchema = z
  .object({
    as_of: z.iso.date(),
    datasets: z.array(
      z
        .object({
          covered: z.boolean(),
          dataset_id: z.string(),
          effective_from: z.string().nullable().optional(),
          effective_until: z.string().nullable().optional(),
          state: z.enum(["PENDING", "ACTIVE", "EXPIRED", "REVOKED"]),
          use_kind: z.string(),
        })
        .strict(),
    ),
  })
  .strict();

export type LicensingStatusModel = z.infer<typeof licensingStatusSchema>;

export const reportProvenanceSchema = z
  .object({
    cash_weight: z
      .string()
      .regex(/^\d+(?:\.\d+)?$/)
      .optional(),
    data_version: z.string().default("Not reported"),
    dataset_id: z.string().optional(),
    dataset_version: z.string().optional(),
    engine_version: z.string().default("Not reported"),
    factor_snapshot_hash: z.string().optional(),
    manifest_sha256: z
      .string()
      .regex(/^[0-9a-f]{64}$/)
      .optional(),
    origin: z.enum(["credentialed", "synthetic"]).optional(),
    portfolio_snapshot_id: z.string().optional(),
    strategy_version: z.string().default("Not reported"),
    universe_snapshot_id: z.string().optional(),
    warnings: z.array(z.string()).default([]),
  })
  .passthrough();

export type ReportProvenance = z.infer<typeof reportProvenanceSchema>;

export type PageResult<Item> = {
  readonly has_more: boolean;
  readonly items: readonly Item[];
  readonly next_cursor: string | null;
};

export function pageSchema<Item>(item: z.ZodType<Item>) {
  return z
    .object({
      has_more: z.boolean(),
      items: z.array(item),
      next_cursor: z.string().nullable(),
    })
    .strict();
}

export const latestRecommendationSchema = z
  .object({
    latest_run: recommendationRunSchema,
    run: recommendationRunSchema.nullable(),
  })
  .strict();

export type RecommendationLatestModel = z.infer<typeof latestRecommendationSchema>;

export function recommendationProvenance(run: RecommendationRunModel): ReportProvenance {
  const parsed = reportProvenanceSchema.safeParse(run.summary);
  return parsed.success
    ? parsed.data
    : {
        data_version: "Not reported",
        engine_version: "Not reported",
        strategy_version: "Not reported",
        warnings: ["Version metadata was not reported by the server."],
      };
}

export function permitsUse(status: LicensingStatusModel, useKind: string): boolean {
  return status.datasets.some(
    (dataset) => dataset.use_kind === useKind && dataset.state === "ACTIVE" && dataset.covered,
  );
}
