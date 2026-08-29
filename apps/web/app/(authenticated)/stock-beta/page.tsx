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
import { getLocale } from "@/lib/i18n/server";
import {
  InvalidOwnerBetaEquitySignalsFilters,
  type OwnerBetaEquitySignalsFilters,
  type OwnerBetaEquitySignalsSearchParams,
  ownerBetaEquitySignalsFiltersSelected,
  ownerBetaEquitySignalsScreenBody,
  parseOwnerBetaEquitySignalsSearchParams,
} from "@/lib/products/equity-signals-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock signal beta",
};

export type StockBetaPageProps = {
  readonly searchParams?: Promise<OwnerBetaEquitySignalsSearchParams>;
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
  kind: "blocked" | "error",
  title: string,
  message: string,
) {
  return frame(
    t,
    <>
      <StockBetaPolicyNotice t={t} />
      <StatePanel kind={kind} message={message} title={title} />
    </>,
  );
}

async function renderStockBetaProduct(
  { searchParams }: StockBetaPageProps,
  t: StockBetaDictionary,
) {
  let filters: OwnerBetaEquitySignalsFilters;
  try {
    filters = parseOwnerBetaEquitySignalsSearchParams((await searchParams) ?? {});
  } catch (error) {
    if (error instanceof InvalidOwnerBetaEquitySignalsFilters) {
      return errorPage(t, "error", t.invalidFiltersTitle, t.invalidFiltersMessage);
    }
    throw error;
  }

  try {
    const api = await getProductApi();
    const data = ownerBetaEquitySignalsFiltersSelected(filters)
      ? await api.screenOwnerBetaEquitySignals(ownerBetaEquitySignalsScreenBody(filters))
      : await api.getOwnerBetaEquitySignalsLatest();
    return frame(t, <StockBetaWorkspace data={data} filters={filters} t={t} />);
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
    if (error instanceof ApiProblem) {
      if (error.code === "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE") {
        return errorPage(t, "blocked", t.signalUnavailableTitle, t.signalUnavailableMessage);
      }
      if (error.code === "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED") {
        return errorPage(t, "error", t.integrityTitle, t.integrityMessage);
      }
    }
    return errorPage(t, "error", t.genericUnavailableTitle, t.genericUnavailableMessage);
  }
}

export async function StockBetaProductPage(props: StockBetaPageProps = {}) {
  const locale = await getLocale();
  return renderStockBetaProduct(props, stockBetaDictionary[locale]);
}

export default async function StockBetaPage(props: StockBetaPageProps = {}) {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () => renderStockBetaProduct(props, stockBetaDictionary[locale]),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
