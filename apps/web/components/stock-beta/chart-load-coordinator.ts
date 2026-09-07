import {
  assertOwnerEquityV2ChartMatchesExpectation,
  type OwnerEquityV2ChartExpectation,
  type OwnerEquityV2ChartModel,
} from "@/lib/products/equity-signals-contracts";

export type StockBetaChartLoadRequest = OwnerEquityV2ChartExpectation;

export type StockBetaChartFetcher = (
  request: StockBetaChartLoadRequest,
  signal: AbortSignal,
) => Promise<OwnerEquityV2ChartModel>;

export type StockBetaChartLoadHandlers = {
  readonly onFailure: (error: unknown, request: StockBetaChartLoadRequest) => void;
  readonly onSuccess: (chart: OwnerEquityV2ChartModel, request: StockBetaChartLoadRequest) => void;
};

export type StockBetaChartLoadResult = "failure" | "stale" | "success";

/**
 * Coordinates one snapshot-pinned chart request at a time.
 *
 * Abort is an optimization for cooperative fetchers. The request token is the
 * correctness boundary for fetchers that resolve after their signal aborts.
 */
export class StockBetaChartLoadCoordinator {
  private requestToken = 0;
  private activeController: AbortController | undefined;

  invalidate(): void {
    this.requestToken += 1;
    this.activeController?.abort();
    this.activeController = undefined;
  }

  dispose(): void {
    this.invalidate();
  }

  async run(
    request: StockBetaChartLoadRequest,
    fetcher: StockBetaChartFetcher,
    handlers: StockBetaChartLoadHandlers,
  ): Promise<StockBetaChartLoadResult> {
    this.activeController?.abort();
    const controller = new AbortController();
    const token = ++this.requestToken;
    this.activeController = controller;

    try {
      const chart = await fetcher(request, controller.signal);
      if (!this.isCurrent(token, controller)) return "stale";
      try {
        assertOwnerEquityV2ChartMatchesExpectation(chart, request);
      } catch (error) {
        handlers.onFailure(error, request);
        return "failure";
      }
      handlers.onSuccess(chart, request);
      return "success";
    } catch (error) {
      if (!this.isCurrent(token, controller)) return "stale";
      handlers.onFailure(error, request);
      return "failure";
    } finally {
      if (this.isCurrent(token, controller)) this.activeController = undefined;
    }
  }

  private isCurrent(token: number, controller: AbortController): boolean {
    return (
      token === this.requestToken &&
      this.activeController === controller &&
      !controller.signal.aborted
    );
  }
}
