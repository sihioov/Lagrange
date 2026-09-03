import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { StatePanel } from "@/components/states/state-panel";
import { StockBetaDetailPolicyNotice } from "@/components/stock-beta/detail/widgets/policy-boundary-widget";
import {
  StockBetaDetail,
  StockBetaDetailBackLink,
} from "@/components/stock-beta/stock-beta-detail";
import { StockBetaTerminalPage } from "@/components/stock-beta/terminal";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import { getLocale } from "@/lib/i18n/server";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;
export const metadata: Metadata = { title: "Stock signal beta detail" };

export type StockBetaDetailPageProps = {
  readonly params: Promise<{ readonly instrument: string }>;
};

function detailErrorPage(
  t: StockBetaDictionary,
  instrument: string,
  kind: "blocked" | "empty" | "error",
  title: string,
  message: string,
) {
  return (
    <StockBetaTerminalPage
      context={<StockBetaDetailBackLink backHref="/stock-beta" t={t} />}
      title={t.detailTitle(instrument)}
    >
      <StockBetaDetailPolicyNotice t={t} />
      <StatePanel kind={kind} message={message} title={title} />
    </StockBetaTerminalPage>
  );
}

async function renderStockBetaDetailProduct(
  instrument: string,
  t: StockBetaDictionary,
  locale: Locale,
) {
  try {
    const api = await getProductApi();
    const detail = await api.getOwnerEquityV2SignalDetail(instrument);
    return <StockBetaDetail detail={detail} locale={locale} t={t} />;
  } catch (error) {
    if (isLoginRequiredError(error)) redirect("/login");
    if (error instanceof ApiProblem) {
      if (error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE")
        return detailErrorPage(
          t,
          instrument,
          "blocked",
          t.signalUnavailableTitle,
          t.signalUnavailableMessage,
        );
      if (error.code === "OWNER_EQUITY_INTEGRITY_FAILED")
        return detailErrorPage(t, instrument, "error", t.integrityTitle, t.integrityMessage);
      if (error.code === "RESOURCE_NOT_FOUND" || error.code === "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND")
        return detailErrorPage(
          t,
          instrument,
          "empty",
          t.instrumentNotFoundTitle,
          t.instrumentNotFoundMessage,
        );
      return detailErrorPage(
        t,
        instrument,
        "error",
        t.genericUnavailableTitle,
        t.requestFailure(error.code),
      );
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
  return renderStockBetaDetailProduct(instrument, stockBetaDictionary[locale], locale);
}

export default async function StockBetaDetailPage({ params }: StockBetaDetailPageProps) {
  const { instrument } = await params;
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () =>
      renderStockBetaDetailProduct(instrument, stockBetaDictionary[locale], locale),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
