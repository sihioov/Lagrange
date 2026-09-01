import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import {
  StockBetaPolicyNotice,
  StockBetaWorkspace,
} from "@/components/stock-beta/stock-beta-workspace";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import { getLocale } from "@/lib/i18n/server";
import type { OwnerBetaEquitySignalsSearchParams } from "@/lib/products/equity-signals-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock signal beta",
};

export type StockBetaPageProps = {
  readonly searchParams?: Promise<OwnerBetaEquitySignalsSearchParams> | undefined;
};

function frame(t: StockBetaDictionary, children: React.ReactNode) {
  return (
    <RoutePage description={t.pageDescription} title={t.pageTitle}>
      {children}
    </RoutePage>
  );
}

function errorPage(
  t: StockBetaDictionary,
  locale: Locale,
  kind: "blocked" | "error",
  title: string,
  message: string,
) {
  return frame(
    t,
    <>
      <StockBetaPolicyNotice locale={locale} />
      <StatePanel kind={kind} message={message} title={title} />
    </>,
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
      if (isLoginRequiredError(error)) {
        redirect("/login");
      }
      if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
        initialSignalUnavailable = true;
      } else {
        throw error;
      }
    }
    return frame(
      t,
      <StockBetaWorkspace
        initialMemberships={memberships}
        initialSignalUnavailable={initialSignalUnavailable}
        initialSignals={signals}
        locale={locale}
      />,
    );
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
    if (error instanceof ApiProblem) {
      if (error.code === "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE") {
        return errorPage(
          t,
          locale,
          "blocked",
          t.signalUnavailableTitle,
          t.signalUnavailableMessage,
        );
      }
      if (error.code === "OWNER_EQUITY_INTEGRITY_FAILED") {
        return errorPage(t, locale, "error", t.integrityTitle, t.integrityMessage);
      }
      return errorPage(t, locale, "error", t.genericUnavailableTitle, t.requestFailure(error.code));
    }
    return errorPage(t, locale, "error", t.genericUnavailableTitle, t.genericUnavailableMessage);
  }
}

export async function StockBetaProductPage(_props: StockBetaPageProps = {}) {
  const locale = await getLocale();
  return renderStockBetaProduct(stockBetaDictionary[locale], locale);
}

export default async function StockBetaPage(_props: StockBetaPageProps = {}) {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () => renderStockBetaProduct(stockBetaDictionary[locale], locale),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
