import Link from "next/link";
import type { ReactNode } from "react";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import type { ApiSession } from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";

const OWNER_ACCESS_BY_ROLE = {
  member: false,
  owner: true,
} as const satisfies Record<ApiSession["role"], boolean>;

export type OwnerRouteProps = {
  readonly children: ReactNode;
  readonly description: string;
  readonly title: string;
};

export async function OwnerRoute({ children, description, title }: OwnerRouteProps) {
  const session = await getServerSession();
  if (!OWNER_ACCESS_BY_ROLE[session.role]) {
    return (
      <RoutePage description={description} title={title}>
        <StatePanel
          action={
            <Link className="secondary-action" href="/">
              Return to dashboard
            </Link>
          }
          kind="blocked"
          message="This workspace requires the Owner role. Your current session remains signed in with Member access."
          title="Owner access required"
        />
      </RoutePage>
    );
  }
  return (
    <RoutePage description={description} title={title}>
      {children}
    </RoutePage>
  );
}
