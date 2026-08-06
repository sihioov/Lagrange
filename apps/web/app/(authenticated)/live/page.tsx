import type { Metadata } from "next";
import { OwnerRoute } from "@/components/pages/owner-route";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Live controls",
};

export default function LivePage() {
  return (
    <OwnerRoute
      description="Phase-three connection, reconciliation, order-limit, and kill-switch controls remain isolated from the MVP."
      title="Live controls"
    >
      <StatePanel
        kind="blocked"
        message="Live controls are profile-gated and disabled in this build. No brokerage credential or order control is exposed to the browser."
        title="Live trading is not enabled"
      />
    </OwnerRoute>
  );
}
