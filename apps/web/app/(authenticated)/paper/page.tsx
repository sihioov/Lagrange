import type { Metadata } from "next";
import Link from "next/link";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Paper account",
};

export default function PaperPage() {
  return (
    <RoutePage
      description="Review the cash, positions, orders, fills, and daily performance of your private simulated account."
      title="Paper account"
    >
      <StatePanel
        action={
          <Link className="secondary-action" href="/strategies">
            Review strategies
          </Link>
        }
        kind="empty"
        message="No paper account is selected. Account data can populate this route only after server ownership checks succeed."
        title="No paper account selected"
      />
    </RoutePage>
  );
}
