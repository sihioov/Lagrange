import Link from "next/link";
import type { ReactNode } from "react";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import type { ApiSession } from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";
import { shellDictionary } from "@/lib/i18n/dictionaries/shell";
import type { Locale } from "@/lib/i18n/locale";
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

export type OwnerAccessRefusalProps = {
  readonly locale: Locale;
  readonly title: string;
};

/** One non-enumerating refusal surface shared by every Owner role boundary. */
export function OwnerAccessRefusal({ locale, title }: OwnerAccessRefusalProps) {
  const t = shellDictionary[locale];
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

/** Non-sensitive readiness refusal for an authenticated Owner. */
export function OwnerBetaPaperUnavailable({ locale, title }: OwnerAccessRefusalProps) {
  const t = shellDictionary[locale];
  return (
    <RoutePage description={t.ownerBetaPaperUnavailableDescription} title={title}>
      <StatePanel
        action={
          <Link className="quiet-action" href="/">
            {t.returnToDashboard}
          </Link>
        }
        kind="blocked"
        message={t.ownerBetaPaperUnavailableMessage}
        title={t.ownerBetaPaperUnavailableTitle}
      />
    </RoutePage>
  );
}

export async function OwnerRoute({ children, description, title }: OwnerRouteProps) {
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  if (!OWNER_ACCESS_BY_ROLE[session.role]) {
    // `t.refusedDescription` is deliberately NOT the caller's `description`:
    // that text exists to orient the Owner and therefore enumerates the
    // capabilities behind the gate. Rendering it to a refused Member would
    // disclose, in prose, exactly what the refusal is meant to conceal — for
    // the Live route it named broker connections, node lifecycle, and the
    // kill switch.
    return <OwnerAccessRefusal locale={locale} title={title} />;
  }
  return (
    <RoutePage description={description} title={title}>
      {children}
    </RoutePage>
  );
}
