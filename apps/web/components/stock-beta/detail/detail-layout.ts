import type { StockBetaWidgetLayout } from "../shared/widget-types";
import type { StockBetaDetailWidgetId } from "./types";

const ids = [
  "instrument-header",
  "returns",
  "risk",
  "activity",
  "snapshot",
  "policy-boundary",
] as const;
export const stockBetaDetailLayout = {
  desktop: ids.map((id, order) => ({
    id,
    size:
      id === "instrument-header" || id === "snapshot" || id === "policy-boundary"
        ? "full"
        : "medium",
    visible: true,
    order,
  })),
  tablet: ids.map((id, order) => ({
    id,
    size:
      id === "instrument-header" || id === "snapshot" || id === "policy-boundary"
        ? "full"
        : "medium",
    visible: true,
    order,
  })),
  mobile: ids.map((id, order) => ({ id, size: "full", visible: true, order })),
} as const satisfies StockBetaWidgetLayout<StockBetaDetailWidgetId>;
