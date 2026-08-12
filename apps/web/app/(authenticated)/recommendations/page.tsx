import type { Metadata } from "next";
import { RoutePage } from "@/components/pages/route-page";
import { RecommendationHistory } from "@/components/recommendations/recommendation-history";
import { RecommendationReport } from "@/components/recommendations/recommendation-report";
import { RecommendationRunForm } from "@/components/recommendations/recommendation-run-form";
import { RecommendationRunStatus } from "@/components/recommendations/recommendation-run-status";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
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

function blockedPage(message: string) {
  return (
    <RoutePage
      description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
      title="Recommendations"
    >
      <StatePanel kind="blocked" message={message} title="Recommendation data is blocked" />
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

export default async function RecommendationsPage({ searchParams }: RecommendationsPageProps = {}) {
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "recommendation")) {
      return blockedPage(
        "The recommendation entitlement is inactive. Creation is disabled and proprietary candidate data is not rendered.",
      );
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
    const latestSuccessful = latest?.run ?? null;
    const activeRun = selected ?? latest?.latest_run ?? null;
    const reportRun = selected?.status === "SUCCEEDED" ? selected : latestSuccessful;

    return (
      <RoutePage
        description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
        title="Recommendations"
      >
        {configs.items.length === 0 ? (
          <StatePanel
            kind="empty"
            message="Save an allowed strategy configuration before creating a recommendation run."
            title="No strategy configuration is available"
          />
        ) : (
          <section aria-labelledby="recommendation-run-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">New governed run</p>
                <h2 id="recommendation-run-title">Generate recommendation</h2>
              </div>
              <p>The API validates the stored strategy configuration and as-of dataset.</p>
            </div>
            <RecommendationRunForm
              configs={configs.items.map((config) => ({
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
          activeRun === null && configs.items.length > 0 ? (
            <StatePanel
              kind="empty"
              message="Generate a recommendation to inspect its governed proposal."
              title="No recommendation available"
            />
          ) : null
        ) : (
          <RecommendationReport
            licenseState={recommendationLicenseState(licensing)}
            run={reportRun}
          />
        )}
        <RecommendationHistory runs={history.items} />
      </RoutePage>
    );
  } catch (error) {
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return blockedPage(
        "The recommendation entitlement or dataset is blocked. Creation is disabled and proprietary candidate data is not rendered.",
      );
    }
    return (
      <RoutePage
        description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
        title="Recommendations"
      >
        <StatePanel
          kind="error"
          message="Recommendation data could not be loaded. Retry after checking the service status."
          title="Recommendations unavailable"
        />
      </RoutePage>
    );
  }
}
