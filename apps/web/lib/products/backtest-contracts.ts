import { z } from "zod";
import { reportProvenanceSchema } from "./contracts";

export const backtestRunSchema = z
  .object({
    benchmark: z.string().nullable().optional(),
    config_sha256: z.string().optional(),
    created_at: z.iso.datetime().optional(),
    dataset_version: z.string().optional(),
    end_date: z.iso.date().nullable().optional(),
    engine: z.string().optional(),
    engine_version: z.string().optional(),
    finished_at: z.iso.datetime().nullable().optional(),
    id: z.uuid(),
    job_id: z.uuid().nullable().optional(),
    start_date: z.iso.date().nullable().optional(),
    started_at: z.iso.datetime().nullable().optional(),
    status: z.enum(["PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"]),
    strategy_id: z.string(),
    strategy_version: z.string(),
    summary: z.record(z.string(), z.unknown()).optional(),
  })
  .strict();

export type BacktestRunModel = z.infer<typeof backtestRunSchema>;

const robustnessEvidenceSchema = z
  .object({
    cost_stress: z.string(),
    parameter_sensitivity: z.string(),
    validation_periods: z.string(),
  })
  .strict();

const backtestSummarySchema = z
  .object({
    cost_profile_id: z.string().optional(),
    dataset_version_id: z.uuid().optional(),
    execution_profile: z.string().optional(),
    progress_percent: z.string().optional(),
    robustness_evidence: robustnessEvidenceSchema.optional(),
    run_label: z.string().optional(),
    strategy_config_id: z.uuid().optional(),
  })
  .passthrough();

export type BacktestCreationDefaults = {
  readonly benchmark: string;
  readonly costProfileId: string;
  readonly datasetVersionId: string;
  readonly executionProfile: string;
  readonly strategyConfigId: string;
};

export function backtestCreationDefaults(run: BacktestRunModel): BacktestCreationDefaults | null {
  const parsed = backtestSummarySchema.safeParse(run.summary);
  if (!parsed.success) {
    return null;
  }
  const summary = parsed.data;
  if (
    summary.cost_profile_id === undefined ||
    summary.dataset_version_id === undefined ||
    summary.execution_profile === undefined ||
    summary.strategy_config_id === undefined ||
    run.benchmark === undefined ||
    run.benchmark === null
  ) {
    return null;
  }
  return {
    benchmark: run.benchmark,
    costProfileId: summary.cost_profile_id,
    datasetVersionId: summary.dataset_version_id,
    executionProfile: summary.execution_profile,
    strategyConfigId: summary.strategy_config_id,
  };
}

export function backtestRunLabel(run: BacktestRunModel): string {
  const parsed = backtestSummarySchema.safeParse(run.summary);
  return parsed.success && parsed.data.run_label !== undefined
    ? parsed.data.run_label
    : `${run.strategy_id} ${run.start_date ?? "open"}-${run.end_date ?? "open"}`;
}

export function backtestProgress(run: BacktestRunModel): string | null {
  const parsed = backtestSummarySchema.safeParse(run.summary);
  return parsed.success ? (parsed.data.progress_percent ?? null) : null;
}

export function backtestRobustness(run: BacktestRunModel) {
  const parsed = backtestSummarySchema.safeParse(run.summary);
  return parsed.success ? (parsed.data.robustness_evidence ?? null) : null;
}

export const backtestPageSchema = z
  .object({
    has_more: z.boolean(),
    items: z.array(backtestRunSchema),
    next_cursor: z.string().nullable(),
  })
  .strict();

export const metricSchema = z.object({ metric_key: z.string(), metric_value: z.string() }).strict();

export const backtestMetricsSchema = z.object({ items: z.array(metricSchema) }).strict();
export type BacktestMetricsModel = z.infer<typeof backtestMetricsSchema>;

const seriesPointSchema = z.object({ date: z.iso.date(), value: z.string() }).strict();
const monthlyReturnSchema = z
  .object({ month: z.string().regex(/^\d{4}-\d{2}$/), value: z.string() })
  .strict();

export const artifactSchema = z
  .object({
    artifact_type: z.enum([
      "EQUITY_CURVE",
      "DRAWDOWN_CURVE",
      "MONTHLY_RETURNS",
      "ORDERS",
      "FILLS",
      "POSITIONS",
      "CASH_LEDGER",
      "FEES",
      "BENCHMARK",
    ]),
    download_path: z.string(),
    id: z.uuid(),
    row_count: z.number().int().nonnegative(),
    run_id: z.uuid(),
    sha256: z.string(),
    size_bytes: z.number().int().nonnegative(),
    summary: z.record(z.string(), z.unknown()).optional(),
  })
  .strict();

export const backtestEquitySchema = z
  .object({
    artifact: artifactSchema,
    run_id: z.uuid(),
    summary: z
      .object({
        drawdown_curve: z.array(seriesPointSchema),
        equity_curve: z.array(seriesPointSchema),
        monthly_returns: z.array(monthlyReturnSchema),
      })
      .strict(),
  })
  .strict();

export type BacktestEquityModel = z.infer<typeof backtestEquitySchema>;

export const backtestTradeSchema = z
  .object({
    cost: z.string(),
    executed_at: z.iso.datetime(),
    instrument_id: z.string(),
    quantity: z.string(),
    side: z.enum(["BUY", "SELL"]),
    trade_id: z.string(),
  })
  .strict();

export const backtestTradesSchema = z
  .object({
    has_more: z.boolean(),
    items: z.array(backtestTradeSchema),
    next_cursor: z.string().nullable(),
    total_count: z.number().int().nonnegative(),
  })
  .strict();

export type BacktestTradesModel = z.infer<typeof backtestTradesSchema>;

export const backtestCompareSchema = z
  .object({
    deltas: z.object({ total_return: z.string() }).catchall(z.string()),
    run_ids: z.array(z.uuid()).min(2),
    runs: z.array(
      z
        .object({
          run_id: z.uuid(),
          status: z.string(),
          strategy_id: z.string(),
          summary: z.record(z.string(), z.unknown()),
        })
        .strict(),
    ),
  })
  .strict();

export type BacktestCompareModel = z.infer<typeof backtestCompareSchema>;

export const cancelBacktestSchema = z
  .object({
    job_id: z.uuid().nullable().optional(),
    run_id: z.uuid(),
    status: z.literal("CANCEL_REQUESTED"),
  })
  .strict();

const robustnessChildSchema = z
  .object({
    axis: z.string(),
    job_id: z.uuid(),
    run_id: z.uuid(),
    status: z.enum(["QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"]),
  })
  .strict();

export const robustnessQueuedSchema = z
  .object({
    children: z.array(robustnessChildSchema),
    run_id: z.uuid(),
    suite_id: z.uuid(),
  })
  .strict();

export const backtestCreateSchema = backtestRunSchema;

export type BacktestReportModel = {
  readonly equity: BacktestEquityModel;
  readonly metrics: BacktestMetricsModel;
  readonly provenance: z.infer<typeof reportProvenanceSchema>;
  readonly run: BacktestRunModel;
  readonly trades: BacktestTradesModel;
};

export function backtestProvenance(run: BacktestRunModel) {
  const parsed = reportProvenanceSchema.safeParse(run.summary);
  return parsed.success
    ? parsed.data
    : {
        data_version: run.dataset_version ?? "Not reported",
        engine_version: run.engine_version ?? "Not reported",
        strategy_version: `${run.strategy_id}@${run.strategy_version}`,
        warnings: ["Complete version metadata was not reported by the server."],
      };
}

export function metricValue(metrics: BacktestMetricsModel, key: string): string | null {
  return metrics.items.find((metric) => metric.metric_key === key)?.metric_value ?? null;
}
