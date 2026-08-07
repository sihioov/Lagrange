import type { Metadata } from "next";
import { BacktestComparison } from "@/components/backtests/backtest-comparison";
import { BacktestCreateForm } from "@/components/backtests/backtest-create-form";
import { BacktestHistory } from "@/components/backtests/backtest-history";
import { BacktestProgress } from "@/components/backtests/backtest-progress";
import { BacktestReport } from "@/components/backtests/backtest-report";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type BacktestRunModel, backtestCreationDefaults } from "@/lib/products/backtest-contracts";
import { type LicensingStatusModel, permitsUse } from "@/lib/products/contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Backtests",
};

const BLOCKED_CODES = new Set([
  "DATASET_BLOCKED",
  "DATA_ENTITLEMENT_REQUIRED",
  "DATA_STALE",
  "FORBIDDEN",
]);

function blockedPage(message: string) {
  return (
    <RoutePage
      description="Create reproducible simulations and inspect performance, cost, drawdown, and robustness evidence."
      title="Backtests"
    >
      <StatePanel kind="blocked" message={message} title="Backtest data is blocked" />
    </RoutePage>
  );
}

function backtestLicenseState(status: LicensingStatusModel): string {
  return (
    status.datasets.find((dataset) => dataset.use_kind === "backtest")?.state ?? "NOT REPORTED"
  );
}

function firstByStatus(runs: readonly BacktestRunModel[], status: BacktestRunModel["status"]) {
  return runs.find((run) => run.status === status);
}

export default async function BacktestsPage() {
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "backtest")) {
      return blockedPage(
        "The backtest entitlement is inactive. Creation is disabled and proprietary results are not rendered.",
      );
    }
    const runs = await api.getBacktestRuns();
    const succeeded = runs.items.filter((run) => run.status === "SUCCEEDED");
    const selected = succeeded[0];
    const report = selected === undefined ? null : await api.getBacktestReport(selected);
    const defaultsSource = selected ?? runs.items[0];
    const defaults = defaultsSource === undefined ? null : backtestCreationDefaults(defaultsSource);
    const running = firstByStatus(runs.items, "RUNNING");
    const failed = firstByStatus(runs.items, "FAILED");
    const canceled = firstByStatus(runs.items, "CANCELED");

    return (
      <RoutePage
        description="Create reproducible simulations and inspect performance, cost, drawdown, and robustness evidence."
        title="Backtests"
      >
        {defaults === null ? (
          <StatePanel
            kind="blocked"
            message="The server did not provide the versioned strategy and dataset defaults required for creation."
            title="Backtest creation is unavailable"
          />
        ) : (
          <section aria-labelledby="create-backtest-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Version-pinned simulation</p>
                <h2 id="create-backtest-title">Create backtest</h2>
              </div>
              <p>Only the server queues and calculates backtest results.</p>
            </div>
            <BacktestCreateForm {...defaults} />
          </section>
        )}
        {failed === undefined ? null : (
          <StatePanel
            kind="error"
            message="The worker did not produce a verified result. Review the run status before retrying."
            title="Backtest failed"
          />
        )}
        {canceled === undefined ? null : (
          <p className="blocked-inline">Canceled and failed runs do not expose result payloads.</p>
        )}
        {running === undefined ? null : <BacktestProgress run={running} />}
        {report === null ? null : (
          <BacktestReport licenseState={backtestLicenseState(licensing)} report={report} />
        )}
        {succeeded.length < 2 ? null : <BacktestComparison runs={succeeded} />}
        {runs.items.length === 0 ? (
          <StatePanel
            kind="empty"
            message="Create a version-pinned backtest to populate this history."
            title="No backtests available"
          />
        ) : (
          <BacktestHistory runs={runs.items} />
        )}
      </RoutePage>
    );
  } catch (error) {
    if (error instanceof ApiProblem && BLOCKED_CODES.has(error.code)) {
      return blockedPage(
        "The entitlement or dataset is blocked. Creation is disabled and proprietary results are not rendered.",
      );
    }
    return (
      <RoutePage
        description="Create reproducible simulations and inspect performance, cost, drawdown, and robustness evidence."
        title="Backtests"
      >
        <StatePanel
          kind="error"
          message="Backtest data could not be loaded. Retry after checking the service status."
          title="Backtests unavailable"
        />
      </RoutePage>
    );
  }
}
