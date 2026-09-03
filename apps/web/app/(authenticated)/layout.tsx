import type { ReactNode } from "react";
import { AppShell } from "@/components/shell/app-shell";
import { getServerSession } from "@/lib/api/server-session";
import { getLocale } from "@/lib/i18n/server";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export type AuthenticatedLayoutProps = {
  readonly children: ReactNode;
};

export default async function AuthenticatedLayout({ children }: AuthenticatedLayoutProps) {
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  return (
    <AppShell locale={locale} session={session}>
      {children}
    </AppShell>
  );
}
