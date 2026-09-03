import { defineStockBetaWidget, defineStockBetaWidgetArchitecture } from "../shared/widget-types";
import { stockBetaDetailLayout } from "./detail-layout";
import type { StockBetaDetailWidgetViewModel } from "./types";
import { ActivityWidget } from "./widgets/activity-widget";
import { FactorEvidenceWidget } from "./widgets/factor-evidence-widget";
import { PolicyBoundaryWidget } from "./widgets/policy-boundary-widget";
import { ProvenanceWidget } from "./widgets/provenance-widget";
import { ReturnsWidget } from "./widgets/returns-widget";

const returns = defineStockBetaWidget<"returns", StockBetaDetailWidgetViewModel>({
  id: "returns",
  component: ReturnsWidget,
  defaultSize: "medium",
  required: true,
  defaultVisible: true,
  order: 0,
});

const activity = defineStockBetaWidget<"activity", StockBetaDetailWidgetViewModel>({
  id: "activity",
  component: ActivityWidget,
  defaultSize: "medium",
  required: true,
  defaultVisible: true,
  order: 1,
});

const factorEvidence = defineStockBetaWidget<"factor-evidence", StockBetaDetailWidgetViewModel>({
  id: "factor-evidence",
  component: FactorEvidenceWidget,
  defaultSize: "large",
  required: true,
  defaultVisible: true,
  order: 2,
});

const policyBoundary = defineStockBetaWidget<"policy-boundary", StockBetaDetailWidgetViewModel>({
  id: "policy-boundary",
  component: PolicyBoundaryWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 3,
});

const provenance = defineStockBetaWidget<"provenance", StockBetaDetailWidgetViewModel>({
  id: "provenance",
  component: ProvenanceWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 4,
});

export const stockBetaDetailDefinitions = [
  returns,
  activity,
  factorEvidence,
  policyBoundary,
  provenance,
] as const;

export const stockBetaDetailArchitecture = defineStockBetaWidgetArchitecture({
  definitions: stockBetaDetailDefinitions,
  requiredWidgetIds: ["returns", "activity", "factor-evidence", "policy-boundary", "provenance"],
  layout: stockBetaDetailLayout,
});
