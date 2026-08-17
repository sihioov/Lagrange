import type { components } from "@lagrange/api-contract";
import { z } from "zod";

export type CandidateFeedContract = components["schemas"]["CandidateFeed"];
export type CandidateAnalysisContract = components["schemas"]["CandidateAnalysis"];
export type StockAnalysisContract = components["schemas"]["StockAnalysisResponse"];
export type ScreenerResultContract = components["schemas"]["ScreenerResult"];
export type SavedScreenContract = components["schemas"]["SavedScreen"];
export type CandidateLicenseAttributionContract =
  components["schemas"]["CandidateLicenseAttribution"];

export type UniverseKey = components["schemas"]["UniverseKey"];

export const UNIVERSE_KEYS = ["kospi200", "kosdaq150"] as const satisfies readonly UniverseKey[];
export const DEFAULT_UNIVERSE: UniverseKey = "kospi200";
export const UNIVERSE_LABELS: Readonly<Record<UniverseKey, string>> = {
  kospi200: "KOSPI 200",
  kosdaq150: "KOSDAQ 150",
};

export function universeLabel(universe: UniverseKey): string {
  return UNIVERSE_LABELS[universe];
}

export function isUniverseKey(value: string | undefined): value is UniverseKey {
  return value !== undefined && (UNIVERSE_KEYS as readonly string[]).includes(value);
}

const sha256Schema = z.string().regex(/^[0-9a-f]{64}$/);
const candidateSourcePinSchema = z
  .object({
    dataset_version_id: z.uuid(),
    manifest_sha256: sha256Schema,
  })
  .strict();

export const candidateDatasetPinsSchema = z
  .object({
    universe_snapshot_id: z.uuid(),
    price: z
      .object({
        dataset_version_id: z.uuid(),
        curated_version: z.number().int().min(1),
        manifest_sha256: sha256Schema,
      })
      .strict(),
    market_status: candidateSourcePinSchema,
    flow: candidateSourcePinSchema,
    fundamental: candidateSourcePinSchema,
    sector_version_id: z.uuid(),
    input_identity_sha256: sha256Schema,
  })
  .strict();

const candidateScoresSchema = z
  .object({
    flow: z.number().min(0).max(100).nullable(),
    fundamental: z.number().min(0).max(100).nullable(),
    technical: z.number().min(0).max(100).nullable(),
    total: z.number().min(0).max(100).nullable(),
  })
  .strict();

const candidateCoverageSchema = z
  .object({
    flow: z.number().min(0).max(1),
    fundamental: z.number().min(0).max(1),
    technical: z.number().min(0).max(1),
  })
  .strict();

export const candidateLicenseAttributionSchema = z
  .object({
    source: z.enum(["price", "universe", "market_status", "flow", "fundamental", "sector"]),
    dataset_id: z.string().regex(/^krx_[a-z0-9_]+$/),
    license_ref: z.string().min(1),
    entitlement_id: z.uuid(),
    contract_reference: z.string().min(1),
    contract_document_sha256: sha256Schema,
  })
  .strict();

export type CandidateLicenseAttribution = z.infer<typeof candidateLicenseAttributionSchema>;

export const candidateAnalysisSchema = z
  .object({
    analysis_id: z.uuid(),
    run_id: z.uuid(),
    universe: z.enum(["kospi200", "kosdaq150"]),
    instrument_id: z.string().regex(/^\d{6}\.KRX$/),
    name: z.string().nullable().optional(),
    sector_code: z.string().min(1),
    fundamental_profile: z.enum([
      "candidate-non-financial-v1",
      "candidate-financial-v1",
      "unsupported",
    ]),
    eligible: z.boolean(),
    exclusion_codes: z.array(z.string()),
    scores: candidateScoresSchema,
    coverage: candidateCoverageSchema,
    evidence_strength: z.enum(["STRONG", "MODERATE", "WEAK"]),
    rank: z.number().int().min(1).nullable().optional(),
    normalization_scope: z.enum(["SECTOR", "UNIVERSE_FALLBACK", "UNAVAILABLE"]),
    factors: z.record(z.string(), z.unknown()),
    scenarios: z.record(z.string(), z.unknown()),
    provenance: z.record(z.string(), z.unknown()),
    content_sha256: sha256Schema,
  })
  .strict();

export type CandidateAnalysis = z.infer<typeof candidateAnalysisSchema>;

const candidateEnvelopeFields = {
  // Row-bearing success payloads never carry BLOCKED. Blocked entitlement
  // responses are parsed as ApiProblem before a product contract is touched.
  universe: z.enum(["kospi200", "kosdaq150"]).nullable().optional(),
  state: z.enum(["READY", "STALE"]),
  as_of: z.iso.date(),
  cutoff_at: z.iso.datetime(),
  scoring_config: z
    .object({
      version: z.string().min(1),
      sha256: sha256Schema,
    })
    .strict(),
  dataset_pins: candidateDatasetPinsSchema,
  license_attributions: z.array(candidateLicenseAttributionSchema).min(1),
  disclaimer: z.string().min(1),
} as const;

export const candidateFeedSchema = z
  .object({
    ...candidateEnvelopeFields,
    feed_id: z.uuid(),
    universe: z.enum(["kospi200", "kosdaq150"]),
    published_at: z.iso.datetime(),
    computation_seq: z.number().int().min(1),
    items: z.array(candidateAnalysisSchema).length(5),
  })
  .strict();

export type CandidateFeed = z.infer<typeof candidateFeedSchema>;

export const stockAnalysisResponseSchema = z
  .object({
    ...candidateEnvelopeFields,
    universe: z.enum(["kospi200", "kosdaq150"]),
    analysis: candidateAnalysisSchema,
  })
  .strict();

export type StockAnalysisResponse = z.infer<typeof stockAnalysisResponseSchema>;

export const screenCriteriaSchema = z
  .object({
    universes: z.array(z.enum(["kospi200", "kosdaq150"])).optional(),
    sectors: z.array(z.string().min(1).max(32)).max(64).optional(),
    evidence_strength: z.array(z.enum(["STRONG", "MODERATE", "WEAK"])).optional(),
    min_total_score: z.number().min(0).max(100).nullable().optional(),
    min_flow_score: z.number().min(0).max(100).nullable().optional(),
    min_fundamental_score: z.number().min(0).max(100).nullable().optional(),
    min_technical_score: z.number().min(0).max(100).nullable().optional(),
  })
  .strict();

export type ScreenCriteria = z.infer<typeof screenCriteriaSchema>;

export function defaultUniverses(criteria: ScreenCriteria): ScreenCriteria {
  return {
    ...criteria,
    universes:
      criteria.universes === undefined || criteria.universes.length === 0
        ? [DEFAULT_UNIVERSE]
        : criteria.universes,
  };
}

export type ScreenerQuery = {
  readonly run_id?: string | null;
  readonly as_of?: string;
  readonly criteria: ScreenCriteria;
  readonly cursor?: string | null;
  readonly limit?: number | null;
};

export const screenerResultSchema = z
  .object({
    ...candidateEnvelopeFields,
    universe: z.enum(["kospi200", "kosdaq150"]).nullable().optional(),
    universes: z.array(z.enum(["kospi200", "kosdaq150"])).min(1),
    run_id: z.uuid().nullable().optional(),
    run_ids: z
      .array(z.object({ universe: z.enum(["kospi200", "kosdaq150"]), run_id: z.uuid() }).strict())
      .min(1),
    items: z.array(candidateAnalysisSchema),
    next_cursor: z.string().nullable(),
  })
  .strict();

export type ScreenerResult = z.infer<typeof screenerResultSchema>;

export const savedScreenSchema = z
  .object({
    id: z.uuid(),
    name: z.string().min(1).max(80),
    criteria_schema_version: z.union([z.literal(1), z.literal(2)]),
    criteria: screenCriteriaSchema,
    created_at: z.iso.datetime(),
    updated_at: z.iso.datetime(),
  })
  .strict();

export type SavedScreen = z.infer<typeof savedScreenSchema>;

export const savedScreenListSchema = z.object({ items: z.array(savedScreenSchema) }).strict();

export const deleteSavedScreenSchema = z
  .object({ id: z.uuid(), deleted: z.literal(true) })
  .strict();

export function candidateProfileLabel(profile: CandidateAnalysis["fundamental_profile"]): string {
  switch (profile) {
    case "candidate-financial-v1":
      return "Financial-company profile";
    case "candidate-non-financial-v1":
      return "Non-financial profile";
    case "unsupported":
      return "Unsupported profile";
  }
}
