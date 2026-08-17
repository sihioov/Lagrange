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
import { type BacktestsDictionary, backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import { getLocale } from "@/lib/i18n/server";
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

function blockedPage(t: BacktestsDictionary, message: string) {
  return (
    <RoutePage description={t.pageDescription} title={t.pageTitle}>
      <StatePanel kind="blocked" message={message} title={t.blockedTitle} />
    </RoutePage>
  );
}

function backtestLicenseState(t: BacktestsDictionary, status: LicensingStatusModel): string {
  return (
    status.datasets.find((dataset) => dataset.use_kind === "backtest")?.state ??
    t.licenseStateNotReported
  );
}

function firstByStatus(runs: readonly BacktestRunModel[], status: BacktestRunModel["status"]) {
  return runs.find((run) => run.status === status);
}

export default async function BacktestsPage() {
  const locale = await getLocale();
  const t = backtestsDictionary[locale];
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "backtest")) {
      return blockedPage(t, t.entitlementInactiveMessage);
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
      <RoutePage description={t.pageDescription} title={t.pageTitle}>
        {defaults === null ? (
          <StatePanel
            kind="blocked"
            message={t.creationUnavailableMessage}
            title={t.creationUnavailableTitle}
          />
        ) : (
          <section aria-labelledby="create-backtest-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">{t.createEyebrow}</p>
                <h2 id="create-backtest-title">{t.createBacktestLabel}</h2>
              </div>
              <p>{t.serverQueuesMessage}</p>
            </div>
            <BacktestCreateForm {...defaults} />
          </section>
        )}
        {failed === undefined ? null : (
          <StatePanel kind="error" message={t.failedMessage} title={t.failedTitle} />
        )}
        {canceled === undefined ? null : <p className="blocked-inline">{t.canceledRunsMessage}</p>}
        {running === undefined ? null : <BacktestProgress run={running} />}
        {report === null ? null : (
          <BacktestReport licenseState={backtestLicenseState(t, licensing)} report={report} t={t} />
        )}
        {succeeded.length < 2 ? null : <BacktestComparison runs={succeeded} />}
        {runs.items.length === 0 ? (
          <StatePanel kind="empty" message={t.emptyMessage} title={t.emptyTitle} />
        ) : (
          <BacktestHistory runs={runs.items} t={t} />
        )}
      </RoutePage>
    );
  } catch (error) {
    if (error instanceof ApiProblem && BLOCKED_CODES.has(error.code)) {
      return blockedPage(t, t.datasetBlockedMessage);
    }
    return (
      <RoutePage description={t.pageDescription} title={t.pageTitle}>
        <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />
      </RoutePage>
    );
  }
}
