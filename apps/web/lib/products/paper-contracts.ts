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
    owner_beta_daily_enabled: z.boolean().optional(),
    owner_beta_daily_epoch: z.number().int().nonnegative().refine(Number.isSafeInteger).optional(),
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

const paperBetaUuidSchema = z.uuid();
const paperBetaRequestUuidSchema = z
  .string()
  .regex(
    /^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
  );
const paperBetaHashSchema = z.string().regex(/^sha256:[0-9a-f]{64}$/);
const paperBetaDecimalSchema = z.string().regex(/^-?\d+(\.\d+)?$/);
const paperBetaDateSchema = z.iso.date();
const paperBetaTimestampSchema = z.iso.datetime({ offset: true });

const ownerBetaPaperSessionStateSchema = z.enum([
  "PENDING",
  "WAITING_PRICE",
  "WAITING_ADMISSION",
  "HELD",
  "BLOCKED",
  "RECOVERY_REQUIRED",
  "SETTLED_TRADE",
  "SETTLED_NO_TRADE",
  "VALUED",
]);

const ownerBetaPaperSessionSummarySchema = z
  .object({
    pending_target_id: paperBetaUuidSchema,
    effective_session: paperBetaDateSchema,
    state: ownerBetaPaperSessionStateSchema,
    settlement_kind: z.enum(["TRADE", "NO_TRADE"]).nullable(),
    reason_code: z
      .string()
      .regex(/^[A-Z][A-Z0-9_]{0,63}$/)
      .nullable(),
    source_original_retrieved_at: paperBetaTimestampSchema.nullable(),
    input_admitted_at: paperBetaTimestampSchema.nullable(),
    usable_at: paperBetaTimestampSchema.nullable(),
    settled_at: paperBetaTimestampSchema.nullable(),
    valued_at: paperBetaTimestampSchema.nullable(),
  })
  .strict();

const ownerBetaPaperDailyPinsSchema = z
  .object({
    admission_sha256: paperBetaHashSchema,
    content_sha256: paperBetaHashSchema,
    manifest_sha256: paperBetaHashSchema,
    policy_sha256: paperBetaHashSchema,
  })
  .strict();

const ownerBetaPaperLineageSchema = z
  .object({
    recommendation_run_id: paperBetaUuidSchema,
    strategy_config_id: paperBetaUuidSchema,
    signal_date: paperBetaDateSchema,
    effective_session: paperBetaDateSchema,
    control_epoch: z.number().int().nonnegative(),
    config_epoch: z.number().int().nonnegative(),
    daily_activation_epoch: z.number().int().nonnegative(),
    cohort_sha256: paperBetaHashSchema,
    daily_pins: ownerBetaPaperDailyPinsSchema,
    factor_snapshot_sha256: paperBetaHashSchema,
    target_snapshot_sha256: paperBetaHashSchema,
    signal_allocation_sha256: paperBetaHashSchema,
    target_weights: z.array(
      z.object({ instrument_id: z.string(), weight: paperBetaDecimalSchema }).strict(),
    ),
  })
  .strict();

const ownerBetaPaperCalculationResultSchema = z
  .object({
    schema: z.literal("owner-beta-paper-preview-result-v1"),
    target_snapshot_sha256: paperBetaHashSchema,
    signal_allocation_sha256: paperBetaHashSchema,
    plan_status: z.enum(["REBALANCE", "NO_TRADE"]),
    no_trade_reasons: z.array(z.string().regex(/^[A-Z][A-Z0-9_]{0,63}$/)),
    equity: paperBetaDecimalSchema,
    cash_before: paperBetaDecimalSchema,
    available_cash: paperBetaDecimalSchema,
    leftover_cash: paperBetaDecimalSchema,
    buy_notional: paperBetaDecimalSchema,
    sell_notional: paperBetaDecimalSchema,
    explicit_fees: paperBetaDecimalSchema,
    informational_slippage: paperBetaDecimalSchema,
    decisions: z.array(rebalancePreviewDecisionSchema),
    orders: z.array(rebalancePreviewOrderSchema),
    cost_profile_id: z.string(),
    cost_profile_version: z.number().int().positive(),
    price_observations: z.array(
      z
        .object({
          instrument_id: z.string(),
          session_date: paperBetaDateSchema,
          observed_at: paperBetaTimestampSchema,
        })
        .strict(),
    ),
  })
  .strict();

export const ownerBetaPaperStateSchema = z
  .object({
    schema: z.literal("owner-beta-paper-state-v1"),
    state: z.enum(["DISABLED", "UNBOUND", "OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"]),
    account_id: paperBetaUuidSchema.nullable(),
    strategy_config_id: paperBetaUuidSchema.nullable(),
    strategy_id: z.string().nullable(),
    strategy_version: z.string().nullable(),
    control_epoch: z.number().int().nonnegative().nullable(),
    consent_present: z.boolean(),
    daily_cohort_sha256: paperBetaHashSchema.nullable(),
    process_binding_sha256: paperBetaHashSchema.nullable(),
    sessions: z.array(ownerBetaPaperSessionSummarySchema).max(20),
  })
  .strict();

export const ownerBetaPaperConsentBodySchema = z
  .object({ opt_in: z.boolean(), expected_epoch: z.number().int().nonnegative() })
  .strict();

export const ownerBetaPaperPreviewBodySchema = z
  .object({
    recommendation_run_id: paperBetaRequestUuidSchema,
    expected_epoch: z.number().int().nonnegative(),
  })
  .strict();

export const ownerBetaPaperApplyBodySchema = z
  .object({
    preview_token: z.string().regex(/^[0-9a-f]{64}$/),
    expected_epoch: z.number().int().nonnegative(),
  })
  .strict();

export const ownerBetaPaperConsentSchema = z
  .object({
    schema: z.literal("owner-beta-paper-consent-v1"),
    account_id: paperBetaUuidSchema,
    event_id: paperBetaUuidSchema,
    control_epoch: z.number().int().nonnegative(),
    opt_in: z.boolean(),
    consent_present: z.boolean(),
    replayed: z.boolean(),
  })
  .strict();

export const ownerBetaPaperPreviewSchema = z
  .object({
    schema: z.literal("owner-beta-paper-preview-v1"),
    preview_id: paperBetaUuidSchema,
    job_id: paperBetaUuidSchema,
    account_id: paperBetaUuidSchema,
    status: z.enum(["PENDING", "RUNNING", "READY", "FAILED", "APPLIED"]),
    signal_date: paperBetaDateSchema,
    effective_session: paperBetaDateSchema,
    control_epoch: z.number().int().nonnegative(),
    config_epoch: z.number().int().nonnegative(),
    daily_activation_epoch: z.number().int().nonnegative(),
    lineage: ownerBetaPaperLineageSchema,
    preview_token: z
      .string()
      .regex(/^[0-9a-f]{64}$/)
      .nullable(),
    cost_profile_id: z.string().nullable(),
    cost_profile_version: z.number().int().positive().nullable(),
    result: ownerBetaPaperCalculationResultSchema.nullable(),
    error: z
      .object({ code: z.string().regex(/^[A-Z][A-Z0-9_]{0,63}$/) })
      .strict()
      .nullable(),
    created_at: paperBetaTimestampSchema,
    started_at: paperBetaTimestampSchema.nullable(),
    completed_at: paperBetaTimestampSchema.nullable(),
    applied_at: paperBetaTimestampSchema.nullable(),
    updated_at: paperBetaTimestampSchema,
  })
  .strict()
  .superRefine((preview, context) => {
    if (preview.signal_date !== preview.lineage.signal_date) {
      context.addIssue({
        code: "custom",
        path: ["lineage", "signal_date"],
        message: "Preview and lineage signal dates must match.",
      });
    }
    if (preview.effective_session !== preview.lineage.effective_session) {
      context.addIssue({
        code: "custom",
        path: ["lineage", "effective_session"],
        message: "Preview and lineage economic sessions must match.",
      });
    }
    if (preview.control_epoch !== preview.lineage.control_epoch) {
      context.addIssue({
        code: "custom",
        path: ["lineage", "control_epoch"],
        message: "Preview and lineage control epochs must match.",
      });
    }
    for (const [index, observation] of preview.result?.price_observations.entries() ?? []) {
      if (observation.session_date !== preview.signal_date) {
        context.addIssue({
          code: "custom",
          path: ["result", "price_observations", index, "session_date"],
          message: "Price observations must use the recommendation signal date.",
        });
      }
    }
  });

export const ownerBetaPaperApplySchema = z
  .object({
    schema: z.literal("owner-beta-paper-apply-v1"),
    preview_id: paperBetaUuidSchema,
    pending_target_id: paperBetaUuidSchema,
    lineage_id: paperBetaUuidSchema,
    effective_session: paperBetaDateSchema,
    control_epoch: z.number().int().nonnegative(),
    status: z.literal("APPLIED"),
    replayed: z.boolean(),
  })
  .strict();

export const ownerBetaPaperSessionDetailSchema = z
  .object({
    schema: z.literal("owner-beta-paper-session-v1"),
    pending_target_id: paperBetaUuidSchema,
    signal_date: paperBetaDateSchema,
    effective_session: paperBetaDateSchema,
    state: ownerBetaPaperSessionStateSchema,
    processing_origin: z.enum(["MANUAL", "AUTO", "RECOVERY"]),
    lineage: ownerBetaPaperLineageSchema,
    source_original_retrieved_at: paperBetaTimestampSchema.nullable(),
    input_admitted_at: paperBetaTimestampSchema.nullable(),
    usable_at: paperBetaTimestampSchema.nullable(),
    settled_at: paperBetaTimestampSchema.nullable(),
    valued_at: paperBetaTimestampSchema.nullable(),
    settlement_kind: z.enum(["TRADE", "NO_TRADE"]).nullable(),
    reason_code: z
      .string()
      .regex(/^[A-Z][A-Z0-9_]{0,63}$/)
      .nullable(),
    amounts: z
      .object({
        cash_after: paperBetaDecimalSchema.nullable(),
        valuation_cash: paperBetaDecimalSchema.nullable(),
        positions_value: paperBetaDecimalSchema.nullable(),
        equity: paperBetaDecimalSchema.nullable(),
      })
      .strict(),
    order_references: z.array(
      z.object({ order_ref: paperBetaUuidSchema, fill_ref: paperBetaUuidSchema }).strict(),
    ),
    comparison: z
      .object({
        disposition: z.literal("NOT_COMPARABLE"),
        reason: z.literal("NO_MATCHED_BACKTEST_EXECUTION_CONTRACT"),
      })
      .strict(),
  })
  .strict()
  .superRefine((session, context) => {
    if (session.signal_date !== session.lineage.signal_date) {
      context.addIssue({
        code: "custom",
        path: ["lineage", "signal_date"],
        message: "Session and lineage signal dates must match.",
      });
    }
    if (session.effective_session !== session.lineage.effective_session) {
      context.addIssue({
        code: "custom",
        path: ["lineage", "effective_session"],
        message: "Session and lineage economic sessions must match.",
      });
    }
  });

export type OwnerBetaPaperStateModel = z.infer<typeof ownerBetaPaperStateSchema>;
export type OwnerBetaPaperConsentBody = z.infer<typeof ownerBetaPaperConsentBodySchema>;
export type OwnerBetaPaperPreviewBody = z.infer<typeof ownerBetaPaperPreviewBodySchema>;
export type OwnerBetaPaperApplyBody = z.infer<typeof ownerBetaPaperApplyBodySchema>;
export type OwnerBetaPaperConsentModel = z.infer<typeof ownerBetaPaperConsentSchema>;
export type OwnerBetaPaperPreviewModel = z.infer<typeof ownerBetaPaperPreviewSchema>;
export type OwnerBetaPaperApplyModel = z.infer<typeof ownerBetaPaperApplySchema>;
export type OwnerBetaPaperSessionDetailModel = z.infer<typeof ownerBetaPaperSessionDetailSchema>;

export const OWNER_BETA_PAPER_STATE_PATH = "/api/v1/paper/owner-beta/state" as const;
export const OWNER_BETA_PAPER_CONSENT_PATH = "/api/v1/paper/owner-beta/consent" as const;
export const OWNER_BETA_PAPER_PREVIEWS_PATH = "/api/v1/paper/owner-beta/previews" as const;

export function ownerBetaPaperPreviewPath(
  previewId: string,
): `/api/v1/paper/owner-beta/previews/${string}` {
  return `${OWNER_BETA_PAPER_PREVIEWS_PATH}/${encodeURIComponent(previewId)}`;
}

export function ownerBetaPaperApplyPath(
  previewId: string,
): `/api/v1/paper/owner-beta/previews/${string}/apply` {
  return `${ownerBetaPaperPreviewPath(previewId)}/apply`;
}

export function ownerBetaPaperSessionPath(pendingTargetId: string): string {
  return `/api/v1/paper/owner-beta/sessions/${encodeURIComponent(pendingTargetId)}`;
}

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
