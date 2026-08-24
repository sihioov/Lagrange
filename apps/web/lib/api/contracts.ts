import type { components, paths } from "@lagrange/api-contract";
import { z } from "zod";

export type ApiPath = keyof paths;
export type ApiSession = components["schemas"]["Session"];
export type ApiErrorCode = components["schemas"]["ErrorCode"];
export type ApiErrorEnvelope = components["schemas"]["ErrorEnvelope"];
export type OwnerBetaProduct = "recommendations" | "backtests" | "paper";

/** Web defense in depth only; the API admission middleware is authoritative. */
export function permitsOwnerBetaProduct(session: ApiSession, product: OwnerBetaProduct): boolean {
  if (session.owner_beta_access_mode === "disabled") {
    return true;
  }
  if (session.role === "member") {
    return false;
  }
  return product !== "paper" || session.owner_beta_paper_mode === "enabled";
}

export type ProductMutationPath =
  | "/api/v1/backtests"
  | "/api/v1/backtests/compare"
  | "/api/v1/recommendations/owner-beta/price-only/runs"
  | "/api/v1/recommendations/runs"
  | "/api/v1/screener/screens"
  | `/api/v1/backtests/${string}/cancel`
  | `/api/v1/backtests/${string}/robustness`
  | `/api/v1/admin/live/kill-switch/${string}`
  | `/api/v1/paper/accounts/${string}/bind-strategy`
  | `/api/v1/paper/accounts/${string}/recommendation-previews`
  | `/api/v1/paper/accounts/${string}/recommendation-previews/${string}/apply`
  | `/api/v1/screener/screens/${string}`
  | `/api/v1/strategies/${string}/configs`;

export const AUTH_API_PATHS = {
  csrf: "/api/v1/auth/csrf",
  logout: "/api/v1/auth/logout",
  session: "/api/v1/auth/session",
} as const satisfies Record<string, ApiPath>;

export const apiSessionSchema = z
  .object({
    user_id: z.uuid(),
    role: z.enum(["owner", "member"]),
    expires_at_secs: z.number().int(),
    auth_time_secs: z.number().int().exactOptional(),
    // An older API predates owner-beta policy, so an absent field denotes its
    // normal multi-user (`disabled`) contract. Unknown values and fields still
    // fail the strict parser instead of becoming an accidentally open mode.
    owner_beta_access_mode: z.enum(["disabled", "owner_only"]).default("disabled"),
    // The same legacy API also predates the separate Paper activation. Only
    // absence defaults; unknown present values remain contract failures.
    owner_beta_paper_mode: z.enum(["disabled", "enabled"]).default("disabled"),
  })
  .strict() satisfies z.ZodType<ApiSession>;

export const apiErrorEnvelopeSchema = z
  .object({
    error: z
      .object({
        code: z.enum([
          "SESSION_UNKNOWN",
          "SESSION_EXPIRED",
          "FORBIDDEN",
          "DATA_ENTITLEMENT_REQUIRED",
          "OWNER_ONLY_DEVELOPMENT_PATH",
          "CSRF_DENIED",
          "STEP_UP_NOT_OWNER",
          "STEP_UP_MFA_REQUIRED",
          "STEP_UP_AUTH_TIME_ABSENT",
          "STEP_UP_AUTH_TIME_STALE",
          "RESOURCE_NOT_FOUND",
          "INVALID_PARAMETER",
          "INVALID_DATE",
          "INVALID_DECIMAL",
          "INVALID_CURSOR",
          "IDEMPOTENCY_KEY_REQUIRED",
          "IDEMPOTENCY_KEY_MISMATCH",
          "DUPLICATE_RESOURCE",
          "PAYLOAD_TOO_LARGE",
          "DATASET_BLOCKED",
          "DATA_STALE",
          "INVALID_STRATEGY_PARAMETER",
          "UNSUPPORTED_MARKET_CURRENCY",
          "BACKTEST_CAPACITY_EXCEEDED",
          "RECOMMENDATION_CAPACITY_EXCEEDED",
          "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
          "OWNER_BETA_STRATEGY_UNSUPPORTED",
          "RESULT_INTEGRITY_FAILED",
          "LIVE_RECONCILIATION_REQUIRED",
          "LIVE_KILL_SWITCH_ENGAGED",
          "RISK_LIMIT_EXCEEDED",
          "ORDER_STATE_UNKNOWN",
          "NOT_IMPLEMENTED",
          "INTERNAL",
        ]),
        message: z.string(),
        request_id: z.string(),
        details: z.record(z.string(), z.unknown()).exactOptional(),
      })
      .strict(),
  })
  .strict() satisfies z.ZodType<ApiErrorEnvelope>;

export const csrfTokenSchema = z
  .object({
    csrf_token: z.string().min(1),
  })
  .strict();
