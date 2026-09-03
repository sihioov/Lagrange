import type {
  StockBetaWidgetGridLayout,
  StockBetaWidgetGridPlacement,
  StockBetaWidgetGridState,
  StockBetaWidgetSize,
} from "../shared/widget-types";
import type { StockBetaDashboardWidgetId } from "./types";

type GridPosition = Omit<StockBetaWidgetGridState, "order" | "visible">;

function gridState(
  column: number,
  columnSpan: number,
  row: number,
  order: number,
  visible: boolean,
): StockBetaWidgetGridState {
  return { column, columnSpan, row, order, visible };
}

function placement(
  id: StockBetaDashboardWidgetId,
  size: StockBetaWidgetSize,
  position: GridPosition,
  order: number,
  visible: boolean,
  empty: StockBetaWidgetGridState,
): StockBetaWidgetGridPlacement<StockBetaDashboardWidgetId> {
  return { id, size, ...position, order, visible, empty };
}

const desktop = [
  placement(
    "ranked-signals",
    "small",
    { column: 1, columnSpan: 3, row: 1 },
    0,
    true,
    gridState(1, 3, 1, 4, false),
  ),
  placement(
    "signal-profile",
    "large",
    { column: 4, columnSpan: 6, row: 1 },
    1,
    true,
    gridState(4, 6, 1, 5, false),
  ),
  placement(
    "signal-decomposition",
    "small",
    { column: 10, columnSpan: 3, row: 1 },
    2,
    true,
    gridState(10, 3, 1, 6, false),
  ),
  placement(
    "condition-matrix",
    "small",
    { column: 1, columnSpan: 3, row: 2 },
    3,
    true,
    gridState(1, 3, 2, 7, false),
  ),
  placement(
    "snapshot-tape",
    "large",
    { column: 4, columnSpan: 9, row: 2 },
    4,
    true,
    gridState(4, 9, 2, 8, false),
  ),
  placement(
    "universe-management",
    "large",
    { column: 1, columnSpan: 7, row: 3 },
    5,
    true,
    gridState(1, 7, 1, 0, true),
  ),
  placement(
    "membership-status",
    "medium",
    { column: 8, columnSpan: 5, row: 3 },
    6,
    true,
    gridState(8, 5, 1, 1, true),
  ),
  placement(
    "signal-state",
    "full",
    { column: 1, columnSpan: 12, row: 2 },
    7,
    false,
    gridState(1, 12, 2, 2, true),
  ),
  placement(
    "policy-boundary",
    "full",
    { column: 1, columnSpan: 12, row: 4 },
    8,
    true,
    gridState(1, 12, 3, 3, true),
  ),
  placement(
    "provenance",
    "full",
    { column: 1, columnSpan: 12, row: 5 },
    9,
    true,
    gridState(1, 12, 4, 9, false),
  ),
] as const;

const tablet = [
  placement(
    "universe-management",
    "full",
    { column: 1, columnSpan: 12, row: 1 },
    0,
    true,
    gridState(1, 12, 1, 0, true),
  ),
  placement(
    "membership-status",
    "full",
    { column: 1, columnSpan: 12, row: 2 },
    1,
    true,
    gridState(1, 12, 2, 1, true),
  ),
  placement(
    "signal-state",
    "full",
    { column: 1, columnSpan: 12, row: 3 },
    2,
    false,
    gridState(1, 12, 3, 2, true),
  ),
  placement(
    "ranked-signals",
    "medium",
    { column: 1, columnSpan: 6, row: 3 },
    3,
    true,
    gridState(1, 6, 3, 3, false),
  ),
  placement(
    "signal-profile",
    "medium",
    { column: 7, columnSpan: 6, row: 3 },
    4,
    true,
    gridState(7, 6, 3, 4, false),
  ),
  placement(
    "signal-decomposition",
    "full",
    { column: 1, columnSpan: 12, row: 4 },
    5,
    true,
    gridState(1, 12, 4, 5, false),
  ),
  placement(
    "condition-matrix",
    "full",
    { column: 1, columnSpan: 6, row: 5 },
    6,
    true,
    gridState(1, 6, 5, 6, false),
  ),
  placement(
    "snapshot-tape",
    "full",
    { column: 7, columnSpan: 6, row: 5 },
    7,
    true,
    gridState(7, 6, 5, 7, false),
  ),
  placement(
    "policy-boundary",
    "full",
    { column: 1, columnSpan: 12, row: 6 },
    8,
    true,
    gridState(1, 12, 4, 8, true),
  ),
  placement(
    "provenance",
    "full",
    { column: 1, columnSpan: 12, row: 7 },
    9,
    true,
    gridState(1, 12, 5, 9, false),
  ),
] as const;

const mobile = [
  placement(
    "universe-management",
    "full",
    { column: 1, columnSpan: 1, row: 1 },
    0,
    true,
    gridState(1, 1, 1, 0, true),
  ),
  placement(
    "membership-status",
    "full",
    { column: 1, columnSpan: 1, row: 2 },
    1,
    true,
    gridState(1, 1, 2, 1, true),
  ),
  placement(
    "signal-state",
    "full",
    { column: 1, columnSpan: 1, row: 3 },
    2,
    false,
    gridState(1, 1, 3, 2, true),
  ),
  placement(
    "ranked-signals",
    "full",
    { column: 1, columnSpan: 1, row: 3 },
    3,
    true,
    gridState(1, 1, 3, 3, false),
  ),
  placement(
    "signal-profile",
    "full",
    { column: 1, columnSpan: 1, row: 4 },
    4,
    true,
    gridState(1, 1, 4, 4, false),
  ),
  placement(
    "signal-decomposition",
    "full",
    { column: 1, columnSpan: 1, row: 5 },
    5,
    true,
    gridState(1, 1, 5, 5, false),
  ),
  placement(
    "condition-matrix",
    "full",
    { column: 1, columnSpan: 1, row: 6 },
    6,
    true,
    gridState(1, 1, 6, 6, false),
  ),
  placement(
    "snapshot-tape",
    "full",
    { column: 1, columnSpan: 1, row: 7 },
    7,
    true,
    gridState(1, 1, 7, 7, false),
  ),
  placement(
    "policy-boundary",
    "full",
    { column: 1, columnSpan: 1, row: 8 },
    8,
    true,
    gridState(1, 1, 4, 8, true),
  ),
  placement(
    "provenance",
    "full",
    { column: 1, columnSpan: 1, row: 9 },
    9,
    true,
    gridState(1, 1, 5, 9, false),
  ),
] as const;

export const stockBetaDashboardLayout = {
  desktop,
  tablet,
  mobile,
} as const satisfies StockBetaWidgetGridLayout<StockBetaDashboardWidgetId>;
