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

export const rebalancePreviewErrorSchema = z
  .object({
    code: z.string(),
    message: z.string(),
  })
  .strict();

export const rebalancePreviewDecisionSchema = z
  .object({
    action: z.enum(["BUY", "SELL", "SKIP"]),
    current_quantity: z.string(),
    current_value: z.string(),
    current_weight: z.string(),
    delta_value: z.string(),
    instrument_id: z.string(),
    skip_reason: z
      .enum([
        "BELOW_REBALANCE_THRESHOLD",
        "BELOW_MIN_TRADE",
        "NO_AVAILABLE_CASH",
        "NO_AFFORDABLE_LOT",
      ])
      .nullable(),
    target_value: z.string(),
    target_weight: z.string(),
  })
  .strict();

export const rebalancePreviewOrderSchema = z
  .object({
    commission: z.string(),
    estimated_execution_price: z.string(),
    informational_slippage: z.string(),
    instrument_id: z.string(),
    notional: z.string(),
    quantity: z.string(),
    raw_price: z.string(),
    side: z.enum(["BUY", "SELL"]),
    tax: z.string(),
  })
  .strict();

export const rebalancePreviewLineageSchema = z
  .object({
    account_id: z.uuid(),
    account_state_sha256: z.string().regex(/^[0-9a-f]{64}$/),
    account_state_version: z.number(),
    curated_version: z.number(),
    dataset_manifest_sha256: z.string().regex(/^[0-9a-f]{64}$/),
    dataset_version_id: z.uuid(),
    recommendation_run_id: z.uuid(),
    strategy_config_id: z.uuid(),
    target_portfolio_id: z.uuid(),
    target_portfolio_sha256: z.string().regex(/^[0-9a-f]{64}$/),
  })
  .strict();

export const rebalancePreviewResultSchema = z
  .object({
    available_cash: z.string(),
    buy_notional: z.string(),
    cash_before: z.string(),
    decisions: z.array(rebalancePreviewDecisionSchema),
    equity: z.string(),
    explicit_fees: z.string(),
    informational_slippage: z.string(),
    leftover_cash: z.string(),
    lineage: rebalancePreviewLineageSchema,
    orders: z.array(rebalancePreviewOrderSchema),
    price_basis: z.literal("RECOMMENDATION_CLOSE"),
    price_date: z.string(),
    proposed_effective_date: z.string(),
    schema_version: z.literal(1),
    sell_notional: z.string(),
    warning_code: z.literal("INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED"),
  })
  .strict();

export const rebalancePreviewSchema = z
  .object({
    account_id: z.uuid(),
    applied_at: z.string().nullable(),
    completed_at: z.string().nullable(),
    created_at: z.string(),
    dataset_manifest_sha256: z.string().regex(/^[0-9a-f]{64}$/),
    dataset_version_id: z.uuid(),
    error: rebalancePreviewErrorSchema.optional(),
    id: z.uuid(),
    job_id: z.uuid(),
    preview_token: z
      .string()
      .regex(/^[0-9a-f]{64}$/)
      .nullable(),
    price_basis: z.literal("RECOMMENDATION_CLOSE"),
    price_date: z.string(),
    proposed_effective_date: z.string().nullable(),
    recommendation_run_id: z.uuid(),
    result: rebalancePreviewResultSchema.optional(),
    started_at: z.string().nullable(),
    status: z.enum(["PENDING", "RUNNING", "READY", "FAILED", "APPLIED"]),
    strategy_config_id: z.uuid(),
    target_portfolio_id: z.uuid(),
    target_portfolio_sha256: z.string().regex(/^[0-9a-f]{64}$/),
    updated_at: z.string(),
  })
  .strict();

export const appliedRebalancePreviewSchema = z
  .object({
    effective_date: z.string(),
    pending_target_id: z.uuid(),
    preview_id: z.uuid(),
    source_kind: z.literal("MANUAL_RECOMMENDATION"),
    status: z.literal("APPLIED"),
  })
  .strict();

export type RebalancePreviewErrorModel = z.infer<typeof rebalancePreviewErrorSchema>;
export type RebalancePreviewDecisionModel = z.infer<typeof rebalancePreviewDecisionSchema>;
export type RebalancePreviewOrderModel = z.infer<typeof rebalancePreviewOrderSchema>;
export type RebalancePreviewLineageModel = z.infer<typeof rebalancePreviewLineageSchema>;
export type RebalancePreviewResultModel = z.infer<typeof rebalancePreviewResultSchema>;
export type RebalancePreviewModel = z.infer<typeof rebalancePreviewSchema>;
export type AppliedRebalancePreviewModel = z.infer<typeof appliedRebalancePreviewSchema>;

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
