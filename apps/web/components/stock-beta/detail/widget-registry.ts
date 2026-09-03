import { defineStockBetaWidget, defineStockBetaWidgetArchitecture } from "../shared/widget-types";
import { stockBetaDetailLayout } from "./detail-layout";
import type { StockBetaDetailWidgetViewModel } from "./types";
import { ActivityWidget } from "./widgets/activity-widget";
import { InstrumentHeaderWidget } from "./widgets/instrument-header-widget";
import { PolicyBoundaryWidget } from "./widgets/policy-boundary-widget";
import { ProvenanceWidget } from "./widgets/provenance-widget";
import { ReturnsWidget } from "./widgets/returns-widget";
import { RiskWidget } from "./widgets/risk-widget";

const instrumentHeader = defineStockBetaWidget<"instrument-header", StockBetaDetailWidgetViewModel>(
  {
    id: "instrument-header",
    component: InstrumentHeaderWidget,
    defaultSize: "full",
    required: true,
    defaultVisible: true,
    order: 0,
  },
);
const returns = defineStockBetaWidget<"returns", StockBetaDetailWidgetViewModel>({
  id: "returns",
  component: ReturnsWidget,
  defaultSize: "medium",
  required: true,
  defaultVisible: true,
  order: 1,
});
const risk = defineStockBetaWidget<"risk", StockBetaDetailWidgetViewModel>({
  id: "risk",
  component: RiskWidget,
  defaultSize: "medium",
  required: true,
  defaultVisible: true,
  order: 2,
});
const activity = defineStockBetaWidget<"activity", StockBetaDetailWidgetViewModel>({
  id: "activity",
  component: ActivityWidget,
  defaultSize: "medium",
  required: true,
  defaultVisible: true,
  order: 3,
});
const snapshot = defineStockBetaWidget<"snapshot", StockBetaDetailWidgetViewModel>({
  id: "snapshot",
  component: ProvenanceWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 4,
});
const policyBoundary = defineStockBetaWidget<"policy-boundary", StockBetaDetailWidgetViewModel>({
  id: "policy-boundary",
  component: PolicyBoundaryWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 5,
});

export const stockBetaDetailDefinitions = [
  instrumentHeader,
  returns,
  risk,
  activity,
  snapshot,
  policyBoundary,
] as const;
export const stockBetaDetailArchitecture = defineStockBetaWidgetArchitecture({
  definitions: stockBetaDetailDefinitions,
  requiredWidgetIds: [
    "instrument-header",
    "returns",
    "risk",
    "activity",
    "snapshot",
    "policy-boundary",
  ],
  layout: stockBetaDetailLayout,
});
