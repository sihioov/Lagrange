import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type {
  OwnerEquityV2MembershipModel,
  OwnerEquityV2SignalDetailModel,
} from "@/lib/products/equity-signals-contracts";

export type StockBetaDetailViewModel = {
  readonly backHref: string;
  readonly copy: StockBetaDictionary;
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly intradayEnabled?: boolean;
  readonly marketStreamEnabled?: boolean;
  readonly intradayMembership?: OwnerEquityV2MembershipModel | null;
  readonly locale: Locale;
};
export type StockBetaDetailWidgetViewModel = StockBetaDetailViewModel;
