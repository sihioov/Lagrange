import type { KyInstance } from "ky";
import type { z } from "zod";
import {
  type BacktestReportModel,
  type BacktestRunModel,
  backtestEquitySchema,
  backtestMetricsSchema,
  backtestPageSchema,
  backtestProvenance,
  backtestTradesSchema,
} from "@/lib/products/backtest-contracts";
import {
  type CandidateFeed,
  candidateFeedSchema,
  DEFAULT_UNIVERSE,
  type SavedScreen,
  type ScreenerQuery,
  type ScreenerResult,
  type StockAnalysisResponse,
  savedScreenListSchema,
  screenerResultSchema,
  stockAnalysisResponseSchema,
  type UniverseKey,
} from "@/lib/products/candidate-contracts";
import {
  type LicensingStatusModel,
  latestRecommendationSchema,
  licensingStatusSchema,
  type PageResult,
  pageSchema,
  type RecommendationLatestModel,
  type RecommendationRunModel,
  recommendationRunSchema,
  type StrategyCatalogItem,
  strategySchema,
} from "@/lib/products/contracts";
import {
  OWNER_BETA_EQUITY_SIGNALS_LATEST_PATH,
  OWNER_BETA_EQUITY_SIGNALS_SCREEN_PATH,
  type OwnerBetaEquitySignalsDetailModel,
  type OwnerBetaEquitySignalsLatestModel,
  type OwnerBetaEquitySignalsScreenBody,
  type OwnerBetaEquitySignalsScreenModel,
  ownerBetaEquitySignalsDetailPath,
  ownerBetaEquitySignalsDetailSchema,
  ownerBetaEquitySignalsLatestSchema,
  ownerBetaEquitySignalsScreenBodySchema,
  ownerBetaEquitySignalsScreenSchema,
} from "@/lib/products/equity-signals-contracts";
import { type LiveConnectionModel, liveConnectionSchema } from "@/lib/products/live-contracts";
import {
  OWNER_BETA_PRICE_ONLY_RUNS_PATH,
  OWNER_BETA_PRICE_ONLY_SUPPORTED_AS_OF_PATH,
  type OwnerBetaRunModel,
  type OwnerBetaRunPageModel,
  type OwnerBetaSupportedAsOfModel,
  ownerBetaRunPageSchema,
  ownerBetaRunPath,
  ownerBetaRunSchema,
  ownerBetaSupportedAsOfSchema,
} from "@/lib/products/owner-beta-contracts";
import {
  type NotificationModel,
  notificationSchema,
  type PaperAccountModel,
  type PaperLineageModel,
  type PaperParityModel,
  type PaperPerformanceModel,
  paperAccountSchema,
  paperLineageSchema,
  paperOrderSchema,
  paperParitySchema,
  paperPerformanceSchema,
  paperPositionSchema,
  type RebalancePreviewModel,
  rebalancePreviewSchema,
  type StrategyConfigModel,
  strategyConfigSchema,
} from "@/lib/products/paper-contracts";
import { parseApiResponse } from "./response";
import { createServerTransport, type ServerApiClientOptions } from "./server-client";

export type ProductApiClient = {
  readonly getBacktestReport: (run: BacktestRunModel) => Promise<BacktestReportModel>;
  readonly getBacktestRuns: () => Promise<z.infer<typeof backtestPageSchema>>;
  readonly getLatestRecommendation: () => Promise<RecommendationLatestModel>;
  readonly getRecommendationRun: (runId: string) => Promise<RecommendationRunModel>;
  readonly getLicensingStatus: () => Promise<LicensingStatusModel>;
  readonly getRecommendationRuns: () => Promise<PageResult<RecommendationRunModel>>;
  readonly getOwnerBetaRecommendationRuns: () => Promise<OwnerBetaRunPageModel>;
  readonly getOwnerBetaRecommendationRun: (runId: string) => Promise<OwnerBetaRunModel>;
  readonly getOwnerBetaSupportedAsOf: () => Promise<OwnerBetaSupportedAsOfModel>;
  readonly getOwnerBetaEquitySignalsLatest: () => Promise<OwnerBetaEquitySignalsLatestModel>;
  readonly screenOwnerBetaEquitySignals: (
    body: OwnerBetaEquitySignalsScreenBody,
  ) => Promise<OwnerBetaEquitySignalsScreenModel>;
  readonly getOwnerBetaEquitySignalDetail: (
    instrumentId: string,
  ) => Promise<OwnerBetaEquitySignalsDetailModel>;
  readonly getStrategies: () => Promise<PageResult<StrategyCatalogItem>>;
  readonly getPaperAccounts: () => Promise<PageResult<PaperAccountModel>>;
  readonly getPaperAccount: (accountId: string) => Promise<PaperAccountModel>;
  readonly getNotifications: () => Promise<PageResult<NotificationModel>>;
  readonly getStrategyConfigs: () => Promise<PageResult<StrategyConfigModel>>;
  readonly getLiveConnections: () => Promise<PageResult<LiveConnectionModel>>;
  readonly getCandidateFeed: (asOf?: string, universe?: UniverseKey) => Promise<CandidateFeed>;
  readonly getSavedScreens: () => Promise<{ readonly items: readonly SavedScreen[] }>;
  readonly getStockAnalysis: (
    instrumentId: string,
    asOf?: string,
    universe?: UniverseKey,
  ) => Promise<StockAnalysisResponse>;
  readonly queryScreener: (query: ScreenerQuery) => Promise<ScreenerResult>;
  readonly getPaperPerformance: (accountId: string) => Promise<PaperPerformanceModel>;
  readonly getPaperLineage: (accountId: string) => Promise<PaperLineageModel>;
  readonly getPaperParity: (accountId: string, asOf: string) => Promise<PaperParityModel>;
  readonly getPaperPositions: (
    accountId: string,
  ) => Promise<PageResult<z.infer<typeof paperPositionSchema>>>;
  readonly getPaperOrders: (
    accountId: string,
  ) => Promise<PageResult<z.infer<typeof paperOrderSchema>>>;
  readonly getRebalancePreview: (
    accountId: string,
    previewId: string,
  ) => Promise<RebalancePreviewModel>;
};

const paperAccountPageSchema = pageSchema(paperAccountSchema);
const paperPositionPageSchema = pageSchema(paperPositionSchema);
const paperOrderPageSchema = pageSchema(paperOrderSchema);
const notificationPageSchema = pageSchema(notificationSchema);
const strategyConfigPageSchema = pageSchema(strategyConfigSchema);
const liveConnectionPageSchema = pageSchema(liveConnectionSchema);

const strategyPageSchema = pageSchema(strategySchema);
const recommendationPageSchema = pageSchema(recommendationRunSchema);

async function getParsed<Output>(
  client: KyInstance,
  path: string,
  schema: z.ZodType<Output>,
): Promise<Output> {
  const response = await client.get(path.slice(1));
  return parseApiResponse(response, schema);
}

export function createProductApiClient(options: ServerApiClientOptions): ProductApiClient {
  const client = createServerTransport(options);
  return {
    getBacktestReport: async (run) => {
      const encodedRunId = encodeURIComponent(run.id);
      const [metrics, equity, trades] = await Promise.all([
        getParsed(client, `/api/v1/backtests/${encodedRunId}/metrics`, backtestMetricsSchema),
        getParsed(client, `/api/v1/backtests/${encodedRunId}/equity`, backtestEquitySchema),
        getParsed(client, `/api/v1/backtests/${encodedRunId}/trades`, backtestTradesSchema),
      ]);
      return { equity, metrics, provenance: backtestProvenance(run), run, trades };
    },
    getBacktestRuns: () => getParsed(client, "/api/v1/backtests", backtestPageSchema),
    getCandidateFeed: (asOf, universe = DEFAULT_UNIVERSE) => {
      const path =
        asOf === undefined
          ? "/api/v1/candidates/feed/latest"
          : `/api/v1/candidates/feed/${encodeURIComponent(asOf)}`;
      const params = new URLSearchParams({ universe });
      return getParsed(client, `${path}?${params.toString()}`, candidateFeedSchema);
    },
    getLatestRecommendation: () =>
      getParsed(client, "/api/v1/recommendations/latest", latestRecommendationSchema),
    getLicensingStatus: () => getParsed(client, "/api/v1/licensing-status", licensingStatusSchema),
    getRecommendationRuns: () =>
      getParsed(client, "/api/v1/recommendations/runs", recommendationPageSchema),
    getOwnerBetaRecommendationRuns: () =>
      getParsed(client, OWNER_BETA_PRICE_ONLY_RUNS_PATH, ownerBetaRunPageSchema),
    getOwnerBetaRecommendationRun: (runId) =>
      getParsed(client, ownerBetaRunPath(runId), ownerBetaRunSchema),
    getOwnerBetaSupportedAsOf: () =>
      getParsed(client, OWNER_BETA_PRICE_ONLY_SUPPORTED_AS_OF_PATH, ownerBetaSupportedAsOfSchema),
    getOwnerBetaEquitySignalsLatest: () =>
      getParsed(client, OWNER_BETA_EQUITY_SIGNALS_LATEST_PATH, ownerBetaEquitySignalsLatestSchema),
    screenOwnerBetaEquitySignals: async (body) => {
      const requestBody = ownerBetaEquitySignalsScreenBodySchema.parse(body);
      const response = await client.post(OWNER_BETA_EQUITY_SIGNALS_SCREEN_PATH.slice(1), {
        json: requestBody,
      });
      return parseApiResponse(response, ownerBetaEquitySignalsScreenSchema);
    },
    getOwnerBetaEquitySignalDetail: (instrumentId) =>
      getParsed(
        client,
        ownerBetaEquitySignalsDetailPath(instrumentId),
        ownerBetaEquitySignalsDetailSchema,
      ),
    getRecommendationRun: (runId) =>
      getParsed(
        client,
        `/api/v1/recommendations/runs/${encodeURIComponent(runId)}`,
        recommendationRunSchema,
      ),
    getStrategies: () => getParsed(client, "/api/v1/strategies", strategyPageSchema),
    getPaperAccounts: () => getParsed(client, "/api/v1/paper/accounts", paperAccountPageSchema),
    getNotifications: () => getParsed(client, "/api/v1/notifications", notificationPageSchema),
    getStrategyConfigs: () =>
      getParsed(client, "/api/v1/strategy-configs", strategyConfigPageSchema),
    getLiveConnections: () =>
      getParsed(client, "/api/v1/admin/live/connections", liveConnectionPageSchema),
    getSavedScreens: () => getParsed(client, "/api/v1/screener/screens", savedScreenListSchema),
    getStockAnalysis: (instrumentId, asOf, universe = DEFAULT_UNIVERSE) => {
      const params = new URLSearchParams({ universe });
      if (asOf !== undefined) params.set("date", asOf);
      return getParsed(
        client,
        `/api/v1/stocks/${encodeURIComponent(instrumentId)}/analysis?${params.toString()}`,
        stockAnalysisResponseSchema,
      );
    },
    getPaperAccount: (accountId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}`,
        paperAccountSchema,
      ),
    getPaperPerformance: (accountId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/performance`,
        paperPerformanceSchema,
      ),
    getPaperLineage: (accountId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/lineage`,
        paperLineageSchema,
      ),
    getPaperParity: (accountId, asOf) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/parity?as_of=${encodeURIComponent(asOf)}`,
        paperParitySchema,
      ),
    getPaperPositions: (accountId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/positions`,
        paperPositionPageSchema,
      ),
    getPaperOrders: (accountId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/orders`,
        paperOrderPageSchema,
      ),
    getRebalancePreview: (accountId, previewId) =>
      getParsed(
        client,
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/recommendation-previews/${encodeURIComponent(previewId)}`,
        rebalancePreviewSchema,
      ),
    queryScreener: async (query) => {
      const response = await client.post("api/v1/screener/query", { json: query });
      return parseApiResponse(response, screenerResultSchema);
    },
  };
}
