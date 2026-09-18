import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import { StatePanel } from "@/components/states/state-panel";
import type { StockBetaChartError } from "@/components/stock-beta/dashboard/types";
import { StockBetaPolicyNotice } from "@/components/stock-beta/dashboard/widgets/policy-boundary-widget";
import { isStockBetaIntradayQuotesEnabled } from "@/components/stock-beta/quote/intraday-quotes-mode";
import { StockBetaWorkspace } from "@/components/stock-beta/stock-beta-workspace";
import { StockBetaTerminalPage } from "@/components/stock-beta/terminal";
import { ApiContractError, ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import { getLocale } from "@/lib/i18n/server";
import {
  assertOwnerEquityV2ChartMatchesExpectation,
  OwnerEquityV2ChartIntegrityError,
  type OwnerEquityV2ChartModel,
  type OwnerEquityV2LatestSignalsModel,
} from "@/lib/products/equity-signals-contracts";

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

function signalFailureCode(error: unknown): string {
  if (error instanceof ApiProblem) return error.code;
  if (error instanceof ApiContractError) return "CONTRACT_ERROR";
  return "UNCLASSIFIED_ERROR";
}

function chartFailure(error: unknown): StockBetaChartError {
  if (error instanceof ApiProblem) {
    if (error.code === "OWNER_EQUITY_CHART_UNAVAILABLE") {
      return { code: "OWNER_EQUITY_CHART_UNAVAILABLE", kind: "unavailable" };
    }
    if (error.code === "OWNER_EQUITY_INTEGRITY_FAILED") {
      return { code: "OWNER_EQUITY_INTEGRITY_FAILED", kind: "integrity" };
    }
    return { code: error.code, kind: "error" };
  }
  if (error instanceof OwnerEquityV2ChartIntegrityError) {
    return { code: "OWNER_EQUITY_INTEGRITY_FAILED", kind: "integrity" };
  }
  if (error instanceof ApiContractError) {
    return { code: "CHART_CONTRACT_INVALID", kind: "integrity" };
  }
  return { code: "UNCLASSIFIED_ERROR", kind: "error" };
}

async function renderStockBetaProduct(t: StockBetaDictionary, locale: Locale) {
  try {
    const intradayEnabled = isStockBetaIntradayQuotesEnabled();
    const api = await getProductApi();
    const memberships = await api.getOwnerEquityV2Memberships();
    let signals: OwnerEquityV2LatestSignalsModel | null = null;
    let initialSignalUnavailable = false;
    let initialSignalError: string | null = null;
    try {
      signals = await api.getOwnerEquityV2LatestSignals();
    } catch (error) {
      if (isLoginRequiredError(error)) redirect("/login");
      if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE")
        initialSignalUnavailable = true;
      else initialSignalError = signalFailureCode(error);
    }
    let initialChart: OwnerEquityV2ChartModel | null = null;
    let initialChartError: StockBetaChartError | null = null;
    const defaultSignal = signals?.top5[0] ?? signals?.rows[0];
    if (
      signals !== null &&
      defaultSignal !== undefined &&
      typeof api.getOwnerEquityV2Chart === "function"
    ) {
      try {
        const chart = await api.getOwnerEquityV2Chart(
          defaultSignal.instrument_id,
          signals.snapshot.snapshot_id,
          "1y",
        );
        initialChart = assertOwnerEquityV2ChartMatchesExpectation(chart, {
          asOf: signals.snapshot.as_of,
          generation: defaultSignal.generation,
          instrumentId: defaultSignal.instrument_id,
          range: "1y",
          snapshotId: signals.snapshot.snapshot_id,
        });
      } catch (error) {
        if (isLoginRequiredError(error)) redirect("/login");
        initialChartError = chartFailure(error);
      }
    }
    return (
      <StockBetaWorkspace
        initialChart={initialChart}
        initialChartError={initialChartError}
        initialMemberships={memberships}
        initialSignalError={initialSignalError}
        initialSignalUnavailable={initialSignalUnavailable}
        initialSignals={signals}
        intradayEnabled={intradayEnabled}
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

export default async function StockBetaPage() {
  const locale = await getLocale();
  return OwnerBetaProductRoute({
    product: "stock-beta",
    renderProduct: () => renderStockBetaProduct(stockBetaDictionary[locale], locale),
    title: stockBetaDictionary[locale].pageTitle,
  });
}
