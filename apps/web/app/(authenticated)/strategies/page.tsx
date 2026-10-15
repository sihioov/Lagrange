import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { StrategyCatalog } from "@/components/strategies/strategy-catalog";
import { StrategyDailyControl } from "@/components/strategies/strategy-daily-control";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { getServerSession } from "@/lib/api/server-session";
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
  const api = await getProductApi();
  const session = await getServerSession();
  try {
    const [catalog, licensing] = await Promise.all([
      api.getStrategies(),
      api.getLicensingStatus(),
    ]);
    const canDaily =
      session.role === "owner" &&
      session.owner_beta_access_mode === "owner_only" &&
      permitsUse(licensing, "recommendation");
    const configs = canDaily ? await api.getStrategyConfigs() : null;
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
        <StrategyCatalog
          canConfigure={canConfigure}
          canDaily={canDaily}
          strategies={catalog.items}
          t={t}
        />
        {configs === null ? null : (
          <section aria-labelledby="daily-configurations-title" className="product-section">
            <h2 id="daily-configurations-title">{t.dailyConfigurationsHeading}</h2>
            {configs.items.length === 0 ? <p>{t.dailyConfigurationsEmpty}</p> : null}
            <div className="strategy-grid">
              {configs.items.map((config) => (
                <article className="strategy-panel" key={config.id}>
                  <h3>
                    {config.strategy_id}@{config.strategy_version}
                  </h3>
                  <Link href={`/recommendations?daily_config_id=${encodeURIComponent(config.id)}`}>
                    {config.id}
                  </Link>
                  {config.is_active ? null : <p>{t.dailyConfigurationInactive}</p>}
                  <StrategyDailyControl
                    configId={config.id}
                    enabled={config.owner_beta_daily_enabled}
                    key={config.id}
                  />
                </article>
              ))}
            </div>
          </section>
        )}
      </RoutePage>
    );
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
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
