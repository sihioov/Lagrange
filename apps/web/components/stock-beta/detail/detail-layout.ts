import type { StockBetaWidgetLayout } from "../shared/widget-types";
import type { StockBetaDetailWidgetId } from "./types";

export const stockBetaDetailLayout = {
  desktop: [
    { id: "returns", size: "medium", visible: true, order: 0 },
    { id: "activity", size: "medium", visible: true, order: 1 },
    { id: "factor-evidence", size: "medium", visible: true, order: 2 },
    { id: "policy-boundary", size: "full", visible: true, order: 3 },
    { id: "provenance", size: "full", visible: true, order: 4 },
  ],
  tablet: [
    { id: "returns", size: "medium", visible: true, order: 0 },
    { id: "activity", size: "medium", visible: true, order: 1 },
    { id: "factor-evidence", size: "full", visible: true, order: 2 },
    { id: "policy-boundary", size: "full", visible: true, order: 3 },
    { id: "provenance", size: "full", visible: true, order: 4 },
  ],
  mobile: [
    { id: "returns", size: "full", visible: true, order: 0 },
    { id: "activity", size: "full", visible: true, order: 1 },
    { id: "factor-evidence", size: "full", visible: true, order: 2 },
    { id: "policy-boundary", size: "full", visible: true, order: 3 },
    { id: "provenance", size: "full", visible: true, order: 4 },
  ],
} as const satisfies StockBetaWidgetLayout<StockBetaDetailWidgetId>;
