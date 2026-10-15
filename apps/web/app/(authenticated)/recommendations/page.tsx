import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { RoutePage } from "@/components/pages/route-page";
import { OwnerBetaDailyPanel } from "@/components/recommendations/owner-beta-daily-panel";
import { OwnerBetaHistory } from "@/components/recommendations/owner-beta-history";
import { OwnerBetaReport } from "@/components/recommendations/owner-beta-report";
import { OwnerBetaRunForm } from "@/components/recommendations/owner-beta-run-form";
import { OwnerBetaRunStatus } from "@/components/recommendations/owner-beta-run-status";
import { RecommendationHistory } from "@/components/recommendations/recommendation-history";
import { RecommendationReport } from "@/components/recommendations/recommendation-report";
import { RecommendationRunForm } from "@/components/recommendations/recommendation-run-form";
import { RecommendationRunStatus } from "@/components/recommendations/recommendation-run-status";
import { StatePanel } from "@/components/states/state-panel";
import { StrategyDailyControl } from "@/components/strategies/strategy-daily-control";
import type { ApiSession } from "@/lib/api/contracts";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import {
  type RecommendationsDictionary,
  recommendationsDictionary,
} from "@/lib/i18n/dictionaries/recommendations";
import { getLocale } from "@/lib/i18n/server";
import { type LicensingStatusModel, permitsUse } from "@/lib/products/contracts";
import { ownerBetaDailyConfigPath } from "@/lib/products/owner-beta-daily-contracts";
import { retainOwnerBetaHistory } from "@/lib/products/owner-beta-history";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Recommendations",
};

type RecommendationsPageProps = {
  readonly searchParams?: Promise<{
    readonly daily_config_id?: string | string[];
    readonly history_cursor?: string | string[];
    readonly run_id?: string | string[];
  }>;
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

async function optionalOwnerBetaRead<T>(read: () => Promise<T>): Promise<T | null> {
  try {
    return await read();
  } catch (error) {
    if (
      isLoginRequiredError(error) ||
      (error instanceof ApiProblem &&
        ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code))
    ) {
      throw error;
    }
    return null;
  }
}

async function OwnerBetaRecommendationsProductPage(
  { searchParams }: RecommendationsPageProps,
  t: RecommendationsDictionary,
) {
  const query = await searchParams;
  const requestedRunId = query?.run_id;
  const requestedConfigId = query?.daily_config_id;
  const historyCursor = query?.history_cursor;
  if (
    Array.isArray(requestedRunId) ||
    Array.isArray(requestedConfigId) ||
    Array.isArray(historyCursor)
  ) {
    return ownerBetaUnavailablePage(t);
  }
  const api = await getProductApi();
  try {
    if (requestedConfigId !== undefined) {
      ownerBetaDailyConfigPath(requestedConfigId);
    }
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "recommendation")) {
      return ownerBetaBlockedPage(t.entitlementInactiveMessage, t);
    }

    const [discovery, configs, firstHistory] = await Promise.all([
      optionalOwnerBetaRead(() => api.getOwnerBetaSupportedAsOf()),
      api.getStrategyConfigs(),
      api.getOwnerBetaRecommendationRuns({
        ...(historyCursor === undefined ? {} : { cursor: historyCursor }),
        limit: 100,
      }),
    ]);
    const requestedRun =
      requestedRunId === undefined || requestedRunId === ""
        ? null
        : await api.getOwnerBetaRecommendationRun(requestedRunId);
    if (
      requestedConfigId !== undefined &&
      requestedRun !== null &&
      requestedRun.strategy_config_id !== requestedConfigId
    ) {
      return ownerBetaUnavailablePage(t);
    }
    const preferredConfigId =
      requestedConfigId ??
      requestedRun?.strategy_config_id ??
      firstHistory.items[0]?.strategy_config_id;
    const selectedConfig =
      configs.items.find((config) => config.id === preferredConfigId) ??
      (preferredConfigId === undefined ? configs.items[0] : undefined);
    if (requestedConfigId !== undefined && selectedConfig === undefined) {
      return ownerBetaUnavailablePage(t);
    }
    const configId = selectedConfig?.id ?? preferredConfigId;
    const history = await retainOwnerBetaHistory(
      api.getOwnerBetaRecommendationRuns,
      firstHistory,
      configId,
      historyCursor,
    );
    const selectedRunId = history.runs[0]?.id;
    const selected =
      requestedRun ??
      (selectedRunId === undefined ? null : await api.getOwnerBetaRecommendationRun(selectedRunId));
    const successful = history.runs.find((run) => run.status === "SUCCEEDED");
    const report =
      selected?.status === "SUCCEEDED"
        ? selected
        : successful === undefined
          ? null
          : await api.getOwnerBetaRecommendationRun(successful.id);
    if (
      (selected !== null && selected.strategy_config_id !== configId) ||
      (report !== null && (report.strategy_config_id !== configId || report.status !== "SUCCEEDED"))
    ) {
      return ownerBetaUnavailablePage(t);
    }
    const dailyState =
      selectedConfig?.owner_beta_daily_enabled === undefined
        ? null
        : await optionalOwnerBetaRead(() => api.getOwnerBetaDailyState(selectedConfig.id));
    const activeConfigs = configs.items.filter((config) => config.is_active);
    const inFlight = selected === null || selected.status === "SUCCEEDED" ? null : selected;
    const olderQuery = new URLSearchParams();
    if (configId !== undefined) {
      olderQuery.set("daily_config_id", configId);
    }
    if (history.nextCursor !== null) {
      olderQuery.set("history_cursor", history.nextCursor);
    }

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
        {configs.items.length === 0 ? null : (
          <section aria-labelledby="owner-beta-config-title" className="workflow-panel">
            <h2 id="owner-beta-config-title">{t.dailyConfigHeading}</h2>
            <form action="/recommendations" className="config-form" method="GET">
              <label className="form-field" htmlFor="daily-config-id">
                <span>{t.selectStrategyConfig}</span>
                <select defaultValue={configId} id="daily-config-id" name="daily_config_id">
                  {configs.items.map((config) => (
                    <option key={config.id} value={config.id}>
                      {configLabel(config)}
                    </option>
                  ))}
                </select>
              </label>
              <button type="submit">{t.dailySelectConfig}</button>
            </form>
            {selectedConfig === undefined ? null : (
              <StrategyDailyControl
                configId={selectedConfig.id}
                enabled={selectedConfig.owner_beta_daily_enabled}
                key={selectedConfig.id}
              />
            )}
          </section>
        )}
        {selectedConfig?.owner_beta_daily_enabled === undefined ? null : (
          <OwnerBetaDailyPanel
            configId={selectedConfig.id}
            initialState={dailyState}
            key={selectedConfig.id}
          />
        )}
        {discovery === null || discovery.supported_as_of.length === 0 ? (
          <StatePanel
            kind="error"
            message={t.dailyManualUnavailableMessage}
            title={t.dailyManualUnavailableTitle}
          />
        ) : activeConfigs.length === 0 ? (
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
              defaultAsOf={discovery.default_as_of}
              supportedAsOf={discovery.supported_as_of}
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
        {report !== null && report.id !== selected?.id ? (
          <aside className="warning-strip" role="status">
            <strong>{t.dailyPreviousResult}</strong>
            <p>{t.dailyPreviousResultHelp}</p>
          </aside>
        ) : null}
        {report === null ? null : <OwnerBetaReport run={report} t={t} />}
        {selected === null && history.runs.length === 0 && !history.hasMore ? (
          <StatePanel
            kind="empty"
            message={t.ownerBetaNoRunsMessage}
            title={t.noRecommendationTitle}
          />
        ) : null}
        {history.previousResultSearchIncomplete ? (
          <p role="status">{t.dailyHistoryIncomplete}</p>
        ) : null}
        <OwnerBetaHistory runs={history.runs} t={t} />
        {history.hasMore ? (
          <Link href={`/recommendations?${olderQuery.toString()}`}>{t.dailyOlderRuns}</Link>
        ) : null}
      </RoutePage>
    );
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return ownerBetaBlockedPage(t.entitlementBlockedMessage, t);
    }
    return ownerBetaUnavailablePage(t);
  }
}

export default async function RecommendationsPage(props: RecommendationsPageProps) {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "recommendations",
    renderProduct: (session) => RecommendationsProductPage(props, session),
    title: recommendationsDictionary[locale].routeTitle,
  });
}

async function RecommendationsProductPage(
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
  const api = await getProductApi();
  try {
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
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
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
