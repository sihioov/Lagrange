import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerEquityV2SignalDetailModel } from "@/lib/products/equity-signals-contracts";

export type StockBetaDetailViewModel = {
  readonly backHref: string;
  readonly copy: StockBetaDictionary;
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly locale: Locale;
};
export type StockBetaDetailWidgetViewModel = StockBetaDetailViewModel;
export const STOCK_BETA_DETAIL_WIDGET_IDS = [
  "instrument-header",
  "returns",
  "risk",
  "activity",
  "snapshot",
  "policy-boundary",
] as const;
export type StockBetaDetailWidgetId = (typeof STOCK_BETA_DETAIL_WIDGET_IDS)[number];
