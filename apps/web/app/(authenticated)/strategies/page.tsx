import type { Metadata } from "next";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { StrategyCatalog } from "@/components/strategies/strategy-catalog";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { strategiesDictionary } from "@/lib/i18n/dictionaries/strategies";
import { getLocale } from "@/lib/i18n/server";
import { permitsUse } from "@/lib/products/contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Strategies",
};

const BLOCKED_CODES = new Set(["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"]);

export default async function StrategiesPage() {
  const locale = await getLocale();
  const t = strategiesDictionary[locale];
  try {
    const api = await getProductApi();
    const [catalog, licensing] = await Promise.all([api.getStrategies(), api.getLicensingStatus()]);
    if (catalog.items.length === 0) {
      return (
        <RoutePage description={t.routeDescription} title={t.routeTitle}>
          <StatePanel kind="empty" message={t.catalogEmptyMessage} title={t.catalogEmptyTitle} />
        </RoutePage>
      );
    }
    const canConfigure =
      permitsUse(licensing, "recommendation") || permitsUse(licensing, "backtest");
    return (
      <RoutePage description={t.routeDescription} title={t.routeTitle}>
        <StrategyCatalog canConfigure={canConfigure} strategies={catalog.items} t={t} />
      </RoutePage>
    );
  } catch (error) {
    const blocked = error instanceof ApiProblem && BLOCKED_CODES.has(error.code);
    return (
      <RoutePage description={t.routeDescription} title={t.routeTitle}>
        <StatePanel
          kind={blocked ? "blocked" : "error"}
          message={blocked ? t.blockedCatalogMessage : t.unavailableCatalogMessage}
          title={blocked ? t.blockedCatalogTitle : t.unavailableCatalogTitle}
        />
      </RoutePage>
    );
  }
}
