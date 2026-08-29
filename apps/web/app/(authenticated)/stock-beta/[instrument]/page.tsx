import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { StockBetaDetail } from "@/components/stock-beta/stock-beta-detail";
import { StockBetaPolicyNotice } from "@/components/stock-beta/stock-beta-workspace";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { getLocale } from "@/lib/i18n/server";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock signal beta detail",
};

export type StockBetaDetailPageProps = {
  readonly params: Promise<{ readonly instrument: string }>;
};

function frame(t: StockBetaDictionary, instrument: string, children: React.ReactNode) {
  return (
    <RoutePage description={t.detailDescription} title={t.detailTitle(instrument)}>
      {children}
    </RoutePage>
  );
}

function detailErrorPage(
  t: StockBetaDictionary,
  instrument: string,
  kind: "blocked" | "empty" | "error",
  title: string,
  message: string,
) {
  return frame(
    t,
    instrument,
    <>
      <StockBetaPolicyNotice t={t} />
      <StatePanel kind={kind} message={message} title={title} />
    </>,
  );
}

async function renderStockBetaDetailProduct(instrument: string, t: StockBetaDictionary) {
  try {
    const api = await getProductApi();
    const detail = await api.getOwnerBetaEquitySignalDetail(instrument);
    return frame(t, instrument, <StockBetaDetail detail={detail} t={t} />);
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirect("/login");
    }
    if (error instanceof ApiProblem) {
      if (error.code === "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE") {
        return detailErrorPage(
          t,
          instrument,
          "blocked",
          t.signalUnavailableTitle,
          t.signalUnavailableMessage,
        );
      }
      if (error.code === "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED") {
        return detailErrorPage(t, instrument, "error", t.integrityTitle, t.integrityMessage);
      }
      if (error.code === "RESOURCE_NOT_FOUND") {
        return detailErrorPage(
          t,
          instrument,
          "empty",
          t.instrumentNotFoundTitle,
          t.instrumentNotFoundMessage,
        );
      }
    }
    return detailErrorPage(
      t,
      instrument,
      "error",
      t.genericUnavailableTitle,
      t.genericUnavailableMessage,
    );
  }
}

export async function StockBetaDetailProductPage(instrument: string) {
  const locale = await getLocale();
  return renderStockBetaDetailProduct(instrument, stockBetaDictionary[locale]);
}

export default async function StockBetaDetailPage({ params }: StockBetaDetailPageProps) {
  const { instrument } = await params;
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () => renderStockBetaDetailProduct(instrument, stockBetaDictionary[locale]),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
