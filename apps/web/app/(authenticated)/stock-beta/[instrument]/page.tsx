import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { StatePanel } from "@/components/states/state-panel";
import { StockBetaDetailPolicyNotice } from "@/components/stock-beta/detail/widgets/policy-boundary-widget";
import { stockBetaIntradayTransport } from "@/components/stock-beta/quote/intraday-quotes-mode";
import { matchReadyIntradayQuoteMembership } from "@/components/stock-beta/quote/membership";
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
import type { OwnerEquityV2MembershipModel } from "@/lib/products/equity-signals-contracts";

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
  streamSessionKey: string,
) {
  try {
    const transport = stockBetaIntradayTransport();
    const intradayEnabled = transport !== "off";
    const api = await getProductApi();
    const detail = await api.getOwnerEquityV2SignalDetail(instrument);
    let intradayMembership = null;
    let streamMemberships: readonly OwnerEquityV2MembershipModel[] = [];
    if (intradayEnabled) {
      try {
        const memberships = await api.getOwnerEquityV2Memberships();
        streamMemberships = memberships.memberships;
        intradayMembership = matchReadyIntradayQuoteMembership(memberships.memberships, {
          instrument_id: detail.signal.instrument_id,
          generation: detail.signal.generation,
        });
      } catch (error) {
        if (isLoginRequiredError(error)) throw error;
      }
    }
    return (
      <StockBetaDetail
        detail={detail}
        intradayEnabled={intradayEnabled}
        marketStreamEnabled={transport === "market_ws"}
        streamSessionKey={streamSessionKey}
        streamMemberships={streamMemberships}
        intradayMembership={intradayMembership}
        locale={locale}
        t={t}
      />
    );
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

export default async function StockBetaDetailPage({ params }: StockBetaDetailPageProps) {
  const { instrument } = await params;
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: (session) =>
      renderStockBetaDetailProduct(
        instrument,
        stockBetaDictionary[locale],
        locale,
        `${session.user_id}:${session.expires_at_secs}`,
      ),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
