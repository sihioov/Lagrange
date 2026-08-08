import type { components, paths } from "@lagrange/api-contract";
import { z } from "zod";

export type ApiPath = keyof paths;
export type ApiSession = components["schemas"]["Session"];
export type ApiErrorCode = components["schemas"]["ErrorCode"];
export type ApiErrorEnvelope = components["schemas"]["ErrorEnvelope"];

export type ProductMutationPath =
  | "/api/v1/backtests"
  | "/api/v1/backtests/compare"
  | "/api/v1/recommendations/runs"
  | `/api/v1/backtests/${string}/cancel`
  | `/api/v1/backtests/${string}/robustness`
  | `/api/v1/admin/live/kill-switch/${string}`
  | `/api/v1/paper/accounts/${string}/bind-strategy`
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
