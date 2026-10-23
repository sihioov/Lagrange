import type { components } from "@lagrange/api-contract";
import { z } from "zod";

export const ownerBetaDailyStateCodeSchema = z.enum([
  "DISABLED",
  "UNVERIFIABLE",
  "INPUT_PENDING",
  "INPUT_BLOCKED",
  "QUEUED",
  "RUNNING",
  "RETRYING",
  "FAILED",
  "CURRENT",
  "STALE",
]);

export const ownerBetaDailyReasonCodeSchema = z.enum([
  "RUNTIME_DISABLED",
  "CONFIG_DISABLED",
  "CONFIG_INACTIVE",
  "CONTROL_UNAVAILABLE",
  "CALENDAR_COVERAGE_GAP",
  "VERIFICATION_EXPIRED",
  "INPUT_UNAVAILABLE",
  "INPUT_NOT_READY",
  "BEFORE_ACTIVATION",
  "AWAITING_SCHEDULE",
  "JOB_QUEUED",
  "JOB_RUNNING",
  "JOB_RETRYING",
  "JOB_FAILED",
  "CAPACITY_EXCEEDED",
  "SUPERSEDED",
  "CONFIGURATION_CHANGED",
  "FUTURE_EVIDENCE",
  "SOURCE_CONFLICT",
  "SEALED_SOURCE_CHANGED",
  "SOURCE_REVOKED",
  "SOURCE_INTEGRITY",
  "CALENDAR_CONFLICT",
  "NONCASH_ACTION",
]);

const uuidSchema = z.uuid().refine((value) => value !== "00000000-0000-0000-0000-000000000000");
const timestampSchema = z.iso.datetime({ offset: true });

export const ownerBetaDailyStateSchema = z
  .object({
    enabled: z.boolean(),
    runtime_enabled: z.boolean(),
    expected_session: z.iso.date().nullable(),
    data_support_end: z.iso.date().nullable(),
    recommendation_as_of: z.iso.date().nullable(),
    published_at: timestampSchema.nullable(),
    input_kind: z.literal("owner_beta_daily_etf11_price_only_v1").nullable(),
    trigger_kind: z.literal("DAILY").nullable(),
    current_reservation_run_id: uuidSchema.nullable(),
    state: ownerBetaDailyStateCodeSchema,
    reason_code: ownerBetaDailyReasonCodeSchema.nullable(),
    data_due_at: timestampSchema.nullable(),
    observed_at: timestampSchema,
  })
  .strict()
  .superRefine((value, context) => {
    const hasPublication = value.published_at !== null;
    if (
      hasPublication !== (value.recommendation_as_of !== null) ||
      hasPublication !== (value.input_kind !== null) ||
      hasPublication !== (value.trigger_kind !== null)
    ) {
      context.addIssue({ code: "custom", message: "incomplete daily publication" });
    }
    if (
      value.published_at !== null &&
      Date.parse(value.published_at) > Date.parse(value.observed_at)
    ) {
      context.addIssue({ code: "custom", message: "future daily publication" });
    }
    if (
      value.state === "CURRENT" &&
      (!value.enabled ||
        !value.runtime_enabled ||
        value.expected_session === null ||
        value.data_support_end === null ||
        value.data_support_end < value.expected_session ||
        value.recommendation_as_of !== value.expected_session ||
        !hasPublication ||
        value.reason_code !== null)
    ) {
      context.addIssue({ code: "custom", message: "unproven current daily result" });
    }
  }) satisfies z.ZodType<components["schemas"]["OwnerBetaDailyState"]>;

export type OwnerBetaDailyStateModel = z.infer<typeof ownerBetaDailyStateSchema>;
export type OwnerBetaDailyStateCode = z.infer<typeof ownerBetaDailyStateCodeSchema>;
export type OwnerBetaDailyReasonCode = z.infer<typeof ownerBetaDailyReasonCodeSchema>;

export const OWNER_BETA_DAILY_STATE_PATH =
  "/api/v1/recommendations/owner-beta/price-only/daily-state";

export function ownerBetaDailyStatePath(configId: string): string {
  const query = new URLSearchParams({ strategy_config_id: uuidSchema.parse(configId) });
  return OWNER_BETA_DAILY_STATE_PATH + "?" + query.toString();
}

export function ownerBetaDailyConfigPath(configId: string): `/api/v1/strategy-configs/${string}` {
  return `/api/v1/strategy-configs/${uuidSchema.parse(configId)}`;
}
