import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Backtests",
};

export default function BacktestsPage() {
  return (
    <RoutePage
      description="Create reproducible simulations and inspect performance, cost, drawdown, and robustness evidence."
      title="Backtests"
    >
      <StatePanel
        action={
          <Link className="secondary-action" href="/strategies">
            Choose a strategy
          </Link>
        }
        kind="empty"
        message="No backtest is selected. A run can populate this route while preserving its strategy, data, and engine provenance."
        title="No backtest selected"
      />
    </RoutePage>
  );
}
