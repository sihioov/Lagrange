import type { Metadata } from "next";
import { OwnerRoute } from "@/components/pages/owner-route";
import { StatePanel } from "@/components/states/state-panel";

export const metadata: Metadata = {
  title: "Administration",
};

export default function AdminPage() {
  return (
    <OwnerRoute
      description="Review datasets, jobs, workers, users, and immutable audit evidence through explicit Owner pathways."
      title="Administration"
    >
      <StatePanel
        kind="empty"
        message="No administrative area is selected. Operational data can populate this route only through audited Owner APIs."
        title="Choose an administrative area"
      />
    </OwnerRoute>
  );
}
