import type { StatusTone } from "@/components/states/status-pill";
import type { OwnerEquityV2SignalModel } from "@/lib/products/equity-signals-contracts";
import type { StockBetaDashboardCopy } from "./types";

export function stockBetaConditionLabel(
  condition: OwnerEquityV2SignalModel["condition"],
  t: Pick<StockBetaDashboardCopy, "bearishLabel" | "bullishLabel" | "neutralLabel">,
): string {
  switch (condition) {
    case "BULLISH":
      return t.bullishLabel;
    case "NEUTRAL":
      return t.neutralLabel;
    case "BEARISH":
      return t.bearishLabel;
  }
}

export function stockBetaConditionTone(
  condition: OwnerEquityV2SignalModel["condition"],
): StatusTone {
  switch (condition) {
    case "BULLISH":
      return "success";
    case "NEUTRAL":
      return "neutral";
    case "BEARISH":
      return "warning";
  }
}
