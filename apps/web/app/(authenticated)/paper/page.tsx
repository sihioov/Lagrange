import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { PaperBindForm } from "@/components/paper/paper-bind-form";
import { PaperHoldings } from "@/components/paper/paper-holdings";
import { PaperLineage } from "@/components/paper/paper-lineage";
import { PaperNotifications } from "@/components/paper/paper-notifications";
import { PaperParityPanel } from "@/components/paper/paper-parity-panel";
import { PaperPerformance } from "@/components/paper/paper-performance";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import {
  defaultAccount,
  type PaperLineageModel,
  type PaperParityModel,
} from "@/lib/products/paper-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Paper account",
};

const DESCRIPTION =
  "Review the cash, positions, orders, fills, and daily performance of your private simulated account.";

const BLOCKED_CODES = new Set([
  "DATASET_BLOCKED",
  "DATA_ENTITLEMENT_REQUIRED",
  "DATA_STALE",
  "FORBIDDEN",
]);

function shell(children: React.ReactNode) {
  return (
    <RoutePage description={DESCRIPTION} title="Paper account">
      {children}
    </RoutePage>
  );
}

/**
 * The session a parity report is requested for.
 *
 * It is the close that produced the most recent target — never today's date
 * and never a hardcoded one. A parity claim is only meaningful for a session
 * both sides actually computed.
 */
function latestSession(lineage: PaperLineageModel): string | null {
  const sessions = lineage.targets.map((target) => target.computed_on).sort();
  return sessions.at(-1) ?? null;
}

export default async function PaperPage() {
  try {
    const api = await getProductApi();
    const accounts = await api.getPaperAccounts();
    const account = defaultAccount(accounts.items);
    if (account === null) {
      return shell(
        <StatePanel
          action={
            <Link className="secondary-action" href="/strategies">
              Review strategies
            </Link>
          }
          kind="empty"
          message="No paper account is selected. Account data can populate this route only after server ownership checks succeed."
          title="No paper account selected"
        />,
      );
    }

    const [performance, lineage, positions, orders, configs, notifications] = await Promise.all([
      api.getPaperPerformance(account.id),
      api.getPaperLineage(account.id),
      api.getPaperPositions(account.id),
      api.getPaperOrders(account.id),
      api.getStrategyConfigs(),
      api.getNotifications(),
    ]);

    const session = latestSession(lineage);
    let parity: PaperParityModel | null = null;
    if (session !== null) {
      parity = await api.getPaperParity(account.id, session);
    }

    const activeBinding = lineage.bindings.find((binding) => binding.active);
    const bindable = configs.items.map((config) => ({
      id: config.id,
      label: `${config.strategy_id}@${config.strategy_version}`,
    }));

    return shell(
      <>
        <PaperHoldings account={account} orders={orders.items} positions={positions.items} />
        <PaperPerformance performance={performance} />
        {parity === null ? (
          <StatePanel
            kind="empty"
            message="No session has queued a target yet, so there is nothing to compare against a backtest."
            title="No parity report available"
          />
        ) : (
          <PaperParityPanel parity={parity} />
        )}
        <PaperLineage lineage={lineage} />
        {bindable.length === 0 ? (
          <StatePanel
            action={
              <Link className="secondary-action" href="/strategies">
                Create a strategy configuration
              </Link>
            }
            kind="empty"
            message="Binding an account needs a saved strategy configuration."
            title="No strategy configuration to bind"
          />
        ) : (
          <section aria-labelledby="paper-bind-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Account branching</p>
                <h2 id="paper-bind-title">Bind strategy</h2>
              </div>
              <p>Only the server opens and closes bindings.</p>
            </div>
            <PaperBindForm
              accountId={account.id}
              activeStrategy={
                activeBinding === undefined
                  ? null
                  : `${activeBinding.strategy_id}@${activeBinding.strategy_version}`
              }
              configs={bindable}
            />
          </section>
        )}
        <PaperNotifications notifications={notifications.items} />
      </>,
    );
  } catch (error) {
    if (error instanceof ApiProblem && BLOCKED_CODES.has(error.code)) {
      return shell(
        <StatePanel
          kind="blocked"
          message="The paper entitlement is inactive. Account data and simulated results are not rendered."
          title="Paper data is blocked"
        />,
      );
    }
    return shell(
      <StatePanel
        kind="error"
        message="Paper account data could not be loaded. Retry after checking the service status."
        title="Paper account unavailable"
      />,
    );
  }
}
