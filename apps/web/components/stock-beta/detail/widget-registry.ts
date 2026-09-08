import { CurrentQuoteWidget } from "../quote/current-quote-widget";
import {
  defineStockBetaWidgetArchitecture,
  defineStockBetaWidgetCatalog,
  type StockBetaWidgetPlacementState,
  type StockBetaWidgetSize,
} from "../shared/widget-types";
import type { StockBetaDetailWidgetViewModel } from "./types";
import { ActivityWidget } from "./widgets/activity-widget";
import { InstrumentHeaderWidget } from "./widgets/instrument-header-widget";
import { PolicyBoundaryWidget } from "./widgets/policy-boundary-widget";
import { ProvenanceWidget } from "./widgets/provenance-widget";
import { ReturnsWidget } from "./widgets/returns-widget";
import { RiskWidget } from "./widgets/risk-widget";

function visible(size: StockBetaWidgetSize): StockBetaWidgetPlacementState {
  return { size, visible: true };
}

export const defineStockBetaDetailCatalog =
  defineStockBetaWidgetCatalog<StockBetaDetailWidgetViewModel>();

/** Array order is the canonical detail DOM and accessibility reading order. */
export const stockBetaDetailCatalog = defineStockBetaDetailCatalog([
  {
    id: "instrument-header",
    component: InstrumentHeaderWidget,
    required: true,
    placements: {
      desktop: visible("full"),
      tablet: visible("full"),
      mobile: visible("full"),
    },
  },
  {
    id: "returns",
    component: ReturnsWidget,
    required: true,
    placements: {
      desktop: visible("medium"),
      tablet: visible("medium"),
      mobile: visible("full"),
    },
  },
  {
    id: "risk",
    component: RiskWidget,
    required: true,
    placements: {
      desktop: visible("medium"),
      tablet: visible("medium"),
      mobile: visible("full"),
    },
  },
  {
    id: "activity",
    component: ActivityWidget,
    required: true,
    placements: {
      desktop: visible("medium"),
      tablet: visible("medium"),
      mobile: visible("full"),
    },
  },
  {
    id: "snapshot",
    component: ProvenanceWidget,
    required: true,
    placements: {
      desktop: visible("full"),
      tablet: visible("full"),
      mobile: visible("full"),
    },
  },
  {
    id: "policy-boundary",
    component: PolicyBoundaryWidget,
    required: true,
    placements: {
      desktop: visible("full"),
      tablet: visible("full"),
      mobile: visible("full"),
    },
  },
  {
    id: "current-quote",
    component: CurrentQuoteWidget,
    required: false,
    placements: {
      desktop: visible("full"),
      tablet: visible("full"),
      mobile: visible("full"),
    },
  },
]);

export type StockBetaDetailWidgetId = (typeof stockBetaDetailCatalog)[number]["id"];

export const stockBetaDetailArchitecture =
  defineStockBetaWidgetArchitecture(stockBetaDetailCatalog);
