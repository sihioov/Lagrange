import type { Metadata } from "next";
import { OwnerRoute } from "@/components/pages/owner-route";
import { StatePanel } from "@/components/states/state-panel";
import { adminDictionary } from "@/lib/i18n/dictionaries/admin";
import { getLocale } from "@/lib/i18n/server";

export const metadata: Metadata = {
  title: "Administration",
};

export default async function AdminPage() {
  const locale = await getLocale();
  const t = adminDictionary[locale];
  return (
    <OwnerRoute description={t.pageDescription} title={t.pageTitle}>
      <StatePanel kind="empty" message={t.noAreaMessage} title={t.noAreaTitle} />
    </OwnerRoute>
  );
}
