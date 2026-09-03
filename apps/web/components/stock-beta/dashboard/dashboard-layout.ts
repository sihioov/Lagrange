import type { StockBetaWidgetLayout } from "../shared/widget-types";
import type { StockBetaDashboardWidgetId } from "./types";

export const stockBetaDashboardLayout = {
  desktop: [
    { id: "policy-boundary", size: "full", visible: true, order: 0 },
    { id: "ranked-signals", size: "small", visible: true, order: 1 },
    { id: "signal-profile", size: "large", visible: true, order: 2 },
    { id: "signal-decomposition", size: "small", visible: true, order: 3 },
    { id: "condition-matrix", size: "small", visible: true, order: 4 },
    { id: "snapshot-tape", size: "large", visible: true, order: 5 },
    { id: "provenance", size: "full", visible: true, order: 6 },
  ],
  tablet: [
    { id: "policy-boundary", size: "full", visible: true, order: 0 },
    { id: "ranked-signals", size: "medium", visible: true, order: 1 },
    { id: "signal-profile", size: "medium", visible: true, order: 2 },
    { id: "signal-decomposition", size: "full", visible: true, order: 3 },
    { id: "condition-matrix", size: "medium", visible: true, order: 4 },
    { id: "snapshot-tape", size: "medium", visible: true, order: 5 },
    { id: "provenance", size: "full", visible: true, order: 6 },
  ],
  mobile: [
    { id: "policy-boundary", size: "full", visible: true, order: 0 },
    { id: "ranked-signals", size: "full", visible: true, order: 1 },
    { id: "signal-profile", size: "full", visible: true, order: 2 },
    { id: "signal-decomposition", size: "full", visible: true, order: 3 },
    { id: "condition-matrix", size: "full", visible: true, order: 4 },
    { id: "snapshot-tape", size: "full", visible: true, order: 5 },
    { id: "provenance", size: "full", visible: true, order: 6 },
  ],
} as const satisfies StockBetaWidgetLayout<StockBetaDashboardWidgetId>;
