import { defineStockBetaWidget, defineStockBetaWidgetArchitecture } from "../shared/widget-types";
import { stockBetaDashboardLayout } from "./dashboard-layout";
import type { StockBetaDashboardWidgetViewModel } from "./types";
import { ConditionMatrixWidget } from "./widgets/condition-matrix-widget";
import { PolicyBoundaryWidget } from "./widgets/policy-boundary-widget";
import { ProvenanceWidget } from "./widgets/provenance-widget";
import { RankedSignalsWidget } from "./widgets/ranked-signals-widget";
import { SignalDecompositionWidget } from "./widgets/signal-decomposition-widget";
import { SignalPreviewWidget } from "./widgets/signal-preview-widget";
import { SnapshotTapeWidget } from "./widgets/snapshot-tape-widget";

const policyBoundary = defineStockBetaWidget<"policy-boundary", StockBetaDashboardWidgetViewModel>({
  id: "policy-boundary",
  component: PolicyBoundaryWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 0,
});

const rankedSignals = defineStockBetaWidget<"ranked-signals", StockBetaDashboardWidgetViewModel>({
  id: "ranked-signals",
  component: RankedSignalsWidget,
  defaultSize: "small",
  required: true,
  defaultVisible: true,
  order: 1,
});

const signalProfile = defineStockBetaWidget<"signal-profile", StockBetaDashboardWidgetViewModel>({
  id: "signal-profile",
  component: SignalPreviewWidget,
  defaultSize: "large",
  required: true,
  defaultVisible: true,
  order: 2,
});

const signalDecomposition = defineStockBetaWidget<
  "signal-decomposition",
  StockBetaDashboardWidgetViewModel
>({
  id: "signal-decomposition",
  component: SignalDecompositionWidget,
  defaultSize: "small",
  required: true,
  defaultVisible: true,
  order: 3,
});

const conditionMatrix = defineStockBetaWidget<
  "condition-matrix",
  StockBetaDashboardWidgetViewModel
>({
  id: "condition-matrix",
  component: ConditionMatrixWidget,
  defaultSize: "small",
  required: true,
  defaultVisible: true,
  order: 4,
});

const snapshotTape = defineStockBetaWidget<"snapshot-tape", StockBetaDashboardWidgetViewModel>({
  id: "snapshot-tape",
  component: SnapshotTapeWidget,
  defaultSize: "large",
  required: true,
  defaultVisible: true,
  order: 5,
});

const provenance = defineStockBetaWidget<"provenance", StockBetaDashboardWidgetViewModel>({
  id: "provenance",
  component: ProvenanceWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 6,
});

export const stockBetaDashboardDefinitions = [
  policyBoundary,
  rankedSignals,
  signalProfile,
  signalDecomposition,
  conditionMatrix,
  snapshotTape,
  provenance,
] as const;

export const stockBetaDashboardArchitecture = defineStockBetaWidgetArchitecture({
  definitions: stockBetaDashboardDefinitions,
  requiredWidgetIds: [
    "policy-boundary",
    "ranked-signals",
    "signal-profile",
    "signal-decomposition",
    "condition-matrix",
    "snapshot-tape",
    "provenance",
  ],
  layout: stockBetaDashboardLayout,
});
