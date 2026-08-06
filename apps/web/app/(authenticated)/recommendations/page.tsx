import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Recommendations",
};

export default function RecommendationsPage() {
  return (
    <RoutePage
      description="Inspect server-produced candidates, target weights, factor evidence, and exclusions."
      title="Recommendations"
    >
      <StatePanel
        action={
          <Link className="secondary-action" href="/strategies">
            Review strategies
          </Link>
        }
        kind="empty"
        message="No recommendation run is selected. Results can populate this route without exposing another user’s data."
        title="No recommendation selected"
      />
    </RoutePage>
  );
}
