import type { KyInstance } from "ky";
import type { z } from "zod";
import {
  backtestEquitySchema,
  backtestMetricsSchema,
  backtestPageSchema,
  backtestProvenance,
  backtestTradesSchema,
  type BacktestReportModel,
  type BacktestRunModel,
} from "@/lib/products/backtest-contracts";
import {
  latestRecommendationSchema,
  licensingStatusSchema,
  pageSchema,
  recommendationRunSchema,
  strategySchema,
  type LicensingStatusModel,
  type PageResult,
  type RecommendationRunModel,
  type StrategyCatalogItem,
} from "@/lib/products/contracts";
import { parseApiResponse } from "./response";
import { createServerTransport, type ServerApiClientOptions } from "./server-client";

export type ProductApiClient = {
  readonly getBacktestReport: (run: BacktestRunModel) => Promise<BacktestReportModel>;
  readonly getBacktestRuns: () => Promise<z.infer<typeof backtestPageSchema>>;
  readonly getLatestRecommendation: () => Promise<RecommendationRunModel>;
  readonly getLicensingStatus: () => Promise<LicensingStatusModel>;
  readonly getRecommendationRuns: () => Promise<PageResult<RecommendationRunModel>>;
  readonly getStrategies: () => Promise<PageResult<StrategyCatalogItem>>;
};

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
        getParsed(
          client,
          `/api/v1/backtests/${encodedRunId}/metrics`,
          backtestMetricsSchema,
        ),
        getParsed(
          client,
          `/api/v1/backtests/${encodedRunId}/equity`,
          backtestEquitySchema,
        ),
        getParsed(
          client,
          `/api/v1/backtests/${encodedRunId}/trades`,
          backtestTradesSchema,
        ),
      ]);
      return { equity, metrics, provenance: backtestProvenance(run), run, trades };
    },
    getBacktestRuns: () => getParsed(client, "/api/v1/backtests", backtestPageSchema),
    getLatestRecommendation: async () => {
      const envelope = await getParsed(
        client,
        "/api/v1/recommendations/latest",
        latestRecommendationSchema,
      );
      return envelope.run;
    },
    getLicensingStatus: () =>
      getParsed(client, "/api/v1/licensing-status", licensingStatusSchema),
    getRecommendationRuns: () =>
      getParsed(client, "/api/v1/recommendations/runs", recommendationPageSchema),
    getStrategies: () => getParsed(client, "/api/v1/strategies", strategyPageSchema),
  };
}
