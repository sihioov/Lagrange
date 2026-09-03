import type { StockBetaWidgetLayout } from "../shared/widget-types";
import type { StockBetaDashboardWidgetId } from "./types";

const responsivePlacements = [
  ["universe-management", "large"],
  ["membership-status", "medium"],
  ["signal-state", "full"],
  ["ranked-signals", "small"],
  ["signal-profile", "large"],
  ["signal-decomposition", "small"],
  ["condition-matrix", "small"],
  ["snapshot-tape", "large"],
  ["policy-boundary", "full"],
  ["provenance", "full"],
] as const;

const desktopPlacements = [
  ["ranked-signals", "small"],
  ["signal-profile", "large"],
  ["signal-decomposition", "small"],
  ["condition-matrix", "small"],
  ["snapshot-tape", "large"],
  ["universe-management", "large"],
  ["membership-status", "medium"],
  ["signal-state", "full"],
  ["policy-boundary", "full"],
  ["provenance", "full"],
] as const;

export const stockBetaDashboardLayout = {
  desktop: desktopPlacements.map(([id, size], order) => ({ id, size, visible: true, order })),
  tablet: responsivePlacements.map(([id], order) => ({
    id,
    size: id === "ranked-signals" || id === "signal-profile" ? "medium" : "full",
    visible: true,
    order,
  })),
  mobile: responsivePlacements.map(([id], order) => ({
    id,
    size: "full",
    visible: true,
    order,
  })),
} as const satisfies StockBetaWidgetLayout<StockBetaDashboardWidgetId>;
