import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { StatePanel } from "@/components/states/state-panel";
import { StockBetaPolicyNotice } from "@/components/stock-beta/dashboard/widgets/policy-boundary-widget";
import { StockBetaWorkspace } from "@/components/stock-beta/stock-beta-workspace";
import { StockBetaTerminalPage } from "@/components/stock-beta/terminal";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import { getLocale } from "@/lib/i18n/server";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = { title: "Stock signal beta" };

function errorPage(
  t: StockBetaDictionary,
  kind: "blocked" | "error",
  title: string,
  message: string,
) {
  return (
    <StockBetaTerminalPage context={<span>{t.terminalContextLabel}</span>} title={t.pageTitle}>
      <StockBetaPolicyNotice t={t} />
      <StatePanel kind={kind} message={message} title={title} />
    </StockBetaTerminalPage>
  );
}

async function renderStockBetaProduct(t: StockBetaDictionary, locale: Locale) {
  try {
    const api = await getProductApi();
    const memberships = await api.getOwnerEquityV2Memberships();
    let signals = null;
    let initialSignalUnavailable = false;
    try {
      signals = await api.getOwnerEquityV2LatestSignals();
    } catch (error) {
      if (isLoginRequiredError(error)) redirect("/login");
      if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE")
        initialSignalUnavailable = true;
      else throw error;
    }
    return (
      <StockBetaWorkspace
        initialMemberships={memberships}
        initialSignalUnavailable={initialSignalUnavailable}
        initialSignals={signals}
        locale={locale}
      />
    );
  } catch (error) {
    if (isLoginRequiredError(error)) redirect("/login");
    if (error instanceof ApiProblem) {
      if (error.code === "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE")
        return errorPage(t, "blocked", t.signalUnavailableTitle, t.signalUnavailableMessage);
      if (error.code === "OWNER_EQUITY_INTEGRITY_FAILED")
        return errorPage(t, "error", t.integrityTitle, t.integrityMessage);
      return errorPage(t, "error", t.genericUnavailableTitle, t.requestFailure(error.code));
    }
    return errorPage(t, "error", t.genericUnavailableTitle, t.genericUnavailableMessage);
  }
}

export async function StockBetaProductPage() {
  const locale = await getLocale();
  return renderStockBetaProduct(stockBetaDictionary[locale], locale);
}

export default async function StockBetaPage() {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () => renderStockBetaProduct(stockBetaDictionary[locale], locale),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
