import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type {
  OwnerEquityV2LatestSignalsModel,
  OwnerEquityV2MembershipModel,
  OwnerEquityV2PolicyModel,
} from "@/lib/products/equity-signals-contracts";

export type StockBetaDashboardCopy = StockBetaDictionary;

export type StockBetaSignalState =
  | { readonly kind: "ready" }
  | { readonly kind: "not-ready" }
  | { readonly kind: "unavailable" }
  | { readonly code: string; readonly kind: "error" };

export type StockBetaDashboardViewModel = {
  readonly actionError: string | null;
  readonly actionMessage: string | null;
  readonly busy: boolean;
  readonly copy: StockBetaDashboardCopy;
  readonly disableId: string | null;
  readonly inputError: string | null;
  readonly instrumentCode: string;
  readonly locale: Locale;
  readonly memberships: readonly OwnerEquityV2MembershipModel[];
  readonly mutationPending: boolean;
  readonly onAdd: () => Promise<void>;
  readonly onCancelDisable: () => void;
  readonly onConfirmDisable: () => Promise<void>;
  readonly onInstrumentCodeChange: (value: string) => void;
  readonly onRequestDisable: (membershipId: string) => void;
  readonly onRetry: (membershipId: string) => Promise<void>;
  readonly pendingMembershipId: string | null;
  readonly policy: OwnerEquityV2PolicyModel;
  readonly pollError: boolean;
  readonly signalState: StockBetaSignalState;
  readonly signals: OwnerEquityV2LatestSignalsModel | null;
};

export type StockBetaDashboardWidgetViewModel = StockBetaDashboardViewModel;

export const STOCK_BETA_DASHBOARD_WIDGET_IDS = [
  "universe-management",
  "membership-status",
  "signal-state",
  "ranked-signals",
  "signal-profile",
  "signal-decomposition",
  "condition-matrix",
  "snapshot-tape",
  "policy-boundary",
  "provenance",
] as const;

export type StockBetaDashboardWidgetId = (typeof STOCK_BETA_DASHBOARD_WIDGET_IDS)[number];
