import type { Metadata } from "next";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { StrategyCatalog } from "@/components/strategies/strategy-catalog";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { permitsUse } from "@/lib/products/contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Strategies",
};

const BLOCKED_CODES = new Set(["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"]);

export default async function StrategiesPage() {
  try {
    const api = await getProductApi();
    const [catalog, licensing] = await Promise.all([api.getStrategies(), api.getLicensingStatus()]);
    if (catalog.items.length === 0) {
      return (
        <RoutePage
          description="Review approved strategy definitions, versions, states, and constrained parameters."
          title="Strategies"
        >
          <StatePanel
            kind="empty"
            message="The server returned no approved strategies. No configuration can be created."
            title="Strategy catalog is empty"
          />
        </RoutePage>
      );
    }
    const canConfigure =
      permitsUse(licensing, "recommendation") || permitsUse(licensing, "backtest");
    return (
      <RoutePage
        description="Review approved strategy definitions, versions, states, and constrained parameters."
        title="Strategies"
      >
        <StrategyCatalog canConfigure={canConfigure} strategies={catalog.items} />
      </RoutePage>
    );
  } catch (error) {
    const blocked = error instanceof ApiProblem && BLOCKED_CODES.has(error.code);
    return (
      <RoutePage
        description="Review approved strategy definitions, versions, states, and constrained parameters."
        title="Strategies"
      >
        <StatePanel
          kind={blocked ? "blocked" : "error"}
          message={
            blocked
              ? "Strategy configuration is blocked because the required data entitlement is inactive. No configuration was submitted."
              : "The strategy catalog could not be loaded. Retry after checking the service status."
          }
          title={blocked ? "Strategy configuration is blocked" : "Strategy catalog unavailable"}
        />
      </RoutePage>
    );
  }
}
