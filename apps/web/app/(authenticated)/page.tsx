import { ArrowUpRightIcon } from "@phosphor-icons/react/ssr";
import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";

const WORKSPACES = [
  {
    description: "Review approved strategies and their constrained parameters.",
    href: "/strategies",
    label: "Strategies",
  },
  {
    description: "Inspect explainable candidates, weights, and exclusions.",
    href: "/recommendations",
    label: "Recommendations",
  },
  {
    description: "Create reproducible runs and review risk evidence.",
    href: "/backtests",
    label: "Backtests",
  },
  {
    description: "Monitor your private simulated account and orders.",
    href: "/paper",
    label: "Paper account",
  },
] as const;

export const metadata: Metadata = {
  title: "Dashboard",
};

export default function DashboardPage() {
  return (
    <RoutePage
      description="Move between isolated research workspaces. Authenticated data is fetched per request and is never shared across sessions."
      title="Research dashboard"
    >
      <section aria-labelledby="workspace-heading" className="surface-panel workspace-overview">
        <div className="workspace-introduction">
          <p className="status-pill">
            <span aria-hidden="true" />
            Private session
          </p>
          <h2 id="workspace-heading">Choose a workspace</h2>
          <p>Each destination opens a server-authorized view with conservative failure states.</p>
        </div>
        <div className="workspace-grid">
          {WORKSPACES.map((workspace) => (
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
