import type { Metadata } from "next";
import { redirect } from "next/navigation";
import type { ReactNode } from "react";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { StatePanel } from "@/components/states/state-panel";
import { stockBetaDetailBackHref } from "@/components/stock-beta/detail/filter-context";
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
import type { OwnerBetaEquitySignalsSearchParams } from "@/lib/products/equity-signals-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock signal beta detail",
};

export type StockBetaDetailPageProps = {
  readonly params: Promise<{ readonly instrument: string }>;
  readonly searchParams?: Promise<OwnerBetaEquitySignalsSearchParams>;
};

function frame(t: StockBetaDictionary, instrument: string, backHref: string, children: ReactNode) {
  return (
    <StockBetaTerminalPage
      context={<StockBetaDetailBackLink backHref={backHref} t={t} />}
      title={t.detailTitle(instrument)}
    >
      {children}
    </StockBetaTerminalPage>
  );
}

function detailErrorPage(
  t: StockBetaDictionary,
  instrument: string,
  kind: "blocked" | "empty" | "error",
  title: string,
  message: string,
  backHref: string,
) {
  return frame(
    t,
    instrument,
    backHref,
    <>
      <StockBetaDetailPolicyNotice t={t} />
      <StatePanel kind={kind} message={message} title={title} />
    </>,
  );
}

async function detailBackLink(
  searchParams: Promise<OwnerBetaEquitySignalsSearchParams> | undefined,
): Promise<string> {
  try {
    return stockBetaDetailBackHref(searchParams === undefined ? undefined : await searchParams);
  } catch {
    return "/stock-beta";
  }
}

async function renderStockBetaDetailProduct(
  instrument: string,
  t: StockBetaDictionary,
  locale: Locale,
  searchParams?: Promise<OwnerBetaEquitySignalsSearchParams>,
) {
  const backHref = await detailBackLink(searchParams);

  try {
    const api = await getProductApi();
    const detail = await api.getOwnerBetaEquitySignalDetail(instrument);
    return <StockBetaDetail backHref={backHref} detail={detail} locale={locale} t={t} />;
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
          backHref,
        );
      }
      if (error.code === "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED") {
        return detailErrorPage(
          t,
          instrument,
          "error",
          t.integrityTitle,
          t.integrityMessage,
          backHref,
        );
      }
      if (error.code === "RESOURCE_NOT_FOUND") {
        return detailErrorPage(
          t,
          instrument,
          "empty",
          t.instrumentNotFoundTitle,
          t.instrumentNotFoundMessage,
          backHref,
        );
      }
    }
    return detailErrorPage(
      t,
      instrument,
      "error",
      t.genericUnavailableTitle,
      t.genericUnavailableMessage,
      backHref,
    );
  }
}

export async function StockBetaDetailProductPage(
  instrument: string,
  searchParams?: Promise<OwnerBetaEquitySignalsSearchParams>,
) {
  const locale = await getLocale();
  return renderStockBetaDetailProduct(
    instrument,
    stockBetaDictionary[locale],
    locale,
    searchParams,
  );
}

export default async function StockBetaDetailPage({
  params,
  searchParams,
}: StockBetaDetailPageProps) {
  const { instrument } = await params;
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () =>
      renderStockBetaDetailProduct(instrument, stockBetaDictionary[locale], locale, searchParams),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
