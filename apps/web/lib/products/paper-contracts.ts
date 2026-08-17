import { z } from "zod";

/**
 * Paper account surfaces (Todo 32).
 *
 * Every schema is `.strict()`: an unexpected field from the server is a
 * contract break we want to see, not silently render.
 */

export const paperAccountSchema = z
  .object({
    account_type: z.literal("PAPER"),
    cost_profile_id: z.enum(["KRX_ETF_DEFAULT", "CUSTOM"]),
    cost_profile_version: z.number(),
    created_at: z.string(),
    currency: z.literal("KRW"),
    id: z.uuid(),
    initial_cash: z.string().nullable(),
    owner_user_id: z.uuid(),
    can_manage: z.boolean(),
    name: z.string(),
    status: z.enum(["ACTIVE", "SUSPENDED", "CLOSED"]),
    updated_at: z.string(),
  })
  .strict();

export const paperPositionSchema = z
  .object({
    avg_price: z.string().nullable(),
    instrument_id: z.string(),
    quantity: z.string(),
    updated_at: z.string(),
  })
  .strict();

export const paperOrderSchema = z
  .object({
    created_at: z.string(),
    id: z.uuid(),
    instrument_id: z.string(),
    order_ref: z.string(),
    price: z.string().nullable(),
    quantity: z.string(),
    side: z.enum(["BUY", "SELL"]),
    status: z.string(),
    submitted_at: z.string().nullable(),
  })
  .strict();

const performancePointSchema = z
  .object({
    cash: z.string(),
    currency: z.string(),
    equity: z.string(),
    positions_value: z.string(),
    return_pct: z.string().nullable(),
    trading_date: z.string(),
  })
  .strict();

export const paperPerformanceSchema = z
  .object({
    account_id: z.uuid(),
    disclaimer: z.string(),
    points: z.array(performancePointSchema),
  })
  .strict();

const bindingSchema = z
  .object({
    active: z.boolean(),
    bound_at: z.string(),
    strategy_config_id: z.uuid(),
    strategy_id: z.string(),
    strategy_version: z.string(),
    unbound_at: z.string().nullable(),
  })
  .strict();

const targetLineageSchema = z
  .object({
    computed_on: z.string(),
    effective_date: z.string(),
    executed_at: z.string().nullable(),
    id: z.uuid(),
    status: z.enum(["PENDING", "EXECUTED", "SKIPPED"]),
  })
  .strict();

export const paperLineageSchema = z
  .object({
    account_id: z.uuid(),
    bindings: z.array(bindingSchema),
    targets: z.array(targetLineageSchema),
  })
  .strict();

const lineageFieldSchema = z
  .object({
    backtest: z.string(),
    field: z.string(),
    paper: z.string(),
  })
  .strict();

const divergenceSchema = z
  .object({
    backtest_weight: z.string().nullable(),
    instrument_id: z.string(),
    paper_weight: z.string().nullable(),
  })
  .strict();

export const paperParitySchema = z
  .object({
    account_id: z.uuid(),
    as_of: z.string(),
    divergences: z.array(divergenceSchema),
    fill_model_difference: z.string(),
    lineage: z.object({ fields: z.array(lineageFieldSchema) }).strict(),
    status: z.enum(["MATCH", "DIVERGENT", "NOT_COMPARABLE"]),
    warrants_alert: z.boolean(),
  })
  .strict();

export const strategyConfigSchema = z
  .object({
    config: z.unknown().optional(),
    created_at: z.string(),
    id: z.uuid(),
    is_active: z.boolean(),
    strategy_id: z.string(),
    strategy_version: z.string(),
    updated_at: z.string(),
  })
  .strict();

export const bindStrategySchema = z
  .object({
    account_id: z.uuid(),
    bound_at: z.string(),
    strategy_config_id: z.uuid(),
    strategy_id: z.string(),
    strategy_version: z.string(),
  })
  .strict();

const deliverySchema = z
  .object({
    channel: z.enum(["web", "email", "admin"]),
    error_detail: z.string().optional(),
    status: z.enum(["SUCCESS", "FAILED"]),
  })
  .strict();

export const notificationSchema = z
  .object({
    body: z.string(),
    created_at: z.string(),
    deliveries: z.array(deliverySchema),
    id: z.uuid(),
    kind: z.enum(["job", "recommendation", "backtest", "alert"]),
    read_at: z.string().optional(),
    title: z.string(),
  })
  .strict();

export type PaperAccountModel = z.infer<typeof paperAccountSchema>;
export type StrategyConfigModel = z.infer<typeof strategyConfigSchema>;
export type NotificationModel = z.infer<typeof notificationSchema>;
export type NotificationDeliveryModel = z.infer<typeof deliverySchema>;
export type PaperPerformanceModel = z.infer<typeof paperPerformanceSchema>;
export type PaperLineageModel = z.infer<typeof paperLineageSchema>;
export type PaperParityModel = z.infer<typeof paperParitySchema>;
export type PaperPositionModel = z.infer<typeof paperPositionSchema>;
export type PaperOrderModel = z.infer<typeof paperOrderSchema>;

/**
 * The account a session's reports should default to.
 *
 * Prefer the current user's own active account, then any owned account,
 * before falling back to a shared account.
 */
export function defaultAccount(accounts: readonly PaperAccountModel[]): PaperAccountModel | null {
  return (
    accounts.find((account) => account.can_manage && account.status === "ACTIVE") ??
    accounts.find((account) => account.can_manage) ??
    accounts.find((account) => account.status === "ACTIVE") ??
    accounts[0] ??
    null
  );
}

/**
 * The human-readable reason a parity report reached its status. Rendered
 * next to the badge so a reader never has to interpret the enum alone.
 */
export function parityReason(parity: PaperParityModel): string {
  if (parity.status === "MATCH") {
    return "Paper executed the same signals the backtest produced for this session.";
  }
  if (parity.status === "DIVERGENT") {
    return `Paper and the backtest produced different target weights for ${parity.divergences.length} instrument(s).`;
  }
  const mismatched = parity.lineage.fields
    .filter((field) => field.backtest !== field.paper)
    .map((field) => field.field);
  return mismatched.length > 0
    ? `The two sides came from different inputs (${mismatched.join(", ")}), so no parity claim is possible.`
    : "The two sides could not be compared.";
}
