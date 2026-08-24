import type { Metadata } from "next";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { RoutePage } from "@/components/pages/route-page";
import { OwnerBetaHistory } from "@/components/recommendations/owner-beta-history";
import { OwnerBetaReport } from "@/components/recommendations/owner-beta-report";
import { OwnerBetaRunForm } from "@/components/recommendations/owner-beta-run-form";
import { OwnerBetaRunStatus } from "@/components/recommendations/owner-beta-run-status";
import { RecommendationHistory } from "@/components/recommendations/recommendation-history";
import { RecommendationReport } from "@/components/recommendations/recommendation-report";
import { RecommendationRunForm } from "@/components/recommendations/recommendation-run-form";
import { RecommendationRunStatus } from "@/components/recommendations/recommendation-run-status";
import { StatePanel } from "@/components/states/state-panel";
import type { ApiSession } from "@/lib/api/contracts";
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
  readonly searchParams?: Promise<{ readonly run_id?: string | string[] }>;
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

function ownerBetaBlockedPage(message: string, t: RecommendationsDictionary) {
  return (
    <RoutePage description={t.ownerBetaRouteDescription} title={t.ownerBetaRouteTitle}>
      <aside aria-label={t.warningsAriaLabel} className="warning-strip" role="status">
        <strong>{t.warningsLabel}</strong>
        <p>{t.ownerBetaInputWarning}</p>
        <p>
          {t.ownerBetaAudienceValue} · {t.ownerBetaCapabilityValue} ·{" "}
          {t.ownerBetaVendorSnapshotValue} · {t.ownerBetaStrictPitValue}
        </p>
      </aside>
      <StatePanel kind="blocked" message={message} title={t.blockedDataTitle} />
    </RoutePage>
  );
}

function ownerBetaUnavailablePage(t: RecommendationsDictionary) {
  return (
    <RoutePage description={t.ownerBetaRouteDescription} title={t.ownerBetaRouteTitle}>
      <aside aria-label={t.warningsAriaLabel} className="warning-strip" role="status">
        <strong>{t.warningsLabel}</strong>
        <p>{t.ownerBetaInputWarning}</p>
        <p>
          {t.ownerBetaAudienceValue} · {t.ownerBetaCapabilityValue} ·{" "}
          {t.ownerBetaVendorSnapshotValue} · {t.ownerBetaStrictPitValue}
        </p>
      </aside>
      <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />
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

async function OwnerBetaRecommendationsProductPage(
  { searchParams }: RecommendationsPageProps,
  t: RecommendationsDictionary,
) {
  const requestedRunId = (await searchParams)?.run_id;
  if (Array.isArray(requestedRunId)) {
    return ownerBetaUnavailablePage(t);
  }
  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "recommendation")) {
      return ownerBetaBlockedPage(t.entitlementInactiveMessage, t);
    }

    const [configs, history] = await Promise.all([
      api.getStrategyConfigs(),
      api.getOwnerBetaRecommendationRuns(),
    ]);
    const selectedRunId =
      requestedRunId === undefined || requestedRunId === "" ? history.items[0]?.id : requestedRunId;
    const selected =
      selectedRunId === undefined ? null : await api.getOwnerBetaRecommendationRun(selectedRunId);
    const activeConfigs = configs.items.filter((config) => config.is_active);
    const inFlight = selected === null || selected.status === "SUCCEEDED" ? null : selected;

    return (
      <RoutePage description={t.ownerBetaRouteDescription} title={t.ownerBetaRouteTitle}>
        <aside aria-label={t.warningsAriaLabel} className="warning-strip" role="status">
          <strong>{t.warningsLabel}</strong>
          <p>{t.ownerBetaInputWarning}</p>
          <p>
            {t.ownerBetaAudienceValue} · {t.ownerBetaCapabilityValue} ·{" "}
            {t.ownerBetaVendorSnapshotValue} · {t.ownerBetaStrictPitValue}
          </p>
        </aside>
        {activeConfigs.length === 0 ? (
          <StatePanel kind="empty" message={t.noConfigMessage} title={t.noConfigTitle} />
        ) : (
          <section aria-labelledby="owner-beta-run-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">{t.newRunEyebrow}</p>
                <h2 id="owner-beta-run-title">{t.ownerBetaSubmit}</h2>
              </div>
              <p>{t.ownerBetaInputWarning}</p>
            </div>
            <OwnerBetaRunForm
              configs={activeConfigs.map((config) => ({
                id: config.id,
                label: configLabel(config),
              }))}
              defaultAsOf={licensing.as_of}
            />
          </section>
        )}
        {inFlight === null ? null : (
          <OwnerBetaRunStatus
            initialStatus={inFlight.status}
            key={inFlight.id}
            runId={inFlight.id}
          />
        )}
        {selected?.status === "SUCCEEDED" ? <OwnerBetaReport run={selected} t={t} /> : null}
        {selected === null && history.items.length === 0 ? (
          <StatePanel
            kind="empty"
            message={t.ownerBetaNoRunsMessage}
            title={t.noRecommendationTitle}
          />
        ) : null}
        <OwnerBetaHistory runs={history.items} t={t} />
      </RoutePage>
    );
  } catch (error) {
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return ownerBetaBlockedPage(t.entitlementBlockedMessage, t);
    }
    return ownerBetaUnavailablePage(t);
  }
}

export default async function RecommendationsPage(props: RecommendationsPageProps = {}) {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "recommendations",
    renderProduct: (session) => RecommendationsProductPage(props, session),
    title: recommendationsDictionary[locale].routeTitle,
  });
}

export async function RecommendationsProductPage(
  { searchParams }: RecommendationsPageProps = {},
  session?: ApiSession,
) {
  const locale = await getLocale();
  const t = recommendationsDictionary[locale];
  if (session?.role === "owner" && session.owner_beta_access_mode === "owner_only") {
    return OwnerBetaRecommendationsProductPage(
      searchParams === undefined ? {} : { searchParams },
      t,
    );
  }
  const requestedRunId = (await searchParams)?.run_id;
  if (Array.isArray(requestedRunId)) {
    return (
      <RoutePage description={t.routeDescription} title={t.routeTitle}>
        <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />
      </RoutePage>
    );
  }
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
