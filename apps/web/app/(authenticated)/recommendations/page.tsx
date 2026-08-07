import type { Metadata } from "next";
import { RoutePage } from "@/components/pages/route-page";
import { RecommendationHistory } from "@/components/recommendations/recommendation-history";
import { RecommendationReport } from "@/components/recommendations/recommendation-report";
import { RecommendationRunForm } from "@/components/recommendations/recommendation-run-form";
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

export default async function RecommendationsPage() {
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "recommendation")) {
      return blockedPage(
        "The recommendation entitlement is inactive. Creation is disabled and proprietary candidate data is not rendered.",
      );
    }
    const [latest, history] = await Promise.all([
      api.getLatestRecommendation(),
      api.getRecommendationRuns(),
    ]);
    if (latest.status === "BLOCKED" || latest.status === "FAILED") {
      return (
        <RoutePage
          description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
          title="Recommendations"
        >
          <StatePanel
            kind="error"
            message="The latest recommendation run did not produce a report. Candidate payloads remain hidden."
            title={`Recommendation run ${latest.status.toLowerCase()}`}
          />
          <RecommendationHistory runs={history.items} />
        </RoutePage>
      );
    }
    if (latest.status === "PENDING" || latest.items === undefined) {
      return (
        <RoutePage
          description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
          title="Recommendations"
        >
          <StatePanel
            kind="loading"
            message="The server is producing the recommendation. Refresh to read the completed report."
            title="Recommendation is in progress"
          />
          <RecommendationHistory runs={history.items} />
        </RoutePage>
      );
    }
    return (
      <RoutePage
        description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
        title="Recommendations"
      >
        {latest.strategy_config_id === undefined || latest.strategy_config_id === null ? null : (
          <section aria-labelledby="recommendation-run-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">New governed run</p>
                <h2 id="recommendation-run-title">Generate recommendation</h2>
              </div>
              <p>The API validates the stored strategy configuration and as-of dataset.</p>
            </div>
            <RecommendationRunForm
              defaultAsOf={licensing.as_of}
              strategyConfigId={latest.strategy_config_id}
            />
          </section>
        )}
        <RecommendationReport licenseState={recommendationLicenseState(licensing)} run={latest} />
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
    if (error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND") {
      return (
        <RoutePage
          description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
          title="Recommendations"
        >
          <StatePanel
            kind="empty"
            message="No recommendation run exists yet. Save an allowed strategy configuration before creating one."
            title="No recommendation available"
          />
        </RoutePage>
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
