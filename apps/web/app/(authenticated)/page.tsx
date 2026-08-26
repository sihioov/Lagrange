import { ArrowUpRightIcon } from "@phosphor-icons/react/ssr";
import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import {
  type ApiSession,
  type OwnerBetaProduct,
  permitsOwnerBetaProduct,
} from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";
import { type ShellDictionary, shellDictionary } from "@/lib/i18n/dictionaries/shell";
import { getLocale } from "@/lib/i18n/server";

const OWNER_BETA_DESTINATIONS = {
  "/backtests": "backtests",
  "/paper": "paper",
  "/recommendations": "recommendations",
} as const satisfies Record<string, OwnerBetaProduct>;

function workspaces(t: ShellDictionary, session: ApiSession) {
  const items = [
    { description: t.strategiesDescription, href: "/strategies", label: t.navStrategies },
    {
      description: t.recommendationsDescription,
      href: "/recommendations",
      label: t.navRecommendations,
    },
    { description: t.backtestsDescription, href: "/backtests", label: t.navBacktests },
    { description: t.paperAccountDescription, href: "/paper", label: t.navPaperAccount },
  ] as const;

  return items.filter((workspace) => {
    const product = OWNER_BETA_DESTINATIONS[workspace.href as keyof typeof OWNER_BETA_DESTINATIONS];
    return product === undefined || permitsOwnerBetaProduct(session, product);
  });
}

export const metadata: Metadata = {
  title: "Dashboard",
};

export default async function DashboardPage() {
  const [locale, session] = await Promise.all([getLocale(), getServerSession()]);
  const t = shellDictionary[locale];
  return (
    <RoutePage description={t.dashboardDescription} title={t.dashboardTitle}>
      <section aria-labelledby="workspace-heading" className="surface-panel workspace-overview">
        <div className="workspace-introduction">
          <p className="status-pill">
            <span aria-hidden="true" />
            {t.privateSession}
          </p>
          <h2 id="workspace-heading">{t.chooseWorkspaceHeading}</h2>
          <p>{t.chooseWorkspaceDescription}</p>
        </div>
        <div className="workspace-grid">
          {workspaces(t, session).map((workspace) => (
            <Link className="workspace-link" href={workspace.href} key={workspace.href}>
              <span>
                <strong>{workspace.label}</strong>
                <small>{workspace.description}</small>
              </span>
              <ArrowUpRightIcon aria-hidden="true" size={18} weight="regular" />
            </Link>
          ))}
        </div>
      </section>
    </RoutePage>
  );
}
