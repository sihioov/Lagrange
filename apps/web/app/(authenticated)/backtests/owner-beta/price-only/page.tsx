import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaSimulationStatus } from "@/components/backtests/owner-beta-simulation-status";
import { OwnerAccessRefusal } from "@/components/pages/owner-route";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { getServerSession } from "@/lib/api/server-session";
import { backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import { ownerBetaSimulationDictionary } from "@/lib/i18n/dictionaries/owner-beta-simulation";
import { getLocale } from "@/lib/i18n/server";
import { permitsUse } from "@/lib/products/contracts";
import { eligibleOwnerBetaSimulationConfigs } from "@/lib/products/owner-beta-simulation-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Owner-only price-only simulation",
};

function BoundaryLabels({ locale }: { readonly locale: "en" | "ko" }) {
  const t = ownerBetaSimulationDictionary[locale];
  return (
    <section aria-label={t.boundaryAriaLabel} className="warning-strip" role="status">
      <ul>
        <li>{t.audienceLabel}</li>
        <li>{t.capabilityLabel}</li>
        <li>{t.vendorSnapshotLabel}</li>
        <li>{t.strictPitLabel}</li>
      </ul>
      <p>{t.boundaryExplanation}</p>
      <p>{t.priceReturnExplanation}</p>
      <p>{t.costExplanation}</p>
      <p>{t.executionExplanation}</p>
      <p>{t.datePhaseExplanation}</p>
      <p>{t.evaluationWindowExplanation}</p>
      <p>{t.defaultWindowExplanation}</p>
    </section>
  );
}

function unavailablePage(locale: "en" | "ko", message: string) {
  const t = ownerBetaSimulationDictionary[locale];
  return (
    <RoutePage description={t.pageDescription} title={t.pageTitle}>
      <BoundaryLabels locale={locale} />
      <StatePanel kind="error" message={message} title={t.unavailableTitle} />
    </RoutePage>
  );
}

export default async function OwnerBetaPriceOnlySimulationPage() {
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  const t = ownerBetaSimulationDictionary[locale];
  if (session.role !== "owner") {
    return <OwnerAccessRefusal locale={locale} title={backtestsDictionary[locale].pageTitle} />;
  }
  if (session.owner_beta_access_mode !== "owner_only") {
    return (
      <RoutePage description={t.pageDescription} title={t.pageTitle}>
        <BoundaryLabels locale={locale} />
        <StatePanel kind="blocked" message={t.disabledModeMessage} title={t.disabledModeTitle} />
      </RoutePage>
    );
  }

  try {
    const api = await getProductApi();
    const licensing = await api.getLicensingStatus();
    if (!permitsUse(licensing, "backtest")) {
      return (
        <RoutePage description={t.pageDescription} title={t.pageTitle}>
          <BoundaryLabels locale={locale} />
          <StatePanel kind="blocked" message={t.entitlementMessage} title={t.entitlementTitle} />
        </RoutePage>
      );
    }
    let configs: Awaited<ReturnType<typeof api.getStrategyConfigs>>;
    try {
      configs = await api.getStrategyConfigs();
    } catch (error) {
      if (isLoginRequiredError(error)) redirect("/login");
      return (
        <RoutePage description={t.pageDescription} title={t.pageTitle}>
          <BoundaryLabels locale={locale} />
          <StatePanel
            kind="error"
            message={t.configUnavailableMessage}
            title={t.configUnavailableTitle}
          />
        </RoutePage>
      );
    }
    const eligible = eligibleOwnerBetaSimulationConfigs(configs.items).map((config) => ({
      createdAt: config.created_at,
      id: config.id,
    }));
    return (
      <RoutePage description={t.pageDescription} title={t.pageTitle}>
        <BoundaryLabels locale={locale} />
        <OwnerBetaSimulationStatus configs={eligible} />
      </RoutePage>
    );
  } catch (error) {
    if (isLoginRequiredError(error)) redirect("/login");
    if (
      error instanceof ApiProblem &&
      (error.code === "DATA_ENTITLEMENT_REQUIRED" ||
        error.code === "DATASET_BLOCKED" ||
        error.code === "DATA_STALE")
    ) {
      return (
        <RoutePage description={t.pageDescription} title={t.pageTitle}>
          <BoundaryLabels locale={locale} />
          <StatePanel kind="blocked" message={t.entitlementMessage} title={t.entitlementTitle} />
        </RoutePage>
      );
    }
    return unavailablePage(locale, t.genericErrorMessage);
  }
}
