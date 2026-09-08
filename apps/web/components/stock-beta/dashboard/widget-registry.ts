import { CurrentQuoteWidget } from "../quote/current-quote-widget";
import {
  defineStockBetaWidgetArchitecture,
  defineStockBetaWidgetCatalog,
  type StockBetaWidgetGridPlacementState,
  type StockBetaWidgetSize,
} from "../shared/widget-types";
import type { StockBetaDashboardWidgetViewModel } from "./types";
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

function gridPlacement(
  size: StockBetaWidgetSize,
  column: number,
  columnSpan: number,
  row: number,
  visible: boolean,
  empty: StockBetaWidgetGridPlacementState["empty"],
): StockBetaWidgetGridPlacementState {
  return { size, column, columnSpan, row, visible, empty };
}

export const defineStockBetaDashboardCatalog =
  defineStockBetaWidgetCatalog<StockBetaDashboardWidgetViewModel>();

/** Array order is the canonical dashboard DOM and accessibility reading order. */
export const stockBetaDashboardCatalog = defineStockBetaDashboardCatalog([
  {
    id: "universe-management",
    component: UniverseManagementWidget,
    required: true,
    placements: {
      desktop: gridPlacement("large", 1, 7, 3, true, {
        column: 1,
        columnSpan: 7,
        row: 1,
        visible: true,
      }),
      tablet: gridPlacement("full", 1, 12, 1, true, {
        column: 1,
        columnSpan: 12,
        row: 1,
        visible: true,
      }),
      mobile: gridPlacement("full", 1, 1, 1, true, {
        column: 1,
        columnSpan: 1,
        row: 1,
        visible: true,
      }),
    },
  },
  {
    id: "membership-status",
    component: MembershipStatusWidget,
    required: true,
    placements: {
      desktop: gridPlacement("medium", 8, 5, 3, true, {
        column: 8,
        columnSpan: 5,
        row: 1,
        visible: true,
      }),
      tablet: gridPlacement("full", 1, 12, 2, true, {
        column: 1,
        columnSpan: 12,
        row: 2,
        visible: true,
      }),
      mobile: gridPlacement("full", 1, 1, 2, true, {
        column: 1,
        columnSpan: 1,
        row: 2,
        visible: true,
      }),
    },
  },
  {
    id: "signal-state",
    component: SignalStateWidget,
    required: false,
    placements: {
      desktop: gridPlacement("full", 1, 12, 2, false, {
        column: 1,
        columnSpan: 12,
        row: 2,
        visible: true,
      }),
      tablet: gridPlacement("full", 1, 12, 3, false, {
        column: 1,
        columnSpan: 12,
        row: 3,
        visible: true,
      }),
      mobile: gridPlacement("full", 1, 1, 3, false, {
        column: 1,
        columnSpan: 1,
        row: 3,
        visible: true,
      }),
    },
  },
  {
    id: "ranked-signals",
    component: RankedSignalsWidget,
    required: true,
    placements: {
      desktop: gridPlacement("small", 1, 3, 1, true, {
        column: 1,
        columnSpan: 3,
        row: 1,
        visible: false,
      }),
      tablet: gridPlacement("medium", 1, 6, 3, true, {
        column: 1,
        columnSpan: 6,
        row: 3,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 3, true, {
        column: 1,
        columnSpan: 1,
        row: 3,
        visible: false,
      }),
    },
  },
  {
    id: "signal-profile",
    component: SignalPreviewWidget,
    required: true,
    placements: {
      // The profile widget is a chart-capable primary analysis surface.
      desktop: gridPlacement("large", 4, 6, 1, true, {
        column: 4,
        columnSpan: 6,
        row: 1,
        visible: false,
      }),
      tablet: gridPlacement("medium", 7, 6, 3, true, {
        column: 7,
        columnSpan: 6,
        row: 3,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 4, true, {
        column: 1,
        columnSpan: 1,
        row: 4,
        visible: false,
      }),
    },
  },
  {
    id: "signal-decomposition",
    component: SignalDecompositionWidget,
    required: true,
    placements: {
      desktop: gridPlacement("small", 10, 3, 1, true, {
        column: 10,
        columnSpan: 3,
        row: 1,
        visible: false,
      }),
      tablet: gridPlacement("full", 1, 12, 4, true, {
        column: 1,
        columnSpan: 12,
        row: 4,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 5, true, {
        column: 1,
        columnSpan: 1,
        row: 5,
        visible: false,
      }),
    },
  },
  {
    id: "condition-matrix",
    component: ConditionMatrixWidget,
    required: true,
    placements: {
      desktop: gridPlacement("small", 1, 3, 2, true, {
        column: 1,
        columnSpan: 3,
        row: 2,
        visible: false,
      }),
      tablet: gridPlacement("full", 1, 6, 5, true, {
        column: 1,
        columnSpan: 6,
        row: 5,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 6, true, {
        column: 1,
        columnSpan: 1,
        row: 6,
        visible: false,
      }),
    },
  },
  {
    id: "snapshot-tape",
    component: SnapshotTapeWidget,
    required: true,
    placements: {
      desktop: gridPlacement("large", 4, 9, 2, true, {
        column: 4,
        columnSpan: 9,
        row: 2,
        visible: false,
      }),
      tablet: gridPlacement("full", 7, 6, 5, true, {
        column: 7,
        columnSpan: 6,
        row: 5,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 7, true, {
        column: 1,
        columnSpan: 1,
        row: 7,
        visible: false,
      }),
    },
  },
  {
    id: "policy-boundary",
    component: PolicyBoundaryWidget,
    required: true,
    placements: {
      desktop: gridPlacement("full", 1, 12, 4, true, {
        column: 1,
        columnSpan: 12,
        row: 3,
        visible: true,
      }),
      tablet: gridPlacement("full", 1, 12, 6, true, {
        column: 1,
        columnSpan: 12,
        row: 4,
        visible: true,
      }),
      mobile: gridPlacement("full", 1, 1, 8, true, {
        column: 1,
        columnSpan: 1,
        row: 4,
        visible: true,
      }),
    },
  },
  {
    id: "provenance",
    component: ProvenanceWidget,
    required: true,
    placements: {
      desktop: gridPlacement("full", 1, 12, 5, true, {
        column: 1,
        columnSpan: 12,
        row: 4,
        visible: false,
      }),
      tablet: gridPlacement("full", 1, 12, 7, true, {
        column: 1,
        columnSpan: 12,
        row: 5,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 9, true, {
        column: 1,
        columnSpan: 1,
        row: 5,
        visible: false,
      }),
    },
  },
  {
    id: "current-quote",
    component: CurrentQuoteWidget,
    required: false,
    placements: {
      desktop: gridPlacement("full", 1, 12, 6, true, {
        column: 1,
        columnSpan: 12,
        row: 6,
        visible: false,
      }),
      tablet: gridPlacement("full", 1, 12, 8, true, {
        column: 1,
        columnSpan: 12,
        row: 8,
        visible: false,
      }),
      mobile: gridPlacement("full", 1, 1, 10, true, {
        column: 1,
        columnSpan: 1,
        row: 10,
        visible: false,
      }),
    },
  },
]);

export type StockBetaDashboardWidgetId = (typeof stockBetaDashboardCatalog)[number]["id"];

export const stockBetaDashboardArchitecture =
  defineStockBetaWidgetArchitecture(stockBetaDashboardCatalog);
