import type { Metadata } from "next";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { RoutePage } from "@/components/pages/route-page";
import { RecommendationHistory } from "@/components/recommendations/recommendation-history";
import { RecommendationReport } from "@/components/recommendations/recommendation-report";
import { RecommendationRunForm } from "@/components/recommendations/recommendation-run-form";
import { RecommendationRunStatus } from "@/components/recommendations/recommendation-run-status";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import {
  type RecommendationsDictionary,
  recommendationsDictionary,
} from "@/lib/i18n/dictionaries/recommendations";
import { getLocale } from "@/lib/i18n/server";
import { type LicensingStatusModel, permitsUse } from "@/lib/products/contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Recommendations",
};

type RecommendationsPageProps = {
  readonly searchParams?: Promise<{ readonly run_id?: string }>;
};

function recommendationLicenseState(status: LicensingStatusModel): string {
  return (
    status.datasets.find((dataset) => dataset.use_kind === "recommendation")?.state ??
    "NOT REPORTED"
  );
}

function blockedPage(message: string, t: RecommendationsDictionary) {
  return (
    <RoutePage description={t.routeDescription} title={t.routeTitle}>
      <StatePanel kind="blocked" message={message} title={t.blockedDataTitle} />
    </RoutePage>
  );
}

function configLabel(config: {
  readonly id: string;
  readonly strategy_id: string;
  readonly strategy_version: string;
}): string {
  return `${config.strategy_id}@${config.strategy_version} (${config.id})`;
}

function noRun(error: unknown): boolean {
  return error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND";
}

export default async function RecommendationsPage(props: RecommendationsPageProps = {}) {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    renderProduct: () => RecommendationsProductPage(props),
    title: recommendationsDictionary[locale].routeTitle,
  });
}

export async function RecommendationsProductPage({ searchParams }: RecommendationsPageProps = {}) {
  const locale = await getLocale();
  const t = recommendationsDictionary[locale];
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "recommendation")) {
      return blockedPage(t.entitlementInactiveMessage, t);
    }

    const [configs, latest, history] = await Promise.all([
      api.getStrategyConfigs(),
      api.getLatestRecommendation().catch((error: unknown) => {
        if (noRun(error)) {
          return null;
        }
        throw error;
      }),
      api.getRecommendationRuns(),
    ]);
    const requestedRunId = (await searchParams)?.run_id;
    const selected =
      requestedRunId === undefined || requestedRunId === ""
        ? null
        : await api.getRecommendationRun(requestedRunId);
    const activeConfigs = configs.items.filter((config) => config.is_active);
    const latestSuccessful = latest?.run ?? null;
    const activeRun = selected ?? latest?.latest_run ?? null;
    const reportRun = selected?.status === "SUCCEEDED" ? selected : latestSuccessful;

    return (
      <RoutePage description={t.routeDescription} title={t.routeTitle}>
        {activeConfigs.length === 0 ? (
          <StatePanel kind="empty" message={t.noConfigMessage} title={t.noConfigTitle} />
        ) : (
          <section aria-labelledby="recommendation-run-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">{t.newRunEyebrow}</p>
                <h2 id="recommendation-run-title">{t.generateRecommendation}</h2>
              </div>
              <p>{t.newRunHelp}</p>
            </div>
            <RecommendationRunForm
              configs={activeConfigs.map((config) => ({
                id: config.id,
                label: configLabel(config),
              }))}
              defaultAsOf={licensing.as_of}
            />
          </section>
        )}
        {activeRun === null || activeRun.status === "SUCCEEDED" ? null : (
          <RecommendationRunStatus run={activeRun} />
        )}
        {reportRun === null ? (
          activeRun === null && activeConfigs.length > 0 ? (
            <StatePanel
              kind="empty"
              message={t.noRecommendationMessage}
              title={t.noRecommendationTitle}
            />
          ) : null
        ) : (
          <RecommendationReport
            licenseState={recommendationLicenseState(licensing)}
            run={reportRun}
            t={t}
          />
        )}
        <RecommendationHistory runs={history.items} t={t} />
      </RoutePage>
    );
  } catch (error) {
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return blockedPage(t.entitlementBlockedMessage, t);
    }
    return (
      <RoutePage description={t.routeDescription} title={t.routeTitle}>
        <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />
      </RoutePage>
    );
  }
}
