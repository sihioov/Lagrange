import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Strategies",
};

export default function StrategiesPage() {
  return (
    <RoutePage
      description="Review only approved strategy definitions, versions, and constrained parameters."
      title="Strategies"
    >
      <StatePanel
        action={
          <Link className="secondary-action" href="/">
            Return to dashboard
          </Link>
        }
        kind="empty"
        message="No strategy is selected. The catalog can populate this route without changing the authenticated shell."
        title="No strategy selected"
      />
    </RoutePage>
  );
}
