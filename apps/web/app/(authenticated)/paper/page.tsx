import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { PaperBindForm } from "@/components/paper/paper-bind-form";
import { PaperHoldings } from "@/components/paper/paper-holdings";
import { PaperLineage } from "@/components/paper/paper-lineage";
import { PaperNotifications } from "@/components/paper/paper-notifications";
import { PaperParityPanel } from "@/components/paper/paper-parity-panel";
import { PaperPerformance } from "@/components/paper/paper-performance";
import { PaperRebalancePreview } from "@/components/paper/paper-rebalance-preview";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type PaperDictionary, paperDictionary } from "@/lib/i18n/dictionaries/paper";
import { getLocale } from "@/lib/i18n/server";
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

const BLOCKED_CODES = new Set([
  "DATASET_BLOCKED",
  "DATA_ENTITLEMENT_REQUIRED",
  "DATA_STALE",
  "FORBIDDEN",
]);

function shell(children: React.ReactNode, t: PaperDictionary) {
  return (
    <RoutePage description={t.pageDescription} title={t.pageTitle}>
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

export type PaperPageProps = {
  readonly searchParams?: Promise<{ readonly account?: string }>;
};

export default async function PaperPage({ searchParams }: PaperPageProps = {}) {
  const locale = await getLocale();
  const t = paperDictionary[locale];
  try {
    const api = await getProductApi();
    const accounts = await api.getPaperAccounts();
    const requestedAccount = (await searchParams)?.account;
    const account =
      accounts.items.find((candidate) => candidate.id === requestedAccount) ??
      defaultAccount(accounts.items);
    if (account === null) {
      return shell(
        <StatePanel
          action={
            <Link className="secondary-action" href="/strategies">
              {t.reviewStrategiesLink}
            </Link>
          }
          kind="empty"
          message={t.noAccountMessage}
          title={t.noAccountTitle}
        />,
        t,
      );
    }

    const [performance, lineage, positions, orders, configs, notifications, recommendationRuns] =
      await Promise.all([
        api.getPaperPerformance(account.id),
        api.getPaperLineage(account.id),
        api.getPaperPositions(account.id),
        api.getPaperOrders(account.id),
        api.getStrategyConfigs(),
        api.getNotifications(),
        api.getRecommendationRuns(),
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
        <section aria-labelledby="paper-account-switcher-title" className="workflow-panel">
          <div className="section-heading">
            <div>
              <p className="eyebrow">{t.accountSwitcherEyebrow}</p>
              <h2 id="paper-account-switcher-title">{t.accountSwitcherHeading}</h2>
            </div>
          </div>
          <nav aria-label={t.accountSwitcherHeading} className="status-cluster">
            {accounts.items.map((candidate) => (
              <Link
                aria-current={candidate.id === account.id ? "page" : undefined}
                className="secondary-action"
                href={`/paper?account=${candidate.id}`}
                key={candidate.id}
              >
                {candidate.name} ·{" "}
                {candidate.can_manage ? t.yourAccountLabel : t.sharedAccountShortLabel}
              </Link>
            ))}
          </nav>
        </section>
        <PaperHoldings account={account} orders={orders.items} positions={positions.items} t={t} />
        <PaperPerformance performance={performance} t={t} />
        {parity === null ? (
          <StatePanel kind="empty" message={t.noParityMessage} title={t.noParityTitle} />
        ) : (
          <PaperParityPanel parity={parity} t={t} />
        )}
        <PaperLineage lineage={lineage} t={t} />
        {!account.can_manage ? null : bindable.length === 0 ? (
          <StatePanel
            action={
              <Link className="secondary-action" href="/strategies">
                {t.createStrategyConfigLink}
              </Link>
            }
            kind="empty"
            message={t.noConfigMessage}
            title={t.noConfigTitle}
          />
        ) : (
          <section aria-labelledby="paper-bind-title" className="workflow-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">{t.accountBranchingEyebrow}</p>
                <h2 id="paper-bind-title">{t.bindStrategyHeading}</h2>
              </div>
              <p>{t.onlyServerBindsNote}</p>
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
        {!account.can_manage ? null : (
          <PaperRebalancePreview
            accountId={account.id}
            runs={recommendationRuns.items
              .filter((run) => run.status === "SUCCEEDED")
              .map((run) => ({ asOf: run.as_of, id: run.id }))}
          />
        )}
        <PaperNotifications notifications={notifications.items} t={t} />
      </>,
      t,
    );
  } catch (error) {
    if (error instanceof ApiProblem && BLOCKED_CODES.has(error.code)) {
      return shell(
        <StatePanel
          kind="blocked"
          message={t.entitlementInactiveMessage}
          title={t.dataBlockedTitle}
        />,
        t,
      );
    }
    return shell(
      <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />,
      t,
    );
  }
}
