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

/**
 * What a refused visitor is told the page is about.
 *
 * NOT the caller's `description`: that text exists to orient the Owner and
 * therefore enumerates the capabilities behind the gate. Rendering it to a
 * refused Member would disclose, in prose, exactly what the refusal is meant
 * to conceal — for the Live route it named broker connections, node lifecycle,
 * and the kill switch.
 */
const REFUSED_DESCRIPTION = "This area is restricted to the Owner.";

export async function OwnerRoute({ children, description, title }: OwnerRouteProps) {
  const session = await getServerSession();
  if (!OWNER_ACCESS_BY_ROLE[session.role]) {
    return (
      <RoutePage description={REFUSED_DESCRIPTION} title={title}>
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
