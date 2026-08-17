import Link from "next/link";
import type { ReactNode } from "react";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import type { ApiSession } from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";
import { shellDictionary } from "@/lib/i18n/dictionaries/shell";
import { getLocale } from "@/lib/i18n/server";

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
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  const t = shellDictionary[locale];
  if (!OWNER_ACCESS_BY_ROLE[session.role]) {
    // `t.refusedDescription` is deliberately NOT the caller's `description`:
    // that text exists to orient the Owner and therefore enumerates the
    // capabilities behind the gate. Rendering it to a refused Member would
    // disclose, in prose, exactly what the refusal is meant to conceal — for
    // the Live route it named broker connections, node lifecycle, and the
    // kill switch.
    return (
      <RoutePage description={t.refusedDescription} title={title}>
        <StatePanel
          action={
            <Link className="quiet-action" href="/">
              {t.returnToDashboard}
            </Link>
          }
          kind="blocked"
          message={t.ownerAccessRequiredMessage}
          title={t.ownerAccessRequiredTitle}
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
