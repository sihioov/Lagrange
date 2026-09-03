import type { ComponentType } from "react";
import {
  defineStockBetaWidget,
  defineStockBetaWidgetArchitecture,
  type StockBetaWidgetSize,
} from "../shared/widget-types";
import { stockBetaDashboardLayout } from "./dashboard-layout";
import type { StockBetaDashboardWidgetId, StockBetaDashboardWidgetViewModel } from "./types";
import { ConditionMatrixWidget } from "./widgets/condition-matrix-widget";
import { MembershipStatusWidget } from "./widgets/membership-status-widget";
import { PolicyBoundaryWidget } from "./widgets/policy-boundary-widget";
import { ProvenanceWidget } from "./widgets/provenance-widget";
import { RankedSignalsWidget } from "./widgets/ranked-signals-widget";
import { SignalDecompositionWidget } from "./widgets/signal-decomposition-widget";
import { SignalPreviewWidget } from "./widgets/signal-preview-widget";
import { SignalStateWidget } from "./widgets/signal-state-widget";
import { SnapshotTapeWidget } from "./widgets/snapshot-tape-widget";
import { UniverseManagementWidget } from "./widgets/universe-management-widget";

function widget(
  id: StockBetaDashboardWidgetId,
  component: ComponentType<{ readonly viewModel: StockBetaDashboardWidgetViewModel }>,
  order: number,
  defaultSize: StockBetaWidgetSize,
  required = true,
) {
  return defineStockBetaWidget({
    id,
    component,
    defaultSize,
    required,
    defaultVisible: true,
    order,
  });
}

export const stockBetaDashboardDefinitions = [
  widget("universe-management", UniverseManagementWidget, 0, "large"),
  widget("membership-status", MembershipStatusWidget, 1, "medium"),
  widget("signal-state", SignalStateWidget, 2, "full", false),
  widget("ranked-signals", RankedSignalsWidget, 3, "small"),
  widget("signal-profile", SignalPreviewWidget, 4, "large"),
  widget("signal-decomposition", SignalDecompositionWidget, 5, "small"),
  widget("condition-matrix", ConditionMatrixWidget, 6, "small"),
  widget("snapshot-tape", SnapshotTapeWidget, 7, "large"),
  widget("policy-boundary", PolicyBoundaryWidget, 8, "full"),
  widget("provenance", ProvenanceWidget, 9, "full"),
] as const;

export const stockBetaDashboardArchitecture = defineStockBetaWidgetArchitecture({
  definitions: stockBetaDashboardDefinitions,
  requiredWidgetIds: [
    "universe-management",
    "membership-status",
    "ranked-signals",
    "signal-profile",
    "signal-decomposition",
    "condition-matrix",
    "snapshot-tape",
    "policy-boundary",
    "provenance",
  ],
  layout: stockBetaDashboardLayout,
});
