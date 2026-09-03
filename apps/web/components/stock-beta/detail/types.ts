import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerBetaEquitySignalsDetailModel } from "@/lib/products/equity-signals-contracts";

export type StockBetaDetailCopy = StockBetaDictionary;

export type StockBetaDetailViewModel = {
  readonly backHref: string;
  readonly copy: StockBetaDetailCopy;
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly locale: Locale;
};

export type StockBetaDetailWidgetViewModel = StockBetaDetailViewModel;

export const STOCK_BETA_DETAIL_WIDGET_IDS = [
  "returns",
  "activity",
  "factor-evidence",
  "policy-boundary",
  "provenance",
] as const;

export type StockBetaDetailWidgetId = (typeof STOCK_BETA_DETAIL_WIDGET_IDS)[number];
