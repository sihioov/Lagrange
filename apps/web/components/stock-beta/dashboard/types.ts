import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type {
  OwnerBetaEquitySignalsFilters,
  OwnerBetaEquitySignalsLatestModel,
  OwnerBetaEquitySignalsScreenModel,
} from "@/lib/products/equity-signals-contracts";

export type StockBetaDashboardCopy = Omit<StockBetaDictionary, "detailTitle">;

export type StockBetaDashboardData =
  | OwnerBetaEquitySignalsLatestModel
  | OwnerBetaEquitySignalsScreenModel;

export type StockBetaDashboardBaseViewModel = {
  readonly copy: StockBetaDashboardCopy;
  readonly data: StockBetaDashboardData;
  readonly filters: OwnerBetaEquitySignalsFilters;
  readonly locale: Locale;
  readonly filtered: boolean;
};

// Static widgets receive only server-supplied data. Selection belongs to the narrow client island.
export type StockBetaDashboardWidgetViewModel = StockBetaDashboardBaseViewModel;

export const STOCK_BETA_DASHBOARD_WIDGET_IDS = [
  "policy-boundary",
  "ranked-signals",
  "signal-profile",
  "signal-decomposition",
  "condition-matrix",
  "snapshot-tape",
  "provenance",
] as const;

export type StockBetaDashboardWidgetId = (typeof STOCK_BETA_DASHBOARD_WIDGET_IDS)[number];

export type StockBetaPolicyCopy = Pick<
  StockBetaDashboardCopy,
  | "activityPolicy"
  | "conditionPolicy"
  | "fixedListPolicy"
  | "originalPricePolicy"
  | "policyAriaLabel"
  | "policyBoundaryDescription"
  | "policyBoundaryDetailsLabel"
  | "policyBoundaryHeading"
  | "policyBoundarySummary"
  | "warningLabel"
>;

export type StockBetaProvenanceCopy = Pick<
  StockBetaDashboardCopy,
  | "activityProxyLabel"
  | "artifactHashLabel"
  | "asOfLabel"
  | "audienceLabel"
  | "batchIdLabel"
  | "capabilityLabel"
  | "entitlementHashLabel"
  | "factorVersionLabel"
  | "indexMembershipLabel"
  | "materializationStatusLabel"
  | "originalPriceLabel"
  | "publicationStatusLabel"
  | "provenanceDescription"
  | "provenanceDisclosureLabel"
  | "provenanceHeading"
  | "redistributionLabel"
  | "registrationStatusLabel"
  | "registryHashLabel"
  | "selectionBasisLabel"
  | "snapshotHashLabel"
  | "strictPitLabel"
  | "universeHashLabel"
  | "vendorSnapshotLabel"
  | "warningLabel"
  | "yes"
  | "no"
>;
